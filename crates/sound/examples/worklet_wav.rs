//! Render a scene through the BROWSER's renderer and write it as a stereo WAV.
//!
//!   cargo run -p sound --example worklet_wav -- <path.wav> [seconds]
//!
//! **Why this exists.** No gate in this repo starts a browser and this box has
//! no sound card, so "the browser makes sound" has been a claim about
//! arithmetic rather than something anybody heard. `sound::worklet::Worklet`
//! is exactly what runs inside the `AudioWorkletProcessor` — and
//! `ci/check_worklet.mjs` proves the shipped wasm module renders it sample for
//! sample — so a WAV written from it here **is** what a tab plays, decoded on
//! a machine that has speakers instead of in one that does not.
//!
//! It is a listening aid and not a gate: nothing here asserts, because what it
//! is for is the judgement a person makes with their ears, which is the same
//! division `CLAUDE.md` draws for the frame.

use std::io::Write;

use sound::engine::{rate, Cmd, BLOCK};
use sound::worklet::Worklet;
use sound::{synth, Cue, SAMPLE_RATE};

/// 48 kHz: what most devices run, and the rate at which `engine::rate` is
/// actually resampling the 44.1 kHz bank — so this hears the resampler too.
const OUT_RATE: u32 = 48_000;

/// A one-shot at two ear gains and a speed, built the way the mixer builds one.
fn start(cue: Cue, l: f32, r: f32, speed: f32) -> Cmd {
    Cmd::Start {
        cue,
        gain_l: l,
        gain_r: r,
        rate: rate(speed, OUT_RATE),
    }
}

/// Interleaved f32 in `[-1, 1]` to a 16-bit stereo WAV. `synth::to_wav16` is
/// mono and private; this is its stereo twin, 44 bytes of header and the
/// samples.
fn wav_stereo(lr: &[f32], rate_hz: u32) -> Vec<u8> {
    let bytes = (lr.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + bytes as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + bytes).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&2u16.to_le_bytes()); // stereo
    w.extend_from_slice(&rate_hz.to_le_bytes());
    w.extend_from_slice(&(rate_hz * 4).to_le_bytes()); // byte rate
    w.extend_from_slice(&4u16.to_le_bytes()); // block align
    w.extend_from_slice(&16u16.to_le_bytes()); // bits
    w.extend_from_slice(b"data");
    w.extend_from_slice(&bytes.to_le_bytes());
    for s in lr {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        w.extend_from_slice(&v.to_le_bytes());
    }
    w
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "worklet.wav".into());
    let secs: f32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(12.0);

    let mut w = Worklet::new(OUT_RATE);
    for (i, pcm) in synth::pcm_bank().into_iter().enumerate() {
        assert!(w.install(i, &pcm), "cue {i} did not install");
    }

    // ── the scene ───────────────────────────────────────────────────────────
    // A walk across surfaces with the wind under it, someone else moving on
    // the right, a gather, a swing into wood, and a piece of music over the
    // top: enough of the bank in one take to tell whether the pan, the pitch
    // and the mix are right, rather than whether a sound exists.
    let mut script: Vec<(f32, Cmd)> = vec![
        // The bed, from the first block, quietly.
        (
            0.0,
            Cmd::Loop {
                slot: 0,
                cue: Cue::BedWind,
                gain: 0.28,
            },
        ),
        // Music, on a held slot of its own.
        (
            0.4,
            Cmd::Play {
                slot: 3,
                cue: Cue::MusicOpenCalm,
                gain: 0.30,
            },
        ),
    ];
    // Our own footsteps, centred, every 0.48 s — grass, then litter, then rock.
    let surfaces = [
        Cue::StepGrass,
        Cue::StepGrass,
        Cue::StepLitter,
        Cue::StepLitter,
        Cue::StepRock,
        Cue::StepRock,
        Cue::StepWater,
        Cue::StepWater,
    ];
    for (i, cue) in surfaces.iter().enumerate() {
        let t = 0.9 + i as f32 * 0.48;
        // A little speed variation, as the mixer gives a step.
        let speed = 0.94 + (i % 3) as f32 * 0.05;
        script.push((t, start(*cue, 0.55, 0.55, speed)));
    }
    // Someone else, crossing from the right to the centre.
    for i in 0..5 {
        let t = 1.6 + i as f32 * 0.62;
        let pan = 1.0 - i as f32 * 0.18;
        script.push((
            t,
            start(
                Cue::RemoteStepGrass,
                0.30 * (1.0 - pan * 0.6),
                0.42 * pan,
                1.0,
            ),
        ));
    }
    // A gather, a swing, and the wood it lands in — slightly to the left.
    script.push((5.0, start(Cue::Gather, 0.6, 0.45, 1.0)));
    script.push((6.2, start(Cue::Swing, 0.62, 0.5, 1.0)));
    script.push((6.45, start(Cue::ImpactWood, 0.7, 0.42, 1.0)));
    script.push((7.1, start(Cue::Swing, 0.62, 0.5, 1.02)));
    script.push((7.35, start(Cue::ImpactWood, 0.7, 0.42, 0.98)));
    // A bird, far right. A boar, far left, answering.
    script.push((3.2, start(Cue::Bird, 0.10, 0.34, 1.0)));
    script.push((8.4, start(Cue::Snort, 0.40, 0.11, 1.0)));
    script.push((9.2, start(Cue::Growl, 0.44, 0.14, 1.0)));
    // A shot, and a hit on it.
    script.push((10.2, start(Cue::ShotBow, 0.6, 0.6, 1.0)));
    script.push((10.55, start(Cue::HitLimb, 0.5, 0.5, 1.0)));
    // The bed fades out at the end so the tail is audible.
    script.push((
        secs - 1.6,
        Cmd::Gain {
            slot: 0,
            gain: 0.05,
        },
    ));
    script.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    // ── render ──────────────────────────────────────────────────────────────
    let blocks = ((secs * OUT_RATE as f32) / BLOCK as f32) as usize;
    let per_block = BLOCK as f32 / OUT_RATE as f32;
    let mut lr: Vec<f32> = Vec::with_capacity(blocks * 2 * BLOCK);
    let mut next = 0;
    let mut peak = 0.0f32;
    for b in 0..blocks {
        let t = b as f32 * per_block;
        while next < script.len() && script[next].0 <= t {
            // Through the real encoder and decoder, so this hears what the
            // page's bytes actually say, not what the command meant.
            let mut rec = [0u8; 16];
            let n = sound::worklet::encode([script[next].1].into_iter(), &mut rec);
            assert_eq!(n, 16);
            assert_eq!(w.push_bytes(&rec), 1);
            next += 1;
        }
        w.render();
        let planar = w.planar();
        for i in 0..BLOCK {
            let (l, r) = (planar[i], planar[BLOCK + i]);
            peak = peak.max(l.abs()).max(r.abs());
            lr.push(l);
            lr.push(r);
        }
    }

    let bytes = wav_stereo(&lr, OUT_RATE);
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&bytes))
        .unwrap_or_else(|e| panic!("cannot write {path}: {e}"));
    let rep = w.report();
    println!(
        "{path}: {secs:.1}s stereo {OUT_RATE} Hz, peak {peak:.3}, {} events, bank at {SAMPLE_RATE} Hz",
        script.len()
    );
    println!(
        "  renderer: {} blocks, {} cues installed, refused {}, unbanked {}, dropped {}, bad {}",
        rep.stats.blocks,
        rep.stats.installed,
        rep.stats.refused,
        rep.stats.unbanked,
        rep.stats.dropped,
        rep.stats.bad_cmd + rep.bad_records + rep.bad_installs
    );
}
