use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Image,
    Video,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputAssetKind {
    Image,
    Video,
    Audio,
    Mask,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputAsset {
    pub kind: InputAssetKind,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}
