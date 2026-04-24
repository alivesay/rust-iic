// Drive status info passed from the IWM to the GUI renderer.
pub struct DriveStatusInfo {
    pub has_disk: bool,
    pub is_active: bool,
    pub is_write_protected: bool,
}

// Direct 1:1 copy from src into frame. Both must have same dimensions.
// Used when buffer is at source resolution (CRT mode).
pub fn blit_direct(frame: &mut [u8], src: &[u8]) {
    let len = src.len().min(frame.len());
    frame[..len].copy_from_slice(&src[..len]);
}

// Nearest-neighbor blit from src into frame at (dst_x, dst_y) scaled to (dst_w × dst_h).
// Preserves pixel-perfect sharpness, each source pixel maps to an integer number of
// destination pixels. Caller should ensure dst_w/dst_h are integer multiples of src_w/src_h.
pub fn blit_nearest(
    frame: &mut [u8],
    frame_w: u32,
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_x: u32,
    dst_y: u32,
    dst_w: u32,
    dst_h: u32,
) {
    for y in 0..dst_h {
        let src_y = (y as u64 * src_h as u64 / dst_h as u64) as usize;
        let src_y = src_y.min(src_h as usize - 1);
        let src_row = src_y * src_w as usize * 4;
        let dst_row = (dst_y + y) as usize * frame_w as usize * 4;

        for x in 0..dst_w {
            let src_x = (x as u64 * src_w as u64 / dst_w as u64) as usize;
            let src_x = src_x.min(src_w as usize - 1);

            let si = src_row + src_x * 4;
            let di = dst_row + (dst_x + x) as usize * 4;

            if si + 4 <= src.len() && di + 4 <= frame.len() {
                frame[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
}

// Toolbar action returned from the egui toolbar.
#[derive(Default)]
pub struct ToolbarAction {
    pub reset: bool,
    pub power: bool,
    pub toggle_col80: bool,
    pub load_disk: Option<usize>,
    pub toggle_write_protect: Option<usize>,
    pub eject_disk: Option<usize>,
}

// Render the toolbar as an egui bottom panel overlay.
// Returns actions that were triggered by user interaction.
pub fn render_toolbar_ui(
    ctx: &egui::Context,
    drives: &[DriveStatusInfo; 4],
    col80: bool,
) -> ToolbarAction {
    let mut action = ToolbarAction::default();

    egui::TopBottomPanel::bottom("toolbar")
        .resizable(false)
        .frame(egui::Frame::new().fill(egui::Color32::from_rgb(32, 32, 32)))
        .show(ctx, |ui| {
            ui.set_min_height(48.0);

            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;

                // Reset button
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("RST")
                                .color(egui::Color32::from_rgb(200, 200, 200))
                                .size(14.0),
                        )
                        .min_size(egui::vec2(40.0, 32.0)),
                    )
                    .clicked()
                {
                    action.reset = true;
                }

                // Power button
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("PWR")
                                .color(egui::Color32::from_rgb(255, 100, 100))
                                .size(14.0),
                        )
                        .min_size(egui::vec2(40.0, 32.0)),
                    )
                    .clicked()
                {
                    action.power = true;
                }

                // 40/80 column toggle
                let col_text = if col80 { "80" } else { "40" };
                let col_color = if col80 {
                    egui::Color32::from_rgb(100, 200, 100)
                } else {
                    egui::Color32::from_rgb(200, 200, 100)
                };
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new(col_text).color(col_color).size(14.0))
                            .min_size(egui::vec2(32.0, 32.0)),
                    )
                    .clicked()
                {
                    action.toggle_col80 = true;
                }

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(16.0);

                // Drive slots
                let drive_labels = ["5¼₁", "5¼₂", "3½₁", "3½₂"];
                let drive_types = ["5.25\"", "5.25\"", "3.5\"", "3.5\""];
                for i in 0..4 {
                    let drive = &drives[i];

                    // Button color based on state
                    let (text, color) = if drive.is_active {
                        ("●", egui::Color32::from_rgb(120, 255, 120))
                    } else if drive.has_disk {
                        ("●", egui::Color32::from_rgb(200, 200, 200))
                    } else {
                        ("○", egui::Color32::from_rgb(60, 60, 60))
                    };

                    let btn_text = format!("{}\n{}", drive_labels[i], text);
                    let response = ui.add(
                        egui::Button::new(egui::RichText::new(btn_text).color(color).size(12.0))
                            .min_size(egui::vec2(36.0, 36.0)),
                    );

                    if response.clicked() {
                        action.load_disk = Some(i);
                    }
                    if response.double_clicked() && drive.has_disk {
                        action.eject_disk = Some(i);
                    }
                    if response.secondary_clicked() && drive.has_disk {
                        action.toggle_write_protect = Some(i);
                    }

                    // Hover tooltip
                    response.on_hover_ui(|ui| {
                        ui.set_min_width(120.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "{} Drive {}",
                                drive_types[i], drive_labels[i]
                            ))
                            .strong(),
                        );
                        ui.separator();

                        if drive.has_disk {
                            ui.label(format!(
                                "Status: {}",
                                if drive.is_active { "Active" } else { "Idle" }
                            ));

                            let wp_text = if drive.is_write_protected {
                                "🔒 Write Protected"
                            } else {
                                "🔓 Writable"
                            };
                            let wp_color = if drive.is_write_protected {
                                egui::Color32::from_rgb(255, 120, 120)
                            } else {
                                egui::Color32::from_rgb(120, 255, 120)
                            };
                            ui.label(egui::RichText::new(wp_text).color(wp_color));

                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new("Click: Load disk")
                                    .small()
                                    .color(egui::Color32::GRAY),
                            );
                            ui.label(
                                egui::RichText::new("Double-click: Eject")
                                    .small()
                                    .color(egui::Color32::GRAY),
                            );
                            ui.label(
                                egui::RichText::new("Right-click: Toggle write protect")
                                    .small()
                                    .color(egui::Color32::GRAY),
                            );
                        } else {
                            ui.label("No disk inserted");
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new("Click to load disk")
                                    .small()
                                    .color(egui::Color32::GRAY),
                            );
                        }
                    });

                    if i < 3 {
                        ui.add_space(4.0);
                    }
                }
            });
        });

    action
}
