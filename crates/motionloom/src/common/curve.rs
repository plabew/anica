// =========================================
// =========================================
// crates/motionloom/src/eval.rs

use crate::model::AnimF32;

pub fn sample_anim_f32(anim: &AnimF32, frame: u32) -> f32 {
    match anim {
        AnimF32::Const(v) => *v,
        AnimF32::Linear {
            from,
            to,
            start_frame,
            end_frame,
        } => {
            if end_frame <= start_frame {
                return *to;
            }
            let t =
                (frame.saturating_sub(*start_frame)) as f32 / (*end_frame - *start_frame) as f32;
            from + (to - from) * t.clamp(0.0, 1.0)
        }
        AnimF32::Keyframes(points) => {
            if points.is_empty() {
                return 1.0;
            }
            if points.len() == 1 {
                return points[0].1;
            }

            let mut prev = points[0];
            if frame <= prev.0 {
                return prev.1;
            }

            for next in points.iter().copied().skip(1) {
                if frame <= next.0 {
                    if next.0 <= prev.0 {
                        return next.1;
                    }
                    let span = (next.0 - prev.0) as f32;
                    let local = (frame - prev.0) as f32;
                    let t = (local / span).clamp(0.0, 1.0);
                    return prev.1 + (next.1 - prev.1) * t;
                }
                prev = next;
            }

            prev.1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sample_anim_f32;
    use crate::model::AnimF32;

    #[test]
    fn linear_sampling_works() {
        let v = sample_anim_f32(
            &AnimF32::Linear {
                from: 1.0,
                to: 2.0,
                start_frame: 0,
                end_frame: 100,
            },
            25,
        );
        assert!((v - 1.25).abs() < 0.001);
    }

    #[test]
    fn keyframe_sampling_interpolates() {
        let v = sample_anim_f32(&AnimF32::Keyframes(vec![(0, 1.0), (10, 2.0)]), 5);
        assert!((v - 1.5).abs() < 0.001);
    }
}
