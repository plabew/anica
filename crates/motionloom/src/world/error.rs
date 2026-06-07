use crate::error::GraphParseError;
use crate::world::gltf_loader::GlbLoadError;
use crate::world::render::WorldRenderError as WorldRenderFailure;

#[derive(Debug, thiserror::Error)]
pub enum MotionLoomWorldError {
    #[error(transparent)]
    Parse(#[from] GraphParseError),
    #[error(transparent)]
    Render(#[from] WorldRenderFailure),
    #[error(transparent)]
    Asset(#[from] GlbLoadError),
}

pub type WorldError = MotionLoomWorldError;
pub type WorldParseError = GraphParseError;
pub type WorldAssetError = GlbLoadError;
