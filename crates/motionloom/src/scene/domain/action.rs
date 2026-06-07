use std::collections::{HashMap, HashSet};

use crate::dsl::GraphScript;
use crate::scene::backend::sizing::format_scene_number;
use crate::scene::dsl::{ActionNode, ApplyActionNode, SkeletonNode};
use crate::scene::model::{PartNode, SceneNode};
use crate::scene::render::{MotionLoomSceneRenderError, eval_scene_number};
use crate::scene::spatial::Affine2;

#[derive(Debug, Clone, Copy, Default)]
struct ActionBoneSample {
    x: Option<f32>,
    y: Option<f32>,
    rotation: Option<f32>,
    scale: Option<f32>,
    opacity: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
struct BoneWorldSample {
    transform: Affine2,
    rotation: f32,
    scale: f32,
    opacity: f32,
}

pub(crate) fn apply_action_graph_at_time(
    graph: &GraphScript,
    time_norm: f32,
    time_sec: f32,
) -> Result<Option<GraphScript>, MotionLoomSceneRenderError> {
    if graph.actions.is_empty() || graph.apply_actions.is_empty() {
        return Ok(None);
    }

    let action_map = graph
        .actions
        .iter()
        .map(|action| (action.id.as_str(), action))
        .collect::<HashMap<_, _>>();
    let skeleton_map = graph
        .skeletons
        .iter()
        .map(|skeleton| (skeleton.id.as_str(), skeleton))
        .collect::<HashMap<_, _>>();
    let mut next = graph.clone();

    for apply in &graph.apply_actions {
        let action = action_map.get(apply.action.as_str()).ok_or_else(|| {
            MotionLoomSceneRenderError::InvalidExpression {
                expr: apply.action.clone(),
                message: "ApplyAction references an unknown Action".to_string(),
            }
        })?;
        let samples = sample_action_bones(action, apply, time_norm, time_sec)?;
        if samples.is_empty() {
            continue;
        }
        apply_action_to_nodes(
            &mut next.scene_nodes,
            &apply.target,
            action.skeleton.as_deref(),
            &skeleton_map,
            &samples,
            time_norm,
            time_sec,
        )?;
        for scene in &mut next.scenes {
            apply_action_to_nodes(
                &mut scene.children,
                &apply.target,
                action.skeleton.as_deref(),
                &skeleton_map,
                &samples,
                time_norm,
                time_sec,
            )?;
        }
    }

    Ok(Some(next))
}

fn sample_action_bones(
    action: &ActionNode,
    apply: &ApplyActionNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<HashMap<String, ActionBoneSample>, MotionLoomSceneRenderError> {
    let local_sec = time_sec - apply.at_ms as f32 / 1000.0;
    let duration_sec = (action.duration_ms as f32 / 1000.0).max(0.0001);
    if local_sec < -0.0001 || local_sec > duration_sec + 0.0001 || action.poses.is_empty() {
        return Ok(HashMap::new());
    }
    let local_sec = local_sec.clamp(0.0, duration_sec);

    let mut prev_ix = 0usize;
    let mut next_ix = action.poses.len().saturating_sub(1);
    for (ix, pose) in action.poses.iter().enumerate() {
        if pose.t <= local_sec {
            prev_ix = ix;
        }
        if pose.t >= local_sec {
            next_ix = ix;
            break;
        }
    }
    if action.poses[prev_ix].t > local_sec {
        prev_ix = next_ix;
    }

    let prev_pose = &action.poses[prev_ix];
    let next_pose = &action.poses[next_ix];
    let span = (next_pose.t - prev_pose.t).abs();
    let mix = if span <= 0.0001 {
        0.0
    } else {
        ((local_sec - prev_pose.t) / span).clamp(0.0, 1.0)
    };

    let mut bone_ids = Vec::<String>::new();
    for bone in &prev_pose.bones {
        if !bone_ids.iter().any(|id| id == &bone.id) {
            bone_ids.push(bone.id.clone());
        }
    }
    for bone in &next_pose.bones {
        if !bone_ids.iter().any(|id| id == &bone.id) {
            bone_ids.push(bone.id.clone());
        }
    }

    let mut samples = HashMap::<String, ActionBoneSample>::new();
    for bone_id in bone_ids {
        let prev = prev_pose.bones.iter().find(|bone| bone.id == bone_id);
        let next = next_pose.bones.iter().find(|bone| bone.id == bone_id);
        let sample = ActionBoneSample {
            x: interpolate_action_attr(
                prev.and_then(|bone| bone.x.as_ref()),
                next.and_then(|bone| bone.x.as_ref()),
                mix,
                time_norm,
                time_sec,
            )?,
            y: interpolate_action_attr(
                prev.and_then(|bone| bone.y.as_ref()),
                next.and_then(|bone| bone.y.as_ref()),
                mix,
                time_norm,
                time_sec,
            )?,
            rotation: interpolate_action_attr(
                prev.and_then(|bone| bone.rotation.as_ref()),
                next.and_then(|bone| bone.rotation.as_ref()),
                mix,
                time_norm,
                time_sec,
            )?,
            scale: interpolate_action_attr(
                prev.and_then(|bone| bone.scale.as_ref()),
                next.and_then(|bone| bone.scale.as_ref()),
                mix,
                time_norm,
                time_sec,
            )?,
            opacity: interpolate_action_attr(
                prev.and_then(|bone| bone.opacity.as_ref()),
                next.and_then(|bone| bone.opacity.as_ref()),
                mix,
                time_norm,
                time_sec,
            )?,
        };
        samples.insert(bone_id, sample);
    }

    Ok(samples)
}

fn interpolate_action_attr(
    prev: Option<&String>,
    next: Option<&String>,
    mix: f32,
    time_norm: f32,
    time_sec: f32,
) -> Result<Option<f32>, MotionLoomSceneRenderError> {
    let prev = prev
        .map(|value| eval_scene_number(value, time_norm, time_sec))
        .transpose()?;
    let next = next
        .map(|value| eval_scene_number(value, time_norm, time_sec))
        .transpose()?;
    Ok(match (prev, next) {
        (Some(a), Some(b)) => Some(a + (b - a) * mix),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    })
}

fn apply_action_to_nodes(
    nodes: &mut [SceneNode],
    target: &str,
    action_skeleton: Option<&str>,
    skeleton_map: &HashMap<&str, &SkeletonNode>,
    samples: &HashMap<String, ActionBoneSample>,
    time_norm: f32,
    time_sec: f32,
) -> Result<(), MotionLoomSceneRenderError> {
    for node in nodes {
        match node {
            SceneNode::Timeline(timeline) => apply_action_to_nodes(
                &mut timeline.children,
                target,
                action_skeleton,
                skeleton_map,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Track(track) => apply_action_to_nodes(
                &mut track.children,
                target,
                action_skeleton,
                skeleton_map,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Sequence(sequence) => apply_action_to_nodes(
                &mut sequence.children,
                target,
                action_skeleton,
                skeleton_map,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Chain(chain) => apply_action_to_nodes(
                &mut chain.children,
                target,
                action_skeleton,
                skeleton_map,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Layer(layer) => apply_action_to_nodes(
                &mut layer.children,
                target,
                action_skeleton,
                skeleton_map,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Character(character) => {
                if character.id.as_deref() == Some(target) {
                    let skeleton_id = action_skeleton.or(character.rig.as_deref());
                    if let Some(skeleton) = skeleton_id.and_then(|id| skeleton_map.get(id).copied())
                    {
                        let bone_world =
                            sample_skeleton_bones(skeleton, samples, time_norm, time_sec)?;
                        apply_skeleton_action_to_character_children(
                            &mut character.children,
                            &bone_world,
                            samples,
                            time_norm,
                            time_sec,
                        )?;
                    } else {
                        apply_action_to_character_children(
                            &mut character.children,
                            samples,
                            time_norm,
                            time_sec,
                        )?;
                    }
                } else {
                    apply_action_to_nodes(
                        &mut character.children,
                        target,
                        action_skeleton,
                        skeleton_map,
                        samples,
                        time_norm,
                        time_sec,
                    )?;
                }
            }
            SceneNode::Group(group) => apply_action_to_nodes(
                &mut group.children,
                target,
                action_skeleton,
                skeleton_map,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Part(part) => apply_action_to_nodes(
                &mut part.children,
                target,
                action_skeleton,
                skeleton_map,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Repeat(repeat) => apply_action_to_nodes(
                &mut repeat.children,
                target,
                action_skeleton,
                skeleton_map,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Mask(mask) => apply_action_to_nodes(
                &mut mask.children,
                target,
                action_skeleton,
                skeleton_map,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Precompose(precompose) => apply_action_to_nodes(
                &mut precompose.children,
                target,
                action_skeleton,
                skeleton_map,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Camera(camera) => apply_action_to_nodes(
                &mut camera.children,
                target,
                action_skeleton,
                skeleton_map,
                samples,
                time_norm,
                time_sec,
            )?,
            _ => {}
        }
    }
    Ok(())
}

fn sample_skeleton_bones(
    skeleton: &SkeletonNode,
    samples: &HashMap<String, ActionBoneSample>,
    time_norm: f32,
    time_sec: f32,
) -> Result<HashMap<String, BoneWorldSample>, MotionLoomSceneRenderError> {
    let bone_ids = skeleton
        .bones
        .iter()
        .map(|bone| bone.id.as_str())
        .collect::<HashSet<_>>();
    let mut out = HashMap::<String, BoneWorldSample>::new();
    let mut visiting = HashSet::<String>::new();
    for bone in &skeleton.bones {
        sample_skeleton_bone(
            skeleton,
            &bone_ids,
            &bone.id,
            samples,
            time_norm,
            time_sec,
            &mut visiting,
            &mut out,
        )?;
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn sample_skeleton_bone(
    skeleton: &SkeletonNode,
    bone_ids: &HashSet<&str>,
    bone_id: &str,
    samples: &HashMap<String, ActionBoneSample>,
    time_norm: f32,
    time_sec: f32,
    visiting: &mut HashSet<String>,
    out: &mut HashMap<String, BoneWorldSample>,
) -> Result<BoneWorldSample, MotionLoomSceneRenderError> {
    if let Some(existing) = out.get(bone_id).copied() {
        return Ok(existing);
    }
    if !visiting.insert(bone_id.to_string()) {
        return Err(MotionLoomSceneRenderError::InvalidExpression {
            expr: bone_id.to_string(),
            message: format!("Skeleton {} contains a cyclic bone hierarchy.", skeleton.id),
        });
    }

    let bone = skeleton
        .bones
        .iter()
        .find(|bone| bone.id == bone_id)
        .ok_or_else(|| MotionLoomSceneRenderError::InvalidExpression {
            expr: bone_id.to_string(),
            message: format!("Skeleton {} bone not found.", skeleton.id),
        })?;
    let parent = if let Some(parent_id) = bone.parent.as_deref() {
        if !bone_ids.contains(parent_id) {
            return Err(MotionLoomSceneRenderError::InvalidExpression {
                expr: parent_id.to_string(),
                message: format!(
                    "Skeleton {} bone {} references an unknown parent.",
                    skeleton.id, bone.id
                ),
            });
        }
        sample_skeleton_bone(
            skeleton, bone_ids, parent_id, samples, time_norm, time_sec, visiting, out,
        )?
    } else {
        BoneWorldSample {
            transform: Affine2::identity(),
            rotation: 0.0,
            scale: 1.0,
            opacity: 1.0,
        }
    };

    let sample = samples.get(&bone.id).copied().unwrap_or_default();
    let x = eval_scene_number(&bone.x, time_norm, time_sec)? + sample.x.unwrap_or(0.0);
    let y = eval_scene_number(&bone.y, time_norm, time_sec)? + sample.y.unwrap_or(0.0);
    let rotation =
        eval_scene_number(&bone.rotation, time_norm, time_sec)? + sample.rotation.unwrap_or(0.0);
    let scale = (eval_scene_number(&bone.scale, time_norm, time_sec)?
        * sample.scale.unwrap_or(1.0))
    .clamp(0.001, 64.0);
    let opacity = sample.opacity.unwrap_or(1.0).clamp(0.0, 1.0);
    let transform = parent
        .transform
        .mul(Affine2::translate(x, y))
        .mul(Affine2::rotate_deg(rotation))
        .mul(Affine2::scale(scale));
    let world = BoneWorldSample {
        transform,
        rotation: parent.rotation + rotation,
        scale: parent.scale * scale,
        opacity: parent.opacity * opacity,
    };

    visiting.remove(bone_id);
    out.insert(bone.id.clone(), world);
    Ok(world)
}

fn apply_skeleton_action_to_character_children(
    nodes: &mut [SceneNode],
    bone_world: &HashMap<String, BoneWorldSample>,
    samples: &HashMap<String, ActionBoneSample>,
    time_norm: f32,
    time_sec: f32,
) -> Result<(), MotionLoomSceneRenderError> {
    for node in nodes {
        match node {
            SceneNode::Timeline(timeline) => apply_skeleton_action_to_character_children(
                &mut timeline.children,
                bone_world,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Track(track) => apply_skeleton_action_to_character_children(
                &mut track.children,
                bone_world,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Sequence(sequence) => apply_skeleton_action_to_character_children(
                &mut sequence.children,
                bone_world,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Chain(chain) => apply_skeleton_action_to_character_children(
                &mut chain.children,
                bone_world,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Layer(layer) => apply_skeleton_action_to_character_children(
                &mut layer.children,
                bone_world,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Part(part) => {
                if let Some(attach_to) = part.attach_to.as_deref()
                    && let Some(bone) = bone_world.get(attach_to).copied()
                {
                    apply_bone_world_to_part(part, bone, time_norm, time_sec)?;
                } else if let Some(id) = part.id.as_deref()
                    && let Some(sample) = samples.get(id)
                {
                    apply_action_sample_to_part(part, *sample, time_norm, time_sec)?;
                }
                apply_skeleton_action_to_character_children(
                    &mut part.children,
                    bone_world,
                    samples,
                    time_norm,
                    time_sec,
                )?;
            }
            SceneNode::Group(group) => apply_skeleton_action_to_character_children(
                &mut group.children,
                bone_world,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Repeat(repeat) => apply_skeleton_action_to_character_children(
                &mut repeat.children,
                bone_world,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Mask(mask) => apply_skeleton_action_to_character_children(
                &mut mask.children,
                bone_world,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Precompose(precompose) => apply_skeleton_action_to_character_children(
                &mut precompose.children,
                bone_world,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Camera(camera) => apply_skeleton_action_to_character_children(
                &mut camera.children,
                bone_world,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Character(character) => apply_skeleton_action_to_character_children(
                &mut character.children,
                bone_world,
                samples,
                time_norm,
                time_sec,
            )?,
            _ => {}
        }
    }
    Ok(())
}

fn apply_bone_world_to_part(
    part: &mut PartNode,
    bone: BoneWorldSample,
    time_norm: f32,
    time_sec: f32,
) -> Result<(), MotionLoomSceneRenderError> {
    let local_x = eval_scene_number(&part.x, time_norm, time_sec)?;
    let local_y = eval_scene_number(&part.y, time_norm, time_sec)?;
    let local_rotation = eval_scene_number(&part.rotation, time_norm, time_sec)?;
    let local_scale = eval_scene_number(&part.scale, time_norm, time_sec)?.clamp(0.001, 64.0);
    let local_opacity = eval_scene_number(&part.opacity, time_norm, time_sec)?.clamp(0.0, 1.0);
    let (world_x, world_y) = bone.transform.transform_point(local_x, local_y);

    part.x = format_scene_number(world_x);
    part.y = format_scene_number(world_y);
    part.rotation = format_scene_number(bone.rotation + local_rotation);
    part.scale = format_scene_number((bone.scale * local_scale).clamp(0.001, 64.0));
    part.opacity = format_scene_number((bone.opacity * local_opacity).clamp(0.0, 1.0));
    Ok(())
}

fn apply_action_to_character_children(
    nodes: &mut [SceneNode],
    samples: &HashMap<String, ActionBoneSample>,
    time_norm: f32,
    time_sec: f32,
) -> Result<(), MotionLoomSceneRenderError> {
    for node in nodes {
        match node {
            SceneNode::Timeline(timeline) => apply_action_to_character_children(
                &mut timeline.children,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Track(track) => apply_action_to_character_children(
                &mut track.children,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Sequence(sequence) => apply_action_to_character_children(
                &mut sequence.children,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Chain(chain) => apply_action_to_character_children(
                &mut chain.children,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Layer(layer) => apply_action_to_character_children(
                &mut layer.children,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Part(part) => {
                if let Some(id) = part.id.as_deref()
                    && let Some(sample) = samples.get(id)
                {
                    apply_action_sample_to_part(part, *sample, time_norm, time_sec)?;
                }
                apply_action_to_character_children(
                    &mut part.children,
                    samples,
                    time_norm,
                    time_sec,
                )?;
            }
            SceneNode::Group(group) => apply_action_to_character_children(
                &mut group.children,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Repeat(repeat) => apply_action_to_character_children(
                &mut repeat.children,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Mask(mask) => apply_action_to_character_children(
                &mut mask.children,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Precompose(precompose) => apply_action_to_character_children(
                &mut precompose.children,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Camera(camera) => apply_action_to_character_children(
                &mut camera.children,
                samples,
                time_norm,
                time_sec,
            )?,
            SceneNode::Character(character) => apply_action_to_character_children(
                &mut character.children,
                samples,
                time_norm,
                time_sec,
            )?,
            _ => {}
        }
    }
    Ok(())
}

fn apply_action_sample_to_part(
    part: &mut PartNode,
    sample: ActionBoneSample,
    time_norm: f32,
    time_sec: f32,
) -> Result<(), MotionLoomSceneRenderError> {
    if let Some(x) = sample.x {
        part.x = format_scene_number(eval_scene_number(&part.x, time_norm, time_sec)? + x);
    }
    if let Some(y) = sample.y {
        part.y = format_scene_number(eval_scene_number(&part.y, time_norm, time_sec)? + y);
    }
    if let Some(rotation) = sample.rotation {
        part.rotation =
            format_scene_number(eval_scene_number(&part.rotation, time_norm, time_sec)? + rotation);
    }
    if let Some(scale) = sample.scale {
        part.scale = format_scene_number(
            eval_scene_number(&part.scale, time_norm, time_sec)? * scale.max(0.001),
        );
    }
    if let Some(opacity) = sample.opacity {
        part.opacity = format_scene_number(
            eval_scene_number(&part.opacity, time_norm, time_sec)? * opacity.clamp(0.0, 1.0),
        );
    }
    Ok(())
}
