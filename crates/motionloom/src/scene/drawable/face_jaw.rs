use crate::scene::model::{FaceJawNode, PathNode};
use crate::scene::render::{MotionLoomSceneRenderError, eval_scene_number};

pub(crate) fn face_jaw_to_path_node(
    face_jaw: &FaceJawNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<PathNode, MotionLoomSceneRenderError> {
    let cx = eval_scene_number(&face_jaw.x, time_norm, time_sec)?;
    let y = eval_scene_number(&face_jaw.y, time_norm, time_sec)?;
    let scale = eval_scene_number(&face_jaw.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
    let width = eval_scene_number(&face_jaw.width, time_norm, time_sec)?.max(0.0) * scale;
    let height = eval_scene_number(&face_jaw.height, time_norm, time_sec)?.max(0.0) * scale;
    let cheek_width =
        eval_scene_number(&face_jaw.cheek_width, time_norm, time_sec)?.max(0.0) * scale;
    let chin_width = eval_scene_number(&face_jaw.chin_width, time_norm, time_sec)?.max(0.0) * scale;
    let sharpness =
        eval_scene_number(&face_jaw.chin_sharpness, time_norm, time_sec)?.clamp(0.0, 1.0);
    let ease = eval_scene_number(&face_jaw.jaw_ease, time_norm, time_sec)?.clamp(0.0, 1.0);

    let temple_y = y + height * 0.14;
    let cheek_y = y + height * 0.68;
    let chin_y = y + height;
    let temple_half = width * 0.49;
    let cheek_half = cheek_width * 0.5;
    let chin_half = (chin_width * 0.5 * (1.0 - sharpness * 0.82)).max(width * 0.004);
    let side_bulge = width * (0.02 + ease * 0.06);
    let cheek_lift = height * (0.08 + ease * 0.08);
    let chin_ctrl_y = chin_y - height * (0.04 + (1.0 - sharpness) * 0.11);
    let top_y = y + height * (0.01 - ease * 0.035);
    let top_handle = width * (0.30 + ease * 0.15);

    let left_temple = cx - temple_half;
    let right_temple = cx + temple_half;
    let left_cheek = cx - cheek_half;
    let right_cheek = cx + cheek_half;

    let jaw_from_left = format!(
        "C {lcx:.3} {lcy1:.3} {lhx:.3} {lh_y:.3} {cx1:.3} {cy1:.3} \
         C {chin_lx:.3} {chin_ctrl_y:.3} {chin_lx:.3} {chin_ctrl_y:.3} {cx:.3} {chin_y:.3} \
         C {chin_rx:.3} {chin_ctrl_y:.3} {chin_rx:.3} {chin_ctrl_y:.3} {cx2:.3} {cy2:.3} \
         C {rhx:.3} {rh_y:.3} {rcx:.3} {rcy1:.3} {rt:.3} {ty:.3}",
        lcx = left_temple - side_bulge,
        lcy1 = temple_y + height * 0.25,
        lhx = left_cheek,
        lh_y = cheek_y - cheek_lift,
        cx1 = left_cheek,
        cy1 = cheek_y,
        chin_lx = cx - chin_half,
        chin_ctrl_y = chin_ctrl_y,
        cx = cx,
        chin_y = chin_y,
        chin_rx = cx + chin_half,
        cx2 = right_cheek,
        cy2 = cheek_y,
        rhx = right_cheek,
        rh_y = cheek_y - cheek_lift,
        rcx = right_temple + side_bulge,
        rcy1 = temple_y + height * 0.25,
        rt = right_temple,
        ty = temple_y,
    );

    let closed = scene_bool(&face_jaw.closed);
    let d = if closed {
        format!(
            "M {lt:.3} {ty:.3} \
             C {tlh:.3} {top_y:.3} {trh:.3} {top_y:.3} {rt:.3} {ty:.3} \
             C {rc1:.3} {rcy1:.3} {rhx:.3} {rh_y:.3} {rcx:.3} {cy:.3} \
             C {chin_rx:.3} {chin_ctrl_y:.3} {chin_rx:.3} {chin_ctrl_y:.3} {cx:.3} {chin_y:.3} \
             C {chin_lx:.3} {chin_ctrl_y:.3} {chin_lx:.3} {chin_ctrl_y:.3} {lcx:.3} {cy:.3} \
             C {lhx:.3} {rh_y:.3} {lc1:.3} {rcy1:.3} {lt:.3} {ty:.3} Z",
            lt = left_temple,
            ty = temple_y,
            tlh = cx - top_handle,
            top_y = top_y,
            trh = cx + top_handle,
            rt = right_temple,
            rc1 = right_temple + side_bulge,
            rcy1 = temple_y + height * 0.25,
            rhx = right_cheek,
            rh_y = cheek_y - cheek_lift,
            rcx = right_cheek,
            cy = cheek_y,
            chin_rx = cx + chin_half,
            chin_ctrl_y = chin_ctrl_y,
            cx = cx,
            chin_y = chin_y,
            chin_lx = cx - chin_half,
            lcx = left_cheek,
            lhx = left_cheek,
            lc1 = left_temple - side_bulge,
        )
    } else {
        format!("M {left_temple:.3} {temple_y:.3} {jaw_from_left}")
    };

    Ok(PathNode {
        id: face_jaw.id.clone(),
        brush: None,
        x: "0".to_string(),
        y: "0".to_string(),
        rotation: "0".to_string(),
        scale: "1".to_string(),
        scale_x: "1".to_string(),
        scale_y: "1".to_string(),
        skew_x: "0".to_string(),
        skew_y: "0".to_string(),
        transform_origin_x: "0".to_string(),
        transform_origin_y: "0".to_string(),
        d,
        stroke: face_jaw.stroke.clone(),
        fill: face_jaw.fill.clone(),
        fill_rule: "nonzero".to_string(),
        stroke_width: face_jaw.stroke_width.clone(),
        opacity: face_jaw.opacity.clone(),
        trim_start: face_jaw.trim_start.clone(),
        trim_end: face_jaw.trim_end.clone(),
        line_cap: face_jaw.line_cap.clone(),
        line_join: face_jaw.line_join.clone(),
        taper_start: face_jaw.taper_start.clone(),
        taper_end: face_jaw.taper_end.clone(),
        stroke_style: face_jaw.stroke_style.clone(),
        stroke_roughness: face_jaw.stroke_roughness.clone(),
        stroke_copies: face_jaw.stroke_copies.clone(),
        stroke_texture: face_jaw.stroke_texture.clone(),
        stroke_bristles: face_jaw.stroke_bristles.clone(),
        stroke_pressure: face_jaw.stroke_pressure.clone(),
        stroke_pressure_min: face_jaw.stroke_pressure_min.clone(),
        stroke_pressure_curve: face_jaw.stroke_pressure_curve.clone(),
        blend: face_jaw.blend.clone(),
        texture: None,
        texture_opacity: "1".to_string(),
        texture_scale: "1".to_string(),
        texture_mask: "0".to_string(),
    })
}

fn scene_bool(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "no" | "off"
    )
}
