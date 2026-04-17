//! Command-line argument parsing for the Apple IIc emulator.

use clap::{Parser, ValueEnum};

/// Display shader type for post-processing effects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ShaderType {
    /// No post-processing shader (raw pixels)
    #[default]
    None,
    /// CRT monitor effect (scanlines, curvature, bloom)
    Crt,
    /// Apple IIc LCD flat panel effect
    Lcd,
}

/// Apple //c Emulator command-line arguments.
#[derive(Parser)]
#[command(version, about = "Apple //c Emulator")]
pub struct Args {
    /// Run without video output (headless mode)
    #[arg(long)]
    pub no_video: bool,

    /// Start in interactive monitor/debugger mode
    #[arg(long)]
    pub monitor: bool,

    /// ROM type selection (auto, 3, 4, etc.)
    #[arg(long, default_value = "auto")]
    pub rom_type: String,

    /// Enable debug logging
    #[arg(long, short)]
    pub debug: bool,

    /// CPU speed multiplier (1.0 = 1.023 MHz)
    #[arg(long, default_value_t = 1.0)]
    pub speed: f32,

    /// Enable monochrome (green phosphor) display
    #[arg(long)]
    pub monochrome: bool,

    /// Scanline intensity for CRT shader (0.0 - 1.0)
    #[arg(long, default_value_t = 0.5)]
    pub scanline_intensity: f32,

    /// Show performance metrics
    #[arg(long)]
    pub perf: bool,

    /// Boot into self-test mode (hold Open+Closed Apple)
    #[arg(long)]
    pub self_test: bool,

    /// Run at fast speed until PC reaches this address (hex)
    #[arg(long)]
    pub fast_until: Option<String>,

    /// Enable logging until PC reaches this address, then exit (hex)
    #[arg(long)]
    pub log_until: Option<String>,

    /// Speed multiplier for fast mode
    #[arg(long, default_value_t = 10.0)]
    pub fast_speed: f32,

    /// Path to disk image for drive 1
    #[arg(index = 1)]
    pub disk: Option<String>,

    /// Path to disk image for drive 2
    #[arg(long)]
    pub disk2: Option<String>,

    /// Enable fast disk mode (skip rotational latency)
    #[arg(long)]
    pub fast_disk: bool,

    /// Display shader: none, crt, lcd
    #[arg(long, value_enum, default_value_t = ShaderType::None)]
    pub shader: ShaderType,

    /// Connect modem port (SCC Ch A) to a TCP host, e.g. --serial bbs.example.com:23
    #[arg(long)]
    pub serial: Option<String>,

    /// Enable virtual Hayes modem on slot 1 (use ATDT from terminal software to connect)
    #[arg(long)]
    pub modem: bool,

    /// Enable serial loopback mode (for diagnostic testing with loopback cable)
    #[arg(long, conflicts_with = "modem")]
    pub serial_loopback: bool,

    /// Enable ZIP CHIP II-8 accelerator (8MHz, toggle with Ctrl+Z)
    #[arg(long)]
    pub zip: bool,

    /// Enable Mockingboard sound card in slot 5
    #[arg(long)]
    pub mockingboard: bool,

    /// Enable second Mockingboard in slot 4 (disables memory expansion, for Ultima V etc.)
    #[arg(long)]
    pub mockingboard2: bool,
}
