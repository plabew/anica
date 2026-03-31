use crate::error::Result;
use crate::gateway::GatewayContext;
use crate::protocol::GenerateRequest;
use crate::provider::{
    ProviderAdapter, ProviderPollResult, ProviderSubmitResult, unsupported_provider,
};
use async_trait::async_trait;

#[derive(Debug, Clone, Copy, Default)]
pub struct RunwayAdapter;

#[async_trait]
impl ProviderAdapter for RunwayAdapter {
    fn provider(&self) -> &'static str {
        "runway"
    }

    async fn submit(
        &self,
        _ctx: &GatewayContext,
        _request: &GenerateRequest,
    ) -> Result<ProviderSubmitResult> {
        Err(unsupported_provider(self.provider()))
    }

    async fn poll(
        &self,
        _ctx: &GatewayContext,
        _model: &str,
        _provider_job_id: &str,
    ) -> Result<ProviderPollResult> {
        Err(unsupported_provider(self.provider()))
    }

    async fn cancel(
        &self,
        _ctx: &GatewayContext,
        _model: &str,
        _provider_job_id: &str,
    ) -> Result<()> {
        Err(unsupported_provider(self.provider()))
    }
}
