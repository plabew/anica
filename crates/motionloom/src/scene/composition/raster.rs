use image::{Rgba, RgbaImage};

use crate::scene::drawable::Point2;
use crate::scene::drawable::SceneBlendMode;
use crate::scene::spatial::{
    Affine2, CameraRect, EvaluatedDeformGrid, triangle_barycentric,
    triangle_barycentric_denominator,
};

pub(crate) fn apply_alpha_mask(layer: &mut RgbaImage, mask: &RgbaImage) {
    apply_alpha_mask_with_invert(layer, mask, false);
}

pub(crate) fn apply_alpha_mask_with_invert(layer: &mut RgbaImage, mask: &RgbaImage, invert: bool) {
    let w = layer.width().min(mask.width());
    let h = layer.height().min(mask.height());
    for y in 0..h {
        for x in 0..w {
            let mut alpha = mask.get_pixel(x, y)[3] as f32 / 255.0;
            if invert {
                alpha = 1.0 - alpha;
            }
            let pixel = layer.get_pixel_mut(x, y);
            pixel[3] = ((pixel[3] as f32) * alpha).round().clamp(0.0, 255.0) as u8;
        }
    }
}

pub(crate) fn composite_layer(canvas: &mut RgbaImage, layer: &RgbaImage) {
    for (x, y, pixel) in layer.enumerate_pixels() {
        if pixel[3] > 0 && x < canvas.width() && y < canvas.height() {
            blend_pixel(canvas, x, y, pixel.0);
        }
    }
}

pub(crate) fn composite_transformed_layer(
    canvas: &mut RgbaImage,
    layer: &RgbaImage,
    x: f32,
    y: f32,
    rotation_deg: f32,
    scale: f32,
) {
    let theta = rotation_deg.to_radians();
    let (sin_t, cos_t) = theta.sin_cos();
    for (src_x, src_y, pixel) in layer.enumerate_pixels() {
        if pixel[3] == 0 {
            continue;
        }
        let sx = src_x as f32 * scale;
        let sy = src_y as f32 * scale;
        let dx = x + sx * cos_t - sy * sin_t;
        let dy = y + sx * sin_t + sy * cos_t;
        let dst_x = dx.round() as i32;
        let dst_y = dy.round() as i32;
        if dst_x < 0 || dst_y < 0 {
            continue;
        }
        let (dst_x, dst_y) = (dst_x as u32, dst_y as u32);
        if dst_x >= canvas.width() || dst_y >= canvas.height() {
            continue;
        }
        blend_pixel(canvas, dst_x, dst_y, pixel.0);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn composite_transformed_layer_anchored(
    canvas: &mut RgbaImage,
    layer: &RgbaImage,
    x: f32,
    y: f32,
    rotation_deg: f32,
    scale: f32,
    anchor_x: f32,
    anchor_y: f32,
) {
    let theta = rotation_deg.to_radians();
    let (sin_t, cos_t) = theta.sin_cos();
    for (src_x, src_y, pixel) in layer.enumerate_pixels() {
        if pixel[3] == 0 {
            continue;
        }
        let sx = (src_x as f32 - anchor_x) * scale;
        let sy = (src_y as f32 - anchor_y) * scale;
        let dx = x + sx * cos_t - sy * sin_t;
        let dy = y + sx * sin_t + sy * cos_t;
        let dst_x = dx.round() as i32;
        let dst_y = dy.round() as i32;
        if dst_x < 0 || dst_y < 0 {
            continue;
        }
        let (dst_x, dst_y) = (dst_x as u32, dst_y as u32);
        if dst_x >= canvas.width() || dst_y >= canvas.height() {
            continue;
        }
        blend_pixel(canvas, dst_x, dst_y, pixel.0);
    }
}

pub(crate) fn composite_layer_affine(
    canvas: &mut RgbaImage,
    layer: &RgbaImage,
    transform: Affine2,
) {
    composite_layer_affine_clipped(canvas, layer, transform, None);
}

pub(crate) fn composite_layer_affine_blend(
    canvas: &mut RgbaImage,
    layer: &RgbaImage,
    transform: Affine2,
    opacity: f32,
    blend: SceneBlendMode,
) {
    composite_layer_affine_blend_clipped(canvas, layer, transform, opacity, blend, None);
}

pub(crate) fn composite_layer_affine_blend_clipped(
    canvas: &mut RgbaImage,
    layer: &RgbaImage,
    transform: Affine2,
    opacity: f32,
    blend: SceneBlendMode,
    clip: Option<CameraRect>,
) {
    let opacity = opacity.clamp(0.0, 1.0);
    if opacity <= 0.0001 {
        return;
    }
    let Some(inverse) = transform.inverse() else {
        return;
    };
    let w = layer.width() as f32;
    let h = layer.height() as f32;
    if w <= 0.0 || h <= 0.0 {
        return;
    }

    let corners = [
        transform.transform_point(0.0, 0.0),
        transform.transform_point(w - 1.0, 0.0),
        transform.transform_point(w - 1.0, h - 1.0),
        transform.transform_point(0.0, h - 1.0),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in corners {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    let mut x0 = (min_x.floor() as i32 - 2).clamp(0, canvas.width() as i32);
    let mut y0 = (min_y.floor() as i32 - 2).clamp(0, canvas.height() as i32);
    let mut x1 = (max_x.ceil() as i32 + 2).clamp(0, canvas.width() as i32);
    let mut y1 = (max_y.ceil() as i32 + 2).clamp(0, canvas.height() as i32);
    if let Some(clip) = clip {
        x0 = x0.max(clip.x.floor() as i32);
        y0 = y0.max(clip.y.floor() as i32);
        x1 = x1.min((clip.x + clip.width).ceil() as i32);
        y1 = y1.min((clip.y + clip.height).ceil() as i32);
    }
    if x1 <= x0 || y1 <= y0 {
        return;
    }

    for dst_y in y0..y1 {
        for dst_x in x0..x1 {
            let (src_x, src_y) = inverse.transform_point(dst_x as f32, dst_y as f32);
            let Some(mut pixel) = sample_layer_bilinear(layer, src_x, src_y) else {
                continue;
            };
            if pixel[3] == 0 {
                continue;
            }
            pixel[3] = ((pixel[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
            blend_pixel_with_mode(canvas, dst_x as u32, dst_y as u32, pixel, blend);
        }
    }
}

pub(crate) fn composite_layer_affine_clipped(
    canvas: &mut RgbaImage,
    layer: &RgbaImage,
    transform: Affine2,
    clip: Option<CameraRect>,
) {
    let Some(inverse) = transform.inverse() else {
        return;
    };
    let w = layer.width() as f32;
    let h = layer.height() as f32;
    if w <= 0.0 || h <= 0.0 {
        return;
    }

    let corners = [
        transform.transform_point(0.0, 0.0),
        transform.transform_point(w - 1.0, 0.0),
        transform.transform_point(w - 1.0, h - 1.0),
        transform.transform_point(0.0, h - 1.0),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in corners {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    let mut x0 = (min_x.floor() as i32 - 2).clamp(0, canvas.width() as i32);
    let mut y0 = (min_y.floor() as i32 - 2).clamp(0, canvas.height() as i32);
    let mut x1 = (max_x.ceil() as i32 + 2).clamp(0, canvas.width() as i32);
    let mut y1 = (max_y.ceil() as i32 + 2).clamp(0, canvas.height() as i32);
    if let Some(clip) = clip {
        x0 = x0.max(clip.x.floor() as i32);
        y0 = y0.max(clip.y.floor() as i32);
        x1 = x1.min((clip.x + clip.width).ceil() as i32);
        y1 = y1.min((clip.y + clip.height).ceil() as i32);
    }
    if x1 <= x0 || y1 <= y0 {
        return;
    }

    for dst_y in y0..y1 {
        for dst_x in x0..x1 {
            let (src_x, src_y) = inverse.transform_point(dst_x as f32, dst_y as f32);
            let Some(pixel) = sample_layer_bilinear(layer, src_x, src_y) else {
                continue;
            };
            if pixel[3] == 0 {
                continue;
            }
            blend_pixel(canvas, dst_x as u32, dst_y as u32, pixel);
        }
    }
}

pub(crate) fn sample_layer_bilinear(layer: &RgbaImage, x: f32, y: f32) -> Option<[u8; 4]> {
    if x < -0.5 || y < -0.5 || x > layer.width() as f32 - 0.5 || y > layer.height() as f32 - 0.5 {
        return None;
    }

    let x = x.clamp(0.0, layer.width().saturating_sub(1) as f32);
    let y = y.clamp(0.0, layer.height().saturating_sub(1) as f32);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(layer.width().saturating_sub(1));
    let y1 = (y0 + 1).min(layer.height().saturating_sub(1));
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;

    let samples = [
        (layer.get_pixel(x0, y0).0, (1.0 - tx) * (1.0 - ty)),
        (layer.get_pixel(x1, y0).0, tx * (1.0 - ty)),
        (layer.get_pixel(x0, y1).0, (1.0 - tx) * ty),
        (layer.get_pixel(x1, y1).0, tx * ty),
    ];

    let mut premul = [0.0_f32; 3];
    let mut alpha = 0.0_f32;
    for (rgba, weight) in samples {
        let a = rgba[3] as f32 / 255.0;
        alpha += a * weight;
        premul[0] += rgba[0] as f32 * a * weight;
        premul[1] += rgba[1] as f32 * a * weight;
        premul[2] += rgba[2] as f32 * a * weight;
    }
    if alpha <= 0.0001 {
        return None;
    }

    Some([
        (premul[0] / alpha).round().clamp(0.0, 255.0) as u8,
        (premul[1] / alpha).round().clamp(0.0, 255.0) as u8,
        (premul[2] / alpha).round().clamp(0.0, 255.0) as u8,
        (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    ])
}

pub(crate) fn draw_rgba_image(
    canvas: &mut RgbaImage,
    image: &RgbaImage,
    x: f32,
    y: f32,
    opacity: f32,
) {
    let base_x = x.round() as i32;
    let base_y = y.round() as i32;
    for (src_x, src_y, pixel) in image.enumerate_pixels() {
        let dst_x = base_x + src_x as i32;
        let dst_y = base_y + src_y as i32;
        if dst_x < 0 || dst_y < 0 {
            continue;
        }
        let (dst_x, dst_y) = (dst_x as u32, dst_y as u32);
        if dst_x >= canvas.width() || dst_y >= canvas.height() {
            continue;
        }
        let mut src = pixel.0;
        src[3] = ((src[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
        if src[3] == 0 {
            continue;
        }
        blend_pixel(canvas, dst_x, dst_y, src);
    }
}

pub(crate) fn apply_deform_grid(source: &RgbaImage, grid: &EvaluatedDeformGrid) -> RgbaImage {
    let mut out = RgbaImage::from_pixel(source.width(), source.height(), Rgba([0, 0, 0, 0]));
    for row in 0..grid.rows - 1 {
        for col in 0..grid.cols - 1 {
            let i00 = row * grid.cols + col;
            let i10 = i00 + 1;
            let i01 = (row + 1) * grid.cols + col;
            let i11 = i01 + 1;
            raster_deform_triangle(
                &mut out,
                source,
                [grid.from[i00], grid.from[i10], grid.from[i11]],
                [grid.to[i00], grid.to[i10], grid.to[i11]],
            );
            raster_deform_triangle(
                &mut out,
                source,
                [grid.from[i00], grid.from[i11], grid.from[i01]],
                [grid.to[i00], grid.to[i11], grid.to[i01]],
            );
        }
    }
    out
}

fn raster_deform_triangle(
    out: &mut RgbaImage,
    source: &RgbaImage,
    src: [Point2; 3],
    dst: [Point2; 3],
) {
    let min_x = dst
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min)
        .floor() as i32
        - 1;
    let min_y = dst
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min)
        .floor() as i32
        - 1;
    let max_x = dst
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil() as i32
        + 1;
    let max_y = dst
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil() as i32
        + 1;

    let x0 = min_x.clamp(0, out.width() as i32);
    let y0 = min_y.clamp(0, out.height() as i32);
    let x1 = max_x.clamp(0, out.width() as i32);
    let y1 = max_y.clamp(0, out.height() as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }

    let denom = triangle_barycentric_denominator(dst);
    if denom.abs() <= 0.00001 {
        return;
    }

    for y in y0..y1 {
        for x in x0..x1 {
            let point = Point2::new(x as f32, y as f32);
            let Some((w0, w1, w2)) = triangle_barycentric(point, dst, denom) else {
                continue;
            };
            if w0 < -0.001 || w1 < -0.001 || w2 < -0.001 {
                continue;
            }
            let src_x = src[0].x * w0 + src[1].x * w1 + src[2].x * w2;
            let src_y = src[0].y * w0 + src[1].y * w1 + src[2].y * w2;
            let Some(pixel) = sample_layer_bilinear(source, src_x, src_y) else {
                continue;
            };
            if pixel[3] == 0 {
                continue;
            }
            out.put_pixel(x as u32, y as u32, Rgba(pixel));
        }
    }
}

pub(crate) fn blend_pixel_with_mode(
    canvas: &mut RgbaImage,
    x: u32,
    y: u32,
    src: [u8; 4],
    mode: SceneBlendMode,
) {
    if mode == SceneBlendMode::Normal {
        blend_pixel(canvas, x, y, src);
        return;
    }

    let dst = canvas.get_pixel_mut(x, y);
    let sa = src[3] as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let sr = src[0] as f32 / 255.0;
    let sg = src[1] as f32 / 255.0;
    let sb = src[2] as f32 / 255.0;
    let dr = dst[0] as f32 / 255.0;
    let dg = dst[1] as f32 / 255.0;
    let db = dst[2] as f32 / 255.0;
    let blend_channel = |s: f32, d: f32| match mode {
        SceneBlendMode::Normal => s,
        SceneBlendMode::Multiply => s * d,
        SceneBlendMode::Screen => 1.0 - (1.0 - s) * (1.0 - d),
        SceneBlendMode::Add => (s + d).min(1.0),
    };
    let br = blend_channel(sr, dr);
    let bg = blend_channel(sg, dg);
    let bb = blend_channel(sb, db);
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        *dst = Rgba([0, 0, 0, 0]);
        return;
    }

    let out_r = (br * sa + dr * da * (1.0 - sa)) / out_a;
    let out_g = (bg * sa + dg * da * (1.0 - sa)) / out_a;
    let out_b = (bb * sa + db * da * (1.0 - sa)) / out_a;
    *dst = Rgba([
        (out_r * 255.0).round().clamp(0.0, 255.0) as u8,
        (out_g * 255.0).round().clamp(0.0, 255.0) as u8,
        (out_b * 255.0).round().clamp(0.0, 255.0) as u8,
        (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
    ]);
}

pub(crate) fn blend_pixel(canvas: &mut RgbaImage, x: u32, y: u32, src: [u8; 4]) {
    let dst = canvas.get_pixel_mut(x, y);
    let (sr, sg, sb, sa) = (src[0] as f32, src[1] as f32, src[2] as f32, src[3] as f32);
    let (dr, dg, db, da) = (dst[0] as f32, dst[1] as f32, dst[2] as f32, dst[3] as f32);

    let sa = sa / 255.0;
    let da = da / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        *dst = Rgba([0, 0, 0, 0]);
        return;
    }

    let out_r = (sr * sa + dr * da * (1.0 - sa)) / out_a;
    let out_g = (sg * sa + dg * da * (1.0 - sa)) / out_a;
    let out_b = (sb * sa + db * da * (1.0 - sa)) / out_a;

    *dst = Rgba([
        out_r.round().clamp(0.0, 255.0) as u8,
        out_g.round().clamp(0.0, 255.0) as u8,
        out_b.round().clamp(0.0, 255.0) as u8,
        (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
    ]);
}
