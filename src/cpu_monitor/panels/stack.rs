use crate::cpu_monitor::state::CpuState;

pub fn render(
    ui: &mut egui::Ui,
    state: &CpuState,
    memory_reader: &dyn Fn(u16) -> u8,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("STACK").small().strong());
        ui.weak(format!("SP:{:02X}", state.sp));
    });

    egui::ScrollArea::vertical()
        .id_salt("cpu_stack")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let sp = state.sp;
            let mut shown = 0;
            let mut offset: u16 = 1;
            let top_color = egui::Color32::from_rgb(0xE5, 0xC0, 0x70);
            while sp as u16 + offset <= 0xFF {
                let stack_addr = 0x0100u16 + sp as u16 + offset;
                let value = memory_reader(stack_addr);
                let line = if shown == 0 {
                    format!("\u{25B6} {:04X}  {:02X}", stack_addr, value)
                } else {
                    format!("  {:04X}  {:02X}", stack_addr, value)
                };
                let mut rich = egui::RichText::new(line).monospace();
                if shown == 0 {
                    rich = rich.strong().color(top_color);
                }
                ui.label(rich);
                shown += 1;
                offset += 1;
                if shown >= 64 {
                    break;
                }
            }
            if shown == 0 {
                ui.weak("(empty)");
            }
        });
}
