// =========================================
// =========================================
// src/ui/inspector_panel.rs

use crate::core::export::get_media_duration;
use crate::core::global_state::{
    ClipKeyframeChannel, GlobalState, MAX_LOCAL_MASK_LAYERS, MediaPoolUiEvent,
    SemanticSchemaValidation,
};
use crate::ui::motionloom_templates;
use crate::ui::painting::mask_editor::{MaskPaintTool, MaskPainterState};
use async_trait::async_trait;
use gpui::{
    Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, Render, Subscription, Window, div, prelude::*, px, relative, rgb, rgba, white,
};
use gpui_component::{
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    slider::{Slider, SliderEvent, SliderState},
};
use media_gen_protocol::auth::StaticKeyResolver;
use media_gen_protocol::job::InMemoryJobStore;
use media_gen_protocol::model::{ModelRegistry, ModelSpec};
use media_gen_protocol::output::OutputUploader;
use media_gen_protocol::provider::google_genai::GoogleGenAiAdapter;
use media_gen_protocol::provider::openai::OpenAiAdapter;
use media_gen_protocol::{
    AspectRatioResolutionMap, AssetKind as ProtocolAssetKind, ErrorCode as ProtocolErrorCode,
    GatewayContext as ProtocolGatewayContext, GatewayService as ProtocolGatewayService,
    GenerateRequest as ProtocolGenerateRequest, ImageResolutionPreset,
    InputAsset as ProtocolInputAsset, InputAssetKind as ProtocolInputAssetKind,
    ModelResolutionCatalog, ProtocolError as MediaGenProtocolError, VideoResolutionConstraint,
    VideoResolutionConstraintMap, model_resolution_catalog,
};
use motionloom::{GraphApplyScope, compile_runtime_program, is_graph_script, parse_graph_script};
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

use crate::core::global_state::SlideDirection;

#[derive(Clone)]
struct SubtitleFont {
    label: String,
    family: String,
    path: String,
}

impl SelectItem for SubtitleFont {
    type Value = String;

    fn title(&self) -> gpui::SharedString {
        gpui::SharedString::from(self.label.clone())
    }

    fn value(&self) -> &Self::Value {
        &self.path
    }
}

#[derive(Clone)]
struct SemanticSelectOption {
    label: String,
    value: String,
}

impl SelectItem for SemanticSelectOption {
    type Value = String;

    fn title(&self) -> gpui::SharedString {
        gpui::SharedString::from(self.label.clone())
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

struct LayerFxCurveEditor {
    pass_id: String,
    effect_name: String,
    param_key: String,
    param_label: String,
    value_min: f32,
    value_max: f32,
    duration_sec: f32,
    points: Vec<LayerFxCurvePoint>,
    selected_point: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LayerFxCurveEase {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

#[derive(Clone, Copy)]
struct LayerFxCurvePoint {
    t_sec: f32,
    value: f32,
    ease: LayerFxCurveEase,
}

#[derive(Clone, Copy)]
struct LayerFxCurveDragState {
    row_idx: usize,
    point_idx: usize,
    start_mouse_x: f32,
    start_mouse_y: f32,
    start_t_sec: f32,
    start_value: f32,
}

#[derive(Clone, Copy)]
struct CurveParamSpec {
    key: &'static str,
    label: &'static str,
    value_min: f32,
    value_max: f32,
    default_value: f32,
}

// Tracks state for click-to-edit slider value input
struct EditingSliderInfo {
    key: String,
    input: Entity<InputState>,
    _input_sub: Subscription,
    min: f32,
    max: f32,
}

pub struct InspectorPanel {
    pub global: Entity<GlobalState>,
    pub focus_handle: FocusHandle,
    subtitle_editing_id: Option<u64>,
    subtitle_input: Option<Entity<InputState>>,
    subtitle_input_sub: Option<Subscription>,
    sub_pos_x_slider: Option<Entity<SliderState>>,
    sub_pos_x_sub: Option<Subscription>,
    sub_pos_y_slider: Option<Entity<SliderState>>,
    sub_pos_y_sub: Option<Subscription>,
    sub_size_slider: Option<Entity<SliderState>>,
    sub_size_sub: Option<Subscription>,
    sub_group_size_slider: Option<Entity<SliderState>>,
    sub_group_size_sub: Option<Subscription>,
    subtitle_fonts: Vec<SubtitleFont>,
    subtitle_font_select: Option<Entity<SelectState<SearchableVec<SubtitleFont>>>>,
    subtitle_font_select_sub: Option<Subscription>,
    subtitle_color_hex_input: Option<Entity<InputState>>,
    subtitle_color_hex_sub: Option<Subscription>,
    subtitle_edit_mode: SubtitleEditMode,
    active_local_mask_layer: usize,
    sub_color_hue_slider: Option<Entity<SliderState>>,
    sub_color_hue_sub: Option<Subscription>,
    sub_color_sat_slider: Option<Entity<SliderState>>,
    sub_color_sat_sub: Option<Subscription>,
    sub_color_lum_slider: Option<Entity<SliderState>>,
    sub_color_lum_sub: Option<Subscription>,
    sub_color_alpha_slider: Option<Entity<SliderState>>,
    sub_color_alpha_sub: Option<Subscription>,
    audio_clip_gain_slider: Option<Entity<SliderState>>,
    audio_clip_gain_sub: Option<Subscription>,

    // ── Semantic clip label editing ──
    semantic_editing_id: Option<u64>,
    semantic_type_input: Option<Entity<InputState>>,
    semantic_type_input_sub: Option<Subscription>,
    semantic_label_input: Option<Entity<InputState>>,
    semantic_label_input_sub: Option<Subscription>,
    semantic_prompt_input: Option<Entity<InputState>>,
    semantic_prompt_input_sub: Option<Subscription>,
    semantic_image_api_key: String,
    semantic_image_api_key_placeholder: String,
    semantic_image_api_key_input: Option<Entity<InputState>>,
    semantic_image_api_key_input_sub: Option<Subscription>,
    semantic_input_image_path: String,
    semantic_input_image_path_input: Option<Entity<InputState>>,
    semantic_input_image_path_input_sub: Option<Subscription>,
    semantic_input_mask_path: String,
    semantic_input_mask_path_input: Option<Entity<InputState>>,
    semantic_input_mask_path_input_sub: Option<Subscription>,
    semantic_output_width: String,
    semantic_output_width_input: Option<Entity<InputState>>,
    semantic_output_width_input_sub: Option<Subscription>,
    semantic_output_height: String,
    semantic_output_height_input: Option<Entity<InputState>>,
    semantic_output_height_input_sub: Option<Subscription>,
    semantic_resolution_select: Option<Entity<SelectState<SearchableVec<SemanticSelectOption>>>>,
    semantic_resolution_select_sub: Option<Subscription>,
    semantic_resolution_select_sig: String,
    semantic_selected_resolution: String,
    semantic_resolution_apply_pending: bool,
    semantic_mask_painter: MaskPainterState,
    semantic_generate_status: String,
    semantic_schema_input: Option<Entity<InputState>>,
    semantic_schema_input_sub: Option<Subscription>,
    semantic_schema_text: String,
    semantic_schema_clip_id: Option<u64>,
    semantic_schema_mode: String,
    semantic_schema_status: String,
    semantic_schema_modal_open: bool,

    // =========================================================
    // 1. HSLA Overlay Sliders
    // =========================================================
    hue_slider: Option<Entity<SliderState>>,
    hue_sub: Option<Subscription>,
    sat_slider: Option<Entity<SliderState>>,
    sat_sub: Option<Subscription>, // HSLA Saturation
    lum_slider: Option<Entity<SliderState>>,
    lum_sub: Option<Subscription>,
    alpha_slider: Option<Entity<SliderState>>,
    alpha_sub: Option<Subscription>,

    // =========================================================
    // 2. TRANSFORM Sliders
    // =========================================================
    scale_slider: Option<Entity<SliderState>>,
    scale_sub: Option<Subscription>,
    rotation_slider: Option<Entity<SliderState>>,
    rotation_sub: Option<Subscription>,
    pos_x_slider: Option<Entity<SliderState>>,
    pos_x_sub: Option<Subscription>,
    pos_y_slider: Option<Entity<SliderState>>,
    pos_y_sub: Option<Subscription>,

    // =========================================================
    // 3. VIDEO EFFECT Sliders
    // =========================================================
    bright_slider: Option<Entity<SliderState>>,
    bright_sub: Option<Subscription>,
    contrast_slider: Option<Entity<SliderState>>,
    contrast_sub: Option<Subscription>,
    vid_sat_slider: Option<Entity<SliderState>>,
    vid_sat_sub: Option<Subscription>, // Video Saturation
    opacity_slider: Option<Entity<SliderState>>,
    opacity_sub: Option<Subscription>,
    blur_slider: Option<Entity<SliderState>>,
    blur_sub: Option<Subscription>,
    layer_brightness_slider: Option<Entity<SliderState>>,
    layer_brightness_sub: Option<Subscription>,
    layer_contrast_slider: Option<Entity<SliderState>>,
    layer_contrast_sub: Option<Subscription>,
    layer_saturation_slider: Option<Entity<SliderState>>,
    layer_saturation_sub: Option<Subscription>,
    layer_blur_slider: Option<Entity<SliderState>>,
    layer_blur_sub: Option<Subscription>,
    local_mask_center_x_slider: Option<Entity<SliderState>>,
    local_mask_center_x_sub: Option<Subscription>,
    local_mask_center_y_slider: Option<Entity<SliderState>>,
    local_mask_center_y_sub: Option<Subscription>,
    local_mask_radius_slider: Option<Entity<SliderState>>,
    local_mask_radius_sub: Option<Subscription>,
    local_mask_feather_slider: Option<Entity<SliderState>>,
    local_mask_feather_sub: Option<Subscription>,
    local_mask_strength_slider: Option<Entity<SliderState>>,
    local_mask_strength_sub: Option<Subscription>,
    local_mask_bright_slider: Option<Entity<SliderState>>,
    local_mask_bright_sub: Option<Subscription>,
    local_mask_contrast_slider: Option<Entity<SliderState>>,
    local_mask_contrast_sub: Option<Subscription>,
    local_mask_sat_slider: Option<Entity<SliderState>>,
    local_mask_sat_sub: Option<Subscription>,
    local_mask_opacity_slider: Option<Entity<SliderState>>,
    local_mask_opacity_sub: Option<Subscription>,
    local_mask_blur_slider: Option<Entity<SliderState>>,
    local_mask_blur_sub: Option<Subscription>,
    fade_in_slider: Option<Entity<SliderState>>,
    fade_in_sub: Option<Subscription>,
    fade_out_slider: Option<Entity<SliderState>>,
    fade_out_sub: Option<Subscription>,
    dissolve_in_slider: Option<Entity<SliderState>>,
    dissolve_in_sub: Option<Subscription>,
    dissolve_out_slider: Option<Entity<SliderState>>,
    dissolve_out_sub: Option<Subscription>,
    slide_in_slider: Option<Entity<SliderState>>,
    slide_in_sub: Option<Subscription>,
    slide_out_slider: Option<Entity<SliderState>>,
    slide_out_sub: Option<Subscription>,
    zoom_in_slider: Option<Entity<SliderState>>,
    zoom_in_sub: Option<Subscription>,
    zoom_out_slider: Option<Entity<SliderState>>,
    zoom_out_sub: Option<Subscription>,
    zoom_amount_slider: Option<Entity<SliderState>>,
    zoom_amount_sub: Option<Subscription>,
    shock_in_slider: Option<Entity<SliderState>>,
    shock_in_sub: Option<Subscription>,
    shock_out_slider: Option<Entity<SliderState>>,
    shock_out_sub: Option<Subscription>,
    shock_amount_slider: Option<Entity<SliderState>>,
    shock_amount_sub: Option<Subscription>,
    layer_fx_script_input: Option<Entity<InputState>>,
    layer_fx_script_input_sub: Option<Subscription>,
    layer_fx_script_text: String,
    layer_fx_script_layer_id: Option<u64>,
    layer_fx_script_status: String,
    layer_fx_script_modal_open: bool,
    layer_fx_template_modal_open: bool,
    layer_fx_template_add_time_parameter: bool,
    layer_fx_template_add_curve_parameter: bool,
    layer_fx_template_selected: Vec<motionloom_templates::LayerEffectTemplateKind>,
    layer_fx_curve_editors: Vec<LayerFxCurveEditor>,
    layer_fx_curve_drag: Option<LayerFxCurveDragState>,
    layer_fx_curve_open_menu: Option<(usize, usize)>,
    // Manual slider value editing (click value text to type)
    editing_slider: Option<EditingSliderInfo>,
    selected_clip_keyframe_channel: Option<ClipKeyframeChannel>,
    state_sig: u64,
}

const CURVE_GRAPH_W: f32 = 360.0;
const CURVE_GRAPH_H: f32 = 120.0;
const CURVE_TIME_EPS: f32 = 0.01;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SubtitleEditMode {
    Individual,
    Group,
}

#[derive(Debug, Clone)]
struct SemanticFileOutputUploader {
    output_path: PathBuf,
}

impl SemanticFileOutputUploader {
    fn new(output_path: PathBuf) -> Self {
        Self { output_path }
    }
}

#[async_trait]
impl OutputUploader for SemanticFileOutputUploader {
    async fn upload_bytes(
        &self,
        _content_type: &str,
        bytes: &[u8],
    ) -> media_gen_protocol::Result<String> {
        if let Some(parent) = self.output_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                MediaGenProtocolError::new(
                    ProtocolErrorCode::OutputStoreFailed,
                    format!(
                        "failed to create semantic output directory '{}': {err}",
                        parent.display()
                    ),
                )
                .with_provider("local")
            })?;
        }
        fs::write(&self.output_path, bytes).map_err(|err| {
            MediaGenProtocolError::new(
                ProtocolErrorCode::OutputStoreFailed,
                format!(
                    "failed to write semantic output '{}': {err}",
                    self.output_path.display()
                ),
            )
            .with_provider("local")
        })?;
        Ok(self.output_path.to_string_lossy().to_string())
    }
}

mod audio;
mod core;
mod layer_fx;
mod render;
mod semantic_layer;
mod subtitle;
mod video;

#[cfg(test)]
mod tests;
