// =========================================
// =========================================
// src/core/global_state.rs
use crate::core::effects::{LayerColorBlurEffects, PerClipColorBlurEffects};
use crate::core::media_tools::MediaDependencyStatus;
use crate::core::project_state::default_project_dir;
use crate::core::proxy::{
    ProxyEntry, ProxyJob, ProxyLookup, ProxyStatus, proxy_dir_for, proxy_key, proxy_path_for_in,
};
use crate::core::srt::{SrtError, parse_srt};
use crate::core::waveform::{
    WaveformEntry, WaveformJob, WaveformLookup, WaveformStatus, load_waveform_file, waveform_key,
    waveform_path_for_in,
};
use gpui::EventEmitter;
use log::info;
use motionloom::{
    LayerEffectClip as MotionLoomLayerEffectClip, RuntimeFrameOutput, RuntimeProgram,
    compile_runtime_program, is_graph_script, keyframe, parse_graph_script, transitions,
};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use thiserror::Error;

fn motionloom_runtime_cache() -> &'static Mutex<HashMap<u64, Option<RuntimeProgram>>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, Option<RuntimeProgram>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn motionloom_script_hash(script: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    script.hash(&mut hasher);
    hasher.finish()
}

fn motionloom_runtime_for_script(script: &str) -> Option<RuntimeProgram> {
    let script = script.trim();
    if script.is_empty() || !is_graph_script(script) {
        return None;
    }
    let key = motionloom_script_hash(script);
    if let Ok(cache) = motionloom_runtime_cache().lock()
        && let Some(existing) = cache.get(&key)
    {
        return existing.clone();
    }
    let compiled = parse_graph_script(script)
        .ok()
        .and_then(|graph| compile_runtime_program(graph).ok());
    if let Ok(mut cache) = motionloom_runtime_cache().lock() {
        cache.insert(key, compiled.clone());
    }
    compiled
}

fn motionloom_output_for_layer(
    layer: &MotionLoomLayerEffectClip,
    timeline_time: Duration,
) -> Option<RuntimeFrameOutput> {
    if !layer.motionloom_enabled {
        return None;
    }
    let runtime = motionloom_runtime_for_script(&layer.motionloom_script)?;
    let local = layer.local_time(timeline_time)?;
    Some(runtime.evaluate_at_time_sec(local.as_secs_f32(), Some(layer.duration.as_secs_f32())))
}

fn is_image_media_path(path: &str) -> bool {
    let p = path.to_lowercase();
    p.ends_with(".jpg")
        || p.ends_with(".jpeg")
        || p.ends_with(".png")
        || p.ends_with(".webp")
        || p.ends_with(".bmp")
        || p.ends_with(".gif")
}

fn is_supported_media_path(path: &str) -> bool {
    let p = path.to_lowercase();
    p.ends_with(".mp4")
        || p.ends_with(".mov")
        || p.ends_with(".mkv")
        || p.ends_with(".webm")
        || p.ends_with(".avi")
        || p.ends_with(".flv")
        || p.ends_with(".m4v")
        || p.ends_with(".mp3")
        || p.ends_with(".wav")
        || p.ends_with(".m4a")
        || p.ends_with(".aac")
        || p.ends_with(".flac")
        || p.ends_with(".ogg")
        || p.ends_with(".opus")
        || p.ends_with(".jpg")
        || p.ends_with(".jpeg")
        || p.ends_with(".png")
        || p.ends_with(".webp")
        || p.ends_with(".bmp")
        || p.ends_with(".gif")
        || p.ends_with(".tif")
        || p.ends_with(".tiff")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTool {
    Select,
    Razor,
    TrackSweep,
}
// ==========

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppPage {
    Editor,
    AiSrt,
    AiAgents,
    MotionLoom,
    VectorLab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiChatRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone)]
pub struct AiChatMessage {
    pub role: AiChatRole,
    pub text: String,
    pub pending: bool,
}

// ── Silence preview modal state for ACP inspect-intent flow ──
// Each candidate row holds a silence range the user can select/deselect
// before confirming the edit back to the ACP agent.
#[derive(Debug, Clone)]
pub struct SilencePreviewCandidate {
    pub start_ms: u64,
    pub end_ms: u64,
    pub confidence: f32,
    pub reason: String,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct SilencePreviewModalState {
    pub candidates: Vec<SilencePreviewCandidate>,
    pub timeline_revision: String,
}

pub type SlideDirection = motionloom::SlideDirection;
pub type VideoEffect = motionloom::VideoEffect;
pub const MAX_LOCAL_MASK_LAYERS: usize = motionloom::MAX_LOCAL_MASK_LAYERS;
pub type LocalMaskLayer = motionloom::LocalMaskLayer;
#[derive(Debug, Clone)]
pub struct Clip {
    pub id: u64,
    pub label: String,
    pub file_path: String,

    // Timeline placement
    pub start: Duration,
    pub duration: Duration,

    // Source mapping
    pub source_in: Duration,

    pub media_duration: Duration,
    // Keep paired audio/video relationship for linked-edit workflows.
    pub link_group_id: Option<u64>,
    // Per-clip audio trim gain in dB (applies on top of track gain for audio tracks).
    pub audio_gain_db: f32,

    // Dissolve trimming bookkeeping (for V1 auto-trim)
    pub dissolve_trim_in: Duration,
    pub dissolve_trim_out: Duration,

    // brightness default (0.0)
    // pub brightness: f32,
    pub video_effects: Vec<VideoEffect>,
    pub local_mask_layers: Vec<LocalMaskLayer>,
    pub pos_x_keyframes: Vec<ScalarKeyframe>,
    pub pos_y_keyframes: Vec<ScalarKeyframe>,
    pub scale_keyframes: Vec<ScalarKeyframe>,
    pub rotation_keyframes: Vec<ScalarKeyframe>,
    pub brightness_keyframes: Vec<ScalarKeyframe>,
    pub contrast_keyframes: Vec<ScalarKeyframe>,
    pub saturation_keyframes: Vec<ScalarKeyframe>,
    pub opacity_keyframes: Vec<ScalarKeyframe>,
    pub blur_keyframes: Vec<ScalarKeyframe>,
}

#[derive(Debug, Clone)]
pub struct SubtitleClip {
    pub id: u64,
    pub text: String,
    pub start: Duration,
    pub duration: Duration,
    pub pos_x: f32,
    pub pos_y: f32,
    pub font_size: f32,
    pub color_rgba: (u8, u8, u8, u8),
    pub font_family: Option<String>,
    pub font_path: Option<String>,
    pub group_id: Option<u64>,
}

pub type ScalarKeyframe = motionloom::keyframe::ScalarKeyframe;

impl SubtitleClip {
    pub fn end(&self) -> Duration {
        self.start + self.duration
    }
}

#[derive(Debug, Clone)]
pub struct SubtitleTrack {
    pub name: String,
    pub clips: Vec<SubtitleClip>,
}

const DEFAULT_SEMANTIC_ASSET_MODE: &str = "video";
const DEFAULT_SEMANTIC_PROVIDER: &str = "veo_3_1";
const DEFAULT_SEMANTIC_VIDEO_WIDTH: u32 = 1280;
const DEFAULT_SEMANTIC_VIDEO_HEIGHT: u32 = 720;
const SEMANTIC_DURATION_SYNC_EPS_SEC: f64 = 0.02;

#[derive(Debug, Clone, Default)]
pub struct SemanticSchemaValidation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl SemanticSchemaValidation {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

#[derive(Debug, Error)]
pub enum GlobalStateError {
    #[error("No semantic clip selected.")]
    NoSemanticClipSelected,
    #[error("Invalid JSON schema: {source}")]
    InvalidSemanticSchemaJson { source: serde_json::Error },
    #[error("duration_sec must be a number (seconds).")]
    InvalidSemanticDurationType,
    #[error("duration_sec must be > 0.")]
    InvalidSemanticDurationValue,
    #[error("Selected semantic clip no longer exists.")]
    SelectedSemanticClipMissing,
    #[error("{message}")]
    SemanticSchemaValidationFailed { message: String },
    #[error("Target semantic clip no longer exists.")]
    TargetSemanticClipMissing,
    #[error("Generated media path is not a supported format.")]
    UnsupportedGeneratedMediaPath,
    #[error("Clip no longer exists.")]
    ClipNoLongerExists,
    #[error("Dissolve requires adjacent clips on V1.")]
    DissolveRequiresAdjacentClips,
    #[error("Insufficient media (handles) for dissolve.")]
    InsufficientDissolveHandles,
    #[error("Failed to parse SRT: {source}")]
    SrtParse { source: SrtError },
    #[error("No clip to cut on {track}")]
    NoClipToCut { track: &'static str },
    #[error("No subtitle to cut on Subtitle Track")]
    NoSubtitleToCut,
}

fn merge_json_value(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_obj), Value::Object(overlay_obj)) => {
            for (key, value) in overlay_obj {
                if let Some(existing) = base_obj.get_mut(key) {
                    merge_json_value(existing, value);
                } else {
                    base_obj.insert(key.clone(), value.clone());
                }
            }
        }
        (base_value, overlay_value) => {
            *base_value = overlay_value.clone();
        }
    }
}

fn ensure_child_object<'a>(
    parent: &'a mut Map<String, Value>,
    key: &str,
) -> &'a mut Map<String, Value> {
    let entry = parent
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    entry
        .as_object_mut()
        .expect("semantic schema child object must exist")
}

fn ensure_value_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value
        .as_object_mut()
        .expect("semantic schema root must be object")
}

fn semantic_duration_sec(duration: Duration) -> f64 {
    duration.as_secs_f64()
}

fn semantic_provider_needs_video_1280x720(provider: &str, mode: &str) -> bool {
    if mode != "video" {
        return false;
    }
    matches!(
        provider,
        "veo_3_1"
            | "google/veo_3_1"
            | "google/veo-3.1-generate-preview"
            | "openai/sora-2"
            | "sora-2"
            | "sora_2"
            | "openai/sora-2-pro"
            | "sora-2-pro"
            | "sora_2_pro"
    )
}

fn default_semantic_prompt_schema(
    clip_id: u64,
    _start: Duration,
    duration: Duration,
    semantic_type: &str,
    label: &str,
) -> Value {
    let duration_sec = semantic_duration_sec(duration);
    let normalized_type = if semantic_type.trim().is_empty() {
        "content_support"
    } else {
        semantic_type.trim()
    };
    let normalized_label = if label.trim().is_empty() {
        "semantic"
    } else {
        label.trim()
    };

    json!({
        "shot_id": format!("semantic-{clip_id}"),
        "asset_mode": DEFAULT_SEMANTIC_ASSET_MODE,
        "provider": DEFAULT_SEMANTIC_PROVIDER,
        "semantic_goal": normalized_type,
        "script_anchor": "",
        "duration_sec": duration_sec,
        "visual": {
            "entities": [],
            "visual_type": "live_action",
            "camera": "medium shot, slow push-in",
            "style": "cinematic, neutral grade",
            "lighting": "soft natural light",
            "negative_constraints": [
                "no watermark",
                "no logo",
                "no gibberish text"
            ]
        },
        "prompts": {
            "image_prompt": normalized_label,
            "video_prompt": normalized_label
        },
        "image_options": {
            "width": DEFAULT_SEMANTIC_VIDEO_WIDTH,
            "height": DEFAULT_SEMANTIC_VIDEO_HEIGHT
        }
    })
}

fn sync_semantic_prompt_schema_fields(clip: &mut SemanticClip) {
    let mut normalized = default_semantic_prompt_schema(
        clip.id,
        clip.start,
        clip.duration,
        clip.semantic_type.as_str(),
        clip.label.as_str(),
    );
    merge_json_value(&mut normalized, &clip.prompt_schema);

    let duration_sec = semantic_duration_sec(clip.duration);

    if let Some(root) = normalized.as_object_mut() {
        root.insert(
            "shot_id".to_string(),
            Value::String(format!("semantic-{}", clip.id)),
        );
        root.insert("duration_sec".to_string(), json!(duration_sec));
        // Timing follows semantic clip placement/duration; remove legacy fields to keep schema lean.
        root.remove("schema_version");
        root.remove("timecode_start_ms");
        root.remove("timecode_end_ms");
        root.remove("timing");
        root.remove("provider_prompts");
        root.remove("provider_limits");
        root.remove("meta");
        if let Some(mode) = root.get("asset_mode").and_then(Value::as_str) {
            if mode.trim().is_empty() {
                root.insert(
                    "asset_mode".to_string(),
                    Value::String(DEFAULT_SEMANTIC_ASSET_MODE.to_string()),
                );
            }
        } else {
            root.insert(
                "asset_mode".to_string(),
                Value::String(DEFAULT_SEMANTIC_ASSET_MODE.to_string()),
            );
        }
        if let Some(provider) = root.get("provider").and_then(Value::as_str) {
            if provider.trim().is_empty() {
                root.insert(
                    "provider".to_string(),
                    Value::String(DEFAULT_SEMANTIC_PROVIDER.to_string()),
                );
            }
        } else {
            root.insert(
                "provider".to_string(),
                Value::String(DEFAULT_SEMANTIC_PROVIDER.to_string()),
            );
        }
        if clip.semantic_type.trim().is_empty() {
            root.insert(
                "semantic_goal".to_string(),
                Value::String("content_support".to_string()),
            );
        } else {
            root.insert(
                "semantic_goal".to_string(),
                Value::String(clip.semantic_type.clone()),
            );
        }
        let prompts = ensure_child_object(root, "prompts");
        if prompts
            .get("image_prompt")
            .and_then(Value::as_str)
            .is_none_or(|v| v.trim().is_empty())
        {
            prompts.insert(
                "image_prompt".to_string(),
                Value::String(clip.label.clone()),
            );
        }
        if prompts
            .get("video_prompt")
            .and_then(Value::as_str)
            .is_none_or(|v| v.trim().is_empty())
        {
            prompts.insert(
                "video_prompt".to_string(),
                Value::String(clip.label.clone()),
            );
        }
        prompts.remove("fallback_search_query");

        // Keep width/height defaults available for video providers that require fixed sizes.
        let mode_text = root
            .get("asset_mode")
            .and_then(Value::as_str)
            .map(|v| v.trim().to_ascii_lowercase())
            .unwrap_or_else(|| DEFAULT_SEMANTIC_ASSET_MODE.to_string());
        let provider_text = root
            .get("provider")
            .and_then(Value::as_str)
            .map(|v| v.trim().to_ascii_lowercase())
            .unwrap_or_else(|| DEFAULT_SEMANTIC_PROVIDER.to_string());
        if semantic_provider_needs_video_1280x720(provider_text.as_str(), mode_text.as_str()) {
            let image_options = ensure_child_object(root, "image_options");
            let width = image_options
                .get("width")
                .and_then(Value::as_u64)
                .filter(|v| *v > 0 && *v <= u32::MAX as u64);
            let height = image_options
                .get("height")
                .and_then(Value::as_u64)
                .filter(|v| *v > 0 && *v <= u32::MAX as u64);
            if width.is_none() {
                image_options.insert("width".to_string(), json!(DEFAULT_SEMANTIC_VIDEO_WIDTH));
            }
            if height.is_none() {
                image_options.insert("height".to_string(), json!(DEFAULT_SEMANTIC_VIDEO_HEIGHT));
            }
        }
    } else {
        normalized = default_semantic_prompt_schema(
            clip.id,
            clip.start,
            clip.duration,
            clip.semantic_type.as_str(),
            clip.label.as_str(),
        );
    }

    clip.prompt_schema = normalized;
}

fn semantic_provider_from_schema(schema: &Value) -> String {
    schema
        .get("provider")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_SEMANTIC_PROVIDER.to_string())
}

fn semantic_asset_mode_from_schema(schema: &Value) -> String {
    schema
        .get("asset_mode")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_SEMANTIC_ASSET_MODE.to_string())
}

fn semantic_prompt_key_for_mode(mode: &str) -> &'static str {
    if mode == "image" {
        "image_prompt"
    } else {
        "video_prompt"
    }
}

fn semantic_prompt_text_for_mode(schema: &Value, mode: &str) -> Option<String> {
    let prompts = schema.get("prompts").and_then(Value::as_object)?;
    prompts
        .get(semantic_prompt_key_for_mode(mode))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn semantic_image_dimension_from_schema(schema: &Value, key: &str) -> Option<u32> {
    let value = schema
        .get("image_options")
        .and_then(Value::as_object)
        .and_then(|opts| opts.get(key))
        .and_then(Value::as_u64)?;
    if value == 0 || value > u32::MAX as u64 {
        return None;
    }
    Some(value as u32)
}

fn normalize_semantic_asset_mode(raw: &str) -> &'static str {
    if raw.trim().eq_ignore_ascii_case("image") {
        "image"
    } else {
        "video"
    }
}

fn validate_semantic_prompt_schema(clip: &SemanticClip) -> SemanticSchemaValidation {
    let mut out = SemanticSchemaValidation::default();
    if !clip.prompt_schema.is_object() {
        out.errors
            .push("Semantic prompt schema must be a JSON object.".to_string());
        return out;
    }

    let duration_sec = semantic_duration_sec(clip.duration);
    if let Some(schema_duration) = clip
        .prompt_schema
        .get("duration_sec")
        .and_then(Value::as_f64)
        && (schema_duration - duration_sec).abs() > SEMANTIC_DURATION_SYNC_EPS_SEC
    {
        out.warnings.push(format!(
            "duration_sec ({schema_duration:.2}) was out of sync; expected {duration_sec:.2}s."
        ));
    }

    out
}

#[derive(Debug, Clone)]
pub struct SemanticClip {
    pub id: u64,
    pub start: Duration,
    pub duration: Duration,
    // Stores marker category (for example: content supplement / cover edit).
    pub semantic_type: String,
    pub label: String,
    // Stores provider-agnostic generation schema editable from Inspector.
    pub prompt_schema: Value,
}

impl SemanticClip {
    pub fn end(&self) -> Duration {
        self.start + self.duration
    }
}

#[derive(Clone, Debug, Default)]
pub struct SubtitleGroupTransform {
    pub offset_x: f32,
    pub offset_y: f32,
    pub scale: f32,
}

impl SubtitleTrack {
    pub fn new(name: String) -> Self {
        Self {
            name,
            clips: Vec::new(),
        }
    }
}

impl Clip {
    pub fn end(&self) -> Duration {
        self.start + self.duration
    }

    fn set_scalar_keyframe(keys: &mut Vec<ScalarKeyframe>, t: Duration, value: f32) {
        keyframe::set_or_insert(keys, t, value, Duration::from_millis(33));
    }

    fn scalar_keyframe_index_at(keys: &[ScalarKeyframe], t: Duration) -> Option<usize> {
        keyframe::index_at(keys, t, Duration::from_millis(33))
    }

    fn sample_scalar(keys: &[ScalarKeyframe], t: Duration, fallback: f32) -> f32 {
        keyframe::sample_linear(keys, t, fallback)
    }

    pub fn set_pos_x_keyframe(&mut self, t: Duration, value: f32) {
        let t = t.min(self.duration);
        Self::set_scalar_keyframe(&mut self.pos_x_keyframes, t, value);
    }

    pub fn pos_x_keyframe_index_at(&self, t: Duration) -> Option<usize> {
        Self::scalar_keyframe_index_at(&self.pos_x_keyframes, t)
    }

    pub fn sample_pos_x(&self, t: Duration) -> f32 {
        let t = t.min(self.duration);
        Self::sample_scalar(&self.pos_x_keyframes, t, self.get_pos_x())
    }

    pub fn set_pos_y_keyframe(&mut self, t: Duration, value: f32) {
        let t = t.min(self.duration);
        Self::set_scalar_keyframe(&mut self.pos_y_keyframes, t, value);
    }

    pub fn pos_y_keyframe_index_at(&self, t: Duration) -> Option<usize> {
        Self::scalar_keyframe_index_at(&self.pos_y_keyframes, t)
    }

    pub fn sample_pos_y(&self, t: Duration) -> f32 {
        let t = t.min(self.duration);
        Self::sample_scalar(&self.pos_y_keyframes, t, self.get_pos_y())
    }

    pub fn set_scale_keyframe(&mut self, t: Duration, value: f32) {
        let t = t.min(self.duration);
        Self::set_scalar_keyframe(&mut self.scale_keyframes, t, value);
    }

    pub fn scale_keyframe_index_at(&self, t: Duration) -> Option<usize> {
        Self::scalar_keyframe_index_at(&self.scale_keyframes, t)
    }

    pub fn sample_scale(&self, t: Duration) -> f32 {
        let t = t.min(self.duration);
        Self::sample_scalar(&self.scale_keyframes, t, self.get_scale())
    }

    pub fn set_rotation_keyframe(&mut self, t: Duration, value: f32) {
        let t = t.min(self.duration);
        Self::set_scalar_keyframe(&mut self.rotation_keyframes, t, value);
    }

    pub fn rotation_keyframe_index_at(&self, t: Duration) -> Option<usize> {
        Self::scalar_keyframe_index_at(&self.rotation_keyframes, t)
    }

    pub fn sample_rotation(&self, t: Duration) -> f32 {
        let t = t.min(self.duration);
        Self::sample_scalar(&self.rotation_keyframes, t, self.get_rotation())
    }

    pub fn set_brightness_keyframe(&mut self, t: Duration, value: f32) {
        let t = t.min(self.duration);
        Self::set_scalar_keyframe(&mut self.brightness_keyframes, t, value);
    }

    pub fn brightness_keyframe_index_at(&self, t: Duration) -> Option<usize> {
        Self::scalar_keyframe_index_at(&self.brightness_keyframes, t)
    }

    pub fn sample_brightness(&self, t: Duration) -> f32 {
        let t = t.min(self.duration);
        Self::sample_scalar(&self.brightness_keyframes, t, self.get_brightness())
    }

    pub fn set_contrast_keyframe(&mut self, t: Duration, value: f32) {
        let t = t.min(self.duration);
        Self::set_scalar_keyframe(&mut self.contrast_keyframes, t, value);
    }

    pub fn contrast_keyframe_index_at(&self, t: Duration) -> Option<usize> {
        Self::scalar_keyframe_index_at(&self.contrast_keyframes, t)
    }

    pub fn sample_contrast(&self, t: Duration) -> f32 {
        let t = t.min(self.duration);
        Self::sample_scalar(&self.contrast_keyframes, t, self.get_contrast())
    }

    pub fn set_saturation_keyframe(&mut self, t: Duration, value: f32) {
        let t = t.min(self.duration);
        Self::set_scalar_keyframe(&mut self.saturation_keyframes, t, value);
    }

    pub fn saturation_keyframe_index_at(&self, t: Duration) -> Option<usize> {
        Self::scalar_keyframe_index_at(&self.saturation_keyframes, t)
    }

    pub fn sample_saturation(&self, t: Duration) -> f32 {
        let t = t.min(self.duration);
        Self::sample_scalar(&self.saturation_keyframes, t, self.get_saturation())
    }

    pub fn set_opacity_keyframe(&mut self, t: Duration, value: f32) {
        let t = t.min(self.duration);
        Self::set_scalar_keyframe(&mut self.opacity_keyframes, t, value);
    }

    pub fn opacity_keyframe_index_at(&self, t: Duration) -> Option<usize> {
        Self::scalar_keyframe_index_at(&self.opacity_keyframes, t)
    }

    pub fn sample_opacity(&self, t: Duration) -> f32 {
        let t = t.min(self.duration);
        Self::sample_scalar(&self.opacity_keyframes, t, self.get_opacity())
    }

    pub fn set_blur_keyframe(&mut self, t: Duration, value: f32) {
        let t = t.min(self.duration);
        Self::set_scalar_keyframe(&mut self.blur_keyframes, t, value);
    }

    pub fn blur_keyframe_index_at(&self, t: Duration) -> Option<usize> {
        Self::scalar_keyframe_index_at(&self.blur_keyframes, t)
    }

    pub fn sample_blur(&self, t: Duration) -> f32 {
        let t = t.min(self.duration);
        Self::sample_scalar(&self.blur_keyframes, t, self.get_blur_sigma())
    }
    // ===
    // ✅ [Helper] Quickly retrieve the current brightness (for compatibility with legacy logic)
    // Logic: iterate through the list, find the first ColorCorrection effect, and return its brightness
    pub fn get_brightness(&self) -> f32 {
        for effect in &self.video_effects {
            if let VideoEffect::ColorCorrection { brightness, .. } = effect {
                return *brightness;
            }
        }
        0.0 // Default value
    }

    // ✅ [Helper] Convenience helper for setting brightness
    // Logic: update the existing color-correction effect if present; otherwise add a new one
    pub fn set_brightness(&mut self, new_val: f32) {
        for effect in &mut self.video_effects {
            if let VideoEffect::ColorCorrection { brightness, .. } = effect {
                *brightness = new_val;
                return;
            }
        }
        // If there is no effect yet, add a new one
        self.video_effects.push(VideoEffect::ColorCorrection {
            brightness: new_val,
            contrast: 1.0,
            saturation: 1.0,
        });
    }

    // ✅ [NEW] Helper: Contrast (default 1.0)
    pub fn get_contrast(&self) -> f32 {
        for effect in &self.video_effects {
            if let VideoEffect::ColorCorrection { contrast, .. } = effect {
                return *contrast;
            }
        }
        1.0 // Note: the default contrast is 1.0
    }

    pub fn set_contrast(&mut self, new_val: f32) {
        for effect in &mut self.video_effects {
            if let VideoEffect::ColorCorrection { contrast, .. } = effect {
                *contrast = new_val;
                return;
            }
        }
        // If none is found, add one (defaults: brightness 0, contrast 1, saturation 1)
        self.video_effects.push(VideoEffect::ColorCorrection {
            brightness: 0.0,
            contrast: new_val,
            saturation: 1.0,
        });
    }

    // ✅ [NEW] Helper: Saturation (default 1.0)
    pub fn get_saturation(&self) -> f32 {
        for effect in &self.video_effects {
            if let VideoEffect::ColorCorrection { saturation, .. } = effect {
                return *saturation;
            }
        }
        1.0 // Note: the default saturation is 1.0
    }

    pub fn set_saturation(&mut self, new_val: f32) {
        for effect in &mut self.video_effects {
            if let VideoEffect::ColorCorrection { saturation, .. } = effect {
                *saturation = new_val;
                return;
            }
        }
        self.video_effects.push(VideoEffect::ColorCorrection {
            brightness: 0.0,
            contrast: 1.0,
            saturation: new_val,
        });
    }
    // 1. Scale
    pub fn get_scale(&self) -> f32 {
        for effect in &self.video_effects {
            if let VideoEffect::Transform { scale, .. } = effect {
                return *scale;
            }
        }
        1.0 // Default value
        // 0.8 // Fit for GPUI default scale setting, need to be adjusted with FFMPEG
    }

    pub fn set_scale(&mut self, new_val: f32) {
        for effect in &mut self.video_effects {
            if let VideoEffect::Transform { scale, .. } = effect {
                *scale = new_val;
                return;
            }
        }
        // If none is found, insert a new Transform effect
        self.video_effects.push(VideoEffect::Transform {
            scale: new_val,
            position_x: 0.0,
            position_y: 0.0,
            rotation_deg: 0.0,
        });
    }

    // 2. Position X
    pub fn get_pos_x(&self) -> f32 {
        for effect in &self.video_effects {
            if let VideoEffect::Transform { position_x, .. } = effect {
                return *position_x;
            }
        }
        0.0
    }

    pub fn set_pos_x(&mut self, new_val: f32) {
        for effect in &mut self.video_effects {
            if let VideoEffect::Transform { position_x, .. } = effect {
                *position_x = new_val;
                return;
            }
        }
        self.video_effects.push(VideoEffect::Transform {
            scale: 1.0,
            position_x: new_val,
            position_y: 0.0,
            rotation_deg: 0.0,
        });
    }

    // 3. Position Y
    pub fn get_pos_y(&self) -> f32 {
        for effect in &self.video_effects {
            if let VideoEffect::Transform { position_y, .. } = effect {
                return *position_y;
            }
        }
        0.0
    }

    pub fn set_pos_y(&mut self, new_val: f32) {
        for effect in &mut self.video_effects {
            if let VideoEffect::Transform { position_y, .. } = effect {
                *position_y = new_val;
                return;
            }
        }
        self.video_effects.push(VideoEffect::Transform {
            scale: 1.0,
            position_x: 0.0,
            position_y: new_val,
            rotation_deg: 0.0,
        });
    }

    pub fn get_rotation(&self) -> f32 {
        for effect in &self.video_effects {
            if let VideoEffect::Transform { rotation_deg, .. } = effect {
                return *rotation_deg;
            }
        }
        0.0
    }

    pub fn set_rotation(&mut self, new_val: f32) {
        let rotation = new_val.clamp(-180.0, 180.0);
        for effect in &mut self.video_effects {
            if let VideoEffect::Transform { rotation_deg, .. } = effect {
                *rotation_deg = rotation;
                return;
            }
        }
        self.video_effects.push(VideoEffect::Transform {
            scale: 1.0,
            position_x: 0.0,
            position_y: 0.0,
            rotation_deg: rotation,
        });
    }
    pub fn get_hsla_overlay(&self) -> (f32, f32, f32, f32) {
        for effect in &self.video_effects {
            if let VideoEffect::Tint {
                hue,
                saturation,
                lightness,
                alpha,
            } = effect
            {
                return (*hue, *saturation, *lightness, *alpha);
            }
        }
        (0.0, 0.0, 0.0, 0.0) // Default: no color overlay
    }

    // HSLA overlay setter (canonical)
    pub fn set_hsla_overlay(&mut self, h: f32, s: f32, l: f32, a: f32) {
        for effect in &mut self.video_effects {
            if let VideoEffect::Tint {
                hue,
                saturation,
                lightness,
                alpha,
            } = effect
            {
                *hue = h;
                *saturation = s;
                *lightness = l;
                *alpha = a;
                return;
            }
        }
        // Add one if none is found
        self.video_effects.push(VideoEffect::Tint {
            hue: h,
            saturation: s,
            lightness: l,
            alpha: a,
        });
    }

    // ===

    pub fn get_opacity(&self) -> f32 {
        for effect in &self.video_effects {
            if let VideoEffect::Opacity { alpha } = effect {
                return *alpha;
            }
        }
        1.0
    }

    pub fn set_opacity(&mut self, new_val: f32) {
        let alpha = new_val.clamp(0.0, 1.0);
        for effect in &mut self.video_effects {
            if let VideoEffect::Opacity { alpha: current } = effect {
                *current = alpha;
                return;
            }
        }
        self.video_effects.push(VideoEffect::Opacity { alpha });
    }

    pub fn get_blur_sigma(&self) -> f32 {
        for effect in &self.video_effects {
            if let VideoEffect::GaussianBlur { sigma } = effect {
                return *sigma;
            }
        }
        0.0
    }

    pub fn set_blur_sigma(&mut self, new_val: f32) {
        let sigma = new_val.clamp(0.0, 64.0);
        for effect in &mut self.video_effects {
            if let VideoEffect::GaussianBlur { sigma: current } = effect {
                *current = sigma;
                return;
            }
        }
        self.video_effects.push(VideoEffect::GaussianBlur { sigma });
    }

    pub fn base_color_blur_effects(&self) -> PerClipColorBlurEffects {
        PerClipColorBlurEffects {
            brightness: self.get_brightness(),
            contrast: self.get_contrast(),
            saturation: self.get_saturation(),
            blur_sigma: self.get_blur_sigma(),
        }
        .normalized()
    }

    fn legacy_local_mask(&self) -> (bool, f32, f32, f32, f32, f32) {
        for effect in &self.video_effects {
            if let VideoEffect::LocalMask {
                enabled,
                center_x,
                center_y,
                radius,
                feather,
                strength,
            } = effect
            {
                return (*enabled, *center_x, *center_y, *radius, *feather, *strength);
            }
        }
        let d = LocalMaskLayer::default();
        (
            d.enabled, d.center_x, d.center_y, d.radius, d.feather, d.strength,
        )
    }

    fn legacy_local_mask_adjust(&self) -> (f32, f32, f32, f32, f32) {
        for effect in &self.video_effects {
            if let VideoEffect::LocalMaskAdjust {
                brightness,
                contrast,
                saturation,
                opacity,
                blur_sigma,
            } = effect
            {
                return (*brightness, *contrast, *saturation, *opacity, *blur_sigma);
            }
        }
        let d = LocalMaskLayer::default();
        (
            d.brightness,
            d.contrast,
            d.saturation,
            d.opacity,
            d.blur_sigma,
        )
    }

    fn ensure_local_mask_layer(&mut self, layer_index: usize) -> usize {
        let index = layer_index.min(MAX_LOCAL_MASK_LAYERS.saturating_sub(1));
        if self.local_mask_layers.is_empty() {
            let (enabled, center_x, center_y, radius, feather, strength) = self.legacy_local_mask();
            let (brightness, contrast, saturation, opacity, blur_sigma) =
                self.legacy_local_mask_adjust();
            self.local_mask_layers.push(LocalMaskLayer {
                enabled,
                center_x,
                center_y,
                radius,
                feather,
                strength,
                brightness,
                contrast,
                saturation,
                opacity,
                blur_sigma,
            });
        }
        while self.local_mask_layers.len() <= index
            && self.local_mask_layers.len() < MAX_LOCAL_MASK_LAYERS
        {
            self.local_mask_layers.push(LocalMaskLayer::default());
        }
        index.min(self.local_mask_layers.len().saturating_sub(1))
    }

    pub fn local_mask_layer_count(&self) -> usize {
        self.local_mask_layers.len().clamp(1, MAX_LOCAL_MASK_LAYERS)
    }

    pub fn add_local_mask_layer(&mut self) -> Option<usize> {
        if self.local_mask_layers.len() >= MAX_LOCAL_MASK_LAYERS {
            return None;
        }
        if self.local_mask_layers.is_empty() {
            let _ = self.ensure_local_mask_layer(0);
        }
        self.local_mask_layers.push(LocalMaskLayer::default());
        Some(self.local_mask_layers.len().saturating_sub(1))
    }

    pub fn get_local_mask_layers(&self) -> Vec<LocalMaskLayer> {
        if self.local_mask_layers.is_empty() {
            let (enabled, center_x, center_y, radius, feather, strength) = self.legacy_local_mask();
            let (brightness, contrast, saturation, opacity, blur_sigma) =
                self.legacy_local_mask_adjust();
            return vec![LocalMaskLayer {
                enabled,
                center_x,
                center_y,
                radius,
                feather,
                strength,
                brightness,
                contrast,
                saturation,
                opacity,
                blur_sigma,
            }];
        }
        self.local_mask_layers
            .iter()
            .take(MAX_LOCAL_MASK_LAYERS)
            .cloned()
            .collect()
    }

    pub fn get_local_mask_layer(&self, layer_index: usize) -> (bool, f32, f32, f32, f32, f32) {
        if self.local_mask_layers.is_empty() {
            if layer_index == 0 {
                return self.legacy_local_mask();
            }
            let d = LocalMaskLayer::default();
            return (
                d.enabled, d.center_x, d.center_y, d.radius, d.feather, d.strength,
            );
        }
        let layer = self
            .local_mask_layers
            .get(layer_index.min(MAX_LOCAL_MASK_LAYERS.saturating_sub(1)))
            .unwrap_or_else(|| self.local_mask_layers.last().expect("at least one layer"));
        (
            layer.enabled,
            layer.center_x,
            layer.center_y,
            layer.radius,
            layer.feather,
            layer.strength,
        )
    }

    pub fn set_local_mask_layer(
        &mut self,
        layer_index: usize,
        enabled: bool,
        center_x: f32,
        center_y: f32,
        radius: f32,
        feather: f32,
        strength: f32,
    ) {
        let index = self.ensure_local_mask_layer(layer_index);
        if let Some(layer) = self.local_mask_layers.get_mut(index) {
            layer.enabled = enabled;
            layer.center_x = center_x.clamp(0.0, 1.0);
            layer.center_y = center_y.clamp(0.0, 1.0);
            layer.radius = radius.clamp(0.0, 1.0);
            layer.feather = feather.clamp(0.0, 1.0);
            layer.strength = strength.clamp(0.0, 1.0);
        }
    }

    pub fn get_local_mask_adjust_layer(&self, layer_index: usize) -> (f32, f32, f32, f32, f32) {
        if self.local_mask_layers.is_empty() {
            if layer_index == 0 {
                return self.legacy_local_mask_adjust();
            }
            let d = LocalMaskLayer::default();
            return (
                d.brightness,
                d.contrast,
                d.saturation,
                d.opacity,
                d.blur_sigma,
            );
        }
        let layer = self
            .local_mask_layers
            .get(layer_index.min(MAX_LOCAL_MASK_LAYERS.saturating_sub(1)))
            .unwrap_or_else(|| self.local_mask_layers.last().expect("at least one layer"));
        (
            layer.brightness,
            layer.contrast,
            layer.saturation,
            layer.opacity,
            layer.blur_sigma,
        )
    }

    pub fn set_local_mask_adjust_layer(
        &mut self,
        layer_index: usize,
        brightness: f32,
        contrast: f32,
        saturation: f32,
        opacity: f32,
        blur_sigma: f32,
    ) {
        let index = self.ensure_local_mask_layer(layer_index);
        if let Some(layer) = self.local_mask_layers.get_mut(index) {
            layer.brightness = brightness.clamp(-1.0, 1.0);
            layer.contrast = contrast.clamp(0.0, 2.0);
            layer.saturation = saturation.clamp(0.0, 2.0);
            layer.opacity = opacity.clamp(0.0, 1.0);
            layer.blur_sigma = blur_sigma.clamp(0.0, 64.0);
        }
    }

    pub fn get_fade(&self) -> (f32, f32) {
        for effect in &self.video_effects {
            if let VideoEffect::Fade { fade_in, fade_out } = effect {
                return (*fade_in, *fade_out);
            }
        }
        (0.0, 0.0)
    }

    pub fn get_fade_in(&self) -> f32 {
        self.get_fade().0
    }

    pub fn get_fade_out(&self) -> f32 {
        self.get_fade().1
    }

    pub fn set_fade_in(&mut self, new_val: f32) {
        let max = self.duration.as_secs_f32().max(0.0);
        let fade_in = new_val.clamp(0.0, max);
        for effect in &mut self.video_effects {
            if let VideoEffect::Fade {
                fade_in: current, ..
            } = effect
            {
                *current = fade_in;
                return;
            }
        }
        self.video_effects.push(VideoEffect::Fade {
            fade_in,
            fade_out: 0.0,
        });
    }

    pub fn set_fade_out(&mut self, new_val: f32) {
        let max = self.duration.as_secs_f32().max(0.0);
        let fade_out = new_val.clamp(0.0, max);
        for effect in &mut self.video_effects {
            if let VideoEffect::Fade {
                fade_out: current, ..
            } = effect
            {
                *current = fade_out;
                return;
            }
        }
        self.video_effects.push(VideoEffect::Fade {
            fade_in: 0.0,
            fade_out,
        });
    }

    pub fn sample_fade_factor(&self, t: Duration) -> f32 {
        let (fade_in_raw, fade_out_raw) = self.get_fade();
        transitions::sample_fade_factor(self.duration, fade_in_raw, fade_out_raw, t)
    }

    pub fn get_dissolve(&self) -> (f32, f32) {
        for effect in &self.video_effects {
            if let VideoEffect::Dissolve {
                dissolve_in,
                dissolve_out,
            } = effect
            {
                return (*dissolve_in, *dissolve_out);
            }
        }
        (0.0, 0.0)
    }

    pub fn get_dissolve_in(&self) -> f32 {
        self.get_dissolve().0
    }

    pub fn get_dissolve_out(&self) -> f32 {
        self.get_dissolve().1
    }

    pub fn set_dissolve_in(&mut self, new_val: f32) {
        let max = self.duration.as_secs_f32().max(0.0);
        let dissolve_in = new_val.clamp(0.0, max);
        for effect in &mut self.video_effects {
            if let VideoEffect::Dissolve {
                dissolve_in: current,
                ..
            } = effect
            {
                *current = dissolve_in;
                return;
            }
        }
        self.video_effects.push(VideoEffect::Dissolve {
            dissolve_in,
            dissolve_out: 0.0,
        });
    }

    pub fn set_dissolve_out(&mut self, new_val: f32) {
        let max = self.duration.as_secs_f32().max(0.0);
        let dissolve_out = new_val.clamp(0.0, max);
        for effect in &mut self.video_effects {
            if let VideoEffect::Dissolve {
                dissolve_out: current,
                ..
            } = effect
            {
                *current = dissolve_out;
                return;
            }
        }
        self.video_effects.push(VideoEffect::Dissolve {
            dissolve_in: 0.0,
            dissolve_out,
        });
    }

    pub fn sample_dissolve_factor(&self, t: Duration) -> f32 {
        let (dissolve_in_raw, dissolve_out_raw) = self.get_dissolve();
        transitions::sample_dissolve_factor(self.duration, dissolve_in_raw, dissolve_out_raw, t)
    }

    pub fn get_slide(&self) -> (SlideDirection, SlideDirection, f32, f32) {
        for effect in &self.video_effects {
            if let VideoEffect::Slide {
                in_direction,
                out_direction,
                slide_in,
                slide_out,
            } = effect
            {
                return (*in_direction, *out_direction, *slide_in, *slide_out);
            }
        }
        (SlideDirection::Right, SlideDirection::Left, 0.0, 0.0)
    }

    pub fn set_slide(
        &mut self,
        in_direction: SlideDirection,
        out_direction: SlideDirection,
        slide_in: f32,
        slide_out: f32,
    ) {
        let total = self.duration.as_secs_f32().max(0.0);
        let slide_in = slide_in.clamp(0.0, total);
        let slide_out = slide_out.clamp(0.0, total);
        for effect in &mut self.video_effects {
            if let VideoEffect::Slide {
                in_direction: in_dir,
                out_direction: out_dir,
                slide_in: in_dur,
                slide_out: out_dur,
            } = effect
            {
                *in_dir = in_direction;
                *out_dir = out_direction;
                *in_dur = slide_in;
                *out_dur = slide_out;
                return;
            }
        }
        self.video_effects.push(VideoEffect::Slide {
            in_direction,
            out_direction,
            slide_in,
            slide_out,
        });
    }

    pub fn sample_slide_offset(&self, t: Duration) -> (f32, f32) {
        let (in_dir, out_dir, slide_in_raw, slide_out_raw) = self.get_slide();
        let dir_vec = |dir: SlideDirection| match dir {
            SlideDirection::Left => (-1.0, 0.0),
            SlideDirection::Right => (1.0, 0.0),
            SlideDirection::Up => (0.0, -1.0),
            SlideDirection::Down => (0.0, 1.0),
        };
        transitions::sample_slide_offset(
            self.duration,
            dir_vec(in_dir),
            dir_vec(out_dir),
            slide_in_raw,
            slide_out_raw,
            t,
        )
    }

    pub fn get_zoom(&self) -> (f32, f32, f32) {
        for effect in &self.video_effects {
            if let VideoEffect::Zoom {
                zoom_in,
                zoom_out,
                zoom_amount,
            } = effect
            {
                return (*zoom_in, *zoom_out, *zoom_amount);
            }
        }
        (0.0, 0.0, 1.1)
    }

    pub fn set_zoom(&mut self, zoom_in: f32, zoom_out: f32, zoom_amount: f32) {
        let total = self.duration.as_secs_f32().max(0.0);
        let zoom_in = zoom_in.clamp(0.0, total);
        let zoom_out = zoom_out.clamp(0.0, total);
        let zoom_amount = zoom_amount.clamp(0.1, 4.0);
        for effect in &mut self.video_effects {
            if let VideoEffect::Zoom {
                zoom_in: in_dur,
                zoom_out: out_dur,
                zoom_amount: amount,
            } = effect
            {
                *in_dur = zoom_in;
                *out_dur = zoom_out;
                *amount = zoom_amount;
                return;
            }
        }
        self.video_effects.push(VideoEffect::Zoom {
            zoom_in,
            zoom_out,
            zoom_amount,
        });
    }

    pub fn sample_zoom_factor(&self, t: Duration) -> f32 {
        let (zoom_in_raw, zoom_out_raw, zoom_amount) = self.get_zoom();
        transitions::sample_zoom_factor(self.duration, zoom_in_raw, zoom_out_raw, zoom_amount, t)
    }

    pub fn get_shock_zoom(&self) -> (f32, f32, f32) {
        for effect in &self.video_effects {
            if let VideoEffect::ShockZoom {
                shock_in,
                shock_out,
                shock_amount,
            } = effect
            {
                return (*shock_in, *shock_out, *shock_amount);
            }
        }
        (0.0, 0.0, 1.2)
    }

    pub fn set_shock_zoom(&mut self, shock_in: f32, shock_out: f32, shock_amount: f32) {
        let total = self.duration.as_secs_f32().max(0.0);
        let shock_in = shock_in.clamp(0.0, total);
        let shock_out = shock_out.clamp(0.0, total);
        let shock_amount = shock_amount.clamp(0.1, 4.0);
        for effect in &mut self.video_effects {
            if let VideoEffect::ShockZoom {
                shock_in: in_dur,
                shock_out: out_dur,
                shock_amount: amount,
            } = effect
            {
                *in_dur = shock_in;
                *out_dur = shock_out;
                *amount = shock_amount;
                return;
            }
        }
        self.video_effects.push(VideoEffect::ShockZoom {
            shock_in,
            shock_out,
            shock_amount,
        });
    }

    pub fn sample_shock_zoom_factor(&self, t: Duration) -> f32 {
        let (shock_in_raw, shock_out_raw, shock_amount) = self.get_shock_zoom();
        transitions::sample_shock_zoom_factor(
            self.duration,
            shock_in_raw,
            shock_out_raw,
            shock_amount,
            t,
        )
    }

    // ===
}

#[derive(Clone, Debug)]
pub struct TimelineState {
    pub v1_clips: Vec<Clip>,
    pub audio_tracks: Vec<AudioTrack>,
    pub video_tracks: Vec<VideoTrack>,
    pub subtitle_tracks: Vec<SubtitleTrack>,
    pub semantic_clips: Vec<SemanticClip>,
    pub track_visibility: HashMap<String, bool>,
    pub track_lock: HashMap<String, bool>,
    pub track_mute: HashMap<String, bool>,
    pub subtitle_groups: HashMap<u64, SubtitleGroupTransform>,
    pub next_subtitle_group_id: u64,
    pub playhead: Duration,
    pub semantic_mark_start: Option<Duration>,
    pub layer_color_blur_effects: LayerColorBlurEffects,
    pub layer_effect_clips: Vec<LayerEffectClip>,
    pub selected_layer_effect_clip_id: Option<u64>,
    pub selected_semantic_clip_id: Option<u64>,
    pub media_pool_state: Option<MediaPoolUndoState>,
}

#[derive(Clone, Debug)]
pub struct MediaPoolUndoState {
    pub active_source_path: String,
    pub active_source_name: String,
    pub active_source_duration: Duration,
    pub media_pool: Vec<MediaPoolItem>,
    pub pending_media_pool_path: Option<String>,
}

pub type LayerEffectClip = motionloom::LayerEffectClip;

// [Added] Define the single audio-track structure
#[derive(Debug, Clone)]
pub struct AudioTrack {
    pub name: String,
    pub clips: Vec<Clip>,
    pub gain_db: f32,
}

impl AudioTrack {
    pub fn new(name: String) -> Self {
        Self {
            name,
            clips: Vec::new(),
            gain_db: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VideoTrack {
    pub name: String,
    pub clips: Vec<Clip>,
}
impl VideoTrack {
    pub fn new(name: String) -> Self {
        Self {
            name,
            clips: Vec::new(),
        }
    }
}

// 2. Define the undo manager
#[derive(Clone, Debug)]
pub struct UndoManager {
    past: Vec<TimelineState>,
    future: Vec<TimelineState>,
}

impl UndoManager {
    pub fn new() -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
        }
    }

    pub fn push(&mut self, state: TimelineState) {
        self.future.clear(); // Once a new action occurs, redo history becomes invalid
        self.past.push(state);

        // Limit the number of steps (for example, 50) to avoid unbounded memory growth
        if self.past.len() > 50 {
            self.past.remove(0);
        }
    }

    pub fn undo(&mut self, current: TimelineState) -> Option<TimelineState> {
        if let Some(prev) = self.past.pop() {
            self.future.push(current);
            return Some(prev);
        }
        None
    }

    pub fn redo(&mut self, current: TimelineState) -> Option<TimelineState> {
        if let Some(next) = self.future.pop() {
            self.past.push(current);
            return Some(next);
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackType {
    V1,                  // Primary track
    Audio(usize),        // Audio track (index maps to audio_tracks)
    VideoOverlay(usize), // V2+ track (index maps to video_tracks)
    Subtitle(usize),     // Subtitle track (index maps to subtitle_tracks)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionType {
    Fade,
    Dissolve,
    Slide,
    Zoom,
    ShockZoom,
}

#[derive(Clone, Debug)]
pub enum PendingTrimTrack {
    V1,
}

#[derive(Clone, Debug)]
pub struct PendingTrimToFit {
    pub left_id: u64,
    pub right_id: u64,
    pub requested: f32,
    pub track: PendingTrimTrack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportColorMode {
    Fast,
    Hybrid,
    Exact,
}

impl ExportColorMode {
    pub fn label(self) -> &'static str {
        match self {
            ExportColorMode::Fast => "Color: Fast",
            ExportColorMode::Hybrid => "Color: Hybrid",
            ExportColorMode::Exact => "Color: Exact",
        }
    }

    pub fn next(self) -> Self {
        match self {
            ExportColorMode::Fast => ExportColorMode::Hybrid,
            ExportColorMode::Hybrid => ExportColorMode::Exact,
            ExportColorMode::Exact => ExportColorMode::Fast,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewFps {
    Fps24,
    Fps25,
    Fps30,
    Fps48,
    Fps50,
    Fps60,
    Fps72,
    Fps90,
    Fps100,
    Fps120,
    Fps144,
}

impl PreviewFps {
    pub fn from_value(value: u32) -> Option<Self> {
        match value {
            24 => Some(PreviewFps::Fps24),
            25 => Some(PreviewFps::Fps25),
            30 => Some(PreviewFps::Fps30),
            48 => Some(PreviewFps::Fps48),
            50 => Some(PreviewFps::Fps50),
            60 => Some(PreviewFps::Fps60),
            72 => Some(PreviewFps::Fps72),
            90 => Some(PreviewFps::Fps90),
            100 => Some(PreviewFps::Fps100),
            120 => Some(PreviewFps::Fps120),
            144 => Some(PreviewFps::Fps144),
            _ => None,
        }
    }

    pub fn value(self) -> u32 {
        match self {
            PreviewFps::Fps24 => 24,
            PreviewFps::Fps25 => 25,
            PreviewFps::Fps30 => 30,
            PreviewFps::Fps48 => 48,
            PreviewFps::Fps50 => 50,
            PreviewFps::Fps60 => 60,
            PreviewFps::Fps72 => 72,
            PreviewFps::Fps90 => 90,
            PreviewFps::Fps100 => 100,
            PreviewFps::Fps120 => 120,
            PreviewFps::Fps144 => 144,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewQuality {
    Full,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyRenderMode {
    Nv12Surface,
    BgraImage,
}

impl ProxyRenderMode {
    pub fn label(self) -> &'static str {
        match self {
            ProxyRenderMode::Nv12Surface => "NV12",
            ProxyRenderMode::BgraImage => "BGRA",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            ProxyRenderMode::Nv12Surface => ProxyRenderMode::BgraImage,
            ProxyRenderMode::BgraImage => ProxyRenderMode::Nv12Surface,
        }
    }

    pub fn prefer_surface(self) -> bool {
        matches!(self, ProxyRenderMode::Nv12Surface)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacPreviewRenderMode {
    // Default macOS mode: keep NV12 surface path when possible.
    HybridNv12,
    // Force all visual preview clips through BGRA decode/image path.
    FullBgra,
}

impl MacPreviewRenderMode {
    pub fn label(self) -> &'static str {
        match self {
            MacPreviewRenderMode::HybridNv12 => "Hybrid NV12",
            MacPreviewRenderMode::FullBgra => "Full BGRA",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            MacPreviewRenderMode::HybridNv12 => MacPreviewRenderMode::FullBgra,
            MacPreviewRenderMode::FullBgra => MacPreviewRenderMode::HybridNv12,
        }
    }
}

impl PreviewQuality {
    pub fn label(self) -> &'static str {
        match self {
            PreviewQuality::Full => "Preview Full",
            PreviewQuality::High => "Preview Medium",
            PreviewQuality::Medium => "Preview Low",
            PreviewQuality::Low => "Preview Super Low",
        }
    }

    pub fn max_dim(self) -> Option<u32> {
        match self {
            PreviewQuality::Full => None,
            PreviewQuality::High => Some(480),
            PreviewQuality::Medium => Some(360),
            PreviewQuality::Low => Some(144),
        }
    }

    pub fn pixelate(self) -> bool {
        matches!(self, PreviewQuality::Low)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V1MoveMode {
    Magnetic,
    Free,
}

impl V1MoveMode {
    pub fn label(self) -> &'static str {
        match self {
            V1MoveMode::Magnetic => "V1: Magnetic",
            V1MoveMode::Free => "V1: Free",
        }
    }

    pub fn next(self) -> Self {
        match self {
            V1MoveMode::Magnetic => V1MoveMode::Free,
            V1MoveMode::Free => V1MoveMode::Magnetic,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProxyDeleteReport {
    pub deleted_files: usize,
    pub removed_jobs: usize,
    pub blocked_active_jobs: usize,
}

#[derive(Debug, Clone)]
pub struct MediaPoolItem {
    pub path: String,
    pub name: String,
    pub duration: Duration,
    pub preview_jpeg_base64: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MediaPoolDragState {
    pub path: String,
    pub name: String,
    pub cursor_x: f32,
    pub cursor_y: f32,
}

#[derive(Debug, Clone)]
pub struct MediaPoolContextMenuState {
    pub path: String,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaPoolUiEvent {
    StateChanged,
    DragCursorChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackUiEvent {
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipKeyframeChannel {
    Scale,
    Rotation,
    PosX,
    PosY,
    Brightness,
    Contrast,
    Saturation,
    Opacity,
    Blur,
}

#[derive(Debug, Clone)]
pub struct GlobalState {
    // --- System Configuration (Global) ---
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    pub gstreamer_path: String,
    pub gstreamer_available: bool,
    pub media_dependency: MediaDependencyStatus,
    pub show_media_dependency_modal: bool,
    pub show_preview_memory_budget_modal: bool,
    pub preview_memory_budget_mb: Option<usize>,
    pub canvas_w: f32,
    pub canvas_h: f32,
    pub layer_color_blur_effects: LayerColorBlurEffects,

    // Transport
    pub is_playing: bool,
    pub playhead: Duration,

    // Tools
    pub active_tool: ActiveTool,
    pub active_page: AppPage,

    // --- Source Monitor Info ---
    pub active_source_path: String,
    pub active_source_name: String,
    pub active_source_duration: Duration,
    pub media_pool: Vec<MediaPoolItem>,
    pub pending_media_pool_path: Option<String>,
    pub media_pool_drag: Option<MediaPoolDragState>,
    pub media_pool_context_menu: Option<MediaPoolContextMenuState>,
    pub layer_effect_clips: Vec<LayerEffectClip>,
    pub selected_layer_effect_clip_id: Option<u64>,
    pub selected_semantic_clip_id: Option<u64>,
    pub project_file_path: Option<PathBuf>,

    // --- Timeline Tracks ---
    pub v1_clips: Vec<Clip>,

    // Multi-track audio support
    pub audio_tracks: Vec<AudioTrack>,
    pub video_tracks: Vec<VideoTrack>,
    pub subtitle_tracks: Vec<SubtitleTrack>,
    pub semantic_clips: Vec<SemanticClip>,
    // ACP-editable per-track states keyed by:
    // v1, audio:<idx>, video:<idx>, subtitle:<idx>
    pub track_visibility: HashMap<String, bool>,
    pub track_lock: HashMap<String, bool>,
    pub track_mute: HashMap<String, bool>,
    pub subtitle_groups: HashMap<u64, SubtitleGroupTransform>,
    // undo logic
    pub undo_manager: UndoManager,

    pub selected_clip_id: Option<u64>,
    pub selected_subtitle_id: Option<u64>,
    pub selected_clip_ids: Vec<u64>,
    pub selected_subtitle_ids: Vec<u64>,
    pub active_local_mask_layer: usize,
    pub next_subtitle_group_id: u64,

    next_clip_id: u64,
    timeline_edit_token: u64,

    // Export status
    pub export_in_progress: bool,
    pub export_last_error: Option<String>,
    pub export_last_out_path: Option<String>,
    pub export_progress_ratio: f32,
    pub export_progress_rendered: Duration,
    pub export_progress_total: Duration,
    pub export_eta: Option<Duration>,
    pub export_color_mode: ExportColorMode,
    pub preview_fps: PreviewFps,
    // Runtime diagnostic: effective decoded frame-arrival FPS seen by preview.
    pub preview_video_input_fps: f32,
    // Runtime diagnostic: estimated on-screen present FPS and present drops.
    pub preview_present_fps: f32,
    pub preview_present_dropped_frames: u64,
    pub preview_quality: PreviewQuality,
    pub proxy_render_mode_high: ProxyRenderMode,
    pub proxy_render_mode_medium: ProxyRenderMode,
    pub proxy_render_mode_low: ProxyRenderMode,
    pub mac_preview_render_mode: MacPreviewRenderMode,
    pub v1_move_mode: V1MoveMode,
    pub inspector_expanded: bool,
    pub pending_transition: Option<TransitionType>,
    pub ui_notice: Option<String>,
    pub semantic_mark_start: Option<Duration>,
    pub ai_chat_messages: Vec<AiChatMessage>,
    // Shared MotionLoom VFX graph script for ACP <-> VFX page editing.
    pub motionloom_scene_script: String,
    pub motionloom_scene_script_revision: u64,
    pub motionloom_scene_apply_revision: u64,
    pub motionloom_scene_render_mode: Option<String>,
    pub motionloom_scene_render_revision: u64,
    /// Silence preview modal: shown when ACP inspect-intent returns silence candidates.
    pub silence_preview_modal: Option<SilencePreviewModalState>,
    pub pending_trim_to_fit: Option<PendingTrimToFit>,
    pub proxy_entries: HashMap<String, ProxyEntry>,
    pub proxy_queue: VecDeque<ProxyJob>,
    pub proxy_active: Option<ProxyJob>,
    pub waveform_entries: HashMap<String, WaveformEntry>,
    pub waveform_queue: VecDeque<WaveformJob>,
    pub waveform_active: Option<WaveformJob>,
    // Run legacy V1/A1 link backfill only once to avoid re-linking clips users explicitly unlinked.
    primary_av_links_repair_done: bool,
}

impl EventEmitter<MediaPoolUiEvent> for GlobalState {}
impl EventEmitter<PlaybackUiEvent> for GlobalState {}

impl Default for GlobalState {
    fn default() -> Self {
        Self {
            ffmpeg_path: "ffmpeg".to_string(),
            ffprobe_path: "ffprobe".to_string(),
            gstreamer_path: "gst-launch-1.0".to_string(),
            gstreamer_available: false,
            media_dependency: MediaDependencyStatus::default(),
            show_media_dependency_modal: false,
            show_preview_memory_budget_modal: false,
            preview_memory_budget_mb: None,
            canvas_w: 1920.0,
            canvas_h: 1080.0,
            layer_color_blur_effects: LayerColorBlurEffects::default(),

            is_playing: false,
            playhead: Duration::ZERO,
            active_tool: ActiveTool::Select,
            active_page: AppPage::Editor,

            active_source_path: String::new(),
            active_source_name: "No Source Loaded".to_string(),
            active_source_duration: Duration::ZERO,
            media_pool: Vec::new(),
            pending_media_pool_path: None,
            media_pool_drag: None,
            media_pool_context_menu: None,
            layer_effect_clips: Vec::new(),
            selected_layer_effect_clip_id: None,
            selected_semantic_clip_id: None,
            project_file_path: None,

            v1_clips: Vec::new(),
            // Keep A1 reserved for main-track linked audio by default.
            audio_tracks: vec![AudioTrack::new("A1".to_string())],
            // V2, V3 to more (not V1)
            video_tracks: Vec::new(),
            subtitle_tracks: Vec::new(),
            semantic_clips: Vec::new(),
            track_visibility: HashMap::new(),
            track_lock: HashMap::new(),
            track_mute: HashMap::new(),
            subtitle_groups: HashMap::new(),

            //  Initialize Undo Manager here
            undo_manager: UndoManager::new(),

            selected_clip_id: None,
            selected_subtitle_id: None,
            selected_clip_ids: Vec::new(),
            selected_subtitle_ids: Vec::new(),
            active_local_mask_layer: 0,
            next_subtitle_group_id: 1,

            next_clip_id: 1,
            timeline_edit_token: 0,

            export_in_progress: false,
            export_last_error: None,
            export_last_out_path: None,
            export_progress_ratio: 0.0,
            export_progress_rendered: Duration::ZERO,
            export_progress_total: Duration::ZERO,
            export_eta: None,
            export_color_mode: ExportColorMode::Hybrid,
            preview_fps: PreviewFps::Fps60,
            preview_video_input_fps: 0.0,
            preview_present_fps: 0.0,
            preview_present_dropped_frames: 0,
            preview_quality: PreviewQuality::Full,
            proxy_render_mode_high: ProxyRenderMode::Nv12Surface,
            proxy_render_mode_medium: ProxyRenderMode::Nv12Surface,
            proxy_render_mode_low: ProxyRenderMode::Nv12Surface,
            mac_preview_render_mode: MacPreviewRenderMode::HybridNv12,
            v1_move_mode: V1MoveMode::Magnetic,
            inspector_expanded: false,
            pending_transition: None,
            ui_notice: None,
            semantic_mark_start: None,
            ai_chat_messages: Vec::new(),
            motionloom_scene_script: String::new(),
            motionloom_scene_script_revision: 0,
            motionloom_scene_apply_revision: 0,
            motionloom_scene_render_mode: None,
            motionloom_scene_render_revision: 0,
            silence_preview_modal: None,
            pending_trim_to_fit: None,
            proxy_entries: HashMap::new(),
            proxy_queue: VecDeque::new(),
            proxy_active: None,
            waveform_entries: HashMap::new(),
            waveform_queue: VecDeque::new(),
            waveform_active: None,
            primary_av_links_repair_done: false,
        }
    }
}

impl GlobalState {
    pub fn motionloom_scene_script(&self) -> &str {
        &self.motionloom_scene_script
    }

    pub fn motionloom_scene_script_revision(&self) -> u64 {
        self.motionloom_scene_script_revision
    }

    pub fn motionloom_scene_apply_revision(&self) -> u64 {
        self.motionloom_scene_apply_revision
    }

    pub fn motionloom_scene_render_revision(&self) -> u64 {
        self.motionloom_scene_render_revision
    }

    pub fn motionloom_scene_render_mode(&self) -> Option<&str> {
        self.motionloom_scene_render_mode.as_deref()
    }

    pub fn set_motionloom_scene_script(&mut self, script: String, apply_now: bool) -> (bool, bool) {
        let mut updated = false;
        if self.motionloom_scene_script != script {
            self.motionloom_scene_script = script;
            self.motionloom_scene_script_revision =
                self.motionloom_scene_script_revision.saturating_add(1);
            updated = true;
        }

        let mut apply_requested = false;
        if apply_now {
            self.motionloom_scene_apply_revision =
                self.motionloom_scene_apply_revision.saturating_add(1);
            apply_requested = true;
        }

        (updated, apply_requested)
    }

    pub fn request_motionloom_scene_render(&mut self, mode: &str) -> Result<u64, String> {
        let normalized = mode.trim().to_ascii_lowercase();
        let canonical = match normalized.as_str() {
            "gpu" | "gpu_render" | "gpu_h264" => "gpu",
            "gpu_prores" | "gpu-prores" | "prores_gpu" => "gpu_prores",
            "compatibility_cpu" | "compatibility-cpu" | "cpu" | "cpu_render" => "compatibility_cpu",
            other => {
                return Err(format!(
                    "unsupported motionloom render mode `{other}`. Use `gpu`, `gpu_prores`, or `compatibility_cpu`."
                ));
            }
        };
        self.motionloom_scene_render_mode = Some(canonical.to_string());
        self.motionloom_scene_render_revision =
            self.motionloom_scene_render_revision.saturating_add(1);
        Ok(self.motionloom_scene_render_revision)
    }

    // Ensure the first audio lane exists so V1-linked audio can always be inserted on A1.
    fn ensure_primary_audio_track(&mut self) {
        // Reuse an existing A1 if present, otherwise insert a dedicated A1 lane at index 0.
        if let Some(a1_index) = self
            .audio_tracks
            .iter()
            .position(|track| track.name.trim().eq_ignore_ascii_case("A1"))
        {
            if a1_index != 0 {
                let a1_track = self.audio_tracks.remove(a1_index);
                self.audio_tracks.insert(0, a1_track);
            }
            return;
        }

        self.audio_tracks
            .insert(0, AudioTrack::new("A1".to_string()));
    }

    // Pick the next visible audio lane number so new lanes follow A1, A2, A3... without collisions.
    fn next_audio_track_number(&self) -> usize {
        self.audio_tracks
            .iter()
            .filter_map(|track| {
                track
                    .name
                    .trim()
                    .strip_prefix('A')
                    .and_then(|num| num.parse::<usize>().ok())
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    // Allocate one stable link id by scanning existing clips so persisted projects remain collision-safe.
    fn allocate_next_link_group_id(&self) -> u64 {
        self.v1_clips
            .iter()
            .chain(self.audio_tracks.iter().flat_map(|t| t.clips.iter()))
            .chain(self.video_tracks.iter().flat_map(|t| t.clips.iter()))
            .filter_map(|clip| clip.link_group_id)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    // Resolve link-group id for any clip id across V1, audio, and overlay video tracks.
    fn clip_link_group_id(&self, clip_id: u64) -> Option<u64> {
        self.v1_clips
            .iter()
            .chain(
                self.audio_tracks
                    .iter()
                    .flat_map(|track| track.clips.iter()),
            )
            .chain(
                self.video_tracks
                    .iter()
                    .flat_map(|track| track.clips.iter()),
            )
            .find(|clip| clip.id == clip_id)
            .and_then(|clip| clip.link_group_id)
    }

    // Return one V1 start anchor per link group so non-V1 members can follow the same delta.
    fn v1_group_anchor_starts(&self) -> HashMap<u64, Duration> {
        let mut out = HashMap::new();
        for clip in &self.v1_clips {
            if let Some(link_group_id) = clip.link_group_id {
                out.entry(link_group_id).or_insert(clip.start);
            }
        }
        out
    }

    // Check whether a link group contains any V1 clip.
    fn link_group_has_v1(&self, link_group_id: u64) -> bool {
        self.v1_clips
            .iter()
            .any(|clip| clip.link_group_id == Some(link_group_id))
    }

    // Return the left-most V1 start in a link group, if the group has V1 members.
    fn link_group_min_v1_start_sec(&self, link_group_id: u64) -> Option<f64> {
        self.v1_clips
            .iter()
            .filter(|clip| clip.link_group_id == Some(link_group_id))
            .map(|clip| clip.start.as_secs_f64())
            .reduce(f64::min)
    }

    // Shift all clips in one link group by the same delta, optionally skipping one actively moved clip.
    fn sync_linked_group_delta(
        &mut self,
        link_group_id: u64,
        requested_delta_sec: f64,
        exclude_clip_id: Option<u64>,
    ) {
        if requested_delta_sec.abs() < f64::EPSILON {
            return;
        }

        let exclude_clip_id = exclude_clip_id.unwrap_or(u64::MAX);
        let mut min_start_sec = f64::INFINITY;
        for clip in &self.v1_clips {
            if clip.id != exclude_clip_id && clip.link_group_id == Some(link_group_id) {
                min_start_sec = min_start_sec.min(clip.start.as_secs_f64());
            }
        }
        for track in &self.audio_tracks {
            for clip in &track.clips {
                if clip.id != exclude_clip_id && clip.link_group_id == Some(link_group_id) {
                    min_start_sec = min_start_sec.min(clip.start.as_secs_f64());
                }
            }
        }
        for track in &self.video_tracks {
            for clip in &track.clips {
                if clip.id != exclude_clip_id && clip.link_group_id == Some(link_group_id) {
                    min_start_sec = min_start_sec.min(clip.start.as_secs_f64());
                }
            }
        }
        if !min_start_sec.is_finite() {
            return;
        }

        let applied_delta_sec = requested_delta_sec.max(-min_start_sec);
        if applied_delta_sec.abs() < f64::EPSILON {
            return;
        }

        for clip in &mut self.v1_clips {
            if clip.id != exclude_clip_id && clip.link_group_id == Some(link_group_id) {
                let next = (clip.start.as_secs_f64() + applied_delta_sec).max(0.0);
                clip.start = Duration::from_secs_f64(next);
            }
        }
        self.v1_clips.sort_by_key(|clip| clip.start);

        for track in &mut self.audio_tracks {
            let mut changed = false;
            for clip in &mut track.clips {
                if clip.id != exclude_clip_id && clip.link_group_id == Some(link_group_id) {
                    let next = (clip.start.as_secs_f64() + applied_delta_sec).max(0.0);
                    clip.start = Duration::from_secs_f64(next);
                    changed = true;
                }
            }
            if changed {
                track.clips.sort_by_key(|clip| clip.start);
            }
        }

        for track in &mut self.video_tracks {
            let mut changed = false;
            for clip in &mut track.clips {
                if clip.id != exclude_clip_id && clip.link_group_id == Some(link_group_id) {
                    let next = (clip.start.as_secs_f64() + applied_delta_sec).max(0.0);
                    clip.start = Duration::from_secs_f64(next);
                    changed = true;
                }
            }
            if changed {
                track.clips.sort_by_key(|clip| clip.start);
            }
        }
    }

    // Apply the same V1 movement delta to linked non-V1 clips so offsets are preserved.
    fn sync_linked_tracks_from_v1_deltas(&mut self, old_v1_group_starts: &HashMap<u64, Duration>) {
        if old_v1_group_starts.is_empty() {
            return;
        }

        let new_v1_group_starts = self.v1_group_anchor_starts();
        let mut group_delta_sec = HashMap::new();
        for (link_group_id, old_start) in old_v1_group_starts {
            if let Some(new_start) = new_v1_group_starts.get(link_group_id) {
                let delta = new_start.as_secs_f64() - old_start.as_secs_f64();
                if delta.abs() > f64::EPSILON {
                    group_delta_sec.insert(*link_group_id, delta);
                }
            }
        }
        if group_delta_sec.is_empty() {
            return;
        }

        for track in &mut self.audio_tracks {
            let mut changed = false;
            for clip in &mut track.clips {
                if let Some(link_group_id) = clip.link_group_id
                    && let Some(delta_sec) = group_delta_sec.get(&link_group_id)
                {
                    let next = (clip.start.as_secs_f64() + *delta_sec).max(0.0);
                    clip.start = Duration::from_secs_f64(next);
                    changed = true;
                }
            }
            if changed {
                track.clips.sort_by_key(|clip| clip.start);
            }
        }

        for track in &mut self.video_tracks {
            let mut changed = false;
            for clip in &mut track.clips {
                if let Some(link_group_id) = clip.link_group_id
                    && let Some(delta_sec) = group_delta_sec.get(&link_group_id)
                {
                    let next = (clip.start.as_secs_f64() + *delta_sec).max(0.0);
                    clip.start = Duration::from_secs_f64(next);
                    changed = true;
                }
            }
            if changed {
                track.clips.sort_by_key(|clip| clip.start);
            }
        }
    }

    // Expand selected clip ids so linked companions (A/V pairs) are edited together by default.
    fn expand_clip_ids_by_link_group(
        &self,
        selected: &std::collections::HashSet<u64>,
    ) -> std::collections::HashSet<u64> {
        let mut link_groups = std::collections::HashSet::new();
        for clip in self
            .v1_clips
            .iter()
            .chain(
                self.audio_tracks
                    .iter()
                    .flat_map(|track| track.clips.iter()),
            )
            .chain(
                self.video_tracks
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
        for clip in self
            .v1_clips
            .iter()
            .chain(
                self.audio_tracks
                    .iter()
                    .flat_map(|track| track.clips.iter()),
            )
            .chain(
                self.video_tracks
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

    // Backfill missing V1<->A1 link ids for legacy timelines so linked edit actions work consistently.
    fn repair_missing_primary_av_links(&mut self) {
        // Guard the migration so explicit unlink operations stay respected after first pass.
        if self.primary_av_links_repair_done {
            return;
        }

        let Some(a1) = self.audio_tracks.first() else {
            return;
        };
        if self.v1_clips.is_empty() || a1.clips.is_empty() {
            return;
        }

        // Build candidate pair list first, then apply mutations in a second pass to avoid borrow conflicts.
        let mut used_audio_indices = std::collections::HashSet::new();
        for (idx, clip) in a1.clips.iter().enumerate() {
            if clip.link_group_id.is_some() {
                used_audio_indices.insert(idx);
            }
        }

        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for (v_idx, v_clip) in self.v1_clips.iter().enumerate() {
            if v_clip.link_group_id.is_some() {
                continue;
            }

            let expected_audio_label = format!("(Audio) {}", v_clip.label);
            let mut best_audio_idx: Option<usize> = None;
            let mut best_start_delta = f64::INFINITY;

            for (a_idx, a_clip) in a1.clips.iter().enumerate() {
                if used_audio_indices.contains(&a_idx) || a_clip.link_group_id.is_some() {
                    continue;
                }
                if a_clip.label != expected_audio_label {
                    continue;
                }
                if a_clip.file_path != v_clip.file_path {
                    continue;
                }
                if a_clip.source_in != v_clip.source_in {
                    continue;
                }
                if a_clip.duration != v_clip.duration {
                    continue;
                }

                let delta = (a_clip.start.as_secs_f64() - v_clip.start.as_secs_f64()).abs();
                if delta < best_start_delta {
                    best_start_delta = delta;
                    best_audio_idx = Some(a_idx);
                }
            }

            if let Some(a_idx) = best_audio_idx {
                used_audio_indices.insert(a_idx);
                pairs.push((v_idx, a_idx));
            }
        }

        for (v_idx, a_idx) in pairs {
            let link_group_id = self.allocate_next_link_group_id();
            self.v1_clips[v_idx].link_group_id = Some(link_group_id);
            if let Some(a1_mut) = self.audio_tracks.get_mut(0)
                && let Some(a_clip) = a1_mut.clips.get_mut(a_idx)
            {
                a_clip.link_group_id = Some(link_group_id);
            }
        }

        // Mark migration complete even when no pairs were found to prevent accidental future re-linking.
        self.primary_av_links_repair_done = true;
    }

    // --- Configuration Methods ---
    pub fn apply_media_dependency_status(
        &mut self,
        status: MediaDependencyStatus,
        open_modal_if_missing: bool,
    ) {
        self.ffmpeg_path = status.ffmpeg_command.clone();
        self.ffprobe_path = status.ffprobe_command.clone();
        self.media_dependency = status;
        if open_modal_if_missing
            && (!self.media_dependency.all_available() || !self.gstreamer_available)
        {
            self.show_media_dependency_modal = true;
        }
    }

    pub fn apply_gstreamer_dependency_status(&mut self, gstreamer_cli: Option<String>) {
        match gstreamer_cli {
            Some(path) => {
                self.gstreamer_path = path;
                self.gstreamer_available = true;
            }
            None => {
                self.gstreamer_path = "gst-launch-1.0".to_string();
                self.gstreamer_available = false;
            }
        }
    }

    pub fn hide_media_dependency_modal(&mut self) {
        self.show_media_dependency_modal = false;
    }

    pub fn show_media_dependency_modal(&mut self) {
        self.show_media_dependency_modal = true;
    }

    pub fn show_preview_memory_budget_modal(&mut self) {
        self.show_preview_memory_budget_modal = true;
    }

    pub fn hide_preview_memory_budget_modal(&mut self) {
        self.show_preview_memory_budget_modal = false;
    }

    pub fn set_preview_memory_budget_mb(&mut self, budget_mb: Option<usize>) {
        self.preview_memory_budget_mb = budget_mb.map(|v| v.clamp(256, 32768));
    }

    /// Open the silence preview modal so user can pick which silence ranges to cut.
    pub fn show_silence_preview_modal(&mut self, state: SilencePreviewModalState) {
        self.silence_preview_modal = Some(state);
    }

    /// Close the silence preview modal and discard any pending selections.
    pub fn hide_silence_preview_modal(&mut self) {
        self.silence_preview_modal = None;
    }

    /// Toggle the selected state of a silence candidate by index.
    pub fn toggle_silence_candidate(&mut self, index: usize) {
        if let Some(modal) = self.silence_preview_modal.as_mut()
            && let Some(c) = modal.candidates.get_mut(index)
        {
            c.selected = !c.selected;
        }
    }

    /// Collect selected silence candidates as JSON ready for ACP agent injection.
    pub fn selected_silence_candidates_json(&self) -> Option<String> {
        let modal = self.silence_preview_modal.as_ref()?;
        let selected: Vec<_> = modal
            .candidates
            .iter()
            .filter(|c| c.selected)
            .map(|c| {
                serde_json::json!({
                    "start_ms": c.start_ms,
                    "end_ms": c.end_ms,
                    "confidence": c.confidence,
                    "reason": c.reason,
                })
            })
            .collect();
        if selected.is_empty() {
            return None;
        }
        Some(serde_json::json!({
            "user_selected_silence_ranges": selected,
            "timeline_revision": modal.timeline_revision,
            "instruction": "Apply ripple_delete_range for each selected range. Validate then apply."
        }).to_string())
    }

    pub fn media_tools_ready_for_export(&self) -> bool {
        self.media_dependency.ffmpeg_available && self.media_dependency.ffprobe_available
    }

    pub fn media_tools_ready_for_preview_gen(&self) -> bool {
        self.media_dependency.ffmpeg_available
    }

    pub fn set_canvas_size(&mut self, w: f32, h: f32) {
        let min = 144.0;
        // Allow 8K timeline presets (including 7680x4320 and 4320x7680) without
        // collapsing into a clamped "custom" 4096x4096 canvas.
        let max = 7680.0;
        self.canvas_w = w.clamp(min, max);
        self.canvas_h = h.clamp(min, max);
    }

    pub fn inspector_panel_expanded(&self) -> bool {
        self.inspector_expanded
    }

    pub fn set_inspector_panel_expanded(&mut self, expanded: bool) {
        self.inspector_expanded = expanded;
    }

    pub fn inspector_panel_width(&self) -> f32 {
        360.0
    }

    pub fn layer_color_blur_effects(&self) -> LayerColorBlurEffects {
        self.layer_color_blur_effects
    }

    pub fn layer_color_blur_effects_at(&self, timeline_time: Duration) -> LayerColorBlurEffects {
        let mut has_active = false;
        let mut out = LayerColorBlurEffects::default();
        for layer in &self.layer_effect_clips {
            let Some(mut sampled) = layer.effects_at(timeline_time) else {
                continue;
            };
            // Keep backward compatibility with legacy projects where layer effects were global.
            if !layer.has_any_effect_enabled() && !self.layer_color_blur_effects.is_identity() {
                sampled = self.layer_color_blur_effects;
            }
            if let Some(runtime_out) = motionloom_output_for_layer(layer, timeline_time) {
                let mut runtime_signed_sigma = 0.0_f32;
                if let Some(v) = runtime_out.layer_blur_sigma {
                    runtime_signed_sigma += v.clamp(0.0, 64.0);
                }
                if let Some(v) = runtime_out.layer_sharpen_sigma {
                    // Signed blur contract: negative means sharpen amount.
                    runtime_signed_sigma -= v.clamp(0.0, 64.0);
                }
                sampled.blur_sigma = runtime_signed_sigma.clamp(-64.0, 64.0);
            }
            has_active = true;
            out.brightness += sampled.brightness;
            out.contrast *= sampled.contrast;
            out.saturation *= sampled.saturation;
            out.blur_sigma += sampled.blur_sigma;
        }
        if has_active {
            out.normalized()
        } else {
            LayerColorBlurEffects::default()
        }
    }

    pub fn layer_opacity_factor_at(&self, timeline_time: Duration) -> f32 {
        let mut has_active = false;
        let mut strength = 0.0_f32;
        let mut transition_opacity = 1.0_f32;
        for layer in &self.layer_effect_clips {
            let end = layer.end();
            if timeline_time < layer.start || timeline_time >= end {
                continue;
            }
            has_active = true;
            strength = strength.max(layer.envelope_factor_at(timeline_time));
            if let Some(runtime_out) = motionloom_output_for_layer(layer, timeline_time)
                && let Some(v) = runtime_out.layer_transition_opacity
            {
                transition_opacity = transition_opacity.min(v.clamp(0.0, 1.0));
            }
        }
        if has_active {
            (strength * transition_opacity).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    pub fn layer_lut_mix_at(&self, timeline_time: Duration) -> f32 {
        let mut lut_mix = 0.0_f32;
        for layer in &self.layer_effect_clips {
            let end = layer.end();
            if timeline_time < layer.start || timeline_time >= end {
                continue;
            }
            let strength = layer.envelope_factor_at(timeline_time).clamp(0.0, 1.0);
            if let Some(runtime_out) = motionloom_output_for_layer(layer, timeline_time)
                && let Some(v) = runtime_out.layer_lut_mix
            {
                let weighted = v.clamp(0.0, 1.0) * strength;
                lut_mix = lut_mix.max(weighted);
            }
        }
        lut_mix.clamp(0.0, 1.0)
    }

    pub fn layer_hsla_overlay_at(&self, timeline_time: Duration) -> (f32, f32, f32, f32) {
        let mut best: Option<(f32, f32, f32, f32)> = None;
        let mut best_weighted_alpha = 0.0_f32;
        for layer in &self.layer_effect_clips {
            let end = layer.end();
            if timeline_time < layer.start || timeline_time >= end {
                continue;
            }
            let strength = layer.envelope_factor_at(timeline_time).clamp(0.0, 1.0);
            if let Some(runtime_out) = motionloom_output_for_layer(layer, timeline_time)
                && let (Some(h), Some(s), Some(l), Some(a)) = (
                    runtime_out.layer_hsla_hue,
                    runtime_out.layer_hsla_saturation,
                    runtime_out.layer_hsla_lightness,
                    runtime_out.layer_hsla_alpha,
                )
            {
                let weighted_alpha = (a.clamp(0.0, 1.0) * strength).clamp(0.0, 1.0);
                if weighted_alpha > best_weighted_alpha {
                    best_weighted_alpha = weighted_alpha;
                    best = Some((h, s, l, weighted_alpha));
                }
            }
        }
        best.unwrap_or((0.0, 0.0, 0.0, 0.0))
    }

    pub fn layer_transition_dissolve_mix_at(&self, timeline_time: Duration) -> Option<f32> {
        let mut out: Option<f32> = None;
        for layer in &self.layer_effect_clips {
            let end = layer.end();
            if timeline_time < layer.start || timeline_time >= end {
                continue;
            }
            if let Some(runtime_out) = motionloom_output_for_layer(layer, timeline_time)
                && let Some(v) = runtime_out.transition_dissolve_mix
            {
                out = Some(out.map_or(v, |prev| prev.max(v)));
            }
        }
        out.map(|v| v.clamp(0.0, 1.0))
    }

    pub fn set_layer_color_blur_effects(&mut self, effects: LayerColorBlurEffects) {
        self.layer_color_blur_effects = effects.normalized();
    }

    pub fn layer_effect_clips(&self) -> &[LayerEffectClip] {
        &self.layer_effect_clips
    }

    pub fn layer_effect_clip_selected(&self) -> bool {
        self.selected_layer_effect_clip_id().is_some()
    }

    pub fn selected_layer_effect_clip_id(&self) -> Option<u64> {
        let id = self.selected_layer_effect_clip_id?;
        self.layer_effect_clips
            .iter()
            .any(|clip| clip.id == id)
            .then_some(id)
    }

    pub fn selected_layer_effect_clip(&self) -> Option<LayerEffectClip> {
        let id = self.selected_layer_effect_clip_id()?;
        self.layer_effect_clips
            .iter()
            .find(|clip| clip.id == id)
            .cloned()
    }

    pub fn set_layer_effect_clip_motionloom_script(
        &mut self,
        clip_id: u64,
        script: String,
    ) -> bool {
        let Some(clip) = self
            .layer_effect_clips
            .iter_mut()
            .find(|clip| clip.id == clip_id)
        else {
            return false;
        };
        let normalized = script.trim().to_string();
        clip.motionloom_enabled = !normalized.is_empty();
        clip.motionloom_script = normalized;
        true
    }

    fn selected_layer_effect_clip_index(&self) -> Option<usize> {
        let id = self.selected_layer_effect_clip_id()?;
        self.layer_effect_clips
            .iter()
            .position(|clip| clip.id == id)
    }

    fn selected_layer_effect_local_time(&self) -> Option<(usize, Duration)> {
        let idx = self.selected_layer_effect_clip_index()?;
        let local = self.layer_effect_clips[idx].local_time(self.playhead)?;
        Some((idx, local))
    }

    pub fn selected_layer_effect_brightness_enabled(&self) -> bool {
        self.selected_layer_effect_clip()
            .map(|clip| clip.brightness_enabled)
            .unwrap_or(false)
    }

    pub fn selected_layer_effect_contrast_enabled(&self) -> bool {
        self.selected_layer_effect_clip()
            .map(|clip| clip.contrast_enabled)
            .unwrap_or(false)
    }

    pub fn selected_layer_effect_saturation_enabled(&self) -> bool {
        self.selected_layer_effect_clip()
            .map(|clip| clip.saturation_enabled)
            .unwrap_or(false)
    }

    pub fn selected_layer_effect_blur_enabled(&self) -> bool {
        self.selected_layer_effect_clip()
            .map(|clip| clip.blur_enabled)
            .unwrap_or(false)
    }

    pub fn get_selected_layer_effect_brightness(&self) -> Option<f32> {
        let clip = self.selected_layer_effect_clip()?;
        if let Some(local) = clip.local_time(self.playhead) {
            Some(clip.sample_brightness_local(local))
        } else if clip.brightness_enabled {
            Some(clip.brightness)
        } else {
            Some(0.0)
        }
    }

    pub fn get_selected_layer_effect_contrast(&self) -> Option<f32> {
        let clip = self.selected_layer_effect_clip()?;
        if let Some(local) = clip.local_time(self.playhead) {
            Some(clip.sample_contrast_local(local))
        } else if clip.contrast_enabled {
            Some(clip.contrast)
        } else {
            Some(1.0)
        }
    }

    pub fn get_selected_layer_effect_saturation(&self) -> Option<f32> {
        let clip = self.selected_layer_effect_clip()?;
        if let Some(local) = clip.local_time(self.playhead) {
            Some(clip.sample_saturation_local(local))
        } else if clip.saturation_enabled {
            Some(clip.saturation)
        } else {
            Some(1.0)
        }
    }

    pub fn get_selected_layer_effect_blur(&self) -> Option<f32> {
        let clip = self.selected_layer_effect_clip()?;
        if let Some(local) = clip.local_time(self.playhead) {
            Some(clip.sample_blur_local(local))
        } else if clip.blur_enabled {
            Some(clip.blur_sigma)
        } else {
            Some(0.0)
        }
    }

    pub fn set_selected_layer_effect_brightness(&mut self, val: f32) -> bool {
        let Some(idx) = self.selected_layer_effect_clip_index() else {
            return false;
        };
        let new_val = val.clamp(-1.0, 1.0);
        let current = self.get_selected_layer_effect_brightness().unwrap_or(0.0);
        if (current - new_val).abs() <= 0.001 {
            return false;
        }
        self.save_for_undo();
        let clip = &mut self.layer_effect_clips[idx];
        clip.brightness_enabled = true;
        if let Some(local) = clip.local_time(self.playhead)
            && clip.brightness_keyframe_index_at(local).is_some()
        {
            clip.set_brightness_keyframe(local, new_val);
        } else {
            clip.brightness = new_val;
        }
        self.layer_color_blur_effects.brightness = clip.brightness;
        true
    }

    pub fn set_selected_layer_effect_contrast(&mut self, val: f32) -> bool {
        let Some(idx) = self.selected_layer_effect_clip_index() else {
            return false;
        };
        let new_val = val.clamp(0.0, 2.0);
        let current = self.get_selected_layer_effect_contrast().unwrap_or(1.0);
        if (current - new_val).abs() <= 0.001 {
            return false;
        }
        self.save_for_undo();
        let clip = &mut self.layer_effect_clips[idx];
        clip.contrast_enabled = true;
        if let Some(local) = clip.local_time(self.playhead)
            && clip.contrast_keyframe_index_at(local).is_some()
        {
            clip.set_contrast_keyframe(local, new_val);
        } else {
            clip.contrast = new_val;
        }
        self.layer_color_blur_effects.contrast = clip.contrast;
        true
    }

    pub fn set_selected_layer_effect_saturation(&mut self, val: f32) -> bool {
        let Some(idx) = self.selected_layer_effect_clip_index() else {
            return false;
        };
        let new_val = val.clamp(0.0, 2.0);
        let current = self.get_selected_layer_effect_saturation().unwrap_or(1.0);
        if (current - new_val).abs() <= 0.001 {
            return false;
        }
        self.save_for_undo();
        let clip = &mut self.layer_effect_clips[idx];
        clip.saturation_enabled = true;
        if let Some(local) = clip.local_time(self.playhead)
            && clip.saturation_keyframe_index_at(local).is_some()
        {
            clip.set_saturation_keyframe(local, new_val);
        } else {
            clip.saturation = new_val;
        }
        self.layer_color_blur_effects.saturation = clip.saturation;
        true
    }

    pub fn set_selected_layer_effect_blur(&mut self, val: f32) -> bool {
        let Some(idx) = self.selected_layer_effect_clip_index() else {
            return false;
        };
        let new_val = val.clamp(0.0, 64.0);
        let current = self.get_selected_layer_effect_blur().unwrap_or(0.0);
        if (current - new_val).abs() <= 0.001 {
            return false;
        }
        self.save_for_undo();
        let clip = &mut self.layer_effect_clips[idx];
        clip.blur_enabled = true;
        if let Some(local) = clip.local_time(self.playhead)
            && clip.blur_keyframe_index_at(local).is_some()
        {
            clip.set_blur_keyframe(local, new_val);
        } else {
            clip.blur_sigma = new_val;
        }
        self.layer_color_blur_effects.blur_sigma = clip.blur_sigma;
        true
    }

    pub fn add_selected_layer_effect_brightness_keyframe(&mut self) -> bool {
        let Some((idx, local)) = self.selected_layer_effect_local_time() else {
            return false;
        };
        self.save_for_undo();
        let clip = &mut self.layer_effect_clips[idx];
        clip.brightness_enabled = true;
        let value = clip.sample_brightness_local(local);
        clip.set_brightness_keyframe(local, value);
        true
    }

    pub fn add_selected_layer_effect_contrast_keyframe(&mut self) -> bool {
        let Some((idx, local)) = self.selected_layer_effect_local_time() else {
            return false;
        };
        self.save_for_undo();
        let clip = &mut self.layer_effect_clips[idx];
        clip.contrast_enabled = true;
        let value = clip.sample_contrast_local(local);
        clip.set_contrast_keyframe(local, value);
        true
    }

    pub fn add_selected_layer_effect_saturation_keyframe(&mut self) -> bool {
        let Some((idx, local)) = self.selected_layer_effect_local_time() else {
            return false;
        };
        self.save_for_undo();
        let clip = &mut self.layer_effect_clips[idx];
        clip.saturation_enabled = true;
        let value = clip.sample_saturation_local(local);
        clip.set_saturation_keyframe(local, value);
        true
    }

    pub fn add_selected_layer_effect_blur_keyframe(&mut self) -> bool {
        let Some((idx, local)) = self.selected_layer_effect_local_time() else {
            return false;
        };
        self.save_for_undo();
        let clip = &mut self.layer_effect_clips[idx];
        clip.blur_enabled = true;
        let value = clip.sample_blur_local(local);
        clip.set_blur_keyframe(local, value);
        true
    }

    pub fn selected_layer_effect_has_brightness_keyframe(&self) -> bool {
        let Some((idx, local)) = self.selected_layer_effect_local_time() else {
            return false;
        };
        self.layer_effect_clips[idx]
            .brightness_keyframe_index_at(local)
            .is_some()
    }

    pub fn selected_layer_effect_has_contrast_keyframe(&self) -> bool {
        let Some((idx, local)) = self.selected_layer_effect_local_time() else {
            return false;
        };
        self.layer_effect_clips[idx]
            .contrast_keyframe_index_at(local)
            .is_some()
    }

    pub fn selected_layer_effect_has_saturation_keyframe(&self) -> bool {
        let Some((idx, local)) = self.selected_layer_effect_local_time() else {
            return false;
        };
        self.layer_effect_clips[idx]
            .saturation_keyframe_index_at(local)
            .is_some()
    }

    pub fn selected_layer_effect_has_blur_keyframe(&self) -> bool {
        let Some((idx, local)) = self.selected_layer_effect_local_time() else {
            return false;
        };
        self.layer_effect_clips[idx]
            .blur_keyframe_index_at(local)
            .is_some()
    }

    pub fn remove_selected_layer_effect_brightness(&mut self) -> bool {
        let Some(idx) = self.selected_layer_effect_clip_index() else {
            return false;
        };
        if !self.layer_effect_clips[idx].brightness_enabled {
            return false;
        }
        self.save_for_undo();
        self.layer_effect_clips[idx].clear_brightness_effect();
        self.layer_color_blur_effects.brightness = 0.0;
        true
    }

    pub fn remove_selected_layer_effect_contrast(&mut self) -> bool {
        let Some(idx) = self.selected_layer_effect_clip_index() else {
            return false;
        };
        if !self.layer_effect_clips[idx].contrast_enabled {
            return false;
        }
        self.save_for_undo();
        self.layer_effect_clips[idx].clear_contrast_effect();
        self.layer_color_blur_effects.contrast = 1.0;
        true
    }

    pub fn remove_selected_layer_effect_saturation(&mut self) -> bool {
        let Some(idx) = self.selected_layer_effect_clip_index() else {
            return false;
        };
        if !self.layer_effect_clips[idx].saturation_enabled {
            return false;
        }
        self.save_for_undo();
        self.layer_effect_clips[idx].clear_saturation_effect();
        self.layer_color_blur_effects.saturation = 1.0;
        true
    }

    pub fn remove_selected_layer_effect_blur(&mut self) -> bool {
        let Some(idx) = self.selected_layer_effect_clip_index() else {
            return false;
        };
        if !self.layer_effect_clips[idx].blur_enabled {
            return false;
        }
        self.save_for_undo();
        self.layer_effect_clips[idx].clear_blur_effect();
        self.layer_color_blur_effects.blur_sigma = 0.0;
        true
    }

    pub fn clear_layer_effect_clip_selection(&mut self) {
        self.selected_layer_effect_clip_id = None;
    }

    pub fn select_layer_effect_clip(&mut self, clip_id: u64) {
        if !self
            .layer_effect_clips
            .iter()
            .any(|clip| clip.id == clip_id)
        {
            self.selected_layer_effect_clip_id = None;
            return;
        }
        self.selected_clip_id = None;
        self.selected_subtitle_id = None;
        self.selected_clip_ids.clear();
        self.selected_subtitle_ids.clear();
        self.selected_semantic_clip_id = None;
        self.selected_layer_effect_clip_id = Some(clip_id);
    }

    pub fn add_layer_effect_clip_on_top_video_track(&mut self) {
        let mut created_track = false;
        if self.video_tracks.is_empty() {
            self.add_new_video_track();
            created_track = true;
        }

        let top_track_index = self.video_tracks.len().saturating_sub(1);
        // Keep new layer clips short by default; user can resize as needed.
        let duration = Duration::from_secs(10);

        if !created_track {
            self.save_for_undo();
        }
        let id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);
        self.layer_effect_clips.push(LayerEffectClip {
            id,
            start: Duration::ZERO,
            duration,
            track_index: top_track_index,
            fade_in: Duration::ZERO,
            fade_out: Duration::ZERO,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            blur_sigma: 0.0,
            brightness_enabled: false,
            contrast_enabled: false,
            saturation_enabled: false,
            blur_enabled: false,
            brightness_keyframes: Vec::new(),
            contrast_keyframes: Vec::new(),
            saturation_keyframes: Vec::new(),
            blur_keyframes: Vec::new(),
            motionloom_enabled: false,
            motionloom_script: String::new(),
        });
        self.layer_effect_clips
            .sort_by_key(|clip| (clip.track_index, clip.start));
        self.select_layer_effect_clip(id);
    }

    pub fn semantic_clips(&self) -> &[SemanticClip] {
        &self.semantic_clips
    }

    /// Ensure every semantic clip carries a normalized prompt schema.
    pub fn normalize_all_semantic_prompt_schemas(&mut self) {
        for clip in &mut self.semantic_clips {
            sync_semantic_prompt_schema_fields(clip);
        }
    }

    pub fn selected_semantic_clip_id(&self) -> Option<u64> {
        let id = self.selected_semantic_clip_id?;
        if self.semantic_clips.iter().any(|clip| clip.id == id) {
            Some(id)
        } else {
            None
        }
    }

    pub fn clear_semantic_clip_selection(&mut self) {
        self.selected_semantic_clip_id = None;
    }

    pub fn select_semantic_clip(&mut self, clip_id: u64) {
        if !self.semantic_clips.iter().any(|clip| clip.id == clip_id) {
            self.selected_semantic_clip_id = None;
            return;
        }
        self.selected_semantic_clip_id = Some(clip_id);
        self.selected_layer_effect_clip_id = None;
        self.selected_clip_id = None;
        self.selected_subtitle_id = None;
        self.selected_clip_ids.clear();
        self.selected_subtitle_ids.clear();
    }

    pub fn mark_semantic_start_at_playhead(&mut self) {
        self.semantic_mark_start = Some(self.playhead);
        self.ui_notice = Some(format!(
            "Semantic start marked at {:.2}s. Press K to close segment.",
            self.playhead.as_secs_f32()
        ));
    }

    pub fn commit_semantic_segment_at_playhead(&mut self) -> bool {
        let Some(mark_start) = self.semantic_mark_start else {
            self.ui_notice = Some("No semantic start mark. Press J first.".to_string());
            return false;
        };

        let start = mark_start.min(self.playhead);
        let end = mark_start.max(self.playhead);
        let duration = if end > start {
            end.saturating_sub(start)
        } else {
            Duration::from_millis(1)
        };

        self.save_for_undo();
        let id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);
        self.semantic_clips.push(SemanticClip {
            id,
            start,
            duration,
            semantic_type: "content_support".to_string(),
            label: "semantic".to_string(),
            prompt_schema: default_semantic_prompt_schema(
                id,
                start,
                duration,
                "content_support",
                "semantic",
            ),
        });
        self.semantic_clips.sort_by_key(|clip| clip.start);
        self.select_semantic_clip(id);
        self.semantic_mark_start = None;
        self.ui_notice = Some(format!(
            "Semantic segment added: {:.2}s -> {:.2}s",
            start.as_secs_f32(),
            (start + duration).as_secs_f32()
        ));
        true
    }

    pub fn add_semantic_clip_at_playhead(&mut self) -> bool {
        let start = self.playhead;
        let duration = Duration::from_secs(2);

        self.save_for_undo();
        let id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);
        self.semantic_clips.push(SemanticClip {
            id,
            start,
            duration,
            semantic_type: "content_support".to_string(),
            label: "semantic".to_string(),
            prompt_schema: default_semantic_prompt_schema(
                id,
                start,
                duration,
                "content_support",
                "semantic",
            ),
        });
        self.semantic_clips.sort_by_key(|clip| clip.start);
        self.select_semantic_clip(id);
        self.semantic_mark_start = None;
        self.ui_notice = Some(format!(
            "Semantic segment added at {:.2}s ({:.2}s).",
            start.as_secs_f32(),
            duration.as_secs_f32()
        ));
        true
    }

    /// Insert a semantic clip at an arbitrary position with a custom label (used by ACP edit-plan).
    pub fn insert_semantic_clip(
        &mut self,
        start: Duration,
        duration: Duration,
        semantic_type: String,
        label: String,
        prompt_schema: Option<Value>,
    ) -> bool {
        self.save_for_undo();
        let id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);
        let semantic_type_text = if semantic_type.trim().is_empty() {
            "content_support".to_string()
        } else {
            semantic_type
        };
        let label_text = if label.trim().is_empty() {
            "semantic".to_string()
        } else {
            label
        };
        let mut clip = SemanticClip {
            id,
            start,
            duration,
            semantic_type: semantic_type_text.clone(),
            label: label_text.clone(),
            prompt_schema: prompt_schema.unwrap_or_else(|| {
                default_semantic_prompt_schema(
                    id,
                    start,
                    duration,
                    semantic_type_text.as_str(),
                    label_text.as_str(),
                )
            }),
        };
        // Keep schema fields synchronized with semantic clip timing.
        sync_semantic_prompt_schema_fields(&mut clip);
        self.semantic_clips.push(clip);
        self.semantic_clips.sort_by_key(|clip| clip.start);
        true
    }

    pub fn move_semantic_clip(&mut self, clip_id: u64, new_start: Duration) -> bool {
        let Some(idx) = self
            .semantic_clips
            .iter()
            .position(|clip| clip.id == clip_id)
        else {
            return false;
        };
        self.semantic_clips[idx].start = new_start;
        sync_semantic_prompt_schema_fields(&mut self.semantic_clips[idx]);
        self.semantic_clips.sort_by_key(|clip| clip.start);
        self.selected_semantic_clip_id = Some(clip_id);
        true
    }

    pub fn resize_semantic_clip(&mut self, clip_id: u64, new_duration: Duration) -> bool {
        let Some(idx) = self
            .semantic_clips
            .iter()
            .position(|clip| clip.id == clip_id)
        else {
            return false;
        };
        self.semantic_clips[idx].duration = new_duration.max(Duration::from_millis(1));
        sync_semantic_prompt_schema_fields(&mut self.semantic_clips[idx]);
        self.selected_semantic_clip_id = Some(clip_id);
        true
    }

    /// Get the label text of the currently selected semantic clip.
    pub fn get_selected_semantic_label(&self) -> Option<String> {
        let id = self.selected_semantic_clip_id?;
        self.semantic_clips
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.label.clone())
    }

    /// Get the marker category of the currently selected semantic clip.
    pub fn get_selected_semantic_type(&self) -> Option<String> {
        let id = self.selected_semantic_clip_id?;
        self.semantic_clips
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.semantic_type.clone())
    }

    /// Return selected semantic schema as pretty JSON for Inspector editing.
    pub fn get_selected_semantic_schema_json(&self) -> Option<String> {
        let id = self.selected_semantic_clip_id?;
        self.semantic_clips
            .iter()
            .find(|c| c.id == id)
            .and_then(|clip| serde_json::to_string_pretty(&clip.prompt_schema).ok())
    }

    /// Return selected semantic clip duration in seconds for UI status text.
    pub fn get_selected_semantic_duration_sec(&self) -> Option<f64> {
        let id = self.selected_semantic_clip_id?;
        self.semantic_clips
            .iter()
            .find(|c| c.id == id)
            .map(|clip| semantic_duration_sec(clip.duration))
    }

    /// Return current semantic asset mode (image/video) for selected semantic clip.
    pub fn get_selected_semantic_asset_mode(&self) -> Option<String> {
        let id = self.selected_semantic_clip_id?;
        self.semantic_clips
            .iter()
            .find(|c| c.id == id)
            .map(|clip| semantic_asset_mode_from_schema(&clip.prompt_schema))
    }

    /// Return current model/provider for selected semantic clip.
    pub fn get_selected_semantic_model(&self) -> Option<String> {
        let id = self.selected_semantic_clip_id?;
        self.semantic_clips
            .iter()
            .find(|c| c.id == id)
            .map(|clip| semantic_provider_from_schema(&clip.prompt_schema))
    }

    /// Return user-facing prompt text for selected semantic clip, mode-aware (image/video).
    pub fn get_selected_semantic_prompt_text(&self) -> Option<String> {
        let id = self.selected_semantic_clip_id?;
        let clip = self.semantic_clips.iter().find(|c| c.id == id)?;
        let mode = semantic_asset_mode_from_schema(&clip.prompt_schema);
        semantic_prompt_text_for_mode(&clip.prompt_schema, mode.as_str())
    }

    /// Return selected semantic image generation size overrides from schema.
    pub fn get_selected_semantic_image_size(&self) -> Option<(Option<u32>, Option<u32>)> {
        let id = self.selected_semantic_clip_id?;
        let clip = self.semantic_clips.iter().find(|c| c.id == id)?;
        Some((
            semantic_image_dimension_from_schema(&clip.prompt_schema, "width"),
            semantic_image_dimension_from_schema(&clip.prompt_schema, "height"),
        ))
    }

    /// Validate selected semantic schema against provider constraints.
    pub fn validate_selected_semantic_schema(&self) -> Option<SemanticSchemaValidation> {
        let id = self.selected_semantic_clip_id?;
        self.semantic_clips
            .iter()
            .find(|clip| clip.id == id)
            .map(validate_semantic_prompt_schema)
    }

    /// Parse and apply schema JSON from Inspector editor.
    pub fn set_selected_semantic_schema_json(
        &mut self,
        json_text: String,
    ) -> Result<(), GlobalStateError> {
        let Some(id) = self.selected_semantic_clip_id else {
            return Err(GlobalStateError::NoSemanticClipSelected);
        };
        let parsed: Value = serde_json::from_str(json_text.trim())
            .map_err(|source| GlobalStateError::InvalidSemanticSchemaJson { source })?;
        let requested_duration_sec = if let Some(raw_duration) = parsed.get("duration_sec") {
            let Some(duration_sec) = raw_duration.as_f64() else {
                return Err(GlobalStateError::InvalidSemanticDurationType);
            };
            if !duration_sec.is_finite() || duration_sec <= 0.0 {
                return Err(GlobalStateError::InvalidSemanticDurationValue);
            }
            Some(duration_sec)
        } else {
            None
        };
        let requested_semantic_goal = parsed
            .get("semantic_goal")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToString::to_string);
        let Some(clip) = self.semantic_clips.iter_mut().find(|c| c.id == id) else {
            return Err(GlobalStateError::SelectedSemanticClipMissing);
        };
        if let Some(duration_sec) = requested_duration_sec {
            // Allow schema Apply to drive semantic layer length from duration_sec.
            clip.duration = Duration::from_secs_f64(duration_sec.max(0.001));
        }
        if let Some(semantic_goal) = requested_semantic_goal {
            // Keep semantic type and schema semantic_goal bidirectionally synchronized.
            clip.semantic_type = semantic_goal;
        }
        clip.prompt_schema = parsed;
        sync_semantic_prompt_schema_fields(clip);
        let validation = validate_semantic_prompt_schema(clip);
        if !validation.is_ok() {
            return Err(GlobalStateError::SemanticSchemaValidationFailed {
                message: validation.errors.join(" "),
            });
        }
        Ok(())
    }

    /// Update selected semantic clip asset mode (image/video) without hand-editing JSON.
    pub fn set_selected_semantic_asset_mode(&mut self, mode: String) {
        let Some(id) = self.selected_semantic_clip_id else {
            return;
        };
        if let Some(clip) = self.semantic_clips.iter_mut().find(|c| c.id == id) {
            let mode_text = normalize_semantic_asset_mode(mode.as_str()).to_string();
            let root = ensure_value_object(&mut clip.prompt_schema);
            root.insert("asset_mode".to_string(), Value::String(mode_text));
            sync_semantic_prompt_schema_fields(clip);
        }
    }

    /// Update selected semantic clip provider/model without hand-editing JSON.
    pub fn set_selected_semantic_model(&mut self, model: String) {
        let Some(id) = self.selected_semantic_clip_id else {
            return;
        };
        let model_text = model.trim().to_ascii_lowercase().trim().to_string();
        if model_text.is_empty() {
            return;
        }
        if let Some(clip) = self.semantic_clips.iter_mut().find(|c| c.id == id) {
            let root = ensure_value_object(&mut clip.prompt_schema);
            root.insert("provider".to_string(), Value::String(model_text.clone()));
            let is_image_model = matches!(
                model_text.as_str(),
                "nanobanana"
                    | "google/nanobanana"
                    | "gemini-3.1-flash-image-preview"
                    | "google/gemini-3.1-flash-image-preview"
                    | "gemini-3-pro-image-preview"
                    | "google/gemini-3-pro-image-preview"
                    | "gemini-2.5-flash-image"
                    | "google/gemini-2.5-flash-image"
                    | "gpt-image-1"
                    | "gpt-image-1.5"
                    | "gpt-image-1-mini"
                    | "openai/gpt-image-1"
                    | "openai/gpt-image-1.5"
                    | "openai/gpt-image-1-mini"
            );
            let is_video_model = matches!(
                model_text.as_str(),
                "veo_3_1"
                    | "google/veo_3_1"
                    | "google/veo-3.1-generate-preview"
                    | "openai/sora-2"
                    | "sora-2"
                    | "sora_2"
                    | "openai/sora-2-pro"
                    | "sora-2-pro"
                    | "sora_2_pro"
            );
            if is_image_model {
                root.insert("asset_mode".to_string(), Value::String("image".to_string()));
            } else if is_video_model {
                root.insert("asset_mode".to_string(), Value::String("video".to_string()));
            }
            sync_semantic_prompt_schema_fields(clip);
        }
    }

    /// Update selected semantic prompt text and route it to image/video prompt by current mode.
    pub fn set_selected_semantic_prompt_text(&mut self, text: String) {
        let Some(id) = self.selected_semantic_clip_id else {
            return;
        };
        if let Some(clip) = self.semantic_clips.iter_mut().find(|c| c.id == id) {
            let mode = semantic_asset_mode_from_schema(&clip.prompt_schema);
            let key = semantic_prompt_key_for_mode(mode.as_str());
            let root = ensure_value_object(&mut clip.prompt_schema);
            let prompts = ensure_child_object(root, "prompts");
            prompts.insert(key.to_string(), Value::String(text));
            prompts.remove("fallback_search_query");
            sync_semantic_prompt_schema_fields(clip);
        }
    }

    /// Update selected semantic image output size overrides in schema.
    pub fn set_selected_semantic_image_size(&mut self, width: Option<u32>, height: Option<u32>) {
        let Some(id) = self.selected_semantic_clip_id else {
            return;
        };
        if let Some(clip) = self.semantic_clips.iter_mut().find(|c| c.id == id) {
            let root = ensure_value_object(&mut clip.prompt_schema);
            if width.is_none() && height.is_none() {
                root.remove("image_options");
                sync_semantic_prompt_schema_fields(clip);
                return;
            }
            let image_options = ensure_child_object(root, "image_options");
            if let Some(width) = width {
                image_options.insert("width".to_string(), json!(width));
            } else {
                image_options.remove("width");
            }
            if let Some(height) = height {
                image_options.insert("height".to_string(), json!(height));
            } else {
                image_options.remove("height");
            }
            if image_options.is_empty() {
                root.remove("image_options");
            }
            sync_semantic_prompt_schema_fields(clip);
        }
    }

    /// Set the label text of the currently selected semantic clip (for B-roll notes, markers, etc).
    pub fn set_selected_semantic_label(&mut self, text: String) {
        let Some(id) = self.selected_semantic_clip_id else {
            return;
        };
        if let Some(clip) = self.semantic_clips.iter_mut().find(|c| c.id == id) {
            clip.label = text;
            // Keep schema prompts aligned with semantic label edits.
            sync_semantic_prompt_schema_fields(clip);
        }
    }

    /// Set the marker category of the currently selected semantic clip.
    pub fn set_selected_semantic_type(&mut self, text: String) {
        let Some(id) = self.selected_semantic_clip_id else {
            return;
        };
        if let Some(clip) = self.semantic_clips.iter_mut().find(|c| c.id == id) {
            clip.semantic_type = text;
            // Keep schema semantic goal aligned with semantic type edits.
            sync_semantic_prompt_schema_fields(clip);
        }
    }

    pub fn remove_selected_semantic_clip(&mut self) -> bool {
        let Some(id) = self.selected_semantic_clip_id() else {
            return false;
        };
        self.save_for_undo();
        let before = self.semantic_clips.len();
        self.semantic_clips.retain(|clip| clip.id != id);
        if self.semantic_clips.len() == before {
            return false;
        }
        self.selected_semantic_clip_id = None;
        true
    }

    pub fn remove_selected_layer_effect_clip(&mut self) -> bool {
        let Some(id) = self.selected_layer_effect_clip_id() else {
            return false;
        };
        self.save_for_undo();
        let before = self.layer_effect_clips.len();
        self.layer_effect_clips.retain(|clip| clip.id != id);
        if self.layer_effect_clips.len() == before {
            return false;
        }
        self.selected_layer_effect_clip_id = None;
        true
    }

    pub fn duplicate_timeline_clip_after(&mut self, track_type: TrackType, clip_id: u64) -> bool {
        let Some(base_end) = self.timeline_clip_end(track_type, clip_id) else {
            return false;
        };
        self.duplicate_timeline_clip_at(track_type, clip_id, base_end)
    }

    pub fn duplicate_timeline_clip_at(
        &mut self,
        track_type: TrackType,
        clip_id: u64,
        new_start: Duration,
    ) -> bool {
        self.repair_missing_primary_av_links();
        let Some(base) = self.timeline_clip_clone(track_type, clip_id) else {
            return false;
        };

        self.save_for_undo();
        let id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);

        let mut duplicated = base;
        duplicated.id = id;
        duplicated.start = new_start.max(Duration::ZERO);
        duplicated.link_group_id = None;
        duplicated.dissolve_trim_in = Duration::ZERO;
        duplicated.dissolve_trim_out = Duration::ZERO;

        match track_type {
            TrackType::V1 => {
                let old_v1_group_starts = self.v1_group_anchor_starts();
                let shift_from = duplicated.start;
                let shift_by = duplicated.duration;
                for clip in &mut self.v1_clips {
                    if clip.start >= shift_from {
                        clip.start += shift_by;
                    }
                }
                self.v1_clips.push(duplicated);
                self.v1_clips.sort_by_key(|clip| clip.start);
                self.sync_linked_tracks_from_v1_deltas(&old_v1_group_starts);
            }
            TrackType::Audio(track_index) => {
                let Some(track) = self.audio_tracks.get_mut(track_index) else {
                    return false;
                };
                track.clips.push(duplicated);
                track.clips.sort_by_key(|clip| clip.start);
            }
            TrackType::VideoOverlay(track_index) => {
                let Some(track) = self.video_tracks.get_mut(track_index) else {
                    return false;
                };
                track.clips.push(duplicated);
                track.clips.sort_by_key(|clip| clip.start);
            }
            TrackType::Subtitle(_) => {
                return false;
            }
        }

        self.selected_clip_id = Some(id);
        self.selected_clip_ids = vec![id];
        self.selected_subtitle_id = None;
        self.selected_subtitle_ids.clear();
        self.selected_layer_effect_clip_id = None;
        self.selected_semantic_clip_id = None;
        true
    }

    pub fn duplicate_subtitle_clip_after(&mut self, track_index: usize, clip_id: u64) -> bool {
        let Some(base_end) = self.subtitle_clip_end(track_index, clip_id) else {
            return false;
        };
        self.duplicate_subtitle_clip_at(track_index, clip_id, base_end)
    }

    pub fn duplicate_subtitle_clip_at(
        &mut self,
        track_index: usize,
        clip_id: u64,
        new_start: Duration,
    ) -> bool {
        let Some(base) = self.subtitle_clip_clone(track_index, clip_id) else {
            return false;
        };

        self.save_for_undo();
        let id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);

        let mut duplicated = base;
        duplicated.id = id;
        duplicated.start = new_start.max(Duration::ZERO);

        let Some(track) = self.subtitle_tracks.get_mut(track_index) else {
            return false;
        };
        track.clips.push(duplicated);
        track.clips.sort_by_key(|clip| clip.start);

        self.selected_subtitle_id = Some(id);
        self.selected_subtitle_ids = vec![id];
        self.selected_clip_id = None;
        self.selected_clip_ids.clear();
        self.selected_layer_effect_clip_id = None;
        self.selected_semantic_clip_id = None;
        true
    }

    pub fn duplicate_semantic_clip_after(&mut self, clip_id: u64) -> bool {
        let Some(base_end) = self.semantic_clip_end(clip_id) else {
            return false;
        };
        self.duplicate_semantic_clip_at(clip_id, base_end)
    }

    pub fn duplicate_semantic_clip_at(&mut self, clip_id: u64, new_start: Duration) -> bool {
        let Some(base) = self.semantic_clip_clone(clip_id) else {
            return false;
        };

        self.save_for_undo();
        let id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);

        let mut duplicated = base;
        duplicated.id = id;
        duplicated.start = new_start.max(Duration::ZERO);
        // Keep duplicated schema synchronized with new clip identity and timing.
        sync_semantic_prompt_schema_fields(&mut duplicated);

        self.semantic_clips.push(duplicated);
        self.semantic_clips.sort_by_key(|clip| clip.start);
        self.selected_semantic_clip_id = Some(id);
        self.selected_layer_effect_clip_id = None;
        self.selected_clip_id = None;
        self.selected_subtitle_id = None;
        self.selected_clip_ids.clear();
        self.selected_subtitle_ids.clear();
        true
    }

    pub fn duplicate_layer_effect_clip_at(&mut self, clip_id: u64, new_start: Duration) -> bool {
        let Some(base) = self
            .layer_effect_clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .cloned()
        else {
            return false;
        };
        if self.video_tracks.is_empty() {
            return false;
        }

        self.save_for_undo();
        let id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);

        let duplicated = LayerEffectClip {
            id,
            start: new_start.max(Duration::ZERO),
            duration: base.duration,
            track_index: base
                .track_index
                .min(self.video_tracks.len().saturating_sub(1)),
            fade_in: base.fade_in,
            fade_out: base.fade_out,
            brightness: base.brightness,
            contrast: base.contrast,
            saturation: base.saturation,
            blur_sigma: base.blur_sigma,
            brightness_enabled: base.brightness_enabled,
            contrast_enabled: base.contrast_enabled,
            saturation_enabled: base.saturation_enabled,
            blur_enabled: base.blur_enabled,
            brightness_keyframes: base.brightness_keyframes.clone(),
            contrast_keyframes: base.contrast_keyframes.clone(),
            saturation_keyframes: base.saturation_keyframes.clone(),
            blur_keyframes: base.blur_keyframes.clone(),
            motionloom_enabled: base.motionloom_enabled,
            motionloom_script: base.motionloom_script.clone(),
        };
        self.layer_effect_clips.push(duplicated);
        self.layer_effect_clips
            .sort_by_key(|clip| (clip.track_index, clip.start));
        self.select_layer_effect_clip(id);
        true
    }

    pub fn duplicate_selected_layer_effect_clip(&mut self) -> bool {
        let Some(base) = self.selected_layer_effect_clip() else {
            return false;
        };
        let new_start = base.start + base.duration.max(Duration::from_secs(1));
        self.duplicate_layer_effect_clip_at(base.id, new_start)
    }

    fn timeline_clip_clone(&self, track_type: TrackType, clip_id: u64) -> Option<Clip> {
        match track_type {
            TrackType::V1 => self
                .v1_clips
                .iter()
                .find(|clip| clip.id == clip_id)
                .cloned(),
            TrackType::Audio(track_index) => self
                .audio_tracks
                .get(track_index)
                .and_then(|track| track.clips.iter().find(|clip| clip.id == clip_id))
                .cloned(),
            TrackType::VideoOverlay(track_index) => self
                .video_tracks
                .get(track_index)
                .and_then(|track| track.clips.iter().find(|clip| clip.id == clip_id))
                .cloned(),
            TrackType::Subtitle(_) => None,
        }
    }

    fn timeline_clip_end(&self, track_type: TrackType, clip_id: u64) -> Option<Duration> {
        self.timeline_clip_clone(track_type, clip_id)
            .map(|clip| clip.end())
    }

    fn subtitle_clip_clone(&self, track_index: usize, clip_id: u64) -> Option<SubtitleClip> {
        self.subtitle_tracks
            .get(track_index)
            .and_then(|track| track.clips.iter().find(|clip| clip.id == clip_id))
            .cloned()
    }

    fn subtitle_clip_end(&self, track_index: usize, clip_id: u64) -> Option<Duration> {
        self.subtitle_clip_clone(track_index, clip_id)
            .map(|clip| clip.end())
    }

    fn semantic_clip_clone(&self, clip_id: u64) -> Option<SemanticClip> {
        self.semantic_clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .cloned()
    }

    fn semantic_clip_end(&self, clip_id: u64) -> Option<Duration> {
        self.semantic_clip_clone(clip_id).map(|clip| clip.end())
    }

    pub fn move_layer_effect_clip(
        &mut self,
        clip_id: u64,
        new_start: Duration,
        new_track_index: Option<usize>,
    ) -> bool {
        let Some(clip) = self
            .layer_effect_clips
            .iter_mut()
            .find(|clip| clip.id == clip_id)
        else {
            return false;
        };
        clip.start = new_start.max(Duration::ZERO);
        if let Some(track_index) = new_track_index
            && track_index < self.video_tracks.len()
        {
            clip.track_index = track_index;
        }
        self.layer_effect_clips
            .sort_by_key(|layer| (layer.track_index, layer.start));
        true
    }

    pub fn resize_layer_effect_clip(&mut self, clip_id: u64, new_duration: Duration) -> bool {
        let Some(clip) = self
            .layer_effect_clips
            .iter_mut()
            .find(|clip| clip.id == clip_id)
        else {
            return false;
        };
        let max_dur = Duration::from_secs(3600 * 24);
        clip.duration = new_duration.clamp(Duration::from_millis(100), max_dur);
        true
    }

    pub fn set_next_clip_id(&mut self, id: u64) {
        self.next_clip_id = id.max(1);
    }

    pub fn next_clip_id(&self) -> u64 {
        self.next_clip_id
    }

    pub fn timeline_edit_token(&self) -> u64 {
        self.timeline_edit_token
    }

    fn bump_timeline_edit_token(&mut self) {
        // Monotonic token for cache invalidation in UI subsystems.
        self.timeline_edit_token = self.timeline_edit_token.wrapping_add(1);
    }

    pub fn cycle_export_color_mode(&mut self) {
        self.export_color_mode = self.export_color_mode.next();
    }

    pub fn set_preview_fps_value(&mut self, fps: u32) {
        if let Some(value) = PreviewFps::from_value(fps) {
            self.preview_fps = value;
        }
    }

    pub fn set_preview_video_input_fps(&mut self, fps: f32) {
        self.preview_video_input_fps = if fps.is_finite() { fps.max(0.0) } else { 0.0 };
    }

    pub fn set_preview_present_metrics(&mut self, fps: f32, dropped_frames: u64) {
        self.preview_present_fps = if fps.is_finite() { fps.max(0.0) } else { 0.0 };
        self.preview_present_dropped_frames = dropped_frames;
    }

    pub fn proxy_render_mode_for_quality(&self, quality: PreviewQuality) -> ProxyRenderMode {
        match quality {
            PreviewQuality::High => self.proxy_render_mode_high,
            PreviewQuality::Medium => self.proxy_render_mode_medium,
            PreviewQuality::Low => self.proxy_render_mode_low,
            PreviewQuality::Full => ProxyRenderMode::Nv12Surface,
        }
    }

    pub fn toggle_proxy_render_mode_for_quality(
        &mut self,
        quality: PreviewQuality,
    ) -> Option<ProxyRenderMode> {
        let mode = match quality {
            PreviewQuality::High => {
                self.proxy_render_mode_high = self.proxy_render_mode_high.toggled();
                self.proxy_render_mode_high
            }
            PreviewQuality::Medium => {
                self.proxy_render_mode_medium = self.proxy_render_mode_medium.toggled();
                self.proxy_render_mode_medium
            }
            PreviewQuality::Low => {
                self.proxy_render_mode_low = self.proxy_render_mode_low.toggled();
                self.proxy_render_mode_low
            }
            PreviewQuality::Full => return None,
        };
        Some(mode)
    }

    pub fn toggle_mac_preview_render_mode(&mut self) -> MacPreviewRenderMode {
        self.mac_preview_render_mode = self.mac_preview_render_mode.toggled();
        self.mac_preview_render_mode
    }

    // Toggle the default V1 dragging behavior between magnetic ripple and free overlap mode.
    pub fn cycle_v1_move_mode(&mut self) {
        self.v1_move_mode = self.v1_move_mode.next();
    }

    // Resolve effective magnetic behavior and allow Alt/Option to temporarily invert mode while dragging.
    pub fn effective_v1_magnetic(&self, alt_invert: bool) -> bool {
        let base_magnetic = matches!(self.v1_move_mode, V1MoveMode::Magnetic);
        if alt_invert {
            !base_magnetic
        } else {
            base_magnetic
        }
    }

    pub fn set_project_file_path(&mut self, path: Option<PathBuf>) {
        // Reset runtime proxy tracking when cache root scope changes.
        let changed = self.project_file_path != path;
        self.project_file_path = path;
        if changed {
            self.proxy_entries.clear();
            self.proxy_queue.clear();
            self.proxy_active = None;
            self.waveform_entries.clear();
            self.waveform_queue.clear();
            self.waveform_active = None;
        }
    }

    pub fn cache_root_dir(&self) -> PathBuf {
        // Unsaved projects use the shared AnicaProjects cache root.
        // Saved projects use a cache folder next to the project file.
        if let Some(project_path) = &self.project_file_path
            && let Some(parent) = project_path.parent()
        {
            return parent.join(".anica_cache");
        }
        default_project_dir()
    }

    pub fn generated_media_root_dir(&self) -> PathBuf {
        if let Some(project_path) = &self.project_file_path
            && let Some(parent) = project_path.parent()
        {
            return parent.to_path_buf();
        }
        default_project_dir()
    }

    pub fn generated_media_dir_for_semantic_mode(&self, mode: &str) -> PathBuf {
        let root = self.generated_media_root_dir();
        if normalize_semantic_asset_mode(mode) == "image" {
            root.join("images_generated")
        } else {
            root.join("videos_generated")
        }
    }

    pub fn ensure_proxy_for_path(&mut self, src_path: &str, max_dim: u32) -> ProxyLookup {
        let src = PathBuf::from(src_path);
        if !src.exists() {
            return ProxyLookup {
                path: None,
                status: ProxyStatus::Failed,
            };
        }
        let key = proxy_key(&src, max_dim);
        if let Some(entry) = self.proxy_entries.get(&key) {
            match entry.status {
                ProxyStatus::Ready => {
                    if entry.path.exists() {
                        return ProxyLookup {
                            path: Some(entry.path.to_string_lossy().to_string()),
                            status: ProxyStatus::Ready,
                        };
                    }
                }
                ProxyStatus::Pending => {
                    return ProxyLookup {
                        path: None,
                        status: ProxyStatus::Pending,
                    };
                }
                ProxyStatus::Failed => {
                    return ProxyLookup {
                        path: None,
                        status: ProxyStatus::Failed,
                    };
                }
                ProxyStatus::Missing => {}
            }
        }

        let cache_root = self.cache_root_dir();
        let dst = proxy_path_for_in(&cache_root, &src, max_dim);
        if dst.exists() {
            let entry = ProxyEntry {
                status: ProxyStatus::Ready,
                path: dst.clone(),
                error: None,
            };
            self.proxy_entries.insert(key, entry);
            return ProxyLookup {
                path: Some(dst.to_string_lossy().to_string()),
                status: ProxyStatus::Ready,
            };
        }

        if self.proxy_active.as_ref().is_some_and(|job| job.key == key)
            || self.proxy_queue.iter().any(|job| job.key == key)
        {
            return ProxyLookup {
                path: None,
                status: ProxyStatus::Pending,
            };
        }

        self.proxy_entries.insert(
            key.clone(),
            ProxyEntry {
                status: ProxyStatus::Pending,
                path: dst.clone(),
                error: None,
            },
        );
        info!(
            "[Proxy] enqueue {} -> {} ({}p)",
            src.to_string_lossy(),
            dst.to_string_lossy(),
            max_dim
        );
        self.proxy_queue.push_back(ProxyJob {
            key,
            src_path: src,
            dst_path: dst,
            max_dim,
        });

        ProxyLookup {
            path: None,
            status: ProxyStatus::Pending,
        }
    }

    pub fn lookup_proxy_for_path(&mut self, src_path: &str, max_dim: u32) -> ProxyLookup {
        let src = PathBuf::from(src_path);
        if !src.exists() {
            return ProxyLookup {
                path: None,
                status: ProxyStatus::Failed,
            };
        }

        let key = proxy_key(&src, max_dim);
        if let Some(entry) = self.proxy_entries.get(&key) {
            if entry.status == ProxyStatus::Ready && entry.path.exists() {
                return ProxyLookup {
                    path: Some(entry.path.to_string_lossy().to_string()),
                    status: ProxyStatus::Ready,
                };
            }
            if entry.status == ProxyStatus::Pending {
                return ProxyLookup {
                    path: None,
                    status: ProxyStatus::Pending,
                };
            }
        }

        let cache_root = self.cache_root_dir();
        let dst = proxy_path_for_in(&cache_root, &src, max_dim);
        if dst.exists() {
            let entry = ProxyEntry {
                status: ProxyStatus::Ready,
                path: dst.clone(),
                error: None,
            };
            self.proxy_entries.insert(key, entry);
            return ProxyLookup {
                path: Some(dst.to_string_lossy().to_string()),
                status: ProxyStatus::Ready,
            };
        }

        ProxyLookup {
            path: None,
            status: ProxyStatus::Missing,
        }
    }

    fn selected_video_proxy_sources(&self) -> Vec<PathBuf> {
        let mut selected_ids = self.selected_clip_ids.clone();
        if selected_ids.is_empty()
            && let Some(id) = self.selected_clip_id
        {
            selected_ids.push(id);
        }
        if selected_ids.is_empty() {
            return Vec::new();
        }

        let mut paths = HashSet::new();
        let mut push_clip = |clip: &Clip| {
            if selected_ids.contains(&clip.id) {
                let p = clip.file_path.to_lowercase();
                if p.ends_with(".jpg")
                    || p.ends_with(".jpeg")
                    || p.ends_with(".png")
                    || p.ends_with(".webp")
                    || p.ends_with(".bmp")
                    || p.ends_with(".gif")
                {
                    return;
                }
                paths.insert(PathBuf::from(&clip.file_path));
            }
        };

        for c in &self.v1_clips {
            push_clip(c);
        }
        for track in &self.video_tracks {
            for c in &track.clips {
                push_clip(c);
            }
        }

        paths.into_iter().collect()
    }

    pub fn delete_selected_proxy_for_quality(&mut self, max_dim: u32) -> ProxyDeleteReport {
        let mut report = ProxyDeleteReport::default();
        let sources = self.selected_video_proxy_sources();
        if sources.is_empty() {
            return report;
        }

        for src in sources {
            let key = proxy_key(&src, max_dim);
            let cache_root = self.cache_root_dir();
            let expected_path = proxy_path_for_in(&cache_root, &src, max_dim);

            if self.proxy_active.as_ref().is_some_and(|job| job.key == key) {
                report.blocked_active_jobs += 1;
                continue;
            }

            let mut new_queue = VecDeque::new();
            while let Some(job) = self.proxy_queue.pop_front() {
                if job.key == key {
                    report.removed_jobs += 1;
                } else {
                    new_queue.push_back(job);
                }
            }
            self.proxy_queue = new_queue;

            let mut path_to_delete = expected_path;
            if let Some(entry) = self.proxy_entries.remove(&key) {
                path_to_delete = entry.path;
            }

            if path_to_delete.exists() && fs::remove_file(&path_to_delete).is_ok() {
                report.deleted_files += 1;
            }
        }

        report
    }

    pub fn delete_all_proxies(&mut self) -> ProxyDeleteReport {
        let mut report = ProxyDeleteReport::default();
        let active_key = self.proxy_active.as_ref().map(|job| job.key.clone());
        let active_path = self.proxy_active.as_ref().map(|job| job.dst_path.clone());

        report.removed_jobs = self.proxy_queue.len();
        self.proxy_queue.clear();

        let mut keep_entries = HashMap::new();
        for (key, entry) in self.proxy_entries.drain() {
            if active_key.as_ref() == Some(&key) {
                keep_entries.insert(key, entry);
                report.blocked_active_jobs += 1;
                continue;
            }

            if entry.path.exists() && fs::remove_file(&entry.path).is_ok() {
                report.deleted_files += 1;
            }
        }
        self.proxy_entries = keep_entries;

        let cache_root = self.cache_root_dir();
        let dir = proxy_dir_for(&cache_root);
        if let Ok(read_dir) = fs::read_dir(&dir) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                if active_path.as_ref() == Some(&path) {
                    continue;
                }
                if path.is_file() && fs::remove_file(&path).is_ok() {
                    report.deleted_files += 1;
                }
            }
        }

        report
    }

    pub fn take_next_proxy_job(&mut self) -> Option<ProxyJob> {
        if self.proxy_active.is_some() {
            return None;
        }
        let next = self.proxy_queue.pop_front()?;
        self.proxy_active = Some(next.clone());
        Some(next)
    }

    pub fn finish_proxy_job(&mut self, key: &str, status: ProxyStatus, error: Option<String>) {
        if let Some(entry) = self.proxy_entries.get_mut(key) {
            entry.status = status;
            entry.error = error;
        }
        if let Some(active) = self.proxy_active.as_ref()
            && active.key == key
        {
            self.proxy_active = None;
        }
    }

    /// Read-only waveform lookup — returns cached peaks without mutating state.
    /// Use this during rendering to avoid triggering observer-based re-render loops.
    pub fn lookup_waveform_for_path(&self, src_path: &str, bucket_count: usize) -> WaveformLookup {
        if is_image_media_path(src_path) {
            return WaveformLookup {
                peaks: None,
                status: WaveformStatus::Missing,
            };
        }
        let src = PathBuf::from(src_path);
        if !src.exists() || bucket_count == 0 {
            return WaveformLookup {
                peaks: None,
                status: WaveformStatus::Failed,
            };
        }
        let key = waveform_key(&src, bucket_count);
        if let Some(entry) = self.waveform_entries.get(&key) {
            match entry.status {
                WaveformStatus::Ready => {
                    if let Some(peaks) = entry.peaks.clone() {
                        return WaveformLookup {
                            peaks: Some(peaks),
                            status: WaveformStatus::Ready,
                        };
                    }
                    // Peaks not yet loaded into memory — needs mutable access.
                    return WaveformLookup {
                        peaks: None,
                        status: WaveformStatus::Ready,
                    };
                }
                WaveformStatus::Pending => {
                    return WaveformLookup {
                        peaks: None,
                        status: WaveformStatus::Pending,
                    };
                }
                WaveformStatus::Failed => {
                    return WaveformLookup {
                        peaks: None,
                        status: WaveformStatus::Failed,
                    };
                }
                WaveformStatus::Missing => {}
            }
        }
        // Check if already queued
        if self
            .waveform_active
            .as_ref()
            .is_some_and(|job| job.key == key)
            || self.waveform_queue.iter().any(|job| job.key == key)
        {
            return WaveformLookup {
                peaks: None,
                status: WaveformStatus::Pending,
            };
        }
        // Not found — caller should use ensure_waveform_for_path via update()
        WaveformLookup {
            peaks: None,
            status: WaveformStatus::Missing,
        }
    }

    pub fn ensure_waveform_for_path(
        &mut self,
        src_path: &str,
        bucket_count: usize,
    ) -> WaveformLookup {
        if is_image_media_path(src_path) {
            return WaveformLookup {
                peaks: None,
                status: WaveformStatus::Missing,
            };
        }

        let src = PathBuf::from(src_path);
        if !src.exists() || bucket_count == 0 {
            return WaveformLookup {
                peaks: None,
                status: WaveformStatus::Failed,
            };
        }

        let key = waveform_key(&src, bucket_count);
        if let Some(entry) = self.waveform_entries.get_mut(&key) {
            match entry.status {
                WaveformStatus::Ready => {
                    // Load cache on-demand the first time we need this waveform in memory.
                    if entry.peaks.is_none()
                        && entry.path.exists()
                        && let Ok(peaks) = load_waveform_file(&entry.path)
                    {
                        entry.peaks = Some(std::sync::Arc::new(peaks));
                    }
                    if let Some(peaks) = entry.peaks.clone() {
                        return WaveformLookup {
                            peaks: Some(peaks),
                            status: WaveformStatus::Ready,
                        };
                    }
                }
                WaveformStatus::Pending => {
                    return WaveformLookup {
                        peaks: None,
                        status: WaveformStatus::Pending,
                    };
                }
                WaveformStatus::Failed => {
                    return WaveformLookup {
                        peaks: None,
                        status: WaveformStatus::Failed,
                    };
                }
                WaveformStatus::Missing => {}
            }
        }

        let cache_root = self.cache_root_dir();
        let dst = waveform_path_for_in(&cache_root, &src, bucket_count);
        if dst.exists()
            && let Ok(peaks) = load_waveform_file(&dst)
        {
            let peaks = std::sync::Arc::new(peaks);
            self.waveform_entries.insert(
                key,
                WaveformEntry {
                    status: WaveformStatus::Ready,
                    path: dst,
                    peaks: Some(peaks.clone()),
                    error: None,
                },
            );
            return WaveformLookup {
                peaks: Some(peaks),
                status: WaveformStatus::Ready,
            };
        }

        if self
            .waveform_active
            .as_ref()
            .is_some_and(|job| job.key == key)
            || self.waveform_queue.iter().any(|job| job.key == key)
        {
            return WaveformLookup {
                peaks: None,
                status: WaveformStatus::Pending,
            };
        }

        // Queue waveform generation once and let the preview worker process it in background.
        self.waveform_entries.insert(
            key.clone(),
            WaveformEntry {
                status: WaveformStatus::Pending,
                path: dst.clone(),
                peaks: None,
                error: None,
            },
        );
        info!(
            "[Waveform] enqueue {} -> {} ({})",
            src.to_string_lossy(),
            dst.to_string_lossy(),
            bucket_count
        );
        self.waveform_queue.push_back(WaveformJob {
            key,
            src_path: src,
            dst_path: dst,
            bucket_count,
        });

        WaveformLookup {
            peaks: None,
            status: WaveformStatus::Pending,
        }
    }

    pub fn take_next_waveform_job(&mut self) -> Option<WaveformJob> {
        if self.waveform_active.is_some() {
            return None;
        }
        let next = self.waveform_queue.pop_front()?;
        self.waveform_active = Some(next.clone());
        Some(next)
    }

    pub fn finish_waveform_job(
        &mut self,
        key: &str,
        status: WaveformStatus,
        error: Option<String>,
        peaks: Option<Vec<f32>>,
    ) {
        if let Some(entry) = self.waveform_entries.get_mut(key) {
            entry.status = status;
            entry.error = error;
            if status == WaveformStatus::Ready {
                entry.peaks = peaks.map(std::sync::Arc::new);
            } else {
                entry.peaks = None;
            }
        }
        if self
            .waveform_active
            .as_ref()
            .is_some_and(|active| active.key == key)
        {
            self.waveform_active = None;
        }
    }

    pub fn begin_transition_drag(&mut self, transition: TransitionType) {
        self.pending_transition = Some(transition);
    }

    pub fn clear_transition_drag(&mut self) {
        self.pending_transition = None;
    }

    pub fn apply_transition_to_clip(&mut self, clip_id: u64, transition: TransitionType) -> bool {
        match transition {
            TransitionType::Fade => self.apply_fade_transition(clip_id),
            TransitionType::Dissolve => self.apply_dissolve_transition(clip_id),
            TransitionType::Slide => self.apply_slide_transition(clip_id),
            TransitionType::Zoom => self.apply_zoom_transition(clip_id),
            TransitionType::ShockZoom => self.apply_shock_zoom_transition(clip_id),
        }
    }

    fn apply_fade_transition(&mut self, clip_id: u64) -> bool {
        const DEFAULT_FADE_SEC: f32 = 0.5;

        if self.v1_clips.iter().any(|c| c.id == clip_id) {
            self.save_for_undo();
            if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == clip_id) {
                c.set_fade_in(DEFAULT_FADE_SEC);
                c.set_fade_out(DEFAULT_FADE_SEC);
                c.set_dissolve_in(0.0);
                c.set_dissolve_out(0.0);
                self.ui_notice = None;
                return true;
            }
        }

        for idx in 0..self.video_tracks.len() {
            if self.video_tracks[idx].clips.iter().any(|c| c.id == clip_id) {
                self.save_for_undo();
                if let Some(c) = self.video_tracks[idx]
                    .clips
                    .iter_mut()
                    .find(|c| c.id == clip_id)
                {
                    c.set_fade_in(DEFAULT_FADE_SEC);
                    c.set_fade_out(DEFAULT_FADE_SEC);
                    c.set_dissolve_in(0.0);
                    c.set_dissolve_out(0.0);
                    self.ui_notice = None;
                    return true;
                }
            }
        }

        false
    }

    fn apply_dissolve_transition(&mut self, clip_id: u64) -> bool {
        const DEFAULT_DISSOLVE_SEC: f32 = 0.5;

        let find_overlap_pair = |clips: &[Clip]| -> Option<(usize, usize, f32)> {
            let target_idx = clips.iter().position(|c| c.id == clip_id)?;
            let target = &clips[target_idx];
            let mut best: Option<(usize, f32)> = None;
            for (idx, c) in clips.iter().enumerate() {
                if idx == target_idx {
                    continue;
                }
                let start = if target.start > c.start {
                    target.start
                } else {
                    c.start
                };
                let end = if target.end() < c.end() {
                    target.end()
                } else {
                    c.end()
                };
                if end > start {
                    let overlap = (end - start).as_secs_f32();
                    if overlap > 0.0
                        && best
                            .as_ref()
                            .map(|(_, best_overlap)| overlap > *best_overlap)
                            .unwrap_or(true)
                    {
                        best = Some((idx, overlap));
                    }
                }
            }
            best.map(|(idx, overlap)| (target_idx, idx, overlap))
        };
        let find_adjacent_pair = |clips: &[Clip]| -> Option<(usize, usize)> {
            let target_idx = clips.iter().position(|c| c.id == clip_id)?;
            if target_idx + 1 < clips.len() {
                return Some((target_idx, target_idx + 1));
            }
            if target_idx >= 1 {
                return Some((target_idx - 1, target_idx));
            }
            None
        };

        if self.v1_clips.iter().any(|c| c.id == clip_id) {
            let Some(target_idx) = self.v1_clips.iter().position(|c| c.id == clip_id) else {
                return false;
            };
            let (out_idx, in_idx) = if target_idx + 1 < self.v1_clips.len() {
                (target_idx, target_idx + 1)
            } else if target_idx >= 1 {
                (target_idx - 1, target_idx)
            } else {
                self.ui_notice = Some("Dissolve requires an adjacent clip on V1.".to_string());
                return false;
            };

            let left = &self.v1_clips[out_idx];
            let right = &self.v1_clips[in_idx];
            let left_post = left
                .media_duration
                .saturating_sub(left.source_in + left.duration)
                .as_secs_f32();
            let right_pre = right.source_in.as_secs_f32();
            let max_d = (left_post.min(right_pre)) * 2.0;
            let dur = DEFAULT_DISSOLVE_SEC.min(max_d);
            if dur <= 0.001 {
                self.ui_notice =
                    Some("Insufficient media (handles) for dissolve. Trim to fit?".to_string());
                self.pending_trim_to_fit = Some(PendingTrimToFit {
                    left_id: left.id,
                    right_id: right.id,
                    requested: DEFAULT_DISSOLVE_SEC,
                    track: PendingTrimTrack::V1,
                });
                return false;
            }

            self.save_for_undo();
            self.v1_clips[out_idx].set_dissolve_out(dur);
            self.v1_clips[out_idx].set_dissolve_in(0.0);
            self.v1_clips[in_idx].set_dissolve_in(dur);
            self.v1_clips[in_idx].set_dissolve_out(0.0);
            self.v1_clips[out_idx].set_fade_in(0.0);
            self.v1_clips[out_idx].set_fade_out(0.0);
            self.v1_clips[in_idx].set_fade_in(0.0);
            self.v1_clips[in_idx].set_fade_out(0.0);
            self.pending_trim_to_fit = None;
            self.ui_notice = None;
            return true;
        }

        for idx in 0..self.video_tracks.len() {
            if self.video_tracks[idx].clips.iter().any(|c| c.id == clip_id) {
                let mut use_overlap = true;
                let (target_idx, other_idx, overlap) =
                    if let Some(pair) = find_overlap_pair(&self.video_tracks[idx].clips) {
                        pair
                    } else {
                        use_overlap = false;
                        let Some((a, b)) = find_adjacent_pair(&self.video_tracks[idx].clips) else {
                            self.ui_notice = Some(
                            "Dissolve requires overlapping or adjacent clips on the same track."
                                .to_string(),
                        );
                            self.pending_trim_to_fit = None;
                            return false;
                        };
                        (a, b, 0.0)
                    };

                let (out_idx, in_idx) = if self.video_tracks[idx].clips[target_idx].start
                    <= self.video_tracks[idx].clips[other_idx].start
                {
                    (target_idx, other_idx)
                } else {
                    (other_idx, target_idx)
                };

                let clips = &self.video_tracks[idx].clips;
                let left = &clips[out_idx];
                let right = &clips[in_idx];
                if !use_overlap {
                    let left_end = left.start + left.duration;
                    if right.start > left_end + Duration::from_millis(1) {
                        self.ui_notice = Some(
                            "Dissolve requires overlapping or adjacent clips on the same track."
                                .to_string(),
                        );
                        self.pending_trim_to_fit = None;
                        return false;
                    }
                }

                let mut dur = if use_overlap {
                    overlap.min(DEFAULT_DISSOLVE_SEC)
                } else {
                    DEFAULT_DISSOLVE_SEC
                };
                dur = dur.min(left.duration.as_secs_f32());
                dur = dur.min(right.duration.as_secs_f32());
                if dur <= 0.001 {
                    self.ui_notice = Some(
                        "Dissolve requires overlapping or adjacent clips on the same track."
                            .to_string(),
                    );
                    self.pending_trim_to_fit = None;
                    return false;
                }
                if use_overlap && overlap + 0.0001 < DEFAULT_DISSOLVE_SEC {
                    self.ui_notice = Some("Not enough overlap for requested dissolve.".to_string());
                }
                self.pending_trim_to_fit = None;
                self.save_for_undo();
                let clips = &mut self.video_tracks[idx].clips;
                clips[out_idx].set_dissolve_out(dur);
                clips[out_idx].set_dissolve_in(0.0);
                clips[in_idx].set_dissolve_in(dur);
                clips[in_idx].set_dissolve_out(0.0);
                clips[out_idx].set_fade_in(0.0);
                clips[out_idx].set_fade_out(0.0);
                clips[in_idx].set_fade_in(0.0);
                clips[in_idx].set_fade_out(0.0);
                if use_overlap && overlap + 0.0001 >= DEFAULT_DISSOLVE_SEC {
                    self.ui_notice = None;
                }
                return true;
            }
        }

        false
    }

    pub fn apply_pending_trim_to_fit(&mut self) -> bool {
        let pending = match self.pending_trim_to_fit.clone() {
            Some(pending) => pending,
            None => return false,
        };

        let requested = pending.requested.max(0.0);
        if requested <= 0.001 {
            self.pending_trim_to_fit = None;
            self.ui_notice = Some("Invalid transition duration.".to_string());
            return false;
        }

        match pending.track {
            PendingTrimTrack::V1 => {
                let Some(left_idx) = self.v1_clips.iter().position(|c| c.id == pending.left_id)
                else {
                    self.pending_trim_to_fit = None;
                    self.ui_notice = Some("Clip no longer exists.".to_string());
                    return false;
                };
                let Some(right_idx) = self.v1_clips.iter().position(|c| c.id == pending.right_id)
                else {
                    self.pending_trim_to_fit = None;
                    self.ui_notice = Some("Clip no longer exists.".to_string());
                    return false;
                };

                let (left_idx, right_idx) = if left_idx <= right_idx {
                    (left_idx, right_idx)
                } else {
                    (right_idx, left_idx)
                };
                if right_idx <= left_idx {
                    self.pending_trim_to_fit = None;
                    self.ui_notice = Some("Dissolve requires adjacent clips on V1.".to_string());
                    return false;
                }

                let left = &self.v1_clips[left_idx];
                let right = &self.v1_clips[right_idx];
                let half = requested * 0.5;
                let left_post = left
                    .media_duration
                    .saturating_sub(left.source_in + left.duration)
                    .as_secs_f32();
                let right_pre = right.source_in.as_secs_f32();
                let need_left = (half - left_post).max(0.0);
                let need_right = (half - right_pre).max(0.0);

                let min_dur = Duration::from_millis(100);
                let max_trim_left = left.duration.saturating_sub(min_dur).as_secs_f32();
                let max_trim_right = right.duration.saturating_sub(min_dur).as_secs_f32();
                let trim_left = need_left.min(max_trim_left);
                let trim_right = need_right.min(max_trim_right);

                if trim_left <= 0.001 && trim_right <= 0.001 {
                    self.pending_trim_to_fit = None;
                    self.ui_notice = Some("Insufficient media (handles) for dissolve.".to_string());
                    return false;
                }

                self.save_for_undo();

                if left_idx < right_idx {
                    let (left_slice, right_slice) = self.v1_clips.split_at_mut(right_idx);
                    let left = &mut left_slice[left_idx];
                    let right = &mut right_slice[0];
                    if trim_left > 0.001 {
                        let delta = Duration::from_secs_f32(trim_left);
                        left.duration = left.duration.saturating_sub(delta);
                        left.dissolve_trim_out += delta;
                    }
                    if trim_right > 0.001 {
                        let delta = Duration::from_secs_f32(trim_right);
                        right.source_in += delta;
                        right.duration = right.duration.saturating_sub(delta);
                        right.dissolve_trim_in += delta;
                    }
                } else {
                    let (right_slice, left_slice) = self.v1_clips.split_at_mut(left_idx);
                    let right = &mut right_slice[right_idx];
                    let left = &mut left_slice[0];
                    if trim_left > 0.001 {
                        let delta = Duration::from_secs_f32(trim_left);
                        left.duration = left.duration.saturating_sub(delta);
                        left.dissolve_trim_out += delta;
                    }
                    if trim_right > 0.001 {
                        let delta = Duration::from_secs_f32(trim_right);
                        right.source_in += delta;
                        right.duration = right.duration.saturating_sub(delta);
                        right.dissolve_trim_in += delta;
                    }
                }

                self.ripple_v1_starts();

                let Some(left_idx) = self.v1_clips.iter().position(|c| c.id == pending.left_id)
                else {
                    self.pending_trim_to_fit = None;
                    self.ui_notice = Some("Clip no longer exists.".to_string());
                    return false;
                };
                let Some(right_idx) = self.v1_clips.iter().position(|c| c.id == pending.right_id)
                else {
                    self.pending_trim_to_fit = None;
                    self.ui_notice = Some("Clip no longer exists.".to_string());
                    return false;
                };
                let (left_idx, right_idx) = if left_idx <= right_idx {
                    (left_idx, right_idx)
                } else {
                    (right_idx, left_idx)
                };
                if right_idx <= left_idx {
                    self.pending_trim_to_fit = None;
                    self.ui_notice = Some("Dissolve requires adjacent clips on V1.".to_string());
                    return false;
                }

                let (left, right) = {
                    let (left_slice, right_slice) = self.v1_clips.split_at_mut(right_idx);
                    (&mut left_slice[left_idx], &mut right_slice[0])
                };
                let left_post = left
                    .media_duration
                    .saturating_sub(left.source_in + left.duration)
                    .as_secs_f32();
                let right_pre = right.source_in.as_secs_f32();
                let max_d = (left_post.min(right_pre)) * 2.0;
                let dur = requested.min(max_d);
                if dur <= 0.001 {
                    self.pending_trim_to_fit = None;
                    self.ui_notice =
                        Some("Insufficient media (handles) even after trim.".to_string());
                    return false;
                }

                left.set_dissolve_out(dur);
                left.set_dissolve_in(0.0);
                right.set_dissolve_in(dur);
                right.set_dissolve_out(0.0);
                left.set_fade_in(0.0);
                left.set_fade_out(0.0);
                right.set_fade_in(0.0);
                right.set_fade_out(0.0);
                self.pending_trim_to_fit = None;
                if dur + 0.0001 < requested {
                    self.ui_notice = Some(format!(
                        "Trimmed to fit: dissolve shortened to {:.2}s.",
                        dur
                    ));
                } else {
                    self.ui_notice = None;
                }
                true
            }
        }
    }

    fn ripple_v1_starts(&mut self) {
        self.v1_clips.sort_by_key(|c| c.start);
        let mut cursor = Duration::ZERO;
        for c in &mut self.v1_clips {
            c.start = cursor;
            cursor += c.duration;
        }
    }

    fn apply_slide_transition(&mut self, clip_id: u64) -> bool {
        const DEFAULT_SLIDE_SEC: f32 = 0.5;
        let in_dir = SlideDirection::Right;
        let out_dir = SlideDirection::Left;

        if self.v1_clips.iter().any(|c| c.id == clip_id) {
            self.save_for_undo();
            if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == clip_id) {
                c.set_slide(in_dir, out_dir, DEFAULT_SLIDE_SEC, DEFAULT_SLIDE_SEC);
                self.ui_notice = None;
                return true;
            }
        }

        for idx in 0..self.video_tracks.len() {
            if self.video_tracks[idx].clips.iter().any(|c| c.id == clip_id) {
                self.save_for_undo();
                if let Some(c) = self.video_tracks[idx]
                    .clips
                    .iter_mut()
                    .find(|c| c.id == clip_id)
                {
                    c.set_slide(in_dir, out_dir, DEFAULT_SLIDE_SEC, DEFAULT_SLIDE_SEC);
                    self.ui_notice = None;
                    return true;
                }
            }
        }

        false
    }

    fn apply_zoom_transition(&mut self, clip_id: u64) -> bool {
        const DEFAULT_ZOOM_SEC: f32 = 0.5;
        const DEFAULT_ZOOM_AMOUNT: f32 = 1.1;

        if self.v1_clips.iter().any(|c| c.id == clip_id) {
            self.save_for_undo();
            if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == clip_id) {
                c.set_zoom(DEFAULT_ZOOM_SEC, DEFAULT_ZOOM_SEC, DEFAULT_ZOOM_AMOUNT);
                self.ui_notice = None;
                return true;
            }
        }

        for idx in 0..self.video_tracks.len() {
            if self.video_tracks[idx].clips.iter().any(|c| c.id == clip_id) {
                self.save_for_undo();
                if let Some(c) = self.video_tracks[idx]
                    .clips
                    .iter_mut()
                    .find(|c| c.id == clip_id)
                {
                    c.set_zoom(DEFAULT_ZOOM_SEC, DEFAULT_ZOOM_SEC, DEFAULT_ZOOM_AMOUNT);
                    self.ui_notice = None;
                    return true;
                }
            }
        }

        false
    }

    fn apply_shock_zoom_transition(&mut self, clip_id: u64) -> bool {
        const DEFAULT_SHOCK_SEC: f32 = 0.2;
        const DEFAULT_SHOCK_AMOUNT: f32 = 1.4;

        if let Some(idx) = self.v1_clips.iter().position(|c| c.id == clip_id) {
            self.save_for_undo();
            let len = self.v1_clips.len();
            if len >= 2 {
                if idx + 1 < len {
                    let (left, right) = self.v1_clips.split_at_mut(idx + 1);
                    let left_clip = &mut left[idx];
                    let right_clip = &mut right[0];
                    left_clip.set_shock_zoom(0.0, DEFAULT_SHOCK_SEC, DEFAULT_SHOCK_AMOUNT);
                    right_clip.set_shock_zoom(DEFAULT_SHOCK_SEC, 0.0, DEFAULT_SHOCK_AMOUNT);
                } else if idx > 0 {
                    let (left, right) = self.v1_clips.split_at_mut(idx);
                    let left_clip = &mut left[idx - 1];
                    let right_clip = &mut right[0];
                    left_clip.set_shock_zoom(0.0, DEFAULT_SHOCK_SEC, DEFAULT_SHOCK_AMOUNT);
                    right_clip.set_shock_zoom(DEFAULT_SHOCK_SEC, 0.0, DEFAULT_SHOCK_AMOUNT);
                } else {
                    self.v1_clips[idx].set_shock_zoom(
                        DEFAULT_SHOCK_SEC,
                        DEFAULT_SHOCK_SEC,
                        DEFAULT_SHOCK_AMOUNT,
                    );
                }
            } else {
                self.v1_clips[idx].set_shock_zoom(
                    DEFAULT_SHOCK_SEC,
                    DEFAULT_SHOCK_SEC,
                    DEFAULT_SHOCK_AMOUNT,
                );
            }
            self.ui_notice = None;
            return true;
        }

        for t_idx in 0..self.video_tracks.len() {
            if let Some(idx) = self.video_tracks[t_idx]
                .clips
                .iter()
                .position(|c| c.id == clip_id)
            {
                self.save_for_undo();
                let len = self.video_tracks[t_idx].clips.len();
                if len >= 2 {
                    if idx + 1 < len {
                        let (left, right) = self.video_tracks[t_idx].clips.split_at_mut(idx + 1);
                        let left_clip = &mut left[idx];
                        let right_clip = &mut right[0];
                        left_clip.set_shock_zoom(0.0, DEFAULT_SHOCK_SEC, DEFAULT_SHOCK_AMOUNT);
                        right_clip.set_shock_zoom(DEFAULT_SHOCK_SEC, 0.0, DEFAULT_SHOCK_AMOUNT);
                    } else if idx > 0 {
                        let (left, right) = self.video_tracks[t_idx].clips.split_at_mut(idx);
                        let left_clip = &mut left[idx - 1];
                        let right_clip = &mut right[0];
                        left_clip.set_shock_zoom(0.0, DEFAULT_SHOCK_SEC, DEFAULT_SHOCK_AMOUNT);
                        right_clip.set_shock_zoom(DEFAULT_SHOCK_SEC, 0.0, DEFAULT_SHOCK_AMOUNT);
                    } else {
                        self.video_tracks[t_idx].clips[idx].set_shock_zoom(
                            DEFAULT_SHOCK_SEC,
                            DEFAULT_SHOCK_SEC,
                            DEFAULT_SHOCK_AMOUNT,
                        );
                    }
                } else {
                    self.video_tracks[t_idx].clips[idx].set_shock_zoom(
                        DEFAULT_SHOCK_SEC,
                        DEFAULT_SHOCK_SEC,
                        DEFAULT_SHOCK_AMOUNT,
                    );
                }
                self.ui_notice = None;
                return true;
            }
        }

        false
    }

    pub fn load_source_video(&mut self, path: PathBuf, duration: Duration) {
        // Add dropped/imported media into the pool so users can reuse it later.
        self.add_media_pool_item(path.clone(), duration);
        self.active_source_path = path.to_string_lossy().to_string();

        if let Some(name) = path.file_name() {
            self.active_source_name = name.to_string_lossy().to_string();
        } else {
            self.active_source_name = "Unknown".to_string();
        }

        self.active_source_duration = duration;
    }

    pub fn add_generated_asset_to_semantic_timeline(
        &mut self,
        semantic_clip_id: u64,
        path: PathBuf,
        media_duration: Duration,
    ) -> Result<(), GlobalStateError> {
        let Some(semantic) = self
            .semantic_clips
            .iter()
            .find(|clip| clip.id == semantic_clip_id)
            .cloned()
        else {
            return Err(GlobalStateError::TargetSemanticClipMissing);
        };

        let path_str = path.to_string_lossy().to_string();
        if !is_supported_media_path(&path_str) {
            return Err(GlobalStateError::UnsupportedGeneratedMediaPath);
        }

        let media_duration = media_duration.max(Duration::from_millis(1));
        let semantic_duration = semantic.duration.max(Duration::from_millis(1));
        let is_image = is_image_media_path(&path_str);
        let timeline_duration = if is_image {
            semantic_duration
        } else {
            semantic_duration
                .min(media_duration)
                .max(Duration::from_millis(1))
        };

        self.save_for_undo_with_media_pool();
        self.add_media_pool_item(path.clone(), media_duration);

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| "generated_asset".to_string());

        self.active_source_path = path_str.clone();
        self.active_source_name = name.clone();
        self.active_source_duration = media_duration;

        if self.video_tracks.is_empty() {
            self.video_tracks.push(VideoTrack::new("V2".to_string()));
        }

        let clip_id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);
        self.video_tracks[0].clips.push(Clip {
            id: clip_id,
            label: if is_image {
                format!("(Image) {}", name)
            } else {
                format!("(Video) {}", name)
            },
            file_path: path_str,
            start: semantic.start,
            duration: timeline_duration,
            source_in: Duration::ZERO,
            media_duration,
            link_group_id: None,
            audio_gain_db: 0.0,
            dissolve_trim_in: Duration::ZERO,
            dissolve_trim_out: Duration::ZERO,
            video_effects: VideoEffect::standard_set(),
            local_mask_layers: vec![LocalMaskLayer::default()],
            pos_x_keyframes: Vec::new(),
            pos_y_keyframes: Vec::new(),
            scale_keyframes: Vec::new(),
            brightness_keyframes: Vec::new(),
            contrast_keyframes: Vec::new(),
            saturation_keyframes: Vec::new(),
            opacity_keyframes: Vec::new(),
            blur_keyframes: Vec::new(),
            rotation_keyframes: Vec::new(),
        });
        self.video_tracks[0].clips.sort_by_key(|clip| clip.start);

        self.playhead = semantic.start;
        self.selected_clip_id = None;
        self.selected_clip_ids.clear();
        self.selected_subtitle_id = None;
        self.selected_subtitle_ids.clear();
        self.selected_layer_effect_clip_id = None;
        self.ui_notice = Some(format!(
            "Generated media placed at semantic {:.2}s on V2 and added to Media Pool.",
            semantic.start.as_secs_f32()
        ));

        Ok(())
    }

    pub fn add_media_pool_item(&mut self, path: PathBuf, duration: Duration) {
        let path_str = path.to_string_lossy().to_string();
        if !is_supported_media_path(&path_str) {
            return;
        }
        if let Some(existing) = self
            .media_pool
            .iter_mut()
            .find(|item| item.path == path_str)
        {
            existing.duration = duration;
            return;
        }

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        self.media_pool.push(MediaPoolItem {
            path: path_str,
            name,
            duration,
            preview_jpeg_base64: None,
        });
    }

    pub fn remove_media_pool_item(&mut self, path: &str) -> bool {
        if !self.media_pool.iter().any(|item| item.path == path) {
            return false;
        }

        // Capture both timeline and media pool before removal so Cmd+Z can restore both.
        self.save_for_undo_with_media_pool();

        let timeline_has_related_clips = self.v1_clips.iter().any(|clip| clip.file_path == path)
            || self
                .audio_tracks
                .iter()
                .any(|track| track.clips.iter().any(|clip| clip.file_path == path))
            || self
                .video_tracks
                .iter()
                .any(|track| track.clips.iter().any(|clip| clip.file_path == path));

        let before_len = self.media_pool.len();
        self.media_pool.retain(|item| item.path != path);
        if self.media_pool.len() == before_len {
            return false;
        }
        if timeline_has_related_clips {
            self.remove_timeline_clips_for_media_path(path);
        }

        if self.pending_media_pool_path.as_deref() == Some(path) {
            self.pending_media_pool_path = None;
        }
        if self.media_pool_drag.as_ref().map(|drag| drag.path.as_str()) == Some(path) {
            self.media_pool_drag = None;
        }
        if self
            .media_pool_context_menu
            .as_ref()
            .map(|menu| menu.path.as_str())
            == Some(path)
        {
            self.media_pool_context_menu = None;
        }

        if self.active_source_path == path {
            if let Some(next) = self.media_pool.first().cloned() {
                self.active_source_path = next.path;
                self.active_source_name = next.name;
                self.active_source_duration = next.duration;
            } else {
                self.active_source_path.clear();
                self.active_source_name = "No Source Loaded".to_string();
                self.active_source_duration = Duration::ZERO;
            }
        }

        true
    }

    fn remove_timeline_clips_for_media_path(&mut self, path: &str) {
        let mut removed_v1_ranges = Vec::new();

        self.v1_clips.retain(|clip| {
            let should_remove = clip.file_path == path;
            if should_remove {
                removed_v1_ranges.push((clip.start, clip.end()));
            }
            !should_remove
        });

        for track in &mut self.audio_tracks {
            track.clips.retain(|clip| clip.file_path != path);
        }
        for track in &mut self.video_tracks {
            track.clips.retain(|clip| clip.file_path != path);
        }

        // Keep default behavior non-ripple. Only remove timeline time when V1 is magnetic.
        if matches!(self.v1_move_mode, V1MoveMode::Magnetic) && !removed_v1_ranges.is_empty() {
            let merged_ranges = Self::merge_time_ranges(removed_v1_ranges);
            if !merged_ranges.is_empty() {
                self.apply_time_removal_to_av_tracks(&merged_ranges);
            }
        }

        self.prune_clip_selection_after_timeline_remove();
    }

    fn merge_time_ranges(mut ranges: Vec<(Duration, Duration)>) -> Vec<(Duration, Duration)> {
        ranges.retain(|(start, end)| end > start);
        if ranges.is_empty() {
            return Vec::new();
        }

        ranges.sort_by_key(|(start, _)| *start);
        let mut merged = Vec::with_capacity(ranges.len());
        for (start, end) in ranges {
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
        merged
    }

    fn remap_time_after_removed_ranges(
        t: Duration,
        removed_ranges: &[(Duration, Duration)],
    ) -> Duration {
        let mut removed_before = Duration::ZERO;
        for (start, end) in removed_ranges {
            if t < *start {
                break;
            }
            if t >= *end {
                removed_before += *end - *start;
                continue;
            }
            return start.saturating_sub(removed_before);
        }
        t.saturating_sub(removed_before)
    }

    fn apply_time_removal_to_av_tracks(&mut self, removed_ranges: &[(Duration, Duration)]) {
        for clip in &mut self.v1_clips {
            clip.start = Self::remap_time_after_removed_ranges(clip.start, removed_ranges);
        }
        self.v1_clips.sort_by_key(|clip| clip.start);

        for track in &mut self.audio_tracks {
            for clip in &mut track.clips {
                clip.start = Self::remap_time_after_removed_ranges(clip.start, removed_ranges);
            }
            track.clips.sort_by_key(|clip| clip.start);
        }

        for track in &mut self.video_tracks {
            for clip in &mut track.clips {
                clip.start = Self::remap_time_after_removed_ranges(clip.start, removed_ranges);
            }
            track.clips.sort_by_key(|clip| clip.start);
        }

        self.playhead = Self::remap_time_after_removed_ranges(self.playhead, removed_ranges);
    }

    fn prune_clip_selection_after_timeline_remove(&mut self) {
        let existing_clip_ids: HashSet<u64> = self
            .v1_clips
            .iter()
            .chain(self.audio_tracks.iter().flat_map(|t| t.clips.iter()))
            .chain(self.video_tracks.iter().flat_map(|t| t.clips.iter()))
            .map(|c| c.id)
            .collect();
        self.selected_clip_ids
            .retain(|id| existing_clip_ids.contains(id));
        if self
            .selected_clip_id
            .is_some_and(|id| !existing_clip_ids.contains(&id))
        {
            self.selected_clip_id = self.selected_clip_ids.last().copied();
        }
    }

    pub fn open_media_pool_context_menu(&mut self, path: String, x: f32, y: f32) -> bool {
        if !self.media_pool.iter().any(|item| item.path == path) {
            self.media_pool_context_menu = None;
            return false;
        }
        self.media_pool_context_menu = Some(MediaPoolContextMenuState { path, x, y });
        true
    }

    pub fn close_media_pool_context_menu(&mut self) {
        self.media_pool_context_menu = None;
    }

    /// Replace old_path with new_path in media pool and all timeline clips.
    pub fn relocate_media_path(&mut self, old_path: &str, new_path: &str) {
        // Update media pool item and clear cached preview so thumbnail regenerates.
        for item in &mut self.media_pool {
            if item.path == old_path {
                item.path = new_path.to_string();
                item.name = std::path::Path::new(new_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| new_path.to_string());
                item.preview_jpeg_base64 = None;
            }
        }
        // Update V1 clips.
        for clip in &mut self.v1_clips {
            if clip.file_path == old_path {
                clip.file_path = new_path.to_string();
            }
        }
        // Update audio track clips.
        for track in &mut self.audio_tracks {
            for clip in &mut track.clips {
                if clip.file_path == old_path {
                    clip.file_path = new_path.to_string();
                }
            }
        }
        // Update video track clips.
        for track in &mut self.video_tracks {
            for clip in &mut track.clips {
                if clip.file_path == old_path {
                    clip.file_path = new_path.to_string();
                }
            }
        }
        // Update active source if it matches.
        if self.active_source_path == old_path {
            self.active_source_path = new_path.to_string();
        }
    }

    pub fn activate_media_pool_item(&mut self, path: &str) -> bool {
        let Some(item) = self
            .media_pool
            .iter()
            .find(|item| item.path == path)
            .cloned()
        else {
            return false;
        };
        self.active_source_path = item.path.clone();
        self.active_source_name = item.name.clone();
        self.active_source_duration = item.duration;
        true
    }

    pub fn begin_media_pool_drag(&mut self, path: String) -> bool {
        // Arm both drop payload and drag payload for internal media pool DnD.
        let Some(item) = self
            .media_pool
            .iter()
            .find(|item| item.path == path)
            .cloned()
        else {
            return false;
        };
        self.pending_media_pool_path = Some(item.path.clone());
        self.media_pool_drag = Some(MediaPoolDragState {
            path: item.path,
            name: item.name,
            cursor_x: 0.0,
            cursor_y: 0.0,
        });
        self.media_pool_context_menu = None;
        true
    }

    pub fn update_media_pool_drag_cursor(&mut self, x: f32, y: f32) {
        // Keep ghost cursor in sync while dragging across editor views.
        if let Some(drag) = self.media_pool_drag.as_mut() {
            drag.cursor_x = x;
            drag.cursor_y = y;
        }
    }

    pub fn clear_media_pool_drag(&mut self) {
        // Clear both armed click-drop and active drag state.
        self.pending_media_pool_path = None;
        self.media_pool_drag = None;
    }

    pub fn toggle_playing(&mut self) {
        self.is_playing = !self.is_playing;
    }

    pub fn set_tool(&mut self, tool: ActiveTool) {
        self.active_tool = tool;
    }

    pub fn set_active_page(&mut self, page: AppPage) {
        self.active_page = page;
    }

    pub fn active_local_mask_layer(&self) -> usize {
        let max_layers = self
            .get_selected_clip_local_mask_layer_count()
            .unwrap_or(1)
            .clamp(1, MAX_LOCAL_MASK_LAYERS);
        self.active_local_mask_layer
            .min(max_layers.saturating_sub(1))
            .min(MAX_LOCAL_MASK_LAYERS.saturating_sub(1))
    }

    pub fn set_active_local_mask_layer(&mut self, layer_index: usize) {
        let max_layers = self
            .get_selected_clip_local_mask_layer_count()
            .unwrap_or(1)
            .clamp(1, MAX_LOCAL_MASK_LAYERS);
        self.active_local_mask_layer = layer_index
            .min(max_layers.saturating_sub(1))
            .min(MAX_LOCAL_MASK_LAYERS.saturating_sub(1));
    }

    pub fn set_playhead(&mut self, t: Duration) {
        self.playhead = t.min(self.sequence_duration());
    }

    pub fn sequence_duration(&self) -> Duration {
        let v1_end = self
            .v1_clips
            .last()
            .map(|c| c.end())
            .unwrap_or(Duration::ZERO);
        let audio_end = self
            .audio_tracks
            .iter()
            .flat_map(|t| t.clips.last().map(|c| c.end()))
            .max()
            .unwrap_or(Duration::ZERO);
        let video_end = self
            .video_tracks
            .iter()
            .flat_map(|t| t.clips.last().map(|c| c.end()))
            .max()
            .unwrap_or(Duration::ZERO);
        let subtitle_end = self
            .subtitle_tracks
            .iter()
            .flat_map(|t| t.clips.last().map(|c| c.end()))
            .max()
            .unwrap_or(Duration::ZERO);
        let semantic_end = self
            .semantic_clips
            .last()
            .map(|c| c.end())
            .unwrap_or(Duration::ZERO);

        v1_end
            .max(audio_end)
            .max(video_end)
            .max(subtitle_end)
            .max(semantic_end)
    }

    pub fn timeline_total(&self) -> Duration {
        let min_total = Duration::from_secs(60);
        let rounded = round_up_to_step(self.sequence_duration(), Duration::from_secs(15));
        std::cmp::max(min_total, rounded)
    }

    // ----------------------------------------------------
    // Selection Methods
    // ----------------------------------------------------

    // [Added] Select only V1
    pub fn select_v1_clip_at(&mut self, t: Duration) {
        self.selected_layer_effect_clip_id = None;
        self.selected_semantic_clip_id = None;
        self.selected_clip_id = None; // Clear first, then set it only if a clip is actually hit
        self.selected_subtitle_id = None;
        self.selected_clip_ids.clear();
        self.selected_subtitle_ids.clear();
        for clip in &self.v1_clips {
            if t >= clip.start && t < clip.end() {
                self.selected_clip_id = Some(clip.id);
                self.selected_clip_ids.push(clip.id);
                break;
            }
        }
    }

    // [Added] Select only Audio
    pub fn select_audio_clip_at(&mut self, track_index: usize, t: Duration) {
        self.selected_layer_effect_clip_id = None;
        self.selected_semantic_clip_id = None;
        self.selected_clip_id = None;
        self.selected_subtitle_id = None;
        self.selected_clip_ids.clear();
        self.selected_subtitle_ids.clear();
        if let Some(track) = self.audio_tracks.get(track_index) {
            for clip in &track.clips {
                if t >= clip.start && t < clip.end() {
                    self.selected_clip_id = Some(clip.id);
                    self.selected_clip_ids.push(clip.id);
                    break;
                }
            }
        }
    }

    pub fn select_video_clip_at(&mut self, track_index: usize, t: Duration) {
        self.selected_layer_effect_clip_id = None;
        self.selected_semantic_clip_id = None;
        self.selected_clip_id = None;
        self.selected_subtitle_id = None;
        self.selected_clip_ids.clear();
        self.selected_subtitle_ids.clear();
        if let Some(track) = self.video_tracks.get(track_index) {
            for clip in &track.clips {
                if t >= clip.start && t < clip.end() {
                    self.selected_clip_id = Some(clip.id);
                    self.selected_clip_ids.push(clip.id);
                    break;
                }
            }
        }
    }

    pub fn select_subtitle_clip_at(&mut self, track_index: usize, t: Duration) {
        self.selected_layer_effect_clip_id = None;
        self.selected_semantic_clip_id = None;
        self.selected_subtitle_id = None;
        self.selected_clip_id = None;
        self.selected_subtitle_ids.clear();
        self.selected_clip_ids.clear();
        if let Some(track) = self.subtitle_tracks.get(track_index) {
            for clip in &track.clips {
                if t >= clip.start && t < clip.end() {
                    self.selected_subtitle_id = Some(clip.id);
                    self.selected_subtitle_ids.push(clip.id);
                    break;
                }
            }
        }
    }

    // ----------------------------------------------------
    // [Fix] Move / drag methods (supports dragging)
    // ----------------------------------------------------

    // Magnetic V1 move (reorder + ripple)
    pub fn move_v1_clip_magnetic(&mut self, clip_id: u64, new_start_hint: Duration) {
        self.repair_missing_primary_av_links();
        let old_v1_group_starts = self.v1_group_anchor_starts();

        // 1. Find and remove
        let idx = match self.v1_clips.iter().position(|c| c.id == clip_id) {
            Some(i) => i,
            None => return,
        };
        let clip = self.v1_clips.remove(idx);

        // 2. Find the new insertion point (based on time)
        let mut insert_idx = self.v1_clips.len();
        // Simple rule: if the dragged time is before a clip midpoint, insert before that clip
        let mut cursor = Duration::ZERO;
        for (i, c) in self.v1_clips.iter().enumerate() {
            let mid = cursor + (c.duration / 2);
            if new_start_hint < mid {
                insert_idx = i;
                break;
            }
            cursor += c.duration;
        }

        // 3. Insert
        self.v1_clips.insert(insert_idx, clip);

        // 4. Recalculate all times (ripple)
        let mut current_pos = Duration::ZERO;
        for c in &mut self.v1_clips {
            c.start = current_pos;
            current_pos += c.duration;
        }
        // Keep linked overlays/audio in-step with actual V1 movement without forcing absolute start alignment.
        self.sync_linked_tracks_from_v1_deltas(&old_v1_group_starts);
    }

    pub fn move_v1_clip_free(&mut self, clip_id: u64, new_start: Duration) {
        self.repair_missing_primary_av_links();

        // Keep toolbar mode consistent with actual behavior once any free move is applied to V1.
        if matches!(self.v1_move_mode, V1MoveMode::Magnetic) {
            self.v1_move_mode = V1MoveMode::Free;
        }

        let clip_info = self
            .v1_clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .map(|clip| (clip.start, clip.link_group_id));
        let Some((old_start, link_group_id)) = clip_info else {
            return;
        };
        let requested_delta_sec = new_start.as_secs_f64() - old_start.as_secs_f64();

        if let Some(link_group_id) = link_group_id {
            // Move the entire linked group by one clamped delta to avoid per-clip drift at the timeline wall.
            self.sync_linked_group_delta(link_group_id, requested_delta_sec, None);
            return;
        }

        if let Some(clip) = self.v1_clips.iter_mut().find(|clip| clip.id == clip_id) {
            clip.start = new_start;
        }
        self.v1_clips.sort_by_key(|clip| clip.start);
    }

    // Free audio movement
    pub fn move_audio_clip_free(&mut self, track_index: usize, clip_id: u64, new_start: Duration) {
        self.repair_missing_primary_av_links();

        let mut moved_link_group = None;
        let mut requested_delta_sec = 0.0;
        let mut blocked_linked_audio_drag = false;
        if let Some(track) = self.audio_tracks.get_mut(track_index) {
            if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                // Keep linked A1 audio locked to its parent until user explicitly unlinks the group.
                if clip.link_group_id.is_some() {
                    blocked_linked_audio_drag = true;
                } else {
                    requested_delta_sec = new_start.as_secs_f64() - clip.start.as_secs_f64();
                    clip.start = new_start;
                    moved_link_group = clip.link_group_id;
                }
            }
            // Keep free tracks sorted for easier management, even though visual overlap is allowed
            if !blocked_linked_audio_drag {
                track.clips.sort_by_key(|c| c.start);
            }
        }
        if blocked_linked_audio_drag {
            self.ui_notice =
                Some("Linked audio is locked. Unlink clip first to drag audio.".to_string());
            return;
        }
        self.ui_notice = None;
        // Moving a linked audio clip should move its linked video companion until user unlinks.
        if let Some(link_group_id) = moved_link_group {
            self.sync_linked_group_delta(link_group_id, requested_delta_sec, Some(clip_id));
        }
    }
    pub fn move_video_clip_free(&mut self, track_index: usize, clip_id: u64, new_start: Duration) {
        self.repair_missing_primary_av_links();

        let clip_info = self
            .video_tracks
            .get(track_index)
            .and_then(|track| track.clips.iter().find(|clip| clip.id == clip_id))
            .map(|clip| (clip.start, clip.link_group_id));
        let Some((old_start, linked_group_id)) = clip_info else {
            return;
        };
        if let Some(link_group_id) = linked_group_id
            && matches!(self.v1_move_mode, V1MoveMode::Magnetic)
            && self.link_group_has_v1(link_group_id)
        {
            // When V1 is magnetic, linked overlay clips cannot move independently.
            self.ui_notice =
                Some("Linked V2/V3 is locked while V1 is Magnetic. Move V1 instead.".to_string());
            return;
        }
        self.ui_notice = None;

        let requested_delta_sec = new_start.as_secs_f64() - old_start.as_secs_f64();
        if let Some(link_group_id) = linked_group_id {
            // When linked to V1, block leftward overlay drag once V1 reaches timeline start.
            if requested_delta_sec < 0.0
                && let Some(min_v1_start_sec) = self.link_group_min_v1_start_sec(link_group_id)
                && min_v1_start_sec <= 1e-6
            {
                return;
            }
            // Move linked group as one unit so V2/V3 cannot drift ahead when V1 hits timeline start.
            self.sync_linked_group_delta(link_group_id, requested_delta_sec, None);
            return;
        }

        if let Some(track) = self.video_tracks.get_mut(track_index) {
            if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                clip.start = new_start;
            }
            // Keep free tracks sorted for easier management, even though visual overlap is allowed
            track.clips.sort_by_key(|c| c.start);
        }
    }

    // Move a V2+ clip by id and optionally re-route it to another V2+ track during drag.
    pub fn move_video_clip_free_any_track(
        &mut self,
        clip_id: u64,
        new_start: Duration,
        target_track_index: Option<usize>,
    ) {
        self.repair_missing_primary_av_links();

        let mut source_track_index = None;
        let mut old_start = Duration::ZERO;
        let mut linked_group_id = None;
        for (track_idx, track) in self.video_tracks.iter().enumerate() {
            if let Some(clip) = track.clips.iter().find(|c| c.id == clip_id) {
                source_track_index = Some(track_idx);
                old_start = clip.start;
                linked_group_id = clip.link_group_id;
                break;
            }
        }
        let Some(source_track_index) = source_track_index else {
            return;
        };

        if let Some(link_group_id) = linked_group_id
            && matches!(self.v1_move_mode, V1MoveMode::Magnetic)
            && self.link_group_has_v1(link_group_id)
        {
            self.ui_notice =
                Some("Linked V2/V3 is locked while V1 is Magnetic. Move V1 instead.".to_string());
            return;
        }
        self.ui_notice = None;

        let requested_delta_sec = new_start.as_secs_f64() - old_start.as_secs_f64();
        if let Some(link_group_id) = linked_group_id {
            if requested_delta_sec < 0.0
                && let Some(min_v1_start_sec) = self.link_group_min_v1_start_sec(link_group_id)
                && min_v1_start_sec <= 1e-6
            {
                return;
            }
            self.sync_linked_group_delta(link_group_id, requested_delta_sec, None);
        } else {
            if let Some(track) = self.video_tracks.get_mut(source_track_index)
                && let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id)
            {
                clip.start = new_start;
            }
            if let Some(track) = self.video_tracks.get_mut(source_track_index) {
                track.clips.sort_by_key(|c| c.start);
            }
        }

        let Some(target_track_index) = target_track_index else {
            return;
        };
        if target_track_index >= self.video_tracks.len() {
            return;
        }

        let mut current_track = None;
        let mut current_pos = None;
        for (track_idx, track) in self.video_tracks.iter().enumerate() {
            if let Some(pos) = track.clips.iter().position(|c| c.id == clip_id) {
                current_track = Some(track_idx);
                current_pos = Some(pos);
                break;
            }
        }
        let (Some(current_track), Some(current_pos)) = (current_track, current_pos) else {
            return;
        };
        if current_track == target_track_index {
            return;
        }

        let clip = self.video_tracks[current_track].clips.remove(current_pos);
        self.video_tracks[target_track_index].clips.push(clip);
        self.video_tracks[target_track_index]
            .clips
            .sort_by_key(|c| c.start);
    }

    // Check if any currently selected clip belongs to a link group.
    pub fn selected_clips_have_any_link_group(&self) -> bool {
        let mut selected = self.selected_clip_ids.clone();
        if selected.is_empty()
            && let Some(id) = self.selected_clip_id
        {
            selected.push(id);
        }
        selected
            .into_iter()
            .any(|clip_id| self.clip_link_group_id(clip_id).is_some())
    }

    // Create one unified A/V link group for the currently selected timeline clips.
    pub fn link_selected_clips_into_group(&mut self) -> bool {
        let mut selected = self.selected_clip_ids.clone();
        if selected.is_empty()
            && let Some(id) = self.selected_clip_id
        {
            selected.push(id);
        }
        if selected.len() < 2 {
            return false;
        }

        let selected_set: HashSet<u64> = selected.into_iter().collect();
        let mut matched_ids = HashSet::new();

        for clip in &self.v1_clips {
            if selected_set.contains(&clip.id) {
                matched_ids.insert(clip.id);
            }
        }
        for track in &self.audio_tracks {
            for clip in &track.clips {
                if selected_set.contains(&clip.id) {
                    matched_ids.insert(clip.id);
                }
            }
        }
        for track in &self.video_tracks {
            for clip in &track.clips {
                if selected_set.contains(&clip.id) {
                    matched_ids.insert(clip.id);
                }
            }
        }

        if matched_ids.len() < 2 {
            return false;
        }

        self.save_for_undo();
        let link_group_id = self.allocate_next_link_group_id();

        // Assign one fresh group id so selected clips move as a single linked unit.
        for clip in &mut self.v1_clips {
            if matched_ids.contains(&clip.id) {
                clip.link_group_id = Some(link_group_id);
            }
        }
        for track in &mut self.audio_tracks {
            for clip in &mut track.clips {
                if matched_ids.contains(&clip.id) {
                    clip.link_group_id = Some(link_group_id);
                }
            }
        }
        for track in &mut self.video_tracks {
            for clip in &mut track.clips {
                if matched_ids.contains(&clip.id) {
                    clip.link_group_id = Some(link_group_id);
                }
            }
        }

        true
    }

    // Remove link ids for every link group touched by the current selection.
    pub fn unlink_selected_clips_groups(&mut self) -> bool {
        let mut selected = self.selected_clip_ids.clone();
        if selected.is_empty()
            && let Some(id) = self.selected_clip_id
        {
            selected.push(id);
        }
        if selected.is_empty() {
            return false;
        }

        let group_ids: HashSet<u64> = selected
            .into_iter()
            .filter_map(|clip_id| self.clip_link_group_id(clip_id))
            .collect();
        if group_ids.is_empty() {
            return false;
        }

        self.save_for_undo();
        let mut changed = false;

        for clip in &mut self.v1_clips {
            if clip
                .link_group_id
                .is_some_and(|link_group_id| group_ids.contains(&link_group_id))
            {
                clip.link_group_id = None;
                changed = true;
            }
        }
        for track in &mut self.audio_tracks {
            for clip in &mut track.clips {
                if clip
                    .link_group_id
                    .is_some_and(|link_group_id| group_ids.contains(&link_group_id))
                {
                    clip.link_group_id = None;
                    changed = true;
                }
            }
        }
        for track in &mut self.video_tracks {
            for clip in &mut track.clips {
                if clip
                    .link_group_id
                    .is_some_and(|link_group_id| group_ids.contains(&link_group_id))
                {
                    clip.link_group_id = None;
                    changed = true;
                }
            }
        }

        changed
    }

    pub fn move_subtitle_clip_free(
        &mut self,
        track_index: usize,
        clip_id: u64,
        new_start: Duration,
    ) {
        if let Some(track) = self.subtitle_tracks.get_mut(track_index) {
            if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                clip.start = new_start;
            }
            track.clips.sort_by_key(|c| c.start);
        }
    }
    // effect----------------------------------------------------
    // ✅ [NEW] Get the brightness of the currently selected clip
    pub fn get_selected_clip_brightness(&self) -> Option<f32> {
        let id = self.selected_clip_id?;

        // Search V1
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            let val = Self::clip_local_time(self.playhead, c)
                .map(|t| c.sample_brightness(t))
                .unwrap_or_else(|| c.get_brightness());
            return Some(val);
        }
        // Search video overlays
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                let val = Self::clip_local_time(self.playhead, c)
                    .map(|t| c.sample_brightness(t))
                    .unwrap_or_else(|| c.get_brightness());
                return Some(val);
            }
        }
        None
    }

    // ✅ [NEW] Set the brightness of the currently selected clip
    pub fn set_selected_clip_brightness(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };

        // 1. Check whether the value actually changed (to avoid meaningless undo entries)
        let current_val = self.get_selected_clip_brightness().unwrap_or(0.0);
        if (current_val - val).abs() < 0.001 {
            return;
        }

        // 2. ✅ Save state before modifying (to support undo)
        // Note: if this comes from slider dragging, it may generate many undo entries.
        // This line is required for the [+] and [-] buttons.
        self.save_for_undo();

        let new_val = val.clamp(-1.0, 1.0);

        // ===
        // Update V1
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c)
                && c.brightness_keyframe_index_at(local).is_some()
            {
                c.set_brightness_keyframe(local, new_val);
                return;
            }
            c.set_brightness(new_val); // ✅ Correct
            return;
        }

        // Update video overlays
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c)
                    && c.brightness_keyframe_index_at(local).is_some()
                {
                    c.set_brightness_keyframe(local, new_val);
                    return;
                }
                c.set_brightness(new_val); // ✅ Correct
                return;
            }
        }

        // ===
    }
    // ✅ [NEW] Contrast Wrapper
    pub fn get_selected_clip_contrast(&self) -> Option<f32> {
        let id = self.selected_clip_id?;
        // Simplified lookup logic: search V1 first, then video tracks
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            let val = Self::clip_local_time(self.playhead, c)
                .map(|t| c.sample_contrast(t))
                .unwrap_or_else(|| c.get_contrast());
            return Some(val);
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                let val = Self::clip_local_time(self.playhead, c)
                    .map(|t| c.sample_contrast(t))
                    .unwrap_or_else(|| c.get_contrast());
                return Some(val);
            }
        }
        None
    }

    pub fn set_selected_clip_contrast(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.clamp(0.0, 2.0); // Clamp to the 0-2 range

        self.save_for_undo(); // Remember undo

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c)
                && c.contrast_keyframe_index_at(local).is_some()
            {
                c.set_contrast_keyframe(local, new_val);
                return;
            }
            c.set_contrast(new_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c)
                    && c.contrast_keyframe_index_at(local).is_some()
                {
                    c.set_contrast_keyframe(local, new_val);
                    return;
                }
                c.set_contrast(new_val);
                return;
            }
        }
    }

    // ✅ [NEW] Saturation Wrapper
    pub fn get_selected_clip_saturation(&self) -> Option<f32> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            let val = Self::clip_local_time(self.playhead, c)
                .map(|t| c.sample_saturation(t))
                .unwrap_or_else(|| c.get_saturation());
            return Some(val);
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                let val = Self::clip_local_time(self.playhead, c)
                    .map(|t| c.sample_saturation(t))
                    .unwrap_or_else(|| c.get_saturation());
                return Some(val);
            }
        }
        None
    }

    pub fn set_selected_clip_saturation(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.clamp(0.0, 2.0); // Clamp to the 0-2 range

        self.save_for_undo();

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c)
                && c.saturation_keyframe_index_at(local).is_some()
            {
                c.set_saturation_keyframe(local, new_val);
                return;
            }
            c.set_saturation(new_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c)
                    && c.saturation_keyframe_index_at(local).is_some()
                {
                    c.set_saturation_keyframe(local, new_val);
                    return;
                }
                c.set_saturation(new_val);
                return;
            }
        }
    }

    pub fn get_selected_clip_opacity(&self) -> Option<f32> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            let val = Self::clip_local_time(self.playhead, c)
                .map(|t| c.sample_opacity(t))
                .unwrap_or_else(|| c.get_opacity());
            return Some(val);
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                let val = Self::clip_local_time(self.playhead, c)
                    .map(|t| c.sample_opacity(t))
                    .unwrap_or_else(|| c.get_opacity());
                return Some(val);
            }
        }
        None
    }

    pub fn get_selected_clip_blur_sigma(&self) -> Option<f32> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            let val = Self::clip_local_time(self.playhead, c)
                .map(|t| c.sample_blur(t))
                .unwrap_or_else(|| c.get_blur_sigma());
            return Some(val);
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                let val = Self::clip_local_time(self.playhead, c)
                    .map(|t| c.sample_blur(t))
                    .unwrap_or_else(|| c.get_blur_sigma());
                return Some(val);
            }
        }
        None
    }

    pub fn get_selected_clip_local_mask_layer_count(&self) -> Option<usize> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Some(c.local_mask_layer_count());
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Some(c.local_mask_layer_count());
            }
        }
        None
    }

    pub fn get_selected_clip_local_mask_layer(
        &self,
        layer_index: usize,
    ) -> Option<(bool, f32, f32, f32, f32, f32)> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Some(c.get_local_mask_layer(layer_index));
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Some(c.get_local_mask_layer(layer_index));
            }
        }
        None
    }

    pub fn get_selected_clip_local_mask_adjust_layer(
        &self,
        layer_index: usize,
    ) -> Option<(f32, f32, f32, f32, f32)> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Some(c.get_local_mask_adjust_layer(layer_index));
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Some(c.get_local_mask_adjust_layer(layer_index));
            }
        }
        None
    }

    pub fn get_selected_clip_slide(&self) -> Option<(SlideDirection, SlideDirection, f32, f32)> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Some(c.get_slide());
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Some(c.get_slide());
            }
        }
        None
    }

    pub fn set_selected_clip_slide_in(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.max(0.0);
        let (in_dir, out_dir, _in, out) = self.get_selected_clip_slide().unwrap_or((
            SlideDirection::Right,
            SlideDirection::Left,
            0.0,
            0.0,
        ));
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            c.set_slide(in_dir, out_dir, new_val, out);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                c.set_slide(in_dir, out_dir, new_val, out);
                return;
            }
        }
    }

    pub fn set_selected_clip_slide_out(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.max(0.0);
        let (in_dir, out_dir, in_val, _out) = self.get_selected_clip_slide().unwrap_or((
            SlideDirection::Right,
            SlideDirection::Left,
            0.0,
            0.0,
        ));
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            c.set_slide(in_dir, out_dir, in_val, new_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                c.set_slide(in_dir, out_dir, in_val, new_val);
                return;
            }
        }
    }

    pub fn set_selected_clip_slide_in_direction(&mut self, dir: SlideDirection) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let (_in_dir, out_dir, in_val, out_val) = self.get_selected_clip_slide().unwrap_or((
            SlideDirection::Right,
            SlideDirection::Left,
            0.0,
            0.0,
        ));
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            c.set_slide(dir, out_dir, in_val, out_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                c.set_slide(dir, out_dir, in_val, out_val);
                return;
            }
        }
    }

    pub fn set_selected_clip_slide_out_direction(&mut self, dir: SlideDirection) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let (in_dir, _out_dir, in_val, out_val) = self.get_selected_clip_slide().unwrap_or((
            SlideDirection::Right,
            SlideDirection::Left,
            0.0,
            0.0,
        ));
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            c.set_slide(in_dir, dir, in_val, out_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                c.set_slide(in_dir, dir, in_val, out_val);
                return;
            }
        }
    }

    pub fn get_selected_clip_zoom(&self) -> Option<(f32, f32, f32)> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Some(c.get_zoom());
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Some(c.get_zoom());
            }
        }
        None
    }

    pub fn get_selected_clip_shock_zoom(&self) -> Option<(f32, f32, f32)> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Some(c.get_shock_zoom());
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Some(c.get_shock_zoom());
            }
        }
        None
    }

    pub fn set_selected_clip_zoom_in(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.max(0.0);
        let (_in, out, amount) = self.get_selected_clip_zoom().unwrap_or((0.0, 0.0, 1.1));
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            c.set_zoom(new_val, out, amount);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                c.set_zoom(new_val, out, amount);
                return;
            }
        }
    }

    pub fn set_selected_clip_zoom_out(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.max(0.0);
        let (in_val, _out, amount) = self.get_selected_clip_zoom().unwrap_or((0.0, 0.0, 1.1));
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            c.set_zoom(in_val, new_val, amount);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                c.set_zoom(in_val, new_val, amount);
                return;
            }
        }
    }

    pub fn set_selected_clip_zoom_amount(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.clamp(0.1, 4.0);
        let (in_val, out_val, _amount) = self.get_selected_clip_zoom().unwrap_or((0.0, 0.0, 1.1));
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            c.set_zoom(in_val, out_val, new_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                c.set_zoom(in_val, out_val, new_val);
                return;
            }
        }
    }

    pub fn set_selected_clip_shock_zoom_in(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.max(0.0);
        let (_in, out, amount) = self
            .get_selected_clip_shock_zoom()
            .unwrap_or((0.0, 0.0, 1.2));
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            c.set_shock_zoom(new_val, out, amount);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                c.set_shock_zoom(new_val, out, amount);
                return;
            }
        }
    }

    pub fn set_selected_clip_shock_zoom_out(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.max(0.0);
        let (in_val, _out, amount) = self
            .get_selected_clip_shock_zoom()
            .unwrap_or((0.0, 0.0, 1.2));
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            c.set_shock_zoom(in_val, new_val, amount);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                c.set_shock_zoom(in_val, new_val, amount);
                return;
            }
        }
    }

    pub fn set_selected_clip_shock_zoom_amount(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.clamp(0.1, 4.0);
        let (in_val, out_val, _amount) = self
            .get_selected_clip_shock_zoom()
            .unwrap_or((0.0, 0.0, 1.2));
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            c.set_shock_zoom(in_val, out_val, new_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                c.set_shock_zoom(in_val, out_val, new_val);
                return;
            }
        }
    }

    pub fn set_selected_clip_opacity(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.clamp(0.0, 1.0);
        self.save_for_undo();

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c)
                && c.opacity_keyframe_index_at(local).is_some()
            {
                c.set_opacity_keyframe(local, new_val);
                return;
            }
            c.set_opacity(new_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c)
                    && c.opacity_keyframe_index_at(local).is_some()
                {
                    c.set_opacity_keyframe(local, new_val);
                    return;
                }
                c.set_opacity(new_val);
                return;
            }
        }
    }

    pub fn set_selected_clip_blur_sigma(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.clamp(0.0, 64.0);
        self.save_for_undo();

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c)
                && c.blur_keyframe_index_at(local).is_some()
            {
                c.set_blur_keyframe(local, new_val);
                return;
            }
            c.set_blur_sigma(new_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c)
                    && c.blur_keyframe_index_at(local).is_some()
                {
                    c.set_blur_keyframe(local, new_val);
                    return;
                }
                c.set_blur_sigma(new_val);
                return;
            }
        }
    }

    pub fn add_selected_clip_local_mask_layer(&mut self) -> Option<usize> {
        let id = self.selected_clip_id?;
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            return c.add_local_mask_layer();
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                return c.add_local_mask_layer();
            }
        }
        None
    }

    pub fn set_selected_clip_local_mask_enabled_at(&mut self, layer_index: usize, enabled: bool) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            let (_, cx, cy, radius, feather, strength) = c.get_local_mask_layer(layer_index);
            c.set_local_mask_layer(layer_index, enabled, cx, cy, radius, feather, strength);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                let (_, cx, cy, radius, feather, strength) = c.get_local_mask_layer(layer_index);
                c.set_local_mask_layer(layer_index, enabled, cx, cy, radius, feather, strength);
                return;
            }
        }
    }

    pub fn set_selected_clip_local_mask_center_x_at(&mut self, layer_index: usize, center_x: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            let (enabled, _, cy, radius, feather, strength) = c.get_local_mask_layer(layer_index);
            c.set_local_mask_layer(
                layer_index,
                enabled,
                center_x,
                cy,
                radius,
                feather,
                strength,
            );
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                let (enabled, _, cy, radius, feather, strength) =
                    c.get_local_mask_layer(layer_index);
                c.set_local_mask_layer(
                    layer_index,
                    enabled,
                    center_x,
                    cy,
                    radius,
                    feather,
                    strength,
                );
                return;
            }
        }
    }

    pub fn set_selected_clip_local_mask_center_y_at(&mut self, layer_index: usize, center_y: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            let (enabled, cx, _, radius, feather, strength) = c.get_local_mask_layer(layer_index);
            c.set_local_mask_layer(
                layer_index,
                enabled,
                cx,
                center_y,
                radius,
                feather,
                strength,
            );
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                let (enabled, cx, _, radius, feather, strength) =
                    c.get_local_mask_layer(layer_index);
                c.set_local_mask_layer(
                    layer_index,
                    enabled,
                    cx,
                    center_y,
                    radius,
                    feather,
                    strength,
                );
                return;
            }
        }
    }

    pub fn set_selected_clip_local_mask_radius_at(&mut self, layer_index: usize, radius: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            let (enabled, cx, cy, _, feather, strength) = c.get_local_mask_layer(layer_index);
            c.set_local_mask_layer(layer_index, enabled, cx, cy, radius, feather, strength);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                let (enabled, cx, cy, _, feather, strength) = c.get_local_mask_layer(layer_index);
                c.set_local_mask_layer(layer_index, enabled, cx, cy, radius, feather, strength);
                return;
            }
        }
    }

    pub fn set_selected_clip_local_mask_feather_at(&mut self, layer_index: usize, feather: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            let (enabled, cx, cy, radius, _, strength) = c.get_local_mask_layer(layer_index);
            c.set_local_mask_layer(layer_index, enabled, cx, cy, radius, feather, strength);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                let (enabled, cx, cy, radius, _, strength) = c.get_local_mask_layer(layer_index);
                c.set_local_mask_layer(layer_index, enabled, cx, cy, radius, feather, strength);
                return;
            }
        }
    }

    pub fn set_selected_clip_local_mask_strength_at(&mut self, layer_index: usize, strength: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            let (enabled, cx, cy, radius, feather, _) = c.get_local_mask_layer(layer_index);
            c.set_local_mask_layer(layer_index, enabled, cx, cy, radius, feather, strength);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                let (enabled, cx, cy, radius, feather, _) = c.get_local_mask_layer(layer_index);
                c.set_local_mask_layer(layer_index, enabled, cx, cy, radius, feather, strength);
                return;
            }
        }
    }

    pub fn set_selected_clip_local_mask_adjust_brightness_at(
        &mut self,
        layer_index: usize,
        brightness: f32,
    ) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            let (_, contrast, saturation, opacity, blur_sigma) =
                c.get_local_mask_adjust_layer(layer_index);
            c.set_local_mask_adjust_layer(
                layer_index,
                brightness,
                contrast,
                saturation,
                opacity,
                blur_sigma,
            );
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                let (_, contrast, saturation, opacity, blur_sigma) =
                    c.get_local_mask_adjust_layer(layer_index);
                c.set_local_mask_adjust_layer(
                    layer_index,
                    brightness,
                    contrast,
                    saturation,
                    opacity,
                    blur_sigma,
                );
                return;
            }
        }
    }

    pub fn set_selected_clip_local_mask_adjust_contrast_at(
        &mut self,
        layer_index: usize,
        contrast: f32,
    ) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            let (brightness, _, saturation, opacity, blur_sigma) =
                c.get_local_mask_adjust_layer(layer_index);
            c.set_local_mask_adjust_layer(
                layer_index,
                brightness,
                contrast,
                saturation,
                opacity,
                blur_sigma,
            );
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                let (brightness, _, saturation, opacity, blur_sigma) =
                    c.get_local_mask_adjust_layer(layer_index);
                c.set_local_mask_adjust_layer(
                    layer_index,
                    brightness,
                    contrast,
                    saturation,
                    opacity,
                    blur_sigma,
                );
                return;
            }
        }
    }

    pub fn set_selected_clip_local_mask_adjust_saturation_at(
        &mut self,
        layer_index: usize,
        saturation: f32,
    ) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            let (brightness, contrast, _, opacity, blur_sigma) =
                c.get_local_mask_adjust_layer(layer_index);
            c.set_local_mask_adjust_layer(
                layer_index,
                brightness,
                contrast,
                saturation,
                opacity,
                blur_sigma,
            );
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                let (brightness, contrast, _, opacity, blur_sigma) =
                    c.get_local_mask_adjust_layer(layer_index);
                c.set_local_mask_adjust_layer(
                    layer_index,
                    brightness,
                    contrast,
                    saturation,
                    opacity,
                    blur_sigma,
                );
                return;
            }
        }
    }

    pub fn set_selected_clip_local_mask_adjust_opacity_at(
        &mut self,
        layer_index: usize,
        opacity: f32,
    ) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            let (brightness, contrast, saturation, _, blur_sigma) =
                c.get_local_mask_adjust_layer(layer_index);
            c.set_local_mask_adjust_layer(
                layer_index,
                brightness,
                contrast,
                saturation,
                opacity,
                blur_sigma,
            );
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                let (brightness, contrast, saturation, _, blur_sigma) =
                    c.get_local_mask_adjust_layer(layer_index);
                c.set_local_mask_adjust_layer(
                    layer_index,
                    brightness,
                    contrast,
                    saturation,
                    opacity,
                    blur_sigma,
                );
                return;
            }
        }
    }

    pub fn set_selected_clip_local_mask_adjust_blur_sigma_at(
        &mut self,
        layer_index: usize,
        blur_sigma: f32,
    ) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            let (brightness, contrast, saturation, opacity, _) =
                c.get_local_mask_adjust_layer(layer_index);
            c.set_local_mask_adjust_layer(
                layer_index,
                brightness,
                contrast,
                saturation,
                opacity,
                blur_sigma,
            );
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                let (brightness, contrast, saturation, opacity, _) =
                    c.get_local_mask_adjust_layer(layer_index);
                c.set_local_mask_adjust_layer(
                    layer_index,
                    brightness,
                    contrast,
                    saturation,
                    opacity,
                    blur_sigma,
                );
                return;
            }
        }
    }

    fn selected_clip_ref(&self) -> Option<&Clip> {
        let id = self.selected_clip_id?;
        if let Some(clip) = self.v1_clips.iter().find(|clip| clip.id == id) {
            return Some(clip);
        }
        for track in &self.video_tracks {
            if let Some(clip) = track.clips.iter().find(|clip| clip.id == id) {
                return Some(clip);
            }
        }
        None
    }

    fn selected_clip_mut(&mut self) -> Option<&mut Clip> {
        let id = self.selected_clip_id?;
        if let Some(idx) = self.v1_clips.iter().position(|clip| clip.id == id) {
            return self.v1_clips.get_mut(idx);
        }
        for track in &mut self.video_tracks {
            if let Some(idx) = track.clips.iter().position(|clip| clip.id == id) {
                return track.clips.get_mut(idx);
            }
        }
        None
    }

    fn clip_channel_keyframes(clip: &Clip, channel: ClipKeyframeChannel) -> &[ScalarKeyframe] {
        match channel {
            ClipKeyframeChannel::Scale => &clip.scale_keyframes,
            ClipKeyframeChannel::Rotation => &clip.rotation_keyframes,
            ClipKeyframeChannel::PosX => &clip.pos_x_keyframes,
            ClipKeyframeChannel::PosY => &clip.pos_y_keyframes,
            ClipKeyframeChannel::Brightness => &clip.brightness_keyframes,
            ClipKeyframeChannel::Contrast => &clip.contrast_keyframes,
            ClipKeyframeChannel::Saturation => &clip.saturation_keyframes,
            ClipKeyframeChannel::Opacity => &clip.opacity_keyframes,
            ClipKeyframeChannel::Blur => &clip.blur_keyframes,
        }
    }

    fn clip_channel_keyframes_mut(
        clip: &mut Clip,
        channel: ClipKeyframeChannel,
    ) -> &mut Vec<ScalarKeyframe> {
        match channel {
            ClipKeyframeChannel::Scale => &mut clip.scale_keyframes,
            ClipKeyframeChannel::Rotation => &mut clip.rotation_keyframes,
            ClipKeyframeChannel::PosX => &mut clip.pos_x_keyframes,
            ClipKeyframeChannel::PosY => &mut clip.pos_y_keyframes,
            ClipKeyframeChannel::Brightness => &mut clip.brightness_keyframes,
            ClipKeyframeChannel::Contrast => &mut clip.contrast_keyframes,
            ClipKeyframeChannel::Saturation => &mut clip.saturation_keyframes,
            ClipKeyframeChannel::Opacity => &mut clip.opacity_keyframes,
            ClipKeyframeChannel::Blur => &mut clip.blur_keyframes,
        }
    }

    pub fn selected_clip_duration(&self) -> Option<Duration> {
        self.selected_clip_ref()
            .map(|clip| clip.duration.max(Duration::from_millis(1)))
    }

    pub fn selected_clip_local_playhead_time(&self) -> Option<Duration> {
        let clip = self.selected_clip_ref()?;
        Self::clip_local_time(self.playhead, clip)
    }

    pub fn selected_clip_set_playhead_to_local_time(&mut self, local_time: Duration) -> bool {
        let Some(clip) = self.selected_clip_ref() else {
            return false;
        };
        let local = local_time.min(clip.duration);
        self.set_playhead(clip.start + local);
        true
    }

    pub fn selected_clip_keyframe_times(&self, channel: ClipKeyframeChannel) -> Vec<Duration> {
        let Some(clip) = self.selected_clip_ref() else {
            return Vec::new();
        };
        Self::clip_channel_keyframes(clip, channel)
            .iter()
            .map(|k| k.time)
            .collect()
    }

    pub fn remove_selected_clip_keyframe_at_playhead(
        &mut self,
        channel: ClipKeyframeChannel,
    ) -> bool {
        let key_idx = {
            let Some(clip) = self.selected_clip_ref() else {
                return false;
            };
            let Some(local) = Self::clip_local_time(self.playhead, clip) else {
                return false;
            };
            keyframe::index_at(
                Self::clip_channel_keyframes(clip, channel),
                local,
                Duration::from_millis(33),
            )
        };

        let Some(key_idx) = key_idx else {
            return false;
        };

        self.save_for_undo();
        let Some(clip) = self.selected_clip_mut() else {
            return false;
        };
        let keys = Self::clip_channel_keyframes_mut(clip, channel);
        if key_idx >= keys.len() {
            return false;
        }
        keys.remove(key_idx);
        true
    }

    pub fn toggle_selected_clip_keyframe_at_playhead(
        &mut self,
        channel: ClipKeyframeChannel,
    ) -> bool {
        if self.remove_selected_clip_keyframe_at_playhead(channel) {
            return true;
        }
        if self.selected_clip_local_playhead_time().is_none() {
            return false;
        }

        match channel {
            ClipKeyframeChannel::Scale => self.add_selected_clip_scale_keyframe(),
            ClipKeyframeChannel::Rotation => self.add_selected_clip_rotation_keyframe(),
            ClipKeyframeChannel::PosX => self.add_selected_clip_pos_x_keyframe(),
            ClipKeyframeChannel::PosY => self.add_selected_clip_pos_y_keyframe(),
            ClipKeyframeChannel::Brightness => self.add_selected_clip_brightness_keyframe(),
            ClipKeyframeChannel::Contrast => self.add_selected_clip_contrast_keyframe(),
            ClipKeyframeChannel::Saturation => self.add_selected_clip_saturation_keyframe(),
            ClipKeyframeChannel::Opacity => self.add_selected_clip_opacity_keyframe(),
            ClipKeyframeChannel::Blur => self.add_selected_clip_blur_keyframe(),
        }
        true
    }

    pub fn add_selected_clip_opacity_keyframe(&mut self) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c) {
                let value = c.sample_opacity(local);
                c.set_opacity_keyframe(local, value);
            }
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c) {
                    let value = c.sample_opacity(local);
                    c.set_opacity_keyframe(local, value);
                }
                return;
            }
        }
    }

    pub fn selected_clip_has_opacity_keyframe(&self) -> bool {
        let Some(id) = self.selected_clip_id else {
            return false;
        };
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Self::clip_local_time(self.playhead, c)
                .and_then(|local| c.opacity_keyframe_index_at(local))
                .is_some();
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Self::clip_local_time(self.playhead, c)
                    .and_then(|local| c.opacity_keyframe_index_at(local))
                    .is_some();
            }
        }
        false
    }

    pub fn add_selected_clip_blur_keyframe(&mut self) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c) {
                let value = c.sample_blur(local);
                c.set_blur_keyframe(local, value);
            }
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c) {
                    let value = c.sample_blur(local);
                    c.set_blur_keyframe(local, value);
                }
                return;
            }
        }
    }

    pub fn selected_clip_has_blur_keyframe(&self) -> bool {
        let Some(id) = self.selected_clip_id else {
            return false;
        };
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Self::clip_local_time(self.playhead, c)
                .and_then(|local| c.blur_keyframe_index_at(local))
                .is_some();
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Self::clip_local_time(self.playhead, c)
                    .and_then(|local| c.blur_keyframe_index_at(local))
                    .is_some();
            }
        }
        false
    }

    pub fn get_selected_clip_fade_in(&self) -> Option<f32> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Some(c.get_fade_in());
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Some(c.get_fade_in());
            }
        }
        None
    }

    pub fn get_selected_clip_fade_out(&self) -> Option<f32> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Some(c.get_fade_out());
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Some(c.get_fade_out());
            }
        }
        None
    }

    pub fn get_selected_clip_dissolve_in(&self) -> Option<f32> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Some(c.get_dissolve_in());
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Some(c.get_dissolve_in());
            }
        }
        None
    }

    pub fn get_selected_clip_dissolve_out(&self) -> Option<f32> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Some(c.get_dissolve_out());
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Some(c.get_dissolve_out());
            }
        }
        None
    }

    pub fn set_selected_clip_fade_in(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.max(0.0);
        let current_val = self.get_selected_clip_fade_in().unwrap_or(0.0);
        if (current_val - new_val).abs() < 0.001 {
            return;
        }
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            c.set_fade_in(new_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                c.set_fade_in(new_val);
                return;
            }
        }
    }

    pub fn set_selected_clip_dissolve_in(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.max(0.0);
        let current_val = self.get_selected_clip_dissolve_in().unwrap_or(0.0);
        if (current_val - new_val).abs() < 0.001 {
            return;
        }
        if let Some(idx) = self.v1_clips.iter().position(|c| c.id == id) {
            if idx == 0 && new_val > 0.001 {
                self.ui_notice = Some("Dissolve requires an adjacent clip on V1.".to_string());
                return;
            }
            self.save_for_undo();
            let mut final_val = new_val;
            let mut notice: Option<String> = None;
            if idx >= 1 {
                let left_id = self.v1_clips[idx - 1].id;
                let right_id = self.v1_clips[idx].id;
                match self.trim_v1_pair_for_dissolve(left_id, right_id, new_val) {
                    Ok((dur, maybe_notice)) => {
                        final_val = dur;
                        notice = maybe_notice;
                    }
                    Err(msg) => {
                        self.ui_notice = Some(msg.to_string());
                        return;
                    }
                }
            }
            let left_id = if idx >= 1 {
                Some(self.v1_clips[idx - 1].id)
            } else {
                None
            };
            let right_id = self.v1_clips[idx].id;
            if let Some(right_idx) = self.v1_clips.iter().position(|c| c.id == right_id) {
                self.v1_clips[right_idx].set_dissolve_in(final_val);
                self.v1_clips[right_idx].set_fade_in(0.0);
                self.v1_clips[right_idx].set_fade_out(0.0);
            }
            if let Some(left_id) = left_id
                && let Some(left_idx) = self.v1_clips.iter().position(|c| c.id == left_id)
            {
                self.v1_clips[left_idx].set_dissolve_out(final_val);
                self.v1_clips[left_idx].set_fade_in(0.0);
                self.v1_clips[left_idx].set_fade_out(0.0);
            }
            self.pending_trim_to_fit = None;
            self.ui_notice = notice;
            return;
        }
        self.save_for_undo();
        for track in &mut self.video_tracks {
            if let Some(idx) = track.clips.iter().position(|c| c.id == id) {
                track.clips[idx].set_dissolve_in(new_val);
                track.clips[idx].set_fade_in(0.0);
                track.clips[idx].set_fade_out(0.0);
                if idx >= 1 {
                    track.clips[idx - 1].set_dissolve_out(new_val);
                    track.clips[idx - 1].set_fade_in(0.0);
                    track.clips[idx - 1].set_fade_out(0.0);
                }
                self.ui_notice = None;
                return;
            }
        }
    }

    pub fn set_selected_clip_fade_out(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.max(0.0);
        let current_val = self.get_selected_clip_fade_out().unwrap_or(0.0);
        if (current_val - new_val).abs() < 0.001 {
            return;
        }
        self.save_for_undo();
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            c.set_fade_out(new_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                c.set_fade_out(new_val);
                return;
            }
        }
    }

    pub fn set_selected_clip_dissolve_out(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        let new_val = val.max(0.0);
        let current_val = self.get_selected_clip_dissolve_out().unwrap_or(0.0);
        if (current_val - new_val).abs() < 0.001 {
            return;
        }
        if let Some(idx) = self.v1_clips.iter().position(|c| c.id == id) {
            if idx + 1 >= self.v1_clips.len() && new_val > 0.001 {
                self.ui_notice = Some("Dissolve requires an adjacent clip on V1.".to_string());
                return;
            }
            self.save_for_undo();
            let mut final_val = new_val;
            let mut notice: Option<String> = None;
            if idx + 1 < self.v1_clips.len() {
                let left_id = self.v1_clips[idx].id;
                let right_id = self.v1_clips[idx + 1].id;
                match self.trim_v1_pair_for_dissolve(left_id, right_id, new_val) {
                    Ok((dur, maybe_notice)) => {
                        final_val = dur;
                        notice = maybe_notice;
                    }
                    Err(msg) => {
                        self.ui_notice = Some(msg.to_string());
                        return;
                    }
                }
            }
            let left_id = self.v1_clips[idx].id;
            let right_id = if idx + 1 < self.v1_clips.len() {
                Some(self.v1_clips[idx + 1].id)
            } else {
                None
            };
            if let Some(left_idx) = self.v1_clips.iter().position(|c| c.id == left_id) {
                self.v1_clips[left_idx].set_dissolve_out(final_val);
                self.v1_clips[left_idx].set_fade_in(0.0);
                self.v1_clips[left_idx].set_fade_out(0.0);
            }
            if let Some(right_id) = right_id
                && let Some(right_idx) = self.v1_clips.iter().position(|c| c.id == right_id)
            {
                self.v1_clips[right_idx].set_dissolve_in(final_val);
                self.v1_clips[right_idx].set_fade_in(0.0);
                self.v1_clips[right_idx].set_fade_out(0.0);
            }
            self.pending_trim_to_fit = None;
            self.ui_notice = notice;
            return;
        }
        self.save_for_undo();
        for track in &mut self.video_tracks {
            if let Some(idx) = track.clips.iter().position(|c| c.id == id) {
                track.clips[idx].set_dissolve_out(new_val);
                track.clips[idx].set_fade_in(0.0);
                track.clips[idx].set_fade_out(0.0);
                if idx + 1 < track.clips.len() {
                    track.clips[idx + 1].set_dissolve_in(new_val);
                    track.clips[idx + 1].set_fade_in(0.0);
                    track.clips[idx + 1].set_fade_out(0.0);
                }
                self.ui_notice = None;
                return;
            }
        }
    }

    fn trim_v1_pair_for_dissolve(
        &mut self,
        left_id: u64,
        right_id: u64,
        requested: f32,
    ) -> Result<(f32, Option<String>), GlobalStateError> {
        let requested = requested.max(0.0);
        let Some(left_idx) = self.v1_clips.iter().position(|c| c.id == left_id) else {
            return Err(GlobalStateError::ClipNoLongerExists);
        };
        let Some(right_idx) = self.v1_clips.iter().position(|c| c.id == right_id) else {
            return Err(GlobalStateError::ClipNoLongerExists);
        };
        let (left_idx, right_idx) = if left_idx <= right_idx {
            (left_idx, right_idx)
        } else {
            (right_idx, left_idx)
        };
        if right_idx <= left_idx {
            return Err(GlobalStateError::DissolveRequiresAdjacentClips);
        }

        let half = requested * 0.5;
        let (left_post, right_pre) = {
            let left = &self.v1_clips[left_idx];
            let right = &self.v1_clips[right_idx];
            let left_post = left
                .media_duration
                .saturating_sub(left.source_in + left.duration)
                .as_secs_f32();
            let right_pre = right.source_in.as_secs_f32();
            (left_post, right_pre)
        };

        let min_dur = Duration::from_millis(100);
        let left_max_trim = self.v1_clips[left_idx]
            .duration
            .saturating_sub(min_dur)
            .as_secs_f32();
        let right_max_trim = self.v1_clips[right_idx]
            .duration
            .saturating_sub(min_dur)
            .as_secs_f32();
        let need_left = (half - left_post).max(0.0);
        let need_right = (half - right_pre).max(0.0);
        let extra_left = (left_post - half).max(0.0);
        let extra_right = (right_pre - half).max(0.0);
        let left_trim_out = self.v1_clips[left_idx].dissolve_trim_out.as_secs_f32();
        let right_trim_in = self.v1_clips[right_idx].dissolve_trim_in.as_secs_f32();
        let trim_left = need_left.min(left_max_trim);
        let trim_right = need_right.min(right_max_trim);
        let restore_left = extra_left.min(left_trim_out);
        let restore_right = extra_right.min(right_trim_in);

        if trim_left > 0.001 || trim_right > 0.001 || restore_left > 0.001 || restore_right > 0.001
        {
            if left_idx < right_idx {
                let (left_slice, right_slice) = self.v1_clips.split_at_mut(right_idx);
                let left = &mut left_slice[left_idx];
                let right = &mut right_slice[0];
                if restore_left > 0.001 {
                    let delta = Duration::from_secs_f32(restore_left);
                    left.duration += delta;
                    left.dissolve_trim_out = left.dissolve_trim_out.saturating_sub(delta);
                }
                if restore_right > 0.001 {
                    let max_restore = right.source_in.as_secs_f32();
                    let restore = restore_right.min(max_restore);
                    let delta = Duration::from_secs_f32(restore);
                    right.source_in = right.source_in.saturating_sub(delta);
                    right.duration += delta;
                    right.dissolve_trim_in = right.dissolve_trim_in.saturating_sub(delta);
                }
                if trim_left > 0.001 {
                    let delta = Duration::from_secs_f32(trim_left);
                    left.duration = left.duration.saturating_sub(delta);
                    left.dissolve_trim_out += delta;
                }
                if trim_right > 0.001 {
                    let delta = Duration::from_secs_f32(trim_right);
                    right.source_in += delta;
                    right.duration = right.duration.saturating_sub(delta);
                    right.dissolve_trim_in += delta;
                }
            } else {
                let (right_slice, left_slice) = self.v1_clips.split_at_mut(left_idx);
                let right = &mut right_slice[right_idx];
                let left = &mut left_slice[0];
                if restore_left > 0.001 {
                    let delta = Duration::from_secs_f32(restore_left);
                    left.duration += delta;
                    left.dissolve_trim_out = left.dissolve_trim_out.saturating_sub(delta);
                }
                if restore_right > 0.001 {
                    let max_restore = right.source_in.as_secs_f32();
                    let restore = restore_right.min(max_restore);
                    let delta = Duration::from_secs_f32(restore);
                    right.source_in = right.source_in.saturating_sub(delta);
                    right.duration += delta;
                    right.dissolve_trim_in = right.dissolve_trim_in.saturating_sub(delta);
                }
                if trim_left > 0.001 {
                    let delta = Duration::from_secs_f32(trim_left);
                    left.duration = left.duration.saturating_sub(delta);
                    left.dissolve_trim_out += delta;
                }
                if trim_right > 0.001 {
                    let delta = Duration::from_secs_f32(trim_right);
                    right.source_in += delta;
                    right.duration = right.duration.saturating_sub(delta);
                    right.dissolve_trim_in += delta;
                }
            }
            self.ripple_v1_starts();
        }

        let Some(left_idx) = self.v1_clips.iter().position(|c| c.id == left_id) else {
            return Err(GlobalStateError::ClipNoLongerExists);
        };
        let Some(right_idx) = self.v1_clips.iter().position(|c| c.id == right_id) else {
            return Err(GlobalStateError::ClipNoLongerExists);
        };
        let (left_idx, right_idx) = if left_idx <= right_idx {
            (left_idx, right_idx)
        } else {
            (right_idx, left_idx)
        };
        if right_idx <= left_idx {
            return Err(GlobalStateError::DissolveRequiresAdjacentClips);
        }
        let (left_post, right_pre) = {
            let left = &self.v1_clips[left_idx];
            let right = &self.v1_clips[right_idx];
            let left_post = left
                .media_duration
                .saturating_sub(left.source_in + left.duration)
                .as_secs_f32();
            let right_pre = right.source_in.as_secs_f32();
            (left_post, right_pre)
        };
        if requested <= 0.001 {
            return Ok((0.0, None));
        }

        let max_d = (left_post.min(right_pre)) * 2.0;
        let dur = requested.min(max_d);
        if dur <= 0.001 {
            return Err(GlobalStateError::InsufficientDissolveHandles);
        }
        if dur + 0.0001 < requested {
            return Ok((
                dur,
                Some(format!(
                    "Insufficient handles. Dissolve shortened to {:.2}s.",
                    dur
                )),
            ));
        }
        Ok((dur, None))
    }

    pub fn get_selected_clip_transform(&self) -> Option<(f32, f32, f32)> {
        let id = self.selected_clip_id?;

        // Find in V1
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            let (scale, pos_x, pos_y) = Self::clip_local_time(self.playhead, c)
                .map(|t| (c.sample_scale(t), c.sample_pos_x(t), c.sample_pos_y(t)))
                .unwrap_or_else(|| (c.get_scale(), c.get_pos_x(), c.get_pos_y()));
            return Some((scale, pos_x, pos_y));
        }
        // Find in Video Tracks
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                let (scale, pos_x, pos_y) = Self::clip_local_time(self.playhead, c)
                    .map(|t| (c.sample_scale(t), c.sample_pos_x(t), c.sample_pos_y(t)))
                    .unwrap_or_else(|| (c.get_scale(), c.get_pos_x(), c.get_pos_y()));
                return Some((scale, pos_x, pos_y));
            }
        }
        None
    }

    pub fn get_selected_clip_rotation(&self) -> Option<f32> {
        let id = self.selected_clip_id?;

        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            let rotation = Self::clip_local_time(self.playhead, c)
                .map(|t| c.sample_rotation(t))
                .unwrap_or_else(|| c.get_rotation());
            return Some(rotation);
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                let rotation = Self::clip_local_time(self.playhead, c)
                    .map(|t| c.sample_rotation(t))
                    .unwrap_or_else(|| c.get_rotation());
                return Some(rotation);
            }
        }
        None
    }

    pub fn set_selected_clip_scale(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        let new_val = val.clamp(0.01, 5.0); // 1% ~ 500%

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c)
                && c.scale_keyframe_index_at(local).is_some()
            {
                c.set_scale_keyframe(local, new_val);
                return;
            }
            c.set_scale(new_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c)
                    && c.scale_keyframe_index_at(local).is_some()
                {
                    c.set_scale_keyframe(local, new_val);
                    return;
                }
                c.set_scale(new_val);
                return;
            }
        }
    }

    pub fn set_selected_clip_rotation(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        let new_val = val.clamp(-180.0, 180.0);

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c)
                && c.rotation_keyframe_index_at(local).is_some()
            {
                c.set_rotation_keyframe(local, new_val);
                return;
            }
            c.set_rotation(new_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c)
                    && c.rotation_keyframe_index_at(local).is_some()
                {
                    c.set_rotation_keyframe(local, new_val);
                    return;
                }
                c.set_rotation(new_val);
                return;
            }
        }
    }

    pub fn set_selected_clip_pos_x(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        let new_val = val.clamp(-1.0, 1.0);

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c)
                && c.pos_x_keyframe_index_at(local).is_some()
            {
                c.set_pos_x_keyframe(local, new_val);
                return;
            }
            c.set_pos_x(new_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c)
                    && c.pos_x_keyframe_index_at(local).is_some()
                {
                    c.set_pos_x_keyframe(local, new_val);
                    return;
                }
                c.set_pos_x(new_val);
                return;
            }
        }
    }

    pub fn add_selected_clip_pos_x_keyframe(&mut self) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c) {
                let value = c.sample_pos_x(local);
                c.set_pos_x_keyframe(local, value);
            }
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c) {
                    let value = c.sample_pos_x(local);
                    c.set_pos_x_keyframe(local, value);
                }
                return;
            }
        }
    }

    pub fn selected_clip_has_pos_x_keyframe(&self) -> bool {
        let Some(id) = self.selected_clip_id else {
            return false;
        };
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Self::clip_local_time(self.playhead, c)
                .and_then(|local| c.pos_x_keyframe_index_at(local))
                .is_some();
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Self::clip_local_time(self.playhead, c)
                    .and_then(|local| c.pos_x_keyframe_index_at(local))
                    .is_some();
            }
        }
        false
    }

    pub fn add_selected_clip_pos_y_keyframe(&mut self) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c) {
                let value = c.sample_pos_y(local);
                c.set_pos_y_keyframe(local, value);
            }
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c) {
                    let value = c.sample_pos_y(local);
                    c.set_pos_y_keyframe(local, value);
                }
                return;
            }
        }
    }

    pub fn selected_clip_has_pos_y_keyframe(&self) -> bool {
        let Some(id) = self.selected_clip_id else {
            return false;
        };
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Self::clip_local_time(self.playhead, c)
                .and_then(|local| c.pos_y_keyframe_index_at(local))
                .is_some();
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Self::clip_local_time(self.playhead, c)
                    .and_then(|local| c.pos_y_keyframe_index_at(local))
                    .is_some();
            }
        }
        false
    }

    pub fn add_selected_clip_scale_keyframe(&mut self) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c) {
                let value = c.sample_scale(local);
                c.set_scale_keyframe(local, value);
            }
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c) {
                    let value = c.sample_scale(local);
                    c.set_scale_keyframe(local, value);
                }
                return;
            }
        }
    }

    pub fn selected_clip_has_scale_keyframe(&self) -> bool {
        let Some(id) = self.selected_clip_id else {
            return false;
        };
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Self::clip_local_time(self.playhead, c)
                .and_then(|local| c.scale_keyframe_index_at(local))
                .is_some();
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Self::clip_local_time(self.playhead, c)
                    .and_then(|local| c.scale_keyframe_index_at(local))
                    .is_some();
            }
        }
        false
    }

    pub fn add_selected_clip_rotation_keyframe(&mut self) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c) {
                let value = c.sample_rotation(local);
                c.set_rotation_keyframe(local, value);
            }
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c) {
                    let value = c.sample_rotation(local);
                    c.set_rotation_keyframe(local, value);
                }
                return;
            }
        }
    }

    pub fn selected_clip_has_rotation_keyframe(&self) -> bool {
        let Some(id) = self.selected_clip_id else {
            return false;
        };
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Self::clip_local_time(self.playhead, c)
                .and_then(|local| c.rotation_keyframe_index_at(local))
                .is_some();
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Self::clip_local_time(self.playhead, c)
                    .and_then(|local| c.rotation_keyframe_index_at(local))
                    .is_some();
            }
        }
        false
    }

    pub fn add_selected_clip_brightness_keyframe(&mut self) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c) {
                let value = c.sample_brightness(local);
                c.set_brightness_keyframe(local, value);
            }
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c) {
                    let value = c.sample_brightness(local);
                    c.set_brightness_keyframe(local, value);
                }
                return;
            }
        }
    }

    pub fn selected_clip_has_brightness_keyframe(&self) -> bool {
        let Some(id) = self.selected_clip_id else {
            return false;
        };
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Self::clip_local_time(self.playhead, c)
                .and_then(|local| c.brightness_keyframe_index_at(local))
                .is_some();
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Self::clip_local_time(self.playhead, c)
                    .and_then(|local| c.brightness_keyframe_index_at(local))
                    .is_some();
            }
        }
        false
    }

    pub fn add_selected_clip_contrast_keyframe(&mut self) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c) {
                let value = c.sample_contrast(local);
                c.set_contrast_keyframe(local, value);
            }
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c) {
                    let value = c.sample_contrast(local);
                    c.set_contrast_keyframe(local, value);
                }
                return;
            }
        }
    }

    pub fn selected_clip_has_contrast_keyframe(&self) -> bool {
        let Some(id) = self.selected_clip_id else {
            return false;
        };
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Self::clip_local_time(self.playhead, c)
                .and_then(|local| c.contrast_keyframe_index_at(local))
                .is_some();
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Self::clip_local_time(self.playhead, c)
                    .and_then(|local| c.contrast_keyframe_index_at(local))
                    .is_some();
            }
        }
        false
    }

    pub fn add_selected_clip_saturation_keyframe(&mut self) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c) {
                let value = c.sample_saturation(local);
                c.set_saturation_keyframe(local, value);
            }
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c) {
                    let value = c.sample_saturation(local);
                    c.set_saturation_keyframe(local, value);
                }
                return;
            }
        }
    }

    pub fn selected_clip_has_saturation_keyframe(&self) -> bool {
        let Some(id) = self.selected_clip_id else {
            return false;
        };
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Self::clip_local_time(self.playhead, c)
                .and_then(|local| c.saturation_keyframe_index_at(local))
                .is_some();
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Self::clip_local_time(self.playhead, c)
                    .and_then(|local| c.saturation_keyframe_index_at(local))
                    .is_some();
            }
        }
        false
    }

    fn clip_local_time(playhead: Duration, clip: &Clip) -> Option<Duration> {
        if playhead < clip.start || playhead > clip.end() {
            return None;
        }
        Some((playhead - clip.start).min(clip.duration))
    }

    fn split_keyframes(
        keys: &[ScalarKeyframe],
        split_time: Duration,
    ) -> (Vec<ScalarKeyframe>, Vec<ScalarKeyframe>) {
        let mut left = Vec::new();
        let mut right = Vec::new();
        for key in keys {
            if key.time <= split_time {
                left.push(key.clone());
            }
            if key.time >= split_time {
                let mut shifted = key.clone();
                shifted.time = if key.time > split_time {
                    key.time - split_time
                } else {
                    Duration::ZERO
                };
                right.push(shifted);
            }
        }
        (left, right)
    }

    pub fn set_selected_clip_pos_y(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        let new_val = val.clamp(-1.0, 1.0);

        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            if let Some(local) = Self::clip_local_time(self.playhead, c)
                && c.pos_y_keyframe_index_at(local).is_some()
            {
                c.set_pos_y_keyframe(local, new_val);
                return;
            }
            c.set_pos_y(new_val);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                if let Some(local) = Self::clip_local_time(self.playhead, c)
                    && c.pos_y_keyframe_index_at(local).is_some()
                {
                    c.set_pos_y_keyframe(local, new_val);
                    return;
                }
                c.set_pos_y(new_val);
                return;
            }
        }
    }

    // ====
    // Get the selected clip HSLA overlay (canonical)
    pub fn get_selected_clip_hsla_overlay(&self) -> Option<(f32, f32, f32, f32)> {
        let id = self.selected_clip_id?;
        if let Some(c) = self.v1_clips.iter().find(|c| c.id == id) {
            return Some(c.get_hsla_overlay());
        }
        for track in &self.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == id) {
                return Some(c.get_hsla_overlay());
            }
        }
        None
    }

    // Helper methods for setting HSLA overlay properties

    // Set hue
    pub fn set_selected_clip_hsla_overlay_hue(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        let (_, s, l, a) = self
            .get_selected_clip_hsla_overlay()
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        let h = val.clamp(0.0, 360.0); // 0-360
        self.apply_hsla_overlay_change(id, h, s, l, a);
    }

    // Set alpha (opacity)
    pub fn set_selected_clip_hsla_overlay_alpha(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        let (h, s, l, _) = self
            .get_selected_clip_hsla_overlay()
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        let a = val.clamp(0.0, 1.0); // 0-1
        self.apply_hsla_overlay_change(id, h, s, l, a);
    }

    // Set saturation
    pub fn set_selected_clip_hsla_overlay_saturation(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        let (h, _, l, a) = self
            .get_selected_clip_hsla_overlay()
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        let s = val.clamp(0.0, 1.0);
        self.apply_hsla_overlay_change(id, h, s, l, a);
    }

    // Set lightness
    pub fn set_selected_clip_hsla_overlay_lightness(&mut self, val: f32) {
        let Some(id) = self.selected_clip_id else {
            return;
        };
        self.save_for_undo();
        let (h, s, _, a) = self
            .get_selected_clip_hsla_overlay()
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        let l = val.clamp(0.0, 1.0);
        self.apply_hsla_overlay_change(id, h, s, l, a);
    }

    // Internal helper: apply the actual write
    fn apply_hsla_overlay_change(&mut self, id: u64, h: f32, s: f32, l: f32, a: f32) {
        if let Some(c) = self.v1_clips.iter_mut().find(|c| c.id == id) {
            c.set_hsla_overlay(h, s, l, a);
            return;
        }
        for track in &mut self.video_tracks {
            if let Some(c) = track.clips.iter_mut().find(|c| c.id == id) {
                c.set_hsla_overlay(h, s, l, a);
                return;
            }
        }
    }

    // ====

    // -------------------------
    // Edit Operations
    // -------------------------

    pub fn add_new_audio_track(&mut self) {
        self.save_for_undo();
        let idx = self.next_audio_track_number();
        let name = format!("A{}", idx);
        self.audio_tracks.push(AudioTrack::new(name));
        println!(
            "[GlobalState] Added Audio Track. Total: {}",
            self.audio_tracks.len()
        );
    }

    pub fn get_audio_track_gain_db(&self, track_index: usize) -> f32 {
        self.audio_tracks
            .get(track_index)
            .map(|track| track.gain_db)
            .unwrap_or(0.0)
    }

    pub fn get_selected_audio_clip_gain_db(&self) -> Option<f32> {
        let id = self.selected_clip_id?;
        self.audio_tracks
            .iter()
            .find_map(|track| track.clips.iter().find(|clip| clip.id == id))
            .map(|clip| clip.audio_gain_db)
    }

    pub fn set_selected_audio_clip_gain_db(&mut self, gain_db: f32) -> bool {
        let Some(id) = self.selected_clip_id else {
            return false;
        };
        let clamped = gain_db.clamp(-60.0, 12.0);
        let current = self
            .audio_tracks
            .iter()
            .find_map(|track| track.clips.iter().find(|clip| clip.id == id))
            .map(|clip| clip.audio_gain_db);
        let Some(current) = current else {
            return false;
        };
        if (current - clamped).abs() < 0.001 {
            return false;
        }

        self.save_for_undo();
        for track in &mut self.audio_tracks {
            if let Some(clip) = track.clips.iter_mut().find(|clip| clip.id == id) {
                clip.audio_gain_db = clamped;
                return true;
            }
        }
        false
    }

    pub fn set_audio_track_gain_db(&mut self, track_index: usize, gain_db: f32) -> bool {
        let Some(track) = self.audio_tracks.get(track_index) else {
            return false;
        };
        let clamped = gain_db.clamp(-60.0, 12.0);
        if (track.gain_db - clamped).abs() < 0.001 {
            return false;
        }
        self.save_for_undo();
        if let Some(track_mut) = self.audio_tracks.get_mut(track_index) {
            track_mut.gain_db = clamped;
            return true;
        }
        false
    }

    pub fn add_new_video_track(&mut self) {
        self.save_for_undo();
        let idx = self.video_tracks.len() + 2; // V1 is the base track, so start numbering from V2
        let name = format!("V{}", idx);
        // Insert new tracks at the front so rendering stacks from bottom to top (depending on your render order)
        // Usually we append at the end, and let the UI decide the render order
        self.video_tracks.push(VideoTrack::new(name));
        println!(
            "[GlobalState] Added Video Track. Total: {}",
            self.video_tracks.len()
        );
    }

    pub fn add_new_subtitle_track(&mut self) {
        self.save_for_undo();
        let idx = self.subtitle_tracks.len() + 1;
        let name = format!("S{}", idx);
        self.subtitle_tracks.push(SubtitleTrack::new(name));
        println!(
            "[GlobalState] Added Subtitle Track. Total: {}",
            self.subtitle_tracks.len()
        );
    }

    pub fn add_subtitle_clip(
        &mut self,
        track_index: usize,
        start: Duration,
        duration: Duration,
        text: String,
    ) -> Option<u64> {
        self.subtitle_tracks.get(track_index)?;
        self.save_for_undo();
        let id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);

        let clip = SubtitleClip {
            id,
            text,
            start,
            duration,
            pos_x: -0.30,
            pos_y: 0.35,
            font_size: 48.0,
            color_rgba: (255, 255, 255, 255),
            font_family: None,
            font_path: None,
            group_id: None,
        };

        if let Some(track) = self.subtitle_tracks.get_mut(track_index) {
            track.clips.push(clip);
            track.clips.sort_by_key(|c| c.start);
        }
        self.selected_subtitle_id = Some(id);
        self.selected_subtitle_ids = vec![id];
        self.selected_clip_id = None;
        self.selected_clip_ids.clear();
        Some(id)
    }

    pub fn import_srt(&mut self, srt_text: &str) -> Result<usize, GlobalStateError> {
        let mut cues =
            parse_srt(srt_text).map_err(|source| GlobalStateError::SrtParse { source })?;
        cues.sort_by_key(|cue| cue.start);

        self.save_for_undo();

        let mut imported = 0usize;
        let mut last_id = None;
        let mut touched_tracks: Vec<usize> = Vec::new();

        for cue in cues {
            if cue.end <= cue.start {
                continue;
            }
            let duration = match cue.end.checked_sub(cue.start) {
                Some(dur) => dur,
                None => continue,
            };
            if duration.is_zero() {
                continue;
            }
            let id = self.next_clip_id;
            self.next_clip_id = self.next_clip_id.saturating_add(1);

            let target_track_index = self.subtitle_tracks.iter().position(|track| {
                !track.clips.iter().any(|clip| {
                    let clip_end = clip.start.saturating_add(clip.duration);
                    cue.start < clip_end && clip.start < cue.end
                })
            });

            let target_track_index = match target_track_index {
                Some(idx) => idx,
                None => {
                    let idx = self.subtitle_tracks.len();
                    self.subtitle_tracks
                        .push(SubtitleTrack::new(format!("S{}", idx + 1)));
                    idx
                }
            };

            self.subtitle_tracks[target_track_index]
                .clips
                .push(SubtitleClip {
                    id,
                    text: cue.text,
                    start: cue.start,
                    duration,
                    pos_x: -0.30,
                    pos_y: 0.35,
                    font_size: 48.0,
                    color_rgba: (255, 255, 255, 255),
                    font_family: None,
                    font_path: None,
                    group_id: None,
                });
            if !touched_tracks.contains(&target_track_index) {
                touched_tracks.push(target_track_index);
            }
            imported += 1;
            last_id = Some(id);
        }

        for track_index in touched_tracks {
            if let Some(track) = self.subtitle_tracks.get_mut(track_index) {
                track.clips.sort_by_key(|c| c.start);
            }
        }
        if let Some(id) = last_id {
            self.selected_subtitle_id = Some(id);
            self.selected_subtitle_ids = vec![id];
            self.selected_clip_id = None;
            self.selected_clip_ids.clear();
        }
        Ok(imported)
    }

    pub fn get_selected_subtitle_text(&self) -> Option<String> {
        let id = self.selected_subtitle_id?;
        for track in &self.subtitle_tracks {
            if let Some(clip) = track.clips.iter().find(|c| c.id == id) {
                return Some(clip.text.clone());
            }
        }
        None
    }

    pub fn set_selected_subtitle_text(&mut self, text: String) {
        let Some(id) = self.selected_subtitle_id else {
            return;
        };
        for track in &mut self.subtitle_tracks {
            if let Some(clip) = track.clips.iter_mut().find(|c| c.id == id) {
                clip.text = text;
                return;
            }
        }
    }

    pub fn get_selected_subtitle_transform(&self) -> Option<(f32, f32, f32)> {
        let id = self.selected_subtitle_id?;
        for track in &self.subtitle_tracks {
            if let Some(clip) = track.clips.iter().find(|c| c.id == id) {
                return Some((clip.pos_x, clip.pos_y, clip.font_size));
            }
        }
        None
    }

    pub fn get_selected_subtitle_color_rgba(&self) -> Option<(u8, u8, u8, u8)> {
        let id = self.selected_subtitle_id?;
        for track in &self.subtitle_tracks {
            if let Some(clip) = track.clips.iter().find(|c| c.id == id) {
                return Some(clip.color_rgba);
            }
        }
        None
    }

    pub fn get_selected_subtitle_group_id(&self) -> Option<u64> {
        let id = self.selected_subtitle_id?;
        for track in &self.subtitle_tracks {
            if let Some(clip) = track.clips.iter().find(|c| c.id == id) {
                return clip.group_id;
            }
        }
        None
    }

    pub fn get_selected_subtitle_group_color_rgba(&self) -> Option<(u8, u8, u8, u8)> {
        let group_id = self.get_selected_subtitle_group_id()?;
        for track in &self.subtitle_tracks {
            if let Some(clip) = track.clips.iter().find(|c| c.group_id == Some(group_id)) {
                return Some(clip.color_rgba);
            }
        }
        None
    }

    pub fn get_selected_subtitle_group_transform(&self) -> Option<(f32, f32, f32)> {
        let group_id = self.get_selected_subtitle_group_id()?;
        let group = self.subtitle_groups.get(&group_id)?;
        Some((group.offset_x, group.offset_y, group.scale))
    }

    pub fn set_selected_subtitle_group_pos_x(&mut self, val: f32) {
        let Some(group_id) = self.get_selected_subtitle_group_id() else {
            return;
        };
        self.save_for_undo();
        if let Some(group) = self.subtitle_groups.get_mut(&group_id) {
            group.offset_x = val.clamp(-1.0, 1.0);
        }
    }

    pub fn set_selected_subtitle_group_pos_y(&mut self, val: f32) {
        let Some(group_id) = self.get_selected_subtitle_group_id() else {
            return;
        };
        self.save_for_undo();
        if let Some(group) = self.subtitle_groups.get_mut(&group_id) {
            group.offset_y = val.clamp(-1.0, 1.0);
        }
    }

    pub fn set_selected_subtitle_group_scale(&mut self, val: f32) {
        let Some(group_id) = self.get_selected_subtitle_group_id() else {
            return;
        };
        self.save_for_undo();
        if let Some(group) = self.subtitle_groups.get_mut(&group_id) {
            group.scale = val.clamp(0.25, 4.0);
        }
    }

    pub fn set_selected_subtitle_group_color_rgba(&mut self, color: (u8, u8, u8, u8)) {
        let Some(group_id) = self.get_selected_subtitle_group_id() else {
            return;
        };
        self.save_for_undo();
        for track in &mut self.subtitle_tracks {
            for clip in &mut track.clips {
                if clip.group_id == Some(group_id) {
                    clip.color_rgba = color;
                }
            }
        }
    }

    pub fn set_selected_subtitle_group_font(
        &mut self,
        path: Option<String>,
        family: Option<String>,
    ) {
        let Some(group_id) = self.get_selected_subtitle_group_id() else {
            return;
        };
        self.save_for_undo();
        for track in &mut self.subtitle_tracks {
            for clip in &mut track.clips {
                if clip.group_id == Some(group_id) {
                    clip.font_path = path.clone();
                    clip.font_family = family.clone();
                }
            }
        }
    }

    pub fn effective_subtitle_transform(&self, clip: &SubtitleClip) -> (f32, f32, f32) {
        if let Some(group_id) = clip.group_id
            && let Some(group) = self.subtitle_groups.get(&group_id)
        {
            return (
                clip.pos_x + group.offset_x,
                clip.pos_y + group.offset_y,
                (clip.font_size * group.scale).max(1.0),
            );
        }
        (clip.pos_x, clip.pos_y, clip.font_size)
    }

    pub fn group_selected_subtitles(&mut self) -> Option<u64> {
        if self.selected_subtitle_ids.len() < 2 {
            return None;
        }
        self.save_for_undo();
        let group_id = self.next_subtitle_group_id;
        self.next_subtitle_group_id = self.next_subtitle_group_id.saturating_add(1);
        self.subtitle_groups.insert(
            group_id,
            SubtitleGroupTransform {
                offset_x: 0.0,
                offset_y: 0.0,
                scale: 1.0,
            },
        );
        let ids: HashSet<u64> = self.selected_subtitle_ids.iter().copied().collect();
        for track in &mut self.subtitle_tracks {
            for clip in &mut track.clips {
                if ids.contains(&clip.id) {
                    clip.group_id = Some(group_id);
                }
            }
        }
        self.selected_subtitle_id = self.selected_subtitle_ids.last().copied();
        self.selected_clip_id = None;
        self.selected_clip_ids.clear();
        Some(group_id)
    }

    pub fn select_subtitle_group(&mut self, group_id: u64) {
        let mut ids = Vec::new();
        for track in &self.subtitle_tracks {
            for clip in &track.clips {
                if clip.group_id == Some(group_id) {
                    ids.push(clip.id);
                }
            }
        }
        if ids.is_empty() {
            return;
        }
        self.selected_subtitle_ids = ids.clone();
        self.selected_subtitle_id = ids.last().copied();
        self.selected_clip_id = None;
        self.selected_clip_ids.clear();
    }

    pub fn set_selected_subtitle_pos_x(&mut self, val: f32) {
        let Some(id) = self.selected_subtitle_id else {
            return;
        };
        self.save_for_undo();
        let new_val = val.clamp(-1.0, 1.0);
        for track in &mut self.subtitle_tracks {
            if let Some(clip) = track.clips.iter_mut().find(|c| c.id == id) {
                clip.pos_x = new_val;
                return;
            }
        }
    }

    pub fn set_selected_subtitle_pos_y(&mut self, val: f32) {
        let Some(id) = self.selected_subtitle_id else {
            return;
        };
        self.save_for_undo();
        let new_val = val.clamp(-1.0, 1.0);
        for track in &mut self.subtitle_tracks {
            if let Some(clip) = track.clips.iter_mut().find(|c| c.id == id) {
                clip.pos_y = new_val;
                return;
            }
        }
    }

    pub fn set_selected_subtitle_font_size(&mut self, val: f32) {
        let Some(id) = self.selected_subtitle_id else {
            return;
        };
        self.save_for_undo();
        let new_val = val.clamp(8.0, 256.0);
        for track in &mut self.subtitle_tracks {
            if let Some(clip) = track.clips.iter_mut().find(|c| c.id == id) {
                clip.font_size = new_val;
                return;
            }
        }
    }

    pub fn set_selected_subtitle_color_rgba(&mut self, color: (u8, u8, u8, u8)) {
        let Some(id) = self.selected_subtitle_id else {
            return;
        };
        self.save_for_undo();
        for track in &mut self.subtitle_tracks {
            if let Some(clip) = track.clips.iter_mut().find(|c| c.id == id) {
                clip.color_rgba = color;
                return;
            }
        }
    }

    pub fn get_selected_subtitle_font(&self) -> Option<(String, String)> {
        let id = self.selected_subtitle_id?;
        for track in &self.subtitle_tracks {
            if let Some(clip) = track.clips.iter().find(|c| c.id == id) {
                let path = clip.font_path.clone()?;
                let family = clip.font_family.clone().unwrap_or_default();
                return Some((path, family));
            }
        }
        None
    }

    pub fn set_selected_subtitle_font(&mut self, path: Option<String>, family: Option<String>) {
        let Some(id) = self.selected_subtitle_id else {
            return;
        };
        self.save_for_undo();
        for track in &mut self.subtitle_tracks {
            if let Some(clip) = track.clips.iter_mut().find(|c| c.id == id) {
                clip.font_path = path.clone();
                clip.font_family = family.clone();
                return;
            }
        }
    }

    // --- V1 Operations ---
    // Insert into V1 by current mode so toolbar mode also controls import/drop behavior.
    pub fn insert_active_source_v1(&mut self, insert_duration: Duration) {
        // Capture undo before any timeline mutation triggered by media insertion.
        self.save_for_undo();
        if matches!(self.v1_move_mode, V1MoveMode::Magnetic) {
            // Magnetic mode always appends to the tail, so middle insertion cannot split V1 clips.
            self.append_active_source_v1(insert_duration);
        } else {
            self.free_insert_active_source_v1(insert_duration);
        }
    }

    // Append on V1 tail and keep A1 linked, used by magnetic insert mode.
    fn append_active_source_v1(&mut self, insert_duration: Duration) {
        if self.active_source_path.is_empty() {
            return;
        }

        self.ensure_primary_audio_track();
        let tail_start = self
            .v1_clips
            .iter()
            .map(|clip| clip.start + clip.duration)
            .max()
            .unwrap_or(Duration::ZERO);
        let link_group_id = Some(self.allocate_next_link_group_id());

        let new_id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);
        self.v1_clips.push(Clip {
            id: new_id,
            label: self.active_source_name.clone(),
            file_path: self.active_source_path.clone(),
            start: tail_start,
            duration: insert_duration,
            source_in: Duration::ZERO,
            media_duration: insert_duration,
            link_group_id,
            audio_gain_db: 0.0,
            dissolve_trim_in: Duration::ZERO,
            dissolve_trim_out: Duration::ZERO,
            video_effects: VideoEffect::standard_set(),
            local_mask_layers: vec![LocalMaskLayer::default()],
            pos_x_keyframes: Vec::new(),
            pos_y_keyframes: Vec::new(),
            scale_keyframes: Vec::new(),
            brightness_keyframes: Vec::new(),
            contrast_keyframes: Vec::new(),
            saturation_keyframes: Vec::new(),
            opacity_keyframes: Vec::new(),
            blur_keyframes: Vec::new(),
            rotation_keyframes: Vec::new(),
        });
        self.v1_clips.sort_by_key(|clip| clip.start);

        let audio_id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);
        self.audio_tracks[0].clips.push(Clip {
            id: audio_id,
            label: format!("(Audio) {}", self.active_source_name),
            file_path: self.active_source_path.clone(),
            start: tail_start,
            duration: insert_duration,
            source_in: Duration::ZERO,
            media_duration: insert_duration,
            link_group_id,
            audio_gain_db: 0.0,
            dissolve_trim_in: Duration::ZERO,
            dissolve_trim_out: Duration::ZERO,
            video_effects: VideoEffect::standard_set(),
            local_mask_layers: vec![LocalMaskLayer::default()],
            pos_x_keyframes: Vec::new(),
            pos_y_keyframes: Vec::new(),
            scale_keyframes: Vec::new(),
            brightness_keyframes: Vec::new(),
            contrast_keyframes: Vec::new(),
            saturation_keyframes: Vec::new(),
            opacity_keyframes: Vec::new(),
            blur_keyframes: Vec::new(),
            rotation_keyframes: Vec::new(),
        });
        self.audio_tracks[0].clips.sort_by_key(|clip| clip.start);

        self.playhead = tail_start;
        self.selected_clip_id = Some(new_id);
        self.selected_clip_ids = vec![new_id];
        self.selected_subtitle_id = None;
        self.selected_subtitle_ids.clear();
    }

    // Free V1 insert keeps timeline untouched and allows overlap while still creating linked A1 audio.
    fn free_insert_active_source_v1(&mut self, insert_duration: Duration) {
        if self.active_source_path.is_empty() {
            return;
        }

        self.ensure_primary_audio_track();
        let t = self.playhead;
        let link_group_id = Some(self.allocate_next_link_group_id());

        let new_id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);

        self.v1_clips.push(Clip {
            id: new_id,
            label: self.active_source_name.clone(),
            file_path: self.active_source_path.clone(),
            start: t,
            duration: insert_duration,
            source_in: Duration::ZERO,
            media_duration: insert_duration,
            link_group_id,
            audio_gain_db: 0.0,
            dissolve_trim_in: Duration::ZERO,
            dissolve_trim_out: Duration::ZERO,
            video_effects: VideoEffect::standard_set(),
            local_mask_layers: vec![LocalMaskLayer::default()],
            pos_x_keyframes: Vec::new(),
            pos_y_keyframes: Vec::new(),
            scale_keyframes: Vec::new(),
            brightness_keyframes: Vec::new(),
            contrast_keyframes: Vec::new(),
            saturation_keyframes: Vec::new(),
            opacity_keyframes: Vec::new(),
            blur_keyframes: Vec::new(),
            rotation_keyframes: Vec::new(),
        });
        self.v1_clips.sort_by_key(|clip| clip.start);

        let audio_id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);
        self.audio_tracks[0].clips.push(Clip {
            id: audio_id,
            label: format!("(Audio) {}", self.active_source_name),
            file_path: self.active_source_path.clone(),
            start: t,
            duration: insert_duration,
            source_in: Duration::ZERO,
            media_duration: insert_duration,
            link_group_id,
            audio_gain_db: 0.0,
            dissolve_trim_in: Duration::ZERO,
            dissolve_trim_out: Duration::ZERO,
            video_effects: VideoEffect::standard_set(),
            local_mask_layers: vec![LocalMaskLayer::default()],
            pos_x_keyframes: Vec::new(),
            pos_y_keyframes: Vec::new(),
            scale_keyframes: Vec::new(),
            brightness_keyframes: Vec::new(),
            contrast_keyframes: Vec::new(),
            saturation_keyframes: Vec::new(),
            opacity_keyframes: Vec::new(),
            blur_keyframes: Vec::new(),
            rotation_keyframes: Vec::new(),
        });
        self.audio_tracks[0].clips.sort_by_key(|clip| clip.start);

        self.selected_clip_id = Some(new_id);
        self.selected_clip_ids = vec![new_id];
        self.selected_subtitle_id = None;
        self.selected_subtitle_ids.clear();
    }

    // --- Audio Operations ---
    pub fn ripple_insert_active_source_audio(
        &mut self,
        track_index: usize,
        insert_duration: Duration,
    ) {
        if self.active_source_path.is_empty() {
            return;
        }

        // Capture undo before inserting media pool content into audio lanes.
        self.save_for_undo();

        if let Some(track) = self.audio_tracks.get_mut(track_index) {
            let t = self.playhead;
            let path = self.active_source_path.clone();
            let name = format!("(Audio) {}", self.active_source_name);

            // [Changed] Use free insert instead, without pushing other clips aside (overlap is allowed)
            let new_id = self.next_clip_id;
            self.next_clip_id += 1;

            track.clips.push(Clip {
                id: new_id,
                label: name,
                file_path: path,
                start: t,
                duration: insert_duration,
                source_in: Duration::ZERO,
                media_duration: insert_duration,
                link_group_id: None,
                audio_gain_db: 0.0,
                dissolve_trim_in: Duration::ZERO,
                dissolve_trim_out: Duration::ZERO,
                video_effects: VideoEffect::standard_set(),
                local_mask_layers: vec![LocalMaskLayer::default()],
                pos_x_keyframes: Vec::new(),
                pos_y_keyframes: Vec::new(),
                scale_keyframes: Vec::new(),
                brightness_keyframes: Vec::new(),
                contrast_keyframes: Vec::new(),
                saturation_keyframes: Vec::new(),
                opacity_keyframes: Vec::new(),
                blur_keyframes: Vec::new(),
                rotation_keyframes: Vec::new(),
            });
            track.clips.sort_by_key(|c| c.start);

            self.selected_clip_id = Some(new_id);
            self.selected_clip_ids = vec![new_id];
            self.selected_subtitle_id = None;
            self.selected_subtitle_ids.clear();
        }
    }
    // Video v2 to more operation (not v1)
    pub fn ripple_insert_active_source_video(
        &mut self,
        track_index: usize,
        insert_duration: Duration,
    ) {
        if self.active_source_path.is_empty() {
            return;
        }
        // Capture undo before inserting media pool content into overlay video lanes.
        self.save_for_undo();
        if let Some(track) = self.video_tracks.get_mut(track_index) {
            let t = self.playhead;
            let path = self.active_source_path.clone();
            let name = format!("(Video) {}", self.active_source_name);
            let new_id = self.next_clip_id;
            self.next_clip_id += 1;

            track.clips.push(Clip {
                id: new_id,
                label: name,
                file_path: path,
                start: t,
                duration: insert_duration,
                source_in: Duration::ZERO,
                media_duration: insert_duration,
                link_group_id: None,
                audio_gain_db: 0.0,
                dissolve_trim_in: Duration::ZERO,
                dissolve_trim_out: Duration::ZERO,
                video_effects: VideoEffect::standard_set(),
                local_mask_layers: vec![LocalMaskLayer::default()],
                pos_x_keyframes: Vec::new(),
                pos_y_keyframes: Vec::new(),
                scale_keyframes: Vec::new(),
                brightness_keyframes: Vec::new(),
                contrast_keyframes: Vec::new(),
                saturation_keyframes: Vec::new(),
                opacity_keyframes: Vec::new(),
                blur_keyframes: Vec::new(),
                rotation_keyframes: Vec::new(),
            });
            track.clips.sort_by_key(|c| c.start);

            self.selected_clip_id = Some(new_id);
            self.selected_clip_ids = vec![new_id];
            self.selected_subtitle_id = None;
            self.selected_subtitle_ids.clear();
        }
    }

    // --- Razor Operations ---
    pub fn razor_v1_at_playhead(&mut self) -> Result<(), GlobalStateError> {
        self.repair_missing_primary_av_links();

        // Save state BEFORE deleting!
        self.save_for_undo();

        let t = self.playhead;
        // Split V1 and linked companions together so deleting one side after razor keeps A/V consistent.
        let Some(target_clip) = self
            .v1_clips
            .iter()
            .find(|clip| t > clip.start && t < clip.end())
            .map(|clip| (clip.id, clip.link_group_id))
        else {
            return Err(GlobalStateError::NoClipToCut { track: "V1" });
        };

        let (target_v1_id, left_group_id) = target_clip;
        let right_group_id = left_group_id.map(|_| self.allocate_next_link_group_id());

        let Some(new_id) = Self::perform_razor_for_clip_id_with_groups(
            &mut self.v1_clips,
            &mut self.next_clip_id,
            target_v1_id,
            t,
            left_group_id,
            right_group_id,
        ) else {
            return Err(GlobalStateError::NoClipToCut { track: "V1" });
        };

        if let Some(link_group_id) = left_group_id {
            for track in &mut self.audio_tracks {
                if let Some(linked_clip_id) = track
                    .clips
                    .iter()
                    .find(|clip| {
                        clip.link_group_id == Some(link_group_id)
                            && t > clip.start
                            && t < clip.end()
                    })
                    .map(|clip| clip.id)
                {
                    let _ = Self::perform_razor_for_clip_id_with_groups(
                        &mut track.clips,
                        &mut self.next_clip_id,
                        linked_clip_id,
                        t,
                        left_group_id,
                        right_group_id,
                    );
                }
            }

            for track in &mut self.video_tracks {
                if let Some(linked_clip_id) = track
                    .clips
                    .iter()
                    .find(|clip| {
                        clip.link_group_id == Some(link_group_id)
                            && t > clip.start
                            && t < clip.end()
                    })
                    .map(|clip| clip.id)
                {
                    let _ = Self::perform_razor_for_clip_id_with_groups(
                        &mut track.clips,
                        &mut self.next_clip_id,
                        linked_clip_id,
                        t,
                        left_group_id,
                        right_group_id,
                    );
                }
            }
        }

        self.selected_clip_id = Some(new_id);
        self.selected_clip_ids = vec![new_id];
        self.selected_subtitle_id = None;
        self.selected_subtitle_ids.clear();
        Ok(())
    }

    pub fn razor_audio_at_playhead(&mut self, track_index: usize) -> Result<(), GlobalStateError> {
        // Save state BEFORE deleting!
        self.save_for_undo();

        let t = self.playhead;
        if let Some(track) = self.audio_tracks.get_mut(track_index)
            && let Some(new_id) = Self::perform_razor(&mut track.clips, &mut self.next_clip_id, t)
        {
            self.selected_clip_id = Some(new_id);
            self.selected_clip_ids = vec![new_id];
            self.selected_subtitle_id = None;
            self.selected_subtitle_ids.clear();
            return Ok(());
        }
        Err(GlobalStateError::NoClipToCut {
            track: "Audio Track",
        })
    }

    pub fn razor_video_at_playhead(&mut self, track_index: usize) -> Result<(), GlobalStateError> {
        // Save state BEFORE deleting!
        self.save_for_undo();

        let t = self.playhead;
        if let Some(track) = self.video_tracks.get_mut(track_index)
            && let Some(new_id) = Self::perform_razor(&mut track.clips, &mut self.next_clip_id, t)
        {
            self.selected_clip_id = Some(new_id);
            self.selected_clip_ids = vec![new_id];
            self.selected_subtitle_id = None;
            self.selected_subtitle_ids.clear();
            return Ok(());
        }
        Err(GlobalStateError::NoClipToCut {
            track: "Video Track",
        })
    }

    pub fn razor_subtitle_at_playhead(
        &mut self,
        track_index: usize,
    ) -> Result<(), GlobalStateError> {
        self.save_for_undo();

        let t = self.playhead;
        if let Some(track) = self.subtitle_tracks.get_mut(track_index)
            && let Some(new_id) =
                Self::perform_subtitle_razor(&mut track.clips, &mut self.next_clip_id, t)
        {
            self.selected_subtitle_id = Some(new_id);
            self.selected_subtitle_ids = vec![new_id];
            self.selected_clip_id = None;
            self.selected_clip_ids.clear();
            return Ok(());
        }
        Err(GlobalStateError::NoSubtitleToCut)
    }

    fn perform_razor(track: &mut Vec<Clip>, next_id_counter: &mut u64, t: Duration) -> Option<u64> {
        let target_clip_id = track
            .iter()
            .find(|clip| t > clip.start && t < clip.end())
            .map(|clip| clip.id)?;
        let group = track
            .iter()
            .find(|clip| clip.id == target_clip_id)
            .and_then(|clip| clip.link_group_id);
        Self::perform_razor_for_clip_id_with_groups(
            track,
            next_id_counter,
            target_clip_id,
            t,
            group,
            group,
        )
    }

    // Split one specific clip id at playhead and assign explicit left/right link groups.
    fn perform_razor_for_clip_id_with_groups(
        track: &mut Vec<Clip>,
        next_id_counter: &mut u64,
        clip_id: u64,
        t: Duration,
        left_group_id: Option<u64>,
        right_group_id: Option<u64>,
    ) -> Option<u64> {
        let target_index = track
            .iter()
            .position(|clip| clip.id == clip_id && t > clip.start && t < clip.end())?;

        let clip = &track[target_index];
        let left_dur = t - clip.start;
        let right_dur = clip.duration - left_dur;

        let (left_pos_x, right_pos_x) = Self::split_keyframes(&clip.pos_x_keyframes, left_dur);
        let (left_pos_y, right_pos_y) = Self::split_keyframes(&clip.pos_y_keyframes, left_dur);
        let (left_scale, right_scale) = Self::split_keyframes(&clip.scale_keyframes, left_dur);
        let (left_brightness, right_brightness) =
            Self::split_keyframes(&clip.brightness_keyframes, left_dur);
        let (left_contrast, right_contrast) =
            Self::split_keyframes(&clip.contrast_keyframes, left_dur);
        let (left_saturation, right_saturation) =
            Self::split_keyframes(&clip.saturation_keyframes, left_dur);
        let (left_opacity, right_opacity) =
            Self::split_keyframes(&clip.opacity_keyframes, left_dur);
        let (left_blur, right_blur) = Self::split_keyframes(&clip.blur_keyframes, left_dur);
        let (left_rotation, right_rotation) =
            Self::split_keyframes(&clip.rotation_keyframes, left_dur);

        if left_dur.as_secs_f32() < 0.001 || right_dur.as_secs_f32() < 0.001 {
            return None;
        }

        let left_id = *next_id_counter;
        *next_id_counter += 1;

        let right_id = *next_id_counter;
        *next_id_counter += 1;

        let left = Clip {
            id: left_id,
            label: clip.label.clone(),
            file_path: clip.file_path.clone(),
            start: clip.start,
            duration: left_dur,
            source_in: clip.source_in,
            media_duration: clip.media_duration,
            link_group_id: left_group_id,
            audio_gain_db: clip.audio_gain_db,
            dissolve_trim_in: clip.dissolve_trim_in,
            dissolve_trim_out: Duration::ZERO,
            video_effects: clip.video_effects.clone(),
            local_mask_layers: clip.local_mask_layers.clone(),
            pos_x_keyframes: left_pos_x,
            pos_y_keyframes: left_pos_y,
            scale_keyframes: left_scale,
            brightness_keyframes: left_brightness,
            contrast_keyframes: left_contrast,
            saturation_keyframes: left_saturation,
            opacity_keyframes: left_opacity,
            blur_keyframes: left_blur,
            rotation_keyframes: left_rotation,
        };

        let right = Clip {
            id: right_id,
            label: clip.label.clone(),
            file_path: clip.file_path.clone(),
            start: t,
            duration: right_dur,
            source_in: clip.source_in + left_dur,
            media_duration: clip.media_duration,
            link_group_id: right_group_id,
            audio_gain_db: clip.audio_gain_db,
            dissolve_trim_in: Duration::ZERO,
            dissolve_trim_out: clip.dissolve_trim_out,
            video_effects: clip.video_effects.clone(),
            local_mask_layers: clip.local_mask_layers.clone(),
            pos_x_keyframes: right_pos_x,
            pos_y_keyframes: right_pos_y,
            scale_keyframes: right_scale,
            brightness_keyframes: right_brightness,
            contrast_keyframes: right_contrast,
            saturation_keyframes: right_saturation,
            opacity_keyframes: right_opacity,
            blur_keyframes: right_blur,
            rotation_keyframes: right_rotation,
        };

        track.splice(target_index..=target_index, [left, right]);
        track.sort_by_key(|clip| clip.start);
        Some(right_id)
    }

    fn perform_subtitle_razor(
        track: &mut Vec<SubtitleClip>,
        next_id_counter: &mut u64,
        t: Duration,
    ) -> Option<u64> {
        let mut target_index = None;

        for (i, clip) in track.iter().enumerate() {
            if t > clip.start && t < clip.end() {
                target_index = Some(i);
                break;
            }
        }

        if let Some(i) = target_index {
            let clip = &track[i];
            let left_dur = t - clip.start;
            let right_dur = clip.duration - left_dur;

            if left_dur.as_secs_f32() < 0.001 || right_dur.as_secs_f32() < 0.001 {
                return None;
            }

            let left_id = *next_id_counter;
            *next_id_counter += 1;
            let right_id = *next_id_counter;
            *next_id_counter += 1;

            let left = SubtitleClip {
                id: left_id,
                text: clip.text.clone(),
                start: clip.start,
                duration: left_dur,
                pos_x: clip.pos_x,
                pos_y: clip.pos_y,
                font_size: clip.font_size,
                color_rgba: clip.color_rgba,
                font_family: clip.font_family.clone(),
                font_path: clip.font_path.clone(),
                group_id: clip.group_id,
            };

            let right = SubtitleClip {
                id: right_id,
                text: clip.text.clone(),
                start: t,
                duration: right_dur,
                pos_x: clip.pos_x,
                pos_y: clip.pos_y,
                font_size: clip.font_size,
                color_rgba: clip.color_rgba,
                font_family: clip.font_family.clone(),
                font_path: clip.font_path.clone(),
                group_id: clip.group_id,
            };

            track.splice(i..=i, [left, right]);
            track.sort_by_key(|c| c.start);
            return Some(right_id);
        }
        None
    }

    fn razor_all_clips_at(track: &mut Vec<Clip>, next_id_counter: &mut u64, t: Duration) {
        // Keep splitting until no clip still straddles `t` in this track.
        while Self::perform_razor(track, next_id_counter, t).is_some() {}
    }

    fn razor_all_subtitles_at(
        track: &mut Vec<SubtitleClip>,
        next_id_counter: &mut u64,
        t: Duration,
    ) {
        // Keep splitting until no subtitle still straddles `t` in this track.
        while Self::perform_subtitle_razor(track, next_id_counter, t).is_some() {}
    }

    /// Ripple-delete a timeline range across V1, audio, video overlays, and subtitles.
    ///
    /// This is the foundational edit op used by ACP plan-apply for "remove silence" workflows.
    pub fn ripple_delete_time_range_all_tracks(&mut self, start: Duration, end: Duration) -> bool {
        if end <= start {
            return false;
        }

        let delta = end - start;
        self.save_for_undo();

        // First split every track at both boundaries so range deletion becomes idempotent
        // remove+shift operations without partial-clip math.
        Self::razor_all_clips_at(&mut self.v1_clips, &mut self.next_clip_id, start);
        Self::razor_all_clips_at(&mut self.v1_clips, &mut self.next_clip_id, end);
        for track in &mut self.audio_tracks {
            Self::razor_all_clips_at(&mut track.clips, &mut self.next_clip_id, start);
            Self::razor_all_clips_at(&mut track.clips, &mut self.next_clip_id, end);
        }
        for track in &mut self.video_tracks {
            Self::razor_all_clips_at(&mut track.clips, &mut self.next_clip_id, start);
            Self::razor_all_clips_at(&mut track.clips, &mut self.next_clip_id, end);
        }
        for track in &mut self.subtitle_tracks {
            Self::razor_all_subtitles_at(&mut track.clips, &mut self.next_clip_id, start);
            Self::razor_all_subtitles_at(&mut track.clips, &mut self.next_clip_id, end);
        }

        let mut changed = false;

        let mut process_clip_track = |track: &mut Vec<Clip>| {
            let before_len = track.len();
            track.retain(|clip| !(clip.start >= start && clip.end() <= end));
            if track.len() != before_len {
                changed = true;
            }
            for clip in track.iter_mut() {
                if clip.start >= end {
                    clip.start -= delta;
                    changed = true;
                }
            }
            track.sort_by_key(|clip| clip.start);
        };

        process_clip_track(&mut self.v1_clips);
        for track in &mut self.audio_tracks {
            process_clip_track(&mut track.clips);
        }
        for track in &mut self.video_tracks {
            process_clip_track(&mut track.clips);
        }

        for track in &mut self.subtitle_tracks {
            let before_len = track.clips.len();
            track.clips.retain(|clip| {
                let clip_end = clip.end();
                !(clip.start >= start && clip_end <= end)
            });
            if track.clips.len() != before_len {
                changed = true;
            }
            for clip in &mut track.clips {
                if clip.start >= end {
                    clip.start -= delta;
                    changed = true;
                }
            }
            track.clips.sort_by_key(|clip| clip.start);
        }

        if !changed {
            return false;
        }

        // Clean stale selections after remove/split operations.
        let existing_clip_ids: HashSet<u64> = self
            .v1_clips
            .iter()
            .chain(self.audio_tracks.iter().flat_map(|t| t.clips.iter()))
            .chain(self.video_tracks.iter().flat_map(|t| t.clips.iter()))
            .map(|c| c.id)
            .collect();
        self.selected_clip_ids
            .retain(|id| existing_clip_ids.contains(id));
        if self
            .selected_clip_id
            .is_some_and(|id| !existing_clip_ids.contains(&id))
        {
            self.selected_clip_id = self.selected_clip_ids.last().copied();
        }

        let existing_subtitle_ids: HashSet<u64> = self
            .subtitle_tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.id)
            .collect();
        self.selected_subtitle_ids
            .retain(|id| existing_subtitle_ids.contains(id));
        if self
            .selected_subtitle_id
            .is_some_and(|id| !existing_subtitle_ids.contains(&id))
        {
            self.selected_subtitle_id = self.selected_subtitle_ids.last().copied();
        }

        // Keep playhead visually stable after ripple delete.
        if self.playhead >= end {
            self.playhead -= delta;
        } else if self.playhead > start {
            self.playhead = start;
        }

        true
    }

    pub fn delete_selected_items(&mut self) -> bool {
        self.repair_missing_primary_av_links();

        let mut clip_ids = self.selected_clip_ids.clone();
        if clip_ids.is_empty()
            && let Some(id) = self.selected_clip_id
        {
            clip_ids.push(id);
        }

        let mut subtitle_ids = self.selected_subtitle_ids.clone();
        if subtitle_ids.is_empty()
            && let Some(id) = self.selected_subtitle_id
        {
            subtitle_ids.push(id);
        }

        let semantic_id = self.selected_semantic_clip_id();
        if clip_ids.is_empty() && subtitle_ids.is_empty() && semantic_id.is_none() {
            return false;
        }

        self.save_for_undo();

        if !clip_ids.is_empty() {
            let old_v1_group_starts = self.v1_group_anchor_starts();
            // Expand linked A/V ids so deleting one linked segment removes its companion segment too.
            let mut clip_set: std::collections::HashSet<u64> = clip_ids.iter().copied().collect();
            clip_set = self.expand_clip_ids_by_link_group(&clip_set);

            let before_v1 = self.v1_clips.len();
            self.v1_clips.retain(|c| !clip_set.contains(&c.id));
            let mut v1_ripple_changed = false;
            if self.v1_clips.len() != before_v1 {
                self.v1_clips.sort_by_key(|c| c.start);
                let mut cursor = Duration::ZERO;
                for c in &mut self.v1_clips {
                    c.start = cursor;
                    cursor += c.duration;
                }
                v1_ripple_changed = true;
            }

            for track in &mut self.audio_tracks {
                track.clips.retain(|c| !clip_set.contains(&c.id));
            }
            for track in &mut self.video_tracks {
                track.clips.retain(|c| !clip_set.contains(&c.id));
            }

            if v1_ripple_changed {
                self.sync_linked_tracks_from_v1_deltas(&old_v1_group_starts);
            }
        }

        if !subtitle_ids.is_empty() {
            let subtitle_set: std::collections::HashSet<u64> =
                subtitle_ids.iter().copied().collect();
            for track in &mut self.subtitle_tracks {
                track.clips.retain(|c| !subtitle_set.contains(&c.id));
            }
        }

        if let Some(id) = semantic_id {
            self.semantic_clips.retain(|clip| clip.id != id);
        }

        self.selected_clip_id = None;
        self.selected_subtitle_id = None;
        self.selected_layer_effect_clip_id = None;
        self.selected_semantic_clip_id = None;
        self.selected_clip_ids.clear();
        self.selected_subtitle_ids.clear();
        true
    }

    pub fn delete_audio_track(&mut self, track_index: usize) -> bool {
        if track_index >= self.audio_tracks.len() {
            return false;
        }
        self.save_for_undo();
        let removed = self.audio_tracks.remove(track_index);
        if !removed.clips.is_empty() {
            let removed_ids: std::collections::HashSet<u64> =
                removed.clips.iter().map(|c| c.id).collect();
            self.selected_clip_ids
                .retain(|id| !removed_ids.contains(id));
            if let Some(sel) = self.selected_clip_id
                && removed_ids.contains(&sel)
            {
                self.selected_clip_id = None;
            }
        }
        if self.selected_clip_id.is_none() {
            self.selected_clip_id = self.selected_clip_ids.last().copied();
        }
        true
    }

    pub fn delete_video_track(&mut self, track_index: usize) -> bool {
        if track_index >= self.video_tracks.len() {
            return false;
        }
        self.save_for_undo();
        let removed = self.video_tracks.remove(track_index);
        if self.video_tracks.is_empty() {
            self.layer_effect_clips.clear();
            self.selected_layer_effect_clip_id = None;
        } else {
            let replacement_track = self.video_tracks.len().saturating_sub(1);
            for layer in &mut self.layer_effect_clips {
                if layer.track_index == track_index {
                    layer.track_index = replacement_track;
                } else if layer.track_index > track_index {
                    layer.track_index = layer.track_index.saturating_sub(1);
                }
            }
        }
        if !removed.clips.is_empty() {
            let removed_ids: std::collections::HashSet<u64> =
                removed.clips.iter().map(|c| c.id).collect();
            self.selected_clip_ids
                .retain(|id| !removed_ids.contains(id));
            if let Some(sel) = self.selected_clip_id
                && removed_ids.contains(&sel)
            {
                self.selected_clip_id = None;
            }
        }
        if self.selected_clip_id.is_none() {
            self.selected_clip_id = self.selected_clip_ids.last().copied();
        }
        true
    }

    pub fn delete_subtitle_track(&mut self, track_index: usize) -> bool {
        if track_index >= self.subtitle_tracks.len() {
            return false;
        }
        self.save_for_undo();
        let removed = self.subtitle_tracks.remove(track_index);
        if !removed.clips.is_empty() {
            let removed_ids: std::collections::HashSet<u64> =
                removed.clips.iter().map(|c| c.id).collect();
            self.selected_subtitle_ids
                .retain(|id| !removed_ids.contains(id));
            if let Some(sel) = self.selected_subtitle_id
                && removed_ids.contains(&sel)
            {
                self.selected_subtitle_id = None;
            }
        }
        if self.selected_subtitle_id.is_none() {
            self.selected_subtitle_id = self.selected_subtitle_ids.last().copied();
        }
        true
    }

    pub fn resize_clip(&mut self, track_type: TrackType, clip_id: u64, new_duration: Duration) {
        // Find the clip based on the track type
        let clip_opt = match track_type {
            TrackType::V1 => self.v1_clips.iter_mut().find(|c| c.id == clip_id),
            TrackType::Audio(idx) => self
                .audio_tracks
                .get_mut(idx)
                .and_then(|t| t.clips.iter_mut().find(|c| c.id == clip_id)),
            TrackType::VideoOverlay(idx) => self
                .video_tracks
                .get_mut(idx)
                .and_then(|t| t.clips.iter_mut().find(|c| c.id == clip_id)), // Use VideoOverlay
            TrackType::Subtitle(_) => None,
        };

        if let Some(clip) = clip_opt {
            // Determine whether it is an image
            let is_img = clip.file_path.to_lowercase().ends_with(".jpg")
                || clip.file_path.to_lowercase().ends_with(".png")
                || clip.file_path.to_lowercase().ends_with(".webp")
                || clip.file_path.to_lowercase().ends_with(".bmp");

            let max_dur = if is_img {
                Duration::from_secs(3600 * 24)
            } else {
                // Duration::from_secs(3600 * 24)
                clip.media_duration.saturating_sub(clip.source_in)
            };

            // Clamp to the minimum length (for example, 0.1s) and the maximum length
            clip.duration = new_duration.clamp(Duration::from_millis(100), max_dur);
        }
    }

    pub fn resize_subtitle_clip(
        &mut self,
        track_index: usize,
        clip_id: u64,
        new_duration: Duration,
    ) {
        if let Some(track) = self.subtitle_tracks.get_mut(track_index)
            && let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id)
        {
            clip.duration = new_duration.max(Duration::from_millis(100));
        }
    }

    // --Export-------------------------
    pub fn export_begin(&mut self, out_path: String, total_duration: Duration) {
        self.export_in_progress = true;
        self.export_last_error = None;
        self.export_last_out_path = Some(out_path);
        self.export_progress_ratio = 0.0;
        self.export_progress_rendered = Duration::ZERO;
        self.export_progress_total = total_duration.max(Duration::from_millis(1));
        self.export_eta = None;
    }

    pub fn export_update_progress(
        &mut self,
        rendered: Duration,
        total: Duration,
        speed: Option<f32>,
    ) {
        let total = total.max(Duration::from_millis(1));
        let rendered = rendered.min(total);
        self.export_progress_total = total;
        self.export_progress_rendered = rendered;
        let ratio = rendered.as_secs_f64() / total.as_secs_f64();
        self.export_progress_ratio = ratio.clamp(0.0, 1.0) as f32;
        self.export_eta = speed.and_then(|s| {
            if s <= 0.001 {
                return None;
            }
            let remain = (total.as_secs_f64() - rendered.as_secs_f64()).max(0.0);
            Some(Duration::from_secs_f64(remain / s as f64))
        });
    }

    pub fn export_done(&mut self) {
        self.export_in_progress = false;
        self.export_progress_ratio = 1.0;
        self.export_progress_rendered = self.export_progress_total;
        self.export_eta = Some(Duration::ZERO);
    }

    pub fn export_fail(&mut self, err: String) {
        self.export_in_progress = false;
        // Surface the first error line in the timeline status area for quick diagnosis.
        let first_line = err
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("Export failed.");
        self.ui_notice = Some(format!("Export failed: {}", first_line));
        self.export_last_error = Some(err);
        self.export_eta = None;
    }

    pub fn export_cancelled(&mut self) {
        self.export_in_progress = false;
        self.export_last_error = None;
        self.export_last_out_path = None;
        self.export_eta = None;
    }

    // -------------------------
    // Internals
    // -------------------------

    // ============================================
    // [NEW] 1. Capture current state (Snapshot)
    fn capture_state(&self) -> TimelineState {
        self.capture_state_inner(false)
    }

    fn capture_state_with_media_pool(&self) -> TimelineState {
        self.capture_state_inner(true)
    }

    fn capture_state_inner(&self, include_media_pool: bool) -> TimelineState {
        TimelineState {
            v1_clips: self.v1_clips.clone(),
            audio_tracks: self.audio_tracks.clone(),
            video_tracks: self.video_tracks.clone(),
            subtitle_tracks: self.subtitle_tracks.clone(),
            semantic_clips: self.semantic_clips.clone(),
            track_visibility: self.track_visibility.clone(),
            track_lock: self.track_lock.clone(),
            track_mute: self.track_mute.clone(),
            subtitle_groups: self.subtitle_groups.clone(),
            next_subtitle_group_id: self.next_subtitle_group_id,
            playhead: self.playhead,
            semantic_mark_start: self.semantic_mark_start,
            layer_color_blur_effects: self.layer_color_blur_effects,
            layer_effect_clips: self.layer_effect_clips.clone(),
            selected_layer_effect_clip_id: self.selected_layer_effect_clip_id,
            selected_semantic_clip_id: self.selected_semantic_clip_id,
            media_pool_state: if include_media_pool {
                Some(MediaPoolUndoState {
                    active_source_path: self.active_source_path.clone(),
                    active_source_name: self.active_source_name.clone(),
                    active_source_duration: self.active_source_duration,
                    media_pool: self.media_pool.clone(),
                    pending_media_pool_path: self.pending_media_pool_path.clone(),
                })
            } else {
                None
            },
        }
    }

    // [NEW] 2. Restore state from Snapshot
    fn restore_state(&mut self, state: TimelineState) {
        self.v1_clips = state.v1_clips;
        self.audio_tracks = state.audio_tracks;
        self.video_tracks = state.video_tracks;
        self.subtitle_tracks = state.subtitle_tracks;
        self.semantic_clips = state.semantic_clips;
        self.track_visibility = state.track_visibility;
        self.track_lock = state.track_lock;
        self.track_mute = state.track_mute;
        self.subtitle_groups = state.subtitle_groups;
        self.next_subtitle_group_id = state.next_subtitle_group_id;
        self.playhead = state.playhead;
        self.semantic_mark_start = state.semantic_mark_start;
        self.layer_color_blur_effects = state.layer_color_blur_effects;
        self.layer_effect_clips = state.layer_effect_clips;
        self.selected_layer_effect_clip_id = state.selected_layer_effect_clip_id;
        self.selected_semantic_clip_id = state.selected_semantic_clip_id;
        if let Some(media_pool_state) = state.media_pool_state {
            self.active_source_path = media_pool_state.active_source_path;
            self.active_source_name = media_pool_state.active_source_name;
            self.active_source_duration = media_pool_state.active_source_duration;
            self.media_pool = media_pool_state.media_pool;
            self.pending_media_pool_path = media_pool_state.pending_media_pool_path;
            self.media_pool_drag = None;
            self.media_pool_context_menu = None;
        }
        self.selected_clip_id = None;
        self.selected_subtitle_id = None;
        self.selected_clip_ids.clear();
        self.selected_subtitle_ids.clear();
    }

    // [NEW] 3. Call this BEFORE modifying data (Public API)
    pub fn save_for_undo(&mut self) {
        let state = self.capture_state();
        self.undo_manager.push(state);
        self.bump_timeline_edit_token();
    }

    pub fn save_for_undo_with_media_pool(&mut self) {
        let state = self.capture_state_with_media_pool();
        self.undo_manager.push(state);
        self.bump_timeline_edit_token();
    }

    // [NEW] 4. Execute Undo
    pub fn undo(&mut self) {
        let current = self.capture_state_with_media_pool();
        if let Some(prev) = self.undo_manager.undo(current) {
            self.restore_state(prev);
            self.bump_timeline_edit_token();
        }
    }

    // [NEW] 5. Execute Redo
    pub fn redo(&mut self) {
        let current = self.capture_state_with_media_pool();
        if let Some(next) = self.undo_manager.redo(current) {
            self.restore_state(next);
            self.bump_timeline_edit_token();
        }
    }
}

fn round_up_to_step(value: Duration, step: Duration) -> Duration {
    let v = value.as_secs();
    let s = step.as_secs().max(1);
    let rounded = v.div_ceil(s) * s;
    Duration::from_secs(rounded)
}
