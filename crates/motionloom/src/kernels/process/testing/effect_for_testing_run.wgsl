fn ml_effect_for_testing_run(
    tex: texture_2d<f32>,
    samp: sampler,
    uv: vec2<f32>,
    texel: vec2<f32>,
) -> vec3<f32> {
    // Test-only blur helper. Runtime currently maps sigma from params and applies host-side preview blur.
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
