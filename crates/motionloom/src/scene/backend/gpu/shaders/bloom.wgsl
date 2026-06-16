
struct BloomParams {
    canvas: vec2<f32>,
    threshold: f32,
    intensity: f32,
};

@group(0) @binding(0) var original_tex: texture_2d<f32>;
@group(0) @binding(1) var blurred_tex: texture_2d<f32>;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var<uniform> params: BloomParams;

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= u32(params.canvas.x) || y >= u32(params.canvas.y)) {
        return;
    }

    let src = textureLoad(original_tex, vec2<i32>(i32(x), i32(y)), 0);
    let blur = textureLoad(blurred_tex, vec2<i32>(i32(x), i32(y)), 0);

    let threshold = params.threshold;
    let intensity = params.intensity;

    // Use the blurred texture to drive the halo. Thresholding only the current
    // source pixel brightens the original text but does not spread glow outward.
    let blur_lum = dot(blur.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let knee = max(0.001, threshold * 0.35);
    let t = smoothstep(knee, max(knee + 0.001, threshold), blur_lum);
    let glow = blur.rgb * t * intensity;

    let rgb = src.rgb + glow;
    textureStore(out_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), src.a));
}
