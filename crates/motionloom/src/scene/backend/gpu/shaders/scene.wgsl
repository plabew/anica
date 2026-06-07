
struct Params {
    canvas: vec4<f32>,
    image: vec4<f32>,
    opacity: vec4<f32>,
};

@group(0) @binding(0) var base_tex: texture_2d<f32>;
@group(0) @binding(1) var image_tex: texture_2d<f32>;
@group(0) @binding(2) var image_sampler: sampler;
@group(0) @binding(3) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= u32(params.canvas.x) || y >= u32(params.canvas.y)) {
        return;
    }

    let pos = vec2<i32>(i32(x), i32(y));
    let base = textureLoad(base_tex, pos, 0);
    var out_color = base;

    let left = params.canvas.z;
    let top = params.canvas.w;
    let width = params.image.x;
    let height = params.image.y;
    let px = f32(x) + 0.5;
    let py = f32(y) + 0.5;

    if (width > 0.0 && height > 0.0 && px >= left && py >= top && px < left + width && py < top + height) {
        let uv = vec2<f32>((px - left) / width, (py - top) / height);
        let src = textureSampleLevel(image_tex, image_sampler, uv, 0.0);
        let src_a = clamp(src.a * params.opacity.x, 0.0, 1.0);
        let dst_a = base.a;
        let out_a = src_a + dst_a * (1.0 - src_a);
        if (out_a <= 0.000001) {
            out_color = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        } else {
            let rgb = (src.rgb * src_a + base.rgb * dst_a * (1.0 - src_a)) / out_a;
            out_color = vec4<f32>(rgb, out_a);
        }
    }

    textureStore(out_tex, pos, out_color);
}
