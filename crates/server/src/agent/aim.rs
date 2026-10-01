//! Where to point a weapon whose round flies: the arc an arrow needs to
//! reach a spot, and where a moving body will be by the time it gets there.
//!
//! An arrow is not lag-compensated (`sim_core::ranged`): it flies against
//! the present, so the body it is meant for keeps walking while my input
//! crosses the wire and while the arrow is in the air. A player leads by
//! eye; this leads by the track's velocity over the same span. A bullet is
//! judged against the pose my screen showed and needs neither.
//!
//! The flight is the sim's own integer one, read in metres: each tick the
//! round first loses `drop` of its climb, then moves by its velocity. So
//! after `n` ticks it has gone `n·v·cos θ` out and `n·v·sin θ − drop·n(n+1)/2`
//! up. Speed and drop come from the wiki (`Page::ranged`).

use sim_core::limits::TICK_HZ;

/// Steeper than this is a lob no one looses at a body: out of range.
pub const MAX_ARC_DEG: f32 = 35.0;
/// Rounds of the fixed point: the flight time barely moves the pitch, so
/// a handful settles it to well under a wire pitch step.
const ITERATIONS: usize = 6;

/// The pitch (radians, up positive) that carries a round from the eye to
/// a point `h` metres out and `dy` metres up, and the ticks it flies
/// there. `None` past [`MAX_ARC_DEG`] or for a round with no speed.
pub fn arc(h: f32, dy: f32, speed_mmpt: u16, drop_mmpt2: u16) -> Option<(f32, f32)> {
    let v = f32::from(speed_mmpt) / 1000.0;
    let g = f32::from(drop_mmpt2) / 1000.0;
    if v <= 0.0 || h <= 0.0 {
        return None;
    }
    let mut theta = dy.atan2(h);
    let mut n = 0.0;
    for _ in 0..ITERATIONS {
        n = h / (v * theta.cos());
        let sag = g * n * (n + 1.0) * 0.5;
        theta = ((dy + sag) / h).atan();
    }
    (theta.to_degrees() <= MAX_ARC_DEG && n.is_finite()).then_some((theta, n))
}

/// The point to look at so that a round loosed `lead_ticks` from now hits
/// a body that stands at `at` (its aim point, metres) moving at `vel`
/// (metres per second): its place when the round arrives, raised by the
/// arc. With the flight time it needs.
pub fn lead(
    eye: [f32; 3],
    at: [f32; 3],
    vel: [f32; 3],
    lead_ticks: f32,
    speed_mmpt: u16,
    drop_mmpt2: u16,
) -> Option<([f32; 3], f32)> {
    let mut flight = 0.0;
    let mut out = None;
    for _ in 0..3 {
        let k = (lead_ticks + flight) / TICK_HZ as f32;
        let to = [at[0] + vel[0] * k, at[1], at[2] + vel[2] * k];
        let (dx, dz) = (to[0] - eye[0], to[2] - eye[2]);
        let h = dx.hypot(dz);
        let (theta, n) = arc(h, to[1] - eye[1], speed_mmpt, drop_mmpt2)?;
        flight = n;
        out = Some(([to[0], eye[1] + h * theta.tan(), to[2]], n));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sim's own flight, tick by tick, for a pitch: where the round is
    /// once it has gone `h` out.
    fn fly(theta: f32, h: f32, speed: u16, drop: u16) -> f32 {
        let (v, g) = (f32::from(speed) / 1000.0, f32::from(drop) / 1000.0);
        let (mut x, mut y, mut vy) = (0.0f32, 0.0f32, v * theta.sin());
        let vx = v * theta.cos();
        while x + vx < h {
            vy -= g;
            x += vx;
            y += vy;
        }
        // The last part-tick, along the segment the sim samples.
        let f = (h - x) / vx;
        y + (vy - g) * f
    }

    /// A wooden arrow (40 m/s, 20 m/s² at 30 Hz) solved for a body 20 m
    /// out and level, and 30 m out and 2 m below, lands within a hand of
    /// where it was meant to, flown the way the sim flies it.
    #[test]
    fn the_arc_carries_the_round_to_the_mark() {
        let (speed, drop) = (1333, 22);
        for (h, dy) in [(20.0, 0.0), (30.0, -2.0), (8.0, 0.5)] {
            let (theta, n) = arc(h, dy, speed, drop).unwrap();
            assert!(theta > dy.atan2(h), "it aims above the mark");
            assert!((n - h / 1.333).abs() < 1.0, "{n}");
            let y = fly(theta, h, speed, drop);
            assert!((y - dy).abs() < 0.15, "{h} m: landed at {y}, wanted {dy}");
        }
        assert!(arc(200.0, 0.0, speed, drop).is_none(), "out of range");
    }

    /// A body walking across the line is led by where it will be when the
    /// arrow arrives, not where it stands.
    #[test]
    fn a_walking_body_is_led() {
        let eye = [0.0, 1.6, 0.0];
        let at = [0.0, 1.2, 20.0];
        let vel = [3.0, 0.0, 0.0];
        let (p, n) = lead(eye, at, vel, 4.0, 1333, 22).unwrap();
        let want = 3.0 * (4.0 + n) / 30.0;
        assert!((p[0] - want).abs() < 0.05, "{p:?} {want}");
        assert!(p[1] > 1.6, "raised by the arc");
        let (still, _) = lead(eye, at, [0.0; 3], 4.0, 1333, 22).unwrap();
        assert_eq!(still[0], 0.0);
    }
}
