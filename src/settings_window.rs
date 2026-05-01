use crate::cli::ShaderType;
use crate::config::Config;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Video,
    Audio,
    Disk,
    Machine,
    Serial,
    Debug,
}

impl SettingsTab {
    fn label(self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Disk => "Disk",
            Self::Machine => "Machine",
            Self::Serial => "Serial",
            Self::Debug => "Debug",
        }
    }
}

#[derive(Default)]
pub struct SettingsResult {
    pub changed: bool,
    pub save_requested: bool,
    pub reload_requested: bool,
    pub open_shader_panel: bool,
    pub open_drive_audio_panel: bool,
    #[allow(dead_code)]
    pub status_message: Option<String>,
}

pub struct SettingsState {
    pub active_tab: SettingsTab,
    pub status: Option<String>,
    pub just_opened: bool,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            active_tab: SettingsTab::default(),
            status: None,
            just_opened: false,
        }
    }
}

pub fn render_settings_window(
    ctx: &egui::Context,
    config: &mut Config,
    state: &mut SettingsState,
    open: &mut bool,
) -> SettingsResult {
    let mut result = SettingsResult::default();

    let win_size = egui::vec2(460.0, 360.0);

    state.just_opened = false;

    egui::Window::new("Settings")
        .id(egui::Id::new("settings_window"))
        .open(open)
        .resizable(false)
        .collapsible(false)
        .movable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(win_size)
        .show(ctx, |ui| {
            // --- tab strip -----------------------------------------------------
            ui.horizontal(|ui| {
                for tab in [
                    SettingsTab::Video,
                    SettingsTab::Audio,
                    SettingsTab::Disk,
                    SettingsTab::Machine,
                    SettingsTab::Serial,
                    SettingsTab::Debug,
                ] {
                    if ui
                        .selectable_label(state.active_tab == tab, tab.label())
                        .clicked()
                    {
                        state.active_tab = tab;
                    }
                }
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .max_height(240.0)
                .auto_shrink([false, true])
                .show(ui, |ui| match state.active_tab {
                    SettingsTab::Video => render_video_tab(ui, config, &mut result),
                    SettingsTab::Audio => render_audio_tab(ui, config, &mut result),
                    SettingsTab::Disk => render_disk_tab(ui, config, &mut result),
                    SettingsTab::Machine => render_machine_tab(ui, config, &mut result),
                    SettingsTab::Serial => render_serial_tab(ui, config, &mut result),
                    SettingsTab::Debug => render_debug_tab(ui, config, &mut result),
                });

            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    result.save_requested = true;
                }
                if ui.button("Reload from disk").clicked() {
                    result.reload_requested = true;
                }
                if let Some(msg) = state.status.as_deref() {
                    ui.label(msg);
                }
            });
            ui.label(format!("Config file: {}", crate::config::config_path().display()));
        });

    result
}

fn render_video_tab(ui: &mut egui::Ui, cfg: &mut Config, result: &mut SettingsResult) {
    let d = &mut cfg.display;
    ui.heading("Display");
    result.changed |= ui.checkbox(&mut d.fullscreen, "Start fullscreen").changed();
    result.changed |= ui.checkbox(&mut d.monochrome, "Monochrome (green phosphor)").changed();

    ui.horizontal(|ui| {
        ui.label("Shader:");
        for (label, ty) in [
            ("None", ShaderType::None),
            ("CRT", ShaderType::Crt),
            ("LCD", ShaderType::Lcd),
        ] {
            if ui.selectable_label(d.shader_type == ty, label).clicked() {
                d.shader_type = ty;
                result.changed = true;
            }
        }
    });
    result.changed |= ui
        .add(egui::Slider::new(&mut d.scanline_intensity, 0.0..=1.0).text("Scanline intensity"))
        .changed();

    ui.separator();
    ui.heading("Detailed CRT / NTSC parameters");
    ui.label("Open the F7 panel for fine-grained shader controls.");
    if ui.button("Open shader panel (F7)").clicked() {
        result.open_shader_panel = true;
    }
}

fn render_audio_tab(ui: &mut egui::Ui, cfg: &mut Config, result: &mut SettingsResult) {
    ui.heading("Mix levels");
    let a = &mut cfg.audio;
    result.changed |= ui.checkbox(&mut a.muted, "Mute all").changed();
    result.changed |= ui
        .add(egui::Slider::new(&mut a.master, 0.0..=2.0).text("Master"))
        .changed();
    result.changed |= ui
        .add(egui::Slider::new(&mut a.speaker, 0.0..=2.0).text("Speaker (square-wave beeps)"))
        .changed();
    result.changed |= ui
        .add(egui::Slider::new(&mut a.mockingboard1, 0.0..=2.0).text("Mockingboard #1 (slot 5)"))
        .changed();
    result.changed |= ui
        .add(egui::Slider::new(&mut a.mockingboard2, 0.0..=2.0).text("Mockingboard #2 (slot 4)"))
        .changed();
    result.changed |= ui
        .add(egui::Slider::new(&mut a.drive, 0.0..=2.0).text("Disk drives"))
        .changed();

    ui.separator();
    ui.heading("Drive audio (high-level)");
    let d = &mut cfg.drive_audio;
    result.changed |= ui.checkbox(&mut d.enabled, "Drive sound effects enabled").changed();
    result.changed |= ui
        .add(egui::Slider::new(&mut d.master_volume, 0.0..=4.0).text("Drive synth master"))
        .changed();

    ui.separator();
    ui.label("Open the F8 panel for per-component drive sound design.");
    if ui.button("Open drive audio panel (F8)").clicked() {
        result.open_drive_audio_panel = true;
    }
}

fn render_disk_tab(ui: &mut egui::Ui, cfg: &mut Config, result: &mut SettingsResult) {
    ui.label(
        egui::RichText::new("Changes here apply at next boot.")
            .italics()
            .weak(),
    );
    let b = &mut cfg.boot;

    disk_path_row(ui, "5.25\" S6 D1", &mut b.disk1, result);
    disk_path_row(ui, "5.25\" S6 D2", &mut b.disk2, result);
    disk_path_row(ui, "3.5\"  Drive 1", &mut b.disk35_1, result);
    disk_path_row(ui, "3.5\"  Drive 2", &mut b.disk35_2, result);
    disk_path_row(ui, "HDV    1", &mut b.hdv1, result);
    disk_path_row(ui, "HDV    2", &mut b.hdv2, result);

    ui.separator();
    result.changed |= ui
        .checkbox(&mut b.self_test, "Boot into self-test (Open+Closed Apple)")
        .changed();
}

fn disk_path_row(ui: &mut egui::Ui, label: &str, path: &mut String, result: &mut SettingsResult) {
    ui.horizontal(|ui| {
        ui.label(format!("{label}:"));
        result.changed |= ui
            .add(egui::TextEdit::singleline(path).desired_width(220.0).hint_text("(empty = none)"))
            .changed();
        if ui.button("Browse").clicked() {
            if let Some(picked) = rfd::FileDialog::new()
                .add_filter("Disk image", &["woz", "po", "dsk", "do", "2mg", "hdv"])
                .pick_file()
            {
                *path = picked.to_string_lossy().to_string();
                result.changed = true;
            }
        }
        if ui.button("Clear").clicked() {
            path.clear();
            result.changed = true;
        }
    });
}

fn render_machine_tab(ui: &mut egui::Ui, cfg: &mut Config, result: &mut SettingsResult) {
    ui.label(
        egui::RichText::new("Changes here apply at next boot.")
            .italics()
            .weak(),
    );
    let m = &mut cfg.machine;

    ui.heading("Slot cards");
    result.changed |= ui
        .checkbox(&mut m.mockingboard, "Slot 5: Mockingboard")
        .changed();
    let mb2_resp = ui.checkbox(
        &mut m.mockingboard2,
        "Slot 4: Mockingboard #2 (replaces 1 MB memory expansion)",
    );
    if mb2_resp.changed() {
        result.changed = true;
    }

    ui.separator();
    ui.heading("Accelerator");
    result.changed |= ui
        .checkbox(&mut m.zip_chip, "Zip Chip II-8 (8 MHz, Ctrl-Z toggles)")
        .changed();

    ui.separator();
    ui.heading("Input");
    result.changed |= ui
        .checkbox(&mut m.mouse, "Apple //c mouse emulation")
        .changed();
    result.changed |= ui
        .checkbox(&mut m.paddle, "Paddles via host gamepad")
        .changed();

    ui.separator();
    ui.heading("CPU");
    result.changed |= ui
        .add(egui::Slider::new(&mut m.speed, 0.5..=4.0).text("Speed multiplier"))
        .changed();
    result.changed |= ui
        .add(egui::Slider::new(&mut m.fast_speed, 1.0..=50.0).text("Fast-mode multiplier"))
        .changed();
    result.changed |= ui
        .checkbox(&mut m.fast_disk, "Fast disk (skip rotational latency)")
        .changed();
}

fn render_serial_tab(ui: &mut egui::Ui, cfg: &mut Config, result: &mut SettingsResult) {
    ui.label(
        egui::RichText::new("Changes here apply at next boot.")
            .italics()
            .weak(),
    );
    let s = &mut cfg.serial;

    ui.horizontal(|ui| {
        ui.label("TCP host:port:");
        result.changed |= ui
            .add(
                egui::TextEdit::singleline(&mut s.host)
                    .desired_width(240.0)
                    .hint_text("e.g. bbs.example.com:23"),
            )
            .changed();
    });
    result.changed |= ui
        .checkbox(&mut s.modem, "Virtual Hayes modem (SCC Ch A)")
        .changed();
    result.changed |= ui
        .checkbox(&mut s.loopback, "Diagnostic loopback")
        .changed();

    if s.modem && !s.host.is_empty() {
        ui.colored_label(
            egui::Color32::from_rgb(220, 160, 0),
            "Note: modem and TCP host are mutually exclusive on Ch A.",
        );
    }
}

fn render_debug_tab(ui: &mut egui::Ui, cfg: &mut Config, result: &mut SettingsResult) {
    let d = &mut cfg.debug;
    result.changed |= ui.checkbox(&mut d.debug, "Verbose debug logging").changed();
    result.changed |= ui.checkbox(&mut d.perf, "Show perf overlay").changed();
}
