//! Recorded sessions, the raw stock for trailers (`ci/film.sh`).
//!
//! A recording is everything a live session's core was handed, and when: the
//! welcome, then every datagram and reliable event with the session clock
//! (the sum of `pump`'s `dt_ms`) it was drained at. `bin/record.rs` makes one
//! against a live shard in real time; `render::film` hands the same bytes back
//! to an ordinary [`Session`](crate::Session) on a fixed frame clock, however
//! long a CPU rasterizer takes to draw each frame. `ClientCore` is pure, so
//! the same bytes at the same clock are the same world, drawn smoothly.
//!
//! A recording is only good for the build that made it: the entries are wire
//! bytes, so the header carries `PROTO_VER` and [`read`] refuses any other.
//!
//! A spectator's tape (`record --spectate`) also carries the watch the shard
//! sent after its welcome, so its replay is a spectator core too: the welcome's
//! `player_id` is then the watched body, which a film can follow.

use std::io::{self, Read, Write};

pub const MAGIC: &[u8; 8] = b"GATESREC";
/// A spectator's tape: [`MAGIC`]'s header, then the encoded watch.
pub const MAGIC_WATCH: &[u8; 8] = b"GATESRCW";
/// Room for an encoded watch: an address and a 16-byte name.
const WATCH_BYTES: usize = 64;

/// Which lane an entry arrived on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Datagram = 0,
    Event = 1,
}

pub struct Entry {
    /// The session clock the entry was drained at, ms since the first pump.
    pub t_ms: f64,
    pub lane: Lane,
    pub bytes: Vec<u8>,
}

pub struct Recording {
    pub welcome: protocol::Welcome,
    /// Who the recorder watched, on a spectator's tape.
    pub watch: Option<protocol::Watch>,
    pub entries: Vec<Entry>,
}

impl Recording {
    /// The clock of the last entry — how much world the recording holds.
    pub fn end_ms(&self) -> f64 {
        self.entries.last().map_or(0.0, |e| e.t_ms)
    }
}

pub fn write_header(
    w: &mut impl Write,
    welcome: &protocol::Welcome,
    watch: Option<&protocol::Watch>,
) -> io::Result<()> {
    w.write_all(if watch.is_some() { MAGIC_WATCH } else { MAGIC })?;
    w.write_all(&u32::from(protocol::PROTO_VER).to_le_bytes())?;
    w.write_all(&welcome.player_id.to_le_bytes())?;
    w.write_all(&welcome.seed.to_le_bytes())?;
    w.write_all(&welcome.tick.to_le_bytes())?;
    w.write_all(&[welcome.dev as u8])?;
    w.write_all(&welcome.blight_salt.to_le_bytes())?;
    if let Some(watch) = watch {
        let mut buf = [0u8; WATCH_BYTES];
        let n = protocol::encode_watch(watch, &mut buf)
            .map_err(|e| bad(format!("encode watch: {e:?}")))?;
        w.write_all(&(n as u32).to_le_bytes())?;
        w.write_all(&buf[..n])?;
    }
    Ok(())
}

pub fn write_entry(w: &mut impl Write, t_ms: f64, lane: Lane, bytes: &[u8]) -> io::Result<()> {
    w.write_all(&t_ms.to_le_bytes())?;
    w.write_all(&[lane as u8])?;
    w.write_all(&(bytes.len() as u32).to_le_bytes())?;
    w.write_all(bytes)
}

fn bad(why: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

/// A cursor over the whole file, so a short read is an error with an offset.
struct Cursor<'a> {
    all: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        let s = self
            .all
            .get(self.at..self.at + n)
            .ok_or_else(|| bad(format!("truncated at byte {}", self.at)))?;
        self.at += n;
        Ok(s)
    }

    fn u32(&mut self) -> io::Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> io::Result<u64> {
        let mut b = [0u8; 8];
        b.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(b))
    }
}

pub fn read(r: &mut impl Read) -> io::Result<Recording> {
    let mut all = Vec::new();
    r.read_to_end(&mut all)?;
    let mut c = Cursor { all: &all, at: 0 };
    let watched = match c.take(8)? {
        m if m == MAGIC => false,
        m if m == MAGIC_WATCH => true,
        _ => return Err(bad("not a gates recording".into())),
    };
    let ver = c.u32()?;
    if ver != u32::from(protocol::PROTO_VER) {
        return Err(bad(format!(
            "recorded on wire v{ver}, this build speaks v{} — record it again",
            protocol::PROTO_VER
        )));
    }
    let player_id = c.u32()?;
    let seed = c.u64()?;
    let tick = c.u32()?;
    let dev = c.take(1)?[0] != 0;
    let blight_salt = c.u32()?;
    let welcome = protocol::Welcome {
        player_id,
        seed,
        tick,
        dev,
        blight_salt,
    };
    let watch = if watched {
        let len = c.u32()? as usize;
        let bytes = c.take(len)?;
        Some(protocol::decode_watch(bytes).map_err(|e| bad(format!("watch: {e:?}")))?)
    } else {
        None
    };
    let mut entries = Vec::new();
    while c.at < all.len() {
        let t_ms = f64::from_bits(c.u64()?);
        let lane = match c.take(1)?[0] {
            0 => Lane::Datagram,
            1 => Lane::Event,
            other => return Err(bad(format!("unknown lane {other}"))),
        };
        let len = c.u32()? as usize;
        let bytes = c.take(len)?.to_vec();
        entries.push(Entry { t_ms, lane, bytes });
    }
    Ok(Recording {
        welcome,
        watch,
        entries,
    })
}

/// Events the replay may queue ahead of a pump. Far above what one frame
/// carries; the join burst is the largest and it is dozens.
pub const REPLAY_EVENTS: usize = 1024;

/// A session with no shard behind it: its lanes are fed from a recording
/// ([`Replay`]) instead of a socket, and its inputs go nowhere. Everything
/// the renderer reads — the core, the welcome, `pump` — is the live session's
/// own code, which is the point: a replay is drawn exactly as the game draws.
///
/// Here and not in `lib.rs`, whose one native `impl Session` is the desktop
/// join sequence `tests/connect_twins.rs` holds against the browser's. On
/// both targets: a page replays a clip (`crate::clip`) the same way.
impl crate::Session {
    pub fn replay(rec: Recording) -> (Self, Replay) {
        use sim_core::limits::{CLIENT_DG_RING, DATAGRAM_BUDGET_BYTES};
        let welcome = rec.welcome;
        let watching = rec.watch;
        let datagrams = crate::net::datagram_lane();
        let (ev_tx, events) = tokio::sync::mpsc::channel::<Vec<u8>>(REPLAY_EVENTS);
        // No writer task holds the receiver, so an action is refused as
        // `Closed` — nothing a replay could send has anywhere to go.
        let (actions, _) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
        let replay = Replay::new(rec, datagrams.clone(), ev_tx);
        // A spectator's tape replays as a spectator: the core interpolates
        // the watched body, which is the one a film follows.
        let core = if watching.is_some() {
            client_core::core::ClientCore::spectator(welcome.seed, welcome.player_id, welcome.tick)
        } else {
            client_core::core::ClientCore::new(welcome.seed, welcome.player_id, welcome.tick)
        };
        let session = Self {
            core,
            applied: 0,
            applied2: 0,
            welcome,
            watching,
            wire: crate::net::ActiveWire::detached(),
            actions,
            events,
            datagrams,
            snapshots: 0,
            input_buf: [0u8; DATAGRAM_BUDGET_BYTES],
            dg_scratch: (0..CLIENT_DG_RING)
                .map(|_| Vec::with_capacity(DATAGRAM_BUDGET_BYTES))
                .collect(),
            closed: false,
            refused: None,
            travel: None,
            event_observer: None,
            observer_failed: false,
            tap: None,
            clip: None,
        };
        (session, replay)
    }
}

/// The recording's end of a replayed session's two inbound lanes.
///
/// [`Session::replay`](crate::Session::replay) builds the session and this
/// together; each frame the film pushes whatever the recorded clock has
/// reached and the session's ordinary `pump` drains it, so everything
/// downstream of the lanes runs exactly as it does live.
pub struct Replay {
    entries: Vec<Entry>,
    next: usize,
    pub(crate) datagrams: crate::net::DatagramRx,
    pub(crate) events: tokio::sync::mpsc::Sender<Vec<u8>>,
    /// The session clock the replay has reached, ms.
    pub clock_ms: f64,
    end_ms: f64,
}

impl Replay {
    /// Built only beside a replayed `Session` (`Session::replay`).
    pub(crate) fn new(
        rec: Recording,
        datagrams: crate::net::DatagramRx,
        events: tokio::sync::mpsc::Sender<Vec<u8>>,
    ) -> Self {
        let end_ms = rec.end_ms();
        Self {
            entries: rec.entries,
            next: 0,
            datagrams,
            events,
            clock_ms: 0.0,
            end_ms,
        }
    }

    /// Queue every entry recorded at or before the clock. Stops early rather
    /// than drop anything when a lane is full: the snapshots are deltas
    /// against ones the recorder acked, so a replay that lost one would
    /// decode the rest against the wrong baseline. The caller pumps and
    /// feeds again.
    pub fn feed(&mut self) -> usize {
        let mut fed = 0;
        while let Some(e) = self.entries.get(self.next) {
            if e.t_ms > self.clock_ms {
                break;
            }
            match e.lane {
                Lane::Datagram => {
                    let Ok(mut ring) = self.datagrams.lock() else {
                        break;
                    };
                    if ring.is_full() {
                        break;
                    }
                    ring.push(&e.bytes);
                }
                Lane::Event => {
                    if self.events.try_send(e.bytes.clone()).is_err() {
                        break;
                    }
                }
            }
            self.next += 1;
            fed += 1;
        }
        fed
    }

    /// Is every entry at or before the clock queued?
    pub fn caught_up(&self) -> bool {
        self.entries
            .get(self.next)
            .is_none_or(|e| e.t_ms > self.clock_ms)
    }

    pub fn end_ms(&self) -> f64 {
        self.end_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recording_reads_back_what_was_written() {
        let welcome = protocol::Welcome {
            player_id: 7,
            seed: 0xdead_beef_0102_0304,
            tick: 912,
            dev: true,
            blight_salt: 0x0B11_6417,
        };
        let mut buf = Vec::new();
        write_header(&mut buf, &welcome, None).unwrap();
        write_entry(&mut buf, 0.0, Lane::Event, &[1, 2, 3]).unwrap();
        write_entry(&mut buf, 33.5, Lane::Datagram, &[]).unwrap();
        write_entry(&mut buf, 66.25, Lane::Datagram, &[9; 300]).unwrap();
        let rec = read(&mut buf.as_slice()).unwrap();
        assert_eq!(rec.welcome, welcome);
        assert_eq!(rec.entries.len(), 3);
        assert_eq!(rec.entries[0].lane, Lane::Event);
        assert_eq!(rec.entries[0].bytes, [1, 2, 3]);
        assert_eq!(rec.entries[2].t_ms, 66.25);
        assert_eq!(rec.entries[2].bytes.len(), 300);
        assert_eq!(rec.end_ms(), 66.25);
        // A torn tail is an error, never a shorter recording.
        buf.truncate(buf.len() - 1);
        assert!(read(&mut buf.as_slice()).is_err());
    }

    #[test]
    fn a_spectators_tape_reads_back_its_watch() {
        let welcome = protocol::Welcome {
            player_id: 0x0207,
            seed: 11,
            tick: 3,
            dev: false,
            blight_salt: 0,
        };
        let watch = protocol::Watch {
            address: protocol::Address([0x5a; 20]),
            agent: true,
            name: protocol::Name::new("Doctor Wood").unwrap(),
        };
        let mut buf = Vec::new();
        write_header(&mut buf, &welcome, Some(&watch)).unwrap();
        write_entry(&mut buf, 12.5, Lane::Datagram, &[4; 40]).unwrap();
        let rec = read(&mut buf.as_slice()).unwrap();
        assert_eq!(rec.welcome, welcome);
        assert_eq!(rec.watch, Some(watch));
        assert_eq!(rec.entries.len(), 1);
        assert_eq!(rec.entries[0].t_ms, 12.5);
    }
}
