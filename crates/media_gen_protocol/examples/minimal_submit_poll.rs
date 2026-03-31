use media_gen_protocol::auth::StaticKeyResolver;
use media_gen_protocol::gateway::{GatewayContext, GatewayService};
use media_gen_protocol::job::InMemoryJobStore;
use media_gen_protocol::model::{ModelRegistry, ModelSpec};
use media_gen_protocol::output::NoopOutputUploader;
use media_gen_protocol::protocol::{AssetKind, GenerateRequest};
use media_gen_protocol::provider::openai::OpenAiAdapter;
use std::sync::Arc;

fn main() {
    let mut registry = ModelRegistry::new();
    let _ = registry.insert(ModelSpec {
        route_key: "openai/gpt-image-1".to_string(),
        label: "GPT Image 1".to_string(),
        supported_assets: vec![AssetKind::Image],
        enabled: true,
        api_key_slot: Some("openai".to_string()),
    });

    let context = GatewayContext::new(
        Arc::new(registry),
        Arc::new(StaticKeyResolver::new().with_key("openai", "demo-key")),
        Arc::new(NoopOutputUploader),
    );

    let mut service = GatewayService::new(context, Arc::new(InMemoryJobStore::new()));
    service.register_adapter(Arc::new(OpenAiAdapter));

    let _future = service.submit(GenerateRequest {
        model: "openai/gpt-image-1".to_string(),
        asset_kind: AssetKind::Image,
        prompt: "hero close-up".to_string(),
        negative_prompt: Some("blurry".to_string()),
        inputs: Vec::new(),
        duration_sec: None,
        aspect_ratio: Some("16:9".to_string()),
        provider_options: Default::default(),
        callback_url: None,
        idempotency_key: Some("demo-idempotency-key".to_string()),
        metadata: Default::default(),
    });

    println!("created submit future; run in async runtime to execute.");
}
