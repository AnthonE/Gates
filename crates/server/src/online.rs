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
}

/// Rosters queued for the writer. One a minute is written in well under a
/// second, so a backlog means the disk is stuck; the newest is dropped.
const ONLINE_RING: usize = 4;

/// The most of the file the status thread will serve.
pub const ONLINE_JSON_CAP: u64 = 64 * 1024;

pub fn json_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.online.json", base.display()))
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
        let (tx, mut rx) = rtrb::RingBuffer::<Vec<Who>>::new(ONLINE_RING);
        let spawned = std::thread::Builder::new()
            .name("online".into())
            .spawn(move || {
                let mut since = BTreeMap::new();
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
        }
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
