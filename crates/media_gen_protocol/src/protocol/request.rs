// =========================================
// =========================================
// crates/media_gen_protocol/src/protocol/request.rs

use crate::error::{ErrorCode, ProtocolError, Result};
use crate::model::ModelRouteKey;
use crate::protocol::{AssetKind, InputAsset};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerateRequest {
    /// Routing key: "{provider}/{model-name}".
    pub model: String,
    pub asset_kind: AssetKind,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<InputAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_sec: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    /// Provider-specific escape hatch.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub provider_options: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub metadata: Map<String, Value>,
}

impl GenerateRequest {
    pub fn route_key(&self) -> Result<ModelRouteKey> {
        ModelRouteKey::parse(&self.model)
    }

    pub fn validate(&self) -> Result<()> {
        self.route_key()?;
        if self.prompt.trim().is_empty() {
            return Err(ProtocolError::new(
                ErrorCode::InvalidArgument,
                "prompt cannot be empty",
            ));
        }
        if let Some(duration) = self.duration_sec
            && (!duration.is_finite() || duration <= 0.0)
        {
            return Err(ProtocolError::new(
                ErrorCode::InvalidArgument,
                "duration_sec must be a finite value > 0",
            ));
        }
        Ok(())
    }
}
