pub trait PostProcessor {
    /// Returns the intermediate texture view to render emulator output into.
    fn intermediate_view(&self) -> &wgpu::TextureView;

    /// Handle window/surface resize.
    fn resize(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    );

    /// Update the content rect and source dimensions based on actual blit geometry.
    fn update_content_rect(
        &self,
        queue: &wgpu::Queue,
        surface_w: u32,
        surface_h: u32,
        offset_x: u32,
        offset_y: u32,
        dst_w: u32,
        dst_h: u32,
        bar_h: u32,
        source_width: f32,
        source_height: f32,
    );

    /// Update the time uniform for animation effects.
    fn update_time(&self, queue: &wgpu::Queue, time: f32);

    /// Update the monochrome flag.
    fn update_monochrome(&self, queue: &wgpu::Queue, monochrome: bool);

    /// Update text-only mode (disables NTSC color processing like real CRT auto-detect).
    fn update_text_mode(&self, queue: &wgpu::Queue, text_only: bool);

    /// Update power-on elapsed time for CRT startup sync effect.
    fn update_power_on_time(&self, queue: &wgpu::Queue, elapsed_secs: f32);

    /// Update shader-specific parameters.
    fn update_shader_params(&self, queue: &wgpu::Queue, params: &shader_ui::ShaderParams);

    /// Clear the intermediate texture to black before rendering.
    fn clear_intermediate(&self, encoder: &mut wgpu::CommandEncoder);

    /// Execute the post-processing render passes.
    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        render_target: &wgpu::TextureView,
        device: &wgpu::Device,
    );
}
