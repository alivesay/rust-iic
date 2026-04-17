use wgpu::util::DeviceExt;

use super::screen::PostProcessor;

/// Post-processing CRT shader renderer with multi-pass bloom.
///
/// CRT mode uses multi-pass bloom:
///   Pass 1: scaling renderer → intermediate texture (full resolution)
///   Pass 2: intermediate → bloom texture (1/4 resolution, blurred)
///   Pass 3: CRT shader composites both → final surface
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
    // Separable Gaussian blur (halation - 1/2 res)
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
    // Large glow (1/8 res for fullscreen CRT glow)
    glowx_uniform_buffer: wgpu::Buffer,
    glowy_uniform_buffer: wgpu::Buffer,
    glowx_texture: wgpu::Texture,
    glowx_view: wgpu::TextureView,
    glowx_bind_group: wgpu::BindGroup,
    glowy_bind_group: wgpu::BindGroup,
    glow_texture: wgpu::Texture,
    glow_view: wgpu::TextureView,
    // Mipmap generation (for rasterbloom avgbright sampling)
    mipgen_pipeline: wgpu::RenderPipeline,
    mipgen_bind_group_layout: wgpu::BindGroupLayout,
    mipgen_vertex_buffer: wgpu::Buffer,
    mip_level_count: u32,
    // Shader params
    shader_params_buffer: wgpu::Buffer,
    // Surface format for resize (currently using texture.format() instead)
    #[allow(dead_code)]
    surface_format: wgpu::TextureFormat,
    // Phosphor persistence (ping-pong history buffers)
    phosphor_pipeline: wgpu::RenderPipeline,
    phosphor_bind_group_layout: wgpu::BindGroupLayout,
    phosphor_vertex_buffer: wgpu::Buffer,
    phosphor_uniform_buffer: wgpu::Buffer,
    phosphor_history_a: wgpu::Texture,
    phosphor_history_a_view: wgpu::TextureView,
    phosphor_history_b: wgpu::Texture,
    phosphor_history_b_view: wgpu::TextureView,
    phosphor_bind_group_a: wgpu::BindGroup,  // reads A, writes B
    phosphor_bind_group_b: wgpu::BindGroup,  // reads B, writes A
    phosphor_frame_idx: std::cell::Cell<u32>,  // toggles 0/1 for ping-pong
    clear_frames_remaining: std::cell::Cell<u32>,  // Clear textures for this many frames on startup/resize
    // Chromatic aberration post-processing pass
    chroma_pipeline: wgpu::RenderPipeline,
    chroma_bind_group_layout: wgpu::BindGroupLayout,
    chroma_vertex_buffer: wgpu::Buffer,
    chroma_uniform_buffer: wgpu::Buffer,
    chroma_texture: wgpu::Texture,  // CRT output before chromatic aberration
    chroma_texture_view: wgpu::TextureView,
    chroma_bind_group: wgpu::BindGroup,
    // Cached values for chroma uniform updates
    chroma_amount: std::cell::Cell<f32>,
    is_mono: std::cell::Cell<f32>,
    // Cached for glow curvature alignment
    content_rect_cache: std::cell::Cell<[f32; 4]>,
    curvature_cache: std::cell::Cell<[f32; 4]>,  // d, R, overscan_x/100, overscan_y/100
    glow_amt_cache: std::cell::Cell<f32>,
    curvature_on_cache: std::cell::Cell<f32>,
    source_width_cache: std::cell::Cell<f32>,
    source_height_cache: std::cell::Cell<f32>,
    // NTSC notch filter pass
    ntsc_pipeline: wgpu::RenderPipeline,
    ntsc_bind_group_layout: wgpu::BindGroupLayout,
    ntsc_bind_group: wgpu::BindGroup,
    ntsc_uniform_buffer: wgpu::Buffer,
    ntsc_texture: wgpu::Texture,
    ntsc_view: wgpu::TextureView,
    ntsc_strength: std::cell::Cell<f32>,
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

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ChromaUniforms {
    // x = chromatic_aberration amount (0-1), y = is_mono (0 or 1), z = glow_amt, w = curvature_on
    params: [f32; 4],
    // content_rect: left, top, right, bottom (normalized screen coords)
    content_rect: [f32; 4],
    // x = d (distance), y = R (radius), z = overscan_x/100, w = overscan_y/100
    curvature: [f32; 4],
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
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/crt.wgsl"));

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
        let mipgen_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/mipgen.wgsl"));

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
        let gauss_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/gauss.wgsl"));

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

        // Gauss textures at 1/2 surface resolution for smoother blur
        let gauss_w = (surface_width / 2).max(1);
        let gauss_h = (surface_height / 2).max(1);

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

        // Glow textures at 1/8 surface resolution for larger fullscreen glow
        let glow_w = (surface_width / 8).max(1);
        let glow_h = (surface_height / 8).max(1);

        // Glow uniform buffers (direction + blur_width + source_size)
        let default_glow_width = shader_ui::ShaderParams::default().glow_width;
        let glowx_uniforms: [f32; 4] = [1.0, 0.0, default_glow_width, glow_w as f32];
        let glowx_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("glowx_uniform_buffer"),
            contents: bytemuck::cast_slice(&glowx_uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let glowy_uniforms: [f32; 4] = [0.0, 1.0, default_glow_width, glow_h as f32];
        let glowy_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("glowy_uniform_buffer"),
            contents: bytemuck::cast_slice(&glowy_uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Glowx intermediate texture (1/8 resolution, no mipmaps)
        let (glowx_texture, glowx_view, _) = Self::create_intermediate(
            device, glow_w, glow_h, surface_format, 1,
        );
        // Final glow texture (glowy output, 1/8 resolution, no mipmaps)
        let (glow_texture, glow_view, _) = Self::create_intermediate(
            device, glow_w, glow_h, surface_format, 1,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
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

        // Glow bind groups: glowx reads blur_texture, glowy reads glowx
        let glowx_bind_group = Self::create_gauss_bind_group(
            device, &gauss_bind_group_layout, &blur_view, &sampler, &glowx_uniform_buffer,
        );
        let glowy_bind_group = Self::create_gauss_bind_group(
            device, &gauss_bind_group_layout, &glowx_view, &sampler, &glowy_uniform_buffer,
        );

        let bind_group = Self::create_bind_group(
            device,
            &bind_group_layout,
            &intermediate_view,
            &sampler,
            &uniform_buffer,
            &blur_view,
            &shader_params_buffer,
            &glow_view,
        );

        // --- Phosphor persistence pipeline ---
        let phosphor_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/phosphor.wgsl"));

        let phosphor_vertex_data: [[f32; 2]; 3] = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
        let phosphor_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("phosphor_vertex_buffer"),
            contents: bytemuck::cast_slice(&phosphor_vertex_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Phosphor uniforms: decay factor
        let phosphor_uniforms: [f32; 4] = [0.0, 0.0, 0.0, 0.0];  // decay=0 means disabled by default
        let phosphor_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("phosphor_uniform_buffer"),
            contents: bytemuck::cast_slice(&phosphor_uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let phosphor_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("phosphor_bind_group_layout"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
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

        let phosphor_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("phosphor_pipeline_layout"),
            bind_group_layouts: &[&phosphor_bind_group_layout],
            push_constant_ranges: &[],
        });

        let phosphor_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("phosphor_pipeline"),
            layout: Some(&phosphor_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &phosphor_shader,
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
                module: &phosphor_shader,
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

        // Phosphor history textures (same size as intermediate, no mipmaps)
        let (phosphor_history_a, phosphor_history_a_view, _) =
            Self::create_intermediate(device, surface_width, surface_height, surface_format, 1);
        let (phosphor_history_b, phosphor_history_b_view, _) =
            Self::create_intermediate(device, surface_width, surface_height, surface_format, 1);

        // Phosphor bind groups for ping-pong
        // A: reads intermediate + history_a, writes to history_b
        let phosphor_bind_group_a = Self::create_phosphor_bind_group(
            device, &phosphor_bind_group_layout, &intermediate_view, &phosphor_history_a_view,
            &sampler, &phosphor_uniform_buffer,
        );
        // B: reads intermediate + history_b, writes to history_a
        let phosphor_bind_group_b = Self::create_phosphor_bind_group(
            device, &phosphor_bind_group_layout, &intermediate_view, &phosphor_history_b_view,
            &sampler, &phosphor_uniform_buffer,
        );

        // --- Chromatic aberration post-processing pipeline ---
        let chroma_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/chromatic.wgsl"));

        let chroma_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("chroma_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let chroma_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("chroma_pipeline_layout"),
            bind_group_layouts: &[&chroma_bind_group_layout],
            push_constant_ranges: &[],
        });

        let chroma_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("chroma_render_pipeline"),
            layout: Some(&chroma_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &chroma_shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        offset: 0,
                        shader_location: 0,
                        format: wgpu::VertexFormat::Float32x2,
                    }],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &chroma_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let chroma_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chroma_vertex_buffer"),
            contents: bytemuck::cast_slice(&vertex_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let chroma_uniforms = ChromaUniforms {
            params: [0.0, 0.0, 0.0, 1.0],  // chroma_amt=0, is_mono=0, glow=0, curv_on=1
            content_rect: [0.0, 0.0, 1.0, 1.0],
            curvature: [3.0, 1.3, 1.0, 1.0],  // default d, R, overscan
        };
        let chroma_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chroma_uniform_buffer"),
            contents: bytemuck::bytes_of(&chroma_uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Intermediate texture for CRT output (before chromatic aberration)
        let chroma_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("chroma_intermediate_texture"),
            size: wgpu::Extent3d {
                width: surface_width,
                height: surface_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: surface_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let chroma_texture_view = chroma_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let chroma_bind_group = Self::create_chroma_bind_group(
            device, &chroma_bind_group_layout, &chroma_texture_view, &sampler, &chroma_uniform_buffer, &glow_view,
        );

        // --- NTSC notch filter pass ---
        let ntsc_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/ntsc.wgsl"));

        let ntsc_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ntsc_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
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

        let ntsc_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ntsc_pipeline_layout"),
            bind_group_layouts: &[&ntsc_bind_group_layout],
            push_constant_ranges: &[],
        });

        let ntsc_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ntsc_render_pipeline"),
            layout: Some(&ntsc_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &ntsc_shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &ntsc_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // NTSC uniforms: params (filter_strength, source_width, source_height, is_mono) + content_rect
        let ntsc_uniforms: [f32; 8] = [0.0, source_width, source_height, 0.0, 0.0, 0.0, 1.0, 1.0];
        let ntsc_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ntsc_uniform_buffer"),
            contents: bytemuck::cast_slice(&ntsc_uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // NTSC output texture (same size as intermediate)
        let ntsc_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ntsc_texture"),
            size: wgpu::Extent3d {
                width: surface_width,
                height: surface_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: surface_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let ntsc_view = ntsc_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let ntsc_bind_group = Self::create_ntsc_bind_group(
            device, &ntsc_bind_group_layout, &intermediate_view, &sampler, &ntsc_uniform_buffer,
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
            glowx_uniform_buffer,
            glowy_uniform_buffer,
            glowx_texture,
            glowx_view,
            glowx_bind_group,
            glowy_bind_group,
            glow_texture,
            glow_view,
            mipgen_pipeline,
            mipgen_bind_group_layout,
            mipgen_vertex_buffer,
            mip_level_count,
            shader_params_buffer,
            surface_format,
            phosphor_pipeline,
            phosphor_bind_group_layout,
            phosphor_vertex_buffer,
            phosphor_uniform_buffer,
            phosphor_history_a,
            phosphor_history_a_view,
            phosphor_history_b,
            phosphor_history_b_view,
            phosphor_bind_group_a,
            phosphor_bind_group_b,
            phosphor_frame_idx: std::cell::Cell::new(0),
            clear_frames_remaining: std::cell::Cell::new(10),  // Clear for multiple frames
            chroma_pipeline,
            chroma_bind_group_layout,
            chroma_vertex_buffer,
            chroma_uniform_buffer,
            chroma_texture,
            chroma_texture_view,
            chroma_bind_group,
            chroma_amount: std::cell::Cell::new(0.0),
            is_mono: std::cell::Cell::new(0.0),
            content_rect_cache: std::cell::Cell::new([0.0, 0.0, 1.0, 1.0]),
            curvature_cache: std::cell::Cell::new([3.0, 1.3, 1.0, 1.0]),
            glow_amt_cache: std::cell::Cell::new(0.0),
            curvature_on_cache: std::cell::Cell::new(1.0),
            source_width_cache: std::cell::Cell::new(560.0),
            source_height_cache: std::cell::Cell::new(384.0),
            ntsc_pipeline,
            ntsc_bind_group_layout,
            ntsc_bind_group,
            ntsc_uniform_buffer,
            ntsc_texture,
            ntsc_view,
            ntsc_strength: std::cell::Cell::new(0.0),
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT 
                 | wgpu::TextureUsages::TEXTURE_BINDING
                 | wgpu::TextureUsages::COPY_DST
                 | wgpu::TextureUsages::COPY_SRC,
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
        glow_view: &wgpu::TextureView,
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
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(glow_view),
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

    fn create_phosphor_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        current_view: &wgpu::TextureView,
        history_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        uniform_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("phosphor_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(current_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(history_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        })
    }

    fn create_chroma_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        texture_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        uniform_buffer: &wgpu::Buffer,
        glow_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("chroma_bind_group"),
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
                    resource: wgpu::BindingResource::TextureView(glow_view),
                },
            ],
        })
    }

    fn create_ntsc_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        texture_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        uniform_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ntsc_bind_group"),
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

        // Recreate gaussx intermediate texture (1/2 resolution, no mipmaps)
        let gauss_w = (width / 2).max(1);
        let gauss_h = (height / 2).max(1);
        let (gaussx_tex, gaussx_v, _) = Self::create_intermediate(
            device, gauss_w, gauss_h, format, 1,
        );
        self.gaussx_texture = gaussx_tex;
        self.gaussx_view = gaussx_v;

        // Recreate blur texture (gaussy output, 1/2 resolution, no mipmaps)
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

        // Recreate glow textures (1/8 resolution for fullscreen glow)
        let glow_w = (width / 8).max(1);
        let glow_h = (height / 8).max(1);
        let (glowx_tex, glowx_v, _) = Self::create_intermediate(
            device, glow_w, glow_h, format, 1,
        );
        self.glowx_texture = glowx_tex;
        self.glowx_view = glowx_v;

        let (glow_tex, glow_v, _) = Self::create_intermediate(
            device, glow_w, glow_h, format, 1,
        );
        self.glow_texture = glow_tex;
        self.glow_view = glow_v;

        // Recreate glow bind groups
        self.glowx_bind_group = Self::create_gauss_bind_group(
            device, &self.gauss_bind_group_layout, &self.blur_view, &self.sampler, &self.glowx_uniform_buffer,
        );
        self.glowy_bind_group = Self::create_gauss_bind_group(
            device, &self.gauss_bind_group_layout, &self.glowx_view, &self.sampler, &self.glowy_uniform_buffer,
        );

        // Update source_size in glow uniforms
        let glow_w_f32 = glow_w as f32;
        let glow_h_f32 = glow_h as f32;
        queue.write_buffer(&self.glowx_uniform_buffer, 12, bytemuck::bytes_of(&glow_w_f32));
        queue.write_buffer(&self.glowy_uniform_buffer, 12, bytemuck::bytes_of(&glow_h_f32));

        // Recreate CRT bind group (reads from intermediate + blur + glow)
        self.bind_group = Self::create_bind_group(
            device,
            &self.bind_group_layout,
            &self.intermediate_view,
            &self.sampler,
            &self.uniform_buffer,
            &self.blur_view,
            &self.shader_params_buffer,
            &self.glow_view,
        );

        // Recreate phosphor history textures at new resolution
        let (hist_a, hist_a_view, _) = Self::create_intermediate(device, width, height, format, 1);
        let (hist_b, hist_b_view, _) = Self::create_intermediate(device, width, height, format, 1);
        self.phosphor_history_a = hist_a;
        self.phosphor_history_a_view = hist_a_view;
        self.phosphor_history_b = hist_b;
        self.phosphor_history_b_view = hist_b_view;

        // Recreate phosphor bind groups
        self.phosphor_bind_group_a = Self::create_phosphor_bind_group(
            device, &self.phosphor_bind_group_layout,
            &self.intermediate_view, &self.phosphor_history_a_view,
            &self.sampler, &self.phosphor_uniform_buffer,
        );
        self.phosphor_bind_group_b = Self::create_phosphor_bind_group(
            device, &self.phosphor_bind_group_layout,
            &self.intermediate_view, &self.phosphor_history_b_view,
            &self.sampler, &self.phosphor_uniform_buffer,
        );
        self.phosphor_frame_idx.set(0);
        self.clear_frames_remaining.set(10);  // Clear for multiple frames

        // Recreate chroma intermediate texture at new resolution
        let chroma_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("chroma_intermediate_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.chroma_texture_view = chroma_tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.chroma_texture = chroma_tex;

        // Recreate chroma bind group
        self.chroma_bind_group = Self::create_chroma_bind_group(
            device, &self.chroma_bind_group_layout, &self.chroma_texture_view,
            &self.sampler, &self.chroma_uniform_buffer, &self.glow_view,
        );

        // Recreate NTSC filter texture at new resolution
        let ntsc_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ntsc_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.ntsc_view = ntsc_tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.ntsc_texture = ntsc_tex;

        // Recreate NTSC bind group
        self.ntsc_bind_group = Self::create_ntsc_bind_group(
            device, &self.ntsc_bind_group_layout, &self.intermediate_view,
            &self.sampler, &self.ntsc_uniform_buffer,
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
        // Update glow blur width (separate control from halation)
        queue.write_buffer(&self.glowx_uniform_buffer, 8, bytemuck::bytes_of(&params.glow_width));
        queue.write_buffer(&self.glowy_uniform_buffer, 8, bytemuck::bytes_of(&params.glow_width));
        // Update phosphor decay (offset 0 = first float)
        queue.write_buffer(&self.phosphor_uniform_buffer, 0, bytemuck::bytes_of(&params.phosphor));
        // Update chromatic aberration + glow + curvature for chroma pass
        self.chroma_amount.set(params.chromatic_aberration);
        self.glow_amt_cache.set(params.glow);
        self.curvature_on_cache.set(params.curvature);
        self.curvature_cache.set([
            params.distance,
            params.radius,
            params.overscan_x / 100.0,
            params.overscan_y / 100.0,
        ]);
        let chroma_uniforms = ChromaUniforms {
            params: [params.chromatic_aberration, self.is_mono.get(), params.glow, params.curvature],
            content_rect: self.content_rect_cache.get(),
            curvature: self.curvature_cache.get(),
        };
        queue.write_buffer(&self.chroma_uniform_buffer, 0, bytemuck::bytes_of(&chroma_uniforms));
        // Update NTSC filter uniforms: params + content_rect
        self.ntsc_strength.set(params.ntsc_filter);
        let content_rect = self.content_rect_cache.get();
        let ntsc_uniforms: [f32; 8] = [
            params.ntsc_filter,
            self.source_width_cache.get(),
            self.source_height_cache.get(),
            self.is_mono.get(),
            content_rect[0], content_rect[1], content_rect[2], content_rect[3],
        ];
        queue.write_buffer(&self.ntsc_uniform_buffer, 0, bytemuck::bytes_of(&ntsc_uniforms));
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

        // Cache content_rect for chroma pass (glow curvature alignment)
        self.content_rect_cache.set([left as f32, top as f32, right as f32, bottom as f32]);
        // Cache source dimensions for NTSC pass
        self.source_width_cache.set(source_width);
        self.source_height_cache.set(source_height);

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
        // Update is_mono in chroma uniforms cache
        self.is_mono.set(val);
        // Note: chroma_uniform_buffer is updated in update_shader_params which should be called per-frame
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

    /// Clear the intermediate texture to black.
    /// Call this before the scaling renderer writes to prevent ghosting artifacts.
    pub fn clear_intermediate(&self, encoder: &mut wgpu::CommandEncoder) {
        let _rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("intermediate_clear_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.intermediate_render_view,
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
        // Pass drops immediately, executing the clear
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

        // Phosphor persistence (blend current frame with decayed history)
        // Clear history textures ONCE after resize to remove stale GPU memory,
        // then immediately resume normal blending so history can rebuild
        {
            let clear_remaining = self.clear_frames_remaining.get();
            if clear_remaining > 0 {
                self.clear_frames_remaining.set(clear_remaining - 1);
                // Clear both history textures to black (removes garbage from old GPU memory)
                {
                    let _rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("clear_phosphor_a"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.phosphor_history_a_view,
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
                }
                {
                    let _rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("clear_phosphor_b"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.phosphor_history_b_view,
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
                }
                // Fall through to normal blending - history is now clean black,
                // so the first blend will just copy current frame to history
            }
            
            // Normal phosphor blending (runs every frame, even after clear)
            let frame_idx = self.phosphor_frame_idx.get();
            let (bind_group, dst_view, dst_texture) = if frame_idx == 0 {
                (&self.phosphor_bind_group_a, &self.phosphor_history_b_view, &self.phosphor_history_b)
            } else {
                (&self.phosphor_bind_group_b, &self.phosphor_history_a_view, &self.phosphor_history_a)
            };
            self.phosphor_frame_idx.set(1 - frame_idx);

            // Phosphor blend pass
            {
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("phosphor_render_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: dst_view,
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
                rpass.set_pipeline(&self.phosphor_pipeline);
                rpass.set_bind_group(0, bind_group, &[]);
                rpass.set_vertex_buffer(0, self.phosphor_vertex_buffer.slice(..));
                rpass.draw(0..3, 0..1);
            }

            // Copy phosphor result back to intermediate (level 0 only)
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: dst_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.intermediate_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: self.tex_width,
                    height: self.tex_height,
                    depth_or_array_layers: 1,
                },
            );
        }

        // Pass 0.5: NTSC notch filter (intermediate → ntsc_texture → intermediate)
        // Simulates composite video decoding with notch filter for authentic artifact colors
        if self.ntsc_strength.get() > 0.01 && self.is_mono.get() < 0.5 {
            // NTSC filter pass
            {
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ntsc_render_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.ntsc_view,
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
                rpass.set_pipeline(&self.ntsc_pipeline);
                rpass.set_bind_group(0, &self.ntsc_bind_group, &[]);
                rpass.set_vertex_buffer(0, self.gauss_vertex_buffer.slice(..));  // Reuse vertex buffer
                rpass.draw(0..3, 0..1);
            }

            // Copy NTSC result back to intermediate so all subsequent passes use filtered colors
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.ntsc_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.intermediate_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: self.tex_width,
                    height: self.tex_height,
                    depth_or_array_layers: 1,
                },
            );
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

        // Pass 2.5: Glow horizontal blur (blur_texture → glowx_texture)
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glowx_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.glowx_view,
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
            rpass.set_bind_group(0, &self.glowx_bind_group, &[]);
            rpass.set_vertex_buffer(0, self.gauss_vertex_buffer.slice(..));
            rpass.draw(0..3, 0..1);
        }

        // Pass 2.6: Glow vertical blur (glowx_texture → glow_texture)
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glowy_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.glow_view,
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
            rpass.set_bind_group(0, &self.glowy_bind_group, &[]);
            rpass.set_vertex_buffer(0, self.gauss_vertex_buffer.slice(..));
            rpass.draw(0..3, 0..1);
        }

        // Pass 3: CRT-Geom-Deluxe composite (intermediate + blur → chroma texture, no glow)
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("crt_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.chroma_texture_view,
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

        // Pass 4: Chromatic aberration + glow composite → screen
        // Chromatic aberration applied to CRT output, glow added after (unaffected by aberration)
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("chroma_render_pass"),
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
            rpass.set_pipeline(&self.chroma_pipeline);
            rpass.set_bind_group(0, &self.chroma_bind_group, &[]);
            rpass.set_vertex_buffer(0, self.chroma_vertex_buffer.slice(..));
            rpass.draw(0..3, 0..1);
        }
    }
}

impl PostProcessor for CrtRenderer {
    fn intermediate_view(&self) -> &wgpu::TextureView {
        CrtRenderer::intermediate_view(self)
    }

    fn resize(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        CrtRenderer::resize(self, device, queue, width, height)
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
        CrtRenderer::update_content_rect(
            self, queue, surface_w, surface_h,
            offset_x, offset_y, dst_w, dst_h,
            bar_h, source_width, source_height,
        )
    }

    fn update_time(&self, queue: &wgpu::Queue, time: f32) {
        CrtRenderer::update_time(self, queue, time)
    }

    fn update_monochrome(&self, queue: &wgpu::Queue, monochrome: bool) {
        CrtRenderer::update_monochrome(self, queue, monochrome)
    }

    fn update_shader_params(&self, queue: &wgpu::Queue, params: &shader_ui::ShaderParams) {
        CrtRenderer::update_shader_params(self, queue, params)
    }

    fn clear_intermediate(&self, encoder: &mut wgpu::CommandEncoder) {
        CrtRenderer::clear_intermediate(self, encoder)
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        render_target: &wgpu::TextureView,
        device: &wgpu::Device,
    ) {
        CrtRenderer::render(self, encoder, render_target, device)
    }
}
