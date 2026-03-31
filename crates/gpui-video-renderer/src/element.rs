// =========================================
// =========================================
// crates/gpui-video-renderer/src/element.rs

#[cfg(target_os = "macos")]
use core_foundation::base::CFType;
#[cfg(target_os = "macos")]
use core_foundation::base::TCFType;
#[cfg(target_os = "macos")]
use core_foundation::boolean::CFBoolean;
#[cfg(target_os = "macos")]
use core_foundation::dictionary::CFDictionary;
#[cfg(target_os = "macos")]
use core_foundation::string::CFString;
#[cfg(target_os = "macos")]
use core_video::metal_texture_cache::CVMetalTextureCache;
#[cfg(target_os = "macos")]
use core_video::pixel_buffer::{
    CVPixelBuffer, CVPixelBufferKeys, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
};
use gpui::{
    Bounds, DefiniteLength, Element, ElementId, Entity, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Length, Pixels, RenderImage, Size, Style, Window,
};
use image::imageops;
use image::{ImageBuffer, Rgba};
#[cfg(target_os = "macos")]
use metal::foreign_types::ForeignTypeRef;
#[cfg(target_os = "macos")]
use metal::{
    CommandBuffer, CompileOptions, ComputeCommandEncoderRef, ComputePipelineState,
    ComputePipelineStateRef, Device, MTLCommandBufferStatus, MTLSize, MTLStorageMode,
    MTLTextureType, MTLTextureUsage, Texture, TextureDescriptor, TextureRef,
};
use smallvec::SmallVec;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::Hasher;
#[cfg(target_os = "macos")]
use std::mem;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use video_engine::Video;

const DEFAULT_FRAME_RAM_CACHE_MB: usize = 512;
pub const VIDEO_MAX_LOCAL_MASK_LAYERS: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoLocalMaskLayer {
    pub enabled: bool,
    pub center_x: f32,
    pub center_y: f32,
    pub radius: f32,
    pub feather: f32,
    pub strength: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub opacity: f32,
    pub blur_sigma: f32,
}

impl Default for VideoLocalMaskLayer {
    fn default() -> Self {
        Self {
            enabled: false,
            center_x: 0.5,
            center_y: 0.5,
            radius: 0.25,
            feather: 0.15,
            strength: 1.0,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            opacity: 1.0,
            blur_sigma: 0.0,
        }
    }
}

#[cfg(target_os = "macos")]
const METAL_GAUSSIAN_SHADER: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct BlurParams {
    float sigma;
    uint radius;
    uint width;
    uint height;
};

struct ColorParams {
    float brightness;
    float contrast;
    float saturation;
    float lut_mix;
    uint width;
    uint height;
    uint rotation_enabled;
    float rotation_cos;
    float rotation_sin;
    float transform_scale;
    float transform_pos_x;
    float transform_pos_y;
    float tint_y;
    float tint_u;
    float tint_v;
    float tint_alpha;
};

struct UnsharpParams {
    float amount;
    uint width;
    uint height;
    uint _pad;
};

inline float gaussian_weight(int x, float sigma) {
    return exp(-(float(x * x)) / (2.0f * sigma * sigma));
}

kernel void gaussian_blur_bgra_h(
    texture2d<float, access::read> src [[texture(0)]],
    texture2d<float, access::write> dst [[texture(1)]],
    constant BlurParams& params [[buffer(0)]],
    uint2 gid [[thread_position_in_grid]]
) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    if (params.radius == 0 || params.sigma < 0.001f) {
        dst.write(src.read(gid), gid);
        return;
    }

    float4 sum = float4(0.0f);
    float norm = 0.0f;
    int y = int(gid.y);
    int x0 = int(gid.x);
    int limit = min(int(params.radius), 64);

    for (int i = -64; i <= 64; i++) {
        if (abs(i) > limit) {
            continue;
        }
        int x = clamp(x0 + i, 0, int(params.width) - 1);
        float w = gaussian_weight(i, params.sigma);
        sum += src.read(uint2(uint(x), uint(y))) * w;
        norm += w;
    }

    dst.write(sum / max(norm, 1e-6f), gid);
}

kernel void gaussian_blur_bgra_v(
    texture2d<float, access::read> src [[texture(0)]],
    texture2d<float, access::write> dst [[texture(1)]],
    constant BlurParams& params [[buffer(0)]],
    uint2 gid [[thread_position_in_grid]]
) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    if (params.radius == 0 || params.sigma < 0.001f) {
        dst.write(src.read(gid), gid);
        return;
    }

    float4 sum = float4(0.0f);
    float norm = 0.0f;
    int x = int(gid.x);
    int y0 = int(gid.y);
    int limit = min(int(params.radius), 64);

    for (int i = -64; i <= 64; i++) {
        if (abs(i) > limit) {
            continue;
        }
        int y = clamp(y0 + i, 0, int(params.height) - 1);
        float w = gaussian_weight(i, params.sigma);
        sum += src.read(uint2(uint(x), uint(y))) * w;
        norm += w;
    }

    dst.write(sum / max(norm, 1e-6f), gid);
}

kernel void gaussian_blur_r8_h(
    texture2d<float, access::read> src [[texture(0)]],
    texture2d<float, access::write> dst [[texture(1)]],
    constant BlurParams& params [[buffer(0)]],
    uint2 gid [[thread_position_in_grid]]
) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    if (params.radius == 0 || params.sigma < 0.001f) {
        dst.write(src.read(gid), gid);
        return;
    }

    float4 sum = float4(0.0f);
    float norm = 0.0f;
    int y = int(gid.y);
    int x0 = int(gid.x);
    int limit = min(int(params.radius), 64);

    for (int i = -64; i <= 64; i++) {
        if (abs(i) > limit) {
            continue;
        }
        int x = clamp(x0 + i, 0, int(params.width) - 1);
        float w = gaussian_weight(i, params.sigma);
        sum += src.read(uint2(uint(x), uint(y))) * w;
        norm += w;
    }

    dst.write(sum / max(norm, 1e-6f), gid);
}

kernel void gaussian_blur_r8_v(
    texture2d<float, access::read> src [[texture(0)]],
    texture2d<float, access::write> dst [[texture(1)]],
    constant BlurParams& params [[buffer(0)]],
    uint2 gid [[thread_position_in_grid]]
) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    if (params.radius == 0 || params.sigma < 0.001f) {
        dst.write(src.read(gid), gid);
        return;
    }

    float4 sum = float4(0.0f);
    float norm = 0.0f;
    int x = int(gid.x);
    int y0 = int(gid.y);
    int limit = min(int(params.radius), 64);

    for (int i = -64; i <= 64; i++) {
        if (abs(i) > limit) {
            continue;
        }
        int y = clamp(y0 + i, 0, int(params.height) - 1);
        float w = gaussian_weight(i, params.sigma);
        sum += src.read(uint2(uint(x), uint(y))) * w;
        norm += w;
    }

    dst.write(sum / max(norm, 1e-6f), gid);
}

kernel void gaussian_blur_rg8_h(
    texture2d<float, access::read> src [[texture(0)]],
    texture2d<float, access::write> dst [[texture(1)]],
    constant BlurParams& params [[buffer(0)]],
    uint2 gid [[thread_position_in_grid]]
) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    if (params.radius == 0 || params.sigma < 0.001f) {
        dst.write(src.read(gid), gid);
        return;
    }

    float4 sum = float4(0.0f);
    float norm = 0.0f;
    int y = int(gid.y);
    int x0 = int(gid.x);
    int limit = min(int(params.radius), 64);

    for (int i = -64; i <= 64; i++) {
        if (abs(i) > limit) {
            continue;
        }
        int x = clamp(x0 + i, 0, int(params.width) - 1);
        float w = gaussian_weight(i, params.sigma);
        sum += src.read(uint2(uint(x), uint(y))) * w;
        norm += w;
    }

    dst.write(sum / max(norm, 1e-6f), gid);
}

kernel void gaussian_blur_rg8_v(
    texture2d<float, access::read> src [[texture(0)]],
    texture2d<float, access::write> dst [[texture(1)]],
    constant BlurParams& params [[buffer(0)]],
    uint2 gid [[thread_position_in_grid]]
) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    if (params.radius == 0 || params.sigma < 0.001f) {
        dst.write(src.read(gid), gid);
        return;
    }

    float4 sum = float4(0.0f);
    float norm = 0.0f;
    int x = int(gid.x);
    int y0 = int(gid.y);
    int limit = min(int(params.radius), 64);

    for (int i = -64; i <= 64; i++) {
        if (abs(i) > limit) {
            continue;
        }
        int y = clamp(y0 + i, 0, int(params.height) - 1);
        float w = gaussian_weight(i, params.sigma);
        sum += src.read(uint2(uint(x), uint(y))) * w;
        norm += w;
    }

    dst.write(sum / max(norm, 1e-6f), gid);
}

kernel void color_correct_nv12_y(
    texture2d<float, access::read> src [[texture(0)]],
    texture2d<float, access::write> dst [[texture(1)]],
    constant ColorParams& params [[buffer(0)]],
    uint2 gid [[thread_position_in_grid]]
) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    uint2 src_gid = gid;
    {
        float2 dims = float2(float(params.width), float(params.height));
        float aspect = dims.x / max(dims.y, 1e-6f);
        float2 out_uv = (float2(gid) + float2(0.5f, 0.5f)) / dims - float2(0.5f, 0.5f);
        float2 out_center = float2(out_uv.x * aspect, out_uv.y)
            - float2(params.transform_pos_x * aspect, params.transform_pos_y);
        float2 in_center = out_center;
        if (params.rotation_enabled != 0u) {
            in_center = float2(
                out_center.x * params.rotation_cos + out_center.y * params.rotation_sin,
                -out_center.x * params.rotation_sin + out_center.y * params.rotation_cos
            );
        }
        float inv_scale = 1.0f / max(params.transform_scale, 1e-6f);
        in_center *= inv_scale;
        float2 in_uv = float2(in_center.x / aspect, in_center.y) + float2(0.5f, 0.5f);
        if (in_uv.x < 0.0f || in_uv.x > 1.0f || in_uv.y < 0.0f || in_uv.y > 1.0f) {
            dst.write(float4(0.0f, 0.0f, 0.0f, 1.0f), gid);
            return;
        }
        float sx_f = clamp(in_uv.x * dims.x, 0.0f, dims.x - 1.0f);
        float sy_f = clamp(in_uv.y * dims.y, 0.0f, dims.y - 1.0f);
        src_gid = uint2(uint(sx_f), uint(sy_f));
    }
    float y = src.read(src_gid).r;
    y = (y - 0.5f) * params.contrast + 0.5f + params.brightness;
    y = clamp(y, 0.0f, 1.0f);
    if (params.lut_mix > 0.001f) {
        float m = clamp(params.lut_mix, 0.0f, 1.0f);
        float warm_y = clamp(y * 1.01f, 0.0f, 1.0f);
        y = mix(y, warm_y, m);
    }
    if (params.tint_alpha > 0.001f) {
        y = mix(y, params.tint_y, clamp(params.tint_alpha, 0.0f, 1.0f));
    }
    dst.write(float4(y, 0.0f, 0.0f, 1.0f), gid);
}

kernel void color_correct_nv12_uv(
    texture2d<float, access::read> src [[texture(0)]],
    texture2d<float, access::write> dst [[texture(1)]],
    constant ColorParams& params [[buffer(0)]],
    uint2 gid [[thread_position_in_grid]]
) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    uint2 src_gid = gid;
    {
        float2 dims = float2(float(params.width), float(params.height));
        float aspect = dims.x / max(dims.y, 1e-6f);
        float2 out_uv = (float2(gid) + float2(0.5f, 0.5f)) / dims - float2(0.5f, 0.5f);
        float2 out_center = float2(out_uv.x * aspect, out_uv.y)
            - float2(params.transform_pos_x * aspect, params.transform_pos_y);
        float2 in_center = out_center;
        if (params.rotation_enabled != 0u) {
            in_center = float2(
                out_center.x * params.rotation_cos + out_center.y * params.rotation_sin,
                -out_center.x * params.rotation_sin + out_center.y * params.rotation_cos
            );
        }
        float inv_scale = 1.0f / max(params.transform_scale, 1e-6f);
        in_center *= inv_scale;
        float2 in_uv = float2(in_center.x / aspect, in_center.y) + float2(0.5f, 0.5f);
        if (in_uv.x < 0.0f || in_uv.x > 1.0f || in_uv.y < 0.0f || in_uv.y > 1.0f) {
            dst.write(float4(0.5f, 0.5f, 0.0f, 1.0f), gid);
            return;
        }
        float sx_f = clamp(in_uv.x * dims.x, 0.0f, dims.x - 1.0f);
        float sy_f = clamp(in_uv.y * dims.y, 0.0f, dims.y - 1.0f);
        src_gid = uint2(uint(sx_f), uint(sy_f));
    }
    float2 uv = src.read(src_gid).rg;
    uv = (uv - float2(0.5f, 0.5f)) * params.saturation + float2(0.5f, 0.5f);
    uv = clamp(uv, float2(0.0f, 0.0f), float2(1.0f, 1.0f));
    if (params.lut_mix > 0.001f) {
        float m = clamp(params.lut_mix, 0.0f, 1.0f);
        float2 warm_uv = float2(
            clamp(uv.x * 0.98f + 0.01f, 0.0f, 1.0f),
            clamp(uv.y * 1.02f, 0.0f, 1.0f)
        );
        uv = mix(uv, warm_uv, m);
    }
    if (params.tint_alpha > 0.001f) {
        uv = mix(uv, float2(params.tint_u, params.tint_v), clamp(params.tint_alpha, 0.0f, 1.0f));
    }
    dst.write(float4(uv.x, uv.y, 0.0f, 1.0f), gid);
}

kernel void unsharp_nv12_y(
    texture2d<float, access::read> blurred [[texture(0)]],
    texture2d<float, access::write> dst [[texture(1)]],
    texture2d<float, access::read> orig [[texture(2)]],
    constant UnsharpParams& params [[buffer(0)]],
    uint2 gid [[thread_position_in_grid]]
) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    float amount = clamp(params.amount, 0.0f, 4.0f);
    float yb = blurred.read(gid).r;
    float yo = orig.read(gid).r;
    float y = clamp(yo + (yo - yb) * amount, 0.0f, 1.0f);
    dst.write(float4(y, 0.0f, 0.0f, 1.0f), gid);
}

kernel void unsharp_nv12_uv(
    texture2d<float, access::read> blurred [[texture(0)]],
    texture2d<float, access::write> dst [[texture(1)]],
    texture2d<float, access::read> orig [[texture(2)]],
    constant UnsharpParams& params [[buffer(0)]],
    uint2 gid [[thread_position_in_grid]]
) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    float amount = clamp(params.amount, 0.0f, 4.0f);
    float2 ub = blurred.read(gid).rg;
    float2 uo = orig.read(gid).rg;
    float2 uv = clamp(uo + (uo - ub) * amount, float2(0.0f, 0.0f), float2(1.0f, 1.0f));
    dst.write(float4(uv.x, uv.y, 0.0f, 1.0f), gid);
}
"#;

const WGPU_BGRA_EFFECT_SHADER: &str = r#"
struct BlurParams {
    sigma: f32,
    radius: u32,
    width: u32,
    height: u32,
};

struct ColorParams {
    brightness: f32,
    contrast: f32,
    saturation: f32,
    lut_mix: f32,
    opacity: f32,
    sharpen_amount: f32,
    width: u32,
    height: u32,
    local_mask_enabled: u32,
    rotation_enabled: u32,
    rotation_cos: f32,
    rotation_sin: f32,
    transform_scale: f32,
    transform_pos_x: f32,
    transform_pos_y: f32,
    local_mask_center_x: f32,
    local_mask_center_y: f32,
    local_mask_radius: f32,
    local_mask_feather: f32,
    local_mask_strength: f32,
    tint_r: f32,
    tint_g: f32,
    tint_b: f32,
    tint_alpha: f32,
};

@group(0) @binding(0)
var src_tex: texture_2d<f32>;
@group(0) @binding(1)
var dst_tex: texture_storage_2d<rgba8unorm, write>;

@group(0) @binding(2)
var<storage, read> blur_params: BlurParams;
@group(0) @binding(3)
var<storage, read> color_params: ColorParams;
@group(0) @binding(4)
var orig_tex: texture_2d<f32>;

fn gaussian_weight(i: i32, sigma: f32) -> f32 {
    return exp(-(f32(i * i)) / (2.0 * sigma * sigma));
}

@compute @workgroup_size(16, 16, 1)
fn blur_h(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= blur_params.width || gid.y >= blur_params.height) {
        return;
    }
    let sigma = max(blur_params.sigma, 0.001);
    let radius = min(i32(blur_params.radius), 64);
    if (radius == 0) {
        textureStore(dst_tex, vec2<i32>(i32(gid.x), i32(gid.y)), textureLoad(src_tex, vec2<i32>(i32(gid.x), i32(gid.y)), 0));
        return;
    }

    var sum: vec4<f32> = vec4<f32>(0.0);
    var norm: f32 = 0.0;
    let y = i32(gid.y);
    let x0 = i32(gid.x);
    let max_x = i32(blur_params.width) - 1;
    for (var i: i32 = -64; i <= 64; i = i + 1) {
        if (abs(i) > radius) {
            continue;
        }
        let x = clamp(x0 + i, 0, max_x);
        let w = gaussian_weight(i, sigma);
        sum = sum + textureLoad(src_tex, vec2<i32>(x, y), 0) * w;
        norm = norm + w;
    }
    textureStore(dst_tex, vec2<i32>(x0, y), sum / max(norm, 1e-6));
}

@compute @workgroup_size(16, 16, 1)
fn blur_v(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= blur_params.width || gid.y >= blur_params.height) {
        return;
    }
    let sigma = max(blur_params.sigma, 0.001);
    let radius = min(i32(blur_params.radius), 64);
    if (radius == 0) {
        textureStore(dst_tex, vec2<i32>(i32(gid.x), i32(gid.y)), textureLoad(src_tex, vec2<i32>(i32(gid.x), i32(gid.y)), 0));
        return;
    }

    var sum: vec4<f32> = vec4<f32>(0.0);
    var norm: f32 = 0.0;
    let x = i32(gid.x);
    let y0 = i32(gid.y);
    let max_y = i32(blur_params.height) - 1;
    for (var i: i32 = -64; i <= 64; i = i + 1) {
        if (abs(i) > radius) {
            continue;
        }
        let y = clamp(y0 + i, 0, max_y);
        let w = gaussian_weight(i, sigma);
        sum = sum + textureLoad(src_tex, vec2<i32>(x, y), 0) * w;
        norm = norm + w;
    }
    textureStore(dst_tex, vec2<i32>(x, y0), sum / max(norm, 1e-6));
}

@compute @workgroup_size(16, 16, 1)
fn color_correct(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= color_params.width || gid.y >= color_params.height) {
        return;
    }
    let coord = vec2<i32>(i32(gid.x), i32(gid.y));
    let dims = vec2<f32>(f32(color_params.width), max(f32(color_params.height), 1.0));
    var sample_coord = coord;
    {
        let aspect = dims.x / max(dims.y, 1e-6);
        let out_uv = (vec2<f32>(f32(gid.x) + 0.5, f32(gid.y) + 0.5) / dims) - vec2<f32>(0.5, 0.5);
        let out_center = vec2<f32>(out_uv.x * aspect, out_uv.y)
            - vec2<f32>(color_params.transform_pos_x * aspect, color_params.transform_pos_y);
        var src_centered = out_center;
        if (color_params.rotation_enabled != 0u) {
            src_centered = vec2<f32>(
                out_center.x * color_params.rotation_cos + out_center.y * color_params.rotation_sin,
                -out_center.x * color_params.rotation_sin + out_center.y * color_params.rotation_cos
            );
        }
        let inv_scale = 1.0 / max(color_params.transform_scale, 1e-6);
        src_centered = src_centered * inv_scale;
        let src_uv = vec2<f32>(src_centered.x / aspect, src_centered.y) + vec2<f32>(0.5, 0.5);
        if (src_uv.x < 0.0 || src_uv.x >= 1.0 || src_uv.y < 0.0 || src_uv.y >= 1.0) {
            textureStore(dst_tex, coord, vec4<f32>(0.0, 0.0, 0.0, 0.0));
            return;
        }
        sample_coord = vec2<i32>(
            clamp(i32(src_uv.x * dims.x), 0, i32(color_params.width) - 1),
            clamp(i32(src_uv.y * dims.y), 0, i32(color_params.height) - 1)
        );
    }
    let px = textureLoad(src_tex, sample_coord, 0);
    var px_effect = px;
    if (color_params.sharpen_amount > 0.001) {
        let original_px = textureLoad(orig_tex, sample_coord, 0);
        let amount = clamp(color_params.sharpen_amount, 0.0, 4.0);
        px_effect = clamp(
            original_px + (original_px - px) * amount,
            vec4<f32>(0.0),
            vec4<f32>(1.0)
        );
    }

    // Input bytes are BGRA. In RGBA texture terms: r=B, g=G, b=R.
    var b = px_effect.r;
    var g = px_effect.g;
    var r = px_effect.b;
    var a = px_effect.a;

    let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    r = lum + (r - lum) * color_params.saturation;
    g = lum + (g - lum) * color_params.saturation;
    b = lum + (b - lum) * color_params.saturation;

    let bright = color_params.brightness * 255.0;
    r = clamp((((r * 255.0) - 128.0) * color_params.contrast + 128.0 + bright) / 255.0, 0.0, 1.0);
    g = clamp((((g * 255.0) - 128.0) * color_params.contrast + 128.0 + bright) / 255.0, 0.0, 1.0);
    b = clamp((((b * 255.0) - 128.0) * color_params.contrast + 128.0 + bright) / 255.0, 0.0, 1.0);
    if (color_params.lut_mix > 0.001) {
        let m = clamp(color_params.lut_mix, 0.0, 1.0);
        let warm_r = clamp(r * 1.03, 0.0, 1.0);
        let warm_g = g;
        let warm_b = clamp(b * 0.97, 0.0, 1.0);
        r = mix(r, warm_r, m);
        g = mix(g, warm_g, m);
        b = mix(b, warm_b, m);
    }
    a = clamp(a * color_params.opacity, 0.0, 1.0);

    var out_px = vec4<f32>(b, g, r, a);
    if (color_params.local_mask_enabled != 0u) {
        let original_px = textureLoad(orig_tex, sample_coord, 0);
        let uv = (vec2<f32>(f32(gid.x) + 0.5, f32(gid.y) + 0.5) / dims);
        var delta = uv - vec2<f32>(color_params.local_mask_center_x, color_params.local_mask_center_y);
        let aspect = dims.x / dims.y;
        delta.x = delta.x * aspect;
        let dist = length(delta);
        let radius = clamp(color_params.local_mask_radius, 0.0, 1.0);
        let feather = max(color_params.local_mask_feather, 1e-6);
        var mask = 1.0 - smoothstep(radius, radius + feather, dist);
        mask = clamp(mask * color_params.local_mask_strength, 0.0, 1.0);
        out_px = mix(original_px, out_px, mask);
    }

    if (color_params.tint_alpha > 0.001) {
        let ta = clamp(color_params.tint_alpha, 0.0, 1.0);
        out_px = vec4<f32>(
            mix(out_px.r, color_params.tint_b, ta),
            mix(out_px.g, color_params.tint_g, ta),
            mix(out_px.b, color_params.tint_r, ta),
            out_px.a
        );
    }

    // Store back as BGRA in byte representation.
    textureStore(dst_tex, coord, out_px);
}
"#;

const WGPU_BGRA_WORKGROUP_SIZE: u32 = 16;

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct MetalBlurParams {
    sigma: f32,
    radius: u32,
    width: u32,
    height: u32,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct MetalColorParams {
    brightness: f32,
    contrast: f32,
    saturation: f32,
    lut_mix: f32,
    width: u32,
    height: u32,
    rotation_enabled: u32,
    rotation_cos: f32,
    rotation_sin: f32,
    transform_scale: f32,
    transform_pos_x: f32,
    transform_pos_y: f32,
    tint_y: f32,
    tint_u: f32,
    tint_v: f32,
    tint_alpha: f32,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct MetalUnsharpParams {
    amount: f32,
    width: u32,
    height: u32,
    _pad: u32,
}

#[cfg(target_os = "macos")]
struct MetalGaussianBlurContext {
    device: Device,
    queue: metal::CommandQueue,
    core_video_texture_cache: CVMetalTextureCache,
    blur_h_r8: ComputePipelineState,
    blur_v_r8: ComputePipelineState,
    blur_h_rg8: ComputePipelineState,
    blur_v_rg8: ComputePipelineState,
    color_y_r8: ComputePipelineState,
    color_uv_rg8: ComputePipelineState,
    unsharp_y_r8: ComputePipelineState,
    unsharp_uv_rg8: ComputePipelineState,
}

#[cfg(target_os = "macos")]
impl MetalGaussianBlurContext {
    fn new() -> Result<Self, String> {
        let device = Device::system_default().ok_or("Metal unavailable on this device")?;
        let queue = device.new_command_queue();
        let core_video_texture_cache = CVMetalTextureCache::new(None, device.clone(), None)
            .map_err(|status| format!("CVMetalTextureCache::new failed: status={status}"))?;
        let library =
            device.new_library_with_source(METAL_GAUSSIAN_SHADER, &CompileOptions::new())?;
        let func_h_r8 = library.get_function("gaussian_blur_r8_h", None)?;
        let func_v_r8 = library.get_function("gaussian_blur_r8_v", None)?;
        let func_h_rg8 = library.get_function("gaussian_blur_rg8_h", None)?;
        let func_v_rg8 = library.get_function("gaussian_blur_rg8_v", None)?;
        let func_color_y = library.get_function("color_correct_nv12_y", None)?;
        let func_color_uv = library.get_function("color_correct_nv12_uv", None)?;
        let func_unsharp_y = library.get_function("unsharp_nv12_y", None)?;
        let func_unsharp_uv = library.get_function("unsharp_nv12_uv", None)?;
        let blur_h_r8 = device.new_compute_pipeline_state_with_function(&func_h_r8)?;
        let blur_v_r8 = device.new_compute_pipeline_state_with_function(&func_v_r8)?;
        let blur_h_rg8 = device.new_compute_pipeline_state_with_function(&func_h_rg8)?;
        let blur_v_rg8 = device.new_compute_pipeline_state_with_function(&func_v_rg8)?;
        let color_y_r8 = device.new_compute_pipeline_state_with_function(&func_color_y)?;
        let color_uv_rg8 = device.new_compute_pipeline_state_with_function(&func_color_uv)?;
        let unsharp_y_r8 = device.new_compute_pipeline_state_with_function(&func_unsharp_y)?;
        let unsharp_uv_rg8 = device.new_compute_pipeline_state_with_function(&func_unsharp_uv)?;
        Ok(Self {
            device,
            queue,
            core_video_texture_cache,
            blur_h_r8,
            blur_v_r8,
            blur_h_rg8,
            blur_v_rg8,
            color_y_r8,
            color_uv_rg8,
            unsharp_y_r8,
            unsharp_uv_rg8,
        })
    }

    fn make_texture_with_format(
        &self,
        width: u32,
        height: u32,
        pixel_format: metal::MTLPixelFormat,
    ) -> Texture {
        let desc = TextureDescriptor::new();
        desc.set_texture_type(MTLTextureType::D2);
        desc.set_width(width as u64);
        desc.set_height(height as u64);
        desc.set_pixel_format(pixel_format);
        desc.set_storage_mode(MTLStorageMode::Shared);
        desc.set_usage(MTLTextureUsage::ShaderRead | MTLTextureUsage::ShaderWrite);
        self.device.new_texture(&desc)
    }

    fn encode_pass(
        encoder: &ComputeCommandEncoderRef,
        pipeline: &ComputePipelineStateRef,
        src: &TextureRef,
        dst: &TextureRef,
        params: &MetalBlurParams,
    ) {
        encoder.set_compute_pipeline_state(pipeline);
        encoder.set_texture(0, Some(src));
        encoder.set_texture(1, Some(dst));
        encoder.set_bytes(
            0,
            mem::size_of::<MetalBlurParams>() as u64,
            params as *const MetalBlurParams as *const std::ffi::c_void,
        );

        let threads_w = pipeline.thread_execution_width().max(1);
        let max_threads = pipeline.max_total_threads_per_threadgroup().max(threads_w);
        let threads_h = (max_threads / threads_w).clamp(1, 16);
        let threads_per_group = MTLSize::new(threads_w, threads_h, 1);
        let threads_per_grid = MTLSize::new(params.width as u64, params.height as u64, 1);
        encoder.dispatch_threads(threads_per_grid, threads_per_group);
    }

    fn encode_color_pass(
        encoder: &ComputeCommandEncoderRef,
        pipeline: &ComputePipelineStateRef,
        src: &TextureRef,
        dst: &TextureRef,
        params: &MetalColorParams,
    ) {
        encoder.set_compute_pipeline_state(pipeline);
        encoder.set_texture(0, Some(src));
        encoder.set_texture(1, Some(dst));
        encoder.set_bytes(
            0,
            mem::size_of::<MetalColorParams>() as u64,
            params as *const MetalColorParams as *const std::ffi::c_void,
        );

        let threads_w = pipeline.thread_execution_width().max(1);
        let max_threads = pipeline.max_total_threads_per_threadgroup().max(threads_w);
        let threads_h = (max_threads / threads_w).clamp(1, 16);
        let threads_per_group = MTLSize::new(threads_w, threads_h, 1);
        let threads_per_grid = MTLSize::new(params.width as u64, params.height as u64, 1);
        encoder.dispatch_threads(threads_per_grid, threads_per_group);
    }

    fn as_texture_ref_from_cv_metal_texture<'a>(
        cv_texture: &'a core_video::metal_texture::CVMetalTexture,
    ) -> Result<&'a TextureRef, String> {
        let raw = unsafe {
            core_video::metal_texture::CVMetalTextureGetTexture(cv_texture.as_concrete_TypeRef())
        };
        if raw.is_null() {
            return Err("CVMetalTextureGetTexture returned null".to_string());
        }
        Ok(unsafe { TextureRef::from_ptr(raw as *mut _) })
    }

    fn encode_blur_two_pass(
        &self,
        command_buffer: &metal::CommandBufferRef,
        src: &TextureRef,
        tmp: &TextureRef,
        dst: &TextureRef,
        width: u32,
        height: u32,
        sigma: f32,
        blur_h: &ComputePipelineStateRef,
        blur_v: &ComputePipelineStateRef,
    ) {
        let radius = ((sigma * 3.0).ceil() as u32).clamp(0, 64);
        let params = MetalBlurParams {
            sigma: sigma.max(0.001),
            radius,
            width,
            height,
        };

        {
            let encoder = command_buffer.new_compute_command_encoder();
            Self::encode_pass(encoder, blur_h, src, tmp, &params);
            encoder.end_encoding();
        }
        {
            let encoder = command_buffer.new_compute_command_encoder();
            Self::encode_pass(encoder, blur_v, tmp, dst, &params);
            encoder.end_encoding();
        }
    }

    fn encode_color_dispatch(
        &self,
        command_buffer: &metal::CommandBufferRef,
        src: &TextureRef,
        dst: &TextureRef,
        width: u32,
        height: u32,
        brightness: f32,
        contrast: f32,
        saturation: f32,
        lut_mix: f32,
        rotation_deg: f32,
        transform_scale: f32,
        transform_pos_x: f32,
        transform_pos_y: f32,
        transform_ref_width: f32,
        transform_ref_height: f32,
        tint_y: f32,
        tint_u: f32,
        tint_v: f32,
        tint_alpha: f32,
        pipeline: &ComputePipelineStateRef,
    ) {
        let angle = rotation_deg.to_radians();
        let width_f = width.max(1) as f32;
        let height_f = height.max(1) as f32;
        let ref_w = transform_ref_width.max(1.0);
        let ref_h = transform_ref_height.max(1.0);
        let pos_x_norm = transform_pos_x * (ref_w / width_f);
        let pos_y_norm = transform_pos_y * (ref_h / height_f);
        let params = MetalColorParams {
            brightness,
            contrast,
            saturation,
            lut_mix: lut_mix.clamp(0.0, 1.0),
            width,
            height,
            rotation_enabled: if rotation_deg.abs() >= 0.001 { 1 } else { 0 },
            rotation_cos: angle.cos(),
            rotation_sin: angle.sin(),
            transform_scale: transform_scale.clamp(0.01, 5.0),
            transform_pos_x: pos_x_norm,
            transform_pos_y: pos_y_norm,
            tint_y: tint_y.clamp(0.0, 1.0),
            tint_u: tint_u.clamp(0.0, 1.0),
            tint_v: tint_v.clamp(0.0, 1.0),
            tint_alpha: tint_alpha.clamp(0.0, 1.0),
        };
        {
            let encoder = command_buffer.new_compute_command_encoder();
            Self::encode_color_pass(encoder, pipeline, src, dst, &params);
            encoder.end_encoding();
        }
    }

    fn encode_unsharp_pass(
        encoder: &ComputeCommandEncoderRef,
        pipeline: &ComputePipelineStateRef,
        blurred: &TextureRef,
        dst: &TextureRef,
        orig: &TextureRef,
        params: &MetalUnsharpParams,
    ) {
        encoder.set_compute_pipeline_state(pipeline);
        encoder.set_texture(0, Some(blurred));
        encoder.set_texture(1, Some(dst));
        encoder.set_texture(2, Some(orig));
        encoder.set_bytes(
            0,
            mem::size_of::<MetalUnsharpParams>() as u64,
            params as *const MetalUnsharpParams as *const std::ffi::c_void,
        );

        let threads_w = pipeline.thread_execution_width().max(1);
        let max_threads = pipeline.max_total_threads_per_threadgroup().max(threads_w);
        let threads_h = (max_threads / threads_w).clamp(1, 16);
        let threads_per_group = MTLSize::new(threads_w, threads_h, 1);
        let threads_per_grid = MTLSize::new(params.width as u64, params.height as u64, 1);
        encoder.dispatch_threads(threads_per_grid, threads_per_group);
    }

    fn encode_unsharp_dispatch(
        &self,
        command_buffer: &metal::CommandBufferRef,
        blurred: &TextureRef,
        dst: &TextureRef,
        orig: &TextureRef,
        width: u32,
        height: u32,
        amount: f32,
        pipeline: &ComputePipelineStateRef,
    ) {
        let params = MetalUnsharpParams {
            amount: amount.clamp(0.0, 4.0),
            width,
            height,
            _pad: 0,
        };
        let encoder = command_buffer.new_compute_command_encoder();
        Self::encode_unsharp_pass(encoder, pipeline, blurred, dst, orig, &params);
        encoder.end_encoding();
    }

    fn process_nv12_surface_zero_copy(
        &mut self,
        source_surface: &CVPixelBuffer,
        sigma: f32,
        brightness: f32,
        contrast: f32,
        saturation: f32,
        lut_mix: f32,
        rotation_deg: f32,
        transform_scale: f32,
        transform_pos_x: f32,
        transform_pos_y: f32,
        transform_ref_width: f32,
        transform_ref_height: f32,
        tint_hue: f32,
        tint_saturation: f32,
        tint_lightness: f32,
        tint_alpha: f32,
    ) -> Result<CVPixelBuffer, String> {
        let sigma_abs = sigma.abs();
        let has_blur = sigma_abs > 0.001;
        let has_sharpen = sigma < -0.001;
        let has_color = brightness.abs() >= 0.001
            || (contrast - 1.0).abs() >= 0.001
            || (saturation - 1.0).abs() >= 0.001
            || lut_mix.abs() >= 0.001
            || rotation_deg.abs() >= 0.001
            || (transform_scale - 1.0).abs() >= 0.001
            || transform_pos_x.abs() >= 0.001
            || transform_pos_y.abs() >= 0.001
            || tint_alpha.abs() >= 0.001;
        if !has_blur && !has_color {
            return Ok(source_surface.clone());
        }
        let (tr, tg, tb) = hsla_to_rgb(
            tint_hue,
            tint_saturation.clamp(0.0, 1.0),
            tint_lightness.clamp(0.0, 1.0),
        );
        let tint_y = (0.299 * tr + 0.587 * tg + 0.114 * tb).clamp(0.0, 1.0);
        let tint_u = (-0.168_736 * tr - 0.331_264 * tg + 0.5 * tb + 0.5).clamp(0.0, 1.0);
        let tint_v = (0.5 * tr - 0.418_688 * tg - 0.081_312 * tb + 0.5).clamp(0.0, 1.0);
        let tint_alpha = tint_alpha.clamp(0.0, 1.0);

        let pixel_format = source_surface.get_pixel_format();
        if pixel_format != kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
            && pixel_format != kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        {
            return Err(format!("unsupported source NV12 format: {pixel_format:#x}"));
        }
        if source_surface.get_plane_count() < 2 {
            return Err("source surface has no NV12 planes".to_string());
        }

        let width = source_surface.get_width() as u32;
        let height = source_surface.get_height() as u32;
        let y_w = source_surface.get_width_of_plane(0) as u32;
        let y_h = source_surface.get_height_of_plane(0) as u32;
        let uv_w = source_surface.get_width_of_plane(1) as u32;
        let uv_h = source_surface.get_height_of_plane(1) as u32;
        if width == 0 || height == 0 || y_w == 0 || y_h == 0 || uv_w == 0 || uv_h == 0 {
            return Err("source surface has invalid dimensions".to_string());
        }

        let iosurface_props: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[]);
        let metal_surface_options: CFDictionary<CFString, CFType> =
            CFDictionary::from_CFType_pairs(&[
                (
                    CFString::from(CVPixelBufferKeys::MetalCompatibility),
                    CFBoolean::true_value().as_CFType(),
                ),
                (
                    CFString::from(CVPixelBufferKeys::IOSurfaceProperties),
                    iosurface_props.as_CFType(),
                ),
            ]);
        let attrs = source_surface.copy_creation_attributes();
        let mut dest_surface = CVPixelBuffer::new(
            pixel_format,
            width as usize,
            height as usize,
            attrs.as_ref(),
        )
        .or_else(|_| {
            CVPixelBuffer::new(
                pixel_format,
                width as usize,
                height as usize,
                Some(&metal_surface_options),
            )
        })
        .or_else(|_| CVPixelBuffer::new(pixel_format, width as usize, height as usize, None))
        .map_err(|status| format!("CVPixelBuffer::new failed: status={status}"))?;

        let src_y_cv = self
            .core_video_texture_cache
            .create_texture_from_image(
                source_surface.as_concrete_TypeRef(),
                None,
                metal::MTLPixelFormat::R8Unorm,
                y_w as usize,
                y_h as usize,
                0,
            )
            .map_err(|status| format!("create src Y texture failed: status={status}"))?;
        let src_uv_cv = self
            .core_video_texture_cache
            .create_texture_from_image(
                source_surface.as_concrete_TypeRef(),
                None,
                metal::MTLPixelFormat::RG8Unorm,
                uv_w as usize,
                uv_h as usize,
                1,
            )
            .map_err(|status| format!("create src UV texture failed: status={status}"))?;
        let mut dst_y_cv_result = self.core_video_texture_cache.create_texture_from_image(
            dest_surface.as_concrete_TypeRef(),
            None,
            metal::MTLPixelFormat::R8Unorm,
            y_w as usize,
            y_h as usize,
            0,
        );
        let mut dst_uv_cv_result = self.core_video_texture_cache.create_texture_from_image(
            dest_surface.as_concrete_TypeRef(),
            None,
            metal::MTLPixelFormat::RG8Unorm,
            uv_w as usize,
            uv_h as usize,
            1,
        );
        if dst_y_cv_result.is_err() || dst_uv_cv_result.is_err() {
            let dst_y_status = dst_y_cv_result.as_ref().err().copied();
            let dst_uv_status = dst_uv_cv_result.as_ref().err().copied();
            if nv12_debug_enabled() {
                log::warn!(
                    "[VideoElement][NV12FX] dst texture create failed -> retry force-metal fmt={}({:#x}) y_status={:?} uv_status={:?}",
                    nv12_pixel_format_tag(pixel_format),
                    pixel_format,
                    dst_y_status,
                    dst_uv_status
                );
            }
            dest_surface = CVPixelBuffer::new(
                pixel_format,
                width as usize,
                height as usize,
                Some(&metal_surface_options),
            )
            .or_else(|_| CVPixelBuffer::new(pixel_format, width as usize, height as usize, None))
            .map_err(|status| {
                format!("CVPixelBuffer::new (force metal attrs) failed: status={status}")
            })?;
            dst_y_cv_result = self.core_video_texture_cache.create_texture_from_image(
                dest_surface.as_concrete_TypeRef(),
                None,
                metal::MTLPixelFormat::R8Unorm,
                y_w as usize,
                y_h as usize,
                0,
            );
            dst_uv_cv_result = self.core_video_texture_cache.create_texture_from_image(
                dest_surface.as_concrete_TypeRef(),
                None,
                metal::MTLPixelFormat::RG8Unorm,
                uv_w as usize,
                uv_h as usize,
                1,
            );
        }
        let dst_y_cv = dst_y_cv_result
            .map_err(|status| format!("create dst Y texture failed: status={status}"))?;
        let dst_uv_cv = dst_uv_cv_result
            .map_err(|status| format!("create dst UV texture failed: status={status}"))?;

        let src_y = Self::as_texture_ref_from_cv_metal_texture(&src_y_cv)?;
        let src_uv = Self::as_texture_ref_from_cv_metal_texture(&src_uv_cv)?;
        let dst_y = Self::as_texture_ref_from_cv_metal_texture(&dst_y_cv)?;
        let dst_uv = Self::as_texture_ref_from_cv_metal_texture(&dst_uv_cv)?;
        let command_buffer = self.queue.new_command_buffer();

        if has_blur {
            let uv_ref_w = transform_ref_width * ((uv_w as f32) / (y_w.max(1) as f32));
            let uv_ref_h = transform_ref_height * ((uv_h as f32) / (y_h.max(1) as f32));
            let mut effect_y: &TextureRef = src_y;
            let mut effect_uv: &TextureRef = src_uv;
            let mut stage_output_y: Option<Texture> = None;
            let mut stage_output_uv: Option<Texture> = None;

            if has_sharpen {
                let sharpen_stages = sharpen_stages_for_sigma(sigma_abs);
                for (idx, stage) in sharpen_stages.iter().enumerate() {
                    let input_y: &TextureRef = if let Some(tex) = stage_output_y.as_ref() {
                        tex.as_ref()
                    } else {
                        effect_y
                    };
                    let input_uv: &TextureRef = if let Some(tex) = stage_output_uv.as_ref() {
                        tex.as_ref()
                    } else {
                        effect_uv
                    };
                    let tmp_y =
                        self.make_texture_with_format(y_w, y_h, metal::MTLPixelFormat::R8Unorm);
                    let tmp_uv =
                        self.make_texture_with_format(uv_w, uv_h, metal::MTLPixelFormat::RG8Unorm);
                    let blur_y =
                        self.make_texture_with_format(y_w, y_h, metal::MTLPixelFormat::R8Unorm);
                    let blur_uv =
                        self.make_texture_with_format(uv_w, uv_h, metal::MTLPixelFormat::RG8Unorm);

                    self.encode_blur_two_pass(
                        &command_buffer,
                        input_y,
                        tmp_y.as_ref(),
                        blur_y.as_ref(),
                        y_w,
                        y_h,
                        stage.sigma,
                        &self.blur_h_r8,
                        &self.blur_v_r8,
                    );
                    self.encode_blur_two_pass(
                        &command_buffer,
                        input_uv,
                        tmp_uv.as_ref(),
                        blur_uv.as_ref(),
                        uv_w,
                        uv_h,
                        stage.sigma,
                        &self.blur_h_rg8,
                        &self.blur_v_rg8,
                    );

                    let is_last = idx + 1 == sharpen_stages.len();
                    if is_last && !has_color {
                        self.encode_unsharp_dispatch(
                            &command_buffer,
                            blur_y.as_ref(),
                            dst_y,
                            input_y,
                            y_w,
                            y_h,
                            stage.amount,
                            &self.unsharp_y_r8,
                        );
                        self.encode_unsharp_dispatch(
                            &command_buffer,
                            blur_uv.as_ref(),
                            dst_uv,
                            input_uv,
                            uv_w,
                            uv_h,
                            stage.amount,
                            &self.unsharp_uv_rg8,
                        );
                    } else {
                        let sharp_y =
                            self.make_texture_with_format(y_w, y_h, metal::MTLPixelFormat::R8Unorm);
                        let sharp_uv = self.make_texture_with_format(
                            uv_w,
                            uv_h,
                            metal::MTLPixelFormat::RG8Unorm,
                        );
                        self.encode_unsharp_dispatch(
                            &command_buffer,
                            blur_y.as_ref(),
                            sharp_y.as_ref(),
                            input_y,
                            y_w,
                            y_h,
                            stage.amount,
                            &self.unsharp_y_r8,
                        );
                        self.encode_unsharp_dispatch(
                            &command_buffer,
                            blur_uv.as_ref(),
                            sharp_uv.as_ref(),
                            input_uv,
                            uv_w,
                            uv_h,
                            stage.amount,
                            &self.unsharp_uv_rg8,
                        );
                        stage_output_y = Some(sharp_y);
                        stage_output_uv = Some(sharp_uv);
                    }
                }
            } else if has_color {
                let tmp_y = self.make_texture_with_format(y_w, y_h, metal::MTLPixelFormat::R8Unorm);
                let tmp_uv =
                    self.make_texture_with_format(uv_w, uv_h, metal::MTLPixelFormat::RG8Unorm);
                let blur_y =
                    self.make_texture_with_format(y_w, y_h, metal::MTLPixelFormat::R8Unorm);
                let blur_uv =
                    self.make_texture_with_format(uv_w, uv_h, metal::MTLPixelFormat::RG8Unorm);
                self.encode_blur_two_pass(
                    &command_buffer,
                    src_y,
                    tmp_y.as_ref(),
                    blur_y.as_ref(),
                    y_w,
                    y_h,
                    sigma_abs,
                    &self.blur_h_r8,
                    &self.blur_v_r8,
                );
                self.encode_blur_two_pass(
                    &command_buffer,
                    src_uv,
                    tmp_uv.as_ref(),
                    blur_uv.as_ref(),
                    uv_w,
                    uv_h,
                    sigma_abs,
                    &self.blur_h_rg8,
                    &self.blur_v_rg8,
                );
                stage_output_y = Some(blur_y);
                stage_output_uv = Some(blur_uv);
            } else {
                // blur only
                let tmp_y = self.make_texture_with_format(y_w, y_h, metal::MTLPixelFormat::R8Unorm);
                let tmp_uv =
                    self.make_texture_with_format(uv_w, uv_h, metal::MTLPixelFormat::RG8Unorm);
                self.encode_blur_two_pass(
                    &command_buffer,
                    src_y,
                    tmp_y.as_ref(),
                    dst_y,
                    y_w,
                    y_h,
                    sigma_abs,
                    &self.blur_h_r8,
                    &self.blur_v_r8,
                );
                self.encode_blur_two_pass(
                    &command_buffer,
                    src_uv,
                    tmp_uv.as_ref(),
                    dst_uv,
                    uv_w,
                    uv_h,
                    sigma_abs,
                    &self.blur_h_rg8,
                    &self.blur_v_rg8,
                );
            }

            if let Some(tex) = stage_output_y.as_ref() {
                effect_y = tex.as_ref();
            }
            if let Some(tex) = stage_output_uv.as_ref() {
                effect_uv = tex.as_ref();
            }

            if has_color {
                self.encode_color_dispatch(
                    &command_buffer,
                    effect_y,
                    dst_y,
                    y_w,
                    y_h,
                    brightness,
                    contrast,
                    saturation,
                    lut_mix,
                    rotation_deg,
                    transform_scale,
                    transform_pos_x,
                    transform_pos_y,
                    transform_ref_width,
                    transform_ref_height,
                    tint_y,
                    tint_u,
                    tint_v,
                    tint_alpha,
                    &self.color_y_r8,
                );
                self.encode_color_dispatch(
                    &command_buffer,
                    effect_uv,
                    dst_uv,
                    uv_w,
                    uv_h,
                    brightness,
                    contrast,
                    saturation,
                    lut_mix,
                    rotation_deg,
                    transform_scale,
                    transform_pos_x,
                    transform_pos_y,
                    uv_ref_w,
                    uv_ref_h,
                    tint_y,
                    tint_u,
                    tint_v,
                    tint_alpha,
                    &self.color_uv_rg8,
                );
            }
        } else {
            let uv_ref_w = transform_ref_width * ((uv_w as f32) / (y_w.max(1) as f32));
            let uv_ref_h = transform_ref_height * ((uv_h as f32) / (y_h.max(1) as f32));
            self.encode_color_dispatch(
                &command_buffer,
                src_y,
                dst_y,
                y_w,
                y_h,
                brightness,
                contrast,
                saturation,
                lut_mix,
                rotation_deg,
                transform_scale,
                transform_pos_x,
                transform_pos_y,
                transform_ref_width,
                transform_ref_height,
                tint_y,
                tint_u,
                tint_v,
                tint_alpha,
                &self.color_y_r8,
            );
            self.encode_color_dispatch(
                &command_buffer,
                src_uv,
                dst_uv,
                uv_w,
                uv_h,
                brightness,
                contrast,
                saturation,
                lut_mix,
                rotation_deg,
                transform_scale,
                transform_pos_x,
                transform_pos_y,
                uv_ref_w,
                uv_ref_h,
                tint_y,
                tint_u,
                tint_v,
                tint_alpha,
                &self.color_uv_rg8,
            );
        }
        command_buffer.commit();
        command_buffer.wait_until_completed();

        Ok(dest_surface)
    }

    /// Non-blocking variant: dispatches Metal compute but does NOT wait for
    /// GPU completion. Returns the destination CVPixelBuffer together with
    /// a retained CommandBuffer whose `status()` the caller can poll.
    /// The dest surface contains valid data only once
    /// `cmd_buf.status() == MTLCommandBufferStatus::Completed`.
    fn process_nv12_surface_no_wait(
        &mut self,
        source_surface: &CVPixelBuffer,
        sigma: f32,
        brightness: f32,
        contrast: f32,
        saturation: f32,
        lut_mix: f32,
        rotation_deg: f32,
        transform_scale: f32,
        transform_pos_x: f32,
        transform_pos_y: f32,
        transform_ref_width: f32,
        transform_ref_height: f32,
        tint_hue: f32,
        tint_saturation: f32,
        tint_lightness: f32,
        tint_alpha: f32,
    ) -> Result<(CVPixelBuffer, metal::CommandBuffer), String> {
        let sigma_abs = sigma.abs();
        let has_blur = sigma_abs > 0.001;
        let has_sharpen = sigma < -0.001;
        let has_color = brightness.abs() >= 0.001
            || (contrast - 1.0).abs() >= 0.001
            || (saturation - 1.0).abs() >= 0.001
            || lut_mix.abs() >= 0.001
            || rotation_deg.abs() >= 0.001
            || (transform_scale - 1.0).abs() >= 0.001
            || transform_pos_x.abs() >= 0.001
            || transform_pos_y.abs() >= 0.001
            || tint_alpha.abs() >= 0.001;
        if !has_blur && !has_color {
            // No GPU work needed — commit an empty buffer so status is Completed immediately.
            let cmd = self.queue.new_command_buffer().to_owned();
            cmd.commit();
            return Ok((source_surface.clone(), cmd));
        }
        let (tr, tg, tb) = hsla_to_rgb(
            tint_hue,
            tint_saturation.clamp(0.0, 1.0),
            tint_lightness.clamp(0.0, 1.0),
        );
        let tint_y = (0.299 * tr + 0.587 * tg + 0.114 * tb).clamp(0.0, 1.0);
        let tint_u = (-0.168_736 * tr - 0.331_264 * tg + 0.5 * tb + 0.5).clamp(0.0, 1.0);
        let tint_v = (0.5 * tr - 0.418_688 * tg - 0.081_312 * tb + 0.5).clamp(0.0, 1.0);
        let tint_alpha = tint_alpha.clamp(0.0, 1.0);

        let pixel_format = source_surface.get_pixel_format();
        if pixel_format != kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
            && pixel_format != kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        {
            return Err(format!("unsupported source NV12 format: {pixel_format:#x}"));
        }
        if source_surface.get_plane_count() < 2 {
            return Err("source surface has no NV12 planes".to_string());
        }

        let width = source_surface.get_width() as u32;
        let height = source_surface.get_height() as u32;
        let y_w = source_surface.get_width_of_plane(0) as u32;
        let y_h = source_surface.get_height_of_plane(0) as u32;
        let uv_w = source_surface.get_width_of_plane(1) as u32;
        let uv_h = source_surface.get_height_of_plane(1) as u32;
        if width == 0 || height == 0 || y_w == 0 || y_h == 0 || uv_w == 0 || uv_h == 0 {
            return Err("source surface has invalid dimensions".to_string());
        }

        let iosurface_props: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[]);
        let metal_surface_options: CFDictionary<CFString, CFType> =
            CFDictionary::from_CFType_pairs(&[
                (
                    CFString::from(CVPixelBufferKeys::MetalCompatibility),
                    CFBoolean::true_value().as_CFType(),
                ),
                (
                    CFString::from(CVPixelBufferKeys::IOSurfaceProperties),
                    iosurface_props.as_CFType(),
                ),
            ]);
        let attrs = source_surface.copy_creation_attributes();
        let mut dest_surface = CVPixelBuffer::new(
            pixel_format,
            width as usize,
            height as usize,
            attrs.as_ref(),
        )
        .or_else(|_| {
            CVPixelBuffer::new(
                pixel_format,
                width as usize,
                height as usize,
                Some(&metal_surface_options),
            )
        })
        .or_else(|_| CVPixelBuffer::new(pixel_format, width as usize, height as usize, None))
        .map_err(|status| format!("CVPixelBuffer::new failed: status={status}"))?;

        let src_y_cv = self
            .core_video_texture_cache
            .create_texture_from_image(
                source_surface.as_concrete_TypeRef(),
                None,
                metal::MTLPixelFormat::R8Unorm,
                y_w as usize,
                y_h as usize,
                0,
            )
            .map_err(|status| format!("create src Y texture failed: status={status}"))?;
        let src_uv_cv = self
            .core_video_texture_cache
            .create_texture_from_image(
                source_surface.as_concrete_TypeRef(),
                None,
                metal::MTLPixelFormat::RG8Unorm,
                uv_w as usize,
                uv_h as usize,
                1,
            )
            .map_err(|status| format!("create src UV texture failed: status={status}"))?;
        let mut dst_y_cv_result = self.core_video_texture_cache.create_texture_from_image(
            dest_surface.as_concrete_TypeRef(),
            None,
            metal::MTLPixelFormat::R8Unorm,
            y_w as usize,
            y_h as usize,
            0,
        );
        let mut dst_uv_cv_result = self.core_video_texture_cache.create_texture_from_image(
            dest_surface.as_concrete_TypeRef(),
            None,
            metal::MTLPixelFormat::RG8Unorm,
            uv_w as usize,
            uv_h as usize,
            1,
        );
        if dst_y_cv_result.is_err() || dst_uv_cv_result.is_err() {
            let dst_y_status = dst_y_cv_result.as_ref().err().copied();
            let dst_uv_status = dst_uv_cv_result.as_ref().err().copied();
            if nv12_debug_enabled() {
                log::warn!(
                    "[VideoElement][NV12FX] dst texture create failed -> retry force-metal fmt={}({:#x}) y_status={:?} uv_status={:?}",
                    nv12_pixel_format_tag(pixel_format),
                    pixel_format,
                    dst_y_status,
                    dst_uv_status
                );
            }
            dest_surface = CVPixelBuffer::new(
                pixel_format,
                width as usize,
                height as usize,
                Some(&metal_surface_options),
            )
            .or_else(|_| CVPixelBuffer::new(pixel_format, width as usize, height as usize, None))
            .map_err(|status| {
                format!("CVPixelBuffer::new (force metal attrs) failed: status={status}")
            })?;
            dst_y_cv_result = self.core_video_texture_cache.create_texture_from_image(
                dest_surface.as_concrete_TypeRef(),
                None,
                metal::MTLPixelFormat::R8Unorm,
                y_w as usize,
                y_h as usize,
                0,
            );
            dst_uv_cv_result = self.core_video_texture_cache.create_texture_from_image(
                dest_surface.as_concrete_TypeRef(),
                None,
                metal::MTLPixelFormat::RG8Unorm,
                uv_w as usize,
                uv_h as usize,
                1,
            );
        }
        let dst_y_cv = dst_y_cv_result
            .map_err(|status| format!("create dst Y texture failed: status={status}"))?;
        let dst_uv_cv = dst_uv_cv_result
            .map_err(|status| format!("create dst UV texture failed: status={status}"))?;

        let src_y = Self::as_texture_ref_from_cv_metal_texture(&src_y_cv)?;
        let src_uv = Self::as_texture_ref_from_cv_metal_texture(&src_uv_cv)?;
        let dst_y = Self::as_texture_ref_from_cv_metal_texture(&dst_y_cv)?;
        let dst_uv = Self::as_texture_ref_from_cv_metal_texture(&dst_uv_cv)?;
        let command_buffer = self.queue.new_command_buffer();

        if has_blur {
            let uv_ref_w = transform_ref_width * ((uv_w as f32) / (y_w.max(1) as f32));
            let uv_ref_h = transform_ref_height * ((uv_h as f32) / (y_h.max(1) as f32));
            let mut effect_y: &TextureRef = src_y;
            let mut effect_uv: &TextureRef = src_uv;
            let mut stage_output_y: Option<Texture> = None;
            let mut stage_output_uv: Option<Texture> = None;

            if has_sharpen {
                let sharpen_stages = sharpen_stages_for_sigma(sigma_abs);
                for (idx, stage) in sharpen_stages.iter().enumerate() {
                    let input_y: &TextureRef = if let Some(tex) = stage_output_y.as_ref() {
                        tex.as_ref()
                    } else {
                        effect_y
                    };
                    let input_uv: &TextureRef = if let Some(tex) = stage_output_uv.as_ref() {
                        tex.as_ref()
                    } else {
                        effect_uv
                    };
                    let tmp_y =
                        self.make_texture_with_format(y_w, y_h, metal::MTLPixelFormat::R8Unorm);
                    let tmp_uv =
                        self.make_texture_with_format(uv_w, uv_h, metal::MTLPixelFormat::RG8Unorm);
                    let blur_y =
                        self.make_texture_with_format(y_w, y_h, metal::MTLPixelFormat::R8Unorm);
                    let blur_uv =
                        self.make_texture_with_format(uv_w, uv_h, metal::MTLPixelFormat::RG8Unorm);

                    self.encode_blur_two_pass(
                        &command_buffer,
                        input_y,
                        tmp_y.as_ref(),
                        blur_y.as_ref(),
                        y_w,
                        y_h,
                        stage.sigma,
                        &self.blur_h_r8,
                        &self.blur_v_r8,
                    );
                    self.encode_blur_two_pass(
                        &command_buffer,
                        input_uv,
                        tmp_uv.as_ref(),
                        blur_uv.as_ref(),
                        uv_w,
                        uv_h,
                        stage.sigma,
                        &self.blur_h_rg8,
                        &self.blur_v_rg8,
                    );

                    let is_last = idx + 1 == sharpen_stages.len();
                    if is_last && !has_color {
                        self.encode_unsharp_dispatch(
                            &command_buffer,
                            blur_y.as_ref(),
                            dst_y,
                            input_y,
                            y_w,
                            y_h,
                            stage.amount,
                            &self.unsharp_y_r8,
                        );
                        self.encode_unsharp_dispatch(
                            &command_buffer,
                            blur_uv.as_ref(),
                            dst_uv,
                            input_uv,
                            uv_w,
                            uv_h,
                            stage.amount,
                            &self.unsharp_uv_rg8,
                        );
                    } else {
                        let sharp_y =
                            self.make_texture_with_format(y_w, y_h, metal::MTLPixelFormat::R8Unorm);
                        let sharp_uv = self.make_texture_with_format(
                            uv_w,
                            uv_h,
                            metal::MTLPixelFormat::RG8Unorm,
                        );
                        self.encode_unsharp_dispatch(
                            &command_buffer,
                            blur_y.as_ref(),
                            sharp_y.as_ref(),
                            input_y,
                            y_w,
                            y_h,
                            stage.amount,
                            &self.unsharp_y_r8,
                        );
                        self.encode_unsharp_dispatch(
                            &command_buffer,
                            blur_uv.as_ref(),
                            sharp_uv.as_ref(),
                            input_uv,
                            uv_w,
                            uv_h,
                            stage.amount,
                            &self.unsharp_uv_rg8,
                        );
                        stage_output_y = Some(sharp_y);
                        stage_output_uv = Some(sharp_uv);
                    }
                }
            } else if has_color {
                let tmp_y = self.make_texture_with_format(y_w, y_h, metal::MTLPixelFormat::R8Unorm);
                let tmp_uv =
                    self.make_texture_with_format(uv_w, uv_h, metal::MTLPixelFormat::RG8Unorm);
                let blur_y =
                    self.make_texture_with_format(y_w, y_h, metal::MTLPixelFormat::R8Unorm);
                let blur_uv =
                    self.make_texture_with_format(uv_w, uv_h, metal::MTLPixelFormat::RG8Unorm);
                self.encode_blur_two_pass(
                    &command_buffer,
                    src_y,
                    tmp_y.as_ref(),
                    blur_y.as_ref(),
                    y_w,
                    y_h,
                    sigma_abs,
                    &self.blur_h_r8,
                    &self.blur_v_r8,
                );
                self.encode_blur_two_pass(
                    &command_buffer,
                    src_uv,
                    tmp_uv.as_ref(),
                    blur_uv.as_ref(),
                    uv_w,
                    uv_h,
                    sigma_abs,
                    &self.blur_h_rg8,
                    &self.blur_v_rg8,
                );
                stage_output_y = Some(blur_y);
                stage_output_uv = Some(blur_uv);
            } else {
                // blur only
                let tmp_y = self.make_texture_with_format(y_w, y_h, metal::MTLPixelFormat::R8Unorm);
                let tmp_uv =
                    self.make_texture_with_format(uv_w, uv_h, metal::MTLPixelFormat::RG8Unorm);
                self.encode_blur_two_pass(
                    &command_buffer,
                    src_y,
                    tmp_y.as_ref(),
                    dst_y,
                    y_w,
                    y_h,
                    sigma_abs,
                    &self.blur_h_r8,
                    &self.blur_v_r8,
                );
                self.encode_blur_two_pass(
                    &command_buffer,
                    src_uv,
                    tmp_uv.as_ref(),
                    dst_uv,
                    uv_w,
                    uv_h,
                    sigma_abs,
                    &self.blur_h_rg8,
                    &self.blur_v_rg8,
                );
            }

            if let Some(tex) = stage_output_y.as_ref() {
                effect_y = tex.as_ref();
            }
            if let Some(tex) = stage_output_uv.as_ref() {
                effect_uv = tex.as_ref();
            }

            if has_color {
                self.encode_color_dispatch(
                    &command_buffer,
                    effect_y,
                    dst_y,
                    y_w,
                    y_h,
                    brightness,
                    contrast,
                    saturation,
                    lut_mix,
                    rotation_deg,
                    transform_scale,
                    transform_pos_x,
                    transform_pos_y,
                    transform_ref_width,
                    transform_ref_height,
                    tint_y,
                    tint_u,
                    tint_v,
                    tint_alpha,
                    &self.color_y_r8,
                );
                self.encode_color_dispatch(
                    &command_buffer,
                    effect_uv,
                    dst_uv,
                    uv_w,
                    uv_h,
                    brightness,
                    contrast,
                    saturation,
                    lut_mix,
                    rotation_deg,
                    transform_scale,
                    transform_pos_x,
                    transform_pos_y,
                    uv_ref_w,
                    uv_ref_h,
                    tint_y,
                    tint_u,
                    tint_v,
                    tint_alpha,
                    &self.color_uv_rg8,
                );
            }
        } else {
            let uv_ref_w = transform_ref_width * ((uv_w as f32) / (y_w.max(1) as f32));
            let uv_ref_h = transform_ref_height * ((uv_h as f32) / (y_h.max(1) as f32));
            self.encode_color_dispatch(
                &command_buffer,
                src_y,
                dst_y,
                y_w,
                y_h,
                brightness,
                contrast,
                saturation,
                lut_mix,
                rotation_deg,
                transform_scale,
                transform_pos_x,
                transform_pos_y,
                transform_ref_width,
                transform_ref_height,
                tint_y,
                tint_u,
                tint_v,
                tint_alpha,
                &self.color_y_r8,
            );
            self.encode_color_dispatch(
                &command_buffer,
                src_uv,
                dst_uv,
                uv_w,
                uv_h,
                brightness,
                contrast,
                saturation,
                lut_mix,
                rotation_deg,
                transform_scale,
                transform_pos_x,
                transform_pos_y,
                uv_ref_w,
                uv_ref_h,
                tint_y,
                tint_u,
                tint_v,
                tint_alpha,
                &self.color_uv_rg8,
            );
        }
        // Commit without waiting — caller polls status() for completion.
        let owned_cmd = command_buffer.to_owned();
        owned_cmd.commit();

        Ok((dest_surface, owned_cmd))
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct WgpuBlurParams {
    sigma: f32,
    radius: u32,
    width: u32,
    height: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct WgpuColorParams {
    brightness: f32,
    contrast: f32,
    saturation: f32,
    lut_mix: f32,
    opacity: f32,
    sharpen_amount: f32,
    width: u32,
    height: u32,
    local_mask_enabled: u32,
    rotation_enabled: u32,
    rotation_cos: f32,
    rotation_sin: f32,
    transform_scale: f32,
    transform_pos_x: f32,
    transform_pos_y: f32,
    local_mask_center_x: f32,
    local_mask_center_y: f32,
    local_mask_radius: f32,
    local_mask_feather: f32,
    local_mask_strength: f32,
    tint_r: f32,
    tint_g: f32,
    tint_b: f32,
    tint_alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WgpuTextureSlot {
    Src,
    Tmp,
    Dst,
}

struct WgpuBgraEffectContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    bind_group_layout: wgpu::BindGroupLayout,
    blur_h_pipeline: wgpu::ComputePipeline,
    blur_v_pipeline: wgpu::ComputePipeline,
    color_pipeline: wgpu::ComputePipeline,
    blur_params_global_buffer: wgpu::Buffer,
    color_params_global_buffer: wgpu::Buffer,
    src_texture: Option<wgpu::Texture>,
    tmp_texture: Option<wgpu::Texture>,
    dst_texture: Option<wgpu::Texture>,
    readback_buffer: Option<wgpu::Buffer>,
    dims: Option<(u32, u32)>,
    padded_bytes_per_row: u32,
    device_lost: Arc<AtomicBool>,
}

impl WgpuBgraEffectContext {
    fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .map_err(|_| "No suitable WGPU adapter found".to_string())?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("anica-wgpu-bgra-effects-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|err| format!("request_device failed: {err}"))?;

        let device_lost = Arc::new(AtomicBool::new(false));
        {
            let lost_flag = device_lost.clone();
            device.set_device_lost_callback(move |reason, message| {
                lost_flag.store(true, Ordering::Relaxed);
                set_cpu_safe_mode(format!("WGPU device lost ({reason:?}): {message}"));
            });
        }
        device.on_uncaptured_error(Box::new(|err| {
            log::error!("[VideoElement][WgpuBgra] uncaptured error: {err}");
        }));

        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("anica-wgpu-bgra-effects-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(WGPU_BGRA_EFFECT_SHADER)),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("anica-wgpu-bgra-effects-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("anica-wgpu-bgra-effects-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let blur_h_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("anica-wgpu-bgra-blur-h"),
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: Some("blur_h"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let blur_v_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("anica-wgpu-bgra-blur-v"),
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: Some("blur_v"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let color_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("anica-wgpu-bgra-color"),
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: Some("color_correct"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let blur_params_global_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-wgpu-blur-params-global"),
            size: std::mem::size_of::<WgpuBlurParams>() as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let color_params_global_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-wgpu-color-params-global"),
            size: std::mem::size_of::<WgpuColorParams>() as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        Ok(Self {
            device,
            queue,
            bind_group_layout,
            blur_h_pipeline,
            blur_v_pipeline,
            color_pipeline,
            blur_params_global_buffer,
            color_params_global_buffer,
            src_texture: None,
            tmp_texture: None,
            dst_texture: None,
            readback_buffer: None,
            dims: None,
            padded_bytes_per_row: 0,
            device_lost,
        })
    }

    fn make_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("anica-wgpu-bgra-effects-tex"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn align_to(value: u32, alignment: u32) -> u32 {
        if alignment == 0 {
            return value;
        }
        value.div_ceil(alignment) * alignment
    }

    fn ensure_resources(&mut self, width: u32, height: u32) {
        if self.dims == Some((width, height))
            && self.src_texture.is_some()
            && self.tmp_texture.is_some()
            && self.dst_texture.is_some()
            && self.readback_buffer.is_some()
        {
            return;
        }

        self.src_texture = Some(Self::make_texture(&self.device, width, height));
        self.tmp_texture = Some(Self::make_texture(&self.device, width, height));
        self.dst_texture = Some(Self::make_texture(&self.device, width, height));

        let unpadded_bpr = width.saturating_mul(4);
        self.padded_bytes_per_row =
            Self::align_to(unpadded_bpr, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback_size = self.padded_bytes_per_row as u64 * height as u64;
        self.readback_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-wgpu-bgra-effects-readback"),
            size: readback_size.max(4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        }));
        self.dims = Some((width, height));
    }

    fn texture(&self, slot: WgpuTextureSlot) -> Result<&wgpu::Texture, String> {
        match slot {
            WgpuTextureSlot::Src => self
                .src_texture
                .as_ref()
                .ok_or("missing src texture".to_string()),
            WgpuTextureSlot::Tmp => self
                .tmp_texture
                .as_ref()
                .ok_or("missing tmp texture".to_string()),
            WgpuTextureSlot::Dst => self
                .dst_texture
                .as_ref()
                .ok_or("missing dst texture".to_string()),
        }
    }

    fn as_bytes<T>(value: &T) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(value as *const T as *const u8, std::mem::size_of::<T>())
        }
    }

    fn dispatch_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        src_slot: WgpuTextureSlot,
        dst_slot: WgpuTextureSlot,
        orig_slot: WgpuTextureSlot,
        blur_params_buffer: &wgpu::Buffer,
        color_params_buffer: &wgpu::Buffer,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let src_view = self
            .texture(src_slot)?
            .create_view(&wgpu::TextureViewDescriptor::default());
        let dst_view = self
            .texture(dst_slot)?
            .create_view(&wgpu::TextureViewDescriptor::default());
        let orig_view = self
            .texture(orig_slot)?
            .create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anica-wgpu-bgra-effects-bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&src_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&dst_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: blur_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: color_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&orig_view),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("anica-wgpu-bgra-effects-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        let gx = width.div_ceil(WGPU_BGRA_WORKGROUP_SIZE).max(1);
        let gy = height.div_ceil(WGPU_BGRA_WORKGROUP_SIZE).max(1);
        pass.dispatch_workgroups(gx, gy, 1);
        Ok(())
    }

    fn process_frame(
        &mut self,
        data: &mut Vec<u8>,
        width: u32,
        height: u32,
        brightness: f32,
        contrast: f32,
        saturation: f32,
        lut_mix: f32,
        opacity: f32,
        rotation_deg: f32,
        transform_scale: f32,
        transform_pos_x: f32,
        transform_pos_y: f32,
        transform_ref_width: f32,
        transform_ref_height: f32,
        tint_hue: f32,
        tint_saturation: f32,
        tint_lightness: f32,
        tint_alpha: f32,
        blur_sigma: f32,
        local_layers: &[VideoLocalMaskLayer],
    ) -> Result<(), String> {
        if self.device_lost.load(Ordering::Relaxed) {
            return Err("WGPU device already lost".to_string());
        }
        if width == 0 || height == 0 {
            return Ok(());
        }
        let expected = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        if data.len() != expected {
            return Err(format!(
                "invalid BGRA buffer size: got={}, expected={expected}",
                data.len()
            ));
        }

        self.ensure_resources(width, height);
        let src = self.texture(WgpuTextureSlot::Src)?;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: src,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width.saturating_mul(4)),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let has_global_blur = blur_sigma >= 0.001;
        let has_global_sharpen = blur_sigma <= -0.001;
        let sharpen_stages = if has_global_sharpen {
            sharpen_stages_for_sigma(blur_sigma.abs())
        } else {
            SmallVec::<[SharpenStage; 2]>::new()
        };
        let has_global_blur_or_sharpen = has_global_blur || !sharpen_stages.is_empty();
        let has_global_rotation = rotation_deg.abs() >= 0.001;
        let has_global_transform = (transform_scale - 1.0).abs() >= 0.001
            || transform_pos_x.abs() >= 0.001
            || transform_pos_y.abs() >= 0.001;
        let has_global_color = brightness.abs() >= 0.001
            || (contrast - 1.0).abs() >= 0.001
            || (saturation - 1.0).abs() >= 0.001
            || lut_mix.abs() >= 0.001
            || (opacity - 1.0).abs() >= 0.001
            || has_global_rotation
            || has_global_transform
            || tint_alpha.abs() >= 0.001;
        let (tint_r, tint_g, tint_b) = hsla_to_rgb(
            tint_hue,
            tint_saturation.clamp(0.0, 1.0),
            tint_lightness.clamp(0.0, 1.0),
        );
        let tint_alpha = tint_alpha.clamp(0.0, 1.0);
        let has_any_local_effect =
            local_layers
                .iter()
                .take(VIDEO_MAX_LOCAL_MASK_LAYERS)
                .any(|layer| {
                    let has_shape = layer.enabled
                        && layer.strength >= 0.001
                        && layer.radius >= 0.0001
                        && (layer.feather >= 0.0001 || layer.radius > 0.0001);
                    let has_color = layer.brightness.abs() >= 0.001
                        || (layer.contrast - 1.0).abs() >= 0.001
                        || (layer.saturation - 1.0).abs() >= 0.001
                        || (layer.opacity - 1.0).abs() >= 0.001;
                    let has_blur = layer.blur_sigma.abs() >= 0.001;
                    has_shape && (has_color || has_blur)
                });

        if !has_global_blur_or_sharpen && !has_global_color && !has_any_local_effect {
            return Ok(());
        }

        let make_blur_params = |sigma: f32| WgpuBlurParams {
            sigma: sigma.abs().max(0.001).clamp(0.0, 64.0),
            radius: ((sigma.abs().clamp(0.0, 64.0) * 3.0).ceil() as u32).clamp(0, 64),
            width,
            height,
        };
        let make_color_params = |b: f32,
                                 c: f32,
                                 s: f32,
                                 lut_m: f32,
                                 o: f32,
                                 sharpen_amount: f32,
                                 rotation: f32,
                                 transform_enabled: bool,
                                 tint_rgba: (f32, f32, f32, f32),
                                 mask: Option<&VideoLocalMaskLayer>|
         -> WgpuColorParams {
            let ref_w = transform_ref_width.max(1.0);
            let ref_h = transform_ref_height.max(1.0);
            let width_f = width.max(1) as f32;
            let height_f = height.max(1) as f32;
            let pos_x_norm = if transform_enabled {
                transform_pos_x * (ref_w / width_f)
            } else {
                0.0
            };
            let pos_y_norm = if transform_enabled {
                transform_pos_y * (ref_h / height_f)
            } else {
                0.0
            };
            let transform_scale_val = if transform_enabled {
                transform_scale.clamp(0.01, 5.0)
            } else {
                1.0
            };
            let (mask_enabled, center_x, center_y, radius, feather, strength) =
                if let Some(layer) = mask {
                    (
                        layer.enabled,
                        layer.center_x,
                        layer.center_y,
                        layer.radius,
                        layer.feather,
                        layer.strength,
                    )
                } else {
                    (false, 0.5, 0.5, 0.25, 0.15, 1.0)
                };
            WgpuColorParams {
                brightness: b.clamp(-1.0, 1.0),
                contrast: c.clamp(0.0, 2.0),
                saturation: s.clamp(0.0, 2.0),
                lut_mix: lut_m.clamp(0.0, 1.0),
                opacity: o.clamp(0.0, 1.0),
                sharpen_amount: sharpen_amount.clamp(0.0, 4.0),
                width,
                height,
                local_mask_enabled: if mask_enabled { 1 } else { 0 },
                rotation_enabled: if transform_enabled && rotation.abs() >= 0.001 {
                    1
                } else {
                    0
                },
                rotation_cos: rotation.to_radians().cos(),
                rotation_sin: rotation.to_radians().sin(),
                transform_scale: transform_scale_val,
                transform_pos_x: pos_x_norm,
                transform_pos_y: pos_y_norm,
                local_mask_center_x: center_x.clamp(0.0, 1.0),
                local_mask_center_y: center_y.clamp(0.0, 1.0),
                local_mask_radius: radius.clamp(0.0, 1.0),
                local_mask_feather: feather.clamp(0.0, 1.0),
                local_mask_strength: strength.clamp(0.0, 1.0),
                tint_r: tint_rgba.0.clamp(0.0, 1.0),
                tint_g: tint_rgba.1.clamp(0.0, 1.0),
                tint_b: tint_rgba.2.clamp(0.0, 1.0),
                tint_alpha: tint_rgba.3.clamp(0.0, 1.0),
            }
        };
        let global_blur_params = make_blur_params(blur_sigma);
        let global_color_params = make_color_params(
            brightness,
            contrast,
            saturation,
            lut_mix,
            opacity,
            0.0,
            rotation_deg,
            true,
            (tint_r, tint_g, tint_b, tint_alpha),
            None,
        );
        let color_params_size = std::mem::size_of::<WgpuColorParams>() as u64;
        let blur_params_size = std::mem::size_of::<WgpuBlurParams>() as u64;
        let make_params_buffer = |label: &'static str, size: u64| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };

        // Use dedicated buffers per pass type. Reusing one buffer with multiple queue.write_buffer
        // calls before a single submit can make all passes observe only the last write.
        self.queue.write_buffer(
            &self.blur_params_global_buffer,
            0,
            Self::as_bytes(&global_blur_params),
        );
        self.queue.write_buffer(
            &self.color_params_global_buffer,
            0,
            Self::as_bytes(&global_color_params),
        );

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("anica-wgpu-bgra-effects-encoder"),
            });

        let mut current = WgpuTextureSlot::Src;
        if has_global_blur {
            self.dispatch_pass(
                &mut encoder,
                &self.blur_h_pipeline,
                WgpuTextureSlot::Src,
                WgpuTextureSlot::Tmp,
                WgpuTextureSlot::Src,
                &self.blur_params_global_buffer,
                &self.color_params_global_buffer,
                width,
                height,
            )?;
            self.dispatch_pass(
                &mut encoder,
                &self.blur_v_pipeline,
                WgpuTextureSlot::Tmp,
                WgpuTextureSlot::Dst,
                WgpuTextureSlot::Tmp,
                &self.blur_params_global_buffer,
                &self.color_params_global_buffer,
                width,
                height,
            )?;
            current = WgpuTextureSlot::Dst;
        }

        if has_global_sharpen {
            let mut global_sharpen_stage_buffers: Vec<(wgpu::Buffer, wgpu::Buffer)> = Vec::new();
            for stage in &sharpen_stages {
                let stage_blur_params = make_blur_params(stage.sigma);
                let stage_sharpen_params = make_color_params(
                    0.0,
                    1.0,
                    1.0,
                    0.0,
                    1.0,
                    stage.amount,
                    0.0,
                    false,
                    (0.0, 0.0, 0.0, 0.0),
                    None,
                );
                let stage_blur_buf =
                    make_params_buffer("anica-wgpu-global-sharpen-stage-blur", blur_params_size);
                let stage_sharpen_buf =
                    make_params_buffer("anica-wgpu-global-sharpen-stage-color", color_params_size);
                self.queue
                    .write_buffer(&stage_blur_buf, 0, Self::as_bytes(&stage_blur_params));
                self.queue.write_buffer(
                    &stage_sharpen_buf,
                    0,
                    Self::as_bytes(&stage_sharpen_params),
                );
                global_sharpen_stage_buffers.push((stage_blur_buf, stage_sharpen_buf));
                let idx = global_sharpen_stage_buffers.len() - 1;
                let stage_blur_params_buf = &global_sharpen_stage_buffers[idx].0;
                let stage_sharpen_params_buf = &global_sharpen_stage_buffers[idx].1;

                let (tmp, blurred, out) = match current {
                    WgpuTextureSlot::Src => (
                        WgpuTextureSlot::Tmp,
                        WgpuTextureSlot::Dst,
                        WgpuTextureSlot::Tmp,
                    ),
                    WgpuTextureSlot::Tmp => (
                        WgpuTextureSlot::Src,
                        WgpuTextureSlot::Dst,
                        WgpuTextureSlot::Src,
                    ),
                    WgpuTextureSlot::Dst => (
                        WgpuTextureSlot::Src,
                        WgpuTextureSlot::Tmp,
                        WgpuTextureSlot::Src,
                    ),
                };

                self.dispatch_pass(
                    &mut encoder,
                    &self.blur_h_pipeline,
                    current,
                    tmp,
                    current,
                    stage_blur_params_buf,
                    stage_sharpen_params_buf,
                    width,
                    height,
                )?;
                self.dispatch_pass(
                    &mut encoder,
                    &self.blur_v_pipeline,
                    tmp,
                    blurred,
                    tmp,
                    stage_blur_params_buf,
                    stage_sharpen_params_buf,
                    width,
                    height,
                )?;
                self.dispatch_pass(
                    &mut encoder,
                    &self.color_pipeline,
                    blurred,
                    out,
                    current,
                    stage_blur_params_buf,
                    stage_sharpen_params_buf,
                    width,
                    height,
                )?;
                current = out;
            }
        }

        if has_global_color {
            let out = match current {
                WgpuTextureSlot::Src => WgpuTextureSlot::Dst,
                WgpuTextureSlot::Tmp => WgpuTextureSlot::Dst,
                // Keep output away from input in this pass.
                WgpuTextureSlot::Dst => WgpuTextureSlot::Tmp,
            };
            self.dispatch_pass(
                &mut encoder,
                &self.color_pipeline,
                current,
                out,
                current,
                &self.blur_params_global_buffer,
                &self.color_params_global_buffer,
                width,
                height,
            )?;
            current = out;
        }

        let mut per_layer_param_buffers: Vec<wgpu::Buffer> = Vec::new();

        for layer in local_layers.iter().take(VIDEO_MAX_LOCAL_MASK_LAYERS) {
            let has_local_mask_shape = layer.enabled
                && layer.strength >= 0.001
                && layer.radius >= 0.0001
                && (layer.feather >= 0.0001 || layer.radius > 0.0001);
            let has_local_color = layer.brightness.abs() >= 0.001
                || (layer.contrast - 1.0).abs() >= 0.001
                || (layer.saturation - 1.0).abs() >= 0.001
                || (layer.opacity - 1.0).abs() >= 0.001;
            let has_local_blur = layer.blur_sigma >= 0.001;
            let has_local_sharpen = layer.blur_sigma <= -0.001;
            let has_local_blur_or_sharpen = has_local_blur || has_local_sharpen;
            let has_local_effect =
                has_local_mask_shape && (has_local_color || has_local_blur_or_sharpen);
            if !has_local_effect {
                continue;
            }

            let local_blur_params = make_blur_params(layer.blur_sigma);
            let local_color_params = make_color_params(
                layer.brightness,
                layer.contrast,
                layer.saturation,
                0.0,
                layer.opacity,
                0.0,
                0.0,
                false,
                (0.0, 0.0, 0.0, 0.0),
                None,
            );
            let local_sharpen_params = make_color_params(
                0.0,
                1.0,
                1.0,
                0.0,
                1.0,
                if has_local_sharpen { 1.0 } else { 0.0 },
                0.0,
                false,
                (0.0, 0.0, 0.0, 0.0),
                None,
            );
            let blend_color_params = make_color_params(
                0.0,
                1.0,
                1.0,
                0.0,
                1.0,
                0.0,
                0.0,
                false,
                (0.0, 0.0, 0.0, 0.0),
                Some(layer),
            );
            let blur_params_buffer =
                make_params_buffer("anica-wgpu-layer-blur-params", blur_params_size);
            let color_params_buffer =
                make_params_buffer("anica-wgpu-layer-color-params", color_params_size);
            let sharpen_params_buffer =
                make_params_buffer("anica-wgpu-layer-sharpen-params", color_params_size);
            let blend_params_buffer =
                make_params_buffer("anica-wgpu-layer-blend-params", color_params_size);
            self.queue
                .write_buffer(&blur_params_buffer, 0, Self::as_bytes(&local_blur_params));
            self.queue
                .write_buffer(&color_params_buffer, 0, Self::as_bytes(&local_color_params));
            self.queue.write_buffer(
                &sharpen_params_buffer,
                0,
                Self::as_bytes(&local_sharpen_params),
            );
            self.queue
                .write_buffer(&blend_params_buffer, 0, Self::as_bytes(&blend_color_params));

            per_layer_param_buffers.push(blur_params_buffer);
            per_layer_param_buffers.push(color_params_buffer);
            per_layer_param_buffers.push(sharpen_params_buffer);
            per_layer_param_buffers.push(blend_params_buffer);
            let base = per_layer_param_buffers.len() - 4;
            let layer_blur_params = &per_layer_param_buffers[base];
            let layer_color_params = &per_layer_param_buffers[base + 1];
            let layer_sharpen_params = &per_layer_param_buffers[base + 2];
            let layer_blend_params = &per_layer_param_buffers[base + 3];

            let global_slot = current;
            let (slot_a, slot_b) = match global_slot {
                WgpuTextureSlot::Src => (WgpuTextureSlot::Tmp, WgpuTextureSlot::Dst),
                WgpuTextureSlot::Tmp => (WgpuTextureSlot::Src, WgpuTextureSlot::Dst),
                WgpuTextureSlot::Dst => (WgpuTextureSlot::Src, WgpuTextureSlot::Tmp),
            };
            let mut local_slot = global_slot;

            if has_local_blur_or_sharpen {
                self.dispatch_pass(
                    &mut encoder,
                    &self.blur_h_pipeline,
                    local_slot,
                    slot_a,
                    local_slot,
                    layer_blur_params,
                    layer_color_params,
                    width,
                    height,
                )?;
                self.dispatch_pass(
                    &mut encoder,
                    &self.blur_v_pipeline,
                    slot_a,
                    slot_b,
                    slot_a,
                    layer_blur_params,
                    layer_color_params,
                    width,
                    height,
                )?;
                local_slot = slot_b;
            }

            if has_local_sharpen {
                let local_sharpen_out = if local_slot == slot_a { slot_b } else { slot_a };
                self.dispatch_pass(
                    &mut encoder,
                    &self.color_pipeline,
                    local_slot,
                    local_sharpen_out,
                    global_slot,
                    layer_blur_params,
                    layer_sharpen_params,
                    width,
                    height,
                )?;
                local_slot = local_sharpen_out;
            }

            if has_local_color {
                let local_color_out = if local_slot == slot_a { slot_b } else { slot_a };
                self.dispatch_pass(
                    &mut encoder,
                    &self.color_pipeline,
                    local_slot,
                    local_color_out,
                    local_slot,
                    layer_blur_params,
                    layer_color_params,
                    width,
                    height,
                )?;
                local_slot = local_color_out;
            }

            let blend_out = match (global_slot, local_slot) {
                (WgpuTextureSlot::Src, WgpuTextureSlot::Tmp)
                | (WgpuTextureSlot::Tmp, WgpuTextureSlot::Src) => WgpuTextureSlot::Dst,
                (WgpuTextureSlot::Src, WgpuTextureSlot::Dst)
                | (WgpuTextureSlot::Dst, WgpuTextureSlot::Src) => WgpuTextureSlot::Tmp,
                (WgpuTextureSlot::Tmp, WgpuTextureSlot::Dst)
                | (WgpuTextureSlot::Dst, WgpuTextureSlot::Tmp) => WgpuTextureSlot::Src,
                _ => WgpuTextureSlot::Dst,
            };
            self.dispatch_pass(
                &mut encoder,
                &self.color_pipeline,
                local_slot,
                blend_out,
                global_slot,
                &self.blur_params_global_buffer,
                layer_blend_params,
                width,
                height,
            )?;
            current = blend_out;
        }

        let readback = self
            .readback_buffer
            .as_ref()
            .ok_or("missing readback buffer".to_string())?;
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: self.texture(current)?,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let _submission = self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait())
            .map_err(|err| format!("device.poll failed: {err}"))?;
        rx.recv()
            .map_err(|err| format!("map callback failed: {err}"))?
            .map_err(|err| format!("buffer map failed: {err}"))?;

        let mapped = slice.get_mapped_range();
        let row_bytes = width as usize * 4;
        let padded_row_bytes = self.padded_bytes_per_row as usize;
        for row in 0..height as usize {
            let src_off = row * padded_row_bytes;
            let dst_off = row * row_bytes;
            data[dst_off..(dst_off + row_bytes)]
                .copy_from_slice(&mapped[src_off..(src_off + row_bytes)]);
        }
        drop(mapped);
        readback.unmap();
        Ok(())
    }
}

fn safe_mode_reason_store() -> &'static Mutex<Option<String>> {
    static STORE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(None))
}

fn set_cpu_safe_mode(reason: String) {
    let was_enabled = WGPU_BGRA_CPU_SAFE_MODE.swap(true, Ordering::Relaxed);
    if let Ok(mut slot) = safe_mode_reason_store().lock() {
        *slot = Some(reason.clone());
    }
    if !was_enabled {
        log::error!("[VideoElement][WgpuBgra] CPU SAFE MODE ON: {}", reason);
    }
}

fn cpu_safe_mode_message() -> Option<String> {
    if !WGPU_BGRA_CPU_SAFE_MODE.load(Ordering::Relaxed) {
        return None;
    }
    if let Ok(slot) = safe_mode_reason_store().lock() {
        if let Some(reason) = slot.as_ref() {
            return Some(format!("CPU SAFE MODE ON ({reason})"));
        }
    }
    Some("CPU SAFE MODE ON".to_string())
}

pub fn bgra_cpu_safe_mode_notice() -> Option<String> {
    cpu_safe_mode_message()
}

/// Global WGPU context for the public `process_bgra_effects` function.
/// Uses a static Mutex instead of thread-local so the wgpu device outlives
/// background thread pool threads.  (Thread-local wgpu contexts panic on
/// thread shutdown because wgpu's own internal TLS is destroyed first.)
static IMAGE_WGPU_BGRA_CONTEXT: OnceLock<Mutex<Option<WgpuBgraEffectContext>>> = OnceLock::new();

/// Public entry point for GPU-accelerated BGRA effects processing.
/// Used by image clips (and any non-video BGRA source) to run blur, color
/// correction, rotation, tint overlay, etc. through the same WGPU compute
/// pipeline that video clips use.  Returns `true` when GPU processing
/// succeeded; the caller should fall back to CPU only when this returns `false`.
pub fn process_bgra_effects(
    data: &mut Vec<u8>,
    width: u32,
    height: u32,
    brightness: f32,
    contrast: f32,
    saturation: f32,
    lut_mix: f32,
    rotation_deg: f32,
    blur_sigma: f32,
    tint_hue: f32,
    tint_saturation: f32,
    tint_lightness: f32,
    tint_alpha: f32,
) -> bool {
    if WGPU_BGRA_CPU_SAFE_MODE.load(Ordering::Relaxed) {
        return false;
    }

    let mutex = IMAGE_WGPU_BGRA_CONTEXT.get_or_init(|| Mutex::new(None));
    let mut slot = match mutex.lock() {
        Ok(guard) => guard,
        Err(_) => return false,
    };

    if slot.is_none() {
        match WgpuBgraEffectContext::new() {
            Ok(ctx) => {
                *slot = Some(ctx);
            }
            Err(err) => {
                set_cpu_safe_mode(format!("WGPU init failed: {err}"));
                return false;
            }
        }
    }

    let ctx = match slot.as_mut() {
        Some(ctx) => ctx,
        None => return false,
    };

    // Image clips pass all effect parameters through to the GPU pipeline.
    // Opacity and transform (scale/pos) are handled by the caller in UI layer.
    match ctx.process_frame(
        data,
        width,
        height,
        brightness,
        contrast,
        saturation,
        lut_mix,
        1.0, // opacity (always 1.0 here, applied separately in UI)
        rotation_deg,
        1.0, // transform_scale (always 1.0, position done in UI)
        0.0, // transform_pos_x (always 0.0)
        0.0, // transform_pos_y (always 0.0)
        0.0, // transform_ref_width (always 0.0)
        0.0, // transform_ref_height (always 0.0)
        tint_hue,
        tint_saturation,
        tint_lightness,
        tint_alpha,
        blur_sigma,
        &[], // no local mask layers for now
    ) {
        Ok(()) => true,
        Err(err) => {
            let lost = ctx.device_lost.load(Ordering::Relaxed);
            *slot = None;
            if WGPU_BGRA_CPU_SAFE_MODE.load(Ordering::Relaxed) || lost {
                set_cpu_safe_mode(format!("WGPU device-lost fallback: {err}"));
            } else {
                log::error!(
                    "[process_bgra_effects] runtime failed (keeping GPU path): {}",
                    err
                );
            }
            false
        }
    }
}

#[cfg(target_os = "macos")]
thread_local! {
    static METAL_BLUR_CONTEXT: RefCell<Option<MetalGaussianBlurContext>> = RefCell::new(None);
}

thread_local! {
    static WGPU_BGRA_CONTEXT: RefCell<Option<WgpuBgraEffectContext>> = RefCell::new(None);
}

#[cfg(target_os = "macos")]
static METAL_BLUR_INIT_FAILED: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static METAL_BLUR_RUNTIME_FAILED: AtomicBool = AtomicBool::new(false);
static WGPU_BGRA_CPU_SAFE_MODE: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
fn nv12_debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("ANICA_DEBUG_NV12_PATH")
            .ok()
            .map(|raw| {
                let s = raw.trim();
                s == "1"
                    || s.eq_ignore_ascii_case("true")
                    || s.eq_ignore_ascii_case("yes")
                    || s.eq_ignore_ascii_case("on")
            })
            .unwrap_or(false)
    })
}

#[cfg(target_os = "macos")]
fn nv12_pixel_format_tag(pixel_format: u32) -> &'static str {
    if pixel_format == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange {
        "420f"
    } else if pixel_format == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange {
        "420v"
    } else {
        "other"
    }
}

fn hsla_to_rgb(hue_deg: f32, sat: f32, light: f32) -> (f32, f32, f32) {
    let h = ((hue_deg / 360.0) % 1.0 + 1.0) % 1.0;
    let s = sat.clamp(0.0, 1.0);
    let l = light.clamp(0.0, 1.0);
    if s <= 0.0001 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue_to_rgb = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (
        hue_to_rgb(h + 1.0 / 3.0),
        hue_to_rgb(h),
        hue_to_rgb(h - 1.0 / 3.0),
    )
}

#[derive(Clone, Copy)]
struct SharpenStage {
    sigma: f32,
    amount: f32,
}

fn sharpen_stages_for_sigma(sigma_abs: f32) -> SmallVec<[SharpenStage; 2]> {
    let sigma = sigma_abs.clamp(0.0, 64.0);
    let mut out = SmallVec::<[SharpenStage; 2]>::new();
    if sigma <= 0.001 {
        return out;
    }

    if sigma >= 7.0 {
        let step = ((sigma - 7.0) / (64.0 - 7.0)).clamp(0.0, 1.0);
        let major_step = (step.sqrt() * 5.0).floor() as i32; // 0..5
        let major = (13 + major_step * 2).clamp(13, 23) as f32;
        let minor = (13 - major_step * 2).clamp(3, 13) as f32;
        let amount = (1.0 + step * 0.35).clamp(0.0, 4.0);
        out.push(SharpenStage {
            sigma: major * 0.5,
            amount,
        });
        if major_step > 0 {
            out.push(SharpenStage {
                sigma: minor * 0.5,
                amount,
            });
        }
        return out;
    }

    out.push(SharpenStage {
        sigma,
        amount: 1.05,
    });
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PixelProcessKey {
    brightness: i16,
    contrast: i16,
    saturation: i16,
    lut_mix: i16,
    opacity: i16,
    tint_hue: i16,
    tint_saturation: i16,
    tint_lightness: i16,
    tint_alpha: i16,
    blur_sigma: i16,
    rotation_deg: i16,
    transform_scale: i16,
    transform_pos_x: i16,
    transform_pos_y: i16,
    transform_ref_width: u16,
    transform_ref_height: u16,
    blur_fast_mode: bool,
    local_layer_count: u8,
    local_layers_hash: u64,
}

impl PixelProcessKey {
    fn from_values(
        brightness: f32,
        contrast: f32,
        saturation: f32,
        lut_mix: f32,
        opacity: f32,
        tint_hue: f32,
        tint_saturation: f32,
        tint_lightness: f32,
        tint_alpha: f32,
        blur_sigma: f32,
        rotation_deg: f32,
        transform_scale: f32,
        transform_pos_x: f32,
        transform_pos_y: f32,
        transform_ref_width: f32,
        transform_ref_height: f32,
        blur_fast_mode: bool,
        local_layers: &[VideoLocalMaskLayer],
    ) -> Self {
        const SCALE: f32 = 1000.0;
        // i16 key fields would saturate for large domains if we used SCALE=1000 everywhere:
        // rotation [-180,180] and blur [0,64]. Use dedicated scales to preserve range.
        const ROT_SCALE: f32 = 100.0;
        const BLUR_SCALE: f32 = 100.0;
        let effective_blur_sigma = if blur_fast_mode {
            (blur_sigma * 2.0).round() * 0.5
        } else {
            blur_sigma
        };
        let mut layer_hash = DefaultHasher::new();
        let mut layer_count: u8 = 0;
        for layer in local_layers.iter().take(VIDEO_MAX_LOCAL_MASK_LAYERS) {
            layer_count = layer_count.saturating_add(1);
            layer_hash.write_u8(layer.enabled as u8);
            layer_hash.write_i16((layer.center_x * SCALE).round() as i16);
            layer_hash.write_i16((layer.center_y * SCALE).round() as i16);
            layer_hash.write_i16((layer.radius * SCALE).round() as i16);
            layer_hash.write_i16((layer.feather * SCALE).round() as i16);
            layer_hash.write_i16((layer.strength * SCALE).round() as i16);
            layer_hash.write_i16((layer.brightness * SCALE).round() as i16);
            layer_hash.write_i16((layer.contrast * SCALE).round() as i16);
            layer_hash.write_i16((layer.saturation * SCALE).round() as i16);
            layer_hash.write_i16((layer.opacity * SCALE).round() as i16);
            let effective_local_blur_sigma = if blur_fast_mode {
                (layer.blur_sigma * 2.0).round() * 0.5
            } else {
                layer.blur_sigma
            };
            layer_hash.write_i16((effective_local_blur_sigma * BLUR_SCALE).round() as i16);
        }
        Self {
            brightness: (brightness * SCALE).round() as i16,
            contrast: (contrast * SCALE).round() as i16,
            saturation: (saturation * SCALE).round() as i16,
            lut_mix: (lut_mix * SCALE).round() as i16,
            opacity: (opacity * SCALE).round() as i16,
            tint_hue: (tint_hue * 10.0).round() as i16,
            tint_saturation: (tint_saturation * SCALE).round() as i16,
            tint_lightness: (tint_lightness * SCALE).round() as i16,
            tint_alpha: (tint_alpha * SCALE).round() as i16,
            blur_sigma: (effective_blur_sigma * BLUR_SCALE).round() as i16,
            rotation_deg: (rotation_deg * ROT_SCALE).round() as i16,
            transform_scale: (transform_scale * SCALE).round() as i16,
            transform_pos_x: (transform_pos_x * SCALE).round() as i16,
            transform_pos_y: (transform_pos_y * SCALE).round() as i16,
            transform_ref_width: transform_ref_width.round().clamp(0.0, u16::MAX as f32) as u16,
            transform_ref_height: transform_ref_height.round().clamp(0.0, u16::MAX as f32) as u16,
            blur_fast_mode,
            local_layer_count: layer_count,
            local_layers_hash: layer_hash.finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct FrameRamCacheKey {
    video_id: u64,
    pts_ns: u64,
    pixel_key: PixelProcessKey,
}

#[derive(Clone)]
struct FrameRamCacheEntry {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
    bytes: usize,
}

#[cfg(target_os = "macos")]
#[derive(Clone)]
struct SurfaceBlurCacheEntry {
    frame_pts_ns: u64,
    pixel_key: PixelProcessKey,
    surface: CVPixelBuffer,
}

/// In-flight NV12 blur compute job. The dest_surface is only valid once
/// the GPU finishes — poll `cmd_buf.status()` to check.
#[cfg(target_os = "macos")]
struct PendingNv12Blur {
    cmd_buf: CommandBuffer,
    dest_surface: CVPixelBuffer,
    frame_pts_ns: u64,
    pixel_key: PixelProcessKey,
}

struct FrameRamCache {
    entries: HashMap<FrameRamCacheKey, FrameRamCacheEntry>,
    lru: VecDeque<FrameRamCacheKey>,
    total_bytes: usize,
    budget_bytes: usize,
    hits: u64,
    misses: u64,
}

impl FrameRamCache {
    fn budget_bytes() -> usize {
        static BUDGET: OnceLock<usize> = OnceLock::new();
        *BUDGET.get_or_init(|| {
            let mb = std::env::var("ANICA_FRAME_RAM_CACHE_MB")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .or_else(|| {
                    std::env::var("ANICA_PREVIEW_RAM_BUDGET_MB")
                        .ok()
                        .and_then(|v| v.parse::<usize>().ok())
                        .map(|v| (v / 2).max(64))
                })
                .unwrap_or(DEFAULT_FRAME_RAM_CACHE_MB)
                .clamp(64, 8192);
            let bytes = mb.saturating_mul(1024 * 1024);
            log::info!(
                "[VideoElement][RamCache] budget_mb={} (env: ANICA_FRAME_RAM_CACHE_MB)",
                mb
            );
            bytes
        })
    }

    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            lru: VecDeque::new(),
            total_bytes: 0,
            budget_bytes: Self::budget_bytes(),
            hits: 0,
            misses: 0,
        }
    }

    fn touch(&mut self, key: FrameRamCacheKey) {
        if let Some(pos) = self.lru.iter().position(|k| *k == key) {
            let _ = self.lru.remove(pos);
        }
        self.lru.push_back(key);
    }

    fn get(&mut self, key: FrameRamCacheKey) -> Option<FrameRamCacheEntry> {
        let value = self.entries.get(&key).cloned();
        if value.is_some() {
            self.hits = self.hits.saturating_add(1);
            self.touch(key);
        } else {
            self.misses = self.misses.saturating_add(1);
        }
        if (self.hits + self.misses) % 240 == 0 {
            let total_mb = self.total_bytes as f64 / (1024.0 * 1024.0);
            let budget_mb = self.budget_bytes as f64 / (1024.0 * 1024.0);
            log::info!(
                "[VideoElement][RamCache] entries={} mem_mb={:.1}/{:.1} hits={} misses={}",
                self.entries.len(),
                total_mb,
                budget_mb,
                self.hits,
                self.misses
            );
        }
        value
    }

    fn insert(
        &mut self,
        key: FrameRamCacheKey,
        entry: FrameRamCacheEntry,
    ) -> Vec<Arc<RenderImage>> {
        let mut evicted_images = Vec::new();

        if let Some(old) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(old.bytes);
            self.touch(key);
        } else {
            self.lru.push_back(key);
        }

        self.total_bytes = self.total_bytes.saturating_add(entry.bytes);
        self.entries.insert(key, entry);

        while self.total_bytes > self.budget_bytes {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.bytes);
                evicted_images.push(evicted.image);
            }
        }
        evicted_images
    }

    fn clear_video(&mut self, video_id: u64) -> Vec<Arc<RenderImage>> {
        let mut dropped = Vec::new();
        let keys: Vec<FrameRamCacheKey> = self
            .entries
            .keys()
            .copied()
            .filter(|key| key.video_id == video_id)
            .collect();
        for key in keys {
            if let Some(entry) = self.entries.remove(&key) {
                self.total_bytes = self.total_bytes.saturating_sub(entry.bytes);
                dropped.push(entry.image);
            }
        }
        self.lru.retain(|key| key.video_id != video_id);
        dropped
    }
}

// ---------------------------------------------------------------------------
// NV12 Surface RAM Cache (macOS only)
// Caches decoded CVPixelBuffer (IOSurface-backed) frames to avoid re-decoding.
// Mirrors the FrameRamCache design but stores zero-copy NV12 surfaces.
// ---------------------------------------------------------------------------
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SurfaceCacheKey {
    video_id: u64,
    pts_ns: u64,
}

#[cfg(target_os = "macos")]
struct SurfaceCacheEntry {
    surface: CVPixelBuffer,
    estimated_bytes: usize,
}

#[cfg(target_os = "macos")]
struct SurfaceRamCache {
    entries: HashMap<SurfaceCacheKey, SurfaceCacheEntry>,
    lru: VecDeque<SurfaceCacheKey>,
    total_bytes: usize,
    budget_bytes: usize,
    hits: u64,
    misses: u64,
}

#[cfg(target_os = "macos")]
impl SurfaceRamCache {
    fn new() -> Self {
        let budget = FrameRamCache::budget_bytes();
        Self {
            entries: HashMap::new(),
            lru: VecDeque::new(),
            total_bytes: 0,
            budget_bytes: budget,
            hits: 0,
            misses: 0,
        }
    }

    fn touch(&mut self, key: SurfaceCacheKey) {
        if let Some(pos) = self.lru.iter().position(|k| *k == key) {
            let _ = self.lru.remove(pos);
        }
        self.lru.push_back(key);
    }

    fn get(&mut self, key: SurfaceCacheKey) -> Option<CVPixelBuffer> {
        let surface = self.entries.get(&key).map(|e| e.surface.clone());
        if surface.is_some() {
            self.hits = self.hits.saturating_add(1);
            self.touch(key);
        } else {
            self.misses = self.misses.saturating_add(1);
        }
        if (self.hits + self.misses) % 120 == 0 {
            // Print cache stats periodically for diagnostics
            eprintln!(
                "[VideoElement][SurfaceRamCache] entries={} mem_mb={:.1}/{:.1} hits={} misses={} hit_rate={:.1}%",
                self.entries.len(),
                self.total_bytes as f64 / (1024.0 * 1024.0),
                self.budget_bytes as f64 / (1024.0 * 1024.0),
                self.hits,
                self.misses,
                if self.hits + self.misses > 0 {
                    self.hits as f64 / (self.hits + self.misses) as f64 * 100.0
                } else {
                    0.0
                }
            );
        }
        surface
    }

    fn insert(&mut self, key: SurfaceCacheKey, surface: CVPixelBuffer, estimated_bytes: usize) {
        if let Some(old) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(old.estimated_bytes);
            self.touch(key);
        } else {
            self.lru.push_back(key);
        }

        self.total_bytes = self.total_bytes.saturating_add(estimated_bytes);
        self.entries.insert(
            key,
            SurfaceCacheEntry {
                surface,
                estimated_bytes,
            },
        );

        while self.total_bytes > self.budget_bytes {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.estimated_bytes);
            }
        }
    }

    fn clear_video(&mut self, video_id: u64) {
        let keys: Vec<SurfaceCacheKey> = self
            .entries
            .keys()
            .copied()
            .filter(|key| key.video_id == video_id)
            .collect();
        for key in keys {
            if let Some(entry) = self.entries.remove(&key) {
                self.total_bytes = self.total_bytes.saturating_sub(entry.estimated_bytes);
            }
        }
        self.lru.retain(|key| key.video_id != video_id);
    }
}

/// Render a decoded video frame into GPUI by converting engine BGRA bytes into a `RenderImage`.
pub struct VideoElement {
    video: Video,
    element_id: Option<ElementId>,
    brightness: f32,
    contrast: f32,
    saturation: f32,
    lut_mix: f32,
    opacity: f32,
    blur_sigma: f32,
    tint_hue: f32,
    tint_saturation: f32,
    tint_lightness: f32,
    tint_alpha: f32,
    rotation_deg: f32,
    transform_scale: f32,
    transform_pos_x: f32,
    transform_pos_y: f32,
    transform_ref_width: f32,
    transform_ref_height: f32,
    blur_fast_mode: bool,
    local_mask_enabled: bool,
    local_mask_center_x: f32,
    local_mask_center_y: f32,
    local_mask_radius: f32,
    local_mask_feather: f32,
    local_mask_strength: f32,
    local_brightness: f32,
    local_contrast: f32,
    local_saturation: f32,
    local_opacity: f32,
    local_blur_sigma: f32,
    local_mask_layers: SmallVec<[VideoLocalMaskLayer; VIDEO_MAX_LOCAL_MASK_LAYERS]>,
}

impl VideoElement {
    pub fn new(video: Video) -> Self {
        Self {
            video,
            element_id: None,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            lut_mix: 0.0,
            opacity: 1.0,
            blur_sigma: 0.0,
            tint_hue: 0.0,
            tint_saturation: 0.0,
            tint_lightness: 0.0,
            tint_alpha: 0.0,
            rotation_deg: 0.0,
            transform_scale: 1.0,
            transform_pos_x: 0.0,
            transform_pos_y: 0.0,
            transform_ref_width: 1920.0,
            transform_ref_height: 1080.0,
            blur_fast_mode: false,
            local_mask_enabled: false,
            local_mask_center_x: 0.5,
            local_mask_center_y: 0.5,
            local_mask_radius: 0.25,
            local_mask_feather: 0.15,
            local_mask_strength: 1.0,
            local_brightness: 0.0,
            local_contrast: 1.0,
            local_saturation: 1.0,
            local_opacity: 1.0,
            local_blur_sigma: 0.0,
            local_mask_layers: SmallVec::new(),
        }
    }

    // Allow parent views to assign a stable element id.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.element_id = Some(id.into());
        self
    }

    pub fn color_balance(mut self, brightness: f32, contrast: f32, saturation: f32) -> Self {
        self.brightness = brightness;
        self.contrast = contrast;
        self.saturation = saturation;
        self
    }

    pub fn lut_mix(mut self, lut_mix: f32) -> Self {
        self.lut_mix = lut_mix.clamp(0.0, 1.0);
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    pub fn blur_sigma(mut self, blur_sigma: f32) -> Self {
        // Signed blur contract:
        // > 0 : blur sigma
        // < 0 : sharpen amount
        self.blur_sigma = blur_sigma.clamp(-64.0, 64.0);
        self
    }

    pub fn tint_overlay(mut self, hue: f32, saturation: f32, lightness: f32, alpha: f32) -> Self {
        self.tint_hue = hue.clamp(0.0, 360.0);
        self.tint_saturation = saturation.clamp(0.0, 1.0);
        self.tint_lightness = lightness.clamp(0.0, 1.0);
        self.tint_alpha = alpha.clamp(0.0, 1.0);
        self
    }

    pub fn rotation_deg(mut self, rotation_deg: f32) -> Self {
        self.rotation_deg = rotation_deg.clamp(-180.0, 180.0);
        self
    }

    pub fn preview_transform(
        mut self,
        scale: f32,
        pos_x: f32,
        pos_y: f32,
        canvas_w: f32,
        canvas_h: f32,
    ) -> Self {
        self.transform_scale = scale.clamp(0.01, 5.0);
        self.transform_pos_x = pos_x.clamp(-1.0, 1.0);
        self.transform_pos_y = pos_y.clamp(-1.0, 1.0);
        self.transform_ref_width = canvas_w.max(1.0);
        self.transform_ref_height = canvas_h.max(1.0);
        self
    }

    pub fn blur_fast_mode(mut self, blur_fast_mode: bool) -> Self {
        self.blur_fast_mode = blur_fast_mode;
        self
    }

    fn ensure_first_local_layer(&mut self) -> &mut VideoLocalMaskLayer {
        if self.local_mask_layers.is_empty() {
            self.local_mask_layers.push(VideoLocalMaskLayer {
                enabled: self.local_mask_enabled,
                center_x: self.local_mask_center_x,
                center_y: self.local_mask_center_y,
                radius: self.local_mask_radius,
                feather: self.local_mask_feather,
                strength: self.local_mask_strength,
                brightness: self.local_brightness,
                contrast: self.local_contrast,
                saturation: self.local_saturation,
                opacity: self.local_opacity,
                blur_sigma: self.local_blur_sigma,
            });
        }
        self.local_mask_layers
            .first_mut()
            .expect("first local mask layer must exist")
    }

    fn effective_local_mask_layers(
        &self,
    ) -> SmallVec<[VideoLocalMaskLayer; VIDEO_MAX_LOCAL_MASK_LAYERS]> {
        if self.local_mask_layers.is_empty() {
            let mut layers = SmallVec::<[VideoLocalMaskLayer; VIDEO_MAX_LOCAL_MASK_LAYERS]>::new();
            layers.push(VideoLocalMaskLayer {
                enabled: self.local_mask_enabled,
                center_x: self.local_mask_center_x,
                center_y: self.local_mask_center_y,
                radius: self.local_mask_radius,
                feather: self.local_mask_feather,
                strength: self.local_mask_strength,
                brightness: self.local_brightness,
                contrast: self.local_contrast,
                saturation: self.local_saturation,
                opacity: self.local_opacity,
                blur_sigma: self.local_blur_sigma,
            });
            return layers;
        }
        self.local_mask_layers
            .iter()
            .take(VIDEO_MAX_LOCAL_MASK_LAYERS)
            .copied()
            .collect()
    }

    pub fn local_mask_layers(mut self, layers: &[VideoLocalMaskLayer]) -> Self {
        self.local_mask_layers.clear();
        for layer in layers.iter().take(VIDEO_MAX_LOCAL_MASK_LAYERS) {
            self.local_mask_layers.push(VideoLocalMaskLayer {
                enabled: layer.enabled,
                center_x: layer.center_x.clamp(0.0, 1.0),
                center_y: layer.center_y.clamp(0.0, 1.0),
                radius: layer.radius.clamp(0.0, 1.0),
                feather: layer.feather.clamp(0.0, 1.0),
                strength: layer.strength.clamp(0.0, 1.0),
                brightness: layer.brightness.clamp(-1.0, 1.0),
                contrast: layer.contrast.clamp(0.0, 2.0),
                saturation: layer.saturation.clamp(0.0, 2.0),
                opacity: layer.opacity.clamp(0.0, 1.0),
                blur_sigma: layer.blur_sigma.clamp(0.0, 64.0),
            });
        }
        self
    }

    pub fn local_mask(
        mut self,
        enabled: bool,
        center_x: f32,
        center_y: f32,
        radius: f32,
        feather: f32,
        strength: f32,
    ) -> Self {
        self.local_mask_enabled = enabled;
        self.local_mask_center_x = center_x.clamp(0.0, 1.0);
        self.local_mask_center_y = center_y.clamp(0.0, 1.0);
        self.local_mask_radius = radius.clamp(0.0, 1.0);
        self.local_mask_feather = feather.clamp(0.0, 1.0);
        self.local_mask_strength = strength.clamp(0.0, 1.0);
        let enabled = self.local_mask_enabled;
        let center_x = self.local_mask_center_x;
        let center_y = self.local_mask_center_y;
        let radius = self.local_mask_radius;
        let feather = self.local_mask_feather;
        let strength = self.local_mask_strength;
        let layer = self.ensure_first_local_layer();
        layer.enabled = enabled;
        layer.center_x = center_x;
        layer.center_y = center_y;
        layer.radius = radius;
        layer.feather = feather;
        layer.strength = strength;
        self
    }

    pub fn local_mask_adjust(
        mut self,
        brightness: f32,
        contrast: f32,
        saturation: f32,
        opacity: f32,
        blur_sigma: f32,
    ) -> Self {
        self.local_brightness = brightness.clamp(-1.0, 1.0);
        self.local_contrast = contrast.clamp(0.0, 2.0);
        self.local_saturation = saturation.clamp(0.0, 2.0);
        self.local_opacity = opacity.clamp(0.0, 1.0);
        self.local_blur_sigma = blur_sigma.clamp(0.0, 64.0);
        let brightness = self.local_brightness;
        let contrast = self.local_contrast;
        let saturation = self.local_saturation;
        let opacity = self.local_opacity;
        let blur_sigma = self.local_blur_sigma;
        let layer = self.ensure_first_local_layer();
        layer.brightness = brightness;
        layer.contrast = contrast;
        layer.saturation = saturation;
        layer.opacity = opacity;
        layer.blur_sigma = blur_sigma;
        self
    }

    /// Compute aspect-fit destination bounds inside the container.
    fn fitted_bounds(
        &self,
        bounds: Bounds<Pixels>,
        frame_width: u32,
        frame_height: u32,
    ) -> Bounds<Pixels> {
        let container_w: f32 = bounds.size.width.into();
        let container_h: f32 = bounds.size.height.into();
        let frame_w = frame_width as f32;
        let frame_h = frame_height as f32;

        // Guard against invalid source size.
        if frame_w == 0.0 || frame_h == 0.0 {
            return bounds;
        }

        let scale = (container_w / frame_w).min(container_h / frame_h);

        let dest_w = frame_w * scale;
        let dest_h = frame_h * scale;
        let offset_x = (container_w - dest_w) * 0.5;
        let offset_y = (container_h - dest_h) * 0.5;

        Bounds::new(
            gpui::point(
                bounds.origin.x + gpui::px(offset_x),
                bounds.origin.y + gpui::px(offset_y),
            ),
            gpui::size(gpui::px(dest_w), gpui::px(dest_h)),
        )
    }

    fn apply_color_correction(&self, data: &mut [u8]) {
        let b = self.brightness.clamp(-1.0, 1.0) * 255.0;
        let c = self.contrast.clamp(0.0, 2.0);
        let s = self.saturation.clamp(0.0, 2.0);
        let lut_mix = self.lut_mix.clamp(0.0, 1.0);
        let opacity = self.opacity.clamp(0.0, 1.0);
        let tint_alpha = self.tint_alpha.clamp(0.0, 1.0);
        let (tint_r, tint_g, tint_b) = hsla_to_rgb(
            self.tint_hue,
            self.tint_saturation.clamp(0.0, 1.0),
            self.tint_lightness.clamp(0.0, 1.0),
        );
        let tint_r_255 = tint_r * 255.0;
        let tint_g_255 = tint_g * 255.0;
        let tint_b_255 = tint_b * 255.0;
        if b.abs() < 0.001
            && (c - 1.0).abs() < 0.001
            && (s - 1.0).abs() < 0.001
            && lut_mix < 0.001
            && (opacity - 1.0).abs() < 0.001
            && tint_alpha < 0.001
        {
            return;
        }

        for px in data.chunks_mut(4) {
            let b0 = px[0] as f32;
            let g0 = px[1] as f32;
            let r0 = px[2] as f32;

            let mut r = r0;
            let mut g = g0;
            let mut bch = b0;

            let l = 0.2126 * r + 0.7152 * g + 0.0722 * bch;
            r = l + (r - l) * s;
            g = l + (g - l) * s;
            bch = l + (bch - l) * s;

            r = (r - 128.0) * c + 128.0 + b;
            g = (g - 128.0) * c + 128.0 + b;
            bch = (bch - 128.0) * c + 128.0 + b;
            if lut_mix > 0.001 {
                let warm_r = r * 1.03;
                let warm_g = g;
                let warm_b = bch * 0.97;
                r = r + (warm_r - r) * lut_mix;
                g = g + (warm_g - g) * lut_mix;
                bch = bch + (warm_b - bch) * lut_mix;
            }
            if tint_alpha > 0.001 {
                r = r + (tint_r_255 - r) * tint_alpha;
                g = g + (tint_g_255 - g) * tint_alpha;
                bch = bch + (tint_b_255 - bch) * tint_alpha;
            }

            px[2] = r.clamp(0.0, 255.0) as u8;
            px[1] = g.clamp(0.0, 255.0) as u8;
            px[0] = bch.clamp(0.0, 255.0) as u8;
            if (opacity - 1.0).abs() > 0.001 {
                px[3] = ((px[3] as f32) * opacity).clamp(0.0, 255.0) as u8;
            }
        }
    }

    fn apply_transform_cpu(&self, data: &mut [u8], width: u32, height: u32) {
        if width == 0 || height == 0 || !self.has_transform_effects() {
            return;
        }
        let Ok(width_usize) = usize::try_from(width) else {
            return;
        };
        let Ok(height_usize) = usize::try_from(height) else {
            return;
        };
        let pixel_count = width_usize.saturating_mul(height_usize);
        let expected_len = pixel_count.saturating_mul(4);
        if data.len() != expected_len {
            return;
        }

        let source = data.to_vec();
        let mut transformed = vec![0_u8; source.len()];
        let width_f = width as f32;
        let height_f = height as f32;
        let aspect = width_f / height_f.max(1e-6);
        let ref_w = self.transform_ref_width.max(1.0);
        let ref_h = self.transform_ref_height.max(1.0);
        let pos_x_norm = self.transform_pos_x * (ref_w / width_f.max(1.0));
        let pos_y_norm = self.transform_pos_y * (ref_h / height_f.max(1.0));
        let inv_scale = 1.0 / self.transform_scale.clamp(0.01, 5.0).max(1e-6);
        let angle = self.rotation_deg.clamp(-180.0, 180.0);
        let rotation_enabled = angle.abs() >= 0.001;
        let theta = angle.to_radians();
        let sin_t = theta.sin();
        let cos_t = theta.cos();

        for y in 0..height_usize {
            for x in 0..width_usize {
                let out_uv_x = ((x as f32) + 0.5) / width_f - 0.5;
                let out_uv_y = ((y as f32) + 0.5) / height_f - 0.5;
                let out_center_x = out_uv_x * aspect - pos_x_norm * aspect;
                let out_center_y = out_uv_y - pos_y_norm;

                let (mut src_center_x, mut src_center_y) = (out_center_x, out_center_y);
                if rotation_enabled {
                    src_center_x = out_center_x * cos_t + out_center_y * sin_t;
                    src_center_y = -out_center_x * sin_t + out_center_y * cos_t;
                }

                src_center_x *= inv_scale;
                src_center_y *= inv_scale;
                let src_uv_x = src_center_x / aspect + 0.5;
                let src_uv_y = src_center_y + 0.5;

                if !(0.0..1.0).contains(&src_uv_x) || !(0.0..1.0).contains(&src_uv_y) {
                    continue;
                }

                let sx = (src_uv_x * width_f).clamp(0.0, width_f - 1.0).floor() as usize;
                let sy = (src_uv_y * height_f).clamp(0.0, height_f - 1.0).floor() as usize;
                let src_idx = (sy * width_usize + sx) * 4;
                let dst_idx = (y * width_usize + x) * 4;
                transformed[dst_idx..dst_idx + 4].copy_from_slice(&source[src_idx..src_idx + 4]);
            }
        }

        data.copy_from_slice(&transformed);
    }

    fn effective_blur_sigma(&self) -> f32 {
        let mut sigma = self.effective_signed_blur_sigma().abs();
        if self.blur_fast_mode {
            sigma = (sigma * 2.0).round() * 0.5;
        }
        sigma
    }

    fn effective_signed_blur_sigma(&self) -> f32 {
        let mut sigma = self.blur_sigma.clamp(-64.0, 64.0);
        if self.blur_fast_mode {
            let sign = if sigma < 0.0 { -1.0 } else { 1.0 };
            sigma = sign * ((sigma.abs() * 2.0).round() * 0.5);
        }
        sigma
    }

    #[cfg(target_os = "macos")]
    fn has_nv12_color_processing(&self) -> bool {
        // Transform is now handled by the anica render shader (surface_vertex_anica),
        // so it no longer triggers the Metal compute blur/color path.
        self.brightness.abs() >= 0.001
            || (self.contrast - 1.0).abs() >= 0.001
            || (self.saturation - 1.0).abs() >= 0.001
            || self.lut_mix.abs() >= 0.001
            || self.tint_alpha.abs() >= 0.001
    }

    fn local_mask_layer_has_shape(layer: &VideoLocalMaskLayer) -> bool {
        layer.enabled
            && layer.strength >= 0.001
            && layer.radius >= 0.0001
            && (layer.feather >= 0.0001 || layer.radius > 0.0001)
    }

    fn local_mask_layer_has_color_or_alpha_effect(layer: &VideoLocalMaskLayer) -> bool {
        layer.brightness.abs() >= 0.001
            || (layer.contrast - 1.0).abs() >= 0.001
            || (layer.saturation - 1.0).abs() >= 0.001
            || (layer.opacity - 1.0).abs() >= 0.001
    }

    fn local_mask_active(&self) -> bool {
        self.effective_local_mask_layers()
            .iter()
            .any(Self::local_mask_layer_has_shape)
    }

    fn has_color_or_alpha_effects(&self) -> bool {
        self.brightness.abs() >= 0.001
            || (self.contrast - 1.0).abs() >= 0.001
            || (self.saturation - 1.0).abs() >= 0.001
            || self.lut_mix.abs() >= 0.001
            || (self.opacity - 1.0).abs() >= 0.001
            || self.tint_alpha.abs() >= 0.001
    }

    fn has_transform_effects(&self) -> bool {
        self.rotation_deg.abs() >= 0.001
            || (self.transform_scale - 1.0).abs() >= 0.001
            || self.transform_pos_x.abs() >= 0.001
            || self.transform_pos_y.abs() >= 0.001
    }

    fn has_local_color_or_alpha_effects(&self) -> bool {
        self.effective_local_mask_layers().iter().any(|layer| {
            Self::local_mask_layer_has_shape(layer)
                && Self::local_mask_layer_has_color_or_alpha_effect(layer)
        })
    }

    fn has_local_blur_effects(&self) -> bool {
        self.effective_local_mask_layers()
            .iter()
            .any(|layer| Self::local_mask_layer_has_shape(layer) && layer.blur_sigma.abs() >= 0.001)
    }

    #[cfg(target_os = "macos")]
    fn apply_metal_nv12_effects_surface(&self, surface: &CVPixelBuffer) -> Option<CVPixelBuffer> {
        let sigma = self.effective_signed_blur_sigma();
        let has_color = self.has_nv12_color_processing();
        if sigma.abs() <= 0.001 && !has_color {
            return Some(surface.clone());
        }
        if nv12_debug_enabled() {
            static SYNC_HIT_COUNT: AtomicU64 = AtomicU64::new(0);
            let hit = SYNC_HIT_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            if hit <= 20 || hit % 120 == 0 {
                let pf = surface.get_pixel_format();
                log::info!(
                    "[VideoElement][NV12FX][sync] hit={} video_id={} fmt={}({:#x}) sigma={:.3} b={:.3} c={:.3} s={:.3} lut={:.3} tintA={:.3}",
                    hit,
                    self.video.id(),
                    nv12_pixel_format_tag(pf),
                    pf,
                    sigma,
                    self.brightness,
                    self.contrast,
                    self.saturation,
                    self.lut_mix,
                    self.tint_alpha
                );
            }
        }

        let mut effect_ok = false;
        let mut out_surface: Option<CVPixelBuffer> = None;
        METAL_BLUR_CONTEXT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                match MetalGaussianBlurContext::new() {
                    Ok(ctx) => {
                        *slot = Some(ctx);
                        METAL_BLUR_INIT_FAILED.store(false, Ordering::Relaxed);
                    }
                    Err(err) => {
                        if !METAL_BLUR_INIT_FAILED.swap(true, Ordering::Relaxed) {
                            log::error!("[VideoElement][MetalBlur] init failed: {}", err);
                        }
                        return;
                    }
                }
            }
            if let Some(ctx) = slot.as_mut() {
                // Transform (rotation/scale/position) is now handled by
                // the anica render shader, so pass identity values here.
                match ctx.process_nv12_surface_zero_copy(
                    surface,
                    sigma,
                    self.brightness,
                    self.contrast,
                    self.saturation,
                    self.lut_mix,
                    0.0, // rotation_deg — handled by surface_vertex_anica
                    1.0, // transform_scale — handled by surface_vertex_anica
                    0.0, // transform_pos_x — handled by surface_vertex_anica
                    0.0, // transform_pos_y — handled by surface_vertex_anica
                    self.transform_ref_width,
                    self.transform_ref_height,
                    self.tint_hue,
                    self.tint_saturation,
                    self.tint_lightness,
                    self.tint_alpha,
                ) {
                    Ok(blurred) => {
                        METAL_BLUR_RUNTIME_FAILED.store(false, Ordering::Relaxed);
                        effect_ok = true;
                        out_surface = Some(blurred);
                    }
                    Err(err) => {
                        if !METAL_BLUR_RUNTIME_FAILED.swap(true, Ordering::Relaxed) {
                            log::error!("[VideoElement][MetalBlur] NV12 effects failed: {}", err);
                        }
                    }
                }
            }
        });
        if !effect_ok {
            return None;
        }
        out_surface
    }

    /// Submit non-blocking NV12 blur via Metal compute. Stores the pending
    /// job in `pending_slot`; caller polls completion on the next frame.
    /// Returns false if the submission fails (caller should fall back to sync).
    #[cfg(target_os = "macos")]
    fn submit_async_nv12_blur(
        &self,
        surface: &CVPixelBuffer,
        frame_pts_ns: u64,
        pixel_key: PixelProcessKey,
        pending_slot: &Entity<Option<PendingNv12Blur>>,
        cx: &mut gpui::App,
    ) -> bool {
        let sigma = self.effective_signed_blur_sigma();
        let has_color = self.has_nv12_color_processing();
        if sigma.abs() <= 0.001 && !has_color {
            // No blur/color needed — nothing to submit.
            return false;
        }
        if nv12_debug_enabled() {
            static ASYNC_HIT_COUNT: AtomicU64 = AtomicU64::new(0);
            let hit = ASYNC_HIT_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            if hit <= 20 || hit % 120 == 0 {
                let pf = surface.get_pixel_format();
                log::info!(
                    "[VideoElement][NV12FX][submit] hit={} video_id={} pts={} fmt={}({:#x}) sigma={:.3} b={:.3} c={:.3} s={:.3} lut={:.3} tintA={:.3}",
                    hit,
                    self.video.id(),
                    frame_pts_ns,
                    nv12_pixel_format_tag(pf),
                    pf,
                    sigma,
                    self.brightness,
                    self.contrast,
                    self.saturation,
                    self.lut_mix,
                    self.tint_alpha
                );
            }
        }

        let mut submitted = false;
        METAL_BLUR_CONTEXT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                match MetalGaussianBlurContext::new() {
                    Ok(ctx) => {
                        *slot = Some(ctx);
                        METAL_BLUR_INIT_FAILED.store(false, Ordering::Relaxed);
                    }
                    Err(err) => {
                        if !METAL_BLUR_INIT_FAILED.swap(true, Ordering::Relaxed) {
                            log::error!("[VideoElement][AsyncNv12Blur] init failed: {}", err);
                        }
                        return;
                    }
                }
            }
            if let Some(ctx) = slot.as_mut() {
                // Transform is handled by the anica render shader — pass identity values.
                match ctx.process_nv12_surface_no_wait(
                    surface,
                    sigma,
                    self.brightness,
                    self.contrast,
                    self.saturation,
                    self.lut_mix,
                    0.0, // rotation_deg — handled by surface_vertex_anica
                    1.0, // transform_scale — handled by surface_vertex_anica
                    0.0, // transform_pos_x — handled by surface_vertex_anica
                    0.0, // transform_pos_y — handled by surface_vertex_anica
                    self.transform_ref_width,
                    self.transform_ref_height,
                    self.tint_hue,
                    self.tint_saturation,
                    self.tint_lightness,
                    self.tint_alpha,
                ) {
                    Ok((dest, cmd_buf)) => {
                        let _ = pending_slot.update(cx, |state, _| {
                            *state = Some(PendingNv12Blur {
                                cmd_buf,
                                dest_surface: dest,
                                frame_pts_ns,
                                pixel_key,
                            });
                        });
                        submitted = true;
                    }
                    Err(err) => {
                        if !METAL_BLUR_RUNTIME_FAILED.swap(true, Ordering::Relaxed) {
                            log::error!("[VideoElement][AsyncNv12Blur] submit failed: {}", err);
                        }
                    }
                }
            }
        });
        submitted
    }

    fn apply_gaussian_blur(&self, data: &mut Vec<u8>, width: u32, height: u32) {
        let sigma = self.effective_blur_sigma();
        if sigma <= 0.001 || width == 0 || height == 0 {
            return;
        }

        let raw = data.clone();
        if let Some(buffer) = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, raw) {
            let blurred = imageops::blur(&buffer, sigma);
            *data = blurred.into_raw();
        }
    }

    fn apply_unsharp_with_amount(
        &self,
        data: &mut Vec<u8>,
        width: u32,
        height: u32,
        sigma: f32,
        amount: f32,
    ) {
        let sigma = sigma.clamp(0.0, 64.0);
        let amount = amount.clamp(0.0, 4.0);
        if sigma <= 0.001 || amount <= 0.0001 || width == 0 || height == 0 {
            return;
        }

        let raw = data.clone();
        if let Some(buffer) = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, raw.clone()) {
            let blurred = imageops::blur(&buffer, sigma).into_raw();
            let mut out = raw;
            for i in (0..out.len()).step_by(4) {
                for ch in 0..3 {
                    // BGRA in byte memory.
                    let b = out[i + ch] as f32 / 255.0;
                    let bl = blurred[i + ch] as f32 / 255.0;
                    let v = (b + (b - bl) * amount).clamp(0.0, 1.0);
                    out[i + ch] = (v * 255.0 + 0.5) as u8;
                }
            }
            *data = out;
        }
    }

    fn apply_unsharp(&self, data: &mut Vec<u8>, width: u32, height: u32) {
        let sigma = self.effective_blur_sigma();
        let stages = sharpen_stages_for_sigma(sigma);
        for stage in &stages {
            self.apply_unsharp_with_amount(data, width, height, stage.sigma, stage.amount);
        }
    }

    fn apply_wgpu_bgra_effects(&self, data: &mut Vec<u8>, width: u32, height: u32) -> bool {
        if WGPU_BGRA_CPU_SAFE_MODE.load(Ordering::Relaxed) {
            return false;
        }

        let sigma = self.effective_signed_blur_sigma();
        let local_layers = self.effective_local_mask_layers();
        let mut applied = false;
        WGPU_BGRA_CONTEXT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                match WgpuBgraEffectContext::new() {
                    Ok(ctx) => {
                        *slot = Some(ctx);
                    }
                    Err(err) => {
                        set_cpu_safe_mode(format!("WGPU init failed: {err}"));
                        return;
                    }
                }
            }

            if let Some(ctx) = slot.as_mut() {
                match ctx.process_frame(
                    data,
                    width,
                    height,
                    self.brightness,
                    self.contrast,
                    self.saturation,
                    self.lut_mix,
                    self.opacity,
                    self.rotation_deg,
                    self.transform_scale,
                    self.transform_pos_x,
                    self.transform_pos_y,
                    self.transform_ref_width,
                    self.transform_ref_height,
                    self.tint_hue,
                    self.tint_saturation,
                    self.tint_lightness,
                    self.tint_alpha,
                    sigma,
                    &local_layers,
                ) {
                    Ok(()) => {
                        applied = true;
                    }
                    Err(err) => {
                        let lost = ctx.device_lost.load(Ordering::Relaxed);
                        *slot = None;
                        if WGPU_BGRA_CPU_SAFE_MODE.load(Ordering::Relaxed) || lost {
                            set_cpu_safe_mode(format!("WGPU device-lost fallback: {err}"));
                        } else {
                            log::error!(
                                "[VideoElement][WgpuBgra] runtime failed (keeping GPU path): {}",
                                err
                            );
                        }
                    }
                }
            }
        });
        applied
    }

    fn apply_pixel_processing(&self, data: &mut Vec<u8>, width: u32, height: u32) {
        let has_blur = self.blur_sigma >= 0.001;
        let has_sharpen = self.blur_sigma <= -0.001;
        let has_transform = self.has_transform_effects();
        let local_mask_active = self.local_mask_active();
        let has_local_blur = self.has_local_blur_effects();
        let has_color_effects = self.has_color_or_alpha_effects();
        let has_local_effects =
            local_mask_active && (has_local_blur || self.has_local_color_or_alpha_effects());
        if !has_blur && !has_sharpen && !has_color_effects && !has_local_effects && !has_transform {
            return;
        }
        if self.apply_wgpu_bgra_effects(data, width, height) {
            return;
        }
        if !WGPU_BGRA_CPU_SAFE_MODE.load(Ordering::Relaxed) {
            // Keep behavior deterministic: CPU fallback is reserved for explicit safe mode.
            return;
        }
        if has_blur {
            self.apply_gaussian_blur(data, width, height);
        } else if has_sharpen {
            self.apply_unsharp(data, width, height);
        }
        if has_transform {
            self.apply_transform_cpu(data, width, height);
        }
        if has_color_effects {
            self.apply_color_correction(data);
        } else if local_mask_active {
            // CPU safe mode does not implement local blur-mask blending yet.
        }
    }

    /// Build extended surface params for the anica NV12 zero-copy shader.
    /// Converts clip opacity / scale / rotation / position into GPU params.
    #[cfg(target_os = "macos")]
    fn build_surface_params_anica(
        &self,
        dest_bounds: &gpui::Bounds<gpui::Pixels>,
    ) -> gpui::SurfaceExParams_anica {
        let dest_w: f32 = dest_bounds.size.width.into();
        let dest_h: f32 = dest_bounds.size.height.into();
        let ref_w = self.transform_ref_width.max(1.0);
        let ref_h = self.transform_ref_height.max(1.0);
        // Map reference-space position to destination pixel offset.
        let translate_x = self.transform_pos_x * (dest_w / ref_w);
        let translate_y = self.transform_pos_y * (dest_h / ref_h);
        gpui::SurfaceExParams_anica {
            opacity: self.opacity.clamp(0.0, 1.0),
            scale: self.transform_scale.clamp(0.01, 5.0),
            rotation_deg: self.rotation_deg.clamp(-180.0, 180.0),
            translate: gpui::point(
                gpui::ScaledPixels::from(translate_x),
                gpui::ScaledPixels::from(translate_y),
            ),
        }
    }

    fn needs_pixel_processing(&self) -> bool {
        let has_localized_effect = self.local_mask_active()
            && (self.has_local_blur_effects() || self.has_local_color_or_alpha_effects());
        #[cfg(target_os = "macos")]
        {
            // NV12 Metal path already supports global color/blur/LUT/tint processing.
            // Keep BGRA fallback only for local-mask compositing.
            self.local_mask_active() || has_localized_effect
        }
        #[cfg(not(target_os = "macos"))]
        {
            let has_transform = self.has_transform_effects();
            let has_color_or_alpha = self.has_color_or_alpha_effects();
            has_color_or_alpha
                || has_transform
                || self.blur_sigma.abs() >= 0.001
                || has_localized_effect
        }
    }
}

impl Element for VideoElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        self.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // Fill parent bounds by default.
        let style = Style {
            size: Size {
                width: Length::Definite(DefiniteLength::Fraction(1.0)),
                height: Length::Definite(DefiniteLength::Fraction(1.0)),
            },
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        // Keep animation ticking while playing or while a fresh frame is pending upload.
        if !self.video.paused() || self.video.peek_frame_ready() {
            window.request_animation_frame();
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        let last_render_image: Entity<Option<Arc<RenderImage>>> = window.use_state(cx, |_, _| None);
        let last_pixel_key: Entity<Option<PixelProcessKey>> = window.use_state(cx, |_, _| None);
        let last_paused_state: Entity<Option<bool>> = window.use_state(cx, |_, _| None);
        let frame_ram_cache: Entity<FrameRamCache> =
            window.use_state(cx, |_, _| FrameRamCache::new());
        #[cfg(target_os = "macos")]
        let last_surface_buffer: Entity<Option<CVPixelBuffer>> = window.use_state(cx, |_, _| None);
        #[cfg(target_os = "macos")]
        let surface_ram_cache: Entity<SurfaceRamCache> =
            window.use_state(cx, |_, _| SurfaceRamCache::new());
        #[cfg(target_os = "macos")]
        let last_surface_blur_cache: Entity<Option<SurfaceBlurCacheEntry>> =
            window.use_state(cx, |_, _| None);
        // Async NV12 blur: holds an in-flight Metal compute job.
        #[cfg(target_os = "macos")]
        let pending_nv12_blur: Entity<Option<PendingNv12Blur>> = window.use_state(cx, |_, _| None);
        #[cfg(target_os = "macos")]
        let nv12_effect_fail_streak: Entity<u32> = window.use_state(cx, |_, _| 0);
        let has_new_frame = self.video.take_frame_ready();
        let is_paused = self.video.paused();
        let was_paused = last_paused_state.read(cx).as_ref().copied();
        let playback_state_changed = was_paused != Some(is_paused);
        let _ = last_paused_state.update(cx, |state, _| state.replace(is_paused));
        let allow_ram_cache = is_paused;
        if playback_state_changed && !is_paused {
            // Continuous playback generates unique PTS frames and can balloon RAM.
            // Flush this video's paused/scrub caches when entering play.
            let dropped = frame_ram_cache.update(cx, |cache, _| cache.clear_video(self.video.id()));
            for img in dropped {
                cx.drop_image(img, Some(window));
            }
            #[cfg(target_os = "macos")]
            {
                surface_ram_cache.update(cx, |cache, _| cache.clear_video(self.video.id()));
                let _ = last_surface_buffer.update(cx, |state, _| state.take());
                let _ = last_surface_blur_cache.update(cx, |state, _| state.take());
                let _ = pending_nv12_blur.update(cx, |state, _| state.take());
                let _ = nv12_effect_fail_streak.update(cx, |streak, _| *streak = 0);
            }
        }
        let mut image_to_paint = last_render_image.read(cx).clone();
        let mut frame_size = self.video.display_size();
        let needs_pixel_processing = self.needs_pixel_processing();
        let strict_surface_only = self.video.strict_surface_only();
        let local_layers = self.effective_local_mask_layers();
        let pixel_key = PixelProcessKey::from_values(
            self.brightness,
            self.contrast,
            self.saturation,
            self.lut_mix,
            self.opacity,
            self.tint_hue,
            self.tint_saturation,
            self.tint_lightness,
            self.tint_alpha,
            self.blur_sigma,
            self.rotation_deg,
            self.transform_scale,
            self.transform_pos_x,
            self.transform_pos_y,
            self.transform_ref_width,
            self.transform_ref_height,
            self.blur_fast_mode,
            &local_layers,
        );
        let pixel_key_changed = last_pixel_key.read(cx).as_ref().copied() != Some(pixel_key);
        #[cfg(target_os = "macos")]
        if pixel_key_changed {
            let _ = nv12_effect_fail_streak.update(cx, |streak, _| *streak = 0);
        }
        let frame_pts_ns = self.video.last_frame_pts_ns();
        let frame_cache_key = FrameRamCacheKey {
            video_id: self.video.id(),
            pts_ns: frame_pts_ns,
            pixel_key,
        };

        #[cfg(target_os = "macos")]
        {
            // macOS keeps zero-copy NV12 surface path whenever local-mask fallback is not required.
            if needs_pixel_processing {
                // Local-mask compositing still uses BGRA fallback; drop stale NV12 caches first.
                let _ = last_surface_buffer.update(cx, |state, _| state.take());
                let _ = last_surface_blur_cache.update(cx, |state, _| state.take());
                let _ = pending_nv12_blur.update(cx, |state, _| state.take());
                let _ = nv12_effect_fail_streak.update(cx, |streak, _| *streak = 0);
            } else {
                let mut refreshed_surface = false;
                if has_new_frame || last_surface_buffer.read(cx).is_none() {
                    let cache_key = SurfaceCacheKey {
                        video_id: self.video.id(),
                        pts_ns: frame_pts_ns,
                    };

                    // 1. Try surface RAM cache first (avoids GStreamer decode)
                    let cached = if allow_ram_cache && frame_pts_ns > 0 {
                        surface_ram_cache.update(cx, |cache, _| cache.get(cache_key))
                    } else {
                        None
                    };

                    if let Some(surface) = cached {
                        // Cache hit — skip GStreamer decode entirely
                        let _ = last_surface_buffer.update(cx, |state, _| state.replace(surface));
                        refreshed_surface = true;
                    } else if let Some(surface) = self.video.current_frame_surface_nv12() {
                        // Cache miss — decoded via GStreamer, insert into cache for future reuse
                        if allow_ram_cache && frame_pts_ns > 0 {
                            let (w, h) = frame_size;
                            let estimated_bytes = (w as usize) * (h as usize) * 3 / 2;
                            surface_ram_cache.update(cx, |cache, _| {
                                cache.insert(cache_key, surface.clone(), estimated_bytes);
                            });
                        }
                        let _ = last_surface_buffer.update(cx, |state, _| state.replace(surface));
                        refreshed_surface = true;
                    }
                }

                if has_new_frame && !refreshed_surface {
                    // A new decoded frame exists but no NV12 surface was produced (e.g. BGRA pipeline mode).
                    let _ = last_surface_buffer.update(cx, |state, _| state.take());
                    let _ = last_surface_blur_cache.update(cx, |state, _| state.take());
                    let _ = pending_nv12_blur.update(cx, |state, _| state.take());
                }

                // If a surface exists, render it first. We still keep BGRA fallback below for safety.
                if let Some(surface) = last_surface_buffer.read(cx).clone() {
                    let (w, h) = frame_size;
                    let dest_bounds = self.fitted_bounds(bounds, w.max(1), h.max(1));
                    let has_surface_effects =
                        self.effective_blur_sigma() > 0.001 || self.has_nv12_color_processing();
                    let fail_streak = *nv12_effect_fail_streak.read(cx);
                    let mut rendered_on_surface = false;
                    if has_surface_effects && fail_streak >= 3 {
                        // Avoid expensive per-frame retries when NV12 effect dispatch keeps failing.
                        let _ = last_surface_blur_cache.update(cx, |state, _| state.take());
                        let _ = pending_nv12_blur.update(cx, |state, _| state.take());
                        let _ = last_surface_buffer.update(cx, |state, _| state.take());
                    } else if has_surface_effects {
                        // ── Async NV12 blur pipeline ──────────────────────────
                        // Step 1: Harvest completed GPU work from previous frame.
                        let async_completed = pending_nv12_blur.update(cx, |pending, _| {
                            if let Some(p) = pending.as_ref() {
                                let status = p.cmd_buf.status();
                                if status == MTLCommandBufferStatus::Completed {
                                    // GPU finished — take the result.
                                    let done = pending.take().unwrap();
                                    return Some((
                                        done.dest_surface,
                                        done.frame_pts_ns,
                                        done.pixel_key,
                                    ));
                                } else if status == MTLCommandBufferStatus::Error {
                                    log::error!(
                                        "[VideoElement][AsyncNv12Blur] command buffer error"
                                    );
                                    pending.take();
                                }
                            }
                            None
                        });
                        // Update the blur cache with the completed async result.
                        if let Some((completed_surface, completed_pts, completed_key)) =
                            async_completed
                        {
                            let entry = SurfaceBlurCacheEntry {
                                frame_pts_ns: completed_pts,
                                pixel_key: completed_key,
                                surface: completed_surface,
                            };
                            let _ =
                                last_surface_blur_cache.update(cx, |state, _| state.replace(entry));
                        }

                        // Step 2: Try exact cache hit (same frame + same params).
                        let mut blurred_surface = last_surface_blur_cache
                            .read(cx)
                            .as_ref()
                            .cloned()
                            .and_then(|cached| {
                                if cached.frame_pts_ns == frame_pts_ns
                                    && cached.pixel_key == pixel_key
                                {
                                    Some(cached.surface)
                                } else {
                                    None
                                }
                            });
                        let mut nv12_effect_failed = false;

                        // Step 3: Submit new async blur if no work is in flight.
                        let is_in_flight = pending_nv12_blur.read(cx).is_some();
                        if blurred_surface.is_none() && !is_in_flight {
                            // Dispatch non-blocking Metal compute.
                            let submit_ok = self.submit_async_nv12_blur(
                                &surface,
                                frame_pts_ns,
                                pixel_key,
                                &pending_nv12_blur,
                                cx,
                            );
                            if !submit_ok {
                                // Async submission failed.
                                // Try one synchronous recovery attempt, then switch to BGRA fallback.
                                if fail_streak == 0 {
                                    let sync_result =
                                        self.apply_metal_nv12_effects_surface(&surface);
                                    if let Some(s) = sync_result.as_ref() {
                                        let entry = SurfaceBlurCacheEntry {
                                            frame_pts_ns,
                                            pixel_key,
                                            surface: s.clone(),
                                        };
                                        let _ = last_surface_blur_cache
                                            .update(cx, |state, _| state.replace(entry));
                                        let _ = nv12_effect_fail_streak
                                            .update(cx, |streak, _| *streak = 0);
                                    }
                                    if sync_result.is_none() {
                                        nv12_effect_failed = true;
                                    }
                                    blurred_surface = sync_result;
                                } else {
                                    nv12_effect_failed = true;
                                }
                            } else {
                                // Async blur submitted — schedule a repaint so the
                                // completed result is harvested on the next frame.
                                // Without this, paused video never picks up the
                                // GPU-processed surface (e.g. Layer FX effects).
                                window.request_animation_frame();
                                let _ = nv12_effect_fail_streak.update(cx, |streak, _| *streak = 0);
                            }
                        }
                        let has_pending = pending_nv12_blur.read(cx).is_some();
                        // Also keep repainting while prior async work is still in flight.
                        if has_pending {
                            window.request_animation_frame();
                        }

                        // Step 4: Use stale cache if exact hit missed (1-frame delay during playback).
                        if blurred_surface.is_none() {
                            blurred_surface = last_surface_blur_cache
                                .read(cx)
                                .as_ref()
                                .map(|cached| cached.surface.clone());
                        }

                        if let Some(blurred) = blurred_surface {
                            // Use anica extended surface path for opacity/transform on NV12.
                            let params = self.build_surface_params_anica(&dest_bounds);
                            window.paint_surface_anica(dest_bounds, blurred, params);
                            let _ = nv12_effect_fail_streak.update(cx, |streak, _| *streak = 0);
                            rendered_on_surface = true;
                        } else if has_pending {
                            // No blur result yet (first frame). Render original surface unblurred
                            // to avoid a blank frame while GPU work is in flight.
                            let params = self.build_surface_params_anica(&dest_bounds);
                            window.paint_surface_anica(dest_bounds, surface.clone(), params);
                            let _ = nv12_effect_fail_streak.update(cx, |streak, _| *streak = 0);
                            rendered_on_surface = true;
                        } else {
                            // NV12 effect path is unavailable; clear surface caches and
                            // continue into BGRA fallback below so effects remain visible.
                            let fail_hits = nv12_effect_fail_streak.update(cx, |streak, _| {
                                *streak = streak.saturating_add(1);
                                *streak
                            });
                            if nv12_effect_failed
                                || METAL_BLUR_RUNTIME_FAILED.load(Ordering::Relaxed)
                            {
                                if fail_hits <= 3 || fail_hits % 120 == 0 {
                                    log::warn!(
                                        "[VideoElement][NV12] effect path unavailable -> BGRA fallback video_id={} streak={}",
                                        self.video.id(),
                                        fail_hits
                                    );
                                }
                            }
                            let _ = last_surface_blur_cache.update(cx, |state, _| state.take());
                            let _ = pending_nv12_blur.update(cx, |state, _| state.take());
                            let _ = last_surface_buffer.update(cx, |state, _| state.take());
                        }
                    } else {
                        let _ = last_surface_blur_cache.update(cx, |state, _| state.take());
                        let _ = pending_nv12_blur.update(cx, |state, _| state.take());
                        let _ = nv12_effect_fail_streak.update(cx, |streak, _| *streak = 0);
                        // Use anica extended surface path for opacity/transform on NV12.
                        let params = self.build_surface_params_anica(&dest_bounds);
                        window.paint_surface_anica(dest_bounds, surface, params);
                        rendered_on_surface = true;
                    }
                    if rendered_on_surface {
                        return;
                    }
                }

                if strict_surface_only {
                    // Strict proxy NV12 mode: do not switch to BGRA image fallback.
                    if has_new_frame {
                        static STRICT_MISS_COUNT: AtomicU64 = AtomicU64::new(0);
                        let hit = STRICT_MISS_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                        if hit <= 8 || hit % 120 == 0 {
                            let (w, h) = frame_size;
                            log::warn!(
                                "[VideoElement] strict-surface miss hit={} video_id={} frame={}x{}",
                                hit,
                                self.video.id(),
                                w.max(1),
                                h.max(1)
                            );
                        }
                    }
                    return;
                }
            }
        }

        // Update texture only when a fresh decoded frame arrives (or first paint has no texture yet).
        if has_new_frame || image_to_paint.is_none() || pixel_key_changed {
            // True RAM cache path: key by (video_id + frame pts + effect key).
            if allow_ram_cache
                && frame_pts_ns > 0
                && let Some(cached) =
                    frame_ram_cache.update(cx, |cache, _| cache.get(frame_cache_key))
            {
                image_to_paint = Some(cached.image.clone());
                frame_size = (cached.width, cached.height);
                let _ = last_render_image.update(cx, |state, _| state.replace(cached.image));
                let _ = last_pixel_key.update(cx, |state, _| state.replace(pixel_key));
            } else if let Some((mut bgra_data, width, height)) = self.video.current_frame_data() {
                self.apply_pixel_processing(&mut bgra_data, width, height);
                let frame_bytes = bgra_data.len();
                if let Some(image_buffer) =
                    ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, bgra_data)
                {
                    let frames = SmallVec::from_elem(image::Frame::new(image_buffer), 1);
                    let render_image = Arc::new(RenderImage::new(frames));
                    let prev_image = last_render_image
                        .update(cx, |state, _| state.replace(render_image.clone()));
                    image_to_paint = Some(render_image.clone());
                    frame_size = (width, height);
                    let _ = last_pixel_key.update(cx, |state, _| state.replace(pixel_key));

                    if allow_ram_cache && frame_pts_ns > 0 {
                        let evicted = frame_ram_cache.update(cx, |cache, _| {
                            cache.insert(
                                frame_cache_key,
                                FrameRamCacheEntry {
                                    image: render_image,
                                    width,
                                    height,
                                    bytes: frame_bytes,
                                },
                            )
                        });
                        for evicted_image in evicted {
                            cx.drop_image(evicted_image, Some(window));
                        }
                    } else if let Some(prev) = prev_image {
                        cx.drop_image(prev, Some(window));
                    }
                }
            }
        }

        if let Some(render_image) = image_to_paint {
            let (w, h) = frame_size;
            let dest_bounds = self.fitted_bounds(bounds, w.max(1), h.max(1));
            window
                .paint_image(
                    dest_bounds,
                    gpui::Corners::default(),
                    render_image,
                    0,
                    false,
                )
                .ok();
        }
    }
}

impl IntoElement for VideoElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}
