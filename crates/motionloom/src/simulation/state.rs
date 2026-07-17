// =========================================
// =========================================
// crates/motionloom/src/simulation/state.rs

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleState {
    pub position: [f32; 2],
    pub previous: [f32; 2],
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DynamicCurveState {
    pub particles: Vec<ParticleState>,
    pub rest_lengths: Vec<f32>,
}
