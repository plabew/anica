// =========================================
// =========================================
// crates/motionloom/src/model.rs
use crate::effects::LayerColorBlurEffects;
use crate::keyframe::{self, ScalarKeyframe};
use crate::transitions;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ColorRgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AnimF32 {
    Const(f32),
    Linear {
        from: f32,
        to: f32,
        start_frame: u32,
        end_frame: u32,
    },
    Keyframes(Vec<(u32, f32)>),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TextStyle {
    pub justify_center: bool,
    pub align_center: bool,
    pub background: ColorRgba,
    pub font_size: AnimF32,
    pub opacity: AnimF32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ZoomStyle {
    pub scale: AnimF32,
}

impl Default for ZoomStyle {
    fn default() -> Self {
        Self {
            scale: AnimF32::Const(1.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClipZoomSpec {
    pub clip_id: Option<u64>,
    pub start_frame: u32,
    pub end_frame: u32,
    pub zoom: ZoomStyle,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum SlideDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum VideoEffect {
    ColorCorrection {
        brightness: f32,
        contrast: f32,
        saturation: f32,
    },
    Transform {
        scale: f32,
        position_x: f32,
        position_y: f32,
        #[serde(default)]
        rotation_deg: f32,
    },
    Tint {
        hue: f32,
        saturation: f32,
        lightness: f32,
        alpha: f32,
    },
    Opacity {
        alpha: f32,
    },
    Fade {
        fade_in: f32,
        fade_out: f32,
    },
    Dissolve {
        dissolve_in: f32,
        dissolve_out: f32,
    },
    Slide {
        in_direction: SlideDirection,
        out_direction: SlideDirection,
        slide_in: f32,
        slide_out: f32,
    },
    Zoom {
        zoom_in: f32,
        zoom_out: f32,
        zoom_amount: f32,
    },
    ShockZoom {
        shock_in: f32,
        shock_out: f32,
        shock_amount: f32,
    },
    GaussianBlur {
        sigma: f32,
    },
    LocalMask {
        enabled: bool,
        center_x: f32,
        center_y: f32,
        radius: f32,
        feather: f32,
        strength: f32,
    },
    LocalMaskAdjust {
        brightness: f32,
        contrast: f32,
        saturation: f32,
        opacity: f32,
        blur_sigma: f32,
    },
    Pixelate {
        block_size: u32,
    },
}

pub const MAX_LOCAL_MASK_LAYERS: usize = 5;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct LocalMaskLayer {
    pub enabled: bool,
    pub center_x: f32,
    pub center_y: f32,
    pub radius: f32,
    pub feather: f32,
    pub strength: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub opacity: f32,
    pub blur_sigma: f32,
}

impl Default for LocalMaskLayer {
    fn default() -> Self {
        Self {
            enabled: false,
            center_x: 0.5,
            center_y: 0.5,
            radius: 0.25,
            feather: 0.15,
            strength: 1.0,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            opacity: 1.0,
            blur_sigma: 0.0,
        }
    }
}

impl VideoEffect {
    pub fn new_color() -> Self {
        VideoEffect::ColorCorrection {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
        }
    }

    pub fn new_transform() -> Self {
        VideoEffect::Transform {
            scale: 1.0,
            position_x: 0.0,
            position_y: 0.0,
            rotation_deg: 0.0,
        }
    }

    pub fn new_tint() -> Self {
        VideoEffect::Tint {
            hue: 0.0,
            saturation: 0.0,
            lightness: 0.0,
            alpha: 0.0,
        }
    }

    pub fn new_opacity() -> Self {
        VideoEffect::Opacity { alpha: 1.0 }
    }

    pub fn new_gaussian_blur() -> Self {
        VideoEffect::GaussianBlur { sigma: 0.0 }
    }

    pub fn new_local_mask() -> Self {
        VideoEffect::LocalMask {
            enabled: false,
            center_x: 0.5,
            center_y: 0.5,
            radius: 0.25,
            feather: 0.15,
            strength: 1.0,
        }
    }

    pub fn new_local_mask_adjust() -> Self {
        VideoEffect::LocalMaskAdjust {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            opacity: 1.0,
            blur_sigma: 0.0,
        }
    }

    pub fn new_fade() -> Self {
        VideoEffect::Fade {
            fade_in: 0.0,
            fade_out: 0.0,
        }
    }

    pub fn new_dissolve() -> Self {
        VideoEffect::Dissolve {
            dissolve_in: 0.0,
            dissolve_out: 0.0,
        }
    }

    pub fn new_slide() -> Self {
        VideoEffect::Slide {
            in_direction: SlideDirection::Right,
            out_direction: SlideDirection::Left,
            slide_in: 0.0,
            slide_out: 0.0,
        }
    }

    pub fn new_zoom() -> Self {
        VideoEffect::Zoom {
            zoom_in: 0.0,
            zoom_out: 0.0,
            zoom_amount: 1.1,
        }
    }

    pub fn new_shock_zoom() -> Self {
        VideoEffect::ShockZoom {
            shock_in: 0.0,
            shock_out: 0.0,
            shock_amount: 1.2,
        }
    }

    pub fn standard_set() -> Vec<Self> {
        vec![
            Self::new_color(),
            Self::new_transform(),
            Self::new_tint(),
            Self::new_opacity(),
            Self::new_gaussian_blur(),
            Self::new_local_mask(),
            Self::new_local_mask_adjust(),
            Self::new_fade(),
            Self::new_dissolve(),
            Self::new_slide(),
            Self::new_zoom(),
            Self::new_shock_zoom(),
        ]
    }
}

impl Default for VideoEffect {
    fn default() -> Self {
        Self::new_color()
    }
}

#[derive(Debug, Clone)]
pub struct LayerEffectClip {
    pub id: u64,
    pub start: Duration,
    pub duration: Duration,
    pub track_index: usize,
    pub fade_in: Duration,
    pub fade_out: Duration,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub blur_sigma: f32,
    pub brightness_enabled: bool,
    pub contrast_enabled: bool,
    pub saturation_enabled: bool,
    pub blur_enabled: bool,
    pub brightness_keyframes: Vec<ScalarKeyframe>,
    pub contrast_keyframes: Vec<ScalarKeyframe>,
    pub saturation_keyframes: Vec<ScalarKeyframe>,
    pub blur_keyframes: Vec<ScalarKeyframe>,
    pub motionloom_enabled: bool,
    pub motionloom_script: String,
}

impl LayerEffectClip {
    pub fn end(&self) -> Duration {
        self.start.saturating_add(self.duration)
    }

    pub fn local_time(&self, timeline_time: Duration) -> Option<Duration> {
        let clip_end = self.end();
        if self.duration <= Duration::ZERO
            || timeline_time < self.start
            || timeline_time >= clip_end
        {
            return None;
        }
        Some(timeline_time.saturating_sub(self.start).min(self.duration))
    }

    fn set_scalar_keyframe(keys: &mut Vec<ScalarKeyframe>, t: Duration, value: f32) {
        keyframe::set_or_insert(keys, t, value, Duration::from_secs_f32(1.0 / 240.0));
    }

    fn scalar_keyframe_index_at(keys: &[ScalarKeyframe], t: Duration) -> Option<usize> {
        keyframe::index_at(keys, t, Duration::from_secs_f32(1.0 / 240.0))
    }

    fn sample_scalar(keys: &[ScalarKeyframe], t: Duration, fallback: f32) -> f32 {
        keyframe::sample_linear(keys, t, fallback)
    }

    pub fn set_brightness_keyframe(&mut self, t: Duration, value: f32) {
        Self::set_scalar_keyframe(&mut self.brightness_keyframes, t, value.clamp(-1.0, 1.0));
    }

    pub fn set_contrast_keyframe(&mut self, t: Duration, value: f32) {
        Self::set_scalar_keyframe(&mut self.contrast_keyframes, t, value.clamp(0.0, 2.0));
    }

    pub fn set_saturation_keyframe(&mut self, t: Duration, value: f32) {
        Self::set_scalar_keyframe(&mut self.saturation_keyframes, t, value.clamp(0.0, 2.0));
    }

    pub fn set_blur_keyframe(&mut self, t: Duration, value: f32) {
        Self::set_scalar_keyframe(&mut self.blur_keyframes, t, value.clamp(0.0, 64.0));
    }

    pub fn brightness_keyframe_index_at(&self, t: Duration) -> Option<usize> {
        Self::scalar_keyframe_index_at(&self.brightness_keyframes, t)
    }

    pub fn contrast_keyframe_index_at(&self, t: Duration) -> Option<usize> {
        Self::scalar_keyframe_index_at(&self.contrast_keyframes, t)
    }

    pub fn saturation_keyframe_index_at(&self, t: Duration) -> Option<usize> {
        Self::scalar_keyframe_index_at(&self.saturation_keyframes, t)
    }

    pub fn blur_keyframe_index_at(&self, t: Duration) -> Option<usize> {
        Self::scalar_keyframe_index_at(&self.blur_keyframes, t)
    }

    pub fn sample_brightness_local(&self, t: Duration) -> f32 {
        if !self.brightness_enabled {
            return 0.0;
        }
        Self::sample_scalar(&self.brightness_keyframes, t, self.brightness).clamp(-1.0, 1.0)
    }

    pub fn sample_contrast_local(&self, t: Duration) -> f32 {
        if !self.contrast_enabled {
            return 1.0;
        }
        Self::sample_scalar(&self.contrast_keyframes, t, self.contrast).clamp(0.0, 2.0)
    }

    pub fn sample_saturation_local(&self, t: Duration) -> f32 {
        if !self.saturation_enabled {
            return 1.0;
        }
        Self::sample_scalar(&self.saturation_keyframes, t, self.saturation).clamp(0.0, 2.0)
    }

    pub fn sample_blur_local(&self, t: Duration) -> f32 {
        if !self.blur_enabled {
            return 0.0;
        }
        Self::sample_scalar(&self.blur_keyframes, t, self.blur_sigma).clamp(0.0, 64.0)
    }

    pub fn has_any_effect_enabled(&self) -> bool {
        self.brightness_enabled
            || self.contrast_enabled
            || self.saturation_enabled
            || self.blur_enabled
    }

    pub fn effects_at(&self, timeline_time: Duration) -> Option<LayerColorBlurEffects> {
        let local = self.local_time(timeline_time)?;
        Some(
            LayerColorBlurEffects {
                brightness: self.sample_brightness_local(local),
                contrast: self.sample_contrast_local(local),
                saturation: self.sample_saturation_local(local),
                blur_sigma: self.sample_blur_local(local),
            }
            .normalized(),
        )
    }

    pub fn clear_brightness_effect(&mut self) {
        self.brightness_enabled = false;
        self.brightness = 0.0;
        self.brightness_keyframes.clear();
    }

    pub fn clear_contrast_effect(&mut self) {
        self.contrast_enabled = false;
        self.contrast = 1.0;
        self.contrast_keyframes.clear();
    }

    pub fn clear_saturation_effect(&mut self) {
        self.saturation_enabled = false;
        self.saturation = 1.0;
        self.saturation_keyframes.clear();
    }

    pub fn clear_blur_effect(&mut self) {
        self.blur_enabled = false;
        self.blur_sigma = 0.0;
        self.blur_keyframes.clear();
    }

    pub fn envelope_factor_at(&self, timeline_time: Duration) -> f32 {
        let clip_end = self.end();
        if self.duration <= Duration::ZERO
            || timeline_time < self.start
            || timeline_time >= clip_end
        {
            return 0.0;
        }

        transitions::envelope_factor_at(
            self.duration,
            self.fade_in,
            self.fade_out,
            timeline_time.saturating_sub(self.start),
        )
    }
}
