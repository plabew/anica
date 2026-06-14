use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use image::RgbaImage;

use crate::common::gpu_async::{
    BufferMapAsyncFuture, DevicePoller, request_adapter_async, request_device_async,
};
use crate::dsl::GraphScript;
use crate::scene::backend::gpu::shaders::{
    WGPU_BATCH_SHAPE_SHADER, WGPU_MATTE_TEXTURE_SHADER, WGPU_POST_SHADER, WGPU_SCENE_SHADER,
};
use crate::scene::drawable::{
    GpuSceneMatteMode, GpuSceneNativeTexture, GpuScenePrimitive, GpuSceneTextureLayer,
    GpuSceneTextureSource, batch_shape_storage_bytes, batch_shape_uniform, matte_texture_uniform,
    post_blur_uniform, post_color_uniform, post_opacity_uniform, post_tint_uniform,
    texture_layer_bounds,
};
use crate::scene::render::{MotionLoomSceneRenderError, eval_scene_number};
use crate::scene::resource::{load_rgba_image_source, load_svg_source};
use crate::scene::spatial::{TextureRect, resolve_axis};

struct WgpuImageTexture {
    pub(crate) width: u32,
    pub(crate) height: u32,
    texture: std::sync::Arc<wgpu::Texture>,
}

pub(crate) struct WgpuSceneCompositor {
    device: Arc<wgpu::Device>,
    queue: wgpu::Queue,
    _poller: DevicePoller,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    matte_texture_bind_group_layout: wgpu::BindGroupLayout,
    matte_texture_pipeline: wgpu::ComputePipeline,
    shape_bind_group_layout: wgpu::BindGroupLayout,
    shape_pipeline: wgpu::ComputePipeline,
    post_bind_group_layout: wgpu::BindGroupLayout,
    post_pipeline: wgpu::ComputePipeline,
    sampler: wgpu::Sampler,
    pub(crate) width: u32,
    pub(crate) height: u32,
    tex_a: wgpu::Texture,
    tex_b: wgpu::Texture,
    readback_buffer: wgpu::Buffer,
    padded_bytes_per_row: u32,
    image_textures: HashMap<String, WgpuImageTexture>,
    asset_resolver: Arc<dyn crate::asset::AssetResolver>,
}

impl WgpuSceneCompositor {
    pub(crate) async fn new(
        width: u32,
        height: u32,
        asset_resolver: Arc<dyn crate::asset::AssetResolver>,
    ) -> Result<Self, MotionLoomSceneRenderError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = request_scene_gpu_adapter_async(&instance).await?;
        let adapter_limits = adapter.limits();
        let max_texture_dimension_2d = adapter_limits.max_texture_dimension_2d;
        if width > max_texture_dimension_2d || height > max_texture_dimension_2d {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: format!(
                    "requested scene render size {}x{} exceeds GPU max 2D texture dimension {}",
                    width, height, max_texture_dimension_2d
                ),
            });
        }

        let (device, queue) = request_device_async(
            &adapter,
            &wgpu::DeviceDescriptor {
                label: Some("anica-motionloom-scene-gpu-device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter_limits,
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            },
        )
        .await
        .map_err(|err| MotionLoomSceneRenderError::GpuRender {
            message: format!("device request failed: {err}"),
        })?;
        let device = Arc::new(device);
        let poller = DevicePoller::start(device.clone());

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("anica-motionloom-scene-gpu-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(WGPU_SCENE_SHADER)),
        });
        let matte_texture_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("anica-motionloom-scene-matte-texture-gpu-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(WGPU_MATTE_TEXTURE_SHADER)),
        });
        let shape_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("anica-motionloom-scene-shape-gpu-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(WGPU_BATCH_SHAPE_SHADER)),
        });
        let post_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("anica-motionloom-scene-post-gpu-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(WGPU_POST_SHADER)),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("anica-motionloom-scene-gpu-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shape_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("anica-motionloom-scene-shape-gpu-bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let matte_texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("anica-motionloom-scene-matte-texture-gpu-bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let post_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("anica-motionloom-scene-post-gpu-bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
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
            label: Some("anica-motionloom-scene-gpu-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("anica-motionloom-scene-gpu-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let matte_texture_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("anica-motionloom-scene-matte-texture-gpu-pipeline-layout"),
                bind_group_layouts: &[&matte_texture_bind_group_layout],
                push_constant_ranges: &[],
            });
        let matte_texture_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("anica-motionloom-scene-matte-texture-gpu-pipeline"),
                layout: Some(&matte_texture_pipeline_layout),
                module: &matte_texture_shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
        let shape_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("anica-motionloom-scene-shape-gpu-pipeline-layout"),
                bind_group_layouts: &[&shape_bind_group_layout],
                push_constant_ranges: &[],
            });
        let shape_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("anica-motionloom-scene-shape-gpu-pipeline"),
            layout: Some(&shape_pipeline_layout),
            module: &shape_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let post_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("anica-motionloom-scene-post-gpu-pipeline-layout"),
            bind_group_layouts: &[&post_bind_group_layout],
            push_constant_ranges: &[],
        });
        let post_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("anica-motionloom-scene-post-gpu-pipeline"),
            layout: Some(&post_pipeline_layout),
            module: &post_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("anica-motionloom-scene-gpu-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let tex_a = Self::make_canvas_texture(&device, width, height);
        let tex_b = Self::make_canvas_texture(&device, width, height);
        let padded_bytes_per_row = align_to_256(width.saturating_mul(4));
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-motionloom-scene-gpu-readback"),
            size: (padded_bytes_per_row as u64 * height as u64).max(4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Ok(Self {
            device,
            queue,
            _poller: poller,
            bind_group_layout,
            pipeline,
            matte_texture_bind_group_layout,
            matte_texture_pipeline,
            shape_bind_group_layout,
            shape_pipeline,
            post_bind_group_layout,
            post_pipeline,
            sampler,
            width,
            height,
            tex_a,
            tex_b,
            readback_buffer,
            padded_bytes_per_row,
            image_textures: HashMap::new(),
            asset_resolver,
        })
    }

    pub(crate) fn make_canvas_texture(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("anica-motionloom-scene-gpu-canvas"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    pub(crate) fn make_source_texture(&self, width: u32, height: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("anica-motionloom-scene-gpu-source"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    pub(crate) async fn render(
        &mut self,
        graph: &GraphScript,
        solid: [u8; 4],
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let canvas_len = (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(4);
        let mut base = vec![0u8; canvas_len];
        for pixel in base.chunks_exact_mut(4) {
            pixel.copy_from_slice(&solid);
        }
        self.write_texture_rgba(&self.tex_a, self.width, self.height, &base)?;

        let mut current_is_a = true;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("anica-motionloom-scene-gpu-encoder"),
            });
        let mut uniform_buffers = Vec::with_capacity(graph.images.len() + graph.svgs.len());

        for image_node in &graph.images {
            let opacity =
                eval_scene_number(&image_node.opacity, time_norm, time_sec)?.clamp(0.0, 1.0);
            if opacity <= 0.0001 {
                continue;
            }

            let (source_w, source_h, source_texture) = self.load_image_texture(&image_node.src)?;
            let scale =
                eval_scene_number(&image_node.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
            let target_w = ((source_w as f32) * scale).round().max(1.0);
            let target_h = ((source_h as f32) * scale).round().max(1.0);
            let x_base = resolve_axis(
                &image_node.x,
                self.width as f32,
                target_w,
                time_norm,
                time_sec,
            )?;
            let y_base = resolve_axis(
                &image_node.y,
                self.height as f32,
                target_h,
                time_norm,
                time_sec,
            )?;

            let mut uniform = [0u8; 48];
            let values = [
                self.width as f32,
                self.height as f32,
                x_base,
                y_base,
                target_w,
                target_h,
                source_w as f32,
                source_h as f32,
                opacity,
                0.0,
                0.0,
                0.0,
            ];
            for (ix, value) in values.iter().enumerate() {
                uniform[ix * 4..ix * 4 + 4].copy_from_slice(&value.to_ne_bytes());
            }
            let uniform_buffer = self.make_uniform_buffer(&uniform);

            let (src_canvas, dst_canvas) = if current_is_a {
                (&self.tex_a, &self.tex_b)
            } else {
                (&self.tex_b, &self.tex_a)
            };
            self.dispatch_image_pass(
                &mut encoder,
                src_canvas,
                &source_texture,
                dst_canvas,
                &uniform_buffer,
            );
            uniform_buffers.push(uniform_buffer);
            current_is_a = !current_is_a;
        }

        for svg_node in &graph.svgs {
            let opacity =
                eval_scene_number(&svg_node.opacity, time_norm, time_sec)?.clamp(0.0, 1.0);
            if opacity <= 0.0001 {
                continue;
            }

            let (source_w, source_h, source_texture) = self.load_svg_texture(&svg_node.src)?;
            let scale = eval_scene_number(&svg_node.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
            let target_w = ((source_w as f32) * scale).round().max(1.0);
            let target_h = ((source_h as f32) * scale).round().max(1.0);
            let x_base = resolve_axis(
                &svg_node.x,
                self.width as f32,
                target_w,
                time_norm,
                time_sec,
            )?;
            let y_base = resolve_axis(
                &svg_node.y,
                self.height as f32,
                target_h,
                time_norm,
                time_sec,
            )?;

            let mut uniform = [0u8; 48];
            let values = [
                self.width as f32,
                self.height as f32,
                x_base,
                y_base,
                target_w,
                target_h,
                source_w as f32,
                source_h as f32,
                opacity,
                0.0,
                0.0,
                0.0,
            ];
            for (ix, value) in values.iter().enumerate() {
                uniform[ix * 4..ix * 4 + 4].copy_from_slice(&value.to_ne_bytes());
            }
            let uniform_buffer = self.make_uniform_buffer(&uniform);

            let (src_canvas, dst_canvas) = if current_is_a {
                (&self.tex_a, &self.tex_b)
            } else {
                (&self.tex_b, &self.tex_a)
            };
            self.dispatch_image_pass(
                &mut encoder,
                src_canvas,
                &source_texture,
                dst_canvas,
                &uniform_buffer,
            );
            uniform_buffers.push(uniform_buffer);
            current_is_a = !current_is_a;
        }

        let final_texture = if current_is_a {
            &self.tex_a
        } else {
            &self.tex_b
        };
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: final_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let rendered = self.readback_rgba_async().await;
        drop(uniform_buffers);
        rendered
    }

    pub(crate) async fn render_scene_content(
        &mut self,
        primitives: &[GpuScenePrimitive],
        texture_layers: &[GpuSceneTextureLayer],
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let final_texture =
            self.render_scene_content_to_texture(primitives, texture_layers, [0, 0, 0, 0])?;
        self.readback_texture_rgba_async(&final_texture.texture)
            .await
    }

    pub(crate) fn render_scene_content_to_texture(
        &mut self,
        primitives: &[GpuScenePrimitive],
        texture_layers: &[GpuSceneTextureLayer],
        clear: [u8; 4],
    ) -> Result<GpuSceneNativeTexture, MotionLoomSceneRenderError> {
        let canvas_len = (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(4);
        let mut base = vec![0u8; canvas_len];
        for pixel in base.chunks_exact_mut(4) {
            pixel.copy_from_slice(&clear);
        }

        let tex_a = std::sync::Arc::new(Self::make_canvas_texture(
            &self.device,
            self.width,
            self.height,
        ));
        let tex_b = std::sync::Arc::new(Self::make_canvas_texture(
            &self.device,
            self.width,
            self.height,
        ));
        self.write_texture_rgba(&tex_a, self.width, self.height, &base)?;
        self.write_texture_rgba(&tex_b, self.width, self.height, &base)?;

        let mut current_is_a = true;
        let mut dirty_a: Option<TextureRect> = None;
        let mut dirty_b: Option<TextureRect> = None;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("anica-motionloom-scene-shape-gpu-encoder"),
            });
        let mut uniform_buffers = Vec::with_capacity(texture_layers.len() + 2);
        let mut texture_sources =
            Vec::<std::sync::Arc<wgpu::Texture>>::with_capacity(texture_layers.len());

        let shape_batch = batch_shape_storage_bytes(primitives, self.width, self.height)?;
        if shape_batch.primitive_count > 0 {
            let uniform = batch_shape_uniform(
                self.width,
                self.height,
                shape_batch.primitive_count,
                shape_batch.tile_size,
                shape_batch.tiles_x,
                shape_batch.tiles_y,
            );
            let uniform_buffer = self.make_batch_shape_uniform_buffer(&uniform);
            let storage_buffer = self.make_storage_buffer(
                "anica-motionloom-scene-shape-gpu-storage",
                &shape_batch.primitive_bytes,
            );
            let tile_range_buffer = self.make_storage_buffer(
                "anica-motionloom-scene-shape-gpu-tile-ranges",
                &shape_batch.tile_range_bytes,
            );
            let tile_index_buffer = self.make_storage_buffer(
                "anica-motionloom-scene-shape-gpu-tile-indices",
                &shape_batch.tile_index_bytes,
            );
            self.dispatch_batched_shape_pass(
                &mut encoder,
                &tex_a,
                &tex_b,
                &uniform_buffer,
                &storage_buffer,
                &tile_range_buffer,
                &tile_index_buffer,
            );
            uniform_buffers.push(uniform_buffer);
            uniform_buffers.push(storage_buffer);
            uniform_buffers.push(tile_range_buffer);
            uniform_buffers.push(tile_index_buffer);
            current_is_a = false;
            dirty_a = Some(TextureRect {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            });
            dirty_b = None;
        }

        for layer in texture_layers {
            let layer_w = layer.source.width();
            let layer_h = layer.source.height();
            if layer.opacity <= 0.0001 || layer_w == 0 || layer_h == 0 {
                continue;
            }
            let Some((bounds_x, bounds_y, bounds_w, bounds_h)) =
                texture_layer_bounds(layer.transform, layer_w, layer_h, self.width, self.height)
            else {
                continue;
            };
            if bounds_w == 0 || bounds_h == 0 {
                continue;
            }

            let source_texture = match &layer.source {
                GpuSceneTextureSource::Cpu(image) => {
                    let texture = std::sync::Arc::new(
                        self.make_source_texture(image.width().max(1), image.height().max(1)),
                    );
                    self.write_texture_rgba(
                        &texture,
                        image.width().max(1),
                        image.height().max(1),
                        image.as_raw(),
                    )?;
                    texture_sources.push(texture.clone());
                    texture
                }
                GpuSceneTextureSource::Gpu(texture) => texture.texture.clone(),
            };
            let (matte_texture, matte_w, matte_h, matte_mode, invert_matte) =
                if let Some(matte) = layer.matte.as_ref() {
                    (
                        matte.texture.texture.clone(),
                        matte.texture.width,
                        matte.texture.height,
                        matte.mode,
                        matte.invert,
                    )
                } else {
                    (
                        source_texture.clone(),
                        layer_w,
                        layer_h,
                        GpuSceneMatteMode::None,
                        false,
                    )
                };
            let uniform = matte_texture_uniform(
                layer,
                self.width,
                self.height,
                bounds_x,
                bounds_y,
                bounds_w,
                bounds_h,
                layer_w,
                layer_h,
                matte_w,
                matte_h,
                matte_mode,
                invert_matte,
            )?;
            let uniform_buffer = self.make_matte_texture_uniform_buffer(&uniform);
            let (src_canvas, dst_canvas) = if current_is_a {
                (&tex_a, &tex_b)
            } else {
                (&tex_b, &tex_a)
            };
            let dst_dirty = if current_is_a { dirty_b } else { dirty_a };
            if let Some(rect) = dst_dirty {
                self.copy_texture_rect(&mut encoder, src_canvas, dst_canvas, rect);
            }
            self.dispatch_matte_texture_pass(
                &mut encoder,
                src_canvas,
                &source_texture,
                &matte_texture,
                dst_canvas,
                &uniform_buffer,
                bounds_w,
                bounds_h,
            );
            uniform_buffers.push(uniform_buffer);
            let changed = TextureRect {
                x: bounds_x,
                y: bounds_y,
                width: bounds_w,
                height: bounds_h,
            };
            if current_is_a {
                dirty_b = None;
                dirty_a = union_texture_rect(dirty_a, changed);
            } else {
                dirty_a = None;
                dirty_b = union_texture_rect(dirty_b, changed);
            }
            current_is_a = !current_is_a;
        }

        let final_texture = if current_is_a { tex_a } else { tex_b };
        self.queue.submit([encoder.finish()]);
        drop(uniform_buffers);
        drop(texture_sources);
        Ok(GpuSceneNativeTexture {
            texture: final_texture,
            width: self.width,
            height: self.height,
        })
    }

    pub(crate) async fn apply_gpu_blur_passes(
        &mut self,
        input: &RgbaImage,
        passes: &[(bool, f32)],
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        if passes.is_empty() {
            return Ok(input.clone());
        }
        if input.width() != self.width || input.height() != self.height {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: format!(
                    "post-pass input size {}x{} does not match GPU compositor {}x{}",
                    input.width(),
                    input.height(),
                    self.width,
                    self.height
                ),
            });
        }

        self.write_texture_rgba(&self.tex_a, self.width, self.height, input.as_raw())?;

        let mut current_is_a = true;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("anica-motionloom-scene-post-gpu-encoder"),
            });
        let mut uniform_buffers = Vec::with_capacity(passes.len());

        for (horizontal, sigma) in passes {
            let uniform = post_blur_uniform(self.width, self.height, *horizontal, *sigma);
            let uniform_buffer = self.make_post_uniform_buffer(&uniform);
            let (src_canvas, dst_canvas) = if current_is_a {
                (&self.tex_a, &self.tex_b)
            } else {
                (&self.tex_b, &self.tex_a)
            };
            self.dispatch_post_pass(&mut encoder, src_canvas, dst_canvas, &uniform_buffer);
            uniform_buffers.push(uniform_buffer);
            current_is_a = !current_is_a;
        }

        let final_texture = if current_is_a {
            &self.tex_a
        } else {
            &self.tex_b
        };
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: final_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let rendered = self.readback_rgba_async().await;
        drop(uniform_buffers);
        rendered
    }

    pub(crate) async fn apply_gpu_opacity_pass(
        &mut self,
        input: &RgbaImage,
        opacity: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        if input.width() != self.width || input.height() != self.height {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: format!(
                    "opacity input size {}x{} does not match GPU compositor {}x{}",
                    input.width(),
                    input.height(),
                    self.width,
                    self.height
                ),
            });
        }

        self.write_texture_rgba(&self.tex_a, self.width, self.height, input.as_raw())?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("anica-motionloom-scene-opacity-gpu-encoder"),
            });
        let uniform = post_opacity_uniform(self.width, self.height, opacity);
        let uniform_buffer = self.make_post_uniform_buffer(&uniform);
        self.dispatch_post_pass(&mut encoder, &self.tex_a, &self.tex_b, &uniform_buffer);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.tex_b,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let rendered = self.readback_rgba_async().await;
        drop(uniform_buffer);
        rendered
    }

    pub(crate) fn apply_gpu_blur_texture(
        &mut self,
        input: &GpuSceneNativeTexture,
        passes: &[(bool, f32)],
    ) -> Result<GpuSceneNativeTexture, MotionLoomSceneRenderError> {
        if passes.is_empty() {
            return Ok(input.clone());
        }
        let width = input.width.max(1);
        let height = input.height.max(1);

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("anica-motionloom-scene-post-texture-gpu-encoder"),
            });
        let mut uniform_buffers = Vec::with_capacity(passes.len());
        let mut temp_textures = Vec::<std::sync::Arc<wgpu::Texture>>::with_capacity(passes.len());
        let mut current = input.texture.clone();

        for (horizontal, sigma) in passes {
            let uniform = post_blur_uniform(width, height, *horizontal, *sigma);
            let uniform_buffer = self.make_post_uniform_buffer(&uniform);
            let dst = std::sync::Arc::new(Self::make_canvas_texture(&self.device, width, height));
            self.dispatch_post_pass_sized(
                &mut encoder,
                &current,
                &dst,
                &uniform_buffer,
                width,
                height,
            );
            uniform_buffers.push(uniform_buffer);
            current = dst.clone();
            temp_textures.push(dst);
        }

        self.queue.submit([encoder.finish()]);
        drop(uniform_buffers);
        Ok(GpuSceneNativeTexture {
            texture: current,
            width,
            height,
        })
    }

    pub(crate) fn apply_gpu_tint_texture(
        &mut self,
        input: &GpuSceneNativeTexture,
        color: [u8; 4],
        intensity: f32,
    ) -> GpuSceneNativeTexture {
        let width = input.width.max(1);
        let height = input.height.max(1);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("anica-motionloom-scene-post-tint-texture-gpu-encoder"),
            });
        let uniform = post_tint_uniform(width, height, color, intensity);
        let uniform_buffer = self.make_post_uniform_buffer(&uniform);
        let dst = std::sync::Arc::new(Self::make_canvas_texture(&self.device, width, height));
        self.dispatch_post_pass_sized(
            &mut encoder,
            &input.texture,
            &dst,
            &uniform_buffer,
            width,
            height,
        );
        self.queue.submit([encoder.finish()]);
        drop(uniform_buffer);
        GpuSceneNativeTexture {
            texture: dst,
            width,
            height,
        }
    }

    pub(crate) fn upload_gpu_rgba_texture(
        &mut self,
        image: &RgbaImage,
    ) -> Result<GpuSceneNativeTexture, MotionLoomSceneRenderError> {
        let width = image.width().max(1);
        let height = image.height().max(1);
        let texture = std::sync::Arc::new(self.make_source_texture(width, height));
        self.write_texture_rgba(&texture, width, height, image.as_raw())?;
        Ok(GpuSceneNativeTexture {
            texture,
            width,
            height,
        })
    }

    pub(crate) fn apply_gpu_color_texture(
        &mut self,
        input: &GpuSceneNativeTexture,
        brightness: f32,
        contrast: f32,
        saturation: f32,
    ) -> Result<GpuSceneNativeTexture, MotionLoomSceneRenderError> {
        if input.width != self.width || input.height != self.height {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: format!(
                    "texture color input size {}x{} does not match GPU compositor {}x{}",
                    input.width, input.height, self.width, self.height
                ),
            });
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("anica-motionloom-scene-post-color-texture-gpu-encoder"),
            });
        let uniform = post_color_uniform(self.width, self.height, brightness, contrast, saturation);
        let uniform_buffer = self.make_post_uniform_buffer(&uniform);
        let dst = std::sync::Arc::new(Self::make_canvas_texture(
            &self.device,
            self.width,
            self.height,
        ));
        self.dispatch_post_pass(&mut encoder, &input.texture, &dst, &uniform_buffer);
        self.queue.submit([encoder.finish()]);
        drop(uniform_buffer);
        Ok(GpuSceneNativeTexture {
            texture: dst,
            width: self.width,
            height: self.height,
        })
    }

    pub(crate) fn make_uniform_buffer(&self, uniform: &[u8; 48]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-motionloom-scene-gpu-uniform"),
            size: uniform.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .copy_from_slice(uniform);
        buffer.unmap();
        buffer
    }

    pub(crate) fn make_matte_texture_uniform_buffer(&self, uniform: &[u8; 96]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-motionloom-scene-matte-texture-gpu-uniform"),
            size: uniform.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .copy_from_slice(uniform);
        buffer.unmap();
        buffer
    }

    pub(crate) fn make_post_uniform_buffer(&self, uniform: &[u8; 32]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-motionloom-scene-post-gpu-uniform"),
            size: uniform.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .copy_from_slice(uniform);
        buffer.unmap();
        buffer
    }

    pub(crate) fn make_batch_shape_uniform_buffer(&self, uniform: &[u8; 32]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-motionloom-scene-batch-shape-gpu-uniform"),
            size: uniform.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .copy_from_slice(uniform);
        buffer.unmap();
        buffer
    }

    pub(crate) fn make_storage_buffer(&self, label: &'static str, data: &[u8]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: data.len().max(4) as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        });
        buffer
            .slice(..data.len() as u64)
            .get_mapped_range_mut()
            .copy_from_slice(data);
        buffer.unmap();
        buffer
    }

    pub(crate) fn write_texture_rgba(
        &self,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), MotionLoomSceneRenderError> {
        if width == 0 || height == 0 {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: "cannot upload zero-sized RGBA texture".to_string(),
            });
        }
        let row_bytes =
            width
                .checked_mul(4)
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: format!("RGBA texture row is too wide: {width} px"),
                })?;
        let expected_len = (row_bytes as usize)
            .checked_mul(height as usize)
            .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                message: format!("RGBA texture is too large: {width}x{height}"),
            })?;
        if rgba.len() < expected_len {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: format!(
                    "RGBA texture upload has insufficient data: expected {expected_len} bytes for {width}x{height}, got {} bytes",
                    rgba.len()
                ),
            });
        }

        // Text layers are arbitrary-width CPU bitmaps. Padding rows keeps uploads
        // valid across stricter backends while preserving exact texel content.
        const ROW_ALIGNMENT: u32 = 256;
        let padded_row_bytes = row_bytes.div_ceil(ROW_ALIGNMENT) * ROW_ALIGNMENT;
        let upload: Cow<'_, [u8]> = if padded_row_bytes == row_bytes {
            Cow::Borrowed(&rgba[..expected_len])
        } else {
            let mut padded = vec![0u8; (padded_row_bytes as usize) * (height as usize)];
            for row in 0..height as usize {
                let src_start = row * row_bytes as usize;
                let dst_start = row * padded_row_bytes as usize;
                padded[dst_start..dst_start + row_bytes as usize]
                    .copy_from_slice(&rgba[src_start..src_start + row_bytes as usize]);
            }
            Cow::Owned(padded)
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &upload,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row_bytes),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    pub(crate) fn copy_texture_rect(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        src_texture: &wgpu::Texture,
        dst_texture: &wgpu::Texture,
        rect: TextureRect,
    ) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: src_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: rect.x,
                    y: rect.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: dst_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: rect.x,
                    y: rect.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: rect.width,
                height: rect.height,
                depth_or_array_layers: 1,
            },
        );
    }

    pub(crate) fn dispatch_image_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        base_texture: &wgpu::Texture,
        image_texture: &wgpu::Texture,
        out_texture: &wgpu::Texture,
        uniform_buffer: &wgpu::Buffer,
    ) {
        let base_view = base_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let image_view = image_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let out_view = out_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anica-motionloom-scene-gpu-bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&base_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&image_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&out_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("anica-motionloom-scene-gpu-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(
            self.width.div_ceil(16).max(1),
            self.height.div_ceil(16).max(1),
            1,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch_batched_shape_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        base_texture: &wgpu::Texture,
        out_texture: &wgpu::Texture,
        uniform_buffer: &wgpu::Buffer,
        primitive_buffer: &wgpu::Buffer,
        tile_range_buffer: &wgpu::Buffer,
        tile_index_buffer: &wgpu::Buffer,
    ) {
        let base_view = base_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let out_view = out_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anica-motionloom-scene-shape-gpu-bg"),
            layout: &self.shape_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&base_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&out_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: primitive_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: tile_range_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: tile_index_buffer.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("anica-motionloom-scene-shape-gpu-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.shape_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(
            self.width.div_ceil(16).max(1),
            self.height.div_ceil(16).max(1),
            1,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch_matte_texture_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        base_texture: &wgpu::Texture,
        image_texture: &wgpu::Texture,
        matte_texture: &wgpu::Texture,
        out_texture: &wgpu::Texture,
        uniform_buffer: &wgpu::Buffer,
        bounds_w: u32,
        bounds_h: u32,
    ) {
        let base_view = base_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let image_view = image_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let matte_view = matte_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let out_view = out_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anica-motionloom-scene-matte-texture-gpu-bg"),
            layout: &self.matte_texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&base_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&image_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&matte_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&out_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("anica-motionloom-scene-matte-texture-gpu-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.matte_texture_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(
            bounds_w.div_ceil(16).max(1),
            bounds_h.div_ceil(16).max(1),
            1,
        );
    }

    pub(crate) fn dispatch_post_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        base_texture: &wgpu::Texture,
        out_texture: &wgpu::Texture,
        uniform_buffer: &wgpu::Buffer,
    ) {
        self.dispatch_post_pass_sized(
            encoder,
            base_texture,
            out_texture,
            uniform_buffer,
            self.width,
            self.height,
        );
    }

    pub(crate) fn dispatch_post_pass_sized(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        base_texture: &wgpu::Texture,
        out_texture: &wgpu::Texture,
        uniform_buffer: &wgpu::Buffer,
        width: u32,
        height: u32,
    ) {
        let base_view = base_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let out_view = out_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anica-motionloom-scene-post-gpu-bg"),
            layout: &self.post_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&base_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&out_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("anica-motionloom-scene-post-gpu-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.post_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(width.div_ceil(16).max(1), height.div_ceil(16).max(1), 1);
    }

    pub(crate) fn load_image_texture(
        &mut self,
        src: &str,
    ) -> Result<(u32, u32, std::sync::Arc<wgpu::Texture>), MotionLoomSceneRenderError> {
        if !self.image_textures.contains_key(src) {
            let image = load_rgba_image_source(src, self.asset_resolver.as_ref())?;
            let (width, height) = image.dimensions();
            let texture = self.make_source_texture(width.max(1), height.max(1));
            self.write_texture_rgba(&texture, width.max(1), height.max(1), image.as_raw())?;
            self.image_textures.insert(
                src.to_string(),
                WgpuImageTexture {
                    width: width.max(1),
                    height: height.max(1),
                    texture: std::sync::Arc::new(texture),
                },
            );
        }
        let source = self
            .image_textures
            .get(src)
            .expect("GPU image texture inserted before lookup");
        Ok((source.width, source.height, source.texture.clone()))
    }

    pub(crate) fn load_svg_texture(
        &mut self,
        src: &str,
    ) -> Result<(u32, u32, std::sync::Arc<wgpu::Texture>), MotionLoomSceneRenderError> {
        let cache_key = format!("svg:{src}");
        if !self.image_textures.contains_key(&cache_key) {
            let image = load_svg_source(src, self.asset_resolver.as_ref())?;
            let (width, height) = image.dimensions();
            let texture = self.make_source_texture(width.max(1), height.max(1));
            self.write_texture_rgba(&texture, width.max(1), height.max(1), image.as_raw())?;
            self.image_textures.insert(
                cache_key.clone(),
                WgpuImageTexture {
                    width: width.max(1),
                    height: height.max(1),
                    texture: std::sync::Arc::new(texture),
                },
            );
        }
        let source = self
            .image_textures
            .get(&cache_key)
            .expect("GPU SVG texture inserted before lookup");
        Ok((source.width, source.height, source.texture.clone()))
    }

    pub(crate) async fn readback_texture_rgba_async(
        &self,
        texture: &wgpu::Texture,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("anica-motionloom-scene-gpu-readback-encoder"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        self.readback_rgba_async().await
    }

    pub(crate) async fn readback_rgba_async(
        &self,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let slice = self.readback_buffer.slice(..);
        BufferMapAsyncFuture::new(&self._poller, &self.readback_buffer)
            .await
            .map_err(|err| MotionLoomSceneRenderError::GpuRender {
                message: format!("readback map failed: {err}"),
            })?;

        let mapped = slice.get_mapped_range();
        let row_bytes = self.width as usize * 4;
        let padded_row = self.padded_bytes_per_row as usize;
        let mut out = vec![0u8; row_bytes * self.height as usize];
        for row in 0..self.height as usize {
            let src_off = row * padded_row;
            let dst_off = row * row_bytes;
            out[dst_off..dst_off + row_bytes]
                .copy_from_slice(&mapped[src_off..src_off + row_bytes]);
        }
        drop(mapped);
        self.readback_buffer.unmap();
        RgbaImage::from_raw(self.width, self.height, out).ok_or_else(|| {
            MotionLoomSceneRenderError::GpuRender {
                message: "failed to build RGBA image from GPU readback".to_string(),
            }
        })
    }
}

async fn request_scene_gpu_adapter_async(
    instance: &wgpu::Instance,
) -> Result<wgpu::Adapter, MotionLoomSceneRenderError> {
    let first_preference =
        wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::HighPerformance);
    let preferences = [
        first_preference,
        wgpu::PowerPreference::HighPerformance,
        wgpu::PowerPreference::LowPower,
        wgpu::PowerPreference::None,
    ];

    let mut errors = Vec::<String>::new();
    for (ix, preference) in preferences.iter().copied().enumerate() {
        if preferences[..ix].contains(&preference) {
            continue;
        }
        // Stay GPU-first by falling back between GPU adapter classes, not to CPU rendering.
        match request_adapter_async(
            instance,
            &wgpu::RequestAdapterOptions {
                power_preference: preference,
                force_fallback_adapter: false,
                compatible_surface: None,
            },
        )
        .await
        {
            Ok(adapter) => return Ok(adapter),
            Err(err) => errors.push(format!("{preference:?}: {err}")),
        }
    }

    Err(MotionLoomSceneRenderError::GpuRender {
        message: format!(
            "no compatible GPU adapter was available for scene rendering ({})",
            errors.join("; ")
        ),
    })
}

fn align_to_256(v: u32) -> u32 {
    const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    v.div_ceil(ALIGN) * ALIGN
}

fn union_texture_rect(current: Option<TextureRect>, next: TextureRect) -> Option<TextureRect> {
    if next.width == 0 || next.height == 0 {
        return current;
    }
    let Some(current) = current else {
        return Some(next);
    };
    let x0 = current.x.min(next.x);
    let y0 = current.y.min(next.y);
    let x1 = current
        .x
        .saturating_add(current.width)
        .max(next.x.saturating_add(next.width));
    let y1 = current
        .y
        .saturating_add(current.height)
        .max(next.y.saturating_add(next.height));
    Some(TextureRect {
        x: x0,
        y: y0,
        width: x1.saturating_sub(x0),
        height: y1.saturating_sub(y0),
    })
}
