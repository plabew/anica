// =========================================
// =========================================
// crates/motionloom/src/keyframe.rs

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScalarKeyframe {
    pub time: Duration,
    pub value: f32,
}

pub fn index_at(keys: &[ScalarKeyframe], t: Duration, epsilon: Duration) -> Option<usize> {
    keys.iter().position(|k| {
        if k.time >= t {
            k.time - t <= epsilon
        } else {
            t - k.time <= epsilon
        }
    })
}

pub fn set_or_insert(keys: &mut Vec<ScalarKeyframe>, t: Duration, value: f32, epsilon: Duration) {
    if let Some(idx) = index_at(keys, t, epsilon) {
        keys[idx].value = value;
    } else {
        keys.push(ScalarKeyframe { time: t, value });
        keys.sort_by_key(|k| k.time);
    }
}

pub fn sample_linear(keys: &[ScalarKeyframe], t: Duration, fallback: f32) -> f32 {
    if keys.is_empty() {
        return fallback;
    }

    let first = &keys[0];
    if t <= first.time {
        return first.value;
    }

    let last = keys.last().expect("keys is not empty");
    if t >= last.time {
        return last.value;
    }

    for window in keys.windows(2) {
        let a = &window[0];
        let b = &window[1];
        if t >= a.time && t <= b.time {
            let span = (b.time - a.time).as_secs_f32().max(0.0001);
            let local = (t - a.time).as_secs_f32();
            let frac = (local / span).clamp(0.0, 1.0);
            return a.value + (b.value - a.value) * frac;
        }
    }

    fallback
}
