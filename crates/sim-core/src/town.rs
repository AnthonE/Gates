//! The town — "THE GATE" on the map: the island's safe hub, a walled scrap
//! town of stacked containers and sheet metal built around a ruined ancient
//! gate (WORLD.md §0/§3), in the central plain.
//!
//! Rust's Outpost is the reference for what it holds (Workbench 1, a research
//! table, recyclers, vendors; `reference` research 2026-09-30): the public
//! stations are owner-0 deployables seeded at its anchors (`world.rs`,
//! `seed_authored`), the vendors trade at its kiosks (`vend.rs`), and inside
//! [`safe`] no player is hurt (`combat::hurt`).
//!
//! Placement is `terrain::solve_town`: after the lesser sites so the depot and
//! the waystations stay put, on a 24 m lattice (the lcm of the 3 m build cell
//! and the 8 m scatter cell) with an exact quarter turn, so every station
//! anchor is a build-cell centre in the world whatever the rotation.

use crate::kit::{self, part, part_f, KitMat as M, KitPart, Placed, DECOR, ROOF, STACK};
use crate::terrain::SiteFootprint;

/// The town's lattice, metres: the centre snaps to a multiple of this.
pub const TOWN_SNAP_M: f32 = 24.0;
/// How far from the island's centre the town may stand, metres — the central
/// plain inside the massifs' ring (`terrain.rs`, the 340–1,060 m annulus).
pub const TOWN_R_MAX: f32 = 480.0;
/// Where a gate's road starts, metres out from the centre along a local axis.
pub const PORT_R: f32 = 46.0;
/// Half the side of the safe square, metres: the walls (42) plus a margin.
pub const SAFE_HALF_M: f32 = 48.0;
/// Half the side of the square nothing may be built in, metres.
pub const RESERVE_HALF_M: f32 = 60.0;
/// What the town's public recyclers pay, per cent of a player's own —
/// Rust's safe-zone 40% against 60%.
pub const PUBLIC_RECYCLE_PCT: u32 = 67;
/// Roads the town solves: one opposite pair of gates.
pub const TOWN_ROADS: usize = 2;

/// The town's masks. The floor covers the compound's corners (42·√2 ≈ 59.4 m)
/// with a clutter cell to spare, and the blend runs long and shallow.
pub const TOWN_FOOTPRINT: SiteFootprint = SiteFootprint {
    swept_m: 60.0,
    stamp_m: 62.0,
    scatter_m: 70.0,
    blend_m: 110.0,
};

/// One placed town. `live` false on a seed with nowhere to put it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Town {
    pub x: f32,
    pub z: f32,
    /// Raw ground at the centre, the solver's input.
    pub y: f32,
    /// The carved floor, a multiple of 0.5 m.
    pub floor_y: f32,
    pub relief: f32,
    /// Quarter turns: local +Z (the market street) faces yaw index `rot * 64`.
    pub rot: u8,
    pub live: bool,
}

impl Town {
    pub const NONE: Town = Town {
        x: 0.0,
        z: 0.0,
        y: 0.0,
        floor_y: 0.0,
        relief: 0.0,
        rot: 0,
        live: false,
    };

    pub fn placed(&self) -> Placed {
        Placed {
            x: self.x,
            z: self.z,
            floor_y: self.floor_y,
            rot: self.rot,
        }
    }
}

const W: f32 = 5.2; // two containers
const F: f32 = -2.0; // footing below the floor

/// The kit. Local metres, +Z the market street's north gate.
pub const PARTS: &[KitPart] = &[
    // The yard.
    part([-42.0, -0.3, -42.0, 42.0, 0.0, 42.0], M::Yard),
    // Container walls, two high, broken by four gates.
    part_f([-42.0, F, 39.6, -5.0, W, 42.0], M::Cargo, STACK),
    part_f([5.0, F, 39.6, 42.0, W, 42.0], M::Cargo, STACK),
    part_f([-42.0, F, -42.0, -5.0, W, -39.6], M::Cargo, STACK),
    part_f([5.0, F, -42.0, 42.0, W, -39.6], M::Cargo, STACK),
    part_f([-42.0, F, 4.0, -39.6, W, 39.6], M::Cargo, STACK),
    part_f([-42.0, F, -39.6, -39.6, W, -4.0], M::Cargo, STACK),
    part_f([39.6, F, 4.0, 42.0, W, 39.6], M::Cargo, STACK),
    part_f([39.6, F, -39.6, 42.0, W, -4.0], M::Cargo, STACK),
    // Gate posts and the sign beams.
    part([-6.0, F, 39.2, -5.0, 7.4, 42.4], M::Steel),
    part([5.0, F, 39.2, 6.0, 7.4, 42.4], M::Steel),
    part([-6.0, 6.2, 40.2, 6.0, 7.4, 41.4], M::Steel),
    part([-6.0, F, -42.4, -5.0, 7.4, -39.2], M::Steel),
    part([5.0, F, -42.4, 6.0, 7.4, -39.2], M::Steel),
    part([-6.0, 6.2, -41.4, 6.0, 7.4, -40.2], M::Steel),
    part([39.2, F, -5.0, 42.4, 7.4, -4.0], M::Steel),
    part([39.2, F, 4.0, 42.4, 7.4, 5.0], M::Steel),
    part([40.2, 6.2, -5.0, 41.4, 7.4, 5.0], M::Steel),
    part([-42.4, F, -5.0, -39.2, 7.4, -4.0], M::Steel),
    part([-42.4, F, 4.0, -39.2, 7.4, 5.0], M::Steel),
    part([-41.4, 6.2, -5.0, -40.2, 7.4, 5.0], M::Steel),
    // The gate: a dais either side of the street, two pylons, gilt caps.
    part([-14.0, 0.0, -6.0, -5.5, 0.5, 6.0], M::Obsidian),
    part([5.5, 0.0, -6.0, 14.0, 0.5, 6.0], M::Obsidian),
    part([-11.0, F, -2.0, -8.0, 30.0, 2.0], M::Obsidian),
    part([8.0, F, -2.0, 11.0, 30.0, 2.0], M::Obsidian),
    part([-11.4, 30.0, -2.4, -7.6, 31.0, 2.4], M::Gilt),
    part([7.6, 30.0, -2.4, 11.4, 31.0, 2.4], M::Gilt),
    // The broken ring between the pylons — drawn, never collided: segments
    // of a 7.6 m circle about (0, 18) in the XY plane, its lower arc fallen.
    part_f([6.4, 16.8, -0.6, 8.8, 19.2, 0.6], M::Gilt, DECOR),
    part_f([5.38, 20.6, -0.6, 7.78, 23.0, 0.6], M::Gilt, DECOR),
    part_f([2.6, 23.38, -0.6, 5.0, 25.78, 0.6], M::Gilt, DECOR),
    part_f([-1.2, 24.4, -0.6, 1.2, 26.8, 0.6], M::Gilt, DECOR),
    part_f([-5.0, 23.38, -0.6, -2.6, 25.78, 0.6], M::Gilt, DECOR),
    part_f([-7.78, 20.6, -0.6, -5.38, 23.0, 0.6], M::Gilt, DECOR),
    part_f([-8.8, 16.8, -0.6, -6.4, 19.2, 0.6], M::Gilt, DECOR),
    part_f([-7.78, 13.0, -0.6, -5.38, 15.4, 0.6], M::Gilt, DECOR),
    part_f([5.38, 13.0, -0.6, 7.78, 15.4, 0.6], M::Gilt, DECOR),
    // Fallen ring fragments and pylon blocks in the plaza.
    part([16.0, -1.0, -12.0, 22.0, 1.8, -7.0], M::Gilt),
    part([-22.0, -1.0, -10.0, -16.0, 2.2, -6.0], M::Obsidian),
    part([12.0, -1.0, 8.0, 16.0, 1.2, 10.5], M::Obsidian),
    // Market: six container kiosks opening on the street, with counters.
    part([-8.44, 0.0, 11.0, -6.0, 2.6, 17.1], M::Cargo),
    part([-8.44, 0.0, 19.0, -6.0, 2.6, 25.1], M::Cargo),
    part([-8.44, 0.0, 27.0, -6.0, 2.6, 33.1], M::Cargo),
    part([6.0, 0.0, 11.0, 8.44, 2.6, 17.1], M::Cargo),
    part([6.0, 0.0, 19.0, 8.44, 2.6, 25.1], M::Cargo),
    part([6.0, 0.0, 27.0, 8.44, 2.6, 33.1], M::Cargo),
    part([-6.0, 0.0, 11.6, -5.4, 1.1, 16.5], M::Timber),
    part([-6.0, 0.0, 19.6, -5.4, 1.1, 24.5], M::Timber),
    part([-6.0, 0.0, 27.6, -5.4, 1.1, 32.5], M::Timber),
    part([5.4, 0.0, 11.6, 6.0, 1.1, 16.5], M::Timber),
    part([5.4, 0.0, 19.6, 6.0, 1.1, 24.5], M::Timber),
    part([5.4, 0.0, 27.6, 6.0, 1.1, 32.5], M::Timber),
    part_f([-8.6, 2.7, 11.0, -4.2, 2.9, 17.1], M::Canvas, DECOR | ROOF),
    part_f([-8.6, 2.7, 19.0, -4.2, 2.9, 25.1], M::Canvas, DECOR | ROOF),
    part_f([-8.6, 2.7, 27.0, -4.2, 2.9, 33.1], M::Canvas, DECOR | ROOF),
    part_f([4.2, 2.7, 11.0, 8.6, 2.9, 17.1], M::Canvas, DECOR | ROOF),
    part_f([4.2, 2.7, 19.0, 8.6, 2.9, 25.1], M::Canvas, DECOR | ROOF),
    part_f([4.2, 2.7, 27.0, 8.6, 2.9, 33.1], M::Canvas, DECOR | ROOF),
    // Container stacks behind the market.
    part([-24.0, 0.0, 20.0, -17.9, 2.6, 22.44], M::Cargo),
    part([-24.0, 2.6, 20.0, -17.9, 5.2, 22.44], M::Cargo),
    part([-30.0, 0.0, 30.0, -27.56, 2.6, 36.1], M::Cargo),
    part([18.0, 0.0, 24.0, 24.1, 2.6, 26.44], M::Cargo),
    part([26.0, 0.0, 30.0, 28.44, 2.6, 36.1], M::Cargo),
    part([26.0, 2.6, 30.0, 28.44, 5.2, 36.1], M::Cargo),
    // Workshop (SW): a roof on posts over the public stations, and their
    // plinths (a station stands at its column's band, floor + 0.5).
    part_f([-34.0, 3.5, -34.0, -16.0, 3.8, -18.0], M::Sheet, ROOF),
    part([-34.0, 0.0, -34.0, -33.7, 3.5, -33.7], M::Steel),
    part([-16.3, 0.0, -34.0, -16.0, 3.5, -33.7], M::Steel),
    part([-34.0, 0.0, -18.3, -33.7, 3.5, -18.0], M::Steel),
    part([-16.3, 0.0, -18.3, -16.0, 3.5, -18.0], M::Steel),
    part([-25.15, 0.0, -34.0, -24.85, 3.5, -33.7], M::Steel),
    part([-25.15, 0.0, -18.3, -24.85, 3.5, -18.0], M::Steel),
    part([-33.0, 0.0, -30.0, -18.0, 0.5, -27.0], M::Concrete),
    part([-30.0, 0.0, -24.0, -21.0, 0.5, -21.0], M::Concrete),
    // Canteen (SE): a container bar, tables, and a tarp over the tables on
    // poles through them (`POLES`).
    part([16.0, 0.0, -34.0, 22.1, 2.6, -31.56], M::Cargo),
    part([28.0, 0.0, -34.0, 34.1, 2.6, -31.56], M::Cargo),
    part([20.0, 0.0, -25.0, 22.0, 0.8, -23.0], M::Timber),
    part([26.0, 0.0, -25.0, 28.0, 0.8, -23.0], M::Timber),
    part([20.0, 0.0, -20.0, 22.0, 0.8, -18.0], M::Timber),
    part([26.0, 0.0, -20.0, 28.0, 0.8, -18.0], M::Timber),
    part_f(
        [18.0, 3.2, -27.0, 30.0, 3.4, -16.0],
        M::Canvas,
        DECOR | ROOF,
    ),
    // Four corner watchtowers: legs collide, decks and roofs are drawn.
    part([-37.0, 0.0, -37.0, -36.6, 8.0, -36.6], M::Timber),
    part([-33.4, 0.0, -37.0, -33.0, 8.0, -36.6], M::Timber),
    part([-37.0, 0.0, -33.4, -36.6, 8.0, -33.0], M::Timber),
    part([-33.4, 0.0, -33.4, -33.0, 8.0, -33.0], M::Timber),
    part_f([-37.4, 7.0, -37.4, -32.6, 7.3, -32.6], M::Timber, DECOR),
    part_f([-37.8, 9.6, -37.8, -32.2, 9.9, -32.2], M::Sheet, DECOR),
    part([33.0, 0.0, -37.0, 33.4, 8.0, -36.6], M::Timber),
    part([36.6, 0.0, -37.0, 37.0, 8.0, -36.6], M::Timber),
    part([33.0, 0.0, -33.4, 33.4, 8.0, -33.0], M::Timber),
    part([36.6, 0.0, -33.4, 37.0, 8.0, -33.0], M::Timber),
    part_f([32.6, 7.0, -37.4, 37.4, 7.3, -32.6], M::Timber, DECOR),
    part_f([32.2, 9.6, -37.8, 37.8, 9.9, -32.2], M::Sheet, DECOR),
    part([-37.0, 0.0, 33.0, -36.6, 8.0, 33.4], M::Timber),
    part([-33.4, 0.0, 33.0, -33.0, 8.0, 33.4], M::Timber),
    part([-37.0, 0.0, 36.6, -36.6, 8.0, 37.0], M::Timber),
    part([-33.4, 0.0, 36.6, -33.0, 8.0, 37.0], M::Timber),
    part_f([-37.4, 7.0, 32.6, -32.6, 7.3, 37.4], M::Timber, DECOR),
    part_f([-37.8, 9.6, 32.2, -32.2, 9.9, 37.8], M::Sheet, DECOR),
    part([33.0, 0.0, 33.0, 33.4, 8.0, 33.4], M::Timber),
    part([36.6, 0.0, 33.0, 37.0, 8.0, 33.4], M::Timber),
    part([33.0, 0.0, 36.6, 33.4, 8.0, 37.0], M::Timber),
    part([36.6, 0.0, 36.6, 37.0, 8.0, 37.0], M::Timber),
    part_f([32.6, 7.0, 32.6, 37.4, 7.3, 37.4], M::Timber, DECOR),
    part_f([32.2, 9.6, 32.2, 37.8, 9.9, 37.8], M::Sheet, DECOR),
    // Street lamp posts.
    part([-5.9, 0.0, -30.1, -5.7, 5.0, -29.9], M::Steel),
    part([5.7, 0.0, -18.1, 5.9, 5.0, -17.9], M::Steel),
    part([-5.9, 0.0, -9.1, -5.7, 5.0, -8.9], M::Steel),
    part([-5.9, 0.0, 17.95, -5.7, 5.0, 18.15], M::Steel),
    part([5.7, 0.0, 25.95, 5.9, 5.0, 26.15], M::Steel),
];

/// What a station anchor stands up: the deployable archetype's name in
/// `content/deployables.toml` and a build-cell centre in the local frame.
pub const STATIONS: [(&str, f32, f32); 5] = [
    ("workbench", -28.5, -22.5),
    ("research", -22.5, -22.5),
    ("recycler", -31.5, -28.5),
    ("recycler", -25.5, -28.5),
    ("recycler", -19.5, -28.5),
];

/// Where a player stands to trade at kiosk `k`, local (x, z). Kiosk `k`
/// serves vendor `k` (`content/sites.toml`).
pub const KIOSKS: [(f32, f32); 6] = [
    (-4.6, 14.05),
    (4.6, 14.05),
    (-4.6, 22.05),
    (4.6, 22.05),
    (-4.6, 30.05),
    (4.6, 30.05),
];

/// Night lights, local (x, y, z) — drawn, never simulated.
pub const LAMPS: [(f32, f32, f32); 12] = [
    (-5.8, 5.0, -30.0),
    (5.8, 5.0, -18.0),
    (-5.8, 5.0, -9.0),
    (-5.8, 5.0, 18.05),
    (5.8, 5.0, 26.05),
    (-35.0, 9.2, -35.0),
    (35.0, 9.2, -35.0),
    (-35.0, 9.2, 35.0),
    (35.0, 9.2, 35.0),
    (9.5, 31.8, 0.0),
    (-25.0, 3.3, -26.0),
    (25.0, 3.0, -22.0),
];

/// The canteen tarp's poles, local (x, z) — drawn, never simulated. Each
/// stands through a table, so the table stops a body before the pole would.
pub const POLES: [(f32, f32); 4] = [(21.0, -24.0), (27.0, -24.0), (21.0, -19.0), (27.0, -19.0)];

/// The sentry guns (`sentry.rs`), local (x, gun height over the floor, z):
/// one on each watchtower's roof, high enough to see over the container
/// walls into every corner of the yard.
pub const SENTRY_POSTS: [(f32, f32, f32); 4] = [
    (-35.0, 10.7, -35.0),
    (35.0, 10.7, -35.0),
    (-35.0, 10.7, 35.0),
    (35.0, 10.7, 35.0),
];

/// Where a player woken at THE GATE stands up (local x, z): the market
/// street south of the gate, spread across its width by who they are.
pub const SPAWN_STREET: (f32, f32) = (0.0, -24.0);

const _: () = {
    assert!(kit::well_formed(PARTS));
    // Every part inside the carved floor, corners included.
    let e = kit::envelope(PARTS);
    assert!(e * e * 2.0 <= TOWN_FOOTPRINT.stamp_m * TOWN_FOOTPRINT.stamp_m);
    assert!(TOWN_FOOTPRINT.swept_m <= TOWN_FOOTPRINT.stamp_m);
    assert!(TOWN_FOOTPRINT.stamp_m < TOWN_FOOTPRINT.scatter_m);
    assert!(TOWN_FOOTPRINT.scatter_m < TOWN_FOOTPRINT.blend_m);
    assert!(PORT_R > e && PORT_R < TOWN_FOOTPRINT.stamp_m);
    // Station anchors are build-cell centres: 3i + 1.5 on both axes.
    let mut i = 0;
    while i < STATIONS.len() {
        let (_, x, z) = STATIONS[i];
        let cx = (x - 1.5) / 3.0;
        let cz = (z - 1.5) / 3.0;
        assert!(cx == (cx as i32) as f32 && cz == (cz as i32) as f32);
        i += 1;
    }
    // Every pole stands through a part that stops a body short of it.
    let mut i = 0;
    while i < POLES.len() {
        let (x, z) = POLES[i];
        let mut held = false;
        let mut k = 0;
        while k < PARTS.len() {
            let (b, flags) = (PARTS[k].b, PARTS[k].flags);
            if flags & DECOR == 0
                && b[1] <= 0.0
                && b[4] > crate::movement::STEP_UP
                && b[0] + 0.5 < x
                && x < b[3] - 0.5
                && b[2] + 0.5 < z
                && z < b[5] - 0.5
            {
                held = true;
            }
            k += 1;
        }
        assert!(held);
        i += 1;
    }
    // The lattice is a multiple of the build cell and the scatter cell.
    assert!(TOWN_SNAP_M == 24.0);
};

fn near(t: &Town, x: f32, z: f32, half: f32) -> bool {
    if !t.live {
        return false;
    }
    let (dx, dz) = (x - t.x, z - t.z);
    dx >= -half && dx <= half && dz >= -half && dz <= half
}

/// Whether the town's boxes stop a volume (`landmark::blocks`'s shape).
pub fn blocks(t: &Town, x: f32, z: f32, feet: f32, r: f32, h: f32) -> bool {
    if !near(t, x, z, 43.0 + r) {
        return false;
    }
    let (lx, lz) = kit::to_local(&t.placed(), x, z);
    kit::blocks_local(PARTS, t.floor_y, 0, lx, lz, feet, r, h)
}

/// The highest town surface within a step of `feet` under a disc of radius
/// `r` at (`x`, `z`) — `kit::ground_local`'s footprint.
pub fn ground(t: &Town, x: f32, z: f32, feet: f32, r: f32) -> f32 {
    if !near(t, x, z, 43.0 + r) {
        return crate::collide::NO_SURFACE;
    }
    let (lx, lz) = kit::to_local(&t.placed(), x, z);
    kit::ground_local(PARTS, t.floor_y, 0, lx, lz, feet, r)
}

/// Whether a roof keeps the rain off a body here.
pub fn roofed(t: &Town, x: f32, z: f32, feet: f32) -> bool {
    if !near(t, x, z, 43.0) {
        return false;
    }
    let (lx, lz) = kit::to_local(&t.placed(), x, z);
    kit::roofed_local(PARTS, t.floor_y, lx, lz, feet)
}

/// Inside the safe zone: no player is hurt here and nobody hurts from here.
pub fn safe(t: &Town, x: f32, z: f32) -> bool {
    near(t, x, z, SAFE_HALF_M)
}

/// Whether building at (`x`, `z`) with `margin` would touch the town.
pub fn reserves(t: &Town, x: f32, z: f32, margin: f32) -> bool {
    near(t, x, z, RESERVE_HALF_M + margin)
}

/// Whether (`x`, `z`) is inside the town's scatter mask (nothing grows).
pub fn covers(t: &Town, x: f32, z: f32, pad: f32) -> bool {
    if !t.live {
        return false;
    }
    let (dx, dz) = (x - t.x, z - t.z);
    let r = TOWN_FOOTPRINT.scatter_m + pad;
    dx * dx + dz * dz <= r * r
}

/// Kiosk `k`'s trading spot in world space.
pub fn kiosk_world(t: &Town, k: usize) -> Option<(f32, f32)> {
    let &(x, z) = KIOSKS.get(k)?;
    Some(kit::to_world(&t.placed(), x, z))
}

/// Sentry `k`'s gun in world space: (x, y, z), metres.
pub fn sentry_world(t: &Town, k: usize) -> Option<(f32, f32, f32)> {
    let &(x, y, z) = SENTRY_POSTS.get(k)?;
    let (wx, wz) = kit::to_world(&t.placed(), x, z);
    Some((wx, t.floor_y + y, wz))
}

/// Station `k` in world space: (archetype name, x, z).
pub fn station_world(t: &Town, k: usize) -> Option<(&'static str, f32, f32)> {
    let &(arch, x, z) = STATIONS.get(k)?;
    let (wx, wz) = kit::to_world(&t.placed(), x, z);
    Some((arch, wx, wz))
}

/// The kit as JSON for `ci/site_kit.py` (`examples/kit_dump.rs`).
pub fn dump(w: &mut impl core::fmt::Write) -> core::fmt::Result {
    const N: usize = STATIONS.len() + KIOSKS.len() + LAMPS.len() + POLES.len();
    let mut anchors: [(&str, f32, f32, f32); N] = [("", 0.0, 0.0, 0.0); N];
    let mut n = 0;
    for (arch, x, z) in STATIONS {
        anchors[n] = (arch, x, 0.5, z);
        n += 1;
    }
    for (x, z) in KIOSKS {
        anchors[n] = ("kiosk", x, 0.0, z);
        n += 1;
    }
    for (x, y, z) in LAMPS {
        anchors[n] = ("lamp", x, y, z);
        n += 1;
    }
    for (x, z) in POLES {
        anchors[n] = ("pole", x, 0.0, z);
        n += 1;
    }
    kit::dump("town", PARTS, &anchors, w)
}
