//! High-level Scene authoring domains.
//!
//! Owns author-friendly concepts that lower into core Resource, Timeline,
//! Composition, Spatial, and Drawable nodes, such as Character, Part, Skeleton2D,
//! and future scene-specific actors.

mod action;
pub(crate) use action::apply_action_graph_at_time;
