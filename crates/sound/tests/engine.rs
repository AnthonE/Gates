//! The sample renderer's gate — code tier: no clock, no device, no Bevy.
//!
//! `engine.rs` is the one place a decision about sound becomes samples, on
//! both targets, so what a player hears is exactly what this file can prove
//! about it. The load-bearing test is the naive rebuild
//! (`every_sample_is_rebuilt_from_published_parts`): every output sample is
//! recomputed here from `synth::pcm`, `engine::rate` and a test-local lerp,
//! ramp and clamp — never calling `render` or `apply` — and compared on
//! `to_bits`. `CLAUDE.md`'s lattice entry is why the rebuild shares no code
//! with the thing it checks, and why the mutants below were run rather than
//! reasoned about.
//!
//! **The allocation gate is not in this binary.** It is
//! `tests/engine_alloc.rs`, alone, for `sim-core/tests/alloc_zero.rs`'s
//! reason: a counting `GlobalAlloc` is process-wide, and the test harness
//! spawns and reaps threads for every other test in the binary while the
//! measured window is open.
//!
//! ## Mutants run by hand against the rebuild
//!
//! Each was applied to `engine.rs`, the suite run single-threaded, and the
//! file restored byte for byte. The first reddening assertion is recorded,
//! and two of them were not where they were predicted to be: every cue's
//! sample 0 is faded to zero by `synth::edges`, so a wrong ear or a wrong
//! neighbour first shows on **sample 1**, and the off-by-one is caught at
//! the bank's own rate by the 1.37× voice rather than by the 48 kHz pass —
//! a prediction about which case catches a mutant is itself a claim to run.
//!
//! | mutant | reddened by |
//! |---|---|
//! | `gain_l` / `gain_r` swapped in `mix_voice` | `every_sample_is_rebuilt_from_published_parts`, 44.1 kHz, block 0 sample 1, left ear: −0.10530963 against the rebuild's −0.10519281 |
//! | `pos + 1` → `pos` for `tap`'s neighbour in `mix_voice` | `every_sample_is_rebuilt_from_published_parts`, 44.1 kHz, block 0 sample 1, left ear: −0.10492859 against −0.10519281 (the 1.37× voice's first fractional tap) |
//! | a held loop that clamps `pos` to `len − 1` instead of wrapping | `a_loop_wraps_without_a_seam`, output sample 463,050 (the join) is not `bank[0]`; and `every_sample_is_rebuilt_from_published_parts` in the block after the wrap |
//! | the `Gain` ramp applied at block start (`t = 1.0` for every sample) | `every_sample_is_rebuilt_from_published_parts`, 44.1 kHz, block 3 sample 0, left ear: 0.48732442 against 0.4152903 (the first ramped sample) |

use std::sync::OnceLock;

use sim_core::yaw_dir;
use sound::engine::{
    self, channel, pan, rate, start_cmd, Cmd, Live, Renderer, BLOCK, CMD_BYTES, CMD_RING_CAP,
    CMD_RING_CAP_NATIVE, HELD, VOICE_SLOTS,
};
use sound::mixer::{Start, SPEED_MAX, SPEED_MIN};
use sound::{synth, Cue, CUE_COUNT, SAMPLE_RATE};

/// The bank, generated once per binary: it is ~0.8 s of synthesis and every
/// test here wants it.
fn bank() -> &'static [Box<[i16]>; CUE_COUNT] {
    static BANK: OnceLock<[Box<[i16]>; CUE_COUNT]> = OnceLock::new();
    BANK.get_or_init(synth::pcm_bank)
}

fn pcm(cue: Cue) -> &'static [i16] {
    &bank()[cue.idx()]
}

/// A renderer with the whole bank installed.
fn loaded(out_rate: u32) -> Renderer {
    let mut r = Renderer::new(out_rate);
    for cue in Cue::ALL {
        assert!(
            r.install(cue, pcm(cue).into()).is_ok(),
            "{cue:?} was refused on a fresh renderer"
        );
    }
    r
}

fn block() -> [f32; 2 * BLOCK] {
    [0.0; 2 * BLOCK]
}

fn start(cue: Cue, gain_l: f32, gain_r: f32, rate: f32) -> Cmd {
    Cmd::Start {
        cue,
        take: 0,
        takes: 1,
        lp: 0,
        gain_l,
        gain_r,
        rate,
    }
}

fn len_s(cue: Cue) -> f32 {
    pcm(cue).len() as f32 / SAMPLE_RATE as f32
}

// ---------------------------------------------------------------------------
// The naive rebuild: the engine's laws, restated here from published parts.
// ---------------------------------------------------------------------------

/// A cursor as the test understands it.
#[derive(Clone, Copy)]
struct Cursor {
    pos: u32,
    frac: f32,
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// `bank[pos]` toward its neighbour, scaled from i16 full scale. `wrap`
/// says whether the neighbour of the last sample is sample 0.
fn sample(b: &[i16], c: Cursor, wrap: bool) -> f32 {
    let p = c.pos as usize;
    let n = if wrap && p + 1 >= b.len() { 0 } else { p + 1 };
    lerp(b[p] as f32, b[n] as f32, c.frac) * (1.0 / 32768.0)
}

fn step(c: &mut Cursor, rate: f32) {
    c.frac += rate;
    let whole = c.frac as u32;
    c.pos += whole;
    c.frac -= whole as f32;
}

fn ramp(g0: f32, g1: f32, i: usize, n: usize) -> f32 {
    g0 + (g1 - g0) * ((i + 1) as f32 / n as f32)
}

/// A one-shot in the rebuild.
struct NVoice {
    cue: Cue,
    c: Cursor,
    rate: f32,
    gl: f32,
    gr: f32,
    live: bool,
}

/// A held loop in the rebuild.
struct NLoop {
    cue: Cue,
    c: Cursor,
    gain: f32,
    target: f32,
    on: bool,
}

/// Render one block the slow way: held first, then voices in slot order,
/// then the clamp.
fn naive_block(held: &mut [NLoop], voices: &mut [NVoice], held_rate: f32, out: &mut [f32]) {
    for s in out.iter_mut() {
        *s = 0.0;
    }
    let n = out.len() / 2;
    for h in held.iter_mut() {
        let (g0, g1) = (h.gain, h.target);
        if h.on {
            let b = pcm(h.cue);
            for (i, fr) in out.chunks_exact_mut(2).enumerate() {
                if h.c.pos >= b.len() as u32 {
                    h.c.pos %= b.len() as u32;
                }
                let x = sample(b, h.c, true) * ramp(g0, g1, i, n);
                fr[0] += x;
                fr[1] += x;
                step(&mut h.c, held_rate);
            }
        }
        h.gain = g1;
    }
    for v in voices.iter_mut().filter(|v| v.live) {
        let b = pcm(v.cue);
        for fr in out.chunks_exact_mut(2) {
            if v.c.pos >= b.len() as u32 - 1 {
                v.live = false;
                break;
            }
            let s = sample(b, v.c, false);
            fr[0] += s * v.gl;
            fr[1] += s * v.gr;
            step(&mut v.c, v.rate);
        }
    }
    for s in out.iter_mut() {
        *s = s.clamp(-1.0, 1.0);
    }
}

/// The scripted mix: two one-shots at unequal ear gains and rates, a loop
/// with two gain ramps, a third one-shot started late enough to be live
/// across the loop's wrap. Both sides are driven from this one table.
fn rebuild_script(blocks: usize) -> Vec<(usize, Cmd)> {
    vec![
        (0, start(Cue::StepRock, 0.3, 0.9, 1.0)),
        (0, start(Cue::ImpactWood, 0.9, 0.3, 1.37)),
        (
            0,
            Cmd::Loop {
                slot: 1,
                cue: Cue::BedSurf,
                gain: 0.5,
            },
        ),
        (3, Cmd::Gain { slot: 1, gain: 0.2 }),
        (
            6,
            Cmd::Gain {
                slot: 1,
                gain: 0.45,
            },
        ),
        (blocks - 40, start(Cue::Growl, 0.6, 0.6, 0.83)),
    ]
}

fn rebuild_at(out_rate: u32) {
    let surf = pcm(Cue::BedSurf).len();
    // Enough blocks for the loop to wrap once at this rate, plus a margin.
    let held_rate = rate(1.0, out_rate);
    let blocks = (surf as f32 / held_rate / BLOCK as f32) as usize + 24;
    let script = rebuild_script(blocks);

    let mut r = loaded(out_rate);
    let mut held: Vec<NLoop> = (0..HELD)
        .map(|_| NLoop {
            cue: Cue::BedSurf,
            c: Cursor { pos: 0, frac: 0.0 },
            gain: 0.0,
            target: 0.0,
            on: false,
        })
        .collect();
    let mut voices: Vec<NVoice> = Vec::new();

    let mut got = block();
    let mut want = block();
    let mut wrapped = false;
    for blk in 0..blocks {
        for (_, cmd) in script.iter().filter(|(at, _)| *at == blk) {
            r.push(*cmd);
            match *cmd {
                Cmd::Start {
                    cue,
                    gain_l,
                    gain_r,
                    rate,
                    ..
                } => voices.push(NVoice {
                    cue,
                    c: Cursor { pos: 0, frac: 0.0 },
                    rate,
                    gl: gain_l,
                    gr: gain_r,
                    live: true,
                }),
                Cmd::Loop { slot, cue, gain } => {
                    held[slot as usize] = NLoop {
                        cue,
                        c: Cursor { pos: 0, frac: 0.0 },
                        gain,
                        target: gain,
                        on: true,
                    }
                }
                Cmd::Gain { slot, gain } => held[slot as usize].target = gain,
                other => panic!("the rebuild script does not model {other:?}"),
            }
        }
        r.render(&mut got);
        naive_block(&mut held, &mut voices, held_rate, &mut want);
        wrapped |= held[1].c.pos < BLOCK as u32 && blk > 0;
        for i in 0..2 * BLOCK {
            assert_eq!(
                got[i].to_bits(),
                want[i].to_bits(),
                "at {out_rate} Hz, block {blk} sample {} ({} ear): the engine produced {} where \
                 the rebuild from synth::pcm, engine::rate and the stated lerp/ramp/clamp gives \
                 {} — the renderer has drifted from its own published law",
                i / 2,
                if i % 2 == 0 { "left" } else { "right" },
                got[i],
                want[i]
            );
        }
    }
    assert!(
        wrapped,
        "the rebuild never crossed the loop's wrap, so the wrap law was not compared"
    );
    assert!(
        voices.iter().filter(|v| v.live).count() == 1 && r.live() == 1,
        "by the end only the late Growl should be live on both sides (rebuild {}, engine {})",
        voices.iter().filter(|v| v.live).count(),
        r.live()
    );
}

/// (4) The naive rebuild, at the bank's own rate (every rate 1.0 tap is an
/// exact copy, so a wrong neighbour is invisible — which is why the 48 kHz
/// pass exists) and at a device rate that makes every held tap fractional.
#[test]
fn every_sample_is_rebuilt_from_published_parts() {
    rebuild_at(44_100);
    rebuild_at(48_000);
}

// ---------------------------------------------------------------------------
// Bounds.
// ---------------------------------------------------------------------------

/// (2) Every queue and pool has its cap, its policy and its counter.
#[test]
fn the_pool_refuses_the_forty_first_voice_and_never_steals() {
    let mut r = loaded(44_100);
    for _ in 0..VOICE_SLOTS + 1 {
        r.push(start(Cue::BedWind, 0.1, 0.1, 1.0));
    }
    let mut out = block();
    r.render(&mut out);
    assert_eq!(
        r.live(),
        VOICE_SLOTS,
        "forty-one simultaneous starts should leave exactly the pool's forty live"
    );
    assert_eq!(
        r.refused, 1,
        "the forty-first start must be refused and counted, not stolen from an older voice"
    );
    // The first voice is still the first voice: nothing was stolen.
    r.push(Cmd::CutVoices);
    r.render(&mut out);
    assert_eq!(r.live(), 0, "CutVoices must end every one-shot");
}

#[test]
fn the_command_queue_drops_the_newest_and_counts_it() {
    let mut r = loaded(44_100);
    for i in 0..CMD_RING_CAP + 1 {
        r.push(Cmd::Gain {
            slot: 0,
            gain: i as f32,
        });
    }
    assert_eq!(
        r.dropped, 1,
        "the sixty-fifth push in one block must be dropped and counted"
    );
    let mut out = block();
    r.render(&mut out);
    assert_eq!(
        r.dropped, 1,
        "rendering must not count the queue's drain as a drop"
    );
    r.push(Cmd::CutVoices);
    assert_eq!(
        r.dropped, 1,
        "the queue must be empty again after a render, so a fresh push is not dropped"
    );
}

#[test]
fn the_native_ring_drops_past_its_cap_and_drains_a_block_at_a_time() {
    let (mut out, mut inp) = channel();
    for _ in 0..CMD_RING_CAP_NATIVE {
        assert!(
            out.send(Cmd::CutVoices),
            "the ring refused a send below its cap"
        );
    }
    assert!(
        !out.send(Cmd::CutVoices) && !out.send(Cmd::CutVoices),
        "a send past CMD_RING_CAP_NATIVE must be refused"
    );
    assert_eq!(
        out.ring_dropped, 2,
        "every refused send must be counted in ring_dropped"
    );
    assert_eq!(out.backlog(), CMD_RING_CAP_NATIVE);
    let mut r = Renderer::new(48_000);
    inp.drain_into(&mut r);
    assert_eq!(
        out.backlog(),
        CMD_RING_CAP_NATIVE - CMD_RING_CAP,
        "one drain must apply at most CMD_RING_CAP commands, so a backlog is worked off a block at a time"
    );
    // The mailbox back: one slot, and it keeps the OLDEST unread report —
    // a post while it is full is skipped, not a replacement.
    assert!(inp.report(&r), "an empty mailbox must accept a report");
    r.blocks = 7;
    assert!(
        !inp.report(&r),
        "a mailbox holding an unread report must refuse rather than grow"
    );
    let s = out.stats().expect("a posted report must be readable");
    assert_eq!(
        s.blocks, 0,
        "the report read is the one that was posted first, not the one that was refused"
    );
    assert!(out.stats().is_none(), "a report is read once");
    // And the next post after a read lands, carrying the newer counters —
    // nothing was lost, only deferred.
    assert!(inp.report(&r), "a read mailbox must accept the next report");
    assert_eq!(out.stats().map(|s| s.blocks), Some(7));
}

/// A second install of a cue is refused, counted, and the samples come back
/// to the caller unfreed — natively over the return ring to the game thread,
/// which is where a free may happen.
#[test]
fn a_second_install_of_a_cue_is_refused_counted_and_handed_back() {
    let mut r = Renderer::new(44_100);
    let first: Box<[i16]> = pcm(Cue::Gather).into();
    assert!(r.install(Cue::Gather, first).is_ok());
    let second: Box<[i16]> = vec![7i16; 16].into_boxed_slice();
    let again = r
        .install(Cue::Gather, second)
        .expect_err("a second install of an installed cue must be refused");
    assert_eq!(
        again.as_ref(),
        &[7i16; 16],
        "the refused box must come back to the caller, not a different one"
    );
    assert_eq!(
        r.installed(),
        1,
        "a refused install must not change what is installed"
    );
    assert_eq!(
        r.stats().reinstall_refused,
        1,
        "a refused install is counted"
    );
    // The first samples are still the ones that play.
    let mut out = block();
    r.push(start(Cue::Gather, 1.0, 1.0, 1.0));
    r.render(&mut out);
    let want = pcm(Cue::Gather)[1] as f32 * (1.0 / 32768.0);
    assert_eq!(
        out[2].to_bits(),
        want.to_bits(),
        "the refused samples replaced the installed ones"
    );

    // Through the seam: the refusal rides the return ring to the game thread.
    let (mut out, mut inp) = channel();
    let mut r = Renderer::new(44_100);
    assert!(out.install(Cue::Bird, pcm(Cue::Bird).into()));
    assert!(out.install(Cue::Bird, pcm(Cue::Bird).into()));
    assert_eq!(out.reclaim(), 0, "nothing comes back before the drain");
    inp.drain_into(&mut r);
    assert_eq!(r.installed(), 1);
    assert_eq!(r.stats().reinstall_refused, 1);
    assert_eq!(
        out.reclaim(),
        1,
        "the refused install must come back to the game thread to be dropped there"
    );
    assert_eq!(out.reclaim(), 0, "a box is handed back once");
    assert!(inp.report(&r));
    let s = out.stats().expect("a report");
    assert_eq!(s.reinstall_refused, 1);
    assert_eq!(
        s.return_dropped, 0,
        "the return ring had room, so nothing was freed on the audio thread"
    );
}

#[test]
fn a_play_onto_a_busy_slot_cuts_what_was_there_and_counts_it() {
    let mut r = loaded(44_100);
    let mut out = block();
    r.push(Cmd::Play {
        slot: 3,
        cue: Cue::MusicOpenCalm,
        gain: 1.0,
    });
    r.render(&mut out);
    assert_eq!(r.cut, 0);
    r.push(Cmd::Play {
        slot: 3,
        cue: Cue::MusicTurnTense,
        gain: 1.0,
    });
    r.render(&mut out);
    assert_eq!(
        r.cut, 1,
        "a Play onto a playing slot must end the piece and count the cut"
    );
    r.push(Cmd::Loop {
        slot: 3,
        cue: Cue::BedUnder,
        gain: 0.3,
    });
    r.render(&mut out);
    assert_eq!(r.cut, 2, "a Loop onto a busy slot is a cut too");
    r.push(Cmd::Stop { slot: 3 });
    r.push(Cmd::Play {
        slot: 3,
        cue: Cue::MusicCloseCalm,
        gain: 1.0,
    });
    r.render(&mut out);
    assert_eq!(r.cut, 2, "a Play onto a stopped slot cuts nothing");
    // The piece that replaced the first starts from ITS sample 0, so the
    // first output sample is that piece's first sample and not a leftover.
    let want = pcm(Cue::MusicCloseCalm)[0] as f32 * (1.0 / 32768.0);
    assert_eq!(
        out[0].to_bits(),
        want.to_bits(),
        "a Play must start its piece at sample 0"
    );
}

#[test]
fn malformed_commands_are_refused_and_counted() {
    let mut r = loaded(44_100);
    r.apply(Cmd::Loop {
        slot: HELD as u8,
        cue: Cue::BedWind,
        gain: 1.0,
    });
    r.apply(Cmd::Gain {
        slot: 200,
        gain: 1.0,
    });
    r.apply(Cmd::Stop { slot: HELD as u8 });
    r.apply(start(Cue::Swing, 1.0, 1.0, 0.0));
    r.apply(start(Cue::Swing, f32::NAN, 1.0, 1.0));
    r.apply(Cmd::Play {
        slot: 0,
        cue: Cue::MusicOpenCalm,
        gain: f32::INFINITY,
    });
    // Outside the mixer's speed band, by a hair on either side. At the bank's
    // own rate the band is `SPEED_MIN..=SPEED_MAX` exactly.
    r.apply(start(Cue::Swing, 1.0, 1.0, rate(SPEED_MIN, 44_100) - 1e-6));
    r.apply(start(Cue::Swing, 1.0, 1.0, rate(SPEED_MAX, 44_100) + 1e-6));
    assert_eq!(
        r.bad_cmd, 8,
        "a slot past HELD, a rate of zero, a gain that is not finite, and a rate outside the \
         mixer's band are each a bad command"
    );
    assert_eq!(r.live(), 0, "a bad Start must not take a voice");
    assert!(!r.slot_busy(0), "a bad Play must not take a slot");
    // The rails themselves are inside the band: the mixer's clamp is
    // inclusive and a `Start` built from a clamped speed must never be
    // refused.
    r.apply(start(Cue::Swing, 1.0, 1.0, rate(SPEED_MIN, 44_100)));
    r.apply(start(Cue::Swing, 1.0, 1.0, rate(SPEED_MAX, 44_100)));
    assert_eq!(r.bad_cmd, 8, "a rate at either rail is not a bad command");
    assert_eq!(r.live(), 2, "a rate at either rail takes a voice");
}

#[test]
fn an_uninstalled_cue_is_silence_and_counted_except_a_loop_which_waits() {
    let mut r = Renderer::new(44_100);
    assert!(r.install(Cue::UiClick, pcm(Cue::UiClick).into()).is_ok());
    r.apply(start(Cue::Swing, 1.0, 1.0, 1.0));
    r.apply(Cmd::Play {
        slot: 4,
        cue: Cue::MusicOpenCalm,
        gain: 1.0,
    });
    r.apply(Cmd::Loop {
        slot: 0,
        cue: Cue::BedWind,
        gain: 1.0,
    });
    assert_eq!(
        r.unbanked, 2,
        "a Start and a Play on an uninstalled cue are each counted as unbanked"
    );
    assert_eq!(r.live(), 0, "an unbanked Start takes no voice");
    assert!(!r.slot_busy(4), "an unbanked Play frees its slot");
    assert!(
        r.slot_busy(0),
        "an unbanked Loop must hold its slot and wait for the bank"
    );
    assert_eq!(r.installed(), 1);
}

// ---------------------------------------------------------------------------
// Determinism.
// ---------------------------------------------------------------------------

/// A deterministic script generator: xorshift over the whole vocabulary.
fn scripted(seed: u32, blk: usize, out: &mut Vec<Cmd>) {
    out.clear();
    let mut x = seed ^ (blk as u32).wrapping_mul(0x9E37_79B9) | 1;
    let mut roll = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    let n = (roll() % 5) as usize;
    for _ in 0..n {
        let cue = Cue::ALL[(roll() % CUE_COUNT as u32) as usize];
        let slot = (roll() % HELD as u32) as u8;
        let g = (roll() % 1000) as f32 / 1000.0;
        let cmd = match roll() % 7 {
            0..=2 => start(cue, g, 1.0 - g, 0.25 + (roll() % 375) as f32 / 100.0),
            3 => Cmd::Loop { slot, cue, gain: g },
            4 => Cmd::Play { slot, cue, gain: g },
            5 => Cmd::Gain { slot, gain: g },
            _ => {
                if roll() % 8 == 0 {
                    Cmd::CutVoices
                } else {
                    Cmd::Stop { slot }
                }
            }
        };
        out.push(cmd);
    }
}

/// (3) Same script, same bits, on every sample of 500 blocks.
#[test]
fn two_renderers_on_one_script_agree_to_the_bit() {
    let mut a = loaded(48_000);
    let mut b = loaded(48_000);
    let (mut oa, mut ob) = (block(), block());
    let mut cmds = Vec::new();
    let mut nonzero = 0usize;
    for blk in 0..500 {
        scripted(0xC0FFEE, blk, &mut cmds);
        for c in &cmds {
            a.push(*c);
            b.push(*c);
        }
        a.render(&mut oa);
        b.render(&mut ob);
        for i in 0..2 * BLOCK {
            assert_eq!(
                oa[i].to_bits(),
                ob[i].to_bits(),
                "block {blk} sample {i}: two renderers fed one script diverged — something in the \
                 render path reads state that is not the script"
            );
        }
        nonzero += oa.iter().filter(|s| **s != 0.0).count();
    }
    assert!(
        nonzero > 10_000,
        "the script produced near silence, so the comparison proved little"
    );
    assert_eq!(
        a.stats(),
        b.stats(),
        "the counters are part of the determinism claim"
    );
}

// ---------------------------------------------------------------------------
// Loops, pieces and the join.
// ---------------------------------------------------------------------------

fn rms(s: &[f32]) -> f32 {
    (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
}

/// (5) A bed rendered across its wrap: the join is no larger a step than the
/// bank's own largest, and the level either side of it agrees.
#[test]
fn a_loop_wraps_without_a_seam() {
    let bed = Cue::BedWind;
    let b = pcm(bed);
    let len = b.len();
    let mut r = loaded(44_100);
    r.push(Cmd::Loop {
        slot: 0,
        cue: bed,
        gain: 1.0,
    });
    // Left channel of every block up to the join and a tenth of a second past
    // it — the window the level comparison below needs on each side.
    let w = SAMPLE_RATE as usize / 10;
    let blocks = (len + w) / BLOCK + 2;
    let mut left: Vec<f32> = Vec::with_capacity(blocks * BLOCK);
    let mut out = block();
    for _ in 0..blocks {
        r.render(&mut out);
        left.extend(out.chunks_exact(2).map(|fr| fr[0]));
    }
    // The engine's output IS the bank, wrapped: sample k is bank[k % len].
    for (k, s) in left.iter().enumerate() {
        let want = b[k % len] as f32 * (1.0 / 32768.0);
        assert_eq!(
            s.to_bits(),
            want.to_bits(),
            "output sample {k} is not bank[{}] — the loop does not wrap at the bank's length",
            k % len
        );
    }
    let bank_step = b
        .windows(2)
        .map(|w| (w[1] as f32 - w[0] as f32).abs())
        .fold(0.0f32, f32::max)
        * (1.0 / 32768.0);
    let join_step = (left[len] - left[len - 1]).abs();
    assert!(
        join_step <= bank_step,
        "the step across the join ({join_step}) exceeds the bank's own largest step ({bank_step}) — \
         the wrap would click once every loop, forever"
    );
    let before = rms(&left[len - w..len]);
    let after = rms(&left[len..len + w]);
    let ratio = after / before;
    assert!(
        (0.9..=1.1).contains(&ratio),
        "the level after the join is {ratio:.3}x the level before it — the crossfade is audible"
    );
}

/// (6) A piece frees its slot when it ends; a loop holds it forever.
#[test]
fn a_play_frees_its_slot_when_the_piece_ends_and_a_loop_does_not() {
    let mut r = loaded(44_100);
    let click = pcm(Cue::UiClick).len();
    r.push(Cmd::Play {
        slot: 5,
        cue: Cue::UiClick,
        gain: 1.0,
    });
    r.push(Cmd::Loop {
        slot: 0,
        cue: Cue::UiClick,
        gain: 1.0,
    });
    let mut out = block();
    let blocks = click / BLOCK + 2;
    for blk in 0..blocks {
        r.render(&mut out);
        if blk == 0 {
            assert!(
                r.slot_busy(5),
                "the piece must be playing on its first block"
            );
        }
    }
    assert!(
        !r.slot_busy(5),
        "a Play must free its slot once its piece has reached its last sample"
    );
    assert!(
        r.slot_busy(0),
        "a Loop of the same cue must still hold its slot after the cue's length has passed"
    );
    // The loop is audible: it wrapped and kept going.
    assert!(
        out.iter().any(|s| *s != 0.0),
        "the loop went silent after the cue's length — it ended instead of wrapping"
    );
    assert_eq!(r.cut, 0, "a slot freed by its own ending is not a cut");
}

/// (11) A loop asked for before its cue landed starts at sample 0 the block
/// the bank lands — the loading screen's contract.
#[test]
fn a_loop_sent_before_its_cue_is_installed_starts_at_sample_zero_when_it_lands() {
    let mut r = Renderer::new(44_100);
    r.push(Cmd::Loop {
        slot: 0,
        cue: Cue::BedWind,
        gain: 1.0,
    });
    let mut out = block();
    for _ in 0..3 {
        r.render(&mut out);
        assert!(
            out.iter().all(|s| *s == 0.0),
            "a loop whose cue is not installed must be silence, not noise"
        );
    }
    assert!(
        r.slot_busy(0),
        "the pending loop must hold its slot while it waits"
    );
    assert!(r.install(Cue::BedWind, pcm(Cue::BedWind).into()).is_ok());
    r.render(&mut out);
    let b = pcm(Cue::BedWind);
    for (i, fr) in out.chunks_exact(2).enumerate() {
        let want = b[i] as f32 * (1.0 / 32768.0);
        assert_eq!(
            fr[0].to_bits(),
            want.to_bits(),
            "sample {i} of the block the bank landed in is not bank[{i}] — the loop did not start \
             from sample 0 on arrival"
        );
    }
    assert_eq!(r.unbanked, 0, "a waiting loop is not an unbanked play");
}

/// (13) Stop silences the next block; CutVoices ends every one-shot.
#[test]
fn stop_silences_the_next_block_and_cut_voices_ends_every_one_shot() {
    let mut r = loaded(44_100);
    let mut out = block();
    r.push(Cmd::Loop {
        slot: 2,
        cue: Cue::BedSurf,
        gain: 1.0,
    });
    r.render(&mut out);
    r.render(&mut out);
    assert!(
        out.iter().any(|s| *s != 0.0),
        "the bed must be audible before the stop"
    );
    r.push(Cmd::Stop { slot: 2 });
    r.render(&mut out);
    assert!(
        out.iter().all(|s| *s == 0.0),
        "the block after a Stop must be silent"
    );
    assert!(!r.slot_busy(2));

    for _ in 0..10 {
        r.push(start(Cue::TreeFall, 0.5, 0.5, 1.0));
    }
    r.render(&mut out);
    assert_eq!(r.live(), 10);
    r.push(Cmd::CutVoices);
    r.render(&mut out);
    assert_eq!(r.live(), 0, "CutVoices must end every one-shot");
    assert!(
        out.iter().all(|s| *s == 0.0),
        "the block after CutVoices must be silent with nothing held"
    );
}

// ---------------------------------------------------------------------------
// Resampling.
// ---------------------------------------------------------------------------

/// How many blocks until no one-shot is live.
fn blocks_until_silent(r: &mut Renderer, cap: usize) -> usize {
    let mut out = block();
    for n in 1..=cap {
        r.render(&mut out);
        if r.live() == 0 {
            return n;
        }
    }
    panic!("still live after {cap} blocks");
}

/// (8) A voice at rate 4 ends within `len/4 + 1` samples, one at 0.25 does
/// not end early, and neither reads `bank[len]` — an index past the end
/// would have panicked, which is the assertion. A rate past the mixer's band
/// never reaches a cursor at all: it is refused as a bad command.
#[test]
fn resampling_ends_where_the_arithmetic_says_and_never_reads_past_the_bank() {
    let cue = Cue::Splash;
    let len = pcm(cue).len();

    let mut r = loaded(44_100);
    r.push(start(cue, 1.0, 1.0, 4.0));
    let n = blocks_until_silent(&mut r, len);
    assert!(
        (n - 1) * BLOCK <= len / 4 + 1,
        "at rate 4 the voice was still live after {} samples, past len/4 + 1 = {}",
        (n - 1) * BLOCK,
        len / 4 + 1
    );
    assert!(
        n * BLOCK + BLOCK > len / 4,
        "at rate 4 the voice ended after only {} samples, before len/4 = {}",
        n * BLOCK,
        len / 4
    );

    let mut r = loaded(44_100);
    r.push(start(cue, 1.0, 1.0, 0.25));
    let expect = ((len - 1) * 4).div_ceil(BLOCK);
    let n = blocks_until_silent(&mut r, expect + 4);
    assert_eq!(
        n, expect,
        "at rate 0.25 the voice must end in the block that reaches sample (len-1)*4, not early"
    );

    let mut r = loaded(44_100);
    r.push(start(cue, 1.0, 1.0, 1.0));
    let n = blocks_until_silent(&mut r, len);
    assert_eq!(
        n,
        (len - 1).div_ceil(BLOCK),
        "at rate 1 the voice ends at sample len-1"
    );

    // A rate past the bank's length is past the mixer's band too: refused
    // and counted before it can take a voice, so nothing is ever read at
    // that stride.
    let mut r = loaded(44_100);
    r.push(start(cue, 1.0, 1.0, len as f32 * 2.0));
    let mut out = block();
    r.render(&mut out);
    assert_eq!(
        r.bad_cmd, 1,
        "a rate past SPEED_MAX must be refused as a bad command"
    );
    assert_eq!(r.live(), 0, "a refused rate must not take a voice");
}

// ---------------------------------------------------------------------------
// The pan.
// ---------------------------------------------------------------------------

/// The listener's right vector for a wire yaw — `look::right_dir`'s
/// arithmetic, `(−fz, fx)`, restated here because this crate takes the
/// vector and the client owns the convention.
fn right_of(yaw: u16) -> ([f32; 2], [f32; 3]) {
    let (fx, fz) = yaw_dir(yaw);
    ([-fz, fx], [fx, 0.0, fz])
}

/// (7) Equal power over the arc, a hard side is one ear, ahead is both —
/// every property stated against the right vector, never a yaw.
#[test]
fn the_pan_is_equal_power_and_sided_against_the_right_vector() {
    for yaw in [0u16, 8_192, 16_384, 40_000, 65_000] {
        let (right, fwd) = right_of(yaw);
        for k in 0..64 {
            let phi = k as f32 * core::f32::consts::TAU / 64.0;
            let (gl, gr) = pan([phi.cos() * 7.0, 1.5, phi.sin() * 7.0], right);
            let power = gl * gl + gr * gr;
            assert!(
                (power - 1.0).abs() <= 1e-6,
                "yaw {yaw} azimuth {k}: gl² + gr² = {power}, the pan is not equal-power"
            );
            assert!(
                gl >= -1e-6 && gr >= -1e-6,
                "a pan gain must not go negative"
            );
        }
        let (gl, gr) = pan([right[0] * 3.0, 0.0, right[1] * 3.0], right);
        assert!(
            gl.abs() < 1e-6 && (gr - 1.0).abs() < 1e-6,
            "yaw {yaw}: a source along the right vector must be all right ear, got ({gl}, {gr})"
        );
        let (gl, gr) = pan([-right[0] * 3.0, 0.0, -right[1] * 3.0], right);
        assert!(
            (gl - 1.0).abs() < 1e-6 && gr.abs() < 1e-6,
            "yaw {yaw}: a source opposite the right vector must be all left ear, got ({gl}, {gr})"
        );
        let (gl, gr) = pan([fwd[0] * 5.0, 2.0, fwd[2] * 5.0], right);
        assert_eq!(
            gl.to_bits(),
            gr.to_bits(),
            "yaw {yaw}: a source dead ahead must reach both ears equally, got ({gl}, {gr})"
        );
        assert!(
            (gl - core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6,
            "yaw {yaw}: the centre of an equal-power pan is 1/√2 per ear, got {gl}"
        );
        let (gl, gr) = pan([-fwd[0], 0.0, -fwd[2]], right);
        assert_eq!(gl.to_bits(), gr.to_bits(), "dead behind is centred too");
    }
    let (gl, gr) = pan([0.0, 3.0, 0.0], [1.0, 0.0]);
    assert_eq!(
        gl.to_bits(),
        gr.to_bits(),
        "a source on the listener's own axis has no side and is centred"
    );
}

#[test]
fn start_cmd_pans_a_positional_cue_and_leaves_an_own_fact_at_the_table_gain() {
    let (right, fwd) = right_of(12_345);
    let listener = [100.0, 20.0, -40.0];
    let own = Start {
        cue: Cue::Swing,
        gain: 0.45,
        at: None,
        speed: 1.03,
    };
    let Cmd::Start {
        cue,
        gain_l,
        gain_r,
        rate: r,
        ..
    } = start_cmd(&own, listener, right, 48_000)
    else {
        panic!("start_cmd must build a Start")
    };
    assert_eq!(cue, Cue::Swing);
    assert!(
        gain_l == 0.45 && gain_r == 0.45,
        "an own-fact is heard at the table's gain in both ears, no pan, got ({gain_l}, {gain_r})"
    );
    assert_eq!(
        r.to_bits(),
        rate(1.03, 48_000).to_bits(),
        "the rate is engine::rate of the mixer's speed at the device rate"
    );

    let ahead = Start {
        cue: Cue::ImpactWood,
        gain: 0.6,
        at: Some([
            listener[0] + fwd[0] * 9.0,
            listener[1],
            listener[2] + fwd[2] * 9.0,
        ]),
        speed: 1.0,
    };
    let Cmd::Start { gain_l, gain_r, .. } = start_cmd(&ahead, listener, right, 44_100) else {
        panic!()
    };
    // Within rounding rather than to the bit: `p − listener` is a subtraction
    // that does not return exactly `fwd · 9`, and the bit-exact centre is
    // `the_pan_is_equal_power_and_sided_against_the_right_vector`'s claim.
    assert!(
        (gain_l - gain_r).abs() < 1e-6,
        "a positional cue dead ahead must reach both ears equally, got ({gain_l}, {gain_r})"
    );
    assert!(
        (gain_l - 0.6 * core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6,
        "a positional cue dead ahead is gain/√2 per ear, not gain — a source sweeping past \
         must not get louder in the middle; got {gain_l}"
    );
    let side = Start {
        at: Some([
            listener[0] + right[0] * 4.0,
            listener[1] + 3.0,
            listener[2] + right[1] * 4.0,
        ]),
        ..ahead
    };
    let Cmd::Start { gain_l, gain_r, .. } = start_cmd(&side, listener, right, 44_100) else {
        panic!()
    };
    assert!(
        gain_l.abs() < 1e-6 && (gain_r - 0.6).abs() < 1e-6,
        "a positional cue hard right is silent in the left ear and at full gain in the right"
    );
}

// ---------------------------------------------------------------------------
// The codec.
// ---------------------------------------------------------------------------

/// (9) A fixed script's bytes, pinned. The floats are 0.3, 0.9, 1.37, 0.5,
/// 1.0 and 0.2 in little-endian IEEE-754 — the layout is the contract the
/// browser transport is written against, so a change here is a change there.
#[test]
fn the_command_codec_is_pinned_and_refuses_a_bad_record() {
    let script = [
        start(Cue::Swing, 0.3, 0.9, 1.37),
        Cmd::Loop {
            slot: 0,
            cue: Cue::BedWind,
            gain: 0.5,
        },
        Cmd::Play {
            slot: 3,
            cue: Cue::MusicOpenCalm,
            gain: 1.0,
        },
        Cmd::Gain { slot: 3, gain: 0.2 },
        Cmd::Stop { slot: 6 },
        Cmd::CutVoices,
    ];
    let golden: [[u8; CMD_BYTES]; 6] = [
        [
            0, 5, 0, 0, 154, 153, 153, 62, 102, 102, 102, 63, 41, 92, 175, 63,
        ],
        [1, 19, 0, 0, 0, 0, 0, 63, 0, 0, 0, 0, 0, 0, 0, 0],
        [2, 29, 3, 0, 0, 0, 128, 63, 0, 0, 0, 0, 0, 0, 0, 0],
        [3, 0, 3, 0, 205, 204, 76, 62, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 0, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ];
    for (cmd, want) in script.iter().zip(golden.iter()) {
        let got = cmd.to_bytes();
        assert_eq!(
            &got, want,
            "{cmd:?} encodes differently from the pinned record — the browser transport \
             reads this layout"
        );
        assert_eq!(
            Cmd::from_bytes(&got),
            Some(*cmd),
            "{cmd:?} does not survive a round trip"
        );
    }
    // Every cue and every slot round-trips.
    for cue in Cue::ALL {
        let c = start(cue, 0.0, 1.0, 2.5);
        assert_eq!(Cmd::from_bytes(&c.to_bytes()), Some(c));
    }
    for slot in 0..HELD as u8 {
        let c = Cmd::Stop { slot };
        assert_eq!(Cmd::from_bytes(&c.to_bytes()), Some(c));
    }
    let mut bad = script[0].to_bytes();
    bad[1] = CUE_COUNT as u8;
    assert_eq!(
        Cmd::from_bytes(&bad),
        None,
        "a cue byte off Cue::ALL must decode to nothing, not to a nearby cue"
    );
    let mut bad = script[1].to_bytes();
    bad[2] = HELD as u8;
    assert_eq!(
        Cmd::from_bytes(&bad),
        None,
        "a slot byte at HELD must decode to nothing"
    );
    let mut bad = script[5].to_bytes();
    bad[0] = 6;
    assert_eq!(
        Cmd::from_bytes(&bad),
        None,
        "an unknown tag must decode to nothing"
    );
}

/// A cue's takes are equal slices of its bank: take 1 of 3 plays exactly the
/// middle third and ends inside it. A low-passed voice is the one pole the
/// header states; every rebuild above runs at `lp == 0`, which is what holds
/// an unfiltered voice to its old bits.
#[test]
fn a_take_plays_its_own_slice_and_the_low_pass_is_one_pole() {
    const SPAN: usize = 200;
    // Three ramps at three levels, so a sample says which take it came from.
    let takes: Vec<i16> = (0..3 * SPAN)
        .map(|i| ((i / SPAN) as i16 + 1) * 1000 + (i % SPAN) as i16)
        .collect();
    let mut r = Renderer::new(SAMPLE_RATE);
    assert!(r.install(Cue::Knock, takes.clone().into()).is_ok());
    r.push(start(Cue::Knock, 1.0, 1.0, 1.0).with_take(1, 3));
    let mut heard = Vec::new();
    for _ in 0..3 {
        let mut out = block();
        r.render(&mut out);
        heard.extend(out.chunks_exact(2).map(|f| f[0]));
    }
    for (i, h) in heard.iter().enumerate().take(SPAN - 1) {
        assert_eq!(
            *h,
            takes[SPAN + i] as f32 * (1.0 / 32768.0),
            "sample {i} is not take 1's"
        );
    }
    assert!(
        heard[SPAN - 1..].iter().all(|h| *h == 0.0),
        "take 1 ran on into take 2"
    );
    assert_eq!(r.live(), 0);
    // A take past its takes is refused, and so is a record that says so.
    r.apply(start(Cue::Knock, 1.0, 1.0, 1.0).with_take(3, 3));
    assert_eq!(r.bad_cmd, 1);
    let c = start(Cue::Knock, 0.5, 0.25, 1.0).with_take(4, 5);
    let Cmd::Start {
        cue,
        gain_l,
        gain_r,
        rate,
        ..
    } = c
    else {
        unreachable!()
    };
    let lp = Cmd::Start {
        cue,
        take: 4,
        takes: 5,
        lp: 77,
        gain_l,
        gain_r,
        rate,
    };
    assert_eq!(Cmd::from_bytes(&lp.to_bytes()), Some(lp));
    let mut bad = lp.to_bytes();
    bad[2] = 0x45; // take 5 of 5
    assert_eq!(Cmd::from_bytes(&bad), None);

    // The filter: a step through one pole, bit for bit.
    let mut r = Renderer::new(SAMPLE_RATE);
    assert!(r
        .install(Cue::Knock, vec![16_384i16; 4 * BLOCK].into())
        .is_ok());
    r.push(Cmd::Start {
        cue: Cue::Knock,
        take: 0,
        takes: 1,
        lp: 40,
        gain_l: 1.0,
        gain_r: 1.0,
        rate: 1.0,
    });
    let mut out = block();
    r.render(&mut out);
    let (a, x) = (40.0f32 * (1.0 / 256.0), 16_384.0f32 * (1.0 / 32768.0));
    let mut y = 0.0f32;
    for (i, f) in out.chunks_exact(2).enumerate() {
        y += a * (x - y);
        assert_eq!(f[0], y, "sample {i} is not the one-pole's");
    }
    // Distance muffles past `LP_FROM` of the radius, more the further out.
    let near = engine::lp_of(0.2 * 40.0, 40.0, 48_000);
    let mid = engine::lp_of(0.5 * 40.0, 40.0, 48_000);
    let far = engine::lp_of(0.95 * 40.0, 40.0, 48_000);
    assert_eq!(near, 0, "a near sound is not filtered");
    assert!(mid > far && far > 0, "{mid} then {far}");
}

// ---------------------------------------------------------------------------
// The ledger.
// ---------------------------------------------------------------------------

/// (10) The game thread's prediction of the live count matches the renderer
/// frame for frame, through mixed rates and a cut.
#[test]
fn the_live_ledger_matches_the_renderer_frame_for_frame() {
    const BLOCKS_PER_FRAME: usize = 4;
    let out_rate = 44_100;
    let dt = (BLOCKS_PER_FRAME * BLOCK) as f32 / out_rate as f32;
    let mut r = loaded(out_rate);
    let mut ledger = Live::new();
    let mut out = block();
    let starts: [(usize, Cue, f32); 12] = [
        (0, Cue::StepRock, 1.0),
        (0, Cue::ImpactWood, 1.37),
        (2, Cue::Gather, 0.8),
        (5, Cue::Splash, 0.25),
        (5, Cue::TreeFall, 1.0),
        (9, Cue::Howl, 0.91),
        (30, Cue::Snort, 1.1),
        (31, Cue::Growl, 4.0),
        (60, Cue::ShotGun, 1.0),
        (61, Cue::Bird, 1.16),
        (61, Cue::Death, 1.0),
        (90, Cue::Hurt, 0.93),
    ];
    let cut_at = 45;
    let mut peak = 0;
    for frame in 0..200 {
        for (_, cue, speed) in starts.iter().filter(|(f, _, _)| *f == frame) {
            r.push(start(*cue, 0.5, 0.5, rate(*speed, out_rate)));
            assert!(ledger.start(len_s(*cue), *speed));
        }
        if frame == cut_at {
            r.push(Cmd::CutVoices);
            ledger.cut();
        }
        for _ in 0..BLOCKS_PER_FRAME {
            r.render(&mut out);
        }
        ledger.tick(dt);
        assert_eq!(
            ledger.count(),
            r.live(),
            "frame {frame}: the ledger predicts {} live voices and the renderer has {} — the \
             mixer would be handed the wrong count",
            ledger.count(),
            r.live()
        );
        peak = peak.max(r.live());
    }
    assert!(
        peak >= 4,
        "the script never had four voices up at once, so it proved little"
    );
    assert_eq!(r.live(), 0, "everything should have ended by frame 200");
}

#[test]
fn the_ledger_refuses_a_forty_first_voice_like_the_pool() {
    let mut l = Live::new();
    for _ in 0..VOICE_SLOTS {
        assert!(l.start(1.0, 1.0));
    }
    assert!(
        !l.start(1.0, 1.0),
        "the ledger has exactly the pool's slots"
    );
    assert_eq!(l.count(), VOICE_SLOTS);
    l.tick(0.5);
    assert_eq!(l.count(), VOICE_SLOTS);
    l.tick(0.5);
    assert_eq!(l.count(), 0, "a one-second voice is over after one second");
}

// ---------------------------------------------------------------------------
// Memory and the bank.
// ---------------------------------------------------------------------------

/// (12) A renderer and a whole bank install fit in a 1 MiB stack — the wasm
/// shadow stack's size — because the samples arrive boxed and the arrays
/// around them are small.
#[test]
fn a_renderer_and_a_whole_bank_fit_in_a_one_mebibyte_stack() {
    let fresh: [Box<[i16]>; CUE_COUNT] = bank().clone();
    let done = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let mut r = Renderer::new(48_000);
            for (i, pcm) in fresh.into_iter().enumerate() {
                assert!(r.install(Cue::ALL[i], pcm).is_ok());
            }
            let mut out = block();
            r.push(Cmd::Loop {
                slot: 0,
                cue: Cue::BedWind,
                gain: 0.3,
            });
            r.push(start(Cue::ShotGun, 0.5, 0.5, 1.0));
            r.render(&mut out);
            (r.installed(), r.live(), out.iter().any(|s| *s != 0.0))
        })
        .expect("spawn")
        .join()
        .expect("the renderer overflowed a 1 MiB stack");
    assert_eq!(done, (CUE_COUNT, 1, true));
}

/// `synth::wav` is exactly the 44-byte header over `synth::pcm`, and the
/// PCM bank is the per-cue PCM in `Cue::ALL` order — so the rodio path and
/// the renderer play the same samples.
#[test]
fn the_wav_is_the_header_over_the_pcm_and_the_bank_is_the_cues_in_order() {
    let bank = bank();
    for (i, cue) in Cue::ALL.iter().enumerate() {
        let wav = synth::wav(*cue);
        let pcm = &bank[i];
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(
            wav.len(),
            44 + pcm.len() * 2,
            "{cue:?}: the WAV is not a 44-byte header over the PCM"
        );
        for (k, s) in pcm.iter().enumerate() {
            let got = i16::from_le_bytes([wav[44 + 2 * k], wav[45 + 2 * k]]);
            assert_eq!(got, *s, "{cue:?} sample {k}: the WAV and the PCM disagree");
        }
        assert_eq!(
            pcm.as_ref(),
            synth::pcm(*cue).as_ref(),
            "{cue:?}: pcm_bank()[{i}] is not pcm(Cue::ALL[{i}])"
        );
        assert!(pcm.len() >= 2, "{cue:?} is too short to lerp");
    }
}

/// The renderer's out_rate is what it was built with, floored at 1 so a
/// held voice's rate is never a division by zero.
#[test]
fn the_out_rate_is_kept_and_a_zero_is_floored() {
    assert_eq!(Renderer::new(48_000).out_rate(), 48_000);
    assert_eq!(Renderer::new(0).out_rate(), 1);
    assert_eq!(rate(1.0, SAMPLE_RATE), 1.0);
    assert_eq!(rate(2.0, SAMPLE_RATE), 2.0);
    assert!((rate(1.0, 48_000) - 0.91875).abs() < 1e-6);
    let s = engine::Stats::default();
    assert_eq!(s.blocks, 0);
}
