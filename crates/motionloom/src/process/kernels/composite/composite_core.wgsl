// composite_core.wgsl
//
// Canonical effect keys mapped to this file:
// - opacity
//
// This keeps compositing semantics separate from color grading kernels.

struct CompositeCoreParams {
    opacity: f32,
}

fn ml_composite_opacity(rgb: vec3<f32>, opacity: f32) -> vec3<f32> {
    return rgb * clamp(opacity, 0.0, 1.0);
}

// effect_id map:
//  1 opacity
fn ml_composite_core_dispatch(
    effect_id: u32,
    rgb: vec3<f32>,
    _uv: vec2<f32>,
    params: CompositeCoreParams
) -> vec3<f32> {
    switch effect_id {
        case 1u: {
            return ml_composite_opacity(rgb, params.opacity);
        }
        default: {
            return rgb;
        }
    }
}
