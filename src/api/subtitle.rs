use serde::{Deserialize, Serialize};

pub const IMPORT_SRT_PLACEMENT_AUTO_NON_OVERLAP: &str = "auto_non_overlap";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpSubtitleImportSrtRequest {
    pub srt_text: String,
    #[serde(default)]
    pub track_index: Option<usize>,
    #[serde(default = "default_import_srt_placement")]
    pub placement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpSubtitleImportSrtResponse {
    pub ok: bool,
    pub imported_cues: usize,
    pub placement_used: String,
    pub track_index: Option<usize>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

pub fn default_import_srt_placement() -> String {
    IMPORT_SRT_PLACEMENT_AUTO_NON_OVERLAP.to_string()
}

pub fn normalize_import_srt_placement(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        default_import_srt_placement()
    } else {
        normalized
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpSubtitleAddTrackRequest {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpSubtitleAddTrackResponse {
    pub ok: bool,
    pub track_index: usize,
    pub name: String,
    pub error: Option<String>,
}

impl AcpSubtitleImportSrtResponse {
    pub fn success(imported_cues: usize, placement_used: String) -> Self {
        Self {
            ok: true,
            imported_cues,
            placement_used,
            track_index: None,
            warnings: Vec::new(),
            error: None,
        }
    }

    pub fn failure(error: impl Into<String>, placement_used: String) -> Self {
        Self {
            ok: false,
            imported_cues: 0,
            placement_used,
            track_index: None,
            warnings: Vec::new(),
            error: Some(error.into()),
        }
    }
}
