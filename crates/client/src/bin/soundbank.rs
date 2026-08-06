//! Write the generated sound bank to a directory, so somebody can listen to it.
//!
//! ```text
//! cargo run -p client --bin soundbank -- /tmp/bank
//! ```
//!
//! **This exists because the gate cannot hear.** `crates/client/tests/sound.rs`
//! asserts that every cue has energy, does not clip, does not click at either
//! end and loops without a seam — and none of that can tell whether the bank
//! sounds like a forest. `CLAUDE.md`'s beige-smear entry is the same lesson on
//! the visual side: a pixel statistic passed 36 checks on a frame with nothing
//! in it, and the fix was to look at the picture. This is how you look at the
//! picture.
//!
//! No `render` feature, no Bevy, no audio device: `crate::sound::synth` is pure
//! and this is 20 lines around it. It is also the fastest way to hear a change
//! to a cue — regenerate, play, decide — without launching a client, joining a
//! shard and walking to a tree.

use std::path::PathBuf;

use client::sound::{synth, Cue};

fn main() {
    let dir: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "bank".to_string())
        .into();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("soundbank: cannot create {}: {e}", dir.display());
        std::process::exit(1);
    }
    let mut total = 0usize;
    for cue in Cue::ALL {
        let wav = synth::wav(cue);
        // `{:?}` on the cue is its variant name, which is the only name a cue
        // has — there is deliberately no string table (`sound/mod.rs`).
        let path = dir.join(format!("{:02}_{:?}.wav", cue.idx(), cue));
        if let Err(e) = std::fs::write(&path, &wav) {
            eprintln!("soundbank: cannot write {}: {e}", path.display());
            std::process::exit(1);
        }
        let secs = (wav.len().saturating_sub(44) / 2) as f32 / client::sound::SAMPLE_RATE as f32;
        println!("{:>5.2}s  {}", secs, path.display());
        total += wav.len();
    }
    println!(
        "{} cues, {} kB, {} Hz mono 16-bit",
        Cue::ALL.len(),
        total / 1024,
        client::sound::SAMPLE_RATE
    );
}
