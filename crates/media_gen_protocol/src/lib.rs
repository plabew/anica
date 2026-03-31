#![forbid(unsafe_code)]

pub mod auth;
pub mod error;
pub mod gateway;
pub mod http;
pub mod job;
pub mod model;
pub mod output;
pub mod protocol;
pub mod provider;
pub mod webhook;

pub use error::{ErrorCode, ProtocolError, Result};
pub use gateway::{GatewayContext, GatewayService};
pub use model::{
    AspectRatioResolutionMap, ImageResolutionPreset, ModelResolutionCatalog,
    VideoResolutionConstraint, VideoResolutionConstraintMap, model_resolution_catalog,
    model_resolution_catalog_json,
};
pub use protocol::{
    AssetKind, GenerateAccepted, GenerateRequest, GenerateResult, InputAsset, InputAssetKind,
    JobSnapshot, JobStatus, OutputAsset, Usage,
};
