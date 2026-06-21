use crate::scene::model::{
    CharacterNode, CircleNode, GroupNode, LineNode, PathNode, PolylineNode, RectNode,
    SceneLayerNode, UseNode,
};
use crate::scene::render::{MotionLoomSceneRenderError, eval_scene_number};
use crate::scene::text::TextNode;

#[derive(Debug, Clone, Copy)]
pub(crate) struct CameraRect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ActiveSceneCamera {
    pub(crate) transform: Affine2,
    pub(crate) viewport: CameraRect,
    pub(crate) layer_width: u32,
    pub(crate) layer_height: u32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TextureRect {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Affine2 {
    pub(crate) m00: f32,
    pub(crate) m01: f32,
    pub(crate) m02: f32,
    pub(crate) m10: f32,
    pub(crate) m11: f32,
    pub(crate) m12: f32,
}

impl Affine2 {
    pub(crate) const fn identity() -> Self {
        Self {
            m00: 1.0,
            m01: 0.0,
            m02: 0.0,
            m10: 0.0,
            m11: 1.0,
            m12: 0.0,
        }
    }

    pub(crate) const fn translate(x: f32, y: f32) -> Self {
        Self {
            m00: 1.0,
            m01: 0.0,
            m02: x,
            m10: 0.0,
            m11: 1.0,
            m12: y,
        }
    }

    pub(crate) fn rotate_deg(deg: f32) -> Self {
        let (sin_t, cos_t) = deg.to_radians().sin_cos();
        Self {
            m00: cos_t,
            m01: -sin_t,
            m02: 0.0,
            m10: sin_t,
            m11: cos_t,
            m12: 0.0,
        }
    }

    pub(crate) const fn scale(scale: f32) -> Self {
        Self {
            m00: scale,
            m01: 0.0,
            m02: 0.0,
            m10: 0.0,
            m11: scale,
            m12: 0.0,
        }
    }

    pub(crate) const fn scale_xy(scale_x: f32, scale_y: f32) -> Self {
        Self {
            m00: scale_x,
            m01: 0.0,
            m02: 0.0,
            m10: 0.0,
            m11: scale_y,
            m12: 0.0,
        }
    }

    pub(crate) fn skew_deg(skew_x: f32, skew_y: f32) -> Self {
        let tx = skew_x.clamp(-89.9, 89.9).to_radians().tan();
        let ty = skew_y.clamp(-89.9, 89.9).to_radians().tan();
        Self {
            m00: 1.0,
            m01: tx,
            m02: 0.0,
            m10: ty,
            m11: 1.0,
            m12: 0.0,
        }
    }

    pub(crate) fn mul(self, rhs: Self) -> Self {
        Self {
            m00: self.m00 * rhs.m00 + self.m01 * rhs.m10,
            m01: self.m00 * rhs.m01 + self.m01 * rhs.m11,
            m02: self.m00 * rhs.m02 + self.m01 * rhs.m12 + self.m02,
            m10: self.m10 * rhs.m00 + self.m11 * rhs.m10,
            m11: self.m10 * rhs.m01 + self.m11 * rhs.m11,
            m12: self.m10 * rhs.m02 + self.m11 * rhs.m12 + self.m12,
        }
    }

    pub(crate) fn inverse(self) -> Option<Self> {
        let det = self.m00 * self.m11 - self.m01 * self.m10;
        if det.abs() <= 0.000001 {
            return None;
        }
        let inv_det = 1.0 / det;
        let m00 = self.m11 * inv_det;
        let m01 = -self.m01 * inv_det;
        let m10 = -self.m10 * inv_det;
        let m11 = self.m00 * inv_det;
        Some(Self {
            m00,
            m01,
            m02: -(m00 * self.m02 + m01 * self.m12),
            m10,
            m11,
            m12: -(m10 * self.m02 + m11 * self.m12),
        })
    }

    pub(crate) fn transform_point(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.m00 * x + self.m01 * y + self.m02,
            self.m10 * x + self.m11 * y + self.m12,
        )
    }
}

pub(crate) fn clamp_nonzero_signed_scale(value: f32) -> f32 {
    if value.abs() < 0.001 {
        if value.is_sign_negative() {
            -0.001
        } else {
            0.001
        }
    } else {
        value.clamp(-64.0, 64.0)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn scene_local_transform(
    x_value: &str,
    y_value: &str,
    rotation_value: &str,
    scale_value: &str,
    scale_x_value: &str,
    scale_y_value: &str,
    skew_x_value: &str,
    skew_y_value: &str,
    transform_origin_x_value: &str,
    transform_origin_y_value: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    let x = eval_scene_number(x_value, time_norm, time_sec)?;
    let y = eval_scene_number(y_value, time_norm, time_sec)?;
    let rotation = eval_scene_number(rotation_value, time_norm, time_sec)?;
    let scale = eval_scene_number(scale_value, time_norm, time_sec)?.clamp(0.001, 64.0);
    let scale_x =
        clamp_nonzero_signed_scale(scale * eval_scene_number(scale_x_value, time_norm, time_sec)?);
    let scale_y =
        clamp_nonzero_signed_scale(scale * eval_scene_number(scale_y_value, time_norm, time_sec)?);
    let skew_x = eval_scene_number(skew_x_value, time_norm, time_sec)?;
    let skew_y = eval_scene_number(skew_y_value, time_norm, time_sec)?;
    let origin_x = eval_scene_number(transform_origin_x_value, time_norm, time_sec)?;
    let origin_y = eval_scene_number(transform_origin_y_value, time_norm, time_sec)?;

    Ok(Affine2::translate(x, y)
        .mul(Affine2::translate(origin_x, origin_y))
        .mul(Affine2::rotate_deg(rotation))
        .mul(Affine2::skew_deg(skew_x, skew_y))
        .mul(Affine2::scale_xy(scale_x, scale_y))
        .mul(Affine2::translate(-origin_x, -origin_y)))
}

pub(crate) fn scene_group_local_transform(
    group: &GroupNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    scene_local_transform(
        &group.x,
        &group.y,
        &group.rotation,
        &group.scale,
        &group.scale_x,
        &group.scale_y,
        &group.skew_x,
        &group.skew_y,
        &group.transform_origin_x,
        &group.transform_origin_y,
        time_norm,
        time_sec,
    )
}

pub(crate) fn scene_group_local_transform_opt(
    group: &GroupNode,
    time_norm: f32,
    time_sec: f32,
) -> Option<Affine2> {
    scene_group_local_transform(group, time_norm, time_sec).ok()
}

pub(crate) fn scene_layer_local_transform(
    layer: &SceneLayerNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    let base = scene_local_transform(
        &layer.x,
        &layer.y,
        &layer.rotation,
        &layer.scale,
        &layer.scale_x,
        &layer.scale_y,
        &layer.skew_x,
        &layer.skew_y,
        &layer.transform_origin_x,
        &layer.transform_origin_y,
        time_norm,
        time_sec,
    )?;
    if !layer.is_3d {
        return Ok(base);
    }

    // Scene Layer3D is AE-like 2.5D: flat 2D content receives depth scale and
    // rotation foreshortening, but it remains a composited scene layer.
    let z = eval_scene_number(&layer.z, time_norm, time_sec)?;
    let rotation_x = eval_scene_number(&layer.rotation_x, time_norm, time_sec)?;
    let rotation_y = eval_scene_number(&layer.rotation_y, time_norm, time_sec)?;
    let perspective = eval_scene_number(&layer.perspective, time_norm, time_sec)?
        .abs()
        .clamp(1.0, 100_000.0);
    let depth_scale = (perspective / (perspective + z).max(1.0)).clamp(0.01, 64.0);
    let (sin_x, cos_x) = rotation_x.to_radians().sin_cos();
    let (sin_y, cos_y) = rotation_y.to_radians().sin_cos();
    let foreshorten_x = clamp_nonzero_signed_scale(cos_y * depth_scale);
    let foreshorten_y = clamp_nonzero_signed_scale(cos_x * depth_scale);
    let skew_x = (-sin_y * 0.34 * depth_scale).clamp(-2.5, 2.5);
    let skew_y = (sin_x * 0.34 * depth_scale).clamp(-2.5, 2.5);
    Ok(base.mul(Affine2 {
        m00: foreshorten_x,
        m01: skew_x,
        m02: 0.0,
        m10: skew_y,
        m11: foreshorten_y,
        m12: 0.0,
    }))
}

pub(crate) fn scene_use_local_transform(
    use_node: &UseNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    scene_local_transform(
        &use_node.x,
        &use_node.y,
        &use_node.rotation,
        &use_node.scale,
        &use_node.scale_x,
        &use_node.scale_y,
        &use_node.skew_x,
        &use_node.skew_y,
        &use_node.transform_origin_x,
        &use_node.transform_origin_y,
        time_norm,
        time_sec,
    )
}

pub(crate) fn scene_character_local_transform(
    character: &CharacterNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    scene_local_transform(
        &character.x,
        &character.y,
        &character.rotation,
        &character.scale,
        &character.scale_x,
        &character.scale_y,
        &character.skew_x,
        &character.skew_y,
        &character.transform_origin_x,
        &character.transform_origin_y,
        time_norm,
        time_sec,
    )
}

pub(crate) fn scene_character_local_transform_opt(
    character: &CharacterNode,
    time_norm: f32,
    time_sec: f32,
) -> Option<Affine2> {
    scene_character_local_transform(character, time_norm, time_sec).ok()
}

pub(crate) fn scene_rect_local_transform(
    rect: &RectNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    scene_local_transform(
        "0",
        "0",
        &rect.rotation,
        &rect.scale,
        &rect.scale_x,
        &rect.scale_y,
        &rect.skew_x,
        &rect.skew_y,
        &rect.transform_origin_x,
        &rect.transform_origin_y,
        time_norm,
        time_sec,
    )
}

pub(crate) fn scene_circle_local_transform(
    circle: &CircleNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    scene_local_transform(
        "0",
        "0",
        &circle.rotation,
        &circle.scale,
        &circle.scale_x,
        &circle.scale_y,
        &circle.skew_x,
        &circle.skew_y,
        &circle.transform_origin_x,
        &circle.transform_origin_y,
        time_norm,
        time_sec,
    )
}

pub(crate) fn scene_line_local_transform(
    line: &LineNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    scene_local_transform(
        &line.x,
        &line.y,
        &line.rotation,
        &line.scale,
        &line.scale_x,
        &line.scale_y,
        &line.skew_x,
        &line.skew_y,
        &line.transform_origin_x,
        &line.transform_origin_y,
        time_norm,
        time_sec,
    )
}

pub(crate) fn scene_polyline_local_transform(
    polyline: &PolylineNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    scene_local_transform(
        &polyline.x,
        &polyline.y,
        &polyline.rotation,
        &polyline.scale,
        &polyline.scale_x,
        &polyline.scale_y,
        &polyline.skew_x,
        &polyline.skew_y,
        &polyline.transform_origin_x,
        &polyline.transform_origin_y,
        time_norm,
        time_sec,
    )
}

pub(crate) fn scene_path_local_transform(
    path: &PathNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    scene_local_transform(
        &path.x,
        &path.y,
        &path.rotation,
        &path.scale,
        &path.scale_x,
        &path.scale_y,
        &path.skew_x,
        &path.skew_y,
        &path.transform_origin_x,
        &path.transform_origin_y,
        time_norm,
        time_sec,
    )
}

pub(crate) fn scene_text_local_transform(
    text: &TextNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    scene_local_transform(
        "0",
        "0",
        &text.rotation,
        &text.scale,
        &text.scale_x,
        &text.scale_y,
        &text.skew_x,
        &text.skew_y,
        &text.transform_origin_x,
        &text.transform_origin_y,
        time_norm,
        time_sec,
    )
}

pub(crate) fn affine_is_identity(transform: Affine2) -> bool {
    (transform.m00 - 1.0).abs() <= 0.0001
        && transform.m01.abs() <= 0.0001
        && transform.m02.abs() <= 0.0001
        && transform.m10.abs() <= 0.0001
        && (transform.m11 - 1.0).abs() <= 0.0001
        && transform.m12.abs() <= 0.0001
}

pub(crate) fn resolve_axis(
    raw: &str,
    canvas_extent: f32,
    content_extent: f32,
    time_norm: f32,
    time_sec: f32,
) -> Result<f32, MotionLoomSceneRenderError> {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    let value = match lower.as_str() {
        "center" | "middle" => ((canvas_extent - content_extent) * 0.5).max(0.0),
        "left" | "top" => 0.0,
        "right" | "bottom" => (canvas_extent - content_extent).max(0.0),
        _ => {
            if let Some(percent) = lower.strip_suffix('%')
                && let Ok(value) = percent.trim().parse::<f32>()
            {
                return Ok((canvas_extent - content_extent) * (value / 100.0));
            }
            trimmed
                .parse::<f32>()
                .or_else(|_| eval_scene_number(trimmed, time_norm, time_sec))?
        }
    };
    Ok(value)
}
