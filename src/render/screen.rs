pub struct RendererInit<'a> {
    pub device: &'a wgpu::Device,
    pub surface_width: u32,
    pub surface_height: u32,
    pub buffer_width: u32,
    pub buffer_height: u32,
    pub bar_height: u32,
    pub source_width: f32,
    pub source_height: f32,
    pub surface_format: wgpu::TextureFormat,
}

pub struct ContentRect {
    pub surface_w: u32,
    pub surface_h: u32,
    pub offset_x: u32,
    pub offset_y: u32,
    pub dst_w: u32,
    pub dst_h: u32,
    pub bar_h: u32,
    pub source_width: f32,
    pub source_height: f32,

    pub overscan_x_px: u32,
    pub overscan_y_px: u32,
}

pub trait PostProcessor {
    // Returns the intermediate texture view to render emulator output into.
    fn intermediate_view(&self) -> &wgpu::TextureView;

    // Handle window/surface resize.
    fn resize(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    );

    // Update the content rect and source dimensions based on actual blit geometry.
    fn update_content_rect(&self, queue: &wgpu::Queue, rect: &ContentRect);

    // Update the time uniform for animation effects.
    fn update_time(&self, queue: &wgpu::Queue, time: f32);

    // Update the monochrome flag.
    fn update_monochrome(&self, queue: &wgpu::Queue, monochrome: bool);

    // Update text-only mode (disables NTSC color processing like real CRT auto-detect).
    fn update_text_mode(&self, queue: &wgpu::Queue, text_only: bool);

    // Update power-on elapsed time for CRT startup sync effect.
    fn update_power_on_time(&self, queue: &wgpu::Queue, elapsed_secs: f32);

    // Update shader-specific parameters.
    fn update_shader_params(&self, queue: &wgpu::Queue, params: &shader_ui::ShaderParams);

    // Toggle LCD invert.
    fn set_invert(&self, _queue: &wgpu::Queue, _invert: bool) {}

    fn set_scale_factor(&self, _factor: f32) {}

    // Clear the intermediate texture to black before rendering.
    fn clear_intermediate(&self, encoder: &mut wgpu::CommandEncoder);

    // Execute the post-processing render passes.
    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        render_target: &wgpu::TextureView,
        device: &wgpu::Device,
    );
}
