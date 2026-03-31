use crate::error::{ErrorCode, ProtocolError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRouteKey {
    pub provider: String,
    pub model_name: String,
}

impl ModelRouteKey {
    pub fn parse(input: &str) -> Result<Self> {
        let normalized = input.trim();
        let Some((provider, model_name)) = normalized.split_once('/') else {
            return Err(ProtocolError::new(
                ErrorCode::InvalidArgument,
                "model must use provider/model-name format",
            ));
        };
        let provider = provider.trim();
        let model_name = model_name.trim();
        if provider.is_empty() || model_name.is_empty() {
            return Err(ProtocolError::new(
                ErrorCode::InvalidArgument,
                "model route key must include both provider and model name",
            ));
        }
        Ok(Self {
            provider: provider.to_string(),
            model_name: model_name.to_string(),
        })
    }
}
