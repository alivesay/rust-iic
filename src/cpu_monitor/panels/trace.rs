use std::collections::VecDeque;
use crate::cpu_monitor::state::CpuTraceEntry;

pub fn render(
    ui: &mut egui::Ui,
    trace_buffer: &VecDeque<CpuTraceEntry>,
    auto_scroll: bool,
) {
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::vertical()
        .id_salt("cpu_trace")
        .auto_shrink([false, false])
        .stick_to_bottom(auto_scroll)
        .show_rows(ui, row_height, trace_buffer.len(), |ui, range| {
            for idx in range {
                if let Some(entry) = trace_buffer.get(idx) {
                    let highlight = idx + 1 == trace_buffer.len();
                    let text = format!(
                        "  {:04X}  {}  A:{:02X} X:{:02X} Y:{:02X} SP:{:02X}  {}",
                        entry.pc,
                        entry.format_bytes(),
                        entry.a,
                        entry.x,
                        entry.y,
                        entry.sp,
                        entry.format_flags(),
                    );
                    let mut rich = egui::RichText::new(text).monospace();
                    if highlight {
                        rich = rich.strong().color(egui::Color32::from_rgb(0xE5, 0xC0, 0x70));
                    }
                    ui.label(rich);
                }
            }
        });
}
