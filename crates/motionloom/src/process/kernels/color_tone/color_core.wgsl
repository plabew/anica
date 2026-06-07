// color_core.wgsl
//
// Monolithic color/look utility kernel for common Adjustment Layer workflows.
//
// Canonical effect keys mapped to this file:
// 1) Color correction: exposure_contrast, saturation, curves, lut, hsla_overlay
// 2) Stylize: vignette, film_look, black_white
// 3) Noise/grain: noise_grain
// 4) Glow/bloom: glow_bloom
//
// Note:
// - Current runtime may evaluate many effects on CPU side for preview/export logic.
// - This file is the canonical WGSL source bucket for future GPU routing.

struct ColorCoreParams {
    exposure_ev: f32,
    contrast: f32,
    pivot: f32,
    saturation: f32,
    curve_gamma: f32,
    lut_mix: f32,
    vignette_strength: f32,
    vignette_roundness: f32,
    film_strength: f32,
    grain_amount: f32,
    glow_threshold: f32,
    glow_intensity: f32,
    hsla_hue: f32,
    hsla_saturation: f32,
    hsla_lightness: f32,
    hsla_alpha: f32,
    time_sec: f32,
}

fn ml_luma(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn ml_saturate(rgb: vec3<f32>, sat: f32) -> vec3<f32> {
    let y = ml_luma(rgb);
    return mix(vec3<f32>(y), rgb, sat);
}

fn ml_exposure_contrast(rgb: vec3<f32>, exposure_ev: f32, contrast: f32, pivot: f32) -> vec3<f32> {
    let exposed = rgb * exp2(exposure_ev);
    return (exposed - vec3<f32>(pivot)) * contrast + vec3<f32>(pivot);
}

fn ml_curves_gamma(rgb: vec3<f32>, gamma: f32) -> vec3<f32> {
    let g = max(gamma, 0.001);
    return pow(max(rgb, vec3<f32>(0.0)), vec3<f32>(1.0 / g));
}

fn ml_hue_to_rgb(p: f32, q: f32, t_in: f32) -> f32 {
    var t = t_in;
    if t < 0.0 {
        t = t + 1.0;
    }
    if t > 1.0 {
        t = t - 1.0;
    }
    if t < (1.0 / 6.0) {
        return p + (q - p) * 6.0 * t;
    }
    if t < 0.5 {
        return q;
    }
    if t < (2.0 / 3.0) {
        return p + (q - p) * ((2.0 / 3.0) - t) * 6.0;
    }
    return p;
}

fn ml_hsla_to_rgb(hue_deg: f32, sat: f32, light: f32) -> vec3<f32> {
    let h = fract(hue_deg / 360.0);
    let s = clamp(sat, 0.0, 1.0);
    let l = clamp(light, 0.0, 1.0);
    if s <= 0.00001 {
        return vec3<f32>(l);
    }
    var q = 0.0;
    if l < 0.5 {
        q = l * (1.0 + s);
    } else {
        q = l + s - (l * s);
    }
    let p = 2.0 * l - q;
    return vec3<f32>(
        ml_hue_to_rgb(p, q, h + (1.0 / 3.0)),
        ml_hue_to_rgb(p, q, h),
        ml_hue_to_rgb(p, q, h - (1.0 / 3.0))
    );
}

fn ml_hsla_overlay(
    rgb: vec3<f32>,
    hue_deg: f32,
    sat: f32,
    light: f32,
    alpha: f32
) -> vec3<f32> {
    let overlay = ml_hsla_to_rgb(hue_deg, sat, light);
    let a = clamp(alpha, 0.0, 1.0);
    return mix(rgb, overlay, a);
}

// Lightweight LUT-style look approximation (placeholder until full 3D LUT sampling path).
fn ml_lut_approx(rgb: vec3<f32>, mix_amount: f32) -> vec3<f32> {
    let m = clamp(mix_amount, 0.0, 1.0);
    let warm = vec3<f32>(
        rgb.r * 1.03,
        rgb.g * 1.00,
        rgb.b * 0.97
    );
    return mix(rgb, warm, m);
}

fn ml_vignette(rgb: vec3<f32>, uv: vec2<f32>, strength: f32, roundness: f32) -> vec3<f32> {
    let centered = uv * 2.0 - vec2<f32>(1.0, 1.0);
    let r = length(vec2<f32>(centered.x, centered.y * mix(1.0, 0.75, roundness)));
    let vig = 1.0 - clamp((r - 0.2) * strength, 0.0, 1.0);
    return rgb * vig;
}

fn ml_film_look(rgb: vec3<f32>, strength: f32) -> vec3<f32> {
    let s = clamp(strength, 0.0, 1.0);
    let toe = pow(max(rgb, vec3<f32>(0.0)), vec3<f32>(1.05));
    let lift = toe + vec3<f32>(0.01, 0.008, 0.006);
    let cool_shadows = vec3<f32>(lift.r * 0.99, lift.g * 1.00, lift.b * 1.02);
    return mix(rgb, cool_shadows, s);
}

fn ml_black_white(rgb: vec3<f32>) -> vec3<f32> {
    let y = ml_luma(rgb);
    return vec3<f32>(y);
}

fn ml_noise_grain(rgb: vec3<f32>, uv: vec2<f32>, time_sec: f32, amount: f32) -> vec3<f32> {
    let n = fract(sin(dot(uv + vec2<f32>(time_sec * 0.07, time_sec * 0.11), vec2<f32>(12.9898, 78.233))) * 43758.5453);
    let centered = (n - 0.5) * 2.0;
    return rgb + vec3<f32>(centered) * amount;
}

fn ml_glow_bloom(base: vec3<f32>, blurred: vec3<f32>, threshold: f32, intensity: f32) -> vec3<f32> {
    let t = max(threshold, 0.0);
    let i = max(intensity, 0.0);
    let lum = ml_luma(base);
    let mask = smoothstep(t - 0.1, t + 0.1, lum);
    return base + blurred * mask * i;
}

// Optional dispatch hook for future compile-time specialization paths.
// effect_id map:
//  1 exposure_contrast
//  2 saturation
//  3 curves
//  4 lut
//  5 vignette
//  6 film_look
//  7 black_white
//  8 noise_grain
//  9 glow_bloom
// 10 hsla_overlay
fn ml_color_core_dispatch(effect_id: u32, rgb: vec3<f32>, uv: vec2<f32>, params: ColorCoreParams) -> vec3<f32> {
    switch effect_id {
        case 1u: {
            return ml_exposure_contrast(rgb, params.exposure_ev, params.contrast, params.pivot);
        }
        case 2u: {
            return ml_saturate(rgb, params.saturation);
        }
        case 3u: {
            return ml_curves_gamma(rgb, params.curve_gamma);
        }
        case 4u: {
            return ml_lut_approx(rgb, params.lut_mix);
        }
        case 5u: {
            return ml_vignette(rgb, uv, params.vignette_strength, params.vignette_roundness);
        }
        case 6u: {
            return ml_film_look(rgb, params.film_strength);
        }
        case 7u: {
            return ml_black_white(rgb);
        }
        case 8u: {
            return ml_noise_grain(rgb, uv, params.time_sec, params.grain_amount);
        }
        case 9u: {
            // Placeholder single-source glow path (base-as-blur) until multi-input bind path is wired.
            return ml_glow_bloom(rgb, rgb, params.glow_threshold, params.glow_intensity);
        }
        case 10u: {
            return ml_hsla_overlay(
                rgb,
                params.hsla_hue,
                params.hsla_saturation,
                params.hsla_lightness,
                params.hsla_alpha
            );
        }
        default: {
            return rgb;
        }
    }
}
