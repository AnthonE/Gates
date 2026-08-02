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
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::MAX_PLAYERS;
use sim_core::rng::Pcg32;
use sim_core::world::{Command, World, EV_BAG_DROPPED, EV_BAG_REMOVED};

/// One tick's commands: every bot's input plus a craft enqueue, a cancel,
/// a place, and an upgrade, so the craft and build verbs sit inside the
/// counted window. Fixed-size — the test itself must not allocate.
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
fn tick_cmds(
    rng: &mut Pcg32,
    yaws: &mut [u16; MAX_PLAYERS],
    t: u16,
    cell: (u16, u16),
) -> [Command; MAX_PLAYERS + 4] {
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
        } else {
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
        }
    })
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

#[test]
fn test_alloc_zero() {
    let mut world = World::new(0xA110C);
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

    let a0 = ALLOCS.load(Ordering::SeqCst);
    let f0 = FREES.load(Ordering::SeqCst);

    // Both baselines are window-scoped on purpose: the warmup runs the
    // same command cycle, so it stands pieces (and reaches the stone rung)
    // before the counter starts. An assert that only asked whether a row-4
    // record exists would be satisfied by the warmup's and say nothing
    // about the window it names.
    let rung_count = |w: &World| w.pieces.entries().iter().filter(|p| p.row == 4).count();
    let placed_before = world.pieces.len();
    let rung_before = rung_count(&world);
    // Stand bot 4 inside bot 3 as the window opens: point-blank has no
    // bearing to test, so the aim cone cannot make this arrangement flaky.
    world.players[3].body = world.players[2].body;
    let deaths_before = world.players[2].deaths + world.players[3].deaths;
    let mut bags_dropped = 0u32;
    let mut bags_emptied = 0u32;
    for t in 0..300u16 {
        let mut cmds = tick_cmds(
            &mut rng,
            &mut yaws,
            t.wrapping_add(30),
            builder_cell(&world),
        );
        for i in [2usize, 3] {
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
        // Both duelists reach for a bag every tick. The command array
        // grows by two on the stack rather than stealing a bot's input
        // slot — `MAX_COMMANDS_PER_TICK` is 256, so 102 still all apply,
        // and copying `Command`s allocates nothing.
        let mut all = [Command::Loot { id: 3 }; MAX_PLAYERS + 6];
        all[..MAX_PLAYERS + 4].copy_from_slice(&cmds);
        all[MAX_PLAYERS + 4] = Command::Loot { id: 3 };
        all[MAX_PLAYERS + 5] = Command::Loot { id: 4 };
        world.tick(&all);
        for ev in world.events.entries() {
            if ev.code == EV_BAG_DROPPED {
                bags_dropped += 1;
            } else if ev.code == EV_BAG_REMOVED && ev.b == BAG_GONE_EMPTIED {
                bags_emptied += 1;
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
        world.pieces.len() > placed_before,
        "no piece was placed inside the counted window — the build write path \
         fell out of the alloc gate"
    );
    assert!(
        rung_count(&world) > rung_before,
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
    assert_eq!(
        (alloc_delta, free_delta),
        (0, 0),
        "heap traffic in the tick: {alloc_delta} allocs, {free_delta} frees over 300 ticks x {MAX_PLAYERS} bots"
    );
}
