use serde::{Deserialize, Serialize};

use crate::dsl::{ImageNode, SolidNode, SvgNode, TextNode};

fn default_scene_blend() -> String {
    "normal".to_string()
}

fn default_stroke_style() -> String {
    "solid".to_string()
}

fn default_stroke_roughness() -> String {
    "0".to_string()
}

fn default_stroke_copies() -> String {
    "1".to_string()
}

fn default_stroke_texture() -> String {
    "0".to_string()
}

fn default_stroke_bristles() -> String {
    "0".to_string()
}

fn default_stroke_pressure() -> String {
    "none".to_string()
}

fn default_stroke_pressure_min() -> String {
    "1".to_string()
}

fn default_stroke_pressure_curve() -> String {
    "1".to_string()
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneRootNode {
    pub id: String,
    pub size: Option<(u32, u32)>,
    pub children: Vec<SceneNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SceneNode {
    Defs(DefsNode),
    Solid(SolidNode),
    Text(TextNode),
    Image(ImageNode),
    Svg(SvgNode),
    Rect(RectNode),
    Circle(CircleNode),
    Line(LineNode),
    Polyline(PolylineNode),
    Path(PathNode),
    FaceJaw(FaceJawNode),
    Shadow(ShadowNode),
    Group(GroupNode),
    Part(PartNode),
    Repeat(RepeatNode),
    Mask(MaskNode),
    Camera(CameraNode),
    Character(CharacterNode),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefsNode {
    pub id: Option<String>,
    pub gradients: Vec<GradientDef>,
    #[serde(default)]
    pub brushes: Vec<BrushDef>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrushDef {
    pub id: String,
    pub stroke: Option<String>,
    pub fill: Option<String>,
    pub stroke_width: Option<String>,
    pub opacity: Option<String>,
    pub line_cap: Option<String>,
    pub line_join: Option<String>,
    pub taper_start: Option<String>,
    pub taper_end: Option<String>,
    pub stroke_style: Option<String>,
    pub stroke_roughness: Option<String>,
    pub stroke_copies: Option<String>,
    pub stroke_texture: Option<String>,
    pub stroke_bristles: Option<String>,
    pub stroke_pressure: Option<String>,
    pub stroke_pressure_min: Option<String>,
    pub stroke_pressure_curve: Option<String>,
    pub blend: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GradientDef {
    Linear(LinearGradientDef),
    Radial(RadialGradientDef),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearGradientDef {
    pub id: String,
    pub x1: String,
    pub y1: String,
    pub x2: String,
    pub y2: String,
    pub stops: Vec<GradientStop>,
    pub units: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RadialGradientDef {
    pub id: String,
    pub cx: String,
    pub cy: String,
    pub r: String,
    pub fx: Option<String>,
    pub fy: Option<String>,
    pub stops: Vec<GradientStop>,
    pub units: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GradientStop {
    pub offset: f32,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RectNode {
    pub id: Option<String>,
    pub x: String,
    pub y: String,
    pub width: String,
    pub height: String,
    pub radius: String,
    pub color: String,
    pub stroke: Option<String>,
    pub stroke_width: String,
    pub opacity: String,
    pub rotation: String,
    #[serde(default = "default_scene_blend")]
    pub blend: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CircleNode {
    pub id: Option<String>,
    pub x: String,
    pub y: String,
    pub radius: String,
    pub color: String,
    pub stroke: Option<String>,
    pub stroke_width: String,
    pub opacity: String,
    #[serde(default = "default_scene_blend")]
    pub blend: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineNode {
    pub id: Option<String>,
    pub x1: String,
    pub y1: String,
    pub x2: String,
    pub y2: String,
    pub width: String,
    pub color: String,
    pub opacity: String,
    pub line_cap: String,
    pub taper_start: String,
    pub taper_end: String,
    #[serde(default = "default_stroke_style")]
    pub stroke_style: String,
    #[serde(default = "default_stroke_roughness")]
    pub stroke_roughness: String,
    #[serde(default = "default_stroke_copies")]
    pub stroke_copies: String,
    #[serde(default = "default_stroke_texture")]
    pub stroke_texture: String,
    #[serde(default = "default_stroke_bristles")]
    pub stroke_bristles: String,
    #[serde(default = "default_stroke_pressure")]
    pub stroke_pressure: String,
    #[serde(default = "default_stroke_pressure_min")]
    pub stroke_pressure_min: String,
    #[serde(default = "default_stroke_pressure_curve")]
    pub stroke_pressure_curve: String,
    #[serde(default = "default_scene_blend")]
    pub blend: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolylineNode {
    pub id: Option<String>,
    pub points: String,
    pub stroke: String,
    pub stroke_width: String,
    pub opacity: String,
    pub trim_start: String,
    pub trim_end: String,
    pub line_cap: String,
    pub line_join: String,
    pub taper_start: String,
    pub taper_end: String,
    #[serde(default = "default_stroke_style")]
    pub stroke_style: String,
    #[serde(default = "default_stroke_roughness")]
    pub stroke_roughness: String,
    #[serde(default = "default_stroke_copies")]
    pub stroke_copies: String,
    #[serde(default = "default_stroke_texture")]
    pub stroke_texture: String,
    #[serde(default = "default_stroke_bristles")]
    pub stroke_bristles: String,
    #[serde(default = "default_stroke_pressure")]
    pub stroke_pressure: String,
    #[serde(default = "default_stroke_pressure_min")]
    pub stroke_pressure_min: String,
    #[serde(default = "default_stroke_pressure_curve")]
    pub stroke_pressure_curve: String,
    #[serde(default = "default_scene_blend")]
    pub blend: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathNode {
    pub id: Option<String>,
    pub brush: Option<String>,
    pub d: String,
    pub stroke: String,
    pub fill: Option<String>,
    pub stroke_width: String,
    pub opacity: String,
    pub trim_start: String,
    pub trim_end: String,
    pub line_cap: String,
    pub line_join: String,
    pub taper_start: String,
    pub taper_end: String,
    #[serde(default = "default_stroke_style")]
    pub stroke_style: String,
    #[serde(default = "default_stroke_roughness")]
    pub stroke_roughness: String,
    #[serde(default = "default_stroke_copies")]
    pub stroke_copies: String,
    #[serde(default = "default_stroke_texture")]
    pub stroke_texture: String,
    #[serde(default = "default_stroke_bristles")]
    pub stroke_bristles: String,
    #[serde(default = "default_stroke_pressure")]
    pub stroke_pressure: String,
    #[serde(default = "default_stroke_pressure_min")]
    pub stroke_pressure_min: String,
    #[serde(default = "default_stroke_pressure_curve")]
    pub stroke_pressure_curve: String,
    #[serde(default = "default_scene_blend")]
    pub blend: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceJawNode {
    pub id: Option<String>,
    pub x: String,
    pub y: String,
    pub width: String,
    pub height: String,
    pub cheek_width: String,
    pub chin_width: String,
    pub chin_sharpness: String,
    pub jaw_ease: String,
    pub scale: String,
    pub closed: String,
    pub stroke: String,
    pub fill: Option<String>,
    pub stroke_width: String,
    pub opacity: String,
    pub trim_start: String,
    pub trim_end: String,
    pub line_cap: String,
    pub line_join: String,
    pub taper_start: String,
    pub taper_end: String,
    #[serde(default = "default_stroke_style")]
    pub stroke_style: String,
    #[serde(default = "default_stroke_roughness")]
    pub stroke_roughness: String,
    #[serde(default = "default_stroke_copies")]
    pub stroke_copies: String,
    #[serde(default = "default_stroke_texture")]
    pub stroke_texture: String,
    #[serde(default = "default_stroke_bristles")]
    pub stroke_bristles: String,
    #[serde(default = "default_stroke_pressure")]
    pub stroke_pressure: String,
    #[serde(default = "default_stroke_pressure_min")]
    pub stroke_pressure_min: String,
    #[serde(default = "default_stroke_pressure_curve")]
    pub stroke_pressure_curve: String,
    #[serde(default = "default_scene_blend")]
    pub blend: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowNode {
    pub id: Option<String>,
    pub x: String,
    pub y: String,
    pub blur: String,
    pub color: String,
    pub opacity: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupNode {
    pub id: Option<String>,
    pub brush: Option<String>,
    pub x: String,
    pub y: String,
    pub rotation: String,
    pub scale: String,
    pub opacity: String,
    pub children: Vec<SceneNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartNode {
    pub id: Option<String>,
    pub label: Option<String>,
    pub role: Option<String>,
    pub brush: Option<String>,
    pub x: String,
    pub y: String,
    pub rotation: String,
    pub scale: String,
    pub opacity: String,
    pub children: Vec<SceneNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepeatNode {
    pub id: Option<String>,
    pub count: String,
    pub x: String,
    pub y: String,
    pub rotation: String,
    pub scale: String,
    pub opacity: String,
    pub x_step: String,
    pub y_step: String,
    pub rotation_step: String,
    pub scale_step: String,
    pub opacity_step: String,
    pub children: Vec<SceneNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskNode {
    pub id: Option<String>,
    pub shape: String,
    pub x: String,
    pub y: String,
    pub width: String,
    pub height: String,
    pub radius: String,
    pub d: Option<String>,
    pub opacity: String,
    pub children: Vec<SceneNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraNode {
    pub id: Option<String>,
    pub mode: String,
    pub x: String,
    pub y: String,
    pub target_x: Option<String>,
    pub target_y: Option<String>,
    pub anchor_x: String,
    pub anchor_y: String,
    pub zoom: String,
    pub rotation: String,
    pub opacity: String,
    pub follow: Option<String>,
    pub viewport: Option<String>,
    pub world_bounds: Option<String>,
    pub children: Vec<SceneNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterNode {
    pub id: Option<String>,
    pub x: String,
    pub y: String,
    pub rotation: String,
    pub scale: String,
    pub opacity: String,
    pub children: Vec<SceneNode>,
}
