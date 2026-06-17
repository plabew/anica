struct DownsampleParams {
    src_size: vec2<f32>,
    dst_size: vec2<f32>,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var<uniform> params: DownsampleParams;

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= u32(params.dst_size.x) || y >= u32(params.dst_size.y)) {
        return;
    }

    let uv = (vec2<f32>(f32(x), f32(y)) + vec2<f32>(0.5)) / max(params.dst_size, vec2<f32>(1.0));
    let color = textureSampleLevel(src_tex, src_sampler, uv, 0.0);
    textureStore(out_tex, vec2<i32>(i32(x), i32(y)), color);
}
