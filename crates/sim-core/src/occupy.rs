//! The world's scattered things, made solid — the consumer side of the seam
//! `terrain.rs` drew and then deliberately left open.
//!
//! `terrain::slot_blocks` answers "does THIS slot stop a capsule here?" for one
//! already-resolved slot. It has been complete and gated (`tests/solid.rs`)
//! and called by nothing, so a player walked through every pine, boulder,
//! barrel, crate and through the haven shelter's walls. This module owns the
//! other half that `terrain.rs:1126` names: *which* slots a body asks about,
//! and how they are indexed.
//!
//! ## Why an index and not just a loop
//!
//! `terrain::scatter` is the only way to get a `Slot`, and it is expensive:
//! measured at ~60 `noise2` evaluations for a typical inland cell (a `height`
//! fan of 11, a `slope` of 44, a `moisture` and a `clump`), ~105 on the coastal
//! road ring. The probe neighbourhood is 3×3 (`terrain::OCCUPANT_PROBE_CELLS`,
//! proved complete rather than assumed there), so a naive per-tick scan costs
//! ~540 `noise2` per body per tick — at `MAX_PLAYERS` and 30 Hz, roughly 1.6 M
//! per second for a query that answers "no" almost every time. `terrain.rs`
//! says so in its own words: "a movement step must never re-derive slots by
//! calling it".
//!
//! It never has to. **Scatter is a pure function of `(seed, cell)`** — slots are
//! potential, not state (`TERRAIN.md` §2) — so a memo is exact, not an
//! approximation. `SlotCache` is direct-mapped: a hit costs one compare, a miss
//! costs one `scatter` and overwrites whatever shared its line. Because the
//! function is pure, *eviction cannot change an answer* — it can only change
//! how long the answer took. That is why the cache is not sim state, is not in
//! `state_hash`, and why the server's copy and the client's copy holding
//! entirely different cells is not a divergence.
//!
//! A body walking at `SPRINT_SPEED` crosses a `CELL_SIZE` cell in ~44 ticks and
//! then admits 3 new cells to the 3×3, so the amortized miss rate is well under
//! one cell per body per tick against the naive nine.
//!
//! ## Harvested slots
//!
//! `scatter` will hand back a pine that was felled twenty minutes ago —
//! standing/harvested is the server's bit, held in `gather::SlotLives`. A
//! solidity test that consulted only `scatter` would leave an invisible trunk
//! in the way for the whole respawn window, so the harvested set is part of the
//! query. It is asked LAST, after a slot has already been found to block:
//! `SlotLives::find` is a linear scan over up to `MAX_SLOT_LIVES` entries, and
//! the common answer to "does anything block?" is no, which costs zero scans.
//!
//! ## Both sides, or prediction drifts
//!
//! `CLAUDE.md`'s quantize-both-sides law is why this is one bundle passed into
//! `movement::step` rather than a server-side check: a wall the client does not
//! predict is a rubber-band every time a player leans on a tree. The client
//! mirrors the harvested set already (`client-core`'s `HarvestedSet`), which is
//! what `Harvested` exists to abstract over — the server's sparse life store
//! and the client's key set answer the same question from different layouts.

use crate::collide::{CAPSULE_HEIGHT_M, CAPSULE_RADIUS_M};
use crate::fmath::floor_i32;
use crate::limits::SLOT_CACHE_SLOTS;
use crate::terrain::{self, Haven, Occupant, ScatterTable, Slot, CELLS_PER_SIDE, CELL_SIZE};

/// "Is the slot in this cell currently gone?" — the one thing the movement
/// path needs from harvest state, asked the same way by both sides.
///
/// A trait rather than a concrete type because the two stores are shaped for
/// different jobs: the server's `SlotLives` carries hit counts and respawn
/// ticks because it owns them, the client's `HarvestedSet` is a bare key set
/// because a mirror needs nothing else. Both are authoritative for their own
/// side and neither wants the other's layout.
pub trait Harvested {
    fn is_harvested(&self, cx: u16, cz: u16) -> bool;
}

/// Nothing has ever been harvested — the island as worldgen drew it. For
/// fixtures that want pristine terrain, and for any caller with no harvest
/// state to offer.
pub struct Pristine;

impl Harvested for Pristine {
    fn is_harvested(&self, _cx: u16, _cz: u16) -> bool {
        false
    }
}

/// Every cell reads as harvested, so nothing scattered blocks anything.
///
/// The other pole, and it earns its place: the piece-collision fixtures in
/// `collide.rs` and `deploy.rs` ask "does this wall stop a body", on real
/// terrain, at whatever coordinates the fixture picked. A pine that happens to
/// stand in one of those paths would fail a test about walls for a reason that
/// has nothing to do with walls. Occupant solidity is gated where it belongs,
/// in `tests/solid.rs`; here it is noise, and this turns it off without
/// pretending the terrain is something it is not.
pub struct Barren;

impl Harvested for Barren {
    fn is_harvested(&self, _cx: u16, _cz: u16) -> bool {
        true
    }
}

impl Harvested for crate::gather::SlotLives {
    fn is_harvested(&self, cx: u16, cz: u16) -> bool {
        crate::gather::SlotLives::is_harvested(self, cx, cz)
    }
}

/// A cell with no slot in it. Also the cache's fill value, which is safe
/// because `key` — not the slot — is what marks an entry live.
const EMPTY_SLOT: Slot = Slot {
    occupant: Occupant::None,
    x: 0.0,
    y: 0.0,
    z: 0.0,
    yaw: 0,
    scale: 1.0,
};

/// No cell can key to this: `cx` and `cz` are each bounded by
/// `CELLS_PER_SIDE`, so the packed key never reaches the top bits.
const NO_KEY: u32 = u32::MAX;

#[derive(Clone, Copy)]
struct Line {
    key: u32,
    slot: Slot,
}

/// A direct-mapped memo of `terrain::scatter`, keyed by cell.
///
/// Direct-mapped on purpose. An associative cache would need an eviction
/// policy, and an eviction policy is a rule that has to be right; here a
/// collision simply re-resolves, and the only thing at stake is time. There is
/// no failure mode to bound, so there is no overflow policy to get wrong —
/// which is the honest reason this is not shaped like the other capped stores
/// in `limits.rs`.
///
/// Not sim state and not hashed: it is a pure function's memo, so two peers
/// holding different lines still answer every query identically.
pub struct SlotCache {
    /// The seed the lines were resolved under. A cache handed a different
    /// seed is not stale, it is wrong, so it is emptied rather than trusted.
    seed: u64,
    /// How many times this cache has actually run `terrain::scatter` — the
    /// work it exists to avoid, counted so "it is cached" is a measurement
    /// rather than a claim. Monotonic across `reset`, because a seed change
    /// does not un-spend the taps already made.
    ///
    /// **A statistic, not sim state**, exactly like the lines it counts: it
    /// is never hashed, and two peers whose caches disagree about it still
    /// answer every query identically. It is what lets a gate assert a cold
    /// scan costs nine resolves and the next one at the same position costs
    /// none — a COUNT, which is quotable on any box, unlike the elapsed time
    /// that motivated the cache.
    resolves: u32,
    lines: [Line; SLOT_CACHE_SLOTS],
}

impl SlotCache {
    pub fn new() -> Self {
        Self {
            seed: 0,
            resolves: 0,
            lines: [Line {
                key: NO_KEY,
                slot: EMPTY_SLOT,
            }; SLOT_CACHE_SLOTS],
        }
    }

    /// The running count of `terrain::scatter` calls this cache has made.
    pub fn resolves(&self) -> u32 {
        self.resolves
    }

    /// Drop every line. Not a tick path — only a seed change reaches it.
    fn reset(&mut self, seed: u64) {
        self.seed = seed;
        let mut i = 0;
        while i < SLOT_CACHE_SLOTS {
            self.lines[i].key = NO_KEY;
            i += 1;
        }
    }

    /// The slot in cell (`cx`, `cz`), resolved once per seed and then read.
    ///
    /// Out-of-island cells never reach the table: `scatter` answers `None` for
    /// them by definition, so caching them would spend lines on the sea.
    pub fn slot(
        &mut self,
        seed: u64,
        table: &ScatterTable,
        haven: &Haven,
        cx: i32,
        cz: i32,
    ) -> Slot {
        if cx < 0 || cz < 0 || cx >= CELLS_PER_SIDE || cz >= CELLS_PER_SIDE {
            return EMPTY_SLOT;
        }
        if self.seed != seed {
            self.reset(seed);
        }
        let key = ((cx as u32) << 16) | cz as u32;
        // Fibonacci hash, the same mix `collide::ColIndex` uses to spread its
        // build cells; the low bits of a cell key alone would put a whole row
        // of the island on one line.
        let ix = (key.wrapping_mul(2_654_435_761) >> 16) as usize & (SLOT_CACHE_SLOTS - 1);
        if self.lines[ix].key == key {
            return self.lines[ix].slot;
        }
        let slot = terrain::scatter(seed, table, haven, cx, cz);
        self.resolves = self.resolves.wrapping_add(1);
        self.lines[ix] = Line { key, slot };
        slot
    }
}

impl Default for SlotCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Everything the occupant query needs that is not the body — one borrow
/// bundle so `movement::step` grows one argument rather than four.
pub struct Occupants<'a> {
    pub table: &'a ScatterTable,
    pub haven: &'a Haven,
    pub harvested: &'a dyn Harvested,
    pub cache: &'a mut SlotCache,
}

impl Occupants<'_> {
    /// Does anything scattered stop a capsule standing at (`x`, `z`) with its
    /// feet at `feet_y`?
    ///
    /// The 3×3 scan is complete rather than approximate — see
    /// `terrain::OCCUPANT_PROBE_CELLS`, which proves it against the widest
    /// occupant plus a capsule against `CELL_SIZE` in a const block.
    pub fn blocks(&mut self, seed: u64, x: f32, z: f32, feet_y: f32) -> bool {
        self.blocks_volume(seed, x, z, feet_y, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M)
    }

    /// The highest scattered surface a capsule at `feet_y` may stand on
    /// under (`x`, `z`), or `collide::NO_SURFACE` — `blocks`'s twin over
    /// [`terrain::slot_ground`], and the query that makes a crate top, a
    /// boulder top and the shelter's plinth ground instead of geometry a
    /// body sinks through (`NOW.md` §0q item 3).
    ///
    /// The same 3×3 scan, complete for the same reason with margin: a
    /// ground footprint is the occupant's own, which reaches strictly
    /// less far than the capsule-inflated blocking footprint the probe
    /// bound was proved against. Harvested is asked last and only for a
    /// slot that would otherwise answer, `blocks`'s own cost argument.
    pub fn ground(&mut self, seed: u64, x: f32, z: f32, feet_y: f32) -> f32 {
        let pcx = floor_i32(x / CELL_SIZE);
        let pcz = floor_i32(z / CELL_SIZE);
        let mut best = crate::collide::NO_SURFACE;
        let mut dz = -terrain::OCCUPANT_PROBE_CELLS;
        while dz <= terrain::OCCUPANT_PROBE_CELLS {
            let mut dx = -terrain::OCCUPANT_PROBE_CELLS;
            while dx <= terrain::OCCUPANT_PROBE_CELLS {
                let (cx, cz) = (pcx + dx, pcz + dz);
                dx += 1;
                let slot = self.cache.slot(seed, self.table, self.haven, cx, cz);
                if slot.occupant == Occupant::None {
                    continue;
                }
                let g = terrain::slot_ground(&slot, x, z, feet_y);
                if g <= best {
                    continue;
                }
                if self.harvested.is_harvested(cx as u16, cz as u16) {
                    continue;
                }
                best = g;
            }
            dz += 1;
        }
        best
    }

    /// The same question for a volume that is not a player. `blocks` is this
    /// with the capsule's own two constants; `ranged::step` passes an
    /// arrow's, which is nearly a point.
    ///
    /// The 3×3 scan stays complete for any radius **at or under the
    /// capsule's** — `terrain::OCCUPANT_PROBE_CELLS` proves its bound
    /// against `CAPSULE_RADIUS_M`, and a narrower query reaches no further
    /// than a wider one. A caller passing something fatter than a player
    /// would be outside what that const block proved, so the assert below
    /// says so rather than leaving it to the reader.
    pub fn blocks_volume(
        &mut self,
        seed: u64,
        x: f32,
        z: f32,
        feet_y: f32,
        r: f32,
        h: f32,
    ) -> bool {
        debug_assert!(
            r <= CAPSULE_RADIUS_M,
            "the 3x3 probe is proved complete against the capsule radius; a \
             wider query needs OCCUPANT_PROBE_CELLS re-proved with it"
        );
        let pcx = floor_i32(x / CELL_SIZE);
        let pcz = floor_i32(z / CELL_SIZE);
        let mut dz = -terrain::OCCUPANT_PROBE_CELLS;
        while dz <= terrain::OCCUPANT_PROBE_CELLS {
            let mut dx = -terrain::OCCUPANT_PROBE_CELLS;
            while dx <= terrain::OCCUPANT_PROBE_CELLS {
                let (cx, cz) = (pcx + dx, pcz + dz);
                dx += 1;
                let slot = self.cache.slot(seed, self.table, self.haven, cx, cz);
                if slot.occupant == Occupant::None {
                    continue;
                }
                if !terrain::slot_blocks(&slot, x, z, feet_y, r, h) {
                    continue;
                }
                // Last, and only for a slot that would otherwise stop us: this
                // is a linear scan over the life store, and "nothing blocks"
                // is the answer almost every tick. `cx`/`cz` are in range here
                // because an out-of-island cell resolved to `None` above.
                if self.harvested.is_harvested(cx as u16, cz as u16) {
                    continue;
                }
                return true;
            }
            dz += 1;
        }
        false
    }
}

/// An owned occupant bundle for fixtures, which have nowhere else to keep the
/// parts. Construct once and reuse — `terrain::haven` is a bounded argmax over
/// the whole road ring and has no business inside a step loop.
pub struct Scratch<H: Harvested> {
    pub table: ScatterTable,
    pub haven: Haven,
    pub harvested: H,
    pub cache: SlotCache,
}

impl Scratch<Barren> {
    /// A world where nothing scattered blocks — for fixtures about pieces.
    /// The haven is a placeholder because no answer depends on it here.
    pub fn barren() -> Self {
        Self {
            table: ScatterTable::alpha_default(),
            // Off the island entirely, so `in_haven` can never claim a cell a
            // fixture cares about. No answer here depends on it — `Barren`
            // has already made every slot pass through.
            haven: Haven {
                x: -1.0e6,
                z: -1.0e6,
                y: 0.0,
                relief: 0.0,
                phase: 0,
                shelter: 0,
                // No lesser tier either: `Barren` has already made every slot
                // pass through, and an inert site is what `in_waystation`
                // tests for first.
                minor: [terrain::Waystation::NONE; terrain::WAYSTATIONS],
            },
            harvested: Barren,
            cache: SlotCache::new(),
        }
    }
}

impl Scratch<Pristine> {
    /// The island as this seed draws it, nothing harvested.
    pub fn live(seed: u64) -> Self {
        Self {
            table: ScatterTable::alpha_default(),
            haven: terrain::haven(seed),
            harvested: Pristine,
            cache: SlotCache::new(),
        }
    }
}

impl<H: Harvested> Scratch<H> {
    /// This seed's island, with harvest state the caller supplies.
    pub fn with(seed: u64, harvested: H) -> Self {
        Self {
            table: ScatterTable::alpha_default(),
            haven: terrain::haven(seed),
            harvested,
            cache: SlotCache::new(),
        }
    }

    pub fn occupants(&mut self) -> Occupants<'_> {
        Occupants {
            table: &self.table,
            haven: &self.haven,
            harvested: &self.harvested,
            cache: &mut self.cache,
        }
    }
}

const _: () = {
    // Direct-mapped indexing masks, so the line count must be a power of two.
    assert!(SLOT_CACHE_SLOTS.is_power_of_two());
    // The packed key must not collide with the empty sentinel, and must not
    // lose a cell to truncation.
    assert!(CELLS_PER_SIDE > 0 && CELLS_PER_SIDE < 0x1_0000);
    // A destination-point test cannot tunnel. `blocks` asks about where a body
    // WOULD stand, not the segment it travels, which is sound only while one
    // tick's travel is shorter than the thinnest blocking region is wide. The
    // narrowest such region is a slot of radius ~0 grown by the capsule, i.e.
    // `2 * CAPSULE_RADIUS_M` across, and the longest step is a sprint tick.
    assert!(crate::movement::SPRINT_SPEED * crate::movement::DT < CAPSULE_RADIUS_M);
};
