//! The browser seam's gate — code tier: no browser, no wasm, no Bevy.
//!
//! `crates/sound-worklet` is the module a tab loads and it is nothing but
//! `#[wasm_bindgen]` wrappers, so every decision on the browser's audio path
//! lives in `sound::worklet` and is driven from here on an ordinary box.
//! That division is the same one `client-core/tests/wire.rs` makes for the
//! transport, and it is why the deleted `client-wasm`'s 1,635-line hand-
//! written ABI is not being rebuilt.
//!
//! The load-bearing test is `the_seam_renders_what_the_native_path_renders`:
//! one script of commands is driven twice at the same rate — once straight
//! into a `Renderer` the way `audio_out.rs`'s ring does, once through
//! `encode` → `postMessage`'s bytes → `push_bytes` — and every sample is
//! compared on `to_bits`. A browser that is handed the same script has to
//! hear what a desktop hears, and "it plays something" is not that claim.
//!
//! ## Mutants run by hand
//!
//! Each was applied to `worklet.rs`, the suite re-run, and the file restored.
//!
//! | mutant | reddened by |
//! |---|---|
//! | `planar[i]` / `planar[BLOCK + i]` swapped (ears crossed in the de-interleave) | `the_seam_renders_what_the_native_path_renders`, 44.1 kHz, block 0 sample 1, left ear: −0.08425598 against −0.08387146. **Sample 1, not 0** — `synth::edges` fades every cue's first sample to zero, so a crossed ear has nothing to disagree about there; the engine suite's own mutant table records the same surprise |
//! | `put32(.., 28, s.live)` → `put32(.., 28, s.installed)` (two fields of one type transposed) | `a_report_round_trips_field_by_field` |
//! | `encode` truncating its last record instead of stopping (`out[at..at + room]` with `room = min(out.len() − at, CMD_BYTES)`) | `encode_never_writes_half_a_command` — "two whole records, not two and a half". The one-token version of this mutant (widening the bound) does not compile into a defect, it panics in `copy_from_slice`; the truncating rewrite is the shape someone would reach for to *silence* that panic, which is the defect worth gating |
//! | `bytes.chunks_exact` → `bytes.chunks` (a trailing partial decoded) | `push_bytes_refuses_a_malformed_record_and_counts_it`, as a **panic** rather than an assertion: `copy_from_slice: source slice length (3) does not match destination slice length (16)`. Recorded as a panic on purpose — on the audio thread that is the processor gone for the session, which is why the decoder counts a short tail instead of reaching it |

use sound::engine::{rate, Cmd, Renderer, Stats, BLOCK, CMD_BYTES, CMD_RING_CAP};
use sound::worklet::{encode, Report, Worklet, REPORT_BYTES};
use sound::{synth, Cue, CUE_COUNT};

/// The cues this file plays, installed into both sides of every comparison.
const USED: [Cue; 4] = [Cue::StepRock, Cue::ImpactWood, Cue::BedSurf, Cue::Gather];

/// A one-shot at two ear gains and a speed, built the way the mixer builds
/// one — `rate` against the device rate, so the command is in band.
fn start(cue: Cue, l: f32, r: f32, speed: f32, out_rate: u32) -> Cmd {
    Cmd::Start {
        cue,
        take: 0,
        takes: 1,
        lp: 0,
        gain_l: l,
        gain_r: r,
        rate: rate(speed, out_rate),
    }
}

/// The script both sides are driven with: overlapping one-shots at three
/// speeds, a bed that loops under them, a gain ramp on it and a stop, and a
/// `CutVoices` at the end. Keyed by the block the command is pushed before.
fn script(out_rate: u32) -> Vec<(usize, Cmd)> {
    vec![
        (0, start(Cue::StepRock, 0.3, 0.9, 1.0, out_rate)),
        (
            0,
            Cmd::Loop {
                slot: 0,
                cue: Cue::BedSurf,
                gain: 0.4,
            },
        ),
        (1, start(Cue::ImpactWood, 0.9, 0.3, 1.37, out_rate)),
        (2, start(Cue::Gather, 0.6, 0.6, 0.83, out_rate)),
        (3, Cmd::Gain { slot: 0, gain: 0.9 }),
        (5, start(Cue::StepRock, 1.0, 0.0, 1.0, out_rate)),
        (7, Cmd::Stop { slot: 0 }),
        (9, Cmd::CutVoices),
    ]
}

/// **The claim: a browser hears what a desktop hears.**
///
/// The native side is a `Renderer` fed by `push`, which is what
/// `audio_out.rs`'s ring does on the other side of `In::drain_into`. The
/// browser side is a `Worklet` fed the same commands as the bytes a
/// `postMessage` carries. Compared on `to_bits` at both rates that matter:
/// the bank's own 44.1 kHz, and the 48 kHz most devices actually run — where
/// `engine::rate`'s resample is doing work and an ear swap or an off-by-one
/// in the de-interleave has somewhere to hide.
#[test]
fn the_seam_renders_what_the_native_path_renders() {
    for out_rate in [44_100, 48_000] {
        let mut native = Renderer::new(out_rate);
        let mut web = Worklet::new(out_rate);
        for cue in USED {
            let pcm = synth::pcm(cue);
            assert!(native.install(cue, pcm.clone()).is_ok());
            assert!(web.install(cue.idx(), &pcm), "install {cue:?}");
        }
        assert_eq!(web.out_rate(), out_rate);

        let script = script(out_rate);
        let blocks = 12;
        let mut lr = [0.0f32; 2 * BLOCK];
        // Sized as the page sizes it: a frame's worth of records.
        let mut bytes = [0u8; 16 * CMD_BYTES];

        for block in 0..blocks {
            let due: Vec<Cmd> = script
                .iter()
                .filter(|(b, _)| *b == block)
                .map(|(_, c)| *c)
                .collect();
            for cmd in &due {
                native.push(*cmd);
            }
            let n = encode(due.iter().copied(), &mut bytes);
            assert_eq!(n, due.len() * CMD_BYTES, "every due command encoded");
            assert_eq!(web.push_bytes(&bytes[..n]), due.len(), "and decoded");

            native.render(&mut lr);
            web.render();
            let planar = web.planar();
            for i in 0..BLOCK {
                assert_eq!(
                    lr[2 * i].to_bits(),
                    planar[i].to_bits(),
                    "{out_rate} Hz, block {block} sample {i}, LEFT: the worklet's \
                     {} against the native path's {}",
                    planar[i],
                    lr[2 * i]
                );
                assert_eq!(
                    lr[2 * i + 1].to_bits(),
                    planar[BLOCK + i].to_bits(),
                    "{out_rate} Hz, block {block} sample {i}, RIGHT: the worklet's \
                     {} against the native path's {}",
                    planar[BLOCK + i],
                    lr[2 * i + 1]
                );
            }
        }

        // The script is audible: a comparison of two silences would pass
        // every assertion above and prove nothing (`CLAUDE.md`'s beige-smear
        // entry, one tier down).
        let r = web.report();
        assert!(r.stats.blocks == blocks as u64, "blocks rendered");
        assert_eq!(
            r.bad_records, 0,
            "nothing malformed in a well-formed script"
        );
        assert_eq!(r.bad_installs, 0);
        assert_eq!(r.stats.installed, USED.len() as u32);
    }
}

/// A silent script would satisfy the comparison above, so this is the test
/// that the fixture makes sound at all — the same shape as the loot-storm
/// entry's "ground truth comes off the store".
#[test]
fn the_compared_script_is_not_silence() {
    let out_rate = 48_000;
    let mut web = Worklet::new(out_rate);
    for cue in USED {
        assert!(web.install(cue.idx(), &synth::pcm(cue)));
    }
    let mut bytes = [0u8; 16 * CMD_BYTES];
    let mut loudest = 0.0f32;
    let script = script(out_rate);
    for block in 0..12 {
        let due: Vec<Cmd> = script
            .iter()
            .filter(|(b, _)| *b == block)
            .map(|(_, c)| *c)
            .collect();
        let n = encode(due.iter().copied(), &mut bytes);
        web.push_bytes(&bytes[..n]);
        web.render();
        for s in web.planar().iter() {
            loudest = loudest.max(s.abs());
        }
    }
    assert!(
        loudest > 0.05,
        "the script rendered a peak of {loudest}, which is silence dressed as a comparison"
    );
}

/// Every field of a report survives the crossing, and no two of them are
/// transposed. Distinct values throughout, for the byte-golden reason: a
/// record of zeros round-trips under any permutation.
#[test]
fn a_report_round_trips_field_by_field() {
    let want = Report {
        stats: Stats {
            refused: 11,
            unbanked: 22,
            cut: 33,
            dropped: 44,
            bad_cmd: 55,
            reinstall_refused: 66,
            return_dropped: 77,
            blocks: 0x0102_0304_0506_0708,
            live: 88,
            installed: 99,
        },
        bad_records: 111,
        bad_installs: 122,
    };
    let bytes = want.to_bytes();
    assert_eq!(bytes.len(), REPORT_BYTES);
    assert_eq!(Report::from_bytes(&bytes), want);
}

/// `encode` fills what fits and stops — it never writes a partial record,
/// because half a command is not a smaller command.
#[test]
fn encode_never_writes_half_a_command() {
    let cmds = [
        Cmd::Stop { slot: 0 },
        Cmd::Stop { slot: 1 },
        Cmd::Stop { slot: 2 },
    ];
    // Room for two and a half.
    let mut out = [0xAAu8; CMD_BYTES * 2 + 8];
    let n = encode(cmds.iter().copied(), &mut out);
    assert_eq!(n, CMD_BYTES * 2, "two whole records, not two and a half");
    assert!(
        out[CMD_BYTES * 2..].iter().all(|b| *b == 0xAA),
        "the tail past the last whole record is untouched"
    );
    // And what did fit decodes back to the first two commands.
    let mut w = Worklet::new(48_000);
    assert_eq!(w.push_bytes(&out[..n]), 2);
    assert_eq!(w.report().bad_records, 0);
}

/// A record with a tag that is no command, and a trailing partial record,
/// are each refused and counted rather than guessed at — they arrive from
/// JS, where nothing type-checks them.
#[test]
fn push_bytes_refuses_a_malformed_record_and_counts_it() {
    let mut w = Worklet::new(48_000);

    // A tag past the last variant.
    let mut bad = [0u8; CMD_BYTES];
    bad[0] = 200;
    assert_eq!(w.push_bytes(&bad), 0);
    assert_eq!(w.report().bad_records, 1);

    // A whole command followed by three stray bytes.
    let mut buf = [0u8; CMD_BYTES + 3];
    let n = encode([Cmd::CutVoices].into_iter(), &mut buf[..CMD_BYTES]);
    assert_eq!(n, CMD_BYTES);
    assert_eq!(w.push_bytes(&buf), 1, "the whole record still lands");
    assert_eq!(w.report().bad_records, 2, "and the tail is counted");

    // An empty batch is not an error.
    assert_eq!(w.push_bytes(&[]), 0);
    assert_eq!(w.report().bad_records, 2);
}

/// The renderer's queue is bounded and the seam does not grow it: past
/// `CMD_RING_CAP` in one batch the newest are dropped and counted, which is
/// `CLAUDE.md` wall 4 at this seam.
#[test]
fn a_batch_past_the_queue_cap_drops_the_newest_and_counts_it() {
    let mut w = Worklet::new(48_000);
    let over = CMD_RING_CAP + 9;
    let mut bytes = vec![0u8; over * CMD_BYTES];
    let n = encode(
        (0..over).map(|i| Cmd::Gain {
            slot: (i % 7) as u8,
            gain: 0.5,
        }),
        &mut bytes,
    );
    assert_eq!(n, over * CMD_BYTES);
    assert_eq!(w.push_bytes(&bytes[..n]), over, "every record decoded");
    w.render();
    assert_eq!(
        w.report().stats.dropped,
        9,
        "the nine past the cap were dropped by the renderer, not buffered here"
    );
}

/// A cue index from JS that is not a cue is refused and counted — never an
/// index into the bank, and never a panic: a panic on the audio thread takes
/// the processor down for the rest of the session.
#[test]
fn an_install_naming_no_cue_is_refused_and_counted() {
    let mut w = Worklet::new(48_000);
    assert!(!w.install(CUE_COUNT, &[0, 1, 2]));
    assert!(!w.install(usize::MAX, &[0, 1, 2]));
    assert_eq!(w.report().bad_installs, 2);
    assert_eq!(w.installed(), 0, "nothing landed in the bank");

    // And a real one still works beside them.
    assert!(w.install(Cue::Gather.idx(), &synth::pcm(Cue::Gather)));
    assert_eq!(w.installed(), 1);
}

/// A cue is installed once per boot, so a second install is refused and
/// counted rather than replacing a box the audio thread would then free.
#[test]
fn a_second_install_of_one_cue_is_refused_and_counted() {
    let mut w = Worklet::new(48_000);
    let pcm = synth::pcm(Cue::Gather);
    assert!(w.install(Cue::Gather.idx(), &pcm));
    assert!(!w.install(Cue::Gather.idx(), &pcm), "the second is refused");
    assert_eq!(w.report().stats.reinstall_refused, 1);
    assert_eq!(w.installed(), 1);
}

/// The whole bank fits the seam: every cue crosses by index and lands.
#[test]
fn every_cue_crosses_the_seam_by_index() {
    let mut w = Worklet::new(48_000);
    for (i, pcm) in synth::pcm_bank().into_iter().enumerate() {
        assert!(w.install(i, &pcm), "cue {i} ({:?})", Cue::ALL[i]);
    }
    assert_eq!(w.installed(), CUE_COUNT);
    assert_eq!(w.report().bad_installs, 0);
    assert_eq!(w.report().stats.reinstall_refused, 0);
}
