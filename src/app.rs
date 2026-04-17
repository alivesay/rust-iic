//! Application state and winit event handling for the Apple IIc emulator.
//!
//! This module contains the main application struct and the winit event loop
//! handler. It's responsible for window management, input handling, and
//! coordinating rendering.

use std::sync::Arc;
use std::time::{Duration, Instant};

use log::error;
use pixels::{Pixels, ScalingMode, SurfaceTexture};
use shader_ui::ShaderParams;
use winit::{
    dpi::LogicalSize,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey},
    window::{Window, WindowId},
};

#[cfg(target_os = "macos")]
use winit::platform::macos::WindowExtMacOS;

use crate::cli::ShaderType;
use crate::cpu::CPU;
use crate::cpu_monitor::{CpuMonitor, CpuState};
use crate::monitor::Monitor;
use crate::render::{
    blit_scaled, hit_test_col_button, hit_test_drive_icon, hit_test_power_button,
    hit_test_reset_button, hit_test_write_toggle, render_status_bar, CrtRenderer,
    DriveStatusInfo, LcdRenderer, PostProcessor, STATUS_BAR_HEIGHT,
};

/// Main application state.
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
    pub modifiers: ModifiersState,
    pub last_cursor_pos: Option<(f64, f64)>,
    pub show_toolbar: bool,
    pub is_fullscreen: bool,
    pub last_drive_click: Option<(usize, Instant)>,
    // egui state for shader parameter UI
    pub egui_ctx: egui::Context,
    pub egui_state: Option<egui_winit::State>,
    pub egui_renderer: Option<egui_wgpu::Renderer>,
    pub shader_params: ShaderParams,
    pub show_shader_ui: bool,
    pub cpu_monitor: CpuMonitor,
}

impl App {
    /// Create a new App with the given CPU and shader type.
    pub fn new(cpu: CPU, shader_type: ShaderType) -> Self {
        let (width, height) = cpu.bus.video.get_dimensions();
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
            modifiers: ModifiersState::default(),
            last_cursor_pos: None,
            show_toolbar: false,
            is_fullscreen: false,
            last_drive_click: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            egui_renderer: None,
            shader_params: ShaderParams::default(),
            show_shader_ui: false,
            cpu_monitor: CpuMonitor::new(),
        }
    }

    /// Flush disk changes before exit.
    pub fn flush_disks(&mut self) {
        println!("Flushing disks before exit...");
        self.cpu.bus.iou.iwm.eject_disk(0);
        self.cpu.bus.iou.iwm.eject_disk(1);
    }
}

impl winit::application::ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let (emu_w, emu_h) = self.cpu.bus.video.get_dimensions();

        // Window size: emulator at 2× logical height + status bar.
        // STATUS_BAR_HEIGHT is in physical pixels; convert to logical by dividing
        // by the expected scale factor (2.0 on Retina).
        let win_w = emu_w as f64;

        // Apply LCD aspect correction if in LCD mode
        // The Apple IIc flat panel was vertically compressed (~85% of CRT height)
        let height_scale = if self.shader_type == ShaderType::Lcd {
            2.0 * LcdRenderer::LCD_ASPECT_CORRECTION as f64
        } else {
            2.0
        };
        let win_h = emu_h as f64 * height_scale + (STATUS_BAR_HEIGHT as f64 / 2.0);

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Apple //c")
                        .with_inner_size(LogicalSize::new(win_w, win_h)),
                )
                .unwrap(),
        );

        self.window = Some(window.clone());

        let window_size = window.inner_size();
        self.surface_width = window_size.width;
        self.surface_height = window_size.height;
        self.buffer_width = window_size.width;
        self.buffer_height = window_size.height;

        let surface_texture =
            SurfaceTexture::new(window_size.width, window_size.height, window.clone());

        // Create pixels buffer at window size
        self.pixels = match Pixels::new(window_size.width, window_size.height, surface_texture) {
            Ok(mut pixels) => {
                // Use Fill scaling so fractional scaling works at any resolution
                pixels.set_scaling_mode(ScalingMode::Fill);
                // Ensure clear color is black (avoids grey border in fullscreen)
                pixels.clear_color(wgpu::Color::BLACK);
                // Create post-processor shader if enabled
                if self.shader_type != ShaderType::None {
                    let surface_format = pixels.render_texture_format();
                    let (src_w, src_h) = self.cpu.bus.video.get_dimensions();
                    // Create appropriate renderer based on shader type
                    self.post_processor = match self.shader_type {
                        ShaderType::Crt => Some(Box::new(CrtRenderer::new(
                            pixels.device(),
                            window_size.width,
                            window_size.height,
                            self.buffer_width,
                            self.buffer_height,
                            STATUS_BAR_HEIGHT,
                            src_w as f32,
                            src_h as f32,
                            surface_format,
                        )) as Box<dyn PostProcessor>),
                        ShaderType::Lcd => Some(Box::new(LcdRenderer::new(
                            pixels.device(),
                            window_size.width,
                            window_size.height,
                            self.buffer_width,
                            self.buffer_height,
                            STATUS_BAR_HEIGHT,
                            src_w as f32,
                            src_h as f32,
                            surface_format,
                        )) as Box<dyn PostProcessor>),
                        ShaderType::None => None,
                    };

                    // Initialize egui for shader parameter UI
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
                }
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
        // Only forward events to egui when shader UI is visible
        let egui_consumed = if self.show_shader_ui {
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
                            self.buffer_width = size.width;
                            self.buffer_height = size.height;
                            let _ = pixels.resize_buffer(size.width, size.height);

                            if let Some(pp) = self.post_processor.as_mut() {
                                pp.resize(pixels.device(), pixels.queue(), size.width, size.height);
                            }
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
    /// Handle redraw requests - renders frame and UI.
    fn handle_redraw(&mut self) {
        // Pending single-click on drive icon: open file dialog (blocking)
        if let Some((drive, click_time)) = self.last_drive_click {
            if click_time.elapsed() >= Duration::from_millis(400) {
                self.last_drive_click = None;
                let file = rfd::FileDialog::new()
                    .add_filter("WOZ Disk Image", &["woz"])
                    .pick_file();
                if let Some(path) = file {
                    println!("Loading disk into drive {}: {}", drive + 1, path.display());
                    if drive == 0 {
                        if let Err(e) = self.cpu.bus.iou.iwm.load_disk(&path) {
                            println!("Error loading disk: {}", e);
                        }
                    } else {
                        if let Err(e) = self.cpu.bus.iou.iwm.load_disk2(&path) {
                            println!("Error loading disk: {}", e);
                        }
                    }
                }
            }
        }

        if let Some(pixels) = self.pixels.as_mut() {
            if self.surface_width == 0 || self.surface_height == 0 {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                return;
            }

            self.cpu.video_update();

            let (src_w, src_h) = self.cpu.bus.video.get_dimensions();
            let video_pixels = self.cpu.bus.video.get_pixels();
            let surf_w = self.buffer_width;
            let surf_h = self.buffer_height;
            let bar_h = if self.show_toolbar { STATUS_BAR_HEIGHT } else { 0 };

            let frame = pixels.frame_mut();

            // Clear entire frame to black with alpha
            frame.fill(0);
            for chunk in frame.chunks_exact_mut(4) {
                chunk[3] = 255;
            }

            // Calculate scaled emulator display
            let display_region_h = surf_h.saturating_sub(bar_h);
            let mut blit_offset_x = 0u32;
            let mut blit_offset_y = 0u32;
            let mut blit_dst_w = 0u32;
            let mut blit_dst_h = 0u32;

            if display_region_h > 0 && surf_w > 0 {
                let virtual_src_h = if self.shader_type == ShaderType::Lcd {
                    src_h as f64 * LcdRenderer::LCD_ASPECT_CORRECTION as f64
                } else {
                    src_h as f64
                };

                let target_aspect = src_w as f64 / virtual_src_h;
                let window_aspect = surf_w as f64 / display_region_h as f64;

                let scale = if window_aspect > target_aspect {
                    (display_region_h as f64 / virtual_src_h).floor().max(1.0)
                } else {
                    (surf_w as f64 / src_w as f64).floor().max(1.0)
                };

                blit_dst_w = (src_w as f64 * scale) as u32;
                blit_dst_h = (virtual_src_h * scale) as u32;
                blit_offset_x = (surf_w - blit_dst_w) / 2;
                blit_offset_y = (display_region_h - blit_dst_h) / 2;

                blit_scaled(
                    frame,
                    surf_w,
                    video_pixels,
                    src_w,
                    src_h,
                    blit_offset_x,
                    blit_offset_y,
                    blit_dst_w,
                    blit_dst_h,
                );
            }

            // Render status bar
            let drive_status: [DriveStatusInfo; 2] = [
                {
                    let (has_disk, is_active, wp) = self.cpu.bus.iou.iwm.drive_status(0);
                    DriveStatusInfo {
                        has_disk,
                        is_active,
                        is_write_protected: wp,
                    }
                },
                {
                    let (has_disk, is_active, wp) = self.cpu.bus.iou.iwm.drive_status(1);
                    DriveStatusInfo {
                        has_disk,
                        is_active,
                        is_write_protected: wp,
                    }
                },
            ];
            let col80 = self.cpu.bus.iou.col80_switch;
            if self.show_toolbar {
                render_status_bar(frame, surf_w, surf_h, bar_h, &drive_status, col80);
            }

            // Post-processing render
            let render_result = if let Some(crt) = &self.post_processor {
                let border = self.cpu.bus.video.border_size as u32;
                let scale_x = if src_w > 0 {
                    blit_dst_w as f64 / src_w as f64
                } else {
                    1.0
                };
                let scale_y = if src_h > 0 {
                    blit_dst_h as f64 / src_h as f64
                } else {
                    1.0
                };

                let active_offset_x = blit_offset_x + (border as f64 * scale_x) as u32;
                let active_offset_y = blit_offset_y + (border as f64 * scale_y) as u32;
                let active_w = src_w - border * 2;
                let active_h = src_h - border * 2;
                let active_dst_w = (active_w as f64 * scale_x) as u32;
                let active_dst_h = (active_h as f64 * scale_y) as u32;

                crt.update_content_rect(
                    pixels.queue(),
                    self.buffer_width,
                    self.buffer_height,
                    active_offset_x,
                    active_offset_y,
                    active_dst_w,
                    active_dst_h,
                    bar_h,
                    active_w as f32,
                    (active_h / 2) as f32,
                );

                let elapsed = self.shader_start_time.elapsed().as_secs_f32();
                crt.update_time(pixels.queue(), elapsed);
                crt.update_monochrome(pixels.queue(), self.cpu.bus.video.monochrome);
                crt.update_shader_params(pixels.queue(), &self.shader_params);

                // Run egui UI (for shader UI and/or CPU monitor)
                let egui_output = if self.show_shader_ui || self.cpu_monitor.visible {
                    if let Some(egui_state) = self.egui_state.as_mut() {
                        let window = self.window.as_ref().unwrap();
                        let raw_input = egui_state.take_egui_input(window.as_ref());
                        
                        // Prepare CPU state for monitor
                        let cpu_state = CpuState {
                            pc: self.cpu.pc,
                            a: self.cpu.regs.a,
                            x: self.cpu.regs.x,
                            y: self.cpu.regs.y,
                            sp: self.cpu.regs.sp,
                            p: self.cpu.p.bits(),
                            cycles: self.cpu.cycles,
                        };
                        
                        // Copy the last trace entry from CPU to monitor
                        if self.cpu.capture_trace && self.cpu_monitor.enabled {
                            self.cpu_monitor.record(self.cpu.last_trace);
                        }
                        
                        // Snapshot memory for the monitor (stack + current memory page)
                        let mut memory_snapshot = [0u8; 512]; // Stack page + one more page
                        for i in 0..256 {
                            memory_snapshot[i] = self.cpu.bus.read_byte(0x0100 + i as u16);
                        }
                        let mem_page = self.cpu_monitor.memory_page;
                        let page_base = (mem_page as u16) << 8;
                        for i in 0..256 {
                            memory_snapshot[256 + i] = self.cpu.bus.read_byte(page_base + i as u16);
                        }
                        
                        let output = self.egui_ctx.run(raw_input, |ctx| {
                            if self.show_shader_ui {
                                shader_ui::render_shader_ui(ctx, &mut self.shader_params, &mut self.show_shader_ui);
                            }
                            if self.cpu_monitor.visible {
                                // Memory reader from snapshot
                                let memory_reader = |addr: u16| -> u8 {
                                    // Stack page
                                    if addr >= 0x0100 && addr < 0x0200 {
                                        memory_snapshot[(addr - 0x0100) as usize]
                                    // Current memory page
                                    } else if addr >= page_base && addr < page_base + 256 {
                                        memory_snapshot[256 + (addr - page_base) as usize]
                                    } else {
                                        0x00 // Unsnapshotted memory
                                    }
                                };
                                self.cpu_monitor.render(ctx, &cpu_state, &memory_reader);
                            }
                        });
                        egui_state.handle_platform_output(window.as_ref(), output.platform_output.clone());
                        let ppp = output.pixels_per_point;
                        let jobs = self.egui_ctx.tessellate(output.shapes.clone(), ppp);
                        Some((output, jobs, ppp))
                    } else {
                        None
                    }
                } else {
                    None
                };

                // Upload egui textures
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

    /// Handle mouse input events.
    fn handle_mouse_input(&mut self, state: ElementState, button: MouseButton) {
        let pressed = state == ElementState::Pressed;
        if pressed {
            if let Some((wx, wy)) = self.last_cursor_pos {
                let mapped = self
                    .pixels
                    .as_ref()
                    .and_then(|p| p.window_pos_to_pixel((wx as f32, wy as f32)).ok());

                if let Some((bx, by)) = mapped {
                    let px = bx as u32;
                    let py = by as u32;

                    if self.show_toolbar {
                        if button == MouseButton::Left
                            && hit_test_reset_button(px, py, self.buffer_height, STATUS_BAR_HEIGHT)
                        {
                            println!("Warm Reset Triggered (RST button)");
                            self.cpu.reset();
                            return;
                        }

                        if button == MouseButton::Left
                            && hit_test_power_button(px, py, self.buffer_height, STATUS_BAR_HEIGHT)
                        {
                            println!("Power Cycle Triggered (PWR button)");
                            self.cpu.power_cycle();
                            self.cpu.bus.write_byte(0x03F4, 0x00);
                            return;
                        }

                        if button == MouseButton::Left
                            && hit_test_col_button(px, py, self.buffer_height, STATUS_BAR_HEIGHT)
                        {
                            self.cpu.bus.iou.col80_switch = !self.cpu.bus.iou.col80_switch;
                            println!(
                                "Column switch: {}",
                                if self.cpu.bus.iou.col80_switch { "80" } else { "40" }
                            );
                            return;
                        }

                        if button == MouseButton::Left {
                            if let Some(drive) = hit_test_write_toggle(
                                px,
                                py,
                                self.buffer_width,
                                self.buffer_height,
                                STATUS_BAR_HEIGHT,
                            ) {
                                let (has_disk, _, _) = self.cpu.bus.iou.iwm.drive_status(drive);
                                if has_disk {
                                    self.cpu.bus.iou.iwm.toggle_write_protect(drive);
                                    let (_, _, wp) = self.cpu.bus.iou.iwm.drive_status(drive);
                                    println!(
                                        "Drive {}: write protect {}",
                                        drive + 1,
                                        if wp { "ON" } else { "OFF" }
                                    );
                                }
                                return;
                            }
                        }

                        if let Some(drive) = hit_test_drive_icon(
                            px,
                            py,
                            self.buffer_width,
                            self.buffer_height,
                            STATUS_BAR_HEIGHT,
                        ) {
                            match button {
                                MouseButton::Left => {
                                    let now = Instant::now();
                                    if let Some((prev_drive, prev_time)) = self.last_drive_click {
                                        if prev_drive == drive
                                            && now.duration_since(prev_time)
                                                < Duration::from_millis(400)
                                        {
                                            self.last_drive_click = None;
                                            let (has_disk, _, _) =
                                                self.cpu.bus.iou.iwm.drive_status(drive);
                                            println!("Drive {}: double-click detected, has_disk={}", drive + 1, has_disk);
                                            if has_disk {
                                                self.cpu.bus.iou.iwm.eject_disk(drive);
                                                println!("Drive {}: ejected", drive + 1);
                                            }
                                            return;
                                        }
                                    }
                                    self.last_drive_click = Some((drive, now));
                                    return;
                                }
                                MouseButton::Right => {
                                    let (has_disk, _, _) = self.cpu.bus.iou.iwm.drive_status(drive);
                                    if has_disk {
                                        self.cpu.bus.iou.iwm.toggle_write_protect(drive);
                                        let (_, _, wp) = self.cpu.bus.iou.iwm.drive_status(drive);
                                        println!(
                                            "Drive {}: write protect {}",
                                            drive + 1,
                                            if wp { "ON" } else { "OFF" }
                                        );
                                    }
                                    return;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        match button {
            MouseButton::Left => self.cpu.bus.iou.mouse.set_button(0, pressed),
            MouseButton::Right => self.cpu.bus.iou.mouse.set_button(1, pressed),
            _ => (),
        }
    }

    /// Handle keyboard input events.
    fn handle_keyboard_input(
        &mut self,
        _event_loop: &ActiveEventLoop,
        event: &winit::event::KeyEvent,
        egui_consumed: bool,
    ) {
        // F7 always toggles shader UI
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

        // F12 toggles CPU monitor
        if event.logical_key == Key::Named(NamedKey::F12) && event.state.is_pressed() {
            self.cpu_monitor.toggle();
            // Enable/disable trace capture on CPU
            self.cpu.capture_trace = self.cpu_monitor.enabled;
            println!(
                "CPU Monitor: {}",
                if self.cpu_monitor.visible { "ON" } else { "OFF" }
            );
            return;
        }

        // Map Command keys to Apple keys
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
            Key::Named(NamedKey::Backspace) => Some(0x7F), // Apple IIc Delete key
            Key::Named(NamedKey::Delete) => Some(0x7F),
            Key::Named(NamedKey::Escape) => Some(0x1B),
            _ => {
                // Handle Control key combinations
                if self.modifiers.control_key() {
                    // Get the base key from physical key for control combinations
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
                    // Normal key handling
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
            // Reset handling
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

            // ZIP CHIP: Ctrl+Z toggles acceleration
            if self.modifiers.control_key() {
                if let PhysicalKey::Code(KeyCode::KeyZ) = event.physical_key {
                    self.cpu.bus.iou.zip.toggle();
                    return;
                }
            }

            // ZIP CHIP: ESC during boot window disables acceleration
            if event.logical_key == Key::Named(NamedKey::Escape) {
                self.cpu.bus.iou.zip.check_boot_escape();
            }

            if let (Some(phys), Some(code)) = (physical_key_id, key_code) {
                self.cpu
                    .bus
                    .iou
                    .keyboard
                    .key_down(phys, code, self.cpu.bus.iou.cycles);
            }
        } else {
            if let Some(phys) = physical_key_id {
                self.cpu
                    .bus
                    .iou
                    .keyboard
                    .key_up(phys, self.cpu.bus.iou.cycles);
            }
        }

        // Function key handlers
        if event.state.is_pressed() {
            match event.logical_key {
                Key::Named(NamedKey::F1) => {
                    println!("F1 Pressed: Entering Monitor Mode");
                    run_monitor_mode(&mut self.cpu);
                }
                Key::Named(NamedKey::F3) => {
                    let current = self.cpu.bus.video.monochrome;
                    self.cpu.bus.video.set_monochrome(!current);
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

            // Cmd+Enter: Toggle fullscreen (macOS)
            #[cfg(target_os = "macos")]
            if event.logical_key == Key::Named(NamedKey::Enter) && self.modifiers.super_key() {
                if let Some(window) = &self.window {
                    let current = window.simple_fullscreen();
                    let entering = !current;
                    
                    if entering {
                        // Disable decorations/shadow BEFORE entering fullscreen
                        window.set_decorations(false);
                        window.set_has_shadow(false);
                    }
                    
                    let success = window.set_simple_fullscreen(entering);
                    
                    if success {
                        self.is_fullscreen = entering;
                        if !entering {
                            // Restore decorations/shadow AFTER exiting fullscreen
                            window.set_decorations(true);
                            window.set_has_shadow(true);
                        }
                    } else {
                        // Restore state on failure
                        if entering {
                            window.set_decorations(true);
                            window.set_has_shadow(true);
                        }
                        eprintln!("Failed to toggle fullscreen");
                    }
                }
                return;
            }
        }
    }
}

/// Enter interactive monitor/debugger mode.
pub fn run_monitor_mode(cpu: &mut CPU) {
    let mut monitor = Monitor::new(cpu);
    monitor.repl();
}
