// =========================================
// =========================================
// crates/motionloom/src/dsl.rs

pub use crate::error::GraphParseError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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
    pub inputs: Vec<InputNode>,
    pub textures: Vec<TexNode>,
    pub buffers: Vec<BufferNode>,
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
            "Graph parsed: scope={:?}, fps={:.2}, apply={:?}, duration={}ms, size={}x{}, input={}, tex={}, buffer={}, pass={}, output={}, present={}",
            self.scope,
            self.fps,
            self.apply,
            self.duration_ms,
            self.size.0,
            self.size.1,
            self.inputs.len(),
            self.textures.len(),
            self.buffers.len(),
            self.passes.len(),
            self.outputs.len(),
            self.present.from
        )
    }

    pub fn resource_size(&self, id: &str) -> Option<(u32, u32)> {
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
}

pub fn is_graph_script(input: &str) -> bool {
    input.contains("<Graph")
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
    let mut outputs = Vec::<OutputNode>::new();
    let mut passes = Vec::<PassNode>::new();
    let mut present: Option<PresentNode> = None;
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
        fps,
        duration_ms,
        size,
        &inputs,
        &textures,
        &buffers,
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
        inputs,
        textures,
        buffers,
        passes,
        outputs,
        present,
    })
}

fn validate_graph(
    fps: f32,
    duration_ms: u64,
    size: (u32, u32),
    inputs: &[InputNode],
    textures: &[TexNode],
    buffers: &[BufferNode],
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
    if passes.is_empty() {
        return Err(GraphParseError {
            line,
            message: "Graph requires at least one <Pass ... /> node.".to_string(),
        });
    }

    let mut resource_ids = HashSet::<String>::new();
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
    for (ix, line) in lines.iter().enumerate().skip(start) {
        let trimmed = line.trim();
        out.push_str(trimmed);
        out.push('\n');
        if requires_self_closing {
            if trimmed.contains("/>") {
                return Ok((out, ix));
            }
        } else if trimmed.contains(end_char) {
            return Ok((out, ix));
        }
    }
    Err(GraphParseError {
        line: start + 1,
        message: "Tag block is not closed.".to_string(),
    })
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
    let pattern = format!("{key}=");
    let start = block.find(&pattern)?;
    let mut rest = block[start + pattern.len()..].trim_start();
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
        PassTransitionMode, Quality, ResourceRef, TextureFormat, is_graph_script,
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
