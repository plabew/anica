use crate::scene::model::{PrecomposeNode, SceneLayerNode, SceneSequenceNode};
use crate::scene::render::{MotionLoomSceneRenderError, eval_scene_number};

pub(crate) fn scene_sequence_local_time(
    sequence: &SceneSequenceNode,
    start_override_ms: Option<i64>,
    scene_time_sec: f32,
) -> Option<(f32, f32)> {
    let base_ms = start_override_ms.unwrap_or(0);
    let start_ms = base_ms + sequence.from_ms as i64;
    let duration_ms = sequence.duration_ms.max(1) as i64;
    let now_ms = (scene_time_sec.max(0.0) * 1000.0).round() as i64;
    if now_ms < start_ms {
        return None;
    }
    let elapsed_ms = now_ms - start_ms;
    if elapsed_ms > duration_ms {
        if sequence.out.trim().eq_ignore_ascii_case("hold") {
            return Some((1.0, duration_ms as f32 / 1000.0));
        }
        return None;
    }
    let local_norm = (elapsed_ms as f32 / duration_ms as f32).clamp(0.0, 1.0);
    Some((local_norm, elapsed_ms as f32 / 1000.0))
}

pub(crate) fn scene_layer_source_time(
    layer: &SceneLayerNode,
    precompose: &PrecomposeNode,
    parent_time_norm: f32,
    parent_time_sec: f32,
) -> Result<Option<(f32, f32)>, MotionLoomSceneRenderError> {
    let playback_rate = eval_scene_number(&layer.playback_rate, parent_time_norm, parent_time_sec)?;
    let source_mode = layer.source_time.trim().to_ascii_lowercase();
    let base_sec = match source_mode.as_str() {
        "" | "local" | "relative" | "parent" | "global" | "scene" | "absolute" => parent_time_sec,
        _ => parent_time_sec,
    };
    let mut source_sec = (base_sec - layer.time_offset_ms as f32 / 1000.0) * playback_rate;
    let out = layer.out.trim().to_ascii_lowercase().replace('_', "-");

    if let Some(duration_ms) = precompose.duration_ms {
        let duration_sec = (duration_ms as f32 / 1000.0).max(0.000_001);
        if source_sec < 0.0 || source_sec > duration_sec {
            match out.as_str() {
                "loop" | "repeat" => {
                    source_sec = source_sec.rem_euclid(duration_sec);
                }
                "none" | "hidden" | "transparent" => return Ok(None),
                _ => {
                    source_sec = source_sec.clamp(0.0, duration_sec);
                }
            }
        }
        return Ok(Some((
            (source_sec / duration_sec).clamp(0.0, 1.0),
            source_sec,
        )));
    }

    if source_sec < 0.0 && matches!(out.as_str(), "none" | "hidden" | "transparent") {
        return Ok(None);
    }
    Ok(Some((parent_time_norm, source_sec.max(0.0))))
}
