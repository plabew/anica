fn ml_distortion_warp_heat_haze(
    uv: vec2<f32>,
    noise: f32,
    amount: f32,
    direction: vec2<f32>,
) -> vec2<f32> {
    let dir_len = max(length(direction), 1e-5);
    let dir = direction / dir_len;
    let signed_noise = noise * 2.0 - 1.0;
    let offset = dir * (signed_noise * amount);
    return uv + offset;
}
