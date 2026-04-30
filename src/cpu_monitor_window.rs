use std::sync::Arc;

use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

pub struct CpuMonitorWindow {
    pub window: Arc<Window>,
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    max_surface_dim: u32,
    pub egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

impl CpuMonitorWindow {
    pub fn new(event_loop: &ActiveEventLoop) -> Result<Self, String> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("CPU Monitor")
                        .with_inner_size(LogicalSize::new(960.0, 660.0))
                        .with_min_inner_size(LogicalSize::new(640.0, 380.0))
                        .with_resizable(true),
                )
                .map_err(|e| format!("create_window: {e}"))?,
        );

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("create_surface: {e}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .map_err(|e| format!("request_adapter: {e:?}"))?;

        let adapter_limits = adapter.limits();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("cpu_monitor_device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter_limits.clone(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::default(),
            }))
            .map_err(|e| format!("request_device: {e:?}"))?;

        let max_dim = adapter_limits.max_texture_dimension_2d.max(1);

        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let surface_format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else if caps.present_modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else {
            caps.present_modes[0]
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.clamp(1, max_dim),
            height: size.height.clamp(1, max_dim),
            present_mode,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(&device, surface_format, Default::default());

        window.request_redraw();

        Ok(Self {
            window,
            _instance: instance,
            surface,
            device,
            queue,
            config,
            max_surface_dim: max_dim,
            egui_ctx,
            egui_state,
            egui_renderer,
        })
    }

    #[inline]
    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    /// Forward a winit window event to egui. Returns `true` if egui consumed it.
    pub fn on_window_event(&mut self, event: &WindowEvent) -> bool {
        self.egui_state
            .on_window_event(self.window.as_ref(), event)
            .consumed
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        let max = self.max_surface_dim;
        self.config.width = w.min(max);
        self.config.height = h.min(max);
        self.surface.configure(&self.device, &self.config);
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn redraw<F: FnMut(&egui::Context)>(&mut self, mut build_ui: F) {
        let raw_input = self.egui_state.take_egui_input(self.window.as_ref());
        let output = self.egui_ctx.run(raw_input, |ctx| build_ui(ctx));
        self.egui_state
            .handle_platform_output(self.window.as_ref(), output.platform_output);

        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Outdated) | Err(wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            Err(e) => {
                eprintln!("cpu_monitor surface: {e:?}");
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());

        let ppp = output.pixels_per_point;
        let jobs = self.egui_ctx.tessellate(output.shapes, ppp);
        for (id, delta) in &output.textures_delta.set {
            self.egui_renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }
        let screen_desc = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: ppp,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cpu_monitor_encoder"),
            });
        let _ = self
            .egui_renderer
            .update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &screen_desc);
        {
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cpu_monitor_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.05,
                            b: 0.06,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let mut rpass = rpass.forget_lifetime();
            self.egui_renderer.render(&mut rpass, &jobs, &screen_desc);
        }
        self.queue.submit([encoder.finish()]);
        frame.present();
        for id in &output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
}
