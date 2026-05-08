use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

// CRT-Geom-Deluxe shader parameters.
// GPU layout: 8 × vec4<f32> = 32 floats.
//   group0: crt_gamma, monitor_gamma, distance, radius
//   group1: corner_size, corner_smooth, overscan_x, overscan_y
//   group2: aperture_strength, aperture_brightboost, scanline_weight, luminance
//   group3: curvature_on, saturation, halation, rasterbloom
//   group4: blur_width, mask_type, vignette, phosphor
//   group5: glow, glow_width, vignette_opacity, flicker
//   group6: chromatic_aberration, white_preservation, tone_knee, v_roll
//   group7: chroma_blur_on, comb_filter_on, phosphor_spread_on, scanline_floor
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ShaderParams {
    // group0
    pub crt_gamma: f32,
    pub monitor_gamma: f32,
    pub distance: f32,
    pub radius: f32,
    // group1
    pub corner_size: f32,
    pub corner_smooth: f32,
    pub overscan_x: f32,
    pub overscan_y: f32,
    // group2
    pub aperture_strength: f32,
    pub aperture_brightboost: f32,
    pub scanline_weight: f32,
    pub luminance: f32,
    // group3
    pub curvature: f32,
    pub saturation: f32,
    pub halation: f32,
    pub rasterbloom: f32,
    // group4
    pub blur_width: f32,
    pub mask_type: f32,
    pub vignette: f32,
    pub phosphor: f32,
    // group5
    pub glow: f32,
    pub glow_width: f32,
    pub vignette_opacity: f32,
    pub flicker: f32,
    // group6
    pub chromatic_aberration: f32,
    pub v_roll: f32,  // V-Hold rolling effect speed (0=off)

    // NTSC signal-chain effects.
    pub chroma_blur: bool,      // 7-tap asymmetric I/Q blur in YIQ
    pub comb_filter: bool,      // 2-line YIQ comb (chroma cross-bleed)
    pub phosphor_spread: bool,  // 3-tap horizontal beam-spot spread
    pub white_preservation: f32, // 1.0 = clean white, 0.0 = NTSC
    pub chroma_saturation: f32,  // CPU NTSC decoder chroma scale (~2.2 default)
    pub chroma_luma_scale: f32,  // luma scale for chromatic pixels (~1.0 default, 0.8 = canonical //c palette)

    pub phosphor_spread_sigma_x: f32,    // horizontal Gaussian width
    pub phosphor_spread_sigma_y: f32,    // vertical Gaussian width
    pub phosphor_spread_intensity: f32,  // 0..1 mix between passthrough and blurred

    // Output tone curve.
    pub tone_knee: f32,       // Reinhard knee point [0..1]; values above asymptote to 1.0
    pub scanline_floor: f32,  // 0..1, fraction of brighter neighbor scanline used as gap fill

    // CRT input-gamma curve selector (voltage -> linear luminance).
    //   0 = pure power: L = V^crt_gamma
    //   1 = customizable measured CRT (crtProperToLinear): alpha1=crt_gamma,
    //       alpha2=crt_alpha2, b=crt_black_lift
    //   2 = locked measured CRT (crtProper2ToLinear): no parameters
    pub crt_curve_mode: u32,
    pub crt_alpha2: f32,      // mode 1 only
    pub crt_black_lift: f32,  // mode 1 only (0..~0.05)

    // ---- LCD shader parameters ----
    pub lcd_invert: bool,
    pub lcd_ghost_decay: f32,        // 0..1, frame-to-frame persistence
    pub lcd_corner_radius_px: f32,   // outer panel corner radius (output px)
    pub lcd_vignette_strength: f32,  // 0..1 mix toward edge tint
    pub lcd_vignette_inner: f32,     // normalised radius where vignette starts
    pub lcd_vignette_outer: f32,     // normalised radius where vignette saturates
    pub lcd_vignette_tint: [f32; 3], // sRGB 0..1 edge tint colour
    pub lcd_threshold: f32,          // 1-bit cutoff luma (0..1); below = off, above = on
    pub lcd_contrast: f32,           // pre-threshold contrast scale around 0.5 (0..3)
    pub lcd_bg_color: [f32; 3],      // sRGB 0..1 panel background (off pixels)
    pub lcd_fg_color: [f32; 3],      // sRGB 0..1 panel foreground (lit pixels)
}

impl Default for ShaderParams {
    fn default() -> Self {
        Self {
            // CRT response curve (gun voltage -> emitted linear luminance).
            // Applied at the input-gamma decode pass; ~2.5 matches a
            // typical Apple //c CRT phosphor.
            crt_gamma: 2.8,
            monitor_gamma: 2.2,
            distance: 3.00,
            radius: 1.3,
            corner_size: 0.001,
            corner_smooth: 2000.0,
            overscan_x: 100.0,
            overscan_y: 100.0,

            scanline_weight: 0.24,
            luminance: 0.1,
            curvature: 1.0,

            saturation: 1.00,
            halation: 0.3,
            blur_width: 0.8,
            rasterbloom: 0.32,
            mask_type: 3.0,
            aperture_strength: 0.3,
            aperture_brightboost: 0.3,
            vignette: 2.22,
            phosphor: 0.58,
            glow: 0.004,
            glow_width: 20.0,
            vignette_opacity: 0.37,
            flicker: 0.4,
            chromatic_aberration: 1.0,
            v_roll: 0.1,

            chroma_blur: true,
            comb_filter: true,
            phosphor_spread: true,
            white_preservation: 0.0,
            chroma_saturation: 1.5,
            chroma_luma_scale: 1.0,

            phosphor_spread_sigma_x: 0.52,
            phosphor_spread_sigma_y: 0.05,
            phosphor_spread_intensity: 1.0,

            tone_knee: 0.75,
            scanline_floor: 0.035,

            crt_curve_mode: 1,
            crt_alpha2: 3.0,
            crt_black_lift: 0.0181,

            lcd_invert: true,
            lcd_ghost_decay: 0.85,
            lcd_corner_radius_px: 15.0,
            lcd_vignette_strength: 0.5,
            lcd_vignette_inner: 0.0,
            lcd_vignette_outer: 1.12,
            lcd_vignette_tint: [28.0 / 255.0, 52.0 / 255.0, 58.0 / 255.0],
            lcd_threshold: 0.43,
            lcd_contrast: 1.0,
            lcd_bg_color: [88.0 / 255.0, 105.0 / 255.0, 50.0 / 255.0],
            lcd_fg_color: [35.0 / 255.0,  47.0 / 255.0, 47.0 / 255.0],        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct ShaderParamsGpu {
    pub data: [f32; 32],
}

impl ShaderParams {
    pub fn to_gpu(&self) -> ShaderParamsGpu {
        let b = |v: bool| if v { 1.0_f32 } else { 0.0_f32 };
        ShaderParamsGpu {
            data: [
                self.crt_gamma, self.monitor_gamma, self.distance, self.radius,
                self.corner_size, self.corner_smooth, self.overscan_x, self.overscan_y,
                self.aperture_strength, self.aperture_brightboost, self.scanline_weight, self.luminance,
                self.curvature, self.saturation, self.halation, self.rasterbloom,
                self.blur_width, self.mask_type, self.vignette, self.phosphor,
                self.glow, self.glow_width, self.vignette_opacity, self.flicker,  // group5
                self.chromatic_aberration, self.white_preservation, self.tone_knee, self.v_roll,  // group6
                b(self.chroma_blur), b(self.comb_filter), b(self.phosphor_spread), self.scanline_floor, // group7
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShaderUiTab {
    Crt,
    Lcd,
}

pub fn render_shader_ui(ctx: &egui::Context, params: &mut ShaderParams, open: &mut bool) -> ShaderUiResult {
    let mut changed = false;
    let mut save_clicked = false;

    egui::Window::new("Shader Settings")
        .open(open)
        .resizable(true)
        .default_width(340.0)
        .show(ctx, |ui| {
            let tab_id = ui.id().with("shader_ui_tab");
            let mut tab = ui.data_mut(|d| *d.get_temp_mut_or(tab_id, ShaderUiTab::Crt));

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if ui.selectable_label(tab == ShaderUiTab::Crt, "CRT").clicked() {
                    tab = ShaderUiTab::Crt;
                }
                if ui.selectable_label(tab == ShaderUiTab::Lcd, "LCD").clicked() {
                    tab = ShaderUiTab::Lcd;
                }
            });
            ui.data_mut(|d| d.insert_temp(tab_id, tab));
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                match tab {
                    ShaderUiTab::Crt => render_crt_tab(ui, params, &mut changed),
                    ShaderUiTab::Lcd => render_lcd_tab(ui, params, &mut changed),
                }

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Reset Defaults").clicked() {
                        *params = ShaderParams::default();
                        changed = true;
                    }
                    if ui.button("Save to config").clicked() {
                        save_clicked = true;
                    }
                });
            });
        });
        ShaderUiResult { changed, save_clicked }
}

fn render_crt_tab(ui: &mut egui::Ui, params: &mut ShaderParams, changed: &mut bool) {
                ui.heading("Geometry");
                *changed |= ui.add(egui::Slider::new(&mut params.curvature, 0.0..=1.0).text("Curvature On/Off")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.distance, 0.1..=3.0).text("Distance")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.radius, 0.5..=10.0).text("Radius")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.corner_size, 0.001..=0.1).text("Corner Size")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.corner_smooth, 100.0..=2000.0).text("Corner Smooth")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.overscan_x, 80.0..=120.0).text("Overscan X %")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.overscan_y, 80.0..=120.0).text("Overscan Y %")).changed();

                ui.separator();
                ui.heading("Scanlines");
                *changed |= ui.add(egui::Slider::new(&mut params.scanline_weight, 0.1..=0.5).text("Scanline Weight")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.luminance, 0.0..=1.0).text("Luminance")).changed();

                ui.separator();
                ui.heading("Shadow Mask");
                *changed |= ui.add(egui::Slider::new(&mut params.mask_type, 1.0..=3.0).step_by(1.0).text("Mask Type (1=grille 2=slot 3=delta)")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.aperture_strength, 0.0..=1.0).text("Mask Strength")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.aperture_brightboost, 0.0..=1.0).text("Mask Bright Boost")).changed();

                ui.separator();
                ui.heading("Halation & Bloom");
                *changed |= ui.add(egui::Slider::new(&mut params.halation, 0.0..=2.0).text("Halation")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.blur_width, 0.2..=3.0).text("Halation Width")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.glow, 0.0..=0.05).text("Glow")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.glow_width, 0.5..=30.0).text("Glow Width")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.rasterbloom, 0.0..=1.0).text("Raster Bloom")).changed();

                ui.separator();
                ui.heading("Color");
                ui.label("CRT response curve (voltage -> luminance)");
                ui.horizontal(|ui| {
                    *changed |= ui.radio_value(&mut params.crt_curve_mode, 0u32, "Pure power").changed();
                    *changed |= ui.radio_value(&mut params.crt_curve_mode, 1u32, "Measured (custom)").changed();
                    *changed |= ui.radio_value(&mut params.crt_curve_mode, 2u32, "Measured (locked)").changed();
                });
                let alpha1_label = if params.crt_curve_mode == 1 {
                    "alpha1 (CRT Gamma, high-side power)"
                } else {
                    "CRT Gamma (input response)"
                };
                ui.add_enabled_ui(params.crt_curve_mode != 2, |ui| {
                    *changed |= ui.add(egui::Slider::new(&mut params.crt_gamma, 1.8..=3.2).text(alpha1_label)).changed();
                });
                ui.add_enabled_ui(params.crt_curve_mode == 1, |ui| {
                    *changed |= ui.add(egui::Slider::new(&mut params.crt_alpha2, 2.0..=4.0).text("alpha2 (low-side power)")).changed();
                    *changed |= ui.add(egui::Slider::new(&mut params.crt_black_lift, 0.0..=0.05).text("Black lift b (brightness)")).changed();
                });
                *changed |= ui.add(egui::Slider::new(&mut params.saturation, 0.0..=2.0).text("Saturation")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.tone_knee, 0.5..=1.0).text("Tone Knee (highlight rolloff)")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.scanline_floor, 0.0..=0.3).text("Scanline Floor (gap fill)")).changed();

                ui.separator();
                ui.heading("Effects");
                *changed |= ui.add(egui::Slider::new(&mut params.vignette, 0.0..=3.0).text("Vignette Size")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.vignette_opacity, 0.0..=1.0).text("Vignette Opacity")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.phosphor, 0.0..=0.95).text("Phosphor Persistence")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.flicker, 0.0..=1.0).text("CRT Flicker")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.chromatic_aberration, 0.0..=1.0).text("Chromatic Aberration")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.v_roll, 0.0..=1.0).text("V-Roll Bar")).changed();

                ui.separator();
                ui.heading("NTSC Signal Chain");
                *changed |= ui.add(egui::Slider::new(&mut params.white_preservation, 0.0..=1.0).text("White Preservation (1=clean, 0=NTSC)")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.chroma_saturation, 0.0..=6.0).text("Chroma Saturation")).changed();
                *changed |= ui.add(egui::Slider::new(&mut params.chroma_luma_scale, 0.0..=1.5).text("Chroma Luma Scale")).changed();
                *changed |= ui.checkbox(&mut params.chroma_blur, "Chroma Blur").changed();
                *changed |= ui.checkbox(&mut params.comb_filter, "Comb Filter").changed();
                *changed |= ui.checkbox(&mut params.phosphor_spread, "Phosphor Spread").changed();
                ui.indent("phosphor_spread_params", |ui| {
                    *changed |= ui.add(egui::Slider::new(&mut params.phosphor_spread_sigma_x, 0.0..=4.0).text("Spread σ X (src px)")).changed();
                    *changed |= ui.add(egui::Slider::new(&mut params.phosphor_spread_sigma_y, 0.0..=2.0).text("Spread σ Y (src px)")).changed();
                    *changed |= ui.add(egui::Slider::new(&mut params.phosphor_spread_intensity, 0.0..=1.0).text("Spread Intensity")).changed();
                });
}

fn render_lcd_tab(ui: &mut egui::Ui, params: &mut ShaderParams, changed: &mut bool) {
    ui.heading("Panel");
    *changed |= ui.checkbox(&mut params.lcd_invert, "Invert (F5)").changed();
    *changed |= ui
        .add(egui::Slider::new(&mut params.lcd_corner_radius_px, 0.0..=40.0).text("Corner radius (px)"))
        .changed();

    ui.horizontal(|ui| {
        ui.label("Background:");
        if color_edit_srgb_f32(ui, &mut params.lcd_bg_color) {
            *changed = true;
        }
        ui.label("Foreground:");
        if color_edit_srgb_f32(ui, &mut params.lcd_fg_color) {
            *changed = true;
        }
        if ui.button("Reset").clicked() {
            params.lcd_bg_color = [88.0 / 255.0, 105.0 / 255.0, 50.0 / 255.0];
            params.lcd_fg_color = [35.0 / 255.0,  47.0 / 255.0, 47.0 / 255.0];
            *changed = true;
        }
    });

    ui.separator();
    ui.heading("Pixel shape");
    let resp = ui.add(
        egui::Slider::new(&mut params.lcd_contrast, 0.0..=4.0)
            .fixed_decimals(2)
            .text("Contrast"),
    );
    *changed |= resp.changed();
    resp.on_hover_text("Steepens the dot-coverage curve. 1.0 = default; >1 = harder edges.");

    let resp = ui.add(
        egui::Slider::new(&mut params.lcd_threshold, 0.0..=1.0)
            .fixed_decimals(3)
            .text("Threshold"),
    );
    *changed |= resp.changed();
    resp.on_hover_text("Coverage cutoff. Lower = thicker dots; higher = thinner.");

    ui.separator();
    ui.heading("Persistence");
    *changed |= ui
        .add(egui::Slider::new(&mut params.lcd_ghost_decay, 0.0..=0.99).text("Ghost trail"))
        .changed();

    ui.separator();
    ui.heading("Vignette");
    *changed |= ui.add(egui::Slider::new(&mut params.lcd_vignette_strength, 0.0..=1.0).text("Strength")).changed();
    *changed |= ui.add(egui::Slider::new(&mut params.lcd_vignette_inner, 0.0..=1.5).text("Inner radius")).changed();
    *changed |= ui.add(egui::Slider::new(&mut params.lcd_vignette_outer, 0.1..=2.0).text("Outer radius")).changed();
    ui.horizontal(|ui| {
        ui.label("Edge tint:");
        let mut tint8 = [
            (params.lcd_vignette_tint[0] * 255.0).round().clamp(0.0, 255.0) as u8,
            (params.lcd_vignette_tint[1] * 255.0).round().clamp(0.0, 255.0) as u8,
            (params.lcd_vignette_tint[2] * 255.0).round().clamp(0.0, 255.0) as u8,
        ];
        if ui.color_edit_button_srgb(&mut tint8).changed() {
            params.lcd_vignette_tint = [
                tint8[0] as f32 / 255.0,
                tint8[1] as f32 / 255.0,
                tint8[2] as f32 / 255.0,
            ];
            *changed = true;
        }
    });
}

// Helper: egui colour picker that operates on `[f32; 3]` sRGB triples.
fn color_edit_srgb_f32(ui: &mut egui::Ui, c: &mut [f32; 3]) -> bool {
    let mut tint8 = [
        (c[0] * 255.0).round().clamp(0.0, 255.0) as u8,
        (c[1] * 255.0).round().clamp(0.0, 255.0) as u8,
        (c[2] * 255.0).round().clamp(0.0, 255.0) as u8,
    ];
    if ui.color_edit_button_srgb(&mut tint8).changed() {
        *c = [
            tint8[0] as f32 / 255.0,
            tint8[1] as f32 / 255.0,
            tint8[2] as f32 / 255.0,
        ];
        true
    } else {
        false
    }
}

#[derive(Default, Clone, Copy)]
pub struct ShaderUiResult {
    pub changed: bool,
    pub save_clicked: bool,
}
