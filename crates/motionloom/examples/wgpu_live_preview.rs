// =========================================
// =========================================
// crates/motionloom/examples/wgpu_live_preview.rs

use std::sync::Arc;
use std::time::{Duration, Instant};

use motionloom::{SceneRenderProfile, SceneRenderer, parse_graph_script};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

const BLIT_SHADER: &str = r#"
@group(0) @binding(0) var scene_tex: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>( 3.0,  1.0),
        vec2<f32>(-1.0,  1.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 2.0),
        vec2<f32>(2.0, 0.0),
        vec2<f32>(0.0, 0.0),
    );

    var out: VertexOut;
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    out.uv = uvs[vertex_index];
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(scene_tex, scene_sampler, in.uv);
}
"#;

const OVERLAY_SHADER: &str = r#"
struct OverlayUniforms {
    surface_size: vec2<f32>,
    active_mode: u32,
    _pad: u32,
};

@group(0) @binding(0) var<uniform> overlay: OverlayUniforms;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) instance: u32,
    @location(1) local: vec2<f32>,
};

fn button_rect(instance: u32) -> vec4<f32> {
    if (instance == 0u) {
        return vec4<f32>(12.0, 12.0, 80.0, 28.0);
    }
    if (instance == 1u) {
        return vec4<f32>(100.0, 12.0, 112.0, 28.0);
    }
    if (instance == 2u) {
        return vec4<f32>(220.0, 12.0, 76.0, 28.0);
    }
    if (instance == 3u) {
        return vec4<f32>(304.0, 12.0, 118.0, 28.0);
    }
    return vec4<f32>(430.0, 12.0, 118.0, 28.0);
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance: u32) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let local = corners[vertex_index];
    let rect = button_rect(instance);
    let pixel = rect.xy + local * rect.zw;
    let ndc = vec2<f32>(
        pixel.x / max(overlay.surface_size.x, 1.0) * 2.0 - 1.0,
        1.0 - pixel.y / max(overlay.surface_size.y, 1.0) * 2.0,
    );

    var out: VertexOut;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.instance = instance;
    out.local = local;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let border = in.local.x < 0.04 || in.local.x > 0.96 || in.local.y < 0.10 || in.local.y > 0.90;
    if (in.instance == overlay.active_mode) {
        if (border) {
            return vec4<f32>(0.70, 0.88, 1.0, 0.95);
        }
        return vec4<f32>(0.12, 0.36, 0.60, 0.78);
    }
    if (border) {
        return vec4<f32>(0.42, 0.46, 0.52, 0.86);
    }
    return vec4<f32>(0.05, 0.06, 0.08, 0.72);
}
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewQuality {
    Full,
    Balanced,
    Speed,
    HighSpeed,
    UltraSpeed,
}

impl PreviewQuality {
    const fn label(self) -> &'static str {
        match self {
            PreviewQuality::Full => "Full",
            PreviewQuality::Balanced => "Balanced 50%",
            PreviewQuality::Speed => "Speed 25%",
            PreviewQuality::HighSpeed => "High Speed 10%",
            PreviewQuality::UltraSpeed => "Ultra Speed 5%",
        }
    }

    const fn scale(self) -> f32 {
        match self {
            PreviewQuality::Full => 1.0,
            PreviewQuality::Balanced => 0.5,
            PreviewQuality::Speed => 0.25,
            PreviewQuality::HighSpeed => 0.10,
            PreviewQuality::UltraSpeed => 0.05,
        }
    }

    const fn index(self) -> u32 {
        match self {
            PreviewQuality::Full => 0,
            PreviewQuality::Balanced => 1,
            PreviewQuality::Speed => 2,
            PreviewQuality::HighSpeed => 3,
            PreviewQuality::UltraSpeed => 4,
        }
    }
}

struct LivePreviewApp {
    script_source: String,
    base_graph: motionloom::GraphScript,
    graph: Option<motionloom::GraphScript>,
    window: Option<Arc<Window>>,
    instance: Option<wgpu::Instance>,
    surface: Option<wgpu::Surface<'static>>,
    adapter: Option<wgpu::Adapter>,
    device: Option<Arc<wgpu::Device>>,
    queue: Option<wgpu::Queue>,
    surface_config: Option<wgpu::SurfaceConfiguration>,
    surface_format: Option<wgpu::TextureFormat>,
    renderer: Option<SceneRenderer>,
    target_texture: Option<wgpu::Texture>,
    target_width: u32,
    target_height: u32,
    sampler: Option<wgpu::Sampler>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    pipeline: Option<wgpu::RenderPipeline>,
    overlay_buffer: Option<wgpu::Buffer>,
    overlay_bind_group: Option<wgpu::BindGroup>,
    overlay_pipeline: Option<wgpu::RenderPipeline>,
    quality: PreviewQuality,
    last_cursor_pos: Option<(f64, f64)>,
    frame: u32,
    total_frames: u32,
    last_frame_at: Instant,
    last_title_at: Instant,
    last_stats_at: Instant,
    print_stats_enabled: bool,
    render_times: Vec<f32>,
    present_times: Vec<f32>,
}

impl LivePreviewApp {
    fn new(
        script_source: String,
        print_stats_enabled: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let script = load_script_source(&script_source)?;
        let graph = parse_graph_script(&script)?;
        let fps = graph.fps.max(1.0);
        let total_frames =
            (((graph.duration_ms as f32 / 1000.0).max(1.0 / fps) * fps).round() as u32).max(1);

        Ok(Self {
            script_source,
            base_graph: graph.clone(),
            graph: Some(graph),
            window: None,
            instance: None,
            surface: None,
            adapter: None,
            device: None,
            queue: None,
            surface_config: None,
            surface_format: None,
            renderer: None,
            target_texture: None,
            target_width: 0,
            target_height: 0,
            sampler: None,
            bind_group_layout: None,
            pipeline: None,
            overlay_buffer: None,
            overlay_bind_group: None,
            overlay_pipeline: None,
            quality: PreviewQuality::Full,
            last_cursor_pos: None,
            frame: 0,
            total_frames,
            last_frame_at: Instant::now(),
            last_title_at: Instant::now(),
            last_stats_at: Instant::now(),
            print_stats_enabled,
            render_times: Vec::with_capacity(240),
            present_times: Vec::with_capacity(240),
        })
    }

    fn preview_size_for_quality(
        graph: &motionloom::GraphScript,
        quality: PreviewQuality,
    ) -> (u32, u32) {
        let (final_width, final_height) = graph.render_size.unwrap_or(graph.size);
        let scale = quality.scale();
        if scale >= 0.999 {
            return (final_width.max(1), final_height.max(1));
        }
        (
            (final_width.max(1) as f32 * scale).round().max(1.0) as u32,
            (final_height.max(1) as f32 * scale).round().max(1.0) as u32,
        )
    }

    fn graph_for_quality(
        base_graph: &motionloom::GraphScript,
        quality: PreviewQuality,
    ) -> motionloom::GraphScript {
        let mut graph = base_graph.clone();
        let final_size = graph.render_size.unwrap_or(graph.size);
        let preview_size = Self::preview_size_for_quality(&graph, quality);
        if preview_size != final_size {
            graph.render_size = Some(preview_size);
            for texture in &mut graph.textures {
                if texture.size == Some(final_size) {
                    texture.size = Some(preview_size);
                }
            }
        }
        graph
    }

    fn create_target_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("motionloom-live-preview-target-texture"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    }

    fn set_quality(&mut self, quality: PreviewQuality) {
        if self.quality == quality {
            return;
        }
        self.quality = quality;
        let graph = Self::graph_for_quality(&self.base_graph, quality);
        let (target_width, target_height) = graph.render_size.unwrap_or(graph.size);
        self.graph = Some(graph);
        self.target_width = target_width.max(1);
        self.target_height = target_height.max(1);
        if let Some(device) = self.device.as_ref() {
            self.target_texture = Some(Self::create_target_texture(
                device,
                self.target_width,
                self.target_height,
            ));
        }
        self.render_times.clear();
        self.present_times.clear();
        self.update_title_now();
    }

    fn quality_button_at(position: (f64, f64)) -> Option<PreviewQuality> {
        let (x, y) = position;
        if !(12.0..=40.0).contains(&y) {
            return None;
        }
        if (12.0..=92.0).contains(&x) {
            return Some(PreviewQuality::Full);
        }
        if (100.0..=212.0).contains(&x) {
            return Some(PreviewQuality::Balanced);
        }
        if (220.0..=296.0).contains(&x) {
            return Some(PreviewQuality::Speed);
        }
        if (304.0..=422.0).contains(&x) {
            return Some(PreviewQuality::HighSpeed);
        }
        if (430.0..=548.0).contains(&x) {
            return Some(PreviewQuality::UltraSpeed);
        }
        None
    }

    fn init_wgpu(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let window = Arc::new(
            event_loop.create_window(
                WindowAttributes::default()
                    .with_title("MotionLoom wgpu live preview")
                    .with_inner_size(PhysicalSize::new(1280, 720)),
            )?,
        );
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("motionloom-live-preview-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            }))?;
        let device = Arc::new(device);
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .unwrap_or(surface_caps.formats[0]);
        let present_mode = surface_caps
            .present_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::PresentMode::Immediate)
            .unwrap_or(wgpu::PresentMode::Fifo);
        let alpha_mode = surface_caps.alpha_modes[0];
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&device, &surface_config);

        let renderer = pollster::block_on(SceneRenderer::new_with_device(
            device.clone(),
            queue.clone(),
            SceneRenderProfile::Gpu,
        ))?;
        let graph = Self::graph_for_quality(&self.base_graph, self.quality);
        let (target_width, target_height) = graph.render_size.unwrap_or(graph.size);
        let target_texture =
            Self::create_target_texture(&device, target_width.max(1), target_height.max(1));

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("motionloom-live-preview-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("motionloom-live-preview-bind-group-layout"),
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
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("motionloom-live-preview-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("motionloom-live-preview-blit-shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("motionloom-live-preview-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let overlay_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("motionloom-live-preview-overlay-buffer"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let overlay_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("motionloom-live-preview-overlay-bind-group-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let overlay_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("motionloom-live-preview-overlay-bind-group"),
            layout: &overlay_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: overlay_buffer.as_entire_binding(),
            }],
        });
        let overlay_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("motionloom-live-preview-overlay-pipeline-layout"),
                bind_group_layouts: &[&overlay_bind_group_layout],
                push_constant_ranges: &[],
            });
        let overlay_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("motionloom-live-preview-overlay-shader"),
            source: wgpu::ShaderSource::Wgsl(OVERLAY_SHADER.into()),
        });
        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("motionloom-live-preview-overlay-pipeline"),
            layout: Some(&overlay_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &overlay_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &overlay_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        self.window = Some(window);
        self.instance = Some(instance);
        self.surface = Some(surface);
        self.adapter = Some(adapter);
        self.device = Some(device);
        self.queue = Some(queue);
        self.surface_config = Some(surface_config);
        self.surface_format = Some(surface_format);
        self.renderer = Some(renderer);
        self.graph = Some(graph);
        self.target_texture = Some(target_texture);
        self.target_width = target_width.max(1);
        self.target_height = target_height.max(1);
        self.sampler = Some(sampler);
        self.bind_group_layout = Some(bind_group_layout);
        self.pipeline = Some(pipeline);
        self.overlay_buffer = Some(overlay_buffer);
        self.overlay_bind_group = Some(overlay_bind_group);
        self.overlay_pipeline = Some(overlay_pipeline);
        Ok(())
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let Some(device) = self.device.as_ref() else {
            return;
        };
        let Some(config) = self.surface_config.as_mut() else {
            return;
        };
        config.width = size.width.max(1);
        config.height = size.height.max(1);
        surface.configure(device, config);
    }

    fn render(&mut self) {
        let (
            Some(graph),
            Some(surface),
            Some(device),
            Some(queue),
            Some(renderer),
            Some(target_texture),
            Some(sampler),
            Some(bind_group_layout),
            Some(pipeline),
            Some(overlay_buffer),
            Some(overlay_bind_group),
            Some(overlay_pipeline),
            Some(surface_config),
        ) = (
            self.graph.as_ref(),
            self.surface.as_ref(),
            self.device.as_ref(),
            self.queue.as_ref(),
            self.renderer.as_mut(),
            self.target_texture.as_ref(),
            self.sampler.as_ref(),
            self.bind_group_layout.as_ref(),
            self.pipeline.as_ref(),
            self.overlay_buffer.as_ref(),
            self.overlay_bind_group.as_ref(),
            self.overlay_pipeline.as_ref(),
            self.surface_config.as_ref(),
        )
        else {
            return;
        };

        let render_start = Instant::now();
        if let Err(err) = pollster::block_on(renderer.render_frame_to_wgpu_target_texture(
            graph,
            self.frame,
            target_texture,
            self.target_width,
            self.target_height,
        )) {
            eprintln!("render frame {} failed: {err}", self.frame);
            return;
        }
        let render_ms = render_start.elapsed().as_secs_f32() * 1000.0;

        let surface_texture = match surface.get_current_texture() {
            Ok(texture) => texture,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                if let Some(config) = self.surface_config.as_ref() {
                    surface.configure(device, config);
                }
                return;
            }
            Err(wgpu::SurfaceError::Timeout) => return,
            Err(wgpu::SurfaceError::OutOfMemory) => {
                eprintln!("surface out of memory");
                return;
            }
            Err(wgpu::SurfaceError::Other) => return,
        };

        let present_start = Instant::now();
        let scene_view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("motionloom-live-preview-bind-group"),
            layout: bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&scene_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("motionloom-live-preview-command-encoder"),
        });
        let mut overlay_uniforms = [0u8; 16];
        overlay_uniforms[0..4].copy_from_slice(&(surface_config.width as f32).to_ne_bytes());
        overlay_uniforms[4..8].copy_from_slice(&(surface_config.height as f32).to_ne_bytes());
        overlay_uniforms[8..12].copy_from_slice(&self.quality.index().to_ne_bytes());
        queue.write_buffer(overlay_buffer, 0, &overlay_uniforms);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("motionloom-live-preview-blit-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(overlay_pipeline);
            pass.set_bind_group(0, overlay_bind_group, &[]);
            pass.draw(0..6, 0..5);
        }
        queue.submit(Some(encoder.finish()));
        surface_texture.present();
        let present_ms = present_start.elapsed().as_secs_f32() * 1000.0;

        self.render_times.push(render_ms);
        self.present_times.push(present_ms);
        if self.render_times.len() > 240 {
            self.render_times.remove(0);
        }
        if self.present_times.len() > 240 {
            self.present_times.remove(0);
        }

        self.frame = self.frame.saturating_add(1) % self.total_frames;
        self.update_title();
        self.print_stats();
    }

    fn update_title(&mut self) {
        if self.last_title_at.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.last_title_at = Instant::now();
        self.update_title_now();
    }

    fn update_title_now(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let avg_render = avg(&self.render_times);
        let avg_present = avg(&self.present_times);
        let fps = if self.last_frame_at.elapsed().as_secs_f32() > 0.0 {
            1.0 / self.last_frame_at.elapsed().as_secs_f32()
        } else {
            0.0
        };
        self.last_frame_at = Instant::now();
        window.set_title(&format!(
            "MotionLoom wgpu live preview | frame {}/{} | render {:.2} ms | blit+present {:.2} ms | tick {:.1} fps | target {}x{} | surface {:?} | quality {} (1 Full, 2 Balanced, 3 Speed, 4 High Speed, 5 Ultra Speed) | {}",
            self.frame,
            self.total_frames,
            avg_render,
            avg_present,
            fps,
            self.target_width,
            self.target_height,
            self.surface_format,
            self.quality.label(),
            self.script_source
        ));
    }

    fn print_stats(&mut self) {
        if !self.print_stats_enabled {
            return;
        }
        if self.last_stats_at.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_stats_at = Instant::now();
        println!(
            "quality={} target={}x{} frame={}/{} render_ms={:.2} blit_present_ms={:.2}",
            self.quality.label(),
            self.target_width,
            self.target_height,
            self.frame,
            self.total_frames,
            avg(&self.render_times),
            avg(&self.present_times)
        );
    }
}

impl ApplicationHandler for LivePreviewApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(err) = self.init_wgpu(event_loop)
        {
            eprintln!("failed to initialize wgpu live preview: {err}");
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                        PhysicalKey::Code(KeyCode::Digit1) => {
                            self.set_quality(PreviewQuality::Full);
                        }
                        PhysicalKey::Code(KeyCode::Digit2) => {
                            self.set_quality(PreviewQuality::Balanced);
                        }
                        PhysicalKey::Code(KeyCode::Digit3) => {
                            self.set_quality(PreviewQuality::Speed);
                        }
                        PhysicalKey::Code(KeyCode::Digit4) => {
                            self.set_quality(PreviewQuality::HighSpeed);
                        }
                        PhysicalKey::Code(KeyCode::Digit5) => {
                            self.set_quality(PreviewQuality::UltraSpeed);
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.last_cursor_pos = Some((position.x, position.y));
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(position) = self.last_cursor_pos
                    && let Some(quality) = Self::quality_button_at(position)
                {
                    self.set_quality(quality);
                }
            }
            WindowEvent::Resized(size) => self.resize(size),
            WindowEvent::RedrawRequested => {
                self.render();
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

fn avg(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f32>() / values.len() as f32
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("https://") || value.starts_with("http://")
}

fn load_script_source(source: &str) -> Result<String, Box<dyn std::error::Error>> {
    if is_http_url(source) {
        let response = ureq::get(source).call()?;
        return Ok(response.into_string()?);
    }
    Ok(std::fs::read_to_string(source)?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut script_source = None;
    let mut print_stats = false;
    for arg in std::env::args().skip(1) {
        if arg == "--stats" || arg == "--print-stats" {
            print_stats = true;
        } else if script_source.is_none() {
            script_source = Some(arg);
        } else {
            eprintln!("unknown extra argument: {arg}");
            std::process::exit(2);
        }
    }
    let Some(script_source) = script_source else {
        eprintln!(
            "usage: cargo run -p motionloom --example wgpu_live_preview -- [--stats] path-or-url/to/main.motionloom"
        );
        std::process::exit(2);
    };
    let event_loop = EventLoop::new()?;
    let mut app = LivePreviewApp::new(script_source, print_stats)?;
    event_loop.run_app(&mut app)?;
    Ok(())
}
