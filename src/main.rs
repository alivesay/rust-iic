#[macro_use]
mod macros;

mod bus;
mod cpu;
mod device;
mod disassembler;
mod interrupts;
mod iou;
mod memory;
mod mmu;
mod monitor;
mod gui;
mod crt;
mod rom;
mod util;
mod video;


use crate::cpu::CPU;
use crate::crt::CrtRenderer;
use crate::gui::{DriveStatusInfo, STATUS_BAR_HEIGHT, blit_scaled, render_status_bar, hit_test_reset_button, hit_test_power_button, hit_test_col_button, hit_test_drive_icon, hit_test_write_toggle};
use crate::monitor::Monitor;
use clap::Parser;
use cpu::{CpuType, SystemType};
use log::error;
use pixels::{Error, Pixels, ScalingMode, SurfaceTexture};
use shader_ui::ShaderParams;
use std::{
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use winit::{
    dpi::LogicalSize,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey, ModifiersState},
    platform::pump_events::{EventLoopExtPumpEvents, PumpStatus},
    window::{Window, WindowId},
};

const O_O: &str = r#"*
     ██▀███   █    ██   ██████ ▄▄▄█████▓ ██▓ ██▓ ▄████▄  
*    ▓██ ▒ ██▒ ██  ▓██▒▒██    ▒ ▓  ██▒ ▓▒▓██▒▓██▒▒██▀ ▀█  
     ▓██ ░▄█ ▒▓██  ▒██░░ ▓██▄   ▒ ▓██░ ▒░▒██▒▒██▒▒▓█    ▄ 
*    ▒██▀▀█▄  ▓▓█  ░██░  ▒   ██▒░ ▓██▓ ░ ░██░░██░▒▓▓▄ ▄██▒
     ░██▓ ▒██▒▒▒█████▓ ▒██████▒▒  ▒██▒ ░ ░██░░██░▒ ▓███▀ ░
*    ░ ▒▓ ░▒▓░░▒▓▒ ▒ ▒ ▒ ▒▓▒ ▒ ░  ▒ ░░   ░▓  ░▓  ░ ░▒ ▒  ░
     ░▒ ░ ▒░░░▒░ ░ ░ ░ ░▒  ░ ░    ░     ▒ ░ ▒ ░  ░  ▒   
*    ░░   ░  ░░░ ░ ░ ░  ░  ░    ░       ▒ ░ ▒ ░░        
     ░        ░           ░            ░   ░  ░ ░      
*                                             ░        

*
"#;

#[derive(Parser)]
#[command(version, about = "Apple //c Emulator")]
struct Args {
    #[arg(long)]
    no_video: bool,

    #[arg(long)]
    monitor: bool,

    #[arg(long, default_value = "auto")]
    rom_type: String,

    #[arg(long, short)]
    debug: bool,

    #[arg(long, default_value_t = 1.0)]
    speed: f32,

    #[arg(long)]
    monochrome: bool,

    #[arg(long, default_value_t = 0.5)]
    scanline_intensity: f32,

    #[arg(long)]
    perf: bool,

    #[arg(long)]
    self_test: bool,

    #[arg(long)]
    fast_until: Option<String>,

    #[arg(long)]
    log_until: Option<String>,

    #[arg(long, default_value_t = 10.0)]
    fast_speed: f32,

    #[arg(index = 1)]
    disk: Option<String>,

    #[arg(long)]
    disk2: Option<String>,

    /// Enable fast disk mode (skip rotational latency)
    #[arg(long)]
    fast_disk: bool,

    /// Enable CRT post-processing shader
    #[arg(long)]
    crt: bool,

    /// Connect modem port (SCC Ch A) to a TCP host, e.g. --serial bbs.example.com:23
    #[arg(long)]
    serial: Option<String>,

    /// Enable virtual Hayes modem on slot 1 (use ATDT from terminal software to connect)
    #[arg(long)]
    modem: bool,

    /// Enable serial loopback mode (for diagnostic testing with loopback cable)
    #[arg(long, conflicts_with = "modem")]
    serial_loopback: bool,
}



pub struct App {
    pixels: Option<Pixels<'static>>,
    window: Option<Arc<Window>>,
    cpu: CPU,
    surface_width: u32,
    surface_height: u32,
    buffer_width: u32,
    buffer_height: u32,
    crt_renderer: Option<CrtRenderer>,
    crt_enabled: bool,
    crt_start_time: Instant,
    modifiers: ModifiersState,
    last_cursor_pos: Option<(f64, f64)>,
    show_toolbar: bool,
    last_drive_click: Option<(usize, Instant)>,
    // egui state for shader parameter UI
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    shader_params: ShaderParams,
    show_shader_ui: bool,
}

fn main() -> Result<(), Error> {
    env_logger::init();

    println!("{}{}", "*\n\n".repeat(8), O_O);

    let args = Args::parse();

    let mut cpu = CPU::new(
        SystemType::AppleIIc,
        CpuType::CMOS65C02,
        (args.speed * 1_023_000.0) as u32,
        args.self_test,
    );
    cpu.debug = args.debug;
    cpu.bus.debug = args.debug;
    cpu.bus.iou.debug = args.debug;
    cpu.bus.iou.iwm.debug = args.debug;
    cpu.bus.iou.iwm.fast_disk = args.fast_disk;
    cpu.bus.video.set_monochrome(args.monochrome);
    cpu.bus.video.crt_enabled = args.crt;
    cpu.bus.video.scanline_intensity = args.scanline_intensity;

    // Connect modem port SCC Channel A to TCP if specified
    if let Some(ref addr) = args.serial {
        cpu.bus.iou.scc.ch_a.debug = args.debug;
        if let Err(e) = cpu.bus.iou.scc.ch_a.tcp_connect(addr) {
            eprintln!("Failed to connect serial to {}: {}", addr, e);
        }
    }

    // Enable virtual Hayes modem on Channel A (modem port, slot 2)
    // Apple IIc has dedicated ports: Channel A = modem, Channel B = printer
    if args.modem {
        cpu.bus.iou.scc.ch_a.modem_enabled = true;
        cpu.bus.iou.scc.ch_a.debug = args.debug;
        println!("Virtual Hayes modem enabled on modem port (slot 2)");
        println!("Use ATDT host:port from terminal software to connect");
    }

    // Enable serial loopback mode
    // This simulates loopback plugs on each port (TX→RX on same port)
    // Compatible with ACIA/6551 loopback tests (e.g., a2ediag external serial test)
    if args.serial_loopback {
        cpu.bus.iou.scc.ch_a.loopback = true;
        cpu.bus.iou.scc.ch_b.loopback = true;
        println!("Serial loopback mode enabled on both ports");
    }

    let iic_rom_file = include_bytes!("../iic3.bin");
    let iic_rom = rom::ROM::load_from_bytes(iic_rom_file, cpu.system_type).unwrap();

    cpu.load_rom(iic_rom);
    cpu.init();

    let disk_path = args.disk.or_else(|| {
        let default_path = "floppies/diag.woz";
        if std::path::Path::new(default_path).exists() {
            Some(default_path.to_string())
        } else {
            None
        }
    });

    if let Some(path) = disk_path {
        println!("Loading disk 1: {}", path);
        cpu.bus.iou.iwm.load_disk(path).unwrap();
    }

    if let Some(path) = &args.disk2 {
        println!("Loading disk 2: {}", path);
        cpu.bus.iou.iwm.load_disk2(path).unwrap();
    }

    if args.monitor {
        run_monitor_mode(&mut cpu);
    }

    if args.no_video {
        run_cpu_console_mode(cpu);
        return Ok(());
    }

    let mut event_loop = EventLoop::new().unwrap();
    let (width, height) = cpu.bus.video.get_dimensions();
    let crt_enabled = args.crt;
    let mut app = App {
        pixels: None,
        window: None,
        cpu,
        surface_width: width * 2,
        surface_height: height * 2,
        buffer_width: width * 2,
        buffer_height: height * 2,
        crt_renderer: None,
        crt_enabled,
        crt_start_time: Instant::now(),
        modifiers: ModifiersState::default(),
        last_cursor_pos: None,
        show_toolbar: true,
        last_drive_click: None,
        egui_ctx: egui::Context::default(),
        egui_state: None,
        egui_renderer: None,
        shader_params: ShaderParams::default(),
        show_shader_ui: false,
    };

    let timeout = Some(Duration::ZERO);
    let target_frame_time = Duration::from_micros(16667); // ~60Hz
    
    let mut fast_mode = args.fast_until.is_some();
    let fast_until_addr = if let Some(s) = &args.fast_until {
        u16::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or(0)
    } else { 0 };
    
    let log_until_addr = if let Some(s) = &args.log_until {
        u16::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or(0)
    } else { 0 };

    let mut cycles_per_frame = if fast_mode {
        (args.fast_speed * 1_023_000.0 / 60.0) as u64
    } else {
        (args.speed * 1_023_000.0 / 60.0) as u64
    };

    let mut next_frame_time = Instant::now();

    // profiling
    let mut perf_start = Instant::now();
    let mut perf_frames = 0;
    let mut perf_cycles_start = app.cpu.cycles;

    // Signal handler for clean shutdown on Ctrl-C
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        println!("\nCtrl-C received, shutting down...");
        r.store(false, Ordering::SeqCst);
    }).expect("Error setting Ctrl-C handler");

    loop {
        // Check for Ctrl-C
        if !running.load(Ordering::SeqCst) {
            app.flush_disks();
            std::process::exit(0);
        }

        let frame_start = Instant::now();
        let mut cpu_time = Duration::ZERO;

        // When fast disk is active and motor is spinning, run extra cycles
        // per frame to speed through disk I/O (frame pacing still applies)
        let iwm_fast = app.cpu.bus.iou.iwm.fast_disk && app.cpu.bus.iou.iwm.motor_on;
        let effective_cpf = if iwm_fast {
            cycles_per_frame * 8
        } else {
            cycles_per_frame
        };

        if app.window.is_some() {
            let mut cycles_run = 0;
            while cycles_run < effective_cpf {
                if fast_mode && app.cpu.pc == fast_until_addr {
                    println!("Reached fast_until address {:04X}. Switching to normal speed and enabling logging.", fast_until_addr);
                    fast_mode = false;
                    cycles_per_frame = (1.0 * 1_023_000.0 / 60.0) as u64;
                    app.cpu.debug = true;
                }
                
                if !fast_mode && args.log_until.is_some() && app.cpu.pc == log_until_addr {
                    println!("Reached log_until address {:04X}. Exiting.", log_until_addr);
                    std::process::exit(0);
                }

                cycles_run += app.cpu.tick();
            }

            cpu_time = frame_start.elapsed();
            if cpu_time > Duration::from_millis(17) {
                println!("Slow CPU Frame! Took {:?} (Target: 16.6ms)", cpu_time);
            }

            app.cpu.bus.iou.speaker.update(app.cpu.bus.iou.cycles);
        }

        let status = event_loop.pump_app_events(timeout, &mut app);

        if let PumpStatus::Exit(exit_code) = status {
            app.flush_disks();
            std::process::exit(exit_code as i32);
        }

        if let Some(window) = &app.window {
            window.request_redraw();
        }

        perf_frames += 1;
        if perf_start.elapsed() >= Duration::from_secs(1) {
            if args.perf {
                let elapsed = perf_start.elapsed().as_secs_f64();
                let cycles_total = app.cpu.cycles - perf_cycles_start;
                let mhz = cycles_total as f64 / elapsed / 1_000_000.0;
                let fps = perf_frames as f64 / elapsed;
                let cycles_per_frame_avg = cycles_total as f64 / perf_frames as f64;
                
                let (iwm_bytes, iwm_motor, iwm_track, iwm_revs, iwm_overruns) = app.cpu.bus.iou.iwm.get_and_reset_metrics();
                let iwm_kb_sec = (iwm_bytes as f64 / elapsed) / 1024.0;

                println!(
                    "Perf: {:.3} MHz (Target: {:.3} MHz) | {:.1} FPS | CPF: {:.0} | CPU Load: {:.1}% | IWM: {:.1} KB/s (M:{}, T:{}, R:{}, O:{})", 
                    mhz, 
                    args.speed * 1.023, 
                    fps,
                    cycles_per_frame_avg,
                    (cpu_time.as_secs_f64() * 60.0) * 100.0,
                    iwm_kb_sec,
                    if iwm_motor { "ON" } else { "OFF" },
                    iwm_track,
                    iwm_revs,
                    iwm_overruns
                );
            } else {
                app.cpu.bus.iou.iwm.get_and_reset_metrics();
            }

            perf_start = Instant::now();
            perf_frames = 0;
            perf_cycles_start = app.cpu.cycles;
        }

        next_frame_time += target_frame_time;
        let now = Instant::now();
        if now < next_frame_time {
            std::thread::sleep(next_frame_time - now);
        } else if now - next_frame_time > Duration::from_millis(50) {
            next_frame_time = now;
        }
    }
}

fn run_monitor_mode(cpu: &mut CPU) {
    let mut monitor = Monitor::new(cpu);
    monitor.repl();
}

fn run_cpu_console_mode(mut cpu: CPU) {
    // let rom = rom::ROM::load_from_bytes(include_bytes!("../iic.bin"), cpu.system_type).unwrap();
    // cpu.load_rom(rom);
    // cpu.init();

    loop {
        cpu.tick();
        if cpu.bus.interrupts.halted {
            println!("*");
            break;
        }
    }
}

impl App {
    fn flush_disks(&mut self) {
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
        let win_h = emu_h as f64 * 2.0 + (STATUS_BAR_HEIGHT as f64 / 2.0);

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

        // Create pixels buffer at initial window size; GPU handles scaling on resize
        self.pixels = match Pixels::new(window_size.width, window_size.height, surface_texture) {
            Ok(mut pixels) => {
                // Use Fill scaling so fractional scaling works at any resolution
                pixels.set_scaling_mode(ScalingMode::Fill);
                // Create CRT renderer if enabled
                if self.crt_enabled {
                    let surface_format = pixels.render_texture_format();
                    let (src_w, src_h) = self.cpu.bus.video.get_dimensions();
                    self.crt_renderer = Some(CrtRenderer::new(
                        pixels.device(),
                        window_size.width,
                        window_size.height,
                        self.buffer_width,
                        self.buffer_height,
                        STATUS_BAR_HEIGHT,
                        src_w as f32,
                        src_h as f32,
                        surface_format,
                    ));

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
                    // macOS fullscreen transition loses focus; re-assert it
                    // and clear stale modifier state
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                // Always reset modifiers on focus change to avoid stuck keys
                self.modifiers = ModifiersState::empty();
            }

            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    self.surface_width = size.width;
                    self.surface_height = size.height;
                    if let Some(pixels) = self.pixels.as_mut() {
                        if let Err(err) = pixels.resize_surface(size.width, size.height) {
                            error!("pixels.resize_surface failed: {}", err);
                            event_loop.exit();
                        }
                    }
                }
            }

            WindowEvent::RedrawRequested => {
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
                    // Lazily resize buffer to match surface (deferred from Resized
                    // to avoid blocking the event loop during fullscreen transitions)
                    if self.surface_width != self.buffer_width || self.surface_height != self.buffer_height {
                        self.buffer_width = self.surface_width;
                        self.buffer_height = self.surface_height;
                        let _ = pixels.resize_buffer(self.surface_width, self.surface_height);
                        if let Some(crt) = self.crt_renderer.as_mut() {
                            crt.resize(
                                pixels.device(), pixels.queue(),
                                self.surface_width, self.surface_height,
                            );
                        }
                    }

                    let render_start = Instant::now();
                    self.cpu.video_update();

                    let render_time = render_start.elapsed();
                    if render_time > Duration::from_millis(5) {
                      //  println!("Slow Video Render! Took {:?}", render_time);
                    }

                    let (src_w, src_h) = self.cpu.bus.video.get_dimensions();
                    let video_pixels = self.cpu.bus.video.get_pixels();
                    let surf_w = self.buffer_width;
                    let surf_h = self.buffer_height;
                    let bar_h = if self.show_toolbar { STATUS_BAR_HEIGHT } else { 0 };

                    let frame = pixels.frame_mut();

                    // Clear entire frame to black
                    frame.fill(0);
                    // Set alpha to 255 for all pixels
                    for chunk in frame.chunks_exact_mut(4) {
                        chunk[3] = 255;
                    }

                    // Calculate scaled emulator display to fit above the status bar.
                    // Use integer scale so CPU scanlines (every-other-row dimming)
                    // remain uniform — fractional scales cause moiré.
                    let display_region_h = surf_h.saturating_sub(bar_h);
                    let mut blit_offset_x = 0u32;
                    let mut blit_offset_y = 0u32;
                    let mut blit_dst_w = 0u32;
                    let blit_dst_h;
                    if display_region_h > 0 && surf_w > 0 {
                        let scale_x = surf_w as f64 / src_w as f64;
                        let scale_y = display_region_h as f64 / src_h as f64;
                        let scale = scale_x.min(scale_y).floor().max(1.0);

                        blit_dst_w = (src_w as f64 * scale) as u32;
                        blit_dst_h = (src_h as f64 * scale) as u32;
                        blit_offset_x = (surf_w - blit_dst_w) / 2;
                        blit_offset_y = (display_region_h - blit_dst_h) / 2;

                        // Nearest-neighbor blit
                        blit_scaled(
                            frame, surf_w,
                            video_pixels, src_w, src_h,
                            blit_offset_x, blit_offset_y, blit_dst_w, blit_dst_h,
                        );
                    }

                    // Render drive status bar at native resolution at the bottom
                    let drive_status: [DriveStatusInfo; 2] = [
                        {
                            let (has_disk, is_active, wp) = self.cpu.bus.iou.iwm.drive_status(0);
                            DriveStatusInfo { has_disk, is_active, is_write_protected: wp }
                        },
                        {
                            let (has_disk, is_active, wp) = self.cpu.bus.iou.iwm.drive_status(1);
                            DriveStatusInfo { has_disk, is_active, is_write_protected: wp }
                        },
                    ];
                    let col80 = self.cpu.bus.iou.col80_switch;
                    if self.show_toolbar {
                        render_status_bar(frame, surf_w, surf_h, bar_h, &drive_status, col80);
                    }

                    let render_result = if let Some(crt) = &self.crt_renderer {
                        // Compute active area within blit (excluding video border)
                        let border = self.cpu.bus.video.border_size as u32;
                        let blit_scale = if src_w > 0 { blit_dst_w / src_w } else { 1 };
                        let active_offset_x = blit_offset_x + border * blit_scale;
                        let active_offset_y = blit_offset_y + border * blit_scale;
                        let active_w = src_w - border * 2;
                        let active_h = src_h - border * 2;
                        let active_dst_w = active_w * blit_scale;
                        let active_dst_h = active_h * blit_scale;

                        // Update CRT uniforms with active area geometry
                        // Video doubles each scanline to 2 rows (384 pixels for 192 scanlines),
                        // so pass true scanline count (active_h / 2) for beam profile alignment.
                        crt.update_content_rect(
                            pixels.queue(),
                            surf_w, surf_h,
                            active_offset_x, active_offset_y,
                            active_dst_w, active_dst_h,
                            bar_h,
                            active_w as f32, (active_h / 2) as f32,
                        );

                        // Update time uniform for flicker effect
                        let elapsed = self.crt_start_time.elapsed().as_secs_f32();
                        crt.update_time(pixels.queue(), elapsed);
                        crt.update_monochrome(pixels.queue(), self.cpu.bus.video.monochrome);
                        crt.update_shader_params(pixels.queue(), &self.shader_params);

                        // Run egui UI
                        let egui_output = if self.show_shader_ui {
                            if let Some(egui_state) = self.egui_state.as_mut() {
                                let window = self.window.as_ref().unwrap();
                                let raw_input = egui_state.take_egui_input(window.as_ref());
                                let output = self.egui_ctx.run(raw_input, |ctx| {
                                    shader_ui::render_shader_ui(ctx, &mut self.shader_params, &mut self.show_shader_ui);
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

                        // Upload egui textures before render_with
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

                        // Multi-pass: scaling renderer → intermediate → mipmaps → bloom → CRT → egui → screen
                        let render_res = pixels.render_with(|encoder, render_target, context| {
                            // Pass 1: scale pixel buffer into the CRT intermediate texture
                            context.scaling_renderer.render(encoder, crt.intermediate_view());
                            // Pass 2+: generate mipmaps, bloom, CRT composite
                            crt.render(encoder, render_target, device);

                            // Pass 3: egui overlay
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

                        // Free old egui textures after render
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
                        error!("pixels.render() failed: {}", err);
                        event_loop.exit();
                    }
                }
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
                if egui_consumed { return; }
                let pressed = state == ElementState::Pressed;
                if pressed {
                    // Check if click is on a UI element in the status bar
                    if let Some((wx, wy)) = self.last_cursor_pos {
                        // Use the pixels crate's coordinate mapping which correctly
                        // accounts for aspect-ratio-preserving GPU scaling.
                        let mapped = self.pixels.as_ref()
                            .and_then(|p| p.window_pos_to_pixel((wx as f32, wy as f32)).ok());

                        if let Some((bx, by)) = mapped {
                        let px = bx as u32;
                        let py = by as u32;

                            if self.show_toolbar {
                            // Check reset button (warm reset — Ctrl+Reset equivalent)
                            if button == MouseButton::Left && hit_test_reset_button(px, py, self.buffer_height, STATUS_BAR_HEIGHT) {
                                println!("Warm Reset Triggered (RST button)");
                                self.cpu.reset();
                                return;
                            }

                            // Check power button (cold reboot — power cycle equivalent)
                            if button == MouseButton::Left && hit_test_power_button(px, py, self.buffer_height, STATUS_BAR_HEIGHT) {
                                println!("Power Cycle Triggered (PWR button)");
                                self.cpu.power_cycle();
                                // Clear warm-start magic so ROM does a full cold boot
                                self.cpu.bus.write_byte(0x03F4, 0x00);
                                return;
                            }

                            // Check 80/40 column switch toggle
                            if button == MouseButton::Left && hit_test_col_button(px, py, self.buffer_height, STATUS_BAR_HEIGHT) {
                                self.cpu.bus.iou.col80_switch = !self.cpu.bus.iou.col80_switch;
                                println!("Column switch: {}", if self.cpu.bus.iou.col80_switch { "80" } else { "40" });
                                return;
                            }

                            // Write-protect toggle switch
                            if button == MouseButton::Left {
                                if let Some(drive) = hit_test_write_toggle(px, py, self.buffer_width, self.buffer_height, STATUS_BAR_HEIGHT) {
                                    let (has_disk, _, _) = self.cpu.bus.iou.iwm.drive_status(drive);
                                    if has_disk {
                                        self.cpu.bus.iou.iwm.toggle_write_protect(drive);
                                        let (_, _, wp) = self.cpu.bus.iou.iwm.drive_status(drive);
                                        println!("Drive {}: write protect {}", drive + 1, if wp { "ON" } else { "OFF" });
                                    }
                                    return;
                                }
                            }

                            if let Some(drive) = hit_test_drive_icon(px, py, self.buffer_width, self.buffer_height, STATUS_BAR_HEIGHT) {
                                match button {
                                    MouseButton::Left => {
                                        let now = Instant::now();
                                        // Double-click: eject
                                        if let Some((prev_drive, prev_time)) = self.last_drive_click {
                                            if prev_drive == drive && now.duration_since(prev_time) < Duration::from_millis(400) {
                                                self.last_drive_click = None;
                                                let (has_disk, _, _) = self.cpu.bus.iou.iwm.drive_status(drive);
                                                if has_disk {
                                                    self.cpu.bus.iou.iwm.eject_disk(drive);
                                                    println!("Drive {}: ejected", drive + 1);
                                                }
                                                return;
                                            }
                                        }
                                        // Record click; dialog opens after 400ms if no second click
                                        self.last_drive_click = Some((drive, now));
                                        return; // Don't forward to Apple mouse
                                    }
                                    MouseButton::Right => {
                                        // Toggle write protect
                                        let (has_disk, _, _) = self.cpu.bus.iou.iwm.drive_status(drive);
                                        if has_disk {
                                            self.cpu.bus.iou.iwm.toggle_write_protect(drive);
                                            let (_, _, wp) = self.cpu.bus.iou.iwm.drive_status(drive);
                                            println!("Drive {}: write protect {}", drive + 1, if wp { "ON" } else { "OFF" });
                                        }
                                        return; // Don't forward to Apple mouse
                                    }
                                    _ => {}
                                }
                            }
                            } // end if self.show_toolbar
                    }
                    }
                }
                match button {
                    MouseButton::Left => self.cpu.bus.iou.mouse.set_button(0, pressed),
                    MouseButton::Right => self.cpu.bus.iou.mouse.set_button(1, pressed),
                    _ => (),
                }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                // F7 always toggles shader UI, even when egui has focus
                if event.logical_key == Key::Named(NamedKey::F7) && event.state.is_pressed() {
                    if self.crt_enabled {
                        self.show_shader_ui = !self.show_shader_ui;
                        println!("Shader UI: {}", if self.show_shader_ui { "ON" } else { "OFF" });
                    }
                    return;
                }

                if egui_consumed { return; }

                if event.state.is_pressed() {
                    // check for Reset (Control + Backspace/Delete)
                    if event.logical_key == Key::Named(NamedKey::Backspace) && self.modifiers.control_key() {
                        if self.modifiers.super_key() {
                            println!("Hard Reset Triggered (Control + Command + Backspace)");
                            // corrupt power-up byte to force cold boot
                            self.cpu.bus.write_byte(0x03F4, 0x00);
                        } else {
                            println!("Reset Triggered (Control + Backspace)");
                        }
                        self.cpu.reset();
                        return;
                    }

                    let mut key_code: Option<u8> = None;

                    match event.logical_key {
                        Key::Named(NamedKey::ArrowLeft) => key_code = Some(0x08),
                        Key::Named(NamedKey::ArrowRight) => key_code = Some(0x15),
                        Key::Named(NamedKey::ArrowUp) => key_code = Some(0x0B),
                        Key::Named(NamedKey::ArrowDown) => key_code = Some(0x0A),
                        Key::Named(NamedKey::Enter) => key_code = Some(0x0D),
                        Key::Named(NamedKey::Backspace) => key_code = Some(0x08), // Backspace = Left arrow (destructive backspace)
                        Key::Named(NamedKey::Delete) => key_code = Some(0x7F),    // Forward delete = DEL
                        Key::Named(NamedKey::Escape) => {
                             // TODO: escape should be sent to emulator...
                             key_code = Some(0x1B);
                        }
                        _ => {
                            if let Some(virtual_key) = event.logical_key.to_text() {
                                let key_char = virtual_key.chars().next().unwrap_or('\0');
                                // TODO: lowercase?
                                key_code = Some(key_char.to_ascii_uppercase() as u8);
                            }
                        }
                    }

                    if let Some(code) = key_code {
                        self.cpu.bus.iou.last_key.set(code);
                        self.cpu.bus.iou.key_ready.set(true);
                        println!("Key Pressed: 0x{:X}", code);
                    }
                }

                if event.logical_key == Key::Named(NamedKey::F1) && event.state.is_pressed() {
                    println!("F1 Pressed: Entering Monitor Mode");
                    run_monitor_mode(&mut self.cpu);
                }

                if event.logical_key == Key::Named(NamedKey::F3) && event.state.is_pressed() {
                    let current = self.cpu.bus.video.monochrome;
                    self.cpu.bus.video.set_monochrome(!current);
                    println!("Monochrome Mode: {}", if !current { "ON" } else { "OFF" });
                }

                if event.logical_key == Key::Named(NamedKey::F10) && event.state.is_pressed() {
                    let new_debug_state = !self.cpu.debug;
                    self.cpu.debug = new_debug_state;
                    self.cpu.bus.debug = new_debug_state;
                    self.cpu.bus.iou.debug = new_debug_state;
                    self.cpu.bus.iou.iwm.debug = new_debug_state;
                    println!("Debug Logging: {}", if new_debug_state { "ON" } else { "OFF" });
                }

                if event.logical_key == Key::Named(NamedKey::F6) && event.state.is_pressed() {
                    self.show_toolbar = !self.show_toolbar;
                    // Toolbar toggle — content rect will update next frame automatically
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }

            _ => (),
        }
    }
}
