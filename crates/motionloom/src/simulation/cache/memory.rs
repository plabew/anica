// =========================================
// =========================================
// crates/motionloom/src/simulation/cache/memory.rs

use crate::simulation::state::DynamicCurveState;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct SimulationMemoryCache {
    frames: HashMap<(String, u32), DynamicCurveState>,
}

impl SimulationMemoryCache {
    pub fn get(&self, target: &str, frame: u32) -> Option<&DynamicCurveState> {
        self.frames.get(&(target.to_string(), frame))
    }
    pub fn insert(&mut self, target: &str, frame: u32, state: DynamicCurveState) {
        self.frames.insert((target.to_string(), frame), state);
    }
    pub fn clear(&mut self) {
        self.frames.clear();
    }
}
