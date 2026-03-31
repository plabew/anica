pub mod registry;
pub mod resolution_catalog;
pub mod route_key;

pub use registry::{ModelRegistry, ModelSpec};
pub use resolution_catalog::{
    AspectRatioResolutionMap, ImageResolutionPreset, ModelResolutionCatalog,
    VideoResolutionConstraint, VideoResolutionConstraintMap, model_resolution_catalog,
    model_resolution_catalog_json,
};
pub use route_key::ModelRouteKey;
