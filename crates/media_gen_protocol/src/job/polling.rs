#[derive(Debug, Clone, Copy)]
pub struct PollingPolicy {
    pub interval_ms: u64,
    pub timeout_ms: u64,
}

impl Default for PollingPolicy {
    fn default() -> Self {
        Self {
            interval_ms: 1_000,
            timeout_ms: 120_000,
        }
    }
}
