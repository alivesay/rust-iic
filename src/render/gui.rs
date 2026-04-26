// Drive status info passed from the IWM to the GUI renderer.
pub struct DriveStatusInfo {
    pub has_disk: bool,
    pub is_active: bool,
    pub is_write_protected: bool,
    pub filename: Option<String>,
}

// Apple IIc font ROM for rendering toolbar labels
const CHAR_ROM: &[u8; 1024] = include_bytes!("../../assets/font.bin");

// Rasterize a string using the Apple IIc character ROM into an RGBA image.
fn rasterize_apple_label(text: &str) -> (usize, usize, Vec<u8>) {
    let scale = 2;
    let char_w = 7 * scale;
    let char_h = 8 * scale;
    let img_w = text.len() * char_w;
    let img_h = char_h;
    let mut pixels = vec![0u8; img_w * img_h * 4];

    for (ci, ch) in text.chars().enumerate() {
        // Map ASCII to font ROM offset: uppercase A-Z at 0x40-0x5A, numbers/symbols at 0x20-0x3F
        let code = ch as u8;
        let font_index = if code >= 0x20 && code <= 0x7F {
            code as usize
        } else {
            0x20
        };
        let font_offset = font_index * 8;

        for row in 0..8_usize {
            let font_byte = CHAR_ROM[font_offset + row];
            for bit in 0..7_usize {
                let pixel_on = (font_byte >> bit) & 1 != 0;
                if pixel_on {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = ci * char_w + bit * scale + sx;
                            let py = row * scale + sy;
                            let idx = (py * img_w + px) * 4;
                            pixels[idx] = 255;
                            pixels[idx + 1] = 255;
                            pixels[idx + 2] = 255;
                            pixels[idx + 3] = 255;
                        }
                    }
                }
            }
        }
    }

    (img_w, img_h, pixels)
}

// Pre-rendered Apple IIc font labels for toolbar buttons.
pub struct ToolbarLabels {
    pub run: egui::TextureHandle,
    pub stp: egui::TextureHandle,
    pub rst: egui::TextureHandle,
    pub pwr: egui::TextureHandle,
    pub col80: egui::TextureHandle,
    pub col40: egui::TextureHandle,
}

impl ToolbarLabels {
    pub fn load(ctx: &egui::Context) -> Self {
        fn make_label(ctx: &egui::Context, name: &str, text: &str) -> egui::TextureHandle {
            let (w, h, pixels) = rasterize_apple_label(text);
            let color_image = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
            ctx.load_texture(name, color_image, egui::TextureOptions::NEAREST)
        }
        Self {
            run: make_label(ctx, "lbl_run", "RUN"),
            stp: make_label(ctx, "lbl_stp", "STP"),
            rst: make_label(ctx, "lbl_rst", "RST"),
            pwr: make_label(ctx, "lbl_pwr", "PWR"),
            col80: make_label(ctx, "lbl_80", "80"),
            col40: make_label(ctx, "lbl_40", "40"),
        }
    }
}

// Toolbar drive icon textures
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
    labels: &ToolbarLabels,
) -> ToolbarAction {
    let mut action = ToolbarAction::default();

    egui::TopBottomPanel::bottom("toolbar")
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::new()
            .fill(egui::Color32::from_rgb(32, 32, 32))
            .corner_radius(0.0)
            .stroke(egui::Stroke::NONE)
            .inner_margin(egui::Margin::symmetric(8, 0)))
        .show(ctx, |ui| {
            ui.set_min_height(48.0);

            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.spacing_mut().button_padding = egui::vec2(4.0, 4.0);

                // Pause/Run button
                let pause_tex = if paused { &labels.run } else { &labels.stp };
                let pause_img = egui::Image::new(pause_tex)
                    .fit_to_exact_size(egui::vec2(pause_tex.size()[0] as f32, pause_tex.size()[1] as f32))
                    .tint(egui::Color32::WHITE);
                if ui
                    .add(egui::Button::image(pause_img).min_size(egui::vec2(32.0, 32.0)))
                    .clicked()
                {
                    action.toggle_pause = true;
                }

                // Reset button
                let rst_img = egui::Image::new(&labels.rst)
                    .fit_to_exact_size(egui::vec2(labels.rst.size()[0] as f32, labels.rst.size()[1] as f32))
                    .tint(egui::Color32::WHITE);
                if ui
                    .add(egui::Button::image(rst_img).min_size(egui::vec2(32.0, 32.0)))
                    .clicked()
                {
                    action.reset = true;
                }

                // Power button
                let pwr_img = egui::Image::new(&labels.pwr)
                    .fit_to_exact_size(egui::vec2(labels.pwr.size()[0] as f32, labels.pwr.size()[1] as f32))
                    .tint(egui::Color32::WHITE);
                if ui
                    .add(egui::Button::image(pwr_img).min_size(egui::vec2(32.0, 32.0)))
                    .clicked()
                {
                    action.power = true;
                }

                // 40/80 column toggle
                let col_tex = if col80 { &labels.col80 } else { &labels.col40 };
                let col_img = egui::Image::new(col_tex)
                    .fit_to_exact_size(egui::vec2(col_tex.size()[0] as f32, col_tex.size()[1] as f32))
                    .tint(egui::Color32::WHITE);
                if ui
                    .add(egui::Button::image(col_img).min_size(egui::vec2(32.0, 32.0)))
                    .clicked()
                {
                    action.toggle_col80 = true;
                }

                // push drive icons to the right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.spacing_mut().button_padding = egui::vec2(2.0, 2.0);

                    let drive_types = ["5.25\"", "5.25\"", "3.5\"", "3.5\""];
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
                        let response = ui.add(
                            egui::Button::image(img)
                                .sense(egui::Sense::click()),
                        );

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
                                if let Some(ref name) = drive.filename {
                                    ui.label(
                                        egui::RichText::new(name)
                                            .color(egui::Color32::from_rgb(180, 220, 255)),
                                    );
                                }
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
