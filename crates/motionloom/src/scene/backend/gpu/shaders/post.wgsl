
struct PostParams {
    canvas: vec4<f32>,
    params: vec4<f32>,
    extra: vec4<f32>,
};

@group(0) @binding(0) var base_tex: texture_2d<f32>;
@group(0) @binding(1) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: PostParams;
@group(0) @binding(3) var overlay_tex: texture_2d<f32>;
@group(0) @binding(4) var height_tex: texture_2d<f32>;
@group(0) @binding(5) var post_sampler: sampler;

fn hue_to_rgb_channel(p: f32, q: f32, t_in: f32) -> f32 {
    var t = t_in;
    if (t < 0.0) {
        t = t + 1.0;
    }
    if (t > 1.0) {
        t = t - 1.0;
    }
    if (t < 1.0 / 6.0) {
        return p + (q - p) * 6.0 * t;
    }
    if (t < 1.0 / 2.0) {
        return q;
    }
    if (t < 2.0 / 3.0) {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    return p;
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> vec3<f32> {
    let h = fract(hue / 360.0);
    let s = clamp(saturation, 0.0, 1.0);
    let l = clamp(lightness, 0.0, 1.0);
    if (s <= 0.0001) {
        return vec3<f32>(l, l, l);
    }
    let q = select(l + s - l * s, l * (1.0 + s), l < 0.5);
    let p = 2.0 * l - q;
    return vec3<f32>(
        hue_to_rgb_channel(p, q, h + 1.0 / 3.0),
        hue_to_rgb_channel(p, q, h),
        hue_to_rgb_channel(p, q, h - 1.0 / 3.0),
    );
}

fn aces_fitted(rgb: vec3<f32>, shoulder: f32) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59 + shoulder * 0.24;
    let e = 0.14;
    return clamp((rgb * (a * rgb + vec3<f32>(b))) / (rgb * (c * rgb + vec3<f32>(d)) + vec3<f32>(e)), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453123);
}

fn noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (vec2<f32>(3.0) - 2.0 * f);
    return mix(
        mix(hash21(i), hash21(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash21(i + vec2<f32>(0.0, 1.0)), hash21(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y
    );
}

fn fbm(p_in: vec2<f32>) -> f32 {
    var p = p_in;
    var amp = 0.5;
    var sum = 0.0;
    for (var i = 0; i < 4; i = i + 1) {
        sum = sum + noise2(p) * amp;
        p = p * 2.03 + vec2<f32>(17.1, 9.2);
        amp = amp * 0.5;
    }
    return sum;
}

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= u32(params.canvas.x) || y >= u32(params.canvas.y)) {
        return;
    }

    let mode = i32(round(params.params.w));
    if (mode == 1) {
        let brightness = params.params.x;
        let contrast = params.params.y;
        let saturation = params.params.z;
        let src = textureLoad(base_tex, vec2<i32>(i32(x), i32(y)), 0);
        let luma = dot(src.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        var rgb = vec3<f32>(
            luma + (src.r - luma) * saturation,
            luma + (src.g - luma) * saturation,
            luma + (src.b - luma) * saturation
        );
        rgb = (rgb - vec3<f32>(0.5)) * contrast + vec3<f32>(0.5 + brightness);
        textureStore(out_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), src.a));
        return;
    }

    if (mode == 2) {
        let src = textureLoad(base_tex, vec2<i32>(i32(x), i32(y)), 0);
        let tint = vec3<f32>(params.params.x, params.params.y, params.params.z);
        let intensity = max(params.canvas.z, 0.0);
        let tint_alpha = clamp(params.canvas.w, 0.0, 1.0);
        textureStore(out_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(tint, clamp(src.a * tint_alpha * intensity, 0.0, 1.0)));
        return;
    }

    if (mode == 3) {
        let src = textureLoad(base_tex, vec2<i32>(i32(x), i32(y)), 0);
        let opacity = clamp(params.params.x, 0.0, 1.0);
        textureStore(out_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(src.rgb, src.a * opacity));
        return;
    }

    if (mode == 4) {
        let src = textureLoad(base_tex, vec2<i32>(i32(x), i32(y)), 0);
        let overlay = hsl_to_rgb(params.params.x, params.params.y, params.params.z);
        let alpha = clamp(params.canvas.w, 0.0, 1.0);
        let rgb = mix(src.rgb, overlay, alpha);
        textureStore(out_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), src.a));
        return;
    }

    if (mode == 5) {
        let src = textureLoad(base_tex, vec2<i32>(i32(x), i32(y)), 0);
        let exposure = params.params.x;
        let contrast = params.params.y;
        let gamma = max(params.params.z, 0.0001);
        let shoulder = clamp(params.canvas.z, 0.0, 2.0);
        let saturation = params.canvas.w;
        var rgb = src.rgb * exp2(exposure);
        rgb = aces_fitted(max(rgb, vec3<f32>(0.0)), shoulder);
        rgb = (rgb - vec3<f32>(0.5)) * contrast + vec3<f32>(0.5);
        let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        rgb = vec3<f32>(luma) + (rgb - vec3<f32>(luma)) * saturation;
        rgb = pow(max(rgb, vec3<f32>(0.0)), vec3<f32>(1.0 / gamma));
        textureStore(out_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), src.a));
        return;
    }

    if (mode == 8) {
        let src = textureLoad(base_tex, vec2<i32>(i32(x), i32(y)), 0);
        let center = params.params.xy;
        let radius = max(params.params.z, 0.001);
        let zoom = max(params.extra.x, 0.001);
        let distortion = params.extra.y;
        let feather = max(params.canvas.z, 0.0);
        let glass = clamp(params.canvas.w, 0.0, 1.0);
        let pixel = vec2<f32>(f32(x) + 0.5, f32(y) + 0.5);
        let delta = pixel - center;
        let dist = length(delta);
        let influence = 1.0 - smoothstep(radius, radius + feather, dist);
        if (influence <= 0.0) {
            textureStore(out_tex, vec2<i32>(i32(x), i32(y)), src);
            return;
        }
        let normalized = clamp(dist / radius, 0.0, 1.0);
        let warp = max(0.001, zoom * (1.0 + distortion * (1.0 - normalized * normalized)));
        let sample_pixel = center + delta / warp;
        let sample_xy = vec2<i32>(
            clamp(i32(sample_pixel.x), 0, i32(params.canvas.x) - 1),
            clamp(i32(sample_pixel.y), 0, i32(params.canvas.y) - 1)
        );
        let lens = textureLoad(base_tex, sample_xy, 0);
        let lens_pos = delta / radius;
        let highlight = pow(max(0.0, 1.0 - length(lens_pos - vec2<f32>(-0.38, -0.42))), 5.0);
        let rim_highlight = (1.0 - clamp(abs(normalized - 0.92) / 0.055, 0.0, 1.0)) * glass;
        let inner_shadow = (1.0 - clamp(abs(normalized - 0.78) / 0.18, 0.0, 1.0)) * glass;
        let rim = 1.0 - smoothstep(0.82, 0.98, normalized);
        let edge_shadow = smoothstep(0.78, 1.0, normalized) * 0.18 * glass;
        var lens_rgb = lens.rgb + vec3<f32>(highlight * glass * 0.32);
        lens_rgb = lens_rgb * (1.0 - edge_shadow - inner_shadow * 0.08)
            + vec3<f32>(0.92, 0.96, 1.0) * (1.0 - rim) * glass * 0.18
            + vec3<f32>(rim_highlight * 0.22);
        textureStore(out_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(clamp(mix(src.rgb, lens_rgb, influence), vec3<f32>(0.0), vec3<f32>(1.0)), src.a));
        return;
    }

    if (mode == 7) {
        let src = textureLoad(base_tex, vec2<i32>(i32(x), i32(y)), 0);
        let uv = (vec2<f32>(f32(x), f32(y)) + vec2<f32>(0.5)) / max(params.canvas.xy, vec2<f32>(1.0));
        let kind = i32(round(params.params.x));
        let scale = max(params.params.y, 0.001);
        let strength = clamp(params.params.z, 0.0, 1.0);
        let seed = params.canvas.z;
        let contrast = clamp(params.canvas.w, 0.0, 2.0);
        let asset_flags = params.extra.w;
        let has_texture = asset_flags >= 0.5;
        let has_height = asset_flags >= 1.5;
        var tex_value = fbm(uv * scale + vec2<f32>(seed, seed * 1.73));
        if (kind == 1) {
            let fibers = 0.5 + 0.5 * sin((uv.y * scale * 8.0 + tex_value * 4.0 + seed) * 6.28318);
            tex_value = mix(tex_value, fibers, 0.35);
        } else if (kind == 2) {
            tex_value = hash21(vec2<f32>(f32(x), f32(y)) + vec2<f32>(seed * 19.17, seed * 7.31));
        } else if (kind == 3) {
            tex_value = 0.5 + 0.5 * sin((uv.y * params.canvas.y * 0.85 + seed) * 6.28318);
        } else if (kind == 4) {
            let weave_x = 0.5 + 0.5 * sin((uv.x * scale * 10.0 + seed) * 6.28318);
            let weave_y = 0.5 + 0.5 * sin((uv.y * scale * 12.0 + seed * 1.37) * 6.28318);
            let ridges = sqrt(max(weave_x * weave_y, 0.0));
            tex_value = mix(tex_value, ridges, 0.55);
        } else if (kind == 5 || kind == 6) {
            let brush_angle = radians(params.extra.x);
            let bump_strength = clamp(params.extra.y, 0.0, 2.0);
            let relief = clamp(params.extra.z, 0.0, 2.0);
            let brush_x = uv.x * cos(brush_angle) - uv.y * sin(brush_angle);
            let brush_y = uv.x * sin(brush_angle) + uv.y * cos(brush_angle);
            let low = fbm(uv * scale * 0.18 + vec2<f32>(seed, seed * 0.61));
            let ridge = 0.5 + 0.5 * sin((brush_x * scale * 18.0 + low * 6.0 + seed) * 6.28318);
            let cross = 0.5 + 0.5 * sin((brush_y * scale * 3.0 + tex_value * 2.0 + seed * 0.7) * 6.28318);
            if (kind == 5) {
                tex_value = ridge * 0.62 + cross * 0.18 + low * 0.20;
            } else {
                tex_value = ridge * 0.50 + tex_value * 0.25 + low * 0.25;
            }
            tex_value = (tex_value - 0.5) * (1.0 + relief * 0.45 + bump_strength * 0.20) + 0.5;
        }
        let tiled_uv = fract(uv * scale);
        let texture_sample = textureSampleLevel(overlay_tex, post_sampler, tiled_uv, 0.0);
        let height_sample = textureSampleLevel(height_tex, post_sampler, tiled_uv, 0.0);
        let texture_luma = dot(texture_sample.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        let height_luma = dot(height_sample.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        tex_value = select(tex_value, select(texture_luma, height_luma, has_height), has_texture);
        let centered = clamp((tex_value - 0.5) * (1.0 + contrast) + 0.5, 0.0, 1.0);
        let material_bump = select(0.0, clamp(params.extra.y, 0.0, 2.0), kind >= 4);
        let shade = 1.0 + (centered - 0.5) * strength * (0.9 + material_bump * 0.55);
        let tint_shade = select(vec3<f32>(1.0), vec3<f32>(1.0) + (texture_sample.rgb - vec3<f32>(0.5)) * strength * 1.2, has_texture);
        textureStore(out_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(clamp(src.rgb * shade * tint_shade, vec3<f32>(0.0), vec3<f32>(1.0)), src.a));
        return;
    }

    let axis = i32(round(params.params.x));
    let radius = i32(round(clamp(params.params.y, 0.0, 64.0)));
    var acc = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var weight_sum = 0.0;

    for (var i = -64; i <= 64; i = i + 1) {
        if (abs(i) <= radius) {
            var sx = i32(x);
            var sy = i32(y);
            if (axis == 0) {
                sx = clamp(i32(x) + i, 0, i32(params.canvas.x) - 1);
            } else {
                sy = clamp(i32(y) + i, 0, i32(params.canvas.y) - 1);
            }
            let dist = f32(i) / max(f32(radius), 1.0);
            let weight = exp(-dist * dist * 2.5);
            acc = acc + textureLoad(base_tex, vec2<i32>(sx, sy), 0) * weight;
            weight_sum = weight_sum + weight;
        }
    }

    textureStore(out_tex, vec2<i32>(i32(x), i32(y)), acc / max(weight_sum, 0.0001));
}
