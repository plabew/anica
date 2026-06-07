use crate::scene::model::SceneLayerNode;
use crate::scene::spatial::{ActiveSceneCamera, Affine2};

use crate::scene::error::MotionLoomSceneRenderError;

use super::eval_scene_number;

#[derive(Clone, Copy)]
pub(super) struct SceneDepthContext<'a> {
    pub(super) active_camera: Option<ActiveSceneCamera>,
    pub(super) canvas_size: (u32, u32),
    pub(super) track_z_depth: &'a str,
}

fn scene_z_depth_scale(z_depth: f32) -> f32 {
    let depth = z_depth.clamp(-8.0, 32.0);
    if depth >= 0.0 {
        1.0 / (1.0 + depth * 0.18)
    } else {
        1.0 + (-depth) * 0.18
    }
}

pub(super) fn scene_z_depth_transform(
    active_camera: Option<ActiveSceneCamera>,
    canvas_size: (u32, u32),
    z_depth: f32,
) -> Affine2 {
    let scale = scene_z_depth_scale(z_depth);
    let (center_x, center_y) = active_camera
        .map(|camera| {
            (
                camera.viewport.x + camera.viewport.width * 0.5,
                camera.viewport.y + camera.viewport.height * 0.5,
            )
        })
        .unwrap_or((canvas_size.0 as f32 * 0.5, canvas_size.1 as f32 * 0.5));
    let perspective = Affine2::translate(center_x, center_y)
        .mul(Affine2::scale(scale))
        .mul(Affine2::translate(-center_x, -center_y));
    if let Some(camera) = active_camera {
        perspective.mul(camera.transform)
    } else {
        perspective
    }
}

pub(super) fn scene_layer_effective_z_depth(
    layer: &SceneLayerNode,
    depth: SceneDepthContext<'_>,
    time_norm: f32,
    time_sec: f32,
) -> Result<f32, MotionLoomSceneRenderError> {
    let value = layer.z_depth.as_deref().unwrap_or(depth.track_z_depth);
    eval_scene_number(value, time_norm, time_sec)
}

pub(super) fn scene_depth_track_sort_key(
    z_depth_value: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<f32, MotionLoomSceneRenderError> {
    eval_scene_number(z_depth_value, time_norm, time_sec)
}
