use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use shader_ui::ShaderParams;

use crate::cli::ShaderType;
use crate::device::drive_audio::DriveAudioParams;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub machine: MachineConfig,
    pub boot: BootConfig,
    pub display: DisplayConfig,
    pub audio: AudioConfig,
    pub shader: ShaderParams,
    pub drive_audio: DriveAudioParams,
    pub serial: SerialConfig,
    pub debug: DebugConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineConfig {
    pub mockingboard: bool,        // slot 5
    pub mockingboard2: bool,       // slot 4 (replaces memexp)
    pub zip_chip: bool,
    pub mouse: bool,
    pub paddle: bool,
    pub fast_disk: bool,
    pub speed: f32,                // CPU speed multiplier
    pub fast_speed: f32,           // multiplier in fast mode
}

impl Default for MachineConfig {
    fn default() -> Self {
        Self {
            mockingboard: false,
            mockingboard2: false,
            zip_chip: false,
            mouse: false,
            paddle: false,
            fast_disk: false,
            speed: 1.0,
            fast_speed: 10.0,
        }
    }
}

// Disks to auto-mount at boot. Empty string = unmounted.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BootConfig {
    pub disk1: String,    // S6 D1 5.25"
    pub disk2: String,    // S6 D2 5.25"
    pub disk35_1: String, // SmartPort 3.5" #1
    pub disk35_2: String, // SmartPort 3.5" #2
    pub hdv1: String,     // SmartPort HDV #1
    pub hdv2: String,     // SmartPort HDV #2
    pub self_test: bool,
}

// Display defaults: fullscreen, monochrome, scanline strength,
// shader selection. Detailed shader knobs live under `[shader]`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub fullscreen: bool,
    pub monochrome: bool,
    pub shader_type: ShaderType,
    pub scanline_intensity: f32,
    pub mono_fg: [u8; 3],
    pub mono_bg: [u8; 3],
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            fullscreen: false,
            monochrome: false,
            shader_type: ShaderType::Crt,
            scanline_intensity: 0.5,
            mono_fg: [118, 255, 211],
            mono_bg: [15, 23, 23],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub muted: bool,
    pub master: f32,
    pub speaker: f32,
    pub mockingboard1: f32,
    pub mockingboard2: f32,
    pub drive: f32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            muted: false,
            master: 1.0,
            speaker: 1.0,
            mockingboard1: 1.0,
            mockingboard2: 1.0,
            drive: 1.0,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SerialConfig {
    pub host: String,         // e.g. "bbs.example.com:23"; empty = none
    pub modem: bool,          // virtual Hayes modem on SCC Ch A
    pub loopback: bool,
}#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DebugConfig {
    pub debug: bool,
    pub perf: bool,
}

// `$HOME/.config/rust-iic/config.toml`
// Falls back to `./rust-iic.toml` if `$HOME` cannot be resolved.
pub fn config_path() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let mut p = PathBuf::from(home);
        p.push(".config/rust-iic/config.toml");
        return p;
    }
    if let Some(up) = std::env::var_os("USERPROFILE") {
        // Windows fallback (MSYS sets HOME so this only fires on cmd.exe)
        let mut p = PathBuf::from(up);
        p.push(".config/rust-iic/config.toml");
        return p;
    }
    PathBuf::from("rust-iic.toml")
}

impl Config {
    pub fn load() -> Self {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(cfg) => cfg,
                Err(e) => {
                    log::warn!("config: failed to parse {}: {}", path.display(), e);
                    Config::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => {
                log::warn!("config: failed to read {}: {}", path.display(), e);
                Config::default()
            }
        }
    }

    pub fn save(&self) -> std::io::Result<PathBuf> {
        let path = config_path();
        self.save_to(&path)?;
        Ok(path)
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })?;
        std::fs::write(path, text)
    }
}
