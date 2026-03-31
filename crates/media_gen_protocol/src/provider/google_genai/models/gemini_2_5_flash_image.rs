// =========================================
// =========================================
// crates/media_gen_protocol/src/provider/google_genai/models/gemini_2_5_flash_image.rs

use super::GoogleGenAiModelSpec;
use crate::protocol::AssetKind;

pub(crate) const NAME: &str = "gemini-2.5-flash-image";

pub(crate) const SPEC: GoogleGenAiModelSpec = GoogleGenAiModelSpec {
    name: NAME,
    api_model_name: NAME,
    asset_kind: AssetKind::Image,
};
