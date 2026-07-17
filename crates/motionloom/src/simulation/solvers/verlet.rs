// =========================================
// =========================================
// crates/motionloom/src/simulation/solvers/verlet.rs

use crate::simulation::constraints::distance::solve_distance;
use crate::simulation::model::ColliderNode;
use crate::simulation::state::DynamicCurveState;

pub fn step(
    state: &mut DynamicCurveState,
    acceleration: impl Fn(usize) -> [f32; 2],
    dt: f32,
    damping: f32,
    stiffness: f32,
    colliders: &[ColliderNode],
    collision_radius: f32,
) {
    let dt2 = dt * dt;
    for (index, particle) in state.particles.iter_mut().enumerate() {
        if particle.pinned {
            continue;
        }
        let velocity = [
            (particle.position[0] - particle.previous[0]) * (1.0 - damping.clamp(0.0, 0.999)),
            (particle.position[1] - particle.previous[1]) * (1.0 - damping.clamp(0.0, 0.999)),
        ];
        let previous = particle.position;
        let force = acceleration(index);
        particle.position[0] += velocity[0] + force[0] * dt2;
        particle.position[1] += velocity[1] + force[1] * dt2;
        particle.previous = previous;
    }
    for _ in 0..6 {
        for index in 0..state.rest_lengths.len() {
            let (left, right) = state.particles.split_at_mut(index + 1);
            solve_distance(
                &mut left[index],
                &mut right[0],
                state.rest_lengths[index],
                stiffness,
            );
        }
        for particle in &mut state.particles {
            if !particle.pinned {
                for collider in colliders {
                    crate::simulation::collision::shapes::project_out(
                        &mut particle.position,
                        collider,
                        collision_radius,
                    );
                }
            }
        }
    }
}
