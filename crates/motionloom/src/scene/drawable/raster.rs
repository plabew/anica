use image::RgbaImage;

use crate::ShadowNode;
use crate::scene::composition::{blend_pixel, blend_pixel_with_mode};
use crate::scene::render::{MotionLoomSceneRenderError, eval_scene_number};
use crate::scene::spatial::Affine2;

use super::{
    PaintBounds, Point2, ResolvedPaint, SceneBlendMode, StrokeCap, StrokeJoin, StrokeStyle,
    StrokeTexture, parse_color, point_distance, point_in_subpaths_even_odd,
    point_in_subpaths_nonzero, polyline_bounds, polyline_total_length, sample_paint,
    stroke_hash_signed, stroke_taper_pressure, stroke_texture_copy_count, stroke_texture_seed,
    stroke_texture_variant, trimmed_polyline_segments_with_progress,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FillRule {
    NonZero,
    EvenOdd,
}

impl FillRule {
    pub(crate) fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "evenodd" | "even-odd" => Self::EvenOdd,
            _ => Self::NonZero,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EvaluatedShadow {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) blur: f32,
    pub(crate) color: [u8; 4],
    pub(crate) opacity: f32,
}

pub(crate) fn evaluate_shadow(
    shadow: &ShadowNode,
    time_norm: f32,
    time_sec: f32,
    inherited_opacity: f32,
) -> Result<EvaluatedShadow, MotionLoomSceneRenderError> {
    let mut color = parse_color(&shadow.color)?;
    let opacity = (eval_scene_number(&shadow.opacity, time_norm, time_sec)? * inherited_opacity)
        .clamp(0.0, 1.0);
    color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
    Ok(EvaluatedShadow {
        x: eval_scene_number(&shadow.x, time_norm, time_sec)?,
        y: eval_scene_number(&shadow.y, time_norm, time_sec)?,
        blur: eval_scene_number(&shadow.blur, time_norm, time_sec)?.max(0.0),
        color,
        opacity,
    })
}

pub(crate) fn draw_rect_shadow(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    shadow: &EvaluatedShadow,
) {
    let steps = (shadow.blur / 6.0).ceil().clamp(1.0, 10.0) as u32;
    for step in (0..steps).rev() {
        let t = (step + 1) as f32 / steps as f32;
        let spread = shadow.blur * t * 0.45;
        let mut color = shadow.color;
        color[3] = ((color[3] as f32) * shadow.opacity * (1.0 - t * 0.82))
            .round()
            .clamp(0.0, 255.0) as u8;
        if color[3] == 0 {
            continue;
        }
        draw_rounded_rect(
            canvas,
            x + shadow.x - spread,
            y + shadow.y - spread,
            width + spread * 2.0,
            height + spread * 2.0,
            radius + spread,
            color,
        );
    }
}

pub(crate) fn draw_circle_shadow(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    radius: f32,
    shadow: &EvaluatedShadow,
) {
    let steps = (shadow.blur / 6.0).ceil().clamp(1.0, 10.0) as u32;
    for step in (0..steps).rev() {
        let t = (step + 1) as f32 / steps as f32;
        let spread = shadow.blur * t * 0.45;
        let mut color = shadow.color;
        color[3] = ((color[3] as f32) * shadow.opacity * (1.0 - t * 0.82))
            .round()
            .clamp(0.0, 255.0) as u8;
        if color[3] == 0 {
            continue;
        }
        draw_circle(canvas, x + shadow.x, y + shadow.y, radius + spread, color);
    }
}

pub(crate) fn draw_rounded_rect(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: [u8; 4],
) {
    if width <= 0.0 || height <= 0.0 || color[3] == 0 {
        return;
    }
    let min_x = x.floor().max(0.0) as u32;
    let min_y = y.floor().max(0.0) as u32;
    let max_x = (x + width).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + height).ceil().min(canvas.height() as f32) as u32;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let cx = px as f32 + 0.5;
            let cy = py as f32 + 0.5;
            if rounded_rect_contains(cx, cy, x, y, width, height, radius) {
                blend_pixel(canvas, px, py, color);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_rounded_rect_stroke(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    stroke_width: f32,
    color: [u8; 4],
) {
    if stroke_width <= 0.0 || color[3] == 0 {
        return;
    }
    let min_x = x.floor().max(0.0) as u32;
    let min_y = y.floor().max(0.0) as u32;
    let max_x = (x + width).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + height).ceil().min(canvas.height() as f32) as u32;
    let inner_x = x + stroke_width;
    let inner_y = y + stroke_width;
    let inner_w = (width - stroke_width * 2.0).max(0.0);
    let inner_h = (height - stroke_width * 2.0).max(0.0);
    let inner_r = (radius - stroke_width).max(0.0);
    for py in min_y..max_y {
        for px in min_x..max_x {
            let cx = px as f32 + 0.5;
            let cy = py as f32 + 0.5;
            if rounded_rect_contains(cx, cy, x, y, width, height, radius)
                && !rounded_rect_contains(cx, cy, inner_x, inner_y, inner_w, inner_h, inner_r)
            {
                blend_pixel(canvas, px, py, color);
            }
        }
    }
}

fn rounded_rect_contains(
    px: f32,
    py: f32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
) -> bool {
    if px < x || py < y || px > x + width || py > y + height {
        return false;
    }
    let radius = radius.min(width * 0.5).min(height * 0.5).max(0.0);
    if radius <= 0.0 {
        return true;
    }
    let left = x + radius;
    let right = x + width - radius;
    let top = y + radius;
    let bottom = y + height - radius;
    let cx = px.clamp(left, right);
    let cy = py.clamp(top, bottom);
    let dx = px - cx;
    let dy = py - cy;
    dx * dx + dy * dy <= radius * radius
}

pub(crate) fn draw_circle(canvas: &mut RgbaImage, x: f32, y: f32, radius: f32, color: [u8; 4]) {
    if radius <= 0.0 || color[3] == 0 {
        return;
    }
    let min_x = (x - radius).floor().max(0.0) as u32;
    let min_y = (y - radius).floor().max(0.0) as u32;
    let max_x = (x + radius).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + radius).ceil().min(canvas.height() as f32) as u32;
    let r2 = radius * radius;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let dx = px as f32 + 0.5 - x;
            let dy = py as f32 + 0.5 - y;
            if dx * dx + dy * dy <= r2 {
                blend_pixel(canvas, px, py, color);
            }
        }
    }
}

pub(crate) fn draw_circle_paint(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    radius: f32,
    paint: &ResolvedPaint,
    opacity: f32,
    blend: SceneBlendMode,
) {
    if radius <= 0.0 || opacity <= 0.0 {
        return;
    }
    let min_x = (x - radius).floor().max(0.0) as u32;
    let min_y = (y - radius).floor().max(0.0) as u32;
    let max_x = (x + radius).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + radius).ceil().min(canvas.height() as f32) as u32;
    let bounds = PaintBounds {
        min_x: x - radius,
        min_y: y - radius,
        max_x: x + radius,
        max_y: y + radius,
    };
    let r2 = radius * radius;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let point = Point2::new(px as f32 + 0.5, py as f32 + 0.5);
            let dx = point.x - x;
            let dy = point.y - y;
            if dx * dx + dy * dy <= r2
                && let Some(src) = sample_paint(paint, point, bounds, opacity)
            {
                blend_pixel_with_mode(canvas, px, py, src, blend);
            }
        }
    }
}

pub(crate) fn draw_circle_stroke(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    radius: f32,
    stroke_width: f32,
    color: [u8; 4],
) {
    if radius <= 0.0 || stroke_width <= 0.0 || color[3] == 0 {
        return;
    }
    let min_x = (x - radius).floor().max(0.0) as u32;
    let min_y = (y - radius).floor().max(0.0) as u32;
    let max_x = (x + radius).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + radius).ceil().min(canvas.height() as f32) as u32;
    let outer = radius * radius;
    let inner_radius = (radius - stroke_width).max(0.0);
    let inner = inner_radius * inner_radius;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let dx = px as f32 + 0.5 - x;
            let dy = py as f32 + 0.5 - y;
            let d2 = dx * dx + dy * dy;
            if d2 <= outer && d2 >= inner {
                blend_pixel(canvas, px, py, color);
            }
        }
    }
}

pub(crate) fn draw_ellipse_paint(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    radius_x: f32,
    radius_y: f32,
    paint: &ResolvedPaint,
    opacity: f32,
    blend: SceneBlendMode,
) {
    if radius_x <= 0.0 || radius_y <= 0.0 || opacity <= 0.0 {
        return;
    }
    let min_x = (x - radius_x).floor().max(0.0) as u32;
    let min_y = (y - radius_y).floor().max(0.0) as u32;
    let max_x = (x + radius_x).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + radius_y).ceil().min(canvas.height() as f32) as u32;
    let bounds = PaintBounds {
        min_x: x - radius_x,
        min_y: y - radius_y,
        max_x: x + radius_x,
        max_y: y + radius_y,
    };
    for py in min_y..max_y {
        for px in min_x..max_x {
            let point = Point2::new(px as f32 + 0.5, py as f32 + 0.5);
            let dx = (point.x - x) / radius_x;
            let dy = (point.y - y) / radius_y;
            if dx * dx + dy * dy <= 1.0
                && let Some(src) = sample_paint(paint, point, bounds, opacity)
            {
                blend_pixel_with_mode(canvas, px, py, src, blend);
            }
        }
    }
}

pub(crate) fn draw_ellipse_stroke(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    radius_x: f32,
    radius_y: f32,
    stroke_width: f32,
    color: [u8; 4],
) {
    if radius_x <= 0.0 || radius_y <= 0.0 || stroke_width <= 0.0 || color[3] == 0 {
        return;
    }
    let min_radius = radius_x.min(radius_y).max(0.0001);
    let pad = stroke_width.ceil();
    let min_x = (x - radius_x - pad).floor().max(0.0) as u32;
    let min_y = (y - radius_y - pad).floor().max(0.0) as u32;
    let max_x = (x + radius_x + pad).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + radius_y + pad).ceil().min(canvas.height() as f32) as u32;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let dx = (px as f32 + 0.5 - x) / radius_x;
            let dy = (py as f32 + 0.5 - y) / radius_y;
            let signed_dist = ((dx * dx + dy * dy).sqrt() - 1.0) * min_radius;
            if signed_dist <= 0.0 && signed_dist >= -stroke_width {
                blend_pixel(canvas, px, py, color);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_rounded_rect_paint(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    paint: &ResolvedPaint,
    opacity: f32,
    blend: SceneBlendMode,
) {
    if width <= 0.0 || height <= 0.0 || opacity <= 0.0 {
        return;
    }
    let min_x = x.floor().max(0.0) as u32;
    let min_y = y.floor().max(0.0) as u32;
    let max_x = (x + width).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + height).ceil().min(canvas.height() as f32) as u32;
    let bounds = PaintBounds {
        min_x: x,
        min_y: y,
        max_x: x + width,
        max_y: y + height,
    };
    for py in min_y..max_y {
        for px in min_x..max_x {
            let point = Point2::new(px as f32 + 0.5, py as f32 + 0.5);
            if rounded_rect_contains(point.x, point.y, x, y, width, height, radius)
                && let Some(src) = sample_paint(paint, point, bounds, opacity)
            {
                blend_pixel_with_mode(canvas, px, py, src, blend);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_stroke_overlays(
    canvas: &mut RgbaImage,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    style: StrokeStyle,
    t0: f32,
    t1: f32,
) {
    if width <= 0.0 || color[3] == 0 {
        return;
    }
    if style.texture_strength > 0.001 {
        draw_stroke_texture_stamps(canvas, p0, p1, width, color, style, t0, t1);
    }
    if style.bristles > 0 {
        draw_stroke_bristles(canvas, p0, p1, width, color, style, t0, t1);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_stroke_texture_stamps(
    canvas: &mut RgbaImage,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    style: StrokeStyle,
    t0: f32,
    t1: f32,
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
        let global_t = t0 + (t1 - t0) * local_t;
        let pressure = stroke_taper_pressure(global_t, style).max(0.05);
        let tangent_noise = stroke_hash_signed(seed + 37.7) * spacing * 0.25;
        let normal_noise = stroke_hash_signed(seed + 91.3) * width * pressure * 0.45;
        let p = p0.lerp(p1, local_t);
        let size_noise = ((stroke_hash_signed(seed + 163.0) + 1.0) * 0.5).clamp(0.0, 1.0);
        let radius = (width * pressure * (0.035 + size_noise * 0.10) * texture_size).max(0.35);
        let mut stamp_color = color;
        stamp_color[3] = ((stamp_color[3] as f32) * strength * alpha_scale)
            .round()
            .clamp(0.0, 255.0) as u8;
        if stamp_color[3] > 0 {
            draw_circle(
                canvas,
                p.x + dx * tangent_noise + nx * normal_noise,
                p.y + dy * tangent_noise + ny * normal_noise,
                radius,
                stamp_color,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_stroke_bristles(
    canvas: &mut RgbaImage,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    style: StrokeStyle,
    t0: f32,
    t1: f32,
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
        ((stroke_taper_pressure(t0, style) + stroke_taper_pressure(t1, style)) * 0.5).max(0.05);
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
        let mut bristle_color = color;
        bristle_color[3] = ((bristle_color[3] as f32) * alpha_scale)
            .round()
            .clamp(0.0, 255.0) as u8;
        if bristle_color[3] > 0 {
            draw_line_segment(
                canvas,
                start.x + nx * offset,
                start.y + ny * offset,
                end.x + nx * offset,
                end.y + ny * offset,
                bristle_width,
                bristle_color,
            );
        }
    }
}

pub(crate) fn draw_transformed_trimmed_polylines_styled(
    canvas: &mut RgbaImage,
    subpaths: &[Vec<Point2>],
    width: f32,
    color: [u8; 4],
    trim: (f32, f32),
    transform: Affine2,
    style: StrokeStyle,
) {
    for segment in trimmed_polyline_segments_with_progress(subpaths, trim) {
        let (x0, y0) = transform.transform_point(segment.p0.x, segment.p0.y);
        let (x1, y1) = transform.transform_point(segment.p1.x, segment.p1.y);
        draw_line_segment_styled(
            canvas,
            Point2::new(x0, y0),
            Point2::new(x1, y1),
            width,
            color,
            style,
            segment.t0,
            segment.t1,
        );
    }
    draw_transformed_polyline_joins(canvas, subpaths, width, color, trim, transform, style);
}

pub(crate) fn affine_uniform_scale(transform: Affine2) -> f32 {
    let x_scale = (transform.m00.powi(2) + transform.m10.powi(2)).sqrt();
    let y_scale = (transform.m01.powi(2) + transform.m11.powi(2)).sqrt();
    ((x_scale + y_scale) * 0.5).max(0.001)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_line_segment_styled(
    canvas: &mut RgbaImage,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    style: StrokeStyle,
    t0: f32,
    t1: f32,
) {
    if style.texture != StrokeTexture::Solid && style.roughness > 0.0001 {
        let copies = stroke_texture_copy_count(style);
        for copy_ix in 0..copies {
            let (start, end, width_scale, opacity_scale) =
                stroke_texture_variant(p0, p1, style, copy_ix);
            let mut copy_color = color;
            copy_color[3] = ((copy_color[3] as f32) * opacity_scale)
                .round()
                .clamp(0.0, 255.0) as u8;
            let mut solid_style = style;
            solid_style.texture = StrokeTexture::Solid;
            solid_style.roughness = 0.0;
            solid_style.copies = 1;
            solid_style.texture_strength = 0.0;
            solid_style.bristles = 0;
            draw_line_segment_styled(
                canvas,
                start,
                end,
                (width * width_scale).max(0.01),
                copy_color,
                solid_style,
                t0,
                t1,
            );
        }
        draw_stroke_overlays(canvas, p0, p1, width, color, style, t0, t1);
        return;
    }
    if style.taper_start > 0.0001 || style.taper_end > 0.0001 || style.pressure_auto {
        draw_tapered_line_segment(canvas, p0, p1, width, color, style, t0, t1);
        draw_stroke_overlays(canvas, p0, p1, width, color, style, t0, t1);
        return;
    }
    match style.cap {
        StrokeCap::Round => draw_line_segment(canvas, p0.x, p0.y, p1.x, p1.y, width, color),
        StrokeCap::Butt => draw_line_segment_butt(canvas, p0, p1, width, color, 0.0),
        StrokeCap::Square => draw_line_segment_butt(canvas, p0, p1, width, color, width * 0.5),
    }
    draw_stroke_overlays(canvas, p0, p1, width, color, style, t0, t1);
}

#[allow(clippy::too_many_arguments)]
fn draw_tapered_line_segment(
    canvas: &mut RgbaImage,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    style: StrokeStyle,
    t0: f32,
    t1: f32,
) {
    let len = point_distance(p0, p1);
    if len <= 0.0001 || width <= 0.0 || color[3] == 0 {
        return;
    }
    let steps = (len / (width.max(1.0) * 0.35)).ceil().clamp(2.0, 256.0) as u32;
    for i in 0..=steps {
        let local_t = i as f32 / steps as f32;
        let global_t = t0 + (t1 - t0) * local_t;
        let pressure = stroke_taper_pressure(global_t, style);
        if pressure <= 0.0001 {
            continue;
        }
        let p = p0.lerp(p1, local_t);
        draw_circle(canvas, p.x, p.y, width * pressure * 0.5, color);
    }
}

fn draw_line_segment_butt(
    canvas: &mut RgbaImage,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    extension: f32,
) {
    if width <= 0.0 || color[3] == 0 {
        return;
    }
    let dx = p1.x - p0.x;
    let dy = p1.y - p0.y;
    let len = (dx * dx + dy * dy).sqrt().max(0.0001);
    let ux = dx / len;
    let uy = dy / len;
    let start = Point2::new(p0.x - ux * extension, p0.y - uy * extension);
    let end = Point2::new(p1.x + ux * extension, p1.y + uy * extension);
    let spread = width * 0.5 + 2.0;
    let min_x = (start.x.min(end.x) - spread).floor().max(0.0) as u32;
    let min_y = (start.y.min(end.y) - spread).floor().max(0.0) as u32;
    let max_x = (start.x.max(end.x) + spread)
        .ceil()
        .min(canvas.width() as f32) as u32;
    let max_y = (start.y.max(end.y) + spread)
        .ceil()
        .min(canvas.height() as f32) as u32;
    let len2 = ((end.x - start.x).powi(2) + (end.y - start.y).powi(2)).max(0.0001);
    let half_width = width * 0.5;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let cx = px as f32 + 0.5;
            let cy = py as f32 + 0.5;
            let t =
                ((cx - start.x) * (end.x - start.x) + (cy - start.y) * (end.y - start.y)) / len2;
            if !(0.0..=1.0).contains(&t) {
                continue;
            }
            let nearest_x = start.x + (end.x - start.x) * t;
            let nearest_y = start.y + (end.y - start.y) * t;
            let dist = ((cx - nearest_x).powi(2) + (cy - nearest_y).powi(2)).sqrt();
            let coverage = (half_width + 0.5 - dist).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let mut src = color;
            src[3] = ((src[3] as f32) * coverage).round().clamp(0.0, 255.0) as u8;
            if src[3] > 0 {
                blend_pixel(canvas, px, py, src);
            }
        }
    }
}

fn draw_transformed_polyline_joins(
    canvas: &mut RgbaImage,
    subpaths: &[Vec<Point2>],
    width: f32,
    color: [u8; 4],
    trim: (f32, f32),
    transform: Affine2,
    style: StrokeStyle,
) {
    if style.join != StrokeJoin::Round || color[3] == 0 || width <= 0.0 {
        return;
    }
    let total = polyline_total_length(subpaths);
    if total <= 0.0001 {
        return;
    }
    let start_distance = trim.0 * total;
    let end_distance = trim.1 * total;
    let mut cursor = 0.0;
    for subpath in subpaths {
        let mut local_distance = 0.0;
        for ix in 1..subpath.len().saturating_sub(1) {
            local_distance += point_distance(subpath[ix - 1], subpath[ix]);
            let distance = cursor + local_distance;
            if distance < start_distance || distance > end_distance {
                continue;
            }
            let pressure = stroke_taper_pressure(distance / total, style);
            if pressure <= 0.0001 {
                continue;
            }
            let (x, y) = transform.transform_point(subpath[ix].x, subpath[ix].y);
            draw_circle(canvas, x, y, width * pressure * 0.5, color);
        }
        cursor += polyline_total_length(std::slice::from_ref(subpath));
    }
}

pub(crate) fn draw_transformed_filled_polylines(
    canvas: &mut RgbaImage,
    subpaths: &[Vec<Point2>],
    color: [u8; 4],
    transform: Affine2,
) {
    let transformed = subpaths
        .iter()
        .map(|subpath| {
            subpath
                .iter()
                .map(|point| {
                    let (x, y) = transform.transform_point(point.x, point.y);
                    Point2::new(x, y)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    draw_filled_polylines_impl(canvas, &transformed, color, FillRule::EvenOdd);
}

fn draw_filled_polylines_impl(
    canvas: &mut RgbaImage,
    subpaths: &[Vec<Point2>],
    color: [u8; 4],
    fill_rule: FillRule,
) {
    if color[3] == 0 {
        return;
    }
    let Some((min_x, min_y, max_x, max_y)) = polyline_bounds(subpaths) else {
        return;
    };
    let min_x = min_x.floor().max(0.0) as u32;
    let min_y = min_y.floor().max(0.0) as u32;
    let max_x = max_x.ceil().min(canvas.width() as f32) as u32;
    let max_y = max_y.ceil().min(canvas.height() as f32) as u32;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let point = Point2::new(px as f32 + 0.5, py as f32 + 0.5);
            if point_in_subpaths(point, subpaths, fill_rule) {
                blend_pixel(canvas, px, py, color);
            }
        }
    }
}

pub(crate) fn draw_filled_polylines_paint_with_rule(
    canvas: &mut RgbaImage,
    subpaths: &[Vec<Point2>],
    paint: &ResolvedPaint,
    opacity: f32,
    blend: SceneBlendMode,
    fill_rule: FillRule,
) {
    let Some((min_x, min_y, max_x, max_y)) = polyline_bounds(subpaths) else {
        return;
    };
    let bounds = PaintBounds {
        min_x,
        min_y,
        max_x,
        max_y,
    };
    let min_x = min_x.floor().max(0.0) as u32;
    let min_y = min_y.floor().max(0.0) as u32;
    let max_x = max_x.ceil().min(canvas.width() as f32) as u32;
    let max_y = max_y.ceil().min(canvas.height() as f32) as u32;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let point = Point2::new(px as f32 + 0.5, py as f32 + 0.5);
            if point_in_subpaths(point, subpaths, fill_rule)
                && let Some(src) = sample_paint(paint, point, bounds, opacity)
            {
                blend_pixel_with_mode(canvas, px, py, src, blend);
            }
        }
    }
}

pub(crate) fn draw_transformed_filled_polylines_paint_with_rule(
    canvas: &mut RgbaImage,
    subpaths: &[Vec<Point2>],
    paint: &ResolvedPaint,
    opacity: f32,
    blend: SceneBlendMode,
    transform: Affine2,
    fill_rule: FillRule,
) {
    let transformed = subpaths
        .iter()
        .map(|subpath| {
            subpath
                .iter()
                .map(|point| {
                    let (x, y) = transform.transform_point(point.x, point.y);
                    Point2::new(x, y)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    draw_filled_polylines_paint_with_rule(canvas, &transformed, paint, opacity, blend, fill_rule);
}

fn point_in_subpaths(point: Point2, subpaths: &[Vec<Point2>], fill_rule: FillRule) -> bool {
    match fill_rule {
        FillRule::NonZero => point_in_subpaths_nonzero(point, subpaths),
        FillRule::EvenOdd => point_in_subpaths_even_odd(point, subpaths),
    }
}

pub(crate) fn draw_line_segment(
    canvas: &mut RgbaImage,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    width: f32,
    color: [u8; 4],
) {
    if width <= 0.0 || color[3] == 0 {
        return;
    }
    let spread = width * 0.5 + 2.0;
    let min_x = (x1.min(x2) - spread).floor().max(0.0) as u32;
    let min_y = (y1.min(y2) - spread).floor().max(0.0) as u32;
    let max_x = (x1.max(x2) + spread).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y1.max(y2) + spread).ceil().min(canvas.height() as f32) as u32;
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len2 = (dx * dx + dy * dy).max(0.0001);
    let half_width = width * 0.5;

    for py in min_y..max_y {
        for px in min_x..max_x {
            let cx = px as f32 + 0.5;
            let cy = py as f32 + 0.5;
            let t = (((cx - x1) * dx + (cy - y1) * dy) / len2).clamp(0.0, 1.0);
            let nearest_x = x1 + dx * t;
            let nearest_y = y1 + dy * t;
            let dist = ((cx - nearest_x).powi(2) + (cy - nearest_y).powi(2)).sqrt();
            let coverage = (half_width + 0.5 - dist).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let mut src = color;
            src[3] = ((src[3] as f32) * coverage).round().clamp(0.0, 255.0) as u8;
            if src[3] > 0 {
                blend_pixel(canvas, px, py, src);
            }
        }
    }
}
