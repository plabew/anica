use serde::{Deserialize, Serialize};

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
pub struct LayerNode {
    pub id: String,
    pub effects: Vec<EffectNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectNode {
    pub id: Option<String>,
    pub r#type: String,
    pub params: Vec<PassParam>,
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
    #[serde(default)]
    pub input: Option<String>,
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
