#[derive(Debug, Clone, Copy)]
pub struct HttpClientConfig {
    pub timeout_ms: u64,
    pub connect_timeout_ms: u64,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 60_000,
            connect_timeout_ms: 10_000,
        }
    }
}
