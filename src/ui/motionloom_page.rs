// =========================================
// =========================================
// src/ui/motionloom_page.rs — MotionLoom VFX Studio page with graph preview and template picker

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use gpui::{
    ClipboardItem, Context, Element, Entity, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, PathPromptOptions, Render, RenderImage,
    ScrollWheelEvent, Style, Subscription, Window, div, prelude::*, px, rgb, rgba,
};
use gpui_component::{
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    white,
};
use image::{ImageBuffer, Rgba};
use motionloom::{
    GraphScope, RuntimeProgram, SceneRenderProfile, SceneRenderProgress, compile_runtime_program,
    is_graph_script, next_scene_output_path_for_profile, parse_graph_script,
    render_scene_graph_frame, render_scene_graph_to_video_with_progress,
};
use smallvec::SmallVec;
use thiserror::Error;
use url::Url;
use video_engine::{Position, Video, VideoOptions};

use crate::core::export::get_media_duration;
use crate::core::global_state::{GlobalState, MediaPoolUiEvent};
use crate::core::thumbnail;
use crate::ui::motionloom_templates;
use crate::ui::motionloom_templates::LayerEffectTemplateKind;

const THUMB_MAX_DIM: u32 = 640;
const SCENE_RENDER_PROGRESS_EVERY_FRAMES: u32 = 10;
const SCENE_RENDER_PROGRESS_POLL_MS: u64 = 120;
const DEFAULT_SCENE_LIVE_NODE_ID: &str = "iris_outer_soft";
const DEFAULT_SCENE_LIVE_ATTR: &str = "x";

fn panic_payload_to_string(payload: Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SceneRenderMode {
    CompatibilityCpu,
    GpuNativeH264,
    GpuNativeProRes,
}

impl SceneRenderMode {
    const fn label(self) -> &'static str {
        match self {
            SceneRenderMode::CompatibilityCpu => "Compatibility Render (CPU)",
            SceneRenderMode::GpuNativeH264 => "GPU Render",
            SceneRenderMode::GpuNativeProRes => "GPU Render (ProRes)",
        }
    }

    const fn profile(self) -> SceneRenderProfile {
        match self {
            SceneRenderMode::CompatibilityCpu => SceneRenderProfile::Cpu,
            SceneRenderMode::GpuNativeH264 => SceneRenderProfile::Gpu,
            SceneRenderMode::GpuNativeProRes => SceneRenderProfile::GpuProRes,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportedClipKind {
    Image,
    Video,
}

impl ImportedClipKind {
    const fn label(self) -> &'static str {
        match self {
            ImportedClipKind::Image => "Image",
            ImportedClipKind::Video => "Video",
        }
    }
}

#[derive(Clone)]
struct LoadedPreview {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
    bgra: Arc<Vec<u8>>,
}

#[derive(Clone)]
struct ImportedClip {
    name: String,
    path: String,
    kind: ImportedClipKind,
    duration: Duration,
    preview: Option<LoadedPreview>,
    error: Option<String>,
}

#[derive(Clone)]
struct VfxAssetItem {
    name: String,
    path: String,
    kind: ImportedClipKind,
    duration: Duration,
    preview: Option<LoadedPreview>,
    error: Option<String>,
}

#[derive(Clone)]
struct VfxAssetContextMenu {
    path: String,
    x: f32,
    y: f32,
}

#[derive(Clone, Debug)]
struct SceneLiveTarget {
    id: String,
    tag: String,
    attrs: Vec<String>,
}

#[derive(Debug, Error)]
enum MotionLoomPageError {
    #[error("Failed to open preview image: {source}")]
    OpenPreviewImage { source: image::ImageError },
    #[error("Failed to construct preview image buffer")]
    BuildPreviewImageBuffer,
    #[error("Failed to construct runtime preview buffer")]
    BuildRuntimePreviewBuffer,
    #[error("Failed to convert path to URL: {path}")]
    PathToUrl { path: PathBuf },
    #[error("Failed to open video preview player: {message}")]
    OpenVideoPreviewPlayer { message: String },
    #[error(transparent)]
    Thumbnail(#[from] crate::core::thumbnail::ThumbnailError),
}

// Fit-to-container preview image element that renders a source image
// centered inside the available bounds with aspect-ratio preservation.
struct FitPreviewImageElement {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
}

impl FitPreviewImageElement {
    fn new(image: Arc<RenderImage>, width: u32, height: u32) -> Self {
        Self {
            image,
            width,
            height,
        }
    }

    // Calculate destination bounds that fit the image into the container
    // while preserving the original aspect ratio.
    fn fitted_bounds(&self, bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
        let container_w: f32 = bounds.size.width.into();
        let container_h: f32 = bounds.size.height.into();
        let frame_w = self.width as f32;
        let frame_h = self.height as f32;
        if frame_w == 0.0 || frame_h == 0.0 {
            return bounds;
        }

        let fit_scale = (container_w / frame_w).min(container_h / frame_h);
        let dest_w = frame_w * fit_scale;
        let dest_h = frame_h * fit_scale;
        let offset_x = (container_w - dest_w) * 0.5;
        let offset_y = (container_h - dest_h) * 0.5;

        gpui::Bounds::new(
            gpui::point(
                bounds.origin.x + gpui::px(offset_x),
                bounds.origin.y + gpui::px(offset_y),
            ),
            gpui::size(gpui::px(dest_w), gpui::px(dest_h)),
        )
    }
}

impl Element for FitPreviewImageElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: gpui::Size {
                width: gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0)),
                height: gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0)),
            },
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _state: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut gpui::App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut gpui::App,
    ) {
        let dest_bounds = self.fitted_bounds(bounds);
        let _ = window.paint_image(
            dest_bounds,
            gpui::Corners::default(),
            self.image.clone(),
            0,
            false,
        );
    }
}

impl IntoElement for FitPreviewImageElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct MotionLoomPage {
    pub global: Entity<GlobalState>,
    clips: Vec<ImportedClip>,
    selected_idx: Option<usize>,
    preview_frame: u32,
    status_line: String,
    script_text: String,
    script_input: Option<Entity<InputState>>,
    script_input_sub: Option<Subscription>,
    motionloom_script_revision: u64,
    motionloom_apply_revision: u64,
    motionloom_render_revision: u64,
    graph_runtime: Option<RuntimeProgram>,
    runtime_preview_cache_key: Option<(usize, u32, i32, i32, i32, i32, i32, i32, u32, u32)>,
    runtime_preview_cache_image: Option<Arc<RenderImage>>,
    scene_live_preview_cache_key: Option<(u64, u32)>,
    scene_live_preview_cache_image: Option<(Arc<RenderImage>, u32, u32)>,
    scene_live_knob_node_id: String,
    scene_live_knob_attr: String,
    scene_live_target_offset: usize,
    scene_live_groups_only: bool,
    preview_playing: bool,
    preview_play_token: u64,
    preview_last_tick: Option<Instant>,
    preview_frame_accum: f32,
    video_preview_player: Option<Video>,
    video_preview_player_path: Option<String>,
    video_preview_last_seek_frame: Option<u32>,
    import_modal_open: bool,
    asset_modal_open: bool,
    asset_folder: Option<PathBuf>,
    asset_items: Vec<VfxAssetItem>,
    asset_selected_idx: Option<usize>,
    asset_context_menu: Option<VfxAssetContextMenu>,
    // Template picker state
    template_modal_open: bool,
    template_selected: Vec<LayerEffectTemplateKind>,
    template_add_time_parameter: bool,
    template_add_curve_parameter: bool,
}

impl MotionLoomPage {
    pub fn new(global: Entity<GlobalState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&global, |_this, _global, cx| {
            cx.notify();
        })
        .detach();

        let (mut initial_script, mut script_revision, apply_revision, render_revision) = {
            let gs = global.read(cx);
            (
                gs.motionloom_scene_script().to_string(),
                gs.motionloom_scene_script_revision(),
                gs.motionloom_scene_apply_revision(),
                gs.motionloom_scene_render_revision(),
            )
        };

        if initial_script.trim().is_empty() {
            initial_script = motionloom_templates::DEFAULT_GRAPH_SCRIPT.to_string();
            let (new_script_revision, _new_apply_revision) = global.update(cx, |gs, _cx| {
                gs.set_motionloom_scene_script(initial_script.clone(), false);
                (
                    gs.motionloom_scene_script_revision(),
                    gs.motionloom_scene_apply_revision(),
                )
            });
            script_revision = new_script_revision;
        }

        Self {
            global,
            clips: Vec::new(),
            selected_idx: None,
            preview_frame: 0,
            status_line: "Import a video or still to start building a MotionLoom graph."
                .to_string(),
            script_text: initial_script,
            script_input: None,
            script_input_sub: None,
            motionloom_script_revision: script_revision,
            motionloom_apply_revision: apply_revision,
            motionloom_render_revision: render_revision,
            graph_runtime: None,
            runtime_preview_cache_key: None,
            runtime_preview_cache_image: None,
            scene_live_preview_cache_key: None,
            scene_live_preview_cache_image: None,
            scene_live_knob_node_id: DEFAULT_SCENE_LIVE_NODE_ID.to_string(),
            scene_live_knob_attr: DEFAULT_SCENE_LIVE_ATTR.to_string(),
            scene_live_target_offset: 0,
            scene_live_groups_only: false,
            preview_playing: false,
            preview_play_token: 0,
            preview_last_tick: None,
            preview_frame_accum: 0.0,
            video_preview_player: None,
            video_preview_player_path: None,
            video_preview_last_seek_frame: None,
            import_modal_open: false,
            asset_modal_open: false,
            asset_folder: None,
            asset_items: Vec::new(),
            asset_selected_idx: None,
            asset_context_menu: None,
            template_modal_open: false,
            template_selected: Vec::new(),
            template_add_time_parameter: false,
            template_add_curve_parameter: false,
        }
    }

    fn is_image_path(path: &str) -> bool {
        let p = path.to_ascii_lowercase();
        p.ends_with(".jpg")
            || p.ends_with(".jpeg")
            || p.ends_with(".png")
            || p.ends_with(".webp")
            || p.ends_with(".bmp")
            || p.ends_with(".gif")
    }

    fn is_video_path(path: &str) -> bool {
        let p = path.to_ascii_lowercase();
        p.ends_with(".mp4")
            || p.ends_with(".mov")
            || p.ends_with(".mkv")
            || p.ends_with(".webm")
            || p.ends_with(".avi")
            || p.ends_with(".flv")
            || p.ends_with(".m4v")
    }

    fn is_supported_clip_path(path: &str) -> bool {
        Self::is_image_path(path) || Self::is_video_path(path)
    }

    fn load_render_image(path: &Path) -> Result<LoadedPreview, MotionLoomPageError> {
        let decoded =
            image::open(path).map_err(|source| MotionLoomPageError::OpenPreviewImage { source })?;
        Self::load_render_image_from_dynamic(decoded)
    }

    fn load_render_image_from_dynamic(
        decoded: image::DynamicImage,
    ) -> Result<LoadedPreview, MotionLoomPageError> {
        let rgba = decoded.to_rgba8();
        let (w, h) = rgba.dimensions();
        let mut bgra = rgba.into_raw();
        for px in bgra.chunks_mut(4) {
            let r = px[0];
            let b = px[2];
            px[0] = b;
            px[2] = r;
        }
        let source_bgra = Arc::new(bgra.clone());
        let image_buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(w, h, bgra)
            .ok_or(MotionLoomPageError::BuildPreviewImageBuffer)?;
        let frames = SmallVec::from_elem(image::Frame::new(image_buffer), 1);
        Ok(LoadedPreview {
            image: Arc::new(RenderImage::new(frames)),
            width: w,
            height: h,
            bgra: source_bgra,
        })
    }

    fn render_image_from_bgra(
        width: u32,
        height: u32,
        bgra: Vec<u8>,
    ) -> Result<Arc<RenderImage>, MotionLoomPageError> {
        let image_buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, bgra)
            .ok_or(MotionLoomPageError::BuildRuntimePreviewBuffer)?;
        let frames = SmallVec::from_elem(image::Frame::new(image_buffer), 1);
        Ok(Arc::new(RenderImage::new(frames)))
    }

    fn script_hash(script: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        script.hash(&mut hasher);
        hasher.finish()
    }

    fn scene_live_preview_image(
        &mut self,
        frame: u32,
    ) -> Result<Option<(Arc<RenderImage>, u32, u32)>, String> {
        let raw = self.script_text.clone();
        if raw.trim().is_empty() {
            return Ok(None);
        }
        if !is_graph_script(&raw) {
            return Err(
                "Scene live preview requires a <Graph ...> MotionLoom DSL block.".to_string(),
            );
        }

        let script_hash = Self::script_hash(&raw);
        let key = (script_hash, frame);
        if self.scene_live_preview_cache_key == Some(key)
            && let Some((image, w, h)) = self.scene_live_preview_cache_image.as_ref()
        {
            return Ok(Some((image.clone(), *w, *h)));
        }

        let graph = parse_graph_script(&raw).map_err(|err| {
            format!(
                "Scene live parse error at line {}: {}",
                err.line, err.message
            )
        })?;
        if graph.scope != GraphScope::Scene {
            return Err("Scene live preview requires <Graph scope=\"scene\" ...>.".to_string());
        }
        if !graph.has_scene_nodes() {
            return Err("Scene live preview needs at least one scene node.".to_string());
        }

        let rgba = render_scene_graph_frame(&graph, frame, SceneRenderProfile::Cpu)
            .map_err(|err| format!("Scene live render error: {err}"))?;
        let (w, h) = rgba.dimensions();
        let mut bgra = rgba.into_raw();
        for px in bgra.chunks_mut(4) {
            px.swap(0, 2);
        }
        let image = Self::render_image_from_bgra(w, h, bgra).map_err(|err| err.to_string())?;
        self.scene_live_preview_cache_key = Some(key);
        self.scene_live_preview_cache_image = Some((image.clone(), w, h));
        Ok(Some((image, w, h)))
    }

    fn format_live_number(value: f32) -> String {
        let mut text = format!("{value:.2}");
        while text.contains('.') && text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
        if text == "-0" { "0".to_string() } else { text }
    }

    fn parse_live_number(raw: &str) -> Option<f32> {
        let mut text = raw.trim();
        if let Some(inner) = text.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
            text = inner.trim();
        }
        text.parse::<f32>().ok()
    }

    fn is_attr_ident_byte(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' || byte == b':'
    }

    fn find_scene_tag_range_by_id(script: &str, node_id: &str) -> Option<(usize, usize)> {
        let id_patterns = [
            format!("id=\"{node_id}\""),
            format!("id='{node_id}'"),
            format!("id={node_id}"),
        ];
        let id_pos = id_patterns
            .iter()
            .filter_map(|pattern| script.find(pattern))
            .min()?;
        let tag_start = script[..id_pos].rfind('<')?;
        let tag_end = id_pos + script[id_pos..].find('>')? + 1;
        Some((tag_start, tag_end))
    }

    fn find_attr_value_range_in_tag(tag: &str, attr: &str) -> Option<(usize, usize)> {
        let bytes = tag.as_bytes();
        let mut search_from = 0;
        while search_from < tag.len() {
            let rel = tag[search_from..].find(attr)?;
            let attr_start = search_from + rel;
            let attr_end = attr_start + attr.len();
            if attr_start > 0 && Self::is_attr_ident_byte(bytes[attr_start - 1]) {
                search_from = attr_end;
                continue;
            }
            if attr_end < bytes.len() && Self::is_attr_ident_byte(bytes[attr_end]) {
                search_from = attr_end;
                continue;
            }

            let mut i = attr_end;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] != b'=' {
                search_from = attr_end;
                continue;
            }
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                return None;
            }
            if bytes[i] == b'"' || bytes[i] == b'\'' {
                let quote = bytes[i];
                let value_start = i + 1;
                let value_end = tag[value_start..]
                    .bytes()
                    .position(|byte| byte == quote)
                    .map(|pos| value_start + pos)?;
                return Some((value_start, value_end));
            }

            let value_start = i;
            while i < bytes.len()
                && !bytes[i].is_ascii_whitespace()
                && bytes[i] != b'>'
                && bytes[i] != b'/'
            {
                i += 1;
            }
            return Some((value_start, i));
        }
        None
    }

    fn tag_name(tag: &str) -> Option<String> {
        let tag = tag.trim_start();
        let body = tag.strip_prefix('<')?.trim_start();
        if body.starts_with('/') || body.starts_with('!') || body.starts_with('?') {
            return None;
        }
        let end = body
            .find(|ch: char| ch.is_ascii_whitespace() || ch == '/' || ch == '>')
            .unwrap_or(body.len());
        if end == 0 {
            None
        } else {
            Some(body[..end].to_string())
        }
    }

    fn is_scene_live_target_tag(tag: &str) -> bool {
        matches!(
            tag,
            "Character"
                | "Group"
                | "Camera"
                | "Mask"
                | "Circle"
                | "Rect"
                | "Path"
                | "FaceJaw"
                | "Line"
                | "Polyline"
                | "Text"
                | "Image"
                | "Svg"
        )
    }

    fn push_scene_live_attr(attrs: &mut Vec<String>, attr: &str) {
        if !attrs.iter().any(|existing| existing == attr) {
            attrs.push(attr.to_string());
        }
    }

    fn scene_live_attrs_for_tag(tag_name: &str, tag: &str) -> Vec<String> {
        let mut attrs = Vec::new();
        match tag_name {
            "Character" | "Group" | "Camera" | "Mask" => {
                for attr in ["x", "y", "rotation", "scale", "opacity"] {
                    Self::push_scene_live_attr(&mut attrs, attr);
                }
            }
            "Circle" => {
                for attr in ["x", "y", "radius", "opacity", "strokeWidth"] {
                    Self::push_scene_live_attr(&mut attrs, attr);
                }
            }
            "Rect" | "Image" | "Svg" | "Text" => {
                for attr in ["x", "y", "width", "height", "rotation", "scale", "opacity"] {
                    Self::push_scene_live_attr(&mut attrs, attr);
                }
            }
            "Line" => {
                for attr in ["x1", "y1", "x2", "y2", "strokeWidth", "opacity"] {
                    Self::push_scene_live_attr(&mut attrs, attr);
                }
            }
            "Path" | "Polyline" => {
                for attr in ["strokeWidth", "opacity", "feather"] {
                    Self::push_scene_live_attr(&mut attrs, attr);
                }
            }
            "FaceJaw" => {
                for attr in [
                    "x",
                    "y",
                    "width",
                    "height",
                    "cheekWidth",
                    "chinWidth",
                    "chinSharpness",
                    "jawEase",
                    "scale",
                    "strokeWidth",
                    "opacity",
                ] {
                    Self::push_scene_live_attr(&mut attrs, attr);
                }
            }
            _ => {}
        }

        for attr in [
            "x",
            "y",
            "x1",
            "y1",
            "x2",
            "y2",
            "radius",
            "width",
            "height",
            "cheekWidth",
            "chinWidth",
            "chinSharpness",
            "jawEase",
            "rotation",
            "scale",
            "opacity",
            "strokeWidth",
            "feather",
        ] {
            if Self::find_attr_value_range_in_tag(tag, attr).is_some() {
                Self::push_scene_live_attr(&mut attrs, attr);
            }
        }
        attrs
    }

    fn extract_scene_live_targets(script: &str) -> Vec<SceneLiveTarget> {
        let mut out = Vec::new();
        let mut search_from = 0;
        while let Some(rel_start) = script[search_from..].find('<') {
            let tag_start = search_from + rel_start;
            let Some(rel_end) = script[tag_start..].find('>') else {
                break;
            };
            let tag_end = tag_start + rel_end + 1;
            let tag = &script[tag_start..tag_end];
            if let Some(tag_name) = Self::tag_name(tag)
                && Self::is_scene_live_target_tag(&tag_name)
                && let Some((id_start, id_end)) = Self::find_attr_value_range_in_tag(tag, "id")
            {
                let id = tag[id_start..id_end].trim().to_string();
                if !id.is_empty() && !out.iter().any(|target: &SceneLiveTarget| target.id == id) {
                    let attrs = Self::scene_live_attrs_for_tag(&tag_name, tag);
                    if !attrs.is_empty() {
                        out.push(SceneLiveTarget {
                            id,
                            tag: tag_name,
                            attrs,
                        });
                    }
                }
            }
            search_from = tag_end;
        }
        out
    }

    fn default_scene_live_attr(attrs: &[String]) -> String {
        for preferred in [
            "x",
            "y",
            "radius",
            "rotation",
            "scale",
            "opacity",
            "strokeWidth",
        ] {
            if attrs.iter().any(|attr| attr == preferred) {
                return preferred.to_string();
            }
        }
        attrs
            .first()
            .cloned()
            .unwrap_or_else(|| DEFAULT_SCENE_LIVE_ATTR.to_string())
    }

    fn overall_scene_live_target<'a>(
        targets: &'a [SceneLiveTarget],
    ) -> Option<&'a SceneLiveTarget> {
        targets
            .iter()
            .find(|target| target.tag == "Character" || target.tag == "Group")
            .or_else(|| {
                targets
                    .iter()
                    .find(|target| target.tag == "Camera" || target.tag == "Mask")
            })
            .or_else(|| targets.first())
    }

    fn ensure_scene_live_selection(&mut self, targets: &[SceneLiveTarget]) {
        let Some(mut target) = targets
            .iter()
            .find(|target| target.id == self.scene_live_knob_node_id)
        else {
            if let Some(next) = Self::overall_scene_live_target(targets) {
                self.scene_live_knob_node_id = next.id.clone();
                self.scene_live_knob_attr = Self::default_scene_live_attr(&next.attrs);
            }
            return;
        };

        if !target
            .attrs
            .iter()
            .any(|attr| attr == &self.scene_live_knob_attr)
        {
            self.scene_live_knob_attr = Self::default_scene_live_attr(&target.attrs);
            target = targets
                .iter()
                .find(|target| target.id == self.scene_live_knob_node_id)
                .unwrap_or(target);
        }

        if target.attrs.is_empty() {
            self.scene_live_knob_attr = DEFAULT_SCENE_LIVE_ATTR.to_string();
        }
    }

    fn select_scene_live_target(&mut self, id: String, attrs: Vec<String>) {
        self.scene_live_knob_node_id = id;
        if !attrs.iter().any(|attr| attr == &self.scene_live_knob_attr) {
            self.scene_live_knob_attr = Self::default_scene_live_attr(&attrs);
        }
        self.status_line = format!(
            "Live target selected: {}.{}.",
            self.scene_live_knob_node_id, self.scene_live_knob_attr
        );
    }

    fn select_scene_live_attr(&mut self, attr: String) {
        self.scene_live_knob_attr = attr;
        self.status_line = format!(
            "Live attribute selected: {}.{}.",
            self.scene_live_knob_node_id, self.scene_live_knob_attr
        );
    }

    fn find_scene_tag_attr_number(script: &str, node_id: &str, attr: &str) -> Option<f32> {
        let (tag_start, tag_end) = Self::find_scene_tag_range_by_id(script, node_id)?;
        let tag = &script[tag_start..tag_end];
        let (value_start, value_end) = Self::find_attr_value_range_in_tag(tag, attr)?;
        Self::parse_live_number(&tag[value_start..value_end])
    }

    fn patch_scene_tag_attr_number(
        script: &str,
        node_id: &str,
        attr: &str,
        value: f32,
    ) -> Result<String, String> {
        let (tag_start, tag_end) = Self::find_scene_tag_range_by_id(script, node_id)
            .ok_or_else(|| format!("Live knob target id=\"{node_id}\" was not found."))?;
        let tag = &script[tag_start..tag_end];
        let value_text = Self::format_live_number(value);

        if let Some((value_start, value_end)) = Self::find_attr_value_range_in_tag(tag, attr) {
            let abs_start = tag_start + value_start;
            let abs_end = tag_start + value_end;
            let mut out = String::with_capacity(script.len() + value_text.len());
            out.push_str(&script[..abs_start]);
            out.push_str(&value_text);
            out.push_str(&script[abs_end..]);
            return Ok(out);
        }

        let insert_at = tag
            .rfind("/>")
            .map(|rel| tag_start + rel)
            .unwrap_or(tag_end.saturating_sub(1));
        let mut out = String::with_capacity(script.len() + attr.len() + value_text.len() + 4);
        out.push_str(&script[..insert_at]);
        out.push(' ');
        out.push_str(attr);
        out.push_str("=\"");
        out.push_str(&value_text);
        out.push('"');
        out.push_str(&script[insert_at..]);
        Ok(out)
    }

    fn scene_live_knob_current_value(&self) -> Option<f32> {
        Self::find_scene_tag_attr_number(
            &self.script_text,
            &self.scene_live_knob_node_id,
            &self.scene_live_knob_attr,
        )
    }

    fn scene_live_attr_default_value(attr: &str) -> f32 {
        match attr {
            "scale" | "opacity" => 1.0,
            _ => 0.0,
        }
    }

    fn clamp_scene_live_attr_value(attr: &str, value: f32) -> f32 {
        match attr {
            "scale" => value.clamp(0.01, 20.0),
            "opacity" => value.clamp(0.0, 1.0),
            "chinSharpness" | "jawEase" => value.clamp(0.0, 1.0),
            "radius" | "width" | "height" | "cheekWidth" | "chinWidth" | "strokeWidth"
            | "feather" => value.max(0.0),
            _ => value,
        }
    }

    fn scene_live_attr_base_step(attr: &str) -> f32 {
        match attr {
            "scale" | "opacity" => 0.05,
            "chinSharpness" | "jawEase" => 0.02,
            _ => 1.0,
        }
    }

    fn nudge_scene_live_knob(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        let current = self
            .scene_live_knob_current_value()
            .unwrap_or_else(|| Self::scene_live_attr_default_value(&self.scene_live_knob_attr));
        let next = Self::clamp_scene_live_attr_value(&self.scene_live_knob_attr, current + delta);
        match Self::patch_scene_tag_attr_number(
            &self.script_text,
            &self.scene_live_knob_node_id,
            &self.scene_live_knob_attr,
            next,
        ) {
            Ok(script) => {
                self.set_script_text(script, window, cx);
                self.scene_live_preview_cache_key = None;
                self.scene_live_preview_cache_image = None;
                self.status_line = format!(
                    "Updated selected group attr: {}.{} = {}.",
                    self.scene_live_knob_node_id,
                    self.scene_live_knob_attr,
                    Self::format_live_number(next)
                );
            }
            Err(message) => {
                self.status_line = message;
            }
        }
    }

    fn scene_live_scroll_delta(evt: &ScrollWheelEvent, attr: &str) -> f32 {
        let delta_y = evt.delta.pixel_delta(px(10.0)).y / px(1.0);
        if delta_y.abs() <= f32::EPSILON {
            return 0.0;
        }
        let base_step = Self::scene_live_attr_base_step(attr);
        let step = if evt.modifiers.shift {
            base_step * 0.1
        } else if evt.modifiers.alt {
            base_step * 10.0
        } else {
            base_step
        };
        if delta_y < 0.0 { step } else { -step }
    }

    fn scene_live_step_labels(attr: &str) -> (f32, f32, String, String, String, String) {
        let small = Self::scene_live_attr_base_step(attr);
        let large = small * 10.0;
        (
            small,
            large,
            format!("-{}", Self::format_live_number(large)),
            format!("-{}", Self::format_live_number(small)),
            format!("+{}", Self::format_live_number(small)),
            format!("+{}", Self::format_live_number(large)),
        )
    }

    fn scene_live_scroll_hint(attr: &str) -> String {
        let base_step = Self::scene_live_attr_base_step(attr);
        format!(
            "Scroll the selected value: normal = {}, Shift = {}, Option/Alt = {}. This edits the DSL immediately.",
            Self::format_live_number(base_step),
            Self::format_live_number(base_step * 0.1),
            Self::format_live_number(base_step * 10.0)
        )
    }

    // Nearest-neighbor resize for CPU preview rendering.
    fn resize_bgra_nearest(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
        if src_w == dst_w && src_h == dst_h {
            return src.to_vec();
        }
        if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
            return src.to_vec();
        }

        let mut dst = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
        for y in 0..dst_h {
            let sy = ((y as u64 * src_h as u64) / dst_h as u64) as u32;
            for x in 0..dst_w {
                let sx = ((x as u64 * src_w as u64) / dst_w as u64) as u32;
                let src_ix = ((sy as usize) * (src_w as usize) + (sx as usize)) * 4;
                let dst_ix = ((y as usize) * (dst_w as usize) + (x as usize)) * 4;
                dst[dst_ix..dst_ix + 4].copy_from_slice(&src[src_ix..src_ix + 4]);
            }
        }
        dst
    }

    // Resolve the target canvas size from the graph runtime or fall back to None.
    fn runtime_target_size(&self) -> Option<(u32, u32)> {
        let runtime = self.graph_runtime.as_ref()?;
        let graph = runtime.graph();
        if let Some(size) = graph.resource_size(&graph.present.from) {
            return Some(size);
        }
        Some(graph.size)
    }

    fn playback_fps(&self) -> f32 {
        self.graph_runtime
            .as_ref()
            .map(|runtime| runtime.graph().fps)
            .filter(|fps| fps.is_finite() && *fps > 0.0)
            .unwrap_or(30.0)
    }

    fn ensure_video_preview_player(&mut self, path: &str) -> Result<(), MotionLoomPageError> {
        if self.video_preview_player.is_some()
            && self.video_preview_player_path.as_deref() == Some(path)
        {
            return Ok(());
        }

        self.video_preview_player = None;
        self.video_preview_player_path = None;
        self.video_preview_last_seek_frame = None;

        let pb = PathBuf::from(path);
        let url = Url::from_file_path(&pb)
            .map_err(|_| MotionLoomPageError::PathToUrl { path: pb.clone() })?;
        let fps = self.playback_fps().round().clamp(1.0, 240.0) as u32;
        let opts = VideoOptions {
            frame_buffer_capacity: Some(2),
            preview_scale: None,
            preview_max_dim: None,
            preview_fps: Some(fps),
            appsink_max_buffers: Some(2),
            #[cfg(target_os = "macos")]
            // VFX Stage favors decode compatibility over NV12 surface fast-path.
            // Scene renders may be ProRes/yuv422p10le and can fail to produce stable
            // surface frames in this panel if NV12 is forced too early.
            prefer_surface: false,
            #[cfg(target_os = "macos")]
            strict_surface_proxy_nv12: false,
            benchmark_raw_appsink: VideoOptions::benchmark_raw_appsink_from_env(),
            ..Default::default()
        };
        let player = Video::new_with_options(&url, opts).map_err(|err| {
            MotionLoomPageError::OpenVideoPreviewPlayer {
                message: err.to_string(),
            }
        })?;
        player.set_muted(true);
        player.set_paused(true);
        let _ = player.seek(Position::Time(Duration::ZERO), false);
        self.video_preview_player = Some(player);
        self.video_preview_player_path = Some(path.to_string());
        Ok(())
    }

    fn seek_video_preview_frame(&mut self, path: &str, frame: u32) -> bool {
        if self.ensure_video_preview_player(path).is_err() {
            return false;
        }
        let fps = self.playback_fps().max(1.0);
        if self.video_preview_last_seek_frame != Some(frame) {
            let seek_t = Duration::from_secs_f64(frame as f64 / fps as f64);
            if let Some(player) = self.video_preview_player.as_ref() {
                let _ = player.seek(Position::Time(seek_t), false);
            }
            self.video_preview_last_seek_frame = Some(frame);
        }
        true
    }

    fn video_preview_frame_bgra(&mut self, path: &str, frame: u32) -> Option<(Vec<u8>, u32, u32)> {
        if !self.seek_video_preview_frame(path, frame) {
            return None;
        }
        self.video_preview_player
            .as_ref()
            .and_then(|player| player.current_frame_data())
    }

    // Calculate total frame count from graph runtime duration or clip duration.
    fn playback_frame_count(&self) -> u32 {
        if let Some(runtime) = self.graph_runtime.as_ref() {
            let graph = runtime.graph();
            let total = ((graph.duration_ms as f64 / 1000.0) * graph.fps as f64).round() as u32;
            if total > 1 {
                return total;
            }
        }

        if let Some(clip) = self.current_clip() {
            let secs = clip.duration.as_secs_f64();
            if secs > 0.0 {
                let fps = self.playback_fps().max(1.0) as f64;
                let total = (secs * fps).round() as u32;
                if total > 1 {
                    return total;
                }
            }
        }

        // Default minimum frame count for still images
        60
    }

    // Schedule the next frame tick for continuous playback preview.
    fn schedule_preview_playback(
        &mut self,
        token: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.on_next_frame(window, move |this, window, cx| {
            if !this.preview_playing || this.preview_play_token != token {
                return;
            }

            let now = Instant::now();
            let dt = now.saturating_duration_since(this.preview_last_tick.unwrap_or(now));
            this.preview_last_tick = Some(now);

            let fps = this.playback_fps();
            this.preview_frame_accum += dt.as_secs_f32() * fps;
            let step = this.preview_frame_accum.floor() as u32;
            if step > 0 {
                this.preview_frame_accum -= step as f32;
                let frame_count = this.playback_frame_count();
                let stop_on_end = this
                    .graph_runtime
                    .as_ref()
                    .map(|runtime| runtime.graph().scope == GraphScope::Scene)
                    .unwrap_or(false);
                if stop_on_end {
                    let last_frame = frame_count.saturating_sub(1);
                    let next = this.preview_frame.saturating_add(step);
                    if next >= frame_count {
                        this.preview_frame = last_frame;
                        this.preview_playing = false;
                        this.preview_last_tick = None;
                        this.preview_frame_accum = 0.0;
                        this.status_line =
                            format!("Scene preview reached end at frame {}.", this.preview_frame);
                    } else {
                        this.preview_frame = next;
                    }
                } else {
                    this.preview_frame = (this.preview_frame + step) % frame_count;
                }
                cx.notify();
            }

            if this.preview_playing && this.preview_play_token == token {
                this.schedule_preview_playback(token, window, cx);
            }
        });
    }

    fn step_preview_frame(&mut self, delta: i32) {
        self.preview_playing = false;
        self.preview_last_tick = None;
        self.preview_frame_accum = 0.0;
        if delta >= 0 {
            self.preview_frame = self.preview_frame.saturating_add(delta as u32);
        } else {
            self.preview_frame = self.preview_frame.saturating_sub(delta.unsigned_abs());
        }
    }

    fn toggle_preview_playback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preview_playing {
            self.preview_playing = false;
            self.preview_last_tick = None;
            self.preview_frame_accum = 0.0;
            self.status_line = format!("Paused at frame {}.", self.preview_frame);
            cx.notify();
            return;
        }
        if self.current_clip().is_none() {
            self.status_line = "Import/select a clip before playback.".to_string();
            cx.notify();
            return;
        }

        self.preview_playing = true;
        self.preview_play_token = self.preview_play_token.wrapping_add(1);
        self.preview_last_tick = Some(Instant::now());
        self.preview_frame_accum = 0.0;
        let token = self.preview_play_token;
        self.status_line = format!("Playback started at {} fps.", self.playback_fps());
        self.schedule_preview_playback(token, window, cx);
        cx.notify();
    }

    // CPU-side preview rendering with color/blur/opacity effects from the graph runtime.
    fn runtime_preview_image(
        &mut self,
        clip_idx: usize,
        source_bgra: &[u8],
        source_w: u32,
        source_h: u32,
        fallback_image: Arc<RenderImage>,
        frame: u32,
        invert_mix: f32,
        brightness: f32,
        contrast: f32,
        saturation: f32,
        blur_sigma: f32,
        opacity: f32,
        target_size: (u32, u32),
    ) -> (Arc<RenderImage>, u32, u32) {
        let mix = invert_mix.clamp(0.0, 1.0);
        let brightness = brightness.clamp(-1.0, 1.0);
        let contrast = contrast.clamp(0.0, 2.0);
        let saturation = saturation.clamp(0.0, 2.0);
        let blur_sigma = blur_sigma.clamp(-64.0, 64.0);
        let opacity = opacity.clamp(0.0, 1.0);
        let target_w = target_size.0.max(1);
        let target_h = target_size.1.max(1);
        let quantized = (mix * 1000.0).round() as i32;
        let bq = (brightness * 1000.0).round() as i32;
        let cq = (contrast * 1000.0).round() as i32;
        let sq = (saturation * 1000.0).round() as i32;
        let blur_q = (blur_sigma * 1000.0).round() as i32;
        let oq = (opacity * 1000.0).round() as i32;
        let key = (
            clip_idx, frame, quantized, bq, cq, sq, blur_q, oq, target_w, target_h,
        );
        if self.runtime_preview_cache_key == Some(key)
            && let Some(image) = self.runtime_preview_cache_image.as_ref()
        {
            return (image.clone(), target_w, target_h);
        }

        let mut bgra =
            Self::resize_bgra_nearest(source_bgra, source_w, source_h, target_w, target_h);

        // Apply color grading effects (invert, brightness, contrast, saturation, opacity)
        if mix > 0.0001
            || brightness.abs() > 0.0001
            || (contrast - 1.0).abs() > 0.0001
            || (saturation - 1.0).abs() > 0.0001
            || (opacity - 1.0).abs() > 0.0001
        {
            for px in bgra.chunks_mut(4) {
                let sb = px[0] as f32;
                let sg = px[1] as f32;
                let sr = px[2] as f32;

                let mut r = sr / 255.0;
                let mut g = sg / 255.0;
                let mut b = sb / 255.0;

                if mix > 0.0001 {
                    r = r * (1.0 - mix) + (1.0 - r) * mix;
                    g = g * (1.0 - mix) + (1.0 - g) * mix;
                    b = b * (1.0 - mix) + (1.0 - b) * mix;
                }

                r = ((r + brightness) - 0.5) * contrast + 0.5;
                g = ((g + brightness) - 0.5) * contrast + 0.5;
                b = ((b + brightness) - 0.5) * contrast + 0.5;

                let luma = 0.299 * r + 0.587 * g + 0.114 * b;
                r = luma + (r - luma) * saturation;
                g = luma + (g - luma) * saturation;
                b = luma + (b - luma) * saturation;

                let mut nr = r.clamp(0.0, 1.0);
                let mut ng = g.clamp(0.0, 1.0);
                let mut nb = b.clamp(0.0, 1.0);
                if opacity < 0.9999 {
                    let srn = sr / 255.0;
                    let sgn = sg / 255.0;
                    let sbn = sb / 255.0;
                    nr = srn * (1.0 - opacity) + nr * opacity;
                    ng = sgn * (1.0 - opacity) + ng * opacity;
                    nb = sbn * (1.0 - opacity) + nb * opacity;
                }

                px[2] = (nr * 255.0).round().clamp(0.0, 255.0) as u8;
                px[1] = (ng * 255.0).round().clamp(0.0, 255.0) as u8;
                px[0] = (nb * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }

        // Apply gaussian blur or unsharp-mask sharpening
        if blur_sigma > 0.05 {
            let mut rgba = Vec::with_capacity(bgra.len());
            for px in bgra.chunks_exact(4) {
                rgba.push(px[2]);
                rgba.push(px[1]);
                rgba.push(px[0]);
                rgba.push(px[3]);
            }
            if let Some(rgba_img) =
                ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(target_w, target_h, rgba)
            {
                let blurred = image::imageops::blur(&rgba_img, blur_sigma);
                let raw = blurred.into_raw();
                for (dst, src) in bgra.chunks_exact_mut(4).zip(raw.chunks_exact(4)) {
                    dst[0] = src[2];
                    dst[1] = src[1];
                    dst[2] = src[0];
                    dst[3] = src[3];
                }
            }
        } else if blur_sigma < -0.05 {
            let sharpen_sigma = blur_sigma.abs();
            let amount = 1.0_f32;
            let base = bgra.clone();
            let mut rgba = Vec::with_capacity(base.len());
            for px in base.chunks_exact(4) {
                rgba.push(px[2]);
                rgba.push(px[1]);
                rgba.push(px[0]);
                rgba.push(px[3]);
            }
            if let Some(rgba_img) =
                ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(target_w, target_h, rgba)
            {
                let blurred = image::imageops::blur(&rgba_img, sharpen_sigma);
                let raw_blur = blurred.into_raw();
                for ((dst, src_base), src_blur) in bgra
                    .chunks_exact_mut(4)
                    .zip(base.chunks_exact(4))
                    .zip(raw_blur.chunks_exact(4))
                {
                    for ch in 0..3 {
                        let b = src_base[ch] as f32 / 255.0;
                        let bl = src_blur[ch] as f32 / 255.0;
                        let v = (b + (b - bl) * amount).clamp(0.0, 1.0);
                        dst[ch] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                    dst[3] = src_base[3];
                }
            }
        }

        let image = Self::render_image_from_bgra(target_w, target_h, bgra)
            .unwrap_or_else(|_| fallback_image.clone());
        self.runtime_preview_cache_key = Some(key);
        self.runtime_preview_cache_image = Some(image.clone());
        (image, target_w, target_h)
    }

    // Build an ImportedClip from a file path, generating a thumbnail if possible.
    fn build_imported_clip(
        path: &str,
        ffmpeg_path: &str,
        cache_root: &Path,
        can_generate_video_thumbnail: bool,
    ) -> ImportedClip {
        let pb = PathBuf::from(path);
        let name = pb
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(path)
            .to_string();
        let duration = if Self::is_video_path(path) {
            get_media_duration(path)
        } else {
            Duration::ZERO
        };

        if Self::is_image_path(path) {
            return match Self::load_render_image(&pb) {
                Ok(preview) => ImportedClip {
                    name,
                    path: path.to_string(),
                    kind: ImportedClipKind::Image,
                    duration,
                    preview: Some(preview),
                    error: None,
                },
                Err(err) => ImportedClip {
                    name,
                    path: path.to_string(),
                    kind: ImportedClipKind::Image,
                    duration,
                    preview: None,
                    error: Some(err.to_string()),
                },
            };
        }

        if !can_generate_video_thumbnail {
            return ImportedClip {
                name,
                path: path.to_string(),
                kind: ImportedClipKind::Video,
                duration,
                preview: None,
                error: Some("FFmpeg is required to generate video preview thumbnails.".to_string()),
            };
        }

        let thumb_path = thumbnail::thumbnail_path_for_in(cache_root, &pb, THUMB_MAX_DIM);
        let preview = thumbnail::run_thumbnail_job(ffmpeg_path, &pb, &thumb_path, THUMB_MAX_DIM)
            .map_err(MotionLoomPageError::from)
            .and_then(|_| Self::load_render_image(&thumb_path));

        match preview {
            Ok(preview) => ImportedClip {
                name,
                path: path.to_string(),
                kind: ImportedClipKind::Video,
                duration,
                preview: Some(preview),
                error: None,
            },
            Err(err) => ImportedClip {
                name,
                path: path.to_string(),
                kind: ImportedClipKind::Video,
                duration,
                preview: None,
                error: Some(err.to_string()),
            },
        }
    }

    fn build_vfx_asset_item(
        path: &str,
        ffmpeg_path: &str,
        cache_root: &Path,
        can_generate_video_thumbnail: bool,
    ) -> VfxAssetItem {
        let pb = PathBuf::from(path);
        let name = pb
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(path)
            .to_string();
        let kind = if Self::is_image_path(path) {
            ImportedClipKind::Image
        } else {
            ImportedClipKind::Video
        };
        let duration = if kind == ImportedClipKind::Video {
            get_media_duration(path)
        } else {
            Duration::ZERO
        };

        if kind == ImportedClipKind::Image {
            return match Self::load_render_image(&pb) {
                Ok(preview) => VfxAssetItem {
                    name,
                    path: path.to_string(),
                    kind,
                    duration,
                    preview: Some(preview),
                    error: None,
                },
                Err(err) => VfxAssetItem {
                    name,
                    path: path.to_string(),
                    kind,
                    duration,
                    preview: None,
                    error: Some(err.to_string()),
                },
            };
        }

        let preview = if can_generate_video_thumbnail {
            let thumb_path = thumbnail::thumbnail_path_for_in(cache_root, &pb, THUMB_MAX_DIM);
            thumbnail::run_thumbnail_job(ffmpeg_path, &pb, &thumb_path, THUMB_MAX_DIM)
                .map_err(MotionLoomPageError::from)
                .and_then(|_| Self::load_render_image(&thumb_path))
                .ok()
        } else {
            None
        };

        VfxAssetItem {
            name,
            path: path.to_string(),
            kind,
            duration,
            preview,
            error: None,
        }
    }

    fn load_vfx_asset_folder(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        let (ffmpeg_path, cache_root, can_generate_video_thumbnail) = {
            let gs = self.global.read(cx);
            (
                gs.ffmpeg_path.clone(),
                gs.cache_root_dir(),
                gs.media_tools_ready_for_preview_gen(),
            )
        };

        let mut paths = match fs::read_dir(&folder) {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| path.is_file())
                .filter(|path| {
                    path.to_str()
                        .map(Self::is_supported_clip_path)
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>(),
            Err(err) => {
                self.status_line = format!("VFX Assets folder error: {err}");
                return;
            }
        };
        paths.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default()
        });

        self.asset_items = paths
            .iter()
            .filter_map(|path| path.to_str())
            .map(|path| {
                Self::build_vfx_asset_item(
                    path,
                    &ffmpeg_path,
                    &cache_root,
                    can_generate_video_thumbnail,
                )
            })
            .collect();
        self.asset_folder = Some(folder.clone());
        self.asset_selected_idx = if self.asset_items.is_empty() {
            None
        } else {
            Some(0)
        };
        self.asset_context_menu = None;
        self.status_line = if self.asset_items.is_empty() {
            format!(
                "VFX Assets loaded 0 item(s) from {}. No supported files found in this folder root.",
                folder.to_string_lossy()
            )
        } else {
            format!(
                "VFX Assets loaded {} item(s) from {}.",
                self.asset_items.len(),
                folder.to_string_lossy()
            )
        };
    }

    fn refresh_vfx_asset_folder(&mut self, cx: &mut Context<Self>) {
        let Some(folder) = self.asset_folder.clone() else {
            self.status_line = "Open a folder before refreshing VFX Assets.".to_string();
            return;
        };
        self.load_vfx_asset_folder(folder, cx);
    }

    fn current_clip(&self) -> Option<&ImportedClip> {
        let idx = self.selected_idx?;
        self.clips.get(idx)
    }

    fn sync_script_to_global(&mut self, cx: &mut Context<Self>, apply_now: bool) {
        let text = self.script_text.clone();
        let (script_revision, apply_revision) = self.global.update(cx, |gs, _cx| {
            gs.set_motionloom_scene_script(text, apply_now);
            (
                gs.motionloom_scene_script_revision(),
                gs.motionloom_scene_apply_revision(),
            )
        });
        self.motionloom_script_revision = script_revision;
        self.motionloom_apply_revision = apply_revision;
    }

    fn sync_script_from_global(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (global_script, script_revision, apply_revision) = {
            let gs = self.global.read(cx);
            (
                gs.motionloom_scene_script().to_string(),
                gs.motionloom_scene_script_revision(),
                gs.motionloom_scene_apply_revision(),
            )
        };

        if script_revision != self.motionloom_script_revision {
            self.motionloom_script_revision = script_revision;
            if global_script != self.script_text {
                self.set_script_text(global_script.clone(), window, cx);
            }
        }

        if apply_revision != self.motionloom_apply_revision {
            self.motionloom_apply_revision = apply_revision;
            if global_script != self.script_text {
                self.set_script_text(global_script, window, cx);
            }
            self.apply_script_command(cx);
        }
    }

    fn parse_scene_render_mode_token(token: &str) -> Option<SceneRenderMode> {
        match token.trim().to_ascii_lowercase().as_str() {
            "gpu" | "gpu_render" | "gpu_h264" | "gpu_native_h264" => {
                Some(SceneRenderMode::GpuNativeH264)
            }
            "gpu_prores" | "gpu-prores" | "prores_gpu" => Some(SceneRenderMode::GpuNativeProRes),
            "compatibility_cpu" | "compatibility-cpu" | "cpu" | "cpu_render" => {
                Some(SceneRenderMode::CompatibilityCpu)
            }
            _ => None,
        }
    }

    fn sync_render_request_from_global(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (render_revision, render_mode) = {
            let gs = self.global.read(cx);
            (
                gs.motionloom_scene_render_revision(),
                gs.motionloom_scene_render_mode().map(ToString::to_string),
            )
        };

        if render_revision == self.motionloom_render_revision {
            return;
        }
        self.motionloom_render_revision = render_revision;

        let Some(mode_token) = render_mode else {
            return;
        };
        let Some(mode) = Self::parse_scene_render_mode_token(&mode_token) else {
            self.status_line = format!(
                "Unsupported MotionLoom render mode token from ACP: {}",
                mode_token
            );
            return;
        };
        self.render_scene_to_media_pool(mode, window, cx);
    }

    fn ensure_script_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.script_input.is_some() {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("tsx")
                .rows(8)
                .line_number(true)
                .soft_wrap(true)
                .placeholder("<Graph ...> MotionLoom DSL script")
        });
        let initial = self.script_text.clone();
        input.update(cx, |this, cx| {
            this.set_value(initial.clone(), window, cx);
        });
        let sub = cx.subscribe(&input, |this, input, ev, cx| match ev {
            InputEvent::Change => {
                this.script_text = input.read(cx).value().to_string();
                this.scene_live_preview_cache_key = None;
                this.scene_live_preview_cache_image = None;
                this.sync_script_to_global(cx, false);
            }
            InputEvent::PressEnter { secondary } => {
                this.script_text = input.read(cx).value().to_string();
                this.scene_live_preview_cache_key = None;
                this.scene_live_preview_cache_image = None;
                this.sync_script_to_global(cx, false);
                if *secondary {
                    this.apply_script_command(cx);
                    cx.notify();
                }
            }
            _ => {}
        });
        self.script_input = Some(input);
        self.script_input_sub = Some(sub);
    }

    // Parse and compile the graph script, activating the runtime for preview.
    fn apply_script_command(&mut self, _cx: &mut Context<Self>) {
        let raw = self.script_text.clone();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            self.status_line =
                "Script is empty. Use the Template Picker or write a <Graph> script.".to_string();
            return;
        }

        // Only accept Graph DSL scripts (XML-based MotionLoom format)
        if !is_graph_script(&raw) {
            self.status_line = "Not a valid Graph script. Use the Template Picker to generate one, or write a <Graph ...> block.".to_string();
            return;
        }

        match parse_graph_script(&raw) {
            Ok(graph) => match compile_runtime_program(graph.clone()) {
                Ok(runtime) => {
                    if !runtime.unsupported_kernels().is_empty() {
                        self.graph_runtime = None;
                        self.status_line = format!(
                            "Unsupported kernel(s): {}",
                            runtime.unsupported_kernels().join(", ")
                        );
                        return;
                    }
                    let graph_summary = graph.summary();
                    let runtime_summary = runtime.summary();
                    self.graph_runtime = Some(runtime);
                    self.preview_frame = 0;
                    self.preview_playing = false;
                    self.preview_last_tick = None;
                    self.preview_frame_accum = 0.0;
                    self.runtime_preview_cache_key = None;
                    self.runtime_preview_cache_image = None;
                    self.scene_live_preview_cache_key = None;
                    self.scene_live_preview_cache_image = None;
                    self.video_preview_last_seek_frame = None;
                    self.status_line =
                        format!("Runtime ACTIVE | {} | {}", graph_summary, runtime_summary);
                }
                Err(err) => {
                    self.graph_runtime = None;
                    self.status_line = format!("Runtime compile error: {}", err.message);
                }
            },
            Err(err) => {
                self.graph_runtime = None;
                self.status_line =
                    format!("Graph parse error at line {}: {}", err.line, err.message);
            }
        }
    }

    fn render_scene_to_media_pool(
        &mut self,
        mode: SceneRenderMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let raw = self.script_text.clone();
        let graph = match parse_graph_script(&raw) {
            Ok(graph) => graph,
            Err(err) => {
                self.status_line = format!(
                    "Scene graph parse error at line {}: {}",
                    err.line, err.message
                );
                return;
            }
        };
        if graph.scope != GraphScope::Scene {
            self.status_line = "Scene render requires <Graph scope=\"scene\" ...>.".to_string();
            return;
        }
        if !graph.has_scene_nodes() {
            self.status_line =
                "Scene graph needs at least one scene node: <Scene>, <Solid>, <Text>, <Image>, <Svg>, <Rect>, <Circle>, <Line>, <Polyline>, <Path>, <Camera>, or <Group>."
                    .to_string();
            return;
        }

        let (ffmpeg_path, output_dir, cache_root, can_generate_video_thumbnail) = {
            let gs = self.global.read(cx);
            if !gs.media_tools_ready_for_preview_gen() {
                self.status_line =
                    "MISSING_FFMPEG: MotionLoom scene render requires FFmpeg.".to_string();
                return;
            }
            (
                gs.ffmpeg_path.clone(),
                gs.generated_media_root_dir().join("motionloom_generated"),
                gs.cache_root_dir(),
                gs.media_tools_ready_for_preview_gen(),
            )
        };
        let profile = mode.profile();
        let output_path = match next_scene_output_path_for_profile(&output_dir, profile) {
            Ok(path) => path,
            Err(err) => {
                self.status_line = format!("Scene output path error: {err}");
                return;
            }
        };
        let duration = Duration::from_millis(graph.duration_ms);
        let global = self.global.clone();
        let ffmpeg_for_render = ffmpeg_path.clone();
        let ffmpeg_for_import = ffmpeg_path.clone();
        let cache_root_for_import = cache_root.clone();

        self.status_line = format!(
            "{} started: {}...",
            mode.label(),
            output_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("motionloom_scene.mov")
        );
        cx.notify();

        enum SceneRenderEvent {
            Progress(SceneRenderProgress),
            Finished(Result<PathBuf, String>),
        }

        let (tx, rx) = mpsc::channel::<SceneRenderEvent>();
        let output_path_for_thread = output_path.clone();
        std::thread::spawn(move || {
            let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let tx_progress = tx.clone();
                render_scene_graph_to_video_with_progress(
                    &ffmpeg_for_render,
                    &graph,
                    &output_path_for_thread,
                    profile,
                    SCENE_RENDER_PROGRESS_EVERY_FRAMES,
                    move |progress| {
                        let _ = tx_progress.send(SceneRenderEvent::Progress(progress));
                    },
                )
                .map(|_| output_path_for_thread.clone())
                .map_err(|err| err.to_string())
            }))
            .unwrap_or_else(|payload| {
                Err(format!(
                    "Scene render worker panicked: {}",
                    panic_payload_to_string(payload)
                ))
            });
            let _ = tx.send(SceneRenderEvent::Finished(render_result));
        });

        cx.spawn_in(window, async move |view, window| {
            loop {
                gpui::Timer::after(Duration::from_millis(SCENE_RENDER_PROGRESS_POLL_MS)).await;

                let mut latest_progress: Option<SceneRenderProgress> = None;
                let mut finished: Option<Result<PathBuf, String>> = None;
                loop {
                    match rx.try_recv() {
                        Ok(SceneRenderEvent::Progress(progress)) => {
                            latest_progress = Some(progress);
                        }
                        Ok(SceneRenderEvent::Finished(result)) => {
                            finished = Some(result);
                            break;
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            finished = Some(Err("Scene render worker disconnected.".to_string()));
                            break;
                        }
                    }
                }

                let has_finished = finished.is_some();
                let _ = view.update_in(window, |this, _window, cx| {
                    if let Some(progress) = latest_progress {
                        let pct = ((progress.rendered_frames as f32 / progress.total_frames as f32)
                            * 100.0)
                            .round()
                            .clamp(0.0, 100.0) as u32;
                        this.status_line = format!(
                            "{}: {}% ({}/{})",
                            mode.label(),
                            pct,
                            progress.rendered_frames,
                            progress.total_frames
                        );
                    }

                    if let Some(result) = finished {
                        match result {
                            Ok(path) => {
                                let path_str = path.to_string_lossy().to_string();
                                global.update(cx, |gs, cx| {
                                    gs.add_media_pool_item(path.clone(), duration);
                                    gs.ui_notice = Some(format!(
                                        "MotionLoom scene added to Media Pool: {path_str}"
                                    ));
                                    cx.emit(MediaPoolUiEvent::StateChanged);
                                    cx.notify();
                                });

                                let clip = Self::build_imported_clip(
                                    &path_str,
                                    &ffmpeg_for_import,
                                    &cache_root_for_import,
                                    can_generate_video_thumbnail,
                                );
                                this.clips.push(clip);
                                this.selected_idx = Some(this.clips.len().saturating_sub(1));
                                this.status_line = format!(
                                    "{} done: {}",
                                    mode.label(),
                                    path.file_name()
                                        .and_then(|name| name.to_str())
                                        .unwrap_or("motionloom_scene.mov")
                                );
                            }
                            Err(err) => {
                                this.status_line = format!("{} failed: {err}", mode.label());
                            }
                        }
                    }
                    cx.notify();
                });

                if has_finished {
                    break;
                }
            }
        })
        .detach();
    }

    // Set the script text and sync into the input widget.
    fn set_script_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.script_text = text.clone();
        self.scene_live_preview_cache_key = None;
        self.scene_live_preview_cache_image = None;
        if let Some(input) = self.script_input.as_ref() {
            input.update(cx, |this, cx| {
                this.set_value(text, window, cx);
            });
        }
        self.sync_script_to_global(cx, false);
    }

    fn control_button(label: &'static str) -> gpui::Div {
        div()
            .h(px(28.0))
            .flex_shrink_0()
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.15))
            .bg(white().opacity(0.06))
            .hover(|s| s.bg(white().opacity(0.1)))
            .cursor_pointer()
            .text_xs()
            .text_color(white().opacity(0.9))
            .flex()
            .items_center()
            .justify_center()
            .child(label)
    }

    fn scene_live_chip(label: String, active: bool) -> gpui::Div {
        let border = if active {
            rgba(0x79c7ffcc)
        } else {
            rgba(0xffffff24)
        };
        let bg = if active {
            rgba(0x1f5c85aa)
        } else {
            rgba(0xffffff0e)
        };
        div()
            .h(px(26.0))
            .flex_shrink_0()
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(bg)
            .hover(|s| s.bg(white().opacity(0.11)))
            .cursor_pointer()
            .text_xs()
            .text_color(white().opacity(if active { 0.96 } else { 0.78 }))
            .flex()
            .items_center()
            .justify_center()
            .child(label)
    }

    fn scene_live_checkbox(label: &'static str, checked: bool) -> gpui::Div {
        let border = if checked {
            rgba(0x79c7ffcc)
        } else {
            rgba(0xffffff28)
        };
        let bg = if checked {
            rgba(0x1f5c85aa)
        } else {
            rgba(0xffffff0c)
        };
        div()
            .h(px(28.0))
            .flex_shrink_0()
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(bg)
            .hover(|s| s.bg(white().opacity(0.11)))
            .cursor_pointer()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(14.0))
                    .h(px(14.0))
                    .rounded_sm()
                    .border_1()
                    .border_color(if checked {
                        rgba(0x9bd8ffdd)
                    } else {
                        rgba(0xffffff45)
                    })
                    .bg(if checked {
                        rgba(0x2b78aacc)
                    } else {
                        rgba(0xffffff08)
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(if checked {
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(white().opacity(0.95))
                            .child("x")
                    } else {
                        div()
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(if checked { 0.95 } else { 0.76 }))
                    .child(label),
            )
    }

    // --- Template picker logic (ported from inspector_panel) ---

    fn template_label(kind: LayerEffectTemplateKind) -> &'static str {
        match kind {
            LayerEffectTemplateKind::BlurGaussian => "Blur Gaussian",
            LayerEffectTemplateKind::Sharpen => "Sharpen",
            LayerEffectTemplateKind::Opacity => "Opacity",
            LayerEffectTemplateKind::Lut => "LUT",
            LayerEffectTemplateKind::HslaOverlay => "HSLA Overlay",
            LayerEffectTemplateKind::TransitionFadeInOut => "Transition Fade In/Out",
        }
    }

    fn toggle_template_selection(&mut self, kind: LayerEffectTemplateKind) {
        if let Some(idx) = self
            .template_selected
            .iter()
            .position(|selected| *selected == kind)
        {
            self.template_selected.remove(idx);
            return;
        }
        self.template_selected.push(kind);
    }

    fn selected_template_summary(&self) -> String {
        if self.template_selected.is_empty() {
            return "No templates selected.".to_string();
        }
        self.template_selected
            .iter()
            .map(|kind| Self::template_label(*kind))
            .collect::<Vec<_>>()
            .join(" -> ")
    }

    // Apply selected templates: generate a new graph script or append to existing.
    fn apply_selected_templates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.template_selected.is_empty() {
            self.status_line = "Choose at least one template before pressing OK.".to_string();
            return;
        }

        let add_time = self.template_add_time_parameter;
        let add_curve = self.template_add_curve_parameter;
        let selected = self.template_selected.clone();
        let existing_script = self.script_text.trim().to_string();
        let selection_label = self.selected_template_summary();

        // Build new chain or append to existing graph
        let result = if existing_script.is_empty() || !is_graph_script(&existing_script) {
            motionloom_templates::build_layer_effect_chain_script(&selected, add_time, add_curve)
        } else {
            motionloom_templates::append_layer_effect_template_chain_script(
                &existing_script,
                &selected,
                add_curve,
            )
        };

        let Some(script) = result else {
            self.status_line =
                "Current script is not a standard chainable layer graph. Clear it before applying a multi-template selection."
                    .to_string();
            return;
        };

        self.set_script_text(script, window, cx);
        self.template_modal_open = false;
        self.template_selected.clear();
        self.status_line = if existing_script.is_empty() || !is_graph_script(&existing_script) {
            if add_time && add_curve {
                format!("Inserted template chain: {selection_label} (+apply graph + curve params).")
            } else if add_time {
                format!("Inserted template chain: {selection_label} (+apply graph, duration 5s).")
            } else if add_curve {
                format!("Inserted template chain: {selection_label} (+curve params).")
            } else {
                format!("Inserted template chain: {selection_label}.")
            }
        } else if add_curve {
            format!("Appended template chain: {selection_label} (+curve params).")
        } else {
            format!("Appended template chain: {selection_label}.")
        };
    }

    fn open_template_modal(&mut self) {
        self.import_modal_open = false;
        self.asset_modal_open = false;
        self.template_modal_open = true;
        self.template_selected.clear();
        self.status_line = "Template picker opened.".to_string();
    }

    fn open_import_modal(&mut self) {
        self.template_modal_open = false;
        self.asset_modal_open = false;
        self.import_modal_open = true;
        self.status_line = "Source/import panel opened.".to_string();
    }

    fn open_asset_modal(&mut self) {
        self.template_modal_open = false;
        self.import_modal_open = false;
        self.asset_modal_open = true;
        self.asset_context_menu = None;
        self.status_line = "VFX Assets browser opened.".to_string();
    }

    fn render_vfx_asset_modal_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let viewport_w = window.viewport_size().width / px(1.0);
        let viewport_h = window.viewport_size().height / px(1.0);
        let modal_w = (viewport_w - 96.0).clamp(760.0, 1180.0);
        let modal_h = (viewport_h - 88.0).clamp(520.0, 820.0);
        let folder_label = self
            .asset_folder
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|| "No folder opened.".to_string());
        let selected_asset = self
            .asset_selected_idx
            .and_then(|idx| self.asset_items.get(idx))
            .cloned();

        let open_folder_button = Self::control_button("Open Folder").on_mouse_down(
            MouseButton::Left,
            cx.listener(|_this, _, win, cx| {
                let rx = cx.prompt_for_paths(PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: Some("Open VFX asset folder".into()),
                });
                cx.spawn_in(win, async move |view, window| {
                    let Ok(result) = rx.await else {
                        return;
                    };
                    let Some(paths) = result.ok().flatten() else {
                        return;
                    };
                    let Some(folder) = paths.into_iter().next() else {
                        return;
                    };
                    let _ = view.update_in(window, |this, _window, cx| {
                        this.load_vfx_asset_folder(folder, cx);
                        cx.notify();
                    });
                })
                .detach();
            }),
        );
        let refresh_button = Self::control_button("Refresh").on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.refresh_vfx_asset_folder(cx);
                cx.notify();
            }),
        );

        let context_menu = self.asset_context_menu.clone().map(|menu| {
            let menu_w = 220.0;
            let menu_h = 40.0;
            let menu_x = menu.x.clamp(8.0, (viewport_w - menu_w - 8.0).max(8.0));
            let menu_y = menu.y.clamp(8.0, (viewport_h - menu_h - 8.0).max(8.0));
            let copy_path = menu.path.clone();
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .right_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.asset_context_menu = None;
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _, _, cx| {
                        this.asset_context_menu = None;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(menu_x))
                        .top(px(menu_y))
                        .w(px(menu_w))
                        .rounded_md()
                        .bg(rgb(0x1f1f23))
                        .border_1()
                        .border_color(white().opacity(0.16))
                        .p_1()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, _, cx| {
                                cx.stop_propagation();
                            }),
                        )
                        .child(
                            div()
                                .h(px(28.0))
                                .rounded_sm()
                                .px_2()
                                .flex()
                                .items_center()
                                .text_sm()
                                .text_color(white().opacity(0.92))
                                .bg(white().opacity(0.04))
                                .hover(|style| style.bg(white().opacity(0.10)))
                                .cursor_pointer()
                                .child("Copy Path")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_path.clone(),
                                        ));
                                        this.asset_context_menu = None;
                                        this.status_line =
                                            format!("Copied VFX asset path: {}", copy_path);
                                        cx.notify();
                                    }),
                                ),
                        ),
                )
                .into_any_element()
        });

        let asset_rows = self.asset_items.iter().enumerate().map(|(idx, item)| {
            let active = self.asset_selected_idx == Some(idx);
            let idx_for_select = idx;
            let item_path_for_menu = item.path.clone();
            let duration_label = if item.duration > Duration::ZERO {
                format!("{:.2}s", item.duration.as_secs_f32())
            } else {
                "Still".to_string()
            };
            let thumb = item
                .preview
                .as_ref()
                .map(|preview| (preview.image.clone(), preview.width, preview.height));
            let mut row = div()
                .rounded_md()
                .border_1()
                .border_color(white().opacity(if active { 0.35 } else { 0.12 }))
                .bg(if active { rgb(0x1f2937) } else { rgb(0x111827) })
                .p_2()
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .hover(|s| s.bg(white().opacity(0.09)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.asset_selected_idx = Some(idx_for_select);
                        this.asset_context_menu = None;
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, evt: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.asset_selected_idx = Some(idx_for_select);
                        this.asset_context_menu = Some(VfxAssetContextMenu {
                            path: item_path_for_menu.clone(),
                            x: evt.position.x / px(1.0),
                            y: evt.position.y / px(1.0),
                        });
                        cx.notify();
                    }),
                );
            row = row.child(
                div()
                    .w(px(72.0))
                    .h(px(44.0))
                    .flex_shrink_0()
                    .rounded_sm()
                    .border_1()
                    .border_color(white().opacity(0.10))
                    .bg(rgb(0x05070c))
                    .overflow_hidden()
                    .when_some(thumb, |el, (image, width, height)| {
                        el.child(FitPreviewImageElement::new(image, width, height))
                    }),
            );
            row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.92))
                            .truncate()
                            .child(item.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.58))
                            .truncate()
                            .child(format!("{} · {}", item.kind.label(), duration_label)),
                    ),
            )
        });

        let preview_panel = if let Some(asset) = selected_asset {
            let preview = asset.preview.as_ref().map(|preview| {
                (
                    preview.image.clone(),
                    preview.width,
                    preview.height,
                    preview.width,
                    preview.height,
                )
            });
            div()
                .w(px(330.0))
                .flex_shrink_0()
                .min_h_0()
                .rounded_md()
                .border_1()
                .border_color(white().opacity(0.12))
                .bg(rgb(0x101722))
                .p_3()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .h(px(210.0))
                        .w_full()
                        .rounded_md()
                        .border_1()
                        .border_color(white().opacity(0.12))
                        .bg(rgb(0x05070c))
                        .overflow_hidden()
                        .when_some(preview, |el, (image, width, height, _, _)| {
                            el.child(FitPreviewImageElement::new(image, width, height))
                        }),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(white().opacity(0.94))
                        .truncate()
                        .child(asset.name),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.62))
                        .child(format!(
                            "{} · {}",
                            asset.kind.label(),
                            if asset.duration > Duration::ZERO {
                                format!("{:.2}s", asset.duration.as_secs_f32())
                            } else {
                                "Still".to_string()
                            }
                        )),
                )
                .child(
                    div().w_full().min_w_0().overflow_x_scrollbar().child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.58))
                            .whitespace_nowrap()
                            .child(asset.path),
                    ),
                )
                .when_some(asset.error, |el, error| {
                    el.child(div().text_xs().text_color(rgba(0xff8a80cc)).child(error))
                })
                .into_any_element()
        } else {
            div()
                .w(px(330.0))
                .flex_shrink_0()
                .min_h_0()
                .rounded_md()
                .border_1()
                .border_color(white().opacity(0.12))
                .bg(rgb(0x101722))
                .p_3()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(white().opacity(0.58))
                .child("Open a folder to browse VFX assets.")
                .into_any_element()
        };

        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .bg(gpui_component::black().opacity(0.55))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.asset_modal_open = false;
                    this.asset_context_menu = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(modal_w))
                    .h(px(modal_h))
                    .rounded_md()
                    .bg(rgb(0x1a202c))
                    .border_1()
                    .border_color(white().opacity(0.16))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.asset_context_menu = None;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(white().opacity(0.9))
                                    .child("VFX ASSETS"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(open_folder_button)
                                    .child(refresh_button)
                                    .child(Self::control_button("Close").on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.asset_modal_open = false;
                                            this.asset_context_menu = None;
                                            cx.notify();
                                        }),
                                    )),
                            ),
                    )
                    .child(
                        div().w_full().min_w_0().overflow_x_scrollbar().child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.58))
                                .whitespace_nowrap()
                                .child(folder_label),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .flex()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .min_h_0()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(white().opacity(0.12))
                                    .bg(rgb(0x111827))
                                    .p_2()
                                    .overflow_y_scrollbar()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .when(self.asset_items.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .h_full()
                                                .w_full()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .text_sm()
                                                .text_color(white().opacity(0.55))
                                                .child("No supported image/video files in this folder."),
                                        )
                                    })
                                    .children(asset_rows),
                            )
                            .child(preview_panel),
                    ),
            )
            .when_some(context_menu, |el, menu| el.child(menu))
            .into_any_element()
    }

    fn render_import_modal_overlay(
        &mut self,
        selected_name: String,
        selected_kind_label: String,
        selected_duration_label: String,
        selected_path_label: String,
        imported_count: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let import_button = Self::control_button("Import Clip").on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_this, _, win, cx| {
                let rx = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: true,
                    prompt: Some("Import clips into MotionLoom".into()),
                });
                cx.spawn_in(win, async move |view, window| {
                    let Ok(result) = rx.await else {
                        return;
                    };
                    let Some(paths) = result.ok().flatten() else {
                        return;
                    };

                    let _ = view.update_in(window, |this, _window, cx| {
                        let (ffmpeg_path, cache_root, can_generate_video_thumbnail) = {
                            let gs = this.global.read(cx);
                            (
                                gs.ffmpeg_path.clone(),
                                gs.cache_root_dir(),
                                gs.media_tools_ready_for_preview_gen(),
                            )
                        };

                        let mut imported = 0usize;
                        for path in paths {
                            let path_str = path.to_string_lossy().to_string();
                            if !Self::is_supported_clip_path(&path_str) {
                                continue;
                            }
                            if this.clips.iter().any(|item| item.path == path_str) {
                                continue;
                            }
                            let clip = Self::build_imported_clip(
                                &path_str,
                                &ffmpeg_path,
                                &cache_root,
                                can_generate_video_thumbnail,
                            );
                            this.clips.push(clip);
                            imported += 1;
                        }

                        if imported > 0 {
                            this.selected_idx = Some(this.clips.len().saturating_sub(1));
                            this.status_line =
                                format!("Imported {} clip(s) into MotionLoom Studio.", imported);
                        } else {
                            this.status_line =
                                "No new supported image/video clip was imported.".to_string();
                        }
                        cx.notify();
                    });
                })
                .detach();
            }),
        );

        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .bg(gpui_component::black().opacity(0.55))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.import_modal_open = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(900.0))
                    .h(px(580.0))
                    .rounded_md()
                    .bg(rgb(0x1a202c))
                    .border_1()
                    .border_color(white().opacity(0.16))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(white().opacity(0.9))
                                    .child("SOURCE / IMPORT PANEL"),
                            )
                            .child(Self::control_button("Close").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.import_modal_open = false;
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.65))
                            .child("Import clips, then click a clip to use it in VFX Stage."),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(import_button)
                            .child(
                                div()
                                    .h(px(28.0))
                                    .px_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(white().opacity(0.14))
                                    .bg(white().opacity(0.05))
                                    .text_xs()
                                    .text_color(white().opacity(0.78))
                                    .flex()
                                    .items_center()
                                    .child(format!("{imported_count} imported")),
                            ),
                    )
                    .child(
                        div()
                            .rounded_md()
                            .border_1()
                            .border_color(white().opacity(0.1))
                            .bg(rgb(0x0f1726))
                            .p_2()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.58))
                                    .child("Current source"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(white().opacity(0.94))
                                    .truncate()
                                    .child(selected_name),
                            )
                            .child(div().text_xs().text_color(white().opacity(0.6)).child(
                                format!(
                                    "{} · {} · {} imported",
                                    selected_kind_label, selected_duration_label, imported_count
                                ),
                            )),
                    )
                    .child(
                        div().w_full().min_w_0().overflow_x_scrollbar().child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.56))
                                .whitespace_nowrap()
                                .child(selected_path_label),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .rounded_sm()
                            .border_1()
                            .border_color(white().opacity(0.12))
                            .bg(rgb(0x131722))
                            .p_2()
                            .overflow_y_scrollbar()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .children(self.clips.iter().enumerate().map(|(idx, clip)| {
                                let active = self.selected_idx == Some(idx);
                                let idx_for_select = idx;
                                let duration_label = if clip.duration > Duration::ZERO {
                                    format!("{:.2}s", clip.duration.as_secs_f32())
                                } else {
                                    "Still".to_string()
                                };
                                div()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(white().opacity(if active { 0.35 } else { 0.14 }))
                                    .bg(if active { rgb(0x1f2937) } else { rgb(0x111827) })
                                    .px_2()
                                    .py_2()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(white().opacity(0.09)))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.6))
                                            .child(clip.kind.label()),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(white().opacity(0.93))
                                            .truncate()
                                            .child(clip.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.6))
                                            .truncate()
                                            .child(duration_label),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.selected_idx = Some(idx_for_select);
                                            cx.notify();
                                        }),
                                    )
                            })),
                    ),
            )
            .into_any_element()
    }

    // Render a single selectable template tile in the picker modal.
    fn render_template_tile(
        &self,
        kind: LayerEffectTemplateKind,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let selected = self.template_selected.contains(&kind);
        let border = if selected {
            rgba(0x4f8fffeb)
        } else {
            rgba(0xffffff3d)
        };
        let bg = if selected {
            rgba(0x253c62c7)
        } else {
            rgba(0xffffff1f)
        };
        let label = Self::template_label(kind);
        div()
            .h(px(34.0))
            .w(px(220.0))
            .px_3()
            .rounded_sm()
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_sm()
            .text_color(white().opacity(0.94))
            .cursor_pointer()
            .overflow_hidden()
            .child(div().w_full().truncate().child(label))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.toggle_template_selection(kind);
                    cx.notify();
                }),
            )
    }

    // Render the full-screen template picker modal overlay.
    fn render_template_modal_overlay(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let add_time_label = if self.template_add_time_parameter {
            "ADD TIME PARAMETER: ON"
        } else {
            "ADD TIME PARAMETER: OFF"
        };
        let add_curve_label = if self.template_add_curve_parameter {
            "ADD CURVE PARAMETER: ON"
        } else {
            "ADD CURVE PARAMETER: OFF"
        };
        let selection_summary = self.selected_template_summary();

        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .bg(gpui_component::black().opacity(0.55))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.template_modal_open = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(820.0))
                    .h(px(500.0))
                    .rounded_md()
                    .bg(rgb(0x1f1f23))
                    .border_1()
                    .border_color(white().opacity(0.16))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_3()
                    // Stop click propagation so clicking inside the modal doesn't close it
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.9))
                            .child("MOTIONLOOM TEMPLATE PICKER"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.65))
                            .child("Select one or more templates, then press OK to generate one graph."),
                    )
                    .child(
                        // Control bar: toggle buttons + OK + Close
                        div()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                div()
                                    .h(px(30.0))
                                    .px_3()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(white().opacity(0.2))
                                    .bg(white().opacity(0.08))
                                    .text_xs()
                                    .text_color(white().opacity(0.9))
                                    .cursor_pointer()
                                    .child(add_time_label)
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.template_add_time_parameter =
                                                !this.template_add_time_parameter;
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(30.0))
                                    .px_3()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(white().opacity(0.2))
                                    .bg(rgba(0x253c62c7))
                                    .text_xs()
                                    .text_color(white().opacity(0.94))
                                    .cursor_pointer()
                                    .child("OK")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, window, cx| {
                                            this.apply_selected_templates(window, cx);
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(30.0))
                                    .px_3()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(white().opacity(0.2))
                                    .bg(white().opacity(0.08))
                                    .text_xs()
                                    .text_color(white().opacity(0.9))
                                    .cursor_pointer()
                                    .child(add_curve_label)
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.template_add_curve_parameter =
                                                !this.template_add_curve_parameter;
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(30.0))
                                    .px_3()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(white().opacity(0.2))
                                    .bg(white().opacity(0.06))
                                    .text_xs()
                                    .text_color(white().opacity(0.82))
                                    .cursor_pointer()
                                    .child("Close")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.template_modal_open = false;
                                            cx.notify();
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        // Template grid organized by category
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .rounded_sm()
                            .border_1()
                            .border_color(white().opacity(0.12))
                            .bg(rgb(0x17181d))
                            .p_2()
                            .overflow_y_scrollbar()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.68))
                                    .child(format!("Selection: {selection_summary}")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.72))
                                    .child("Color Tuning"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(self.render_template_tile(
                                        LayerEffectTemplateKind::HslaOverlay,
                                        cx,
                                    ))
                                    .child(self.render_template_tile(
                                        LayerEffectTemplateKind::Lut,
                                        cx,
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.72))
                                    .child("Blend & Opacity"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(self.render_template_tile(
                                        LayerEffectTemplateKind::Opacity,
                                        cx,
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.72))
                                    .child("Detail & Blur"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(self.render_template_tile(
                                        LayerEffectTemplateKind::Sharpen,
                                        cx,
                                    ))
                                    .child(self.render_template_tile(
                                        LayerEffectTemplateKind::BlurGaussian,
                                        cx,
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.72))
                                    .child("Transitions"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(self.render_template_tile(
                                        LayerEffectTemplateKind::TransitionFadeInOut,
                                        cx,
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.58))
                            .child(
                                "ADD TIME PARAMETER adds apply=graph + duration(5s). ADD CURVE PARAMETER injects curve(...) into template params. Selected templates are chained in the order shown above.",
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for MotionLoomPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_script_from_global(window, cx);
        self.sync_render_request_from_global(window, cx);
        self.ensure_script_input(window, cx);
        let selected_idx = self.selected_idx;
        let selected = self.current_clip().cloned();
        let imported_count = self.clips.len();
        let runtime_active = self.graph_runtime.is_some();
        let selected_name = selected
            .as_ref()
            .map(|clip| clip.name.clone())
            .unwrap_or_else(|| "No source clip selected".to_string());
        let selected_kind_label = selected
            .as_ref()
            .map(|clip| clip.kind.label().to_string())
            .unwrap_or_else(|| "Source".to_string());
        let selected_duration_label = selected
            .as_ref()
            .map(|clip| {
                if clip.duration > Duration::ZERO {
                    format!("{:.2}s", clip.duration.as_secs_f32())
                } else {
                    "Still".to_string()
                }
            })
            .unwrap_or_else(|| "-".to_string());
        let selected_path_label = selected
            .as_ref()
            .map(|clip| clip.path.clone())
            .unwrap_or_else(|| "Import an image or video clip to begin previewing.".to_string());
        let viewport_w = window.viewport_size().width / px(1.0);
        let viewport_h = window.viewport_size().height / px(1.0);
        let content_max_w = if viewport_w >= 1800.0 {
            1560.0
        } else if viewport_w >= 1500.0 {
            1360.0
        } else if viewport_w >= 1280.0 {
            1180.0
        } else if viewport_w >= 1080.0 {
            980.0
        } else {
            860.0
        };
        // VFX preview should be the first visual priority on this page.
        // Scale panel heights continuously with viewport height.
        let stage_panel_h = if viewport_w < 1080.0 {
            (viewport_h * 0.36).clamp(220.0, 420.0)
        } else if viewport_w < 1320.0 {
            (viewport_h * 0.42).clamp(240.0, 520.0)
        } else {
            (viewport_h * 0.48).clamp(260.0, 620.0)
        };
        let preview_min_h = (stage_panel_h - 120.0).clamp(150.0, 420.0);
        let scene_live_preview_h = if viewport_w < 1080.0 {
            (viewport_h * 0.30).clamp(180.0, 320.0)
        } else {
            (viewport_h * 0.36).clamp(220.0, 430.0)
        };
        let scene_live_panel_min_h = (scene_live_preview_h + 180.0).clamp(360.0, 640.0);

        // Evaluate graph runtime output for current frame
        let runtime_output = self
            .graph_runtime
            .as_ref()
            .map(|runtime| runtime.evaluate_frame(self.preview_frame));
        let runtime_target_size = self.runtime_target_size();
        let runtime_mix = runtime_output.as_ref().map(|o| o.invert_mix).unwrap_or(0.0);
        let runtime_brightness = 0.0_f32;
        let runtime_contrast = 1.0_f32;
        let runtime_saturation = 1.0_f32;
        let runtime_blur = runtime_output
            .as_ref()
            .map(|o| {
                o.layer_blur_sigma
                    .or_else(|| o.layer_sharpen_sigma.map(|v| -v))
                    .unwrap_or(0.0)
            })
            .unwrap_or(0.0);
        let runtime_opacity = 1.0_f32;

        // MotionLoom previews need deterministic CPU-side frames for graph effects and
        // ProRes scene renders. Avoid the app-wide VideoElement fast path here; it can
        // stay blank for sources that do not produce a stable surface frame.
        let selected_video_waiting_for_first_frame = false;

        // Build script editor element (expands in left column)
        let script_input_elem = if let Some(input) = self.script_input.as_ref() {
            div()
                .w_full()
                .flex_1()
                .min_h(px(0.0))
                .rounded_md()
                .border_1()
                .border_color(white().opacity(0.18))
                .bg(rgb(0x0b1020))
                .overflow_hidden()
                .child(Input::new(input).h_full().w_full())
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h(px(0.0))
                .w_full()
                .rounded_sm()
                .bg(white().opacity(0.05))
                .into_any_element()
        };

        let preview_card = if let Some(clip) = selected {
            if let Some(preview) = clip.preview {
                let mut source_w = preview.width;
                let mut source_h = preview.height;
                let mut source_bgra = preview.bgra.as_ref().to_vec();
                if clip.kind == ImportedClipKind::Video
                    && let Some((bgra, w, h)) =
                        self.video_preview_frame_bgra(&clip.path, self.preview_frame)
                {
                    source_bgra = bgra;
                    source_w = w;
                    source_h = h;
                }
                let (display_image, display_w, display_h) = if runtime_active {
                    let idx = selected_idx.unwrap_or(0);
                    let size = runtime_target_size.unwrap_or((preview.width, preview.height));
                    self.runtime_preview_image(
                        idx,
                        &source_bgra,
                        source_w,
                        source_h,
                        preview.image.clone(),
                        self.preview_frame,
                        runtime_mix,
                        runtime_brightness,
                        runtime_contrast,
                        runtime_saturation,
                        runtime_blur,
                        runtime_opacity,
                        size,
                    )
                } else {
                    (
                        Self::render_image_from_bgra(source_w, source_h, source_bgra)
                            .unwrap_or_else(|_| preview.image.clone()),
                        source_w,
                        source_h,
                    )
                };
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(preview_min_h))
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(rgb(0x05070c))
                    .overflow_hidden()
                    .child(FitPreviewImageElement::new(
                        display_image,
                        display_w,
                        display_h,
                    ))
                    .into_any_element()
            } else if clip.kind == ImportedClipKind::Video
                && let Some((bgra, w, h)) =
                    self.video_preview_frame_bgra(&clip.path, self.preview_frame)
            {
                let (display_image, display_w, display_h) = if runtime_active {
                    let idx = selected_idx.unwrap_or(0);
                    let size = runtime_target_size.unwrap_or((w, h));
                    let fallback = Self::render_image_from_bgra(w, h, bgra.clone())
                        .unwrap_or_else(|_| Arc::new(RenderImage::new(SmallVec::new())));
                    self.runtime_preview_image(
                        idx,
                        &bgra,
                        w,
                        h,
                        fallback,
                        self.preview_frame,
                        runtime_mix,
                        runtime_brightness,
                        runtime_contrast,
                        runtime_saturation,
                        runtime_blur,
                        runtime_opacity,
                        size,
                    )
                } else {
                    (
                        Self::render_image_from_bgra(w, h, bgra)
                            .unwrap_or_else(|_| Arc::new(RenderImage::new(SmallVec::new()))),
                        w,
                        h,
                    )
                };
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(preview_min_h))
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(rgb(0x05070c))
                    .overflow_hidden()
                    .child(FitPreviewImageElement::new(
                        display_image,
                        display_w,
                        display_h,
                    ))
                    .into_any_element()
            } else {
                let no_preview_message = if clip.kind == ImportedClipKind::Video
                    && selected_video_waiting_for_first_frame
                {
                    "Loading video preview frame...".to_string()
                } else {
                    clip.error
                        .unwrap_or_else(|| "No preview available for this clip.".to_string())
                };
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(preview_min_h))
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(rgb(0x05070c))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.68))
                            .child(no_preview_message),
                    )
                    .into_any_element()
            }
        } else {
            div()
                .w_full()
                .flex_1()
                .min_h(px(preview_min_h))
                .rounded_lg()
                .border_1()
                .border_color(white().opacity(0.14))
                .bg(rgb(0x05070c))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(white().opacity(0.55))
                        .child("Import a clip to start the VFX stage."),
                )
                .into_any_element()
        };

        let mut scene_live_targets = Self::extract_scene_live_targets(&self.script_text);
        if self.scene_live_groups_only {
            scene_live_targets.retain(|target| target.tag == "Group");
        }
        self.ensure_scene_live_selection(&scene_live_targets);
        let scene_live_attrs = scene_live_targets
            .iter()
            .find(|target| target.id == self.scene_live_knob_node_id)
            .map(|target| target.attrs.clone())
            .unwrap_or_else(|| vec![self.scene_live_knob_attr.clone()]);

        let mut scene_live_target_entries = Vec::<(String, String, Vec<String>, bool)>::new();
        if !self.scene_live_groups_only
            && let Some(overall) = Self::overall_scene_live_target(&scene_live_targets)
        {
            scene_live_target_entries.push((
                format!("Overall · {}", overall.id),
                overall.id.clone(),
                overall.attrs.clone(),
                self.scene_live_knob_node_id == overall.id,
            ));
        }
        for target in scene_live_targets.iter() {
            scene_live_target_entries.push((
                format!("{} · {}", target.tag, target.id),
                target.id.clone(),
                target.attrs.clone(),
                self.scene_live_knob_node_id == target.id,
            ));
        }
        let scene_live_target_visible_count = if viewport_w < 1080.0 {
            4_usize
        } else if viewport_w < 1500.0 {
            6_usize
        } else {
            8_usize
        };
        let scene_live_target_total = scene_live_target_entries.len();
        let scene_live_target_max_offset =
            scene_live_target_total.saturating_sub(scene_live_target_visible_count);
        self.scene_live_target_offset = self
            .scene_live_target_offset
            .min(scene_live_target_max_offset);
        let scene_live_target_offset = self.scene_live_target_offset;
        let scene_live_target_chips = scene_live_target_entries
            .iter()
            .skip(scene_live_target_offset)
            .take(scene_live_target_visible_count)
            .map(|(label, id, attrs, active)| {
                let id = id.clone();
                let attrs = attrs.clone();
                Self::scene_live_chip(label.clone(), *active).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.select_scene_live_target(id.clone(), attrs.clone());
                        cx.notify();
                    }),
                )
            })
            .collect::<Vec<_>>();

        let scene_live_attr_chips = scene_live_attrs
            .iter()
            .map(|attr| {
                let attr_value = attr.clone();
                Self::scene_live_chip(
                    attr.clone(),
                    self.scene_live_knob_attr.as_str() == attr.as_str(),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.select_scene_live_attr(attr_value.clone());
                        cx.notify();
                    }),
                )
            })
            .collect::<Vec<_>>();

        let scene_live_selector_panel = div()
            .w_full()
            .overflow_hidden()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.1))
            .bg(white().opacity(0.025))
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(48.0))
                            .text_xs()
                            .text_color(white().opacity(0.52))
                            .child("Target"),
                    )
                    .child(Self::scene_live_chip("<".to_string(), false).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.scene_live_target_offset =
                                this.scene_live_target_offset.saturating_sub(1);
                            cx.notify();
                        }),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .on_scroll_wheel(cx.listener(
                                move |this, evt: &ScrollWheelEvent, _window, cx| {
                                    let delta_y = evt.delta.pixel_delta(px(10.0)).y / px(1.0);
                                    if delta_y.abs() <= f32::EPSILON {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    let max_offset = scene_live_target_total
                                        .saturating_sub(scene_live_target_visible_count);
                                    if delta_y > 0.0 {
                                        this.scene_live_target_offset =
                                            (this.scene_live_target_offset + 1).min(max_offset);
                                    } else {
                                        this.scene_live_target_offset =
                                            this.scene_live_target_offset.saturating_sub(1);
                                    }
                                    cx.notify();
                                },
                            ))
                            .children(scene_live_target_chips),
                    )
                    .child(Self::scene_live_chip(">".to_string(), false).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            let max_offset = scene_live_target_total
                                .saturating_sub(scene_live_target_visible_count);
                            this.scene_live_target_offset =
                                (this.scene_live_target_offset + 1).min(max_offset);
                            cx.notify();
                        }),
                    )),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(48.0))
                            .text_xs()
                            .text_color(white().opacity(0.52))
                            .child("Attr"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .children(scene_live_attr_chips),
                    ),
            );

        let scene_live_preview_card =
            match self.scene_live_preview_image(self.preview_frame) {
                Ok(Some((image, w, h))) => div()
                    .w_full()
                    .h(px(scene_live_preview_h))
                    .flex_shrink_0()
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(rgb(0x05070c))
                    .overflow_hidden()
                    .child(FitPreviewImageElement::new(image, w, h))
                    .into_any_element(),
                Ok(None) => div()
                    .w_full()
                    .h(px(scene_live_preview_h))
                    .flex_shrink_0()
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(rgb(0x05070c))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().text_sm().text_color(white().opacity(0.62)).child(
                        "Scene Live Preview renders the current <Graph scope=\"scene\"> frame.",
                    ))
                    .into_any_element(),
                Err(message) => div()
                    .w_full()
                    .h(px(scene_live_preview_h))
                    .flex_shrink_0()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgba(0xff6655cc))
                    .bg(rgb(0x12080a))
                    .p_3()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.78))
                            .child(message),
                    )
                    .into_any_element(),
            };

        let scene_live_knob_target = format!(
            "{}.{}",
            self.scene_live_knob_node_id, self.scene_live_knob_attr
        );
        let scene_live_knob_value = self
            .scene_live_knob_current_value()
            .map(Self::format_live_number)
            .unwrap_or_else(|| {
                format!(
                    "{}*",
                    Self::format_live_number(Self::scene_live_attr_default_value(
                        &self.scene_live_knob_attr
                    ))
                )
            });
        let scene_live_knob_scroll_label = format!("Scroll {}", self.scene_live_knob_attr);
        let scene_live_scroll_attr = self.scene_live_knob_attr.clone();
        let (
            scene_live_small_step,
            scene_live_large_step,
            scene_live_neg_large_label,
            scene_live_neg_small_label,
            scene_live_pos_small_label,
            scene_live_pos_large_label,
        ) = Self::scene_live_step_labels(&self.scene_live_knob_attr);
        let scene_live_scroll_hint = Self::scene_live_scroll_hint(&self.scene_live_knob_attr);
        let scene_live_knob_scroll = div()
            .h(px(28.0))
            .min_w(px(148.0))
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.18))
            .bg(white().opacity(0.055))
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .on_scroll_wheel(
                cx.listener(move |this, evt: &ScrollWheelEvent, window, cx| {
                    let delta = Self::scene_live_scroll_delta(evt, &scene_live_scroll_attr);
                    if delta.abs() <= f32::EPSILON {
                        return;
                    }
                    cx.stop_propagation();
                    this.nudge_scene_live_knob(delta, window, cx);
                    cx.notify();
                }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.58))
                    .child(scene_live_knob_scroll_label),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.94))
                    .child(scene_live_knob_value),
            );
        let scene_live_controls_row = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                Self::scene_live_checkbox("Group tags", self.scene_live_groups_only).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.scene_live_groups_only = !this.scene_live_groups_only;
                        this.scene_live_target_offset = 0;
                        this.status_line = if this.scene_live_groups_only {
                            "Scene live target filter: showing Group tags only.".to_string()
                        } else {
                            "Scene live target filter: showing all target tags.".to_string()
                        };
                        cx.notify();
                    }),
                ),
            )
            .child(Self::control_button("Frame 0").on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.preview_playing = false;
                    this.preview_last_tick = None;
                    this.preview_frame_accum = 0.0;
                    this.preview_frame = 0;
                    cx.notify();
                }),
            ))
            .child(Self::control_button("F-1").on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.step_preview_frame(-1);
                    cx.notify();
                }),
            ))
            .child(Self::control_button("F+1").on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.step_preview_frame(1);
                    cx.notify();
                }),
            ))
            .child(
                Self::scene_live_chip(scene_live_neg_large_label, false).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.nudge_scene_live_knob(-scene_live_large_step, window, cx);
                        cx.notify();
                    }),
                ),
            )
            .child(
                Self::scene_live_chip(scene_live_neg_small_label, false).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.nudge_scene_live_knob(-scene_live_small_step, window, cx);
                        cx.notify();
                    }),
                ),
            )
            .child(scene_live_knob_scroll)
            .child(
                Self::scene_live_chip(scene_live_pos_small_label, false).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.nudge_scene_live_knob(scene_live_small_step, window, cx);
                        cx.notify();
                    }),
                ),
            )
            .child(
                Self::scene_live_chip(scene_live_pos_large_label, false).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.nudge_scene_live_knob(scene_live_large_step, window, cx);
                        cx.notify();
                    }),
                ),
            );

        let source_button = Self::control_button("Source / Import").on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.open_import_modal();
                cx.notify();
            }),
        );
        let assets_button = Self::control_button("Assets").on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.open_asset_modal();
                cx.notify();
            }),
        );

        // --- Left column: Graph Lab (code) ---
        let graph_lab_panel = div()
            .w_full()
            .flex_1()
            .min_w_0()
            .min_h(px(260.0))
            .rounded_lg()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x0c111b))
            .p_3()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.94))
                            .child("Graph Lab"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(source_button)
                            .child(assets_button)
                            .child(Self::control_button("Scene Template").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.set_script_text(
                                        motionloom_templates::DEFAULT_SCENE_SCRIPT.to_string(),
                                        window,
                                        cx,
                                    );
                                    this.status_line =
                                        "Loaded MotionLoom scene template.".to_string();
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Template Picker").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.open_template_modal();
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Apply Effect").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.apply_script_command(cx);
                                    cx.notify();
                                }),
                            ))
                            .child(
                                Self::control_button("Compatibility Render (CPU)").on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, window, cx| {
                                        this.render_scene_to_media_pool(
                                            SceneRenderMode::CompatibilityCpu,
                                            window,
                                            cx,
                                        );
                                    }),
                                ),
                            )
                            .child(Self::control_button("GPU Render").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.render_scene_to_media_pool(
                                        SceneRenderMode::GpuNativeH264,
                                        window,
                                        cx,
                                    );
                                }),
                            ))
                            .child(Self::control_button("GPU Render (ProRes)").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.render_scene_to_media_pool(
                                        SceneRenderMode::GpuNativeProRes,
                                        window,
                                        cx,
                                    );
                                }),
                            )),
                    ),
            )
            .child(div().text_xs().text_color(white().opacity(0.68)).child(
                "VFX stage on top, graph editor below. Use Source / Import to open clip manager.",
            ))
            .child(script_input_elem);

        // --- Right column: VFX Stage (video) ---
        let mut stage_panel = div()
            .w_full()
            .h(px(stage_panel_h))
            .flex_shrink_0()
            .min_h(px(220.0))
            .rounded_lg()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x0c111b))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(white().opacity(0.95))
                                    .child("VFX Stage"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.55))
                                    .truncate()
                                    .child(format!(
                                        "{} · {} · {}",
                                        selected_name, selected_kind_label, selected_duration_label
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.75))
                                    .child("Frame"),
                            )
                            .child(
                                Self::control_button(if self.preview_playing {
                                    "Pause"
                                } else {
                                    "Play"
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, window, cx| {
                                        this.toggle_preview_playback(window, cx);
                                    }),
                                ),
                            )
                            .child(Self::control_button("-1").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.step_preview_frame(-1);
                                    cx.notify();
                                }),
                            ))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.9))
                                    .child(format!("{}", self.preview_frame)),
                            )
                            .child(Self::control_button("+1").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.step_preview_frame(1);
                                    cx.notify();
                                }),
                            )),
                    ),
            );

        stage_panel = stage_panel.child(preview_card).child(
            div().w_full().min_w_0().overflow_x_scrollbar().child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.66))
                    .whitespace_nowrap()
                    .child(self.status_line.clone()),
            ),
        );

        // --- Scene Live Preview: direct single-frame scene raster, no FFmpeg ---
        let scene_live_panel = div()
            .w_full()
            .h_auto()
            .flex_shrink_0()
            .min_h(px(scene_live_panel_min_h))
            .rounded_lg()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x0c111b))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(white().opacity(0.95))
                                    .child("Scene Live Preview"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.56))
                                    .truncate()
                                    .child(format!(
                                        "Direct frame {} · no FFmpeg · target {}",
                                        self.preview_frame, scene_live_knob_target
                                    )),
                            ),
                    ),
            )
            .child(scene_live_selector_panel)
            .child(scene_live_controls_row)
            .child(scene_live_preview_card)
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.58))
                    .child(scene_live_scroll_hint),
            );

        // --- Template picker modal overlay (rendered on top when open) ---
        let template_modal = if self.template_modal_open {
            Some(self.render_template_modal_overlay(cx))
        } else {
            None
        };

        // --- Import/source modal overlay ---
        let import_modal = if self.import_modal_open {
            Some(self.render_import_modal_overlay(
                selected_name.clone(),
                selected_kind_label.clone(),
                selected_duration_label.clone(),
                selected_path_label.clone(),
                imported_count,
                cx,
            ))
        } else {
            None
        };
        let asset_modal = if self.asset_modal_open {
            Some(self.render_vfx_asset_modal_overlay(window, cx))
        } else {
            None
        };

        let source_summary_line = format!(
            "Current source: {} · {} · {} · {} imported · {}",
            selected_name,
            selected_kind_label,
            selected_duration_label,
            imported_count,
            selected_path_label
        );

        div()
            .size_full()
            .bg(rgb(0x080a10))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(white().opacity(0.12))
                    .bg(rgb(0x090b12))
                    .px_3()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .text_lg()
                                    .text_color(white().opacity(0.96))
                                    .child("MotionLoom · VFX Studio (UI Simplified)"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.66))
                                    .truncate()
                                    .child("Import panel is now modal. Main view is fixed two columns."),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.56))
                                    .whitespace_normal()
                                    .child(source_summary_line),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(content_max_w))
                            .mx_auto()
                            .p_3()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(stage_panel)
                            .child(scene_live_panel)
                            .child(graph_lab_panel),
                    ),
            )
            .when(self.import_modal_open, |el| el.child(import_modal.unwrap()))
            .when(self.asset_modal_open, |el| el.child(asset_modal.unwrap()))
            .when(self.template_modal_open, |el| {
                el.child(template_modal.unwrap())
            })
            .when(self.preview_playing, |el| {
                window.request_animation_frame();
                el
            })
    }
}
