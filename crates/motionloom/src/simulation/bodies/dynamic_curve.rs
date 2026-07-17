// =========================================
// =========================================
// crates/motionloom/src/simulation/bodies/dynamic_curve.rs

use crate::simulation::state::{DynamicCurveState, ParticleState};

pub fn build_dynamic_curve(points: &[[f32; 2]], pin: &str) -> DynamicCurveState {
    let particles = points
        .iter()
        .enumerate()
        .map(|(index, point)| ParticleState {
            position: *point,
            previous: *point,
            pinned: (pin == "start" && index == 0)
                || (pin == "end" && index + 1 == points.len())
                || pin == "both" && (index == 0 || index + 1 == points.len()),
        })
        .collect();
    let rest_lengths = points
        .windows(2)
        .map(|pair| {
            let dx = pair[1][0] - pair[0][0];
            let dy = pair[1][1] - pair[0][1];
            (dx * dx + dy * dy).sqrt()
        })
        .collect();
    DynamicCurveState {
        particles,
        rest_lengths,
    }
}

pub fn resample_polyline(points: &[[f32; 2]], segments: usize) -> Vec<[f32; 2]> {
    if points.len() < 2 || segments < 1 {
        return points.to_vec();
    }
    let lengths: Vec<f32> = points
        .windows(2)
        .map(|pair| {
            let dx = pair[1][0] - pair[0][0];
            let dy = pair[1][1] - pair[0][1];
            (dx * dx + dy * dy).sqrt()
        })
        .collect();
    let total: f32 = lengths.iter().sum();
    if total <= 0.000_1 {
        return vec![points[0]; segments + 1];
    }
    (0..=segments)
        .map(|sample| {
            let target = total * sample as f32 / segments as f32;
            let mut traversed = 0.0;
            for (index, length) in lengths.iter().enumerate() {
                if traversed + length >= target || index + 1 == lengths.len() {
                    let t = ((target - traversed) / length.max(0.000_1)).clamp(0.0, 1.0);
                    return [
                        points[index][0] + (points[index + 1][0] - points[index][0]) * t,
                        points[index][1] + (points[index + 1][1] - points[index][1]) * t,
                    ];
                }
                traversed += length;
            }
            *points.last().unwrap_or(&points[0])
        })
        .collect()
}
