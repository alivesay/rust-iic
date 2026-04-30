use std::collections::BTreeSet;

use crate::disassembler::lookup_system_symbol;

#[derive(Default, Debug, Clone)]
pub struct BreakpointSet {
    addrs: BTreeSet<u16>,
    pub add_input: String,
}

impl BreakpointSet {
    #[inline]
    pub fn contains(&self, addr: u16) -> bool {
        self.addrs.contains(&addr)
    }

    pub fn toggle(&mut self, addr: u16) -> bool {
        if self.addrs.remove(&addr) {
            false
        } else {
            self.addrs.insert(addr);
            true
        }
    }

    pub fn insert(&mut self, addr: u16) {
        self.addrs.insert(addr);
    }

    pub fn remove(&mut self, addr: u16) {
        self.addrs.remove(&addr);
    }

    pub fn clear(&mut self) {
        self.addrs.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = u16> + '_ {
        self.addrs.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.addrs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.addrs.len()
    }
}

pub fn render(ui: &mut egui::Ui, bps: &mut BreakpointSet) {
    let w = ui.available_width();
    ui.set_max_width(w);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Add $").small());
        let resp = ui.add(
            egui::TextEdit::singleline(&mut bps.add_input)
                .desired_width(60.0)
                .hint_text("hex"),
        );
        let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if submit || ui.small_button("+").clicked() {
            if let Ok(addr) = u16::from_str_radix(bps.add_input.trim(), 16) {
                bps.insert(addr);
            }
            bps.add_input.clear();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !bps.is_empty() && ui.small_button("Clear").clicked() {
                bps.clear();
            }
        });
    });

    if bps.is_empty() {
        ui.weak("(no breakpoints)");
        return;
    }

    let dim = ui.visuals().weak_text_color();
    let hot = egui::Color32::from_rgb(0xC8, 0x55, 0x40);

    let entries: Vec<u16> = bps.iter().collect();
    egui::Grid::new("cpu_bp_list")
        .num_columns(3)
        .spacing([6.0, 1.0])
        .min_col_width(0.0)
        .show(ui, |ui| {
            for addr in entries {
                ui.label(
                    egui::RichText::new("*")
                        .monospace()
                        .small()
                        .color(hot),
                );
                ui.label(
                    egui::RichText::new(format!("${:04X}", addr))
                        .monospace()
                        .small(),
                );
                let sym = lookup_system_symbol(addr).unwrap_or("");
                ui.horizontal(|ui| {
                    if !sym.is_empty() {
                        ui.label(egui::RichText::new(sym).small().color(dim));
                    }
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui.small_button("x").on_hover_text("Remove").clicked() {
                                bps.remove(addr);
                            }
                        },
                    );
                });
                ui.end_row();
            }
        });
}
