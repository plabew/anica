use gpui::{
    Bounds, Corners, Pixels, Point, RenderImage, Size, Window, point, px, quad, size,
    transparent_black,
};
use image::{ImageBuffer, Rgba};
use smallvec::SmallVec;
use std::fs;
use std::path::Path;
use std::sync::Arc;

const OVERLAY_PREVIEW_MAX_DIM: u32 = 1024;
const MODAL_MAX_W: f32 = 1080.0;
const MODAL_MAX_H: f32 = 760.0;
const MODAL_MARGIN: f32 = 24.0;
const MODAL_HEADER_H: f32 = 44.0;
const MODAL_FOOTER_H: f32 = 138.0;
const MODAL_PAD: f32 = 14.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskPaintTool {
    Brush,
    Eraser,
}

#[derive(Clone, Copy, Debug)]
pub struct MaskPaintPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug)]
pub struct MaskPaintStroke {
    pub tool: MaskPaintTool,
    pub radius_px: f32,
    pub points: Vec<MaskPaintPoint>,
}

#[derive(Clone, Debug)]
pub struct MaskModalLayout {
    pub card_bounds: Bounds<Pixels>,
    pub canvas_slot_bounds: Bounds<Pixels>,
    pub draw_bounds: Bounds<Pixels>,
}

#[derive(Clone)]
pub struct MaskPainterState {
    open: bool,
    mask_width: u32,
    mask_height: u32,
    brush_radius_px: f32,
    active_tool: MaskPaintTool,
    strokes: Vec<MaskPaintStroke>,
    active_stroke: Option<MaskPaintStroke>,
    source_image_path: String,
    source_image: Option<Arc<RenderImage>>,
    source_image_width: u32,
    source_image_height: u32,
    overlay_image: Option<Arc<RenderImage>>,
    status: String,
}

impl MaskPainterState {
    pub fn new() -> Self {
        Self {
            open: false,
            mask_width: 1024,
            mask_height: 1024,
            brush_radius_px: 22.0,
            active_tool: MaskPaintTool::Brush,
            strokes: Vec::new(),
            active_stroke: None,
            source_image_path: String::new(),
            source_image: None,
            source_image_width: 0,
            source_image_height: 0,
            overlay_image: None,
            status: "Draw mask area to edit. Brush = edit area, Eraser = keep original."
                .to_string(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open_for_semantic(
        &mut self,
        input_image_path: &str,
        fallback_width: u32,
        fallback_height: u32,
    ) {
        self.open = true;
        self.strokes.clear();
        self.active_stroke = None;
        self.overlay_image = None;
        self.active_tool = MaskPaintTool::Brush;
        self.brush_radius_px = 22.0;
        self.mask_width = fallback_width.max(1);
        self.mask_height = fallback_height.max(1);
        self.source_image = None;
        self.source_image_path = input_image_path.trim().to_string();
        self.source_image_width = 0;
        self.source_image_height = 0;

        if self.source_image_path.is_empty() {
            self.status = format!(
                "No input image selected. Painting mask at {}x{}.",
                self.mask_width, self.mask_height
            );
            self.rebuild_overlay_image();
            return;
        }

        let source_path = self.source_image_path.clone();
        match Self::load_render_image(Path::new(&source_path)) {
            Ok((img, w, h)) => {
                self.source_image = Some(img);
                self.source_image_width = w;
                self.source_image_height = h;
                self.mask_width = w.max(1);
                self.mask_height = h.max(1);
                self.status = format!("Loaded input image as mask canvas: {}x{}.", w, h);
            }
            Err(err) => {
                self.status = format!(
                    "Failed to load input image '{}': {err}. Using {}x{}.",
                    source_path, self.mask_width, self.mask_height
                );
            }
        }
        self.rebuild_overlay_image();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.active_stroke = None;
    }

    pub fn status_text(&self) -> &str {
        self.status.as_str()
    }

    pub fn mask_size(&self) -> (u32, u32) {
        (self.mask_width.max(1), self.mask_height.max(1))
    }

    pub fn has_source_image(&self) -> bool {
        self.source_image.is_some()
    }

    pub fn source_image_path(&self) -> &str {
        self.source_image_path.as_str()
    }

    pub fn brush_radius_px(&self) -> f32 {
        self.brush_radius_px
    }

    pub fn set_brush_radius_px(&mut self, value: f32) {
        self.brush_radius_px = value.clamp(2.0, 512.0);
    }

    pub fn active_tool(&self) -> MaskPaintTool {
        self.active_tool
    }

    pub fn set_active_tool(&mut self, tool: MaskPaintTool) {
        self.active_tool = tool;
    }

    pub fn stroke_count(&self) -> usize {
        let mut count = self.strokes.len();
        if self.active_stroke.is_some() {
            count += 1;
        }
        count
    }

    pub fn has_active_stroke(&self) -> bool {
        self.active_stroke.is_some()
    }

    pub fn clear(&mut self) {
        self.strokes.clear();
        self.active_stroke = None;
        self.status = "Mask cleared.".to_string();
        self.rebuild_overlay_image();
    }

    pub fn undo(&mut self) {
        if self.active_stroke.is_some() {
            self.active_stroke = None;
            self.status = "Active stroke removed.".to_string();
            self.rebuild_overlay_image();
            return;
        }
        if self.strokes.pop().is_some() {
            self.status = "Last stroke undone.".to_string();
            self.rebuild_overlay_image();
        } else {
            self.status = "Nothing to undo.".to_string();
        }
    }

    pub fn begin_stroke(&mut self, layout: &MaskModalLayout, window_pos: Point<Pixels>) -> bool {
        let Some(mask_pos) = Self::window_point_to_mask_point(
            layout.draw_bounds,
            window_pos,
            self.mask_width,
            self.mask_height,
        ) else {
            return false;
        };

        let stroke = MaskPaintStroke {
            tool: self.active_tool,
            radius_px: self.brush_radius_px.max(1.0),
            points: vec![mask_pos],
        };
        self.active_stroke = Some(stroke);
        self.rebuild_overlay_image();
        true
    }

    pub fn append_stroke(&mut self, layout: &MaskModalLayout, window_pos: Point<Pixels>) -> bool {
        let Some(stroke) = self.active_stroke.as_mut() else {
            return false;
        };
        let Some(mask_pos) = Self::window_point_to_mask_point(
            layout.draw_bounds,
            window_pos,
            self.mask_width,
            self.mask_height,
        ) else {
            return false;
        };
        if let Some(last) = stroke.points.last() {
            let dx = mask_pos.x - last.x;
            let dy = mask_pos.y - last.y;
            if (dx * dx + dy * dy).sqrt() < 0.5 {
                return false;
            }
        }
        stroke.points.push(mask_pos);
        self.rebuild_overlay_image();
        true
    }

    pub fn end_stroke(&mut self) {
        if let Some(stroke) = self.active_stroke.take() {
            if !stroke.points.is_empty() {
                self.strokes.push(stroke);
            }
            self.status = format!("Mask stroke count: {}.", self.strokes.len());
            self.rebuild_overlay_image();
        }
    }

    pub fn save_mask_png(&mut self, output_path: &Path) -> Result<(), String> {
        if self.stroke_count() == 0 {
            return Err("Mask is empty. Draw at least one brush stroke before saving.".to_string());
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "Failed to create mask directory '{}': {err}",
                    parent.display()
                )
            })?;
        }

        let mut all_strokes = self.strokes.clone();
        if let Some(active) = self.active_stroke.clone() {
            all_strokes.push(active);
        }

        let mask = Self::rasterize_edit_mask(&all_strokes, self.mask_width, self.mask_height);
        let Some(rgba_len) = Self::checked_buffer_len(self.mask_width, self.mask_height, 4) else {
            return Err(format!(
                "Mask size {}x{} is too large to save safely.",
                self.mask_width, self.mask_height
            ));
        };
        let mut rgba = vec![255u8; rgba_len];
        for (idx, &edit_flag) in mask.iter().enumerate() {
            let off = idx * 4;
            rgba[off] = 255;
            rgba[off + 1] = 255;
            rgba[off + 2] = 255;
            // OpenAI image-edit style: transparent area is editable.
            rgba[off + 3] = if edit_flag == 1 { 0 } else { 255 };
        }
        image::save_buffer_with_format(
            output_path,
            &rgba,
            self.mask_width,
            self.mask_height,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|err| format!("Failed to save mask PNG '{}': {err}", output_path.display()))?;

        self.status = format!(
            "Saved mask PNG: {} ({}x{}).",
            output_path.display(),
            self.mask_width,
            self.mask_height
        );
        Ok(())
    }

    pub fn paint_canvas(&self, draw_bounds: Bounds<Pixels>, window: &mut Window) {
        Self::paint_checkerboard(draw_bounds, window);
        if let Some(img) = self.source_image.as_ref() {
            let _ = window.paint_image(draw_bounds, Corners::default(), img.clone(), 0, false);
        }
        if let Some(overlay) = self.overlay_image.as_ref() {
            let _ = window.paint_image(draw_bounds, Corners::default(), overlay.clone(), 0, false);
        }
        let border = gpui::white().opacity(0.32);
        window.paint_quad(quad(
            draw_bounds,
            px(0.0),
            gpui::transparent_black(),
            px(1.0),
            border,
            Default::default(),
        ));
    }

    pub fn compute_layout(&self, viewport: Size<Pixels>) -> MaskModalLayout {
        Self::compute_layout_for_size(viewport, self.mask_width, self.mask_height)
    }

    pub fn compute_layout_for_size(
        viewport: Size<Pixels>,
        mask_width: u32,
        mask_height: u32,
    ) -> MaskModalLayout {
        let viewport_w = f32::from(viewport.width).max(200.0);
        let viewport_h = f32::from(viewport.height).max(200.0);
        let max_card_w = (viewport_w - MODAL_MARGIN * 2.0).max(220.0);
        let max_card_h = (viewport_h - MODAL_MARGIN * 2.0).max(220.0);
        let card_w = MODAL_MAX_W.min(max_card_w);
        let card_h = MODAL_MAX_H.min(max_card_h);
        let card_x = ((viewport_w - card_w) * 0.5).max(0.0);
        let card_y = ((viewport_h - card_h) * 0.5).max(0.0);

        let slot_x = card_x + MODAL_PAD;
        let slot_y = card_y + MODAL_HEADER_H + MODAL_PAD;
        let slot_w = (card_w - MODAL_PAD * 2.0).max(80.0);
        let slot_h = (card_h - MODAL_HEADER_H - MODAL_FOOTER_H - MODAL_PAD * 2.0).max(80.0);

        let mw = mask_width.max(1) as f32;
        let mh = mask_height.max(1) as f32;
        let mask_aspect = (mw / mh).max(0.001);
        let slot_aspect = (slot_w / slot_h).max(0.001);
        let (draw_w, draw_h) = if mask_aspect > slot_aspect {
            let w = slot_w;
            (w, (w / mask_aspect).max(1.0))
        } else {
            let h = slot_h;
            ((h * mask_aspect).max(1.0), h)
        };
        let draw_x = slot_x + (slot_w - draw_w) * 0.5;
        let draw_y = slot_y + (slot_h - draw_h) * 0.5;

        MaskModalLayout {
            card_bounds: Bounds {
                origin: point(px(card_x), px(card_y)),
                size: size(px(card_w), px(card_h)),
            },
            canvas_slot_bounds: Bounds {
                origin: point(px(slot_x), px(slot_y)),
                size: size(px(slot_w), px(slot_h)),
            },
            draw_bounds: Bounds {
                origin: point(px(draw_x), px(draw_y)),
                size: size(px(draw_w), px(draw_h)),
            },
        }
    }

    fn window_point_to_mask_point(
        draw_bounds: Bounds<Pixels>,
        window_pos: Point<Pixels>,
        mask_width: u32,
        mask_height: u32,
    ) -> Option<MaskPaintPoint> {
        if !Self::bounds_contains(draw_bounds, window_pos) {
            return None;
        }
        let draw_x = f32::from(draw_bounds.origin.x);
        let draw_y = f32::from(draw_bounds.origin.y);
        let draw_w = f32::from(draw_bounds.size.width).max(1.0);
        let draw_h = f32::from(draw_bounds.size.height).max(1.0);
        let nx = ((f32::from(window_pos.x) - draw_x) / draw_w).clamp(0.0, 1.0);
        let ny = ((f32::from(window_pos.y) - draw_y) / draw_h).clamp(0.0, 1.0);
        Some(MaskPaintPoint {
            x: nx * mask_width.max(1) as f32,
            y: ny * mask_height.max(1) as f32,
        })
    }

    fn bounds_contains(bounds: Bounds<Pixels>, p: Point<Pixels>) -> bool {
        let x = f32::from(p.x);
        let y = f32::from(p.y);
        let left = f32::from(bounds.origin.x);
        let top = f32::from(bounds.origin.y);
        let right = left + f32::from(bounds.size.width);
        let bottom = top + f32::from(bounds.size.height);
        x >= left && x <= right && y >= top && y <= bottom
    }

    fn preview_dims(mask_w: u32, mask_h: u32) -> (u32, u32, f32, f32) {
        let mask_w = mask_w.max(1);
        let mask_h = mask_h.max(1);
        let max_dim = mask_w.max(mask_h) as f32;
        let scale = (OVERLAY_PREVIEW_MAX_DIM as f32 / max_dim).min(1.0);
        let preview_w = ((mask_w as f32 * scale).round() as u32).max(1);
        let preview_h = ((mask_h as f32 * scale).round() as u32).max(1);
        let sx = preview_w as f32 / mask_w as f32;
        let sy = preview_h as f32 / mask_h as f32;
        (preview_w, preview_h, sx, sy)
    }

    fn rebuild_overlay_image(&mut self) {
        let mut all_strokes = self.strokes.clone();
        if let Some(active) = self.active_stroke.clone() {
            all_strokes.push(active);
        }
        let (preview_w, preview_h, sx, sy) = Self::preview_dims(self.mask_width, self.mask_height);
        let mask = Self::rasterize_scaled_edit_mask(&all_strokes, preview_w, preview_h, sx, sy);

        let Some(bgra_len) = Self::checked_buffer_len(preview_w, preview_h, 4) else {
            self.overlay_image = None;
            self.status = format!(
                "Overlay preview size {}x{} is too large.",
                preview_w, preview_h
            );
            return;
        };
        let mut bgra = vec![0u8; bgra_len];
        for (idx, &edit_flag) in mask.iter().enumerate() {
            if edit_flag == 0 {
                continue;
            }
            let off = idx * 4;
            // BGRA byte order for GPUI image path.
            bgra[off] = 0;
            bgra[off + 1] = 64;
            bgra[off + 2] = 255;
            bgra[off + 3] = 120;
        }
        let Some(image_buffer) = ImageBuffer::<Rgba<u8>, _>::from_raw(preview_w, preview_h, bgra)
        else {
            self.overlay_image = None;
            return;
        };
        let frames = SmallVec::from_elem(image::Frame::new(image_buffer), 1);
        self.overlay_image = Some(Arc::new(RenderImage::new(frames)));
    }

    fn rasterize_edit_mask(strokes: &[MaskPaintStroke], width: u32, height: u32) -> Vec<u8> {
        Self::rasterize_scaled_edit_mask(strokes, width, height, 1.0, 1.0)
    }

    fn rasterize_scaled_edit_mask(
        strokes: &[MaskPaintStroke],
        width: u32,
        height: u32,
        scale_x: f32,
        scale_y: f32,
    ) -> Vec<u8> {
        let width = width.max(1);
        let height = height.max(1);
        let Some(mask_len) = Self::checked_buffer_len(width, height, 1) else {
            return Vec::new();
        };
        let mut out = vec![0u8; mask_len];
        for stroke in strokes {
            Self::apply_stroke_to_mask(&mut out, width, height, stroke, scale_x, scale_y);
        }
        out
    }

    fn apply_stroke_to_mask(
        out: &mut [u8],
        width: u32,
        height: u32,
        stroke: &MaskPaintStroke,
        scale_x: f32,
        scale_y: f32,
    ) {
        if stroke.points.is_empty() {
            return;
        }
        let radius = stroke.radius_px.max(1.0) * ((scale_x + scale_y) * 0.5).max(0.001);
        let set_value = matches!(stroke.tool, MaskPaintTool::Brush);
        let scaled_points: Vec<MaskPaintPoint> = stroke
            .points
            .iter()
            .map(|p| MaskPaintPoint {
                x: p.x * scale_x,
                y: p.y * scale_y,
            })
            .collect();

        if scaled_points.len() == 1 {
            let p = scaled_points[0];
            Self::stamp_circle(out, width, height, p.x, p.y, radius, set_value);
            return;
        }

        for seg in scaled_points.windows(2) {
            let a = seg[0];
            let b = seg[1];
            Self::stamp_segment(out, width, height, a, b, radius, set_value);
        }
    }

    fn stamp_segment(
        out: &mut [u8],
        width: u32,
        height: u32,
        a: MaskPaintPoint,
        b: MaskPaintPoint,
        radius: f32,
        set_value: bool,
    ) {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let dist = (dx * dx + dy * dy).sqrt();
        let step_len = (radius * 0.35).max(0.5);
        let steps = (dist / step_len).ceil() as usize;
        if steps == 0 {
            Self::stamp_circle(out, width, height, a.x, a.y, radius, set_value);
            return;
        }
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let x = a.x + dx * t;
            let y = a.y + dy * t;
            Self::stamp_circle(out, width, height, x, y, radius, set_value);
        }
    }

    fn stamp_circle(
        out: &mut [u8],
        width: u32,
        height: u32,
        cx: f32,
        cy: f32,
        radius: f32,
        set_value: bool,
    ) {
        let r2 = radius * radius;
        let min_x = (cx - radius).floor().max(0.0) as i32;
        let max_x = (cx + radius).ceil().min(width as f32 - 1.0) as i32;
        let min_y = (cy - radius).floor().max(0.0) as i32;
        let max_y = (cy + radius).ceil().min(height as f32 - 1.0) as i32;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                if dx * dx + dy * dy > r2 {
                    continue;
                }
                let idx = y as usize * width as usize + x as usize;
                out[idx] = if set_value { 1 } else { 0 };
            }
        }
    }

    fn paint_checkerboard(bounds: Bounds<Pixels>, window: &mut Window) {
        let left = f32::from(bounds.origin.x);
        let top = f32::from(bounds.origin.y);
        let width = f32::from(bounds.size.width).max(1.0);
        let height = f32::from(bounds.size.height).max(1.0);
        let tile = 14.0;
        let rows = (height / tile).ceil() as i32;
        let cols = (width / tile).ceil() as i32;
        for row in 0..rows {
            for col in 0..cols {
                let x = left + col as f32 * tile;
                let y = top + row as f32 * tile;
                let w = (tile).min((left + width) - x).max(0.0);
                let h = (tile).min((top + height) - y).max(0.0);
                if w <= 0.0 || h <= 0.0 {
                    continue;
                }
                let color = if (row + col) % 2 == 0 {
                    gpui::rgba(0xffffff22)
                } else {
                    gpui::rgba(0x00000026)
                };
                window.paint_quad(quad(
                    Bounds {
                        origin: point(px(x), px(y)),
                        size: size(px(w), px(h)),
                    },
                    px(0.0),
                    color,
                    px(0.0),
                    transparent_black(),
                    Default::default(),
                ));
            }
        }
    }

    fn load_render_image(path: &Path) -> Result<(Arc<RenderImage>, u32, u32), String> {
        let decoded = image::open(path)
            .map_err(|err| format!("failed to decode image '{}': {err}", path.display()))?;
        let rgba = decoded.to_rgba8();
        let (w, h) = rgba.dimensions();
        // GPUI image rendering path in this app expects BGRA channel order.
        let mut bgra = rgba.into_raw();
        for px in bgra.chunks_mut(4) {
            let r = px[0];
            let b = px[2];
            px[0] = b;
            px[2] = r;
        }
        let image_buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(w, h, bgra)
            .ok_or_else(|| "failed to create image buffer".to_string())?;
        let frames = SmallVec::from_elem(image::Frame::new(image_buffer), 1);
        Ok((Arc::new(RenderImage::new(frames)), w, h))
    }

    fn checked_buffer_len(width: u32, height: u32, channels: u64) -> Option<usize> {
        let pixels = (width as u64).checked_mul(height as u64)?;
        let bytes = pixels.checked_mul(channels)?;
        usize::try_from(bytes).ok()
    }
}
