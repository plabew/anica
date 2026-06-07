//! Scene timing graph.
//!
//! Owns existence, sequencing, local-time mapping, playback policy, and
//! timeline containers such as Timeline, Track, Sequence, Chain, Loop, and
//! Freeze. Timeline code should not own pixel rendering.

mod repeat;
mod time;

pub(crate) use repeat::eval_repeat_count;
pub(crate) use time::{scene_layer_source_time, scene_sequence_local_time};
