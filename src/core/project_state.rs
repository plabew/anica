// =========================================
// =========================================
// src/core/project_state.rs
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};

use super::effects::LayerColorBlurEffects;
use super::export::is_supported_media_path;
use super::global_state::{
    AudioTrack, Clip, GlobalState, LayerEffectClip, LocalMaskLayer, MediaPoolItem, ScalarKeyframe,
    SemanticClip, SlideDirection, SubtitleClip, SubtitleGroupTransform, SubtitleTrack, UndoManager,
    VideoEffect, VideoTrack,
};
use super::thumbnail;

const PROJECT_VERSION: u32 = 1;
const RECOVERY_VERSION: u32 = 1;
const DEFAULT_SNAPSHOT_KEEP: usize = 5;
const SUBTITLE_DEFAULT_POS_X: f32 = -0.30;
const SUBTITLE_DEFAULT_POS_Y: f32 = 0.35;
const SUBTITLE_DEFAULT_SIZE: f32 = 48.0;
const SUBTITLE_DEFAULT_COLOR: [u8; 4] = [255, 255, 255, 255];
const MEDIA_POOL_PREVIEW_MAX_DIM: u32 = 320;
const MAX_EMBEDDED_PREVIEW_BYTES: usize = 350_000;
const DEFAULT_SEMANTIC_TYPE: &str = "content_support";

fn default_semantic_type() -> String {
    DEFAULT_SEMANTIC_TYPE.to_string()
}

fn is_default_semantic_type(value: &String) -> bool {
    value == DEFAULT_SEMANTIC_TYPE
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryDraft {
    pub version: u32,
    pub updated_at_ms: u128,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<PathBuf>,
    pub project_name: String,
    pub project: ProjectState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub version: u32,
    pub meta: ProjectMetadata,
    pub canvas: CanvasState,
    pub timeline: TimelineState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_pool: Vec<MediaPoolItemState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_effects: Option<LayerEffectsState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui: Option<UiState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadata {
    pub name: String,
    pub created_at: i64,
    pub last_opened: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasState {
    pub width: f32,
    pub height: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineState {
    pub v1: Vec<ClipState>,
    pub audio_tracks: Vec<TrackState>,
    pub video_tracks: Vec<TrackState>,
    pub subtitle_tracks: Vec<SubtitleTrackState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_clips: Vec<SemanticClipState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtitle_groups: Vec<SubtitleGroupState>,
    pub next_clip_id: u64,
    pub next_subtitle_group_id: u64,
    pub playhead_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticClipState {
    pub id: u64,
    pub start_us: u64,
    pub duration_us: u64,
    #[serde(
        default = "default_semantic_type",
        skip_serializing_if = "is_default_semantic_type"
    )]
    pub semantic_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_schema: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected_clip_ids: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected_subtitle_ids: Vec<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playhead_us: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LayerEffectsState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contrast: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saturation: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blur_sigma: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<LayerEffectClipState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_clip_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerEffectClipState {
    pub id: u64,
    pub start_us: u64,
    pub duration_us: u64,
    pub track_index: usize,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub fade_in_us: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub fade_out_us: u64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub brightness_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub brightness_keys: Vec<PosXKeyframeState>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub contrast_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contrast_keys: Vec<PosXKeyframeState>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub saturation_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saturation_keys: Vec<PosXKeyframeState>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub blur_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blur_sigma: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blur_keys: Vec<PosXKeyframeState>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub motionloom_enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub motionloom_script: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackState {
    pub name: String,
    pub clips: Vec<ClipState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipState {
    pub id: u64,
    pub label: String,
    pub path: String,
    pub start_us: u64,
    pub duration_us: u64,
    pub source_in_us: u64,
    pub media_duration_us: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_group_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_gain_db: Option<f32>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub dissolve_trim_in_us: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub dissolve_trim_out_us: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effects: Option<ClipEffectsState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pos_x_keys: Vec<PosXKeyframeState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pos_y_keys: Vec<PosXKeyframeState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scale_keys: Vec<PosXKeyframeState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rotation_keys: Vec<PosXKeyframeState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub brightness_keys: Vec<PosXKeyframeState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contrast_keys: Vec<PosXKeyframeState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saturation_keys: Vec<PosXKeyframeState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opacity_keys: Vec<PosXKeyframeState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blur_keys: Vec<PosXKeyframeState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClipEffectsState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brightness: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contrast: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saturation: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blur_sigma: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fade_in: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dissolve_in: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dissolve_out: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slide_in: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slide_out: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slide_in_dir: Option<SlideDirection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slide_out_dir: Option<SlideDirection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom_in: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom_out: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom_amount: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shock_in: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shock_out: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shock_amount: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_x: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_y: Option<f32>,
    #[serde(default, alias = "tint", skip_serializing_if = "Option::is_none")]
    pub hsla_overlay: Option<HslaOverlayState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HslaOverlayState {
    pub hue: f32,
    pub saturation: f32,
    pub lightness: f32,
    pub alpha: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PosXKeyframeState {
    pub time_us: u64,
    pub pos_x: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleTrackState {
    pub name: String,
    pub clips: Vec<SubtitleClipState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleClipState {
    pub id: u64,
    pub text: String,
    pub start_us: u64,
    pub duration_us: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_x: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_y: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_rgba: Option<[u8; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleGroupState {
    pub id: u64,
    pub offset_x: f32,
    pub offset_y: f32,
    pub scale: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaPoolItemState {
    pub path: String,
    pub name: String,
    pub duration_us: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_jpeg_base64: Option<String>,
}

impl ProjectState {
    pub fn from_global(gs: &GlobalState) -> Self {
        Self::from_global_with_embed_previews(gs, None)
    }

    fn from_global_with_embed_previews(
        gs: &GlobalState,
        embed_preview_cache_root: Option<&Path>,
    ) -> Self {
        let name = match gs.active_source_name.as_str() {
            "" | "No Source Loaded" => "Untitled".to_string(),
            other => other.to_string(),
        };
        let now = now_epoch_secs();
        let timeline = TimelineState {
            v1: gs.v1_clips.iter().map(clip_to_state).collect(),
            audio_tracks: gs.audio_tracks.iter().map(audio_track_to_state).collect(),
            video_tracks: gs.video_tracks.iter().map(video_track_to_state).collect(),
            subtitle_tracks: gs
                .subtitle_tracks
                .iter()
                .map(subtitle_track_to_state)
                .collect(),
            semantic_clips: gs
                .semantic_clips
                .iter()
                .map(semantic_clip_to_state)
                .collect(),
            subtitle_groups: gs
                .subtitle_groups
                .iter()
                .map(|(id, group)| SubtitleGroupState {
                    id: *id,
                    offset_x: group.offset_x,
                    offset_y: group.offset_y,
                    scale: group.scale,
                })
                .collect(),
            next_clip_id: gs.next_clip_id(),
            next_subtitle_group_id: gs.next_subtitle_group_id,
            playhead_us: dur_to_us(gs.playhead),
        };
        let media_pool = gs
            .media_pool
            .iter()
            .filter(|item| is_supported_media_path(&item.path))
            .map(|item| media_pool_item_to_state(item, embed_preview_cache_root))
            .collect();

        ProjectState {
            version: PROJECT_VERSION,
            meta: ProjectMetadata {
                name,
                created_at: now,
                last_opened: now,
            },
            canvas: CanvasState {
                width: gs.canvas_w,
                height: gs.canvas_h,
                fps: None,
            },
            timeline,
            media_pool,
            layer_effects: layer_effects_from_global(gs),
            ui: Some(UiState {
                selected_clip_ids: gs.selected_clip_ids.clone(),
                selected_subtitle_ids: gs.selected_subtitle_ids.clone(),
                playhead_us: Some(dur_to_us(gs.playhead)),
            }),
        }
    }

    pub fn apply_to(&self, gs: &mut GlobalState) {
        gs.set_canvas_size(self.canvas.width, self.canvas.height);
        gs.v1_clips = self.timeline.v1.iter().map(state_to_clip).collect();
        gs.audio_tracks = self
            .timeline
            .audio_tracks
            .iter()
            .map(state_to_audio_track)
            .collect();
        gs.video_tracks = self
            .timeline
            .video_tracks
            .iter()
            .map(state_to_video_track)
            .collect();
        gs.subtitle_tracks = self
            .timeline
            .subtitle_tracks
            .iter()
            .map(state_to_subtitle_track)
            .collect();
        gs.semantic_clips = self
            .timeline
            .semantic_clips
            .iter()
            .map(state_to_semantic_clip)
            .collect();
        // Normalize semantic prompt schemas so legacy projects gain the full default schema.
        gs.normalize_all_semantic_prompt_schemas();
        gs.semantic_mark_start = None;
        let restored_media_pool = if self.media_pool.is_empty() {
            fallback_media_pool_from_timeline(gs)
        } else {
            self.media_pool
                .iter()
                .map(state_to_media_pool_item)
                .collect()
        };
        let mut filtered_media_pool: Vec<MediaPoolItem> = restored_media_pool
            .into_iter()
            .filter(|item| is_supported_media_path(&item.path))
            .collect();
        if filtered_media_pool.is_empty() {
            filtered_media_pool = fallback_media_pool_from_timeline(gs);
        }
        gs.media_pool = filtered_media_pool;
        gs.subtitle_groups = self
            .timeline
            .subtitle_groups
            .iter()
            .map(|group| {
                (
                    group.id,
                    SubtitleGroupTransform {
                        offset_x: group.offset_x,
                        offset_y: group.offset_y,
                        scale: group.scale,
                    },
                )
            })
            .collect();
        let layer_state =
            layer_effects_to_runtime(self.layer_effects.as_ref(), gs.video_tracks.len());
        gs.set_layer_color_blur_effects(layer_state.0);
        gs.layer_effect_clips = layer_state.1;
        gs.selected_layer_effect_clip_id = layer_state.2;

        let next_layer_clip_id = gs
            .layer_effect_clips
            .iter()
            .map(|clip| clip.id)
            .max()
            .unwrap_or(0)
            .max(
                gs.semantic_clips
                    .iter()
                    .map(|clip| clip.id)
                    .max()
                    .unwrap_or(0),
            )
            .saturating_add(1);
        gs.set_next_clip_id(self.timeline.next_clip_id.max(next_layer_clip_id).max(1));
        gs.next_subtitle_group_id = self.timeline.next_subtitle_group_id.max(1);
        gs.playhead = us_to_dur(self.timeline.playhead_us);
        sync_active_source_with_media_pool(gs);

        if let Some(ui) = &self.ui {
            gs.selected_clip_ids = ui.selected_clip_ids.clone();
            gs.selected_subtitle_ids = ui.selected_subtitle_ids.clone();
            gs.selected_clip_id = gs.selected_clip_ids.last().copied();
            gs.selected_subtitle_id = gs.selected_subtitle_ids.last().copied();
        } else {
            gs.selected_clip_id = None;
            gs.selected_subtitle_id = None;
            gs.selected_clip_ids.clear();
            gs.selected_subtitle_ids.clear();
        }

        gs.undo_manager = UndoManager::new();
        gs.export_in_progress = false;
        gs.export_last_error = None;
        gs.export_last_out_path = None;
        gs.export_progress_ratio = 0.0;
        gs.export_progress_rendered = std::time::Duration::ZERO;
        gs.export_progress_total = std::time::Duration::ZERO;
        gs.export_eta = None;
    }
}

/// Collect all unique media file paths referenced by this project.
pub fn collect_media_paths(project: &ProjectState) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut paths = Vec::new();
    let mut push = |p: &str| {
        if !p.trim().is_empty() && seen.insert(p.to_string()) {
            paths.push(p.to_string());
        }
    };
    // Media pool items.
    for item in &project.media_pool {
        push(&item.path);
    }
    // V1 clips.
    for clip in &project.timeline.v1 {
        push(&clip.path);
    }
    // Audio track clips.
    for track in &project.timeline.audio_tracks {
        for clip in &track.clips {
            push(&clip.path);
        }
    }
    // Video track clips.
    for track in &project.timeline.video_tracks {
        for clip in &track.clips {
            push(&clip.path);
        }
    }
    paths
}

/// Return only the paths that do not exist on disk.
pub fn find_missing_media(project: &ProjectState) -> Vec<String> {
    collect_media_paths(project)
        .into_iter()
        .filter(|p| !Path::new(p).exists())
        .collect()
}

/// Replace all occurrences of old paths with new paths throughout the project.
pub fn remap_project_paths(project: &mut ProjectState, mapping: &[(String, String)]) {
    if mapping.is_empty() {
        return;
    }
    let map: std::collections::HashMap<&str, &str> = mapping
        .iter()
        .map(|(o, n)| (o.as_str(), n.as_str()))
        .collect();
    let remap = |p: &mut String| {
        if let Some(new) = map.get(p.as_str()) {
            *p = new.to_string();
        }
    };
    // Remap media pool paths.
    for item in &mut project.media_pool {
        remap(&mut item.path);
    }
    // Remap V1 clip paths.
    for clip in &mut project.timeline.v1 {
        remap(&mut clip.path);
    }
    // Remap audio track clip paths.
    for track in &mut project.timeline.audio_tracks {
        for clip in &mut track.clips {
            remap(&mut clip.path);
        }
    }
    // Remap video track clip paths.
    for track in &mut project.timeline.video_tracks {
        for clip in &mut track.clips {
            remap(&mut clip.path);
        }
    }
}

pub fn save_project_to_path(gs: &GlobalState, path: impl AsRef<Path>) -> anyhow::Result<()> {
    let cache_root = gs.cache_root_dir();
    let state = ProjectState::from_global_with_embed_previews(gs, Some(cache_root.as_path()));
    let data = serde_json::to_vec(&state)?;
    if let Some(parent) = path.as_ref().parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, data)?;
    Ok(())
}

pub fn load_project_from_path(path: impl AsRef<Path>) -> anyhow::Result<ProjectState> {
    let data = fs::read(path)?;
    let state: ProjectState = serde_json::from_slice(&data)?;
    Ok(state)
}

pub fn save_project_snapshot(
    gs: &GlobalState,
    autosave_dir: impl AsRef<Path>,
    keep: Option<usize>,
) -> anyhow::Result<()> {
    let autosave_dir = autosave_dir.as_ref();
    fs::create_dir_all(autosave_dir)?;
    let ts = now_epoch_millis();
    let filename = format!("snapshot_{ts}.anica.json");
    let path = autosave_dir.join(filename);
    save_project_to_path(gs, &path)?;
    prune_snapshots(autosave_dir, keep.unwrap_or(DEFAULT_SNAPSHOT_KEEP))?;
    Ok(())
}

pub fn default_project_dir() -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    {
        let home = env::var_os("USERPROFILE").map(PathBuf::from).or_else(|| {
            let drive = env::var_os("HOMEDRIVE")?;
            let path = env::var_os("HOMEPATH")?;
            let mut p = PathBuf::from(drive);
            p.push(path);
            Some(p)
        });

        if let Some(home) = home {
            let docs = home.join("Documents");
            if docs.exists() {
                return docs.join("AnicaProjects");
            }

            // Common Windows setup where Documents lives under OneDrive.
            let onedrive_docs = home.join("OneDrive").join("Documents");
            if onedrive_docs.exists() {
                return onedrive_docs.join("AnicaProjects");
            }

            return home.join("AnicaProjects");
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
            let docs = home.join("Documents");
            if docs.exists() {
                return docs.join("AnicaProjects");
            }
            return home.join("AnicaProjects");
        }
    }

    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("AnicaProjects")
}

pub fn autosave_dir(project_dir: impl AsRef<Path>) -> std::path::PathBuf {
    project_dir.as_ref().join(".autosave")
}

pub fn recovery_dir() -> std::path::PathBuf {
    default_project_dir().join(".recovery")
}

pub fn recovery_file_path(project_path: Option<&Path>) -> std::path::PathBuf {
    recovery_dir().join(format!(
        "{}.recovery.anica.json",
        stable_recovery_key(project_path)
    ))
}

pub fn save_recovery_draft(gs: &GlobalState) -> anyhow::Result<PathBuf> {
    let root = recovery_dir();
    fs::create_dir_all(&root)?;

    // Store recovery drafts in a central root so startup recovery works across platforms.
    let path = recovery_file_path(gs.project_file_path.as_deref());
    let tmp_path = path.with_extension("tmp");
    let draft = RecoveryDraft {
        version: RECOVERY_VERSION,
        updated_at_ms: now_epoch_millis(),
        project_path: gs.project_file_path.clone(),
        project_name: recovery_project_name(gs.project_file_path.as_deref()),
        project: ProjectState::from_global(gs),
    };
    let data = serde_json::to_vec(&draft)?;
    fs::write(&tmp_path, data)?;
    if path.exists() {
        let _ = fs::remove_file(&path);
    }
    fs::rename(&tmp_path, &path)?;
    Ok(path)
}

pub fn clear_recovery_draft(project_path: Option<&Path>) -> anyhow::Result<()> {
    let path = recovery_file_path(project_path);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn clear_all_recovery_drafts() -> anyhow::Result<()> {
    let root = recovery_dir();
    if !root.exists() {
        return Ok(());
    }

    // Graceful shutdowns should clear all recovery drafts so next launch stays clean.
    for entry in fs::read_dir(root)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.ends_with(".recovery.anica.json") {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

pub fn load_recovery_draft(path: impl AsRef<Path>) -> anyhow::Result<RecoveryDraft> {
    let data = fs::read(path)?;
    Ok(serde_json::from_slice(&data)?)
}

pub fn latest_recovery_draft() -> anyhow::Result<Option<(PathBuf, RecoveryDraft)>> {
    let root = recovery_dir();
    if !root.exists() {
        return Ok(None);
    }

    let mut best: Option<(PathBuf, RecoveryDraft)> = None;
    for entry in fs::read_dir(&root)? {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".recovery.anica.json"))
        {
            continue;
        }

        let draft = match load_recovery_draft(&path) {
            Ok(draft) => draft,
            Err(err) => {
                eprintln!(
                    "[Project] Failed to parse recovery draft {}: {err}",
                    path.display()
                );
                continue;
            }
        };

        // Skip stale drafts when the saved project is already newer than the recovery copy.
        if saved_project_is_newer_than_draft(&draft) {
            let _ = fs::remove_file(&path);
            continue;
        }

        let replace = best
            .as_ref()
            .map(|(_, current)| current.updated_at_ms < draft.updated_at_ms)
            .unwrap_or(true);
        if replace {
            best = Some((path, draft));
        }
    }

    Ok(best)
}

fn clip_to_state(clip: &Clip) -> ClipState {
    ClipState {
        id: clip.id,
        label: clip.label.clone(),
        path: clip.file_path.clone(),
        start_us: dur_to_us(clip.start),
        duration_us: dur_to_us(clip.duration),
        source_in_us: dur_to_us(clip.source_in),
        media_duration_us: dur_to_us(clip.media_duration),
        link_group_id: clip.link_group_id,
        audio_gain_db: if clip.audio_gain_db.abs() > 0.0001 {
            Some(clip.audio_gain_db)
        } else {
            None
        },
        dissolve_trim_in_us: dur_to_us(clip.dissolve_trim_in),
        dissolve_trim_out_us: dur_to_us(clip.dissolve_trim_out),
        effects: clip_effects_from_clip(clip),
        pos_x_keys: clip
            .pos_x_keyframes
            .iter()
            .map(|k| PosXKeyframeState {
                time_us: dur_to_us(k.time),
                pos_x: k.value,
            })
            .collect(),
        pos_y_keys: clip
            .pos_y_keyframes
            .iter()
            .map(|k| PosXKeyframeState {
                time_us: dur_to_us(k.time),
                pos_x: k.value,
            })
            .collect(),
        scale_keys: clip
            .scale_keyframes
            .iter()
            .map(|k| PosXKeyframeState {
                time_us: dur_to_us(k.time),
                pos_x: k.value,
            })
            .collect(),
        rotation_keys: clip
            .rotation_keyframes
            .iter()
            .map(|k| PosXKeyframeState {
                time_us: dur_to_us(k.time),
                pos_x: k.value,
            })
            .collect(),
        brightness_keys: clip
            .brightness_keyframes
            .iter()
            .map(|k| PosXKeyframeState {
                time_us: dur_to_us(k.time),
                pos_x: k.value,
            })
            .collect(),
        contrast_keys: clip
            .contrast_keyframes
            .iter()
            .map(|k| PosXKeyframeState {
                time_us: dur_to_us(k.time),
                pos_x: k.value,
            })
            .collect(),
        saturation_keys: clip
            .saturation_keyframes
            .iter()
            .map(|k| PosXKeyframeState {
                time_us: dur_to_us(k.time),
                pos_x: k.value,
            })
            .collect(),
        opacity_keys: clip
            .opacity_keyframes
            .iter()
            .map(|k| PosXKeyframeState {
                time_us: dur_to_us(k.time),
                pos_x: k.value,
            })
            .collect(),
        blur_keys: clip
            .blur_keyframes
            .iter()
            .map(|k| PosXKeyframeState {
                time_us: dur_to_us(k.time),
                pos_x: k.value,
            })
            .collect(),
    }
}

fn video_track_to_state(track: &VideoTrack) -> TrackState {
    TrackState {
        name: track.name.clone(),
        clips: track.clips.iter().map(clip_to_state).collect(),
        gain_db: None,
    }
}

fn audio_track_to_state(track: &AudioTrack) -> TrackState {
    TrackState {
        name: track.name.clone(),
        clips: track.clips.iter().map(clip_to_state).collect(),
        gain_db: if track.gain_db.abs() > 0.0001 {
            Some(track.gain_db)
        } else {
            None
        },
    }
}

fn media_pool_item_to_state(
    item: &MediaPoolItem,
    embed_preview_cache_root: Option<&Path>,
) -> MediaPoolItemState {
    let mut preview_jpeg_base64 = item.preview_jpeg_base64.clone();
    if preview_jpeg_base64.is_none()
        && let Some(cache_root) = embed_preview_cache_root
    {
        let thumb_path = thumbnail::thumbnail_path_for_in(
            cache_root,
            Path::new(&item.path),
            MEDIA_POOL_PREVIEW_MAX_DIM,
        );
        preview_jpeg_base64 = encode_preview_file_to_base64(&thumb_path);
    }

    MediaPoolItemState {
        path: item.path.clone(),
        name: item.name.clone(),
        duration_us: dur_to_us(item.duration),
        preview_jpeg_base64,
    }
}

fn subtitle_track_to_state(track: &SubtitleTrack) -> SubtitleTrackState {
    SubtitleTrackState {
        name: track.name.clone(),
        clips: track.clips.iter().map(subtitle_clip_to_state).collect(),
    }
}

fn semantic_clip_to_state(clip: &SemanticClip) -> SemanticClipState {
    SemanticClipState {
        id: clip.id,
        start_us: dur_to_us(clip.start),
        duration_us: dur_to_us(clip.duration),
        semantic_type: if clip.semantic_type.trim().is_empty() {
            DEFAULT_SEMANTIC_TYPE.to_string()
        } else {
            clip.semantic_type.trim().to_string()
        },
        label: clip.label.clone(),
        prompt_schema: if clip.prompt_schema.is_null() {
            None
        } else {
            Some(clip.prompt_schema.clone())
        },
    }
}

fn state_to_clip(state: &ClipState) -> Clip {
    Clip {
        id: state.id,
        label: state.label.clone(),
        file_path: state.path.clone(),
        start: us_to_dur(state.start_us),
        duration: us_to_dur(state.duration_us),
        source_in: us_to_dur(state.source_in_us),
        media_duration: us_to_dur(state.media_duration_us),
        link_group_id: state.link_group_id,
        audio_gain_db: state.audio_gain_db.unwrap_or(0.0).clamp(-60.0, 12.0),
        dissolve_trim_in: us_to_dur(state.dissolve_trim_in_us),
        dissolve_trim_out: us_to_dur(state.dissolve_trim_out_us),
        video_effects: build_video_effects(state.effects.as_ref()),
        local_mask_layers: vec![LocalMaskLayer::default()],
        pos_x_keyframes: state
            .pos_x_keys
            .iter()
            .map(|k| ScalarKeyframe {
                time: us_to_dur(k.time_us),
                value: k.pos_x,
            })
            .collect(),
        pos_y_keyframes: state
            .pos_y_keys
            .iter()
            .map(|k| ScalarKeyframe {
                time: us_to_dur(k.time_us),
                value: k.pos_x,
            })
            .collect(),
        scale_keyframes: state
            .scale_keys
            .iter()
            .map(|k| ScalarKeyframe {
                time: us_to_dur(k.time_us),
                value: k.pos_x,
            })
            .collect(),
        rotation_keyframes: state
            .rotation_keys
            .iter()
            .map(|k| ScalarKeyframe {
                time: us_to_dur(k.time_us),
                value: k.pos_x,
            })
            .collect(),
        brightness_keyframes: state
            .brightness_keys
            .iter()
            .map(|k| ScalarKeyframe {
                time: us_to_dur(k.time_us),
                value: k.pos_x,
            })
            .collect(),
        contrast_keyframes: state
            .contrast_keys
            .iter()
            .map(|k| ScalarKeyframe {
                time: us_to_dur(k.time_us),
                value: k.pos_x,
            })
            .collect(),
        saturation_keyframes: state
            .saturation_keys
            .iter()
            .map(|k| ScalarKeyframe {
                time: us_to_dur(k.time_us),
                value: k.pos_x,
            })
            .collect(),
        opacity_keyframes: state
            .opacity_keys
            .iter()
            .map(|k| ScalarKeyframe {
                time: us_to_dur(k.time_us),
                value: k.pos_x,
            })
            .collect(),
        blur_keyframes: state
            .blur_keys
            .iter()
            .map(|k| ScalarKeyframe {
                time: us_to_dur(k.time_us),
                value: k.pos_x,
            })
            .collect(),
    }
}

fn state_to_video_track(state: &TrackState) -> VideoTrack {
    VideoTrack {
        name: state.name.clone(),
        clips: state.clips.iter().map(state_to_clip).collect(),
    }
}

fn state_to_audio_track(state: &TrackState) -> AudioTrack {
    AudioTrack {
        name: state.name.clone(),
        clips: state.clips.iter().map(state_to_clip).collect(),
        gain_db: state.gain_db.unwrap_or(0.0).clamp(-60.0, 12.0),
    }
}

fn state_to_media_pool_item(state: &MediaPoolItemState) -> MediaPoolItem {
    MediaPoolItem {
        path: state.path.clone(),
        name: state.name.clone(),
        duration: us_to_dur(state.duration_us),
        preview_jpeg_base64: state.preview_jpeg_base64.clone(),
    }
}

fn state_to_subtitle_track(state: &SubtitleTrackState) -> SubtitleTrack {
    SubtitleTrack {
        name: state.name.clone(),
        clips: state.clips.iter().map(state_to_subtitle_clip).collect(),
    }
}

fn state_to_semantic_clip(state: &SemanticClipState) -> SemanticClip {
    SemanticClip {
        id: state.id,
        start: us_to_dur(state.start_us),
        duration: us_to_dur(state.duration_us).max(Duration::from_millis(1)),
        semantic_type: if state.semantic_type.trim().is_empty() {
            DEFAULT_SEMANTIC_TYPE.to_string()
        } else {
            state.semantic_type.trim().to_string()
        },
        label: if state.label.trim().is_empty() {
            "semantic".to_string()
        } else {
            state.label.clone()
        },
        prompt_schema: state.prompt_schema.clone().unwrap_or(Value::Null),
    }
}

fn subtitle_clip_to_state(clip: &SubtitleClip) -> SubtitleClipState {
    let color = [
        clip.color_rgba.0,
        clip.color_rgba.1,
        clip.color_rgba.2,
        clip.color_rgba.3,
    ];
    SubtitleClipState {
        id: clip.id,
        text: clip.text.clone(),
        start_us: dur_to_us(clip.start),
        duration_us: dur_to_us(clip.duration),
        pos_x: if (clip.pos_x - SUBTITLE_DEFAULT_POS_X).abs() > 0.0001 {
            Some(clip.pos_x)
        } else {
            None
        },
        pos_y: if (clip.pos_y - SUBTITLE_DEFAULT_POS_Y).abs() > 0.0001 {
            Some(clip.pos_y)
        } else {
            None
        },
        font_size: if (clip.font_size - SUBTITLE_DEFAULT_SIZE).abs() > 0.0001 {
            Some(clip.font_size)
        } else {
            None
        },
        color_rgba: if color != SUBTITLE_DEFAULT_COLOR {
            Some(color)
        } else {
            None
        },
        font_family: clip.font_family.clone(),
        font_path: clip.font_path.clone(),
        group_id: clip.group_id,
    }
}

fn state_to_subtitle_clip(state: &SubtitleClipState) -> SubtitleClip {
    let color = state.color_rgba.unwrap_or(SUBTITLE_DEFAULT_COLOR);
    SubtitleClip {
        id: state.id,
        text: state.text.clone(),
        start: us_to_dur(state.start_us),
        duration: us_to_dur(state.duration_us).max(Duration::from_millis(100)),
        pos_x: state.pos_x.unwrap_or(SUBTITLE_DEFAULT_POS_X),
        pos_y: state.pos_y.unwrap_or(SUBTITLE_DEFAULT_POS_Y),
        font_size: state.font_size.unwrap_or(SUBTITLE_DEFAULT_SIZE),
        color_rgba: (color[0], color[1], color[2], color[3]),
        font_family: state.font_family.clone(),
        font_path: state.font_path.clone(),
        group_id: state.group_id,
    }
}

fn clip_effects_from_clip(clip: &Clip) -> Option<ClipEffectsState> {
    let mut effects = ClipEffectsState::default();
    let brightness = clip.get_brightness();
    if (brightness - 0.0).abs() > 0.0001 {
        effects.brightness = Some(brightness);
    }
    let contrast = clip.get_contrast();
    if (contrast - 1.0).abs() > 0.0001 {
        effects.contrast = Some(contrast);
    }
    let saturation = clip.get_saturation();
    if (saturation - 1.0).abs() > 0.0001 {
        effects.saturation = Some(saturation);
    }
    let opacity = clip.get_opacity();
    if (opacity - 1.0).abs() > 0.0001 {
        effects.opacity = Some(opacity);
    }
    let blur_sigma = clip.get_blur_sigma();
    if blur_sigma.abs() > 0.0001 {
        effects.blur_sigma = Some(blur_sigma);
    }
    let fade_in = clip.get_fade_in();
    if fade_in.abs() > 0.0001 {
        effects.fade_in = Some(fade_in);
    }
    let fade_out = clip.get_fade_out();
    if fade_out.abs() > 0.0001 {
        effects.fade_out = Some(fade_out);
    }
    let dissolve_in = clip.get_dissolve_in();
    if dissolve_in.abs() > 0.0001 {
        effects.dissolve_in = Some(dissolve_in);
    }
    let dissolve_out = clip.get_dissolve_out();
    if dissolve_out.abs() > 0.0001 {
        effects.dissolve_out = Some(dissolve_out);
    }
    let (slide_in_dir, slide_out_dir, slide_in, slide_out) = clip.get_slide();
    if slide_in.abs() > 0.0001 {
        effects.slide_in = Some(slide_in);
        effects.slide_in_dir = Some(slide_in_dir);
    }
    if slide_out.abs() > 0.0001 {
        effects.slide_out = Some(slide_out);
        if effects.slide_out_dir.is_none() {
            effects.slide_out_dir = Some(slide_out_dir);
        }
    }
    if effects.slide_in_dir.is_none()
        && slide_in.abs() <= 0.0001
        && slide_in_dir != SlideDirection::Right
    {
        effects.slide_in_dir = Some(slide_in_dir);
    }
    if effects.slide_out_dir.is_none()
        && slide_out.abs() <= 0.0001
        && slide_out_dir != SlideDirection::Left
    {
        effects.slide_out_dir = Some(slide_out_dir);
    }
    let (zoom_in, zoom_out, zoom_amount) = clip.get_zoom();
    if zoom_in.abs() > 0.0001 {
        effects.zoom_in = Some(zoom_in);
    }
    if zoom_out.abs() > 0.0001 {
        effects.zoom_out = Some(zoom_out);
    }
    if (zoom_amount - 1.1).abs() > 0.0001 {
        effects.zoom_amount = Some(zoom_amount);
    }
    let (shock_in, shock_out, shock_amount) = clip.get_shock_zoom();
    if shock_in.abs() > 0.0001 {
        effects.shock_in = Some(shock_in);
    }
    if shock_out.abs() > 0.0001 {
        effects.shock_out = Some(shock_out);
    }
    if (shock_amount - 1.2).abs() > 0.0001 {
        effects.shock_amount = Some(shock_amount);
    }
    let scale = clip.get_scale();
    if (scale - 1.0).abs() > 0.0001 {
        effects.scale = Some(scale);
    }
    let rotation = clip.get_rotation();
    if rotation.abs() > 0.0001 {
        effects.rotation = Some(rotation);
    }
    let pos_x = clip.get_pos_x();
    if pos_x.abs() > 0.0001 {
        effects.pos_x = Some(pos_x);
    }
    let pos_y = clip.get_pos_y();
    if pos_y.abs() > 0.0001 {
        effects.pos_y = Some(pos_y);
    }
    let (h, s, l, a) = clip.get_hsla_overlay();
    if h.abs() > 0.0001 || s.abs() > 0.0001 || l.abs() > 0.0001 || a.abs() > 0.0001 {
        effects.hsla_overlay = Some(HslaOverlayState {
            hue: h,
            saturation: s,
            lightness: l,
            alpha: a,
        });
    }
    if effects.is_empty() {
        None
    } else {
        Some(effects)
    }
}

fn layer_effects_from_global(gs: &GlobalState) -> Option<LayerEffectsState> {
    let effects = gs.layer_color_blur_effects();
    let clips: Vec<LayerEffectClipState> = gs
        .layer_effect_clips()
        .iter()
        .cloned()
        .map(layer_effect_clip_to_state)
        .collect();
    let selected_clip_id = gs
        .selected_layer_effect_clip_id()
        .filter(|id| clips.iter().any(|clip| clip.id == *id));
    if effects.is_identity() && clips.is_empty() && selected_clip_id.is_none() {
        return None;
    }
    Some(LayerEffectsState {
        brightness: if effects.brightness.abs() > 0.0001 {
            Some(effects.brightness)
        } else {
            None
        },
        contrast: if (effects.contrast - 1.0).abs() > 0.0001 {
            Some(effects.contrast)
        } else {
            None
        },
        saturation: if (effects.saturation - 1.0).abs() > 0.0001 {
            Some(effects.saturation)
        } else {
            None
        },
        blur_sigma: if effects.blur_sigma.abs() > 0.0001 {
            Some(effects.blur_sigma)
        } else {
            None
        },
        clips,
        selected_clip_id,
    })
}

fn layer_effects_to_runtime(
    state: Option<&LayerEffectsState>,
    video_track_count: usize,
) -> (LayerColorBlurEffects, Vec<LayerEffectClip>, Option<u64>) {
    let Some(state) = state else {
        return (LayerColorBlurEffects::default(), Vec::new(), None);
    };
    let effects = LayerColorBlurEffects {
        brightness: state.brightness.unwrap_or(0.0),
        contrast: state.contrast.unwrap_or(1.0),
        saturation: state.saturation.unwrap_or(1.0),
        blur_sigma: state.blur_sigma.unwrap_or(0.0),
    }
    .normalized();
    let clips: Vec<LayerEffectClip> = state
        .clips
        .iter()
        .map(|clip| state_to_layer_effect_clip(clip, video_track_count))
        .collect();
    let selected_clip_id = state
        .selected_clip_id
        .filter(|id| clips.iter().any(|clip| clip.id == *id));
    (effects, clips, selected_clip_id)
}

fn layer_effect_clip_to_state(clip: LayerEffectClip) -> LayerEffectClipState {
    LayerEffectClipState {
        id: clip.id,
        start_us: dur_to_us(clip.start),
        duration_us: dur_to_us(clip.duration),
        track_index: clip.track_index,
        fade_in_us: dur_to_us(clip.fade_in),
        fade_out_us: dur_to_us(clip.fade_out),
        brightness_enabled: clip.brightness_enabled,
        brightness: if (clip.brightness - 0.0).abs() > 0.0001 {
            Some(clip.brightness)
        } else {
            None
        },
        brightness_keys: clip
            .brightness_keyframes
            .iter()
            .map(|k| PosXKeyframeState {
                time_us: dur_to_us(k.time),
                pos_x: k.value,
            })
            .collect(),
        contrast_enabled: clip.contrast_enabled,
        contrast: if (clip.contrast - 1.0).abs() > 0.0001 {
            Some(clip.contrast)
        } else {
            None
        },
        contrast_keys: clip
            .contrast_keyframes
            .iter()
            .map(|k| PosXKeyframeState {
                time_us: dur_to_us(k.time),
                pos_x: k.value,
            })
            .collect(),
        saturation_enabled: clip.saturation_enabled,
        saturation: if (clip.saturation - 1.0).abs() > 0.0001 {
            Some(clip.saturation)
        } else {
            None
        },
        saturation_keys: clip
            .saturation_keyframes
            .iter()
            .map(|k| PosXKeyframeState {
                time_us: dur_to_us(k.time),
                pos_x: k.value,
            })
            .collect(),
        blur_enabled: clip.blur_enabled,
        blur_sigma: if (clip.blur_sigma - 0.0).abs() > 0.0001 {
            Some(clip.blur_sigma)
        } else {
            None
        },
        blur_keys: clip
            .blur_keyframes
            .iter()
            .map(|k| PosXKeyframeState {
                time_us: dur_to_us(k.time),
                pos_x: k.value,
            })
            .collect(),
        motionloom_enabled: clip.motionloom_enabled,
        motionloom_script: clip.motionloom_script,
    }
}

fn state_to_layer_effect_clip(
    state: &LayerEffectClipState,
    video_track_count: usize,
) -> LayerEffectClip {
    let duration = us_to_dur(state.duration_us).max(Duration::from_millis(100));
    let track_index = if video_track_count == 0 {
        state.track_index
    } else {
        state.track_index.min(video_track_count.saturating_sub(1))
    };
    LayerEffectClip {
        id: state.id,
        start: us_to_dur(state.start_us),
        duration,
        track_index,
        fade_in: us_to_dur(state.fade_in_us).min(duration),
        fade_out: us_to_dur(state.fade_out_us).min(duration),
        brightness: state.brightness.unwrap_or(0.0).clamp(-1.0, 1.0),
        contrast: state.contrast.unwrap_or(1.0).clamp(0.0, 2.0),
        saturation: state.saturation.unwrap_or(1.0).clamp(0.0, 2.0),
        blur_sigma: state.blur_sigma.unwrap_or(0.0).clamp(0.0, 64.0),
        brightness_enabled: state.brightness_enabled,
        contrast_enabled: state.contrast_enabled,
        saturation_enabled: state.saturation_enabled,
        blur_enabled: state.blur_enabled,
        brightness_keyframes: state
            .brightness_keys
            .iter()
            .map(|k| ScalarKeyframe {
                time: us_to_dur(k.time_us),
                value: k.pos_x.clamp(-1.0, 1.0),
            })
            .collect(),
        contrast_keyframes: state
            .contrast_keys
            .iter()
            .map(|k| ScalarKeyframe {
                time: us_to_dur(k.time_us),
                value: k.pos_x.clamp(0.0, 2.0),
            })
            .collect(),
        saturation_keyframes: state
            .saturation_keys
            .iter()
            .map(|k| ScalarKeyframe {
                time: us_to_dur(k.time_us),
                value: k.pos_x.clamp(0.0, 2.0),
            })
            .collect(),
        blur_keyframes: state
            .blur_keys
            .iter()
            .map(|k| ScalarKeyframe {
                time: us_to_dur(k.time_us),
                value: k.pos_x.clamp(0.0, 64.0),
            })
            .collect(),
        motionloom_enabled: state.motionloom_enabled,
        motionloom_script: state.motionloom_script.clone(),
    }
}

fn encode_preview_file_to_base64(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_EMBEDDED_PREVIEW_BYTES {
        return None;
    }
    Some(BASE64_STANDARD.encode(bytes))
}

fn fallback_media_pool_from_timeline(gs: &GlobalState) -> Vec<MediaPoolItem> {
    let mut out: Vec<MediaPoolItem> = Vec::new();

    let mut push_clip = |clip: &Clip| {
        if clip.file_path.trim().is_empty() {
            return;
        }
        if !is_supported_media_path(&clip.file_path) {
            return;
        }
        let duration = clip.media_duration.max(clip.duration);
        if let Some(existing) = out.iter_mut().find(|item| item.path == clip.file_path) {
            if duration > existing.duration {
                existing.duration = duration;
            }
            return;
        }
        let name = Path::new(&clip.file_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        out.push(MediaPoolItem {
            path: clip.file_path.clone(),
            name,
            duration,
            preview_jpeg_base64: None,
        });
    };

    for clip in &gs.v1_clips {
        push_clip(clip);
    }
    for track in &gs.audio_tracks {
        for clip in &track.clips {
            push_clip(clip);
        }
    }
    for track in &gs.video_tracks {
        for clip in &track.clips {
            push_clip(clip);
        }
    }

    out
}

fn sync_active_source_with_media_pool(gs: &mut GlobalState) {
    if let Some(item) = gs
        .media_pool
        .iter()
        .find(|item| item.path == gs.active_source_path)
        .cloned()
    {
        gs.active_source_name = item.name;
        gs.active_source_duration = item.duration;
        return;
    }

    if let Some(first) = gs.media_pool.first().cloned() {
        gs.active_source_path = first.path;
        gs.active_source_name = first.name;
        gs.active_source_duration = first.duration;
    } else {
        gs.active_source_path.clear();
        gs.active_source_name = "No Source Loaded".to_string();
        gs.active_source_duration = Duration::ZERO;
    }
}

impl ClipEffectsState {
    fn is_empty(&self) -> bool {
        self.brightness.is_none()
            && self.contrast.is_none()
            && self.saturation.is_none()
            && self.opacity.is_none()
            && self.blur_sigma.is_none()
            && self.fade_in.is_none()
            && self.fade_out.is_none()
            && self.dissolve_in.is_none()
            && self.dissolve_out.is_none()
            && self.slide_in.is_none()
            && self.slide_out.is_none()
            && self.slide_in_dir.is_none()
            && self.slide_out_dir.is_none()
            && self.zoom_in.is_none()
            && self.zoom_out.is_none()
            && self.zoom_amount.is_none()
            && self.shock_in.is_none()
            && self.shock_out.is_none()
            && self.shock_amount.is_none()
            && self.scale.is_none()
            && self.rotation.is_none()
            && self.pos_x.is_none()
            && self.pos_y.is_none()
            && self.hsla_overlay.is_none()
    }
}

fn build_video_effects(effects: Option<&ClipEffectsState>) -> Vec<VideoEffect> {
    let mut result = VideoEffect::standard_set();
    let Some(effects) = effects else {
        return result;
    };
    for effect in &mut result {
        match effect {
            VideoEffect::ColorCorrection {
                brightness,
                contrast,
                saturation,
            } => {
                if let Some(val) = effects.brightness {
                    *brightness = val;
                }
                if let Some(val) = effects.contrast {
                    *contrast = val;
                }
                if let Some(val) = effects.saturation {
                    *saturation = val;
                }
            }
            VideoEffect::Transform {
                scale,
                position_x,
                position_y,
                rotation_deg,
            } => {
                if let Some(val) = effects.scale {
                    *scale = val;
                }
                if let Some(val) = effects.rotation {
                    *rotation_deg = val;
                }
                if let Some(val) = effects.pos_x {
                    *position_x = val;
                }
                if let Some(val) = effects.pos_y {
                    *position_y = val;
                }
            }
            VideoEffect::Tint {
                hue,
                saturation,
                lightness,
                alpha,
            } => {
                if let Some(hsla_overlay) = effects.hsla_overlay.as_ref() {
                    *hue = hsla_overlay.hue;
                    *saturation = hsla_overlay.saturation;
                    *lightness = hsla_overlay.lightness;
                    *alpha = hsla_overlay.alpha;
                }
            }
            VideoEffect::Opacity { alpha } => {
                if let Some(val) = effects.opacity {
                    *alpha = val;
                }
            }
            VideoEffect::GaussianBlur { sigma } => {
                if let Some(val) = effects.blur_sigma {
                    *sigma = val;
                }
            }
            VideoEffect::Fade { fade_in, fade_out } => {
                if let Some(val) = effects.fade_in {
                    *fade_in = val;
                }
                if let Some(val) = effects.fade_out {
                    *fade_out = val;
                }
            }
            VideoEffect::Dissolve {
                dissolve_in,
                dissolve_out,
            } => {
                if let Some(val) = effects.dissolve_in {
                    *dissolve_in = val;
                }
                if let Some(val) = effects.dissolve_out {
                    *dissolve_out = val;
                }
            }
            VideoEffect::Slide {
                in_direction,
                out_direction,
                slide_in,
                slide_out,
            } => {
                if let Some(val) = effects.slide_in {
                    *slide_in = val;
                }
                if let Some(val) = effects.slide_out {
                    *slide_out = val;
                }
                if let Some(val) = effects.slide_in_dir {
                    *in_direction = val;
                }
                if let Some(val) = effects.slide_out_dir {
                    *out_direction = val;
                }
            }
            VideoEffect::Zoom {
                zoom_in,
                zoom_out,
                zoom_amount,
            } => {
                if let Some(val) = effects.zoom_in {
                    *zoom_in = val;
                }
                if let Some(val) = effects.zoom_out {
                    *zoom_out = val;
                }
                if let Some(val) = effects.zoom_amount {
                    *zoom_amount = val;
                }
            }
            VideoEffect::ShockZoom {
                shock_in,
                shock_out,
                shock_amount,
            } => {
                if let Some(val) = effects.shock_in {
                    *shock_in = val;
                }
                if let Some(val) = effects.shock_out {
                    *shock_out = val;
                }
                if let Some(val) = effects.shock_amount {
                    *shock_amount = val;
                }
            }
            _ => {}
        }
    }
    result
}

fn dur_to_us(dur: Duration) -> u64 {
    dur.as_micros() as u64
}

fn us_to_dur(us: u64) -> Duration {
    Duration::from_micros(us)
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn now_epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn stable_recovery_key(project_path: Option<&Path>) -> String {
    let source = project_path
        .map(|path| {
            let source = path.to_string_lossy().to_string();
            #[cfg(target_os = "windows")]
            {
                // Windows paths are case-insensitive, so normalize to avoid duplicate keys.
                source.to_lowercase()
            }
            #[cfg(not(target_os = "windows"))]
            {
                source
            }
        })
        .unwrap_or_else(|| "__unsaved__".to_string());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn recovery_project_name(project_path: Option<&Path>) -> String {
    project_path
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| "Unsaved Project".to_string())
}

fn saved_project_is_newer_than_draft(draft: &RecoveryDraft) -> bool {
    let Some(project_path) = draft.project_path.as_ref() else {
        return false;
    };
    let Ok(meta) = fs::metadata(project_path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    let Ok(elapsed) = modified.duration_since(UNIX_EPOCH) else {
        return false;
    };
    elapsed.as_millis() >= draft.updated_at_ms
}

fn prune_snapshots(dir: &Path, keep: usize) -> anyhow::Result<()> {
    if keep == 0 {
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("snapshot_"))
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    if entries.len() <= keep {
        return Ok(());
    }
    let remove_count = entries.len().saturating_sub(keep);
    for entry in entries.into_iter().take(remove_count) {
        let _ = fs::remove_file(entry.path());
    }
    Ok(())
}
