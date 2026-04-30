use crate::cpu_monitor::state::{MemoryWatch, MAX_WATCHES};

pub struct WatchesPanelState {
    pub watches: Vec<MemoryWatch>,
    pub new_addr: String,
    pub new_label: String,
}

impl Default for WatchesPanelState {
    fn default() -> Self {
        Self {
            watches: Vec::with_capacity(MAX_WATCHES),
            new_addr: String::new(),
            new_label: String::new(),
        }
    }
}

impl WatchesPanelState {
    pub fn add(&mut self, address: u16, size: u8, label: &str) {
        if self.watches.len() < MAX_WATCHES {
            self.watches.push(MemoryWatch::new(address, size, label));
        }
    }

    pub fn remove(&mut self, idx: usize) {
        if idx < self.watches.len() {
            self.watches.remove(idx);
        }
    }
}

pub fn render(
    ui: &mut egui::Ui,
    state: &mut WatchesPanelState,
    memory_reader: &dyn Fn(u16) -> u8,
) {
    ui.label(egui::RichText::new("WATCHES").small().strong());

    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.new_addr)
                .desired_width(56.0)
                .hint_text("addr"),
        );
        ui.add(
            egui::TextEdit::singleline(&mut state.new_label)
                .desired_width(80.0)
                .hint_text("label"),
        );
        if ui.small_button("+").clicked() {
            if let Ok(addr) = u16::from_str_radix(state.new_addr.trim(), 16) {
                let label = if state.new_label.is_empty() {
                    format!("${:04X}", addr)
                } else {
                    state.new_label.clone()
                };
                state.add(addr, 1, &label);
                state.new_addr.clear();
                state.new_label.clear();
            }
        }
    });

    egui::ScrollArea::vertical()
        .id_salt("cpu_watches")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if state.watches.is_empty() {
                ui.weak("(none)");
                return;
            }
            let mut to_remove = None;
            for (idx, watch) in state.watches.iter().enumerate() {
                ui.horizontal(|ui| {
                    let value = memory_reader(watch.address);
                    ui.monospace(format!(
                        "{:<10} ${:04X}={:02X}",
                        watch.label_str(),
                        watch.address,
                        value,
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("\u{2716}").clicked() {
                            to_remove = Some(idx);
                        }
                    });
                });
            }
            if let Some(idx) = to_remove {
                state.remove(idx);
            }
        });
}
