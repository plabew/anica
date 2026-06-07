//! Scene render backends.
//!
//! Owns GPU execution, CPU compatibility rendering, encoding/readback, and
//! backend-specific shader assets. Backend code consumes compiled scene plans;
//! it should not expand DSL semantics directly long term.

pub(crate) mod encoding;
pub(crate) mod gpu;
pub(crate) mod sizing;
