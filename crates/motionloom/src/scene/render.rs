// =========================================
// crates/motionloom/src/scene/render.rs

use std::collections::{HashMap, HashSet};
use std::fs;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use crate::dsl::GraphScript;
use crate::process::model::PassNode;
use crate::process::runtime::eval_time_expr;

#[cfg(not(target_arch = "wasm32"))]
use crate::scene::backend::encoding::scene_encoder_args;

use crate::asset::{AssetResolver, PathAssetResolver};
pub use crate::scene::backend::encoding::{
    SceneRenderProfile, SceneRenderProgress, next_scene_output_path,
    next_scene_output_path_for_profile,
};
use crate::scene::backend::gpu::WgpuSceneCompositor;
use crate::scene::backend::sizing::{
    fit_logical_canvas_to_output, graph_logical_render_size, graph_output_size,
    render_size_root_transform,
};
use crate::scene::compile::{
    graph_has_rich_scene_tree, scene_nodes_contain_image_or_svg, scene_nodes_for_present,
    scene_nodes_require_cpu_scene_compositing,
};
use crate::scene::composition::{
    apply_alpha_mask, apply_alpha_mask_with_invert, apply_box_blur_pass, apply_deform_grid,
    apply_hsla_pass, apply_layer_effects, apply_over_pass, apply_scene_filter_step,
    apply_scene_post_pass, blend_pixel, build_scene_bloom_prefilter, composite_layer,
    composite_layer_affine, composite_layer_affine_blend, composite_layer_affine_blend_clipped,
    composite_layer_affine_clipped, composite_scene_bloom, composite_transformed_layer,
    composite_transformed_layer_anchored, draw_rgba_image, is_color_key_alpha_effect,
    pass_param_expr, scene_post_bloom_params, scene_post_blur_passes,
};
use crate::scene::domain::apply_action_graph_at_time;
use crate::scene::drawable::{
    CpuSceneOverlay, EvaluatedShadow, GPU_SHAPE_CIRCLE_FILL, GPU_SHAPE_CIRCLE_SHADOW,
    GPU_SHAPE_CIRCLE_STROKE, GPU_SHAPE_LINE, GPU_SHAPE_RECT_FILL, GPU_SHAPE_RECT_SHADOW,
    GPU_SHAPE_RECT_STROKE, GPU_SHAPE_TRIANGLE_FILL, GpuSceneGradientPaint, GpuSceneMatteMode,
    GpuSceneNativeAssets, GpuSceneNativeTexture, GpuScenePrimitive, GpuSceneTextRequest,
    GpuSceneTextureLayer, GpuSceneTextureMatte, GpuSceneTextureSource, PaintBounds, Point2,
    ResolvedPaint, SceneBlendMode, StrokeStyle, StrokeTexture, affine_uniform_scale,
    describe_cpu_scene_overlays, draw_circle, draw_circle_paint, draw_circle_shadow,
    draw_circle_stroke, draw_line_segment_styled, draw_rect_shadow, draw_rounded_rect,
    draw_rounded_rect_paint, draw_rounded_rect_stroke, draw_transformed_filled_polylines,
    draw_transformed_filled_polylines_paint, draw_transformed_trimmed_polylines_styled,
    eval_line_stroke_style, eval_path_d, eval_path_stroke_style, eval_polyline_stroke_style,
    evaluate_shadow, evaluate_trim, face_jaw_to_path_node, gpu_matte_mode, gpu_solid_primitive,
    gradient_ref_id, id_suffix, is_gpu_native_blend, is_none_paint, parse_color, parse_paint,
    parse_path_subpaths, parse_polyline_points, parse_scene_blend, point_distance,
    raster_texture_layer, resolve_gradient_paint, scene_mask_mode_inverts, solid_canvas,
    stroke_hash_signed, stroke_taper_pressure, stroke_texture_copy_count, stroke_texture_seed,
    stroke_texture_variant, trimmed_polyline_segments_with_progress,
};
use crate::scene::dsl::{ImageNode, SvgNode};
use crate::scene::model::{
    CameraNode, CharacterNode, CircleNode, DefsNode, FaceJawNode, FilterDef, FontDef, GradientDef,
    GroupNode, LineNode, MaskNode, PaletteNode, PartNode, PathNode, PixelGridNode, PolylineNode,
    PrecomposeNode, RectNode, RepeatNode, SceneLayerNode, SceneNode, UseNode,
};
pub use crate::scene::resource::{clear_scene_asset_roots, set_scene_asset_roots};
use crate::scene::resource::{
    collect_graph_component_defs, collect_graph_filter_defs, collect_graph_font_defs,
    collect_graph_gradient_defs, collect_graph_mask_defs, collect_graph_palette_defs,
    collect_graph_precompose_defs, default_world_asset_root, load_extra_fonts,
    load_rgba_image_source, load_svg_source, resolve_local_scene_asset_path,
};
use crate::scene::spatial::{
    Affine2, CameraRect, EvaluatedDeformGrid, active_scene_camera_from_tracks, affine_is_identity,
    camera_transform, camera_viewport, camera_world_bounds, eval_group_deform_grid,
    is_scene_camera_track, is_scene_world_track, resolve_axis, scene_character_local_transform,
    scene_circle_local_transform, scene_group_local_transform, scene_layer_local_transform,
    scene_line_local_transform, scene_path_local_transform, scene_polyline_local_transform,
    scene_rect_local_transform, scene_text_local_transform, scene_use_local_transform,
    transform_and_deform_point, transform_and_deform_subpaths, transform_deform_grid,
};
use crate::scene::text::TextNode;
use crate::scene::text::{
    TextAnimatorRasterParams, TextRasterizedLayer, apply_text_layer_effects,
    draw_text_buffer_with_animators, prepare_text_layout_for_value, stroke_layer_from_alpha,
    text_bounds, text_layer_effect_spec,
};
use crate::scene::timeline::{
    eval_repeat_count, scene_layer_source_time, scene_sequence_local_time,
};
use crate::world::{WorldFrameRenderer, parse_world_graph_script};
use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};
use image::{Rgba, RgbaImage, imageops::FilterType};

mod depth;

pub use crate::scene::error::{MotionLoomSceneRenderError, SceneRenderError};
use depth::{
    SceneDepthContext, scene_depth_track_sort_key, scene_layer_effective_z_depth,
    scene_z_depth_transform,
};

#[allow(dead_code)]
pub async fn render_scene_graph_to_video(
    ffmpeg_bin: &str,
    graph: &GraphScript,
    output_path: &Path,
) -> Result<(), MotionLoomSceneRenderError> {
    render_scene_graph_to_video_with_progress(
        ffmpeg_bin,
        graph,
        output_path,
        SceneRenderProfile::Cpu,
        0,
        |_progress| {},
    )
    .await
}

#[cfg_attr(target_arch = "wasm32", allow(unused_mut, unused_variables))]
pub async fn render_scene_graph_to_video_with_progress<F>(
    ffmpeg_bin: &str,
    graph: &GraphScript,
    output_path: &Path,
    profile: SceneRenderProfile,
    progress_every_frames: u32,
    mut progress_callback: F,
) -> Result<(), MotionLoomSceneRenderError>
where
    F: FnMut(SceneRenderProgress),
{
    validate_scene_graph(graph)?;
    if profile.is_png_sequence() {
        return render_scene_graph_to_png_sequence_with_progress(
            graph,
            output_path,
            profile,
            progress_every_frames,
            progress_callback,
        )
        .await;
    }

    #[cfg(target_arch = "wasm32")]
    {
        Err(MotionLoomSceneRenderError::VideoExportNotAvailable {
            message: "FFmpeg video export is not available in WASM".to_string(),
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        use crate::export::{FfmpegVideoEncoder, VideoEncoder};

        let (w, h) = graph_output_size(graph);
        let fps = graph.fps.max(1.0);
        let duration_sec = (graph.duration_ms as f32 / 1000.0).max(1.0 / fps);
        let total_frames = ((duration_sec * fps).round() as u32).max(1);
        let encoder_args = scene_encoder_args(profile);
        let mut renderer = SceneFrameRenderer::new_for_profile(profile).await;
        progress_callback(SceneRenderProgress {
            rendered_frames: 0,
            total_frames,
        });

        // Render the first frame before starting the encoder. This catches GPU
        // adapter and scene render errors before a long-lived encoder is opened.
        let first_image = renderer.render_frame(graph, 0).await?;

        let mut encoder =
            FfmpegVideoEncoder::new(ffmpeg_bin, output_path).with_encoder_args(encoder_args);
        encoder.begin(w, h, fps)?;

        for frame in 0..total_frames {
            let rendered_image;
            let image = if frame == 0 {
                &first_image
            } else {
                rendered_image = renderer.render_frame(graph, frame).await?;
                &rendered_image
            };
            encoder.push_frame(frame, image.as_raw())?;
            let rendered_frames = frame + 1;
            if rendered_frames == total_frames
                || (progress_every_frames > 0 && rendered_frames % progress_every_frames == 0)
            {
                progress_callback(SceneRenderProgress {
                    rendered_frames,
                    total_frames,
                });
            }
        }
        encoder.finish()?;
        Ok(())
    }
}

async fn render_scene_graph_to_png_sequence_with_progress<F>(
    graph: &GraphScript,
    output_dir: &Path,
    profile: SceneRenderProfile,
    progress_every_frames: u32,
    mut progress_callback: F,
) -> Result<(), MotionLoomSceneRenderError>
where
    F: FnMut(SceneRenderProgress),
{
    fs::create_dir_all(output_dir).map_err(|source| {
        MotionLoomSceneRenderError::CreateOutputDir {
            path: output_dir.to_path_buf(),
            source,
        }
    })?;

    let fps = graph.fps.max(1.0);
    let duration_sec = (graph.duration_ms as f32 / 1000.0).max(1.0 / fps);
    let total_frames = ((duration_sec * fps).round() as u32).max(1);
    let mut renderer = SceneFrameRenderer::new_for_profile(profile).await;
    progress_callback(SceneRenderProgress {
        rendered_frames: 0,
        total_frames,
    });

    for frame in 0..total_frames {
        let image = renderer.render_frame(graph, frame).await?;
        let path = output_dir.join(format!("frame_{frame:06}.png"));
        image
            .save(&path)
            .map_err(|source| MotionLoomSceneRenderError::SavePngFrame { path, source })?;
        let rendered_frames = frame + 1;
        if rendered_frames == total_frames
            || (progress_every_frames > 0 && rendered_frames % progress_every_frames == 0)
        {
            progress_callback(SceneRenderProgress {
                rendered_frames,
                total_frames,
            });
        }
    }

    Ok(())
}

pub async fn render_scene_graph_frame(
    graph: &GraphScript,
    frame: u32,
    profile: SceneRenderProfile,
) -> Result<RgbaImage, MotionLoomSceneRenderError> {
    validate_scene_graph(graph)?;
    let mut renderer = SceneFrameRenderer::new_for_profile(profile).await;
    renderer.render_frame(graph, frame).await
}

/// Render a scene frame using a caller-provided asset resolver.
///
/// This allows WASM hosts and tests to supply assets from memory instead of
/// relying on the filesystem.
pub async fn render_scene_graph_frame_with_resolver(
    graph: &GraphScript,
    frame: u32,
    profile: SceneRenderProfile,
    asset_resolver: Arc<dyn AssetResolver>,
) -> Result<RgbaImage, MotionLoomSceneRenderError> {
    validate_scene_graph(graph)?;
    let mut renderer =
        SceneFrameRenderer::new_for_profile_with_resolver(profile, asset_resolver).await;
    renderer.render_frame(graph, frame).await
}

pub async fn render_scene_frame(
    graph: &GraphScript,
    frame: u32,
    profile: SceneRenderProfile,
) -> Result<RgbaImage, SceneRenderError> {
    render_scene_graph_frame(graph, frame, profile).await
}

pub struct SceneRenderer {
    inner: SceneFrameRenderer,
}

impl SceneRenderer {
    pub async fn new(profile: SceneRenderProfile) -> Result<Self, SceneRenderError> {
        Ok(Self {
            inner: SceneFrameRenderer::new_for_profile(profile).await,
        })
    }

    pub async fn with_resolver(
        profile: SceneRenderProfile,
        asset_resolver: Arc<dyn AssetResolver>,
    ) -> Result<Self, SceneRenderError> {
        Ok(Self {
            inner: SceneFrameRenderer::new_for_profile_with_resolver(profile, asset_resolver).await,
        })
    }

    pub async fn render_frame(
        &mut self,
        graph: &GraphScript,
        frame: u32,
    ) -> Result<RgbaImage, SceneRenderError> {
        validate_scene_graph(graph)?;
        self.inner.render_frame(graph, frame).await
    }

    /// Render a GPU-native scene frame directly into a browser canvas.
    ///
    /// This WASM-only path presents the compositor texture to the canvas
    /// surface and avoids CPU readback. It is intentionally strict for now:
    /// unsupported scene graphs return a GPU render error instead of falling
    /// back silently.
    #[cfg(target_arch = "wasm32")]
    pub async fn render_frame_to_canvas(
        &mut self,
        graph: &GraphScript,
        frame: u32,
        canvas: web_sys::HtmlCanvasElement,
    ) -> Result<(), SceneRenderError> {
        validate_scene_graph(graph)?;
        self.inner
            .render_frame_to_canvas(graph, frame, canvas)
            .await
    }

    /// Draw a solid WebGPU color into a browser canvas for surface debugging.
    #[cfg(target_arch = "wasm32")]
    pub async fn debug_solid_to_canvas(
        &mut self,
        canvas: web_sys::HtmlCanvasElement,
        width: u32,
        height: u32,
        color: [f64; 4],
    ) -> Result<(), SceneRenderError> {
        self.inner
            .debug_solid_to_canvas(canvas, width, height, color)
            .await
    }

    /// Upload a solid texture and present it to a browser canvas for debugging.
    #[cfg(target_arch = "wasm32")]
    pub async fn debug_uploaded_texture_to_canvas(
        &mut self,
        canvas: web_sys::HtmlCanvasElement,
        width: u32,
        height: u32,
        color: [u8; 4],
    ) -> Result<(), SceneRenderError> {
        self.inner
            .debug_uploaded_texture_to_canvas(canvas, width, height, color)
            .await
    }

    /// Render an empty scene texture with a white clear color and present it.
    #[cfg(target_arch = "wasm32")]
    pub async fn debug_empty_scene_texture_to_canvas(
        &mut self,
        canvas: web_sys::HtmlCanvasElement,
        width: u32,
        height: u32,
    ) -> Result<(), SceneRenderError> {
        self.inner
            .debug_empty_scene_texture_to_canvas(canvas, width, height)
            .await
    }
}

fn validate_scene_graph(graph: &GraphScript) -> Result<(), MotionLoomSceneRenderError> {
    if !graph.has_scene_nodes() {
        return Err(MotionLoomSceneRenderError::EmptyScene);
    }
    validate_scene_gradient_refs(graph)?;
    Ok(())
}

fn validate_scene_gradient_refs(graph: &GraphScript) -> Result<(), MotionLoomSceneRenderError> {
    let mut gradient_defs = HashMap::new();
    collect_graph_gradient_defs(graph, &mut gradient_defs);

    let mut refs = Vec::new();
    collect_scene_gradient_refs(&graph.scene_nodes, &mut refs);
    for scene in &graph.scenes {
        collect_scene_gradient_refs(&scene.children, &mut refs);
    }

    let mut seen = HashSet::new();
    for (paint, id) in refs {
        if !seen.insert((paint.clone(), id.clone())) {
            continue;
        }
        if !gradient_defs.contains_key(&id) {
            return Err(MotionLoomSceneRenderError::InvalidPaint {
                value: paint,
                message: format!("gradient reference not found: {id}"),
            });
        }
    }
    Ok(())
}

fn collect_scene_gradient_refs(nodes: &[SceneNode], out: &mut Vec<(String, String)>) {
    for node in nodes {
        match node {
            SceneNode::Defs(defs) => collect_defs_gradient_refs(defs, out),
            SceneNode::Timeline(timeline) => collect_scene_gradient_refs(&timeline.children, out),
            SceneNode::Track(track) => collect_scene_gradient_refs(&track.children, out),
            SceneNode::Sequence(sequence) => collect_scene_gradient_refs(&sequence.children, out),
            SceneNode::Chain(chain) => collect_scene_gradient_refs(&chain.children, out),
            SceneNode::Text(text) => collect_text_gradient_refs(text, out),
            SceneNode::Rect(rect) => {
                collect_paint_gradient_ref(&rect.color, out);
                collect_optional_paint_gradient_ref(rect.stroke.as_deref(), out);
            }
            SceneNode::Circle(circle) => {
                collect_paint_gradient_ref(&circle.color, out);
                collect_optional_paint_gradient_ref(circle.stroke.as_deref(), out);
            }
            SceneNode::Line(line) => collect_paint_gradient_ref(&line.color, out),
            SceneNode::Polyline(polyline) => collect_paint_gradient_ref(&polyline.stroke, out),
            SceneNode::Path(path) => {
                collect_paint_gradient_ref(&path.stroke, out);
                collect_optional_paint_gradient_ref(path.fill.as_deref(), out);
            }
            SceneNode::FaceJaw(face_jaw) => {
                collect_paint_gradient_ref(&face_jaw.stroke, out);
                collect_optional_paint_gradient_ref(face_jaw.fill.as_deref(), out);
            }
            SceneNode::Shadow(shadow) => collect_paint_gradient_ref(&shadow.color, out),
            SceneNode::Group(group) => collect_scene_gradient_refs(&group.children, out),
            SceneNode::Part(part) => collect_scene_gradient_refs(&part.children, out),
            SceneNode::Repeat(repeat) => collect_scene_gradient_refs(&repeat.children, out),
            SceneNode::Mask(mask) => collect_scene_gradient_refs(&mask.children, out),
            SceneNode::Precompose(precompose) => {
                collect_scene_gradient_refs(&precompose.children, out)
            }
            SceneNode::Layer(layer) => collect_scene_gradient_refs(&layer.children, out),
            SceneNode::Camera(camera) => collect_scene_gradient_refs(&camera.children, out),
            SceneNode::Character(character) => {
                collect_scene_gradient_refs(&character.children, out)
            }
            SceneNode::Palette(_)
            | SceneNode::PixelGrid(_)
            | SceneNode::Image(_)
            | SceneNode::Svg(_)
            | SceneNode::Use(_) => {}
        }
    }
}

fn collect_defs_gradient_refs(defs: &DefsNode, out: &mut Vec<(String, String)>) {
    for brush in &defs.brushes {
        collect_optional_paint_gradient_ref(brush.stroke.as_deref(), out);
        collect_optional_paint_gradient_ref(brush.fill.as_deref(), out);
    }
    for mask in &defs.masks {
        collect_scene_gradient_refs(&mask.children, out);
    }
    for precompose in &defs.precomposes {
        collect_scene_gradient_refs(&precompose.children, out);
    }
    for component in &defs.components {
        collect_scene_gradient_refs(&component.children, out);
    }
}

fn collect_text_gradient_refs(text: &TextNode, out: &mut Vec<(String, String)>) {
    collect_paint_gradient_ref(&text.color, out);
    collect_optional_paint_gradient_ref(text.box_color.as_deref(), out);
    collect_optional_paint_gradient_ref(text.stroke.as_deref(), out);
    for animator in &text.animators {
        if let Some(style) = animator.style.as_ref() {
            collect_optional_paint_gradient_ref(style.color.as_deref(), out);
            collect_optional_paint_gradient_ref(style.stroke.as_deref(), out);
            collect_optional_paint_gradient_ref(style.shadow_color.as_deref(), out);
        }
        for effect in &animator.effects {
            match effect {
                crate::scene::text::TextEffectNode::Glow(glow) => {
                    collect_optional_paint_gradient_ref(glow.color.as_deref(), out);
                }
            }
        }
    }
}

fn collect_optional_paint_gradient_ref(value: Option<&str>, out: &mut Vec<(String, String)>) {
    if let Some(value) = value {
        collect_paint_gradient_ref(value, out);
    }
}

fn collect_paint_gradient_ref(value: &str, out: &mut Vec<(String, String)>) {
    if let Some(id) = gradient_ref_id(value) {
        out.push((value.to_string(), id.to_string()));
    }
}

struct SceneFrameRenderer {
    profile: SceneRenderProfile,
    asset_resolver: Arc<dyn AssetResolver>,
    font_system: FontSystem,
    swash_cache: SwashCache,
    image_cache: HashMap<String, RgbaImage>,
    svg_cache: HashMap<String, RgbaImage>,
    path_cache: HashMap<String, Vec<Vec<Point2>>>,
    polyline_cache: HashMap<String, Vec<Point2>>,
    gradient_defs: HashMap<String, GradientDef>,
    palette_defs: HashMap<String, PaletteNode>,
    font_defs: HashMap<String, FontDef>,
    filter_defs: HashMap<String, FilterDef>,
    scene_components: HashMap<String, Vec<SceneNode>>,
    scene_precompose_defs: HashMap<String, PrecomposeNode>,
    scene_precomposes: HashMap<String, RgbaImage>,
    scene_masks: HashMap<String, MaskNode>,
    world_renderer: WorldFrameRenderer,
    gpu_compositor: Option<WgpuSceneCompositor>,
}

#[derive(Clone, Copy)]
struct SceneLayerDrawParams {
    source_size: (u32, u32),
    base_transform: Affine2,
    clip: Option<CameraRect>,
    time_norm: f32,
    time_sec: f32,
    inherited_opacity: f32,
}

impl SceneFrameRenderer {
    #[allow(dead_code)]
    async fn new() -> Self {
        Self::new_for_profile(SceneRenderProfile::Cpu).await
    }

    async fn new_for_profile(profile: SceneRenderProfile) -> Self {
        Self::new_for_profile_with_resolver(profile, Arc::new(PathAssetResolver)).await
    }

    async fn new_for_profile_with_resolver(
        profile: SceneRenderProfile,
        asset_resolver: Arc<dyn AssetResolver>,
    ) -> Self {
        let mut font_system = FontSystem::new();
        load_extra_fonts(&mut font_system);
        Self {
            profile,
            asset_resolver,
            font_system,
            swash_cache: SwashCache::new(),
            image_cache: HashMap::new(),
            svg_cache: HashMap::new(),
            path_cache: HashMap::new(),
            polyline_cache: HashMap::new(),
            gradient_defs: HashMap::new(),
            palette_defs: HashMap::new(),
            font_defs: HashMap::new(),
            filter_defs: HashMap::new(),
            scene_components: HashMap::new(),
            scene_precompose_defs: HashMap::new(),
            scene_precomposes: HashMap::new(),
            scene_masks: HashMap::new(),
            world_renderer: WorldFrameRenderer::with_resolver(Arc::new(PathAssetResolver)),
            gpu_compositor: None,
        }
    }

    async fn render_frame(
        &mut self,
        graph: &GraphScript,
        frame: u32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let fps = graph.fps.max(1.0);
        let duration_sec = (graph.duration_ms as f32 / 1000.0).max(1.0 / fps);
        let time_sec = frame as f32 / fps;
        let time_norm = (time_sec / duration_sec).clamp(0.0, 1.0);
        let applied_graph = apply_action_graph_at_time(graph, time_norm, time_sec)?;
        let graph = applied_graph.as_ref().unwrap_or(graph);
        self.gradient_defs.clear();
        self.palette_defs.clear();
        self.font_defs.clear();
        self.filter_defs.clear();
        self.scene_components.clear();
        self.scene_precompose_defs.clear();
        self.scene_precomposes.clear();
        self.scene_masks.clear();
        collect_graph_gradient_defs(graph, &mut self.gradient_defs);
        collect_graph_palette_defs(graph, &mut self.palette_defs);
        collect_graph_font_defs(graph, &mut self.font_defs);
        collect_graph_filter_defs(graph, &mut self.filter_defs);
        collect_graph_component_defs(graph, &mut self.scene_components);
        collect_graph_mask_defs(graph, &mut self.scene_masks);
        for precompose in collect_graph_precompose_defs(graph) {
            self.scene_precompose_defs
                .insert(precompose.id.clone(), precompose);
        }
        if graph_has_rich_scene_tree(graph) {
            return self
                .render_scene_tree_frame(graph, time_norm, time_sec)
                .await;
        }

        let mut canvas = if self.profile.uses_gpu_compositor() {
            self.render_gpu_base_frame(graph, time_norm, time_sec)
                .await?
        } else {
            self.render_cpu_base_frame(graph, time_norm, time_sec)?
        };

        for text in &graph.texts {
            self.draw_text(&mut canvas, text, time_norm, time_sec)?;
        }

        if let Some(output_size) = graph.render_size {
            Ok(fit_logical_canvas_to_output(&canvas, output_size))
        } else {
            Ok(canvas)
        }
    }

    /// Present one strict GPU scene-tree frame directly to an HTML canvas.
    #[cfg(target_arch = "wasm32")]
    async fn render_frame_to_canvas(
        &mut self,
        graph: &GraphScript,
        frame: u32,
        canvas: web_sys::HtmlCanvasElement,
    ) -> Result<(), MotionLoomSceneRenderError> {
        if !self.profile.uses_gpu_compositor() {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: "canvas rendering requires the GPU profile".to_string(),
            });
        }

        let fps = graph.fps.max(1.0);
        let duration_sec = (graph.duration_ms as f32 / 1000.0).max(1.0 / fps);
        let time_sec = frame as f32 / fps;
        let time_norm = (time_sec / duration_sec).clamp(0.0, 1.0);
        let applied_graph = apply_action_graph_at_time(graph, time_norm, time_sec)?;
        let graph = applied_graph.as_ref().unwrap_or(graph);

        self.gradient_defs.clear();
        self.palette_defs.clear();
        self.font_defs.clear();
        self.filter_defs.clear();
        self.scene_components.clear();
        self.scene_precompose_defs.clear();
        self.scene_precomposes.clear();
        self.scene_masks.clear();
        collect_graph_gradient_defs(graph, &mut self.gradient_defs);
        collect_graph_palette_defs(graph, &mut self.palette_defs);
        collect_graph_font_defs(graph, &mut self.font_defs);
        collect_graph_filter_defs(graph, &mut self.filter_defs);
        collect_graph_component_defs(graph, &mut self.scene_components);
        collect_graph_mask_defs(graph, &mut self.scene_masks);
        for precompose in collect_graph_precompose_defs(graph) {
            self.scene_precompose_defs
                .insert(precompose.id.clone(), precompose);
        }

        self.render_scene_tree_frame_to_canvas(graph, time_norm, time_sec, canvas)
            .await
    }

    /// Draw a solid WebGPU color into a browser canvas for surface debugging.
    #[cfg(target_arch = "wasm32")]
    async fn debug_solid_to_canvas(
        &mut self,
        canvas: web_sys::HtmlCanvasElement,
        width: u32,
        height: u32,
        color: [f64; 4],
    ) -> Result<(), MotionLoomSceneRenderError> {
        if !self.profile.uses_gpu_compositor() {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: "debug canvas rendering requires the GPU profile".to_string(),
            });
        }
        self.ensure_gpu_compositor_size(width.max(1), height.max(1))
            .await?;
        let compositor =
            self.gpu_compositor
                .as_ref()
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                })?;
        compositor.debug_present_solid_to_canvas(&canvas, width, height, color)
    }

    /// Upload a solid texture and present it to a browser canvas for debugging.
    #[cfg(target_arch = "wasm32")]
    async fn debug_uploaded_texture_to_canvas(
        &mut self,
        canvas: web_sys::HtmlCanvasElement,
        width: u32,
        height: u32,
        color: [u8; 4],
    ) -> Result<(), MotionLoomSceneRenderError> {
        if !self.profile.uses_gpu_compositor() {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: "debug texture rendering requires the GPU profile".to_string(),
            });
        }
        self.ensure_gpu_compositor_size(width.max(1), height.max(1))
            .await?;
        let compositor =
            self.gpu_compositor
                .as_ref()
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                })?;
        compositor.debug_present_uploaded_texture_to_canvas(&canvas, width, height, color)
    }

    /// Render an empty scene texture with a white clear color and present it.
    #[cfg(target_arch = "wasm32")]
    async fn debug_empty_scene_texture_to_canvas(
        &mut self,
        canvas: web_sys::HtmlCanvasElement,
        width: u32,
        height: u32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        if !self.profile.uses_gpu_compositor() {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: "debug empty scene rendering requires the GPU profile".to_string(),
            });
        }
        self.ensure_gpu_compositor_size(width.max(1), height.max(1))
            .await?;
        let compositor =
            self.gpu_compositor
                .as_mut()
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                })?;
        let texture = compositor.render_scene_content_to_texture(&[], &[], [255, 255, 255, 255])?;
        compositor.present_texture_to_canvas(&texture, &canvas)
    }

    /// Render a GPU-native scene tree to a browser canvas surface.
    #[cfg(target_arch = "wasm32")]
    async fn render_scene_tree_frame_to_canvas(
        &mut self,
        graph: &GraphScript,
        time_norm: f32,
        time_sec: f32,
        canvas: web_sys::HtmlCanvasElement,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let has_composition = !graph.textures.is_empty()
            || !graph.passes.is_empty()
            || !graph.outputs.is_empty()
            || !graph.layers.is_empty()
            || !graph.world_sources.is_empty();
        if has_composition {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message:
                    "direct WASM canvas GPU rendering does not support Tex/Pass/Output composition yet"
                        .to_string(),
            });
        }

        let Some(nodes) = scene_nodes_for_present(graph) else {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: "direct WASM canvas GPU rendering needs a presentable scene tree"
                    .to_string(),
            });
        };
        let background = graph
            .backgrounds
            .last()
            .map(|background| parse_color(&background.color))
            .transpose()?
            .unwrap_or([0, 0, 0, 0]);
        self.present_gpu_scene_nodes_with_background_to_canvas(
            nodes,
            graph_output_size(graph),
            graph_logical_render_size(graph),
            render_size_root_transform(graph_output_size(graph), graph_logical_render_size(graph)),
            time_norm,
            time_sec,
            Some(background),
            canvas,
        )
        .await
    }

    async fn render_scene_tree_frame(
        &mut self,
        graph: &GraphScript,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let has_composition = !graph.textures.is_empty()
            || !graph.passes.is_empty()
            || !graph.outputs.is_empty()
            || !graph.layers.is_empty()
            || !graph.world_sources.is_empty();
        let cpu_scene_compositing_required = scene_nodes_for_present(graph)
            .map(scene_nodes_require_cpu_scene_compositing)
            .unwrap_or(false);
        if !has_composition {
            if let Some(image) = self
                .try_render_gpu_scene_tree_frame(graph, time_norm, time_sec)
                .await?
            {
                return Ok(image);
            }
            if self.profile.uses_gpu_compositor() && !cpu_scene_compositing_required {
                return Err(MotionLoomSceneRenderError::GpuRender {
                    message:
                        "GPU render is strict: this scene graph was not produced by the GPU scene renderer."
                            .to_string(),
                });
            }
        }

        let (w, h) = graph_output_size(graph);
        let output_size = (w, h);
        let logical_size = graph_logical_render_size(graph);
        let root_transform = render_size_root_transform(output_size, logical_size);
        let mut resources = HashMap::<String, RgbaImage>::new();
        let background = graph
            .backgrounds
            .last()
            .map(|background| parse_color(&background.color))
            .transpose()?
            .unwrap_or([0, 0, 0, 0]);
        let background_canvas = if self.profile.uses_gpu_compositor() {
            self.render_gpu_background_frame(output_size, background)
                .await?
        } else {
            solid_canvas(output_size, background)
        };
        resources.insert("scene".to_string(), background_canvas.clone());
        resources.insert("background".to_string(), background_canvas.clone());
        for background_node in &graph.backgrounds {
            if let Some(id) = background_node.id.as_deref() {
                resources.insert(id.to_string(), background_canvas.clone());
            }
        }

        if !graph.world_sources.is_empty() {
            let raw_script = graph.raw_script.as_deref().ok_or_else(|| {
                MotionLoomSceneRenderError::WorldSource {
                    message: "unified graph is missing raw DSL needed to render <World> sources"
                        .to_string(),
                }
            })?;
            let world_frame = (time_sec * graph.fps.max(1.0)).round().max(0.0) as u32;
            let world_asset_root = default_world_asset_root();
            let base_world_graph = parse_world_graph_script(raw_script).map_err(|err| {
                MotionLoomSceneRenderError::WorldSource {
                    message: format!("parse error at line {}: {}", err.line, err.message),
                }
            })?;
            for world_source in &graph.world_sources {
                let mut world_graph = base_world_graph.clone();
                world_graph.present.from = world_source.id.clone();
                world_graph.render_size = Some(output_size);
                let image = self
                    .world_renderer
                    .render_frame_gpu(&world_graph, world_frame, &world_asset_root)
                    .await
                    .map_err(|err| MotionLoomSceneRenderError::WorldSource {
                        message: err.to_string(),
                    })?;
                resources.insert(world_source.id.clone(), image.clone());
                resources.insert(format!("world:{}", world_source.id), image);
            }
        }

        if !graph.scene_nodes.is_empty() {
            let maybe_gpu_image = self
                .try_render_gpu_scene_nodes(
                    &graph.scene_nodes,
                    output_size,
                    logical_size,
                    root_transform,
                    time_norm,
                    time_sec,
                )
                .await?;
            let canvas = if let Some(image) = maybe_gpu_image {
                image
            } else if self.profile.uses_gpu_compositor() {
                return Err(MotionLoomSceneRenderError::GpuRender {
                    message:
                        "GPU render is strict: root scene nodes are not supported by the GPU scene renderer."
                            .to_string(),
                });
            } else {
                self.render_cpu_scene_nodes_scaled(
                    &graph.scene_nodes,
                    output_size,
                    logical_size,
                    time_norm,
                    time_sec,
                    [0, 0, 0, 0],
                )?
            };
            resources.insert("scene".to_string(), canvas);
        }

        for scene in &graph.scenes {
            let scene_size = scene.size.unwrap_or(graph.size);
            let (scene_output_size, scene_logical_size, scene_transform) = if scene.size.is_some() {
                (scene_size, scene_size, Affine2::identity())
            } else {
                (output_size, logical_size, root_transform)
            };
            let maybe_gpu_image = self
                .try_render_gpu_scene_nodes_with_background(
                    &scene.children,
                    scene_output_size,
                    scene_logical_size,
                    scene_transform,
                    time_norm,
                    time_sec,
                    Some(background),
                )
                .await?;
            let scene_canvas = if let Some(image) = maybe_gpu_image {
                image
            } else if self.profile.uses_gpu_compositor() {
                return Err(MotionLoomSceneRenderError::GpuRender {
                    message: format!(
                        "GPU render is strict: <Scene id=\"{}\"> contains nodes not supported by the GPU scene renderer.",
                        scene.id
                    ),
                });
            } else {
                self.render_cpu_scene_nodes_scaled(
                    &scene.children,
                    scene_output_size,
                    scene_logical_size,
                    time_norm,
                    time_sec,
                    background,
                )?
            };
            resources.insert(scene.id.clone(), scene_canvas.clone());
            resources.insert(format!("scene:{}", scene.id), scene_canvas.clone());
            resources.entry("scene".to_string()).or_insert(scene_canvas);
        }

        for tex in &graph.textures {
            let image = if let (Some(layer_id), Some(input_id)) =
                (tex.from.as_deref(), tex.input.as_deref())
            {
                if let Some(layer) = graph.layers.iter().find(|layer| layer.id == layer_id) {
                    let base = resources.get(input_id).cloned().unwrap_or_else(|| {
                        RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([0, 0, 0, 0]))
                    });
                    apply_layer_effects(&base, layer, time_norm, time_sec)?
                } else {
                    resources.get(layer_id).cloned().unwrap_or_else(|| {
                        RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([0, 0, 0, 0]))
                    })
                }
            } else if let Some(from) = tex.from.as_deref() {
                resources.get(from).cloned().unwrap_or_else(|| {
                    RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([0, 0, 0, 0]))
                })
            } else {
                let size = tex.size.unwrap_or(graph.size);
                RgbaImage::from_pixel(size.0.max(1), size.1.max(1), Rgba([0, 0, 0, 0]))
            };
            resources.insert(tex.id.clone(), image);
        }

        for pass in &graph.passes {
            let inputs = pass
                .inputs
                .iter()
                .filter_map(|input| resources.get(input.resource_id()).cloned())
                .collect::<Vec<_>>();
            if inputs.is_empty() {
                continue;
            }
            let output = self
                .apply_scene_post_pass_multi(&inputs, pass, time_norm, time_sec)
                .await?;
            for output_ref in &pass.outputs {
                resources.insert(output_ref.resource_id().to_string(), output.clone());
            }
        }

        // Layer Tex nodes can depend on Pass outputs declared later in the DSL.
        // Re-resolve them after passes so <Tex from="layer" input="comp" /> uses
        // the final upstream resource instead of a transparent placeholder.
        for tex in graph.textures.iter().filter(|tex| tex.input.is_some()) {
            if let (Some(layer_id), Some(input_id)) = (tex.from.as_deref(), tex.input.as_deref())
                && let Some(layer) = graph.layers.iter().find(|layer| layer.id == layer_id)
                && let Some(base) = resources.get(input_id).cloned()
            {
                let image = apply_layer_effects(&base, layer, time_norm, time_sec)?;
                resources.insert(tex.id.clone(), image);
            }
        }

        for output in &graph.outputs {
            if let Some(from) = output.from.as_deref()
                && let Some(image) = resources.get(from).cloned()
            {
                resources.insert(output.id.clone(), image);
            }
        }

        let image = resources
            .get(&graph.present.from)
            .or_else(|| {
                graph
                    .present
                    .from
                    .strip_prefix("scene:")
                    .and_then(|id| resources.get(id))
            })
            .or_else(|| resources.get("scene"))
            .cloned()
            .unwrap_or_else(|| RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([0, 0, 0, 0])));
        if image.width() == w && image.height() == h {
            Ok(image)
        } else {
            let mut canvas = RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([0, 0, 0, 0]));
            draw_rgba_image(&mut canvas, &image, 0.0, 0.0, 1.0);
            Ok(canvas)
        }
    }

    async fn try_render_gpu_scene_tree_frame(
        &mut self,
        graph: &GraphScript,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<Option<RgbaImage>, MotionLoomSceneRenderError> {
        if !self.profile.uses_gpu_compositor() {
            return Ok(None);
        }
        if !graph.textures.is_empty() || !graph.passes.is_empty() || !graph.outputs.is_empty() {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message:
                    "GPU preview is strict: Tex/Pass/Output composition is not GPU-native in scene preview yet."
                        .to_string(),
            });
        }

        let Some(nodes) = scene_nodes_for_present(graph) else {
            return Ok(None);
        };
        let background = graph
            .backgrounds
            .last()
            .map(|background| parse_color(&background.color))
            .transpose()?
            .unwrap_or([0, 0, 0, 0]);

        self.try_render_gpu_scene_nodes_with_background(
            nodes,
            graph_output_size(graph),
            graph_logical_render_size(graph),
            render_size_root_transform(graph_output_size(graph), graph_logical_render_size(graph)),
            time_norm,
            time_sec,
            Some(background),
        )
        .await
    }

    async fn try_render_gpu_scene_nodes(
        &mut self,
        nodes: &[SceneNode],
        output_size: (u32, u32),
        logical_size: (u32, u32),
        root_transform: Affine2,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<Option<RgbaImage>, MotionLoomSceneRenderError> {
        self.try_render_gpu_scene_nodes_with_background(
            nodes,
            output_size,
            logical_size,
            root_transform,
            time_norm,
            time_sec,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn try_render_gpu_scene_nodes_with_background(
        &mut self,
        nodes: &[SceneNode],
        output_size: (u32, u32),
        logical_size: (u32, u32),
        root_transform: Affine2,
        time_norm: f32,
        time_sec: f32,
        background: Option<[u8; 4]>,
    ) -> Result<Option<RgbaImage>, MotionLoomSceneRenderError> {
        if !self.profile.uses_gpu_compositor() {
            return Ok(None);
        }
        if scene_nodes_require_cpu_scene_compositing(nodes)
            || scene_nodes_contain_image_or_svg(nodes)
        {
            if let Some(image) = self
                .try_render_gpu_scene_nodes_composited(
                    nodes,
                    output_size,
                    logical_size,
                    root_transform,
                    time_norm,
                    time_sec,
                    background,
                )
                .await?
            {
                return Ok(Some(image));
            }
            return Ok(None);
        }

        let mut primitives = Vec::<GpuScenePrimitive>::new();
        let mut scene_overlays = Vec::<CpuSceneOverlay>::new();
        let mut text_requests = Vec::<GpuSceneTextRequest>::new();
        collect_gpu_scene_commands(
            nodes,
            root_transform,
            None,
            1.0,
            time_norm,
            time_sec,
            logical_size,
            &self.gradient_defs,
            &self.palette_defs,
            &self.scene_components,
            &mut primitives,
            &mut text_requests,
            &mut scene_overlays,
        )?;

        if !scene_overlays.is_empty() {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: format!(
                    "GPU Render is strict for MotionLoom: CPU overlays are disabled. Unsupported GPU-native nodes: {}",
                    describe_cpu_scene_overlays(&scene_overlays)
                ),
            });
        }

        self.ensure_gpu_compositor_size(output_size.0.max(1), output_size.1.max(1))
            .await?;

        let mut texture_layers = Vec::with_capacity(text_requests.len());
        for request in text_requests {
            texture_layers.extend(self.rasterize_text_texture_layers_gpu_effects(
                &request.node,
                request.transform,
                request.opacity,
                time_norm,
                time_sec,
                logical_size,
            )?);
        }

        let compositor =
            self.gpu_compositor
                .as_mut()
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                })?;
        let texture = compositor.render_scene_content_to_texture(
            &primitives,
            &texture_layers,
            background.unwrap_or([0, 0, 0, 0]),
        )?;
        let canvas = compositor
            .readback_texture_rgba_async(&texture.texture)
            .await?;
        Ok(Some(canvas))
    }

    /// Present GPU-native scene nodes directly to a browser canvas surface.
    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::too_many_arguments)]
    async fn present_gpu_scene_nodes_with_background_to_canvas(
        &mut self,
        nodes: &[SceneNode],
        output_size: (u32, u32),
        logical_size: (u32, u32),
        root_transform: Affine2,
        time_norm: f32,
        time_sec: f32,
        background: Option<[u8; 4]>,
        canvas: web_sys::HtmlCanvasElement,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let scaled_scene = output_size != logical_size || !affine_is_identity(root_transform);
        let scene_transform = if scaled_scene {
            root_transform
        } else {
            Affine2::identity()
        };
        let scene_canvas_size = if scaled_scene {
            logical_size
        } else {
            output_size
        };

        self.ensure_gpu_compositor_size(output_size.0.max(1), output_size.1.max(1))
            .await?;
        let mut assets = GpuSceneNativeAssets::default();
        let mut primitives = Vec::<GpuScenePrimitive>::new();
        let mut texture_layers = Vec::<GpuSceneTextureLayer>::new();
        let mut text_requests = Vec::<GpuSceneTextRequest>::new();
        let mut unsupported = false;
        self.collect_gpu_scene_native_commands(
            nodes,
            scene_transform,
            None,
            1.0,
            time_norm,
            time_sec,
            scene_canvas_size,
            &mut assets,
            &mut primitives,
            &mut texture_layers,
            &mut text_requests,
            &mut unsupported,
        )
        .await?;
        if unsupported {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message:
                    "direct WASM canvas GPU rendering does not support this scene node set yet"
                        .to_string(),
            });
        }

        for request in text_requests {
            texture_layers.extend(self.rasterize_text_texture_layers_gpu_effects(
                &request.node,
                request.transform,
                request.opacity,
                time_norm,
                time_sec,
                scene_canvas_size,
            )?);
        }

        self.ensure_gpu_compositor_size(output_size.0.max(1), output_size.1.max(1))
            .await?;
        let compositor =
            self.gpu_compositor
                .as_mut()
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                })?;
        let texture = compositor.render_scene_content_to_texture(
            &primitives,
            &texture_layers,
            background.unwrap_or([0, 0, 0, 0]),
        )?;
        compositor.present_texture_to_canvas(&texture, &canvas)
    }

    async fn ensure_gpu_compositor_size(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let needs_new_compositor = self
            .gpu_compositor
            .as_ref()
            .map(|compositor| compositor.width != width || compositor.height != height)
            .unwrap_or(true);
        if needs_new_compositor {
            self.gpu_compositor =
                Some(WgpuSceneCompositor::new(width, height, self.asset_resolver.clone()).await?);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn try_render_gpu_scene_nodes_composited(
        &mut self,
        nodes: &[SceneNode],
        output_size: (u32, u32),
        logical_size: (u32, u32),
        root_transform: Affine2,
        time_norm: f32,
        time_sec: f32,
        background: Option<[u8; 4]>,
    ) -> Result<Option<RgbaImage>, MotionLoomSceneRenderError> {
        let scaled_scene = output_size != logical_size || !affine_is_identity(root_transform);
        let scene_transform = if scaled_scene {
            root_transform
        } else {
            Affine2::identity()
        };
        let scene_canvas_size = if scaled_scene {
            logical_size
        } else {
            output_size
        };
        self.ensure_gpu_compositor_size(output_size.0.max(1), output_size.1.max(1))
            .await?;
        let mut assets = GpuSceneNativeAssets::default();
        let mut primitives = Vec::<GpuScenePrimitive>::new();
        let mut texture_layers = Vec::<GpuSceneTextureLayer>::new();
        let mut text_requests = Vec::<GpuSceneTextRequest>::new();
        let mut unsupported = false;
        self.collect_gpu_scene_native_commands(
            nodes,
            scene_transform,
            None,
            1.0,
            time_norm,
            time_sec,
            scene_canvas_size,
            &mut assets,
            &mut primitives,
            &mut texture_layers,
            &mut text_requests,
            &mut unsupported,
        )
        .await?;
        if unsupported {
            return Ok(None);
        }

        for request in text_requests {
            texture_layers.extend(self.rasterize_text_texture_layers_gpu_effects(
                &request.node,
                request.transform,
                request.opacity,
                time_norm,
                time_sec,
                scene_canvas_size,
            )?);
        }

        self.ensure_gpu_compositor_size(output_size.0.max(1), output_size.1.max(1))
            .await?;
        let compositor =
            self.gpu_compositor
                .as_mut()
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                })?;
        let texture = compositor.render_scene_content_to_texture(
            &primitives,
            &texture_layers,
            background.unwrap_or([0, 0, 0, 0]),
        )?;
        compositor
            .readback_texture_rgba_async(&texture.texture)
            .await
            .map(Some)
    }

    #[allow(clippy::too_many_arguments)]
    async fn render_gpu_scene_texture_from_nodes(
        &mut self,
        nodes: &[SceneNode],
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
        canvas_size: (u32, u32),
        assets: &mut GpuSceneNativeAssets,
    ) -> Result<Option<GpuSceneNativeTexture>, MotionLoomSceneRenderError> {
        self.ensure_gpu_compositor_size(canvas_size.0.max(1), canvas_size.1.max(1))
            .await?;
        let mut primitives = Vec::<GpuScenePrimitive>::new();
        let mut texture_layers = Vec::<GpuSceneTextureLayer>::new();
        let mut text_requests = Vec::<GpuSceneTextRequest>::new();
        let mut unsupported = false;
        self.collect_gpu_scene_native_commands(
            nodes,
            transform,
            None,
            inherited_opacity,
            time_norm,
            time_sec,
            canvas_size,
            assets,
            &mut primitives,
            &mut texture_layers,
            &mut text_requests,
            &mut unsupported,
        )
        .await?;
        if unsupported {
            return Ok(None);
        }
        for request in text_requests {
            texture_layers.extend(self.rasterize_text_texture_layers_gpu_effects(
                &request.node,
                request.transform,
                request.opacity,
                time_norm,
                time_sec,
                canvas_size,
            )?);
        }
        self.ensure_gpu_compositor_size(canvas_size.0.max(1), canvas_size.1.max(1))
            .await?;
        let compositor =
            self.gpu_compositor
                .as_mut()
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                })?;
        compositor
            .render_scene_content_to_texture(&primitives, &texture_layers, [0, 0, 0, 0])
            .map(Some)
    }

    async fn render_gpu_precompose_instance(
        &mut self,
        source_id: &str,
        layer: &SceneLayerNode,
        canvas_size: (u32, u32),
        assets: &mut GpuSceneNativeAssets,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<Option<GpuSceneNativeTexture>, MotionLoomSceneRenderError> {
        let Some(precompose) = self.scene_precompose_defs.get(source_id).cloned() else {
            return Ok(assets.precomposes.get(source_id).cloned());
        };
        let size = precompose.size.unwrap_or(canvas_size);
        if size != canvas_size {
            return Ok(None);
        }
        let Some((source_norm, source_sec)) =
            scene_layer_source_time(layer, &precompose, time_norm, time_sec)?
        else {
            return Ok(None);
        };
        self.render_gpu_scene_texture_from_nodes(
            &precompose.children,
            Affine2::identity(),
            1.0,
            source_norm,
            source_sec,
            canvas_size,
            assets,
        )
        .await
    }

    fn apply_gpu_scene_filter_texture(
        &mut self,
        input: GpuSceneNativeTexture,
        filter_id: &str,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<GpuSceneNativeTexture, MotionLoomSceneRenderError> {
        let Some(filter) = self.filter_defs.get(filter_id).cloned() else {
            return Ok(input);
        };
        let compositor =
            self.gpu_compositor
                .as_mut()
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                })?;
        let mut output = input;
        for step in filter.steps {
            let kind = step.kind.trim().to_ascii_lowercase();
            if kind == "blur"
                || kind == "gaussian_blur"
                || kind == "gaussian-blur"
                || kind == "gaussian_5tap_blur"
            {
                let sigma = step
                    .radius
                    .as_deref()
                    .map(|expr| eval_scene_number(expr, time_norm, time_sec))
                    .transpose()?
                    .unwrap_or(2.0)
                    .clamp(0.0, 64.0);
                output =
                    compositor.apply_gpu_blur_texture(&output, &[(true, sigma), (false, sigma)])?;
                continue;
            }
            if kind == "colormatrix" || kind == "color_matrix" || kind == "color-matrix" {
                let brightness = step
                    .brightness
                    .as_deref()
                    .map(|expr| eval_scene_number(expr, time_norm, time_sec))
                    .transpose()?
                    .unwrap_or(1.0)
                    .clamp(0.0, 4.0)
                    - 1.0;
                let contrast = step
                    .contrast
                    .as_deref()
                    .map(|expr| eval_scene_number(expr, time_norm, time_sec))
                    .transpose()?
                    .unwrap_or(1.0)
                    .clamp(0.0, 4.0);
                let saturation = step
                    .saturation
                    .as_deref()
                    .map(|expr| eval_scene_number(expr, time_norm, time_sec))
                    .transpose()?
                    .unwrap_or(1.0)
                    .clamp(0.0, 4.0);
                output = compositor
                    .apply_gpu_color_texture(&output, brightness, contrast, saturation)?;
                continue;
            }
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: format!(
                    "Filter '{}' step '{}' is not GPU-native yet",
                    filter_id, step.kind
                ),
            });
        }
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_gpu_scene_native_commands<'a>(
        &'a mut self,
        nodes: &'a [SceneNode],
        transform: Affine2,
        deform: Option<&'a EvaluatedDeformGrid>,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
        canvas_size: (u32, u32),
        assets: &'a mut GpuSceneNativeAssets,
        primitives: &'a mut Vec<GpuScenePrimitive>,
        texture_layers: &'a mut Vec<GpuSceneTextureLayer>,
        text_requests: &'a mut Vec<GpuSceneTextRequest>,
        unsupported: &'a mut bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), MotionLoomSceneRenderError>> + 'a>> {
        self.collect_gpu_scene_native_commands_with_depth(
            nodes,
            transform,
            deform,
            inherited_opacity,
            time_norm,
            time_sec,
            canvas_size,
            assets,
            primitives,
            texture_layers,
            text_requests,
            unsupported,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_gpu_scene_native_commands_with_depth<'a>(
        &'a mut self,
        nodes: &'a [SceneNode],
        transform: Affine2,
        deform: Option<&'a EvaluatedDeformGrid>,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
        canvas_size: (u32, u32),
        assets: &'a mut GpuSceneNativeAssets,
        primitives: &'a mut Vec<GpuScenePrimitive>,
        texture_layers: &'a mut Vec<GpuSceneTextureLayer>,
        text_requests: &'a mut Vec<GpuSceneTextRequest>,
        unsupported: &'a mut bool,
        depth: Option<SceneDepthContext<'a>>,
    ) -> Pin<Box<dyn Future<Output = Result<(), MotionLoomSceneRenderError>> + 'a>> {
        Box::pin(async move {
            for node in nodes {
                match node {
                    SceneNode::Defs(defs) => {
                        for font in &defs.fonts {
                            self.font_defs.insert(font.id.clone(), font.clone());
                        }
                        for filter in &defs.filters {
                            self.filter_defs.insert(filter.id.clone(), filter.clone());
                        }
                        for component in &defs.components {
                            self.scene_components
                                .insert(component.id.clone(), component.children.clone());
                        }
                        for precompose in &defs.precomposes {
                            self.scene_precompose_defs
                                .insert(precompose.id.clone(), precompose.clone());
                        }
                        for mask in &defs.masks {
                            if let Some(id) = mask.id.as_deref() {
                                let Some(texture) = self
                                    .render_gpu_mask_texture(
                                        mask,
                                        transform,
                                        time_norm,
                                        time_sec,
                                        canvas_size,
                                    )
                                    .await?
                                else {
                                    *unsupported = true;
                                    continue;
                                };
                                assets.masks.insert(id.to_string(), texture);
                            }
                        }
                    }
                    SceneNode::Palette(_) | SceneNode::Shadow(_) => {}
                    SceneNode::Timeline(timeline) => {
                        let mut tracks = timeline
                            .children
                            .iter()
                            .filter_map(|node| match node {
                                SceneNode::Track(track) => Some(track),
                                _ => None,
                            })
                            .collect::<Vec<_>>();
                        tracks.sort_by_key(|track| track.z);
                        let active_camera = active_scene_camera_from_tracks(
                            &tracks,
                            canvas_size.0,
                            canvas_size.1,
                            time_norm,
                            time_sec,
                        )?;
                        tracks.sort_by(|a, b| {
                            let a_world = is_scene_world_track(a);
                            let b_world = is_scene_world_track(b);
                            match (a_world, b_world) {
                                (true, true) => {
                                    let a_depth =
                                        scene_depth_track_sort_key(&a.z_depth, time_norm, time_sec)
                                            .unwrap_or(0.0);
                                    let b_depth =
                                        scene_depth_track_sort_key(&b.z_depth, time_norm, time_sec)
                                            .unwrap_or(0.0);
                                    b_depth.total_cmp(&a_depth).then_with(|| a.z.cmp(&b.z))
                                }
                                _ => a.z.cmp(&b.z),
                            }
                        });
                        for track in tracks {
                            if is_scene_camera_track(track) {
                                continue;
                            }
                            let track_depth = if is_scene_world_track(track) {
                                Some(SceneDepthContext {
                                    active_camera,
                                    canvas_size,
                                    track_z_depth: &track.z_depth,
                                })
                            } else {
                                depth
                            };
                            self.collect_gpu_scene_native_commands_with_depth(
                                &track.children,
                                transform,
                                deform,
                                inherited_opacity,
                                time_norm,
                                time_sec,
                                canvas_size,
                                assets,
                                primitives,
                                texture_layers,
                                text_requests,
                                unsupported,
                                track_depth,
                            )
                            .await?;
                        }
                    }
                    SceneNode::Track(track) => {
                        self.collect_gpu_scene_native_commands_with_depth(
                            &track.children,
                            transform,
                            deform,
                            inherited_opacity,
                            time_norm,
                            time_sec,
                            canvas_size,
                            assets,
                            primitives,
                            texture_layers,
                            text_requests,
                            unsupported,
                            depth,
                        )
                        .await?;
                    }
                    SceneNode::Sequence(sequence) => {
                        if let Some((local_norm, local_sec)) =
                            scene_sequence_local_time(sequence, None, time_sec)
                        {
                            if depth.is_some() {
                                self.collect_gpu_scene_native_commands_depth_sorted(
                                    &sequence.children,
                                    transform,
                                    deform,
                                    inherited_opacity,
                                    local_norm,
                                    local_sec,
                                    canvas_size,
                                    assets,
                                    primitives,
                                    texture_layers,
                                    text_requests,
                                    unsupported,
                                    depth,
                                )
                                .await?;
                            } else {
                                self.collect_gpu_scene_native_commands_with_depth(
                                    &sequence.children,
                                    transform,
                                    deform,
                                    inherited_opacity,
                                    local_norm,
                                    local_sec,
                                    canvas_size,
                                    assets,
                                    primitives,
                                    texture_layers,
                                    text_requests,
                                    unsupported,
                                    depth,
                                )
                                .await?;
                            }
                        }
                    }
                    SceneNode::Chain(chain) => {
                        let mut cursor_ms = chain.from_ms as i64;
                        for child in &chain.children {
                            if let SceneNode::Sequence(sequence) = child {
                                if let Some((local_norm, local_sec)) =
                                    scene_sequence_local_time(sequence, Some(cursor_ms), time_sec)
                                {
                                    if depth.is_some() {
                                        self.collect_gpu_scene_native_commands_depth_sorted(
                                            &sequence.children,
                                            transform,
                                            deform,
                                            inherited_opacity,
                                            local_norm,
                                            local_sec,
                                            canvas_size,
                                            assets,
                                            primitives,
                                            texture_layers,
                                            text_requests,
                                            unsupported,
                                            depth,
                                        )
                                        .await?;
                                    } else {
                                        self.collect_gpu_scene_native_commands_with_depth(
                                            &sequence.children,
                                            transform,
                                            deform,
                                            inherited_opacity,
                                            local_norm,
                                            local_sec,
                                            canvas_size,
                                            assets,
                                            primitives,
                                            texture_layers,
                                            text_requests,
                                            unsupported,
                                            depth,
                                        )
                                        .await?;
                                    }
                                }
                                cursor_ms += sequence.duration_ms as i64 + chain.gap_ms;
                            }
                        }
                    }
                    SceneNode::Mask(mask) => {
                        if let Some(id) = mask.id.as_deref() {
                            let Some(texture) = self
                                .render_gpu_mask_texture(
                                    mask,
                                    transform,
                                    time_norm,
                                    time_sec,
                                    canvas_size,
                                )
                                .await?
                            else {
                                *unsupported = true;
                                continue;
                            };
                            assets.masks.insert(id.to_string(), texture.clone());
                        }
                        if !mask.children.is_empty() {
                            let Some(source) = self
                                .render_gpu_scene_texture_from_nodes(
                                    &mask.children,
                                    transform,
                                    inherited_opacity,
                                    time_norm,
                                    time_sec,
                                    canvas_size,
                                    assets,
                                )
                                .await?
                            else {
                                *unsupported = true;
                                continue;
                            };
                            let Some(matte) = self
                                .render_gpu_mask_texture(
                                    mask,
                                    transform,
                                    time_norm,
                                    time_sec,
                                    canvas_size,
                                )
                                .await?
                            else {
                                *unsupported = true;
                                continue;
                            };
                            texture_layers.push(GpuSceneTextureLayer {
                                source: GpuSceneTextureSource::Gpu(source),
                                transform: Affine2::identity(),
                                opacity: 1.0,
                                blend: SceneBlendMode::Normal,
                                matte: Some(GpuSceneTextureMatte {
                                    texture: matte,
                                    mode: GpuSceneMatteMode::Alpha,
                                    invert: false,
                                }),
                            });
                        }
                    }
                    SceneNode::Precompose(precompose) => {
                        self.scene_precompose_defs
                            .insert(precompose.id.clone(), precompose.clone());
                    }
                    SceneNode::Layer(layer) => {
                        let opacity = (eval_scene_number(&layer.opacity, time_norm, time_sec)?
                            * inherited_opacity)
                            .clamp(0.0, 1.0);
                        if opacity <= 0.0001 {
                            continue;
                        }
                        let blend = parse_scene_blend(&layer.blend)?;
                        let base_transform = if let Some(depth) = depth {
                            let z_depth =
                                scene_layer_effective_z_depth(layer, depth, time_norm, time_sec)?;
                            transform.mul(scene_z_depth_transform(
                                depth.active_camera,
                                depth.canvas_size,
                                z_depth,
                            ))
                        } else {
                            transform
                        };
                        if layer.source.is_none()
                            && layer.mask.is_none()
                            && layer.matte.is_none()
                            && layer.effect.is_none()
                            && !layer.children.is_empty()
                            && blend == SceneBlendMode::Normal
                            && (opacity - inherited_opacity).abs() <= 0.0001
                        {
                            self.collect_gpu_scene_native_commands_with_depth(
                                &layer.children,
                                base_transform
                                    .mul(scene_layer_local_transform(layer, time_norm, time_sec)?),
                                deform,
                                inherited_opacity,
                                time_norm,
                                time_sec,
                                canvas_size,
                                assets,
                                primitives,
                                texture_layers,
                                text_requests,
                                unsupported,
                                None,
                            )
                            .await?;
                            continue;
                        }
                        if layer.source.is_some() && !layer.children.is_empty() {
                            *unsupported = true;
                            continue;
                        }
                        let mut source = if !layer.children.is_empty() {
                            let Some(texture) = self
                                .render_gpu_scene_texture_from_nodes(
                                    &layer.children,
                                    Affine2::identity(),
                                    1.0,
                                    time_norm,
                                    time_sec,
                                    canvas_size,
                                    assets,
                                )
                                .await?
                            else {
                                *unsupported = true;
                                continue;
                            };
                            texture
                        } else if let Some(source_id) = layer.source.as_deref() {
                            if let Some(precompose) = self.scene_precompose_defs.get(source_id)
                                && precompose.size.unwrap_or(canvas_size) != canvas_size
                            {
                                *unsupported = true;
                                continue;
                            }
                            let Some(source) = self
                                .render_gpu_precompose_instance(
                                    source_id,
                                    layer,
                                    canvas_size,
                                    assets,
                                    time_norm,
                                    time_sec,
                                )
                                .await?
                            else {
                                continue;
                            };
                            source
                        } else {
                            continue;
                        };
                        if let Some(filter_id) = layer.effect.as_deref() {
                            source = self.apply_gpu_scene_filter_texture(
                                source, filter_id, time_norm, time_sec,
                            )?;
                        }
                        if layer.mask.is_some() && layer.matte.is_some() {
                            *unsupported = true;
                            continue;
                        }
                        let matte = if let Some(mask_id) = layer.mask.as_deref() {
                            assets
                                .masks
                                .get(mask_id)
                                .cloned()
                                .map(|texture| GpuSceneTextureMatte {
                                    texture,
                                    mode: GpuSceneMatteMode::Alpha,
                                    invert: scene_mask_mode_inverts(&layer.mask_mode),
                                })
                        } else if let Some(matte_id) = layer.matte.as_deref() {
                            if let Some(mask) = assets.masks.get(matte_id).cloned() {
                                Some(GpuSceneTextureMatte {
                                    texture: mask,
                                    mode: GpuSceneMatteMode::Alpha,
                                    invert: scene_bool(&layer.invert_matte),
                                })
                            } else {
                                if let Some(precompose) = self.scene_precompose_defs.get(matte_id)
                                    && precompose.size.unwrap_or(canvas_size) != canvas_size
                                {
                                    *unsupported = true;
                                    continue;
                                }
                                self.render_gpu_precompose_instance(
                                    matte_id,
                                    layer,
                                    canvas_size,
                                    assets,
                                    time_norm,
                                    time_sec,
                                )
                                .await?
                                .map(|texture| {
                                    GpuSceneTextureMatte {
                                        texture,
                                        mode: gpu_matte_mode(&layer.matte_mode),
                                        invert: scene_bool(&layer.invert_matte),
                                    }
                                })
                            }
                        } else {
                            None
                        };
                        texture_layers.push(GpuSceneTextureLayer {
                            source: GpuSceneTextureSource::Gpu(source),
                            transform: base_transform
                                .mul(scene_layer_local_transform(layer, time_norm, time_sec)?),
                            opacity,
                            blend,
                            matte,
                        });
                    }
                    SceneNode::Image(image) => {
                        if deform.is_some() {
                            *unsupported = true;
                            continue;
                        }
                        if let Some(layer) = self.gpu_image_texture_layer(
                            image,
                            transform,
                            inherited_opacity,
                            time_norm,
                            time_sec,
                            canvas_size,
                        )? {
                            texture_layers.push(layer);
                        }
                    }
                    SceneNode::Svg(svg) => {
                        if deform.is_some() {
                            *unsupported = true;
                            continue;
                        }
                        if let Some(layer) = self.gpu_svg_texture_layer(
                            svg,
                            transform,
                            inherited_opacity,
                            time_norm,
                            time_sec,
                            canvas_size,
                        )? {
                            texture_layers.push(layer);
                        }
                    }
                    SceneNode::Group(group) => {
                        let opacity = (eval_scene_number(&group.opacity, time_norm, time_sec)?
                            * inherited_opacity)
                            .clamp(0.0, 1.0);
                        if opacity <= 0.0001 {
                            continue;
                        }
                        let group_local = scene_group_local_transform(group, time_norm, time_sec)?;
                        let group_transform = transform.mul(group_local);
                        let group_deform = eval_group_deform_grid(group, time_norm, time_sec)?;
                        if let Some(mask_id) = group.mask.as_deref() {
                            if !affine_is_identity(group_local) || group_deform.is_some() {
                                *unsupported = true;
                                continue;
                            }
                            let Some(matte) = assets.masks.get(mask_id).cloned() else {
                                *unsupported = true;
                                continue;
                            };
                            let Some(source) = self
                                .render_gpu_scene_texture_from_nodes(
                                    &group.children,
                                    group_transform,
                                    opacity,
                                    time_norm,
                                    time_sec,
                                    canvas_size,
                                    assets,
                                )
                                .await?
                            else {
                                *unsupported = true;
                                continue;
                            };
                            texture_layers.push(GpuSceneTextureLayer {
                                source: GpuSceneTextureSource::Gpu(source),
                                transform: Affine2::identity(),
                                opacity: 1.0,
                                blend: SceneBlendMode::Normal,
                                matte: Some(GpuSceneTextureMatte {
                                    texture: matte,
                                    mode: GpuSceneMatteMode::Alpha,
                                    invert: scene_mask_mode_inverts(&group.mask_mode),
                                }),
                            });
                        } else {
                            let group_deform = group_deform
                                .as_ref()
                                .map(|grid| transform_deform_grid(grid, group_transform));
                            let child_deform = group_deform.as_ref().or(deform);
                            self.collect_gpu_scene_native_commands(
                                &group.children,
                                group_transform,
                                child_deform,
                                opacity,
                                time_norm,
                                time_sec,
                                canvas_size,
                                assets,
                                primitives,
                                texture_layers,
                                text_requests,
                                unsupported,
                            )
                            .await?;
                        }
                    }
                    SceneNode::Camera(camera) => {
                        let opacity = (eval_scene_number(&camera.opacity, time_norm, time_sec)?
                            * inherited_opacity)
                            .clamp(0.0, 1.0);
                        if opacity <= 0.0001 {
                            continue;
                        }
                        let camera_transform = camera_transform(
                            camera,
                            &camera.children,
                            canvas_size.0,
                            canvas_size.1,
                            time_norm,
                            time_sec,
                        )?;
                        self.collect_gpu_scene_native_commands(
                            &camera.children,
                            transform.mul(camera_transform),
                            deform,
                            opacity,
                            time_norm,
                            time_sec,
                            canvas_size,
                            assets,
                            primitives,
                            texture_layers,
                            text_requests,
                            unsupported,
                        )
                        .await?;
                    }
                    SceneNode::Character(character) => {
                        let opacity = (eval_scene_number(&character.opacity, time_norm, time_sec)?
                            * inherited_opacity)
                            .clamp(0.0, 1.0);
                        if opacity <= 0.0001 {
                            continue;
                        }
                        let character_transform = transform.mul(scene_character_local_transform(
                            character, time_norm, time_sec,
                        )?);
                        self.collect_gpu_scene_native_commands(
                            &character.children,
                            character_transform,
                            deform,
                            opacity,
                            time_norm,
                            time_sec,
                            canvas_size,
                            assets,
                            primitives,
                            texture_layers,
                            text_requests,
                            unsupported,
                        )
                        .await?;
                    }
                    SceneNode::Part(part) => {
                        let opacity = (eval_scene_number(&part.opacity, time_norm, time_sec)?
                            * inherited_opacity)
                            .clamp(0.0, 1.0);
                        if opacity <= 0.0001 {
                            continue;
                        }
                        let x = eval_scene_number(&part.x, time_norm, time_sec)?;
                        let y = eval_scene_number(&part.y, time_norm, time_sec)?;
                        let rotation = eval_scene_number(&part.rotation, time_norm, time_sec)?;
                        let scale =
                            eval_scene_number(&part.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
                        let anchor_x = eval_scene_number(&part.anchor_x, time_norm, time_sec)?;
                        let anchor_y = eval_scene_number(&part.anchor_y, time_norm, time_sec)?;
                        let part_transform = transform
                            .mul(Affine2::translate(x, y))
                            .mul(Affine2::rotate_deg(rotation))
                            .mul(Affine2::scale(scale))
                            .mul(Affine2::translate(-anchor_x, -anchor_y));
                        self.collect_gpu_scene_native_commands(
                            &part.children,
                            part_transform,
                            deform,
                            opacity,
                            time_norm,
                            time_sec,
                            canvas_size,
                            assets,
                            primitives,
                            texture_layers,
                            text_requests,
                            unsupported,
                        )
                        .await?;
                    }
                    SceneNode::Repeat(repeat) => {
                        let count = eval_repeat_count(&repeat.count, time_norm, time_sec)?;
                        let x = eval_scene_number(&repeat.x, time_norm, time_sec)?;
                        let y = eval_scene_number(&repeat.y, time_norm, time_sec)?;
                        let rotation = eval_scene_number(&repeat.rotation, time_norm, time_sec)?;
                        let scale = eval_scene_number(&repeat.scale, time_norm, time_sec)?
                            .clamp(0.001, 64.0);
                        let opacity = eval_scene_number(&repeat.opacity, time_norm, time_sec)?;
                        let x_step = eval_scene_number(&repeat.x_step, time_norm, time_sec)?;
                        let y_step = eval_scene_number(&repeat.y_step, time_norm, time_sec)?;
                        let rotation_step =
                            eval_scene_number(&repeat.rotation_step, time_norm, time_sec)?;
                        let scale_step =
                            eval_scene_number(&repeat.scale_step, time_norm, time_sec)?;
                        let opacity_step =
                            eval_scene_number(&repeat.opacity_step, time_norm, time_sec)?;
                        for index in 0..count {
                            let i = index as f32;
                            let copy_opacity =
                                ((opacity + opacity_step * i) * inherited_opacity).clamp(0.0, 1.0);
                            if copy_opacity <= 0.0001 {
                                continue;
                            }
                            let repeat_transform = transform
                                .mul(Affine2::translate(x + x_step * i, y + y_step * i))
                                .mul(Affine2::rotate_deg(rotation + rotation_step * i))
                                .mul(Affine2::scale((scale + scale_step * i).clamp(0.001, 64.0)));
                            self.collect_gpu_scene_native_commands(
                                &repeat.children,
                                repeat_transform,
                                deform,
                                copy_opacity,
                                time_norm,
                                time_sec,
                                canvas_size,
                                assets,
                                primitives,
                                texture_layers,
                                text_requests,
                                unsupported,
                            )
                            .await?;
                        }
                    }
                    SceneNode::Use(use_node) => {
                        let opacity = (eval_scene_number(&use_node.opacity, time_norm, time_sec)?
                            * inherited_opacity)
                            .clamp(0.0, 1.0);
                        if opacity <= 0.0001 {
                            continue;
                        }
                        let Some(children) = self.scene_components.get(&use_node.ref_id).cloned()
                        else {
                            continue;
                        };
                        let use_transform = transform
                            .mul(scene_use_local_transform(use_node, time_norm, time_sec)?);
                        let primitive_start = primitives.len();
                        let texture_start = texture_layers.len();
                        let text_start = text_requests.len();
                        self.collect_gpu_scene_native_commands(
                            &children,
                            use_transform,
                            deform,
                            opacity,
                            time_norm,
                            time_sec,
                            canvas_size,
                            assets,
                            primitives,
                            texture_layers,
                            text_requests,
                            unsupported,
                        )
                        .await?;

                        let use_blend = parse_scene_blend(&use_node.blend)?;
                        if use_blend != SceneBlendMode::Normal {
                            for primitive in &mut primitives[primitive_start..] {
                                primitive.blend = use_blend;
                            }
                            for layer in &mut texture_layers[texture_start..] {
                                layer.blend = use_blend;
                            }
                            if text_requests.len() > text_start {
                                // Text is rasterized as a normal texture layer later; fall back
                                // rather than silently losing a non-normal Use-level blend.
                                *unsupported = true;
                            }
                        }
                    }
                    _ => {
                        let mut overlays = Vec::<CpuSceneOverlay>::new();
                        collect_gpu_scene_commands(
                            std::slice::from_ref(node),
                            transform,
                            deform,
                            inherited_opacity,
                            time_norm,
                            time_sec,
                            canvas_size,
                            &self.gradient_defs,
                            &self.palette_defs,
                            &self.scene_components,
                            primitives,
                            text_requests,
                            &mut overlays,
                        )?;
                        if !overlays.is_empty() {
                            *unsupported = true;
                        }
                    }
                }
            }
            Ok(())
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_gpu_scene_native_commands_depth_sorted<'a>(
        &'a mut self,
        nodes: &'a [SceneNode],
        transform: Affine2,
        deform: Option<&'a EvaluatedDeformGrid>,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
        canvas_size: (u32, u32),
        assets: &'a mut GpuSceneNativeAssets,
        primitives: &'a mut Vec<GpuScenePrimitive>,
        texture_layers: &'a mut Vec<GpuSceneTextureLayer>,
        text_requests: &'a mut Vec<GpuSceneTextRequest>,
        unsupported: &'a mut bool,
        depth: Option<SceneDepthContext<'a>>,
    ) -> Pin<Box<dyn Future<Output = Result<(), MotionLoomSceneRenderError>> + 'a>> {
        Box::pin(async move {
            let Some(depth) = depth else {
                return self
                    .collect_gpu_scene_native_commands_with_depth(
                        nodes,
                        transform,
                        deform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        assets,
                        primitives,
                        texture_layers,
                        text_requests,
                        unsupported,
                        None,
                    )
                    .await;
            };
            let mut layer_items = Vec::<(usize, f32, &SceneNode)>::new();
            for (index, node) in nodes.iter().enumerate() {
                if let SceneNode::Layer(layer) = node {
                    layer_items.push((
                        index,
                        scene_layer_effective_z_depth(layer, depth, time_norm, time_sec)?,
                        node,
                    ));
                } else {
                    self.collect_gpu_scene_native_commands_with_depth(
                        std::slice::from_ref(node),
                        transform,
                        deform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        assets,
                        primitives,
                        texture_layers,
                        text_requests,
                        unsupported,
                        Some(depth),
                    )
                    .await?;
                }
            }
            layer_items.sort_by(|(a_order, a_depth, _), (b_order, b_depth, _)| {
                b_depth
                    .total_cmp(a_depth)
                    .then_with(|| a_order.cmp(b_order))
            });
            for (_, _, node) in layer_items {
                self.collect_gpu_scene_native_commands_with_depth(
                    std::slice::from_ref(node),
                    transform,
                    deform,
                    inherited_opacity,
                    time_norm,
                    time_sec,
                    canvas_size,
                    assets,
                    primitives,
                    texture_layers,
                    text_requests,
                    unsupported,
                    Some(depth),
                )
                .await?;
            }
            Ok(())
        })
    }

    async fn render_gpu_mask_texture(
        &mut self,
        mask: &MaskNode,
        transform: Affine2,
        time_norm: f32,
        time_sec: f32,
        canvas_size: (u32, u32),
    ) -> Result<Option<GpuSceneNativeTexture>, MotionLoomSceneRenderError> {
        let opacity = eval_scene_number(&mask.opacity, time_norm, time_sec)?.clamp(0.0, 1.0);
        let mut primitives = Vec::<GpuScenePrimitive>::new();
        if opacity > 0.0001 {
            let color = [255, 255, 255, 255];
            match mask.shape.trim().to_ascii_lowercase().as_str() {
                "circle" => {
                    let x = eval_scene_number(&mask.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&mask.y, time_norm, time_sec)?;
                    let radius = eval_scene_number(&mask.radius, time_norm, time_sec)?.max(0.0);
                    primitives.push(GpuScenePrimitive {
                        kind: GPU_SHAPE_CIRCLE_FILL,
                        transform,
                        shape: [x, y, radius, 0.0],
                        radius: 0.0,
                        stroke_width: 0.0,
                        blur: 0.0,
                        color,
                        opacity,
                        blend: SceneBlendMode::Normal,
                        gradient: None,
                        line_t0: 0.0,
                        line_t1: 1.0,
                        taper_start: 0.0,
                        taper_end: 0.0,
                    });
                }
                "ellipse" | "oval" => {
                    let x = eval_scene_number(&mask.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&mask.y, time_norm, time_sec)?;
                    let width = eval_scene_number(&mask.width, time_norm, time_sec)?.max(0.0);
                    let height = eval_scene_number(&mask.height, time_norm, time_sec)?.max(0.0);
                    let subpaths = vec![ellipse_polygon(
                        x + width * 0.5,
                        y + height * 0.5,
                        width * 0.5,
                        height * 0.5,
                    )];
                    push_gpu_filled_path_triangles(
                        &mut primitives,
                        transform,
                        &subpaths,
                        color,
                        opacity,
                        None,
                    );
                }
                "path" => {
                    let Some(d) = mask.d.as_deref() else {
                        return Ok(None);
                    };
                    let path = PathNode {
                        id: mask.id.clone(),
                        d: d.to_string(),
                        fill: Some("#ffffff".to_string()),
                        stroke: "none".to_string(),
                        stroke_width: "0".to_string(),
                        line_cap: "round".to_string(),
                        line_join: "round".to_string(),
                        trim_start: "0".to_string(),
                        trim_end: "1".to_string(),
                        taper_start: "0".to_string(),
                        taper_end: "0".to_string(),
                        stroke_style: "clean".to_string(),
                        stroke_roughness: "0".to_string(),
                        stroke_copies: "1".to_string(),
                        stroke_texture: "0".to_string(),
                        stroke_bristles: "1".to_string(),
                        stroke_pressure: "1".to_string(),
                        stroke_pressure_min: "1".to_string(),
                        stroke_pressure_curve: "1".to_string(),
                        opacity: mask.opacity.clone(),
                        blend: "normal".to_string(),
                        brush: None,
                        x: "0".to_string(),
                        y: "0".to_string(),
                        rotation: "0".to_string(),
                        scale: "1".to_string(),
                        scale_x: "1".to_string(),
                        scale_y: "1".to_string(),
                        skew_x: "0".to_string(),
                        skew_y: "0".to_string(),
                        transform_origin_x: "0".to_string(),
                        transform_origin_y: "0".to_string(),
                    };
                    push_gpu_path_commands(
                        &path,
                        transform,
                        None,
                        1.0,
                        time_norm,
                        time_sec,
                        &self.gradient_defs,
                        &mut primitives,
                    )?;
                }
                _ => {
                    let x = eval_scene_number(&mask.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&mask.y, time_norm, time_sec)?;
                    let width = eval_scene_number(&mask.width, time_norm, time_sec)?.max(0.0);
                    let height = eval_scene_number(&mask.height, time_norm, time_sec)?.max(0.0);
                    let radius = eval_scene_number(&mask.radius, time_norm, time_sec)?.max(0.0);
                    primitives.push(GpuScenePrimitive {
                        kind: GPU_SHAPE_RECT_FILL,
                        transform,
                        shape: [x, y, width, height],
                        radius,
                        stroke_width: 0.0,
                        blur: 0.0,
                        color,
                        opacity,
                        blend: SceneBlendMode::Normal,
                        gradient: None,
                        line_t0: 0.0,
                        line_t1: 1.0,
                        taper_start: 0.0,
                        taper_end: 0.0,
                    });
                }
            }
        }

        self.ensure_gpu_compositor_size(canvas_size.0.max(1), canvas_size.1.max(1))
            .await?;
        let compositor =
            self.gpu_compositor
                .as_mut()
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                })?;
        let mut texture =
            compositor.render_scene_content_to_texture(&primitives, &[], [0, 0, 0, 0])?;
        let feather = eval_scene_number(&mask.feather, time_norm, time_sec)?.max(0.0);
        if feather > 0.01 {
            texture = compositor
                .apply_gpu_blur_texture(&texture, &[(true, feather), (false, feather)])?;
        }
        Ok(Some(texture))
    }

    fn render_cpu_scene_nodes_scaled(
        &mut self,
        nodes: &[SceneNode],
        output_size: (u32, u32),
        logical_size: (u32, u32),
        time_norm: f32,
        time_sec: f32,
        background: [u8; 4],
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let mut logical_canvas = RgbaImage::from_pixel(
            logical_size.0.max(1),
            logical_size.1.max(1),
            Rgba(background),
        );
        self.draw_scene_nodes(&mut logical_canvas, nodes, time_norm, time_sec, 1.0)?;
        Ok(fit_logical_canvas_to_output(&logical_canvas, output_size))
    }

    async fn apply_scene_post_pass_multi(
        &mut self,
        inputs: &[RgbaImage],
        pass: &PassNode,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let effect = pass.effect.to_ascii_lowercase();
        if effect == "over" || effect == "composite.over" {
            return Ok(apply_over_pass(inputs));
        }
        if effect == "hsla" || effect == "hsla_overlay" || effect == "color.hsla" {
            return apply_hsla_pass(&inputs[0], pass, time_norm, time_sec);
        }
        if effect == "blur" || effect == "gaussian_blur" || effect == "gaussian_5tap_blur" {
            let sigma = pass_param_expr(pass, "sigma")
                .map(|expr| eval_scene_number(expr, time_norm, time_sec))
                .transpose()?
                .unwrap_or(2.0)
                .clamp(0.0, 64.0);
            if self.profile.uses_gpu_compositor() {
                self.ensure_gpu_compositor_size(
                    inputs[0].width().max(1),
                    inputs[0].height().max(1),
                )
                .await?;
                let compositor = self.gpu_compositor.as_mut().ok_or_else(|| {
                    MotionLoomSceneRenderError::GpuRender {
                        message: "GPU compositor was not initialized".to_string(),
                    }
                })?;
                return compositor
                    .apply_gpu_blur_passes(&inputs[0], &[(true, sigma), (false, sigma)])
                    .await;
            }
            let blurred = apply_box_blur_pass(&inputs[0], sigma, true);
            return Ok(apply_box_blur_pass(&blurred, sigma, false));
        }
        if scene_post_bloom_params(pass, time_norm, time_sec)?.is_some() {
            return self
                .apply_scene_bloom_pass(&inputs[0], pass, time_norm, time_sec)
                .await;
        }
        self.apply_scene_post_pass(&inputs[0], pass, time_norm, time_sec)
            .await
    }

    async fn apply_scene_post_pass(
        &mut self,
        input: &RgbaImage,
        pass: &PassNode,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        if let Some(blur_passes) = scene_post_blur_passes(pass, time_norm, time_sec)?
            && self.profile.uses_gpu_compositor()
        {
            self.ensure_gpu_compositor_size(input.width().max(1), input.height().max(1))
                .await?;
            let compositor = self.gpu_compositor.as_mut().ok_or_else(|| {
                MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                }
            })?;
            return compositor.apply_gpu_blur_passes(input, &blur_passes).await;
        }
        if scene_post_bloom_params(pass, time_norm, time_sec)?.is_some() {
            return self
                .apply_scene_bloom_pass(input, pass, time_norm, time_sec)
                .await;
        }
        let effect = pass.effect.to_ascii_lowercase();
        if effect == "opacity" || effect == "composite.opacity" {
            let opacity = pass_param_expr(pass, "opacity")
                .map(|expr| eval_scene_number(expr, time_norm, time_sec))
                .transpose()?
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            if self.profile.uses_gpu_compositor() {
                self.ensure_gpu_compositor_size(input.width().max(1), input.height().max(1))
                    .await?;
                let compositor = self.gpu_compositor.as_mut().ok_or_else(|| {
                    MotionLoomSceneRenderError::GpuRender {
                        message: "GPU compositor was not initialized".to_string(),
                    }
                })?;
                return compositor.apply_gpu_opacity_pass(input, opacity).await;
            }
        }
        if is_color_key_alpha_effect(&effect) {
            return apply_scene_post_pass(input, pass, time_norm, time_sec);
        }
        if self.profile.uses_gpu_compositor() {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: format!(
                    "GPU Render is strict for MotionLoom: post pass '{}' is not GPU-native yet. Use Compatibility Render (CPU) explicitly, or remove/replace this pass.",
                    pass.effect
                ),
            });
        }
        apply_scene_post_pass(input, pass, time_norm, time_sec)
    }

    async fn apply_scene_bloom_pass(
        &mut self,
        input: &RgbaImage,
        pass: &PassNode,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let Some(params) = scene_post_bloom_params(pass, time_norm, time_sec)? else {
            return Ok(input.clone());
        };
        let prefiltered = build_scene_bloom_prefilter(input, params.threshold);
        let blurred = if self.profile.uses_gpu_compositor() {
            self.ensure_gpu_compositor_size(input.width().max(1), input.height().max(1))
                .await?;
            let compositor = self.gpu_compositor.as_mut().ok_or_else(|| {
                MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                }
            })?;
            compositor
                .apply_gpu_blur_passes(&prefiltered, &[(true, params.sigma), (false, params.sigma)])
                .await?
        } else {
            let blurred_h = apply_box_blur_pass(&prefiltered, params.sigma, true);
            apply_box_blur_pass(&blurred_h, params.sigma, false)
        };
        Ok(composite_scene_bloom(input, &blurred, params.intensity))
    }

    fn draw_scene_nodes(
        &mut self,
        canvas: &mut RgbaImage,
        nodes: &[SceneNode],
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        self.draw_scene_nodes_with_depth(
            canvas,
            nodes,
            time_norm,
            time_sec,
            inherited_opacity,
            None,
        )
    }

    fn draw_scene_nodes_with_depth(
        &mut self,
        canvas: &mut RgbaImage,
        nodes: &[SceneNode],
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
        depth: Option<SceneDepthContext<'_>>,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let mut pending_shadow: Option<EvaluatedShadow> = None;
        for node in nodes {
            match node {
                SceneNode::Defs(defs) => {
                    self.register_defs_resources(defs)?;
                    pending_shadow = None;
                }
                SceneNode::Timeline(timeline) => {
                    let mut tracks = timeline
                        .children
                        .iter()
                        .filter_map(|node| match node {
                            SceneNode::Track(track) => Some(track),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    tracks.sort_by_key(|track| track.z);
                    let active_camera = active_scene_camera_from_tracks(
                        &tracks,
                        canvas.width(),
                        canvas.height(),
                        time_norm,
                        time_sec,
                    )?;
                    tracks.sort_by(|a, b| {
                        let a_world = is_scene_world_track(a);
                        let b_world = is_scene_world_track(b);
                        match (a_world, b_world) {
                            (true, true) => {
                                let a_depth =
                                    scene_depth_track_sort_key(&a.z_depth, time_norm, time_sec)
                                        .unwrap_or(0.0);
                                let b_depth =
                                    scene_depth_track_sort_key(&b.z_depth, time_norm, time_sec)
                                        .unwrap_or(0.0);
                                b_depth.total_cmp(&a_depth).then_with(|| a.z.cmp(&b.z))
                            }
                            _ => a.z.cmp(&b.z),
                        }
                    });
                    for track in tracks {
                        if is_scene_camera_track(track) {
                            continue;
                        }
                        if is_scene_world_track(track) {
                            self.draw_scene_nodes_with_depth(
                                canvas,
                                &track.children,
                                time_norm,
                                time_sec,
                                inherited_opacity,
                                Some(SceneDepthContext {
                                    active_camera,
                                    canvas_size: canvas.dimensions(),
                                    track_z_depth: &track.z_depth,
                                }),
                            )?;
                        } else {
                            self.draw_scene_nodes_with_depth(
                                canvas,
                                &track.children,
                                time_norm,
                                time_sec,
                                inherited_opacity,
                                depth,
                            )?;
                        }
                    }
                    pending_shadow = None;
                }
                SceneNode::Track(track) => {
                    self.draw_scene_nodes_with_depth(
                        canvas,
                        &track.children,
                        time_norm,
                        time_sec,
                        inherited_opacity,
                        depth,
                    )?;
                    pending_shadow = None;
                }
                SceneNode::Sequence(sequence) => {
                    if let Some((local_norm, local_sec)) =
                        scene_sequence_local_time(sequence, None, time_sec)
                    {
                        if depth.is_some() {
                            self.draw_depth_sorted_scene_nodes(
                                canvas,
                                &sequence.children,
                                local_norm,
                                local_sec,
                                inherited_opacity,
                                depth,
                            )?;
                        } else {
                            self.draw_scene_nodes_with_depth(
                                canvas,
                                &sequence.children,
                                local_norm,
                                local_sec,
                                inherited_opacity,
                                depth,
                            )?;
                        }
                    }
                    pending_shadow = None;
                }
                SceneNode::Chain(chain) => {
                    let mut cursor_ms = chain.from_ms as i64;
                    for child in &chain.children {
                        if let SceneNode::Sequence(sequence) = child {
                            if let Some((local_norm, local_sec)) =
                                scene_sequence_local_time(sequence, Some(cursor_ms), time_sec)
                            {
                                if depth.is_some() {
                                    self.draw_depth_sorted_scene_nodes(
                                        canvas,
                                        &sequence.children,
                                        local_norm,
                                        local_sec,
                                        inherited_opacity,
                                        depth,
                                    )?;
                                } else {
                                    self.draw_scene_nodes_with_depth(
                                        canvas,
                                        &sequence.children,
                                        local_norm,
                                        local_sec,
                                        inherited_opacity,
                                        depth,
                                    )?;
                                }
                            }
                            cursor_ms += sequence.duration_ms as i64 + chain.gap_ms;
                        }
                    }
                    pending_shadow = None;
                }
                SceneNode::Palette(_) => {
                    pending_shadow = None;
                }
                SceneNode::PixelGrid(grid) => {
                    self.draw_pixel_grid(
                        canvas,
                        grid,
                        Affine2::identity(),
                        inherited_opacity,
                        time_norm,
                        time_sec,
                    )?;
                    pending_shadow = None;
                }
                SceneNode::Text(text) => {
                    self.draw_text_with_opacity(
                        canvas,
                        text,
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                    pending_shadow = None;
                }
                SceneNode::Image(image) => {
                    self.draw_image_with_opacity(
                        canvas,
                        image,
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                    pending_shadow = None;
                }
                SceneNode::Svg(svg) => {
                    self.draw_svg_with_opacity(
                        canvas,
                        svg,
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                    pending_shadow = None;
                }
                SceneNode::Rect(rect) => {
                    self.draw_rect(
                        canvas,
                        rect,
                        pending_shadow.take(),
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                }
                SceneNode::Circle(circle) => {
                    self.draw_circle(
                        canvas,
                        circle,
                        pending_shadow.take(),
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                }
                SceneNode::Line(line) => {
                    self.draw_line(canvas, line, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Polyline(polyline) => {
                    self.draw_polyline(canvas, polyline, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Path(path) => {
                    self.draw_path(canvas, path, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::FaceJaw(face_jaw) => {
                    self.draw_face_jaw(
                        canvas,
                        face_jaw,
                        Affine2::identity(),
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                    pending_shadow = None;
                }
                SceneNode::Shadow(shadow) => {
                    pending_shadow = Some(evaluate_shadow(
                        shadow,
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?);
                }
                SceneNode::Group(group) => {
                    self.draw_group(canvas, group, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Part(part) => {
                    self.draw_part(canvas, part, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Repeat(repeat) => {
                    self.draw_repeat(canvas, repeat, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Mask(mask) => {
                    if let Some(id) = mask.id.as_deref() {
                        self.scene_masks.insert(id.to_string(), mask.clone());
                    }
                    if !mask.children.is_empty() {
                        self.draw_mask(canvas, mask, time_norm, time_sec, inherited_opacity)?;
                    }
                    pending_shadow = None;
                }
                SceneNode::Precompose(precompose) => {
                    self.scene_precompose_defs
                        .insert(precompose.id.clone(), precompose.clone());
                    pending_shadow = None;
                }
                SceneNode::Use(use_node) => {
                    self.draw_use(canvas, use_node, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Layer(layer) => {
                    if let Some(depth) = depth {
                        let z_depth =
                            scene_layer_effective_z_depth(layer, depth, time_norm, time_sec)?;
                        let depth_transform = scene_z_depth_transform(
                            depth.active_camera,
                            depth.canvas_size,
                            z_depth,
                        );
                        let source_size = depth
                            .active_camera
                            .map(|camera| (camera.layer_width, camera.layer_height))
                            .unwrap_or_else(|| canvas.dimensions());
                        let clip = depth.active_camera.map(|camera| camera.viewport);
                        self.draw_scene_layer_with_transform(
                            canvas,
                            layer,
                            SceneLayerDrawParams {
                                source_size,
                                base_transform: depth_transform,
                                clip,
                                time_norm,
                                time_sec,
                                inherited_opacity,
                            },
                        )?;
                    } else {
                        self.draw_scene_layer(
                            canvas,
                            layer,
                            time_norm,
                            time_sec,
                            inherited_opacity,
                        )?;
                    }
                    pending_shadow = None;
                }
                SceneNode::Camera(camera) => {
                    self.draw_camera(canvas, camera, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Character(character) => {
                    self.draw_character(
                        canvas,
                        character,
                        Affine2::identity(),
                        inherited_opacity,
                        time_norm,
                        time_sec,
                    )?;
                    pending_shadow = None;
                }
            }
        }
        Ok(())
    }

    fn draw_depth_sorted_scene_nodes(
        &mut self,
        canvas: &mut RgbaImage,
        nodes: &[SceneNode],
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
        depth: Option<SceneDepthContext<'_>>,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let Some(depth) = depth else {
            return self.draw_scene_nodes_with_depth(
                canvas,
                nodes,
                time_norm,
                time_sec,
                inherited_opacity,
                None,
            );
        };
        let mut layer_items = Vec::<(usize, f32, &SceneNode)>::new();
        for (index, node) in nodes.iter().enumerate() {
            if let SceneNode::Layer(layer) = node {
                layer_items.push((
                    index,
                    scene_layer_effective_z_depth(layer, depth, time_norm, time_sec)?,
                    node,
                ));
            } else {
                self.draw_scene_nodes_with_depth(
                    canvas,
                    std::slice::from_ref(node),
                    time_norm,
                    time_sec,
                    inherited_opacity,
                    Some(depth),
                )?;
            }
        }
        layer_items.sort_by(|(a_order, a_depth, _), (b_order, b_depth, _)| {
            b_depth
                .total_cmp(a_depth)
                .then_with(|| a_order.cmp(b_order))
        });
        for (_, _, node) in layer_items {
            self.draw_scene_nodes_with_depth(
                canvas,
                std::slice::from_ref(node),
                time_norm,
                time_sec,
                inherited_opacity,
                Some(depth),
            )?;
        }
        Ok(())
    }

    fn register_defs_resources(
        &mut self,
        defs: &DefsNode,
    ) -> Result<(), MotionLoomSceneRenderError> {
        for font in &defs.fonts {
            self.font_defs.insert(font.id.clone(), font.clone());
        }
        for filter in &defs.filters {
            self.filter_defs.insert(filter.id.clone(), filter.clone());
        }
        for mask in &defs.masks {
            if let Some(id) = mask.id.as_deref() {
                self.scene_masks.insert(id.to_string(), mask.clone());
            }
        }
        for component in &defs.components {
            self.scene_components
                .insert(component.id.clone(), component.children.clone());
        }
        for precompose in &defs.precomposes {
            self.scene_precompose_defs
                .insert(precompose.id.clone(), precompose.clone());
        }
        Ok(())
    }

    fn render_cpu_base_frame(
        &mut self,
        graph: &GraphScript,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let (w, h) = graph.size;
        let mut canvas = RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([0, 0, 0, 0]));

        for background in &graph.backgrounds {
            let color = parse_color(&background.color)?;
            for pixel in canvas.pixels_mut() {
                *pixel = Rgba(color);
            }
        }

        for image in &graph.images {
            self.draw_image(&mut canvas, image, time_norm, time_sec)?;
        }
        for svg in &graph.svgs {
            self.draw_svg(&mut canvas, svg, time_norm, time_sec)?;
        }
        Ok(canvas)
    }

    async fn render_gpu_base_frame(
        &mut self,
        graph: &GraphScript,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        if self.gpu_compositor.is_none() {
            self.gpu_compositor = Some(
                WgpuSceneCompositor::new(
                    graph.size.0.max(1),
                    graph.size.1.max(1),
                    self.asset_resolver.clone(),
                )
                .await?,
            );
        }

        let compositor =
            self.gpu_compositor
                .as_mut()
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                })?;
        let background = graph
            .backgrounds
            .last()
            .map(|background| parse_color(&background.color))
            .transpose()?
            .unwrap_or([0, 0, 0, 0]);
        compositor
            .render(graph, background, time_norm, time_sec)
            .await
    }

    async fn render_gpu_background_frame(
        &mut self,
        output_size: (u32, u32),
        color: [u8; 4],
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        self.ensure_gpu_compositor_size(output_size.0.max(1), output_size.1.max(1))
            .await?;
        let compositor =
            self.gpu_compositor
                .as_mut()
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                })?;
        compositor
            .render_scene_content(&[gpu_solid_primitive(color)], &[])
            .await
    }

    fn draw_text(
        &mut self,
        canvas: &mut RgbaImage,
        text: &TextNode,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        self.draw_text_with_opacity(canvas, text, time_norm, time_sec, 1.0)
    }

    fn draw_text_with_opacity(
        &mut self,
        canvas: &mut RgbaImage,
        text: &TextNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        self.draw_text_transformed(
            canvas,
            text,
            Affine2::identity(),
            inherited_opacity,
            time_norm,
            time_sec,
        )
    }

    fn draw_image(
        &mut self,
        canvas: &mut RgbaImage,
        image_node: &ImageNode,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        self.draw_image_with_opacity(canvas, image_node, time_norm, time_sec, 1.0)
    }

    fn draw_image_with_opacity(
        &mut self,
        canvas: &mut RgbaImage,
        image_node: &ImageNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&image_node.opacity, time_norm, time_sec)?
            * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }

        let scale = eval_scene_number(&image_node.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
        let source = self.load_image_asset(&image_node.src)?;
        let target_w = ((source.width() as f32) * scale).round().max(1.0) as u32;
        let target_h = ((source.height() as f32) * scale).round().max(1.0) as u32;

        let x_base = resolve_axis(
            &image_node.x,
            canvas.width() as f32,
            target_w as f32,
            time_norm,
            time_sec,
        )?;
        let y_base = resolve_axis(
            &image_node.y,
            canvas.height() as f32,
            target_h as f32,
            time_norm,
            time_sec,
        )?;
        if target_w == source.width() && target_h == source.height() {
            draw_rgba_image(canvas, source, x_base, y_base, opacity);
        } else {
            let scaled = image::imageops::resize(source, target_w, target_h, FilterType::Lanczos3);
            draw_rgba_image(canvas, &scaled, x_base, y_base, opacity);
        }
        Ok(())
    }

    fn draw_svg(
        &mut self,
        canvas: &mut RgbaImage,
        svg_node: &SvgNode,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        self.draw_svg_with_opacity(canvas, svg_node, time_norm, time_sec, 1.0)
    }

    fn draw_svg_with_opacity(
        &mut self,
        canvas: &mut RgbaImage,
        svg_node: &SvgNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&svg_node.opacity, time_norm, time_sec)?
            * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }

        let scale = eval_scene_number(&svg_node.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
        let source = self.load_svg_asset(&svg_node.src)?;
        let target_w = ((source.width() as f32) * scale).round().max(1.0) as u32;
        let target_h = ((source.height() as f32) * scale).round().max(1.0) as u32;

        let x_base = resolve_axis(
            &svg_node.x,
            canvas.width() as f32,
            target_w as f32,
            time_norm,
            time_sec,
        )?;
        let y_base = resolve_axis(
            &svg_node.y,
            canvas.height() as f32,
            target_h as f32,
            time_norm,
            time_sec,
        )?;
        if target_w == source.width() && target_h == source.height() {
            draw_rgba_image(canvas, source, x_base, y_base, opacity);
        } else {
            let scaled = image::imageops::resize(source, target_w, target_h, FilterType::Lanczos3);
            draw_rgba_image(canvas, &scaled, x_base, y_base, opacity);
        }
        Ok(())
    }

    fn draw_group(
        &mut self,
        canvas: &mut RgbaImage,
        group: &GroupNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&group.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let transform = scene_group_local_transform(group, time_norm, time_sec)?;
        let deform_grid = eval_group_deform_grid(group, time_norm, time_sec)?;

        if group.mask.is_none() && deform_grid.is_none() {
            self.draw_character_nodes_vector(
                canvas,
                &group.children,
                transform,
                opacity,
                time_norm,
                time_sec,
            )?;
            return Ok(());
        }

        let mut layer = RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
        self.draw_scene_nodes(&mut layer, &group.children, time_norm, time_sec, opacity)?;
        if let Some(mask_id) = group.mask.as_deref()
            && let Some(mask_alpha) = self.scene_mask_alpha(
                mask_id,
                canvas.width(),
                canvas.height(),
                time_norm,
                time_sec,
            )?
        {
            let invert = group.mask_mode.trim().eq_ignore_ascii_case("inverse")
                || group.mask_mode.trim().eq_ignore_ascii_case("invert")
                || group.mask_mode.trim().eq_ignore_ascii_case("inverted");
            apply_alpha_mask_with_invert(&mut layer, &mask_alpha, invert);
        }
        if let Some(deform_grid) = deform_grid {
            layer = apply_deform_grid(&layer, &deform_grid);
        }
        composite_layer_affine(canvas, &layer, transform);
        Ok(())
    }

    fn render_precompose_image(
        &mut self,
        precompose: &PrecomposeNode,
        fallback_size: (u32, u32),
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let size = precompose.size.unwrap_or(fallback_size);
        let mut layer = RgbaImage::from_pixel(size.0.max(1), size.1.max(1), Rgba([0, 0, 0, 0]));
        self.draw_scene_nodes(&mut layer, &precompose.children, time_norm, time_sec, 1.0)?;
        Ok(layer)
    }

    fn render_precompose_instance(
        &mut self,
        source_id: &str,
        fallback_size: (u32, u32),
        layer: &SceneLayerNode,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<Option<RgbaImage>, MotionLoomSceneRenderError> {
        let Some(precompose) = self.scene_precompose_defs.get(source_id).cloned() else {
            return Ok(self.scene_precomposes.get(source_id).cloned());
        };
        let Some((source_norm, source_sec)) =
            scene_layer_source_time(layer, &precompose, time_norm, time_sec)?
        else {
            return Ok(None);
        };
        self.render_precompose_image(&precompose, fallback_size, source_norm, source_sec)
            .map(Some)
    }

    fn draw_use(
        &mut self,
        canvas: &mut RgbaImage,
        use_node: &UseNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        self.draw_use_transformed(
            canvas,
            use_node,
            Affine2::identity(),
            time_norm,
            time_sec,
            inherited_opacity,
        )
    }

    fn draw_use_transformed(
        &mut self,
        canvas: &mut RgbaImage,
        use_node: &UseNode,
        base_transform: Affine2,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&use_node.opacity, time_norm, time_sec)?
            * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let Some(children) = self.scene_components.get(&use_node.ref_id).cloned() else {
            return Ok(());
        };

        let mut layer = RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
        self.draw_scene_nodes(&mut layer, &children, time_norm, time_sec, opacity)?;
        let transform =
            base_transform.mul(scene_use_local_transform(use_node, time_norm, time_sec)?);
        let blend = parse_scene_blend(&use_node.blend)?;
        composite_layer_affine_blend(canvas, &layer, transform, 1.0, blend);
        Ok(())
    }

    fn draw_scene_layer(
        &mut self,
        canvas: &mut RgbaImage,
        layer: &SceneLayerNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        self.draw_scene_layer_with_transform(
            canvas,
            layer,
            SceneLayerDrawParams {
                source_size: canvas.dimensions(),
                base_transform: Affine2::identity(),
                clip: None,
                time_norm,
                time_sec,
                inherited_opacity,
            },
        )
    }

    fn draw_scene_layer_with_transform(
        &mut self,
        canvas: &mut RgbaImage,
        layer: &SceneLayerNode,
        params: SceneLayerDrawParams,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&layer.opacity, params.time_norm, params.time_sec)?
            * params.inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let mut source = if let Some(source_id) = layer.source.as_deref() {
            let Some(source) = self.render_precompose_instance(
                source_id,
                params.source_size,
                layer,
                params.time_norm,
                params.time_sec,
            )?
            else {
                return Ok(());
            };
            source
        } else {
            if layer.children.is_empty() {
                return Ok(());
            }
            RgbaImage::from_pixel(
                params.source_size.0,
                params.source_size.1,
                Rgba([0, 0, 0, 0]),
            )
        };
        if !layer.children.is_empty() {
            self.draw_scene_nodes(
                &mut source,
                &layer.children,
                params.time_norm,
                params.time_sec,
                1.0,
            )?;
        }
        if let Some(filter_id) = layer.effect.as_deref() {
            source =
                self.apply_scene_filter(&source, filter_id, params.time_norm, params.time_sec)?;
        }
        if let Some(mask_id) = layer.mask.as_deref()
            && let Some(mask_alpha) = self.scene_mask_alpha(
                mask_id,
                source.width(),
                source.height(),
                params.time_norm,
                params.time_sec,
            )?
        {
            apply_alpha_mask_with_invert(
                &mut source,
                &mask_alpha,
                scene_mask_mode_inverts(&layer.mask_mode),
            );
        }
        if let Some(matte_id) = layer.matte.as_deref()
            && let Some(matte_alpha) = self.scene_matte_alpha(
                matte_id,
                (source.width(), source.height()),
                &layer.matte_mode,
                Some(layer),
                params.time_norm,
                params.time_sec,
            )?
        {
            apply_alpha_mask_with_invert(
                &mut source,
                &matte_alpha,
                scene_bool(&layer.invert_matte),
            );
        }

        let transform = params.base_transform.mul(scene_layer_local_transform(
            layer,
            params.time_norm,
            params.time_sec,
        )?);
        let blend = parse_scene_blend(&layer.blend)?;
        composite_layer_affine_blend_clipped(
            canvas,
            &source,
            transform,
            opacity,
            blend,
            params.clip,
        );
        Ok(())
    }

    fn apply_scene_filter(
        &self,
        input: &RgbaImage,
        filter_id: &str,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let Some(filter) = self.filter_defs.get(filter_id) else {
            return Ok(input.clone());
        };
        let mut output = input.clone();
        for step in &filter.steps {
            output = apply_scene_filter_step(&output, step, time_norm, time_sec)?;
        }
        Ok(output)
    }

    fn scene_mask_alpha(
        &mut self,
        id: &str,
        width: u32,
        height: u32,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<Option<RgbaImage>, MotionLoomSceneRenderError> {
        let Some(mask) = self.scene_masks.get(id).cloned() else {
            return Ok(None);
        };
        self.render_mask_alpha(
            width,
            height,
            &mask,
            Affine2::identity(),
            time_norm,
            time_sec,
        )
        .map(Some)
    }

    fn scene_matte_alpha(
        &mut self,
        id: &str,
        size: (u32, u32),
        mode: &str,
        layer: Option<&SceneLayerNode>,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<Option<RgbaImage>, MotionLoomSceneRenderError> {
        let normalized = mode.trim().to_ascii_lowercase().replace('_', "-");
        let (width, height) = size;
        if let Some(mask) = self.scene_mask_alpha(id, width, height, time_norm, time_sec)? {
            return Ok(Some(mask));
        };
        let matte = if let Some(precompose_layer) = layer {
            self.render_precompose_instance(id, size, precompose_layer, time_norm, time_sec)?
        } else {
            self.scene_precomposes.get(id).cloned()
        };
        let Some(matte) = matte else {
            return Ok(None);
        };
        let mut alpha =
            RgbaImage::from_pixel(width.max(1), height.max(1), Rgba([255, 255, 255, 0]));
        let w = alpha.width().min(matte.width());
        let h = alpha.height().min(matte.height());
        for y in 0..h {
            for x in 0..w {
                let px = matte.get_pixel(x, y).0;
                let src_alpha = px[3] as f32 / 255.0;
                let amount = if normalized == "luma" || normalized == "luminance" {
                    ((0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32)
                        / 255.0)
                        * src_alpha
                } else {
                    src_alpha
                };
                alpha.put_pixel(
                    x,
                    y,
                    Rgba([
                        255,
                        255,
                        255,
                        (amount * 255.0).round().clamp(0.0, 255.0) as u8,
                    ]),
                );
            }
        }
        Ok(Some(alpha))
    }

    fn draw_part(
        &mut self,
        canvas: &mut RgbaImage,
        part: &PartNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&part.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let x = eval_scene_number(&part.x, time_norm, time_sec)?;
        let y = eval_scene_number(&part.y, time_norm, time_sec)?;
        let rotation = eval_scene_number(&part.rotation, time_norm, time_sec)?;
        let scale = eval_scene_number(&part.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
        let anchor_x = eval_scene_number(&part.anchor_x, time_norm, time_sec)?;
        let anchor_y = eval_scene_number(&part.anchor_y, time_norm, time_sec)?;

        let mut layer = RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
        self.draw_scene_nodes(&mut layer, &part.children, time_norm, time_sec, opacity)?;
        composite_transformed_layer_anchored(
            canvas, &layer, x, y, rotation, scale, anchor_x, anchor_y,
        );
        Ok(())
    }

    fn draw_repeat(
        &mut self,
        canvas: &mut RgbaImage,
        repeat: &RepeatNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let count = eval_repeat_count(&repeat.count, time_norm, time_sec)?;
        if count == 0 {
            return Ok(());
        }

        let x = eval_scene_number(&repeat.x, time_norm, time_sec)?;
        let y = eval_scene_number(&repeat.y, time_norm, time_sec)?;
        let rotation = eval_scene_number(&repeat.rotation, time_norm, time_sec)?;
        let scale = eval_scene_number(&repeat.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
        let opacity = eval_scene_number(&repeat.opacity, time_norm, time_sec)?;
        let x_step = eval_scene_number(&repeat.x_step, time_norm, time_sec)?;
        let y_step = eval_scene_number(&repeat.y_step, time_norm, time_sec)?;
        let rotation_step = eval_scene_number(&repeat.rotation_step, time_norm, time_sec)?;
        let scale_step = eval_scene_number(&repeat.scale_step, time_norm, time_sec)?;
        let opacity_step = eval_scene_number(&repeat.opacity_step, time_norm, time_sec)?;

        for index in 0..count {
            let i = index as f32;
            let copy_opacity = ((opacity + opacity_step * i) * inherited_opacity).clamp(0.0, 1.0);
            if copy_opacity <= 0.0001 {
                continue;
            }
            let copy_scale = (scale + scale_step * i).clamp(0.001, 64.0);
            let mut layer =
                RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
            self.draw_scene_nodes(
                &mut layer,
                &repeat.children,
                time_norm,
                time_sec,
                copy_opacity,
            )?;
            composite_transformed_layer(
                canvas,
                &layer,
                x + x_step * i,
                y + y_step * i,
                rotation + rotation_step * i,
                copy_scale,
            );
        }
        Ok(())
    }

    fn draw_mask(
        &mut self,
        canvas: &mut RgbaImage,
        mask: &MaskNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&mask.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let mut layer = RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
        self.draw_scene_nodes(&mut layer, &mask.children, time_norm, time_sec, opacity)?;
        let mask_alpha = self.render_mask_alpha(
            canvas.width(),
            canvas.height(),
            mask,
            Affine2::identity(),
            time_norm,
            time_sec,
        )?;
        apply_alpha_mask(&mut layer, &mask_alpha);
        composite_layer(canvas, &layer);
        Ok(())
    }

    fn draw_camera(
        &mut self,
        canvas: &mut RgbaImage,
        camera: &CameraNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&camera.opacity, time_norm, time_sec)?
            * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let canvas_w = canvas.width();
        let canvas_h = canvas.height();
        let transform = camera_transform(
            camera,
            &camera.children,
            canvas_w,
            canvas_h,
            time_norm,
            time_sec,
        )?;
        let world_bounds = camera_world_bounds(camera, canvas_w, canvas_h, time_norm, time_sec)?;
        let layer_w = world_bounds
            .map(|rect| canvas_w.max((rect.x + rect.width).ceil().max(canvas_w as f32) as u32 + 2))
            .unwrap_or_else(|| canvas_w.saturating_add(2));
        let layer_h = world_bounds
            .map(|rect| canvas_h.max((rect.y + rect.height).ceil().max(canvas_h as f32) as u32 + 2))
            .unwrap_or_else(|| canvas_h.saturating_add(2));
        let mut layer = RgbaImage::from_pixel(layer_w, layer_h, Rgba([0, 0, 0, 0]));
        self.draw_scene_nodes(&mut layer, &camera.children, time_norm, time_sec, opacity)?;
        let viewport = camera_viewport(camera, canvas_w, canvas_h, time_norm, time_sec)?;
        composite_layer_affine_clipped(canvas, &layer, transform, Some(viewport));
        Ok(())
    }

    fn cached_path_subpaths(
        &mut self,
        data: &str,
    ) -> Result<Vec<Vec<Point2>>, MotionLoomSceneRenderError> {
        if let Some(cached) = self.path_cache.get(data) {
            return Ok(cached.clone());
        }
        let parsed = parse_path_subpaths(data)?;
        self.path_cache.insert(data.to_string(), parsed.clone());
        Ok(parsed)
    }

    fn cached_polyline_points(
        &mut self,
        points: &str,
    ) -> Result<Vec<Point2>, MotionLoomSceneRenderError> {
        if let Some(cached) = self.polyline_cache.get(points) {
            return Ok(cached.clone());
        }
        let parsed = parse_polyline_points(points)?;
        self.polyline_cache
            .insert(points.to_string(), parsed.clone());
        Ok(parsed)
    }

    fn render_mask_alpha(
        &mut self,
        width: u32,
        height: u32,
        mask: &MaskNode,
        transform: Affine2,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let mut alpha = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]));
        let opacity = eval_scene_number(&mask.opacity, time_norm, time_sec)?.clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(alpha);
        }
        let mask_color = [255, 255, 255, (opacity * 255.0).round() as u8];
        match mask.shape.trim().to_ascii_lowercase().as_str() {
            "circle" => {
                let x = eval_scene_number(&mask.x, time_norm, time_sec)?;
                let y = eval_scene_number(&mask.y, time_norm, time_sec)?;
                let radius = eval_scene_number(&mask.radius, time_norm, time_sec)?.max(0.0)
                    * affine_uniform_scale(transform);
                let (x, y) = transform.transform_point(x, y);
                draw_circle(&mut alpha, x, y, radius, mask_color);
            }
            "ellipse" | "oval" => {
                let x = eval_scene_number(&mask.x, time_norm, time_sec)?;
                let y = eval_scene_number(&mask.y, time_norm, time_sec)?;
                let width = eval_scene_number(&mask.width, time_norm, time_sec)?.max(0.0);
                let height = eval_scene_number(&mask.height, time_norm, time_sec)?.max(0.0);
                let subpaths = vec![ellipse_polygon(
                    x + width * 0.5,
                    y + height * 0.5,
                    width * 0.5,
                    height * 0.5,
                )];
                draw_transformed_filled_polylines(&mut alpha, &subpaths, mask_color, transform);
            }
            "path" => {
                let Some(d) = mask.d.as_deref() else {
                    return Ok(alpha);
                };
                let path_d = eval_path_d(d, time_norm, time_sec)?;
                let subpaths = self.cached_path_subpaths(path_d.as_ref())?;
                draw_transformed_filled_polylines(&mut alpha, &subpaths, mask_color, transform);
            }
            _ => {
                let scale = affine_uniform_scale(transform);
                let x = eval_scene_number(&mask.x, time_norm, time_sec)?;
                let y = eval_scene_number(&mask.y, time_norm, time_sec)?;
                let w = eval_scene_number(&mask.width, time_norm, time_sec)?.max(0.0) * scale;
                let h = eval_scene_number(&mask.height, time_norm, time_sec)?.max(0.0) * scale;
                let radius = eval_scene_number(&mask.radius, time_norm, time_sec)?.max(0.0) * scale;
                let (x, y) = transform.transform_point(x, y);
                draw_rounded_rect(&mut alpha, x, y, w, h, radius, mask_color);
            }
        }
        let feather = eval_scene_number(&mask.feather, time_norm, time_sec)?.max(0.0);
        if feather > 0.01 {
            let blurred = apply_box_blur_pass(&alpha, feather, true);
            alpha = apply_box_blur_pass(&blurred, feather, false);
        }
        Ok(alpha)
    }

    fn draw_character(
        &mut self,
        canvas: &mut RgbaImage,
        character: &CharacterNode,
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&character.opacity, time_norm, time_sec)?
            * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let character_transform = transform.mul(scene_character_local_transform(
            character, time_norm, time_sec,
        )?);
        self.draw_character_nodes_vector(
            canvas,
            &character.children,
            character_transform,
            opacity,
            time_norm,
            time_sec,
        )
    }

    fn draw_character_nodes_vector(
        &mut self,
        canvas: &mut RgbaImage,
        nodes: &[SceneNode],
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        for node in nodes {
            match node {
                SceneNode::Defs(_) => {}
                SceneNode::Timeline(timeline) => {
                    let mut tracks = timeline
                        .children
                        .iter()
                        .filter_map(|node| match node {
                            SceneNode::Track(track) => Some(track),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    tracks.sort_by_key(|track| track.z);
                    let active_camera = active_scene_camera_from_tracks(
                        &tracks,
                        canvas.width(),
                        canvas.height(),
                        time_norm,
                        time_sec,
                    )?;
                    for track in tracks {
                        if is_scene_camera_track(track) {
                            continue;
                        }
                        let track_transform = if let Some(active_camera) = active_camera {
                            if is_scene_world_track(track) {
                                transform.mul(active_camera.transform)
                            } else {
                                transform
                            }
                        } else {
                            transform
                        };
                        self.draw_character_nodes_vector(
                            canvas,
                            &track.children,
                            track_transform,
                            inherited_opacity,
                            time_norm,
                            time_sec,
                        )?;
                    }
                }
                SceneNode::Track(track) => {
                    self.draw_character_nodes_vector(
                        canvas,
                        &track.children,
                        transform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                    )?;
                }
                SceneNode::Sequence(sequence) => {
                    if let Some((local_norm, local_sec)) =
                        scene_sequence_local_time(sequence, None, time_sec)
                    {
                        self.draw_character_nodes_vector(
                            canvas,
                            &sequence.children,
                            transform,
                            inherited_opacity,
                            local_norm,
                            local_sec,
                        )?;
                    }
                }
                SceneNode::Chain(chain) => {
                    let mut cursor_ms = chain.from_ms as i64;
                    for child in &chain.children {
                        if let SceneNode::Sequence(sequence) = child {
                            if let Some((local_norm, local_sec)) =
                                scene_sequence_local_time(sequence, Some(cursor_ms), time_sec)
                            {
                                self.draw_character_nodes_vector(
                                    canvas,
                                    &sequence.children,
                                    transform,
                                    inherited_opacity,
                                    local_norm,
                                    local_sec,
                                )?;
                            }
                            cursor_ms += sequence.duration_ms as i64 + chain.gap_ms;
                        }
                    }
                }
                SceneNode::Palette(_) => {}
                SceneNode::PixelGrid(grid) => {
                    self.draw_pixel_grid(
                        canvas,
                        grid,
                        transform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                    )?;
                }
                SceneNode::Group(group) => {
                    let opacity = (eval_scene_number(&group.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let group_transform =
                        transform.mul(scene_group_local_transform(group, time_norm, time_sec)?);
                    self.draw_character_nodes_vector(
                        canvas,
                        &group.children,
                        group_transform,
                        opacity,
                        time_norm,
                        time_sec,
                    )?;
                }
                SceneNode::Part(part) => {
                    let opacity = (eval_scene_number(&part.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let x = eval_scene_number(&part.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&part.y, time_norm, time_sec)?;
                    let rotation = eval_scene_number(&part.rotation, time_norm, time_sec)?;
                    let scale =
                        eval_scene_number(&part.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
                    let anchor_x = eval_scene_number(&part.anchor_x, time_norm, time_sec)?;
                    let anchor_y = eval_scene_number(&part.anchor_y, time_norm, time_sec)?;
                    let part_transform = transform
                        .mul(Affine2::translate(x, y))
                        .mul(Affine2::rotate_deg(rotation))
                        .mul(Affine2::scale(scale))
                        .mul(Affine2::translate(-anchor_x, -anchor_y));
                    self.draw_character_nodes_vector(
                        canvas,
                        &part.children,
                        part_transform,
                        opacity,
                        time_norm,
                        time_sec,
                    )?;
                }
                SceneNode::Repeat(repeat) => {
                    let count = eval_repeat_count(&repeat.count, time_norm, time_sec)?;
                    if count == 0 {
                        continue;
                    }
                    let x = eval_scene_number(&repeat.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&repeat.y, time_norm, time_sec)?;
                    let rotation = eval_scene_number(&repeat.rotation, time_norm, time_sec)?;
                    let scale =
                        eval_scene_number(&repeat.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
                    let opacity = eval_scene_number(&repeat.opacity, time_norm, time_sec)?;
                    let x_step = eval_scene_number(&repeat.x_step, time_norm, time_sec)?;
                    let y_step = eval_scene_number(&repeat.y_step, time_norm, time_sec)?;
                    let rotation_step =
                        eval_scene_number(&repeat.rotation_step, time_norm, time_sec)?;
                    let scale_step = eval_scene_number(&repeat.scale_step, time_norm, time_sec)?;
                    let opacity_step =
                        eval_scene_number(&repeat.opacity_step, time_norm, time_sec)?;
                    for index in 0..count {
                        let i = index as f32;
                        let copy_opacity =
                            ((opacity + opacity_step * i) * inherited_opacity).clamp(0.0, 1.0);
                        if copy_opacity <= 0.0001 {
                            continue;
                        }
                        let copy_scale = (scale + scale_step * i).clamp(0.001, 64.0);
                        let repeat_transform = transform
                            .mul(Affine2::translate(x + x_step * i, y + y_step * i))
                            .mul(Affine2::rotate_deg(rotation + rotation_step * i))
                            .mul(Affine2::scale(copy_scale));
                        self.draw_character_nodes_vector(
                            canvas,
                            &repeat.children,
                            repeat_transform,
                            copy_opacity,
                            time_norm,
                            time_sec,
                        )?;
                    }
                }
                SceneNode::Mask(mask) => {
                    let opacity = (eval_scene_number(&mask.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let mut layer =
                        RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
                    self.draw_character_nodes_vector(
                        &mut layer,
                        &mask.children,
                        transform,
                        opacity,
                        time_norm,
                        time_sec,
                    )?;
                    let mask_alpha = self.render_mask_alpha(
                        canvas.width(),
                        canvas.height(),
                        mask,
                        transform,
                        time_norm,
                        time_sec,
                    )?;
                    apply_alpha_mask(&mut layer, &mask_alpha);
                    composite_layer(canvas, &layer);
                }
                SceneNode::Character(character) => {
                    self.draw_character(
                        canvas,
                        character,
                        transform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                    )?;
                }
                SceneNode::Text(text) => {
                    self.draw_text_transformed(
                        canvas,
                        text,
                        transform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                    )?;
                }
                SceneNode::Line(line) => {
                    let node_transform =
                        transform.mul(scene_line_local_transform(line, time_norm, time_sec)?);
                    let opacity = (eval_scene_number(&line.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let style = eval_line_stroke_style(line, time_norm, time_sec)?;
                    let width = eval_scene_number(&line.width, time_norm, time_sec)?.max(0.0)
                        * affine_uniform_scale(node_transform);
                    if width <= 0.0001 {
                        continue;
                    }
                    let x1 = eval_scene_number(&line.x1, time_norm, time_sec)?;
                    let y1 = eval_scene_number(&line.y1, time_norm, time_sec)?;
                    let x2 = eval_scene_number(&line.x2, time_norm, time_sec)?;
                    let y2 = eval_scene_number(&line.y2, time_norm, time_sec)?;
                    let mut color = parse_color(&line.color)?;
                    color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
                    let (x1, y1) = node_transform.transform_point(x1, y1);
                    let (x2, y2) = node_transform.transform_point(x2, y2);
                    draw_line_segment_styled(
                        canvas,
                        Point2::new(x1, y1),
                        Point2::new(x2, y2),
                        width,
                        color,
                        style,
                        0.0,
                        1.0,
                    );
                }
                SceneNode::Polyline(polyline) => {
                    let node_transform = transform.mul(scene_polyline_local_transform(
                        polyline, time_norm, time_sec,
                    )?);
                    let opacity = (eval_scene_number(&polyline.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let style = eval_polyline_stroke_style(polyline, time_norm, time_sec)?;
                    let width = eval_scene_number(&polyline.stroke_width, time_norm, time_sec)?
                        .max(0.0)
                        * affine_uniform_scale(node_transform);
                    if width <= 0.0001 {
                        continue;
                    }
                    let points = self.cached_polyline_points(&polyline.points)?;
                    let trim = evaluate_trim(
                        &polyline.trim_start,
                        &polyline.trim_end,
                        time_norm,
                        time_sec,
                    )?;
                    if let Some(mut color) = parse_paint(&polyline.stroke)? {
                        color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
                        draw_transformed_trimmed_polylines_styled(
                            canvas,
                            &[points],
                            width,
                            color,
                            trim,
                            node_transform,
                            style,
                        );
                    }
                }
                SceneNode::Path(path) => {
                    let node_transform =
                        transform.mul(scene_path_local_transform(path, time_norm, time_sec)?);
                    let opacity = (eval_scene_number(&path.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let path_d = eval_path_d(&path.d, time_norm, time_sec)?;
                    let subpaths = self.cached_path_subpaths(path_d.as_ref())?;
                    if let Some(fill) = path.fill.as_deref() {
                        let paint = self.resolve_paint(fill)?;
                        let blend = parse_scene_blend(&path.blend)?;
                        draw_transformed_filled_polylines_paint(
                            canvas,
                            &subpaths,
                            &paint,
                            opacity,
                            blend,
                            node_transform,
                        );
                    }
                    let width = eval_scene_number(&path.stroke_width, time_norm, time_sec)?
                        .max(0.0)
                        * affine_uniform_scale(node_transform);
                    if width <= 0.0001 {
                        continue;
                    }
                    let trim =
                        evaluate_trim(&path.trim_start, &path.trim_end, time_norm, time_sec)?;
                    let style = eval_path_stroke_style(path, time_norm, time_sec)?;
                    if let Some(mut color) = parse_paint(&path.stroke)? {
                        color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
                        draw_transformed_trimmed_polylines_styled(
                            canvas,
                            &subpaths,
                            width,
                            color,
                            trim,
                            node_transform,
                            style,
                        );
                    }
                }
                SceneNode::FaceJaw(face_jaw) => {
                    self.draw_face_jaw(
                        canvas,
                        face_jaw,
                        transform,
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                }
                SceneNode::Circle(circle) => {
                    let node_transform =
                        transform.mul(scene_circle_local_transform(circle, time_norm, time_sec)?);
                    let opacity = (eval_scene_number(&circle.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let x = eval_scene_number(&circle.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&circle.y, time_norm, time_sec)?;
                    let radius = eval_scene_number(&circle.radius, time_norm, time_sec)?.max(0.0);
                    if radius <= 0.0001 {
                        continue;
                    }
                    let paint = self.resolve_paint(&circle.color)?;
                    let blend = parse_scene_blend(&circle.blend)?;
                    let stroke = circle.stroke.as_deref().map(parse_color).transpose()?;
                    let stroke_width =
                        eval_scene_number(&circle.stroke_width, time_norm, time_sec)?.max(0.0);
                    self.draw_circle_affine(
                        canvas,
                        node_transform,
                        x,
                        y,
                        radius,
                        &paint,
                        opacity,
                        blend,
                        stroke,
                        stroke_width,
                    );
                }
                SceneNode::Rect(rect) => {
                    let node_transform =
                        transform.mul(scene_rect_local_transform(rect, time_norm, time_sec)?);
                    let opacity = (eval_scene_number(&rect.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let x = eval_scene_number(&rect.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&rect.y, time_norm, time_sec)?;
                    let width = eval_scene_number(&rect.width, time_norm, time_sec)?.max(0.0);
                    let height = eval_scene_number(&rect.height, time_norm, time_sec)?.max(0.0);
                    if width <= 0.0001 || height <= 0.0001 {
                        continue;
                    }
                    let radius = eval_scene_number(&rect.radius, time_norm, time_sec)?.max(0.0);
                    let paint = self.resolve_paint(&rect.color)?;
                    let blend = parse_scene_blend(&rect.blend)?;
                    let stroke = rect.stroke.as_deref().map(parse_color).transpose()?;
                    let stroke_width =
                        eval_scene_number(&rect.stroke_width, time_norm, time_sec)?.max(0.0);
                    self.draw_rect_affine(
                        canvas,
                        node_transform,
                        x,
                        y,
                        width,
                        height,
                        radius,
                        &paint,
                        opacity,
                        blend,
                        stroke,
                        stroke_width,
                    );
                }
                SceneNode::Use(use_node) => {
                    self.draw_use_transformed(
                        canvas,
                        use_node,
                        transform,
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                }
                SceneNode::Precompose(_) | SceneNode::Layer(_) | SceneNode::Shadow(_) => {}
                SceneNode::Image(image) => {
                    self.draw_image_transformed(
                        canvas,
                        image,
                        transform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                    )?;
                }
                SceneNode::Svg(svg) => {
                    self.draw_svg_transformed(
                        canvas,
                        svg,
                        transform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                    )?;
                }
                SceneNode::Camera(_) => {}
            }
        }
        Ok(())
    }

    fn resolve_paint(&self, value: &str) -> Result<ResolvedPaint, MotionLoomSceneRenderError> {
        if is_none_paint(value) {
            return Ok(ResolvedPaint::None);
        }
        if let Some(id) = gradient_ref_id(value) {
            let Some(gradient) = self.gradient_defs.get(id) else {
                return Err(MotionLoomSceneRenderError::InvalidPaint {
                    value: value.to_string(),
                    message: format!("gradient reference not found: {id}"),
                });
            };
            return resolve_gradient_paint(value, gradient).map(ResolvedPaint::Gradient);
        }
        parse_color(value).map(ResolvedPaint::Solid)
    }

    fn draw_pixel_grid(
        &mut self,
        canvas: &mut RgbaImage,
        grid: &PixelGridNode,
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&grid.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let pixel_size = eval_scene_number(&grid.pixel_size, time_norm, time_sec)?.max(0.0);
        if pixel_size <= 0.0001 {
            return Ok(());
        }
        let x = eval_scene_number(&grid.x, time_norm, time_sec)?;
        let y = eval_scene_number(&grid.y, time_norm, time_sec)?;
        let scale = affine_uniform_scale(transform).max(0.001);
        let draw_size = pixel_size * scale;
        let palette = self.palette_defs.get(&grid.palette).ok_or_else(|| {
            MotionLoomSceneRenderError::InvalidPaint {
                value: grid.palette.clone(),
                message: format!("PixelGrid palette not found: {}", grid.palette),
            }
        })?;
        let blend = parse_scene_blend(&grid.blend)?;

        for (row, line) in grid.data.lines().enumerate() {
            for (col, ch) in line.chars().enumerate() {
                if ch.is_whitespace() {
                    continue;
                }
                let key = ch.to_string();
                let Some(color_def) = palette.colors.iter().find(|color| color.key == key) else {
                    return Err(MotionLoomSceneRenderError::InvalidPaint {
                        value: key,
                        message: format!(
                            "PixelGrid{} references color key not found in palette '{}'",
                            id_suffix(grid.id.as_deref()),
                            grid.palette
                        ),
                    });
                };
                let color = parse_color(&color_def.value)?;
                if color[3] == 0 {
                    continue;
                }
                let local_x = x + col as f32 * pixel_size;
                let local_y = y + row as f32 * pixel_size;
                let (draw_x, draw_y) = transform.transform_point(local_x, local_y);
                draw_rounded_rect_paint(
                    canvas,
                    draw_x,
                    draw_y,
                    draw_size,
                    draw_size,
                    0.0,
                    &ResolvedPaint::Solid(color),
                    opacity,
                    blend,
                );
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_rect_affine(
        &self,
        canvas: &mut RgbaImage,
        transform: Affine2,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        paint: &ResolvedPaint,
        opacity: f32,
        blend: SceneBlendMode,
        stroke: Option<[u8; 4]>,
        stroke_width: f32,
    ) {
        // Parent/group transforms can rotate or skew the full shape, not just its origin.
        let pad = (stroke_width * 0.5).ceil() + 2.0;
        let min_x = x - pad;
        let min_y = y - pad;
        let layer_w = (width + pad * 2.0).ceil().max(1.0) as u32;
        let layer_h = (height + pad * 2.0).ceil().max(1.0) as u32;
        let mut layer = RgbaImage::from_pixel(layer_w, layer_h, Rgba([0, 0, 0, 0]));
        let local_x = x - min_x;
        let local_y = y - min_y;
        draw_rounded_rect_paint(
            &mut layer,
            local_x,
            local_y,
            width,
            height,
            radius,
            paint,
            opacity,
            SceneBlendMode::Normal,
        );
        if let Some(mut stroke) = stroke {
            stroke[3] = ((stroke[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
            draw_rounded_rect_stroke(
                &mut layer,
                local_x,
                local_y,
                width,
                height,
                radius,
                stroke_width,
                stroke,
            );
        }
        composite_layer_affine_blend(
            canvas,
            &layer,
            transform.mul(Affine2::translate(min_x, min_y)),
            1.0,
            blend,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_circle_affine(
        &self,
        canvas: &mut RgbaImage,
        transform: Affine2,
        x: f32,
        y: f32,
        radius: f32,
        paint: &ResolvedPaint,
        opacity: f32,
        blend: SceneBlendMode,
        stroke: Option<[u8; 4]>,
        stroke_width: f32,
    ) {
        // Parent/group transforms can rotate or skew the full shape, not just its center.
        let pad = (stroke_width * 0.5).ceil() + 2.0;
        let min_x = x - radius - pad;
        let min_y = y - radius - pad;
        let diameter = radius * 2.0;
        let layer_w = (diameter + pad * 2.0).ceil().max(1.0) as u32;
        let layer_h = (diameter + pad * 2.0).ceil().max(1.0) as u32;
        let mut layer = RgbaImage::from_pixel(layer_w, layer_h, Rgba([0, 0, 0, 0]));
        let local_x = x - min_x;
        let local_y = y - min_y;
        draw_circle_paint(
            &mut layer,
            local_x,
            local_y,
            radius,
            paint,
            opacity,
            SceneBlendMode::Normal,
        );
        if let Some(mut stroke) = stroke {
            stroke[3] = ((stroke[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
            draw_circle_stroke(&mut layer, local_x, local_y, radius, stroke_width, stroke);
        }
        composite_layer_affine_blend(
            canvas,
            &layer,
            transform.mul(Affine2::translate(min_x, min_y)),
            1.0,
            blend,
        );
    }

    fn draw_rect(
        &mut self,
        canvas: &mut RgbaImage,
        rect: &RectNode,
        shadow: Option<EvaluatedShadow>,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&rect.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let x = eval_scene_number(&rect.x, time_norm, time_sec)?;
        let y = eval_scene_number(&rect.y, time_norm, time_sec)?;
        let width = eval_scene_number(&rect.width, time_norm, time_sec)?.max(0.0);
        let height = eval_scene_number(&rect.height, time_norm, time_sec)?.max(0.0);
        let radius = eval_scene_number(&rect.radius, time_norm, time_sec)?.max(0.0);
        let paint = self.resolve_paint(&rect.color)?;
        let blend = parse_scene_blend(&rect.blend)?;
        let stroke = rect.stroke.as_deref().map(parse_color).transpose()?;
        let stroke_width = eval_scene_number(&rect.stroke_width, time_norm, time_sec)?.max(0.0);
        let transform = scene_rect_local_transform(rect, time_norm, time_sec)?;

        if !affine_is_identity(transform) {
            let mut layer =
                RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
            if let Some(shadow) = shadow {
                draw_rect_shadow(&mut layer, x, y, width, height, radius, &shadow);
            }
            draw_rounded_rect_paint(
                &mut layer, x, y, width, height, radius, &paint, opacity, blend,
            );
            if let Some(stroke) = stroke {
                draw_rounded_rect_stroke(
                    &mut layer,
                    x,
                    y,
                    width,
                    height,
                    radius,
                    stroke_width,
                    stroke,
                );
            }
            composite_layer_affine(canvas, &layer, transform);
            return Ok(());
        }

        if let Some(shadow) = shadow {
            draw_rect_shadow(canvas, x, y, width, height, radius, &shadow);
        }
        draw_rounded_rect_paint(canvas, x, y, width, height, radius, &paint, opacity, blend);
        if let Some(stroke) = stroke {
            draw_rounded_rect_stroke(canvas, x, y, width, height, radius, stroke_width, stroke);
        }
        Ok(())
    }

    fn draw_circle(
        &mut self,
        canvas: &mut RgbaImage,
        circle: &CircleNode,
        shadow: Option<EvaluatedShadow>,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&circle.opacity, time_norm, time_sec)?
            * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let x = eval_scene_number(&circle.x, time_norm, time_sec)?;
        let y = eval_scene_number(&circle.y, time_norm, time_sec)?;
        let radius = eval_scene_number(&circle.radius, time_norm, time_sec)?.max(0.0);
        let paint = self.resolve_paint(&circle.color)?;
        let blend = parse_scene_blend(&circle.blend)?;
        let stroke = circle.stroke.as_deref().map(parse_color).transpose()?;
        let stroke_width = eval_scene_number(&circle.stroke_width, time_norm, time_sec)?.max(0.0);
        let transform = scene_circle_local_transform(circle, time_norm, time_sec)?;

        if !affine_is_identity(transform) {
            let mut layer =
                RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
            if let Some(shadow) = shadow {
                draw_circle_shadow(&mut layer, x, y, radius, &shadow);
            }
            draw_circle_paint(&mut layer, x, y, radius, &paint, opacity, blend);
            if let Some(stroke) = stroke {
                draw_circle_stroke(&mut layer, x, y, radius, stroke_width, stroke);
            }
            composite_layer_affine(canvas, &layer, transform);
            return Ok(());
        }

        if let Some(shadow) = shadow {
            draw_circle_shadow(canvas, x, y, radius, &shadow);
        }
        draw_circle_paint(canvas, x, y, radius, &paint, opacity, blend);
        if let Some(stroke) = stroke {
            draw_circle_stroke(canvas, x, y, radius, stroke_width, stroke);
        }
        Ok(())
    }

    fn draw_line(
        &mut self,
        canvas: &mut RgbaImage,
        line: &LineNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&line.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let x1 = eval_scene_number(&line.x1, time_norm, time_sec)?;
        let y1 = eval_scene_number(&line.y1, time_norm, time_sec)?;
        let x2 = eval_scene_number(&line.x2, time_norm, time_sec)?;
        let y2 = eval_scene_number(&line.y2, time_norm, time_sec)?;
        let transform = scene_line_local_transform(line, time_norm, time_sec)?;
        let width = eval_scene_number(&line.width, time_norm, time_sec)?.max(0.0)
            * affine_uniform_scale(transform);
        if width <= 0.0001 {
            return Ok(());
        }
        let style = eval_line_stroke_style(line, time_norm, time_sec)?;
        if let Some(mut color) = parse_paint(&line.color)? {
            color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
            let (x1, y1) = transform.transform_point(x1, y1);
            let (x2, y2) = transform.transform_point(x2, y2);
            draw_line_segment_styled(
                canvas,
                Point2::new(x1, y1),
                Point2::new(x2, y2),
                width,
                color,
                style,
                0.0,
                1.0,
            );
        }
        Ok(())
    }

    fn draw_polyline(
        &mut self,
        canvas: &mut RgbaImage,
        polyline: &PolylineNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&polyline.opacity, time_norm, time_sec)?
            * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let transform = scene_polyline_local_transform(polyline, time_norm, time_sec)?;
        let width = eval_scene_number(&polyline.stroke_width, time_norm, time_sec)?.max(0.0)
            * affine_uniform_scale(transform);
        if width <= 0.0001 {
            return Ok(());
        }
        let points = parse_polyline_points(&polyline.points)?;
        let trim = evaluate_trim(
            &polyline.trim_start,
            &polyline.trim_end,
            time_norm,
            time_sec,
        )?;
        let style = eval_polyline_stroke_style(polyline, time_norm, time_sec)?;
        if let Some(mut color) = parse_paint(&polyline.stroke)? {
            color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
            draw_transformed_trimmed_polylines_styled(
                canvas,
                &[points],
                width,
                color,
                trim,
                transform,
                style,
            );
        }
        Ok(())
    }

    fn draw_path(
        &mut self,
        canvas: &mut RgbaImage,
        path: &PathNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&path.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let transform = scene_path_local_transform(path, time_norm, time_sec)?;
        let path_d = eval_path_d(&path.d, time_norm, time_sec)?;
        let subpaths = parse_path_subpaths(path_d.as_ref())?;
        if let Some(fill) = path.fill.as_deref() {
            let paint = self.resolve_paint(fill)?;
            let blend = parse_scene_blend(&path.blend)?;
            draw_transformed_filled_polylines_paint(
                canvas, &subpaths, &paint, opacity, blend, transform,
            );
        }
        let width = eval_scene_number(&path.stroke_width, time_norm, time_sec)?.max(0.0)
            * affine_uniform_scale(transform);
        if width <= 0.0001 {
            return Ok(());
        }
        let trim = evaluate_trim(&path.trim_start, &path.trim_end, time_norm, time_sec)?;
        let style = eval_path_stroke_style(path, time_norm, time_sec)?;
        if let Some(mut color) = parse_paint(&path.stroke)? {
            color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
            draw_transformed_trimmed_polylines_styled(
                canvas, &subpaths, width, color, trim, transform, style,
            );
        }
        Ok(())
    }

    fn draw_face_jaw(
        &mut self,
        canvas: &mut RgbaImage,
        face_jaw: &FaceJawNode,
        transform: Affine2,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let path = face_jaw_to_path_node(face_jaw, time_norm, time_sec)?;
        let opacity = (eval_scene_number(&path.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }

        let subpaths = parse_path_subpaths(&path.d)?;
        if let Some(fill) = path.fill.as_deref() {
            let paint = self.resolve_paint(fill)?;
            let blend = parse_scene_blend(&path.blend)?;
            draw_transformed_filled_polylines_paint(
                canvas, &subpaths, &paint, opacity, blend, transform,
            );
        }

        let width = eval_scene_number(&path.stroke_width, time_norm, time_sec)?.max(0.0)
            * affine_uniform_scale(transform);
        if width <= 0.0001 {
            return Ok(());
        }
        let trim = evaluate_trim(&path.trim_start, &path.trim_end, time_norm, time_sec)?;
        let style = eval_path_stroke_style(&path, time_norm, time_sec)?;
        if let Some(mut color) = parse_paint(&path.stroke)? {
            color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
            draw_transformed_trimmed_polylines_styled(
                canvas, &subpaths, width, color, trim, transform, style,
            );
        }
        Ok(())
    }

    fn draw_text_transformed(
        &mut self,
        canvas: &mut RgbaImage,
        text: &TextNode,
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        if let Some(layer) = self.rasterize_text_texture_layer(
            text,
            transform,
            inherited_opacity,
            time_norm,
            time_sec,
            (canvas.width(), canvas.height()),
        )? {
            if let GpuSceneTextureSource::Cpu(image) = &layer.source {
                composite_layer_affine(canvas, image, layer.transform);
            }
        }
        Ok(())
    }

    fn draw_image_transformed(
        &mut self,
        canvas: &mut RgbaImage,
        image: &ImageNode,
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&image.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }

        let scale = eval_scene_number(&image.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
        let source = self.load_image_asset(&image.src)?;
        let target_w = ((source.width() as f32) * scale).round().max(1.0) as u32;
        let target_h = ((source.height() as f32) * scale).round().max(1.0) as u32;
        let x_base = resolve_axis(
            &image.x,
            canvas.width() as f32,
            target_w as f32,
            time_norm,
            time_sec,
        )?;
        let y_base = resolve_axis(
            &image.y,
            canvas.height() as f32,
            target_h as f32,
            time_norm,
            time_sec,
        )?;
        let local_transform = transform.mul(Affine2::translate(x_base, y_base));

        if target_w == source.width() && target_h == source.height() {
            composite_layer_affine_blend(
                canvas,
                source,
                local_transform,
                opacity,
                SceneBlendMode::Normal,
            );
        } else {
            let scaled = image::imageops::resize(source, target_w, target_h, FilterType::Lanczos3);
            composite_layer_affine_blend(
                canvas,
                &scaled,
                local_transform,
                opacity,
                SceneBlendMode::Normal,
            );
        }
        Ok(())
    }

    fn draw_svg_transformed(
        &mut self,
        canvas: &mut RgbaImage,
        svg: &SvgNode,
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&svg.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }

        let scale = eval_scene_number(&svg.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
        let source = self.load_svg_asset(&svg.src)?;
        let target_w = ((source.width() as f32) * scale).round().max(1.0) as u32;
        let target_h = ((source.height() as f32) * scale).round().max(1.0) as u32;
        let x_base = resolve_axis(
            &svg.x,
            canvas.width() as f32,
            target_w as f32,
            time_norm,
            time_sec,
        )?;
        let y_base = resolve_axis(
            &svg.y,
            canvas.height() as f32,
            target_h as f32,
            time_norm,
            time_sec,
        )?;
        let local_transform = transform.mul(Affine2::translate(x_base, y_base));

        if target_w == source.width() && target_h == source.height() {
            composite_layer_affine_blend(
                canvas,
                source,
                local_transform,
                opacity,
                SceneBlendMode::Normal,
            );
        } else {
            let scaled = image::imageops::resize(source, target_w, target_h, FilterType::Lanczos3);
            composite_layer_affine_blend(
                canvas,
                &scaled,
                local_transform,
                opacity,
                SceneBlendMode::Normal,
            );
        }
        Ok(())
    }

    fn gpu_image_texture_layer(
        &mut self,
        image: &ImageNode,
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
        canvas_size: (u32, u32),
    ) -> Result<Option<GpuSceneTextureLayer>, MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&image.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(None);
        }
        let scale = eval_scene_number(&image.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
        let (width, height, texture) = self
            .gpu_compositor
            .as_mut()
            .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                message: "GPU compositor was not initialized".to_string(),
            })?
            .load_image_texture(&image.src)?;

        raster_texture_layer(
            texture,
            width,
            height,
            &image.x,
            &image.y,
            scale,
            opacity,
            transform,
            time_norm,
            time_sec,
            canvas_size,
        )
    }

    fn gpu_svg_texture_layer(
        &mut self,
        svg: &SvgNode,
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
        canvas_size: (u32, u32),
    ) -> Result<Option<GpuSceneTextureLayer>, MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&svg.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(None);
        }
        let scale = eval_scene_number(&svg.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
        let (width, height, texture) = self
            .gpu_compositor
            .as_mut()
            .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                message: "GPU compositor was not initialized".to_string(),
            })?
            .load_svg_texture(&svg.src)?;

        raster_texture_layer(
            texture,
            width,
            height,
            &svg.x,
            &svg.y,
            scale,
            opacity,
            transform,
            time_norm,
            time_sec,
            canvas_size,
        )
    }

    fn rasterize_text_base_layer(
        &mut self,
        text: &TextNode,
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
        canvas_size: (u32, u32),
    ) -> Result<Option<TextRasterizedLayer>, MotionLoomSceneRenderError> {
        if text.value.trim().is_empty() {
            return Ok(None);
        }
        let font_def = text
            .font
            .as_deref()
            .and_then(|font_id| self.font_defs.get(font_id));
        let font_path = text
            .font_path
            .clone()
            .or_else(|| font_def.and_then(|font| font.path.clone()));
        if let Some(path) = font_path.as_deref() {
            let resolved = resolve_local_scene_asset_path(path);
            if resolved.exists() {
                let _ = self.font_system.db_mut().load_font_file(&resolved);
            }
        }

        let render_scale = eval_text_render_scale(&text.render_scale, time_norm, time_sec)?;
        let font_size = eval_scene_number(&text.font_size, time_norm, time_sec)?.clamp(1.0, 1024.0);
        let raster_font_size = font_size * render_scale;
        let opacity = (eval_scene_number(&text.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(None);
        }

        let line_height_raw = text
            .line_height
            .as_deref()
            .and_then(|expr| eval_scene_number(expr, time_norm, time_sec).ok())
            .unwrap_or(1.2);
        let line_height = if line_height_raw <= 10.0 {
            (font_size * line_height_raw).max(1.0)
        } else {
            line_height_raw.max(1.0)
        };
        let raster_line_height = line_height * render_scale;
        let width = text
            .layout_width_expr()
            .map(|expr| eval_scene_number(expr, time_norm, time_sec).map(|value| value.max(1.0)))
            .transpose()?;
        let raster_width = width.map(|value| value * render_scale);
        let visible_value = if let Some(expr) = text.visible_chars.as_deref() {
            let count = eval_scene_number(expr, time_norm, time_sec)
                .unwrap_or(text.value.chars().count() as f32)
                .floor()
                .clamp(0.0, text.value.chars().count() as f32) as usize;
            text.value.chars().take(count).collect::<String>()
        } else {
            text.value.clone()
        };
        let prepared_text = if text.animators.is_empty() {
            None
        } else {
            Some(
                prepare_text_layout_for_value(text, &visible_value).map_err(|message| {
                    MotionLoomSceneRenderError::InvalidExpression {
                        expr: "TextAnimator".to_string(),
                        message,
                    }
                })?,
            )
        };
        let metrics = Metrics::new(raster_font_size, raster_line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let mut attrs = Attrs::new()
            .family(Family::SansSerif)
            .weight(eval_text_font_weight(
                text.font_weight.as_deref(),
                time_norm,
                time_sec,
            )?)
            .letter_spacing(eval_text_tracking_em(
                text.tracking.as_deref(),
                font_size,
                time_norm,
                time_sec,
            )?);
        let font_family = text
            .font_family
            .as_deref()
            .or_else(|| font_def.and_then(|font| font.family.as_deref()));
        if let Some(family) = font_family
            && !family.trim().is_empty()
        {
            attrs = attrs.family(Family::Name(family));
        }
        // Browser builds use Basic shaping to avoid native font shaping paths.
        #[cfg(target_arch = "wasm32")]
        let shaping = Shaping::Basic;
        #[cfg(not(target_arch = "wasm32"))]
        let shaping = Shaping::Advanced;
        buffer.set_text(&mut self.font_system, &visible_value, &attrs, shaping);
        buffer.set_size(&mut self.font_system, raster_width, None);
        buffer.shape_until_scroll(&mut self.font_system, true);

        let (raster_text_w, raster_text_h) = text_bounds(&buffer, raster_line_height);
        let text_w = raster_text_w / render_scale;
        let text_h = raster_text_h / render_scale;
        let layout_w = width.unwrap_or(text_w);
        let box_style = eval_text_box_style(text, layout_w, text_h, time_norm, time_sec)?;
        let box_pad_x = box_style.map(|style| style.padding_x).unwrap_or(0.0);
        let box_pad_y = box_style.map(|style| style.padding_y).unwrap_or(0.0);
        let box_w = (layout_w + box_pad_x * 2.0).max(1.0);
        let box_h = (text_h + box_pad_y * 2.0).max(1.0);
        let x_base = resolve_axis(&text.x, canvas_size.0 as f32, layout_w, time_norm, time_sec)?;
        let y_base = resolve_axis(&text.y, canvas_size.1 as f32, text_h, time_norm, time_sec)?;
        let layer_effects =
            text_layer_effect_spec(text, time_norm, time_sec)?.scaled_for_raster(render_scale);
        let pad = (3.0_f32 * render_scale).ceil() as i32 + layer_effects.pad_px;
        let text_offset_x = pad + (box_pad_x * render_scale).round() as i32;
        let text_offset_y = pad + (box_pad_y * render_scale).round() as i32;
        let layer_w =
            ((box_w * render_scale).ceil().max(1.0) as u32).saturating_add((pad * 2) as u32);
        let layer_h =
            ((box_h * render_scale).ceil().max(1.0) as u32).saturating_add((pad * 2) as u32);
        let mut layer = RgbaImage::from_pixel(layer_w, layer_h, Rgba([0, 0, 0, 0]));
        if let Some(box_style) = box_style {
            let mut box_color = box_style.color;
            box_color[3] = ((box_color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
            draw_rounded_rect(
                &mut layer,
                pad as f32,
                pad as f32,
                box_w * render_scale,
                box_h * render_scale,
                box_style.radius * render_scale,
                box_color,
            );
        }
        let color = parse_color(&text.color)?;
        let combined_opacity = (color[3] as f32 / 255.0) * opacity;
        let text_color = Color::rgba(color[0], color[1], color[2], 255);
        let max_lines = text
            .max_lines_expr()
            .and_then(|expr| eval_scene_number(expr, time_norm, time_sec).ok())
            .map(|value| value.floor().max(0.0) as usize);

        if let Some(prepared_text) = prepared_text.as_ref() {
            draw_text_buffer_with_animators(
                &buffer,
                &mut self.font_system,
                &mut self.swash_cache,
                &mut layer,
                TextAnimatorRasterParams {
                    text,
                    prepared: prepared_text,
                    value: &visible_value,
                    base_color: color,
                    base_opacity: opacity,
                    offset_x: text_offset_x,
                    offset_y: text_offset_y,
                    raster_scale: render_scale,
                    max_lines,
                    global_time_ms: (time_sec.max(0.0) * 1000.0).round() as i64,
                    time_norm,
                    time_sec,
                },
            )?;
        } else {
            buffer.draw(
                &mut self.font_system,
                &mut self.swash_cache,
                text_color,
                |x, y, _w, _h, color| {
                    if let Some(max_lines) = max_lines {
                        let line_ix = ((y as f32) / raster_line_height).floor().max(0.0) as usize;
                        if line_ix >= max_lines {
                            return;
                        }
                    }
                    let px = x + text_offset_x;
                    let py = y + text_offset_y;
                    if px < 0 || py < 0 {
                        return;
                    }
                    let (px, py) = (px as u32, py as u32);
                    if px >= layer.width() || py >= layer.height() {
                        return;
                    }
                    let (sr, sg, sb, sa) = color.as_rgba_tuple();
                    let sa = ((sa as f32) * combined_opacity).round().clamp(0.0, 255.0) as u8;
                    blend_pixel(&mut layer, px, py, [sr, sg, sb, sa]);
                },
            );
        }
        let text_transform = transform
            .mul(Affine2::translate(
                x_base - box_pad_x - pad as f32 / render_scale,
                y_base - box_pad_y - pad as f32 / render_scale,
            ))
            .mul(scene_text_local_transform(text, time_norm, time_sec)?)
            .mul(Affine2::scale(1.0 / render_scale));
        Ok(Some(TextRasterizedLayer {
            image: layer,
            transform: text_transform,
            effects: layer_effects,
        }))
    }

    fn rasterize_text_texture_layer(
        &mut self,
        text: &TextNode,
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
        canvas_size: (u32, u32),
    ) -> Result<Option<GpuSceneTextureLayer>, MotionLoomSceneRenderError> {
        let Some(base) = self.rasterize_text_base_layer(
            text,
            transform,
            inherited_opacity,
            time_norm,
            time_sec,
            canvas_size,
        )?
        else {
            return Ok(None);
        };
        let image = if base.effects.has_effects() {
            apply_text_layer_effects(&base.image, &base.effects)
        } else {
            base.image
        };
        Ok(Some(GpuSceneTextureLayer {
            source: GpuSceneTextureSource::Cpu(image),
            transform: base.transform,
            opacity: 1.0,
            blend: SceneBlendMode::Normal,
            matte: None,
        }))
    }

    fn rasterize_text_texture_layers_gpu_effects(
        &mut self,
        text: &TextNode,
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
        canvas_size: (u32, u32),
    ) -> Result<Vec<GpuSceneTextureLayer>, MotionLoomSceneRenderError> {
        let Some(base) = self.rasterize_text_base_layer(
            text,
            transform,
            inherited_opacity,
            time_norm,
            time_sec,
            canvas_size,
        )?
        else {
            return Ok(Vec::new());
        };

        if !base.effects.has_effects() {
            return Ok(vec![GpuSceneTextureLayer {
                source: GpuSceneTextureSource::Cpu(base.image),
                transform: base.transform,
                opacity: 1.0,
                blend: SceneBlendMode::Normal,
                matte: None,
            }]);
        }

        let mut layers = Vec::new();
        let source = self
            .gpu_compositor
            .as_mut()
            .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                message: "GPU compositor was not initialized".to_string(),
            })?
            .upload_gpu_rgba_texture(&base.image)?;

        if !base.effects.shadows.is_empty() || !base.effects.glows.is_empty() {
            let compositor = self.gpu_compositor.as_mut().ok_or_else(|| {
                MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                }
            })?;
            for shadow in &base.effects.shadows {
                let mut shadow_texture =
                    compositor.apply_gpu_tint_texture(&source, shadow.color, 1.0);
                if shadow.blur > 0.001 {
                    shadow_texture = compositor.apply_gpu_blur_texture(
                        &shadow_texture,
                        &[(true, shadow.blur), (false, shadow.blur)],
                    )?;
                }
                layers.push(GpuSceneTextureLayer {
                    source: GpuSceneTextureSource::Gpu(shadow_texture),
                    transform: base.transform.mul(Affine2::translate(shadow.x, shadow.y)),
                    opacity: 1.0,
                    blend: SceneBlendMode::Normal,
                    matte: None,
                });
            }
            for glow in &base.effects.glows {
                let glow_texture =
                    compositor.apply_gpu_tint_texture(&source, glow.color, glow.intensity);
                let glow_texture = compositor.apply_gpu_blur_texture(
                    &glow_texture,
                    &[(true, glow.radius), (false, glow.radius)],
                )?;
                layers.push(GpuSceneTextureLayer {
                    source: GpuSceneTextureSource::Gpu(glow_texture),
                    transform: base.transform,
                    opacity: 1.0,
                    blend: SceneBlendMode::Screen,
                    matte: None,
                });
            }
        }

        if let Some(stroke) = base.effects.stroke {
            let stroke_layer = stroke_layer_from_alpha(&base.image, stroke.width, stroke.color);
            layers.push(GpuSceneTextureLayer {
                source: GpuSceneTextureSource::Cpu(stroke_layer),
                transform: base.transform,
                opacity: 1.0,
                blend: SceneBlendMode::Normal,
                matte: None,
            });
        }

        let fill_source = if base.effects.blur_radius > 0.001 {
            let compositor = self.gpu_compositor.as_mut().ok_or_else(|| {
                MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                }
            })?;
            GpuSceneTextureSource::Gpu(compositor.apply_gpu_blur_texture(
                &source,
                &[
                    (true, base.effects.blur_radius),
                    (false, base.effects.blur_radius),
                ],
            )?)
        } else {
            GpuSceneTextureSource::Gpu(source)
        };
        layers.push(GpuSceneTextureLayer {
            source: fill_source,
            transform: base.transform,
            opacity: 1.0,
            blend: SceneBlendMode::Normal,
            matte: None,
        });

        Ok(layers)
    }

    fn load_image_asset(&mut self, src: &str) -> Result<&RgbaImage, MotionLoomSceneRenderError> {
        if !self.image_cache.contains_key(src) {
            let decoded = load_rgba_image_source(src, self.asset_resolver.as_ref())?;
            self.image_cache.insert(src.to_string(), decoded);
        }

        Ok(self
            .image_cache
            .get(src)
            .expect("image cache entry inserted before lookup"))
    }

    fn load_svg_asset(&mut self, src: &str) -> Result<&RgbaImage, MotionLoomSceneRenderError> {
        if !self.svg_cache.contains_key(src) {
            let decoded = load_svg_source(src, self.asset_resolver.as_ref())?;
            self.svg_cache.insert(src.to_string(), decoded);
        }

        Ok(self
            .svg_cache
            .get(src)
            .expect("SVG cache entry inserted before lookup"))
    }
}

pub(crate) fn eval_scene_number(
    expr: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<f32, MotionLoomSceneRenderError> {
    eval_time_expr(expr, time_norm, time_sec).map_err(|message| {
        MotionLoomSceneRenderError::InvalidExpression {
            expr: expr.to_string(),
            message,
        }
    })
}

fn eval_text_render_scale(
    expr: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<f32, MotionLoomSceneRenderError> {
    let trimmed = expr.trim();
    let scale = if let Some(without_suffix) = trimmed
        .strip_suffix('x')
        .or_else(|| trimmed.strip_suffix('X'))
    {
        without_suffix.trim().parse::<f32>().map_err(|err| {
            MotionLoomSceneRenderError::InvalidExpression {
                expr: expr.to_string(),
                message: format!("invalid Text renderScale value: {err}"),
            }
        })?
    } else {
        eval_scene_number(trimmed, time_norm, time_sec)?
    };
    Ok(scale.clamp(1.0, 8.0))
}

fn eval_text_font_weight(
    expr: Option<&str>,
    time_norm: f32,
    time_sec: f32,
) -> Result<Weight, MotionLoomSceneRenderError> {
    let Some(expr) = expr.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Weight::NORMAL);
    };
    let normalized = expr.to_ascii_lowercase().replace(['-', '_', ' '], "");
    let weight = match normalized.as_str() {
        "thin" | "hairline" => Weight::THIN,
        "extralight" | "ultralight" => Weight::EXTRA_LIGHT,
        "light" => Weight::LIGHT,
        "normal" | "regular" | "book" => Weight::NORMAL,
        "medium" => Weight::MEDIUM,
        "semibold" | "demibold" => Weight::SEMIBOLD,
        "bold" | "bolder" => Weight::BOLD,
        "extrabold" | "ultrabold" => Weight::EXTRA_BOLD,
        "black" | "heavy" => Weight::BLACK,
        "lighter" => Weight::LIGHT,
        _ => {
            let value = eval_scene_number(expr, time_norm, time_sec).map_err(|err| {
                MotionLoomSceneRenderError::InvalidExpression {
                    expr: expr.to_string(),
                    message: format!("invalid Text fontWeight value: {err}"),
                }
            })?;
            Weight(value.round().clamp(1.0, 1000.0) as u16)
        }
    };
    Ok(weight)
}

fn eval_text_tracking_em(
    expr: Option<&str>,
    font_size: f32,
    time_norm: f32,
    time_sec: f32,
) -> Result<f32, MotionLoomSceneRenderError> {
    let Some(expr) = expr.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(0.0);
    };
    let tracking_px = eval_scene_number(expr, time_norm, time_sec)?;
    Ok((tracking_px / font_size.max(1.0)).clamp(-1.0, 4.0))
}

#[derive(Clone, Copy, Debug)]
struct TextBoxStyle {
    color: [u8; 4],
    padding_x: f32,
    padding_y: f32,
    radius: f32,
}

fn eval_text_box_style(
    text: &TextNode,
    text_width: f32,
    text_height: f32,
    time_norm: f32,
    time_sec: f32,
) -> Result<Option<TextBoxStyle>, MotionLoomSceneRenderError> {
    let Some(kind) = text.box_style.as_deref().map(str::trim) else {
        return Ok(None);
    };
    if kind.is_empty() {
        return Ok(None);
    }
    let normalized = kind.to_ascii_lowercase().replace(['-', '_', ' '], "");
    if matches!(normalized.as_str(), "none" | "false" | "off" | "0") {
        return Ok(None);
    }
    if !matches!(normalized.as_str(), "pill" | "rect" | "rectangle") {
        return Err(MotionLoomSceneRenderError::InvalidExpression {
            expr: kind.to_string(),
            message: "invalid Text box value. Expected pill, rect, or none.".to_string(),
        });
    }

    let (default_pad_x, default_pad_y) = text
        .box_padding
        .as_deref()
        .map(|expr| eval_text_box_padding(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or((0.0, 0.0));
    let padding_x = text
        .box_padding_x
        .as_deref()
        .map(|expr| eval_scene_number(expr, time_norm, time_sec).map(|value| value.max(0.0)))
        .transpose()?
        .unwrap_or(default_pad_x);
    let padding_y = text
        .box_padding_y
        .as_deref()
        .map(|expr| eval_scene_number(expr, time_norm, time_sec).map(|value| value.max(0.0)))
        .transpose()?
        .unwrap_or(default_pad_y);
    let box_height = (text_height + padding_y * 2.0).max(1.0);
    let radius = text
        .box_radius
        .as_deref()
        .map(|expr| eval_scene_number(expr, time_norm, time_sec).map(|value| value.max(0.0)))
        .transpose()?
        .unwrap_or_else(|| {
            if normalized == "pill" {
                box_height * 0.5
            } else {
                0.0
            }
        });
    let color = parse_color(text.box_color.as_deref().unwrap_or("#000000"))?;

    Ok(Some(TextBoxStyle {
        color,
        padding_x,
        padding_y,
        radius: radius
            .min((text_width + padding_x * 2.0) * 0.5)
            .min(box_height * 0.5),
    }))
}

fn eval_text_box_padding(
    expr: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<(f32, f32), MotionLoomSceneRenderError> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Ok((0.0, 0.0));
    }
    let (x_expr, y_expr) = split_text_box_padding_pair(trimmed).unwrap_or((trimmed, trimmed));
    let x = eval_scene_number(x_expr, time_norm, time_sec)?.max(0.0);
    let y = eval_scene_number(y_expr, time_norm, time_sec)?.max(0.0);
    Ok((x, y))
}

fn split_text_box_padding_pair(expr: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut in_quote = false;
    let mut quote_char = '\0';
    for (ix, ch) in expr.char_indices() {
        if in_quote {
            if ch == quote_char {
                in_quote = false;
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                in_quote = true;
                quote_char = ch;
            }
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let left = expr[..ix].trim();
                let right = expr[ix + ch.len_utf8()..].trim();
                if !left.is_empty() && !right.is_empty() {
                    return Some((left, right));
                }
            }
            _ if depth == 0 && ch.is_whitespace() => {
                let left = expr[..ix].trim();
                let right = expr[ix + ch.len_utf8()..].trim();
                if !left.is_empty() && !right.is_empty() {
                    return Some((left, right));
                }
            }
            _ => {}
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn collect_gpu_scene_commands(
    nodes: &[SceneNode],
    transform: Affine2,
    deform: Option<&EvaluatedDeformGrid>,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    canvas_size: (u32, u32),
    gradient_defs: &HashMap<String, GradientDef>,
    palette_defs: &HashMap<String, PaletteNode>,
    scene_components: &HashMap<String, Vec<SceneNode>>,
    primitives: &mut Vec<GpuScenePrimitive>,
    text_requests: &mut Vec<GpuSceneTextRequest>,
    scene_overlays: &mut Vec<CpuSceneOverlay>,
) -> Result<(), MotionLoomSceneRenderError> {
    collect_gpu_scene_commands_with_depth(
        nodes,
        transform,
        deform,
        inherited_opacity,
        time_norm,
        time_sec,
        canvas_size,
        gradient_defs,
        palette_defs,
        scene_components,
        primitives,
        text_requests,
        scene_overlays,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn collect_gpu_scene_commands_with_depth(
    nodes: &[SceneNode],
    transform: Affine2,
    deform: Option<&EvaluatedDeformGrid>,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    canvas_size: (u32, u32),
    gradient_defs: &HashMap<String, GradientDef>,
    palette_defs: &HashMap<String, PaletteNode>,
    scene_components: &HashMap<String, Vec<SceneNode>>,
    primitives: &mut Vec<GpuScenePrimitive>,
    text_requests: &mut Vec<GpuSceneTextRequest>,
    scene_overlays: &mut Vec<CpuSceneOverlay>,
    depth: Option<SceneDepthContext<'_>>,
) -> Result<(), MotionLoomSceneRenderError> {
    let mut pending_shadow: Option<EvaluatedShadow> = None;
    for node in nodes {
        match node {
            SceneNode::Defs(_) => {
                pending_shadow = None;
            }
            SceneNode::Timeline(timeline) => {
                let mut tracks = timeline
                    .children
                    .iter()
                    .filter_map(|node| match node {
                        SceneNode::Track(track) => Some(track),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                tracks.sort_by_key(|track| track.z);
                let active_camera = active_scene_camera_from_tracks(
                    &tracks,
                    canvas_size.0,
                    canvas_size.1,
                    time_norm,
                    time_sec,
                )?;
                tracks.sort_by(|a, b| {
                    let a_world = is_scene_world_track(a);
                    let b_world = is_scene_world_track(b);
                    match (a_world, b_world) {
                        (true, true) => {
                            let a_depth =
                                scene_depth_track_sort_key(&a.z_depth, time_norm, time_sec)
                                    .unwrap_or(0.0);
                            let b_depth =
                                scene_depth_track_sort_key(&b.z_depth, time_norm, time_sec)
                                    .unwrap_or(0.0);
                            b_depth.total_cmp(&a_depth).then_with(|| a.z.cmp(&b.z))
                        }
                        _ => a.z.cmp(&b.z),
                    }
                });
                for track in tracks {
                    if is_scene_camera_track(track) {
                        continue;
                    }
                    let track_depth = if is_scene_world_track(track) {
                        Some(SceneDepthContext {
                            active_camera,
                            canvas_size,
                            track_z_depth: &track.z_depth,
                        })
                    } else {
                        depth
                    };
                    collect_gpu_scene_commands_with_depth(
                        &track.children,
                        transform,
                        deform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        gradient_defs,
                        palette_defs,
                        scene_components,
                        primitives,
                        text_requests,
                        scene_overlays,
                        track_depth,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Track(track) => {
                collect_gpu_scene_commands_with_depth(
                    &track.children,
                    transform,
                    deform,
                    inherited_opacity,
                    time_norm,
                    time_sec,
                    canvas_size,
                    gradient_defs,
                    palette_defs,
                    scene_components,
                    primitives,
                    text_requests,
                    scene_overlays,
                    depth,
                )?;
                pending_shadow = None;
            }
            SceneNode::Sequence(sequence) => {
                if let Some((local_norm, local_sec)) =
                    scene_sequence_local_time(sequence, None, time_sec)
                {
                    if depth.is_some() {
                        collect_gpu_scene_commands_depth_sorted(
                            &sequence.children,
                            transform,
                            deform,
                            inherited_opacity,
                            local_norm,
                            local_sec,
                            canvas_size,
                            gradient_defs,
                            palette_defs,
                            scene_components,
                            primitives,
                            text_requests,
                            scene_overlays,
                            depth,
                        )?;
                    } else {
                        collect_gpu_scene_commands_with_depth(
                            &sequence.children,
                            transform,
                            deform,
                            inherited_opacity,
                            local_norm,
                            local_sec,
                            canvas_size,
                            gradient_defs,
                            palette_defs,
                            scene_components,
                            primitives,
                            text_requests,
                            scene_overlays,
                            depth,
                        )?;
                    }
                }
                pending_shadow = None;
            }
            SceneNode::Chain(chain) => {
                let mut cursor_ms = chain.from_ms as i64;
                for child in &chain.children {
                    if let SceneNode::Sequence(sequence) = child {
                        if let Some((local_norm, local_sec)) =
                            scene_sequence_local_time(sequence, Some(cursor_ms), time_sec)
                        {
                            if depth.is_some() {
                                collect_gpu_scene_commands_depth_sorted(
                                    &sequence.children,
                                    transform,
                                    deform,
                                    inherited_opacity,
                                    local_norm,
                                    local_sec,
                                    canvas_size,
                                    gradient_defs,
                                    palette_defs,
                                    scene_components,
                                    primitives,
                                    text_requests,
                                    scene_overlays,
                                    depth,
                                )?;
                            } else {
                                collect_gpu_scene_commands_with_depth(
                                    &sequence.children,
                                    transform,
                                    deform,
                                    inherited_opacity,
                                    local_norm,
                                    local_sec,
                                    canvas_size,
                                    gradient_defs,
                                    palette_defs,
                                    scene_components,
                                    primitives,
                                    text_requests,
                                    scene_overlays,
                                    depth,
                                )?;
                            }
                        }
                        cursor_ms += sequence.duration_ms as i64 + chain.gap_ms;
                    }
                }
                pending_shadow = None;
            }
            SceneNode::Palette(_) => {
                pending_shadow = None;
            }
            SceneNode::PixelGrid(grid) => {
                if deform.is_some() {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![SceneNode::PixelGrid(grid.clone())],
                    });
                } else {
                    push_gpu_pixel_grid_commands(
                        grid,
                        transform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        palette_defs,
                        primitives,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Text(text) => {
                if deform.is_some() {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![SceneNode::Text(text.clone())],
                    });
                } else {
                    text_requests.push(GpuSceneTextRequest {
                        node: text.as_ref().clone(),
                        transform,
                        opacity: inherited_opacity,
                    });
                }
                pending_shadow = None;
            }
            SceneNode::Rect(rect) => {
                if rect_requires_cpu_overlay(rect) {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![SceneNode::Rect(rect.clone())],
                    });
                    pending_shadow = None;
                } else {
                    push_gpu_rect_commands(
                        rect,
                        transform,
                        deform,
                        pending_shadow.take(),
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        gradient_defs,
                        primitives,
                    )?;
                }
            }
            SceneNode::Circle(circle) => {
                if circle_requires_cpu_overlay(circle) {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![SceneNode::Circle(circle.clone())],
                    });
                    pending_shadow = None;
                } else {
                    push_gpu_circle_commands(
                        circle,
                        transform,
                        deform,
                        pending_shadow.take(),
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        gradient_defs,
                        primitives,
                    )?;
                }
            }
            SceneNode::Line(line) => {
                if line_requires_cpu_overlay(line) {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![node.clone()],
                    });
                } else {
                    push_gpu_line_command(
                        line,
                        transform,
                        deform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        gradient_defs,
                        primitives,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Polyline(polyline) => {
                if polyline_requires_cpu_overlay(polyline) {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![node.clone()],
                    });
                } else {
                    push_gpu_polyline_commands(
                        polyline,
                        transform,
                        deform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        gradient_defs,
                        primitives,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Path(path) => {
                if path_requires_cpu_overlay(path) {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![node.clone()],
                    });
                } else {
                    push_gpu_path_commands(
                        path,
                        transform,
                        deform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        gradient_defs,
                        primitives,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::FaceJaw(face_jaw) => {
                let path = face_jaw_to_path_node(face_jaw, time_norm, time_sec)?;
                if path_requires_cpu_overlay(&path) {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![SceneNode::FaceJaw(face_jaw.clone())],
                    });
                } else {
                    push_gpu_path_commands(
                        &path,
                        transform,
                        deform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        gradient_defs,
                        primitives,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Shadow(shadow) => {
                pending_shadow = Some(evaluate_shadow(
                    shadow,
                    time_norm,
                    time_sec,
                    inherited_opacity,
                )?);
            }
            SceneNode::Group(group) => {
                let opacity = (eval_scene_number(&group.opacity, time_norm, time_sec)?
                    * inherited_opacity)
                    .clamp(0.0, 1.0);
                if opacity > 0.0001 {
                    let group_transform =
                        transform.mul(scene_group_local_transform(group, time_norm, time_sec)?);
                    let group_deform = eval_group_deform_grid(group, time_norm, time_sec)?
                        .map(|grid| transform_deform_grid(&grid, group_transform));
                    let child_deform = group_deform.as_ref().or(deform);
                    collect_gpu_scene_commands(
                        &group.children,
                        group_transform,
                        child_deform,
                        opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        gradient_defs,
                        palette_defs,
                        scene_components,
                        primitives,
                        text_requests,
                        scene_overlays,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Part(part) => {
                let opacity = (eval_scene_number(&part.opacity, time_norm, time_sec)?
                    * inherited_opacity)
                    .clamp(0.0, 1.0);
                if opacity > 0.0001 {
                    let x = eval_scene_number(&part.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&part.y, time_norm, time_sec)?;
                    let rotation = eval_scene_number(&part.rotation, time_norm, time_sec)?;
                    let scale =
                        eval_scene_number(&part.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
                    let anchor_x = eval_scene_number(&part.anchor_x, time_norm, time_sec)?;
                    let anchor_y = eval_scene_number(&part.anchor_y, time_norm, time_sec)?;
                    let part_transform = transform
                        .mul(Affine2::translate(x, y))
                        .mul(Affine2::rotate_deg(rotation))
                        .mul(Affine2::scale(scale))
                        .mul(Affine2::translate(-anchor_x, -anchor_y));
                    collect_gpu_scene_commands(
                        &part.children,
                        part_transform,
                        deform,
                        opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        gradient_defs,
                        palette_defs,
                        scene_components,
                        primitives,
                        text_requests,
                        scene_overlays,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Repeat(repeat) => {
                let count = eval_repeat_count(&repeat.count, time_norm, time_sec)?;
                let x = eval_scene_number(&repeat.x, time_norm, time_sec)?;
                let y = eval_scene_number(&repeat.y, time_norm, time_sec)?;
                let rotation = eval_scene_number(&repeat.rotation, time_norm, time_sec)?;
                let scale =
                    eval_scene_number(&repeat.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
                let opacity = eval_scene_number(&repeat.opacity, time_norm, time_sec)?;
                let x_step = eval_scene_number(&repeat.x_step, time_norm, time_sec)?;
                let y_step = eval_scene_number(&repeat.y_step, time_norm, time_sec)?;
                let rotation_step = eval_scene_number(&repeat.rotation_step, time_norm, time_sec)?;
                let scale_step = eval_scene_number(&repeat.scale_step, time_norm, time_sec)?;
                let opacity_step = eval_scene_number(&repeat.opacity_step, time_norm, time_sec)?;
                for index in 0..count {
                    let i = index as f32;
                    let copy_opacity =
                        ((opacity + opacity_step * i) * inherited_opacity).clamp(0.0, 1.0);
                    if copy_opacity <= 0.0001 {
                        continue;
                    }
                    let repeat_transform = transform
                        .mul(Affine2::translate(x + x_step * i, y + y_step * i))
                        .mul(Affine2::rotate_deg(rotation + rotation_step * i))
                        .mul(Affine2::scale((scale + scale_step * i).clamp(0.001, 64.0)));
                    collect_gpu_scene_commands(
                        &repeat.children,
                        repeat_transform,
                        deform,
                        copy_opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        gradient_defs,
                        palette_defs,
                        scene_components,
                        primitives,
                        text_requests,
                        scene_overlays,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Mask(mask) => {
                scene_overlays.push(CpuSceneOverlay::Vector {
                    nodes: vec![SceneNode::Mask(mask.clone())],
                });
                pending_shadow = None;
            }
            SceneNode::Precompose(precompose) => {
                scene_overlays.push(CpuSceneOverlay::Vector {
                    nodes: vec![SceneNode::Precompose(precompose.clone())],
                });
                pending_shadow = None;
            }
            SceneNode::Use(use_node) => {
                // Expand Component references so reusable scene fragments stay GPU-native.
                let opacity = (eval_scene_number(&use_node.opacity, time_norm, time_sec)?
                    * inherited_opacity)
                    .clamp(0.0, 1.0);
                let Some(children) = scene_components.get(&use_node.ref_id) else {
                    pending_shadow = None;
                    continue;
                };
                if opacity > 0.0001 {
                    let use_transform =
                        transform.mul(scene_use_local_transform(use_node, time_norm, time_sec)?);
                    let primitive_start = primitives.len();
                    let text_start = text_requests.len();
                    collect_gpu_scene_commands(
                        children,
                        use_transform,
                        deform,
                        opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        gradient_defs,
                        palette_defs,
                        scene_components,
                        primitives,
                        text_requests,
                        scene_overlays,
                    )?;

                    let use_blend = parse_scene_blend(&use_node.blend)?;
                    if use_blend != SceneBlendMode::Normal {
                        if text_requests.len() > text_start {
                            primitives.truncate(primitive_start);
                            text_requests.truncate(text_start);
                            scene_overlays.push(CpuSceneOverlay::Vector {
                                nodes: vec![SceneNode::Use(use_node.clone())],
                            });
                        } else {
                            for primitive in &mut primitives[primitive_start..] {
                                primitive.blend = use_blend;
                            }
                        }
                    }
                }
                pending_shadow = None;
            }
            SceneNode::Layer(layer) => {
                if layer.source.is_some()
                    || layer.mask.is_some()
                    || layer.matte.is_some()
                    || layer.effect.is_some()
                {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![SceneNode::Layer(layer.clone())],
                    });
                    pending_shadow = None;
                    continue;
                }
                let opacity = (eval_scene_number(&layer.opacity, time_norm, time_sec)?
                    * inherited_opacity)
                    .clamp(0.0, 1.0);
                if opacity > 0.0001 && parse_scene_blend(&layer.blend)? == SceneBlendMode::Normal {
                    let base_transform = if let Some(depth) = depth {
                        let z_depth =
                            scene_layer_effective_z_depth(layer, depth, time_norm, time_sec)?;
                        transform.mul(scene_z_depth_transform(
                            depth.active_camera,
                            depth.canvas_size,
                            z_depth,
                        ))
                    } else {
                        transform
                    };
                    collect_gpu_scene_commands_with_depth(
                        &layer.children,
                        base_transform
                            .mul(scene_layer_local_transform(layer, time_norm, time_sec)?),
                        deform,
                        opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        gradient_defs,
                        palette_defs,
                        scene_components,
                        primitives,
                        text_requests,
                        scene_overlays,
                        None,
                    )?;
                } else if opacity > 0.0001 {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![SceneNode::Layer(layer.clone())],
                    });
                }
                pending_shadow = None;
            }
            SceneNode::Camera(camera) => {
                let opacity = (eval_scene_number(&camera.opacity, time_norm, time_sec)?
                    * inherited_opacity)
                    .clamp(0.0, 1.0);
                if opacity > 0.0001 {
                    let camera_transform = camera_transform(
                        camera,
                        &camera.children,
                        canvas_size.0,
                        canvas_size.1,
                        time_norm,
                        time_sec,
                    )?;
                    collect_gpu_scene_commands(
                        &camera.children,
                        transform.mul(camera_transform),
                        deform,
                        opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        gradient_defs,
                        palette_defs,
                        scene_components,
                        primitives,
                        text_requests,
                        scene_overlays,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Character(character) => {
                let opacity = (eval_scene_number(&character.opacity, time_norm, time_sec)?
                    * inherited_opacity)
                    .clamp(0.0, 1.0);
                if opacity > 0.0001 {
                    let character_transform = transform.mul(scene_character_local_transform(
                        character, time_norm, time_sec,
                    )?);
                    collect_gpu_scene_commands(
                        &character.children,
                        character_transform,
                        deform,
                        opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        gradient_defs,
                        palette_defs,
                        scene_components,
                        primitives,
                        text_requests,
                        scene_overlays,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Image(_) | SceneNode::Svg(_) => {
                pending_shadow = None;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn collect_gpu_scene_commands_depth_sorted(
    nodes: &[SceneNode],
    transform: Affine2,
    deform: Option<&EvaluatedDeformGrid>,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    canvas_size: (u32, u32),
    gradient_defs: &HashMap<String, GradientDef>,
    palette_defs: &HashMap<String, PaletteNode>,
    scene_components: &HashMap<String, Vec<SceneNode>>,
    primitives: &mut Vec<GpuScenePrimitive>,
    text_requests: &mut Vec<GpuSceneTextRequest>,
    scene_overlays: &mut Vec<CpuSceneOverlay>,
    depth: Option<SceneDepthContext<'_>>,
) -> Result<(), MotionLoomSceneRenderError> {
    let Some(depth) = depth else {
        return collect_gpu_scene_commands_with_depth(
            nodes,
            transform,
            deform,
            inherited_opacity,
            time_norm,
            time_sec,
            canvas_size,
            gradient_defs,
            palette_defs,
            scene_components,
            primitives,
            text_requests,
            scene_overlays,
            None,
        );
    };
    let mut layer_items = Vec::<(usize, f32, &SceneNode)>::new();
    for (index, node) in nodes.iter().enumerate() {
        if let SceneNode::Layer(layer) = node {
            layer_items.push((
                index,
                scene_layer_effective_z_depth(layer, depth, time_norm, time_sec)?,
                node,
            ));
        } else {
            collect_gpu_scene_commands_with_depth(
                std::slice::from_ref(node),
                transform,
                deform,
                inherited_opacity,
                time_norm,
                time_sec,
                canvas_size,
                gradient_defs,
                palette_defs,
                scene_components,
                primitives,
                text_requests,
                scene_overlays,
                Some(depth),
            )?;
        }
    }
    layer_items.sort_by(|(a_order, a_depth, _), (b_order, b_depth, _)| {
        b_depth
            .total_cmp(a_depth)
            .then_with(|| a_order.cmp(b_order))
    });
    for (_, _, node) in layer_items {
        collect_gpu_scene_commands_with_depth(
            std::slice::from_ref(node),
            transform,
            deform,
            inherited_opacity,
            time_norm,
            time_sec,
            canvas_size,
            gradient_defs,
            palette_defs,
            scene_components,
            primitives,
            text_requests,
            scene_overlays,
            Some(depth),
        )?;
    }
    Ok(())
}

fn rect_requires_cpu_overlay(rect: &RectNode) -> bool {
    !is_gpu_native_blend(&rect.blend)
}

fn circle_requires_cpu_overlay(circle: &CircleNode) -> bool {
    !is_gpu_native_blend(&circle.blend)
}

fn line_requires_cpu_overlay(line: &LineNode) -> bool {
    !is_gpu_native_blend(&line.blend) || !is_default_line_cap(&line.line_cap)
}

fn polyline_requires_cpu_overlay(polyline: &PolylineNode) -> bool {
    !is_gpu_native_blend(&polyline.blend)
        || !is_default_line_cap(&polyline.line_cap)
        || !is_default_line_join(&polyline.line_join)
}

fn push_gpu_pixel_grid_commands(
    grid: &PixelGridNode,
    transform: Affine2,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    palette_defs: &HashMap<String, PaletteNode>,
    primitives: &mut Vec<GpuScenePrimitive>,
) -> Result<(), MotionLoomSceneRenderError> {
    let opacity = (eval_scene_number(&grid.opacity, time_norm, time_sec)? * inherited_opacity)
        .clamp(0.0, 1.0);
    if opacity <= 0.0001 {
        return Ok(());
    }
    let pixel_size = eval_scene_number(&grid.pixel_size, time_norm, time_sec)?.max(0.0);
    if pixel_size <= 0.0001 {
        return Ok(());
    }
    let x = eval_scene_number(&grid.x, time_norm, time_sec)?;
    let y = eval_scene_number(&grid.y, time_norm, time_sec)?;
    let blend = parse_scene_blend(&grid.blend)?;
    let palette = palette_defs.get(&grid.palette).ok_or_else(|| {
        MotionLoomSceneRenderError::InvalidPaint {
            value: grid.palette.clone(),
            message: format!("PixelGrid palette not found: {}", grid.palette),
        }
    })?;

    for (row, line) in grid.data.lines().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            if ch.is_whitespace() {
                continue;
            }
            let key = ch.to_string();
            let Some(color_def) = palette.colors.iter().find(|color| color.key == key) else {
                return Err(MotionLoomSceneRenderError::InvalidPaint {
                    value: key,
                    message: format!(
                        "PixelGrid{} references color key not found in palette '{}'",
                        id_suffix(grid.id.as_deref()),
                        grid.palette
                    ),
                });
            };
            let color = parse_color(&color_def.value)?;
            if color[3] == 0 {
                continue;
            }
            primitives.push(GpuScenePrimitive {
                kind: GPU_SHAPE_RECT_FILL,
                transform,
                shape: [
                    x + col as f32 * pixel_size,
                    y + row as f32 * pixel_size,
                    pixel_size,
                    pixel_size,
                ],
                radius: 0.0,
                stroke_width: 0.0,
                blur: 0.0,
                color,
                opacity,
                blend,
                gradient: None,
                line_t0: 0.0,
                line_t1: 1.0,
                taper_start: 0.0,
                taper_end: 0.0,
            });
        }
    }
    Ok(())
}

fn path_requires_cpu_overlay(path: &PathNode) -> bool {
    let has_visible_fill = path
        .fill
        .as_deref()
        .is_some_and(|fill| !is_none_paint(fill));
    !is_gpu_native_blend(&path.blend)
        || !is_default_line_cap(&path.line_cap)
        || !is_default_line_join(&path.line_join)
        || (is_none_paint(&path.stroke) && !has_visible_fill)
}

fn is_default_line_cap(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.is_empty() || value == "round"
}

fn is_default_line_join(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.is_empty() || value == "round"
}

fn resolve_gpu_scene_paint(
    value: &str,
    gradient_defs: &HashMap<String, GradientDef>,
    bounds: PaintBounds,
) -> Result<([u8; 4], Option<GpuSceneGradientPaint>), MotionLoomSceneRenderError> {
    if let Some(id) = gradient_ref_id(value) {
        let Some(gradient) = gradient_defs.get(id) else {
            return Err(MotionLoomSceneRenderError::InvalidPaint {
                value: value.to_string(),
                message: format!("gradient reference not found: {id}"),
            });
        };
        return Ok((
            [255, 255, 255, 255],
            Some(GpuSceneGradientPaint {
                gradient: resolve_gradient_paint(value, gradient)?,
                bounds,
            }),
        ));
    }
    Ok((parse_color(value)?, None))
}

fn points_bounds(points: &[Point2]) -> Option<PaintBounds> {
    let mut iter = points.iter();
    let first = *iter.next()?;
    let mut min_x = first.x;
    let mut min_y = first.y;
    let mut max_x = first.x;
    let mut max_y = first.y;
    for point in iter {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    Some(PaintBounds::new(min_x, min_y, max_x, max_y))
}

fn subpaths_bounds(subpaths: &[Vec<Point2>]) -> Option<PaintBounds> {
    let mut bounds: Option<PaintBounds> = None;
    for subpath in subpaths {
        let Some(next) = points_bounds(subpath) else {
            continue;
        };
        bounds = Some(match bounds {
            Some(current) => PaintBounds::new(
                current.min_x.min(next.min_x),
                current.min_y.min(next.min_y),
                current.max_x.max(next.max_x),
                current.max_y.max(next.max_y),
            ),
            None => next,
        });
    }
    bounds
}

fn rect_polygon(x: f32, y: f32, width: f32, height: f32) -> Vec<Point2> {
    vec![
        Point2::new(x, y),
        Point2::new(x + width, y),
        Point2::new(x + width, y + height),
        Point2::new(x, y + height),
        Point2::new(x, y),
    ]
}

fn circle_polygon(x: f32, y: f32, radius: f32) -> Vec<Point2> {
    if radius <= 0.0001 {
        return Vec::new();
    }
    ellipse_polygon(x, y, radius, radius)
}

fn ellipse_polygon(x: f32, y: f32, radius_x: f32, radius_y: f32) -> Vec<Point2> {
    if radius_x <= 0.0001 || radius_y <= 0.0001 {
        return Vec::new();
    }
    let steps = 48usize;
    let mut points = Vec::with_capacity(steps + 1);
    for ix in 0..=steps {
        let t = ix as f32 / steps as f32 * std::f32::consts::TAU;
        points.push(Point2::new(x + t.cos() * radius_x, y + t.sin() * radius_y));
    }
    points
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_rect_commands(
    rect: &RectNode,
    transform: Affine2,
    deform: Option<&EvaluatedDeformGrid>,
    shadow: Option<EvaluatedShadow>,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    gradient_defs: &HashMap<String, GradientDef>,
    primitives: &mut Vec<GpuScenePrimitive>,
) -> Result<(), MotionLoomSceneRenderError> {
    let opacity = (eval_scene_number(&rect.opacity, time_norm, time_sec)? * inherited_opacity)
        .clamp(0.0, 1.0);
    if opacity <= 0.0001 {
        return Ok(());
    }
    let x = eval_scene_number(&rect.x, time_norm, time_sec)?;
    let y = eval_scene_number(&rect.y, time_norm, time_sec)?;
    let width = eval_scene_number(&rect.width, time_norm, time_sec)?.max(0.0);
    let height = eval_scene_number(&rect.height, time_norm, time_sec)?.max(0.0);
    let radius = eval_scene_number(&rect.radius, time_norm, time_sec)?.max(0.0);
    let shape_transform = transform.mul(scene_rect_local_transform(rect, time_norm, time_sec)?);
    let paint_bounds = PaintBounds::new(x, y, x + width, y + height);
    let fill_blend = parse_scene_blend(&rect.blend)?;

    if let Some(deform) = deform {
        if let Some(shadow) = shadow {
            let shadow_subpaths = vec![rect_polygon(x + shadow.x, y + shadow.y, width, height)];
            let shadow_subpaths =
                transform_and_deform_subpaths(&shadow_subpaths, shape_transform, deform);
            push_gpu_filled_path_triangles(
                primitives,
                Affine2::identity(),
                &shadow_subpaths,
                shadow.color,
                1.0,
                None,
            );
        }

        let subpaths = vec![rect_polygon(x, y, width, height)];
        let subpaths = transform_and_deform_subpaths(&subpaths, shape_transform, deform);
        let warped_bounds =
            subpaths_bounds(&subpaths).unwrap_or_else(|| PaintBounds::new(0.0, 0.0, 1.0, 1.0));
        let (color, gradient) = resolve_gpu_scene_paint(&rect.color, gradient_defs, warped_bounds)?;
        push_gpu_filled_path_triangles_with_blend(
            primitives,
            Affine2::identity(),
            &subpaths,
            color,
            opacity,
            gradient,
            fill_blend,
        );

        if let Some(stroke_value) = rect
            .stroke
            .as_deref()
            .filter(|stroke| !is_none_paint(stroke))
        {
            let stroke_width = eval_scene_number(&rect.stroke_width, time_norm, time_sec)?.max(0.0)
                * affine_uniform_scale(shape_transform);
            if stroke_width > 0.0 {
                let (stroke, gradient) =
                    resolve_gpu_scene_paint(stroke_value, gradient_defs, warped_bounds)?;
                push_gpu_stroke_segments(
                    primitives,
                    Affine2::identity(),
                    &subpaths,
                    stroke_width,
                    stroke,
                    opacity,
                    gradient,
                    (0.0, 1.0),
                    StrokeStyle::default(),
                    fill_blend,
                );
            }
        }
        let _ = radius;
        return Ok(());
    }

    if let Some(shadow) = shadow {
        primitives.push(GpuScenePrimitive {
            kind: GPU_SHAPE_RECT_SHADOW,
            transform: shape_transform,
            shape: [x + shadow.x, y + shadow.y, width, height],
            radius,
            stroke_width: 0.0,
            blur: shadow.blur,
            color: shadow.color,
            opacity: 1.0,
            blend: SceneBlendMode::Normal,
            gradient: None,
            line_t0: 0.0,
            line_t1: 1.0,
            taper_start: 0.0,
            taper_end: 0.0,
        });
    }

    let (color, gradient) = resolve_gpu_scene_paint(&rect.color, gradient_defs, paint_bounds)?;
    primitives.push(GpuScenePrimitive {
        kind: GPU_SHAPE_RECT_FILL,
        transform: shape_transform,
        shape: [x, y, width, height],
        radius,
        stroke_width: 0.0,
        blur: 0.0,
        color,
        opacity,
        blend: fill_blend,
        gradient,
        line_t0: 0.0,
        line_t1: 1.0,
        taper_start: 0.0,
        taper_end: 0.0,
    });

    if let Some(stroke_value) = rect
        .stroke
        .as_deref()
        .filter(|stroke| !is_none_paint(stroke))
    {
        let stroke_width = eval_scene_number(&rect.stroke_width, time_norm, time_sec)?.max(0.0);
        if stroke_width > 0.0 {
            let (stroke, gradient) =
                resolve_gpu_scene_paint(stroke_value, gradient_defs, paint_bounds)?;
            primitives.push(GpuScenePrimitive {
                kind: GPU_SHAPE_RECT_STROKE,
                transform: shape_transform,
                shape: [x, y, width, height],
                radius,
                stroke_width,
                blur: 0.0,
                color: stroke,
                opacity,
                blend: SceneBlendMode::Normal,
                gradient,
                line_t0: 0.0,
                line_t1: 1.0,
                taper_start: 0.0,
                taper_end: 0.0,
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_circle_commands(
    circle: &CircleNode,
    transform: Affine2,
    deform: Option<&EvaluatedDeformGrid>,
    shadow: Option<EvaluatedShadow>,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    gradient_defs: &HashMap<String, GradientDef>,
    primitives: &mut Vec<GpuScenePrimitive>,
) -> Result<(), MotionLoomSceneRenderError> {
    let opacity = (eval_scene_number(&circle.opacity, time_norm, time_sec)? * inherited_opacity)
        .clamp(0.0, 1.0);
    if opacity <= 0.0001 {
        return Ok(());
    }
    let x = eval_scene_number(&circle.x, time_norm, time_sec)?;
    let y = eval_scene_number(&circle.y, time_norm, time_sec)?;
    let radius = eval_scene_number(&circle.radius, time_norm, time_sec)?.max(0.0);
    let shape_transform = transform.mul(scene_circle_local_transform(circle, time_norm, time_sec)?);
    let paint_bounds = PaintBounds::new(x - radius, y - radius, x + radius, y + radius);
    let blend = parse_scene_blend(&circle.blend)?;

    if let Some(deform) = deform {
        if let Some(shadow) = shadow {
            let shadow_subpaths = vec![circle_polygon(x + shadow.x, y + shadow.y, radius)];
            let shadow_subpaths =
                transform_and_deform_subpaths(&shadow_subpaths, shape_transform, deform);
            push_gpu_filled_path_triangles(
                primitives,
                Affine2::identity(),
                &shadow_subpaths,
                shadow.color,
                1.0,
                None,
            );
        }

        let subpaths = vec![circle_polygon(x, y, radius)];
        let subpaths = transform_and_deform_subpaths(&subpaths, shape_transform, deform);
        let warped_bounds =
            subpaths_bounds(&subpaths).unwrap_or_else(|| PaintBounds::new(0.0, 0.0, 1.0, 1.0));
        let (color, gradient) =
            resolve_gpu_scene_paint(&circle.color, gradient_defs, warped_bounds)?;
        push_gpu_filled_path_triangles_with_blend(
            primitives,
            Affine2::identity(),
            &subpaths,
            color,
            opacity,
            gradient,
            blend,
        );

        if let Some(stroke_value) = circle
            .stroke
            .as_deref()
            .filter(|stroke| !is_none_paint(stroke))
        {
            let stroke_width = eval_scene_number(&circle.stroke_width, time_norm, time_sec)?
                .max(0.0)
                * affine_uniform_scale(shape_transform);
            if stroke_width > 0.0 {
                let (stroke, gradient) =
                    resolve_gpu_scene_paint(stroke_value, gradient_defs, warped_bounds)?;
                push_gpu_stroke_segments(
                    primitives,
                    Affine2::identity(),
                    &subpaths,
                    stroke_width,
                    stroke,
                    opacity,
                    gradient,
                    (0.0, 1.0),
                    StrokeStyle::default(),
                    blend,
                );
            }
        }
        return Ok(());
    }

    if let Some(shadow) = shadow {
        primitives.push(GpuScenePrimitive {
            kind: GPU_SHAPE_CIRCLE_SHADOW,
            transform: shape_transform,
            shape: [x + shadow.x, y + shadow.y, radius, 0.0],
            radius: 0.0,
            stroke_width: 0.0,
            blur: shadow.blur,
            color: shadow.color,
            opacity: 1.0,
            blend: SceneBlendMode::Normal,
            gradient: None,
            line_t0: 0.0,
            line_t1: 1.0,
            taper_start: 0.0,
            taper_end: 0.0,
        });
    }

    let (color, gradient) = resolve_gpu_scene_paint(&circle.color, gradient_defs, paint_bounds)?;
    primitives.push(GpuScenePrimitive {
        kind: GPU_SHAPE_CIRCLE_FILL,
        transform: shape_transform,
        shape: [x, y, radius, 0.0],
        radius: 0.0,
        stroke_width: 0.0,
        blur: 0.0,
        color,
        opacity,
        blend,
        gradient,
        line_t0: 0.0,
        line_t1: 1.0,
        taper_start: 0.0,
        taper_end: 0.0,
    });

    if let Some(stroke_value) = circle
        .stroke
        .as_deref()
        .filter(|stroke| !is_none_paint(stroke))
    {
        let stroke_width = eval_scene_number(&circle.stroke_width, time_norm, time_sec)?.max(0.0);
        if stroke_width > 0.0 {
            let (stroke, gradient) =
                resolve_gpu_scene_paint(stroke_value, gradient_defs, paint_bounds)?;
            primitives.push(GpuScenePrimitive {
                kind: GPU_SHAPE_CIRCLE_STROKE,
                transform: shape_transform,
                shape: [x, y, radius, 0.0],
                radius: 0.0,
                stroke_width,
                blur: 0.0,
                color: stroke,
                opacity,
                blend,
                gradient,
                line_t0: 0.0,
                line_t1: 1.0,
                taper_start: 0.0,
                taper_end: 0.0,
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_line_command(
    line: &LineNode,
    transform: Affine2,
    deform: Option<&EvaluatedDeformGrid>,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    gradient_defs: &HashMap<String, GradientDef>,
    primitives: &mut Vec<GpuScenePrimitive>,
) -> Result<(), MotionLoomSceneRenderError> {
    let opacity = (eval_scene_number(&line.opacity, time_norm, time_sec)? * inherited_opacity)
        .clamp(0.0, 1.0);
    if opacity <= 0.0001 {
        return Ok(());
    }

    let x1 = eval_scene_number(&line.x1, time_norm, time_sec)?;
    let y1 = eval_scene_number(&line.y1, time_norm, time_sec)?;
    let x2 = eval_scene_number(&line.x2, time_norm, time_sec)?;
    let y2 = eval_scene_number(&line.y2, time_norm, time_sec)?;
    let transform = transform.mul(scene_line_local_transform(line, time_norm, time_sec)?);
    let width = eval_scene_number(&line.width, time_norm, time_sec)?.max(0.0);
    if width <= 0.0001 {
        return Ok(());
    }
    let blend = parse_scene_blend(&line.blend)?;

    let p0 = transform_and_deform_point(transform, Point2::new(x1, y1), deform);
    let p1 = transform_and_deform_point(transform, Point2::new(x2, y2), deform);
    let (paint_bounds, primitive_transform, p0, p1, width) = if deform.is_some() {
        (
            PaintBounds::new(
                p0.x.min(p1.x),
                p0.y.min(p1.y),
                p0.x.max(p1.x),
                p0.y.max(p1.y),
            ),
            Affine2::identity(),
            p0,
            p1,
            width * affine_uniform_scale(transform),
        )
    } else {
        (
            PaintBounds::new(x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)),
            transform,
            Point2::new(x1, y1),
            Point2::new(x2, y2),
            width,
        )
    };
    let (color, gradient) = resolve_gpu_scene_paint(&line.color, gradient_defs, paint_bounds)?;
    let style = eval_line_stroke_style(line, time_norm, time_sec)?;
    push_gpu_styled_line_primitives(
        primitives,
        primitive_transform,
        p0,
        p1,
        width,
        color,
        opacity,
        gradient,
        0.0,
        1.0,
        style,
        blend,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_polyline_commands(
    polyline: &PolylineNode,
    transform: Affine2,
    deform: Option<&EvaluatedDeformGrid>,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    gradient_defs: &HashMap<String, GradientDef>,
    primitives: &mut Vec<GpuScenePrimitive>,
) -> Result<(), MotionLoomSceneRenderError> {
    let opacity = (eval_scene_number(&polyline.opacity, time_norm, time_sec)? * inherited_opacity)
        .clamp(0.0, 1.0);
    if opacity <= 0.0001 {
        return Ok(());
    }
    let width = eval_scene_number(&polyline.stroke_width, time_norm, time_sec)?.max(0.0);
    if width <= 0.0001 {
        return Ok(());
    }
    let points = parse_polyline_points(&polyline.points)?;
    let transform = transform.mul(scene_polyline_local_transform(
        polyline, time_norm, time_sec,
    )?);
    let blend = parse_scene_blend(&polyline.blend)?;
    let (points, primitive_transform, width) = if let Some(deform) = deform {
        (
            points
                .iter()
                .map(|point| transform_and_deform_point(transform, *point, Some(deform)))
                .collect::<Vec<_>>(),
            Affine2::identity(),
            width * affine_uniform_scale(transform),
        )
    } else {
        (points, transform, width)
    };
    let trim = evaluate_trim(
        &polyline.trim_start,
        &polyline.trim_end,
        time_norm,
        time_sec,
    )?;
    let paint_bounds =
        points_bounds(&points).unwrap_or_else(|| PaintBounds::new(0.0, 0.0, 1.0, 1.0));
    let (color, gradient) = resolve_gpu_scene_paint(&polyline.stroke, gradient_defs, paint_bounds)?;
    let style = eval_polyline_stroke_style(polyline, time_norm, time_sec)?;
    push_gpu_stroke_segments(
        primitives,
        primitive_transform,
        &[points],
        width,
        color,
        opacity,
        gradient,
        trim,
        style,
        blend,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_path_commands(
    path: &PathNode,
    transform: Affine2,
    deform: Option<&EvaluatedDeformGrid>,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    gradient_defs: &HashMap<String, GradientDef>,
    primitives: &mut Vec<GpuScenePrimitive>,
) -> Result<(), MotionLoomSceneRenderError> {
    let opacity = (eval_scene_number(&path.opacity, time_norm, time_sec)? * inherited_opacity)
        .clamp(0.0, 1.0);
    if opacity <= 0.0001 {
        return Ok(());
    }
    let path_d = eval_path_d(&path.d, time_norm, time_sec)?;
    let transform = transform.mul(scene_path_local_transform(path, time_norm, time_sec)?);
    let blend = parse_scene_blend(&path.blend)?;
    let subpaths = parse_path_subpaths(path_d.as_ref())?;
    let (subpaths, primitive_transform, stroke_width_scale) = if let Some(deform) = deform {
        (
            transform_and_deform_subpaths(&subpaths, transform, deform),
            Affine2::identity(),
            affine_uniform_scale(transform),
        )
    } else {
        (subpaths, transform, 1.0)
    };
    let paint_bounds =
        subpaths_bounds(&subpaths).unwrap_or_else(|| PaintBounds::new(0.0, 0.0, 1.0, 1.0));
    if let Some(fill) = path.fill.as_deref().filter(|fill| !is_none_paint(fill)) {
        let (color, gradient) = resolve_gpu_scene_paint(fill, gradient_defs, paint_bounds)?;
        push_gpu_filled_path_triangles_with_blend(
            primitives,
            primitive_transform,
            &subpaths,
            color,
            opacity,
            gradient,
            blend,
        );
    }

    let width =
        eval_scene_number(&path.stroke_width, time_norm, time_sec)?.max(0.0) * stroke_width_scale;
    if width > 0.0001 && !is_none_paint(&path.stroke) {
        let trim = evaluate_trim(&path.trim_start, &path.trim_end, time_norm, time_sec)?;
        let (color, gradient) = resolve_gpu_scene_paint(&path.stroke, gradient_defs, paint_bounds)?;
        let style = eval_path_stroke_style(path, time_norm, time_sec)?;
        push_gpu_stroke_segments(
            primitives,
            primitive_transform,
            &subpaths,
            width,
            color,
            opacity,
            gradient,
            trim,
            style,
            blend,
        );
    }
    Ok(())
}

fn push_gpu_filled_path_triangles(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    subpaths: &[Vec<Point2>],
    color: [u8; 4],
    opacity: f32,
    gradient: Option<GpuSceneGradientPaint>,
) {
    push_gpu_filled_path_triangles_with_blend(
        primitives,
        transform,
        subpaths,
        color,
        opacity,
        gradient,
        SceneBlendMode::Normal,
    );
}

fn push_gpu_filled_path_triangles_with_blend(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    subpaths: &[Vec<Point2>],
    color: [u8; 4],
    opacity: f32,
    gradient: Option<GpuSceneGradientPaint>,
    blend: SceneBlendMode,
) {
    for subpath in subpaths {
        for [a, b, c] in triangulate_polygon(subpath) {
            primitives.push(GpuScenePrimitive {
                kind: GPU_SHAPE_TRIANGLE_FILL,
                transform,
                shape: [a.x, a.y, b.x, b.y],
                radius: c.x,
                stroke_width: c.y,
                blur: 0.0,
                color,
                opacity,
                blend,
                gradient: gradient.clone(),
                line_t0: 0.0,
                line_t1: 1.0,
                taper_start: 0.0,
                taper_end: 0.0,
            });
        }
    }
}

fn triangulate_polygon(points: &[Point2]) -> Vec<[Point2; 3]> {
    let polygon = sanitize_polygon(points);
    if polygon.len() < 3 {
        return Vec::new();
    }
    if polygon_area(&polygon).abs() <= 0.0001 {
        return Vec::new();
    }

    let ccw = polygon_area(&polygon) > 0.0;
    let mut indices: Vec<usize> = (0..polygon.len()).collect();
    let mut triangles = Vec::with_capacity(polygon.len().saturating_sub(2));
    let mut guard = 0usize;

    while indices.len() > 3 && guard < polygon.len().saturating_mul(polygon.len()).max(16) {
        guard += 1;
        let len = indices.len();
        let mut clipped = false;
        for i in 0..len {
            let prev = indices[(i + len - 1) % len];
            let curr = indices[i];
            let next = indices[(i + 1) % len];
            let a = polygon[prev];
            let b = polygon[curr];
            let c = polygon[next];
            if !is_convex_corner(a, b, c, ccw) {
                continue;
            }
            let contains_other = indices.iter().any(|&candidate| {
                candidate != prev
                    && candidate != curr
                    && candidate != next
                    && point_in_triangle(polygon[candidate], a, b, c)
            });
            if contains_other {
                continue;
            }
            triangles.push([a, b, c]);
            indices.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            return triangulate_polygon_fan(&polygon);
        }
    }

    if indices.len() == 3 {
        triangles.push([
            polygon[indices[0]],
            polygon[indices[1]],
            polygon[indices[2]],
        ]);
    }
    triangles
}

fn sanitize_polygon(points: &[Point2]) -> Vec<Point2> {
    let mut out = Vec::with_capacity(points.len());
    for &point in points {
        if out
            .last()
            .is_some_and(|last: &Point2| points_close(*last, point))
        {
            continue;
        }
        out.push(point);
    }
    if out.len() > 1
        && out
            .last()
            .zip(out.first())
            .is_some_and(|(last, first)| points_close(*last, *first))
    {
        out.pop();
    }
    out
}

fn triangulate_polygon_fan(points: &[Point2]) -> Vec<[Point2; 3]> {
    if points.len() < 3 {
        return Vec::new();
    }
    let mut triangles = Vec::with_capacity(points.len().saturating_sub(2));
    let a = points[0];
    for i in 1..points.len() - 1 {
        triangles.push([a, points[i], points[i + 1]]);
    }
    triangles
}

fn points_close(a: Point2, b: Point2) -> bool {
    (a.x - b.x).abs() <= 0.001 && (a.y - b.y).abs() <= 0.001
}

fn polygon_area(points: &[Point2]) -> f32 {
    let mut area = 0.0;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        area += a.x * b.y - b.x * a.y;
    }
    area * 0.5
}

fn is_convex_corner(a: Point2, b: Point2, c: Point2, ccw: bool) -> bool {
    let cross = cross_points(a, b, c);
    if ccw { cross > 0.001 } else { cross < -0.001 }
}

fn point_in_triangle(p: Point2, a: Point2, b: Point2, c: Point2) -> bool {
    let c0 = cross_points(a, b, p);
    let c1 = cross_points(b, c, p);
    let c2 = cross_points(c, a, p);
    let has_neg = c0 < -0.001 || c1 < -0.001 || c2 < -0.001;
    let has_pos = c0 > 0.001 || c1 > 0.001 || c2 > 0.001;
    !(has_neg && has_pos)
}

fn cross_points(a: Point2, b: Point2, c: Point2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_styled_line_primitives(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    opacity: f32,
    gradient: Option<GpuSceneGradientPaint>,
    line_t0: f32,
    line_t1: f32,
    style: StrokeStyle,
    blend: SceneBlendMode,
) {
    let copies = stroke_texture_copy_count(style);
    for copy_ix in 0..copies {
        let (start, end, width_scale, opacity_scale) =
            stroke_texture_variant(p0, p1, style, copy_ix);
        let stroke_width = (width * width_scale).max(0.01);
        let stroke_opacity = (opacity * opacity_scale).clamp(0.0, 1.0);
        if style.pressure_auto {
            push_gpu_pressure_line_primitives(
                primitives,
                transform,
                start,
                end,
                stroke_width,
                color,
                stroke_opacity,
                gradient.clone(),
                line_t0,
                line_t1,
                style,
                blend,
            );
        } else {
            push_gpu_line_primitive(
                primitives,
                transform,
                start,
                end,
                stroke_width,
                color,
                stroke_opacity,
                gradient.clone(),
                line_t0,
                line_t1,
                style.taper_start,
                style.taper_end,
                blend,
            );
        }
    }
    push_gpu_stroke_overlay_primitives(
        primitives, transform, p0, p1, width, color, opacity, line_t0, line_t1, style, blend,
    );
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_line_primitive(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    opacity: f32,
    gradient: Option<GpuSceneGradientPaint>,
    line_t0: f32,
    line_t1: f32,
    taper_start: f32,
    taper_end: f32,
    blend: SceneBlendMode,
) {
    primitives.push(GpuScenePrimitive {
        kind: GPU_SHAPE_LINE,
        transform,
        shape: [p0.x, p0.y, p1.x, p1.y],
        radius: 0.0,
        stroke_width: width.max(0.01),
        blur: 0.0,
        color,
        opacity: opacity.clamp(0.0, 1.0),
        blend,
        gradient,
        line_t0,
        line_t1,
        taper_start,
        taper_end,
    });
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_pressure_line_primitives(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    opacity: f32,
    gradient: Option<GpuSceneGradientPaint>,
    line_t0: f32,
    line_t1: f32,
    style: StrokeStyle,
    blend: SceneBlendMode,
) {
    let len = point_distance(p0, p1);
    if len <= 0.0001 {
        return;
    }
    let steps = (len / (width.max(1.0) * 4.0)).ceil().clamp(1.0, 8.0) as u32;
    for ix in 0..steps {
        let a_t = ix as f32 / steps as f32;
        let b_t = (ix + 1) as f32 / steps as f32;
        let mid_t = (a_t + b_t) * 0.5;
        let global_t = line_t0 + (line_t1 - line_t0) * mid_t;
        let pressure = stroke_taper_pressure(global_t, style).max(0.05);
        push_gpu_line_primitive(
            primitives,
            transform,
            p0.lerp(p1, a_t),
            p0.lerp(p1, b_t),
            width * pressure,
            color,
            opacity,
            gradient.clone(),
            0.0,
            1.0,
            0.0,
            0.0,
            blend,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_stroke_overlay_primitives(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    opacity: f32,
    line_t0: f32,
    line_t1: f32,
    style: StrokeStyle,
    blend: SceneBlendMode,
) {
    if width <= 0.0 || opacity <= 0.0 {
        return;
    }
    if style.texture_strength > 0.001 {
        push_gpu_stroke_stamp_primitives(
            primitives, transform, p0, p1, width, color, opacity, line_t0, line_t1, style, blend,
        );
    }
    if style.bristles > 0 {
        push_gpu_stroke_bristle_primitives(
            primitives, transform, p0, p1, width, color, opacity, line_t0, line_t1, style, blend,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_stroke_stamp_primitives(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    opacity: f32,
    line_t0: f32,
    line_t1: f32,
    style: StrokeStyle,
    blend: SceneBlendMode,
) {
    let len = point_distance(p0, p1);
    if len <= 0.0001 {
        return;
    }
    let strength = style.texture_strength.clamp(0.0, 1.0);
    let dx = (p1.x - p0.x) / len;
    let dy = (p1.y - p0.y) / len;
    let nx = -dy;
    let ny = dx;
    let spacing = (width * (1.35 - strength * 0.65)).clamp(2.0, 18.0);
    let steps = (len / spacing).ceil().clamp(1.0, 72.0) as u32;
    let texture_size = match style.texture {
        StrokeTexture::Charcoal => 1.65,
        StrokeTexture::Rough => 1.25,
        StrokeTexture::Pencil => 1.0,
        StrokeTexture::Sketch => 0.82,
        StrokeTexture::Marker => 0.72,
        StrokeTexture::Ink => 0.55,
        StrokeTexture::Hairline | StrokeTexture::Solid => 0.46,
    };
    let alpha_scale = match style.texture {
        StrokeTexture::Charcoal => 0.18,
        StrokeTexture::Pencil => 0.16,
        StrokeTexture::Rough => 0.14,
        StrokeTexture::Sketch => 0.12,
        StrokeTexture::Marker => 0.10,
        StrokeTexture::Ink => 0.08,
        StrokeTexture::Hairline | StrokeTexture::Solid => 0.06,
    };
    for step in 0..steps {
        let seed = stroke_texture_seed(p0, p1, step + 271);
        let keep = ((stroke_hash_signed(seed + 13.1) + 1.0) * 0.5).clamp(0.0, 1.0);
        if keep > strength {
            continue;
        }
        let local_t = ((step as f32 + 0.5) / steps as f32).clamp(0.0, 1.0);
        let global_t = line_t0 + (line_t1 - line_t0) * local_t;
        let pressure = stroke_taper_pressure(global_t, style).max(0.05);
        let tangent_noise = stroke_hash_signed(seed + 37.7) * spacing * 0.25;
        let normal_noise = stroke_hash_signed(seed + 91.3) * width * pressure * 0.45;
        let p = p0.lerp(p1, local_t);
        let size_noise = ((stroke_hash_signed(seed + 163.0) + 1.0) * 0.5).clamp(0.0, 1.0);
        let radius = (width * pressure * (0.035 + size_noise * 0.10) * texture_size).max(0.35);
        primitives.push(GpuScenePrimitive {
            kind: GPU_SHAPE_CIRCLE_FILL,
            transform,
            shape: [
                p.x + dx * tangent_noise + nx * normal_noise,
                p.y + dy * tangent_noise + ny * normal_noise,
                radius,
                0.0,
            ],
            radius: 0.0,
            stroke_width: 0.0,
            blur: 0.0,
            color,
            opacity: (opacity * strength * alpha_scale).clamp(0.0, 1.0),
            blend,
            gradient: None,
            line_t0: 0.0,
            line_t1: 1.0,
            taper_start: 0.0,
            taper_end: 0.0,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_stroke_bristle_primitives(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    opacity: f32,
    line_t0: f32,
    line_t1: f32,
    style: StrokeStyle,
    blend: SceneBlendMode,
) {
    let len = point_distance(p0, p1);
    if len <= 0.0001 {
        return;
    }
    let dx = (p1.x - p0.x) / len;
    let dy = (p1.y - p0.y) / len;
    let nx = -dy;
    let ny = dx;
    let count = style.bristles.clamp(0, 24);
    let pressure =
        ((stroke_taper_pressure(line_t0, style) + stroke_taper_pressure(line_t1, style)) * 0.5)
            .max(0.05);
    let bristle_width = (width * 0.08 * pressure).clamp(0.25, 2.2);
    let alpha_scale = match style.texture {
        StrokeTexture::Charcoal => 0.20,
        StrokeTexture::Rough => 0.18,
        StrokeTexture::Pencil => 0.15,
        StrokeTexture::Sketch => 0.13,
        _ => 0.11,
    };
    for ix in 0..count {
        let lane = if count <= 1 {
            0.0
        } else {
            ix as f32 / (count - 1) as f32 * 2.0 - 1.0
        };
        let seed = stroke_texture_seed(p0, p1, ix + 997);
        let offset = lane * width * pressure * 0.42
            + stroke_hash_signed(seed + 21.0) * style.roughness * 0.55;
        let start_t = (stroke_hash_signed(seed + 57.0) * 0.04).max(0.0);
        let end_t = 1.0 - (stroke_hash_signed(seed + 83.0) * 0.04).max(0.0);
        let start = p0.lerp(p1, start_t);
        let end = p0.lerp(p1, end_t);
        push_gpu_line_primitive(
            primitives,
            transform,
            Point2::new(start.x + nx * offset, start.y + ny * offset),
            Point2::new(end.x + nx * offset, end.y + ny * offset),
            bristle_width,
            color,
            (opacity * alpha_scale).clamp(0.0, 1.0),
            None,
            0.0,
            1.0,
            0.0,
            0.0,
            blend,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_stroke_segments(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    subpaths: &[Vec<Point2>],
    width: f32,
    color: [u8; 4],
    opacity: f32,
    gradient: Option<GpuSceneGradientPaint>,
    trim: (f32, f32),
    style: StrokeStyle,
    blend: SceneBlendMode,
) {
    for segment in trimmed_polyline_segments_with_progress(subpaths, trim) {
        push_gpu_styled_line_primitives(
            primitives,
            transform,
            segment.p0,
            segment.p1,
            width,
            color,
            opacity,
            gradient.clone(),
            segment.t0,
            segment.t1,
            style,
            blend,
        );
    }
}

fn scene_bool(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "no" | "off"
    )
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use crate::parse_graph_script;
    use crate::scene::drawable::{GpuSceneTextureLayer, GpuSceneTextureSource};
    use crate::scene::spatial::Affine2;
    use crate::scene::text::TextNode;
    use cosmic_text::Weight;
    use image::{Rgba, RgbaImage};
    use std::path::{Path, PathBuf};

    use super::{
        MotionLoomSceneRenderError, SceneFrameRenderer, SceneRenderProfile, eval_text_box_padding,
        eval_text_font_weight, eval_text_tracking_em, validate_scene_graph,
    };

    fn max_rgb(image: &image::RgbaImage) -> u8 {
        image
            .pixels()
            .map(|pixel| pixel[0].max(pixel[1]).max(pixel[2]))
            .max()
            .unwrap_or(0)
    }

    fn max_green(image: &image::RgbaImage) -> u8 {
        image.pixels().map(|pixel| pixel[1]).max().unwrap_or(0)
    }

    fn basic_text_node(render_scale: &str) -> TextNode {
        TextNode {
            id: None,
            value: "Text".to_string(),
            x: "0".to_string(),
            y: "0".to_string(),
            rotation: "0".to_string(),
            scale: "1".to_string(),
            scale_x: "1".to_string(),
            scale_y: "1".to_string(),
            skew_x: "0".to_string(),
            skew_y: "0".to_string(),
            transform_origin_x: "0".to_string(),
            transform_origin_y: "0".to_string(),
            width: None,
            max_width: None,
            align: None,
            tracking: None,
            font_size: "32".to_string(),
            render_scale: render_scale.to_string(),
            line_height: None,
            color: "#111111".to_string(),
            opacity: "1".to_string(),
            box_style: None,
            box_color: None,
            box_padding: None,
            box_padding_x: None,
            box_padding_y: None,
            box_radius: None,
            stroke: None,
            stroke_width: None,
            stroke_join: None,
            stroke_position: None,
            visible_chars: None,
            max_lines: None,
            font: None,
            font_family: None,
            font_weight: None,
            font_path: None,
            layout: None,
            animators: Vec::new(),
        }
    }

    #[test]
    fn scene_validation_rejects_missing_gradient_refs_before_render() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,48]}>
  <Background color="#000000" />
  <Rect x="0" y="0" width="64" height="48" fill="url(#bg_glow)" />
  <Present from="scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");

        let err = validate_scene_graph(&graph).expect_err("missing gradient ref should fail");
        assert!(
            err.to_string()
                .contains("gradient reference not found: bg_glow"),
            "unexpected validation error: {err}"
        );
    }

    #[test]
    fn text_font_weight_accepts_keywords_and_numeric_values() {
        assert_eq!(
            eval_text_font_weight(Some("bold"), 0.0, 0.0).expect("bold weight"),
            Weight::BOLD
        );
        assert_eq!(
            eval_text_font_weight(Some("semi-bold"), 0.0, 0.0).expect("semibold weight"),
            Weight::SEMIBOLD
        );
        assert_eq!(
            eval_text_font_weight(Some("900"), 0.0, 0.0).expect("numeric weight"),
            Weight::BLACK
        );
        assert_eq!(
            eval_text_font_weight(Some("400 + 300*$time.norm"), 1.0, 0.0)
                .expect("expression weight"),
            Weight::BOLD
        );
    }

    #[test]
    fn text_box_padding_accepts_single_and_pair_values() {
        assert_eq!(
            eval_text_box_padding("54", 0.0, 0.0).expect("single padding"),
            (54.0, 54.0)
        );
        assert_eq!(
            eval_text_box_padding("54 28", 0.0, 0.0).expect("space pair padding"),
            (54.0, 28.0)
        );
        assert_eq!(
            eval_text_box_padding("54,28", 0.0, 0.0).expect("comma pair padding"),
            (54.0, 28.0)
        );
    }

    #[test]
    fn text_gap_tracking_converts_pixels_to_em() {
        assert_eq!(
            eval_text_tracking_em(Some("-2"), 40.0, 0.0, 0.0).expect("negative tracking"),
            -0.05
        );
        assert_eq!(
            eval_text_tracking_em(Some("4"), 40.0, 0.0, 0.0).expect("positive tracking"),
            0.1
        );
    }

    fn cpu_texture_size(layer: GpuSceneTextureLayer) -> (u32, u32) {
        match layer.source {
            GpuSceneTextureSource::Cpu(image) => (image.width(), image.height()),
            GpuSceneTextureSource::Gpu(_) => panic!("expected CPU text texture layer"),
        }
    }

    #[test]
    fn scene_text_render_scale_supersamples_raster_texture() {
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let base = renderer
            .rasterize_text_texture_layer(
                &basic_text_node("1x"),
                Affine2::identity(),
                1.0,
                0.0,
                0.0,
                (800, 450),
            )
            .expect("base text raster")
            .expect("base text layer");
        let high = renderer
            .rasterize_text_texture_layer(
                &basic_text_node("4x"),
                Affine2::identity(),
                1.0,
                0.0,
                0.0,
                (800, 450),
            )
            .expect("4x text raster")
            .expect("4x text layer");

        let (base_w, base_h) = cpu_texture_size(base);
        let (high_w, high_h) = cpu_texture_size(high);
        assert!(
            high_w >= base_w * 3 && high_h >= base_h * 3,
            "expected renderScale=4x to allocate a much larger raster texture, base={base_w}x{base_h}, high={high_w}x{high_h}"
        );
    }

    #[test]
    fn scene_text_pill_box_expands_raster_texture_and_draws_background() {
        let mut text = basic_text_node("1x");
        text.value = "WE APPLY THEM".to_string();
        text.font_weight = Some("900".to_string());
        text.box_style = Some("pill".to_string());
        text.box_color = Some("#D9251D".to_string());
        text.box_padding = Some("54 28".to_string());
        text.box_radius = Some("999".to_string());

        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let layer = renderer
            .rasterize_text_texture_layer(&text, Affine2::identity(), 1.0, 0.0, 0.0, (800, 450))
            .expect("pill text raster")
            .expect("pill text layer");
        let GpuSceneTextureSource::Cpu(image) = layer.source else {
            panic!("expected CPU text texture layer");
        };

        let red_pixels = image
            .pixels()
            .filter(|pixel| pixel[0] > 180 && pixel[1] < 80 && pixel[2] < 70 && pixel[3] > 180)
            .count();
        assert!(
            red_pixels > 1000,
            "expected pill box background to draw many red pixels, got {red_pixels}"
        );
    }

    #[test]
    fn scene_image_asset_resolver_finds_repo_examples_path() {
        let path = super::resolve_local_scene_asset_path(
            "examples/motionloom/sample_assets/README/readme_showcase1.png",
        );
        assert!(
            path.exists(),
            "resolved image path does not exist: {path:?}"
        );
    }

    #[test]
    fn scene_image_asset_resolver_finds_scene_example_path() {
        let path = super::resolve_local_scene_asset_path(
            "examples/motionloom/scene/characters/character9/parts/face_base.png",
        );
        assert!(
            path.exists(),
            "resolved scene example image path does not exist: {path:?}"
        );
    }

    #[test]
    fn scene_image_asset_resolver_finds_motionloom_root_relative_path() {
        let path = super::resolve_local_scene_asset_path(
            "scene/characters/character9/parts/face_base.png",
        );
        assert!(
            path.exists(),
            "resolved motionloom-root scene image path does not exist: {path:?}"
        );
    }

    #[test]
    fn scene_asset_suffixes_accept_motionloom_root_paths() {
        let suffixes = crate::scene::resource::scene_asset_relative_suffixes(Path::new(
            "examples/motionloom/scene/characters/character9/parts/face_base.png",
        ));
        assert!(
            suffixes
                .iter()
                .any(|path| path == Path::new("scene/characters/character9/parts/face_base.png")),
            "expected motionloom-root relative suffix, got {suffixes:?}"
        );
    }

    #[test]
    fn scene_image_asset_resolver_finds_legacy_sample_assets_path() {
        let path =
            super::resolve_local_scene_asset_path("../sample_assets/README/readme_showcase1.png");
        assert!(
            path.exists(),
            "resolved image path does not exist: {path:?}"
        );
    }

    #[test]
    fn scene_color_parser_accepts_bgra_normalized_array() {
        assert_eq!(super::parse_color("[1,0,0,1]").unwrap(), [0, 0, 255, 255]);
        assert_eq!(super::parse_color("[0,0,1,1]").unwrap(), [255, 0, 0, 255]);
        assert_eq!(super::parse_color("[0,1,0,0.5]").unwrap(), [0, 255, 0, 128]);
    }

    #[test]
    fn scene_color_parser_accepts_bgra_byte_array() {
        assert_eq!(
            super::parse_color("[255, 128, 0, 64]").unwrap(),
            [0, 128, 255, 64]
        );
    }

    #[test]
    fn scene_path_morph_interpolates_compatible_path_data() {
        let d = r#"morph("0:M 0 0 L 10 0 L 10 10 Z", "2:M 0 0 L 20 10 L 20 20 Z")"#;
        let interpolated = super::eval_path_d(d, 0.5, 1.0).unwrap().into_owned();

        assert_eq!(interpolated, "M 0 0 L 15 5 L 15 15 Z");
    }

    #[test]
    fn scene_renderer_draws_path_d_morph() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,48]}>
  <Background color="#000000" />
  <Scene id="morph_scene">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Path id="morph_rect"
                  d={morph("0:M 4 4 L 18 4 L 18 20 L 4 20 Z", "1:M 4 4 L 42 4 L 42 20 L 4 20 Z")}
                  fill="#ff0000"
                  stroke="none"
                  opacity="1" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="morph_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");

        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 15)).expect("frame 15");
        let inside = rendered.get_pixel(12, 10);
        assert!(
            inside[0] > 200,
            "expected red morphed path pixel, got {inside:?}"
        );
    }

    #[test]
    fn scene_renderer_applies_text_animator_word_opacity() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[160,72]}>
  <Background color="#000000" />
  <Scene id="text_anim_scene">
    <Timeline>
      <Track id="main" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Text value="AI edits"
                  x="8"
                  y="44"
                  fontSize="32"
                  color="#ffffff">
              <TextAnimator selector="word" duration="0.40s" stagger="0.10s">
                <Style opacity={curve("0:0:linear, 0.30:1:ease_out")} />
              </TextAnimator>
            </Text>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="text_anim_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");

        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let start = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        assert!(
            max_rgb(&start) < 16,
            "expected animated text to start transparent"
        );

        let later = pollster::block_on(renderer.render_frame(&graph, 15)).expect("frame 15");
        assert!(
            max_rgb(&later) > 180,
            "expected animated text to become visible"
        );
    }

    #[test]
    fn scene_renderer_applies_text_stroke() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[120,72]}>
  <Background color="#000000" />
  <Scene id="text_stroke_scene">
    <Timeline>
      <Track id="main" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Text value="I"
                  x="32"
                  y="52"
                  fontSize="48"
                  color="#000000"
                  stroke="#ffffff"
                  strokeWidth="8" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="text_stroke_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");

        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        assert!(
            max_rgb(&rendered) > 180,
            "expected white text stroke on black background"
        );
    }

    #[test]
    fn scene_renderer_applies_text_glow_effect() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[180,96]}>
  <Background color="#000000" />
  <Scene id="text_glow_scene">
    <Timeline>
      <Track id="main" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Text value="GLOW"
                  x="22"
                  y="58"
                  fontSize="32"
                  color="#000000">
              <TextAnimator selector="word" duration="1s">
                <Effects>
                  <Glow radius="12" intensity="1.6" color="#00ff66" />
                </Effects>
              </TextAnimator>
            </Text>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="text_glow_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");

        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 15)).expect("frame 15");
        // The glow blur kernel spreads the green channel over a wider area,
        // so the peak intensity is lower than the raw glow color.
        assert!(
            max_green(&rendered) > 40,
            "expected green glow generated from text alpha, got {}",
            max_green(&rendered)
        );
    }

    #[test]
    fn scene_sequence_uses_local_curve_time() {
        let graph = parse_graph_script(
            r##"
<Graph fps={10} duration="2s" size={[64,48]}>
  <Background color="#000000" />
  <Scene id="timeline_scene">
    <Timeline>
      <Track id="main" z="0">
        <Sequence from="1s" duration="1s" out="hold">
          <Rect x="8"
                y={curve("0:30:linear, 1:10:linear")}
                width="24"
                height="12"
                color="#ffffff" />
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="timeline_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");

        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let before = pollster::block_on(renderer.render_frame(&graph, 5)).expect("frame before");
        assert_eq!(
            before.get_pixel(12, 25)[0],
            0,
            "sequence should be hidden before its from time"
        );

        let mid = pollster::block_on(renderer.render_frame(&graph, 15)).expect("frame mid");
        let inside = mid.get_pixel(12, 25);
        assert!(
            inside[0] > 220,
            "expected local-time rect at frame 15, got {inside:?}"
        );
    }

    #[test]
    fn scene_precompose_source_uses_sequence_local_time() {
        let graph = parse_graph_script(
            r##"
<Graph fps={10} duration="12s" size={[32,32]}>
  <Background color="#000000" />
  <Scene id="precompose_time_scene">
    <Defs>
      <Precompose id="fade_plate" duration="1s" size={[32,32]}>
        <Rect x="0"
              y="0"
              width="32"
              height="32"
              color="#ff0000"
              opacity={curve("0:0:linear, 1:1:linear")} />
      </Precompose>
    </Defs>
    <Timeline>
      <Track id="main" z="0">
        <Sequence from="10s" duration="1s" out="hold">
          <Layer source="fade_plate" />
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="precompose_time_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");

        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 105)).expect("frame 105");
        let pixel = rendered.get_pixel(4, 4);
        // The precompose rect has opacity=0.5 at sequence-local 0.5s.
        // When composited onto the opaque black background, the final pixel
        // is half-red (128) with full alpha (255).
        assert!(
            pixel[0] > 100 && pixel[0] < 160 && pixel[3] > 240,
            "expected precompose source to evaluate at sequence-local 0.5s (should be ~127), got {pixel:?}"
        );
    }

    #[test]
    fn scene_renderer_applies_group_mask_alpha() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,32]}>
  <Background color="#000000" />
  <Scene id="masked_scene">
    <Defs>
      <Mask id="left_half" shape="rect" x="0" y="0" width="32" height="32" />
    </Defs>

    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Group id="masked_red" mask="left_half">
              <Rect x="0" y="0" width="64" height="32" color="#ff0000" />
            </Group>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="masked_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");

        let inside = rendered.get_pixel(8, 8);
        let outside = rendered.get_pixel(48, 8);
        assert!(inside[0] > 200, "expected masked red pixel, got {inside:?}");
        assert!(
            outside[0] < 30 && outside[1] < 30 && outside[2] < 30,
            "expected outside mask to remain background, got {outside:?}"
        );
    }

    #[test]
    fn scene_renderer_applies_ellipse_group_mask_alpha() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,64]}>
  <Background color="#000000" />
  <Scene id="masked_scene">
    <Defs>
      <Mask id="ellipse_mask" shape="ellipse" x="16" y="8" width="32" height="48" />
    </Defs>

    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Group id="masked_red" mask="ellipse_mask">
              <Rect x="0" y="0" width="64" height="64" color="#ff0000" />
            </Group>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="masked_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");

        let center = rendered.get_pixel(32, 32);
        let bounding_box_corner = rendered.get_pixel(17, 9);
        assert!(
            center[0] > 200,
            "expected ellipse center to reveal red, got {center:?}"
        );
        assert!(
            bounding_box_corner[0] < 30
                && bounding_box_corner[1] < 30
                && bounding_box_corner[2] < 30,
            "expected ellipse corner outside shape to remain background, got {bounding_box_corner:?}"
        );
    }

    #[test]
    fn scene_renderer_applies_precompose_layer_luma_matte() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,32]}>
  <Background color="#000000" />
  <Scene id="matte_scene">
    <Defs>
      <Precompose id="green_source" size={[64,32]}>
        <Rect x="0" y="0" width="64" height="32" color="#00ff00" />
      </Precompose>
      <Precompose id="luma_cut" size={[64,32]}>
        <Rect x="0" y="0" width="32" height="32" color="#ffffff" />
        <Rect x="32" y="0" width="32" height="32" color="#000000" />
      </Precompose>
    </Defs>

    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Layer id="matted_green" source="green_source" matte="luma_cut" matteMode="luma" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="matte_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");

        let revealed = rendered.get_pixel(8, 8);
        let hidden = rendered.get_pixel(48, 8);
        assert!(
            revealed[1] > 200,
            "expected luma matte to reveal green pixel, got {revealed:?}"
        );
        assert!(
            hidden[0] < 30 && hidden[1] < 30 && hidden[2] < 30,
            "expected black luma matte to hide source, got {hidden:?}"
        );
    }

    #[test]
    fn scene_renderer_applies_defs_filter_to_layer() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,32]}>
  <Background color="#000000" />
  <Scene id="filter_scene">
    <Defs>
      <Filter id="soft_glow">
        <Blur radius="12" />
        <ColorMatrix saturation="1.0" brightness="1.8" />
      </Filter>
      <Precompose id="white_plate" size={[64,32]}>
        <Rect x="24" y="8" width="16" height="16" color="#ffffff" />
      </Precompose>
    </Defs>

    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Layer id="filtered_plate" source="white_plate" effect="soft_glow" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="filter_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");

        let softened_edge = rendered.get_pixel(18, 16);
        assert!(
            softened_edge[0] > 10 || softened_edge[1] > 10 || softened_edge[2] > 10,
            "expected filter blur to spread light outside source rect, got {softened_edge:?}"
        );
    }

    #[test]
    fn scene_gpu_native_path_applies_defs_filter_to_layer() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,32]}>
  <Background color="#000000" />
  <Scene id="filter_scene">
    <Defs>
      <Filter id="soft_glow">
        <Blur radius="12" />
        <ColorMatrix saturation="1.0" brightness="1.8" />
      </Filter>
      <Precompose id="white_plate" size={[64,32]}>
        <Rect x="24" y="8" width="16" height="16" color="#ffffff" />
      </Precompose>
    </Defs>

    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Layer id="filtered_plate" source="white_plate" effect="soft_glow" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="filter_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered =
            pollster::block_on(renderer.try_render_gpu_scene_tree_frame(&graph, 0.0, 0.0))
                .expect("GPU native filter render")
                .expect("expected GPU-native filter path");

        let softened_edge = rendered.get_pixel(18, 16);
        assert!(
            softened_edge[0] > 10 || softened_edge[1] > 10 || softened_edge[2] > 10,
            "expected GPU filter blur to spread light outside source rect, got {softened_edge:?}"
        );
    }

    #[test]
    fn scene_gpu_native_path_draws_face_jaw_without_cpu_overlay() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,64]}>
  <Background color="#000000" />
  <Scene id="face_jaw_scene">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <FaceJaw x="32"
                     y="8"
                     width="34"
                     height="48"
                     cheekWidth="28"
                     chinWidth="12"
                     chinSharpness="0.75"
                     jawEase="0.65"
                     closed="true"
                     fill="#ff0000"
                     stroke="none"
                     opacity="1" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="face_jaw_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered =
            pollster::block_on(renderer.try_render_gpu_scene_tree_frame(&graph, 0.0, 0.0))
                .expect("GPU native FaceJaw render")
                .expect("expected GPU-native FaceJaw path");

        let max_red = rendered.pixels().map(|pixel| pixel[0]).max().unwrap_or(0);
        assert!(
            max_red > 180,
            "expected GPU-native FaceJaw to draw red pixels, max red={max_red}"
        );
    }

    #[test]
    fn scene_renderer_draws_layer_container_children() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,32]}>
  <Background color="#000000" />
  <Scene id="layer_container_scene">
    <Defs>
      <Mask id="left_half" shape="rect" x="0" y="0" width="32" height="32" />
    </Defs>

    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Layer id="screened_layer" blend="screen" mask="left_half">
              <Rect x="0" y="0" width="64" height="32" color="#ff0000" />
            </Layer>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="layer_container_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");

        let revealed = rendered.get_pixel(8, 8);
        let hidden = rendered.get_pixel(48, 8);
        assert!(
            revealed[0] > 200,
            "expected layer child to render through mask, got {revealed:?}"
        );
        assert!(
            hidden[0] < 30 && hidden[1] < 30 && hidden[2] < 30,
            "expected layer mask to hide child content, got {hidden:?}"
        );
    }

    #[test]
    fn scene_renderer_uses_defs_mask_precompose_and_component() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,32]}>
  <Background color="#000000" />
  <Scene id="defs_resource_scene">
    <Defs>
      <Mask id="left_half" shape="rect" x="0" y="0" width="32" height="32" />
      <Precompose id="red_plate" size={[64,32]}>
        <Rect x="0" y="0" width="64" height="32" color="#ff0000" />
      </Precompose>
      <Component id="green_dot">
        <Circle x="0" y="0" radius="6" color="#00ff00" />
      </Component>
    </Defs>

    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Layer source="red_plate" mask="left_half" />
            <Use ref="green_dot" x="48" y="16" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="defs_resource_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");

        let red_revealed = rendered.get_pixel(8, 8);
        let red_hidden = rendered.get_pixel(40, 8);
        let component_pixel = rendered.get_pixel(48, 16);
        assert!(
            red_revealed[0] > 200,
            "expected Defs precompose to render through Defs mask, got {red_revealed:?}"
        );
        assert!(
            red_hidden[0] < 30 && red_hidden[1] < 30 && red_hidden[2] < 30,
            "expected Defs mask to hide precompose on right side, got {red_hidden:?}"
        );
        assert!(
            component_pixel[1] > 200,
            "expected Defs component to render through Use, got {component_pixel:?}"
        );
    }

    #[test]
    fn scene_gpu_native_path_uses_defs_mask_and_precompose() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,32]}>
  <Background color="#000000" />
  <Scene id="defs_resource_gpu_scene">
    <Defs>
      <Mask id="left_half" shape="rect" x="0" y="0" width="32" height="32" />
      <Precompose id="red_plate" size={[64,32]}>
        <Rect x="0" y="0" width="64" height="32" color="#ff0000" />
      </Precompose>
    </Defs>

    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Layer source="red_plate" mask="left_half" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="defs_resource_gpu_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU Defs resource test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };

        let revealed = rendered.get_pixel(8, 8);
        let hidden = rendered.get_pixel(48, 8);
        assert!(
            revealed[0] > 200,
            "expected GPU Defs precompose to render through Defs mask, got {revealed:?}"
        );
        assert!(
            hidden[0] < 30 && hidden[1] < 30 && hidden[2] < 30,
            "expected GPU Defs mask to hide precompose on right side, got {hidden:?}"
        );
    }

    #[test]
    fn scene_gpu_native_path_expands_repeat_use_components() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[80,32]}>
  <Background color="#000000" />
  <Scene id="component_gpu_scene">
    <Defs>
      <Component id="green_dot">
        <Circle x="0" y="0" radius="5" color="#00ff00" />
      </Component>
    </Defs>

    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Repeat count="3" x="16" y="16" xStep="20">
              <Use ref="green_dot" blend="screen" />
            </Repeat>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="component_gpu_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let nodes = super::scene_nodes_for_present(&graph).expect("present scene");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.try_render_gpu_scene_nodes_composited(
            nodes,
            graph.size,
            graph.size,
            super::Affine2::identity(),
            0.0,
            0.0,
            Some([0, 0, 0, 255]),
        )) {
            Ok(Some(rendered)) => rendered,
            Ok(None) => panic!("Repeat + Use + Component should stay GPU-native"),
            Err(MotionLoomSceneRenderError::GpuRender { message })
                if message.contains("GPU adapter") =>
            {
                eprintln!("Skipping GPU Use/Component native test: {message}");
                return;
            }
            Err(err) => panic!("unexpected native GPU render error: {err}"),
        };

        for x in [16, 36, 56] {
            let pixel = rendered.get_pixel(x, 16);
            assert!(
                pixel[1] > 180,
                "expected GPU-native Use/Component dot at x={x}, got {pixel:?}"
            );
        }
    }

    #[test]
    fn scene_gpu_primitive_path_expands_component_use() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,40]}>
  <Background color="#000000" />
  <Scene id="component_primitive_gpu_scene">
    <Defs>
      <Component id="blue_bar">
        <Rect x="0" y="0" width="18" height="20" color="#0077ff" />
      </Component>
    </Defs>

    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Use ref="blue_bar" x="22" y="10" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="component_primitive_gpu_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message })
                if message.contains("GPU adapter") =>
            {
                eprintln!("Skipping GPU Component primitive test: {message}");
                return;
            }
            Err(err) => panic!("unexpected GPU primitive render error: {err}"),
        };

        let pixel = rendered.get_pixel(30, 18);
        assert!(
            pixel[2] > 180 && pixel[0] < 80,
            "expected Component Use to render on primitive GPU path, got {pixel:?}"
        );
    }

    #[test]
    fn scene_gpu_native_path_draws_layer_container_children() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,32]}>
  <Background color="#000000" />
  <Scene id="layer_container_gpu_scene">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Layer id="screened_layer" blend="screen">
              <Rect x="0" y="0" width="32" height="32" color="#ff0000" />
            </Layer>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="layer_container_gpu_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");

        let inside = rendered.get_pixel(8, 8);
        let outside = rendered.get_pixel(48, 8);
        assert!(
            inside[0] > 200,
            "expected GPU layer child to render, got {inside:?}"
        );
        assert!(
            outside[0] < 30 && outside[1] < 30 && outside[2] < 30,
            "expected outside GPU layer child to remain background, got {outside:?}"
        );
    }

    #[test]
    fn scene_gpu_native_path_handles_mask_matte_precompose() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,32]}>
  <Background color="#000000" />
  <Scene id="gpu_native_matte">
    <Defs>
      <Mask id="left_mask" shape="rect" x="0" y="0" width="32" height="32" feather="1" />
      <Precompose id="green_source" size={[64,32]}>
        <Rect x="0" y="0" width="64" height="32" color="#00ff00" />
      </Precompose>
      <Precompose id="luma_cut" size={[64,32]}>
        <Rect x="0" y="0" width="32" height="32" color="#ffffff" />
        <Rect x="32" y="0" width="32" height="32" color="#000000" />
      </Precompose>
    </Defs>

    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Layer id="matted_green" source="green_source" matte="luma_cut" matteMode="luma" />
            <Group id="masked_blue" mask="left_mask">
              <Rect x="0" y="0" width="64" height="32" color="#0000ff" opacity="0.35" />
            </Group>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="gpu_native_matte" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let nodes = super::scene_nodes_for_present(&graph).expect("present scene");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = pollster::block_on(renderer.try_render_gpu_scene_nodes_composited(
            nodes,
            graph.size,
            graph.size,
            super::Affine2::identity(),
            0.0,
            0.0,
            Some([0, 0, 0, 255]),
        ))
        .expect("native gpu render")
        .expect("expected mask/matte/precompose native GPU path");

        let revealed = rendered.get_pixel(8, 8);
        let hidden = rendered.get_pixel(48, 8);
        assert!(
            revealed[1] > 150 || revealed[2] > 40,
            "expected native GPU matte/mask to reveal left-side content, got {revealed:?}"
        );
        assert!(
            hidden[1] < 80,
            "expected luma matte to hide right-side green source, got {hidden:?}"
        );
    }

    #[test]
    fn scene_group_deform_grid_warps_layer_content() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,48]}>
  <Background color="#000000" />
  <Scene id="deform_scene">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Group id="deformed"
                   deformGrid="2x2"
                   deformAmount="1"
                   gridFrom="10,10 30,10; 10,30 30,30"
                   gridTo="20,10 40,10; 20,30 40,30">
              <Rect id="source_rect"
                    x="10"
                    y="10"
                    width="20"
                    height="20"
                    color="#ff0000"
                    opacity="1" />
            </Group>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="deform_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");

        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let moved = rendered.get_pixel(25, 20);
        let original = rendered.get_pixel(12, 20);
        assert!(
            moved[0] > 200,
            "expected deformed red pixel at moved position, got {moved:?}"
        );
        assert!(
            original[0] < 30,
            "expected original source position to be transparent/black, got {original:?}"
        );
    }

    #[test]
    fn scene_gpu_group_deform_grid_warps_vector_primitives() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,48]}>
  <Background color="#000000" />
  <Scene id="deform_scene">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Group id="deformed"
                   deformGrid="2x2"
                   deformAmount="1"
                   gridFrom="10,10 30,10; 10,30 30,30"
                   gridTo="20,10 40,10; 20,30 40,30">
              <Path id="source_path"
                    d="M 10 10 L 30 10 L 30 30 L 10 30 Z"
                    fill="#ff0000"
                    stroke="none"
                    opacity="1" />
            </Group>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="deform_scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");

        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                if message.contains("CPU overlays") {
                    panic!("DeformGrid vector primitives must stay GPU-native: {message}");
                }
                eprintln!("Skipping GPU DeformGrid vector test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let moved = rendered.get_pixel(25, 20);
        let original = rendered.get_pixel(12, 20);
        assert!(
            moved[0] > 180,
            "expected GPU-deformed red pixel at moved position, got {moved:?}"
        );
        assert!(
            original[0] < 40,
            "expected original source position to be transparent/black, got {original:?}"
        );
    }

    #[test]
    fn scene_renderer_draws_unified_scene_layer_graph() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="4s" size={[128,72]} renderSize={[128,72]}>
  <Background color="#000000" />
  <Scene id="hello_scene">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="4s" out="hold">
          <Layer>
            <Text id="hello_text"
                  value="hello"
                  x="center"
                  y="center"
                  fontSize="18"
                  color="#ffffff"
                  opacity={curve("0:1:linear, 4:1:linear")} />
            <Circle id="accent_orb"
                    x="64"
                    y="36"
                    radius="18"
                    color="#3B82F6"
                    opacity="0.35" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Layer id="blue_mood_layer">
    <Effect id="blue_hsla"
            type="hsla"
            hue="220"
            saturation="0.22"
            lightness="0.08"
            alpha="0.32" />
  </Layer>
  <Tex id="scene_tex" from="hello_scene" fmt="rgba16f" />
  <Tex id="blurred_scene" fmt="rgba16f" size={[128,72]} />
  <Pass id="soft_blur"
        effect="blur"
        in={["scene_tex"]}
        out={["blurred_scene"]}
        params={{ sigma: "1.0" }} />
  <Tex id="final" from="blue_mood_layer" input="blurred_scene" fmt="rgba16f" />
  <Present from="final" />
</Graph>
"##,
        )
        .expect("graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Cpu));
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame");
        assert!(
            max_rgb(&rendered) > 20,
            "expected visible unified scene/layer output"
        );
    }

    #[test]
    fn scene_gpu_renderer_uses_background_as_scene_resource_in_unified_graph() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[32,18]} renderSize={[32,18]}>
  <Background color="#123456" />
  <Scene id="marker">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Circle x="16" y="9" radius="0" color="#ffffff" opacity="0" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Tex id="background_tex" from="scene" fmt="rgba16f" />
  <Present from="background_tex" />
</Graph>
"##,
        )
        .expect("graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("gpu frame");
        assert_eq!(*rendered.get_pixel(1, 1), Rgba([0x12, 0x34, 0x56, 0xff]));
    }

    #[test]
    fn scene_gpu_renderer_applies_graph_background_to_direct_present_scene() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[32,18]} renderSize={[32,18]}>
  <Background color="#f7f7f7" />
  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Circle x="16" y="9" radius="4" color="#000000" opacity="1" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("gpu frame");
        assert_eq!(*rendered.get_pixel(1, 1), Rgba([0xf7, 0xf7, 0xf7, 0xff]));
        assert_eq!(*rendered.get_pixel(16, 9), Rgba([0x00, 0x00, 0x00, 0xff]));
    }

    #[test]
    fn scene_renderer_applies_action_to_matching_character_part() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[160,120]}>
  <Background color="#ffffff" />

  <Action id="raise_arm" skeleton="humanoid_front_v1" duration="1s">
    <Pose t="0">
      <Bone id="upper_arm_r" rotation="0" />
    </Pose>
    <Pose t="0.5">
      <Bone id="upper_arm_r" rotation="-80" />
    </Pose>
    <Pose t="1">
      <Bone id="upper_arm_r" rotation="0" />
    </Pose>
  </Action>

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Character id="hero" rig="humanoid_front_v1" x="80" y="60">
              <Part id="upper_arm_r" x="0" y="0">
                <Path d="M 0 0 L 0 42"
                      stroke="#000000"
                      strokeWidth="8"
                      lineCap="round"
                      fill="none" />
              </Part>
            </Character>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>

  <ApplyAction target="hero" action="raise_arm" at="0s" />
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Cpu));
        let at_rest = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let raised = pollster::block_on(renderer.render_frame(&graph, 15)).expect("frame 15");
        assert_ne!(
            at_rest.as_raw(),
            raised.as_raw(),
            "action should change rendered pixels between poses"
        );
    }

    #[test]
    fn scene_renderer_applies_skeleton_parent_child_constraints() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[180,180]}>
  <Background color="#ffffff" />

  <Skeleton id="humanoid_front_v1">
    <Bone id="root" x="0" y="0" />
    <Bone id="upper_arm_r" parent="root" x="0" y="0" />
    <Bone id="forearm_r" parent="upper_arm_r" x="40" y="0" />
  </Skeleton>

  <Action id="bend" skeleton="humanoid_front_v1" duration="1s">
    <Pose t="0">
      <Bone id="upper_arm_r" rotation="0" />
    </Pose>
    <Pose t="0.5">
      <Bone id="upper_arm_r" rotation="90" />
    </Pose>
    <Pose t="1">
      <Bone id="upper_arm_r" rotation="0" />
    </Pose>
  </Action>

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Character id="hero" rig="humanoid_front_v1" x="80" y="80">
              <Part id="forearm_marker" attachTo="forearm_r">
                <Circle x="0" y="0" radius="8" fill="#ff0000" />
              </Part>
            </Character>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>

  <ApplyAction target="hero" action="bend" at="0s" />
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Cpu));
        let at_rest = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let bent = pollster::block_on(renderer.render_frame(&graph, 15)).expect("frame 15");
        let rest_pixel = at_rest.get_pixel(120, 80);
        let bent_pixel = bent.get_pixel(80, 120);
        assert!(
            rest_pixel[0] > 180 && rest_pixel[1] < 90 && rest_pixel[2] < 90,
            "expected red marker at parent-rest endpoint, got {rest_pixel:?}"
        );
        assert!(
            bent_pixel[0] > 180 && bent_pixel[1] < 90 && bent_pixel[2] < 90,
            "expected red marker to follow rotated parent endpoint, got {bent_pixel:?}"
        );
    }

    #[test]
    fn scene_renderer_draws_scene_group_shapes_and_text() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[220,140]}>
  <Background color="[1,1,1,1]" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Group id="card" x="20" y="20" opacity="1">
              <Shadow x="0" y="10" blur="18" color="[0,0,0,0.24]" />
              <Rect width="100" height="58" radius="8" color="[0,0,1,1]" />
              <Circle x="22" y="29" radius="9" color="[1,0,0,1]" />
              <Text value="Card" x="40" y="16" width="50" fontSize="18" lineHeight="22" color="[0,0,0,1]" />
            </Group>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let rect_pixel = rendered.get_pixel(112, 70);
        let circle_pixel = rendered.get_pixel(42, 49);

        assert!(
            rect_pixel[0] > 200 && rect_pixel[1] < 40 && rect_pixel[2] < 40,
            "expected red rect pixel, got {rect_pixel:?}"
        );
        assert!(
            circle_pixel[2] > 200 && circle_pixel[0] < 40 && circle_pixel[1] < 40,
            "expected blue circle pixel, got {circle_pixel:?}"
        );
    }

    #[test]
    fn scene_renderer_fits_render_size_into_output_size() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[100,100]} renderSize={[200,100]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Circle x="50" y="50" radius="10" color="#ff0000" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let center = rendered.get_pixel(100, 50);
        let left_logical_position = rendered.get_pixel(50, 50);

        assert!(
            center[0] > 200 && center[1] < 40 && center[2] < 40,
            "expected renderSize content to be centered and scaled into output, got {center:?}"
        );
        assert!(
            left_logical_position[0] < 40
                && left_logical_position[1] < 40
                && left_logical_position[2] < 40,
            "expected untransformed logical position to be background, got {left_logical_position:?}"
        );
    }

    #[test]
    fn scene_renderer_draws_trimmed_polyline_and_path() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[160,90]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Polyline points="10,24 110,24"
                      stroke="#2f83ff"
                      strokeWidth="8"
                      trimStart="0"
                      trimEnd="0.5" />
            <Path d="M 10 66 L 110 66"
                  stroke="#ff2f2f"
                  strokeWidth="8"
                  trimStart="0.5"
                  trimEnd="1" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let polyline_drawn = rendered.get_pixel(50, 24);
        let polyline_trimmed = rendered.get_pixel(95, 24);
        let path_trimmed = rendered.get_pixel(25, 66);
        let path_drawn = rendered.get_pixel(90, 66);

        assert!(
            polyline_drawn[2] > 180 && polyline_drawn[0] < 100,
            "expected blue polyline pixel, got {polyline_drawn:?}"
        );
        assert!(
            polyline_trimmed[0] < 20 && polyline_trimmed[1] < 20 && polyline_trimmed[2] < 20,
            "expected trimmed polyline tail to stay black, got {polyline_trimmed:?}"
        );
        assert!(
            path_trimmed[0] < 20 && path_trimmed[1] < 20 && path_trimmed[2] < 20,
            "expected trimmed path head to stay black, got {path_trimmed:?}"
        );
        assert!(
            path_drawn[0] > 180 && path_drawn[2] < 100,
            "expected red path pixel, got {path_drawn:?}"
        );
    }

    #[test]
    fn scene_renderer_draws_brush_part_path() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[120,80]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Defs>
      <Brush id="red_ink" stroke="#ff0000" strokeWidth="6" opacity="1" />
    </Defs>

    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Part id="mark" brush="red_ink" x="20" y="30">
              <Path id="mark_line" d="M 0 0 L 80 0" />
            </Part>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let pixel = rendered.get_pixel(60, 30);

        assert!(
            pixel[0] > 180 && pixel[1] < 80 && pixel[2] < 80,
            "expected red brush path pixel, got {pixel:?}"
        );
    }

    #[test]
    fn scene_renderer_draws_character_vector_nodes() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[120,90]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Character id="face" x="60" y="45" scale="1.5">
              <Path d="M -30 0 C -12 -20 12 -20 30 0"
                    stroke="#ff77aa"
                    strokeWidth="8" />
              <Line x1="-20" y1="12" x2="20" y2="12" width="6" color="#00ff00" />
              <Circle x="0" y="-5" radius="6" color="#ffffff" />
            </Character>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let eye = rendered.get_pixel(60, 37);
        let line = rendered.get_pixel(60, 63);

        assert!(
            eye[0] > 200 && eye[1] > 200 && eye[2] > 200,
            "expected white character circle, got {eye:?}"
        );
        assert!(
            line[1] > 180 && line[0] < 80 && line[2] < 80,
            "expected green character line, got {line:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_draws_character_overlay() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[100,80]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Character x="50" y="40">
              <Path d="M -24 0 L 24 0" stroke="#00ff00" strokeWidth="8" />
            </Character>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU character overlay test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let center = rendered.get_pixel(50, 40);

        assert!(
            center[1] > 180 && center[0] < 80 && center[2] < 80,
            "expected green GPU character path pixel, got {center:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_draws_filled_path_overlay() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[100,80]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Path d="M 20 20 L 70 20 L 70 60 L 20 60 Z"
                  fill="#00ff00"
                  stroke="none" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU filled path overlay test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let center = rendered.get_pixel(45, 40);

        assert!(
            center[1] > 180 && center[0] < 80 && center[2] < 80,
            "expected green GPU filled path overlay pixel, got {center:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_draws_sketch_stroke_style() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[100,80]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Line x1="12" y1="40" x2="88" y2="40"
                  width="8"
                  color="#00ff00"
                  strokeStyle="sketch"
                  strokeRoughness="1.4"
                  strokeCopies="4"
                  strokeTexture="0.35"
                  strokeBristles="3"
                  strokePressure="auto"
                  strokePressureMin="0.4"
                  strokePressureCurve="1.2" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU sketch stroke style test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let center = rendered.get_pixel(50, 40);

        assert!(
            center[1] > 150 && center[0] < 100 && center[2] < 100,
            "expected green GPU sketch line pixel, got {center:?}"
        );
    }

    #[test]
    fn scene_renderer_draws_filled_path_and_mask() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[120,90]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Path d="M 10 10 L 45 10 L 45 45 L 10 45 Z"
                  fill="#00ff00"
                  stroke="none" />
            <Mask shape="circle" x="88" y="42" radius="18">
              <Rect x="65" y="20" width="46" height="46" color="#ff0000" />
            </Mask>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let filled = rendered.get_pixel(28, 28);
        let masked_inside = rendered.get_pixel(88, 42);
        let masked_outside = rendered.get_pixel(66, 22);

        assert!(
            filled[1] > 180 && filled[0] < 80 && filled[2] < 80,
            "expected green filled path pixel, got {filled:?}"
        );
        assert!(
            masked_inside[0] > 180 && masked_inside[1] < 80 && masked_inside[2] < 80,
            "expected red masked center, got {masked_inside:?}"
        );
        assert!(
            masked_outside[0] < 40 && masked_outside[1] < 40 && masked_outside[2] < 40,
            "expected masked corner to stay black, got {masked_outside:?}"
        );
    }

    #[test]
    fn scene_renderer_character_mask_clips_filled_path() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[120,90]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Character x="60" y="45">
              <Mask shape="circle" x="0" y="0" radius="18">
                <Path d="M -30 -30 L 30 -30 L 30 30 L -30 30 Z"
                      fill="#ff77aa"
                      stroke="none" />
              </Mask>
            </Character>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let inside = rendered.get_pixel(60, 45);
        let outside = rendered.get_pixel(35, 45);

        assert!(
            inside[0] > 180 && inside[2] > 100,
            "expected pink character mask fill, got {inside:?}"
        );
        assert!(
            outside[0] < 40 && outside[1] < 40 && outside[2] < 40,
            "expected clipped outside pixel to stay black, got {outside:?}"
        );
    }

    #[test]
    fn scene_renderer_camera_centers_world_coordinate() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[100,80]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track role="camera" z="1000">
        <Sequence duration="1s" out="hold">
          <Camera x="100" y="40" zoom="1" />
        </Sequence>
      </Track>
      <Track space="world" z="0">
        <Sequence duration="1s" out="hold">
          <Layer>
            <Circle x="100" y="40" radius="8" color="#00ff00" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let centered = rendered.get_pixel(50, 40);

        assert!(
            centered[1] > 180 && centered[0] < 80 && centered[2] < 80,
            "expected camera-centered green circle, got {centered:?}"
        );
    }

    #[test]
    fn scene_renderer_camera_follow_maps_node_to_anchor() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[100,80]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track role="camera" z="1000">
        <Sequence duration="1s" out="hold">
          <Camera follow="marker" anchorX="25%" anchorY="75%" zoom="1" worldBounds="0,0,200,160" />
        </Sequence>
      </Track>
      <Track space="world" z="0">
        <Sequence duration="1s" out="hold">
          <Layer>
            <Circle id="marker" x="120" y="60" radius="8" color="#00ff00" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let anchored = rendered.get_pixel(25, 60);

        assert!(
            anchored[1] > 180 && anchored[0] < 80 && anchored[2] < 80,
            "expected camera-followed green circle at anchor, got {anchored:?}"
        );
    }

    #[test]
    fn scene_renderer_higher_track_z_paints_later() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[80,80]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track id="back" space="world" z="0">
        <Sequence duration="1s" out="hold">
          <Layer>
            <Rect x="16" y="16" width="40" height="40" color="#ff0000" />
          </Layer>
        </Sequence>
      </Track>
      <Track id="front" space="world" z="10">
        <Sequence duration="1s" out="hold">
          <Layer>
            <Rect x="24" y="24" width="40" height="40" color="#00ff00" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let overlap = rendered.get_pixel(30, 30);

        assert!(
            overlap[1] > 180 && overlap[0] < 80 && overlap[2] < 80,
            "expected higher-z green rect to paint over lower-z red rect, got {overlap:?}"
        );
    }

    #[test]
    fn scene_renderer_layer_z_depth_sorts_far_before_near() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[100,100]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track space="world" z="0">
        <Sequence duration="1s" out="hold">
          <Layer zDepth="-1">
            <Rect x="35" y="35" width="30" height="30" color="#00ff00" />
          </Layer>
          <Layer zDepth="2">
            <Rect x="30" y="30" width="40" height="40" color="#ff0000" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let overlap = rendered.get_pixel(50, 50);

        assert!(
            overlap[1] > 180 && overlap[0] < 80 && overlap[2] < 80,
            "expected near zDepth=-1 green layer to paint over far zDepth=2 red layer, got {overlap:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_layer_z_depth_sorts_far_before_near() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[100,100]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track space="world" z="0">
        <Sequence duration="1s" out="hold">
          <Layer zDepth="-1">
            <Rect x="35" y="35" width="30" height="30" color="#00ff00" />
          </Layer>
          <Layer zDepth="2">
            <Rect x="30" y="30" width="40" height="40" color="#ff0000" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU zDepth sort test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let overlap = rendered.get_pixel(50, 50);

        assert!(
            overlap[1] > 180 && overlap[0] < 80 && overlap[2] < 80,
            "expected GPU near zDepth=-1 green layer to paint over far zDepth=2 red layer, got {overlap:?}"
        );
    }

    #[test]
    fn scene_renderer_layer_z_depth_overrides_track_z_depth() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[100,100]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track space="world" z="0" zDepth="2">
        <Sequence duration="1s" out="hold">
          <Layer zDepth="-1">
            <Rect x="35" y="35" width="30" height="30" color="#00ff00" />
          </Layer>
          <Layer>
            <Rect x="30" y="30" width="40" height="40" color="#ff0000" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let overlap = rendered.get_pixel(50, 50);

        assert!(
            overlap[1] > 180 && overlap[0] < 80 && overlap[2] < 80,
            "expected Layer zDepth=-1 to override Track zDepth=2 and paint green over inherited-depth red, got {overlap:?}"
        );
    }

    #[test]
    fn scene_renderer_track_z_depth_sorts_world_tracks() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[100,100]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track id="near" space="world" z="0" zDepth="-1">
        <Sequence duration="1s" out="hold">
          <Layer>
            <Rect x="35" y="35" width="30" height="30" color="#00ff00" />
          </Layer>
        </Sequence>
      </Track>
      <Track id="far" space="world" z="10" zDepth="2">
        <Sequence duration="1s" out="hold">
          <Layer>
            <Rect x="30" y="30" width="40" height="40" color="#ff0000" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let overlap = rendered.get_pixel(50, 50);

        assert!(
            overlap[1] > 180 && overlap[0] < 80 && overlap[2] < 80,
            "expected near zDepth=-1 track to paint over higher-z far zDepth=2 track, got {overlap:?}"
        );
    }

    #[test]
    fn scene_renderer_accepts_scene_tex_pass_present_pipeline() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,48]}>
  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Rect x="0" y="0" width="64" height="48" color="#ff0000" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Tex id="src" fmt="rgba16f" from="scene:scene0" />
  <Tex id="out" fmt="rgba16f" size={[64,48]} />
  <Pass id="fx_opacity" kind="compute"
        effect="opacity"
        in={["src"]} out={["out"]}
        params={{ opacity: "0.5" }} />
  <Present from="out" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let pixel = rendered.get_pixel(10, 10);

        assert!(
            pixel[0] > 200 && pixel[1] < 40 && pixel[2] < 40,
            "expected red scene pixel, got {pixel:?}"
        );
        assert!(
            (120..=136).contains(&pixel[3]),
            "expected opacity pass to halve alpha, got {pixel:?}"
        );
    }

    #[test]
    fn scene_renderer_color_to_alpha_keys_background_after_process_pass() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,48]}>
  <Background color="#FFFFFF" />
  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Rect x="20" y="14" width="24" height="20" radius="4" color="#101827" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Process id="keyed">
    <Tex id="src" fmt="rgba16f" from="scene:scene0" />
    <Tex id="out" fmt="rgba16f" size={[64,48]} />
    <Pass id="key_color" kind="compute"
          effect="color_to_alpha"
          in={["src"]} out={["out"]}
          params={{ color: "#FFFFFF", tolerance: "0.02", softness: "0.12" }} />
  </Process>
  <Present from="keyed" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let bg = rendered.get_pixel(4, 4);
        let center = rendered.get_pixel(32, 24);

        assert!(
            bg[3] < 8,
            "expected white background to key out, got {bg:?}"
        );
        assert!(
            center[3] > 240 && center[0] < 40 && center[1] < 50 && center[2] < 70,
            "expected non-white center to remain opaque, got {center:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_draws_scene_group_shapes_and_text() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[220,140]}>
  <Background color="[1,1,1,1]" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Group id="card" x="20" y="20" opacity="1">
              <Shadow x="0" y="10" blur="18" color="[0,0,0,0.24]" />
              <Rect width="100" height="58" radius="8" color="[0,0,1,1]" />
              <Circle x="22" y="29" radius="9" color="[1,0,0,1]" />
              <Text value="Card" x="40" y="16" width="50" fontSize="18" lineHeight="22" color="[0,0,0,1]" />
            </Group>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU scene primitive test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let rect_pixel = rendered.get_pixel(112, 70);
        let circle_pixel = rendered.get_pixel(42, 49);

        assert!(
            rect_pixel[0] > 200 && rect_pixel[1] < 40 && rect_pixel[2] < 40,
            "expected red rect pixel, got {rect_pixel:?}"
        );
        assert!(
            circle_pixel[2] > 200 && circle_pixel[0] < 40 && circle_pixel[1] < 40,
            "expected blue circle pixel, got {circle_pixel:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_draws_text_inside_process_scene_source() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="3s" size={[800,450]} renderSize={[800,450]}>
  <Background color="#FFFFFF" />

  <Scene id="ProcessBasicSource">
    <Timeline>
      <Track id="main" space="world" z="0">
        <Sequence from="0s" duration="3s" out="hold">
          <Layer>
            <Rect x="0" y="0" width="800" height="450" color="#FFFFFF" />
            <Text x="52" y="64" value="Process effect: opacity" fontSize="32" color="#111827" />
            <Text x="52" y="98" value="Standalone scene source for the basic Process pass" fontSize="20" color="#6B7280" />
            <Rect x="250" y="154" width="300" height="142" radius="32" color="#111827" opacity="1" />
            <Circle x="400" y="225" radius="38" color="#FFFFFF" opacity="1" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>

  <Process id="ProcessBasic">
    <Tex id="src" fmt="rgba16f" from="scene:ProcessBasicSource" />
    <Tex id="out" fmt="rgba16f" size={[800,450]} />
    <Pass id="fx_opacity" kind="compute" effect="opacity"
          in={["src"]} out={["out"]}
          params={{ opacity: "1.0" }} />
  </Process>

  <Present from="ProcessBasic" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message })
                if message.contains("GPU adapter")
                    || message.contains("graphics adapter")
                    || message.contains("metal found no adapters") =>
            {
                eprintln!("Skipping GPU process text source test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let mut text_dark_pixels = 0usize;
        for y in 48..118 {
            for x in 48..500 {
                let pixel = rendered.get_pixel(x, y);
                if pixel[0] < 170 && pixel[1] < 180 && pixel[2] < 190 && pixel[3] > 20 {
                    text_dark_pixels += 1;
                }
            }
        }
        assert!(
            text_dark_pixels > 40,
            "expected GPU-rendered text pixels in the scene source, got {text_dark_pixels}"
        );
    }

    #[test]
    fn scene_gpu_renderer_draws_screen_blend_rect() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[96,64]}>
  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Rect x="0" y="0" width="96" height="64" color="#0000ff" />
            <Rect x="20" y="12" width="56" height="40" color="#ff0000" opacity="0.5" blend="screen" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU screen blend rect test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let pixel = rendered.get_pixel(48, 32);
        assert!(
            pixel[0] > 90 && pixel[2] > 200,
            "expected screen-blended magenta/blue pixel, got {pixel:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_batches_many_gradient_paths() {
        let mut script = String::from(
            r##"
<Graph fps={30} duration="1s" size={[240,160]}>
  <Background color="#ffffff" />

  <Scene id="scene0">
    <Defs>
      <LinearGradient id="sclera_soft" x1="0" y1="0" x2="0" y2="1"
                      stops="0:#D3CEE6, 0.30:#F7F7F7, 0.70:#F7F7F7, 1:#ffffff" />
      <LinearGradient id="lid_grey" x1="0" y1="0" x2="0" y2="1"
                      stops="0:#B7C7C7, 0.70:#B7C7C7, 1:#8da0a3" />
    </Defs>
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
"##,
        );
        for ix in 0..72 {
            let x = 18 + (ix % 12) * 17;
            let y = 20 + (ix / 12) * 18;
            script.push_str(&format!(
                r##"    <Path d="M {x} {y} C {cx1} {cy1} {cx2} {cy2} {x2} {y2} C {cx3} {cy3} {cx4} {cy4} {x} {y} Z"
          fill="url(#sclera_soft)" stroke="#3F5877" strokeWidth="1.2" opacity="0.78" />
"##,
                cx1 = x + 8,
                cy1 = y - 8,
                cx2 = x + 28,
                cy2 = y - 8,
                x2 = x + 36,
                y2 = y,
                cx3 = x + 28,
                cy3 = y + 12,
                cx4 = x + 8,
                cy4 = y + 12,
            ));
            script.push_str(&format!(
                r##"    <Path d="M {x} {line_y} C {cx1} {cy1} {cx2} {cy2} {x2} {line_y}"
          fill="none" stroke="#3F5877" strokeWidth="2.4" lineCap="round" />
"##,
                line_y = y + 6,
                cx1 = x + 8,
                cy1 = y - 5,
                cx2 = x + 28,
                cy2 = y - 5,
                x2 = x + 36,
            ));
        }
        script.push_str(
            r##"          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        );

        let graph = parse_graph_script(&script).expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU batched path stress test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };

        assert!(
            rendered
                .pixels()
                .any(|pixel| pixel[0] < 120 && pixel[1] < 140 && pixel[2] < 170),
            "expected batched gradient paths to render visible dark strokes"
        );
    }

    #[test]
    fn scene_gpu_renderer_applies_scene_blur_post_pass() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[80,40]}>
  <Background color="#ffffff" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Rect x="35" y="10" width="10" height="20" color="#000000" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Tex id="src" fmt="rgba16f" from="scene0" />
  <Tex id="out" fmt="rgba16f" size={[80,40]} />
  <Pass id="blur_h" kind="compute"
        effect="gaussian_5tap_h"
        in={["src"]} out={["out"]}
        params={{ sigma: "8" }} />
  <Present from="out" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU scene post-pass test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let softened_edge = rendered.get_pixel(31, 20);

        assert!(
            softened_edge[0] < 245 && softened_edge[1] < 245 && softened_edge[2] < 245,
            "expected GPU blur to soften edge pixel, got {softened_edge:?}"
        );
    }

    #[test]
    fn scene_renderer_applies_process_bloom_to_whole_scene() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[96,48]}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Rect x="42" y="18" width="12" height="12" color="#ffffff" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Process id="final_glow">
    <Tex id="src" fmt="rgba16f" from="scene0" />
    <Tex id="out" fmt="rgba16f" size={[96,48]} />
    <Pass id="post_bloom" kind="compute"
          effect="bloom"
          in={["src"]} out={["out"]}
          params={{ threshold: "0.2", intensity: "2.0", sigma: "8.0" }} />
  </Process>
  <Present from="final_glow" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Cpu));
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("CPU render");
        let halo = rendered.get_pixel(37, 24);

        assert!(
            halo[0] > 0 || halo[1] > 0 || halo[2] > 0,
            "expected bloom to brighten pixels outside the source rect, got {halo:?}"
        );
    }

    fn write_test_svg(name: &str, body: &str) -> PathBuf {
        let svg_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-{name}-{}.svg",
            std::process::id()
        ));
        std::fs::write(&svg_path, body).expect("write test svg");
        svg_path
    }

    #[test]
    fn scene_text_opacity_fades_in_over_time() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="3s" size={[640,360]}>
  <Background color="#000000" />
  <Text value="hello world"
        x="center"
        y="center"
        fontSize="72"
        color="#ffffff"
        opacity="min($time.sec / 1.0, 1.0)" />
  <Present from="scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let at_zero = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let at_half = pollster::block_on(renderer.render_frame(&graph, 15)).expect("frame 15");
        let at_full = pollster::block_on(renderer.render_frame(&graph, 30)).expect("frame 30");

        assert_eq!(max_rgb(&at_zero), 0);
        assert!(max_rgb(&at_half) > 40);
        assert!(max_rgb(&at_half) < max_rgb(&at_full));
        assert!(max_rgb(&at_full) > 200);
    }

    #[test]
    fn scene_image_draws_exact_path_with_scale_and_opacity() {
        let image_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-image-{}.png",
            std::process::id()
        ));
        RgbaImage::from_pixel(8, 6, Rgba([255, 0, 0, 255]))
            .save(&image_path)
            .expect("write test image");

        let graph = parse_graph_script(&format!(
            r##"
<Graph fps={{30}} duration="1s" size={{[64,48]}}>
  <Background color="#000000" />
  <Image src="{}"
         x="10"
         y="12"
         scale="2.0"
         opacity="0.5" />
  <Present from="scene" />
</Graph>
"##,
            image_path.to_string_lossy()
        ))
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let inside = rendered.get_pixel(12, 14);
        let outside = rendered.get_pixel(2, 2);

        assert!(inside[0] > 100, "expected red image pixel, got {inside:?}");
        assert_eq!(inside[1], 0);
        assert_eq!(inside[2], 0);
        assert_eq!(outside[0], 0);

        let _ = std::fs::remove_file(image_path);
    }

    #[test]
    fn scene_character_part_draws_image_child() {
        let image_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-character-image-{}.png",
            std::process::id()
        ));
        RgbaImage::from_pixel(8, 6, Rgba([255, 0, 0, 255]))
            .save(&image_path)
            .expect("write test image");

        let graph = parse_graph_script(&format!(
            r##"
<Graph fps={{30}} duration="1s" size={{[64,48]}}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Character id="test_character" x="20" y="10">
              <Part id="image_part" x="4" y="3">
                <Image src="{}" x="0" y="0" scale="2.0" opacity="1.0" />
              </Part>
            </Character>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>

  <Present from="scene0" />
</Graph>
"##,
            image_path.to_string_lossy()
        ))
        .expect("scene graph parse");

        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Cpu));
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let inside = rendered.get_pixel(25, 14);
        let outside = rendered.get_pixel(2, 2);

        assert!(
            inside[0] > 200 && inside[1] < 40 && inside[2] < 40,
            "expected transformed character image pixel, got {inside:?}"
        );
        assert_eq!(outside[0], 0);

        let _ = std::fs::remove_file(image_path);
    }

    #[test]
    fn scene_character_part_draws_image_child_gpu_profile() {
        let image_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-character-image-gpu-{}.png",
            std::process::id()
        ));
        RgbaImage::from_pixel(8, 6, Rgba([255, 0, 0, 255]))
            .save(&image_path)
            .expect("write test image");

        let graph = parse_graph_script(&format!(
            r##"
<Graph fps={{30}} duration="1s" size={{[64,48]}}>
  <Background color="#000000" />

  <Scene id="scene0">
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Character id="test_character" x="20" y="10">
              <Part id="image_part" x="4" y="3">
                <Image src="{}" x="0" y="0" scale="2.0" opacity="1.0" />
              </Part>
            </Character>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>

  <Present from="scene0" />
</Graph>
"##,
            image_path.to_string_lossy()
        ))
        .expect("scene graph parse");

        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => {
                let inside = rendered.get_pixel(25, 14);
                let outside = rendered.get_pixel(2, 2);
                assert!(
                    inside[0] > 200 && inside[1] < 40 && inside[2] < 40,
                    "expected transformed character image pixel, got {inside:?}"
                );
                assert_eq!(outside[0], 0);
            }
            Err(MotionLoomSceneRenderError::GpuRender { message })
                if message.contains("No compatible GPU adapter found") =>
            {
                eprintln!("skipping GPU character image test: {message}");
            }
            Err(err) => panic!("GPU character image render failed: {err}"),
        }

        let _ = std::fs::remove_file(image_path);
    }

    #[test]
    fn scene_svg_draws_exact_path_with_scale_and_opacity() {
        let svg_path = write_test_svg(
            "svg",
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6" viewBox="0 0 8 6">
  <rect width="8" height="6" fill="#ff0000"/>
</svg>"##,
        );

        let graph = parse_graph_script(&format!(
            r##"
<Graph fps={{30}} duration="1s" size={{[64,48]}}>
  <Background color="#000000" />
  <Svg src="{}"
       x="10"
       y="12"
       scale="2.0"
       opacity="0.5" />
  <Present from="scene" />
</Graph>
"##,
            svg_path.to_string_lossy()
        ))
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let inside = rendered.get_pixel(12, 14);
        let outside = rendered.get_pixel(2, 2);

        assert!(inside[0] > 100, "expected red SVG pixel, got {inside:?}");
        assert_eq!(inside[1], 0);
        assert_eq!(inside[2], 0);
        assert_eq!(outside[0], 0);

        let _ = std::fs::remove_file(svg_path);
    }

    #[test]
    fn scene_svg_data_uri_utf8_renders() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[64,48]}>
  <Background color="#000000" />
  <Svg src="data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='8' height='6' viewBox='0 0 8 6'><rect width='8' height='6' fill='%23ff0000'/></svg>"
       x="10"
       y="12"
       scale="2.0"
       opacity="1.0" />
  <Present from="scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = pollster::block_on(SceneFrameRenderer::new());
        let rendered = pollster::block_on(renderer.render_frame(&graph, 0)).expect("frame 0");
        let inside = rendered.get_pixel(12, 14);
        assert!(
            inside[0] > 200,
            "expected red SVG data URI pixel, got {inside:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_draws_image_when_available() {
        let image_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-gpu-image-{}.png",
            std::process::id()
        ));
        RgbaImage::from_pixel(8, 6, Rgba([0, 255, 0, 255]))
            .save(&image_path)
            .expect("write test image");

        let graph = parse_graph_script(&format!(
            r##"
<Graph fps={{30}} duration="1s" size={{[64,48]}}>
  <Background color="#000000" />
  <Image src="{}"
         x="10"
         y="12"
         scale="2.0"
         opacity="1.0" />
  <Present from="scene" />
</Graph>
"##,
            image_path.to_string_lossy()
        ))
        .expect("scene graph parse");

        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU render test: {message}");
                let _ = std::fs::remove_file(image_path);
                return;
            }
            Err(err) => panic!("unexpected GPU render error: {err}"),
        };
        let inside = rendered.get_pixel(12, 14);
        assert!(
            inside[1] > 150,
            "expected green GPU image pixel, got {inside:?}"
        );

        let _ = std::fs::remove_file(image_path);
    }

    #[test]
    fn scene_gpu_renderer_draws_svg_when_available() {
        let svg_path = write_test_svg(
            "gpu-svg",
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6" viewBox="0 0 8 6">
  <rect width="8" height="6" fill="#00ff00"/>
</svg>"##,
        );

        let graph = parse_graph_script(&format!(
            r##"
<Graph fps={{30}} duration="1s" size={{[64,48]}}>
  <Background color="#000000" />
  <Svg src="{}"
       x="10"
       y="12"
       scale="2.0"
       opacity="1.0" />
  <Present from="scene" />
</Graph>
"##,
            svg_path.to_string_lossy()
        ))
        .expect("scene graph parse");

        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU SVG render test: {message}");
                let _ = std::fs::remove_file(svg_path);
                return;
            }
            Err(err) => panic!("unexpected GPU render error: {err}"),
        };
        let inside = rendered.get_pixel(12, 14);
        assert!(
            inside[1] > 150,
            "expected green GPU SVG pixel, got {inside:?}"
        );

        let _ = std::fs::remove_file(svg_path);
    }

    #[test]
    fn scene_gpu_prores_renderer_draws_image_when_available() {
        let image_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-gpu-prores-image-{}.png",
            std::process::id()
        ));
        RgbaImage::from_pixel(8, 6, Rgba([0, 0, 255, 255]))
            .save(&image_path)
            .expect("write test image");

        let graph = parse_graph_script(&format!(
            r##"
<Graph fps={{30}} duration="1s" size={{[64,48]}}>
  <Background color="#000000" />
  <Image src="{}"
         x="10"
         y="12"
         scale="2.0"
         opacity="1.0" />
  <Present from="scene" />
</Graph>
"##,
            image_path.to_string_lossy()
        ))
        .expect("scene graph parse");

        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU render test: {message}");
                let _ = std::fs::remove_file(image_path);
                return;
            }
            Err(err) => panic!("unexpected GPU render error: {err}"),
        };
        let inside = rendered.get_pixel(12, 14);
        assert!(
            inside[2] > 150,
            "expected blue GPU image pixel, got {inside:?}"
        );

        let _ = std::fs::remove_file(image_path);
    }

    #[test]
    fn scene_gpu_renderer_composites_multiple_images_when_available() {
        let image1_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-gpu-multi-1-{}.png",
            std::process::id()
        ));
        let image2_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-gpu-multi-2-{}.png",
            std::process::id()
        ));
        RgbaImage::from_pixel(8, 8, Rgba([255, 0, 0, 255]))
            .save(&image1_path)
            .expect("write test image 1");
        RgbaImage::from_pixel(8, 8, Rgba([0, 255, 0, 255]))
            .save(&image2_path)
            .expect("write test image 2");

        let graph = parse_graph_script(&format!(
            r##"
<Graph fps={{30}} duration="1s" size={{[64,48]}}>
  <Background color="#000000" />
  <Image src="{}" x="4" y="10" scale="1.0" opacity="1.0" />
  <Image src="{}" x="24" y="10" scale="1.0" opacity="1.0" />
  <Present from="scene" />
</Graph>
"##,
            image1_path.to_string_lossy(),
            image2_path.to_string_lossy()
        ))
        .expect("scene graph parse");

        let mut renderer =
            pollster::block_on(SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu));
        let rendered = match pollster::block_on(renderer.render_frame(&graph, 0)) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU render test: {message}");
                let _ = std::fs::remove_file(image1_path);
                let _ = std::fs::remove_file(image2_path);
                return;
            }
            Err(err) => panic!("unexpected GPU render error: {err}"),
        };
        let first = rendered.get_pixel(5, 11);
        let second = rendered.get_pixel(25, 11);

        assert!(
            first[0] > 200 && first[1] < 30,
            "expected first GPU image to remain visible, got {first:?}"
        );
        assert!(
            second[1] > 200 && second[0] < 30,
            "expected second GPU image to remain visible, got {second:?}"
        );

        let _ = std::fs::remove_file(image1_path);
        let _ = std::fs::remove_file(image2_path);
    }
}
