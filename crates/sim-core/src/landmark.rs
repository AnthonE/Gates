//! Landmarks: the island's ruins, masts, towers, stone rings, container
//! yards and rock formations — anvils, arches, spires — places worth walking
//! to, readable from across the island.
//!
//! **Why a layer and not more `SiteKind`s.** The authored sites (the haven,
//! the waystations, the depot) are few, tiered and wired into the road
//! network and the loot ladder. A 4,096 m island needs more *places* than
//! that — Rust's own maps are dotted with small monuments between the big
//! ones (`reference/MONUMENTS.md`) — and each of these is a kit of boxes and
//! a handful of crates, which the depot already proved is all a monument
//! needs: its boxes are the collision (`blocks`, `ground`, the
//! `depot::blocks` shape), the client draws the same boxes
//! (`render/landmarks.rs`), and its crates are ordinary scatter slots seated
//! at authored anchors (`terrain::scatter`), so looting, respawn and the wire
//! are the ones every crate already has.
//!
//! **Placed once per seed, into `Haven`.** One landmark per 512 m cell at
//! most, kept off the roads, the sites, the water and the steep ground, and
//! its kind chosen by where it stands: a mast on a summit, a tower in the
//! woods, a yard or a stone ring in the open. `terrain::haven` solves the list
//! after the sites and roads exist, so every consumer that already holds a
//! `&Haven` — scatter, collision, the client — has them for free.
//!
//! Every number here is a first cut (operator, 2026-09-28: "more interesting
//! monuments").

use crate::fmath::floor_i32;
use crate::rng::cell_hash;
use crate::terrain::{self, Haven, Occupant};

/// Edge of one landmark cell, metres.
pub const LANDMARK_CELL_M: f32 = 512.0;
/// Landmark cells a side.
pub const LANDMARK_CELLS: usize = (terrain::ISLAND_SIZE / LANDMARK_CELL_M) as usize;
/// The most landmarks an island holds: one per cell.
pub const LANDMARKS: usize = LANDMARK_CELLS * LANDMARK_CELLS;
/// Radius every kit fits inside, metres — the veto and broad-phase disc.
pub const LANDMARK_R_M: f32 = 26.0;
/// Most crates one kit seats.
pub const LANDMARK_CRATES: usize = 3;
/// How far below its base every kit's footings run, metres, so a wall on a
/// slope is rooted on its low side rather than standing on air.
pub const FOOTING_M: f32 = 4.0;

const CH_LANDMARK: u32 = 192;

/// What a landmark is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LandmarkKind {
    /// A steel radio mast on a summit, 40 m tall, with its equipment hut.
    Mast = 0,
    /// A broken stone keep: roofless walls and a corner tower.
    Ruin = 1,
    /// A timber watchtower and a cabin in the woods.
    Tower = 2,
    /// A ring of standing stones around an altar.
    Stones = 3,
    /// A yard of stacked shipping containers.
    Yard = 4,
    /// A rock pillar under a cap far wider than it — the reference game's
    /// anvil and god rocks. The cap overhangs, and a body walks under it.
    Anvil = 5,
    /// Two rock buttresses and the slab across them: a gate in the rock.
    Arch = 6,
    /// A cluster of rock spires, the tallest capped.
    Spires = 7,
    /// A stone quarry cut into foothill rock: a stepped face, cut blocks,
    /// a crane and the cutters' shed. Dry ground between the lowlands and the
    /// summits (`try_site`).
    Quarry = 8,
    /// A roadside comms relay: a fenced pad, a 20 m lattice tower with its
    /// dish, a generator hut and a container. It serves a road, so it
    /// stands within sight of one.
    Relay = 9,
}

/// What a part is made of — the client picks a surface by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Mat {
    Stone = 0,
    Concrete = 1,
    Steel = 2,
    Timber = 3,
    Cargo = 4,
    /// Natural rock: the client draws it as a fractured rock
    /// (`render/boulders.rs`), not a box.
    Rock = 5,
}

impl Mat {
    /// The material role name the Blender dressing and the client share
    /// (`kit::KitMat::role`'s vocabulary).
    pub const fn role(self) -> &'static str {
        match self {
            Mat::Stone => "stone",
            Mat::Concrete => "concrete",
            Mat::Steel => "steel",
            Mat::Timber => "timber",
            Mat::Cargo => "cargo",
            Mat::Rock => "rock",
        }
    }
}

/// One box of a kit, in the landmark's own frame: `[x0, y0, z0, x1, y1, z1]`
/// metres, `y` from the landmark's base.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Part {
    pub b: [f32; 6],
    pub mat: Mat,
}

const fn p(x0: f32, y0: f32, z0: f32, x1: f32, y1: f32, z1: f32, mat: Mat) -> Part {
    Part {
        b: [x0, y0, z0, x1, y1, z1],
        mat,
    }
}

/// A crate anchor in the landmark's frame: `(x, z, what)`.
pub type Anchor = (f32, f32, Occupant);

const F: f32 = -FOOTING_M;

// ── The kits ────────────────────────────────────────────────────────────────

const MAST: &[Part] = &[
    // The pad and the hut.
    p(-6.0, F, -6.0, 6.0, 0.3, 6.0, Mat::Concrete),
    p(-14.0, F, -3.0, -8.0, 3.2, 3.0, Mat::Concrete),
    p(-14.4, 3.2, -3.4, -7.6, 3.5, 3.4, Mat::Steel),
    // Four legs.
    p(-1.9, 0.3, -1.9, -1.5, 36.0, -1.5, Mat::Steel),
    p(1.5, 0.3, -1.9, 1.9, 36.0, -1.5, Mat::Steel),
    p(-1.9, 0.3, 1.5, -1.5, 36.0, 1.9, Mat::Steel),
    p(1.5, 0.3, 1.5, 1.9, 36.0, 1.9, Mat::Steel),
    // Girders every nine metres, and a service platform.
    p(-1.9, 9.0, -1.9, 1.9, 9.3, -1.5, Mat::Steel),
    p(-1.9, 9.0, 1.5, 1.9, 9.3, 1.9, Mat::Steel),
    p(-1.9, 18.0, -1.9, 1.9, 18.3, -1.5, Mat::Steel),
    p(-1.9, 18.0, 1.5, 1.9, 18.3, 1.9, Mat::Steel),
    p(-2.8, 24.0, -2.8, 2.8, 24.3, 2.8, Mat::Steel),
    p(-1.9, 30.0, -1.9, 1.9, 30.3, 1.9, Mat::Steel),
    // The antenna and its dishes.
    p(-0.2, 36.0, -0.2, 0.2, 44.0, 0.2, Mat::Steel),
    p(-2.4, 26.0, 1.9, -0.6, 28.0, 2.3, Mat::Steel),
    p(0.8, 32.0, -2.3, 2.6, 33.6, -1.9, Mat::Steel),
    // Guy anchors.
    p(12.0, F, -1.0, 14.0, 0.8, 1.0, Mat::Concrete),
    p(-1.0, F, 12.0, 1.0, 0.8, 14.0, Mat::Concrete),
    p(-1.0, F, -14.0, 1.0, 0.8, -12.0, Mat::Concrete),
];
const MAST_CRATES: &[Anchor] = &[
    (-11.0, 7.0, Occupant::CacheSlot),
    (4.5, 9.0, Occupant::CrateSlot),
    (9.0, -9.0, Occupant::BarrelSlot),
];

const RUIN: &[Part] = &[
    // The keep's walls: a 20 m square, broken to different heights, a gate
    // gap in the south wall and a breach in the east.
    p(-10.0, F, -10.0, -3.0, 5.5, -8.8, Mat::Stone),
    p(3.0, F, -10.0, 10.0, 3.2, -8.8, Mat::Stone),
    p(-10.0, F, 8.8, 10.0, 7.0, 10.0, Mat::Stone),
    p(-10.0, F, -8.8, -8.8, 6.2, 8.8, Mat::Stone),
    p(8.8, F, -8.8, 10.0, 4.4, -1.5, Mat::Stone),
    p(8.8, F, 3.5, 10.0, 2.0, 8.8, Mat::Stone),
    // The corner tower, still standing.
    p(-13.0, F, 6.0, -6.0, 13.5, 13.0, Mat::Stone),
    p(-13.4, 13.5, 5.6, -5.6, 14.3, 13.4, Mat::Stone),
    // Fallen blocks in the court and outside the breach.
    p(2.0, F, 2.0, 4.5, 1.1, 3.6, Mat::Stone),
    p(12.0, F, 0.0, 14.5, 0.9, 2.2, Mat::Stone),
    p(-4.0, F, -4.5, -2.6, 0.8, -2.0, Mat::Stone),
];
const RUIN_CRATES: &[Anchor] = &[
    (-4.0, 4.0, Occupant::CacheSlot),
    (5.0, -4.5, Occupant::CrateSlot),
    (0.0, -15.0, Occupant::BarrelSlot),
];

const TOWER: &[Part] = &[
    // Four posts, the deck, the rail and the roof.
    p(-2.4, F, -2.4, -1.9, 12.0, -1.9, Mat::Timber),
    p(1.9, F, -2.4, 2.4, 12.0, -1.9, Mat::Timber),
    p(-2.4, F, 1.9, -1.9, 12.0, 2.4, Mat::Timber),
    p(1.9, F, 1.9, 2.4, 12.0, 2.4, Mat::Timber),
    p(-3.0, 9.0, -3.0, 3.0, 9.3, 3.0, Mat::Timber),
    p(-3.0, 9.3, -3.0, 3.0, 10.3, -2.8, Mat::Timber),
    p(-3.0, 9.3, 2.8, 3.0, 10.3, 3.0, Mat::Timber),
    p(-3.5, 12.0, -3.5, 3.5, 12.4, 3.5, Mat::Timber),
    p(-1.0, 12.4, -1.0, 1.0, 13.4, 1.0, Mat::Timber),
    // Cross-bracing low down.
    p(-2.4, 3.0, -2.2, 2.4, 3.3, -2.0, Mat::Timber),
    p(-2.4, 3.0, 2.0, 2.4, 3.3, 2.2, Mat::Timber),
    // The cabin.
    p(6.0, F, -3.0, 12.0, 3.0, 3.0, Mat::Timber),
    p(5.6, 3.0, -3.4, 12.4, 3.4, 3.4, Mat::Timber),
    // A woodpile.
    p(6.0, F, 4.5, 9.0, 1.2, 6.0, Mat::Timber),
];
const TOWER_CRATES: &[Anchor] = &[
    (0.0, 0.0, Occupant::CacheSlot),
    (9.0, -9.0, Occupant::BarrelSlot),
    (-9.0, 8.0, Occupant::BarrelSlot),
];

const STONES: &[Part] = &[
    // Eight places on a 12 m ring; the north one a trilithon — two uprights
    // side by side carrying a lintel. (It was one stone under a lintel
    // reaching for a neighbour 45° round the ring, its far end on air.)
    p(-2.6, F, 11.0, -1.0, 5.8, 12.2, Mat::Stone),
    p(1.0, F, 11.0, 2.6, 5.6, 12.2, Mat::Stone),
    p(7.6, F, 7.6, 9.0, 4.6, 8.8, Mat::Stone),
    p(11.0, F, -0.8, 12.2, 6.4, 0.8, Mat::Stone),
    p(7.6, F, -9.0, 9.0, 3.8, -7.6, Mat::Stone),
    p(-0.8, F, -12.2, 0.8, 5.2, -11.0, Mat::Stone),
    p(-9.0, F, -9.0, -7.6, 4.9, -7.6, Mat::Stone),
    p(-12.2, F, -0.8, -11.0, 6.1, 0.8, Mat::Stone),
    p(-9.0, F, 7.6, -7.6, 2.4, 9.0, Mat::Stone),
    p(-3.0, 5.6, 10.8, 3.0, 6.5, 12.4, Mat::Stone),
    // The altar.
    p(-1.8, F, -1.0, 1.8, 1.0, 1.0, Mat::Stone),
];
const STONES_CRATES: &[Anchor] = &[
    (0.0, 3.5, Occupant::CrateSlot),
    (5.0, -5.0, Occupant::CacheSlot),
    (-17.0, 0.0, Occupant::BarrelSlot),
];

const YARD: &[Part] = &[
    // The slab.
    p(-16.0, F, -11.0, 16.0, 0.2, 11.0, Mat::Concrete),
    // Containers, 6.1 × 2.6 × 2.4, some stacked.
    p(-14.0, F, -9.0, -7.9, 2.8, -6.6, Mat::Cargo),
    p(-14.0, 2.8, -9.0, -7.9, 5.4, -6.6, Mat::Cargo),
    p(-14.0, F, -5.6, -7.9, 2.8, -3.2, Mat::Cargo),
    p(-4.0, F, 5.0, 2.1, 2.8, 7.4, Mat::Cargo),
    p(-4.0, 2.8, 5.0, 2.1, 5.4, 7.4, Mat::Cargo),
    p(-4.0, 5.4, 5.0, 2.1, 8.0, 7.4, Mat::Cargo),
    p(5.0, F, -9.5, 7.4, 2.8, -3.4, Mat::Cargo),
    p(10.0, F, 1.0, 16.1, 2.8, 3.4, Mat::Cargo),
    p(10.5, 2.8, 1.5, 16.6, 5.4, 3.9, Mat::Cargo),
    // A gantry over the middle.
    p(-1.0, F, -9.0, -0.5, 9.0, -8.5, Mat::Steel),
    p(-1.0, F, 8.5, -0.5, 9.0, 9.0, Mat::Steel),
    p(-1.2, 9.0, -9.2, -0.3, 9.8, 9.2, Mat::Steel),
];
const YARD_CRATES: &[Anchor] = &[
    (-4.0, -3.0, Occupant::CacheSlot),
    (12.0, -6.0, Occupant::CacheSlot),
    (-10.0, 5.0, Occupant::CrateSlot),
];

const ANVIL: &[Part] = &[
    // The foot, broken boulders round it.
    p(-6.5, F, -5.0, 6.0, 2.5, 5.5, Mat::Rock),
    // The pillar, waisted, and the neck the cap sits on.
    p(-3.0, 2.0, -2.6, 3.0, 12.5, 2.6, Mat::Rock),
    p(-4.8, 12.0, -4.0, 4.6, 14.6, 4.0, Mat::Rock),
    // The cap: twice the pillar's width, overhanging on every side.
    p(-8.5, 14.0, -6.0, 8.0, 18.5, 6.5, Mat::Rock),
    // Fallen pieces.
    p(8.0, F, -3.5, 11.5, 1.8, 0.5, Mat::Rock),
    p(-11.5, F, 2.0, -8.5, 1.3, 5.0, Mat::Rock),
    p(2.0, F, 8.0, 4.5, 1.0, 10.0, Mat::Rock),
];
const ANVIL_CRATES: &[Anchor] = &[(0.0, -8.5, Occupant::CacheSlot)];

const ARCH: &[Part] = &[
    // The buttresses, and the slab across them 10 m up.
    p(-12.0, F, -4.0, -6.5, 11.0, 4.0, Mat::Rock),
    p(6.0, F, -3.5, 11.5, 10.5, 3.5, Mat::Rock),
    p(-12.5, 10.0, -3.4, 12.0, 14.0, 3.4, Mat::Rock),
    p(-5.0, 13.5, -2.6, 4.0, 15.5, 2.8, Mat::Rock),
    // Rubble in the gate and off its ends.
    p(-2.0, F, 4.5, 1.5, 1.2, 7.0, Mat::Rock),
    p(13.0, F, -2.0, 16.0, 2.0, 1.5, Mat::Rock),
    p(-16.5, F, 1.0, -13.5, 1.6, 4.5, Mat::Rock),
];
const ARCH_CRATES: &[Anchor] = &[
    (0.0, 0.0, Occupant::CrateSlot),
    (-3.0, -9.0, Occupant::BarrelSlot),
];

const SPIRES: &[Part] = &[
    p(-10.0, F, -3.0, -5.5, 12.0, 1.5, Mat::Rock),
    p(-2.5, F, 3.5, 2.5, 16.0, 8.0, Mat::Rock),
    p(-3.8, 15.0, 2.4, 3.8, 18.5, 9.2, Mat::Rock),
    p(5.0, F, -6.5, 9.5, 9.0, -2.0, Mat::Rock),
    p(0.5, F, -11.0, 4.0, 6.0, -7.5, Mat::Rock),
    p(-13.0, F, 6.0, -9.5, 5.0, 9.5, Mat::Rock),
    p(9.0, F, 5.0, 12.5, 3.5, 8.5, Mat::Rock),
];
const SPIRES_CRATES: &[Anchor] = &[
    (-1.0, -3.0, Occupant::CacheSlot),
    (8.0, 1.0, Occupant::BarrelSlot),
];

const QUARRY: &[Part] = &[
    // The cut: a high back face, a bench below it, and the two side walls,
    // all the hill's own rock.
    p(-16.0, F, 8.0, 16.0, 7.0, 12.0, Mat::Rock),
    p(-14.0, F, 4.0, 14.0, 3.0, 8.0, Mat::Rock),
    p(-18.0, F, -6.0, -14.0, 5.5, 12.0, Mat::Rock),
    p(14.0, F, -6.0, 18.0, 4.5, 12.0, Mat::Rock),
    // Cut blocks on the floor.
    p(-6.0, F, -4.0, -3.8, 1.5, -2.0, Mat::Stone),
    p(-3.0, F, -6.5, -0.8, 1.2, -4.5, Mat::Stone),
    p(5.0, F, -3.5, 7.6, 1.0, -1.0, Mat::Stone),
    p(1.0, F, 0.5, 3.2, 2.2, 2.5, Mat::Stone),
    // The crane: a mast and its jib over the floor.
    p(9.6, F, -9.6, 10.4, 12.0, -8.8, Mat::Steel),
    p(2.0, 11.4, -9.5, 11.0, 12.0, -8.9, Mat::Steel),
    // The cutters' shed, and its roof.
    p(-14.0, F, -18.0, -6.0, 3.0, -12.0, Mat::Timber),
    p(-14.5, 3.0, -18.5, -5.5, 3.3, -11.5, Mat::Timber),
];
const QUARRY_CRATES: &[Anchor] = &[
    (-10.0, -9.0, Occupant::CrateSlot),
    (6.0, -14.0, Occupant::BarrelSlot),
    (0.0, -2.0, Occupant::BarrelSlot),
];

const RELAY: &[Part] = &[
    // The pad, and its fence with a gate gap in the south side.
    p(-9.0, F, -9.0, 9.0, 0.25, 9.0, Mat::Concrete),
    p(-9.0, 0.25, 8.8, 9.0, 2.0, 9.0, Mat::Steel),
    p(-9.0, 0.25, -9.0, -2.0, 2.0, -8.8, Mat::Steel),
    p(2.0, 0.25, -9.0, 9.0, 2.0, -8.8, Mat::Steel),
    p(-9.0, 0.25, -8.8, -8.8, 2.0, 8.8, Mat::Steel),
    p(8.8, 0.25, -8.8, 9.0, 2.0, 8.8, Mat::Steel),
    // The tower: four legs, two platforms, the antenna and a dish.
    p(1.8, 0.25, 1.8, 2.2, 20.0, 2.2, Mat::Steel),
    p(3.8, 0.25, 1.8, 4.2, 20.0, 2.2, Mat::Steel),
    p(1.8, 0.25, 3.8, 2.2, 20.0, 4.2, Mat::Steel),
    p(3.8, 0.25, 3.8, 4.2, 20.0, 4.2, Mat::Steel),
    p(1.8, 10.0, 1.8, 4.2, 10.3, 4.2, Mat::Steel),
    p(1.5, 20.0, 1.5, 4.5, 20.3, 4.5, Mat::Steel),
    p(2.85, 20.3, 2.85, 3.15, 26.0, 3.15, Mat::Steel),
    p(4.2, 16.0, 2.4, 4.6, 17.6, 3.6, Mat::Steel),
    // The generator hut and its roof, and a container by the gate.
    p(-7.5, 0.25, 2.0, -2.5, 3.0, 7.0, Mat::Concrete),
    p(-7.8, 3.0, 1.7, -2.2, 3.25, 7.3, Mat::Steel),
    p(-7.5, 0.25, -7.0, -5.1, 2.6, -1.0, Mat::Cargo),
];
const RELAY_CRATES: &[Anchor] = &[
    (-4.0, -5.0, Occupant::CrateSlot),
    (6.0, -6.0, Occupant::BarrelSlot),
];

/// The boxes of a kind.
pub const fn parts(kind: LandmarkKind) -> &'static [Part] {
    match kind {
        LandmarkKind::Mast => MAST,
        LandmarkKind::Ruin => RUIN,
        LandmarkKind::Tower => TOWER,
        LandmarkKind::Stones => STONES,
        LandmarkKind::Yard => YARD,
        LandmarkKind::Anvil => ANVIL,
        LandmarkKind::Arch => ARCH,
        LandmarkKind::Spires => SPIRES,
        LandmarkKind::Quarry => QUARRY,
        LandmarkKind::Relay => RELAY,
    }
}

/// The crate anchors of a kind.
pub const fn anchors(kind: LandmarkKind) -> &'static [Anchor] {
    match kind {
        LandmarkKind::Mast => MAST_CRATES,
        LandmarkKind::Ruin => RUIN_CRATES,
        LandmarkKind::Tower => TOWER_CRATES,
        LandmarkKind::Stones => STONES_CRATES,
        LandmarkKind::Yard => YARD_CRATES,
        LandmarkKind::Anvil => ANVIL_CRATES,
        LandmarkKind::Arch => ARCH_CRATES,
        LandmarkKind::Spires => SPIRES_CRATES,
        LandmarkKind::Quarry => QUARRY_CRATES,
        LandmarkKind::Relay => RELAY_CRATES,
    }
}

/// The built kinds — the ones `ci/site_kit.py` dresses into
/// `assets/models/site/mark_<slug>.glb`. The rock kinds are drawn by the
/// boulders' builder instead.
pub const DRESSED: [LandmarkKind; 5] = [
    LandmarkKind::Mast,
    LandmarkKind::Ruin,
    LandmarkKind::Tower,
    LandmarkKind::Stones,
    LandmarkKind::Yard,
];

/// A kind's file-name stem.
pub const fn slug(kind: LandmarkKind) -> &'static str {
    match kind {
        LandmarkKind::Mast => "mast",
        LandmarkKind::Ruin => "ruin",
        LandmarkKind::Tower => "tower",
        LandmarkKind::Stones => "stones",
        LandmarkKind::Yard => "yard",
        LandmarkKind::Anvil => "anvil",
        LandmarkKind::Arch => "arch",
        LandmarkKind::Spires => "spires",
        LandmarkKind::Quarry => "quarry",
        LandmarkKind::Relay => "relay",
    }
}

/// A kind's boxes and crate anchors as the Blender dresser's kit JSON
/// (`kit::dump`'s shape), named `mark_<slug>`; `ci/kits/mark_<slug>.json`.
pub fn dump(kind: LandmarkKind, w: &mut impl core::fmt::Write) -> core::fmt::Result {
    write!(w, "{{\"name\":\"mark_{}\",\"parts\":[", slug(kind))?;
    for (i, p) in parts(kind).iter().enumerate() {
        if i > 0 {
            w.write_str(",")?;
        }
        let b = p.b;
        write!(
            w,
            "\n{{\"b\":[{},{},{},{},{},{}],\"mat\":\"{}\",\"flags\":0,\"door\":0}}",
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            p.mat.role()
        )?;
    }
    w.write_str("],\"anchors\":[")?;
    for (i, (x, z, _)) in anchors(kind).iter().enumerate() {
        if i > 0 {
            w.write_str(",")?;
        }
        write!(w, "\n{{\"kind\":\"crate\",\"at\":[{x},0,{z}]}}")?;
    }
    w.write_str("]}\n")
}

/// A kind's name, for the map.
pub const fn name(kind: LandmarkKind) -> &'static str {
    match kind {
        LandmarkKind::Mast => "Radio Mast",
        LandmarkKind::Ruin => "Old Keep",
        LandmarkKind::Tower => "Watchtower",
        LandmarkKind::Stones => "Standing Stones",
        LandmarkKind::Yard => "Container Yard",
        LandmarkKind::Anvil => "Anvil Rock",
        LandmarkKind::Arch => "Arch Rock",
        LandmarkKind::Spires => "The Spires",
        LandmarkKind::Quarry => "Stone Quarry",
        LandmarkKind::Relay => "Relay Station",
    }
}

const _: () = {
    // Every kit and every anchor inside the disc the vetoes and the
    // broad phase use.
    let kinds = [
        LandmarkKind::Mast,
        LandmarkKind::Ruin,
        LandmarkKind::Tower,
        LandmarkKind::Stones,
        LandmarkKind::Yard,
        LandmarkKind::Anvil,
        LandmarkKind::Arch,
        LandmarkKind::Spires,
        LandmarkKind::Quarry,
        LandmarkKind::Relay,
    ];
    let mut k = 0;
    while k < kinds.len() {
        let ps = parts(kinds[k]);
        let mut i = 0;
        while i < ps.len() {
            let b = ps[i].b;
            assert!(b[0] < b[3] && b[1] < b[4] && b[2] < b[5]);
            let mut c = 0;
            while c < 4 {
                let x = if c & 1 == 0 { b[0] } else { b[3] };
                let z = if c & 2 == 0 { b[2] } else { b[5] };
                assert!(x * x + z * z <= LANDMARK_R_M * LANDMARK_R_M);
                c += 1;
            }
            i += 1;
        }
        let a = anchors(kinds[k]);
        assert!(a.len() <= LANDMARK_CRATES);
        let mut i = 0;
        while i < a.len() {
            assert!(a[i].0 * a[i].0 + a[i].1 * a[i].1 <= LANDMARK_R_M * LANDMARK_R_M);
            i += 1;
        }
        k += 1;
    }
    assert!(LANDMARK_CELLS * LANDMARK_CELLS == LANDMARKS);
};

/// One placed landmark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landmark {
    pub x: f32,
    pub z: f32,
    /// The base every part's `y` is measured from.
    pub y: f32,
    /// Bearing, as the high byte of a yaw-LUT index.
    pub yaw: u8,
    pub kind: LandmarkKind,
    pub live: bool,
}

impl Landmark {
    pub const NONE: Landmark = Landmark {
        x: 0.0,
        z: 0.0,
        y: 0.0,
        yaw: 0,
        kind: LandmarkKind::Ruin,
        live: false,
    };
}

/// An island with no landmarks — fixtures, and `Haven`s built before the
/// solve.
pub const NO_MARKS: [Landmark; LANDMARKS] = [Landmark::NONE; LANDMARKS];

/// Local +Z is the landmark's bearing; the depot's convention.
pub fn to_world(m: &Landmark, x: f32, z: f32) -> (f32, f32) {
    let (s, c) = crate::yaw_dir((m.yaw as u16) << 8);
    (m.x + x * c + z * s, m.z - x * s + z * c)
}

pub fn to_local(m: &Landmark, x: f32, z: f32) -> (f32, f32) {
    let (s, c) = crate::yaw_dir((m.yaw as u16) << 8);
    let (dx, dz) = (x - m.x, z - m.z);
    (dx * c - dz * s, dx * s + dz * c)
}

fn near(m: &Landmark, x: f32, z: f32, pad: f32) -> bool {
    let (dx, dz) = (x - m.x, z - m.z);
    let r = LANDMARK_R_M + pad;
    m.live && dx * dx + dz * dz <= r * r
}

/// Whether any landmark's disc, padded by `pad`, covers (`x`, `z`).
pub fn covers(marks: &[Landmark], x: f32, z: f32, pad: f32) -> bool {
    marks.iter().any(|m| near(m, x, z, pad))
}

/// Whether a landmark's boxes stop a volume of radius `r`, height `h`, feet
/// at `feet` — `depot::blocks`'s shape.
pub fn blocks(marks: &[Landmark], x: f32, z: f32, feet: f32, r: f32, h: f32) -> bool {
    for m in marks {
        if !near(m, x, z, r) {
            continue;
        }
        let (lx, lz) = to_local(m, x, z);
        for part in parts(m.kind) {
            let b = part.b;
            // A top within a step of the feet is ground, not a wall
            // (`ground` below answers it).
            if feet + crate::movement::STEP_UP >= m.y + b[4] || feet + h <= m.y + b[1] {
                continue;
            }
            let dx = lx - lx.clamp(b[0], b[3]);
            let dz = lz - lz.clamp(b[2], b[5]);
            if dx * dx + dz * dz < r * r {
                return true;
            }
        }
    }
    false
}

/// The highest landmark surface within a step of `feet` under a disc of
/// radius `r` at (`x`, `z`) — `r` 0 is the point; a body passes the capsule
/// radius, the footprint `blocks` stops it with (`kit::ground_local`'s
/// reason).
pub fn ground(marks: &[Landmark], x: f32, z: f32, feet: f32, r: f32) -> f32 {
    let mut best = crate::collide::NO_SURFACE;
    for m in marks {
        if !near(m, x, z, r) {
            continue;
        }
        let (lx, lz) = to_local(m, x, z);
        for part in parts(m.kind) {
            let b = part.b;
            let top = m.y + b[4];
            let dx = lx - lx.clamp(b[0], b[3]);
            let dz = lz - lz.clamp(b[2], b[5]);
            if dx * dx + dz * dz <= r * r && top <= feet + crate::movement::STEP_UP {
                best = best.max(top);
            }
        }
    }
    best
}

/// Crate `k` of a landmark in world space: `(x, z, yaw, occupant)`.
pub fn anchor(m: &Landmark, k: usize) -> Option<(f32, f32, u8, Occupant)> {
    let a = anchors(m.kind).get(k)?;
    let (x, z) = to_world(m, a.0, a.1);
    Some((x, z, m.yaw.wrapping_add((k as u8).wrapping_mul(71)), a.2))
}

#[inline]
fn unit(h: u64, shift: u32) -> f32 {
    ((h >> shift) & 0xFFFF) as f32 * (1.0 / 65_536.0)
}

/// Clearance kept from a road's centreline, beyond the kit's own disc.
const ROAD_CLEAR_M: f32 = 18.0;
/// Clearance kept from a site's blend radius.
const SITE_CLEAR_M: f32 = 40.0;
/// Share of land cells that hold a landmark.
const DENSITY: f32 = 0.7;
/// A relay stands no further than this from a road's centreline (and no
/// nearer than the landmark's own road clearance).
const RELAY_ROAD_M: f32 = LANDMARK_R_M + ROAD_CLEAR_M + 50.0;
/// A quarry's foothills start this high.
const QUARRY_MIN_H: f32 = 26.0;

/// Every landmark on the island, solved once per seed against the sites and
/// roads already in `pad`.
pub fn solve(seed: u64, pad: &Haven) -> [Landmark; LANDMARKS] {
    let mut out = NO_MARKS;
    let mut lat = terrain::Lattice::new();
    for cz in 0..LANDMARK_CELLS {
        for cx in 0..LANDMARK_CELLS {
            let h = cell_hash(seed, cx as i32, cz as i32, CH_LANDMARK);
            if unit(h, 0) >= DENSITY {
                continue;
            }
            // A few tries at a spot inside the cell's middle.
            let mut t = 0u32;
            while t < 6 {
                let ht = cell_hash(seed, cx as i32, cz as i32, CH_LANDMARK + 1 + t);
                t += 1;
                let margin = LANDMARK_R_M + 40.0;
                let span = LANDMARK_CELL_M - 2.0 * margin;
                let x = cx as f32 * LANDMARK_CELL_M + margin + unit(ht, 0) * span;
                let z = cz as f32 * LANDMARK_CELL_M + margin + unit(ht, 16) * span;
                let Some(m) = try_site(seed, pad, &mut lat, x, z, ht) else {
                    continue;
                };
                out[cz * LANDMARK_CELLS + cx] = m;
                break;
            }
        }
    }
    out
}

fn try_site(
    seed: u64,
    pad: &Haven,
    lat: &mut terrain::Lattice,
    x: f32,
    z: f32,
    h: u64,
) -> Option<Landmark> {
    let y0 = terrain::height_memo(lat, seed, x, z);
    if y0 < terrain::BEACH_MAX_H + 3.0 {
        return None;
    }
    // Level enough to build on: the ground across the kit's disc may not
    // spread more than a few metres.
    let k = LANDMARK_R_M * 0.75;
    let (mut lo, mut hi) = (y0, y0);
    for (ox, oz) in [
        (k, 0.0),
        (-k, 0.0),
        (0.0, k),
        (0.0, -k),
        (k * 0.7, k * 0.7),
        (-k * 0.7, -k * 0.7),
    ] {
        let y = terrain::height_memo(lat, seed, x + ox, z + oz);
        lo = lo.min(y);
        hi = hi.max(y);
    }
    // Built on the centre's height: the footings reach `FOOTING_M` down, so
    // the low side may fall away that far less a margin, and the high side
    // may bury a ground-floor part by no more than a door's height.
    if y0 - lo > FOOTING_M - 0.5 || hi - y0 > 2.5 || lo < terrain::LAND_MIN_H + 1.0 {
        return None;
    }
    // Off every road and site.
    let road = LANDMARK_R_M + ROAD_CLEAR_M;
    if pad.ring.dist2(x, z) < road * road {
        return None;
    }
    for sr in pad.roads.iter() {
        if sr.live && sr.dist2(x, z) < road * road {
            return None;
        }
    }
    let clear = |sx: f32, sz: f32, fp: &terrain::SiteFootprint| {
        let d = fp.blend_m + SITE_CLEAR_M + LANDMARK_R_M;
        let (dx, dz) = (x - sx, z - sz);
        dx * dx + dz * dz >= d * d
    };
    if !clear(pad.x, pad.z, &terrain::HAVEN_FOOTPRINT) {
        return None;
    }
    for ws in pad.minor.iter() {
        if ws.live && !clear(ws.x, ws.z, terrain::site_footprint(ws.kind)) {
            return None;
        }
    }
    if pad.town.live && !clear(pad.town.x, pad.town.z, &crate::town::TOWN_FOOTPRINT) {
        return None;
    }
    if pad.ziggurat.live
        && !clear(
            pad.ziggurat.x,
            pad.ziggurat.z,
            &crate::monument::ZIG_FOOTPRINT,
        )
    {
        return None;
    }
    // The kind by where it stands.
    let moist = terrain::moisture_memo(lat, seed, x, z);
    let roll = unit(h, 32);
    // How far the nearest road runs, for the relay, which serves one.
    let mut road_d2 = pad.ring.dist2(x, z);
    for sr in pad.roads.iter() {
        if sr.live {
            road_d2 = road_d2.min(sr.dist2(x, z));
        }
    }
    let kind = if road_d2 <= RELAY_ROAD_M * RELAY_ROAD_M && y0 <= 42.0 && unit(h, 40) < 0.5 {
        // Within sight of a road and off the summits: the relay it serves.
        LandmarkKind::Relay
    } else if (QUARRY_MIN_H..=42.0).contains(&y0) && moist <= 0.05 && unit(h, 40) < 0.45 {
        // Dry foothills: the rock is near the surface and nothing grows on it.
        LandmarkKind::Quarry
    } else if y0 > 42.0 {
        if roll < 0.4 {
            LandmarkKind::Mast
        } else if roll < 0.6 {
            LandmarkKind::Stones
        } else if roll < 0.8 {
            LandmarkKind::Anvil
        } else {
            LandmarkKind::Spires
        }
    } else if moist > 0.05 {
        if roll < 0.4 {
            LandmarkKind::Tower
        } else if roll < 0.7 {
            LandmarkKind::Ruin
        } else if roll < 0.85 {
            LandmarkKind::Arch
        } else {
            LandmarkKind::Mast
        }
    } else if roll < 0.22 {
        LandmarkKind::Yard
    } else if roll < 0.4 {
        LandmarkKind::Ruin
    } else if roll < 0.52 {
        LandmarkKind::Stones
    } else if roll < 0.64 {
        LandmarkKind::Mast
    } else if roll < 0.76 {
        LandmarkKind::Arch
    } else if roll < 0.88 {
        LandmarkKind::Anvil
    } else {
        LandmarkKind::Spires
    };
    Some(Landmark {
        x,
        z,
        y: y0,
        yaw: (h >> 48) as u8,
        kind,
        live: true,
    })
}

/// The landmark whose disc (padded by `pad`) holds (`x`, `z`), if any. Cells
/// are 512 m and a disc 26 m, so only the point's own cell can.
pub fn at(marks: &[Landmark], x: f32, z: f32, pad: f32) -> Option<&Landmark> {
    let cx = floor_i32(x / LANDMARK_CELL_M);
    let cz = floor_i32(z / LANDMARK_CELL_M);
    if cx < 0 || cz < 0 || cx as usize >= LANDMARK_CELLS || cz as usize >= LANDMARK_CELLS {
        return None;
    }
    let m = &marks[cz as usize * LANDMARK_CELLS + cx as usize];
    near(m, x, z, pad).then_some(m)
}
