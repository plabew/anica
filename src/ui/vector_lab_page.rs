// src/ui/vector_lab_page.rs - lightweight MotionLoom path tracing workspace.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use gpui::{
    App, Bounds, ClipboardEntry, ClipboardItem, Context, Element, Entity, FocusHandle, Focusable,
    GlobalElementId, ImageFormat as GpuiImageFormat, InspectorElementId, IntoElement, KeyDownEvent,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder,
    PathPromptOptions, Pixels, Render, RenderImage, Style, Window, canvas, div, point, prelude::*,
    px, rgb, rgba,
};
use gpui_component::{scroll::ScrollableElement, white};
use image::{ImageBuffer, Rgba};
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
        Self::for_preset(BrushPreset::Sketch)
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

#[derive(Clone)]
struct ReferenceImageLayer {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
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
    paths: Vec<VectorPathDraft>,
    selected_path: Option<usize>,
    current_pen_points: Vec<VectorPoint>,
    current_freehand: Vec<VectorPoint>,
    is_drawing_freehand: bool,
    current_brush: BrushSettings,
    simplify_epsilon: f32,
    path_counter: usize,
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
            paths: Vec::new(),
            selected_path: None,
            current_pen_points: Vec::new(),
            current_freehand: Vec::new(),
            is_drawing_freehand: false,
            current_brush: BrushSettings::default(),
            simplify_epsilon: 2.2,
            path_counter: 1,
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
        })
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
        self.selected_path = Some(self.paths.len().saturating_sub(1));
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

    fn all_paths_dsl(&self) -> String {
        if self.paths.is_empty() {
            return "<!-- No vector paths yet. -->".to_string();
        }
        self.paths
            .iter()
            .map(Self::path_to_dsl)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn copy_selected_path_dsl(&mut self, cx: &mut Context<Self>) {
        let dsl = self.selected_path_dsl();
        cx.write_to_clipboard(ClipboardItem::new_string(dsl));
        self.status_line = "Copied selected Path DSL to clipboard.".to_string();
    }

    fn send_to_motionloom(&mut self, cx: &mut Context<Self>) {
        if self.paths.is_empty() {
            self.status_line = "Nothing to send. Create at least one path first.".to_string();
            return;
        }
        let (width, height) = self.current_board_size().unwrap_or((1080, 720));
        let script = format!(
            r##"<Graph scope="scene" fps={{60}} duration="1s" size={{[{width},{height}]}}>
  <Scene id="vector_lab_trace">
    <Solid color="#ffffff" />
    <Group id="vector_lab_paths" x="0" y="0" opacity="1">
{paths}
    </Group>
  </Scene>

  <Present from="vector_lab_trace" />
</Graph>"##,
            width = width.max(1),
            height = height.max(1),
            paths = Self::indent_block(&self.all_paths_dsl(), 6),
        );
        self.global.update(cx, |gs, cx| {
            gs.set_motionloom_scene_script(script, true);
            gs.set_active_page(AppPage::MotionLoom);
            cx.notify();
        });
        self.status_line = "Sent vector paths to MotionLoom VFX editor.".to_string();
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
        let (r, g, b) = if brush.color.eq_ignore_ascii_case("#111111") {
            (0xff, 0xff, 0xff)
        } else {
            Self::parse_hex_rgb(&brush.color).unwrap_or((0xff, 0xff, 0xff))
        };
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
}

impl Focusable for VectorLabPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VectorLabPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

        div()
            .size_full()
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
                            .child(Self::control_button("Send to MotionLoom").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.send_to_motionloom(cx);
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
                                    .bg(rgb(0x0f1624)),
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
    }
}
