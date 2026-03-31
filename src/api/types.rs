use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaPoolAiMetadata {
    pub id: String,
    pub path: String,
    pub name: String,
    pub extension: Option<String>,
    pub media_kind: MediaKind,
    pub duration_seconds: f64,
    pub duration_millis: u64,
    pub exists: bool,
    pub file_size_bytes: Option<u64>,
    pub created_unix_ms: Option<u64>,
    pub modified_unix_ms: Option<u64>,
    pub is_proxy_asset: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub rotation: Option<i32>,
    pub has_audio: Option<bool>,
    pub audio_channels: Option<u32>,
    pub sample_rate: Option<u32>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaPoolAiMetadataResponse {
    pub total_items: usize,
    pub generated_at_unix_ms: u64,
    pub items: Vec<MediaPoolAiMetadata>,
}
