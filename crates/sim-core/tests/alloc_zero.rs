//! `test_alloc_zero` (DESIGN.md §12): 100 bots × 300 ticks after warmup,
//! heap alloc/free count delta == 0, measured by a counting GlobalAlloc.
//! CLAUDE.md wall 2. This binary is the gate; nothing else runs in it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use sim_core::bots::bot_frame;
use sim_core::limits::MAX_PLAYERS;
use sim_core::rng::Pcg32;
use sim_core::world::{Command, World};

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
    let mut rng = Pcg32::new(0xA110C, 3);
    let mut yaws = [0u16; MAX_PLAYERS];

    // Join the full shard, then warm up.
    let joins: [Command; MAX_PLAYERS] =
        core::array::from_fn(|i| Command::Join { id: i as u32 + 1 });
    world.tick(&joins);
    for t in 0..30u16 {
        let cmds: [Command; MAX_PLAYERS] = core::array::from_fn(|i| {
            let f = bot_frame(&mut rng, yaws[i], t);
            yaws[i] = f.yaw;
            Command::Input {
                id: i as u32 + 1,
                frame: f,
            }
        });
        world.tick(&cmds);
    }

    let a0 = ALLOCS.load(Ordering::SeqCst);
    let f0 = FREES.load(Ordering::SeqCst);

    for t in 0..300u16 {
        let cmds: [Command; MAX_PLAYERS] = core::array::from_fn(|i| {
            let f = bot_frame(&mut rng, yaws[i], t.wrapping_add(30));
            yaws[i] = f.yaw;
            Command::Input {
                id: i as u32 + 1,
                frame: f,
            }
        });
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
