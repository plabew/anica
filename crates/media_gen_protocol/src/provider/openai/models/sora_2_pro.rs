use super::OpenAiModelSpec;
use crate::protocol::AssetKind;

pub(crate) const NAME: &str = "sora-2-pro";

pub(crate) const SPEC: OpenAiModelSpec = OpenAiModelSpec {
    name: NAME,
    asset_kind: AssetKind::Video,
    supports_image_edit: false,
};
