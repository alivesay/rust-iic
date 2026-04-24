//! Apple IIc Emulator
//!
//! A cycle-accurate Apple IIc emulator written in Rust.

#[macro_use]
mod macros;

mod app;
mod audio_mixer;
mod bus;
mod cli;
mod cpu;
mod cpu_monitor;
mod device;
mod disassembler;
mod hooks;
mod interrupts;
mod iou;
mod memory;
mod mmu;
mod monitor;
mod render;
mod rom;
mod util;
mod video;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use pixels::Error;
use winit::event_loop::EventLoop;
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};

use crate::app::{run_monitor_mode, App};
use crate::audio_mixer::AudioMixer;
use crate::cli::{Args, ShaderType};
use crate::cpu::{CpuType, SystemType, CPU};

const BANNER: &str = r#"*
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

fn main() -> Result<(), Error> {
    env_logger::init();

    println!("{}{}", "*\n\n".repeat(8), BANNER);

    let args = Args::parse();

    // Create centralized audio mixer - single cpal stream for all audio sources
    let (_audio_mixer, audio_producers) = AudioMixer::new();
    let sample_rate = _audio_mixer.sample_rate();

    let mut cpu = CPU::new(
        SystemType::AppleIIc,
        CpuType::CMOS65C02,
        (args.speed * 1_023_000.0) as u32,
        args.self_test,
        audio_producers.speaker,
        sample_rate,
    );

    // Initialize drive audio with mixer channel
    cpu.bus.iou.iwm.init_audio(audio_producers.drive_audio, sample_rate);
    println!("Drive audio synthesis enabled");

    // Configure CPU/system based on args
    cpu.debug = args.debug;
    cpu.bus.debug = args.debug;
    cpu.bus.iou.debug = args.debug;
    cpu.bus.iou.iwm.debug = args.debug;
    cpu.bus.iou.iwm.fast_disk = args.fast_disk;
    cpu.bus.video.set_monochrome(args.monochrome);
    cpu.bus.video.shader_enabled = args.shader != ShaderType::None;
    cpu.bus.video.scanline_intensity = args.scanline_intensity;

    // Serial port configuration
    if let Some(ref addr) = args.serial {
        cpu.bus.iou.scc.ch_a.debug = args.debug;
        if let Err(e) = cpu.bus.iou.scc.ch_a.tcp_connect(addr) {
            eprintln!("Failed to connect serial to {}: {}", addr, e);
        }
    }

    if args.modem {
        cpu.bus.iou.scc.ch_a.modem_enabled = true;
        cpu.bus.iou.scc.ch_a.debug = args.debug;
        println!("Virtual Hayes modem enabled on modem port (slot 2)");
        println!("Use ATDT host:port from terminal software to connect");
    }

    if args.serial_loopback {
        cpu.bus.iou.scc.ch_a.loopback = true;
        cpu.bus.iou.scc.ch_b.loopback = true;
        println!("Serial loopback mode enabled on both ports");
    }

    // ZIP CHIP II-8 (Model 8000) accelerator
    if args.zip {
        cpu.bus.iou.set_zip_enabled(true);
        println!("ZIP CHIP II-8 enabled (8MHz) - Press ESC during boot to disable, Ctrl+Z to toggle");
    }

    // Mockingboard sound card in slot 5 (no conflict with memory expansion)
    // Uses centralized audio mixer channel
    if args.mockingboard {
        cpu.bus.iou.mockingboard = crate::device::mockingboard::Mockingboard::with_audio(audio_producers.mockingboard1, sample_rate);
        cpu.bus.iou.set_mockingboard_enabled(true);
        
        // Use timer-based activation - wait for DOS/ProDOS to fully initialize
        // ~3M cycles = ~3 seconds at 1MHz, should be enough for most boot scenarios
        cpu.bus.iou.mockingboard.set_hook_activation(true);
        cpu.hooks.register_mockingboard_hook(1, 3_000_000);  // Slot 5
        
        println!("Mockingboard enabled in slot 5");
    }

    // Second Mockingboard in slot 4 (for Ultima V dual-MB support, disables memory expansion)
    // Uses centralized audio mixer channel
    if args.mockingboard2 {
        cpu.bus.iou.mockingboard2 = crate::device::mockingboard::Mockingboard::with_audio(audio_producers.mockingboard2, sample_rate);
        cpu.bus.iou.set_mockingboard2_enabled(true);
        
        // Use timer-based activation
        cpu.bus.iou.mockingboard2.set_hook_activation(true);
        cpu.hooks.register_mockingboard_hook(0, 3_000_000);  // Slot 4
        
        if args.mockingboard {
            println!("Second Mockingboard enabled in slot 4 (12 total sound channels, memory expansion disabled)");
        } else {
            println!("Mockingboard enabled in slot 4 (memory expansion disabled)");
        }
    }

    // Register ProDOS MLI hooks (logs all ProDOS system calls)
    hooks::register_hooks(&mut cpu.hooks);

    // Load ROM
    let iic_rom_file = include_bytes!("../iic3.bin");
    let iic_rom = rom::ROM::load_from_bytes(iic_rom_file, cpu.system_type).unwrap();
    cpu.load_rom(iic_rom);
    cpu.init();

    // Load disks
    let disk_path = args.disk.clone().or_else(|| {
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

    // Load 3.5" disk images (ProDOS order / 2IMG)
    if let Some(path) = &args.disk35 {
        match cpu.bus.iou.iwm.load_disk35(path) {
            Ok(()) => {
                println!("3.5\" drive 1: {}", path);
            }
            Err(e) => {
                eprintln!("Failed to load 3.5\" disk '{}': {}", path, e);
            }
        }
    }

    // Load hard drive images (HDV) into SmartPort device chain
    if let Some(path) = &args.hdv {
        match cpu.bus.iou.iwm.smartport.load_hdv(path) {
            Ok(()) => {
                let dev = &cpu.bus.iou.iwm.smartport.hdv_devices[0];
                println!("Hard drive 1: {} ({} blocks)", path, dev.block_count);
            }
            Err(e) => {
                eprintln!("Failed to load HDV '{}': {}", path, e);
            }
        }
    }

    if let Some(path) = &args.hdv2 {
        match cpu.bus.iou.iwm.smartport.load_hdv(path) {
            Ok(()) => {
                let dev = &cpu.bus.iou.iwm.smartport.hdv_devices[1];
                println!("Hard drive 2: {} ({} blocks)", path, dev.block_count);
            }
            Err(e) => {
                eprintln!("Failed to load HDV2 '{}': {}", path, e);
            }
        }
    }

    // Monitor mode
    if args.monitor {
        run_monitor_mode(&mut cpu);
    }

    // Headless mode
    if args.no_video {
        run_headless(cpu);
        return Ok(());
    }

    // GUI mode
    run_gui(cpu, &args)
}

/// Run emulator in headless (no video) mode.
fn run_headless(mut cpu: CPU) {
    loop {
        cpu.tick();
        if cpu.bus.interrupts.halted {
            println!("*");
            break;
        }
    }
}

fn run_gui(cpu: CPU, args: &Args) -> Result<(), Error> {
    let mut event_loop = EventLoop::new().unwrap();
    let mut app = App::new(cpu, args.shader, args.fullscreen);

    let timeout = Some(Duration::ZERO);
    let target_frame_time = Duration::from_micros(16667); // ~60Hz

    let mut fast_mode = args.fast_until.is_some();
    let fast_until_addr = args
        .fast_until
        .as_ref()
        .and_then(|s| u16::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);
    let log_until_addr = args
        .log_until
        .as_ref()
        .and_then(|s| u16::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);

    let mut cycles_per_frame = if fast_mode {
        (args.fast_speed * 1_023_000.0 / 60.0) as u64
    } else {
        (args.speed * 1_023_000.0 / 60.0) as u64
    };

    let mut next_frame_time = Instant::now();

    let mut perf_start = Instant::now();
    let mut perf_frames = 0u64;
    let mut perf_cycles_start = app.cpu.cycles;

    // Ctrl-C
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        println!("\nCtrl-C received, shutting down...");
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    loop {
        if !running.load(Ordering::SeqCst) {
            app.flush_disks();
            std::process::exit(0);
        }

        let frame_start = Instant::now();
        let mut cpu_time = Duration::ZERO;

        // Fast disk mode: run extra cycles when motor spinning AND not writing
        let iwm = &app.cpu.bus.iou.iwm;
        let iwm_fast = iwm.fast_disk && iwm.motor_on && !iwm.write_mode;
        
        // ZIP CHIP: multiply effective cycles when accelerated
        let zip_multiplier = app.cpu.bus.iou.zip.speed_multiplier() as u64;
        
        let effective_cpf = if iwm_fast {
            cycles_per_frame * 8
        } else {
            cycles_per_frame * zip_multiplier
        };

        if app.window.is_some() {
            let mut cycles_run = 0;
            while cycles_run < effective_cpf {
                if fast_mode && app.cpu.pc == fast_until_addr {
                    println!(
                        "Reached fast_until address {:04X}. Switching to normal speed.",
                        fast_until_addr
                    );
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
            app.cpu.bus.iou.speaker.update(app.cpu.bus.iou.cycles);
            app.cpu.bus.iou.mockingboard.update(app.cpu.bus.iou.cycles);
            app.cpu.bus.iou.mockingboard2.update(app.cpu.bus.iou.cycles);
            app.cpu.bus.iou.iwm.update_audio();
        }

        let status = event_loop.pump_app_events(timeout, &mut app);

        if let PumpStatus::Exit(exit_code) = status {
            app.flush_disks();
            std::process::exit(exit_code as i32);
        }

        if let Some(window) = &app.window {
            window.request_redraw();
        }

        // Performance metrics
        perf_frames += 1;
        if perf_start.elapsed() >= Duration::from_secs(1) {
            if args.perf {
                let elapsed = perf_start.elapsed().as_secs_f64();
                let cycles_total = app.cpu.cycles - perf_cycles_start;
                let mhz = cycles_total as f64 / elapsed / 1_000_000.0;
                let fps = perf_frames as f64 / elapsed;
                let cycles_per_frame_avg = cycles_total as f64 / perf_frames as f64;

                let (iwm_bytes, iwm_motor, iwm_track, iwm_revs, iwm_overruns) =
                    app.cpu.bus.iou.iwm.get_and_reset_metrics();
                let iwm_kb_sec = (iwm_bytes as f64 / elapsed) / 1024.0;

                println!(
                    "Perf: {:.3} MHz | {:.1} FPS | CPF: {:.0} | CPU: {:.1}% | IWM: {:.1} KB/s (M:{}, T:{}, R:{}, O:{})",
                    mhz,
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

        // Frame pacing
        next_frame_time += target_frame_time;
        let now = Instant::now();
        if now < next_frame_time {
            std::thread::sleep(next_frame_time - now);
        } else if now - next_frame_time > Duration::from_millis(50) {
            next_frame_time = now;
        }
    }
}
