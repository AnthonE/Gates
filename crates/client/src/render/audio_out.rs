//! The native client's audio output: the engine inside cpal's device callback.
//!
//! `render/audio.rs` decides nothing and neither does this file — it is the
//! seam between the game thread's [`Engine`](super::audio::Engine) buffer and
//! the audio thread's `sound::engine::Renderer`, and it is exactly two
//! moves: [`flush`] copies the frame's commands into the native ring in
//! `PostUpdate`, and [`Feed::fill`] pulls them back out inside the device
//! callback, one block at a time. Everything between — the pool, the pan,
//! the ramps, the counters — is `crates/sound/src/engine.rs`, gated headless
//! in `crates/sound/tests/`.
//!
//! **The seam is a cpal output callback** — the same shape as the browser's
//! `AudioWorklet::process`, a function handed a buffer of some length at
//! some channel count and asked to fill it — owning one [`Feed`]. What a
//! callback costs: one `In::drain_into` (bounded), `⌈frames / BLOCK⌉`
//! renders through a one-block carry, one mailbox push, and nothing else —
//! **no allocation, no lock, no wait**: the rings are SPSC, the scratch is a
//! fixed array sized at construction, and the two gauges the callback
//! publishes without a ring are atomics. What it does not do: it has **no
//! resampler of its own** — the renderer is built at the DEVICE rate, so
//! `engine::rate` is the only resample in the chain and it never restarts;
//! no volume (the sliders reach the renderer as gains on each command); no
//! channel layout beyond mono (the mean) and stereo-plus-silence.
//!
//! **Why `bevy_audio`'s sink was removed.** The first cut of this seam was a
//! rodio `Source` under one `AudioPlayer`, which read as free: Bevy opens the
//! device, rodio pulls, we render. It was not. rodio's
//! `SourcesQueueOutput::current_frame_len` answers `Some(512)` for a source
//! whose own answer is `None` (`queue.rs`, `THRESHOLD`), so its
//! `UniformSourceIterator` re-bootstrapped a `SampleRateConverter` — a
//! `Vec::with_capacity` and two `collect`s on any device not at 44.1 kHz —
//! every 512 samples, ~172 times a second, **inside the device callback**,
//! with the interpolation phase restarting at every boundary. An allocation
//! on the audio thread is the disease the engine exists to cure, and a
//! resampler that restarts every 11.6 ms is an audible one on every 48 kHz
//! device, which is most of them. So `bin/gates.rs` and `bin/modelview.rs`
//! no longer add `AudioPlugin`, and this file opens the device itself.
//!
//! **A box with no device** — every `--capture` run, CI — is the case this
//! file is written for first: `open` finds no device (or no config, or no
//! stream), says `audio output: none` ONCE, and drops the [`Feed`]. From
//! then on `flush` does not fill a ring nobody drains: it drops every
//! command and install at the flush and counts them
//! ([`Native::unrouted`]), [`Native::alive`] stays false for the life of the
//! run, and `audio::pump` gates its warnings on that bit. cpal on ALSA can
//! also hand back a `default` device that fails only at `build` or `play`,
//! which is why every step falls through to the same unrouted state rather
//! than any one of them being trusted.
//!
//! **`Feed::fill` is free of cpal's types except the two sample traits**, so
//! `tests/engine_out.rs` drives it with no device and no `App` — through
//! 441-frame callbacks, mono and six-channel maps, and `i16`/`u16`
//! conversion — and `tests/engine_out_alloc.rs` holds it to zero
//! allocations under a counting allocator.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use bevy::prelude::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

use crate::sound::engine::{self, Cmd, In, Renderer, Stats, BLOCK};
use crate::sound::{Cue, SAMPLE_RATE};

use super::audio;

/// The audio thread's half of the seam: the renderer, its input rings, the
/// one-block scratch and its carry, and the two gauges it publishes back
/// without a ring.
pub struct Feed {
    renderer: Box<Renderer>,
    input: In,
    /// Interleaved stereo scratch, one block. A callback is any length —
    /// 441 frames is common — and a block is [`BLOCK`], so the block that
    /// straddles two callbacks is carried here between them.
    buf: [f32; 2 * BLOCK],
    /// The carry position into `buf`, in samples; `2 · BLOCK` means dry, so
    /// the next frame asked for renders a block first.
    at: usize,
    /// One-shots live at the last block — what [`Native::live_thread`] reads.
    live: Arc<AtomicU32>,
    /// Set the first time a callback fills: the device exists and is asking.
    alive: Arc<AtomicBool>,
}

impl Feed {
    /// One device callback: drain the rings ONCE (bounded by
    /// `In::drain_into`), then fill `out` — interleaved, `channels` samples
    /// per frame — from block-sized renders through the carry, then post
    /// the report and the gauges.
    ///
    /// Channel map: one channel gets `(l + r) · 0.5`; two or more get `l`,
    /// `r`, then zeros. Every sample is converted with `T::from_sample`, so
    /// an `i16` or `u16` device hears exactly what `cpal` would have made of
    /// the same float.
    ///
    /// **The order is the contract `tests/engine_out.rs` rebuilds**: drain
    /// before render, so a command sent between two callbacks lands before
    /// the NEXT rendered block — never inside a carried one, which was
    /// rendered before the command existed.
    ///
    /// `chunks_exact_mut` leaves a remainder only if `out.len()` is not a
    /// multiple of `channels`, which a device never hands over; it is left
    /// untouched rather than guessed at.
    pub fn fill<T: SizedSample + FromSample<f32>>(&mut self, out: &mut [T], channels: u16) {
        self.input.drain_into(&mut self.renderer);
        let ch = (channels as usize).max(1);
        for frame in out.chunks_exact_mut(ch) {
            if self.at >= 2 * BLOCK {
                self.renderer.render(&mut self.buf);
                self.at = 0;
            }
            let (l, r) = (self.buf[self.at], self.buf[self.at + 1]);
            self.at += 2;
            if ch == 1 {
                frame[0] = T::from_sample((l + r) * 0.5);
            } else {
                frame[0] = T::from_sample(l);
                frame[1] = T::from_sample(r);
                for s in &mut frame[2..] {
                    *s = T::from_sample(0.0f32);
                }
            }
        }
        // A report the game thread has not read yet stays; this one is
        // skipped and the next callback after a read posts again — nothing
        // is lost, every counter is cumulative (`engine::channel`).
        self.input.report(&self.renderer);
        self.live
            .store(self.renderer.live() as u32, Ordering::Relaxed);
        self.alive.store(true, Ordering::Relaxed);
    }

    /// Blocks the renderer has rendered — the test's block counter, so it
    /// can know how many frames the carry holds.
    pub fn blocks(&self) -> u64 {
        self.renderer.blocks
    }
}

/// The game thread's end of the native seam.
///
/// A **non-send** resource, and the type system is the reason twice over:
/// `rtrb`'s producers and consumer are `Send` and not `Sync` (each caches its
/// counterpart's index in a `Cell`), and `cpal::Stream` is neither `Send` nor
/// `Sync` on any platform — while a Bevy `Resource` has to be both. `NonSend`
/// pins [`flush`] to the main thread, which is where the game thread already
/// is, and costs nothing else — `Net` is held the same way.
pub struct Native {
    out: engine::Out,
    live: Arc<AtomicU32>,
    alive: Arc<AtomicBool>,
    /// What the stream's error callback counted: a device that went away, a
    /// backend fault. Read by [`Native::stream_errors`].
    errors: Arc<AtomicU32>,
    /// The device stream, kept alive here — a dropped `cpal::Stream` is a
    /// stopped one. Never read; its lifetime is its job.
    _stream: Option<cpal::Stream>,
    /// Is a [`Feed`] alive to drain the rings? [`pair`] says yes (the caller
    /// holds it); [`open`] says no when no stream was built — the `Feed` is
    /// dropped there, and [`flush`] then drops and counts instead of filling
    /// a ring nobody drains.
    routed: bool,
    /// Installs the install ring refused. It cannot happen on the shipped
    /// path — the whole bank is `CUE_COUNT` installs into a `CUE_COUNT` ring,
    /// once — and it is counted rather than assumed.
    pub install_refused: u32,
    /// Commands and installs dropped at the flush for want of an output.
    pub unrouted: u32,
    /// The renderer's rate: the device's, or [`SAMPLE_RATE`] with no device.
    pub out_rate: u32,
}

impl Native {
    /// Queue a command for the audio thread. `false` — and counted in
    /// [`Native::ring_dropped`] — when the ring is full: the audio thread is
    /// `CMD_RING_CAP_NATIVE` commands behind.
    pub fn send(&mut self, cmd: Cmd) -> bool {
        self.out.send(cmd)
    }

    /// Hand a cue's samples across. A refusal drops them and is counted.
    pub fn install(&mut self, cue: Cue, pcm: Box<[i16]>) -> bool {
        let ok = self.out.install(cue, pcm);
        if !ok {
            self.install_refused = self.install_refused.saturating_add(1);
        }
        ok
    }

    /// The renderer's counters, if it has posted since the last read.
    pub fn stats(&mut self) -> Option<Stats> {
        self.out.stats()
    }

    /// Drop, here, every refused install the audio thread handed back; how
    /// many there were. The flush calls it once a frame.
    pub fn reclaim(&mut self) -> usize {
        self.out.reclaim()
    }

    /// Commands the ring refused since start. Drop-newest.
    pub fn ring_dropped(&self) -> u32 {
        self.out.ring_dropped
    }

    /// Commands waiting in the ring.
    pub fn backlog(&self) -> usize {
        self.out.backlog()
    }

    /// One-shots the audio thread had live at its last block.
    pub fn live_thread(&self) -> u32 {
        self.live.load(Ordering::Relaxed)
    }

    /// Has a device callback filled a buffer yet?
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Is there a [`Feed`] to drain the rings?
    pub fn routed(&self) -> bool {
        self.routed
    }

    /// Errors the device stream reported since it opened.
    pub fn stream_errors(&self) -> u32 {
        self.errors.load(Ordering::Relaxed)
    }
}

/// Both ends of the seam, unattached to any device or app — what
/// `tests/engine_out.rs` drives. The renderer is built at `out_rate`.
/// Allocates the four rings and the renderer, once; `routed`, because the
/// caller holds the [`Feed`].
pub fn pair(out_rate: u32) -> (Native, Feed) {
    let (out, input) = engine::channel();
    let live = Arc::new(AtomicU32::new(0));
    let alive = Arc::new(AtomicBool::new(false));
    (
        Native {
            out,
            live: live.clone(),
            alive: alive.clone(),
            errors: Arc::new(AtomicU32::new(0)),
            _stream: None,
            routed: true,
            install_refused: 0,
            unrouted: 0,
            out_rate,
        },
        Feed {
            renderer: Box::new(Renderer::new(out_rate)),
            input,
            buf: [0.0; 2 * BLOCK],
            // Dry, so the first frame asked for renders the first block.
            at: 2 * BLOCK,
            live,
            alive,
        },
    )
}

/// Open the device and start the callback. At plugin build, after
/// `DefaultPlugins` — `bevy::log` is live by then, and the `Engine`
/// resource this writes the device rate into already exists.
///
/// The order: default host → default output device → its default config →
/// a stream in the config's sample format → `play`. Every failure lands in
/// the same place — one `audio output: none` line, the [`Feed`] dropped,
/// `routed` false — because cpal on ALSA can answer `Some("default")` and
/// then fail at `build`, so no step before the last is proof of a device.
///
/// **The renderer is built at the device's rate**, after that rate is
/// known, so `engine::rate` is the one resample in the chain; on a box with
/// no device it is built at [`SAMPLE_RATE`] and dropped.
///
/// The commands `audio::setup` sends on `OnEnter(Loading)` — which runs
/// BEFORE `Startup` on a connected start, `audio::build_bank`'s trap — sit in
/// the ring until the first callback drains them; they are queued, never
/// skipped, which is the loading screen's contract with the beds.
pub fn open(app: &mut App) -> Native {
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        let (native, feed) = pair(SAMPLE_RATE);
        drop(feed);
        return unrouted(native, "no device");
    };
    let config = match device.default_output_config() {
        Ok(c) => c,
        Err(e) => {
            let (native, feed) = pair(SAMPLE_RATE);
            drop(feed);
            return unrouted(native, &format!("no default output config ({e})"));
        }
    };
    let rate = config.sample_rate().0;
    let format = config.sample_format();
    let stream_config: StreamConfig = config.config();
    let (mut native, feed) = pair(rate);
    let errors = native.errors.clone();
    // `SampleFormat` is `#[non_exhaustive]`, so the wildcard is required —
    // and it is the "unsupported, said once" path rather than a guess.
    let built = match format {
        SampleFormat::F32 => build::<f32>(&device, &stream_config, feed, errors),
        SampleFormat::I16 => build::<i16>(&device, &stream_config, feed, errors),
        SampleFormat::U16 => build::<u16>(&device, &stream_config, feed, errors),
        SampleFormat::I32 => build::<i32>(&device, &stream_config, feed, errors),
        SampleFormat::U32 => build::<u32>(&device, &stream_config, feed, errors),
        SampleFormat::I8 => build::<i8>(&device, &stream_config, feed, errors),
        SampleFormat::U8 => build::<u8>(&device, &stream_config, feed, errors),
        SampleFormat::F64 => build::<f64>(&device, &stream_config, feed, errors),
        SampleFormat::I64 => build::<i64>(&device, &stream_config, feed, errors),
        SampleFormat::U64 => build::<u64>(&device, &stream_config, feed, errors),
        other => {
            drop(feed);
            return unrouted(native, &format!("unsupported sample format {other:?}"));
        }
    };
    let stream = match built {
        Ok(s) => s,
        Err(e) => return unrouted(native, &format!("cannot build the output stream ({e})")),
    };
    if let Err(e) = stream.play() {
        return unrouted(native, &format!("cannot start the output stream ({e})"));
    }
    native._stream = Some(stream);
    // The rate `audio::pump` builds every `Start` against from now on.
    app.world_mut().resource_mut::<audio::Engine>().out_rate = rate;
    info!(
        "audio output: {rate} Hz, {} channel(s), {format:?}, the engine at the device rate",
        stream_config.channels
    );
    native
}

/// The device-less state, said once. Whatever the [`Feed`] was is gone by
/// now, so the rings have no drain and [`flush`] must not fill them.
fn unrouted(mut native: Native, why: &str) -> Native {
    info!("audio output: none - {why}; commands are dropped at the flush and counted");
    native.routed = false;
    native.out_rate = SAMPLE_RATE;
    native
}

/// One output stream in sample format `T`, its callback owning the [`Feed`].
/// The error callback only counts — it runs on cpal's thread and must not
/// log, lock or allocate.
fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut feed: Feed,
    errors: Arc<AtomicU32>,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = config.channels;
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _info: &cpal::OutputCallbackInfo| feed.fill(data, channels),
        move |_e: cpal::StreamError| {
            errors.fetch_add(1, Ordering::Relaxed);
        },
        None,
    )
}

/// Move the frame's traffic across the seam, `PostUpdate`: installs first
/// (so a command in the same drain can play what just landed — the order
/// `In::drain_into` keeps on the other side), then the commands, then the
/// refused installs back to be dropped here, then the audio thread's numbers
/// into [`audio::Diag`](super::audio::Diag), where
/// [`audio::pump`](super::audio::pump) prints the faults and the F7 report
/// and `web::heap_report` print the gauges.
///
/// With no output ([`Native::routed`] false) nothing is pushed: a ring with
/// no drain would fill to its cap and then count every command as a ring
/// drop, which is the wrong fact. They are dropped at the flush and counted
/// as [`Native::unrouted`] instead.
pub fn flush(mut engine: ResMut<audio::Engine>, mut out: NonSendMut<Native>) {
    if out.routed {
        for (cue, pcm) in engine.take_installs() {
            out.install(cue, pcm);
        }
        for cmd in engine.take() {
            out.send(cmd);
        }
    } else {
        let n = engine.take_installs().count() + engine.take().count();
        out.unrouted = out.unrouted.saturating_add(n as u32);
    }
    out.reclaim();
    if let Some(stats) = out.stats() {
        engine.diag.stats = stats;
    }
    let diag = &mut engine.diag;
    diag.ring_dropped = out.ring_dropped();
    diag.install_refused = out.install_refused;
    diag.live_thread = out.live_thread();
    diag.alive = out.alive();
    diag.routed = out.routed();
    diag.backlog = out.backlog();
    diag.unrouted = out.unrouted;
    diag.stream_errors = out.stream_errors();
}
