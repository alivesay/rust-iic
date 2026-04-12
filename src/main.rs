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
mod rom;
mod util;
mod video;


use crate::cpu::CPU;
use crate::monitor::Monitor;
use crate::video::DriveStatusInfo;
use clap::Parser;
use cpu::{CpuType, SystemType};
use log::error;
use pixels::{Error, Pixels, SurfaceTexture};
use std::{
    sync::Arc,
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
}

/// Height of the native-resolution status bar at the bottom of the window.
const STATUS_BAR_HEIGHT: u32 = 96;

pub struct App {
    pixels: Option<Pixels<'static>>,
    window: Option<Arc<Window>>,
    cpu: CPU,
    surface_width: u32,
    surface_height: u32,
    modifiers: ModifiersState,
    last_cursor_pos: Option<(f64, f64)>,
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
    cpu.bus.video.set_monochrome(args.monochrome);
    cpu.bus.video.scanline_intensity = args.scanline_intensity;

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
    let mut app = App {
        pixels: None,
        window: None,
        cpu,
        surface_width: width * 2,
        surface_height: height * 2,
        modifiers: ModifiersState::default(),
        last_cursor_pos: None,
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

    loop {
        let frame_start = Instant::now();
        let mut cpu_time = Duration::ZERO;

        // When fast disk is active and motor is spinning, run many more cycles
        // per frame to speed through disk I/O without wall-clock delay
        let iwm_fast = app.cpu.bus.iou.iwm.fast_disk && app.cpu.bus.iou.iwm.motor_on;
        let effective_cpf = if iwm_fast && !fast_mode {
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
        if !iwm_fast && now < next_frame_time {
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

impl winit::application::ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let (emu_w, emu_h) = self.cpu.bus.video.get_dimensions();

        // Window size: emulator width at 1x logical, height at 2x + status bar.
        // STATUS_BAR_HEIGHT is in physical pixels; convert to logical by dividing
        // by the expected scale factor (2.0 on Retina).
        let win_w = emu_w as f64;
        let win_h = emu_h as f64 * 2.0 + (STATUS_BAR_HEIGHT as f64 / 2.0);

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Apple //c Emulator")
                        .with_inner_size(LogicalSize::new(win_w, win_h)),
                )
                .unwrap(),
        );

        self.window = Some(window.clone());

        let window_size = window.inner_size();
        self.surface_width = window_size.width;
        self.surface_height = window_size.height;
        let surface_texture =
            SurfaceTexture::new(window_size.width, window_size.height, window.clone());

        // Create pixels buffer at window size so we control scaling ourselves
        self.pixels = match Pixels::new(window_size.width, window_size.height, surface_texture) {
            Ok(pixels) => {
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
        match event {
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }

            WindowEvent::CloseRequested => {
                event_loop.exit();
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
                        if let Err(err) = pixels.resize_buffer(size.width, size.height) {
                            error!("pixels.resize_buffer failed: {}", err);
                            event_loop.exit();
                        }
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                if let Some(pixels) = self.pixels.as_mut() {
                    let render_start = Instant::now();
                    self.cpu.video_update();

                    let render_time = render_start.elapsed();
                    if render_time > Duration::from_millis(5) {
                      //  println!("Slow Video Render! Took {:?}", render_time);
                    }

                    let (src_w, src_h) = self.cpu.bus.video.get_dimensions();
                    let video_pixels = self.cpu.bus.video.get_pixels();
                    let surf_w = self.surface_width;
                    let surf_h = self.surface_height;
                    let bar_h = STATUS_BAR_HEIGHT;

                    let frame = pixels.frame_mut();

                    // Clear entire frame to black
                    frame.fill(0);
                    // Set alpha to 255 for all pixels
                    for chunk in frame.chunks_exact_mut(4) {
                        chunk[3] = 255;
                    }

                    // Calculate scaled emulator display to fit above the status bar
                    let display_region_h = surf_h.saturating_sub(bar_h);
                    if display_region_h > 0 && surf_w > 0 {
                        let scale_x = surf_w as f64 / src_w as f64;
                        let scale_y = display_region_h as f64 / src_h as f64;
                        let scale = scale_x.min(scale_y);

                        let dst_w = (src_w as f64 * scale) as u32;
                        let dst_h = (src_h as f64 * scale) as u32;
                        let offset_x = (surf_w - dst_w) / 2;
                        let offset_y = (display_region_h - dst_h) / 2;

                        // Nearest-neighbor blit
                        blit_scaled(
                            frame, surf_w,
                            video_pixels, src_w, src_h,
                            offset_x, offset_y, dst_w, dst_h,
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
                    render_status_bar(frame, surf_w, surf_h, bar_h, &drive_status, col80);

                    if let Err(err) = pixels.render() {
                        error!("pixels.render() failed: {}", err);
                        event_loop.exit();
                    }
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                let x = position.x;
                let y = position.y;
                if let Some((lx, ly)) = self.last_cursor_pos {
                    let dx = x - lx;
                    let dy = y - ly;
                    self.cpu.bus.iou.mouse.add_delta(dx, dy);
                }
                self.last_cursor_pos = Some((x, y));
            }

            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                if pressed {
                    // Check if click is on a drive icon in the status bar
                    if let Some((wx, wy)) = self.last_cursor_pos {
                        if let Some(window) = &self.window {
                            let win_size = window.inner_size();
                            // CursorMoved position is already in physical pixels
                            let px = wx as u32;
                            let py = wy as u32;

                            // Check reset button (warm reset — Ctrl+Reset equivalent)
                            if button == MouseButton::Left && hit_test_reset_button(px, py, win_size.height, STATUS_BAR_HEIGHT) {
                                println!("Warm Reset Triggered (RST button)");
                                self.cpu.reset();
                                return;
                            }

                            // Check power button (cold reboot — power cycle equivalent)
                            if button == MouseButton::Left && hit_test_power_button(px, py, win_size.height, STATUS_BAR_HEIGHT) {
                                println!("Power Cycle Triggered (PWR button)");
                                self.cpu.power_cycle();
                                return;
                            }

                            // Check 80/40 column switch toggle
                            if button == MouseButton::Left && hit_test_col_button(px, py, win_size.height, STATUS_BAR_HEIGHT) {
                                self.cpu.bus.iou.col80_switch = !self.cpu.bus.iou.col80_switch;
                                println!("Column switch: {}", if self.cpu.bus.iou.col80_switch { "80" } else { "40" });
                                return;
                            }

                            if let Some(drive) = hit_test_drive_icon(px, py, win_size.width, win_size.height, STATUS_BAR_HEIGHT) {
                                match button {
                                    MouseButton::Left => {
                                        // Open file dialog to load a disk
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
                        Key::Named(NamedKey::Backspace) => key_code = Some(0x7F), // Delete
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
            }

            _ => (),
        }
    }
}

/// Nearest-neighbor blit from src into frame at (dst_x, dst_y) scaled to (dst_w × dst_h).
fn blit_scaled(
    frame: &mut [u8], frame_w: u32,
    src: &[u8], src_w: u32, src_h: u32,
    dst_x: u32, dst_y: u32, dst_w: u32, dst_h: u32,
) {
    for y in 0..dst_h {
        let sy = (y as u64 * src_h as u64 / dst_h as u64) as usize;
        let src_row = sy * src_w as usize * 4;
        let dst_row = (dst_y + y) as usize * frame_w as usize * 4;
        for x in 0..dst_w {
            let sx = (x as u64 * src_w as u64 / dst_w as u64) as usize;
            let si = src_row + sx * 4;
            let di = dst_row + (dst_x + x) as usize * 4;
            if si + 4 <= src.len() && di + 4 <= frame.len() {
                frame[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
}

/// Draw the drive status bar at native resolution into the bottom bar_h rows of frame.
fn render_status_bar(
    frame: &mut [u8], surf_w: u32, surf_h: u32, bar_h: u32,
    drives: &[DriveStatusInfo; 2], col80: bool,
) {
    let bar_y = surf_h.saturating_sub(bar_h);

    // Dark gray background
    let bg = [32u8, 32, 32, 255];
    for y in bar_y..surf_h {
        for x in 0..surf_w {
            let idx = (y * surf_w + x) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(&bg);
            }
        }
    }

    // Separator line
    let sep = [64u8, 64, 64, 255];
    for x in 0..surf_w {
        let idx = (bar_y * surf_w + x) as usize * 4;
        if idx + 4 <= frame.len() {
            frame[idx..idx + 4].copy_from_slice(&sep);
        }
    }

    // Reset buttons on the left side
    let (rx, ry, rw, rh) = reset_button_rect(bar_y, bar_h);
    draw_button(frame, surf_w, rx, ry, rw, rh, b"RST", &[200, 200, 200, 255]);
    let (px2, py2, pw2, ph2) = power_button_rect(bar_y, bar_h);
    draw_button(frame, surf_w, px2, py2, pw2, ph2, b"PWR", &[255, 100, 100, 255]);
    // 80/40 column switch toggle
    let (cx, cy, cw, ch) = col_button_rect(bar_y, bar_h);
    let col_label = if col80 { b"80" as &[u8] } else { b"40" as &[u8] };
    let col_color = if col80 { [100, 200, 100, 255] } else { [200, 200, 100, 255] };
    draw_button(frame, surf_w, cx, cy, cw, ch, col_label, &col_color);

    // Drive slots in the bottom-right (4x base scale for Retina)
    let slot_width: u32 = 136;
    let total_slots_width = slot_width * 2 + 32;
    let start_x = surf_w.saturating_sub(total_slots_width + 32);

    for drive in 0..2usize {
        let slot_x = start_x + drive as u32 * (slot_width + 32);
        let slot_y = bar_y + (bar_h.saturating_sub(56)) / 2;

        // LED indicator (24×24)
        let led_color = if drives[drive].is_active {
            [0u8, 255, 0, 255]
        } else if drives[drive].has_disk {
            [0u8, 64, 0, 255]
        } else {
            [48u8, 48, 48, 255]
        };
        let led_x = slot_x;
        let led_y = slot_y + 16;
        for dy in 0..24u32 {
            for dx in 0..24u32 {
                let idx = ((led_y + dy) * surf_w + (led_x + dx)) as usize * 4;
                if idx + 4 <= frame.len() {
                    frame[idx..idx + 4].copy_from_slice(&led_color);
                }
            }
        }

        // Disk icon (64×56)
        let icon_x = slot_x + 40;
        let icon_y = slot_y;
        let disk_color: [u8; 4] = if drives[drive].has_disk {
            [180, 180, 180, 255]
        } else {
            [80, 80, 80, 255]
        };
        draw_disk_icon(frame, surf_w, icon_x, icon_y, &disk_color);

        // Write-protect indicator
        if drives[drive].has_disk && drives[drive].is_write_protected {
            let lock = [255u8, 80, 80, 255];
            let lx = icon_x + 4;
            let ly = icon_y + 4;
            for dy in 0..20u32 {
                for dx in 0..4u32 {
                    let idx = ((ly + dy) * surf_w + (lx + dx)) as usize * 4;
                    if idx + 4 <= frame.len() {
                        frame[idx..idx + 4].copy_from_slice(&lock);
                    }
                }
            }
            for dx in 4..12u32 {
                for dy2 in 0..4u32 {
                    let idx = ((ly + 16 + dy2) * surf_w + (lx + dx)) as usize * 4;
                    if idx + 4 <= frame.len() {
                        frame[idx..idx + 4].copy_from_slice(&lock);
                    }
                }
            }
        }

        // Drive number label
        let label_x = icon_x + 72;
        let label_y = slot_y + 18;
        let label_color = [128u8, 128, 128, 255];
        draw_tiny_digit(frame, surf_w, label_x, label_y, (drive + 1) as u8, &label_color);
    }
}

fn draw_disk_icon(frame: &mut [u8], stride: u32, x: u32, y: u32, color: &[u8; 4]) {
    let dark = [color[0] / 2, color[1] / 2, color[2] / 2, 255];
    let slot_color = [color[0] / 3, color[1] / 3, color[2] / 3, 255];

    // Body (64×56)
    for dy in 0..56u32 {
        for dx in 0..64u32 {
            let idx = ((y + dy) * stride + (x + dx)) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(color);
            }
        }
    }
    // Top label area
    for dy in 4..20u32 {
        for dx in 12..52u32 {
            let idx = ((y + dy) * stride + (x + dx)) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(&dark);
            }
        }
    }
    // Bottom slot
    for dy in 36..52u32 {
        for dx in 16..48u32 {
            let idx = ((y + dy) * stride + (x + dx)) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(&slot_color);
            }
        }
    }
    // Metal shutter
    let shutter = [color[0].saturating_add(40), color[1].saturating_add(40), color[2].saturating_add(40), 255];
    for dy in 36..52u32 {
        for dx in 28..32u32 {
            let idx = ((y + dy) * stride + (x + dx)) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(&shutter);
            }
        }
    }
}

fn draw_tiny_digit(frame: &mut [u8], stride: u32, x: u32, y: u32, digit: u8, color: &[u8; 4]) {
    #[rustfmt::skip]
    let patterns: &[&[u8; 15]] = &[
        &[1,1,1, 1,0,1, 1,0,1, 1,0,1, 1,1,1], // 0
        &[0,1,0, 1,1,0, 0,1,0, 0,1,0, 1,1,1], // 1
        &[1,1,1, 0,0,1, 1,1,1, 1,0,0, 1,1,1], // 2
    ];
    let idx = match digit { 1 => 1, 2 => 2, _ => 0 };
    let pattern = patterns[idx];
    // Each logical pixel is 4×4 physical pixels for Retina
    for dy in 0..5u32 {
        for dx in 0..3u32 {
            if pattern[(dy * 3 + dx) as usize] == 1 {
                for sy in 0..4u32 {
                    for sx in 0..4u32 {
                        let pi = ((y + dy * 4 + sy) * stride + (x + dx * 4 + sx)) as usize * 4;
                        if pi + 4 <= frame.len() {
                            frame[pi..pi + 4].copy_from_slice(color);
                        }
                    }
                }
            }
        }
    }
}

/// Returns (x, y, w, h) for the reset button.
fn reset_button_rect(bar_y: u32, bar_h: u32) -> (u32, u32, u32, u32) {
    let margin = 32u32;
    let btn_w = 128u32;
    let btn_h = 56u32;
    let bx = margin;
    let by = bar_y + (bar_h.saturating_sub(btn_h)) / 2;
    (bx, by, btn_w, btn_h)
}

/// Returns (x, y, w, h) for the power/reboot button (right of RST).
fn power_button_rect(bar_y: u32, bar_h: u32) -> (u32, u32, u32, u32) {
    let (rx, _, rw, _) = reset_button_rect(bar_y, bar_h);
    let gap = 16u32;
    let btn_w = 128u32;
    let btn_h = 56u32;
    let bx = rx + rw + gap;
    let by = bar_y + (bar_h.saturating_sub(btn_h)) / 2;
    (bx, by, btn_w, btn_h)
}

/// Returns (x, y, w, h) for the 80/40 column switch button (right of PWR).
fn col_button_rect(bar_y: u32, bar_h: u32) -> (u32, u32, u32, u32) {
    let (px, _, pw, _) = power_button_rect(bar_y, bar_h);
    let gap = 16u32;
    let btn_w = 96u32;
    let btn_h = 56u32;
    let bx = px + pw + gap;
    let by = bar_y + (bar_h.saturating_sub(btn_h)) / 2;
    (bx, by, btn_w, btn_h)
}

/// Draw a button at (x, y) with size (w, h) and a 3-char label.
fn draw_button(frame: &mut [u8], stride: u32, x: u32, y: u32, w: u32, h: u32, label: &[u8], text_color: &[u8; 4]) {
    // Button outline
    let border = [100u8, 100, 100, 255];
    let fill = [56u8, 56, 56, 255];
    // Fill
    for dy in 1..h - 1 {
        for dx in 1..w - 1 {
            let idx = ((y + dy) * stride + (x + dx)) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(&fill);
            }
        }
    }
    // Top/bottom border
    for dx in 0..w {
        let t = ((y) * stride + (x + dx)) as usize * 4;
        let b = ((y + h - 1) * stride + (x + dx)) as usize * 4;
        if t + 4 <= frame.len() { frame[t..t + 4].copy_from_slice(&border); }
        if b + 4 <= frame.len() { frame[b..b + 4].copy_from_slice(&border); }
    }
    // Left/right border
    for dy in 0..h {
        let l = ((y + dy) * stride + x) as usize * 4;
        let r = ((y + dy) * stride + (x + w - 1)) as usize * 4;
        if l + 4 <= frame.len() { frame[l..l + 4].copy_from_slice(&border); }
        if r + 4 <= frame.len() { frame[r..r + 4].copy_from_slice(&border); }
    }
    // Draw label text centered (3 chars × 12px wide + 4px gap = 44px, centered in 128px)
    let char_w = 16u32;
    let num_chars = label.len() as u32;
    let text_total_w = num_chars * char_w - 4; // subtract trailing gap
    let text_x = x + (w.saturating_sub(text_total_w)) / 2;
    let text_y = y + (h - 20) / 2;
    for (i, &ch) in label.iter().enumerate() {
        draw_tiny_letter(frame, stride, text_x + i as u32 * char_w, text_y, ch, text_color);
    }
}

/// Draw a 4x-scaled 3×5 letter at (x, y). Supports R, S, T.
fn draw_tiny_letter(frame: &mut [u8], stride: u32, x: u32, y: u32, ch: u8, color: &[u8; 4]) {
    #[rustfmt::skip]
    let pattern: &[u8; 15] = match ch {
        b'R' => &[1,1,0, 1,0,1, 1,1,0, 1,0,1, 1,0,1],
        b'S' => &[0,1,1, 1,0,0, 0,1,0, 0,0,1, 1,1,0],
        b'T' => &[1,1,1, 0,1,0, 0,1,0, 0,1,0, 0,1,0],
        b'P' => &[1,1,0, 1,0,1, 1,1,0, 1,0,0, 1,0,0],
        b'W' => &[1,0,1, 1,0,1, 1,0,1, 1,1,1, 1,0,1],
        b'0' | b'O' => &[1,1,1, 1,0,1, 1,0,1, 1,0,1, 1,1,1],
        b'4' => &[1,0,1, 1,0,1, 1,1,1, 0,0,1, 0,0,1],
        b'8' => &[1,1,1, 1,0,1, 1,1,1, 1,0,1, 1,1,1],
        _    => &[0,0,0, 0,0,0, 0,0,0, 0,0,0, 0,0,0],
    };
    for dy in 0..5u32 {
        for dx in 0..3u32 {
            if pattern[(dy * 3 + dx) as usize] == 1 {
                for sy in 0..4u32 {
                    for sx in 0..4u32 {
                        let pi = ((y + dy * 4 + sy) * stride + (x + dx * 4 + sx)) as usize * 4;
                        if pi + 4 <= frame.len() {
                            frame[pi..pi + 4].copy_from_slice(color);
                        }
                    }
                }
            }
        }
    }
}

/// Hit-test the reset button. Returns true if (px, py) is inside it.
fn hit_test_reset_button(px: u32, py: u32, surf_h: u32, bar_h: u32) -> bool {
    let bar_y = surf_h.saturating_sub(bar_h);
    let (rx, ry, rw, rh) = reset_button_rect(bar_y, bar_h);
    px >= rx && px < rx + rw && py >= ry && py < ry + rh
}

/// Hit-test the power/reboot button. Returns true if (px, py) is inside it.
fn hit_test_power_button(px: u32, py: u32, surf_h: u32, bar_h: u32) -> bool {
    let bar_y = surf_h.saturating_sub(bar_h);
    let (bx, by, bw, bh) = power_button_rect(bar_y, bar_h);
    px >= bx && px < bx + bw && py >= by && py < by + bh
}

/// Hit-test the 80/40 column switch button.
fn hit_test_col_button(px: u32, py: u32, surf_h: u32, bar_h: u32) -> bool {
    let bar_y = surf_h.saturating_sub(bar_h);
    let (bx, by, bw, bh) = col_button_rect(bar_y, bar_h);
    px >= bx && px < bx + bw && py >= by && py < by + bh
}

/// Hit-test drive icons in the status bar using native window coordinates.
fn hit_test_drive_icon(px: u32, py: u32, surf_w: u32, surf_h: u32, bar_h: u32) -> Option<usize> {
    let bar_y = surf_h.saturating_sub(bar_h);
    if py < bar_y || py >= surf_h {
        return None;
    }

    let slot_width: u32 = 136;
    let total_slots_width = slot_width * 2 + 32;
    let start_x = surf_w.saturating_sub(total_slots_width + 32);

    for drive in 0..2u32 {
        let slot_x = start_x + drive * (slot_width + 32);
        if px >= slot_x && px < slot_x + slot_width {
            return Some(drive as usize);
        }
    }
    None
}
