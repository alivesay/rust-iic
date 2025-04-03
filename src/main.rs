#[macro_use]
mod macros;

mod bus;
mod cpu;
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
    event::{KeyEvent, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
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
}

pub struct App {
    pixels: Option<Pixels<'static>>,
    window: Option<Arc<Window>>,
    cpu: CPU,
    last_width: u32,
    last_height: u32,
}

fn main() -> Result<(), Error> {
    env_logger::init();

    println!("{}{}", "*\n\n".repeat(8), O_O);

    let args = Args::parse();

    let mut cpu = CPU::new(SystemType::AppleIIc, CpuType::CMOS65C02, 1_000_000);

    let iic_rom_file = include_bytes!("../iic3.bin");
    let iic_rom = rom::ROM::load_from_bytes(iic_rom_file, cpu.system_type).unwrap();

    cpu.load_rom(iic_rom);
    cpu.init();

    if args.monitor {
        run_monitor_mode(&mut cpu);
        return Ok(());
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
    };

    let mut last_frame_time = Instant::now();
    let timeout = Some(Duration::ZERO);

    loop {
        app.cpu.tick();

        let now = Instant::now();
        let elapsed = now.duration_since(last_frame_time);

        if elapsed.as_millis() >= 5 {
            let status = event_loop.pump_app_events(timeout, &mut app);

            if let PumpStatus::Exit(exit_code) = status {
                std::process::exit(exit_code as i32);
            }

            if elapsed.as_secs_f64() >= 1.0 / 60.0 {
                if let Some(window) = &app.window {
                    window.request_redraw();
                }
                last_frame_time = now;
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
                        .with_inner_size(LogicalSize::new((width * 2) as f64, (height * 2) as f64)),
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
            WindowEvent::CloseRequested
            | WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Named(NamedKey::Escape),
                        ..
                    },
                ..
            } => {
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

            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(virtual_key) = event.logical_key.to_text() {
                    let key_char = virtual_key.chars().next().unwrap_or('\0') as u8;

                    self.cpu.bus.iou.last_key.set(key_char);
                    self.cpu.bus.iou.key_ready.set(true);

                    println!("Key Pressed: {} (0x{:X})", key_char as char, key_char);
                }
            }

            _ => (),
        }
    }
}
