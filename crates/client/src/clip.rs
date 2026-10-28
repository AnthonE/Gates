//! The clip key's memory: the last minute of a live session, kept so that a
//! player can turn it into a video (`render::film`) without anything leaving
//! their machine.
//!
//! A clip is an ordinary tape (`crate::film`), so the film draws it exactly
//! as it draws a trailer's. What makes it hard is that it starts mid-stream:
//!
//!   · **Snapshots are deltas** against baselines the client acked. A tape
//!     cut at second 600 would open on deltas whose baselines are at second
//!     599, and the replay would refuse every one (`Applied::NoBaseline`).
//!     So each [`Seg`] opens with the baselines the live view held when it
//!     began, re-encoded as keyframes (`baseline_age == 0`). Decoded, they
//!     are byte for byte the snapshots the deltas after them name.
//!   · **Events are state** — the pieces standing, the bags, the catalogs —
//!     built up since the join. They are kept from the welcome on and played
//!     in a burst at the clip's first instant, ahead of the keyframes.
//!
//! Datagrams are kept for [`KEEP_SEGS`] segments of [`SEG_MS`] each and the
//! oldest is dropped as a new one opens. Events are kept for the session, up
//! to [`EVENT_CAP_BYTES`]; past that the clip key is off until a rejoin.
//!
//! The tape replays as a spectator of the player (`Watch` in its header), so
//! the player's own body is interpolated off the wire like any watched body:
//! a replay has no input to predict it from.

use std::collections::VecDeque;

use client_core::view::ClientView;
use protocol::{encode_snapshot, Welcome};

use crate::film::{self, Lane};

/// One segment of datagrams, milliseconds.
pub const SEG_MS: f64 = 15_000.0;
/// Segments kept: a clip may reach back between `(KEEP_SEGS − 1) · SEG_MS`
/// and `KEEP_SEGS · SEG_MS`.
pub const KEEP_SEGS: usize = 5;
/// Events kept from the join, bytes. Past it the clip key is off.
pub const EVENT_CAP_BYTES: usize = 32 << 20;
/// Tape a clip carries ahead of what it asks for: the film's boot plays two
/// seconds and each shot settles for one and a half before it rolls.
pub const LEAD_S: f64 = 4.0;
/// Room for one keyframe: a tick's bodies, absolute.
const KEY_BYTES: usize = 64 << 10;

/// A run of datagrams, and the baselines that let it be decoded alone.
struct Seg {
    /// The session clock the segment opened at.
    start_ms: f64,
    /// The newest baseline's tick, which a replay's clock starts from.
    start_tick: u32,
    keys: Vec<Vec<u8>>,
    dgrams: Vec<(f64, Vec<u8>)>,
}

pub struct Clipper {
    welcome: Welcome,
    watch: protocol::Watch,
    clock_ms: f64,
    events: Vec<(f64, Vec<u8>)>,
    event_bytes: usize,
    segs: VecDeque<Seg>,
    full: bool,
    scratch: Vec<u8>,
}

impl Clipper {
    /// `watch` is who a spectator session watches; a player's own session
    /// passes `None` and is replayed as a watcher of itself.
    pub fn new(welcome: Welcome, watch: Option<protocol::Watch>) -> Self {
        Self {
            welcome,
            watch: watch.unwrap_or(protocol::Watch {
                address: protocol::Address::GUEST,
                agent: false,
                name: protocol::Name::EMPTY,
            }),
            clock_ms: 0.0,
            events: Vec::new(),
            event_bytes: 0,
            segs: VecDeque::with_capacity(KEEP_SEGS + 1),
            full: false,
            scratch: vec![0; KEY_BYTES],
        }
    }

    /// One inbound datagram or event, as `Session::pump` drains it.
    pub fn push(&mut self, lane: Lane, bytes: &[u8]) {
        if self.full {
            return;
        }
        match lane {
            Lane::Event => {
                self.event_bytes += bytes.len();
                if self.event_bytes > EVENT_CAP_BYTES {
                    self.full = true;
                    self.events = Vec::new();
                    self.segs.clear();
                    return;
                }
                self.events.push((self.clock_ms, bytes.to_vec()));
            }
            // Before the first segment there is no baseline to start from;
            // the first segment's keyframes carry what these built.
            Lane::Datagram => {
                if let Some(seg) = self.segs.back_mut() {
                    seg.dgrams.push((self.clock_ms, bytes.to_vec()));
                }
            }
        }
    }

    /// After the pump has applied everything it drained: move the clock on,
    /// and open a segment when the last one is old enough.
    pub fn after_pump(&mut self, view: &ClientView, dt_ms: f64) {
        self.clock_ms += dt_ms;
        if self.full {
            return;
        }
        let due = self
            .segs
            .back()
            .is_none_or(|s| self.clock_ms - s.start_ms >= SEG_MS);
        if !due {
            return;
        }
        let Some(seg) = self.open(view) else {
            return;
        };
        self.segs.push_back(seg);
        while self.segs.len() > KEEP_SEGS {
            self.segs.pop_front();
        }
    }

    /// A segment starting now: every baseline the view holds, absolute.
    fn open(&mut self, view: &ClientView) -> Option<Seg> {
        let held = view.held();
        let start_tick = held.last()?.header.tick;
        let mut keys = Vec::with_capacity(held.len());
        for snap in held {
            let header = protocol::SnapshotHeader {
                baseline_age: 0,
                ..snap.header
            };
            // A tick too big to re-encode is a segment not opened; the last
            // one keeps growing and the next frame tries again.
            let n = encode_snapshot(&header, &[], snap.entities(), &[], &mut self.scratch).ok()?;
            keys.push(self.scratch[..n].to_vec());
        }
        Some(Seg {
            start_ms: self.clock_ms,
            start_tick,
            keys,
            dgrams: Vec::new(),
        })
    }

    /// Off for the rest of the session: the events outgrew their cap.
    pub fn full(&self) -> bool {
        self.full
    }

    /// Seconds of play a clip can reach back over, lead included.
    pub fn held_s(&self) -> f64 {
        self.segs
            .front()
            .map_or(0.0, |s| (self.clock_ms - s.start_ms) / 1000.0)
    }

    /// A tape of at least the last `want_s` seconds plus [`LEAD_S`], or as
    /// much as is held: the newest segment that reaches back that far, its
    /// keyframes, and everything after it, with the clock restarted at its
    /// start. `None` before the first segment.
    pub fn tape(&self, want_s: f64) -> Option<Vec<u8>> {
        let back_to = self.clock_ms - (want_s + LEAD_S) * 1000.0;
        let from = self
            .segs
            .iter()
            .rposition(|s| s.start_ms <= back_to)
            .unwrap_or(0);
        let first = self.segs.get(from)?;
        let t0 = first.start_ms;
        let welcome = Welcome {
            tick: first.start_tick,
            ..self.welcome
        };
        let mut out = Vec::new();
        film::write_header(&mut out, &welcome, Some(&self.watch)).ok()?;
        // Everything the events built before the segment, in a burst at 0.
        let cut = self.events.partition_point(|(t, _)| *t < t0);
        for (_, bytes) in &self.events[..cut] {
            film::write_entry(&mut out, 0.0, Lane::Event, bytes).ok()?;
        }
        for key in &first.keys {
            film::write_entry(&mut out, 0.0, Lane::Datagram, key).ok()?;
        }
        // Then both lanes in clock order. A pump drains datagrams before
        // events, so a datagram wins a tie.
        let mut dgrams = self.segs.iter().skip(from).flat_map(|s| s.dgrams.iter());
        let mut events = self.events[cut..].iter();
        let (mut d, mut e) = (dgrams.next(), events.next());
        loop {
            let take_d = match (d, e) {
                (None, None) => break,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (Some((td, _)), Some((te, _))) => td <= te,
            };
            if take_d {
                let (t, bytes) = d?;
                film::write_entry(&mut out, t - t0, Lane::Datagram, bytes).ok()?;
                d = dgrams.next();
            } else {
                let (t, bytes) = e?;
                film::write_entry(&mut out, t - t0, Lane::Event, bytes).ok()?;
                e = events.next();
            }
        }
        Some(out)
    }
}

/// The clip styles: a shot list each (`render::film::Script`), its `at`s
/// counted back from the tape's end. Vertical unless the name says wide:
/// TikTok, Reels and Shorts are 9:16.
pub const STYLES: [&str; 5] = ["pov", "follow", "mix", "long", "wide"];

/// Seconds of play the longest style draws, which is what the clip key asks
/// the [`Clipper`] for.
pub const CLIP_S: f64 = 30.5;

/// A style's shot list as JSON, `None` for a name not in [`STYLES`].
pub fn script(style: &str) -> Option<String> {
    // The camera behind and a little above the player, slow to follow.
    const CHASE: &str = r#"{"back": 3.4, "up": 1.5, "side": 0.5, "lag": 0.35}"#;
    let pov = |at: f64, len: f64| {
        format!(r#"{{"name": "pov", "at": {at}, "len": {len}, "pov": true, "fov": 78}}"#)
    };
    let follow = |at: f64, len: f64| {
        format!(r#"{{"name": "follow", "at": {at}, "len": {len}, "follow": {CHASE}, "fov": 64}}"#)
    };
    let (size, shots) = match style {
        "pov" => ((1080, 1920), vec![pov(-15.2, 15.0)]),
        "follow" => ((1080, 1920), vec![follow(-15.2, 15.0)]),
        // A cut needs room for its settle between the two (`settle` below).
        "mix" => ((1080, 1920), vec![follow(-16.6, 8.0), pov(-8.1, 8.0)]),
        "long" => ((1080, 1920), vec![pov(-30.2, 30.0)]),
        "wide" => ((1920, 1080), vec![pov(-30.2, 30.0)]),
        _ => return None,
    };
    Some(format!(
        r#"{{"width": {}, "height": {}, "fps": 30, "settle": 12,
            "look": {{"saturation": 1.06, "contrast": 1.04}},
            "shots": [{}]}}"#,
        size.0,
        size.1,
        shots.join(", ")
    ))
}

/// Join a film's shots (`NN-name.mp4` beside `NN-name.wav`) into one video
/// with its sound: the desktop's end of a clip (`gates --clip`). Needs
/// `ffmpeg` on the path, as the film itself does.
#[cfg(not(target_arch = "wasm32"))]
pub fn cut(shots: &std::path::Path, out: &std::path::Path) -> Result<(), String> {
    let mut mp4s: Vec<_> = std::fs::read_dir(shots)
        .map_err(|e| format!("{}: {e}", shots.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "mp4"))
        .collect();
    mp4s.sort();
    if mp4s.is_empty() {
        return Err(format!("{}: no shots", shots.display()));
    }
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    let mut graph = String::new();
    for (i, mp4) in mp4s.iter().enumerate() {
        cmd.arg("-i")
            .arg(mp4)
            .arg("-i")
            .arg(mp4.with_extension("wav"));
        graph.push_str(&format!("[{}:v][{}:a]", 2 * i, 2 * i + 1));
    }
    graph.push_str(&format!("concat=n={}:v=1:a=1[v][a]", mp4s.len()));
    let status = cmd
        .args(["-filter_complex", &graph, "-map", "[v]", "-map", "[a]"])
        .args(["-c:v", "libx264", "-preset", "medium", "-crf", "18"])
        .args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k"])
        .args(["-movflags", "+faststart"])
        .arg(out)
        .status()
        .map_err(|e| format!("cannot start ffmpeg: {e}"))?;
    if !status.success() {
        return Err(format!("ffmpeg could not join the shots ({status})"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use client_core::view::Applied;
    use protocol::{EntityState, Nudge, SnapshotHeader};
    use sim_core::limits::SNAPSHOT_INTERVAL_TICKS;

    const STEP: u32 = SNAPSHOT_INTERVAL_TICKS as u32;

    fn welcome() -> Welcome {
        Welcome {
            player_id: 3,
            seed: 99,
            tick: 1,
            dev: false,
            blight_salt: 0,
        }
    }

    fn bodies(tick: u32) -> Vec<EntityState> {
        (0..4)
            .map(|i| EntityState {
                id: 3 + i,
                qx: (tick as i32) * (i as i32 + 1),
                qz: 40 - i as i32,
                grounded: true,
                ..EntityState::default()
            })
            .collect()
    }

    /// The shard's side: tick `n`'s snapshot as a delta against the one
    /// `age` snapshots back (a round trip's worth of acks), or a keyframe.
    fn wire(n: u32, age: u32) -> Vec<u8> {
        let tick = n * STEP;
        let (baseline_age, base) = if n > age {
            (age * STEP, bodies((n - age) * STEP))
        } else {
            (0, Vec::new())
        };
        let header = SnapshotHeader {
            tick,
            baseline_age: baseline_age as u8,
            last_executed_seq: 0,
            nudge: Nudge::Ok,
            buffered_depth: 0,
            repeat_count: 0,
        };
        let mut buf = vec![0u8; KEY_BYTES];
        let n = encode_snapshot(&header, &[], &bodies(tick), &base, &mut buf).unwrap();
        buf.truncate(n);
        buf
    }

    #[test]
    fn a_clip_cut_mid_stream_decodes_every_delta_after_it() {
        let mut live = ClientView::new();
        let mut clip = Clipper::new(welcome(), None);
        clip.push(Lane::Event, &[7, 7, 7]);
        // Two and a half segments of play at 15 snapshots a second.
        let frames = (SEG_MS * 2.5 / 66.0) as u32;
        for n in 1..=frames {
            let d = wire(n, 4);
            clip.push(Lane::Datagram, &d);
            assert!(matches!(live.apply(&d), Ok(Applied::Ok { .. })));
            if n == frames / 2 {
                clip.push(Lane::Event, &[8]);
            }
            clip.after_pump(&live, 66.0);
        }
        assert!(clip.held_s() > SEG_MS * 2.0 / 1000.0);
        let tape = clip.tape(5.0).unwrap();
        let rec = film::read(&mut tape.as_slice()).unwrap();
        assert!(rec.watch.is_some(), "a player's clip replays as a watcher");
        assert!(rec.end_ms() >= (5.0 + LEAD_S) * 1000.0 - 66.0);
        assert!(rec.end_ms() < SEG_MS * 2.0);
        // The first event is from before the cut, so it plays at 0.
        assert_eq!(rec.entries[0].lane, Lane::Event);
        assert_eq!(rec.entries[0].bytes, [7, 7, 7]);
        assert_eq!(rec.entries[0].t_ms, 0.0);
        let mut replay = ClientView::new();
        let mut refused = 0;
        for e in rec.entries.iter().filter(|e| e.lane == Lane::Datagram) {
            match replay.apply(&e.bytes) {
                Ok(Applied::Ok { .. }) => {}
                other => {
                    refused += 1;
                    eprintln!("refused at {} ms: {other:?}", e.t_ms);
                }
            }
        }
        assert_eq!(refused, 0, "every datagram on the tape decodes");
        assert_eq!(replay.newest_applied, live.newest_applied);
        assert_eq!(replay.entities, live.entities);
    }

    #[test]
    fn a_long_ask_takes_the_oldest_segment_and_events_keep_their_order() {
        let mut live = ClientView::new();
        let mut clip = Clipper::new(welcome(), None);
        assert!(
            clip.tape(10.0).is_none(),
            "nothing before the first snapshot"
        );
        let frames = (SEG_MS * (KEEP_SEGS as f64 + 2.0) / 66.0) as u32;
        for n in 1..=frames {
            let d = wire(n, 2);
            clip.push(Lane::Datagram, &d);
            live.apply(&d).unwrap();
            clip.push(Lane::Event, &n.to_le_bytes());
            clip.after_pump(&live, 66.0);
        }
        let held = clip.held_s();
        assert!(held <= KEEP_SEGS as f64 * SEG_MS / 1000.0 + 0.1, "{held}");
        let rec = film::read(&mut clip.tape(999.0).unwrap().as_slice()).unwrap();
        assert!((rec.end_ms() / 1000.0 - held).abs() < 0.2);
        let mut last = 0.0;
        let mut seen = 0u32;
        for e in &rec.entries {
            assert!(e.t_ms >= last, "the tape runs forward");
            last = e.t_ms;
            if e.lane == Lane::Event {
                let n = u32::from_le_bytes(e.bytes[..4].try_into().unwrap());
                assert_eq!(n, seen + 1, "every event, in order");
                seen = n;
            }
        }
        assert_eq!(seen, frames);
    }
}
