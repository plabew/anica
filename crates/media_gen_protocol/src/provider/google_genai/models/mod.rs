// =========================================
// =========================================
// crates/media_gen_protocol/src/provider/google_genai/models/mod.rs

use crate::error::{ErrorCode, ProtocolError, Result};
use crate::model::ModelRouteKey;
use crate::protocol::AssetKind;

pub(crate) mod gemini_2_5_flash_image;
pub(crate) mod gemini_3_1_flash_image_preview;
pub(crate) mod gemini_3_pro_image_preview;
pub(crate) mod veo_3_1;

pub(crate) use gemini_2_5_flash_image::NAME as MODEL_GEMINI_2_5_FLASH_IMAGE;
pub(crate) use gemini_3_1_flash_image_preview::NAME as MODEL_GEMINI_3_1_FLASH_IMAGE_PREVIEW;
pub(crate) use gemini_3_pro_image_preview::NAME as MODEL_GEMINI_3_PRO_IMAGE_PREVIEW;
pub(crate) use veo_3_1::NAME as MODEL_VEO_3_1;

#[derive(Debug, Clone, Copy)]
pub(crate) struct GoogleGenAiModelSpec {
    pub(crate) name: &'static str,
    pub(crate) api_model_name: &'static str,
    pub(crate) asset_kind: AssetKind,
}

const SUPPORTED_GOOGLE_MODELS: &[&str] = &[
    MODEL_GEMINI_3_1_FLASH_IMAGE_PREVIEW,
    MODEL_GEMINI_3_PRO_IMAGE_PREVIEW,
    MODEL_GEMINI_2_5_FLASH_IMAGE,
    MODEL_VEO_3_1,
];

#[cfg(test)]
pub(crate) fn supported_model_names() -> Vec<String> {
    SUPPORTED_GOOGLE_MODELS
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

pub(crate) fn spec(model_name: &str) -> Option<GoogleGenAiModelSpec> {
    match model_name {
        MODEL_GEMINI_3_1_FLASH_IMAGE_PREVIEW => Some(gemini_3_1_flash_image_preview::SPEC),
        MODEL_GEMINI_3_PRO_IMAGE_PREVIEW => Some(gemini_3_pro_image_preview::SPEC),
        MODEL_GEMINI_2_5_FLASH_IMAGE => Some(gemini_2_5_flash_image::SPEC),
        MODEL_VEO_3_1 | veo_3_1::API_MODEL_NAME => Some(veo_3_1::SPEC),
        _ => None,
    }
}

pub(crate) fn require_supported_model(route: &ModelRouteKey) -> Result<GoogleGenAiModelSpec> {
    spec(route.model_name.as_str()).ok_or_else(|| {
        let supported = SUPPORTED_GOOGLE_MODELS.join(", ");
        ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "unsupported google model '{}'; expected one of: {supported}",
                route.model_name
            ),
        )
        .with_provider("google")
    })
}

pub(crate) fn require_model_supports_asset_kind(
    route: &ModelRouteKey,
    requested_kind: AssetKind,
) -> Result<GoogleGenAiModelSpec> {
    let model = require_supported_model(route)?;
    if model.asset_kind == requested_kind {
        return Ok(model);
    }

    let compatible = compatible_models_for_asset_kind(requested_kind);
    let alternatives = if compatible.is_empty() {
        // Keep alternatives explicit so callers can quickly discover valid choices.
        let available_models = SUPPORTED_GOOGLE_MODELS.join(", ");
        format!(
            "no google models currently support asset_kind='{}'; available models: {available_models}",
            asset_kind_label(requested_kind)
        )
    } else {
        format!("use one of: {}", compatible.join(", "))
    };

    Err(ProtocolError::new(
        ErrorCode::InvalidArgument,
        format!(
            "model '{}' supports asset_kind='{}', but request asset_kind='{}'; {alternatives}",
            model.name,
            asset_kind_label(model.asset_kind),
            asset_kind_label(requested_kind)
        ),
    )
    .with_provider("google"))
}

fn compatible_models_for_asset_kind(requested_kind: AssetKind) -> Vec<&'static str> {
    SUPPORTED_GOOGLE_MODELS
        .iter()
        .copied()
        .filter(|model_name| {
            spec(model_name)
                .map(|model| model.asset_kind == requested_kind)
                .unwrap_or(false)
        })
        .collect()
}

fn asset_kind_label(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Image => "image",
        AssetKind::Video => "video",
    }
}
