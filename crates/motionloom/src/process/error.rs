use crate::error::{GraphParseError, RuntimeCompileError};

#[derive(Debug, thiserror::Error)]
pub enum MotionLoomProcessError {
    #[error(transparent)]
    Parse(#[from] GraphParseError),
    #[error(transparent)]
    RuntimeCompile(#[from] RuntimeCompileError),
    #[error("process graph error: {message}")]
    Graph { message: String },
    #[error("process runtime error: {message}")]
    Runtime { message: String },
}

pub type ProcessError = MotionLoomProcessError;
pub type ProcessGraphError = MotionLoomProcessError;
pub type ProcessParseError = GraphParseError;
pub type ProcessRuntimeError = RuntimeCompileError;
