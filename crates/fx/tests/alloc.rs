//! After a pool is made, emitting, stepping and writing it never touches the
//! heap: an effect runs every frame something is alive.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use fx::pool::{Cam, Orient, Pool, Verts};
use fx::recipe::{emit, Recipe};
use fx::v3;

// Per-thread, so parallel tests in this binary cannot count each other.
thread_local! { static ALLOCS: Cell<Option<u64>> = const { Cell::new(None) }; }

struct CountAlloc;

fn note() {
    let _ = ALLOCS.try_with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
}

unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, new) }
    }
}

#[global_allocator]
static GLOBAL: CountAlloc = CountAlloc;

#[test]
fn emit_step_and_fill_never_allocate() {
    let streak = Recipe {
        orient: Orient::Stretch,
        speed: (3.0, 9.0),
        gravity: 9.81,
        life: (0.2, 0.4),
        stretch: 0.016,
        bounce: true,
        ..Recipe::BASE
    };
    let cap = 64;
    let mut pool = Pool::new(cap, true);
    let (mut pos, mut uv, mut col) = (
        vec![[0.0f32; 3]; cap * 4],
        vec![[0.0f32; 2]; cap * 4],
        vec![[0.0f32; 4]; cap * 4],
    );
    let cam = Cam {
        pos: [0.0, 1.0, 4.0],
        right: v3::X,
        up: v3::Y,
    };
    ALLOCS.with(|c| c.set(Some(0)));
    for frame in 0..240 {
        if frame % 10 == 0 {
            emit(&mut pool, &streak, 40, v3::ZERO, v3::Z, [1.0; 3], 1.0);
            emit(&mut pool, &Recipe::BASE, 8, v3::ZERO, v3::Y, [0.5; 3], 1.5);
        }
        pool.step(1.0 / 60.0);
        let out = Verts {
            pos: &mut pos,
            uv: &mut uv,
            col: &mut col,
        };
        pool.fill(out, &cam, cam.pos);
    }
    let n = ALLOCS.with(|c| c.replace(None)).unwrap();
    assert_eq!(n, 0, "the pool touched the heap {n} times");
    assert!(
        pool.stolen > 0,
        "the pool overflowed, as it must under load"
    );
}
