use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextTransformNode {
    #[serde(default)]
    pub x: Option<String>,
    #[serde(default)]
    pub y: Option<String>,
    #[serde(default)]
    pub rotation: Option<String>,
    #[serde(default)]
    pub scale: Option<String>,
    #[serde(default)]
    pub scale_x: Option<String>,
    #[serde(default)]
    pub scale_y: Option<String>,
    #[serde(default)]
    pub skew_x: Option<String>,
    #[serde(default)]
    pub skew_y: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyleOverrideNode {
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub opacity: Option<String>,
    #[serde(default)]
    pub blur: Option<String>,
    #[serde(default)]
    pub stroke: Option<String>,
    #[serde(default)]
    pub stroke_width: Option<String>,
    #[serde(default)]
    pub stroke_join: Option<String>,
    #[serde(default)]
    pub stroke_position: Option<String>,
    #[serde(default)]
    pub shadow_color: Option<String>,
    #[serde(default)]
    pub shadow_x: Option<String>,
    #[serde(default)]
    pub shadow_y: Option<String>,
    #[serde(default)]
    pub shadow_blur: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TextEffectNode {
    Glow(TextGlowEffectNode),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextGlowEffectNode {
    pub radius: String,
    pub intensity: String,
    #[serde(default)]
    pub color: Option<String>,
}
