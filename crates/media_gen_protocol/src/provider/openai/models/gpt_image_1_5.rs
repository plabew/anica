use super::OpenAiModelSpec;
use crate::protocol::AssetKind;

pub(crate) const NAME: &str = "gpt-image-1.5";

pub(crate) const SPEC: OpenAiModelSpec = OpenAiModelSpec {
    name: NAME,
    asset_kind: AssetKind::Image,
    supports_image_edit: false,
};
