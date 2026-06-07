//! Scene spatial model.
//!
//! Owns transforms, coordinate spaces, Camera2D, bounds, layout, anchors,
//! constraints, and world/screen mapping. Spatial code maps coordinates; it
//! should not perform image processing.

mod camera;
mod deform;
mod transform;

pub(crate) use camera::*;
pub(crate) use deform::*;
pub(crate) use transform::*;
