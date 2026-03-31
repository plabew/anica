use crate::error::Result;
use crate::protocol::JobSnapshot;
use async_trait::async_trait;

#[async_trait]
pub trait WebhookCallback: Send + Sync {
    async fn send(&self, callback_url: &str, snapshot: &JobSnapshot) -> Result<()>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopWebhookCallback;

#[async_trait]
impl WebhookCallback for NoopWebhookCallback {
    async fn send(&self, _callback_url: &str, _snapshot: &JobSnapshot) -> Result<()> {
        Ok(())
    }
}
