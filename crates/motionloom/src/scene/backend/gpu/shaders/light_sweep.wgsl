struct LightSweepParams {
    canvas: vec4<f32>,
    sweep: vec4<f32>,
    color: vec4<f32>,
};

@group(0) @binding(0) var base_tex: texture_2d<f32>;
@group(0) @binding(1) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: LightSweepParams;

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    let width = params.canvas.x;
    let height = params.canvas.y;
    if (x >= u32(width) || y >= u32(height)) {
        return;
    }

    let src = textureLoad(base_tex, vec2<i32>(i32(x), i32(y)), 0);
    let uv = (vec2<f32>(f32(x), f32(y)) + vec2<f32>(0.5)) / max(vec2<f32>(width, height), vec2<f32>(1.0));
    let aspect = width / max(height, 1.0);
    let centered = vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5);
    let angle = radians(params.sweep.y);
    let normal = vec2<f32>(cos(angle), sin(angle));
    let position = (params.sweep.x - 0.5) * (aspect + 1.0);
    let distance = dot(centered, normal) - position;
    let half_width = max(params.sweep.z * 0.5, 0.0001);
    let softness = max(params.canvas.z, 0.0001);
    let band = 1.0 - smoothstep(half_width, half_width + softness, abs(distance));
    let energy = band * max(params.canvas.w, 0.0) * params.color.a;
    let rgb = src.rgb + params.color.rgb * energy;
    textureStore(out_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), src.a));
}
