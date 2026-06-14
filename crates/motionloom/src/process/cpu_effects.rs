// =========================================
// crates/motionloom/src/process/cpu_effects.rs
// =========================================

use image::{Rgba, RgbaImage};

pub(crate) fn apply_hsla_overlay(
    input: &RgbaImage,
    hue: f32,
    saturation: f32,
    lightness: f32,
    alpha: f32,
) -> RgbaImage {
    let [or, og, ob] = hsl_to_rgb(hue, saturation, lightness);
    let alpha = alpha.clamp(0.0, 1.0);
    let mut out = input.clone();
    for pixel in out.pixels_mut() {
        let base_a = pixel[3];
        let r = pixel[0] as f32 / 255.0;
        let g = pixel[1] as f32 / 255.0;
        let b = pixel[2] as f32 / 255.0;
        pixel[0] = (((r * (1.0 - alpha)) + (or * alpha)) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
        pixel[1] = (((g * (1.0 - alpha)) + (og * alpha)) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
        pixel[2] = (((b * (1.0 - alpha)) + (ob * alpha)) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
        pixel[3] = base_a;
    }
    out
}

pub(crate) fn apply_separable_gaussian_blur(
    input: &RgbaImage,
    sigma: f32,
    horizontal: bool,
) -> RgbaImage {
    if sigma <= 0.001 {
        return input.clone();
    }
    let radius = sigma.ceil().clamp(1.0, 64.0) as i32;
    let weights = (-radius..=radius)
        .map(|offset| {
            let distance = offset as f32;
            (-(distance * distance) / (2.0 * sigma.max(0.001).powi(2))).exp()
        })
        .collect::<Vec<_>>();
    let weight_sum = weights.iter().sum::<f32>().max(0.001);
    let mut out = RgbaImage::from_pixel(input.width(), input.height(), Rgba([0, 0, 0, 0]));
    for y in 0..input.height() {
        for x in 0..input.width() {
            let mut acc = [0.0_f32; 4];
            for (weight_ix, offset) in (-radius..=radius).enumerate() {
                let (sx, sy) = if horizontal {
                    (
                        (x as i32 + offset).clamp(0, input.width() as i32 - 1) as u32,
                        y,
                    )
                } else {
                    (
                        x,
                        (y as i32 + offset).clamp(0, input.height() as i32 - 1) as u32,
                    )
                };
                let weight = weights[weight_ix];
                let pixel = input.get_pixel(sx, sy);
                for channel in 0..4 {
                    acc[channel] += pixel[channel] as f32 * weight;
                }
            }
            let mut rgba = [0_u8; 4];
            for channel in 0..4 {
                rgba[channel] = (acc[channel] / weight_sum).round().clamp(0.0, 255.0) as u8;
            }
            *out.get_pixel_mut(x, y) = Rgba(rgba);
        }
    }
    out
}

pub(crate) fn apply_gaussian_blur(input: &RgbaImage, sigma: f32) -> RgbaImage {
    let blurred = apply_separable_gaussian_blur(input, sigma, true);
    apply_separable_gaussian_blur(&blurred, sigma, false)
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let h = (hue.rem_euclid(360.0)) / 360.0;
    let s = saturation.clamp(0.0, 1.0);
    let l = lightness.clamp(0.0, 1.0);
    if s <= 0.0001 {
        return [l, l, l];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    [
        hue_to_rgb_channel(p, q, h + 1.0 / 3.0),
        hue_to_rgb_channel(p, q, h),
        hue_to_rgb_channel(p, q, h - 1.0 / 3.0),
    ]
}

fn hue_to_rgb_channel(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 1.0 / 2.0 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}
