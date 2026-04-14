use wgpu::util::DeviceExt;

/// Post-processing CRT shader renderer with multi-pass bloom.
///
/// Pass 1: scaling renderer → intermediate texture (full resolution)
/// Pass 2: intermediate → bloom texture (1/4 resolution, blurred)
/// Pass 3: CRT shader composites both → final surface
pub struct CrtRenderer {
    // CRT composite pass
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    // Intermediate texture (full resolution, scaling renderer output)
    intermediate_texture: wgpu::Texture,
    intermediate_view: wgpu::TextureView,
    intermediate_render_view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    tex_width: u32,
    tex_height: u32,
    // Separable Gaussian blur (replaces old single-pass bloom)
    gauss_pipeline: wgpu::RenderPipeline,
    gauss_bind_group_layout: wgpu::BindGroupLayout,
    gauss_vertex_buffer: wgpu::Buffer,
    gaussx_uniform_buffer: wgpu::Buffer,
    gaussy_uniform_buffer: wgpu::Buffer,
    // Gaussx intermediate texture (horizontal blur output)
    gaussx_texture: wgpu::Texture,
    gaussx_view: wgpu::TextureView,
    gaussx_bind_group: wgpu::BindGroup,  // reads intermediate → gaussx
    gaussy_bind_group: wgpu::BindGroup,  // reads gaussx → blur_texture
    // Final blur texture (vertical blur output, fed to CRT shader)
    blur_texture: wgpu::Texture,
    blur_view: wgpu::TextureView,
    // Mipmap generation (still needed for rasterbloom avgbright sampling)
    mipgen_pipeline: wgpu::RenderPipeline,
    mipgen_bind_group_layout: wgpu::BindGroupLayout,
    mipgen_vertex_buffer: wgpu::Buffer,
    mip_level_count: u32,
    // Shader params
    shader_params_buffer: wgpu::Buffer,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CrtUniforms {
    // Content rect in UV space: (left, top, right, bottom)
    content_rect: [f32; 4],
    // x = bar_uv_y (status bar boundary in UV), y = source_height, z = time (seconds), w = source_width
    params: [f32; 4],
    // x = monochrome (0.0 or 1.0), y/z/w = reserved
    extra: [f32; 4],
}

impl CrtRenderer {
    pub fn new(
        device: &wgpu::Device,
        surface_width: u32,
        surface_height: u32,
        buffer_width: u32,
        buffer_height: u32,
        bar_height: u32,
        source_width: f32,
        source_height: f32,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/crt.wgsl"));

        // Full-screen triangle
        let vertex_data: [[f32; 2]; 3] = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("crt_vertex_buffer"),
            contents: bytemuck::cast_slice(&vertex_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let uniforms = Self::compute_uniforms(
            surface_width, surface_height,
            buffer_width, buffer_height,
            bar_height, source_width, source_height,
        );
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("crt_uniform_buffer"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("crt_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // --- Mipmap generation pipeline ---
        let mipgen_shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/mipgen.wgsl"));

        let mipgen_vertex_data: [[f32; 2]; 3] = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
        let mipgen_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mipgen_vertex_buffer"),
            contents: bytemuck::cast_slice(&mipgen_vertex_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let mipgen_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mipgen_bind_group_layout"),
            entries: &[
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let mipgen_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mipgen_pipeline_layout"),
            bind_group_layouts: &[&mipgen_bind_group_layout],
            push_constant_ranges: &[],
        });

        let mipgen_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mipgen_pipeline"),
            layout: Some(&mipgen_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &mipgen_shader,
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
                module: &mipgen_shader,
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

        // --- Separable Gaussian blur pipeline (shared by X and Y passes) ---
        let gauss_shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/gauss.wgsl"));

        let gauss_vertex_data: [[f32; 2]; 3] = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
        let gauss_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gauss_vertex_buffer"),
            contents: bytemuck::cast_slice(&gauss_vertex_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let gauss_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gauss_bind_group_layout"),
            entries: &[
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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

        let gauss_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gauss_pipeline_layout"),
            bind_group_layouts: &[&gauss_bind_group_layout],
            push_constant_ranges: &[],
        });

        let gauss_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gauss_pipeline"),
            layout: Some(&gauss_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &gauss_shader,
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
                module: &gauss_shader,
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

        // Gauss textures at 1/4 surface resolution for proper blur spread
        let gauss_w = (surface_width / 4).max(1);
        let gauss_h = (surface_height / 4).max(1);

        // Gauss uniform buffers (direction + blur_width + source_size)
        let default_blur_width = shader_ui::ShaderParams::default().blur_width;
        let gaussx_uniforms: [f32; 4] = [1.0, 0.0, default_blur_width, gauss_w as f32];
        let gaussx_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gaussx_uniform_buffer"),
            contents: bytemuck::cast_slice(&gaussx_uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let gaussy_uniforms: [f32; 4] = [0.0, 1.0, default_blur_width, gauss_h as f32];
        let gaussy_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gaussy_uniform_buffer"),
            contents: bytemuck::cast_slice(&gaussy_uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Gaussx intermediate texture (1/4 resolution, no mipmaps)
        let (gaussx_texture, gaussx_view, _) = Self::create_intermediate(
            device, gauss_w, gauss_h, surface_format, 1,
        );
        // Final blur texture (gaussy output, 1/4 resolution, no mipmaps)
        let (blur_texture, blur_view, _) = Self::create_intermediate(
            device, gauss_w, gauss_h, surface_format, 1,
        );

        // --- CRT composite pass ---
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("crt_bind_group_layout"),
            entries: &[
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("crt_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("crt_pipeline"),
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

        // Create intermediate texture at surface size with mipmaps
        let mip_level_count = Self::mip_levels(surface_width, surface_height);
        let (intermediate_texture, intermediate_view, intermediate_render_view) =
            Self::create_intermediate(device, surface_width, surface_height, surface_format, mip_level_count);

        // Shader params buffer
        let default_params = shader_ui::ShaderParams::default().to_gpu();
        let shader_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shader_params_buffer"),
            contents: bytemuck::bytes_of(&default_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Gauss bind groups: gaussx reads intermediate, gaussy reads gaussx
        let gaussx_bind_group = Self::create_gauss_bind_group(
            device, &gauss_bind_group_layout, &intermediate_view, &sampler, &gaussx_uniform_buffer,
        );
        let gaussy_bind_group = Self::create_gauss_bind_group(
            device, &gauss_bind_group_layout, &gaussx_view, &sampler, &gaussy_uniform_buffer,
        );

        let bind_group = Self::create_bind_group(
            device,
            &bind_group_layout,
            &intermediate_view,
            &sampler,
            &uniform_buffer,
            &blur_view,
            &shader_params_buffer,
        );

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
            gauss_pipeline,
            gauss_bind_group_layout,
            gauss_vertex_buffer,
            gaussx_uniform_buffer,
            gaussy_uniform_buffer,
            gaussx_texture,
            gaussx_view,
            gaussx_bind_group,
            gaussy_bind_group,
            blur_texture,
            blur_view,
            mipgen_pipeline,
            mipgen_bind_group_layout,
            mipgen_vertex_buffer,
            mip_level_count,
            shader_params_buffer,
        }
    }

    fn mip_levels(width: u32, height: u32) -> u32 {
        (width.max(height) as f32).log2().floor() as u32 + 1
    }

    fn create_intermediate(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        mip_level_count: u32,
    ) -> (wgpu::Texture, wgpu::TextureView, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("crt_intermediate_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        // Full view (all mip levels) for sampling
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Mip-0-only view for use as render target
        let render_view = texture.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: 0,
            mip_level_count: Some(1),
            ..Default::default()
        });
        (texture, view, render_view)
    }

    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        texture_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        uniform_buffer: &wgpu::Buffer,
        bloom_view: &wgpu::TextureView,
        shader_params_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("crt_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(bloom_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: shader_params_buffer.as_entire_binding(),
                },
            ],
        })
    }

    fn create_gauss_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        input_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        uniform_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gauss_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(input_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        })
    }

    /// Call when the surface is resized to recreate the intermediate texture.
    /// Uniforms (content_rect) are now updated per-frame via update_content_rect.
    pub fn resize(
        &mut self, device: &wgpu::Device, queue: &wgpu::Queue,
        width: u32, height: u32,
    ) {
        if width == self.tex_width && height == self.tex_height {
            return;
        }
        self.tex_width = width;
        self.tex_height = height;

        let format = self.intermediate_texture.format();
        let mip_level_count = Self::mip_levels(width, height);
        self.mip_level_count = mip_level_count;
        let (tex, view, render_view) = Self::create_intermediate(device, width, height, format, mip_level_count);
        self.intermediate_texture = tex;
        self.intermediate_view = view;
        self.intermediate_render_view = render_view;

        // Recreate gaussx intermediate texture (1/4 resolution, no mipmaps)
        let gauss_w = (width / 4).max(1);
        let gauss_h = (height / 4).max(1);
        let (gaussx_tex, gaussx_v, _) = Self::create_intermediate(
            device, gauss_w, gauss_h, format, 1,
        );
        self.gaussx_texture = gaussx_tex;
        self.gaussx_view = gaussx_v;

        // Recreate blur texture (gaussy output, 1/4 resolution, no mipmaps)
        let (blur_tex, blur_v, _) = Self::create_intermediate(
            device, gauss_w, gauss_h, format, 1,
        );
        self.blur_texture = blur_tex;
        self.blur_view = blur_v;

        // Recreate gauss bind groups
        self.gaussx_bind_group = Self::create_gauss_bind_group(
            device, &self.gauss_bind_group_layout, &self.intermediate_view, &self.sampler, &self.gaussx_uniform_buffer,
        );
        self.gaussy_bind_group = Self::create_gauss_bind_group(
            device, &self.gauss_bind_group_layout, &self.gaussx_view, &self.sampler, &self.gaussy_uniform_buffer,
        );

        // Update source_size in gauss uniforms to match reduced texture dimensions
        let gw_f32 = gauss_w as f32;
        let gh_f32 = gauss_h as f32;
        queue.write_buffer(&self.gaussx_uniform_buffer, 12, bytemuck::bytes_of(&gw_f32));
        queue.write_buffer(&self.gaussy_uniform_buffer, 12, bytemuck::bytes_of(&gh_f32));

        // Recreate CRT bind group (reads from intermediate + blur)
        self.bind_group = Self::create_bind_group(
            device,
            &self.bind_group_layout,
            &self.intermediate_view,
            &self.sampler,
            &self.uniform_buffer,
            &self.blur_view,
            &self.shader_params_buffer,
        );
    }

    /// Update the time uniform for flicker effects. Call once per frame.
    pub fn update_time(&self, queue: &wgpu::Queue, time: f32) {
        // params.z is at byte offset 16 (content_rect) + 8 (params.x, params.y) = 24
        queue.write_buffer(&self.uniform_buffer, 24, bytemuck::bytes_of(&time));
    }

    /// Update all tunable shader parameters. Call once per frame.
    pub fn update_shader_params(&self, queue: &wgpu::Queue, params: &shader_ui::ShaderParams) {
        let gpu = params.to_gpu();
        queue.write_buffer(&self.shader_params_buffer, 0, bytemuck::bytes_of(&gpu));
        // Update blur_width in gauss uniform buffers (offset 8 = third float)
        queue.write_buffer(&self.gaussx_uniform_buffer, 8, bytemuck::bytes_of(&params.blur_width));
        queue.write_buffer(&self.gaussy_uniform_buffer, 8, bytemuck::bytes_of(&params.blur_width));
    }

    /// Update the content rect and source dimensions based on actual blit geometry.
    /// Call each frame with the real position/size of the emulator content in the surface.
    pub fn update_content_rect(
        &self, queue: &wgpu::Queue,
        surface_w: u32, surface_h: u32,
        offset_x: u32, offset_y: u32,
        dst_w: u32, dst_h: u32,
        bar_h: u32,
        source_width: f32, source_height: f32,
    ) {
        let sw = surface_w as f64;
        let sh = surface_h as f64;

        let left = offset_x as f64 / sw;
        let top = offset_y as f64 / sh;
        let right = (offset_x + dst_w) as f64 / sw;
        let bottom = (offset_y + dst_h) as f64 / sh;

        // bar_uv_y: if toolbar visible, its top edge in UV space
        // The bar sits below the blit area in the surface
        let bar_uv_y = if bar_h > 0 {
            (surface_h - bar_h) as f64 / sh
        } else {
            bottom // no bar = bar boundary at bottom of content
        };

        let uniforms = CrtUniforms {
            content_rect: [left as f32, top as f32, right as f32, bottom as f32],
            params: [bar_uv_y as f32, source_height, 0.0, source_width],
            extra: [0.0, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    /// Update the monochrome flag. Call when mode changes or once per frame.
    pub fn update_monochrome(&self, queue: &wgpu::Queue, monochrome: bool) {
        // extra.x is at byte offset 16 (content_rect) + 16 (params) = 32
        let val: f32 = if monochrome { 1.0 } else { 0.0 };
        queue.write_buffer(&self.uniform_buffer, 32, bytemuck::bytes_of(&val));
    }

    /// Compute the content rect and bar boundary in intermediate texture UV space.
    /// The scaling renderer maps the pixel buffer to the surface with aspect-ratio
    /// preservation, creating pillarbox/letterbox bars.  We need to know exactly
    /// where the content sits so the shader can separate emulator from status bar.
    fn compute_uniforms(
        surface_w: u32, surface_h: u32,
        buffer_w: u32, buffer_h: u32,
        bar_h: u32, source_width: f32, source_height: f32,
    ) -> CrtUniforms {
        let sw = surface_w as f64;
        let sh = surface_h as f64;
        let bw = buffer_w as f64;
        let bh = buffer_h as f64;

        let scale = (sw / bw).min(sh / bh);
        let content_w = bw * scale;
        let content_h = bh * scale;
        let offset_x = (sw - content_w) / 2.0;
        let offset_y = (sh - content_h) / 2.0;

        // Content rect in UV [0,1] space
        let left = offset_x / sw;
        let top = offset_y / sh;
        let right = (offset_x + content_w) / sw;
        let bottom = (offset_y + content_h) / sh;

        // Status bar boundary: the emulator occupies (buffer_h - bar_h) / buffer_h
        // of the content height
        let emu_frac_of_content = (bh - bar_h as f64) / bh;
        let bar_uv_y = top + emu_frac_of_content * (bottom - top);

        CrtUniforms {
            content_rect: [left as f32, top as f32, right as f32, bottom as f32],
            params: [bar_uv_y as f32, source_height, 0.0, source_width],
            extra: [0.0, 0.0, 0.0, 0.0],
        }
    }

    /// Get a reference to the intermediate texture view.
    /// The scaling renderer should render into this instead of the final surface.
    pub fn intermediate_view(&self) -> &wgpu::TextureView {
        &self.intermediate_render_view
    }

    /// Render the CRT post-processing passes:
    /// 1. Bloom: blur intermediate → bloom texture (1/4 res)
    /// 2. Composite: CRT shader with intermediate + bloom → final surface
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder, render_target: &wgpu::TextureView, device: &wgpu::Device) {
        // Pass 0: Generate mip chain for the intermediate texture
        if self.mip_level_count > 1 {
            let mut mip_w = self.tex_width;
            let mut mip_h = self.tex_height;
            for level in 1..self.mip_level_count {
                let src_view = self.intermediate_texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: level - 1,
                    mip_level_count: Some(1),
                    ..Default::default()
                });
                let dst_view = self.intermediate_texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                });
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("mipgen_bind_group"),
                    layout: &self.mipgen_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&src_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });

                mip_w = (mip_w / 2).max(1);
                mip_h = (mip_h / 2).max(1);

                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("mipgen_render_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &dst_view,
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
                rpass.set_pipeline(&self.mipgen_pipeline);
                rpass.set_bind_group(0, &bind_group, &[]);
                rpass.set_vertex_buffer(0, self.mipgen_vertex_buffer.slice(..));
                rpass.draw(0..3, 0..1);
            }
        }

        // Pass 1: Gaussian horizontal blur (intermediate → gaussx_texture)
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gaussx_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.gaussx_view,
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
            rpass.set_pipeline(&self.gauss_pipeline);
            rpass.set_bind_group(0, &self.gaussx_bind_group, &[]);
            rpass.set_vertex_buffer(0, self.gauss_vertex_buffer.slice(..));
            rpass.draw(0..3, 0..1);
        }

        // Pass 2: Gaussian vertical blur (gaussx_texture → blur_texture)
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gaussy_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.blur_view,
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
            rpass.set_pipeline(&self.gauss_pipeline);
            rpass.set_bind_group(0, &self.gaussy_bind_group, &[]);
            rpass.set_vertex_buffer(0, self.gauss_vertex_buffer.slice(..));
            rpass.draw(0..3, 0..1);
        }

        // Pass 3: CRT-Geom-Deluxe composite (intermediate + blur → screen)
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("crt_render_pass"),
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
}
