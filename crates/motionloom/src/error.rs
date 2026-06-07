pub use crate::common::error::{GraphParseError, RuntimeCompileError};

use crate::process::error::MotionLoomProcessError;
use crate::scene::error::MotionLoomSceneRenderError;
use crate::world::WorldRenderError;
use crate::world::error::MotionLoomWorldError;

#[derive(Debug, thiserror::Error)]
pub enum MotionLoomError {
    #[error(transparent)]
    Parse(#[from] GraphParseError),
    #[error(transparent)]
    Process(#[from] MotionLoomProcessError),
    #[error(transparent)]
    Scene(#[from] MotionLoomSceneRenderError),
    #[error(transparent)]
    World(#[from] MotionLoomWorldError),
    #[error(transparent)]
    WorldRender(#[from] WorldRenderError),
    #[error("{message}")]
    UnsupportedDocument { message: String },
}

pub type RootGraphError = MotionLoomError;
