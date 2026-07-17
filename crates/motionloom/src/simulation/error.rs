// =========================================
// =========================================
// crates/motionloom/src/simulation/error.rs

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SimulationError {
    #[error("simulation resource '{id}' was not found")]
    MissingResource { id: String },
    #[error("simulation target '{id}' was not found")]
    MissingTarget { id: String },
    #[error("invalid simulation value for '{field}': {value}")]
    InvalidValue { field: &'static str, value: String },
    #[error("invalid simulation points '{value}'")]
    InvalidPoints { value: String },
    #[error("simulation feature '{feature}' is not implemented by this runtime")]
    Unsupported { feature: &'static str },
}
