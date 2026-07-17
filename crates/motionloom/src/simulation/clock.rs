// =========================================
// =========================================
// crates/motionloom/src/simulation/clock.rs

#[derive(Debug, Clone, Copy)]
pub struct SimulationClock {
    pub fps: f32,
    pub frame: u32,
    pub duration_seconds: f32,
}

impl SimulationClock {
    pub fn fixed_dt(self) -> f32 {
        1.0 / self.fps.max(1.0)
    }
    pub fn time_seconds(self) -> f32 {
        self.frame as f32 * self.fixed_dt()
    }

    pub fn time_norm(self) -> f32 {
        (self.time_seconds() / self.duration_seconds.max(self.fixed_dt())).clamp(0.0, 1.0)
    }
}
