// crates/gpui-video-renderer/src/lib.rs
mod element;

pub use element::{
    BgraGpuEffectParams, BgraProcessEffectInstance, BgraProcessParamValue, BlurMode,
    VIDEO_MAX_LOCAL_MASK_LAYERS, VideoElement, VideoLocalMaskLayer, bgra_cpu_safe_mode_notice,
    bgra_process_effects_cache_key, process_bgra_effects, process_bgra_effects_with_params,
};
