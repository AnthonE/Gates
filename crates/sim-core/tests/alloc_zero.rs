//! `test_alloc_zero` (DESIGN.md §12): 100 bots × 300 ticks after warmup,
//! heap alloc/free count delta == 0, measured by a counting GlobalAlloc.
//! CLAUDE.md wall 2. This binary is the gate; nothing else runs in it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use sim_core::backpack::{BackpackContent, BAG_GONE_EMPTIED};
use sim_core::bots::bot_frame;
use sim_core::build::{
    BuildContent, LOC_EDGE_N, LOC_EDGE_W, LOC_PLANE, MAT_METAL, MAT_STONE, MAT_WOOD,
};
use sim_core::combat::CombatContent;
use sim_core::craft::CraftContent;
use sim_core::deploy::DeployContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{INV_SLOTS, MAX_PLAYERS};
use sim_core::rng::Pcg32;
use sim_core::survival::{SurvivalContent, REFUSE_C_NO_WATER};
use sim_core::world::{
    Command, World, EV_BAG_DROPPED, EV_BAG_REMOVED, EV_CONSUMED, EV_CONSUME_REFUSED, EV_DEATH,
    EV_DRANK, EV_PIECE_REMOVED, EV_RESPAWN, EV_STRUCT_HIT,
};

/// One tick's commands: every bot's input plus a craft enqueue, a cancel,
/// a place, an upgrade, a repair, a charge plant, and an eat, so the
/// craft, build, raid and survival verbs sit inside the counted window.
/// Fixed-size — the test itself must not allocate.
///
/// `cell` is bot 1's own build cell, recomputed every tick by the caller,
/// so the place and upgrade requests are always inside the 5 m reach and
/// the write paths — the store insert, the cost payment, the re-row — are
/// what gets counted, not a reach refusal. The place cycles an 8-tick
/// figure: a foundation, a wall on its west edge, a doorway on its north,
/// a floor above, then four requests shaped to be refused (a row past the
/// table, a foundation off the ground, a wall in a plane slot, a floor at
/// ground level). The upgrade rides the wall's address with the material
/// cycling wood → stone → metal: the sideways refusal, the one rung the
/// fixture holds (`row_of`, the cost loop, `set_row`), and the
/// missing-rung refusal — plus the empty-address refusal on every tick
/// the wall is not standing yet.
///
/// The repair rides that same address with the store bit alternating, so
/// both branches are counted: they read different tables and call
/// different setters, and only one of them existed before v21. Whether a
/// given tick's repair lands or refuses is not the point — neither path
/// may reach the allocator.
///
/// The charge plant rides the same address with the bit the other way, and
/// it is the one arm here whose work does not finish on the tick that
/// asked for it: `tick_fuses` runs every tick of the window, over an empty
/// store on most of them and into `damage_piece` on the rest. Both cases
/// are counted, which is the point — a per-tick scan that allocated only
/// when it found something would pass a gate that never planted one.
///
/// The eat rides bot 1's own stock on a 3-tick cycle: slot 20 holds
/// fixture item 0, which the survival fixture makes food, so that arm is
/// the landed consume — the stack decrement, the meter clamp, the heal
/// ramp start and the announcement. Slot 21 holds item 1, which is not
/// food, and the third arm asks for twice `INV_SLOTS`, which is past the
/// inventory entirely: the two refusal shapes, so the window counts what
/// the verb does when it says no as
/// well as when it says yes. Bot 1 carries 60 000 of each, so 100 units
/// eaten over the window cannot bounce a placement on cost.
fn tick_cmds(
    rng: &mut Pcg32,
    yaws: &mut [u16; MAX_PLAYERS],
    t: u16,
    cell: (u16, u16),
) -> [Command; MAX_PLAYERS + 7] {
    core::array::from_fn(|i| {
        if i < MAX_PLAYERS {
            let f = bot_frame(rng, yaws[i], t);
            yaws[i] = f.yaw;
            Command::Input {
                id: i as u32 + 1,
                frame: f,
            }
        } else if i == MAX_PLAYERS {
            Command::Craft {
                id: (t as u32 % MAX_PLAYERS as u32) + 1,
                recipe: t % 4, // 3 is out of range: the refusal path counts too
                count: 1 + t % 2,
            }
        } else if i == MAX_PLAYERS + 1 {
            Command::CraftCancel {
                id: ((t as u32 * 7) % MAX_PLAYERS as u32) + 1,
                index: t % 5,
            }
        } else if i == MAX_PLAYERS + 2 {
            let (row, level, loc) = match t % 8 {
                0 => (0, 0, LOC_PLANE),
                1 => (1, 0, LOC_EDGE_W),
                2 => (3, 0, LOC_EDGE_N),
                3 => (2, 1, LOC_PLANE),
                4 => (5, 0, LOC_PLANE), // past the table
                5 => (0, 1, LOC_PLANE), // a foundation off the ground
                6 => (1, 0, LOC_PLANE), // a wall in a plane slot
                _ => (2, 0, LOC_PLANE), // a floor at ground level
            };
            Command::Place {
                id: 1,
                row,
                cx: cell.0,
                cz: cell.1,
                level,
                loc,
            }
        } else if i == MAX_PLAYERS + 3 {
            Command::Upgrade {
                id: 1,
                cx: cell.0,
                cz: cell.1,
                level: 0,
                loc: LOC_EDGE_W,
                material: match t % 3 {
                    0 => MAT_WOOD,
                    1 => MAT_STONE,
                    _ => MAT_METAL,
                },
            }
        } else if i == MAX_PLAYERS + 4 {
            // The repair verb, alternating stores on the same address the
            // upgrade arm walks. Both bits matter here: the piece branch
            // and the deployable branch take different tables and
            // different setters, and the alloc window has to count both.
            // Whether any given tick's repair lands or refuses is not the
            // point — no path through either may reach the allocator.
            Command::Repair {
                id: 1,
                deploy: t.is_multiple_of(2),
                cx: cell.0,
                cz: cell.1,
                level: 0,
                loc: LOC_EDGE_W,
            }
        } else if i == MAX_PLAYERS + 5 {
            // The raid verb, on the address the repair arm walks. Two
            // allocation surfaces, not one: `place` writes the charge
            // store, and `tick_fuses` runs every tick after it — including
            // the ticks that detonate, which reach `damage_piece` and from
            // there `collapse_from`'s bounded cascade. The fuse scan runs
            // whether or not anything is planted, so this arm keeps the
            // counter over the empty case too.
            Command::Throw {
                id: 1,
                deploy: !t.is_multiple_of(2),
                cx: cell.0,
                cz: cell.1,
                level: 0,
                loc: LOC_EDGE_W,
            }
        } else {
            Command::Consume {
                id: 1,
                slot: match t % 3 {
                    0 => 20,                  // fixture item 0: food
                    1 => 21,                  // fixture item 1: not food
                    _ => INV_SLOTS as u8 * 2, // past the inventory entirely
                },
            }
        }
    })
}

/// A standable point with sea inside `DRINK_REACH_M` — scanned off the
/// heightfield rather than typed in, because a hard-coded coast is a number
/// that goes stale the first time the generator's constants move, and a
/// drinker staged inland would turn this gate's landed-drink assert into a
/// coin flip. Pure float reads of the same function `survival::drink` asks;
/// it allocates nothing, and the caller runs it before the counters open
/// anyway.
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

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * sim_core::build::BUILD_CELL_M,
        (cz as f32 + 0.5) * sim_core::build::BUILD_CELL_M,
    )
}

/// A cell that will hold a ground-class deployable: buildable terrain with
/// no plane piece already on it. Scanned outward in rings from a start
/// cell, for the same reason `shoreline` scans — a coordinate that held at
/// one seed is a fixture that stops meaning what it says the moment the
/// generator moves. The rule asked is `build::foundation_terrain_ok`, which
/// is the terrain half of `deploy::ground_ok` verbatim.
fn buildable_cell(w: &World, seed: u64, cx0: u16, cz0: u16) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue; // ring, not disc
                }
                let cx = (cx0 as i32 + dx).clamp(0, 1023) as u16;
                let cz = (cz0 as i32 + dz).clamp(0, 1023) as u16;
                let (x, z) = cell_center(cx, cz);
                if sim_core::build::foundation_terrain_ok(seed, x, z)
                    && w.pieces.find(cx, cz, 0, LOC_PLANE).is_none()
                {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this gate");
}

struct CountingAlloc;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        FREES.fetch_add(1, Ordering::SeqCst);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// The world seed, named so the raider below can rebuild a body on it.
const SEED: u64 = 0xA110C;

#[test]
fn test_alloc_zero() {
    let mut world = World::new(SEED);
    // The gather fixture puts swings, yields, slot-life writes, and the
    // respawn sweep inside the counted window; the craft fixture adds
    // enqueues, unit completions, refusals, and cancels.
    world.gather = GatherContent::probe_fixture();
    world.craft = CraftContent::probe_fixture();
    world.build = BuildContent::probe_fixture();
    // The combat fixture puts the target scan, the damage write, and the
    // death/respawn path inside the counted window (see the duel below).
    world.combat = CombatContent::probe_fixture();
    // The backpack fixture puts the drop, the nearest-in-reach scan, the
    // take, the emptied-bag removal and the despawn sweep in there too:
    // the duelists stand inside each other, so every death lands a bag at
    // the survivor's feet and the loot commands below open it.
    world.backpack = BackpackContent::probe_fixture();
    // A barrel smash rolls a table and stands a container up, both inside
    // the tick. Neither may allocate: the roll writes into a caller-owned
    // array and the store is fixed-capacity.
    world.loot = sim_core::loot::LootContent::probe_fixture();
    // The survival fixture puts the drain, the announcement, the eat and
    // the clock's own death and respawn-grant inside the counted window.
    // Its spans are seconds, so a hundred bodies drain all the way to
    // empty and take the hp per minute an empty pair costs — the pressure
    // paths run on the whole shard here, not on one arranged body.
    //
    // It is the loudest fixture in this file and deliberately so: a meter
    // step announces `EV_VITALS` + `EV_HEALTH`, and measured this commit
    // the tick's event ring peaks at 210 of `MAX_EVENTS_PER_TICK`'s 256
    // with it installed. That is 46 slots of headroom on a drop-*newest*
    // ring whose late pushes are exactly the raid, bag and eat events the
    // asserts below count — so the drop counter is asserted at the end
    // rather than trusted. Widening the spans to buy headroom would trade
    // this gate's real coverage for a comfortable number; the assert is
    // the honest version of the same worry.
    world.survival = SurvivalContent::probe_fixture();
    // The deploy fixture, for the one path in it this gate has to reach:
    // the respawn's bag scan. A death now walks the deploy store looking
    // for the dying player's own bag before it walks the spawn ring, and a
    // scan is per-tick work like any other — bounded, but only actually
    // *counted* while a bag exists for it to find. Bot 6's is placed below,
    // through the real verb.
    world.deploy = DeployContent::probe_fixture();
    // The animal fixture, and it is the largest per-tick addition in this
    // file: 100 bots spread over the island wake most of a 64-slot roster,
    // so the counted window holds the hatch, the staggered think (the
    // 256-entry heading search included), the wake scan and 64 more
    // capsules through `movement::step` — every tick, not on an event.
    // A melee swing that lands on one also runs `mob::strike` and its
    // inventory write.
    world.mob = sim_core::mob::MobContent::probe_fixture();
    let mut rng = Pcg32::new(0xA110C, 3);
    let mut yaws = [0u16; MAX_PLAYERS];

    // Join the full shard, then warm up.
    let joins: [Command; MAX_PLAYERS] =
        core::array::from_fn(|i| Command::Join { id: i as u32 + 1 });
    world.tick(&joins);
    // Bot 1 is the builder: a stock deep enough that no placement in the
    // counted window ever bounces on cost, so the paid paths are what get
    // counted. A fixture arrangement, like the wire tests' server-side
    // grants — writing an inventory slot allocates nothing.
    world.players[0].inv[20] = ItemStack {
        item: 0,
        count: 60_000,
    };
    world.players[0].inv[21] = ItemStack {
        item: 1,
        count: 60_000,
    };
    // The duel: bots 3 and 4, armed in the hand they hold. Deliberately
    // not bot 1 — the builder must keep its stock and its own cell, and a
    // death would empty both. Their frames are overridden every counted
    // tick (below) to stand still and swing, so the pair stay coincident
    // and trade until one dies: three fixture hits at 34, a swing every
    // 38 ticks, so the kill lands around tick 76 of the 300 — the assert
    // at the end is what holds that claim, not this comment.
    for i in [2usize, 3] {
        world.players[i].inv[0] = ItemStack { item: 0, count: 1 };
    }
    // Bot 1's own build cell, so place and upgrade always have reach.
    let builder_cell = |w: &World| {
        let b = &w.players[0].body;
        let cell = |q: i32| {
            sim_core::build::build_cell_of(q as f32 * sim_core::movement::POS_XZ_Q).clamp(0, 1023)
                as u16
        };
        (cell(b.qx), cell(b.qz))
    };
    for t in 0..30u16 {
        let cmds = tick_cmds(&mut rng, &mut yaws, t, builder_cell(&world));
        world.tick(&cmds);
    }

    // Found before the counters open, because a scan is the test's own
    // work and not the sim's — the window must count the tick, not the
    // fixture. Pure float reads of the same heightfield the verb asks.
    let shore = shoreline(SEED);

    // Bot 6 — the body staged to starve below — gets a bag to wake up on,
    // placed through the real verb on a cell scanned for the same terrain
    // rule `deploy::ground_ok` applies. So the death inside the window is
    // not just a death: it is the bag scan finding an answer, stamping the
    // cooldown onto the store, and putting the body somewhere the spawn
    // ring never would. Placed here rather than after the counters open
    // because a placement is the fixture's work; the *respawn* is the
    // sim's, and that is the half this window counts.
    let bag_cell = buildable_cell(&world, SEED, 341, 341);
    let bag_at = {
        let (x, z) = cell_center(bag_cell.0, bag_cell.1);
        let b = sim_core::movement::Body::at(SEED, x, z);
        (b.qx, b.qz)
    };
    world.players[5].body = {
        let (x, z) = cell_center(bag_cell.0, bag_cell.1);
        sim_core::movement::Body::at(SEED, x, z)
    };
    world.players[5].inv[10] = ItemStack { item: 5, count: 1 };
    world.tick(&[Command::PlaceDeploy {
        id: 6,
        row: 3, // the fixture's ground-class bag
        cx: bag_cell.0,
        cz: bag_cell.1,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(
        world.deploys.len(),
        1,
        "the staged bag did not place — the fixture, not the gate"
    );

    let a0 = ALLOCS.load(Ordering::SeqCst);
    let f0 = FREES.load(Ordering::SeqCst);

    // Both baselines are window-scoped on purpose: the warmup runs the
    // same command cycle, so it stands pieces (and reaches the stone rung)
    // before the counter starts. An assert that only asked whether a row-4
    // record exists would be satisfied by the warmup's and say nothing
    // about the window it names.
    let rung_count = |w: &World| w.pieces.entries().iter().filter(|p| p.row == 4).count();
    // Watched per tick rather than compared end to end. Bot 5 raids inside
    // this same window and a broken piece now takes what it held down with
    // it (build.rs `collapse_from`), so the store's net size at the end is
    // a proxy that a collapse can cancel out — it would read "no placement
    // ran" for a window in which several did. A tick where the store grew
    // is an insert, and a tick where a row-4 record appeared is the
    // upgrade's re-row (nothing else in the cycle can reach row 4).
    let mut pieces_prev = world.pieces.len();
    let mut rung_prev = rung_count(&world);
    let mut placed_in_window = 0u32;
    let mut rung_ups = 0u32;
    // Bot 5 is the raider: stood on a plane piece the warmup actually
    // left standing, holding fixture item 0 (34 structure damage) and
    // swinging every tick. A plane's anchor is its cell center, so the
    // raider is exactly on it — point-blank has no bearing to test, the
    // same reason the duel below stands the pair coincident. 100 hp at 34
    // a swing fells it in three, so the damage write AND the removal path
    // both land inside the counted window. It targets a piece the builder
    // has already walked away from, so bot 1's own asserts stay clear.
    let target = *world
        .pieces
        .entries()
        .iter()
        .find(|p| p.loc == LOC_PLANE)
        .expect("the warmup must leave a plane piece for the raider");
    world.players[4].body = sim_core::movement::Body::at(
        SEED,
        (target.cx as f32 + 0.5) * sim_core::build::BUILD_CELL_M,
        (target.cz as f32 + 0.5) * sim_core::build::BUILD_CELL_M,
    );
    world.players[4].inv[0] = ItemStack { item: 0, count: 1 };
    // Bot 6 is the starving body: both meters emptied as the window opens,
    // deliberately not a bot the duel, the raid or the build script uses.
    // The shard's own meters do not run dry until ~tick 240 and a full body
    // then needs another ~200 ticks to die of it, which is past the 300 this
    // window counts — so the clock's own death is *staged* rather than waited
    // for. At the fixture's rates a dry pair costs 600 + 900 hp a minute, so
    // a 100 hp body has ~120 ticks to live: the hurt path, the `EV_DEATH`
    // whose victim and killer are the same id, and the respawn's
    // `survival::grant` all land inside the window. The asserts at the end
    // are what hold that, not this comment — with the staging removed, the
    // grant assert is the one that fires.
    world.players[5].food = 0;
    world.players[5].water = 0;
    // Bot 7 is the drinker, stood on a shoreline the heightfield is scanned
    // for rather than a coordinate typed in — the drink verb reads
    // `terrain::height` at five taps, so the one thing this gate must not
    // do is stage it somewhere the generator can move out from under.
    // It presses drink every counted tick, and at the fixture's 20 hp a
    // mouthful a 100 hp body drinks itself dead in five: the landed drink,
    // the salt death, the respawn's grant, and — once the ring has put it
    // somewhere inland — the dry refusal all land inside the window.
    world.players[6].body = sim_core::movement::Body::at(SEED, shore.0, shore.1);
    world.players[6].water = 0;
    // Bot 2's meters are the drain witness: it neither duels, raids,
    // builds nor eats, so nothing but the clock moves them.
    let witness_food = world.players[1].food;
    let witness_water = world.players[1].water;
    let witness_deaths = world.players[1].deaths;
    let mut struct_hits = 0u32;
    let mut struct_falls = 0u32;
    let mut starved = 0u32;
    let mut ate = 0u32;
    let mut eat_refused = 0u32;
    let mut drank = 0u32;
    let mut dry_presses = 0u32;
    let drinker_deaths_before = world.players[6].deaths;
    // `EventQueue::dropped` is cleared every tick, so the total is the sum
    // — see the assert at the end for why this gate reads it at all.
    let mut events_dropped = 0u32;
    // Stand bot 4 inside bot 3 as the window opens: point-blank has no
    // bearing to test, so the aim cone cannot make this arrangement flaky.
    world.players[3].body = world.players[2].body;
    let deaths_before = world.players[2].deaths + world.players[3].deaths;
    let mut bags_dropped = 0u32;
    let mut bags_emptied = 0u32;
    // Two counters, not one: `bag_wakes` is the scan finding bot 6's bag,
    // `ring_wakes` is every other body still falling through to the spawn
    // ring. A window that only counted the first would go green on a
    // respawn path that had quietly stopped having a fallback at all.
    let mut bag_wakes = 0u32;
    let mut ring_wakes = 0u32;
    let mut woke_off_the_bag = 0u32;
    // Since wire v16 a death is a body on the death screen, so the window
    // has to *answer* one for a respawn to happen at all. Two counters
    // hold that half: how many slot-ticks were spent dead (the screen's
    // own state exists and is reached), and whether any of them acted.
    let mut screen_ticks = 0u32;
    let mut corpse_acted = 0u32;
    for t in 0..300u16 {
        let mut cmds = tick_cmds(
            &mut rng,
            &mut yaws,
            t.wrapping_add(30),
            builder_cell(&world),
        );
        for i in [2usize, 3, 4] {
            cmds[i] = Command::Input {
                id: i as u32 + 1,
                frame: InputFrame {
                    seq: t,
                    buttons: BTN_PRIMARY,
                    yaw: 0,
                    pitch: 128,
                    move_x: 0,
                    move_z: 0,
                    sel: 0,
                },
            };
        }
        // Bot 6 stands still. It is the body staged to starve on top of its
        // own bag, and `woke_off_the_bag` below reads its position at the
        // end of the tick its respawn landed on — since v16 the wake is a
        // command, applied before the player loop, so a walking frame would
        // step the body off the cell inside the same tick and the check
        // would be measuring the bot script rather than the scan.
        cmds[5] = Command::Input {
            id: 6,
            frame: InputFrame {
                seq: t,
                ..InputFrame::default()
            },
        };
        // Both duelists reach for a bag every tick. The command array
        // grows on the stack rather than stealing a bot's input slot —
        // `MAX_COMMANDS_PER_TICK` is 256 and this is 113, so they all
        // still apply, and copying `Command`s allocates nothing.
        let mut all = [Command::Loot { id: 3 }; MAX_PLAYERS + 14];
        all[..MAX_PLAYERS + 7].copy_from_slice(&cmds);
        all[MAX_PLAYERS + 7] = Command::Loot { id: 3 };
        all[MAX_PLAYERS + 8] = Command::Loot { id: 4 };
        all[MAX_PLAYERS + 9] = Command::Drink { id: 7 };
        // …and every body that can die in this window answers its own
        // death screen every tick. Unconditional because a respawn from a
        // standing body is a no-op by design (world.rs), so this is the
        // whole of the verb — including the press that does nothing —
        // inside the counted window. The choice is the *point*: bot 6 is
        // the one with a bag and asks for it, and the three that have none
        // ask for a beach, so `bag_wakes` and `ring_wakes` below count two
        // different decisions rather than one path's two outcomes.
        all[MAX_PLAYERS + 10] = Command::Respawn {
            id: 6,
            on_bag: true,
        };
        all[MAX_PLAYERS + 11] = Command::Respawn {
            id: 3,
            on_bag: false,
        };
        all[MAX_PLAYERS + 12] = Command::Respawn {
            id: 4,
            on_bag: false,
        };
        all[MAX_PLAYERS + 13] = Command::Respawn {
            id: 7,
            on_bag: false,
        };
        world.tick(&all);
        let pieces_now = world.pieces.len();
        if pieces_now > pieces_prev {
            placed_in_window += 1;
        }
        pieces_prev = pieces_now;
        let rung_now = rung_count(&world);
        if rung_now > rung_prev {
            rung_ups += 1;
        }
        rung_prev = rung_now;
        events_dropped += world.events.dropped;
        for ev in world.events.entries() {
            if ev.code == EV_BAG_DROPPED {
                bags_dropped += 1;
            } else if ev.code == EV_BAG_REMOVED && ev.b == BAG_GONE_EMPTIED {
                bags_emptied += 1;
            } else if ev.code == EV_STRUCT_HIT {
                struct_hits += 1;
            } else if ev.code == EV_PIECE_REMOVED {
                struct_falls += 1;
            } else if ev.code == EV_DEATH && ev.a == ev.b {
                // Victim and killer equal is the clock's own signature; a
                // combat kill always names two different bodies.
                starved += 1;
            } else if ev.code == EV_CONSUMED {
                ate += 1;
            } else if ev.code == EV_CONSUME_REFUSED {
                // Two verbs announce their refusals on this one code, so
                // the counters partition by the body that pressed rather
                // than sharing the branch: bot 1 is the only id the eat
                // script addresses and bot 7 the only one that drinks.
                // Counting the union here would let one verb's refusals
                // satisfy the other's floor — `eat_refused > 0` below
                // would be implied by the drinker alone, and the assert
                // would go on reading as if it still guarded eating while
                // the whole eat refusal path could be deleted under it.
                if ev.a == 1 {
                    eat_refused += 1;
                } else if ev.a == 7 && ev.b == REFUSE_C_NO_WATER {
                    dry_presses += 1;
                }
            } else if ev.code == EV_DRANK {
                drank += 1;
            } else if ev.code == EV_RESPAWN {
                if ev.b == 1 {
                    bag_wakes += 1;
                    // Read on the tick the respawn landed, before the next
                    // step of movement can smear it: the bag's cell, or the
                    // scan answered with a position it did not choose.
                    let b = &world.players[ev.a as usize - 1].body;
                    if (b.qx, b.qz) != bag_at {
                        woke_off_the_bag += 1;
                    }
                } else {
                    ring_wakes += 1;
                }
            }
        }
        // The screen's own state, read after the tick that answers it: a
        // body still dead here is one whose answer has not landed yet, and
        // a dead body with anything in its hands is one that acted.
        for p in world.players.iter() {
            if p.active && p.dead {
                screen_ticks += 1;
                if p.hp > 0 || p.inv.iter().any(|s| s.count > 0) || p.craft_done_at > 0 {
                    corpse_acted += 1;
                }
            }
        }
    }
    // The hash path must be allocation-free too.
    let h = world.state_hash();
    assert_ne!(h, 0);

    let alloc_delta = ALLOCS.load(Ordering::SeqCst) - a0;
    let free_delta = FREES.load(Ordering::SeqCst) - f0;

    // Read after the counters are captured, so the checks themselves can
    // never be what a future reader blames a nonzero delta on. These are
    // what keep the build arms above honest: a gate that only ever drove
    // refusals would count nothing the write paths do.
    assert!(
        placed_in_window > 0,
        "no piece was placed inside the counted window — the build write path \
         fell out of the alloc gate"
    );
    assert!(
        rung_ups > 0,
        "nothing reached the fixture's stone rung inside the counted window — \
         the upgrade write path fell out of the alloc gate"
    );
    assert!(
        world.players[2].deaths + world.players[3].deaths > deaths_before,
        "nobody died inside the counted window — the damage, death and \
         respawn paths fell out of the alloc gate"
    );
    assert!(
        bags_dropped > 0,
        "no backpack was dropped inside the counted window — the death-drop \
         path fell out of the alloc gate"
    );
    assert!(
        bags_emptied > 0,
        "no backpack was looted empty inside the counted window — the take \
         path fell out of the alloc gate"
    );
    assert!(
        struct_hits > 0,
        "no structure took a raid hit inside the counted window — the piece \
         damage write fell out of the alloc gate"
    );
    assert!(
        struct_falls > 0,
        "no structure was broken inside the counted window — the raid removal \
         path fell out of the alloc gate"
    );
    // The survival clock's paths, each read off the thing it moves rather
    // than off the fact that content was installed. A fixture
    // assigned but never exercised is the same defect one level down —
    // which is exactly what this block exists to make impossible.
    assert_eq!(
        world.players[1].deaths, witness_deaths,
        "the drain witness died inside the counted window — pick a quieter \
         slot; its meters no longer measure the drain alone"
    );
    assert!(
        world.players[1].food < witness_food && world.players[1].water < witness_water,
        "the drain witness's meters did not fall inside the counted window \
         (food {} of {witness_food}, water {} of {witness_water}) — \
         survival::step fell out of the alloc gate",
        world.players[1].food,
        world.players[1].water
    );
    assert_eq!(
        (world.players[1].food, world.players[1].water),
        (0, 0),
        "the drain witness's pair never ran dry inside the counted window — \
         the empty-meter hurt path is only in this gate while it does"
    );
    assert!(
        starved > 0,
        "nobody starved inside the counted window — the hurt path and the \
         clock's own death fell out of the alloc gate"
    );
    assert!(
        world.players[5].food > 0,
        "the starved body's meters were never granted again — the respawn \
         grant fell out of the alloc gate"
    );
    assert!(
        screen_ticks > 0,
        "no body was ever on the death screen inside the counted window — \
         `World::die` fell out of the alloc gate, or death is still an \
         immediate respawn and the choice is not reachable"
    );
    assert_eq!(
        corpse_acted, 0,
        "a body on the death screen was carrying hp, items or a craft job — \
         `live_slot_of` is not holding, and a corpse is playing the game"
    );
    // The drink verb's three outcomes, each read off the thing it moves.
    // A verb whose only coverage was "it was in the command array" would be
    // exactly the defect the eat verb's own block above exists against.
    assert!(
        drank > 0,
        "nobody drank inside the counted window — the drinker was staged off \
         the water, or `survival::drink`'s landed path fell out of the alloc gate"
    );
    assert!(
        world.players[6].deaths > drinker_deaths_before,
        "the drinker never died of the salt inside the counted window — the \
         drink's own kill site and the respawn behind it are only in this gate \
         while it does"
    );
    assert!(
        dry_presses > 0,
        "no dry press was refused inside the counted window — the spawn ring \
         put the drinker back on water every time, or the refusal path fell \
         out of the alloc gate"
    );
    assert!(
        world.players[5].deaths > 0,
        "the starved body's death was never counted — a clock death that does \
         not move `deaths` respawns the body on the identical beach, and the \
         increment fell out of the alloc gate"
    );
    // Respawn-on-bag, both halves. The bag scan runs on every death in the
    // window; these say it ran to both of its verdicts, and that neither
    // allocated.
    assert!(
        bag_wakes > 0,
        "nobody woke on a bag inside the counted window — the respawn's bag \
         scan, the cooldown stamp and the store write fell out of the alloc gate"
    );
    assert_eq!(
        woke_off_the_bag, 0,
        "{woke_off_the_bag} respawns announced a bag and then put the body \
         somewhere else"
    );
    assert!(
        ring_wakes > 0,
        "every death in the counted window found a bag — the spawn-ring \
         fallback is no longer in this gate"
    );
    assert!(
        world.deploys.bag_ready()[0] > 0,
        "the bag was woken on but never stamped — a cooldown that is not \
         written is a bag that answers every death in a raid"
    );
    assert!(
        ate > 0 && eat_refused > 0,
        "the eat verb landed {ate} consumes and {eat_refused} refusals inside \
         the counted window — both paths must be in the alloc gate"
    );
    assert!(
        world.players[0].inv[20].count < 60_000,
        "no stack shrank inside the counted window — the consume write path \
         fell out of the alloc gate"
    );
    // The bag, raid, starve and eat counters above are all read off the
    // event ring, so a saturated ring would let them go quiet for a reason
    // that has nothing to do with the path they name — the drop policy is
    // newest-first, and those pushes sit late in a tick's push order. This
    // is the assert that keeps them honest. (The meter asserts are read off
    // player state and do not need it.)
    assert_eq!(
        events_dropped, 0,
        "the event ring overflowed {events_dropped} times inside the counted \
         window: the counters above are no longer reading what they claim to"
    );
    assert_eq!(
        (alloc_delta, free_delta),
        (0, 0),
        "heap traffic in the tick: {alloc_delta} allocs, {free_delta} frees over 300 ticks x {MAX_PLAYERS} bots"
    );
}
