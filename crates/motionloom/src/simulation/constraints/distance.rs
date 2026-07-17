// =========================================
// =========================================
// crates/motionloom/src/simulation/constraints/distance.rs

use crate::simulation::state::ParticleState;

pub fn solve_distance(a: &mut ParticleState, b: &mut ParticleState, rest: f32, stiffness: f32) {
    let dx = b.position[0] - a.position[0];
    let dy = b.position[1] - a.position[1];
    let length = (dx * dx + dy * dy).sqrt().max(0.000_1);
    let correction = (length - rest) / length * stiffness.clamp(0.0, 1.0);
    let shift = [dx * correction, dy * correction];
    match (a.pinned, b.pinned) {
        (true, false) => {
            b.position[0] -= shift[0];
            b.position[1] -= shift[1];
        }
        (false, true) => {
            a.position[0] += shift[0];
            a.position[1] += shift[1];
        }
        (false, false) => {
            a.position[0] += shift[0] * 0.5;
            a.position[1] += shift[1] * 0.5;
            b.position[0] -= shift[0] * 0.5;
            b.position[1] -= shift[1] * 0.5;
        }
        (true, true) => {}
    }
}
