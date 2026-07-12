
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

fn turbulence(p_in: vec2<f32>) -> f32 {
    var p = p_in; var amp = 0.55; var sum = 0.0;
    for (var i = 0; i < 5; i = i + 1) {
        sum = sum + abs(noise2(p) * 2.0 - 1.0) * amp;
        p = p * 2.07 + vec2<f32>(13.7, 8.3); amp = amp * 0.5;
    }
    return clamp(sum, 0.0, 1.0);
}

fn cellular(p: vec2<f32>) -> f32 {
    let cell = floor(p); let local = fract(p); var nearest = 10.0;
    for (var oy = -1; oy <= 1; oy = oy + 1) {
        for (var ox = -1; ox <= 1; ox = ox + 1) {
            let offset = vec2<f32>(f32(ox), f32(oy));
            let point = vec2<f32>(hash21(cell + offset), hash21(cell + offset + vec2<f32>(31.7, 19.3)));
            nearest = min(nearest, length(offset + point - local));
        }
    }
    return clamp(nearest, 0.0, 1.0);
}

fn procedural_noise(kind: i32, p: vec2<f32>, seed: f32, coordinate_scale: f32) -> f32 {
    if (kind == 12) {
        let centered = p - vec2<f32>(coordinate_scale * 0.5);
        let polar = vec2<f32>(length(centered) * 2.4, atan2(centered.y, centered.x) / 6.2831853 * coordinate_scale * 0.3);
        return fbm(polar + vec2<f32>(seed, seed * 0.37));
    }
    let q = p + vec2<f32>(seed, seed * 1.73);
    if (kind == 7) { return turbulence(q); }
    if (kind == 8) { return 1.0 - abs(fbm(q) * 2.0 - 1.0); }
    if (kind == 9) { return cellular(q); }
    if (kind == 10) {
        let stretched = vec2<f32>(q.x * 1.9, q.y * 0.48);
        let warp = vec2<f32>(fbm(stretched * 0.38), fbm(stretched * 0.38 + vec2<f32>(17.2, 8.1))) - vec2<f32>(0.5);
        return fbm(stretched + warp * vec2<f32>(4.2, 1.8));
    }
    if (kind == 11) {
        let warp = fbm(vec2<f32>(q.x * 0.35, q.y * 0.22));
        return clamp(0.5
            + 0.28 * sin(q.y * 2.4 + warp * 5.0)
            + 0.18 * sin(q.y * 5.1 - q.x * 1.7 + warp * 3.0), 0.0, 1.0);
    }
    return fbm(q);
}

fn load_clamped(pixel: vec2<i32>) -> vec4<f32> {
    let limit = vec2<i32>(i32(params.canvas.x) - 1, i32(params.canvas.y) - 1);
    return textureLoad(base_tex, clamp(pixel, vec2<i32>(0), limit), 0);
}

fn local_alpha_edge(pixel: vec2<i32>, radius: i32) -> f32 {
    let r = max(radius, 1);
    var min_alpha = 1.0;
    var max_alpha = 0.0;
    let offsets = array<vec2<i32>, 9>(
        vec2<i32>(0, 0), vec2<i32>(r, 0), vec2<i32>(-r, 0),
        vec2<i32>(0, r), vec2<i32>(0, -r), vec2<i32>(r, r),
        vec2<i32>(-r, r), vec2<i32>(r, -r), vec2<i32>(-r, -r)
    );
    for (var i = 0; i < 9; i = i + 1) {
        let alpha = load_clamped(pixel + offsets[i]).a;
        min_alpha = min(min_alpha, alpha);
        max_alpha = max(max_alpha, alpha);
    }
    return clamp(max_alpha - min_alpha, 0.0, 1.0);
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

    if (mode == 9) {
        let uv = (vec2<f32>(f32(x), f32(y)) + vec2<f32>(0.5)) / max(params.canvas.xy, vec2<f32>(1.0));
        let scale = max(params.params.x, 0.001);
        let amount = params.params.y;
        let seed = params.params.z;
        let roughness = clamp(params.canvas.z, 0.0, 1.0);
        let specular = clamp(params.canvas.w, 0.0, 2.0);
        let kind = i32(round(params.extra.x));
        let n = procedural_noise(kind, uv * scale, seed, scale);
        let nx = procedural_noise(kind, (uv + vec2<f32>(1.0 / params.canvas.x, 0.0)) * scale, seed, scale) - n;
        let ny = procedural_noise(kind, (uv + vec2<f32>(0.0, 1.0 / params.canvas.y)) * scale, seed, scale) - n;
        let sample_xy = vec2<i32>(
            clamp(i32(f32(x) + nx * amount), 0, i32(params.canvas.x) - 1),
            clamp(i32(f32(y) + ny * amount), 0, i32(params.canvas.y) - 1)
        );
        let src = textureLoad(base_tex, sample_xy, 0);
        let normal = normalize(vec3<f32>(-nx * scale, -ny * scale, 1.0));
        let highlight = pow(max(dot(normal, normalize(vec3<f32>(-0.4, -0.5, 1.0))), 0.0), mix(48.0, 4.0, roughness)) * specular;
        textureStore(out_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(clamp(src.rgb + vec3<f32>(highlight), vec3<f32>(0.0), vec3<f32>(1.0)), src.a));
        return;
    }

    if (mode == 10) {
        let pixel = vec2<i32>(i32(x), i32(y));
        let src = load_clamped(pixel);
        let radius = max(i32(round(params.params.x)), 1);
        let amount = clamp(params.params.y, 0.0, 1.0);
        let preserve = clamp(params.extra.x, 0.0, 1.0);
        let edge = local_alpha_edge(pixel, radius);
        let offsets = array<vec2<i32>, 9>(
            vec2<i32>(0, 0), vec2<i32>(radius, 0), vec2<i32>(-radius, 0),
            vec2<i32>(0, radius), vec2<i32>(0, -radius), vec2<i32>(radius, radius),
            vec2<i32>(-radius, radius), vec2<i32>(radius, -radius), vec2<i32>(-radius, -radius)
        );
        var sum = vec4<f32>(0.0);
        var weight_sum = 0.0;
        for (var i = 0; i < 9; i = i + 1) {
            let weight = select(0.72, 1.6, i == 0);
            sum = sum + load_clamped(pixel + offsets[i]) * weight;
            weight_sum = weight_sum + weight;
        }
        let softened = sum / weight_sum;
        let edge_mix = edge * amount * (1.0 - preserve * src.a * 0.82);
        textureStore(out_tex, pixel, mix(src, softened, edge_mix));
        return;
    }

    if (mode == 11) {
        let pixel = vec2<i32>(i32(x), i32(y));
        let src = load_clamped(pixel);
        let amount = clamp(params.params.y, 0.0, 1.0);
        let scale = max(params.params.z, 0.001);
        let seed = params.extra.x;
        let edge = local_alpha_edge(pixel, 1);
        let p = vec2<f32>(f32(x), f32(y)) / scale;
        let nx = noise2(p + vec2<f32>(seed, seed * 1.37)) * 2.0 - 1.0;
        let ny = noise2(p + vec2<f32>(seed * 2.17 + 41.0, seed * 0.73 + 19.0)) * 2.0 - 1.0;
        let displacement = max(params.params.x, 0.0) * amount * edge;
        let sample_pixel = pixel + vec2<i32>(i32(round(nx * displacement)), i32(round(ny * displacement)));
        textureStore(out_tex, pixel, load_clamped(sample_pixel));
        return;
    }

    if (mode == 12) {
        let pixel = vec2<i32>(i32(x), i32(y));
        let src = load_clamped(pixel);
        let radius = max(i32(round(params.params.x)), 1);
        let amount = clamp(params.params.y, 0.0, 1.0);
        let offsets = array<vec2<i32>, 8>(
            vec2<i32>(radius, 0), vec2<i32>(-radius, 0), vec2<i32>(0, radius), vec2<i32>(0, -radius),
            vec2<i32>(radius, radius), vec2<i32>(-radius, radius), vec2<i32>(radius, -radius), vec2<i32>(-radius, -radius)
        );
        var donor = src;
        for (var i = 0; i < 8; i = i + 1) {
            let candidate = load_clamped(pixel + offsets[i]);
            if (candidate.a > donor.a) {
                donor = candidate;
            }
        }
        let edge = clamp(donor.a - src.a + local_alpha_edge(pixel, radius) * 0.35, 0.0, 1.0);
        let bleed = edge * amount;
        let out_alpha = max(src.a, donor.a * bleed);
        let out_rgb = mix(src.rgb, donor.rgb, bleed * (1.0 - src.a * 0.45));
        textureStore(out_tex, pixel, vec4<f32>(out_rgb, out_alpha));
        return;
    }

    if (mode == 13) {
        let pixel = vec2<i32>(i32(x), i32(y));
        let src = load_clamped(pixel);
        let steps = clamp(i32(round(params.params.x)), 1, 64);
        let amount = max(params.params.y, 0.0);
        let angle = params.params.z * 0.0174532925;
        let threshold = clamp(params.extra.x, 0.0, 0.999);
        let direction = vec2<f32>(cos(angle), sin(angle));
        var glow = vec3<f32>(0.0);
        for (var step = -64; step <= 64; step = step + 1) {
            if (abs(step) <= steps) {
                let sample_pixel = pixel + vec2<i32>(round(direction * f32(step)));
                let sample = load_clamped(sample_pixel);
                let luma = dot(sample.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
                let weight = max(0.0, (luma - threshold) / max(1.0 - threshold, 0.001)) / (1.0 + f32(abs(step)) * 0.12);
                glow = glow + sample.rgb * weight;
            }
        }
        textureStore(out_tex, pixel, vec4<f32>(clamp(src.rgb + glow * amount / f32(steps), vec3<f32>(0.0), vec3<f32>(1.0)), src.a));
        return;
    }

    if (mode == 14) {
        let pixel = vec2<i32>(i32(x), i32(y));
        let src = load_clamped(pixel);
        let shift = i32(round(params.params.x * params.params.y));
        let red = load_clamped(pixel + vec2<i32>(shift, 0)).r;
        let blue = load_clamped(pixel - vec2<i32>(shift, 0)).b;
        textureStore(out_tex, pixel, vec4<f32>(red, src.g, blue, src.a));
        return;
    }

    if (mode == 15) {
        let pixel = vec2<i32>(i32(x), i32(y));
        let src = load_clamped(pixel);
        let threshold = clamp(params.params.x, 0.0, 0.999);
        let amount = max(params.params.y, 0.0);
        let over = max(src.rgb - vec3<f32>(threshold), vec3<f32>(0.0));
        let compressed = vec3<f32>(threshold) + over / (vec3<f32>(1.0) + over * amount * 6.0);
        textureStore(out_tex, pixel, vec4<f32>(select(src.rgb, compressed, src.rgb > vec3<f32>(threshold)), src.a));
        return;
    }

    if (mode == 16) {
        let pixel = vec2<i32>(i32(x), i32(y));
        let src = load_clamped(pixel);
        let refraction = max(params.params.x, 0.0);
        let dispersion = max(params.params.y, 0.0);
        let glass = clamp(params.params.z, 0.0, 1.0);
        let uv = vec2<f32>(f32(x), f32(y)) / max(params.canvas.xy, vec2<f32>(1.0));
        let warp = vec2<f32>(noise2(uv * 9.0 + vec2<f32>(3.1, 7.7)), noise2(uv * 9.0 + vec2<f32>(19.4, 2.8))) * 2.0 - vec2<f32>(1.0);
        let offset = warp * refraction;
        let r = load_clamped(pixel + vec2<i32>(i32(round(offset.x + dispersion)), i32(round(offset.y)))).r;
        let g = load_clamped(pixel + vec2<i32>(i32(round(offset.x)), i32(round(offset.y)))).g;
        let b = load_clamped(pixel + vec2<i32>(i32(round(offset.x - dispersion)), i32(round(offset.y)))).b;
        let sheen = pow(max(0.0, 1.0 - length(uv - vec2<f32>(0.32, 0.28)) * 1.8), 5.0) * glass * 0.35;
        textureStore(out_tex, pixel, vec4<f32>(clamp(vec3<f32>(r, g, b) + vec3<f32>(sheen), vec3<f32>(0.0), vec3<f32>(1.0)), src.a));
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
        var tex_value = procedural_noise(kind, uv * scale, seed, scale);
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
