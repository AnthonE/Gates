//! Who is on this shard right now, for the platform: `<base>.online.json`,
//! rewritten once a minute and served by the status thread at
//! `/online.json`. Elo Pros polls it for a player's "playing now" and their
//! hours on record, the way Steam shows both.
//!
//! ```json
//! {"at":1760470000,"online":[{"wallet":"0xab…","name":"Ash","agent":false,"since":1760466400}]}
//! ```
//!
//! - `at` — unix seconds the file was written. A reader treats an old file
//!   as a shard that stopped saying anything.
//! - `since` — unix seconds this wallet came online, as this writer saw it
//!   (to the minute). A restart starts everyone's session again.
//!
//! Wallets only: a guest has no profile to show it on. The sim thread hands
//! over the roster and nothing else; the writer thread owns the clock, so
//! no wall time enters the tick.
//!
//! ## The weekly check-in
//!
//! The same writer keeps `<base>.checkins-<week>.json`: every wallet that
//! stood in the town's safe zone, alive and awake, at a minute's roster this
//! week (weeks start Monday 00:00 UTC, [`week_of`]). Elo Pros reads it at
//! `/checkins/<week>.json` and pays the week's stake rewards only to wallets
//! on it, so the rewards need the island, not a fork of it. Reaching town in
//! a PvP world is the whole test.
//!
//! ```json
//! {"week":2917,"starts":1763942400,"ends":1764547200,
//!  "checked_in":[{"wallet":"0xab…","first":1763990000,"agent":false}]}
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One connected player, as the platform is told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Who {
    /// Lowercase `0x…` hex.
    pub wallet: String,
    /// What the platform calls them, empty for not yet known.
    pub name: String,
    pub agent: bool,
    /// In the town's safe zone this minute, alive and awake.
    pub safe: bool,
}

/// Rosters queued for the writer. One a minute is written in well under a
/// second, so a backlog means the disk is stuck; the newest is dropped.
const ONLINE_RING: usize = 4;

/// The most of the file the status thread will serve.
pub const ONLINE_JSON_CAP: u64 = 64 * 1024;

pub fn json_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.online.json", base.display()))
}

/// The most of a week's check-in file the status thread will serve.
pub const CHECKINS_JSON_CAP: u64 = 4 * 1024 * 1024;

const WEEK_S: u64 = 7 * 86_400;
/// 1970-01-01 was a Thursday; three days on, weeks start on Monday.
const WEEK_SHIFT_S: u64 = 3 * 86_400;

/// The check-in week holding unix second `now`.
pub fn week_of(now: u64) -> u64 {
    (now + WEEK_SHIFT_S) / WEEK_S
}

/// Unix second `week` starts (Monday 00:00 UTC).
pub fn week_start(week: u64) -> u64 {
    (week * WEEK_S).saturating_sub(WEEK_SHIFT_S)
}

pub fn checkins_path(base: &Path, week: u64) -> PathBuf {
    PathBuf::from(format!("{}.checkins-{week}.json", base.display()))
}

/// One week's check-ins: wallet → (first unix second seen in town, agent).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Checkins {
    pub week: u64,
    pub wallets: BTreeMap<String, (u64, bool)>,
}

impl Checkins {
    /// The week's set as the file left it, so a restart keeps it.
    pub fn load(base: &Path, week: u64) -> Self {
        let mut c = Checkins {
            week,
            wallets: BTreeMap::new(),
        };
        let text = std::fs::read_to_string(checkins_path(base, week)).unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        for row in v["checked_in"].as_array().into_iter().flatten() {
            if let (Some(w), Some(first)) = (row["wallet"].as_str(), row["first"].as_u64()) {
                let agent = row["agent"].as_bool().unwrap_or(false);
                c.wallets.insert(w.to_string(), (first, agent));
            }
        }
        c
    }

    /// Add this minute's roster. True when the set grew.
    pub fn note(&mut self, now: u64, roster: &[Who]) -> bool {
        let mut grew = false;
        for w in roster.iter().filter(|w| w.safe) {
            if !self.wallets.contains_key(&w.wallet) {
                self.wallets.insert(w.wallet.clone(), (now, w.agent));
                grew = true;
            }
        }
        grew
    }

    pub fn render(&self) -> String {
        let rows: Vec<serde_json::Value> = self
            .wallets
            .iter()
            .map(|(w, (first, agent))| serde_json::json!({"wallet": w, "first": first, "agent": agent}))
            .collect();
        serde_json::json!({
            "week": self.week,
            "starts": week_start(self.week),
            "ends": week_start(self.week + 1),
            "checked_in": rows,
        })
        .to_string()
    }
}

/// The sim side's end: a ring into the writer thread, or nothing when the
/// shard has no world file to write beside.
pub struct Online {
    out: Option<rtrb::Producer<Vec<Who>>>,
}

impl Online {
    pub fn off() -> Self {
        Online { out: None }
    }

    /// Start the writer for `<base>.online.json`.
    pub fn open(base: Option<&Path>) -> Self {
        let Some(base) = base else {
            return Self::off();
        };
        let path = json_path(base);
        let base = base.to_path_buf();
        let (tx, mut rx) = rtrb::RingBuffer::<Vec<Who>>::new(ONLINE_RING);
        let spawned = std::thread::Builder::new()
            .name("online".into())
            .spawn(move || {
                let mut since = BTreeMap::new();
                let mut checkins: Option<Checkins> = None;
                loop {
                    match rx.pop() {
                        Ok(roster) => {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            let text = render(now, &roster, &mut since);
                            if let Err(e) = write_atomic(&path, &text) {
                                eprintln!("online {}: {e}", path.display());
                            }
                            let week = week_of(now);
                            let c = match checkins.as_mut() {
                                Some(c) if c.week == week => c,
                                _ => checkins.insert(Checkins::load(&base, week)),
                            };
                            if c.note(now, &roster) {
                                let p = checkins_path(&base, week);
                                if let Err(e) = write_atomic(&p, &c.render()) {
                                    eprintln!("checkins {}: {e}", p.display());
                                }
                            }
                        }
                        Err(_) if rx.is_abandoned() => return,
                        Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
                    }
                }
            });
        match spawned {
            Ok(_) => Online { out: Some(tx) },
            Err(_) => Self::off(),
        }
    }

    /// Hand this minute's roster to the writer.
    pub fn publish(&mut self, roster: Vec<Who>) {
        if let Some(out) = self.out.as_mut() {
            let _ = out.push(roster);
        }
    }
}

/// The file for `roster` at `now`. `since` carries each wallet's first
/// sighting from one roster to the next; a wallet that left is forgotten,
/// so coming back starts a new session.
pub fn render(now: u64, roster: &[Who], since: &mut BTreeMap<String, u64>) -> String {
    let mut next = BTreeMap::new();
    let online: Vec<serde_json::Value> = roster
        .iter()
        .map(|w| {
            let at = *since.get(&w.wallet).unwrap_or(&now);
            next.insert(w.wallet.clone(), at);
            serde_json::json!({
                "wallet": w.wallet,
                "name": w.name,
                "agent": w.agent,
                "since": at,
            })
        })
        .collect();
    *since = next;
    serde_json::json!({ "at": now, "online": online }).to_string()
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(w: &str, name: &str) -> Who {
        Who {
            wallet: w.into(),
            name: name.into(),
            agent: false,
            safe: false,
        }
    }

    #[test]
    fn weeks_start_on_monday_utc() {
        // 2026-10-26 00:00 UTC is a Monday
        let monday = 1_792_972_800;
        assert_eq!(week_start(week_of(monday)), monday);
        assert_eq!(week_of(monday - 1) + 1, week_of(monday));
        assert_eq!(week_of(monday + 7 * 86_400 - 1), week_of(monday));
    }

    /// Only a wallet standing in town counts, once a week, at its first
    /// sighting; a restart reloads the week from the file.
    #[test]
    fn a_wallet_checks_in_once_a_week_from_town_and_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("gates-checkins-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("world");
        let a = "0x00000000000000000000000000000000000000aa";
        let b = "0x00000000000000000000000000000000000000bb";
        let mut in_town = who(a, "Ash");
        in_town.safe = true;

        let mut c = Checkins::load(&base, 7);
        assert!(
            !c.note(100, &[who(a, "Ash"), who(b, "")]),
            "out in the wild"
        );
        assert!(c.note(160, &[in_town.clone(), who(b, "")]));
        assert!(!c.note(220, &[in_town]), "already in this week");
        std::fs::write(checkins_path(&base, 7), c.render()).unwrap();

        let back = Checkins::load(&base, 7);
        assert_eq!(back, c);
        assert_eq!(back.wallets[a], (160, false));
        assert!(!back.wallets.contains_key(b));
        let v: serde_json::Value = serde_json::from_str(&back.render()).unwrap();
        assert_eq!(v["starts"], week_start(7));
        assert_eq!(v["checked_in"][0]["wallet"], a);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A session starts at the first roster that has the wallet and holds
    /// while it stays; leaving and coming back is a new session.
    #[test]
    fn a_session_starts_when_a_wallet_appears_and_ends_when_it_leaves() {
        let mut since = BTreeMap::new();
        let a = "0x00000000000000000000000000000000000000aa";
        let b = "0x00000000000000000000000000000000000000bb";

        let v: serde_json::Value =
            serde_json::from_str(&render(100, &[who(a, "Ash")], &mut since)).unwrap();
        assert_eq!(v["at"], 100);
        assert_eq!(v["online"][0]["wallet"], a);
        assert_eq!(v["online"][0]["name"], "Ash");
        assert_eq!(v["online"][0]["since"], 100);

        let v: serde_json::Value =
            serde_json::from_str(&render(160, &[who(a, "Ash"), who(b, "")], &mut since)).unwrap();
        assert_eq!(v["online"][0]["since"], 100, "still the same session");
        assert_eq!(v["online"][1]["since"], 160, "a new arrival starts now");

        render(220, &[who(b, "")], &mut since);
        let v: serde_json::Value =
            serde_json::from_str(&render(280, &[who(a, "Ash"), who(b, "")], &mut since)).unwrap();
        assert_eq!(
            v["online"][0]["since"], 280,
            "back after leaving: a new session"
        );
        assert_eq!(v["online"][1]["since"], 160);
    }

    #[test]
    fn a_name_is_escaped() {
        let mut since = BTreeMap::new();
        let text = render(1, &[who("0xaa", "a\"b\\c")], &mut since);
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(v["online"][0]["name"], "a\"b\\c");
    }
}
