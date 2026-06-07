//! Scene drawable primitives and draw-command emission.
//!
//! Owns primitive visual descriptions such as Rect, Circle, Line, Polyline,
//! Path, Image, Svg, PixelGrid, and low-level draw-command conversion. Drawable
//! code is where semantic nodes become renderable geometry or textures.

mod face_jaw;
mod geometry;
mod gpu;
mod paint;
mod path_morph;
mod raster;
mod stroke;

pub(crate) use face_jaw::*;
pub(crate) use geometry::*;
pub(crate) use gpu::*;
pub(crate) use paint::*;
pub(crate) use path_morph::*;
pub(crate) use raster::*;
pub(crate) use stroke::*;
