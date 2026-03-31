// =========================================
// =========================================
// src/core/waveform.rs
use crate::core::media_tools::ffprobe_from_ffmpeg;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use log::{error, info};
use thiserror::Error;

const WAVEFORM_CACHE_VERSION: u32 = 3;
const WAVEFORM_ANALYSIS_SAMPLE_RATE: u32 = 8_000;

#[derive(Debug, Error)]
pub enum WaveformError {
    #[error("Failed to read waveform cache ({path}): {source}")]
    ReadCache {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to parse waveform cache ({path}): {source}")]
    ParseCache {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("Waveform output path has no parent ({path})")]
    MissingOutputParent { path: PathBuf },
    #[error("Failed to create waveform directory ({path}): {source}")]
    CreateWaveformDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to encode waveform cache ({path}): {source}")]
    EncodeCache {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("Failed to write waveform cache ({path}): {source}")]
    WriteCache {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to execute ffmpeg for waveform: {source}")]
    ExecuteFfmpeg { source: std::io::Error },
    #[error("Waveform ffmpeg failed.\nSTDERR:\n{stderr}")]
    FfmpegFailed { stderr: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveformStatus {
    Missing,
    Pending,
    Ready,
    Failed,
}

#[derive(Debug, Clone)]
pub struct WaveformEntry {
    pub status: WaveformStatus,
    pub path: PathBuf,
    pub peaks: Option<Arc<Vec<f32>>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WaveformJob {
    pub key: String,
    pub src_path: PathBuf,
    pub dst_path: PathBuf,
    pub bucket_count: usize,
}

#[derive(Debug, Clone)]
pub struct WaveformLookup {
    pub peaks: Option<Arc<Vec<f32>>>,
    pub status: WaveformStatus,
}

pub fn waveform_key(src: &Path, bucket_count: usize) -> String {
    let mut hasher = DefaultHasher::new();
    WAVEFORM_CACHE_VERSION.hash(&mut hasher);
    src.to_string_lossy().hash(&mut hasher);
    if let Ok(meta) = fs::metadata(src) {
        meta.len().hash(&mut hasher);
        if let Ok(modified) = meta.modified()
            && let Ok(delta) = modified.duration_since(UNIX_EPOCH)
        {
            delta.as_secs().hash(&mut hasher);
        }
    }
    bucket_count.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

// Store waveform caches next to proxy caches under the selected cache root.
pub fn waveform_dir_for(cache_root: &Path) -> PathBuf {
    cache_root.join(".waveform")
}

pub fn waveform_path_for_in(cache_root: &Path, src: &Path, bucket_count: usize) -> PathBuf {
    let key = waveform_key(src, bucket_count);
    waveform_dir_for(cache_root).join(format!("waveform_{key}_{bucket_count}.json"))
}

pub fn load_waveform_file(path: &Path) -> Result<Vec<f32>, WaveformError> {
    let path_buf = path.to_path_buf();
    let bytes = fs::read(path).map_err(|source| WaveformError::ReadCache {
        path: path_buf.clone(),
        source,
    })?;
    serde_json::from_slice::<Vec<f32>>(&bytes).map_err(|source| WaveformError::ParseCache {
        path: path_buf,
        source,
    })
}

fn save_waveform_file(path: &Path, peaks: &[f32]) -> Result<(), WaveformError> {
    let parent = path
        .parent()
        .ok_or_else(|| WaveformError::MissingOutputParent {
            path: path.to_path_buf(),
        })?;
    fs::create_dir_all(parent).map_err(|source| WaveformError::CreateWaveformDirectory {
        path: parent.to_path_buf(),
        source,
    })?;
    let bytes = serde_json::to_vec(peaks).map_err(|source| WaveformError::EncodeCache {
        path: path.to_path_buf(),
        source,
    })?;
    fs::write(path, bytes).map_err(|source| WaveformError::WriteCache {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn probe_audio_channel_count(ffprobe_bin: &str, src: &Path) -> Option<usize> {
    // Preserve the loudest channel when building the envelope instead of
    // letting a mono downmix hide stereo transients.
    let out = Command::new(ffprobe_bin)
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=channels",
            "-of",
            "default=nokey=1:noprint_wrappers=1",
        ])
        .arg(src)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| line.trim().parse::<usize>().ok())
        .filter(|count| *count > 0)
}

fn extract_peaks_from_pcm_bytes(
    bytes: &[u8],
    bucket_count: usize,
    channel_count: usize,
) -> Vec<f32> {
    if bucket_count == 0 {
        return Vec::new();
    }

    let channel_count = channel_count.max(1);
    let frame_width_bytes = 4usize.saturating_mul(channel_count);
    if bytes.len() < frame_width_bytes {
        return vec![0.0; bucket_count];
    }

    // Map each decoded audio frame straight into its final time bucket so the
    // cached waveform keeps short transients and absolute clip loudness.
    let total_frames = bytes.len() / frame_width_bytes;
    let mut out = vec![0.0_f32; bucket_count];
    let mut byte_idx = 0usize;

    for frame_idx in 0..total_frames {
        let mut frame_peak = 0.0_f32;
        for _ in 0..channel_count {
            let sample = f32::from_le_bytes([
                bytes[byte_idx],
                bytes[byte_idx + 1],
                bytes[byte_idx + 2],
                bytes[byte_idx + 3],
            ]);
            frame_peak = frame_peak.max(sample.abs().clamp(0.0, 1.0));
            byte_idx += 4;
        }

        let bucket = ((frame_idx * bucket_count) / total_frames).min(bucket_count - 1);
        if frame_peak > out[bucket] {
            out[bucket] = frame_peak;
        }
    }

    out
}

pub fn run_waveform_job(ffmpeg_bin: &str, job: &WaveformJob) -> Result<Vec<f32>, WaveformError> {
    if job.dst_path.exists() {
        if let Ok(peaks) = load_waveform_file(&job.dst_path) {
            info!(
                "[Waveform] reuse existing {}",
                job.dst_path.to_string_lossy()
            );
            return Ok(peaks);
        }
        let _ = fs::remove_file(&job.dst_path);
    }

    // Keep original channel peaks when ffprobe can describe the audio stream.
    let ffprobe_bin = ffprobe_from_ffmpeg(ffmpeg_bin);
    let channel_count = probe_audio_channel_count(&ffprobe_bin, &job.src_path).unwrap_or(1);

    let mut args = vec![
        "-hide_banner".to_string(),
        "-v".to_string(),
        "error".to_string(),
        "-i".to_string(),
        job.src_path.to_string_lossy().to_string(),
        "-vn".to_string(),
    ];
    if channel_count == 1 {
        args.extend(["-ac".to_string(), "1".to_string()]);
    }
    args.extend([
        "-ar".to_string(),
        WAVEFORM_ANALYSIS_SAMPLE_RATE.to_string(),
        "-f".to_string(),
        "f32le".to_string(),
        "-".to_string(),
    ]);

    info!("[Waveform] ffmpeg {} {:?}", ffmpeg_bin, args);
    let out = Command::new(ffmpeg_bin)
        .args(&args)
        .output()
        .map_err(|source| WaveformError::ExecuteFfmpeg { source })?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        error!("[Waveform] ffmpeg failed: {}", stderr);
        return Err(WaveformError::FfmpegFailed { stderr });
    }

    let peaks = extract_peaks_from_pcm_bytes(&out.stdout, job.bucket_count, channel_count);
    save_waveform_file(&job.dst_path, &peaks)?;

    info!("[Waveform] ready {}", job.dst_path.to_string_lossy());
    Ok(peaks)
}
