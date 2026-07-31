//! `test_alloc_zero` (DESIGN.md §12): 100 bots × 300 ticks after warmup,
//! heap alloc/free count delta == 0, measured by a counting GlobalAlloc.
//! CLAUDE.md wall 2. This binary is the gate; nothing else runs in it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use sim_core::bots::bot_frame;
use sim_core::build::BuildContent;
use sim_core::craft::CraftContent;
use sim_core::gather::GatherContent;
use sim_core::limits::MAX_PLAYERS;
use sim_core::rng::Pcg32;
use sim_core::world::{Command, World};

/// One tick's commands: every bot's input plus a craft enqueue, a cancel,
/// and a place, so the craft and build verbs (enqueue, step, placement,
/// every refusal) sit inside the counted window. Fixed-size — the test
/// itself must not allocate.
fn tick_cmds(rng: &mut Pcg32, yaws: &mut [u16; MAX_PLAYERS], t: u16) -> [Command; MAX_PLAYERS + 3] {
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
        } else {
            // The island-center cell: bots near it place, the rest refuse
            // on reach — both build paths inside the counted window.
            Command::Place {
                id: ((t as u32 * 11) % MAX_PLAYERS as u32) + 1,
                row: t % 4,
                cx: 341,
                cz: 341,
                level: (t % 2) as u8,
                loc: ((t / 2) % 4) as u8,
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
    let mut rng = Pcg32::new(0xA110C, 3);
    let mut yaws = [0u16; MAX_PLAYERS];

    // Join the full shard, then warm up.
    let joins: [Command; MAX_PLAYERS] =
        core::array::from_fn(|i| Command::Join { id: i as u32 + 1 });
    world.tick(&joins);
    for t in 0..30u16 {
        let cmds = tick_cmds(&mut rng, &mut yaws, t);
        world.tick(&cmds);
    }

    let a0 = ALLOCS.load(Ordering::SeqCst);
    let f0 = FREES.load(Ordering::SeqCst);

    for t in 0..300u16 {
        let cmds = tick_cmds(&mut rng, &mut yaws, t.wrapping_add(30));
        world.tick(&cmds);
    }
    // The hash path must be allocation-free too.
    let h = world.state_hash();
    assert_ne!(h, 0);

    let alloc_delta = ALLOCS.load(Ordering::SeqCst) - a0;
    let free_delta = FREES.load(Ordering::SeqCst) - f0;
    assert_eq!(
        (alloc_delta, free_delta),
        (0, 0),
        "heap traffic in the tick: {alloc_delta} allocs, {free_delta} frees over 300 ticks x {MAX_PLAYERS} bots"
    );
}
