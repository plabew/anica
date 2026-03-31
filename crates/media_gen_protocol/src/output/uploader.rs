use crate::error::{ErrorCode, ProtocolError, Result};
use async_trait::async_trait;

#[async_trait]
pub trait OutputUploader: Send + Sync {
    async fn upload_bytes(&self, content_type: &str, bytes: &[u8]) -> Result<String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopOutputUploader;

#[async_trait]
impl OutputUploader for NoopOutputUploader {
    async fn upload_bytes(&self, _content_type: &str, _bytes: &[u8]) -> Result<String> {
        Err(ProtocolError::new(
            ErrorCode::OutputStoreFailed,
            "no output uploader configured",
        ))
    }
}
