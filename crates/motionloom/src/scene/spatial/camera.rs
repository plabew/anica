use crate::scene::model::{CameraNode, SceneNode, SceneTrackNode};
use crate::scene::render::{MotionLoomSceneRenderError, eval_scene_number};
use crate::scene::timeline::scene_sequence_local_time;

use super::{
    ActiveSceneCamera, Affine2, CameraRect, resolve_axis, scene_character_local_transform_opt,
    scene_group_local_transform_opt, scene_layer_local_transform,
};

pub(crate) fn camera_transform(
    camera: &CameraNode,
    camera_children: &[SceneNode],
    canvas_w: u32,
    canvas_h: u32,
    time_norm: f32,
    time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    camera_transform_with_times(
        camera,
        camera_children,
        canvas_w,
        canvas_h,
        time_norm,
        time_sec,
        time_norm,
        time_sec,
    )
}

#[allow(clippy::too_many_arguments)]
fn camera_transform_with_times(
    camera: &CameraNode,
    camera_children: &[SceneNode],
    canvas_w: u32,
    canvas_h: u32,
    camera_time_norm: f32,
    camera_time_sec: f32,
    target_time_norm: f32,
    target_time_sec: f32,
) -> Result<Affine2, MotionLoomSceneRenderError> {
    let viewport = camera_viewport(
        camera,
        canvas_w,
        canvas_h,
        camera_time_norm,
        camera_time_sec,
    )?;
    let zoom =
        eval_scene_number(&camera.zoom, camera_time_norm, camera_time_sec)?.clamp(0.001, 1024.0);
    let rotation = eval_scene_number(&camera.rotation, camera_time_norm, camera_time_sec)?;
    let base_anchor_x = resolve_camera_anchor(
        &camera.anchor_x,
        viewport.x,
        viewport.width,
        camera_time_norm,
        camera_time_sec,
    )?;
    let base_anchor_y = resolve_camera_anchor(
        &camera.anchor_y,
        viewport.y,
        viewport.height,
        camera_time_norm,
        camera_time_sec,
    )?;
    let (x, y) = camera_target(
        camera,
        camera_children,
        canvas_w,
        canvas_h,
        viewport,
        zoom,
        base_anchor_x,
        base_anchor_y,
        camera_time_norm,
        camera_time_sec,
        target_time_norm,
        target_time_sec,
    )?;
    let offset_x = eval_scene_number(&camera.offset_x, camera_time_norm, camera_time_sec)?
        + eval_scene_number(&camera.shake_x, camera_time_norm, camera_time_sec)?;
    let offset_y = eval_scene_number(&camera.offset_y, camera_time_norm, camera_time_sec)?
        + eval_scene_number(&camera.shake_y, camera_time_norm, camera_time_sec)?;
    let anchor_x = base_anchor_x + offset_x;
    let anchor_y = base_anchor_y + offset_y;

    Ok(camera_transform_from_values(
        x, y, anchor_x, anchor_y, zoom, rotation,
    ))
}

pub(crate) fn active_scene_camera_from_tracks(
    tracks: &[&SceneTrackNode],
    canvas_w: u32,
    canvas_h: u32,
    time_norm: f32,
    time_sec: f32,
) -> Result<Option<ActiveSceneCamera>, MotionLoomSceneRenderError> {
    let Some((camera, camera_time_norm, camera_time_sec)) =
        active_scene_camera_node(tracks, time_sec)
    else {
        return Ok(None);
    };
    let mut target_nodes = Vec::<SceneNode>::new();
    for track in tracks {
        if is_scene_camera_track(track) || !is_scene_world_track(track) {
            continue;
        }
        target_nodes.extend(track.children.iter().cloned());
    }
    let transform = camera_transform_with_times(
        camera,
        &target_nodes,
        canvas_w,
        canvas_h,
        camera_time_norm,
        camera_time_sec,
        time_norm,
        time_sec,
    )?;
    let viewport = camera_viewport(
        camera,
        canvas_w,
        canvas_h,
        camera_time_norm,
        camera_time_sec,
    )?;
    let world_bounds = camera_world_bounds(
        camera,
        canvas_w,
        canvas_h,
        camera_time_norm,
        camera_time_sec,
    )?;
    let layer_width = world_bounds
        .map(|rect| canvas_w.max((rect.x + rect.width).ceil().max(canvas_w as f32) as u32 + 2))
        .unwrap_or_else(|| canvas_w.saturating_add(2));
    let layer_height = world_bounds
        .map(|rect| canvas_h.max((rect.y + rect.height).ceil().max(canvas_h as f32) as u32 + 2))
        .unwrap_or_else(|| canvas_h.saturating_add(2));
    Ok(Some(ActiveSceneCamera {
        transform,
        viewport,
        layer_width,
        layer_height,
    }))
}

fn active_scene_camera_node<'a>(
    tracks: &'a [&'a SceneTrackNode],
    scene_time_sec: f32,
) -> Option<(&'a CameraNode, f32, f32)> {
    let mut active = None;
    for track in tracks {
        if !is_scene_camera_track(track) {
            continue;
        }
        for child in &track.children {
            let SceneNode::Sequence(sequence) = child else {
                continue;
            };
            if let Some((local_norm, local_sec)) =
                scene_sequence_local_time(sequence, None, scene_time_sec)
                && let Some(SceneNode::Camera(camera)) = sequence.children.first()
            {
                active = Some((camera, local_norm, local_sec));
            }
        }
    }
    active
}

pub(crate) fn is_scene_camera_track(track: &SceneTrackNode) -> bool {
    track
        .role
        .as_deref()
        .is_some_and(|role| role.eq_ignore_ascii_case("camera"))
}

pub(crate) fn is_scene_world_track(track: &SceneTrackNode) -> bool {
    track.space.trim().eq_ignore_ascii_case("world")
}

fn camera_transform_from_values(
    x: f32,
    y: f32,
    anchor_x: f32,
    anchor_y: f32,
    zoom: f32,
    rotation: f32,
) -> Affine2 {
    Affine2::translate(anchor_x, anchor_y)
        .mul(Affine2::rotate_deg(rotation))
        .mul(Affine2::scale(zoom))
        .mul(Affine2::translate(-x, -y))
}

#[allow(clippy::too_many_arguments)]
fn camera_target(
    camera: &CameraNode,
    camera_children: &[SceneNode],
    canvas_w: u32,
    canvas_h: u32,
    viewport: CameraRect,
    zoom: f32,
    anchor_x: f32,
    anchor_y: f32,
    camera_time_norm: f32,
    camera_time_sec: f32,
    target_time_norm: f32,
    target_time_sec: f32,
) -> Result<(f32, f32), MotionLoomSceneRenderError> {
    let mut target = (
        resolve_axis(
            &camera.x,
            canvas_w as f32,
            0.0,
            camera_time_norm,
            camera_time_sec,
        )?,
        resolve_axis(
            &camera.y,
            canvas_h as f32,
            0.0,
            camera_time_norm,
            camera_time_sec,
        )?,
    );
    let followed = camera.follow.as_deref().and_then(|id| {
        find_scene_node_anchor(
            camera_children,
            id,
            Affine2::identity(),
            target_time_norm,
            target_time_sec,
        )
    });
    if let Some(followed) = followed {
        target = if let Some(dead_zone) = camera.dead_zone.as_deref() {
            camera_target_with_dead_zone(
                target,
                followed,
                dead_zone,
                viewport,
                zoom,
                anchor_x,
                anchor_y,
                camera_time_norm,
                camera_time_sec,
            )?
        } else {
            followed
        };
    }
    if let Some(target_x) = camera.target_x.as_deref() {
        target.0 = resolve_axis(
            target_x,
            canvas_w as f32,
            0.0,
            camera_time_norm,
            camera_time_sec,
        )?;
    }
    if let Some(target_y) = camera.target_y.as_deref() {
        target.1 = resolve_axis(
            target_y,
            canvas_h as f32,
            0.0,
            camera_time_norm,
            camera_time_sec,
        )?;
    }
    if let Some(bounds) = camera_world_bounds(
        camera,
        canvas_w,
        canvas_h,
        camera_time_norm,
        camera_time_sec,
    )? {
        target = clamp_camera_target_to_bounds(target, bounds, viewport, zoom);
    }
    Ok(target)
}

#[allow(clippy::too_many_arguments)]
fn camera_target_with_dead_zone(
    mut target: (f32, f32),
    followed: (f32, f32),
    dead_zone_raw: &str,
    viewport: CameraRect,
    zoom: f32,
    anchor_x: f32,
    anchor_y: f32,
    time_norm: f32,
    time_sec: f32,
) -> Result<(f32, f32), MotionLoomSceneRenderError> {
    let dead_zone = parse_camera_rect_expr(dead_zone_raw, time_norm, time_sec)?;
    let min_screen_x = viewport.x + dead_zone.x;
    let max_screen_x = min_screen_x + dead_zone.width;
    let min_screen_y = viewport.y + dead_zone.y;
    let max_screen_y = min_screen_y + dead_zone.height;

    let followed_screen_x = anchor_x + (followed.0 - target.0) * zoom;
    let followed_screen_y = anchor_y + (followed.1 - target.1) * zoom;
    if followed_screen_x < min_screen_x {
        target.0 = followed.0 - (min_screen_x - anchor_x) / zoom;
    } else if followed_screen_x > max_screen_x {
        target.0 = followed.0 - (max_screen_x - anchor_x) / zoom;
    }
    if followed_screen_y < min_screen_y {
        target.1 = followed.1 - (min_screen_y - anchor_y) / zoom;
    } else if followed_screen_y > max_screen_y {
        target.1 = followed.1 - (max_screen_y - anchor_y) / zoom;
    }
    Ok(target)
}

fn clamp_camera_target_to_bounds(
    target: (f32, f32),
    bounds: CameraRect,
    viewport: CameraRect,
    zoom: f32,
) -> (f32, f32) {
    let half_w = viewport.width / zoom * 0.5;
    let half_h = viewport.height / zoom * 0.5;
    let min_x = bounds.x + half_w;
    let max_x = bounds.x + bounds.width - half_w;
    let min_y = bounds.y + half_h;
    let max_y = bounds.y + bounds.height - half_h;
    let x = if min_x <= max_x {
        target.0.clamp(min_x, max_x)
    } else {
        bounds.x + bounds.width * 0.5
    };
    let y = if min_y <= max_y {
        target.1.clamp(min_y, max_y)
    } else {
        bounds.y + bounds.height * 0.5
    };
    (x, y)
}

pub(crate) fn camera_viewport(
    camera: &CameraNode,
    canvas_w: u32,
    canvas_h: u32,
    time_norm: f32,
    time_sec: f32,
) -> Result<CameraRect, MotionLoomSceneRenderError> {
    if let Some(viewport) = camera.viewport.as_deref() {
        return parse_camera_rect_expr(viewport, time_norm, time_sec);
    }
    Ok(CameraRect {
        x: 0.0,
        y: 0.0,
        width: canvas_w as f32,
        height: canvas_h as f32,
    })
}

pub(crate) fn camera_world_bounds(
    camera: &CameraNode,
    _canvas_w: u32,
    _canvas_h: u32,
    time_norm: f32,
    time_sec: f32,
) -> Result<Option<CameraRect>, MotionLoomSceneRenderError> {
    camera
        .world_bounds
        .as_deref()
        .map(|bounds| parse_camera_rect_expr(bounds, time_norm, time_sec))
        .transpose()
}

fn parse_camera_rect_expr(
    raw: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<CameraRect, MotionLoomSceneRenderError> {
    let inner = raw
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err(MotionLoomSceneRenderError::InvalidExpression {
            expr: raw.to_string(),
            message: "camera rect must use x,y,width,height".to_string(),
        });
    }
    Ok(CameraRect {
        x: eval_scene_number(parts[0], time_norm, time_sec)?,
        y: eval_scene_number(parts[1], time_norm, time_sec)?,
        width: eval_scene_number(parts[2], time_norm, time_sec)?.max(1.0),
        height: eval_scene_number(parts[3], time_norm, time_sec)?.max(1.0),
    })
}

fn resolve_camera_anchor(
    raw: &str,
    viewport_origin: f32,
    viewport_extent: f32,
    time_norm: f32,
    time_sec: f32,
) -> Result<f32, MotionLoomSceneRenderError> {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    let offset = match lower.as_str() {
        "center" | "middle" => viewport_extent * 0.5,
        "left" | "top" => 0.0,
        "right" | "bottom" => viewport_extent,
        _ => {
            if let Some(percent) = lower.strip_suffix('%')
                && let Ok(value) = percent.trim().parse::<f32>()
            {
                viewport_extent * value / 100.0
            } else {
                let value = trimmed
                    .parse::<f32>()
                    .or_else(|_| eval_scene_number(trimmed, time_norm, time_sec))?;
                if (-1.0..=1.0).contains(&value) {
                    viewport_extent * value
                } else {
                    value
                }
            }
        }
    };
    Ok(viewport_origin + offset)
}

pub(crate) fn find_scene_node_anchor(
    nodes: &[SceneNode],
    id: &str,
    transform: Affine2,
    time_norm: f32,
    time_sec: f32,
) -> Option<(f32, f32)> {
    for node in nodes {
        match node {
            SceneNode::Timeline(timeline) => {
                let mut tracks = timeline
                    .children
                    .iter()
                    .filter_map(|node| match node {
                        SceneNode::Track(track) => Some(track),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                tracks.sort_by_key(|track| track.z);
                for track in tracks {
                    if let Some(point) =
                        find_scene_node_anchor(&track.children, id, transform, time_norm, time_sec)
                    {
                        return Some(point);
                    }
                }
            }
            SceneNode::Track(track) => {
                if let Some(point) =
                    find_scene_node_anchor(&track.children, id, transform, time_norm, time_sec)
                {
                    return Some(point);
                }
            }
            SceneNode::Sequence(sequence) => {
                if let Some((local_norm, local_sec)) =
                    scene_sequence_local_time(sequence, None, time_sec)
                    && let Some(point) = find_scene_node_anchor(
                        &sequence.children,
                        id,
                        transform,
                        local_norm,
                        local_sec,
                    )
                {
                    return Some(point);
                }
            }
            SceneNode::Chain(chain) => {
                let mut cursor_ms = chain.from_ms as i64;
                for child in &chain.children {
                    if let SceneNode::Sequence(sequence) = child {
                        if let Some((local_norm, local_sec)) =
                            scene_sequence_local_time(sequence, Some(cursor_ms), time_sec)
                            && let Some(point) = find_scene_node_anchor(
                                &sequence.children,
                                id,
                                transform,
                                local_norm,
                                local_sec,
                            )
                        {
                            return Some(point);
                        }
                        cursor_ms += sequence.duration_ms as i64 + chain.gap_ms;
                    }
                }
            }
            SceneNode::Rect(rect) if rect.id.as_deref() == Some(id) => {
                let x = eval_scene_number(&rect.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&rect.y, time_norm, time_sec).ok()?;
                let w = eval_scene_number(&rect.width, time_norm, time_sec).ok()?;
                let h = eval_scene_number(&rect.height, time_norm, time_sec).ok()?;
                return Some(transform.transform_point(x + w * 0.5, y + h * 0.5));
            }
            SceneNode::Circle(circle) if circle.id.as_deref() == Some(id) => {
                let x = eval_scene_number(&circle.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&circle.y, time_norm, time_sec).ok()?;
                return Some(transform.transform_point(x, y));
            }
            SceneNode::FaceJaw(face_jaw) if face_jaw.id.as_deref() == Some(id) => {
                let x = eval_scene_number(&face_jaw.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&face_jaw.y, time_norm, time_sec).ok()?;
                let h = eval_scene_number(&face_jaw.height, time_norm, time_sec).ok()?;
                return Some(transform.transform_point(x, y + h * 0.5));
            }
            SceneNode::Group(group) => {
                let group_transform =
                    transform.mul(scene_group_local_transform_opt(group, time_norm, time_sec)?);
                if group.id.as_deref() == Some(id) {
                    return Some(group_transform.transform_point(0.0, 0.0));
                }
                if let Some(point) = find_scene_node_anchor(
                    &group.children,
                    id,
                    group_transform,
                    time_norm,
                    time_sec,
                ) {
                    return Some(point);
                }
            }
            SceneNode::Part(part) => {
                let x = eval_scene_number(&part.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&part.y, time_norm, time_sec).ok()?;
                let rotation = eval_scene_number(&part.rotation, time_norm, time_sec).ok()?;
                let scale = eval_scene_number(&part.scale, time_norm, time_sec)
                    .ok()?
                    .clamp(0.001, 64.0);
                let anchor_x = eval_scene_number(&part.anchor_x, time_norm, time_sec).ok()?;
                let anchor_y = eval_scene_number(&part.anchor_y, time_norm, time_sec).ok()?;
                let part_transform = transform
                    .mul(Affine2::translate(x, y))
                    .mul(Affine2::rotate_deg(rotation))
                    .mul(Affine2::scale(scale))
                    .mul(Affine2::translate(-anchor_x, -anchor_y));
                if part.id.as_deref() == Some(id) {
                    return Some(part_transform.transform_point(0.0, 0.0));
                }
                if let Some(point) =
                    find_scene_node_anchor(&part.children, id, part_transform, time_norm, time_sec)
                {
                    return Some(point);
                }
            }
            SceneNode::Text(text) if text.id.as_deref() == Some(id) => {
                let x = eval_scene_number(&text.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&text.y, time_norm, time_sec).ok()?;
                return Some(transform.transform_point(x, y));
            }
            SceneNode::Image(image) if image.id.as_deref() == Some(id) => {
                let x = eval_scene_number(&image.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&image.y, time_norm, time_sec).ok()?;
                return Some(transform.transform_point(x, y));
            }
            SceneNode::Svg(svg) if svg.id.as_deref() == Some(id) => {
                let x = eval_scene_number(&svg.x, time_norm, time_sec).ok()?;
                let y = eval_scene_number(&svg.y, time_norm, time_sec).ok()?;
                return Some(transform.transform_point(x, y));
            }
            SceneNode::Camera(camera) => {
                if let Some(point) =
                    find_scene_node_anchor(&camera.children, id, transform, time_norm, time_sec)
                {
                    return Some(point);
                }
            }
            SceneNode::Character(character) => {
                let character_transform = transform.mul(scene_character_local_transform_opt(
                    character, time_norm, time_sec,
                )?);
                if character.id.as_deref() == Some(id) {
                    return Some(character_transform.transform_point(0.0, 0.0));
                }
                if let Some(point) = find_scene_node_anchor(
                    &character.children,
                    id,
                    character_transform,
                    time_norm,
                    time_sec,
                ) {
                    return Some(point);
                }
            }
            SceneNode::Mask(mask) => {
                if mask.id.as_deref() == Some(id) {
                    let x = eval_scene_number(&mask.x, time_norm, time_sec).ok()?;
                    let y = eval_scene_number(&mask.y, time_norm, time_sec).ok()?;
                    return Some(transform.transform_point(x, y));
                }
                if let Some(point) =
                    find_scene_node_anchor(&mask.children, id, transform, time_norm, time_sec)
                {
                    return Some(point);
                }
            }
            SceneNode::Precompose(precompose) => {
                if precompose.id == id {
                    return Some(transform.transform_point(0.0, 0.0));
                }
                if let Some(point) =
                    find_scene_node_anchor(&precompose.children, id, transform, time_norm, time_sec)
                {
                    return Some(point);
                }
            }
            SceneNode::Layer(layer) => {
                let layer_transform =
                    transform.mul(scene_layer_local_transform(layer, time_norm, time_sec).ok()?);
                if layer.id.as_deref() == Some(id) {
                    return Some(layer_transform.transform_point(0.0, 0.0));
                }
                if let Some(point) = find_scene_node_anchor(
                    &layer.children,
                    id,
                    layer_transform,
                    time_norm,
                    time_sec,
                ) {
                    return Some(point);
                }
            }
            _ => {}
        }
    }
    None
}
