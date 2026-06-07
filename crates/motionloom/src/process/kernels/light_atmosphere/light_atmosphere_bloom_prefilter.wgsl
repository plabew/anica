fn ml_light_atmosphere_bloom_prefilter(
    rgb: vec3<f32>,
    threshold: f32,
    knee: f32,
) -> vec3<f32> {
    let k = max(knee, 1e-5);
    let peak = max(max(rgb.r, rgb.g), rgb.b);
    let soft = clamp((peak - threshold + k) / (2.0 * k), 0.0, 1.0);
    let contribution = max(peak - threshold, 0.0) + soft * k;
    let gain = contribution / max(peak, 1e-5);
    return rgb * gain;
}
