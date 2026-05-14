use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    CameraNode, CharacterNode, CircleNode, FaceJawNode, GradientDef, GradientStop, GraphScope,
    GraphScript, GroupNode, ImageNode, LineNode, MaskNode, PartNode, PassNode, PathNode,
    PolylineNode, RectNode, RepeatNode, SceneNode, ShadowNode, SvgNode, TextNode, eval_time_expr,
};
use base64::Engine;
use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};
use image::{Rgba, RgbaImage, imageops::FilterType};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MotionLoomSceneRenderError {
    #[error("MotionLoom scene render requires <Graph scope=\"scene\">.")]
    NotSceneGraph,
    #[error(
        "MotionLoom scene graph requires at least one scene node such as <Scene>, <Solid>, <Text>, <Image>, <Svg>, <Rect>, <Circle>, <Line>, <Polyline>, <Path>, <FaceJaw>, <Camera>, <Group>, <Mask>, or <Character>."
    )]
    EmptyScene,
    #[error("failed to read system time: {source}")]
    ReadTime { source: std::time::SystemTimeError },
    #[error("failed to create output directory ({path}): {source}")]
    CreateOutputDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to start ffmpeg: {source}")]
    StartFfmpeg { source: std::io::Error },
    #[error("ffmpeg stdin was not available")]
    MissingFfmpegStdin,
    #[error("failed to write raw frame to ffmpeg: {source}. ffmpeg stderr: {stderr}")]
    WriteFrame {
        source: std::io::Error,
        stderr: String,
    },
    #[error("failed to wait for ffmpeg: {source}")]
    WaitFfmpeg { source: std::io::Error },
    #[error("ffmpeg failed: {stderr}")]
    FfmpegFailed { stderr: String },
    #[error("invalid color '{value}'")]
    InvalidColor { value: String },
    #[error("invalid scene paint '{value}': {message}")]
    InvalidPaint { value: String, message: String },
    #[error("invalid scene expression '{expr}': {message}")]
    InvalidExpression { expr: String, message: String },
    #[error("invalid scene path data '{value}': {message}")]
    InvalidPathData { value: String, message: String },
    #[error("failed to open image asset ({path}): {source}")]
    OpenImage {
        path: PathBuf,
        source: image::ImageError,
    },
    #[error("failed to fetch media asset ({url}): {message}")]
    FetchAsset { url: String, message: String },
    #[error("failed to decode image asset ({source_ref}): {source}")]
    DecodeImage {
        source_ref: String,
        source: image::ImageError,
    },
    #[error("failed to read SVG asset ({path}): {source}")]
    ReadSvg {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse SVG asset ({source_ref}): {source}")]
    ParseSvg {
        source_ref: String,
        source: resvg::usvg::Error,
    },
    #[error("failed to render SVG asset ({source_ref}): invalid SVG size")]
    RenderSvg { source_ref: String },
    #[error("invalid SVG data URI ({source_ref}): {message}")]
    InvalidSvgDataUri { source_ref: String, message: String },
    #[error("GPU scene render failed: {message}")]
    GpuRender { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneRenderProfile {
    Cpu,
    Gpu,
    GpuProRes,
}

impl SceneRenderProfile {
    pub const fn output_extension(self) -> &'static str {
        match self {
            SceneRenderProfile::Cpu => "mov",
            SceneRenderProfile::Gpu => "mp4",
            SceneRenderProfile::GpuProRes => "mov",
        }
    }

    pub const fn output_prefix(self) -> &'static str {
        match self {
            SceneRenderProfile::Cpu => "motionloom_scene",
            SceneRenderProfile::Gpu => "motionloom_scene_gpu",
            SceneRenderProfile::GpuProRes => "motionloom_scene_gpu_prores",
        }
    }

    pub const fn uses_gpu_compositor(self) -> bool {
        matches!(
            self,
            SceneRenderProfile::Gpu | SceneRenderProfile::GpuProRes
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SceneRenderProgress {
    pub rendered_frames: u32,
    pub total_frames: u32,
}

#[allow(dead_code)]
pub fn next_scene_output_path(output_dir: &Path) -> Result<PathBuf, MotionLoomSceneRenderError> {
    next_scene_output_path_for_profile(output_dir, SceneRenderProfile::Cpu)
}

pub fn next_scene_output_path_for_profile(
    output_dir: &Path,
    profile: SceneRenderProfile,
) -> Result<PathBuf, MotionLoomSceneRenderError> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| MotionLoomSceneRenderError::ReadTime { source })?
        .as_millis();
    Ok(output_dir.join(format!(
        "{}_{}.{}",
        profile.output_prefix(),
        stamp,
        profile.output_extension()
    )))
}

#[allow(dead_code)]
pub fn render_scene_graph_to_video(
    ffmpeg_bin: &str,
    graph: &GraphScript,
    output_path: &Path,
) -> Result<(), MotionLoomSceneRenderError> {
    render_scene_graph_to_video_with_progress(
        ffmpeg_bin,
        graph,
        output_path,
        SceneRenderProfile::Cpu,
        0,
        |_progress| {},
    )
}

pub fn render_scene_graph_to_video_with_progress<F>(
    ffmpeg_bin: &str,
    graph: &GraphScript,
    output_path: &Path,
    profile: SceneRenderProfile,
    progress_every_frames: u32,
    mut progress_callback: F,
) -> Result<(), MotionLoomSceneRenderError>
where
    F: FnMut(SceneRenderProgress),
{
    validate_scene_graph(graph)?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            MotionLoomSceneRenderError::CreateOutputDir {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }

    let (w, h) = graph_output_size(graph);
    let fps = graph.fps.max(1.0);
    let duration_sec = (graph.duration_ms as f32 / 1000.0).max(1.0 / fps);
    let total_frames = ((duration_sec * fps).round() as u32).max(1);
    let size_arg = format!("{}x{}", w.max(1), h.max(1));
    let fps_arg = format!("{fps:.6}");
    let output_arg = output_path.to_string_lossy().to_string();
    let encoder_args = scene_encoder_args(profile);
    let mut child = Command::new(ffmpeg_bin)
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &size_arg,
            "-r",
            &fps_arg,
            "-i",
            "pipe:0",
            "-an",
        ])
        .args(&encoder_args)
        .arg(output_arg.as_str())
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| MotionLoomSceneRenderError::StartFfmpeg { source })?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or(MotionLoomSceneRenderError::MissingFfmpegStdin)?;
    let mut renderer = SceneFrameRenderer::new_for_profile(profile);
    progress_callback(SceneRenderProgress {
        rendered_frames: 0,
        total_frames,
    });
    for frame in 0..total_frames {
        let image = renderer.render_frame(graph, frame)?;
        if let Err(source) = stdin.write_all(image.as_raw()) {
            drop(stdin);
            let stderr = child
                .wait_with_output()
                .ok()
                .map(|output| String::from_utf8_lossy(&output.stderr).trim().to_string())
                .unwrap_or_else(|| "unable to collect ffmpeg stderr".to_string());
            return Err(MotionLoomSceneRenderError::WriteFrame { source, stderr });
        }
        let rendered_frames = frame + 1;
        if rendered_frames == total_frames
            || (progress_every_frames > 0 && rendered_frames % progress_every_frames == 0)
        {
            progress_callback(SceneRenderProgress {
                rendered_frames,
                total_frames,
            });
        }
    }
    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|source| MotionLoomSceneRenderError::WaitFfmpeg { source })?;
    if !output.status.success() {
        return Err(MotionLoomSceneRenderError::FfmpegFailed {
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(())
}

pub fn render_scene_graph_frame(
    graph: &GraphScript,
    frame: u32,
    profile: SceneRenderProfile,
) -> Result<RgbaImage, MotionLoomSceneRenderError> {
    validate_scene_graph(graph)?;
    let mut renderer = SceneFrameRenderer::new_for_profile(profile);
    renderer.render_frame(graph, frame)
}

pub type SceneRenderError = MotionLoomSceneRenderError;

pub fn render_scene_frame(
    graph: &GraphScript,
    frame: u32,
    profile: SceneRenderProfile,
) -> Result<RgbaImage, SceneRenderError> {
    render_scene_graph_frame(graph, frame, profile)
}

pub struct SceneRenderer {
    inner: SceneFrameRenderer,
}

impl SceneRenderer {
    pub fn new(profile: SceneRenderProfile) -> Result<Self, SceneRenderError> {
        Ok(Self {
            inner: SceneFrameRenderer::new_for_profile(profile),
        })
    }

    pub fn render_frame(
        &mut self,
        graph: &GraphScript,
        frame: u32,
    ) -> Result<RgbaImage, SceneRenderError> {
        validate_scene_graph(graph)?;
        self.inner.render_frame(graph, frame)
    }
}

fn scene_encoder_args(profile: SceneRenderProfile) -> Vec<String> {
    match profile {
        SceneRenderProfile::Cpu => prores_encoder_args(),
        SceneRenderProfile::Gpu => gpu_h264_encoder_args(),
        SceneRenderProfile::GpuProRes => prores_encoder_args(),
    }
}

fn prores_encoder_args() -> Vec<String> {
    // Keep scene output on an LGPL-safe, GStreamer-friendly path.
    // The app's curated preview runtime does not ship libav, so mp4v/mpeg4
    // decodes poorly there. ProRes MOV is larger but avoids GPL encoders.
    // Use ProRes HQ instead of Proxy: flat anime colors plus fine strokes show
    // visible chroma/luma waves after low-bitrate mezzanine compression.
    vec![
        "-vf".to_string(),
        "format=yuv422p10le".to_string(),
        "-c:v".to_string(),
        "prores_ks".to_string(),
        "-profile:v".to_string(),
        "3".to_string(),
        "-vendor".to_string(),
        "apl0".to_string(),
        "-pix_fmt".to_string(),
        "yuv422p10le".to_string(),
        "-color_primaries".to_string(),
        "bt709".to_string(),
        "-color_trc".to_string(),
        "bt709".to_string(),
        "-colorspace".to_string(),
        "bt709".to_string(),
    ]
}

#[cfg(target_os = "macos")]
fn gpu_h264_encoder_args() -> Vec<String> {
    vec![
        "-c:v".to_string(),
        "h264_videotoolbox".to_string(),
        "-allow_sw".to_string(),
        "1".to_string(),
        "-profile:v".to_string(),
        "high".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-b:v".to_string(),
        "30M".to_string(),
        "-maxrate".to_string(),
        "45M".to_string(),
        "-bufsize".to_string(),
        "90M".to_string(),
        "-color_primaries".to_string(),
        "bt709".to_string(),
        "-color_trc".to_string(),
        "bt709".to_string(),
        "-colorspace".to_string(),
        "bt709".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
    ]
}

#[cfg(not(target_os = "macos"))]
fn gpu_h264_encoder_args() -> Vec<String> {
    vec![
        "-c:v".to_string(),
        "libx264".to_string(),
        "-preset".to_string(),
        "medium".to_string(),
        "-crf".to_string(),
        "16".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-color_primaries".to_string(),
        "bt709".to_string(),
        "-color_trc".to_string(),
        "bt709".to_string(),
        "-colorspace".to_string(),
        "bt709".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
    ]
}

fn validate_scene_graph(graph: &GraphScript) -> Result<(), MotionLoomSceneRenderError> {
    if graph.scope != GraphScope::Scene {
        return Err(MotionLoomSceneRenderError::NotSceneGraph);
    }
    if !graph.has_scene_nodes() {
        return Err(MotionLoomSceneRenderError::EmptyScene);
    }
    Ok(())
}

const WGPU_SCENE_SHADER: &str = r#"
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
"#;

const WGPU_AFFINE_TEXTURE_SHADER: &str = r#"
struct TextureParams {
    canvas: vec4<f32>,
    bounds: vec4<f32>,
    image: vec4<f32>,
    opacity: vec4<f32>,
    inv0: vec4<f32>,
    inv1: vec4<f32>,
};

@group(0) @binding(0) var base_tex: texture_2d<f32>;
@group(0) @binding(1) var image_tex: texture_2d<f32>;
@group(0) @binding(2) var image_sampler: sampler;
@group(0) @binding(3) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(4) var<uniform> params: TextureParams;

fn over(base: vec4<f32>, src: vec4<f32>) -> vec4<f32> {
    let src_a = clamp(src.a * params.opacity.x, 0.0, 1.0);
    let dst_a = base.a;
    let out_a = src_a + dst_a * (1.0 - src_a);
    if (out_a <= 0.000001) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let rgb = (src.rgb * src_a + base.rgb * dst_a * (1.0 - src_a)) / out_a;
    return vec4<f32>(rgb, out_a);
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
    let local = vec2<f32>(
        params.inv0.x * px + params.inv0.y * py + params.inv0.z,
        params.inv1.x * px + params.inv1.y * py + params.inv1.z
    );

    let pos = vec2<i32>(i32(px_u), i32(py_u));
    let base = textureLoad(base_tex, pos, 0);
    var out_color = base;

    if (local.x >= 0.0 && local.y >= 0.0 && local.x < params.image.x && local.y < params.image.y) {
        let uv = vec2<f32>(local.x / params.image.x, local.y / params.image.y);
        let src = textureSampleLevel(image_tex, image_sampler, uv, 0.0);
        out_color = over(base, src);
    }

    textureStore(out_tex, pos, out_color);
}
"#;

#[allow(dead_code)]
const WGPU_SHAPE_SHADER: &str = r#"
struct ShapeParams {
    canvas: vec4<f32>,
    bounds: vec4<f32>,
    shape: vec4<f32>,
    style: vec4<f32>,
    color: vec4<f32>,
    inv0: vec4<f32>,
    inv1: vec4<f32>,
    paint: vec4<f32>,
    paint_bounds: vec4<f32>,
    gradient: vec4<f32>,
    stop_offsets0: vec4<f32>,
    stop_offsets1: vec4<f32>,
    stop_color0: vec4<f32>,
    stop_color1: vec4<f32>,
    stop_color2: vec4<f32>,
    stop_color3: vec4<f32>,
    stop_color4: vec4<f32>,
    stop_color5: vec4<f32>,
    stop_color6: vec4<f32>,
    stop_color7: vec4<f32>,
    line: vec4<f32>,
};

@group(0) @binding(0) var base_tex: texture_2d<f32>;
@group(0) @binding(1) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: ShapeParams;

fn rounded_rect_sdf(p: vec2<f32>, rect: vec4<f32>, radius: f32) -> f32 {
    let half_size = max(rect.zw * 0.5, vec2<f32>(0.0001, 0.0001));
    let center = rect.xy + half_size;
    let r = clamp(radius, 0.0, min(half_size.x, half_size.y));
    let q = abs(p - center) - half_size + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let h = clamp(dot(p - a, ab) / max(dot(ab, ab), 0.0001), 0.0, 1.0);
    return length(p - (a + ab * h));
}

fn cross2(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

fn triangle_coverage(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> f32 {
    let area = cross2(b - a, c - a);
    if (abs(area) <= 0.0001) {
        return 0.0;
    }
    let e0 = cross2(b - a, p - a);
    let e1 = cross2(c - b, p - b);
    let e2 = cross2(a - c, p - c);
    var same_side = false;
    if (area > 0.0) {
        same_side = e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0;
    } else {
        same_side = e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0;
    }
    if (!same_side) {
        return 0.0;
    }
    // Filled paths are triangulated before reaching this shader. Applying AA to
    // every triangle edge makes internal triangulation seams visible as moire /
    // wave bands on flat anime fills. Keep triangle interiors hard-filled here;
    // visible contour quality should be handled by the path outline/stroke.
    return 1.0;
}

fn inside_coverage(dist: f32) -> f32 {
    return clamp(0.5 - dist, 0.0, 1.0);
}

fn over(base: vec4<f32>, src_rgb: vec3<f32>, src_a: f32) -> vec4<f32> {
    let a = clamp(src_a, 0.0, 1.0);
    let out_a = a + base.a * (1.0 - a);
    if (out_a <= 0.000001) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let rgb = (src_rgb * a + base.rgb * base.a * (1.0 - a)) / out_a;
    return vec4<f32>(rgb, out_a);
}

fn stop_offset(index: i32) -> f32 {
    if (index == 0) { return params.stop_offsets0.x; }
    if (index == 1) { return params.stop_offsets0.y; }
    if (index == 2) { return params.stop_offsets0.z; }
    if (index == 3) { return params.stop_offsets0.w; }
    if (index == 4) { return params.stop_offsets1.x; }
    if (index == 5) { return params.stop_offsets1.y; }
    if (index == 6) { return params.stop_offsets1.z; }
    return params.stop_offsets1.w;
}

fn stop_color(index: i32) -> vec4<f32> {
    if (index == 0) { return params.stop_color0; }
    if (index == 1) { return params.stop_color1; }
    if (index == 2) { return params.stop_color2; }
    if (index == 3) { return params.stop_color3; }
    if (index == 4) { return params.stop_color4; }
    if (index == 5) { return params.stop_color5; }
    if (index == 6) { return params.stop_color6; }
    return params.stop_color7;
}

fn sample_gradient_stops(t_in: f32) -> vec4<f32> {
    let count = i32(clamp(round(params.paint.z), 0.0, 8.0));
    if (count <= 0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let t = clamp(t_in, 0.0, 1.0);
    if (t <= stop_offset(0)) {
        return stop_color(0);
    }
    for (var i = 1; i < 8; i = i + 1) {
        if (i < count && t <= stop_offset(i)) {
            let a_off = stop_offset(i - 1);
            let b_off = stop_offset(i);
            let span = max(b_off - a_off, 0.000001);
            let local_t = clamp((t - a_off) / span, 0.0, 1.0);
            let a = stop_color(i - 1);
            let b = stop_color(i);
            return a + (b - a) * local_t;
        }
    }
    return stop_color(max(count - 1, 0));
}

fn sample_shape_paint(local: vec2<f32>) -> vec4<f32> {
    let paint_kind = i32(round(params.paint.x));
    if (paint_kind == 0) {
        return params.color;
    }

    let units = i32(round(params.paint.y));
    let min_p = params.paint_bounds.xy;
    let max_p = params.paint_bounds.zw;
    let size = max(max_p - min_p, vec2<f32>(0.0001, 0.0001));
    var p = local;
    if (units == 0) {
        p = (local - min_p) / size;
    }

    if (paint_kind == 1) {
        let start = params.gradient.xy;
        let end = params.gradient.zw;
        let dir = end - start;
        let len2 = max(dot(dir, dir), 0.000001);
        return sample_gradient_stops(dot(p - start, dir) / len2);
    }

    if (paint_kind == 2) {
        let center = params.gradient.xy;
        var delta = p - center;
        if (units == 0) {
            let aspect = select(1.0, size.y / size.x, size.x > size.y);
            delta.x = delta.x / max(aspect, 0.0001);
        }
        return sample_gradient_stops(length(delta) / max(params.gradient.z, 0.0001));
    }

    return params.color;
}

fn line_taper_pressure(t: f32) -> f32 {
    var pressure = 1.0;
    if (params.line.z > 0.0001) {
        pressure = min(pressure, clamp(t / params.line.z, 0.0, 1.0));
    }
    if (params.line.w > 0.0001) {
        pressure = min(pressure, clamp((1.0 - t) / params.line.w, 0.0, 1.0));
    }
    return pressure;
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

    let pos = vec2<i32>(i32(px_u), i32(py_u));
    let base = textureLoad(base_tex, pos, 0);
    let px = f32(px_u) + 0.5;
    let py = f32(py_u) + 0.5;
    let local = vec2<f32>(
        params.inv0.x * px + params.inv0.y * py + params.inv0.z,
        params.inv1.x * px + params.inv1.y * py + params.inv1.z
    );

    let shape_kind = i32(round(params.canvas.z));
    var coverage = 0.0;
    var replace = false;

    if (shape_kind == 7) {
        coverage = 1.0;
        replace = true;
    } else if (shape_kind == 1) {
        let dist = rounded_rect_sdf(local, params.shape, params.style.x);
        coverage = inside_coverage(dist);
    } else if (shape_kind == 2) {
        let outer = rounded_rect_sdf(local, params.shape, params.style.x);
        let sw = max(params.style.y, 0.0);
        let inner = rounded_rect_sdf(
            local,
            vec4<f32>(params.shape.x + sw, params.shape.y + sw, max(params.shape.z - sw * 2.0, 0.0), max(params.shape.w - sw * 2.0, 0.0)),
            max(params.style.x - sw, 0.0)
        );
        coverage = inside_coverage(outer) * (1.0 - inside_coverage(inner));
    } else if (shape_kind == 3) {
        let dist = length(local - params.shape.xy) - params.shape.z;
        coverage = inside_coverage(dist);
    } else if (shape_kind == 4) {
        let dist = length(local - params.shape.xy) - params.shape.z;
        let sw = max(params.style.y, 0.0);
        coverage = inside_coverage(dist) * (1.0 - inside_coverage(dist + sw));
    } else if (shape_kind == 5) {
        let dist = rounded_rect_sdf(local, params.shape, params.style.x);
        let outside = max(dist, 0.0);
        let blur = max(params.style.z, 1.0);
        let sigma = max(blur * 0.42, 1.0);
        let inside = smoothstep(0.0, blur * 0.7, max(-dist, 0.0));
        let outside_falloff = exp(-(outside * outside) / (2.0 * sigma * sigma));
        coverage = min(max(inside, outside_falloff), 0.86);
    } else if (shape_kind == 6) {
        let dist = length(local - params.shape.xy) - params.shape.z;
        let outside = max(dist, 0.0);
        let blur = max(params.style.z, 1.0);
        let sigma = max(blur * 0.42, 1.0);
        let inside = smoothstep(0.0, blur * 0.7, max(-dist, 0.0));
        let outside_falloff = exp(-(outside * outside) / (2.0 * sigma * sigma));
        coverage = min(max(inside, outside_falloff), 0.86);
    } else if (shape_kind == 8) {
        let ab = params.shape.zw - params.shape.xy;
        let h = clamp(dot(local - params.shape.xy, ab) / max(dot(ab, ab), 0.0001), 0.0, 1.0);
        let global_t = params.line.x + (params.line.y - params.line.x) * h;
        let pressure = line_taper_pressure(global_t);
        let half_width = max(params.style.y * pressure * 0.5, 0.01);
        let dist = segment_distance(local, params.shape.xy, params.shape.zw) - half_width;
        coverage = inside_coverage(dist);
    } else if (shape_kind == 9) {
        coverage = triangle_coverage(local, params.shape.xy, params.shape.zw, params.style.xy);
    }

    coverage = clamp(coverage, 0.0, 1.0);
    let paint_color = sample_shape_paint(local);
    let src_a = paint_color.a * params.style.w * coverage;
    let out_color = select(over(base, paint_color.rgb, src_a), vec4<f32>(paint_color.rgb, paint_color.a * params.style.w), replace);
    textureStore(out_tex, pos, out_color);
}
"#;

const WGPU_BATCH_SHAPE_SHADER: &str = r#"
struct BatchParams {
    canvas: vec4<f32>,
    count: vec4<f32>,
};

struct Primitive {
    info: vec4<f32>,
    bounds: vec4<f32>,
    shape: vec4<f32>,
    style: vec4<f32>,
    color: vec4<f32>,
    inv0: vec4<f32>,
    inv1: vec4<f32>,
    paint: vec4<f32>,
    paint_bounds: vec4<f32>,
    gradient: vec4<f32>,
    stop_offsets0: vec4<f32>,
    stop_offsets1: vec4<f32>,
    stop_color0: vec4<f32>,
    stop_color1: vec4<f32>,
    stop_color2: vec4<f32>,
    stop_color3: vec4<f32>,
    stop_color4: vec4<f32>,
    stop_color5: vec4<f32>,
    stop_color6: vec4<f32>,
    stop_color7: vec4<f32>,
    line: vec4<f32>,
};

struct PrimitiveBuffer {
    items: array<Primitive>,
};

struct TileRangeBuffer {
    items: array<vec4<u32>>,
};

struct TileIndexBuffer {
    items: array<u32>,
};

@group(0) @binding(0) var base_tex: texture_2d<f32>;
@group(0) @binding(1) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: BatchParams;
@group(0) @binding(3) var<storage, read> primitive_buffer: PrimitiveBuffer;
@group(0) @binding(4) var<storage, read> tile_range_buffer: TileRangeBuffer;
@group(0) @binding(5) var<storage, read> tile_index_buffer: TileIndexBuffer;

fn rounded_rect_sdf(p: vec2<f32>, rect: vec4<f32>, radius: f32) -> f32 {
    let half_size = max(rect.zw * 0.5, vec2<f32>(0.0001, 0.0001));
    let center = rect.xy + half_size;
    let r = clamp(radius, 0.0, min(half_size.x, half_size.y));
    let q = abs(p - center) - half_size + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let h = clamp(dot(p - a, ab) / max(dot(ab, ab), 0.0001), 0.0, 1.0);
    return length(p - (a + ab * h));
}

fn cross2(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

fn triangle_coverage(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> f32 {
    let area = cross2(b - a, c - a);
    if (abs(area) <= 0.0001) {
        return 0.0;
    }
    let e0 = cross2(b - a, p - a);
    let e1 = cross2(c - b, p - b);
    let e2 = cross2(a - c, p - c);
    var same_side = false;
    if (area > 0.0) {
        same_side = e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0;
    } else {
        same_side = e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0;
    }
    if (!same_side) {
        return 0.0;
    }
    // Filled paths are triangulated before reaching this shader. Applying AA to
    // every triangle edge makes internal triangulation seams visible as moire /
    // wave bands on flat anime fills. Keep triangle interiors hard-filled here;
    // visible contour quality should be handled by the path outline/stroke.
    return 1.0;
}

fn inside_coverage(dist: f32) -> f32 {
    return clamp(0.5 - dist, 0.0, 1.0);
}

fn over(base: vec4<f32>, src_rgb: vec3<f32>, src_a: f32) -> vec4<f32> {
    let a = clamp(src_a, 0.0, 1.0);
    let out_a = a + base.a * (1.0 - a);
    if (out_a <= 0.000001) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let rgb = (src_rgb * a + base.rgb * base.a * (1.0 - a)) / out_a;
    return vec4<f32>(rgb, out_a);
}

fn stop_offset(p: Primitive, index: i32) -> f32 {
    if (index == 0) { return p.stop_offsets0.x; }
    if (index == 1) { return p.stop_offsets0.y; }
    if (index == 2) { return p.stop_offsets0.z; }
    if (index == 3) { return p.stop_offsets0.w; }
    if (index == 4) { return p.stop_offsets1.x; }
    if (index == 5) { return p.stop_offsets1.y; }
    if (index == 6) { return p.stop_offsets1.z; }
    return p.stop_offsets1.w;
}

fn stop_color(p: Primitive, index: i32) -> vec4<f32> {
    if (index == 0) { return p.stop_color0; }
    if (index == 1) { return p.stop_color1; }
    if (index == 2) { return p.stop_color2; }
    if (index == 3) { return p.stop_color3; }
    if (index == 4) { return p.stop_color4; }
    if (index == 5) { return p.stop_color5; }
    if (index == 6) { return p.stop_color6; }
    return p.stop_color7;
}

fn sample_gradient_stops(p: Primitive, t_in: f32) -> vec4<f32> {
    let count = i32(clamp(round(p.paint.z), 0.0, 8.0));
    if (count <= 0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let t = clamp(t_in, 0.0, 1.0);
    if (t <= stop_offset(p, 0)) {
        return stop_color(p, 0);
    }
    for (var i = 1; i < 8; i = i + 1) {
        if (i < count && t <= stop_offset(p, i)) {
            let a_off = stop_offset(p, i - 1);
            let b_off = stop_offset(p, i);
            let span = max(b_off - a_off, 0.000001);
            let local_t = clamp((t - a_off) / span, 0.0, 1.0);
            let a = stop_color(p, i - 1);
            let b = stop_color(p, i);
            return a + (b - a) * local_t;
        }
    }
    return stop_color(p, max(count - 1, 0));
}

fn sample_shape_paint(p: Primitive, local: vec2<f32>) -> vec4<f32> {
    let paint_kind = i32(round(p.paint.x));
    if (paint_kind == 0) {
        return p.color;
    }

    let units = i32(round(p.paint.y));
    let min_p = p.paint_bounds.xy;
    let max_p = p.paint_bounds.zw;
    let size = max(max_p - min_p, vec2<f32>(0.0001, 0.0001));
    var sample_p = local;
    if (units == 0) {
        sample_p = (local - min_p) / size;
    }

    if (paint_kind == 1) {
        let start = p.gradient.xy;
        let end = p.gradient.zw;
        let dir = end - start;
        let len2 = max(dot(dir, dir), 0.000001);
        return sample_gradient_stops(p, dot(sample_p - start, dir) / len2);
    }

    if (paint_kind == 2) {
        let center = p.gradient.xy;
        var delta = sample_p - center;
        if (units == 0) {
            let aspect = select(1.0, size.y / size.x, size.x > size.y);
            delta.x = delta.x / max(aspect, 0.0001);
        }
        return sample_gradient_stops(p, length(delta) / max(p.gradient.z, 0.0001));
    }

    return p.color;
}

fn line_taper_pressure(p: Primitive, t: f32) -> f32 {
    var pressure = 1.0;
    if (p.line.z > 0.0001) {
        pressure = min(pressure, clamp(t / p.line.z, 0.0, 1.0));
    }
    if (p.line.w > 0.0001) {
        pressure = min(pressure, clamp((1.0 - t) / p.line.w, 0.0, 1.0));
    }
    return pressure;
}

fn primitive_coverage(p: Primitive, local: vec2<f32>) -> vec2<f32> {
    let shape_kind = i32(round(p.info.x));
    var coverage = 0.0;
    var replace = 0.0;

    if (shape_kind == 7) {
        coverage = 1.0;
        replace = 1.0;
    } else if (shape_kind == 1) {
        let dist = rounded_rect_sdf(local, p.shape, p.style.x);
        coverage = inside_coverage(dist);
    } else if (shape_kind == 2) {
        let outer = rounded_rect_sdf(local, p.shape, p.style.x);
        let sw = max(p.style.y, 0.0);
        let inner = rounded_rect_sdf(
            local,
            vec4<f32>(p.shape.x + sw, p.shape.y + sw, max(p.shape.z - sw * 2.0, 0.0), max(p.shape.w - sw * 2.0, 0.0)),
            max(p.style.x - sw, 0.0)
        );
        coverage = inside_coverage(outer) * (1.0 - inside_coverage(inner));
    } else if (shape_kind == 3) {
        let dist = length(local - p.shape.xy) - p.shape.z;
        coverage = inside_coverage(dist);
    } else if (shape_kind == 4) {
        let dist = length(local - p.shape.xy) - p.shape.z;
        let sw = max(p.style.y, 0.0);
        coverage = inside_coverage(dist) * (1.0 - inside_coverage(dist + sw));
    } else if (shape_kind == 5) {
        let dist = rounded_rect_sdf(local, p.shape, p.style.x);
        let outside = max(dist, 0.0);
        let blur = max(p.style.z, 1.0);
        let sigma = max(blur * 0.42, 1.0);
        let inside = smoothstep(0.0, blur * 0.7, max(-dist, 0.0));
        let outside_falloff = exp(-(outside * outside) / (2.0 * sigma * sigma));
        coverage = min(max(inside, outside_falloff), 0.86);
    } else if (shape_kind == 6) {
        let dist = length(local - p.shape.xy) - p.shape.z;
        let outside = max(dist, 0.0);
        let blur = max(p.style.z, 1.0);
        let sigma = max(blur * 0.42, 1.0);
        let inside = smoothstep(0.0, blur * 0.7, max(-dist, 0.0));
        let outside_falloff = exp(-(outside * outside) / (2.0 * sigma * sigma));
        coverage = min(max(inside, outside_falloff), 0.86);
    } else if (shape_kind == 8) {
        let ab = p.shape.zw - p.shape.xy;
        let h = clamp(dot(local - p.shape.xy, ab) / max(dot(ab, ab), 0.0001), 0.0, 1.0);
        let global_t = p.line.x + (p.line.y - p.line.x) * h;
        let pressure = line_taper_pressure(p, global_t);
        let half_width = max(p.style.y * pressure * 0.5, 0.01);
        let dist = segment_distance(local, p.shape.xy, p.shape.zw) - half_width;
        coverage = inside_coverage(dist);
    } else if (shape_kind == 9) {
        coverage = triangle_coverage(local, p.shape.xy, p.shape.zw, p.style.xy);
    }

    return vec2<f32>(clamp(coverage, 0.0, 1.0), replace);
}

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= u32(params.canvas.x) || y >= u32(params.canvas.y)) {
        return;
    }

    let pos = vec2<i32>(i32(x), i32(y));
    let px = f32(x) + 0.5;
    let py = f32(y) + 0.5;
    var out_color = textureLoad(base_tex, pos, 0);
    let primitive_count = u32(round(params.count.x));
    let tile_size = max(u32(round(params.count.y)), 1u);
    let tiles_x = max(u32(round(params.count.z)), 1u);
    let tiles_y = max(u32(round(params.count.w)), 1u);
    let tile_x = min(x / tile_size, tiles_x - 1u);
    let tile_y = min(y / tile_size, tiles_y - 1u);
    let tile = tile_range_buffer.items[tile_y * tiles_x + tile_x];
    let tile_start = tile.x;
    let tile_count = tile.y;

    for (var local_i = 0u; local_i < tile_count; local_i = local_i + 1u) {
        let primitive_index = tile_index_buffer.items[tile_start + local_i];
        if (primitive_index >= primitive_count) {
            continue;
        }
        let p = primitive_buffer.items[primitive_index];
        if (px < p.bounds.x || py < p.bounds.y || px >= p.bounds.x + p.bounds.z || py >= p.bounds.y + p.bounds.w) {
            continue;
        }
        let local = vec2<f32>(
            p.inv0.x * px + p.inv0.y * py + p.inv0.z,
            p.inv1.x * px + p.inv1.y * py + p.inv1.z
        );
        let cover_replace = primitive_coverage(p, local);
        let coverage = cover_replace.x;
        if (coverage <= 0.0) {
            continue;
        }
        let paint_color = sample_shape_paint(p, local);
        if (cover_replace.y > 0.5) {
            out_color = vec4<f32>(paint_color.rgb, paint_color.a * p.style.w);
        } else {
            out_color = over(out_color, paint_color.rgb, paint_color.a * p.style.w * coverage);
        }
    }

    textureStore(out_tex, pos, out_color);
}
"#;

const WGPU_POST_SHADER: &str = r#"
struct PostParams {
    canvas: vec4<f32>,
    params: vec4<f32>,
};

@group(0) @binding(0) var base_tex: texture_2d<f32>;
@group(0) @binding(1) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: PostParams;

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= u32(params.canvas.x) || y >= u32(params.canvas.y)) {
        return;
    }

    let axis = i32(round(params.params.x));
    let radius = i32(round(clamp(params.params.y, 0.0, 64.0)));
    var acc = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var weight_sum = 0.0;

    for (var i = -64; i <= 64; i = i + 1) {
        if (abs(i) <= radius) {
            var sx = i32(x);
            var sy = i32(y);
            if (axis == 0) {
                sx = clamp(i32(x) + i, 0, i32(params.canvas.x) - 1);
            } else {
                sy = clamp(i32(y) + i, 0, i32(params.canvas.y) - 1);
            }
            let dist = f32(i) / max(f32(radius), 1.0);
            let weight = exp(-dist * dist * 2.5);
            acc = acc + textureLoad(base_tex, vec2<i32>(sx, sy), 0) * weight;
            weight_sum = weight_sum + weight;
        }
    }

    textureStore(out_tex, vec2<i32>(i32(x), i32(y)), acc / max(weight_sum, 0.0001));
}
"#;

struct WgpuImageTexture {
    width: u32,
    height: u32,
    texture: std::sync::Arc<wgpu::Texture>,
}

struct WgpuSceneCompositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    texture_pipeline: wgpu::ComputePipeline,
    shape_bind_group_layout: wgpu::BindGroupLayout,
    shape_pipeline: wgpu::ComputePipeline,
    post_bind_group_layout: wgpu::BindGroupLayout,
    post_pipeline: wgpu::ComputePipeline,
    sampler: wgpu::Sampler,
    width: u32,
    height: u32,
    tex_a: wgpu::Texture,
    tex_b: wgpu::Texture,
    readback_buffer: wgpu::Buffer,
    padded_bytes_per_row: u32,
    image_textures: HashMap<String, WgpuImageTexture>,
}

impl WgpuSceneCompositor {
    fn new(width: u32, height: u32) -> Result<Self, MotionLoomSceneRenderError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .map_err(|_| MotionLoomSceneRenderError::GpuRender {
            message: "no high-performance GPU adapter was available".to_string(),
        })?;
        let adapter_limits = adapter.limits();
        let max_texture_dimension_2d = adapter_limits.max_texture_dimension_2d;
        if width > max_texture_dimension_2d || height > max_texture_dimension_2d {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: format!(
                    "requested scene render size {}x{} exceeds GPU max 2D texture dimension {}",
                    width, height, max_texture_dimension_2d
                ),
            });
        }

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("anica-motionloom-scene-gpu-device"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter_limits,
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|err| MotionLoomSceneRenderError::GpuRender {
            message: format!("device request failed: {err}"),
        })?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("anica-motionloom-scene-gpu-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(WGPU_SCENE_SHADER)),
        });
        let texture_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("anica-motionloom-scene-affine-texture-gpu-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                WGPU_AFFINE_TEXTURE_SHADER,
            )),
        });
        let shape_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("anica-motionloom-scene-shape-gpu-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(WGPU_BATCH_SHAPE_SHADER)),
        });
        let post_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("anica-motionloom-scene-post-gpu-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(WGPU_POST_SHADER)),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("anica-motionloom-scene-gpu-bgl"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shape_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("anica-motionloom-scene-shape-gpu-bgl"),
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
                            ty: wgpu::BufferBindingType::Uniform,
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
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let post_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("anica-motionloom-scene-post-gpu-bgl"),
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
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("anica-motionloom-scene-gpu-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("anica-motionloom-scene-gpu-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let texture_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("anica-motionloom-scene-affine-texture-gpu-pipeline"),
            layout: Some(&pipeline_layout),
            module: &texture_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let shape_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("anica-motionloom-scene-shape-gpu-pipeline-layout"),
                bind_group_layouts: &[&shape_bind_group_layout],
                push_constant_ranges: &[],
            });
        let shape_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("anica-motionloom-scene-shape-gpu-pipeline"),
            layout: Some(&shape_pipeline_layout),
            module: &shape_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let post_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("anica-motionloom-scene-post-gpu-pipeline-layout"),
            bind_group_layouts: &[&post_bind_group_layout],
            push_constant_ranges: &[],
        });
        let post_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("anica-motionloom-scene-post-gpu-pipeline"),
            layout: Some(&post_pipeline_layout),
            module: &post_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("anica-motionloom-scene-gpu-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let tex_a = Self::make_canvas_texture(&device, width, height);
        let tex_b = Self::make_canvas_texture(&device, width, height);
        let padded_bytes_per_row = align_to_256(width.saturating_mul(4));
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-motionloom-scene-gpu-readback"),
            size: (padded_bytes_per_row as u64 * height as u64).max(4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Ok(Self {
            device,
            queue,
            bind_group_layout,
            pipeline,
            texture_pipeline,
            shape_bind_group_layout,
            shape_pipeline,
            post_bind_group_layout,
            post_pipeline,
            sampler,
            width,
            height,
            tex_a,
            tex_b,
            readback_buffer,
            padded_bytes_per_row,
            image_textures: HashMap::new(),
        })
    }

    fn make_canvas_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("anica-motionloom-scene-gpu-canvas"),
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
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn make_source_texture(&self, width: u32, height: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("anica-motionloom-scene-gpu-source"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn render(
        &mut self,
        graph: &GraphScript,
        solid: [u8; 4],
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let canvas_len = (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(4);
        let mut base = vec![0u8; canvas_len];
        for pixel in base.chunks_exact_mut(4) {
            pixel.copy_from_slice(&solid);
        }
        self.write_texture_rgba(&self.tex_a, self.width, self.height, &base);

        let mut current_is_a = true;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("anica-motionloom-scene-gpu-encoder"),
            });
        let mut uniform_buffers = Vec::with_capacity(graph.images.len() + graph.svgs.len());

        for image_node in &graph.images {
            let opacity =
                eval_scene_number(&image_node.opacity, time_norm, time_sec)?.clamp(0.0, 1.0);
            if opacity <= 0.0001 {
                continue;
            }

            let (source_w, source_h, source_texture) = self.load_image_texture(&image_node.src)?;
            let scale =
                eval_scene_number(&image_node.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
            let target_w = ((source_w as f32) * scale).round().max(1.0);
            let target_h = ((source_h as f32) * scale).round().max(1.0);
            let x_base = resolve_axis(
                &image_node.x,
                self.width as f32,
                target_w,
                time_norm,
                time_sec,
            )?;
            let y_base = resolve_axis(
                &image_node.y,
                self.height as f32,
                target_h,
                time_norm,
                time_sec,
            )?;

            let mut uniform = [0u8; 48];
            let values = [
                self.width as f32,
                self.height as f32,
                x_base,
                y_base,
                target_w,
                target_h,
                source_w as f32,
                source_h as f32,
                opacity,
                0.0,
                0.0,
                0.0,
            ];
            for (ix, value) in values.iter().enumerate() {
                uniform[ix * 4..ix * 4 + 4].copy_from_slice(&value.to_ne_bytes());
            }
            let uniform_buffer = self.make_uniform_buffer(&uniform);

            let (src_canvas, dst_canvas) = if current_is_a {
                (&self.tex_a, &self.tex_b)
            } else {
                (&self.tex_b, &self.tex_a)
            };
            self.dispatch_image_pass(
                &mut encoder,
                src_canvas,
                &source_texture,
                dst_canvas,
                &uniform_buffer,
            );
            uniform_buffers.push(uniform_buffer);
            current_is_a = !current_is_a;
        }

        for svg_node in &graph.svgs {
            let opacity =
                eval_scene_number(&svg_node.opacity, time_norm, time_sec)?.clamp(0.0, 1.0);
            if opacity <= 0.0001 {
                continue;
            }

            let (source_w, source_h, source_texture) = self.load_svg_texture(&svg_node.src)?;
            let scale = eval_scene_number(&svg_node.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
            let target_w = ((source_w as f32) * scale).round().max(1.0);
            let target_h = ((source_h as f32) * scale).round().max(1.0);
            let x_base = resolve_axis(
                &svg_node.x,
                self.width as f32,
                target_w,
                time_norm,
                time_sec,
            )?;
            let y_base = resolve_axis(
                &svg_node.y,
                self.height as f32,
                target_h,
                time_norm,
                time_sec,
            )?;

            let mut uniform = [0u8; 48];
            let values = [
                self.width as f32,
                self.height as f32,
                x_base,
                y_base,
                target_w,
                target_h,
                source_w as f32,
                source_h as f32,
                opacity,
                0.0,
                0.0,
                0.0,
            ];
            for (ix, value) in values.iter().enumerate() {
                uniform[ix * 4..ix * 4 + 4].copy_from_slice(&value.to_ne_bytes());
            }
            let uniform_buffer = self.make_uniform_buffer(&uniform);

            let (src_canvas, dst_canvas) = if current_is_a {
                (&self.tex_a, &self.tex_b)
            } else {
                (&self.tex_b, &self.tex_a)
            };
            self.dispatch_image_pass(
                &mut encoder,
                src_canvas,
                &source_texture,
                dst_canvas,
                &uniform_buffer,
            );
            uniform_buffers.push(uniform_buffer);
            current_is_a = !current_is_a;
        }

        let final_texture = if current_is_a {
            &self.tex_a
        } else {
            &self.tex_b
        };
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: final_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let rendered = self.readback_rgba();
        drop(uniform_buffers);
        rendered
    }

    fn render_scene_content(
        &mut self,
        primitives: &[GpuScenePrimitive],
        texture_layers: &[GpuSceneTextureLayer],
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let canvas_len = (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(4);
        let base = vec![0u8; canvas_len];
        self.write_texture_rgba(&self.tex_a, self.width, self.height, &base);
        self.write_texture_rgba(&self.tex_b, self.width, self.height, &base);

        let mut current_is_a = true;
        let mut dirty_a: Option<TextureRect> = None;
        let mut dirty_b: Option<TextureRect> = None;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("anica-motionloom-scene-shape-gpu-encoder"),
            });
        let mut uniform_buffers = Vec::with_capacity(texture_layers.len() + 2);
        let mut texture_sources = Vec::<wgpu::Texture>::with_capacity(texture_layers.len());

        let shape_batch = batch_shape_storage_bytes(primitives, self.width, self.height)?;
        if shape_batch.primitive_count > 0 {
            let uniform = batch_shape_uniform(
                self.width,
                self.height,
                shape_batch.primitive_count,
                shape_batch.tile_size,
                shape_batch.tiles_x,
                shape_batch.tiles_y,
            );
            let uniform_buffer = self.make_batch_shape_uniform_buffer(&uniform);
            let storage_buffer = self.make_storage_buffer(
                "anica-motionloom-scene-shape-gpu-storage",
                &shape_batch.primitive_bytes,
            );
            let tile_range_buffer = self.make_storage_buffer(
                "anica-motionloom-scene-shape-gpu-tile-ranges",
                &shape_batch.tile_range_bytes,
            );
            let tile_index_buffer = self.make_storage_buffer(
                "anica-motionloom-scene-shape-gpu-tile-indices",
                &shape_batch.tile_index_bytes,
            );
            self.dispatch_batched_shape_pass(
                &mut encoder,
                &self.tex_a,
                &self.tex_b,
                &uniform_buffer,
                &storage_buffer,
                &tile_range_buffer,
                &tile_index_buffer,
            );
            uniform_buffers.push(uniform_buffer);
            uniform_buffers.push(storage_buffer);
            uniform_buffers.push(tile_range_buffer);
            uniform_buffers.push(tile_index_buffer);
            current_is_a = false;
            dirty_a = Some(TextureRect {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            });
            dirty_b = None;
        }

        for layer in texture_layers {
            if layer.opacity <= 0.0001 || layer.image.width() == 0 || layer.image.height() == 0 {
                continue;
            }
            let Some((bounds_x, bounds_y, bounds_w, bounds_h)) = texture_layer_bounds(
                layer.transform,
                layer.image.width(),
                layer.image.height(),
                self.width,
                self.height,
            ) else {
                continue;
            };
            if bounds_w == 0 || bounds_h == 0 {
                continue;
            }

            let source_texture =
                self.make_source_texture(layer.image.width().max(1), layer.image.height().max(1));
            self.write_texture_rgba(
                &source_texture,
                layer.image.width().max(1),
                layer.image.height().max(1),
                layer.image.as_raw(),
            );
            let uniform = affine_texture_uniform(
                layer,
                self.width,
                self.height,
                bounds_x,
                bounds_y,
                bounds_w,
                bounds_h,
            )?;
            let uniform_buffer = self.make_texture_uniform_buffer(&uniform);
            let (src_canvas, dst_canvas) = if current_is_a {
                (&self.tex_a, &self.tex_b)
            } else {
                (&self.tex_b, &self.tex_a)
            };
            let dst_dirty = if current_is_a { dirty_b } else { dirty_a };
            if let Some(rect) = dst_dirty {
                self.copy_texture_rect(&mut encoder, src_canvas, dst_canvas, rect);
            }
            self.dispatch_affine_texture_pass(
                &mut encoder,
                src_canvas,
                &source_texture,
                dst_canvas,
                &uniform_buffer,
                bounds_w,
                bounds_h,
            );
            uniform_buffers.push(uniform_buffer);
            texture_sources.push(source_texture);
            let changed = TextureRect {
                x: bounds_x,
                y: bounds_y,
                width: bounds_w,
                height: bounds_h,
            };
            if current_is_a {
                dirty_b = None;
                dirty_a = union_texture_rect(dirty_a, changed);
            } else {
                dirty_a = None;
                dirty_b = union_texture_rect(dirty_b, changed);
            }
            current_is_a = !current_is_a;
        }

        let final_texture = if current_is_a {
            &self.tex_a
        } else {
            &self.tex_b
        };
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: final_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let rendered = self.readback_rgba();
        drop(uniform_buffers);
        drop(texture_sources);
        rendered
    }

    fn apply_gpu_blur_passes(
        &mut self,
        input: &RgbaImage,
        passes: &[(bool, f32)],
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        if passes.is_empty() {
            return Ok(input.clone());
        }
        if input.width() != self.width || input.height() != self.height {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: format!(
                    "post-pass input size {}x{} does not match GPU compositor {}x{}",
                    input.width(),
                    input.height(),
                    self.width,
                    self.height
                ),
            });
        }

        self.write_texture_rgba(&self.tex_a, self.width, self.height, input.as_raw());

        let mut current_is_a = true;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("anica-motionloom-scene-post-gpu-encoder"),
            });
        let mut uniform_buffers = Vec::with_capacity(passes.len());

        for (horizontal, sigma) in passes {
            let uniform = post_blur_uniform(self.width, self.height, *horizontal, *sigma);
            let uniform_buffer = self.make_post_uniform_buffer(&uniform);
            let (src_canvas, dst_canvas) = if current_is_a {
                (&self.tex_a, &self.tex_b)
            } else {
                (&self.tex_b, &self.tex_a)
            };
            self.dispatch_post_pass(&mut encoder, src_canvas, dst_canvas, &uniform_buffer);
            uniform_buffers.push(uniform_buffer);
            current_is_a = !current_is_a;
        }

        let final_texture = if current_is_a {
            &self.tex_a
        } else {
            &self.tex_b
        };
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: final_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let rendered = self.readback_rgba();
        drop(uniform_buffers);
        rendered
    }

    fn make_uniform_buffer(&self, uniform: &[u8; 48]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-motionloom-scene-gpu-uniform"),
            size: uniform.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .copy_from_slice(uniform);
        buffer.unmap();
        buffer
    }

    fn make_texture_uniform_buffer(&self, uniform: &[u8; 96]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-motionloom-scene-affine-texture-gpu-uniform"),
            size: uniform.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .copy_from_slice(uniform);
        buffer.unmap();
        buffer
    }

    fn make_post_uniform_buffer(&self, uniform: &[u8; 32]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-motionloom-scene-post-gpu-uniform"),
            size: uniform.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .copy_from_slice(uniform);
        buffer.unmap();
        buffer
    }

    fn make_batch_shape_uniform_buffer(&self, uniform: &[u8; 32]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-motionloom-scene-batch-shape-gpu-uniform"),
            size: uniform.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .copy_from_slice(uniform);
        buffer.unmap();
        buffer
    }

    fn make_storage_buffer(&self, label: &'static str, data: &[u8]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: data.len().max(4) as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        });
        buffer
            .slice(..data.len() as u64)
            .get_mapped_range_mut()
            .copy_from_slice(data);
        buffer.unmap();
        buffer
    }

    fn write_texture_rgba(&self, texture: &wgpu::Texture, width: u32, height: u32, rgba: &[u8]) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
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
    }

    fn copy_texture_rect(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        src_texture: &wgpu::Texture,
        dst_texture: &wgpu::Texture,
        rect: TextureRect,
    ) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: src_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: rect.x,
                    y: rect.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: dst_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: rect.x,
                    y: rect.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: rect.width,
                height: rect.height,
                depth_or_array_layers: 1,
            },
        );
    }

    fn dispatch_image_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        base_texture: &wgpu::Texture,
        image_texture: &wgpu::Texture,
        out_texture: &wgpu::Texture,
        uniform_buffer: &wgpu::Buffer,
    ) {
        let base_view = base_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let image_view = image_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let out_view = out_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anica-motionloom-scene-gpu-bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&base_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&image_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&out_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("anica-motionloom-scene-gpu-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(
            self.width.div_ceil(16).max(1),
            self.height.div_ceil(16).max(1),
            1,
        );
    }

    fn dispatch_batched_shape_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        base_texture: &wgpu::Texture,
        out_texture: &wgpu::Texture,
        uniform_buffer: &wgpu::Buffer,
        primitive_buffer: &wgpu::Buffer,
        tile_range_buffer: &wgpu::Buffer,
        tile_index_buffer: &wgpu::Buffer,
    ) {
        let base_view = base_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let out_view = out_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anica-motionloom-scene-shape-gpu-bg"),
            layout: &self.shape_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&base_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&out_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: primitive_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: tile_range_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: tile_index_buffer.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("anica-motionloom-scene-shape-gpu-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.shape_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(
            self.width.div_ceil(16).max(1),
            self.height.div_ceil(16).max(1),
            1,
        );
    }

    fn dispatch_affine_texture_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        base_texture: &wgpu::Texture,
        image_texture: &wgpu::Texture,
        out_texture: &wgpu::Texture,
        uniform_buffer: &wgpu::Buffer,
        bounds_w: u32,
        bounds_h: u32,
    ) {
        let base_view = base_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let image_view = image_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let out_view = out_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anica-motionloom-scene-affine-texture-gpu-bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&base_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&image_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&out_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("anica-motionloom-scene-affine-texture-gpu-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.texture_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(
            bounds_w.div_ceil(16).max(1),
            bounds_h.div_ceil(16).max(1),
            1,
        );
    }

    fn dispatch_post_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        base_texture: &wgpu::Texture,
        out_texture: &wgpu::Texture,
        uniform_buffer: &wgpu::Buffer,
    ) {
        let base_view = base_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let out_view = out_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anica-motionloom-scene-post-gpu-bg"),
            layout: &self.post_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&base_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&out_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("anica-motionloom-scene-post-gpu-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.post_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(
            self.width.div_ceil(16).max(1),
            self.height.div_ceil(16).max(1),
            1,
        );
    }

    fn load_image_texture(
        &mut self,
        src: &str,
    ) -> Result<(u32, u32, std::sync::Arc<wgpu::Texture>), MotionLoomSceneRenderError> {
        if !self.image_textures.contains_key(src) {
            let image = load_rgba_image_source(src)?;
            let (width, height) = image.dimensions();
            let texture = self.make_source_texture(width.max(1), height.max(1));
            self.write_texture_rgba(&texture, width.max(1), height.max(1), image.as_raw());
            self.image_textures.insert(
                src.to_string(),
                WgpuImageTexture {
                    width: width.max(1),
                    height: height.max(1),
                    texture: std::sync::Arc::new(texture),
                },
            );
        }
        let source = self
            .image_textures
            .get(src)
            .expect("GPU image texture inserted before lookup");
        Ok((source.width, source.height, source.texture.clone()))
    }

    fn load_svg_texture(
        &mut self,
        src: &str,
    ) -> Result<(u32, u32, std::sync::Arc<wgpu::Texture>), MotionLoomSceneRenderError> {
        let cache_key = format!("svg:{src}");
        if !self.image_textures.contains_key(&cache_key) {
            let image = load_svg_source(src)?;
            let (width, height) = image.dimensions();
            let texture = self.make_source_texture(width.max(1), height.max(1));
            self.write_texture_rgba(&texture, width.max(1), height.max(1), image.as_raw());
            self.image_textures.insert(
                cache_key.clone(),
                WgpuImageTexture {
                    width: width.max(1),
                    height: height.max(1),
                    texture: std::sync::Arc::new(texture),
                },
            );
        }
        let source = self
            .image_textures
            .get(&cache_key)
            .expect("GPU SVG texture inserted before lookup");
        Ok((source.width, source.height, source.texture.clone()))
    }

    fn readback_rgba(&self) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let slice = self.readback_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::PollType::wait()).map_err(|err| {
            MotionLoomSceneRenderError::GpuRender {
                message: format!("device poll failed: {err}"),
            }
        })?;
        rx.recv()
            .map_err(|err| MotionLoomSceneRenderError::GpuRender {
                message: format!("readback channel failed: {err}"),
            })?
            .map_err(|err| MotionLoomSceneRenderError::GpuRender {
                message: format!("readback map failed: {err}"),
            })?;

        let mapped = slice.get_mapped_range();
        let row_bytes = self.width as usize * 4;
        let padded_row = self.padded_bytes_per_row as usize;
        let mut out = vec![0u8; row_bytes * self.height as usize];
        for row in 0..self.height as usize {
            let src_off = row * padded_row;
            let dst_off = row * row_bytes;
            out[dst_off..dst_off + row_bytes]
                .copy_from_slice(&mapped[src_off..src_off + row_bytes]);
        }
        drop(mapped);
        self.readback_buffer.unmap();
        RgbaImage::from_raw(self.width, self.height, out).ok_or_else(|| {
            MotionLoomSceneRenderError::GpuRender {
                message: "failed to build RGBA image from GPU readback".to_string(),
            }
        })
    }
}

fn align_to_256(v: u32) -> u32 {
    const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    ((v + ALIGN - 1) / ALIGN) * ALIGN
}

fn union_texture_rect(current: Option<TextureRect>, next: TextureRect) -> Option<TextureRect> {
    if next.width == 0 || next.height == 0 {
        return current;
    }
    let Some(current) = current else {
        return Some(next);
    };
    let x0 = current.x.min(next.x);
    let y0 = current.y.min(next.y);
    let x1 = current
        .x
        .saturating_add(current.width)
        .max(next.x.saturating_add(next.width));
    let y1 = current
        .y
        .saturating_add(current.height)
        .max(next.y.saturating_add(next.height));
    Some(TextureRect {
        x: x0,
        y: y0,
        width: x1.saturating_sub(x0),
        height: y1.saturating_sub(y0),
    })
}

struct SceneFrameRenderer {
    profile: SceneRenderProfile,
    font_system: FontSystem,
    swash_cache: SwashCache,
    image_cache: HashMap<String, RgbaImage>,
    svg_cache: HashMap<String, RgbaImage>,
    path_cache: HashMap<String, Vec<Vec<Point2>>>,
    polyline_cache: HashMap<String, Vec<Point2>>,
    gradient_defs: HashMap<String, GradientDef>,
    gpu_compositor: Option<WgpuSceneCompositor>,
}

#[derive(Debug, Clone)]
struct EvaluatedShadow {
    x: f32,
    y: f32,
    blur: f32,
    color: [u8; 4],
    opacity: f32,
}

#[derive(Debug, Clone)]
enum ResolvedPaint {
    None,
    Solid([u8; 4]),
    Gradient(ResolvedGradient),
}

#[derive(Debug, Clone)]
enum ResolvedGradient {
    Linear {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stops: Vec<ResolvedGradientStop>,
        units: GradientUnits,
    },
    Radial {
        cx: f32,
        cy: f32,
        r: f32,
        stops: Vec<ResolvedGradientStop>,
        units: GradientUnits,
    },
}

#[derive(Debug, Clone, Copy)]
struct ResolvedGradientStop {
    offset: f32,
    color: [u8; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GradientUnits {
    ObjectBoundingBox,
    UserSpace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SceneBlendMode {
    Normal,
    Multiply,
    Screen,
    Add,
}

#[derive(Debug, Clone, Copy)]
struct PaintBounds {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

impl PaintBounds {
    const fn new(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Self {
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CameraRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone, Copy)]
struct TextureRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy)]
struct Affine2 {
    m00: f32,
    m01: f32,
    m02: f32,
    m10: f32,
    m11: f32,
    m12: f32,
}

impl Affine2 {
    const fn identity() -> Self {
        Self {
            m00: 1.0,
            m01: 0.0,
            m02: 0.0,
            m10: 0.0,
            m11: 1.0,
            m12: 0.0,
        }
    }

    const fn translate(x: f32, y: f32) -> Self {
        Self {
            m00: 1.0,
            m01: 0.0,
            m02: x,
            m10: 0.0,
            m11: 1.0,
            m12: y,
        }
    }

    fn rotate_deg(deg: f32) -> Self {
        let (sin_t, cos_t) = deg.to_radians().sin_cos();
        Self {
            m00: cos_t,
            m01: -sin_t,
            m02: 0.0,
            m10: sin_t,
            m11: cos_t,
            m12: 0.0,
        }
    }

    const fn scale(scale: f32) -> Self {
        Self {
            m00: scale,
            m01: 0.0,
            m02: 0.0,
            m10: 0.0,
            m11: scale,
            m12: 0.0,
        }
    }

    fn mul(self, rhs: Self) -> Self {
        Self {
            m00: self.m00 * rhs.m00 + self.m01 * rhs.m10,
            m01: self.m00 * rhs.m01 + self.m01 * rhs.m11,
            m02: self.m00 * rhs.m02 + self.m01 * rhs.m12 + self.m02,
            m10: self.m10 * rhs.m00 + self.m11 * rhs.m10,
            m11: self.m10 * rhs.m01 + self.m11 * rhs.m11,
            m12: self.m10 * rhs.m02 + self.m11 * rhs.m12 + self.m12,
        }
    }

    fn inverse(self) -> Option<Self> {
        let det = self.m00 * self.m11 - self.m01 * self.m10;
        if det.abs() <= 0.000001 {
            return None;
        }
        let inv_det = 1.0 / det;
        let m00 = self.m11 * inv_det;
        let m01 = -self.m01 * inv_det;
        let m10 = -self.m10 * inv_det;
        let m11 = self.m00 * inv_det;
        Some(Self {
            m00,
            m01,
            m02: -(m00 * self.m02 + m01 * self.m12),
            m10,
            m11,
            m12: -(m10 * self.m02 + m11 * self.m12),
        })
    }

    fn transform_point(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.m00 * x + self.m01 * y + self.m02,
            self.m10 * x + self.m11 * y + self.m12,
        )
    }

    fn is_identity(self) -> bool {
        (self.m00 - 1.0).abs() <= 0.000001
            && self.m01.abs() <= 0.000001
            && self.m02.abs() <= 0.000001
            && self.m10.abs() <= 0.000001
            && (self.m11 - 1.0).abs() <= 0.000001
            && self.m12.abs() <= 0.000001
    }
}

fn graph_logical_render_size(graph: &GraphScript) -> (u32, u32) {
    graph.size
}

fn graph_output_size(graph: &GraphScript) -> (u32, u32) {
    graph.render_size.unwrap_or(graph.size)
}

fn render_size_root_transform(output_size: (u32, u32), logical_size: (u32, u32)) -> Affine2 {
    let output_w = output_size.0.max(1) as f32;
    let output_h = output_size.1.max(1) as f32;
    let logical_w = logical_size.0.max(1) as f32;
    let logical_h = logical_size.1.max(1) as f32;
    let scale = (output_w / logical_w).min(output_h / logical_h);
    let x = (output_w - logical_w * scale) * 0.5;
    let y = (output_h - logical_h * scale) * 0.5;
    Affine2::translate(x, y).mul(Affine2::scale(scale))
}

fn fit_logical_canvas_to_output(image: &RgbaImage, output_size: (u32, u32)) -> RgbaImage {
    let output_w = output_size.0.max(1);
    let output_h = output_size.1.max(1);
    if image.width() == output_w && image.height() == output_h {
        return image.clone();
    }

    let logical_w = image.width().max(1) as f32;
    let logical_h = image.height().max(1) as f32;
    let scale = (output_w as f32 / logical_w).min(output_h as f32 / logical_h);
    let target_w = (logical_w * scale).round().max(1.0) as u32;
    let target_h = (logical_h * scale).round().max(1.0) as u32;
    let x = (output_w as f32 - target_w as f32) * 0.5;
    let y = (output_h as f32 - target_h as f32) * 0.5;
    let mut output = RgbaImage::from_pixel(output_w, output_h, Rgba([0, 0, 0, 255]));
    let scaled = image::imageops::resize(image, target_w, target_h, FilterType::Lanczos3);
    draw_rgba_image(&mut output, &scaled, x, y, 1.0);
    output
}

const GPU_SHAPE_RECT_FILL: f32 = 1.0;
const GPU_SHAPE_RECT_STROKE: f32 = 2.0;
const GPU_SHAPE_CIRCLE_FILL: f32 = 3.0;
const GPU_SHAPE_CIRCLE_STROKE: f32 = 4.0;
const GPU_SHAPE_RECT_SHADOW: f32 = 5.0;
const GPU_SHAPE_CIRCLE_SHADOW: f32 = 6.0;
const GPU_SHAPE_SOLID: f32 = 7.0;
const GPU_SHAPE_LINE: f32 = 8.0;
const GPU_SHAPE_TRIANGLE_FILL: f32 = 9.0;

#[derive(Debug, Clone)]
struct GpuScenePrimitive {
    kind: f32,
    transform: Affine2,
    shape: [f32; 4],
    radius: f32,
    stroke_width: f32,
    blur: f32,
    color: [u8; 4],
    opacity: f32,
    gradient: Option<GpuSceneGradientPaint>,
    line_t0: f32,
    line_t1: f32,
    taper_start: f32,
    taper_end: f32,
}

#[derive(Debug, Clone)]
struct GpuSceneGradientPaint {
    gradient: ResolvedGradient,
    bounds: PaintBounds,
}

#[derive(Debug, Clone)]
struct GpuSceneTextRequest {
    node: TextNode,
    transform: Affine2,
    opacity: f32,
}

#[derive(Debug, Clone)]
struct GpuSceneTextureLayer {
    image: RgbaImage,
    transform: Affine2,
    opacity: f32,
}

#[derive(Debug, Clone)]
enum CpuSceneOverlay {
    Vector { nodes: Vec<SceneNode> },
}

fn describe_cpu_scene_overlays(overlays: &[CpuSceneOverlay]) -> String {
    let mut labels: Vec<String> = overlays
        .iter()
        .take(12)
        .map(describe_cpu_scene_overlay)
        .collect();
    if overlays.len() > labels.len() {
        labels.push(format!("+{} more", overlays.len() - labels.len()));
    }
    labels.join(", ")
}

fn describe_cpu_scene_overlay(overlay: &CpuSceneOverlay) -> String {
    match overlay {
        CpuSceneOverlay::Vector { nodes } => nodes
            .first()
            .map(describe_scene_node_for_gpu)
            .unwrap_or_else(|| "Vector".to_string()),
    }
}

fn describe_scene_node_for_gpu(node: &SceneNode) -> String {
    match node {
        SceneNode::Defs(_) => "Defs".to_string(),
        SceneNode::Solid(_) => "Solid".to_string(),
        SceneNode::Text(text) => format!("Text{}", id_suffix(text.id.as_deref())),
        SceneNode::Image(image) => format!("Image{}", id_suffix(image.id.as_deref())),
        SceneNode::Svg(svg) => format!("Svg{}", id_suffix(svg.id.as_deref())),
        SceneNode::Rect(rect) => format!("Rect{}", id_suffix(rect.id.as_deref())),
        SceneNode::Circle(circle) => format!("Circle{}", id_suffix(circle.id.as_deref())),
        SceneNode::Line(line) => format!("Line{}", id_suffix(line.id.as_deref())),
        SceneNode::Polyline(polyline) => {
            format!("Polyline{}", id_suffix(polyline.id.as_deref()))
        }
        SceneNode::Path(path) => format!("Path{}", id_suffix(path.id.as_deref())),
        SceneNode::FaceJaw(face_jaw) => format!("FaceJaw{}", id_suffix(face_jaw.id.as_deref())),
        SceneNode::Shadow(shadow) => format!("Shadow{}", id_suffix(shadow.id.as_deref())),
        SceneNode::Group(group) => format!("Group{}", id_suffix(group.id.as_deref())),
        SceneNode::Part(part) => format!("Part{}", id_suffix(part.id.as_deref())),
        SceneNode::Repeat(repeat) => format!("Repeat{}", id_suffix(repeat.id.as_deref())),
        SceneNode::Mask(mask) => format!("Mask{}", id_suffix(mask.id.as_deref())),
        SceneNode::Camera(camera) => format!("Camera{}", id_suffix(camera.id.as_deref())),
        SceneNode::Character(character) => {
            format!("Character{}", id_suffix(character.id.as_deref()))
        }
    }
}

fn id_suffix(id: Option<&str>) -> String {
    id.map(|id| format!("#{id}")).unwrap_or_default()
}

impl SceneFrameRenderer {
    #[allow(dead_code)]
    fn new() -> Self {
        Self::new_for_profile(SceneRenderProfile::Cpu)
    }

    fn new_for_profile(profile: SceneRenderProfile) -> Self {
        let mut font_system = FontSystem::new();
        load_extra_fonts(&mut font_system);
        Self {
            profile,
            font_system,
            swash_cache: SwashCache::new(),
            image_cache: HashMap::new(),
            svg_cache: HashMap::new(),
            path_cache: HashMap::new(),
            polyline_cache: HashMap::new(),
            gradient_defs: HashMap::new(),
            gpu_compositor: None,
        }
    }

    fn render_frame(
        &mut self,
        graph: &GraphScript,
        frame: u32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let fps = graph.fps.max(1.0);
        let duration_sec = (graph.duration_ms as f32 / 1000.0).max(1.0 / fps);
        let time_sec = frame as f32 / fps;
        let time_norm = (time_sec / duration_sec).clamp(0.0, 1.0);
        self.gradient_defs.clear();
        collect_graph_gradient_defs(graph, &mut self.gradient_defs);
        if graph_has_rich_scene_tree(graph) {
            return self.render_scene_tree_frame(graph, time_norm, time_sec);
        }

        let mut canvas = if self.profile.uses_gpu_compositor()
            && (!graph.images.is_empty() || !graph.svgs.is_empty())
        {
            self.render_gpu_base_frame(graph, time_norm, time_sec)?
        } else {
            self.render_cpu_base_frame(graph, time_norm, time_sec)?
        };

        for text in &graph.texts {
            self.draw_text(&mut canvas, text, time_norm, time_sec)?;
        }

        if let Some(output_size) = graph.render_size {
            Ok(fit_logical_canvas_to_output(&canvas, output_size))
        } else {
            Ok(canvas)
        }
    }

    fn render_scene_tree_frame(
        &mut self,
        graph: &GraphScript,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        if let Some(image) = self.try_render_gpu_scene_tree_frame(graph, time_norm, time_sec)? {
            return Ok(image);
        }

        let (w, h) = graph_output_size(graph);
        let output_size = (w, h);
        let logical_size = graph_logical_render_size(graph);
        let root_transform = render_size_root_transform(output_size, logical_size);
        let mut resources = HashMap::<String, RgbaImage>::new();

        if !graph.scene_nodes.is_empty() {
            let canvas = if let Some(image) = self.try_render_gpu_scene_nodes(
                &graph.scene_nodes,
                output_size,
                logical_size,
                root_transform,
                time_norm,
                time_sec,
            )? {
                image
            } else {
                self.render_cpu_scene_nodes_scaled(
                    &graph.scene_nodes,
                    output_size,
                    logical_size,
                    time_norm,
                    time_sec,
                )?
            };
            resources.insert("scene".to_string(), canvas);
        }

        for scene in &graph.scenes {
            let scene_size = scene.size.unwrap_or(graph.size);
            let (scene_output_size, scene_logical_size, scene_transform) = if scene.size.is_some() {
                (scene_size, scene_size, Affine2::identity())
            } else {
                (output_size, logical_size, root_transform)
            };
            let scene_canvas = if let Some(image) = self.try_render_gpu_scene_nodes(
                &scene.children,
                scene_output_size,
                scene_logical_size,
                scene_transform,
                time_norm,
                time_sec,
            )? {
                image
            } else {
                self.render_cpu_scene_nodes_scaled(
                    &scene.children,
                    scene_output_size,
                    scene_logical_size,
                    time_norm,
                    time_sec,
                )?
            };
            resources.insert(scene.id.clone(), scene_canvas.clone());
            resources.insert(format!("scene:{}", scene.id), scene_canvas.clone());
            resources.entry("scene".to_string()).or_insert(scene_canvas);
        }

        for tex in &graph.textures {
            let image = if let Some(from) = tex.from.as_deref() {
                resources.get(from).cloned().unwrap_or_else(|| {
                    RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([0, 0, 0, 0]))
                })
            } else {
                let size = tex.size.unwrap_or(graph.size);
                RgbaImage::from_pixel(size.0.max(1), size.1.max(1), Rgba([0, 0, 0, 0]))
            };
            resources.insert(tex.id.clone(), image);
        }

        for pass in &graph.passes {
            let Some(input_id) = pass
                .inputs
                .first()
                .map(|input| input.resource_id().to_string())
            else {
                continue;
            };
            let Some(input) = resources.get(&input_id).cloned() else {
                continue;
            };
            let output = self.apply_scene_post_pass(&input, pass, time_norm, time_sec)?;
            for output_ref in &pass.outputs {
                resources.insert(output_ref.resource_id().to_string(), output.clone());
            }
        }

        for output in &graph.outputs {
            if let Some(from) = output.from.as_deref()
                && let Some(image) = resources.get(from).cloned()
            {
                resources.insert(output.id.clone(), image);
            }
        }

        let image = resources
            .get(&graph.present.from)
            .or_else(|| {
                graph
                    .present
                    .from
                    .strip_prefix("scene:")
                    .and_then(|id| resources.get(id))
            })
            .or_else(|| resources.get("scene"))
            .cloned()
            .unwrap_or_else(|| RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([0, 0, 0, 0])));
        if image.width() == w && image.height() == h {
            Ok(image)
        } else {
            let mut canvas = RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([0, 0, 0, 0]));
            draw_rgba_image(&mut canvas, &image, 0.0, 0.0, 1.0);
            Ok(canvas)
        }
    }

    fn try_render_gpu_scene_tree_frame(
        &mut self,
        graph: &GraphScript,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<Option<RgbaImage>, MotionLoomSceneRenderError> {
        if !self.profile.uses_gpu_compositor()
            || !graph.textures.is_empty()
            || !graph.passes.is_empty()
            || !graph.outputs.is_empty()
        {
            return Ok(None);
        }

        let Some(nodes) = scene_nodes_for_present(graph) else {
            return Ok(None);
        };
        if scene_nodes_contain_image_or_svg(nodes) {
            return Ok(None);
        }

        self.try_render_gpu_scene_nodes(
            nodes,
            graph_output_size(graph),
            graph_logical_render_size(graph),
            render_size_root_transform(graph_output_size(graph), graph_logical_render_size(graph)),
            time_norm,
            time_sec,
        )
    }

    fn try_render_gpu_scene_nodes(
        &mut self,
        nodes: &[SceneNode],
        output_size: (u32, u32),
        logical_size: (u32, u32),
        root_transform: Affine2,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<Option<RgbaImage>, MotionLoomSceneRenderError> {
        if !self.profile.uses_gpu_compositor() {
            return Ok(None);
        }
        if scene_nodes_contain_image_or_svg(nodes) {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: "GPU Render is strict for MotionLoom: Image/Svg scene nodes are not GPU-native yet. Use Compatibility Render (CPU) explicitly, or remove Image/Svg from the scene.".to_string(),
            });
        }

        let mut primitives = Vec::<GpuScenePrimitive>::new();
        let mut scene_overlays = Vec::<CpuSceneOverlay>::new();
        let mut text_requests = Vec::<GpuSceneTextRequest>::new();
        collect_gpu_scene_commands(
            nodes,
            root_transform,
            1.0,
            time_norm,
            time_sec,
            logical_size,
            &self.gradient_defs,
            &mut primitives,
            &mut text_requests,
            &mut scene_overlays,
        )?;

        if !scene_overlays.is_empty() {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: format!(
                    "GPU Render is strict for MotionLoom: CPU overlays are disabled. Unsupported GPU-native nodes: {}",
                    describe_cpu_scene_overlays(&scene_overlays)
                ),
            });
        }

        self.ensure_gpu_compositor_size(output_size.0.max(1), output_size.1.max(1))?;

        let mut texture_layers = Vec::with_capacity(text_requests.len());
        for request in text_requests {
            if let Some(layer) = self.rasterize_text_texture_layer(
                &request.node,
                request.transform,
                request.opacity,
                time_norm,
                time_sec,
                logical_size,
            )? {
                texture_layers.push(layer);
            }
        }

        let compositor =
            self.gpu_compositor
                .as_mut()
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                })?;
        let canvas = compositor.render_scene_content(&primitives, &texture_layers)?;
        Ok(Some(canvas))
    }

    fn ensure_gpu_compositor_size(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let needs_new_compositor = self
            .gpu_compositor
            .as_ref()
            .map(|compositor| compositor.width != width || compositor.height != height)
            .unwrap_or(true);
        if needs_new_compositor {
            self.gpu_compositor = Some(WgpuSceneCompositor::new(width, height)?);
        }
        Ok(())
    }

    fn render_cpu_scene_nodes_scaled(
        &mut self,
        nodes: &[SceneNode],
        output_size: (u32, u32),
        logical_size: (u32, u32),
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let mut logical_canvas = RgbaImage::from_pixel(
            logical_size.0.max(1),
            logical_size.1.max(1),
            Rgba([0, 0, 0, 0]),
        );
        self.draw_scene_nodes(&mut logical_canvas, nodes, time_norm, time_sec, 1.0)?;
        Ok(fit_logical_canvas_to_output(&logical_canvas, output_size))
    }

    fn apply_scene_post_pass(
        &mut self,
        input: &RgbaImage,
        pass: &PassNode,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        if let Some((horizontal, sigma)) = scene_post_blur_params(pass, time_norm, time_sec)?
            && self.profile.uses_gpu_compositor()
        {
            self.ensure_gpu_compositor_size(input.width().max(1), input.height().max(1))?;
            let compositor = self.gpu_compositor.as_mut().ok_or_else(|| {
                MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                }
            })?;
            return compositor.apply_gpu_blur_passes(input, &[(horizontal, sigma)]);
        }
        if self.profile.uses_gpu_compositor() {
            return Err(MotionLoomSceneRenderError::GpuRender {
                message: format!(
                    "GPU Render is strict for MotionLoom: post pass '{}' is not GPU-native yet. Use Compatibility Render (CPU) explicitly, or remove/replace this pass.",
                    pass.effect
                ),
            });
        }
        apply_scene_post_pass(input, pass, time_norm, time_sec)
    }

    fn draw_scene_nodes(
        &mut self,
        canvas: &mut RgbaImage,
        nodes: &[SceneNode],
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let mut pending_shadow: Option<EvaluatedShadow> = None;
        for node in nodes {
            match node {
                SceneNode::Defs(_) => {
                    pending_shadow = None;
                }
                SceneNode::Solid(solid) => {
                    let mut color = parse_color(&solid.color)?;
                    color[3] = ((color[3] as f32) * inherited_opacity)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    for pixel in canvas.pixels_mut() {
                        *pixel = Rgba(color);
                    }
                    pending_shadow = None;
                }
                SceneNode::Text(text) => {
                    self.draw_text_with_opacity(
                        canvas,
                        text,
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                    pending_shadow = None;
                }
                SceneNode::Image(image) => {
                    self.draw_image_with_opacity(
                        canvas,
                        image,
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                    pending_shadow = None;
                }
                SceneNode::Svg(svg) => {
                    self.draw_svg_with_opacity(
                        canvas,
                        svg,
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                    pending_shadow = None;
                }
                SceneNode::Rect(rect) => {
                    self.draw_rect(
                        canvas,
                        rect,
                        pending_shadow.take(),
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                }
                SceneNode::Circle(circle) => {
                    self.draw_circle(
                        canvas,
                        circle,
                        pending_shadow.take(),
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                }
                SceneNode::Line(line) => {
                    self.draw_line(canvas, line, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Polyline(polyline) => {
                    self.draw_polyline(canvas, polyline, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Path(path) => {
                    self.draw_path(canvas, path, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::FaceJaw(face_jaw) => {
                    self.draw_face_jaw(
                        canvas,
                        face_jaw,
                        Affine2::identity(),
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                    pending_shadow = None;
                }
                SceneNode::Shadow(shadow) => {
                    pending_shadow = Some(evaluate_shadow(
                        shadow,
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?);
                }
                SceneNode::Group(group) => {
                    self.draw_group(canvas, group, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Part(part) => {
                    self.draw_part(canvas, part, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Repeat(repeat) => {
                    self.draw_repeat(canvas, repeat, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Mask(mask) => {
                    self.draw_mask(canvas, mask, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Camera(camera) => {
                    self.draw_camera(canvas, camera, time_norm, time_sec, inherited_opacity)?;
                    pending_shadow = None;
                }
                SceneNode::Character(character) => {
                    self.draw_character(
                        canvas,
                        character,
                        Affine2::identity(),
                        inherited_opacity,
                        time_norm,
                        time_sec,
                    )?;
                    pending_shadow = None;
                }
            }
        }
        Ok(())
    }

    fn render_cpu_base_frame(
        &mut self,
        graph: &GraphScript,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let (w, h) = graph.size;
        let mut canvas = RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([0, 0, 0, 0]));

        for solid in &graph.solids {
            let color = parse_color(&solid.color)?;
            for pixel in canvas.pixels_mut() {
                *pixel = Rgba(color);
            }
        }

        for image in &graph.images {
            self.draw_image(&mut canvas, image, time_norm, time_sec)?;
        }
        for svg in &graph.svgs {
            self.draw_svg(&mut canvas, svg, time_norm, time_sec)?;
        }
        Ok(canvas)
    }

    fn render_gpu_base_frame(
        &mut self,
        graph: &GraphScript,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        if self.gpu_compositor.is_none() {
            self.gpu_compositor = Some(WgpuSceneCompositor::new(
                graph.size.0.max(1),
                graph.size.1.max(1),
            )?);
        }

        let compositor =
            self.gpu_compositor
                .as_mut()
                .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                    message: "GPU compositor was not initialized".to_string(),
                })?;
        let solid = graph
            .solids
            .last()
            .map(|solid| parse_color(&solid.color))
            .transpose()?
            .unwrap_or([0, 0, 0, 0]);
        compositor.render(graph, solid, time_norm, time_sec)
    }

    fn draw_text(
        &mut self,
        canvas: &mut RgbaImage,
        text: &TextNode,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        self.draw_text_with_opacity(canvas, text, time_norm, time_sec, 1.0)
    }

    fn draw_text_with_opacity(
        &mut self,
        canvas: &mut RgbaImage,
        text: &TextNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        self.draw_text_transformed(
            canvas,
            text,
            Affine2::identity(),
            inherited_opacity,
            time_norm,
            time_sec,
        )
    }

    fn draw_image(
        &mut self,
        canvas: &mut RgbaImage,
        image_node: &ImageNode,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        self.draw_image_with_opacity(canvas, image_node, time_norm, time_sec, 1.0)
    }

    fn draw_image_with_opacity(
        &mut self,
        canvas: &mut RgbaImage,
        image_node: &ImageNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&image_node.opacity, time_norm, time_sec)?
            * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }

        let scale = eval_scene_number(&image_node.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
        let source = self.load_image_asset(&image_node.src)?;
        let target_w = ((source.width() as f32) * scale).round().max(1.0) as u32;
        let target_h = ((source.height() as f32) * scale).round().max(1.0) as u32;

        let x_base = resolve_axis(
            &image_node.x,
            canvas.width() as f32,
            target_w as f32,
            time_norm,
            time_sec,
        )?;
        let y_base = resolve_axis(
            &image_node.y,
            canvas.height() as f32,
            target_h as f32,
            time_norm,
            time_sec,
        )?;
        if target_w == source.width() && target_h == source.height() {
            draw_rgba_image(canvas, source, x_base, y_base, opacity);
        } else {
            let scaled = image::imageops::resize(source, target_w, target_h, FilterType::Lanczos3);
            draw_rgba_image(canvas, &scaled, x_base, y_base, opacity);
        }
        Ok(())
    }

    fn draw_svg(
        &mut self,
        canvas: &mut RgbaImage,
        svg_node: &SvgNode,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        self.draw_svg_with_opacity(canvas, svg_node, time_norm, time_sec, 1.0)
    }

    fn draw_svg_with_opacity(
        &mut self,
        canvas: &mut RgbaImage,
        svg_node: &SvgNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&svg_node.opacity, time_norm, time_sec)?
            * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }

        let scale = eval_scene_number(&svg_node.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
        let source = self.load_svg_asset(&svg_node.src)?;
        let target_w = ((source.width() as f32) * scale).round().max(1.0) as u32;
        let target_h = ((source.height() as f32) * scale).round().max(1.0) as u32;

        let x_base = resolve_axis(
            &svg_node.x,
            canvas.width() as f32,
            target_w as f32,
            time_norm,
            time_sec,
        )?;
        let y_base = resolve_axis(
            &svg_node.y,
            canvas.height() as f32,
            target_h as f32,
            time_norm,
            time_sec,
        )?;
        if target_w == source.width() && target_h == source.height() {
            draw_rgba_image(canvas, source, x_base, y_base, opacity);
        } else {
            let scaled = image::imageops::resize(source, target_w, target_h, FilterType::Lanczos3);
            draw_rgba_image(canvas, &scaled, x_base, y_base, opacity);
        }
        Ok(())
    }

    fn draw_group(
        &mut self,
        canvas: &mut RgbaImage,
        group: &GroupNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&group.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let x = eval_scene_number(&group.x, time_norm, time_sec)?;
        let y = eval_scene_number(&group.y, time_norm, time_sec)?;
        let rotation = eval_scene_number(&group.rotation, time_norm, time_sec)?;
        let scale = eval_scene_number(&group.scale, time_norm, time_sec)?.clamp(0.001, 64.0);

        let mut layer = RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
        self.draw_scene_nodes(&mut layer, &group.children, time_norm, time_sec, opacity)?;
        composite_transformed_layer(canvas, &layer, x, y, rotation, scale);
        Ok(())
    }

    fn draw_part(
        &mut self,
        canvas: &mut RgbaImage,
        part: &PartNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&part.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let x = eval_scene_number(&part.x, time_norm, time_sec)?;
        let y = eval_scene_number(&part.y, time_norm, time_sec)?;
        let rotation = eval_scene_number(&part.rotation, time_norm, time_sec)?;
        let scale = eval_scene_number(&part.scale, time_norm, time_sec)?.clamp(0.001, 64.0);

        let mut layer = RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
        self.draw_scene_nodes(&mut layer, &part.children, time_norm, time_sec, opacity)?;
        composite_transformed_layer(canvas, &layer, x, y, rotation, scale);
        Ok(())
    }

    fn draw_repeat(
        &mut self,
        canvas: &mut RgbaImage,
        repeat: &RepeatNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let count = eval_repeat_count(&repeat.count, time_norm, time_sec)?;
        if count == 0 {
            return Ok(());
        }

        let x = eval_scene_number(&repeat.x, time_norm, time_sec)?;
        let y = eval_scene_number(&repeat.y, time_norm, time_sec)?;
        let rotation = eval_scene_number(&repeat.rotation, time_norm, time_sec)?;
        let scale = eval_scene_number(&repeat.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
        let opacity = eval_scene_number(&repeat.opacity, time_norm, time_sec)?;
        let x_step = eval_scene_number(&repeat.x_step, time_norm, time_sec)?;
        let y_step = eval_scene_number(&repeat.y_step, time_norm, time_sec)?;
        let rotation_step = eval_scene_number(&repeat.rotation_step, time_norm, time_sec)?;
        let scale_step = eval_scene_number(&repeat.scale_step, time_norm, time_sec)?;
        let opacity_step = eval_scene_number(&repeat.opacity_step, time_norm, time_sec)?;

        for index in 0..count {
            let i = index as f32;
            let copy_opacity = ((opacity + opacity_step * i) * inherited_opacity).clamp(0.0, 1.0);
            if copy_opacity <= 0.0001 {
                continue;
            }
            let copy_scale = (scale + scale_step * i).clamp(0.001, 64.0);
            let mut layer =
                RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
            self.draw_scene_nodes(
                &mut layer,
                &repeat.children,
                time_norm,
                time_sec,
                copy_opacity,
            )?;
            composite_transformed_layer(
                canvas,
                &layer,
                x + x_step * i,
                y + y_step * i,
                rotation + rotation_step * i,
                copy_scale,
            );
        }
        Ok(())
    }

    fn draw_mask(
        &mut self,
        canvas: &mut RgbaImage,
        mask: &MaskNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&mask.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let mut layer = RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
        self.draw_scene_nodes(&mut layer, &mask.children, time_norm, time_sec, opacity)?;
        let mask_alpha = self.render_mask_alpha(
            canvas.width(),
            canvas.height(),
            mask,
            Affine2::identity(),
            time_norm,
            time_sec,
        )?;
        apply_alpha_mask(&mut layer, &mask_alpha);
        composite_layer(canvas, &layer);
        Ok(())
    }

    fn draw_camera(
        &mut self,
        canvas: &mut RgbaImage,
        camera: &CameraNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&camera.opacity, time_norm, time_sec)?
            * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        ensure_camera_2d(camera)?;

        let canvas_w = canvas.width();
        let canvas_h = canvas.height();
        let transform = camera_transform(
            camera,
            &camera.children,
            canvas_w,
            canvas_h,
            time_norm,
            time_sec,
        )?;
        let world_bounds = camera_world_bounds(camera, canvas_w, canvas_h, time_norm, time_sec)?;
        let layer_w = world_bounds
            .map(|rect| canvas_w.max((rect.x + rect.width).ceil().max(canvas_w as f32) as u32 + 2))
            .unwrap_or_else(|| canvas_w.saturating_add(2));
        let layer_h = world_bounds
            .map(|rect| canvas_h.max((rect.y + rect.height).ceil().max(canvas_h as f32) as u32 + 2))
            .unwrap_or_else(|| canvas_h.saturating_add(2));
        let mut layer = RgbaImage::from_pixel(layer_w, layer_h, Rgba([0, 0, 0, 0]));
        self.draw_scene_nodes(&mut layer, &camera.children, time_norm, time_sec, opacity)?;
        let viewport = camera_viewport(camera, canvas_w, canvas_h, time_norm, time_sec)?;
        composite_layer_affine_clipped(canvas, &layer, transform, Some(viewport));
        Ok(())
    }

    fn cached_path_subpaths(
        &mut self,
        data: &str,
    ) -> Result<Vec<Vec<Point2>>, MotionLoomSceneRenderError> {
        if let Some(cached) = self.path_cache.get(data) {
            return Ok(cached.clone());
        }
        let parsed = parse_path_subpaths(data)?;
        self.path_cache.insert(data.to_string(), parsed.clone());
        Ok(parsed)
    }

    fn cached_polyline_points(
        &mut self,
        points: &str,
    ) -> Result<Vec<Point2>, MotionLoomSceneRenderError> {
        if let Some(cached) = self.polyline_cache.get(points) {
            return Ok(cached.clone());
        }
        let parsed = parse_polyline_points(points)?;
        self.polyline_cache
            .insert(points.to_string(), parsed.clone());
        Ok(parsed)
    }

    fn render_mask_alpha(
        &mut self,
        width: u32,
        height: u32,
        mask: &MaskNode,
        transform: Affine2,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<RgbaImage, MotionLoomSceneRenderError> {
        let mut alpha = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]));
        let opacity = eval_scene_number(&mask.opacity, time_norm, time_sec)?.clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(alpha);
        }
        let mask_color = [255, 255, 255, (opacity * 255.0).round() as u8];
        match mask.shape.trim().to_ascii_lowercase().as_str() {
            "circle" => {
                let x = eval_scene_number(&mask.x, time_norm, time_sec)?;
                let y = eval_scene_number(&mask.y, time_norm, time_sec)?;
                let radius = eval_scene_number(&mask.radius, time_norm, time_sec)?.max(0.0)
                    * affine_uniform_scale(transform);
                let (x, y) = transform.transform_point(x, y);
                draw_circle(&mut alpha, x, y, radius, mask_color);
            }
            "path" => {
                let Some(d) = mask.d.as_deref() else {
                    return Ok(alpha);
                };
                let subpaths = self.cached_path_subpaths(d)?;
                draw_transformed_filled_polylines(&mut alpha, &subpaths, mask_color, transform);
            }
            _ => {
                let scale = affine_uniform_scale(transform);
                let x = eval_scene_number(&mask.x, time_norm, time_sec)?;
                let y = eval_scene_number(&mask.y, time_norm, time_sec)?;
                let w = eval_scene_number(&mask.width, time_norm, time_sec)?.max(0.0) * scale;
                let h = eval_scene_number(&mask.height, time_norm, time_sec)?.max(0.0) * scale;
                let radius = eval_scene_number(&mask.radius, time_norm, time_sec)?.max(0.0) * scale;
                let (x, y) = transform.transform_point(x, y);
                draw_rounded_rect(&mut alpha, x, y, w, h, radius, mask_color);
            }
        }
        Ok(alpha)
    }

    fn draw_character(
        &mut self,
        canvas: &mut RgbaImage,
        character: &CharacterNode,
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&character.opacity, time_norm, time_sec)?
            * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let x = eval_scene_number(&character.x, time_norm, time_sec)?;
        let y = eval_scene_number(&character.y, time_norm, time_sec)?;
        let rotation = eval_scene_number(&character.rotation, time_norm, time_sec)?;
        let scale = eval_scene_number(&character.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
        let character_transform = transform
            .mul(Affine2::translate(x, y))
            .mul(Affine2::rotate_deg(rotation))
            .mul(Affine2::scale(scale));
        self.draw_character_nodes_vector(
            canvas,
            &character.children,
            character_transform,
            opacity,
            time_norm,
            time_sec,
        )
    }

    fn draw_character_nodes_vector(
        &mut self,
        canvas: &mut RgbaImage,
        nodes: &[SceneNode],
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        for node in nodes {
            match node {
                SceneNode::Defs(_) => {}
                SceneNode::Group(group) => {
                    let opacity = (eval_scene_number(&group.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let x = eval_scene_number(&group.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&group.y, time_norm, time_sec)?;
                    let rotation = eval_scene_number(&group.rotation, time_norm, time_sec)?;
                    let scale =
                        eval_scene_number(&group.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
                    let group_transform = transform
                        .mul(Affine2::translate(x, y))
                        .mul(Affine2::rotate_deg(rotation))
                        .mul(Affine2::scale(scale));
                    self.draw_character_nodes_vector(
                        canvas,
                        &group.children,
                        group_transform,
                        opacity,
                        time_norm,
                        time_sec,
                    )?;
                }
                SceneNode::Part(part) => {
                    let opacity = (eval_scene_number(&part.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let x = eval_scene_number(&part.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&part.y, time_norm, time_sec)?;
                    let rotation = eval_scene_number(&part.rotation, time_norm, time_sec)?;
                    let scale =
                        eval_scene_number(&part.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
                    let part_transform = transform
                        .mul(Affine2::translate(x, y))
                        .mul(Affine2::rotate_deg(rotation))
                        .mul(Affine2::scale(scale));
                    self.draw_character_nodes_vector(
                        canvas,
                        &part.children,
                        part_transform,
                        opacity,
                        time_norm,
                        time_sec,
                    )?;
                }
                SceneNode::Repeat(repeat) => {
                    let count = eval_repeat_count(&repeat.count, time_norm, time_sec)?;
                    if count == 0 {
                        continue;
                    }
                    let x = eval_scene_number(&repeat.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&repeat.y, time_norm, time_sec)?;
                    let rotation = eval_scene_number(&repeat.rotation, time_norm, time_sec)?;
                    let scale =
                        eval_scene_number(&repeat.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
                    let opacity = eval_scene_number(&repeat.opacity, time_norm, time_sec)?;
                    let x_step = eval_scene_number(&repeat.x_step, time_norm, time_sec)?;
                    let y_step = eval_scene_number(&repeat.y_step, time_norm, time_sec)?;
                    let rotation_step =
                        eval_scene_number(&repeat.rotation_step, time_norm, time_sec)?;
                    let scale_step = eval_scene_number(&repeat.scale_step, time_norm, time_sec)?;
                    let opacity_step =
                        eval_scene_number(&repeat.opacity_step, time_norm, time_sec)?;
                    for index in 0..count {
                        let i = index as f32;
                        let copy_opacity =
                            ((opacity + opacity_step * i) * inherited_opacity).clamp(0.0, 1.0);
                        if copy_opacity <= 0.0001 {
                            continue;
                        }
                        let copy_scale = (scale + scale_step * i).clamp(0.001, 64.0);
                        let repeat_transform = transform
                            .mul(Affine2::translate(x + x_step * i, y + y_step * i))
                            .mul(Affine2::rotate_deg(rotation + rotation_step * i))
                            .mul(Affine2::scale(copy_scale));
                        self.draw_character_nodes_vector(
                            canvas,
                            &repeat.children,
                            repeat_transform,
                            copy_opacity,
                            time_norm,
                            time_sec,
                        )?;
                    }
                }
                SceneNode::Mask(mask) => {
                    let opacity = (eval_scene_number(&mask.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let mut layer =
                        RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
                    self.draw_character_nodes_vector(
                        &mut layer,
                        &mask.children,
                        transform,
                        opacity,
                        time_norm,
                        time_sec,
                    )?;
                    let mask_alpha = self.render_mask_alpha(
                        canvas.width(),
                        canvas.height(),
                        mask,
                        transform,
                        time_norm,
                        time_sec,
                    )?;
                    apply_alpha_mask(&mut layer, &mask_alpha);
                    composite_layer(canvas, &layer);
                }
                SceneNode::Character(character) => {
                    self.draw_character(
                        canvas,
                        character,
                        transform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                    )?;
                }
                SceneNode::Text(text) => {
                    self.draw_text_transformed(
                        canvas,
                        text,
                        transform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                    )?;
                }
                SceneNode::Line(line) => {
                    let opacity = (eval_scene_number(&line.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let style = eval_line_stroke_style(line, time_norm, time_sec)?;
                    let width = eval_scene_number(&line.width, time_norm, time_sec)?.max(0.0)
                        * affine_uniform_scale(transform);
                    if width <= 0.0001 {
                        continue;
                    }
                    let x1 = eval_scene_number(&line.x1, time_norm, time_sec)?;
                    let y1 = eval_scene_number(&line.y1, time_norm, time_sec)?;
                    let x2 = eval_scene_number(&line.x2, time_norm, time_sec)?;
                    let y2 = eval_scene_number(&line.y2, time_norm, time_sec)?;
                    let mut color = parse_color(&line.color)?;
                    color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
                    let (x1, y1) = transform.transform_point(x1, y1);
                    let (x2, y2) = transform.transform_point(x2, y2);
                    draw_line_segment_styled(
                        canvas,
                        Point2::new(x1, y1),
                        Point2::new(x2, y2),
                        width,
                        color,
                        style,
                        0.0,
                        1.0,
                    );
                }
                SceneNode::Polyline(polyline) => {
                    let opacity = (eval_scene_number(&polyline.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let style = eval_polyline_stroke_style(polyline, time_norm, time_sec)?;
                    let width = eval_scene_number(&polyline.stroke_width, time_norm, time_sec)?
                        .max(0.0)
                        * affine_uniform_scale(transform);
                    if width <= 0.0001 {
                        continue;
                    }
                    let points = self.cached_polyline_points(&polyline.points)?;
                    let trim = evaluate_trim(
                        &polyline.trim_start,
                        &polyline.trim_end,
                        time_norm,
                        time_sec,
                    )?;
                    if let Some(mut color) = parse_paint(&polyline.stroke)? {
                        color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
                        draw_transformed_trimmed_polylines_styled(
                            canvas,
                            &[points],
                            width,
                            color,
                            trim,
                            transform,
                            style,
                        );
                    }
                }
                SceneNode::Path(path) => {
                    let opacity = (eval_scene_number(&path.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let subpaths = self.cached_path_subpaths(&path.d)?;
                    if let Some(fill) = path.fill.as_deref() {
                        let paint = self.resolve_paint(fill)?;
                        let blend = parse_scene_blend(&path.blend)?;
                        draw_transformed_filled_polylines_paint(
                            canvas, &subpaths, &paint, opacity, blend, transform,
                        );
                    }
                    let width = eval_scene_number(&path.stroke_width, time_norm, time_sec)?
                        .max(0.0)
                        * affine_uniform_scale(transform);
                    if width <= 0.0001 {
                        continue;
                    }
                    let trim =
                        evaluate_trim(&path.trim_start, &path.trim_end, time_norm, time_sec)?;
                    let style = eval_path_stroke_style(path, time_norm, time_sec)?;
                    if let Some(mut color) = parse_paint(&path.stroke)? {
                        color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
                        draw_transformed_trimmed_polylines_styled(
                            canvas, &subpaths, width, color, trim, transform, style,
                        );
                    }
                }
                SceneNode::FaceJaw(face_jaw) => {
                    self.draw_face_jaw(
                        canvas,
                        face_jaw,
                        transform,
                        time_norm,
                        time_sec,
                        inherited_opacity,
                    )?;
                }
                SceneNode::Circle(circle) => {
                    let opacity = (eval_scene_number(&circle.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let x = eval_scene_number(&circle.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&circle.y, time_norm, time_sec)?;
                    let radius = eval_scene_number(&circle.radius, time_norm, time_sec)?.max(0.0)
                        * affine_uniform_scale(transform);
                    if radius <= 0.0001 {
                        continue;
                    }
                    let paint = self.resolve_paint(&circle.color)?;
                    let blend = parse_scene_blend(&circle.blend)?;
                    let stroke = circle.stroke.as_deref().map(parse_color).transpose()?;
                    let stroke_width =
                        eval_scene_number(&circle.stroke_width, time_norm, time_sec)?.max(0.0)
                            * affine_uniform_scale(transform);
                    let (x, y) = transform.transform_point(x, y);
                    draw_circle_paint(canvas, x, y, radius, &paint, opacity, blend);
                    if let Some(mut stroke) = stroke {
                        stroke[3] = ((stroke[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
                        draw_circle_stroke(canvas, x, y, radius, stroke_width, stroke);
                    }
                }
                SceneNode::Rect(rect) => {
                    let opacity = (eval_scene_number(&rect.opacity, time_norm, time_sec)?
                        * inherited_opacity)
                        .clamp(0.0, 1.0);
                    if opacity <= 0.0001 {
                        continue;
                    }
                    let scale = affine_uniform_scale(transform);
                    let x = eval_scene_number(&rect.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&rect.y, time_norm, time_sec)?;
                    let width =
                        eval_scene_number(&rect.width, time_norm, time_sec)?.max(0.0) * scale;
                    let height =
                        eval_scene_number(&rect.height, time_norm, time_sec)?.max(0.0) * scale;
                    if width <= 0.0001 || height <= 0.0001 {
                        continue;
                    }
                    let radius =
                        eval_scene_number(&rect.radius, time_norm, time_sec)?.max(0.0) * scale;
                    let paint = self.resolve_paint(&rect.color)?;
                    let blend = parse_scene_blend(&rect.blend)?;
                    let stroke = rect.stroke.as_deref().map(parse_color).transpose()?;
                    let stroke_width = eval_scene_number(&rect.stroke_width, time_norm, time_sec)?
                        .max(0.0)
                        * scale;
                    let (x, y) = transform.transform_point(x, y);
                    draw_rounded_rect_paint(
                        canvas, x, y, width, height, radius, &paint, opacity, blend,
                    );
                    if let Some(mut stroke) = stroke {
                        stroke[3] = ((stroke[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
                        draw_rounded_rect_stroke(
                            canvas,
                            x,
                            y,
                            width,
                            height,
                            radius,
                            stroke_width,
                            stroke,
                        );
                    }
                }
                SceneNode::Solid(_)
                | SceneNode::Image(_)
                | SceneNode::Svg(_)
                | SceneNode::Shadow(_) => {}
                SceneNode::Camera(_) => {}
            }
        }
        Ok(())
    }

    fn resolve_paint(&self, value: &str) -> Result<ResolvedPaint, MotionLoomSceneRenderError> {
        if is_none_paint(value) {
            return Ok(ResolvedPaint::None);
        }
        if let Some(id) = gradient_ref_id(value) {
            let Some(gradient) = self.gradient_defs.get(id) else {
                return Err(MotionLoomSceneRenderError::InvalidPaint {
                    value: value.to_string(),
                    message: format!("gradient reference not found: {id}"),
                });
            };
            return resolve_gradient_paint(value, gradient).map(ResolvedPaint::Gradient);
        }
        parse_color(value).map(ResolvedPaint::Solid)
    }

    fn draw_rect(
        &mut self,
        canvas: &mut RgbaImage,
        rect: &RectNode,
        shadow: Option<EvaluatedShadow>,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&rect.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let x = eval_scene_number(&rect.x, time_norm, time_sec)?;
        let y = eval_scene_number(&rect.y, time_norm, time_sec)?;
        let width = eval_scene_number(&rect.width, time_norm, time_sec)?.max(0.0);
        let height = eval_scene_number(&rect.height, time_norm, time_sec)?.max(0.0);
        let radius = eval_scene_number(&rect.radius, time_norm, time_sec)?.max(0.0);
        let rotation = eval_scene_number(&rect.rotation, time_norm, time_sec)?;
        let paint = self.resolve_paint(&rect.color)?;
        let blend = parse_scene_blend(&rect.blend)?;
        let stroke = rect.stroke.as_deref().map(parse_color).transpose()?;
        let stroke_width = eval_scene_number(&rect.stroke_width, time_norm, time_sec)?.max(0.0);

        if rotation.abs() > 0.001 {
            let mut layer =
                RgbaImage::from_pixel(canvas.width(), canvas.height(), Rgba([0, 0, 0, 0]));
            if let Some(shadow) = shadow {
                draw_rect_shadow(&mut layer, x, y, width, height, radius, &shadow);
            }
            draw_rounded_rect_paint(
                &mut layer, x, y, width, height, radius, &paint, opacity, blend,
            );
            if let Some(stroke) = stroke {
                draw_rounded_rect_stroke(
                    &mut layer,
                    x,
                    y,
                    width,
                    height,
                    radius,
                    stroke_width,
                    stroke,
                );
            }
            composite_transformed_layer(canvas, &layer, 0.0, 0.0, rotation, 1.0);
            return Ok(());
        }

        if let Some(shadow) = shadow {
            draw_rect_shadow(canvas, x, y, width, height, radius, &shadow);
        }
        draw_rounded_rect_paint(canvas, x, y, width, height, radius, &paint, opacity, blend);
        if let Some(stroke) = stroke {
            draw_rounded_rect_stroke(canvas, x, y, width, height, radius, stroke_width, stroke);
        }
        Ok(())
    }

    fn draw_circle(
        &mut self,
        canvas: &mut RgbaImage,
        circle: &CircleNode,
        shadow: Option<EvaluatedShadow>,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&circle.opacity, time_norm, time_sec)?
            * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let x = eval_scene_number(&circle.x, time_norm, time_sec)?;
        let y = eval_scene_number(&circle.y, time_norm, time_sec)?;
        let radius = eval_scene_number(&circle.radius, time_norm, time_sec)?.max(0.0);
        let paint = self.resolve_paint(&circle.color)?;
        let blend = parse_scene_blend(&circle.blend)?;
        let stroke = circle.stroke.as_deref().map(parse_color).transpose()?;
        let stroke_width = eval_scene_number(&circle.stroke_width, time_norm, time_sec)?.max(0.0);

        if let Some(shadow) = shadow {
            draw_circle_shadow(canvas, x, y, radius, &shadow);
        }
        draw_circle_paint(canvas, x, y, radius, &paint, opacity, blend);
        if let Some(stroke) = stroke {
            draw_circle_stroke(canvas, x, y, radius, stroke_width, stroke);
        }
        Ok(())
    }

    fn draw_line(
        &mut self,
        canvas: &mut RgbaImage,
        line: &LineNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&line.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let x1 = eval_scene_number(&line.x1, time_norm, time_sec)?;
        let y1 = eval_scene_number(&line.y1, time_norm, time_sec)?;
        let x2 = eval_scene_number(&line.x2, time_norm, time_sec)?;
        let y2 = eval_scene_number(&line.y2, time_norm, time_sec)?;
        let width = eval_scene_number(&line.width, time_norm, time_sec)?.max(0.0);
        if width <= 0.0001 {
            return Ok(());
        }
        let style = eval_line_stroke_style(line, time_norm, time_sec)?;
        if let Some(mut color) = parse_paint(&line.color)? {
            color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
            draw_line_segment_styled(
                canvas,
                Point2::new(x1, y1),
                Point2::new(x2, y2),
                width,
                color,
                style,
                0.0,
                1.0,
            );
        }
        Ok(())
    }

    fn draw_polyline(
        &mut self,
        canvas: &mut RgbaImage,
        polyline: &PolylineNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&polyline.opacity, time_norm, time_sec)?
            * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let width = eval_scene_number(&polyline.stroke_width, time_norm, time_sec)?.max(0.0);
        if width <= 0.0001 {
            return Ok(());
        }
        let points = parse_polyline_points(&polyline.points)?;
        let trim = evaluate_trim(
            &polyline.trim_start,
            &polyline.trim_end,
            time_norm,
            time_sec,
        )?;
        let style = eval_polyline_stroke_style(polyline, time_norm, time_sec)?;
        if let Some(mut color) = parse_paint(&polyline.stroke)? {
            color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
            draw_trimmed_polylines_styled(canvas, &[points], width, color, trim, style);
        }
        Ok(())
    }

    fn draw_path(
        &mut self,
        canvas: &mut RgbaImage,
        path: &PathNode,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let opacity = (eval_scene_number(&path.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }
        let subpaths = parse_path_subpaths(&path.d)?;
        if let Some(fill) = path.fill.as_deref() {
            let paint = self.resolve_paint(fill)?;
            let blend = parse_scene_blend(&path.blend)?;
            draw_filled_polylines_paint(canvas, &subpaths, &paint, opacity, blend);
        }
        let width = eval_scene_number(&path.stroke_width, time_norm, time_sec)?.max(0.0);
        if width <= 0.0001 {
            return Ok(());
        }
        let trim = evaluate_trim(&path.trim_start, &path.trim_end, time_norm, time_sec)?;
        let style = eval_path_stroke_style(path, time_norm, time_sec)?;
        if let Some(mut color) = parse_paint(&path.stroke)? {
            color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
            draw_trimmed_polylines_styled(canvas, &subpaths, width, color, trim, style);
        }
        Ok(())
    }

    fn draw_face_jaw(
        &mut self,
        canvas: &mut RgbaImage,
        face_jaw: &FaceJawNode,
        transform: Affine2,
        time_norm: f32,
        time_sec: f32,
        inherited_opacity: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        let path = face_jaw_to_path_node(face_jaw, time_norm, time_sec)?;
        let opacity = (eval_scene_number(&path.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(());
        }

        let subpaths = parse_path_subpaths(&path.d)?;
        if let Some(fill) = path.fill.as_deref() {
            let paint = self.resolve_paint(fill)?;
            let blend = parse_scene_blend(&path.blend)?;
            draw_transformed_filled_polylines_paint(
                canvas, &subpaths, &paint, opacity, blend, transform,
            );
        }

        let width = eval_scene_number(&path.stroke_width, time_norm, time_sec)?.max(0.0)
            * affine_uniform_scale(transform);
        if width <= 0.0001 {
            return Ok(());
        }
        let trim = evaluate_trim(&path.trim_start, &path.trim_end, time_norm, time_sec)?;
        let style = eval_path_stroke_style(&path, time_norm, time_sec)?;
        if let Some(mut color) = parse_paint(&path.stroke)? {
            color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
            draw_transformed_trimmed_polylines_styled(
                canvas, &subpaths, width, color, trim, transform, style,
            );
        }
        Ok(())
    }

    fn draw_text_transformed(
        &mut self,
        canvas: &mut RgbaImage,
        text: &TextNode,
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
    ) -> Result<(), MotionLoomSceneRenderError> {
        if let Some(layer) = self.rasterize_text_texture_layer(
            text,
            transform,
            inherited_opacity,
            time_norm,
            time_sec,
            (canvas.width(), canvas.height()),
        )? {
            composite_layer_affine(canvas, &layer.image, layer.transform);
        }
        Ok(())
    }

    fn rasterize_text_texture_layer(
        &mut self,
        text: &TextNode,
        transform: Affine2,
        inherited_opacity: f32,
        time_norm: f32,
        time_sec: f32,
        canvas_size: (u32, u32),
    ) -> Result<Option<GpuSceneTextureLayer>, MotionLoomSceneRenderError> {
        if text.value.trim().is_empty() {
            return Ok(None);
        }
        if let Some(path) = text.font_path.as_deref()
            && Path::new(path).exists()
        {
            let _ = self.font_system.db_mut().load_font_file(path);
        }

        let font_size = eval_scene_number(&text.font_size, time_norm, time_sec)?.clamp(1.0, 1024.0);
        let opacity = (eval_scene_number(&text.opacity, time_norm, time_sec)? * inherited_opacity)
            .clamp(0.0, 1.0);
        if opacity <= 0.0001 {
            return Ok(None);
        }

        let line_height_raw = text
            .line_height
            .as_deref()
            .and_then(|expr| eval_scene_number(expr, time_norm, time_sec).ok())
            .unwrap_or(1.2);
        let line_height = if line_height_raw <= 10.0 {
            (font_size * line_height_raw).max(1.0)
        } else {
            line_height_raw.max(1.0)
        };
        let width = text
            .width
            .as_deref()
            .map(|expr| eval_scene_number(expr, time_norm, time_sec).map(|value| value.max(1.0)))
            .transpose()?;
        let visible_value = if let Some(expr) = text.visible_chars.as_deref() {
            let count = eval_scene_number(expr, time_norm, time_sec)
                .unwrap_or(text.value.chars().count() as f32)
                .floor()
                .clamp(0.0, text.value.chars().count() as f32) as usize;
            text.value.chars().take(count).collect::<String>()
        } else {
            text.value.clone()
        };
        let metrics = Metrics::new(font_size, line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let mut attrs = Attrs::new().family(Family::SansSerif);
        if let Some(family) = text.font_family.as_deref()
            && !family.trim().is_empty()
        {
            attrs = attrs.family(Family::Name(family));
        }
        buffer.set_text(
            &mut self.font_system,
            &visible_value,
            &attrs,
            Shaping::Advanced,
        );
        buffer.set_size(&mut self.font_system, width, None);
        buffer.shape_until_scroll(&mut self.font_system, true);

        let (text_w, text_h) = text_bounds(&buffer, line_height);
        let layout_w = width.unwrap_or(text_w);
        let x_base = resolve_axis(&text.x, canvas_size.0 as f32, layout_w, time_norm, time_sec)?;
        let y_base = resolve_axis(&text.y, canvas_size.1 as f32, text_h, time_norm, time_sec)?;
        let pad = 3_i32;
        let layer_w = (layout_w.ceil().max(1.0) as u32).saturating_add((pad * 2) as u32);
        let layer_h = (text_h.ceil().max(1.0) as u32).saturating_add((pad * 2) as u32);
        let mut layer = RgbaImage::from_pixel(layer_w, layer_h, Rgba([0, 0, 0, 0]));
        let color = parse_color(&text.color)?;
        let combined_opacity = (color[3] as f32 / 255.0) * opacity;
        let text_color = Color::rgba(color[0], color[1], color[2], 255);
        let max_lines = text
            .max_lines
            .as_deref()
            .and_then(|expr| eval_scene_number(expr, time_norm, time_sec).ok())
            .map(|value| value.floor().max(0.0) as usize);

        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            text_color,
            |x, y, _w, _h, color| {
                if let Some(max_lines) = max_lines {
                    let line_ix = ((y as f32) / line_height).floor().max(0.0) as usize;
                    if line_ix >= max_lines {
                        return;
                    }
                }
                let px = x + pad;
                let py = y + pad;
                if px < 0 || py < 0 {
                    return;
                }
                let (px, py) = (px as u32, py as u32);
                if px >= layer.width() || py >= layer.height() {
                    return;
                }
                let (sr, sg, sb, sa) = color.as_rgba_tuple();
                let sa = ((sa as f32) * combined_opacity).round().clamp(0.0, 255.0) as u8;
                blend_pixel(&mut layer, px, py, [sr, sg, sb, sa]);
            },
        );
        let text_transform =
            transform.mul(Affine2::translate(x_base - pad as f32, y_base - pad as f32));
        Ok(Some(GpuSceneTextureLayer {
            image: layer,
            transform: text_transform,
            opacity: 1.0,
        }))
    }

    fn load_image_asset(&mut self, src: &str) -> Result<&RgbaImage, MotionLoomSceneRenderError> {
        if !self.image_cache.contains_key(src) {
            let decoded = load_rgba_image_source(src)?;
            self.image_cache.insert(src.to_string(), decoded);
        }

        Ok(self
            .image_cache
            .get(src)
            .expect("image cache entry inserted before lookup"))
    }

    fn load_svg_asset(&mut self, src: &str) -> Result<&RgbaImage, MotionLoomSceneRenderError> {
        if !self.svg_cache.contains_key(src) {
            let decoded = load_svg_source(src)?;
            self.svg_cache.insert(src.to_string(), decoded);
        }

        Ok(self
            .svg_cache
            .get(src)
            .expect("SVG cache entry inserted before lookup"))
    }
}

fn load_extra_fonts(font_system: &mut FontSystem) {
    let Some(raw_dirs) = std::env::var_os("MOTIONLOOM_FONT_DIRS")
        .or_else(|| std::env::var_os("MOTIONLOOM_FONT_DIR"))
    else {
        return;
    };

    for dir in std::env::split_paths(&raw_dirs) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            let ext = ext.to_ascii_lowercase();
            if ext == "ttf" || ext == "otf" || ext == "ttc" {
                let _ = font_system.db_mut().load_font_file(&path);
            }
        }
    }
}

fn eval_scene_number(
    expr: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<f32, MotionLoomSceneRenderError> {
    eval_time_expr(expr, time_norm, time_sec).map_err(|message| {
        MotionLoomSceneRenderError::InvalidExpression {
            expr: expr.to_string(),
            message,
        }
    })
}

fn eval_repeat_count(
    expr: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<u32, MotionLoomSceneRenderError> {
    Ok(eval_scene_number(expr, time_norm, time_sec)?
        .round()
        .clamp(0.0, 1000.0) as u32)
}

fn text_bounds(buffer: &Buffer, fallback_line_height: f32) -> (f32, f32) {
    let mut width = 0.0_f32;
    let mut top = f32::INFINITY;
    let mut bottom = f32::NEG_INFINITY;
    for run in buffer.layout_runs() {
        width = width.max(run.line_w);
        top = top.min(run.line_top);
        bottom = bottom.max(run.line_top + run.line_height);
    }
    if !top.is_finite() || !bottom.is_finite() {
        return (0.0, fallback_line_height);
    }
    (width.max(0.0), (bottom - top).max(fallback_line_height))
}

fn resolve_axis(
    raw: &str,
    canvas_extent: f32,
    content_extent: f32,
    time_norm: f32,
    time_sec: f32,
) -> Result<f32, MotionLoomSceneRenderError> {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    let value = match lower.as_str() {
        "center" | "middle" => ((canvas_extent - content_extent) * 0.5).max(0.0),
        "left" | "top" => 0.0,
        "right" | "bottom" => (canvas_extent - content_extent).max(0.0),
        _ => {
            if let Some(percent) = lower.strip_suffix('%')
                && let Ok(value) = percent.trim().parse::<f32>()
            {
                return Ok((canvas_extent - content_extent) * (value / 100.0));
            }
            trimmed
                .parse::<f32>()
                .or_else(|_| eval_scene_number(trimmed, time_norm, time_sec))?
        }
    };
    Ok(value)
}

fn camera_transform(
    camera: &CameraNode,
    camera_children: &[SceneNode],
    canvas_w: u32,
    canvas_h: u32,
    time_norm: f32,
    time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    ensure_camera_2d(camera)?;
    let viewport = camera_viewport(camera, canvas_w, canvas_h, time_norm, time_sec)?;
    let zoom = eval_scene_number(&camera.zoom, time_norm, time_sec)?.clamp(0.001, 1024.0);
    let rotation = eval_scene_number(&camera.rotation, time_norm, time_sec)?;
    let (x, y) = camera_target(
        camera,
        camera_children,
        canvas_w,
        canvas_h,
        viewport,
        zoom,
        time_norm,
        time_sec,
    )?;
    let anchor_x = resolve_camera_anchor(
        &camera.anchor_x,
        viewport.x,
        viewport.width,
        time_norm,
        time_sec,
    )?;
    let anchor_y = resolve_camera_anchor(
        &camera.anchor_y,
        viewport.y,
        viewport.height,
        time_norm,
        time_sec,
    )?;

    Ok(camera_transform_from_values(
        x, y, anchor_x, anchor_y, zoom, rotation,
    ))
}

fn ensure_camera_2d(camera: &CameraNode) -> Result<(), MotionLoomSceneRenderError> {
    let mode = camera.mode.trim().to_ascii_lowercase();
    if mode.is_empty() || mode == "2d" {
        return Ok(());
    }
    Err(MotionLoomSceneRenderError::InvalidExpression {
        expr: format!("Camera mode={}", camera.mode),
        message: "only Camera mode=\"2d\" is supported in the scene renderer today".to_string(),
    })
}

fn camera_transform_from_values(
    x: f32,
    y: f32,
    anchor_x: f32,
    anchor_y: f32,
    zoom: f32,
    rotation: f32,
) -> Affine2 {
    Affine2::translate(anchor_x, anchor_y)
        .mul(Affine2::rotate_deg(rotation))
        .mul(Affine2::scale(zoom))
        .mul(Affine2::translate(-x, -y))
}

#[allow(clippy::too_many_arguments)]
fn camera_target(
    camera: &CameraNode,
    camera_children: &[SceneNode],
    canvas_w: u32,
    canvas_h: u32,
    viewport: CameraRect,
    zoom: f32,
    time_norm: f32,
    time_sec: f32,
) -> Result<(f32, f32), MotionLoomSceneRenderError> {
    let mut target = camera
        .follow
        .as_deref()
        .and_then(|id| {
            find_scene_node_anchor(
                camera_children,
                id,
                Affine2::identity(),
                time_norm,
                time_sec,
            )
        })
        .unwrap_or_else(|| (f32::NAN, f32::NAN));
    if !target.0.is_finite() || !target.1.is_finite() {
        target = (
            resolve_axis(&camera.x, canvas_w as f32, 0.0, time_norm, time_sec)?,
            resolve_axis(&camera.y, canvas_h as f32, 0.0, time_norm, time_sec)?,
        );
    }
    if let Some(target_x) = camera.target_x.as_deref() {
        target.0 = resolve_axis(target_x, canvas_w as f32, 0.0, time_norm, time_sec)?;
    }
    if let Some(target_y) = camera.target_y.as_deref() {
        target.1 = resolve_axis(target_y, canvas_h as f32, 0.0, time_norm, time_sec)?;
    }
    if let Some(bounds) = camera_world_bounds(camera, canvas_w, canvas_h, time_norm, time_sec)? {
        target = clamp_camera_target_to_bounds(target, bounds, viewport, zoom);
    }
    Ok(target)
}

fn clamp_camera_target_to_bounds(
    target: (f32, f32),
    bounds: CameraRect,
    viewport: CameraRect,
    zoom: f32,
) -> (f32, f32) {
    let half_w = viewport.width / zoom * 0.5;
    let half_h = viewport.height / zoom * 0.5;
    let min_x = bounds.x + half_w;
    let max_x = bounds.x + bounds.width - half_w;
    let min_y = bounds.y + half_h;
    let max_y = bounds.y + bounds.height - half_h;
    let x = if min_x <= max_x {
        target.0.clamp(min_x, max_x)
    } else {
        bounds.x + bounds.width * 0.5
    };
    let y = if min_y <= max_y {
        target.1.clamp(min_y, max_y)
    } else {
        bounds.y + bounds.height * 0.5
    };
    (x, y)
}

fn camera_viewport(
    camera: &CameraNode,
    canvas_w: u32,
    canvas_h: u32,
    time_norm: f32,
    time_sec: f32,
) -> Result<CameraRect, MotionLoomSceneRenderError> {
    if let Some(viewport) = camera.viewport.as_deref() {
        return parse_camera_rect_expr(viewport, time_norm, time_sec);
    }
    Ok(CameraRect {
        x: 0.0,
        y: 0.0,
        width: canvas_w as f32,
        height: canvas_h as f32,
    })
}

fn camera_world_bounds(
    camera: &CameraNode,
    _canvas_w: u32,
    _canvas_h: u32,
    time_norm: f32,
    time_sec: f32,
) -> Result<Option<CameraRect>, MotionLoomSceneRenderError> {
    camera
        .world_bounds
        .as_deref()
        .map(|bounds| parse_camera_rect_expr(bounds, time_norm, time_sec))
        .transpose()
}

fn parse_camera_rect_expr(
    raw: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<CameraRect, MotionLoomSceneRenderError> {
    let inner = raw
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err(MotionLoomSceneRenderError::InvalidExpression {
            expr: raw.to_string(),
            message: "camera rect must use x,y,width,height".to_string(),
        });
    }
    Ok(CameraRect {
        x: eval_scene_number(parts[0], time_norm, time_sec)?,
        y: eval_scene_number(parts[1], time_norm, time_sec)?,
        width: eval_scene_number(parts[2], time_norm, time_sec)?.max(1.0),
        height: eval_scene_number(parts[3], time_norm, time_sec)?.max(1.0),
    })
}

fn resolve_camera_anchor(
    raw: &str,
    viewport_origin: f32,
    viewport_extent: f32,
    time_norm: f32,
    time_sec: f32,
) -> Result<f32, MotionLoomSceneRenderError> {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    let offset = match lower.as_str() {
        "center" | "middle" => viewport_extent * 0.5,
        "left" | "top" => 0.0,
        "right" | "bottom" => viewport_extent,
        _ => {
            if let Some(percent) = lower.strip_suffix('%')
                && let Ok(value) = percent.trim().parse::<f32>()
            {
                viewport_extent * value / 100.0
            } else {
                let value = trimmed
                    .parse::<f32>()
                    .or_else(|_| eval_scene_number(trimmed, time_norm, time_sec))?;
                if (-1.0..=1.0).contains(&value) {
                    viewport_extent * value
                } else {
                    value
                }
            }
        }
    };
    Ok(viewport_origin + offset)
}

fn find_scene_node_anchor(
    nodes: &[SceneNode],
    id: &str,
    transform: Affine2,
    time_norm: f32,
    time_sec: f32,
) -> Option<(f32, f32)> {
    for node in nodes {
        match node {
            SceneNode::Rect(rect) if rect.id.as_deref() == Some(id) => {
                let x = eval_scene_number(&rect.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&rect.y, time_norm, time_sec).ok()?;
                let w = eval_scene_number(&rect.width, time_norm, time_sec).ok()?;
                let h = eval_scene_number(&rect.height, time_norm, time_sec).ok()?;
                return Some(transform.transform_point(x + w * 0.5, y + h * 0.5));
            }
            SceneNode::Circle(circle) if circle.id.as_deref() == Some(id) => {
                let x = eval_scene_number(&circle.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&circle.y, time_norm, time_sec).ok()?;
                return Some(transform.transform_point(x, y));
            }
            SceneNode::FaceJaw(face_jaw) if face_jaw.id.as_deref() == Some(id) => {
                let x = eval_scene_number(&face_jaw.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&face_jaw.y, time_norm, time_sec).ok()?;
                let h = eval_scene_number(&face_jaw.height, time_norm, time_sec).ok()?;
                return Some(transform.transform_point(x, y + h * 0.5));
            }
            SceneNode::Group(group) => {
                let x = eval_scene_number(&group.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&group.y, time_norm, time_sec).ok()?;
                let rotation = eval_scene_number(&group.rotation, time_norm, time_sec).ok()?;
                let scale = eval_scene_number(&group.scale, time_norm, time_sec)
                    .ok()?
                    .clamp(0.001, 64.0);
                let group_transform = transform
                    .mul(Affine2::translate(x, y))
                    .mul(Affine2::rotate_deg(rotation))
                    .mul(Affine2::scale(scale));
                if group.id.as_deref() == Some(id) {
                    return Some(group_transform.transform_point(0.0, 0.0));
                }
                if let Some(point) = find_scene_node_anchor(
                    &group.children,
                    id,
                    group_transform,
                    time_norm,
                    time_sec,
                ) {
                    return Some(point);
                }
            }
            SceneNode::Part(part) => {
                let x = eval_scene_number(&part.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&part.y, time_norm, time_sec).ok()?;
                let rotation = eval_scene_number(&part.rotation, time_norm, time_sec).ok()?;
                let scale = eval_scene_number(&part.scale, time_norm, time_sec)
                    .ok()?
                    .clamp(0.001, 64.0);
                let part_transform = transform
                    .mul(Affine2::translate(x, y))
                    .mul(Affine2::rotate_deg(rotation))
                    .mul(Affine2::scale(scale));
                if part.id.as_deref() == Some(id) {
                    return Some(part_transform.transform_point(0.0, 0.0));
                }
                if let Some(point) =
                    find_scene_node_anchor(&part.children, id, part_transform, time_norm, time_sec)
                {
                    return Some(point);
                }
            }
            SceneNode::Text(text) if text.id.as_deref() == Some(id) => {
                let x = eval_scene_number(&text.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&text.y, time_norm, time_sec).ok()?;
                return Some(transform.transform_point(x, y));
            }
            SceneNode::Image(image) if image.id.as_deref() == Some(id) => {
                let x = eval_scene_number(&image.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&image.y, time_norm, time_sec).ok()?;
                return Some(transform.transform_point(x, y));
            }
            SceneNode::Svg(svg) if svg.id.as_deref() == Some(id) => {
                let x = eval_scene_number(&svg.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&svg.y, time_norm, time_sec).ok()?;
                return Some(transform.transform_point(x, y));
            }
            SceneNode::Camera(camera) => {
                if let Some(point) =
                    find_scene_node_anchor(&camera.children, id, transform, time_norm, time_sec)
                {
                    return Some(point);
                }
            }
            SceneNode::Character(character) => {
                let x = eval_scene_number(&character.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&character.y, time_norm, time_sec).ok()?;
                let rotation = eval_scene_number(&character.rotation, time_norm, time_sec).ok()?;
                let scale = eval_scene_number(&character.scale, time_norm, time_sec)
                    .ok()?
                    .clamp(0.001, 64.0);
                let character_transform = transform
                    .mul(Affine2::translate(x, y))
                    .mul(Affine2::rotate_deg(rotation))
                    .mul(Affine2::scale(scale));
                if character.id.as_deref() == Some(id) {
                    return Some(character_transform.transform_point(0.0, 0.0));
                }
                if let Some(point) = find_scene_node_anchor(
                    &character.children,
                    id,
                    character_transform,
                    time_norm,
                    time_sec,
                ) {
                    return Some(point);
                }
            }
            SceneNode::Mask(mask) => {
                if mask.id.as_deref() == Some(id) {
                    let x = eval_scene_number(&mask.x, time_norm, time_sec).ok()?;
                    let y = eval_scene_number(&mask.y, time_norm, time_sec).ok()?;
                    return Some(transform.transform_point(x, y));
                }
                if let Some(point) =
                    find_scene_node_anchor(&mask.children, id, transform, time_norm, time_sec)
                {
                    return Some(point);
                }
            }
            _ => {}
        }
    }
    None
}

fn scene_nodes_for_present<'a>(graph: &'a GraphScript) -> Option<&'a [SceneNode]> {
    let present = graph.present.from.as_str();
    if present == "scene" {
        if !graph.scene_nodes.is_empty() {
            return Some(&graph.scene_nodes);
        }
        return graph.scenes.first().map(|scene| scene.children.as_slice());
    }
    let scene_id = present.strip_prefix("scene:").unwrap_or(present);
    graph
        .scenes
        .iter()
        .find(|scene| scene.id == scene_id)
        .map(|scene| scene.children.as_slice())
}

fn collect_graph_gradient_defs(graph: &GraphScript, out: &mut HashMap<String, GradientDef>) {
    collect_scene_gradient_defs(&graph.scene_nodes, out);
    for scene in &graph.scenes {
        collect_scene_gradient_defs(&scene.children, out);
    }
}

fn collect_scene_gradient_defs(nodes: &[SceneNode], out: &mut HashMap<String, GradientDef>) {
    for node in nodes {
        match node {
            SceneNode::Defs(defs) => {
                for gradient in &defs.gradients {
                    let id = match gradient {
                        GradientDef::Linear(linear) => &linear.id,
                        GradientDef::Radial(radial) => &radial.id,
                    };
                    out.insert(id.clone(), gradient.clone());
                }
            }
            SceneNode::Group(group) => collect_scene_gradient_defs(&group.children, out),
            SceneNode::Part(part) => collect_scene_gradient_defs(&part.children, out),
            SceneNode::Repeat(repeat) => collect_scene_gradient_defs(&repeat.children, out),
            SceneNode::Mask(mask) => collect_scene_gradient_defs(&mask.children, out),
            SceneNode::Camera(camera) => collect_scene_gradient_defs(&camera.children, out),
            SceneNode::Character(character) => {
                collect_scene_gradient_defs(&character.children, out)
            }
            _ => {}
        }
    }
}

fn scene_nodes_contain_image_or_svg(nodes: &[SceneNode]) -> bool {
    nodes.iter().any(|node| match node {
        SceneNode::Image(_) | SceneNode::Svg(_) => true,
        SceneNode::Group(group) => scene_nodes_contain_image_or_svg(&group.children),
        SceneNode::Part(part) => scene_nodes_contain_image_or_svg(&part.children),
        SceneNode::Repeat(repeat) => scene_nodes_contain_image_or_svg(&repeat.children),
        SceneNode::Camera(camera) => scene_nodes_contain_image_or_svg(&camera.children),
        SceneNode::Character(character) => scene_nodes_contain_image_or_svg(&character.children),
        SceneNode::Mask(mask) => scene_nodes_contain_image_or_svg(&mask.children),
        _ => false,
    })
}

fn collect_gpu_scene_commands(
    nodes: &[SceneNode],
    transform: Affine2,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    canvas_size: (u32, u32),
    gradient_defs: &HashMap<String, GradientDef>,
    primitives: &mut Vec<GpuScenePrimitive>,
    text_requests: &mut Vec<GpuSceneTextRequest>,
    scene_overlays: &mut Vec<CpuSceneOverlay>,
) -> Result<(), MotionLoomSceneRenderError> {
    let mut pending_shadow: Option<EvaluatedShadow> = None;
    for node in nodes {
        match node {
            SceneNode::Defs(_) => {
                pending_shadow = None;
            }
            SceneNode::Solid(solid) => {
                let mut color = parse_color(&solid.color)?;
                color[3] = ((color[3] as f32) * inherited_opacity)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                if transform.is_identity() {
                    primitives.push(GpuScenePrimitive {
                        kind: GPU_SHAPE_SOLID,
                        transform: Affine2::identity(),
                        shape: [0.0, 0.0, 0.0, 0.0],
                        radius: 0.0,
                        stroke_width: 0.0,
                        blur: 0.0,
                        color,
                        opacity: 1.0,
                        gradient: None,
                        line_t0: 0.0,
                        line_t1: 1.0,
                        taper_start: 0.0,
                        taper_end: 0.0,
                    });
                } else {
                    primitives.push(GpuScenePrimitive {
                        kind: GPU_SHAPE_RECT_FILL,
                        transform,
                        shape: [0.0, 0.0, canvas_size.0 as f32, canvas_size.1 as f32],
                        radius: 0.0,
                        stroke_width: 0.0,
                        blur: 0.0,
                        color,
                        opacity: 1.0,
                        gradient: None,
                        line_t0: 0.0,
                        line_t1: 1.0,
                        taper_start: 0.0,
                        taper_end: 0.0,
                    });
                }
                pending_shadow = None;
            }
            SceneNode::Text(text) => {
                text_requests.push(GpuSceneTextRequest {
                    node: text.clone(),
                    transform,
                    opacity: inherited_opacity,
                });
                pending_shadow = None;
            }
            SceneNode::Rect(rect) => {
                if rect_requires_cpu_overlay(rect) {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![SceneNode::Rect(rect.clone())],
                    });
                    pending_shadow = None;
                } else {
                    push_gpu_rect_commands(
                        rect,
                        transform,
                        pending_shadow.take(),
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        gradient_defs,
                        primitives,
                    )?;
                }
            }
            SceneNode::Circle(circle) => {
                if circle_requires_cpu_overlay(circle) {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![SceneNode::Circle(circle.clone())],
                    });
                    pending_shadow = None;
                } else {
                    push_gpu_circle_commands(
                        circle,
                        transform,
                        pending_shadow.take(),
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        gradient_defs,
                        primitives,
                    )?;
                }
            }
            SceneNode::Line(line) => {
                if line_requires_cpu_overlay(line) {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![node.clone()],
                    });
                } else {
                    push_gpu_line_command(
                        line,
                        transform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        gradient_defs,
                        primitives,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Polyline(polyline) => {
                if polyline_requires_cpu_overlay(polyline) {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![node.clone()],
                    });
                } else {
                    push_gpu_polyline_commands(
                        polyline,
                        transform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        gradient_defs,
                        primitives,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Path(path) => {
                if path_requires_cpu_overlay(path) {
                    scene_overlays.push(CpuSceneOverlay::Vector {
                        nodes: vec![node.clone()],
                    });
                } else {
                    push_gpu_path_commands(
                        path,
                        transform,
                        inherited_opacity,
                        time_norm,
                        time_sec,
                        gradient_defs,
                        primitives,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::FaceJaw(face_jaw) => {
                scene_overlays.push(CpuSceneOverlay::Vector {
                    nodes: vec![SceneNode::FaceJaw(face_jaw.clone())],
                });
                pending_shadow = None;
            }
            SceneNode::Shadow(shadow) => {
                pending_shadow = Some(evaluate_shadow(
                    shadow,
                    time_norm,
                    time_sec,
                    inherited_opacity,
                )?);
            }
            SceneNode::Group(group) => {
                let opacity = (eval_scene_number(&group.opacity, time_norm, time_sec)?
                    * inherited_opacity)
                    .clamp(0.0, 1.0);
                if opacity > 0.0001 {
                    let x = eval_scene_number(&group.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&group.y, time_norm, time_sec)?;
                    let rotation = eval_scene_number(&group.rotation, time_norm, time_sec)?;
                    let scale =
                        eval_scene_number(&group.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
                    let group_transform = transform
                        .mul(Affine2::translate(x, y))
                        .mul(Affine2::rotate_deg(rotation))
                        .mul(Affine2::scale(scale));
                    collect_gpu_scene_commands(
                        &group.children,
                        group_transform,
                        opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        gradient_defs,
                        primitives,
                        text_requests,
                        scene_overlays,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Part(part) => {
                let opacity = (eval_scene_number(&part.opacity, time_norm, time_sec)?
                    * inherited_opacity)
                    .clamp(0.0, 1.0);
                if opacity > 0.0001 {
                    let x = eval_scene_number(&part.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&part.y, time_norm, time_sec)?;
                    let rotation = eval_scene_number(&part.rotation, time_norm, time_sec)?;
                    let scale =
                        eval_scene_number(&part.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
                    let part_transform = transform
                        .mul(Affine2::translate(x, y))
                        .mul(Affine2::rotate_deg(rotation))
                        .mul(Affine2::scale(scale));
                    collect_gpu_scene_commands(
                        &part.children,
                        part_transform,
                        opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        gradient_defs,
                        primitives,
                        text_requests,
                        scene_overlays,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Repeat(repeat) => {
                let count = eval_repeat_count(&repeat.count, time_norm, time_sec)?;
                let x = eval_scene_number(&repeat.x, time_norm, time_sec)?;
                let y = eval_scene_number(&repeat.y, time_norm, time_sec)?;
                let rotation = eval_scene_number(&repeat.rotation, time_norm, time_sec)?;
                let scale =
                    eval_scene_number(&repeat.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
                let opacity = eval_scene_number(&repeat.opacity, time_norm, time_sec)?;
                let x_step = eval_scene_number(&repeat.x_step, time_norm, time_sec)?;
                let y_step = eval_scene_number(&repeat.y_step, time_norm, time_sec)?;
                let rotation_step = eval_scene_number(&repeat.rotation_step, time_norm, time_sec)?;
                let scale_step = eval_scene_number(&repeat.scale_step, time_norm, time_sec)?;
                let opacity_step = eval_scene_number(&repeat.opacity_step, time_norm, time_sec)?;
                for index in 0..count {
                    let i = index as f32;
                    let copy_opacity =
                        ((opacity + opacity_step * i) * inherited_opacity).clamp(0.0, 1.0);
                    if copy_opacity <= 0.0001 {
                        continue;
                    }
                    let repeat_transform = transform
                        .mul(Affine2::translate(x + x_step * i, y + y_step * i))
                        .mul(Affine2::rotate_deg(rotation + rotation_step * i))
                        .mul(Affine2::scale((scale + scale_step * i).clamp(0.001, 64.0)));
                    collect_gpu_scene_commands(
                        &repeat.children,
                        repeat_transform,
                        copy_opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        gradient_defs,
                        primitives,
                        text_requests,
                        scene_overlays,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Mask(mask) => {
                scene_overlays.push(CpuSceneOverlay::Vector {
                    nodes: vec![SceneNode::Mask(mask.clone())],
                });
                pending_shadow = None;
            }
            SceneNode::Camera(camera) => {
                let opacity = (eval_scene_number(&camera.opacity, time_norm, time_sec)?
                    * inherited_opacity)
                    .clamp(0.0, 1.0);
                if opacity > 0.0001 {
                    let camera_transform = camera_transform(
                        camera,
                        &camera.children,
                        canvas_size.0,
                        canvas_size.1,
                        time_norm,
                        time_sec,
                    )?;
                    collect_gpu_scene_commands(
                        &camera.children,
                        transform.mul(camera_transform),
                        opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        gradient_defs,
                        primitives,
                        text_requests,
                        scene_overlays,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Character(character) => {
                let opacity = (eval_scene_number(&character.opacity, time_norm, time_sec)?
                    * inherited_opacity)
                    .clamp(0.0, 1.0);
                if opacity > 0.0001 {
                    let x = eval_scene_number(&character.x, time_norm, time_sec)?;
                    let y = eval_scene_number(&character.y, time_norm, time_sec)?;
                    let rotation = eval_scene_number(&character.rotation, time_norm, time_sec)?;
                    let scale = eval_scene_number(&character.scale, time_norm, time_sec)?
                        .clamp(0.001, 64.0);
                    let character_transform = transform
                        .mul(Affine2::translate(x, y))
                        .mul(Affine2::rotate_deg(rotation))
                        .mul(Affine2::scale(scale));
                    collect_gpu_scene_commands(
                        &character.children,
                        character_transform,
                        opacity,
                        time_norm,
                        time_sec,
                        canvas_size,
                        gradient_defs,
                        primitives,
                        text_requests,
                        scene_overlays,
                    )?;
                }
                pending_shadow = None;
            }
            SceneNode::Image(_) | SceneNode::Svg(_) => {
                pending_shadow = None;
            }
        }
    }
    Ok(())
}

fn rect_requires_cpu_overlay(rect: &RectNode) -> bool {
    !is_normal_blend(&rect.blend)
}

fn circle_requires_cpu_overlay(circle: &CircleNode) -> bool {
    !is_normal_blend(&circle.blend)
}

fn line_requires_cpu_overlay(line: &LineNode) -> bool {
    !is_normal_blend(&line.blend) || !is_default_line_cap(&line.line_cap)
}

fn polyline_requires_cpu_overlay(polyline: &PolylineNode) -> bool {
    !is_normal_blend(&polyline.blend)
        || !is_default_line_cap(&polyline.line_cap)
        || !is_default_line_join(&polyline.line_join)
}

fn path_requires_cpu_overlay(path: &PathNode) -> bool {
    let has_visible_fill = path
        .fill
        .as_deref()
        .is_some_and(|fill| !is_none_paint(fill));
    !is_normal_blend(&path.blend)
        || !is_default_line_cap(&path.line_cap)
        || !is_default_line_join(&path.line_join)
        || (is_none_paint(&path.stroke) && !has_visible_fill)
}

fn is_default_line_cap(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.is_empty() || value == "round"
}

fn is_default_line_join(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.is_empty() || value == "round"
}

fn resolve_gpu_scene_paint(
    value: &str,
    gradient_defs: &HashMap<String, GradientDef>,
    bounds: PaintBounds,
) -> Result<([u8; 4], Option<GpuSceneGradientPaint>), MotionLoomSceneRenderError> {
    if let Some(id) = gradient_ref_id(value) {
        let Some(gradient) = gradient_defs.get(id) else {
            return Err(MotionLoomSceneRenderError::InvalidPaint {
                value: value.to_string(),
                message: format!("gradient reference not found: {id}"),
            });
        };
        return Ok((
            [255, 255, 255, 255],
            Some(GpuSceneGradientPaint {
                gradient: resolve_gradient_paint(value, gradient)?,
                bounds,
            }),
        ));
    }
    Ok((parse_color(value)?, None))
}

fn points_bounds(points: &[Point2]) -> Option<PaintBounds> {
    let mut iter = points.iter();
    let first = *iter.next()?;
    let mut min_x = first.x;
    let mut min_y = first.y;
    let mut max_x = first.x;
    let mut max_y = first.y;
    for point in iter {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    Some(PaintBounds::new(min_x, min_y, max_x, max_y))
}

fn subpaths_bounds(subpaths: &[Vec<Point2>]) -> Option<PaintBounds> {
    let mut bounds: Option<PaintBounds> = None;
    for subpath in subpaths {
        let Some(next) = points_bounds(subpath) else {
            continue;
        };
        bounds = Some(match bounds {
            Some(current) => PaintBounds::new(
                current.min_x.min(next.min_x),
                current.min_y.min(next.min_y),
                current.max_x.max(next.max_x),
                current.max_y.max(next.max_y),
            ),
            None => next,
        });
    }
    bounds
}

fn push_gpu_rect_commands(
    rect: &RectNode,
    transform: Affine2,
    shadow: Option<EvaluatedShadow>,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    gradient_defs: &HashMap<String, GradientDef>,
    primitives: &mut Vec<GpuScenePrimitive>,
) -> Result<(), MotionLoomSceneRenderError> {
    let opacity = (eval_scene_number(&rect.opacity, time_norm, time_sec)? * inherited_opacity)
        .clamp(0.0, 1.0);
    if opacity <= 0.0001 {
        return Ok(());
    }
    let x = eval_scene_number(&rect.x, time_norm, time_sec)?;
    let y = eval_scene_number(&rect.y, time_norm, time_sec)?;
    let width = eval_scene_number(&rect.width, time_norm, time_sec)?.max(0.0);
    let height = eval_scene_number(&rect.height, time_norm, time_sec)?.max(0.0);
    let radius = eval_scene_number(&rect.radius, time_norm, time_sec)?.max(0.0);
    let rotation = eval_scene_number(&rect.rotation, time_norm, time_sec)?;
    let shape_transform = transform.mul(Affine2::rotate_deg(rotation));
    let paint_bounds = PaintBounds::new(x, y, x + width, y + height);

    if let Some(shadow) = shadow {
        primitives.push(GpuScenePrimitive {
            kind: GPU_SHAPE_RECT_SHADOW,
            transform: shape_transform,
            shape: [x + shadow.x, y + shadow.y, width, height],
            radius,
            stroke_width: 0.0,
            blur: shadow.blur,
            color: shadow.color,
            opacity: 1.0,
            gradient: None,
            line_t0: 0.0,
            line_t1: 1.0,
            taper_start: 0.0,
            taper_end: 0.0,
        });
    }

    let (color, gradient) = resolve_gpu_scene_paint(&rect.color, gradient_defs, paint_bounds)?;
    primitives.push(GpuScenePrimitive {
        kind: GPU_SHAPE_RECT_FILL,
        transform: shape_transform,
        shape: [x, y, width, height],
        radius,
        stroke_width: 0.0,
        blur: 0.0,
        color,
        opacity,
        gradient,
        line_t0: 0.0,
        line_t1: 1.0,
        taper_start: 0.0,
        taper_end: 0.0,
    });

    if let Some(stroke_value) = rect
        .stroke
        .as_deref()
        .filter(|stroke| !is_none_paint(stroke))
    {
        let stroke_width = eval_scene_number(&rect.stroke_width, time_norm, time_sec)?.max(0.0);
        if stroke_width > 0.0 {
            let (stroke, gradient) =
                resolve_gpu_scene_paint(stroke_value, gradient_defs, paint_bounds)?;
            primitives.push(GpuScenePrimitive {
                kind: GPU_SHAPE_RECT_STROKE,
                transform: shape_transform,
                shape: [x, y, width, height],
                radius,
                stroke_width,
                blur: 0.0,
                color: stroke,
                opacity,
                gradient,
                line_t0: 0.0,
                line_t1: 1.0,
                taper_start: 0.0,
                taper_end: 0.0,
            });
        }
    }
    Ok(())
}

fn push_gpu_circle_commands(
    circle: &CircleNode,
    transform: Affine2,
    shadow: Option<EvaluatedShadow>,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    gradient_defs: &HashMap<String, GradientDef>,
    primitives: &mut Vec<GpuScenePrimitive>,
) -> Result<(), MotionLoomSceneRenderError> {
    let opacity = (eval_scene_number(&circle.opacity, time_norm, time_sec)? * inherited_opacity)
        .clamp(0.0, 1.0);
    if opacity <= 0.0001 {
        return Ok(());
    }
    let x = eval_scene_number(&circle.x, time_norm, time_sec)?;
    let y = eval_scene_number(&circle.y, time_norm, time_sec)?;
    let radius = eval_scene_number(&circle.radius, time_norm, time_sec)?.max(0.0);
    let paint_bounds = PaintBounds::new(x - radius, y - radius, x + radius, y + radius);

    if let Some(shadow) = shadow {
        primitives.push(GpuScenePrimitive {
            kind: GPU_SHAPE_CIRCLE_SHADOW,
            transform,
            shape: [x + shadow.x, y + shadow.y, radius, 0.0],
            radius: 0.0,
            stroke_width: 0.0,
            blur: shadow.blur,
            color: shadow.color,
            opacity: 1.0,
            gradient: None,
            line_t0: 0.0,
            line_t1: 1.0,
            taper_start: 0.0,
            taper_end: 0.0,
        });
    }

    let (color, gradient) = resolve_gpu_scene_paint(&circle.color, gradient_defs, paint_bounds)?;
    primitives.push(GpuScenePrimitive {
        kind: GPU_SHAPE_CIRCLE_FILL,
        transform,
        shape: [x, y, radius, 0.0],
        radius: 0.0,
        stroke_width: 0.0,
        blur: 0.0,
        color,
        opacity,
        gradient,
        line_t0: 0.0,
        line_t1: 1.0,
        taper_start: 0.0,
        taper_end: 0.0,
    });

    if let Some(stroke_value) = circle
        .stroke
        .as_deref()
        .filter(|stroke| !is_none_paint(stroke))
    {
        let stroke_width = eval_scene_number(&circle.stroke_width, time_norm, time_sec)?.max(0.0);
        if stroke_width > 0.0 {
            let (stroke, gradient) =
                resolve_gpu_scene_paint(stroke_value, gradient_defs, paint_bounds)?;
            primitives.push(GpuScenePrimitive {
                kind: GPU_SHAPE_CIRCLE_STROKE,
                transform,
                shape: [x, y, radius, 0.0],
                radius: 0.0,
                stroke_width,
                blur: 0.0,
                color: stroke,
                opacity,
                gradient,
                line_t0: 0.0,
                line_t1: 1.0,
                taper_start: 0.0,
                taper_end: 0.0,
            });
        }
    }
    Ok(())
}

fn push_gpu_line_command(
    line: &LineNode,
    transform: Affine2,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    gradient_defs: &HashMap<String, GradientDef>,
    primitives: &mut Vec<GpuScenePrimitive>,
) -> Result<(), MotionLoomSceneRenderError> {
    let opacity = (eval_scene_number(&line.opacity, time_norm, time_sec)? * inherited_opacity)
        .clamp(0.0, 1.0);
    if opacity <= 0.0001 {
        return Ok(());
    }

    let x1 = eval_scene_number(&line.x1, time_norm, time_sec)?;
    let y1 = eval_scene_number(&line.y1, time_norm, time_sec)?;
    let x2 = eval_scene_number(&line.x2, time_norm, time_sec)?;
    let y2 = eval_scene_number(&line.y2, time_norm, time_sec)?;
    let width = eval_scene_number(&line.width, time_norm, time_sec)?.max(0.0);
    if width <= 0.0001 {
        return Ok(());
    }

    let paint_bounds = PaintBounds::new(x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2));
    let (color, gradient) = resolve_gpu_scene_paint(&line.color, gradient_defs, paint_bounds)?;
    let style = eval_line_stroke_style(line, time_norm, time_sec)?;
    push_gpu_styled_line_primitives(
        primitives,
        transform,
        Point2::new(x1, y1),
        Point2::new(x2, y2),
        width,
        color,
        opacity,
        gradient,
        0.0,
        1.0,
        style,
    );
    Ok(())
}

fn push_gpu_polyline_commands(
    polyline: &PolylineNode,
    transform: Affine2,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    gradient_defs: &HashMap<String, GradientDef>,
    primitives: &mut Vec<GpuScenePrimitive>,
) -> Result<(), MotionLoomSceneRenderError> {
    let opacity = (eval_scene_number(&polyline.opacity, time_norm, time_sec)? * inherited_opacity)
        .clamp(0.0, 1.0);
    if opacity <= 0.0001 {
        return Ok(());
    }
    let width = eval_scene_number(&polyline.stroke_width, time_norm, time_sec)?.max(0.0);
    if width <= 0.0001 {
        return Ok(());
    }
    let points = parse_polyline_points(&polyline.points)?;
    let trim = evaluate_trim(
        &polyline.trim_start,
        &polyline.trim_end,
        time_norm,
        time_sec,
    )?;
    let paint_bounds =
        points_bounds(&points).unwrap_or_else(|| PaintBounds::new(0.0, 0.0, 1.0, 1.0));
    let (color, gradient) = resolve_gpu_scene_paint(&polyline.stroke, gradient_defs, paint_bounds)?;
    let style = eval_polyline_stroke_style(polyline, time_norm, time_sec)?;
    push_gpu_stroke_segments(
        primitives,
        transform,
        &[points],
        width,
        color,
        opacity,
        gradient,
        trim,
        style,
    );
    Ok(())
}

fn push_gpu_path_commands(
    path: &PathNode,
    transform: Affine2,
    inherited_opacity: f32,
    time_norm: f32,
    time_sec: f32,
    gradient_defs: &HashMap<String, GradientDef>,
    primitives: &mut Vec<GpuScenePrimitive>,
) -> Result<(), MotionLoomSceneRenderError> {
    let opacity = (eval_scene_number(&path.opacity, time_norm, time_sec)? * inherited_opacity)
        .clamp(0.0, 1.0);
    if opacity <= 0.0001 {
        return Ok(());
    }
    let subpaths = parse_path_subpaths(&path.d)?;
    let paint_bounds =
        subpaths_bounds(&subpaths).unwrap_or_else(|| PaintBounds::new(0.0, 0.0, 1.0, 1.0));
    if let Some(fill) = path.fill.as_deref().filter(|fill| !is_none_paint(fill)) {
        let (color, gradient) = resolve_gpu_scene_paint(fill, gradient_defs, paint_bounds)?;
        push_gpu_filled_path_triangles(primitives, transform, &subpaths, color, opacity, gradient);
    }

    let width = eval_scene_number(&path.stroke_width, time_norm, time_sec)?.max(0.0);
    if width > 0.0001 && !is_none_paint(&path.stroke) {
        let trim = evaluate_trim(&path.trim_start, &path.trim_end, time_norm, time_sec)?;
        let (color, gradient) = resolve_gpu_scene_paint(&path.stroke, gradient_defs, paint_bounds)?;
        let style = eval_path_stroke_style(path, time_norm, time_sec)?;
        push_gpu_stroke_segments(
            primitives, transform, &subpaths, width, color, opacity, gradient, trim, style,
        );
    }
    Ok(())
}

fn push_gpu_filled_path_triangles(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    subpaths: &[Vec<Point2>],
    color: [u8; 4],
    opacity: f32,
    gradient: Option<GpuSceneGradientPaint>,
) {
    for subpath in subpaths {
        for [a, b, c] in triangulate_polygon(subpath) {
            primitives.push(GpuScenePrimitive {
                kind: GPU_SHAPE_TRIANGLE_FILL,
                transform,
                shape: [a.x, a.y, b.x, b.y],
                radius: c.x,
                stroke_width: c.y,
                blur: 0.0,
                color,
                opacity,
                gradient: gradient.clone(),
                line_t0: 0.0,
                line_t1: 1.0,
                taper_start: 0.0,
                taper_end: 0.0,
            });
        }
    }
}

fn triangulate_polygon(points: &[Point2]) -> Vec<[Point2; 3]> {
    let polygon = sanitize_polygon(points);
    if polygon.len() < 3 {
        return Vec::new();
    }
    if polygon_area(&polygon).abs() <= 0.0001 {
        return Vec::new();
    }

    let ccw = polygon_area(&polygon) > 0.0;
    let mut indices: Vec<usize> = (0..polygon.len()).collect();
    let mut triangles = Vec::with_capacity(polygon.len().saturating_sub(2));
    let mut guard = 0usize;

    while indices.len() > 3 && guard < polygon.len().saturating_mul(polygon.len()).max(16) {
        guard += 1;
        let len = indices.len();
        let mut clipped = false;
        for i in 0..len {
            let prev = indices[(i + len - 1) % len];
            let curr = indices[i];
            let next = indices[(i + 1) % len];
            let a = polygon[prev];
            let b = polygon[curr];
            let c = polygon[next];
            if !is_convex_corner(a, b, c, ccw) {
                continue;
            }
            let contains_other = indices.iter().any(|&candidate| {
                candidate != prev
                    && candidate != curr
                    && candidate != next
                    && point_in_triangle(polygon[candidate], a, b, c)
            });
            if contains_other {
                continue;
            }
            triangles.push([a, b, c]);
            indices.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            return triangulate_polygon_fan(&polygon);
        }
    }

    if indices.len() == 3 {
        triangles.push([
            polygon[indices[0]],
            polygon[indices[1]],
            polygon[indices[2]],
        ]);
    }
    triangles
}

fn sanitize_polygon(points: &[Point2]) -> Vec<Point2> {
    let mut out = Vec::with_capacity(points.len());
    for &point in points {
        if out
            .last()
            .is_some_and(|last: &Point2| points_close(*last, point))
        {
            continue;
        }
        out.push(point);
    }
    if out.len() > 1
        && out
            .last()
            .zip(out.first())
            .is_some_and(|(last, first)| points_close(*last, *first))
    {
        out.pop();
    }
    out
}

fn triangulate_polygon_fan(points: &[Point2]) -> Vec<[Point2; 3]> {
    if points.len() < 3 {
        return Vec::new();
    }
    let mut triangles = Vec::with_capacity(points.len().saturating_sub(2));
    let a = points[0];
    for i in 1..points.len() - 1 {
        triangles.push([a, points[i], points[i + 1]]);
    }
    triangles
}

fn points_close(a: Point2, b: Point2) -> bool {
    (a.x - b.x).abs() <= 0.001 && (a.y - b.y).abs() <= 0.001
}

fn polygon_area(points: &[Point2]) -> f32 {
    let mut area = 0.0;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        area += a.x * b.y - b.x * a.y;
    }
    area * 0.5
}

fn is_convex_corner(a: Point2, b: Point2, c: Point2, ccw: bool) -> bool {
    let cross = cross_points(a, b, c);
    if ccw { cross > 0.001 } else { cross < -0.001 }
}

fn point_in_triangle(p: Point2, a: Point2, b: Point2, c: Point2) -> bool {
    let c0 = cross_points(a, b, p);
    let c1 = cross_points(b, c, p);
    let c2 = cross_points(c, a, p);
    let has_neg = c0 < -0.001 || c1 < -0.001 || c2 < -0.001;
    let has_pos = c0 > 0.001 || c1 > 0.001 || c2 > 0.001;
    !(has_neg && has_pos)
}

fn cross_points(a: Point2, b: Point2, c: Point2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn push_gpu_styled_line_primitives(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    opacity: f32,
    gradient: Option<GpuSceneGradientPaint>,
    line_t0: f32,
    line_t1: f32,
    style: StrokeStyle,
) {
    let copies = stroke_texture_copy_count(style);
    for copy_ix in 0..copies {
        let (start, end, width_scale, opacity_scale) =
            stroke_texture_variant(p0, p1, style, copy_ix);
        let stroke_width = (width * width_scale).max(0.01);
        let stroke_opacity = (opacity * opacity_scale).clamp(0.0, 1.0);
        if style.pressure_auto {
            push_gpu_pressure_line_primitives(
                primitives,
                transform,
                start,
                end,
                stroke_width,
                color,
                stroke_opacity,
                gradient.clone(),
                line_t0,
                line_t1,
                style,
            );
        } else {
            push_gpu_line_primitive(
                primitives,
                transform,
                start,
                end,
                stroke_width,
                color,
                stroke_opacity,
                gradient.clone(),
                line_t0,
                line_t1,
                style.taper_start,
                style.taper_end,
            );
        }
    }
    push_gpu_stroke_overlay_primitives(
        primitives, transform, p0, p1, width, color, opacity, line_t0, line_t1, style,
    );
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_line_primitive(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    opacity: f32,
    gradient: Option<GpuSceneGradientPaint>,
    line_t0: f32,
    line_t1: f32,
    taper_start: f32,
    taper_end: f32,
) {
    primitives.push(GpuScenePrimitive {
        kind: GPU_SHAPE_LINE,
        transform,
        shape: [p0.x, p0.y, p1.x, p1.y],
        radius: 0.0,
        stroke_width: width.max(0.01),
        blur: 0.0,
        color,
        opacity: opacity.clamp(0.0, 1.0),
        gradient,
        line_t0,
        line_t1,
        taper_start,
        taper_end,
    });
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_pressure_line_primitives(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    opacity: f32,
    gradient: Option<GpuSceneGradientPaint>,
    line_t0: f32,
    line_t1: f32,
    style: StrokeStyle,
) {
    let len = point_distance(p0, p1);
    if len <= 0.0001 {
        return;
    }
    let steps = (len / (width.max(1.0) * 4.0)).ceil().clamp(1.0, 8.0) as u32;
    for ix in 0..steps {
        let a_t = ix as f32 / steps as f32;
        let b_t = (ix + 1) as f32 / steps as f32;
        let mid_t = (a_t + b_t) * 0.5;
        let global_t = line_t0 + (line_t1 - line_t0) * mid_t;
        let pressure = stroke_taper_pressure(global_t, style).max(0.05);
        push_gpu_line_primitive(
            primitives,
            transform,
            p0.lerp(p1, a_t),
            p0.lerp(p1, b_t),
            width * pressure,
            color,
            opacity,
            gradient.clone(),
            0.0,
            1.0,
            0.0,
            0.0,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_stroke_overlay_primitives(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    opacity: f32,
    line_t0: f32,
    line_t1: f32,
    style: StrokeStyle,
) {
    if width <= 0.0 || opacity <= 0.0 {
        return;
    }
    if style.texture_strength > 0.001 {
        push_gpu_stroke_stamp_primitives(
            primitives, transform, p0, p1, width, color, opacity, line_t0, line_t1, style,
        );
    }
    if style.bristles > 0 {
        push_gpu_stroke_bristle_primitives(
            primitives, transform, p0, p1, width, color, opacity, line_t0, line_t1, style,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_stroke_stamp_primitives(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    opacity: f32,
    line_t0: f32,
    line_t1: f32,
    style: StrokeStyle,
) {
    let len = point_distance(p0, p1);
    if len <= 0.0001 {
        return;
    }
    let strength = style.texture_strength.clamp(0.0, 1.0);
    let dx = (p1.x - p0.x) / len;
    let dy = (p1.y - p0.y) / len;
    let nx = -dy;
    let ny = dx;
    let spacing = (width * (1.35 - strength * 0.65)).clamp(2.0, 18.0);
    let steps = (len / spacing).ceil().clamp(1.0, 72.0) as u32;
    let texture_size = match style.texture {
        StrokeTexture::Charcoal => 1.65,
        StrokeTexture::Rough => 1.25,
        StrokeTexture::Pencil => 1.0,
        StrokeTexture::Sketch => 0.82,
        StrokeTexture::Marker => 0.72,
        StrokeTexture::Ink => 0.55,
        StrokeTexture::Hairline | StrokeTexture::Solid => 0.46,
    };
    let alpha_scale = match style.texture {
        StrokeTexture::Charcoal => 0.18,
        StrokeTexture::Pencil => 0.16,
        StrokeTexture::Rough => 0.14,
        StrokeTexture::Sketch => 0.12,
        StrokeTexture::Marker => 0.10,
        StrokeTexture::Ink => 0.08,
        StrokeTexture::Hairline | StrokeTexture::Solid => 0.06,
    };
    for step in 0..steps {
        let seed = stroke_texture_seed(p0, p1, step + 271);
        let keep = ((stroke_hash_signed(seed + 13.1) + 1.0) * 0.5).clamp(0.0, 1.0);
        if keep > strength {
            continue;
        }
        let local_t = ((step as f32 + 0.5) / steps as f32).clamp(0.0, 1.0);
        let global_t = line_t0 + (line_t1 - line_t0) * local_t;
        let pressure = stroke_taper_pressure(global_t, style).max(0.05);
        let tangent_noise = stroke_hash_signed(seed + 37.7) * spacing * 0.25;
        let normal_noise = stroke_hash_signed(seed + 91.3) * width * pressure * 0.45;
        let p = p0.lerp(p1, local_t);
        let size_noise = ((stroke_hash_signed(seed + 163.0) + 1.0) * 0.5).clamp(0.0, 1.0);
        let radius = (width * pressure * (0.035 + size_noise * 0.10) * texture_size).max(0.35);
        primitives.push(GpuScenePrimitive {
            kind: GPU_SHAPE_CIRCLE_FILL,
            transform,
            shape: [
                p.x + dx * tangent_noise + nx * normal_noise,
                p.y + dy * tangent_noise + ny * normal_noise,
                radius,
                0.0,
            ],
            radius: 0.0,
            stroke_width: 0.0,
            blur: 0.0,
            color,
            opacity: (opacity * strength * alpha_scale).clamp(0.0, 1.0),
            gradient: None,
            line_t0: 0.0,
            line_t1: 1.0,
            taper_start: 0.0,
            taper_end: 0.0,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn push_gpu_stroke_bristle_primitives(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    opacity: f32,
    line_t0: f32,
    line_t1: f32,
    style: StrokeStyle,
) {
    let len = point_distance(p0, p1);
    if len <= 0.0001 {
        return;
    }
    let dx = (p1.x - p0.x) / len;
    let dy = (p1.y - p0.y) / len;
    let nx = -dy;
    let ny = dx;
    let count = style.bristles.clamp(0, 24);
    let pressure =
        ((stroke_taper_pressure(line_t0, style) + stroke_taper_pressure(line_t1, style)) * 0.5)
            .max(0.05);
    let bristle_width = (width * 0.08 * pressure).clamp(0.25, 2.2);
    let alpha_scale = match style.texture {
        StrokeTexture::Charcoal => 0.20,
        StrokeTexture::Rough => 0.18,
        StrokeTexture::Pencil => 0.15,
        StrokeTexture::Sketch => 0.13,
        _ => 0.11,
    };
    for ix in 0..count {
        let lane = if count <= 1 {
            0.0
        } else {
            ix as f32 / (count - 1) as f32 * 2.0 - 1.0
        };
        let seed = stroke_texture_seed(p0, p1, ix + 997);
        let offset = lane * width * pressure * 0.42
            + stroke_hash_signed(seed + 21.0) * style.roughness * 0.55;
        let start_t = (stroke_hash_signed(seed + 57.0) * 0.04).max(0.0);
        let end_t = 1.0 - (stroke_hash_signed(seed + 83.0) * 0.04).max(0.0);
        let start = p0.lerp(p1, start_t);
        let end = p0.lerp(p1, end_t);
        push_gpu_line_primitive(
            primitives,
            transform,
            Point2::new(start.x + nx * offset, start.y + ny * offset),
            Point2::new(end.x + nx * offset, end.y + ny * offset),
            bristle_width,
            color,
            (opacity * alpha_scale).clamp(0.0, 1.0),
            None,
            0.0,
            1.0,
            0.0,
            0.0,
        );
    }
}

fn push_gpu_stroke_segments(
    primitives: &mut Vec<GpuScenePrimitive>,
    transform: Affine2,
    subpaths: &[Vec<Point2>],
    width: f32,
    color: [u8; 4],
    opacity: f32,
    gradient: Option<GpuSceneGradientPaint>,
    trim: (f32, f32),
    style: StrokeStyle,
) {
    for segment in trimmed_polyline_segments_with_progress(subpaths, trim) {
        push_gpu_styled_line_primitives(
            primitives,
            transform,
            segment.p0,
            segment.p1,
            width,
            color,
            opacity,
            gradient.clone(),
            segment.t0,
            segment.t1,
            style,
        );
    }
}

fn primitive_bounds(
    primitive: &GpuScenePrimitive,
    canvas_w: u32,
    canvas_h: u32,
) -> Option<(u32, u32, u32, u32)> {
    if primitive.kind == GPU_SHAPE_SOLID {
        return Some((0, 0, canvas_w.max(1), canvas_h.max(1)));
    }

    let (min_x, min_y, max_x, max_y) = local_primitive_bbox(primitive)?;
    let points = [
        primitive.transform.transform_point(min_x, min_y),
        primitive.transform.transform_point(max_x, min_y),
        primitive.transform.transform_point(max_x, max_y),
        primitive.transform.transform_point(min_x, max_y),
    ];
    let mut bx0 = f32::INFINITY;
    let mut by0 = f32::INFINITY;
    let mut bx1 = f32::NEG_INFINITY;
    let mut by1 = f32::NEG_INFINITY;
    for (x, y) in points {
        bx0 = bx0.min(x);
        by0 = by0.min(y);
        bx1 = bx1.max(x);
        by1 = by1.max(y);
    }
    let pad = 3.0;
    let x0 = (bx0 - pad).floor().clamp(0.0, canvas_w as f32) as u32;
    let y0 = (by0 - pad).floor().clamp(0.0, canvas_h as f32) as u32;
    let x1 = (bx1 + pad).ceil().clamp(0.0, canvas_w as f32) as u32;
    let y1 = (by1 + pad).ceil().clamp(0.0, canvas_h as f32) as u32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((x0, y0, x1 - x0, y1 - y0))
}

fn local_primitive_bbox(primitive: &GpuScenePrimitive) -> Option<(f32, f32, f32, f32)> {
    let s = primitive.shape;
    let spread =
        if primitive.kind == GPU_SHAPE_RECT_SHADOW || primitive.kind == GPU_SHAPE_CIRCLE_SHADOW {
            primitive.blur * 1.8
        } else {
            primitive.stroke_width.max(0.0) + 2.0
        };
    if primitive.kind == GPU_SHAPE_RECT_FILL
        || primitive.kind == GPU_SHAPE_RECT_STROKE
        || primitive.kind == GPU_SHAPE_RECT_SHADOW
    {
        return Some((
            s[0] - spread,
            s[1] - spread,
            s[0] + s[2] + spread,
            s[1] + s[3] + spread,
        ));
    }
    if primitive.kind == GPU_SHAPE_CIRCLE_FILL
        || primitive.kind == GPU_SHAPE_CIRCLE_STROKE
        || primitive.kind == GPU_SHAPE_CIRCLE_SHADOW
    {
        let r = s[2] + spread;
        return Some((s[0] - r, s[1] - r, s[0] + r, s[1] + r));
    }
    if primitive.kind == GPU_SHAPE_LINE {
        let spread = primitive.stroke_width.max(1.0) * 0.5 + 2.0;
        return Some((
            s[0].min(s[2]) - spread,
            s[1].min(s[3]) - spread,
            s[0].max(s[2]) + spread,
            s[1].max(s[3]) + spread,
        ));
    }
    if primitive.kind == GPU_SHAPE_TRIANGLE_FILL {
        let spread = 2.0;
        return Some((
            s[0].min(s[2]).min(primitive.radius) - spread,
            s[1].min(s[3]).min(primitive.stroke_width) - spread,
            s[0].max(s[2]).max(primitive.radius) + spread,
            s[1].max(s[3]).max(primitive.stroke_width) + spread,
        ));
    }
    None
}

fn write_gpu_gradient_uniform(gradient: Option<&GpuSceneGradientPaint>, values: &mut [f32]) {
    debug_assert!(values.len() >= 52);
    let Some(gradient) = gradient else {
        return;
    };

    let (paint_kind, units, params, stops) = match &gradient.gradient {
        ResolvedGradient::Linear {
            x1,
            y1,
            x2,
            y2,
            stops,
            units,
        } => (
            1.0,
            gpu_gradient_units(*units),
            [*x1, *y1, *x2, *y2],
            stops.as_slice(),
        ),
        ResolvedGradient::Radial {
            cx,
            cy,
            r,
            stops,
            units,
        } => (
            2.0,
            gpu_gradient_units(*units),
            [*cx, *cy, *r, 0.0],
            stops.as_slice(),
        ),
    };

    let stop_count = stops.len().min(8);
    values[0..4].copy_from_slice(&[paint_kind, units, stop_count as f32, 0.0]);
    values[4..8].copy_from_slice(&[
        gradient.bounds.min_x,
        gradient.bounds.min_y,
        gradient.bounds.max_x,
        gradient.bounds.max_y,
    ]);
    values[8..12].copy_from_slice(&params);
    for (index, stop) in stops.iter().take(8).enumerate() {
        values[12 + index] = stop.offset;
        let color = rgba_u8_to_unit(stop.color);
        let color_offset = 20 + index * 4;
        values[color_offset..color_offset + 4].copy_from_slice(&color);
    }
}

fn gpu_gradient_units(units: GradientUnits) -> f32 {
    match units {
        GradientUnits::ObjectBoundingBox => 0.0,
        GradientUnits::UserSpace => 1.0,
    }
}

const GPU_BATCH_TILE_SIZE: u32 = 32;

struct BatchedShapeData {
    primitive_bytes: Vec<u8>,
    primitive_count: u32,
    tile_range_bytes: Vec<u8>,
    tile_index_bytes: Vec<u8>,
    tile_size: u32,
    tiles_x: u32,
    tiles_y: u32,
}

fn batch_shape_uniform(
    canvas_w: u32,
    canvas_h: u32,
    primitive_count: u32,
    tile_size: u32,
    tiles_x: u32,
    tiles_y: u32,
) -> [u8; 32] {
    f32_bytes(&[
        canvas_w as f32,
        canvas_h as f32,
        0.0,
        0.0,
        primitive_count as f32,
        tile_size as f32,
        tiles_x as f32,
        tiles_y as f32,
    ])
}

fn batch_shape_storage_bytes(
    primitives: &[GpuScenePrimitive],
    canvas_w: u32,
    canvas_h: u32,
) -> Result<BatchedShapeData, MotionLoomSceneRenderError> {
    let tile_size = GPU_BATCH_TILE_SIZE.max(1);
    let tiles_x = canvas_w.max(1).div_ceil(tile_size);
    let tiles_y = canvas_h.max(1).div_ceil(tile_size);
    let tile_count = tiles_x.saturating_mul(tiles_y) as usize;
    let mut tile_buckets = vec![Vec::<u32>::new(); tile_count];
    let mut primitive_bytes = Vec::with_capacity(primitives.len().saturating_mul(84 * 4));
    let mut primitive_count = 0_u32;

    for primitive in primitives {
        let Some((bounds_x, bounds_y, bounds_w, bounds_h)) =
            primitive_bounds(primitive, canvas_w, canvas_h)
        else {
            continue;
        };
        if bounds_w == 0 || bounds_h == 0 {
            continue;
        }
        let values =
            batch_shape_primitive_values(primitive, bounds_x, bounds_y, bounds_w, bounds_h)?;
        push_f32_bytes(&mut primitive_bytes, &values);

        let x0 = (bounds_x / tile_size).min(tiles_x.saturating_sub(1));
        let y0 = (bounds_y / tile_size).min(tiles_y.saturating_sub(1));
        let x1 = ((bounds_x.saturating_add(bounds_w).saturating_sub(1)) / tile_size)
            .min(tiles_x.saturating_sub(1));
        let y1 = ((bounds_y.saturating_add(bounds_h).saturating_sub(1)) / tile_size)
            .min(tiles_y.saturating_sub(1));
        for tile_y in y0..=y1 {
            for tile_x in x0..=x1 {
                let tile_ix = tile_y.saturating_mul(tiles_x).saturating_add(tile_x) as usize;
                if let Some(bucket) = tile_buckets.get_mut(tile_ix) {
                    bucket.push(primitive_count);
                }
            }
        }

        primitive_count = primitive_count.saturating_add(1);
    }

    let mut tile_range_bytes = Vec::with_capacity(tile_count.saturating_mul(16));
    let mut tile_index_bytes = Vec::new();
    let mut index_offset = 0_u32;
    for bucket in tile_buckets {
        push_u32_bytes(
            &mut tile_range_bytes,
            &[index_offset, bucket.len() as u32, 0, 0],
        );
        for primitive_ix in bucket {
            push_u32_bytes(&mut tile_index_bytes, &[primitive_ix]);
        }
        index_offset = (tile_index_bytes.len() / 4) as u32;
    }

    if tile_index_bytes.is_empty() {
        push_u32_bytes(&mut tile_index_bytes, &[0]);
    }

    Ok(BatchedShapeData {
        primitive_bytes,
        primitive_count,
        tile_range_bytes,
        tile_index_bytes,
        tile_size,
        tiles_x,
        tiles_y,
    })
}

fn batch_shape_primitive_values(
    primitive: &GpuScenePrimitive,
    bounds_x: u32,
    bounds_y: u32,
    bounds_w: u32,
    bounds_h: u32,
) -> Result<[f32; 84], MotionLoomSceneRenderError> {
    let inverse =
        primitive
            .transform
            .inverse()
            .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                message: "shape transform is not invertible".to_string(),
            })?;
    let color = rgba_u8_to_unit(primitive.color);
    let mut values = [0.0_f32; 84];
    values[..28].copy_from_slice(&[
        primitive.kind,
        0.0,
        0.0,
        0.0,
        bounds_x as f32,
        bounds_y as f32,
        bounds_w as f32,
        bounds_h as f32,
        primitive.shape[0],
        primitive.shape[1],
        primitive.shape[2],
        primitive.shape[3],
        primitive.radius,
        primitive.stroke_width,
        primitive.blur,
        primitive.opacity,
        color[0],
        color[1],
        color[2],
        color[3],
        inverse.m00,
        inverse.m01,
        inverse.m02,
        0.0,
        inverse.m10,
        inverse.m11,
        inverse.m12,
        0.0,
    ]);
    write_gpu_gradient_uniform(primitive.gradient.as_ref(), &mut values[28..80]);
    values[80..84].copy_from_slice(&[
        primitive.line_t0,
        primitive.line_t1,
        primitive.taper_start,
        primitive.taper_end,
    ]);
    Ok(values)
}

fn push_f32_bytes(out: &mut Vec<u8>, values: &[f32]) {
    out.reserve(values.len().saturating_mul(4));
    for value in values {
        out.extend_from_slice(&value.to_ne_bytes());
    }
}

fn push_u32_bytes(out: &mut Vec<u8>, values: &[u32]) {
    out.reserve(values.len().saturating_mul(4));
    for value in values {
        out.extend_from_slice(&value.to_ne_bytes());
    }
}

fn f32_bytes<const N: usize>(values: &[f32]) -> [u8; N] {
    debug_assert_eq!(N, values.len().saturating_mul(4));
    let mut bytes = [0u8; N];
    for (ix, value) in values.iter().enumerate() {
        bytes[ix * 4..ix * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

fn texture_layer_bounds(
    transform: Affine2,
    width: u32,
    height: u32,
    canvas_w: u32,
    canvas_h: u32,
) -> Option<(u32, u32, u32, u32)> {
    let w = width as f32;
    let h = height as f32;
    let points = [
        transform.transform_point(0.0, 0.0),
        transform.transform_point(w, 0.0),
        transform.transform_point(w, h),
        transform.transform_point(0.0, h),
    ];
    let mut bx0 = f32::INFINITY;
    let mut by0 = f32::INFINITY;
    let mut bx1 = f32::NEG_INFINITY;
    let mut by1 = f32::NEG_INFINITY;
    for (x, y) in points {
        bx0 = bx0.min(x);
        by0 = by0.min(y);
        bx1 = bx1.max(x);
        by1 = by1.max(y);
    }
    let pad = 2.0;
    let x0 = (bx0 - pad).floor().clamp(0.0, canvas_w as f32) as u32;
    let y0 = (by0 - pad).floor().clamp(0.0, canvas_h as f32) as u32;
    let x1 = (bx1 + pad).ceil().clamp(0.0, canvas_w as f32) as u32;
    let y1 = (by1 + pad).ceil().clamp(0.0, canvas_h as f32) as u32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((x0, y0, x1 - x0, y1 - y0))
}

fn affine_texture_uniform(
    layer: &GpuSceneTextureLayer,
    canvas_w: u32,
    canvas_h: u32,
    bounds_x: u32,
    bounds_y: u32,
    bounds_w: u32,
    bounds_h: u32,
) -> Result<[u8; 96], MotionLoomSceneRenderError> {
    let inverse =
        layer
            .transform
            .inverse()
            .ok_or_else(|| MotionLoomSceneRenderError::GpuRender {
                message: "texture transform is not invertible".to_string(),
            })?;
    let values = [
        canvas_w as f32,
        canvas_h as f32,
        0.0,
        0.0,
        bounds_x as f32,
        bounds_y as f32,
        bounds_w as f32,
        bounds_h as f32,
        layer.image.width() as f32,
        layer.image.height() as f32,
        0.0,
        0.0,
        layer.opacity,
        0.0,
        0.0,
        0.0,
        inverse.m00,
        inverse.m01,
        inverse.m02,
        0.0,
        inverse.m10,
        inverse.m11,
        inverse.m12,
        0.0,
    ];
    let mut uniform = [0u8; 96];
    for (ix, value) in values.iter().enumerate() {
        uniform[ix * 4..ix * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
    Ok(uniform)
}

fn post_blur_uniform(canvas_w: u32, canvas_h: u32, horizontal: bool, sigma: f32) -> [u8; 32] {
    let values = [
        canvas_w as f32,
        canvas_h as f32,
        0.0,
        0.0,
        if horizontal { 0.0 } else { 1.0 },
        sigma.ceil().clamp(0.0, 64.0),
        0.0,
        0.0,
    ];
    let mut uniform = [0u8; 32];
    for (ix, value) in values.iter().enumerate() {
        uniform[ix * 4..ix * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
    uniform
}

fn rgba_u8_to_unit(color: [u8; 4]) -> [f32; 4] {
    [
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
        color[3] as f32 / 255.0,
    ]
}

fn graph_has_rich_scene_tree(graph: &GraphScript) -> bool {
    !graph.scenes.is_empty() || graph.scene_nodes.iter().any(scene_node_is_rich)
}

fn scene_node_is_rich(node: &SceneNode) -> bool {
    match node {
        SceneNode::Defs(_)
        | SceneNode::Solid(_)
        | SceneNode::Text(_)
        | SceneNode::Image(_)
        | SceneNode::Svg(_) => false,
        SceneNode::Rect(_)
        | SceneNode::Circle(_)
        | SceneNode::Line(_)
        | SceneNode::Polyline(_)
        | SceneNode::Path(_)
        | SceneNode::FaceJaw(_)
        | SceneNode::Shadow(_)
        | SceneNode::Mask(_) => true,
        SceneNode::Group(group) => group.children.iter().any(scene_node_is_rich),
        SceneNode::Part(part) => part.children.iter().any(scene_node_is_rich),
        SceneNode::Repeat(repeat) => repeat.children.iter().any(scene_node_is_rich),
        SceneNode::Camera(_) | SceneNode::Character(_) => true,
    }
}

fn apply_scene_post_pass(
    input: &RgbaImage,
    pass: &PassNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<RgbaImage, MotionLoomSceneRenderError> {
    let effect = pass.effect.to_ascii_lowercase();
    if effect == "opacity" || effect == "composite.opacity" {
        let opacity = pass_param_expr(pass, "opacity")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        return Ok(apply_opacity_pass(input, opacity));
    }
    if effect.contains("gaussian_5tap_h") || effect.contains("gaussian_h") {
        let sigma = pass_param_expr(pass, "sigma")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(2.0)
            .clamp(0.0, 64.0);
        return Ok(apply_box_blur_pass(input, sigma, true));
    }
    if effect.contains("gaussian_5tap_v") || effect.contains("gaussian_v") {
        let sigma = pass_param_expr(pass, "sigma")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(2.0)
            .clamp(0.0, 64.0);
        return Ok(apply_box_blur_pass(input, sigma, false));
    }
    if effect == "color_core" || effect == "color_blur" {
        let brightness = pass_param_expr(pass, "brightness")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(0.0)
            .clamp(-1.0, 1.0);
        let contrast = pass_param_expr(pass, "contrast")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(1.0)
            .clamp(0.0, 2.0);
        let saturation = pass_param_expr(pass, "saturation")
            .map(|expr| eval_scene_number(expr, time_norm, time_sec))
            .transpose()?
            .unwrap_or(1.0)
            .clamp(0.0, 2.0);
        return Ok(apply_color_core_pass(
            input, brightness, contrast, saturation,
        ));
    }
    Ok(input.clone())
}

fn scene_post_blur_params(
    pass: &PassNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Option<(bool, f32)>, MotionLoomSceneRenderError> {
    let effect = pass.effect.to_ascii_lowercase();
    let horizontal = if effect.contains("gaussian_5tap_h") || effect.contains("gaussian_h") {
        true
    } else if effect.contains("gaussian_5tap_v") || effect.contains("gaussian_v") {
        false
    } else {
        return Ok(None);
    };
    let sigma = pass_param_expr(pass, "sigma")
        .map(|expr| eval_scene_number(expr, time_norm, time_sec))
        .transpose()?
        .unwrap_or(2.0)
        .clamp(0.0, 64.0);
    Ok(Some((horizontal, sigma)))
}

fn pass_param_expr<'a>(pass: &'a PassNode, key: &str) -> Option<&'a str> {
    pass.params
        .iter()
        .find(|param| param.key.eq_ignore_ascii_case(key))
        .map(|param| param.value.as_str())
}

fn apply_opacity_pass(input: &RgbaImage, opacity: f32) -> RgbaImage {
    let mut out = input.clone();
    for pixel in out.pixels_mut() {
        pixel[3] = ((pixel[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
    }
    out
}

fn apply_box_blur_pass(input: &RgbaImage, sigma: f32, horizontal: bool) -> RgbaImage {
    if sigma <= 0.001 {
        return input.clone();
    }
    let radius = sigma.ceil().clamp(1.0, 64.0) as i32;
    let mut out = RgbaImage::from_pixel(input.width(), input.height(), Rgba([0, 0, 0, 0]));
    for y in 0..input.height() {
        for x in 0..input.width() {
            let mut acc = [0.0_f32; 4];
            let mut weight_sum = 0.0_f32;
            for offset in -radius..=radius {
                let (sx, sy) = if horizontal {
                    (
                        (x as i32 + offset).clamp(0, input.width() as i32 - 1) as u32,
                        y,
                    )
                } else {
                    (
                        x,
                        (y as i32 + offset).clamp(0, input.height() as i32 - 1) as u32,
                    )
                };
                let distance = offset as f32;
                let weight = (-(distance * distance) / (2.0 * sigma.max(0.001).powi(2))).exp();
                let pixel = input.get_pixel(sx, sy);
                for channel in 0..4 {
                    acc[channel] += pixel[channel] as f32 * weight;
                }
                weight_sum += weight;
            }
            let mut rgba = [0_u8; 4];
            for channel in 0..4 {
                rgba[channel] = (acc[channel] / weight_sum).round().clamp(0.0, 255.0) as u8;
            }
            *out.get_pixel_mut(x, y) = Rgba(rgba);
        }
    }
    out
}

fn apply_color_core_pass(
    input: &RgbaImage,
    brightness: f32,
    contrast: f32,
    saturation: f32,
) -> RgbaImage {
    let mut out = input.clone();
    for pixel in out.pixels_mut() {
        let a = pixel[3];
        let mut r = pixel[0] as f32 / 255.0;
        let mut g = pixel[1] as f32 / 255.0;
        let mut b = pixel[2] as f32 / 255.0;
        let luma = r * 0.2126 + g * 0.7152 + b * 0.0722;
        r = luma + (r - luma) * saturation;
        g = luma + (g - luma) * saturation;
        b = luma + (b - luma) * saturation;
        r = (r - 0.5) * contrast + 0.5 + brightness;
        g = (g - 0.5) * contrast + 0.5 + brightness;
        b = (b - 0.5) * contrast + 0.5 + brightness;
        pixel[0] = (r * 255.0).round().clamp(0.0, 255.0) as u8;
        pixel[1] = (g * 255.0).round().clamp(0.0, 255.0) as u8;
        pixel[2] = (b * 255.0).round().clamp(0.0, 255.0) as u8;
        pixel[3] = a;
    }
    out
}

fn evaluate_shadow(
    shadow: &ShadowNode,
    time_norm: f32,
    time_sec: f32,
    inherited_opacity: f32,
) -> Result<EvaluatedShadow, MotionLoomSceneRenderError> {
    let mut color = parse_color(&shadow.color)?;
    let opacity = (eval_scene_number(&shadow.opacity, time_norm, time_sec)? * inherited_opacity)
        .clamp(0.0, 1.0);
    color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
    Ok(EvaluatedShadow {
        x: eval_scene_number(&shadow.x, time_norm, time_sec)?,
        y: eval_scene_number(&shadow.y, time_norm, time_sec)?,
        blur: eval_scene_number(&shadow.blur, time_norm, time_sec)?.max(0.0),
        color,
        opacity,
    })
}

fn draw_rect_shadow(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    shadow: &EvaluatedShadow,
) {
    let steps = (shadow.blur / 6.0).ceil().clamp(1.0, 10.0) as u32;
    for step in (0..steps).rev() {
        let t = (step + 1) as f32 / steps as f32;
        let spread = shadow.blur * t * 0.45;
        let mut color = shadow.color;
        color[3] = ((color[3] as f32) * shadow.opacity * (1.0 - t * 0.82))
            .round()
            .clamp(0.0, 255.0) as u8;
        if color[3] == 0 {
            continue;
        }
        draw_rounded_rect(
            canvas,
            x + shadow.x - spread,
            y + shadow.y - spread,
            width + spread * 2.0,
            height + spread * 2.0,
            radius + spread,
            color,
        );
    }
}

fn draw_circle_shadow(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    radius: f32,
    shadow: &EvaluatedShadow,
) {
    let steps = (shadow.blur / 6.0).ceil().clamp(1.0, 10.0) as u32;
    for step in (0..steps).rev() {
        let t = (step + 1) as f32 / steps as f32;
        let spread = shadow.blur * t * 0.45;
        let mut color = shadow.color;
        color[3] = ((color[3] as f32) * shadow.opacity * (1.0 - t * 0.82))
            .round()
            .clamp(0.0, 255.0) as u8;
        if color[3] == 0 {
            continue;
        }
        draw_circle(canvas, x + shadow.x, y + shadow.y, radius + spread, color);
    }
}

fn draw_rounded_rect(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: [u8; 4],
) {
    if width <= 0.0 || height <= 0.0 || color[3] == 0 {
        return;
    }
    let min_x = x.floor().max(0.0) as u32;
    let min_y = y.floor().max(0.0) as u32;
    let max_x = (x + width).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + height).ceil().min(canvas.height() as f32) as u32;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let cx = px as f32 + 0.5;
            let cy = py as f32 + 0.5;
            if rounded_rect_contains(cx, cy, x, y, width, height, radius) {
                blend_pixel(canvas, px, py, color);
            }
        }
    }
}

fn draw_rounded_rect_stroke(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    stroke_width: f32,
    color: [u8; 4],
) {
    if stroke_width <= 0.0 || color[3] == 0 {
        return;
    }
    let min_x = x.floor().max(0.0) as u32;
    let min_y = y.floor().max(0.0) as u32;
    let max_x = (x + width).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + height).ceil().min(canvas.height() as f32) as u32;
    let inner_x = x + stroke_width;
    let inner_y = y + stroke_width;
    let inner_w = (width - stroke_width * 2.0).max(0.0);
    let inner_h = (height - stroke_width * 2.0).max(0.0);
    let inner_r = (radius - stroke_width).max(0.0);
    for py in min_y..max_y {
        for px in min_x..max_x {
            let cx = px as f32 + 0.5;
            let cy = py as f32 + 0.5;
            if rounded_rect_contains(cx, cy, x, y, width, height, radius)
                && !rounded_rect_contains(cx, cy, inner_x, inner_y, inner_w, inner_h, inner_r)
            {
                blend_pixel(canvas, px, py, color);
            }
        }
    }
}

fn rounded_rect_contains(
    px: f32,
    py: f32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
) -> bool {
    if px < x || py < y || px > x + width || py > y + height {
        return false;
    }
    let radius = radius.min(width * 0.5).min(height * 0.5).max(0.0);
    if radius <= 0.0 {
        return true;
    }
    let left = x + radius;
    let right = x + width - radius;
    let top = y + radius;
    let bottom = y + height - radius;
    let cx = px.clamp(left, right);
    let cy = py.clamp(top, bottom);
    let dx = px - cx;
    let dy = py - cy;
    dx * dx + dy * dy <= radius * radius
}

fn draw_circle(canvas: &mut RgbaImage, x: f32, y: f32, radius: f32, color: [u8; 4]) {
    if radius <= 0.0 || color[3] == 0 {
        return;
    }
    let min_x = (x - radius).floor().max(0.0) as u32;
    let min_y = (y - radius).floor().max(0.0) as u32;
    let max_x = (x + radius).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + radius).ceil().min(canvas.height() as f32) as u32;
    let r2 = radius * radius;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let dx = px as f32 + 0.5 - x;
            let dy = py as f32 + 0.5 - y;
            if dx * dx + dy * dy <= r2 {
                blend_pixel(canvas, px, py, color);
            }
        }
    }
}

fn draw_circle_paint(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    radius: f32,
    paint: &ResolvedPaint,
    opacity: f32,
    blend: SceneBlendMode,
) {
    if radius <= 0.0 || opacity <= 0.0 {
        return;
    }
    let min_x = (x - radius).floor().max(0.0) as u32;
    let min_y = (y - radius).floor().max(0.0) as u32;
    let max_x = (x + radius).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + radius).ceil().min(canvas.height() as f32) as u32;
    let bounds = PaintBounds {
        min_x: x - radius,
        min_y: y - radius,
        max_x: x + radius,
        max_y: y + radius,
    };
    let r2 = radius * radius;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let point = Point2::new(px as f32 + 0.5, py as f32 + 0.5);
            let dx = point.x - x;
            let dy = point.y - y;
            if dx * dx + dy * dy <= r2
                && let Some(src) = sample_paint(paint, point, bounds, opacity)
            {
                blend_pixel_with_mode(canvas, px, py, src, blend);
            }
        }
    }
}

fn draw_circle_stroke(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    radius: f32,
    stroke_width: f32,
    color: [u8; 4],
) {
    if radius <= 0.0 || stroke_width <= 0.0 || color[3] == 0 {
        return;
    }
    let min_x = (x - radius).floor().max(0.0) as u32;
    let min_y = (y - radius).floor().max(0.0) as u32;
    let max_x = (x + radius).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + radius).ceil().min(canvas.height() as f32) as u32;
    let outer = radius * radius;
    let inner_radius = (radius - stroke_width).max(0.0);
    let inner = inner_radius * inner_radius;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let dx = px as f32 + 0.5 - x;
            let dy = py as f32 + 0.5 - y;
            let d2 = dx * dx + dy * dy;
            if d2 <= outer && d2 >= inner {
                blend_pixel(canvas, px, py, color);
            }
        }
    }
}

fn draw_rounded_rect_paint(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    paint: &ResolvedPaint,
    opacity: f32,
    blend: SceneBlendMode,
) {
    if width <= 0.0 || height <= 0.0 || opacity <= 0.0 {
        return;
    }
    let min_x = x.floor().max(0.0) as u32;
    let min_y = y.floor().max(0.0) as u32;
    let max_x = (x + width).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y + height).ceil().min(canvas.height() as f32) as u32;
    let bounds = PaintBounds {
        min_x: x,
        min_y: y,
        max_x: x + width,
        max_y: y + height,
    };
    for py in min_y..max_y {
        for px in min_x..max_x {
            let point = Point2::new(px as f32 + 0.5, py as f32 + 0.5);
            if rounded_rect_contains(point.x, point.y, x, y, width, height, radius)
                && let Some(src) = sample_paint(paint, point, bounds, opacity)
            {
                blend_pixel_with_mode(canvas, px, py, src, blend);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Point2 {
    x: f32,
    y: f32,
}

impl Point2 {
    fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            x: self.x + (other.x - self.x) * t,
            y: self.y + (other.y - self.y) * t,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PathToken {
    Command(char),
    Number(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrokeCap {
    Round,
    Butt,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrokeJoin {
    Round,
    Miter,
    Bevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrokeTexture {
    Solid,
    Sketch,
    Rough,
    Pencil,
    Ink,
    Charcoal,
    Marker,
    Hairline,
}

#[derive(Debug, Clone, Copy)]
struct StrokeStyle {
    cap: StrokeCap,
    join: StrokeJoin,
    taper_start: f32,
    taper_end: f32,
    texture: StrokeTexture,
    roughness: f32,
    copies: u32,
    texture_strength: f32,
    bristles: u32,
    pressure_auto: bool,
    pressure_min: f32,
    pressure_curve: f32,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self {
            cap: StrokeCap::Round,
            join: StrokeJoin::Round,
            taper_start: 0.0,
            taper_end: 0.0,
            texture: StrokeTexture::Solid,
            roughness: 0.0,
            copies: 1,
            texture_strength: 0.0,
            bristles: 0,
            pressure_auto: false,
            pressure_min: 1.0,
            pressure_curve: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct StrokeParams {
    roughness: f32,
    copies: u32,
    texture_strength: f32,
    bristles: u32,
    pressure_auto: bool,
    pressure_min: f32,
    pressure_curve: f32,
}

fn evaluate_trim(
    trim_start: &str,
    trim_end: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<(f32, f32), MotionLoomSceneRenderError> {
    let start = eval_scene_number(trim_start, time_norm, time_sec)?.clamp(0.0, 1.0);
    let end = eval_scene_number(trim_end, time_norm, time_sec)?.clamp(0.0, 1.0);
    Ok((start, end))
}

fn eval_line_stroke_style(
    line: &LineNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<StrokeStyle, MotionLoomSceneRenderError> {
    let texture = parse_stroke_texture(&line.stroke_style);
    let params = eval_stroke_params(
        texture,
        &line.stroke_roughness,
        &line.stroke_copies,
        &line.stroke_texture,
        &line.stroke_bristles,
        &line.stroke_pressure,
        &line.stroke_pressure_min,
        &line.stroke_pressure_curve,
        time_norm,
        time_sec,
    )?;
    Ok(StrokeStyle {
        cap: parse_stroke_cap(&line.line_cap),
        join: StrokeJoin::Round,
        taper_start: eval_scene_number(&line.taper_start, time_norm, time_sec)?.clamp(0.0, 0.5),
        taper_end: eval_scene_number(&line.taper_end, time_norm, time_sec)?.clamp(0.0, 0.5),
        texture,
        roughness: params.roughness,
        copies: params.copies,
        texture_strength: params.texture_strength,
        bristles: params.bristles,
        pressure_auto: params.pressure_auto,
        pressure_min: params.pressure_min,
        pressure_curve: params.pressure_curve,
    })
}

fn eval_polyline_stroke_style(
    polyline: &PolylineNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<StrokeStyle, MotionLoomSceneRenderError> {
    let texture = parse_stroke_texture(&polyline.stroke_style);
    let params = eval_stroke_params(
        texture,
        &polyline.stroke_roughness,
        &polyline.stroke_copies,
        &polyline.stroke_texture,
        &polyline.stroke_bristles,
        &polyline.stroke_pressure,
        &polyline.stroke_pressure_min,
        &polyline.stroke_pressure_curve,
        time_norm,
        time_sec,
    )?;
    Ok(StrokeStyle {
        cap: parse_stroke_cap(&polyline.line_cap),
        join: parse_stroke_join(&polyline.line_join),
        taper_start: eval_scene_number(&polyline.taper_start, time_norm, time_sec)?.clamp(0.0, 0.5),
        taper_end: eval_scene_number(&polyline.taper_end, time_norm, time_sec)?.clamp(0.0, 0.5),
        texture,
        roughness: params.roughness,
        copies: params.copies,
        texture_strength: params.texture_strength,
        bristles: params.bristles,
        pressure_auto: params.pressure_auto,
        pressure_min: params.pressure_min,
        pressure_curve: params.pressure_curve,
    })
}

fn eval_path_stroke_style(
    path: &PathNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<StrokeStyle, MotionLoomSceneRenderError> {
    let texture = parse_stroke_texture(&path.stroke_style);
    let params = eval_stroke_params(
        texture,
        &path.stroke_roughness,
        &path.stroke_copies,
        &path.stroke_texture,
        &path.stroke_bristles,
        &path.stroke_pressure,
        &path.stroke_pressure_min,
        &path.stroke_pressure_curve,
        time_norm,
        time_sec,
    )?;
    Ok(StrokeStyle {
        cap: parse_stroke_cap(&path.line_cap),
        join: parse_stroke_join(&path.line_join),
        taper_start: eval_scene_number(&path.taper_start, time_norm, time_sec)?.clamp(0.0, 0.5),
        taper_end: eval_scene_number(&path.taper_end, time_norm, time_sec)?.clamp(0.0, 0.5),
        texture,
        roughness: params.roughness,
        copies: params.copies,
        texture_strength: params.texture_strength,
        bristles: params.bristles,
        pressure_auto: params.pressure_auto,
        pressure_min: params.pressure_min,
        pressure_curve: params.pressure_curve,
    })
}

fn face_jaw_to_path_node(
    face_jaw: &FaceJawNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<PathNode, MotionLoomSceneRenderError> {
    let cx = eval_scene_number(&face_jaw.x, time_norm, time_sec)?;
    let y = eval_scene_number(&face_jaw.y, time_norm, time_sec)?;
    let scale = eval_scene_number(&face_jaw.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
    let width = eval_scene_number(&face_jaw.width, time_norm, time_sec)?.max(0.0) * scale;
    let height = eval_scene_number(&face_jaw.height, time_norm, time_sec)?.max(0.0) * scale;
    let cheek_width =
        eval_scene_number(&face_jaw.cheek_width, time_norm, time_sec)?.max(0.0) * scale;
    let chin_width = eval_scene_number(&face_jaw.chin_width, time_norm, time_sec)?.max(0.0) * scale;
    let sharpness =
        eval_scene_number(&face_jaw.chin_sharpness, time_norm, time_sec)?.clamp(0.0, 1.0);
    let ease = eval_scene_number(&face_jaw.jaw_ease, time_norm, time_sec)?.clamp(0.0, 1.0);

    let temple_y = y + height * 0.14;
    let cheek_y = y + height * 0.68;
    let chin_y = y + height;
    let temple_half = width * 0.49;
    let cheek_half = cheek_width * 0.5;
    let chin_half = (chin_width * 0.5 * (1.0 - sharpness * 0.82)).max(width * 0.004);
    let side_bulge = width * (0.02 + ease * 0.06);
    let cheek_lift = height * (0.08 + ease * 0.08);
    let chin_ctrl_y = chin_y - height * (0.04 + (1.0 - sharpness) * 0.11);
    let top_y = y + height * (0.01 - ease * 0.035);
    let top_handle = width * (0.30 + ease * 0.15);

    let left_temple = cx - temple_half;
    let right_temple = cx + temple_half;
    let left_cheek = cx - cheek_half;
    let right_cheek = cx + cheek_half;

    let jaw_from_left = format!(
        "C {lcx:.3} {lcy1:.3} {lhx:.3} {lh_y:.3} {cx1:.3} {cy1:.3} \
         C {chin_lx:.3} {chin_ctrl_y:.3} {chin_lx:.3} {chin_ctrl_y:.3} {cx:.3} {chin_y:.3} \
         C {chin_rx:.3} {chin_ctrl_y:.3} {chin_rx:.3} {chin_ctrl_y:.3} {cx2:.3} {cy2:.3} \
         C {rhx:.3} {rh_y:.3} {rcx:.3} {rcy1:.3} {rt:.3} {ty:.3}",
        lcx = left_temple - side_bulge,
        lcy1 = temple_y + height * 0.25,
        lhx = left_cheek,
        lh_y = cheek_y - cheek_lift,
        cx1 = left_cheek,
        cy1 = cheek_y,
        chin_lx = cx - chin_half,
        chin_ctrl_y = chin_ctrl_y,
        cx = cx,
        chin_y = chin_y,
        chin_rx = cx + chin_half,
        cx2 = right_cheek,
        cy2 = cheek_y,
        rhx = right_cheek,
        rh_y = cheek_y - cheek_lift,
        rcx = right_temple + side_bulge,
        rcy1 = temple_y + height * 0.25,
        rt = right_temple,
        ty = temple_y,
    );

    let closed = scene_bool(&face_jaw.closed);
    let d = if closed {
        format!(
            "M {lt:.3} {ty:.3} \
             C {tlh:.3} {top_y:.3} {trh:.3} {top_y:.3} {rt:.3} {ty:.3} \
             C {rc1:.3} {rcy1:.3} {rhx:.3} {rh_y:.3} {rcx:.3} {cy:.3} \
             C {chin_rx:.3} {chin_ctrl_y:.3} {chin_rx:.3} {chin_ctrl_y:.3} {cx:.3} {chin_y:.3} \
             C {chin_lx:.3} {chin_ctrl_y:.3} {chin_lx:.3} {chin_ctrl_y:.3} {lcx:.3} {cy:.3} \
             C {lhx:.3} {rh_y:.3} {lc1:.3} {rcy1:.3} {lt:.3} {ty:.3} Z",
            lt = left_temple,
            ty = temple_y,
            tlh = cx - top_handle,
            top_y = top_y,
            trh = cx + top_handle,
            rt = right_temple,
            rc1 = right_temple + side_bulge,
            rcy1 = temple_y + height * 0.25,
            rhx = right_cheek,
            rh_y = cheek_y - cheek_lift,
            rcx = right_cheek,
            cy = cheek_y,
            chin_rx = cx + chin_half,
            chin_ctrl_y = chin_ctrl_y,
            cx = cx,
            chin_y = chin_y,
            chin_lx = cx - chin_half,
            lcx = left_cheek,
            lhx = left_cheek,
            lc1 = left_temple - side_bulge,
        )
    } else {
        format!("M {left_temple:.3} {temple_y:.3} {jaw_from_left}")
    };

    Ok(PathNode {
        id: face_jaw.id.clone(),
        brush: None,
        d,
        stroke: face_jaw.stroke.clone(),
        fill: face_jaw.fill.clone(),
        stroke_width: face_jaw.stroke_width.clone(),
        opacity: face_jaw.opacity.clone(),
        trim_start: face_jaw.trim_start.clone(),
        trim_end: face_jaw.trim_end.clone(),
        line_cap: face_jaw.line_cap.clone(),
        line_join: face_jaw.line_join.clone(),
        taper_start: face_jaw.taper_start.clone(),
        taper_end: face_jaw.taper_end.clone(),
        stroke_style: face_jaw.stroke_style.clone(),
        stroke_roughness: face_jaw.stroke_roughness.clone(),
        stroke_copies: face_jaw.stroke_copies.clone(),
        stroke_texture: face_jaw.stroke_texture.clone(),
        stroke_bristles: face_jaw.stroke_bristles.clone(),
        stroke_pressure: face_jaw.stroke_pressure.clone(),
        stroke_pressure_min: face_jaw.stroke_pressure_min.clone(),
        stroke_pressure_curve: face_jaw.stroke_pressure_curve.clone(),
        blend: face_jaw.blend.clone(),
    })
}

fn scene_bool(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "no" | "off"
    )
}

fn parse_stroke_cap(value: &str) -> StrokeCap {
    match value.trim().to_ascii_lowercase().as_str() {
        "butt" => StrokeCap::Butt,
        "square" => StrokeCap::Square,
        _ => StrokeCap::Round,
    }
}

fn parse_stroke_join(value: &str) -> StrokeJoin {
    match value.trim().to_ascii_lowercase().as_str() {
        "miter" => StrokeJoin::Miter,
        "bevel" => StrokeJoin::Bevel,
        _ => StrokeJoin::Round,
    }
}

fn parse_stroke_texture(value: &str) -> StrokeTexture {
    match value.trim().to_ascii_lowercase().as_str() {
        "sketch" | "hand" | "handdrawn" | "hand_drawn" => StrokeTexture::Sketch,
        "rough" | "dry" => StrokeTexture::Rough,
        "pencil" => StrokeTexture::Pencil,
        "ink" | "cleanink" | "clean_ink" | "boldink" | "bold_ink" => StrokeTexture::Ink,
        "charcoal" => StrokeTexture::Charcoal,
        "marker" => StrokeTexture::Marker,
        "hairline" => StrokeTexture::Hairline,
        _ => StrokeTexture::Solid,
    }
}

fn stroke_texture_defaults(texture: StrokeTexture) -> StrokeParams {
    match texture {
        StrokeTexture::Solid => StrokeParams {
            roughness: 0.0,
            copies: 1,
            texture_strength: 0.0,
            bristles: 0,
            pressure_auto: false,
            pressure_min: 1.0,
            pressure_curve: 1.0,
        },
        StrokeTexture::Sketch => StrokeParams {
            roughness: 1.45,
            copies: 3,
            texture_strength: 0.34,
            bristles: 4,
            pressure_auto: false,
            pressure_min: 0.55,
            pressure_curve: 1.25,
        },
        StrokeTexture::Rough => StrokeParams {
            roughness: 2.4,
            copies: 5,
            texture_strength: 0.62,
            bristles: 7,
            pressure_auto: false,
            pressure_min: 0.45,
            pressure_curve: 1.15,
        },
        StrokeTexture::Pencil => StrokeParams {
            roughness: 1.2,
            copies: 4,
            texture_strength: 0.68,
            bristles: 5,
            pressure_auto: false,
            pressure_min: 0.22,
            pressure_curve: 1.55,
        },
        StrokeTexture::Ink => StrokeParams {
            roughness: 0.65,
            copies: 2,
            texture_strength: 0.05,
            bristles: 0,
            pressure_auto: false,
            pressure_min: 0.74,
            pressure_curve: 0.85,
        },
        StrokeTexture::Charcoal => StrokeParams {
            roughness: 3.2,
            copies: 8,
            texture_strength: 0.90,
            bristles: 14,
            pressure_auto: false,
            pressure_min: 0.18,
            pressure_curve: 1.0,
        },
        StrokeTexture::Marker => StrokeParams {
            roughness: 0.0,
            copies: 1,
            texture_strength: 0.10,
            bristles: 0,
            pressure_auto: false,
            pressure_min: 0.80,
            pressure_curve: 0.7,
        },
        StrokeTexture::Hairline => StrokeParams {
            roughness: 0.05,
            copies: 1,
            texture_strength: 0.0,
            bristles: 0,
            pressure_auto: false,
            pressure_min: 0.95,
            pressure_curve: 1.0,
        },
    }
}

fn eval_stroke_params(
    texture: StrokeTexture,
    roughness_expr: &str,
    copies_expr: &str,
    texture_expr: &str,
    bristles_expr: &str,
    pressure_expr: &str,
    pressure_min_expr: &str,
    pressure_curve_expr: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<StrokeParams, MotionLoomSceneRenderError> {
    let defaults = stroke_texture_defaults(texture);
    let roughness = eval_scene_number(roughness_expr, time_norm, time_sec)?
        .max(0.0)
        .min(32.0);
    let copies = eval_scene_number(copies_expr, time_norm, time_sec)?
        .round()
        .clamp(1.0, 12.0) as u32;
    let texture_strength = eval_scene_number(texture_expr, time_norm, time_sec)?.clamp(0.0, 1.0);
    let bristles = eval_scene_number(bristles_expr, time_norm, time_sec)?
        .round()
        .clamp(0.0, 24.0) as u32;
    let pressure_auto = matches!(
        pressure_expr.trim().to_ascii_lowercase().as_str(),
        "auto" | "true" | "1" | "yes" | "on"
    );
    let pressure_min = eval_scene_number(pressure_min_expr, time_norm, time_sec)?.clamp(0.0, 1.0);
    let pressure_curve = eval_scene_number(pressure_curve_expr, time_norm, time_sec)?
        .max(0.05)
        .min(8.0);
    if texture == StrokeTexture::Solid {
        return Ok(StrokeParams {
            roughness,
            copies: copies.max(1),
            texture_strength,
            bristles,
            pressure_auto,
            pressure_min,
            pressure_curve,
        });
    }
    Ok(StrokeParams {
        roughness: if roughness <= 0.0001 {
            defaults.roughness
        } else {
            roughness
        },
        copies: if copies <= 1 { defaults.copies } else { copies },
        texture_strength: if texture_strength <= 0.0001 {
            defaults.texture_strength
        } else {
            texture_strength
        },
        bristles: if bristles == 0 {
            defaults.bristles
        } else {
            bristles
        },
        pressure_auto,
        pressure_min: if pressure_min >= 0.999 && pressure_auto {
            defaults.pressure_min
        } else {
            pressure_min
        },
        pressure_curve: if (pressure_curve - 1.0).abs() <= 0.0001 && pressure_auto {
            defaults.pressure_curve
        } else {
            pressure_curve
        },
    })
}

fn stroke_texture_copy_count(style: StrokeStyle) -> u32 {
    if style.texture == StrokeTexture::Solid || style.roughness <= 0.0001 {
        1
    } else {
        style.copies.clamp(1, 12)
    }
}

fn stroke_texture_variant(
    p0: Point2,
    p1: Point2,
    style: StrokeStyle,
    copy_ix: u32,
) -> (Point2, Point2, f32, f32) {
    if copy_ix == 0 || style.texture == StrokeTexture::Solid || style.roughness <= 0.0001 {
        return (p0, p1, 1.0, 1.0);
    }

    let dx = p1.x - p0.x;
    let dy = p1.y - p0.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len <= 0.0001 {
        return (p0, p1, 1.0, 1.0);
    }
    let tx = dx / len;
    let ty = dy / len;
    let nx = -ty;
    let ny = tx;
    let copy = copy_ix as f32;
    let rough = style.roughness * (0.75 + copy * 0.18);
    let seed = stroke_texture_seed(p0, p1, copy_ix);
    let n0 = stroke_hash_signed(seed + 11.7) * rough;
    let n1 = stroke_hash_signed(seed + 37.3) * rough;
    let t0 = stroke_hash_signed(seed + 71.9) * rough * 0.35;
    let t1 = stroke_hash_signed(seed + 103.1) * rough * 0.35;
    let start = Point2::new(p0.x + nx * n0 + tx * t0, p0.y + ny * n0 + ty * t0);
    let end = Point2::new(p1.x + nx * n1 + tx * t1, p1.y + ny * n1 + ty * t1);
    let (width_scale, opacity_scale) = match style.texture {
        StrokeTexture::Solid => (1.0, 1.0),
        StrokeTexture::Ink => (0.58, 0.34),
        StrokeTexture::Sketch => ((0.86 - copy * 0.07).max(0.42), 0.38),
        StrokeTexture::Rough => ((0.72 - copy * 0.05).max(0.35), 0.30),
        StrokeTexture::Pencil => ((0.46 - copy * 0.025).max(0.24), 0.24),
        StrokeTexture::Charcoal => ((0.38 - copy * 0.015).max(0.14), 0.18),
        StrokeTexture::Marker => (0.94, 0.72),
        StrokeTexture::Hairline => (0.75, 0.50),
    };
    (start, end, width_scale, opacity_scale)
}

fn stroke_texture_seed(p0: Point2, p1: Point2, copy_ix: u32) -> f32 {
    p0.x * 12.9898 + p0.y * 78.233 + p1.x * 37.719 + p1.y * 11.131 + copy_ix as f32 * 19.19
}

fn stroke_hash_signed(seed: f32) -> f32 {
    let raw = (seed.sin() * 43_758.547).fract();
    let unit = if raw < 0.0 { raw + 1.0 } else { raw };
    unit * 2.0 - 1.0
}

fn draw_stroke_overlays(
    canvas: &mut RgbaImage,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    style: StrokeStyle,
    t0: f32,
    t1: f32,
) {
    if width <= 0.0 || color[3] == 0 {
        return;
    }
    if style.texture_strength > 0.001 {
        draw_stroke_texture_stamps(canvas, p0, p1, width, color, style, t0, t1);
    }
    if style.bristles > 0 {
        draw_stroke_bristles(canvas, p0, p1, width, color, style, t0, t1);
    }
}

fn draw_stroke_texture_stamps(
    canvas: &mut RgbaImage,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    style: StrokeStyle,
    t0: f32,
    t1: f32,
) {
    let len = point_distance(p0, p1);
    if len <= 0.0001 {
        return;
    }
    let strength = style.texture_strength.clamp(0.0, 1.0);
    let dx = (p1.x - p0.x) / len;
    let dy = (p1.y - p0.y) / len;
    let nx = -dy;
    let ny = dx;
    let spacing = (width * (1.35 - strength * 0.65)).clamp(2.0, 18.0);
    let steps = (len / spacing).ceil().clamp(1.0, 72.0) as u32;
    let texture_size = match style.texture {
        StrokeTexture::Charcoal => 1.65,
        StrokeTexture::Rough => 1.25,
        StrokeTexture::Pencil => 1.0,
        StrokeTexture::Sketch => 0.82,
        StrokeTexture::Marker => 0.72,
        StrokeTexture::Ink => 0.55,
        StrokeTexture::Hairline | StrokeTexture::Solid => 0.46,
    };
    let alpha_scale = match style.texture {
        StrokeTexture::Charcoal => 0.18,
        StrokeTexture::Pencil => 0.16,
        StrokeTexture::Rough => 0.14,
        StrokeTexture::Sketch => 0.12,
        StrokeTexture::Marker => 0.10,
        StrokeTexture::Ink => 0.08,
        StrokeTexture::Hairline | StrokeTexture::Solid => 0.06,
    };
    for step in 0..steps {
        let seed = stroke_texture_seed(p0, p1, step + 271);
        let keep = ((stroke_hash_signed(seed + 13.1) + 1.0) * 0.5).clamp(0.0, 1.0);
        if keep > strength {
            continue;
        }
        let local_t = ((step as f32 + 0.5) / steps as f32).clamp(0.0, 1.0);
        let global_t = t0 + (t1 - t0) * local_t;
        let pressure = stroke_taper_pressure(global_t, style).max(0.05);
        let tangent_noise = stroke_hash_signed(seed + 37.7) * spacing * 0.25;
        let normal_noise = stroke_hash_signed(seed + 91.3) * width * pressure * 0.45;
        let p = p0.lerp(p1, local_t);
        let size_noise = ((stroke_hash_signed(seed + 163.0) + 1.0) * 0.5).clamp(0.0, 1.0);
        let radius = (width * pressure * (0.035 + size_noise * 0.10) * texture_size).max(0.35);
        let mut stamp_color = color;
        stamp_color[3] = ((stamp_color[3] as f32) * strength * alpha_scale)
            .round()
            .clamp(0.0, 255.0) as u8;
        if stamp_color[3] > 0 {
            draw_circle(
                canvas,
                p.x + dx * tangent_noise + nx * normal_noise,
                p.y + dy * tangent_noise + ny * normal_noise,
                radius,
                stamp_color,
            );
        }
    }
}

fn draw_stroke_bristles(
    canvas: &mut RgbaImage,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    style: StrokeStyle,
    t0: f32,
    t1: f32,
) {
    let len = point_distance(p0, p1);
    if len <= 0.0001 {
        return;
    }
    let dx = (p1.x - p0.x) / len;
    let dy = (p1.y - p0.y) / len;
    let nx = -dy;
    let ny = dx;
    let count = style.bristles.clamp(0, 24);
    let pressure =
        ((stroke_taper_pressure(t0, style) + stroke_taper_pressure(t1, style)) * 0.5).max(0.05);
    let bristle_width = (width * 0.08 * pressure).clamp(0.25, 2.2);
    let alpha_scale = match style.texture {
        StrokeTexture::Charcoal => 0.20,
        StrokeTexture::Rough => 0.18,
        StrokeTexture::Pencil => 0.15,
        StrokeTexture::Sketch => 0.13,
        _ => 0.11,
    };
    for ix in 0..count {
        let lane = if count <= 1 {
            0.0
        } else {
            ix as f32 / (count - 1) as f32 * 2.0 - 1.0
        };
        let seed = stroke_texture_seed(p0, p1, ix + 997);
        let offset = lane * width * pressure * 0.42
            + stroke_hash_signed(seed + 21.0) * style.roughness * 0.55;
        let start_t = (stroke_hash_signed(seed + 57.0) * 0.04).max(0.0);
        let end_t = 1.0 - (stroke_hash_signed(seed + 83.0) * 0.04).max(0.0);
        let start = p0.lerp(p1, start_t);
        let end = p0.lerp(p1, end_t);
        let mut bristle_color = color;
        bristle_color[3] = ((bristle_color[3] as f32) * alpha_scale)
            .round()
            .clamp(0.0, 255.0) as u8;
        if bristle_color[3] > 0 {
            draw_line_segment(
                canvas,
                start.x + nx * offset,
                start.y + ny * offset,
                end.x + nx * offset,
                end.y + ny * offset,
                bristle_width,
                bristle_color,
            );
        }
    }
}

fn parse_polyline_points(points: &str) -> Result<Vec<Point2>, MotionLoomSceneRenderError> {
    let values = points
        .replace(',', " ")
        .split_whitespace()
        .map(|raw| {
            raw.parse::<f32>()
                .map_err(|_| MotionLoomSceneRenderError::InvalidPathData {
                    value: points.to_string(),
                    message: format!("invalid point number: {raw}"),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() < 4 || values.len() % 2 != 0 {
        return Err(MotionLoomSceneRenderError::InvalidPathData {
            value: points.to_string(),
            message: "Polyline points must contain at least two x,y pairs.".to_string(),
        });
    }
    Ok(values
        .chunks_exact(2)
        .map(|pair| Point2::new(pair[0], pair[1]))
        .collect())
}

fn draw_trimmed_polylines_styled(
    canvas: &mut RgbaImage,
    subpaths: &[Vec<Point2>],
    width: f32,
    color: [u8; 4],
    trim: (f32, f32),
    style: StrokeStyle,
) {
    for segment in trimmed_polyline_segments_with_progress(subpaths, trim) {
        draw_line_segment_styled(
            canvas, segment.p0, segment.p1, width, color, style, segment.t0, segment.t1,
        );
    }
    draw_polyline_joins(canvas, subpaths, width, color, trim, style);
}

fn draw_transformed_trimmed_polylines_styled(
    canvas: &mut RgbaImage,
    subpaths: &[Vec<Point2>],
    width: f32,
    color: [u8; 4],
    trim: (f32, f32),
    transform: Affine2,
    style: StrokeStyle,
) {
    for segment in trimmed_polyline_segments_with_progress(subpaths, trim) {
        let (x0, y0) = transform.transform_point(segment.p0.x, segment.p0.y);
        let (x1, y1) = transform.transform_point(segment.p1.x, segment.p1.y);
        draw_line_segment_styled(
            canvas,
            Point2::new(x0, y0),
            Point2::new(x1, y1),
            width,
            color,
            style,
            segment.t0,
            segment.t1,
        );
    }
    draw_transformed_polyline_joins(canvas, subpaths, width, color, trim, transform, style);
}

fn affine_uniform_scale(transform: Affine2) -> f32 {
    let x_scale = (transform.m00.powi(2) + transform.m10.powi(2)).sqrt();
    let y_scale = (transform.m01.powi(2) + transform.m11.powi(2)).sqrt();
    ((x_scale + y_scale) * 0.5).max(0.001)
}

#[derive(Debug, Clone, Copy)]
struct TrimmedSegment {
    p0: Point2,
    p1: Point2,
    t0: f32,
    t1: f32,
}

fn trimmed_polyline_segments_with_progress(
    subpaths: &[Vec<Point2>],
    trim: (f32, f32),
) -> Vec<TrimmedSegment> {
    if trim.1 <= trim.0 {
        return Vec::new();
    }
    let total = polyline_total_length(subpaths);
    if total <= 0.0001 {
        return Vec::new();
    }
    let start_distance = trim.0 * total;
    let end_distance = trim.1 * total;
    let mut cursor = 0.0;
    let mut out = Vec::new();

    for subpath in subpaths {
        for segment in subpath.windows(2) {
            let p0 = segment[0];
            let p1 = segment[1];
            let len = point_distance(p0, p1);
            if len <= 0.0001 {
                continue;
            }
            let seg_start = cursor;
            let seg_end = cursor + len;
            let draw_start = start_distance.max(seg_start);
            let draw_end = end_distance.min(seg_end);
            if draw_end > draw_start {
                let t0 = (draw_start - seg_start) / len;
                let t1 = (draw_end - seg_start) / len;
                out.push(TrimmedSegment {
                    p0: p0.lerp(p1, t0),
                    p1: p0.lerp(p1, t1),
                    t0: draw_start / total,
                    t1: draw_end / total,
                });
            }
            cursor = seg_end;
        }
    }
    out
}

fn polyline_total_length(subpaths: &[Vec<Point2>]) -> f32 {
    subpaths
        .iter()
        .flat_map(|subpath| subpath.windows(2))
        .map(|segment| point_distance(segment[0], segment[1]))
        .sum()
}

fn point_distance(a: Point2, b: Point2) -> f32 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt()
}

fn parse_path_subpaths(data: &str) -> Result<Vec<Vec<Point2>>, MotionLoomSceneRenderError> {
    let tokens = tokenize_path_data(data)?;
    let mut i = 0usize;
    let mut command: Option<char> = None;
    let mut current = Point2::new(0.0, 0.0);
    let mut subpath_start = current;
    let mut active = Vec::<Point2>::new();
    let mut subpaths = Vec::<Vec<Point2>>::new();

    while i < tokens.len() {
        if let Some(PathToken::Command(cmd)) = tokens.get(i).copied() {
            command = Some(cmd);
            i += 1;
        }
        let cmd = command.ok_or_else(|| MotionLoomSceneRenderError::InvalidPathData {
            value: data.to_string(),
            message: "path data must start with a command.".to_string(),
        })?;

        match cmd {
            'M' | 'm' => {
                flush_active_subpath(&mut active, &mut subpaths);
                let relative = cmd == 'm';
                let first = consume_path_point(&tokens, &mut i, current, relative, data)?;
                current = first;
                subpath_start = first;
                active.push(first);
                let line_cmd = if relative { 'l' } else { 'L' };
                while next_path_token_is_number(&tokens, i) {
                    current = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    active.push(current);
                }
                command = Some(line_cmd);
            }
            'L' | 'l' => {
                let relative = cmd == 'l';
                while next_path_token_is_number(&tokens, i) {
                    current = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    active.push(current);
                }
            }
            'H' | 'h' => {
                let relative = cmd == 'h';
                while next_path_token_is_number(&tokens, i) {
                    let x = consume_path_number(&tokens, &mut i, data)?;
                    current = if relative {
                        Point2::new(current.x + x, current.y)
                    } else {
                        Point2::new(x, current.y)
                    };
                    active.push(current);
                }
            }
            'V' | 'v' => {
                let relative = cmd == 'v';
                while next_path_token_is_number(&tokens, i) {
                    let y = consume_path_number(&tokens, &mut i, data)?;
                    current = if relative {
                        Point2::new(current.x, current.y + y)
                    } else {
                        Point2::new(current.x, y)
                    };
                    active.push(current);
                }
            }
            'C' | 'c' => {
                let relative = cmd == 'c';
                while next_path_token_is_number(&tokens, i) {
                    let c1 = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    let c2 = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    let end = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    sample_cubic(current, c1, c2, end, &mut active);
                    current = end;
                }
            }
            'Q' | 'q' => {
                let relative = cmd == 'q';
                while next_path_token_is_number(&tokens, i) {
                    let c = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    let end = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    sample_quadratic(current, c, end, &mut active);
                    current = end;
                }
            }
            'Z' | 'z' => {
                if !active.is_empty() {
                    active.push(subpath_start);
                    current = subpath_start;
                    flush_active_subpath(&mut active, &mut subpaths);
                }
                command = None;
            }
            _ => {
                return Err(MotionLoomSceneRenderError::InvalidPathData {
                    value: data.to_string(),
                    message: format!("unsupported path command: {cmd}"),
                });
            }
        }
    }

    flush_active_subpath(&mut active, &mut subpaths);
    if subpaths.is_empty() {
        return Err(MotionLoomSceneRenderError::InvalidPathData {
            value: data.to_string(),
            message: "path does not contain drawable segments.".to_string(),
        });
    }
    Ok(subpaths)
}

fn tokenize_path_data(data: &str) -> Result<Vec<PathToken>, MotionLoomSceneRenderError> {
    let bytes = data.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch.is_ascii_whitespace() || ch == ',' {
            i += 1;
            continue;
        }
        if is_path_command(ch) {
            tokens.push(PathToken::Command(ch));
            i += 1;
            continue;
        }
        if is_path_number_start(ch) {
            let start = i;
            i += 1;
            while i < bytes.len() {
                let ch = bytes[i] as char;
                if ch.is_ascii_digit() || ch == '.' {
                    i += 1;
                    continue;
                }
                if ch == 'e' || ch == 'E' {
                    i += 1;
                    if i < bytes.len() {
                        let sign = bytes[i] as char;
                        if sign == '+' || sign == '-' {
                            i += 1;
                        }
                    }
                    continue;
                }
                break;
            }
            let raw = &data[start..i];
            let value =
                raw.parse::<f32>()
                    .map_err(|_| MotionLoomSceneRenderError::InvalidPathData {
                        value: data.to_string(),
                        message: format!("invalid path number: {raw}"),
                    })?;
            tokens.push(PathToken::Number(value));
            continue;
        }
        return Err(MotionLoomSceneRenderError::InvalidPathData {
            value: data.to_string(),
            message: format!("unexpected path character: {ch}"),
        });
    }
    Ok(tokens)
}

fn is_path_command(ch: char) -> bool {
    matches!(
        ch,
        'M' | 'm' | 'L' | 'l' | 'H' | 'h' | 'V' | 'v' | 'C' | 'c' | 'Q' | 'q' | 'Z' | 'z'
    )
}

fn is_path_number_start(ch: char) -> bool {
    ch.is_ascii_digit() || ch == '-' || ch == '+' || ch == '.'
}

fn next_path_token_is_number(tokens: &[PathToken], index: usize) -> bool {
    matches!(tokens.get(index), Some(PathToken::Number(_)))
}

fn consume_path_number(
    tokens: &[PathToken],
    index: &mut usize,
    source: &str,
) -> Result<f32, MotionLoomSceneRenderError> {
    match tokens.get(*index).copied() {
        Some(PathToken::Number(value)) => {
            *index += 1;
            Ok(value)
        }
        _ => Err(MotionLoomSceneRenderError::InvalidPathData {
            value: source.to_string(),
            message: "path command is missing a numeric parameter.".to_string(),
        }),
    }
}

fn consume_path_point(
    tokens: &[PathToken],
    index: &mut usize,
    current: Point2,
    relative: bool,
    source: &str,
) -> Result<Point2, MotionLoomSceneRenderError> {
    let x = consume_path_number(tokens, index, source)?;
    let y = consume_path_number(tokens, index, source)?;
    if relative {
        Ok(Point2::new(current.x + x, current.y + y))
    } else {
        Ok(Point2::new(x, y))
    }
}

fn flush_active_subpath(active: &mut Vec<Point2>, subpaths: &mut Vec<Vec<Point2>>) {
    if active.len() >= 2 {
        subpaths.push(std::mem::take(active));
    } else {
        active.clear();
    }
}

fn sample_cubic(p0: Point2, c1: Point2, c2: Point2, p1: Point2, out: &mut Vec<Point2>) {
    const STEPS: usize = 28;
    for step in 1..=STEPS {
        let t = step as f32 / STEPS as f32;
        let mt = 1.0 - t;
        out.push(Point2::new(
            mt.powi(3) * p0.x
                + 3.0 * mt.powi(2) * t * c1.x
                + 3.0 * mt * t.powi(2) * c2.x
                + t.powi(3) * p1.x,
            mt.powi(3) * p0.y
                + 3.0 * mt.powi(2) * t * c1.y
                + 3.0 * mt * t.powi(2) * c2.y
                + t.powi(3) * p1.y,
        ));
    }
}

fn sample_quadratic(p0: Point2, c: Point2, p1: Point2, out: &mut Vec<Point2>) {
    const STEPS: usize = 20;
    for step in 1..=STEPS {
        let t = step as f32 / STEPS as f32;
        let mt = 1.0 - t;
        out.push(Point2::new(
            mt.powi(2) * p0.x + 2.0 * mt * t * c.x + t.powi(2) * p1.x,
            mt.powi(2) * p0.y + 2.0 * mt * t * c.y + t.powi(2) * p1.y,
        ));
    }
}

fn draw_line_segment_styled(
    canvas: &mut RgbaImage,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    style: StrokeStyle,
    t0: f32,
    t1: f32,
) {
    if style.texture != StrokeTexture::Solid && style.roughness > 0.0001 {
        let copies = stroke_texture_copy_count(style);
        for copy_ix in 0..copies {
            let (start, end, width_scale, opacity_scale) =
                stroke_texture_variant(p0, p1, style, copy_ix);
            let mut copy_color = color;
            copy_color[3] = ((copy_color[3] as f32) * opacity_scale)
                .round()
                .clamp(0.0, 255.0) as u8;
            let mut solid_style = style;
            solid_style.texture = StrokeTexture::Solid;
            solid_style.roughness = 0.0;
            solid_style.copies = 1;
            solid_style.texture_strength = 0.0;
            solid_style.bristles = 0;
            draw_line_segment_styled(
                canvas,
                start,
                end,
                (width * width_scale).max(0.01),
                copy_color,
                solid_style,
                t0,
                t1,
            );
        }
        draw_stroke_overlays(canvas, p0, p1, width, color, style, t0, t1);
        return;
    }
    if style.taper_start > 0.0001 || style.taper_end > 0.0001 || style.pressure_auto {
        draw_tapered_line_segment(canvas, p0, p1, width, color, style, t0, t1);
        draw_stroke_overlays(canvas, p0, p1, width, color, style, t0, t1);
        return;
    }
    match style.cap {
        StrokeCap::Round => draw_line_segment(canvas, p0.x, p0.y, p1.x, p1.y, width, color),
        StrokeCap::Butt => draw_line_segment_butt(canvas, p0, p1, width, color, 0.0),
        StrokeCap::Square => draw_line_segment_butt(canvas, p0, p1, width, color, width * 0.5),
    }
    draw_stroke_overlays(canvas, p0, p1, width, color, style, t0, t1);
}

fn draw_tapered_line_segment(
    canvas: &mut RgbaImage,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    style: StrokeStyle,
    t0: f32,
    t1: f32,
) {
    let len = point_distance(p0, p1);
    if len <= 0.0001 || width <= 0.0 || color[3] == 0 {
        return;
    }
    let steps = (len / (width.max(1.0) * 0.35)).ceil().clamp(2.0, 256.0) as u32;
    for i in 0..=steps {
        let local_t = i as f32 / steps as f32;
        let global_t = t0 + (t1 - t0) * local_t;
        let pressure = stroke_taper_pressure(global_t, style);
        if pressure <= 0.0001 {
            continue;
        }
        let p = p0.lerp(p1, local_t);
        draw_circle(canvas, p.x, p.y, width * pressure * 0.5, color);
    }
}

fn stroke_taper_pressure(t: f32, style: StrokeStyle) -> f32 {
    let mut pressure: f32 = 1.0;
    if style.taper_start > 0.0001 {
        pressure = pressure.min((t / style.taper_start).clamp(0.0, 1.0));
    }
    if style.taper_end > 0.0001 {
        pressure = pressure.min(((1.0 - t) / style.taper_end).clamp(0.0, 1.0));
    }
    if style.pressure_auto {
        let bell = (std::f32::consts::PI * t.clamp(0.0, 1.0)).sin().max(0.0);
        let shaped = bell.powf(style.pressure_curve.max(0.05));
        let auto_pressure = style.pressure_min + (1.0 - style.pressure_min) * shaped;
        pressure *= auto_pressure.clamp(0.0, 1.0);
    }
    pressure
}

fn draw_line_segment_butt(
    canvas: &mut RgbaImage,
    p0: Point2,
    p1: Point2,
    width: f32,
    color: [u8; 4],
    extension: f32,
) {
    if width <= 0.0 || color[3] == 0 {
        return;
    }
    let dx = p1.x - p0.x;
    let dy = p1.y - p0.y;
    let len = (dx * dx + dy * dy).sqrt().max(0.0001);
    let ux = dx / len;
    let uy = dy / len;
    let start = Point2::new(p0.x - ux * extension, p0.y - uy * extension);
    let end = Point2::new(p1.x + ux * extension, p1.y + uy * extension);
    let spread = width * 0.5 + 2.0;
    let min_x = (start.x.min(end.x) - spread).floor().max(0.0) as u32;
    let min_y = (start.y.min(end.y) - spread).floor().max(0.0) as u32;
    let max_x = (start.x.max(end.x) + spread)
        .ceil()
        .min(canvas.width() as f32) as u32;
    let max_y = (start.y.max(end.y) + spread)
        .ceil()
        .min(canvas.height() as f32) as u32;
    let len2 = ((end.x - start.x).powi(2) + (end.y - start.y).powi(2)).max(0.0001);
    let half_width = width * 0.5;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let cx = px as f32 + 0.5;
            let cy = py as f32 + 0.5;
            let t =
                ((cx - start.x) * (end.x - start.x) + (cy - start.y) * (end.y - start.y)) / len2;
            if !(0.0..=1.0).contains(&t) {
                continue;
            }
            let nearest_x = start.x + (end.x - start.x) * t;
            let nearest_y = start.y + (end.y - start.y) * t;
            let dist = ((cx - nearest_x).powi(2) + (cy - nearest_y).powi(2)).sqrt();
            let coverage = (half_width + 0.5 - dist).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let mut src = color;
            src[3] = ((src[3] as f32) * coverage).round().clamp(0.0, 255.0) as u8;
            if src[3] > 0 {
                blend_pixel(canvas, px, py, src);
            }
        }
    }
}

fn draw_polyline_joins(
    canvas: &mut RgbaImage,
    subpaths: &[Vec<Point2>],
    width: f32,
    color: [u8; 4],
    trim: (f32, f32),
    style: StrokeStyle,
) {
    if style.join != StrokeJoin::Round || color[3] == 0 || width <= 0.0 {
        return;
    }
    let total = polyline_total_length(subpaths);
    if total <= 0.0001 {
        return;
    }
    let start_distance = trim.0 * total;
    let end_distance = trim.1 * total;
    let mut cursor = 0.0;
    for subpath in subpaths {
        for (ix, point) in subpath.iter().enumerate() {
            if ix == 0 || ix + 1 == subpath.len() {
                continue;
            }
            let distance = cursor + polyline_total_length(&[subpath[..=ix].to_vec()]);
            if distance < start_distance || distance > end_distance {
                continue;
            }
            let pressure = stroke_taper_pressure(distance / total, style);
            if pressure > 0.0001 {
                draw_circle(canvas, point.x, point.y, width * pressure * 0.5, color);
            }
        }
        cursor += polyline_total_length(std::slice::from_ref(subpath));
    }
}

fn draw_transformed_polyline_joins(
    canvas: &mut RgbaImage,
    subpaths: &[Vec<Point2>],
    width: f32,
    color: [u8; 4],
    trim: (f32, f32),
    transform: Affine2,
    style: StrokeStyle,
) {
    if style.join != StrokeJoin::Round || color[3] == 0 || width <= 0.0 {
        return;
    }
    let total = polyline_total_length(subpaths);
    if total <= 0.0001 {
        return;
    }
    let start_distance = trim.0 * total;
    let end_distance = trim.1 * total;
    let mut cursor = 0.0;
    for subpath in subpaths {
        let mut local_distance = 0.0;
        for ix in 1..subpath.len().saturating_sub(1) {
            local_distance += point_distance(subpath[ix - 1], subpath[ix]);
            let distance = cursor + local_distance;
            if distance < start_distance || distance > end_distance {
                continue;
            }
            let pressure = stroke_taper_pressure(distance / total, style);
            if pressure <= 0.0001 {
                continue;
            }
            let (x, y) = transform.transform_point(subpath[ix].x, subpath[ix].y);
            draw_circle(canvas, x, y, width * pressure * 0.5, color);
        }
        cursor += polyline_total_length(std::slice::from_ref(subpath));
    }
}

fn draw_transformed_filled_polylines(
    canvas: &mut RgbaImage,
    subpaths: &[Vec<Point2>],
    color: [u8; 4],
    transform: Affine2,
) {
    let transformed = subpaths
        .iter()
        .map(|subpath| {
            subpath
                .iter()
                .map(|point| {
                    let (x, y) = transform.transform_point(point.x, point.y);
                    Point2::new(x, y)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    draw_filled_polylines_impl(canvas, &transformed, color);
}

fn draw_filled_polylines_impl(canvas: &mut RgbaImage, subpaths: &[Vec<Point2>], color: [u8; 4]) {
    if color[3] == 0 {
        return;
    }
    let Some((min_x, min_y, max_x, max_y)) = polyline_bounds(subpaths) else {
        return;
    };
    let min_x = min_x.floor().max(0.0) as u32;
    let min_y = min_y.floor().max(0.0) as u32;
    let max_x = max_x.ceil().min(canvas.width() as f32) as u32;
    let max_y = max_y.ceil().min(canvas.height() as f32) as u32;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let point = Point2::new(px as f32 + 0.5, py as f32 + 0.5);
            if point_in_subpaths_even_odd(point, subpaths) {
                blend_pixel(canvas, px, py, color);
            }
        }
    }
}

fn draw_filled_polylines_paint(
    canvas: &mut RgbaImage,
    subpaths: &[Vec<Point2>],
    paint: &ResolvedPaint,
    opacity: f32,
    blend: SceneBlendMode,
) {
    let Some((min_x, min_y, max_x, max_y)) = polyline_bounds(subpaths) else {
        return;
    };
    let bounds = PaintBounds {
        min_x,
        min_y,
        max_x,
        max_y,
    };
    let min_x = min_x.floor().max(0.0) as u32;
    let min_y = min_y.floor().max(0.0) as u32;
    let max_x = max_x.ceil().min(canvas.width() as f32) as u32;
    let max_y = max_y.ceil().min(canvas.height() as f32) as u32;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let point = Point2::new(px as f32 + 0.5, py as f32 + 0.5);
            if point_in_subpaths_even_odd(point, subpaths)
                && let Some(src) = sample_paint(paint, point, bounds, opacity)
            {
                blend_pixel_with_mode(canvas, px, py, src, blend);
            }
        }
    }
}

fn draw_transformed_filled_polylines_paint(
    canvas: &mut RgbaImage,
    subpaths: &[Vec<Point2>],
    paint: &ResolvedPaint,
    opacity: f32,
    blend: SceneBlendMode,
    transform: Affine2,
) {
    let transformed = subpaths
        .iter()
        .map(|subpath| {
            subpath
                .iter()
                .map(|point| {
                    let (x, y) = transform.transform_point(point.x, point.y);
                    Point2::new(x, y)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    draw_filled_polylines_paint(canvas, &transformed, paint, opacity, blend);
}

fn polyline_bounds(subpaths: &[Vec<Point2>]) -> Option<(f32, f32, f32, f32)> {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut any = false;
    for point in subpaths.iter().flatten() {
        any = true;
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    any.then_some((min_x, min_y, max_x, max_y))
}

fn point_in_subpaths_even_odd(point: Point2, subpaths: &[Vec<Point2>]) -> bool {
    let mut inside = false;
    for subpath in subpaths {
        if subpath.len() < 3 {
            continue;
        }
        let mut prev = *subpath.last().unwrap_or(&subpath[0]);
        for current in subpath {
            let denom = prev.y - current.y;
            if ((current.y > point.y) != (prev.y > point.y))
                && (point.x
                    < (prev.x - current.x) * (point.y - current.y)
                        / if denom.abs() <= 0.000001 {
                            0.000001
                        } else {
                            denom
                        }
                        + current.x)
            {
                inside = !inside;
            }
            prev = *current;
        }
    }
    inside
}

fn draw_line_segment(
    canvas: &mut RgbaImage,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    width: f32,
    color: [u8; 4],
) {
    if width <= 0.0 || color[3] == 0 {
        return;
    }
    let spread = width * 0.5 + 2.0;
    let min_x = (x1.min(x2) - spread).floor().max(0.0) as u32;
    let min_y = (y1.min(y2) - spread).floor().max(0.0) as u32;
    let max_x = (x1.max(x2) + spread).ceil().min(canvas.width() as f32) as u32;
    let max_y = (y1.max(y2) + spread).ceil().min(canvas.height() as f32) as u32;
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len2 = (dx * dx + dy * dy).max(0.0001);
    let half_width = width * 0.5;

    for py in min_y..max_y {
        for px in min_x..max_x {
            let cx = px as f32 + 0.5;
            let cy = py as f32 + 0.5;
            let t = (((cx - x1) * dx + (cy - y1) * dy) / len2).clamp(0.0, 1.0);
            let nearest_x = x1 + dx * t;
            let nearest_y = y1 + dy * t;
            let dist = ((cx - nearest_x).powi(2) + (cy - nearest_y).powi(2)).sqrt();
            let coverage = (half_width + 0.5 - dist).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let mut src = color;
            src[3] = ((src[3] as f32) * coverage).round().clamp(0.0, 255.0) as u8;
            if src[3] > 0 {
                blend_pixel(canvas, px, py, src);
            }
        }
    }
}

fn apply_alpha_mask(layer: &mut RgbaImage, mask: &RgbaImage) {
    let w = layer.width().min(mask.width());
    let h = layer.height().min(mask.height());
    for y in 0..h {
        for x in 0..w {
            let alpha = mask.get_pixel(x, y)[3] as f32 / 255.0;
            let pixel = layer.get_pixel_mut(x, y);
            pixel[3] = ((pixel[3] as f32) * alpha).round().clamp(0.0, 255.0) as u8;
        }
    }
}

fn composite_layer(canvas: &mut RgbaImage, layer: &RgbaImage) {
    for (x, y, pixel) in layer.enumerate_pixels() {
        if pixel[3] > 0 && x < canvas.width() && y < canvas.height() {
            blend_pixel(canvas, x, y, pixel.0);
        }
    }
}

fn composite_transformed_layer(
    canvas: &mut RgbaImage,
    layer: &RgbaImage,
    x: f32,
    y: f32,
    rotation_deg: f32,
    scale: f32,
) {
    let theta = rotation_deg.to_radians();
    let (sin_t, cos_t) = theta.sin_cos();
    for (src_x, src_y, pixel) in layer.enumerate_pixels() {
        if pixel[3] == 0 {
            continue;
        }
        let sx = src_x as f32 * scale;
        let sy = src_y as f32 * scale;
        let dx = x + sx * cos_t - sy * sin_t;
        let dy = y + sx * sin_t + sy * cos_t;
        let dst_x = dx.round() as i32;
        let dst_y = dy.round() as i32;
        if dst_x < 0 || dst_y < 0 {
            continue;
        }
        let (dst_x, dst_y) = (dst_x as u32, dst_y as u32);
        if dst_x >= canvas.width() || dst_y >= canvas.height() {
            continue;
        }
        blend_pixel(canvas, dst_x, dst_y, pixel.0);
    }
}

fn composite_layer_affine(canvas: &mut RgbaImage, layer: &RgbaImage, transform: Affine2) {
    composite_layer_affine_clipped(canvas, layer, transform, None);
}

fn composite_layer_affine_clipped(
    canvas: &mut RgbaImage,
    layer: &RgbaImage,
    transform: Affine2,
    clip: Option<CameraRect>,
) {
    let Some(inverse) = transform.inverse() else {
        return;
    };
    let w = layer.width() as f32;
    let h = layer.height() as f32;
    if w <= 0.0 || h <= 0.0 {
        return;
    }

    let corners = [
        transform.transform_point(0.0, 0.0),
        transform.transform_point(w - 1.0, 0.0),
        transform.transform_point(w - 1.0, h - 1.0),
        transform.transform_point(0.0, h - 1.0),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in corners {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    let mut x0 = (min_x.floor() as i32 - 2).clamp(0, canvas.width() as i32);
    let mut y0 = (min_y.floor() as i32 - 2).clamp(0, canvas.height() as i32);
    let mut x1 = (max_x.ceil() as i32 + 2).clamp(0, canvas.width() as i32);
    let mut y1 = (max_y.ceil() as i32 + 2).clamp(0, canvas.height() as i32);
    if let Some(clip) = clip {
        x0 = x0.max(clip.x.floor() as i32);
        y0 = y0.max(clip.y.floor() as i32);
        x1 = x1.min((clip.x + clip.width).ceil() as i32);
        y1 = y1.min((clip.y + clip.height).ceil() as i32);
    }
    if x1 <= x0 || y1 <= y0 {
        return;
    }

    for dst_y in y0..y1 {
        for dst_x in x0..x1 {
            let (src_x, src_y) = inverse.transform_point(dst_x as f32, dst_y as f32);
            let Some(pixel) = sample_layer_bilinear(layer, src_x, src_y) else {
                continue;
            };
            if pixel[3] == 0 {
                continue;
            }
            blend_pixel(canvas, dst_x as u32, dst_y as u32, pixel);
        }
    }
}

fn sample_layer_bilinear(layer: &RgbaImage, x: f32, y: f32) -> Option<[u8; 4]> {
    if x < -0.5 || y < -0.5 || x > layer.width() as f32 - 0.5 || y > layer.height() as f32 - 0.5 {
        return None;
    }

    let x = x.clamp(0.0, layer.width().saturating_sub(1) as f32);
    let y = y.clamp(0.0, layer.height().saturating_sub(1) as f32);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(layer.width().saturating_sub(1));
    let y1 = (y0 + 1).min(layer.height().saturating_sub(1));
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;

    let samples = [
        (layer.get_pixel(x0, y0).0, (1.0 - tx) * (1.0 - ty)),
        (layer.get_pixel(x1, y0).0, tx * (1.0 - ty)),
        (layer.get_pixel(x0, y1).0, (1.0 - tx) * ty),
        (layer.get_pixel(x1, y1).0, tx * ty),
    ];

    let mut premul = [0.0_f32; 3];
    let mut alpha = 0.0_f32;
    for (rgba, weight) in samples {
        let a = rgba[3] as f32 / 255.0;
        alpha += a * weight;
        premul[0] += rgba[0] as f32 * a * weight;
        premul[1] += rgba[1] as f32 * a * weight;
        premul[2] += rgba[2] as f32 * a * weight;
    }
    if alpha <= 0.0001 {
        return None;
    }

    Some([
        (premul[0] / alpha).round().clamp(0.0, 255.0) as u8,
        (premul[1] / alpha).round().clamp(0.0, 255.0) as u8,
        (premul[2] / alpha).round().clamp(0.0, 255.0) as u8,
        (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    ])
}

const MAX_REMOTE_ASSET_BYTES: u64 = 64 * 1024 * 1024;

fn load_rgba_image_source(src: &str) -> Result<RgbaImage, MotionLoomSceneRenderError> {
    if is_remote_image_source(src) {
        let bytes = fetch_remote_asset_bytes(src)?;
        return image::load_from_memory(&bytes)
            .map_err(|source| MotionLoomSceneRenderError::DecodeImage {
                source_ref: src.to_string(),
                source,
            })
            .map(|decoded| decoded.to_rgba8());
    }

    let path = Path::new(src);
    image::open(path)
        .map_err(|source| MotionLoomSceneRenderError::OpenImage {
            path: path.to_path_buf(),
            source,
        })
        .map(|decoded| decoded.to_rgba8())
}

fn load_svg_source(src: &str) -> Result<RgbaImage, MotionLoomSceneRenderError> {
    let (bytes, resources_dir) = if is_svg_data_uri(src) {
        (decode_svg_data_uri(src)?, None)
    } else if is_remote_image_source(src) {
        (fetch_remote_asset_bytes(src)?, None)
    } else {
        let path = Path::new(src);
        let bytes = fs::read(path).map_err(|source| MotionLoomSceneRenderError::ReadSvg {
            path: path.to_path_buf(),
            source,
        })?;
        (bytes, path.parent().map(Path::to_path_buf))
    };

    render_svg_bytes(src, &bytes, resources_dir)
}

fn is_svg_data_uri(src: &str) -> bool {
    src.trim_start()
        .to_ascii_lowercase()
        .starts_with("data:image/svg+xml")
}

fn decode_svg_data_uri(src: &str) -> Result<Vec<u8>, MotionLoomSceneRenderError> {
    let trimmed = src.trim_start();
    let Some(comma_ix) = trimmed.find(',') else {
        return Err(MotionLoomSceneRenderError::InvalidSvgDataUri {
            source_ref: src.to_string(),
            message: "missing data payload separator ','".to_string(),
        });
    };
    let (header, payload) = trimmed.split_at(comma_ix);
    let payload = &payload[1..];
    let header_lower = header.to_ascii_lowercase();

    if !header_lower.starts_with("data:image/svg+xml") {
        return Err(MotionLoomSceneRenderError::InvalidSvgDataUri {
            source_ref: src.to_string(),
            message: "expected data:image/svg+xml media type".to_string(),
        });
    }

    if header_lower.contains(";base64") {
        return base64::engine::general_purpose::STANDARD
            .decode(payload)
            .map_err(|err| MotionLoomSceneRenderError::InvalidSvgDataUri {
                source_ref: src.to_string(),
                message: format!("base64 decode failed: {err}"),
            });
    }

    percent_decode_bytes(payload.as_bytes()).map_err(|message| {
        MotionLoomSceneRenderError::InvalidSvgDataUri {
            source_ref: src.to_string(),
            message,
        }
    })
}

fn percent_decode_bytes(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0usize;
    while i < input.len() {
        let ch = input[i];
        if ch == b'%' {
            if i + 2 >= input.len() {
                return Err("truncated percent escape".to_string());
            }
            let hi = decode_hex_nibble(input[i + 1])
                .ok_or_else(|| "invalid percent escape".to_string())?;
            let lo = decode_hex_nibble(input[i + 2])
                .ok_or_else(|| "invalid percent escape".to_string())?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(ch);
            i += 1;
        }
    }
    Ok(out)
}

fn decode_hex_nibble(ch: u8) -> Option<u8> {
    match ch {
        b'0'..=b'9' => Some(ch - b'0'),
        b'a'..=b'f' => Some(ch - b'a' + 10),
        b'A'..=b'F' => Some(ch - b'A' + 10),
        _ => None,
    }
}

fn render_svg_bytes(
    source_ref: &str,
    bytes: &[u8],
    resources_dir: Option<PathBuf>,
) -> Result<RgbaImage, MotionLoomSceneRenderError> {
    let mut options = resvg::usvg::Options::default();
    options.resources_dir = resources_dir;
    options.fontdb_mut().load_system_fonts();

    let tree = resvg::usvg::Tree::from_data(bytes, &options).map_err(|source| {
        MotionLoomSceneRenderError::ParseSvg {
            source_ref: source_ref.to_string(),
            source,
        }
    })?;
    let svg_size = tree.size();
    let width = svg_size.width().ceil().max(1.0) as u32;
    let height = svg_size.height().ceil().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or_else(|| {
        MotionLoomSceneRenderError::RenderSvg {
            source_ref: source_ref.to_string(),
        }
    })?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        width as f32 / svg_size.width(),
        height as f32 / svg_size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    rgba_image_from_pixmap(pixmap, source_ref)
}

fn rgba_image_from_pixmap(
    pixmap: resvg::tiny_skia::Pixmap,
    source_ref: &str,
) -> Result<RgbaImage, MotionLoomSceneRenderError> {
    let width = pixmap.width();
    let height = pixmap.height();
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        rgba.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
    }
    RgbaImage::from_raw(width, height, rgba).ok_or_else(|| MotionLoomSceneRenderError::RenderSvg {
        source_ref: source_ref.to_string(),
    })
}

fn is_remote_image_source(src: &str) -> bool {
    url::Url::parse(src)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

fn fetch_remote_asset_bytes(src: &str) -> Result<Vec<u8>, MotionLoomSceneRenderError> {
    let response = ureq::get(src)
        .call()
        .map_err(|err| MotionLoomSceneRenderError::FetchAsset {
            url: src.to_string(),
            message: format_ureq_error(err),
        })?;
    let mut reader = response.into_reader().take(MAX_REMOTE_ASSET_BYTES);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|source| MotionLoomSceneRenderError::FetchAsset {
            url: src.to_string(),
            message: source.to_string(),
        })?;
    if bytes.is_empty() {
        return Err(MotionLoomSceneRenderError::FetchAsset {
            url: src.to_string(),
            message: "response body was empty".to_string(),
        });
    }
    Ok(bytes)
}

fn format_ureq_error(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, response) => {
            format!("HTTP {code} {}", response.status_text())
        }
        other => other.to_string(),
    }
}

fn resolve_gradient_paint(
    source_value: &str,
    gradient: &GradientDef,
) -> Result<ResolvedGradient, MotionLoomSceneRenderError> {
    match gradient {
        GradientDef::Linear(linear) => Ok(ResolvedGradient::Linear {
            x1: parse_gradient_number(&linear.x1, 0.0),
            y1: parse_gradient_number(&linear.y1, 0.0),
            x2: parse_gradient_number(&linear.x2, 1.0),
            y2: parse_gradient_number(&linear.y2, 0.0),
            stops: resolve_gradient_stops(source_value, &linear.stops)?,
            units: parse_gradient_units(&linear.units),
        }),
        GradientDef::Radial(radial) => Ok(ResolvedGradient::Radial {
            cx: parse_gradient_number(&radial.cx, 0.5),
            cy: parse_gradient_number(&radial.cy, 0.5),
            r: parse_gradient_number(&radial.r, 0.5).max(0.0001),
            stops: resolve_gradient_stops(source_value, &radial.stops)?,
            units: parse_gradient_units(&radial.units),
        }),
    }
}

fn resolve_gradient_stops(
    source_value: &str,
    stops: &[GradientStop],
) -> Result<Vec<ResolvedGradientStop>, MotionLoomSceneRenderError> {
    stops
        .iter()
        .map(|stop| {
            Ok(ResolvedGradientStop {
                offset: stop.offset.clamp(0.0, 1.0),
                color: parse_color(&stop.color).map_err(|err| {
                    MotionLoomSceneRenderError::InvalidPaint {
                        value: source_value.to_string(),
                        message: err.to_string(),
                    }
                })?,
            })
        })
        .collect()
}

fn parse_gradient_units(value: &str) -> GradientUnits {
    match value.trim().to_ascii_lowercase().as_str() {
        "userspace" | "user-space" | "userspaceonuse" | "user-space-on-use" => {
            GradientUnits::UserSpace
        }
        _ => GradientUnits::ObjectBoundingBox,
    }
}

fn parse_gradient_number(value: &str, default: f32) -> f32 {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        return percent
            .trim()
            .parse::<f32>()
            .map(|v| v / 100.0)
            .unwrap_or(default);
    }
    value.parse::<f32>().unwrap_or(default)
}

fn gradient_ref_id(value: &str) -> Option<&str> {
    let value = value.trim();
    let rest = value.strip_prefix("url(#")?;
    rest.strip_suffix(')')
}

fn is_normal_blend(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "normal" | "over" | "source-over"
    )
}

fn parse_scene_blend(value: &str) -> Result<SceneBlendMode, MotionLoomSceneRenderError> {
    match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "" | "normal" | "over" | "source-over" => Ok(SceneBlendMode::Normal),
        "multiply" => Ok(SceneBlendMode::Multiply),
        "screen" => Ok(SceneBlendMode::Screen),
        "add" | "plus" | "linear-dodge" => Ok(SceneBlendMode::Add),
        other => Err(MotionLoomSceneRenderError::InvalidPaint {
            value: value.to_string(),
            message: format!("unsupported blend mode: {other}"),
        }),
    }
}

fn parse_paint(value: &str) -> Result<Option<[u8; 4]>, MotionLoomSceneRenderError> {
    if is_none_paint(value) {
        return Ok(None);
    }
    let color = parse_color(value)?;
    if color[3] == 0 {
        return Ok(None);
    }
    Ok(Some(color))
}

fn is_none_paint(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.is_empty() || value == "none" || value == "transparent"
}

fn parse_color(value: &str) -> Result<[u8; 4], MotionLoomSceneRenderError> {
    let trimmed = value.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return parse_bgra_array_color(trimmed, value);
    }

    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "black" => return Ok([0, 0, 0, 255]),
        "white" => return Ok([255, 255, 255, 255]),
        "red" => return Ok([255, 0, 0, 255]),
        "green" => return Ok([0, 255, 0, 255]),
        "blue" => return Ok([0, 0, 255, 255]),
        "transparent" => return Ok([0, 0, 0, 0]),
        _ => {}
    }

    let hex = lower
        .strip_prefix('#')
        .or_else(|| lower.strip_prefix("0x"))
        .unwrap_or(lower.as_str());
    let expanded;
    let hex = if hex.len() == 3 {
        expanded = hex.chars().flat_map(|ch| [ch, ch]).collect::<String>();
        expanded.as_str()
    } else {
        hex
    };
    if hex.len() != 6 && hex.len() != 8 {
        return Err(MotionLoomSceneRenderError::InvalidColor {
            value: value.to_string(),
        });
    }
    let r = parse_hex_byte(hex, 0, value)?;
    let g = parse_hex_byte(hex, 2, value)?;
    let b = parse_hex_byte(hex, 4, value)?;
    let a = if hex.len() == 8 {
        parse_hex_byte(hex, 6, value)?
    } else {
        255
    };
    Ok([r, g, b, a])
}

fn parse_bgra_array_color(
    value: &str,
    original: &str,
) -> Result<[u8; 4], MotionLoomSceneRenderError> {
    let inner = value
        .strip_prefix('[')
        .and_then(|text| text.strip_suffix(']'))
        .ok_or_else(|| MotionLoomSceneRenderError::InvalidColor {
            value: original.to_string(),
        })?;
    let parts = inner
        .split(',')
        .map(str::trim)
        .map(|part| part.parse::<f32>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| MotionLoomSceneRenderError::InvalidColor {
            value: original.to_string(),
        })?;
    if parts.len() != 4 || parts.iter().any(|component| !component.is_finite()) {
        return Err(MotionLoomSceneRenderError::InvalidColor {
            value: original.to_string(),
        });
    }

    let uses_byte_range = parts.iter().any(|component| *component > 1.0);
    let to_byte = |component: f32| {
        let scaled = if uses_byte_range {
            component
        } else {
            component * 255.0
        };
        scaled.round().clamp(0.0, 255.0) as u8
    };

    let b = to_byte(parts[0]);
    let g = to_byte(parts[1]);
    let r = to_byte(parts[2]);
    let a = to_byte(parts[3]);
    Ok([r, g, b, a])
}

fn parse_hex_byte(
    hex: &str,
    start: usize,
    original: &str,
) -> Result<u8, MotionLoomSceneRenderError> {
    u8::from_str_radix(&hex[start..start + 2], 16).map_err(|_| {
        MotionLoomSceneRenderError::InvalidColor {
            value: original.to_string(),
        }
    })
}

fn draw_rgba_image(canvas: &mut RgbaImage, image: &RgbaImage, x: f32, y: f32, opacity: f32) {
    let base_x = x.round() as i32;
    let base_y = y.round() as i32;
    for (src_x, src_y, pixel) in image.enumerate_pixels() {
        let dst_x = base_x + src_x as i32;
        let dst_y = base_y + src_y as i32;
        if dst_x < 0 || dst_y < 0 {
            continue;
        }
        let (dst_x, dst_y) = (dst_x as u32, dst_y as u32);
        if dst_x >= canvas.width() || dst_y >= canvas.height() {
            continue;
        }
        let mut src = pixel.0;
        src[3] = ((src[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
        if src[3] == 0 {
            continue;
        }
        blend_pixel(canvas, dst_x, dst_y, src);
    }
}

fn sample_paint(
    paint: &ResolvedPaint,
    point: Point2,
    bounds: PaintBounds,
    opacity: f32,
) -> Option<[u8; 4]> {
    let mut color = match paint {
        ResolvedPaint::None => return None,
        ResolvedPaint::Solid(color) => *color,
        ResolvedPaint::Gradient(gradient) => sample_gradient(gradient, point, bounds),
    };
    color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
    (color[3] > 0).then_some(color)
}

fn sample_gradient(gradient: &ResolvedGradient, point: Point2, bounds: PaintBounds) -> [u8; 4] {
    match gradient {
        ResolvedGradient::Linear {
            x1,
            y1,
            x2,
            y2,
            stops,
            units,
        } => {
            let (px, py, sx, sy, ex, ey) = match units {
                GradientUnits::ObjectBoundingBox => {
                    let w = (bounds.max_x - bounds.min_x).max(0.0001);
                    let h = (bounds.max_y - bounds.min_y).max(0.0001);
                    (
                        (point.x - bounds.min_x) / w,
                        (point.y - bounds.min_y) / h,
                        *x1,
                        *y1,
                        *x2,
                        *y2,
                    )
                }
                GradientUnits::UserSpace => (point.x, point.y, *x1, *y1, *x2, *y2),
            };
            let dx = ex - sx;
            let dy = ey - sy;
            let len2 = (dx * dx + dy * dy).max(0.000001);
            let t = (((px - sx) * dx + (py - sy) * dy) / len2).clamp(0.0, 1.0);
            sample_gradient_stops(stops, t)
        }
        ResolvedGradient::Radial {
            cx,
            cy,
            r,
            stops,
            units,
        } => {
            let t = match units {
                GradientUnits::ObjectBoundingBox => {
                    let w = (bounds.max_x - bounds.min_x).max(0.0001);
                    let h = (bounds.max_y - bounds.min_y).max(0.0001);
                    let px = (point.x - bounds.min_x) / w;
                    let py = (point.y - bounds.min_y) / h;
                    let aspect = if w > h { h / w } else { 1.0 };
                    let dx = (px - *cx) / aspect.max(0.0001);
                    let dy = py - *cy;
                    ((dx * dx + dy * dy).sqrt() / *r).clamp(0.0, 1.0)
                }
                GradientUnits::UserSpace => {
                    let dx = point.x - *cx;
                    let dy = point.y - *cy;
                    ((dx * dx + dy * dy).sqrt() / *r).clamp(0.0, 1.0)
                }
            };
            sample_gradient_stops(stops, t)
        }
    }
}

fn sample_gradient_stops(stops: &[ResolvedGradientStop], t: f32) -> [u8; 4] {
    let Some(first) = stops.first() else {
        return [0, 0, 0, 0];
    };
    if t <= first.offset {
        return first.color;
    }
    for pair in stops.windows(2) {
        let a = pair[0];
        let b = pair[1];
        if t <= b.offset {
            let span = (b.offset - a.offset).max(0.000001);
            let local_t = ((t - a.offset) / span).clamp(0.0, 1.0);
            return lerp_color(a.color, b.color, local_t);
        }
    }
    stops.last().map(|stop| stop.color).unwrap_or(first.color)
}

fn lerp_color(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8,
        (a[3] as f32 + (b[3] as f32 - a[3] as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8,
    ]
}

fn blend_pixel_with_mode(
    canvas: &mut RgbaImage,
    x: u32,
    y: u32,
    src: [u8; 4],
    mode: SceneBlendMode,
) {
    if mode == SceneBlendMode::Normal {
        blend_pixel(canvas, x, y, src);
        return;
    }

    let dst = canvas.get_pixel_mut(x, y);
    let sa = src[3] as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let sr = src[0] as f32 / 255.0;
    let sg = src[1] as f32 / 255.0;
    let sb = src[2] as f32 / 255.0;
    let dr = dst[0] as f32 / 255.0;
    let dg = dst[1] as f32 / 255.0;
    let db = dst[2] as f32 / 255.0;
    let blend_channel = |s: f32, d: f32| match mode {
        SceneBlendMode::Normal => s,
        SceneBlendMode::Multiply => s * d,
        SceneBlendMode::Screen => 1.0 - (1.0 - s) * (1.0 - d),
        SceneBlendMode::Add => (s + d).min(1.0),
    };
    let br = blend_channel(sr, dr);
    let bg = blend_channel(sg, dg);
    let bb = blend_channel(sb, db);
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        *dst = Rgba([0, 0, 0, 0]);
        return;
    }

    let out_r = (br * sa + dr * da * (1.0 - sa)) / out_a;
    let out_g = (bg * sa + dg * da * (1.0 - sa)) / out_a;
    let out_b = (bb * sa + db * da * (1.0 - sa)) / out_a;
    *dst = Rgba([
        (out_r * 255.0).round().clamp(0.0, 255.0) as u8,
        (out_g * 255.0).round().clamp(0.0, 255.0) as u8,
        (out_b * 255.0).round().clamp(0.0, 255.0) as u8,
        (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
    ]);
}

fn blend_pixel(canvas: &mut RgbaImage, x: u32, y: u32, src: [u8; 4]) {
    let dst = canvas.get_pixel_mut(x, y);
    let (sr, sg, sb, sa) = (src[0] as f32, src[1] as f32, src[2] as f32, src[3] as f32);
    let (dr, dg, db, da) = (dst[0] as f32, dst[1] as f32, dst[2] as f32, dst[3] as f32);

    let sa = sa / 255.0;
    let da = da / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        *dst = Rgba([0, 0, 0, 0]);
        return;
    }

    let out_r = (sr * sa + dr * da * (1.0 - sa)) / out_a;
    let out_g = (sg * sa + dg * da * (1.0 - sa)) / out_a;
    let out_b = (sb * sa + db * da * (1.0 - sa)) / out_a;

    *dst = Rgba([
        out_r.round().clamp(0.0, 255.0) as u8,
        out_g.round().clamp(0.0, 255.0) as u8,
        out_b.round().clamp(0.0, 255.0) as u8,
        (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
    ]);
}

#[cfg(test)]
mod tests {
    use crate::parse_graph_script;
    use image::{Rgba, RgbaImage};
    use std::path::PathBuf;

    use super::{MotionLoomSceneRenderError, SceneFrameRenderer, SceneRenderProfile};

    fn max_rgb(image: &image::RgbaImage) -> u8 {
        image
            .pixels()
            .map(|pixel| pixel[0].max(pixel[1]).max(pixel[2]))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn scene_color_parser_accepts_bgra_normalized_array() {
        assert_eq!(super::parse_color("[1,0,0,1]").unwrap(), [0, 0, 255, 255]);
        assert_eq!(super::parse_color("[0,0,1,1]").unwrap(), [255, 0, 0, 255]);
        assert_eq!(super::parse_color("[0,1,0,0.5]").unwrap(), [0, 255, 0, 128]);
    }

    #[test]
    fn scene_color_parser_accepts_bgra_byte_array() {
        assert_eq!(
            super::parse_color("[255, 128, 0, 64]").unwrap(),
            [0, 128, 255, 64]
        );
    }

    #[test]
    fn scene_renderer_draws_scene_group_shapes_and_text() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[220,140]}>
  <Scene id="scene0">
    <Solid color="[1,1,1,1]" />
    <Group id="card" x="20" y="20" opacity="1">
      <Shadow x="0" y="10" blur="18" color="[0,0,0,0.24]" />
      <Rect width="100" height="58" radius="8" color="[0,0,1,1]" />
      <Circle x="22" y="29" radius="9" color="[1,0,0,1]" />
      <Text value="Card" x="40" y="16" width="50" fontSize="18" lineHeight="22" color="[0,0,0,1]" />
    </Group>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let rendered = renderer.render_frame(&graph, 0).expect("frame 0");
        let rect_pixel = rendered.get_pixel(112, 70);
        let circle_pixel = rendered.get_pixel(42, 49);

        assert!(
            rect_pixel[0] > 200 && rect_pixel[1] < 40 && rect_pixel[2] < 40,
            "expected red rect pixel, got {rect_pixel:?}"
        );
        assert!(
            circle_pixel[2] > 200 && circle_pixel[0] < 40 && circle_pixel[1] < 40,
            "expected blue circle pixel, got {circle_pixel:?}"
        );
    }

    #[test]
    fn scene_renderer_fits_render_size_into_output_size() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[100,100]} renderSize={[200,100]}>
  <Scene id="scene0">
    <Solid color="#000000" />
    <Circle x="50" y="50" radius="10" color="#ff0000" />
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let rendered = renderer.render_frame(&graph, 0).expect("frame 0");
        let center = rendered.get_pixel(100, 50);
        let left_logical_position = rendered.get_pixel(50, 50);

        assert!(
            center[0] > 200 && center[1] < 40 && center[2] < 40,
            "expected renderSize content to be centered and scaled into output, got {center:?}"
        );
        assert!(
            left_logical_position[0] < 40
                && left_logical_position[1] < 40
                && left_logical_position[2] < 40,
            "expected untransformed logical position to be background, got {left_logical_position:?}"
        );
    }

    #[test]
    fn scene_renderer_draws_trimmed_polyline_and_path() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[160,90]}>
  <Scene id="scene0">
    <Solid color="#000000" />
    <Polyline points="10,24 110,24"
              stroke="#2f83ff"
              strokeWidth="8"
              trimStart="0"
              trimEnd="0.5" />
    <Path d="M 10 66 L 110 66"
          stroke="#ff2f2f"
          strokeWidth="8"
          trimStart="0.5"
          trimEnd="1" />
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let rendered = renderer.render_frame(&graph, 0).expect("frame 0");
        let polyline_drawn = rendered.get_pixel(50, 24);
        let polyline_trimmed = rendered.get_pixel(95, 24);
        let path_trimmed = rendered.get_pixel(25, 66);
        let path_drawn = rendered.get_pixel(90, 66);

        assert!(
            polyline_drawn[2] > 180 && polyline_drawn[0] < 100,
            "expected blue polyline pixel, got {polyline_drawn:?}"
        );
        assert!(
            polyline_trimmed[0] < 20 && polyline_trimmed[1] < 20 && polyline_trimmed[2] < 20,
            "expected trimmed polyline tail to stay black, got {polyline_trimmed:?}"
        );
        assert!(
            path_trimmed[0] < 20 && path_trimmed[1] < 20 && path_trimmed[2] < 20,
            "expected trimmed path head to stay black, got {path_trimmed:?}"
        );
        assert!(
            path_drawn[0] > 180 && path_drawn[2] < 100,
            "expected red path pixel, got {path_drawn:?}"
        );
    }

    #[test]
    fn scene_renderer_draws_brush_part_path() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[120,80]}>
  <Scene id="scene0">
    <Solid color="#000000" />
    <Defs>
      <Brush id="red_ink" stroke="#ff0000" strokeWidth="6" opacity="1" />
    </Defs>
    <Part id="mark" brush="red_ink" x="20" y="30">
      <Path id="mark_line" d="M 0 0 L 80 0" />
    </Part>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let rendered = renderer.render_frame(&graph, 0).expect("frame 0");
        let pixel = rendered.get_pixel(60, 30);

        assert!(
            pixel[0] > 180 && pixel[1] < 80 && pixel[2] < 80,
            "expected red brush path pixel, got {pixel:?}"
        );
    }

    #[test]
    fn scene_renderer_draws_character_vector_nodes() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[120,90]}>
  <Scene id="scene0">
    <Solid color="#000000" />
    <Character id="face" x="60" y="45" scale="1.5">
      <Path d="M -30 0 C -12 -20 12 -20 30 0"
            stroke="#ff77aa"
            strokeWidth="8" />
      <Line x1="-20" y1="12" x2="20" y2="12" width="6" color="#00ff00" />
      <Circle x="0" y="-5" radius="6" color="#ffffff" />
    </Character>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let rendered = renderer.render_frame(&graph, 0).expect("frame 0");
        let eye = rendered.get_pixel(60, 37);
        let line = rendered.get_pixel(60, 63);

        assert!(
            eye[0] > 200 && eye[1] > 200 && eye[2] > 200,
            "expected white character circle, got {eye:?}"
        );
        assert!(
            line[1] > 180 && line[0] < 80 && line[2] < 80,
            "expected green character line, got {line:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_draws_character_overlay() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[100,80]}>
  <Scene id="scene0">
    <Solid color="#000000" />
    <Character x="50" y="40">
      <Path d="M -24 0 L 24 0" stroke="#00ff00" strokeWidth="8" />
    </Character>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu);
        let rendered = match renderer.render_frame(&graph, 0) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU character overlay test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let center = rendered.get_pixel(50, 40);

        assert!(
            center[1] > 180 && center[0] < 80 && center[2] < 80,
            "expected green GPU character path pixel, got {center:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_draws_filled_path_overlay() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[100,80]}>
  <Scene id="scene0">
    <Solid color="#000000" />
    <Path d="M 20 20 L 70 20 L 70 60 L 20 60 Z"
          fill="#00ff00"
          stroke="none" />
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu);
        let rendered = match renderer.render_frame(&graph, 0) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU filled path overlay test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let center = rendered.get_pixel(45, 40);

        assert!(
            center[1] > 180 && center[0] < 80 && center[2] < 80,
            "expected green GPU filled path overlay pixel, got {center:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_draws_sketch_stroke_style() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[100,80]}>
  <Scene id="scene0">
    <Solid color="#000000" />
    <Line x1="12" y1="40" x2="88" y2="40"
          width="8"
          color="#00ff00"
          strokeStyle="sketch"
          strokeRoughness="1.4"
          strokeCopies="4"
          strokeTexture="0.35"
          strokeBristles="3"
          strokePressure="auto"
          strokePressureMin="0.4"
          strokePressureCurve="1.2" />
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu);
        let rendered = match renderer.render_frame(&graph, 0) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU sketch stroke style test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let center = rendered.get_pixel(50, 40);

        assert!(
            center[1] > 150 && center[0] < 100 && center[2] < 100,
            "expected green GPU sketch line pixel, got {center:?}"
        );
    }

    #[test]
    fn scene_renderer_draws_filled_path_and_mask() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[120,90]}>
  <Scene id="scene0">
    <Solid color="#000000" />
    <Path d="M 10 10 L 45 10 L 45 45 L 10 45 Z"
          fill="#00ff00"
          stroke="none" />
    <Mask shape="circle" x="88" y="42" radius="18">
      <Rect x="65" y="20" width="46" height="46" color="#ff0000" />
    </Mask>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let rendered = renderer.render_frame(&graph, 0).expect("frame 0");
        let filled = rendered.get_pixel(28, 28);
        let masked_inside = rendered.get_pixel(88, 42);
        let masked_outside = rendered.get_pixel(66, 22);

        assert!(
            filled[1] > 180 && filled[0] < 80 && filled[2] < 80,
            "expected green filled path pixel, got {filled:?}"
        );
        assert!(
            masked_inside[0] > 180 && masked_inside[1] < 80 && masked_inside[2] < 80,
            "expected red masked center, got {masked_inside:?}"
        );
        assert!(
            masked_outside[0] < 40 && masked_outside[1] < 40 && masked_outside[2] < 40,
            "expected masked corner to stay black, got {masked_outside:?}"
        );
    }

    #[test]
    fn scene_renderer_character_mask_clips_filled_path() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[120,90]}>
  <Scene id="scene0">
    <Solid color="#000000" />
    <Character x="60" y="45">
      <Mask shape="circle" x="0" y="0" radius="18">
        <Path d="M -30 -30 L 30 -30 L 30 30 L -30 30 Z"
              fill="#ff77aa"
              stroke="none" />
      </Mask>
    </Character>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let rendered = renderer.render_frame(&graph, 0).expect("frame 0");
        let inside = rendered.get_pixel(60, 45);
        let outside = rendered.get_pixel(35, 45);

        assert!(
            inside[0] > 180 && inside[2] > 100,
            "expected pink character mask fill, got {inside:?}"
        );
        assert!(
            outside[0] < 40 && outside[1] < 40 && outside[2] < 40,
            "expected clipped outside pixel to stay black, got {outside:?}"
        );
    }

    #[test]
    fn scene_renderer_camera_centers_world_coordinate() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[100,80]}>
  <Scene id="scene0">
    <Solid color="#000000" />
    <Camera x="100" y="40" zoom="1">
      <Circle x="100" y="40" radius="8" color="#00ff00" />
    </Camera>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let rendered = renderer.render_frame(&graph, 0).expect("frame 0");
        let centered = rendered.get_pixel(50, 40);

        assert!(
            centered[1] > 180 && centered[0] < 80 && centered[2] < 80,
            "expected camera-centered green circle, got {centered:?}"
        );
    }

    #[test]
    fn scene_renderer_camera_follow_maps_node_to_anchor() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[100,80]}>
  <Scene id="scene0">
    <Solid color="#000000" />
    <Camera mode="2d" follow="marker" anchorX="25%" anchorY="75%" zoom="1" worldBounds="0,0,200,160">
      <Circle id="marker" x="120" y="60" radius="8" color="#00ff00" />
    </Camera>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let rendered = renderer.render_frame(&graph, 0).expect("frame 0");
        let anchored = rendered.get_pixel(25, 60);

        assert!(
            anchored[1] > 180 && anchored[0] < 80 && anchored[2] < 80,
            "expected camera-followed green circle at anchor, got {anchored:?}"
        );
    }

    #[test]
    fn scene_renderer_accepts_scene_tex_pass_present_pipeline() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[64,48]}>
  <Scene id="scene0">
    <Solid color="[0,0,1,1]" />
  </Scene>
  <Tex id="src" fmt="rgba16f" from="scene:scene0" />
  <Tex id="out" fmt="rgba16f" size={[64,48]} />
  <Pass id="fx_opacity" kind="compute"
        effect="opacity"
        in={["src"]} out={["out"]}
        params={{ opacity: "0.5" }} />
  <Present from="out" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let rendered = renderer.render_frame(&graph, 0).expect("frame 0");
        let pixel = rendered.get_pixel(10, 10);

        assert!(
            pixel[0] > 200 && pixel[1] < 40 && pixel[2] < 40,
            "expected red scene pixel, got {pixel:?}"
        );
        assert!(
            (120..=136).contains(&pixel[3]),
            "expected opacity pass to halve alpha, got {pixel:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_draws_scene_group_shapes_and_text() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[220,140]}>
  <Scene id="scene0">
    <Solid color="[1,1,1,1]" />
    <Group id="card" x="20" y="20" opacity="1">
      <Shadow x="0" y="10" blur="18" color="[0,0,0,0.24]" />
      <Rect width="100" height="58" radius="8" color="[0,0,1,1]" />
      <Circle x="22" y="29" radius="9" color="[1,0,0,1]" />
      <Text value="Card" x="40" y="16" width="50" fontSize="18" lineHeight="22" color="[0,0,0,1]" />
    </Group>
  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu);
        let rendered = match renderer.render_frame(&graph, 0) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU scene primitive test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let rect_pixel = rendered.get_pixel(112, 70);
        let circle_pixel = rendered.get_pixel(42, 49);

        assert!(
            rect_pixel[0] > 200 && rect_pixel[1] < 40 && rect_pixel[2] < 40,
            "expected red rect pixel, got {rect_pixel:?}"
        );
        assert!(
            circle_pixel[2] > 200 && circle_pixel[0] < 40 && circle_pixel[1] < 40,
            "expected blue circle pixel, got {circle_pixel:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_batches_many_gradient_paths() {
        let mut script = String::from(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[240,160]}>
  <Scene id="scene0">
    <Solid color="#ffffff" />
    <Defs>
      <LinearGradient id="sclera_soft" x1="0" y1="0" x2="0" y2="1"
                      stops="0:#D3CEE6, 0.30:#F7F7F7, 0.70:#F7F7F7, 1:#ffffff" />
      <LinearGradient id="lid_grey" x1="0" y1="0" x2="0" y2="1"
                      stops="0:#B7C7C7, 0.70:#B7C7C7, 1:#8da0a3" />
    </Defs>
"##,
        );
        for ix in 0..72 {
            let x = 18 + (ix % 12) * 17;
            let y = 20 + (ix / 12) * 18;
            script.push_str(&format!(
                r##"    <Path d="M {x} {y} C {cx1} {cy1} {cx2} {cy2} {x2} {y2} C {cx3} {cy3} {cx4} {cy4} {x} {y} Z"
          fill="url(#sclera_soft)" stroke="#3F5877" strokeWidth="1.2" opacity="0.78" />
"##,
                cx1 = x + 8,
                cy1 = y - 8,
                cx2 = x + 28,
                cy2 = y - 8,
                x2 = x + 36,
                y2 = y,
                cx3 = x + 28,
                cy3 = y + 12,
                cx4 = x + 8,
                cy4 = y + 12,
            ));
            script.push_str(&format!(
                r##"    <Path d="M {x} {line_y} C {cx1} {cy1} {cx2} {cy2} {x2} {line_y}"
          fill="none" stroke="#3F5877" strokeWidth="2.4" lineCap="round" />
"##,
                line_y = y + 6,
                cx1 = x + 8,
                cy1 = y - 5,
                cx2 = x + 28,
                cy2 = y - 5,
                x2 = x + 36,
            ));
        }
        script.push_str(
            r##"  </Scene>
  <Present from="scene0" />
</Graph>
"##,
        );

        let graph = parse_graph_script(&script).expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu);
        let rendered = match renderer.render_frame(&graph, 0) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU batched path stress test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };

        assert!(
            rendered
                .pixels()
                .any(|pixel| pixel[0] < 120 && pixel[1] < 140 && pixel[2] < 170),
            "expected batched gradient paths to render visible dark strokes"
        );
    }

    #[test]
    fn scene_gpu_renderer_applies_scene_blur_post_pass() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[80,40]}>
  <Scene id="scene0">
    <Solid color="#ffffff" />
    <Rect x="35" y="10" width="10" height="20" color="#000000" />
  </Scene>
  <Tex id="src" fmt="rgba16f" from="scene0" />
  <Tex id="out" fmt="rgba16f" size={[80,40]} />
  <Pass id="blur_h" kind="compute"
        effect="gaussian_5tap_h"
        in={["src"]} out={["out"]}
        params={{ sigma: "8" }} />
  <Present from="out" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu);
        let rendered = match renderer.render_frame(&graph, 0) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU scene post-pass test: {message}");
                return;
            }
            Err(err) => panic!("unexpected render error: {err}"),
        };
        let softened_edge = rendered.get_pixel(31, 20);

        assert!(
            softened_edge[0] < 245 && softened_edge[1] < 245 && softened_edge[2] < 245,
            "expected GPU blur to soften edge pixel, got {softened_edge:?}"
        );
    }

    fn write_test_svg(name: &str, body: &str) -> PathBuf {
        let svg_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-{name}-{}.svg",
            std::process::id()
        ));
        std::fs::write(&svg_path, body).expect("write test svg");
        svg_path
    }

    #[test]
    fn scene_text_opacity_fades_in_over_time() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="3s" size={[640,360]}>
  <Solid color="#000000" />
  <Text value="hello world"
        x="center"
        y="center"
        fontSize="72"
        color="#ffffff"
        opacity="min($time.sec / 1.0, 1.0)" />
  <Present from="scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let at_zero = renderer.render_frame(&graph, 0).expect("frame 0");
        let at_half = renderer.render_frame(&graph, 30).expect("frame 30");
        let at_full = renderer.render_frame(&graph, 60).expect("frame 60");

        assert_eq!(max_rgb(&at_zero), 0);
        assert!(max_rgb(&at_half) > 40);
        assert!(max_rgb(&at_half) < max_rgb(&at_full));
        assert!(max_rgb(&at_full) > 200);
    }

    #[test]
    fn scene_image_draws_exact_path_with_scale_and_opacity() {
        let image_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-image-{}.png",
            std::process::id()
        ));
        RgbaImage::from_pixel(8, 6, Rgba([255, 0, 0, 255]))
            .save(&image_path)
            .expect("write test image");

        let graph = parse_graph_script(&format!(
            r##"
<Graph scope="scene" fps={{60}} duration="1s" size={{[64,48]}}>
  <Solid color="#000000" />
  <Image src="{}"
         x="10"
         y="12"
         scale="2.0"
         opacity="0.5" />
  <Present from="scene" />
</Graph>
"##,
            image_path.to_string_lossy()
        ))
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let rendered = renderer.render_frame(&graph, 0).expect("frame 0");
        let inside = rendered.get_pixel(12, 14);
        let outside = rendered.get_pixel(2, 2);

        assert!(inside[0] > 100, "expected red image pixel, got {inside:?}");
        assert_eq!(inside[1], 0);
        assert_eq!(inside[2], 0);
        assert_eq!(outside[0], 0);

        let _ = std::fs::remove_file(image_path);
    }

    #[test]
    fn scene_svg_draws_exact_path_with_scale_and_opacity() {
        let svg_path = write_test_svg(
            "svg",
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6" viewBox="0 0 8 6">
  <rect width="8" height="6" fill="#ff0000"/>
</svg>"##,
        );

        let graph = parse_graph_script(&format!(
            r##"
<Graph scope="scene" fps={{60}} duration="1s" size={{[64,48]}}>
  <Solid color="#000000" />
  <Svg src="{}"
       x="10"
       y="12"
       scale="2.0"
       opacity="0.5" />
  <Present from="scene" />
</Graph>
"##,
            svg_path.to_string_lossy()
        ))
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let rendered = renderer.render_frame(&graph, 0).expect("frame 0");
        let inside = rendered.get_pixel(12, 14);
        let outside = rendered.get_pixel(2, 2);

        assert!(inside[0] > 100, "expected red SVG pixel, got {inside:?}");
        assert_eq!(inside[1], 0);
        assert_eq!(inside[2], 0);
        assert_eq!(outside[0], 0);

        let _ = std::fs::remove_file(svg_path);
    }

    #[test]
    fn scene_svg_data_uri_utf8_renders() {
        let graph = parse_graph_script(
            r##"
<Graph scope="scene" fps={60} duration="1s" size={[64,48]}>
  <Solid color="#000000" />
  <Svg src="data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='8' height='6' viewBox='0 0 8 6'><rect width='8' height='6' fill='%23ff0000'/></svg>"
       x="10"
       y="12"
       scale="2.0"
       opacity="1.0" />
  <Present from="scene" />
</Graph>
"##,
        )
        .expect("scene graph parse");
        let mut renderer = SceneFrameRenderer::new();
        let rendered = renderer.render_frame(&graph, 0).expect("frame 0");
        let inside = rendered.get_pixel(12, 14);
        assert!(
            inside[0] > 200,
            "expected red SVG data URI pixel, got {inside:?}"
        );
    }

    #[test]
    fn scene_gpu_renderer_draws_image_when_available() {
        let image_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-gpu-image-{}.png",
            std::process::id()
        ));
        RgbaImage::from_pixel(8, 6, Rgba([0, 255, 0, 255]))
            .save(&image_path)
            .expect("write test image");

        let graph = parse_graph_script(&format!(
            r##"
<Graph scope="scene" fps={{60}} duration="1s" size={{[64,48]}}>
  <Solid color="#000000" />
  <Image src="{}"
         x="10"
         y="12"
         scale="2.0"
         opacity="1.0" />
  <Present from="scene" />
</Graph>
"##,
            image_path.to_string_lossy()
        ))
        .expect("scene graph parse");

        let mut renderer = SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu);
        let rendered = match renderer.render_frame(&graph, 0) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU render test: {message}");
                let _ = std::fs::remove_file(image_path);
                return;
            }
            Err(err) => panic!("unexpected GPU render error: {err}"),
        };
        let inside = rendered.get_pixel(12, 14);
        assert!(
            inside[1] > 150,
            "expected green GPU image pixel, got {inside:?}"
        );

        let _ = std::fs::remove_file(image_path);
    }

    #[test]
    fn scene_gpu_renderer_draws_svg_when_available() {
        let svg_path = write_test_svg(
            "gpu-svg",
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6" viewBox="0 0 8 6">
  <rect width="8" height="6" fill="#00ff00"/>
</svg>"##,
        );

        let graph = parse_graph_script(&format!(
            r##"
<Graph scope="scene" fps={{60}} duration="1s" size={{[64,48]}}>
  <Solid color="#000000" />
  <Svg src="{}"
       x="10"
       y="12"
       scale="2.0"
       opacity="1.0" />
  <Present from="scene" />
</Graph>
"##,
            svg_path.to_string_lossy()
        ))
        .expect("scene graph parse");

        let mut renderer = SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu);
        let rendered = match renderer.render_frame(&graph, 0) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU SVG render test: {message}");
                let _ = std::fs::remove_file(svg_path);
                return;
            }
            Err(err) => panic!("unexpected GPU render error: {err}"),
        };
        let inside = rendered.get_pixel(12, 14);
        assert!(
            inside[1] > 150,
            "expected green GPU SVG pixel, got {inside:?}"
        );

        let _ = std::fs::remove_file(svg_path);
    }

    #[test]
    fn scene_gpu_prores_renderer_draws_image_when_available() {
        let image_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-gpu-prores-image-{}.png",
            std::process::id()
        ));
        RgbaImage::from_pixel(8, 6, Rgba([0, 0, 255, 255]))
            .save(&image_path)
            .expect("write test image");

        let graph = parse_graph_script(&format!(
            r##"
<Graph scope="scene" fps={{60}} duration="1s" size={{[64,48]}}>
  <Solid color="#000000" />
  <Image src="{}"
         x="10"
         y="12"
         scale="2.0"
         opacity="1.0" />
  <Present from="scene" />
</Graph>
"##,
            image_path.to_string_lossy()
        ))
        .expect("scene graph parse");

        let mut renderer = SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu);
        let rendered = match renderer.render_frame(&graph, 0) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU render test: {message}");
                let _ = std::fs::remove_file(image_path);
                return;
            }
            Err(err) => panic!("unexpected GPU render error: {err}"),
        };
        let inside = rendered.get_pixel(12, 14);
        assert!(
            inside[2] > 150,
            "expected blue GPU image pixel, got {inside:?}"
        );

        let _ = std::fs::remove_file(image_path);
    }

    #[test]
    fn scene_gpu_renderer_composites_multiple_images_when_available() {
        let image1_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-gpu-multi-1-{}.png",
            std::process::id()
        ));
        let image2_path = std::env::temp_dir().join(format!(
            "anica-motionloom-scene-gpu-multi-2-{}.png",
            std::process::id()
        ));
        RgbaImage::from_pixel(8, 8, Rgba([255, 0, 0, 255]))
            .save(&image1_path)
            .expect("write test image 1");
        RgbaImage::from_pixel(8, 8, Rgba([0, 255, 0, 255]))
            .save(&image2_path)
            .expect("write test image 2");

        let graph = parse_graph_script(&format!(
            r##"
<Graph scope="scene" fps={{60}} duration="1s" size={{[64,48]}}>
  <Solid color="#000000" />
  <Image src="{}" x="4" y="10" scale="1.0" opacity="1.0" />
  <Image src="{}" x="24" y="10" scale="1.0" opacity="1.0" />
  <Present from="scene" />
</Graph>
"##,
            image1_path.to_string_lossy(),
            image2_path.to_string_lossy()
        ))
        .expect("scene graph parse");

        let mut renderer = SceneFrameRenderer::new_for_profile(SceneRenderProfile::Gpu);
        let rendered = match renderer.render_frame(&graph, 0) {
            Ok(rendered) => rendered,
            Err(MotionLoomSceneRenderError::GpuRender { message }) => {
                eprintln!("Skipping GPU render test: {message}");
                let _ = std::fs::remove_file(image1_path);
                let _ = std::fs::remove_file(image2_path);
                return;
            }
            Err(err) => panic!("unexpected GPU render error: {err}"),
        };
        let first = rendered.get_pixel(5, 11);
        let second = rendered.get_pixel(25, 11);

        assert!(
            first[0] > 200 && first[1] < 30,
            "expected first GPU image to remain visible, got {first:?}"
        );
        assert!(
            second[1] > 200 && second[0] < 30,
            "expected second GPU image to remain visible, got {second:?}"
        );

        let _ = std::fs::remove_file(image1_path);
        let _ = std::fs::remove_file(image2_path);
    }
}
