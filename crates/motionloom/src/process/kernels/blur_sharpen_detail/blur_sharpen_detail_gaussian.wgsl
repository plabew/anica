// Pass-level effect values (set on <Pass effect="...">):
// - gaussian_blur (alias of gaussian_5tap_h)
// - sharpen (alias of unsharp)
// - gaussian_5tap_h
// - gaussian_5tap_v
// - box
// - unsharp
fn ml_blur_sharpen_detail_gaussian_5tap_h(
    tex: texture_2d<f32>,
    samp: sampler,
    uv: vec2<f32>,
    texel: vec2<f32>,
) -> vec3<f32> {
    let w0 = 0.22702703;
    let w1 = 0.31621622;
    let w2 = 0.07027027;

    let c0 = textureSampleLevel(tex, samp, uv, 0.0).rgb * w0;
    let c1 =
        textureSampleLevel(tex, samp, uv + vec2<f32>(texel.x, 0.0), 0.0).rgb * w1 +
        textureSampleLevel(tex, samp, uv - vec2<f32>(texel.x, 0.0), 0.0).rgb * w1;
    let c2 =
        textureSampleLevel(tex, samp, uv + vec2<f32>(texel.x * 2.0, 0.0), 0.0).rgb * w2 +
        textureSampleLevel(tex, samp, uv - vec2<f32>(texel.x * 2.0, 0.0), 0.0).rgb * w2;
    return c0 + c1 + c2;
}

fn ml_blur_sharpen_detail_gaussian_5tap_v(
    tex: texture_2d<f32>,
    samp: sampler,
    uv: vec2<f32>,
    texel: vec2<f32>,
) -> vec3<f32> {
    let w0 = 0.22702703;
    let w1 = 0.31621622;
    let w2 = 0.07027027;

    let c0 = textureSampleLevel(tex, samp, uv, 0.0).rgb * w0;
    let c1 =
        textureSampleLevel(tex, samp, uv + vec2<f32>(0.0, texel.y), 0.0).rgb * w1 +
        textureSampleLevel(tex, samp, uv - vec2<f32>(0.0, texel.y), 0.0).rgb * w1;
    let c2 =
        textureSampleLevel(tex, samp, uv + vec2<f32>(0.0, texel.y * 2.0), 0.0).rgb * w2 +
        textureSampleLevel(tex, samp, uv - vec2<f32>(0.0, texel.y * 2.0), 0.0).rgb * w2;
    return c0 + c1 + c2;
}

fn ml_blur_sharpen_detail_box(
    tex: texture_2d<f32>,
    samp: sampler,
    uv: vec2<f32>,
    texel: vec2<f32>,
) -> vec3<f32> {
    var acc = vec3<f32>(0.0, 0.0, 0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let offset = vec2<f32>(f32(x) * texel.x, f32(y) * texel.y);
            acc = acc + textureSampleLevel(tex, samp, uv + offset, 0.0).rgb;
        }
    }
    return acc / 9.0;
}

fn ml_blur_sharpen_detail_unsharp(
    tex: texture_2d<f32>,
    samp: sampler,
    uv: vec2<f32>,
    texel: vec2<f32>,
) -> vec3<f32> {
    let base = textureSampleLevel(tex, samp, uv, 0.0).rgb;
    let blur = ml_blur_sharpen_detail_gaussian_5tap_h(tex, samp, uv, texel);
    let amount = 1.0;
    return clamp(base + (base - blur) * amount, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn ml_blur_sharpen_detail_sharpen(
    tex: texture_2d<f32>,
    samp: sampler,
    uv: vec2<f32>,
    texel: vec2<f32>,
) -> vec3<f32> {
    return ml_blur_sharpen_detail_unsharp(tex, samp, uv, texel);
}

fn ml_blur_sharpen_detail_gaussian(
    tex: texture_2d<f32>,
    samp: sampler,
    uv: vec2<f32>,
    texel: vec2<f32>,
) -> vec3<f32> {
    // Default helper when caller does not route by mode.
    return ml_blur_sharpen_detail_gaussian_5tap_h(tex, samp, uv, texel);
}
