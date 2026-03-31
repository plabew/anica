use crate::error::{ErrorCode, ProtocolError, Result};
use crate::model::ModelRouteKey;
use crate::protocol::AssetKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSpec {
    pub route_key: String,
    pub label: String,
    pub supported_assets: Vec<AssetKind>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_slot: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelRegistry {
    items: HashMap<String, ModelSpec>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self {
            items: HashMap::new(),
        }
    }

    pub fn insert(&mut self, spec: ModelSpec) -> Result<()> {
        ModelRouteKey::parse(&spec.route_key)?;
        self.items.insert(spec.route_key.clone(), spec);
        Ok(())
    }

    pub fn get(&self, route_key: &str) -> Option<&ModelSpec> {
        self.items.get(route_key)
    }

    pub fn require_enabled(&self, route_key: &str) -> Result<&ModelSpec> {
        let Some(spec) = self.items.get(route_key) else {
            return Err(ProtocolError::new(
                ErrorCode::InvalidArgument,
                format!("unknown model route key: {route_key}"),
            ));
        };
        if !spec.enabled {
            return Err(ProtocolError::new(
                ErrorCode::InvalidArgument,
                format!("model is disabled: {route_key}"),
            ));
        }
        Ok(spec)
    }
}
