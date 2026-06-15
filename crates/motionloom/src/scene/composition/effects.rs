use std::collections::VecDeque;

use image::{Rgba, RgbaImage};

use crate::process::model::{EffectNode, LayerNode, PassNode};
use crate::scene::drawable::parse_color;
use crate::scene::model::FilterStepDef;
use crate::scene::render::{MotionLoomSceneRenderError, eval_scene_number};

#[derive(Debug, Clone, Copy)]
pub(crate) struct SceneBloomParams {
    pub(crate) threshold: f32,
    pub(crate) intensity: f32,
    pub(crate) sigma: f32,
}

pub(crate) fn apply_scene_post_pass(
    input: &RgbaImage,
    pass: &PassNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<RgbaImage, MotionLoomSceneRenderError> {
    let effect = pass.effect.to_ascii_lowercase();
    if effect == "opacity" || effect == "composite.opacity" {
        let opacity = pass_param_expr(pass, "opacity")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        return Ok(apply_opacity_pass(input, opacity));
    }
    if effect.contains("gaussian_5tap_h") || effect.contains("gaussian_h") {
        let sigma = pass_param_expr(pass, "sigma")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(2.0)
            .clamp(0.0, 64.0);
        return Ok(apply_box_blur_pass(input, sigma, true));
    }
    if effect.contains("gaussian_5tap_v") || effect.contains("gaussian_v") {
        let sigma = pass_param_expr(pass, "sigma")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(2.0)
            .clamp(0.0, 64.0);
        return Ok(apply_box_blur_pass(input, sigma, false));
    }
    if effect == "blur" || effect == "gaussian_blur" || effect == "gaussian_5tap_blur" {
        let sigma = pass_param_expr(pass, "sigma")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(2.0)
            .clamp(0.0, 64.0);
        let blurred = apply_box_blur_pass(input, sigma, true);
        return Ok(apply_box_blur_pass(&blurred, sigma, false));
    }
    if let Some(params) = scene_post_bloom_params(pass, time_norm, time_sec)? {
        let prefiltered = build_scene_bloom_prefilter(input, params.threshold);
        let blurred_h = apply_box_blur_pass(&prefiltered, params.sigma, true);
        let blurred = apply_box_blur_pass(&blurred_h, params.sigma, false);
        return Ok(composite_scene_bloom(input, &blurred, params.intensity));
    }
    if is_color_key_alpha_effect(&effect) {
        return apply_color_key_alpha_pass(input, pass, time_norm, time_sec);
    }
    if effect == "hsla" || effect == "hsla_overlay" || effect == "color.hsla" {
        return apply_hsla_pass(input, pass, time_norm, time_sec);
    }
    if effect == "color_core" || effect == "color_blur" {
        let brightness = pass_param_expr(pass, "brightness")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(0.0)
            .clamp(-1.0, 1.0);
        let contrast = pass_param_expr(pass, "contrast")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(1.0)
            .clamp(0.0, 2.0);
        let saturation = pass_param_expr(pass, "saturation")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(1.0)
            .clamp(0.0, 2.0);
        return Ok(apply_color_core_pass(
            input, brightness, contrast, saturation,
        ));
    }
    Ok(input.clone())
}

pub(crate) fn is_color_key_alpha_effect(effect: &str) -> bool {
    let normalized = effect
        .trim()
        .trim_matches('"')
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_");
    matches!(
        normalized.as_str(),
        "color_to_alpha"
            | "color2alpha"
            | "alpha_from_color"
            | "white_to_alpha"
            | "white2alpha"
            | "color_key_alpha"
            | "key_color_alpha"
            | "background_to_alpha"
            | "key_to_alpha"
            | "key.alpha"
            | "alpha.key"
    )
}

pub(crate) fn apply_color_key_alpha_pass(
    input: &RgbaImage,
    pass: &PassNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<RgbaImage, MotionLoomSceneRenderError> {
    let key_color = pass_param_expr_any(pass, &["color", "key", "keyColor", "key_color"])
        .map(|value| parse_color(value.trim().trim_matches('"').trim_matches('\'')))
        .transpose()?
        .unwrap_or([255, 255, 255, 255]);
    let tolerance = pass_param_expr(pass, "tolerance")
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(0.035)
        .clamp(0.0, 1.0);
    let softness = pass_param_expr(pass, "softness")
        .or_else(|| pass_param_expr(pass, "feather"))
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(0.16)
        .clamp(0.0001, 1.0);
    let strength = pass_param_expr(pass, "strength")
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    let unspill = pass_param_expr(pass, "unspill")
        .or_else(|| pass_param_expr(pass, "despill"))
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let edge_connected = pass_param_expr(pass, "edge")
        .or_else(|| pass_param_expr(pass, "edgeConnected"))
        .or_else(|| pass_param_expr(pass, "edge_connected"))
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    Ok(apply_color_key_alpha(
        input,
        key_color,
        tolerance,
        softness,
        strength,
        unspill,
        edge_connected > 0.5,
    ))
}

pub(crate) fn apply_layer_effects(
    input: &RgbaImage,
    layer: &LayerNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<RgbaImage, MotionLoomSceneRenderError> {
    let mut out = input.clone();
    for effect in &layer.effects {
        out = apply_layer_effect(&out, effect, time_norm, time_sec)?;
    }
    Ok(out)
}

pub(crate) fn apply_layer_effect(
    input: &RgbaImage,
    effect: &EffectNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<RgbaImage, MotionLoomSceneRenderError> {
    let effect_type = effect.r#type.to_ascii_lowercase();
    if effect_type == "blur" || effect_type == "gaussian_blur" {
        let sigma = effect_param_expr(effect, "sigma")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(2.0)
            .clamp(0.0, 64.0);
        let blurred = apply_box_blur_pass(input, sigma, true);
        return Ok(apply_box_blur_pass(&blurred, sigma, false));
    }
    if effect_type == "hsla" || effect_type == "hsla_overlay" || effect_type == "color.hsla" {
        let hue = effect_param_expr(effect, "hue")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(0.0);
        let saturation = effect_param_expr(effect, "saturation")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let lightness = effect_param_expr(effect, "lightness")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        let alpha = effect_param_expr(effect, "alpha")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        return Ok(apply_hsla_overlay(input, hue, saturation, lightness, alpha));
    }
    Ok(input.clone())
}

pub(crate) fn apply_scene_filter_step(
    input: &RgbaImage,
    step: &FilterStepDef,
    time_norm: f32,
    time_sec: f32,
) -> Result<RgbaImage, MotionLoomSceneRenderError> {
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
        let blurred = apply_box_blur_pass(input, sigma, true);
        return Ok(apply_box_blur_pass(&blurred, sigma, false));
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
        let mut out = apply_color_core_pass(input, brightness, contrast, saturation);
        if let Some(opacity_expr) = step.opacity.as_deref() {
            let opacity = eval_scene_number(opacity_expr, time_norm, time_sec)?.clamp(0.0, 1.0);
            out = apply_opacity_pass(&out, opacity);
        }
        return Ok(out);
    }
    if kind == "opacity" {
        let opacity = step
            .opacity
            .as_deref()
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        return Ok(apply_opacity_pass(input, opacity));
    }
    Ok(input.clone())
}

pub(crate) fn apply_over_pass(inputs: &[RgbaImage]) -> RgbaImage {
    let Some(first) = inputs.first() else {
        return RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 0]));
    };
    let mut out = first.clone();
    for image in inputs.iter().skip(1) {
        composite_image_over_origin(&mut out, image);
    }
    out
}

fn composite_image_over_origin(canvas: &mut RgbaImage, image: &RgbaImage) {
    let width = canvas.width().min(image.width());
    let height = canvas.height().min(image.height());
    for y in 0..height {
        for x in 0..width {
            let src = image.get_pixel(x, y).0;
            if src[3] == 0 {
                continue;
            }
            blend_pixel_normal(canvas, x, y, src);
        }
    }
}

fn blend_pixel_normal(canvas: &mut RgbaImage, x: u32, y: u32, src: [u8; 4]) {
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

pub(crate) fn apply_hsla_pass(
    input: &RgbaImage,
    pass: &PassNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<RgbaImage, MotionLoomSceneRenderError> {
    let hue = pass_param_expr(pass, "hue")
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(0.0);
    let saturation = pass_param_expr(pass, "saturation")
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let lightness = pass_param_expr(pass, "lightness")
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let alpha = pass_param_expr(pass, "alpha")
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    Ok(apply_hsla_overlay(input, hue, saturation, lightness, alpha))
}

fn apply_hsla_overlay(
    input: &RgbaImage,
    hue: f32,
    saturation: f32,
    lightness: f32,
    alpha: f32,
) -> RgbaImage {
    let [or, og, ob] = hsl_to_rgb(hue, saturation, lightness);
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

pub(crate) fn scene_post_blur_passes(
    pass: &PassNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Option<Vec<(bool, f32)>>, MotionLoomSceneRenderError> {
    let effect = pass.effect.to_ascii_lowercase();
    let directions =
        if effect == "blur" || effect == "gaussian_blur" || effect == "gaussian_5tap_blur" {
            vec![true, false]
        } else if effect.contains("gaussian_5tap_h") || effect.contains("gaussian_h") {
            vec![true]
        } else if effect.contains("gaussian_5tap_v") || effect.contains("gaussian_v") {
            vec![false]
        } else {
            return Ok(None);
        };
    let sigma = pass_param_expr(pass, "sigma")
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(2.0)
        .clamp(0.0, 64.0);
    Ok(Some(
        directions
            .into_iter()
            .map(|horizontal| (horizontal, sigma))
            .collect(),
    ))
}

pub(crate) fn scene_post_bloom_params(
    pass: &PassNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Option<SceneBloomParams>, MotionLoomSceneRenderError> {
    let effect = pass
        .effect
        .trim()
        .trim_matches('"')
        .trim()
        .to_ascii_lowercase();
    let effect = effect.replace('-', "_");
    if !matches!(
        effect.as_str(),
        "bloom"
            | "glow"
            | "glow_bloom"
            | "post.bloom"
            | "post.glow"
            | "light_atmosphere.bloom"
            | "light_atmosphere.glow"
            | "light_atmosphere.glow_bloom"
    ) {
        return Ok(None);
    }

    let threshold = pass_param_expr_any(pass, &["threshold", "glowThreshold", "glow_threshold"])
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(0.72)
        .clamp(0.0, 1.0);
    let intensity = pass_param_expr_any(pass, &["intensity", "glowIntensity", "glow_intensity"])
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(1.0)
        .clamp(0.0, 8.0);
    let sigma = pass_param_expr(pass, "sigma")
        .or_else(|| pass_param_expr(pass, "radius"))
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(14.0)
        .clamp(0.0, 64.0);

    Ok(Some(SceneBloomParams {
        threshold,
        intensity,
        sigma,
    }))
}

pub(crate) fn pass_param_expr<'a>(pass: &'a PassNode, key: &str) -> Option<&'a str> {
    pass.params
        .iter()
        .find(|param| param.key.eq_ignore_ascii_case(key))
        .map(|param| param.value.as_str())
}

fn pass_param_expr_any<'a>(pass: &'a PassNode, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| pass_param_expr(pass, key))
}

pub(crate) fn effect_param_expr<'a>(effect: &'a EffectNode, key: &str) -> Option<&'a str> {
    effect
        .params
        .iter()
        .find(|param| param.key.eq_ignore_ascii_case(key))
        .map(|param| param.value.as_str())
}

pub(crate) fn apply_opacity_pass(input: &RgbaImage, opacity: f32) -> RgbaImage {
    let mut out = input.clone();
    for pixel in out.pixels_mut() {
        pixel[3] = ((pixel[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
    }
    out
}

pub(crate) fn apply_color_key_alpha(
    input: &RgbaImage,
    key_color: [u8; 4],
    tolerance: f32,
    softness: f32,
    strength: f32,
    unspill: f32,
    edge_connected: bool,
) -> RgbaImage {
    let mut out = input.clone();
    let key = [
        key_color[0] as f32 / 255.0,
        key_color[1] as f32 / 255.0,
        key_color[2] as f32 / 255.0,
    ];
    let edge0 = tolerance.clamp(0.0, 1.0);
    let edge1 = (edge0 + softness.max(0.0001)).clamp(edge0 + 0.0001, 1.0);
    let connected_key = edge_connected.then(|| connected_key_pixels(input, key, edge1));
    for (index, pixel) in out.pixels_mut().enumerate() {
        if let Some(mask) = connected_key.as_ref()
            && !mask.get(index).copied().unwrap_or(false)
        {
            continue;
        }
        let input_alpha = pixel[3] as f32 / 255.0;
        if input_alpha <= 0.0 {
            continue;
        }
        let rgb = [
            pixel[0] as f32 / 255.0,
            pixel[1] as f32 / 255.0,
            pixel[2] as f32 / 255.0,
        ];
        let distance = color_key_distance(rgb, key);
        let keep = smoothstep(edge0, edge1, distance);
        let alpha_factor = (1.0 - strength) + keep * strength;
        let out_alpha = (input_alpha * alpha_factor).clamp(0.0, 1.0);

        if unspill > 0.0 && out_alpha > 0.001 && alpha_factor < 0.999 {
            for channel in 0..3 {
                let recovered = ((rgb[channel] - key[channel] * (1.0 - alpha_factor))
                    / alpha_factor.max(0.001))
                .clamp(0.0, 1.0);
                let mixed = rgb[channel] * (1.0 - unspill) + recovered * unspill;
                pixel[channel] = (mixed * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        pixel[3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        if pixel[3] == 0 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
        }
    }
    out
}

fn connected_key_pixels(input: &RgbaImage, key: [f32; 3], threshold: f32) -> Vec<bool> {
    let width = input.width() as usize;
    let height = input.height() as usize;
    let len = width.saturating_mul(height);
    let mut visited = vec![false; len];
    if width == 0 || height == 0 {
        return visited;
    }

    let mut queue = VecDeque::<(usize, usize)>::new();
    for x in 0..width {
        push_key_seed(input, key, threshold, (x, 0), &mut visited, &mut queue);
        if height > 1 {
            push_key_seed(
                input,
                key,
                threshold,
                (x, height - 1),
                &mut visited,
                &mut queue,
            );
        }
    }
    for y in 1..height.saturating_sub(1) {
        push_key_seed(input, key, threshold, (0, y), &mut visited, &mut queue);
        if width > 1 {
            push_key_seed(
                input,
                key,
                threshold,
                (width - 1, y),
                &mut visited,
                &mut queue,
            );
        }
    }

    while let Some((x, y)) = queue.pop_front() {
        let neighbors = [
            (x.wrapping_sub(1), y, x > 0),
            (x + 1, y, x + 1 < width),
            (x, y.wrapping_sub(1), y > 0),
            (x, y + 1, y + 1 < height),
        ];
        for (nx, ny, valid) in neighbors {
            if valid {
                push_key_seed(input, key, threshold, (nx, ny), &mut visited, &mut queue);
            }
        }
    }

    visited
}

fn push_key_seed(
    input: &RgbaImage,
    key: [f32; 3],
    threshold: f32,
    point: (usize, usize),
    visited: &mut [bool],
    queue: &mut VecDeque<(usize, usize)>,
) {
    let (x, y) = point;
    let width = input.width() as usize;
    let index = y.saturating_mul(width).saturating_add(x);
    if visited.get(index).copied().unwrap_or(true) {
        return;
    }
    let pixel = input.get_pixel(x as u32, y as u32);
    let rgb = [
        pixel[0] as f32 / 255.0,
        pixel[1] as f32 / 255.0,
        pixel[2] as f32 / 255.0,
    ];
    if color_key_distance(rgb, key) <= threshold {
        visited[index] = true;
        queue.push_back((x, y));
    }
}

fn color_key_distance(rgb: [f32; 3], key: [f32; 3]) -> f32 {
    let dr = rgb[0] - key[0];
    let dg = rgb[1] - key[1];
    let db = rgb[2] - key[2];
    ((dr * dr + dg * dg + db * db) / 3.0).sqrt().clamp(0.0, 1.0)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(0.0001)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub(crate) fn apply_box_blur_pass(input: &RgbaImage, sigma: f32, horizontal: bool) -> RgbaImage {
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

pub(crate) fn build_scene_bloom_prefilter(input: &RgbaImage, threshold: f32) -> RgbaImage {
    let mut out = RgbaImage::from_pixel(input.width(), input.height(), Rgba([0, 0, 0, 0]));
    let threshold = threshold.clamp(0.0, 1.0);
    let range = (1.0 - threshold).max(0.001);
    for (x, y, pixel) in input.enumerate_pixels() {
        let alpha = pixel[3] as f32 / 255.0;
        let r = pixel[0] as f32 / 255.0;
        let g = pixel[1] as f32 / 255.0;
        let b = pixel[2] as f32 / 255.0;
        let luma = (0.2126 * r + 0.7152 * g + 0.0722 * b) * alpha;
        let bloom = ((luma - threshold) / range).clamp(0.0, 1.0);
        if bloom <= 0.0 {
            continue;
        }
        out.put_pixel(
            x,
            y,
            Rgba([
                (r * bloom * 255.0).round().clamp(0.0, 255.0) as u8,
                (g * bloom * 255.0).round().clamp(0.0, 255.0) as u8,
                (b * bloom * 255.0).round().clamp(0.0, 255.0) as u8,
                (alpha * bloom * 255.0).round().clamp(0.0, 255.0) as u8,
            ]),
        );
    }
    out
}

pub(crate) fn composite_scene_bloom(
    input: &RgbaImage,
    blurred: &RgbaImage,
    intensity: f32,
) -> RgbaImage {
    let mut out = input.clone();
    let intensity = intensity.clamp(0.0, 8.0);
    for (x, y, pixel) in out.enumerate_pixels_mut() {
        let glow = blurred.get_pixel(x.min(blurred.width() - 1), y.min(blurred.height() - 1));
        let base_a = pixel[3] as f32 / 255.0;
        let glow_a = (glow[3] as f32 / 255.0 * intensity).clamp(0.0, 1.0);
        for channel in 0..3 {
            let base = pixel[channel] as f32 / 255.0;
            let glow_rgb = glow[channel] as f32 / 255.0;
            pixel[channel] = ((base + glow_rgb * intensity).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        pixel[3] = (base_a.max(glow_a) * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out
}

pub(crate) fn apply_color_core_pass(
    input: &RgbaImage,
    brightness: f32,
    contrast: f32,
    saturation: f32,
) -> RgbaImage {
    let mut out = input.clone();
    for pixel in out.pixels_mut() {
        let a = pixel[3];
        let mut r = pixel[0] as f32 / 255.0;
        let mut g = pixel[1] as f32 / 255.0;
        let mut b = pixel[2] as f32 / 255.0;
        let luma = r * 0.2126 + g * 0.7152 + b * 0.0722;
        r = luma + (r - luma) * saturation;
        g = luma + (g - luma) * saturation;
        b = luma + (b - luma) * saturation;
        r = (r - 0.5) * contrast + 0.5 + brightness;
        g = (g - 0.5) * contrast + 0.5 + brightness;
        b = (b - 0.5) * contrast + 0.5 + brightness;
        pixel[0] = (r * 255.0).round().clamp(0.0, 255.0) as u8;
        pixel[1] = (g * 255.0).round().clamp(0.0, 255.0) as u8;
        pixel[2] = (b * 255.0).round().clamp(0.0, 255.0) as u8;
        pixel[3] = a;
    }
    out
}
