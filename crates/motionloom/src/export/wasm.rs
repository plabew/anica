// =========================================
// =========================================
// crates/motionloom/src/export/wasm.rs

use crate::export::{EncodeError, VideoEncoder};

/// WebCodecs-bound video encoder (scaffold).
///
/// The boundary is in place for `begin`/`push_frame`/`finish`. A full
/// implementation will wire this to `VideoEncoder`, `VideoFrame`, and
/// `EncodedVideoChunk` via `web_sys` in the browser host.
///
/// NOTE: This is intentionally a stub. It documents the intended WASM-side
/// implementation but does not yet produce valid video output.
pub struct WebCodecsVideoEncoder {
    started: bool,
}

impl WebCodecsVideoEncoder {
    pub fn new() -> Self {
        Self { started: false }
    }
}

impl VideoEncoder for WebCodecsVideoEncoder {
    fn begin(&mut self, _width: u32, _height: u32, _fps: f32) -> Result<(), EncodeError> {
        self.started = true;
        // TODO: create web_sys::VideoEncoder with H.264 config.
        Err(EncodeError::NotImplemented(
            "WebCodecs encoder is not yet implemented".to_string(),
        ))
    }

    fn push_frame(&mut self, _frame_index: u32, _rgba: &[u8]) -> Result<(), EncodeError> {
        if !self.started {
            return Err(EncodeError::NotStarted);
        }
        // TODO: draw RGBA to OffscreenCanvas/VideoFrame and encode.
        Ok(())
    }

    fn finish(&mut self) -> Result<(), EncodeError> {
        if !self.started {
            return Err(EncodeError::NotStarted);
        }
        // TODO: flush encoder and return Blob URL.
        Ok(())
    }
}
