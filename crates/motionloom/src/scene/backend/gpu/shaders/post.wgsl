
struct PostParams {
    canvas: vec4<f32>,
    params: vec4<f32>,
};

@group(0) @binding(0) var base_tex: texture_2d<f32>;
@group(0) @binding(1) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: PostParams;

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
