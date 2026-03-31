use media_gen_protocol::protocol::{AssetKind, GenerateRequest};
use serde_json::json;

fn main() {
    let mut req = GenerateRequest {
        model: "runway/gen4-image".to_string(),
        asset_kind: AssetKind::Image,
        prompt: "cinematic alley at night".to_string(),
        negative_prompt: Some("watermark, logo".to_string()),
        inputs: Vec::new(),
        duration_sec: None,
        aspect_ratio: Some("16:9".to_string()),
        provider_options: Default::default(),
        callback_url: None,
        idempotency_key: None,
        metadata: Default::default(),
    };

    req.provider_options
        .insert("version".to_string(), json!("2025-03-01"));
    req.provider_options.insert(
        "camera_control".to_string(),
        json!({"pan": 0.2, "tilt": -0.1}),
    );

    println!(
        "provider options count = {} (caller passes through provider-specific options)",
        req.provider_options.len()
    );
}
