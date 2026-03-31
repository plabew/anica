use crate::error::Result;
use crate::protocol::JobSnapshot;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::RwLock;

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn upsert(&self, snapshot: JobSnapshot) -> Result<()>;
    async fn get(&self, job_id: &str) -> Result<Option<JobSnapshot>>;
}

#[derive(Debug, Default)]
pub struct InMemoryJobStore {
    jobs: RwLock<HashMap<String, JobSnapshot>>,
}

impl InMemoryJobStore {
    pub fn new() -> Self {
        Self {
            jobs: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl JobStore for InMemoryJobStore {
    async fn upsert(&self, snapshot: JobSnapshot) -> Result<()> {
        let mut lock = self.jobs.write().expect("job store poisoned");
        lock.insert(snapshot.job_id.clone(), snapshot);
        Ok(())
    }

    async fn get(&self, job_id: &str) -> Result<Option<JobSnapshot>> {
        let lock = self.jobs.read().expect("job store poisoned");
        Ok(lock.get(job_id).cloned())
    }
}
