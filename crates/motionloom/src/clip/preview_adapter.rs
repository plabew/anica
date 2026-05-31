// =========================================
// =========================================
// crates/motionloom/src/preview_adapter.rs

use crate::backend::OutputFormat;
use crate::graph::MotionGraph;

#[derive(Debug, Clone)]
pub struct PreviewSample {
    pub zoom_scale: f32,
    pub output_format: OutputFormat,
}

pub fn sample_preview_zoom(
    graph: &MotionGraph,
    clip_id: u64,
    frame: u32,
    output_format: OutputFormat,
) -> PreviewSample {
    PreviewSample {
        zoom_scale: graph.sample_zoom_for_clip(clip_id, frame),
        output_format,
    }
}
