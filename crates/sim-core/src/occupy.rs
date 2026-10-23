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

    /// How much of the slot stands, per mille (tree growth v0): 0 when
    /// harvested, a sapling's share while one regrows, 1000 otherwise. The
    /// default knows no saplings, so every fixture store stays exactly
    /// what it was.
    fn standing_pm(&self, cx: u16, cz: u16) -> u16 {
        if self.is_harvested(cx, cz) {
            0
        } else {
            1000
        }
    }

    /// How much of a TREE slot stands, per mille (tree ages v0): what
    /// [`Harvested::standing_pm`] says, and for a tree the store has not
    /// felled, the wild stand's age (`gather::wild_pm`). The default knows
    /// no ages, so every fixture store stays exactly what it was.
    fn tree_pm(&self, _seed: u64, cx: u16, cz: u16) -> u16 {
        self.standing_pm(cx, cz)
    }

    /// A felled tree's stump still has wood in it (`NOW.md` §0stump). Asked
    /// only of a harvested tree slot. The default knows no stumps, so no
    /// fixture store grows a swingable one.
    fn stump_standing(&self, _cx: u16, _cz: u16) -> bool {
        false
    }

    /// How much of a worn ore node's height still stands, per mille
    /// (`gather::ORE_STAGE_TOP_PM`): a node mined down is struck and stood on
    /// at what is left of it. Asked only of a standing slot. The default
    /// knows no wear.
    fn ore_top_pm(&self, _cx: u16, _cz: u16, _o: Occupant) -> u16 {
        1000
    }
}

/// How much of the slot at `(cx, cz)` stands, per mille: a tree's own
/// answer, age included ([`Harvested::tree_pm`]), or anything else's.
#[inline]
pub fn slot_pm(h: &dyn Harvested, seed: u64, cx: u16, cz: u16, occupant: Occupant) -> u16 {
    if occupant == Occupant::Tree {
        h.tree_pm(seed, cx, cz)
    } else {
        h.standing_pm(cx, cz)
    }
}

/// A slot sunk to `pm` per mille of its height: the same radius, its top
/// where a worn ore node's top is now. Nothing that asks reaches below the
/// ground it sinks through.
#[inline]
pub fn sunk(slot: &Slot, pm: u16) -> Slot {
    let (_, top) = terrain::slot_volume(slot);
    Slot {
        y: slot.y - top * slot.scale * ((1000 - pm.min(1000)) as f32 * 0.001),
        ..*slot
    }
}

/// A slot at `pm` per mille of its size — a regrowing sapling is the tree
/// it will be, smaller, standing where it will stand.
#[inline]
pub fn grown(slot: &Slot, pm: u16) -> Slot {
    Slot {
        scale: slot.scale * (pm as f32 * 0.001),
        ..*slot
    }
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

    fn standing_pm(&self, cx: u16, cz: u16) -> u16 {
        crate::gather::SlotLives::standing_pm(self, cx, cz)
    }

    fn tree_pm(&self, seed: u64, cx: u16, cz: u16) -> u16 {
        crate::gather::SlotLives::tree_pm(self, seed, cx, cz)
    }

    fn stump_standing(&self, cx: u16, cz: u16) -> bool {
        self.find(cx, cz)
            .is_some_and(|e| e.respawn_at != 0 && e.hits != crate::gather::STUMP_GRUBBED)
    }

    fn ore_top_pm(&self, cx: u16, cz: u16, o: Occupant) -> u16 {
        crate::gather::SlotLives::ore_top_pm(self, cx, cz, o)
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
    species: 0,
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
    /// Sized at construction like the crags: one body's prediction needs
    /// [`SLOT_CACHE_SLOTS`], a shard's spread-out roster
    /// `limits::SLOT_CACHE_LINES`.
    lines: Box<[Line]>,
    line_shift: u32,
    /// Resolved rock formations (`boulder::formation`), keyed by formation
    /// cell. On the heap: the line array above already sits in this struct.
    rocks: Box<[RockLine; ROCK_CACHE_LINES]>,
    /// Resolved cliff crags (`cliff::crag`), keyed by spot, sized at
    /// construction (a power of two; `crag_shift` takes the hash's top bits).
    crags: Box<[CragLine]>,
    crag_shift: u32,
    /// The ground's height, the faces' coarse points (`face::coarse_at`),
    /// the solid surface (`face::surface`) and its face law
    /// (`movement::face_fall`), keyed by whole-metre point, and the skin
    /// (`face::skin_point`) by half-metre point, each sized at construction
    /// like the crags.
    heights: Box<[SurfLine]>,
    coarses: Box<[CoarseLine]>,
    surfs: Box<[SurfLine]>,
    falls: Box<[FallLine]>,
    skins: Box<[SurfLine]>,
    face_shift: u32,
    skin_shift: u32,
    /// The worldgen memo a crag resolves through. It changes no answer.
    lat: Box<terrain::Lattice>,
}

/// Formation cells the cache keeps. A body touches at most four; a shard's
/// roster spreads over a few hundred. A collision re-resolves, bit-identically.
pub const ROCK_CACHE_LINES: usize = 256;

/// How far a body is held off a rock's drawn surface, metres: the capsule's
/// own radius, less a little so it reads as touching rather than hovering.
pub const ROCK_SKIN_M: f32 = CAPSULE_RADIUS_M * 0.85;

const _: () = assert!(ROCK_CACHE_LINES.is_power_of_two());

#[derive(Clone, Copy)]
struct RockLine {
    key: u32,
    f: crate::boulder::Formation,
}

#[derive(Clone, Copy)]
struct CragLine {
    key: u32,
    c: crate::cliff::Crag,
}

#[derive(Clone, Copy)]
struct FallLine {
    key: u32,
    /// The fall line, or `(0, 0)` where a body may stand.
    fall: (f32, f32),
}

#[derive(Clone, Copy)]
struct CoarseLine {
    key: u32,
    c: crate::face::Coarse,
}

#[derive(Clone, Copy)]
struct SurfLine {
    key: u32,
    y: f32,
}

/// A whole- or half-metre point's key. Points are never negative on the
/// island, and a coordinate past 0xFFFF is never a body's.
#[inline]
fn point_key(ix: i32, iz: i32) -> u32 {
    ((ix as u32 & 0xFFFF) << 16) | (iz as u32 & 0xFFFF)
}

impl SlotCache {
    /// A cache for one body's queries: a client's prediction, a fixture.
    pub fn new() -> Self {
        Self::with_lines(
            SLOT_CACHE_SLOTS,
            crate::limits::CRAG_CACHE_LINES_ONE,
            crate::limits::FACE_CACHE_LINES_ONE,
        )
    }

    /// A cache for a whole shard's bodies.
    pub fn shard() -> Self {
        Self::with_lines(
            crate::limits::SLOT_CACHE_LINES,
            crate::limits::CRAG_CACHE_LINES,
            crate::limits::FACE_CACHE_LINES,
        )
    }

    fn with_lines(slots: usize, n: usize, faces: usize) -> Self {
        assert!(slots.is_power_of_two() && slots >= 2);
        assert!(n.is_power_of_two() && n >= 4);
        assert!(faces.is_power_of_two() && faces >= 2);
        Self {
            seed: 0,
            resolves: 0,
            lines: vec![
                Line {
                    key: NO_KEY,
                    slot: EMPTY_SLOT,
                };
                slots
            ]
            .into_boxed_slice(),
            line_shift: 32 - slots.trailing_zeros(),
            rocks: crate::boxed_array(RockLine {
                key: NO_KEY,
                f: crate::boulder::Formation::EMPTY,
            }),
            crags: vec![
                CragLine {
                    key: NO_KEY,
                    c: crate::cliff::Crag::NONE,
                };
                n
            ]
            .into_boxed_slice(),
            // One set per two lines (`crag`).
            crag_shift: 32 - (n / 2).trailing_zeros(),
            heights: vec![
                SurfLine {
                    key: NO_KEY,
                    y: 0.0
                };
                faces
            ]
            .into_boxed_slice(),
            falls: vec![
                FallLine {
                    key: NO_KEY,
                    fall: (0.0, 0.0),
                };
                faces
            ]
            .into_boxed_slice(),
            coarses: vec![
                CoarseLine {
                    key: NO_KEY,
                    c: crate::face::Coarse::default(),
                };
                faces
            ]
            .into_boxed_slice(),
            surfs: vec![
                SurfLine {
                    key: NO_KEY,
                    y: 0.0
                };
                faces
            ]
            .into_boxed_slice(),
            skins: vec![
                SurfLine {
                    key: NO_KEY,
                    y: 0.0
                };
                faces * 2
            ]
            .into_boxed_slice(),
            face_shift: 32 - faces.trailing_zeros(),
            skin_shift: 32 - (faces * 2).trailing_zeros(),
            lat: Box::new(terrain::Lattice::new()),
        }
    }

    /// The crag spot (`gi`, `gj`) grows, resolved once per seed.
    #[inline]
    ///
    /// Two-way: the lines pair into sets, the more recent of a set first. A
    /// spread roster's spots collide often enough in a direct map that two
    /// bodies' spots took turns evicting each other every tick (profile,
    /// 2026-10-20); a second way ends that at the same size.
    pub fn crag(&mut self, seed: u64, haven: &Haven, gi: i32, gj: i32) -> crate::cliff::Crag {
        if self.seed != seed {
            self.reset(seed);
        }
        let key = ((gi as u32 & 0xFFFF) << 16) | (gj as u32 & 0xFFFF);
        let set = ((key.wrapping_mul(2_654_435_761) >> self.crag_shift) as usize) << 1;
        if self.crags[set].key == key {
            return self.crags[set].c;
        }
        if self.crags[set + 1].key == key {
            self.crags.swap(set, set + 1);
            return self.crags[set].c;
        }
        let c = crate::cliff::crag(&mut self.lat, seed, haven, gi, gj);
        self.crags[set + 1] = self.crags[set];
        self.crags[set] = CragLine { key, c };
        c
    }

    /// `terrain::ground` at any point, through this cache's worldgen memo:
    /// the same bits, minus re-hashing the corners a neighbouring read just
    /// hashed. What a ray's dozen samples a metre apart mostly share.
    #[inline]
    pub(crate) fn ground(&mut self, seed: u64, haven: &Haven, x: f32, z: f32) -> f32 {
        terrain::ground_memo(&mut self.lat, seed, haven, x, z)
    }

    /// The sim's ground at whole-metre point (`ix`, `iz`), resolved once per
    /// seed.
    #[inline]
    pub(crate) fn ground_at(&mut self, seed: u64, haven: &Haven, ix: i32, iz: i32) -> f32 {
        if self.seed != seed {
            self.reset(seed);
        }
        let key = point_key(ix, iz);
        let i = (key.wrapping_mul(2_654_435_761) >> self.face_shift) as usize;
        if self.heights[i].key == key {
            return self.heights[i].y;
        }
        let y = terrain::ground_memo(&mut self.lat, seed, haven, ix as f32, iz as f32);
        self.heights[i] = SurfLine { key, y };
        y
    }

    /// The faces' coarse point at whole-metre (`ix`, `iz`)
    /// (`face::coarse_at`), resolved once per seed.
    pub(crate) fn coarse(
        &mut self,
        seed: u64,
        haven: &Haven,
        ix: i32,
        iz: i32,
    ) -> crate::face::Coarse {
        if self.seed != seed {
            self.reset(seed);
        }
        let key = point_key(ix, iz);
        let i = (key.wrapping_mul(2_654_435_761) >> self.face_shift) as usize;
        if self.coarses[i].key == key {
            return self.coarses[i].c;
        }
        let c = crate::face::coarse_at(seed, haven, ix, iz, |i, j| {
            self.ground_at(seed, haven, i, j)
        });
        self.coarses[i] = CoarseLine { key, c };
        c
    }

    /// The skin's height at half-metre point (`hx`, `hz`) — (`hx / 2`,
    /// `hz / 2`) metres — (`face::skin_point`), resolved once per seed.
    pub(crate) fn skin(&mut self, seed: u64, haven: &Haven, hx: i32, hz: i32) -> f32 {
        if self.seed != seed {
            self.reset(seed);
        }
        let key = point_key(hx, hz);
        let i = (key.wrapping_mul(2_654_435_761) >> self.skin_shift) as usize;
        if self.skins[i].key == key {
            return self.skins[i].y;
        }
        let (x, z) = (hx as f32 * 0.5, hz as f32 * 0.5);
        let (x0, z0) = (floor_i32(x), floor_i32(z));
        let q = [
            self.coarse(seed, haven, x0, z0),
            self.coarse(seed, haven, x0 + 1, z0),
            self.coarse(seed, haven, x0, z0 + 1),
            self.coarse(seed, haven, x0 + 1, z0 + 1),
        ];
        let mut room = 0.0;
        if q.iter().any(|q| q.fade > 0.0) {
            room = 1.0f32;
            crate::cliff::spots_near(x, z, crate::face::CRAG_FADE_M, |gi, gj| {
                let c = self.crag(seed, haven, gi, gj);
                room = room.min(crate::face::crag_room([&c], x, z));
            });
        }
        let y = crate::face::skin_point(crate::face::seed_key(seed), x, z, &q, room).y;
        self.skins[i] = SurfLine { key, y };
        y
    }

    /// The solid surface at whole-metre (`ix`, `iz`) (`face::surface`),
    /// resolved once per seed.
    pub(crate) fn surf(&mut self, seed: u64, haven: &Haven, ix: i32, iz: i32) -> f32 {
        if self.seed != seed {
            self.reset(seed);
        }
        let key = point_key(ix, iz);
        let i = (key.wrapping_mul(2_654_435_761) >> self.face_shift) as usize;
        if self.surfs[i].key == key {
            return self.surfs[i].y;
        }
        // `face::surface` at a whole-metre point: its own coarse point and
        // the skin there, or the ground off the faces.
        let mut steep = false;
        let mut drawn = 0.0;
        for k in 0..4 {
            let c = self.coarse(seed, haven, ix + k % 2, iz + k / 2);
            steep |= c.gentlest > crate::face::SKIN_SLOPE_LO;
            if k == 0 {
                drawn = c.drawn;
            }
        }
        let y = if steep {
            drawn.max(self.skin(seed, haven, ix * 2, iz * 2))
        } else {
            drawn
        };
        self.surfs[i] = SurfLine { key, y };
        y
    }

    /// The face law at whole-metre (`ix`, `iz`) (`movement::face_fall`),
    /// resolved once per seed.
    pub(crate) fn fall(
        &mut self,
        seed: u64,
        haven: &Haven,
        ix: i32,
        iz: i32,
    ) -> Option<(f32, f32)> {
        if self.seed != seed {
            self.reset(seed);
        }
        let key = point_key(ix, iz);
        let i = (key.wrapping_mul(2_654_435_761) >> self.face_shift) as usize;
        if self.falls[i].key != key {
            let fall = crate::movement::fall_at(ix, iz, |i, j| self.surf(seed, haven, i, j));
            self.falls[i] = FallLine {
                key,
                fall: fall.unwrap_or((0.0, 0.0)),
            };
        }
        let fall = self.falls[i].fall;
        (fall != (0.0, 0.0)).then_some(fall)
    }

    /// The formation in rock cell (`bx`, `bz`), resolved once per seed.
    pub fn formation(
        &mut self,
        seed: u64,
        haven: &Haven,
        bx: i32,
        bz: i32,
    ) -> crate::boulder::Formation {
        if self.seed != seed {
            self.reset(seed);
        }
        let key = ((bx as u32 & 0xFFFF) << 16) | (bz as u32 & 0xFFFF);
        let ix = (key.wrapping_mul(2_654_435_761) >> 20) as usize & (ROCK_CACHE_LINES - 1);
        if self.rocks[ix].key == key {
            return self.rocks[ix].f;
        }
        let f = crate::boulder::formation(seed, haven, bx, bz);
        self.rocks[ix] = RockLine { key, f };
        f
    }

    /// The running count of `terrain::scatter` calls this cache has made.
    pub fn resolves(&self) -> u32 {
        self.resolves
    }

    /// Drop every line. Not a tick path — only a seed change reaches it.
    fn reset(&mut self, seed: u64) {
        self.seed = seed;
        for line in self.lines.iter_mut() {
            line.key = NO_KEY;
        }
        let mut i = 0;
        while i < ROCK_CACHE_LINES {
            self.rocks[i].key = NO_KEY;
            i += 1;
        }
        for line in self.crags.iter_mut() {
            line.key = NO_KEY;
        }
        for line in self.coarses.iter_mut() {
            line.key = NO_KEY;
        }
        for line in self
            .heights
            .iter_mut()
            .chain(self.surfs.iter_mut())
            .chain(self.skins.iter_mut())
        {
            line.key = NO_KEY;
        }
        for line in self.falls.iter_mut() {
            line.key = NO_KEY;
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
        let ix = (key.wrapping_mul(2_654_435_761) >> self.line_shift) as usize;
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
    /// The card doors open now (`monument::open_bits`): an open door's leaf
    /// does not collide.
    pub doors: u32,
}

impl Occupants<'_> {
    /// The highest rock crown under (`x`, `z`) within a step of `feet_y`, the
    /// rock grown by `r` — `rock_blocks`' own dilation for the same `r`, so a
    /// body is held up by exactly the rock that would stop it.
    fn rock_ground(&mut self, seed: u64, x: f32, z: f32, feet_y: f32, r: f32) -> f32 {
        let mut best = crate::collide::NO_SURFACE;
        let (cache, haven) = (&mut *self.cache, self.haven);
        let dil = ROCK_SKIN_M.max(r);
        crate::boulder::cells_near(x, z, dil, |bx, bz| {
            let f = cache.formation(seed, haven, bx, bz);
            for b in f.iter() {
                if let Some((s, _, _)) = crate::boulder::surface(b, x, z, dil) {
                    if s <= feet_y + crate::movement::STEP_UP && s > best {
                        best = s;
                    }
                }
            }
        });
        best
    }

    /// The highest cliff crag top under (`x`, `z`) within a step of
    /// `feet_y`, the crag grown by `r` — `rock_ground`'s rule.
    fn crag_ground(&mut self, seed: u64, x: f32, z: f32, feet_y: f32, r: f32) -> f32 {
        let mut best = crate::collide::NO_SURFACE;
        let (cache, haven) = (&mut *self.cache, self.haven);
        let dil = ROCK_SKIN_M.max(r);
        crate::cliff::spots_near(x, z, dil, |gi, gj| {
            let c = cache.crag(seed, haven, gi, gj);
            if !c.is_some() {
                return;
            }
            if let Some(s) = c.top(x, z, dil) {
                if s <= feet_y + crate::movement::STEP_UP && s > best {
                    best = s;
                }
            }
        });
        best
    }

    /// Whether a cliff crag stops a volume — `rock_blocks`' rule, with no
    /// underside: a crag is rock down into the face, and nothing passes
    /// under one.
    fn crag_blocks(&mut self, seed: u64, x: f32, z: f32, feet_y: f32, r: f32, h: f32) -> bool {
        let mut hit = false;
        let (cache, haven) = (&mut *self.cache, self.haven);
        let dil = ROCK_SKIN_M.max(r);
        crate::cliff::spots_near(x, z, dil, |gi, gj| {
            if hit {
                return;
            }
            let c = cache.crag(seed, haven, gi, gj);
            if !c.is_some() {
                return;
            }
            let Some(s) = c.top(x, z, dil) else {
                return;
            };
            if s > feet_y && (h < crate::movement::STEP_UP || s > feet_y + crate::movement::STEP_UP)
            {
                hit = true;
            }
        });
        hit
    }

    /// Whether a rock stops a volume of radius `r` and height `h` at
    /// (`x`, `z`) with its bottom at `feet_y`.
    ///
    /// A body (a volume a step or more tall) is stopped by rock more than a
    /// step above its feet and steps up onto anything lower, the rule the
    /// landmark and kit boxes keep. Until 2026-10-06 a block's side stopped
    /// it from 5 cm, so a body walking down a hill hung on the buried uphill
    /// edge of a block whose drawn top rounds into the ground there. Anything
    /// smaller (an arrow's probe) is stopped by being inside the rock at all.
    /// A block overhead, its bottom above the volume's top, is passed under.
    fn rock_blocks(&mut self, seed: u64, x: f32, z: f32, feet_y: f32, r: f32, h: f32) -> bool {
        let mut hit = false;
        let (cache, haven) = (&mut *self.cache, self.haven);
        let dil = ROCK_SKIN_M.max(r);
        crate::boulder::cells_near(x, z, dil, |bx, bz| {
            if hit {
                return;
            }
            let f = cache.formation(seed, haven, bx, bz);
            for b in f.iter() {
                let Some((s, _, bottom)) = crate::boulder::surface(b, x, z, dil) else {
                    continue;
                };
                if s <= feet_y || feet_y + h <= bottom {
                    continue;
                }
                // A short volume is stopped by any rock above its feet; a
                // body only by a rise it cannot step.
                if h < crate::movement::STEP_UP || s > feet_y + crate::movement::STEP_UP {
                    hit = true;
                    return;
                }
            }
        });
        hit
    }

    /// Does anything scattered stop a capsule standing at (`x`, `z`) with its
    /// feet at `feet_y`?
    ///
    /// The 3×3 scan is complete rather than approximate — see
    /// `terrain::OCCUPANT_PROBE_CELLS`, which proves it against the widest
    /// occupant plus a capsule against `CELL_SIZE` in a const block.
    pub fn blocks(&mut self, seed: u64, x: f32, z: f32, feet_y: f32) -> bool {
        self.blocks_volume(seed, x, z, feet_y, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M)
    }

    /// The highest scattered surface under the POINT (`x`, `z`) within a
    /// step of `feet_y` — where a dropped item comes to rest. A body asks
    /// [`Occupants::body_ground`].
    pub fn ground(&mut self, seed: u64, x: f32, z: f32, feet_y: f32) -> f32 {
        self.ground_volume(seed, x, z, feet_y, 0.0)
    }

    /// The highest scattered surface a capsule at `feet_y` may stand on at
    /// (`x`, `z`) — every top within a step that the capsule's own disc
    /// overlaps, which is the footprint [`Occupants::blocks`] stops it with.
    ///
    /// **One footprint for both questions, or a body falls into things.** It
    /// was the point under the centre until 2026-10-02 while `blocks` used
    /// the disc, so a body walking off a ledge left it with its centre past
    /// the edge and 0.4 m of itself still over the stone. It fell down the
    /// face overlapping it; once the face was more than a step above the
    /// feet `blocks` called the body *inside* it, the veto lift in
    /// `movement::step` let it walk anywhere, and pressing back toward the
    /// face walked it into a Black Ziggurat terrace — out again freely, back
    /// in never. With the disc a body stays on a top until it is clear of
    /// it, so it can only fall where nothing overlaps it.
    pub fn body_ground(&mut self, seed: u64, x: f32, z: f32, feet_y: f32) -> f32 {
        self.ground_volume(seed, x, z, feet_y, CAPSULE_RADIUS_M)
    }

    /// The highest scattered surface within a step of `feet_y` under a disc
    /// of radius `r` at (`x`, `z`), or `collide::NO_SURFACE` — `blocks`'s twin
    /// over [`terrain::slot_ground`], and the query that makes a crate top, a
    /// boulder top and the shelter's plinth ground instead of geometry a
    /// body sinks through (`NOW.md` §0q item 3).
    ///
    /// The same 3×3 scan, complete for the same reason: a ground footprint
    /// is the occupant's own grown by `r`, which reaches no further than
    /// the capsule-inflated blocking footprint the probe bound was proved
    /// against while `r` is at most the capsule's. Harvested is asked last
    /// and only for a slot that would otherwise answer, `blocks`'s own cost
    /// argument.
    fn ground_volume(&mut self, seed: u64, x: f32, z: f32, feet_y: f32, r: f32) -> f32 {
        debug_assert!(
            r <= CAPSULE_RADIUS_M,
            "the 3x3 probe is proved complete against the capsule radius"
        );
        let pcx = floor_i32(x / CELL_SIZE);
        let pcz = floor_i32(z / CELL_SIZE);
        let mut best = crate::depot::ground(self.haven, x, z, feet_y, r);
        // The rock formations' tops (`boulder.rs`): a block's top is ground
        // within a step of the feet, like any other occupant's lid. Its side
        // is a wall, which `blocks_volume` answers.
        best = best.max(self.rock_ground(seed, x, z, feet_y, r));
        best = best.max(self.crag_ground(seed, x, z, feet_y, r));
        best = best.max(crate::landmark::ground(&self.haven.marks, x, z, feet_y, r));
        best = best.max(crate::town::ground(&self.haven.town, x, z, feet_y, r));
        best = best.max(crate::monument::ground(
            &self.haven.ziggurat,
            self.doors,
            x,
            z,
            feet_y,
            r,
        ));
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
                let g = terrain::slot_ground(&slot, x, z, feet_y, r);
                if g <= best {
                    continue;
                }
                // A sapling stands at its own size (tree growth v0), which
                // sits inside the full one — so re-asking only after the
                // full-size answer keeps this as cheap as it was.
                let g = match slot_pm(self.harvested, seed, cx as u16, cz as u16, slot.occupant) {
                    0 => continue,
                    1000 => match self
                        .harvested
                        .ore_top_pm(cx as u16, cz as u16, slot.occupant)
                    {
                        1000 => g,
                        pm => terrain::slot_ground(&sunk(&slot, pm), x, z, feet_y, r),
                    },
                    pm => terrain::slot_ground(&grown(&slot, pm), x, z, feet_y, r),
                };
                if g <= best {
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
    /// The centre of a round scatter occupant (a trunk, a cactus, a node, a
    /// barrel) that stops a body at (`x`, `z`), or `None`. What
    /// `movement::step` slides a body round when walking into one head-on
    /// stops both of its axis slides: the boxed structures are walls, which
    /// those slides already follow.
    pub fn round_blocker(&mut self, seed: u64, x: f32, z: f32, feet_y: f32) -> Option<(f32, f32)> {
        let pcx = floor_i32(x / CELL_SIZE);
        let pcz = floor_i32(z / CELL_SIZE);
        let mut dz = -terrain::OCCUPANT_PROBE_CELLS;
        while dz <= terrain::OCCUPANT_PROBE_CELLS {
            let mut dx = -terrain::OCCUPANT_PROBE_CELLS;
            while dx <= terrain::OCCUPANT_PROBE_CELLS {
                let (cx, cz) = (pcx + dx, pcz + dz);
                dx += 1;
                let slot = self.cache.slot(seed, self.table, self.haven, cx, cz);
                if matches!(
                    slot.occupant,
                    Occupant::None
                        | Occupant::HavenShelter
                        | Occupant::WaystationCanopy
                        | Occupant::RoadSign
                        | Occupant::CarWreck
                ) || !terrain::slot_blocks(
                    &slot,
                    x,
                    z,
                    feet_y,
                    CAPSULE_RADIUS_M,
                    CAPSULE_HEIGHT_M,
                ) || slot_pm(self.harvested, seed, cx as u16, cz as u16, slot.occupant) == 0
                {
                    continue;
                }
                return Some((slot.x, slot.z));
            }
            dz += 1;
        }
        None
    }

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
        if crate::depot::blocks(self.haven, x, z, feet_y, r, h) {
            return true;
        }
        if self.rock_blocks(seed, x, z, feet_y, r, h) {
            return true;
        }
        if self.crag_blocks(seed, x, z, feet_y, r, h) {
            return true;
        }
        if crate::landmark::blocks(&self.haven.marks, x, z, feet_y, r, h) {
            return true;
        }
        if crate::poles::blocks(self.haven, x, z, r) {
            return true;
        }
        if crate::town::blocks(&self.haven.town, x, z, feet_y, r, h) {
            return true;
        }
        if crate::monument::blocks(&self.haven.ziggurat, self.doors, x, z, feet_y, r, h) {
            return true;
        }
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
                match slot_pm(self.harvested, seed, cx as u16, cz as u16, slot.occupant) {
                    0 => continue,
                    1000 => {
                        let pm = self
                            .harvested
                            .ore_top_pm(cx as u16, cz as u16, slot.occupant);
                        if pm < 1000 && !terrain::slot_blocks(&sunk(&slot, pm), x, z, feet_y, r, h)
                        {
                            continue;
                        }
                    }
                    pm => {
                        if !terrain::slot_blocks(&grown(&slot, pm), x, z, feet_y, r, h) {
                            continue;
                        }
                    }
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
                // A ring nothing asks about: this fixture is off the island.
                ring: terrain::RingPath::FLAT,
                x: -1.0e6,
                z: -1.0e6,
                y: 0.0,
                floor_y: 0.0,
                relief: 0.0,
                phase: 0,
                shelter: 0,
                // No lesser tier either: `Barren` has already made every slot
                // pass through, and an inert site is what `in_waystation`
                // tests for first.
                minor: terrain::empty_minor(),
                // No side road either, for the same reason.
                roads: [terrain::SideRoad::NONE; terrain::SIDE_ROADS],
                ore_pm: terrain::ORE_PM_UNIT,
                marks: crate::landmark::NO_MARKS,
                trails: [terrain::SideRoad::NONE; crate::landmark::LANDMARKS],
                town: crate::town::Town::NONE,
                ziggurat: crate::monument::Ziggurat::NONE,
                poles: crate::poles::Poles::NONE,
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
            doors: 0,
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
