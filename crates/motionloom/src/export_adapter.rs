// =========================================
// =========================================
// crates/motionloom/src/export_adapter.rs

use crate::graph::MotionGraph;

#[derive(Debug, Clone)]
pub struct ExportSample {
    pub frame: u32,
    pub zoom_scale: f32,
}

pub fn build_export_zoom_sequence(
    graph: &MotionGraph,
    clip_id: u64,
    start_frame: u32,
    end_frame: u32,
) -> Vec<ExportSample> {
    let mut out = Vec::new();
    let mut frame = start_frame;
    while frame <= end_frame {
        out.push(ExportSample {
            frame,
            zoom_scale: graph.sample_zoom_for_clip(clip_id, frame),
        });
        if frame == u32::MAX {
            break;
        }
        frame += 1;
    }
    out
}
