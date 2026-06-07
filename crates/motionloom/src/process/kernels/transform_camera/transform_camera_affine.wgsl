fn ml_transform_camera_affine(
    uv: vec2<f32>,
    center: vec2<f32>,
    scale: vec2<f32>,
    rotation_rad: f32,
    translate: vec2<f32>,
) -> vec2<f32> {
    let safe_scale = max(scale, vec2<f32>(1e-6, 1e-6));
    let local = (uv - center) / safe_scale;
    let c = cos(rotation_rad);
    let s = sin(rotation_rad);
    let rotated = vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);
    return rotated + center + translate;
}
