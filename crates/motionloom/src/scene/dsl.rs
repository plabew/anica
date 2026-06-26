// =========================================
// crates/motionloom/src/scene/dsl.rs

use crate::dsl::{
    attr_value, collect_self_closing_block, collect_tag_block, find_matching_close_tag,
    is_self_closing_tag, parse_duration_ms, parse_signed_time_ms, parse_size, parse_time_seconds,
    required_attr_value, required_attr_value_any, starts_open_tag, strip_wrappers,
};
use crate::error::GraphParseError;
use crate::scene::model::*;
use crate::scene::text::{
    TextAnimatorNode, TextEffectNode, TextGlowEffectNode, TextLayoutNode, TextNode,
    TextSelectorKind, TextStyleOverrideNode, TextTransformNode,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfileNode {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub model: Option<String>,
    pub preset: String,
    #[serde(default)]
    pub retarget: Option<ModelProfileRetargetNode>,
    #[serde(default)]
    pub bone_axis_map: Option<ModelProfileBoneAxisMapNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfileRetargetNode {
    pub preset: String,
    #[serde(default)]
    pub maps: Vec<ModelProfileRetargetMapNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfileRetargetMapNode {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfileBoneAxisMapNode {
    #[serde(default)]
    pub axes: Vec<ModelProfileBoneAxisNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfileBoneAxisNode {
    pub bone: String,
    #[serde(default)]
    pub forward: Option<String>,
    #[serde(default)]
    pub side: Option<String>,
    #[serde(default)]
    pub twist: Option<String>,
    #[serde(default)]
    pub bend: Option<String>,
    #[serde(default)]
    pub turn: Option<String>,
    #[serde(default)]
    pub rest_forward: Option<String>,
    #[serde(default)]
    pub rest_side: Option<String>,
    #[serde(default)]
    pub rest_twist: Option<String>,
    #[serde(default)]
    pub rest_bend: Option<String>,
    #[serde(default)]
    pub rest_turn: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkeletonNode {
    pub id: String,
    pub bones: Vec<SkeletonBoneNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkeletonBoneNode {
    pub id: String,
    pub parent: Option<String>,
    pub x: String,
    pub y: String,
    pub rotation: String,
    pub scale: String,
    pub length: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionNode {
    pub id: String,
    pub skeleton: Option<String>,
    pub duration_ms: u64,
    pub poses: Vec<ActionPoseNode>,
    #[serde(default)]
    pub iks: Vec<ActionIkNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionPoseNode {
    pub t: f32,
    pub bones: Vec<ActionBoneNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionBoneNode {
    pub id: String,
    pub x: Option<String>,
    pub y: Option<String>,
    pub rotation: Option<String>,
    pub scale: Option<String>,
    pub opacity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionIkNode {
    pub root: String,
    pub mid: String,
    pub end: String,
    #[serde(default)]
    pub chain: Vec<String>,
    pub target_x: String,
    pub target_y: String,
    pub bend: String,
    pub weight: String,
    pub iterations: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyActionNode {
    pub target: String,
    pub action: String,
    pub at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundNode {
    pub id: Option<String>,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageNode {
    pub id: Option<String>,
    pub src: String,
    pub x: String,
    pub y: String,
    pub scale: String,
    pub opacity: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SvgNode {
    pub id: Option<String>,
    pub src: String,
    pub x: String,
    pub y: String,
    pub scale: String,
    pub opacity: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BrushParseContext {
    brushes: HashMap<String, BrushDef>,
    inherited_brush: Option<String>,
}

impl BrushParseContext {
    fn define_brushes(&mut self, brushes: &[BrushDef]) {
        for brush in brushes {
            self.brushes.insert(brush.id.clone(), brush.clone());
        }
    }

    fn with_inherited_brush(&self, brush: Option<String>) -> Self {
        let mut next = self.clone();
        if let Some(brush) = brush {
            next.inherited_brush = Some(brush);
        }
        next
    }

    fn validate_brush_ref(&self, brush: Option<&str>, line: usize) -> Result<(), GraphParseError> {
        let Some(brush) = brush else {
            return Ok(());
        };
        if self.brushes.contains_key(brush) {
            return Ok(());
        }
        Err(GraphParseError {
            line,
            message: format!("brush reference not found: {brush}"),
        })
    }

    fn brush_for_path<'a>(
        &'a self,
        block: &str,
        line: usize,
    ) -> Result<(Option<String>, Option<&'a BrushDef>), GraphParseError> {
        let brush_id = attr_value(block, "brush")
            .map(|v| strip_wrappers(&v).to_string())
            .or_else(|| self.inherited_brush.clone());
        self.validate_brush_ref(brush_id.as_deref(), line)?;
        let brush = brush_id.as_ref().and_then(|id| self.brushes.get(id));
        Ok((brush_id, brush))
    }
}

pub(crate) fn validate_scene_camera_structure(
    scenes: &[SceneRootNode],
    scene_nodes: &[SceneNode],
    line: usize,
) -> Result<(), GraphParseError> {
    for scene in scenes {
        validate_scene_camera_structure_in_nodes(&scene.children, false, line)?;
    }
    validate_scene_camera_structure_in_nodes(scene_nodes, false, line)
}

fn validate_scene_camera_structure_in_nodes(
    nodes: &[SceneNode],
    in_camera_track: bool,
    line: usize,
) -> Result<(), GraphParseError> {
    for node in nodes {
        match node {
            SceneNode::Defs(defs) => {
                for mask in &defs.masks {
                    validate_scene_camera_structure_in_nodes(&mask.children, false, line)?;
                }
                for precompose in &defs.precomposes {
                    validate_scene_camera_structure_in_nodes(&precompose.children, false, line)?;
                }
                for component in &defs.components {
                    validate_scene_camera_structure_in_nodes(&component.children, false, line)?;
                }
            }
            SceneNode::Timeline(timeline) => {
                validate_scene_camera_structure_in_nodes(&timeline.children, false, line)?;
            }
            SceneNode::Track(track) => {
                if is_scene_camera_track(track) {
                    validate_scene_camera_track(track, line)?;
                } else {
                    validate_scene_camera_structure_in_nodes(&track.children, false, line)?;
                }
            }
            SceneNode::Sequence(sequence) => {
                validate_scene_camera_structure_in_nodes(
                    &sequence.children,
                    in_camera_track,
                    line,
                )?;
            }
            SceneNode::Chain(chain) => {
                validate_scene_camera_structure_in_nodes(&chain.children, in_camera_track, line)?;
            }
            SceneNode::Camera(_) if !in_camera_track => {
                return Err(GraphParseError {
                    line,
                    message: "<Camera> must be inside <Track role=\"camera\"><Sequence><Camera ... /></Sequence></Track>. Put visual content in <Track space=\"world\"> or <Track space=\"screen\">.".to_string(),
                });
            }
            SceneNode::Camera(camera) => {
                if !camera.children.is_empty() {
                    return Err(GraphParseError {
                        line,
                        message: "<Scene> Camera must be self-closing and cannot contain visual children.".to_string(),
                    });
                }
            }
            SceneNode::Group(group) => {
                validate_scene_camera_structure_in_nodes(&group.children, false, line)?;
            }
            SceneNode::Layer(layer) => {
                validate_scene_camera_structure_in_nodes(&layer.children, false, line)?;
            }
            SceneNode::Character(character) => {
                validate_scene_camera_structure_in_nodes(&character.children, false, line)?;
            }
            SceneNode::Part(part) => {
                validate_scene_camera_structure_in_nodes(&part.children, false, line)?;
            }
            SceneNode::Repeat(repeat) => {
                validate_scene_camera_structure_in_nodes(&repeat.children, false, line)?;
            }
            SceneNode::Mask(mask) => {
                validate_scene_camera_structure_in_nodes(&mask.children, false, line)?;
            }
            SceneNode::Precompose(precompose) => {
                validate_scene_camera_structure_in_nodes(&precompose.children, false, line)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_scene_camera_track(track: &SceneTrackNode, line: usize) -> Result<(), GraphParseError> {
    if track.children.is_empty() {
        return Err(GraphParseError {
            line,
            message: "<Track role=\"camera\"> requires at least one <Sequence> containing a single <Camera />.".to_string(),
        });
    }
    for child in &track.children {
        let SceneNode::Sequence(sequence) = child else {
            return Err(GraphParseError {
                line,
                message: "<Track role=\"camera\"> only accepts <Sequence> children. Each sequence must contain a single self-closing <Camera />.".to_string(),
            });
        };
        if sequence.children.len() != 1 {
            return Err(GraphParseError {
                line,
                message: "<Track role=\"camera\"><Sequence> must contain exactly one self-closing <Camera />.".to_string(),
            });
        }
        let SceneNode::Camera(camera) = &sequence.children[0] else {
            return Err(GraphParseError {
                line,
                message: "<Track role=\"camera\"><Sequence> must contain exactly one self-closing <Camera />.".to_string(),
            });
        };
        if !camera.children.is_empty() {
            return Err(GraphParseError {
                line,
                message: "<Scene> Camera must be self-closing and cannot contain visual children."
                    .to_string(),
            });
        }
    }
    Ok(())
}

fn is_scene_camera_track(track: &SceneTrackNode) -> bool {
    track
        .role
        .as_deref()
        .is_some_and(|role| role.eq_ignore_ascii_case("camera"))
}

pub(crate) fn validate_scene_model_profile_refs(
    scenes: &[SceneRootNode],
    scene_nodes: &[SceneNode],
    model_profile_ids: &HashSet<String>,
    line: usize,
) -> Result<(), GraphParseError> {
    for scene in scenes {
        validate_scene_model_profile_refs_in_nodes(&scene.children, model_profile_ids, line)?;
    }
    validate_scene_model_profile_refs_in_nodes(scene_nodes, model_profile_ids, line)
}

fn validate_scene_model_profile_refs_in_nodes(
    nodes: &[SceneNode],
    model_profile_ids: &HashSet<String>,
    line: usize,
) -> Result<(), GraphParseError> {
    for node in nodes {
        match node {
            SceneNode::Defs(defs) => {
                for mask in &defs.masks {
                    validate_scene_model_profile_refs_in_nodes(
                        &mask.children,
                        model_profile_ids,
                        line,
                    )?;
                }
                for precompose in &defs.precomposes {
                    validate_scene_model_profile_refs_in_nodes(
                        &precompose.children,
                        model_profile_ids,
                        line,
                    )?;
                }
                for component in &defs.components {
                    validate_scene_model_profile_refs_in_nodes(
                        &component.children,
                        model_profile_ids,
                        line,
                    )?;
                }
            }
            SceneNode::Timeline(timeline) => {
                validate_scene_model_profile_refs_in_nodes(
                    &timeline.children,
                    model_profile_ids,
                    line,
                )?;
            }
            SceneNode::Track(track) => {
                validate_scene_model_profile_refs_in_nodes(
                    &track.children,
                    model_profile_ids,
                    line,
                )?;
            }
            SceneNode::Sequence(sequence) => {
                validate_scene_model_profile_refs_in_nodes(
                    &sequence.children,
                    model_profile_ids,
                    line,
                )?;
            }
            SceneNode::Chain(chain) => {
                validate_scene_model_profile_refs_in_nodes(
                    &chain.children,
                    model_profile_ids,
                    line,
                )?;
            }
            SceneNode::Character(character) => {
                if let Some(model_profile) = character.model_profile.as_deref()
                    && !model_profile_ids.contains(model_profile)
                {
                    return Err(GraphParseError {
                        line,
                        message: format!("Character modelProfile not found: {model_profile}"),
                    });
                }
                validate_scene_model_profile_refs_in_nodes(
                    &character.children,
                    model_profile_ids,
                    line,
                )?;
            }
            SceneNode::Group(group) => {
                validate_scene_model_profile_refs_in_nodes(
                    &group.children,
                    model_profile_ids,
                    line,
                )?;
            }
            SceneNode::Part(part) => {
                validate_scene_model_profile_refs_in_nodes(
                    &part.children,
                    model_profile_ids,
                    line,
                )?;
            }
            SceneNode::Repeat(repeat) => {
                validate_scene_model_profile_refs_in_nodes(
                    &repeat.children,
                    model_profile_ids,
                    line,
                )?;
            }
            SceneNode::Mask(mask) => {
                validate_scene_model_profile_refs_in_nodes(
                    &mask.children,
                    model_profile_ids,
                    line,
                )?;
            }
            SceneNode::Precompose(precompose) => {
                validate_scene_model_profile_refs_in_nodes(
                    &precompose.children,
                    model_profile_ids,
                    line,
                )?;
            }
            SceneNode::Layer(layer) => {
                validate_scene_model_profile_refs_in_nodes(
                    &layer.children,
                    model_profile_ids,
                    line,
                )?;
            }
            SceneNode::Camera(camera) => {
                validate_scene_model_profile_refs_in_nodes(
                    &camera.children,
                    model_profile_ids,
                    line,
                )?;
            }
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn parse_scene_root_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(SceneRootNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Scene")?;
    let id = strip_wrappers(&required_attr_value(&open_tag, "id", start + 1)?).to_string();
    let size = attr_value(&open_tag, "size")
        .as_deref()
        .map(|v| parse_size(v, start + 1, "size"))
        .transpose()?;
    let mut child_ctx = brush_ctx.clone();
    let children = parse_scene_root_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((SceneRootNode { id, size, children }, close_ix))
}

fn parse_scene_root_nodes(
    lines: &[&str],
    start: usize,
    end: usize,
    brush_ctx: &mut BrushParseContext,
) -> Result<Vec<SceneNode>, GraphParseError> {
    let mut nodes = Vec::<SceneNode>::new();
    let mut i = start;
    while i < end {
        let line = lines[i].trim();
        if line.is_empty()
            || line.starts_with("//")
            || line.starts_with('{')
            || line.starts_with("<!--")
        {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Defs") {
            let (defs, end_ix) = parse_defs_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Defs(defs));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Timeline") {
            let (timeline, end_ix) = parse_timeline_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Timeline(timeline));
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!(
                "<Scene> root only accepts <Defs> and <Timeline>. Visual nodes must be wrapped in <Timeline><Track><Sequence>..., got: {line}"
            ),
        });
    }
    Ok(nodes)
}

pub(crate) fn parse_model_profile_block(
    lines: &[&str],
    start: usize,
) -> Result<(ModelProfileNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    if is_self_closing_tag(&open_tag) {
        return Ok((
            parse_model_profile_node(&open_tag, None, None, start + 1)?,
            open_end_ix,
        ));
    }

    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "ModelProfile")?;
    let mut retarget = None;
    let mut bone_axis_map = None;
    let mut i = open_end_ix + 1;

    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty()
            || line.starts_with("//")
            || line.starts_with('{')
            || line.starts_with("<!--")
        {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Retarget") {
            let (node, end_ix) = parse_model_profile_retarget_block(lines, i)?;
            retarget = Some(node);
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "BoneAxisMap") {
            let (node, end_ix) = parse_model_profile_bone_axis_map_block(lines, i)?;
            bone_axis_map = Some(node);
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!(
                "<ModelProfile> only accepts <Retarget> or <BoneAxisMap> children, got: {line}"
            ),
        });
    }

    Ok((
        parse_model_profile_node(&open_tag, retarget, bone_axis_map, start + 1)?,
        close_ix,
    ))
}

fn parse_model_profile_node(
    block: &str,
    retarget: Option<ModelProfileRetargetNode>,
    bone_axis_map: Option<ModelProfileBoneAxisMapNode>,
    line: usize,
) -> Result<ModelProfileNode, GraphParseError> {
    let id = strip_wrappers(&required_attr_value(block, "id", line)?).to_string();
    let kind = attr_value(block, "kind")
        .map(|v| strip_wrappers(&v).to_ascii_lowercase())
        .unwrap_or_else(|| "2d".to_string());
    if !matches!(kind.as_str(), "2d" | "3d") {
        return Err(GraphParseError {
            line,
            message: format!("ModelProfile {id} kind must be \"2d\" or \"3d\", got: {kind}"),
        });
    }
    let model = attr_value(block, "model")
        .or_else(|| attr_value(block, "src"))
        .map(|v| strip_wrappers(&v).to_string())
        .filter(|v| !v.is_empty());
    let preset = attr_value(block, "preset")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "humanoid_v1".to_string());

    Ok(ModelProfileNode {
        id,
        kind,
        model,
        preset,
        retarget,
        bone_axis_map,
    })
}

fn parse_model_profile_retarget_block(
    lines: &[&str],
    start: usize,
) -> Result<(ModelProfileRetargetNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let preset = attr_value(&open_tag, "preset")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "humanoid_v1".to_string());
    if is_self_closing_tag(&open_tag) {
        return Ok((
            ModelProfileRetargetNode {
                preset,
                maps: Vec::new(),
            },
            open_end_ix,
        ));
    }

    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Retarget")?;
    let mut maps = Vec::<ModelProfileRetargetMapNode>::new();
    let mut i = open_end_ix + 1;

    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('{') {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Map") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            maps.push(ModelProfileRetargetMapNode {
                from: strip_wrappers(&required_attr_value(&tag, "from", i + 1)?).to_string(),
                to: strip_wrappers(&required_attr_value(&tag, "to", i + 1)?).to_string(),
            });
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!("<Retarget> only accepts <Map /> children, got: {line}"),
        });
    }

    Ok((ModelProfileRetargetNode { preset, maps }, close_ix))
}

fn parse_model_profile_bone_axis_map_block(
    lines: &[&str],
    start: usize,
) -> Result<(ModelProfileBoneAxisMapNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    if is_self_closing_tag(&open_tag) {
        return Ok((
            ModelProfileBoneAxisMapNode { axes: Vec::new() },
            open_end_ix,
        ));
    }

    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "BoneAxisMap")?;
    let mut axes = Vec::<ModelProfileBoneAxisNode>::new();
    let mut i = open_end_ix + 1;

    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('{') {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Axis") || starts_open_tag(line, "Bone") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            axes.push(parse_model_profile_bone_axis_node(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!(
                "<BoneAxisMap> only accepts <Axis /> or <Bone /> children, got: {line}"
            ),
        });
    }

    Ok((ModelProfileBoneAxisMapNode { axes }, close_ix))
}

fn parse_model_profile_bone_axis_node(
    block: &str,
    line: usize,
) -> Result<ModelProfileBoneAxisNode, GraphParseError> {
    let bone = strip_wrappers(&required_attr_value_any(block, &["bone", "id"], line)?).to_string();
    let attr = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| attr_value(block, key))
            .map(|v| strip_wrappers(&v).to_string())
    };

    Ok(ModelProfileBoneAxisNode {
        bone,
        forward: attr(&["forward"]),
        side: attr(&["side"]),
        twist: attr(&["twist"]),
        bend: attr(&["bend"]),
        turn: attr(&["turn"]),
        rest_forward: attr(&["restForward", "rest_forward"]),
        rest_side: attr(&["restSide", "rest_side"]),
        rest_twist: attr(&["restTwist", "rest_twist"]),
        rest_bend: attr(&["restBend", "rest_bend"]),
        rest_turn: attr(&["restTurn", "rest_turn"]),
    })
}

pub(crate) fn parse_skeleton_block(
    lines: &[&str],
    start: usize,
) -> Result<(SkeletonNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Skeleton")?;
    let id = strip_wrappers(&required_attr_value(&open_tag, "id", start + 1)?).to_string();
    let mut bones = Vec::<SkeletonBoneNode>::new();
    let mut i = open_end_ix + 1;

    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('{') {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Bone") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            bones.push(parse_skeleton_bone_node(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!("<Skeleton> only accepts <Bone /> children, got: {line}"),
        });
    }

    Ok((SkeletonNode { id, bones }, close_ix))
}

fn parse_skeleton_bone_node(block: &str, line: usize) -> Result<SkeletonBoneNode, GraphParseError> {
    let id = strip_wrappers(&required_attr_value(block, "id", line)?).to_string();
    let parent = attr_value(block, "parent")
        .or_else(|| attr_value(block, "parentId"))
        .or_else(|| attr_value(block, "parent_id"))
        .map(|v| strip_wrappers(&v).to_string())
        .filter(|v| !v.is_empty());
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let rotation = attr_value(block, "rotation")
        .or_else(|| attr_value(block, "rotate"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let scale = attr_value(block, "scale")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let length = attr_value(block, "length")
        .or_else(|| attr_value(block, "len"))
        .map(|v| strip_wrappers(&v).to_string());

    Ok(SkeletonBoneNode {
        id,
        parent,
        x,
        y,
        rotation,
        scale,
        length,
    })
}

pub(crate) fn parse_action_block(
    lines: &[&str],
    start: usize,
) -> Result<(ActionNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Action")?;
    let id = strip_wrappers(&required_attr_value(&open_tag, "id", start + 1)?).to_string();
    let skeleton = attr_value(&open_tag, "skeleton")
        .or_else(|| attr_value(&open_tag, "rig"))
        .map(|v| strip_wrappers(&v).to_string());
    let duration_explicit = attr_value(&open_tag, "duration").is_some();
    let mut poses = Vec::<ActionPoseNode>::new();
    let mut iks = Vec::<ActionIkNode>::new();
    let mut i = open_end_ix + 1;

    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('{') {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Pose") {
            let (pose, end_ix) = parse_action_pose_block(lines, i)?;
            poses.push(pose);
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "IK") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            iks.push(parse_action_ik_node(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!("<Action> only accepts <Pose> or <IK /> children, got: {line}"),
        });
    }

    poses.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    let duration_ms = if duration_explicit {
        parse_duration_ms(&open_tag, start + 1, 0)?
    } else {
        poses
            .iter()
            .map(|pose| (pose.t.max(0.0) * 1000.0).round() as u64)
            .max()
            .unwrap_or(0)
    };
    if duration_ms == 0 {
        return Err(GraphParseError {
            line: start + 1,
            message: format!("Action {id} duration must be greater than zero."),
        });
    }

    Ok((
        ActionNode {
            id,
            skeleton,
            duration_ms,
            poses,
            iks,
        },
        close_ix,
    ))
}

fn parse_action_pose_block(
    lines: &[&str],
    start: usize,
) -> Result<(ActionPoseNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Pose")?;
    let t_raw = required_attr_value(&open_tag, "t", start + 1)
        .or_else(|_| required_attr_value(&open_tag, "time", start + 1))?;
    let t = parse_time_seconds(&t_raw, start + 1, "t")?;
    let mut bones = Vec::<ActionBoneNode>::new();
    let mut i = open_end_ix + 1;

    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('{') {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Bone") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            bones.push(parse_action_bone_node(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!("<Pose> only accepts <Bone /> children, got: {line}"),
        });
    }

    Ok((ActionPoseNode { t, bones }, close_ix))
}

fn parse_action_bone_node(block: &str, line: usize) -> Result<ActionBoneNode, GraphParseError> {
    let id = strip_wrappers(&required_attr_value(block, "id", line)?).to_string();
    Ok(ActionBoneNode {
        id,
        x: attr_value(block, "x").map(|v| strip_wrappers(&v).to_string()),
        y: attr_value(block, "y").map(|v| strip_wrappers(&v).to_string()),
        rotation: attr_value(block, "rotation")
            .or_else(|| attr_value(block, "rotate"))
            .map(|v| strip_wrappers(&v).to_string()),
        scale: attr_value(block, "scale").map(|v| strip_wrappers(&v).to_string()),
        opacity: attr_value(block, "opacity").map(|v| strip_wrappers(&v).to_string()),
    })
}

fn parse_action_ik_node(block: &str, line: usize) -> Result<ActionIkNode, GraphParseError> {
    let chain = attr_value(block, "chain")
        .map(|v| {
            strip_wrappers(&v)
                .split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !chain.is_empty() && chain.len() < 3 {
        return Err(GraphParseError {
            line,
            message: "<IK chain=\"...\"> requires at least three bone ids.".to_string(),
        });
    }
    let root = if let Some(root) = chain.first() {
        root.clone()
    } else {
        required_attr_value(block, "root", line)
            .or_else(|_| required_attr_value(block, "start", line))
            .map(|v| strip_wrappers(&v).to_string())?
    };
    let mid = if chain.len() >= 3 {
        chain[1].clone()
    } else {
        required_attr_value(block, "mid", line)
            .or_else(|_| required_attr_value(block, "joint", line))
            .map(|v| strip_wrappers(&v).to_string())?
    };
    let end = if let Some(end) = chain.last() {
        end.clone()
    } else {
        required_attr_value(block, "end", line)
            .or_else(|_| required_attr_value(block, "tip", line))
            .map(|v| strip_wrappers(&v).to_string())?
    };
    let target_x = required_attr_value_any(block, &["targetX", "target_x", "x"], line)
        .map(|v| strip_wrappers(&v).to_string())?;
    let target_y = required_attr_value_any(block, &["targetY", "target_y", "y"], line)
        .map(|v| strip_wrappers(&v).to_string())?;
    let bend = attr_value(block, "bend")
        .or_else(|| attr_value(block, "pole"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1".to_string());
    let weight = attr_value(block, "weight")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1".to_string());
    let iterations = attr_value(block, "iterations")
        .or_else(|| attr_value(block, "iters"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "8".to_string());

    Ok(ActionIkNode {
        root,
        mid,
        end,
        chain,
        target_x,
        target_y,
        bend,
        weight,
        iterations,
    })
}

pub(crate) fn parse_apply_action_node(
    block: &str,
    line: usize,
) -> Result<ApplyActionNode, GraphParseError> {
    let target = strip_wrappers(&required_attr_value(block, "target", line)?).to_string();
    let action = strip_wrappers(&required_attr_value(block, "action", line)?).to_string();
    let at_ms = attr_value(block, "at")
        .as_deref()
        .map(|value| parse_time_seconds(value, line, "at"))
        .transpose()?
        .map(|seconds| (seconds.max(0.0) * 1000.0).round() as u64)
        .unwrap_or(0);
    Ok(ApplyActionNode {
        target,
        action,
        at_ms,
    })
}

fn parse_timeline_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &mut BrushParseContext,
) -> Result<(SceneTimelineNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Timeline")?;
    let id = attr_value(&open_tag, "id").map(|v| strip_wrappers(&v).to_string());
    let mut children = Vec::<SceneNode>::new();
    let mut i = open_end_ix + 1;

    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('{') {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Track") {
            let (track, end_ix) = parse_track_block(lines, i, brush_ctx)?;
            children.push(SceneNode::Track(track));
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!("<Timeline> only accepts <Track> children, got: {line}"),
        });
    }

    Ok((SceneTimelineNode { id, children }, close_ix))
}

fn parse_track_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &mut BrushParseContext,
) -> Result<(SceneTrackNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Track")?;
    let id = attr_value(&open_tag, "id").map(|v| strip_wrappers(&v).to_string());
    let role = attr_value(&open_tag, "role")
        .map(|v| strip_wrappers(&v).to_ascii_lowercase())
        .filter(|v| !v.trim().is_empty());
    if let Some(role) = role.as_deref()
        && role != "camera"
    {
        return Err(GraphParseError {
            line: start + 1,
            message: format!(
                "Invalid Track role=\"{role}\". Expected role=\"camera\" or omit role."
            ),
        });
    }
    let space_attr = attr_value(&open_tag, "space")
        .map(|v| strip_wrappers(&v).to_ascii_lowercase())
        .filter(|v| !v.trim().is_empty());
    if role.as_deref() == Some("camera") && space_attr.is_some() {
        return Err(GraphParseError {
            line: start + 1,
            message: "<Track role=\"camera\"> must not set space. Use space=\"world\" or space=\"screen\" only on visual tracks.".to_string(),
        });
    }
    let space = space_attr.unwrap_or_else(|| "world".to_string());
    if !matches!(space.as_str(), "world" | "screen") {
        return Err(GraphParseError {
            line: start + 1,
            message: format!(
                "Invalid Track space=\"{space}\". Expected space=\"world\" or space=\"screen\"."
            ),
        });
    }
    let z = attr_value(&open_tag, "z")
        .map(|v| {
            let text = strip_wrappers(&v);
            text.parse::<i32>().map_err(|_| GraphParseError {
                line: start + 1,
                message: format!("Invalid Track z value: {text}"),
            })
        })
        .transpose()?
        .unwrap_or(0);
    let z_depth = attr_value(&open_tag, "zDepth")
        .or_else(|| attr_value(&open_tag, "z_depth"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let children = parse_timeline_item_nodes(lines, open_end_ix + 1, close_ix, brush_ctx)?;
    Ok((
        SceneTrackNode {
            id,
            role,
            space,
            z,
            z_depth,
            children,
        },
        close_ix,
    ))
}

fn parse_timeline_item_nodes(
    lines: &[&str],
    start: usize,
    end: usize,
    brush_ctx: &mut BrushParseContext,
) -> Result<Vec<SceneNode>, GraphParseError> {
    let mut nodes = Vec::<SceneNode>::new();
    let mut i = start;
    while i < end {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('{') {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Sequence") {
            let (sequence, end_ix) = parse_sequence_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Sequence(sequence));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Chain") {
            let (chain, end_ix) = parse_chain_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Chain(chain));
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!("<Track> only accepts <Sequence> or <Chain> children, got: {line}"),
        });
    }
    Ok(nodes)
}

fn parse_sequence_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &mut BrushParseContext,
) -> Result<(SceneSequenceNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Sequence")?;
    let id = attr_value(&open_tag, "id").map(|v| strip_wrappers(&v).to_string());
    let from_ms = attr_value(&open_tag, "from")
        .or_else(|| attr_value(&open_tag, "at"))
        .as_deref()
        .map(|value| parse_time_seconds(value, start + 1, "from"))
        .transpose()?
        .map(|seconds| (seconds * 1000.0).round() as u64)
        .unwrap_or(0);
    let duration_ms = parse_duration_ms(&open_tag, start + 1, 0)?;
    if duration_ms == 0 {
        return Err(GraphParseError {
            line: start + 1,
            message: "<Sequence> requires duration greater than zero.".to_string(),
        });
    }
    let out = attr_value(&open_tag, "out")
        .map(|v| strip_wrappers(&v).to_ascii_lowercase())
        .unwrap_or_else(|| "hide".to_string());
    if !matches!(out.as_str(), "hide" | "hold") {
        return Err(GraphParseError {
            line: start + 1,
            message: format!("Sequence out must be \"hide\" or \"hold\", got: {out}"),
        });
    }
    let mut child_ctx = brush_ctx.clone();
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((
        SceneSequenceNode {
            id,
            from_ms,
            duration_ms,
            out,
            children,
        },
        close_ix,
    ))
}

fn parse_chain_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &mut BrushParseContext,
) -> Result<(SceneChainNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Chain")?;
    let id = attr_value(&open_tag, "id").map(|v| strip_wrappers(&v).to_string());
    let from_ms = attr_value(&open_tag, "from")
        .or_else(|| attr_value(&open_tag, "at"))
        .as_deref()
        .map(|value| parse_time_seconds(value, start + 1, "from"))
        .transpose()?
        .map(|seconds| (seconds * 1000.0).round() as u64)
        .unwrap_or(0);
    let gap_ms = attr_value(&open_tag, "gap")
        .as_deref()
        .map(|value| parse_signed_time_ms(value, start + 1, "gap"))
        .transpose()?
        .unwrap_or(0);
    let mut children = Vec::<SceneNode>::new();
    let mut i = open_end_ix + 1;

    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('{') {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Sequence") {
            let (sequence, end_ix) = parse_sequence_block(lines, i, brush_ctx)?;
            children.push(SceneNode::Sequence(sequence));
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!("<Chain> only accepts <Sequence> children, got: {line}"),
        });
    }

    Ok((
        SceneChainNode {
            id,
            from_ms,
            gap_ms,
            children,
        },
        close_ix,
    ))
}

pub(crate) fn parse_group_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(GroupNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Group")?;
    let brush = attr_value(&open_tag, "brush").map(|v| strip_wrappers(&v).to_string());
    brush_ctx.validate_brush_ref(brush.as_deref(), start + 1)?;
    let mut child_ctx = brush_ctx.with_inherited_brush(brush);
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((parse_group_node(&open_tag, start + 1, children)?, close_ix))
}

pub(crate) fn parse_puppet_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(PuppetNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    if is_self_closing_tag(&open_tag) {
        return Ok((
            parse_puppet_node(&open_tag, start + 1, Vec::new())?,
            open_end_ix,
        ));
    }
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Puppet")?;
    let mut child_ctx = brush_ctx.clone();
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((parse_puppet_node(&open_tag, start + 1, children)?, close_ix))
}

pub(crate) fn parse_mesh_topology_block(
    lines: &[&str],
    start: usize,
) -> Result<(MeshTopologyNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    if is_self_closing_tag(&open_tag) {
        return Ok((
            parse_mesh_topology_node(&open_tag, start + 1, Vec::new())?,
            open_end_ix,
        ));
    }
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "MeshTopology")?;
    let mut children = Vec::<SceneNode>::new();
    let mut i = open_end_ix + 1;
    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Vertex") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            children.push(SceneNode::Vertex(parse_vertex_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Triangle") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            children.push(SceneNode::Triangle(parse_triangle_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Edge") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            children.push(SceneNode::Edge(parse_edge_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Region") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            children.push(SceneNode::Region(parse_region_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!("Unsupported <MeshTopology> child: {line}"),
        });
    }
    Ok((
        parse_mesh_topology_node(&open_tag, start + 1, children)?,
        close_ix,
    ))
}

pub(crate) fn parse_part_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(PartNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Part")?;
    let brush = attr_value(&open_tag, "brush").map(|v| strip_wrappers(&v).to_string());
    brush_ctx.validate_brush_ref(brush.as_deref(), start + 1)?;
    let mut child_ctx = brush_ctx.with_inherited_brush(brush);
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((parse_part_node(&open_tag, start + 1, children)?, close_ix))
}

pub(crate) fn parse_repeat_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(RepeatNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Repeat")?;
    let mut child_ctx = brush_ctx.clone();
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((parse_repeat_node(&open_tag, start + 1, children)?, close_ix))
}

pub(crate) fn parse_mask_any(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(MaskNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    if is_self_closing_tag(&open_tag) {
        return Ok((
            parse_mask_node(&open_tag, start + 1, Vec::new())?,
            open_end_ix,
        ));
    }
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Mask")?;
    let mut child_ctx = brush_ctx.clone();
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((parse_mask_node(&open_tag, start + 1, children)?, close_ix))
}

pub(crate) fn parse_precompose_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(PrecomposeNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    if is_self_closing_tag(&open_tag) {
        let id = strip_wrappers(&required_attr_value(&open_tag, "id", start + 1)?).to_string();
        let duration_ms = if attr_value(&open_tag, "duration").is_some() {
            Some(parse_duration_ms(&open_tag, start + 1, 0)?)
        } else {
            None
        };
        let size = attr_value(&open_tag, "size")
            .as_deref()
            .map(|value| parse_size(value, start + 1, "size"))
            .transpose()?;
        return Ok((
            PrecomposeNode {
                id,
                duration_ms,
                size,
                children: Vec::new(),
            },
            open_end_ix,
        ));
    }
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Precompose")?;
    let id = strip_wrappers(&required_attr_value(&open_tag, "id", start + 1)?).to_string();
    let duration_ms = if attr_value(&open_tag, "duration").is_some() {
        Some(parse_duration_ms(&open_tag, start + 1, 0)?)
    } else {
        None
    };
    let size = attr_value(&open_tag, "size")
        .as_deref()
        .map(|value| parse_size(value, start + 1, "size"))
        .transpose()?;
    let mut child_ctx = brush_ctx.clone();
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((
        PrecomposeNode {
            id,
            duration_ms,
            size,
            children,
        },
        close_ix,
    ))
}

fn parse_use_node(block: &str, line: usize) -> Result<UseNode, GraphParseError> {
    let ref_id = attr_value(block, "ref")
        .map(|v| strip_wrappers(&v).trim_start_matches('#').to_string())
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| GraphParseError {
            line,
            message: "<Use> requires ref=\"component_id\".".to_string(),
        })?;
    Ok(UseNode {
        id: attr_value(block, "id").map(|v| strip_wrappers(&v).to_string()),
        ref_id,
        x: scene_attr_or_default(block, &["x"], "0"),
        y: scene_attr_or_default(block, &["y"], "0"),
        rotation: scene_attr_or_default(block, &["rotation"], "0"),
        scale: scene_attr_or_default(block, &["scale"], "1"),
        scale_x: scene_attr_or_default(block, &["scaleX", "scale_x"], "1"),
        scale_y: scene_attr_or_default(block, &["scaleY", "scale_y"], "1"),
        skew_x: scene_attr_or_default(block, &["skewX", "skew_x"], "0"),
        skew_y: scene_attr_or_default(block, &["skewY", "skew_y"], "0"),
        transform_origin_x: scene_attr_or_default(
            block,
            &["transformOriginX", "transform_origin_x"],
            "0",
        ),
        transform_origin_y: scene_attr_or_default(
            block,
            &["transformOriginY", "transform_origin_y"],
            "0",
        ),
        opacity: scene_attr_or_default(block, &["opacity"], "1"),
        blend: scene_attr_or_default(block, &["blend"], "normal"),
    })
}

pub(crate) fn parse_camera_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(CameraNode, usize), GraphParseError> {
    let _ = brush_ctx;
    let (_open_tag, _open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    Err(GraphParseError {
        line: start + 1,
        message: "<Scene> Camera is an active Camera2D controller and must be self-closing. Use <Track role=\"camera\"><Sequence><Camera ... /></Sequence></Track>. Put visuals in <Track space=\"world\"> or <Track space=\"screen\">.".to_string(),
    })
}

pub(crate) fn parse_character_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(CharacterNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    if is_self_closing_tag(&open_tag) {
        // Image-only characters need no child scene nodes.
        return Ok((
            parse_character_node(&open_tag, start + 1, Vec::new())?,
            open_end_ix,
        ));
    }
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Character")?;
    let mut child_ctx = brush_ctx.clone();
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((
        parse_character_node(&open_tag, start + 1, children)?,
        close_ix,
    ))
}

fn parse_scene_nodes(
    lines: &[&str],
    start: usize,
    end: usize,
    brush_ctx: &mut BrushParseContext,
) -> Result<Vec<SceneNode>, GraphParseError> {
    let mut nodes = Vec::<SceneNode>::new();
    let mut i = start;
    while i < end {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('{') {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Defs") {
            let (defs, end_ix) = parse_defs_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Defs(defs));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Timeline") {
            let (timeline, end_ix) = parse_timeline_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Timeline(timeline));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "PixelGrid") {
            let (grid, end_ix) = parse_pixel_grid_block(lines, i)?;
            nodes.push(SceneNode::PixelGrid(grid));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Solid") {
            return Err(GraphParseError {
                line: i + 1,
                message:
                    "<Solid> has been removed. Use top-level <Background color=\"...\" /> instead."
                        .to_string(),
            });
        }
        if starts_open_tag(line, "Text") {
            let (text, end_ix) = parse_text_any(lines, i)?;
            nodes.push(SceneNode::Text(Box::new(text)));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Image") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            nodes.push(SceneNode::Image(parse_image_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Svg") || starts_open_tag(line, "SVG") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            nodes.push(SceneNode::Svg(parse_svg_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Rect") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            nodes.push(SceneNode::Rect(parse_rect_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Circle") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            nodes.push(SceneNode::Circle(parse_circle_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Line") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            nodes.push(SceneNode::Line(parse_line_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Polyline") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            nodes.push(SceneNode::Polyline(parse_polyline_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Path") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            nodes.push(SceneNode::Path(parse_path_node(&tag, i + 1, brush_ctx)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "FaceJaw") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            nodes.push(SceneNode::FaceJaw(parse_face_jaw_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Shadow") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            nodes.push(SceneNode::Shadow(parse_shadow_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Group") {
            let (group, end_ix) = parse_group_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Group(group));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Puppet") {
            let (puppet, end_ix) = parse_puppet_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Puppet(puppet));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Pin") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            nodes.push(SceneNode::Pin(parse_pin_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "MeshTopology") {
            let (topology, end_ix) = parse_mesh_topology_block(lines, i)?;
            nodes.push(SceneNode::MeshTopology(topology));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Part") {
            let (part, end_ix) = parse_part_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Part(part));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Repeat") {
            let (repeat, end_ix) = parse_repeat_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Repeat(repeat));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Mask") {
            let (mask, end_ix) = parse_mask_any(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Mask(mask));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Precompose") {
            let (precompose, end_ix) = parse_precompose_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Precompose(precompose));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Use") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            nodes.push(SceneNode::Use(parse_use_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Layer3D") {
            let (tag, tag_end_ix) = collect_tag_block(lines, i, '>', false)?;
            if is_self_closing_tag(&tag) {
                nodes.push(SceneNode::Layer(parse_scene_layer_node(
                    &tag,
                    i + 1,
                    Vec::new(),
                    true,
                    true,
                )?));
                i = tag_end_ix + 1;
            } else {
                let (layer, end_ix) =
                    parse_scene_layer_block(lines, i, brush_ctx, "Layer3D", true)?;
                nodes.push(SceneNode::Layer(layer));
                i = end_ix + 1;
            }
            continue;
        }
        if starts_open_tag(line, "Layer") {
            let (tag, tag_end_ix) = collect_tag_block(lines, i, '>', false)?;
            if is_self_closing_tag(&tag) {
                nodes.push(SceneNode::Layer(parse_scene_layer_node(
                    &tag,
                    i + 1,
                    Vec::new(),
                    true,
                    false,
                )?));
                i = tag_end_ix + 1;
            } else {
                let (layer, end_ix) = parse_scene_layer_block(lines, i, brush_ctx, "Layer", false)?;
                nodes.push(SceneNode::Layer(layer));
                i = end_ix + 1;
            }
            continue;
        }
        if starts_open_tag(line, "Character") {
            let (character, end_ix) = parse_character_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Character(character));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Camera") {
            let (tag, tag_end_ix) = collect_tag_block(lines, i, '>', false)?;
            if is_self_closing_tag(&tag) {
                nodes.push(SceneNode::Camera(parse_camera_node(
                    &tag,
                    i + 1,
                    Vec::new(),
                )?));
                i = tag_end_ix + 1;
            } else {
                let (camera, end_ix) = parse_camera_block(lines, i, brush_ctx)?;
                nodes.push(SceneNode::Camera(camera));
                i = end_ix + 1;
            }
            continue;
        }
        i += 1;
    }
    Ok(nodes)
}

pub(crate) fn parse_background_node(
    block: &str,
    line: usize,
) -> Result<BackgroundNode, GraphParseError> {
    let id = attr_value(block, "id")
        .map(|v| strip_wrappers(&v).to_string())
        .or_else(|| Some("background".to_string()));
    let color =
        strip_wrappers(&attr_value(block, "color").unwrap_or_else(|| "#000000".to_string()))
            .to_string();
    if color.is_empty() {
        return Err(GraphParseError {
            line,
            message: "Background color must not be empty.".to_string(),
        });
    }
    Ok(BackgroundNode { id, color })
}

fn scene_attr_or_default(block: &str, names: &[&str], default_value: &str) -> String {
    names
        .iter()
        .find_map(|name| attr_value(block, name))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| default_value.to_string())
}

pub(crate) fn parse_text_node(
    block: &str,
    line: usize,
    layout: Option<TextLayoutNode>,
    animators: Vec<TextAnimatorNode>,
) -> Result<TextNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let value = strip_wrappers(&required_attr_value(block, "value", line)?).to_string();
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "center".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "center".to_string());
    let width = attr_value(block, "width").map(|v| strip_wrappers(&v).to_string());
    let max_width = attr_value(block, "maxWidth")
        .or_else(|| attr_value(block, "max_width"))
        .map(|v| strip_wrappers(&v).to_string());
    let align = attr_value(block, "align").map(|v| strip_wrappers(&v).to_string());
    if let Some(align) = align.as_deref()
        && crate::scene::text::TextAlignMode::parse(align).is_none()
    {
        return Err(GraphParseError {
            line,
            message: format!("Invalid Text align=\"{align}\". Expected left, center, or right."),
        });
    }
    let tracking = attr_value(block, "textGap")
        .or_else(|| attr_value(block, "text_gap"))
        .or_else(|| attr_value(block, "tracking"))
        .map(|v| strip_wrappers(&v).to_string());
    let font_size = attr_value(block, "fontSize")
        .or_else(|| attr_value(block, "font_size"))
        .or_else(|| attr_value(block, "size"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "96".to_string());
    let render_scale = attr_value(block, "renderScale")
        .or_else(|| attr_value(block, "render_scale"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1x".to_string());
    let antialias = attr_value(block, "antialias")
        .or_else(|| attr_value(block, "antiAlias"))
        .or_else(|| attr_value(block, "aa"))
        .map(|v| strip_wrappers(&v).to_string());
    let edge_smoothing = attr_value(block, "edgeSmoothing")
        .or_else(|| attr_value(block, "edge_smoothing"))
        .map(|v| strip_wrappers(&v).to_string());
    let soft_edge = attr_value(block, "softEdge")
        .or_else(|| attr_value(block, "soft_edge"))
        .map(|v| strip_wrappers(&v).to_string());
    let blur = attr_value(block, "blur").map(|v| strip_wrappers(&v).to_string());
    let line_height = attr_value(block, "lineHeight")
        .or_else(|| attr_value(block, "line_height"))
        .map(|v| strip_wrappers(&v).to_string());
    let color = attr_value(block, "color")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "#ffffff".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let box_style = attr_value(block, "box").map(|v| strip_wrappers(&v).to_string());
    let box_color = attr_value(block, "boxColor")
        .or_else(|| attr_value(block, "box_color"))
        .map(|v| strip_wrappers(&v).to_string());
    let box_padding = attr_value(block, "boxPadding")
        .or_else(|| attr_value(block, "box_padding"))
        .map(|v| strip_wrappers(&v).to_string());
    let box_padding_x = attr_value(block, "boxPaddingX")
        .or_else(|| attr_value(block, "box_padding_x"))
        .map(|v| strip_wrappers(&v).to_string());
    let box_padding_y = attr_value(block, "boxPaddingY")
        .or_else(|| attr_value(block, "box_padding_y"))
        .map(|v| strip_wrappers(&v).to_string());
    let box_radius = attr_value(block, "boxRadius")
        .or_else(|| attr_value(block, "box_radius"))
        .map(|v| strip_wrappers(&v).to_string());
    let stroke = attr_value(block, "stroke").map(|v| strip_wrappers(&v).to_string());
    let stroke_width = attr_value(block, "strokeWidth")
        .or_else(|| attr_value(block, "stroke_width"))
        .map(|v| strip_wrappers(&v).to_string());
    let stroke_join = attr_value(block, "strokeJoin")
        .or_else(|| attr_value(block, "stroke_join"))
        .map(|v| strip_wrappers(&v).to_string());
    let stroke_position = attr_value(block, "strokePosition")
        .or_else(|| attr_value(block, "stroke_position"))
        .map(|v| strip_wrappers(&v).to_string());
    let font_family = attr_value(block, "fontFamily")
        .or_else(|| attr_value(block, "font_family"))
        .map(|v| strip_wrappers(&v).to_string());
    let font_weight = attr_value(block, "fontWeight")
        .or_else(|| attr_value(block, "font_weight"))
        .map(|v| strip_wrappers(&v).to_string());
    let font = attr_value(block, "font").map(|v| strip_wrappers(&v).to_string());
    let font_path = attr_value(block, "fontPath")
        .or_else(|| attr_value(block, "font_path"))
        .map(|v| strip_wrappers(&v).to_string());
    let visible_chars = attr_value(block, "visibleChars")
        .or_else(|| attr_value(block, "visible_chars"))
        .map(|v| strip_wrappers(&v).to_string());
    let max_lines = attr_value(block, "maxLines")
        .or_else(|| attr_value(block, "max_lines"))
        .map(|v| strip_wrappers(&v).to_string());

    Ok(TextNode {
        id,
        value,
        x,
        y,
        rotation: scene_attr_or_default(block, &["rotation"], "0"),
        scale: scene_attr_or_default(block, &["scale"], "1"),
        scale_x: scene_attr_or_default(block, &["scaleX", "scale_x"], "1"),
        scale_y: scene_attr_or_default(block, &["scaleY", "scale_y"], "1"),
        skew_x: scene_attr_or_default(block, &["skewX", "skew_x"], "0"),
        skew_y: scene_attr_or_default(block, &["skewY", "skew_y"], "0"),
        transform_origin_x: scene_attr_or_default(
            block,
            &["transformOriginX", "transform_origin_x"],
            "0",
        ),
        transform_origin_y: scene_attr_or_default(
            block,
            &["transformOriginY", "transform_origin_y"],
            "0",
        ),
        width,
        max_width,
        align,
        tracking,
        font_size,
        render_scale,
        antialias,
        edge_smoothing,
        blur,
        soft_edge,
        line_height,
        color,
        opacity,
        box_style,
        box_color,
        box_padding,
        box_padding_x,
        box_padding_y,
        box_radius,
        stroke,
        stroke_width,
        stroke_join,
        stroke_position,
        visible_chars,
        max_lines,
        font,
        font_family,
        font_weight,
        font_path,
        layout,
        animators,
    })
}

fn parse_text_any(lines: &[&str], start: usize) -> Result<(TextNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    if is_self_closing_tag(&open_tag) {
        return Ok((
            parse_text_node(&open_tag, start + 1, None, Vec::new())?,
            open_end_ix,
        ));
    }

    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Text")?;
    let (layout, animators) = parse_text_children(lines, open_end_ix + 1, close_ix)?;
    Ok((
        parse_text_node(&open_tag, start + 1, layout, animators)?,
        close_ix,
    ))
}

fn parse_text_children(
    lines: &[&str],
    start: usize,
    end: usize,
) -> Result<(Option<TextLayoutNode>, Vec<TextAnimatorNode>), GraphParseError> {
    let mut layout = None;
    let mut animators = Vec::new();
    let mut i = start;
    while i < end {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with("<!--") {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "TextLayout") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            if layout.is_some() {
                return Err(GraphParseError {
                    line: i + 1,
                    message: "<Text> accepts at most one <TextLayout /> child.".to_string(),
                });
            }
            layout = Some(parse_text_layout_node(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "TextAnimator") {
            let (animator, end_ix) = parse_text_animator_any(lines, i)?;
            animators.push(animator);
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!(
                "<Text> only accepts <TextLayout /> and <TextAnimator> children, got: {line}"
            ),
        });
    }
    Ok((layout, animators))
}

fn parse_text_layout_node(block: &str, line: usize) -> Result<TextLayoutNode, GraphParseError> {
    let wrap = attr_value(block, "wrap")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "normal".to_string());
    if crate::scene::text::TextWrapMode::parse(&wrap).is_none() {
        return Err(GraphParseError {
            line,
            message: format!(
                "Invalid TextLayout wrap=\"{wrap}\". Expected none, normal, or balance."
            ),
        });
    }
    let overflow = attr_value(block, "overflow")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "clip".to_string());
    if crate::scene::text::TextOverflowMode::parse(&overflow).is_none() {
        return Err(GraphParseError {
            line,
            message: format!(
                "Invalid TextLayout overflow=\"{overflow}\". Expected clip, fit, or ellipsis."
            ),
        });
    }
    let safe_area = attr_value(block, "safeArea")
        .or_else(|| attr_value(block, "safe_area"))
        .map(|v| strip_wrappers(&v).to_string());
    if let Some(safe_area) = safe_area.as_deref() {
        crate::scene::text::parse_safe_area(safe_area)
            .map_err(|message| GraphParseError { line, message })?;
    }
    let max_lines = attr_value(block, "maxLines")
        .or_else(|| attr_value(block, "max_lines"))
        .map(|v| strip_wrappers(&v).to_string());
    let align = attr_value(block, "align").map(|v| strip_wrappers(&v).to_string());
    if let Some(align) = align.as_deref()
        && crate::scene::text::TextAlignMode::parse(align).is_none()
    {
        return Err(GraphParseError {
            line,
            message: format!(
                "Invalid TextLayout align=\"{align}\". Expected left, center, or right."
            ),
        });
    }

    Ok(TextLayoutNode {
        wrap,
        overflow,
        safe_area,
        max_lines,
        align,
    })
}

fn parse_text_animator_any(
    lines: &[&str],
    start: usize,
) -> Result<(TextAnimatorNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    if is_self_closing_tag(&open_tag) {
        return Ok((
            parse_text_animator_node(&open_tag, start + 1, None, None, Vec::new())?,
            open_end_ix,
        ));
    }

    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "TextAnimator")?;
    let (transform, style, effects) =
        parse_text_animator_children(lines, open_end_ix + 1, close_ix)?;
    Ok((
        parse_text_animator_node(&open_tag, start + 1, transform, style, effects)?,
        close_ix,
    ))
}

type TextAnimatorChildren = (
    Option<TextTransformNode>,
    Option<TextStyleOverrideNode>,
    Vec<TextEffectNode>,
);

fn parse_text_animator_children(
    lines: &[&str],
    start: usize,
    end: usize,
) -> Result<TextAnimatorChildren, GraphParseError> {
    let mut transform = None;
    let mut style = None;
    let mut effects = Vec::new();
    let mut i = start;
    while i < end {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with("<!--") {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Transform") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            if transform.is_some() {
                return Err(GraphParseError {
                    line: i + 1,
                    message: "<TextAnimator> accepts at most one <Transform /> child.".to_string(),
                });
            }
            transform = Some(parse_text_transform_node(&tag));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Style") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            if style.is_some() {
                return Err(GraphParseError {
                    line: i + 1,
                    message: "<TextAnimator> accepts at most one <Style /> child.".to_string(),
                });
            }
            style = Some(parse_text_style_override_node(&tag));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Effects") {
            let (mut parsed_effects, end_ix) = parse_text_effects_block(lines, i)?;
            effects.append(&mut parsed_effects);
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!(
                "<TextAnimator> only accepts <Transform />, <Style />, and <Effects> children, got: {line}"
            ),
        });
    }
    Ok((transform, style, effects))
}

fn parse_text_animator_node(
    block: &str,
    line: usize,
    transform: Option<TextTransformNode>,
    style: Option<TextStyleOverrideNode>,
    effects: Vec<TextEffectNode>,
) -> Result<TextAnimatorNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let selector_raw = attr_value(block, "selector")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "word".to_string());
    let selector = TextSelectorKind::parse(&selector_raw).ok_or_else(|| GraphParseError {
        line,
        message: format!(
            "Invalid TextAnimator selector=\"{selector_raw}\". Expected char, word, line, or range."
        ),
    })?;
    let mode = attr_value(block, "mode")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "normal".to_string());
    if !matches!(mode.as_str(), "normal" | "karaoke") {
        return Err(GraphParseError {
            line,
            message: format!("Invalid TextAnimator mode=\"{mode}\". Expected normal or karaoke."),
        });
    }
    let from_ms = attr_value(block, "from")
        .map(|v| parse_signed_time_ms(&v, line, "TextAnimator.from"))
        .transpose()?
        .unwrap_or(0);
    let duration_ms = attr_value(block, "duration")
        .map(|v| parse_signed_time_ms(&v, line, "TextAnimator.duration"))
        .transpose()?
        .map(|value| value.max(0) as u64);
    let stagger_ms = attr_value(block, "stagger")
        .map(|v| parse_signed_time_ms(&v, line, "TextAnimator.stagger"))
        .transpose()?
        .unwrap_or(0);
    let order = attr_value(block, "order")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "forward".to_string());
    if !matches!(order.as_str(), "forward" | "reverse" | "random") {
        return Err(GraphParseError {
            line,
            message: format!(
                "Invalid TextAnimator order=\"{order}\". Expected forward, reverse, or random."
            ),
        });
    }
    let pre_roll_ms = attr_value(block, "preRoll")
        .or_else(|| attr_value(block, "pre_roll"))
        .map(|v| parse_signed_time_ms(&v, line, "TextAnimator.preRoll"))
        .transpose()?
        .unwrap_or(0);
    let post_roll_ms = attr_value(block, "postRoll")
        .or_else(|| attr_value(block, "post_roll"))
        .map(|v| parse_signed_time_ms(&v, line, "TextAnimator.postRoll"))
        .transpose()?
        .unwrap_or(0);
    let active_word = attr_value(block, "activeWord")
        .or_else(|| attr_value(block, "active_word"))
        .map(|v| strip_wrappers(&v).to_string());
    let random_seed = attr_value(block, "randomSeed")
        .or_else(|| attr_value(block, "random_seed"))
        .map(|v| {
            let text = strip_wrappers(&v);
            text.parse::<u64>().map_err(|_| GraphParseError {
                line,
                message: format!("Invalid TextAnimator randomSeed value: {text}"),
            })
        })
        .transpose()?;
    let range = attr_value(block, "range").map(|v| strip_wrappers(&v).to_string());

    Ok(TextAnimatorNode {
        id,
        selector,
        mode,
        from_ms,
        duration_ms,
        stagger_ms,
        order,
        pre_roll_ms,
        post_roll_ms,
        active_word,
        random_seed,
        range,
        transform,
        style,
        effects,
    })
}

fn parse_text_transform_node(block: &str) -> TextTransformNode {
    TextTransformNode {
        x: attr_value(block, "x").map(|v| strip_wrappers(&v).to_string()),
        y: attr_value(block, "y").map(|v| strip_wrappers(&v).to_string()),
        rotation: attr_value(block, "rotation").map(|v| strip_wrappers(&v).to_string()),
        scale: attr_value(block, "scale").map(|v| strip_wrappers(&v).to_string()),
        scale_x: attr_value(block, "scaleX")
            .or_else(|| attr_value(block, "scale_x"))
            .map(|v| strip_wrappers(&v).to_string()),
        scale_y: attr_value(block, "scaleY")
            .or_else(|| attr_value(block, "scale_y"))
            .map(|v| strip_wrappers(&v).to_string()),
        skew_x: attr_value(block, "skewX")
            .or_else(|| attr_value(block, "skew_x"))
            .map(|v| strip_wrappers(&v).to_string()),
        skew_y: attr_value(block, "skewY")
            .or_else(|| attr_value(block, "skew_y"))
            .map(|v| strip_wrappers(&v).to_string()),
    }
}

fn parse_text_style_override_node(block: &str) -> TextStyleOverrideNode {
    TextStyleOverrideNode {
        color: attr_value(block, "color").map(|v| strip_wrappers(&v).to_string()),
        opacity: attr_value(block, "opacity").map(|v| strip_wrappers(&v).to_string()),
        blur: attr_value(block, "blur").map(|v| strip_wrappers(&v).to_string()),
        stroke: attr_value(block, "stroke").map(|v| strip_wrappers(&v).to_string()),
        stroke_width: attr_value(block, "strokeWidth")
            .or_else(|| attr_value(block, "stroke_width"))
            .map(|v| strip_wrappers(&v).to_string()),
        stroke_join: attr_value(block, "strokeJoin")
            .or_else(|| attr_value(block, "stroke_join"))
            .map(|v| strip_wrappers(&v).to_string()),
        stroke_position: attr_value(block, "strokePosition")
            .or_else(|| attr_value(block, "stroke_position"))
            .map(|v| strip_wrappers(&v).to_string()),
        shadow_color: attr_value(block, "shadowColor")
            .or_else(|| attr_value(block, "shadow_color"))
            .map(|v| strip_wrappers(&v).to_string()),
        shadow_x: attr_value(block, "shadowX")
            .or_else(|| attr_value(block, "shadow_x"))
            .map(|v| strip_wrappers(&v).to_string()),
        shadow_y: attr_value(block, "shadowY")
            .or_else(|| attr_value(block, "shadow_y"))
            .map(|v| strip_wrappers(&v).to_string()),
        shadow_blur: attr_value(block, "shadowBlur")
            .or_else(|| attr_value(block, "shadow_blur"))
            .map(|v| strip_wrappers(&v).to_string()),
    }
}

fn parse_text_effects_block(
    lines: &[&str],
    start: usize,
) -> Result<(Vec<TextEffectNode>, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    if is_self_closing_tag(&open_tag) {
        return Ok((Vec::new(), open_end_ix));
    }
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Effects")?;
    let mut effects = Vec::new();
    let mut i = open_end_ix + 1;
    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with("<!--") {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Glow") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            effects.push(TextEffectNode::Glow(parse_text_glow_effect_node(
                &tag,
                i + 1,
            )?));
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!("<Effects> only accepts <Glow /> children for Text, got: {line}"),
        });
    }
    Ok((effects, close_ix))
}

fn parse_text_glow_effect_node(
    block: &str,
    line: usize,
) -> Result<TextGlowEffectNode, GraphParseError> {
    let radius = attr_value(block, "radius")
        .map(|v| strip_wrappers(&v).to_string())
        .ok_or_else(|| GraphParseError {
            line,
            message: "<Glow> requires radius=\"...\".".to_string(),
        })?;
    let intensity = attr_value(block, "intensity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1".to_string());
    let color = attr_value(block, "color").map(|v| strip_wrappers(&v).to_string());
    Ok(TextGlowEffectNode {
        radius,
        intensity,
        color,
    })
}

pub(crate) fn parse_image_node(block: &str, line: usize) -> Result<ImageNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let src = strip_wrappers(&required_attr_value_any(block, &["src", "path"], line)?).to_string();
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "center".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "center".to_string());
    let scale = attr_value(block, "scale")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());

    Ok(ImageNode {
        id,
        src,
        x,
        y,
        scale,
        opacity,
    })
}

pub(crate) fn parse_svg_node(block: &str, line: usize) -> Result<SvgNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let src = strip_wrappers(&required_attr_value_any(block, &["src", "path"], line)?).to_string();
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "center".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "center".to_string());
    let scale = attr_value(block, "scale")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());

    Ok(SvgNode {
        id,
        src,
        x,
        y,
        scale,
        opacity,
    })
}

pub(crate) fn parse_defs_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &mut BrushParseContext,
) -> Result<(DefsNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Defs")?;
    let id = attr_value(&open_tag, "id").map(|v| strip_wrappers(&v).to_string());
    let mut gradients = Vec::<GradientDef>::new();
    let mut textures = Vec::<TextureDef>::new();
    let mut brushes = Vec::<BrushDef>::new();
    let mut masks = Vec::<MaskNode>::new();
    let mut precomposes = Vec::<PrecomposeNode>::new();
    let mut components = Vec::<ComponentNode>::new();
    let mut filters = Vec::<FilterDef>::new();
    let mut fonts = Vec::<FontDef>::new();
    let mut palettes = Vec::<PaletteNode>::new();
    let mut i = open_end_ix + 1;

    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty()
            || line.starts_with("//")
            || line.starts_with('{')
            || line.starts_with("<!--")
        {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "LinearGradient") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            gradients.push(GradientDef::Linear(parse_linear_gradient_def(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "RadialGradient") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            gradients.push(GradientDef::Radial(parse_radial_gradient_def(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Texture") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            textures.push(parse_texture_def(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Brush") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            let brush = parse_brush_def(&tag, i + 1)?;
            brush_ctx.define_brushes(std::slice::from_ref(&brush));
            brushes.push(brush);
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Mask") {
            let (mask, end_ix) = parse_mask_any(lines, i, brush_ctx)?;
            masks.push(mask);
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Precompose") {
            let (precompose, end_ix) = parse_precompose_block(lines, i, brush_ctx)?;
            precomposes.push(precompose);
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Component") {
            let (component, end_ix) = parse_component_block(lines, i, brush_ctx)?;
            components.push(component);
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Filter") {
            let (filter, end_ix) = parse_filter_block(lines, i)?;
            filters.push(filter);
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Font") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            fonts.push(parse_font_def(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Palette") {
            let (palette, end_ix) = parse_palette_block(lines, i)?;
            palettes.push(palette);
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!(
                "<Defs> only accepts resource tags: <LinearGradient />, <RadialGradient />, <Texture />, <Brush />, <Mask>, <Precompose>, <Component>, <Filter>, <Font />, or <Palette>, got: {line}"
            ),
        });
    }

    Ok((
        DefsNode {
            id,
            gradients,
            textures,
            brushes,
            masks,
            precomposes,
            components,
            filters,
            fonts,
            palettes,
        },
        close_ix,
    ))
}

fn parse_texture_def(block: &str, line: usize) -> Result<TextureDef, GraphParseError> {
    let id = strip_wrappers(&required_attr_value(block, "id", line)?).to_string();
    let kind = attr_value(block, "kind")
        .or_else(|| attr_value(block, "type"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "paper".to_string());
    Ok(TextureDef {
        id,
        src: scene_attr_or_default(block, &["src", "source", "href"], ""),
        kind,
        scale: scene_attr_or_default(block, &["scale"], "42"),
        strength: scene_attr_or_default(block, &["strength", "amount"], "0.25"),
        contrast: scene_attr_or_default(block, &["contrast"], "0.5"),
        seed: scene_attr_or_default(block, &["seed"], "0"),
        brush_angle: scene_attr_or_default(block, &["brushAngle", "brush_angle", "angle"], "-8"),
        bump_strength: scene_attr_or_default(
            block,
            &["bumpStrength", "bump_strength", "bump", "impastoStrength"],
            "0.35",
        ),
        relief: scene_attr_or_default(block, &["relief"], "0.45"),
    })
}

fn parse_component_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(ComponentNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Component")?;
    let id = strip_wrappers(&required_attr_value(&open_tag, "id", start + 1)?).to_string();
    let mut child_ctx = brush_ctx.clone();
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((ComponentNode { id, children }, close_ix))
}

fn parse_filter_block(lines: &[&str], start: usize) -> Result<(FilterDef, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Filter")?;
    let id = strip_wrappers(&required_attr_value(&open_tag, "id", start + 1)?).to_string();
    let mut steps = Vec::<FilterStepDef>::new();
    let mut i = open_end_ix + 1;

    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('{') {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Blur") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            steps.push(parse_filter_step_def("blur", &tag));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "ColorMatrix") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            steps.push(parse_filter_step_def("colorMatrix", &tag));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Effect") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            let kind = attr_value(&tag, "type")
                .or_else(|| attr_value(&tag, "effect"))
                .map(|v| strip_wrappers(&v).to_string())
                .unwrap_or_else(|| "effect".to_string());
            steps.push(parse_filter_step_def(&kind, &tag));
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!(
                "<Filter> only accepts <Blur />, <ColorMatrix />, or <Effect />, got: {line}"
            ),
        });
    }

    Ok((FilterDef { id, steps }, close_ix))
}

fn parse_filter_step_def(kind: &str, block: &str) -> FilterStepDef {
    FilterStepDef {
        kind: kind.to_string(),
        radius: attr_value(block, "radius")
            .or_else(|| attr_value(block, "sigma"))
            .map(|v| strip_wrappers(&v).to_string()),
        saturation: attr_value(block, "saturation").map(|v| strip_wrappers(&v).to_string()),
        brightness: attr_value(block, "brightness").map(|v| strip_wrappers(&v).to_string()),
        contrast: attr_value(block, "contrast").map(|v| strip_wrappers(&v).to_string()),
        opacity: attr_value(block, "opacity").map(|v| strip_wrappers(&v).to_string()),
    }
}

fn parse_font_def(block: &str, line: usize) -> Result<FontDef, GraphParseError> {
    let id = strip_wrappers(&required_attr_value(block, "id", line)?).to_string();
    Ok(FontDef {
        id,
        family: attr_value(block, "family")
            .or_else(|| attr_value(block, "fontFamily"))
            .or_else(|| attr_value(block, "font_family"))
            .map(|v| strip_wrappers(&v).to_string()),
        path: attr_value(block, "path")
            .or_else(|| attr_value(block, "fontPath"))
            .or_else(|| attr_value(block, "font_path"))
            .map(|v| strip_wrappers(&v).to_string()),
        fallback: attr_value(block, "fallback").map(|v| strip_wrappers(&v).to_string()),
    })
}

fn parse_palette_block(
    lines: &[&str],
    start: usize,
) -> Result<(PaletteNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Palette")?;
    let id = strip_wrappers(&required_attr_value(&open_tag, "id", start + 1)?).to_string();
    let mut colors = Vec::<PaletteColorDef>::new();
    let mut i = open_end_ix + 1;

    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('{') {
            i += 1;
            continue;
        }
        if starts_open_tag(line, "Color") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            colors.push(parse_palette_color_def(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!("<Palette> only accepts <Color />, got: {line}"),
        });
    }

    Ok((PaletteNode { id, colors }, close_ix))
}

fn parse_palette_color_def(block: &str, line: usize) -> Result<PaletteColorDef, GraphParseError> {
    Ok(PaletteColorDef {
        key: strip_wrappers(&required_attr_value(block, "key", line)?).to_string(),
        value: strip_wrappers(&required_attr_value(block, "value", line)?).to_string(),
    })
}

pub(crate) fn parse_pixel_grid_block(
    lines: &[&str],
    start: usize,
) -> Result<(PixelGridNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "PixelGrid")?;
    let data = parse_pixel_grid_data(&lines[open_end_ix + 1..close_ix]);

    Ok((
        PixelGridNode {
            id: attr_value(&open_tag, "id").map(|v| strip_wrappers(&v).to_string()),
            x: attr_value(&open_tag, "x")
                .map(|v| strip_wrappers(&v).to_string())
                .unwrap_or_else(|| "0".to_string()),
            y: attr_value(&open_tag, "y")
                .map(|v| strip_wrappers(&v).to_string())
                .unwrap_or_else(|| "0".to_string()),
            pixel_size: attr_value(&open_tag, "pixelSize")
                .or_else(|| attr_value(&open_tag, "pixel_size"))
                .map(|v| strip_wrappers(&v).to_string())
                .unwrap_or_else(|| "1".to_string()),
            palette: strip_wrappers(&required_attr_value(&open_tag, "palette", start + 1)?)
                .to_string(),
            opacity: attr_value(&open_tag, "opacity")
                .map(|v| strip_wrappers(&v).to_string())
                .unwrap_or_else(|| "1".to_string()),
            blend: attr_value(&open_tag, "blend")
                .map(|v| strip_wrappers(&v).to_string())
                .unwrap_or_else(|| "normal".to_string()),
            data,
        },
        close_ix,
    ))
}

fn parse_pixel_grid_data(lines: &[&str]) -> String {
    let mut body = lines.join("\n");
    if let Some(start) = body.find("<![CDATA[") {
        body = body[start + "<![CDATA[".len()..].to_string();
    }
    if let Some(end) = body.rfind("]]>") {
        body.truncate(end);
    }
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_brush_def(block: &str, line: usize) -> Result<BrushDef, GraphParseError> {
    let id = strip_wrappers(&required_attr_value(block, "id", line)?).to_string();
    Ok(BrushDef {
        id,
        stroke: attr_value(block, "stroke")
            .or_else(|| attr_value(block, "color"))
            .map(|v| strip_wrappers(&v).to_string()),
        fill: attr_value(block, "fill").map(|v| strip_wrappers(&v).to_string()),
        stroke_width: attr_value(block, "strokeWidth")
            .or_else(|| attr_value(block, "stroke_width"))
            .or_else(|| attr_value(block, "width"))
            .map(|v| strip_wrappers(&v).to_string()),
        opacity: attr_value(block, "opacity").map(|v| strip_wrappers(&v).to_string()),
        line_cap: attr_value(block, "lineCap")
            .or_else(|| attr_value(block, "line_cap"))
            .map(|v| strip_wrappers(&v).to_string()),
        line_join: attr_value(block, "lineJoin")
            .or_else(|| attr_value(block, "line_join"))
            .map(|v| strip_wrappers(&v).to_string()),
        taper_start: attr_value(block, "taperStart")
            .or_else(|| attr_value(block, "taper_start"))
            .map(|v| strip_wrappers(&v).to_string()),
        taper_end: attr_value(block, "taperEnd")
            .or_else(|| attr_value(block, "taper_end"))
            .map(|v| strip_wrappers(&v).to_string()),
        stroke_style: attr_value(block, "strokeStyle")
            .or_else(|| attr_value(block, "stroke_style"))
            .or_else(|| attr_value(block, "style"))
            .map(|v| strip_wrappers(&v).to_string()),
        stroke_roughness: attr_value(block, "strokeRoughness")
            .or_else(|| attr_value(block, "stroke_roughness"))
            .or_else(|| attr_value(block, "roughness"))
            .map(|v| strip_wrappers(&v).to_string()),
        stroke_copies: attr_value(block, "strokeCopies")
            .or_else(|| attr_value(block, "stroke_copies"))
            .or_else(|| attr_value(block, "copies"))
            .map(|v| strip_wrappers(&v).to_string()),
        stroke_texture: attr_value(block, "strokeTexture")
            .or_else(|| attr_value(block, "stroke_texture"))
            .or_else(|| attr_value(block, "texture"))
            .map(|v| strip_wrappers(&v).to_string()),
        stroke_bristles: attr_value(block, "strokeBristles")
            .or_else(|| attr_value(block, "stroke_bristles"))
            .or_else(|| attr_value(block, "bristles"))
            .map(|v| strip_wrappers(&v).to_string()),
        stroke_pressure: attr_value(block, "strokePressure")
            .or_else(|| attr_value(block, "stroke_pressure"))
            .or_else(|| attr_value(block, "pressure"))
            .map(|v| strip_wrappers(&v).to_string()),
        stroke_pressure_min: attr_value(block, "strokePressureMin")
            .or_else(|| attr_value(block, "stroke_pressure_min"))
            .or_else(|| attr_value(block, "pressureMin"))
            .or_else(|| attr_value(block, "pressure_min"))
            .map(|v| strip_wrappers(&v).to_string()),
        stroke_pressure_curve: attr_value(block, "strokePressureCurve")
            .or_else(|| attr_value(block, "stroke_pressure_curve"))
            .or_else(|| attr_value(block, "pressureCurve"))
            .or_else(|| attr_value(block, "pressure_curve"))
            .map(|v| strip_wrappers(&v).to_string()),
        blend: attr_value(block, "blend").map(|v| strip_wrappers(&v).to_string()),
    })
}

fn parse_linear_gradient_def(
    block: &str,
    line: usize,
) -> Result<LinearGradientDef, GraphParseError> {
    let id = strip_wrappers(&required_attr_value(block, "id", line)?).to_string();
    let x1 = attr_value(block, "x1")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let y1 = attr_value(block, "y1")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let x2 = attr_value(block, "x2")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1".to_string());
    let y2 = attr_value(block, "y2")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let stops_raw = strip_wrappers(&required_attr_value(block, "stops", line)?).to_string();
    let stops = parse_gradient_stops(&stops_raw, line)?;
    let units = attr_value(block, "units")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "objectBoundingBox".to_string());
    Ok(LinearGradientDef {
        id,
        x1,
        y1,
        x2,
        y2,
        stops,
        units,
    })
}

fn parse_radial_gradient_def(
    block: &str,
    line: usize,
) -> Result<RadialGradientDef, GraphParseError> {
    let id = strip_wrappers(&required_attr_value(block, "id", line)?).to_string();
    let cx = attr_value(block, "cx")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0.5".to_string());
    let cy = attr_value(block, "cy")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0.5".to_string());
    let r = attr_value(block, "r")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0.5".to_string());
    let fx = attr_value(block, "fx").map(|v| strip_wrappers(&v).to_string());
    let fy = attr_value(block, "fy").map(|v| strip_wrappers(&v).to_string());
    let stops_raw = strip_wrappers(&required_attr_value(block, "stops", line)?).to_string();
    let stops = parse_gradient_stops(&stops_raw, line)?;
    let units = attr_value(block, "units")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "objectBoundingBox".to_string());
    Ok(RadialGradientDef {
        id,
        cx,
        cy,
        r,
        fx,
        fy,
        stops,
        units,
    })
}

fn parse_gradient_stops(raw: &str, line: usize) -> Result<Vec<GradientStop>, GraphParseError> {
    let mut stops = Vec::<GradientStop>::new();
    for token in raw.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let Some((offset_raw, color_raw)) = token.split_once(':') else {
            return Err(GraphParseError {
                line,
                message: format!("gradient stop must be 'offset:color', got: {token}"),
            });
        };
        let offset = offset_raw
            .trim()
            .parse::<f32>()
            .map_err(|_| GraphParseError {
                line,
                message: format!("invalid gradient stop offset: {}", offset_raw.trim()),
            })?
            .clamp(0.0, 1.0);
        let color = color_raw.trim();
        if color.is_empty() {
            return Err(GraphParseError {
                line,
                message: "gradient stop color cannot be empty".to_string(),
            });
        }
        stops.push(GradientStop {
            offset,
            color: color.to_string(),
        });
    }
    if stops.len() < 2 {
        return Err(GraphParseError {
            line,
            message: "gradient requires at least two stops".to_string(),
        });
    }
    stops.sort_by(|a, b| {
        a.offset
            .partial_cmp(&b.offset)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(stops)
}

pub(crate) fn parse_rect_node(block: &str, line: usize) -> Result<RectNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let width = strip_wrappers(&required_attr_value(block, "width", line)?).to_string();
    let height = strip_wrappers(&required_attr_value(block, "height", line)?).to_string();
    let radius = attr_value(block, "radius")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let color = attr_value(block, "color")
        .or_else(|| attr_value(block, "fill"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "#ffffff".to_string());
    let stroke = attr_value(block, "stroke").map(|v| strip_wrappers(&v).to_string());
    let stroke_width = attr_value(block, "strokeWidth")
        .or_else(|| attr_value(block, "stroke_width"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let rotation = attr_value(block, "rotation")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());

    Ok(RectNode {
        id,
        x,
        y,
        width,
        height,
        radius,
        color,
        stroke,
        stroke_width,
        opacity,
        rotation,
        scale: scene_attr_or_default(block, &["scale"], "1"),
        scale_x: scene_attr_or_default(block, &["scaleX", "scale_x"], "1"),
        scale_y: scene_attr_or_default(block, &["scaleY", "scale_y"], "1"),
        skew_x: scene_attr_or_default(block, &["skewX", "skew_x"], "0"),
        skew_y: scene_attr_or_default(block, &["skewY", "skew_y"], "0"),
        transform_origin_x: scene_attr_or_default(
            block,
            &["transformOriginX", "transform_origin_x"],
            "0",
        ),
        transform_origin_y: scene_attr_or_default(
            block,
            &["transformOriginY", "transform_origin_y"],
            "0",
        ),
        blend: attr_value(block, "blend")
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "normal".to_string()),
        texture: attr_value(block, "texture").map(|v| strip_wrappers(&v).to_string()),
        texture_opacity: scene_attr_or_default(block, &["textureOpacity", "texture_opacity"], "1"),
        texture_scale: scene_attr_or_default(block, &["textureScale", "texture_scale"], "1"),
        texture_mask: scene_attr_or_default(block, &["textureMask", "texture_mask"], "0"),
    })
}

pub(crate) fn parse_circle_node(block: &str, line: usize) -> Result<CircleNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let radius = strip_wrappers(&required_attr_value(block, "radius", line)?).to_string();
    let color = attr_value(block, "color")
        .or_else(|| attr_value(block, "fill"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "#ffffff".to_string());
    let stroke = attr_value(block, "stroke").map(|v| strip_wrappers(&v).to_string());
    let stroke_width = attr_value(block, "strokeWidth")
        .or_else(|| attr_value(block, "stroke_width"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    Ok(CircleNode {
        id,
        x,
        y,
        radius,
        color,
        stroke,
        stroke_width,
        opacity,
        rotation: scene_attr_or_default(block, &["rotation"], "0"),
        scale: scene_attr_or_default(block, &["scale"], "1"),
        scale_x: scene_attr_or_default(block, &["scaleX", "scale_x"], "1"),
        scale_y: scene_attr_or_default(block, &["scaleY", "scale_y"], "1"),
        skew_x: scene_attr_or_default(block, &["skewX", "skew_x"], "0"),
        skew_y: scene_attr_or_default(block, &["skewY", "skew_y"], "0"),
        transform_origin_x: scene_attr_or_default(
            block,
            &["transformOriginX", "transform_origin_x"],
            "0",
        ),
        transform_origin_y: scene_attr_or_default(
            block,
            &["transformOriginY", "transform_origin_y"],
            "0",
        ),
        blend: attr_value(block, "blend")
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "normal".to_string()),
        texture: attr_value(block, "texture").map(|v| strip_wrappers(&v).to_string()),
        texture_opacity: scene_attr_or_default(block, &["textureOpacity", "texture_opacity"], "1"),
        texture_scale: scene_attr_or_default(block, &["textureScale", "texture_scale"], "1"),
        texture_mask: scene_attr_or_default(block, &["textureMask", "texture_mask"], "0"),
    })
}

#[derive(Debug, Clone)]
struct StrokeAttrs {
    style: String,
    roughness: String,
    copies: String,
    texture: String,
    bristles: String,
    pressure: String,
    pressure_min: String,
    pressure_curve: String,
}

fn stroke_style_attrs(block: &str) -> StrokeAttrs {
    stroke_style_attrs_with_brush(block, None)
}

fn attr_string(block: &str, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| attr_value(block, name))
        .map(|v| strip_wrappers(&v).to_string())
}

fn attr_or_brush(
    block: &str,
    names: &[&str],
    brush_value: Option<&String>,
    default_value: &str,
) -> String {
    attr_string(block, names)
        .or_else(|| brush_value.cloned())
        .unwrap_or_else(|| default_value.to_string())
}

fn stroke_style_attrs_with_brush(block: &str, brush: Option<&BrushDef>) -> StrokeAttrs {
    let stroke_style = attr_value(block, "strokeStyle")
        .or_else(|| attr_value(block, "stroke_style"))
        .or_else(|| attr_value(block, "style"))
        .map(|v| strip_wrappers(&v).to_string())
        .or_else(|| brush.and_then(|brush| brush.stroke_style.clone()))
        .unwrap_or_else(|| "solid".to_string());
    let stroke_roughness = attr_value(block, "strokeRoughness")
        .or_else(|| attr_value(block, "stroke_roughness"))
        .or_else(|| attr_value(block, "roughness"))
        .map(|v| strip_wrappers(&v).to_string())
        .or_else(|| brush.and_then(|brush| brush.stroke_roughness.clone()))
        .unwrap_or_else(|| "0".to_string());
    let stroke_copies = attr_value(block, "strokeCopies")
        .or_else(|| attr_value(block, "stroke_copies"))
        .or_else(|| attr_value(block, "copies"))
        .map(|v| strip_wrappers(&v).to_string())
        .or_else(|| brush.and_then(|brush| brush.stroke_copies.clone()))
        .unwrap_or_else(|| "1".to_string());
    let stroke_texture = attr_value(block, "strokeTexture")
        .or_else(|| attr_value(block, "stroke_texture"))
        .or_else(|| attr_value(block, "texture"))
        .map(|v| strip_wrappers(&v).to_string())
        .or_else(|| brush.and_then(|brush| brush.stroke_texture.clone()))
        .unwrap_or_else(|| "0".to_string());
    let stroke_bristles = attr_value(block, "strokeBristles")
        .or_else(|| attr_value(block, "stroke_bristles"))
        .or_else(|| attr_value(block, "bristles"))
        .map(|v| strip_wrappers(&v).to_string())
        .or_else(|| brush.and_then(|brush| brush.stroke_bristles.clone()))
        .unwrap_or_else(|| "0".to_string());
    let stroke_pressure = attr_value(block, "strokePressure")
        .or_else(|| attr_value(block, "stroke_pressure"))
        .or_else(|| attr_value(block, "pressure"))
        .map(|v| strip_wrappers(&v).to_string())
        .or_else(|| brush.and_then(|brush| brush.stroke_pressure.clone()))
        .unwrap_or_else(|| "none".to_string());
    let stroke_pressure_min = attr_value(block, "strokePressureMin")
        .or_else(|| attr_value(block, "stroke_pressure_min"))
        .or_else(|| attr_value(block, "pressureMin"))
        .or_else(|| attr_value(block, "pressure_min"))
        .map(|v| strip_wrappers(&v).to_string())
        .or_else(|| brush.and_then(|brush| brush.stroke_pressure_min.clone()))
        .unwrap_or_else(|| "1".to_string());
    let stroke_pressure_curve = attr_value(block, "strokePressureCurve")
        .or_else(|| attr_value(block, "stroke_pressure_curve"))
        .or_else(|| attr_value(block, "pressureCurve"))
        .or_else(|| attr_value(block, "pressure_curve"))
        .map(|v| strip_wrappers(&v).to_string())
        .or_else(|| brush.and_then(|brush| brush.stroke_pressure_curve.clone()))
        .unwrap_or_else(|| "1".to_string());
    StrokeAttrs {
        style: stroke_style,
        roughness: stroke_roughness,
        copies: stroke_copies,
        texture: stroke_texture,
        bristles: stroke_bristles,
        pressure: stroke_pressure,
        pressure_min: stroke_pressure_min,
        pressure_curve: stroke_pressure_curve,
    }
}

pub(crate) fn parse_line_node(block: &str, line: usize) -> Result<LineNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let x1 = strip_wrappers(&required_attr_value(block, "x1", line)?).to_string();
    let y1 = strip_wrappers(&required_attr_value(block, "y1", line)?).to_string();
    let x2 = strip_wrappers(&required_attr_value(block, "x2", line)?).to_string();
    let y2 = strip_wrappers(&required_attr_value(block, "y2", line)?).to_string();
    let width = attr_value(block, "width")
        .or_else(|| attr_value(block, "strokeWidth"))
        .or_else(|| attr_value(block, "stroke_width"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "4".to_string());
    let color = attr_value(block, "color")
        .or_else(|| attr_value(block, "stroke"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "#ffffff".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let stroke_attrs = stroke_style_attrs(block);
    Ok(LineNode {
        id,
        x: scene_attr_or_default(block, &["x"], "0"),
        y: scene_attr_or_default(block, &["y"], "0"),
        rotation: scene_attr_or_default(block, &["rotation"], "0"),
        scale: scene_attr_or_default(block, &["scale"], "1"),
        scale_x: scene_attr_or_default(block, &["scaleX", "scale_x"], "1"),
        scale_y: scene_attr_or_default(block, &["scaleY", "scale_y"], "1"),
        skew_x: scene_attr_or_default(block, &["skewX", "skew_x"], "0"),
        skew_y: scene_attr_or_default(block, &["skewY", "skew_y"], "0"),
        transform_origin_x: scene_attr_or_default(
            block,
            &["transformOriginX", "transform_origin_x"],
            "0",
        ),
        transform_origin_y: scene_attr_or_default(
            block,
            &["transformOriginY", "transform_origin_y"],
            "0",
        ),
        x1,
        y1,
        x2,
        y2,
        width,
        color,
        opacity,
        line_cap: attr_value(block, "lineCap")
            .or_else(|| attr_value(block, "line_cap"))
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "round".to_string()),
        taper_start: attr_value(block, "taperStart")
            .or_else(|| attr_value(block, "taper_start"))
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "0".to_string()),
        taper_end: attr_value(block, "taperEnd")
            .or_else(|| attr_value(block, "taper_end"))
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "0".to_string()),
        stroke_style: stroke_attrs.style,
        stroke_roughness: stroke_attrs.roughness,
        stroke_copies: stroke_attrs.copies,
        stroke_texture: stroke_attrs.texture,
        stroke_bristles: stroke_attrs.bristles,
        stroke_pressure: stroke_attrs.pressure,
        stroke_pressure_min: stroke_attrs.pressure_min,
        stroke_pressure_curve: stroke_attrs.pressure_curve,
        blend: attr_value(block, "blend")
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "normal".to_string()),
    })
}

pub(crate) fn parse_polyline_node(
    block: &str,
    line: usize,
) -> Result<PolylineNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let points = strip_wrappers(&required_attr_value(block, "points", line)?).to_string();
    let stroke = attr_value(block, "stroke")
        .or_else(|| attr_value(block, "color"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "#ffffff".to_string());
    let stroke_width = attr_value(block, "strokeWidth")
        .or_else(|| attr_value(block, "stroke_width"))
        .or_else(|| attr_value(block, "width"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "4".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let trim_start = attr_value(block, "trimStart")
        .or_else(|| attr_value(block, "trim_start"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0.0".to_string());
    let trim_end = attr_value(block, "trimEnd")
        .or_else(|| attr_value(block, "trim_end"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let stroke_attrs = stroke_style_attrs(block);

    Ok(PolylineNode {
        id,
        x: scene_attr_or_default(block, &["x"], "0"),
        y: scene_attr_or_default(block, &["y"], "0"),
        rotation: scene_attr_or_default(block, &["rotation"], "0"),
        scale: scene_attr_or_default(block, &["scale"], "1"),
        scale_x: scene_attr_or_default(block, &["scaleX", "scale_x"], "1"),
        scale_y: scene_attr_or_default(block, &["scaleY", "scale_y"], "1"),
        skew_x: scene_attr_or_default(block, &["skewX", "skew_x"], "0"),
        skew_y: scene_attr_or_default(block, &["skewY", "skew_y"], "0"),
        transform_origin_x: scene_attr_or_default(
            block,
            &["transformOriginX", "transform_origin_x"],
            "0",
        ),
        transform_origin_y: scene_attr_or_default(
            block,
            &["transformOriginY", "transform_origin_y"],
            "0",
        ),
        points,
        stroke,
        stroke_width,
        opacity,
        trim_start,
        trim_end,
        line_cap: attr_value(block, "lineCap")
            .or_else(|| attr_value(block, "line_cap"))
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "round".to_string()),
        line_join: attr_value(block, "lineJoin")
            .or_else(|| attr_value(block, "line_join"))
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "round".to_string()),
        taper_start: attr_value(block, "taperStart")
            .or_else(|| attr_value(block, "taper_start"))
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "0".to_string()),
        taper_end: attr_value(block, "taperEnd")
            .or_else(|| attr_value(block, "taper_end"))
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "0".to_string()),
        stroke_style: stroke_attrs.style,
        stroke_roughness: stroke_attrs.roughness,
        stroke_copies: stroke_attrs.copies,
        stroke_texture: stroke_attrs.texture,
        stroke_bristles: stroke_attrs.bristles,
        stroke_pressure: stroke_attrs.pressure,
        stroke_pressure_min: stroke_attrs.pressure_min,
        stroke_pressure_curve: stroke_attrs.pressure_curve,
        blend: attr_value(block, "blend")
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "normal".to_string()),
    })
}

pub(crate) fn parse_path_node(
    block: &str,
    line: usize,
    brush_ctx: &BrushParseContext,
) -> Result<PathNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let (brush_id, brush) = brush_ctx.brush_for_path(block, line)?;
    let d = strip_wrappers(&required_attr_value(block, "d", line)?).to_string();
    let fill = attr_value(block, "fill")
        .map(|v| strip_wrappers(&v).to_string())
        .or_else(|| brush.and_then(|brush| brush.fill.clone()));
    let stroke = attr_value(block, "stroke")
        .or_else(|| attr_value(block, "color"))
        .map(|v| strip_wrappers(&v).to_string())
        .or_else(|| brush.and_then(|brush| brush.stroke.clone()))
        .unwrap_or_else(|| {
            if fill.is_some() {
                "none".to_string()
            } else {
                "#ffffff".to_string()
            }
        });
    let stroke_width = attr_or_brush(
        block,
        &["strokeWidth", "stroke_width", "width"],
        brush.and_then(|brush| brush.stroke_width.as_ref()),
        "4",
    );
    let opacity = attr_or_brush(
        block,
        &["opacity"],
        brush.and_then(|brush| brush.opacity.as_ref()),
        "1.0",
    );
    let trim_start = attr_value(block, "trimStart")
        .or_else(|| attr_value(block, "trim_start"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0.0".to_string());
    let trim_end = attr_value(block, "trimEnd")
        .or_else(|| attr_value(block, "trim_end"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let stroke_attrs = stroke_style_attrs_with_brush(block, brush);

    Ok(PathNode {
        id,
        brush: brush_id,
        x: scene_attr_or_default(block, &["x"], "0"),
        y: scene_attr_or_default(block, &["y"], "0"),
        rotation: scene_attr_or_default(block, &["rotation"], "0"),
        scale: scene_attr_or_default(block, &["scale"], "1"),
        scale_x: scene_attr_or_default(block, &["scaleX", "scale_x"], "1"),
        scale_y: scene_attr_or_default(block, &["scaleY", "scale_y"], "1"),
        skew_x: scene_attr_or_default(block, &["skewX", "skew_x"], "0"),
        skew_y: scene_attr_or_default(block, &["skewY", "skew_y"], "0"),
        transform_origin_x: scene_attr_or_default(
            block,
            &["transformOriginX", "transform_origin_x"],
            "0",
        ),
        transform_origin_y: scene_attr_or_default(
            block,
            &["transformOriginY", "transform_origin_y"],
            "0",
        ),
        d,
        stroke,
        fill,
        stroke_width,
        opacity,
        trim_start,
        trim_end,
        line_cap: attr_or_brush(
            block,
            &["lineCap", "line_cap"],
            brush.and_then(|brush| brush.line_cap.as_ref()),
            "round",
        ),
        line_join: attr_or_brush(
            block,
            &["lineJoin", "line_join"],
            brush.and_then(|brush| brush.line_join.as_ref()),
            "round",
        ),
        taper_start: attr_or_brush(
            block,
            &["taperStart", "taper_start"],
            brush.and_then(|brush| brush.taper_start.as_ref()),
            "0",
        ),
        taper_end: attr_or_brush(
            block,
            &["taperEnd", "taper_end"],
            brush.and_then(|brush| brush.taper_end.as_ref()),
            "0",
        ),
        stroke_style: stroke_attrs.style,
        stroke_roughness: stroke_attrs.roughness,
        stroke_copies: stroke_attrs.copies,
        stroke_texture: stroke_attrs.texture,
        stroke_bristles: stroke_attrs.bristles,
        stroke_pressure: stroke_attrs.pressure,
        stroke_pressure_min: stroke_attrs.pressure_min,
        stroke_pressure_curve: stroke_attrs.pressure_curve,
        blend: attr_or_brush(
            block,
            &["blend"],
            brush.and_then(|brush| brush.blend.as_ref()),
            "normal",
        ),
        texture: attr_value(block, "texture").map(|v| strip_wrappers(&v).to_string()),
        texture_opacity: scene_attr_or_default(block, &["textureOpacity", "texture_opacity"], "1"),
        texture_scale: scene_attr_or_default(block, &["textureScale", "texture_scale"], "1"),
        texture_mask: scene_attr_or_default(block, &["textureMask", "texture_mask"], "0"),
    })
}

pub(crate) fn parse_face_jaw_node(
    block: &str,
    _line: usize,
) -> Result<FaceJawNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let width = attr_value(block, "width")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "100".to_string());
    let height = attr_value(block, "height")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "100".to_string());
    let cheek_width = attr_value(block, "cheekWidth")
        .or_else(|| attr_value(block, "cheek_width"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| width.clone());
    let chin_width = attr_value(block, "chinWidth")
        .or_else(|| attr_value(block, "chin_width"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "24".to_string());
    let chin_sharpness = attr_value(block, "chinSharpness")
        .or_else(|| attr_value(block, "chin_sharpness"))
        .or_else(|| attr_value(block, "sharpness"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0.45".to_string());
    let jaw_ease = attr_value(block, "jawEase")
        .or_else(|| attr_value(block, "jaw_ease"))
        .or_else(|| attr_value(block, "ease"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0.55".to_string());
    let scale = attr_value(block, "scale")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1".to_string());
    let closed = attr_value(block, "closed")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "true".to_string());
    let fill = attr_value(block, "fill").map(|v| strip_wrappers(&v).to_string());
    let stroke = attr_value(block, "stroke")
        .or_else(|| attr_value(block, "color"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| {
            if fill.is_some() {
                "none".to_string()
            } else {
                "#ffffff".to_string()
            }
        });
    let stroke_width = attr_value(block, "strokeWidth")
        .or_else(|| attr_value(block, "stroke_width"))
        .or_else(|| attr_value(block, "widthStroke"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "4".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let trim_start = attr_value(block, "trimStart")
        .or_else(|| attr_value(block, "trim_start"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0.0".to_string());
    let trim_end = attr_value(block, "trimEnd")
        .or_else(|| attr_value(block, "trim_end"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let stroke_attrs = stroke_style_attrs(block);
    Ok(FaceJawNode {
        id,
        x,
        y,
        width,
        height,
        cheek_width,
        chin_width,
        chin_sharpness,
        jaw_ease,
        scale,
        closed,
        stroke,
        fill,
        stroke_width,
        opacity,
        trim_start,
        trim_end,
        line_cap: attr_value(block, "lineCap")
            .or_else(|| attr_value(block, "line_cap"))
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "round".to_string()),
        line_join: attr_value(block, "lineJoin")
            .or_else(|| attr_value(block, "line_join"))
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "round".to_string()),
        taper_start: attr_value(block, "taperStart")
            .or_else(|| attr_value(block, "taper_start"))
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "0".to_string()),
        taper_end: attr_value(block, "taperEnd")
            .or_else(|| attr_value(block, "taper_end"))
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "0".to_string()),
        stroke_style: stroke_attrs.style,
        stroke_roughness: stroke_attrs.roughness,
        stroke_copies: stroke_attrs.copies,
        stroke_texture: stroke_attrs.texture,
        stroke_bristles: stroke_attrs.bristles,
        stroke_pressure: stroke_attrs.pressure,
        stroke_pressure_min: stroke_attrs.pressure_min,
        stroke_pressure_curve: stroke_attrs.pressure_curve,
        blend: attr_value(block, "blend")
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "normal".to_string()),
    })
}

pub(crate) fn parse_shadow_node(block: &str, _line: usize) -> Result<ShadowNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "18".to_string());
    let blur = attr_value(block, "blur")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "36".to_string());
    let color = attr_value(block, "color")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "[0,0,0,0.18]".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    Ok(ShadowNode {
        id,
        x,
        y,
        blur,
        color,
        opacity,
    })
}

fn parse_group_node(
    block: &str,
    _line: usize,
    children: Vec<SceneNode>,
) -> Result<GroupNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let brush = attr_value(block, "brush").map(|v| strip_wrappers(&v).to_string());
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let rotation = attr_value(block, "rotation")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let scale = attr_value(block, "scale")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let scale_x = attr_value(block, "scaleX")
        .or_else(|| attr_value(block, "scale_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let scale_y = attr_value(block, "scaleY")
        .or_else(|| attr_value(block, "scale_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let skew_x = attr_value(block, "skewX")
        .or_else(|| attr_value(block, "skew_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let skew_y = attr_value(block, "skewY")
        .or_else(|| attr_value(block, "skew_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let transform_origin_x = attr_value(block, "transformOriginX")
        .or_else(|| attr_value(block, "transform_origin_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let transform_origin_y = attr_value(block, "transformOriginY")
        .or_else(|| attr_value(block, "transform_origin_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let deform_grid = attr_value(block, "deformGrid")
        .or_else(|| attr_value(block, "deform_grid"))
        .map(|v| strip_wrappers(&v).to_string());
    let grid_from = attr_value(block, "gridFrom")
        .or_else(|| attr_value(block, "grid_from"))
        .map(|v| strip_wrappers(&v).to_string());
    let grid_to = attr_value(block, "gridTo")
        .or_else(|| attr_value(block, "grid_to"))
        .map(|v| strip_wrappers(&v).to_string());
    let deform_amount = attr_value(block, "deformAmount")
        .or_else(|| attr_value(block, "deform_amount"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let mask = attr_value(block, "mask")
        .or_else(|| attr_value(block, "maskId"))
        .or_else(|| attr_value(block, "mask_id"))
        .map(|v| strip_wrappers(&v).to_string())
        .filter(|v| !v.trim().is_empty());
    let mask_from = attr_value(block, "maskFrom")
        .or_else(|| attr_value(block, "mask_from"))
        .or_else(|| attr_value(block, "matteFrom"))
        .or_else(|| attr_value(block, "matte_from"))
        .map(|v| strip_wrappers(&v).to_string())
        .filter(|v| !v.trim().is_empty());
    let mask_mode = attr_value(block, "maskMode")
        .or_else(|| attr_value(block, "mask_mode"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "alpha".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    Ok(GroupNode {
        id,
        brush,
        x,
        y,
        rotation,
        scale,
        scale_x,
        scale_y,
        skew_x,
        skew_y,
        transform_origin_x,
        transform_origin_y,
        deform_grid,
        grid_from,
        grid_to,
        deform_amount,
        mask,
        mask_from,
        mask_mode,
        opacity,
        children,
    })
}

fn parse_puppet_node(
    block: &str,
    _line: usize,
    children: Vec<SceneNode>,
) -> Result<PuppetNode, GraphParseError> {
    Ok(PuppetNode {
        id: attr_value(block, "id").map(|v| strip_wrappers(&v).to_string()),
        target: attr_value(block, "target")
            .or_else(|| attr_value(block, "targetId"))
            .or_else(|| attr_value(block, "target_id"))
            .map(|v| strip_wrappers(&v).to_string())
            .filter(|v| !v.trim().is_empty()),
        mesh: scene_attr_or_default(block, &["mesh"], "auto"),
        density: scene_attr_or_default(block, &["density"], "medium"),
        x: scene_attr_or_default(block, &["x"], "0"),
        y: scene_attr_or_default(block, &["y"], "0"),
        rotation: scene_attr_or_default(block, &["rotation"], "0"),
        scale: scene_attr_or_default(block, &["scale"], "1"),
        scale_x: scene_attr_or_default(block, &["scaleX", "scale_x"], "1"),
        scale_y: scene_attr_or_default(block, &["scaleY", "scale_y"], "1"),
        skew_x: scene_attr_or_default(block, &["skewX", "skew_x"], "0"),
        skew_y: scene_attr_or_default(block, &["skewY", "skew_y"], "0"),
        transform_origin_x: scene_attr_or_default(
            block,
            &["transformOriginX", "transform_origin_x"],
            "0",
        ),
        transform_origin_y: scene_attr_or_default(
            block,
            &["transformOriginY", "transform_origin_y"],
            "0",
        ),
        width: scene_attr_or_default(block, &["width", "w"], "512"),
        height: scene_attr_or_default(block, &["height", "h"], "512"),
        amount: scene_attr_or_default(block, &["amount", "deformAmount", "deform_amount"], "1"),
        opacity: scene_attr_or_default(block, &["opacity"], "1"),
        children,
    })
}

pub(crate) fn parse_pin_node(block: &str, _line: usize) -> Result<PinNode, GraphParseError> {
    Ok(PinNode {
        id: attr_value(block, "id").map(|v| strip_wrappers(&v).to_string()),
        vertex: attr_value(block, "vertex")
            .or_else(|| attr_value(block, "vertexId"))
            .or_else(|| attr_value(block, "vertex_id"))
            .map(|v| strip_wrappers(&v).to_string())
            .filter(|v| !v.trim().is_empty()),
        x: attr_value(block, "x").map(|v| strip_wrappers(&v).to_string()),
        y: attr_value(block, "y").map(|v| strip_wrappers(&v).to_string()),
        target_x: attr_value(block, "targetX")
            .or_else(|| attr_value(block, "target_x"))
            .map(|v| strip_wrappers(&v).to_string()),
        target_y: attr_value(block, "targetY")
            .or_else(|| attr_value(block, "target_y"))
            .map(|v| strip_wrappers(&v).to_string()),
        radius: scene_attr_or_default(block, &["radius", "r"], "120"),
        strength: scene_attr_or_default(block, &["strength", "weight"], "1"),
        falloff: scene_attr_or_default(block, &["falloff"], "smooth"),
        fixed: scene_attr_or_default(block, &["fixed", "lock", "locked"], "false"),
    })
}

fn parse_mesh_topology_node(
    block: &str,
    _line: usize,
    children: Vec<SceneNode>,
) -> Result<MeshTopologyNode, GraphParseError> {
    Ok(MeshTopologyNode {
        id: attr_value(block, "id").map(|v| strip_wrappers(&v).to_string()),
        mode: attr_value(block, "mode")
            .or_else(|| attr_value(block, "kind"))
            .map(|v| strip_wrappers(&v).to_string()),
        children,
    })
}

fn parse_vertex_node(block: &str, line: usize) -> Result<VertexNode, GraphParseError> {
    Ok(VertexNode {
        id: required_attr_value(block, "id", line)?,
        x: required_attr_value(block, "x", line)?,
        y: required_attr_value(block, "y", line)?,
    })
}

fn parse_triangle_node(block: &str, line: usize) -> Result<TriangleNode, GraphParseError> {
    Ok(TriangleNode {
        id: attr_value(block, "id").map(|v| strip_wrappers(&v).to_string()),
        a: required_attr_value(block, "a", line)?,
        b: required_attr_value(block, "b", line)?,
        c: required_attr_value(block, "c", line)?,
    })
}

fn parse_edge_node(block: &str, line: usize) -> Result<EdgeNode, GraphParseError> {
    Ok(EdgeNode {
        id: attr_value(block, "id").map(|v| strip_wrappers(&v).to_string()),
        a: required_attr_value(block, "a", line)?,
        b: required_attr_value(block, "b", line)?,
        boundary: scene_attr_or_default(block, &["boundary"], "false"),
    })
}

fn parse_region_node(block: &str, line: usize) -> Result<RegionNode, GraphParseError> {
    Ok(RegionNode {
        id: required_attr_value(block, "id", line)?,
        vertices: scene_attr_or_default(block, &["vertices", "verts"], ""),
        triangles: scene_attr_or_default(block, &["triangles"], ""),
        weight: scene_attr_or_default(block, &["weight"], "1"),
    })
}

fn parse_part_node(
    block: &str,
    _line: usize,
    children: Vec<SceneNode>,
) -> Result<PartNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let label = attr_value(block, "label").map(|v| strip_wrappers(&v).to_string());
    let role = attr_value(block, "role").map(|v| strip_wrappers(&v).to_string());
    let attach_to = attr_value(block, "attachTo")
        .or_else(|| attr_value(block, "attach_to"))
        .or_else(|| attr_value(block, "bone"))
        .map(|v| strip_wrappers(&v).to_string())
        .filter(|v| !v.is_empty());
    let brush = attr_value(block, "brush").map(|v| strip_wrappers(&v).to_string());
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let rotation = attr_value(block, "rotation")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let scale = attr_value(block, "scale")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let anchor_x = attr_value(block, "anchorX")
        .or_else(|| attr_value(block, "anchor_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let anchor_y = attr_value(block, "anchorY")
        .or_else(|| attr_value(block, "anchor_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    Ok(PartNode {
        id,
        label,
        role,
        attach_to,
        brush,
        x,
        y,
        rotation,
        scale,
        opacity,
        anchor_x,
        anchor_y,
        children,
    })
}

fn parse_repeat_node(
    block: &str,
    _line: usize,
    children: Vec<SceneNode>,
) -> Result<RepeatNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let count = attr_value(block, "count")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1".to_string());
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let rotation = attr_value(block, "rotation")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let scale = attr_value(block, "scale")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let x_step = attr_value(block, "xStep")
        .or_else(|| attr_value(block, "x_step"))
        .or_else(|| attr_value(block, "dx"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let y_step = attr_value(block, "yStep")
        .or_else(|| attr_value(block, "y_step"))
        .or_else(|| attr_value(block, "dy"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let rotation_step = attr_value(block, "rotationStep")
        .or_else(|| attr_value(block, "rotation_step"))
        .or_else(|| attr_value(block, "dRotation"))
        .or_else(|| attr_value(block, "d_rotation"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let scale_step = attr_value(block, "scaleStep")
        .or_else(|| attr_value(block, "scale_step"))
        .or_else(|| attr_value(block, "dScale"))
        .or_else(|| attr_value(block, "d_scale"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let opacity_step = attr_value(block, "opacityStep")
        .or_else(|| attr_value(block, "opacity_step"))
        .or_else(|| attr_value(block, "dOpacity"))
        .or_else(|| attr_value(block, "d_opacity"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());

    Ok(RepeatNode {
        id,
        count,
        x,
        y,
        rotation,
        scale,
        opacity,
        x_step,
        y_step,
        rotation_step,
        scale_step,
        opacity_step,
        children,
    })
}

fn parse_mask_node(
    block: &str,
    _line: usize,
    children: Vec<SceneNode>,
) -> Result<MaskNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let follow = attr_value(block, "follow")
        .or_else(|| attr_value(block, "target"))
        .or_else(|| attr_value(block, "followTarget"))
        .or_else(|| attr_value(block, "follow_target"))
        .map(|v| strip_wrappers(&v).to_string());
    let shape = attr_value(block, "shape")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "rect".to_string());
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let width = attr_value(block, "width")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1920".to_string());
    let height = attr_value(block, "height")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1080".to_string());
    let radius = attr_value(block, "radius")
        .or_else(|| attr_value(block, "r"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let d = attr_value(block, "d").map(|v| strip_wrappers(&v).to_string());
    let feather = attr_value(block, "feather")
        .or_else(|| attr_value(block, "softness"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    Ok(MaskNode {
        id,
        follow,
        shape,
        x,
        y,
        width,
        height,
        radius,
        d,
        feather,
        opacity,
        children,
    })
}

fn parse_scene_layer_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
    tag_name: &str,
    is_3d: bool,
) -> Result<(SceneLayerNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, tag_name)?;
    let mut child_ctx = brush_ctx.clone();
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((
        parse_scene_layer_node(&open_tag, start + 1, children, false, is_3d)?,
        close_ix,
    ))
}

fn parse_scene_layer_node(
    block: &str,
    line: usize,
    children: Vec<SceneNode>,
    require_source: bool,
    is_3d: bool,
) -> Result<SceneLayerNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let source = attr_value(block, "source")
        .or_else(|| attr_value(block, "src"))
        .or_else(|| attr_value(block, "from"))
        .map(|v| strip_wrappers(&v).to_string())
        .filter(|v| !v.trim().is_empty());
    if require_source && source.is_none() {
        return Err(GraphParseError {
            line,
            message: "Scene <Layer> requires source=\"precompose_id\".".to_string(),
        });
    }
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let z = attr_value(block, "z")
        .or_else(|| attr_value(block, "translateZ"))
        .or_else(|| attr_value(block, "translate_z"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let rotation_x = attr_value(block, "rotationX")
        .or_else(|| attr_value(block, "rotation_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let rotation_y = attr_value(block, "rotationY")
        .or_else(|| attr_value(block, "rotation_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let rotation = attr_value(block, "rotation")
        .or_else(|| attr_value(block, "rotationZ"))
        .or_else(|| attr_value(block, "rotation_z"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let perspective = attr_value(block, "perspective")
        .or_else(|| attr_value(block, "cameraDistance"))
        .or_else(|| attr_value(block, "camera_distance"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "900".to_string());
    let scale = attr_value(block, "scale")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let scale_x = attr_value(block, "scaleX")
        .or_else(|| attr_value(block, "scale_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let scale_y = attr_value(block, "scaleY")
        .or_else(|| attr_value(block, "scale_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let skew_x = attr_value(block, "skewX")
        .or_else(|| attr_value(block, "skew_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let skew_y = attr_value(block, "skewY")
        .or_else(|| attr_value(block, "skew_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let transform_origin_x = attr_value(block, "transformOriginX")
        .or_else(|| attr_value(block, "transform_origin_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let transform_origin_y = attr_value(block, "transformOriginY")
        .or_else(|| attr_value(block, "transform_origin_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let z_depth = attr_value(block, "zDepth")
        .or_else(|| attr_value(block, "z_depth"))
        .map(|v| strip_wrappers(&v).to_string())
        .filter(|v| !v.trim().is_empty());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let blend = attr_value(block, "blend")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "normal".to_string());
    let effect = attr_value(block, "effect")
        .or_else(|| attr_value(block, "filter"))
        .map(|v| strip_wrappers(&v).to_string())
        .filter(|v| !v.trim().is_empty());
    let source_time = attr_value(block, "sourceTime")
        .or_else(|| attr_value(block, "source_time"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "local".to_string());
    let time_offset_ms = attr_value(block, "timeOffset")
        .or_else(|| attr_value(block, "time_offset"))
        .or_else(|| attr_value(block, "sourceTimeOffset"))
        .or_else(|| attr_value(block, "source_time_offset"))
        .as_deref()
        .map(|v| parse_signed_time_ms(v, line, "Layer.timeOffset"))
        .transpose()?
        .unwrap_or(0);
    let playback_rate = attr_value(block, "playbackRate")
        .or_else(|| attr_value(block, "playback_rate"))
        .or_else(|| attr_value(block, "speed"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1".to_string());
    let out = attr_value(block, "out")
        .or_else(|| attr_value(block, "sourceOut"))
        .or_else(|| attr_value(block, "source_out"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "hold".to_string());
    let mask = attr_value(block, "mask")
        .or_else(|| attr_value(block, "maskId"))
        .or_else(|| attr_value(block, "mask_id"))
        .map(|v| strip_wrappers(&v).to_string())
        .filter(|v| !v.trim().is_empty());
    let mask_from = attr_value(block, "maskFrom")
        .or_else(|| attr_value(block, "mask_from"))
        .map(|v| strip_wrappers(&v).to_string())
        .filter(|v| !v.trim().is_empty());
    let mask_mode = attr_value(block, "maskMode")
        .or_else(|| attr_value(block, "mask_mode"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "alpha".to_string());
    let matte = attr_value(block, "matte")
        .or_else(|| attr_value(block, "trackMatte"))
        .or_else(|| attr_value(block, "track_matte"))
        .map(|v| strip_wrappers(&v).to_string())
        .filter(|v| !v.trim().is_empty());
    let matte_from = attr_value(block, "matteFrom")
        .or_else(|| attr_value(block, "matte_from"))
        .or_else(|| attr_value(block, "maskFrom"))
        .or_else(|| attr_value(block, "mask_from"))
        .map(|v| strip_wrappers(&v).to_string())
        .filter(|v| !v.trim().is_empty());
    let matte_mode = attr_value(block, "matteMode")
        .or_else(|| attr_value(block, "matte_mode"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "alpha".to_string());
    let invert_matte = attr_value(block, "invertMatte")
        .or_else(|| attr_value(block, "invert_matte"))
        .or_else(|| attr_value(block, "matteInvert"))
        .or_else(|| attr_value(block, "matte_invert"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "false".to_string());

    Ok(SceneLayerNode {
        id,
        source,
        is_3d,
        x,
        y,
        z,
        rotation_x,
        rotation_y,
        rotation,
        perspective,
        scale,
        scale_x,
        scale_y,
        skew_x,
        skew_y,
        transform_origin_x,
        transform_origin_y,
        z_depth,
        opacity,
        blend,
        effect,
        source_time,
        time_offset_ms,
        playback_rate,
        out,
        mask,
        mask_from,
        mask_mode,
        matte,
        matte_from,
        matte_mode,
        invert_matte,
        children,
    })
}

pub(crate) fn parse_camera_node(
    block: &str,
    line: usize,
    children: Vec<SceneNode>,
) -> Result<CameraNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    if attr_value(block, "mode").is_some() {
        return Err(GraphParseError {
            line,
            message: "<Scene> Camera is always Camera2D; remove mode=\"...\". Use <World><Camera> for 3D/world cameras.".to_string(),
        });
    }
    let x = attr_value(block, "x")
        .or_else(|| attr_value(block, "positionX"))
        .or_else(|| attr_value(block, "centerX"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "center".to_string());
    let y = attr_value(block, "y")
        .or_else(|| attr_value(block, "positionY"))
        .or_else(|| attr_value(block, "centerY"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "center".to_string());
    let target_x = attr_value(block, "targetX")
        .or_else(|| attr_value(block, "target_x"))
        .or_else(|| attr_value(block, "pointOfInterestX"))
        .or_else(|| attr_value(block, "focusX"))
        .map(|v| strip_wrappers(&v).to_string());
    let target_y = attr_value(block, "targetY")
        .or_else(|| attr_value(block, "target_y"))
        .or_else(|| attr_value(block, "pointOfInterestY"))
        .or_else(|| attr_value(block, "focusY"))
        .map(|v| strip_wrappers(&v).to_string());
    let anchor_x = attr_value(block, "anchorX")
        .or_else(|| attr_value(block, "anchor_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0.5".to_string());
    let anchor_y = attr_value(block, "anchorY")
        .or_else(|| attr_value(block, "anchor_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0.5".to_string());
    let offset_x = attr_value(block, "offsetX")
        .or_else(|| attr_value(block, "offset_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let offset_y = attr_value(block, "offsetY")
        .or_else(|| attr_value(block, "offset_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let shake_x = attr_value(block, "shakeX")
        .or_else(|| attr_value(block, "shake_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let shake_y = attr_value(block, "shakeY")
        .or_else(|| attr_value(block, "shake_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let zoom = attr_value(block, "zoom")
        .or_else(|| attr_value(block, "scale"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let rotation = attr_value(block, "rotation")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let follow = attr_value(block, "follow")
        .or_else(|| attr_value(block, "target"))
        .or_else(|| attr_value(block, "followTarget"))
        .map(|v| strip_wrappers(&v).to_string());
    let dead_zone = attr_value(block, "deadZone")
        .or_else(|| attr_value(block, "dead_zone"))
        .or_else(|| attr_value(block, "dragMargin"))
        .map(|v| strip_wrappers(&v).to_string());
    let viewport = attr_value(block, "viewport")
        .or_else(|| attr_value(block, "crop"))
        .map(|v| strip_wrappers(&v).to_string());
    let world_bounds = attr_value(block, "worldBounds")
        .or_else(|| attr_value(block, "world_bounds"))
        .or_else(|| attr_value(block, "limit"))
        .or_else(|| attr_value(block, "limits"))
        .map(|v| strip_wrappers(&v).to_string());

    Ok(CameraNode {
        id,
        x,
        y,
        target_x,
        target_y,
        anchor_x,
        anchor_y,
        offset_x,
        offset_y,
        shake_x,
        shake_y,
        zoom,
        rotation,
        opacity,
        follow,
        dead_zone,
        viewport,
        world_bounds,
        children,
    })
}

fn parse_character_node(
    block: &str,
    _line: usize,
    children: Vec<SceneNode>,
) -> Result<CharacterNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let src = attr_value(block, "src")
        .or_else(|| attr_value(block, "image"))
        .or_else(|| attr_value(block, "path"))
        .map(|v| strip_wrappers(&v).to_string());
    let rig = attr_value(block, "rig")
        .or_else(|| attr_value(block, "skeleton"))
        .map(|v| strip_wrappers(&v).to_string());
    let model_profile = attr_value(block, "modelProfile")
        .or_else(|| attr_value(block, "model_profile"))
        .or_else(|| attr_value(block, "profile"))
        .map(|v| strip_wrappers(&v).to_string());
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let rotation = attr_value(block, "rotation")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let scale = attr_value(block, "scale")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let scale_x = attr_value(block, "scaleX")
        .or_else(|| attr_value(block, "scale_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let scale_y = attr_value(block, "scaleY")
        .or_else(|| attr_value(block, "scale_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let skew_x = attr_value(block, "skewX")
        .or_else(|| attr_value(block, "skew_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let skew_y = attr_value(block, "skewY")
        .or_else(|| attr_value(block, "skew_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let transform_origin_x = attr_value(block, "transformOriginX")
        .or_else(|| attr_value(block, "transform_origin_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let transform_origin_y = attr_value(block, "transformOriginY")
        .or_else(|| attr_value(block, "transform_origin_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    Ok(CharacterNode {
        id,
        src,
        rig,
        model_profile,
        x,
        y,
        rotation,
        scale,
        scale_x,
        scale_y,
        skew_x,
        skew_y,
        transform_origin_x,
        transform_origin_y,
        opacity,
        children,
    })
}
