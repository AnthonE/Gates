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

/// Golden terrain fingerprint: 64×64 heights sampled on a 32 m grid, then
/// the 256 scatter cells of the island-center 16×16 block (cells 120..136;
/// the corner block would be all sea and pin nothing). Hashes f32 bit
/// patterns — bit-identical or bust.
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
    for cz in 120..136i32 {
        for cx in 120..136i32 {
            let s = terrain::scatter(seed, &table, cx, cz);
            h.update(&[s.occupant as u8, s.yaw]);
            h.update(&s.x.to_bits().to_le_bytes());
            h.update(&s.y.to_bits().to_le_bytes());
            h.update(&s.z.to_bits().to_le_bytes());
            h.update(&s.scale.to_bits().to_le_bytes());
        }
    }
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
            };
            // Bot 2 pokes the deploy verb at its own feet: row 4 is out
            // of range and loc/level cycle, so placements AND every
            // deploy refusal reason ride the surface; feed mostly hits
            // the no-hearth refusal, and the successes cover stock. The
            // gated craft (recipe 2) arms once bot 2's workbench stands.
            let own2 = {
                let b = &world.players[1].body;
                let cx = crate::build::build_cell_of(b.qx as f32 * crate::movement::POS_XZ_Q);
                let cz = crate::build::build_cell_of(b.qz as f32 * crate::movement::POS_XZ_Q);
                (cx.clamp(0, 1023) as u16, cz.clamp(0, 1023) as u16)
            };
            let place_deploy = Command::PlaceDeploy {
                id: 2,
                row: ((t / 16) % 5) as u16,
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
                    Command::Input { id: 1, frame: f1 },
                    Command::Input { id: 2, frame: f2 },
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
                    },
                ]);
                continue;
            }
            if t % 16 == 3 {
                world.tick(&[
                    Command::Input { id: 1, frame: f1 },
                    Command::Input { id: 2, frame: f2 },
                    place_deploy,
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
                    Command::Input { id: 1, frame: f1 },
                    Command::Input { id: 2, frame: f2 },
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
                    Command::Input { id: 1, frame: f1 },
                    Command::Input { id: 2, frame: f2 },
                    craft,
                    Command::Craft {
                        id: 2,
                        recipe: 0,
                        count: 1,
                    },
                ]);
            } else if t % 16 == 11 {
                world.tick(&[
                    Command::Input { id: 1, frame: f1 },
                    Command::Input { id: 2, frame: f2 },
                    place,
                ]);
            } else if t % 16 == 13 {
                // Bot 2 pokes the use verb at its own cell's edges: no
                // door there almost always (the door refusal path), the
                // real toggle when its wanderings placed one — either
                // way the door command is inside the parity/replay/alloc
                // surface (loc 2/3 cycle; 0/1 hit the not-a-door arm).
                world.tick(&[
                    Command::Input { id: 1, frame: f1 },
                    Command::Input { id: 2, frame: f2 },
                    Command::Use {
                        id: 2,
                        cx: own2.0,
                        cz: own2.1,
                        level: ((t / 64) % 2) as u8,
                        loc: ((t / 16) % 4) as u8,
                    },
                ]);
            } else if t % 64 == 20 {
                world.tick(&[
                    Command::Input { id: 1, frame: f1 },
                    Command::Input { id: 2, frame: f2 },
                    cancel,
                ]);
            } else {
                world.tick(&[
                    Command::Input { id: 1, frame: f1 },
                    Command::Input { id: 2, frame: f2 },
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
    for s in 0..sequences {
        let seq_seed = splitmix64(master_seed ^ (s as u64));
        let mut world = World::new(seq_seed);
        world.combat = crate::combat::CombatContent::probe_fixture();
        // Seconds-long spans: a body empties, starves, dies and wakes
        // several times inside one sequence, which is what puts the scan,
        // the cooldown and the ring fallback all on this surface.
        world.survival = crate::survival::SurvivalContent::probe_fixture();
        world.deploy = crate::deploy::DeployContent::probe_fixture();
        world.dev_spawn = Some(buildable_near(seq_seed, world.spawn_pos(1)));
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
            world.tick(&[
                Command::Input { id: 1, frame: f1 },
                Command::Input { id: 2, frame: f2 },
                Command::Input { id: 3, frame: f3 },
                Command::PlaceDeploy {
                    id: placer as u32 + 1,
                    row: 3,
                    cx,
                    cz,
                    level: 0,
                    loc: crate::build::LOC_PLANE,
                },
            ]);
            for e in world.events.entries() {
                if e.code == crate::world::EV_RESPAWN && e.b == 1 {
                    wakes = wakes.saturating_add(1);
                }
            }
        }
        h.update(&world.state_hash().to_le_bytes());
    }
    ((wakes as u64) << 32) | (h.digest() & 0xFFFF_FFFF)
}

/// The first cell center at or inland of `(x, z)` that will hold a
/// ground-class deployable, stepping toward the island center. Bounded at
/// 200 steps of one build cell — 600 m, further than beach-to-center — so
/// it is a search and never a walk to nowhere. No trig and no `sqrt`: the
/// step direction is normalized by its own largest component, which is a
/// division (wall 1) and moves along the same ray.
fn buildable_near(seed: u64, (x, z): (f32, f32)) -> (f32, f32) {
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
        if crate::build::foundation_terrain_ok(seed, ax, az) {
            return (ax, az);
        }
        px += dx * crate::build::BUILD_CELL_M;
        pz += dz * crate::build::BUILD_CELL_M;
        i += 1;
    }
    (x, z)
}

/// Combat parity: `sequences` independent three-bot brawls, each a fresh
/// world × `ticks` ticks, folded into one digest — the melee half of
/// `test_parity_wasm`.
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
#[no_mangle]
pub extern "C" fn probe_combat(master_seed: u64, sequences: u32, ticks: u32) -> u64 {
    let mut h = Xxh3::new();
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
        world.dev_spawn = Some(world.spawn_pos(1));
        world.tick(&[
            Command::Join { id: 1 },
            Command::Join { id: 2 },
            Command::Join { id: 3 },
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
                };
            }
        }
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
            world.tick(&[
                Command::Input { id: 1, frame: f1 },
                Command::Input { id: 2, frame: f2 },
                Command::Input { id: 3, frame: f3 },
                Command::Loot { id: (t % 3) + 1 },
                Command::Consume {
                    id: (t % 3) + 1,
                    slot: (t % 8) as u8,
                },
                Command::Drink { id: (t % 3) + 1 },
            ]);
        }
        h.update(&world.state_hash().to_le_bytes());
    }
    h.digest()
}
