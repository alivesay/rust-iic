use std::sync::Arc;
use std::time::{Duration, Instant};

use log::error;
use pixels::{Pixels, PixelsBuilder, ScalingMode, SurfaceTexture};
use shader_ui::ShaderParams;
use winit::{
    dpi::LogicalSize,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey},
    window::{Window, WindowButtons, WindowId},
};
#[cfg(not(target_os = "macos"))]
use winit::window::Fullscreen;

#[cfg(target_os = "macos")]
use winit::platform::macos::WindowExtMacOS;

use crate::cli::ShaderType;
use crate::config::Config;
use crate::audio_mixer::AudioControls;
use crate::cpu::Cpu;
use crate::cpu_monitor::{CpuMonitor, CpuState};
use crate::cpu_monitor_window::CpuMonitorWindow;
use crate::device::drive_audio::DriveAudioParams;
use crate::monitor::Monitor;
use crate::render::{
    blit_direct, blit_nearest, BlitRect, ContentRect, CrtRenderer,
    DriveIcons, DriveStatusInfo, LcdRenderer, PostProcessor, RendererInit,
    ToolbarAction, ToolbarLabels, render_toolbar_ui,
};
use crate::settings_window::{render_settings_window, SettingsState};

pub struct App {
    pub pixels: Option<Pixels<'static>>,
    pub window: Option<Arc<Window>>,
    pub cpu: Cpu,
    pub surface_width: u32,
    pub surface_height: u32,
    pub buffer_width: u32,
    pub buffer_height: u32,
    pub post_processor: Option<Box<dyn PostProcessor>>,
    pub shader_type: ShaderType,
    pub shader_start_time: Instant,
    pub power_on_time: Instant,
    pub modifiers: ModifiersState,
    pub last_cursor_pos: Option<(f64, f64)>,
    pub mouse_grabbed: bool,
    pub mouse_enabled: bool,
    pub show_toolbar: bool,
    pub is_fullscreen: bool,
    pub start_fullscreen: bool,
    pub last_drive_click: Option<(usize, Instant)>,
    // egui state for shader parameter UI
    pub egui_ctx: egui::Context,
    pub egui_state: Option<egui_winit::State>,
    pub egui_renderer: Option<egui_wgpu::Renderer>,
    pub shader_params: ShaderParams,
    pub show_shader_ui: bool,
    pub show_drive_audio_ui: bool,
    pub drive_audio_params: DriveAudioParams,

    pub config: Config,
    pub show_settings_window: bool,
    pub settings_state: SettingsState,

    pub audio_controls: Option<AudioControls>,

    pub pending_shader_change: Option<ShaderType>,
    pub cpu_monitor: CpuMonitor,
    pub monitor_window: Option<CpuMonitorWindow>,
    pub drive_icons: Option<DriveIcons>,
    pub toolbar_labels: Option<ToolbarLabels>,
    pub paused: bool,
    pub window_aspect_ratio: f64,
    pub last_resize_time: Option<Instant>,
    pub frame_progress: FrameProgress,
    pub last_memexp_flush: Instant,
    pub bbs_handle: Option<crate::bbs::BbsHandle>,

    pub gpu_perf: GpuPerf,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct GpuPerf {
    pub samples: u32,
    pub total_us: u64,
    pub max_us: u64,
}

impl GpuPerf {
    pub fn record(&mut self, us: u64) {
        self.samples = self.samples.saturating_add(1);
        self.total_us = self.total_us.saturating_add(us);
        if us > self.max_us {
            self.max_us = us;
        }
    }
    pub fn drain(&mut self) -> (f64, u64, u32) {
        let avg = if self.samples > 0 {
            self.total_us as f64 / self.samples as f64
        } else {
            0.0
        };
        let out = (avg, self.max_us, self.samples);
        self.samples = 0;
        self.total_us = 0;
        self.max_us = 0;
        out
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FrameProgress {
    pub active: bool,
    pub scanline: usize,
    pub cycles_run: u64,
    pub target_cycles: u64,
}

impl App {
    #[allow(dead_code)]
    pub fn new(cpu: Cpu, shader_type: ShaderType, start_fullscreen: bool, mouse_enabled: bool) -> Self {
        Self::new_with_config(cpu, shader_type, start_fullscreen, mouse_enabled, Config::default())
    }

    pub fn new_with_config(
        cpu: Cpu,
        shader_type: ShaderType,
        start_fullscreen: bool,
        mouse_enabled: bool,
        config: Config,
    ) -> Self {
        let (width, height) = cpu.bus.video.get_active_dimensions();
        let shader_params = config.shader.clone();
        let drive_audio_params = config.drive_audio.clone();
        Self {
            pixels: None,
            window: None,
            cpu,
            surface_width: width * 2,
            surface_height: height * 2,
            buffer_width: width * 2,
            buffer_height: height * 2,
            post_processor: None,
            shader_type,
            shader_start_time: Instant::now(),
            power_on_time: Instant::now().checked_sub(Duration::from_secs(5)).unwrap_or_else(Instant::now),
            modifiers: ModifiersState::default(),
            last_cursor_pos: None,
            mouse_grabbed: false,
            mouse_enabled,
            show_toolbar: false,
            is_fullscreen: false,
            start_fullscreen,
            last_drive_click: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            egui_renderer: None,
            shader_params,
            show_shader_ui: false,
            show_drive_audio_ui: false,
            drive_audio_params,
            config,
            show_settings_window: false,
            settings_state: SettingsState::default(),
            audio_controls: None,
            pending_shader_change: None,
            cpu_monitor: CpuMonitor::new(),
            monitor_window: None,
            drive_icons: None,
            toolbar_labels: None,
            paused: false,
            window_aspect_ratio: 1.0,
            last_resize_time: None,
            frame_progress: FrameProgress::default(),
            last_memexp_flush: Instant::now(),
            bbs_handle: None,
            gpu_perf: GpuPerf::default(),
        }
    }

    fn load_window_icon() -> Option<winit::window::Icon> {
        let icon_data = include_bytes!("../assets/disk2.png");
        let img = image::load_from_memory(icon_data).ok()?.into_rgba8();
        let (w, h) = img.dimensions();
        winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
    }

    #[cfg(target_os = "macos")]
    fn set_macos_dock_icon() {
        use objc2::MainThreadMarker;
        use objc2::AnyThread;
        use objc2_app_kit::{NSApplication, NSImage};
        use objc2_foundation::{NSData, NSSize};

        let icon_data = include_bytes!("../assets/disk2.png");
        let src = image::load_from_memory(icon_data).unwrap().into_rgba8();
        let scaled = image::imageops::resize(&src, 128, 128, image::imageops::FilterType::Nearest);
        let mut png_buf = std::io::Cursor::new(Vec::new());
        scaled.write_to(&mut png_buf, image::ImageFormat::Png).unwrap();

        unsafe {
            let mtm = MainThreadMarker::new_unchecked();
            let data = NSData::with_bytes(png_buf.get_ref());
            if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
                image.setSize(NSSize::new(128.0, 128.0));
                let app = NSApplication::sharedApplication(mtm);
                app.setApplicationIconImage(Some(&image));
            }
        }
    }

    pub fn flush_disks(&mut self) {
        self.cpu.bus.iou.iwm.eject_disk(0);
        self.cpu.bus.iou.iwm.eject_disk(1);
        self.cpu.bus.iou.iwm.smartport.flush_all();
        // persist battery-backed RAM expansion image (RAM Express II+)
        self.cpu.bus.iou.memexp.save_to_file(&crate::config::memexp_path());
        self.last_memexp_flush = Instant::now();
    }

    pub fn maybe_flush_memexp(&mut self) {
        const INTERVAL: Duration = Duration::from_secs(5);
        if !self.cpu.bus.iou.memexp.is_dirty() {
            return;
        }
        if self.last_memexp_flush.elapsed() < INTERVAL {
            return;
        }
        self.cpu.bus.iou.memexp.flush_if_dirty(&crate::config::memexp_path());
        self.last_memexp_flush = Instant::now();
    }

    pub fn poll_bbs_events(&mut self) {
        let Some(handle) = self.bbs_handle.as_ref() else { return };
        loop {
            match handle.events.try_recv() {
                Ok(crate::bbs::BbsEvent::DownloadCompleted { name, path, .. }) => {
                    log::info!("bbs: saved {} -> {}", name, path.display());
                }
                Ok(crate::bbs::BbsEvent::DownloadFailed { name, error }) => {
                    log::warn!("bbs: download {} failed: {}", name, error);
                }
                Ok(crate::bbs::BbsEvent::Disconnected) => {}
                Err(_) => break,
            }
        }
    }

    pub fn boot_into_bbs(&mut self) {
        if self.bbs_handle.is_none() {
            match crate::bbs::start() {
                Ok(handle) => {
                    println!("bbs   {:>12} {:>8}    {}", "BBS", "ONLINE", handle.addr);
                    self.bbs_handle = Some(handle);
                }
                Err(e) => {
                    eprintln!("bbs   {:>12} {:>8}    {}", "BBS", "ERROR", e);
                    return;
                }
            }
        }

        let port = match self.bbs_handle.as_ref() {
            Some(h) => h.addr.port(),
            None => return,
        };


        self.cpu.bus.iou.iwm.eject_disk(0);
        self.cpu.bus.iou.iwm.eject_disk(1);
        self.cpu.bus.iou.iwm.smartport.eject_all();

        crate::bbs::jumpstart_term(&mut self.cpu);
        println!("bbs   {:>12} {:>8}    rustiic_term @ $0801", "TERM", "RAMBOOT");

        self.cpu.bus.iou.scc.ch_a.line_baud = 115200;
        self.cpu.bus.iou.set_zip_enabled(true);
        let addr = format!("127.0.0.1:{}", port);
        if let Err(e) = self.cpu.bus.iou.scc.ch_a.dial_loopback(port) {
            eprintln!("bbs   {:>12} {:>8}    dial: {}", "BBS", "ERROR", e);
        } else {
            println!("bbs   {:>12} {:>8}    {}", "BBS_DIAL", "CONNECT", addr);
        }


        println!("Ctrl+F11: rebooting into BBS");
        self.cpu.bus.write_byte(0x03F4, 0x00);
        self.cpu.reset();
    }

    pub fn apply_audio_config(&self) {
        if let Some(c) = self.audio_controls.as_ref() {
            let a = &self.config.audio;
            c.set_muted(a.muted);
            c.set_master(a.master);
            c.set_speaker(a.speaker);
            c.set_mockingboard1(a.mockingboard1);
            c.set_mockingboard2(a.mockingboard2);
            c.set_drive(a.drive);
        }
    }

    pub fn rebuild_render_pipeline(&mut self) {
        let Some(window) = self.window.clone() else { return; };

        self.cpu.bus.video.shader_enabled = self.shader_type != ShaderType::None;
        self.cpu.bus.video.force_neutral_mono = self.shader_type == ShaderType::Lcd;

        // recompute window aspect ratio
        let (src_w_native, _) = self.cpu.bus.video.get_dimensions();
        let native_h = self.cpu.bus.video.get_dimensions().1 / 2;
        let aspect_correction = match self.shader_type {
            ShaderType::Lcd => LcdRenderer::LCD_ASPECT_CORRECTION as f64,
            _ => CrtRenderer::CRT_ASPECT_CORRECTION as f64,
        };
        let base_w = src_w_native as f64;
        let base_h = native_h as f64 * 2.0 * aspect_correction;
        let new_aspect = base_w / base_h;
        if (new_aspect - self.window_aspect_ratio).abs() > 0.001 && !self.is_fullscreen {
            self.window_aspect_ratio = new_aspect;

            let scale = window.scale_factor();
            let cur_w_logical = self.surface_width as f64 / scale;
            let new_h_logical = cur_w_logical / new_aspect;
            let _ = window.request_inner_size(LogicalSize::new(cur_w_logical, new_h_logical));
        } else {
            self.window_aspect_ratio = new_aspect;
        }

        self.post_processor = None;
        self.egui_renderer = None;
        self.egui_state = None;
        self.pixels = None;

        let surface_w = self.surface_width.max(1);
        let surface_h = self.surface_height.max(1);
        let (src_w, src_h) = self.cpu.bus.video.get_dimensions();
        let (active_w, active_h) = self.cpu.bus.video.get_active_dimensions();

        let (buf_w, buf_h) = match self.shader_type {
            ShaderType::Crt | ShaderType::None => (src_w, src_h),
            ShaderType::Lcd => (surface_w, surface_h),
        };
        self.buffer_width = buf_w;
        self.buffer_height = buf_h;

        let surface_texture = SurfaceTexture::new(surface_w, surface_h, window.clone());
        let mut pixels = match PixelsBuilder::new(buf_w, buf_h, surface_texture)
            .texture_format(wgpu::TextureFormat::Rgba8Unorm)
            .present_mode(wgpu::PresentMode::Mailbox)
            .build()
        {
            Ok(p) => p,
            Err(err) => {
                error!("rebuild_render_pipeline: pixels::new failed: {}", err);
                return;
            }
        };

        let mode = if self.shader_type == ShaderType::Crt {
            ScalingMode::PixelPerfect
        } else {
            ScalingMode::Fill
        };
        pixels.set_scaling_mode(mode);
        pixels.clear_color(wgpu::Color::BLACK);
        let surface_format = pixels.surface_texture_format();

        self.post_processor = match self.shader_type {
            ShaderType::Crt => Some(Box::new(CrtRenderer::new(RendererInit {
                device: pixels.device(),
                surface_width: surface_w,
                surface_height: surface_h,
                buffer_width: buf_w,
                buffer_height: buf_h,
                bar_height: 0,
                source_width: active_w as f32,
                source_height: active_h as f32,
                surface_format,
            })) as Box<dyn PostProcessor>),
            ShaderType::Lcd => Some(Box::new(LcdRenderer::new(RendererInit {
                device: pixels.device(),
                surface_width: surface_w,
                surface_height: surface_h,
                buffer_width: buf_w,
                buffer_height: buf_h,
                bar_height: 0,
                source_width: active_w as f32,
                source_height: active_h as f32,
                surface_format,
            })) as Box<dyn PostProcessor>),
            ShaderType::None => None,
        };
        if let Some(pp) = &mut self.post_processor {
            pp.resize(pixels.device(), pixels.queue(), surface_w, surface_h);
        }


        self.egui_ctx = egui::Context::default();

        self.drive_icons = None;
        self.toolbar_labels = None;

        let egui_state = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            Some(pixels.device().limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(
            pixels.device(),
            surface_format,
            Default::default(),
        );
        self.egui_state = Some(egui_state);
        self.egui_renderer = Some(egui_renderer);
        self.pixels = Some(pixels);
        self.shader_start_time = Instant::now();
        window.request_redraw();
        log::info!("rebuild_render_pipeline: shader = {:?}", self.shader_type);
    }

    pub fn snap_aspect_ratio(&mut self) {
        if let Some(last_resize) = self.last_resize_time {
            if last_resize.elapsed() >= Duration::from_millis(150) {
                self.last_resize_time = None;

                let target_ratio = self.window_aspect_ratio;
                let current_ratio = self.surface_width as f64 / self.surface_height as f64;

                if (current_ratio - target_ratio).abs() > 0.01 {
                    // Keep the wider dimension, adjust the other
                    let (new_w, new_h) = if current_ratio > target_ratio {
                        // Too wide, shrink width to match height
                        ((self.surface_height as f64 * target_ratio).round() as u32, self.surface_height)
                    } else {
                        // Too tall, shrink height to match width
                        (self.surface_width, (self.surface_width as f64 / target_ratio).round() as u32)
                    };

                    if let Some(window) = &self.window {
                        let scale = window.scale_factor();
                        let logical = LogicalSize::new(new_w as f64 / scale, new_h as f64 / scale);
                        let _ = window.request_inner_size(logical);
                    }
                }
            }
        }
    }
}

fn render_drive_audio_ui(ctx: &egui::Context, params: &mut DriveAudioParams, open: &mut bool) -> DriveAudioUiResult {
    let mut changed = false;
    let mut save_clicked = false;
    let p = params;
    
    egui::Window::new("Drive Audio Settings")
        .open(open)
        .resizable(true)
        .default_width(320.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Master");
                changed |= ui.add(egui::Slider::new(&mut p.master_volume, 0.0..=4.0).text("Master Volume")).changed();
                changed |= ui.checkbox(&mut p.enabled, "Enabled").changed();

                ui.separator();
                ui.heading("Stepper Click");
                changed |= ui.add(egui::Slider::new(&mut p.click_volume, 0.0..=1.0).text("Volume")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.click_noise_decay_ms, 1.0..=30.0).text("Noise Decay (ms)")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.click_filter_freq, 500.0..=6000.0).text("Noise Filter (Hz)")).changed();
                ui.label("Body Clack (multi-stage impact)");
                changed |= ui.add(egui::Slider::new(&mut p.click_body_freq, 200.0..=1200.0).text("Body Freq (Hz)")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.click_body_decay_ms, 2.0..=30.0).text("Body Decay (ms)")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.click_body_mix, 0.0..=1.0).text("Body Mix")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.click_attack_mix, 0.0..=1.5).text("Attack Mix")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.click_attack_decay_ms, 0.3..=5.0).text("Attack Decay (ms)")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.click_pitch_sweep, 1.0..=2.0).text("Pitch Sweep")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.click_pitch_sweep_ms, 1.0..=10.0).text("Sweep Time (ms)")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.click_harmonic_mix, 0.0..=1.0).text("Harmonic Mix")).changed();
                ui.label("Metallic Tick (~1500 Hz)");
                changed |= ui.add(egui::Slider::new(&mut p.click_tick_freq, 800.0..=3000.0).text("Tick Freq (Hz)")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.click_tick_decay_ms, 2.0..=20.0).text("Tick Decay (ms)")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.click_tick_mix, 0.0..=1.0).text("Tick Mix")).changed();
                ui.label("Crunch (high-freq grit)");
                changed |= ui.add(egui::Slider::new(&mut p.click_crunch_decay_ms, 1.0..=15.0).text("Crunch Decay (ms)")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.click_crunch_freq, 1000.0..=8000.0).text("Crunch Freq (Hz)")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.click_crunch_mix, 0.0..=1.0).text("Crunch Mix")).changed();

                ui.separator();
                ui.heading("Motor Relay Click");
                changed |= ui.add(egui::Slider::new(&mut p.relay_volume, 0.0..=1.0).text("Volume")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.relay_freq, 400.0..=1200.0).text("Freq (Hz)")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.relay_decay_ms, 2.0..=15.0).text("Decay (ms)")).changed();

                ui.separator();
                ui.heading("Motor");
                changed |= ui.add(egui::Slider::new(&mut p.motor_volume, 0.0..=0.1).text("Volume")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.motor_filter_freq, 50.0..=500.0).text("Filter (Hz)")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.motor_cog_freq, 10.0..=100.0).text("Cog Freq (Hz)")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.motor_cog_mix, 0.0..=1.0).text("Cog Mix")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.motor_spinup_ms, 50.0..=500.0).text("Spinup (ms)")).changed();
                changed |= ui.add(egui::Slider::new(&mut p.motor_spindown_ms, 100.0..=800.0).text("Spindown (ms)")).changed();

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Reset Defaults").clicked() {
                        *p = DriveAudioParams::default();
                        changed = true;
                    }
                    if ui.button("Print Values").clicked() {
                        println!("--- Drive Audio Parameters ---");
                        println!("master_volume: {:.2}", p.master_volume);
                        println!("click_volume: {:.2}", p.click_volume);
                        println!("click_noise_decay_ms: {:.1}", p.click_noise_decay_ms);
                        println!("click_filter_freq: {:.0}", p.click_filter_freq);
                        println!("click_body_freq: {:.0}", p.click_body_freq);
                        println!("click_body_decay_ms: {:.1}", p.click_body_decay_ms);
                        println!("click_body_mix: {:.2}", p.click_body_mix);
                        println!("click_attack_mix: {:.2}", p.click_attack_mix);
                        println!("click_attack_decay_ms: {:.1}", p.click_attack_decay_ms);
                        println!("click_pitch_sweep: {:.2}", p.click_pitch_sweep);
                        println!("click_pitch_sweep_ms: {:.1}", p.click_pitch_sweep_ms);
                        println!("click_harmonic_mix: {:.2}", p.click_harmonic_mix);
                        println!("click_tick_freq: {:.0}", p.click_tick_freq);
                        println!("click_tick_decay_ms: {:.1}", p.click_tick_decay_ms);
                        println!("click_tick_mix: {:.2}", p.click_tick_mix);
                        println!("click_crunch_decay_ms: {:.1}", p.click_crunch_decay_ms);
                        println!("click_crunch_freq: {:.0}", p.click_crunch_freq);
                        println!("click_crunch_mix: {:.2}", p.click_crunch_mix);
                        println!("relay_volume: {:.2}", p.relay_volume);
                        println!("relay_freq: {:.0}", p.relay_freq);
                        println!("relay_decay_ms: {:.1}", p.relay_decay_ms);
                        println!("motor_volume: {:.3}", p.motor_volume);
                        println!("motor_filter_freq: {:.0}", p.motor_filter_freq);
                        println!("motor_cog_freq: {:.0}", p.motor_cog_freq);
                        println!("motor_cog_mix: {:.2}", p.motor_cog_mix);
                        println!("motor_spinup_ms: {:.0}", p.motor_spinup_ms);
                        println!("motor_spindown_ms: {:.0}", p.motor_spindown_ms);
                        println!("------------------------------");
                    }
                    if ui.button("Save to config").clicked() {
                        save_clicked = true;
                    }
                });
            });
        });

    DriveAudioUiResult { changed, save_clicked }
}

#[derive(Default, Clone, Copy)]
pub struct DriveAudioUiResult {
    pub changed: bool,
    pub save_clicked: bool,
}

impl winit::application::ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let (buf_w, buf_h) = self.cpu.bus.video.get_dimensions();
        let native_h = buf_h / 2;

        let aspect = if self.shader_type == ShaderType::Lcd {
            LcdRenderer::LCD_ASPECT_CORRECTION as f64
        } else {
            CrtRenderer::CRT_ASPECT_CORRECTION as f64
        };
        
        let base_w = buf_w as f64;
        let base_h = native_h as f64 * 2.0 * aspect;

        // Pick the largest integer scale that fits within 80% of the monitor (logical points)
        let scale = if let Some(monitor) = event_loop.primary_monitor().or_else(|| event_loop.available_monitors().next()) {
            let monitor_size = monitor.size();
            let dpi_scale = monitor.scale_factor();
            let logical_w = monitor_size.width as f64 / dpi_scale;
            let logical_h = monitor_size.height as f64 / dpi_scale;
            let max_w = logical_w * 0.80;
            let max_h = logical_h * 0.80;
            let max_scale_w = (max_w / base_w).floor() as u32;
            let max_scale_h = (max_h / base_h).floor() as u32;
            max_scale_w.min(max_scale_h).max(1)
        } else {
            2
        };

        let win_w = base_w * scale as f64;
        let win_h = base_h * scale as f64;

        self.window_aspect_ratio = base_w / base_h;

        let window_buttons = WindowButtons::CLOSE | WindowButtons::MINIMIZE;

        #[cfg(not(target_os = "macos"))]
        let initial_fullscreen = if self.start_fullscreen {
            Some(Fullscreen::Borderless(None))
        } else {
            None
        };

        #[cfg(target_os = "macos")]
        let attrs = Window::default_attributes()
            .with_title("Apple //c")
            .with_inner_size(LogicalSize::new(win_w, win_h))
            .with_min_inner_size(LogicalSize::new(base_w, base_h))
            .with_enabled_buttons(window_buttons)
            .with_window_icon(Self::load_window_icon());
        #[cfg(not(target_os = "macos"))]
        let attrs = {
            let mut attrs = Window::default_attributes()
                .with_title("Apple //c")
                .with_inner_size(LogicalSize::new(win_w, win_h))
                .with_min_inner_size(LogicalSize::new(base_w, base_h))
                .with_enabled_buttons(window_buttons)
                .with_window_icon(Self::load_window_icon())
                .with_fullscreen(initial_fullscreen);
            if self.start_fullscreen {
                attrs = attrs.with_decorations(false);
            }
            attrs
        };

        let window = Arc::new(event_loop.create_window(attrs).unwrap());

        self.window = Some(window.clone());

        #[cfg(target_os = "macos")]
        Self::set_macos_dock_icon();

        #[cfg(target_os = "macos")]
        if self.start_fullscreen {
            window.set_decorations(false);
            window.set_has_shadow(false);
            let _ = window.set_simple_fullscreen(true);
            self.is_fullscreen = true;
        }

        #[cfg(not(target_os = "macos"))]
        if self.start_fullscreen {
            self.is_fullscreen = true;
        }

        let scale_factor = window.scale_factor();
        let window_size = window.inner_size();

        let phys_w = (win_w * scale_factor) as u32;
        let phys_h = (win_h * scale_factor) as u32;
        #[cfg_attr(target_os = "macos", allow(unused_mut))]
        let mut surface_w = window_size.width.max(phys_w);
        #[cfg_attr(target_os = "macos", allow(unused_mut))]
        let mut surface_h = window_size.height.max(phys_h);

        #[cfg(not(target_os = "macos"))]
        if self.start_fullscreen {
            if let Some(monitor) = window
                .current_monitor()
                .or_else(|| event_loop.primary_monitor())
                .or_else(|| event_loop.available_monitors().next())
            {
                let ms = monitor.size();
                if ms.width > 0 && ms.height > 0 {
                    surface_w = ms.width;
                    surface_h = ms.height;
                }
            }
        }

        self.surface_width = surface_w;
        self.surface_height = surface_h;
        
        let (src_w, src_h) = self.cpu.bus.video.get_dimensions();
        let (active_w, active_h) = self.cpu.bus.video.get_active_dimensions();
        
        let (buf_w, buf_h) = match self.shader_type {
            ShaderType::Crt | ShaderType::None => (src_w, src_h),
            ShaderType::Lcd => (surface_w, surface_h),
        };
        self.buffer_width = buf_w;
        self.buffer_height = buf_h;

        let surface_texture =
            SurfaceTexture::new(surface_w, surface_h, window.clone());

        self.pixels = match PixelsBuilder::new(buf_w, buf_h, surface_texture)
            .texture_format(wgpu::TextureFormat::Rgba8Unorm)
            // non-blocking present
            .present_mode(wgpu::PresentMode::Mailbox)
            .build() {
            Ok(mut pixels) => {
                let mode = if self.shader_type == ShaderType::Crt {
                    ScalingMode::PixelPerfect
                } else {
                    ScalingMode::Fill
                };
                pixels.set_scaling_mode(mode);
                pixels.clear_color(wgpu::Color::BLACK);
                let surface_format = pixels.surface_texture_format();

                if self.shader_type != ShaderType::None {
                    self.post_processor = match self.shader_type {
                        ShaderType::Crt => Some(Box::new(CrtRenderer::new(RendererInit {
                            device: pixels.device(),
                            surface_width: surface_w,
                            surface_height: surface_h,
                            buffer_width: buf_w,
                            buffer_height: buf_h,
                            bar_height: 0,
                            source_width: active_w as f32,
                            source_height: active_h as f32,
                            surface_format,
                        })) as Box<dyn PostProcessor>),
                        ShaderType::Lcd => Some(Box::new(LcdRenderer::new(RendererInit {
                            device: pixels.device(),
                            surface_width: surface_w,
                            surface_height: surface_h,
                            buffer_width: buf_w,
                            buffer_height: buf_h,
                            bar_height: 0,
                            source_width: active_w as f32,
                            source_height: active_h as f32,
                            surface_format,
                        })) as Box<dyn PostProcessor>),
                        ShaderType::None => None,
                    };

                    if let Some(pp) = &mut self.post_processor {
                        pp.resize(pixels.device(), pixels.queue(), surface_w, surface_h);
                    }
                }

                let egui_state = egui_winit::State::new(
                    self.egui_ctx.clone(),
                    egui::ViewportId::ROOT,
                    window.as_ref(),
                    Some(window.scale_factor() as f32),
                    None,
                    Some(pixels.device().limits().max_texture_dimension_2d as usize),
                );
                let egui_renderer = egui_wgpu::Renderer::new(
                    pixels.device(),
                    surface_format,
                    Default::default(),
                );
                self.egui_state = Some(egui_state);
                self.egui_renderer = Some(egui_renderer);

                window.request_redraw();
                Some(pixels)
            }
            Err(err) => {
                error!("pixels::new failed: {}", err);
                event_loop.exit();
                None
            }
        };
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(mw) = self.monitor_window.as_mut() {
            if mw.id() == id {
                self.handle_monitor_event(event);
                return;
            }
        }

        let egui_consumed = if self.show_shader_ui || self.show_drive_audio_ui || self.show_toolbar || self.show_settings_window {
            if let Some(egui_state) = self.egui_state.as_mut() {
                if let Some(window) = self.window.as_ref() {
                    let response = egui_state.on_window_event(window.as_ref(), &event);
                    response.consumed
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        match event {
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }

            WindowEvent::CloseRequested => {
                println!("Flushing disks before exit...");
                self.flush_disks();
                event_loop.exit();
            }

            WindowEvent::Focused(focused) => {
                if focused {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                } else {
                    self.release_mouse();
                }
                self.modifiers = ModifiersState::empty();
            }

            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0
                    && (size.width != self.surface_width || size.height != self.surface_height) {
                        self.surface_width = size.width;
                        self.surface_height = size.height;

                        if let Some(pixels) = self.pixels.as_mut() {
                            let _ = pixels.resize_surface(size.width, size.height);

                            if self.shader_type == ShaderType::Lcd {
                                self.buffer_width = size.width;
                                self.buffer_height = size.height;
                                let _ = pixels.resize_buffer(size.width, size.height);
                            }

                            if let Some(pp) = self.post_processor.as_mut() {
                                pp.resize(pixels.device(), pixels.queue(), size.width, size.height);
                            }
                        }

                        // Mark resize timestamp for deferred aspect-ratio snap
                        if !self.is_fullscreen {
                            self.last_resize_time = Some(Instant::now());
                        }

                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
            }

            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            WindowEvent::RedrawRequested => {
                self.handle_redraw();
            }

            WindowEvent::CursorEntered { .. } => {}

            WindowEvent::CursorLeft { .. } => {}

            WindowEvent::CursorMoved { position, .. } => {
                self.last_cursor_pos = Some((position.x, position.y));
            }

            WindowEvent::MouseInput { state, button, .. } => {
                if egui_consumed {
                    return;
                }
                self.handle_mouse_input(state, button);
            }

            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_keyboard_input(event_loop, &event, egui_consumed);
            }

            _ => (),
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        if let winit::event::DeviceEvent::MouseMotion { delta } = event {
            if !self.mouse_grabbed {
                return;
            }

            let Some(window) = self.window.as_ref() else { return };
            let win = window.inner_size();
            if win.width == 0 || win.height == 0 {
                return;
            }
            let sf = window.scale_factor();
            let win_w_logical = win.width as f64 / sf;
            let win_h_logical = win.height as f64 / sf;

            const IIC_CLAMP_RANGE: f64 = 1023.0;

            const SENSITIVITY: f64 = 1.0;
            let scale_x = SENSITIVITY * IIC_CLAMP_RANGE / win_w_logical;
            let scale_y = SENSITIVITY * IIC_CLAMP_RANGE / win_h_logical;
            self.cpu
                .bus
                .iou
                .mouse
                .add_delta(delta.0 * scale_x, delta.1 * scale_y);
        }
    }
}

impl App {
    fn paste_clipboard_to_keyboard(&mut self) {
        let text = match arboard::Clipboard::new().and_then(|mut cb| cb.get_text()) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Clipboard paste failed: {e}");
                return;
            }
        };
        if text.is_empty() {
            return;
        }
        let mut bytes: Vec<u8> = Vec::with_capacity(text.len());
        let mut prev_was_cr = false;
        for c in text.chars() {
            let b: Option<u8> = match c {
                '\r' => {
                    prev_was_cr = true;
                    Some(0x0D)
                }
                '\n' => {
                    if prev_was_cr {
                        prev_was_cr = false;
                        None // swallow LF half of CRLF
                    } else {
                        Some(0x0D)
                    }
                }
                '\t' => {
                    prev_was_cr = false;
                    Some(b' ')
                }
                c if (' '..='~').contains(&c) => {
                    prev_was_cr = false;
                    Some(c.to_ascii_uppercase() as u8)
                }
                _ => {
                    prev_was_cr = false;
                    None
                }
            };
            if let Some(b) = b {
                bytes.push(b);
            }
        }
        if bytes.is_empty() {
            return;
        }
        println!("Pasting {} byte(s) to keyboard", bytes.len());
        self.cpu.bus.iou.keyboard.paste_bytes(&bytes);
    }

    pub fn toggle_monitor_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.monitor_window.is_some() {
            self.monitor_window = None;
            self.cpu_monitor.visible = false;
            self.cpu_monitor.enabled = false;
            self.cpu_monitor.paused = false;
            self.cpu_monitor.on_window_closed();
            self.cpu.capture_trace = false;
            return;
        }
        match CpuMonitorWindow::new(event_loop) {
            Ok(mw) => {
                self.monitor_window = Some(mw);
                self.cpu_monitor.visible = true;
                self.cpu_monitor.enabled = true;
                self.cpu.capture_trace = true;
                self.release_mouse();
            }
            Err(e) => {
                error!("Failed to open CPU monitor window: {e}");
            }
        }
    }

    fn handle_monitor_event(&mut self, event: WindowEvent) {
        if let Some(mw) = self.monitor_window.as_mut() {
            let _consumed = mw.on_window_event(&event);
        }

        match event {
            WindowEvent::CloseRequested => {
                self.monitor_window = None;
                self.cpu_monitor.visible = false;
                self.cpu_monitor.enabled = false;
                self.cpu_monitor.paused = false;
                self.cpu_monitor.on_window_closed();
                self.cpu.capture_trace = false;
            }
            WindowEvent::Resized(size) => {
                if let Some(mw) = self.monitor_window.as_mut() {
                    mw.resize(size.width, size.height);
                    mw.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(mw) = self.monitor_window.as_ref() {
                    mw.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                self.handle_monitor_redraw();
            }
            _ => {}
        }
    }

    fn handle_monitor_redraw(&mut self) {
        // Build per-frame snapshots for the monitor UI.
        let cpu_state = CpuState {
            pc: self.cpu.pc,
            a: self.cpu.regs.a,
            x: self.cpu.regs.x,
            y: self.cpu.regs.y,
            sp: self.cpu.regs.sp,
            p: self.cpu.p.bits(),
            cycles: self.cpu.cycles,
        };

        let mut memory_snapshot = [0u8; 768];
        for (i, slot) in memory_snapshot[..256].iter_mut().enumerate() {
            *slot = self.cpu.bus.read_byte(0x0100 + i as u16);
        }
        let mem_page = self.cpu_monitor.memory.page;
        let page_base = (mem_page as u16) << 8;
        for (i, slot) in memory_snapshot[256..512].iter_mut().enumerate() {
            *slot = self.cpu.bus.read_byte(page_base + i as u16);
        }
        let pc_base = cpu_state.pc.wrapping_sub(32) & 0xFF00;
        for (i, slot) in memory_snapshot[512..768].iter_mut().enumerate() {
            *slot = self.cpu.bus.read_byte(pc_base.wrapping_add(i as u16));
        }

        let iou_snapshot = {
            let iou = &self.cpu.bus.iou;
            let mut softswitches = [None; 256];
            for (i, slot) in softswitches.iter_mut().enumerate() {
                *slot = iou.peek_softswitch(0xC000 + i as u16);
            }
            // Mirror the tail of the IOU access log into the snapshot so the
            // monitor panel can render it without holding a borrow into the
            // emulator. `recent_accesses` is fixed-size to keep IouSnapshot
            // `Copy`; `recent_access_count` tells the renderer how many of
            // those slots are valid (newest last).
            let mut recent_accesses =
                [crate::cpu_monitor::IouAccessSample::default(); 32];
            let log_len = iou.access_log.len();
            let take = log_len.min(recent_accesses.len());
            let start = log_len - take;
            for (dst, entry) in recent_accesses
                .iter_mut()
                .zip(iou.access_log.iter().skip(start))
            {
                *dst = crate::cpu_monitor::IouAccessSample {
                    addr: entry.addr,
                    pc: entry.pc,
                    cycle: entry.cycle,
                    value: entry.value,
                    write: entry.write,
                };
            }
            let kbd = iou.keyboard.debug_state();
            crate::cpu_monitor::IouSnapshot {
                mem_state: iou.mem_state.get(),
                video_mode: iou.video_mode.get(),
                is_80store: iou.is_80store.get(),
                ioudis: iou.ioudis.get(),
                col80_switch: iou.col80_switch,
                disk35_mode: iou.disk35_mode,
                self_test: iou.self_test,
                scan_cycle: iou.scan_cycle,
                floating_bus: iou.floating_bus,
                irq_pending: self.cpu.bus.interrupts.irq,
                nmi_pending: self.cpu.bus.interrupts.nmi,
                mouse_x_int: iou.mouse.x_int.get(),
                mouse_y_int: iou.mouse.y_int.get(),
                mouse_vbl_int: iou.mouse.vbl_int.get(),
                mouse_xy_mask: iou.mouse.xy_mask.get(),
                mouse_vbl_mask: iou.mouse.vbl_mask.get(),
                mouse_x: iou.mouse.x.get(),
                mouse_y: iou.mouse.y.get(),
                mouse_button0: iou.mouse.button0.get(),
                mouse_button1: iou.mouse.button1.get(),
                kbd_last_key: kbd.0,
                kbd_strobe: kbd.1,
                kbd_queued: kbd.2 as u16,
                kbd_held: kbd.3 as u16,
                scc_crossloop: iou.scc.crossloop,
                scc_a: iou.scc.ch_a.debug_state(),
                scc_b: iou.scc.ch_b.debug_state(),
                softswitches,
                recent_accesses,
                recent_access_count: take as u8,
            }
        };

        // Per-frame Devices snapshot: drive activity LEDs + audio scopes.
        let devices_snapshot = {
            let iou = &self.cpu.bus.iou;
            let mut drive_active = [false; 4];
            let mut drive_present = [false; 4];
            let mut drive_write_protect = [false; 4];
            let mut drive_head_qt = [0u16; 4];
            for d in 0..2 {
                let (has, act, wp) = iou.iwm.drive_status(d);
                drive_present[d] = has;
                drive_active[d] = act;
                drive_write_protect[d] = wp;
                drive_head_qt[d] = iou.iwm.drive_head_qt(d);
            }
            for d in 0..2 {
                let (has, act, wp) = iou.iwm.drive_status_35(d);
                drive_present[d + 2] = has;
                drive_active[d + 2] = act;
                drive_write_protect[d + 2] = wp;
            }
            let (iwm_motor_on, iwm_motor_on35, iwm_drive_select, iwm_phases,
                 iwm_write_mode, iwm_head35) = iou.iwm.debug_state();

            const SCOPE_FRAMES: usize = 1024;
            let mut speaker_scope = Vec::with_capacity(SCOPE_FRAMES);
            iou.speaker.scope_snapshot(&mut speaker_scope, SCOPE_FRAMES);

            let mut mb1 = Vec::new();
            if iou.mockingboard.is_enabled() {
                iou.mockingboard.scope_snapshot(&mut mb1, SCOPE_FRAMES * 2);
            }
            let mut mb2 = Vec::new();
            if iou.mockingboard2.is_enabled() {
                iou.mockingboard2.scope_snapshot(&mut mb2, SCOPE_FRAMES * 2);
            }

            crate::cpu_monitor::DevicesSnapshot {
                drive_active,
                drive_present,
                drive_write_protect,
                drive_head_qt,
                iwm_motor_on,
                iwm_motor_on35,
                iwm_drive_select,
                iwm_phases,
                iwm_write_mode,
                iwm_head35,
                speaker_scope,
                mockingboard1_scope: mb1,
                mockingboard2_scope: mb2,
                mockingboard1_enabled: iou.mockingboard.is_enabled(),
                mockingboard2_enabled: iou.mockingboard2.is_enabled(),
            }
        };

        let mut mw = self.monitor_window.take();

        let (fb_w, fb_h) = self.cpu.bus.video.get_dimensions();
        let fb_pixels: Vec<u8> = if self.frame_progress.active && self.frame_progress.scanline > 0 {
            let scanline = self.frame_progress.scanline;
            self.cpu.video_compose_monitor_partial(scanline).to_vec()
        } else {
            self.cpu.bus.video.get_pixels().to_vec()
        };
        let fb_raw_pixels: Vec<u8> = self.cpu.bus.video.get_raw_pixels().to_vec();
        if let Some(ref mut mw) = mw {
            let cpu_monitor = &mut self.cpu_monitor;
            mw.redraw(|ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(ctx.style().visuals.panel_fill))
                    .show(ctx, |ui| {
                        let memory_reader = |addr: u16| -> u8 {
                            if (0x0100..0x0200).contains(&addr) {
                                memory_snapshot[(addr - 0x0100) as usize]
                            } else if addr >= page_base && addr < page_base + 256 {
                                memory_snapshot[256 + (addr - page_base) as usize]
                            } else if addr >= pc_base && addr < pc_base.wrapping_add(256) {
                                memory_snapshot[512 + addr.wrapping_sub(pc_base) as usize]
                            } else {
                                0x00
                            }
                        };
                        let fb_view = crate::cpu_monitor::FramebufferView {
                            pixels: &fb_pixels,
                            width: fb_w,
                            height: fb_h,
                        };
                        let fb_raw_view = crate::cpu_monitor::FramebufferView {
                            pixels: &fb_raw_pixels,
                            width: fb_w,
                            height: fb_h,
                        };
                        cpu_monitor.render_inline(
                            ui,
                            crate::cpu_monitor::MonitorFrame {
                                cpu_state: &cpu_state,
                                iou: &iou_snapshot,
                                devices: &devices_snapshot,
                                memory_reader: &memory_reader,
                                framebuffer: Some(fb_view),
                                framebuffer_raw: Some(fb_raw_view),
                            },
                        );
                    });
            });
        }
        self.monitor_window = mw;
    }

    fn handle_redraw(&mut self) {
        if let Some(new_ty) = self.pending_shader_change.take() {
            self.shader_type = new_ty;
            self.rebuild_render_pipeline();
        }

        if let Some((drive, click_time)) = self.last_drive_click {
            if click_time.elapsed() >= Duration::from_millis(400) {
                self.last_drive_click = None;
                
                let file = if drive < 2 {
                    rfd::FileDialog::new()
                        .add_filter(
                            "5.25\" Disk Image",
                            &["woz", "dsk", "do", "po", "d13", "nib", "nb2", "2mg"],
                        )
                        .pick_file()
                } else {
                    rfd::FileDialog::new()
                        .add_filter("3.5\" Disk Image", &["po", "2mg", "2img"])
                        .pick_file()
                };
                
                if let Some(path) = file {
                    println!("Loading disk into drive {}: {}", drive + 1, path.display());
                    let result = match drive {
                        0 => self.cpu.bus.iou.iwm.load_disk(&path),
                        1 => self.cpu.bus.iou.iwm.load_disk2(&path),
                        2 => self.cpu.bus.iou.iwm.load_disk35(&path),
                        3 => self.cpu.bus.iou.iwm.load_disk35_drive(1, &path),
                        _ => Ok(()),
                    };
                    if let Err(e) = result {
                        println!("Error loading disk: {}", e);
                    }
                }
            }
        }

        if let Some(pixels) = self.pixels.as_mut() {
            if let Some(window) = &self.window {
                let size = window.inner_size();
                if size.width > 0 && size.height > 0 
                    && (size.width != self.surface_width || size.height != self.surface_height) 
                {
                    self.surface_width = size.width;
                    self.surface_height = size.height;
                    let _ = pixels.resize_surface(size.width, size.height);
                    if let Some(pp) = self.post_processor.as_mut() {
                        pp.resize(pixels.device(), pixels.queue(), size.width, size.height);
                    }
                }
            }

            if self.surface_width == 0 || self.surface_height == 0 {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                return;
            }

            self.cpu.bus.video.effects.chroma_blur        = self.shader_params.chroma_blur;
            self.cpu.bus.video.effects.comb_filter        = self.shader_params.comb_filter;
            self.cpu.bus.video.effects.phosphor_spread    = self.shader_params.phosphor_spread;
            self.cpu.bus.video.effects.white_preservation = self.shader_params.white_preservation;
            self.cpu.bus.video.effects.chroma_saturation  = self.shader_params.chroma_saturation;
            self.cpu.bus.video.effects.chroma_luma_scale  = self.shader_params.chroma_luma_scale;

            self.cpu.video_update();

            let (src_w, src_h) = self.cpu.bus.video.get_dimensions();
            let video_pixels = self.cpu.bus.video.get_pixels();
            let buf_w = self.buffer_width;
            let buf_h = self.buffer_height;
            
            let frame = pixels.frame_mut();

            if self.shader_type == ShaderType::Crt || self.shader_type == ShaderType::None {
                blit_direct(frame, video_pixels);
                
                if let Some(crt) = &self.post_processor {
                    let surf_w = self.surface_width;
                    let surf_h = self.surface_height;
                    let scale_x = surf_w as f64 / buf_w as f64;
                    let scale_y = surf_h as f64 / buf_h as f64;
                    
                    let scale = if self.is_fullscreen {
                        scale_x.min(scale_y).floor().max(1.0)
                    } else {
                        scale_x.min(scale_y)
                    };
                    
                    let scaled_w = (buf_w as f64 * scale) as u32;
                    let scaled_h = (buf_h as f64 * scale) as u32;
                    let offset_x = (surf_w - scaled_w) / 2;
                    let offset_y = (surf_h - scaled_h) / 2;
                    
                    let (active_w, active_h) = self.cpu.bus.video.get_active_dimensions();
                    let border = self.cpu.bus.video.get_border_size();
                    let border_inset_x = (border as f64 * scale) as u32;
                    let border_inset_y = (border as f64 * scale) as u32;
                    
                    crt.update_content_rect(
                        pixels.queue(),
                        &ContentRect {
                            surface_w: surf_w,
                            surface_h: surf_h,
                            offset_x: offset_x + border_inset_x,
                            offset_y: offset_y + border_inset_y,
                            dst_w: scaled_w - 2 * border_inset_x,
                            dst_h: scaled_h - 2 * border_inset_y,
                            bar_h: 0,
                            source_width: active_w as f32,
                            source_height: (active_h / 2) as f32,
                        },
                    );

                    let elapsed = self.shader_start_time.elapsed().as_secs_f32();
                    crt.update_time(pixels.queue(), elapsed);
                    crt.update_monochrome(pixels.queue(), self.cpu.bus.video.monochrome);
                    
                    let video_mode = self.cpu.bus.iou.video_mode.get();
                    let text_only = (video_mode & crate::video::VideoModeMask::TEXT) != 0 
                                 && (video_mode & crate::video::VideoModeMask::MIXED) == 0;
                    crt.update_text_mode(pixels.queue(), text_only);
                    
                    let power_on_elapsed = self.power_on_time.elapsed().as_secs_f32();
                    crt.update_power_on_time(pixels.queue(), power_on_elapsed);
                }
            } else {
                let bar_h = 0;

                frame.fill(0);
                for chunk in frame.chunks_exact_mut(4) {
                    chunk[3] = 255;
                }

                let display_region_h = buf_h.saturating_sub(bar_h);
                let mut blit_offset_x = 0u32;
                let mut blit_offset_y = 0u32;
                let mut blit_dst_w = 0u32;
                let mut blit_dst_h = 0u32;

                if display_region_h > 0 && buf_w > 0 {
                    let scale_x = buf_w as f64 / src_w as f64;
                    let scale_y = display_region_h as f64 / src_h as f64;
                    let int_scale = scale_x.min(scale_y).floor().max(1.0) as u32;

                    blit_dst_w = src_w * int_scale;
                    blit_dst_h = src_h * int_scale;

                    blit_offset_x = (buf_w - blit_dst_w) / 2;
                    blit_offset_y = (display_region_h - blit_dst_h) / 2;

                    blit_nearest(
                        frame,
                        video_pixels,
                        BlitRect {
                            frame_w: buf_w,
                            src_w,
                            src_h,
                            dst_x: blit_offset_x,
                            dst_y: blit_offset_y,
                            dst_w: blit_dst_w,
                            dst_h: blit_dst_h,
                        },
                    );
                }

                if let Some(lcd) = &self.post_processor {
                    let border = self.cpu.bus.video.border_size as u32;
                    let scale_x = if src_w > 0 { blit_dst_w as f64 / src_w as f64 } else { 1.0 };
                    let scale_y = if src_h > 0 { blit_dst_h as f64 / src_h as f64 } else { 1.0 };

                    let active_offset_x = blit_offset_x + (border as f64 * scale_x) as u32;
                    let active_offset_y = blit_offset_y + (border as f64 * scale_y) as u32;
                    let active_w = src_w - border * 2;
                    let active_h = src_h - border * 2;
                    let active_dst_w = (active_w as f64 * scale_x) as u32;
                    let active_dst_h = (active_h as f64 * scale_y) as u32;

                    lcd.update_content_rect(
                        pixels.queue(),
                        &ContentRect {
                            surface_w: buf_w,
                            surface_h: buf_h,
                            offset_x: active_offset_x,
                            offset_y: active_offset_y,
                            dst_w: active_dst_w,
                            dst_h: active_dst_h,
                            bar_h,
                            source_width: active_w as f32,
                            source_height: (active_h / 2) as f32,
                        },
                    );
                }
            }

            let render_result = {
                if let Some(crt) = self.post_processor.as_ref() {
                    crt.update_shader_params(pixels.queue(), &self.shader_params);
                }

                let egui_output = if self.show_shader_ui || self.show_drive_audio_ui || self.show_toolbar || self.show_settings_window {
                    if let Some(egui_state) = self.egui_state.as_mut() {
                        let window = self.window.as_ref().unwrap();
                        let raw_input = egui_state.take_egui_input(window.as_ref());

                        let col80 = self.cpu.bus.iou.col80_switch;
                        let drive_status: [DriveStatusInfo; 4] = [
                            {
                                let (has_disk, is_active, wp) = self.cpu.bus.iou.iwm.drive_status(0);
                                let filename = self.cpu.bus.iou.iwm.disk_filename(0);
                                DriveStatusInfo { has_disk, is_active, is_write_protected: wp, filename }
                            },
                            {
                                let (has_disk, is_active, wp) = self.cpu.bus.iou.iwm.drive_status(1);
                                let filename = self.cpu.bus.iou.iwm.disk_filename(1);
                                DriveStatusInfo { has_disk, is_active, is_write_protected: wp, filename }
                            },
                            {
                                let (has_disk, is_active, wp) = self.cpu.bus.iou.iwm.drive_status_35(0);
                                let filename = self.cpu.bus.iou.iwm.disk_filename_35(0);
                                DriveStatusInfo { has_disk, is_active, is_write_protected: wp, filename }
                            },
                            {
                                let (has_disk, is_active, wp) = self.cpu.bus.iou.iwm.drive_status_35(1);
                                let filename = self.cpu.bus.iou.iwm.disk_filename_35(1);
                                DriveStatusInfo { has_disk, is_active, is_write_protected: wp, filename }
                            },
                        ];

                        let mut drive_audio_changed = false;
                        let mut shader_save_clicked = false;
                        let mut drive_audio_save_clicked = false;
                        let mut settings_save_clicked = false;
                        let mut settings_reload_clicked = false;
                        let mut settings_changed = false;
                        let mut open_shader_panel = false;
                        let mut open_drive_audio_panel = false;
                        let mut toolbar_action = ToolbarAction::default();
                        let output = self.egui_ctx.run(raw_input, |ctx| {
                            if self.show_shader_ui {
                                let r = shader_ui::render_shader_ui(ctx, &mut self.shader_params, &mut self.show_shader_ui);
                                shader_save_clicked = r.save_clicked;
                            }
                            if self.show_drive_audio_ui {
                                let r = render_drive_audio_ui(ctx, &mut self.drive_audio_params, &mut self.show_drive_audio_ui);
                                drive_audio_changed = r.changed;
                                drive_audio_save_clicked = r.save_clicked;
                            }
                            if self.show_settings_window {
                                let r = render_settings_window(
                                    ctx,
                                    &mut self.config,
                                    &mut self.settings_state,
                                    &mut self.show_settings_window,
                                );
                                settings_changed = r.changed;
                                settings_save_clicked = r.save_requested;
                                settings_reload_clicked = r.reload_requested;
                                open_shader_panel = r.open_shader_panel;
                                open_drive_audio_panel = r.open_drive_audio_panel;
                            }
                            if self.show_toolbar {
                                if self.drive_icons.is_none() {
                                    self.drive_icons = Some(DriveIcons::load(ctx));
                                }
                                if self.toolbar_labels.is_none() {
                                    self.toolbar_labels = Some(ToolbarLabels::load(ctx));
                                }
                                toolbar_action = render_toolbar_ui(ctx, &drive_status, col80, self.paused, self.drive_icons.as_ref().unwrap(), self.toolbar_labels.as_ref().unwrap());
                            }
                        });
                        egui_state.handle_platform_output(window.as_ref(), output.platform_output.clone());

                        if drive_audio_changed {
                            self.cpu.bus.iou.iwm.drive_audio.params = self.drive_audio_params.clone();
                            self.cpu.bus.iou.iwm.drive_audio.apply_params();
                        }

                        if shader_save_clicked {
                            let mut cfg = Config::load();
                            cfg.shader = self.shader_params.clone();
                            match cfg.save() {
                                Ok(p) => log::info!("config: shader saved to {}", p.display()),
                                Err(e) => log::warn!("config: save failed: {}", e),
                            }
                            self.config.shader = self.shader_params.clone();
                        }
                        if drive_audio_save_clicked {
                            let mut cfg = Config::load();
                            cfg.drive_audio = self.drive_audio_params.clone();
                            match cfg.save() {
                                Ok(p) => log::info!("config: drive_audio saved to {}", p.display()),
                                Err(e) => log::warn!("config: save failed: {}", e),
                            }
                            self.config.drive_audio = self.drive_audio_params.clone();
                        }
    
                        if settings_changed {
                            self.cpu.bus.video.set_monochrome(self.config.display.monochrome);
                            self.cpu.bus.video.scanline_intensity = self.config.display.scanline_intensity;
                            self.cpu.bus.video.stable_page = self.config.display.stable_page;
                            self.cpu.bus.video.set_mono_colors(
                                self.config.display.mono_fg,
                                self.config.display.mono_bg,
                            );
                            // Shader type changes need a pipeline rebuild;
                            // defer to the next redraw because pixels is
                            // currently borrowed by this closure.
                            if self.config.display.shader_type != self.shader_type {
                                self.pending_shader_change = Some(self.config.display.shader_type);
                            }
                            if let Some(c) = self.audio_controls.as_ref() {
                                let a = &self.config.audio;
                                c.set_muted(a.muted);
                                c.set_master(a.master);
                                c.set_speaker(a.speaker);
                                c.set_mockingboard1(a.mockingboard1);
                                c.set_mockingboard2(a.mockingboard2);
                                c.set_drive(a.drive);
                            }
                        }
                        if settings_save_clicked {

                            self.config.shader = self.shader_params.clone();
                            self.config.drive_audio = self.drive_audio_params.clone();
                            match self.config.save() {
                                Ok(p) => {
                                    log::info!("config: saved to {}", p.display());
                                    self.settings_state.status = Some(format!("Saved → {}", p.display()));
                                }
                                Err(e) => {
                                    log::warn!("config: save failed: {}", e);
                                    self.settings_state.status = Some(format!("Save failed: {e}"));
                                }
                            }
                        }
                        if settings_reload_clicked {
                            self.config = Config::load();
                            self.shader_params = self.config.shader.clone();
                            self.drive_audio_params = self.config.drive_audio.clone();
                            self.cpu.bus.iou.iwm.drive_audio.params = self.drive_audio_params.clone();
                            self.cpu.bus.iou.iwm.drive_audio.apply_params();
                            self.cpu.bus.video.set_monochrome(self.config.display.monochrome);
                            self.cpu.bus.video.scanline_intensity = self.config.display.scanline_intensity;
                            self.cpu.bus.video.stable_page = self.config.display.stable_page;
                            self.cpu.bus.video.set_mono_colors(
                                self.config.display.mono_fg,
                                self.config.display.mono_bg,
                            );
                            if self.config.display.shader_type != self.shader_type {
                                self.pending_shader_change = Some(self.config.display.shader_type);
                            }
                            if let Some(c) = self.audio_controls.as_ref() {
                                let a = &self.config.audio;
                                c.set_muted(a.muted);
                                c.set_master(a.master);
                                c.set_speaker(a.speaker);
                                c.set_mockingboard1(a.mockingboard1);
                                c.set_mockingboard2(a.mockingboard2);
                                c.set_drive(a.drive);
                            }
                            self.settings_state.status = Some("Reloaded from disk".to_string());
                        }
                        if open_shader_panel {
                            self.show_shader_ui = true;
                        }
                        if open_drive_audio_panel {
                            self.show_drive_audio_ui = true;
                        }
                        
                        if toolbar_action.toggle_pause {
                            self.paused = !self.paused;
                        }
                        if toolbar_action.reset {
                            self.cpu.reset();
                        }
                        if toolbar_action.power {
                            self.cpu.power_cycle();
                            self.power_on_time = Instant::now();
                        }
                        if toolbar_action.toggle_col80 {
                            self.cpu.bus.iou.col80_switch = !self.cpu.bus.iou.col80_switch;
                        }
                        if let Some(drive) = toolbar_action.load_disk {
                            self.last_drive_click = Some((drive, Instant::now()));
                        }
                        if let Some(drive) = toolbar_action.toggle_write_protect {
                            if drive < 2 {
                                self.cpu.bus.iou.iwm.toggle_write_protect(drive);
                            } else {
                                self.cpu.bus.iou.iwm.toggle_write_protect_35(drive - 2);
                            }
                        }
                        if let Some(drive) = toolbar_action.eject_disk {
                            // Cancel any pending single-click for this drive
                            if let Some((d, _)) = self.last_drive_click {
                                if d == drive { self.last_drive_click = None; }
                            }
                            if drive < 2 {
                                self.cpu.bus.iou.iwm.eject_disk(drive);
                            } else {
                                self.cpu.bus.iou.iwm.eject_disk_35(drive - 2);
                            }
                        }
                        
                        let ppp = output.pixels_per_point;
                        let jobs = self.egui_ctx.tessellate(output.shapes.clone(), ppp);
                        Some((output, jobs, ppp))
                    } else {
                        None
                    }
                } else {
                    None
                };

                let device = pixels.device();
                let queue = pixels.queue();
                if let Some((ref output, _, _)) = egui_output {
                    if let Some(egui_renderer) = self.egui_renderer.as_mut() {
                        for (id, delta) in &output.textures_delta.set {
                            egui_renderer.update_texture(device, queue, *id, delta);
                        }
                    }
                }

                let sw = self.surface_width;
                let sh = self.surface_height;
                let mut egui_renderer = self.egui_renderer.take();

                let post_proc = self.post_processor.take();

                let render_t0 = Instant::now();
                let render_res = pixels.render_with(|encoder, render_target, context| {
                    if let Some(ref crt) = post_proc {
                        crt.clear_intermediate(encoder);
                        context.scaling_renderer.render(encoder, crt.intermediate_view());
                        crt.render(encoder, render_target, device);
                    } else {
                        context.scaling_renderer.render(encoder, render_target);
                    }

                    if let (Some(ref mut egui_rend), Some((_, ref jobs, ppp))) = (&mut egui_renderer, &egui_output) {
                        if !jobs.is_empty() {
                            let screen_desc = egui_wgpu::ScreenDescriptor {
                                size_in_pixels: [sw, sh],
                                pixels_per_point: *ppp,
                            };
                            let _ = egui_rend.update_buffers(device, queue, encoder, jobs, &screen_desc);
                            {
                                let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                    label: Some("egui_render_pass"),
                                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                        view: render_target,
                                        resolve_target: None,
                                        depth_slice: None,
                                        ops: wgpu::Operations {
                                            load: wgpu::LoadOp::Load,
                                            store: wgpu::StoreOp::Store,
                                        },
                                    })],
                                    depth_stencil_attachment: None,
                                    timestamp_writes: None,
                                    occlusion_query_set: None,
                                });
                                let mut rpass = rpass.forget_lifetime();
                                egui_rend.render(&mut rpass, jobs, &screen_desc);
                            }
                        }
                    }

                    Ok(())
                });

                self.egui_renderer = egui_renderer;
                self.post_processor = post_proc;

                self.gpu_perf.record(render_t0.elapsed().as_micros() as u64);

                // Free old egui textures
                if let Some((ref output, _, _)) = egui_output {
                    if let Some(egui_renderer) = self.egui_renderer.as_mut() {
                        for id in &output.textures_delta.free {
                            egui_renderer.free_texture(id);
                        }
                    }
                }

                render_res
            };

            if let Err(err) = render_result {
                eprintln!("pixels.render() warning: {} (will retry)", err);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }
    }

    fn handle_mouse_input(&mut self, state: ElementState, button: MouseButton) {
        if !self.mouse_enabled {
            return;
        }
        let pressed = state == ElementState::Pressed;
        if !self.mouse_grabbed {
            if pressed && button == MouseButton::Left {
                self.grab_mouse();
            }
            return;
        }
        match button {
            MouseButton::Left => {
                self.cpu.bus.iou.mouse.set_button(0, pressed);
            }
            MouseButton::Right => self.cpu.bus.iou.mouse.set_button(1, pressed),
            _ => (),
        }
    }

    fn grab_mouse(&mut self) {
        if !self.mouse_enabled || self.mouse_grabbed {
            return;
        }
        if let Some(window) = &self.window {
            let result = window
                .set_cursor_grab(winit::window::CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(winit::window::CursorGrabMode::Confined));
            if result.is_ok() {
                window.set_cursor_visible(false);
                self.mouse_grabbed = true;
            }
        }
    }

    fn release_mouse(&mut self) {
        if !self.mouse_grabbed {
            return;
        }
        if let Some(window) = &self.window {
            let _ = window.set_cursor_grab(winit::window::CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
        self.mouse_grabbed = false;
    }

    fn handle_keyboard_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: &winit::event::KeyEvent,
        egui_consumed: bool,
    ) {
        if event.logical_key == Key::Named(NamedKey::F7)
            && event.state.is_pressed()
            && !self.modifiers.control_key()
        {
            self.show_settings_window = !self.show_settings_window;
            if self.show_settings_window {
                self.settings_state.just_opened = true;
                self.release_mouse();
            }
            return;
        }

        if event.logical_key == Key::Named(NamedKey::F8)
            && event.state.is_pressed()
            && !self.modifiers.control_key()
            && !self.modifiers.shift_key()
        {
            if self.shader_type != ShaderType::None {
                self.show_shader_ui = !self.show_shader_ui;
                if self.show_shader_ui {
                    self.release_mouse();
                }
                println!(
                    "Shader UI: {}",
                    if self.show_shader_ui { "ON" } else { "OFF" }
                );
            }
            return;
        }

        if event.logical_key == Key::Named(NamedKey::F9)
            && event.state.is_pressed()
            && !self.modifiers.control_key()
        {
            self.show_drive_audio_ui = !self.show_drive_audio_ui;
            if self.show_drive_audio_ui {
                self.release_mouse();
            }
            println!(
                "Drive Audio UI: {}",
                if self.show_drive_audio_ui { "ON" } else { "OFF" }
            );
            return;
        }

        // F1–F4: open file dialog for drive 1, 2, 3.5" #1, 3.5" #2
        let drive_key = match event.logical_key {
            Key::Named(NamedKey::F1) => Some(0_usize),
            Key::Named(NamedKey::F2) => Some(1),
            Key::Named(NamedKey::F3) => Some(2),
            Key::Named(NamedKey::F4) => Some(3),
            _ => None,
        };
        if let Some(drive) = drive_key {
            if event.state.is_pressed() {
                self.last_drive_click = Some((
                    drive,
                    Instant::now() - Duration::from_millis(500),
                ));
                self.release_mouse();
            }
            return;
        }

        if event.logical_key == Key::Named(NamedKey::F12) && event.state.is_pressed() {
            self.toggle_monitor_window(event_loop);
            println!(
                "CPU Monitor: {}",
                if self.monitor_window.is_some() { "ON" } else { "OFF" }
            );
            return;
        }

        if !egui_consumed && event.state.is_pressed() {
            #[cfg(target_os = "macos")]
            let paste_modifier = self.modifiers.super_key();
            #[cfg(not(target_os = "macos"))]
            let paste_modifier = self.modifiers.control_key();
            if paste_modifier {
                if let PhysicalKey::Code(KeyCode::KeyV) = event.physical_key {
                    self.paste_clipboard_to_keyboard();
                    return;
                }
            }
        }

        let is_enter = event.logical_key == Key::Named(NamedKey::Enter);
        let is_f11 = event.logical_key == Key::Named(NamedKey::F11);
        #[cfg(target_os = "macos")]
        let fullscreen_modifier = self.modifiers.super_key();
        #[cfg(not(target_os = "macos"))]
        let fullscreen_modifier = self.modifiers.alt_key();
        let fullscreen_combo = (is_enter && fullscreen_modifier) || is_f11;
        if fullscreen_combo && event.state.is_pressed() {
            if let Some(window) = &self.window {
                #[cfg(target_os = "macos")]
                let current = window.simple_fullscreen();
                #[cfg(not(target_os = "macos"))]
                let current = window.fullscreen().is_some();
                let entering = !current;

                if entering {
                    window.set_decorations(false);
                    #[cfg(target_os = "macos")]
                    window.set_has_shadow(false);
                }

                self.is_fullscreen = entering;
                self.last_resize_time = None;

                #[cfg(target_os = "macos")]
                let success = window.set_simple_fullscreen(entering);
                #[cfg(not(target_os = "macos"))]
                let success = {
                    if entering {
                        window.set_fullscreen(Some(Fullscreen::Borderless(None)));
                    } else {
                        window.set_fullscreen(None);
                    }
                    true
                };

                if success {
                    if let Some(pixels) = &mut self.pixels {
                        let mode = if self.shader_type == ShaderType::Crt {
                            ScalingMode::PixelPerfect
                        } else {
                            ScalingMode::Fill
                        };
                        pixels.set_scaling_mode(mode);
                    }

                    if !entering {
                        window.set_decorations(true);
                        #[cfg(target_os = "macos")]
                        {
                            window.set_has_shadow(true);
                            let window_buttons = WindowButtons::CLOSE | WindowButtons::MINIMIZE;
                            window.set_enabled_buttons(window_buttons);
                        }
                    }
                } else {
                    self.is_fullscreen = !entering;
                    if entering {
                        window.set_decorations(true);
                        #[cfg(target_os = "macos")]
                        window.set_has_shadow(true);
                    }
                    eprintln!("Failed to toggle fullscreen");
                }
            }

            return;
        }

        match event.physical_key {
            #[cfg(target_os = "macos")]
            PhysicalKey::Code(KeyCode::SuperLeft) => {
                if self.cpu.bus.iou.debug {
                    println!(
                        "BUTTON: Left Cmd {} -> Open Apple",
                        if event.state.is_pressed() { "PRESS" } else { "RELEASE" }
                    );
                }
                self.cpu.bus.iou.mouse.open_apple.set(event.state.is_pressed());
            }
            #[cfg(target_os = "macos")]
            PhysicalKey::Code(KeyCode::SuperRight) => {
                if self.cpu.bus.iou.debug {
                    println!(
                        "BUTTON: Right Cmd {} -> Solid Apple",
                        if event.state.is_pressed() { "PRESS" } else { "RELEASE" }
                    );
                }
                self.cpu.bus.iou.mouse.solid_apple.set(event.state.is_pressed());
            }
            #[cfg(not(target_os = "macos"))]
            PhysicalKey::Code(KeyCode::AltLeft) => {
                self.cpu.bus.iou.mouse.open_apple.set(event.state.is_pressed());
            }
            #[cfg(not(target_os = "macos"))]
            PhysicalKey::Code(KeyCode::AltRight) => {
                self.cpu.bus.iou.mouse.solid_apple.set(event.state.is_pressed());
            }
            _ => {}
        }

        if egui_consumed {
            return;
        }

        let physical_key_id: Option<u16> = match event.physical_key {
            PhysicalKey::Code(code) => Some(code as u16),
            _ => None,
        };

        let key_code: Option<u8> = match event.logical_key {
            Key::Named(NamedKey::ArrowLeft) => Some(0x08),
            Key::Named(NamedKey::ArrowRight) => Some(0x15),
            Key::Named(NamedKey::ArrowUp) => Some(0x0B),
            Key::Named(NamedKey::ArrowDown) => Some(0x0A),
            Key::Named(NamedKey::Enter) => Some(0x0D),
            Key::Named(NamedKey::Tab) => Some(0x09),
            Key::Named(NamedKey::Backspace) => Some(0x7F),
            Key::Named(NamedKey::Delete) => Some(0x7F),
            Key::Named(NamedKey::Escape) => Some(0x1B),
            _ => {
                if self.modifiers.control_key() {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        let ctrl_code = match code {
                            KeyCode::KeyA => Some(0x01),
                            KeyCode::KeyB => Some(0x02),
                            KeyCode::KeyC => Some(0x03),
                            KeyCode::KeyD => Some(0x04),
                            KeyCode::KeyE => Some(0x05),
                            KeyCode::KeyF => Some(0x06),
                            KeyCode::KeyG => Some(0x07),
                            KeyCode::KeyH => Some(0x08),
                            KeyCode::KeyI => Some(0x09),
                            KeyCode::KeyJ => Some(0x0A),
                            KeyCode::KeyK => Some(0x0B),
                            KeyCode::KeyL => Some(0x0C),
                            KeyCode::KeyM => Some(0x0D),
                            KeyCode::KeyN => Some(0x0E),
                            KeyCode::KeyO => Some(0x0F),
                            KeyCode::KeyP => Some(0x10),
                            KeyCode::KeyQ => Some(0x11),
                            KeyCode::KeyR => Some(0x12),
                            KeyCode::KeyS => Some(0x13),
                            KeyCode::KeyT => Some(0x14),
                            KeyCode::KeyU => Some(0x15),
                            KeyCode::KeyV => Some(0x16),
                            KeyCode::KeyW => Some(0x17),
                            KeyCode::KeyX => Some(0x18),
                            KeyCode::KeyY => Some(0x19),
                            KeyCode::KeyZ => Some(0x1A),
                            KeyCode::BracketLeft => Some(0x1B),
                            KeyCode::Backslash => Some(0x1C),
                            KeyCode::BracketRight => Some(0x1D),
                            // Ctrl-6 = RS ($1E), Ctrl-- = US ($1F)
                            // Ctrl-Space / Ctrl-2 = NUL ($00)
                            KeyCode::Digit6 => Some(0x1E),
                            KeyCode::Minus => Some(0x1F),
                            KeyCode::Digit2 => Some(0x00),
                            KeyCode::Space => Some(0x00),
                            _ => None,
                        };
                        if ctrl_code.is_some() {
                            ctrl_code
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    if let Some(virtual_key) = event.logical_key.to_text() {
                        let key_char = virtual_key.chars().next().unwrap_or('\0');
 
                        Some(key_char.to_ascii_uppercase() as u8)
                    } else {
                        None
                    }
                }
            }
        };

        if event.state.is_pressed() {
            if event.logical_key == Key::Named(NamedKey::Backspace) && self.modifiers.control_key()
            {
                #[cfg(target_os = "macos")]
                let hard_reset = self.modifiers.super_key();
                #[cfg(not(target_os = "macos"))]
                let hard_reset = self.modifiers.alt_key();
                if hard_reset {
                    println!("Hard Reset Triggered");
                    self.cpu.bus.write_byte(0x03F4, 0x00);
                } else {
                    println!("Reset Triggered (Control + Backspace)");
                }
                self.cpu.reset();
                return;
            }

            if self.modifiers.control_key() {
                if let PhysicalKey::Code(KeyCode::KeyZ) = event.physical_key {
                    self.cpu.bus.iou.zip.toggle();
                    return;
                }
            }

            if event.logical_key == Key::Named(NamedKey::Escape) {
                self.cpu.bus.iou.zip.check_boot_escape();
            }

            if let (Some(phys), Some(code)) = (physical_key_id, key_code) {
                if self.cpu.bus.iou.debug {
                    println!(
                        "KBD EVENT: down phys={phys:#06X} code={code:#04X} logical={:?} consumed={egui_consumed}",
                        event.logical_key
                    );
                }
                self.cpu
                    .bus
                    .iou
                    .keyboard
                    .key_down(phys, code, self.cpu.bus.iou.cycles);
            }
        } else {
            if let Some(phys) = physical_key_id {
                if self.cpu.bus.iou.debug {
                    println!(
                        "KBD EVENT: up   phys={phys:#06X} logical={:?} consumed={egui_consumed}",
                        event.logical_key
                    );
                }
                self.cpu
                    .bus
                    .iou
                    .keyboard
                    .key_up(phys, self.cpu.bus.iou.cycles);
            }
        }

        if event.state.is_pressed() {
            match event.logical_key {
                Key::Named(NamedKey::F5) => {
                    let current = self.cpu.bus.video.monochrome;
                    self.cpu.bus.video.set_monochrome(!current);
                    self.power_on_time = Instant::now();
                    self.cpu.bus.iou.iwm.drive_audio.trigger_channel_static();
                }
                Key::Named(NamedKey::F6) => {
                    self.show_toolbar = !self.show_toolbar;

                    if self.show_toolbar {
                        self.release_mouse();
                    }
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                Key::Named(NamedKey::F10) if self.modifiers.control_key() => {
                    let new_debug_state = !self.cpu.debug;
                    self.cpu.debug = new_debug_state;
                    self.cpu.bus.debug = new_debug_state;
                    self.cpu.bus.iou.debug = new_debug_state;
                    self.cpu.bus.iou.iwm.debug = new_debug_state;
                    println!(
                        "Debug Logging: {}",
                        if new_debug_state { "ON" } else { "OFF" }
                    );
                }
                Key::Named(NamedKey::F11) if self.modifiers.control_key() => {
                    self.boot_into_bbs();
                }
                _ => {}
            }
        }
    }
}

pub fn run_monitor_mode(cpu: &mut Cpu) {
    let mut monitor = Monitor::new(cpu);
    monitor.repl();
}
