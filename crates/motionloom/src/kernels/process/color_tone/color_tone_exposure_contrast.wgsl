fn ml_color_tone_exposure_contrast(
    rgb: vec3<f32>,
    exposure_ev: f32,
    contrast: f32,
    pivot: f32,
) -> vec3<f32> {
    let exposed = rgb * exp2(exposure_ev);
    return (exposed - vec3<f32>(pivot)) * contrast + vec3<f32>(pivot);
}
