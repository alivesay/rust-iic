// Drive status info passed from the IWM to the GUI renderer.
pub struct DriveStatusInfo {
    pub has_disk: bool,
    pub is_active: bool,
    pub is_write_protected: bool,
}

/// Toolbar drive icon textures, loaded once from embedded PNGs.
pub struct DriveIcons {
    pub disk1: egui::TextureHandle,
    pub disk2: egui::TextureHandle,
    pub disk35_1: egui::TextureHandle,
    pub disk35_2: egui::TextureHandle,
}

impl DriveIcons {
    pub fn load(ctx: &egui::Context) -> Self {
        fn load_png(ctx: &egui::Context, name: &str, bytes: &[u8]) -> egui::TextureHandle {
            let img = image::load_from_memory(bytes).expect("embedded PNG decode failed");
            let rgba = img.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let pixels = rgba.into_raw();
            let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
            ctx.load_texture(name, color_image, egui::TextureOptions::NEAREST)
        }
        Self {
            disk1: load_png(ctx, "disk1", include_bytes!("../../assets/disk1.png")),
            disk2: load_png(ctx, "disk2", include_bytes!("../../assets/disk2.png")),
            disk35_1: load_png(ctx, "disk35_1", include_bytes!("../../assets/disk35_1.png")),
            disk35_2: load_png(ctx, "disk35_2", include_bytes!("../../assets/disk35_2.png")),
        }
    }

    pub fn texture_for(&self, drive_index: usize) -> &egui::TextureHandle {
        match drive_index {
            0 => &self.disk1,
            1 => &self.disk2,
            2 => &self.disk35_1,
            3 => &self.disk35_2,
            _ => &self.disk1,
        }
    }
}

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
    pub toggle_pause: bool,
}

// Render the toolbar as an egui bottom panel overlay.
// Returns actions that were triggered by user interaction.
pub fn render_toolbar_ui(
    ctx: &egui::Context,
    drives: &[DriveStatusInfo; 4],
    col80: bool,
    paused: bool,
    icons: &DriveIcons,
) -> ToolbarAction {
    let mut action = ToolbarAction::default();

    egui::TopBottomPanel::bottom("toolbar")
        .resizable(false)
        .frame(egui::Frame::new()
            .fill(egui::Color32::from_rgb(32, 32, 32))
            .inner_margin(egui::Margin::symmetric(8, 0)))
        .show(ctx, |ui| {
            ui.set_min_height(48.0);

            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.spacing_mut().button_padding = egui::vec2(4.0, 4.0);

                // Pause/Run button
                let pause_text = if paused { "\u{25B6}" } else { "\u{23F8}" };
                let pause_color = if paused {
                    egui::Color32::from_rgb(100, 255, 100)
                } else {
                    egui::Color32::from_rgb(255, 200, 100)
                };
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(pause_text)
                                .color(pause_color)
                                .size(16.0),
                        )
                        .min_size(egui::vec2(32.0, 32.0)),
                    )
                    .clicked()
                {
                    action.toggle_pause = true;
                }

                // Reset button
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("\u{21BA}")
                                .color(egui::Color32::from_rgb(200, 200, 200))
                                .size(18.0),
                        )
                        .min_size(egui::vec2(32.0, 32.0)),
                    )
                    .clicked()
                {
                    action.reset = true;
                }

                // Power button
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("\u{23FB}")
                                .color(egui::Color32::from_rgb(255, 100, 100))
                                .size(18.0),
                        )
                        .min_size(egui::vec2(32.0, 32.0)),
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

                // Push drive icons to the right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.spacing_mut().button_padding = egui::vec2(2.0, 2.0);

                    let drive_types = ["5.25\"", "5.25\"", "3.5\"", "3.5\""];
                    // Reverse order so right-to-left layout renders disk35_2 rightmost
                    for i in (0..4).rev() {
                        let drive = &drives[i];

                        let tint = if drive.is_active {
                            egui::Color32::from_rgb(120, 255, 120)
                        } else if drive.has_disk {
                            egui::Color32::from_rgb(200, 200, 200)
                        } else {
                            egui::Color32::from_rgb(60, 60, 60)
                        };

                        let tex = icons.texture_for(i);
                        let img = egui::Image::new(tex)
                            .fit_to_exact_size(egui::vec2(32.0, 32.0))
                            .tint(tint);
                        let response = ui.add(egui::Button::image(img));

                        if response.double_clicked() && drive.has_disk {
                            action.eject_disk = Some(i);
                        } else if response.clicked() {
                            action.load_disk = Some(i);
                        }
                        if response.secondary_clicked() && drive.has_disk {
                            action.toggle_write_protect = Some(i);
                        }

                        response.on_hover_ui(|ui| {
                            ui.set_min_width(120.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} Drive {}",
                                    drive_types[i], i + 1
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
                    }
                });
            });
        });

    action
}
