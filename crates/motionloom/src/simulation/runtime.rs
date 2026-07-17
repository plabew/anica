// =========================================
// =========================================
// crates/motionloom/src/simulation/runtime.rs

use crate::simulation::bodies::dynamic_curve::{build_dynamic_curve, resample_polyline};
use crate::simulation::clock::SimulationClock;
use crate::simulation::model::{AttractionNode, ColliderNode, SpringChainNode, WindNode};
use crate::simulation::solvers::verlet;
use crate::simulation::state::DynamicCurveState;

pub fn simulate_spring_chain(
    points: &[[f32; 2]],
    binding: &SpringChainNode,
    wind: Option<&WindNode>,
    attraction: Option<&AttractionNode>,
    colliders: &[ColliderNode],
    clock: SimulationClock,
) -> DynamicCurveState {
    let sampled = resample_polyline(points, binding.segments.max(1));
    let mut state = build_dynamic_curve(&sampled, &binding.pin);
    let dt = clock.fixed_dt();
    for frame in 0..clock.frame {
        let time = frame as f32 * dt;
        let positions = state
            .particles
            .iter()
            .map(|particle| particle.position)
            .collect::<Vec<_>>();
        verlet::step(
            &mut state,
            |index| {
                let mut force = binding.gravity;
                if let Some(wind) = wind {
                    let value = crate::simulation::forces::wind_acceleration(wind, time, index);
                    force[0] += value[0];
                    force[1] += value[1];
                }
                if let Some(attraction) = attraction {
                    let position = positions[index];
                    let delta = [
                        attraction.point[0] - position[0],
                        attraction.point[1] - position[1],
                    ];
                    let length = (delta[0] * delta[0] + delta[1] * delta[1])
                        .sqrt()
                        .max(0.000_1);
                    if length <= attraction.radius {
                        force[0] += delta[0] / length * attraction.strength;
                        force[1] += delta[1] / length * attraction.strength;
                    }
                }
                force
            },
            dt,
            binding.damping,
            binding.stiffness,
            colliders,
            binding.collision_radius,
        );
    }
    state
}
