// =========================================
// =========================================
// src/core/proxy.rs
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use log::{error, info};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("Proxy output path has no parent ({path})")]
    MissingOutputParent { path: PathBuf },
    #[error("Failed to create proxy directory ({path}): {source}")]
    CreateProxyDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Proxy output missing ({path}): {source}")]
    OutputMissing {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Proxy output is empty ({path})")]
    OutputEmpty { path: PathBuf },
    #[error("Failed to execute ffmpeg: {source}")]
    ExecuteFfmpeg { source: std::io::Error },
    #[error("Proxy ffmpeg failed.\nSTDERR:\n{stderr}")]
    FfmpegFailed { stderr: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyStatus {
    Missing,
    Pending,
    Ready,
    Failed,
}

#[derive(Debug, Clone)]
pub struct ProxyEntry {
    pub status: ProxyStatus,
    pub path: PathBuf,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProxyJob {
    pub key: String,
    pub src_path: PathBuf,
    pub dst_path: PathBuf,
    pub max_dim: u32,
}

#[derive(Debug, Clone)]
pub struct ProxyLookup {
    pub path: Option<String>,
    pub status: ProxyStatus,
}

pub fn proxy_key(src: &Path, max_dim: u32) -> String {
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
    max_dim.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

// Resolve proxy directory under a chosen cache root.
pub fn proxy_dir_for(cache_root: &Path) -> PathBuf {
    cache_root.join(".proxy")
}

// Build proxy output path under a chosen cache root.
pub fn proxy_path_for_in(cache_root: &Path, src: &Path, max_dim: u32) -> PathBuf {
    let key = proxy_key(src, max_dim);
    proxy_dir_for(cache_root).join(format!("proxy_{key}_{max_dim}p.mp4"))
}

pub fn build_ffmpeg_args(src: &Path, dst: &Path, max_dim: u32) -> Vec<String> {
    let scale = format!("scale=w={max_dim}:h=-2:flags=fast_bilinear");
    let mut args = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-i".into(),
        src.to_string_lossy().to_string(),
        "-vf".into(),
        scale,
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "0:a?".into(),
    ];

    if cfg!(target_os = "macos") {
        args.push("-c:v".into());
        args.push("h264_videotoolbox".into());
        args.push("-allow_sw".into());
        args.push("1".into());
        args.push("-b:v".into());
        args.push("2500k".into());
        args.push("-maxrate".into());
        args.push("3000k".into());
        args.push("-bufsize".into());
        args.push("6000k".into());
    } else {
        args.push("-c:v".into());
        args.push("libopenh264".into());
        args.push("-b:v".into());
        args.push("2500k".into());
        args.push("-maxrate".into());
        args.push("3000k".into());
        args.push("-bufsize".into());
        args.push("6000k".into());
    }

    args.push("-pix_fmt".into());
    args.push("yuv420p".into());
    args.push("-c:a".into());
    args.push("aac".into());
    args.push("-b:a".into());
    args.push("96k".into());
    args.push(dst.to_string_lossy().to_string());
    args
}

#[cfg(target_os = "macos")]
fn build_ffmpeg_args_macos_gpu(src: &Path, dst: &Path, max_dim: u32) -> Vec<String> {
    let scale = format!("scale=w={max_dim}:h=-2:flags=fast_bilinear");
    vec![
        "-y".into(),
        "-hide_banner".into(),
        "-i".into(),
        src.to_string_lossy().to_string(),
        "-vf".into(),
        scale,
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "0:a?".into(),
        "-c:v".into(),
        "h264_videotoolbox".into(),
        // Permit encoder-side software fallback inside videotoolbox when HW path is unavailable.
        "-allow_sw".into(),
        "1".into(),
        // Keep proxy bitrate modest and stable for timeline usage.
        "-b:v".into(),
        "2500k".into(),
        "-maxrate".into(),
        "3000k".into(),
        "-bufsize".into(),
        "6000k".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "96k".into(),
        dst.to_string_lossy().to_string(),
    ]
}

fn validate_proxy_output(dst: &Path) -> Result<(), ProxyError> {
    let meta = fs::metadata(dst).map_err(|source| ProxyError::OutputMissing {
        path: dst.to_path_buf(),
        source,
    })?;
    if meta.len() == 0 {
        return Err(ProxyError::OutputEmpty {
            path: dst.to_path_buf(),
        });
    }
    Ok(())
}

pub fn run_proxy_job(ffmpeg_bin: &str, job: &ProxyJob) -> Result<(), ProxyError> {
    let parent = job
        .dst_path
        .parent()
        .ok_or_else(|| ProxyError::MissingOutputParent {
            path: job.dst_path.clone(),
        })?;
    fs::create_dir_all(parent).map_err(|source| ProxyError::CreateProxyDirectory {
        path: parent.to_path_buf(),
        source,
    })?;

    if job.dst_path.exists() {
        if let Ok(meta) = fs::metadata(&job.dst_path)
            && meta.len() > 0
        {
            info!("[Proxy] reuse existing {}", job.dst_path.to_string_lossy());
            return Ok(());
        }
        let _ = fs::remove_file(&job.dst_path);
    }

    #[cfg(target_os = "macos")]
    {
        let gpu_args = build_ffmpeg_args_macos_gpu(&job.src_path, &job.dst_path, job.max_dim);
        info!("[Proxy] ffmpeg (macOS GPU) {} {:?}", ffmpeg_bin, gpu_args);
        match Command::new(ffmpeg_bin).args(&gpu_args).output() {
            Ok(out) if out.status.success() => {
                if validate_proxy_output(&job.dst_path).is_ok() {
                    info!(
                        "[Proxy] ready {} (macOS GPU)",
                        job.dst_path.to_string_lossy()
                    );
                    return Ok(());
                }
                let _ = fs::remove_file(&job.dst_path);
                log::warn!("[Proxy] macOS GPU encode produced invalid output, falling back to CPU");
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                log::warn!(
                    "[Proxy] macOS GPU encode failed, falling back to CPU. stderr={}",
                    stderr.trim()
                );
                let _ = fs::remove_file(&job.dst_path);
            }
            Err(err) => {
                log::warn!(
                    "[Proxy] macOS GPU encode launch failed, falling back to CPU: {}",
                    err
                );
                let _ = fs::remove_file(&job.dst_path);
            }
        }
    }

    let args = build_ffmpeg_args(&job.src_path, &job.dst_path, job.max_dim);
    info!("[Proxy] ffmpeg {} {:?}", ffmpeg_bin, args);
    let out = Command::new(ffmpeg_bin)
        .args(&args)
        .output()
        .map_err(|source| ProxyError::ExecuteFfmpeg { source })?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        error!("[Proxy] ffmpeg failed: {}", stderr);
        return Err(ProxyError::FfmpegFailed { stderr });
    }

    validate_proxy_output(&job.dst_path)?;
    info!("[Proxy] ready {}", job.dst_path.to_string_lossy());
    Ok(())
}
