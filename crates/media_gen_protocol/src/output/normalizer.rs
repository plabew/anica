use crate::error::{ErrorCode, ProtocolError, Result};
use crate::output::OutputUploader;
use crate::protocol::OutputAsset;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderRawOutput {
    pub content_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base64_data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

pub async fn normalize_provider_output(
    raw: ProviderRawOutput,
    uploader: &dyn OutputUploader,
) -> Result<OutputAsset> {
    if let Some(url) = raw.url {
        return Ok(OutputAsset {
            url,
            content_type: raw.content_type,
            width: raw.width,
            height: raw.height,
            duration_ms: raw.duration_ms,
            sha256: None,
            bytes: None,
            expires_at_ms: None,
        });
    }

    if let Some(base64_data) = raw.base64_data {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(base64_data.trim())
            .map_err(|_| {
                ProtocolError::new(
                    ErrorCode::InvalidArgument,
                    "provider base64 output is invalid",
                )
            })?;
        let uploaded_url = uploader.upload_bytes(&raw.content_type, &decoded).await?;
        return Ok(OutputAsset {
            url: uploaded_url,
            content_type: raw.content_type,
            width: raw.width,
            height: raw.height,
            duration_ms: raw.duration_ms,
            sha256: None,
            bytes: Some(decoded.len() as u64),
            expires_at_ms: None,
        });
    }

    Err(ProtocolError::new(
        ErrorCode::JobFailed,
        "provider output contains neither url nor base64_data",
    ))
}
