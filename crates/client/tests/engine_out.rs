//! The native output seam, with no device and no `App`:
//! `render/audio_out.rs`'s `Feed::fill`, gated headless.
//!
//! `crates/sound/tests/engine.rs` proves the renderer; this file proves the
//! callback body around it is a callback body and nothing more — that
//! `Feed::fill` hands out exactly the samples `Renderer::render` would have
//! produced for the same script, on `to_bits`, through callbacks of ANY
//! length (441 frames is the common one, and a block is 128), carrying the
//! straddling block across; that a command sent between two callbacks lands
//! before the next rendered block and never inside a carried one; that a
//! mono device gets the mean and a six-channel one gets `l, r, 0, 0, 0, 0`;
//! that an `i16` or `u16` device hears `cpal`'s own conversion of the same
//! float; that the game side's rings refuse past their caps and count; that
//! a refused install comes back to be dropped on the game thread; and that
//! the two atomics the audio thread publishes without a ring read back.
//!
//! **The mirror shares no code with the seam.** `Mirror` is a bare
//! `Renderer` fed the same installs and commands, whose blocks are queued
//! into an expected stream that models the carry by arithmetic —
//! `⌈(frames − carried) / BLOCK⌉` blocks per callback — so the equality is
//! against a second implementation of the carry, not against the carry. The
//! block counts are pinned by hand in `the_carry_survives_a_441_frame_callback`
//! so the model is itself checked against a number written down.
//!
//! **Mutants run by hand** (2026-09-14): the mono map taking the left ear
//! alone reddens `a_mono_device_gets_the_mean_and_a_six_channel_device_gets_l_r_zeros`;
//! the stereo ears swapped reddens `fill_is_the_renderer_block_for_block_through_any_callback_length`
//! at callback 0; the carry dropped (`at` reset at every callback) reddens
//! `the_carry_survives_a_441_frame_callback` on the second callback's first
//! frame and on its block count.
//!
//! **Native only.** `audio_out` is `cfg(not(wasm32))` — a page reaches the
//! same renderer through a worklet — so the guard below keeps the wasm32
//! renderer lint (`--target wasm32-unknown-unknown --all-targets`) from
//! opening a file that names it.
#![cfg(all(feature = "render", not(target_arch = "wasm32")))]

use std::collections::VecDeque;

use cpal::{FromSample, SizedSample};

use client::render::audio_out::{pair, Feed, Native};
use client::sound::engine::{rate, Cmd, Renderer, BLOCK, CMD_RING_CAP_NATIVE, HELD};
use client::sound::{synth, Cue, CUE_COUNT, SAMPLE_RATE};

/// The cues a script here plays: two beds, a music piece and three
/// one-shots — enough to exercise every command against real samples
/// without rendering the whole ~12 MB bank in a debug build.
const CUES: [Cue; 6] = [
    Cue::BedWind,
    Cue::BedSurf,
    Cue::Swing,
    Cue::Gather,
    Cue::Refused,
    Cue::Bird,
];

/// The callback lengths one script walks: the common 441, the block itself,
/// a multiple, a fraction, a long one and a tiny one.
const LENGTHS: [usize; 6] = [441, 128, 512, 64, 1000, 3];

fn xorshift(x: &mut u32) -> u32 {
    *x ^= *x << 13;
    *x ^= *x >> 17;
    *x ^= *x << 5;
    *x
}

/// One callback's commands from a fixed stream: every variant appears, and
/// the ears are unequal so a swapped or averaged ear is visible.
fn script(x: &mut u32, k: usize, out_rate: u32, out: &mut Vec<Cmd>) {
    let n = (xorshift(x) % 4) as usize;
    for _ in 0..n {
        let cue = CUES[(xorshift(x) % CUES.len() as u32) as usize];
        let slot = (xorshift(x) % HELD as u32) as u8;
        let g = (xorshift(x) % 1000) as f32 / 1000.0;
        let cmd = match xorshift(x) % 6 {
            0 => Cmd::Start {
                cue,
                gain_l: g,
                gain_r: 1.0 - g,
                rate: rate(0.9 + (xorshift(x) % 200) as f32 / 1000.0, out_rate),
            },
            1 => Cmd::Loop { slot, cue, gain: g },
            2 => Cmd::Play { slot, cue, gain: g },
            3 => Cmd::Gain { slot, gain: g },
            4 => Cmd::Stop { slot },
            _ => Cmd::CutVoices,
        };
        out.push(cmd);
    }
    // Something is always sounding from the first callback on, so silence
    // cannot pass.
    if k == 0 {
        out.push(Cmd::Loop {
            slot: 0,
            cue: Cue::BedWind,
            gain: 0.5,
        });
    }
}

/// The reference: a bare renderer and the interleaved stereo stream it has
/// rendered that the seam has not yet handed out — the carry, modelled by
/// arithmetic rather than by the seam's own bookkeeping.
struct Mirror {
    r: Renderer,
    expect: VecDeque<f32>,
}

impl Mirror {
    fn new(out_rate: u32) -> Self {
        Self {
            r: Renderer::new(out_rate),
            expect: VecDeque::new(),
        }
    }

    /// Frames rendered and not yet handed out.
    fn carried(&self) -> usize {
        self.expect.len() / 2
    }

    /// What a callback of `frames` frames must produce: exactly the blocks
    /// the seam will have to render to cover it, appended to the carry, then
    /// the first `frames` frames of that.
    fn expect_frames(&mut self, frames: usize) -> Vec<f32> {
        let need = frames.saturating_sub(self.carried()).div_ceil(BLOCK);
        let mut blk = [0.0f32; 2 * BLOCK];
        for _ in 0..need {
            self.r.render(&mut blk);
            self.expect.extend(blk.iter().copied());
        }
        self.expect.drain(..2 * frames).collect()
    }
}

/// Both sides with the test cues installed.
fn loaded(out_rate: u32) -> (Native, Feed, Mirror) {
    let (mut native, feed) = pair(out_rate);
    let mut mirror = Mirror::new(out_rate);
    for cue in CUES {
        let pcm = synth::pcm(cue);
        assert!(mirror.r.install(cue, pcm.clone()).is_ok());
        assert!(native.install(cue, pcm), "the install ring refused {cue:?}");
    }
    assert_eq!(native.install_refused, 0);
    (native, feed, mirror)
}

fn same_bits(got: &[f32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: lengths differ");
    for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
        assert_eq!(
            g.to_bits(),
            w.to_bits(),
            "{what}: sample {i} — the seam gave {g}, the renderer {w}"
        );
    }
}

// ---------------------------------------------------------------------------

/// (1) A pair with no device: nothing has pulled, and every gauge says so.
#[test]
fn pair_with_no_device_is_silent_and_says_so() {
    let (mut native, _feed) = pair(SAMPLE_RATE);
    assert!(!native.alive(), "alive with no callback");
    assert!(native.routed(), "a pair holds its feed, so it is routed");
    assert_eq!(native.live_thread(), 0);
    assert!(native.stats().is_none(), "a report with no callback");
    assert_eq!(native.backlog(), 0);
    assert_eq!(native.ring_dropped(), 0);
    assert_eq!(native.stream_errors(), 0);
    assert_eq!(native.unrouted, 0);
    assert_eq!(native.out_rate, SAMPLE_RATE);
}

/// (2) The command ring refuses past its cap and counts — nothing else moves.
#[test]
fn sending_past_the_ring_counts_ring_dropped_and_nothing_else() {
    let (mut native, _feed) = pair(SAMPLE_RATE);
    for _ in 0..CMD_RING_CAP_NATIVE {
        assert!(
            native.send(Cmd::CutVoices),
            "the ring refused inside its cap"
        );
    }
    assert_eq!(native.backlog(), CMD_RING_CAP_NATIVE);
    for k in 1..=5 {
        assert!(
            !native.send(Cmd::CutVoices),
            "the ring took a {k}th past its cap"
        );
        assert_eq!(native.ring_dropped(), k, "a refused send was not counted");
    }
    assert_eq!(
        native.backlog(),
        CMD_RING_CAP_NATIVE,
        "a refused send took a slot"
    );
    assert!(!native.alive(), "alive with no audio thread");
    assert_eq!(native.live_thread(), 0);
    assert!(native.stats().is_none(), "a report with no audio thread");
}

/// (3) `fill` is the renderer, block for block, through every callback
/// length — and the gauges and the mailbox agree with the mirror after each.
#[test]
fn fill_is_the_renderer_block_for_block_through_any_callback_length() {
    let (mut native, mut feed, mut mirror) = loaded(SAMPLE_RATE);
    assert!(!native.alive(), "alive before the first fill");
    let mut x = 0xC0FF_EE11u32;
    let mut cmds = Vec::new();
    let mut out = vec![0.0f32; 2 * 1000];
    let mut loud = false;
    for k in 0..96 {
        cmds.clear();
        script(&mut x, k, SAMPLE_RATE, &mut cmds);
        for cmd in &cmds {
            assert!(native.send(*cmd));
            mirror.r.apply(*cmd);
        }
        let frames = LENGTHS[k % LENGTHS.len()];
        let want = mirror.expect_frames(frames);
        let got = &mut out[..2 * frames];
        feed.fill::<f32>(got, 2);
        same_bits(got, &want, &format!("callback {k} of {frames} frames"));
        // The two numbers the audio thread publishes without a ring.
        assert!(native.alive(), "callback {k}: not alive after a fill");
        assert_eq!(
            native.live_thread(),
            mirror.r.live() as u32,
            "callback {k}: the live count did not read back"
        );
        assert_eq!(
            feed.blocks(),
            mirror.r.blocks,
            "callback {k}: the seam rendered a different number of blocks"
        );
        // And the mailbox: a report after every callback, equal to the
        // mirror's counters.
        let stats = native.stats().expect("a report after every callback");
        assert_eq!(
            stats,
            mirror.r.stats(),
            "callback {k}: the report is not the mirror's"
        );
        loud |= got.iter().any(|s| s.abs() > 1e-4);
    }
    // The script was not silence: a beige smear passes a bit-equality test.
    assert!(
        loud,
        "every callback was silent - the script exercised nothing"
    );
    assert_eq!(
        native.ring_dropped(),
        0,
        "a command was dropped on a drained ring"
    );
}

/// (4) The alive bit flips on the first fill, however short.
#[test]
fn alive_flips_on_the_first_fill() {
    let (mut native, mut feed) = pair(SAMPLE_RATE);
    assert!(!native.alive());
    let mut one = [0.0f32; 2];
    feed.fill::<f32>(&mut one, 2);
    assert!(native.alive(), "one frame filled and not alive");
    assert_eq!(feed.blocks(), 1, "one frame costs one rendered block");
    let s = native.stats().expect("the first fill posts a report");
    assert_eq!(s.blocks, 1);
    assert_eq!(native.live_thread(), 0);
}

/// One script through a `T` device against the f32 mirror converted with
/// `cpal`'s own `Sample::from_sample`. At 48 kHz, so every held tap is
/// fractional and the renderer's device rate is exercised too.
fn converted_like_cpal<T>()
where
    T: SizedSample + FromSample<f32> + PartialEq + std::fmt::Debug,
{
    let (mut native, mut feed, mut mirror) = loaded(48_000);
    let mut x = 0xBEEF_1234u32;
    let mut cmds = Vec::new();
    let mut out = [T::EQUILIBRIUM; 2 * 512];
    let mut moved = false;
    for k in 0..24 {
        cmds.clear();
        script(&mut x, k, 48_000, &mut cmds);
        for cmd in &cmds {
            assert!(native.send(*cmd));
            mirror.r.apply(*cmd);
        }
        let frames = [441usize, 512][k % 2];
        let want = mirror.expect_frames(frames);
        let got = &mut out[..2 * frames];
        feed.fill::<T>(got, 2);
        for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
            assert_eq!(
                *g,
                T::from_sample(*w),
                "callback {k} sample {i}: the seam converted {w} differently from cpal"
            );
        }
        moved |= got.iter().any(|s| *s != T::EQUILIBRIUM);
    }
    assert!(moved, "the converted stream never left equilibrium");
    assert_eq!(feed.blocks(), mirror.r.blocks);
}

/// (5) An `i16` or `u16` device hears `cpal`'s conversion of the float.
#[test]
fn fill_i16_converts_as_cpal_sample_would() {
    converted_like_cpal::<i16>();
    converted_like_cpal::<u16>();
}

/// (6) One channel gets the mean; six get `l, r` and four zeros.
#[test]
fn a_mono_device_gets_the_mean_and_a_six_channel_device_gets_l_r_zeros() {
    // Mono.
    let (mut native, mut feed, mut mirror) = loaded(SAMPLE_RATE);
    let mut x = 0x0DDB_A11Eu32;
    let mut cmds = Vec::new();
    let mut out = vec![7.0f32; 6 * 441];
    let mut sided = false;
    for k in 0..12 {
        cmds.clear();
        script(&mut x, k, SAMPLE_RATE, &mut cmds);
        for cmd in &cmds {
            assert!(native.send(*cmd));
            mirror.r.apply(*cmd);
        }
        let want = mirror.expect_frames(441);
        let got = &mut out[..441];
        feed.fill::<f32>(got, 1);
        for (i, fr) in want.chunks_exact(2).enumerate() {
            let (l, r) = (fr[0], fr[1]);
            sided |= l.to_bits() != r.to_bits();
            assert_eq!(
                got[i].to_bits(),
                ((l + r) * 0.5).to_bits(),
                "callback {k} frame {i}: mono is not the mean of ({l}, {r})"
            );
        }
    }
    assert!(
        sided,
        "the script never put a sample on one side - the mean was never tested"
    );

    // Six channels, into a buffer pre-filled with a sentinel so an untouched
    // slot is caught as well as a wrong one.
    let (mut native, mut feed, mut mirror) = loaded(SAMPLE_RATE);
    let mut x = 0x0DDB_A11Eu32;
    for k in 0..12 {
        cmds.clear();
        script(&mut x, k, SAMPLE_RATE, &mut cmds);
        for cmd in &cmds {
            assert!(native.send(*cmd));
            mirror.r.apply(*cmd);
        }
        let want = mirror.expect_frames(441);
        for s in out.iter_mut() {
            *s = 7.0;
        }
        feed.fill::<f32>(&mut out[..6 * 441], 6);
        for (i, fr) in want.chunks_exact(2).enumerate() {
            let six = &out[6 * i..6 * i + 6];
            assert_eq!(
                six[0].to_bits(),
                fr[0].to_bits(),
                "callback {k} frame {i}: left"
            );
            assert_eq!(
                six[1].to_bits(),
                fr[1].to_bits(),
                "callback {k} frame {i}: right"
            );
            for (c, s) in six[2..].iter().enumerate() {
                assert_eq!(
                    s.to_bits(),
                    0.0f32.to_bits(),
                    "callback {k} frame {i}: channel {} is {s}, not silence",
                    c + 2
                );
            }
        }
    }
}

/// (7) A block straddling two 441-frame callbacks is carried, not
/// re-rendered: the concatenation equals the mirror's, and the block counts
/// are the arithmetic's — 4, then 7, then 8.
#[test]
fn the_carry_survives_a_441_frame_callback() {
    let (mut native, mut feed, mut mirror) = loaded(SAMPLE_RATE);
    let start = Cmd::Start {
        cue: Cue::Gather,
        gain_l: 0.8,
        gain_r: 0.3,
        rate: 1.0,
    };
    let bed = Cmd::Loop {
        slot: 1,
        cue: Cue::BedSurf,
        gain: 0.6,
    };
    for cmd in [start, bed] {
        assert!(native.send(cmd));
        mirror.r.apply(cmd);
    }
    let mut got = Vec::new();
    let mut want = Vec::new();
    // 441 = 3 blocks + 57 → 4 rendered, 71 carried; 441 + 71 → 3 more, 14
    // carried; 128 → 1 more, 14 carried.
    for (k, (frames, blocks)) in [(441usize, 4u64), (441, 7), (128, 8)].iter().enumerate() {
        let mut buf = vec![0.0f32; 2 * frames];
        want.extend(mirror.expect_frames(*frames));
        feed.fill::<f32>(&mut buf, 2);
        assert_eq!(
            feed.blocks(),
            *blocks,
            "callback {k} of {frames} frames: the seam has rendered {} blocks, the arithmetic \
             says {blocks} — the carry is not being kept",
            feed.blocks()
        );
        got.extend(buf);
    }
    assert_eq!(mirror.r.blocks, 8, "the mirror's own arithmetic drifted");
    same_bits(&got, &want, "three callbacks concatenated");
    assert!(
        got.iter().any(|s| s.abs() > 1e-4),
        "the three callbacks were silent"
    );
}

/// (8a) The install ring refuses past its cap and counts.
#[test]
fn an_install_past_the_ring_is_refused_and_counted() {
    let (mut native, _feed) = pair(SAMPLE_RATE);
    // Nothing drains: every install sits in the ring, whose cap is one per
    // cue — the shipped path installs each exactly once, so the count can
    // only rise on a bug, and it is counted rather than assumed away.
    for i in 0..CUE_COUNT {
        assert!(
            native.install(Cue::ALL[i], vec![0i16; 4].into_boxed_slice()),
            "install {i} refused inside the cap"
        );
    }
    assert_eq!(native.install_refused, 0);
    assert!(!native.install(Cue::ALL[0], vec![0i16; 4].into_boxed_slice()));
    assert_eq!(native.install_refused, 1);
}

/// (8b) A cue installed twice: the renderer refuses the second, counts it,
/// and the box comes back over the return ring to be dropped HERE — the
/// callback frees nothing.
#[test]
fn a_reinstall_comes_back_to_the_game_thread() {
    let (mut native, mut feed) = pair(SAMPLE_RATE);
    assert!(native.install(Cue::Bird, synth::pcm(Cue::Bird)));
    assert!(native.install(Cue::Bird, synth::pcm(Cue::Bird)));
    assert_eq!(native.reclaim(), 0, "nothing comes back before a callback");
    let mut out = [0.0f32; 2 * BLOCK];
    feed.fill::<f32>(&mut out, 2);
    assert_eq!(
        native.reclaim(),
        1,
        "the refused install must come back to the game thread"
    );
    assert_eq!(native.reclaim(), 0, "a box is handed back once");
    let s = native.stats().expect("a report");
    assert_eq!(
        s.installed, 1,
        "the second install must not replace the first"
    );
    assert_eq!(s.reinstall_refused, 1, "the refusal is counted");
    assert_eq!(
        s.return_dropped, 0,
        "the return ring had room, so nothing was freed on the audio thread"
    );
}

/// (9) A command sent between two callbacks lands before the NEXT rendered
/// block and never inside the carried one: after a 441-frame callback the
/// 71 carried frames are the old block's bits, and the block after them is
/// the loop's.
#[test]
fn commands_sent_between_callbacks_land_before_the_next_rendered_block_not_inside_the_carry() {
    let (mut native, mut feed, mut mirror) = loaded(SAMPLE_RATE);
    // Something sounding, so the carry is not trivially silence.
    let start = Cmd::Start {
        cue: Cue::Gather,
        gain_l: 0.7,
        gain_r: 0.4,
        rate: 1.0,
    };
    assert!(native.send(start));
    mirror.r.apply(start);
    let mut first = vec![0.0f32; 2 * 441];
    let want = mirror.expect_frames(441);
    feed.fill::<f32>(&mut first, 2);
    same_bits(&first, &want, "the first callback");
    assert_eq!(feed.blocks(), 4);
    assert_eq!(mirror.carried(), 71);

    // Sent BETWEEN callbacks: the seam has not seen it yet.
    let bed = Cmd::Loop {
        slot: 0,
        cue: Cue::BedWind,
        gain: 1.0,
    };
    assert!(native.send(bed));
    // The mirror: the carry first (rendered before the loop existed),
    // then the loop, then one fresh block with it.
    let carried = mirror.expect_frames(71);
    assert_eq!(mirror.r.blocks, 4, "the carry must cost no render");
    mirror.r.apply(bed);
    let fresh = mirror.expect_frames(BLOCK);
    assert_eq!(mirror.r.blocks, 5);

    let mut second = vec![0.0f32; 2 * (71 + BLOCK)];
    feed.fill::<f32>(&mut second, 2);
    assert_eq!(feed.blocks(), 5, "71 carried + one block is one render");
    same_bits(&second[..2 * 71], &carried, "the carried frames");
    same_bits(&second[2 * 71..], &fresh, "the block after the carry");
    assert_eq!(native.live_thread(), mirror.r.live() as u32);

    // And the loop is genuinely IN that block: a renderer never sent it
    // renders block 5 differently.
    let mut alone = Renderer::new(SAMPLE_RATE);
    for cue in CUES {
        assert!(alone.install(cue, synth::pcm(cue)).is_ok());
    }
    alone.apply(start);
    let mut blk = [0.0f32; 2 * BLOCK];
    for _ in 0..5 {
        alone.render(&mut blk);
    }
    assert!(
        fresh
            .iter()
            .zip(blk.iter())
            .any(|(a, b)| a.to_bits() != b.to_bits()),
        "the block after the carry is the same with and without the loop - the command did \
         not land where the contract says"
    );
    // The bed's own samples fade in from zero over half a millisecond
    // (`synth::edges`), so the proof of life is inside this block.
    assert!(
        fresh.iter().any(|s| s.abs() > 1e-4),
        "the loop's first block is silent"
    );
}
