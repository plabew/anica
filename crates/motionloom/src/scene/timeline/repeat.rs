use crate::scene::render::{MotionLoomSceneRenderError, eval_scene_number};

pub(crate) fn eval_repeat_count(
    expr: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<u32, MotionLoomSceneRenderError> {
    Ok(eval_scene_number(expr, time_norm, time_sec)?
        .round()
        .clamp(0.0, 1000.0) as u32)
}
