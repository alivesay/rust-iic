//! Post-processing screen renderer abstraction.
//!
//! Provides a common trait for CRT and LCD shader renderers with display-specific
//! implementations that handle different visual characteristics and aspect ratios.

use wgpu::util::DeviceExt;

/// Common interface for post-processing screen shaders.
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

    /// Update shader-specific parameters.
    fn update_shader_params(&self, queue: &wgpu::Queue, params: &shader_ui::ShaderParams);

    /// Execute the post-processing render passes.
    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        render_target: &wgpu::TextureView,
        device: &wgpu::Device,
    );
}

// ============================================================================
// LCD Renderer - Apple IIc Flat Panel Display
// ============================================================================

/// Apple IIc flat panel LCD renderer.
///
/// The Apple IIc LCD was a 9" passive-matrix STN display with:
/// - Vertically compressed aspect ratio (shorter than CRT equivalent)
/// - Visible pixel grid structure
/// - Green-tinted background with dark pixels
/// - Relatively slow response time (ghosting)
///
/// This renderer is simpler than CRT as it doesn't need bloom/halation passes.
pub struct LcdRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    // Intermediate texture for scaling renderer output
    intermediate_texture: wgpu::Texture,
    intermediate_view: wgpu::TextureView,
    intermediate_render_view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    tex_width: u32,
    tex_height: u32,
    // Shader params
    shader_params_buffer: wgpu::Buffer,
    surface_format: wgpu::TextureFormat,
    // LCD aspect ratio adjustment (vertical squish factor)
    // The Apple IIc LCD was approximately 15% shorter than CRT equivalent
    aspect_correction: f32,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct LcdUniforms {
    // Content rect in UV space: (left, top, right, bottom)
    content_rect: [f32; 4],
    // x = surface_w (unused), y = source_height, z = time, w = source_width
    params: [f32; 4],
    // x = monochrome, y = aspect_correction, z/w = reserved
    extra: [f32; 4],
}

impl LcdRenderer {
    /// LCD vertical aspect correction factor.
    /// The Apple IIc flat panel was significantly shorter than CRT equivalent.
    /// Contemporary reviews noted it "squishes 25 lines into a 16 line space"
    /// which gives us 16/25 = 0.64
    pub const LCD_ASPECT_CORRECTION: f32 = 0.64;

    pub fn new(
        device: &wgpu::Device,
        surface_width: u32,
        surface_height: u32,
        _buffer_width: u32,
        _buffer_height: u32,
        bar_height: u32,
        source_width: f32,
        source_height: f32,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/lcd.wgsl"));

        // Full-screen triangle
        let vertex_data: [[f32; 2]; 3] = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lcd_vertex_buffer"),
            contents: bytemuck::cast_slice(&vertex_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let uniforms = Self::compute_uniforms(
            surface_width,
            surface_height,
            bar_height,
            source_width,
            source_height,
            Self::LCD_ASPECT_CORRECTION,
        );
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lcd_uniform_buffer"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Use nearest-neighbor filtering for sharp LCD pixels
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lcd_sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Shader params buffer (matches CRT layout for UI compatibility)
        let shader_params = shader_ui::ShaderParams::default().to_gpu();
        let shader_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lcd_params_buffer"),
            contents: bytemuck::bytes_of(&shader_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Intermediate texture (no mipmaps needed for LCD)
        let (intermediate_texture, intermediate_view, intermediate_render_view) =
            Self::create_intermediate(device, surface_width, surface_height, surface_format);

        // Bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lcd_bind_group_layout"),
            entries: &[
                // @binding(0): intermediate texture
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                // @binding(1): sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // @binding(2): uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(3): blur texture (dummy for compatibility)
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                // @binding(4): shader params
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Create bind group
        let bind_group = Self::create_bind_group(
            device,
            &bind_group_layout,
            &intermediate_view,
            &sampler,
            &uniform_buffer,
            &shader_params_buffer,
        );

        // Pipeline
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lcd_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lcd_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    }],
                }],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
            vertex_buffer,
            uniform_buffer,
            sampler,
            intermediate_texture,
            intermediate_view,
            intermediate_render_view,
            bind_group,
            tex_width: surface_width,
            tex_height: surface_height,
            shader_params_buffer,
            surface_format,
            aspect_correction: Self::LCD_ASPECT_CORRECTION,
        }
    }

    fn compute_uniforms(
        _surface_w: u32,
        surface_h: u32,
        bar_h: u32,
        source_width: f32,
        source_height: f32,
        aspect_correction: f32,
    ) -> LcdUniforms {
        let sh = surface_h as f64;
        let bar_uv_y = if bar_h > 0 {
            (surface_h - bar_h) as f64 / sh
        } else {
            1.0
        };

        LcdUniforms {
            content_rect: [0.0, 0.0, 1.0, 1.0], // Will be updated per-frame
            params: [bar_uv_y as f32, source_height, 0.0, source_width],
            extra: [0.0, aspect_correction, 0.0, 0.0],
        }
    }

    fn create_intermediate(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> (wgpu::Texture, wgpu::TextureView, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lcd_intermediate_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING 
                 | wgpu::TextureUsages::RENDER_ATTACHMENT
                 | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let render_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view, render_view)
    }

    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        intermediate_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        uniform_buffer: &wgpu::Buffer,
        shader_params_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lcd_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(intermediate_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
                // binding 3: use intermediate as dummy blur texture (LCD doesn't use it)
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(intermediate_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: shader_params_buffer.as_entire_binding(),
                },
            ],
        })
    }
}

impl PostProcessor for LcdRenderer {
    fn intermediate_view(&self) -> &wgpu::TextureView {
        &self.intermediate_render_view
    }

    fn resize(
        &mut self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        if width == self.tex_width && height == self.tex_height {
            return;
        }
        self.tex_width = width;
        self.tex_height = height;

        let (tex, view, render_view) =
            Self::create_intermediate(device, width, height, self.surface_format);
        self.intermediate_texture = tex;
        self.intermediate_view = view;
        self.intermediate_render_view = render_view;

        self.bind_group = Self::create_bind_group(
            device,
            &self.bind_group_layout,
            &self.intermediate_view,
            &self.sampler,
            &self.uniform_buffer,
            &self.shader_params_buffer,
        );
    }

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
    ) {
        let sw = surface_w as f64;
        let sh = surface_h as f64;

        let left = offset_x as f64 / sw;
        let top = offset_y as f64 / sh;
        let right = (offset_x + dst_w) as f64 / sw;
        let bottom = (offset_y + dst_h) as f64 / sh;

        let bar_uv_y = if bar_h > 0 {
            (surface_h - bar_h) as f64 / sh
        } else {
            bottom
        };

        let uniforms = LcdUniforms {
            content_rect: [left as f32, top as f32, right as f32, bottom as f32],
            params: [bar_uv_y as f32, source_height, 0.0, source_width],
            extra: [0.0, self.aspect_correction, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    fn update_time(&self, queue: &wgpu::Queue, time: f32) {
        // params.z is at byte offset 16 (content_rect) + 8 = 24
        queue.write_buffer(&self.uniform_buffer, 24, bytemuck::bytes_of(&time));
    }

    fn update_monochrome(&self, queue: &wgpu::Queue, monochrome: bool) {
        // extra.x is at byte offset 16 (content_rect) + 16 (params) = 32
        let val: f32 = if monochrome { 1.0 } else { 0.0 };
        queue.write_buffer(&self.uniform_buffer, 32, bytemuck::bytes_of(&val));
    }

    fn update_shader_params(&self, queue: &wgpu::Queue, params: &shader_ui::ShaderParams) {
        let gpu = params.to_gpu();
        queue.write_buffer(&self.shader_params_buffer, 0, bytemuck::bytes_of(&gpu));
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        render_target: &wgpu::TextureView,
        _device: &wgpu::Device,
    ) {
        // LCD is a single pass - no bloom/mipmap generation needed
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lcd_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: render_target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.bind_group, &[]);
        rpass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        rpass.draw(0..3, 0..1);
    }
}
