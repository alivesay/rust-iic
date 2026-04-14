use bytemuck::{Pod, Zeroable};

/// CRT-Geom-Deluxe shader parameters.
/// GPU layout: 5 × vec4<f32> = 20 floats.
///   group0: crt_gamma, monitor_gamma, distance, radius
///   group1: corner_size, corner_smooth, overscan_x, overscan_y
///   group2: aperture_strength, aperture_brightboost, scanline_weight, luminance
///   group3: curvature_on, saturation, halation, rasterbloom
///   group4: blur_width, mask_type, reserved, reserved
#[derive(Clone, Debug)]
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
    pub _reserved0: f32,
    pub _reserved1: f32,
}

impl Default for ShaderParams {
    fn default() -> Self {
        Self {
            crt_gamma: 2.4,
            monitor_gamma: 2.2,
            distance: 1.37,
            radius: 2.0,
            corner_size: 0.002,
            corner_smooth: 100.0,
            overscan_x: 100.0,
            overscan_y: 100.0,
            aperture_strength: 0.48,
            aperture_brightboost: 0.16,
            scanline_weight: 0.245,
            luminance: 0.0,
            curvature: 1.0,
            saturation: 1.0,
            halation: 0.01,
            rasterbloom: 0.01,
            blur_width: 0.7,
            mask_type: 3.0,
            _reserved0: 0.0,
            _reserved1: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct ShaderParamsGpu {
    pub data: [f32; 20],
}

impl ShaderParams {
    pub fn to_gpu(&self) -> ShaderParamsGpu {
        ShaderParamsGpu {
            data: [
                self.crt_gamma, self.monitor_gamma, self.distance, self.radius,
                self.corner_size, self.corner_smooth, self.overscan_x, self.overscan_y,
                self.aperture_strength, self.aperture_brightboost, self.scanline_weight, self.luminance,
                self.curvature, self.saturation, self.halation, self.rasterbloom,
                self.blur_width, self.mask_type, self._reserved0, self._reserved1,
            ],
        }
    }
}

pub fn render_shader_ui(ctx: &egui::Context, params: &mut ShaderParams, open: &mut bool) -> bool {
    let mut changed = false;

    egui::Window::new("CRT-Geom-Deluxe Settings")
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
                changed |= ui.add(egui::Slider::new(&mut params.halation, 0.0..=0.3).text("Halation")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.rasterbloom, 0.0..=1.0).text("Raster Bloom")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.blur_width, 0.1..=4.0).text("Blur Width")).changed();

                ui.separator();
                ui.heading("Color");
                changed |= ui.add(egui::Slider::new(&mut params.crt_gamma, 0.7..=4.0).text("CRT Gamma")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.monitor_gamma, 0.7..=4.0).text("Monitor Gamma")).changed();
                changed |= ui.add(egui::Slider::new(&mut params.saturation, 0.0..=2.0).text("Saturation")).changed();

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
                        println!("------------------------------");
                    }
                });
            });
        });

    changed
}
