//! Somewhere to hide: a tree or a rock with me on the far side of it from
//! the danger, the way a player under fire looks for the nearest trunk.
//!
//! Read from the scatter the client already draws (the same cells a
//! gather scan reads), never from a list of what stands where: a felled
//! tree is no cover, and the client knows which it felled because the
//! stump is on its screen. The scan is a fixed window of cells, so it
//! costs the same wherever the body is.

use client_core::core::ClientCore;
use sim_core::terrain::{self, Occupant, CELL_SIZE};

/// Stand this far past the far edge of what hides me, found in steps of
/// [`EDGE_STEP_M`] out from its middle, no further than [`EDGE_MAX_M`].
pub const BEHIND_M: f32 = 0.6;
pub const EDGE_STEP_M: f32 = 0.4;
pub const EDGE_MAX_M: f32 = 4.0;
/// Cover that would take me this much nearer the danger is no cover.
pub const TOWARD_SLACK_M: f32 = 3.0;
/// Something to hide behind stands at least this far from the danger:
/// the body beside it can walk round.
pub const MIN_THREAT_M: f32 = 4.0;
/// It is solid this high off the ground at its middle: a head's height,
/// so a body standing behind it is out of sight, not just its legs.
pub const HIDE_Y_M: f32 = 1.5;
/// A spot this near one that already failed is the same spot.
pub const SAME_SPOT_M: f32 = 2.0;

/// The nearest place within `max_m` of `me` with a tree or a rock between
/// it and `threat` (ground positions, metres), other than `avoid` (a spot
/// that turned out to be in their sight after all).
pub fn find(
    core: &mut ClientCore,
    me: [f32; 2],
    threat: [f32; 2],
    max_m: f32,
    avoid: Option<[f32; 2]>,
) -> Option<[f32; 2]> {
    let reach = (max_m / CELL_SIZE).ceil() as i32 + 1;
    let (mcx, mcz) = (
        (me[0] / CELL_SIZE).floor() as i32,
        (me[1] / CELL_SIZE).floor() as i32,
    );
    let from_threat = dist(me, threat);
    let mut best: Option<(f32, [f32; 2])> = None;
    for dz in -reach..=reach {
        for dx in -reach..=reach {
            let (cx, cz) = (mcx + dx, mcz + dz);
            if !(0..terrain::CELLS_PER_SIDE).contains(&cx)
                || !(0..terrain::CELLS_PER_SIDE).contains(&cz)
            {
                continue;
            }
            let (seed, mut island) = core.island();
            let slot = island.cache.slot(seed, island.table, island.haven, cx, cz);
            let solid = matches!(
                slot.occupant,
                Occupant::Tree
                    | Occupant::StoneNode
                    | Occupant::MetalNode
                    | Occupant::SulfurNode
                    | Occupant::Rock
            );
            // A felled trunk is a stump: `blocks_volume` reads the
            // harvested set the client mirrors.
            if !solid {
                continue;
            }
            let floor = terrain::ground(seed, island.haven, slot.x, slot.z);
            if !island.blocks_volume(seed, slot.x, slot.z, floor + HIDE_Y_M, 0.0, 0.0) {
                continue;
            }
            let o = [slot.x, slot.z];
            let off = dist(o, threat);
            if off < MIN_THREAT_M {
                continue;
            }
            let away = [(o[0] - threat[0]) / off, (o[1] - threat[1]) / off];
            // Past the far edge of it: a boulder is wider than a trunk.
            let mut edge = EDGE_STEP_M;
            while edge < EDGE_MAX_M {
                let (x, z) = (o[0] + away[0] * edge, o[1] + away[1] * edge);
                let y = terrain::ground(seed, island.haven, x, z) + 0.5;
                if !island.blocks_volume(seed, x, z, y, 0.0, 0.0) {
                    break;
                }
                edge += EDGE_STEP_M;
            }
            let spot = [
                o[0] + away[0] * (edge + BEHIND_M),
                o[1] + away[1] * (edge + BEHIND_M),
            ];
            let d = dist(me, spot);
            if d > max_m
                || dist(spot, threat) < from_threat - TOWARD_SLACK_M
                || avoid.is_some_and(|a| dist(a, spot) < SAME_SPOT_M)
            {
                continue;
            }
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, spot));
            }
        }
    }
    best.map(|(_, spot)| spot)
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a real island, cover found from a threat is a trunk or a rock
    /// standing between the spot and the threat, near me, and not taken
    /// toward the danger.
    #[test]
    fn cover_puts_something_solid_between_me_and_the_danger() {
        let seed = 20260731;
        let haven = terrain::haven(seed);
        let mut core = ClientCore::new(seed, 1, 0);
        let mut found = 0;
        for cz in (40..200).step_by(17) {
            for cx in (40..200).step_by(13) {
                let me = [cx as f32 * CELL_SIZE, cz as f32 * CELL_SIZE];
                let threat = [me[0] + 20.0, me[1] + 5.0];
                let Some(spot) = find(&mut core, me, threat, 20.0, None) else {
                    continue;
                };
                found += 1;
                assert!(dist(me, spot) <= 20.0);
                assert!(dist(spot, threat) >= dist(me, threat) - TOWARD_SLACK_M);
                // Something solid at head height stands on the line from
                // the spot toward the threat, close in front of it.
                let (ux, uz) = (
                    (threat[0] - spot[0]) / dist(spot, threat),
                    (threat[1] - spot[1]) / dist(spot, threat),
                );
                let hidden = (1..=((EDGE_MAX_M + BEHIND_M) / 0.2) as i32).any(|k| {
                    let (x, z) = (spot[0] + ux * 0.2 * k as f32, spot[1] + uz * 0.2 * k as f32);
                    let (seed, mut island) = core.island();
                    let y = terrain::ground(seed, &haven, x, z) + HIDE_Y_M;
                    island.blocks_volume(seed, x, z, y, 0.0, 0.0)
                });
                assert!(hidden, "nothing in front of {spot:?}");
                let other = find(&mut core, me, threat, 20.0, Some(spot));
                assert!(other.is_none_or(|o| dist(o, spot) >= SAME_SPOT_M));
            }
        }
        assert!(found > 5, "only {found} spots had cover");
    }
}
