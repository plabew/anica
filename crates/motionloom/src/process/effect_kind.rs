// =========================================
// =========================================
// crates/motionloom/src/process/effect_kind.rs
// =========================================

use crate::process::pass::normalize_effect_key;

/// Canonical process effect kinds after alias resolution.
///
/// The DSL accepts multiple aliases for the same effect (e.g. `"bloom"` and
/// `"glow_bloom"` both map to `ProcessEffect::GlowBloom`). This enum provides
/// a single canonical representation so the renderer and compatibility
/// inspector do not need to repeat alias matching logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessEffect {
    HslaOverlay,
    GaussianBlur,
    GaussianBlurHorizontal,
    GaussianBlurVertical,
    GlowBloom,
    GlowStack,
    ToneMap,
    LightSweep,
    TextureOverlay,
    MagnifyLens,
}

/// Resolve a raw effect string (including aliases) to its canonical
/// `ProcessEffect`.
///
/// Returns `None` for unknown or unsupported effects.
pub fn resolve_process_effect(effect: &str) -> Option<ProcessEffect> {
    let normalized = normalize_effect_key(effect).replace(['.', '-'], "_");
    match normalized.as_str() {
        "hsla_overlay" | "hsla" | "tint_overlay" | "color_tone_hsla_overlay" => {
            Some(ProcessEffect::HslaOverlay)
        }
        "gaussian_5tap_blur" | "gaussian_blur" | "blur" => Some(ProcessEffect::GaussianBlur),
        "gaussian_5tap_h" => Some(ProcessEffect::GaussianBlurHorizontal),
        "gaussian_5tap_v" => Some(ProcessEffect::GaussianBlurVertical),
        "bloom"
        | "glow"
        | "glow_bloom"
        | "post_bloom"
        | "post_glow"
        | "post_glow_bloom"
        | "light_atmosphere_bloom"
        | "light_atmosphere_glow"
        | "light_atmosphere_glow_bloom" => Some(ProcessEffect::GlowBloom),
        "glow_stack"
        | "post_glow_stack"
        | "light_atmosphere_glow_stack"
        | "light_atmosphere_stack_glow" => Some(ProcessEffect::GlowStack),
        "tone_map" | "tonemap" | "post_tone_map" | "color_tone_tone_map" | "color_tone_tonemap" => {
            Some(ProcessEffect::ToneMap)
        }
        "light_sweep"
        | "post_light_sweep"
        | "light_atmosphere_light_sweep"
        | "light_atmosphere_sweep" => Some(ProcessEffect::LightSweep),
        "texture_overlay"
        | "post_texture_overlay"
        | "paper_texture"
        | "texture_paper"
        | "film_grain"
        | "scanlines"
        | "canvas_texture"
        | "impasto_texture"
        | "brushed_paint"
        | "stylize_look_texture_overlay" => Some(ProcessEffect::TextureOverlay),
        "magnify_lens" | "lens_magnify" | "post_magnify_lens" | "distortion_warp_magnify_lens" => {
            Some(ProcessEffect::MagnifyLens)
        }
        _ => None,
    }
}

/// Returns true if the effect is in the bloom family.
pub fn is_bloom_family(effect: &str) -> bool {
    matches!(
        resolve_process_effect(effect),
        Some(ProcessEffect::GlowBloom | ProcessEffect::GlowStack)
    )
}

/// Returns true if the effect is supported by the WASM WebGPU process path.
///
/// This list must stay in sync with the WebGPU shader and dispatcher.
/// `GlowBloom` is recognized as a canonical alias and the shader already
/// includes `ml_glow_bloom` from `color_core.wgsl`; the dispatch wiring is
/// completed in P2.
pub fn is_wasm_webgpu_compatible_effect(effect: &str) -> bool {
    matches!(
        resolve_process_effect(effect),
        Some(
            ProcessEffect::HslaOverlay
                | ProcessEffect::GaussianBlur
                | ProcessEffect::GaussianBlurHorizontal
                | ProcessEffect::GaussianBlurVertical
                | ProcessEffect::GlowBloom
                | ProcessEffect::GlowStack
                | ProcessEffect::ToneMap
                | ProcessEffect::LightSweep
                | ProcessEffect::TextureOverlay
                | ProcessEffect::MagnifyLens
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloom_alias_maps_to_glow_bloom() {
        assert_eq!(
            resolve_process_effect("bloom"),
            Some(ProcessEffect::GlowBloom)
        );
        assert_eq!(
            resolve_process_effect("glow_bloom"),
            Some(ProcessEffect::GlowBloom)
        );
        assert_eq!(
            resolve_process_effect("post.bloom"),
            Some(ProcessEffect::GlowBloom)
        );
        assert_eq!(
            resolve_process_effect("light_atmosphere.glow_bloom"),
            Some(ProcessEffect::GlowBloom)
        );
        assert_eq!(
            resolve_process_effect("glow_stack"),
            Some(ProcessEffect::GlowStack)
        );
    }

    #[test]
    fn non_aliased_effects_resolve_correctly() {
        assert_eq!(
            resolve_process_effect("hsla_overlay"),
            Some(ProcessEffect::HslaOverlay)
        );
        assert_eq!(
            resolve_process_effect("gaussian_5tap_blur"),
            Some(ProcessEffect::GaussianBlur)
        );
        assert_eq!(
            resolve_process_effect("tone_map"),
            Some(ProcessEffect::ToneMap)
        );
        assert_eq!(
            resolve_process_effect("light_sweep"),
            Some(ProcessEffect::LightSweep)
        );
        assert_eq!(
            resolve_process_effect("texture_overlay"),
            Some(ProcessEffect::TextureOverlay)
        );
        assert_eq!(
            resolve_process_effect("magnify_lens"),
            Some(ProcessEffect::MagnifyLens)
        );
    }

    #[test]
    fn is_bloom_family_detects_bloom_aliases() {
        assert!(is_bloom_family("bloom"));
        assert!(is_bloom_family("glow_bloom"));
        assert!(is_bloom_family("glow_stack"));
        assert!(is_bloom_family("post.bloom"));
        assert!(!is_bloom_family("hsla_overlay"));
    }

    #[test]
    fn wasm_webgpu_compatibility_list() {
        assert!(is_wasm_webgpu_compatible_effect("hsla_overlay"));
        assert!(is_wasm_webgpu_compatible_effect("gaussian_5tap_blur"));
        assert!(is_wasm_webgpu_compatible_effect("blur"));
        // Bloom alias is now wired to the WASM WebGPU shader (P2).
        assert!(is_wasm_webgpu_compatible_effect("bloom"));
        assert!(is_wasm_webgpu_compatible_effect("glow_bloom"));
        assert!(is_wasm_webgpu_compatible_effect("glow_stack"));
        assert!(is_wasm_webgpu_compatible_effect("tone_map"));
        assert!(is_wasm_webgpu_compatible_effect("light_sweep"));
        assert!(is_wasm_webgpu_compatible_effect("texture_overlay"));
        assert!(is_wasm_webgpu_compatible_effect("magnify_lens"));
    }
}
