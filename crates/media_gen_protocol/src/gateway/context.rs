use crate::auth::KeyResolver;
use crate::model::ModelRegistry;
use crate::output::OutputUploader;
use std::sync::Arc;

#[derive(Clone)]
pub struct GatewayContext {
    pub model_registry: Arc<ModelRegistry>,
    pub key_resolver: Arc<dyn KeyResolver>,
    pub output_uploader: Arc<dyn OutputUploader>,
}

impl GatewayContext {
    pub fn new(
        model_registry: Arc<ModelRegistry>,
        key_resolver: Arc<dyn KeyResolver>,
        output_uploader: Arc<dyn OutputUploader>,
    ) -> Self {
        Self {
            model_registry,
            key_resolver,
            output_uploader,
        }
    }
}
