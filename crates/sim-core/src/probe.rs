//! The parity/golden probe surface: `extern "C"` exports that build to
//! wasm32-unknown-unknown with no bindgen, driven from node by
//! `ci/parity.mjs` and from native by `examples/probe.rs`. Same functions,
//! both targets, byte-equal answers — that is `test_parity_wasm`
//! (DESIGN.md §12) and the wasm half of `test_terrain_golden`
//! (TERRAIN.md §0).

use crate::bots::bot_frame;
use crate::gather::ItemStack;
use crate::limits::HOTBAR_SLOTS;
use crate::rng::{splitmix64, Pcg32};
use crate::terrain::{self, ScatterTable};
use crate::world::{Command, World};
use xxhash_rust::xxh3::Xxh3;

/// Side of a probe scatter window, in cells. 16, the block size the
/// island-center window has always used, so a site window and the center
/// window contribute the same 256 cells and neither dominates the digest.
/// Public with `probe_window_origin`, and for the same reason.
pub const PROBE_WINDOW_CELLS: i32 = 16;

/// Bearings and radii of the road-ring sweep in `probe_sites`.
///
/// 64 bearings divide the 256-entry yaw LUT evenly (step 4). The radii count
/// is the derived half, and the derivation is the whole point: a sweep that
/// never crosses the road hashes a constant and pins nothing, so the radial
/// step has to be under the road's widest band — the shoulder, at
/// `ROAD_SHOULDER_HALF_W * 2` = 10 m. 51 radii puts a sample every
/// `(ROAD_R_MAX - ROAD_R_MIN) / 50` = 8 m, which clears it.
///
/// It is NOT the first count that clears it, and this comment used to say so:
/// 42 radii gives a 9.76 m step and is strictly earlier
/// (`pass-20260805-074623-02-judge.md` fix 3). 8 m is the coarsest step that
/// is an integer number of metres AND divides the 400 m span evenly, which is
/// a tidiness argument rather than a derivation — the load-bearing half is
/// the measurement below, not the superlative.
///
/// Measured on the three probe seeds rather than assumed: at 8 radii (a 57 m
/// step) only 9–15 of the 64 bearings saw any road at all, so most of the
/// sweep was hashing `Off`. At 51 every one of the 64 bearings sees road on
/// all three seeds, for ~2 ms. `tests/terrain_golden.rs` asserts that
/// instead of trusting this comment.
pub const PROBE_ROAD_BEARINGS: u16 = 64;
pub const PROBE_ROAD_RADII: i32 = 51;

/// World position of one road-sweep sample, off the island center on the
/// yaw LUT — wall 1 bans libm, and the LUT is the exact table the rest of
/// worldgen turns bearings with, so these are the points worldgen would
/// pick. Public and shared with the coverage test for the same reason
/// `probe_window_origin` is: a test that recomputes the sweep is a test of
/// itself, not of the digest.
pub fn probe_road_point(bearing: u16, radius_ix: i32) -> (f32, f32) {
    let c = terrain::ISLAND_SIZE * 0.5;
    let span = terrain::ROAD_R_MAX - terrain::ROAD_R_MIN;
    let (dx, dz) = crate::yaw_lut::yaw_dir(bearing << 8);
    let d = terrain::ROAD_R_MIN + span * (radius_ix as f32 / (PROBE_ROAD_RADII - 1) as f32);
    (c + dx * d, c + dz * d)
}

/// Samples along each side road's own length.
///
/// Sized like [`PROBE_ROAD_RADII`] and for its reason: the step has to be
/// under the road's widest band so a sample cannot fall between two answers.
/// A side road is at most `ROAD_R_MAX` long from a port inside
/// `INLAND_R_MAX`, which is 1,000 m, and 129 samples put one every 7.8 m
/// against a 10 m shoulder. It is a count rather than a pitch so the byte
/// count of this digest does not depend on how long a seed's road came out.
pub const PROBE_SIDE_ROAD_SAMPLES: i32 = 129;

/// World position of one side-road sample: `i / (N-1)` of the way from the
/// port to the ring **along the road**, not along its chord.
///
/// By ARC LENGTH rather than by leg index, so the pitch between two samples
/// is the same everywhere and the 7.8 m the constant above argues for is
/// still what it is. Walking by leg index would put `PROBE_SIDE_ROAD_SAMPLES
/// / (SIDE_ROAD_POINTS - 1)` samples on each leg whatever its length, and the
/// digest would sample a short leg finely and a long one coarsely for no
/// reason a reader could see.
///
/// Public and shared with the coverage test for `probe_road_point`'s reason —
/// a test that recomputes the sweep is a test of itself. A dead road has every
/// node at the origin, so every sample of it is the same point and the digest
/// carries `Off` 129 times, which is an honest answer about an island with no
/// road rather than a hole.
pub fn probe_side_road_point(road: &terrain::SideRoad, i: i32) -> (f32, f32) {
    let t = i as f32 / (PROBE_SIDE_ROAD_SAMPLES - 1) as f32;
    let mut want = road.path_len() * t;
    let last = terrain::SIDE_ROAD_POINTS - 1;
    let (mut ax, mut az) = road.node(0);
    for k in 1..=last {
        let (bx, bz) = road.node(k);
        let (ex, ez) = (bx - ax, bz - az);
        let len = (ex * ex + ez * ez).sqrt();
        if want <= len || k == last {
            let f = if len > 0.0 {
                (want / len).clamp(0.0, 1.0)
            } else {
                0.0
            };
            return (ax + ex * f, az + ez * f);
        }
        want -= len;
        ax = bx;
        az = bz;
    }
    road.node(last)
}

/// Origin cell of the probe's scatter window around a world position.
///
/// Public because `tests/terrain_golden.rs` asserts the digest's coverage
/// over exactly the cells the digest hashes; a coverage test that recomputes
/// this arithmetic is a test of itself. Clamped to the grid so a site near
/// the island edge still contributes exactly 256 cells — a short window
/// would make the digest's byte count seed-dependent, which is legal and
/// makes a diff unreadable. Positive coords only (the ring bracket is
/// 600..1000 m from a center at half of `ISLAND_SIZE`), so the truncating
/// cast is a floor, as it is in `terrain::scatter`.
pub fn probe_window_origin(x: f32, z: f32) -> (i32, i32) {
    let w = PROBE_WINDOW_CELLS;
    let hi = terrain::CELLS_PER_SIDE - w;
    let cx = ((x * (1.0 / terrain::CELL_SIZE)) as i32 - w / 2).clamp(0, hi);
    let cz = ((z * (1.0 / terrain::CELL_SIZE)) as i32 - w / 2).clamp(0, hi);
    (cx, cz)
}

fn hash_f32(h: &mut Xxh3, v: f32) {
    h.update(&v.to_bits().to_le_bytes());
}

fn hash_scatter_window(h: &mut Xxh3, seed: u64, haven: &terrain::Haven, x: f32, z: f32) {
    let table = ScatterTable::alpha_default();
    let (cx0, cz0) = probe_window_origin(x, z);
    for cz in cz0..cz0 + PROBE_WINDOW_CELLS {
        for cx in cx0..cx0 + PROBE_WINDOW_CELLS {
            let s = terrain::scatter(seed, &table, haven, cx, cz);
            h.update(&[s.occupant as u8, s.yaw, s.species]);
            hash_f32(h, s.x);
            hash_f32(h, s.y);
            hash_f32(h, s.z);
            hash_f32(h, s.scale);
        }
    }
}

/// The island's authored half, as VALUES: the haven pad's own fields, its
/// shelter, its five containers, both waystations and their containers, then
/// the coast road ring all three stand on.
///
/// A separate export rather than more bytes inside `probe_terrain`, so that
/// when the two targets disagree the diff NAMES the authored geometry
/// instead of moving one opaque terrain digest. `examples/probe.rs` and
/// `ci/parity.mjs` both print it on its own line and `ci/gates.sh` requires
/// that line to be present.
///
/// It exists because `probe_terrain` could not see any of this. Its scatter
/// block is cells 120..136 — a ±64 m window on the island center — and every
/// authored site sits on the 600..1000 m road ring. Measured on the three
/// seeds `examples/probe.rs` drives, not reasoned about: of that block's 256
/// cells, the number inside `in_haven` or `in_waystation` was **zero on all
/// three**. So `haven(seed)` was resolved by the probe and reached the digest
/// through nothing at all — the pad, both waystations, all five pad crates,
/// the shelter and all four waystation crates could each have taken different
/// coordinates on wasm than on native with `test_terrain_golden` and
/// `test_parity_wasm` both green.
///
/// That is not hypothetical drift. `client-core` resolves `terrain::haven`
/// on the WASM build (`bridge.rs`, `core.rs`) and the server resolves it
/// natively, so those two answers agreeing IS the client↔server contract for
/// where the island's destinations are, and nothing was holding it.
#[no_mangle]
pub extern "C" fn probe_sites(seed: u64) -> u64 {
    let mut h = Xxh3::new();
    for part in &crate::depot::DEPOT_PARTS {
        h.update(&[part.kind as u8]);
        for value in part.bounds {
            hash_f32(&mut h, value);
        }
    }
    let haven = terrain::haven(seed);
    for i in 0..terrain::RING_BEARINGS {
        hash_f32(&mut h, haven.ring.r[i]);
        hash_f32(&mut h, haven.ring.y[i]);
        let (x, z) = haven.ring.node(i as i32);
        hash_f32(&mut h, terrain::ground(seed, &haven, x, z));
    }
    hash_f32(&mut h, haven.x);
    hash_f32(&mut h, haven.z);
    hash_f32(&mut h, haven.y);
    // The carved floor's level: worldgen state since the carve was armed, and
    // the datum every seated object and every cut depth is measured from.
    hash_f32(&mut h, haven.floor_y);
    hash_f32(&mut h, haven.relief);
    h.update(&[haven.phase, haven.shelter]);
    // The ore budget: it moves every rock-channel cell's draw, and none of
    // those need lie inside the scatter windows `probe_terrain` hashes — on
    // the golden seed none does. Folded here so a budget change moves the
    // world digest and an old save refuses rather than loading onto moved
    // nodes; and it is a haven field the client resolves on wasm.
    for pm in haven.ore_pm {
        h.update(&pm.to_le_bytes());
    }
    let (sx, sz, syaw) = terrain::haven_shelter(&haven);
    hash_f32(&mut h, sx);
    hash_f32(&mut h, sz);
    h.update(&[syaw]);
    let mut k = 0i32;
    while k < terrain::HAVEN_CRATES {
        let (ax, az, yaw) = terrain::haven_crate(&haven, k);
        hash_f32(&mut h, ax);
        hash_f32(&mut h, az);
        h.update(&[yaw]);
        k += 1;
    }
    // `live` is hashed with the geometry on purpose: a seed whose ring
    // cannot hold a full tier leaves `Waystation::NONE` at (0,0), and that
    // is a worldgen answer the fingerprint should carry rather than a hole
    // it should paper over.
    for ws in &haven.minor {
        hash_f32(&mut h, ws.x);
        hash_f32(&mut h, ws.z);
        hash_f32(&mut h, ws.y);
        hash_f32(&mut h, ws.floor_y);
        // `kind` rides with the geometry for the same reason `live` does: a
        // site that moved tier without moving is a worldgen answer, and the
        // two tiers differ in what they SPAWN rather than in where they
        // stand, so nothing else in this digest could see it.
        h.update(&[ws.phase, ws.live as u8, ws.kind as u8]);
        if crate::depot::is_depot(ws) {
            for part in &crate::depot::DEPOT_PARTS {
                let b = part.bounds;
                let (x, z) = crate::depot::to_world(ws, (b[0] + b[3]) * 0.5, (b[2] + b[5]) * 0.5);
                let y = ws.floor_y + (b[1] + b[4]) * 0.5;
                h.update(&[crate::depot::blocks(&haven, x, z, y, 0.01, 0.01) as u8]);
                hash_f32(
                    &mut h,
                    crate::depot::ground(&haven, x, z, ws.floor_y + b[4]),
                );
            }
        }
        // The tier's own count, so a site that stands no containers hashes
        // no anchors rather than hashing a ring nothing will build.
        let mut c = 0i32;
        while c < terrain::site_crates(ws.kind) {
            let (ax, az, yaw) = terrain::waystation_crate(ws, c);
            hash_f32(&mut h, ax);
            hash_f32(&mut h, az);
            h.update(&[yaw]);
            c += 1;
        }
    }
    // The road itself, on the same bracket that hid the sites — it is what
    // the sites on the ring stand on and what carries the barrel band.
    //
    // ⚠ **That bracket is `ROAD_R_MIN..ROAD_R_MAX` and a side road is not in
    // it**, which is a wall-5 hole and not a coverage nicety: a road solved
    // in the interior could differ between native and wasm with this digest,
    // `test_replay` and `test_parity_wasm` all green, and `client-core` reads
    // the wasm answer while the server reads the native one. The sweep below
    // is what closes it, and it is a SEPARATE sweep rather than a widened
    // bracket because widening this one would re-hash 400 m of open interior
    // to reach 5 m of road.
    let mut b = 0u16;
    while b < 256 {
        let mut r = 0i32;
        while r < PROBE_ROAD_RADII {
            let (px, pz) = probe_road_point(b, r);
            h.update(&[terrain::road_band(seed, &haven, px, pz) as u8]);
            r += 1;
        }
        b += 256 / PROBE_ROAD_BEARINGS;
    }

    // Every side road, along its own length. The polyline first — it is the
    // solved answer, and two ends that moved would move everything below —
    // then the band it produces, sampled at `PROBE_SIDE_ROAD_SAMPLES` points
    // walked from the port to the ring. Sampling the band rather than only
    // the endpoints is the half that matters: the endpoints are stored floats
    // and `side_band` is arithmetic over them, so a divergence in the
    // arithmetic would otherwise be invisible.
    for road in haven.roads.iter() {
        hash_f32(&mut h, road.px);
        hash_f32(&mut h, road.pz);
        hash_f32(&mut h, road.rx);
        hash_f32(&mut h, road.rz);
        h.update(&[road.port, road.live as u8]);
        let mut i = 0i32;
        while i < PROBE_SIDE_ROAD_SAMPLES {
            let (sx, sz) = probe_side_road_point(road, i);
            h.update(&[terrain::road_band(seed, &haven, sx, sz) as u8]);
            i += 1;
        }
    }
    h.digest()
}

/// Golden terrain fingerprint, in four stages: 64×64 heights sampled on a
/// 32 m grid, the 256 scatter cells of the island-center 16×16 block (cells
/// 120..136; the corner block would be all sea and pin nothing), a 256-cell
/// scatter window on each of the three authored sites, and `probe_sites`
/// folded in whole. Hashes f32 bit patterns — bit-identical or bust.
///
/// The site windows are what put the authored OCCUPANTS on the surface —
/// the `HavenShelter` and `CrateSlot` slots `scatter` places, and the
/// `in_haven` / `in_waystation` veto that clears the ground around them.
/// `probe_sites` covers the anchors those slots are computed from. Neither
/// subsumes the other: the anchors could hold while `scatter` stopped
/// emitting them, and `tests/terrain_golden.rs` asserts the window coverage
/// as a count so that re-centring a window on empty sea fails loudly rather
/// than regenerating a clean golden over nothing.
#[no_mangle]
pub extern "C" fn probe_terrain(seed: u64) -> u64 {
    let mut h = Xxh3::new();
    for gz in 0..64i32 {
        for gx in 0..64i32 {
            let x = gx as f32 * 32.0 + 16.0;
            let z = gz as f32 * 32.0 + 16.0;
            h.update(&terrain::height(seed, x, z).to_bits().to_le_bytes());
        }
    }
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    for cz in 120..136i32 {
        for cx in 120..136i32 {
            let s = terrain::scatter(seed, &table, &haven, cx, cz);
            h.update(&[s.occupant as u8, s.yaw, s.species]);
            h.update(&s.x.to_bits().to_le_bytes());
            h.update(&s.y.to_bits().to_le_bytes());
            h.update(&s.z.to_bits().to_le_bytes());
            h.update(&s.scale.to_bits().to_le_bytes());
        }
    }
    hash_scatter_window(&mut h, seed, &haven, haven.x, haven.z);
    for ws in &haven.minor {
        hash_scatter_window(&mut h, seed, &haven, ws.x, ws.z);
    }
    h.update(&probe_sites(seed).to_le_bytes());
    h.digest()
}

/// Movement + gather parity: `sequences` independent random input
/// sequences, each a fresh world + 2 bots × `ticks` ticks; the
/// per-sequence state hashes fold into one digest (DESIGN.md §4: 10,000
/// sequences through both builds). Bots hold the primary button in
/// bursts and the world carries the synthetic gather fixture, so slot
/// life, yields, and inventories are inside the parity surface. It also
/// carries `CombatContent::raid_fixture()` — a table that cannot fight
/// and can chip a wall — so the raid scan, the structure-damage write and
/// the removal path ride parity here without a brawl emptying the
/// inventories the rest of this probe's coverage depends on.
#[no_mangle]
pub extern "C" fn probe_parity(master_seed: u64, sequences: u32, ticks: u32) -> u64 {
    let mut h = Xxh3::new();
    for s in 0..sequences {
        let seq_seed = splitmix64(master_seed ^ (s as u64));
        let mut world = World::new(seq_seed);
        world.gather = crate::gather::GatherContent::probe_fixture();
        world.craft = crate::craft::CraftContent::probe_fixture();
        world.build = crate::build::BuildContent::probe_fixture();
        world.deploy = crate::deploy::DeployContent::probe_fixture();
        // The barrel roll is integer-only on purpose (loot.rs), and this
        // is the gate that says so: `splitmix64`, the multiply-shift pick
        // and the count draw all have to land bit-identical on wasm32 and
        // native, and the container they fill is hashed state.
        world.loot = crate::loot::LootContent::probe_fixture();
        world.backpack = crate::backpack::BackpackContent::probe_fixture();
        world.combat = crate::combat::CombatContent::raid_fixture();
        world.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
        let mut rng = Pcg32::new(seq_seed, 7);
        let mut yaws = [0u16; 2];
        for t in 0..ticks {
            let f1 = bot_frame(&mut rng, yaws[0], t as u16);
            let f2 = bot_frame(&mut rng, yaws[1], t as u16);
            yaws = [f1.yaw, f2.yaw];
            // Bots poke the craft verb on a fixed cadence so enqueue,
            // completion, refusal, and cancel are all inside the parity/
            // replay/alloc surface (fixture recipes, gathered inputs).
            let craft = Command::Craft {
                id: 1,
                recipe: (t % 4) as u16, // 3 = out of range: refusal path
                count: 1 + (t % 2) as u16,
                skin: 0,
            };
            let cancel = Command::CraftCancel {
                id: 2,
                index: (t % 5) as u16,
            };
            // Bot 1 also pokes the build verb at its own feet: row 5 is
            // out of range and the loc cycle mismatches most shapes, so
            // placement successes AND every refusal reason ride the
            // parity/replay/alloc surface once inventories fill —
            // including doorways (row 3), which arm the door verb below.
            let own_cell = {
                let b = &world.players[0].body;
                let cx = crate::build::build_cell_of(b.qx as f32 * crate::movement::POS_XZ_Q);
                let cz = crate::build::build_cell_of(b.qz as f32 * crate::movement::POS_XZ_Q);
                (cx.clamp(0, 1023) as u16, cz.clamp(0, 1023) as u16)
            };
            // t ≡ 11 (mod 16) on every place tick, so cycle on t/16.
            let place = Command::Place {
                id: 1,
                row: ((t / 16) % 6) as u16,
                cx: own_cell.0,
                cz: own_cell.1,
                level: ((t / 32) % 2) as u8,
                loc: ((t / 16) % 4) as u8,
                freehand: false,
                // Both signs and zero, so the asked band (foundation
                // height v0) — and its refusal past the window — is on the
                // parity/replay surface rather than a constant nothing
                // exercises.
                plate: ((t / 64) % 5) as i8 - 2,
            };
            // Bot 2 pokes the deploy verb at its own feet: loc/level
            // cycle, so placements AND every deploy refusal reason ride
            // the surface; feed mostly hits the no-hearth refusal, and the
            // successes cover stock. The gated craft (recipe 2) arms once
            // bot 2's workbench stands.
            //
            // The modulus is the fixture's `def_count` (8 since research
            // v0), so every archetype the table declares gets placed here
            // — including the lock, which almost always refuses because no
            // door stands at the address, and the recycler, whose switch
            // must NOT refuse for want of fuel. Widening it was the whole
            // cost of putting the new archetype inside the parity, replay
            // and alloc surfaces.
            let own2 = {
                let b = &world.players[1].body;
                let cx = crate::build::build_cell_of(b.qx as f32 * crate::movement::POS_XZ_Q);
                let cz = crate::build::build_cell_of(b.qz as f32 * crate::movement::POS_XZ_Q);
                (cx.clamp(0, 1023) as u16, cz.clamp(0, 1023) as u16)
            };
            let place_deploy = Command::PlaceDeploy {
                id: 2,
                row: ((t / 16) % 8) as u16,
                cx: own2.0,
                cz: own2.1,
                level: ((t / 64) % 2) as u8,
                loc: ((t / 16) % 4) as u8,
            };
            if t == ticks / 2 {
                // Leap the clock 30 upkeep periods so charge, decay, and
                // removal run inside the parity/replay/alloc surface
                // without simulating thirty real hours (deploy.rs; every
                // timer is tick-driven, so the leap is deterministic).
                world.tick += 30 * crate::deploy::UPKEEP_PERIOD_TICKS;
            }
            // 35 ≡ 3 (mod 16): the feed branch must test first or the
            // deploy branch would shadow it.
            if t % 64 == 35 {
                world.tick(&[
                    Command::Input {
                        id: 1,
                        frame: f1,
                        favour: 0,
                    },
                    Command::Input {
                        id: 2,
                        frame: f2,
                        favour: 0,
                    },
                    Command::Feed {
                        id: 2,
                        cx: own2.0,
                        cz: own2.1,
                        level: 0,
                    },
                    Command::Craft {
                        id: 2,
                        recipe: 2,
                        count: 1,
                        skin: 0,
                    },
                ]);
                continue;
            }
            if t % 16 == 3 {
                world.tick(&[
                    Command::Input {
                        id: 1,
                        frame: f1,
                        favour: 0,
                    },
                    Command::Input {
                        id: 2,
                        frame: f2,
                        favour: 0,
                    },
                    place_deploy,
                ]);
                continue;
            }
            if t % 16 == 9 {
                // And repairs them. Both stores, alternating on the same
                // addresses the upgrade arm walks, so the parity, replay
                // and alloc surfaces carry the verb itself rather than
                // only the price that makes it possible: a repair mutates
                // `Pieces` (or `Deploys`) *and* `Player::inv`, which is
                // exactly `test_replay`'s remit. Most land as refusals —
                // intact, unpriced, no such address — and refusals are
                // half of what these gates are for.
                world.tick(&[
                    Command::Input {
                        id: 1,
                        frame: f1,
                        favour: 0,
                    },
                    Command::Input {
                        id: 2,
                        frame: f2,
                        favour: 0,
                    },
                    Command::Repair {
                        id: 1,
                        deploy: (t / 16) % 2 == 0,
                        cx: own_cell.0,
                        cz: own_cell.1,
                        level: ((t / 32) % 2) as u8,
                        loc: ((t / 16) % 4) as u8,
                    },
                    // Bot 2 plants charges on bot 1's addresses on the same
                    // beat. It is the *other* half of what makes this verb
                    // worth a parity gate: `place` mutates the charge store
                    // and `Player::inv`, and `tick_fuses` then mutates a
                    // structure store on a **later tick than the command**
                    // — the only path in the sim where a command's effect
                    // lands after the tick that carried it, so a native and
                    // a wasm build that disagreed by one tick would show up
                    // here and nowhere else.
                    Command::Throw {
                        id: 2,
                        deploy: (t / 16) % 2 == 1,
                        cx: own_cell.0,
                        cz: own_cell.1,
                        level: ((t / 32) % 2) as u8,
                        loc: ((t / 16) % 4) as u8,
                    },
                ]);
                continue;
            }
            if t % 16 == 5 {
                // Bot 1 also pokes the upgrade verb at the same addresses
                // it builds on, cycling the whole ladder: wood is the
                // sideways/downward refusal, stone the one rung the
                // fixture holds (so real re-rows, payments, and damage
                // carry ride the surface), metal the missing-rung
                // refusal — plus every empty address and empty purse in
                // between.
                world.tick(&[
                    Command::Input {
                        id: 1,
                        frame: f1,
                        favour: 0,
                    },
                    Command::Input {
                        id: 2,
                        frame: f2,
                        favour: 0,
                    },
                    Command::Upgrade {
                        id: 1,
                        cx: own_cell.0,
                        cz: own_cell.1,
                        level: ((t / 32) % 2) as u8,
                        loc: ((t / 16) % 4) as u8,
                        material: ((t / 16) % 3) as u8,
                    },
                ]);
            } else if t % 16 == 7 {
                world.tick(&[
                    Command::Input {
                        id: 1,
                        frame: f1,
                        favour: 0,
                    },
                    Command::Input {
                        id: 2,
                        frame: f2,
                        favour: 0,
                    },
                    craft,
                    Command::Craft {
                        id: 2,
                        recipe: 0,
                        count: 1,
                        skin: 0,
                    },
                ]);
            } else if t % 16 == 11 {
                world.tick(&[
                    Command::Input {
                        id: 1,
                        frame: f1,
                        favour: 0,
                    },
                    Command::Input {
                        id: 2,
                        frame: f2,
                        favour: 0,
                    },
                    place,
                ]);
            } else if t % 16 == 13 {
                // Bot 2 pokes the use verb at its own cell's edges: no
                // door there almost always (the door refusal path), the
                // real toggle when its wanderings placed one — either
                // way the door command is inside the parity/replay/alloc
                // surface (loc 2/3 cycle; 0/1 hit the not-a-door arm).
                world.tick(&[
                    Command::Input {
                        id: 1,
                        frame: f1,
                        favour: 0,
                    },
                    Command::Input {
                        id: 2,
                        frame: f2,
                        favour: 0,
                    },
                    Command::Use {
                        id: 2,
                        cx: own2.0,
                        cz: own2.1,
                        level: ((t / 64) % 2) as u8,
                        loc: ((t / 16) % 4) as u8,
                    },
                ]);
            } else if t % 16 == 15 {
                // The access verb, cycled through all NINE ops — the six
                // lock ops at a door's edge and the three crew ops at the
                // cell body (hearth crew v1).
                // Bot 2's wanderings place row 4 — the code lock — often
                // enough that the store is sometimes occupied and
                // sometimes not, so both the no-lock refusal and every
                // landed op are inside the parity/replay/alloc surface.
                // The codes alternate so the ENTER op hits its miss path
                // (the shock, which writes a player's hp from a door
                // verb) as well as its match.
                let op_now = ((t / 16) % (crate::deploy::ACCESS_OP_MAX as u32 + 1)) as u8;
                world.tick(&[
                    Command::Input {
                        id: 1,
                        frame: f1,
                        favour: 0,
                    },
                    Command::Input {
                        id: 2,
                        frame: f2,
                        favour: 0,
                    },
                    Command::Access {
                        id: 2,
                        cx: own2.0,
                        cz: own2.1,
                        level: ((t / 64) % 2) as u8,
                        // A crew op addresses the cell body; a lock op an
                        // edge. Sending each at the address its verb
                        // means is what puts BOTH halves of the dispatch
                        // inside the parity/replay/alloc surface rather
                        // than one half and a refusal.
                        loc: if crate::deploy::op_is_crew(op_now) {
                            crate::build::LOC_PLANE
                        } else {
                            ((t / 16) % 4) as u8
                        },
                        op: op_now,
                        code: ((t * 7) % 10_000) as u16,
                    },
                ]);
            } else if t % 64 == 44 {
                // The demolish verb, both stores (demolish v1). Bot 2's
                // wanderings mean the address usually holds nothing, so
                // the empty-address refusal is inside the surface as
                // readily as the landing; and the window arithmetic runs
                // either way, which is what puts a **tick comparison** —
                // the one new kind of state this slice added — under the
                // parity and replay gates.
                world.tick(&[
                    Command::Input {
                        id: 1,
                        frame: f1,
                        favour: 0,
                    },
                    Command::Input {
                        id: 2,
                        frame: f2,
                        favour: 0,
                    },
                    Command::Demolish {
                        id: 2,
                        deploy: (t / 64) % 2 == 0,
                        cx: own2.0,
                        cz: own2.1,
                        level: 0,
                        loc: ((t / 64) % 4) as u8,
                    },
                ]);
            } else if t % 64 == 20 {
                world.tick(&[
                    Command::Input {
                        id: 1,
                        frame: f1,
                        favour: 0,
                    },
                    Command::Input {
                        id: 2,
                        frame: f2,
                        favour: 0,
                    },
                    cancel,
                ]);
            } else {
                world.tick(&[
                    Command::Input {
                        id: 1,
                        frame: f1,
                        favour: 0,
                    },
                    Command::Input {
                        id: 2,
                        frame: f2,
                        favour: 0,
                    },
                ]);
            }
        }
        h.update(&world.state_hash().to_le_bytes());
    }
    h.digest()
}

/// Respawn-on-bag parity: `sequences` independent worlds, each three bots
/// standing on buildable ground with bags in their packs and a clock fast
/// enough to kill them several times over, folded into one digest — the
/// bag half of `test_parity_wasm`.
///
/// A probe of its own rather than three more lines in `probe_combat`, and
/// the reason is that probe's own arrangement: it pins `dev_spawn` to a
/// *shoreline* point so the drink verb has sea inside reach, and a bag is
/// ground-class — it wants 1.5 m of height where the beach ring stands at
/// ~1.2. Moving that pin to buy bag coverage would have taken the landed
/// drink and the salt death out of the digest, which is trading one gate's
/// coverage for another's. This world walks inland instead, and gives up
/// the drink it never had.
///
/// **The return value carries a count as well as a digest**, packed
/// `wakes << 32 | (digest & 0xFFFF_FFFF)`, and that is deliberate: two
/// targets agreeing on the digest of a path that never ran is exactly the
/// shape of a pass nobody earned. `ci/gates.sh` reads the high half and
/// fails on zero, so the claim "the bag scan is inside `test_parity_wasm`"
/// is asserted rather than asserted-about. The count saturates rather than
/// wrapping, so it can never be zero for having been large.
#[no_mangle]
pub extern "C" fn probe_bags(master_seed: u64, sequences: u32, ticks: u32) -> u64 {
    let mut h = Xxh3::new();
    let mut wakes: u32 = 0;
    let mut screens: u32 = 0;
    for s in 0..sequences {
        let seq_seed = splitmix64(master_seed ^ (s as u64));
        let mut world = World::new(seq_seed);
        world.combat = crate::combat::CombatContent::probe_fixture();
        // Seconds-long spans: a body empties, starves, dies and wakes
        // several times inside one sequence, which is what puts the scan,
        // the cooldown and the ring fallback all on this surface.
        world.survival = crate::survival::SurvivalContent::probe_fixture();
        world.deploy = crate::deploy::DeployContent::probe_fixture();
        world.dev_spawn = Some(buildable_near(seq_seed, &world.haven, world.spawn_pos(1)));
        world.tick(&[
            Command::Join { id: 1 },
            Command::Join { id: 2 },
            Command::Join { id: 3 },
        ]);
        // Fixture arrangement: `BAG_CAP` bags each, so the cap, the
        // cooldown walk to a *second* bag, and the fallback to the ring
        // when every one of them is spent all ride the digest.
        for p in world.players.iter_mut().take(3) {
            p.inv[10] = ItemStack {
                item: 5, // the fixture's bag item (deploy row 3)
                count: crate::deploy::BAG_CAP as u16,
                cond: 0,
                skin: 0,
            };
        }
        let mut rng = Pcg32::new(seq_seed, 13);
        let mut yaws = [0u16; 3];
        for t in 0..ticks {
            let f1 = bot_frame(&mut rng, yaws[0], t as u16);
            let f2 = bot_frame(&mut rng, yaws[1], t as u16);
            let f3 = bot_frame(&mut rng, yaws[2], t as u16);
            yaws = [f1.yaw, f2.yaw, f3.yaw];
            // One bot plants a bag at its own feet every tick, in
            // rotation. They wander, so most requests land on terrain a
            // ground deploy refuses and the successes spread the bags out
            // — which is the only arrangement under which "nearest" is a
            // question with a wrong answer.
            let placer = (t % 3) as usize;
            let (cx, cz) = {
                let b = &world.players[placer].body;
                let cell = |q: i32| {
                    crate::build::build_cell_of(q as f32 * crate::movement::POS_XZ_Q).clamp(0, 1023)
                        as u16
                };
                (cell(b.qx), cell(b.qz))
            };
            // Two bots ask for a bag and one asks for the beach, every
            // tick, unconditionally — a press from a standing body is a
            // no-op (world.rs), so this puts *both* answers on the parity
            // surface permanently rather than whichever one a schedule
            // happened to reach. Bot 3's beach press is also the only
            // thing that proves the refusal does not spend a bag on one
            // target and does on the other.
            world.tick(&[
                Command::Input {
                    id: 1,
                    frame: f1,
                    favour: 0,
                },
                Command::Input {
                    id: 2,
                    frame: f2,
                    favour: 0,
                },
                Command::Input {
                    id: 3,
                    frame: f3,
                    favour: 0,
                },
                Command::PlaceDeploy {
                    id: placer as u32 + 1,
                    row: 3,
                    cx,
                    cz,
                    level: 0,
                    loc: crate::build::LOC_PLANE,
                },
                Command::Respawn {
                    id: 1,
                    on_bag: true,
                },
                Command::Respawn {
                    id: 2,
                    on_bag: true,
                },
                Command::Respawn {
                    id: 3,
                    on_bag: false,
                },
            ]);
            for e in world.events.entries() {
                if e.code == crate::world::EV_RESPAWN && e.b == 1 {
                    wakes = wakes.saturating_add(1);
                }
            }
            // Slot-ticks spent on the death screen. Counted beside the
            // wakes for the reason the wakes are counted beside the
            // digest: two targets can agree byte-for-byte about a path
            // neither ran, and `dead` is now the gate between a death and
            // a respawn — a zero here means the screen state itself has
            // fallen off the surface, whatever the wakes say.
            for p in world.players.iter().take(3) {
                if p.dead {
                    screens = screens.saturating_add(1);
                }
            }
        }
        h.update(&world.state_hash().to_le_bytes());
    }
    // Folded into the digest rather than returned beside the wakes: the
    // count of corpse-ticks is real parity surface (two targets that
    // disagreed about how long a body lay there would disagree here), but
    // it needs no gate of its own. Since v16 a wake is only reachable
    // *through* the screen — `Command::Respawn` does nothing to a standing
    // body — so `ci/gates.sh`'s existing "wakes > 0" is now strictly
    // stronger than it was: it proves the death, the screen, the answer
    // and the scan, in that order.
    h.update(&screens.to_le_bytes());
    ((wakes as u64) << 32) | (h.digest() & 0xFFFF_FFFF)
}

/// The first cell center at or inland of `(x, z)` that will hold a
/// ground-class deployable, stepping toward the island center. Bounded at
/// 200 steps of one build cell — 600 m, further than beach-to-center — so
/// it is a search and never a walk to nowhere. No trig and no `sqrt`: the
/// step direction is normalized by its own largest component, which is a
/// division (wall 1) and moves along the same ray.
fn buildable_near(seed: u64, haven: &terrain::Haven, (x, z): (f32, f32)) -> (f32, f32) {
    let c = terrain::ISLAND_SIZE * 0.5;
    let (mut dx, mut dz) = (c - x, c - z);
    let m = if dx < 0.0 { -dx } else { dx }.max(if dz < 0.0 { -dz } else { dz });
    if m <= 0.0 {
        return (x, z);
    }
    dx /= m;
    dz /= m;
    let (mut px, mut pz) = (x, z);
    let mut i = 0;
    while i < 200 {
        let cx = crate::build::build_cell_of(px);
        let cz = crate::build::build_cell_of(pz);
        let ax = (cx as f32 + 0.5) * crate::build::BUILD_CELL_M;
        let az = (cz as f32 + 0.5) * crate::build::BUILD_CELL_M;
        if crate::build::foundation_terrain_ok(seed, haven, ax, az) {
            return (ax, az);
        }
        px += dx * crate::build::BUILD_CELL_M;
        pz += dz * crate::build::BUILD_CELL_M;
        i += 1;
    }
    (x, z)
}

/// The fixture's firearm and its round (`CombatContent::probe_fixture`),
/// and the slots `probe_combat` keeps them in.
///
/// The gun takes the **last** hotbar slot rather than a new one: `bot_frame`
/// draws `sel` uniformly over `0..HOTBAR_SLOTS` and adding a seventh slot
/// would put the gun out of reach of every bot that already exists. Slot 5
/// held the fixture's 12-damage club, which is still on slots 1 and 3, so
/// no melee row leaves the surface to pay for this.
///
/// The rounds live in a **backpack** slot, past the hotbar the fill above
/// rewrites, for `probe_bags`' reason for `inv[10]`: a hand is chosen by
/// `sel` and a pocket is not, so ammunition that shared the hotbar would be
/// held instead of spent on whichever tick `sel` landed on it.
const GUN_SLOT: usize = HOTBAR_SLOTS - 1;
const ROUND_SLOT: usize = 10;
const GUN_ITEM: u16 = 6;
const ROUND_ITEM: u16 = 7;
/// Rounds re-stamped per tick. Only one can be spent per tick per bot (the
/// cadence is one shot per `rate_ticks`), so this is depth against the
/// `Consume`/`Loot` verbs reaching into the same pack, not a magazine.
const GUN_ROUNDS: u16 = 8;

/// Put `probe_combat`'s firearm back in every bot's hand.
///
/// **Called every tick, and the cost of not doing it was measured rather
/// than argued.** `World::die` clears `inv` through `..Player::default()`
/// and hands the contents to a backpack, so a gun stamped once is gone at a
/// bot's first death — and in this probe they die constantly and by design.
/// Dropping this call and arming once before the loop takes the rewound-shot
/// count from **2415 to 658** (500 × 256, measured 2026-08-30): not to zero,
/// so the gate below would still have passed, and down to 27%, with what
/// survives being whatever the `Command::Loot` rotation happened to hand
/// back. Re-stamping buys 3.7× the coverage and, more to the point, makes
/// the count a function of the fixture instead of the loot lottery.
///
/// The hotbar slots the fill writes are deliberately *not* re-stamped: their
/// drop-and-loot cycle is coverage this probe already had, and re-arming
/// them would delete it to buy nothing.
fn arm_guns(world: &mut World) {
    for p in world.players.iter_mut().take(3) {
        p.inv[GUN_SLOT] = ItemStack {
            item: GUN_ITEM,
            count: 1,
            cond: 0,
            skin: 0,
        };
        p.inv[ROUND_SLOT] = ItemStack {
            item: ROUND_ITEM,
            count: GUN_ROUNDS,
            cond: 0,
            skin: 0,
        };
    }
}

/// Combat parity: `sequences` independent three-bot brawls, each a fresh
/// world × `ticks` ticks, folded into one digest — the melee **and hitscan**
/// half of `test_parity_wasm`.
///
/// It is a probe of its own rather than three more lines inside
/// `probe_parity` because the two want opposite worlds. `probe_parity`'s
/// bots must stay alive to fill inventories, stand pieces, feed a hearth
/// and reach the upgrade rung; a brawl empties their pockets every few
/// seconds and would quietly hollow out the coverage that probe exists
/// for. So combat gets a world shaped for it: `dev_spawn` pinned to bot
/// 1's own ring point, which puts all three on the same sand at join —
/// and puts every respawn back there too, so the fight restarts instead
/// of ending — and a weapon in every hotbar slot, so the held-item read
/// is armed whichever slot a bot's wandering `sel` lands on. Hits, kills,
/// respawns and the whiff scan all ride the surface, native and wasm.
///
/// # The gun, and why the count is returned rather than folded
///
/// One of those hotbar weapons is a **hitscan firearm** since 2026-08-30
/// (`NOW.md` §0lc item 2). That matters because `ranged::hitscan` is the
/// only shot path that reads the lag-comp ring — `Pose::Rewound` at
/// `favour[i]` ticks back, where the arrow deliberately stays `Pose::Live`
/// — and the three bots already press their triggers at three favour
/// phases that share no period. Before it, this probe drove a nonzero
/// favour through the *melee* reader only, so the sentence "the rewind is
/// on the parity surface" was true of one of the two readers and was
/// written as though it were true of both.
///
/// **The return value carries a count as well as a digest**, packed
/// `rewound << 32 | (digest & 0xFFFF_FFFF)` — `probe_bags`' shape and its
/// reason, which applies here twice over. Two targets agreeing on the
/// digest of a path that never ran is a pass nobody earned, and this path
/// is reached through four gates that can each silently close: the fixture
/// row, `sel` landing on the gun, the shared swing cadence, and a round in
/// the pack. `ci/gates.sh` reads the high half and fails on zero. The count
/// saturates rather than wrapping, so it can never read zero for having
/// been large.
///
/// **What is counted is the consequence, not the arithmetic**, which is the
/// lesson §0lc paid for: sixteen gates were green under the `favour: 0`
/// literal that shipped, because a counter written beside a value cannot
/// witness that value reaching its destination. So `rewound` is not "a gun
/// fired" — it is *a hitscan shot fired by a bot whose favour this tick was
/// nonzero*, which is the only event that proves `Pose::Rewound` was the
/// pose the scan actually used. A fixture that silently stopped arming the
/// gun, and one that armed it while every favour collapsed to zero, both
/// read zero here. Plain `shots` is folded into the digest instead: a
/// native/wasm disagreement about how many rounds left a muzzle moves the
/// hash, but it needs no gate of its own once `rewound` has one.
///
/// A **hitscan** shot specifically, not any shot. `EV_SHOT` is raised by
/// every ranged thing, and the fixture also arms a throwable on item 3, so
/// the filter is the wire's own partition: `c`'s high half is the speed and
/// zero reads as *instantaneous* (`world.rs`, `EV_SHOT`), and its low half
/// is then the reach in decimetres, which pins the shot to this row's
/// `range_mm` and to no other. That stays exact on the day a bow joins this
/// table, where a speed test alone would not.
#[no_mangle]
pub extern "C" fn probe_combat(master_seed: u64, sequences: u32, ticks: u32) -> u64 {
    let mut h = Xxh3::new();
    let mut shots: u32 = 0;
    let mut rewound: u32 = 0;
    for s in 0..sequences {
        let seq_seed = splitmix64(master_seed ^ (s as u64));
        let mut world = World::new(seq_seed);
        world.gather = crate::gather::GatherContent::probe_fixture();
        world.combat = crate::combat::CombatContent::probe_fixture();
        world.backpack = crate::backpack::BackpackContent::probe_fixture();
        // The survival clock, on the probe with hp and respawns: its
        // fixture spans are seconds, so meters drain, empty, starve and
        // are granted again inside this window. What that buys is exactly
        // one gate and it is worth naming precisely: `test_parity_wasm`,
        // which folds this digest native and wasm and asserts the two are
        // byte-identical. `test_alloc_zero` and `test_replay` cover the
        // clock too, but they do it by installing the content in their own
        // fixtures — this probe is not what puts it there, and reading it
        // as though it were is how a coverage claim outruns its code.
        world.survival = crate::survival::SurvivalContent::probe_fixture();
        // Wet and cold (weather v0), armed on its seconds-fast fixture under
        // a storm forced at midnight on the first tick: the soak, the chill,
        // the cold's hp and the admin verb all ride the parity digest.
        world.survival.exposure = crate::exposure::ExposureContent::probe_fixture();
        world.dev_spawn = Some(world.spawn_pos(1));
        world.tick(&[
            Command::Join { id: 1 },
            Command::Join { id: 2 },
            Command::Join { id: 3 },
            Command::AdminEnv {
                weather: crate::weather::STORM,
                time_pm: 937,
            },
        ]);
        // Fixture arrangement, like the wire tests' server-side grants:
        // a weapon in hand whichever hotbar slot the bot frame selects.
        // Item 1 (12 damage) on the odd slots and item 0 (34) on the even
        // ones, so both a long trade and a three-hit kill are reachable.
        for p in world.players.iter_mut().take(3) {
            for (slot, s) in p.inv.iter_mut().take(HOTBAR_SLOTS).enumerate() {
                *s = ItemStack {
                    item: (slot % 2) as u16,
                    count: 1,
                    cond: 0,
                    skin: 0,
                };
            }
        }
        // …and the firearm over the top of the last of them, so tick 0 is
        // already armed rather than waiting on the loop's first stamp.
        arm_guns(&mut world);
        let mut rng = Pcg32::new(seq_seed, 11);
        let mut yaws = [0u16; 3];
        for t in 0..ticks {
            let f1 = bot_frame(&mut rng, yaws[0], t as u16);
            let f2 = bot_frame(&mut rng, yaws[1], t as u16);
            let f3 = bot_frame(&mut rng, yaws[2], t as u16);
            yaws = [f1.yaw, f2.yaw, f3.yaw];
            // One bot reaches for a bag every tick, in rotation. The
            // brawl drops them at everyone's feet and every respawn puts
            // the fight back on the same sand, so this is not a hopeful
            // gesture: the nearest-in-reach scan, the partial take, the
            // emptied-bag removal and the despawn sweep all land inside
            // the digest, native and wasm.
            // And one bot eats, in the same rotation and one slot behind
            // it, so the eat verb's three outcomes — a landed consume, a
            // refusal on an empty or non-food slot, and a refusal on a
            // full pair — all ride the digest too. The fixture's item 0
            // is food and a weapon at once, which is why the slots the
            // brawl fills are also the slots this reaches into.
            // And every bot presses drink, every tick. That is deliberate
            // over-pressing: the verb's answer is a *float* comparison
            // against `terrain::height` at five taps, which is exactly the
            // kind of arithmetic that goes different ways on two targets,
            // and the fixture's drink is lethal in five mouthfuls — so the
            // digest carries the dry refusal, the landed drink, the full
            // refusal, and the salt death with its respawn, on both
            // targets or on neither.
            // …and every bot answers its own death screen every tick,
            // which since wire v16 is what a respawn *is*. Unconditional,
            // because a press from a standing body is a no-op by design
            // (world.rs) — so the surface carries the wake, the corpse tick
            // that precedes it, and the press that does nothing, on both
            // targets or on neither. The beach for all three: a bag scan
            // is `probe_bags`'s subject and this world places none.
            // …and every bot swings with a different, moving lag-comp
            // favour, which is what puts the rewind on the parity surface.
            // Three phases that share no period, so in most ticks the three
            // attackers resolve the same brawl at three different depths
            // and the ring is read at every offset it holds. The moduli are
            // chosen for what they reach, not for variety: `% 9` **exceeds**
            // `REWIND_MAX_TICKS` (7), so `apply`'s clamp is exercised on
            // both targets; `% 5` and `/ 2 % 8` both include 0, so the
            // short-circuit-to-live path is too.
            //
            // This is arithmetic wall 1 has to hold: a rewound scan is the
            // same float distance test as before, run against integers read
            // out of a ring, and if the two targets ever disagreed about a
            // quantized position the brawl would diverge here first.
            //
            // Bound rather than written inline, because the shot count
            // below has to know which bots were rewound *this* tick — a
            // second copy of these three expressions beside the counter is
            // the hand-kept mirror `CLAUDE.md` warns about twice, and it
            // would drift the first time a modulus moved.
            let favours = [(t % 9) as u8, (t % 5) as u8, (t / 2 % 8) as u8];
            // The gun, back in every hand, before the tick that fires it.
            arm_guns(&mut world);
            world.tick(&[
                Command::Input {
                    id: 1,
                    frame: f1,
                    favour: favours[0],
                },
                Command::Input {
                    id: 2,
                    frame: f2,
                    favour: favours[1],
                },
                Command::Input {
                    id: 3,
                    frame: f3,
                    favour: favours[2],
                },
                Command::Loot { id: (t % 3) + 1 },
                Command::Consume {
                    id: (t % 3) + 1,
                    slot: (t % 8) as u8,
                },
                Command::Drink { id: (t % 3) + 1 },
                // …and one bot presses reload every tick, in the same
                // rotation the loot and the eat use (reload v1). Deliberate
                // over-pressing, exactly as the drink above is: most
                // presses land on a full cylinder and raise
                // `REFUSE_RL_FULL`, a press inside a shot's cadence raises
                // `REFUSE_RL_BUSY`, and the one after a burst actually
                // moves rounds out of the pack — so all three outcomes ride
                // the digest, native and wasm, or none of them do.
                //
                // The rotation and not all three, and that is a
                // *measurement* rather than a taste: a successful fill
                // pushes `next_swing` forward by `reload_ticks`, which is
                // the same field a shot pays, so three bots reloading every
                // tick spend most of the run with the arm locked and the
                // firearm barely fires. `the_guns_rewind_rides_the_parity_
                // surface` is the gate that says so — it counts rewound
                // hitscan shots and goes red at zero.
                Command::Reload { id: (t % 3) + 1 },
                Command::Respawn {
                    id: 1,
                    on_bag: false,
                },
                Command::Respawn {
                    id: 2,
                    on_bag: false,
                },
                Command::Respawn {
                    id: 3,
                    on_bag: false,
                },
            ]);
            // Every bullet that left a muzzle this tick, and the subset of
            // them whose shooter was being rewound while it did.
            //
            // `c`'s high half is the projectile speed and zero is the
            // wire's *instantaneous* marker, which makes the low half the
            // reach in decimetres — so this pair identifies a shot from the
            // fixture's one firearm row and nothing else on the code.
            //
            // The reach is **read off the table the world is running**
            // rather than written here as `20_000 / 100`. A literal would be
            // a hand-kept mirror of a constant in another module, which is
            // the drift `CLAUDE.md` names twice: move the fixture's
            // `range_mm` and the filter silently matches nothing. It fails
            // safe — the gate reads zero and goes red — but "the gate is red
            // because the probe stopped recognising its own gun" is a
            // morning spent in the wrong file.
            let reach_dm = world.combat.ranged[GUN_ITEM as usize].range_mm / 100;
            for e in world.events.entries() {
                if e.code != crate::world::EV_SHOT || e.c >> 16 != 0 {
                    continue;
                }
                if e.c & 0xFFFF != reach_dm {
                    continue;
                }
                shots = shots.saturating_add(1);
                // Ids are 1..=3 and `favours` is indexed from zero. A
                // shooter outside that range cannot happen in this world
                // and is not counted rather than being assumed.
                //
                // `favours` holds what the tick was *asked* for, and
                // `World::tick` clamps it to `Rewind::max_back()` — which is
                // why `% 9` is in the list at all. The clamp cannot turn a
                // nonzero request into zero, so `back != 0` means the same
                // thing on both sides of it.
                if let Some(&back) = favours.get(e.a.wrapping_sub(1) as usize) {
                    if back != 0 {
                        rewound = rewound.saturating_add(1);
                    }
                }
            }
        }
        h.update(&world.state_hash().to_le_bytes());
    }
    // Folded rather than returned, for the reason `probe_bags` folds its
    // corpse-ticks: the number of rounds fired is real parity surface — two
    // targets that disagreed about a cadence or an empty pack would
    // disagree here — but `rewound` is the count that carries the claim, so
    // it is the one that gets a gate.
    h.update(&shots.to_le_bytes());
    ((rewound as u64) << 32) | (h.digest() & 0xFFFF_FFFF)
}

/// A released hold followed by a complete hand revive. The upper word counts
/// actual recoveries; the lower word hashes progress, cancellation and state.
/// A second wounded body fails its roll and spends a belt recovery item;
/// the hand-revived body keeps its own. Both the parity driver and allocation
/// gate use these live write paths.
#[no_mangle]
pub extern "C" fn probe_assist(seed: u64) -> u64 {
    run_assist_probe(&mut assist_probe_world(seed))
}

/// Startup fixture kept outside the allocation gate's measured window.
pub fn assist_probe_world(seed: u64) -> World {
    use crate::movement::{Body, POS_XZ_Q};
    let mut w = World::new(seed);
    w.combat = crate::combat::CombatContent::probe_fixture();
    // Set fixture health through the normal join constructor. The probe
    // measures recovery, and no body needs a separate damage write.
    w.combat.player_hp = crate::wound::WOUNDED_HP;
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[
        Command::Join { id: 1 },
        Command::Join { id: 2 },
        Command::Join { id: 3 },
    ]);
    let a = w.players[0].body;
    w.players[1].body = Body::at(
        seed,
        &w.haven,
        a.qx as f32 * POS_XZ_Q,
        a.qz as f32 * POS_XZ_Q + 1.5,
    );
    w.tick(&[]);
    w.players[1].wounded = true;
    // The first hold crosses this deadline; releasing must leave time for
    // another attempt rather than rolling immediately on the old deadline.
    w.players[1].wound_until = w.tick + 2;
    w.survival.belt_recovery[1] = true;
    for p in &mut w.players[1..=2] {
        p.inv[0] = ItemStack {
            item: 1,
            count: 1,
            cond: 0,
            skin: 0,
        };
    }
    w.players[2].wounded = true;
    w.players[2].wound_until = (w.tick + 2..w.tick + crate::assist::ASSIST_TICKS as u64)
        .find(|&t| !crate::wound::recovers(seed, 3, t, crate::wound::RECOVER_BASE_PM))
        .expect("the probe must exercise a failed natural recovery");
    w
}

/// Execute only ticks and hashes; construction belongs to `assist_probe_world`.
pub fn run_assist_probe(w: &mut World) -> u64 {
    use crate::input::{InputFrame, BTN_ASSIST};
    use crate::movement::{POS_XZ_Q, POS_Y_Q};
    let mut hash = Xxh3::new();
    let mut recovered = 0u64;
    for t in 0..crate::assist::ASSIST_TICKS + 46 {
        let (a, b) = (w.players[0].body, w.players[1].body);
        let run = (b.qz - a.qz) as f32 * POS_XZ_Q;
        let rise = (b.qy - a.qy) as f32 * POS_Y_Q + 0.3 - 1.6;
        let mut pitch = 0;
        let mut best = f32::MIN;
        for candidate in 0..=255u8 {
            let (c, s) = crate::pitch_dir(candidate);
            let dot = c * run + s * rise;
            if dot > best {
                best = dot;
                pitch = candidate;
            }
        }
        w.tick(&[
            Command::Assist { id: 1, target: 2 },
            Command::Input {
                id: 1,
                favour: 0,
                frame: InputFrame {
                    seq: t,
                    yaw: 0,
                    pitch,
                    buttons: if t == 45 { 0 } else { BTN_ASSIST },
                    ..InputFrame::default()
                },
            },
        ]);
        for e in w.events.entries() {
            if e.code == crate::world::EV_RECOVERED && e.a == 2 {
                recovered += 1;
            }
            hash.update(&[e.code]);
            hash.update(&e.a.to_le_bytes());
            hash.update(&e.b.to_le_bytes());
            hash.update(&e.c.to_le_bytes());
        }
        hash.update(&w.state_hash().to_le_bytes());
    }
    (recovered << 32) | (hash.digest() & 0xFFFF_FFFF)
}

/// A built base for the rotation parity/allocation probe. The existing fixture
/// rows price it; the stairs and open floor copy those rows with new shapes.
pub fn rotation_probe_world() -> World {
    use crate::build::*;
    let mut w = World::new(HEADROOM_SEED);
    w.gather = crate::gather::GatherContent::probe_fixture();
    w.combat = crate::combat::CombatContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.build.piece_count = 9;
    w.build.pieces[7] = PieceDef {
        shape: SHAPE_STAIRS,
        ..w.build.pieces[0]
    };
    w.build.pieces[8] = PieceDef {
        shape: SHAPE_FLOOR_FRAME,
        ..w.build.pieces[0]
    };
    w.dev_spawn = Some(anchor(HEADROOM_CX, HEADROOM_CZ, LOC_PLANE));
    w.tick(&[Command::Join { id: 1 }]);
    w.players[0].inv[0] = ItemStack {
        item: 0,
        count: 100,
        cond: 0,
        skin: 0,
    };
    for (row, level, loc) in [
        (0, 0, LOC_PLANE),
        (7, 0, LOC_RISER),
        (1, 0, LOC_EDGE_XLO),
        (8, 1, LOC_PLANE),
    ] {
        w.tick(&[Command::Place {
            id: 1,
            row,
            cx: HEADROOM_CX,
            cz: HEADROOM_CZ,
            level,
            loc,
            freehand: true,
            plate: 1,
        }]);
    }
    w
}

/// Four stair turns and four wall flips. The upper word counts actual
/// placements, so matching hashes cannot hide a probe that only refused.
/// Construction stays outside the allocation gate's measured window.
pub fn run_rotation_probe(w: &mut World) -> u64 {
    use crate::build::*;
    let mut hash = Xxh3::new();
    let mut turns = 0u64;
    for loc in STAIR_LOCS {
        w.tick(&[
            Command::Rotate {
                id: 1,
                cx: HEADROOM_CX,
                cz: HEADROOM_CZ,
                level: 0,
                loc,
            },
            Command::Rotate {
                id: 1,
                cx: HEADROOM_CX,
                cz: HEADROOM_CZ,
                level: 0,
                loc: LOC_EDGE_XLO,
            },
        ]);
        for e in w.events.entries() {
            turns += u64::from(e.code == crate::world::EV_PIECE_PLACED);
            hash.update(&[e.code]);
            hash.update(&e.a.to_le_bytes());
            hash.update(&e.b.to_le_bytes());
            hash.update(&e.c.to_le_bytes());
        }
        hash.update(&w.state_hash().to_le_bytes());
        // A point away from both diagonals distinguishes the four ramps.
        // Derived collision is deliberately absent from state_hash.
        let x = HEADROOM_CX as f32 * BUILD_CELL_M + 0.5;
        let z = HEADROOM_CZ as f32 * BUILD_CELL_M + 1.0;
        let feet = column_floor_y(w.seed, &w.haven, HEADROOM_CX, HEADROOM_CZ, 1) + LEVEL_H_M;
        hash_f32(
            &mut hash,
            crate::collide::piece_ground(w.seed, &w.haven, w.pieces.cols(), x, z, feet),
        );
    }
    (turns << 32) | (hash.digest() & 0xFFFF_FFFF)
}

#[no_mangle]
pub extern "C" fn probe_rotation() -> u64 {
    run_rotation_probe(&mut rotation_probe_world())
}

/// Shared fixture for the parity and allocation gates. Construction allocates;
/// `run_headroom_probe` exercises covered stairs and a jump under a triangle.
pub struct HeadroomProbe {
    haven: terrain::Haven,
    cols: crate::collide::ColIndex,
    scratch: crate::occupy::Scratch<crate::occupy::Barren>,
}

const HEADROOM_SEED: u64 = 20260731;
const HEADROOM_CX: u16 = 341;
const HEADROOM_CZ: u16 = 341;

pub fn headroom_probe() -> HeadroomProbe {
    use crate::build::*;
    let haven = terrain::haven(HEADROOM_SEED);
    let mut cols = crate::collide::ColIndex::new();
    let band = terrain_band(HEADROOM_SEED, &haven, HEADROOM_CX, HEADROOM_CZ);
    for (dx, level, loc, shape) in [
        (0, 0, LOC_PLANE, SHAPE_FOUNDATION),
        (0, 0, LOC_RISER, SHAPE_STAIRS),
        (0, 1, LOC_PLANE, SHAPE_FLOOR),
        (1, 0, LOC_PLANE, SHAPE_FOUNDATION),
        (1, 1, LOC_TRI_XLO_ZLO, SHAPE_TRI_ROOF),
    ] {
        let cx = HEADROOM_CX + dx;
        let plate = (band - terrain_band(HEADROOM_SEED, &haven, cx, HEADROOM_CZ)) as i8;
        cols.add(cx, HEADROOM_CZ, level, loc, shape, plate);
    }
    // Separated columns exercise all four ramps and their open upper floor
    // under the same native/Wasm arithmetic gate as the ceiling contacts.
    for (turn, loc) in STAIR_LOCS.into_iter().enumerate() {
        let cx = HEADROOM_CX + 3 + turn as u16 * 2;
        let plate = (band - terrain_band(HEADROOM_SEED, &haven, cx, HEADROOM_CZ)) as i8;
        cols.add(cx, HEADROOM_CZ, 1, loc, SHAPE_STAIRS, plate);
        cols.add(cx, HEADROOM_CZ, 2, LOC_PLANE, SHAPE_FLOOR_FRAME, plate);
    }
    // Half-storey movement is part of the same native/Wasm and allocation
    // probe: cross low cover at ground height, then at its actual upper socket.
    for start in [12, 15] {
        for dx in [start, start + 1] {
            let cx = HEADROOM_CX + dx;
            let plate = (band - terrain_band(HEADROOM_SEED, &haven, cx, HEADROOM_CZ)) as i8;
            cols.add(
                cx,
                HEADROOM_CZ,
                if start == 12 { 0 } else { 8 },
                LOC_PLANE,
                SHAPE_FLOOR,
                plate,
            );
            if dx == start + 1 {
                cols.add(cx, HEADROOM_CZ, 0, LOC_EDGE_XLO, SHAPE_HALF_WALL, plate);
            }
        }
    }
    // Every new circulation kind enters the allocation and native/Wasm
    // fixture. Each plot uses its own terrain band so a distant hill cannot
    // bury a flight and silently remove it from the movement coverage.
    for (i, shape) in [
        SHAPE_FOUNDATION_STEPS,
        SHAPE_RAMP,
        SHAPE_STAIRS_L,
        SHAPE_STAIRS_U,
        SHAPE_STAIRS_SPIRAL,
        SHAPE_STAIRS_TRI_SPIRAL,
    ]
    .into_iter()
    .enumerate()
    {
        let cx = HEADROOM_CX + 20 + i as u16 * 2;
        let steps = shape == SHAPE_FOUNDATION_STEPS;
        let plate = if steps { PLATE_RISE_MAX_BANDS as i8 } else { 0 };
        if !steps {
            cols.add(cx, HEADROOM_CZ, 1, LOC_PLANE, SHAPE_FLOOR, plate);
        }
        cols.add(
            cx,
            HEADROOM_CZ,
            if steps { 0 } else { 1 },
            LOC_RISER,
            shape,
            plate,
        );
        cols.add(
            cx,
            HEADROOM_CZ,
            3,
            LOC_TRI_XHI_ZHI,
            SHAPE_TRI_FLOOR_FRAME,
            plate,
        );
    }
    HeadroomProbe {
        haven,
        cols,
        scratch: crate::occupy::Scratch::barren(),
    }
}

/// Upper word: bit 0 means the covered ramp stopped, bit 1 a jump hit a
/// ceiling. Lower word: every quantized body state, including the return and
/// landing. A matching hash with missing contacts cannot pass the gate.
pub fn run_headroom_probe(p: &mut HeadroomProbe) -> u64 {
    use crate::build::{column_floor_y, BUILD_CELL_M, LEVEL_H_M};
    use crate::collide::{CAPSULE_HEIGHT_M, PLANE_THICKNESS_M};
    use crate::input::{InputFrame, BTN_JUMP};
    use crate::movement::{self, quant_xz, quant_y, Body, POS_Y_Q, STEP_UP};
    let base = column_floor_y(HEADROOM_SEED, &p.haven, HEADROOM_CX, HEADROOM_CZ, 0);
    let underside = base + LEVEL_H_M - PLANE_THICKNESS_M;
    let mut hash = Xxh3::new();
    let mut contacts = 0u64;
    for route in 0..8 {
        let (dx, dz, rise) = if route == 0 {
            (1.5, 0.09, 0.09)
        } else if route == 1 {
            (BUILD_CELL_M + 0.6, 0.6, 0.0)
        } else if route >= 6 {
            let cell = if route == 6 { 13 } else { 16 };
            (
                cell as f32 * BUILD_CELL_M - 0.6,
                1.5,
                if route == 6 { 0.0 } else { LEVEL_H_M * 0.5 },
            )
        } else {
            let turn = route - 2;
            let offset = (3 + turn * 2) as f32 * BUILD_CELL_M;
            let (x, z) = match turn {
                0 => (1.5, 0.09),
                1 => (0.09, 1.5),
                2 => (1.5, 2.91),
                _ => (2.91, 1.5),
            };
            (offset + x, z, LEVEL_H_M + 0.09)
        };
        let mut body = Body {
            qx: quant_xz(HEADROOM_CX as f32 * BUILD_CELL_M + dx),
            qz: quant_xz(HEADROOM_CZ as f32 * BUILD_CELL_M + dz),
            qy: quant_y(base + rise),
            qvy: 0,
            grounded: true,
        };
        for tick in 0..80 {
            let before = body;
            let direction = if tick < 20 { 127 } else { -127 };
            let frame = InputFrame {
                move_x: match route {
                    3 | 6 | 7 => direction,
                    5 => -direction,
                    _ => 0,
                },
                move_z: if route == 0 {
                    if tick < 40 {
                        127
                    } else {
                        -127
                    }
                } else if route == 2 {
                    direction
                } else if route == 4 {
                    -direction
                } else {
                    0
                },
                buttons: if route == 1 && tick == 0 { BTN_JUMP } else { 0 },
                ..InputFrame::default()
            };
            movement::step(
                HEADROOM_SEED,
                &p.haven,
                &p.cols,
                &mut p.scratch.occupants(),
                &mut body,
                &frame,
            );
            let feet = body.qy as f32 * POS_Y_Q;
            if route == 0
                && tick < 40
                && body.qz == before.qz
                && body.grounded
                && feet > base + STEP_UP
                && feet + CAPSULE_HEIGHT_M <= underside
            {
                contacts |= 1;
            }
            if route == 1 && before.qvy > 0 && body.qvy == 0 && !body.grounded {
                contacts |= 2;
            }
            for q in [body.qx, body.qy, body.qz, body.qvy] {
                hash.update(&q.to_le_bytes());
            }
            hash.update(&[body.grounded as u8]);
        }
    }
    // Sweep the lanes and sample projectile contact too, without allocating
    // paths. Assertions about traversal live in tests/circulation.rs.
    for route in 0..6 {
        let cx = HEADROOM_CX + 20 + route * 2;
        let x = cx as f32 * BUILD_CELL_M + 0.5;
        let z = HEADROOM_CZ as f32 * BUILD_CELL_M;
        let own_base = column_floor_y(
            HEADROOM_SEED,
            &p.haven,
            cx,
            HEADROOM_CZ,
            p.cols.get(cx, HEADROOM_CZ).plate,
        );
        let start_z = match route {
            4 => 1.5,
            5 => 2.45,
            _ => 0.1,
        };
        let mut body = Body {
            qx: quant_xz(x),
            qz: quant_xz(z + start_z),
            qy: quant_y(
                own_base
                    + if route == 0 {
                        -LEVEL_H_M * 0.5 + start_z * 0.5
                    } else {
                        LEVEL_H_M
                    },
            ),
            qvy: 0,
            grounded: true,
        };
        for tick in 0..80 {
            let frame = InputFrame {
                move_z: if (tick < 40) != (route == 5) { 40 } else { -40 },
                ..InputFrame::default()
            };
            movement::step(
                HEADROOM_SEED,
                &p.haven,
                &p.cols,
                &mut p.scratch.occupants(),
                &mut body,
                &frame,
            );
            for q in [body.qx, body.qy, body.qz, body.qvy] {
                hash.update(&q.to_le_bytes());
            }
            let stop = crate::collide::shot_stop(
                HEADROOM_SEED,
                &p.haven,
                &p.cols,
                x,
                z + 1.0,
                x,
                z + 1.1,
                body.qy as f32 * POS_Y_Q - 0.1,
                crate::ranged::ARROW_R_M,
            );
            hash.update(&[stop.map_or(255, |hit| hit.loc)]);
        }
        // Both the open centroid and solid corner of the triangular frame
        // contribute, so its mask cannot disappear without moving the hash.
        for offset in [2.0, 2.9] {
            let at_x = cx as f32 * BUILD_CELL_M + offset;
            let stop = crate::collide::shot_stop(
                HEADROOM_SEED,
                &p.haven,
                &p.cols,
                at_x,
                z + offset,
                at_x,
                z + offset,
                own_base + 3.0 * LEVEL_H_M - 0.1,
                crate::ranged::ARROW_R_M,
            );
            hash.update(&[stop.map_or(255, |hit| hit.loc)]);
        }
    }
    (contacts << 32) | (hash.digest() & 0xFFFF_FFFF)
}

#[no_mangle]
pub extern "C" fn probe_headroom() -> u64 {
    run_headroom_probe(&mut headroom_probe())
}
