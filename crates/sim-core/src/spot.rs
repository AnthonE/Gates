//! Where the arc's things stand (`ARC.md` §4): a work's terminal, and later
//! a speaker, an inscription or a puzzle's device.
//!
//! **One answer for all of them.** A [`Spot`] is a site plus an offset in
//! that site's own frame, so content says "beside the anvil rock nearest the
//! middle of the island" and never a world coordinate. The sites are already
//! a pure function of the seed (`terrain::Haven`), so the server and every
//! client resolve the same place without a byte on the wire.
//!
//! A place on a seed that has no such site resolves to `None`, and whatever
//! stands there is simply absent on that island. A work still lights on its
//! fallback hour (`works.rs`).

use crate::landmark::{self, Landmark, LANDMARKS};
use crate::terrain::{Haven, ISLAND_SIZE};

/// THE GATE (`town.rs`).
pub const SITE_TOWN: u8 = 0;
/// The Black Ziggurat (`monument.rs`).
pub const SITE_ZIGGURAT: u8 = 1;
/// A landmark: this plus its `landmark::LandmarkKind`.
pub const SITE_LANDMARK0: u8 = 8;
/// The highest site code.
pub const SITE_MAX: u8 = SITE_LANDMARK0 + 7;

/// A site and an offset in its frame, centimetres. `nth` picks among
/// several of one landmark kind: 0 is the one nearest the island's middle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spot {
    pub site: u8,
    pub nth: u8,
    pub x_cm: i16,
    pub y_cm: i16,
    pub z_cm: i16,
}

/// The `nth` live landmark of `kind`, nearest the island's middle first
/// (ties to the lower slot). No allocation: each pass takes the next nearest
/// after the last one taken.
pub fn landmark_nth(h: &Haven, kind: u8, nth: u8) -> Option<&Landmark> {
    let mid = ISLAND_SIZE * 0.5;
    let d2 = |m: &Landmark| {
        let (dx, dz) = (m.x - mid, m.z - mid);
        dx * dx + dz * dz
    };
    // (distance², slot) of the last one taken; the next must sort after it.
    let mut last: Option<(f32, usize)> = None;
    let mut pick = None;
    for _ in 0..=nth {
        let mut best: Option<(f32, usize)> = None;
        for (i, m) in h.marks.iter().enumerate().take(LANDMARKS) {
            if !m.live || m.kind as u8 != kind {
                continue;
            }
            let k = (d2(m), i);
            let after = last.is_none_or(|l| k.0 > l.0 || (k.0 == l.0 && k.1 > l.1));
            let better = best.is_none_or(|b| k.0 < b.0 || (k.0 == b.0 && k.1 < b.1));
            if after && better {
                best = Some(k);
            }
        }
        let b = best?;
        last = Some(b);
        pick = Some(b.1);
    }
    pick.map(|i| &h.marks[i])
}

/// Where `p` stands in the world: (x, the site's floor + `y`, z), metres.
pub fn world(h: &Haven, p: &Spot) -> Option<(f32, f32, f32)> {
    let (x, y, z) = (
        p.x_cm as f32 * 0.01,
        p.y_cm as f32 * 0.01,
        p.z_cm as f32 * 0.01,
    );
    match p.site {
        SITE_TOWN => {
            let t = &h.town;
            if !t.live {
                return None;
            }
            let (wx, wz) = crate::kit::to_world(&t.placed(), x, z);
            Some((wx, t.floor_y + y, wz))
        }
        SITE_ZIGGURAT => {
            let g = &h.ziggurat;
            if !g.live {
                return None;
            }
            let (wx, wz) = crate::kit::to_world(&g.placed(), x, z);
            Some((wx, g.floor_y + y, wz))
        }
        s if (SITE_LANDMARK0..=SITE_MAX).contains(&s) => {
            let m = landmark_nth(h, s - SITE_LANDMARK0, p.nth)?;
            let (wx, wz) = landmark::to_world(m, x, z);
            Some((wx, m.y + y, wz))
        }
        _ => None,
    }
}

/// Whether a body with feet at (`x`, `feet`, `z`) is within `reach` metres of
/// `p` across the ground and a storey of it up or down.
pub fn within(h: &Haven, p: &Spot, x: f32, feet: f32, z: f32, reach: f32) -> bool {
    world(h, p).is_some_and(|(px, py, pz)| {
        let (dx, dz) = (x - px, z - pz);
        dx * dx + dz * dz <= reach * reach && crate::fmath::fabs(feet - py) <= 4.0
    })
}
