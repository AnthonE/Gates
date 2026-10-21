//! The compass tape across the top of the screen (Rust's): a strip of
//! bearings that slides as you turn, a tick every 5°, a number every 15°
//! and the eight points by letter. Marks (your last death, your map pins)
//! ride a row over it at their bearing. Pure, so the arithmetic is tested
//! without a window; `render::hud` draws it.

/// How far either side of your heading the tape reaches, degrees. A mark
/// further round than this is held at the end it is nearer.
pub const HALF_SPAN_DEG: f32 = 60.0;
/// A tick every this many degrees.
pub const TICK_DEG: u16 = 5;
/// A label every this many degrees. The eight points fall on it, so a
/// letter replaces a number rather than crowding one.
pub const LABEL_DEG: u16 = 15;

/// `bearing` relative to `facing`, degrees in `-180..180`. Clockwise of
/// your heading is positive, which is to the right on the tape.
pub fn rel_deg(facing: f32, bearing: f32) -> f32 {
    (bearing - facing + 540.0).rem_euclid(360.0) - 180.0
}

/// What the tape prints at `deg`: the point's letters on the eight points,
/// the bearing's number everywhere else.
pub fn label(deg: u16) -> String {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let deg = deg % 360;
    if deg.is_multiple_of(45) {
        POINTS[(deg / 45) as usize].to_string()
    } else {
        deg.to_string()
    }
}

/// How tall a tick stands: 2 on a point, 1 under a number, 0 between.
pub fn tick_rank(deg: u16) -> u8 {
    match deg % 360 {
        d if d.is_multiple_of(45) => 2,
        d if d.is_multiple_of(LABEL_DEG) => 1,
        _ => 0,
    }
}

/// How strongly something `rel` degrees off the heading is drawn: full
/// across the middle, fading out over the tape's last fifth so the strip
/// has no hard ends.
pub fn fade(rel: f32) -> f32 {
    ((1.0 - rel.abs() / HALF_SPAN_DEG) * 5.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_points_are_letters_and_the_rest_are_numbers() {
        assert_eq!(label(0), "N");
        assert_eq!(label(45), "NE");
        assert_eq!(label(180), "S");
        assert_eq!(label(315), "NW");
        assert_eq!(label(360), "N");
        assert_eq!(label(15), "15");
        assert_eq!(label(210), "210");
        assert_eq!(tick_rank(90), 2);
        assert_eq!(tick_rank(105), 1);
        assert_eq!(tick_rank(110), 0);
    }

    #[test]
    fn right_of_the_heading_is_clockwise_and_it_wraps() {
        assert!((rel_deg(0.0, 30.0) - 30.0).abs() < 1e-4);
        assert!((rel_deg(0.0, 330.0) + 30.0).abs() < 1e-4);
        assert!((rel_deg(350.0, 10.0) - 20.0).abs() < 1e-4);
        assert!((rel_deg(10.0, 350.0) + 20.0).abs() < 1e-4);
        assert_eq!(fade(0.0), 1.0);
        assert_eq!(fade(HALF_SPAN_DEG), 0.0);
    }

    /// The tape reads the same bearing the old strip printed: yaw 0 is north,
    /// a quarter turn of yaw is west (`look.rs`'s header), and the smooth
    /// bearing the tape slides on never strays a LUT step from the quantized
    /// one the map and the wire use.
    #[test]
    fn the_tape_walks_the_sims_bearing() {
        use crate::look::{bearing_deg, bearing_smooth_deg};
        use std::f32::consts::{FRAC_PI_2, PI, TAU};
        let centre = |yaw: f32| label(((bearing_smooth_deg(yaw) / 45.0).round() as u16) * 45);
        assert_eq!(centre(0.0), "N");
        assert_eq!(centre(FRAC_PI_2), "W");
        assert_eq!(centre(PI), "S");
        assert_eq!(centre(3.0 * FRAC_PI_2), "E");
        assert_eq!(centre(TAU - 0.001), "N");
        for i in 0..720 {
            let yaw = i as f32 * TAU / 720.0;
            let d = rel_deg(bearing_deg(yaw), bearing_smooth_deg(yaw)).abs();
            assert!(d < 1.41, "yaw {yaw}: off by {d}°");
        }
    }
}
