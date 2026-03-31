pub mod bfl_flux;
pub mod google_genai;
pub mod luma;
pub mod openai;
pub mod runway;
pub mod stability;

use crate::error::{ErrorCode, ProtocolError, Result};
use crate::gateway::GatewayContext;
use crate::protocol::{GenerateRequest, GenerateResult, JobStatus};
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderSubmitResult {
    pub status: JobStatus,
    pub provider_job_id: Option<String>,
    pub result: Option<GenerateResult>,
    pub error: Option<ProtocolError>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderPollResult {
    pub status: JobStatus,
    pub result: Option<GenerateResult>,
    pub error: Option<ProtocolError>,
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn provider(&self) -> &'static str;

    async fn submit(
        &self,
        ctx: &GatewayContext,
        request: &GenerateRequest,
    ) -> Result<ProviderSubmitResult>;

    async fn poll(
        &self,
        ctx: &GatewayContext,
        model: &str,
        provider_job_id: &str,
    ) -> Result<ProviderPollResult>;

    async fn cancel(&self, ctx: &GatewayContext, model: &str, provider_job_id: &str) -> Result<()>;
}

pub fn unsupported_provider(provider: &str) -> ProtocolError {
    ProtocolError::new(
        ErrorCode::ProviderUnavailable,
        format!("provider adapter '{provider}' is not implemented yet"),
    )
}
