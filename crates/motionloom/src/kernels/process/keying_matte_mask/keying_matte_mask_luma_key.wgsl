fn ml_keying_matte_mask_luma_key(
    rgb: vec3<f32>,
    threshold: f32,
    softness: f32,
) -> f32 {
    let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let edge = max(softness, 1e-5);
    let lo = threshold - edge * 0.5;
    let hi = threshold + edge * 0.5;
    return smoothstep(lo, hi, luma);
}
