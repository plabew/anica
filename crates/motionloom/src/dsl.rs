// =========================================
// =========================================
// crates/motionloom/src/dsl.rs

pub use crate::error::GraphParseError;
use crate::scene::{
    BrushDef, CameraNode, CharacterNode, CircleNode, DefsNode, FaceJawNode, GradientDef,
    GradientStop, GroupNode, LineNode, LinearGradientDef, MaskNode, PartNode, PathNode,
    PolylineNode, RadialGradientDef, RectNode, RepeatNode, SceneNode, SceneRootNode, ShadowNode,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorSpace {
    Srgb,
    LinearSrgb,
    DisplayP3,
    Rec709,
    Rec2020,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AlphaMode {
    Straight,
    Premul,
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextureFormat {
    // Keep legacy alias for current scripts.
    Rgba8,
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Bgra8Unorm,
    Bgra8UnormSrgb,
    Rgba16f,
    Rgba32f,
    R16f,
    R32f,
    Depth24plus,
    Depth32f,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Quality<T> {
    Uniform(T),
    Split { preview: T, r#final: T },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GraphScope {
    Layer,
    Clip,
    Scene,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GraphApplyScope {
    Clip,
    Graph,
}

impl Default for GraphApplyScope {
    fn default() -> Self {
        Self::Clip
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InputType {
    Video,
    Image,
    Mask,
    Depth,
    Normal,
    Motion,
    Audio,
}

impl Default for InputType {
    fn default() -> Self {
        Self::Video
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TexUsage {
    Sampled,
    Storage,
    ColorAttachment,
    DepthStencilAttachment,
    CopySrc,
    CopyDst,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BufferUsage {
    Uniform,
    Storage,
    Vertex,
    Index,
    Indirect,
    CopySrc,
    CopyDst,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BufferElemType {
    F32,
    U32,
    I32,
    Vec2f,
    Vec4f,
    Mat4f,
    Struct,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PassKind {
    Compute,
    Render,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PassRole {
    Effect,
    Transition,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PassCache {
    None,
    Frame,
    Static,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LoadOp {
    Load,
    Clear([f32; 4]),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StoreOp {
    Store,
    Discard,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlendMode {
    Replace,
    Add,
    Screen,
    Multiply,
    Over,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PassTransitionMode {
    Auto,
    Off,
    Force,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PassTransitionFallback {
    Under,
    Prev,
    Next,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PassTransitionEasing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PassTransitionClips {
    Overlap,
    NonOverlap,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputTarget {
    Screen,
    File,
    Host,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PresentTarget {
    Screen,
    Host,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SampleFilter {
    Nearest,
    Linear,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SampleAddress {
    Clamp,
    Repeat,
    Mirror,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleConfig {
    pub filter: Option<SampleFilter>,
    pub address: Option<SampleAddress>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ResourceRef {
    Id {
        id: String,
    },
    Tex {
        tex: String,
        #[serde(default)]
        sample: Option<SampleConfig>,
    },
    Buffer {
        buf: String,
    },
}

impl ResourceRef {
    pub fn resource_id(&self) -> &str {
        match self {
            ResourceRef::Id { id } => id,
            ResourceRef::Tex { tex, .. } => tex,
            ResourceRef::Buffer { buf } => buf,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphScript {
    pub id: Option<String>,
    pub version: Option<String>,
    pub scope: GraphScope,
    pub fps: f32,
    #[serde(default)]
    pub apply: GraphApplyScope,
    pub duration_ms: u64,
    #[serde(default)]
    pub duration_explicit: bool,
    pub size: (u32, u32),
    #[serde(default)]
    pub render_size: Option<(u32, u32)>,
    pub inputs: Vec<InputNode>,
    pub textures: Vec<TexNode>,
    pub buffers: Vec<BufferNode>,
    #[serde(default)]
    pub solids: Vec<SolidNode>,
    #[serde(default)]
    pub texts: Vec<TextNode>,
    #[serde(default)]
    pub images: Vec<ImageNode>,
    #[serde(default)]
    pub svgs: Vec<SvgNode>,
    #[serde(default)]
    pub scenes: Vec<SceneRootNode>,
    #[serde(default)]
    pub scene_nodes: Vec<SceneNode>,
    pub passes: Vec<PassNode>,
    pub outputs: Vec<OutputNode>,
    pub present: PresentNode,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputNode {
    pub id: String,
    #[serde(default)]
    pub r#type: InputType,
    pub from: Option<String>,
    pub fmt: Option<TextureFormat>,
    pub size: Option<(u32, u32)>,
    pub color_space: Option<ColorSpace>,
    pub alpha: Option<AlphaMode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TexNode {
    pub id: String,
    pub fmt: TextureFormat,
    pub from: Option<String>,
    pub size: Option<(u32, u32)>,
    pub usage: Vec<TexUsage>,
    pub transient: Option<bool>,
    pub pingpong: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BufferNode {
    pub id: String,
    pub elem_type: BufferElemType,
    pub length: Option<u32>,
    pub stride: Option<u32>,
    pub usage: Vec<BufferUsage>,
    pub transient: Option<bool>,
    pub pingpong: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SolidNode {
    pub id: Option<String>,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextNode {
    pub id: Option<String>,
    pub value: String,
    pub x: String,
    pub y: String,
    pub width: Option<String>,
    pub font_size: String,
    pub line_height: Option<String>,
    pub color: String,
    pub opacity: String,
    pub visible_chars: Option<String>,
    pub max_lines: Option<String>,
    pub font_family: Option<String>,
    pub font_path: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputNode {
    pub id: String,
    pub from: Option<String>,
    pub to: OutputTarget,
    pub fmt: Option<TextureFormat>,
    pub size: Option<(u32, u32)>,
    pub color_space: Option<ColorSpace>,
    pub alpha: Option<AlphaMode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PassNode {
    pub id: String,
    pub kind: PassKind,
    pub role: Option<PassRole>,
    pub kernel: Option<String>,
    pub mode: Option<String>,
    pub effect: String,
    pub transition: Option<PassTransitionMode>,
    pub transition_fallback: Option<PassTransitionFallback>,
    pub transition_easing: Option<PassTransitionEasing>,
    pub transition_clips: Option<PassTransitionClips>,
    #[serde(rename = "in")]
    pub inputs: Vec<ResourceRef>,
    #[serde(rename = "out")]
    pub outputs: Vec<ResourceRef>,
    pub params: Vec<PassParam>,
    pub iterate: Option<Quality<u32>>,
    pub pingpong: Option<String>,
    pub cache: Option<PassCache>,
    pub blend: Option<BlendMode>,
    pub load_op: Option<LoadOp>,
    pub store_op: Option<StoreOp>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PassParam {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentNode {
    pub from: String,
    pub to: PresentTarget,
    pub vsync: Option<bool>,
}

impl GraphScript {
    pub fn summary(&self) -> String {
        format!(
            "Graph parsed: scope={:?}, fps={:.2}, apply={:?}, duration={}ms, size={}x{}, input={}, tex={}, buffer={}, scene={}, scene_node={}, pass={}, output={}, present={}",
            self.scope,
            self.fps,
            self.apply,
            self.duration_ms,
            self.size.0,
            self.size.1,
            self.inputs.len(),
            self.textures.len(),
            self.buffers.len(),
            self.scenes.len(),
            self.scene_nodes.len(),
            self.passes.len(),
            self.outputs.len(),
            self.present.from
        )
    }

    pub fn resource_size(&self, id: &str) -> Option<(u32, u32)> {
        if self.scope == GraphScope::Scene && id == "scene" {
            return Some(self.size);
        }
        if let Some(scene_id) = id.strip_prefix("scene:") {
            return self
                .scenes
                .iter()
                .find(|scene| scene.id == scene_id)
                .map(|scene| scene.size.unwrap_or(self.size));
        }
        if self.scope == GraphScope::Scene {
            if let Some(scene) = self.scenes.iter().find(|scene| scene.id == id) {
                return Some(scene.size.unwrap_or(self.size));
            }
        }
        self.outputs
            .iter()
            .find(|o| o.id == id)
            .and_then(|o| o.size)
            .or_else(|| {
                self.textures
                    .iter()
                    .find(|t| t.id == id)
                    .and_then(|t| t.size)
            })
            .or_else(|| self.inputs.iter().find(|i| i.id == id).and_then(|i| i.size))
    }

    pub fn has_scene_nodes(&self) -> bool {
        !self.solids.is_empty()
            || !self.texts.is_empty()
            || !self.images.is_empty()
            || !self.svgs.is_empty()
            || !self.scenes.is_empty()
            || !self.scene_nodes.is_empty()
    }
}

pub fn is_graph_script(input: &str) -> bool {
    input.contains("<Graph")
}

#[derive(Debug, Clone, Default)]
struct BrushParseContext {
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

    fn brush_for_path<'a>(&'a self, block: &str) -> (Option<String>, Option<&'a BrushDef>) {
        let brush_id = attr_value(block, "brush")
            .map(|v| strip_wrappers(&v).to_string())
            .or_else(|| self.inherited_brush.clone());
        let brush = brush_id.as_ref().and_then(|id| self.brushes.get(id));
        (brush_id, brush)
    }
}

pub fn parse_graph_script(input: &str) -> Result<GraphScript, GraphParseError> {
    const DEFAULT_GRAPH_DURATION_MS: u64 = 2_000;
    let normalized = input.replace('＝', "=");
    let lines: Vec<&str> = normalized.lines().collect();
    let Some(graph_start_ix) = lines
        .iter()
        .position(|line| line.trim_start().starts_with("<Graph"))
    else {
        return Err(GraphParseError {
            line: 0,
            message: "Missing <Graph ...> root tag.".to_string(),
        });
    };

    let (graph_open, graph_open_end_ix) = collect_tag_block(&lines, graph_start_ix, '>', false)?;
    let id = attr_value(&graph_open, "id").map(|v| strip_wrappers(&v).to_string());
    let version = attr_value(&graph_open, "version").map(|v| strip_wrappers(&v).to_string());
    let scope = parse_graph_scope(
        &required_attr_value(&graph_open, "scope", graph_start_ix + 1)?,
        graph_start_ix + 1,
        "scope",
    )?;
    let fps = parse_fps(&graph_open, graph_start_ix + 1)?;
    let apply = attr_value(&graph_open, "apply")
        .as_deref()
        .map(|v| parse_graph_apply_scope(v, graph_start_ix + 1, "apply"))
        .transpose()?
        .unwrap_or(GraphApplyScope::Clip);
    let duration_explicit = attr_value(&graph_open, "duration").is_some();
    let duration_ms =
        parse_duration_ms(&graph_open, graph_start_ix + 1, DEFAULT_GRAPH_DURATION_MS)?;
    let size = parse_size(
        &required_attr_value(&graph_open, "size", graph_start_ix + 1)?,
        graph_start_ix + 1,
        "size",
    )?;
    let render_size = attr_value(&graph_open, "renderSize")
        .as_deref()
        .map(|value| parse_size(value, graph_start_ix + 1, "renderSize"))
        .transpose()?;
    if let Some((0, _)) | Some((_, 0)) = render_size {
        return Err(GraphParseError {
            line: graph_start_ix + 1,
            message: "renderSize width and height must be greater than zero.".to_string(),
        });
    }

    let Some(graph_close_ix) = lines
        .iter()
        .enumerate()
        .skip(graph_open_end_ix + 1)
        .find(|(_, line)| line.trim_start().starts_with("</Graph>"))
        .map(|(ix, _)| ix)
    else {
        return Err(GraphParseError {
            line: graph_start_ix + 1,
            message: "Missing </Graph> closing tag.".to_string(),
        });
    };

    let mut inputs = Vec::<InputNode>::new();
    let mut textures = Vec::<TexNode>::new();
    let mut buffers = Vec::<BufferNode>::new();
    let mut solids = Vec::<SolidNode>::new();
    let mut texts = Vec::<TextNode>::new();
    let mut images = Vec::<ImageNode>::new();
    let mut svgs = Vec::<SvgNode>::new();
    let mut scenes = Vec::<SceneRootNode>::new();
    let mut scene_nodes = Vec::<SceneNode>::new();
    let mut outputs = Vec::<OutputNode>::new();
    let mut passes = Vec::<PassNode>::new();
    let mut present: Option<PresentNode> = None;
    let mut brush_ctx = BrushParseContext::default();
    let mut i = graph_open_end_ix + 1;

    while i < graph_close_ix {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('{') {
            i += 1;
            continue;
        }

        if line.starts_with("<Input") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            inputs.push(parse_input_node(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Defs") {
            let (defs, end_ix) = parse_defs_block(&lines, i)?;
            brush_ctx.define_brushes(&defs.brushes);
            scene_nodes.push(SceneNode::Defs(defs));
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Scene") {
            let (scene, end_ix) = parse_scene_root_block(&lines, i, &brush_ctx)?;
            scenes.push(scene);
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Solid") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            let node = parse_solid_node(&tag, i + 1)?;
            scene_nodes.push(SceneNode::Solid(node.clone()));
            solids.push(node);
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Text") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            let node = parse_text_node(&tag, i + 1)?;
            scene_nodes.push(SceneNode::Text(node.clone()));
            texts.push(node);
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Image") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            let node = parse_image_node(&tag, i + 1)?;
            scene_nodes.push(SceneNode::Image(node.clone()));
            images.push(node);
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Svg") || starts_open_tag(line, "SVG") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            let node = parse_svg_node(&tag, i + 1)?;
            scene_nodes.push(SceneNode::Svg(node.clone()));
            svgs.push(node);
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Rect") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            scene_nodes.push(SceneNode::Rect(parse_rect_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Circle") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            scene_nodes.push(SceneNode::Circle(parse_circle_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Line") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            scene_nodes.push(SceneNode::Line(parse_line_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Polyline") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            scene_nodes.push(SceneNode::Polyline(parse_polyline_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Path") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            scene_nodes.push(SceneNode::Path(parse_path_node(&tag, i + 1, &brush_ctx)?));
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "FaceJaw") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            scene_nodes.push(SceneNode::FaceJaw(parse_face_jaw_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Shadow") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            scene_nodes.push(SceneNode::Shadow(parse_shadow_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Group") {
            let (group, end_ix) = parse_group_block(&lines, i, &brush_ctx)?;
            scene_nodes.push(SceneNode::Group(group));
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Part") {
            let (part, end_ix) = parse_part_block(&lines, i, &brush_ctx)?;
            scene_nodes.push(SceneNode::Part(part));
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Repeat") {
            let (repeat, end_ix) = parse_repeat_block(&lines, i, &brush_ctx)?;
            scene_nodes.push(SceneNode::Repeat(repeat));
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Mask") {
            let (mask, end_ix) = parse_mask_block(&lines, i, &brush_ctx)?;
            scene_nodes.push(SceneNode::Mask(mask));
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Character") {
            let (character, end_ix) = parse_character_block(&lines, i, &brush_ctx)?;
            scene_nodes.push(SceneNode::Character(character));
            i = end_ix + 1;
            continue;
        }

        if starts_open_tag(line, "Camera") {
            let (camera, end_ix) = parse_camera_block(&lines, i, &brush_ctx)?;
            scene_nodes.push(SceneNode::Camera(camera));
            i = end_ix + 1;
            continue;
        }

        if line.starts_with("<Tex") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            textures.push(parse_tex_node(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }

        if line.starts_with("<Buffer") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            buffers.push(parse_buffer_node(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }

        if line.starts_with("<Output") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            outputs.push(parse_output_node(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }

        if line.starts_with("<Pass") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            passes.push(parse_pass_node(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }

        if line.starts_with("<Present") {
            let (tag, end_ix) = collect_self_closing_block(&lines, i)?;
            if present.is_some() {
                return Err(GraphParseError {
                    line: i + 1,
                    message: "Only one <Present ... /> node is supported.".to_string(),
                });
            }
            present = Some(parse_present_node(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }

        i += 1;
    }

    let present = present.ok_or_else(|| GraphParseError {
        line: graph_start_ix + 1,
        message: "Missing <Present from=\"...\" /> node.".to_string(),
    })?;

    validate_graph(
        scope,
        fps,
        duration_ms,
        size,
        &inputs,
        &textures,
        &buffers,
        &solids,
        &texts,
        &images,
        &svgs,
        &scenes,
        &scene_nodes,
        &outputs,
        &passes,
        &present,
        graph_start_ix + 1,
    )?;

    Ok(GraphScript {
        id,
        version,
        scope,
        fps,
        apply,
        duration_ms,
        duration_explicit,
        size,
        render_size,
        inputs,
        textures,
        buffers,
        solids,
        texts,
        images,
        svgs,
        scenes,
        scene_nodes,
        passes,
        outputs,
        present,
    })
}

fn validate_graph(
    scope: GraphScope,
    fps: f32,
    duration_ms: u64,
    size: (u32, u32),
    inputs: &[InputNode],
    textures: &[TexNode],
    buffers: &[BufferNode],
    solids: &[SolidNode],
    texts: &[TextNode],
    images: &[ImageNode],
    svgs: &[SvgNode],
    scenes: &[SceneRootNode],
    scene_nodes: &[SceneNode],
    outputs: &[OutputNode],
    passes: &[PassNode],
    present: &PresentNode,
    line: usize,
) -> Result<(), GraphParseError> {
    if !fps.is_finite() || fps <= 0.0 {
        return Err(GraphParseError {
            line,
            message: "fps must be a positive number.".to_string(),
        });
    }
    if duration_ms == 0 {
        return Err(GraphParseError {
            line,
            message: "duration must be greater than zero.".to_string(),
        });
    }
    if size.0 == 0 || size.1 == 0 {
        return Err(GraphParseError {
            line,
            message: "size width and height must be greater than zero.".to_string(),
        });
    }
    let has_scene_nodes = !solids.is_empty()
        || !texts.is_empty()
        || !images.is_empty()
        || !svgs.is_empty()
        || !scenes.is_empty()
        || !scene_nodes.is_empty();
    if scope != GraphScope::Scene && has_scene_nodes {
        return Err(GraphParseError {
            line,
            message: "Scene nodes are only valid when scope=\"scene\".".to_string(),
        });
    }
    if passes.is_empty() && !(scope == GraphScope::Scene && has_scene_nodes) {
        return Err(GraphParseError {
            line,
            message: "Graph requires at least one <Pass ... /> node.".to_string(),
        });
    }

    let mut resource_ids = HashSet::<String>::new();
    if scope == GraphScope::Scene && has_scene_nodes {
        resource_ids.insert("scene".to_string());
    }
    for scene in scenes {
        if !resource_ids.insert(scene.id.clone()) {
            return Err(GraphParseError {
                line,
                message: format!("Duplicate resource id: {}", scene.id),
            });
        }
        let prefixed = format!("scene:{}", scene.id);
        if !resource_ids.insert(prefixed.clone()) {
            return Err(GraphParseError {
                line,
                message: format!("Duplicate resource id: {}", prefixed),
            });
        }
    }
    for input in inputs {
        if !resource_ids.insert(input.id.clone()) {
            return Err(GraphParseError {
                line,
                message: format!("Duplicate resource id: {}", input.id),
            });
        }
    }
    for tex in textures {
        if !resource_ids.insert(tex.id.clone()) {
            return Err(GraphParseError {
                line,
                message: format!("Duplicate resource id: {}", tex.id),
            });
        }
    }
    for buf in buffers {
        if !resource_ids.insert(buf.id.clone()) {
            return Err(GraphParseError {
                line,
                message: format!("Duplicate resource id: {}", buf.id),
            });
        }
    }
    for output in outputs {
        if !resource_ids.insert(output.id.clone()) {
            return Err(GraphParseError {
                line,
                message: format!("Duplicate resource id: {}", output.id),
            });
        }
        if let Some(src) = &output.from
            && !resource_ids.contains(src)
        {
            return Err(GraphParseError {
                line,
                message: format!("Output {} source not found: {}", output.id, src),
            });
        }
    }

    let mut pass_ids = HashSet::<String>::new();
    for pass in passes {
        if !pass_ids.insert(pass.id.clone()) {
            return Err(GraphParseError {
                line,
                message: format!("Duplicate pass id: {}", pass.id),
            });
        }
        if pass.inputs.is_empty() {
            return Err(GraphParseError {
                line,
                message: format!("Pass {} must declare at least one input.", pass.id),
            });
        }
        if pass.outputs.is_empty() {
            return Err(GraphParseError {
                line,
                message: format!("Pass {} must declare at least one output.", pass.id),
            });
        }
        for tex_in in &pass.inputs {
            if !resource_ids.contains(tex_in.resource_id()) {
                return Err(GraphParseError {
                    line,
                    message: format!(
                        "Pass {} input resource not found: {}",
                        pass.id,
                        tex_in.resource_id()
                    ),
                });
            }
        }
        for tex_out in &pass.outputs {
            if !resource_ids.contains(tex_out.resource_id()) {
                return Err(GraphParseError {
                    line,
                    message: format!(
                        "Pass {} output resource not found: {}",
                        pass.id,
                        tex_out.resource_id()
                    ),
                });
            }
        }
    }

    if !resource_ids.contains(&present.from) {
        return Err(GraphParseError {
            line,
            message: format!("Present source resource not found: {}", present.from),
        });
    }

    Ok(())
}

fn collect_self_closing_block(
    lines: &[&str],
    start: usize,
) -> Result<(String, usize), GraphParseError> {
    collect_tag_block(lines, start, '/', true)
}

fn collect_tag_block(
    lines: &[&str],
    start: usize,
    end_char: char,
    requires_self_closing: bool,
) -> Result<(String, usize), GraphParseError> {
    let mut out = String::new();
    let mut in_double_quote = false;
    let mut prev_char: Option<char> = None;
    for (ix, line) in lines.iter().enumerate().skip(start) {
        let trimmed = line.trim();
        out.push_str(trimmed);
        out.push('\n');
        for ch in trimmed.chars() {
            if ch == '"' {
                in_double_quote = !in_double_quote;
                continue;
            }
            if in_double_quote {
                prev_char = Some(ch);
                continue;
            }
            if requires_self_closing {
                // detect '/>' outside quoted attributes only
                if ch == '>' && prev_char == Some('/') {
                    return Ok((out, ix));
                }
            } else if ch == end_char {
                return Ok((out, ix));
            }
            prev_char = Some(ch);
        }
        prev_char = Some('\n');
    }
    Err(GraphParseError {
        line: start + 1,
        message: "Tag block is not closed.".to_string(),
    })
}

fn starts_open_tag(line: &str, tag_name: &str) -> bool {
    let Some(rest) = line.trim_start().strip_prefix('<') else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(tag_name) else {
        return false;
    };
    matches!(
        rest.chars().next(),
        None | Some(' ') | Some('\t') | Some('\r') | Some('\n') | Some('>') | Some('/')
    )
}

fn starts_close_tag(line: &str, tag_name: &str) -> bool {
    let Some(rest) = line.trim_start().strip_prefix("</") else {
        return false;
    };
    rest.strip_prefix(tag_name)
        .is_some_and(|rest| rest.trim_start().starts_with('>'))
}

fn parse_scene_root_block(
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
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((SceneRootNode { id, size, children }, close_ix))
}

fn parse_group_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(GroupNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Group")?;
    let brush = attr_value(&open_tag, "brush").map(|v| strip_wrappers(&v).to_string());
    let mut child_ctx = brush_ctx.with_inherited_brush(brush);
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((parse_group_node(&open_tag, start + 1, children)?, close_ix))
}

fn parse_part_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(PartNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Part")?;
    let brush = attr_value(&open_tag, "brush").map(|v| strip_wrappers(&v).to_string());
    let mut child_ctx = brush_ctx.with_inherited_brush(brush);
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((parse_part_node(&open_tag, start + 1, children)?, close_ix))
}

fn parse_repeat_block(
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

fn parse_mask_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(MaskNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Mask")?;
    let mut child_ctx = brush_ctx.clone();
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((parse_mask_node(&open_tag, start + 1, children)?, close_ix))
}

fn parse_camera_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(CameraNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Camera")?;
    let mut child_ctx = brush_ctx.clone();
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((parse_camera_node(&open_tag, start + 1, children)?, close_ix))
}

fn parse_character_block(
    lines: &[&str],
    start: usize,
    brush_ctx: &BrushParseContext,
) -> Result<(CharacterNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Character")?;
    let mut child_ctx = brush_ctx.clone();
    let children = parse_scene_nodes(lines, open_end_ix + 1, close_ix, &mut child_ctx)?;
    Ok((
        parse_character_node(&open_tag, start + 1, children)?,
        close_ix,
    ))
}

fn find_matching_close_tag(
    lines: &[&str],
    start: usize,
    tag_name: &str,
) -> Result<usize, GraphParseError> {
    let mut depth = 0usize;
    for (ix, line) in lines.iter().enumerate().skip(start) {
        let trimmed = line.trim_start();
        if starts_close_tag(trimmed, tag_name) {
            if depth == 0 {
                return Ok(ix);
            }
            depth = depth.saturating_sub(1);
            continue;
        }
        if starts_open_tag(trimmed, tag_name) && !trimmed.contains("/>") {
            depth = depth.saturating_add(1);
        }
    }
    Err(GraphParseError {
        line: start + 1,
        message: format!("Missing </{tag_name}> closing tag."),
    })
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
            let (defs, end_ix) = parse_defs_block(lines, i)?;
            brush_ctx.define_brushes(&defs.brushes);
            nodes.push(SceneNode::Defs(defs));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Solid") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            nodes.push(SceneNode::Solid(parse_solid_node(&tag, i + 1)?));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Text") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            nodes.push(SceneNode::Text(parse_text_node(&tag, i + 1)?));
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
            let (mask, end_ix) = parse_mask_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Mask(mask));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Character") {
            let (character, end_ix) = parse_character_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Character(character));
            i = end_ix + 1;
            continue;
        }
        if starts_open_tag(line, "Camera") {
            let (camera, end_ix) = parse_camera_block(lines, i, brush_ctx)?;
            nodes.push(SceneNode::Camera(camera));
            i = end_ix + 1;
            continue;
        }
        i += 1;
    }
    Ok(nodes)
}

fn parse_input_node(block: &str, line: usize) -> Result<InputNode, GraphParseError> {
    let id = strip_wrappers(&required_attr_value(block, "id", line)?).to_string();
    let input_type = attr_value(block, "type")
        .as_deref()
        .map(|raw| parse_input_type(raw, line, "type"))
        .transpose()?
        .unwrap_or(InputType::Video);
    let from = attr_value(block, "from").map(|v| strip_wrappers(&v).to_string());
    let fmt = attr_value(block, "fmt")
        .map(|v| parse_texture_format(&v, line, "fmt"))
        .transpose()?;
    let size = attr_value(block, "size")
        .as_deref()
        .map(|v| parse_size(v, line, "size"))
        .transpose()?;
    let color_space = attr_value(block, "colorSpace")
        .or_else(|| attr_value(block, "color_space"))
        .map(|v| parse_color_space(&v, line, "colorSpace"))
        .transpose()?;
    let alpha = attr_value(block, "alpha")
        .map(|v| parse_alpha_mode(&v, line, "alpha"))
        .transpose()?;

    Ok(InputNode {
        id,
        r#type: input_type,
        from,
        fmt,
        size,
        color_space,
        alpha,
    })
}

fn parse_tex_node(block: &str, line: usize) -> Result<TexNode, GraphParseError> {
    let id = required_attr_value(block, "id", line)?;
    let fmt = required_attr_value(block, "fmt", line)?;
    let from = attr_value(block, "from")
        .or_else(|| attr_value(block, "src"))
        .map(|v| strip_wrappers(&v).to_string());
    let size = attr_value(block, "size")
        .as_deref()
        .map(|v| parse_size(v, line, "size"))
        .transpose()?;
    let usage = attr_value(block, "usage")
        .as_deref()
        .map(|v| parse_tex_usage_array(v, line, "usage"))
        .transpose()?
        .unwrap_or_default();
    let transient = attr_value(block, "transient")
        .as_deref()
        .map(|v| parse_bool(v, line, "transient"))
        .transpose()?;
    let pingpong = attr_value(block, "pingpong").map(|v| strip_wrappers(&v).to_string());

    Ok(TexNode {
        id: strip_wrappers(&id).to_string(),
        fmt: parse_texture_format(&fmt, line, "fmt")?,
        from,
        size,
        usage,
        transient,
        pingpong,
    })
}

fn parse_buffer_node(block: &str, line: usize) -> Result<BufferNode, GraphParseError> {
    let id = strip_wrappers(&required_attr_value(block, "id", line)?).to_string();
    let elem_raw = required_attr_value_any(block, &["elemType", "elem_type"], line)?;
    let elem_type = parse_buffer_elem_type(&elem_raw, line, "elemType")?;
    let length = attr_value(block, "length")
        .as_deref()
        .map(|v| parse_u32(v, line, "length"))
        .transpose()?;
    let stride = attr_value(block, "stride")
        .as_deref()
        .map(|v| parse_u32(v, line, "stride"))
        .transpose()?;
    let usage = attr_value(block, "usage")
        .as_deref()
        .map(|v| parse_buffer_usage_array(v, line, "usage"))
        .transpose()?
        .unwrap_or_default();
    let transient = attr_value(block, "transient")
        .as_deref()
        .map(|v| parse_bool(v, line, "transient"))
        .transpose()?;
    let pingpong = attr_value(block, "pingpong").map(|v| strip_wrappers(&v).to_string());

    Ok(BufferNode {
        id,
        elem_type,
        length,
        stride,
        usage,
        transient,
        pingpong,
    })
}

fn parse_solid_node(block: &str, line: usize) -> Result<SolidNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let color = strip_wrappers(&required_attr_value(block, "color", line)?).to_string();
    Ok(SolidNode { id, color })
}

fn parse_text_node(block: &str, line: usize) -> Result<TextNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let value = strip_wrappers(&required_attr_value(block, "value", line)?).to_string();
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "center".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "center".to_string());
    let width = attr_value(block, "width").map(|v| strip_wrappers(&v).to_string());
    let font_size = attr_value(block, "fontSize")
        .or_else(|| attr_value(block, "font_size"))
        .or_else(|| attr_value(block, "size"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "96".to_string());
    let line_height = attr_value(block, "lineHeight")
        .or_else(|| attr_value(block, "line_height"))
        .map(|v| strip_wrappers(&v).to_string());
    let color = attr_value(block, "color")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "#ffffff".to_string());
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    let font_family = attr_value(block, "fontFamily")
        .or_else(|| attr_value(block, "font_family"))
        .map(|v| strip_wrappers(&v).to_string());
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
        width,
        font_size,
        line_height,
        color,
        opacity,
        visible_chars,
        max_lines,
        font_family,
        font_path,
    })
}

fn parse_image_node(block: &str, line: usize) -> Result<ImageNode, GraphParseError> {
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

fn parse_svg_node(block: &str, line: usize) -> Result<SvgNode, GraphParseError> {
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

fn parse_defs_block(lines: &[&str], start: usize) -> Result<(DefsNode, usize), GraphParseError> {
    let (open_tag, open_end_ix) = collect_tag_block(lines, start, '>', false)?;
    let close_ix = find_matching_close_tag(lines, open_end_ix + 1, "Defs")?;
    let id = attr_value(&open_tag, "id").map(|v| strip_wrappers(&v).to_string());
    let mut gradients = Vec::<GradientDef>::new();
    let mut brushes = Vec::<BrushDef>::new();
    let mut i = open_end_ix + 1;

    while i < close_ix {
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('{') {
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
        if starts_open_tag(line, "Brush") {
            let (tag, end_ix) = collect_self_closing_block(lines, i)?;
            brushes.push(parse_brush_def(&tag, i + 1)?);
            i = end_ix + 1;
            continue;
        }
        return Err(GraphParseError {
            line: i + 1,
            message: format!(
                "<Defs> only accepts <LinearGradient />, <RadialGradient />, or <Brush />, got: {line}"
            ),
        });
    }

    Ok((
        DefsNode {
            id,
            gradients,
            brushes,
        },
        close_ix,
    ))
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

fn parse_rect_node(block: &str, line: usize) -> Result<RectNode, GraphParseError> {
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
        blend: attr_value(block, "blend")
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "normal".to_string()),
    })
}

fn parse_circle_node(block: &str, line: usize) -> Result<CircleNode, GraphParseError> {
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
        blend: attr_value(block, "blend")
            .map(|v| strip_wrappers(&v).to_string())
            .unwrap_or_else(|| "normal".to_string()),
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

fn parse_line_node(block: &str, line: usize) -> Result<LineNode, GraphParseError> {
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

fn parse_polyline_node(block: &str, line: usize) -> Result<PolylineNode, GraphParseError> {
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

fn parse_path_node(
    block: &str,
    line: usize,
    brush_ctx: &BrushParseContext,
) -> Result<PathNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let (brush_id, brush) = brush_ctx.brush_for_path(block);
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
    })
}

fn parse_face_jaw_node(block: &str, _line: usize) -> Result<FaceJawNode, GraphParseError> {
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

fn parse_shadow_node(block: &str, _line: usize) -> Result<ShadowNode, GraphParseError> {
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
        opacity,
        children,
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
    Ok(PartNode {
        id,
        label,
        role,
        brush,
        x,
        y,
        rotation,
        scale,
        opacity,
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
    let opacity = attr_value(block, "opacity")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "1.0".to_string());
    Ok(MaskNode {
        id,
        shape,
        x,
        y,
        width,
        height,
        radius,
        d,
        opacity,
        children,
    })
}

fn parse_camera_node(
    block: &str,
    _line: usize,
    children: Vec<SceneNode>,
) -> Result<CameraNode, GraphParseError> {
    let id = attr_value(block, "id").map(|v| strip_wrappers(&v).to_string());
    let mode = attr_value(block, "mode")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "2d".to_string());
    let x = attr_value(block, "x")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "center".to_string());
    let y = attr_value(block, "y")
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "center".to_string());
    let target_x = attr_value(block, "targetX")
        .or_else(|| attr_value(block, "target_x"))
        .map(|v| strip_wrappers(&v).to_string());
    let target_y = attr_value(block, "targetY")
        .or_else(|| attr_value(block, "target_y"))
        .map(|v| strip_wrappers(&v).to_string());
    let anchor_x = attr_value(block, "anchorX")
        .or_else(|| attr_value(block, "anchor_x"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0.5".to_string());
    let anchor_y = attr_value(block, "anchorY")
        .or_else(|| attr_value(block, "anchor_y"))
        .map(|v| strip_wrappers(&v).to_string())
        .unwrap_or_else(|| "0.5".to_string());
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
    let follow = attr_value(block, "follow").map(|v| strip_wrappers(&v).to_string());
    let viewport = attr_value(block, "viewport").map(|v| strip_wrappers(&v).to_string());
    let world_bounds = attr_value(block, "worldBounds")
        .or_else(|| attr_value(block, "world_bounds"))
        .map(|v| strip_wrappers(&v).to_string());

    Ok(CameraNode {
        id,
        mode,
        x,
        y,
        target_x,
        target_y,
        anchor_x,
        anchor_y,
        zoom,
        rotation,
        opacity,
        follow,
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
    Ok(CharacterNode {
        id,
        x,
        y,
        rotation,
        scale,
        opacity,
        children,
    })
}

fn parse_output_node(block: &str, line: usize) -> Result<OutputNode, GraphParseError> {
    let id = strip_wrappers(&required_attr_value(block, "id", line)?).to_string();
    let from = attr_value(block, "from").map(|v| strip_wrappers(&v).to_string());
    let to = attr_value(block, "to")
        .as_deref()
        .map(|v| parse_output_target(v, line, "to"))
        .transpose()?
        .unwrap_or(OutputTarget::Screen);
    let fmt = attr_value(block, "fmt")
        .map(|v| parse_texture_format(&v, line, "fmt"))
        .transpose()?;
    let size = attr_value(block, "size")
        .as_deref()
        .map(|v| parse_size(v, line, "size"))
        .transpose()?;
    let color_space = attr_value(block, "colorSpace")
        .or_else(|| attr_value(block, "color_space"))
        .map(|v| parse_color_space(&v, line, "colorSpace"))
        .transpose()?;
    let alpha = attr_value(block, "alpha")
        .map(|v| parse_alpha_mode(&v, line, "alpha"))
        .transpose()?;

    Ok(OutputNode {
        id,
        from,
        to,
        fmt,
        size,
        color_space,
        alpha,
    })
}

fn parse_pass_node(block: &str, line: usize) -> Result<PassNode, GraphParseError> {
    let id = strip_wrappers(&required_attr_value(block, "id", line)?).to_string();
    let kind = attr_value(block, "kind")
        .as_deref()
        .map(|v| parse_pass_kind(v, line, "kind"))
        .transpose()?
        .unwrap_or(PassKind::Compute);
    let role = attr_value(block, "role")
        .as_deref()
        .map(|v| parse_pass_role(v, line, "role"))
        .transpose()?;
    let kernel = attr_value(block, "kernel").map(|v| strip_wrappers(&v).to_string());
    let mode = attr_value(block, "mode").map(|v| strip_wrappers(&v).to_string());
    let effect = strip_wrappers(&required_attr_value(block, "effect", line)?).to_string();
    let transition = attr_value(block, "transition")
        .as_deref()
        .map(|v| parse_transition_mode(v, line, "transition"))
        .transpose()?;
    let transition_fallback = attr_value(block, "transitionFallback")
        .or_else(|| attr_value(block, "transition_fallback"))
        .as_deref()
        .map(|v| parse_transition_fallback(v, line, "transitionFallback"))
        .transpose()?;
    let transition_easing = attr_value(block, "transitionEasing")
        .or_else(|| attr_value(block, "transition_easing"))
        .as_deref()
        .map(|v| parse_transition_easing(v, line, "transitionEasing"))
        .transpose()?;
    let transition_clips = attr_value(block, "transitionClips")
        .or_else(|| attr_value(block, "transition_clips"))
        .as_deref()
        .map(|v| parse_transition_clips(v, line, "transitionClips"))
        .transpose()?;
    let inputs = parse_resource_ref_array(&required_attr_value(block, "in", line)?, line, "in")?;
    let outputs = parse_resource_ref_array(&required_attr_value(block, "out", line)?, line, "out")?;
    let params = parse_params(block);
    let iterate = attr_value(block, "iterate")
        .as_deref()
        .map(|v| parse_quality_u32(v, line, "iterate"))
        .transpose()?;
    let pingpong = attr_value(block, "pingpong").map(|v| strip_wrappers(&v).to_string());
    let cache = attr_value(block, "cache")
        .as_deref()
        .map(|v| parse_pass_cache(v, line, "cache"))
        .transpose()?;
    let blend = attr_value(block, "blend")
        .as_deref()
        .map(|v| parse_blend_mode(v, line, "blend"))
        .transpose()?;
    let load_op = attr_value(block, "loadOp")
        .or_else(|| attr_value(block, "load_op"))
        .as_deref()
        .map(|v| parse_load_op(v, line, "loadOp"))
        .transpose()?;
    let store_op = attr_value(block, "storeOp")
        .or_else(|| attr_value(block, "store_op"))
        .as_deref()
        .map(|v| parse_store_op(v, line, "storeOp"))
        .transpose()?;

    Ok(PassNode {
        id,
        kind,
        role,
        kernel,
        mode,
        effect,
        transition,
        transition_fallback,
        transition_easing,
        transition_clips,
        inputs,
        outputs,
        params,
        iterate,
        pingpong,
        cache,
        blend,
        load_op,
        store_op,
    })
}

fn parse_present_node(block: &str, line: usize) -> Result<PresentNode, GraphParseError> {
    let from = strip_wrappers(&required_attr_value(block, "from", line)?).to_string();
    let to = attr_value(block, "to")
        .as_deref()
        .map(|v| parse_present_target(v, line, "to"))
        .transpose()?
        .unwrap_or(PresentTarget::Screen);
    let vsync = attr_value(block, "vsync")
        .as_deref()
        .map(|v| parse_bool(v, line, "vsync"))
        .transpose()?;
    Ok(PresentNode { from, to, vsync })
}

fn parse_params(block: &str) -> Vec<PassParam> {
    let Some(start_ix) = block.find("params={{") else {
        return Vec::new();
    };
    let after = &block[start_ix + "params={{".len()..];
    let Some(end_ix) = after.find("}}") else {
        return Vec::new();
    };
    let body = &after[..end_ix];
    let mut cleaned_body = String::new();
    for line in body.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if !cleaned_body.is_empty() {
            cleaned_body.push(' ');
        }
        cleaned_body.push_str(line);
    }
    let mut params = Vec::new();
    for entry in split_top_level_csv(&cleaned_body) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let Some(colon_ix) = entry.find(':') else {
            continue;
        };
        let key = entry[..colon_ix].trim().trim_end_matches(',');
        let value = entry[colon_ix + 1..].trim().trim_end_matches(',');
        if key.is_empty() || value.is_empty() {
            continue;
        }
        params.push(PassParam {
            key: key.to_string(),
            value: value.to_string(),
        });
    }
    params
}

fn split_top_level_csv(input: &str) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut cur = String::new();
    let mut paren_depth = 0_i32;
    let mut brace_depth = 0_i32;
    let mut bracket_depth = 0_i32;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escape = false;

    for ch in input.chars() {
        if escape {
            cur.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' {
            cur.push(ch);
            escape = true;
            continue;
        }
        if in_single_quote {
            cur.push(ch);
            if ch == '\'' {
                in_single_quote = false;
            }
            continue;
        }
        if in_double_quote {
            cur.push(ch);
            if ch == '"' {
                in_double_quote = false;
            }
            continue;
        }
        match ch {
            '\'' => {
                in_single_quote = true;
                cur.push(ch);
            }
            '"' => {
                in_double_quote = true;
                cur.push(ch);
            }
            '(' => {
                paren_depth += 1;
                cur.push(ch);
            }
            ')' => {
                paren_depth -= 1;
                cur.push(ch);
            }
            '{' => {
                brace_depth += 1;
                cur.push(ch);
            }
            '}' => {
                brace_depth -= 1;
                cur.push(ch);
            }
            '[' => {
                bracket_depth += 1;
                cur.push(ch);
            }
            ']' => {
                bracket_depth -= 1;
                cur.push(ch);
            }
            ',' if paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 => {
                let token = cur.trim();
                if !token.is_empty() {
                    out.push(token.to_string());
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let token = cur.trim();
    if !token.is_empty() {
        out.push(token.to_string());
    }
    out
}

fn parse_fps(block: &str, line: usize) -> Result<f32, GraphParseError> {
    let raw = required_attr_value(block, "fps", line)?;
    let text = strip_wrappers(&raw);
    let fps = text.parse::<f32>().map_err(|_| GraphParseError {
        line,
        message: format!("Invalid fps value: {}", text),
    })?;
    Ok(fps)
}

fn parse_duration_ms(block: &str, line: usize, default_ms: u64) -> Result<u64, GraphParseError> {
    let Some(raw) = attr_value(block, "duration") else {
        return Ok(default_ms);
    };
    let text = strip_wrappers(&raw);
    if let Some(ms) = text.strip_suffix("ms") {
        let val = ms.trim().parse::<f64>().map_err(|_| GraphParseError {
            line,
            message: format!("Invalid duration value: {}", text),
        })?;
        return Ok(val.max(0.0).round() as u64);
    }
    if let Some(sec) = text.strip_suffix('s') {
        let val = sec.trim().parse::<f64>().map_err(|_| GraphParseError {
            line,
            message: format!("Invalid duration value: {}", text),
        })?;
        return Ok((val.max(0.0) * 1000.0).round() as u64);
    }
    let val = text.parse::<f64>().map_err(|_| GraphParseError {
        line,
        message: format!("Invalid duration value: {}", text),
    })?;
    Ok((val.max(0.0) * 1000.0).round() as u64)
}

fn parse_graph_apply_scope(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<GraphApplyScope, GraphParseError> {
    match strip_wrappers(raw).to_ascii_lowercase().as_str() {
        "clip" => Ok(GraphApplyScope::Clip),
        "graph" => Ok(GraphApplyScope::Graph),
        other => Err(GraphParseError {
            line,
            message: format!(
                "Invalid {} '{}'. Expected one of: clip, graph.",
                field, other
            ),
        }),
    }
}

fn parse_graph_scope(raw: &str, line: usize, field: &str) -> Result<GraphScope, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "layer" | "adjustment" => Ok(GraphScope::Layer),
        "clip" | "clip-fusion" => Ok(GraphScope::Clip),
        "scene" | "fusion" | "fusion-comp" => Ok(GraphScope::Scene),
        other => Err(GraphParseError {
            line,
            message: format!(
                "Invalid {} '{}'. Expected one of: layer, clip, scene.",
                field, other
            ),
        }),
    }
}

fn parse_size(raw: &str, line: usize, field: &str) -> Result<(u32, u32), GraphParseError> {
    let text = strip_wrappers(raw).trim();
    let inner = text
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| GraphParseError {
            line,
            message: format!("{} must be an array [width,height].", field),
        })?;
    let mut parts = inner.split(',').map(str::trim);
    let Some(w) = parts.next() else {
        return Err(GraphParseError {
            line,
            message: format!("{} is missing width.", field),
        });
    };
    let Some(h) = parts.next() else {
        return Err(GraphParseError {
            line,
            message: format!("{} is missing height.", field),
        });
    };
    let width = w.parse::<u32>().map_err(|_| GraphParseError {
        line,
        message: format!("Invalid {} width: {}", field, w),
    })?;
    let height = h.parse::<u32>().map_err(|_| GraphParseError {
        line,
        message: format!("Invalid {} height: {}", field, h),
    })?;
    Ok((width, height))
}

fn parse_resource_ref_array(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<Vec<ResourceRef>, GraphParseError> {
    let text = strip_wrappers(raw).trim();
    let inner = text
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| GraphParseError {
            line,
            message: format!("{} must be an array of resource refs.", field),
        })?;
    let mut out = Vec::<ResourceRef>::new();
    for item in split_top_level_csv(inner) {
        let token = item.trim();
        if token.is_empty() {
            continue;
        }
        out.push(parse_resource_ref(token, line, field)?);
    }
    if out.is_empty() {
        return Err(GraphParseError {
            line,
            message: format!("{} cannot be empty.", field),
        });
    }
    Ok(out)
}

fn parse_resource_ref(
    token: &str,
    line: usize,
    field: &str,
) -> Result<ResourceRef, GraphParseError> {
    let trimmed = token.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let body = &trimmed[1..trimmed.len() - 1];
        let entries = parse_inline_object_entries(body);
        if let Some(tex_raw) = entries.get("tex") {
            let sample = entries
                .get("sample")
                .map(|raw| parse_sample_config(raw, line, field))
                .transpose()?;
            return Ok(ResourceRef::Tex {
                tex: strip_wrappers(tex_raw).to_string(),
                sample,
            });
        }
        if let Some(buf_raw) = entries.get("buf") {
            return Ok(ResourceRef::Buffer {
                buf: strip_wrappers(buf_raw).to_string(),
            });
        }
        if let Some(id_raw) = entries.get("id") {
            return Ok(ResourceRef::Id {
                id: strip_wrappers(id_raw).to_string(),
            });
        }
        if let Some(target_raw) = entries.get("target") {
            return Ok(ResourceRef::Id {
                id: strip_wrappers(target_raw).to_string(),
            });
        }
        return Err(GraphParseError {
            line,
            message: format!(
                "{} object ref must contain one of: tex|buf|id|target",
                field
            ),
        });
    }

    let id = strip_wrappers(trimmed);
    if id.is_empty() {
        return Err(GraphParseError {
            line,
            message: format!("{} contains an empty resource id.", field),
        });
    }
    Ok(ResourceRef::Id { id: id.to_string() })
}

fn parse_sample_config(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<SampleConfig, GraphParseError> {
    let raw_trimmed = raw.trim();
    let entries = if raw_trimmed.starts_with('{') && raw_trimmed.ends_with('}') {
        parse_inline_object_entries(&raw_trimmed[1..raw_trimmed.len() - 1])
    } else {
        let text = strip_wrappers(raw).trim();
        if text.is_empty() || !text.contains(':') {
            return Err(GraphParseError {
                line,
                message: format!("{}.sample must be an object.", field),
            });
        }
        parse_inline_object_entries(text)
    };
    let filter = entries
        .get("filter")
        .map(|raw| parse_sample_filter(raw, line, "sample.filter"))
        .transpose()?;
    let address = entries
        .get("address")
        .map(|raw| parse_sample_address(raw, line, "sample.address"))
        .transpose()?;
    Ok(SampleConfig { filter, address })
}

fn parse_inline_object_entries(body: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::<String, String>::new();
    for entry in split_top_level_csv(body) {
        let Some((k, v)) = entry.split_once(':') else {
            continue;
        };
        let key = strip_wrappers(k).trim().to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        map.insert(key, v.trim().to_string());
    }
    map
}

fn parse_tex_usage_array(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<Vec<TexUsage>, GraphParseError> {
    parse_enum_array(raw, line, field, parse_tex_usage)
}

fn parse_buffer_usage_array(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<Vec<BufferUsage>, GraphParseError> {
    parse_enum_array(raw, line, field, parse_buffer_usage)
}

fn parse_enum_array<T>(
    raw: &str,
    line: usize,
    field: &str,
    parser: fn(&str, usize, &str) -> Result<T, GraphParseError>,
) -> Result<Vec<T>, GraphParseError> {
    let text = strip_wrappers(raw).trim();
    let inner = text
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| GraphParseError {
            line,
            message: format!("{} must be an array.", field),
        })?;
    let mut out = Vec::new();
    for chunk in split_top_level_csv(inner) {
        let token = chunk.trim();
        if token.is_empty() {
            continue;
        }
        out.push(parser(token, line, field)?);
    }
    Ok(out)
}

fn parse_quality_u32(raw: &str, line: usize, field: &str) -> Result<Quality<u32>, GraphParseError> {
    let raw_trimmed = raw.trim();
    if raw_trimmed.starts_with('{') && raw_trimmed.ends_with('}') {
        let entries = parse_inline_object_entries(&raw_trimmed[1..raw_trimmed.len() - 1]);
        let Some(preview_raw) = entries.get("preview") else {
            return Err(GraphParseError {
                line,
                message: format!("{} quality object missing preview.", field),
            });
        };
        let final_raw = entries.get("final").ok_or_else(|| GraphParseError {
            line,
            message: format!("{} quality object missing final.", field),
        })?;
        return Ok(Quality::Split {
            preview: parse_u32(preview_raw, line, "preview")?,
            r#final: parse_u32(final_raw, line, "final")?,
        });
    }

    let text = strip_wrappers(raw).trim();
    if text.contains(':') {
        let entries = parse_inline_object_entries(text);
        let Some(preview_raw) = entries.get("preview") else {
            return Err(GraphParseError {
                line,
                message: format!("{} quality object missing preview.", field),
            });
        };
        let final_raw = entries.get("final").ok_or_else(|| GraphParseError {
            line,
            message: format!("{} quality object missing final.", field),
        })?;
        return Ok(Quality::Split {
            preview: parse_u32(preview_raw, line, "preview")?,
            r#final: parse_u32(final_raw, line, "final")?,
        });
    }

    Ok(Quality::Uniform(parse_u32(text, line, field)?))
}

fn parse_u32(raw: &str, line: usize, field: &str) -> Result<u32, GraphParseError> {
    let text = strip_wrappers(raw);
    text.parse::<u32>().map_err(|_| GraphParseError {
        line,
        message: format!("Invalid {} value: {}", field, text),
    })
}

fn parse_bool(raw: &str, line: usize, field: &str) -> Result<bool, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} boolean value: {}", field, other),
        }),
    }
}

fn parse_texture_format(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<TextureFormat, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "rgba8" => Ok(TextureFormat::Rgba8),
        "rgba8unorm" => Ok(TextureFormat::Rgba8Unorm),
        "rgba8unorm-srgb" => Ok(TextureFormat::Rgba8UnormSrgb),
        "bgra8unorm" => Ok(TextureFormat::Bgra8Unorm),
        "bgra8unorm-srgb" => Ok(TextureFormat::Bgra8UnormSrgb),
        "rgba16f" => Ok(TextureFormat::Rgba16f),
        "rgba32f" => Ok(TextureFormat::Rgba32f),
        "r16f" => Ok(TextureFormat::R16f),
        "r32f" => Ok(TextureFormat::R32f),
        "depth24plus" => Ok(TextureFormat::Depth24plus),
        "depth32f" => Ok(TextureFormat::Depth32f),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} format: {}", field, other),
        }),
    }
}

fn parse_color_space(raw: &str, line: usize, field: &str) -> Result<ColorSpace, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "srgb" => Ok(ColorSpace::Srgb),
        "linear-srgb" => Ok(ColorSpace::LinearSrgb),
        "display-p3" => Ok(ColorSpace::DisplayP3),
        "rec709" => Ok(ColorSpace::Rec709),
        "rec2020" => Ok(ColorSpace::Rec2020),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_alpha_mode(raw: &str, line: usize, field: &str) -> Result<AlphaMode, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "straight" => Ok(AlphaMode::Straight),
        "premul" => Ok(AlphaMode::Premul),
        "opaque" => Ok(AlphaMode::Opaque),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_input_type(raw: &str, line: usize, field: &str) -> Result<InputType, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "video" => Ok(InputType::Video),
        "image" => Ok(InputType::Image),
        "mask" => Ok(InputType::Mask),
        "depth" => Ok(InputType::Depth),
        "normal" => Ok(InputType::Normal),
        "motion" => Ok(InputType::Motion),
        "audio" => Ok(InputType::Audio),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_tex_usage(raw: &str, line: usize, field: &str) -> Result<TexUsage, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "sampled" => Ok(TexUsage::Sampled),
        "storage" => Ok(TexUsage::Storage),
        "color-attachment" => Ok(TexUsage::ColorAttachment),
        "depth-stencil-attachment" => Ok(TexUsage::DepthStencilAttachment),
        "copy-src" => Ok(TexUsage::CopySrc),
        "copy-dst" => Ok(TexUsage::CopyDst),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_buffer_usage(raw: &str, line: usize, field: &str) -> Result<BufferUsage, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "uniform" => Ok(BufferUsage::Uniform),
        "storage" => Ok(BufferUsage::Storage),
        "vertex" => Ok(BufferUsage::Vertex),
        "index" => Ok(BufferUsage::Index),
        "indirect" => Ok(BufferUsage::Indirect),
        "copy-src" => Ok(BufferUsage::CopySrc),
        "copy-dst" => Ok(BufferUsage::CopyDst),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_buffer_elem_type(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<BufferElemType, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "f32" => Ok(BufferElemType::F32),
        "u32" => Ok(BufferElemType::U32),
        "i32" => Ok(BufferElemType::I32),
        "vec2f" => Ok(BufferElemType::Vec2f),
        "vec4f" => Ok(BufferElemType::Vec4f),
        "mat4f" => Ok(BufferElemType::Mat4f),
        "struct" => Ok(BufferElemType::Struct),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_pass_kind(raw: &str, line: usize, field: &str) -> Result<PassKind, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "compute" => Ok(PassKind::Compute),
        "render" => Ok(PassKind::Render),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_pass_role(raw: &str, line: usize, field: &str) -> Result<PassRole, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "effect" => Ok(PassRole::Effect),
        "transition" => Ok(PassRole::Transition),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_pass_cache(raw: &str, line: usize, field: &str) -> Result<PassCache, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "none" => Ok(PassCache::None),
        "frame" => Ok(PassCache::Frame),
        "static" => Ok(PassCache::Static),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_blend_mode(raw: &str, line: usize, field: &str) -> Result<BlendMode, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "replace" => Ok(BlendMode::Replace),
        "add" => Ok(BlendMode::Add),
        "screen" => Ok(BlendMode::Screen),
        "multiply" => Ok(BlendMode::Multiply),
        "over" => Ok(BlendMode::Over),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_transition_mode(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<PassTransitionMode, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "auto" => Ok(PassTransitionMode::Auto),
        "off" => Ok(PassTransitionMode::Off),
        "force" => Ok(PassTransitionMode::Force),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_transition_fallback(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<PassTransitionFallback, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "under" => Ok(PassTransitionFallback::Under),
        "prev" => Ok(PassTransitionFallback::Prev),
        "next" => Ok(PassTransitionFallback::Next),
        "skip" => Ok(PassTransitionFallback::Skip),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_transition_easing(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<PassTransitionEasing, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "linear" => Ok(PassTransitionEasing::Linear),
        "ease-in" => Ok(PassTransitionEasing::EaseIn),
        "ease-out" => Ok(PassTransitionEasing::EaseOut),
        "ease-in-out" => Ok(PassTransitionEasing::EaseInOut),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_transition_clips(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<PassTransitionClips, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "overlap" => Ok(PassTransitionClips::Overlap),
        "non-overlap" => Ok(PassTransitionClips::NonOverlap),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_load_op(raw: &str, line: usize, field: &str) -> Result<LoadOp, GraphParseError> {
    let text = strip_wrappers(raw).trim();
    if text.starts_with('{') && text.ends_with('}') {
        let entries = parse_inline_object_entries(&text[1..text.len() - 1]);
        if let Some(clear_raw) = entries.get("clear") {
            let clear = parse_vec4_f32(clear_raw, line, "clear")?;
            return Ok(LoadOp::Clear(clear));
        }
    }
    match normalize_ident(raw).as_str() {
        "load" => Ok(LoadOp::Load),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_store_op(raw: &str, line: usize, field: &str) -> Result<StoreOp, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "store" => Ok(StoreOp::Store),
        "discard" => Ok(StoreOp::Discard),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_output_target(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<OutputTarget, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "screen" => Ok(OutputTarget::Screen),
        "file" => Ok(OutputTarget::File),
        "host" => Ok(OutputTarget::Host),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_present_target(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<PresentTarget, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "screen" => Ok(PresentTarget::Screen),
        "host" => Ok(PresentTarget::Host),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_sample_filter(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<SampleFilter, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "nearest" => Ok(SampleFilter::Nearest),
        "linear" => Ok(SampleFilter::Linear),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_sample_address(
    raw: &str,
    line: usize,
    field: &str,
) -> Result<SampleAddress, GraphParseError> {
    match normalize_ident(raw).as_str() {
        "clamp" => Ok(SampleAddress::Clamp),
        "repeat" => Ok(SampleAddress::Repeat),
        "mirror" => Ok(SampleAddress::Mirror),
        other => Err(GraphParseError {
            line,
            message: format!("Invalid {} value: {}", field, other),
        }),
    }
}

fn parse_vec4_f32(raw: &str, line: usize, field: &str) -> Result<[f32; 4], GraphParseError> {
    let text = strip_wrappers(raw).trim();
    let inner = text
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| GraphParseError {
            line,
            message: format!("{} must be [r,g,b,a].", field),
        })?;
    let parts: Vec<&str> = inner
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() != 4 {
        return Err(GraphParseError {
            line,
            message: format!("{} must have 4 values.", field),
        });
    }
    let mut out = [0.0f32; 4];
    for (ix, raw_part) in parts.iter().enumerate() {
        out[ix] = raw_part.parse::<f32>().map_err(|_| GraphParseError {
            line,
            message: format!("{} has invalid number: {}", field, raw_part),
        })?;
    }
    Ok(out)
}

fn normalize_ident(raw: &str) -> String {
    strip_wrappers(raw)
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-")
}

fn required_attr_value(block: &str, key: &str, line: usize) -> Result<String, GraphParseError> {
    attr_value(block, key).ok_or_else(|| GraphParseError {
        line,
        message: format!("Missing required attribute: {}", key),
    })
}

fn required_attr_value_any(
    block: &str,
    keys: &[&str],
    line: usize,
) -> Result<String, GraphParseError> {
    for key in keys {
        if let Some(v) = attr_value(block, key) {
            return Ok(v);
        }
    }
    Err(GraphParseError {
        line,
        message: format!("Missing required attribute: {}", keys.join("|")),
    })
}

fn attr_value(block: &str, key: &str) -> Option<String> {
    let start = find_attr_start(block, key)?;
    let mut rest = block[start..].trim_start();
    if !rest.starts_with('=') {
        return None;
    }
    rest = rest[1..].trim_start();
    if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"')?;
        return Some(stripped[..end].to_string());
    }
    if let Some(stripped) = rest.strip_prefix('{') {
        let mut depth = 1usize;
        let mut out = String::new();
        for ch in stripped.chars() {
            if ch == '{' {
                depth += 1;
                out.push(ch);
                continue;
            }
            if ch == '}' {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(out);
                }
                out.push(ch);
                continue;
            }
            out.push(ch);
        }
        return None;
    }
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    rest = &rest[..end];
    Some(rest.to_string())
}

fn find_attr_start(block: &str, key: &str) -> Option<usize> {
    let bytes = block.as_bytes();
    let key_bytes = key.as_bytes();
    if key_bytes.is_empty() || bytes.len() < key_bytes.len() + 1 {
        return None;
    }
    let mut in_double_quote = false;
    let mut i = 0usize;
    while i + key_bytes.len() < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            in_double_quote = !in_double_quote;
            i += 1;
            continue;
        }
        if in_double_quote {
            i += 1;
            continue;
        }
        if &bytes[i..i + key_bytes.len()] == key_bytes {
            let prev_ok = i == 0
                || bytes[i - 1].is_ascii_whitespace()
                || bytes[i - 1] == b'<'
                || bytes[i - 1] == b'\n';
            let mut j = i + key_bytes.len();
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if prev_ok && j < bytes.len() && bytes[j] == b'=' {
                return Some(i + key_bytes.len());
            }
        }
        i += 1;
    }
    None
}

fn strip_wrappers(raw: &str) -> &str {
    let mut text = raw.trim();
    loop {
        if text.starts_with('{') && text.ends_with('}') && text.len() >= 2 {
            text = text[1..text.len() - 1].trim();
            continue;
        }
        if text.starts_with('"') && text.ends_with('"') && text.len() >= 2 {
            text = text[1..text.len() - 1].trim();
            continue;
        }
        break;
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{
        ColorSpace, GraphApplyScope, GraphParseError, GraphScope, InputType, PassCache, PassKind,
        PassRole, PassTransitionClips, PassTransitionEasing, PassTransitionFallback,
        PassTransitionMode, Quality, ResourceRef, SceneNode, TextureFormat, is_graph_script,
        parse_graph_script,
    };

    #[test]
    fn graph_parser_accepts_basic_example() {
        let script = r#"
<Graph scope="clip" fps={60} duration="2s" size={[256,256]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[256,256]} />
  <Pass id="invert_pulse" kernel="invert_mix.wgsl" effect="invert_mix"
        in={["src"]}
        out={["out"]}
        params={{
          t: "$time.norm",
          mix: "0.5 + 0.5*sin($time.sec*6.28318)"
        }} />
  <Present from="out" />
</Graph>
"#;
        assert!(is_graph_script(script));
        let graph = parse_graph_script(script).expect("graph should parse");
        assert_eq!(graph.textures.len(), 2);
        assert_eq!(graph.passes.len(), 1);
        assert_eq!(graph.present.from, "out");
        assert_eq!(graph.scope, GraphScope::Clip);
        assert_eq!(graph.passes[0].kind, PassKind::Compute);
    }

    #[test]
    fn graph_parser_accepts_render_size() {
        let script = r##"
<Graph scope="scene" fps={60} duration="1s" size={[734,555]} renderSize={[3840,2160]}>
  <Scene id="scene0">
    <Solid color="#ffffff" />
  </Scene>
  <Present from="scene0" />
</Graph>
"##;
        let graph = parse_graph_script(script).expect("graph should parse");
        assert_eq!(graph.size, (734, 555));
        assert_eq!(graph.render_size, Some((3840, 2160)));
    }

    #[test]
    fn graph_parser_rejects_missing_resource_ref() {
        let script = r#"
<Graph scope="clip" fps={60} duration="2s" size={[256,256]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Pass id="invert" kernel="invert_mix.wgsl" effect="invert_mix" in={["src"]} out={["out"]} />
  <Present from="out" />
</Graph>
"#;
        let err = parse_graph_script(script).expect_err("missing tex should fail");
        assert!(
            err.message.contains("output resource not found"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn graph_parser_parses_new_nodes_and_enums() -> Result<(), GraphParseError> {
        let script = r#"
<Graph id="v2" version="2.0" scope="scene" fps={60} duration="2s" size={[1920,1080]}>
  <Input id="clip0" type="video" from="input:clip0" fmt="rgba8unorm-srgb" colorSpace="srgb" />
  <Buffer id="state" elemType="vec4f" usage={["storage","copy-dst"]} />
  <Tex id="work" fmt="rgba16f" usage={["sampled","storage"]} />
  <Output id="screen" to="screen" fmt="bgra8unorm-srgb" colorSpace="srgb" />
  <Pass id="prep" kind="compute" kernel="normalize_input.wgsl" effect="normalize_input"
        in={[{ tex:"clip0", sample:{ filter:"linear", address:"clamp" } }]}
        out={["work"]}
        cache="frame"
        iterate={{ preview: 1, final: 2 }} />
  <Present from="screen" to="screen" vsync={true} />
</Graph>
"#;
        let graph = parse_graph_script(script)?;
        assert_eq!(graph.id.as_deref(), Some("v2"));
        assert_eq!(graph.version.as_deref(), Some("2.0"));
        assert_eq!(graph.scope, GraphScope::Scene);
        assert_eq!(graph.inputs[0].r#type, InputType::Video);
        assert_eq!(graph.inputs[0].fmt, Some(TextureFormat::Rgba8UnormSrgb));
        assert_eq!(graph.inputs[0].color_space, Some(ColorSpace::Srgb));
        assert_eq!(graph.passes[0].cache, Some(PassCache::Frame));
        assert_eq!(
            graph.passes[0].iterate,
            Some(Quality::Split {
                preview: 1,
                r#final: 2
            })
        );
        match &graph.passes[0].inputs[0] {
            ResourceRef::Tex { tex, .. } => assert_eq!(tex, "clip0"),
            other => panic!("unexpected input ref: {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn graph_parser_accepts_scene_solid_text_without_passes() -> Result<(), GraphParseError> {
        let script = r##"
<Graph scope="scene" fps={60} duration="3s" size={[1920,1080]}>
  <Solid color="#000000" />
  <Text value="hello world"
        x="center"
        y="center"
        fontSize="96"
        color="#ffffff"
        opacity="min($time.sec / 1.0, 1.0)" />
  <Present from="scene" />
</Graph>
"##;
        let graph = parse_graph_script(script)?;
        assert_eq!(graph.scope, GraphScope::Scene);
        assert_eq!(graph.solids.len(), 1);
        assert_eq!(graph.texts.len(), 1);
        assert_eq!(graph.images.len(), 0);
        assert_eq!(graph.svgs.len(), 0);
        assert_eq!(graph.texts[0].value, "hello world");
        assert_eq!(graph.texts[0].font_size, "96");
        assert_eq!(graph.present.from, "scene");
        assert_eq!(graph.resource_size("scene"), Some((1920, 1080)));
        Ok(())
    }

    #[test]
    fn graph_parser_accepts_scene_image_without_passes() -> Result<(), GraphParseError> {
        let script = r##"
<Graph scope="scene" fps={60} duration="3s" size={[1920,1080]}>
  <Image src="/tmp/anica-test-image.png"
         x="center"
         y="120"
         scale="0.5 + 0.5*$time.norm"
         opacity="0.8" />
  <Present from="scene" />
</Graph>
"##;
        let graph = parse_graph_script(script)?;
        assert_eq!(graph.scope, GraphScope::Scene);
        assert_eq!(graph.images.len(), 1);
        assert_eq!(graph.images[0].src, "/tmp/anica-test-image.png");
        assert_eq!(graph.images[0].x, "center");
        assert_eq!(graph.images[0].y, "120");
        assert_eq!(graph.images[0].scale, "0.5 + 0.5*$time.norm");
        assert_eq!(graph.present.from, "scene");
        assert_eq!(graph.resource_size("scene"), Some((1920, 1080)));
        Ok(())
    }

    #[test]
    fn graph_parser_accepts_scene_svg_without_passes() -> Result<(), GraphParseError> {
        let script = r##"
<Graph scope="scene" fps={60} duration="3s" size={[1920,1080]}>
  <Svg src="/tmp/anica-test-logo.svg"
       x="center"
       y="25%"
       scale="0.5 + 0.5*$time.norm"
       opacity="0.8" />
  <Present from="scene" />
</Graph>
"##;
        let graph = parse_graph_script(script)?;
        assert_eq!(graph.scope, GraphScope::Scene);
        assert_eq!(graph.svgs.len(), 1);
        assert_eq!(graph.svgs[0].src, "/tmp/anica-test-logo.svg");
        assert_eq!(graph.svgs[0].x, "center");
        assert_eq!(graph.svgs[0].y, "25%");
        assert_eq!(graph.svgs[0].scale, "0.5 + 0.5*$time.norm");
        assert_eq!(graph.svgs[0].opacity, "0.8");
        assert_eq!(graph.present.from, "scene");
        assert_eq!(graph.resource_size("scene"), Some((1920, 1080)));
        Ok(())
    }

    #[test]
    fn graph_parser_accepts_scene_container_and_primitives() -> Result<(), GraphParseError> {
        let script = r##"
<Graph scope="scene" fps={60} duration="8s" size={[1920,1080]}>
  <Scene id="scene0">
    <Solid color="[0.96,0.96,0.96,1]" />
    <Group id="card" x="100" y="100" rotation="-4" opacity="1">
      <Shadow x="0" y="18" blur="36" color="[0,0,0,0.16]" />
      <Rect width="360" height="78" radius="20"
            color="[1,1,1,1]"
            stroke="[0.84,0.84,0.84,1]"
            strokeWidth="1" />
      <Circle x="38" y="39" radius="7" color="[0,0.34,0.95,1]" />
      <Polyline points="20,110 140,96 260,124"
                stroke="#2f83ff"
                strokeWidth="5"
                trimEnd="0.75"
                strokeStyle="sketch"
                strokeRoughness="1.5"
                strokeCopies="4"
                strokeTexture="0.45"
                strokeBristles="5"
                strokePressure="auto"
                strokePressureMin="0.25"
                strokePressureCurve="1.4" />
      <Path d="M 20 145 C 120 95 240 195 340 145"
            stroke="#ff7a2f"
            strokeWidth="6"
            trimStart="0.1"
            trimEnd="0.9"
            strokeStyle="pencil" />
      <Text value="TechCrunch - OpenAI..."
            x="62" y="22"
            width="270"
            fontSize="28"
            lineHeight="34"
            color="[0.08,0.08,0.08,1]" />
    </Group>
  </Scene>
  <Present from="scene0" />
</Graph>
"##;
        let graph = parse_graph_script(script)?;
        assert_eq!(graph.scope, GraphScope::Scene);
        assert_eq!(graph.scenes.len(), 1);
        assert_eq!(graph.scenes[0].id, "scene0");
        assert_eq!(graph.scenes[0].children.len(), 2);
        let SceneNode::Group(group) = &graph.scenes[0].children[1] else {
            panic!("expected group child");
        };
        assert_eq!(group.children.len(), 6);
        let SceneNode::Polyline(polyline) = &group.children[3] else {
            panic!("expected polyline child");
        };
        assert_eq!(polyline.stroke_style, "sketch");
        assert_eq!(polyline.stroke_roughness, "1.5");
        assert_eq!(polyline.stroke_copies, "4");
        assert_eq!(polyline.stroke_texture, "0.45");
        assert_eq!(polyline.stroke_bristles, "5");
        assert_eq!(polyline.stroke_pressure, "auto");
        assert_eq!(polyline.stroke_pressure_min, "0.25");
        assert_eq!(polyline.stroke_pressure_curve, "1.4");
        let SceneNode::Path(path) = &group.children[4] else {
            panic!("expected path child");
        };
        assert_eq!(path.stroke_style, "pencil");
        assert!(graph.scene_nodes.is_empty());
        assert_eq!(graph.present.from, "scene0");
        assert_eq!(graph.resource_size("scene0"), Some((1920, 1080)));
        assert_eq!(graph.resource_size("scene:scene0"), Some((1920, 1080)));
        Ok(())
    }

    #[test]
    fn graph_parser_accepts_brush_part_inheritance() -> Result<(), GraphParseError> {
        let script = r##"
<Graph scope="scene" fps={60} duration="1s" size={[320,180]}>
  <Scene id="scene0">
    <Defs>
      <Brush id="eyebrow_sketch"
             stroke="#111111"
             strokeWidth="0.3"
             strokeStyle="pencil"
             strokeRoughness="1.8"
             strokeCopies="6"
             strokeTexture="0.7"
             strokeBristles="5"
             opacity="0.4"
             lineCap="round" />
    </Defs>
    <Part id="left_eyebrow" label="Left eyebrow" role="eyebrow" brush="eyebrow_sketch" x="10" y="20">
      <Path id="brow_cluster"
            d="M 0 0 C 20 -8 40 -8 60 0 M 4 4 C 24 -2 42 -1 56 5" />
      <Path id="brow_override"
            d="M 0 10 L 60 10"
            strokeWidth="1.2"
            opacity="0.8" />
    </Part>
  </Scene>
  <Present from="scene0" />
</Graph>
"##;
        let graph = parse_graph_script(script)?;
        let SceneNode::Defs(defs) = &graph.scenes[0].children[0] else {
            panic!("expected defs child");
        };
        assert_eq!(defs.brushes.len(), 1);
        assert_eq!(defs.brushes[0].id, "eyebrow_sketch");

        let SceneNode::Part(part) = &graph.scenes[0].children[1] else {
            panic!("expected part child");
        };
        assert_eq!(part.id.as_deref(), Some("left_eyebrow"));
        assert_eq!(part.label.as_deref(), Some("Left eyebrow"));
        assert_eq!(part.role.as_deref(), Some("eyebrow"));
        assert_eq!(part.brush.as_deref(), Some("eyebrow_sketch"));

        let SceneNode::Path(inherited) = &part.children[0] else {
            panic!("expected inherited brush path");
        };
        assert_eq!(inherited.brush.as_deref(), Some("eyebrow_sketch"));
        assert_eq!(inherited.stroke, "#111111");
        assert_eq!(inherited.stroke_width, "0.3");
        assert_eq!(inherited.stroke_style, "pencil");
        assert_eq!(inherited.stroke_roughness, "1.8");
        assert_eq!(inherited.stroke_copies, "6");
        assert_eq!(inherited.stroke_texture, "0.7");
        assert_eq!(inherited.stroke_bristles, "5");
        assert_eq!(inherited.opacity, "0.4");

        let SceneNode::Path(overridden) = &part.children[1] else {
            panic!("expected override path");
        };
        assert_eq!(overridden.brush.as_deref(), Some("eyebrow_sketch"));
        assert_eq!(overridden.stroke_width, "1.2");
        assert_eq!(overridden.opacity, "0.8");
        Ok(())
    }

    #[test]
    fn graph_parser_accepts_scene_camera_container() -> Result<(), GraphParseError> {
        let script = r##"
<Graph scope="scene" fps={60} duration="3s" size={[1920,1080]}>
  <Scene id="scene0">
    <Solid color="#101418" />
    <Camera mode="2d"
            x="960 + 500*smoothstep(0,1,$time.norm)"
            y="540"
            targetX="1100"
            anchorX="35%"
            anchorY="0.6"
            zoom="1.4"
            follow="marker"
            viewport="100,80,1720,920"
            worldBounds="0,0,2400,1400">
      <Path d="M 300 600 C 700 300 1200 400 1600 520"
            stroke="#2f83ff"
            strokeWidth="8"
            trimEnd="smoothstep(0.2,0.8,$time.norm)" />
      <Circle id="marker" x="1200" y="500" radius="12" color="#ffffff" />
    </Camera>
  </Scene>
  <Present from="scene0" />
</Graph>
"##;
        let graph = parse_graph_script(script)?;
        let SceneNode::Camera(camera) = &graph.scenes[0].children[1] else {
            panic!("expected camera child");
        };
        assert_eq!(camera.mode, "2d");
        assert_eq!(camera.x, "960 + 500*smoothstep(0,1,$time.norm)");
        assert_eq!(camera.target_x.as_deref(), Some("1100"));
        assert_eq!(camera.anchor_x, "35%");
        assert_eq!(camera.anchor_y, "0.6");
        assert_eq!(camera.zoom, "1.4");
        assert_eq!(camera.follow.as_deref(), Some("marker"));
        assert_eq!(camera.viewport.as_deref(), Some("100,80,1720,920"));
        assert_eq!(camera.world_bounds.as_deref(), Some("0,0,2400,1400"));
        assert_eq!(camera.children.len(), 2);
        Ok(())
    }

    #[test]
    fn graph_parser_accepts_scene_character_container() -> Result<(), GraphParseError> {
        let script = r##"
<Graph scope="scene" fps={60} duration="3s" size={[1920,1080]}>
  <Scene id="scene0">
    <Solid color="#101418" />
    <Character id="heroFace" x="960" y="540" rotation="-2" scale="1.1" opacity="1">
      <Group x="0" y="-30">
        <Path d="M -160 0 C -80 -130 80 -130 160 0"
              stroke="#f6bfd0"
              strokeWidth="42" />
        <Line x1="-120" y1="24" x2="120" y2="24" width="6" color="#5d5961" />
        <Circle x="-70" y="70" radius="26" color="#e05b78" />
      </Group>
    </Character>
  </Scene>
  <Present from="scene0" />
</Graph>
"##;
        let graph = parse_graph_script(script)?;
        let SceneNode::Character(character) = &graph.scenes[0].children[1] else {
            panic!("expected character child");
        };
        assert_eq!(character.id.as_deref(), Some("heroFace"));
        assert_eq!(character.x, "960");
        assert_eq!(character.rotation, "-2");
        assert_eq!(character.children.len(), 1);
        Ok(())
    }

    #[test]
    fn graph_parser_accepts_scene_repeat_container() -> Result<(), GraphParseError> {
        let script = r##"
<Graph scope="scene" fps={60} duration="3s" size={[1920,1080]}>
  <Scene id="scene0">
    <Character id="eye" x="260" y="130">
      <Repeat id="lashFan"
              count="12"
              x="180"
              y="-10"
              xStep="5"
              yStep="-8"
              rotationStep="-1.5"
              scaleStep="0.02"
              opacityStep="-0.04">
        <Path d="M 0 0 C 42 24 76 62 100 114"
              stroke="#15091d"
              strokeWidth="5"
              lineCap="round"
              lineJoin="round"
              taperEnd="0.5" />
      </Repeat>
    </Character>
  </Scene>
  <Present from="scene0" />
</Graph>
"##;
        let graph = parse_graph_script(script)?;
        let SceneNode::Character(character) = &graph.scenes[0].children[0] else {
            panic!("expected character child");
        };
        let SceneNode::Repeat(repeat) = &character.children[0] else {
            panic!("expected repeat child");
        };
        assert_eq!(repeat.id.as_deref(), Some("lashFan"));
        assert_eq!(repeat.count, "12");
        assert_eq!(repeat.x_step, "5");
        assert_eq!(repeat.y_step, "-8");
        assert_eq!(repeat.rotation_step, "-1.5");
        assert_eq!(repeat.scale_step, "0.02");
        assert_eq!(repeat.opacity_step, "-0.04");
        assert_eq!(repeat.children.len(), 1);
        Ok(())
    }

    #[test]
    fn graph_parser_accepts_path_fill_taper_and_mask() -> Result<(), GraphParseError> {
        let script = r##"
<Graph scope="scene" fps={60} duration="3s" size={[1920,1080]}>
  <Scene id="scene0">
    <Solid color="#101418" />
    <Mask id="eyeClip" shape="circle" x="120" y="90" radius="40">
      <Path d="M 80 60 L 160 60 L 160 120 L 80 120 Z"
            fill="#ff77aa"
            stroke="#5d5961"
            strokeWidth="5"
            lineCap="round"
            lineJoin="round"
            taperStart="0.15"
            taperEnd="0.2" />
    </Mask>
  </Scene>
  <Present from="scene0" />
</Graph>
"##;
        let graph = parse_graph_script(script)?;
        let SceneNode::Mask(mask) = &graph.scenes[0].children[1] else {
            panic!("expected mask child");
        };
        assert_eq!(mask.id.as_deref(), Some("eyeClip"));
        assert_eq!(mask.shape, "circle");
        let SceneNode::Path(path) = &mask.children[0] else {
            panic!("expected masked path");
        };
        assert_eq!(path.fill.as_deref(), Some("#ff77aa"));
        assert_eq!(path.line_cap, "round");
        assert_eq!(path.taper_start, "0.15");
        assert_eq!(path.taper_end, "0.2");
        Ok(())
    }

    #[test]
    fn graph_parser_accepts_decimal_duration_two_dp() -> Result<(), GraphParseError> {
        let script = r#"
<Graph scope="clip" fps={60} duration="2.35s" size={[1920,1080]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[1920,1080]} />
  <Pass id="copy" kernel="invert_mix.wgsl" effect="invert_mix" in={["src"]} out={["out"]} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script)?;
        assert_eq!(graph.duration_ms, 2350);
        assert!(graph.duration_explicit);
        Ok(())
    }

    #[test]
    fn graph_parser_defaults_apply_clip_and_duration_when_omitted() -> Result<(), GraphParseError> {
        let script = r#"
<Graph scope="clip" fps={60} size={[1920,1080]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[1920,1080]} />
  <Pass id="copy" kernel="invert_mix.wgsl" effect="invert_mix" in={["src"]} out={["out"]} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script)?;
        assert_eq!(graph.apply, GraphApplyScope::Clip);
        assert_eq!(graph.duration_ms, 2000);
        assert!(!graph.duration_explicit);
        Ok(())
    }

    #[test]
    fn graph_parser_rejects_missing_scope() {
        let script = r#"
<Graph fps={60} size={[1920,1080]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[1920,1080]} />
  <Pass id="copy" kernel="invert_mix.wgsl" effect="invert_mix" in={["src"]} out={["out"]} />
  <Present from="out" />
</Graph>
"#;
        let err = parse_graph_script(script).expect_err("scope should be required");
        assert!(
            err.message.contains("Missing required attribute: scope"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn graph_parser_rejects_missing_pass_effect() {
        let script = r#"
<Graph scope="clip" fps={60} size={[1920,1080]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[1920,1080]} />
  <Pass id="copy" kernel="invert_mix.wgsl" in={["src"]} out={["out"]} />
  <Present from="out" />
</Graph>
"#;
        let err = parse_graph_script(script).expect_err("effect should be required");
        assert!(
            err.message.contains("Missing required attribute: effect"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn graph_parser_accepts_missing_pass_kernel_when_effect_present() -> Result<(), GraphParseError>
    {
        let script = r#"
<Graph scope="clip" fps={60} size={[1920,1080]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[1920,1080]} />
  <Pass id="copy" effect="exposure_contrast" in={["src"]} out={["out"]} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script)?;
        assert_eq!(graph.passes[0].kernel, None);
        assert_eq!(graph.passes[0].effect, "exposure_contrast");
        Ok(())
    }

    #[test]
    fn graph_parser_accepts_face_jaw_scene_node() -> Result<(), GraphParseError> {
        let script = r##"
<Graph scope="scene" fps={60} size={[512,512]}>
  <Scene id="face">
    <Group id="face_shape_param_group" x="0" y="0" scale="1">
      <FaceJaw id="jaw"
               x="256" y="64"
               width="300" height="360"
               cheekWidth="260"
               chinWidth="42"
               chinSharpness="0.35"
               jawEase="0.6"
               closed="false"
               stroke="#000000"
               strokeWidth="3"
               fill="none" />
    </Group>
  </Scene>
  <Present from="face" />
</Graph>
"##;
        let graph = parse_graph_script(script)?;
        let SceneNode::Group(group) = &graph.scenes[0].children[0] else {
            panic!("expected group");
        };
        let SceneNode::FaceJaw(face_jaw) = &group.children[0] else {
            panic!("expected FaceJaw");
        };
        assert_eq!(face_jaw.id.as_deref(), Some("jaw"));
        assert_eq!(face_jaw.chin_sharpness, "0.35");
        assert_eq!(face_jaw.jaw_ease, "0.6");
        Ok(())
    }

    #[test]
    fn graph_parser_params_support_single_line_multi_key_values() -> Result<(), GraphParseError> {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fx_hsla_overlay" effect="hsla_overlay" in={["under"]} out={["out"]}
        params={{ hue: "210.0", saturation: "0.70", lightness: "0.41", alpha: "0.45" }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script)?;
        let params = &graph.passes[0].params;
        assert_eq!(params.len(), 4);
        assert_eq!(params[0].key, "hue");
        assert_eq!(params[0].value, "\"210.0\"");
        assert_eq!(params[3].key, "alpha");
        assert_eq!(params[3].value, "\"0.45\"");
        Ok(())
    }

    #[test]
    fn graph_parser_params_preserve_curve_with_commas() -> Result<(), GraphParseError> {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fx_hsla_overlay" effect="hsla_overlay" in={["under"]} out={["out"]}
        params={{ hue: "210.0", saturation: "0.70", lightness: "0.41", alpha: curve("0.00:0.0:linear, 2.00:0.45:ease_in_out") }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script)?;
        let params = &graph.passes[0].params;
        assert_eq!(params.len(), 4);
        assert_eq!(params[3].key, "alpha");
        assert!(
            params[3]
                .value
                .contains("curve(\"0.00:0.0:linear, 2.00:0.45:ease_in_out\")")
        );
        Ok(())
    }

    #[test]
    fn graph_parser_parses_pass_transition_fields() -> Result<(), GraphParseError> {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Input id="prev" type="video" from="input:prev" />
  <Input id="next" type="video" from="input:next" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="dissolve" kind="render" role="transition"
        kernel="transition_core.wgsl"
        effect="dissolve"
        in={["prev","next"]} out={["out"]}
        transition="auto"
        transitionFallback="under"
        transitionEasing="ease-in-out"
        transitionClips="overlap" />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script)?;
        assert_eq!(graph.scope, GraphScope::Layer);
        let pass = &graph.passes[0];
        assert_eq!(pass.role, Some(PassRole::Transition));
        assert_eq!(pass.effect, "dissolve");
        assert_eq!(pass.transition, Some(PassTransitionMode::Auto));
        assert_eq!(
            pass.transition_fallback,
            Some(PassTransitionFallback::Under)
        );
        assert_eq!(
            pass.transition_easing,
            Some(PassTransitionEasing::EaseInOut)
        );
        assert_eq!(pass.transition_clips, Some(PassTransitionClips::Overlap));
        Ok(())
    }
}
