use media_gen_protocol::model::ModelRouteKey;

#[test]
fn parse_provider_model_route_key() {
    let key = ModelRouteKey::parse("google/nano-banana").expect("route key should parse");
    assert_eq!(key.provider, "google");
    assert_eq!(key.model_name, "nano-banana");
}

#[test]
fn reject_invalid_route_key() {
    assert!(ModelRouteKey::parse("nano-banana").is_err());
}
