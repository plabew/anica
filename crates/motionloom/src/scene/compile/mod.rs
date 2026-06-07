//! Scene validation and lowering.
//!
//! Owns semantic validation, compatibility checks, and lowering from parsed DSL
//! nodes into render plans. Compile code should detect invalid authoring shapes
//! before backend execution where possible.

mod compat;

pub(crate) use compat::{
    graph_has_rich_scene_tree, scene_nodes_contain_image_or_svg, scene_nodes_for_present,
    scene_nodes_require_cpu_scene_compositing,
};
