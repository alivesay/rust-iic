#[macro_use]
mod macros;

mod app;
mod audio_mixer;
mod bus;
mod cli;
mod config;
mod cpu;
mod cpu_monitor;
mod cpu_monitor_window;
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
mod settings_window;
mod timing;
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

    let config = config::Config::load();

    let (sample_rate, audio_producers, audio_controls, _audio_mixer, _dummy_mixer);
    if args.no_audio {
        let (dm, ap) = audio_mixer::DummyAudioMixer::new();
        sample_rate = dm.sample_rate();
        audio_producers = ap;
        audio_controls = Some(dm.controls());
        _dummy_mixer = Some(dm);
        _audio_mixer = None;
        println!("audio {:>12} {:>8}", "MIXER", "OFFLINE");
    } else {
        let (am, ap) = AudioMixer::new();
        sample_rate = am.sample_rate();
        audio_producers = ap;
        audio_controls = Some(am.controls());
        _audio_mixer = Some(am);
        _dummy_mixer = None;
        println!("audio {:>12} {:>8}    {} Hz", "MIXER", "ONLINE", sample_rate);
    }

    let mut cpu = CPU::new(
        SystemType::AppleIIc,
        CpuType::CMOS65C02,
        (args.speed as f64 * timing::CYCLES_PER_SECOND) as u32,
        args.self_test,
        audio_producers.speaker,
        sample_rate,
    );

    cpu.bus.iou.iwm.init_audio(audio_producers.drive_audio, sample_rate);
    println!("audio {:>12} {:>8}", "DRIVE_SYNTH", "ONLINE");

    println!("video {:>12} {:>8}    560x384 NTSC", "DISPLAY", "ONLINE");
    println!("serial{:>12} {:>8}    Z8530 SCC", "PORT_1+2", "ONLINE");
    println!("input {:>12} {:>8}", "KEYBOARD", "ONLINE");
    println!("input {:>12} {:>8}", "MOUSE", "ONLINE");

    cpu.debug = args.debug;
    cpu.bus.debug = args.debug;
    cpu.bus.iou.debug = args.debug;
    cpu.bus.iou.iwm.debug = args.debug;
    cpu.bus.iou.iwm.fast_disk = args.fast_disk;
    cpu.bus.video.set_monochrome(args.monochrome || config.display.monochrome);
    cpu.bus.video.set_mono_colors(config.display.mono_fg, config.display.mono_bg);
    cpu.bus.video.shader_enabled = args.shader != ShaderType::None;
    cpu.bus.video.scanline_intensity = args.scanline_intensity;
    cpu.bus.video.effects.chroma_blur = !args.no_chroma_blur;
    cpu.bus.video.effects.comb_filter = !args.no_comb_filter;
    cpu.bus.video.effects.phosphor_spread = !args.no_phosphor_spread;

    if let Some(ref addr) = args.serial {
        cpu.bus.iou.scc.ch_a.debug = args.debug;
        if let Err(e) = cpu.bus.iou.scc.ch_a.tcp_connect(addr) {
            eprintln!("serial {:>12} {:>8}    {}", "SCC_A", "ERROR", e);
        }
    }

    if args.modem {
        cpu.bus.iou.scc.ch_a.modem.enabled = true;
        cpu.bus.iou.scc.ch_a.debug = args.debug;
        println!("serial {:>11} {:>8}    Hayes modem on slot 2", "MODEM", "ONLINE");
    }

    if args.serial_loopback {
        cpu.bus.iou.scc.ch_a.loopback = true;
        cpu.bus.iou.scc.ch_b.loopback = true;
        println!("serial {:>12} {:>8}", "LOOPBACK", "ONLINE");
    }

    if args.zip {
        cpu.bus.iou.set_zip_enabled(true);
        println!("accel {:>12} {:>8}    8 MHz", "ZIP_II-8", "ONLINE");
    }

    if !args.mockingboard2 {
        println!("slot4 {:>12} {:>8}    1024 KB Slinky", "MEMEXP", "ONLINE");
        // Battery-backed RAM
        let path = crate::config::memexp_path();
        cpu.bus.iou.memexp.load_from_file(&path);
    }

    // Mockingboard sound card in slot 5
    if args.mockingboard {
        cpu.bus.iou.mockingboard = crate::device::mockingboard::Mockingboard::with_audio(audio_producers.mockingboard1, sample_rate);
        cpu.bus.iou.set_mockingboard_enabled(true);
        
        // timer-based activation, wait for system to fully initialize
        cpu.bus.iou.mockingboard.set_hook_activation(true);
        cpu.hooks.register_mockingboard_hook(1, 4_000_000);  // Slot 5
        
        println!("slot5 {:>12} {:>8}", "MOCKINGBRD", "ONLINE");
    }

    // Second Mockingboard in slot 4 (disables memory expansion)
    if args.mockingboard2 {
        cpu.bus.iou.mockingboard2 = crate::device::mockingboard::Mockingboard::with_audio(audio_producers.mockingboard2, sample_rate);
        cpu.bus.iou.set_mockingboard2_enabled(true);
        
        // timer-based activation
        cpu.bus.iou.mockingboard2.set_hook_activation(true);
        cpu.hooks.register_mockingboard_hook(0, 3_000_000);  // Slot 4
        
        println!("slot4 {:>12} {:>8}    memexp disabled", "MOCKINGBRD", "ONLINE");
    }

    // Register ProDOS MLI hooks
    hooks::register_hooks(&mut cpu.hooks);

    if args.paddle {
        cpu.bus.iou.paddle.enable_gamepad();
        println!("input {:>12} {:>8}", "PADDLE", "ONLINE");
    }

    // Load ROM
    let iic_rom_file = include_bytes!("../assets/iic3.bin");
    let iic_rom = rom::ROM::load_from_bytes(iic_rom_file, cpu.system_type).unwrap();
    cpu.load_rom(iic_rom);
    cpu.init();

    // Load disks
    if let Some(path) = &args.disk {
        cpu.bus.iou.iwm.load_disk(path.clone()).unwrap();
        println!("disk  {:>12} {:>8}    {}", "5.25_D1", "LOADED", path);
    }

    if let Some(path) = &args.disk2 {
        cpu.bus.iou.iwm.load_disk2(path).unwrap();
        println!("disk  {:>12} {:>8}    {}", "5.25_D2", "LOADED", path);
    }

    // Load 3.5" disk images (ProDOS order / 2IMG)
    if let Some(path) = &args.disk35 {
        match cpu.bus.iou.iwm.load_disk35(path) {
            Ok(()) => {
                println!("disk  {:>12} {:>8}    {}", "3.5_D1", "LOADED", path);
            }
            Err(e) => {
                eprintln!("disk  {:>12} {:>8}    {}: {}", "3.5_D1", "ERROR", path, e);
            }
        }
    }

    if let Some(path) = &args.disk35_2 {
        match cpu.bus.iou.iwm.load_disk35_drive(1, path) {
            Ok(()) => {
                println!("disk  {:>12} {:>8}    {}", "3.5_D2", "LOADED", path);
            }
            Err(e) => {
                eprintln!("disk  {:>12} {:>8}    {}: {}", "3.5_D2", "ERROR", path, e);
            }
        }
    }

    // Load hard drive images (HDV) into SmartPort device chain
    if let Some(path) = &args.hdv {
        match cpu.bus.iou.iwm.smartport.load_hdv(path) {
            Ok(()) => {
                let dev = &cpu.bus.iou.iwm.smartport.hdv_devices[0];
                println!("disk  {:>12} {:>8}    {} ({} blocks)", "HDV_1", "LOADED", path, dev.block_count);
            }
            Err(e) => {
                eprintln!("disk  {:>12} {:>8}    {}: {}", "HDV_1", "ERROR", path, e);
            }
        }
    }

    if let Some(path) = &args.hdv2 {
        match cpu.bus.iou.iwm.smartport.load_hdv(path) {
            Ok(()) => {
                let dev = &cpu.bus.iou.iwm.smartport.hdv_devices[1];
                println!("disk  {:>12} {:>8}    {} ({} blocks)", "HDV_2", "LOADED", path, dev.block_count);
            }
            Err(e) => {
                eprintln!("disk  {:>12} {:>8}    {}: {}", "HDV_2", "ERROR", path, e);
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
    run_gui(cpu, &args, config, audio_controls)
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

fn run_gui(
    mut cpu: CPU,
    args: &Args,
    config: config::Config,
    audio_controls: Option<audio_mixer::AudioControls>,
) -> Result<(), Error> {
    let mut event_loop = EventLoop::new().unwrap();

    let start_fullscreen = args.fullscreen || config.display.fullscreen;
    let mouse_enabled = args.mouse || config.machine.mouse;

    let shader = if args.shader == ShaderType::Crt {
        config.display.shader_type
    } else {
        args.shader
    };

    cpu.bus.video.force_neutral_mono = shader == ShaderType::Lcd;
    let mut app = App::new_with_config(cpu, shader, start_fullscreen, mouse_enabled, config);

    if args.no_chroma_blur { app.shader_params.chroma_blur = false; }
    if args.no_comb_filter { app.shader_params.comb_filter = false; }
    if args.no_phosphor_spread { app.shader_params.phosphor_spread = false; }
    app.shader_params.ntsc_strength = args.ntsc_strength.clamp(0.0, 1.0);

    app.cpu.bus.iou.iwm.drive_audio.params = app.drive_audio_params.clone();
    app.cpu.bus.iou.iwm.drive_audio.apply_params();

    app.audio_controls = audio_controls;
    app.apply_audio_config();

    let timeout = Some(Duration::ZERO);
    let target_frame_time = Duration::from_micros(timing::FRAME_DURATION_MICROS);

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
        (args.fast_speed as f64 * timing::CYCLES_PER_FRAME as f64) as u64
    } else {
        (args.speed as f64 * timing::CYCLES_PER_FRAME as f64) as u64
    };

    let mut next_frame_time = Instant::now();

    let mut perf_start = Instant::now();
    let mut perf_frames = 0u64;
    let mut monitor_frame_ctr: u32 = 0;

    let mut cpu_time_ema_us: f64 = 0.0;

    let mut diag_pump_us: u64 = 0;
    let mut diag_audio_us: u64 = 0;
    let mut diag_monitor_redraws: u64 = 0;
    let mut diag_heavy_frames: u64 = 0;
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
        
        let effective_cpf: u64 = if iwm_fast {
            cycles_per_frame * 8
        } else {
            cycles_per_frame * zip_multiplier
        };

        // CPU monitor (F12)
        let monitor_paused = app.cpu_monitor.is_paused();
        let pending_steps = app.cpu_monitor.take_pending_steps();
        let run_full_frame = app.window.is_some() && !app.paused && !monitor_paused;

        if run_full_frame {
            let cycles_per_scanline = effective_cpf / timing::SCANLINES_PER_FRAME;
            let remainder = effective_cpf % timing::SCANLINES_PER_FRAME;
            let mut hit_breakpoint: Option<u16> = None;

            if !app.frame_progress.active {
                app.frame_progress = crate::app::FrameProgress {
                    active: true,
                    scanline: 0,
                    cycles_run: 0,
                    target_cycles: 0,
                };
                app.cpu.video_begin_frame();
            }


            let mut scanline = app.frame_progress.scanline;
            let mut cycles_run = app.frame_progress.cycles_run;
            let mut target_cycles = app.frame_progress.target_cycles;

            while scanline < timing::SCANLINES_PER_FRAME as usize {
                // overshoot from one scanline naturally reduces the next
                target_cycles += cycles_per_scanline + if (scanline as u64) < remainder { 1 } else { 0 };

                const TRACE_SAMPLE_STRIDE: u32 = 32;
                let monitor_capturing = app.cpu.capture_trace && app.cpu_monitor.enabled;
                let mut trace_ctr: u32 = 0;

                while cycles_run < target_cycles {
                    if fast_mode && app.cpu.pc == fast_until_addr {
                        println!(
                            "Reached fast_until address {:04X}. Switching to normal speed.",
                            fast_until_addr
                        );
                        fast_mode = false;
                        cycles_per_frame = timing::CYCLES_PER_FRAME;
                        app.cpu.debug = true;
                    }

                    if !fast_mode && args.log_until.is_some() && app.cpu.pc == log_until_addr {
                        println!("Reached log_until address {:04X}. Exiting.", log_until_addr);
                        std::process::exit(0);
                    }


                    if app.cpu_monitor.enabled
                        && app.cpu_monitor.breakpoints.contains(app.cpu.pc)
                    {
                        if app.cpu_monitor.skip_next_breakpoint {
                            app.cpu_monitor.skip_next_breakpoint = false;
                        } else {
                            hit_breakpoint = Some(app.cpu.pc);
                            break;
                        }
                    }

                    if monitor_capturing {
                        let sample = trace_ctr == 0;
                        app.cpu.capture_trace = sample;
                        trace_ctr = (trace_ctr + 1) % TRACE_SAMPLE_STRIDE;
                        cycles_run += app.cpu.tick();
                        if sample {
                            app.cpu_monitor.record(app.cpu.last_trace);
                        }
                    } else {
                        cycles_run += app.cpu.tick();
                    }
                }

                if hit_breakpoint.is_some() {
                    break;
                }

                if monitor_capturing {
                    app.cpu.capture_trace = true;
                }

                if scanline < 192 {
                    app.cpu.video_snapshot_scanline(scanline);
                }
                scanline += 1;
            }

            let frame_complete = scanline >= timing::SCANLINES_PER_FRAME as usize;
            app.frame_progress = crate::app::FrameProgress {
                active: !frame_complete,
                scanline,
                cycles_run,
                target_cycles,
            };

            cpu_time = frame_start.elapsed();

            if frame_complete {
                let audio_start = Instant::now();
                app.cpu.bus.iou.speaker.update(app.cpu.bus.iou.cycles);
                app.cpu.bus.iou.mockingboard.update(app.cpu.bus.iou.cycles);
                app.cpu.bus.iou.mockingboard2.update(app.cpu.bus.iou.cycles);
                app.cpu.bus.iou.iwm.update_audio();
                diag_audio_us += audio_start.elapsed().as_micros() as u64;

                app.cpu.bus.iou.paddle.poll();

                // Battery-backed RAM expansion: opportunistic flush
                app.maybe_flush_memexp();
            }

            if let Some(addr) = hit_breakpoint {
                println!("Hit breakpoint at ${:04X} — pausing CPU monitor.", addr);
                app.cpu_monitor.paused = true;

                app.cpu_monitor.fresh_step_sample = true;
            }
        } else if monitor_paused && pending_steps > 0 && app.window.is_some() {
            let capture = app.cpu.capture_trace;
            let mut stepped_cycles: u64 = 0;
            for _ in 0..pending_steps {
                stepped_cycles += app.cpu.tick();
                if capture {
                    app.cpu_monitor.record(app.cpu.last_trace);
                }
            }
            if app.frame_progress.active {
                app.frame_progress.cycles_run = app
                    .frame_progress
                    .cycles_run
                    .saturating_add(stepped_cycles);
            }
            app.cpu.bus.iou.iwm.update_audio();
        }

        let pump_start = Instant::now();
        let status = event_loop.pump_app_events(timeout, &mut app);
        diag_pump_us += pump_start.elapsed().as_micros() as u64;

        if let PumpStatus::Exit(exit_code) = status {
            app.flush_disks();
            std::process::exit(exit_code as i32);
        }

        // snap window to aspect ratio after user finishes resizing
        app.snap_aspect_ratio();

        if let Some(window) = &app.window {
            window.request_redraw();
        }

        if let Some(mw) = &app.monitor_window {
            let cpu_us = cpu_time.as_micros() as f64;
            cpu_time_ema_us = cpu_time_ema_us * 0.875 + cpu_us * 0.125;
            const HEAVY_FRAME_US: f64 = 12_000.0;
            monitor_frame_ctr = monitor_frame_ctr.wrapping_add(1);
            let heavy = cpu_time_ema_us > HEAVY_FRAME_US;
            if heavy {
                diag_heavy_frames += 1;
            }

            let stride = if monitor_paused || pending_steps > 0 {
                1
            } else if heavy {
                12
            } else {
                6
            };
            if monitor_frame_ctr % stride == 0 {
                mw.request_redraw();
                diag_monitor_redraws += 1;
            }
        }

        // performance metrics
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
                println!(
                    "      pump:{:>5.1}ms/f  audio:{:>5.1}ms/f  mon_redraws:{:>3}/s  heavy:{:>3}/s  cpu_ema:{:>5.1}ms  mon_open:{}",
                    diag_pump_us as f64 / perf_frames as f64 / 1000.0,
                    diag_audio_us as f64 / perf_frames as f64 / 1000.0,
                    diag_monitor_redraws,
                    diag_heavy_frames,
                    cpu_time_ema_us / 1000.0,
                    app.monitor_window.is_some(),
                );
                let (hits, misses) = app.cpu_monitor.take_disasm_cache_stats();
                println!(
                    "      disasm_cache: hits:{} misses:{}  trace_buf:{}",
                    hits,
                    misses,
                    app.cpu_monitor.trace_buffer.len(),
                );
            } else {
                app.cpu.bus.iou.iwm.get_and_reset_metrics();
            }

            diag_pump_us = 0;
            diag_audio_us = 0;
            diag_monitor_redraws = 0;
            diag_heavy_frames = 0;
            perf_start = Instant::now();
            perf_frames = 0;
            perf_cycles_start = app.cpu.cycles;
        }

        // frame pacing
        next_frame_time += target_frame_time;
        let now = Instant::now();
        if now < next_frame_time {
            std::thread::sleep(next_frame_time - now);
        } else if now - next_frame_time > Duration::from_millis(50) {
            next_frame_time = now;
        }
    }
}
