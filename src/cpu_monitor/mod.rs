use std::collections::VecDeque;

pub mod state;
pub mod widgets;
pub mod panels;

pub use state::{CpuState, CpuTraceEntry, DevicesSnapshot, IouAccessSample, IouSnapshot, MAX_TRACE_ENTRIES};

use panels::memory::MemoryPanelState;
use panels::watches::WatchesPanelState;
use panels::breakpoints::BreakpointSet;
use panels::disassembly::DisassemblyPanelState;

// Borrowed view of the live emulator framebuffer for display in the Video
// pane. Pixels are tightly packed RGBA8 (4 bytes per pixel, row-major).
pub struct FramebufferView<'a> {
    pub pixels: &'a [u8],
    pub width: u32,
    pub height: u32,
}

pub struct MonitorFrame<'a> {
    pub cpu_state: &'a CpuState,
    pub iou: &'a IouSnapshot,
    pub devices: &'a DevicesSnapshot,
    pub memory_reader: &'a dyn Fn(u16) -> u8,
    pub framebuffer: Option<FramebufferView<'a>>,
    pub framebuffer_raw: Option<FramebufferView<'a>>,
}

// Default height (px) of the bottom-pinned Trace and Video panes. They share
// this so their tops align across the Code and Memory columns.
const BOTTOM_PANE_HEIGHT: f32 = 280.0;

pub struct CpuMonitor {
    // Whether the monitor is actively capturing traces.
    pub enabled: bool,
    // Whether the monitor window should be visible.
    pub visible: bool,

    // Ring buffer of recent trace entries. Still recorded for stats and
    // future trace exports; the in-window trace panel was removed in favor
    // of the main-view trace overlay.
    pub trace_buffer: VecDeque<CpuTraceEntry>,

    // Auto-scroll trace view to bottom (kept for compatibility with the
    // main-view trace overlay).
    pub auto_scroll: bool,

    // Pause emulation (drained by main loop).
    pub paused: bool,

    // When `true`, the breakpoint check in the main loop will skip the
    // *next* instruction even if its PC matches a breakpoint, then
    // re-arm. Set when resuming from a halt-on-breakpoint or stepping
    // over the BP'd instruction so Run/Step actually makes forward
    // progress instead of re-tripping the same BP immediately.
    pub skip_next_breakpoint: bool,

    // Number of `cpu.tick()`s to execute while paused (drained by main loop).
    pub pending_steps: u32,

    // Sticky flag: `true` while the current paused snapshot of
    // `scan_cycle` reflects an actual single-instruction sample (i.e.
    // the user just stepped, or the CPU halted on a breakpoint mid-
    // frame). Cleared the moment the user resumes execution. The IOU
    // scan-position bar consults this to decide whether to show its
    // L<line>.<cyc> readout: while free-running every snapshot lands
    // on a frame boundary so the value is misleading drift, but step
    // samples are accurate to the cycle.
    pub fresh_step_sample: bool,

    // Right-pane collapsing-section state.
    pub watches_open: bool,
    pub breakpoints_open: bool,

    // Per-panel state.
    pub memory: MemoryPanelState,
    pub watches: WatchesPanelState,
    pub disasm: DisassemblyPanelState,
    pub breakpoints: BreakpointSet,

    // Cached egui texture mirroring the emulator framebuffer for the Video
    // pane preview. Lazily created; reuploaded each frame.
    fb_texture: Option<egui::TextureHandle>,

    // `true` → Video pane shows the raw pre-effects framebuffer;
    // `false` (default) → final processed framebuffer.
    fb_show_raw: bool,
}

impl Default for CpuMonitor {
    fn default() -> Self {
        Self {
            enabled: false,
            visible: false,
            trace_buffer: VecDeque::with_capacity(MAX_TRACE_ENTRIES),
            auto_scroll: true,
            paused: false,
            pending_steps: 0,
            fresh_step_sample: false,
            skip_next_breakpoint: false,
            watches_open: false,
            breakpoints_open: false,
            memory: MemoryPanelState::default(),
            watches: WatchesPanelState::default(),
            disasm: DisassemblyPanelState::default(),
            breakpoints: BreakpointSet::default(),
            fb_texture: None,
            fb_show_raw: false,
        }
    }
}

impl CpuMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(dead_code)]
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        self.enabled = self.visible;
    }

    // Drop GPU/egui resources tied to a specific egui context. Call when
    // the monitor's OS window is closed so a stale `TextureHandle` from
    // the old context isn't reused after the window reopens with a new
    // context.
    pub fn on_window_closed(&mut self) {
        self.fb_texture = None;
    }

    // Record a trace entry. Called both during free-running execution
    // (once per frame from `cpu.last_trace`) and per-instruction during
    // paused single-stepping.
    #[inline]
    pub fn record(&mut self, entry: CpuTraceEntry) {
        if !self.enabled {
            return;
        }
        if self.trace_buffer.len() >= MAX_TRACE_ENTRIES {
            self.trace_buffer.pop_front();
        }
        self.trace_buffer.push_back(entry);
    }

    pub fn clear_trace(&mut self) {
        self.trace_buffer.clear();
    }

    // Read & reset the disasm cache hit/miss counters (for `--perf`).
    pub fn take_disasm_cache_stats(&mut self) -> (u64, u64) {
        let stats = (self.disasm.cache_hits, self.disasm.cache_misses);
        self.disasm.cache_hits = 0;
        self.disasm.cache_misses = 0;
        stats
    }

    #[inline]
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    #[inline]
    pub fn take_pending_steps(&mut self) -> u32 {
        std::mem::take(&mut self.pending_steps)
    }

    // Render the monitor window. Returns whether the window is still open.
    #[allow(dead_code)]
    pub fn render(
        &mut self,
        ctx: &egui::Context,
        cpu_state: &CpuState,
        iou: &IouSnapshot,
        memory_reader: &dyn Fn(u16) -> u8,
    ) -> bool {
        if !self.visible {
            return false;
        }

        let mut open = self.visible;
        let screen = ctx.content_rect();
        let max_h = (screen.height() - 40.0).max(360.0);
        let max_w = (screen.width() - 40.0).max(640.0);

        egui::Window::new("CPU Monitor")
            .id(egui::Id::new("cpu_monitor_v8"))
            .open(&mut open)
            .default_size([960.0, 660.0])
            .min_width(640.0)
            .min_height(380.0)
            .max_width(max_w)
            .max_height(max_h)
            .resizable(true)
            .vscroll(false)
            .hscroll(false)
            .show(ctx, |ui| {
                let devs = DevicesSnapshot::default();
                self.render_body(ui, MonitorFrame {
                    cpu_state,
                    iou,
                    devices: &devs,
                    memory_reader,
                    framebuffer: None,
                    framebuffer_raw: None,
                });
            });

        self.visible = open;
        if !open {
            self.enabled = false;
            self.paused = false;
        }

        open
    }

    // Render the monitor contents directly into a Ui, without an
    // egui::Window wrapper. Used when the monitor lives in its own
    // OS-level window.
    pub fn render_inline(&mut self, ui: &mut egui::Ui, frame: MonitorFrame<'_>) {
        self.render_body(ui, frame);
    }

    fn render_body(&mut self, ui: &mut egui::Ui, frame: MonitorFrame<'_>) {
        let MonitorFrame {
            cpu_state,
            iou,
            devices,
            memory_reader,
            framebuffer,
            framebuffer_raw,
        } = frame;
        // The monitor displays many fields that change every CPU tick
        // (scan_cycle, cycle counter, $C0xx access log, audio scopes…).
        // egui only repaints on input events by default, which makes
        // those fields alias badly (e.g. the scan-position cursor appears
        // to drift over many seconds). Request a fresh frame every ~16 ms
        // while the monitor is open so the displayed snapshot tracks
        // emulator wall-clock time.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        // Header strip: run/pause/step controls. Fixed height.
        egui::TopBottomPanel::top("cpu_monitor_header")
            .resizable(false)
            .show_inside(ui, |ui| {
                ui.add_space(2.0);
                self.render_header(ui);
                ui.add_space(2.0);
            });

        // Status strip: registers + flags + cycles. Fixed height.
        egui::TopBottomPanel::top("cpu_monitor_status")
            .resizable(false)
            .show_inside(ui, |ui| {
                ui.add_space(2.0);
                panels::registers::render(ui, cpu_state, iou);
                ui.add_space(2.0);
            });

        // Right column: stack + watches + breakpoints. User-resizable.
        egui::SidePanel::right("cpu_monitor_stack")
            .resizable(true)
            .default_width(220.0)
            .min_width(200.0)
            .max_width(520.0)
            .show_inside(ui, |ui| {
                self.render_stack_pane(ui, cpu_state, memory_reader);
            });

        // Devices column: IOU / Devices / Interrupts stacked vertically.
        // Allocated after the stack panel so it ends up immediately to its left.
        egui::SidePanel::right("cpu_monitor_devices")
            .resizable(true)
            .default_width(320.0)
            .min_width(240.0)
            .max_width(460.0)
            .show_inside(ui, |ui| {
                self.render_devices_pane(ui, iou, devices);
            });

        // Left column: disassembly with a trace pane pinned beneath.
        // Trace shares its top edge with the Video pane in the central area
        // (matching default heights).
        egui::SidePanel::left("cpu_monitor_code")
            .resizable(true)
            .default_width(520.0)
            .min_width(320.0)
            .show_inside(ui, |ui| {
                egui::TopBottomPanel::bottom("cpu_monitor_trace")
                    .resizable(true)
                    .default_height(BOTTOM_PANE_HEIGHT)
                    .min_height(120.0)
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Trace").strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("Clear").clicked() {
                                        self.clear_trace();
                                    }
                                    ui.checkbox(&mut self.auto_scroll, "Follow");
                                },
                            );
                        });
                        ui.separator();
                        panels::trace::render(ui, &self.trace_buffer, self.auto_scroll);
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Code").strong());
                            ui.separator();
                            panels::disassembly::render_toolbar(ui, &mut self.disasm);
                        });
                        ui.separator();
                        panels::disassembly::render(ui, &mut self.disasm, cpu_state, &mut self.breakpoints, memory_reader);
                    });
            });

        // Center column: Memory (sized to its 16 fixed rows) pinned at the
        // top, Video pane filling the remaining vertical space below it so
        // the framebuffer can grow to use any free area.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| {
                egui::TopBottomPanel::top("cpu_monitor_memory")
                    .resizable(true)
                    .show_inside(ui, |ui| {
                        ui.label(egui::RichText::new("Memory").strong());
                        egui::Frame::group(ui.style())
                            .inner_margin(egui::Margin::symmetric(6, 4))
                            .show(ui, |ui| {
                                panels::iou::render_mmu_pane(ui, iou);
                            });
                        ui.add_space(4.0);
                        panels::memory::render(ui, &mut self.memory, memory_reader);
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 0 }))
                    .show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Video").strong());
                            ui.add_space(8.0);
                            panels::iou::scan_position_bar(ui, iou, self.fresh_step_sample);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.selectable_value(
                                        &mut self.fb_show_raw,
                                        true,
                                        "Raw",
                                    )
                                    .on_hover_text(
                                        "Pre-effects framebuffer (no NTSC blur, comb filter, phosphor spread, or scanlines).",
                                    );
                                    ui.selectable_value(
                                        &mut self.fb_show_raw,
                                        false,
                                        "Processed",
                                    )
                                    .on_hover_text(
                                        "Final CPU-side framebuffer fed to the GPU shader.",
                                    );
                                },
                            );
                        });
                        ui.separator();
                        panels::iou::render_video_pane(ui, iou);
                        ui.add_space(4.0);
                        let fb_to_show = if self.fb_show_raw {
                            framebuffer_raw.as_ref().or(framebuffer.as_ref())
                        } else {
                            framebuffer.as_ref()
                        };
                        self.render_framebuffer(ui, fb_to_show);
                    });
            });
    }

    fn render_devices_pane(
        &mut self,
        ui: &mut egui::Ui,
        iou: &IouSnapshot,
        devices: &DevicesSnapshot,
    ) {
        // Two stacked sections (Interrupts moved to the status row).
        ui.label(egui::RichText::new("IOU").strong());
        ui.separator();
        panels::iou::iou_body(ui, iou);
        ui.add_space(10.0);

        ui.label(egui::RichText::new("Devices").strong());
        ui.separator();
        panels::devices::render(ui, devices);
    }

    fn render_stack_pane(
        &mut self,
        ui: &mut egui::Ui,
        cpu_state: &CpuState,
        memory_reader: &dyn Fn(u16) -> u8,
    ) {
        // Lock content to the column's current width so a wide widget
        // (e.g. the Watches text input or a long Breakpoint symbol) can
        // never push the SidePanel itself wider on the next frame.
        let col_w = ui.available_width();
        ui.set_max_width(col_w);

        // Reserve vertical space for the bottom collapsing sections based on
        // their *current* open/closed state.
        let total_h = ui.available_height();
        let bottom_reserve = {
            // Header bar height per CollapsingHeader (~22px) plus separator.
            let header_h = 22.0;
            let separator_h = 6.0;
            let mut h = header_h * 2.0 + separator_h;
            if self.watches_open || self.breakpoints_open {
                // When a section is open, give the bottom group up to half
                // the column for content + scrollbar.
                h = (total_h * 0.5).max(h + 120.0);
            }
            h.min(total_h - 60.0).max(header_h * 2.0 + separator_h)
        };
        let stack_h = (total_h - bottom_reserve).max(60.0);
        let body_max = (bottom_reserve - 50.0).max(40.0);

        // Stack
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), stack_h),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| panels::stack::render(ui, cpu_state, memory_reader),
        );

        ui.separator();

        // Watches + Breakpoints
        let watches_resp = egui::CollapsingHeader::new("Watches")
            .id_salt("cpu_watches_header")
            .default_open(self.watches_open)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("cpu_watches_scroll")
                    .max_height(body_max * 0.5)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        panels::watches::render(ui, &mut self.watches, memory_reader);
                    });
            });
        self.watches_open = watches_resp.fully_open();

        let bp_resp = egui::CollapsingHeader::new(format!("Breakpoints ({})", self.breakpoints.len()))
            .id_salt("cpu_bp_header")
            .default_open(self.breakpoints_open)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("cpu_bp_scroll")
                    .max_height(body_max * 0.5)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        panels::breakpoints::render(ui, &mut self.breakpoints);
                    });
            });
        self.breakpoints_open = bp_resp.fully_open();
    }

    fn render_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Status badge
            let (badge_text, badge_color) = if self.paused {
                ("PAUSED", egui::Color32::from_rgb(0xC8, 0x55, 0x40))
            } else {
                ("RUNNING", egui::Color32::from_rgb(0x4A, 0xA8, 0x60))
            };
            ui.label(
                egui::RichText::new(badge_text)
                    .color(egui::Color32::WHITE)
                    .background_color(badge_color)
                    .strong()
                    .monospace(),
            );

            ui.separator();

            let toggle_label = if self.paused { "\u{25B6}  Run" } else { "\u{23F8}  Pause" };
            if ui.button(toggle_label).clicked() {
                let was_paused = self.paused;
                self.paused = !self.paused;
                // Resuming: arm the one-shot BP skip so we step *past* the
                // current PC even if it matches a breakpoint. Otherwise the
                // BP check would re-fire instantly and we'd never make
                // forward progress from a halt.
                if was_paused && !self.paused {
                    self.skip_next_breakpoint = true;
                    // Resuming invalidates the cycle-accurate scan sample
                    self.fresh_step_sample = false;
                }
            }

            ui.add_enabled_ui(self.paused, |ui| {
                if ui.button("Step").on_hover_text("Execute one instruction").clicked() {
                    self.pending_steps = self.pending_steps.saturating_add(1);
                    self.skip_next_breakpoint = true;
                    self.fresh_step_sample = true;
                }
                if ui
                    .button("Step Over")
                    .on_hover_text("Coming soon — JSR-aware step")
                    .clicked()
                {
                    self.pending_steps = self.pending_steps.saturating_add(1);
                    self.fresh_step_sample = true;
                }
                if ui
                    .button("Step Out")
                    .on_hover_text("Coming soon — run until RTS")
                    .clicked()
                {
                    self.pending_steps = self.pending_steps.saturating_add(1);
                    self.fresh_step_sample = true;
                }
                if ui.button("\u{00D7}100").on_hover_text("Execute 100 instructions").clicked() {
                    self.pending_steps = self.pending_steps.saturating_add(100);
                    self.fresh_step_sample = true;
                }
                if ui.button("\u{00D7}1k").on_hover_text("Execute 1000 instructions").clicked() {
                    self.pending_steps = self.pending_steps.saturating_add(1_000);
                    self.fresh_step_sample = true;
                }
            });
        });
    }
}

fn placeholder(ui: &mut egui::Ui, text: &str) {
    // Inline weak label that takes only its own height — do NOT use
    // `centered_and_justified`, which expands to fill the parent and
    // hides anything laid out after it.
    ui.weak(text);
}

impl CpuMonitor {
    fn render_framebuffer(&mut self, ui: &mut egui::Ui, fb: Option<&FramebufferView<'_>>) {
        let Some(fb) = fb else {
            placeholder(ui, "(no framebuffer available)");
            return;
        };
        if fb.width == 0 || fb.height == 0
            || fb.pixels.len() < (fb.width as usize) * (fb.height as usize) * 4
        {
            placeholder(ui, "(invalid framebuffer)");
            return;
        }

        let size = [fb.width as usize, fb.height as usize];
        let image = egui::ColorImage::from_rgba_unmultiplied(size, fb.pixels);

        // Lazily create / update the egui texture from the latest pixels.
        let tex = self.fb_texture.get_or_insert_with(|| {
            ui.ctx().load_texture(
                "cpu_monitor_framebuffer",
                image.clone(),
                egui::TextureOptions::LINEAR,
            )
        });
        tex.set(image, egui::TextureOptions::LINEAR);

        // Fit into available space preserving aspect ratio.
        let avail = ui.available_size();
        let aspect = fb.width as f32 / fb.height as f32;
        let mut w = avail.x;
        let mut h = w / aspect;
        if h > avail.y {
            h = avail.y;
            w = h * aspect;
        }
        let sized = egui::vec2(w.max(1.0), h.max(1.0));

        ui.centered_and_justified(|ui| {
            ui.add(
                egui::Image::new((tex.id(), sized))
                    .fit_to_exact_size(sized)
                    .maintain_aspect_ratio(true),
            );
        });
    }
}
