use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::scene::render::MotionLoomSceneRenderError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneRenderProfile {
    Cpu,
    Gpu,
    GpuProRes,
    GpuProRes4444,
    GpuPngSequence,
}

impl SceneRenderProfile {
    pub const fn output_extension(self) -> &'static str {
        match self {
            SceneRenderProfile::Cpu => "mov",
            SceneRenderProfile::Gpu => "mp4",
            SceneRenderProfile::GpuProRes => "mov",
            SceneRenderProfile::GpuProRes4444 => "mov",
            SceneRenderProfile::GpuPngSequence => "png",
        }
    }

    pub const fn output_prefix(self) -> &'static str {
        match self {
            SceneRenderProfile::Cpu => "motionloom_scene",
            SceneRenderProfile::Gpu => "motionloom_scene_gpu",
            SceneRenderProfile::GpuProRes => "motionloom_scene_gpu_prores",
            SceneRenderProfile::GpuProRes4444 => "motionloom_scene_gpu_prores4444",
            SceneRenderProfile::GpuPngSequence => "motionloom_scene_gpu_png",
        }
    }

    pub const fn uses_gpu_compositor(self) -> bool {
        matches!(
            self,
            SceneRenderProfile::Gpu
                | SceneRenderProfile::GpuProRes
                | SceneRenderProfile::GpuProRes4444
                | SceneRenderProfile::GpuPngSequence
        )
    }

    pub const fn is_png_sequence(self) -> bool {
        matches!(self, SceneRenderProfile::GpuPngSequence)
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
    if profile.is_png_sequence() {
        return Ok(output_dir.join(format!("{}_{}", profile.output_prefix(), stamp)));
    }

    Ok(output_dir.join(format!(
        "{}_{}.{}",
        profile.output_prefix(),
        stamp,
        profile.output_extension()
    )))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn scene_encoder_args(profile: SceneRenderProfile) -> Vec<String> {
    match profile {
        SceneRenderProfile::Cpu => prores_encoder_args(),
        SceneRenderProfile::Gpu => gpu_h264_encoder_args(),
        SceneRenderProfile::GpuProRes => prores_encoder_args(),
        SceneRenderProfile::GpuProRes4444 => prores_4444_encoder_args(),
        SceneRenderProfile::GpuPngSequence => Vec::new(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn prores_encoder_args() -> Vec<String> {
    // Keep scene output on an LGPL-safe FFmpeg-friendly path.
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

#[cfg(not(target_arch = "wasm32"))]
fn prores_4444_encoder_args() -> Vec<String> {
    vec![
        "-vf".to_string(),
        "format=yuva444p10le".to_string(),
        "-c:v".to_string(),
        "prores_ks".to_string(),
        "-profile:v".to_string(),
        "4".to_string(),
        "-vendor".to_string(),
        "apl0".to_string(),
        "-alpha_bits".to_string(),
        "16".to_string(),
        "-vtag".to_string(),
        "ap4h".to_string(),
        "-pix_fmt".to_string(),
        "yuva444p10le".to_string(),
        "-color_primaries".to_string(),
        "bt709".to_string(),
        "-color_trc".to_string(),
        "bt709".to_string(),
        "-colorspace".to_string(),
        "bt709".to_string(),
    ]
}

#[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
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

#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
fn gpu_h264_encoder_args() -> Vec<String> {
    vec![
        "-c:v".to_string(),
        "h264_mf".to_string(),
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

#[cfg(all(
    not(target_arch = "wasm32"),
    not(target_os = "macos"),
    not(target_os = "windows")
))]
fn gpu_h264_encoder_args() -> Vec<String> {
    vec![
        "-c:v".to_string(),
        "libopenh264".to_string(),
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
