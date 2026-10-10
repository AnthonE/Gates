//! World-space anchors' arithmetic (NOW §0x item 2): where on a structure
//! a label hangs, where on the screen that lands, and how far off it still
//! draws. Pure, so it is testable in the code tier; `render/anchor.rs` is
//! the Bevy half that moves the two text nodes.
//!
//! The address is the wire's and the store bit beside it is what picks the
//! point: a piece (and an edge insert, a door in its doorway) hangs at
//! `sim_core::build::anchor`, a deployable placed freely at its own centre
//! (`sim_core::deploy::rec_anchor`). `build::anchor` read on a body slot's
//! `loc` would answer a corner of the cell the box is nowhere near, which is
//! why `charge_deploy` and `own_struct_deploy` ride beside their tuples.

use client_core::core::ClientCore;
use sim_core::deploy::DeployRec;
use sim_core::terrain::Haven;

/// A structure's address and which store it lives in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spot {
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub loc: u8,
    pub deploy: bool,
}

impl Spot {
    /// The last charge planted (`ClientCore::charge_placed`).
    pub fn charge(core: &ClientCore) -> Self {
        let (cx, cz, level, loc, _, _) = core.charge_placed;
        Self {
            cx,
            cz,
            level,
            loc,
            deploy: core.charge_deploy,
        }
    }

    /// The structure this player's own last blow landed on
    /// (`ClientCore::own_struct_hit`).
    pub fn own_hit(core: &ClientCore) -> Self {
        let (cx, cz, level, loc, _, _) = core.own_struct_hit;
        Self {
            cx,
            cz,
            level,
            loc,
            deploy: core.own_struct_deploy,
        }
    }
}

/// Where a charge's clock hangs over the floor its structure stands on,
/// metres: where `render/audio.rs` sounds its fuse.
pub const CHARGE_LIFT_M: f32 = 1.0;

/// Where the wall readout hangs: a metre over the clock, so a charge on the
/// next wall along does not print on top of the number. (On the wall itself
/// the clock outranks it and the number stands down, as on the pinned line.)
pub const WALL_LIFT_M: f32 = 2.0;

/// A tag draws whole out to here…
pub const TAG_FULL_M: f32 = 40.0;
/// …and is gone by here. The pinned readout (`hud::readout`) carries the
/// bearing and distance past it; a number at a wall a hill away is noise.
pub const TAG_GONE_M: f32 = 60.0;

/// How far past the screen's edge, as a fraction of it, a tag's point may
/// stray and still draw, so it slides off rather than blinking out while
/// half its text is still on screen.
pub const EDGE_SLACK: f32 = 0.05;

/// The record a deployable spot names, if the mirror holds it.
fn deploy_rec(deploys: &[DeployRec], s: Spot) -> Option<&DeployRec> {
    if !s.deploy {
        return None;
    }
    deploys
        .iter()
        .find(|r| (r.cx, r.cz, r.level, r.loc) == (s.cx, s.cz, s.level, s.loc))
}

/// The planar point a label at `s` hangs over: the deployable's own anchor
/// when the spot names one the mirror holds, else the piece anchor — which
/// is also right for an edge insert, and is the cell's own corner of the
/// world for a record already gone (the blast took it).
pub fn spot_xz(deploys: &[DeployRec], s: Spot) -> (f32, f32) {
    match deploy_rec(deploys, s) {
        Some(rec) => sim_core::deploy::rec_anchor(rec),
        None => sim_core::build::anchor(s.cx, s.cz, s.loc),
    }
}

/// The world point a label at `s` hangs at: [`spot_xz`], `lift` metres over
/// the floor the structure stands on — the deployable's own floor
/// (`deploy::body_base_y`, the one the renderer stands it on) or the
/// column's floor at the spot's storey.
pub fn structure_point(
    seed: u64,
    haven: &Haven,
    core: &ClientCore,
    s: Spot,
    lift: f32,
) -> [f32; 3] {
    let cols = core.pieces.cols();
    let (x, z) = spot_xz(core.deploys.entries(), s);
    let rec =
        deploy_rec(core.deploys.entries(), s).filter(|r| (r.row as u16) < core.deploy_defs_have);
    let floor = match rec {
        Some(r) => {
            let arch = core.deploy_defs.defs[r.row as usize].arch;
            sim_core::deploy::body_base_y(seed, haven, cols, r, arch)
        }
        None => {
            let plate = cols.plate(s.cx, s.cz).unwrap_or(0);
            sim_core::build::column_floor_y(seed, haven, s.cx, s.cz, plate)
                + sim_core::build::level_y(s.level)
        }
    };
    [x, floor + lift, z]
}

/// Where a projected point lands on the screen, as fractions of its width
/// and height from the top left, or `None` when it does not draw: behind the
/// eye (`ndc.z` at or under 0 under Bevy's reversed-Z perspective), inside
/// the near plane (over 1), or further off the edge than [`EDGE_SLACK`].
///
/// Fractions rather than pixels so a UI node can be placed in percent: the
/// eye may render below window size (`render_scale.rs`) and the browser
/// scales the UI (`UiScale`), and a fraction of the screen is the one unit
/// both of those leave alone.
pub fn screen_frac(ndc: [f32; 3]) -> Option<[f32; 2]> {
    let [x, y, z] = ndc;
    if !(z > 0.0 && z <= 1.0) {
        return None;
    }
    // NDC's y runs up; the screen's runs down.
    let (fx, fy) = ((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    let on = |f: f32| (-EDGE_SLACK..=1.0 + EDGE_SLACK).contains(&f);
    (on(fx) && on(fy)).then_some([fx, fy])
}

/// How opaque a tag is at `dist_m` from the eye: whole to [`TAG_FULL_M`],
/// gone by [`TAG_GONE_M`], linear between.
pub fn range_alpha(dist_m: f32) -> f32 {
    ((TAG_GONE_M - dist_m) / (TAG_GONE_M - TAG_FULL_M)).clamp(0.0, 1.0)
}

/// Whole seconds a clock shows for `secs_left`, or `None` once it is spent.
/// Ceiling — a live thing must not read as finished, so a fuse says `1s`
/// until the instant it is gone. The pinned readout and the clock on the
/// charge both count through this, so the two can never disagree.
pub fn clock_secs(secs_left: f32) -> Option<u32> {
    (secs_left > 0.0).then(|| secs_left.ceil() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::build::{BUILD_CELL_M, LOC_EDGE_XLO};

    /// The store bit is what picks the point. A box in a body slot of cell
    /// (10, 12), stood off the centre by its pose, must hang its clock over
    /// itself — the piece rule would hang it at whatever corner that slot
    /// number names as a piece location. A piece, and a deployable spot the
    /// mirror no longer holds, take `build::anchor`.
    #[test]
    fn the_store_bit_picks_where_the_tag_hangs() {
        let body_loc = sim_core::deploy::BODY_LOCS[1];
        let rec = DeployRec {
            cx: 10,
            cz: 12,
            level: 0,
            loc: body_loc,
            pose: sim_core::footprint::Pose {
                ox: 20,
                oz: -30,
                yaw: 0,
            },
            ..Default::default()
        };
        let spot = Spot {
            cx: 10,
            cz: 12,
            level: 0,
            loc: body_loc,
            deploy: true,
        };
        assert_eq!(spot_xz(&[rec], spot), rec.xz());
        assert_ne!(
            rec.xz(),
            sim_core::build::anchor(10, 12, body_loc),
            "the case would prove nothing if the two rules agreed here"
        );
        let piece = Spot {
            deploy: false,
            ..spot
        };
        assert_eq!(
            spot_xz(&[rec], piece),
            sim_core::build::anchor(10, 12, body_loc)
        );
        assert_eq!(
            spot_xz(&[], spot),
            sim_core::build::anchor(10, 12, body_loc),
            "a record already gone falls back to its address"
        );
        // A door (an edge insert) and its frame hang at one point.
        let door = DeployRec {
            loc: LOC_EDGE_XLO,
            pose: sim_core::footprint::Pose::default(),
            ..rec
        };
        let at = Spot {
            loc: LOC_EDGE_XLO,
            ..spot
        };
        assert_eq!(
            spot_xz(&[door], at),
            (10.0 * BUILD_CELL_M, 12.5 * BUILD_CELL_M)
        );
    }

    /// NDC to screen: the middle is the middle, up is the top, and a point
    /// behind the eye or well off the edge draws nowhere — the case a naive
    /// projection gets wrong, mirrored back onto the screen from behind.
    #[test]
    fn a_point_lands_where_the_eye_sees_it() {
        assert_eq!(screen_frac([0.0, 0.0, 0.5]), Some([0.5, 0.5]));
        assert_eq!(screen_frac([-1.0, 1.0, 0.5]), Some([0.0, 0.0]), "top left");
        assert_eq!(
            screen_frac([1.0, -1.0, 0.01]),
            Some([1.0, 1.0]),
            "bottom right"
        );
        assert_eq!(screen_frac([0.0, 0.0, -0.2]), None, "behind the eye");
        assert_eq!(screen_frac([0.0, 0.0, 0.0]), None, "at infinity behind");
        assert_eq!(screen_frac([0.0, 0.0, 1.5]), None, "inside the near plane");
        assert_eq!(screen_frac([1.3, 0.0, 0.5]), None, "off the right edge");
        assert!(
            screen_frac([1.05, 0.0, 0.5]).is_some(),
            "sliding off, still drawn"
        );
        assert_eq!(screen_frac([f32::NAN, 0.0, 0.5]), None);
    }

    #[test]
    fn a_tag_fades_out_with_range() {
        assert_eq!(range_alpha(0.0), 1.0);
        assert_eq!(range_alpha(TAG_FULL_M), 1.0);
        let mid = range_alpha((TAG_FULL_M + TAG_GONE_M) * 0.5);
        assert!((mid - 0.5).abs() < 1e-6, "{mid}");
        assert_eq!(range_alpha(TAG_GONE_M), 0.0);
        assert_eq!(range_alpha(500.0), 0.0);
    }

    #[test]
    fn a_clock_never_reads_finished_while_it_runs() {
        assert_eq!(clock_secs(0.0), None);
        assert_eq!(clock_secs(-1.0), None);
        assert_eq!(clock_secs(0.01), Some(1));
        assert_eq!(clock_secs(7.0), Some(7));
        assert_eq!(clock_secs(7.01), Some(8));
    }
}
