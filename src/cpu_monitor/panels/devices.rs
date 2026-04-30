use egui::{Color32, Pos2, Sense, Stroke, StrokeKind, Vec2};

use crate::cpu_monitor::state::DevicesSnapshot;

const LED_DIM: Color32 = Color32::from_rgb(0x22, 0x22, 0x22); // No disk in drive.
const LED_OFF: Color32 = Color32::from_rgb(0x55, 0x55, 0x55); // Disk loaded, idle.
const LED_ON: Color32 = Color32::from_rgb(0x4A, 0xC8, 0x60);  // Drive active.

pub fn render(ui: &mut egui::Ui, devs: &DevicesSnapshot) {
    drive_row(ui, devs);
    iwm_state_row(ui, devs);
    ui.add_space(8.0);

    scope(ui, "Speaker", &devs.speaker_scope, false, true);

    if devs.mockingboard1_enabled {
        ui.add_space(6.0);
        scope(ui, "Mockingboard 1", &devs.mockingboard1_scope, true, true);
    }
    if devs.mockingboard2_enabled {
        ui.add_space(6.0);
        scope(ui, "Mockingboard 2", &devs.mockingboard2_scope, true, true);
    }
}

fn drive_row(ui: &mut egui::Ui, devs: &DevicesSnapshot) {
    let dim = ui.visuals().weak_text_color();

    let render_cell = |ui: &mut egui::Ui, idx: usize, label: &str| {
        let active = devs.drive_active[idx];
        let present = devs.drive_present[idx];
        led(ui, active, present);
        ui.label(label);

        let track_text = if idx < 2 {
            if present {
                let qt = devs.drive_head_qt[idx];
                let track = qt / 4;
                let phase = qt % 4;
                if phase == 0 {
                    format!("T{:>2}", track)
                } else {
                    format!("T{:>2}.{}", track, phase)
                }
            } else {
                "—".to_string()
            }
        } else if present {
            format!("side {}", devs.iwm_head35)
        } else {
            "—".to_string()
        };
        ui.label(
            egui::RichText::new(track_text)
                .monospace().small()
                .color(if active { ui.visuals().text_color() } else { dim }),
        );

        let wp_text = if present && devs.drive_write_protect[idx] {
            "WP"
        } else {
            "  "
        };
        ui.label(
            egui::RichText::new(wp_text)
                .monospace().small().color(dim),
        );
    };

    egui::Grid::new("devices_drive_grid")
        .num_columns(2)
        .spacing([18.0, 2.0])
        .show(ui, |ui| {
            ui.horizontal(|ui| render_cell(ui, 0, "5.25\" 1"));
            ui.horizontal(|ui| render_cell(ui, 1, "5.25\" 2"));
            ui.end_row();
            ui.horizontal(|ui| render_cell(ui, 2, "3.5\" 1 "));
            ui.horizontal(|ui| render_cell(ui, 3, "3.5\" 2 "));
            ui.end_row();
        });
}

fn iwm_state_row(ui: &mut egui::Ui, devs: &DevicesSnapshot) {
    let dim = ui.visuals().weak_text_color();
    let on_col = egui::Color32::from_rgb(0xE5, 0xC0, 0x70);
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("IWM").monospace().small().color(dim));

        ui.label(egui::RichText::new("PH").monospace().small().color(dim));
        for bit in 0..4 {
            let on = devs.iwm_phases & (1 << bit) != 0;
            ui.label(
                egui::RichText::new(if on { "1" } else { "·" })
                    .monospace()
                    .small()
                    .color(if on { on_col } else { dim }),
            );
        }

        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(format!("DRV{}", devs.iwm_drive_select + 1))
                .monospace().small()
                .color(if devs.iwm_motor_on { on_col } else { dim }),
        );

        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(if devs.iwm_motor_on { "MOT5" } else { "    " })
                .monospace().small().color(on_col),
        );
        ui.label(
            egui::RichText::new(if devs.iwm_motor_on35 { "MOT35" } else { "     " })
                .monospace().small().color(on_col),
        );
        ui.label(
            egui::RichText::new(if devs.iwm_write_mode { "WRT" } else { "   " })
                .monospace()
                .small()
                .color(egui::Color32::from_rgb(0xE5, 0x90, 0x60)),
        );
    });
}

fn led(ui: &mut egui::Ui, active: bool, present: bool) {
    let size = Vec2::splat(10.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let color = if active {
        LED_ON
    } else if present {
        LED_OFF
    } else {
        LED_DIM
    };
    ui.painter().circle_filled(rect.center(), 4.5, color);
    ui.painter().circle_stroke(
        rect.center(),
        4.5,
        Stroke::new(1.0, Color32::from_rgb(0x10, 0x10, 0x10)),
    );
}

fn scope(ui: &mut egui::Ui, label: &str, samples: &[f32], stereo: bool, _show_label: bool) {
    ui.label(egui::RichText::new(label).small().weak());
    let avail_w = ui.available_width().max(64.0);
    let height = 72.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(avail_w, height), Sense::hover());

    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 2.0, Color32::from_rgb(0x12, 0x12, 0x18));
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, Color32::from_rgb(0x33, 0x33, 0x40)),
        StrokeKind::Outside,
    );

    let mid_y = rect.center().y;
    painter.line_segment(
        [Pos2::new(rect.left(), mid_y), Pos2::new(rect.right(), mid_y)],
        Stroke::new(1.0, Color32::from_rgb(0x2A, 0x2A, 0x33)),
    );

    if samples.is_empty() {
        return;
    }

    let frames: Vec<f32> = if stereo {
        samples
            .chunks_exact(2)
            .map(|c| 0.5 * (c[0] + c[1]))
            .collect()
    } else {
        samples.to_vec()
    };
    if frames.is_empty() {
        return;
    }

    let mean: f32 = frames.iter().copied().sum::<f32>() / frames.len() as f32;
    let mut peak = 0.0f32;
    for &v in &frames {
        let a = (v - mean).abs();
        if a > peak {
            peak = a;
        }
    }

    let gain = if peak > 1e-4 { 0.9 / peak } else { 0.0 };

    let n_cols = rect.width().floor().max(1.0) as usize;
    let stride = (frames.len() / n_cols).max(1);
    let display = n_cols.min(frames.len() / stride);
    let half_h = rect.height() * 0.5 - 2.0;
    let trace_color = Color32::from_rgb(0xA0, 0xE0, 0x70);

    let start = frames.len().saturating_sub(display * stride);
    let mut points = Vec::with_capacity(display);
    for i in 0..display {
        let idx = (start + i * stride + (stride - 1)).min(frames.len() - 1);
        let v = ((frames[idx] - mean) * gain).clamp(-1.0, 1.0);
        let x = rect.left() + i as f32;
        let y = mid_y - v * half_h;
        points.push(Pos2::new(x, y));
    }
    if points.len() >= 2 {
        painter.add(egui::epaint::Shape::line(
            points,
            Stroke::new(1.2, trace_color),
        ));
    }
}
