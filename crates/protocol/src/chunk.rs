//! Class-S chunk addresses. Both peers use the same integer geometry.
use sim_core::limits::{AOI_ENTER_CM, AOI_EXIT_CM};
use sim_core::space::{SPACE_BUCKET_M, SPACE_SIDE};

/// Sixteen broadphase buckets form one 64 m network chunk.
pub const BUCKETS_PER_SIDE: usize = 16;
pub const SIDE_CM: i64 = SPACE_BUCKET_M as i64 * 100 * BUCKETS_PER_SIDE as i64;
pub const SIDE: usize = SPACE_SIDE.div_ceil(BUCKETS_PER_SIDE);
pub const COUNT: usize = SIDE * SIDE;
const _: () = assert!(COUNT <= u16::MAX as usize);
/// A circle's enclosing square, including the two partial edge cells.
pub const VISIBLE_CAP: usize = ((2 * AOI_EXIT_CM / SIDE_CM + 2) as usize).pow(2);

pub fn at_cm(x: i64, z: i64) -> Option<u16> {
    let (x, z) = (x.div_euclid(SIDE_CM), z.div_euclid(SIDE_CM));
    if x >= 0 && z >= 0 && x < SIDE as i64 && z < SIDE as i64 {
        Some((z * SIDE as i64 + x) as u16)
    } else {
        None
    }
}

pub fn at_cell(x: u16, z: u16) -> Option<u16> {
    at_cm(i64::from(x) * 300 + 150, i64::from(z) * 300 + 150)
}

/// Distance to the chunk's closed bounds. Entering subscribes before the
/// nearest structure can be inside the shared AOI radius.
pub fn in_range(id: u16, here: (i64, i64), subscribed: bool) -> bool {
    if usize::from(id) >= COUNT {
        return false;
    }
    let x = i64::from(id) % SIDE as i64 * SIDE_CM;
    let z = i64::from(id) / SIDE as i64 * SIDE_CM;
    let dx = here.0.saturating_sub(here.0.clamp(x, x + SIDE_CM));
    let dz = here.1.saturating_sub(here.1.clamp(z, z + SIDE_CM));
    let radius = if subscribed {
        AOI_EXIT_CM
    } else {
        AOI_ENTER_CM
    };
    dx.unsigned_abs() <= radius as u64
        && dz.unsigned_abs() <= radius as u64
        && dx * dx + dz * dz <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_and_hysteresis_share_the_aoi_units() {
        assert_eq!(at_cm(-1, 0), None);
        assert_eq!(at_cm(i64::MAX, i64::MIN), None);
        assert!(!in_range(0, (i64::MIN, i64::MAX), false));
        assert_eq!(at_cm(6399, 6400), Some(SIDE as u16));
        assert_eq!(at_cm(SIDE as i64 * SIDE_CM, 0), None);
        let at = (SIDE_CM + AOI_ENTER_CM + 1, 100);
        assert!(!in_range(0, at, false));
        assert!(in_range(0, at, true));
        assert!(!in_range(0, (SIDE_CM + AOI_EXIT_CM + 1, 100), true));
    }
}
