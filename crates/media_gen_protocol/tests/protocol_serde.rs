use media_gen_protocol::protocol::{AssetKind, GenerateRequest};

#[test]
fn request_roundtrip_json() {
    let req = GenerateRequest {
        model: "openai/gpt-image-1".to_string(),
        asset_kind: AssetKind::Image,
        prompt: "hero shot".to_string(),
        negative_prompt: Some("low quality".to_string()),
        inputs: Vec::new(),
        duration_sec: None,
        aspect_ratio: Some("16:9".to_string()),
        provider_options: Default::default(),
        callback_url: None,
        idempotency_key: Some("abc-123".to_string()),
        metadata: Default::default(),
    };

    let text = serde_json::to_string(&req).expect("serialize request");
    let parsed: GenerateRequest = serde_json::from_str(&text).expect("deserialize request");
    assert_eq!(parsed.model, "openai/gpt-image-1");
    assert_eq!(parsed.asset_kind, AssetKind::Image);
}
