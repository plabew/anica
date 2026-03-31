use crate::error::{ErrorCode, ProtocolError, Result};
use crate::model::ModelRouteKey;
use crate::protocol::AssetKind;

pub(crate) mod gpt_image_1;
pub(crate) mod gpt_image_1_5;
pub(crate) mod gpt_image_1_mini;
pub(crate) mod sora_2;
pub(crate) mod sora_2_pro;

pub(crate) use gpt_image_1::NAME as MODEL_GPT_IMAGE_1;
pub(crate) use gpt_image_1_5::NAME as MODEL_GPT_IMAGE_1_5;
pub(crate) use gpt_image_1_mini::NAME as MODEL_GPT_IMAGE_1_MINI;
pub(crate) use sora_2::NAME as MODEL_SORA_2;
pub(crate) use sora_2_pro::NAME as MODEL_SORA_2_PRO;

#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenAiModelSpec {
    pub(crate) name: &'static str,
    pub(crate) asset_kind: AssetKind,
    pub(crate) supports_image_edit: bool,
}

const SUPPORTED_OPENAI_MODELS: &[&str] = &[
    MODEL_GPT_IMAGE_1,
    MODEL_GPT_IMAGE_1_MINI,
    MODEL_GPT_IMAGE_1_5,
    MODEL_SORA_2,
    MODEL_SORA_2_PRO,
];

pub(crate) fn spec(model_name: &str) -> Option<OpenAiModelSpec> {
    match model_name {
        MODEL_GPT_IMAGE_1 => Some(gpt_image_1::SPEC),
        MODEL_GPT_IMAGE_1_MINI => Some(gpt_image_1_mini::SPEC),
        MODEL_GPT_IMAGE_1_5 => Some(gpt_image_1_5::SPEC),
        MODEL_SORA_2 => Some(sora_2::SPEC),
        MODEL_SORA_2_PRO => Some(sora_2_pro::SPEC),
        _ => None,
    }
}

pub(crate) fn is_image_model(model_name: &str) -> bool {
    spec(model_name)
        .map(|model| model.asset_kind == AssetKind::Image)
        .unwrap_or(false)
}

pub(crate) fn require_supported_model(route: &ModelRouteKey) -> Result<OpenAiModelSpec> {
    spec(route.model_name.as_str()).ok_or_else(|| {
        let supported = SUPPORTED_OPENAI_MODELS.join(", ");
        ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "unsupported openai model '{}', expected one of: {supported}",
                route.model_name
            ),
        )
        .with_provider("openai")
    })
}

pub(crate) fn require_model_supports_asset_kind(
    route: &ModelRouteKey,
    requested_kind: AssetKind,
) -> Result<OpenAiModelSpec> {
    let model = require_supported_model(route)?;
    if model.asset_kind == requested_kind {
        return Ok(model);
    }

    Err(ProtocolError::new(
        ErrorCode::InvalidArgument,
        format!(
            "model '{}' supports asset_kind='{}', but request asset_kind='{}'",
            model.name,
            asset_kind_label(model.asset_kind),
            asset_kind_label(requested_kind)
        ),
    )
    .with_provider("openai"))
}

pub(crate) fn require_model_supports_image_edit(route: &ModelRouteKey) -> Result<()> {
    let model = require_model_supports_asset_kind(route, AssetKind::Image)?;

    if model.supports_image_edit {
        return Ok(());
    }

    Err(ProtocolError::new(
        ErrorCode::InvalidArgument,
        format!(
            "model '{}' does not support image input edits; use '{}' for image/mask inputs",
            model.name, MODEL_GPT_IMAGE_1
        ),
    )
    .with_provider("openai"))
}

fn asset_kind_label(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Image => "image",
        AssetKind::Video => "video",
    }
}
