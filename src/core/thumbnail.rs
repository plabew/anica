// =========================================
// =========================================
// src/core/thumbnail.rs
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use log::{error, info};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ThumbnailError {
    #[error("Thumbnail output path has no parent ({path})")]
    MissingOutputParent { path: PathBuf },
    #[error("Failed to create thumbnail directory ({path}): {source}")]
    CreateThumbnailDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to execute ffmpeg: {source}")]
    ExecuteFfmpeg { source: std::io::Error },
    #[error("Thumbnail ffmpeg failed.\nSTDERR:\n{stderr}")]
    FfmpegFailed { stderr: String },
    #[error("Thumbnail output missing ({path}): {source}")]
    OutputMissing {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Thumbnail output is empty ({path})")]
    OutputEmpty { path: PathBuf },
    #[error("Failed to generate thumbnail")]
    FailedToGenerate,
}

// Bump this when thumbnail generation logic changes so old cached files are ignored.
const THUMBNAIL_ALGO_VERSION: u32 = 2;

// Keep media-pool thumbnail files in a dedicated folder under a cache root.
pub fn thumbnail_dir_for(cache_root: &Path) -> PathBuf {
    cache_root.join(".thumb")
}

// Hash source path + file metadata so the thumbnail key changes when media changes.
pub fn thumbnail_key(src: &Path, max_dim: u32) -> String {
    let mut hasher = DefaultHasher::new();
    src.to_string_lossy().hash(&mut hasher);
    if let Ok(meta) = fs::metadata(src) {
        meta.len().hash(&mut hasher);
        if let Ok(modified) = meta.modified()
            && let Ok(delta) = modified.duration_since(UNIX_EPOCH)
        {
            delta.as_secs().hash(&mut hasher);
        }
    }
    THUMBNAIL_ALGO_VERSION.hash(&mut hasher);
    max_dim.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

// Build deterministic thumbnail output path for one source media file.
pub fn thumbnail_path_for_in(cache_root: &Path, src: &Path, max_dim: u32) -> PathBuf {
    let key = thumbnail_key(src, max_dim);
    thumbnail_dir_for(cache_root).join(format!("thumb_{key}_{max_dim}.jpg"))
}

fn build_ffmpeg_args(
    src: &Path,
    dst: &Path,
    max_dim: u32,
    seek_seconds: Option<f32>,
) -> Vec<String> {
    // Fit the longest side to max_dim while preserving aspect ratio.
    let scale = format!(
        "scale='if(gte(iw,ih),{max_dim},-2)':'if(gte(iw,ih),-2,{max_dim})':flags=fast_bilinear"
    );
    let mut args = vec!["-y".into(), "-hide_banner".into()];
    if let Some(seek) = seek_seconds {
        // Start from a non-zero position to avoid common "black first frame" intros.
        args.push("-ss".into());
        args.push(format!("{seek:.3}"));
    }
    // The thumbnail filter picks a representative frame in a short window.
    let vf = format!("thumbnail=120,{scale}");
    args.extend([
        "-i".into(),
        src.to_string_lossy().to_string(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        vf,
        "-q:v".into(),
        "5".into(),
        dst.to_string_lossy().to_string(),
    ]);
    args
}

fn probe_duration_seconds(src: &Path) -> Option<f32> {
    // Probe source duration so seek attempts can cover the whole clip.
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            src.to_string_lossy().as_ref(),
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.trim().parse::<f32>().ok()
}

fn build_seek_attempts(src: &Path) -> Vec<Option<f32>> {
    let mut attempts: Vec<Option<f32>> = Vec::new();
    if let Some(duration) = probe_duration_seconds(src)
        && duration.is_finite()
        && duration > 0.5
    {
        // Sample multiple parts of the clip to reduce black-intro thumbnails.
        let fractions = [0.04_f32, 0.12, 0.22, 0.35, 0.50, 0.68, 0.84];
        for frac in fractions {
            let mut seek = duration * frac;
            let max_seek = (duration - 0.2).max(0.2);
            seek = seek.clamp(0.2, max_seek);
            if !attempts
                .iter()
                .flatten()
                .any(|existing| (existing - seek).abs() < 0.08)
            {
                attempts.push(Some(seek));
            }
        }
    } else {
        attempts.extend([Some(1.0_f32), Some(3.0_f32), Some(5.0_f32), Some(8.0_f32)]);
    }
    // Keep a final no-seek fallback so very short or unusual clips still get a thumbnail.
    attempts.push(None);
    attempts
}

fn run_ffmpeg(ffmpeg_bin: &str, args: &[String]) -> Result<(), ThumbnailError> {
    let out = Command::new(ffmpeg_bin)
        .args(args)
        .output()
        .map_err(|source| ThumbnailError::ExecuteFfmpeg { source })?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        error!("[Thumb] ffmpeg failed: {}", stderr);
        return Err(ThumbnailError::FfmpegFailed { stderr });
    }

    Ok(())
}

fn is_mostly_black(path: &Path) -> bool {
    let Ok(img) = image::open(path) else {
        return false;
    };
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w == 0 || h == 0 {
        return false;
    }

    let step_x = (w / 64).max(1) as usize;
    let step_y = (h / 64).max(1) as usize;
    let mut luminance_sum = 0.0f64;
    let mut samples = 0usize;
    for y in (0..h).step_by(step_y) {
        for x in (0..w).step_by(step_x) {
            let p = rgb.get_pixel(x, y);
            let r = p[0] as f64;
            let g = p[1] as f64;
            let b = p[2] as f64;
            let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            luminance_sum += lum;
            samples += 1;
        }
    }
    if samples == 0 {
        return false;
    }
    let avg_luminance = luminance_sum / samples as f64;
    avg_luminance < 18.0
}

// Run ffmpeg thumbnail extraction for media-pool preview cards.
pub fn run_thumbnail_job(
    ffmpeg_bin: &str,
    src: &Path,
    dst: &Path,
    max_dim: u32,
) -> Result<(), ThumbnailError> {
    let parent = dst
        .parent()
        .ok_or_else(|| ThumbnailError::MissingOutputParent {
            path: dst.to_path_buf(),
        })?;
    fs::create_dir_all(parent).map_err(|source| ThumbnailError::CreateThumbnailDirectory {
        path: parent.to_path_buf(),
        source,
    })?;

    if dst.exists()
        && let Ok(meta) = fs::metadata(dst)
        && meta.len() > 0
    {
        info!("[Thumb] reuse existing {}", dst.to_string_lossy());
        return Ok(());
    }

    // Try several seek points so clips with black intros are less likely to get a black thumbnail.
    let attempts = build_seek_attempts(src);
    let mut last_error: Option<ThumbnailError> = None;
    for (idx, seek) in attempts.iter().enumerate() {
        let args = build_ffmpeg_args(src, dst, max_dim, *seek);
        info!("[Thumb] ffmpeg {} {:?}", ffmpeg_bin, args);
        if let Err(err) = run_ffmpeg(ffmpeg_bin, &args) {
            last_error = Some(err);
            continue;
        }

        let meta = fs::metadata(dst).map_err(|source| ThumbnailError::OutputMissing {
            path: dst.to_path_buf(),
            source,
        })?;
        if meta.len() == 0 {
            last_error = Some(ThumbnailError::OutputEmpty {
                path: dst.to_path_buf(),
            });
            continue;
        }

        // Retry with another seek if this attempt still looks mostly black.
        let last_attempt = idx + 1 == attempts.len();
        if !last_attempt && is_mostly_black(dst) {
            info!("[Thumb] dark thumbnail detected, retrying later seek");
            continue;
        }

        info!("[Thumb] ready {}", dst.to_string_lossy());
        return Ok(());
    }

    Err(last_error.unwrap_or(ThumbnailError::FailedToGenerate))
}
