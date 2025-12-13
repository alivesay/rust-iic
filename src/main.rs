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
}

pub struct App {
    pixels: Option<Pixels<'static>>,
    window: Option<Arc<Window>>,
    cpu: CPU,
    last_width: u32,
    last_height: u32,
    modifiers: ModifiersState,
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
        println!("Loading disk: {}", path);
        cpu.bus.iou.iwm.load_disk(path).unwrap();
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
        last_width: width,
        last_height: height,
        modifiers: ModifiersState::default(),
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

        if app.window.is_some() {
            let mut cycles_run = 0;
            while cycles_run < cycles_per_frame {
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
            
            // trigger VBL Interrupt at end of frame
            app.cpu.bus.iou.mouse.vbl_int.set(true);

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
        if now < next_frame_time {
            std::thread::sleep(next_frame_time - now);
        } else {
            if now - next_frame_time > Duration::from_millis(50) {
                next_frame_time = now;
            }
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
        let (width, height) = self.cpu.bus.video.get_dimensions();

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Apple //c Emulator")
                        .with_inner_size(LogicalSize::new(width as f64, (height * 2) as f64)),
                )
                .unwrap(),
        );

        self.window = Some(window.clone());

        let window_size = window.inner_size();
        let surface_texture =
            SurfaceTexture::new(window_size.width, window_size.height, window.clone());

        self.pixels = match Pixels::new(width, height, surface_texture) {
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
                if let Some(pixels) = self.pixels.as_mut() {
                    if let Err(err) = pixels.resize_surface(size.width, size.height) {
                        error!("pixels.resize_surface failed: {}", err);
                        event_loop.exit();
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                if let Some(pixels) = self.pixels.as_mut() {
                    // update video buffer only on redraw
                    let render_start = Instant::now();
                    self.cpu.video_update();
                    let render_time = render_start.elapsed();
                    if render_time > Duration::from_millis(5) {
                      //  println!("Slow Video Render! Took {:?}", render_time);
                    }

                    let (width, height) = self.cpu.bus.video.get_dimensions();

                    if width != self.last_width || height != self.last_height {
                        println!("Resizing buffer to {}x{}", width, height);
                        if let Err(err) = pixels.resize_buffer(width, height) {
                            error!("pixels.resize_buffer failed: {}", err);
                            event_loop.exit();
                        }
                        self.last_width = width;
                        self.last_height = height;
                    }

                    let frame = pixels.frame_mut();
                    let video_pixels = self.cpu.bus.video.get_pixels();

                    if frame.len() == video_pixels.len() {
                        frame.copy_from_slice(video_pixels);
                    } else {
                        error!(
                            "Framebuffer size mismatch! pixels.frame_mut() = {}, video.get_pixels() = {}",
                            frame.len(),
                            video_pixels.len()
                        );
                    }

                    if let Err(err) = pixels.render() {
                        error!("pixels.render() failed: {}", err);
                        event_loop.exit();
                    }
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                if let Some(pixels) = self.pixels.as_ref() {
                    if let Ok((x, y)) = pixels.window_pos_to_pixel((position.x as f32, position.y as f32)) {
                        self.cpu.bus.iou.mouse.set_xy(x as u16, y as u16);
                    }
                }
            }

            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
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
