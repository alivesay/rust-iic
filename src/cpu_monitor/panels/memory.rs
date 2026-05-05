#[derive(Default)]
pub struct MemoryPanelState {
    pub page: u8,
    pub goto: String,
}

pub fn render(
    ui: &mut egui::Ui,
    state: &mut MemoryPanelState,
    memory_reader: &dyn Fn(u16) -> u8,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("MEMORY").small().strong());
        ui.separator();
        ui.label("page");
        ui.add(
            egui::DragValue::new(&mut state.page)
                .hexadecimal(2, false, true)
                .range(0..=0xFF),
        );
        ui.separator();
        ui.label("goto");
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.goto)
                .desired_width(60.0)
                .hint_text("hex"),
        );
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Ok(addr) = u16::from_str_radix(state.goto.trim(), 16) {
                state.page = (addr >> 8) as u8;
            }
            state.goto.clear();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button(">").on_hover_text("Next page").clicked() {
                state.page = state.page.wrapping_add(1);
            }
            if ui.small_button("<").on_hover_text("Previous page").clicked() {
                state.page = state.page.wrapping_sub(1);
            }
        });
    });

    let page_base = (state.page as u16) << 8;
    egui::ScrollArea::vertical()
        .id_salt("cpu_memory")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for row in 0..16u16 {
                let addr = page_base.wrapping_add(row * 16);
                let mut hex = String::with_capacity(48);
                let mut ascii = String::with_capacity(16);
                for col in 0..16u16 {
                    let byte = memory_reader(addr.wrapping_add(col));
                    hex.push_str(&format!("{:02X} ", byte));
                    ascii.push(if (0x20..0x7F).contains(&byte) {
                        byte as char
                    } else {
                        '.'
                    });
                    if col == 7 {
                        hex.push(' ');
                    }
                }
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{:04X}", addr))
                            .monospace()
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.monospace(hex);
                    ui.label(
                        egui::RichText::new(ascii)
                            .monospace()
                            .color(ui.visuals().weak_text_color()),
                    );
                });
            }
        });
}
