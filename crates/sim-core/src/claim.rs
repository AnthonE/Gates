//! **Who may build here** — privilege as a volume the base emits, not a
//! sphere around the hearth (privilege v1, `reference/BUILDING.md` §3).
//!
//! ## What this replaces
//!
//! A planar circle: `HEARTH_RADIUS_M` from the hearth's cell centre,
//! claimed for the crew and refused to everyone else. That is the shape
//! the reference shipped and then **replaced in Devblog 185** with
//! privilege *emitted by the building blocks*, and their reason is the
//! obvious one once said out loud: a circle centred on a point is the
//! wrong shape for a building. It over-claims open ground on every side a
//! base does not extend to, and under-claims the far end of any base
//! longer than its radius.
//!
//! Ours is now: **a point is claimed when a piece of somebody's base
//! stands within [`PRIV_CUSHION_M`] of it, and that base has a hearth.**
//! Connectivity is a flood fill over the build grid.
//!
//! ## Why the grid makes this cheaper for us than it was for them
//!
//! Their query is `GetBuildingPrivilege(OBB, …)` — an oriented bounding
//! box against physics, which is why it needed a persistent building
//! identity with `Merge` and `Split` to be affordable. Our pieces are
//! already cells and already adjacent, so the same question is a
//! breadth-first walk over a hash-indexed grid: no physics, no building
//! id, no merge, no split, and deterministic by construction because the
//! queue order and the neighbour order are both fixed.
//!
//! ## The direction of the walk is the whole performance argument
//!
//! The obvious implementation flood-fills **from each hearth** and asks
//! whether the point is near anything it reached. That is one walk per
//! hearth — up to `MAX_HEARTHS` of them — for a question asked on every
//! placement.
//!
//! This walks the other way: seed from the pieces **near the point**,
//! flood outward through connected structure, and see which hearths the
//! walk reaches. One walk per query, whatever the shard holds.
//!
//! ## Bounds, and the policy when they bite
//!
//! [`PRIV_BFS_CELLS`] caps the walk. Overflow policy, stated: **the walk
//! stops and privilege does not extend past it** — a base larger than the
//! cap protects the cells the walk reached and not the rest. That is the
//! honest failure for a bounded search: it can under-claim, never
//! over-claim, so the worst it does is let somebody build against the far
//! wall of a fortress, and it can never lock a player out of open ground.
//!
//! Pure like the rest of the crate: no allocation (the queue and the
//! visited set are one fixed array), no `HashMap`, and floats only in the
//! wall-1 set.

use crate::build::{build_cell_of, Pieces, BUILD_CELL_M};
use crate::deploy::Deploys;
use crate::limits::{MAX_BUILD_COORD, PRIV_BFS_CELLS};

/// How far privilege reaches from a piece of the base, in meters. The
/// reference's own figure — roughly 16 m of cushion beyond the outermost
/// connected block (`BUILDING.md` §3). Proposed default, DECISIONS.md
/// §open (privilege v1).
pub const PRIV_CUSHION_M: f32 = 16.0;

/// The cushion in whole build cells, rounded **up** so the seed scan can
/// never miss a cell whose centre is inside the cushion. Integer
/// arithmetic on a constant, evaluated here rather than at each call.
const CUSHION_CELLS: i32 = (PRIV_CUSHION_M / BUILD_CELL_M) as i32 + 1;

/// Squared cushion, so the distance test needs no `sqrt` (wall 1).
const CUSHION2: f32 = PRIV_CUSHION_M * PRIV_CUSHION_M;

/// A cell address packed into one `u32`, which is what the walk's visited
/// set holds — comparing one integer beats comparing a pair, and the
/// packing is the same shape `gather::cell_key` uses for the wire.
#[inline]
fn key(cx: u16, cz: u16) -> u32 {
    ((cx as u32) << 16) | cz as u32
}

/// Does this cell hold any piece at all, at any level?
///
/// Planar on purpose: a base's *footprint* is what emits privilege, and a
/// second storey standing on a first adds nothing to the shape on the
/// ground. It also means the walk never has to care which level a piece
/// is on, which is the difference between one `ColMasks` lookup and eight.
#[inline]
fn built(pieces: &Pieces, cx: u16, cz: u16) -> bool {
    let m = pieces.cols().get(cx, cz);
    m.planes | m.stairs | m.walls_w | m.walls_n | m.doors_w | m.doors_n != 0
}

/// The walk's scratch: one fixed array used as both the queue and the
/// visited set, because a breadth-first walk that never revisits is
/// exactly a queue you also search.
struct Walk {
    seen: [u32; PRIV_BFS_CELLS],
    len: usize,
}

impl Walk {
    fn new() -> Self {
        Self {
            seen: [0; PRIV_BFS_CELLS],
            len: 0,
        }
    }

    /// Enqueue a cell if it is new and there is room. False means the cap
    /// is spent — the caller stops, which is the stated overflow policy.
    fn push(&mut self, k: u32) -> bool {
        if self.seen[..self.len].contains(&k) {
            return true;
        }
        if self.len == PRIV_BFS_CELLS {
            return false;
        }
        self.seen[self.len] = k;
        self.len += 1;
        true
    }
}

/// Every cell of connected structure reachable from within the cushion of
/// `(x, z)`, walked once. Returns the walk so a caller can ask more than
/// one question of it without paying twice.
fn reach(pieces: &Pieces, x: f32, z: f32) -> Walk {
    let mut w = Walk::new();
    let (cx0, cz0) = (build_cell_of(x), build_cell_of(z));

    // --- seeds: built cells whose centre is inside the cushion ----------
    for dz in -CUSHION_CELLS..=CUSHION_CELLS {
        for dx in -CUSHION_CELLS..=CUSHION_CELLS {
            let (cx, cz) = (cx0 + dx, cz0 + dz);
            if cx < 0 || cz < 0 || cx >= MAX_BUILD_COORD as i32 || cz >= MAX_BUILD_COORD as i32 {
                continue;
            }
            let (cx, cz) = (cx as u16, cz as u16);
            if !built(pieces, cx, cz) {
                continue;
            }
            let px = cx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5;
            let pz = cz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5;
            let (ddx, ddz) = (px - x, pz - z);
            if ddx * ddx + ddz * ddz > CUSHION2 {
                continue;
            }
            if !w.push(key(cx, cz)) {
                return w;
            }
        }
    }

    // --- the walk -------------------------------------------------------
    // A queue and its visited set are one array: `at` is the head, `len`
    // the tail, and everything between them has been seen. The neighbour
    // order is fixed, so two runs of the same world visit in the same
    // order — which is what makes the cap deterministic as well as bounded.
    let mut at = 0usize;
    while at < w.len {
        let k = w.seen[at];
        at += 1;
        let (cx, cz) = ((k >> 16) as i32, (k & 0xFFFF) as i32);
        for (dx, dz) in [(0i32, -1i32), (0, 1), (-1, 0), (1, 0)] {
            let (nx, nz) = (cx + dx, cz + dz);
            if nx < 0 || nz < 0 || nx >= MAX_BUILD_COORD as i32 || nz >= MAX_BUILD_COORD as i32 {
                continue;
            }
            let (nx, nz) = (nx as u16, nz as u16);
            if !built(pieces, nx, nz) {
                continue;
            }
            if !w.push(key(nx, nz)) {
                return w;
            }
        }
    }
    w
}

/// **The one privilege question**: is `(x, z)` inside a claim that `who`
/// is not on the crew of?
///
/// True refuses the verb. Every build verb asks this and nothing else, so
/// place, upgrade, repair and deploy cannot drift apart about whose base
/// is whose.
///
/// A point with no connected structure near it is **unclaimed**, whatever
/// hearths stand elsewhere — that is the whole change from the circle, and
/// it is what stops a hearth in a valley from claiming the hillside above
/// it (`BUILDING.md` §3).
pub fn foreign_claim(pieces: &Pieces, deploys: &Deploys, x: f32, z: f32, who: u32) -> bool {
    let hearths = deploys.hearths();
    if hearths.is_empty() {
        return false;
    }
    let w = reach(pieces, x, z);
    if w.len == 0 {
        return false;
    }
    hearths.iter().any(|h| {
        // Conservative on purpose: **any** reachable hearth whose crew
        // this hand is not on refuses. A base with two hearths is two
        // claims over one building, and being on one of them is not
        // permission from the other.
        !h.crew.contains(who) && w.seen[..w.len].contains(&key(h.cx, h.cz))
    })
}

/// Whether `(x, z)` is inside **any** hearth's claim, crewed or not — the
/// question the hearth-spacing rule asks (`place_deploy` refuses a hearth
/// inside one, own included) and the one `foreign_claim` deliberately is
/// not.
pub fn any_claim(pieces: &Pieces, deploys: &Deploys, x: f32, z: f32) -> bool {
    let hearths = deploys.hearths();
    if hearths.is_empty() {
        return false;
    }
    let w = reach(pieces, x, z);
    if w.len == 0 {
        return false;
    }
    hearths
        .iter()
        .any(|h| w.seen[..w.len].contains(&key(h.cx, h.cz)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{BuildContent, LOC_PLANE};
    use crate::world::World;

    const SEED: u64 = 0x0047_4154_4553;
    const OWNER: u32 = 7;
    const STRANGER: u32 = 9;

    /// A world holding pieces at the given cells and one hearth at the
    /// first of them. Built by writing the stores directly rather than by
    /// driving the verbs: this module is being tested, and the verbs
    /// *call* it, so driving them would make the fixture depend on the
    /// answer.
    fn base(cells: &[(u16, u16)]) -> (Pieces, Deploys) {
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        for &(cx, cz) in cells {
            pieces.insert_for_test(cx, cz, 0, LOC_PLANE, 0, &bc);
        }
        let (cx, cz) = cells[0];
        deploys.push_hearth_for_test(cx, cz, 0, OWNER);
        let _ = SEED;
        (pieces, deploys)
    }

    fn centre(cx: u16, cz: u16) -> (f32, f32) {
        (
            cx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
            cz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
        )
    }

    #[test]
    fn open_ground_is_unclaimed_however_close_the_hearth_is_to_it() {
        // The circle's defining behaviour, gone. A lone foundation with a
        // hearth on it claims the cushion around ITSELF and nothing on the
        // far side of the island.
        let (pieces, deploys) = base(&[(100, 100)]);
        let (nx, nz) = centre(101, 100);
        assert!(
            foreign_claim(&pieces, &deploys, nx, nz, STRANGER),
            "one cell away is inside the cushion"
        );
        let (fx, fz) = centre(140, 100);
        assert!(
            !foreign_claim(&pieces, &deploys, fx, fz, STRANGER),
            "120 m away is open ground, and a hearth in a valley does not \\
             own the hillside"
        );
    }

    #[test]
    fn privilege_follows_the_base_past_the_old_radius() {
        // A corridor 20 cells long — 60 m, well past `HEARTH_RADIUS_M`'s
        // 24. The far end is claimed because the STRUCTURE reaches it,
        // which is exactly what the circle could not say.
        let cells: Vec<(u16, u16)> = (100..120).map(|cx| (cx, 100)).collect();
        let (pieces, deploys) = base(&cells);
        let (fx, fz) = centre(119, 100);
        assert!(
            foreign_claim(&pieces, &deploys, fx, fz, STRANGER),
            "the far end of a long base is still the owner's"
        );
        assert!(
            !foreign_claim(&pieces, &deploys, fx, fz, OWNER),
            "and the crew builds there"
        );
    }

    #[test]
    fn a_disconnected_shack_next_door_is_not_covered() {
        // Two foundations with a gap: the walk cannot cross open ground,
        // so the shack emits no privilege of the hearth's. The gap is
        // wider than the cushion, or the seed scan would reach it
        // directly — which is the other half of the rule and is what the
        // distance below is chosen to separate.
        let mut cells = vec![(100u16, 100u16)];
        cells.push((130, 100));
        let (pieces, deploys) = base(&cells);
        let (sx, sz) = centre(130, 100);
        assert!(
            !foreign_claim(&pieces, &deploys, sx, sz, STRANGER),
            "a shack the base does not touch carries none of its claim"
        );
    }

    #[test]
    fn a_crewmate_is_never_refused_and_a_hearthless_world_never_claims() {
        let (pieces, deploys) = base(&[(100, 100)]);
        let (x, z) = centre(100, 100);
        assert!(!foreign_claim(&pieces, &deploys, x, z, OWNER));

        let bc = BuildContent::probe_fixture();
        let mut bare = Pieces::new();
        bare.insert_for_test(100, 100, 0, LOC_PLANE, 0, &bc);
        let empty = Deploys::new();
        assert!(
            !foreign_claim(&bare, &empty, x, z, STRANGER),
            "structure with no hearth on it claims nothing"
        );
        assert!(!any_claim(&bare, &empty, x, z));
    }

    #[test]
    fn any_claim_sees_the_owners_own_hearth_and_foreign_claim_does_not() {
        // The two questions are different and the hearth-spacing rule
        // needs the one `foreign_claim` refuses to answer.
        let (pieces, deploys) = base(&[(100, 100)]);
        let (x, z) = centre(100, 100);
        assert!(!foreign_claim(&pieces, &deploys, x, z, OWNER));
        assert!(
            any_claim(&pieces, &deploys, x, z),
            "you may not stack a second hearth on your own claim"
        );
    }

    #[test]
    fn the_walk_is_bounded_and_under_claims_rather_than_over_claims() {
        // A base past `PRIV_BFS_CELLS`. The stated overflow policy is that
        // privilege stops rather than extending, so the far end of an
        // enormous base is buildable — under-claiming, never over-claiming,
        // which is the failure a bounded search is allowed to have.
        let cells: Vec<(u16, u16)> = (0..PRIV_BFS_CELLS as u16 + 40)
            .map(|i| (100 + i, 100))
            .collect();
        let (pieces, deploys) = base(&cells);
        let (fx, fz) = centre(100 + PRIV_BFS_CELLS as u16 + 39, 100);
        assert!(
            !foreign_claim(&pieces, &deploys, fx, fz, STRANGER),
            "past the cap the walk stops — bounded, and stated"
        );
        let (nx, nz) = centre(101, 100);
        assert!(
            foreign_claim(&pieces, &deploys, nx, nz, STRANGER),
            "and everything the walk did reach is still claimed"
        );
    }

    /// The walk is a pure function of the stores, so two runs agree — the
    /// property wall 5 needs from anything a command path calls.
    #[test]
    fn the_answer_does_not_depend_on_the_order_the_stores_grew() {
        let forward: Vec<(u16, u16)> = (100..110).map(|cx| (cx, 100)).collect();
        let mut backward = forward.clone();
        backward.reverse();
        // Same cells, opposite insertion order, hearth on the same cell.
        let bc = BuildContent::probe_fixture();
        let mut a = Pieces::new();
        for &(cx, cz) in &forward {
            a.insert_for_test(cx, cz, 0, LOC_PLANE, 0, &bc);
        }
        let mut b = Pieces::new();
        for &(cx, cz) in &backward {
            b.insert_for_test(cx, cz, 0, LOC_PLANE, 0, &bc);
        }
        let mut da = Deploys::new();
        da.push_hearth_for_test(100, 100, 0, OWNER);
        let mut db = Deploys::new();
        db.push_hearth_for_test(100, 100, 0, OWNER);
        for cx in 100..115u16 {
            let (x, z) = centre(cx, 100);
            assert_eq!(
                foreign_claim(&a, &da, x, z, STRANGER),
                foreign_claim(&b, &db, x, z, STRANGER),
                "cell {cx} answered differently for two identical worlds"
            );
        }
    }

    /// A `World` is constructible with the module in it — the smoke that
    /// catches a store field going missing.
    #[test]
    fn a_fresh_world_claims_nothing() {
        let w = World::new(SEED);
        assert!(!foreign_claim(
            &w.pieces, &w.deploys, 100.0, 100.0, STRANGER
        ));
    }
}
