use crate::error::{ErrorCode, ProtocolError, Result};
use crate::gateway::GatewayContext;
use crate::job::JobStore;
use crate::model::ModelRouteKey;
use crate::model::{
    ModelResolutionCatalog, model_resolution_catalog, model_resolution_catalog_json,
};
use crate::protocol::{GenerateAccepted, GenerateRequest, JobSnapshot, JobStatus};
use crate::provider::{ProviderAdapter, ProviderPollResult, ProviderSubmitResult};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct GatewayService {
    context: GatewayContext,
    adapters: HashMap<String, Arc<dyn ProviderAdapter>>,
    job_store: Arc<dyn JobStore>,
    seq: AtomicU64,
}

impl GatewayService {
    pub fn new(context: GatewayContext, job_store: Arc<dyn JobStore>) -> Self {
        Self {
            context,
            adapters: HashMap::new(),
            job_store,
            seq: AtomicU64::new(1),
        }
    }

    pub fn register_adapter(&mut self, adapter: Arc<dyn ProviderAdapter>) {
        self.adapters
            .insert(adapter.provider().to_string(), adapter);
    }

    /// Return static resolution presets by asset kind/model for UI dropdown filtering.
    pub fn model_resolution_catalog(&self) -> ModelResolutionCatalog {
        model_resolution_catalog()
    }

    /// Return static resolution presets as JSON for UI consumption.
    pub fn model_resolution_catalog_json(&self) -> Value {
        model_resolution_catalog_json()
    }

    pub async fn submit(&self, request: GenerateRequest) -> Result<GenerateAccepted> {
        request.validate()?;

        let route = request.route_key()?;
        let adapter = self.adapter_for(&route.provider)?;

        let created_at_ms = now_ms();
        let job_id = format!(
            "job_{}_{}",
            created_at_ms,
            self.seq.fetch_add(1, Ordering::Relaxed)
        );

        let mut snapshot = JobSnapshot::new(job_id.clone(), request.model.clone(), created_at_ms);
        self.job_store.upsert(snapshot.clone()).await?;

        match adapter.submit(&self.context, &request).await {
            Ok(provider_res) => {
                apply_submit_result(&mut snapshot, provider_res);
            }
            Err(err) => {
                snapshot.status = JobStatus::Failed;
                snapshot.error = Some(err);
                snapshot.finished_at_ms = Some(now_ms());
            }
        }

        self.job_store.upsert(snapshot.clone()).await?;

        Ok(GenerateAccepted {
            job_id,
            status: snapshot.status,
            created_at_ms,
        })
    }

    pub async fn poll(&self, job_id: &str) -> Result<JobSnapshot> {
        let mut snapshot = self.require_job(job_id).await?;
        if snapshot.status.is_terminal() {
            return Ok(snapshot);
        }

        let route = ModelRouteKey::parse(&snapshot.model)?;
        let adapter = self.adapter_for(&route.provider)?;

        let Some(provider_job_id) = snapshot.provider_job_id.clone() else {
            return Ok(snapshot);
        };

        match adapter
            .poll(&self.context, &snapshot.model, &provider_job_id)
            .await
        {
            Ok(provider_res) => {
                apply_poll_result(&mut snapshot, provider_res);
            }
            Err(err) => {
                snapshot.status = JobStatus::Failed;
                snapshot.error = Some(err);
                snapshot.finished_at_ms = Some(now_ms());
            }
        }

        self.job_store.upsert(snapshot.clone()).await?;
        Ok(snapshot)
    }

    pub async fn cancel(&self, job_id: &str) -> Result<JobSnapshot> {
        let mut snapshot = self.require_job(job_id).await?;
        if snapshot.status.is_terminal() {
            return Ok(snapshot);
        }

        let route = ModelRouteKey::parse(&snapshot.model)?;
        let adapter = self.adapter_for(&route.provider)?;

        if let Some(provider_job_id) = snapshot.provider_job_id.as_deref() {
            let _ = adapter
                .cancel(&self.context, &snapshot.model, provider_job_id)
                .await;
        }

        snapshot.status = JobStatus::Canceled;
        snapshot.finished_at_ms = Some(now_ms());
        self.job_store.upsert(snapshot.clone()).await?;
        Ok(snapshot)
    }

    async fn require_job(&self, job_id: &str) -> Result<JobSnapshot> {
        self.job_store.get(job_id).await?.ok_or_else(|| {
            ProtocolError::new(ErrorCode::JobNotFound, format!("unknown job_id: {job_id}"))
        })
    }

    fn adapter_for(&self, provider: &str) -> Result<&Arc<dyn ProviderAdapter>> {
        self.adapters.get(provider).ok_or_else(|| {
            ProtocolError::new(
                ErrorCode::ProviderUnavailable,
                format!("no adapter registered for provider '{provider}'"),
            )
        })
    }
}

fn apply_submit_result(snapshot: &mut JobSnapshot, provider_res: ProviderSubmitResult) {
    snapshot.status = provider_res.status;
    snapshot.provider_job_id = provider_res.provider_job_id;
    snapshot.result = provider_res.result;
    snapshot.error = provider_res.error;

    if matches!(snapshot.status, JobStatus::Running | JobStatus::Succeeded) {
        snapshot.started_at_ms.get_or_insert_with(now_ms);
    }
    if snapshot.status.is_terminal() {
        snapshot.finished_at_ms = Some(now_ms());
    }
}

fn apply_poll_result(snapshot: &mut JobSnapshot, provider_res: ProviderPollResult) {
    snapshot.status = provider_res.status;
    snapshot.result = provider_res.result;
    snapshot.error = provider_res.error;

    if matches!(snapshot.status, JobStatus::Running | JobStatus::Succeeded) {
        snapshot.started_at_ms.get_or_insert_with(now_ms);
    }
    if snapshot.status.is_terminal() {
        snapshot.finished_at_ms = Some(now_ms());
    }
}

fn now_ms() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as u64,
        Err(_) => 0,
    }
}
