// =========================================
// =========================================
// crates/media_gen_protocol/src/provider/google_genai/models/gemini_3_pro_image_preview.rs

use super::GoogleGenAiModelSpec;
use crate::protocol::AssetKind;

pub(crate) const NAME: &str = "gemini-3-pro-image-preview";

pub(crate) const SPEC: GoogleGenAiModelSpec = GoogleGenAiModelSpec {
    name: NAME,
    api_model_name: NAME,
    asset_kind: AssetKind::Image,
};
