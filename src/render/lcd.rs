// Multi-pass pipeline (based on libretro slang-shaders gameboy.slangp):
//   pass 0: dot matrix generation              src → t0
//   pass 1: adjacent-texel alpha blending      t0  → t1
//   pass 2: horizontal Gaussian blur of alpha  t1  → t2
//   pass 3: vertical   Gaussian blur of alpha  t2  → t3   (shadow map)
//   pass 4: final compositing                  src + t1 + t3 → surface
//
// Notes vs the upstream slang shader:
//   • response_time / 7-frame history is omitted — the IIc framebuffer
//     is updated every frame, no ghosting needed.
//   • COLOR_PALETTE.png and BACKGROUND.png LUTs are not bundled — the
//     palette is selected via a hard-coded switch in pass0, and the
//     background is a solid LCD bg colour in pass4.
//   • The bezel/overscan ring is rendered by passthrough of the
//     pre-filled framebuffer outside the content_rect (handled in
//     pass0 and pass4).

use wgpu::util::DeviceExt;

use super::screen::{ContentRect, PostProcessor, RendererInit};

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GbParams {
    output_size:  [f32; 4], // x=w, y=h, z=1/w, w=1/h
    source_size:  [f32; 4], // x=w, y=h, z=1/w, w=1/h
    content_rect: [f32; 4], // l, t, r, b in [0,1]
    pass1_size:   [f32; 4],
    config_a:     [f32; 4], // pixel_size, pixel_softness, sharpening, pixel_shape
    config_b:     [f32; 4], // sharp_mode, color_toggle, palette, baseline_alpha
    config_c:     [f32; 4], // grey_balance, brightness_mode, blending_mode, adjacent_blend
    config_d:     [f32; 4], // contrast, screen_light, pixel_opacity, bg_smoothing
    config_e:     [f32; 4], // shadow_opacity, shadow_x, shadow_y, shadow_enable
    config_f:     [f32; 4], // screen_x, screen_y, response_time, integer_mode/invert
    panel_extras: [f32; 4], // x=overscan_uv_x, y=overscan_uv_y, z=corner_radius_px, w=ghost_decay
    vignette_params: [f32; 4], // x=strength, y=inner_radius, z=outer_radius, w=unused
    vignette_tint:   [f32; 4], // rgb (sRGB), w=unused
    lcd_extras:      [f32; 4], // x=threshold, y=contrast, z=_, w=_
    lcd_bg_color:    [f32; 4], // rgb (sRGB), w=_
    lcd_fg_color:    [f32; 4], // rgb (sRGB), w=_
}

impl Default for GbParams {
    fn default() -> Self {
        Self {
            output_size:  [1.0, 1.0, 1.0, 1.0],
            source_size:  [560.0, 192.0, 1.0 / 560.0, 1.0 / 192.0],
            content_rect: [0.0, 0.0, 1.0, 1.0],
            pass1_size:   [1.0, 1.0, 1.0, 1.0],
            // pixel_size bumped from 0.80 → 0.95 so the LCD looks more
            config_a: [0.95, 1.0, 1.0, 1.0],   // pixel_size, softness, sharpening, shape
            config_b: [1.0, 0.0, 4.0, 0.10],   // sharp_mode=sigmoid, color=greyscale, palette=DMG green, baseline 0.1
            config_c: [3.0, 0.0, 0.0, 0.1755], // grey_balance, brightness_mode=simple, blending_mode=gaps, adjacent 0.1755
            config_d: [0.95, 1.0, 1.0, 0.75],  // contrast, screen_light, pixel_opacity, bg_smoothing
            config_e: [0.55, 1.25, 1.25, 1.0],   // shadow_opacity, shadow_x, shadow_y (texels), shadow_enable=1
            config_f: [0.0, 0.0, 0.0, 0.0],    // screen offsets, response_time off, invert flag (F5)
            // panel_extras: overscan uv (filled per-frame), corner_radius_px,
            // ghost_decay. Decay 0.0 = no ghosting, 0.95+ = strong trails.
            panel_extras: [0.0, 0.0, 5.0, 0.85],
            // LCD edge vignette (cool blue-green tint at corners).
            vignette_params: [0.55, 0.55, 1.10, 0.0],
            vignette_tint:   [28.0 / 255.0, 52.0 / 255.0, 58.0 / 255.0, 0.0],
            lcd_extras:      [0.5, 1.0, 0.0, 0.0],
            lcd_bg_color:    [88.0 / 255.0, 105.0 / 255.0, 50.0 / 255.0, 0.0],
            lcd_fg_color:    [35.0 / 255.0,  47.0 / 255.0, 47.0 / 255.0, 0.0],
        }
    }
}

const PASS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub struct LcdRenderer {
    sampler:        wgpu::Sampler,
    vertex_buffer:  wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    surface_format: wgpu::TextureFormat,
    aspect_correction: f32,

    // Layout 0: { tex2d, sampler, uniform } — used by passes 0..3.
    bgl_single: wgpu::BindGroupLayout,
    // Layout 1: { tex2d (orig), sampler, uniform, tex2d (pass1), tex2d (pass3) } — used by pass 4.
    bgl_compose: wgpu::BindGroupLayout,
    // Layout 2: { tex2d (pass1), sampler, uniform, tex2d (prev_ghost) } — ghost pass.
    bgl_ghost: wgpu::BindGroupLayout,

    // Pipelines (one per pass).
    pipe_pass0: wgpu::RenderPipeline,
    pipe_pass1: wgpu::RenderPipeline,
    pipe_pass2: wgpu::RenderPipeline,
    pipe_pass3: wgpu::RenderPipeline,
    pipe_pass4: wgpu::RenderPipeline,
    pipe_ghost: wgpu::RenderPipeline,

    // Cached current size.
    tex_w: u32,
    tex_h: u32,

    // Intermediate textures + views.
    src_tex:   wgpu::Texture, // emulator framebuffer destination (input to pass0 and pass4)
    src_view:  wgpu::TextureView,
    src_rview: wgpu::TextureView,
    t0_tex:    wgpu::Texture,
    t0_view:   wgpu::TextureView,
    t0_rview:  wgpu::TextureView,
    t1_tex:    wgpu::Texture,
    t1_view:   wgpu::TextureView,
    t1_rview:  wgpu::TextureView,
    t2_tex:    wgpu::Texture,
    t2_view:   wgpu::TextureView,
    t2_rview:  wgpu::TextureView,
    t3_tex:    wgpu::Texture,
    t3_view:   wgpu::TextureView,
    t3_rview:  wgpu::TextureView,
    // Ghost ping-pong textures (response_time / persistence emulation).
    ga_tex:    wgpu::Texture,
    ga_view:   wgpu::TextureView,
    ga_rview:  wgpu::TextureView,
    gb_tex:    wgpu::Texture,
    gb_view:   wgpu::TextureView,
    gb_rview:  wgpu::TextureView,

    // Bind groups.
    bg_pass0: wgpu::BindGroup,    // src → t0
    bg_pass1: wgpu::BindGroup,    // t0  → t1
    bg_pass2: wgpu::BindGroup,    // t1  → t2
    bg_pass3: wgpu::BindGroup,    // t2  → t3
    // Ghost: write side A reads (t1 + prev=B); write side B reads (t1 + prev=A).
    bg_ghost_to_a: wgpu::BindGroup,
    bg_ghost_to_b: wgpu::BindGroup,
    // Pass4: choose ghost side as foreground.
    bg_pass4_a: wgpu::BindGroup,  // foreground = ga
    bg_pass4_b: wgpu::BindGroup,  // foreground = gb

    // Ping-pong selector. False ⇒ next frame writes ghost A, reads B; True ⇒ vice-versa.
    ghost_side: std::cell::Cell<bool>,
}

impl LcdRenderer {
    // Reference monitors a real Apple IIc flat panel as ~16/25 vertical squish.
    // Per-pixel vertical squash applied on top of the standard ×2 row
    // doubling. Calibrated so the rendered window matches the real Apple
    // //c flat-panel LCD physical aspect (11.25" × 5.25" = 15:7 ≈ 2.143).
    pub const LCD_WINDOW_ASPECT: f32 = 15.0 / 7.0;
    // Kept for legacy callers; no longer used for LCD sizing.
    pub const LCD_ASPECT_CORRECTION: f32 = 0.6806;

    pub fn new(init: RendererInit<'_>) -> Self {
        let RendererInit {
            device,
            surface_width,
            surface_height,
            buffer_width: _,
            buffer_height: _,
            bar_height: _,
            source_width,
            source_height,
            surface_format,
        } = init;

        let shader0 = device.create_shader_module(wgpu::include_wgsl!("shaders/gb_pass0.wgsl"));
        let shader1 = device.create_shader_module(wgpu::include_wgsl!("shaders/gb_pass1.wgsl"));
        let shader2 = device.create_shader_module(wgpu::include_wgsl!("shaders/gb_pass2.wgsl"));
        let shader3 = device.create_shader_module(wgpu::include_wgsl!("shaders/gb_pass3.wgsl"));
        let shader4 = device.create_shader_module(wgpu::include_wgsl!("shaders/gb_pass4.wgsl"));
        let shader_ghost = device.create_shader_module(wgpu::include_wgsl!("shaders/gb_ghost.wgsl"));

        // Full-screen triangle.
        let vertex_data: [[f32; 2]; 3] = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gb_vertex_buffer"),
            contents: bytemuck::cast_slice(&vertex_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let mut params = GbParams::default();
        params.output_size = [
            surface_width as f32, surface_height as f32,
            1.0 / surface_width as f32, 1.0 / surface_height as f32,
        ];
        params.pass1_size = params.output_size;
        params.source_size = [
            source_width, source_height,
            1.0 / source_width, 1.0 / source_height,
        ];

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gb_uniform_buffer"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gb_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        // Bind group layouts.
        let bgl_single = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gb_bgl_single"),
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
            ],
        });

        let bgl_compose = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gb_bgl_compose"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        // Pipeline layouts.
        let pl_single = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gb_pl_single"),
            bind_group_layouts: &[&bgl_single],
            push_constant_ranges: &[],
        });
        let pl_compose = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gb_pl_compose"),
            bind_group_layouts: &[&bgl_compose],
            push_constant_ranges: &[],
        });

        // Ghost pass layout: { pass1_tex, sampler, uniform, prev_ghost_tex }.
        let bgl_ghost = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gb_bgl_ghost"),
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
            ],
        });
        let pl_ghost = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gb_pl_ghost"),
            bind_group_layouts: &[&bgl_ghost],
            push_constant_ranges: &[],
        });

        let vbuf_layout = wgpu::VertexBufferLayout {
            array_stride: 8,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            }],
        };

        let make_pipeline = |device: &wgpu::Device,
                             label: &str,
                             layout: &wgpu::PipelineLayout,
                             shader: &wgpu::ShaderModule,
                             format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers: std::slice::from_ref(&vbuf_layout),
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview: None,
                cache: None,
            })
        };

        let pipe_pass0 = make_pipeline(device, "gb_pipe_pass0", &pl_single, &shader0, PASS_FORMAT);
        let pipe_pass1 = make_pipeline(device, "gb_pipe_pass1", &pl_single, &shader1, PASS_FORMAT);
        let pipe_pass2 = make_pipeline(device, "gb_pipe_pass2", &pl_single, &shader2, PASS_FORMAT);
        let pipe_pass3 = make_pipeline(device, "gb_pipe_pass3", &pl_single, &shader3, PASS_FORMAT);
        let pipe_pass4 = make_pipeline(device, "gb_pipe_pass4", &pl_compose, &shader4, surface_format);
        let pipe_ghost = make_pipeline(device, "gb_pipe_ghost", &pl_ghost,   &shader_ghost, PASS_FORMAT);

        // Intermediate textures.
        let (src_tex, src_view, src_rview)   = make_tex(device, surface_width, surface_height, surface_format, "gb_src");
        let (t0_tex,  t0_view,  t0_rview)    = make_tex(device, surface_width, surface_height, PASS_FORMAT, "gb_t0");
        let (t1_tex,  t1_view,  t1_rview)    = make_tex(device, surface_width, surface_height, PASS_FORMAT, "gb_t1");
        let (t2_tex,  t2_view,  t2_rview)    = make_tex(device, surface_width, surface_height, PASS_FORMAT, "gb_t2");
        let (t3_tex,  t3_view,  t3_rview)    = make_tex(device, surface_width, surface_height, PASS_FORMAT, "gb_t3");
        let (ga_tex,  ga_view,  ga_rview)    = make_tex(device, surface_width, surface_height, PASS_FORMAT, "gb_ghost_a");
        let (gb_tex,  gb_view,  gb_rview)    = make_tex(device, surface_width, surface_height, PASS_FORMAT, "gb_ghost_b");

        let bg_pass0 = make_bg_single(device, &bgl_single, &src_view, &sampler, &uniform_buffer, "gb_bg_pass0");
        let bg_pass1 = make_bg_single(device, &bgl_single, &t0_view,  &sampler, &uniform_buffer, "gb_bg_pass1");
        let bg_pass2 = make_bg_single(device, &bgl_single, &t1_view,  &sampler, &uniform_buffer, "gb_bg_pass2");
        let bg_pass3 = make_bg_single(device, &bgl_single, &t2_view,  &sampler, &uniform_buffer, "gb_bg_pass3");
        let bg_ghost_to_a = make_bg_ghost(device, &bgl_ghost, &t1_view, &sampler, &uniform_buffer, &gb_view, "gb_bg_ghost_to_a");
        let bg_ghost_to_b = make_bg_ghost(device, &bgl_ghost, &t1_view, &sampler, &uniform_buffer, &ga_view, "gb_bg_ghost_to_b");
        let bg_pass4_a = make_bg_compose(
            device, &bgl_compose, &src_view, &sampler, &uniform_buffer,
            &ga_view, &t3_view, "gb_bg_pass4_a",
        );
        let bg_pass4_b = make_bg_compose(
            device, &bgl_compose, &src_view, &sampler, &uniform_buffer,
            &gb_view, &t3_view, "gb_bg_pass4_b",
        );

        Self {
            sampler,
            vertex_buffer,
            uniform_buffer,
            surface_format,
            aspect_correction: Self::LCD_ASPECT_CORRECTION,
            bgl_single,
            bgl_compose,
            bgl_ghost,
            pipe_pass0,
            pipe_pass1,
            pipe_pass2,
            pipe_pass3,
            pipe_pass4,
            pipe_ghost,
            tex_w: surface_width,
            tex_h: surface_height,
            src_tex, src_view, src_rview,
            t0_tex,  t0_view,  t0_rview,
            t1_tex,  t1_view,  t1_rview,
            t2_tex,  t2_view,  t2_rview,
            t3_tex,  t3_view,  t3_rview,
            ga_tex,  ga_view,  ga_rview,
            gb_tex,  gb_view,  gb_rview,
            bg_pass0, bg_pass1, bg_pass2, bg_pass3,
            bg_ghost_to_a, bg_ghost_to_b,
            bg_pass4_a, bg_pass4_b,
            ghost_side: std::cell::Cell::new(false),
        }
    }

    fn rebuild_textures(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        let (src_tex, src_view, src_rview) = make_tex(device, w, h, self.surface_format, "gb_src");
        let (t0_tex,  t0_view,  t0_rview)  = make_tex(device, w, h, PASS_FORMAT, "gb_t0");
        let (t1_tex,  t1_view,  t1_rview)  = make_tex(device, w, h, PASS_FORMAT, "gb_t1");
        let (t2_tex,  t2_view,  t2_rview)  = make_tex(device, w, h, PASS_FORMAT, "gb_t2");
        let (t3_tex,  t3_view,  t3_rview)  = make_tex(device, w, h, PASS_FORMAT, "gb_t3");
        let (ga_tex,  ga_view,  ga_rview)  = make_tex(device, w, h, PASS_FORMAT, "gb_ghost_a");
        let (gb_tex,  gb_view,  gb_rview)  = make_tex(device, w, h, PASS_FORMAT, "gb_ghost_b");

        self.src_tex = src_tex; self.src_view = src_view; self.src_rview = src_rview;
        self.t0_tex  = t0_tex;  self.t0_view  = t0_view;  self.t0_rview  = t0_rview;
        self.t1_tex  = t1_tex;  self.t1_view  = t1_view;  self.t1_rview  = t1_rview;
        self.t2_tex  = t2_tex;  self.t2_view  = t2_view;  self.t2_rview  = t2_rview;
        self.t3_tex  = t3_tex;  self.t3_view  = t3_view;  self.t3_rview  = t3_rview;
        self.ga_tex  = ga_tex;  self.ga_view  = ga_view;  self.ga_rview  = ga_rview;
        self.gb_tex  = gb_tex;  self.gb_view  = gb_view;  self.gb_rview  = gb_rview;

        self.bg_pass0 = make_bg_single(device, &self.bgl_single, &self.src_view, &self.sampler, &self.uniform_buffer, "gb_bg_pass0");
        self.bg_pass1 = make_bg_single(device, &self.bgl_single, &self.t0_view,  &self.sampler, &self.uniform_buffer, "gb_bg_pass1");
        self.bg_pass2 = make_bg_single(device, &self.bgl_single, &self.t1_view,  &self.sampler, &self.uniform_buffer, "gb_bg_pass2");
        self.bg_pass3 = make_bg_single(device, &self.bgl_single, &self.t2_view,  &self.sampler, &self.uniform_buffer, "gb_bg_pass3");
        self.bg_ghost_to_a = make_bg_ghost(device, &self.bgl_ghost, &self.t1_view, &self.sampler, &self.uniform_buffer, &self.gb_view, "gb_bg_ghost_to_a");
        self.bg_ghost_to_b = make_bg_ghost(device, &self.bgl_ghost, &self.t1_view, &self.sampler, &self.uniform_buffer, &self.ga_view, "gb_bg_ghost_to_b");
        self.bg_pass4_a = make_bg_compose(
            device, &self.bgl_compose, &self.src_view, &self.sampler, &self.uniform_buffer,
            &self.ga_view, &self.t3_view, "gb_bg_pass4_a",
        );
        self.bg_pass4_b = make_bg_compose(
            device, &self.bgl_compose, &self.src_view, &self.sampler, &self.uniform_buffer,
            &self.gb_view, &self.t3_view, "gb_bg_pass4_b",
        );

        self.tex_w = w;
        self.tex_h = h;
    }
}

fn make_tex(
    device: &wgpu::Device,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
             | wgpu::TextureUsages::RENDER_ATTACHMENT
             | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view  = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let rview = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view, rview)
}

fn make_bg_single(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    tex: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniform: &wgpu::Buffer,
    label: &str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(tex) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
        ],
    })
}

#[allow(clippy::too_many_arguments)]
fn make_bg_compose(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    src: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniform: &wgpu::Buffer,
    pass1: &wgpu::TextureView,
    pass3: &wgpu::TextureView,
    label: &str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(pass1) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(pass3) },
        ],
    })
}

fn make_bg_ghost(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    pass1: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniform: &wgpu::Buffer,
    prev_ghost: &wgpu::TextureView,
    label: &str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(pass1) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(prev_ghost) },
        ],
    })
}

impl PostProcessor for LcdRenderer {
    fn intermediate_view(&self) -> &wgpu::TextureView {
        // pixels.rs renders the emulator framebuffer here. Pass0 reads it,
        // and pass4 also samples it for the bezel/overscan passthrough.
        &self.src_rview
    }

    fn resize(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        if width == self.tex_w && height == self.tex_h {
            return;
        }
        self.rebuild_textures(device, width, height);

        // Update output_size / pass1_size in the uniform buffer.
        let os = [width as f32, height as f32, 1.0 / width as f32, 1.0 / height as f32];
        // output_size at offset 0, pass1_size at offset 16+16+16=48.
        queue.write_buffer(&self.uniform_buffer, 0,  bytemuck::bytes_of(&os));
        queue.write_buffer(&self.uniform_buffer, 48, bytemuck::bytes_of(&os));
    }

    fn update_content_rect(&self, queue: &wgpu::Queue, rect: &ContentRect) {
        let &ContentRect {
            surface_w, surface_h,
            offset_x, offset_y,
            dst_w, dst_h,
            bar_h: _,
            source_width, source_height,
            overscan_x_px,
            overscan_y_px,
        } = rect;

        let sw = surface_w as f32;
        let sh = surface_h as f32;
        let cr = [
            offset_x as f32 / sw,
            offset_y as f32 / sh,
            (offset_x + dst_w) as f32 / sw,
            (offset_y + dst_h) as f32 / sh,
        ];

        // content_rect at offset 32 (output_size + source_size each 16 bytes).
        queue.write_buffer(&self.uniform_buffer, 32, bytemuck::bytes_of(&cr));

        // source_size at offset 16.
        let ss = [source_width, source_height, 1.0 / source_width, 1.0 / source_height];
        queue.write_buffer(&self.uniform_buffer, 16, bytemuck::bytes_of(&ss));

        // panel_extras .xy = overscan ring width in uv space (offset 160).
        let ovx = overscan_x_px as f32 / sw;
        let ovy = overscan_y_px as f32 / sh;
        let ov = [ovx, ovy];
        queue.write_buffer(&self.uniform_buffer, 160, bytemuck::bytes_of(&ov));

        // Mark aspect_correction usage so tooling doesn't flag it as dead code.
        let _ = self.aspect_correction;
    }

    fn update_time(&self, _queue: &wgpu::Queue, _time: f32) {
        // No animated effects in this port.
    }

    fn update_monochrome(&self, _queue: &wgpu::Queue, _monochrome: bool) {
        // LCD is intrinsically monochrome.
    }

    fn update_text_mode(&self, _queue: &wgpu::Queue, _text_only: bool) {
        // No NTSC processing.
    }

    fn update_power_on_time(&self, _queue: &wgpu::Queue, _elapsed_secs: f32) {
        // No power-on sync.
    }

    fn update_shader_params(&self, queue: &wgpu::Queue, params: &shader_ui::ShaderParams) {
        // Reuse the global ShaderParams for LCD-specific knobs.
        // Invert (config_f.w at offset 156).
        let inv: f32 = if params.lcd_invert { 1.0 } else { 0.0 };
        queue.write_buffer(&self.uniform_buffer, 156, bytemuck::bytes_of(&inv));
        // panel_extras layout (offset 160): xy=overscan (filled per-frame in
        // update_content_rect), z=corner_radius_px, w=ghost_decay.
        let corner = params.lcd_corner_radius_px.max(0.0);
        let decay  = params.lcd_ghost_decay.clamp(0.0, 0.999);
        queue.write_buffer(&self.uniform_buffer, 168, bytemuck::bytes_of(&corner));
        queue.write_buffer(&self.uniform_buffer, 172, bytemuck::bytes_of(&decay));
        // vignette_params (offset 176): strength, inner_r, outer_r, _.
        let vp: [f32; 4] = [
            params.lcd_vignette_strength.clamp(0.0, 1.0),
            params.lcd_vignette_inner.max(0.0),
            params.lcd_vignette_outer.max(params.lcd_vignette_inner + 0.001),
            0.0,
        ];
        queue.write_buffer(&self.uniform_buffer, 176, bytemuck::bytes_of(&vp));
        // vignette_tint (offset 192): rgb, _.
        let vt: [f32; 4] = [
            params.lcd_vignette_tint[0],
            params.lcd_vignette_tint[1],
            params.lcd_vignette_tint[2],
            0.0,
        ];
        queue.write_buffer(&self.uniform_buffer, 192, bytemuck::bytes_of(&vt));
        // lcd_extras (offset 208): threshold, contrast, _, _.
        let le: [f32; 4] = [
            params.lcd_threshold.clamp(0.0, 1.0),
            params.lcd_contrast.max(0.0),
            0.0,
            0.0,
        ];
        queue.write_buffer(&self.uniform_buffer, 208, bytemuck::bytes_of(&le));
        // lcd_bg_color (offset 224), lcd_fg_color (offset 240).
        let bg: [f32; 4] = [
            params.lcd_bg_color[0], params.lcd_bg_color[1], params.lcd_bg_color[2], 0.0,
        ];
        let fg: [f32; 4] = [
            params.lcd_fg_color[0], params.lcd_fg_color[1], params.lcd_fg_color[2], 0.0,
        ];
        queue.write_buffer(&self.uniform_buffer, 224, bytemuck::bytes_of(&bg));
        queue.write_buffer(&self.uniform_buffer, 240, bytemuck::bytes_of(&fg));
    }

    fn set_invert(&self, queue: &wgpu::Queue, invert: bool) {
        // config_f.w lives at offset 144 + 12 = 156. Reuse the (otherwise
        // unused) integer_mode slot to carry the invert flag into pass4.
        let v: f32 = if invert { 1.0 } else { 0.0 };
        queue.write_buffer(&self.uniform_buffer, 156, bytemuck::bytes_of(&v));
    }

    fn clear_intermediate(&self, encoder: &mut wgpu::CommandEncoder) {
        // Clear the source intermediate before pixels.rs writes into it.
        let _rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gb_clear_src"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.src_rview,
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

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        render_target: &wgpu::TextureView,
        _device: &wgpu::Device,
    ) {
        // Helper to issue a single full-screen pass.
        let mut run_pass = |label: &str,
                            target: &wgpu::TextureView,
                            pipe: &wgpu::RenderPipeline,
                            bg: &wgpu::BindGroup| {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(pipe);
            rpass.set_bind_group(0, bg, &[]);
            rpass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            rpass.draw(0..3, 0..1);
        };

        run_pass("gb_pass0", &self.t0_rview, &self.pipe_pass0, &self.bg_pass0);
        run_pass("gb_pass1", &self.t1_rview, &self.pipe_pass1, &self.bg_pass1);

        // Ghost / response-time pass: leaky-max accumulation of the pass1
        // foreground into one of the two ghost textures (ping-pong).
        let write_a = !self.ghost_side.get();
        let (ghost_target, ghost_bg, pass4_bg) = if write_a {
            (&self.ga_rview, &self.bg_ghost_to_a, &self.bg_pass4_a)
        } else {
            (&self.gb_rview, &self.bg_ghost_to_b, &self.bg_pass4_b)
        };
        run_pass("gb_pass_ghost", ghost_target, &self.pipe_ghost, ghost_bg);

        run_pass("gb_pass2", &self.t2_rview, &self.pipe_pass2, &self.bg_pass2);
        run_pass("gb_pass3", &self.t3_rview, &self.pipe_pass3, &self.bg_pass3);
        run_pass("gb_pass4", render_target,  &self.pipe_pass4, pass4_bg);

        // Flip ghost side for next frame.
        self.ghost_side.set(write_a);
    }
}
