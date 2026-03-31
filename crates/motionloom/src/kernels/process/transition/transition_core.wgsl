// transition_core.wgsl
//
// Single transition kernel family:
// - effect="fade_in"
// - effect="fade_out"
// - effect="dip"
// - effect="dissolve"
//
// Runtime currently evaluates transition behavior on CPU for preview logic.
// This file is kept as the canonical WGSL entry for cataloging and future GPU routing.

fn ml_transition_core(
    prev_tex: texture_2d<f32>,
    next_tex: texture_2d<f32>,
    samp: sampler,
    uv: vec2<f32>,
    progress: f32,
) -> vec4<f32> {
    let a = textureSampleLevel(prev_tex, samp, uv, 0.0);
    let b = textureSampleLevel(next_tex, samp, uv, 0.0);
    let t = clamp(progress, 0.0, 1.0);
    return a * (1.0 - t) + b * t;
}
