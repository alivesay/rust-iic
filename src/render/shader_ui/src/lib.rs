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
        }
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

pub fn render_shader_ui(ctx: &egui::Context, params: &mut ShaderParams, open: &mut bool) -> ShaderUiResult {
    let mut changed = false;
    let mut save_clicked = false;

    egui::Window::new("Shader Settings")
        .open(open)
        .resizable(true)
        .default_width(320.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Geometry");
                changed |= ui.add(egui::Slider::new(&mut params.curvature, 0.0..=1.0).text("Curvature On/Off")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.distance, 0.1..=3.0).text("Distance")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.radius, 0.5..=10.0).text("Radius")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.corner_size, 0.001..=0.1).text("Corner Size")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.corner_smooth, 100.0..=2000.0).text("Corner Smooth")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.overscan_x, 80.0..=120.0).text("Overscan X %")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.overscan_y, 80.0..=120.0).text("Overscan Y %")).changed();

                ui.separator();
                ui.heading("Scanlines");
                changed |= ui.add(egui::Slider::new(&mut params.scanline_weight, 0.1..=0.5).text("Scanline Weight")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.luminance, 0.0..=1.0).text("Luminance")).changed();

                ui.separator();
                ui.heading("Shadow Mask");
                changed |= ui.add(egui::Slider::new(&mut params.mask_type, 1.0..=3.0).step_by(1.0).text("Mask Type (1=grille 2=slot 3=delta)")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.aperture_strength, 0.0..=1.0).text("Mask Strength")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.aperture_brightboost, 0.0..=1.0).text("Mask Bright Boost")).changed();

                ui.separator();
                ui.heading("Halation & Bloom");
                changed |= ui.add(egui::Slider::new(&mut params.halation, 0.0..=2.0).text("Halation")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.blur_width, 0.2..=3.0).text("Halation Width")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.glow, 0.0..=0.05).text("Glow")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.glow_width, 0.5..=30.0).text("Glow Width")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.rasterbloom, 0.0..=1.0).text("Raster Bloom")).changed();

                ui.separator();
                ui.heading("Color");
                ui.label("CRT response curve (voltage -> luminance)");
                ui.horizontal(|ui| {
                    changed |= ui.radio_value(&mut params.crt_curve_mode, 0u32, "Pure power").changed();
                    changed |= ui.radio_value(&mut params.crt_curve_mode, 1u32, "Measured (custom)").changed();
                    changed |= ui.radio_value(&mut params.crt_curve_mode, 2u32, "Measured (locked)").changed();
                });
                let alpha1_label = if params.crt_curve_mode == 1 {
                    "alpha1 (CRT Gamma, high-side power)"
                } else {
                    "CRT Gamma (input response)"
                };
                ui.add_enabled_ui(params.crt_curve_mode != 2, |ui| {
                    changed |= ui.add(egui::Slider::new(&mut params.crt_gamma, 1.8..=3.2).text(alpha1_label)).changed();
                });
                ui.add_enabled_ui(params.crt_curve_mode == 1, |ui| {
                    changed |= ui.add(egui::Slider::new(&mut params.crt_alpha2, 2.0..=4.0).text("alpha2 (low-side power)")).changed();
                    changed |= ui.add(egui::Slider::new(&mut params.crt_black_lift, 0.0..=0.05).text("Black lift b (brightness)")).changed();
                });
                changed |= ui.add(egui::Slider::new(&mut params.saturation, 0.0..=2.0).text("Saturation")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.tone_knee, 0.5..=1.0).text("Tone Knee (highlight rolloff)")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.scanline_floor, 0.0..=0.3).text("Scanline Floor (gap fill)")).changed();

                ui.separator();
                ui.heading("Effects");
                changed |= ui.add(egui::Slider::new(&mut params.vignette, 0.0..=3.0).text("Vignette Size")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.vignette_opacity, 0.0..=1.0).text("Vignette Opacity")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.phosphor, 0.0..=0.95).text("Phosphor Persistence")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.flicker, 0.0..=1.0).text("CRT Flicker")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.chromatic_aberration, 0.0..=1.0).text("Chromatic Aberration")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.v_roll, 0.0..=1.0).text("V-Roll Bar")).changed();

                ui.separator();
                ui.heading("NTSC Signal Chain");
                changed |= ui.add(egui::Slider::new(&mut params.white_preservation, 0.0..=1.0).text("White Preservation (1=clean, 0=NTSC)")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.chroma_saturation, 0.0..=6.0).text("Chroma Saturation")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.chroma_luma_scale, 0.0..=1.5).text("Chroma Luma Scale")).changed();
                changed |= ui.checkbox(&mut params.chroma_blur, "Chroma Blur").changed();
                changed |= ui.checkbox(&mut params.comb_filter, "Comb Filter").changed();
                changed |= ui.checkbox(&mut params.phosphor_spread, "Phosphor Spread").changed();
                ui.indent("phosphor_spread_params", |ui| {
                    changed |= ui.add(egui::Slider::new(&mut params.phosphor_spread_sigma_x, 0.0..=4.0).text("Spread σ X (src px)")).changed();
                    changed |= ui.add(egui::Slider::new(&mut params.phosphor_spread_sigma_y, 0.0..=2.0).text("Spread σ Y (src px)")).changed();
                    changed |= ui.add(egui::Slider::new(&mut params.phosphor_spread_intensity, 0.0..=1.0).text("Spread Intensity")).changed();
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Reset Defaults").clicked() {
                        *params = ShaderParams::default();
                        changed = true;
                    }
                    if ui.button("Print Values").clicked() {
                        println!("--- CRT-Geom-Deluxe Values ---");
                        println!("CRTgamma           = {:.2}", params.crt_gamma);
                        println!("monitorgamma       = {:.2}", params.monitor_gamma);
                        println!("d                  = {:.2}", params.distance);
                        println!("R                  = {:.2}", params.radius);
                        println!("cornersize         = {:.3}", params.corner_size);
                        println!("cornersmooth       = {:.0}", params.corner_smooth);
                        println!("overscan_x         = {:.0}", params.overscan_x);
                        println!("overscan_y         = {:.0}", params.overscan_y);
                        println!("aperture_strength  = {:.2}", params.aperture_strength);
                        println!("aperture_brightbst = {:.2}", params.aperture_brightboost);
                        println!("scanline_weight    = {:.2}", params.scanline_weight);
                        println!("lum                = {:.2}", params.luminance);
                        println!("CURVATURE          = {:.0}", params.curvature);
                        println!("SATURATION         = {:.2}", params.saturation);
                        println!("halation           = {:.2}", params.halation);
                        println!("rasterbloom        = {:.2}", params.rasterbloom);
                        println!("blur_width         = {:.1}", params.blur_width);
                        println!("mask_type          = {:.0}", params.mask_type);
                        println!("vignette           = {:.2}", params.vignette);
                        println!("vignette_opacity   = {:.2}", params.vignette_opacity);
                        println!("phosphor           = {:.2}", params.phosphor);
                        println!("glow               = {:.2} (effective {:.3})", params.glow, params.glow * 0.1);
                        println!("glow_width         = {:.2}", params.glow_width);
                        println!("flicker            = {:.2}", params.flicker);
                        println!("chrom_aberration   = {:.2}", params.chromatic_aberration);
                        println!("------------------------------");
                    }
                    if ui.button("Save to config").clicked() {
                        save_clicked = true;
                    }
                });
            });
        });

    ShaderUiResult { changed, save_clicked }
}

#[derive(Default, Clone, Copy)]
pub struct ShaderUiResult {
    pub changed: bool,
    pub save_clicked: bool,
}
