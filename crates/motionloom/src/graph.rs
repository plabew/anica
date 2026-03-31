// =========================================
// =========================================
// crates/motionloom/src/graph.rs

use crate::eval::sample_anim_f32;
use crate::model::ClipZoomSpec;

#[derive(Debug, Clone)]
pub struct MotionGraph {
    pub fps: f32,
    pub zoom_specs: Vec<ClipZoomSpec>,
}

impl MotionGraph {
    pub fn new(fps: f32) -> Self {
        Self {
            fps,
            zoom_specs: Vec::new(),
        }
    }

    pub fn sample_zoom_for_clip(&self, clip_id: u64, frame: u32) -> f32 {
        for spec in &self.zoom_specs {
            if let Some(target_id) = spec.clip_id
                && target_id != clip_id
            {
                continue;
            }
            if frame < spec.start_frame || frame > spec.end_frame {
                continue;
            }
            return sample_anim_f32(&spec.zoom.scale, frame);
        }
        1.0
    }
}
