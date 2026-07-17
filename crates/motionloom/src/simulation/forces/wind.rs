// =========================================
// =========================================
// crates/motionloom/src/simulation/forces/wind.rs

use crate::simulation::model::WindNode;

pub fn wind_acceleration(wind: &WindNode, time: f32, index: usize) -> [f32; 2] {
    let phase = time * wind.noise_scale + index as f32 * 0.618_034;
    let modulation = 1.0 + phase.sin() * wind.turbulence;
    [
        wind.direction[0] * wind.strength * modulation,
        wind.direction[1] * wind.strength * modulation,
    ]
}
