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

#[cfg(target_os = "macos")]
use winit::platform::macos::WindowExtMacOS;

use crate::cli::ShaderType;
use crate::cpu::CPU;
use crate::cpu_monitor::{CpuMonitor, CpuState};
use crate::device::drive_audio::DriveAudioParams;
use crate::monitor::Monitor;
use crate::render::{
    blit_direct, blit_nearest, CrtRenderer,
    DriveIcons, DriveStatusInfo, LcdRenderer, PostProcessor, ToolbarAction, render_toolbar_ui,
};

pub struct App {
    pub pixels: Option<Pixels<'static>>,
    pub window: Option<Arc<Window>>,
    pub cpu: CPU,
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
    pub cpu_monitor: CpuMonitor,
    pub drive_icons: Option<DriveIcons>,
    pub paused: bool,
    pub window_aspect_ratio: f64,
    pub last_resize_time: Option<Instant>,
}

impl App {
    pub fn new(cpu: CPU, shader_type: ShaderType, start_fullscreen: bool) -> Self {
        // Use active dimensions for initial sizing (excludes border)
        let (width, height) = cpu.bus.video.get_active_dimensions();
        Self {
            pixels: None,
            window: None,
            cpu,
            // Initial values will be overwritten in resumed() when window is created
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
            show_toolbar: false,
            is_fullscreen: false,
            start_fullscreen,
            last_drive_click: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            egui_renderer: None,
            shader_params: ShaderParams::default(),
            show_shader_ui: false,
            show_drive_audio_ui: false,
            drive_audio_params: DriveAudioParams::default(),
            cpu_monitor: CpuMonitor::new(),
            drive_icons: None,
            paused: false,
            window_aspect_ratio: 1.0,
            last_resize_time: None,
        }
    }

    pub fn flush_disks(&mut self) {
        self.cpu.bus.iou.iwm.eject_disk(0);
        self.cpu.bus.iou.iwm.eject_disk(1);
        self.cpu.bus.iou.iwm.smartport.flush_all();
    }

    /// Snap window to correct aspect ratio after user finishes resizing
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

fn render_drive_audio_ui(ctx: &egui::Context, params: &mut DriveAudioParams, open: &mut bool) -> bool {
    let mut changed = false;
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
                });
            });
        });

    changed
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
        
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Apple //c")
                        .with_inner_size(LogicalSize::new(win_w, win_h))
                        .with_min_inner_size(LogicalSize::new(base_w, base_h))
                        .with_enabled_buttons(window_buttons),
                )
                .unwrap(),
        );

        self.window = Some(window.clone());

        #[cfg(target_os = "macos")]
        if self.start_fullscreen {
            window.set_decorations(false);
            window.set_has_shadow(false);
            let _ = window.set_simple_fullscreen(true);
            self.is_fullscreen = true;
        }

        let scale_factor = window.scale_factor();
        let window_size = window.inner_size();
        
        let phys_w = (win_w * scale_factor) as u32;
        let phys_h = (win_h * scale_factor) as u32;
        let surface_w = window_size.width.max(phys_w);
        let surface_h = window_size.height.max(phys_h);
        
        self.surface_width = surface_w;
        self.surface_height = surface_h;
        
        let (src_w, src_h) = self.cpu.bus.video.get_dimensions();
        let (active_w, active_h) = self.cpu.bus.video.get_active_dimensions();
        
        let (buf_w, buf_h) = match self.shader_type {
            ShaderType::Crt => (src_w, src_h),
            _ => (surface_w, surface_h),
        };
        self.buffer_width = buf_w;
        self.buffer_height = buf_h;

        let surface_texture =
            SurfaceTexture::new(surface_w, surface_h, window.clone());

        self.pixels = match PixelsBuilder::new(buf_w, buf_h, surface_texture)
            .texture_format(wgpu::TextureFormat::Rgba8UnormSrgb)
            .render_texture_format(wgpu::TextureFormat::Bgra8UnormSrgb)
            .build() {
            Ok(mut pixels) => {
                if self.is_fullscreen {
                    pixels.set_scaling_mode(ScalingMode::PixelPerfect);
                } else {
                    pixels.set_scaling_mode(ScalingMode::Fill);
                }
                pixels.clear_color(wgpu::Color::BLACK);
                let surface_format = pixels.render_texture_format();

                if self.shader_type != ShaderType::None {
                    self.post_processor = match self.shader_type {
                        ShaderType::Crt => Some(Box::new(CrtRenderer::new(
                            pixels.device(),
                            surface_w,
                            surface_h,
                            buf_w,
                            buf_h,
                            0,
                            active_w as f32,
                            active_h as f32,
                            surface_format,
                        )) as Box<dyn PostProcessor>),
                        ShaderType::Lcd => Some(Box::new(LcdRenderer::new(
                            pixels.device(),
                            surface_w,
                            surface_h,
                            buf_w,
                            buf_h,
                            0,
                            active_w as f32,
                            active_h as f32,
                            surface_format,
                        )) as Box<dyn PostProcessor>),
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

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let egui_consumed = if self.show_shader_ui || self.show_drive_audio_ui || self.cpu_monitor.visible || self.show_toolbar {
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
                }
                self.modifiers = ModifiersState::empty();
            }

            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if size.width != self.surface_width || size.height != self.surface_height {
                        self.surface_width = size.width;
                        self.surface_height = size.height;

                        if let Some(pixels) = self.pixels.as_mut() {
                            let _ = pixels.resize_surface(size.width, size.height);
                            
                            if self.shader_type != ShaderType::Crt {
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
            }

            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            WindowEvent::RedrawRequested => {
                self.handle_redraw();
            }

            WindowEvent::CursorMoved { position, .. } => {
                let x = position.x;
                let y = position.y;
                if !egui_consumed {
                    if let Some((lx, ly)) = self.last_cursor_pos {
                        let dx = x - lx;
                        let dy = y - ly;
                        self.cpu.bus.iou.mouse.add_delta(dx, dy);
                    }
                }
                self.last_cursor_pos = Some((x, y));
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
}

impl App {
    fn handle_redraw(&mut self) {
        if let Some((drive, click_time)) = self.last_drive_click {
            if click_time.elapsed() >= Duration::from_millis(400) {
                self.last_drive_click = None;
                
                let file = if drive < 2 {
                    rfd::FileDialog::new()
                        .add_filter("WOZ Disk Image", &["woz"])
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

            self.cpu.video_update();

            let (src_w, src_h) = self.cpu.bus.video.get_dimensions();
            let video_pixels = self.cpu.bus.video.get_pixels();
            let buf_w = self.buffer_width;
            let buf_h = self.buffer_height;
            
            let frame = pixels.frame_mut();

            if self.shader_type == ShaderType::Crt {
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
                        surf_w,
                        surf_h,
                        offset_x + border_inset_x,
                        offset_y + border_inset_y,
                        scaled_w - 2 * border_inset_x,
                        scaled_h - 2 * border_inset_y,
                        0,
                        active_w as f32,
                        (active_h / 2) as f32,
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
                        buf_w,
                        video_pixels,
                        src_w,
                        src_h,
                        blit_offset_x,
                        blit_offset_y,
                        blit_dst_w,
                        blit_dst_h,
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
                        buf_w,
                        buf_h,
                        active_offset_x,
                        active_offset_y,
                        active_dst_w,
                        active_dst_h,
                        bar_h,
                        active_w as f32,
                        (active_h / 2) as f32,
                    );
                }
            }

            let render_result = if let Some(crt) = &self.post_processor {
                crt.update_shader_params(pixels.queue(), &self.shader_params);

                let egui_output = if self.show_shader_ui || self.show_drive_audio_ui || self.cpu_monitor.visible || self.show_toolbar {
                    if let Some(egui_state) = self.egui_state.as_mut() {
                        let window = self.window.as_ref().unwrap();
                        let raw_input = egui_state.take_egui_input(window.as_ref());
                        
                        let cpu_state = CpuState {
                            pc: self.cpu.pc,
                            a: self.cpu.regs.a,
                            x: self.cpu.regs.x,
                            y: self.cpu.regs.y,
                            sp: self.cpu.regs.sp,
                            p: self.cpu.p.bits(),
                            cycles: self.cpu.cycles,
                        };
                        
                        if self.cpu.capture_trace && self.cpu_monitor.enabled {
                            self.cpu_monitor.record(self.cpu.last_trace);
                        }
                        
                        let mut memory_snapshot = [0u8; 512];
                        for i in 0..256 {
                            memory_snapshot[i] = self.cpu.bus.read_byte(0x0100 + i as u16);
                        }
                        let mem_page = self.cpu_monitor.memory_page;
                        let page_base = (mem_page as u16) << 8;
                        for i in 0..256 {
                            memory_snapshot[256 + i] = self.cpu.bus.read_byte(page_base + i as u16);
                        }
                        
                        let col80 = self.cpu.bus.iou.col80_switch;
                        let drive_status: [DriveStatusInfo; 4] = [
                            {
                                let (has_disk, is_active, wp) = self.cpu.bus.iou.iwm.drive_status(0);
                                DriveStatusInfo { has_disk, is_active, is_write_protected: wp }
                            },
                            {
                                let (has_disk, is_active, wp) = self.cpu.bus.iou.iwm.drive_status(1);
                                DriveStatusInfo { has_disk, is_active, is_write_protected: wp }
                            },
                            {
                                let (has_disk, is_active, wp) = self.cpu.bus.iou.iwm.drive_status_35(0);
                                DriveStatusInfo { has_disk, is_active, is_write_protected: wp }
                            },
                            {
                                let (has_disk, is_active, wp) = self.cpu.bus.iou.iwm.drive_status_35(1);
                                DriveStatusInfo { has_disk, is_active, is_write_protected: wp }
                            },
                        ];
                        
                        let mut drive_audio_changed = false;
                        let mut toolbar_action = ToolbarAction::default();
                        let output = self.egui_ctx.run(raw_input, |ctx| {
                            if self.show_shader_ui {
                                shader_ui::render_shader_ui(ctx, &mut self.shader_params, &mut self.show_shader_ui);
                            }
                            if self.show_drive_audio_ui {
                                drive_audio_changed = render_drive_audio_ui(ctx, &mut self.drive_audio_params, &mut self.show_drive_audio_ui);
                            }
                            if self.cpu_monitor.visible {
                                let memory_reader = |addr: u16| -> u8 {
                                    if addr >= 0x0100 && addr < 0x0200 {
                                        memory_snapshot[(addr - 0x0100) as usize]
                                    } else if addr >= page_base && addr < page_base + 256 {
                                        memory_snapshot[256 + (addr - page_base) as usize]
                                    } else {
                                        0x00
                                    }
                                };
                                self.cpu_monitor.render(ctx, &cpu_state, &memory_reader);
                            }
                            if self.show_toolbar {
                                if self.drive_icons.is_none() {
                                    self.drive_icons = Some(DriveIcons::load(ctx));
                                }
                                toolbar_action = render_toolbar_ui(ctx, &drive_status, col80, self.paused, self.drive_icons.as_ref().unwrap());
                            }
                        });
                        egui_state.handle_platform_output(window.as_ref(), output.platform_output.clone());
                        
                        if drive_audio_changed {
                            self.cpu.bus.iou.iwm.drive_audio.params = self.drive_audio_params.clone();
                            self.cpu.bus.iou.iwm.drive_audio.apply_params();
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
                            let file = if drive < 2 {
                                rfd::FileDialog::new()
                                    .add_filter("WOZ Disk Image", &["woz"])
                                    .pick_file()
                            } else {
                                rfd::FileDialog::new()
                                    .add_filter("3.5\" Disk Image", &["po", "2mg", "2img"])
                                    .pick_file()
                            };
                            if let Some(path) = file {
                                let _ = match drive {
                                    0 => self.cpu.bus.iou.iwm.load_disk(&path),
                                    1 => self.cpu.bus.iou.iwm.load_disk2(&path),
                                    2 => self.cpu.bus.iou.iwm.load_disk35(&path),
                                    3 => self.cpu.bus.iou.iwm.load_disk35_drive(1, &path),
                                    _ => Ok(()),
                                };
                            }
                        }
                        if let Some(drive) = toolbar_action.toggle_write_protect {
                            if drive < 2 {
                                self.cpu.bus.iou.iwm.toggle_write_protect(drive);
                            } else {
                                self.cpu.bus.iou.iwm.toggle_write_protect_35(drive - 2);
                            }
                        }
                        if let Some(drive) = toolbar_action.eject_disk {
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
                let egui_renderer = self.egui_renderer.as_mut();

                let render_res = pixels.render_with(|encoder, render_target, context| {
                    crt.clear_intermediate(encoder);
                    context.scaling_renderer.render(encoder, crt.intermediate_view());
                    crt.render(encoder, render_target, device);

                    if let (Some(egui_rend), Some((_, ref jobs, ppp))) = (egui_renderer, &egui_output) {
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

                    Ok(())
                });

                // Free old egui textures
                if let Some((ref output, _, _)) = egui_output {
                    if let Some(egui_renderer) = self.egui_renderer.as_mut() {
                        for id in &output.textures_delta.free {
                            egui_renderer.free_texture(id);
                        }
                    }
                }

                render_res
            } else {
                pixels.render()
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
        let pressed = state == ElementState::Pressed;
        if pressed {
            if let Some((wx, wy)) = self.last_cursor_pos {
                let mapped = self
                    .pixels
                    .as_ref()
                    .and_then(|p| p.window_pos_to_pixel((wx as f32, wy as f32)).ok());

                if let Some((bx, by)) = mapped {
                    let _px = bx as u32;
                    let _py = by as u32;
                }
            }
        }
        match button {
            MouseButton::Left => self.cpu.bus.iou.mouse.set_button(0, pressed),
            MouseButton::Right => self.cpu.bus.iou.mouse.set_button(1, pressed),
            _ => (),
        }
    }

    fn handle_keyboard_input(
        &mut self,
        _event_loop: &ActiveEventLoop,
        event: &winit::event::KeyEvent,
        egui_consumed: bool,
    ) {
        if event.logical_key == Key::Named(NamedKey::F7) && event.state.is_pressed() {
            if self.shader_type != ShaderType::None {
                self.show_shader_ui = !self.show_shader_ui;
                println!(
                    "Shader UI: {}",
                    if self.show_shader_ui { "ON" } else { "OFF" }
                );
            }
            return;
        }

        if event.logical_key == Key::Named(NamedKey::F8) && event.state.is_pressed() {
            self.show_drive_audio_ui = !self.show_drive_audio_ui;
            println!(
                "Drive Audio UI: {}",
                if self.show_drive_audio_ui { "ON" } else { "OFF" }
            );
            return;
        }

        if event.logical_key == Key::Named(NamedKey::F12) && event.state.is_pressed() {
            self.cpu_monitor.toggle();
            self.cpu.capture_trace = self.cpu_monitor.enabled;
            println!(
                "CPU Monitor: {}",
                if self.cpu_monitor.visible { "ON" } else { "OFF" }
            );
            return;
        }

        #[cfg(target_os = "macos")]
        if event.logical_key == Key::Named(NamedKey::Enter) && self.modifiers.super_key() && event.state.is_pressed() {
            if let Some(window) = &self.window {
                let current = window.simple_fullscreen();
                let entering = !current;
                
                if entering {
                    window.set_decorations(false);
                    window.set_has_shadow(false);
                }
                
                let success = window.set_simple_fullscreen(entering);
                
                if success {
                    self.is_fullscreen = entering;
                    
                    if let Some(pixels) = &mut self.pixels {
                        if entering {
                            pixels.set_scaling_mode(ScalingMode::PixelPerfect);
                        } else {
                            pixels.set_scaling_mode(ScalingMode::Fill);
                        }
                    }
                    
                    if !entering {
                        window.set_decorations(true);
                        window.set_has_shadow(true);
                        let window_buttons = WindowButtons::CLOSE | WindowButtons::MINIMIZE;
                        window.set_enabled_buttons(window_buttons);
                    }
                } else {
                    if entering {
                        window.set_decorations(true);
                        window.set_has_shadow(true);
                    }
                    eprintln!("Failed to toggle fullscreen");
                }
            }
            return;
        }

        match event.physical_key {
            PhysicalKey::Code(KeyCode::SuperLeft) => {
                if self.cpu.bus.iou.debug {
                    println!(
                        "BUTTON: Left Cmd {} -> Open Apple (button0)",
                        if event.state.is_pressed() { "PRESS" } else { "RELEASE" }
                    );
                }
                self.cpu.bus.iou.mouse.set_button(0, event.state.is_pressed());
            }
            PhysicalKey::Code(KeyCode::SuperRight) => {
                if self.cpu.bus.iou.debug {
                    println!(
                        "BUTTON: Right Cmd {} -> Closed Apple (button1)",
                        if event.state.is_pressed() { "PRESS" } else { "RELEASE" }
                    );
                }
                self.cpu.bus.iou.mouse.set_button(1, event.state.is_pressed());
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
                if self.modifiers.super_key() {
                    println!("Hard Reset Triggered (Control + Command + Backspace)");
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
                Key::Named(NamedKey::F1) => {
                    println!("F1 Pressed: Entering Monitor Mode");
                    run_monitor_mode(&mut self.cpu);
                }
                Key::Named(NamedKey::F3) => {
                    let current = self.cpu.bus.video.monochrome;
                    self.cpu.bus.video.set_monochrome(!current);
                    self.power_on_time = Instant::now();
                    self.cpu.bus.iou.iwm.drive_audio.trigger_channel_static();
                }
                Key::Named(NamedKey::F6) => {
                    self.show_toolbar = !self.show_toolbar;
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                Key::Named(NamedKey::F10) => {
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
                _ => {}
            }
        }
    }
}

pub fn run_monitor_mode(cpu: &mut CPU) {
    let mut monitor = Monitor::new(cpu);
    monitor.repl();
}
