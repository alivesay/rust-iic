use crate::cpu::Flags;
use crate::cpu_monitor::state::{CpuState, IouSnapshot};
use crate::cpu_monitor::widgets::{flag_chip, format_cycles};

const HEADER_FONT_SIZE: f32 = 10.0;
const VALUE_FONT_SIZE: f32 = 14.0;

pub fn render(ui: &mut egui::Ui, state: &CpuState, iou: &IouSnapshot) {
    let header_color = ui.visuals().weak_text_color();
    let header_font = egui::FontId::proportional(HEADER_FONT_SIZE);
    let value_font = egui::FontId::monospace(VALUE_FONT_SIZE);

    const COLS: &[(&str, f32)] = &[
        ("PC", 56.0),
        ("A", 36.0),
        ("X", 36.0),
        ("Y", 36.0),
        ("SP", 36.0),
    ];

    let values = [
        format!("{:04X}", state.pc),
        format!("{:02X}", state.a),
        format!("{:02X}", state.x),
        format!("{:02X}", state.y),
        format!("{:02X}", state.sp),
    ];

    let flags = Flags::from_bits_truncate(state.p);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;

        // Register columns: header above value.
        for ((label, w), val) in COLS.iter().zip(values.iter()) {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add_sized(
                    [*w, 12.0],
                    egui::Label::new(
                        egui::RichText::new(*label)
                            .font(header_font.clone())
                            .color(header_color),
                    ),
                );
                ui.add_sized(
                    [*w, 18.0],
                    egui::Label::new(
                        egui::RichText::new(val).font(value_font.clone()).strong(),
                    ),
                );
            });
        }

        ui.add_space(6.0);

        // FLAGS group.
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            section_label(ui, "FLAGS", &header_font, header_color);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                flag_chip(ui, "N", flags.contains(Flags::NEGATIVE));
                flag_chip(ui, "V", flags.contains(Flags::OVERFLOW));
                flag_chip(ui, "-", false);
                flag_chip(ui, "B", flags.contains(Flags::BREAK));
                flag_chip(ui, "D", flags.contains(Flags::DECIMAL));
                flag_chip(ui, "I", flags.contains(Flags::IRQ_DISABLE));
                flag_chip(ui, "Z", flags.contains(Flags::ZERO));
                flag_chip(ui, "C", flags.contains(Flags::CARRY));
            });
        });

        ui.add_space(12.0);

        // INTERRUPTS group.
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            section_label(ui, "INTERRUPTS", &header_font, header_color);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                flag_chip(ui, "IRQ",  iou.irq_pending);
                flag_chip(ui, "NMI",  iou.nmi_pending);
                flag_chip(ui, "MX",   iou.mouse_x_int);
                flag_chip(ui, "MY",   iou.mouse_y_int);
                flag_chip(ui, "MVBL", iou.mouse_vbl_int);
            });
        });

        ui.add_space(12.0);

        // MASK group.
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            section_label(ui, "MASK", &header_font, header_color);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                flag_chip(ui, "XY",  iou.mouse_xy_mask);
                flag_chip(ui, "VBL", iou.mouse_vbl_mask);
            });
        });

        // Cycle counter
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(format_cycles(state.cycles))
                        .font(value_font.clone())
                        .strong(),
                );
            },
        );
    });
}

fn section_label(ui: &mut egui::Ui, text: &str, font: &egui::FontId, color: egui::Color32) {
    ui.label(
        egui::RichText::new(text)
            .font(font.clone())
            .color(color),
    );
}
