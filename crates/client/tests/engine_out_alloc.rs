//! The device callback body allocates nothing — `CLAUDE.md` wall 2 applied
//! to `render/audio_out.rs::Feed::fill`, measured by a counting `GlobalAlloc`
//! (`sim-core/tests/alloc_zero.rs`'s, verbatim).
//!
//! **This binary is the gate; nothing else runs in it.** The allocator is
//! process-wide, and the test harness spawns and reaps a thread for every
//! other test in a binary while the measured window is open, so a second
//! test here would put its own heap traffic in the count. The rest of the
//! seam's gates are `tests/engine_out.rs`; the renderer's own are
//! `crates/sound/tests/engine_alloc.rs`, which measures the same law one
//! layer down and is deliberately not this file — a `fill` that allocated
//! around a renderer that did not would pass that one and fail this.
//!
//! What is inside the window: 200 callbacks alternating `f32` and `i16`
//! output at 441, 512, 128 and 1,000 frames, each preceded by 0–5 scripted
//! commands of every variant sent through the native seam, each followed by
//! the flush's two reads (`Native::stats`, `Native::reclaim`). What is
//! outside: `pair(48_000)` (the rings and the renderer), the whole bank
//! (`synth::pcm_bank` is ~12 MB of `Vec` pushes, installed through the
//! ring), the output buffers, and one warm-up callback of 441 frames.
//!
//! **Native only** — `audio_out` is `cfg(not(wasm32))`.
#![cfg(all(feature = "render", not(target_arch = "wasm32")))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use client::render::audio_out::pair;
use client::sound::engine::{rate, Cmd, HELD};
use client::sound::mixer::{SPEED_MAX, SPEED_MIN};
use client::sound::{synth, Cue, CUE_COUNT};

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

/// The device rate the pair is built at — not the bank's, so every held
/// tap is fractional and `engine::rate` is doing its one resample.
const DEVICE_HZ: u32 = 48_000;

/// One callback's commands, from a fixed xorshift stream, into a fixed
/// array — the script itself must not allocate. Every variant appears, and
/// every rate is the mixer's band through `engine::rate`, so `bad_cmd` at
/// the end is a claim about the renderer rather than about the roll.
fn script(x: &mut u32, k: usize, out: &mut [Cmd; 6]) -> usize {
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
                rate: rate(SPEED_MIN + (roll() % 375) as f32 / 100.0, DEVICE_HZ),
            },
            4 => Cmd::Loop { slot, cue, gain: g },
            5 => Cmd::Play { slot, cue, gain: g },
            6 => Cmd::Gain { slot, gain: g },
            _ => {
                if k.is_multiple_of(97) {
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
fn test_engine_out_alloc_zero() {
    let (mut native, mut feed) = pair(DEVICE_HZ);
    for (i, pcm) in synth::pcm_bank().into_iter().enumerate() {
        assert!(
            native.install(Cue::ALL[i], pcm),
            "the install ring holds a whole bank"
        );
    }
    const LONGEST: usize = 1000;
    let mut floats = vec![0.0f32; 2 * LONGEST];
    let mut shorts = vec![0i16; 2 * LONGEST];
    let mut cmds = [Cmd::CutVoices; 6];
    let mut x = 0x2545_F491u32;
    const LENGTHS: [usize; 4] = [441, 512, 128, LONGEST];

    // Warm-up: the installs land, one callback renders, one report posts
    // and is read, the (empty) return ring is reclaimed.
    feed.fill::<f32>(&mut floats[..2 * 441], 2);
    let warm = native.stats().expect("the warm-up posts a report");
    assert_eq!(warm.installed, CUE_COUNT as u32, "the bank did not land");
    assert_eq!(native.reclaim(), 0);
    assert!(native.alive());

    let a0 = ALLOCS.load(Ordering::SeqCst);
    let f0 = FREES.load(Ordering::SeqCst);
    let mut sent = 0u32;
    let mut sum = 0.0f32;
    for k in 0..200 {
        let n = script(&mut x, k, &mut cmds);
        for cmd in cmds.iter().take(n) {
            if native.send(*cmd) {
                sent += 1;
            }
        }
        let frames = LENGTHS[k % LENGTHS.len()];
        if k % 2 == 0 {
            let out = &mut floats[..2 * frames];
            feed.fill::<f32>(out, 2);
            sum += out[0].abs() + out[2 * frames - 1].abs();
        } else {
            let out = &mut shorts[..2 * frames];
            feed.fill::<i16>(out, 2);
            sum += (out[0] as f32).abs() + (out[2 * frames - 1] as f32).abs();
        }
        // The flush's two reads, every frame.
        let _ = native.stats();
        let _ = native.reclaim();
    }
    let alloc_delta = ALLOCS.load(Ordering::SeqCst) - a0;
    let free_delta = FREES.load(Ordering::SeqCst) - f0;
    assert_eq!(
        alloc_delta, 0,
        "the callback body allocated {alloc_delta} time(s) across 200 fills after the bank was \
         installed — the audio thread has a heap touch on its play path"
    );
    assert_eq!(
        free_delta, 0,
        "the callback body freed {free_delta} time(s) across 200 fills — something on the play \
         path owns heap"
    );
    assert!(
        sent > 200,
        "the script sent too few commands ({sent}) to have exercised the seam"
    );
    assert!(
        sum > 0.0,
        "two hundred callbacks of a busy script rendered silence"
    );
    assert_eq!(native.ring_dropped(), 0, "a drained ring dropped a command");
    // The last report — one more read outside the window, so the counters
    // below are the renderer's after every command landed.
    feed.fill::<f32>(&mut floats[..2 * 128], 2);
    let s = native.stats().expect("a report after the last fill");
    assert!(
        s.refused + s.cut + s.unbanked > 0 || s.live > 0,
        "the script never filled anything ({s:?})"
    );
    assert_eq!(s.bad_cmd, 0, "the script sends only well-formed commands");
    assert_eq!(
        s.dropped, 0,
        "the native path applies straight from its ring and never queues"
    );
    assert_eq!(s.reinstall_refused, 0, "the bank was installed once");
}
