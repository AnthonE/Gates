//! Gate: the native score stream decodes a shipped track, hands it to the
//! device callback's tap at the level asked for, and a stop silences it.
//!
//! No device: the tap is driven by hand the way `audio_out::Feed::fill`
//! drives it, one block at a time, while the real decoder thread fills the
//! ring from `assets/music/`.

#![cfg(all(feature = "render", not(target_arch = "wasm32")))]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use client::render::music_out::pair;
use client::sound::engine::BLOCK;
use client::sound::music::Track;

fn assets() -> Option<PathBuf> {
    Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets"))
}

/// Mix blocks until one carries sound or `secs` of wall time pass.
fn first_loud_block(
    tap: &mut client::render::music_out::Tap,
    secs: f32,
) -> Option<[f32; 2 * BLOCK]> {
    let t0 = Instant::now();
    while t0.elapsed().as_secs_f32() < secs {
        let mut buf = [0.0f32; 2 * BLOCK];
        tap.mix(&mut buf);
        if buf.iter().any(|v| v.abs() > 1e-3) {
            return Some(buf);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    None
}

#[test]
fn a_track_streams_into_the_tap_and_a_stop_silences_it() {
    let (feeder, mut tap) = pair(48_000);
    let mut stream = feeder.spawn_at(assets()).expect("a decoder thread");
    stream.set_gain(1.0);
    stream.play(Track::SafeZone);
    let buf = first_loud_block(&mut tap, 20.0).expect("Safe Zone never reached the tap");
    assert!(
        buf.iter().all(|v| v.is_finite() && v.abs() <= 1.0),
        "the stream is out of range: {buf:?}"
    );
    assert!(
        buf.chunks_exact(2).any(|f| f[0] != f[1]),
        "the score arrived in mono"
    );

    stream.stop();
    // The tap carries the stop out on its next block and acknowledges it;
    // nothing after that may sound.
    std::thread::sleep(Duration::from_millis(50));
    let mut buf = [0.0f32; 2 * BLOCK];
    tap.mix(&mut buf);
    std::thread::sleep(Duration::from_millis(400));
    for _ in 0..50 {
        let mut buf = [0.0f32; 2 * BLOCK];
        tap.mix(&mut buf);
        assert!(buf.iter().all(|v| *v == 0.0), "music after a stop");
    }
}

/// A dry ring adds nothing at all, not even `+0.0`: a client with no music
/// renders exactly what the engine alone renders.
#[test]
fn a_dry_tap_leaves_the_block_untouched() {
    let (_feeder, mut tap) = pair(44_100);
    let mut buf = [-0.0f32; 2 * BLOCK];
    tap.mix(&mut buf);
    assert!(buf.iter().all(|v| v.to_bits() == (-0.0f32).to_bits()));
}

/// A track asked for with no asset root is counted, not a panic.
#[test]
fn no_asset_root_is_counted() {
    let (feeder, _tap) = pair(44_100);
    let mut stream = feeder.spawn_at(None).expect("a decoder thread");
    stream.play(Track::GreenVeil);
    assert_eq!(stream.unrooted, 1);
}
