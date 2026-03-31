// crates/gpui-video-renderer/src/lib.rs
mod element;

pub use element::{
    VIDEO_MAX_LOCAL_MASK_LAYERS, VideoElement, VideoLocalMaskLayer, bgra_cpu_safe_mode_notice,
    process_bgra_effects,
};
