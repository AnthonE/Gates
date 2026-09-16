//! Write the fixture `ci/check_worklet.mjs` replays through the real browser
//! module: the cues to install, the command bytes to post, and the samples a
//! native `Renderer` produces for them.
//!
//! The point is that **every byte in it is written by the same code the
//! client uses** — `worklet::encode` for the commands, `Renderer` for the
//! expected output — so the harness compares the browser's wasm module
//! against this repo's own arithmetic rather than against numbers somebody
//! typed into a JS file. A hand-written fixture would prove the harness
//! agrees with itself.
//!
//!   cargo run -p sound --example worklet_fixture -- <path>

use std::io::Write;

use sound::engine::{rate, Cmd, Renderer, BLOCK};
use sound::worklet::encode;
use sound::{synth, Cue};

/// The context rate the fixture is built for — 48 kHz, what most devices run
/// and where `engine::rate`'s resample is doing work.
const OUT_RATE: u32 = 48_000;
/// Quanta rendered. Enough to cover a one-shot's attack and a bed's ramp.
const QUANTA: usize = 8;
/// The cues the script plays, and the quantum each is installed at.
///
/// **Spread, because that is the path that actually runs.** `SPREAD_BANK` is
/// true on wasm32 (`render/audio.rs`), so a tab installs one cue per FRAME
/// while the processor is already rendering — and a `Renderer::install`
/// allocates, which grows wasm memory, which detaches every `Float32Array`
/// view over it. A fixture that installed everything up front would build the
/// processor's view after the last growth and never exercise the rebuild;
/// measured 2026-09-16, that version of this file passed with the rebuild
/// deleted. The second cue lands mid-render on purpose.
const USED: [(Cue, usize); 2] = [(Cue::StepRock, 0), (Cue::BedSurf, 3)];

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: worklet_fixture <path>");
    let mut out: Vec<u8> = Vec::new();

    out.extend_from_slice(&OUT_RATE.to_le_bytes());

    let mut r = Renderer::new(OUT_RATE);
    out.extend_from_slice(&(USED.len() as u32).to_le_bytes());
    for (cue, at) in USED {
        let pcm = synth::pcm(cue);
        out.extend_from_slice(&(cue.idx() as u32).to_le_bytes());
        out.extend_from_slice(&(at as u32).to_le_bytes());
        out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        for s in pcm.iter() {
            out.extend_from_slice(&s.to_le_bytes());
        }
    }

    // One one-shot hard left, a bed under it: a script whose output differs
    // per ear, so a harness that crossed them would not pass. The bed is
    // asked for at quantum 0 and its cue does not land until 3, so this also
    // drives the renderer's remembered ask — a `Loop` on an uninstalled cue
    // starts from sample 0 the block the bank arrives.
    let cmds = [
        Cmd::Start {
            cue: Cue::StepRock,
            gain_l: 0.9,
            gain_r: 0.2,
            rate: rate(1.0, OUT_RATE),
        },
        Cmd::Loop {
            slot: 0,
            cue: Cue::BedSurf,
            gain: 0.5,
        },
    ];
    let mut bytes = vec![0u8; cmds.len() * 16];
    let n = encode(cmds.iter().copied(), &mut bytes);
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend_from_slice(&bytes[..n]);
    for c in cmds {
        r.push(c);
    }

    out.extend_from_slice(&(QUANTA as u32).to_le_bytes());
    let mut lr = [0.0f32; 2 * BLOCK];
    let mut peak = 0.0f32;
    for q in 0..QUANTA {
        // The install lands BEFORE the quantum it is keyed to, which is the
        // order the harness delivers it in: a `postMessage` is handled before
        // the next `process()` call, never inside one.
        for (cue, at) in USED {
            if at == q {
                r.install(cue, synth::pcm(cue)).expect("install");
            }
        }
        r.render(&mut lr);
        // Planar, as the processor hands it to `process()`.
        for i in 0..BLOCK {
            out.extend_from_slice(&lr[2 * i].to_le_bytes());
        }
        for i in 0..BLOCK {
            out.extend_from_slice(&lr[2 * i + 1].to_le_bytes());
        }
        for s in lr.iter() {
            peak = peak.max(s.abs());
        }
    }
    assert!(
        peak > 0.05,
        "the fixture renders a peak of {peak} — a comparison of two silences \
         proves nothing (CLAUDE.md, the beige-smear entry)"
    );

    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&out))
        .expect("write fixture");
    eprintln!(
        "{path}: {} bytes, {QUANTA} quanta at {OUT_RATE} Hz, peak {peak:.4}",
        out.len()
    );
}
