use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidArgument,
    AuthFailed,
    RateLimited,
    ContentPolicyViolation,
    ProviderUnavailable,
    Timeout,
    JobNotFound,
    JobFailed,
    OutputStoreFailed,
    Internal,
}
