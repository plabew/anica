// =========================================
// =========================================
// crates/media_gen_protocol/src/provider/google_genai/models/veo_3_1.rs

use super::GoogleGenAiModelSpec;
use crate::protocol::AssetKind;

pub(crate) const NAME: &str = "veo_3_1";
pub(crate) const API_MODEL_NAME: &str = "veo-3.1-generate-preview";

pub(crate) const SPEC: GoogleGenAiModelSpec = GoogleGenAiModelSpec {
    name: NAME,
    api_model_name: API_MODEL_NAME,
    asset_kind: AssetKind::Video,
};
