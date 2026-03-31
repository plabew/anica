// =========================================
// =========================================
// crates/motionloom/src/effects.rs

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PerClipColorBlurEffects {
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub blur_sigma: f32,
}

impl Default for PerClipColorBlurEffects {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            blur_sigma: 0.0,
        }
    }
}

impl PerClipColorBlurEffects {
    pub fn normalized(self) -> Self {
        Self {
            brightness: self.brightness.clamp(-1.0, 1.0),
            contrast: self.contrast.clamp(0.0, 2.0),
            saturation: self.saturation.clamp(0.0, 2.0),
            // Signed blur domain:
            // > 0 : blur sigma
            // < 0 : sharpen amount (unsharp family)
            blur_sigma: self.blur_sigma.clamp(-64.0, 64.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LayerColorBlurEffects {
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub blur_sigma: f32,
}

impl Default for LayerColorBlurEffects {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            blur_sigma: 0.0,
        }
    }
}

impl LayerColorBlurEffects {
    pub fn normalized(self) -> Self {
        Self {
            brightness: self.brightness.clamp(-1.0, 1.0),
            contrast: self.contrast.clamp(0.0, 2.0),
            saturation: self.saturation.clamp(0.0, 2.0),
            blur_sigma: self.blur_sigma.clamp(-64.0, 64.0),
        }
    }

    pub fn is_identity(self) -> bool {
        self.brightness.abs() <= 0.001
            && (self.contrast - 1.0).abs() <= 0.001
            && (self.saturation - 1.0).abs() <= 0.001
            && self.blur_sigma.abs() <= 0.001
    }
}

pub fn combine_clip_with_layer(
    clip: PerClipColorBlurEffects,
    layer: LayerColorBlurEffects,
) -> PerClipColorBlurEffects {
    PerClipColorBlurEffects {
        brightness: (clip.brightness + layer.brightness).clamp(-1.0, 1.0),
        contrast: (clip.contrast * layer.contrast).clamp(0.0, 2.0),
        saturation: (clip.saturation * layer.saturation).clamp(0.0, 2.0),
        blur_sigma: (clip.blur_sigma + layer.blur_sigma).clamp(-64.0, 64.0),
    }
}
