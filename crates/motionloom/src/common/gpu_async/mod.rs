// =========================================
// =========================================
// crates/motionloom/src/common/gpu_async/mod.rs

#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(target_arch = "wasm32")]
pub use wasm::*;
