// =========================================
// =========================================
// src/ui/video_preview.rs

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use core_foundation::base::{CFType, TCFType};
#[cfg(target_os = "macos")]
use core_foundation::boolean::CFBoolean;
#[cfg(target_os = "macos")]
use core_foundation::dictionary::CFDictionary;
#[cfg(target_os = "macos")]
use core_foundation::string::CFString;
#[cfg(target_os = "macos")]
use core_video::pixel_buffer::{
    CVPixelBuffer, CVPixelBufferKeys, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
};
#[cfg(target_os = "macos")]
use core_video::r#return::kCVReturnSuccess;
#[cfg(target_os = "macos")]
use gpui::SurfaceExParams_anica;
use gpui::{
    Context, Element, ElementId, Entity, FocusHandle, Focusable, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Negate as _, Render, RenderImage, Style,
    TransformationMatrix, Window, div, prelude::*, px, radians, rgb,
};
use gpui_component::{black, white};
use image::{
    ImageBuffer, Rgba,
    imageops::{self, FilterType},
};
use smallvec::SmallVec;
use url::Url;

use crate::core::effects::{PerClipColorBlurEffects, combine_clip_with_layer};
#[cfg(target_os = "macos")]
use crate::core::global_state::MacPreviewRenderMode;
use crate::core::global_state::PreviewQuality;
use crate::core::global_state::{AudioTrack, GlobalState, PlaybackUiEvent, SubtitleClip};
use crate::core::proxy;
use crate::core::proxy::ProxyStatus;
use crate::core::waveform;
use crate::core::waveform::WaveformStatus;
// Import the engine and renderer
use gpui_video_renderer::{
    VIDEO_MAX_LOCAL_MASK_LAYERS, VideoElement, VideoLocalMaskLayer, bgra_cpu_safe_mode_notice,
    process_bgra_effects,
};
use video_engine::{Position, Video, VideoOptions};

const PREVIEW_BASE_HEIGHT: f32 = 450.0;
const PREVIEW_MIN_HEIGHT: f32 = 260.0;
const PREVIEW_MAX_HEIGHT: f32 = 760.0;
const PREVIEW_MAX_WIDTH: f32 = 1800.0;
const SIDEBAR_W: f32 = 64.0;
const EDITOR_PANEL_W: f32 = 300.0;
const TIMELINE_PANEL_H: f32 = 364.0;
const DEFAULT_VISUAL_PLAYER_CACHE_LIMIT: usize = 16;
const DEFAULT_AUDIO_PLAYER_CACHE_LIMIT: usize = 16;
const DEFAULT_IMAGE_CACHE_LIMIT: usize = 32;
const DEFAULT_IMAGE_MAX_DIM_FULL: u32 = 1280;
const COLOR_KEY_SCALE: f32 = 1000.0;
const SYNC_SLOW_MS: u128 = 80;
const SCRUB_REFRESH_TAIL_MS: u64 = 120;
const FAST_BLUR_SETTLE_MS: u64 = 220;
const AUDIO_PREWARM_LOOKAHEAD_MS: u64 = 350;
const VIDEO_INPUT_FPS_SAMPLE_MS: u64 = 300;
const PRESENT_FPS_SAMPLE_MS: u64 = 300;
const PRESENT_FRAME_DT_MIN_S: f32 = 1.0 / 240.0;
const PRESENT_FRAME_DT_MAX_S: f32 = 0.200;
const MB: usize = 1024 * 1024;

/// Helper to determine if a file path points to an image.
fn is_image_ext(path: &str) -> bool {
    let p = path.to_lowercase();
    p.ends_with(".jpg")
        || p.ends_with(".jpeg")
        || p.ends_with(".png")
        || p.ends_with(".webp")
        || p.ends_with(".bmp")
}

fn fitted_media_bounds(
    width: u32,
    height: u32,
    bounds: gpui::Bounds<gpui::Pixels>,
) -> gpui::Bounds<gpui::Pixels> {
    let container_w: f32 = bounds.size.width.into();
    let container_h: f32 = bounds.size.height.into();
    let frame_w = width as f32;
    let frame_h = height as f32;

    if frame_w == 0.0 || frame_h == 0.0 {
        return bounds;
    }

    let scale = (container_w / frame_w).min(container_h / frame_h);
    let dest_w = frame_w * scale;
    let dest_h = frame_h * scale;
    let offset_x = (container_w - dest_w) * 0.5;
    let offset_y = (container_h - dest_h) * 0.5;

    gpui::Bounds::new(
        gpui::point(
            bounds.origin.x + gpui::px(offset_x),
            bounds.origin.y + gpui::px(offset_y),
        ),
        gpui::size(gpui::px(dest_w), gpui::px(dest_h)),
    )
}

#[derive(Clone)]
struct ImageCache {
    base_data: Vec<u8>,
    width: u32,
    height: u32,
    has_transparency: bool,
    render_width: u32,
    render_height: u32,
    render_key: Option<ImageRenderKey>,
    render_image: Option<Arc<RenderImage>>,
    /// Key for the effect render currently being computed in a background thread.
    pending_effect_key: Option<ImageRenderKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ImageRenderKey {
    brightness: i16,
    contrast: i16,
    saturation: i16,
    lut_mix: i16,
    blur_sigma: i16,
    tint_hue: i16,
    tint_saturation: i16,
    tint_lightness: i16,
    tint_alpha: i16,
    fast_mode: bool,
}

impl ImageRenderKey {
    /// Quantize color controls so static clips reuse one cached texture across frames.
    fn from_values(
        brightness: f32,
        contrast: f32,
        saturation: f32,
        lut_mix: f32,
        blur_sigma: f32,
        tint_hue: f32,
        tint_saturation: f32,
        tint_lightness: f32,
        tint_alpha: f32,
        fast_mode: bool,
    ) -> Self {
        Self {
            brightness: (brightness * COLOR_KEY_SCALE).round() as i16,
            contrast: (contrast * COLOR_KEY_SCALE).round() as i16,
            saturation: (saturation * COLOR_KEY_SCALE).round() as i16,
            lut_mix: (lut_mix * COLOR_KEY_SCALE).round() as i16,
            blur_sigma: (blur_sigma * COLOR_KEY_SCALE).round() as i16,
            tint_hue: (tint_hue * COLOR_KEY_SCALE).round() as i16,
            tint_saturation: (tint_saturation * COLOR_KEY_SCALE).round() as i16,
            tint_lightness: (tint_lightness * COLOR_KEY_SCALE).round() as i16,
            tint_alpha: (tint_alpha * COLOR_KEY_SCALE).round() as i16,
            fast_mode,
        }
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MacVideoEffectKey {
    brightness: i16,
    contrast: i16,
    saturation: i16,
    lut_mix: i16,
    opacity: i16,
    blur_sigma: i16,
}

#[cfg(target_os = "macos")]
impl MacVideoEffectKey {
    fn from_values(
        brightness: f32,
        contrast: f32,
        saturation: f32,
        lut_mix: f32,
        opacity: f32,
        blur_sigma: f32,
    ) -> Self {
        Self {
            brightness: (brightness * COLOR_KEY_SCALE).round() as i16,
            contrast: (contrast * COLOR_KEY_SCALE).round() as i16,
            saturation: (saturation * COLOR_KEY_SCALE).round() as i16,
            lut_mix: (lut_mix * COLOR_KEY_SCALE).round() as i16,
            opacity: (opacity * COLOR_KEY_SCALE).round() as i16,
            blur_sigma: (blur_sigma * COLOR_KEY_SCALE).round() as i16,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PreviewMemoryTuning {
    budget_mb: usize,
    budget_bytes: usize,
    enforce_budget: bool,
    visual_cache_limit: usize,
    audio_cache_limit: usize,
    image_cache_limit: usize,
    frame_buffer_capacity: usize,
    appsink_max_buffers: u32,
    estimated_audio_player_bytes: usize,
}

#[derive(Clone, Debug, Default)]
struct AudioTrackTimeIndex {
    // Start times in track order (same index as `track.clips[idx]`).
    starts_ns: Vec<u64>,
    // Prefix max of clip end times; enables binary search for the first possibly-active clip.
    prefix_max_end_ns: Vec<u64>,
    clip_count: usize,
    first_clip_id: u64,
    first_start_ns: u64,
    first_duration_ns: u64,
    last_clip_id: u64,
    last_start_ns: u64,
    last_duration_ns: u64,
}

impl PreviewMemoryTuning {
    fn from_budget_mb(raw_budget_mb: usize) -> Self {
        // Keep budget in a sane range to avoid pathological cache sizing.
        let budget_mb = raw_budget_mb.clamp(256, 32768);
        let budget_bytes = budget_mb.saturating_mul(MB);

        // Scale cache sizes with budget while preserving prior baseline behavior at low values.
        let visual_cache_limit = (budget_mb / 128).clamp(DEFAULT_VISUAL_PLAYER_CACHE_LIMIT, 128);
        let audio_cache_limit = (budget_mb / 128).clamp(DEFAULT_AUDIO_PLAYER_CACHE_LIMIT, 128);
        let image_cache_limit = (budget_mb / 16).clamp(DEFAULT_IMAGE_CACHE_LIMIT, 1024);
        let frame_buffer_capacity = (budget_mb / 320).clamp(3, 48);
        let appsink_max_buffers = (budget_mb / 320).clamp(1, 48) as u32;

        Self {
            budget_mb,
            budget_bytes,
            enforce_budget: true,
            visual_cache_limit,
            audio_cache_limit,
            image_cache_limit,
            frame_buffer_capacity,
            appsink_max_buffers,
            // Small conservative estimate; most memory pressure comes from decoded video frames.
            estimated_audio_player_bytes: 4 * MB,
        }
    }

    fn unlimited() -> Self {
        Self {
            budget_mb: 0,
            budget_bytes: usize::MAX,
            enforce_budget: false,
            // "No limit" in UI still keeps practical caps so preview cannot grow unbounded.
            visual_cache_limit: 128,
            audio_cache_limit: 128,
            image_cache_limit: 1024,
            frame_buffer_capacity: 48,
            appsink_max_buffers: 48,
            estimated_audio_player_bytes: 4 * MB,
        }
    }

    fn from_budget_option(budget_mb: Option<usize>) -> Self {
        match budget_mb {
            Some(v) if v > 0 => Self::from_budget_mb(v),
            _ => Self::unlimited(),
        }
    }

    fn parse_budget_env() -> Option<usize> {
        let raw = std::env::var("ANICA_PREVIEW_RAM_BUDGET_MB").ok()?;
        let normalized = raw.trim().to_ascii_lowercase();
        if normalized.is_empty()
            || normalized == "none"
            || normalized == "off"
            || normalized == "unlimited"
            || normalized == "0"
        {
            return None;
        }
        normalized.parse::<usize>().ok()
    }

    fn load(initial_budget_mb: Option<usize>) -> Self {
        let configured = initial_budget_mb.or_else(Self::parse_budget_env);
        let tuning = Self::from_budget_option(configured);
        if tuning.enforce_budget {
            log::info!(
                "[Preview][Memory] budget_mb={} visual_limit={} audio_limit={} image_limit={} frame_buffer={} appsink_max_buffers={}",
                tuning.budget_mb,
                tuning.visual_cache_limit,
                tuning.audio_cache_limit,
                tuning.image_cache_limit,
                tuning.frame_buffer_capacity,
                tuning.appsink_max_buffers
            );
        } else {
            log::info!(
                "[Preview][Memory] budget=none visual_limit={} audio_limit={} image_limit={} frame_buffer={} appsink_max_buffers={}",
                tuning.visual_cache_limit,
                tuning.audio_cache_limit,
                tuning.image_cache_limit,
                tuning.frame_buffer_capacity,
                tuning.appsink_max_buffers
            );
        }
        tuning
    }
}

struct ImageElement {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
    rotation_deg: f32,
    element_id: Option<ElementId>,
}

impl ImageElement {
    fn new(image: Arc<RenderImage>, width: u32, height: u32) -> Self {
        Self {
            image,
            width,
            height,
            rotation_deg: 0.0,
            element_id: None,
        }
    }

    fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.element_id = Some(id.into());
        self
    }

    fn rotation_deg(mut self, rotation_deg: f32) -> Self {
        self.rotation_deg = rotation_deg.clamp(-180.0, 180.0);
        self
    }

    fn transformation(
        &self,
        bounds: gpui::Bounds<gpui::Pixels>,
        scale_factor: f32,
    ) -> TransformationMatrix {
        if self.rotation_deg.abs() < 0.001 {
            return TransformationMatrix::unit();
        }

        let center = bounds.center().scale(scale_factor);
        TransformationMatrix::unit()
            .translate(center)
            .rotate(radians(self.rotation_deg.to_radians()))
            .translate(center.negate())
    }

    fn fitted_bounds(&self, bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
        fitted_media_bounds(self.width, self.height, bounds)
    }
}

impl Element for ImageElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        self.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: gpui::Size {
                width: gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0)),
                height: gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0)),
            },
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _state: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut gpui::App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut gpui::App,
    ) {
        let dest_bounds = self.fitted_bounds(bounds);
        let transformation = self.transformation(dest_bounds, window.scale_factor());
        window
            .paint_image_anica(
                dest_bounds,
                gpui::Corners::default(),
                self.image.clone(),
                0,
                false,
                transformation,
            )
            .ok();
    }
}

impl IntoElement for ImageElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

#[cfg(target_os = "macos")]
struct SurfaceImageElement {
    surface: CVPixelBuffer,
    width: u32,
    height: u32,
    rotation_deg: f32,
    opacity: f32,
    element_id: Option<ElementId>,
}

#[cfg(target_os = "macos")]
impl SurfaceImageElement {
    fn new(surface: CVPixelBuffer, width: u32, height: u32) -> Self {
        Self {
            surface,
            width,
            height,
            rotation_deg: 0.0,
            opacity: 1.0,
            element_id: None,
        }
    }

    fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.element_id = Some(id.into());
        self
    }

    fn rotation_deg(mut self, rotation_deg: f32) -> Self {
        self.rotation_deg = rotation_deg.clamp(-180.0, 180.0);
        self
    }

    fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    fn fitted_bounds(&self, bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
        fitted_media_bounds(self.width, self.height, bounds)
    }
}

#[cfg(target_os = "macos")]
impl Element for SurfaceImageElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        self.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: gpui::Size {
                width: gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0)),
                height: gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0)),
            },
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _state: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut gpui::App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut gpui::App,
    ) {
        let dest_bounds = self.fitted_bounds(bounds);
        window.paint_surface_anica(
            dest_bounds,
            self.surface.clone(),
            SurfaceExParams_anica {
                opacity: self.opacity,
                rotation_deg: self.rotation_deg,
                ..Default::default()
            },
        );
    }
}

#[cfg(target_os = "macos")]
impl IntoElement for SurfaceImageElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct VideoPreview {
    /// The main visual players responsible for visual output.
    visual_players: HashMap<u64, Video>,
    visual_order: Vec<u64>,
    audio_players: HashMap<u64, Video>,
    last_seek_requests: HashMap<u64, Duration>,
    pub global: Entity<GlobalState>,
    focus_handle: Option<FocusHandle>,
    pump_running: bool,
    pump_token: u64,
    last_pump_instant: Option<Instant>,
    image_cache: HashMap<u64, ImageCache>,
    image_cache_paths: HashMap<u64, String>,
    #[cfg(target_os = "macos")]
    mac_image_surfaces: HashMap<u64, (ImageRenderKey, CVPixelBuffer)>,
    video_cache_paths: HashMap<u64, String>,
    visual_last_used: HashMap<u64, u64>,
    audio_last_used: HashMap<u64, u64>,
    image_last_used: HashMap<u64, u64>,
    visual_paused_state: HashMap<u64, bool>,
    audio_paused_state: HashMap<u64, bool>,
    last_active_visual_ids: HashSet<u64>,
    last_active_audio_ids: HashSet<u64>,
    image_decode_in_flight: HashSet<u64>,
    cache_touch_counter: u64,
    last_debug_log_at: Option<Instant>,
    max_visual_cached_seen: usize,
    max_audio_cached_seen: usize,
    max_image_cached_seen: usize,
    max_seek_entries_seen: usize,
    last_preview_fps: u32,
    last_preview_quality: PreviewQuality,
    last_pump_playhead: Option<Duration>,
    scrub_refresh_until: Option<Instant>,
    blur_interaction_until: Option<Instant>,
    last_clip_blur_sigmas: HashMap<u64, f32>,
    video_blur_keys: HashMap<u64, i16>,
    last_video_frame_counters: HashMap<u64, u64>,
    video_input_fps_window_start: Option<Instant>,
    video_input_fps_window_frames: u32,
    video_input_fps_ema: f32,
    present_last_frame_instant: Option<Instant>,
    present_fps_window_start: Option<Instant>,
    present_fps_window_frames: u32,
    present_fps_ema: f32,
    present_refresh_interval_estimate_s: f32,
    present_dropped_frames_total: u64,
    memory_tuning: PreviewMemoryTuning,
    audio_track_time_indices: HashMap<usize, AudioTrackTimeIndex>,
    audio_track_index_token: u64,
    #[cfg(target_os = "macos")]
    mac_video_effect_keys: HashMap<u64, MacVideoEffectKey>,
    #[cfg(target_os = "macos")]
    mac_surface_mode_keys: HashMap<u64, bool>,
}

impl VideoPreview {
    pub fn new(global: Entity<GlobalState>, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&global, |_, _, evt: &PlaybackUiEvent, cx| {
            if matches!(evt, PlaybackUiEvent::Tick) {
                cx.notify();
            }
        })
        .detach();

        let configured_budget_mb = global.read(cx).preview_memory_budget_mb;
        let memory_tuning = PreviewMemoryTuning::load(configured_budget_mb);
        Self {
            visual_players: HashMap::new(),
            visual_order: Vec::new(),
            audio_players: HashMap::new(),
            last_seek_requests: HashMap::new(),
            global,
            focus_handle: Some(cx.focus_handle()),
            pump_running: false,
            pump_token: 0,
            last_pump_instant: None,
            image_cache: HashMap::new(),
            image_cache_paths: HashMap::new(),
            #[cfg(target_os = "macos")]
            mac_image_surfaces: HashMap::new(),
            video_cache_paths: HashMap::new(),
            visual_last_used: HashMap::new(),
            audio_last_used: HashMap::new(),
            image_last_used: HashMap::new(),
            visual_paused_state: HashMap::new(),
            audio_paused_state: HashMap::new(),
            last_active_visual_ids: HashSet::new(),
            last_active_audio_ids: HashSet::new(),
            image_decode_in_flight: HashSet::new(),
            cache_touch_counter: 0,
            last_debug_log_at: None,
            max_visual_cached_seen: 0,
            max_audio_cached_seen: 0,
            max_image_cached_seen: 0,
            max_seek_entries_seen: 0,
            last_preview_fps: 60,
            last_preview_quality: PreviewQuality::Full,
            last_pump_playhead: None,
            scrub_refresh_until: None,
            blur_interaction_until: None,
            last_clip_blur_sigmas: HashMap::new(),
            video_blur_keys: HashMap::new(),
            last_video_frame_counters: HashMap::new(),
            video_input_fps_window_start: None,
            video_input_fps_window_frames: 0,
            video_input_fps_ema: 0.0,
            present_last_frame_instant: None,
            present_fps_window_start: None,
            present_fps_window_frames: 0,
            present_fps_ema: 0.0,
            present_refresh_interval_estimate_s: 0.0,
            present_dropped_frames_total: 0,
            memory_tuning,
            audio_track_time_indices: HashMap::new(),
            audio_track_index_token: 0,
            #[cfg(target_os = "macos")]
            mac_video_effect_keys: HashMap::new(),
            #[cfg(target_os = "macos")]
            mac_surface_mode_keys: HashMap::new(),
        }
    }

    pub fn set_memory_budget_mb(&mut self, budget_mb: Option<usize>) {
        self.memory_tuning = PreviewMemoryTuning::load(budget_mb);

        // Existing players can apply only frame-buffer capacity at runtime.
        // appsink_max_buffers applies on next player creation/reload.
        for player in self.visual_players.values() {
            player.set_frame_buffer_capacity(self.memory_tuning.frame_buffer_capacity);
        }
        for player in self.audio_players.values() {
            player.set_frame_buffer_capacity(self.memory_tuning.frame_buffer_capacity);
        }
    }

    fn apply_color_correction(
        data: &mut [u8],
        brightness: f32,
        contrast: f32,
        saturation: f32,
        lut_mix: f32,
        opacity: f32,
    ) {
        let b = brightness.clamp(-1.0, 1.0) * 255.0;
        let c = contrast.clamp(0.0, 2.0);
        let s = saturation.clamp(0.0, 2.0);
        let lut_mix = lut_mix.clamp(0.0, 1.0);

        let opacity = opacity.clamp(0.0, 1.0);
        if b.abs() < 0.001
            && (c - 1.0).abs() < 0.001
            && (s - 1.0).abs() < 0.001
            && lut_mix < 0.001
            && (opacity - 1.0).abs() < 0.001
        {
            return;
        }

        for px in data.chunks_mut(4) {
            let b0 = px[0] as f32;
            let g0 = px[1] as f32;
            let r0 = px[2] as f32;

            let mut r = r0;
            let mut g = g0;
            let mut bch = b0;

            let l = 0.2126 * r + 0.7152 * g + 0.0722 * bch;
            r = l + (r - l) * s;
            g = l + (g - l) * s;
            bch = l + (bch - l) * s;

            r = (r - 128.0) * c + 128.0 + b;
            g = (g - 128.0) * c + 128.0 + b;
            bch = (bch - 128.0) * c + 128.0 + b;

            if lut_mix > 0.001 {
                let warm_r = r * 1.03;
                let warm_g = g;
                let warm_b = bch * 0.97;
                r = r + (warm_r - r) * lut_mix;
                g = g + (warm_g - g) * lut_mix;
                bch = bch + (warm_b - bch) * lut_mix;
            }

            px[2] = r.clamp(0.0, 255.0) as u8;
            px[1] = g.clamp(0.0, 255.0) as u8;
            px[0] = bch.clamp(0.0, 255.0) as u8;
            if (opacity - 1.0).abs() > 0.001 {
                px[3] = ((px[3] as f32) * opacity).clamp(0.0, 255.0) as u8;
            }
        }
    }

    fn apply_gaussian_blur(data: &mut Vec<u8>, width: u32, height: u32, sigma: f32) {
        let sigma = sigma.clamp(0.0, 64.0);
        if sigma <= 0.001 || width == 0 || height == 0 {
            return;
        }

        let raw = data.clone();
        if let Some(buffer) = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, raw) {
            let blurred = imageops::blur(&buffer, sigma);
            *data = blurred.into_raw();
        }
    }

    fn apply_gaussian_blur_fast(data: &mut Vec<u8>, width: u32, height: u32, sigma: f32) {
        let sigma = sigma.clamp(0.0, 64.0);
        if sigma <= 0.001 || width == 0 || height == 0 {
            return;
        }

        let raw = data.clone();
        if let Some(buffer) = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, raw) {
            let downsample = if sigma >= 12.0 {
                0.25
            } else if sigma >= 6.0 {
                0.33
            } else {
                0.5
            };
            let small_w = ((width as f32) * downsample).round().max(1.0) as u32;
            let small_h = ((height as f32) * downsample).round().max(1.0) as u32;
            let reduced = imageops::resize(&buffer, small_w, small_h, FilterType::Triangle);
            let reduced_sigma = (sigma * downsample).max(0.1);
            let blurred_small = imageops::blur(&reduced, reduced_sigma);
            let upscaled = imageops::resize(&blurred_small, width, height, FilterType::Triangle);
            *data = upscaled.into_raw();
        }
    }

    fn apply_unsharp(data: &mut Vec<u8>, width: u32, height: u32, sigma: f32, amount: f32) {
        let sigma = sigma.clamp(0.0, 64.0);
        if sigma <= 0.001 || width == 0 || height == 0 {
            return;
        }
        let amount = amount.clamp(0.0, 4.0);
        if amount <= 0.0001 {
            return;
        }

        let base = data.clone();
        if let Some(buffer) = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, base.clone()) {
            let blurred = imageops::blur(&buffer, sigma).into_raw();
            let mut out = base;
            for i in (0..out.len()).step_by(4) {
                // BGRA order in this path.
                for ch in 0..3 {
                    let b = out[i + ch] as f32 / 255.0;
                    let bl = blurred[i + ch] as f32 / 255.0;
                    let v = (b + (b - bl) * amount).clamp(0.0, 1.0);
                    out[i + ch] = (v * 255.0 + 0.5) as u8;
                }
            }
            *data = out;
        }
    }

    fn apply_preview_sharpen(
        data: &mut Vec<u8>,
        width: u32,
        height: u32,
        sharpen_sigma: f32,
        fast_blur_mode: bool,
    ) {
        let sharpen_sigma = sharpen_sigma.clamp(0.0, 64.0);
        if sharpen_sigma <= 0.001 || width == 0 || height == 0 {
            return;
        }

        if fast_blur_mode {
            // Keep interactive scrub smooth.
            let sigma = (sharpen_sigma * 0.6).max(0.5);
            Self::apply_unsharp(data, width, height, sigma, 0.85);
            return;
        }

        // Mirror export sharpen strategy:
        // - low sigma: single pass
        // - high sigma (>=7): dual pass with increasing directional intent.
        if sharpen_sigma >= 7.0 {
            let step = ((sharpen_sigma - 7.0) / (64.0 - 7.0)).clamp(0.0, 1.0);
            let major_step = (step.sqrt() * 5.0).floor() as i32; // 0..5
            let major = (13 + major_step * 2).clamp(13, 23) as f32;
            let minor = (13 - major_step * 2).clamp(3, 13) as f32;
            let amount = (1.00 + step * 0.35).clamp(0.0, 4.0);

            if major_step == 0 {
                Self::apply_unsharp(data, width, height, major * 0.5, amount);
            } else {
                Self::apply_unsharp(data, width, height, major * 0.5, amount);
                Self::apply_unsharp(data, width, height, minor * 0.5, amount);
            }
        } else {
            Self::apply_unsharp(data, width, height, sharpen_sigma, 1.05);
        }
    }

    fn build_render_image_from_bgra(
        data: Vec<u8>,
        width: u32,
        height: u32,
    ) -> Option<Arc<RenderImage>> {
        let buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, data)?;
        let frames = SmallVec::from_elem(image::Frame::new(buffer), 1);
        Some(Arc::new(RenderImage::new(frames)))
    }

    #[cfg(target_os = "macos")]
    fn build_surface_from_bgra(data: &[u8], width: u32, height: u32) -> Option<CVPixelBuffer> {
        let src_w = width as usize;
        let src_h = height as usize;
        if src_w == 0 || src_h == 0 || data.len() != src_w.checked_mul(src_h)?.checked_mul(4)? {
            return None;
        }

        let surface_w = if (src_w & 1) == 0 { src_w } else { src_w + 1 };
        let surface_h = if (src_h & 1) == 0 { src_h } else { src_h + 1 };

        let iosurface_props: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[]);
        let cv_options: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[
            (
                CFString::from(CVPixelBufferKeys::MetalCompatibility),
                CFBoolean::true_value().as_CFType(),
            ),
            (
                CFString::from(CVPixelBufferKeys::IOSurfaceProperties),
                iosurface_props.as_CFType(),
            ),
        ]);

        let pixel_buffer = CVPixelBuffer::new(
            kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
            surface_w,
            surface_h,
            Some(&cv_options),
        )
        .ok()?;
        if pixel_buffer.lock_base_address(0) != kCVReturnSuccess {
            return None;
        }

        let copied = (|| {
            if pixel_buffer.get_plane_count() < 2 {
                return None;
            }

            let y_stride = pixel_buffer.get_bytes_per_row_of_plane(0);
            let uv_stride = pixel_buffer.get_bytes_per_row_of_plane(1);
            let y_plane_h = pixel_buffer.get_height_of_plane(0);
            let uv_plane_h = pixel_buffer.get_height_of_plane(1);
            if y_stride < surface_w
                || uv_stride < surface_w
                || y_plane_h < surface_h
                || uv_plane_h < (surface_h / 2)
            {
                return None;
            }

            let y_ptr = unsafe { pixel_buffer.get_base_address_of_plane(0) as *mut u8 };
            let uv_ptr = unsafe { pixel_buffer.get_base_address_of_plane(1) as *mut u8 };
            if y_ptr.is_null() || uv_ptr.is_null() {
                return None;
            }

            let y_plane = unsafe { std::slice::from_raw_parts_mut(y_ptr, y_stride * y_plane_h) };
            let uv_plane =
                unsafe { std::slice::from_raw_parts_mut(uv_ptr, uv_stride * uv_plane_h) };

            let sample_rgb = |sx: usize, sy: usize| -> (f32, f32, f32) {
                let clamped_x = sx.min(src_w.saturating_sub(1));
                let clamped_y = sy.min(src_h.saturating_sub(1));
                let idx = (clamped_y * src_w + clamped_x) * 4;
                let b = data[idx] as f32;
                let g = data[idx + 1] as f32;
                let r = data[idx + 2] as f32;
                (r, g, b)
            };

            for y in 0..surface_h {
                for x in 0..surface_w {
                    let (r, g, b) = sample_rgb(x, y);
                    let luma = (0.299 * r + 0.587 * g + 0.114 * b).round() as i32;
                    y_plane[y * y_stride + x] = luma.clamp(0, 255) as u8;
                }
            }

            for y in (0..surface_h).step_by(2) {
                let uv_row = (y / 2) * uv_stride;
                for x in (0..surface_w).step_by(2) {
                    let samples = [
                        sample_rgb(x, y),
                        sample_rgb(x + 1, y),
                        sample_rgb(x, y + 1),
                        sample_rgb(x + 1, y + 1),
                    ];
                    let mut u_sum = 0.0;
                    let mut v_sum = 0.0;
                    for (r, g, b) in samples {
                        u_sum += -0.168_736 * r - 0.331_264 * g + 0.5 * b + 128.0;
                        v_sum += 0.5 * r - 0.418_688 * g - 0.081_312 * b + 128.0;
                    }
                    uv_plane[uv_row + x] = (u_sum * 0.25).round().clamp(0.0, 255.0) as u8;
                    uv_plane[uv_row + x + 1] = (v_sum * 0.25).round().clamp(0.0, 255.0) as u8;
                }
            }

            Some(())
        })()
        .is_some();

        let _ = pixel_buffer.unlock_base_address(0);
        if copied { Some(pixel_buffer) } else { None }
    }

    /// Decode an image into BGRA bytes and downscale it for preview to avoid huge allocations.
    fn decode_image_cache(path: &str, max_dim: Option<u32>, pixelate: bool) -> Option<ImageCache> {
        let mut img = image::open(path).ok()?;
        if let Some(limit) = max_dim {
            let (src_w, src_h) = (img.width(), img.height());
            let longest = src_w.max(src_h);
            if limit > 0 && longest > limit {
                let ratio = limit as f32 / longest as f32;
                let dst_w = ((src_w as f32) * ratio).round().max(1.0) as u32;
                let dst_h = ((src_h as f32) * ratio).round().max(1.0) as u32;
                let filter = if pixelate {
                    FilterType::Nearest
                } else {
                    FilterType::Triangle
                };
                img = img.resize_exact(dst_w, dst_h, filter);
            }
        }

        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let mut data = rgba.into_raw();
        let has_transparency = data.chunks(4).any(|px| px[3] < 255);
        for px in data.chunks_mut(4) {
            let r = px[0];
            let b = px[2];
            px[0] = b;
            px[2] = r;
        }

        Some(ImageCache {
            base_data: data,
            width: w,
            height: h,
            has_transparency,
            render_width: w,
            render_height: h,
            render_key: None,
            render_image: None,
            pending_effect_key: None,
        })
    }

    /// Track recent usage so we can evict old cached players/images instead of reallocating each loop.
    fn touch_cache(last_used: &mut HashMap<u64, u64>, counter: &mut u64, clip_id: u64) {
        *counter = counter.wrapping_add(1);
        last_used.insert(clip_id, *counter);
    }

    /// Clear image render cache entry before replacing/removing decoded image data.
    fn queue_cached_image_drop(&mut self, clip_id: u64) {
        if let Some(cache) = self.image_cache.get_mut(&clip_id) {
            cache.render_image = None;
            cache.render_key = None;
        }
        #[cfg(target_os = "macos")]
        self.mac_image_surfaces.remove(&clip_id);
    }

    /// Keep only a bounded number of inactive visual players to avoid boundary stutter from recreate/destroy churn.
    fn trim_visual_player_cache(&mut self, active_video_ids: &HashSet<u64>) {
        let visual_cache_limit = self.memory_tuning.visual_cache_limit;
        if self.visual_players.len() <= visual_cache_limit {
            return;
        }

        let mut removable: Vec<(u64, u64)> = self
            .visual_players
            .keys()
            .filter(|id| !active_video_ids.contains(id))
            .map(|id| (*id, *self.visual_last_used.get(id).unwrap_or(&0)))
            .collect();
        if removable.is_empty() {
            return;
        }

        removable.sort_by_key(|(_, used)| *used);

        for (clip_id, _) in removable {
            if self.visual_players.len() <= visual_cache_limit {
                break;
            }
            log::debug!("[Preview][Cache] evict visual clip={}", clip_id);
            self.visual_players.remove(&clip_id);
            self.video_cache_paths.remove(&clip_id);
            self.visual_last_used.remove(&clip_id);
            self.visual_paused_state.remove(&clip_id);
            self.last_seek_requests.remove(&clip_id);
            self.video_blur_keys.remove(&clip_id);
            #[cfg(target_os = "macos")]
            self.mac_video_effect_keys.remove(&clip_id);
        }

        if self.visual_players.len() > visual_cache_limit {
            log::warn!(
                "[Preview][Cache] visual cache remains above limit: size={} limit={} active_video={}",
                self.visual_players.len(),
                visual_cache_limit,
                active_video_ids.len()
            );
        }
    }

    /// Keep only a bounded number of inactive audio players to avoid create/drop churn during scrub.
    fn trim_audio_player_cache(&mut self, active_audio_ids: &HashSet<u64>) {
        let audio_cache_limit = self.memory_tuning.audio_cache_limit;
        if self.audio_players.len() <= audio_cache_limit {
            return;
        }

        let mut removable: Vec<(u64, u64)> = self
            .audio_players
            .keys()
            .filter(|id| !active_audio_ids.contains(id))
            .map(|id| (*id, *self.audio_last_used.get(id).unwrap_or(&0)))
            .collect();
        removable.sort_by_key(|(_, used)| *used);

        for (clip_id, _) in removable {
            if self.audio_players.len() <= audio_cache_limit {
                break;
            }
            log::debug!("[Preview][Cache] evict audio clip={}", clip_id);
            self.audio_players.remove(&clip_id);
            self.audio_last_used.remove(&clip_id);
            self.audio_paused_state.remove(&clip_id);
            self.last_seek_requests.remove(&clip_id);
        }

        if self.audio_players.len() > audio_cache_limit {
            log::warn!(
                "[Preview][Cache] audio cache remains above limit: size={} limit={} active_audio={}",
                self.audio_players.len(),
                audio_cache_limit,
                active_audio_ids.len()
            );
        }
    }

    /// Bound image cache size while keeping recent clips warm across timeline boundaries.
    fn trim_image_cache(&mut self, active_image_ids: &HashSet<u64>) {
        let image_cache_limit = self.memory_tuning.image_cache_limit;
        if self.image_cache.len() <= image_cache_limit {
            return;
        }

        let mut removable: Vec<(u64, u64)> = self
            .image_cache
            .keys()
            .filter(|id| !active_image_ids.contains(id))
            .map(|id| (*id, *self.image_last_used.get(id).unwrap_or(&0)))
            .collect();
        removable.sort_by_key(|(_, used)| *used);

        for (clip_id, _) in removable {
            if self.image_cache.len() <= image_cache_limit {
                break;
            }
            self.queue_cached_image_drop(clip_id);
            self.image_cache.remove(&clip_id);
            self.image_cache_paths.remove(&clip_id);
            self.image_last_used.remove(&clip_id);
        }

        if self.image_cache.len() > image_cache_limit {
            log::warn!(
                "[Preview][Cache] image cache remains above limit: size={} limit={} active_image={}",
                self.image_cache.len(),
                image_cache_limit,
                active_image_ids.len()
            );
        }
    }

    fn estimate_visual_player_bytes(&self, clip_id: u64, player: &Video) -> usize {
        let (w_raw, h_raw) = player.size();
        let width = w_raw.max(1) as usize;
        let height = h_raw.max(1) as usize;
        #[cfg(target_os = "macos")]
        let bytes_per_pixel = if self
            .mac_surface_mode_keys
            .get(&clip_id)
            .copied()
            .unwrap_or(true)
        {
            2usize
        } else {
            4usize
        };
        #[cfg(not(target_os = "macos"))]
        let bytes_per_pixel = 4usize;

        // Keep a small cushion for decoder/internal queueing beyond app-level frame buffer.
        let buffered_frames = self
            .memory_tuning
            .frame_buffer_capacity
            .max(self.memory_tuning.appsink_max_buffers as usize)
            .max(1)
            .saturating_add(2);

        width
            .saturating_mul(height)
            .saturating_mul(bytes_per_pixel)
            .saturating_mul(buffered_frames)
    }

    fn estimated_cache_bytes(&self) -> usize {
        let image_bytes: usize = self
            .image_cache
            .values()
            .map(|cache| cache.base_data.len())
            .sum();
        let visual_bytes: usize = self
            .visual_players
            .iter()
            .map(|(clip_id, player)| self.estimate_visual_player_bytes(*clip_id, player))
            .sum();
        let audio_bytes = self
            .audio_players
            .len()
            .saturating_mul(self.memory_tuning.estimated_audio_player_bytes);
        image_bytes
            .saturating_add(visual_bytes)
            .saturating_add(audio_bytes)
    }

    fn enforce_memory_budget(
        &mut self,
        active_video_ids: &HashSet<u64>,
        active_audio_ids: &HashSet<u64>,
        active_image_ids: &HashSet<u64>,
    ) {
        if !self.memory_tuning.enforce_budget {
            return;
        }
        let budget = self.memory_tuning.budget_bytes;
        let mut estimated = self.estimated_cache_bytes();
        if estimated <= budget {
            return;
        }

        log::warn!(
            "[Preview][Memory] over budget estimated_mb={:.1} budget_mb={} (evicting inactive caches)",
            estimated as f64 / (1024.0 * 1024.0),
            self.memory_tuning.budget_mb
        );

        let mut image_candidates: Vec<(u64, u64)> = self
            .image_cache
            .keys()
            .filter(|id| !active_image_ids.contains(id))
            .map(|id| (*id, *self.image_last_used.get(id).unwrap_or(&0)))
            .collect();
        image_candidates.sort_by_key(|(_, used)| *used);
        for (clip_id, _) in image_candidates {
            if estimated <= budget {
                break;
            }
            self.queue_cached_image_drop(clip_id);
            self.image_cache.remove(&clip_id);
            self.image_cache_paths.remove(&clip_id);
            self.image_last_used.remove(&clip_id);
            estimated = self.estimated_cache_bytes();
        }

        let mut visual_candidates: Vec<(u64, u64)> = self
            .visual_players
            .keys()
            .filter(|id| !active_video_ids.contains(id))
            .map(|id| (*id, *self.visual_last_used.get(id).unwrap_or(&0)))
            .collect();
        visual_candidates.sort_by_key(|(_, used)| *used);
        for (clip_id, _) in visual_candidates {
            if estimated <= budget {
                break;
            }
            self.visual_players.remove(&clip_id);
            self.video_cache_paths.remove(&clip_id);
            self.visual_last_used.remove(&clip_id);
            self.visual_paused_state.remove(&clip_id);
            self.last_seek_requests.remove(&clip_id);
            self.video_blur_keys.remove(&clip_id);
            #[cfg(target_os = "macos")]
            self.mac_video_effect_keys.remove(&clip_id);
            #[cfg(target_os = "macos")]
            self.mac_surface_mode_keys.remove(&clip_id);
            estimated = self.estimated_cache_bytes();
        }

        let mut audio_candidates: Vec<(u64, u64)> = self
            .audio_players
            .keys()
            .filter(|id| !active_audio_ids.contains(id))
            .map(|id| (*id, *self.audio_last_used.get(id).unwrap_or(&0)))
            .collect();
        audio_candidates.sort_by_key(|(_, used)| *used);
        for (clip_id, _) in audio_candidates {
            if estimated <= budget {
                break;
            }
            self.audio_players.remove(&clip_id);
            self.audio_last_used.remove(&clip_id);
            self.audio_paused_state.remove(&clip_id);
            self.last_seek_requests.remove(&clip_id);
            estimated = self.estimated_cache_bytes();
        }

        if estimated > budget {
            log::warn!(
                "[Preview][Memory] budget still exceeded estimated_mb={:.1} budget_mb={} (active caches dominate)",
                estimated as f64 / (1024.0 * 1024.0),
                self.memory_tuning.budget_mb
            );
        }
    }

    /// Emit cache growth milestones so RAM buildup can be correlated with cache-map sizes.
    fn log_cache_growth_if_needed(&mut self) {
        let visual_len = self.visual_players.len();
        let audio_len = self.audio_players.len();
        let image_len = self.image_cache.len();
        let seek_len = self.last_seek_requests.len();

        if visual_len > self.max_visual_cached_seen {
            self.max_visual_cached_seen = visual_len;
            log::debug!(
                "[Preview][CacheGrowth] visual_cached={} limit={}",
                visual_len,
                self.memory_tuning.visual_cache_limit
            );
        }
        if audio_len > self.max_audio_cached_seen {
            self.max_audio_cached_seen = audio_len;
            log::debug!("[Preview][CacheGrowth] audio_cached={}", audio_len);
        }
        if image_len > self.max_image_cached_seen {
            self.max_image_cached_seen = image_len;
            log::debug!(
                "[Preview][CacheGrowth] image_cached={} limit={}",
                image_len,
                self.memory_tuning.image_cache_limit
            );
        }
        if seek_len > self.max_seek_entries_seen {
            self.max_seek_entries_seen = seek_len;
            log::debug!("[Preview][CacheGrowth] seek_entries={}", seek_len);
        }
    }

    // Build (or reuse) a cached GPUI image texture for the current color controls.
    // fn image_render_for_clip(
    //     &mut self,
    //     clip_id: u64,
    //     brightness: f32,
    //     contrast: f32,
    //     saturation: f32,
    // ) -> Option<(Arc<RenderImage>, u32, u32)> {
    //     let cache = self.image_cache.get_mut(&clip_id)?;
    //     let key = ImageRenderKey::from_values(brightness, contrast, saturation);
    //     if cache.render_key == Some(key)
    //         && let Some(image) = cache.render_image.as_ref()
    //     {
    //         return Some((image.clone(), cache.width, cache.height));
    //     }

    //     let mut data = cache.base_data.clone();
    //     Self::apply_color_correction(&mut data, brightness, contrast, saturation, 1.0);
    //     let buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(cache.width, cache.height, data)?;
    //     let frames = SmallVec::from_elem(image::Frame::new(buffer), 1);
    //     let image = Arc::new(RenderImage::new(frames));
    //     cache.render_image = Some(image.clone());
    //     cache.render_key = Some(key);
    //     Some((image, cache.width, cache.height))
    // }
    // ======
    /// Keep image effect outputs warm and cached. On macOS we materialize both
    /// `RenderImage` and `CVPixelBuffer` so the preview can switch between BGRA
    /// and surface paths without re-running effects.
    fn ensure_image_render_cache(
        &mut self,
        clip_id: u64,
        brightness: f32,
        contrast: f32,
        saturation: f32,
        lut_mix: f32,
        blur_sigma: f32,
        tint_hue: f32,
        tint_saturation: f32,
        tint_lightness: f32,
        tint_alpha: f32,
        fast_blur_mode: bool,
        #[cfg(target_os = "macos")] require_surface: bool,
        cx: &mut Context<Self>,
    ) -> Option<()> {
        let effective_blur_sigma = if fast_blur_mode {
            (blur_sigma * 2.0).round() * 0.5
        } else {
            blur_sigma
        };
        let key = ImageRenderKey::from_values(
            brightness,
            contrast,
            saturation,
            lut_mix,
            effective_blur_sigma,
            tint_hue,
            tint_saturation,
            tint_lightness,
            tint_alpha,
            fast_blur_mode,
        );
        #[cfg(target_os = "macos")]
        let has_matching_surface = self
            .mac_image_surfaces
            .get(&clip_id)
            .map(|(surface_key, _)| *surface_key == key)
            .unwrap_or(false);
        let cache = self.image_cache.get_mut(&clip_id)?;

        // 1. Cache hit: return immediately without any work.
        if cache.render_key == Some(key) && cache.render_image.is_some() {
            #[cfg(target_os = "macos")]
            if !require_surface || cache.has_transparency || has_matching_surface {
                return Some(());
            }
            #[cfg(not(target_os = "macos"))]
            return Some(());
        }

        // 2. Check if all effects are at default (no texture rebuild needed).
        let is_default = brightness.abs() < 0.01
            && (contrast - 1.0).abs() < 0.01
            && (saturation - 1.0).abs() < 0.01
            && lut_mix.abs() < 0.01
            && effective_blur_sigma.abs() < 0.01
            && tint_hue.abs() < 0.01
            && tint_saturation.abs() < 0.01
            && tint_lightness.abs() < 0.01
            && tint_alpha.abs() < 0.01;

        // 3. Fast path: effects are at default — use clean base_data image.
        //    When transitioning from non-default → default (e.g. Layer FX dragged
        //    away), the cached render_image may still contain the old effect, so
        //    we must rebuild from base_data in that case.
        if is_default {
            let data = cache.base_data.clone();
            let width = cache.width;
            let height = cache.height;
            let has_transparency = cache.has_transparency;
            let image = Self::build_render_image_from_bgra(data.clone(), width, height)?;
            cache.render_image = Some(image.clone());
            cache.render_width = width;
            cache.render_height = height;
            cache.render_key = Some(key);
            #[cfg(target_os = "macos")]
            {
                if has_transparency {
                    self.mac_image_surfaces.remove(&clip_id);
                } else if let Some(surface) = Self::build_surface_from_bgra(&data, width, height) {
                    self.mac_image_surfaces.insert(clip_id, (key, surface));
                } else {
                    self.mac_image_surfaces.remove(&clip_id);
                }
            }
            return Some(());
        }

        // 4. Effects changed: spawn async processing if not already in flight for this key.
        //    Return stale cached image immediately so UI stays responsive.
        if cache.pending_effect_key != Some(key) {
            cache.pending_effect_key = Some(key);
            let base_data = cache.base_data.clone();
            let width = cache.width;
            let height = cache.height;
            let target_clip_id = clip_id;

            cx.spawn(async move |view, cx| {
                // Run effect processing via WGPU GPU compute in background thread.
                // Falls back to CPU only when GPU is unavailable (safe mode).
                let result = cx
                    .background_spawn(async move {
                        let mut data = base_data;
                        // Try GPU path first — handles blur, color, and tint in one dispatch.
                        let gpu_ok = process_bgra_effects(
                            &mut data,
                            width,
                            height,
                            brightness,
                            contrast,
                            saturation,
                            lut_mix,
                            0.0,
                            effective_blur_sigma,
                            tint_hue,
                            tint_saturation,
                            tint_lightness,
                            tint_alpha,
                        );
                        if !gpu_ok {
                            // CPU fallback when GPU is not available.
                            if effective_blur_sigma > 0.001 {
                                if fast_blur_mode {
                                    Self::apply_gaussian_blur_fast(
                                        &mut data,
                                        width,
                                        height,
                                        effective_blur_sigma,
                                    );
                                } else {
                                    Self::apply_gaussian_blur(
                                        &mut data,
                                        width,
                                        height,
                                        effective_blur_sigma,
                                    );
                                }
                            } else if effective_blur_sigma < -0.001 {
                                let sharpen_sigma = effective_blur_sigma.abs();
                                Self::apply_preview_sharpen(
                                    &mut data,
                                    width,
                                    height,
                                    sharpen_sigma,
                                    fast_blur_mode,
                                );
                            }
                            Self::apply_color_correction(
                                &mut data, brightness, contrast, saturation, lut_mix, 1.0,
                            );
                        }
                        (data, width, height)
                    })
                    .await;

                let _ = view.update(cx, |this, cx| {
                    let Some(cache) = this.image_cache.get_mut(&target_clip_id) else {
                        return;
                    };
                    // Only apply if this is still the latest requested key (discard stale results).
                    if cache.pending_effect_key != Some(key) {
                        return;
                    }
                    let has_transparency = cache.has_transparency;
                    cache.pending_effect_key = None;
                    let (result, render_width, render_height) = result;
                    if let Some(image) = Self::build_render_image_from_bgra(
                        result.clone(),
                        render_width,
                        render_height,
                    ) {
                        cache.render_image = Some(image);
                        cache.render_width = render_width;
                        cache.render_height = render_height;
                        cache.render_key = Some(key);
                    }
                    #[cfg(target_os = "macos")]
                    {
                        if has_transparency {
                            this.mac_image_surfaces.remove(&target_clip_id);
                        } else if let Some(surface) =
                            Self::build_surface_from_bgra(&result, render_width, render_height)
                        {
                            this.mac_image_surfaces
                                .insert(target_clip_id, (key, surface));
                        } else {
                            this.mac_image_surfaces.remove(&target_clip_id);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }

        // 5. Background work is in flight. If no previous output exists yet,
        // return None and let the first completed frame populate the cache.
        if cache.render_image.is_some() {
            Some(())
        } else {
            None
        }
    }

    /// Non-blocking BGRA image renderer used by Windows/Linux and macOS Full BGRA mode.
    fn image_render_for_clip(
        &mut self,
        clip_id: u64,
        brightness: f32,
        contrast: f32,
        saturation: f32,
        lut_mix: f32,
        blur_sigma: f32,
        tint_hue: f32,
        tint_saturation: f32,
        tint_lightness: f32,
        tint_alpha: f32,
        fast_blur_mode: bool,
        cx: &mut Context<Self>,
    ) -> Option<(Arc<RenderImage>, u32, u32)> {
        self.ensure_image_render_cache(
            clip_id,
            brightness,
            contrast,
            saturation,
            lut_mix,
            blur_sigma,
            tint_hue,
            tint_saturation,
            tint_lightness,
            tint_alpha,
            fast_blur_mode,
            #[cfg(target_os = "macos")]
            false,
            cx,
        )?;
        let cache = self.image_cache.get(&clip_id)?;
        cache
            .render_image
            .as_ref()
            .map(|img| (img.clone(), cache.render_width, cache.render_height))
    }

    #[cfg(target_os = "macos")]
    fn image_surface_for_clip(
        &mut self,
        clip_id: u64,
        brightness: f32,
        contrast: f32,
        saturation: f32,
        lut_mix: f32,
        blur_sigma: f32,
        tint_hue: f32,
        tint_saturation: f32,
        tint_lightness: f32,
        tint_alpha: f32,
        fast_blur_mode: bool,
        cx: &mut Context<Self>,
    ) -> Option<(CVPixelBuffer, u32, u32)> {
        self.ensure_image_render_cache(
            clip_id,
            brightness,
            contrast,
            saturation,
            lut_mix,
            blur_sigma,
            tint_hue,
            tint_saturation,
            tint_lightness,
            tint_alpha,
            fast_blur_mode,
            true,
            cx,
        )?;
        let cache = self.image_cache.get(&clip_id)?;
        self.mac_image_surfaces
            .get(&clip_id)
            .as_ref()
            .and_then(|(surface_key, surface)| {
                if *surface_key == cache.render_key? {
                    Some((surface.clone(), cache.render_width, cache.render_height))
                } else {
                    None
                }
            })
    }

    #[cfg(target_os = "macos")]
    fn prefer_surface_for_image_clip(&self, clip_id: u64, cx: &mut Context<Self>) -> bool {
        let gs = self.global.read(cx);
        gs.mac_preview_render_mode != MacPreviewRenderMode::FullBgra
            && self
                .image_cache
                .get(&clip_id)
                .map(|cache| !cache.has_transparency)
                .unwrap_or(false)
    }

    // ======

    /// Pause/unpause cached visual players by current active ids while avoiding redundant state flips.
    fn update_visual_player_transport(&mut self, active_visual_ids: &HashSet<u64>) {
        for (clip_id, player) in &self.visual_players {
            let should_pause = !active_visual_ids.contains(clip_id);
            let state_changed = self
                .visual_paused_state
                .get(clip_id)
                .is_none_or(|paused| *paused != should_pause);
            let backend_paused = player.paused();
            let backend_mismatch = backend_paused != should_pause;
            if state_changed || backend_mismatch {
                player.set_paused(should_pause);
                self.visual_paused_state.insert(*clip_id, should_pause);
                if backend_mismatch {
                    let (current_state, pending_state) = player.state_debug();
                    let pos_s = player.position().as_secs_f64();
                    log::warn!(
                        "[Preview][Transport] visual pause-state mismatch clip={} desired_paused={} backend_paused={} current_state={} pending_state={} pos={:.3}s",
                        clip_id,
                        should_pause,
                        backend_paused,
                        current_state,
                        pending_state,
                        pos_s
                    );
                } else {
                    log::debug!(
                        "[Preview][Transport] visual clip={} paused={}",
                        clip_id,
                        should_pause
                    );
                }
            }
            if !player.muted() {
                player.set_muted(true);
            }
        }
    }

    /// Pause/unpause audio players by current active ids while avoiding redundant state flips.
    fn update_audio_player_transport(&mut self, active_audio_ids: &HashSet<u64>) {
        for (clip_id, player) in &self.audio_players {
            let should_pause = !active_audio_ids.contains(clip_id);
            let desired_state_changed = self
                .audio_paused_state
                .get(clip_id)
                .is_none_or(|paused| *paused != should_pause);
            // Backend pipeline state may drift from our cached paused map when rapid toggles happen.
            // Reconcile against actual player state so audio never gets stuck muted/paused.
            let backend_paused = player.paused();
            let backend_mismatch = backend_paused != should_pause;
            if desired_state_changed || backend_mismatch {
                player.set_paused(should_pause);
                self.audio_paused_state.insert(*clip_id, should_pause);
                if backend_mismatch {
                    let (current_state, pending_state) = player.state_debug();
                    let muted = player.muted();
                    let pos_s = player.position().as_secs_f64();
                    log::warn!(
                        "[Preview][Transport] audio pause-state mismatch clip={} desired_paused={} backend_paused={} current_state={} pending_state={} muted={} pos={:.3}s",
                        clip_id,
                        should_pause,
                        backend_paused,
                        current_state,
                        pending_state,
                        muted,
                        pos_s
                    );
                } else {
                    log::debug!(
                        "[Preview][Transport] audio clip={} paused={}",
                        clip_id,
                        should_pause
                    );
                }
            }
            if !should_pause && player.muted() {
                player.set_muted(false);
            }
        }
    }

    /// Print active clip transitions so boundary behavior can be correlated with freeze reports.
    fn log_active_set_transition(
        &mut self,
        playhead: Duration,
        active_visual_ids: &HashSet<u64>,
        active_audio_ids: &HashSet<u64>,
    ) {
        if self.last_active_visual_ids == *active_visual_ids
            && self.last_active_audio_ids == *active_audio_ids
        {
            return;
        }
        self.last_active_visual_ids = active_visual_ids.clone();
        self.last_active_audio_ids = active_audio_ids.clone();

        let mut visual_sorted: Vec<u64> = active_visual_ids.iter().copied().collect();
        let mut audio_sorted: Vec<u64> = active_audio_ids.iter().copied().collect();
        visual_sorted.sort_unstable();
        audio_sorted.sort_unstable();

        log::debug!(
            "[Preview][Boundary] playhead={:.3}s visual={:?} audio={:?}",
            playhead.as_secs_f64(),
            visual_sorted,
            audio_sorted
        );
    }

    /// Emit periodic runtime diagnostics so hangs can be correlated with cache/player state in terminal logs.
    fn maybe_log_runtime_state(
        &mut self,
        playhead: Duration,
        active_visual_count: usize,
        active_audio_count: usize,
    ) {
        let now = Instant::now();
        if self
            .last_debug_log_at
            .is_some_and(|last| now.saturating_duration_since(last) < Duration::from_secs(1))
        {
            return;
        }
        self.last_debug_log_at = Some(now);
        let image_bytes: usize = self.image_cache.values().map(|c| c.base_data.len()).sum();
        let estimated_cache_bytes = self.estimated_cache_bytes();
        let budget_label = if self.memory_tuning.enforce_budget {
            format!(
                "{:.1}",
                self.memory_tuning.budget_bytes as f64 / (1024.0 * 1024.0)
            )
        } else {
            "none".to_string()
        };
        log::debug!(
            "[Preview][Heartbeat] playhead={:.3}s active_visual={} cached_visual={} active_audio={} cached_audio={} cached_images={} image_mb={:.2} est_cache_mb={:.1}/{}",
            playhead.as_secs_f64(),
            active_visual_count,
            self.visual_players.len(),
            active_audio_count,
            self.audio_players.len(),
            self.image_cache.len(),
            image_bytes as f64 / (1024.0 * 1024.0),
            estimated_cache_bytes as f64 / (1024.0 * 1024.0),
            budget_label,
        );
    }

    fn sequence_total(gs: &GlobalState) -> Duration {
        let v1_end = gs
            .v1_clips
            .iter()
            .map(|c| c.start + c.duration)
            .max()
            .unwrap_or(Duration::ZERO);
        let audio_end = gs
            .audio_tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.start + c.duration)
            .max()
            .unwrap_or(Duration::ZERO);
        let video_overlay_end = gs
            .video_tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.start + c.duration)
            .max()
            .unwrap_or(Duration::ZERO);
        let subtitle_end = gs
            .subtitle_tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.start + c.duration)
            .max()
            .unwrap_or(Duration::ZERO);
        v1_end
            .max(audio_end)
            .max(video_overlay_end)
            .max(subtitle_end)
    }

    // [Helper] Read transform values from a clip.
    fn get_clip_transform(
        gs: &GlobalState,
        clip_id: u64,
        playhead: Duration,
    ) -> Option<(f32, f32, f32, f32)> {
        if let Some(c) = gs.v1_clips.iter().find(|c| c.id == clip_id) {
            let (scale, pos_x, pos_y, rotation_deg) = Self::sample_transform_for_clip(c, playhead);
            return Some((scale, pos_x, pos_y, rotation_deg));
        }
        for track in &gs.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == clip_id) {
                let (scale, pos_x, pos_y, rotation_deg) =
                    Self::sample_transform_for_clip(c, playhead);
                return Some((scale, pos_x, pos_y, rotation_deg));
            }
        }
        None
    }

    fn sample_transform_for_clip(
        clip: &crate::core::global_state::Clip,
        playhead: Duration,
    ) -> (f32, f32, f32, f32) {
        if playhead < clip.start || playhead > clip.end() {
            return (
                clip.get_scale(),
                clip.get_pos_x(),
                clip.get_pos_y(),
                clip.get_rotation(),
            );
        }
        let local = (playhead - clip.start).min(clip.duration);
        let (slide_x, slide_y) = clip.sample_slide_offset(local);
        let zoom = clip.sample_zoom_factor(local);
        let shock_zoom = clip.sample_shock_zoom_factor(local);
        (
            clip.sample_scale(local) * zoom * shock_zoom,
            clip.sample_pos_x(local) + slide_x,
            clip.sample_pos_y(local) + slide_y,
            clip.sample_rotation(local),
        )
    }

    fn get_clip_tint(gs: &GlobalState, clip_id: u64) -> Option<(f32, f32, f32, f32)> {
        let layer_hsla = gs.layer_hsla_overlay_at(gs.playhead);
        if let Some(c) = gs.v1_clips.iter().find(|c| c.id == clip_id) {
            let clip_hsla = c.get_hsla_overlay();
            return Some(Self::blend_hsla_overlay(clip_hsla, layer_hsla));
        }
        for track in &gs.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == clip_id) {
                let clip_hsla = c.get_hsla_overlay();
                return Some(Self::blend_hsla_overlay(clip_hsla, layer_hsla));
            }
        }
        None
    }

    fn blend_hsla_overlay(
        base: (f32, f32, f32, f32),
        layer: (f32, f32, f32, f32),
    ) -> (f32, f32, f32, f32) {
        let (base_h, base_s, base_l, base_a) = base;
        let (layer_h, layer_s, layer_l, layer_a) = layer;
        let layer_a = layer_a.clamp(0.0, 1.0);
        let base_a = base_a.clamp(0.0, 1.0);
        if layer_a <= 0.0 {
            return (base_h, base_s, base_l, base_a);
        }
        if base_a <= 0.0 {
            return (layer_h, layer_s, layer_l, layer_a);
        }
        let w = layer_a;
        let out_h = (base_h * (1.0 - w) + layer_h * w).rem_euclid(360.0);
        let out_s = (base_s * (1.0 - w) + layer_s * w).clamp(0.0, 1.0);
        let out_l = (base_l * (1.0 - w) + layer_l * w).clamp(0.0, 1.0);
        let out_a = 1.0 - (1.0 - base_a) * (1.0 - layer_a);
        (out_h, out_s, out_l, out_a.clamp(0.0, 1.0))
    }

    fn get_clip_bcs(gs: &GlobalState, clip_id: u64) -> Option<(f32, f32, f32)> {
        let layer = gs.layer_color_blur_effects_at(gs.playhead);
        if let Some(c) = gs.v1_clips.iter().find(|c| c.id == clip_id) {
            let effects = Self::sample_color_blur_for_clip(c, gs.playhead, layer);
            return Some((effects.brightness, effects.contrast, effects.saturation));
        }
        for track in &gs.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == clip_id) {
                let effects = Self::sample_color_blur_for_clip(c, gs.playhead, layer);
                return Some((effects.brightness, effects.contrast, effects.saturation));
            }
        }
        None
    }

    fn find_clip_by_id(gs: &GlobalState, clip_id: u64) -> Option<&crate::core::global_state::Clip> {
        if let Some(c) = gs.v1_clips.iter().find(|c| c.id == clip_id) {
            return Some(c);
        }
        for track in &gs.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == clip_id) {
                return Some(c);
            }
        }
        None
    }

    fn clip_source_time_for_preview(
        clip: &crate::core::global_state::Clip,
        t: Duration,
    ) -> Duration {
        let src = if t <= clip.start {
            clip.source_in
        } else if t >= clip.end() {
            clip.source_in.saturating_add(clip.duration)
        } else {
            clip.source_in.saturating_add(t.saturating_sub(clip.start))
        };
        src.min(clip.media_duration)
    }

    fn resolve_layer_transition_pair(gs: &GlobalState, t: Duration) -> Option<(u64, u64)> {
        fn best_pair_in_sequence(
            clips: &[crate::core::global_state::Clip],
            t: Duration,
        ) -> Option<(u64, u64, Duration)> {
            if clips.len() < 2 {
                return None;
            }
            let mut best: Option<(u64, u64, Duration)> = None;
            for pair in clips.windows(2) {
                let left = &pair[0];
                let right = &pair[1];
                let cut = right.start;
                let dist = t.abs_diff(cut);
                if best
                    .as_ref()
                    .is_none_or(|(_, _, best_dist)| dist < *best_dist)
                {
                    best = Some((left.id, right.id, dist));
                }
            }
            best
        }

        let v1_muted = gs.track_mute.get("v1").copied().unwrap_or(false);
        let mut best = if v1_muted {
            None
        } else {
            best_pair_in_sequence(&gs.v1_clips, t)
        };
        for (track_idx, track) in gs.video_tracks.iter().enumerate() {
            let track_key = format!("video:{track_idx}");
            let is_muted = gs.track_mute.get(&track_key).copied().unwrap_or(false);
            if is_muted {
                continue;
            }
            if let Some(candidate) = best_pair_in_sequence(&track.clips, t)
                && best
                    .as_ref()
                    .is_none_or(|(_, _, best_dist)| candidate.2 < *best_dist)
            {
                best = Some(candidate);
            }
        }
        best.map(|(prev, next, _)| (prev, next))
    }

    fn get_clip_opacity(gs: &GlobalState, clip_id: u64) -> Option<f32> {
        let layer_opacity = gs.layer_opacity_factor_at(gs.playhead);
        let layer_transition_mix = gs.layer_transition_dissolve_mix_at(gs.playhead);
        if let Some(c) = gs.v1_clips.iter().find(|c| c.id == clip_id) {
            let factor = Self::v1_dissolve_factor(gs, clip_id, gs.playhead).unwrap_or(1.0);
            let local = if gs.playhead <= c.start {
                Duration::ZERO
            } else if gs.playhead >= c.end() {
                c.duration
            } else {
                gs.playhead - c.start
            };
            let base = c.sample_opacity(local);
            let fade = c.sample_fade_factor(local);
            let mut out = base * fade * factor * layer_opacity;
            if let Some(mix) = layer_transition_mix
                && let Some((_, next_id)) = Self::resolve_layer_transition_pair(gs, gs.playhead)
                && clip_id == next_id
            {
                out *= mix.clamp(0.0, 1.0);
            }
            return Some(out);
        }
        for track in &gs.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == clip_id) {
                let val = if gs.playhead < c.start || gs.playhead > c.end() {
                    c.get_opacity()
                } else {
                    let local = (gs.playhead - c.start).min(c.duration);
                    let base = c.sample_opacity(local);
                    base * c.sample_fade_factor(local) * c.sample_dissolve_factor(local)
                };
                let mut out = val * layer_opacity;
                if let Some(mix) = layer_transition_mix
                    && let Some((_, next_id)) = Self::resolve_layer_transition_pair(gs, gs.playhead)
                    && clip_id == next_id
                {
                    out *= mix.clamp(0.0, 1.0);
                }
                return Some(out);
            }
        }
        None
    }

    fn get_clip_blur_sigma(gs: &GlobalState, clip_id: u64) -> Option<f32> {
        let layer = gs.layer_color_blur_effects_at(gs.playhead);
        if let Some(c) = gs.v1_clips.iter().find(|c| c.id == clip_id) {
            let effects = Self::sample_color_blur_for_clip(c, gs.playhead, layer);
            return Some(effects.blur_sigma);
        }
        for track in &gs.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == clip_id) {
                let effects = Self::sample_color_blur_for_clip(c, gs.playhead, layer);
                return Some(effects.blur_sigma);
            }
        }
        None
    }

    fn get_clip_lut_mix(gs: &GlobalState, clip_id: u64) -> Option<f32> {
        let layer_mix = gs.layer_lut_mix_at(gs.playhead).clamp(0.0, 1.0);
        if gs.v1_clips.iter().any(|c| c.id == clip_id) {
            return Some(layer_mix);
        }
        for track in &gs.video_tracks {
            if track.clips.iter().any(|c| c.id == clip_id) {
                return Some(layer_mix);
            }
        }
        None
    }

    fn get_clip_local_mask_layers(gs: &GlobalState, clip_id: u64) -> Vec<VideoLocalMaskLayer> {
        let map_layer = |layer: crate::core::global_state::LocalMaskLayer| VideoLocalMaskLayer {
            enabled: layer.enabled,
            center_x: layer.center_x.clamp(0.0, 1.0),
            center_y: layer.center_y.clamp(0.0, 1.0),
            radius: layer.radius.clamp(0.0, 1.0),
            feather: layer.feather.clamp(0.0, 1.0),
            strength: layer.strength.clamp(0.0, 1.0),
            brightness: layer.brightness.clamp(-1.0, 1.0),
            contrast: layer.contrast.clamp(0.0, 2.0),
            saturation: layer.saturation.clamp(0.0, 2.0),
            opacity: layer.opacity.clamp(0.0, 1.0),
            blur_sigma: layer.blur_sigma.clamp(0.0, 64.0),
        };

        let to_renderer_layers = |clip: &crate::core::global_state::Clip| {
            clip.get_local_mask_layers()
                .into_iter()
                .take(VIDEO_MAX_LOCAL_MASK_LAYERS)
                .map(map_layer)
                .collect::<Vec<_>>()
        };

        if let Some(c) = gs.v1_clips.iter().find(|c| c.id == clip_id) {
            return to_renderer_layers(c);
        }
        for track in &gs.video_tracks {
            if let Some(c) = track.clips.iter().find(|c| c.id == clip_id) {
                return to_renderer_layers(c);
            }
        }
        vec![VideoLocalMaskLayer::default()]
    }

    fn v1_dissolve_factor(gs: &GlobalState, clip_id: u64, t: Duration) -> Option<f32> {
        if gs.v1_clips.len() < 2 {
            return None;
        }
        let t_sec = t.as_secs_f32();
        for idx in 0..(gs.v1_clips.len() - 1) {
            let left = &gs.v1_clips[idx];
            let right = &gs.v1_clips[idx + 1];
            let d = left.get_dissolve_out().min(right.get_dissolve_in());
            if d <= 0.001 {
                continue;
            }
            let half = d * 0.5;
            let cut = right.start.as_secs_f32();
            if t_sec < (cut - half) || t_sec > (cut + half) {
                continue;
            }
            let mut alpha_right = ((t_sec - (cut - half)) / d).clamp(0.0, 1.0);
            alpha_right = alpha_right.powf(0.4545);
            if clip_id == left.id {
                return Some(1.0);
            }
            if clip_id == right.id {
                return Some(alpha_right);
            }
            return None;
        }
        None
    }

    fn sample_color_blur_for_clip(
        clip: &crate::core::global_state::Clip,
        playhead: Duration,
        layer_effects: crate::core::effects::LayerColorBlurEffects,
    ) -> PerClipColorBlurEffects {
        let clip_effects = if playhead < clip.start || playhead > clip.end() {
            clip.base_color_blur_effects()
        } else {
            let local = (playhead - clip.start).min(clip.duration);
            PerClipColorBlurEffects {
                brightness: clip.sample_brightness(local),
                contrast: clip.sample_contrast(local),
                saturation: clip.sample_saturation(local),
                blur_sigma: clip.sample_blur(local),
            }
            .normalized()
        };
        combine_clip_with_layer(clip_effects, layer_effects)
    }

    fn resolve_visible_subtitles(gs: &GlobalState, t: Duration) -> Vec<SubtitleClip> {
        let mut visible = Vec::new();
        for (track_idx, track) in gs.subtitle_tracks.iter().enumerate() {
            let track_key = format!("subtitle:{track_idx}");
            let is_muted = gs.track_mute.get(&track_key).copied().unwrap_or(false);
            if is_muted {
                continue;
            }
            for clip in &track.clips {
                if t >= clip.start && t < (clip.start + clip.duration) {
                    visible.push(clip.clone());
                }
            }
        }
        visible
    }

    fn resolve_visible_clips(
        gs: &GlobalState,
        t: Duration,
    ) -> Vec<(u64, String, Duration, f32, f32, f32, f32)> {
        let layer_effects = gs.layer_color_blur_effects_at(t);
        let layer_transition_mix = gs.layer_transition_dissolve_mix_at(t);
        let mut visible_clips = Vec::new();
        let v1_muted = gs.track_mute.get("v1").copied().unwrap_or(false);
        // V1 (with virtual dissolve overlap)
        if !v1_muted {
            let mut v1_handled = false;
            if gs.v1_clips.len() >= 2 {
                let t_sec = t.as_secs_f32();
                for idx in 0..(gs.v1_clips.len() - 1) {
                    let left = &gs.v1_clips[idx];
                    let right = &gs.v1_clips[idx + 1];
                    let d = left.get_dissolve_out().min(right.get_dissolve_in());
                    if d <= 0.001 {
                        continue;
                    }
                    let half = d * 0.5;
                    let cut = right.start.as_secs_f32();
                    if t_sec < (cut - half) || t_sec > (cut + half) {
                        continue;
                    }

                    let left_start = left.start.as_secs_f32();
                    let left_src_in = left.source_in.as_secs_f32();
                    let left_dur = left.duration.as_secs_f32();
                    let right_start = right.start.as_secs_f32();
                    let right_src_in = right.source_in.as_secs_f32();

                    let left_src_sec = if t_sec <= cut {
                        left_src_in + (t_sec - left_start)
                    } else {
                        left_src_in + left_dur + (t_sec - cut)
                    };
                    let right_src_sec = right_src_in + (t_sec - right_start);

                    let left_src_sec = left_src_sec.clamp(0.0, left.media_duration.as_secs_f32());
                    let right_src_sec =
                        right_src_sec.clamp(0.0, right.media_duration.as_secs_f32());
                    let left_effects = Self::sample_color_blur_for_clip(left, t, layer_effects);
                    let right_effects = Self::sample_color_blur_for_clip(right, t, layer_effects);

                    visible_clips.push((
                        left.id,
                        left.file_path.clone(),
                        Duration::from_secs_f32(left_src_sec),
                        left_effects.brightness,
                        left_effects.contrast,
                        left_effects.saturation,
                        left_effects.blur_sigma,
                    ));
                    visible_clips.push((
                        right.id,
                        right.file_path.clone(),
                        Duration::from_secs_f32(right_src_sec),
                        right_effects.brightness,
                        right_effects.contrast,
                        right_effects.saturation,
                        right_effects.blur_sigma,
                    ));
                    v1_handled = true;
                    break;
                }
            }
            if !v1_handled {
                for clip in &gs.v1_clips {
                    if t >= clip.start && t < (clip.start + clip.duration) {
                        let src_time = clip.source_in + (t - clip.start);
                        let effects = Self::sample_color_blur_for_clip(clip, t, layer_effects);
                        visible_clips.push((
                            clip.id,
                            clip.file_path.clone(),
                            src_time,
                            effects.brightness,
                            effects.contrast,
                            effects.saturation,
                            effects.blur_sigma,
                        ));
                        break;
                    }
                }
            }
        }
        // V2+
        for (track_idx, track) in gs.video_tracks.iter().enumerate() {
            let track_key = format!("video:{track_idx}");
            let is_muted = gs.track_mute.get(&track_key).copied().unwrap_or(false);
            if is_muted {
                continue;
            }
            for clip in &track.clips {
                if t >= clip.start && t < (clip.start + clip.duration) {
                    let src_time = clip.source_in + (t - clip.start);
                    let effects = Self::sample_color_blur_for_clip(clip, t, layer_effects);
                    visible_clips.push((
                        clip.id,
                        clip.file_path.clone(),
                        src_time,
                        effects.brightness,
                        effects.contrast,
                        effects.saturation,
                        effects.blur_sigma,
                    ));
                }
            }
        }

        // v1 layer-transition dissolve: ensure prev/next pair is present so preview can do true A/B blend.
        if layer_transition_mix.is_some()
            && let Some((prev_id, next_id)) = Self::resolve_layer_transition_pair(gs, t)
        {
            let mut append_entry = |clip_id: u64| {
                if visible_clips.iter().any(|(id, ..)| *id == clip_id) {
                    return;
                }
                let Some(clip) = Self::find_clip_by_id(gs, clip_id) else {
                    return;
                };
                let src_time = Self::clip_source_time_for_preview(clip, t);
                let effects = Self::sample_color_blur_for_clip(clip, t, layer_effects);
                visible_clips.push((
                    clip.id,
                    clip.file_path.clone(),
                    src_time,
                    effects.brightness,
                    effects.contrast,
                    effects.saturation,
                    effects.blur_sigma,
                ));
            };
            append_entry(prev_id);
            append_entry(next_id);

            let prev_pos = visible_clips.iter().position(|(id, ..)| *id == prev_id);
            let next_pos = visible_clips.iter().position(|(id, ..)| *id == next_id);
            if let (Some(p), Some(n)) = (prev_pos, next_pos)
                && p > n
            {
                visible_clips.swap(p, n);
            }
        }
        visible_clips
    }

    fn db_to_linear(gain_db: f32) -> f64 {
        10.0_f64.powf((gain_db as f64) / 20.0)
    }

    fn duration_to_ns(value: Duration) -> u64 {
        // Keep index math in u64 nanoseconds and saturate to avoid overflow.
        value
            .as_nanos()
            .min(u128::from(u64::MAX))
            .try_into()
            .unwrap_or(u64::MAX)
    }

    fn build_audio_track_time_index(track: &AudioTrack) -> AudioTrackTimeIndex {
        // Build monotonic arrays once, then reuse binary search for active/prewarm lookups.
        let mut starts_ns = Vec::with_capacity(track.clips.len());
        let mut prefix_max_end_ns = Vec::with_capacity(track.clips.len());
        let mut running_max_end_ns = 0_u64;

        for clip in &track.clips {
            let start_ns = Self::duration_to_ns(clip.start);
            let end_ns = Self::duration_to_ns(clip.start.saturating_add(clip.duration));
            starts_ns.push(start_ns);
            running_max_end_ns = running_max_end_ns.max(end_ns);
            prefix_max_end_ns.push(running_max_end_ns);
        }

        let first_clip = track.clips.first();
        let last_clip = track.clips.last();

        AudioTrackTimeIndex {
            starts_ns,
            prefix_max_end_ns,
            clip_count: track.clips.len(),
            first_clip_id: first_clip.map(|clip| clip.id).unwrap_or(0),
            first_start_ns: first_clip
                .map(|clip| Self::duration_to_ns(clip.start))
                .unwrap_or(0),
            first_duration_ns: first_clip
                .map(|clip| Self::duration_to_ns(clip.duration))
                .unwrap_or(0),
            last_clip_id: last_clip.map(|clip| clip.id).unwrap_or(0),
            last_start_ns: last_clip
                .map(|clip| Self::duration_to_ns(clip.start))
                .unwrap_or(0),
            last_duration_ns: last_clip
                .map(|clip| Self::duration_to_ns(clip.duration))
                .unwrap_or(0),
        }
    }

    fn first_prefix_max_gt(prefix: &[u64], upper_exclusive: usize, t_ns: u64) -> usize {
        // Binary search the first index whose prefix max end is still after playhead.
        let mut lo = 0usize;
        let mut hi = upper_exclusive;
        while lo < hi {
            let mid = lo + ((hi - lo) / 2);
            if prefix[mid] <= t_ns {
                lo = mid.saturating_add(1);
            } else {
                hi = mid;
            }
        }
        lo
    }

    fn collect_indexed_audio_clips(
        audio_track_time_indices: &mut HashMap<usize, AudioTrackTimeIndex>,
        audio_track_index_token: &mut u64,
        gs: &GlobalState,
        t: Duration,
        lookahead: Duration,
    ) -> (
        Vec<(u64, String, Duration, f64)>,
        Vec<(u64, String, Duration)>,
    ) {
        // Invalidate cached per-track indices on timeline edits.
        let edit_token = gs.timeline_edit_token();
        if *audio_track_index_token != edit_token {
            audio_track_time_indices.clear();
            *audio_track_index_token = edit_token;
        }

        let t_ns = Self::duration_to_ns(t);
        let window_end = t.saturating_add(lookahead);
        let window_end_ns = Self::duration_to_ns(window_end);

        let mut active = Vec::new();
        let mut prewarm = Vec::new();

        for (track_idx, track) in gs.audio_tracks.iter().enumerate() {
            let track_key = format!("audio:{track_idx}");
            let is_muted = gs.track_mute.get(&track_key).copied().unwrap_or(false);
            if is_muted {
                continue;
            }
            if track.clips.is_empty() {
                continue;
            }

            let needs_rebuild = audio_track_time_indices
                .get(&track_idx)
                .map(|cached| {
                    let first = track.clips.first();
                    let last = track.clips.last();
                    cached.clip_count != track.clips.len()
                        || cached.first_clip_id != first.map(|clip| clip.id).unwrap_or(0)
                        || cached.first_start_ns
                            != first
                                .map(|clip| Self::duration_to_ns(clip.start))
                                .unwrap_or(0)
                        || cached.first_duration_ns
                            != first
                                .map(|clip| Self::duration_to_ns(clip.duration))
                                .unwrap_or(0)
                        || cached.last_clip_id != last.map(|clip| clip.id).unwrap_or(0)
                        || cached.last_start_ns
                            != last
                                .map(|clip| Self::duration_to_ns(clip.start))
                                .unwrap_or(0)
                        || cached.last_duration_ns
                            != last
                                .map(|clip| Self::duration_to_ns(clip.duration))
                                .unwrap_or(0)
                })
                .unwrap_or(true);
            if needs_rebuild {
                audio_track_time_indices
                    .insert(track_idx, Self::build_audio_track_time_index(track));
            }
            let Some(index) = audio_track_time_indices.get(&track_idx) else {
                continue;
            };

            // Active query: clips with start<=t and end>t.
            let started_upper = index.starts_ns.partition_point(|start| *start <= t_ns);
            if started_upper > 0 {
                let first_candidate =
                    Self::first_prefix_max_gt(&index.prefix_max_end_ns, started_upper, t_ns);
                let track_gain_linear = Self::db_to_linear(track.gain_db);
                for clip in &track.clips[first_candidate..started_upper] {
                    if t >= clip.start && t < clip.start.saturating_add(clip.duration) {
                        let clip_gain_linear = Self::db_to_linear(clip.audio_gain_db);
                        active.push((
                            clip.id,
                            clip.file_path.clone(),
                            clip.source_in.saturating_add(t.saturating_sub(clip.start)),
                            track_gain_linear * clip_gain_linear,
                        ));
                    }
                }
            }

            // Prewarm query: clips whose start lies in (t, t+lookahead].
            if lookahead > Duration::ZERO {
                let prewarm_start = index.starts_ns.partition_point(|start| *start <= t_ns);
                let prewarm_end = index
                    .starts_ns
                    .partition_point(|start| *start <= window_end_ns);
                for clip in &track.clips[prewarm_start..prewarm_end] {
                    if clip.start > t && clip.start <= window_end {
                        // Prewarm always seeks to clip start; active path reseeks to exact local time.
                        prewarm.push((clip.id, clip.file_path.clone(), clip.source_in));
                    }
                }
            }
        }

        (active, prewarm)
    }

    fn sync_video_engine(&mut self, cx: &mut Context<Self>) -> bool {
        let sync_started = Instant::now();
        let (playhead, visual_clips, preview_fps, preview_quality, canvas_w, canvas_h) = {
            let gs = self.global.read(cx);
            (
                gs.playhead,
                Self::resolve_visible_clips(gs, gs.playhead),
                gs.preview_fps.value(),
                gs.preview_quality,
                gs.canvas_w,
                gs.canvas_h,
            )
        };
        let lookahead = Duration::from_millis(AUDIO_PREWARM_LOOKAHEAD_MS);
        let (audio_clips, audio_prewarm_clips) = {
            let gs = self.global.read(cx);
            Self::collect_indexed_audio_clips(
                &mut self.audio_track_time_indices,
                &mut self.audio_track_index_token,
                gs,
                playhead,
                lookahead,
            )
        };

        let mut changed = false;
        let mut active_visual_ids = HashSet::new();
        let mut active_video_ids = HashSet::new();
        let mut active_image_ids = HashSet::new();
        let mut next_order = Vec::new();
        let full_preview_max_dim =
            (canvas_w.max(canvas_h) as u32).clamp(1, DEFAULT_IMAGE_MAX_DIM_FULL);

        if preview_fps != self.last_preview_fps || preview_quality != self.last_preview_quality {
            // Reset decode caches when preview settings change so old resolution/fps resources can be released.
            let cached_image_ids: Vec<u64> = self.image_cache.keys().copied().collect();
            for clip_id in cached_image_ids {
                self.queue_cached_image_drop(clip_id);
            }
            self.visual_players.clear();
            self.visual_order.clear();
            self.last_seek_requests.clear();
            self.video_cache_paths.clear();
            self.visual_last_used.clear();
            self.visual_paused_state.clear();
            self.image_cache.clear();
            self.image_cache_paths.clear();
            self.image_last_used.clear();
            self.image_decode_in_flight.clear();
            self.audio_paused_state.clear();
            self.audio_track_time_indices.clear();
            self.audio_track_index_token = 0;
            self.last_active_visual_ids.clear();
            self.last_active_audio_ids.clear();
            self.last_clip_blur_sigmas.clear();
            self.video_blur_keys.clear();
            self.blur_interaction_until = None;
            #[cfg(target_os = "macos")]
            self.mac_video_effect_keys.clear();
            #[cfg(target_os = "macos")]
            self.mac_surface_mode_keys.clear();
            self.last_preview_fps = preview_fps;
            self.last_preview_quality = preview_quality;
            changed = true;
        }

        // Phase 1: Visual
        for (id, path_str, src_time, b, c, s, blur_sigma) in visual_clips {
            active_visual_ids.insert(id);
            next_order.push(id);
            let prev_blur = self.last_clip_blur_sigmas.get(&id).copied();
            if prev_blur.is_none_or(|prev| (prev - blur_sigma).abs() > 0.0005) {
                self.blur_interaction_until =
                    Some(Instant::now() + Duration::from_millis(FAST_BLUR_SETTLE_MS));
            }
            self.last_clip_blur_sigmas.insert(id, blur_sigma);

            if is_image_ext(&path_str) {
                active_image_ids.insert(id);
                Self::touch_cache(&mut self.image_last_used, &mut self.cache_touch_counter, id);
                let image_target_max_dim =
                    Some(preview_quality.max_dim().unwrap_or(full_preview_max_dim));
                let needs_reload = (self.image_cache_paths.get(&id) != Some(&path_str))
                    || !self.image_cache.contains_key(&id);
                if needs_reload && !self.image_decode_in_flight.contains(&id) {
                    self.queue_cached_image_drop(id);
                    self.image_cache.remove(&id);
                    self.image_cache_paths.remove(&id);
                    self.image_decode_in_flight.insert(id);

                    let decode_path = path_str.clone();
                    let cache_path = path_str.clone();
                    let max_dim = image_target_max_dim;
                    let pixelate = preview_quality.pixelate();
                    let clip_id = id;
                    cx.spawn(async move |view, cx| {
                        let result = cx
                            .background_spawn(async move {
                                Self::decode_image_cache(&decode_path, max_dim, pixelate)
                            })
                            .await;

                        let _ = view.update(cx, |this, cx| {
                            // If clip was removed from in-flight (e.g. settings change),
                            // discard the stale result.
                            if !this.image_decode_in_flight.remove(&clip_id) {
                                return;
                            }
                            if let Some(cache) = result {
                                log::info!(
                                    "[Preview] image cache load (async) clip={} path={} size={}x{} bytes={}",
                                    clip_id,
                                    cache_path,
                                    cache.width,
                                    cache.height,
                                    cache.base_data.len()
                                );
                                this.image_cache.insert(clip_id, cache);
                                this.image_cache_paths.insert(clip_id, cache_path);
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                }
                continue;
            }

            active_video_ids.insert(id);
            Self::touch_cache(
                &mut self.visual_last_used,
                &mut self.cache_touch_counter,
                id,
            );
            let mut clip_path = path_str.clone();
            let requested_max_dim = preview_quality.max_dim();
            if let Some(max_dim) = requested_max_dim {
                let lookup = self
                    .global
                    .update(cx, |gs, _| gs.lookup_proxy_for_path(&path_str, max_dim));
                let _proxy_status = lookup.status;
                if let Some(proxy_path) = lookup.path {
                    clip_path = proxy_path;
                }
            }
            let use_proxy = clip_path != path_str;
            #[cfg(target_os = "macos")]
            let (clip_opacity, clip_lut_mix, target_prefer_surface) = {
                let gs = self.global.read(cx);
                let opacity = Self::get_clip_opacity(gs, id).unwrap_or(1.0);
                let lut_mix = Self::get_clip_lut_mix(gs, id).unwrap_or(0.0);
                let (transform_scale, pos_x, pos_y, rotation_deg) =
                    Self::get_clip_transform(gs, id, playhead).unwrap_or((1.0, 0.0, 0.0, 0.0));
                let local_mask_layers = Self::get_clip_local_mask_layers(gs, id);
                let manual_surface_mode =
                    if gs.mac_preview_render_mode == MacPreviewRenderMode::FullBgra {
                        false
                    } else if use_proxy {
                        gs.proxy_render_mode_for_quality(preview_quality)
                            .prefer_surface()
                    } else {
                        true
                    };
                // Opacity is supported on the NV12 surface path via `paint_surface_anica`
                // parameters; keep BGRA fallback only for features not yet supported there.
                let force_bgra_for_transform = (transform_scale - 1.0).abs() > 0.001
                    || pos_x.abs() > 0.001
                    || pos_y.abs() > 0.001
                    || rotation_deg.abs() > 0.001;
                let force_bgra_for_local_mask = local_mask_layers.iter().any(|layer| {
                    let has_shape = layer.enabled
                        && layer.strength >= 0.001
                        && layer.radius >= 0.0001
                        && (layer.feather >= 0.0001 || layer.radius > 0.0001);
                    let has_color = layer.brightness.abs() > 0.001
                        || (layer.contrast - 1.0).abs() > 0.001
                        || (layer.saturation - 1.0).abs() > 0.001
                        || (layer.opacity - 1.0).abs() > 0.001;
                    let has_blur = layer.blur_sigma.abs() > 0.001;
                    has_shape && (has_color || has_blur)
                });
                (
                    opacity,
                    lut_mix,
                    manual_surface_mode && !force_bgra_for_transform && !force_bgra_for_local_mask,
                )
            };
            let path_changed = self.video_cache_paths.get(&id) != Some(&clip_path);
            let mut needs_reload = path_changed;
            #[cfg(target_os = "macos")]
            {
                if self.visual_players.contains_key(&id)
                    && self.mac_surface_mode_keys.get(&id).copied() != Some(target_prefer_surface)
                {
                    needs_reload = true;
                }
            }

            if needs_reload {
                self.visual_players.remove(&id);
                self.last_seek_requests.remove(&id);
                self.visual_paused_state.remove(&id);
                self.video_blur_keys.remove(&id);
                #[cfg(target_os = "macos")]
                self.mac_video_effect_keys.remove(&id);
                #[cfg(target_os = "macos")]
                self.mac_surface_mode_keys.remove(&id);
                changed = true;
            }

            if !self.visual_players.contains_key(&id) {
                let path = PathBuf::from(&clip_path);
                if let Ok(url) = Url::from_file_path(&path) {
                    let preview_scale = None;
                    let preview_max_dim = if use_proxy { None } else { requested_max_dim };
                    #[cfg(target_os = "macos")]
                    let prefer_surface = target_prefer_surface;
                    #[cfg(not(target_os = "macos"))]
                    let prefer_surface = true;
                    #[cfg(target_os = "macos")]
                    let strict_surface_proxy_nv12 = use_proxy && prefer_surface;
                    #[cfg(not(target_os = "macos"))]
                    let strict_surface_proxy_nv12 = false;
                    let opts = VideoOptions {
                        frame_buffer_capacity: Some(self.memory_tuning.frame_buffer_capacity),
                        preview_scale,
                        preview_max_dim,
                        preview_fps: Some(preview_fps),
                        appsink_max_buffers: Some(self.memory_tuning.appsink_max_buffers),
                        prefer_surface,
                        strict_surface_proxy_nv12,
                        benchmark_raw_appsink: VideoOptions::benchmark_raw_appsink_from_env(),
                        ..Default::default()
                    };
                    if let Ok(video) = Video::new_with_options(&url, opts) {
                        log::info!(
                            "[Preview] create visual player clip={} path={} proxy={} fps={}",
                            id,
                            clip_path,
                            use_proxy,
                            preview_fps
                        );
                        // Keep visual decode separate from audio routing to avoid ghost audio after clip unlink/delete.
                        // Boot in paused state first; transport pass below decides whether to unpause.
                        video.set_paused(true);
                        video.set_muted(true);
                        self.visual_paused_state.insert(id, true);
                        let (w, h) = video.size();
                        if w > 0 && h > 0 {
                            let aspect = w as f32 / h as f32;
                            video.set_display_height(Some(PREVIEW_BASE_HEIGHT as u32));
                            video.set_display_width(Some((PREVIEW_BASE_HEIGHT * aspect) as u32));
                        }
                        let _ = video.seek(Position::Time(src_time), false);
                        self.last_seek_requests.insert(id, src_time);
                        self.visual_players.insert(id, video);
                        self.video_cache_paths.insert(id, clip_path.clone());
                        #[cfg(target_os = "macos")]
                        self.mac_surface_mode_keys.insert(id, target_prefer_surface);
                        changed = true;
                    }
                }
            } else {
                self.video_cache_paths.insert(id, clip_path.clone());
                #[cfg(target_os = "macos")]
                self.mac_surface_mode_keys.insert(id, target_prefer_surface);
            }

            if let Some(video) = self.visual_players.get(&id).cloned() {
                // Visual players always stay muted; audio output is handled by dedicated audio players.
                video.set_muted(true);
                #[cfg(target_os = "macos")]
                {
                    // Renderer (NV12 Metal / BGRA WGPU) owns clip effects; keep only a lightweight key.
                    let effect_key = MacVideoEffectKey::from_values(
                        b,
                        c,
                        s,
                        clip_lut_mix,
                        clip_opacity,
                        blur_sigma,
                    );
                    self.mac_video_effect_keys.insert(id, effect_key);
                }
                // Effects are renderer-driven and no longer rely on GStreamer videobalance/alpha.
            }
        }

        // Keep inactive cached players paused to avoid background decoding.
        for (clip_id, player) in &self.visual_players {
            if !active_video_ids.contains(clip_id) {
                let state_changed = self
                    .visual_paused_state
                    .get(clip_id)
                    .is_none_or(|paused| !*paused);
                if state_changed {
                    player.set_paused(true);
                    self.visual_paused_state.insert(*clip_id, true);
                    log::debug!("[Preview][Transport] visual clip={} paused=true", clip_id);
                }
            }
            if !player.muted() {
                player.set_muted(true);
            }
        }

        self.visual_order = next_order;
        self.trim_visual_player_cache(&active_video_ids);
        self.trim_image_cache(&active_image_ids);
        self.video_cache_paths
            .retain(|clip_id, _| self.visual_players.contains_key(clip_id));
        self.image_cache_paths.retain(|clip_id, _| {
            self.image_cache.contains_key(clip_id) || self.image_decode_in_flight.contains(clip_id)
        });
        self.visual_last_used
            .retain(|clip_id, _| self.visual_players.contains_key(clip_id));
        self.image_last_used.retain(|clip_id, _| {
            self.image_cache.contains_key(clip_id) || self.image_decode_in_flight.contains(clip_id)
        });
        self.image_decode_in_flight
            .retain(|clip_id| active_image_ids.contains(clip_id));
        self.last_clip_blur_sigmas
            .retain(|clip_id, _| active_visual_ids.contains(clip_id));
        self.video_blur_keys
            .retain(|clip_id, _| self.visual_players.contains_key(clip_id));
        #[cfg(target_os = "macos")]
        self.mac_video_effect_keys
            .retain(|clip_id, _| self.visual_players.contains_key(clip_id));
        #[cfg(target_os = "macos")]
        self.mac_surface_mode_keys
            .retain(|clip_id, _| self.visual_players.contains_key(clip_id));

        // Phase 2: Audio
        if !audio_clips.is_empty() {
            log::warn!(
                "[Audio] audio_clips count={} ids={:?}",
                audio_clips.len(),
                audio_clips
                    .iter()
                    .map(|(id, _, _, gl)| (*id, *gl))
                    .collect::<Vec<_>>()
            );
        }
        let mut ensure_audio_player = |id: u64, path: &str, src: Duration, gain_linear: f64| {
            if let Some(existing) = self.audio_players.get(&id) {
                let cur = existing.volume();
                if (cur - gain_linear).abs() > 0.001 {
                    log::warn!(
                        "[Audio] clip={} set_volume old={:.4} new={:.4}",
                        id,
                        cur,
                        gain_linear
                    );
                    existing.set_volume(gain_linear);
                }
                return;
            }
            log::warn!(
                "[Audio] clip={} CREATE player gain={:.4} path={}",
                id,
                gain_linear,
                path
            );
            if let Ok(url) = Url::from_file_path(PathBuf::from(path)) {
                let opts = VideoOptions {
                    is_audio_only: true,
                    frame_buffer_capacity: Some(0),
                    appsink_max_buffers: Some(1),
                    ..Default::default()
                };
                if let Ok(p) = Video::new_with_options(&url, opts) {
                    // Always boot cached audio players in paused state first. This keeps transport
                    // deterministic and avoids play-edge races where first unpause can be skipped.
                    p.set_paused(true);
                    p.set_muted(false);
                    p.set_volume(gain_linear);
                    let _ = p.seek(Position::Time(src), false);
                    self.last_seek_requests.insert(id, src);
                    self.audio_paused_state.insert(id, true);
                    self.audio_players.insert(id, p);
                }
            }
        };

        let mut active_audio_ids = HashSet::new();
        for (id, path, src, gain_linear) in &audio_clips {
            let id = *id;
            if active_visual_ids.contains(&id) || is_image_ext(path) {
                continue;
            }
            active_audio_ids.insert(id);
            Self::touch_cache(&mut self.audio_last_used, &mut self.cache_touch_counter, id);
            ensure_audio_player(id, path, *src, *gain_linear);
        }

        // Prewarm near-future audio clips so boundary playback doesn't miss the first seconds.
        for (id, path, src) in &audio_prewarm_clips {
            let id = *id;
            if active_audio_ids.contains(&id)
                || active_visual_ids.contains(&id)
                || is_image_ext(path)
            {
                continue;
            }
            ensure_audio_player(id, path, *src, 1.0);
        }
        self.trim_audio_player_cache(&active_audio_ids);
        self.last_seek_requests.retain(|k, _| {
            self.visual_players.contains_key(k) || self.audio_players.contains_key(k)
        });
        self.audio_paused_state
            .retain(|clip_id, _| self.audio_players.contains_key(clip_id));
        self.audio_last_used
            .retain(|clip_id, _| self.audio_players.contains_key(clip_id));
        self.enforce_memory_budget(&active_video_ids, &active_audio_ids, &active_image_ids);
        self.log_cache_growth_if_needed();

        self.maybe_log_runtime_state(playhead, active_visual_ids.len(), active_audio_ids.len());
        let sync_elapsed_ms = sync_started.elapsed().as_millis();
        if sync_elapsed_ms >= SYNC_SLOW_MS {
            log::debug!(
                "[Preview][SlowSync] elapsed_ms={} playhead={:.3}s visual_active={} visual_cached={} audio_cached={} image_cached={}",
                sync_elapsed_ms,
                playhead.as_secs_f64(),
                active_visual_ids.len(),
                self.visual_players.len(),
                self.audio_players.len(),
                self.image_cache.len()
            );
        }

        // Kick both proxy and waveform workers so queued background tasks progress.
        self.start_proxy_job_if_needed(cx);
        self.start_waveform_job_if_needed(cx);

        changed
    }

    fn blur_fast_mode_active(&self) -> bool {
        self.blur_interaction_until
            .map(|until| Instant::now() < until)
            .unwrap_or(false)
    }

    fn start_proxy_job_if_needed(&mut self, cx: &mut Context<Self>) {
        let media_tools_ready = self.global.read(cx).media_tools_ready_for_preview_gen();
        let job = self.global.update(cx, |gs, _| gs.take_next_proxy_job());
        let Some(job) = job else {
            return;
        };
        if !media_tools_ready {
            let job_key = job.key.clone();
            self.global.update(cx, |gs, cx| {
                gs.finish_proxy_job(
                    &job_key,
                    ProxyStatus::Failed,
                    Some("MISSING_FFMPEG: proxy generation requires ffmpeg.".to_string()),
                );
                gs.ui_notice = Some("Proxy generation requires FFmpeg.".to_string());
                gs.show_media_dependency_modal();
                cx.notify();
            });
            return;
        }

        let ffmpeg_path = self.global.read(cx).ffmpeg_path.clone();
        let global = self.global.clone();
        let job_key = job.key.clone();
        let src_path = job.src_path.clone();
        let dst_path = job.dst_path.clone();
        log::info!(
            "[Proxy] start {} -> {}",
            src_path.to_string_lossy(),
            dst_path.to_string_lossy()
        );
        cx.spawn(async move |_view, cx| {
            let result = cx
                .background_spawn(async move { proxy::run_proxy_job(&ffmpeg_path, &job) })
                .await;

            let (status, error) = match result {
                Ok(()) => (ProxyStatus::Ready, None),
                Err(err) => (ProxyStatus::Failed, Some(err.to_string())),
            };

            let _ = global.update(cx, |gs, cx| {
                gs.finish_proxy_job(&job_key, status, error);
                cx.notify();
            });
        })
        .detach();
    }

    fn start_waveform_job_if_needed(&mut self, cx: &mut Context<Self>) {
        let media_tools_ready = self.global.read(cx).media_tools_ready_for_preview_gen();
        let job = self.global.update(cx, |gs, _| gs.take_next_waveform_job());
        let Some(job) = job else {
            return;
        };
        if !media_tools_ready {
            let job_key = job.key.clone();
            self.global.update(cx, |gs, cx| {
                gs.finish_waveform_job(
                    &job_key,
                    WaveformStatus::Failed,
                    Some("MISSING_FFMPEG: waveform generation requires ffmpeg.".to_string()),
                    None,
                );
                gs.ui_notice = Some("Waveform generation requires FFmpeg.".to_string());
                gs.show_media_dependency_modal();
                cx.notify();
            });
            return;
        }

        let ffmpeg_path = self.global.read(cx).ffmpeg_path.clone();
        let global = self.global.clone();
        let job_key = job.key.clone();
        let src_path = job.src_path.clone();
        let dst_path = job.dst_path.clone();
        log::info!(
            "[Waveform] start {} -> {}",
            src_path.to_string_lossy(),
            dst_path.to_string_lossy()
        );

        cx.spawn(async move |_view, cx| {
            let result = cx
                .background_spawn(async move { waveform::run_waveform_job(&ffmpeg_path, &job) })
                .await;

            let (status, error, peaks) = match result {
                Ok(peaks) => (WaveformStatus::Ready, None, Some(peaks)),
                Err(err) => (WaveformStatus::Failed, Some(err.to_string()), None),
            };

            let _ = global.update(cx, |gs, cx| {
                gs.finish_waveform_job(&job_key, status, error, peaks);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_transport_and_seek_when_paused(&mut self, cx: &mut Context<Self>) {
        let is_playing = self.global.read(cx).is_playing;
        let playhead = self.global.read(cx).playhead;

        if is_playing {
            // Only active clips should run during playback; cached players stay paused.
            let gs = self.global.read(cx);
            let active_visual_ids: HashSet<u64> = Self::resolve_visible_clips(gs, playhead)
                .into_iter()
                .map(|(id, ..)| id)
                .collect();
            let (active_audio, _) = Self::collect_indexed_audio_clips(
                &mut self.audio_track_time_indices,
                &mut self.audio_track_index_token,
                gs,
                playhead,
                Duration::ZERO,
            );
            let active_audio_ids: HashSet<u64> =
                active_audio.into_iter().map(|(id, ..)| id).collect();
            self.log_active_set_transition(playhead, &active_visual_ids, &active_audio_ids);
            self.update_visual_player_transport(&active_visual_ids);
            self.update_audio_player_transport(&active_audio_ids);
            self.last_pump_playhead = Some(playhead);
            return;
        }

        let no_visual: HashSet<u64> = HashSet::new();
        let no_audio: HashSet<u64> = HashSet::new();
        self.log_active_set_transition(playhead, &no_visual, &no_audio);
        self.update_visual_player_transport(&no_visual);
        self.update_audio_player_transport(&no_audio);
        self.last_pump_playhead = None;

        if !is_playing {
            // `render()` already calls `sync_video_engine()` before this method.
            // Avoid a duplicate sync pass while scrubbing to reduce drag stutter.
            let gs = self.global.read(cx);
            let seek_epsilon = 0.001_f64;
            let visible = Self::resolve_visible_clips(gs, playhead);
            let mut issued_seek = false;

            for (id, _, src_time, ..) in visible {
                if let Some(v) = self.visual_players.get(&id) {
                    let should = self.last_seek_requests.get(&id).is_none_or(|last| {
                        (last.as_secs_f64() - src_time.as_secs_f64()).abs() > seek_epsilon
                    });
                    if should {
                        let _ = v.seek(Position::Time(src_time), false);
                        self.last_seek_requests.insert(id, src_time);
                        issued_seek = true;
                    }
                }
            }
            if issued_seek {
                self.scrub_refresh_until =
                    Some(Instant::now() + Duration::from_millis(SCRUB_REFRESH_TAIL_MS));
            }
        }
    }

    fn schedule_pump_frame(&mut self, token: u64, window: &mut Window, cx: &mut Context<Self>) {
        // Drive playback from animation frames to avoid timer wake starvation on busy event loops.
        cx.on_next_frame(window, move |this, window, cx| {
            if this.pump_token != token || !this.global.read(cx).is_playing {
                this.pump_running = false;
                this.last_pump_playhead = None;
                return;
            }

            let now = Instant::now();
            let dt = now.saturating_duration_since(this.last_pump_instant.unwrap_or(now));
            this.last_pump_instant = Some(now);

            let (playhead_before_tick, next_ph, end) = {
                let gs = this.global.read(cx);
                let total = VideoPreview::sequence_total(gs);
                let current = gs.playhead;
                let next = gs.playhead + dt;
                if next >= total {
                    (current, total, true)
                } else {
                    (current, next, false)
                }
            };
            let external_playhead_jump = this
                .last_pump_playhead
                .map(|last| (last.as_secs_f64() - playhead_before_tick.as_secs_f64()).abs() > 0.050)
                .unwrap_or(false);

            this.global.update(cx, |gs, cx| {
                gs.playhead = next_ph;
                if end {
                    gs.is_playing = false;
                }
                cx.emit(PlaybackUiEvent::Tick);
            });

            if end {
                for (clip_id, v) in &this.visual_players {
                    v.set_paused(true);
                    this.visual_paused_state.insert(*clip_id, true);
                }
                for (clip_id, p) in &this.audio_players {
                    p.set_paused(true);
                    this.audio_paused_state.insert(*clip_id, true);
                }
                this.pump_running = false;
                this.last_pump_playhead = None;
                cx.notify();
                return;
            }

            // Do not hard-seek all visible clips on every timeline change while playing.
            // New/reloaded players are already seeked in `sync_video_engine`; avoiding full
            // re-seek here removes playhead "snap stalls" at cut boundaries.
            this.sync_video_engine(cx);

            let gs = this.global.read(cx);
            let current_playhead = gs.playhead;
            let active_visual = Self::resolve_visible_clips(gs, current_playhead);
            let (active_audio, _) = Self::collect_indexed_audio_clips(
                &mut this.audio_track_time_indices,
                &mut this.audio_track_index_token,
                gs,
                current_playhead,
                Duration::ZERO,
            );
            let active_visual_ids: HashSet<u64> =
                active_visual.iter().map(|(id, ..)| *id).collect();
            let active_audio_ids: HashSet<u64> = active_audio.iter().map(|(id, ..)| *id).collect();
            let newly_active_visual_ids: HashSet<u64> = active_visual_ids
                .difference(&this.last_active_visual_ids)
                .copied()
                .collect();
            let newly_active_audio_ids: HashSet<u64> = active_audio_ids
                .difference(&this.last_active_audio_ids)
                .copied()
                .collect();

            // When timeline edits move playhead abruptly during playback (e.g. ripple delete),
            // force one hard seek for active players so audio/video stay in lockstep.
            let seek_epsilon = 0.004_f64;
            for (id, _, src_time, ..) in &active_visual {
                let should_force = external_playhead_jump || newly_active_visual_ids.contains(id);
                if !should_force {
                    continue;
                }
                if let Some(v) = this.visual_players.get(id) {
                    let should_seek = this.last_seek_requests.get(id).is_none_or(|last| {
                        (last.as_secs_f64() - src_time.as_secs_f64()).abs() > seek_epsilon
                    });
                    if should_seek {
                        let _ = v.seek(Position::Time(*src_time), false);
                        this.last_seek_requests.insert(*id, *src_time);
                    }
                }
            }
            for (id, _, src_time, _) in &active_audio {
                let should_force = external_playhead_jump || newly_active_audio_ids.contains(id);
                if !should_force {
                    continue;
                }
                if let Some(p) = this.audio_players.get(id) {
                    let should_seek = this.last_seek_requests.get(id).is_none_or(|last| {
                        (last.as_secs_f64() - src_time.as_secs_f64()).abs() > seek_epsilon
                    });
                    if should_seek {
                        let _ = p.seek(Position::Time(*src_time), false);
                        this.last_seek_requests.insert(*id, *src_time);
                    }
                }
            }

            this.log_active_set_transition(current_playhead, &active_visual_ids, &active_audio_ids);
            this.update_visual_player_transport(&active_visual_ids);
            this.update_audio_player_transport(&active_audio_ids);
            this.last_pump_playhead = Some(current_playhead);
            cx.notify();

            // Keep pumping while playback remains active.
            if this.pump_token == token && this.global.read(cx).is_playing {
                this.schedule_pump_frame(token, window, cx);
            } else {
                this.pump_running = false;
            }
        });
    }

    fn ensure_pump_running(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let playing_now = self.global.read(cx).is_playing;
        if !playing_now {
            return;
        }
        if self.pump_running {
            return;
        }

        self.pump_running = true;
        self.pump_token = self.pump_token.wrapping_add(1);
        let token = self.pump_token;
        self.last_pump_instant = Some(Instant::now());
        self.reset_present_metrics(cx);

        self.sync_video_engine(cx);
        let gs = self.global.read(cx);
        let visible = Self::resolve_visible_clips(gs, gs.playhead);
        let active_visual_ids: HashSet<u64> = visible.iter().map(|(id, ..)| *id).collect();
        for (id, _, src, ..) in visible {
            if let Some(v) = self.visual_players.get(&id) {
                let _ = v.seek(Position::Time(src), false);
            }
        }
        let (active, _) = Self::collect_indexed_audio_clips(
            &mut self.audio_track_time_indices,
            &mut self.audio_track_index_token,
            gs,
            gs.playhead,
            Duration::ZERO,
        );
        let active_audio_ids: HashSet<u64> = active.iter().map(|(id, ..)| *id).collect();
        if !active_visual_ids.is_empty() && active_audio_ids.is_empty() {
            log::warn!(
                "[Preview][Transport] play-start has visual clip(s) but no active audio clip at playhead={:.3}s",
                gs.playhead.as_secs_f64()
            );
        }
        for (id, _, st, gain_linear) in active {
            if let Some(p) = self.audio_players.get(&id) {
                // Force a fresh paused->playing transition on play start.
                p.set_paused(true);
                p.set_volume(gain_linear);
                p.set_muted(false);
                self.audio_paused_state.insert(id, true);
                let _ = p.seek(Position::Time(st), false);
            }
        }
        self.log_active_set_transition(gs.playhead, &active_visual_ids, &active_audio_ids);
        self.update_visual_player_transport(&active_visual_ids);
        self.update_audio_player_transport(&active_audio_ids);
        self.last_pump_playhead = Some(gs.playhead);
        self.schedule_pump_frame(token, window, cx);
    }

    fn reset_present_metrics(&mut self, cx: &mut Context<Self>) {
        self.present_last_frame_instant = None;
        self.present_fps_window_start = None;
        self.present_fps_window_frames = 0;
        self.present_fps_ema = 0.0;
        self.present_refresh_interval_estimate_s = 0.0;
        self.present_dropped_frames_total = 0;
        self.global.update(cx, |gs, _| {
            gs.set_preview_present_metrics(0.0, 0);
        });
    }

    fn sample_present_fps(&mut self, cx: &mut Context<Self>, is_playing: bool) {
        if !is_playing {
            self.present_last_frame_instant = None;
            return;
        }

        let now = Instant::now();
        if self.present_fps_window_start.is_none() {
            self.present_fps_window_start = Some(now);
        }
        self.present_fps_window_frames = self.present_fps_window_frames.saturating_add(1);

        if let Some(last) = self.present_last_frame_instant {
            let dt = now.saturating_duration_since(last).as_secs_f32();
            if (PRESENT_FRAME_DT_MIN_S..=PRESENT_FRAME_DT_MAX_S).contains(&dt) {
                if self.present_refresh_interval_estimate_s <= 0.0 {
                    self.present_refresh_interval_estimate_s = dt;
                } else if dt < self.present_refresh_interval_estimate_s {
                    self.present_refresh_interval_estimate_s =
                        self.present_refresh_interval_estimate_s * 0.80 + dt * 0.20;
                } else {
                    self.present_refresh_interval_estimate_s =
                        self.present_refresh_interval_estimate_s * 0.995 + dt * 0.005;
                }

                let base_interval = self
                    .present_refresh_interval_estimate_s
                    .max(PRESENT_FRAME_DT_MIN_S);
                let expected_frames = ((dt / base_interval).round().clamp(1.0, 12.0)) as u64;
                if expected_frames > 1 {
                    self.present_dropped_frames_total = self
                        .present_dropped_frames_total
                        .saturating_add(expected_frames - 1);
                }
            } else if dt > PRESENT_FRAME_DT_MAX_S {
                // Do not treat long stalls (sleep/background) as frame drops.
                self.present_refresh_interval_estimate_s = 0.0;
            }
        }
        self.present_last_frame_instant = Some(now);

        let mut publish = false;
        if let Some(start) = self.present_fps_window_start {
            let elapsed = now.saturating_duration_since(start);
            if elapsed >= Duration::from_millis(PRESENT_FPS_SAMPLE_MS) {
                let elapsed_sec = elapsed.as_secs_f32().max(0.000_1);
                let sample_fps = self.present_fps_window_frames as f32 / elapsed_sec;
                self.present_fps_ema = if self.present_fps_ema <= 0.0 {
                    sample_fps
                } else {
                    self.present_fps_ema * 0.82 + sample_fps * 0.18
                };
                self.present_fps_window_start = Some(now);
                self.present_fps_window_frames = 0;
                publish = true;
            }
        }

        if publish {
            let fps = if self.present_fps_ema.is_finite() {
                self.present_fps_ema.max(0.0)
            } else {
                0.0
            };
            let dropped = self.present_dropped_frames_total;
            self.global.update(cx, |gs, _| {
                gs.set_preview_present_metrics(fps, dropped);
            });
        }
    }

    fn sample_video_input_fps(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        if self.video_input_fps_window_start.is_none() {
            self.video_input_fps_window_start = Some(now);
        }

        let mut active_video_count: usize = 0;
        for clip_id in &self.visual_order {
            let Some(video) = self.visual_players.get(clip_id) else {
                continue;
            };
            active_video_count = active_video_count.saturating_add(1);
            let counter = video.decoded_frame_counter();
            let prev_counter = self
                .last_video_frame_counters
                .get(clip_id)
                .copied()
                .unwrap_or(0);
            if prev_counter > 0 && counter > prev_counter {
                let delta = counter.saturating_sub(prev_counter);
                let delta_u32 = u32::try_from(delta).unwrap_or(u32::MAX);
                self.video_input_fps_window_frames =
                    self.video_input_fps_window_frames.saturating_add(delta_u32);
            }
            self.last_video_frame_counters.insert(*clip_id, counter);
        }
        self.last_video_frame_counters
            .retain(|clip_id, _| self.visual_players.contains_key(clip_id));

        let mut publish = false;
        if let Some(start) = self.video_input_fps_window_start {
            let elapsed = now.saturating_duration_since(start);
            if elapsed >= Duration::from_millis(VIDEO_INPUT_FPS_SAMPLE_MS) {
                let sample_fps = if active_video_count > 0 {
                    let elapsed_sec = elapsed.as_secs_f32().max(0.000_1);
                    (self.video_input_fps_window_frames as f32 / elapsed_sec)
                        / (active_video_count as f32)
                } else {
                    0.0
                };
                self.video_input_fps_ema = if self.video_input_fps_ema <= 0.0 {
                    sample_fps
                } else {
                    self.video_input_fps_ema * 0.82 + sample_fps * 0.18
                };
                self.video_input_fps_window_start = Some(now);
                self.video_input_fps_window_frames = 0;
                publish = true;
            }
        }

        if publish {
            let fps = if self.video_input_fps_ema.is_finite() {
                self.video_input_fps_ema.max(0.0)
            } else {
                0.0
            };
            self.global.update(cx, |gs, _| {
                gs.set_preview_video_input_fps(fps);
            });
        }
    }
}

impl Focusable for VideoPreview {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone().unwrap()
    }
}

impl Render for VideoPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let total = {
            let gs = self.global.read(cx);
            Self::sequence_total(gs)
        };
        if self.global.read(cx).playhead > total {
            self.global.update(cx, |gs, cx| {
                gs.playhead = total;
                cx.notify();
            });
        }

        let is_playing = self.global.read(cx).is_playing;
        if is_playing {
            // Keep playback updates in a single pump path to avoid duplicate sync work
            // from both `render()` and the on-next-frame playback callback.
            self.ensure_pump_running(window, cx);
            // Keep RAF requested from render (GPUI expects this API to be called in paint/render path).
            window.request_animation_frame();
        } else {
            // Paused mode still needs immediate sync + seek so timeline scrub and inspector edits refresh.
            self.sync_video_engine(cx);
            self.apply_transport_and_seek_when_paused(cx);
            if let Some(until) = self.scrub_refresh_until {
                if Instant::now() < until {
                    window.request_animation_frame();
                } else {
                    self.scrub_refresh_until = None;
                }
            }
            if self.blur_fast_mode_active() {
                // Keep refreshing while blur is actively changing; when it settles we auto-render HQ pass.
                window.request_animation_frame();
            }
        }

        self.sample_video_input_fps(cx);
        self.sample_present_fps(cx, is_playing);

        let gs = self.global.read(cx);
        let mut canvas_w = gs.canvas_w;
        let mut canvas_h = gs.canvas_h;
        if canvas_w <= 0.0 || canvas_h <= 0.0 {
            canvas_w = 920.0;
            canvas_h = 2080.0;
        }

        // -------------------------------------------------------------
        // [Core] Virtual-canvas rendering flow.
        // -------------------------------------------------------------

        // 1. Compute view scale from available viewport space.
        // Keep preview scalable without changing coordinate mapping behavior.
        let viewport = window.viewport_size();
        let viewport_w: f32 = viewport.width.into();
        let viewport_h: f32 = viewport.height.into();
        // Use the area above timeline as the effective height budget for preview.
        let max_card_height =
            (viewport_h - TIMELINE_PANEL_H - 28.0).clamp(PREVIEW_MIN_HEIGHT, PREVIEW_MAX_HEIGHT);
        // Keep a small horizontal safety margin while allowing a wider preview.
        let reserved_w = SIDEBAR_W + EDITOR_PANEL_W + gs.inspector_panel_width() + 32.0;
        let approx_available_w = (viewport_w - reserved_w).max(200.0);
        let max_preview_w = approx_available_w.min(PREVIEW_MAX_WIDTH);
        let max_preview_h = max_card_height.clamp(PREVIEW_MIN_HEIGHT, PREVIEW_MAX_HEIGHT);
        let view_scale = (max_preview_w / canvas_w)
            .min(max_preview_h / canvas_h)
            .max(0.01);

        // Compute the physical canvas size rendered on screen.
        let physical_canvas_w = canvas_w * view_scale;
        let physical_canvas_h = canvas_h * view_scale;

        // 2. Build clip stack (all video layers).
        let mut clip_stack = div().relative().size_full(); // Fill the canvas container.

        if self.visual_order.is_empty() {
            clip_stack = clip_stack.flex().items_center().justify_center().child(
                div()
                    .text_color(white().opacity(0.3))
                    .child("No Media Loaded"),
            );
        } else {
            let visual_order = self.visual_order.clone();
            let fast_blur_mode = self.blur_fast_mode_active();
            let active_local_mask_layer = gs.active_local_mask_layer();

            // Pre-collect all per-clip data from gs so we can drop the immutable borrow
            // before calling image_render_for_clip (which needs &mut cx for async spawning).
            struct ClipRenderData {
                clip_id: u64,
                b: f32,
                c: f32,
                s: f32,
                opacity: f32,
                blur_sigma: f32,
                lut_mix: f32,
                local_mask_layers: Vec<VideoLocalMaskLayer>,
                local_mask_enabled: bool,
                local_mask_center_x: f32,
                local_mask_center_y: f32,
                local_mask_radius: f32,
                local_mask_feather: f32,
                hue: f32,
                sat: f32,
                light: f32,
                alpha: f32,
                scale: f32,
                pos_x: f32,
                pos_y: f32,
                rotation_deg: f32,
            }
            let clip_data: Vec<ClipRenderData> = visual_order
                .iter()
                .map(|clip_id| {
                    let (b, c, s) = Self::get_clip_bcs(gs, *clip_id).unwrap_or((0.0, 1.0, 1.0));
                    let opacity = Self::get_clip_opacity(gs, *clip_id).unwrap_or(1.0);
                    let blur_sigma = Self::get_clip_blur_sigma(gs, *clip_id).unwrap_or(0.0);
                    let lut_mix = Self::get_clip_lut_mix(gs, *clip_id).unwrap_or(0.0);
                    let local_mask_layers = Self::get_clip_local_mask_layers(gs, *clip_id);
                    let active_layer_idx = active_local_mask_layer
                        .min(local_mask_layers.len().saturating_sub(1))
                        .min(VIDEO_MAX_LOCAL_MASK_LAYERS.saturating_sub(1));
                    let active_local_layer = local_mask_layers
                        .get(active_layer_idx)
                        .copied()
                        .unwrap_or_default();
                    let (hue, sat, light, alpha) =
                        Self::get_clip_tint(gs, *clip_id).unwrap_or((0.0, 0.0, 0.0, 0.0));
                    let (scale, pos_x, pos_y, rotation_deg) =
                        Self::get_clip_transform(gs, *clip_id, gs.playhead)
                            .unwrap_or((1.0, 0.0, 0.0, 0.0));
                    ClipRenderData {
                        clip_id: *clip_id,
                        b,
                        c,
                        s,
                        opacity,
                        blur_sigma,
                        lut_mix,
                        local_mask_layers,
                        local_mask_enabled: active_local_layer.enabled,
                        local_mask_center_x: active_local_layer.center_x,
                        local_mask_center_y: active_local_layer.center_y,
                        local_mask_radius: active_local_layer.radius,
                        local_mask_feather: active_local_layer.feather,
                        hue,
                        sat,
                        light,
                        alpha,
                        scale,
                        pos_x,
                        pos_y,
                        rotation_deg,
                    }
                })
                .collect();

            let selected_clip_id = gs.selected_clip_id;
            // Release gs borrow so cx is available for mutable use below.
            let _ = gs;

            for cd in &clip_data {
                let clip_id = &cd.clip_id;
                let (b, c, s) = (cd.b, cd.c, cd.s);
                let opacity = cd.opacity;
                let blur_sigma = cd.blur_sigma;
                let lut_mix = cd.lut_mix;
                let local_mask_layers = &cd.local_mask_layers;
                let local_mask_enabled = cd.local_mask_enabled;
                let local_mask_center_x = cd.local_mask_center_x;
                let local_mask_center_y = cd.local_mask_center_y;
                let local_mask_radius = cd.local_mask_radius;
                let local_mask_feather = cd.local_mask_feather;
                let (hue, sat, light, alpha) = (cd.hue, cd.sat, cd.light, cd.alpha);
                let (scale, pos_x, pos_y, rotation_deg) =
                    (cd.scale, cd.pos_x, cd.pos_y, cd.rotation_deg);

                // B. Compute logical size and position.
                let logical_w = canvas_w * scale;
                let logical_h = canvas_h * scale;

                let logical_center_x = (canvas_w / 2.0) + (pos_x * canvas_w);
                let logical_center_y = (canvas_h / 2.0) + (pos_y * canvas_h);

                let logical_left = logical_center_x - (logical_w / 2.0);
                let logical_top = logical_center_y - (logical_h / 2.0);

                // C. Convert to physical pixels.
                let final_w = logical_w * view_scale;
                let final_h = logical_h * view_scale;
                let final_left = logical_left * view_scale;
                let final_top = logical_top * view_scale;

                let mut is_full_canvas_video = false;
                #[cfg(target_os = "macos")]
                let mut clip_content = {
                    if self.prefer_surface_for_image_clip(*clip_id, cx) {
                        if let Some((surface, width, height)) = self.image_surface_for_clip(
                            *clip_id,
                            b,
                            c,
                            s,
                            lut_mix,
                            blur_sigma,
                            hue,
                            sat,
                            light,
                            alpha,
                            fast_blur_mode,
                            cx,
                        ) {
                            div()
                                .absolute()
                                .flex_none()
                                .w(px(final_w))
                                .h(px(final_h))
                                .left(px(final_left))
                                .top(px(final_top))
                                .child(
                                    SurfaceImageElement::new(surface, width, height)
                                        .opacity(opacity)
                                        .rotation_deg(rotation_deg)
                                        .id(ElementId::Name(
                                            format!("img-surface-{}", clip_id).into(),
                                        )),
                                )
                        } else if let Some((image, width, height)) = self.image_render_for_clip(
                            *clip_id,
                            b,
                            c,
                            s,
                            lut_mix,
                            blur_sigma,
                            hue,
                            sat,
                            light,
                            alpha,
                            fast_blur_mode,
                            cx,
                        ) {
                            div()
                                .absolute()
                                .flex_none()
                                .w(px(final_w))
                                .h(px(final_h))
                                .left(px(final_left))
                                .top(px(final_top))
                                .opacity(opacity)
                                .child(
                                    ImageElement::new(image, width, height)
                                        .rotation_deg(rotation_deg)
                                        .id(ElementId::Name(format!("img-{}", clip_id).into())),
                                )
                        } else if let Some(video_player) = self.visual_players.get(clip_id) {
                            is_full_canvas_video = true;
                            let use_surface_mode = self
                                .mac_surface_mode_keys
                                .get(clip_id)
                                .copied()
                                .unwrap_or(true);
                            let styled_opacity = if use_surface_mode { 1.0 } else { opacity };
                            let element_opacity = if use_surface_mode { opacity } else { 1.0 };

                            div()
                                .absolute()
                                .flex_none()
                                .w(px(physical_canvas_w))
                                .h(px(physical_canvas_h))
                                .left(px(0.0))
                                .top(px(0.0))
                                .opacity(styled_opacity)
                                .child(
                                    VideoElement::new(video_player.clone())
                                        .color_balance(b, c, s)
                                        .lut_mix(lut_mix)
                                        .tint_overlay(
                                            hue,
                                            sat,
                                            light,
                                            (alpha * opacity).clamp(0.0, 1.0),
                                        )
                                        .blur_sigma(blur_sigma)
                                        .rotation_deg(rotation_deg)
                                        .preview_transform(scale, pos_x, pos_y, canvas_w, canvas_h)
                                        .opacity(element_opacity)
                                        .local_mask_layers(local_mask_layers)
                                        .id(ElementId::Name(format!("vid-{}", clip_id).into())),
                                )
                        } else {
                            continue;
                        }
                    } else if let Some((image, width, height)) = self.image_render_for_clip(
                        *clip_id,
                        b,
                        c,
                        s,
                        lut_mix,
                        blur_sigma,
                        hue,
                        sat,
                        light,
                        alpha,
                        fast_blur_mode,
                        cx,
                    ) {
                        div()
                            .absolute()
                            .flex_none()
                            .w(px(final_w))
                            .h(px(final_h))
                            .left(px(final_left))
                            .top(px(final_top))
                            .opacity(opacity)
                            .child(
                                ImageElement::new(image, width, height)
                                    .rotation_deg(rotation_deg)
                                    .id(ElementId::Name(format!("img-{}", clip_id).into())),
                            )
                    } else if let Some(video_player) = self.visual_players.get(clip_id) {
                        is_full_canvas_video = true;
                        let use_surface_mode = self
                            .mac_surface_mode_keys
                            .get(clip_id)
                            .copied()
                            .unwrap_or(true);
                        let styled_opacity = if use_surface_mode { 1.0 } else { opacity };
                        let element_opacity = if use_surface_mode { opacity } else { 1.0 };

                        div()
                            .absolute()
                            .flex_none()
                            .w(px(physical_canvas_w))
                            .h(px(physical_canvas_h))
                            .left(px(0.0))
                            .top(px(0.0))
                            .opacity(styled_opacity)
                            .child(
                                VideoElement::new(video_player.clone())
                                    .color_balance(b, c, s)
                                    .lut_mix(lut_mix)
                                    .tint_overlay(
                                        hue,
                                        sat,
                                        light,
                                        (alpha * opacity).clamp(0.0, 1.0),
                                    )
                                    .blur_sigma(blur_sigma)
                                    .rotation_deg(rotation_deg)
                                    .preview_transform(scale, pos_x, pos_y, canvas_w, canvas_h)
                                    .opacity(element_opacity)
                                    .local_mask_layers(local_mask_layers)
                                    .id(ElementId::Name(format!("vid-{}", clip_id).into())),
                            )
                    } else {
                        continue;
                    }
                };

                #[cfg(not(target_os = "macos"))]
                let mut clip_content = if let Some((image, width, height)) = self
                    .image_render_for_clip(
                        *clip_id,
                        b,
                        c,
                        s,
                        lut_mix,
                        blur_sigma,
                        hue,
                        sat,
                        light,
                        alpha,
                        fast_blur_mode,
                        cx,
                    ) {
                    div()
                        .absolute()
                        .flex_none()
                        .w(px(final_w))
                        .h(px(final_h))
                        .left(px(final_left))
                        .top(px(final_top))
                        .opacity(opacity)
                        .child(
                            ImageElement::new(image, width, height)
                                .rotation_deg(rotation_deg)
                                .id(ElementId::Name(format!("img-{}", clip_id).into())),
                        )
                } else if let Some(video_player) = self.visual_players.get(clip_id) {
                    is_full_canvas_video = true;
                    div()
                        .absolute()
                        .flex_none()
                        .w(px(physical_canvas_w))
                        .h(px(physical_canvas_h))
                        .left(px(0.0))
                        .top(px(0.0))
                        .child(
                            VideoElement::new(video_player.clone())
                                .color_balance(b, c, s)
                                .lut_mix(lut_mix)
                                .tint_overlay(hue, sat, light, (alpha * opacity).clamp(0.0, 1.0))
                                .blur_sigma(blur_sigma)
                                .rotation_deg(rotation_deg)
                                .preview_transform(scale, pos_x, pos_y, canvas_w, canvas_h)
                                .opacity(opacity)
                                .local_mask_layers(local_mask_layers)
                                .id(ElementId::Name(format!("vid-{}", clip_id).into())),
                        )
                } else {
                    continue;
                };

                let show_mask_overlay = local_mask_enabled && selected_clip_id == Some(*clip_id);
                if show_mask_overlay {
                    // Match shader space exactly:
                    // dist = length(vec2((x-cx)*aspect, (y-cy))), so radius uses frame-height units.
                    let inner_radius = local_mask_radius.clamp(0.0, 1.0);
                    let outer_radius = (local_mask_radius + local_mask_feather).clamp(0.001, 2.0);
                    let center_x_px = final_left + (local_mask_center_x * final_w);
                    let center_y_px = final_top + (local_mask_center_y * final_h);
                    let inner_diameter_px = (2.0 * inner_radius * final_h).max(2.0);
                    let outer_diameter_px = (2.0 * outer_radius * final_h).max(2.0);
                    let inner_left = center_x_px - (inner_diameter_px * 0.5);
                    let inner_top = center_y_px - (inner_diameter_px * 0.5);
                    let outer_left = center_x_px - (outer_diameter_px * 0.5);
                    let outer_top = center_y_px - (outer_diameter_px * 0.5);
                    clip_content = clip_content.child(
                        div()
                            .absolute()
                            .left(px(outer_left))
                            .top(px(outer_top))
                            .w(px(outer_diameter_px))
                            .h(px(outer_diameter_px))
                            .rounded_full()
                            .border_1()
                            .border_color(white().opacity(0.45))
                            .child(
                                div()
                                    .absolute()
                                    .left(px(inner_left - outer_left))
                                    .top(px(inner_top - outer_top))
                                    .w(px(inner_diameter_px))
                                    .h(px(inner_diameter_px))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(white().opacity(0.85)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left(px((outer_diameter_px * 0.5) - 2.0))
                                    .top(px((outer_diameter_px * 0.5) - 2.0))
                                    .w(px(4.0))
                                    .h(px(4.0))
                                    .rounded_full()
                                    .bg(white().opacity(0.9)),
                            ),
                    );
                }

                let overlay_alpha = alpha * opacity;
                if overlay_alpha > 0.001 && !is_full_canvas_video {
                    clip_content = clip_content.child(div().absolute().inset_0().bg(gpui::hsla(
                        hue / 360.0,
                        sat,
                        light,
                        overlay_alpha,
                    )));
                }

                clip_stack = clip_stack.child(clip_content);
            }
        }

        let mut subtitle_layer = div().absolute().inset_0();
        // Re-borrow gs for subtitle rendering after the clip loop released it.
        let gs = self.global.read(cx);
        let visible_subtitles = Self::resolve_visible_subtitles(gs, gs.playhead);
        for sub in visible_subtitles {
            let (pos_x, pos_y, font_size_raw) = gs.effective_subtitle_transform(&sub);
            let center_x = (canvas_w / 2.0) + (pos_x * canvas_w);
            let center_y = (canvas_h / 2.0) + (pos_y * canvas_h);
            let left = center_x * view_scale;
            let top = center_y * view_scale;
            let font_size = font_size_raw.max(1.0) * view_scale;
            let (r, g, b, a) = sub.color_rgba;
            let color_hex =
                ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | (a as u32);

            subtitle_layer = subtitle_layer.child(
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .text_size(px(font_size))
                    .text_color(gpui::rgba(color_hex))
                    .when_some(sub.font_family.clone(), |this, family| {
                        this.font_family(family)
                    })
                    .child(sub.text),
            );
        }

        let cpu_safe_mode_notice = bgra_cpu_safe_mode_notice();

        // --- Outer preview card layout ---

        div()
            .size_full()
            .bg(rgb(0x000000))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(max_preview_w))
                    .max_h(px(max_card_height))
                    .rounded_xl()
                    .border_1()
                    .border_color(white().opacity(0.1))
                    .bg(rgb(0x09090b))
                    .shadow_2xl()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        // Preview area container (gray background).
                        div()
                            .relative()
                            .w_full()
                            .h_auto()
                            .flex_1()
                            .max_h(px(max_preview_h))
                            .min_h(px(PREVIEW_MIN_HEIGHT.min(max_preview_h)))
                            .bg(rgb(0x2a2a2a))
                            .flex() // Center the canvas with flex layout.
                            .items_center()
                            .justify_center()
                            .child(
                                // Actual canvas area (16:9 black frame).
                                div()
                                    .relative()
                                    .flex_none() // Fixed size; not stretchable.
                                    .w(px(physical_canvas_w))
                                    .h(px(physical_canvas_h))
                                    .bg(black()) // Canvas background color.
                                    .overflow_hidden() // Clip overflow content (FFmpeg-like).
                                    .child(clip_stack) // Insert all video layers.
                                    .child(subtitle_layer),
                            )
                            .when_some(cpu_safe_mode_notice, |s, note| {
                                s.child(
                                    div()
                                        .absolute()
                                        .top_2()
                                        .left_2()
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(white().opacity(0.45))
                                        .bg(rgb(0x3b0a0a))
                                        .text_xs()
                                        .text_color(white().opacity(0.95))
                                        .child(note),
                                )
                            }),
                    ),
            )
    }
}
