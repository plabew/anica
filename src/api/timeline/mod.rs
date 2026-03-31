use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::info;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::core::global_state::{
    AudioTrack, Clip, GlobalState, SemanticClip, SubtitleClip, SubtitleTrack, TransitionType,
    VideoTrack,
};

mod errors;
mod validation;

use self::errors::TimelineEditError;
pub use self::validation::validate_edit_plan;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TimelineCanvas {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineClipView {
    pub clip_id: u64,
    pub label: String,
    pub file_path: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub source_in_ms: u64,
    pub media_duration_ms: u64,
    pub link_group_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineTrackView {
    pub index: usize,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub muted: bool,
    pub clips: Vec<TimelineClipView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineSubtitleClipView {
    pub clip_id: u64,
    pub text: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub pos_x: f32,
    pub pos_y: f32,
    pub font_size: f32,
    pub color_rgba: (u8, u8, u8, u8),
    pub group_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineSubtitleTrackView {
    pub index: usize,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub muted: bool,
    pub clips: Vec<TimelineSubtitleClipView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineLinkGroupView {
    pub group_id: u64,
    pub clip_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineSemanticClipView {
    pub clip_id: u64,
    pub semantic_type: String,
    pub label: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub prompt_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineSnapshotRequest {
    #[serde(default = "default_true")]
    pub include_subtitles: bool,
}

impl Default for TimelineSnapshotRequest {
    fn default() -> Self {
        Self {
            include_subtitles: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineSnapshotResponse {
    pub timeline_revision: String,
    pub generated_at_unix_ms: u64,
    pub fps: f32,
    pub duration_ms: u64,
    pub canvas: TimelineCanvas,
    pub v1: TimelineTrackView,
    pub audio_tracks: Vec<TimelineTrackView>,
    pub video_tracks: Vec<TimelineTrackView>,
    pub subtitle_tracks: Vec<TimelineSubtitleTrackView>,
    pub semantic_clips: Vec<TimelineSemanticClipView>,
    pub link_groups: Vec<TimelineLinkGroupView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSilenceMapRequest {
    #[serde(default = "default_rms_threshold_db")]
    pub rms_threshold_db: f32,
    #[serde(default = "default_min_silence_ms")]
    pub min_silence_ms: u64,
    #[serde(default = "default_pad_ms")]
    pub pad_ms: u64,
    #[serde(default = "default_detect_low_energy_repeats")]
    pub detect_low_energy_repeats: bool,
    #[serde(default = "default_repeat_similarity_threshold")]
    pub repeat_similarity_threshold: f32,
    #[serde(default = "default_repeat_window_ms")]
    pub repeat_window_ms: u64,
}

impl Default for AudioSilenceMapRequest {
    fn default() -> Self {
        Self {
            rms_threshold_db: default_rms_threshold_db(),
            min_silence_ms: default_min_silence_ms(),
            pad_ms: default_pad_ms(),
            detect_low_energy_repeats: default_detect_low_energy_repeats(),
            repeat_similarity_threshold: default_repeat_similarity_threshold(),
            repeat_window_ms: default_repeat_window_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineSegment {
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SilenceCutCandidate {
    pub start_ms: u64,
    pub end_ms: u64,
    pub confidence: f32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSilenceDebugRow {
    pub start_ms: u64,
    pub end_ms: u64,
    pub duration_ms: u64,
    pub mean_rms: f32,
    pub threshold_rms: f32,
    pub threshold_db: f32,
    pub confidence: f32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSilenceMapResponse {
    pub timeline_revision: String,
    pub generated_at_unix_ms: u64,
    pub analysis_source: String,
    pub speech_segments: Vec<TimelineSegment>,
    pub silence_segments: Vec<TimelineSegment>,
    pub cut_candidates: Vec<SilenceCutCandidate>,
    pub debug_rows: Vec<AudioSilenceDebugRow>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AudioSilenceCutPlanRequest {
    #[serde(default)]
    pub rms_threshold_db: Option<f32>,
    #[serde(default)]
    pub min_silence_ms: Option<u64>,
    #[serde(default)]
    pub pad_ms: Option<u64>,
    #[serde(default)]
    pub detect_low_energy_repeats: Option<bool>,
    #[serde(default)]
    pub repeat_similarity_threshold: Option<f32>,
    #[serde(default)]
    pub repeat_window_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSilenceCutPlanResponse {
    pub timeline_revision: String,
    pub generated_at_unix_ms: u64,
    pub analysis_source: String,
    pub candidate_ranges: Vec<TimelineSegment>,
    pub debug_rows: Vec<AudioSilenceDebugRow>,
    pub operations: Vec<TimelineEditOperation>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleGapMapRequest {
    #[serde(default = "default_subtitle_gap_mode")]
    pub mode: String,
    #[serde(default)]
    pub min_gap_ms: Option<u64>,
    #[serde(default)]
    pub edge_pad_ms: Option<u64>,
    #[serde(default)]
    pub include_head_tail: bool,
    #[serde(default)]
    pub track_indices: Option<Vec<usize>>,
}

impl Default for SubtitleGapMapRequest {
    fn default() -> Self {
        Self {
            mode: default_subtitle_gap_mode(),
            min_gap_ms: None,
            edge_pad_ms: None,
            include_head_tail: true,
            track_indices: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleGapModeOption {
    pub key: String,
    pub min_gap_ms: u64,
    pub edge_pad_ms: u64,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleGapMapResponse {
    pub timeline_revision: String,
    pub generated_at_unix_ms: u64,
    pub analysis_source: String,
    pub mode_used: String,
    pub min_gap_ms: u64,
    pub edge_pad_ms: u64,
    pub available_modes: Vec<SubtitleGapModeOption>,
    pub analysis_window: Option<TimelineSegment>,
    pub subtitle_segments: Vec<TimelineSegment>,
    pub gap_segments: Vec<TimelineSegment>,
    pub cut_candidates: Vec<SilenceCutCandidate>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleGapCutPlanRequest {
    #[serde(default = "default_subtitle_gap_mode")]
    pub mode: String,
    #[serde(default)]
    pub min_gap_ms: Option<u64>,
    #[serde(default)]
    pub edge_pad_ms: Option<u64>,
    #[serde(default)]
    pub include_head_tail: bool,
    #[serde(default)]
    pub track_indices: Option<Vec<usize>>,
    #[serde(default = "default_subtitle_gap_cut_strategy")]
    pub cut_strategy: String,
    #[serde(default = "default_audio_align_min_overlap_ms")]
    pub audio_align_min_overlap_ms: u64,
    #[serde(default)]
    pub audio_rms_threshold_db: Option<f32>,
    #[serde(default)]
    pub audio_min_silence_ms: Option<u64>,
    #[serde(default)]
    pub audio_pad_ms: Option<u64>,
}

impl Default for SubtitleGapCutPlanRequest {
    fn default() -> Self {
        Self {
            mode: default_subtitle_gap_mode(),
            min_gap_ms: None,
            edge_pad_ms: None,
            include_head_tail: true,
            track_indices: None,
            cut_strategy: default_subtitle_gap_cut_strategy(),
            audio_align_min_overlap_ms: default_audio_align_min_overlap_ms(),
            audio_rms_threshold_db: None,
            audio_min_silence_ms: None,
            audio_pad_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleGapCutPlanResponse {
    pub timeline_revision: String,
    pub generated_at_unix_ms: u64,
    pub analysis_source: String,
    pub cut_strategy_used: String,
    pub mode_used: String,
    pub candidate_ranges: Vec<TimelineSegment>,
    pub operations: Vec<TimelineEditOperation>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutonomousEditPlanRequest {
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub aggressiveness: Option<String>,
    #[serde(default)]
    pub prefer_language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousEditPlanObservation {
    pub category: String,
    pub tool: String,
    pub arguments: Value,
    pub candidate_count: usize,
    pub estimated_removed_ms: u64,
    pub estimated_removed_ratio: f32,
    pub operations_preview: Vec<TimelineEditOperation>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousEditPlanSummary {
    pub timeline_duration_ms: u64,
    pub v1_clip_count: usize,
    pub audio_clip_count: usize,
    pub video_clip_count: usize,
    pub subtitle_clip_count: usize,
    pub semantic_clip_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousEditPlanResponse {
    pub timeline_revision: String,
    pub generated_at_unix_ms: u64,
    pub analysis_source: String,
    pub goal: String,
    pub aggressiveness_used: String,
    pub decision_owner: String,
    pub summary: AutonomousEditPlanSummary,
    pub target_removed_ratio: f32,
    pub candidate_counts: HashMap<String, usize>,
    pub observations: Vec<AutonomousEditPlanObservation>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleSemanticRepeatsRequest {
    #[serde(default = "default_semantic_window_ms")]
    pub window_ms: u64,
    #[serde(default = "default_semantic_similarity_threshold")]
    pub similarity_threshold: f32,
    #[serde(default)]
    pub track_indices: Option<Vec<usize>>,
}

impl Default for SubtitleSemanticRepeatsRequest {
    fn default() -> Self {
        Self {
            window_ms: default_semantic_window_ms(),
            similarity_threshold: default_semantic_similarity_threshold(),
            track_indices: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleSemanticRepeatPair {
    pub left_clip_id: u64,
    pub right_clip_id: u64,
    pub similarity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleSemanticRepeatMember {
    pub clip_id: u64,
    pub track_index: usize,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleSemanticRepeatGroup {
    pub group_id: usize,
    pub keep_clip_id: u64,
    pub confidence: f32,
    pub members: Vec<SubtitleSemanticRepeatMember>,
    pub matched_pairs: Vec<SubtitleSemanticRepeatPair>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleSemanticRepeatsResponse {
    pub timeline_revision: String,
    pub generated_at_unix_ms: u64,
    pub analysis_source: String,
    pub window_ms: u64,
    pub similarity_threshold: f32,
    pub repeat_groups: Vec<SubtitleSemanticRepeatGroup>,
    pub cut_candidates: Vec<SilenceCutCandidate>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptLowConfidenceMapRequest {
    #[serde(default)]
    pub transcript_confidence_json: Option<Value>,
    #[serde(default = "default_transcript_uncertainty_threshold")]
    pub uncertainty_threshold: f32,
    #[serde(default = "default_transcript_min_duration_ms")]
    pub min_duration_ms: u64,
    #[serde(default = "default_transcript_edge_pad_ms")]
    pub edge_pad_ms: u64,
    #[serde(default = "default_true")]
    pub enable_semantic_fallback: bool,
    #[serde(default = "default_semantic_window_ms")]
    pub fallback_window_ms: u64,
    #[serde(default = "default_semantic_similarity_threshold")]
    pub fallback_similarity_threshold: f32,
    #[serde(default)]
    pub track_indices: Option<Vec<usize>>,
}

impl Default for TranscriptLowConfidenceMapRequest {
    fn default() -> Self {
        Self {
            transcript_confidence_json: None,
            uncertainty_threshold: default_transcript_uncertainty_threshold(),
            min_duration_ms: default_transcript_min_duration_ms(),
            edge_pad_ms: default_transcript_edge_pad_ms(),
            enable_semantic_fallback: true,
            fallback_window_ms: default_semantic_window_ms(),
            fallback_similarity_threshold: default_semantic_similarity_threshold(),
            track_indices: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptLowConfidenceDebugRow {
    pub start_ms: u64,
    pub end_ms: u64,
    pub duration_ms: u64,
    pub uncertainty: f32,
    pub uncertainty_threshold: f32,
    pub avg_logprob: Option<f32>,
    pub no_speech_prob: Option<f32>,
    pub silence_probability: Option<f32>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptLowConfidenceMapResponse {
    pub timeline_revision: String,
    pub generated_at_unix_ms: u64,
    pub analysis_source: String,
    pub requires_transcript_confidence_json: bool,
    pub transcript_json_status: String,
    pub cut_candidates: Vec<SilenceCutCandidate>,
    pub debug_rows: Vec<TranscriptLowConfidenceDebugRow>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptLowConfidenceCutPlanRequest {
    #[serde(default)]
    pub transcript_confidence_json: Option<Value>,
    #[serde(default)]
    pub uncertainty_threshold: Option<f32>,
    #[serde(default)]
    pub min_duration_ms: Option<u64>,
    #[serde(default)]
    pub edge_pad_ms: Option<u64>,
    #[serde(default = "default_true")]
    pub enable_semantic_fallback: bool,
    #[serde(default)]
    pub fallback_window_ms: Option<u64>,
    #[serde(default)]
    pub fallback_similarity_threshold: Option<f32>,
    #[serde(default = "default_true")]
    pub enable_subtitle_long_silence_cut: bool,
    #[serde(default)]
    pub subtitle_long_silence_min_ms: Option<u64>,
    #[serde(default)]
    pub subtitle_long_silence_rms_threshold_db: Option<f32>,
    #[serde(default)]
    pub subtitle_long_silence_pad_ms: Option<u64>,
    #[serde(default)]
    pub track_indices: Option<Vec<usize>>,
}

impl Default for TranscriptLowConfidenceCutPlanRequest {
    fn default() -> Self {
        Self {
            transcript_confidence_json: None,
            uncertainty_threshold: None,
            min_duration_ms: None,
            edge_pad_ms: None,
            enable_semantic_fallback: true,
            fallback_window_ms: None,
            fallback_similarity_threshold: None,
            enable_subtitle_long_silence_cut: true,
            subtitle_long_silence_min_ms: None,
            subtitle_long_silence_rms_threshold_db: None,
            subtitle_long_silence_pad_ms: None,
            track_indices: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptLowConfidenceCutPlanResponse {
    pub timeline_revision: String,
    pub generated_at_unix_ms: u64,
    pub analysis_source: String,
    pub fallback_used: bool,
    pub transcript_json_status: String,
    pub transcript_json_prompt: Option<String>,
    pub candidate_ranges: Vec<TimelineSegment>,
    pub operations: Vec<TimelineEditOperation>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleGenerateEntry {
    pub text: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    #[serde(default)]
    pub pos_x: Option<f32>,
    #[serde(default)]
    pub pos_y: Option<f32>,
    #[serde(default)]
    pub font_size: Option<f32>,
    #[serde(default)]
    pub color_rgba: Option<(u8, u8, u8, u8)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitlePatch {
    pub clip_id: u64,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub start_ms: Option<u64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub track_index: Option<usize>,
    #[serde(default)]
    pub pos_x: Option<f32>,
    #[serde(default)]
    pub pos_y: Option<f32>,
    #[serde(default)]
    pub font_size: Option<f32>,
    #[serde(default)]
    pub color_rgba: Option<(u8, u8, u8, u8)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TimelineEditOperation {
    AddTrack {
        track_type: String,
        #[serde(default)]
        name: Option<String>,
    },
    AddAudioTrack {
        #[serde(default)]
        name: Option<String>,
    },
    AddVideoTrack {
        #[serde(default)]
        name: Option<String>,
    },
    AddSubtitleTrack {
        #[serde(default)]
        name: Option<String>,
    },
    RemoveTrack {
        track_type: String,
        index: usize,
    },
    RemoveAudioTrack {
        index: usize,
    },
    RemoveVideoTrack {
        index: usize,
    },
    RemoveSubtitleTrack {
        index: usize,
    },
    SetTrackVisibility {
        track_type: String,
        index: usize,
        visible: bool,
    },
    SetTrackLock {
        track_type: String,
        index: usize,
        locked: bool,
    },
    SetTrackMute {
        track_type: String,
        index: usize,
        muted: bool,
    },
    InsertClip {
        track_type: String,
        #[serde(default)]
        track_index: Option<usize>,
        #[serde(default)]
        media_pool_item_id: Option<usize>,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        start_ms: Option<u64>,
        #[serde(default)]
        source_in_ms: Option<u64>,
        #[serde(default)]
        source_out_ms: Option<u64>,
        #[serde(default)]
        duration_ms: Option<u64>,
    },
    InsertFromMediaPool {
        track_type: String,
        #[serde(default)]
        track_index: Option<usize>,
        media_pool_item_id: usize,
        #[serde(default)]
        start_ms: Option<u64>,
        #[serde(default)]
        source_in_ms: Option<u64>,
        #[serde(default)]
        source_out_ms: Option<u64>,
        #[serde(default)]
        duration_ms: Option<u64>,
    },
    SetSourceInOut {
        clip_id: u64,
        source_in_ms: u64,
        #[serde(default)]
        source_out_ms: Option<u64>,
        #[serde(default)]
        duration_ms: Option<u64>,
    },
    #[serde(alias = "remove_clip")]
    DeleteClip {
        clip_id: u64,
        #[serde(default)]
        ripple: Option<bool>,
    },
    #[serde(alias = "remove_track_clips", alias = "clear_track_clips")]
    DeleteTrackClips {
        track_type: String,
        #[serde(default)]
        track_index: Option<usize>,
        #[serde(default = "default_true")]
        with_linked: bool,
    },
    RippleDeleteRange {
        start_ms: u64,
        end_ms: u64,
        #[serde(default)]
        mode: Option<String>,
    },
    TrimClip {
        clip_id: u64,
        new_start_ms: u64,
        new_duration_ms: u64,
    },
    MoveClip {
        clip_id: u64,
        new_start_ms: u64,
    },
    SplitClip {
        clip_id: u64,
        at_ms: u64,
    },
    ShiftSubtitlesRange {
        start_ms: u64,
        end_ms: u64,
        delta_ms: i64,
    },
    GenerateSubtitles {
        #[serde(default)]
        track_index: Option<usize>,
        entries: Vec<SubtitleGenerateEntry>,
    },
    MoveSubtitle {
        clip_id: u64,
        new_start_ms: u64,
        #[serde(default)]
        to_track_index: Option<usize>,
    },
    BatchUpdateSubtitles {
        updates: Vec<SubtitlePatch>,
    },
    DeleteSubtitleRange {
        start_ms: u64,
        end_ms: u64,
        #[serde(default)]
        track_indices: Option<Vec<usize>>,
    },
    ApplyEffect {
        clip_id: u64,
        effect: String,
        #[serde(default)]
        params: Option<Value>,
    },
    UpdateEffectParams {
        clip_id: u64,
        effect: String,
        #[serde(default)]
        params: Option<Value>,
    },
    RemoveEffect {
        clip_id: u64,
        effect: String,
    },
    ApplyTransition {
        clip_id: u64,
        transition: String,
    },
    UpdateTransition {
        clip_id: u64,
        transition: String,
        #[serde(default)]
        params: Option<Value>,
    },
    RemoveTransition {
        clip_id: u64,
        #[serde(default)]
        transition: Option<String>,
    },
    /// Insert a non-destructive semantic layer marker (e.g. B-roll planning annotation).
    InsertSemanticClip {
        start_ms: u64,
        duration_ms: u64,
        #[serde(default)]
        semantic_type: Option<String>,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        prompt_schema: Option<Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TimelineEditPlanRequest {
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub based_on_revision: Option<String>,
    #[serde(default)]
    pub intent: Option<String>,
    #[serde(default)]
    pub atomic: Option<bool>,
    #[serde(default)]
    pub operations: Vec<TimelineEditOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEditValidationResponse {
    pub ok: bool,
    pub before_revision: String,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub estimated_removed_ms: u64,
    pub affected_clip_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEditApplyResponse {
    pub ok: bool,
    pub before_revision: String,
    pub after_revision: Option<String>,
    pub applied_ops: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_semantic_type_text() -> String {
    "content_support".to_string()
}

fn semantic_schema_provider(schema: &Value) -> String {
    schema
        .get("provider")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "veo_3_1".to_string())
}

fn semantic_schema_asset_mode(schema: &Value) -> String {
    schema
        .get("asset_mode")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "video".to_string())
}

fn semantic_schema_provider_limit_sec(schema: &Value, provider: &str) -> Option<f64> {
    schema
        .get("provider_limits")
        .and_then(Value::as_object)
        .and_then(|root| root.get(provider))
        .and_then(Value::as_object)
        .and_then(|item| item.get("max_duration_sec"))
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0)
}

fn default_rms_threshold_db() -> f32 {
    -30.0
}

fn default_min_silence_ms() -> u64 {
    280
}

fn default_pad_ms() -> u64 {
    80
}

fn default_detect_low_energy_repeats() -> bool {
    false
}

fn default_repeat_similarity_threshold() -> f32 {
    0.82
}

fn default_repeat_window_ms() -> u64 {
    8000
}

fn default_transcript_uncertainty_threshold() -> f32 {
    0.40
}

fn default_transcript_min_duration_ms() -> u64 {
    260
}

fn default_transcript_edge_pad_ms() -> u64 {
    70
}

fn default_subtitle_gap_mode() -> String {
    "balanced".to_string()
}

fn default_subtitle_gap_cut_strategy() -> String {
    "subtitle_only".to_string()
}

fn default_audio_align_min_overlap_ms() -> u64 {
    80
}

fn default_semantic_window_ms() -> u64 {
    30_000
}

fn default_semantic_similarity_threshold() -> f32 {
    0.75
}

fn default_subtitle_long_silence_min_ms() -> u64 {
    2_500
}

fn default_subtitle_long_silence_rms_threshold_db() -> f32 {
    -45.0
}

fn default_subtitle_long_silence_pad_ms() -> u64 {
    60
}

fn normalize_autonomous_aggressiveness(raw: Option<&str>, goal: &str) -> String {
    let from_request = raw.unwrap_or_default().trim().to_ascii_lowercase();
    if matches!(
        from_request.as_str(),
        "conservative" | "balanced" | "aggressive"
    ) {
        return from_request;
    }

    let goal_lower = goal.to_ascii_lowercase();
    let has_aggressive = [
        "aggressive",
        "hard cut",
        "tight cut",
        "faster pace",
        "maximum trim",
    ]
    .iter()
    .any(|kw| goal_lower.contains(kw));
    if has_aggressive {
        return "aggressive".to_string();
    }

    let has_conservative = [
        "conservative",
        "safe",
        "minimal",
        "light touch",
        "keep context",
    ]
    .iter()
    .any(|kw| goal_lower.contains(kw));
    if has_conservative {
        return "conservative".to_string();
    }

    "balanced".to_string()
}

fn autonomous_target_removed_ratio(aggressiveness: &str) -> f32 {
    match aggressiveness {
        "conservative" => 0.06,
        "aggressive" => 0.20,
        _ => 0.12,
    }
}

fn sum_segment_duration_ms(segments: &[TimelineSegment]) -> u64 {
    segments.iter().fold(0_u64, |acc, s| {
        acc.saturating_add(s.end_ms.saturating_sub(s.start_ms))
    })
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn dur_ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

fn clip_view(clip: &Clip) -> TimelineClipView {
    TimelineClipView {
        clip_id: clip.id,
        label: clip.label.clone(),
        file_path: clip.file_path.clone(),
        start_ms: dur_ms(clip.start),
        duration_ms: dur_ms(clip.duration),
        source_in_ms: dur_ms(clip.source_in),
        media_duration_ms: dur_ms(clip.media_duration),
        link_group_id: clip.link_group_id,
    }
}

fn subtitle_view(clip: &SubtitleClip) -> TimelineSubtitleClipView {
    TimelineSubtitleClipView {
        clip_id: clip.id,
        text: clip.text.clone(),
        start_ms: dur_ms(clip.start),
        duration_ms: dur_ms(clip.duration),
        pos_x: clip.pos_x,
        pos_y: clip.pos_y,
        font_size: clip.font_size,
        color_rgba: clip.color_rgba,
        group_id: clip.group_id,
    }
}

fn semantic_view(clip: &SemanticClip) -> TimelineSemanticClipView {
    TimelineSemanticClipView {
        clip_id: clip.id,
        semantic_type: if clip.semantic_type.trim().is_empty() {
            default_semantic_type_text()
        } else {
            clip.semantic_type.trim().to_string()
        },
        label: clip.label.clone(),
        start_ms: duration_to_ms(clip.start),
        duration_ms: duration_to_ms(clip.duration),
        prompt_schema: clip.prompt_schema.clone(),
    }
}

fn to_segment(start: Duration, end: Duration) -> Option<TimelineSegment> {
    if end <= start {
        return None;
    }
    Some(TimelineSegment {
        start_ms: dur_ms(start),
        end_ms: dur_ms(end),
    })
}

fn normalize_segments(mut segments: Vec<(Duration, Duration)>) -> Vec<(Duration, Duration)> {
    segments.retain(|(s, e)| *e > *s);
    segments.sort_by_key(|(s, _)| *s);
    let mut out: Vec<(Duration, Duration)> = Vec::new();
    for (start, end) in segments {
        if let Some((_, last_end)) = out.last_mut()
            && start <= *last_end
        {
            if end > *last_end {
                *last_end = end;
            }
            continue;
        }
        out.push((start, end));
    }
    out
}

fn subtract_segments(
    base: &[(Duration, Duration)],
    subtract: &[(Duration, Duration)],
) -> Vec<(Duration, Duration)> {
    if base.is_empty() {
        return Vec::new();
    }
    if subtract.is_empty() {
        return base.to_vec();
    }

    let mut out = Vec::new();
    for (b_start, b_end) in base {
        let mut cursor = *b_start;
        for (s_start, s_end) in subtract {
            if *s_end <= cursor {
                continue;
            }
            if *s_start >= *b_end {
                break;
            }
            if *s_start > cursor {
                out.push((cursor, (*s_start).min(*b_end)));
            }
            if *s_end >= *b_end {
                cursor = *b_end;
                break;
            }
            cursor = (*s_end).max(cursor);
        }
        if cursor < *b_end {
            out.push((cursor, *b_end));
        }
    }
    normalize_segments(out)
}

fn ms_to_duration(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn duration_to_ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApiTrackKind {
    V1,
    Audio,
    Video,
    Subtitle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipLocation {
    V1,
    Audio(usize),
    Video(usize),
}

fn parse_track_kind(raw: &str) -> Option<ApiTrackKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "v1" => Some(ApiTrackKind::V1),
        "audio" | "a" => Some(ApiTrackKind::Audio),
        "video" | "video_overlay" | "overlay" | "v2plus" | "v2+" => Some(ApiTrackKind::Video),
        "subtitle" | "s" => Some(ApiTrackKind::Subtitle),
        _ => None,
    }
}

fn track_state_key(kind: ApiTrackKind, index: usize) -> String {
    match kind {
        ApiTrackKind::V1 => "v1".to_string(),
        ApiTrackKind::Audio => format!("audio:{index}"),
        ApiTrackKind::Video => format!("video:{index}"),
        ApiTrackKind::Subtitle => format!("subtitle:{index}"),
    }
}

fn track_exists(global: &GlobalState, kind: ApiTrackKind, index: usize) -> bool {
    match kind {
        ApiTrackKind::V1 => index == 0,
        ApiTrackKind::Audio => index < global.audio_tracks.len(),
        ApiTrackKind::Video => index < global.video_tracks.len(),
        ApiTrackKind::Subtitle => index < global.subtitle_tracks.len(),
    }
}

fn track_visible(global: &GlobalState, kind: ApiTrackKind, index: usize) -> bool {
    global
        .track_visibility
        .get(&track_state_key(kind, index))
        .copied()
        .unwrap_or(true)
}

fn track_locked(global: &GlobalState, kind: ApiTrackKind, index: usize) -> bool {
    global
        .track_lock
        .get(&track_state_key(kind, index))
        .copied()
        .unwrap_or(false)
}

fn track_muted(global: &GlobalState, kind: ApiTrackKind, index: usize) -> bool {
    global
        .track_mute
        .get(&track_state_key(kind, index))
        .copied()
        .unwrap_or(false)
}

fn set_track_visible(global: &mut GlobalState, kind: ApiTrackKind, index: usize, visible: bool) {
    let key = track_state_key(kind, index);
    if visible {
        global.track_visibility.remove(&key);
    } else {
        global.track_visibility.insert(key, false);
    }
}

fn set_track_locked(global: &mut GlobalState, kind: ApiTrackKind, index: usize, locked: bool) {
    let key = track_state_key(kind, index);
    if locked {
        global.track_lock.insert(key, true);
    } else {
        global.track_lock.remove(&key);
    }
}

fn set_track_muted(global: &mut GlobalState, kind: ApiTrackKind, index: usize, muted: bool) {
    let key = track_state_key(kind, index);
    if muted {
        global.track_mute.insert(key, true);
    } else {
        global.track_mute.remove(&key);
    }
}

fn remap_indexed_track_map(map: &mut HashMap<String, bool>, prefix: &str, removed_index: usize) {
    let mut updated: Vec<(String, bool)> = Vec::new();
    for (key, value) in map.iter() {
        if let Some(idx_raw) = key.strip_prefix(prefix)
            && let Ok(idx) = idx_raw.parse::<usize>()
        {
            if idx == removed_index {
                continue;
            }
            if idx > removed_index {
                updated.push((format!("{prefix}{}", idx.saturating_sub(1)), *value));
            } else {
                updated.push((key.clone(), *value));
            }
        } else {
            updated.push((key.clone(), *value));
        }
    }
    map.clear();
    for (key, value) in updated {
        map.insert(key, value);
    }
}

fn remap_track_state_after_remove(
    global: &mut GlobalState,
    kind: ApiTrackKind,
    removed_index: usize,
) {
    match kind {
        ApiTrackKind::Audio => {
            remap_indexed_track_map(&mut global.track_visibility, "audio:", removed_index);
            remap_indexed_track_map(&mut global.track_lock, "audio:", removed_index);
            remap_indexed_track_map(&mut global.track_mute, "audio:", removed_index);
        }
        ApiTrackKind::Video => {
            remap_indexed_track_map(&mut global.track_visibility, "video:", removed_index);
            remap_indexed_track_map(&mut global.track_lock, "video:", removed_index);
            remap_indexed_track_map(&mut global.track_mute, "video:", removed_index);
        }
        ApiTrackKind::Subtitle => {
            remap_indexed_track_map(&mut global.track_visibility, "subtitle:", removed_index);
            remap_indexed_track_map(&mut global.track_lock, "subtitle:", removed_index);
            remap_indexed_track_map(&mut global.track_mute, "subtitle:", removed_index);
        }
        ApiTrackKind::V1 => {}
    }
}

fn find_clip_location(global: &GlobalState, clip_id: u64) -> Option<ClipLocation> {
    if global.v1_clips.iter().any(|c| c.id == clip_id) {
        return Some(ClipLocation::V1);
    }
    for (idx, track) in global.audio_tracks.iter().enumerate() {
        if track.clips.iter().any(|c| c.id == clip_id) {
            return Some(ClipLocation::Audio(idx));
        }
    }
    for (idx, track) in global.video_tracks.iter().enumerate() {
        if track.clips.iter().any(|c| c.id == clip_id) {
            return Some(ClipLocation::Video(idx));
        }
    }
    None
}

fn find_subtitle_location(global: &GlobalState, clip_id: u64) -> Option<(usize, usize)> {
    for (track_idx, track) in global.subtitle_tracks.iter().enumerate() {
        if let Some(clip_idx) = track.clips.iter().position(|c| c.id == clip_id) {
            return Some((track_idx, clip_idx));
        }
    }
    None
}

fn find_clip_ref(global: &GlobalState, clip_id: u64) -> Option<&Clip> {
    match find_clip_location(global, clip_id)? {
        ClipLocation::V1 => global.v1_clips.iter().find(|clip| clip.id == clip_id),
        ClipLocation::Audio(track_index) => global
            .audio_tracks
            .get(track_index)
            .and_then(|track| track.clips.iter().find(|clip| clip.id == clip_id)),
        ClipLocation::Video(track_index) => global
            .video_tracks
            .get(track_index)
            .and_then(|track| track.clips.iter().find(|clip| clip.id == clip_id)),
    }
}

fn find_subtitle_ref(global: &GlobalState, clip_id: u64) -> Option<&SubtitleClip> {
    let (track_index, clip_index) = find_subtitle_location(global, clip_id)?;
    global
        .subtitle_tracks
        .get(track_index)
        .and_then(|track| track.clips.get(clip_index))
}

fn first_missing_track_index(
    global: &GlobalState,
    kind: ApiTrackKind,
    indexes: &[usize],
) -> Option<usize> {
    indexes
        .iter()
        .copied()
        .find(|index| !track_exists(global, kind, *index))
}

fn resolve_indexed_track_target(
    track_type: &str,
    kind: ApiTrackKind,
    track_index: Option<usize>,
) -> Result<usize, TimelineEditError> {
    match kind {
        ApiTrackKind::V1 => match track_index {
            None | Some(0) => Ok(0),
            Some(index) => Err(TimelineEditError::TrackTypeOnlySupportsTrackIndexZero {
                track_type: track_type.to_string(),
                index,
            }),
        },
        ApiTrackKind::Audio | ApiTrackKind::Video | ApiTrackKind::Subtitle => {
            track_index.ok_or(TimelineEditError::TrackTypeRequiresTrackIndex {
                track_type: track_type.to_string(),
            })
        }
    }
}

fn clip_ids_on_track(global: &GlobalState, kind: ApiTrackKind, index: usize) -> Vec<u64> {
    match kind {
        ApiTrackKind::V1 => global.v1_clips.iter().map(|clip| clip.id).collect(),
        ApiTrackKind::Audio => global
            .audio_tracks
            .get(index)
            .map(|track| track.clips.iter().map(|clip| clip.id).collect())
            .unwrap_or_default(),
        ApiTrackKind::Video => global
            .video_tracks
            .get(index)
            .map(|track| track.clips.iter().map(|clip| clip.id).collect())
            .unwrap_or_default(),
        ApiTrackKind::Subtitle => Vec::new(),
    }
}

fn subtitle_ids_on_track(global: &GlobalState, index: usize) -> Vec<u64> {
    global
        .subtitle_tracks
        .get(index)
        .map(|track| track.clips.iter().map(|clip| clip.id).collect())
        .unwrap_or_default()
}

fn expand_clip_ids_by_link_group(global: &GlobalState, selected: &HashSet<u64>) -> HashSet<u64> {
    let mut link_groups = HashSet::new();
    for clip in global
        .v1_clips
        .iter()
        .chain(
            global
                .audio_tracks
                .iter()
                .flat_map(|track| track.clips.iter()),
        )
        .chain(
            global
                .video_tracks
                .iter()
                .flat_map(|track| track.clips.iter()),
        )
    {
        if selected.contains(&clip.id)
            && let Some(link_group_id) = clip.link_group_id
        {
            link_groups.insert(link_group_id);
        }
    }

    if link_groups.is_empty() {
        return selected.clone();
    }

    let mut expanded = selected.clone();
    for clip in global
        .v1_clips
        .iter()
        .chain(
            global
                .audio_tracks
                .iter()
                .flat_map(|track| track.clips.iter()),
        )
        .chain(
            global
                .video_tracks
                .iter()
                .flat_map(|track| track.clips.iter()),
        )
    {
        if clip
            .link_group_id
            .is_some_and(|link_group_id| link_groups.contains(&link_group_id))
        {
            expanded.insert(clip.id);
        }
    }
    expanded
}

fn sanitize_selected_clip_ids(global: &mut GlobalState) {
    let existing_clip_ids: HashSet<u64> = global
        .v1_clips
        .iter()
        .chain(
            global
                .audio_tracks
                .iter()
                .flat_map(|track| track.clips.iter()),
        )
        .chain(
            global
                .video_tracks
                .iter()
                .flat_map(|track| track.clips.iter()),
        )
        .map(|clip| clip.id)
        .collect();
    global
        .selected_clip_ids
        .retain(|clip_id| existing_clip_ids.contains(clip_id));
    if global
        .selected_clip_id
        .is_some_and(|clip_id| !existing_clip_ids.contains(&clip_id))
    {
        global.selected_clip_id = global.selected_clip_ids.last().copied();
    }
}

fn sanitize_selected_subtitle_ids(global: &mut GlobalState) {
    let existing_subtitle_ids: HashSet<u64> = global
        .subtitle_tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .map(|clip| clip.id)
        .collect();
    global
        .selected_subtitle_ids
        .retain(|clip_id| existing_subtitle_ids.contains(clip_id));
    if global
        .selected_subtitle_id
        .is_some_and(|clip_id| !existing_subtitle_ids.contains(&clip_id))
    {
        global.selected_subtitle_id = global.selected_subtitle_ids.last().copied();
    }
}

fn remove_clip_ids_without_ripple(global: &mut GlobalState, clip_ids: &HashSet<u64>) -> bool {
    if clip_ids.is_empty() {
        return false;
    }

    let before_v1 = global.v1_clips.len();
    global.v1_clips.retain(|clip| !clip_ids.contains(&clip.id));
    let mut changed = before_v1 != global.v1_clips.len();

    for track in &mut global.audio_tracks {
        let before = track.clips.len();
        track.clips.retain(|clip| !clip_ids.contains(&clip.id));
        if before != track.clips.len() {
            changed = true;
        }
    }

    for track in &mut global.video_tracks {
        let before = track.clips.len();
        track.clips.retain(|clip| !clip_ids.contains(&clip.id));
        if before != track.clips.len() {
            changed = true;
        }
    }

    if changed {
        sanitize_selected_clip_ids(global);
    }
    changed
}

fn remove_subtitle_ids(global: &mut GlobalState, subtitle_ids: &HashSet<u64>) -> bool {
    if subtitle_ids.is_empty() {
        return false;
    }
    let mut changed = false;
    for track in &mut global.subtitle_tracks {
        let before = track.clips.len();
        track.clips.retain(|clip| !subtitle_ids.contains(&clip.id));
        if before != track.clips.len() {
            changed = true;
        }
    }
    if changed {
        sanitize_selected_subtitle_ids(global);
    }
    changed
}

fn resolve_insert_source(
    global: &GlobalState,
    media_pool_item_id: Option<usize>,
    path: Option<&str>,
) -> Result<(String, String, Duration), TimelineEditError> {
    if let Some(media_pool_item_id) = media_pool_item_id {
        let item = global.media_pool.get(media_pool_item_id).ok_or(
            TimelineEditError::MediaPoolItemOutOfRange {
                media_pool_item_id,
                media_pool_len: global.media_pool.len(),
            },
        )?;
        return Ok((item.path.clone(), item.name.clone(), item.duration));
    }

    let path = path.unwrap_or("").trim();
    if path.is_empty() {
        return Err(TimelineEditError::InsertClipRequiresMediaPoolItemIdOrPath);
    }

    if let Some(item) = global
        .media_pool
        .iter()
        .find(|item| item.path == path || item.name == path)
    {
        return Ok((item.path.clone(), item.name.clone(), item.duration));
    }

    let file_name = Path::new(path)
        .file_name()
        .and_then(|v| v.to_str())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(path)
        .to_string();
    Ok((path.to_string(), file_name, Duration::ZERO))
}

fn resolve_insert_window(
    media_duration: Duration,
    source_in_ms: Option<u64>,
    source_out_ms: Option<u64>,
    duration_ms: Option<u64>,
    op_label: &str,
    warnings: &mut Vec<String>,
) -> Result<(Duration, Duration), TimelineEditError> {
    let media_total_ms = duration_to_ms(media_duration);
    let mut source_in = source_in_ms.unwrap_or(0);

    if media_total_ms > 0 && source_in > media_total_ms {
        source_in = media_total_ms;
        warnings.push(format!(
            "{op_label}: source_in_ms exceeds media duration and was clamped."
        ));
    }

    if let Some(mut source_out) = source_out_ms {
        if source_out <= source_in {
            return Err(TimelineEditError::SourceOutMustBeGreaterThanSourceIn);
        }
        if media_total_ms > 0 && source_out > media_total_ms {
            source_out = media_total_ms;
            warnings.push(format!(
                "{op_label}: source_out_ms exceeds media duration and was clamped."
            ));
        }
        if source_out <= source_in {
            return Err(TimelineEditError::SourceWindowEmptyAfterClamping);
        }
        if duration_ms.is_some() {
            warnings.push(format!(
                "{op_label}: duration_ms ignored because source_out_ms is provided."
            ));
        }
        return Ok((
            Duration::from_millis(source_in),
            Duration::from_millis(source_out.saturating_sub(source_in).max(1)),
        ));
    }

    let mut resolved_duration_ms = if let Some(duration_ms) = duration_ms {
        duration_ms
    } else if media_total_ms > 0 {
        media_total_ms.saturating_sub(source_in)
    } else {
        1000
    };

    if resolved_duration_ms == 0 {
        return Err(TimelineEditError::ResolvedInsertDurationIsZero);
    }

    if media_total_ms > 0 {
        let max_duration = media_total_ms.saturating_sub(source_in);
        if resolved_duration_ms > max_duration {
            resolved_duration_ms = max_duration;
            warnings.push(format!(
                "{op_label}: duration exceeds media range and was clamped."
            ));
        }
        if resolved_duration_ms == 0 {
            return Err(TimelineEditError::ResolvedInsertDurationOutsideMediaRange);
        }
    }

    Ok((
        Duration::from_millis(source_in),
        Duration::from_millis(resolved_duration_ms),
    ))
}

fn set_clip_source_window(
    clip: &mut Clip,
    source_in: Duration,
    duration: Duration,
    media_duration: Duration,
) {
    clip.source_in = source_in;
    clip.duration = duration.max(Duration::from_millis(1));
    if media_duration > Duration::ZERO {
        clip.media_duration = media_duration;
    } else {
        clip.media_duration = clip
            .media_duration
            .max(source_in.saturating_add(clip.duration));
    }
}

fn update_inserted_clip_source_window(
    global: &mut GlobalState,
    inserted_clip_id: u64,
    source_in: Duration,
    duration: Duration,
    media_duration: Duration,
) {
    let mut inserted_link_group: Option<u64> = None;

    if let Some(clip) = global
        .v1_clips
        .iter_mut()
        .find(|clip| clip.id == inserted_clip_id)
    {
        inserted_link_group = clip.link_group_id;
        set_clip_source_window(clip, source_in, duration, media_duration);
    }
    for track in &mut global.audio_tracks {
        if let Some(clip) = track
            .clips
            .iter_mut()
            .find(|clip| clip.id == inserted_clip_id)
        {
            inserted_link_group = clip.link_group_id;
            set_clip_source_window(clip, source_in, duration, media_duration);
        }
    }
    for track in &mut global.video_tracks {
        if let Some(clip) = track
            .clips
            .iter_mut()
            .find(|clip| clip.id == inserted_clip_id)
        {
            inserted_link_group = clip.link_group_id;
            set_clip_source_window(clip, source_in, duration, media_duration);
        }
    }

    if let Some(link_group_id) = inserted_link_group {
        for track in &mut global.audio_tracks {
            for clip in &mut track.clips {
                if clip.id != inserted_clip_id && clip.link_group_id == Some(link_group_id) {
                    set_clip_source_window(clip, source_in, duration, media_duration);
                }
            }
        }
        for track in &mut global.video_tracks {
            for clip in &mut track.clips {
                if clip.id != inserted_clip_id && clip.link_group_id == Some(link_group_id) {
                    set_clip_source_window(clip, source_in, duration, media_duration);
                }
            }
        }
    }
}

fn apply_insert_clip_common(
    global: &mut GlobalState,
    op_label: &str,
    warnings: &mut Vec<String>,
    track_type: &str,
    track_index: Option<usize>,
    media_pool_item_id: Option<usize>,
    path: Option<&str>,
    start_ms: Option<u64>,
    source_in_ms: Option<u64>,
    source_out_ms: Option<u64>,
    duration_ms: Option<u64>,
) -> Result<bool, TimelineEditError> {
    let kind =
        parse_track_kind(track_type).ok_or(TimelineEditError::InvalidTrackTypeForInsertClip {
            track_type: track_type.to_string(),
        })?;
    let (resolved_path, resolved_name, resolved_media_duration) =
        resolve_insert_source(global, media_pool_item_id, path)?;
    global.playhead = ms_to_duration(start_ms.unwrap_or(duration_to_ms(global.playhead)));

    let (source_in, insert_duration) = resolve_insert_window(
        resolved_media_duration,
        source_in_ms,
        source_out_ms,
        duration_ms,
        op_label,
        warnings,
    )?;

    global.active_source_path = resolved_path;
    global.active_source_name = resolved_name;
    global.active_source_duration = if resolved_media_duration > Duration::ZERO {
        resolved_media_duration
    } else {
        source_in.saturating_add(insert_duration)
    };

    let changed = match kind {
        ApiTrackKind::V1 => {
            let before_v1_len = global.v1_clips.len();
            let before_a1_len = global
                .audio_tracks
                .first()
                .map(|track| track.clips.len())
                .unwrap_or(0);
            let prev_mode = global.v1_move_mode;
            global.v1_move_mode = crate::core::global_state::V1MoveMode::Free;
            global.insert_active_source_v1(insert_duration);
            global.v1_move_mode = prev_mode;
            global.v1_clips.len() > before_v1_len
                || global
                    .audio_tracks
                    .first()
                    .map(|track| track.clips.len())
                    .unwrap_or(0)
                    > before_a1_len
        }
        ApiTrackKind::Audio => {
            let track_index =
                track_index.ok_or(TimelineEditError::InsertClipRequiresAudioTrackIndex)?;
            let before_len = global
                .audio_tracks
                .get(track_index)
                .map(|track| track.clips.len())
                .unwrap_or(0);
            global.ripple_insert_active_source_audio(track_index, insert_duration);
            global
                .audio_tracks
                .get(track_index)
                .map(|track| track.clips.len() > before_len)
                .unwrap_or(false)
        }
        ApiTrackKind::Video => {
            let track_index =
                track_index.ok_or(TimelineEditError::InsertClipRequiresVideoTrackIndex)?;
            let before_len = global
                .video_tracks
                .get(track_index)
                .map(|track| track.clips.len())
                .unwrap_or(0);
            global.ripple_insert_active_source_video(track_index, insert_duration);
            global
                .video_tracks
                .get(track_index)
                .map(|track| track.clips.len() > before_len)
                .unwrap_or(false)
        }
        ApiTrackKind::Subtitle => Err(TimelineEditError::InsertClipDoesNotSupportSubtitleTracks)?,
    };

    if changed && let Some(inserted_clip_id) = global.selected_clip_id {
        update_inserted_clip_source_window(
            global,
            inserted_clip_id,
            source_in,
            insert_duration,
            resolved_media_duration,
        );
    }
    Ok(changed)
}

fn with_selected_clip<R>(
    global: &mut GlobalState,
    clip_id: u64,
    f: impl FnOnce(&mut GlobalState) -> R,
) -> R {
    let prev_selected_clip_id = global.selected_clip_id;
    let prev_selected_clip_ids = global.selected_clip_ids.clone();
    let prev_selected_subtitle_id = global.selected_subtitle_id;
    let prev_selected_subtitle_ids = global.selected_subtitle_ids.clone();

    global.selected_clip_id = Some(clip_id);
    global.selected_clip_ids = vec![clip_id];
    global.selected_subtitle_id = None;
    global.selected_subtitle_ids.clear();

    let out = f(global);

    global.selected_clip_id = prev_selected_clip_id;
    global.selected_clip_ids = prev_selected_clip_ids;
    global.selected_subtitle_id = prev_selected_subtitle_id;
    global.selected_subtitle_ids = prev_selected_subtitle_ids;
    out
}

fn read_f32_param(params: Option<&Value>, key: &str, fallback: f32) -> f32 {
    params
        .and_then(|p| p.get(key))
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(fallback)
}

fn read_str_param<'a>(params: Option<&'a Value>, key: &str) -> Option<&'a str> {
    params.and_then(|p| p.get(key)).and_then(|v| v.as_str())
}

fn parse_slide_direction(raw: Option<&str>) -> Option<crate::core::global_state::SlideDirection> {
    match raw?.trim().to_ascii_lowercase().as_str() {
        "left" => Some(crate::core::global_state::SlideDirection::Left),
        "right" => Some(crate::core::global_state::SlideDirection::Right),
        "up" => Some(crate::core::global_state::SlideDirection::Up),
        "down" => Some(crate::core::global_state::SlideDirection::Down),
        _ => None,
    }
}

fn normalize_effect_name(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "tint" | "hsla" | "hsla_overlay" | "overlay_hsla" => "hsla_overlay".to_string(),
        other => other.to_string(),
    }
}

fn effect_name_supported(raw: &str) -> bool {
    matches!(
        normalize_effect_name(raw).as_str(),
        "brightness"
            | "contrast"
            | "saturation"
            | "opacity"
            | "rotation"
            | "rotate"
            | "blur"
            | "blur_sigma"
            | "fade_in"
            | "fade_out"
            | "dissolve_in"
            | "dissolve_out"
            | "slide"
            | "zoom"
            | "shock_zoom"
            | "shockzoom"
            | "hsla_overlay"
    )
}

fn parse_transition_type(raw: &str) -> Option<TransitionType> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "fade" => Some(TransitionType::Fade),
        "dissolve" => Some(TransitionType::Dissolve),
        "slide" => Some(TransitionType::Slide),
        "zoom" => Some(TransitionType::Zoom),
        "shock_zoom" | "shockzoom" => Some(TransitionType::ShockZoom),
        _ => None,
    }
}

fn transition_name_supported(raw: &str) -> bool {
    parse_transition_type(raw).is_some()
}

fn apply_or_update_effect(
    global: &mut GlobalState,
    clip_id: u64,
    effect: &str,
    params: Option<&Value>,
) -> Result<(), TimelineEditError> {
    if find_clip_location(global, clip_id).is_none() {
        return Err(TimelineEditError::ClipNotFound { clip_id });
    }
    let effect = normalize_effect_name(effect);

    with_selected_clip(global, clip_id, |gs| match effect.as_str() {
        "brightness" => {
            let cur = gs.get_selected_clip_brightness().unwrap_or(0.0);
            gs.set_selected_clip_brightness(read_f32_param(params, "value", cur));
        }
        "contrast" => {
            let cur = gs.get_selected_clip_contrast().unwrap_or(1.0);
            gs.set_selected_clip_contrast(read_f32_param(params, "value", cur));
        }
        "saturation" => {
            let cur = gs.get_selected_clip_saturation().unwrap_or(1.0);
            gs.set_selected_clip_saturation(read_f32_param(params, "value", cur));
        }
        "opacity" => {
            let cur = gs.get_selected_clip_opacity().unwrap_or(1.0);
            gs.set_selected_clip_opacity(read_f32_param(params, "value", cur));
        }
        "rotation" | "rotate" => {
            let cur = gs.get_selected_clip_rotation().unwrap_or(0.0);
            gs.set_selected_clip_rotation(read_f32_param(params, "value", cur));
        }
        "blur" | "blur_sigma" => {
            let cur = gs.get_selected_clip_blur_sigma().unwrap_or(0.0);
            gs.set_selected_clip_blur_sigma(read_f32_param(params, "value", cur));
        }
        "fade_in" => {
            let cur = gs.get_selected_clip_fade_in().unwrap_or(0.0);
            gs.set_selected_clip_fade_in(read_f32_param(params, "value", cur));
        }
        "fade_out" => {
            let cur = gs.get_selected_clip_fade_out().unwrap_or(0.0);
            gs.set_selected_clip_fade_out(read_f32_param(params, "value", cur));
        }
        "dissolve_in" => {
            let cur = gs.get_selected_clip_dissolve_in().unwrap_or(0.0);
            gs.set_selected_clip_dissolve_in(read_f32_param(params, "value", cur));
        }
        "dissolve_out" => {
            let cur = gs.get_selected_clip_dissolve_out().unwrap_or(0.0);
            gs.set_selected_clip_dissolve_out(read_f32_param(params, "value", cur));
        }
        "slide" => {
            let (_, _, in_dur, out_dur) = gs.get_selected_clip_slide().unwrap_or((
                crate::core::global_state::SlideDirection::Left,
                crate::core::global_state::SlideDirection::Left,
                0.0,
                0.0,
            ));
            let next_in = read_f32_param(params, "in", in_dur);
            let next_out = read_f32_param(params, "out", out_dur);
            gs.set_selected_clip_slide_in(next_in);
            gs.set_selected_clip_slide_out(next_out);
            if let Some(dir) = parse_slide_direction(read_str_param(params, "in_direction")) {
                gs.set_selected_clip_slide_in_direction(dir);
            }
            if let Some(dir) = parse_slide_direction(read_str_param(params, "out_direction")) {
                gs.set_selected_clip_slide_out_direction(dir);
            }
        }
        "zoom" => {
            let (zin, zout, amount) = gs.get_selected_clip_zoom().unwrap_or((0.0, 0.0, 1.2));
            gs.set_selected_clip_zoom_in(read_f32_param(params, "in", zin));
            gs.set_selected_clip_zoom_out(read_f32_param(params, "out", zout));
            gs.set_selected_clip_zoom_amount(read_f32_param(params, "amount", amount));
        }
        "shock_zoom" | "shockzoom" => {
            let (zin, zout, amount) = gs.get_selected_clip_shock_zoom().unwrap_or((0.0, 0.0, 1.2));
            gs.set_selected_clip_shock_zoom_in(read_f32_param(params, "in", zin));
            gs.set_selected_clip_shock_zoom_out(read_f32_param(params, "out", zout));
            gs.set_selected_clip_shock_zoom_amount(read_f32_param(params, "amount", amount));
        }
        "hsla_overlay" => {
            let (h, s, l, a) = gs
                .get_selected_clip_hsla_overlay()
                .unwrap_or((0.0, 0.0, 0.0, 0.0));
            gs.set_selected_clip_hsla_overlay_hue(read_f32_param(params, "hue", h));
            gs.set_selected_clip_hsla_overlay_saturation(read_f32_param(params, "saturation", s));
            gs.set_selected_clip_hsla_overlay_lightness(read_f32_param(params, "lightness", l));
            gs.set_selected_clip_hsla_overlay_alpha(read_f32_param(params, "alpha", a));
        }
        _ => {}
    });

    match effect.as_str() {
        "brightness" | "contrast" | "saturation" | "opacity" | "rotation" | "rotate" | "blur"
        | "blur_sigma" | "fade_in" | "fade_out" | "dissolve_in" | "dissolve_out" | "slide"
        | "zoom" | "shock_zoom" | "shockzoom" | "hsla_overlay" => Ok(()),
        _ => Err(TimelineEditError::UnsupportedEffect {
            effect: effect.to_string(),
        }),
    }
}

fn remove_effect(
    global: &mut GlobalState,
    clip_id: u64,
    effect: &str,
) -> Result<(), TimelineEditError> {
    if find_clip_location(global, clip_id).is_none() {
        return Err(TimelineEditError::ClipNotFound { clip_id });
    }
    let effect = normalize_effect_name(effect);

    with_selected_clip(global, clip_id, |gs| match effect.as_str() {
        "brightness" => gs.set_selected_clip_brightness(0.0),
        "contrast" => gs.set_selected_clip_contrast(1.0),
        "saturation" => gs.set_selected_clip_saturation(1.0),
        "opacity" => gs.set_selected_clip_opacity(1.0),
        "rotation" | "rotate" => gs.set_selected_clip_rotation(0.0),
        "blur" | "blur_sigma" => gs.set_selected_clip_blur_sigma(0.0),
        "fade_in" => gs.set_selected_clip_fade_in(0.0),
        "fade_out" => gs.set_selected_clip_fade_out(0.0),
        "dissolve_in" => gs.set_selected_clip_dissolve_in(0.0),
        "dissolve_out" => gs.set_selected_clip_dissolve_out(0.0),
        "slide" => {
            gs.set_selected_clip_slide_in(0.0);
            gs.set_selected_clip_slide_out(0.0);
        }
        "zoom" => {
            gs.set_selected_clip_zoom_in(0.0);
            gs.set_selected_clip_zoom_out(0.0);
            gs.set_selected_clip_zoom_amount(1.2);
        }
        "shock_zoom" | "shockzoom" => {
            gs.set_selected_clip_shock_zoom_in(0.0);
            gs.set_selected_clip_shock_zoom_out(0.0);
            gs.set_selected_clip_shock_zoom_amount(1.2);
        }
        "hsla_overlay" => {
            gs.set_selected_clip_hsla_overlay_hue(0.0);
            gs.set_selected_clip_hsla_overlay_saturation(0.0);
            gs.set_selected_clip_hsla_overlay_lightness(0.0);
            gs.set_selected_clip_hsla_overlay_alpha(0.0);
        }
        _ => {}
    });

    match effect.as_str() {
        "brightness" | "contrast" | "saturation" | "opacity" | "rotation" | "rotate" | "blur"
        | "blur_sigma" | "fade_in" | "fade_out" | "dissolve_in" | "dissolve_out" | "slide"
        | "zoom" | "shock_zoom" | "shockzoom" | "hsla_overlay" => Ok(()),
        _ => Err(TimelineEditError::UnsupportedEffect {
            effect: effect.to_string(),
        }),
    }
}

fn apply_transition(
    global: &mut GlobalState,
    clip_id: u64,
    transition: &str,
) -> Result<(), TimelineEditError> {
    let Some(kind) = parse_transition_type(transition) else {
        return Err(TimelineEditError::UnsupportedTransition {
            transition: transition.to_string(),
        });
    };
    if !global.apply_transition_to_clip(clip_id, kind) {
        return Err(TimelineEditError::FailedToApplyTransition {
            transition: transition.to_string(),
            clip_id,
        });
    }
    Ok(())
}

fn update_transition(
    global: &mut GlobalState,
    clip_id: u64,
    transition: &str,
    params: Option<&Value>,
) -> Result<(), TimelineEditError> {
    if find_clip_location(global, clip_id).is_none() {
        return Err(TimelineEditError::ClipNotFound { clip_id });
    }
    let transition = transition.trim().to_ascii_lowercase();
    with_selected_clip(global, clip_id, |gs| match transition.as_str() {
        "fade" => {
            let cur_in = gs.get_selected_clip_fade_in().unwrap_or(0.0);
            let cur_out = gs.get_selected_clip_fade_out().unwrap_or(0.0);
            gs.set_selected_clip_fade_in(read_f32_param(params, "in", cur_in));
            gs.set_selected_clip_fade_out(read_f32_param(params, "out", cur_out));
        }
        "dissolve" => {
            let cur_in = gs.get_selected_clip_dissolve_in().unwrap_or(0.0);
            let cur_out = gs.get_selected_clip_dissolve_out().unwrap_or(0.0);
            gs.set_selected_clip_dissolve_in(read_f32_param(params, "in", cur_in));
            gs.set_selected_clip_dissolve_out(read_f32_param(params, "out", cur_out));
        }
        "slide" => {
            let (_, _, in_dur, out_dur) = gs.get_selected_clip_slide().unwrap_or((
                crate::core::global_state::SlideDirection::Left,
                crate::core::global_state::SlideDirection::Left,
                0.0,
                0.0,
            ));
            gs.set_selected_clip_slide_in(read_f32_param(params, "in", in_dur));
            gs.set_selected_clip_slide_out(read_f32_param(params, "out", out_dur));
            if let Some(dir) = parse_slide_direction(read_str_param(params, "in_direction")) {
                gs.set_selected_clip_slide_in_direction(dir);
            }
            if let Some(dir) = parse_slide_direction(read_str_param(params, "out_direction")) {
                gs.set_selected_clip_slide_out_direction(dir);
            }
        }
        "zoom" => {
            let (zin, zout, amount) = gs.get_selected_clip_zoom().unwrap_or((0.0, 0.0, 1.2));
            gs.set_selected_clip_zoom_in(read_f32_param(params, "in", zin));
            gs.set_selected_clip_zoom_out(read_f32_param(params, "out", zout));
            gs.set_selected_clip_zoom_amount(read_f32_param(params, "amount", amount));
        }
        "shock_zoom" | "shockzoom" => {
            let (zin, zout, amount) = gs.get_selected_clip_shock_zoom().unwrap_or((0.0, 0.0, 1.2));
            gs.set_selected_clip_shock_zoom_in(read_f32_param(params, "in", zin));
            gs.set_selected_clip_shock_zoom_out(read_f32_param(params, "out", zout));
            gs.set_selected_clip_shock_zoom_amount(read_f32_param(params, "amount", amount));
        }
        _ => {}
    });
    match transition.as_str() {
        "fade" | "dissolve" | "slide" | "zoom" | "shock_zoom" | "shockzoom" => Ok(()),
        _ => Err(TimelineEditError::UnsupportedTransition {
            transition: transition.to_string(),
        }),
    }
}

fn remove_transition(
    global: &mut GlobalState,
    clip_id: u64,
    transition: Option<&str>,
) -> Result<(), TimelineEditError> {
    if find_clip_location(global, clip_id).is_none() {
        return Err(TimelineEditError::ClipNotFound { clip_id });
    }
    let normalized = transition
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "all".to_string());
    with_selected_clip(global, clip_id, |gs| match normalized.as_str() {
        "fade" => {
            gs.set_selected_clip_fade_in(0.0);
            gs.set_selected_clip_fade_out(0.0);
        }
        "dissolve" => {
            gs.set_selected_clip_dissolve_in(0.0);
            gs.set_selected_clip_dissolve_out(0.0);
        }
        "slide" => {
            gs.set_selected_clip_slide_in(0.0);
            gs.set_selected_clip_slide_out(0.0);
        }
        "zoom" => {
            gs.set_selected_clip_zoom_in(0.0);
            gs.set_selected_clip_zoom_out(0.0);
            gs.set_selected_clip_zoom_amount(1.2);
        }
        "shock_zoom" | "shockzoom" => {
            gs.set_selected_clip_shock_zoom_in(0.0);
            gs.set_selected_clip_shock_zoom_out(0.0);
            gs.set_selected_clip_shock_zoom_amount(1.2);
        }
        "all" => {
            gs.set_selected_clip_fade_in(0.0);
            gs.set_selected_clip_fade_out(0.0);
            gs.set_selected_clip_dissolve_in(0.0);
            gs.set_selected_clip_dissolve_out(0.0);
            gs.set_selected_clip_slide_in(0.0);
            gs.set_selected_clip_slide_out(0.0);
            gs.set_selected_clip_zoom_in(0.0);
            gs.set_selected_clip_zoom_out(0.0);
            gs.set_selected_clip_zoom_amount(1.2);
            gs.set_selected_clip_shock_zoom_in(0.0);
            gs.set_selected_clip_shock_zoom_out(0.0);
            gs.set_selected_clip_shock_zoom_amount(1.2);
        }
        _ => {}
    });
    match normalized.as_str() {
        "fade" | "dissolve" | "slide" | "zoom" | "shock_zoom" | "shockzoom" | "all" => Ok(()),
        _ => Err(TimelineEditError::UnsupportedTransition {
            transition: normalized,
        }),
    }
}

fn mask_to_segments(
    mask: &[bool],
    step_ms: u64,
    timeline_end_ms: u64,
) -> Vec<(Duration, Duration)> {
    if mask.is_empty() || step_ms == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < mask.len() {
        if !mask[idx] {
            idx += 1;
            continue;
        }
        let start_idx = idx;
        while idx < mask.len() && mask[idx] {
            idx += 1;
        }
        let end_idx = idx;
        let start_ms = (start_idx as u64).saturating_mul(step_ms);
        let end_ms = ((end_idx as u64).saturating_mul(step_ms)).min(timeline_end_ms);
        if end_ms > start_ms {
            out.push((ms_to_duration(start_ms), ms_to_duration(end_ms)));
        }
    }
    out
}

fn inflate_segments(
    segments: &[(Duration, Duration)],
    pad_ms: u64,
    limit: Duration,
) -> Vec<(Duration, Duration)> {
    if segments.is_empty() {
        return Vec::new();
    }
    let pad = Duration::from_millis(pad_ms);
    normalize_segments(
        segments
            .iter()
            .map(|(start, end)| (start.saturating_sub(pad), (*end + pad).min(limit)))
            .collect(),
    )
}

fn estimate_confidence_from_rms(avg_rms: f32, threshold_rms: f32) -> f32 {
    if threshold_rms <= 0.0 {
        return 0.7;
    }
    let ratio = (avg_rms / threshold_rms).clamp(0.0, 2.0);
    (0.96 - ratio * 0.25).clamp(0.55, 0.98)
}

fn resolve_subtitle_gap_mode(mode_raw: &str) -> (&'static str, u64, u64) {
    match mode_raw.trim().to_ascii_lowercase().as_str() {
        "conservative" => ("conservative", 350, 120),
        "aggressive" => ("aggressive", 150, 60),
        _ => ("balanced", 250, 100),
    }
}

fn normalize_subtitle_gap_cut_strategy(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "subtitle_audio_aligned"
        | "subtitle_with_audio_align"
        | "subtitle_plus_audio_align"
        | "audio_aligned" => "subtitle_audio_aligned",
        _ => "subtitle_only",
    }
}

fn intersect_ranges_with_min_overlap(
    left: &[(u64, u64)],
    right: &[(u64, u64)],
    min_overlap_ms: u64,
) -> Vec<(u64, u64)> {
    if left.is_empty() || right.is_empty() {
        return Vec::new();
    }
    let min_overlap_ms = min_overlap_ms.max(1);
    let mut out = Vec::new();
    for (left_start, left_end) in left {
        for (right_start, right_end) in right {
            if right_end <= left_start {
                continue;
            }
            if right_start >= left_end {
                break;
            }
            let start = (*left_start).max(*right_start);
            let end = (*left_end).min(*right_end);
            if end > start && end.saturating_sub(start) >= min_overlap_ms {
                out.push((start, end));
            }
        }
    }
    out
}

fn collect_subtitle_coverage_ranges_ms(
    global: &GlobalState,
    track_indices: &Option<Vec<usize>>,
    warnings: &mut Vec<String>,
) -> Vec<(u64, u64)> {
    let allowed_track_indices: Option<HashSet<usize>> = match track_indices {
        Some(indices) if !indices.is_empty() => {
            let mut out = HashSet::new();
            for idx in indices {
                if *idx < global.subtitle_tracks.len() {
                    out.insert(*idx);
                } else {
                    warnings.push(format!(
                        "Requested subtitle track index {} is out of range (track_count={}).",
                        idx,
                        global.subtitle_tracks.len()
                    ));
                }
            }
            Some(out)
        }
        _ => None,
    };

    let mut subtitle_segments_raw: Vec<(Duration, Duration)> = Vec::new();
    for (track_idx, track) in global.subtitle_tracks.iter().enumerate() {
        if let Some(allowed) = &allowed_track_indices
            && !allowed.contains(&track_idx)
        {
            continue;
        }
        for clip in &track.clips {
            if clip.duration.is_zero() {
                continue;
            }
            let start = clip.start;
            let end = clip.start + clip.duration;
            if end > start {
                subtitle_segments_raw.push((start, end));
            }
        }
    }

    normalize_segments(subtitle_segments_raw)
        .into_iter()
        .map(|(start, end)| (duration_to_ms(start), duration_to_ms(end)))
        .filter(|(start_ms, end_ms)| end_ms > start_ms)
        .collect()
}

fn collect_subtitle_covered_long_silence_ranges(
    global: &GlobalState,
    request: &TranscriptLowConfidenceCutPlanRequest,
    warnings: &mut Vec<String>,
) -> Vec<(u64, u64)> {
    if !request.enable_subtitle_long_silence_cut {
        return Vec::new();
    }

    let min_silence_ms = request
        .subtitle_long_silence_min_ms
        .unwrap_or(default_subtitle_long_silence_min_ms())
        .max(200);
    let rms_threshold_db = request
        .subtitle_long_silence_rms_threshold_db
        .unwrap_or(default_subtitle_long_silence_rms_threshold_db())
        .clamp(-90.0, -1.0);
    let pad_ms = request
        .subtitle_long_silence_pad_ms
        .unwrap_or(default_subtitle_long_silence_pad_ms())
        .min(5_000);

    let audio_map = get_audio_silence_map(
        global,
        AudioSilenceMapRequest {
            rms_threshold_db,
            min_silence_ms,
            pad_ms,
            detect_low_energy_repeats: false,
            repeat_similarity_threshold: default_repeat_similarity_threshold(),
            repeat_window_ms: default_repeat_window_ms(),
        },
    );
    if !audio_map.warnings.is_empty() {
        warnings.extend(
            audio_map
                .warnings
                .iter()
                .map(|w| format!("subtitle_long_silence: {w}")),
        );
    }

    let silence_ranges: Vec<(u64, u64)> = audio_map
        .cut_candidates
        .iter()
        .filter(|c| c.end_ms > c.start_ms)
        .map(|c| (c.start_ms, c.end_ms))
        .collect();
    if silence_ranges.is_empty() {
        return Vec::new();
    }

    let subtitle_ranges =
        collect_subtitle_coverage_ranges_ms(global, &request.track_indices, warnings);
    if subtitle_ranges.is_empty() {
        return Vec::new();
    }

    intersect_ranges_with_min_overlap(&silence_ranges, &subtitle_ranges, min_silence_ms)
}

fn subtitle_gap_mode_options() -> Vec<SubtitleGapModeOption> {
    vec![
        SubtitleGapModeOption {
            key: "conservative".to_string(),
            min_gap_ms: 350,
            edge_pad_ms: 120,
            description: "Fewer cuts, safer pacing for speech-heavy takes.".to_string(),
        },
        SubtitleGapModeOption {
            key: "balanced".to_string(),
            min_gap_ms: 250,
            edge_pad_ms: 100,
            description: "Default mode for normal talking-head cleanup.".to_string(),
        },
        SubtitleGapModeOption {
            key: "aggressive".to_string(),
            min_gap_ms: 150,
            edge_pad_ms: 60,
            description: "More cuts for faster pacing; may feel tighter.".to_string(),
        },
    ]
}

#[derive(Debug, Clone)]
struct TranscriptConfidenceSpan {
    start_ms: u64,
    end_ms: u64,
    uncertainty: f32,
    avg_logprob: Option<f32>,
    no_speech_prob: Option<f32>,
    silence_probability: Option<f32>,
}

fn value_to_f64(value: &Value) -> Option<f64> {
    if let Some(v) = value.as_f64() {
        return Some(v);
    }
    value.as_str().and_then(|s| s.parse::<f64>().ok())
}

fn read_number_field(obj: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(v) = obj.get(*key)
            && let Some(parsed) = value_to_f64(v)
        {
            return Some(parsed);
        }
    }
    None
}

fn parse_transcript_confidence_segments(
    segments: &[Value],
    out: &mut Vec<TranscriptConfidenceSpan>,
) {
    for seg in segments {
        let start_sec = read_number_field(seg, &["start_sec", "start"]).unwrap_or(-1.0);
        let end_sec = read_number_field(seg, &["end_sec", "end"]).unwrap_or(-1.0);
        if !start_sec.is_finite() || !end_sec.is_finite() || end_sec <= start_sec {
            continue;
        }

        let start_ms = (start_sec.max(0.0) * 1000.0).round() as u64;
        let end_ms = (end_sec.max(0.0) * 1000.0).round() as u64;
        if end_ms <= start_ms {
            continue;
        }

        let avg_logprob_opt = read_number_field(seg, &["avg_logprob"]).map(|v| v as f32);
        let no_speech_prob_opt =
            read_number_field(seg, &["no_speech_prob"]).map(|v| v.clamp(0.0, 1.0) as f32);
        let silence_prob_opt =
            read_number_field(seg, &["silence_probability"]).map(|v| v.clamp(0.0, 1.0) as f32);
        let avg_logprob = avg_logprob_opt.unwrap_or(-0.35);
        let no_speech_prob = no_speech_prob_opt.unwrap_or(0.0);
        let silence_prob = silence_prob_opt.unwrap_or(no_speech_prob);

        let uncertainty = transcript_uncertainty_score(avg_logprob, no_speech_prob, silence_prob);

        out.push(TranscriptConfidenceSpan {
            start_ms,
            end_ms,
            uncertainty,
            avg_logprob: avg_logprob_opt,
            no_speech_prob: no_speech_prob_opt,
            silence_probability: silence_prob_opt,
        });
    }
}

fn transcript_uncertainty_score(avg_logprob: f32, no_speech_prob: f32, silence_prob: f32) -> f32 {
    // Whisper avg_logprob is often around -0.2..-0.6 for usable speech and drops lower when ASR is shaky.
    let logprob_uncertainty = (((-avg_logprob) - 0.30) / 0.90).clamp(0.0, 1.0);
    let speech_absence_uncertainty = no_speech_prob.max(silence_prob).clamp(0.0, 1.0);
    // Keep logprob and no-speech balanced, but never suppress strong no-speech signals.
    (logprob_uncertainty * 0.50 + speech_absence_uncertainty * 0.50)
        .max(speech_absence_uncertainty * 0.95)
        .clamp(0.0, 1.0)
}

fn collect_transcript_confidence_spans(source: &Value) -> Vec<TranscriptConfidenceSpan> {
    let mut spans = Vec::new();

    if let Some(segments) = source.as_array() {
        parse_transcript_confidence_segments(segments, &mut spans);
    }
    if let Some(segments) = source.get("segments").and_then(Value::as_array) {
        parse_transcript_confidence_segments(segments, &mut spans);
    }
    if let Some(segments) = source
        .get("native")
        .and_then(|v| v.get("segments"))
        .and_then(Value::as_array)
    {
        parse_transcript_confidence_segments(segments, &mut spans);
    }
    if let Some(segments) = source
        .get("normalized")
        .and_then(|v| v.get("segments"))
        .and_then(Value::as_array)
    {
        parse_transcript_confidence_segments(segments, &mut spans);
    }

    let mut seen = HashSet::new();
    spans.retain(|s| seen.insert((s.start_ms, s.end_ms)));
    spans.sort_by_key(|s| (s.start_ms, s.end_ms));
    spans
}

fn overlap_weighted_uncertainty(
    spans: &[TranscriptConfidenceSpan],
    start_ms: u64,
    end_ms: u64,
) -> Option<f32> {
    if end_ms <= start_ms || spans.is_empty() {
        return None;
    }

    let mut weighted_sum = 0.0_f64;
    let mut weight_total = 0.0_f64;
    for span in spans {
        let overlap_start = span.start_ms.max(start_ms);
        let overlap_end = span.end_ms.min(end_ms);
        if overlap_end <= overlap_start {
            continue;
        }
        let w = (overlap_end - overlap_start) as f64;
        weighted_sum += (span.uncertainty as f64) * w;
        weight_total += w;
    }
    if weight_total <= f64::EPSILON {
        None
    } else {
        Some((weighted_sum / weight_total) as f32)
    }
}

fn mean_rms_in_range(timeline_rms: &[f32], step_ms: u64, start_ms: u64, end_ms: u64) -> f32 {
    if timeline_rms.is_empty() || step_ms == 0 || end_ms <= start_ms {
        return 0.0;
    }
    let start_bin = (start_ms / step_ms) as usize;
    let end_bin = end_ms.div_ceil(step_ms) as usize;
    if start_bin >= timeline_rms.len() {
        return 0.0;
    }
    let end_bin = end_bin.min(timeline_rms.len());
    if end_bin <= start_bin {
        return 0.0;
    }
    let slice = &timeline_rms[start_bin..end_bin];
    let sum: f32 = slice.iter().copied().sum();
    sum / (slice.len() as f32)
}

fn decode_clip_rms_windows(
    ffmpeg_bin: &str,
    file_path: &str,
    source_in: Duration,
    duration: Duration,
    sample_rate: u32,
    step_ms: u64,
) -> Result<Vec<f32>, String> {
    if duration.is_zero() || step_ms == 0 {
        return Ok(Vec::new());
    }

    let ss = source_in.as_secs_f64();
    let t = duration.as_secs_f64();
    let args = vec![
        "-hide_banner".to_string(),
        "-v".to_string(),
        "error".to_string(),
        "-i".to_string(),
        file_path.to_string(),
        "-ss".to_string(),
        format!("{ss:.6}"),
        "-t".to_string(),
        format!("{t:.6}"),
        "-vn".to_string(),
        "-ac".to_string(),
        "1".to_string(),
        "-ar".to_string(),
        sample_rate.to_string(),
        "-f".to_string(),
        "f32le".to_string(),
        "-".to_string(),
    ];

    let out = Command::new(ffmpeg_bin)
        .args(&args)
        .output()
        .map_err(|e| format!("ffmpeg execute failed for {}: {e}", file_path))?;
    if !out.status.success() {
        return Err(format!(
            "ffmpeg decode failed for {}: {}",
            file_path,
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let bytes = &out.stdout;
    if bytes.len() < 4 {
        return Ok(Vec::new());
    }

    let samples_per_window =
        (((sample_rate as u64).saturating_mul(step_ms)) / 1000).max(1) as usize;
    let total_samples = bytes.len() / 4;
    let window_count = total_samples.div_ceil(samples_per_window);
    let mut rms_windows = Vec::with_capacity(window_count);

    let mut sample_idx = 0usize;
    let mut byte_idx = 0usize;
    while sample_idx < total_samples && byte_idx + 4 <= bytes.len() {
        let mut sum_sq = 0.0_f64;
        let mut n = 0usize;
        while n < samples_per_window && sample_idx < total_samples && byte_idx + 4 <= bytes.len() {
            let v = f32::from_le_bytes([
                bytes[byte_idx],
                bytes[byte_idx + 1],
                bytes[byte_idx + 2],
                bytes[byte_idx + 3],
            ]);
            sum_sq += (v as f64) * (v as f64);
            n += 1;
            sample_idx += 1;
            byte_idx += 4;
        }
        if n > 0 {
            rms_windows.push((sum_sq / (n as f64)).sqrt() as f32);
        }
    }

    Ok(rms_windows)
}

pub fn timeline_revision(global: &GlobalState) -> String {
    let mut hasher = DefaultHasher::new();
    global.canvas_w.to_bits().hash(&mut hasher);
    global.canvas_h.to_bits().hash(&mut hasher);
    global.v1_clips.len().hash(&mut hasher);
    global.audio_tracks.len().hash(&mut hasher);
    global.video_tracks.len().hash(&mut hasher);
    global.subtitle_tracks.len().hash(&mut hasher);

    for clip in &global.v1_clips {
        clip.id.hash(&mut hasher);
        dur_ms(clip.start).hash(&mut hasher);
        dur_ms(clip.duration).hash(&mut hasher);
        dur_ms(clip.source_in).hash(&mut hasher);
        clip.file_path.hash(&mut hasher);
    }
    for track in &global.audio_tracks {
        track.name.hash(&mut hasher);
        for clip in &track.clips {
            clip.id.hash(&mut hasher);
            dur_ms(clip.start).hash(&mut hasher);
            dur_ms(clip.duration).hash(&mut hasher);
            dur_ms(clip.source_in).hash(&mut hasher);
            clip.file_path.hash(&mut hasher);
        }
    }
    for track in &global.video_tracks {
        track.name.hash(&mut hasher);
        for clip in &track.clips {
            clip.id.hash(&mut hasher);
            dur_ms(clip.start).hash(&mut hasher);
            dur_ms(clip.duration).hash(&mut hasher);
            dur_ms(clip.source_in).hash(&mut hasher);
            clip.file_path.hash(&mut hasher);
        }
    }
    for track in &global.subtitle_tracks {
        track.name.hash(&mut hasher);
        for clip in &track.clips {
            clip.id.hash(&mut hasher);
            dur_ms(clip.start).hash(&mut hasher);
            dur_ms(clip.duration).hash(&mut hasher);
            clip.text.hash(&mut hasher);
        }
    }

    let mut visibility_keys: Vec<_> = global.track_visibility.keys().cloned().collect();
    visibility_keys.sort();
    for key in visibility_keys {
        key.hash(&mut hasher);
        global
            .track_visibility
            .get(&key)
            .copied()
            .unwrap_or(true)
            .hash(&mut hasher);
    }

    let mut lock_keys: Vec<_> = global.track_lock.keys().cloned().collect();
    lock_keys.sort();
    for key in lock_keys {
        key.hash(&mut hasher);
        global
            .track_lock
            .get(&key)
            .copied()
            .unwrap_or(false)
            .hash(&mut hasher);
    }

    let mut mute_keys: Vec<_> = global.track_mute.keys().cloned().collect();
    mute_keys.sort();
    for key in mute_keys {
        key.hash(&mut hasher);
        global
            .track_mute
            .get(&key)
            .copied()
            .unwrap_or(false)
            .hash(&mut hasher);
    }

    format!("rev_{:x}", hasher.finish())
}

pub fn get_timeline_snapshot(
    global: &GlobalState,
    request: TimelineSnapshotRequest,
) -> TimelineSnapshotResponse {
    let v1 = TimelineTrackView {
        index: 0,
        name: "V1".to_string(),
        visible: track_visible(global, ApiTrackKind::V1, 0),
        locked: track_locked(global, ApiTrackKind::V1, 0),
        muted: false,
        clips: global.v1_clips.iter().map(clip_view).collect(),
    };

    let audio_tracks = global
        .audio_tracks
        .iter()
        .enumerate()
        .map(|(idx, track)| TimelineTrackView {
            index: idx,
            name: track.name.clone(),
            visible: track_visible(global, ApiTrackKind::Audio, idx),
            locked: track_locked(global, ApiTrackKind::Audio, idx),
            muted: track_muted(global, ApiTrackKind::Audio, idx),
            clips: track.clips.iter().map(clip_view).collect(),
        })
        .collect();

    let video_tracks = global
        .video_tracks
        .iter()
        .enumerate()
        .map(|(idx, track)| TimelineTrackView {
            index: idx,
            name: track.name.clone(),
            visible: track_visible(global, ApiTrackKind::Video, idx),
            locked: track_locked(global, ApiTrackKind::Video, idx),
            muted: track_muted(global, ApiTrackKind::Video, idx),
            clips: track.clips.iter().map(clip_view).collect(),
        })
        .collect();

    let subtitle_tracks = if request.include_subtitles {
        global
            .subtitle_tracks
            .iter()
            .enumerate()
            .map(|(idx, track)| TimelineSubtitleTrackView {
                index: idx,
                name: track.name.clone(),
                visible: track_visible(global, ApiTrackKind::Subtitle, idx),
                locked: track_locked(global, ApiTrackKind::Subtitle, idx),
                muted: track_muted(global, ApiTrackKind::Subtitle, idx),
                clips: track.clips.iter().map(subtitle_view).collect(),
            })
            .collect()
    } else {
        Vec::new()
    };

    let semantic_clips = global.semantic_clips.iter().map(semantic_view).collect();

    let mut link_map: std::collections::HashMap<u64, Vec<u64>> = std::collections::HashMap::new();
    for clip in global
        .v1_clips
        .iter()
        .chain(global.audio_tracks.iter().flat_map(|t| t.clips.iter()))
        .chain(global.video_tracks.iter().flat_map(|t| t.clips.iter()))
    {
        if let Some(group_id) = clip.link_group_id {
            link_map.entry(group_id).or_default().push(clip.id);
        }
    }
    let mut link_groups: Vec<TimelineLinkGroupView> = link_map
        .into_iter()
        .map(|(group_id, mut clip_ids)| {
            clip_ids.sort_unstable();
            TimelineLinkGroupView { group_id, clip_ids }
        })
        .collect();
    link_groups.sort_by_key(|g| g.group_id);

    TimelineSnapshotResponse {
        timeline_revision: timeline_revision(global),
        generated_at_unix_ms: now_unix_ms(),
        fps: global.preview_fps.value() as f32,
        duration_ms: dur_ms(global.timeline_total()),
        canvas: TimelineCanvas {
            width: global.canvas_w,
            height: global.canvas_h,
        },
        v1,
        audio_tracks,
        video_tracks,
        subtitle_tracks,
        semantic_clips,
        link_groups,
    }
}

pub fn get_audio_silence_map(
    global: &GlobalState,
    request: AudioSilenceMapRequest,
) -> AudioSilenceMapResponse {
    let timeline_end = global.timeline_total();
    let timeline_end_ms = duration_to_ms(timeline_end);
    let mut warnings = Vec::new();

    if timeline_end_ms == 0 {
        warnings.push("Timeline is empty.".to_string());
        return AudioSilenceMapResponse {
            timeline_revision: timeline_revision(global),
            generated_at_unix_ms: now_unix_ms(),
            analysis_source: "rms_waveform".to_string(),
            speech_segments: Vec::new(),
            silence_segments: Vec::new(),
            cut_candidates: Vec::new(),
            debug_rows: Vec::new(),
            warnings,
        };
    }

    let ffmpeg_bin = if global.ffmpeg_path.trim().is_empty() {
        "ffmpeg".to_string()
    } else {
        global.ffmpeg_path.clone()
    };

    // 20ms windows are a good balance: stable for speech VAD and still reasonably quick.
    let step_ms = 20_u64;
    let sample_rate = 16_000_u32;
    let total_bins = timeline_end_ms.div_ceil(step_ms) as usize;
    let mut timeline_rms = vec![0.0_f32; total_bins];

    let mut audio_coverage = Vec::new();
    for clip in global
        .audio_tracks
        .iter()
        .flat_map(|track| track.clips.iter())
    {
        if clip.duration.is_zero() {
            continue;
        }
        let clip_end = clip.start + clip.duration;
        audio_coverage.push((clip.start, clip_end));

        match decode_clip_rms_windows(
            &ffmpeg_bin,
            &clip.file_path,
            clip.source_in,
            clip.duration,
            sample_rate,
            step_ms,
        ) {
            Ok(rms_windows) => {
                let base_bin = (duration_to_ms(clip.start) / step_ms) as usize;
                for (i, rms) in rms_windows.iter().copied().enumerate() {
                    let idx = base_bin + i;
                    if idx >= timeline_rms.len() {
                        break;
                    }
                    if rms > timeline_rms[idx] {
                        timeline_rms[idx] = rms;
                    }
                }
            }
            Err(err) => warnings.push(err),
        }
    }

    let audio_coverage = normalize_segments(audio_coverage);
    if audio_coverage.is_empty() {
        warnings.push("No audio clips found on timeline audio tracks.".to_string());
        return AudioSilenceMapResponse {
            timeline_revision: timeline_revision(global),
            generated_at_unix_ms: now_unix_ms(),
            analysis_source: "rms_waveform".to_string(),
            speech_segments: Vec::new(),
            silence_segments: Vec::new(),
            cut_candidates: Vec::new(),
            debug_rows: Vec::new(),
            warnings,
        };
    }

    let threshold_rms = 10_f32
        .powf(request.rms_threshold_db / 20.0)
        .clamp(0.000001, 1.0);
    let mut speech_mask: Vec<bool> = timeline_rms.iter().map(|v| *v >= threshold_rms).collect();

    // Fill tiny silent holes so speech regions become stable (e.g. plosive gaps).
    let hole_fill_bins = (request.pad_ms.max(step_ms) / step_ms).max(1) as usize;
    let mut idx = 0usize;
    while idx < speech_mask.len() {
        if speech_mask[idx] {
            idx += 1;
            continue;
        }
        let gap_start = idx;
        while idx < speech_mask.len() && !speech_mask[idx] {
            idx += 1;
        }
        let gap_end = idx;
        let has_left = gap_start > 0 && speech_mask[gap_start - 1];
        let has_right = gap_end < speech_mask.len() && speech_mask[gap_end];
        if has_left && has_right && (gap_end - gap_start) <= hole_fill_bins {
            for bit in speech_mask.iter_mut().take(gap_end).skip(gap_start) {
                *bit = true;
            }
        }
    }

    let speech_segments_raw =
        normalize_segments(mask_to_segments(&speech_mask, step_ms, timeline_end_ms));
    let speech_segments_for_cut =
        inflate_segments(&speech_segments_raw, request.pad_ms, timeline_end);

    let min_silence = Duration::from_millis(request.min_silence_ms);
    let silence_segments: Vec<(Duration, Duration)> =
        subtract_segments(&audio_coverage, &speech_segments_for_cut)
            .into_iter()
            .filter(|(start, end)| *end > *start && (*end - *start) >= min_silence)
            .collect();

    let mut cut_candidates: Vec<SilenceCutCandidate> = silence_segments
        .iter()
        .map(|(start, end)| {
            let start_ms = duration_to_ms(*start);
            let end_ms = duration_to_ms(*end);
            let mean_rms = mean_rms_in_range(&timeline_rms, step_ms, start_ms, end_ms);
            SilenceCutCandidate {
                start_ms,
                end_ms,
                confidence: estimate_confidence_from_rms(mean_rms, threshold_rms),
                reason: format!(
                    "rms_silence(mean_rms={:.5}, threshold_rms={:.5}, threshold_db={:.1})",
                    mean_rms, threshold_rms, request.rms_threshold_db
                ),
            }
        })
        .collect();

    if request.detect_low_energy_repeats {
        warnings.push(
            "repeat_low_energy text-similarity heuristic is disabled; use LLM similar-sentence cut flow instead."
                .to_string(),
        );
    }

    cut_candidates.sort_by_key(|c| c.start_ms);

    let debug_rows: Vec<AudioSilenceDebugRow> = cut_candidates
        .iter()
        .map(|c| {
            let mean_rms = mean_rms_in_range(&timeline_rms, step_ms, c.start_ms, c.end_ms);
            AudioSilenceDebugRow {
                start_ms: c.start_ms,
                end_ms: c.end_ms,
                duration_ms: c.end_ms.saturating_sub(c.start_ms),
                mean_rms,
                threshold_rms,
                threshold_db: request.rms_threshold_db,
                confidence: c.confidence,
                reason: c.reason.clone(),
            }
        })
        .collect();

    for row in &debug_rows {
        info!(
            "[AudioSilence] candidate [{}, {}] dur={}ms mean_rms={:.6} threshold_rms={:.6} threshold_db={:.1} confidence={:.3} reason={}",
            row.start_ms,
            row.end_ms,
            row.duration_ms,
            row.mean_rms,
            row.threshold_rms,
            row.threshold_db,
            row.confidence,
            row.reason
        );
    }

    AudioSilenceMapResponse {
        timeline_revision: timeline_revision(global),
        generated_at_unix_ms: now_unix_ms(),
        analysis_source: if request.detect_low_energy_repeats {
            "rms_waveform+repeat_heuristic_disabled".to_string()
        } else {
            "rms_waveform".to_string()
        },
        speech_segments: speech_segments_raw
            .into_iter()
            .filter_map(|(s, e)| to_segment(s, e))
            .collect(),
        silence_segments: silence_segments
            .into_iter()
            .filter_map(|(s, e)| to_segment(s, e))
            .collect(),
        cut_candidates,
        debug_rows,
        warnings,
    }
}

pub fn build_audio_silence_cut_plan(
    global: &GlobalState,
    request: AudioSilenceCutPlanRequest,
) -> AudioSilenceCutPlanResponse {
    let mut audio_request = AudioSilenceMapRequest::default();
    if let Some(v) = request.rms_threshold_db {
        audio_request.rms_threshold_db = v;
    }
    if let Some(v) = request.min_silence_ms {
        audio_request.min_silence_ms = v;
    }
    if let Some(v) = request.pad_ms {
        audio_request.pad_ms = v;
    }
    if let Some(v) = request.detect_low_energy_repeats {
        audio_request.detect_low_energy_repeats = v;
    }
    if let Some(v) = request.repeat_similarity_threshold {
        audio_request.repeat_similarity_threshold = v;
    }
    if let Some(v) = request.repeat_window_ms {
        audio_request.repeat_window_ms = v;
    }

    let audio_map = get_audio_silence_map(global, audio_request);
    let mut warnings = audio_map.warnings.clone();
    let candidate_ranges: Vec<(u64, u64)> = audio_map
        .cut_candidates
        .iter()
        .filter(|c| c.end_ms > c.start_ms)
        .map(|c| (c.start_ms, c.end_ms))
        .collect();

    let (normalized_ranges, input_count) = canonicalize_ripple_delete_ranges(&candidate_ranges);
    if input_count > normalized_ranges.len() && !normalized_ranges.is_empty() {
        warnings.push(format!(
            "Merged {} cut ranges into {} non-overlapping ranges.",
            input_count,
            normalized_ranges.len()
        ));
    }

    let operations: Vec<TimelineEditOperation> = normalized_ranges
        .iter()
        .map(
            |(start_ms, end_ms)| TimelineEditOperation::RippleDeleteRange {
                start_ms: *start_ms,
                end_ms: *end_ms,
                mode: Some("all_tracks".to_string()),
            },
        )
        .collect();

    AudioSilenceCutPlanResponse {
        timeline_revision: audio_map.timeline_revision,
        generated_at_unix_ms: now_unix_ms(),
        analysis_source: "audio_silence_cut_plan".to_string(),
        candidate_ranges: normalized_ranges
            .iter()
            .map(|(start_ms, end_ms)| TimelineSegment {
                start_ms: *start_ms,
                end_ms: *end_ms,
            })
            .collect(),
        debug_rows: audio_map.debug_rows,
        operations,
        warnings,
    }
}

pub fn get_transcript_low_confidence_map(
    global: &GlobalState,
    request: TranscriptLowConfidenceMapRequest,
) -> TranscriptLowConfidenceMapResponse {
    let timeline_revision = timeline_revision(global);
    let generated_at_unix_ms = now_unix_ms();
    let mut warnings = Vec::new();
    let mut cut_candidates = Vec::new();
    let mut debug_rows = Vec::new();
    let threshold = request.uncertainty_threshold.clamp(0.0, 1.0);
    let min_duration_ms = request.min_duration_ms.max(40);
    let edge_pad_ms = request.edge_pad_ms.min(5_000);

    let Some(json_value) = request.transcript_confidence_json.as_ref() else {
        warnings.push(
            "Confidence metadata missing; continuing with semantic fallback if enabled."
                .to_string(),
        );
        return TranscriptLowConfidenceMapResponse {
            timeline_revision,
            generated_at_unix_ms,
            analysis_source: "transcript_low_confidence".to_string(),
            requires_transcript_confidence_json: false,
            transcript_json_status: "missing".to_string(),
            cut_candidates,
            debug_rows,
            warnings,
        };
    };

    let spans = collect_transcript_confidence_spans(json_value);
    if spans.is_empty() {
        warnings.push(
            "Confidence metadata is invalid; continuing with semantic fallback if enabled."
                .to_string(),
        );
        return TranscriptLowConfidenceMapResponse {
            timeline_revision,
            generated_at_unix_ms,
            analysis_source: "transcript_low_confidence".to_string(),
            requires_transcript_confidence_json: false,
            transcript_json_status: "invalid".to_string(),
            cut_candidates,
            debug_rows,
            warnings,
        };
    }

    let mut raw_ranges = Vec::new();
    for span in &spans {
        let padded_start = span.start_ms.saturating_sub(edge_pad_ms);
        let padded_end = span.end_ms.saturating_add(edge_pad_ms);
        if padded_end <= padded_start {
            continue;
        }
        if span.uncertainty >= threshold {
            raw_ranges.push((padded_start, padded_end));
            cut_candidates.push(SilenceCutCandidate {
                start_ms: padded_start,
                end_ms: padded_end,
                confidence: span.uncertainty.clamp(0.0, 1.0),
                reason: format!(
                    "transcript_low_confidence(uncertainty={:.3}, threshold={:.3})",
                    span.uncertainty, threshold
                ),
            });
        }
        debug_rows.push(TranscriptLowConfidenceDebugRow {
            start_ms: span.start_ms,
            end_ms: span.end_ms,
            duration_ms: span.end_ms.saturating_sub(span.start_ms),
            uncertainty: span.uncertainty,
            uncertainty_threshold: threshold,
            avg_logprob: span.avg_logprob,
            no_speech_prob: span.no_speech_prob,
            silence_probability: span.silence_probability,
            reason: if span.uncertainty >= threshold {
                "above_threshold".to_string()
            } else {
                "below_threshold".to_string()
            },
        });
    }

    let (merged_ranges, input_count) = canonicalize_ripple_delete_ranges(&raw_ranges);
    if input_count > merged_ranges.len() && !merged_ranges.is_empty() {
        warnings.push(format!(
            "Merged {} low-confidence ranges into {} non-overlapping ranges.",
            input_count,
            merged_ranges.len()
        ));
    }

    cut_candidates = merged_ranges
        .into_iter()
        .filter(|(start_ms, end_ms)| end_ms.saturating_sub(*start_ms) >= min_duration_ms)
        .map(|(start_ms, end_ms)| {
            let uncertainty = overlap_weighted_uncertainty(&spans, start_ms, end_ms).unwrap_or(0.0);
            SilenceCutCandidate {
                start_ms,
                end_ms,
                confidence: uncertainty.clamp(0.0, 1.0),
                reason: format!(
                    "transcript_low_confidence(uncertainty={:.3}, threshold={:.3}, min_duration_ms={})",
                    uncertainty, threshold, min_duration_ms
                ),
            }
        })
        .collect();

    if cut_candidates.is_empty() {
        let max_uncertainty = spans.iter().map(|s| s.uncertainty).fold(0.0_f32, f32::max);
        warnings.push(format!(
            "No low-confidence ranges passed threshold (max_uncertainty={:.3}, threshold={:.3}). Try lowering uncertainty_threshold.",
            max_uncertainty, threshold
        ));
    }

    TranscriptLowConfidenceMapResponse {
        timeline_revision,
        generated_at_unix_ms,
        analysis_source: "transcript_low_confidence".to_string(),
        requires_transcript_confidence_json: false,
        transcript_json_status: "ok".to_string(),
        cut_candidates,
        debug_rows,
        warnings,
    }
}

pub fn build_transcript_low_confidence_cut_plan(
    global: &GlobalState,
    request: TranscriptLowConfidenceCutPlanRequest,
) -> TranscriptLowConfidenceCutPlanResponse {
    let map_request = TranscriptLowConfidenceMapRequest {
        transcript_confidence_json: request.transcript_confidence_json.clone(),
        uncertainty_threshold: request
            .uncertainty_threshold
            .unwrap_or(default_transcript_uncertainty_threshold()),
        min_duration_ms: request
            .min_duration_ms
            .unwrap_or(default_transcript_min_duration_ms()),
        edge_pad_ms: request
            .edge_pad_ms
            .unwrap_or(default_transcript_edge_pad_ms()),
        enable_semantic_fallback: request.enable_semantic_fallback,
        fallback_window_ms: request
            .fallback_window_ms
            .unwrap_or(default_semantic_window_ms()),
        fallback_similarity_threshold: request
            .fallback_similarity_threshold
            .unwrap_or(default_semantic_similarity_threshold()),
        track_indices: request.track_indices.clone(),
    };
    let map = get_transcript_low_confidence_map(global, map_request);
    let mut warnings = map.warnings.clone();
    let transcript_json_status = map.transcript_json_status.clone();
    let mut candidate_ranges: Vec<(u64, u64)> = map
        .cut_candidates
        .iter()
        .filter(|c| c.end_ms > c.start_ms)
        .map(|c| (c.start_ms, c.end_ms))
        .collect();
    let fallback_used = false;

    if candidate_ranges.is_empty() && request.enable_semantic_fallback {
        warnings.push(
            "semantic_fallback disabled: deterministic text-similarity repeat detection has been removed; use LLM similar-sentence cut flow."
                .to_string(),
        );
    }

    let subtitle_covered_long_silence_ranges =
        collect_subtitle_covered_long_silence_ranges(global, &request, &mut warnings);
    if !subtitle_covered_long_silence_ranges.is_empty() {
        warnings.push(format!(
            "Added {} subtitle-covered long-silence cut range(s) (min_silence_ms={}).",
            subtitle_covered_long_silence_ranges.len(),
            request
                .subtitle_long_silence_min_ms
                .unwrap_or(default_subtitle_long_silence_min_ms())
        ));
        candidate_ranges.extend(subtitle_covered_long_silence_ranges);
    }

    let (normalized_ranges, input_count) = canonicalize_ripple_delete_ranges(&candidate_ranges);
    if input_count > normalized_ranges.len() && !normalized_ranges.is_empty() {
        warnings.push(format!(
            "Merged {} cut ranges into {} non-overlapping ranges.",
            input_count,
            normalized_ranges.len()
        ));
    }

    let operations: Vec<TimelineEditOperation> = normalized_ranges
        .iter()
        .map(
            |(start_ms, end_ms)| TimelineEditOperation::RippleDeleteRange {
                start_ms: *start_ms,
                end_ms: *end_ms,
                mode: Some("all_tracks".to_string()),
            },
        )
        .collect();

    TranscriptLowConfidenceCutPlanResponse {
        timeline_revision: map.timeline_revision,
        generated_at_unix_ms: now_unix_ms(),
        analysis_source: if fallback_used {
            "transcript_low_confidence_cut_plan+semantic_fallback".to_string()
        } else {
            "transcript_low_confidence_cut_plan".to_string()
        },
        fallback_used,
        transcript_json_status: transcript_json_status.clone(),
        transcript_json_prompt: None,
        candidate_ranges: normalized_ranges
            .iter()
            .map(|(start_ms, end_ms)| TimelineSegment {
                start_ms: *start_ms,
                end_ms: *end_ms,
            })
            .collect(),
        operations,
        warnings,
    }
}

pub fn get_subtitle_gap_map(
    global: &GlobalState,
    request: SubtitleGapMapRequest,
) -> SubtitleGapMapResponse {
    let timeline_revision = timeline_revision(global);
    let generated_at_unix_ms = now_unix_ms();
    let timeline_end = global.timeline_total();
    let mut warnings = Vec::new();
    let available_modes = subtitle_gap_mode_options();

    if timeline_end.is_zero() {
        warnings.push("Timeline is empty.".to_string());
        let (mode_used, base_min_gap_ms, base_edge_pad_ms) =
            resolve_subtitle_gap_mode(&request.mode);
        let min_gap_ms = request.min_gap_ms.unwrap_or(base_min_gap_ms);
        let edge_pad_ms = request.edge_pad_ms.unwrap_or(base_edge_pad_ms);
        return SubtitleGapMapResponse {
            timeline_revision,
            generated_at_unix_ms,
            analysis_source: "subtitle_gap_map".to_string(),
            mode_used: mode_used.to_string(),
            min_gap_ms,
            edge_pad_ms,
            available_modes,
            analysis_window: None,
            subtitle_segments: Vec::new(),
            gap_segments: Vec::new(),
            cut_candidates: Vec::new(),
            warnings,
        };
    }

    let (mode_used, base_min_gap_ms, base_edge_pad_ms) = resolve_subtitle_gap_mode(&request.mode);
    let min_gap_ms = request.min_gap_ms.unwrap_or(base_min_gap_ms).min(120_000);
    let edge_pad_ms = request.edge_pad_ms.unwrap_or(base_edge_pad_ms).min(10_000);

    let allowed_track_indices: Option<HashSet<usize>> = match &request.track_indices {
        Some(indices) if !indices.is_empty() => {
            let mut out = HashSet::new();
            for idx in indices {
                if *idx < global.subtitle_tracks.len() {
                    out.insert(*idx);
                } else {
                    warnings.push(format!(
                        "Requested subtitle track index {} is out of range (track_count={}).",
                        idx,
                        global.subtitle_tracks.len()
                    ));
                }
            }
            Some(out)
        }
        _ => None,
    };

    let mut subtitle_segments_raw: Vec<(Duration, Duration)> = Vec::new();
    for (track_idx, track) in global.subtitle_tracks.iter().enumerate() {
        if let Some(allowed) = &allowed_track_indices
            && !allowed.contains(&track_idx)
        {
            continue;
        }
        for clip in &track.clips {
            let start = clip.start;
            let end = clip.end();
            if end > start {
                subtitle_segments_raw.push((start, end));
            }
        }
    }

    let subtitle_segments = normalize_segments(subtitle_segments_raw);
    if subtitle_segments.is_empty() {
        warnings.push("No subtitle clips found for selected scope.".to_string());
        return SubtitleGapMapResponse {
            timeline_revision,
            generated_at_unix_ms,
            analysis_source: "subtitle_gap_map".to_string(),
            mode_used: mode_used.to_string(),
            min_gap_ms,
            edge_pad_ms,
            available_modes,
            analysis_window: None,
            subtitle_segments: Vec::new(),
            gap_segments: Vec::new(),
            cut_candidates: Vec::new(),
            warnings,
        };
    }

    let analysis_window = if request.include_head_tail {
        (Duration::ZERO, timeline_end)
    } else {
        let first_start = subtitle_segments[0].0;
        let last_end = subtitle_segments
            .last()
            .map(|(_, end)| *end)
            .unwrap_or(timeline_end);
        (first_start, last_end)
    };

    let analysis_base = vec![analysis_window];
    let raw_gap_segments = subtract_segments(&analysis_base, &subtitle_segments);
    let edge_pad = Duration::from_millis(edge_pad_ms);
    let min_gap = Duration::from_millis(min_gap_ms);

    let mut gap_segments: Vec<(Duration, Duration)> = Vec::new();
    let mut cut_candidates: Vec<SilenceCutCandidate> = Vec::new();
    for (raw_start, raw_end) in raw_gap_segments {
        if raw_end <= raw_start {
            continue;
        }
        let cut_start = raw_start + edge_pad;
        let cut_end = raw_end.saturating_sub(edge_pad);
        if cut_end <= cut_start {
            continue;
        }
        let len = cut_end - cut_start;
        if len < min_gap {
            continue;
        }

        let len_ms = duration_to_ms(len);
        let confidence = if len_ms >= 2000 {
            0.96
        } else if len_ms >= 1000 {
            0.89
        } else if len_ms >= 600 {
            0.81
        } else {
            0.72
        };

        gap_segments.push((cut_start, cut_end));
        cut_candidates.push(SilenceCutCandidate {
            start_ms: duration_to_ms(cut_start),
            end_ms: duration_to_ms(cut_end),
            confidence,
            reason: format!(
                "subtitle_gap(mode={}, min_gap_ms={}, edge_pad_ms={})",
                mode_used, min_gap_ms, edge_pad_ms
            ),
        });
    }

    SubtitleGapMapResponse {
        timeline_revision,
        generated_at_unix_ms,
        analysis_source: "subtitle_gap_map".to_string(),
        mode_used: mode_used.to_string(),
        min_gap_ms,
        edge_pad_ms,
        available_modes,
        analysis_window: to_segment(analysis_window.0, analysis_window.1),
        subtitle_segments: subtitle_segments
            .into_iter()
            .filter_map(|(start, end)| to_segment(start, end))
            .collect(),
        gap_segments: gap_segments
            .into_iter()
            .filter_map(|(start, end)| to_segment(start, end))
            .collect(),
        cut_candidates,
        warnings,
    }
}

pub fn build_subtitle_gap_cut_plan(
    global: &GlobalState,
    request: SubtitleGapCutPlanRequest,
) -> SubtitleGapCutPlanResponse {
    let subtitle_map = get_subtitle_gap_map(
        global,
        SubtitleGapMapRequest {
            mode: request.mode,
            min_gap_ms: request.min_gap_ms,
            edge_pad_ms: request.edge_pad_ms,
            include_head_tail: request.include_head_tail,
            track_indices: request.track_indices,
        },
    );

    let cut_strategy_used = normalize_subtitle_gap_cut_strategy(&request.cut_strategy).to_string();
    let mut warnings = subtitle_map.warnings.clone();
    let mut candidate_ranges: Vec<(u64, u64)> = subtitle_map
        .cut_candidates
        .iter()
        .filter(|c| c.end_ms > c.start_ms)
        .map(|c| (c.start_ms, c.end_ms))
        .collect();

    if cut_strategy_used == "subtitle_audio_aligned" {
        let mut audio_request = AudioSilenceMapRequest::default();
        if let Some(v) = request.audio_rms_threshold_db {
            audio_request.rms_threshold_db = v;
        }
        if let Some(v) = request.audio_min_silence_ms {
            audio_request.min_silence_ms = v;
        }
        if let Some(v) = request.audio_pad_ms {
            audio_request.pad_ms = v;
        }
        audio_request.detect_low_energy_repeats = false;

        let audio_map = get_audio_silence_map(global, audio_request);
        if !audio_map.warnings.is_empty() {
            warnings.extend(
                audio_map
                    .warnings
                    .iter()
                    .map(|w| format!("audio_alignment: {w}")),
            );
        }
        let silence_ranges: Vec<(u64, u64)> = audio_map
            .silence_segments
            .iter()
            .filter(|s| s.end_ms > s.start_ms)
            .map(|s| (s.start_ms, s.end_ms))
            .collect();
        candidate_ranges = intersect_ranges_with_min_overlap(
            &candidate_ranges,
            &silence_ranges,
            request.audio_align_min_overlap_ms,
        );
    }

    let (normalized_ranges, input_count) = canonicalize_ripple_delete_ranges(&candidate_ranges);
    if input_count > normalized_ranges.len() && !normalized_ranges.is_empty() {
        warnings.push(format!(
            "Merged {} cut ranges into {} non-overlapping ranges.",
            input_count,
            normalized_ranges.len()
        ));
    }
    if cut_strategy_used == "subtitle_audio_aligned" && candidate_ranges.is_empty() {
        warnings.push(
            "No subtitle gaps overlapped with audio silence after alignment. No cut operations generated."
                .to_string(),
        );
    }

    let operations: Vec<TimelineEditOperation> = normalized_ranges
        .iter()
        .map(
            |(start_ms, end_ms)| TimelineEditOperation::RippleDeleteRange {
                start_ms: *start_ms,
                end_ms: *end_ms,
                mode: Some("all_tracks".to_string()),
            },
        )
        .collect();

    SubtitleGapCutPlanResponse {
        timeline_revision: subtitle_map.timeline_revision,
        generated_at_unix_ms: now_unix_ms(),
        analysis_source: "subtitle_gap_cut_plan".to_string(),
        cut_strategy_used,
        mode_used: subtitle_map.mode_used,
        candidate_ranges: normalized_ranges
            .iter()
            .map(|(start_ms, end_ms)| TimelineSegment {
                start_ms: *start_ms,
                end_ms: *end_ms,
            })
            .collect(),
        operations,
        warnings,
    }
}

pub fn build_autonomous_edit_plan(
    global: &GlobalState,
    request: AutonomousEditPlanRequest,
) -> AutonomousEditPlanResponse {
    let timeline_revision = timeline_revision(global);
    let generated_at_unix_ms = now_unix_ms();
    let mut warnings = Vec::new();

    let goal = request.goal.unwrap_or_else(|| {
        "Build an adaptive edit plan to remove weak speech and long blank pauses.".to_string()
    });
    let aggressiveness_used =
        normalize_autonomous_aggressiveness(request.aggressiveness.as_deref(), &goal);
    let target_removed_ratio = autonomous_target_removed_ratio(&aggressiveness_used);

    let summary = AutonomousEditPlanSummary {
        timeline_duration_ms: dur_ms(global.timeline_total()),
        v1_clip_count: global.v1_clips.len(),
        audio_clip_count: global.audio_tracks.iter().map(|t| t.clips.len()).sum(),
        video_clip_count: global.video_tracks.iter().map(|t| t.clips.len()).sum(),
        subtitle_clip_count: global.subtitle_tracks.iter().map(|t| t.clips.len()).sum(),
        semantic_clip_count: global.semantic_clips.len(),
    };

    if summary.timeline_duration_ms == 0 {
        warnings.push("Timeline is empty. No autonomous edit plan can be generated.".to_string());
        return AutonomousEditPlanResponse {
            timeline_revision,
            generated_at_unix_ms,
            analysis_source: "autonomous_edit_plan".to_string(),
            goal,
            aggressiveness_used,
            decision_owner: "llm".to_string(),
            summary,
            target_removed_ratio,
            candidate_counts: HashMap::new(),
            observations: Vec::new(),
            warnings,
        };
    }

    let ratio_denominator = summary.timeline_duration_ms.max(1) as f32;
    let preview_limit = 24_usize;
    let mut observations = Vec::new();
    let mut candidate_counts = HashMap::new();

    let (audio_thresholds, audio_min_silence_ms, audio_pad_ms, repeat_similarity, repeat_window_ms) =
        match aggressiveness_used.as_str() {
            "conservative" => (vec![-14.0], 320_u64, 70_u64, 0.86_f32, 9_000_u64),
            "aggressive" => (vec![-14.0], 200_u64, 110_u64, 0.78_f32, 7_000_u64),
            _ => (vec![-14.0], 260_u64, 85_u64, 0.82_f32, 8_000_u64),
        };

    for threshold in audio_thresholds {
        let args = AudioSilenceCutPlanRequest {
            rms_threshold_db: Some(threshold),
            min_silence_ms: Some(audio_min_silence_ms),
            pad_ms: Some(audio_pad_ms),
            detect_low_energy_repeats: Some(true),
            repeat_similarity_threshold: Some(repeat_similarity),
            repeat_window_ms: Some(repeat_window_ms),
        };
        let plan = build_audio_silence_cut_plan(global, args.clone());
        let estimated_removed_ms = sum_segment_duration_ms(&plan.candidate_ranges);
        let estimated_removed_ratio =
            (estimated_removed_ms as f32 / ratio_denominator).clamp(0.0, 1.0);
        observations.push(AutonomousEditPlanObservation {
            category: "audio_silence".to_string(),
            tool: "anica.timeline/build_audio_silence_cut_plan".to_string(),
            arguments: serde_json::to_value(&args).unwrap_or(json!({})),
            candidate_count: plan.candidate_ranges.len(),
            estimated_removed_ms,
            estimated_removed_ratio,
            operations_preview: plan.operations.into_iter().take(preview_limit).collect(),
            warnings: plan.warnings,
        });
    }

    let base_audio_threshold_db: f32 = match aggressiveness_used.as_str() {
        "conservative" => -36.0_f32,
        "aggressive" => -40.0_f32,
        _ => -38.0_f32,
    };

    let transcript_profiles: Vec<(f32, u64, u64, f32, u64, u64, u64)> =
        match aggressiveness_used.as_str() {
            "conservative" => vec![
                (0.62, 380, 50, 0.93, 30_000, 3_000, 40),
                (0.58, 340, 60, 0.92, 30_000, 2_800, 50),
            ],
            "aggressive" => vec![
                (0.50, 280, 70, 0.90, 30_000, 2_500, 60),
                (0.45, 240, 85, 0.88, 26_000, 2_200, 75),
                (0.40, 200, 95, 0.86, 24_000, 2_000, 90),
            ],
            _ => vec![
                (0.58, 320, 60, 0.92, 30_000, 2_800, 50),
                (0.52, 280, 70, 0.90, 30_000, 2_500, 60),
                (0.46, 240, 85, 0.88, 26_000, 2_200, 75),
            ],
        };

    for (
        uncertainty,
        min_duration_ms,
        edge_pad_ms,
        fallback_similarity,
        fallback_window_ms,
        subtitle_long_silence_min_ms,
        subtitle_pad_ms,
    ) in transcript_profiles
    {
        let args = TranscriptLowConfidenceCutPlanRequest {
            transcript_confidence_json: None,
            uncertainty_threshold: Some(uncertainty),
            min_duration_ms: Some(min_duration_ms),
            edge_pad_ms: Some(edge_pad_ms),
            enable_semantic_fallback: true,
            fallback_window_ms: Some(fallback_window_ms),
            fallback_similarity_threshold: Some(fallback_similarity),
            enable_subtitle_long_silence_cut: true,
            subtitle_long_silence_min_ms: Some(subtitle_long_silence_min_ms),
            subtitle_long_silence_rms_threshold_db: Some(
                (base_audio_threshold_db - 6.0_f32).clamp(-55.0_f32, -24.0_f32),
            ),
            subtitle_long_silence_pad_ms: Some(subtitle_pad_ms),
            track_indices: None,
        };
        let plan = build_transcript_low_confidence_cut_plan(global, args.clone());
        let estimated_removed_ms = sum_segment_duration_ms(&plan.candidate_ranges);
        let estimated_removed_ratio =
            (estimated_removed_ms as f32 / ratio_denominator).clamp(0.0, 1.0);
        observations.push(AutonomousEditPlanObservation {
            category: "low_confidence_or_semantic".to_string(),
            tool: "anica.timeline/build_transcript_low_confidence_cut_plan".to_string(),
            arguments: serde_json::to_value(&args).unwrap_or(json!({})),
            candidate_count: plan.candidate_ranges.len(),
            estimated_removed_ms,
            estimated_removed_ratio,
            operations_preview: plan.operations.into_iter().take(preview_limit).collect(),
            warnings: plan.warnings,
        });
    }

    for mode in ["conservative", "balanced", "aggressive"] {
        let args = SubtitleGapCutPlanRequest {
            mode: mode.to_string(),
            min_gap_ms: None,
            edge_pad_ms: None,
            include_head_tail: true,
            track_indices: None,
            cut_strategy: "subtitle_audio_aligned".to_string(),
            audio_align_min_overlap_ms: if mode == "conservative" {
                120
            } else if mode == "aggressive" {
                60
            } else {
                80
            },
            audio_rms_threshold_db: Some(base_audio_threshold_db),
            audio_min_silence_ms: Some(audio_min_silence_ms),
            audio_pad_ms: Some(audio_pad_ms),
        };
        let plan = build_subtitle_gap_cut_plan(global, args.clone());
        let estimated_removed_ms = sum_segment_duration_ms(&plan.candidate_ranges);
        let estimated_removed_ratio =
            (estimated_removed_ms as f32 / ratio_denominator).clamp(0.0, 1.0);
        observations.push(AutonomousEditPlanObservation {
            category: "subtitle_gap_aligned".to_string(),
            tool: "anica.timeline/build_subtitle_gap_cut_plan".to_string(),
            arguments: serde_json::to_value(&args).unwrap_or(json!({})),
            candidate_count: plan.candidate_ranges.len(),
            estimated_removed_ms,
            estimated_removed_ratio,
            operations_preview: plan.operations.into_iter().take(preview_limit).collect(),
            warnings: plan.warnings,
        });
    }

    for obs in &observations {
        let entry = candidate_counts
            .entry(obs.category.clone())
            .or_insert(0_usize);
        *entry = (*entry).saturating_add(obs.candidate_count);
    }

    if observations.iter().all(|obs| obs.candidate_count == 0) {
        warnings.push(
            "All autonomous observation candidates are empty. LLM may retry with wider thresholds or different strategy."
                .to_string(),
        );
    }

    AutonomousEditPlanResponse {
        timeline_revision,
        generated_at_unix_ms,
        analysis_source: "autonomous_edit_plan".to_string(),
        goal,
        aggressiveness_used,
        decision_owner: "llm".to_string(),
        summary,
        target_removed_ratio,
        candidate_counts,
        observations,
        warnings,
    }
}

pub fn get_subtitle_semantic_repeats(
    global: &GlobalState,
    request: SubtitleSemanticRepeatsRequest,
) -> SubtitleSemanticRepeatsResponse {
    SubtitleSemanticRepeatsResponse {
        timeline_revision: timeline_revision(global),
        generated_at_unix_ms: now_unix_ms(),
        analysis_source: "subtitle_semantic_repeat_disabled".to_string(),
        window_ms: request.window_ms,
        similarity_threshold: request.similarity_threshold,
        repeat_groups: Vec::new(),
        cut_candidates: Vec::new(),
        warnings: vec![
            "Deterministic text-similarity semantic repeat detection is disabled.".to_string(),
            "Use LLM similar-sentence cut flow instead.".to_string(),
        ],
    }
}

fn canonicalize_ripple_delete_ranges(ranges: &[(u64, u64)]) -> (Vec<(u64, u64)>, usize) {
    if ranges.is_empty() {
        return (Vec::new(), 0);
    }

    let mut sorted = ranges.to_vec();
    sorted.sort_by_key(|(start, end)| (*start, *end));
    let input_count = sorted.len();

    let mut merged: Vec<(u64, u64)> = Vec::with_capacity(sorted.len());
    for (start, end) in sorted {
        if let Some((_, last_end)) = merged.last_mut()
            && start <= *last_end
        {
            if end > *last_end {
                *last_end = end;
            }
            continue;
        }
        merged.push((start, end));
    }

    (merged, input_count)
}

pub fn apply_edit_plan(
    global: &mut GlobalState,
    request: &TimelineEditPlanRequest,
) -> TimelineEditApplyResponse {
    let validation = validate_edit_plan(global, request);
    if !validation.ok {
        return TimelineEditApplyResponse {
            ok: false,
            before_revision: validation.before_revision,
            after_revision: None,
            applied_ops: 0,
            errors: validation.errors,
            warnings: validation.warnings,
        };
    }

    let before_revision = timeline_revision(global);
    let mut applied_ops = 0usize;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let atomic = request.atomic.unwrap_or(true);
    let all_ops_are_ripple_delete = request.operations.iter().all(|op| {
        matches!(
            op,
            TimelineEditOperation::RippleDeleteRange {
                start_ms: _,
                end_ms: _,
                mode: _
            }
        )
    });

    // Execute all-ripple plans right-to-left so each range keeps the original snapshot coordinates.
    // Running left-to-right causes timeline drift and can leave unintended gaps.
    if all_ops_are_ripple_delete {
        let raw_ripple_ranges: Vec<(u64, u64)> = request
            .operations
            .iter()
            .filter_map(|op| match op {
                TimelineEditOperation::RippleDeleteRange {
                    start_ms, end_ms, ..
                } => Some((*start_ms, *end_ms)),
                _ => None,
            })
            .collect();
        let (merged_ranges, input_ripple_count) =
            canonicalize_ripple_delete_ranges(&raw_ripple_ranges);
        if input_ripple_count > merged_ranges.len() && !merged_ranges.is_empty() {
            warnings.push(format!(
                "Merged {} ripple_delete_range ops into {} non-overlapping ranges before apply.",
                input_ripple_count,
                merged_ranges.len()
            ));
        }
        if merged_ranges.len() > 1 {
            warnings.push(format!(
                "Applied ripple_delete_range right-to-left across {} ranges to avoid coordinate drift.",
                merged_ranges.len()
            ));
        }
        for (range_idx, (start_ms, end_ms)) in merged_ranges.iter().enumerate().rev() {
            let changed = global.ripple_delete_time_range_all_tracks(
                Duration::from_millis(*start_ms),
                Duration::from_millis(*end_ms),
            );
            if changed {
                applied_ops = applied_ops.saturating_add(1);
            } else {
                warnings.push(format!(
                    "ripple_range#{}: operation caused no timeline change.",
                    range_idx.saturating_add(1)
                ));
            }
        }
        return TimelineEditApplyResponse {
            ok: true,
            before_revision,
            after_revision: Some(timeline_revision(global)),
            applied_ops,
            errors,
            warnings,
        };
    }

    for (op_index, op) in request.operations.iter().enumerate() {
        let op_label = format!("op#{}", op_index.saturating_add(1));

        let result: Result<bool, TimelineEditError> = (|| -> Result<bool, TimelineEditError> {
            match op {
                TimelineEditOperation::AddTrack { track_type, name } => {
                    let kind = parse_track_kind(track_type).ok_or_else(|| {
                        TimelineEditError::InvalidTrackTypeForOperation {
                            track_type: track_type.clone(),
                            operation: "add_track",
                        }
                    })?;
                    match kind {
                        ApiTrackKind::V1 => Err(TimelineEditError::V1CannotBeAddedOrRemoved),
                        ApiTrackKind::Audio => {
                            let next_audio_number = global
                                .audio_tracks
                                .iter()
                                .filter_map(|track| {
                                    track
                                        .name
                                        .trim()
                                        .strip_prefix('A')
                                        .and_then(|raw| raw.parse::<usize>().ok())
                                })
                                .max()
                                .unwrap_or(0)
                                .saturating_add(1);
                            let name = name
                                .clone()
                                .filter(|v| !v.trim().is_empty())
                                .unwrap_or_else(|| format!("A{next_audio_number}"));
                            global.save_for_undo();
                            global.audio_tracks.push(AudioTrack::new(name));
                            Ok(true)
                        }
                        ApiTrackKind::Video => {
                            let next_video_number = global
                                .video_tracks
                                .iter()
                                .filter_map(|track| {
                                    track
                                        .name
                                        .trim()
                                        .strip_prefix('V')
                                        .and_then(|raw| raw.parse::<usize>().ok())
                                })
                                .max()
                                .unwrap_or(1)
                                .saturating_add(1);
                            let name = name
                                .clone()
                                .filter(|v| !v.trim().is_empty())
                                .unwrap_or_else(|| format!("V{next_video_number}"));
                            global.save_for_undo();
                            global.video_tracks.push(VideoTrack::new(name));
                            Ok(true)
                        }
                        ApiTrackKind::Subtitle => {
                            let next_subtitle_number = global
                                .subtitle_tracks
                                .iter()
                                .filter_map(|track| {
                                    track
                                        .name
                                        .trim()
                                        .strip_prefix('S')
                                        .and_then(|raw| raw.parse::<usize>().ok())
                                })
                                .max()
                                .unwrap_or(0)
                                .saturating_add(1);
                            let name = name
                                .clone()
                                .filter(|v| !v.trim().is_empty())
                                .unwrap_or_else(|| format!("S{next_subtitle_number}"));
                            global.save_for_undo();
                            global.subtitle_tracks.push(SubtitleTrack::new(name));
                            Ok(true)
                        }
                    }
                }
                TimelineEditOperation::AddAudioTrack { name } => {
                    let next_audio_number = global
                        .audio_tracks
                        .iter()
                        .filter_map(|track| {
                            track
                                .name
                                .trim()
                                .strip_prefix('A')
                                .and_then(|raw| raw.parse::<usize>().ok())
                        })
                        .max()
                        .unwrap_or(0)
                        .saturating_add(1);
                    let name = name
                        .clone()
                        .filter(|v| !v.trim().is_empty())
                        .unwrap_or_else(|| format!("A{next_audio_number}"));
                    global.save_for_undo();
                    global.audio_tracks.push(AudioTrack::new(name));
                    Ok(true)
                }
                TimelineEditOperation::AddVideoTrack { name } => {
                    let next_video_number = global
                        .video_tracks
                        .iter()
                        .filter_map(|track| {
                            track
                                .name
                                .trim()
                                .strip_prefix('V')
                                .and_then(|raw| raw.parse::<usize>().ok())
                        })
                        .max()
                        .unwrap_or(1)
                        .saturating_add(1);
                    let name = name
                        .clone()
                        .filter(|v| !v.trim().is_empty())
                        .unwrap_or_else(|| format!("V{next_video_number}"));
                    global.save_for_undo();
                    global.video_tracks.push(VideoTrack::new(name));
                    Ok(true)
                }
                TimelineEditOperation::AddSubtitleTrack { name } => {
                    let next_subtitle_number = global
                        .subtitle_tracks
                        .iter()
                        .filter_map(|track| {
                            track
                                .name
                                .trim()
                                .strip_prefix('S')
                                .and_then(|raw| raw.parse::<usize>().ok())
                        })
                        .max()
                        .unwrap_or(0)
                        .saturating_add(1);
                    let name = name
                        .clone()
                        .filter(|v| !v.trim().is_empty())
                        .unwrap_or_else(|| format!("S{next_subtitle_number}"));
                    global.save_for_undo();
                    global.subtitle_tracks.push(SubtitleTrack::new(name));
                    Ok(true)
                }
                TimelineEditOperation::RemoveTrack { track_type, index } => {
                    let kind = parse_track_kind(track_type).ok_or_else(|| {
                        TimelineEditError::InvalidTrackTypeForOperation {
                            track_type: track_type.clone(),
                            operation: "remove_track",
                        }
                    })?;
                    if matches!(kind, ApiTrackKind::V1) {
                        Err(TimelineEditError::V1CannotBeAddedOrRemoved)
                    } else {
                        let changed = match kind {
                            ApiTrackKind::Audio => global.delete_audio_track(*index),
                            ApiTrackKind::Video => global.delete_video_track(*index),
                            ApiTrackKind::Subtitle => global.delete_subtitle_track(*index),
                            ApiTrackKind::V1 => false,
                        };
                        if changed {
                            remap_track_state_after_remove(global, kind, *index);
                        }
                        Ok(changed)
                    }
                }
                TimelineEditOperation::RemoveAudioTrack { index } => {
                    let changed = global.delete_audio_track(*index);
                    if changed {
                        remap_track_state_after_remove(global, ApiTrackKind::Audio, *index);
                    }
                    Ok(changed)
                }
                TimelineEditOperation::RemoveVideoTrack { index } => {
                    let changed = global.delete_video_track(*index);
                    if changed {
                        remap_track_state_after_remove(global, ApiTrackKind::Video, *index);
                    }
                    Ok(changed)
                }
                TimelineEditOperation::RemoveSubtitleTrack { index } => {
                    let changed = global.delete_subtitle_track(*index);
                    if changed {
                        remap_track_state_after_remove(global, ApiTrackKind::Subtitle, *index);
                    }
                    Ok(changed)
                }
                TimelineEditOperation::SetTrackVisibility {
                    track_type,
                    index,
                    visible,
                } => {
                    let kind = parse_track_kind(track_type).ok_or_else(|| {
                        TimelineEditError::InvalidTrackTypeForOperation {
                            track_type: track_type.clone(),
                            operation: "set_track_visibility",
                        }
                    })?;
                    let current = track_visible(global, kind, *index);
                    if current == *visible {
                        Ok(false)
                    } else {
                        global.save_for_undo();
                        set_track_visible(global, kind, *index, *visible);
                        Ok(true)
                    }
                }
                TimelineEditOperation::SetTrackLock {
                    track_type,
                    index,
                    locked,
                } => {
                    let kind = parse_track_kind(track_type).ok_or_else(|| {
                        TimelineEditError::InvalidTrackTypeForOperation {
                            track_type: track_type.clone(),
                            operation: "set_track_lock",
                        }
                    })?;
                    let current = track_locked(global, kind, *index);
                    if current == *locked {
                        Ok(false)
                    } else {
                        global.save_for_undo();
                        set_track_locked(global, kind, *index, *locked);
                        Ok(true)
                    }
                }
                TimelineEditOperation::SetTrackMute {
                    track_type,
                    index,
                    muted,
                } => {
                    let kind = parse_track_kind(track_type).ok_or_else(|| {
                        TimelineEditError::InvalidTrackTypeForOperation {
                            track_type: track_type.clone(),
                            operation: "set_track_mute",
                        }
                    })?;
                    if matches!(kind, ApiTrackKind::V1) {
                        Err(TimelineEditError::V1DoesNotSupportMute)
                    } else {
                        let current = track_muted(global, kind, *index);
                        if current == *muted {
                            Ok(false)
                        } else {
                            global.save_for_undo();
                            set_track_muted(global, kind, *index, *muted);
                            Ok(true)
                        }
                    }
                }
                TimelineEditOperation::InsertClip {
                    track_type,
                    track_index,
                    media_pool_item_id,
                    path,
                    start_ms,
                    source_in_ms,
                    source_out_ms,
                    duration_ms,
                } => apply_insert_clip_common(
                    global,
                    &op_label,
                    &mut warnings,
                    track_type,
                    *track_index,
                    *media_pool_item_id,
                    path.as_deref(),
                    *start_ms,
                    *source_in_ms,
                    *source_out_ms,
                    *duration_ms,
                ),
                TimelineEditOperation::InsertFromMediaPool {
                    track_type,
                    track_index,
                    media_pool_item_id,
                    start_ms,
                    source_in_ms,
                    source_out_ms,
                    duration_ms,
                } => apply_insert_clip_common(
                    global,
                    &op_label,
                    &mut warnings,
                    track_type,
                    *track_index,
                    Some(*media_pool_item_id),
                    None,
                    *start_ms,
                    *source_in_ms,
                    *source_out_ms,
                    *duration_ms,
                ),
                TimelineEditOperation::SetSourceInOut {
                    clip_id,
                    source_in_ms,
                    source_out_ms,
                    duration_ms,
                } => {
                    let clip = find_clip_ref(global, *clip_id)
                        .ok_or(TimelineEditError::ClipNotFound { clip_id: *clip_id })?;
                    let previous_source_in = clip.source_in;
                    let previous_duration = clip.duration;
                    let media_duration = clip.media_duration;

                    let (next_source_in, next_duration) = resolve_insert_window(
                        media_duration,
                        Some(*source_in_ms),
                        *source_out_ms,
                        *duration_ms,
                        &op_label,
                        &mut warnings,
                    )?;

                    if previous_source_in == next_source_in && previous_duration == next_duration {
                        Ok(false)
                    } else {
                        global.save_for_undo();
                        update_inserted_clip_source_window(
                            global,
                            *clip_id,
                            next_source_in,
                            next_duration,
                            media_duration,
                        );
                        Ok(true)
                    }
                }
                TimelineEditOperation::DeleteClip { clip_id, ripple } => {
                    find_clip_location(global, *clip_id)
                        .ok_or(TimelineEditError::ClipNotFound { clip_id: *clip_id })?;
                    if ripple.unwrap_or(false) {
                        let clip = find_clip_ref(global, *clip_id)
                            .ok_or(TimelineEditError::ClipNotFound { clip_id: *clip_id })?;
                        let start = clip.start;
                        let end = clip.end();
                        Ok(global.ripple_delete_time_range_all_tracks(start, end))
                    } else {
                        let mut clip_ids: HashSet<u64> = HashSet::new();
                        clip_ids.insert(*clip_id);
                        global.save_for_undo();
                        let changed = remove_clip_ids_without_ripple(global, &clip_ids);
                        Ok(changed)
                    }
                }
                TimelineEditOperation::DeleteTrackClips {
                    track_type,
                    track_index,
                    with_linked,
                } => {
                    let kind = parse_track_kind(track_type).ok_or_else(|| {
                        TimelineEditError::InvalidTrackTypeForOperation {
                            track_type: track_type.clone(),
                            operation: "delete_track_clips",
                        }
                    })?;
                    let resolved_index =
                        resolve_indexed_track_target(track_type, kind, *track_index)?;
                    if !track_exists(global, kind, resolved_index) {
                        return Err(TimelineEditError::DeleteTrackClipsTargetNotFound {
                            track_type: track_type.clone(),
                            track_index: resolved_index,
                        });
                    }

                    match kind {
                        ApiTrackKind::Subtitle => {
                            let subtitle_ids: HashSet<u64> =
                                subtitle_ids_on_track(global, resolved_index)
                                    .into_iter()
                                    .collect();
                            if subtitle_ids.is_empty() {
                                Ok(false)
                            } else {
                                global.save_for_undo();
                                Ok(remove_subtitle_ids(global, &subtitle_ids))
                            }
                        }
                        ApiTrackKind::V1 | ApiTrackKind::Audio | ApiTrackKind::Video => {
                            let mut clip_ids: HashSet<u64> =
                                clip_ids_on_track(global, kind, resolved_index)
                                    .into_iter()
                                    .collect();
                            if clip_ids.is_empty() {
                                Ok(false)
                            } else {
                                if *with_linked {
                                    clip_ids = expand_clip_ids_by_link_group(global, &clip_ids);
                                }
                                global.save_for_undo();
                                Ok(remove_clip_ids_without_ripple(global, &clip_ids))
                            }
                        }
                    }
                }
                TimelineEditOperation::RippleDeleteRange {
                    start_ms, end_ms, ..
                } => Ok(global.ripple_delete_time_range_all_tracks(
                    Duration::from_millis(*start_ms),
                    Duration::from_millis(*end_ms),
                )),
                TimelineEditOperation::TrimClip {
                    clip_id,
                    new_start_ms,
                    new_duration_ms,
                } => {
                    let location = find_clip_location(global, *clip_id)
                        .ok_or(TimelineEditError::ClipNotFound { clip_id: *clip_id })?;
                    let new_start = Duration::from_millis(*new_start_ms);
                    let requested_duration = Duration::from_millis(*new_duration_ms);
                    global.save_for_undo();

                    let mut changed = false;
                    let mut apply_trim = |clip: &mut Clip| {
                        let old_start = clip.start;
                        let old_duration = clip.duration;
                        let is_image = {
                            let p = clip.file_path.to_ascii_lowercase();
                            p.ends_with(".jpg")
                                || p.ends_with(".jpeg")
                                || p.ends_with(".png")
                                || p.ends_with(".webp")
                                || p.ends_with(".bmp")
                        };
                        let max_duration = if is_image {
                            Duration::from_secs(24 * 3600)
                        } else {
                            clip.media_duration.saturating_sub(clip.source_in)
                        };
                        let bounded_duration =
                            requested_duration.clamp(Duration::from_millis(100), max_duration);
                        clip.start = new_start;
                        clip.duration = bounded_duration;
                        if old_start != clip.start || old_duration != clip.duration {
                            changed = true;
                        }
                    };

                    match location {
                        ClipLocation::V1 => {
                            if let Some(clip) =
                                global.v1_clips.iter_mut().find(|clip| clip.id == *clip_id)
                            {
                                apply_trim(clip);
                            }
                            global.v1_clips.sort_by_key(|clip| clip.start);
                        }
                        ClipLocation::Audio(track_index) => {
                            if let Some(track) = global.audio_tracks.get_mut(track_index) {
                                if let Some(clip) =
                                    track.clips.iter_mut().find(|clip| clip.id == *clip_id)
                                {
                                    apply_trim(clip);
                                }
                                track.clips.sort_by_key(|clip| clip.start);
                            }
                        }
                        ClipLocation::Video(track_index) => {
                            if let Some(track) = global.video_tracks.get_mut(track_index) {
                                if let Some(clip) =
                                    track.clips.iter_mut().find(|clip| clip.id == *clip_id)
                                {
                                    apply_trim(clip);
                                }
                                track.clips.sort_by_key(|clip| clip.start);
                            }
                        }
                    }
                    Ok(changed)
                }
                TimelineEditOperation::MoveClip {
                    clip_id,
                    new_start_ms,
                } => {
                    let location = find_clip_location(global, *clip_id)
                        .ok_or(TimelineEditError::ClipNotFound { clip_id: *clip_id })?;
                    let old_clip = find_clip_ref(global, *clip_id)
                        .ok_or(TimelineEditError::ClipNotFound { clip_id: *clip_id })?;
                    let old_start = old_clip.start;
                    let new_start = Duration::from_millis(*new_start_ms);
                    if old_start == new_start {
                        Ok(false)
                    } else {
                        global.save_for_undo();
                        match location {
                            ClipLocation::V1 => global.move_v1_clip_free(*clip_id, new_start),
                            ClipLocation::Audio(track_index) => {
                                global.move_audio_clip_free(track_index, *clip_id, new_start)
                            }
                            ClipLocation::Video(track_index) => {
                                global.move_video_clip_free(track_index, *clip_id, new_start)
                            }
                        }
                        let changed = find_clip_ref(global, *clip_id)
                            .map(|clip| clip.start != old_start)
                            .unwrap_or(false);
                        Ok(changed)
                    }
                }
                TimelineEditOperation::SplitClip { clip_id, at_ms } => {
                    let location = find_clip_location(global, *clip_id)
                        .ok_or(TimelineEditError::ClipNotFound { clip_id: *clip_id })?;
                    let at = Duration::from_millis(*at_ms);
                    global.playhead = at;
                    let split_result = match location {
                        ClipLocation::V1 => global.razor_v1_at_playhead(),
                        ClipLocation::Audio(track_index) => {
                            global.razor_audio_at_playhead(track_index)
                        }
                        ClipLocation::Video(track_index) => {
                            global.razor_video_at_playhead(track_index)
                        }
                    };
                    split_result
                        .map(|_| true)
                        .map_err(|err| TimelineEditError::SplitClipFailed {
                            message: err.to_string(),
                        })
                }
                TimelineEditOperation::ShiftSubtitlesRange {
                    start_ms,
                    end_ms,
                    delta_ms,
                } => {
                    if *delta_ms == 0 {
                        Ok(false)
                    } else {
                        let start = Duration::from_millis(*start_ms);
                        let end = Duration::from_millis(*end_ms);
                        let has_targets = global.subtitle_tracks.iter().any(|track| {
                            track
                                .clips
                                .iter()
                                .any(|clip| clip.start < end && clip.end() > start)
                        });
                        if !has_targets {
                            Ok(false)
                        } else {
                            global.save_for_undo();
                            let shift_ms = delta_ms.unsigned_abs();
                            let shift = Duration::from_millis(shift_ms);
                            for track in &mut global.subtitle_tracks {
                                for clip in &mut track.clips {
                                    if clip.start < end && clip.end() > start {
                                        if *delta_ms >= 0 {
                                            clip.start = clip.start.saturating_add(shift);
                                        } else {
                                            clip.start = clip.start.saturating_sub(shift);
                                        }
                                    }
                                }
                                track.clips.sort_by_key(|clip| clip.start);
                            }
                            Ok(true)
                        }
                    }
                }
                TimelineEditOperation::GenerateSubtitles {
                    track_index,
                    entries,
                } => {
                    let track_index = track_index.unwrap_or(0);
                    if !track_exists(global, ApiTrackKind::Subtitle, track_index) {
                        Err(TimelineEditError::SubtitleTrackNotFound { track_index })
                    } else {
                        let mut inserted = 0usize;
                        for entry in entries {
                            let inserted_id = global.add_subtitle_clip(
                                track_index,
                                ms_to_duration(entry.start_ms),
                                ms_to_duration(entry.duration_ms.max(1)),
                                entry.text.clone(),
                            );
                            if let Some(inserted_id) = inserted_id {
                                inserted = inserted.saturating_add(1);
                                if let Some((track_idx, clip_idx)) =
                                    find_subtitle_location(global, inserted_id)
                                    && let Some(clip) = global
                                        .subtitle_tracks
                                        .get_mut(track_idx)
                                        .and_then(|track| track.clips.get_mut(clip_idx))
                                {
                                    if let Some(pos_x) = entry.pos_x {
                                        clip.pos_x = pos_x;
                                    }
                                    if let Some(pos_y) = entry.pos_y {
                                        clip.pos_y = pos_y;
                                    }
                                    if let Some(font_size) = entry.font_size {
                                        clip.font_size = font_size.max(1.0);
                                    }
                                    if let Some(color_rgba) = entry.color_rgba {
                                        clip.color_rgba = color_rgba;
                                    }
                                }
                            }
                        }
                        Ok(inserted > 0)
                    }
                }
                TimelineEditOperation::MoveSubtitle {
                    clip_id,
                    new_start_ms,
                    to_track_index,
                } => {
                    let (src_track_index, src_clip_index) =
                        find_subtitle_location(global, *clip_id)
                            .ok_or(TimelineEditError::SubtitleClipNotFound { clip_id: *clip_id })?;
                    let target_track_index = to_track_index.unwrap_or(src_track_index);
                    if !track_exists(global, ApiTrackKind::Subtitle, target_track_index) {
                        Err(TimelineEditError::SubtitleTargetTrackNotFound {
                            track_index: target_track_index,
                        })
                    } else {
                        let current_start =
                            global.subtitle_tracks[src_track_index].clips[src_clip_index].start;
                        let new_start = ms_to_duration(*new_start_ms);
                        if current_start == new_start && src_track_index == target_track_index {
                            Ok(false)
                        } else {
                            global.save_for_undo();
                            let mut clip = global.subtitle_tracks[src_track_index]
                                .clips
                                .remove(src_clip_index);
                            clip.start = new_start;
                            global.subtitle_tracks[target_track_index].clips.push(clip);
                            global.subtitle_tracks[target_track_index]
                                .clips
                                .sort_by_key(|c| c.start);
                            if src_track_index != target_track_index {
                                global.subtitle_tracks[src_track_index]
                                    .clips
                                    .sort_by_key(|c| c.start);
                            }
                            Ok(true)
                        }
                    }
                }
                TimelineEditOperation::BatchUpdateSubtitles { updates } => {
                    let mut did_change = false;
                    let mut did_save = false;
                    for patch in updates {
                        let Some((src_track_index, src_clip_index)) =
                            find_subtitle_location(global, patch.clip_id)
                        else {
                            continue;
                        };
                        let target_track_index = patch.track_index.unwrap_or(src_track_index);
                        if !track_exists(global, ApiTrackKind::Subtitle, target_track_index) {
                            continue;
                        }
                        if !did_save {
                            global.save_for_undo();
                            did_save = true;
                        }

                        let mut clip = global.subtitle_tracks[src_track_index]
                            .clips
                            .remove(src_clip_index);
                        if let Some(text) = &patch.text {
                            clip.text = text.clone();
                        }
                        if let Some(start_ms) = patch.start_ms {
                            clip.start = ms_to_duration(start_ms);
                        }
                        if let Some(duration_ms) = patch.duration_ms {
                            clip.duration = ms_to_duration(duration_ms.max(1));
                        }
                        if let Some(pos_x) = patch.pos_x {
                            clip.pos_x = pos_x;
                        }
                        if let Some(pos_y) = patch.pos_y {
                            clip.pos_y = pos_y;
                        }
                        if let Some(font_size) = patch.font_size {
                            clip.font_size = font_size.max(1.0);
                        }
                        if let Some(color_rgba) = patch.color_rgba {
                            clip.color_rgba = color_rgba;
                        }

                        global.subtitle_tracks[target_track_index].clips.push(clip);
                        global.subtitle_tracks[target_track_index]
                            .clips
                            .sort_by_key(|c| c.start);
                        if src_track_index != target_track_index {
                            global.subtitle_tracks[src_track_index]
                                .clips
                                .sort_by_key(|c| c.start);
                        }
                        did_change = true;
                    }
                    Ok(did_change)
                }
                TimelineEditOperation::DeleteSubtitleRange {
                    start_ms,
                    end_ms,
                    track_indices,
                } => {
                    let start = ms_to_duration(*start_ms);
                    let end = ms_to_duration(*end_ms);
                    let target_tracks: Vec<usize> = if let Some(indexes) = track_indices {
                        indexes.clone()
                    } else {
                        (0..global.subtitle_tracks.len()).collect()
                    };
                    let has_target = target_tracks.iter().any(|track_index| {
                        global
                            .subtitle_tracks
                            .get(*track_index)
                            .map(|track| {
                                track
                                    .clips
                                    .iter()
                                    .any(|clip| clip.start < end && clip.end() > start)
                            })
                            .unwrap_or(false)
                    });
                    if !has_target {
                        Ok(false)
                    } else {
                        global.save_for_undo();
                        for track_index in &target_tracks {
                            if let Some(track) = global.subtitle_tracks.get_mut(*track_index) {
                                track
                                    .clips
                                    .retain(|clip| !(clip.start < end && clip.end() > start));
                            }
                        }
                        Ok(true)
                    }
                }
                TimelineEditOperation::ApplyEffect {
                    clip_id,
                    effect,
                    params,
                }
                | TimelineEditOperation::UpdateEffectParams {
                    clip_id,
                    effect,
                    params,
                } => {
                    apply_or_update_effect(global, *clip_id, effect, params.as_ref())?;
                    Ok(true)
                }
                TimelineEditOperation::RemoveEffect { clip_id, effect } => {
                    remove_effect(global, *clip_id, effect)?;
                    Ok(true)
                }
                TimelineEditOperation::ApplyTransition {
                    clip_id,
                    transition,
                } => {
                    apply_transition(global, *clip_id, transition)?;
                    Ok(true)
                }
                TimelineEditOperation::UpdateTransition {
                    clip_id,
                    transition,
                    params,
                } => {
                    update_transition(global, *clip_id, transition, params.as_ref())?;
                    Ok(true)
                }
                TimelineEditOperation::RemoveTransition {
                    clip_id,
                    transition,
                } => {
                    remove_transition(global, *clip_id, transition.as_deref())?;
                    Ok(true)
                }
                // Apply semantic layer marker insertion (B-roll planning annotations).
                TimelineEditOperation::InsertSemanticClip {
                    start_ms,
                    duration_ms,
                    semantic_type,
                    label,
                    prompt_schema,
                } => {
                    let start = Duration::from_millis(*start_ms);
                    let duration = Duration::from_millis(*duration_ms);
                    let semantic_type_text = semantic_type
                        .clone()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(default_semantic_type_text);
                    let label_text = label
                        .clone()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| "semantic".to_string());
                    // Insert ACP semantic markers as non-destructive timeline annotations.
                    global.insert_semantic_clip(
                        start,
                        duration,
                        semantic_type_text,
                        label_text,
                        prompt_schema.clone(),
                    );
                    Ok(true)
                }
            }
        })();

        match result {
            Ok(true) => {
                applied_ops = applied_ops.saturating_add(1);
            }
            Ok(false) => {
                warnings.push(format!("{op_label}: operation caused no timeline change."));
            }
            Err(err) => {
                errors.push(format!("{op_label}: {err}"));
                if atomic {
                    break;
                }
            }
        }
    }

    if !errors.is_empty() {
        return TimelineEditApplyResponse {
            ok: false,
            before_revision,
            after_revision: None,
            applied_ops,
            errors,
            warnings,
        };
    }

    TimelineEditApplyResponse {
        ok: true,
        before_revision,
        after_revision: Some(timeline_revision(global)),
        applied_ops,
        errors,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::transcript_uncertainty_score;

    #[test]
    fn transcript_uncertainty_stays_low_for_clean_speech() {
        let score = transcript_uncertainty_score(-0.22, 0.03, 0.03);
        assert!(score < 0.20, "expected low uncertainty, got {score}");
    }

    #[test]
    fn transcript_uncertainty_rises_with_no_speech_signal() {
        let score = transcript_uncertainty_score(-0.30, 0.52, 0.48);
        assert!(score > 0.45, "expected elevated uncertainty, got {score}");
    }

    #[test]
    fn transcript_uncertainty_rises_with_poor_logprob() {
        let score = transcript_uncertainty_score(-1.10, 0.08, 0.08);
        assert!(score > 0.40, "expected elevated uncertainty, got {score}");
    }
}
