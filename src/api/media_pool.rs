use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::api::types::{MediaKind, MediaPoolAiMetadata, MediaPoolAiMetadataResponse};
use crate::core::global_state::{GlobalState, MediaPoolItem};

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MediaPoolItemRef {
    Index(usize),
    Id(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoveMediaPoolByIdRequest {
    pub id: MediaPoolItemRef,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoveMediaPoolByIdResponse {
    pub ok: bool,
    pub removed: bool,
    pub removed_id: Option<String>,
    pub removed_name: Option<String>,
    pub remaining_items: usize,
    pub physical_file_deleted: bool,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ClearMediaPoolRequest {}

#[derive(Debug, Clone, Serialize)]
pub struct ClearMediaPoolResponse {
    pub ok: bool,
    pub removed_count: usize,
    pub remaining_items: usize,
    pub physical_file_deleted: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMediaPoolMetadataRequest {
    pub include_missing_files: bool,
    #[serde(default = "default_include_file_stats")]
    pub include_file_stats: bool,
    #[serde(default = "default_include_media_probe")]
    pub include_media_probe: bool,
}

fn default_include_file_stats() -> bool {
    true
}

fn default_include_media_probe() -> bool {
    true
}

impl Default for ListMediaPoolMetadataRequest {
    fn default() -> Self {
        Self {
            include_missing_files: true,
            include_file_stats: true,
            include_media_probe: true,
        }
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn extension_lower(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_ascii_lowercase())
}

fn media_kind_from_ext(ext: Option<&str>) -> MediaKind {
    let Some(ext) = ext else {
        return MediaKind::Unknown;
    };

    match ext {
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "flv" | "m4v" => MediaKind::Video,
        "mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" | "opus" => MediaKind::Audio,
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" | "tif" | "tiff" => MediaKind::Image,
        _ => MediaKind::Unknown,
    }
}

#[derive(Debug, Deserialize)]
struct FfprobeStreamTags {
    #[serde(default)]
    rotate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStreamSideData {
    #[serde(default)]
    rotation: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    #[serde(default)]
    codec_type: Option<String>,
    #[serde(default)]
    codec_name: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    avg_frame_rate: Option<String>,
    #[serde(default)]
    r_frame_rate: Option<String>,
    #[serde(default)]
    channels: Option<u32>,
    #[serde(default)]
    sample_rate: Option<String>,
    #[serde(default)]
    tags: Option<FfprobeStreamTags>,
    #[serde(default)]
    side_data_list: Vec<FfprobeStreamSideData>,
}

#[derive(Debug, Deserialize)]
struct FfprobeResponse {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
}

#[derive(Debug, Default)]
struct MediaProbeInfo {
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<f32>,
    video_codec: Option<String>,
    audio_codec: Option<String>,
    rotation: Option<i32>,
    has_audio: Option<bool>,
    audio_channels: Option<u32>,
    sample_rate: Option<u32>,
}

fn parse_fps_ratio(raw: &str) -> Option<f32> {
    let txt = raw.trim();
    if txt.is_empty() || txt == "N/A" {
        return None;
    }
    if let Some((n, d)) = txt.split_once('/') {
        let n = n.trim().parse::<f32>().ok()?;
        let d = d.trim().parse::<f32>().ok()?;
        if d.abs() < f32::EPSILON {
            return None;
        }
        let fps = n / d;
        if fps.is_finite() && fps > 0.0 {
            return Some(fps);
        }
        return None;
    }
    let fps = txt.parse::<f32>().ok()?;
    if fps.is_finite() && fps > 0.0 {
        Some(fps)
    } else {
        None
    }
}

fn parse_u32_text(raw: &str) -> Option<u32> {
    raw.trim().parse::<u32>().ok()
}

fn normalize_rotation(raw: i32) -> i32 {
    let mut deg = raw.rem_euclid(360);
    if deg > 180 {
        deg -= 360;
    }
    deg
}

fn stream_rotation(stream: &FfprobeStream) -> Option<i32> {
    stream
        .side_data_list
        .iter()
        .find_map(|x| x.rotation)
        .or_else(|| {
            stream
                .tags
                .as_ref()
                .and_then(|tags| tags.rotate.as_deref())
                .and_then(|v| v.trim().parse::<i32>().ok())
        })
        .map(normalize_rotation)
}

fn probe_media_stream_details(path: &str, ffprobe_bin: &str) -> Option<MediaProbeInfo> {
    let output = Command::new(ffprobe_bin)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name,width,height,avg_frame_rate,r_frame_rate,channels,sample_rate,side_data_list:stream_tags=rotate",
            "-of",
            "json",
            path,
        ])
        .output();

    let Ok(out) = output else {
        return None;
    };
    if !out.status.success() {
        return None;
    }
    let Ok(parsed) = serde_json::from_slice::<FfprobeResponse>(&out.stdout) else {
        return None;
    };
    let mut info = MediaProbeInfo::default();

    if let Some(video_stream) = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
    {
        info.width = video_stream.width;
        info.height = video_stream.height;
        info.video_codec = video_stream.codec_name.clone();
        info.fps = video_stream
            .avg_frame_rate
            .as_deref()
            .and_then(parse_fps_ratio)
            .or_else(|| {
                video_stream
                    .r_frame_rate
                    .as_deref()
                    .and_then(parse_fps_ratio)
            });
        info.rotation = stream_rotation(video_stream);
    }

    if let Some(audio_stream) = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"))
    {
        info.has_audio = Some(true);
        info.audio_codec = audio_stream.codec_name.clone();
        info.audio_channels = audio_stream.channels;
        info.sample_rate = audio_stream.sample_rate.as_deref().and_then(parse_u32_text);
    } else {
        info.has_audio = Some(false);
    }

    Some(info)
}

/// Return all media-pool metadata in a format friendly for AI video-editing agents.
///
/// This is intentionally read-only and does not touch UI entities.
pub fn list_media_metadata_from_pool_items(
    media_pool: &[MediaPoolItem],
    request: ListMediaPoolMetadataRequest,
    ffprobe_bin: Option<&str>,
) -> MediaPoolAiMetadataResponse {
    let mut out = Vec::new();
    let ffprobe_bin = ffprobe_bin
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("ffprobe");

    for item in media_pool {
        let ext = extension_lower(&item.path);
        let kind = media_kind_from_ext(ext.as_deref());
        let is_proxy = item.path.contains("/.proxy/") || item.path.contains("\\.proxy\\");

        let (exists, file_size_bytes, created_unix_ms, modified_unix_ms) =
            if request.include_file_stats {
                let mut exists = false;
                let mut file_size_bytes = None;
                let mut created_unix_ms = None;
                let mut modified_unix_ms = None;

                if let Ok(meta) = fs::metadata(&item.path) {
                    exists = true;
                    file_size_bytes = Some(meta.len());
                    if let Ok(created) = meta.created()
                        && let Ok(since_epoch) = created.duration_since(UNIX_EPOCH)
                    {
                        created_unix_ms = Some(since_epoch.as_millis() as u64);
                    }
                    if let Ok(modified) = meta.modified()
                        && let Ok(since_epoch) = modified.duration_since(UNIX_EPOCH)
                    {
                        modified_unix_ms = Some(since_epoch.as_millis() as u64);
                    }
                }
                // On platforms/filesystems where file birth time is unavailable, fall back to modified.
                if created_unix_ms.is_none() {
                    created_unix_ms = modified_unix_ms;
                }
                (exists, file_size_bytes, created_unix_ms, modified_unix_ms)
            } else {
                (false, None, None, None)
            };

        if !request.include_missing_files && !exists {
            continue;
        }

        let probe_info = if request.include_media_probe && exists {
            probe_media_stream_details(&item.path, ffprobe_bin)
        } else {
            None
        };

        out.push(MediaPoolAiMetadata {
            id: item.path.clone(),
            path: item.path.clone(),
            name: item.name.clone(),
            extension: ext,
            media_kind: kind,
            duration_seconds: item.duration.as_secs_f64(),
            duration_millis: item.duration.as_millis() as u64,
            exists,
            file_size_bytes,
            created_unix_ms,
            modified_unix_ms,
            is_proxy_asset: is_proxy,
            width: probe_info.as_ref().and_then(|p| p.width),
            height: probe_info.as_ref().and_then(|p| p.height),
            fps: probe_info.as_ref().and_then(|p| p.fps),
            video_codec: probe_info.as_ref().and_then(|p| p.video_codec.clone()),
            audio_codec: probe_info.as_ref().and_then(|p| p.audio_codec.clone()),
            rotation: probe_info.as_ref().and_then(|p| p.rotation),
            has_audio: probe_info.as_ref().and_then(|p| p.has_audio),
            audio_channels: probe_info.as_ref().and_then(|p| p.audio_channels),
            sample_rate: probe_info.as_ref().and_then(|p| p.sample_rate),
            notes: None,
        });
    }

    MediaPoolAiMetadataResponse {
        total_items: out.len(),
        generated_at_unix_ms: now_unix_ms(),
        items: out,
    }
}

fn resolve_media_pool_path(
    media_pool: &[MediaPoolItem],
    target: &MediaPoolItemRef,
) -> Option<String> {
    match target {
        MediaPoolItemRef::Index(idx) => media_pool.get(*idx).map(|item| item.path.clone()),
        MediaPoolItemRef::Id(id) => {
            let trimmed = id.trim();
            if trimmed.is_empty() {
                return None;
            }
            media_pool
                .iter()
                .find(|item| item.path == trimmed)
                .map(|item| item.path.clone())
        }
    }
}

pub fn remove_media_pool_by_id(
    global: &mut GlobalState,
    request: RemoveMediaPoolByIdRequest,
) -> RemoveMediaPoolByIdResponse {
    let target = resolve_media_pool_path(&global.media_pool, &request.id);
    let Some(target_path) = target else {
        return RemoveMediaPoolByIdResponse {
            ok: true,
            removed: false,
            removed_id: None,
            removed_name: None,
            remaining_items: global.media_pool.len(),
            physical_file_deleted: false,
            message: "Media pool item not found by id.".to_string(),
        };
    };

    let removed_name = global
        .media_pool
        .iter()
        .find(|item| item.path == target_path)
        .map(|item| item.name.clone());
    let removed = global.remove_media_pool_item(&target_path);

    RemoveMediaPoolByIdResponse {
        ok: true,
        removed,
        removed_id: if removed {
            Some(target_path.clone())
        } else {
            None
        },
        removed_name: if removed { removed_name } else { None },
        remaining_items: global.media_pool.len(),
        physical_file_deleted: false,
        message: if removed {
            "Removed item from media pool only. Physical file is untouched.".to_string()
        } else {
            "Media pool item was not removed.".to_string()
        },
    }
}

pub fn clear_media_pool(
    global: &mut GlobalState,
    _request: ClearMediaPoolRequest,
) -> ClearMediaPoolResponse {
    let targets: Vec<String> = global
        .media_pool
        .iter()
        .map(|item| item.path.clone())
        .collect();
    if targets.is_empty() {
        return ClearMediaPoolResponse {
            ok: true,
            removed_count: 0,
            remaining_items: 0,
            physical_file_deleted: false,
            message: "Media pool is already empty.".to_string(),
        };
    }

    let mut removed_count = 0usize;
    for path in targets {
        if global.remove_media_pool_item(&path) {
            removed_count += 1;
        }
    }

    ClearMediaPoolResponse {
        ok: true,
        removed_count,
        remaining_items: global.media_pool.len(),
        physical_file_deleted: false,
        message: "Cleared media pool entries only. Physical files are untouched.".to_string(),
    }
}
