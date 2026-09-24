//! The renderer allocates nothing after the bank is installed — `CLAUDE.md`
//! wall 2 applied to the audio thread, measured by a counting `GlobalAlloc`
//! (`sim-core/tests/alloc_zero.rs`'s, verbatim).
//!
//! **This binary is the gate; nothing else runs in it.** The allocator is
//! process-wide, and the test harness spawns and reaps a thread for every
//! other test in a binary while the measured window is open, so a second
//! test here would put its own heap traffic in the count. The rest of the
//! engine's gates are `tests/engine.rs`.
//!
//! What is inside the window: 2,000 blocks of a scripted mix of every `Cmd`
//! variant, sent through the native seam (`Out::send` → `In::drain_into`),
//! rendered, reported back (`In::report` → `Out::stats`), the return ring
//! reclaimed (`Out::reclaim`, empty — nothing is installed twice), with the
//! live ledger ticking beside it. What is outside: `Renderer::new`, the bank
//! (`synth::pcm_bank` is ~12 MB of `Vec` pushes), `channel()` (four rings,
//! allocated once), and one warm-up block.
//!
//! The script's rates are the mixer's speed band through `engine::rate` at
//! the device rate, which is what `start_cmd` sends: the renderer refuses a
//! rate outside that band as a bad command, and this file asserts the script
//! sends none.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use sound::engine::{channel, rate, Cmd, Live, Renderer, BLOCK, HELD};
use sound::mixer::{SPEED_MAX, SPEED_MIN};
use sound::{synth, Cue, CUE_COUNT};

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

/// One block's commands, from a fixed xorshift stream, into a fixed array —
/// the script itself must not allocate. Every variant appears.
fn script(x: &mut u32, blk: usize, out: &mut [Cmd; 6]) -> usize {
    let mut roll = || {
        *x ^= *x << 13;
        *x ^= *x >> 17;
        *x ^= *x << 5;
        *x
    };
    let n = (roll() % 6) as usize;
    for slot_out in out.iter_mut().take(n) {
        let cue = Cue::ALL[(roll() % CUE_COUNT as u32) as usize];
        let slot = (roll() % HELD as u32) as u8;
        let g = (roll() % 1000) as f32 / 1000.0;
        *slot_out = match roll() % 8 {
            0..=3 => Cmd::Start {
                cue,
                take: 0,
                takes: 1,
                lp: 0,
                gain_l: g,
                gain_r: 1.0 - g,
                rate: rate(SPEED_MIN + (roll() % 375) as f32 / 100.0, 48_000),
            },
            4 => Cmd::Loop { slot, cue, gain: g },
            5 => Cmd::Play { slot, cue, gain: g },
            6 => Cmd::Gain { slot, gain: g },
            _ => {
                if blk.is_multiple_of(97) {
                    Cmd::CutVoices
                } else {
                    Cmd::Stop { slot }
                }
            }
        };
    }
    n
}

/// The script's top speed stays inside the mixer's band, so every `Start` it
/// sends is well-formed by construction and the `bad_cmd` assertion below is
/// a claim about the renderer rather than about the roll.
const _: () = assert!(SPEED_MIN + 374.0 / 100.0 <= SPEED_MAX);

#[test]
fn test_engine_alloc_zero() {
    let (mut out, mut inp) = channel();
    let mut r = Renderer::new(48_000);
    for (i, pcm) in synth::pcm_bank().into_iter().enumerate() {
        assert!(
            out.install(Cue::ALL[i], pcm),
            "the install ring holds a whole bank"
        );
    }
    let mut ledger = Live::new();
    let mut buf = [0.0f32; 2 * BLOCK];
    let mut cmds = [Cmd::CutVoices; 6];
    let mut x = 0x2545_F491u32;

    // Warm-up: the installs land, one block renders, one report posts.
    inp.drain_into(&mut r);
    assert_eq!(r.installed(), CUE_COUNT);
    r.render(&mut buf);
    inp.report(&r);
    let _ = out.stats();

    let a0 = ALLOCS.load(Ordering::SeqCst);
    let f0 = FREES.load(Ordering::SeqCst);
    let mut sent = 0u32;
    let mut sum = 0.0f32;
    for blk in 0..2_000 {
        let n = script(&mut x, blk, &mut cmds);
        for cmd in cmds.iter().take(n) {
            if out.send(*cmd) {
                sent += 1;
            }
            match *cmd {
                // The ledger's arithmetic is the same whatever the length;
                // the real length is the adapter's fact.
                Cmd::Start { rate, .. } => {
                    let _ = ledger.start(0.4, rate);
                }
                Cmd::CutVoices => ledger.cut(),
                _ => {}
            }
        }
        inp.drain_into(&mut r);
        r.render(&mut buf);
        inp.report(&r);
        let _ = out.stats();
        let _ = out.reclaim();
        ledger.tick(BLOCK as f32 / 48_000.0);
        sum += buf[0].abs() + buf[2 * BLOCK - 1].abs();
    }
    let alloc_delta = ALLOCS.load(Ordering::SeqCst) - a0;
    let free_delta = FREES.load(Ordering::SeqCst) - f0;
    assert_eq!(
        alloc_delta, 0,
        "the renderer allocated {alloc_delta} time(s) across 2,000 blocks after the bank was \
         installed — the audio thread has a heap touch on its play path"
    );
    assert_eq!(
        free_delta, 0,
        "the renderer freed {free_delta} time(s) across 2,000 blocks — something on the play \
         path owns heap"
    );
    assert!(
        sent > 2_000,
        "the script sent too few commands ({sent}) to have exercised the seam"
    );
    assert!(
        sum > 0.0,
        "two thousand blocks of a busy script rendered silence"
    );
    let s = r.stats();
    assert!(
        s.refused + s.cut + s.unbanked > 0 || s.live > 0,
        "the script never filled anything ({s:?})"
    );
    assert_eq!(
        s.dropped, 0,
        "the native path applies straight from its ring and never queues"
    );
    assert_eq!(s.bad_cmd, 0, "the script sends only well-formed commands");
}
