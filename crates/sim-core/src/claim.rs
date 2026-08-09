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
use crate::deploy::{Deploys, HearthRec};
use crate::limits::{CLAIM_INDEX_SLOTS, MAX_BUILD_COORD, MAX_HEARTHS, MAX_PIECES, PRIV_BFS_CELLS};

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

// ---------------------------------------------------------------------------
// The cached volume — the same shape, held still for the sweep
// ---------------------------------------------------------------------------

/// "No volume": the hearth's own cell held no structure when the cache was
/// last built, or the cache has never been built at all. Either way the
/// hearth covers nothing, which is the safe direction — an unbuilt cache
/// under-covers (things decay a period early) and can never protect a
/// piece the live walk would not.
const VOL_NONE: u16 = u16::MAX;

/// A table key is a cell key with a bit that cannot be zero, so zero can
/// mean *empty slot* — `collide::ColIndex`'s encoding, for its reason.
const TBL_OCCUPIED: u32 = 1 << 31;

/// Fibonacci-hash home slot — pure integer, wasm-identical, the same mix
/// `collide::ColIndex` uses.
#[inline]
fn tbl_home(key: u32) -> usize {
    (key.wrapping_mul(0x9E37_79B1) >> 18) as usize & (CLAIM_INDEX_SLOTS - 1)
}

/// **The privilege volumes, cached per hearth** — the shape [`foreign_claim`]
/// walks per query, computed once per mutation instead of once per tick.
///
/// The upkeep sweep asks "which hearths cover this point" for every due
/// entry, every tick. The walk above is priced for a keypress; per tick it
/// is not affordable, and that is the whole reason this cache exists
/// (`NOW.md` §0aa items 1–2 were one item wearing two numbers). What is
/// cached per hearth is its **connected component** of built cells — the
/// cells [`reach`] would flood through — and a query is then "is the point
/// within [`PRIV_CUSHION_M`] of any cached cell", which is exactly the
/// walk's own answer with the flood direction reversed (connectivity is
/// symmetric, so seeding from the hearth instead of the point changes
/// nothing but the cost model).
///
/// ## Determinism (wall 5), the whole argument
///
/// The cache is a **pure function** of the piece store and the hearth
/// list, both of which are canonical sim state. It is rebuilt at exactly
/// one place — the top of `deploy::upkeep_sweep`, a fixed point in every
/// tick, after the tick's commands and before any coverage question —
/// and only when the generation stamps say a piece or a hearth changed
/// since the last rebuild. Those stamps advance only at command
/// application and sweep removals, which are stream-ordered, so a live
/// run and its replay rebuild on the same ticks from identical inputs
/// and hold identical caches. There is **no lazy fill on first query**:
/// the refresh runs whether or not any entry is due, so no query order
/// can influence what the cache holds. Mutations *inside* a sweep (a
/// decay cascade taking pieces out mid-walk) leave the geometry one tick
/// stale for the remainder of that sweep — deterministically, since a
/// replay performs the same removals in the same visits — and the next
/// tick's refresh picks them up; the hearth *list* alignment, which
/// cannot be allowed to go stale even for a tick (a swap-removed hearth
/// would inherit another base's volume), is kept exact by
/// [`ClaimCache::hearth_swap_remove`].
///
/// Like `Pieces::cols`, it is derived state: never hashed, never saved.
///
/// ## Bounds (wall 4)
///
/// Everything rides existing caps. The pool holds each built cell at most
/// once, so it is `MAX_PIECES` long and cannot fill past the store that
/// feeds it; a single volume stops at [`PRIV_BFS_CELLS`] with the walk's
/// own stated policy (under-claim, never over-claim); the volume lists
/// ride `MAX_HEARTHS`; and the scratch map is `CLAIM_INDEX_SLOTS`
/// (limits.rs has the derivation). A rebuild is one bounded pass —
/// O(cells walked) probes plus one table clear — so a placement-spamming
/// client buys at most one rebuild per tick, not one per placement.
///
/// The one shape difference from the per-query walk, stated: a component
/// bigger than `PRIV_BFS_CELLS` is pooled in breadth-first order from the
/// **first hearth in list order** that reached it, and a later hearth on
/// the same component shares that truncated volume rather than walking
/// its own 256 cells from itself. Both under-claim; neither over-claims.
pub(crate) struct ClaimCache {
    /// Cells of every hearth-bearing component, packed back to back in
    /// walk order; volume `v` is `pool[vol_start[v]..][..vol_len[v]]`.
    pool: Box<[u32; MAX_PIECES]>,
    pool_len: usize,
    vol_start: [u16; MAX_HEARTHS],
    vol_len: [u16; MAX_HEARTHS],
    /// Planar bounds per volume, **cushion-expanded**, in metres:
    /// `[min_x, min_z, max_x, max_z]`. The cheap pre-reject that keeps a
    /// covers() miss at four compares instead of a cell scan.
    vol_bb: [[f32; 4]; MAX_HEARTHS],
    vol_count: usize,
    /// Which volume claims for each hearth, index-aligned to the hearth
    /// list ([`VOL_NONE`] = none). The one row `Deploys::remove_at` must
    /// keep aligned mid-tick.
    vol_of: [u16; MAX_HEARTHS],
    /// Open-addressed cell → volume map, rebuild-only scratch (queries
    /// never touch it). Lives here rather than in a stack frame because
    /// 96 KB of scratch in a frame is the wasm shadow-stack trap
    /// (`crate::boxed_array`'s doc), and clearing it is a `fill`, not an
    /// allocation.
    tbl_key: Box<[u32; CLAIM_INDEX_SLOTS]>,
    tbl_vol: Box<[u16; CLAIM_INDEX_SLOTS]>,
    /// The generation stamps the cache was built against — `u64::MAX`
    /// (never a live gen, which start at 0) until the first rebuild, so a
    /// fresh or freshly-loaded world always rebuilds on its first sweep.
    stamp_pieces: u64,
    stamp_hearths: u64,
}

impl ClaimCache {
    pub(crate) fn new() -> Box<Self> {
        Box::new(Self {
            pool: crate::boxed_array(0),
            pool_len: 0,
            vol_start: [0; MAX_HEARTHS],
            vol_len: [0; MAX_HEARTHS],
            vol_bb: [[0.0; 4]; MAX_HEARTHS],
            vol_count: 0,
            vol_of: [VOL_NONE; MAX_HEARTHS],
            tbl_key: crate::boxed_array(0),
            tbl_vol: crate::boxed_array(0),
            stamp_pieces: u64::MAX,
            stamp_hearths: u64::MAX,
        })
    }

    /// Whether the cache still describes stores at these generations.
    pub(crate) fn fresh_for(&self, pieces_gen: u64, hearth_gen: u64) -> bool {
        self.stamp_pieces == pieces_gen && self.stamp_hearths == hearth_gen
    }

    /// Mirror the hearth list's swap-remove so `vol_of` keeps describing
    /// the hearth it is indexed against — `bag_ready`'s parallel-array
    /// contract, applied to the cache row. The volumes themselves are not
    /// touched: geometry staleness inside the removing tick is accepted
    /// and documented above; row misalignment is not.
    pub(crate) fn hearth_swap_remove(&mut self, i: usize, last: usize) {
        self.vol_of[i] = self.vol_of[last];
        self.vol_of[last] = VOL_NONE;
    }

    /// Rebuild every hearth's volume from the stores. One table clear,
    /// then one breadth-first walk per **component** (hearths sharing a
    /// component share a walk — the map is what makes the share O(1)).
    pub(crate) fn rebuild(
        &mut self,
        pieces: &Pieces,
        hearths: &[HearthRec],
        pieces_gen: u64,
        hearth_gen: u64,
    ) {
        self.tbl_key.fill(0);
        self.vol_count = 0;
        self.pool_len = 0;
        for (hi, h) in hearths.iter().enumerate().take(MAX_HEARTHS) {
            let (hcx, hcz) = (h.cx, h.cz);
            let seed = TBL_OCCUPIED | key(hcx, hcz);
            // Share: a hearth standing on a component an earlier walk
            // already pooled reuses that volume. The probe terminates at
            // the first empty slot (limits.rs: the table never passes
            // half load), and that slot is exactly where the seed goes if
            // this walk turns out to be a new one.
            let mut slot = tbl_home(seed);
            let mut found = VOL_NONE;
            loop {
                let k = self.tbl_key[slot];
                if k == 0 {
                    break;
                }
                if k == seed {
                    found = self.tbl_vol[slot];
                    break;
                }
                slot = (slot + 1) & (CLAIM_INDEX_SLOTS - 1);
            }
            if found != VOL_NONE {
                self.vol_of[hi] = found;
                continue;
            }
            if !built(pieces, hcx, hcz) {
                // A hearth whose own cell holds no piece (a hostile save
                // could present one; the verbs cannot) emits nothing.
                self.vol_of[hi] = VOL_NONE;
                continue;
            }
            // A new volume: breadth-first from the hearth's cell over
            // built cells, the pool slice doubling as the queue exactly
            // as `Walk::seen` does, neighbour order fixed.
            let v = self.vol_count as u16;
            let start = self.pool_len;
            self.pool[self.pool_len] = key(hcx, hcz);
            self.pool_len += 1;
            self.tbl_key[slot] = seed;
            self.tbl_vol[slot] = v;
            let mut at = start;
            'walk: while at < self.pool_len {
                let c = self.pool[at];
                at += 1;
                let (cx, cz) = ((c >> 16) as i32, (c & 0xFFFF) as i32);
                for (dx, dz) in [(0i32, -1i32), (0, 1), (-1, 0), (1, 0)] {
                    let (nx, nz) = (cx + dx, cz + dz);
                    if nx < 0
                        || nz < 0
                        || nx >= MAX_BUILD_COORD as i32
                        || nz >= MAX_BUILD_COORD as i32
                    {
                        continue;
                    }
                    let (nx, nz) = (nx as u16, nz as u16);
                    if !built(pieces, nx, nz) {
                        continue;
                    }
                    let nk = TBL_OCCUPIED | key(nx, nz);
                    let mut s = tbl_home(nk);
                    loop {
                        let k = self.tbl_key[s];
                        if k == nk {
                            break; // seen — by this walk, or held by an
                                   // earlier capped one (both stop here)
                        }
                        if k == 0 {
                            if self.pool_len - start == PRIV_BFS_CELLS
                                || self.pool_len == MAX_PIECES
                            {
                                // The cap — the walk's own stated policy:
                                // it stops, and privilege does not extend
                                // past it. (The pool bound is provably
                                // unreachable — each pooled cell is built
                                // and distinct — but stated is cheaper
                                // than proven every time this is read.)
                                break 'walk;
                            }
                            self.tbl_key[s] = nk;
                            self.tbl_vol[s] = v;
                            self.pool[self.pool_len] = key(nx, nz);
                            self.pool_len += 1;
                            break;
                        }
                        s = (s + 1) & (CLAIM_INDEX_SLOTS - 1);
                    }
                }
            }
            // The cushion-expanded bounds of what got pooled.
            let (mut min_x, mut min_z) = (f32::INFINITY, f32::INFINITY);
            let (mut max_x, mut max_z) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
            for &k in &self.pool[start..self.pool_len] {
                let px = (k >> 16) as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5;
                let pz = (k & 0xFFFF) as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5;
                min_x = min_x.min(px);
                min_z = min_z.min(pz);
                max_x = max_x.max(px);
                max_z = max_z.max(pz);
            }
            self.vol_bb[v as usize] = [
                min_x - PRIV_CUSHION_M,
                min_z - PRIV_CUSHION_M,
                max_x + PRIV_CUSHION_M,
                max_z + PRIV_CUSHION_M,
            ];
            self.vol_start[v as usize] = start as u16;
            self.vol_len[v as usize] = (self.pool_len - start) as u16;
            self.vol_of[hi] = v;
            self.vol_count += 1;
        }
        self.stamp_pieces = pieces_gen;
        self.stamp_hearths = hearth_gen;
    }

    /// Whether hearth `hearth`'s cached volume covers the planar point —
    /// the walk's own predicate ("within the cushion of the connected
    /// structure that reaches this hearth"), read instead of walked.
    pub(crate) fn covers(&self, hearth: usize, x: f32, z: f32) -> bool {
        if hearth >= MAX_HEARTHS {
            return false;
        }
        let v = self.vol_of[hearth];
        if v == VOL_NONE {
            return false;
        }
        let bb = self.vol_bb[v as usize];
        if x < bb[0] || z < bb[1] || x > bb[2] || z > bb[3] {
            return false;
        }
        let s = self.vol_start[v as usize] as usize;
        let e = s + self.vol_len[v as usize] as usize;
        self.pool[s..e].iter().any(|&k| {
            let px = (k >> 16) as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5;
            let pz = (k & 0xFFFF) as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5;
            let (dx, dz) = (px - x, pz - z);
            dx * dx + dz * dz <= CUSHION2
        })
    }
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

    /// **The sweep's shape is the verbs' shape.** The cached volume the
    /// upkeep sweep reads and the walk the build verbs run must answer
    /// identically at every point (below the cap, where the two flood
    /// directions are exactly symmetric) — this is the gate on "the
    /// semantics of covered may drift only from circle to base-shape",
    /// asserted as a point-by-point equality rather than prose.
    #[test]
    fn the_cached_volume_and_the_walk_agree_on_the_claim_shape() {
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        for cx in 100..120u16 {
            pieces.insert_for_test(cx, 100, 0, LOC_PLANE, 0, &bc);
        }
        deploys.push_hearth_for_test(100, 100, 0, OWNER);
        deploys.refresh_claims(&pieces);
        for cx in 90..=130u16 {
            for cz in 92..=108u16 {
                let (x, z) = centre(cx, cz);
                assert_eq!(
                    deploys.hearth_covers(0, x, z),
                    any_claim(&pieces, &deploys, x, z),
                    "the cached shape and the walked shape disagree at ({cx},{cz})"
                );
            }
        }
    }

    /// Two hearths standing on one connected base share one cached
    /// volume (the rebuild walks each component once); demolishing the
    /// middle splits the base, and after a refresh each hearth covers
    /// its own island and not the other's.
    #[test]
    fn two_hearths_share_a_component_until_it_splits() {
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        for cx in 100..120u16 {
            pieces.insert_for_test(cx, 100, 0, LOC_PLANE, 0, &bc);
        }
        deploys.push_hearth_for_test(100, 100, 0, OWNER);
        deploys.push_hearth_for_test(119, 100, 0, OWNER);
        deploys.refresh_claims(&pieces);
        let near = centre(100, 100);
        let far = centre(119, 100);
        assert!(deploys.hearth_covers(0, far.0, far.1));
        assert!(deploys.hearth_covers(1, near.0, near.1));

        // Take out the middle: 103..=116 leaves two three-cell islands
        // 42 m apart — far outside the cushion, so coverage has to split
        // with the structure.
        for cx in 103..=116u16 {
            let i = pieces.find_index(cx, 100, 0, LOC_PLANE).unwrap();
            let shape = bc.pieces[pieces.entries()[i].row as usize].shape;
            pieces.remove_at(i, shape);
        }
        deploys.refresh_claims(&pieces);
        assert!(
            deploys.hearth_covers(0, near.0, near.1) && !deploys.hearth_covers(0, far.0, far.1),
            "hearth 0 keeps its island and loses the other"
        );
        assert!(
            deploys.hearth_covers(1, far.0, far.1) && !deploys.hearth_covers(1, near.0, near.1),
            "hearth 1 likewise"
        );
    }
}
