// =========================================
// =========================================
// crates/motionloom/src/simulation/constraints/pin.rs

use crate::simulation::state::ParticleState;

pub fn apply_pin(particle: &mut ParticleState, anchor: [f32; 2]) {
    if particle.pinned {
        particle.position = anchor;
        particle.previous = anchor;
    }
}
