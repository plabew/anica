
struct TextureMatteParams {
    canvas: vec4<f32>,
    bounds: vec4<f32>,
    image: vec4<f32>,
    opacity: vec4<f32>,
    inv0: vec4<f32>,
    inv1: vec4<f32>,
    quad0: vec4<f32>,
    quad1: vec4<f32>,
    quad2: vec4<f32>,
    quad3: vec4<f32>,
};

@group(0) @binding(0) var base_tex: texture_2d<f32>;
@group(0) @binding(1) var image_tex: texture_2d<f32>;
@group(0) @binding(2) var matte_tex: texture_2d<f32>;
@group(0) @binding(3) var image_sampler: sampler;
@group(0) @binding(4) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(5) var<uniform> params: TextureMatteParams;

fn over(base: vec4<f32>, src_rgb: vec3<f32>, src_a: f32) -> vec4<f32> {
    let a = clamp(src_a, 0.0, 1.0);
    let out_a = a + base.a * (1.0 - a);
    if (out_a <= 0.000001) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let rgb = (src_rgb * a + base.rgb * base.a * (1.0 - a)) / out_a;
    return vec4<f32>(rgb, out_a);
}

fn blend_over(base: vec4<f32>, src_rgb: vec3<f32>, src_a: f32, mode: f32) -> vec4<f32> {
    let blend_mode = i32(round(mode));
    if (blend_mode == 0) {
        return over(base, src_rgb, src_a);
    }

    let a = clamp(src_a, 0.0, 1.0);
    if (a <= 0.0) {
        return base;
    }

    var blended = src_rgb;
    if (blend_mode == 1) {
        blended = src_rgb * base.rgb;
    } else if (blend_mode == 2) {
        let one = vec3<f32>(1.0, 1.0, 1.0);
        blended = one - (one - src_rgb) * (one - base.rgb);
    } else if (blend_mode == 3) {
        blended = min(src_rgb + base.rgb, vec3<f32>(1.0, 1.0, 1.0));
    }

    let out_a = a + base.a * (1.0 - a);
    if (out_a <= 0.000001) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let rgb = (blended * a + base.rgb * base.a * (1.0 - a)) / out_a;
    return vec4<f32>(rgb, out_a);
}

fn encode_pick_id(id_f: f32) -> vec4<f32> {
    let id = u32(round(id_f));
    let r = f32(id & 255u) / 255.0;
    let g = f32((id >> 8u) & 255u) / 255.0;
    let b = f32((id >> 16u) & 255u) / 255.0;
    return vec4<f32>(r, g, b, 1.0);
}

fn sample_source(local: vec2<f32>) -> vec4<f32> {
    if (params.canvas.z > 0.5) {
        let tx = clamp(i32(floor(local.x)), 0, i32(params.image.x) - 1);
        let ty = clamp(i32(floor(local.y)), 0, i32(params.image.y) - 1);
        return textureLoad(image_tex, vec2<i32>(tx, ty), 0);
    }
    let uv = vec2<f32>(local.x / params.image.x, local.y / params.image.y);
    return textureSampleLevel(image_tex, image_sampler, uv, 0.0);
}

fn sample_matte(local: vec2<f32>) -> vec4<f32> {
    if (params.canvas.w > 0.5) {
        let tx = clamp(i32(floor(local.x)), 0, i32(params.image.z) - 1);
        let ty = clamp(i32(floor(local.y)), 0, i32(params.image.w) - 1);
        return textureLoad(matte_tex, vec2<i32>(tx, ty), 0);
    }
    let matte_uv = vec2<f32>(local.x / params.image.z, local.y / params.image.w);
    return textureSampleLevel(matte_tex, image_sampler, matte_uv, 0.0);
}

fn barycentric(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> vec3<f32> {
    let v0 = b - a;
    let v1 = c - a;
    let v2 = p - a;
    let den = v0.x * v1.y - v1.x * v0.y;
    if (abs(den) <= 0.000001) {
        return vec3<f32>(-1.0, -1.0, -1.0);
    }
    let v = (v2.x * v1.y - v1.x * v2.y) / den;
    let w = (v0.x * v2.y - v2.x * v0.y) / den;
    let u = 1.0 - v - w;
    return vec3<f32>(u, v, w);
}

fn inside_tri(b: vec3<f32>) -> bool {
    return b.x >= -0.0005 && b.y >= -0.0005 && b.z >= -0.0005;
}

fn projective_local_from_tri(
    bary: vec3<f32>,
    uv0: vec2<f32>,
    uv1: vec2<f32>,
    uv2: vec2<f32>,
    q0: f32,
    q1: f32,
    q2: f32,
) -> vec3<f32> {
    let denom = bary.x * q0 + bary.y * q1 + bary.z * q2;
    if (abs(denom) <= 0.000001) {
        return vec3<f32>(-1.0, -1.0, 0.0);
    }
    let uv = (uv0 * bary.x * q0 + uv1 * bary.y * q1 + uv2 * bary.z * q2) / denom;
    return vec3<f32>(uv, 1.0);
}

fn projective_local(p: vec2<f32>) -> vec3<f32> {
    let p0 = params.quad0.xy;
    let p1 = params.quad1.xy;
    let p2 = params.quad2.xy;
    let p3 = params.quad3.xy;
    let uv0 = vec2<f32>(0.0, 0.0);
    let uv1 = vec2<f32>(params.image.x, 0.0);
    let uv2 = vec2<f32>(params.image.x, params.image.y);
    let uv3 = vec2<f32>(0.0, params.image.y);

    let b012 = barycentric(p, p0, p1, p2);
    if (inside_tri(b012)) {
        return projective_local_from_tri(b012, uv0, uv1, uv2, params.quad0.z, params.quad1.z, params.quad2.z);
    }
    let b023 = barycentric(p, p0, p2, p3);
    if (inside_tri(b023)) {
        return projective_local_from_tri(b023, uv0, uv2, uv3, params.quad0.z, params.quad2.z, params.quad3.z);
    }
    return vec3<f32>(-1.0, -1.0, 0.0);
}

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= u32(params.bounds.z) || gid.y >= u32(params.bounds.w)) {
        return;
    }

    let px_u = u32(params.bounds.x) + gid.x;
    let py_u = u32(params.bounds.y) + gid.y;
    if (px_u >= u32(params.canvas.x) || py_u >= u32(params.canvas.y)) {
        return;
    }

    let px = f32(px_u) + 0.5;
    let py = f32(py_u) + 0.5;
    var local = vec2<f32>(
        params.inv0.x * px + params.inv0.y * py + params.inv0.z,
        params.inv1.x * px + params.inv1.y * py + params.inv1.z
    );
    var projective_valid = true;
    if (params.inv0.w > 0.5) {
        let projected = projective_local(vec2<f32>(px, py));
        local = projected.xy;
        projective_valid = projected.z > 0.5;
    }

    let pos = vec2<i32>(i32(px_u), i32(py_u));
    let base = textureLoad(base_tex, pos, 0);
    var out_color = base;

    if (projective_valid && local.x >= 0.0 && local.y >= 0.0 && local.x < params.image.x && local.y < params.image.y) {
        let src = sample_source(local);
        var matte_factor = 1.0;
        let matte_mode = i32(round(params.opacity.z));
        if (matte_mode != 0) {
            if (local.x >= 0.0 && local.y >= 0.0 && local.x < params.image.z && local.y < params.image.w) {
                let matte = sample_matte(local);
                if (matte_mode == 2) {
                    matte_factor = dot(matte.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)) * matte.a;
                } else {
                    matte_factor = matte.a;
                }
            } else {
                matte_factor = 0.0;
            }
            if (params.opacity.w > 0.5) {
                matte_factor = 1.0 - matte_factor;
            }
        }
        let src_a = src.a * params.opacity.x * clamp(matte_factor, 0.0, 1.0);
        if (params.quad1.w > 0.5) {
            if (src_a > 0.001 && params.quad0.w > 0.5) {
                out_color = encode_pick_id(params.quad0.w);
            }
        } else {
            out_color = blend_over(base, src.rgb, src_a, params.opacity.y);
        }
    }

    textureStore(out_tex, pos, out_color);
}
