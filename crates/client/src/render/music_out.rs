//! The native client's music output: a decoder thread streaming one track at
//! a time into a ring that cpal's callback drains beside the engine.
//!
//! The score is whole tracks, four to five minutes each (`sound::music`).
//! Banked like a cue, a track is ~26 MB of PCM and a boot's worth of
//! decoding, so it is streamed instead: [`Stream`] owns a thread that opens
//! the track's Ogg file under the asset root, decodes it with `lewton`,
//! resamples it to the device rate and keeps about a second of stereo frames
//! in an `rtrb` ring. [`Tap::mix`] runs inside `audio_out::Feed::fill`, once
//! per rendered block, and adds what the ring holds at the music level.
//!
//! The callback keeps the engine's discipline: no allocation, no lock, no
//! wait. The level is an atomic the game thread writes each frame and the
//! tap ramps to across a block, so a slider or a fade never steps. A stop is
//! a generation counter: the tap drops whatever the ring holds the first
//! block it sees a new one and acknowledges it, and the thread waits for the
//! acknowledgement before it writes the next track — so the tail of the old
//! track never plays under the head of the new one.
//!
//! The browser has no twin of this file: the page plays the score through
//! its own `<audio>` element (`render/audio_web.rs`).

#![cfg(not(target_arch = "wasm32"))]

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use bevy::log::warn;
use lewton::inside_ogg::OggStreamReader;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::sound::music::Track;

/// How much decoded music the ring holds, seconds. Long enough to ride out a
/// stalled decoder thread, short enough that a stop drops little.
pub const RING_S: f32 = 1.0;

/// How long the thread waits for the tap to acknowledge a flush. A box whose
/// device never calls back would otherwise wait forever.
const FLUSH_WAIT: Duration = Duration::from_millis(300);

/// What the two threads share without a ring.
#[derive(Default)]
struct Shared {
    /// The music level, `f32` bits.
    gain: AtomicU32,
    /// Bumped by the thread on every stop or new track.
    flush: AtomicU32,
    /// The last flush the tap has carried out.
    ack: AtomicU32,
}

/// The callback's end: the ring's consumer and the level it ramps to.
pub struct Tap {
    ring: Consumer<f32>,
    shared: Arc<Shared>,
    /// The flush generation this tap has carried out.
    seen: u32,
    /// The level the last block ended on.
    g: f32,
}

impl Tap {
    /// Add one block of music to `buf` (interleaved stereo). Allocation-free.
    ///
    /// Adds nothing at all — not even `+0.0` — when the ring is dry, so a
    /// client with no music playing renders exactly what the engine alone
    /// renders.
    pub fn mix(&mut self, buf: &mut [f32]) {
        let gen = self.shared.flush.load(Ordering::Acquire);
        if gen != self.seen {
            let n = self.ring.slots();
            if let Ok(chunk) = self.ring.read_chunk(n) {
                chunk.commit_all();
            }
            self.seen = gen;
            self.shared.ack.store(gen, Ordering::Release);
        }
        let frames = buf.len() / 2;
        let target = f32::from_bits(self.shared.gain.load(Ordering::Relaxed));
        let g0 = self.g;
        self.g = target;
        let have = (self.ring.slots() / 2).min(frames);
        if have == 0 || frames == 0 {
            return;
        }
        let Ok(chunk) = self.ring.read_chunk(have * 2) else {
            return;
        };
        let (a, b) = chunk.as_slices();
        let mut it = a.iter().chain(b.iter());
        let step = (target - g0) / frames as f32;
        for i in 0..have {
            let g = g0 + step * (i + 1) as f32;
            let l = it.next().copied().unwrap_or(0.0);
            let r = it.next().copied().unwrap_or(0.0);
            buf[2 * i] += l * g;
            buf[2 * i + 1] += r * g;
        }
        chunk.commit_all();
    }
}

/// The game thread's end, before the thread exists.
pub struct Feeder {
    ring: Producer<f32>,
    shared: Arc<Shared>,
    out_rate: u32,
}

/// Both ends, unattached: what `audio_out::pair` builds beside the engine's
/// rings. The ring is allocated here, once.
pub fn pair(out_rate: u32) -> (Feeder, Tap) {
    let cap = ((out_rate as f32 * RING_S) as usize).max(1024) * 2;
    let (prod, cons) = RingBuffer::new(cap);
    let shared = Arc::new(Shared::default());
    (
        Feeder {
            ring: prod,
            shared: shared.clone(),
            out_rate,
        },
        Tap {
            ring: cons,
            shared,
            seen: 0,
            g: 0.0,
        },
    )
}

enum Ask {
    Play(PathBuf),
    Stop,
}

/// The running stream: the game thread's handle on the decoder thread.
pub struct Stream {
    tx: Sender<Ask>,
    shared: Arc<Shared>,
    root: Option<PathBuf>,
    /// Starts asked for with no asset root to read from.
    pub unrooted: u32,
}

impl Feeder {
    /// Start the decoder thread. `None` if the OS refuses a thread, which is
    /// said once by the caller and leaves the game silent of music only.
    pub fn spawn(self) -> Option<Stream> {
        let root = asset_root();
        self.spawn_at(root)
    }

    /// [`Feeder::spawn`] reading tracks under `root` (`assets/`) rather than
    /// wherever [`asset_root`] finds one — what a test hands it.
    pub fn spawn_at(self, root: Option<PathBuf>) -> Option<Stream> {
        let (tx, rx) = mpsc::channel();
        let shared = self.shared.clone();
        thread::Builder::new()
            .name("music".into())
            .spawn(move || run(rx, self.ring, self.shared, self.out_rate))
            .ok()?;
        Some(Stream {
            tx,
            shared,
            root,
            unrooted: 0,
        })
    }
}

impl Stream {
    /// Play `track` from its start, replacing whatever is playing.
    pub fn play(&mut self, track: Track) {
        let Some(root) = self.root.as_ref() else {
            self.unrooted = self.unrooted.saturating_add(1);
            return;
        };
        let _ = self.tx.send(Ask::Play(root.join(track.file())));
    }

    /// End what is playing.
    pub fn stop(&mut self) {
        let _ = self.tx.send(Ask::Stop);
    }

    /// The level to hold the music at; the tap ramps to it.
    pub fn set_gain(&mut self, gain: f32) {
        let g = if gain.is_finite() { gain.max(0.0) } else { 0.0 };
        self.shared.gain.store(g.to_bits(), Ordering::Relaxed);
    }
}

/// Where `assets/` is: the working directory's (a repo checkout, which is
/// what `bin/gates.rs` hands Bevy), `BEVY_ASSET_ROOT`'s, or the executable's
/// own directory's (a shipped layout).
pub fn asset_root() -> Option<PathBuf> {
    let mut tries: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        tries.push(cwd.join("assets"));
    }
    if let Some(dir) = std::env::var_os("BEVY_ASSET_ROOT") {
        tries.push(PathBuf::from(dir).join("assets"));
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        tries.push(dir.join("assets"));
    }
    tries.into_iter().find(|p| p.join("music").is_dir())
}

/// The decoder thread.
fn run(rx: Receiver<Ask>, mut ring: Producer<f32>, shared: Arc<Shared>, out_rate: u32) {
    let mut cur: Option<Reader> = None;
    let mut pending: Vec<f32> = Vec::with_capacity(8192);
    let mut at = 0usize;
    loop {
        let idle = cur.is_none() && at >= pending.len();
        let ask = if idle {
            match rx.recv() {
                Ok(a) => Some(a),
                Err(_) => return,
            }
        } else {
            match rx.try_recv() {
                Ok(a) => Some(a),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        };
        if let Some(ask) = ask {
            cur = None;
            pending.clear();
            at = 0;
            flush(&shared);
            if let Ask::Play(path) = ask {
                cur = Reader::open(&path, out_rate);
            }
            continue;
        }
        if at >= pending.len() {
            pending.clear();
            at = 0;
            if let Some(r) = cur.as_mut() {
                if !r.next(&mut pending) {
                    cur = None;
                }
            }
            continue;
        }
        let n = ring.slots().min(pending.len() - at) & !1;
        if n == 0 {
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        if let Ok(mut chunk) = ring.write_chunk(n) {
            let (a, b) = chunk.as_mut_slices();
            let (x, y) = pending[at..at + n].split_at(a.len());
            a.copy_from_slice(x);
            b.copy_from_slice(y);
            chunk.commit_all();
            at += n;
        }
    }
}

/// Ask the tap to drop what the ring holds, and wait (bounded) until it has.
fn flush(shared: &Shared) {
    let gen = shared.flush.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    let t0 = Instant::now();
    while shared.ack.load(Ordering::Acquire) != gen && t0.elapsed() < FLUSH_WAIT {
        thread::sleep(Duration::from_millis(2));
    }
}

/// One track being decoded and resampled to the device rate.
struct Reader {
    ogg: OggStreamReader<BufReader<File>>,
    channels: usize,
    /// Source frames per output frame.
    step: f64,
    /// Where the next output frame sits between `prev` and the next source
    /// frame, in source frames.
    frac: f64,
    prev: [f32; 2],
    primed: bool,
}

impl Reader {
    fn open(path: &Path, out_rate: u32) -> Option<Reader> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                warn!("music: cannot open {} ({e})", path.display());
                return None;
            }
        };
        let ogg = match OggStreamReader::new(BufReader::new(file)) {
            Ok(r) => r,
            Err(e) => {
                warn!("music: {} is not an Ogg Vorbis file ({e})", path.display());
                return None;
            }
        };
        let channels = ogg.ident_hdr.audio_channels as usize;
        let rate = ogg.ident_hdr.audio_sample_rate;
        if channels == 0 || rate == 0 || out_rate == 0 {
            return None;
        }
        Some(Reader {
            ogg,
            channels,
            step: rate as f64 / out_rate as f64,
            frac: 0.0,
            prev: [0.0; 2],
            primed: false,
        })
    }

    /// Decode the next packet into `out` (interleaved stereo at the device
    /// rate). False at the end of the track or on a decode error.
    fn next(&mut self, out: &mut Vec<f32>) -> bool {
        loop {
            match self.ogg.read_dec_packet_itl() {
                Ok(Some(pkt)) if pkt.is_empty() => continue,
                Ok(Some(pkt)) => {
                    for fr in pkt.chunks_exact(self.channels) {
                        let l = fr[0] as f32 / 32_768.0;
                        let r = fr.get(1).map_or(l, |r| *r as f32 / 32_768.0);
                        self.push([l, r], out);
                    }
                    return true;
                }
                Ok(None) => return false,
                Err(e) => {
                    warn!("music: decode stopped ({e})");
                    return false;
                }
            }
        }
    }

    /// One source frame through the linear resampler.
    fn push(&mut self, cur: [f32; 2], out: &mut Vec<f32>) {
        if !self.primed {
            self.prev = cur;
            self.primed = true;
            return;
        }
        while self.frac < 1.0 {
            let t = self.frac as f32;
            out.push(self.prev[0] + (cur[0] - self.prev[0]) * t);
            out.push(self.prev[1] + (cur[1] - self.prev[1]) * t);
            self.frac += self.step;
        }
        self.frac -= 1.0;
        self.prev = cur;
    }
}
