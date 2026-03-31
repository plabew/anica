#[derive(Debug, Clone, Copy)]
pub struct SchedulerConfig {
    pub worker_count: usize,
    pub max_inflight_jobs: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            worker_count: 4,
            max_inflight_jobs: 128,
        }
    }
}
