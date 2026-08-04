//! `test_replay` (DESIGN.md §7/§12): same build + same seed + same command
//! log → the same state hashes, every stamp. Until the server's WAL exists
//! this drives a deterministic in-memory command script — the contract is
//! identical, the fixture just isn't a file yet. The final hash is also
//! pinned: any accidental drift in sim behavior reddens this gate.

use sim_core::backpack::BackpackContent;
use sim_core::bots::bot_frame;
use sim_core::build::BuildContent;
use sim_core::craft::CraftContent;
use sim_core::gather::GatherContent;
use sim_core::limits::{STATE_HASH_INTERVAL, TICK_HZ};
use sim_core::loot::LootContent;
use sim_core::rng::Pcg32;
use sim_core::survival::SurvivalContent;
use sim_core::world::{Command, World};

const SEED: u64 = 0x0047_4154_4553; // "GATES"
const TICKS: u64 = 900;

/// Pinned end-state hash for (SEED, the script below). Regenerates only
/// with an intentional sim change, in the same commit (CLAUDE.md wall 5).
///
/// Regenerated this commit, and **behaviourally: the barrel loop runs on
/// this surface.** Bot 31 is held on four scanned barrel cells in turn and
/// swings them apart, so the number below is a function of the weighted
/// pick, the count draw, the stack rule that files the roll into a
/// container, and the container's own address and timer. Change a weight,
/// the Lemire multiply-shift, the roll-count band or `hits`, and this
/// reddens.
///
/// It first went green *without* covering any of that, which is the more
/// useful half of this note: arming `LootContent` moved the hash on barrel
/// **hits** alone — `slot_lives` is hashed, so counting a hit is visible
/// even when nothing ever breaks — and the run made zero containers. A
/// bot walks out of a barrel's 2 m reach long before the second of two
/// swings 38 ticks apart, so a teleport-once fixture reproduces hits
/// forever and a smash never. The `made > 0` assert below exists so that
/// cannot recur silently: a moved hash is not evidence, and the count is.
///
/// The regeneration before this one was **structural rather than
/// behavioural** — the distinction matters, because only one of the two is
/// evidence that a new rule runs here. The death screen put five fields on every
/// `Player` (`dead` and the four facts the wire encodes off it) and all
/// five are in the state digest, so the number below moved by ten bytes
/// per active body. It moved for no other reason: **nothing on this
/// surface can die**, so every one of those ten bytes is the value a live
/// body carries — `false`, `0`, `0`, `NO_ITEM`, `0`. The script installs
/// no `CombatContent`, so every body here has `hp_max == 0` (the drink
/// comment below says the same thing about the salt death), and a body
/// that cannot die never reaches `World::die`.
/// `test_alloc_zero` and `crates/sim-core/tests/bag_respawn.rs` own the
/// behaviour; what this gate owns is that the screen is *in the hash*,
/// which is what a WAL resumed over a corpse depends on.
///
/// The regeneration before it was structural for the same reason:
/// respawn-on-bag put the deploy store's bag cooldowns into the digest,
/// and this script stands a great many deployables, so it moved by eight
/// zero bytes per record.
///
/// The regeneration before that: **the drink verb runs on this surface.**
/// Bot 21 is stood on a scanned shoreline and presses drink every seven
/// ticks from the moment it joins, so the number below is a function of
/// the meter write, the full refusal, the dry refusal, and the five
/// `terrain::height` compares that decide which of the three a press is.
/// Move `DRINK_REACH_M`, change the tap pattern, or let a full meter pay
/// hp, and this reddens.
///
/// The regeneration before it: **the survival clock started running on
/// this surface** — 64 bodies draining every tick and two of them eating,
/// which is what made the hash a function of the drain arithmetic rather
/// than of its absence. Change a numerator, a denominator or the
/// drain→hurt→heal order and this reddens too.
///
/// The two before those were structural rather than behavioural: the
/// survival module's fields entering `state_hash` while the script left
/// them all zero, and before that the death backpack's two zero-length
/// store fields.
const GOLDEN_FINAL_HASH: u64 = 0x533D_812B_6F76_BA08;

/// A standable point with sea inside `DRINK_REACH_M`, scanned off the
/// heightfield rather than typed in — the same reason `walk_up_the_beach`
/// walks instead of teleporting to a remembered coordinate: a number that
/// held at one seed and one set of generator constants is a fixture that
/// silently stops meaning what it says.
fn shoreline(seed: u64) -> (f32, f32) {
    let r = sim_core::survival::DRINK_REACH_M;
    let mut x = 0.0f32;
    while x < sim_core::terrain::ISLAND_SIZE {
        let mut z = 0.0f32;
        while z < sim_core::terrain::ISLAND_SIZE {
            let h = sim_core::terrain::height(seed, x, z);
            if (sim_core::terrain::SEA_LEVEL..sim_core::terrain::BEACH_MAX_H).contains(&h)
                && (sim_core::terrain::height(seed, x + r, z) < sim_core::terrain::SEA_LEVEL
                    || sim_core::terrain::height(seed, x - r, z) < sim_core::terrain::SEA_LEVEL
                    || sim_core::terrain::height(seed, x, z + r) < sim_core::terrain::SEA_LEVEL
                    || sim_core::terrain::height(seed, x, z - r) < sim_core::terrain::SEA_LEVEL)
            {
                return (x, z);
            }
            z += 4.0;
        }
        x += 4.0;
    }
    panic!("this island has no coast — the generator changed under this gate");
}

/// The first `n` barrel cells on the island, scanned off `terrain::scatter`
/// rather than typed in — same reason `shoreline` scans: a cell that held
/// a barrel at one seed and one weight table is a fixture that silently
/// stops meaning what it says.
///
/// Returns the barrel's own world position, because the smasher is stood
/// exactly on it: `POINT_BLANK_M2` bypasses the aim cone, so the swing
/// lands without the script also having to reproduce a yaw.
fn barrel_cells(seed: u64, scatter: &sim_core::terrain::ScatterTable, n: usize) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    // The same haven the sim resolves at init: it vetoes scatter, so a script
    // built without it could aim a swing at a barrel the world never placed.
    let haven = sim_core::terrain::haven(seed);
    let span = (sim_core::terrain::ISLAND_SIZE / sim_core::terrain::CELL_SIZE) as i32;
    let mut cz = 0;
    while cz < span && out.len() < n {
        let mut cx = 0;
        while cx < span && out.len() < n {
            let s = sim_core::terrain::scatter(seed, scatter, &haven, cx, cz);
            if s.occupant == sim_core::terrain::Occupant::BarrelSlot {
                out.push((s.x, s.z));
            }
            cx += 1;
        }
        cz += 1;
    }
    assert!(
        !out.is_empty(),
        "this island has no barrels — the scatter table changed under this gate"
    );
    out
}

/// Stand a kitted bot on ground that will hold a foundation.
///
/// The beach spawn ring puts a fresh player at ~1.2 m and a foundation
/// wants 1.5 m (`build::FOUNDATION_MIN_H_M`), so before this the whole
/// scripted build/deploy/door/lock/upgrade arc below depended on where a
/// five-second random walk happened to stop — which held at this seed and
/// collapsed to one placement at `SEED ^ 1`. Stepping inland along the ray
/// to island center until the cell center is buildable is what a player
/// does at a fresh spawn, done deterministically: same seed, same result,
/// and the same `foundation_terrain_ok` the sim refuses on, so the fixture
/// cannot drift away from the rule.
///
/// A fixture arrangement, like the inventory grants beside it — applied
/// identically on both runs, so the replay contract is untouched.
fn walk_up_the_beach(world: &mut World, seed: u64, slot: usize) {
    let b = world.players[slot].body;
    let mut x = b.qx as f32 * sim_core::movement::POS_XZ_Q;
    let mut z = b.qz as f32 * sim_core::movement::POS_XZ_Q;
    let c = sim_core::terrain::ISLAND_SIZE * 0.5;
    let (mut dx, mut dz) = (c - x, c - z);
    let d = (dx * dx + dz * dz).sqrt();
    if d <= 0.0 {
        return;
    }
    dx /= d;
    dz /= d;
    // Bounded: 200 steps of 3 m is 600 m, more than the beach-to-center
    // distance, so the loop is a search and never a walk to nowhere.
    for _ in 0..200 {
        let cx = sim_core::build::build_cell_of(x);
        let cz = sim_core::build::build_cell_of(z);
        let ax = (cx as f32 + 0.5) * sim_core::build::BUILD_CELL_M;
        let az = (cz as f32 + 0.5) * sim_core::build::BUILD_CELL_M;
        if sim_core::build::foundation_terrain_ok(seed, ax, az) {
            break;
        }
        x += dx * sim_core::build::BUILD_CELL_M;
        z += dz * sim_core::build::BUILD_CELL_M;
    }
    world.players[slot].body = sim_core::movement::Body::at(seed, x, z);
}

fn run(seed: u64) -> (Vec<u64>, u64) {
    let mut world = World::new(seed);
    world.gather = GatherContent::probe_fixture();
    world.craft = CraftContent::probe_fixture();
    world.build = BuildContent::probe_fixture();
    world.deploy = sim_core::deploy::DeployContent::probe_fixture();
    // Barrels, on the replayed surface. Both halves are needed or the
    // gate watches nothing: the loot table decides a barrel is breakable
    // at all, and the despawn ladder is what lets the container it breaks
    // into actually stand up. Armed together, a smashed barrel writes to
    // both hashed stores — `slot_lives` (the slot's bit and its jittered
    // respawn) and `backpacks` (the container, its address, its timer and
    // its rolled contents) — so the weighted walk, the count draw and the
    // stacking rule are all functions of the number below.
    world.loot = LootContent::probe_fixture();
    world.backpack = BackpackContent::probe_fixture();
    let barrels = barrel_cells(seed, &world.scatter, 4);
    // The clock, on the replayed surface: meters granted at join, drained
    // every tick by the rational accumulator, and eaten back up by the
    // scripted consumes below — so a change to `survival::step`'s
    // arithmetic moves the pinned hash instead of passing unnoticed.
    //
    // The spans are widened off the fixture's seconds, and the reason is
    // this script rather than the ring: at 10 s / 8 s every body on the
    // island empties inside the first 240 ticks and starts dying of it
    // around tick 440, and a body that dies wakes up somewhere else —
    // which would quietly move the builder, the hearth's feeder and the
    // door's owner off the arc this script exists to pin. Wider than the
    // 900 ticks it runs, nobody empties, nobody starves, and every body
    // the script addresses is still standing where the script left it.
    // The meter floor at the end is what holds that claim.
    let mut survival = SurvivalContent::probe_fixture();
    survival.food_span_ticks = 90 * TICK_HZ;
    survival.water_span_ticks = 72 * TICK_HZ;
    world.survival = survival;
    let mut rng = Pcg32::new(seed, 11);
    let mut yaws = [0u16; 64];
    let mut joined: u32 = 0;
    let mut hashes = Vec::new();
    let (mut placed, mut deployed, mut decayed, mut doors) = (0u32, 0u32, 0u32, 0u32);
    let (mut locked_seen, mut unlocked_seen, mut upgraded_seen) = (false, false, false);
    let (mut eaten, mut eat_refused) = (0u32, 0u32);
    let (mut drank, mut drink_refused) = (0u32, 0u32);
    let mut hearth_cell = (0u16, 0u16);

    for t in 0..TICKS {
        let mut cmds: Vec<Command> = Vec::new();
        if t % 9 == 0 && joined < 64 {
            joined += 1;
            cmds.push(Command::Join { id: joined });
        }
        if t == 450 {
            cmds.push(Command::Leave { id: 3 });
        }
        if t == 500 {
            cmds.push(Command::Join { id: 3 }); // slot reuse is part of the contract
        }
        for id in 1..=joined {
            if (450..500).contains(&t) && id == 3 {
                continue;
            }
            let f = bot_frame(&mut rng, yaws[id as usize - 1], t as u16);
            yaws[id as usize - 1] = f.yaw;
            cmds.push(Command::Input { id, frame: f });
            // The craft verb rides the same log: periodic enqueues (row 3
            // is out of range — the refusal path) and rarer cancels.
            if (t + id as u64).is_multiple_of(37) {
                cmds.push(Command::Craft {
                    id,
                    recipe: ((t / 37 + id as u64) % 4) as u16,
                    count: 1 + (id as u64 % 3) as u16,
                });
            }
            if (t + id as u64).is_multiple_of(149) {
                cmds.push(Command::CraftCancel {
                    id,
                    index: (t % 4) as u16,
                });
            }
            // The build verb rides the log too: places at the player's own
            // cell (successes once wood accrues) plus out-of-range rows and
            // mismatched locs (the refusal paths).
            if (t + id as u64).is_multiple_of(53) {
                let b = &world.players[(id as usize - 1) % 64].body;
                let cell = |q: i32| {
                    sim_core::build::build_cell_of(q as f32 * sim_core::movement::POS_XZ_Q)
                        .clamp(0, 1023) as u16
                };
                // Row 5 is out of range (the fixture is 5 rows since the
                // stone rung joined it) — the refusal path stays in
                // surface.
                cmds.push(Command::Place {
                    id,
                    row: ((t / 53 + id as u64) % 6) as u16,
                    cx: cell(b.qx),
                    cz: cell(b.qz),
                    level: ((t / 106) % 2) as u8,
                    loc: ((t / 53 + id as u64) % 4) as u8,
                });
            }
            // The deploy verb too: a bag and a workbench at the player's
            // feet (the success shapes, once the granted or gathered
            // items are there), a rotating junk request for the refusal
            // reasons, and a feed (mostly the no-hearth refusal).
            if (t + id as u64).is_multiple_of(71) {
                let b = &world.players[(id as usize - 1) % 64].body;
                let cell = |q: i32| {
                    sim_core::build::build_cell_of(q as f32 * sim_core::movement::POS_XZ_Q)
                        .clamp(0, 1023) as u16
                };
                let (cx, cz) = (cell(b.qx), cell(b.qz));
                cmds.push(Command::PlaceDeploy {
                    id,
                    row: 3,
                    cx,
                    cz,
                    level: 0,
                    loc: 0,
                });
                cmds.push(Command::PlaceDeploy {
                    id,
                    row: 1,
                    cx: (cx + 1).min(1023),
                    cz,
                    level: 0,
                    loc: 0,
                });
                cmds.push(Command::PlaceDeploy {
                    id,
                    row: ((t / 71 + id as u64) % 5) as u16,
                    cx,
                    cz,
                    level: ((t / 142) % 2) as u8,
                    loc: ((t / 71 + id as u64) % 4) as u8,
                });
                cmds.push(Command::Feed {
                    id,
                    cx,
                    cz,
                    level: 0,
                });
            }
            // The eat verb rides the log too, on two of the eight bots the
            // kit at t=149 reaches and neither of the two the hearth arc
            // addresses: id 2 eats slot 20, which that kit fills with
            // fixture item 0 — the one item the survival fixture makes food
            // — and id 4 reaches for slot 21, which it fills with item 1,
            // which is not. The landed consume and the announced refusal,
            // both on the replayed surface; before t=149 both slots are
            // empty, so the early ticks ride the refusal too.
            if id == 2 && (t + id as u64).is_multiple_of(41) {
                cmds.push(Command::Consume { id, slot: 20 });
            }
            if id == 4 && (t + id as u64).is_multiple_of(43) {
                cmds.push(Command::Consume { id, slot: 21 });
            }
            // The drink verb rides it too, on bot 21 — which joins at
            // t=180, carries no kit, founds nothing and is addressed by no
            // other scripted arm. It is stood on a scanned shoreline at
            // t=200 (below); before that the presses land on wherever the
            // ring put it, which is the refusal path. So the landed drink,
            // the dry refusal and the full refusal are all on the replayed
            // surface, and a change to the verb's arithmetic — the meter
            // write, or the five `terrain::height` compares that decide
            // whether there is water — moves the pinned hash.
            //
            // **The salt death is deliberately NOT on this surface**, and
            // that is a fact about the fixture rather than about the verb:
            // this script installs no `CombatContent`, so every body here
            // has `hp_max == 0`, and a cost clamped to a body's own hp is
            // zero. `test_alloc_zero` and `tests/survival.rs` own the kill
            // site; arming combat here to reach it would put 64 bots in a
            // brawl through the middle of the build arc this script exists
            // to pin. Every 7 ticks and
            // not every 29 on purpose: the widened water span drops one
            // unit every ~22 ticks, so a 7-tick cadence finds the meter
            // still full on most presses — which is how the *full* refusal
            // gets onto the replayed surface at all, rather than only in
            // the unit tests.
            if id == 21 && (t + id as u64).is_multiple_of(7) {
                cmds.push(Command::Drink { id });
            }
        }
        // A scripted hearth: grant a kit to the first eight bots (a
        // fixture arrangement, like the wire tests' server-side grants —
        // identical on both runs, so the replay contract holds), then
        // bot 1 founds, hearths, and feeds one remembered cell. This
        // pins the pay path — everything unpaid decays by the leaps.
        if t == 149 {
            for w in 0..8usize {
                if world.players[w].active {
                    for (k, &(item, count)) in [(0u16, 200u16), (1, 200), (2, 50), (3, 50), (4, 50)]
                        .iter()
                        .enumerate()
                    {
                        world.players[w].inv[20 + k] = sim_core::gather::ItemStack { item, count };
                    }
                    walk_up_the_beach(&mut world, seed, w);
                }
            }
        }
        // The drinker, stood on a shoreline scanned off the heightfield —
        // a fixture arrangement like the kit grant above, identical on both
        // runs, so the replay contract holds. Scanned rather than typed in
        // because a hard-coded coast goes stale the first time the
        // generator's constants move, and a drinker staged inland would
        // turn the landed-drink floor below into a coin flip.
        if t == 200 && world.players[20].active {
            let (x, z) = shoreline(seed);
            world.players[20].body = sim_core::movement::Body::at(seed, x, z);
        }
        // The smasher. Held on a scanned barrel rather than teleported once,
        // because a barrel wants two landed swings 38 ticks apart and a bot
        // walks out of its 2 m reach long before the second: teleport-once
        // reproduced barrel *hits* and never a single smash, which is how
        // this surface first went green while covering nothing. Rotating it
        // through four barrels every 200 ticks means four different cells
        // roll four different tables, so the weighted walk is in the hash
        // and not just the slot bit.
        if world.players[30].active {
            let (x, z) = barrels[(t as usize / 200) % barrels.len()];
            world.players[30].body = sim_core::movement::Body::at(seed, x, z);
        }
        if t == 150 {
            let b = &world.players[0].body;
            let cell = |q: i32| {
                sim_core::build::build_cell_of(q as f32 * sim_core::movement::POS_XZ_Q)
                    .clamp(0, 1023) as u16
            };
            hearth_cell = (cell(b.qx), cell(b.qz));
        }
        if (150..=164).contains(&t) {
            let (cx, cz) = hearth_cell;
            let id = world.players[0].id;
            match t {
                150 => cmds.push(Command::Place {
                    id,
                    row: 0,
                    cx,
                    cz,
                    level: 0,
                    loc: 0,
                }),
                151 => cmds.push(Command::PlaceDeploy {
                    id,
                    row: 0,
                    cx,
                    cz,
                    level: 0,
                    loc: 0,
                }),
                // A doorway on the same cell's west edge, a door in it,
                // then the door verbs' whole arc — placement seals the
                // edge locked, its owner's toggles open and reseal it,
                // and the lock verb rides both ways (a stranger's lock
                // attempt in between, refused) — so the bodies that walk
                // that edge afterwards feel each state. All of it before
                // the feeds, which hand the same wood to the hearth.
                152 => cmds.push(Command::Place {
                    id,
                    row: 3,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_W,
                }),
                153 => cmds.push(Command::PlaceDeploy {
                    id,
                    row: 2,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_W,
                }),
                154 | 157 => cmds.push(Command::Use {
                    id,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_W,
                }),
                // Then the upgrade verb's own arc on a second wall: a
                // wood wall at the north edge, climbed to the fixture's
                // stone rung, then asked back down (the tier refusal),
                // then asked for by a second bot — which bounces on
                // **reach**, not on the hearth's claim: the two have
                // wandered ~90 m apart by now and the reach test comes
                // first. The claim refusal is `build::tests::
                // upgrade_answers_to_a_foreign_claim`'s to assert; what
                // rides here is the re-row and two of the bounces. All
                // of it before the feeds, which hand the builder's whole
                // stock to the hearth.
                159 => cmds.push(Command::Place {
                    id,
                    row: 1,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_N,
                }),
                160 | 161 => cmds.push(Command::Upgrade {
                    id,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_N,
                    material: if t == 160 {
                        sim_core::build::MAT_STONE
                    } else {
                        sim_core::build::MAT_WOOD
                    },
                }),
                162 => cmds.push(Command::Upgrade {
                    id: world.players[1].id,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_N,
                    material: sim_core::build::MAT_METAL,
                }),
                155 | 156 | 158 => cmds.push(Command::Lock {
                    // 156 is a hand that does not own this door — the
                    // refusal path, on the replayed surface too.
                    id: if t == 156 { world.players[1].id } else { id },
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_W,
                    locked: t == 158,
                }),
                _ => cmds.push(Command::Feed {
                    id,
                    cx,
                    cz,
                    level: 0,
                }),
            }
        }
        // Leap the clock three upkeep periods on a cadence: over the run
        // that is ~40 periods, enough to decay unpaid fixture pieces
        // (100 hp − 5/period) all the way to removal — charge, decay,
        // and the removal cascade all inside the replayed surface.
        if t % 64 == 63 {
            world.tick += 3 * sim_core::deploy::UPKEEP_PERIOD_TICKS;
        }
        world.tick(&cmds);
        // The scripted upgrade's own verdict, read from the store the
        // tick after it ran: an event says a piece was announced, only
        // the record says which rung it stands on now.
        if t == 160 {
            upgraded_seen = world
                .pieces
                .find(hearth_cell.0, hearth_cell.1, 0, sim_core::build::LOC_EDGE_N)
                .is_some_and(|p| p.row == 4);
        }
        for e in world.events.entries() {
            match e.code {
                sim_core::world::EV_PIECE_PLACED => placed += 1,
                sim_core::world::EV_DEPLOY_PLACED => deployed += 1,
                sim_core::world::EV_PIECE_REMOVED | sim_core::world::EV_DEPLOY_REMOVED => {
                    decayed += 1
                }
                sim_core::world::EV_CONSUMED => eaten += 1,
                // One refusal code, two verbs — so the counters partition
                // by the body that pressed. Ids 2 and 4 are the only ones
                // the eat script addresses, 21 the only drinker. Counting
                // the union would sink the eat floor below: bot 21 presses
                // every 7 ticks from t=180 and most of those find a full
                // meter, so its refusals alone clear `eat_refused >= 8`
                // and the assert stops being able to fail on the verb its
                // own message names.
                sim_core::world::EV_CONSUME_REFUSED => match e.a {
                    2 | 4 => eat_refused += 1,
                    21 => drink_refused += 1,
                    _ => {}
                },
                sim_core::world::EV_DRANK => drank += 1,
                sim_core::world::EV_DOOR => {
                    doors += 1;
                    if e.b & 2 == 0 {
                        unlocked_seen = true;
                    } else {
                        locked_seen = true;
                    }
                }
                _ => {}
            }
        }
        if world.tick.is_multiple_of(STATE_HASH_INTERVAL) {
            hashes.push(world.last_hash);
        }
    }
    // Counted from events, not the final stores: decay legitimately
    // removes early placements before the run ends.
    // Floors, not `> 0`: a script that lands one placement exercises the
    // build path about as well as one that lands none — and that is
    // exactly what the beach ring did to `SEED ^ 1` before
    // `walk_up_the_beach` staged the kitted bots, while `> 0` stayed
    // green. Measured this commit at placed 8 / deployed 50 / decayed 28
    // at `SEED` and 5 / 34 / 18 at `SEED ^ 1`; the floors sit under both,
    // where thinning stops being incidental and becomes a lost gate.
    assert!(
        placed >= 4,
        "the script placed {placed} pieces — the build success path is falling out \
         of the replay surface"
    );
    assert!(
        deployed >= 16,
        "the script deployed {deployed} — the deploy success path is falling out \
         of the replay surface"
    );
    assert!(
        decayed >= 8,
        "only {decayed} decayed away — the removal path is falling out of the \
         replay surface"
    );
    assert!(
        doors >= 4,
        "the scripted door never toggled — the use verb fell out of the replay surface"
    );
    assert!(
        upgraded_seen,
        "the scripted wall never reached its stone rung — the upgrade verb fell \
         out of the replay surface"
    );
    assert!(
        locked_seen && unlocked_seen,
        "the scripted door never changed hands both ways — the lock verb fell out \
         of the replay surface"
    );
    // The clock's own three claims on this surface, each read off the thing
    // it moves. Content installed and never exercised would hash the same
    // as content that ran, which is the whole failure this block forecloses.
    let witness = &world.players[9]; // joins at t=81, eats nothing, is addressed by no scripted arm
                                     // Strictly inside the ceiling at both ends, and both ends matter: at the
                                     // ceiling means nothing drained, at zero means the meter was never
                                     // granted — which is exactly the state inert content leaves, and the
                                     // state the previous commit's script hashed while claiming coverage.
    assert!(
        witness.food > 0
            && witness.food < survival.max_food
            && witness.water > 0
            && witness.water < survival.max_water,
        "the drain witness reads food {} water {} against a ceiling of {} / {} \
         — a meter at its ceiling never drained and a meter at zero was never \
         granted; either way survival::step is not on the replay surface",
        witness.food,
        witness.water,
        survival.max_food,
        survival.max_water
    );
    assert!(
        world
            .players
            .iter()
            .all(|p| !p.active || (p.food > 0 && p.water > 0)),
        "a body ran its meters dry inside the script — the widened spans above \
         no longer hold, and a starvation respawn is about to move a body the \
         build arc depends on"
    );
    assert!(
        eaten >= 8 && eat_refused >= 8,
        "the script landed {eaten} consumes and {eat_refused} refusals — the eat \
         verb is falling out of the replay surface"
    );
    // Floors, not `> 0`, for the same reason the build ones are floors: a
    // script that lands one drink exercises the verb about as well as one
    // that lands none. Both halves are named because both are arithmetic
    // the pinned hash is now a function of — the meter write and the hp
    // debit on one side, five `terrain::height` compares on the other.
    assert!(
        drank >= 4 && drink_refused >= 4,
        "the script landed {drank} drinks and {drink_refused} refusals — the \
         drink verb is falling out of the replay surface"
    );
    assert_eq!(
        world.players[20].hp_max, 0,
        "the drinker has hp on this surface now — the drink's hp debit is live \
         here and the comment above it, which says it is not, has gone stale"
    );
    // The barrel loop ran on this surface, and the golden below is only
    // evidence of the roll while this holds. Bots swing on a beach where a
    // quarter of the cells hold a barrel, so an empty container store means
    // the smash never fired and the pinned hash has quietly stopped
    // watching it — the exact failure the loot slice found here, where the
    // gate went green because the fixture was unarmed rather than because
    // the code was right.
    // `next_id` is monotonic from 1, so it counts containers *created*
    // rather than containers still standing — the fixture's despawn ladder
    // is short enough that an end-state count would read zero on a surface
    // that smashed plenty.
    let made = world.backpacks.next_id() - 1;
    assert!(
        made >= 2,
        "only {made} containers stood up in {TICKS} ticks — the barrel smash \
         is not running on this surface and the golden no longer covers it"
    );
    (hashes, world.state_hash())
}

#[test]
fn test_replay() {
    let (hashes_a, final_a) = run(SEED);
    let (hashes_b, final_b) = run(SEED);

    assert_eq!(hashes_a.len() as u64, TICKS / STATE_HASH_INTERVAL);
    assert_eq!(
        hashes_a, hashes_b,
        "replay diverged: same seed + same commands must reproduce every stamped hash"
    );
    assert_eq!(final_a, final_b);
    assert_eq!(
        final_a, GOLDEN_FINAL_HASH,
        "sim behavior drifted from the pinned replay golden; if intentional, \
         regenerate the golden in this same commit"
    );

    // A different seed must actually change the world (guards against a
    // degenerate hash or a sim that ignores its inputs).
    let (_, final_other) = run(SEED ^ 1);
    assert_ne!(final_a, final_other);
}
