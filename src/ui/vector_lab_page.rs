// src/ui/vector_lab_page.rs - lightweight MotionLoom path tracing workspace.
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui::{
    App, Bounds, ClipboardEntry, ClipboardItem, Context, Element, Entity, FocusHandle, Focusable,
    GlobalElementId, Hsla, ImageFormat as GpuiImageFormat, InspectorElementId, IntoElement,
    KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder,
    PathPromptOptions, Pixels, Render, RenderImage, Style, Subscription, Timer, Window, canvas,
    div, point, prelude::*, px, rgb, rgba,
};
use gpui_component::{
    Colorize, Sizable,
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    white,
};
use image::{ImageBuffer, Rgba};
use motionloom::{
    SceneRenderProfile, WorldFrameRenderer, is_graph_script, is_world_graph_script,
    parse_graph_script, parse_world_graph_script, render_scene_graph_frame,
};
use smallvec::SmallVec;

use crate::core::global_state::{AppPage, GlobalState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VectorTool {
    Pen,
    Freehand,
}

impl VectorTool {
    fn label(self) -> &'static str {
        match self {
            Self::Pen => "Point",
            Self::Freehand => "Freehand",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct VectorPoint {
    x: f32,
    y: f32,
    pressure: f32,
}

impl Default for VectorPoint {
    fn default() -> Self {
        Self::new(0.0, 0.0)
    }
}

impl VectorPoint {
    fn new(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            pressure: 1.0,
        }
    }

    fn with_pressure(self, pressure: f32) -> Self {
        Self {
            pressure: pressure.clamp(0.05, 1.0),
            ..self
        }
    }

    fn distance_to(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VectorPathKind {
    Pen,
    Freehand,
}

impl VectorPathKind {
    fn label(self) -> &'static str {
        match self {
            Self::Pen => "pen bezier",
            Self::Freehand => "freehand simplified",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BrushPreset {
    CleanInk,
    BoldInk,
    Pencil,
    BluePencil,
    Sketch,
    Rough,
    Charcoal,
    Marker,
    Hairline,
}

impl BrushPreset {
    const ALL: [Self; 9] = [
        Self::CleanInk,
        Self::BoldInk,
        Self::Pencil,
        Self::BluePencil,
        Self::Sketch,
        Self::Rough,
        Self::Charcoal,
        Self::Marker,
        Self::Hairline,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::CleanInk => "Clean Ink",
            Self::BoldInk => "Bold Ink",
            Self::Pencil => "Pencil",
            Self::BluePencil => "Blue Pencil",
            Self::Sketch => "Sketch",
            Self::Rough => "Rough",
            Self::Charcoal => "Charcoal",
            Self::Marker => "Marker",
            Self::Hairline => "Hairline",
        }
    }

    fn dsl_style(self) -> &'static str {
        match self {
            Self::CleanInk => "ink",
            Self::BoldInk => "ink",
            Self::Pencil => "pencil",
            Self::BluePencil => "pencil",
            Self::Sketch => "sketch",
            Self::Rough => "rough",
            Self::Charcoal => "charcoal",
            Self::Marker => "marker",
            Self::Hairline => "hairline",
        }
    }
}

#[derive(Clone, Debug)]
struct BrushSettings {
    preset: BrushPreset,
    stroke_width: f32,
    opacity: f32,
    roughness: f32,
    copies: u32,
    pressure_min: f32,
    pressure_curve: f32,
    texture_strength: f32,
    stamp_spacing: f32,
    bristle_count: u32,
    color: String,
}

impl BrushSettings {
    fn for_preset(preset: BrushPreset) -> Self {
        let (
            stroke_width,
            opacity,
            roughness,
            copies,
            pressure_min,
            pressure_curve,
            texture_strength,
            stamp_spacing,
            bristle_count,
            color,
        ) = match preset {
            BrushPreset::CleanInk => (3.0, 0.98, 0.02, 1, 0.88, 0.80, 0.00, 10.0, 0, "#111111"),
            BrushPreset::BoldInk => (5.2, 0.96, 0.08, 1, 0.74, 0.72, 0.04, 10.0, 0, "#111111"),
            BrushPreset::Pencil => (1.6, 0.36, 1.80, 6, 0.18, 1.65, 0.74, 5.5, 5, "#111111"),
            BrushPreset::BluePencil => (1.5, 0.40, 1.45, 4, 0.20, 1.50, 0.60, 6.0, 4, "#3f79c5"),
            BrushPreset::Sketch => (2.6, 0.56, 1.25, 5, 0.30, 1.30, 0.38, 7.0, 4, "#111111"),
            BrushPreset::Rough => (4.4, 0.50, 2.20, 7, 0.26, 1.15, 0.72, 5.0, 8, "#111111"),
            BrushPreset::Charcoal => (10.5, 0.30, 3.40, 10, 0.12, 1.00, 0.95, 4.0, 14, "#111111"),
            BrushPreset::Marker => (11.0, 0.64, 0.00, 1, 0.80, 0.70, 0.12, 12.0, 0, "#111111"),
            BrushPreset::Hairline => (1.2, 0.94, 0.08, 1, 0.90, 1.00, 0.00, 10.0, 0, "#111111"),
        };
        Self {
            preset,
            stroke_width,
            opacity,
            roughness,
            copies,
            pressure_min,
            pressure_curve,
            texture_strength,
            stamp_spacing,
            bristle_count,
            color: color.to_string(),
        }
    }

    fn summary(&self) -> String {
        format!(
            "{} W{} O{} R{} C{} T{} B{}",
            self.preset.label(),
            Self::format_value(self.stroke_width),
            Self::format_value(self.opacity),
            Self::format_value(self.roughness),
            self.copies,
            Self::format_value(self.texture_strength),
            self.bristle_count
        )
    }

    fn format_value(value: f32) -> String {
        let rounded = (value * 10.0).round() / 10.0;
        if (rounded - rounded.round()).abs() < 0.05 {
            format!("{:.0}", rounded)
        } else {
            format!("{:.1}", rounded)
        }
    }
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self::for_preset(BrushPreset::Pencil)
    }
}

#[derive(Clone, Debug)]
struct VectorPathDraft {
    id: String,
    kind: VectorPathKind,
    points: Vec<VectorPoint>,
    brush: BrushSettings,
    export_simplify_epsilon: f32,
}

#[derive(Clone, Debug)]
struct VectorBrushDef {
    id: String,
    brush: BrushSettings,
}

#[derive(Clone, Debug)]
struct VectorGroupExport {
    brush_defs: Vec<VectorBrushDef>,
    group_block: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReferenceImageSource {
    Imported,
    VfxPreview,
}

impl Default for ReferenceImageSource {
    fn default() -> Self {
        Self::Imported
    }
}

#[derive(Clone)]
struct ReferenceImageLayer {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
    source: ReferenceImageSource,
}

struct ReferenceImageElement {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
    zoom: f32,
    offset: VectorPoint,
}

impl ReferenceImageElement {
    fn new(layer: &ReferenceImageLayer, zoom: f32, offset: VectorPoint) -> Self {
        Self {
            image: layer.image.clone(),
            width: layer.width,
            height: layer.height,
            zoom,
            offset,
        }
    }

    fn fitted_bounds(&self, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        let container_w = bounds.size.width / px(1.0);
        let container_h = bounds.size.height / px(1.0);
        let frame_w = self.width.max(1) as f32;
        let frame_h = self.height.max(1) as f32;
        let fit_scale = (container_w / frame_w).min(container_h / frame_h);
        let scale = fit_scale * self.zoom.max(0.05);
        let dest_w = frame_w * scale;
        let dest_h = frame_h * scale;
        let offset_x = (container_w - dest_w) * 0.5 + self.offset.x;
        let offset_y = (container_h - dest_h) * 0.5 + self.offset.y;

        Bounds::new(
            point(
                bounds.origin.x + px(offset_x),
                bounds.origin.y + px(offset_y),
            ),
            gpui::size(px(dest_w), px(dest_h)),
        )
    }
}

impl Element for ReferenceImageElement {
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
        cx: &mut App,
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
        _bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let _ = window.paint_image(
            self.fitted_bounds(bounds),
            gpui::Corners::default(),
            self.image.clone(),
            0,
            false,
        );
    }
}

impl IntoElement for ReferenceImageElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[derive(Clone)]
struct VectorCanvasSnapshot {
    paths: Vec<VectorPathDraft>,
    selected_path: Option<usize>,
    current_pen_points: Vec<VectorPoint>,
    current_freehand: Vec<VectorPoint>,
    current_brush: BrushSettings,
    bounds: Arc<Mutex<Option<Bounds<Pixels>>>>,
}

pub struct VectorLabPage {
    global: Entity<GlobalState>,
    focus_handle: FocusHandle,
    tool: VectorTool,
    image_controls_open: bool,
    brush_menu_open: bool,
    advanced_brush_open: bool,
    reference_image: Option<ReferenceImageLayer>,
    reference_opacity: f32,
    reference_zoom: f32,
    reference_offset: VectorPoint,
    canvas_bg_color: Hsla,
    canvas_bg_picker: Option<Entity<ColorPickerState>>,
    canvas_bg_picker_sub: Option<Subscription>,
    brush_color_picker: Option<Entity<ColorPickerState>>,
    brush_color_picker_sub: Option<Subscription>,
    paths: Vec<VectorPathDraft>,
    selected_path: Option<usize>,
    current_pen_points: Vec<VectorPoint>,
    current_freehand: Vec<VectorPoint>,
    is_drawing_freehand: bool,
    current_brush: BrushSettings,
    simplify_epsilon: f32,
    path_counter: usize,
    motionloom_group_id_text: String,
    motionloom_group_id_input: Option<Entity<InputState>>,
    motionloom_group_id_input_sub: Option<Subscription>,
    group_picker_open: bool,
    status_line: String,
    board_bounds: Arc<Mutex<Option<Bounds<Pixels>>>>,
}

impl VectorLabPage {
    pub fn new(global: Entity<GlobalState>, cx: &mut Context<Self>) -> Self {
        Self {
            global,
            focus_handle: cx.focus_handle(),
            tool: VectorTool::Freehand,
            image_controls_open: false,
            brush_menu_open: false,
            advanced_brush_open: false,
            reference_image: None,
            reference_opacity: 0.42,
            reference_zoom: 1.0,
            reference_offset: VectorPoint::default(),
            canvas_bg_color: Hsla::from(rgb(0xffffff)),
            canvas_bg_picker: None,
            canvas_bg_picker_sub: None,
            brush_color_picker: None,
            brush_color_picker_sub: None,
            paths: Vec::new(),
            selected_path: None,
            current_pen_points: Vec::new(),
            current_freehand: Vec::new(),
            is_drawing_freehand: false,
            current_brush: BrushSettings::default(),
            simplify_epsilon: 2.2,
            path_counter: 1,
            motionloom_group_id_text: String::new(),
            motionloom_group_id_input: None,
            motionloom_group_id_input_sub: None,
            group_picker_open: false,
            status_line:
                "Vector Lab ready. Upload/paste a reference image, then trace with Pen or Freehand."
                    .to_string(),
            board_bounds: Arc::new(Mutex::new(None)),
        }
    }

    fn control_button(label: &'static str) -> gpui::Div {
        div()
            .h(px(30.0))
            .flex_shrink_0()
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.15))
            .bg(white().opacity(0.06))
            .hover(|s| s.bg(white().opacity(0.11)))
            .cursor_pointer()
            .text_xs()
            .text_color(white().opacity(0.9))
            .flex()
            .items_center()
            .justify_center()
            .child(label)
    }

    fn tool_button(label: &'static str, active: bool) -> gpui::Div {
        let border = if active {
            rgba(0x79c7ffcc)
        } else {
            rgba(0xffffff26)
        };
        let bg = if active {
            rgba(0x1d4f7acc)
        } else {
            rgba(0xffffff10)
        };
        div()
            .h(px(30.0))
            .flex_shrink_0()
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(bg)
            .hover(|s| s.bg(white().opacity(0.12)))
            .cursor_pointer()
            .text_xs()
            .text_color(white().opacity(if active { 0.98 } else { 0.82 }))
            .flex()
            .items_center()
            .justify_center()
            .child(label)
    }

    fn dynamic_button(label: String, active: bool) -> gpui::Div {
        let border = if active {
            rgba(0x79c7ffcc)
        } else {
            rgba(0xffffff26)
        };
        let bg = if active {
            rgba(0x1d4f7acc)
        } else {
            rgba(0xffffff10)
        };
        div()
            .h(px(30.0))
            .flex_shrink_0()
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(bg)
            .hover(|s| s.bg(white().opacity(0.12)))
            .cursor_pointer()
            .text_xs()
            .text_color(white().opacity(if active { 0.98 } else { 0.82 }))
            .flex()
            .items_center()
            .justify_center()
            .child(label)
    }

    fn section_label(label: &'static str) -> gpui::Div {
        div()
            .h(px(30.0))
            .flex_shrink_0()
            .px_2()
            .text_xs()
            .text_color(white().opacity(0.48))
            .flex()
            .items_center()
            .child(label)
    }

    fn value_pill(value: String) -> gpui::Div {
        div()
            .h(px(30.0))
            .flex_shrink_0()
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.10))
            .bg(rgba(0x050914cc))
            .text_xs()
            .text_color(white().opacity(0.76))
            .flex()
            .items_center()
            .justify_center()
            .child(value)
    }

    fn sync_selected_brush(&mut self) {
        if let Some(idx) = self.selected_path {
            if let Some(path) = self.paths.get_mut(idx) {
                path.brush = self.current_brush.clone();
            }
        }
    }

    fn set_brush_color(&mut self, color: Hsla) {
        self.current_brush.color = color.to_hex();
        self.sync_selected_brush();
        self.status_line = format!("Brush color: {}.", self.current_brush.color);
    }

    fn select_brush_preset(&mut self, preset: BrushPreset) {
        self.current_brush = BrushSettings::for_preset(preset);
        self.brush_menu_open = false;
        self.sync_selected_brush();
        self.status_line = format!("Brush selected: {}.", self.current_brush.summary());
    }

    fn adjust_brush_width(&mut self, delta: f32) {
        self.current_brush.stroke_width =
            (self.current_brush.stroke_width + delta).clamp(0.01, 40.0);
        self.sync_selected_brush();
        self.status_line = format!(
            "Brush width: {}.",
            Self::format_coord(self.current_brush.stroke_width)
        );
    }

    fn adjust_brush_opacity(&mut self, delta: f32) {
        self.current_brush.opacity = (self.current_brush.opacity + delta).clamp(0.05, 1.0);
        self.sync_selected_brush();
        self.status_line = format!(
            "Brush opacity: {}.",
            Self::format_coord(self.current_brush.opacity)
        );
    }

    fn adjust_brush_roughness(&mut self, delta: f32) {
        self.current_brush.roughness = (self.current_brush.roughness + delta).clamp(0.0, 4.0);
        self.sync_selected_brush();
        self.status_line = format!(
            "Brush roughness: {}.",
            Self::format_coord(self.current_brush.roughness)
        );
    }

    fn adjust_brush_copies(&mut self, delta: i32) {
        let next = (self.current_brush.copies as i32 + delta).clamp(1, 12);
        self.current_brush.copies = next as u32;
        self.sync_selected_brush();
        self.status_line = format!("Brush copies: {}.", self.current_brush.copies);
    }

    fn adjust_pressure_curve(&mut self, delta: f32) {
        self.current_brush.pressure_curve =
            (self.current_brush.pressure_curve + delta).clamp(0.25, 3.0);
        self.sync_selected_brush();
        self.status_line = format!(
            "Pressure curve: {}. Lower is fuller, higher is sharper.",
            Self::format_coord(self.current_brush.pressure_curve)
        );
    }

    fn adjust_texture_strength(&mut self, delta: f32) {
        self.current_brush.texture_strength =
            (self.current_brush.texture_strength + delta).clamp(0.0, 1.0);
        self.sync_selected_brush();
        self.status_line = format!(
            "Texture mask strength: {}.",
            Self::format_coord(self.current_brush.texture_strength)
        );
    }

    fn adjust_bristles(&mut self, delta: i32) {
        let next = (self.current_brush.bristle_count as i32 + delta).clamp(0, 24);
        self.current_brush.bristle_count = next as u32;
        self.sync_selected_brush();
        self.status_line = format!(
            "Bristle/noise strands: {}.",
            self.current_brush.bristle_count
        );
    }

    fn adjust_simplify(&mut self, delta: f32) {
        self.simplify_epsilon = (self.simplify_epsilon + delta).clamp(0.2, 12.0);
        self.status_line = format!(
            "Freehand simplify: {}. Lower keeps more detail.",
            Self::format_coord(self.simplify_epsilon)
        );
    }

    fn is_supported_image_path(path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg"
                )
            })
            .unwrap_or(false)
    }

    fn set_reference_path(&mut self, path: PathBuf) {
        if !path.exists() || !Self::is_supported_image_path(&path) {
            self.status_line =
                "Reference layer expects a local PNG/JPG/WebP/GIF/BMP/TIFF/SVG path.".to_string();
            return;
        }
        match Self::load_reference_image(&path) {
            Ok(layer) => {
                self.reference_image = Some(layer);
                self.reference_zoom = 1.0;
                self.reference_offset = VectorPoint::default();
                self.status_line = format!("Reference layer loaded: {}", path.display());
            }
            Err(err) => {
                self.status_line = err;
            }
        }
    }

    fn load_reference_image(path: &Path) -> Result<ReferenceImageLayer, String> {
        let decoded =
            image::open(path).map_err(|e| format!("Failed to open reference image: {e}"))?;
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        let mut bgra = rgba.into_raw();
        for pixel in bgra.chunks_mut(4) {
            let red = pixel[0];
            let blue = pixel[2];
            pixel[0] = blue;
            pixel[2] = red;
        }
        let image_buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, bgra)
            .ok_or_else(|| "Failed to construct reference image buffer.".to_string())?;
        let frames = SmallVec::from_elem(image::Frame::new(image_buffer), 1);
        Ok(ReferenceImageLayer {
            image: Arc::new(RenderImage::new(frames)),
            width,
            height,
            source: ReferenceImageSource::Imported,
        })
    }

    fn reference_layer_from_bgra(
        width: u32,
        height: u32,
        bgra: Vec<u8>,
    ) -> Result<ReferenceImageLayer, String> {
        let image_buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, bgra)
            .ok_or_else(|| "Failed to construct VFX preview reference image buffer.".to_string())?;
        let frames = SmallVec::from_elem(image::Frame::new(image_buffer), 1);
        Ok(ReferenceImageLayer {
            image: Arc::new(RenderImage::new(frames)),
            width,
            height,
            source: ReferenceImageSource::VfxPreview,
        })
    }

    fn world_asset_root() -> PathBuf {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        for candidate in [
            cwd.join("examples/motionloom/world"),
            cwd.join("anica/examples/motionloom/world"),
            PathBuf::from("examples/motionloom/world"),
            PathBuf::from("anica/examples/motionloom/world"),
        ] {
            if candidate.exists() {
                return candidate;
            }
        }
        PathBuf::from("examples/motionloom/world")
    }

    fn uses_pure_world_renderer(raw: &str) -> bool {
        is_world_graph_script(raw)
            && !raw.contains("<Tex")
            && !raw.contains("<Pass")
            && !raw.contains("<Layer")
            && !raw.contains("<Scene")
            && !raw.contains("<Clip")
    }

    async fn render_vfx_preview_reference_frame(
        raw: String,
        frame: u32,
    ) -> Result<(u32, u32, Vec<u8>), String> {
        if raw.trim().is_empty() {
            return Err("VFX Studio has no MotionLoom script to import.".to_string());
        }
        if !is_graph_script(&raw) {
            return Err(
                "VFX Studio preview import requires a <Graph> MotionLoom DSL block.".to_string(),
            );
        }

        let rgba = if Self::uses_pure_world_renderer(&raw) {
            let graph = parse_world_graph_script(&raw).map_err(|err| {
                format!(
                    "VFX world parse error at line {}: {}",
                    err.line, err.message
                )
            })?;
            let mut renderer = WorldFrameRenderer::new();
            renderer
                .render_frame_gpu(&graph, frame, Self::world_asset_root())
                .await
                .map_err(|err| format!("VFX world render error: {err}"))?
        } else {
            let graph = parse_graph_script(&raw).map_err(|err| {
                format!(
                    "VFX scene parse error at line {}: {}",
                    err.line, err.message
                )
            })?;
            if !graph.has_scene_nodes() {
                return Err("VFX scene preview needs at least one <Scene> node.".to_string());
            }
            render_scene_graph_frame(&graph, frame, SceneRenderProfile::Gpu)
                .await
                .map_err(|err| format!("VFX scene render error: {err}"))?
        };

        let (width, height) = rgba.dimensions();
        let mut bgra = rgba.into_raw();
        for px in bgra.chunks_mut(4) {
            px.swap(0, 2);
        }
        Ok((width, height, bgra))
    }

    fn import_vfx_preview_reference(&mut self, cx: &mut Context<Self>) {
        let (raw, frame) = {
            let gs = self.global.read(cx);
            (
                gs.motionloom_scene_script().to_string(),
                gs.motionloom_scene_preview_frame(),
            )
        };

        self.status_line = format!("Importing VFX Studio preview frame {frame}...");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = pollster::block_on(Self::render_vfx_preview_reference_frame(raw, frame));
            let _ = tx.send((frame, result));
        });

        cx.spawn(async move |view, cx| {
            loop {
                Timer::after(Duration::from_millis(16)).await;
                let mut done = false;
                let _ = view.update(cx, |this, cx| match rx.try_recv() {
                    Ok((frame, Ok((width, height, bgra)))) => {
                        match Self::reference_layer_from_bgra(width, height, bgra) {
                            Ok(layer) => {
                                this.reference_image = Some(layer);
                                this.reference_zoom = 1.0;
                                this.reference_offset = VectorPoint::default();
                                this.status_line = format!(
                                    "Imported VFX Studio preview frame {frame} as reference ({width}x{height})."
                                );
                            }
                            Err(err) => {
                                this.status_line = err;
                            }
                        }
                        done = true;
                        cx.notify();
                    }
                    Ok((_frame, Err(err))) => {
                        this.status_line = err;
                        done = true;
                        cx.notify();
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => {
                        this.status_line =
                            "VFX Studio preview import was interrupted.".to_string();
                        done = true;
                        cx.notify();
                    }
                });
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    fn zoom_reference(&mut self, factor: f32) {
        if self.reference_image.is_none() {
            self.status_line = "Load a reference image before changing image size.".to_string();
            return;
        }
        self.reference_zoom = (self.reference_zoom * factor).clamp(0.1, 8.0);
        self.status_line = format!(
            "Reference image zoom: {}%.",
            (self.reference_zoom * 100.0).round()
        );
    }

    fn fit_reference(&mut self) {
        if self.reference_image.is_none() {
            self.status_line = "Load a reference image before fitting it.".to_string();
            return;
        }
        self.reference_zoom = 1.0;
        self.reference_offset = VectorPoint::default();
        self.status_line =
            "Reference image fitted to canvas with preserved aspect ratio.".to_string();
    }

    fn nudge_reference(&mut self, dx: f32, dy: f32) {
        if self.reference_image.is_none() {
            self.status_line = "Load a reference image before moving it.".to_string();
            return;
        }
        self.reference_offset.x += dx;
        self.reference_offset.y += dy;
        self.status_line = format!(
            "Reference image offset: {}, {}.",
            Self::format_coord(self.reference_offset.x),
            Self::format_coord(self.reference_offset.y)
        );
    }

    fn save_clipboard_image(image: &gpui::Image) -> Result<PathBuf, String> {
        let ext = match image.format {
            GpuiImageFormat::Png => "png",
            GpuiImageFormat::Jpeg => "jpg",
            GpuiImageFormat::Webp => "webp",
            GpuiImageFormat::Gif => "gif",
            GpuiImageFormat::Svg => "svg",
            GpuiImageFormat::Bmp => "bmp",
            GpuiImageFormat::Tiff => "tiff",
        };
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("System clock error: {e}"))?
            .as_millis();
        let path = std::env::temp_dir().join(format!("anica_vector_lab_clipboard_{millis}.{ext}"));
        fs::write(&path, &image.bytes)
            .map_err(|e| format!("Failed to save clipboard image: {e}"))?;
        Ok(path)
    }

    fn paste_reference_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            self.status_line = "Clipboard is empty.".to_string();
            return;
        };

        for entry in item.entries() {
            if let ClipboardEntry::Image(image) = entry {
                match Self::save_clipboard_image(image) {
                    Ok(path) => {
                        self.set_reference_path(path);
                        return;
                    }
                    Err(err) => {
                        self.status_line = err;
                        return;
                    }
                }
            }
        }

        if let Some(text) = item.text() {
            let trimmed = text.trim().trim_matches('"');
            let path = PathBuf::from(trimmed);
            if path.exists() {
                self.set_reference_path(path);
            } else {
                self.status_line = "Clipboard text is not an existing local image path. Image clipboard and path text are supported.".to_string();
            }
        } else {
            self.status_line = "Clipboard has no supported image or text path.".to_string();
        }
    }

    fn point_from_event_position(&self, x: f32, y: f32) -> Option<VectorPoint> {
        let bounds = *self.board_bounds.lock().ok()?.as_ref()?;
        let left = bounds.origin.x / px(1.0);
        let top = bounds.origin.y / px(1.0);
        let width = bounds.size.width / px(1.0);
        let height = bounds.size.height / px(1.0);
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let local_x = (x - left).clamp(0.0, width);
        let local_y = (y - top).clamp(0.0, height);
        Some(VectorPoint::new(local_x, local_y))
    }

    fn mouse_down_point(&self, evt: &MouseDownEvent) -> Option<VectorPoint> {
        self.point_from_event_position(evt.position.x / px(1.0), evt.position.y / px(1.0))
    }

    fn mouse_move_point(&self, evt: &MouseMoveEvent) -> Option<VectorPoint> {
        self.point_from_event_position(evt.position.x / px(1.0), evt.position.y / px(1.0))
    }

    fn mouse_up_point(&self, evt: &MouseUpEvent) -> Option<VectorPoint> {
        self.point_from_event_position(evt.position.x / px(1.0), evt.position.y / px(1.0))
    }

    fn freehand_point_with_pressure(&self, point: VectorPoint, ending: bool) -> VectorPoint {
        let Some(last) = self.current_freehand.last().copied() else {
            return point.with_pressure(0.48);
        };
        let distance = last.distance_to(point);
        let speed_pressure = (1.18 - distance / 24.0).clamp(0.26, 1.0);
        let start_taper = if self.current_freehand.len() < 4 {
            0.52 + self.current_freehand.len() as f32 * 0.14
        } else {
            1.0
        };
        let end_taper = if ending { 0.36 } else { 1.0 };
        let target = speed_pressure * start_taper.min(1.0) * end_taper;
        let pressure = (last.pressure * 0.58 + target * 0.42).clamp(0.18, 1.0);
        point.with_pressure(pressure)
    }

    fn handle_canvas_mouse_down(&mut self, evt: &MouseDownEvent) {
        let Some(point) = self.mouse_down_point(evt) else {
            self.status_line = "Canvas bounds not ready yet.".to_string();
            return;
        };
        match self.tool {
            VectorTool::Pen => {
                self.current_pen_points.push(point);
                self.status_line = format!(
                    "Point added at {}, {}. Use Finish Path to create a path.",
                    Self::format_coord(point.x),
                    Self::format_coord(point.y)
                );
            }
            VectorTool::Freehand => {
                self.current_freehand.clear();
                self.current_freehand.push(point.with_pressure(0.48));
                self.is_drawing_freehand = true;
                self.status_line = "Freehand live stroke started.".to_string();
            }
        }
    }

    fn handle_canvas_mouse_move(&mut self, evt: &MouseMoveEvent) {
        if self.tool != VectorTool::Freehand || !self.is_drawing_freehand {
            return;
        }
        let Some(point) = self.mouse_move_point(evt) else {
            return;
        };
        let should_push = self
            .current_freehand
            .last()
            .map(|last| last.distance_to(point) >= 0.8)
            .unwrap_or(true);
        if should_push {
            self.current_freehand
                .push(self.freehand_point_with_pressure(point, false));
        }
    }

    fn handle_canvas_mouse_up(&mut self, evt: &MouseUpEvent) {
        if self.tool != VectorTool::Freehand || !self.is_drawing_freehand {
            return;
        }
        if let Some(point) = self.mouse_up_point(evt) {
            if self
                .current_freehand
                .last()
                .map(|last| last.distance_to(point) >= 1.0)
                .unwrap_or(true)
            {
                self.current_freehand
                    .push(self.freehand_point_with_pressure(point, true));
            }
        }
        self.is_drawing_freehand = false;
        self.finish_freehand_path();
    }

    fn finish_pen_path(&mut self) {
        if self.current_pen_points.len() < 2 {
            self.status_line = "Point path needs at least two points.".to_string();
            return;
        }
        let points = std::mem::take(&mut self.current_pen_points);
        self.push_path(VectorPathKind::Pen, points);
    }

    fn finish_freehand_path(&mut self) {
        if self.current_freehand.len() < 2 {
            self.current_freehand.clear();
            self.status_line = "Freehand path needs a longer stroke.".to_string();
            return;
        }
        let raw_points = std::mem::take(&mut self.current_freehand);
        self.push_path(VectorPathKind::Freehand, raw_points);
    }

    fn push_path(&mut self, kind: VectorPathKind, points: Vec<VectorPoint>) {
        let id = format!("vector_path_{:02}", self.path_counter);
        self.path_counter = self.path_counter.saturating_add(1);
        self.paths.push(VectorPathDraft {
            id: id.clone(),
            kind,
            points,
            brush: self.current_brush.clone(),
            export_simplify_epsilon: self.simplify_epsilon,
        });
        // New strokes are not auto-selected; brush changes should target the next stroke by default.
        self.selected_path = None;
        self.status_line = format!(
            "Created {id} ({}) with {}.",
            kind.label(),
            self.current_brush.summary()
        );
    }

    fn clear_current(&mut self) {
        self.current_pen_points.clear();
        self.current_freehand.clear();
        self.is_drawing_freehand = false;
        self.status_line = "Current in-progress path cleared.".to_string();
    }

    fn undo_last_action(&mut self) {
        if self.is_drawing_freehand || !self.current_freehand.is_empty() {
            self.current_freehand.clear();
            self.is_drawing_freehand = false;
            self.status_line = "Undo: cancelled current freehand stroke.".to_string();
            return;
        }

        if self.current_pen_points.pop().is_some() {
            self.status_line = format!(
                "Undo: removed last Point Tool point. {} point(s) remain.",
                self.current_pen_points.len()
            );
            return;
        }

        if let Some(path) = self.paths.pop() {
            self.selected_path = if self.paths.is_empty() {
                None
            } else {
                Some(self.paths.len().saturating_sub(1))
            };
            self.status_line = format!("Undo: removed {}.", path.id);
            return;
        }

        self.status_line = "Nothing to undo.".to_string();
    }

    fn clear_all_paths(&mut self) {
        self.paths.clear();
        self.selected_path = None;
        self.current_pen_points.clear();
        self.current_freehand.clear();
        self.is_drawing_freehand = false;
        self.status_line = "All vector paths cleared.".to_string();
    }

    fn delete_selected_path(&mut self) {
        let Some(idx) = self.selected_path else {
            self.status_line = "No selected path to delete.".to_string();
            return;
        };
        if idx >= self.paths.len() {
            self.selected_path = None;
            self.status_line = "No selected path to delete.".to_string();
            return;
        }
        let removed = self.paths.remove(idx);
        self.selected_path = if self.paths.is_empty() {
            None
        } else {
            Some(idx.min(self.paths.len().saturating_sub(1)))
        };
        self.status_line = format!("Deleted {}.", removed.id);
    }

    fn selected_path(&self) -> Option<&VectorPathDraft> {
        self.selected_path.and_then(|idx| self.paths.get(idx))
    }

    fn selected_path_d(&self) -> String {
        self.selected_path()
            .map(Self::path_d)
            .unwrap_or_else(|| "No selected path yet.".to_string())
    }

    fn selected_path_dsl(&self) -> String {
        self.selected_path()
            .map(Self::path_to_dsl)
            .unwrap_or_else(|| "<!-- No selected path yet. -->".to_string())
    }

    fn copy_selected_path_dsl(&mut self, cx: &mut Context<Self>) {
        let dsl = self.selected_path_dsl();
        cx.write_to_clipboard(ClipboardItem::new_string(dsl));
        self.status_line = "Copied selected Path DSL to clipboard.".to_string();
    }

    fn copy_group_dsl(&mut self, cx: &mut Context<Self>) {
        if self.paths.is_empty() {
            self.status_line = "No vector paths to copy.".to_string();
            return;
        }
        let group_id = if self.motionloom_group_id_text.trim().is_empty() {
            "vector_group_preview".to_string()
        } else {
            self.motionloom_group_id_text.trim().to_string()
        };
        let export_paths = self.paths_for_motionloom_export();
        let export = Self::vector_group_export(&group_id, &export_paths);
        let dsl = format!(
            "{}\n{}",
            Self::brush_defs_block(&export.brush_defs),
            export.group_block
        );
        cx.write_to_clipboard(ClipboardItem::new_string(dsl));
        self.status_line = format!("Copied compact group DSL for '{group_id}'.");
    }

    fn ensure_group_id_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.motionloom_group_id_input.is_some() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("group id, optional"));
        let initial = self.motionloom_group_id_text.clone();
        input.update(cx, |this, cx| {
            this.set_value(initial, window, cx);
        });
        let sub = cx.subscribe(&input, |this, input, ev, cx| match ev {
            InputEvent::Change | InputEvent::PressEnter { .. } => {
                this.motionloom_group_id_text = input.read(cx).value().to_string();
            }
            _ => {}
        });
        self.motionloom_group_id_input = Some(input);
        self.motionloom_group_id_input_sub = Some(sub);
    }

    fn ensure_canvas_bg_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.canvas_bg_picker.is_some() {
            return;
        }

        let initial = self.canvas_bg_color;
        let picker = cx.new(|cx| ColorPickerState::new(window, cx).default_value(initial));
        let sub = cx.subscribe(&picker, |this, _picker, ev, cx| {
            let ColorPickerEvent::Change(value) = ev;
            if let Some(color) = *value {
                this.canvas_bg_color = color;
                this.status_line = format!("Vector Lab background: {}.", color.to_hex());
                cx.notify();
            }
        });
        self.canvas_bg_picker = Some(picker);
        self.canvas_bg_picker_sub = Some(sub);
    }

    fn ensure_brush_color_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.brush_color_picker.is_some() {
            return;
        }

        let initial = Self::brush_color_hsla(&self.current_brush.color);
        let picker = cx.new(|cx| ColorPickerState::new(window, cx).default_value(initial));
        let sub = cx.subscribe(&picker, |this, _picker, ev, cx| {
            let ColorPickerEvent::Change(value) = ev;
            if let Some(color) = *value {
                this.set_brush_color(color);
                cx.notify();
            }
        });
        self.brush_color_picker = Some(picker);
        self.brush_color_picker_sub = Some(sub);
    }

    fn sync_brush_color_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = self.brush_color_picker.as_ref() {
            let target = Self::brush_color_hsla(&self.current_brush.color);
            let current = picker.read(cx).value();
            if current != Some(target) {
                picker.update(cx, |picker, cx| {
                    picker.set_value(target, window, cx);
                });
            }
        }
    }

    fn brush_color_hsla(value: &str) -> Hsla {
        Hsla::parse_hex(value).unwrap_or_else(|_| Hsla::from(rgb(0x111111)))
    }

    fn set_group_id_input_value(
        &mut self,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.motionloom_group_id_text = value.clone();
        if let Some(input) = self.motionloom_group_id_input.as_ref() {
            input.update(cx, |input, cx| {
                input.set_value(value, window, cx);
            });
        }
    }

    fn attach_to_motionloom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.paths.is_empty() {
            self.status_line = "Nothing to attach. Create at least one path first.".to_string();
            return;
        }
        if let Some(input) = self.motionloom_group_id_input.as_ref() {
            self.motionloom_group_id_text = input.read(cx).value().to_string();
        }
        let (width, height) = self.motionloom_export_size();
        let existing_script = self
            .global
            .read(cx)
            .motionloom_scene_script()
            .trim()
            .to_string();
        let existing_group_ids = Self::extract_group_ids(&existing_script);
        let typed_group_id = self.motionloom_group_id_text.trim();
        let group_id = if typed_group_id.is_empty() {
            Self::next_vector_group_id(&existing_group_ids)
        } else {
            typed_group_id.to_string()
        };
        let existed = existing_group_ids.iter().any(|id| id == &group_id);
        let export_paths = self.paths_for_motionloom_export();
        let export = Self::vector_group_export(&group_id, &export_paths);
        let background_color = self.canvas_bg_color.to_hex();
        let updated_script = Self::patch_motionloom_group_script(
            &existing_script,
            &group_id,
            &export,
            width.max(1),
            height.max(1),
            &background_color,
        );
        self.global.update(cx, |gs, cx| {
            gs.set_motionloom_scene_script(updated_script, true);
            gs.set_active_page(AppPage::MotionLoom);
            cx.notify();
        });
        self.set_group_id_input_value(group_id.clone(), window, cx);
        self.status_line = if existed {
            format!("Replaced existing MotionLoom group '{group_id}'.")
        } else {
            format!("Attached vector paths as new MotionLoom group '{group_id}'.")
        };
    }

    fn motionloom_export_size(&self) -> (u32, u32) {
        if let Some(layer) = self.reference_image.as_ref()
            && layer.source == ReferenceImageSource::VfxPreview
        {
            return (layer.width.max(1), layer.height.max(1));
        }
        self.current_board_size().unwrap_or((1080, 720))
    }

    fn paths_for_motionloom_export(&self) -> Vec<VectorPathDraft> {
        let Some(layer) = self.reference_image.as_ref() else {
            return self.paths.clone();
        };
        if layer.source != ReferenceImageSource::VfxPreview {
            return self.paths.clone();
        }
        let Some((offset_x, offset_y, scale)) = self.reference_image_canvas_transform(layer) else {
            return self.paths.clone();
        };
        if scale <= 0.0001 {
            return self.paths.clone();
        }

        self.paths
            .iter()
            .map(|path| {
                let mut mapped = path.clone();
                mapped.points = path
                    .points
                    .iter()
                    .map(|point| VectorPoint {
                        x: (point.x - offset_x) / scale,
                        y: (point.y - offset_y) / scale,
                        pressure: point.pressure,
                    })
                    .collect();
                mapped
            })
            .collect()
    }

    fn reference_image_canvas_transform(
        &self,
        layer: &ReferenceImageLayer,
    ) -> Option<(f32, f32, f32)> {
        let (container_w, container_h) = self.current_board_size()?;
        let container_w = container_w as f32;
        let container_h = container_h as f32;
        let frame_w = layer.width.max(1) as f32;
        let frame_h = layer.height.max(1) as f32;
        let fit_scale = (container_w / frame_w).min(container_h / frame_h);
        let scale = fit_scale * self.reference_zoom.max(0.05);
        let dest_w = frame_w * scale;
        let dest_h = frame_h * scale;
        Some((
            (container_w - dest_w) * 0.5 + self.reference_offset.x,
            (container_h - dest_h) * 0.5 + self.reference_offset.y,
            scale,
        ))
    }

    fn vector_group_export(group_id: &str, paths: &[VectorPathDraft]) -> VectorGroupExport {
        let mut brush_defs = Vec::<VectorBrushDef>::new();
        let mut brush_ids_by_key = HashMap::<String, String>::new();
        let mut path_brush_ids = Vec::<String>::with_capacity(paths.len());
        let brush_prefix = Self::brush_id_prefix(group_id);

        for path in paths {
            let key = Self::brush_key(&path.brush);
            let brush_id = if let Some(existing) = brush_ids_by_key.get(&key) {
                existing.clone()
            } else {
                let id = format!("{}_brush_{:02}", brush_prefix, brush_defs.len() + 1);
                brush_defs.push(VectorBrushDef {
                    id: id.clone(),
                    brush: path.brush.clone(),
                });
                brush_ids_by_key.insert(key, id.clone());
                id
            };
            path_brush_ids.push(brush_id);
        }

        let group_brush = if path_brush_ids
            .first()
            .is_some_and(|first| path_brush_ids.iter().all(|id| id == first))
        {
            path_brush_ids.first().cloned()
        } else {
            None
        };
        let path_lines = paths
            .iter()
            .zip(path_brush_ids.iter())
            .map(|(path, brush_id)| {
                Self::path_to_compact_dsl(path, group_brush.as_deref(), brush_id)
            })
            .collect::<Vec<_>>()
            .join("\n");
        VectorGroupExport {
            brush_defs,
            group_block: Self::vector_group_dsl(group_id, group_brush.as_deref(), &path_lines),
        }
    }

    fn vector_group_dsl(group_id: &str, brush: Option<&str>, paths: &str) -> String {
        let brush_attr = brush
            .map(|brush| format!(r#" brush="{}""#, Self::escape_attr(brush)))
            .unwrap_or_default();
        format!(
            r##"<Group id="{group_id}"{brush_attr} x="0" y="0" opacity="1">
{paths}
</Group>"##,
            group_id = Self::escape_attr(group_id),
            brush_attr = brush_attr,
            paths = Self::indent_block(paths, 2),
        )
    }

    fn new_vector_scene_script(
        group_id: &str,
        width: u32,
        height: u32,
        export: &VectorGroupExport,
        background_color: &str,
    ) -> String {
        let defs_block = Self::brush_defs_block(&export.brush_defs);
        let timeline_block = Self::vector_group_timeline_block(group_id, &export.group_block, "1s");
        let background_color = if background_color.trim().is_empty() {
            "#ffffff"
        } else {
            background_color.trim()
        };
        format!(
            r##"<Graph fps={{30}} duration="1s" size={{[{width},{height}]}}>
  <Background color="{background_color}" />

  <Scene id="vector_lab_trace">
{defs}
{timeline}
  </Scene>

  <Present from="vector_lab_trace" />
</Graph>"##,
            width = width.max(1),
            height = height.max(1),
            background_color = Self::escape_attr(background_color),
            defs = Self::indent_block(&defs_block, 4),
            timeline = Self::indent_block(&timeline_block, 4),
        )
    }

    fn patch_motionloom_group_script(
        existing_script: &str,
        group_id: &str,
        export: &VectorGroupExport,
        width: u32,
        height: u32,
        background_color: &str,
    ) -> String {
        let trimmed = existing_script.trim();
        if trimmed.is_empty() || !trimmed.contains("<Scene") || !trimmed.contains("</Scene>") {
            return Self::new_vector_scene_script(
                group_id,
                width,
                height,
                export,
                background_color,
            );
        }

        let duration = Self::motionloom_graph_duration(trimmed).unwrap_or_else(|| "1s".to_string());
        let trimmed = Self::patch_motionloom_brush_defs(trimmed, &export.brush_defs);
        let trimmed = Self::patch_motionloom_background_color(&trimmed, background_color);
        if let Some((start, end)) = Self::find_group_block_range(&trimmed, group_id) {
            let mut out = trimmed.to_string();
            if Self::range_inside_tag(&out, start, end, "Timeline") {
                out.replace_range(start..end, &export.group_block);
                return out;
            }

            out.replace_range(start..end, "");
            return Self::insert_vector_group_into_timeline(
                &out,
                group_id,
                &export.group_block,
                &duration,
            )
            .unwrap_or_else(|| {
                Self::new_vector_scene_script(group_id, width, height, export, background_color)
            });
        }

        Self::insert_vector_group_into_timeline(&trimmed, group_id, &export.group_block, &duration)
            .unwrap_or_else(|| {
                Self::new_vector_scene_script(group_id, width, height, export, background_color)
            })
    }

    fn patch_motionloom_background_color(script: &str, background_color: &str) -> String {
        let color = if background_color.trim().is_empty() {
            "#ffffff"
        } else {
            background_color.trim()
        };
        let escaped = Self::escape_attr(color);
        if let Some(start) = Self::find_ascii_case_insensitive(script, "<Background", 0)
            && let Some(end) = Self::find_tag_end(script, start)
        {
            let tag = &script[start..=end];
            let replacement = if let Some(color_pos) = Self::find_attr_value_range(tag, "color") {
                let mut next_tag = tag.to_string();
                next_tag.replace_range(color_pos.0..color_pos.1, &escaped);
                next_tag
            } else {
                let insert_at = tag
                    .len()
                    .saturating_sub(if tag.ends_with("/>") { 2 } else { 1 });
                let mut next_tag = tag.to_string();
                next_tag.insert_str(insert_at, &format!(r#" color="{escaped}""#));
                next_tag
            };
            let mut out = script.to_string();
            out.replace_range(start..=end, &replacement);
            return out;
        }

        if let Some(graph_start) = Self::find_ascii_case_insensitive(script, "<Graph", 0)
            && let Some(graph_end) = Self::find_tag_end(script, graph_start)
        {
            let mut out = String::with_capacity(script.len() + escaped.len() + 32);
            out.push_str(&script[..=graph_end]);
            out.push_str(&format!("\n  <Background color=\"{escaped}\" />"));
            out.push_str(&script[graph_end.saturating_add(1)..]);
            return out;
        }

        script.to_string()
    }

    fn vector_group_timeline_block(group_id: &str, group_block: &str, duration: &str) -> String {
        let track = Self::vector_group_track_block(group_id, group_block, duration);
        format!(
            r##"<Timeline>
{track}
</Timeline>"##,
            track = Self::indent_block(&track, 2),
        )
    }

    fn vector_group_track_block(group_id: &str, group_block: &str, duration: &str) -> String {
        let track_id = Self::vector_track_id(group_id);
        format!(
            r##"<Track id="{track_id}" space="world" z="1000">
  <Sequence from="0s" duration="{duration}" out="hold">
    <Layer>
{group}
    </Layer>
  </Sequence>
</Track>"##,
            track_id = Self::escape_attr(&track_id),
            duration = Self::escape_attr(duration),
            group = Self::indent_block(group_block, 6),
        )
    }

    fn insert_vector_group_into_timeline(
        script: &str,
        group_id: &str,
        group_block: &str,
        duration: &str,
    ) -> Option<String> {
        let track = Self::vector_group_track_block(group_id, group_block, duration);
        if let Some(timeline_close) = Self::find_ascii_case_insensitive(script, "</Timeline>", 0) {
            let mut out = String::with_capacity(script.len() + track.len() + 16);
            out.push_str(&script[..timeline_close]);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&Self::indent_block(&track, 6));
            out.push('\n');
            out.push_str(&script[timeline_close..]);
            return Some(out);
        }

        let scene_close = Self::find_ascii_case_insensitive(script, "</Scene>", 0)?;
        let timeline = Self::vector_group_timeline_block(group_id, group_block, duration);
        let mut out = String::with_capacity(script.len() + timeline.len() + 16);
        out.push_str(&script[..scene_close]);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&Self::indent_block(&timeline, 4));
        out.push('\n');
        out.push_str(&script[scene_close..]);
        Some(out)
    }

    fn vector_track_id(group_id: &str) -> String {
        let sanitized = group_id
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_string();
        if sanitized.is_empty() {
            "vector_lab_track".to_string()
        } else {
            format!("{sanitized}_track")
        }
    }

    fn motionloom_graph_duration(script: &str) -> Option<String> {
        let graph_start = Self::find_ascii_case_insensitive(script, "<Graph", 0)?;
        let graph_end = Self::find_tag_end(script, graph_start)?;
        Self::extract_attr(&script[graph_start..=graph_end], "duration")
    }

    fn range_inside_tag(script: &str, start: usize, end: usize, tag_name: &str) -> bool {
        let open_prefix = format!("<{tag_name}");
        let close_tag = format!("</{tag_name}>");
        let mut cursor = 0usize;
        while let Some(open) = Self::find_ascii_case_insensitive(script, &open_prefix, cursor) {
            let Some(open_end) = Self::find_tag_end(script, open) else {
                return false;
            };
            let Some(close) =
                Self::find_ascii_case_insensitive(script, &close_tag, open_end.saturating_add(1))
            else {
                return false;
            };
            let close_end = close.saturating_add(close_tag.len());
            if open <= start && end <= close_end {
                return true;
            }
            cursor = close_end;
        }
        false
    }

    fn brush_key(brush: &BrushSettings) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            brush.preset.dsl_style(),
            brush.color,
            Self::format_coord(brush.stroke_width),
            Self::format_coord(brush.opacity),
            Self::format_coord(brush.roughness),
            brush.copies,
            Self::format_coord(brush.texture_strength),
            brush.bristle_count,
            Self::format_coord(brush.pressure_min),
            Self::format_coord(brush.pressure_curve),
        )
    }

    fn brush_id_prefix(group_id: &str) -> String {
        let sanitized = group_id
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_string();
        if sanitized.is_empty() {
            "vector_group".to_string()
        } else {
            format!("vector_{sanitized}")
        }
    }

    fn brush_to_dsl(def: &VectorBrushDef) -> String {
        format!(
            r##"<Brush id="{}"
       stroke="{}"
       strokeWidth="{}"
       strokeStyle="{}"
       strokeRoughness="{}"
       strokeCopies="{}"
       strokeTexture="{}"
       strokeBristles="{}"
       strokePressure="auto"
       strokePressureMin="{}"
       strokePressureCurve="{}"
       opacity="{}"
       lineCap="round"
       lineJoin="round"
       fill="none" />"##,
            Self::escape_attr(&def.id),
            Self::escape_attr(&def.brush.color),
            Self::format_coord(def.brush.stroke_width),
            def.brush.preset.dsl_style(),
            Self::format_coord(def.brush.roughness),
            def.brush.copies,
            Self::format_coord(def.brush.texture_strength),
            def.brush.bristle_count,
            Self::format_coord(def.brush.pressure_min),
            Self::format_coord(def.brush.pressure_curve),
            Self::format_coord(def.brush.opacity),
        )
    }

    fn brush_defs_block(brush_defs: &[VectorBrushDef]) -> String {
        let brushes = brush_defs
            .iter()
            .map(Self::brush_to_dsl)
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            r##"<Defs>
{brushes}
</Defs>"##,
            brushes = Self::indent_block(&brushes, 2)
        )
    }

    fn patch_motionloom_brush_defs(script: &str, brush_defs: &[VectorBrushDef]) -> String {
        if brush_defs.is_empty() {
            return script.to_string();
        }
        let mut out = script.to_string();
        let mut missing = Vec::<VectorBrushDef>::new();
        for def in brush_defs {
            if let Some((start, end)) = Self::find_brush_block_range(&out, &def.id) {
                out.replace_range(start..end, &Self::brush_to_dsl(def));
            } else {
                missing.push(def.clone());
            }
        }
        if missing.is_empty() {
            return out;
        }

        let missing_block = missing
            .iter()
            .map(Self::brush_to_dsl)
            .map(|brush| Self::indent_block(&brush, 6))
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(defs_close) = Self::find_ascii_case_insensitive(&out, "</Defs>", 0) {
            let mut next = String::with_capacity(out.len() + missing_block.len() + 2);
            next.push_str(&out[..defs_close]);
            if !next.ends_with('\n') {
                next.push('\n');
            }
            next.push_str(&missing_block);
            next.push('\n');
            next.push_str(&out[defs_close..]);
            return next;
        }

        if let Some(scene_start) = Self::find_ascii_case_insensitive(&out, "<Scene", 0)
            && let Some(scene_open_end) = Self::find_tag_end(&out, scene_start)
        {
            let defs_block = Self::indent_block(&Self::brush_defs_block(&missing), 4);
            let insert_at = scene_open_end.saturating_add(1);
            let mut next = String::with_capacity(out.len() + defs_block.len() + 2);
            next.push_str(&out[..insert_at]);
            next.push('\n');
            next.push_str(&defs_block);
            next.push_str(&out[insert_at..]);
            return next;
        }

        out
    }

    fn find_brush_block_range(script: &str, brush_id: &str) -> Option<(usize, usize)> {
        let mut cursor = 0usize;
        while let Some(start) = Self::find_ascii_case_insensitive(script, "<Brush", cursor) {
            let tag_end = Self::find_tag_end(script, start)?;
            let tag = &script[start..=tag_end];
            if Self::extract_attr(tag, "id").as_deref() == Some(brush_id) {
                return Some((start, tag_end.saturating_add(1)));
            }
            cursor = tag_end.saturating_add(1);
        }
        None
    }

    fn current_board_size(&self) -> Option<(u32, u32)> {
        let bounds = *self.board_bounds.lock().ok()?.as_ref()?;
        let width = (bounds.size.width / px(1.0)).round().max(1.0) as u32;
        let height = (bounds.size.height / px(1.0)).round().max(1.0) as u32;
        Some((width, height))
    }

    fn indent_block(text: &str, spaces: usize) -> String {
        let prefix = " ".repeat(spaces);
        text.lines()
            .map(|line| format!("{prefix}{line}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn escape_attr(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn next_vector_group_id(existing_ids: &[String]) -> String {
        let mut index = 1usize;
        loop {
            let candidate = format!("vector_group_{index:02}");
            if !existing_ids.iter().any(|id| id == &candidate) {
                return candidate;
            }
            index = index.saturating_add(1);
        }
    }

    fn extract_group_ids(script: &str) -> Vec<String> {
        let mut ids = Vec::new();
        let mut cursor = 0usize;
        while let Some(start) = Self::find_ascii_case_insensitive(script, "<Group", cursor) {
            let Some(tag_end) = Self::find_tag_end(script, start) else {
                break;
            };
            let tag = &script[start..=tag_end];
            if let Some(id) = Self::extract_attr(tag, "id") {
                if !ids.iter().any(|existing| existing == &id) {
                    ids.push(id);
                }
            }
            cursor = tag_end.saturating_add(1);
        }
        ids
    }

    fn extract_attr(tag: &str, name: &str) -> Option<String> {
        Self::find_attr_value_range(tag, name).map(|(start, end)| tag[start..end].to_string())
    }

    fn find_attr_value_range(tag: &str, name: &str) -> Option<(usize, usize)> {
        let bytes = tag.as_bytes();
        let name_bytes = name.as_bytes();
        let mut cursor = 0usize;
        while cursor + name_bytes.len() < bytes.len() {
            let rel = tag[cursor..].find(name)?;
            let name_start = cursor + rel;
            let name_end = name_start + name_bytes.len();
            let before_ok = name_start == 0
                || bytes
                    .get(name_start.saturating_sub(1))
                    .is_some_and(|b| b.is_ascii_whitespace() || *b == b'<');
            let after_ok = bytes
                .get(name_end)
                .is_some_and(|b| b.is_ascii_whitespace() || *b == b'=');
            if !before_ok || !after_ok {
                cursor = name_end;
                continue;
            }

            let mut eq = name_end;
            while bytes.get(eq).is_some_and(|b| b.is_ascii_whitespace()) {
                eq += 1;
            }
            if bytes.get(eq) != Some(&b'=') {
                cursor = name_end;
                continue;
            }

            let mut value_start = eq + 1;
            while bytes
                .get(value_start)
                .is_some_and(|b| b.is_ascii_whitespace())
            {
                value_start += 1;
            }
            let quote = *bytes.get(value_start)?;
            if quote != b'"' && quote != b'\'' {
                cursor = value_start.saturating_add(1);
                continue;
            }
            let attr_value_start = value_start + 1;
            let value_end = tag[attr_value_start..]
                .find(quote as char)
                .map(|offset| attr_value_start + offset)?;
            return Some((attr_value_start, value_end));
        }
        None
    }

    fn find_tag_end(script: &str, tag_start: usize) -> Option<usize> {
        let mut quote: Option<u8> = None;
        for (offset, byte) in script.as_bytes().iter().enumerate().skip(tag_start) {
            match (quote, *byte) {
                (None, b'"') | (None, b'\'') => quote = Some(*byte),
                (Some(q), b) if b == q => quote = None,
                (None, b'>') => return Some(offset),
                _ => {}
            }
        }
        None
    }

    fn find_ascii_case_insensitive(haystack: &str, needle: &str, from: usize) -> Option<usize> {
        if from >= haystack.len() {
            return None;
        }
        let lower_haystack = haystack[from..].to_ascii_lowercase();
        let lower_needle = needle.to_ascii_lowercase();
        lower_haystack
            .find(lower_needle.as_str())
            .map(|offset| from + offset)
    }

    fn find_group_block_range(script: &str, group_id: &str) -> Option<(usize, usize)> {
        let mut cursor = 0usize;
        while let Some(start) = Self::find_ascii_case_insensitive(script, "<Group", cursor) {
            let tag_end = Self::find_tag_end(script, start)?;
            let tag = &script[start..=tag_end];
            if Self::extract_attr(tag, "id").as_deref() == Some(group_id) {
                if tag.trim_end().ends_with("/>") {
                    return Some((start, tag_end.saturating_add(1)));
                }
                let end = Self::find_matching_group_end(script, start)?;
                return Some((start, end));
            }
            cursor = tag_end.saturating_add(1);
        }
        None
    }

    fn find_matching_group_end(script: &str, group_start: usize) -> Option<usize> {
        let mut cursor = Self::find_tag_end(script, group_start)?.saturating_add(1);
        let mut depth = 1usize;
        while cursor < script.len() {
            let next_open = Self::find_ascii_case_insensitive(script, "<Group", cursor);
            let next_close = Self::find_ascii_case_insensitive(script, "</Group>", cursor);
            match (next_open, next_close) {
                (Some(open), Some(close)) if open < close => {
                    depth = depth.saturating_add(1);
                    cursor = Self::find_tag_end(script, open)?.saturating_add(1);
                }
                (_, Some(close)) => {
                    depth = depth.saturating_sub(1);
                    let end = Self::find_tag_end(script, close)?.saturating_add(1);
                    if depth == 0 {
                        return Some(end);
                    }
                    cursor = end;
                }
                _ => return None,
            }
        }
        None
    }

    fn import_motionloom_group(
        &mut self,
        group_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let script = self
            .global
            .read(cx)
            .motionloom_scene_script()
            .trim()
            .to_string();
        let Some((start, end)) = Self::find_group_block_range(&script, &group_id) else {
            self.status_line = format!("MotionLoom group '{group_id}' was not found.");
            return;
        };
        let group_block = &script[start..end];
        let brush_defs = Self::brush_defs_from_script(&script);
        let imported = Self::paths_from_group_block_with_brushes(group_block, &brush_defs);
        if imported.is_empty() {
            self.paths.clear();
            self.selected_path = None;
            self.current_pen_points.clear();
            self.current_freehand.clear();
            self.is_drawing_freehand = false;
            self.set_group_id_input_value(group_id.clone(), window, cx);
            self.group_picker_open = false;
            self.status_line = format!(
                "Selected MotionLoom group '{group_id}'. It has no editable Path nodes yet; new Vector Lab strokes will replace it on attach."
            );
            return;
        }
        let max_index = imported
            .iter()
            .filter_map(|path| path.id.strip_prefix("vector_path_"))
            .filter_map(|suffix| suffix.parse::<usize>().ok())
            .max()
            .unwrap_or(imported.len());
        self.paths = imported;
        // Require an explicit path-list click before brush controls edit an imported path.
        self.selected_path = None;
        self.current_pen_points.clear();
        self.current_freehand.clear();
        self.is_drawing_freehand = false;
        self.path_counter = max_index.saturating_add(1).max(self.paths.len() + 1);
        self.current_brush = BrushSettings::default();
        self.set_group_id_input_value(group_id.clone(), window, cx);
        self.group_picker_open = false;
        self.status_line = format!(
            "Loaded MotionLoom group '{group_id}' into Vector Lab: {} editable path(s).",
            self.paths.len()
        );
    }

    fn paths_from_group_block(group_block: &str) -> Vec<VectorPathDraft> {
        Self::paths_from_group_block_with_brushes(group_block, &HashMap::new())
    }

    fn paths_from_group_block_with_brushes(
        group_block: &str,
        brush_defs: &HashMap<String, BrushSettings>,
    ) -> Vec<VectorPathDraft> {
        Self::paths_from_group_block_with_inherited(group_block, brush_defs, None)
    }

    fn paths_from_group_block_with_inherited(
        group_block: &str,
        brush_defs: &HashMap<String, BrushSettings>,
        inherited_brush: Option<&BrushSettings>,
    ) -> Vec<VectorPathDraft> {
        let mut paths = Vec::new();
        let Some(group_start) = Self::find_ascii_case_insensitive(group_block, "<Group", 0) else {
            return paths;
        };
        let Some(group_tag_end) = Self::find_tag_end(group_block, group_start) else {
            return paths;
        };
        let group_tag = &group_block[group_start..=group_tag_end];
        let group_brush = Self::extract_attr(group_tag, "brush")
            .and_then(|id| brush_defs.get(&id))
            .cloned()
            .or_else(|| inherited_brush.cloned());
        let body_start = group_tag_end.saturating_add(1);
        let body_end = group_block
            .rfind("</Group>")
            .unwrap_or(group_block.len())
            .max(body_start);
        Self::collect_paths_from_group_body(
            &group_block[body_start..body_end],
            brush_defs,
            group_brush.as_ref(),
            &mut paths,
        );
        paths
    }

    fn collect_paths_from_group_body(
        body: &str,
        brush_defs: &HashMap<String, BrushSettings>,
        inherited_brush: Option<&BrushSettings>,
        paths: &mut Vec<VectorPathDraft>,
    ) {
        let mut cursor = 0usize;
        while cursor < body.len() {
            let next_path = Self::find_ascii_case_insensitive(body, "<Path", cursor);
            let next_group = Self::find_ascii_case_insensitive(body, "<Group", cursor);
            match (next_path, next_group) {
                (Some(path_start), Some(group_start)) if group_start < path_start => {
                    let Some(group_end) = Self::find_matching_group_end(body, group_start) else {
                        break;
                    };
                    let nested = &body[group_start..group_end];
                    paths.extend(Self::paths_from_group_block_with_inherited(
                        nested,
                        brush_defs,
                        inherited_brush,
                    ));
                    cursor = group_end;
                }
                (Some(path_start), _) => {
                    let Some(tag_end) = Self::find_tag_end(body, path_start) else {
                        break;
                    };
                    let tag = &body[path_start..=tag_end];
                    let path_brush = Self::extract_attr(tag, "brush")
                        .and_then(|id| brush_defs.get(&id))
                        .or(inherited_brush);
                    if let Some(path) = Self::path_from_dsl_tag(tag, paths.len() + 1, path_brush) {
                        paths.push(path);
                    }
                    cursor = tag_end.saturating_add(1);
                }
                (None, Some(group_start)) => {
                    let Some(group_end) = Self::find_matching_group_end(body, group_start) else {
                        break;
                    };
                    let nested = &body[group_start..group_end];
                    paths.extend(Self::paths_from_group_block_with_inherited(
                        nested,
                        brush_defs,
                        inherited_brush,
                    ));
                    cursor = group_end;
                }
                (None, None) => break,
            }
        }
    }

    fn group_path_count(script: &str, group_id: &str) -> usize {
        Self::find_group_block_range(script, group_id)
            .map(|(start, end)| Self::paths_from_group_block(&script[start..end]).len())
            .unwrap_or(0)
    }

    fn brush_defs_from_script(script: &str) -> HashMap<String, BrushSettings> {
        let mut defs = HashMap::<String, BrushSettings>::new();
        let mut cursor = 0usize;
        while let Some(start) = Self::find_ascii_case_insensitive(script, "<Brush", cursor) {
            let Some(tag_end) = Self::find_tag_end(script, start) else {
                break;
            };
            let tag = &script[start..=tag_end];
            if let Some(id) = Self::extract_attr(tag, "id") {
                defs.insert(id, Self::brush_from_dsl_attrs(tag, None));
            }
            cursor = tag_end.saturating_add(1);
        }
        defs
    }

    fn path_from_dsl_tag(
        tag: &str,
        fallback_index: usize,
        inherited_brush: Option<&BrushSettings>,
    ) -> Option<VectorPathDraft> {
        let d = Self::extract_attr(tag, "d")?;
        let points = Self::points_from_path_d(&d);
        if points.len() < 2 {
            return None;
        }
        let brush = Self::brush_from_dsl_attrs(tag, inherited_brush);
        Some(VectorPathDraft {
            id: Self::extract_attr(tag, "id")
                .unwrap_or_else(|| format!("vector_path_{fallback_index:02}")),
            kind: VectorPathKind::Freehand,
            points,
            brush,
            export_simplify_epsilon: 0.2,
        })
    }

    fn brush_from_dsl_attrs(tag: &str, inherited_brush: Option<&BrushSettings>) -> BrushSettings {
        let mut brush = if let Some(style) =
            Self::extract_attr(tag, "strokeStyle").or_else(|| Self::extract_attr(tag, "style"))
        {
            BrushSettings::for_preset(Self::brush_preset_from_style(&style))
        } else {
            inherited_brush
                .cloned()
                .unwrap_or_else(|| BrushSettings::for_preset(BrushPreset::Sketch))
        };
        if let Some(stroke) =
            Self::extract_attr(tag, "stroke").or_else(|| Self::extract_attr(tag, "color"))
        {
            brush.color = stroke;
        }
        if let Some(width) =
            Self::extract_attr(tag, "strokeWidth").and_then(|value| value.parse::<f32>().ok())
        {
            brush.stroke_width = width;
        }
        if let Some(opacity) =
            Self::extract_attr(tag, "opacity").and_then(|value| value.parse::<f32>().ok())
        {
            brush.opacity = opacity;
        }
        if let Some(roughness) =
            Self::extract_attr(tag, "strokeRoughness").and_then(|value| value.parse::<f32>().ok())
        {
            brush.roughness = roughness;
        }
        if let Some(copies) =
            Self::extract_attr(tag, "strokeCopies").and_then(|value| value.parse::<u32>().ok())
        {
            brush.copies = copies.clamp(1, 12);
        }
        if let Some(texture) =
            Self::extract_attr(tag, "strokeTexture").and_then(|value| value.parse::<f32>().ok())
        {
            brush.texture_strength = texture.clamp(0.0, 1.0);
        }
        if let Some(bristles) =
            Self::extract_attr(tag, "strokeBristles").and_then(|value| value.parse::<u32>().ok())
        {
            brush.bristle_count = bristles.min(24);
        }
        if let Some(pressure_min) =
            Self::extract_attr(tag, "strokePressureMin").and_then(|value| value.parse::<f32>().ok())
        {
            brush.pressure_min = pressure_min.clamp(0.05, 1.0);
        }
        if let Some(pressure_curve) = Self::extract_attr(tag, "strokePressureCurve")
            .and_then(|value| value.parse::<f32>().ok())
        {
            brush.pressure_curve = pressure_curve.clamp(0.25, 3.0);
        }
        brush
    }

    fn brush_preset_from_style(style: &str) -> BrushPreset {
        match style.trim().to_ascii_lowercase().as_str() {
            "ink" => BrushPreset::CleanInk,
            "pencil" => BrushPreset::Pencil,
            "rough" => BrushPreset::Rough,
            "charcoal" => BrushPreset::Charcoal,
            "marker" => BrushPreset::Marker,
            "hairline" => BrushPreset::Hairline,
            _ => BrushPreset::Sketch,
        }
    }

    fn points_from_path_d(d: &str) -> Vec<VectorPoint> {
        let tokens = Self::path_d_tokens(d);
        let mut points = Vec::new();
        let mut cursor = 0usize;
        let mut command = 'M';
        while cursor < tokens.len() {
            if tokens[cursor].len() == 1 {
                let ch = tokens[cursor].chars().next().unwrap_or(' ');
                if ch.is_ascii_alphabetic() {
                    command = ch;
                    cursor += 1;
                    continue;
                }
            }
            match command {
                'M' | 'L' => {
                    if cursor + 1 >= tokens.len() {
                        break;
                    }
                    if let (Ok(x), Ok(y)) = (
                        tokens[cursor].parse::<f32>(),
                        tokens[cursor + 1].parse::<f32>(),
                    ) {
                        points.push(VectorPoint::new(x, y));
                    }
                    cursor += 2;
                    command = if command == 'M' { 'L' } else { command };
                }
                'C' => {
                    if cursor + 5 >= tokens.len() {
                        break;
                    }
                    if let (Ok(x), Ok(y)) = (
                        tokens[cursor + 4].parse::<f32>(),
                        tokens[cursor + 5].parse::<f32>(),
                    ) {
                        points.push(VectorPoint::new(x, y));
                    }
                    cursor += 6;
                }
                _ => {
                    cursor += 1;
                }
            }
        }
        points
    }

    fn path_d_tokens(d: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current = String::new();
        for ch in d.chars() {
            if ch.is_ascii_alphabetic() {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(ch.to_string());
            } else if ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '+' {
                if (ch == '-' || ch == '+') && !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                current.push(ch);
            } else if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }
        tokens
    }

    fn path_to_dsl(path: &VectorPathDraft) -> String {
        format!(
            r##"<Path id="{}"
      d="{}"
      stroke="{}"
      strokeWidth="{}"
      strokeStyle="{}"
      strokeRoughness="{}"
      strokeCopies="{}"
      strokeTexture="{}"
      strokeBristles="{}"
      strokePressure="auto"
      strokePressureMin="{}"
      strokePressureCurve="{}"
      opacity="{}"
      lineCap="round"
      lineJoin="round"
      fill="none" />"##,
            path.id,
            Self::path_d(path),
            path.brush.color.as_str(),
            Self::format_coord(path.brush.stroke_width),
            path.brush.preset.dsl_style(),
            Self::format_coord(path.brush.roughness),
            path.brush.copies,
            Self::format_coord(path.brush.texture_strength),
            path.brush.bristle_count,
            Self::format_coord(path.brush.pressure_min),
            Self::format_coord(path.brush.pressure_curve),
            Self::format_coord(path.brush.opacity)
        )
    }

    fn path_to_compact_dsl(
        path: &VectorPathDraft,
        group_brush: Option<&str>,
        path_brush: &str,
    ) -> String {
        let brush_attr = if group_brush == Some(path_brush) {
            String::new()
        } else {
            format!(r#" brush="{}""#, Self::escape_attr(path_brush))
        };
        format!(
            r##"<Path id="{}"{} d="{}" />"##,
            Self::escape_attr(&path.id),
            brush_attr,
            Self::escape_attr(&Self::path_d(path)),
        )
    }

    fn path_d(path: &VectorPathDraft) -> String {
        match path.kind {
            VectorPathKind::Pen => Self::smooth_path_d(&path.points),
            VectorPathKind::Freehand => {
                let export_points =
                    Self::simplify_points(&path.points, path.export_simplify_epsilon);
                Self::polyline_path_d(&export_points)
            }
        }
    }

    fn smooth_path_d(points: &[VectorPoint]) -> String {
        if points.is_empty() {
            return String::new();
        }
        if points.len() == 1 {
            return format!(
                "M {} {}",
                Self::format_coord(points[0].x),
                Self::format_coord(points[0].y)
            );
        }

        let mut out = format!(
            "M {} {}",
            Self::format_coord(points[0].x),
            Self::format_coord(points[0].y)
        );
        for i in 0..points.len() - 1 {
            let p0 = if i == 0 { points[i] } else { points[i - 1] };
            let p1 = points[i];
            let p2 = points[i + 1];
            let p3 = if i + 2 < points.len() {
                points[i + 2]
            } else {
                p2
            };
            let c1 = VectorPoint::new(p1.x + (p2.x - p0.x) / 6.0, p1.y + (p2.y - p0.y) / 6.0);
            let c2 = VectorPoint::new(p2.x - (p3.x - p1.x) / 6.0, p2.y - (p3.y - p1.y) / 6.0);
            out.push_str(&format!(
                " C {} {}, {} {}, {} {}",
                Self::format_coord(c1.x),
                Self::format_coord(c1.y),
                Self::format_coord(c2.x),
                Self::format_coord(c2.y),
                Self::format_coord(p2.x),
                Self::format_coord(p2.y)
            ));
        }
        out
    }

    fn polyline_path_d(points: &[VectorPoint]) -> String {
        if points.is_empty() {
            return String::new();
        }
        let mut out = format!(
            "M {} {}",
            Self::format_coord(points[0].x),
            Self::format_coord(points[0].y)
        );
        for point in &points[1..] {
            out.push_str(&format!(
                " L {} {}",
                Self::format_coord(point.x),
                Self::format_coord(point.y)
            ));
        }
        out
    }

    fn format_coord(value: f32) -> String {
        let rounded = (value * 10.0).round() / 10.0;
        if (rounded - rounded.round()).abs() < 0.05 {
            format!("{:.0}", rounded)
        } else {
            format!("{:.1}", rounded)
        }
    }

    fn simplify_points(points: &[VectorPoint], epsilon: f32) -> Vec<VectorPoint> {
        if points.len() <= 2 {
            return points.to_vec();
        }
        let mut keep = vec![false; points.len()];
        keep[0] = true;
        keep[points.len() - 1] = true;
        Self::rdp_mark(points, 0, points.len() - 1, epsilon, &mut keep);
        points
            .iter()
            .zip(keep)
            .filter_map(|(point, keep)| keep.then_some(*point))
            .collect()
    }

    fn rdp_mark(points: &[VectorPoint], start: usize, end: usize, epsilon: f32, keep: &mut [bool]) {
        if end <= start + 1 {
            return;
        }
        let a = points[start];
        let b = points[end];
        let mut max_dist = 0.0;
        let mut max_idx = start;
        for (idx, point) in points.iter().enumerate().take(end).skip(start + 1) {
            let dist = Self::point_line_distance(*point, a, b);
            if dist > max_dist {
                max_dist = dist;
                max_idx = idx;
            }
        }
        if max_dist > epsilon {
            keep[max_idx] = true;
            Self::rdp_mark(points, start, max_idx, epsilon, keep);
            Self::rdp_mark(points, max_idx, end, epsilon, keep);
        }
    }

    fn point_line_distance(point: VectorPoint, a: VectorPoint, b: VectorPoint) -> f32 {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len_sq = dx * dx + dy * dy;
        if len_sq <= f32::EPSILON {
            return point.distance_to(a);
        }
        let t = (((point.x - a.x) * dx + (point.y - a.y) * dy) / len_sq).clamp(0.0, 1.0);
        let projection = VectorPoint::new(a.x + t * dx, a.y + t * dy);
        point.distance_to(projection)
    }

    fn draw_snapshot(bounds: Bounds<Pixels>, snapshot: &VectorCanvasSnapshot, window: &mut Window) {
        if let Ok(mut board_bounds) = snapshot.bounds.lock() {
            *board_bounds = Some(bounds);
        }

        Self::paint_grid(bounds, window);
        for (idx, path) in snapshot.paths.iter().enumerate() {
            let selected = snapshot.selected_path == Some(idx);
            Self::paint_vector_path(bounds, path, selected, window);
        }
        if !snapshot.current_pen_points.is_empty() {
            let draft = VectorPathDraft {
                id: "current_pen".to_string(),
                kind: VectorPathKind::Pen,
                points: snapshot.current_pen_points.clone(),
                brush: snapshot.current_brush.clone(),
                export_simplify_epsilon: 0.0,
            };
            Self::paint_vector_path(bounds, &draft, false, window);
            Self::paint_points(
                bounds,
                &snapshot.current_pen_points,
                rgba(0x79c7ffff),
                window,
            );
        }
        if !snapshot.current_freehand.is_empty() {
            let draft = VectorPathDraft {
                id: "current_freehand".to_string(),
                kind: VectorPathKind::Freehand,
                points: snapshot.current_freehand.clone(),
                brush: snapshot.current_brush.clone(),
                export_simplify_epsilon: 0.0,
            };
            Self::paint_vector_path(bounds, &draft, true, window);
        }
    }

    fn paint_grid(bounds: Bounds<Pixels>, window: &mut Window) {
        let step = 48.0;
        let width = bounds.size.width / px(1.0);
        let height = bounds.size.height / px(1.0);
        let mut builder = PathBuilder::stroke(px(1.0));
        let mut x = 0.0;
        while x <= width {
            builder.move_to(point(bounds.origin.x + px(x), bounds.origin.y));
            builder.line_to(point(
                bounds.origin.x + px(x),
                bounds.origin.y + bounds.size.height,
            ));
            x += step;
        }
        let mut y = 0.0;
        while y <= height {
            builder.move_to(point(bounds.origin.x, bounds.origin.y + px(y)));
            builder.line_to(point(
                bounds.origin.x + bounds.size.width,
                bounds.origin.y + px(y),
            ));
            y += step;
        }
        if let Ok(path) = builder.build() {
            window.paint_path(path, rgba(0xffffff10));
        }
    }

    fn paint_vector_path(
        bounds: Bounds<Pixels>,
        path: &VectorPathDraft,
        selected: bool,
        window: &mut Window,
    ) {
        if path.points.len() < 2 {
            return;
        }
        if path.kind == VectorPathKind::Freehand {
            Self::paint_freehand_brush_path(bounds, path, window);
            return;
        }
        let copies = path.brush.copies.clamp(1, 12);
        for copy_ix in 0..copies {
            let jitter = Self::brush_jitter(&path.id, copy_ix, &path.brush);
            let width_jitter = Self::brush_width_jitter(&path.id, copy_ix, &path.brush);
            let width = (path.brush.stroke_width * width_jitter).max(0.3);
            let color = Self::brush_preview_color(&path.brush, copy_ix, copies);
            Self::paint_vector_path_once(bounds, path, width, color, jitter, window);
        }
        if selected {
            Self::paint_vector_path_once(
                bounds,
                path,
                path.brush.stroke_width + 2.0,
                rgba(0x79c7ffcc),
                VectorPoint::default(),
                window,
            );
        }
    }

    fn paint_freehand_brush_path(
        bounds: Bounds<Pixels>,
        path: &VectorPathDraft,
        window: &mut Window,
    ) {
        let copies = path.brush.copies.clamp(1, 12);
        for copy_ix in 0..copies {
            let jitter = Self::brush_jitter(&path.id, copy_ix, &path.brush);
            let color = Self::brush_preview_color(&path.brush, copy_ix, copies);
            Self::paint_pressure_segments(bounds, path, copy_ix, jitter, color, window);
            Self::paint_bristle_strands(bounds, path, copy_ix, window);
            Self::paint_texture_stamps(bounds, path, copy_ix, window);
        }
    }

    fn paint_pressure_segments(
        bounds: Bounds<Pixels>,
        path: &VectorPathDraft,
        copy_ix: u32,
        jitter: VectorPoint,
        color: gpui::Rgba,
        window: &mut Window,
    ) {
        let stride = (path.points.len() / 900).max(1);
        for i in (0..path.points.len() - 1).step_by(stride) {
            let end = (i + stride).min(path.points.len() - 1);
            let a = path.points[i];
            let b = path.points[end];
            if a.distance_to(b) < 0.1 {
                continue;
            }
            let pressure = (a.pressure + b.pressure) * 0.5;
            let pressure_width = Self::brush_pressure_scale(&path.brush, pressure);
            let seed = Self::stable_brush_seed(&path.id, copy_ix ^ i as u32);
            let local_width_noise = 1.0
                + Self::hash_unit(seed ^ 0xa511_e9b3)
                    * path.brush.texture_strength
                    * match path.brush.preset {
                        BrushPreset::Marker | BrushPreset::CleanInk | BrushPreset::BoldInk => 0.04,
                        BrushPreset::Charcoal => 0.26,
                        _ => 0.14,
                    };
            let width = (path.brush.stroke_width * pressure_width * local_width_noise).max(0.25);
            let mut builder = PathBuilder::stroke(px(width));
            builder.move_to(Self::canvas_point_offset(bounds, a, jitter));
            builder.line_to(Self::canvas_point_offset(bounds, b, jitter));
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        }
    }

    fn paint_bristle_strands(
        bounds: Bounds<Pixels>,
        path: &VectorPathDraft,
        copy_ix: u32,
        window: &mut Window,
    ) {
        let strand_count = path.brush.bristle_count.min(18);
        if strand_count == 0 || path.points.len() < 2 {
            return;
        }
        let color = Self::brush_preview_color_with_alpha(&path.brush, path.brush.opacity * 0.16);
        let step = match path.brush.preset {
            BrushPreset::Charcoal => 1,
            BrushPreset::Rough => 2,
            _ => 3,
        };
        for strand_ix in 0..strand_count {
            let lane = if strand_count <= 1 {
                0.0
            } else {
                strand_ix as f32 / (strand_count - 1) as f32 * 2.0 - 1.0
            };
            let mut builder = PathBuilder::stroke(px((path.brush.stroke_width * 0.11).max(0.35)));
            for (seq, point_ix) in (0..path.points.len()).step_by(step).enumerate() {
                let point = path.points[point_ix];
                let normal = Self::path_normal_at(&path.points, point_ix);
                let pressure = Self::brush_pressure_scale(&path.brush, point.pressure);
                let seed = Self::stable_brush_seed(
                    &path.id,
                    copy_ix ^ (strand_ix * 4099) ^ point_ix as u32,
                );
                let spread = path.brush.stroke_width * pressure * 0.54;
                let noise = Self::hash_unit(seed ^ 0x94d0_49bb) * path.brush.roughness * 0.85;
                let offset = VectorPoint::new(
                    normal.x * (lane * spread + noise),
                    normal.y * (lane * spread + noise),
                );
                let canvas_point = Self::canvas_point_offset(bounds, point, offset);
                if seq == 0 {
                    builder.move_to(canvas_point);
                } else {
                    builder.line_to(canvas_point);
                }
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        }
    }

    fn paint_texture_stamps(
        bounds: Bounds<Pixels>,
        path: &VectorPathDraft,
        copy_ix: u32,
        window: &mut Window,
    ) {
        if path.brush.texture_strength <= 0.001 || path.points.len() < 2 {
            return;
        }
        let mut stamp_count = 0usize;
        let max_stamps = match path.brush.preset {
            BrushPreset::Charcoal => 1400,
            BrushPreset::Rough => 900,
            _ => 650,
        };
        for i in 0..path.points.len() - 1 {
            let a = path.points[i];
            let b = path.points[i + 1];
            let distance = a.distance_to(b);
            if distance < 0.1 {
                continue;
            }
            let steps = (distance / path.brush.stamp_spacing.max(2.0))
                .ceil()
                .max(1.0) as usize;
            let normal = Self::segment_normal(a, b);
            for step_ix in 0..steps {
                if stamp_count >= max_stamps {
                    return;
                }
                let seed =
                    Self::stable_brush_seed(&path.id, copy_ix ^ (i as u32 * 8191) ^ step_ix as u32);
                let keep = (Self::hash_unit(seed ^ 0x27d4_eb2f) + 1.0) * 0.5;
                if keep > path.brush.texture_strength.clamp(0.0, 1.0) {
                    continue;
                }
                let t = (step_ix as f32 + 0.5) / steps as f32;
                let pressure = a.pressure + (b.pressure - a.pressure) * t;
                let pressure_scale = Self::brush_pressure_scale(&path.brush, pressure);
                let base = Self::lerp_point(a, b, t);
                let spread = path.brush.stroke_width * pressure_scale * 0.56;
                let tangent_noise = Self::hash_unit(seed ^ 0x632b_e59b) * path.brush.stamp_spacing;
                let normal_noise = Self::hash_unit(seed ^ 0x8515_7af5) * spread;
                let center = VectorPoint::new(
                    base.x + normal.x * normal_noise + (b.x - a.x).signum() * tangent_noise * 0.12,
                    base.y + normal.y * normal_noise + (b.y - a.y).signum() * tangent_noise * 0.12,
                );
                let size_noise = (Self::hash_unit(seed ^ 0x1656_67b1) + 1.0) * 0.5;
                let diameter = (path.brush.stroke_width
                    * pressure_scale
                    * (0.06 + size_noise * 0.20)
                    * match path.brush.preset {
                        BrushPreset::Charcoal => 1.55,
                        BrushPreset::Rough => 1.20,
                        _ => 1.0,
                    })
                .max(0.6);
                let alpha = path.brush.opacity
                    * path.brush.texture_strength
                    * match path.brush.preset {
                        BrushPreset::Charcoal => 0.20,
                        BrushPreset::Pencil | BrushPreset::BluePencil => 0.18,
                        _ => 0.14,
                    };
                Self::paint_stamp(bounds, center, diameter, &path.brush, alpha, window);
                stamp_count += 1;
            }
        }
    }

    fn paint_vector_path_once(
        bounds: Bounds<Pixels>,
        path: &VectorPathDraft,
        width: f32,
        color: gpui::Rgba,
        offset: VectorPoint,
        window: &mut Window,
    ) {
        let mut builder = PathBuilder::stroke(px(width));
        builder.move_to(Self::canvas_point_offset(bounds, path.points[0], offset));
        match path.kind {
            VectorPathKind::Pen => {
                for i in 0..path.points.len() - 1 {
                    let p0 = if i == 0 {
                        path.points[i]
                    } else {
                        path.points[i - 1]
                    };
                    let p1 = path.points[i];
                    let p2 = path.points[i + 1];
                    let p3 = if i + 2 < path.points.len() {
                        path.points[i + 2]
                    } else {
                        p2
                    };
                    let c1 =
                        VectorPoint::new(p1.x + (p2.x - p0.x) / 6.0, p1.y + (p2.y - p0.y) / 6.0);
                    let c2 =
                        VectorPoint::new(p2.x - (p3.x - p1.x) / 6.0, p2.y - (p3.y - p1.y) / 6.0);
                    builder.cubic_bezier_to(
                        Self::canvas_point_offset(bounds, p2, offset),
                        Self::canvas_point_offset(bounds, c1, offset),
                        Self::canvas_point_offset(bounds, c2, offset),
                    );
                }
            }
            VectorPathKind::Freehand => {
                for point in &path.points[1..] {
                    builder.line_to(Self::canvas_point_offset(bounds, *point, offset));
                }
            }
        }
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    }

    fn brush_pressure_scale(brush: &BrushSettings, pressure: f32) -> f32 {
        let shaped = pressure
            .clamp(0.0, 1.0)
            .powf(brush.pressure_curve.max(0.05));
        brush.pressure_min + (1.0 - brush.pressure_min) * shaped
    }

    fn brush_preview_color(brush: &BrushSettings, copy_ix: u32, copies: u32) -> gpui::Rgba {
        let alpha = Self::brush_preview_alpha(brush, copy_ix, copies);
        Self::brush_preview_color_with_alpha(brush, alpha)
    }

    fn brush_preview_alpha(brush: &BrushSettings, copy_ix: u32, copies: u32) -> f32 {
        let alpha_scale = match brush.preset {
            BrushPreset::CleanInk
            | BrushPreset::BoldInk
            | BrushPreset::Marker
            | BrushPreset::Hairline => 1.0,
            BrushPreset::Pencil | BrushPreset::BluePencil => 0.34,
            BrushPreset::Sketch => (0.68 - copy_ix as f32 * 0.055).max(0.28),
            BrushPreset::Rough => (0.58 - copy_ix as f32 * 0.040).max(0.22),
            BrushPreset::Charcoal => (0.32 - copy_ix as f32 * 0.012).max(0.13),
        };
        let copy_balance = match brush.preset {
            BrushPreset::CleanInk
            | BrushPreset::BoldInk
            | BrushPreset::Marker
            | BrushPreset::Hairline => 1.0,
            _ => 1.0 / (copies as f32).sqrt(),
        };
        (brush.opacity * alpha_scale * copy_balance).clamp(0.035, 1.0)
    }

    fn brush_preview_color_with_alpha(brush: &BrushSettings, alpha: f32) -> gpui::Rgba {
        let a = (alpha * 255.0).round() as u32;
        let (r, g, b) = Self::parse_hex_rgb(&brush.color).unwrap_or((0xff, 0xff, 0xff));
        rgba((r << 24) | (g << 16) | (b << 8) | a)
    }

    fn paint_stamp(
        bounds: Bounds<Pixels>,
        point: VectorPoint,
        diameter: f32,
        brush: &BrushSettings,
        alpha: f32,
        window: &mut Window,
    ) {
        let center = Self::canvas_point(bounds, point);
        let radius = diameter * 0.5;
        window.paint_quad(gpui::quad(
            Bounds {
                origin: gpui::point(center.x - px(radius), center.y - px(radius)),
                size: gpui::size(px(diameter), px(diameter)),
            },
            px(radius),
            Self::brush_preview_color_with_alpha(brush, alpha.clamp(0.02, 1.0)),
            px(0.0),
            rgba(0x00000000),
            Default::default(),
        ));
    }

    fn parse_hex_rgb(value: &str) -> Option<(u32, u32, u32)> {
        let hex = value.strip_prefix('#').unwrap_or(value);
        if hex.len() != 6 {
            return None;
        }
        let raw = u32::from_str_radix(hex, 16).ok()?;
        Some(((raw >> 16) & 0xff, (raw >> 8) & 0xff, raw & 0xff))
    }

    fn brush_jitter(path_id: &str, copy_ix: u32, brush: &BrushSettings) -> VectorPoint {
        if copy_ix == 0 || brush.roughness <= 0.0001 {
            return VectorPoint::default();
        }
        let style_factor = match brush.preset {
            BrushPreset::CleanInk
            | BrushPreset::BoldInk
            | BrushPreset::Marker
            | BrushPreset::Hairline => 0.35,
            BrushPreset::Pencil | BrushPreset::BluePencil => 1.65,
            BrushPreset::Sketch => 1.25,
            BrushPreset::Rough => 1.85,
            BrushPreset::Charcoal => 2.65,
        };
        let seed = Self::stable_brush_seed(path_id, copy_ix);
        let x = Self::hash_unit(seed ^ 0x9e37_79b9);
        let y = Self::hash_unit(seed ^ 0x85eb_ca6b);
        VectorPoint::new(
            x * brush.roughness * style_factor,
            y * brush.roughness * style_factor,
        )
    }

    fn brush_width_jitter(path_id: &str, copy_ix: u32, brush: &BrushSettings) -> f32 {
        if copy_ix == 0 || brush.roughness <= 0.0001 {
            return 1.0;
        }
        let width_factor = match brush.preset {
            BrushPreset::CleanInk | BrushPreset::BoldInk | BrushPreset::Hairline => 0.025,
            BrushPreset::Marker => 0.0,
            BrushPreset::Pencil | BrushPreset::BluePencil => 0.11,
            BrushPreset::Sketch => 0.08,
            BrushPreset::Rough => 0.16,
            BrushPreset::Charcoal => 0.24,
        };
        let seed = Self::stable_brush_seed(path_id, copy_ix) ^ 0xc2b2_ae35;
        (1.0 + Self::hash_unit(seed) * brush.roughness.min(4.0) * width_factor).max(0.35)
    }

    fn path_normal_at(points: &[VectorPoint], index: usize) -> VectorPoint {
        if points.len() < 2 {
            return VectorPoint::new(0.0, -1.0);
        }
        let prev = points[index.saturating_sub(1)];
        let next = points[(index + 1).min(points.len() - 1)];
        Self::segment_normal(prev, next)
    }

    fn segment_normal(a: VectorPoint, b: VectorPoint) -> VectorPoint {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len = (dx * dx + dy * dy).sqrt();
        if len <= f32::EPSILON {
            return VectorPoint::new(0.0, -1.0);
        }
        VectorPoint::new(-dy / len, dx / len)
    }

    fn lerp_point(a: VectorPoint, b: VectorPoint, t: f32) -> VectorPoint {
        VectorPoint::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
            .with_pressure(a.pressure + (b.pressure - a.pressure) * t)
    }

    fn stable_brush_seed(path_id: &str, copy_ix: u32) -> u32 {
        let mut hash = 0x811c_9dc5u32 ^ copy_ix.wrapping_mul(0x45d9_f3b);
        for byte in path_id.bytes() {
            hash ^= byte as u32;
            hash = hash.wrapping_mul(0x0100_0193);
        }
        hash
    }

    fn hash_unit(mut value: u32) -> f32 {
        value ^= value >> 16;
        value = value.wrapping_mul(0x7feb_352d);
        value ^= value >> 15;
        value = value.wrapping_mul(0x846c_a68b);
        value ^= value >> 16;
        (value as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    fn paint_points(
        bounds: Bounds<Pixels>,
        points: &[VectorPoint],
        color: gpui::Rgba,
        window: &mut Window,
    ) {
        for point in points {
            let center = Self::canvas_point(bounds, *point);
            window.paint_quad(gpui::quad(
                Bounds {
                    origin: gpui::point(center.x - px(4.0), center.y - px(4.0)),
                    size: gpui::size(px(8.0), px(8.0)),
                },
                px(4.0),
                color,
                px(0.0),
                rgba(0x00000000),
                Default::default(),
            ));
        }
    }

    fn canvas_point(bounds: Bounds<Pixels>, point: VectorPoint) -> gpui::Point<Pixels> {
        gpui::point(bounds.origin.x + px(point.x), bounds.origin.y + px(point.y))
    }

    fn canvas_point_offset(
        bounds: Bounds<Pixels>,
        point: VectorPoint,
        offset: VectorPoint,
    ) -> gpui::Point<Pixels> {
        gpui::point(
            bounds.origin.x + px(point.x + offset.x),
            bounds.origin.y + px(point.y + offset.y),
        )
    }

    fn render_path_list(&self, cx: &mut Context<Self>) -> gpui::Div {
        let mut list = div().flex().flex_col().gap_1();
        if self.paths.is_empty() {
            return list.child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.48))
                    .child("No paths yet."),
            );
        }
        for (idx, path) in self.paths.iter().enumerate() {
            let active = self.selected_path == Some(idx);
            let label = format!(
                "{} · {} · {} · {} pt",
                path.id,
                path.brush.summary(),
                path.kind.label(),
                path.points.len()
            );
            list = list.child(
                div()
                    .h(px(28.0))
                    .rounded_md()
                    .border_1()
                    .border_color(if active {
                        rgba(0x79c7ffcc)
                    } else {
                        rgba(0xffffff1e)
                    })
                    .bg(if active {
                        rgba(0x14324acc)
                    } else {
                        rgba(0xffffff08)
                    })
                    .px_2()
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(white().opacity(if active { 0.96 } else { 0.72 }))
                    .cursor_pointer()
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.selected_path = Some(idx);
                            if let Some(path) = this.paths.get(idx) {
                                this.current_brush = path.brush.clone();
                                this.status_line =
                                    format!("Selected {} with {}.", path.id, path.brush.summary());
                            }
                            cx.notify();
                        }),
                    ),
            );
        }
        list
    }

    fn render_group_picker_modal(
        &self,
        group_options: Vec<(String, usize)>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut group_list = div().flex().flex_col().gap_2();
        if group_options.is_empty() {
            group_list = group_list.child(
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(white().opacity(0.10))
                    .bg(rgb(0x090d14))
                    .p_3()
                    .text_sm()
                    .text_color(white().opacity(0.58))
                    .child("No MotionLoom groups found in the current DSL."),
            );
        } else {
            for (group_id, path_count) in group_options {
                let label = group_id.clone();
                let group_id_for_down = group_id.clone();
                let group_id_for_up = group_id;
                group_list = group_list.child(
                    div()
                        .h(px(34.0))
                        .rounded_md()
                        .border_1()
                        .border_color(white().opacity(0.14))
                        .bg(white().opacity(0.06))
                        .hover(|s| s.bg(white().opacity(0.12)))
                        .cursor_pointer()
                        .px_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_sm()
                        .text_color(white().opacity(0.86))
                        .child(label)
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(if path_count > 0 {
                                    0.62
                                } else {
                                    0.36
                                }))
                                .child(if path_count == 1 {
                                    "1 path".to_string()
                                } else {
                                    format!("{path_count} paths")
                                }),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.import_motionloom_group(group_id_for_down.clone(), window, cx);
                                cx.notify();
                            }),
                        )
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.import_motionloom_group(group_id_for_up.clone(), window, cx);
                                cx.notify();
                            }),
                        ),
                );
            }
        }

        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .bg(rgba(0x00000099))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(460.0))
                    .max_h(px(560.0))
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.18))
                    .bg(rgb(0x10151f))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(white().opacity(0.10))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_base()
                                            .text_color(white().opacity(0.94))
                                            .child("Load MotionLoom Group"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.52))
                                            .child("Choose a MotionLoom group. Groups with Path nodes load back as editable Vector Lab strokes."),
                                    ),
                            )
                            .child(Self::control_button("Close").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.group_picker_open = false;
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .p_4()
                            .max_h(px(470.0))
                            .overflow_y_scrollbar()
                            .child(group_list),
                    ),
            )
    }
}

impl Focusable for VectorLabPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VectorLabPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_group_id_input(window, cx);
        self.ensure_canvas_bg_picker(window, cx);
        self.ensure_brush_color_picker(window, cx);
        self.sync_brush_color_picker(window, cx);
        let snapshot = VectorCanvasSnapshot {
            paths: self.paths.clone(),
            selected_path: self.selected_path,
            current_pen_points: self.current_pen_points.clone(),
            current_freehand: self.current_freehand.clone(),
            current_brush: self.current_brush.clone(),
            bounds: self.board_bounds.clone(),
        };
        let selected_d = self.selected_path_d();
        let selected_dsl = self.selected_path_dsl();
        let reference_image = self.reference_image.clone();
        let reference_opacity = self.reference_opacity;
        let reference_zoom = self.reference_zoom;
        let reference_offset = self.reference_offset;
        let canvas_bg_color = self.canvas_bg_color;
        let tool_label = self.tool.label();
        let path_count = self.paths.len();
        let brush_summary = self.current_brush.summary();
        let simplify_label = Self::format_coord(self.simplify_epsilon);
        let width_label = Self::format_coord(self.current_brush.stroke_width);
        let opacity_label = Self::format_coord(self.current_brush.opacity);
        let roughness_label = Self::format_coord(self.current_brush.roughness);
        let copies_label = self.current_brush.copies.to_string();
        let pressure_curve_label = Self::format_coord(self.current_brush.pressure_curve);
        let texture_label = Self::format_coord(self.current_brush.texture_strength);
        let bristle_label = self.current_brush.bristle_count.to_string();
        let image_controls_label = if self.image_controls_open {
            "Image controls ^"
        } else {
            "Image controls v"
        };
        let brush_menu_label = if self.brush_menu_open {
            format!("Brush: {} ^", self.current_brush.preset.label())
        } else {
            format!("Brush: {} v", self.current_brush.preset.label())
        };
        let advanced_brush_label = if self.advanced_brush_open {
            "Advanced brush ^"
        } else {
            "Advanced brush v"
        };
        let motionloom_group_options = {
            let gs = self.global.read(cx);
            let script = gs.motionloom_scene_script();
            Self::extract_group_ids(script)
                .into_iter()
                .map(|group_id| {
                    let path_count = Self::group_path_count(script, &group_id);
                    (group_id, path_count)
                })
                .collect::<Vec<_>>()
        };
        let group_picker_modal = self
            .group_picker_open
            .then(|| self.render_group_picker_modal(motionloom_group_options.clone(), cx));
        let group_id_input_elem = if let Some(input) = self.motionloom_group_id_input.as_ref() {
            let input_for_focus = input.clone();
            div()
                .h(px(30.0))
                .w(px(190.0))
                .rounded_md()
                .border_1()
                .border_color(white().opacity(0.16))
                .bg(rgb(0x090d14))
                .overflow_hidden()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_, _, window, cx| {
                        cx.stop_propagation();
                        input_for_focus.read(cx).focus_handle(cx).focus(window);
                    }),
                )
                .child(Input::new(input).h_full().w_full())
                .into_any_element()
        } else {
            div().h(px(30.0)).w(px(190.0)).into_any_element()
        };
        let bg_color_picker_elem = if let Some(picker) = self.canvas_bg_picker.as_ref() {
            div()
                .h(px(30.0))
                .flex()
                .items_center()
                .child(
                    ColorPicker::new(picker)
                        .small()
                        .label("BG Color")
                        .featured_colors(vec![
                            Hsla::from(rgb(0x0f1624)),
                            Hsla::from(rgb(0x111827)),
                            Hsla::from(rgb(0xffffff)),
                            Hsla::from(rgb(0xf7f7f7)),
                            Hsla::from(rgb(0xf8f3ed)),
                            Hsla::from(rgb(0x0b1220)),
                            Hsla::from(rgb(0x1f2937)),
                            Hsla::from(rgb(0x3f79c5)),
                        ]),
                )
                .into_any_element()
        } else {
            div().h(px(30.0)).into_any_element()
        };
        let brush_color_picker_elem = if let Some(picker) = self.brush_color_picker.as_ref() {
            div()
                .h(px(30.0))
                .flex()
                .items_center()
                .child(
                    ColorPicker::new(picker)
                        .small()
                        .label("Line Color")
                        .featured_colors(vec![
                            Hsla::from(rgb(0x111111)),
                            Hsla::from(rgb(0xffffff)),
                            Hsla::from(rgb(0x3f79c5)),
                            Hsla::from(rgb(0xef4444)),
                            Hsla::from(rgb(0xf59e0b)),
                            Hsla::from(rgb(0x22c55e)),
                            Hsla::from(rgb(0xa855f7)),
                            Hsla::from(rgb(0x06b6d4)),
                        ]),
                )
                .into_any_element()
        } else {
            div().h(px(30.0)).into_any_element()
        };

        div()
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .bg(rgb(0x0b0f18))
            .text_color(white().opacity(0.9))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, _cx| {
                    this.focus_handle.focus(window);
                }),
            )
            .on_key_down(cx.listener(|this, evt: &KeyDownEvent, _window, cx| {
                let key = evt.keystroke.key.as_str();
                let modifiers = evt.keystroke.modifiers;
                if (modifiers.platform || modifiers.control)
                    && !modifiers.shift
                    && key.eq_ignore_ascii_case("z")
                {
                    this.undo_last_action();
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(54.0))
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(white().opacity(0.12))
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .text_color(white().opacity(0.96))
                                    .child("MotionLoom · Vector Lab"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.52))
                                    .child("Trace reference images into MotionLoom Path DSL."),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.62))
                            .child(format!(
                                "Tool: {tool_label} · Brush: {brush_summary} · Paths: {path_count}"
                            )),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(white().opacity(0.10))
                    .px_4()
                    .py_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(Self::section_label("REFERENCE"))
                            .child(Self::control_button("Upload Image").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _, win, cx| {
                                    let rx = cx.prompt_for_paths(PathPromptOptions {
                                        files: true,
                                        directories: false,
                                        multiple: false,
                                        prompt: Some("Upload Vector Lab reference image".into()),
                                    });
                                    cx.spawn_in(win, async move |view, window| {
                                        let Ok(result) = rx.await else {
                                            return;
                                        };
                                        let Some(paths) = result.ok().flatten() else {
                                            return;
                                        };
                                        let Some(path) = paths.into_iter().next() else {
                                            return;
                                        };
                                        let _ = view.update_in(window, |this, _window, cx| {
                                            this.set_reference_path(path);
                                            cx.notify();
                                        });
                                    })
                                    .detach();
                                }),
                            ))
                            .child(Self::control_button("Paste Image").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.paste_reference_from_clipboard(cx);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Import VFX Preview").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.import_vfx_preview_reference(cx);
                                    cx.notify();
                                }),
                            ))
                            .child(bg_color_picker_elem)
                            .child(brush_color_picker_elem)
                            .child(
                                Self::dynamic_button(
                                    image_controls_label.to_string(),
                                    self.image_controls_open,
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.image_controls_open = !this.image_controls_open;
                                        cx.notify();
                                    }),
                                ),
                            )
                            .child(Self::section_label("DRAW"))
                            .child(
                                Self::tool_button("Point Tool", self.tool == VectorTool::Pen)
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.tool = VectorTool::Pen;
                                            this.is_drawing_freehand = false;
                                            this.status_line = "Point Tool selected. Click points, then Finish Path.".to_string();
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .child(
                                Self::tool_button(
                                    "Freehand Tool",
                                    self.tool == VectorTool::Freehand,
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.tool = VectorTool::Freehand;
                                        this.current_pen_points.clear();
                                        this.status_line =
                                            "Freehand Tool selected. Drag on the canvas."
                                                .to_string();
                                        cx.notify();
                                    }),
                                ),
                            )
                            .child(
                                Self::dynamic_button(brush_menu_label, self.brush_menu_open)
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.brush_menu_open = !this.brush_menu_open;
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .child(Self::control_button("Finish Path").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.finish_pen_path();
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Undo").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.undo_last_action();
                                    cx.notify();
                                }),
                            )),
                    )
                    .children(self.image_controls_open.then(|| {
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .pl_2()
                            .child(Self::section_label("IMAGE"))
                            .child(Self::control_button("Img -").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.zoom_reference(0.9);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Img +").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.zoom_reference(1.1);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Fit Img").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.fit_reference();
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Img <").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.nudge_reference(-24.0, 0.0);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Img ^").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.nudge_reference(0.0, -24.0);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Img v").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.nudge_reference(0.0, 24.0);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Img >").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.nudge_reference(24.0, 0.0);
                                    cx.notify();
                                }),
                            ))
                    }))
                    .children(self.brush_menu_open.then(|| {
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .pl_2()
                            .child(Self::section_label("BRUSH PRESET"))
                            .children(BrushPreset::ALL.into_iter().map(|preset| {
                                Self::tool_button(
                                    preset.label(),
                                    self.current_brush.preset == preset,
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.select_brush_preset(preset);
                                        cx.notify();
                                    }),
                                )
                            }))
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(Self::section_label("BRUSH"))
                            .child(Self::control_button("W -").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_brush_width(-0.5);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::value_pill(format!("W {width_label}")))
                            .child(Self::control_button("W +").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_brush_width(0.5);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("O -").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_brush_opacity(-0.05);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::value_pill(format!("O {opacity_label}")))
                            .child(Self::control_button("O +").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_brush_opacity(0.05);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Smooth -").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_simplify(-0.5);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::value_pill(format!("Smooth {simplify_label}")))
                            .child(Self::control_button("Smooth +").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_simplify(0.5);
                                    cx.notify();
                                }),
                            ))
                            .child(
                                Self::dynamic_button(
                                    advanced_brush_label.to_string(),
                                    self.advanced_brush_open,
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.advanced_brush_open =
                                            !this.advanced_brush_open;
                                        cx.notify();
                                    }),
                                ),
                            ),
                    )
                    .children(self.advanced_brush_open.then(|| {
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .pl_2()
                            .child(Self::section_label("ADVANCED"))
                            .child(Self::control_button("R -").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_brush_roughness(-0.1);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::value_pill(format!("R {roughness_label}")))
                            .child(Self::control_button("R +").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_brush_roughness(0.1);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("C -").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_brush_copies(-1);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::value_pill(format!("C {copies_label}")))
                            .child(Self::control_button("C +").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_brush_copies(1);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Curve -").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_pressure_curve(-0.1);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::value_pill(format!("Curve {pressure_curve_label}")))
                            .child(Self::control_button("Curve +").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_pressure_curve(0.1);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Tex -").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_texture_strength(-0.05);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::value_pill(format!("Tex {texture_label}")))
                            .child(Self::control_button("Tex +").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_texture_strength(0.05);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("B -").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_bristles(-1);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::value_pill(format!("B {bristle_label}")))
                            .child(Self::control_button("B +").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.adjust_bristles(1);
                                    cx.notify();
                                }),
                            ))
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(Self::section_label("PATHS"))
                            .child(Self::control_button("Delete Selected").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.delete_selected_path();
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Clear Current").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.clear_current();
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Clear All").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.clear_all_paths();
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Copy Path DSL").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.copy_selected_path_dsl(cx);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Copy Group DSL").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.copy_group_dsl(cx);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::section_label("GROUP ID"))
                            .child(group_id_input_elem)
                            .child(Self::control_button("Load Group").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.group_picker_open = true;
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Attach / Replace Group").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    this.attach_to_motionloom(window, cx);
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap_3()
                    .p_4()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .rounded_lg()
                            .border_1()
                            .border_color(white().opacity(0.14))
                            .bg(rgb(0x111827))
                            .overflow_hidden()
                            .relative()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, evt: &MouseDownEvent, window, cx| {
                                    this.focus_handle.focus(window);
                                    this.handle_canvas_mouse_down(evt);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                            .on_mouse_move(cx.listener(|this, evt: &MouseMoveEvent, _, cx| {
                                this.handle_canvas_mouse_move(evt);
                                cx.notify();
                            }))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, evt: &MouseUpEvent, _, cx| {
                                    this.handle_canvas_mouse_up(evt);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left_0()
                                    .right_0()
                                    .bg(canvas_bg_color),
                            )
                            .children(reference_image.map(|layer| {
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left_0()
                                    .right_0()
                                    .opacity(reference_opacity)
                                    .child(ReferenceImageElement::new(
                                        &layer,
                                        reference_zoom,
                                        reference_offset,
                                    ))
                            }))
                            .child(
                                canvas(
                                    move |_bounds, _window, _cx| snapshot.clone(),
                                    move |bounds, snapshot, window, _cx| {
                                        Self::draw_snapshot(bounds, &snapshot, window);
                                    },
                                )
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left_0()
                                .right_0(),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left(px(12.0))
                                    .bottom(px(12.0))
                                    .rounded_md()
                                    .bg(rgba(0x00000088))
                                    .border_1()
                                    .border_color(white().opacity(0.12))
                                    .px_2()
                                    .py_1()
                                    .text_xs()
                                    .text_color(white().opacity(0.70))
                                    .child(
                                        "Point Tool: click points + Finish Path. Freehand: drag for live brush, auto-saves on mouse up.",
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .w(px(380.0))
                            .flex_shrink_0()
                            .min_h_0()
                            .rounded_lg()
                            .border_1()
                            .border_color(white().opacity(0.14))
                            .bg(rgb(0x10151f))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .px_3()
                                    .py_3()
                                    .border_b_1()
                                    .border_color(white().opacity(0.10))
                                    .text_sm()
                                    .text_color(white().opacity(0.92))
                                    .child("Selected path -> DSL d"),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .p_3()
                                    .border_b_1()
                                    .border_color(white().opacity(0.08))
                                    .child(
                                        div()
                                            .rounded_md()
                                            .bg(rgb(0x090d14))
                                            .border_1()
                                            .border_color(white().opacity(0.10))
                                            .p_2()
                                            .max_h(px(116.0))
                                            .overflow_y_scrollbar()
                                            .text_xs()
                                            .text_color(white().opacity(0.78))
                                            .child(selected_d),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .p_3()
                                    .border_b_1()
                                    .border_color(white().opacity(0.08))
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.58))
                                            .child("Path list"),
                                    )
                                    .child(self.render_path_list(cx)),
                            )
                            .child(
                                div().flex_1().min_h_0().p_3().child(
                                    div()
                                        .size_full()
                                        .rounded_md()
                                        .bg(rgb(0x090d14))
                                        .border_1()
                                        .border_color(white().opacity(0.10))
                                        .p_2()
                                        .overflow_y_scrollbar()
                                        .text_xs()
                                        .text_color(white().opacity(0.72))
                                        .child(selected_dsl),
                                ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(white().opacity(0.10))
                    .px_4()
                    .py_2()
                    .text_xs()
                    .text_color(white().opacity(0.62))
                    .child(self.status_line.clone()),
            )
            .children(group_picker_modal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_lab_group_parser_reads_path_d_not_id() {
        let group = r##"<Group id="7" x="0" y="0" opacity="1">
      <Path id="vector_path_01"
            d="M 448.9 101.5 L 403.7 165 L 381.8 214"
            stroke="#111111"
            strokeWidth="2.6"
            strokeStyle="sketch"
            strokeRoughness="1.3"
            strokeCopies="5"
            strokeTexture="0.4"
            strokeBristles="4"
            strokePressure="auto"
            strokePressureMin="0.3"
            strokePressureCurve="1.3"
            opacity="0.6"
            lineCap="round"
            lineJoin="round"
            fill="none" />
    </Group>"##;

        let paths = VectorLabPage::paths_from_group_block(group);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].id, "vector_path_01");
        assert_eq!(paths[0].points.len(), 3);
        assert_eq!(paths[0].brush.stroke_width, 2.6);
    }

    #[test]
    fn vector_lab_compact_export_uses_shared_brush_defs() {
        let brush = BrushSettings::for_preset(BrushPreset::Pencil);
        let paths = vec![
            VectorPathDraft {
                id: "vector_path_01".to_string(),
                kind: VectorPathKind::Freehand,
                points: vec![VectorPoint::new(0.0, 0.0), VectorPoint::new(10.0, 10.0)],
                brush: brush.clone(),
                export_simplify_epsilon: 0.0,
            },
            VectorPathDraft {
                id: "vector_path_02".to_string(),
                kind: VectorPathKind::Freehand,
                points: vec![VectorPoint::new(4.0, 2.0), VectorPoint::new(14.0, 12.0)],
                brush,
                export_simplify_epsilon: 0.0,
            },
        ];
        let export = VectorLabPage::vector_group_export("eyebrow_left", &paths);
        assert_eq!(export.brush_defs.len(), 1);
        assert!(
            export
                .group_block
                .contains(r#"brush="vector_eyebrow_left_brush_01""#)
        );
        assert!(
            export
                .group_block
                .contains(r#"<Path id="vector_path_01" d="M 0 0 L 10 10" />"#)
        );
        assert!(!export.group_block.contains("strokeWidth"));
        assert!(VectorLabPage::brush_defs_block(&export.brush_defs).contains("<Brush"));
    }

    #[test]
    fn vector_lab_patch_inserts_new_group_inside_timeline() {
        let paths = vec![VectorPathDraft {
            id: "vector_path_01".to_string(),
            kind: VectorPathKind::Freehand,
            points: vec![VectorPoint::new(0.0, 0.0), VectorPoint::new(10.0, 10.0)],
            brush: BrushSettings::for_preset(BrushPreset::Pencil),
            export_simplify_epsilon: 0.0,
        }];
        let export = VectorLabPage::vector_group_export("vector_group_01", &paths);
        let script = r##"<Graph fps={30} duration="8s" size={[1920,1080]}>
  <Scene id="AudioSpectrum">
    <Defs>
      <Brush id="existing_brush" stroke="#111111" strokeWidth="1.6" />
    </Defs>
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="8s" out="hold">
          <Layer>
            <Rect x="0" y="0" width="1920" height="1080" color="#000000" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
</Graph>"##;

        let updated = VectorLabPage::patch_motionloom_group_script(
            script,
            "vector_group_01",
            &export,
            1920,
            1080,
            "#ffffff",
        );
        let timeline_start = updated.find("<Timeline>").unwrap();
        let timeline_end = updated.find("</Timeline>").unwrap();
        let group_start = updated.find(r#"<Group id="vector_group_01""#).unwrap();

        assert!(timeline_start < group_start && group_start < timeline_end);
        assert!(updated.contains(r#"<Track id="vector_group_01_track" space="world" z="1000">"#));
        assert!(updated.contains(r#"<Sequence from="0s" duration="8s" out="hold">"#));
    }

    #[test]
    fn vector_lab_patch_moves_legacy_root_group_into_timeline() {
        let paths = vec![VectorPathDraft {
            id: "vector_path_01".to_string(),
            kind: VectorPathKind::Freehand,
            points: vec![VectorPoint::new(2.0, 4.0), VectorPoint::new(12.0, 14.0)],
            brush: BrushSettings::for_preset(BrushPreset::Pencil),
            export_simplify_epsilon: 0.0,
        }];
        let export = VectorLabPage::vector_group_export("vector_group_01", &paths);
        let script = r##"<Graph fps={30} duration="8s" size={[1920,1080]}>
  <Scene id="AudioSpectrum">
    <Defs>
      <Brush id="vector_vector_group_01_brush_01" stroke="#111111" strokeWidth="1.6" />
    </Defs>
    <Timeline>
      <Track id="scene_content" space="world" z="0">
        <Sequence from="0s" duration="8s" out="hold">
          <Layer>
            <Rect x="0" y="0" width="1920" height="1080" color="#000000" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
    <Group id="vector_group_01" brush="vector_vector_group_01_brush_01" x="0" y="0" opacity="1">
      <Path id="vector_path_old" d="M 1 1 L 2 2" />
    </Group>
  </Scene>
</Graph>"##;

        let updated = VectorLabPage::patch_motionloom_group_script(
            script,
            "vector_group_01",
            &export,
            1920,
            1080,
            "#ffffff",
        );
        let timeline_start = updated.find("<Timeline>").unwrap();
        let timeline_end = updated.find("</Timeline>").unwrap();
        let scene_end = updated.find("</Scene>").unwrap();
        let group_start = updated.find(r#"<Group id="vector_group_01""#).unwrap();

        assert_eq!(updated.matches(r#"<Group id="vector_group_01""#).count(), 1);
        assert!(timeline_start < group_start && group_start < timeline_end);
        assert!(!updated[timeline_end..scene_end].contains(r#"<Group id="vector_group_01""#));
    }

    #[test]
    fn vector_lab_new_scene_uses_selected_background_color() {
        let paths = vec![VectorPathDraft {
            id: "vector_path_01".to_string(),
            kind: VectorPathKind::Freehand,
            points: vec![VectorPoint::new(2.0, 4.0), VectorPoint::new(12.0, 14.0)],
            brush: BrushSettings::for_preset(BrushPreset::Pencil),
            export_simplify_epsilon: 0.0,
        }];
        let export = VectorLabPage::vector_group_export("vector_group_01", &paths);
        let updated = VectorLabPage::patch_motionloom_group_script(
            "",
            "vector_group_01",
            &export,
            1920,
            1080,
            "#123456",
        );

        assert!(updated.contains(r##"<Background color="#123456" />"##));
    }

    #[test]
    fn vector_lab_existing_scene_updates_background_color_on_attach() {
        let paths = vec![VectorPathDraft {
            id: "vector_path_01".to_string(),
            kind: VectorPathKind::Freehand,
            points: vec![VectorPoint::new(2.0, 4.0), VectorPoint::new(12.0, 14.0)],
            brush: BrushSettings::for_preset(BrushPreset::Pencil),
            export_simplify_epsilon: 0.0,
        }];
        let export = VectorLabPage::vector_group_export("vector_group_01", &paths);
        let script = r##"<Graph fps={30} duration="1s" size={[1920,1080]}>
  <Background color="#ffffff" />
  <Scene id="vector_lab_trace">
    <Timeline>
    </Timeline>
  </Scene>
  <Present from="vector_lab_trace" />
</Graph>"##;
        let updated = VectorLabPage::patch_motionloom_group_script(
            script,
            "vector_group_01",
            &export,
            1920,
            1080,
            "#DB2626",
        );

        assert!(updated.contains(r##"<Background color="#DB2626" />"##));
        assert!(!updated.contains(r##"<Background color="#ffffff" />"##));
    }

    #[test]
    fn vector_lab_group_parser_reads_compact_group_brush() {
        let script = r##"
<Graph fps={30} duration="1s" size={[734,517]}>
  <Scene id="vector_lab_trace">
    <Defs>
      <Brush id="vector_eyebrow_brush_01"
             stroke="#111111"
             strokeWidth="1.6"
             strokeStyle="pencil"
             strokeRoughness="1.8"
             strokeCopies="6"
             strokeTexture="0.7"
             strokeBristles="5"
             strokePressure="auto"
             strokePressureMin="0.2"
             strokePressureCurve="1.7"
             opacity="0.4"
             fill="none" />
    </Defs>
    <Group id="eyebrow" brush="vector_eyebrow_brush_01" x="0" y="0" opacity="1">
      <Path id="vector_path_01" d="M 268 293.6 L 266.7 287.3 L 270.4 289.8" />
    </Group>
  </Scene>
  <Present from="vector_lab_trace" />
</Graph>
"##;
        let brush_defs = VectorLabPage::brush_defs_from_script(script);
        let (start, end) = VectorLabPage::find_group_block_range(script, "eyebrow").unwrap();
        let paths =
            VectorLabPage::paths_from_group_block_with_brushes(&script[start..end], &brush_defs);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].brush.preset, BrushPreset::Pencil);
        assert_eq!(paths[0].brush.copies, 6);
        assert_eq!(paths[0].brush.bristle_count, 5);
        assert!((paths[0].brush.stroke_width - 1.6).abs() < 0.001);
    }

    #[test]
    fn vector_lab_group_parser_reads_nested_group_brush_override() {
        let script = r##"
<Graph fps={30} duration="1s" size={[734,517]}>
  <Scene id="vector_lab_trace">
    <Defs>
      <Brush id="outer_brush" stroke="#111111" strokeWidth="1.6" strokeStyle="pencil" />
      <Brush id="inner_brush" stroke="#3f79c5" strokeWidth="2.4" strokeStyle="marker" />
    </Defs>
    <Group id="root_lines" brush="outer_brush" x="0" y="0" opacity="1">
      <Path id="outer_path" d="M 0 0 L 10 10" />
      <Group id="nested_lines" brush="inner_brush" x="0" y="0">
        <Path id="inner_path" d="M 4 0 L 14 10" />
      </Group>
    </Group>
  </Scene>
  <Present from="vector_lab_trace" />
</Graph>
"##;
        let brush_defs = VectorLabPage::brush_defs_from_script(script);
        let (start, end) = VectorLabPage::find_group_block_range(script, "root_lines").unwrap();
        let paths =
            VectorLabPage::paths_from_group_block_with_brushes(&script[start..end], &brush_defs);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].id, "outer_path");
        assert_eq!(paths[0].brush.preset, BrushPreset::Pencil);
        assert!((paths[0].brush.stroke_width - 1.6).abs() < 0.001);
        assert_eq!(paths[1].id, "inner_path");
        assert_eq!(paths[1].brush.preset, BrushPreset::Marker);
        assert_eq!(paths[1].brush.color, "#3f79c5");
        assert!((paths[1].brush.stroke_width - 2.4).abs() < 0.001);
    }
}
