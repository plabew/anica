use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AcpMotionLoomGetSceneScriptRequest {}

#[derive(Debug, Clone, Serialize)]
pub struct AcpMotionLoomGetSceneScriptResponse {
    pub ok: bool,
    pub script: String,
    pub script_length: usize,
    pub script_revision: u64,
    pub apply_revision: u64,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AcpMotionLoomSetSceneScriptRequest {
    pub script: String,
    #[serde(default)]
    pub apply_now: bool,
    #[serde(default = "default_focus_vfx_page")]
    pub focus_vfx_page: bool,
}

fn default_focus_vfx_page() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct AcpMotionLoomSetSceneScriptResponse {
    pub ok: bool,
    pub updated: bool,
    pub apply_requested: bool,
    pub focus_vfx_page: bool,
    pub script_length: usize,
    pub script_revision: u64,
    pub apply_revision: u64,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AcpMotionLoomRenderSceneRequest {
    #[serde(default = "default_render_mode")]
    pub mode: String,
    #[serde(default = "default_focus_vfx_page")]
    pub focus_vfx_page: bool,
}

fn default_render_mode() -> String {
    "gpu".to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct AcpMotionLoomRenderSceneResponse {
    pub ok: bool,
    pub queued: bool,
    pub mode: String,
    pub focus_vfx_page: bool,
    pub render_revision: u64,
    pub message: String,
}
