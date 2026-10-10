//! The wipe (`ARC.md` F8): a posted schedule, the admin's `/wipe`, and the
//! file operation that ends a world.
//!
//! ## What a wipe is, on disk
//!
//! The world file, its backups and its trust log move into an archive
//! directory beside it (`<world_file>.wipes/<unix>/`) — **archived, never
//! deleted** (`ALPHA.md`: a wipe archives the final state). The player store
//! is copied there too, then rewritten in place: a record keeps only what
//! survives a wipe (`DESIGN.md` §2: blueprints, and the glyphs a player has
//! learned to read), as a body that logged off dead, so the next join wakes
//! on a beach with the spawn kit like any fresh character. On a blueprint
//! wipe every record is cleared. The seed stays: a new island is the
//! operator's call (`shard-public.toml`'s seed note).
//!
//! The next boot finds no world file and generates the island fresh, with a
//! fresh arc salt (`net.rs`), so the puzzles and the arc clock reset with it.
//!
//! ## Where it happens, and why at a boot boundary
//!
//! Never under a running sim. A live wipe (the schedule's hour, or an
//! admin's countdown) ends in the ordinary graceful shutdown — the world and
//! every player record flushed — and `bin/shard.rs` then applies the wipe to
//! the closed files and exits; the supervisor (`ops/gates-shard.service`,
//! `Restart=always`) brings the shard back on the fresh island. A shard that
//! was down when its scheduled wipe came applies it at its next boot, before
//! a file is opened ([`State`] remembers when the last one was).
//!
//! ## The schedule
//!
//! `wipe_days` from an anchor instant: wipes fall at `anchor + k · days`.
//! The default anchor is 1970-01-01 19:00 UTC, which was a Thursday — so
//! `wipe_days = 7` is every Thursday at 19:00 UTC (Rust's forced-wipe slot),
//! 14 every other Thursday, 28 every fourth. `wipe_anchor` moves it.
//! `blueprint_wipe_every` makes every Nth wipe a blueprint wipe (default 2:
//! blueprints survive one wipe, `DESIGN.md` §2).

use crate::store::{
    decode_record, encode_record, record_offset, MAX_SAVED_PLAYERS, SAVE_FORMAT, SAVE_HEADER_BYTES,
    SAVE_MAGIC, SAVE_RECORD_BYTES,
};
use protocol::ChatText;
use sim_core::persist::PlayerSave;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const DAY: u64 = 86_400;

/// 1970-01-01 19:00 UTC, a Thursday: the default anchor.
pub const DEFAULT_ANCHOR: u64 = 19 * 3_600;

/// When the countdown speaks, seconds before the wipe. The first one inside
/// the window is said at once (an admin's `/wipe 5` announces "5 min" as it
/// is typed), then each threshold as it passes.
pub const WARN_AT: [u64; 10] = [3_600, 1_800, 900, 600, 300, 120, 60, 30, 10, 3];

/// A posted wipe schedule: every `every` seconds from `anchor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub every: u64,
    pub anchor: u64,
}

impl Schedule {
    pub fn days(days: u32, anchor: u64) -> Self {
        Self {
            every: days as u64 * DAY,
            anchor,
        }
    }

    /// The latest scheduled instant at or before `now`, or `None` before the
    /// first one.
    pub fn last_at_or_before(&self, now: u64) -> Option<u64> {
        (now >= self.anchor).then(|| self.anchor + (now - self.anchor) / self.every * self.every)
    }

    /// The first scheduled instant strictly after `now`.
    pub fn next_after(&self, now: u64) -> u64 {
        match self.last_at_or_before(now) {
            Some(t) => t + self.every,
            None => self.anchor,
        }
    }
}

/// Days since 1970-01-01 of a civil date (Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count (Hinnant's algorithm): (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

/// `wipe_anchor`'s value: `YYYY-MM-DD HH:MM`, UTC, with a `T` instead of the
/// space, optional `:SS` and an optional trailing `Z` all accepted.
pub fn parse_anchor(s: &str) -> Result<u64, String> {
    let bad = || format!("wipe_anchor `{s}`: want YYYY-MM-DD HH:MM (UTC)");
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = s.split_once(['T', ' ']).ok_or_else(bad)?;
    let mut d = date.split('-');
    let y: i64 = d.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    let mo: u32 = d.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    let da: u32 = d.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    let mut t = time.split(':');
    let h: u64 = t.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    let mi: u64 = t.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    let sec: u64 = match t.next() {
        Some(v) => v.parse().map_err(|_| bad())?,
        None => 0,
    };
    if d.next().is_some()
        || t.next().is_some()
        || !(1970..=9999).contains(&y)
        || !(1..=12).contains(&mo)
        || !(1..=31).contains(&da)
        || h > 23
        || mi > 59
        || sec > 59
    {
        return Err(bad());
    }
    let days = days_from_civil(y, mo, da);
    if civil_from_days(days) != (y, mo, da) {
        return Err(bad()); // the 31st of a 30-day month
    }
    Ok(days as u64 * DAY + h * 3_600 + mi * 60 + sec)
}

/// `Thu 15 Oct 19:00 UTC` — what a player is told.
pub fn fmt_utc(unix: u64) -> String {
    const WD: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MO: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = (unix / DAY) as i64;
    let (_, m, d) = civil_from_days(days);
    let secs = unix % DAY;
    format!(
        "{} {d} {} {:02}:{:02} UTC",
        WD[(days % 7) as usize],
        MO[m as usize - 1],
        secs / 3_600,
        secs % 3_600 / 60
    )
}

/// `1 h`, `30 min`, `10 s`.
fn fmt_left(secs: u64) -> String {
    if secs >= 3_600 && secs.is_multiple_of(3_600) {
        format!("{} h", secs / 3_600)
    } else if secs >= 60 {
        format!("{} min", secs.div_ceil(60))
    } else {
        format!("{secs} s")
    }
}

/// Every `every`th wipe takes the blueprints. 0 = never by schedule.
pub fn blueprints_due(wipe_number: u32, every: u32) -> bool {
    every > 0 && wipe_number.is_multiple_of(every)
}

// ---- the state file --------------------------------------------------------

/// What the shard remembers between wipes: how many there have been (the
/// blueprint cadence counts them) and when the last one was (so a shard
/// that was down at its wipe hour applies it at the next boot). A text
/// file, `<world_file>.wipe` (or `<save_file>.wipe` on a shard with no
/// world file), so an operator can read and edit it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub wipes: u32,
    pub last: u64,
}

/// Where the state lives: beside the world file, else the player store.
pub fn state_path(world_file: Option<&str>, save_file: Option<&str>) -> Option<PathBuf> {
    world_file
        .or(save_file)
        .map(|f| PathBuf::from(format!("{f}.wipe")))
}

pub fn read_state(path: &Path) -> Result<Option<State>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("wipe state {}: {e}", path.display())),
    };
    let mut st = State::default();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let bad = || format!("wipe state {}: bad line `{line}`", path.display());
        let (k, v) = line.split_once('=').ok_or_else(bad)?;
        match k.trim() {
            "wipes" => st.wipes = v.trim().parse().map_err(|_| bad())?,
            "last" => st.last = v.trim().parse().map_err(|_| bad())?,
            _ => return Err(bad()),
        }
    }
    Ok(Some(st))
}

pub fn write_state(path: &Path, st: &State) -> std::io::Result<()> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    let text = format!(
        "# Gates wipe state, written by the shard at every wipe.\n\
         # last = the unix second the current world began.\n\
         wipes = {}\nlast = {}\n",
        st.wipes, st.last
    );
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_data()?;
    }
    std::fs::rename(&tmp, path)
}

// ---- the file operation ----------------------------------------------------

/// What a wipe did, for the boot log.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub archive: PathBuf,
    pub world_archived: bool,
    /// Player records rewritten carrying blueprints and glyphs.
    pub kept: usize,
    /// Player records cleared.
    pub cleared: usize,
}

/// What a player keeps across a map wipe: what they know, as a body that
/// logged off dead, so the restore wakes them on a beach with the spawn kit
/// (`World::seat` sends a dead record through `wake`).
pub fn survivor(save: &PlayerSave) -> PlayerSave {
    let mut s = PlayerSave::EMPTY;
    s.dead = true;
    s.known = save.known;
    s.glyphs = save.glyphs;
    s
}

/// End the world on disk. Every file must be closed (a boot before it
/// opens them, or `bin/shard.rs` after the shutdown flush).
pub fn apply(
    world_file: Option<&Path>,
    save_file: Option<&Path>,
    now: u64,
    blueprints: bool,
) -> Result<Report, String> {
    let base = world_file
        .or(save_file)
        .ok_or("a wipe needs a world_file or a save_file to act on")?;
    let archive = PathBuf::from(format!("{}.wipes/{now}", base.display()));
    std::fs::create_dir_all(&archive)
        .map_err(|e| format!("wipe archive {}: {e}", archive.display()))?;
    let mut report = Report {
        archive: archive.clone(),
        ..Report::default()
    };
    let into_archive = |p: &Path| -> PathBuf {
        archive.join(p.file_name().map(|n| n.to_os_string()).unwrap_or_default())
    };

    if let Some(world) = world_file {
        // The world, its backups, a half-written temp and the trust log
        // that was stamped with its ticks.
        for suffix in ["", ".1", ".2", ".3", ".tmp", ".trust"] {
            let p = PathBuf::from(format!("{}{suffix}", world.display()));
            if p.exists() {
                std::fs::rename(&p, into_archive(&p))
                    .map_err(|e| format!("wipe: archiving {}: {e}", p.display()))?;
                report.world_archived |= suffix.is_empty();
            }
        }
    }

    // The standings go with the world they ranked; the hall and last
    // wipe's placings stay (`standings::close_wipe` wrote them first).
    for p in [
        crate::standings::standings_path(base),
        crate::standings::json_path(base),
    ] {
        if p.exists() {
            std::fs::rename(&p, into_archive(&p))
                .map_err(|e| format!("wipe: archiving {}: {e}", p.display()))?;
        }
    }

    if let Some(save) = save_file.filter(|p| p.exists()) {
        std::fs::copy(save, into_archive(save))
            .map_err(|e| format!("wipe: archiving {}: {e}", save.display()))?;
        let (kept, cleared) = rewrite_store(save, blueprints)?;
        report.kept = kept;
        report.cleared = cleared;
    }
    Ok(report)
}

/// Rewrite every record of the player store: survivors in place, the rest
/// zeroed (a free slot). The header is left alone — same seed, same content.
fn rewrite_store(path: &Path, blueprints: bool) -> Result<(usize, usize), String> {
    let err = |e: std::io::Error| format!("wipe: player store {}: {e}", path.display());
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(err)?;
    let mut head = [0u8; SAVE_HEADER_BYTES];
    f.read_exact(&mut head).map_err(err)?;
    if head[0..8] != SAVE_MAGIC
        || u16::from_le_bytes([head[8], head[9]]) != SAVE_FORMAT
        || u16::from_le_bytes([head[10], head[11]]) as usize != SAVE_RECORD_BYTES
    {
        return Err(format!(
            "wipe: {} is not a player store this build writes — refusing to rewrite it \
             (the archive holds a copy)",
            path.display()
        ));
    }
    let (mut kept, mut cleared) = (0, 0);
    let mut rec = [0u8; SAVE_RECORD_BYTES];
    for index in 0..MAX_SAVED_PLAYERS {
        f.seek(SeekFrom::Start(record_offset(index))).map_err(err)?;
        if f.read_exact(&mut rec).is_err() {
            break; // a short file: nothing past here to rewrite
        }
        let out = match decode_record(&rec) {
            Ok(None) => continue,
            Ok(Some((key, stamp, save)))
                if !blueprints && (save.known != 0 || save.glyphs != 0) =>
            {
                let mut out = [0u8; SAVE_RECORD_BYTES];
                encode_record(&mut out, &key, stamp, &survivor(&save));
                kept += 1;
                out
            }
            _ => {
                cleared += 1;
                [0u8; SAVE_RECORD_BYTES]
            }
        };
        f.seek(SeekFrom::Start(record_offset(index))).map_err(err)?;
        f.write_all(&out).map_err(err)?;
    }
    f.sync_data().map_err(err)?;
    Ok((kept, cleared))
}

// ---- the countdown ---------------------------------------------------------

/// The wipe that is coming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    pub at: u64,
    pub blueprints: bool,
    /// Set by `/wipe`, not the schedule; `/wipe cancel` drops it.
    pub by_admin: bool,
}

/// What one poll of the clock says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tick {
    Quiet,
    /// Tell everyone.
    Say(String),
    /// The hour has come: say this and wipe.
    Wipe {
        blueprints: bool,
        line: String,
    },
}

/// The sim thread's wipe clock (`ShardCore::wipe`). Wall-clock seconds come
/// in from the boundary loop (`net.rs`), so this holds no clock of its own
/// and its tests drive it by hand.
#[derive(Clone, Debug)]
pub struct Clock {
    schedule: Option<Schedule>,
    blueprint_every: u32,
    /// Wipes before this one: the next is number `wipes + 1`.
    wipes: u32,
    pending: Option<Pending>,
    /// `WARN_AT[..warned]` have been said, or were past when set.
    warned: usize,
    /// An admin just set the wipe: say it now, however far out it is.
    say_now: bool,
    fired: bool,
}

impl Clock {
    /// No schedule and nothing pending: `/wipe` can still start one.
    pub fn off() -> Self {
        Self {
            schedule: None,
            blueprint_every: 0,
            wipes: 0,
            pending: None,
            warned: 0,
            say_now: false,
            fired: false,
        }
    }

    pub fn new(schedule: Option<Schedule>, blueprint_every: u32, wipes: u32, now: u64) -> Self {
        let mut c = Self {
            schedule,
            blueprint_every,
            wipes,
            ..Self::off()
        };
        c.reschedule(now);
        c
    }

    fn reschedule(&mut self, now: u64) {
        self.pending = self.schedule.map(|s| Pending {
            at: s.next_after(now),
            blueprints: blueprints_due(self.wipes + 1, self.blueprint_every),
            by_admin: false,
        });
        self.warned = 0;
    }

    pub fn next(&self) -> Option<Pending> {
        self.pending
    }

    /// The number of the wipe now running: the wipes before it, plus one.
    pub fn number(&self) -> u32 {
        self.wipes + 1
    }

    /// `/wipe <minutes> [bp]`: the next wipe is `minutes` from now. The
    /// blueprint cadence still applies; `bp` forces a blueprint wipe.
    pub fn admin(&mut self, now: u64, minutes: u16, force_bp: bool) {
        self.pending = Some(Pending {
            at: now + minutes as u64 * 60,
            blueprints: force_bp || blueprints_due(self.wipes + 1, self.blueprint_every),
            by_admin: true,
        });
        self.warned = 0;
        self.say_now = true;
    }

    /// `/wipe cancel`: drop an admin's wipe, back to the schedule. False ⇒
    /// nothing an admin started was pending (the schedule is not cancelled
    /// by a chat line; that is `shard.toml`'s).
    pub fn cancel(&mut self, now: u64) -> bool {
        if !self.pending.is_some_and(|p| p.by_admin) || self.fired {
            return false;
        }
        self.reschedule(now);
        self.say_now = false;
        true
    }

    pub fn poll(&mut self, now: u64) -> Tick {
        let Some(p) = self.pending else {
            return Tick::Quiet;
        };
        if self.fired {
            return Tick::Quiet;
        }
        let bp = if p.blueprints {
            "blueprints too"
        } else {
            "blueprints kept"
        };
        if now >= p.at {
            self.fired = true;
            return Tick::Wipe {
                blueprints: p.blueprints,
                line: "WIPING NOW. See you on the new island".into(),
            };
        }
        let left = p.at - now;
        if self.say_now || (self.warned < WARN_AT.len() && left <= WARN_AT[self.warned]) {
            self.say_now = false;
            while self.warned < WARN_AT.len() && WARN_AT[self.warned] >= left {
                self.warned += 1;
            }
            return Tick::Say(format!("WIPE in {} ({bp})", fmt_left(left)));
        }
        Tick::Quiet
    }

    /// The line a joining player is told, if a wipe is coming.
    pub fn notice(&self) -> Option<String> {
        self.pending.map(|p| {
            format!(
                "next wipe {}{}",
                fmt_utc(p.at),
                if p.blueprints { " +BP" } else { "" }
            )
        })
    }
}

/// A clock line as chat. `None` only for a line the chat lane refuses,
/// which none of the above can be.
pub fn chat(line: &str) -> Option<ChatText> {
    ChatText::sanitize(line.as_bytes()).map(|t| crate::admin::server_line(&t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_anchor_is_thursday_evening() {
        assert_eq!(fmt_utc(DEFAULT_ANCHOR), "Thu 1 Jan 19:00 UTC");
        let s = Schedule::days(7, DEFAULT_ANCHOR);
        // 2026-10-07 is a Wednesday; the next wipe is Thursday the 8th.
        let now = parse_anchor("2026-10-07 12:00").unwrap();
        assert_eq!(fmt_utc(s.next_after(now)), "Thu 8 Oct 19:00 UTC");
        assert_eq!(s.next_after(s.next_after(now)) - s.next_after(now), 7 * DAY);
        assert_eq!(
            s.last_at_or_before(now),
            Some(parse_anchor("2026-10-01T19:00Z").unwrap())
        );
    }

    #[test]
    fn anchors_parse_and_refuse() {
        assert_eq!(parse_anchor("1970-01-01 00:00"), Ok(0));
        assert_eq!(parse_anchor("2000-03-01T00:00:00Z"), Ok(951_868_800));
        for bad in [
            "2026-02-30 10:00",
            "2026-13-01 10:00",
            "2026-10-01",
            "x",
            "2026-10-01 24:00",
        ] {
            assert!(parse_anchor(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn every_other_wipe_takes_the_blueprints() {
        assert!(!blueprints_due(1, 2));
        assert!(blueprints_due(2, 2));
        assert!(!blueprints_due(3, 2));
        assert!(!blueprints_due(2, 0), "0 never wipes blueprints");
        assert!(blueprints_due(5, 1));
    }

    /// The countdown says the first warning at once, each threshold once,
    /// then fires exactly once.
    #[test]
    fn the_countdown_warns_then_fires_once() {
        let mut c = Clock::off();
        assert_eq!(c.poll(1_000), Tick::Quiet);
        c.admin(1_000, 5, false);
        assert_eq!(
            c.poll(1_000),
            Tick::Say("WIPE in 5 min (blueprints kept)".into())
        );
        assert_eq!(c.poll(1_001), Tick::Quiet);
        assert_eq!(
            c.poll(1_180),
            Tick::Say("WIPE in 2 min (blueprints kept)".into())
        );
        assert_eq!(c.poll(1_181), Tick::Quiet);
        assert!(matches!(c.poll(1_240), Tick::Say(_)));
        assert!(matches!(c.poll(1_270), Tick::Say(_)));
        assert!(matches!(c.poll(1_290), Tick::Say(_)));
        assert!(matches!(c.poll(1_297), Tick::Say(_)));
        assert!(matches!(
            c.poll(1_300),
            Tick::Wipe {
                blueprints: false,
                ..
            }
        ));
        assert_eq!(c.poll(1_400), Tick::Quiet, "a wipe fires once");
    }

    #[test]
    fn cancel_returns_to_the_schedule() {
        let s = Schedule::days(7, DEFAULT_ANCHOR);
        let now = parse_anchor("2026-10-07 12:00").unwrap();
        let mut c = Clock::new(Some(s), 2, 1, now);
        let scheduled = c.next().unwrap();
        assert!(scheduled.blueprints, "wipe 2 of a cadence of 2");
        assert!(
            !c.cancel(now),
            "the schedule is not a chat line's to cancel"
        );
        c.admin(now, 10, false);
        assert_eq!(c.next().unwrap().at, now + 600);
        assert!(c.cancel(now));
        assert_eq!(c.next(), Some(scheduled));
        assert_eq!(c.poll(now), Tick::Quiet, "a day out says nothing");
        c.admin(now, 120, false);
        assert_eq!(
            c.poll(now),
            Tick::Say("WIPE in 2 h (blueprints too)".into()),
            "said as typed"
        );
        assert_eq!(c.poll(now + 1), Tick::Quiet);
    }

    /// Every line the clock says fits the chat lane with its prefix.
    #[test]
    fn every_line_fits_a_chat_line() {
        let mut c = Clock::new(Some(Schedule::days(14, DEFAULT_ANCHOR)), 1, 9, 0);
        let n = c.notice().unwrap();
        assert!(
            chat(&n).is_some_and(|t| t.as_bytes().ends_with(b"+BP")),
            "{n}"
        );
        c.admin(0, 59, true);
        let Tick::Say(l) = c.poll(1) else { panic!() };
        assert!(chat(&l).unwrap().as_bytes().ends_with(b")"), "{l}");
        let Tick::Wipe { line, .. } = c.poll(59 * 60) else {
            panic!()
        };
        assert!(
            chat(&line).unwrap().as_bytes().ends_with(b"island"),
            "{line}"
        );
    }

    #[test]
    fn a_survivor_keeps_what_they_know_and_wakes_on_a_beach() {
        let mut s = PlayerSave::EMPTY;
        s.known = 0b1011;
        s.glyphs = 0b11;
        s.hp = 40;
        s.hp_max = 100;
        s.inv[0].item = 3;
        s.inv[0].count = 9;
        let w = survivor(&s);
        assert!(w.dead);
        assert_eq!((w.known, w.glyphs), (0b1011, 0b11));
        assert_eq!(w.inv[0].count, 0);
        let mut b = [0u8; sim_core::persist::PLAYER_SAVE_BYTES];
        w.write_le(&mut b);
        assert_eq!(
            PlayerSave::read_le(&b),
            Ok(w),
            "a survivor is a legal record"
        );
    }
}
