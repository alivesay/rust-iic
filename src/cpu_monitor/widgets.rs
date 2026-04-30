pub fn flag_chip(ui: &mut egui::Ui, letter: &str, on: bool) {
    let (bg, fg) = if on {
        (egui::Color32::from_rgb(0x4A, 0xA8, 0x60), egui::Color32::WHITE)
    } else {
        (egui::Color32::from_gray(0x33), egui::Color32::from_gray(0x88))
    };
    let text = egui::RichText::new(letter)
        .monospace()
        .strong()
        .color(fg)
        .background_color(bg);
    ui.label(text);
}

pub fn format_cycles(cycles: u64) -> String {
    if cycles >= 1_000_000_000 {
        format!("{:.2}G", cycles as f64 / 1_000_000_000.0)
    } else if cycles >= 1_000_000 {
        format!("{:.2}M", cycles as f64 / 1_000_000.0)
    } else if cycles >= 10_000 {
        format!("{:.1}k", cycles as f64 / 1_000.0)
    } else {
        cycles.to_string()
    }
}
