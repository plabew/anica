fn ml_stylize_look_posterize(rgb: vec3<f32>, levels: f32) -> vec3<f32> {
    let level_count = max(levels, 2.0);
    let steps = level_count - 1.0;
    return floor(rgb * steps + 0.5) / steps;
}
