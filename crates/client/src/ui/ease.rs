//! The interface's motion: the curves a panel arriving, a bar draining and a
//! cell popping are drawn along. Pure, so they are tested headless.

/// Fast out, slow in: `t` in `0..=1` to `0..=1`.
pub fn out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// Close `cur` on `target` at `rate` per second (an exponential approach,
/// frame-rate independent), snapping the last hair so it settles.
pub fn approach(cur: f32, target: f32, rate: f32, dt: f32) -> f32 {
    let k = 1.0 - (-rate * dt.max(0.0)).exp();
    let next = cur + (target - cur) * k;
    if (next - target).abs() < 1e-3 {
        target
    } else {
        next
    }
}

/// A pop: up from 1 to `1 + amp` and back over `t` in `0..=1`, a sine hump
/// with a fast rise.
pub fn pop(t: f32, amp: f32) -> f32 {
    if !(0.0..1.0).contains(&t) {
        return 1.0;
    }
    let s = (t.sqrt() * std::f32::consts::PI).sin();
    1.0 + amp * s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curves_start_and_end_where_they_say() {
        assert_eq!(out_cubic(0.0), 0.0);
        assert_eq!(out_cubic(1.0), 1.0);
        assert!(out_cubic(0.5) > 0.5, "fast out");
        assert_eq!(pop(0.0, 0.1), 1.0);
        assert_eq!(pop(1.0, 0.1), 1.0);
        assert!(pop(0.25, 0.1) > 1.05);
    }

    /// The approach settles on its target whatever the frame rate, and in
    /// about the same time at 30 and at 144 frames a second.
    #[test]
    fn the_approach_settles_at_any_frame_rate() {
        for fps in [30.0, 60.0, 144.0] {
            let (mut v, mut t) = (0.0, 0.0);
            while v != 1.0 && t < 5.0 {
                v = approach(v, 1.0, 12.0, 1.0 / fps);
                t += 1.0 / fps;
            }
            assert_eq!(v, 1.0, "never settled at {fps} fps");
            assert!((0.4..0.8).contains(&t), "{t} s at {fps} fps");
        }
    }
}
