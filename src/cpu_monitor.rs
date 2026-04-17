//! CPU Monitor - Real-time CPU state visualization via egui
//!
//! Provides an egui window for monitoring CPU execution without the
//! performance overhead of println! debugging. Data is captured as raw
//! values during execution and only formatted when rendered (~60fps).

use std::collections::VecDeque;
use crate::cpu::Flags;

/// Maximum number of trace entries to keep in the ring buffer
const MAX_TRACE_ENTRIES: usize = 2000;

/// Maximum number of memory watch entries
const MAX_WATCHES: usize = 16;

/// A single CPU trace entry - captured each instruction when monitor is active
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct CpuTraceEntry {
    pub pc: u16,
    pub opcode: u8,
    pub operand1: u8,
    pub operand2: u8,
    pub instruction_len: u8,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub p: u8,
    pub cycles: u64,
}

impl Default for CpuTraceEntry {
    fn default() -> Self {
        Self {
            pc: 0,
            opcode: 0,
            operand1: 0,
            operand2: 0,
            instruction_len: 1,
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFF,
            p: 0x34,
            cycles: 0,
        }
    }
}

impl CpuTraceEntry {
    /// Format flags as a string like "NV-BDIZC"
    pub fn format_flags(&self) -> String {
        let flags = Flags::from_bits_truncate(self.p);
        format!(
            "{}{}{}{}{}{}{}{}",
            if flags.contains(Flags::NEGATIVE) { 'N' } else { 'n' },
            if flags.contains(Flags::OVERFLOW) { 'V' } else { 'v' },
            '-',
            if flags.contains(Flags::BREAK) { 'B' } else { 'b' },
            if flags.contains(Flags::DECIMAL) { 'D' } else { 'd' },
            if flags.contains(Flags::IRQ_DISABLE) { 'I' } else { 'i' },
            if flags.contains(Flags::ZERO) { 'Z' } else { 'z' },
            if flags.contains(Flags::CARRY) { 'C' } else { 'c' },
        )
    }

    /// Format the instruction bytes as hex
    pub fn format_bytes(&self) -> String {
        match self.instruction_len {
            1 => format!("{:02X}      ", self.opcode),
            2 => format!("{:02X} {:02X}   ", self.opcode, self.operand1),
            3 => format!("{:02X} {:02X} {:02X}", self.opcode, self.operand1, self.operand2),
            _ => format!("{:02X}      ", self.opcode),
        }
    }
}

/// Memory watch entry
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct MemoryWatch {
    pub address: u16,
    pub size: u8, // 1, 2, or 4 bytes
    pub label: [u8; 16], // Fixed-size label buffer
    pub label_len: u8,
}

impl Default for MemoryWatch {
    fn default() -> Self {
        Self {
            address: 0,
            size: 1,
            label: [0; 16],
            label_len: 0,
        }
    }
}

impl MemoryWatch {
    pub fn new(address: u16, size: u8, label: &str) -> Self {
        let mut watch = Self {
            address,
            size: size.min(4),
            label: [0; 16],
            label_len: 0,
        };
        let bytes = label.as_bytes();
        let len = bytes.len().min(16);
        watch.label[..len].copy_from_slice(&bytes[..len]);
        watch.label_len = len as u8;
        watch
    }

    pub fn label_str(&self) -> &str {
        std::str::from_utf8(&self.label[..self.label_len as usize]).unwrap_or("")
    }
}

/// CPU Monitor state - owned by App
pub struct CpuMonitor {
    /// Whether the monitor is actively capturing traces
    pub enabled: bool,
    
    /// Whether the monitor window should be visible
    pub visible: bool,
    
    /// Ring buffer of recent trace entries
    pub trace_buffer: VecDeque<CpuTraceEntry>,
    
    /// Memory watch addresses
    pub watches: Vec<MemoryWatch>,
    
    /// Auto-scroll trace view to bottom
    pub auto_scroll: bool,
    
    /// Pause trace capture (keep window open but stop recording)
    pub paused: bool,
    
    /// Current memory page to display (high byte)
    pub memory_page: u8,
    
    /// Show/hide sections
    pub show_registers: bool,
    pub show_trace: bool,
    pub show_memory: bool,
    pub show_watches: bool,
    pub show_stack: bool,
    
    /// New watch address input buffer
    pub new_watch_addr: String,
    pub new_watch_label: String,
    
    /// Go-to address input
    pub goto_address: String,
}

impl Default for CpuMonitor {
    fn default() -> Self {
        Self {
            enabled: false,
            visible: false,
            trace_buffer: VecDeque::with_capacity(MAX_TRACE_ENTRIES),
            watches: Vec::with_capacity(MAX_WATCHES),
            auto_scroll: true,
            paused: false,
            memory_page: 0x00,
            show_registers: true,
            show_trace: true,
            show_memory: false,
            show_watches: false,
            show_stack: true,
            new_watch_addr: String::new(),
            new_watch_label: String::new(),
            goto_address: String::new(),
        }
    }
}

impl CpuMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle visibility and enable/disable
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        self.enabled = self.visible;
    }

    /// Record a trace entry (fast path - no formatting)
    #[inline]
    pub fn record(&mut self, entry: CpuTraceEntry) {
        if !self.enabled || self.paused {
            return;
        }
        
        if self.trace_buffer.len() >= MAX_TRACE_ENTRIES {
            self.trace_buffer.pop_front();
        }
        self.trace_buffer.push_back(entry);
    }

    /// Add a memory watch
    pub fn add_watch(&mut self, address: u16, size: u8, label: &str) {
        if self.watches.len() < MAX_WATCHES {
            self.watches.push(MemoryWatch::new(address, size, label));
        }
    }

    /// Remove a watch by index
    pub fn remove_watch(&mut self, index: usize) {
        if index < self.watches.len() {
            self.watches.remove(index);
        }
    }

    /// Clear all trace entries
    pub fn clear_trace(&mut self) {
        self.trace_buffer.clear();
    }

    /// Render the CPU monitor as an egui::Window (returns whether window is open)
    pub fn render(&mut self, ctx: &egui::Context, cpu_state: &CpuState, memory_reader: &dyn Fn(u16) -> u8) -> bool {
        if !self.visible {
            return false;
        }

        let mut open = self.visible;
        
        egui::Window::new("CPU Monitor")
            .open(&mut open)
            .default_size([650.0, 450.0])
            .min_width(400.0)
            .min_height(300.0)
            .resizable(true)
            .show(ctx, |ui| {
                self.render_ui(ui, cpu_state, memory_reader);
            });
        
        self.visible = open;
        if !open {
            self.enabled = false;
        }
        
        open
    }

    fn render_ui(&mut self, ui: &mut egui::Ui, cpu_state: &CpuState, memory_reader: &dyn Fn(u16) -> u8) {
        // Toolbar
        ui.horizontal(|ui| {
            if ui.button(if self.paused { "Resume" } else { "Pause" }).clicked() {
                self.paused = !self.paused;
            }
            if ui.button("Clear").clicked() {
                self.clear_trace();
            }
            ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
            ui.separator();
            ui.checkbox(&mut self.show_registers, "Regs");
            ui.checkbox(&mut self.show_trace, "Trace");
            ui.checkbox(&mut self.show_memory, "Memory");
            ui.checkbox(&mut self.show_stack, "Stack");
            ui.checkbox(&mut self.show_watches, "Watches");
        });

        ui.separator();

        // Current registers (always visible when show_registers)
        if self.show_registers {
            self.render_registers(ui, cpu_state);
            ui.separator();
        }

        // Main content area
        ui.horizontal(|ui| {
            // Left side: trace and memory
            ui.vertical(|ui| {
                ui.set_min_width(350.0);
                if self.show_trace {
                    self.render_trace(ui);
                }
                if self.show_memory {
                    ui.separator();
                    self.render_memory(ui, memory_reader);
                }
            });

            ui.separator();

            // Right side: stack and watches
            ui.vertical(|ui| {
                ui.set_min_width(180.0);
                if self.show_stack {
                    self.render_stack(ui, cpu_state, memory_reader);
                }
                if self.show_watches {
                    ui.separator();
                    self.render_watches(ui, memory_reader);
                }
            });
        });
    }

    fn render_registers(&self, ui: &mut egui::Ui, state: &CpuState) {
        ui.horizontal(|ui| {
            ui.monospace(format!(
                "PC:{:04X}  A:{:02X}  X:{:02X}  Y:{:02X}  SP:{:02X}  P:{:02X} [{}]  CYC:{}",
                state.pc, state.a, state.x, state.y, state.sp, state.p,
                format_flags_short(state.p),
                state.cycles
            ));
        });
    }

    fn render_trace(&mut self, ui: &mut egui::Ui) {
        ui.label(format!("Trace ({} entries)", self.trace_buffer.len()));
        
        let available_height = (ui.available_height() - 20.0).max(100.0);
        
        egui::ScrollArea::vertical()
            .max_height(available_height)
            .auto_shrink([false; 2])
            .stick_to_bottom(self.auto_scroll)
            .show(ui, |ui| {
                for entry in self.trace_buffer.iter() {
                    ui.horizontal(|ui| {
                        ui.monospace(format!(
                            "{:04X}: {} A:{:02X} X:{:02X} Y:{:02X} P:{}",
                            entry.pc,
                            entry.format_bytes(),
                            entry.a,
                            entry.x,
                            entry.y,
                            entry.format_flags(),
                        ));
                    });
                }
            });
    }

    fn render_memory(&mut self, ui: &mut egui::Ui, memory_reader: &dyn Fn(u16) -> u8) {
        ui.horizontal(|ui| {
            ui.label("Page:");
            ui.add(egui::DragValue::new(&mut self.memory_page).hexadecimal(2, false, true));
            
            ui.separator();
            ui.label("Go to:");
            let response = ui.text_edit_singleline(&mut self.goto_address);
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Ok(addr) = u16::from_str_radix(&self.goto_address, 16) {
                    self.memory_page = (addr >> 8) as u8;
                }
            }
        });

        let page_base = (self.memory_page as u16) << 8;
        
        egui::ScrollArea::vertical()
            .max_height(150.0)
            .show(ui, |ui| {
                for row in 0..16 {
                    let addr = page_base + (row * 16);
                    let mut line = format!("{:04X}: ", addr);
                    let mut ascii = String::new();
                    
                    for col in 0..16 {
                        let byte = memory_reader(addr + col);
                        line.push_str(&format!("{:02X} ", byte));
                        ascii.push(if byte >= 0x20 && byte < 0x7F {
                            byte as char
                        } else {
                            '.'
                        });
                    }
                    
                    ui.monospace(format!("{} {}", line, ascii));
                }
            });
    }

    fn render_stack(&self, ui: &mut egui::Ui, state: &CpuState, memory_reader: &dyn Fn(u16) -> u8) {
        ui.label("Stack");
        
        egui::ScrollArea::vertical()
            .max_height(150.0)
            .show(ui, |ui| {
                // Show from current SP up to $01FF
                let sp = state.sp;
                for offset in 0..16u8 {
                    let stack_addr = 0x0100u16 + sp.wrapping_add(offset + 1) as u16;
                    if stack_addr > 0x01FF {
                        break;
                    }
                    let value = memory_reader(stack_addr);
                    let marker = if offset == 0 { ">" } else { " " };
                    ui.monospace(format!("{} {:04X}: {:02X}", marker, stack_addr, value));
                }
            });
    }

    fn render_watches(&mut self, ui: &mut egui::Ui, memory_reader: &dyn Fn(u16) -> u8) {
        ui.horizontal(|ui| {
            ui.label("Watches");
            if ui.small_button("+").clicked() {
                if let Ok(addr) = u16::from_str_radix(&self.new_watch_addr, 16) {
                    let label = if self.new_watch_label.is_empty() {
                        format!("${:04X}", addr)
                    } else {
                        self.new_watch_label.clone()
                    };
                    self.add_watch(addr, 1, &label);
                    self.new_watch_addr.clear();
                    self.new_watch_label.clear();
                }
            }
        });
        
        ui.horizontal(|ui| {
            ui.label("Addr:");
            ui.add(egui::TextEdit::singleline(&mut self.new_watch_addr).desired_width(50.0));
            ui.label("Label:");
            ui.add(egui::TextEdit::singleline(&mut self.new_watch_label).desired_width(80.0));
        });

        let mut to_remove = None;
        for (idx, watch) in self.watches.iter().enumerate() {
            ui.horizontal(|ui| {
                let value = memory_reader(watch.address);
                ui.monospace(format!(
                    "{}: ${:04X} = {:02X} ({})",
                    watch.label_str(),
                    watch.address,
                    value,
                    value
                ));
                if ui.small_button("x").clicked() {
                    to_remove = Some(idx);
                }
            });
        }
        
        if let Some(idx) = to_remove {
            self.remove_watch(idx);
        }
    }
}

/// Current CPU state - passed to render each frame
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuState {
    pub pc: u16,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub p: u8,
    pub cycles: u64,
}

/// Format flags as compact string
fn format_flags_short(p: u8) -> String {
    let flags = Flags::from_bits_truncate(p);
    format!(
        "{}{}{}{}{}{}{}{}",
        if flags.contains(Flags::NEGATIVE) { 'N' } else { 'n' },
        if flags.contains(Flags::OVERFLOW) { 'V' } else { 'v' },
        '-',
        if flags.contains(Flags::BREAK) { 'B' } else { 'b' },
        if flags.contains(Flags::DECIMAL) { 'D' } else { 'd' },
        if flags.contains(Flags::IRQ_DISABLE) { 'I' } else { 'i' },
        if flags.contains(Flags::ZERO) { 'Z' } else { 'z' },
        if flags.contains(Flags::CARRY) { 'C' } else { 'c' },
    )
}
