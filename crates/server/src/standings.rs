//! **The standings** — who did the most with this wipe, ranked four ways,
//! and the hall that remembers it across wipes.
//!
//! Rust sheds most of a server by day two because a player whose base fell
//! has nothing left to play for until the next wipe. Here there is always a
//! board to climb, and one of them cannot be raided:
//!
//! - **THE WORKS** — what you gave the island's works (`ARC.md` F3), valued
//!   at what the goods are worth, plus the deeds: the deposit that lit a work
//!   (worth more the later its act), fuel into a dry tank, a lock solved.
//!   Given is given: a raid cannot take it back.
//! - **THE HOARD** — what your bases hold now: box contents, the hearth's
//!   stock and the deployables inside the claim, split across its crew.
//!   Raidable, recounted every minute, and **frozen at the wipe**, so the
//!   last hour of a wipe is a fight over what counts.
//! - **THE FIGHT** — players killed.
//! - **THE ISLAND** — all of it on one scale (`content/arc.toml`
//!   `[standings]`): every score is farm-minutes of goods, the balance math's
//!   currency (`content::balance::worth_minutes`).
//!
//! At the wipe each board's top five go into the hall (`<world>.hall`) for
//! good, and everyone's island placing into `<world>.lastwipe`, so the next
//! wipe opens by naming last wipe's champions and telling each returning
//! player where they finished.
//!
//! ## Prizes, to the wallet you log in with
//!
//! A shard may pay each board's top places in a coin (`shard.toml`'s
//! `prize_*` keys, [`Prizes`]); absent, it pays nothing. Players log in with
//! their wallet, so every row is already keyed by the address to pay. A
//! place is counted among the **eligible**: a real wallet (not a guest or a
//! dev key), at least `prize_min_minutes` played this wipe, and not a
//! declared agent unless `prize_agents` says agents compete for money too.
//! The STANDINGS page shows each board's purse, what you would take if the
//! wipe ended now, and how long you still need to play to qualify.
//!
//! At the wipe [`close_wipe`] writes `<world>.payout-<wipe>.json` and `.csv`
//! (one row per wallet, totals) beside the world. **Sending is the
//! operator's** (`CLAUDE.md`: anything on-chain): the shard holds no keys
//! and mints nothing; it says who earned what.
//!
//! ## Extraction: the other road out
//!
//! Carried JUNK put through THE EXCHANGE (`sim_core::works::extract`) is
//! credited here to the wallet, less `extract_fee_pct`, up to `extract_cap`
//! a wipe (more while the exchange burns: `KNOB_EXTRACT_PCT`). It lands in
//! the same payout file as the prizes. A shard with no `extract_cap` lets
//! nothing leave.
//!
//! The file is written only after the world save it was taken with has
//! landed ([`Standings::snapshot`], [`Standings::release`]): a crash rolls
//! both back together, so a credit never outlives the world that still
//! has the coin in a pack.
//!
//! ## Where this runs
//!
//! Server-side only: nothing here is sim state or hashed. The sim's events
//! (`EV_GAVE`, `EV_WORK`, `EV_MECH_SOLVED`, `EV_DEATH`) are tallied per
//! **wallet** by `ShardCore`, because a player id is minted per connection.
//! The file is rewritten once a minute by a writer thread fed through a ring
//! (`<world>.standings`, plus `.standings.json` for the status endpoint), and
//! once more, synchronously, at shutdown, after a last count of the hoards.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The boards, in wire order.
pub const BOARDS: usize = 4;
pub const BOARD_ISLAND: u8 = 0;
pub const BOARD_WORKS: u8 = 1;
pub const BOARD_HOARD: u8 = 2;
pub const BOARD_FIGHT: u8 = 3;
/// Last wipe's island, sent after the four.
pub const BOARD_LAST: u8 = 4;
pub const BOARD_NAMES: [&str; BOARDS + 1] = [
    "THE ISLAND",
    "THE WORKS",
    "THE HOARD",
    "THE FIGHT",
    "LAST WIPE",
];

/// Rows a board shows, and the podium the hall keeps.
pub const TOP: usize = 5;
/// Wallets one wipe ranks: the player store's own capacity. A full table
/// ranks nobody new, counted in [`Standings::refused`].
pub const MAX_ROWS: usize = crate::store::MAX_SAVED_PLAYERS;
/// Deeds a wipe remembers by name, for the hall.
pub const MAX_DEEDS: usize = 64;
/// Longest label: the wire's name.
pub const LABEL_MAX: usize = protocol::NAME_MAX_BYTES;
/// Saves waiting for the writer. One is all that matters: the newest.
const FLUSH_RING: usize = 2;
/// A row line's fields: `row key given deeds kills deaths hoard island
/// played flags extracted taken label`, the label last because it may hold
/// spaces.
const ROW_FIELDS: usize = 13;
/// The coin extraction pays in (`content::bake::EXTRACT_COIN`).
pub const EXTRACT_TICKER: &str = "JUNK";
/// `flags`: joined as a declared agent.
const FLAG_AGENT: u64 = 1;

/// What the standings weigh, baked from `content/arc.toml` `[standings]`.
#[derive(Clone, Debug, Default)]
pub struct Rules {
    pub given_pct: u64,
    pub lit_points: u64,
    pub rekindled_points: u64,
    pub solved_points: u64,
    pub kill_points: u64,
    pub hoard_pct: u64,
    /// Hundredths of a farm-minute a unit, by sim item index.
    pub worth: Vec<u32>,
}

impl Rules {
    /// `worth` is `Content::bake_worth`'s table.
    pub fn bake_from(c: &content::Content, worth: Vec<u32>) -> Self {
        let s = &c.standings;
        Rules {
            given_pct: s.given_pct as u64,
            lit_points: s.lit_points as u64,
            rekindled_points: s.rekindled_points as u64,
            solved_points: s.solved_points as u64,
            kill_points: s.kill_points as u64,
            hoard_pct: s.hoard_pct as u64,
            worth,
        }
    }

    /// Hundredths of a farm-minute, `units` of `item`.
    pub fn worth(&self, item: u16, units: u32) -> u64 {
        self.worth.get(item as usize).copied().unwrap_or(0) as u64 * units as u64
    }
}

/// What a shard pays at the wipe (`shard.toml` `prize_*`): an amount of
/// `ticker` for each place on each board, to the wallet that took it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prizes {
    /// The coin, bare: JUNK or ELO (`BUSINESS.md`). Never ORBS: new ORBS comes
    /// only from the elo farm, and `config.rs` refuses it. Empty: no prizes.
    pub ticker: String,
    /// Amount by place (index 0 is first), per board in wire order.
    pub places: [Vec<u64>; BOARDS],
    /// Minutes a wallet must have played this wipe to be paid.
    pub min_minutes: u32,
    /// Whether a declared agent may be paid.
    pub agents: bool,
}

impl Prizes {
    /// Whether this shard pays anything at all.
    pub fn armed(&self) -> bool {
        !self.ticker.is_empty() && self.places.iter().any(|p| p.iter().any(|&a| a > 0))
    }

    /// `"500,300,200"` → the places, best first, at most [`TOP`].
    pub fn parse_places(s: &str) -> Result<Vec<u64>, String> {
        let places: Vec<u64> = s
            .split(',')
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(|v| {
                v.parse::<u64>()
                    .map_err(|_| format!("`{v}` is not a whole amount"))
            })
            .collect::<Result<_, _>>()?;
        if places.len() > TOP {
            return Err(format!("{} places; a board shows {TOP}", places.len()));
        }
        Ok(places)
    }

    /// A ticker as `BUSINESS.md` writes one: 2–8 capitals, no `$`.
    pub fn valid_ticker(t: &str) -> bool {
        (2..=8).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_uppercase())
    }

    /// Whether `key` may be paid, on what it did this wipe.
    pub fn eligible(&self, key: &str, played: u32, agent: bool) -> bool {
        is_wallet(key) && played >= self.min_minutes && (self.agents || !agent)
    }

    /// What place `place` (1-based) on `board` pays.
    pub fn amount(&self, board: u8, place: u32) -> u64 {
        self.places
            .get(board as usize)
            .and_then(|p| p.get(place.checked_sub(1)? as usize))
            .copied()
            .unwrap_or(0)
    }
}

/// What may leave through the exchange (`shard.toml` `extract_cap`,
/// `extract_fee_pct`, `extract_min_minutes`). A zero cap lets nothing out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exits {
    /// JUNK a wallet may take out a wipe, before the exchange's ceiling.
    pub cap: u64,
    /// Per cent of what goes through that is burnt, never credited.
    pub fee_pct: u64,
    /// Minutes a wallet must have played this wipe for what it extracted to
    /// reach the payout file. Zero pays every wallet. A wallet that only
    /// carries someone else's Junk to the exchange plays a few minutes; the
    /// cap is per wallet, so without this a farm of fresh wallets multiplies it.
    pub min_minutes: u32,
}

/// A key that is a real wallet: `0x` and forty hex digits, not the guest.
pub fn is_wallet(key: &str) -> bool {
    protocol::Address::from_hex(key.as_bytes()).is_some_and(|a| !a.is_guest())
}

/// One wallet's wipe. Hundredths of a point where it says so.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    /// What they gave the works, weighted by `given_pct`. Hundredths.
    pub given: u64,
    /// Lit, rekindled, solved. Hundredths.
    pub deeds: u64,
    pub kills: u32,
    pub deaths: u32,
    /// Their share of what their bases held at the last count. Hundredths.
    pub hoard: u64,
    /// Minutes played this wipe (counted once a minute while connected).
    pub played: u32,
    /// Ever joined as a declared agent this wipe.
    pub agent: bool,
    /// JUNK credited through the exchange, after the fee.
    pub extracted: u64,
    /// JUNK put through the exchange, before the fee: what the cap counts.
    pub taken: u64,
}

impl Row {
    /// The row's score on `board`, whole points (kills on THE FIGHT).
    pub fn score(&self, board: u8, r: &Rules) -> u64 {
        match board {
            BOARD_WORKS => (self.given + self.deeds) / 100,
            BOARD_HOARD => self.hoard / 100,
            BOARD_FIGHT => self.kills as u64,
            _ => {
                (self.given + self.deeds + self.hoard * r.hoard_pct / 100) / 100
                    + self.kills as u64 * r.kill_points
            }
        }
    }
}

/// One board, built: its top rows and where everyone stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Board {
    /// `(label, score)`, best first. What the wire carries (`SUB_STANDING`).
    pub top: Vec<(String, u64)>,
    /// The wallet key behind each `top` row, same order: what
    /// `/standings.json` names a row's wallet and agent by. Never on the wire.
    pub top_keys: Vec<String>,
    /// Wallets with a score above zero.
    pub ranked: u32,
    ranks: BTreeMap<String, (u32, u64)>,
    /// Place among those who may be paid (1-based), for the eligible only.
    places: BTreeMap<String, u32>,
}

impl Board {
    /// Rank (1-based) and score of `key`, if it scored.
    pub fn of(&self, key: &str) -> Option<(u32, u64)> {
        self.ranks.get(key).copied()
    }

    /// `key`'s place among the eligible (1-based): what a purse pays on.
    pub fn place_of(&self, key: &str) -> Option<u32> {
        self.places.get(key).copied()
    }

    /// Rank `(key, label, score, eligible)` rows: best first, ties to the
    /// earlier key. The eligible are also placed among themselves.
    fn rank(mut rows: Vec<(&str, &str, u64, bool)>) -> Board {
        rows.retain(|r| r.2 > 0);
        rows.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(b.0)));
        let mut b = Board {
            ranked: rows.len() as u32,
            ..Board::default()
        };
        let mut place = 0;
        for (i, (key, label, score, eligible)) in rows.into_iter().enumerate() {
            if i < TOP {
                b.top.push((label.to_string(), score));
                b.top_keys.push(key.to_string());
            }
            b.ranks.insert(key.to_string(), (i as u32 + 1, score));
            if eligible {
                place += 1;
                b.places.insert(key.to_string(), place);
            }
        }
        b
    }
}

/// What a wallet is called when the platform has not said: its short
/// address (`0x12ab..cdef`), or a dev key cut to the wire's length.
pub fn short_label(key: &str) -> String {
    if key.len() == 42 && key.starts_with("0x") {
        format!("{}..{}", &key[..6], &key[38..])
    } else {
        key.chars().take(LABEL_MAX).collect()
    }
}

/// A key the file can hold: printable, no spaces, within the store's cap.
fn filable(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= crate::store::PLAYER_KEY_MAX_BYTES
        && key.bytes().all(|b| b.is_ascii_graphic())
}

/// A label the wire can carry: printable ASCII, cut to its length.
fn clean_label(label: &str) -> String {
    label
        .chars()
        .filter(|c| (' '..='~').contains(c))
        .take(LABEL_MAX)
        .collect::<String>()
        .trim()
        .to_string()
}

// ---- the hall ----------------------------------------------------------------

/// One podium place, kept for good.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HallLine {
    pub wipe: u32,
    pub board: u8,
    pub rank: u32,
    pub score: u64,
    pub key: String,
    pub label: String,
}

/// What earlier wipes left: every podium, and last wipe's island in full.
#[derive(Clone, Debug, Default)]
pub struct Hall {
    pub lines: Vec<HallLine>,
    /// The wipe `last` is from, 0 for none.
    pub last_wipe: u32,
    /// Last wipe's island: key → (rank, score).
    pub last: BTreeMap<String, (u32, u64)>,
    /// How many were ranked on it.
    pub last_of: u32,
    /// What each wallet was paid last wipe, all boards, and in what.
    pub last_paid: BTreeMap<String, u64>,
    pub last_ticker: String,
    /// What each wallet took out through the exchange last wipe.
    pub last_extracted: BTreeMap<String, u64>,
    /// `(wipe, key)` for every podium wallet that played that wipe as a
    /// declared agent. Wipes closed before the hall kept this read as human.
    pub agents: BTreeSet<(u32, String)>,
}

impl Hall {
    pub fn load(base: &Path) -> Hall {
        let mut h = Hall::default();
        let mut prizes: Vec<(u32, u64, String, String)> = Vec::new();
        let mut extracts: Vec<(u32, u64, String)> = Vec::new();
        if let Ok(text) = std::fs::read_to_string(hall_path(base)) {
            for line in text.lines() {
                let fields: Vec<&str> = line.split('\t').collect();
                if let ["agent", w, key] = fields.as_slice() {
                    if let Ok(w) = w.parse::<u32>() {
                        h.agents.insert((w, key.to_string()));
                    }
                    continue;
                }
                if let ["extract", w, amount, key] = fields.as_slice() {
                    extracts.push((
                        w.parse::<u32>().unwrap_or(0),
                        amount.parse::<u64>().unwrap_or(0),
                        key.to_string(),
                    ));
                    continue;
                }
                if let ["prize", w, _, _, amount, ticker, key] = fields.as_slice() {
                    prizes.push((
                        w.parse::<u32>().unwrap_or(0),
                        amount.parse::<u64>().unwrap_or(0),
                        ticker.to_string(),
                        key.to_string(),
                    ));
                    continue;
                }
                let mut f = line.splitn(7, '\t');
                let (Some("podium"), Some(w), Some(b), Some(r), Some(s), Some(k)) =
                    (f.next(), f.next(), f.next(), f.next(), f.next(), f.next())
                else {
                    continue;
                };
                let (Ok(wipe), Ok(board), Ok(rank), Ok(score)) =
                    (w.parse(), b.parse(), r.parse(), s.parse())
                else {
                    continue;
                };
                h.lines.push(HallLine {
                    wipe,
                    board,
                    rank,
                    score,
                    key: k.to_string(),
                    label: f.next().unwrap_or("").to_string(),
                });
            }
        }
        if let Ok(text) = std::fs::read_to_string(last_path(base)) {
            for line in text.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                match f.as_slice() {
                    ["wipe", w, of] => {
                        h.last_wipe = w.parse().unwrap_or(0);
                        h.last_of = of.parse().unwrap_or(0);
                    }
                    ["rank", k, r, s] => {
                        if let (Ok(r), Ok(s)) = (r.parse(), s.parse()) {
                            h.last.insert(k.to_string(), (r, s));
                        }
                    }
                    _ => {}
                }
            }
        }
        for (wipe, amount, ticker, key) in prizes {
            if wipe == h.last_wipe && amount > 0 {
                *h.last_paid.entry(key).or_insert(0) += amount;
                h.last_ticker = ticker;
            }
        }
        for (wipe, amount, key) in extracts {
            if wipe == h.last_wipe && amount > 0 {
                *h.last_extracted.entry(key).or_insert(0) += amount;
            }
        }
        h
    }

    /// Last wipe's island podium, as a board.
    pub fn last_board(&self) -> Board {
        let mut b = Board {
            ranked: self.last_of,
            ..Board::default()
        };
        let mut podium: Vec<&HallLine> = self
            .lines
            .iter()
            .filter(|l| l.wipe == self.last_wipe && l.board == BOARD_ISLAND)
            .collect();
        podium.sort_by_key(|l| l.rank);
        for l in podium.into_iter().take(TOP) {
            b.top.push((l.label.clone(), l.score));
            b.top_keys.push(l.key.clone());
        }
        b.ranks = self.last.clone();
        b
    }

    /// Whether `key` played wipe `wipe` as a declared agent, as the hall
    /// remembers its podiums.
    pub fn was_agent(&self, wipe: u32, key: &str) -> bool {
        self.agents.contains(&(wipe, key.to_string()))
    }

    /// The newest `wipes` wipes' podiums (`rank <= 3`), newest first, then
    /// by board and place: what `/standings.json` shows as the hall.
    pub fn podiums(&self, wipes: usize) -> Vec<&HallLine> {
        let newest: BTreeSet<u32> = self
            .lines
            .iter()
            .map(|l| l.wipe)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .rev()
            .take(wipes)
            .collect();
        let mut out: Vec<&HallLine> = self
            .lines
            .iter()
            .filter(|l| newest.contains(&l.wipe) && l.rank <= 3 && (l.board as usize) < BOARDS)
            .collect();
        out.sort_by(|a, b| {
            b.wipe
                .cmp(&a.wipe)
                .then(a.board.cmp(&b.board))
                .then(a.rank.cmp(&b.rank))
        });
        out
    }

    /// Who won `board` last wipe.
    pub fn last_winner(&self, board: u8) -> Option<&HallLine> {
        self.lines
            .iter()
            .find(|l| l.wipe == self.last_wipe && l.board == board && l.rank == 1)
    }

    /// `(podium finishes, best (rank, board, wipe))` across every wipe.
    pub fn career(&self, key: &str) -> (u32, Option<(u32, u8, u32)>) {
        let mut n = 0;
        let mut best: Option<(u32, u8, u32)> = None;
        for l in self.lines.iter().filter(|l| l.key == key) {
            n += 1;
            let better = match best {
                None => true,
                Some((r, b, _)) => l.rank < r || (l.rank == r && l.board < b),
            };
            if better {
                best = Some((l.rank, l.board, l.wipe));
            }
        }
        (n, best)
    }
}

pub fn standings_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.standings", base.display()))
}

pub fn json_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.standings.json", base.display()))
}

pub fn hall_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.hall", base.display()))
}

pub fn last_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.lastwipe", base.display()))
}

/// What wipe `wipe` pays, for the operator to send: `.json` and `.csv`.
pub fn payout_path(base: &Path, wipe: u32, ext: &str) -> PathBuf {
    PathBuf::from(format!("{}.payout-{wipe}.{ext}", base.display()))
}

/// Write `text` to `path` by way of a temp file, so a crash leaves the old
/// one whole.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_data()?;
    }
    std::fs::rename(&tmp, path)
}

/// One row of a saved standings file, as the wipe reads it back.
struct Saved {
    key: String,
    label: String,
    scores: [u64; BOARDS],
    played: u32,
    agent: bool,
    extracted: u64,
}

fn read_saved(text: &str) -> Vec<Saved> {
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.splitn(ROW_FIELDS, '\t').collect();
        if f.len() < ROW_FIELDS - 1 || f[0] != "row" {
            continue;
        }
        let n = |i: usize| f[i].parse::<u64>().unwrap_or(0);
        let (given, deeds, kills, hoard, island) = (n(2), n(3), n(4), n(6), n(7));
        out.push(Saved {
            key: f[1].to_string(),
            label: f.get(ROW_FIELDS - 1).copied().unwrap_or("").to_string(),
            scores: [island, (given + deeds) / 100, hoard / 100, kills],
            played: n(8) as u32,
            agent: n(9) & FLAG_AGENT != 0,
            extracted: n(10),
        });
    }
    out
}

/// Whether wipe `wipe` is already closed into the hall beside `base`: its
/// island placings are last wipe's, or a podium line names it. A second
/// close would append its podiums and prizes again and sum them twice.
pub fn is_closed(base: &Path, wipe: u32) -> bool {
    let hall = Hall::load(base);
    hall.last_wipe == wipe || hall.lines.iter().any(|l| l.wipe == wipe)
}

/// What a wipe's close found, for the boot log and the Discord post.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Closed {
    /// The island's winner, `LABEL (score)`.
    pub winner: Option<String>,
    /// Each board's podium, `(board, place, label, score)`.
    pub podiums: Vec<(u8, u32, String, u64)>,
    /// Per wallet and coin, everything it is owed — prizes and what it
    /// extracted: `(address, label, total, ticker)`.
    pub paid: Vec<(String, String, u64, String)>,
    pub ticker: String,
}

/// What one wallet is owed in one coin: the total, and what it is for.
type Owed = (u64, Vec<String>);

/// **The wipe's last word**, on the closed files (`bin/shard.rs`'s
/// `end_world`, before `wipe::apply` archives the standings): each board's
/// podium and prizes into the hall, everyone's island placing into
/// `.lastwipe`, and what is owed to whom into the payout files.
pub fn close_wipe(
    base: &Path,
    wipe: u32,
    prizes: &Prizes,
    exits: &Exits,
) -> Result<Closed, String> {
    let mut closed = Closed {
        ticker: prizes.ticker.clone(),
        ..Closed::default()
    };
    let text = match std::fs::read_to_string(standings_path(base)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(closed),
        Err(e) => return Err(format!("standings {}: {e}", base.display())),
    };
    let rows = read_saved(&text);
    let label_of = |key: &str| {
        rows.iter()
            .find(|r| r.key == key)
            .map_or(String::new(), |r| r.label.clone())
    };
    let mut hall = String::new();
    // Podium wallets that played as declared agents, once each.
    let mut agents: BTreeSet<String> = BTreeSet::new();
    // Keyed (address, ticker): prizes and extraction may be different coins.
    let mut owed: BTreeMap<(String, String), Owed> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.extracted > 0 && is_wallet(&r.key)) {
        hall.push_str(&format!("extract\t{wipe}\t{}\t{}\n", r.extracted, r.key));
        if r.played < exits.min_minutes {
            println!(
                "wipe {wipe}: {} extracted {} but played {} of {} minutes: not paid",
                r.key, r.extracted, r.played, exits.min_minutes
            );
            continue;
        }
        let e = owed
            .entry((r.key.clone(), EXTRACT_TICKER.to_string()))
            .or_default();
        e.0 += r.extracted;
        e.1.push(format!("EXTRACTED {}", r.extracted));
    }
    for board in 0..BOARDS as u8 {
        let b = Board::rank(
            rows.iter()
                .map(|r| {
                    (
                        r.key.as_str(),
                        r.label.as_str(),
                        r.scores[board as usize],
                        prizes.eligible(&r.key, r.played, r.agent),
                    )
                })
                .collect(),
        );
        let mut podium: Vec<(&str, u32, u64)> = b
            .ranks
            .iter()
            .filter(|(_, (rank, _))| *rank as usize <= TOP)
            .map(|(k, (rank, score))| (k.as_str(), *rank, *score))
            .collect();
        podium.sort_by_key(|p| p.1);
        for (key, rank, score) in podium {
            let label = label_of(key);
            if rows.iter().any(|r| r.key == key && r.agent) {
                agents.insert(key.to_string());
            }
            if board == BOARD_ISLAND && rank == 1 {
                closed.winner = Some(format!("{label} ({score})"));
            }
            closed.podiums.push((board, rank, label.clone(), score));
            hall.push_str(&format!(
                "podium\t{wipe}\t{board}\t{rank}\t{score}\t{key}\t{label}\n"
            ));
        }
        if prizes.armed() {
            for (key, place) in &b.places {
                let amount = prizes.amount(board, *place);
                if amount == 0 {
                    continue;
                }
                let e = owed
                    .entry((key.clone(), prizes.ticker.clone()))
                    .or_default();
                e.0 += amount;
                e.1.push(format!("{} #{place} {amount}", SHORT_NAMES[board as usize]));
                hall.push_str(&format!(
                    "prize\t{wipe}\t{board}\t{place}\t{amount}\t{}\t{key}\n",
                    prizes.ticker
                ));
            }
        }
        if board == BOARD_ISLAND {
            let mut last = format!(
                "# Gates: last wipe's island placings.\nwipe\t{wipe}\t{}\n",
                b.ranked
            );
            let mut placed: Vec<(&String, &(u32, u64))> = b.ranks.iter().collect();
            placed.sort_by_key(|(_, (r, _))| *r);
            for (key, (rank, score)) in placed {
                last.push_str(&format!("rank\t{key}\t{rank}\t{score}\n"));
            }
            write_atomic(&last_path(base), &last)
                .map_err(|e| format!("last wipe {}: {e}", base.display()))?;
        }
    }
    if !owed.is_empty() {
        let mut csv = String::from("address,amount,ticker,for\n");
        let mut json = format!("{{\"wipe\":{wipe},\"payouts\":[");
        for (i, ((key, t), (total, parts))) in owed.iter().enumerate() {
            csv.push_str(&format!("{key},{total},{t},{}\n", parts.join(" | ")));
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                "{{\"address\":\"{key}\",\"amount\":{total},\"ticker\":\"{t}\",\"for\":[{}]}}",
                parts
                    .iter()
                    .map(|p| format!("\"{p}\""))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
            closed
                .paid
                .push((key.clone(), label_of(key), *total, t.clone()));
        }
        json.push_str("]}\n");
        write_atomic(&payout_path(base, wipe, "csv"), &csv)
            .and_then(|_| write_atomic(&payout_path(base, wipe, "json"), &json))
            .map_err(|e| format!("payout {}: {e}", base.display()))?;
    }
    for key in &agents {
        hall.push_str(&format!("agent\t{wipe}\t{key}\n"));
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(hall_path(base))
        .map_err(|e| format!("hall {}: {e}", base.display()))?;
    f.write_all(hall.as_bytes())
        .and_then(|_| f.sync_data())
        .map_err(|e| format!("hall {}: {e}", base.display()))?;
    Ok(closed)
}

/// The Discord message a wipe ends with: each board's podium, who was paid,
/// and when the next island opens. Names, never addresses.
pub fn discord_text(wipe: u32, c: &Closed, next: Option<&str>) -> String {
    let mut t = format!("**Wipe {wipe} is over.**\n");
    for board in 0..BOARDS as u8 {
        let podium: Vec<String> = c
            .podiums
            .iter()
            .filter(|p| p.0 == board)
            .take(3)
            .map(|(_, r, l, s)| format!("{r}. {l} {}", thousands(*s)))
            .collect();
        if !podium.is_empty() {
            t.push_str(&format!(
                "**{}**  {}\n",
                BOARD_NAMES[board as usize],
                podium.join("  ·  ")
            ));
        }
    }
    if !c.paid.is_empty() {
        let paid: Vec<String> = c
            .paid
            .iter()
            .map(|(_, l, a, t)| format!("{l} {} {t}", thousands(*a)))
            .collect();
        t.push_str(&format!("**Paid**  {}\n", paid.join("  ·  ")));
    }
    if let Some(next) = next {
        t.push_str(&format!("The next island opens {next}.\n"));
    }
    t.chars().take(1900).collect()
}

/// Post `text` to a Discord webhook. Blocking, bounded, and best-effort: a
/// wipe never waits on Discord or fails for it.
pub fn post_discord(url: &str, text: &str) -> Result<(), String> {
    let body = discord_body(text);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(5)))
        .build()
        .into();
    agent
        .post(url)
        .content_type("application/json")
        .send(&body)
        .map(|_| ())
        .map_err(|e| format!("discord: {e}"))
}

/// The webhook's JSON for `text`. **Mentions are switched off**: a podium
/// name is whatever a player called themselves, and `@everyone` is nine
/// printable bytes — without `allowed_mentions` it would ping the whole
/// server from the wipe post.
fn discord_body(text: &str) -> String {
    format!(
        "{{\"content\":\"{}\",\"allowed_mentions\":{{\"parse\":[]}}}}",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

// ---- this wipe ---------------------------------------------------------------

/// What the writer thread is handed: the file, and the JSON the status
/// endpoint serves.
struct Flush {
    text: String,
    json: String,
}

/// This wipe's standings. `ShardCore::standings`.
pub struct Standings {
    pub rules: Rules,
    pub prizes: Prizes,
    pub exits: Exits,
    /// The file as of the last world save, held until that save lands.
    pending: Option<Flush>,
    rows: BTreeMap<String, Row>,
    /// `(who, what)`: "lit THE CRUCIBLE".
    deeds: Vec<(String, String)>,
    /// This wipe's number (wipes so far + 1).
    pub wipe: u32,
    pub hall: Hall,
    boards: [Board; BOARDS],
    last_board: Board,
    /// Moves every time the boards are rebuilt.
    gen: u32,
    /// `gen` at the last flush: an unchanged wipe is not written again.
    flushed: Option<u32>,
    dirty: bool,
    base: Option<PathBuf>,
    out: Option<rtrb::Producer<Flush>>,
    /// Wallets not ranked because the table was full.
    pub refused: u64,
}

impl Default for Standings {
    fn default() -> Self {
        Self::off()
    }
}

impl Standings {
    /// Nothing on disk and default weights: tests, and a shard that keeps no
    /// files.
    pub fn off() -> Self {
        Standings {
            rules: Rules::default(),
            prizes: Prizes::default(),
            exits: Exits::default(),
            pending: None,
            rows: BTreeMap::new(),
            deeds: Vec::new(),
            wipe: 1,
            hall: Hall::default(),
            boards: Default::default(),
            last_board: Board::default(),
            gen: 0,
            flushed: None,
            dirty: false,
            base: None,
            out: None,
            refused: 0,
        }
    }

    /// This wipe's standings, read back from `<base>.standings` if the shard
    /// restarted mid-wipe, with the hall behind them and a writer thread for
    /// the minute's save. `base` is the world file (else the player store),
    /// the same base `wipe.rs` archives.
    pub fn open(rules: Rules, prizes: Prizes, base: Option<&Path>, wipe: u32) -> Self {
        let mut s = Standings {
            rules,
            prizes,
            wipe,
            ..Standings::off()
        };
        let Some(base) = base else {
            return s;
        };
        s.hall = Hall::load(base);
        s.last_board = s.hall.last_board();
        if let Ok(text) = std::fs::read_to_string(standings_path(base)) {
            s.load(&text);
        }
        s.base = Some(base.to_path_buf());
        let (tx, mut rx) = rtrb::RingBuffer::<Flush>::new(FLUSH_RING);
        let (file, json) = (standings_path(base), json_path(base));
        let spawned = std::thread::Builder::new()
            .name("standings".into())
            .spawn(move || loop {
                match rx.pop() {
                    Ok(f) => {
                        if let Err(e) = write_atomic(&file, &f.text) {
                            eprintln!("standings {}: {e}", file.display());
                        }
                        let _ = write_atomic(&json, &f.json);
                    }
                    Err(_) if rx.is_abandoned() => return,
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
                }
            });
        if spawned.is_ok() {
            s.out = Some(tx);
        }
        s.dirty = true;
        s.rebuild();
        s
    }

    fn load(&mut self, text: &str) {
        for line in text.lines() {
            let f: Vec<&str> = line.splitn(ROW_FIELDS, '\t').collect();
            match f.first() {
                Some(&"wipe") if f.len() >= 2 => {
                    if f[1].parse::<u32>().ok() != Some(self.wipe) {
                        // Last wipe's file that was never archived: start
                        // clean rather than hand its scores to this one.
                        self.rows.clear();
                        self.deeds.clear();
                        return;
                    }
                }
                Some(&"row") if f.len() >= ROW_FIELDS - 1 && filable(f[1]) => {
                    let n = |i: usize| f[i].parse::<u64>().unwrap_or(0);
                    self.rows.insert(
                        f[1].to_string(),
                        Row {
                            label: f.get(ROW_FIELDS - 1).copied().unwrap_or("").to_string(),
                            given: n(2),
                            deeds: n(3),
                            kills: n(4) as u32,
                            deaths: n(5) as u32,
                            hoard: n(6),
                            played: n(8) as u32,
                            agent: n(9) & FLAG_AGENT != 0,
                            extracted: n(10),
                            taken: n(11),
                        },
                    );
                }
                Some(&"deed") if f.len() >= 3 && self.deeds.len() < MAX_DEEDS => {
                    self.deeds.push((f[1].to_string(), f[2].to_string()));
                }
                _ => {}
            }
        }
    }

    /// The row for `key`, made if there is room. `label` refreshes it when
    /// not empty.
    fn row(&mut self, key: &str, label: &str) -> Option<&mut Row> {
        if !filable(key) {
            return None;
        }
        if !self.rows.contains_key(key) {
            if self.rows.len() >= MAX_ROWS {
                self.refused += 1;
                return None;
            }
            self.rows.insert(key.to_string(), Row::default());
        }
        let row = self.rows.get_mut(key)?;
        let label = clean_label(label);
        if !label.is_empty() {
            row.label = label;
        } else if row.label.is_empty() {
            row.label = short_label(key);
        }
        Some(row)
    }

    /// What the platform calls `key` now. Only a row that exists moves.
    pub fn name(&mut self, key: &str, label: &str) {
        let label = clean_label(label);
        if let Some(row) = self.rows.get_mut(key) {
            if !label.is_empty() && row.label != label {
                row.label = label;
                self.dirty = true;
            }
        }
    }

    /// `units` of `item` into a work or its tank.
    pub fn gave(&mut self, key: &str, label: &str, item: u16, units: u32) {
        let value = self.rules.worth(item, units) * self.rules.given_pct / 100;
        if let Some(row) = self.row(key, label) {
            row.given = row.given.saturating_add(value);
            self.dirty = true;
        }
    }

    /// A deed worth `points` whole points, remembered as `what` if named.
    pub fn deed(&mut self, key: &str, label: &str, points: u64, what: Option<&str>) {
        let Some(row) = self.row(key, label) else {
            return;
        };
        row.deeds = row.deeds.saturating_add(points * 100);
        self.dirty = true;
        if let Some(what) = what.filter(|_| self.deeds.len() < MAX_DEEDS) {
            self.deeds.push((key.to_string(), what.replace('\t', " ")));
        }
    }

    /// A player killed another. Either side may be nobody we rank.
    pub fn kill(&mut self, killer: Option<(&str, &str)>, victim: Option<(&str, &str)>) {
        if let Some(row) = killer.and_then(|(k, l)| self.row(k, l)) {
            row.kills += 1;
            self.dirty = true;
        }
        if let Some(row) = victim.and_then(|(k, l)| self.row(k, l)) {
            row.deaths += 1;
            self.dirty = true;
        }
    }

    /// The hoards, recounted: `(key, label, hundredths)` for every crew
    /// member's share. Everyone not named holds nothing now.
    pub fn set_hoards(&mut self, shares: &[(String, String, u64)]) {
        let mut moved = false;
        for row in self.rows.values_mut() {
            if row.hoard != 0 {
                row.hoard = 0;
                moved = true;
            }
        }
        for (key, label, value) in shares {
            if *value == 0 {
                continue;
            }
            if let Some(row) = self.row(key, label) {
                row.hoard = row.hoard.saturating_add(*value);
                moved = true;
            }
        }
        self.dirty |= moved;
    }

    pub fn row_of(&self, key: &str) -> Option<&Row> {
        self.rows.get(key)
    }

    /// A minute played by `key`, connected now. The minute that makes them
    /// eligible for a purse moves the boards, and so does the first minute
    /// as a declared agent: it changes who may be paid and what
    /// `/standings.json` says of the row.
    pub fn minute(&mut self, key: &str, label: &str, agent: bool) {
        let min = self.prizes.min_minutes;
        let armed = self.prizes.armed();
        if let Some(row) = self.row(key, label) {
            row.played = row.played.saturating_add(1);
            let declared = agent && !row.agent;
            row.agent |= agent;
            if declared || (armed && row.played == min.max(1)) {
                self.dirty = true;
            }
        }
    }

    /// `units` of JUNK went through the exchange for `key`: the fee burns,
    /// the rest is credited. The fee rounds up, so splitting a stack into
    /// small runs does not dodge it.
    pub fn extracted(&mut self, key: &str, label: &str, units: u32) {
        let fee = (units as u64 * self.exits.fee_pct.min(100)).div_ceil(100);
        if let Some(row) = self.row(key, label) {
            row.taken = row.taken.saturating_add(units as u64);
            row.extracted = row.extracted.saturating_add(units as u64 - fee);
            self.dirty = true;
        }
    }

    /// The most `key` may take out this wipe, the exchange's ceiling at
    /// `pct` per cent (`works::knob_pct`). Zero for anything that is not a
    /// wallet: the coin goes to an address or nowhere.
    pub fn cap(&self, pct: u32) -> u64 {
        self.exits.cap * pct as u64 / 100
    }

    /// What `key` may still take out this wipe.
    pub fn allowance(&self, key: &str, pct: u32) -> u64 {
        if !is_wallet(key) {
            return 0;
        }
        let taken = self.rows.get(key).map_or(0, |r| r.taken);
        self.cap(pct).saturating_sub(taken)
    }

    /// What `key` would be paid on `board` if the wipe ended now.
    pub fn would_win(&self, board: u8, key: &str) -> u64 {
        if !self.prizes.armed() || board as usize >= BOARDS {
            return 0;
        }
        self.board(board)
            .place_of(key)
            .map_or(0, |p| self.prizes.amount(board, p))
    }

    /// Rebuild the boards if anything moved. True when they did.
    pub fn rebuild(&mut self) -> bool {
        if !self.dirty {
            return false;
        }
        self.dirty = false;
        for board in 0..BOARDS as u8 {
            let (rules, prizes) = (&self.rules, &self.prizes);
            self.boards[board as usize] = Board::rank(
                self.rows
                    .iter()
                    .map(|(k, r)| {
                        (
                            k.as_str(),
                            r.label.as_str(),
                            r.score(board, rules),
                            prizes.eligible(k, r.played, r.agent),
                        )
                    })
                    .collect(),
            );
        }
        self.gen = self.gen.wrapping_add(1);
        true
    }

    pub fn gen(&self) -> u32 {
        self.gen
    }

    /// Board `b`, [`BOARD_LAST`] included.
    pub fn board(&self, b: u8) -> &Board {
        if b == BOARD_LAST {
            &self.last_board
        } else {
            &self.boards[(b as usize).min(BOARDS - 1)]
        }
    }

    /// The deeds this wipe, `(who, what)`.
    pub fn deeds(&self) -> &[(String, String)] {
        &self.deeds
    }

    /// The label of `key`, if it has a row.
    pub fn label_of(&self, key: &str) -> Option<&str> {
        self.rows.get(key).map(|r| r.label.as_str())
    }

    /// The file: a header, every row, every deed.
    pub fn text(&self) -> String {
        let mut t = format!(
            "# Gates standings, written by the shard. Hundredths of a point except\n\
             # kills, deaths, island and minutes played: row key given deeds kills deaths\n\
             # hoard island played flags extracted taken label\n\
             wipe\t{}\n",
            self.wipe
        );
        for (k, r) in &self.rows {
            t.push_str(&format!(
                "row\t{k}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                r.given,
                r.deeds,
                r.kills,
                r.deaths,
                r.hoard,
                r.score(BOARD_ISLAND, &self.rules),
                r.played,
                if r.agent { FLAG_AGENT } else { 0 },
                r.extracted,
                r.taken,
                r.label
            ));
        }
        for (k, what) in &self.deeds {
            t.push_str(&format!("deed\t{k}\t{what}\n"));
        }
        t
    }

    /// How many wipes `/standings.json`'s hall reaches back.
    pub const HALL_WIPES: usize = 8;

    /// What `/standings.json` serves: the wipe, each board's top, the deeds,
    /// and the hall's podiums for the last [`Self::HALL_WIPES`] wipes.
    ///
    /// Every row names its `wallet` (the lowercase key) when the key is a
    /// real wallet ([`is_wallet`]) and leaves the field out for a guest or a
    /// dev key; a board row also says whether it played as a declared
    /// `agent`. Wallets are already public by name through the platform's
    /// face read, so this adds an address beside a name, never a secret.
    pub fn json(&self) -> String {
        fn esc(s: &str) -> String {
            let mut o = String::with_capacity(s.len());
            for c in s.chars() {
                match c {
                    '\\' => o.push_str("\\\\"),
                    '"' => o.push_str("\\\""),
                    c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                    c => o.push(c),
                }
            }
            o
        }
        /// `,"wallet":"0x…"` for a wallet, nothing for anyone else.
        fn wallet(key: Option<&str>) -> String {
            match key.filter(|k| is_wallet(k)) {
                Some(k) => format!(",\"wallet\":\"{}\"", k.to_ascii_lowercase()),
                None => String::new(),
            }
        }
        let mut j = format!(
            "{{\"wipe\":{},\"ticker\":\"{}\",\"prize_min_minutes\":{},\"last_wipe\":{},\"boards\":[",
            self.wipe,
            if self.prizes.armed() {
                self.prizes.ticker.as_str()
            } else {
                ""
            },
            self.prizes.min_minutes,
            self.hall.last_wipe
        );
        for b in 0..=BOARDS as u8 {
            let board = self.board(b);
            if b > 0 {
                j.push(',');
            }
            let purse = self
                .prizes
                .places
                .get(b as usize)
                .filter(|_| self.prizes.armed())
                .map_or(String::new(), |p| {
                    p.iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                });
            j.push_str(&format!(
                "{{\"name\":\"{}\",\"ranked\":{},\"prizes\":[{purse}],\"top\":[",
                BOARD_NAMES[b as usize], board.ranked
            ));
            for (i, (label, score)) in board.top.iter().enumerate() {
                if i > 0 {
                    j.push(',');
                }
                let key = board.top_keys.get(i).map(String::as_str);
                let agent = match key {
                    Some(k) if b == BOARD_LAST => self.hall.was_agent(self.hall.last_wipe, k),
                    Some(k) => self.rows.get(k).is_some_and(|r| r.agent),
                    None => false,
                };
                j.push_str(&format!(
                    "{{\"name\":\"{}\",\"score\":{score}{},\"agent\":{agent}}}",
                    esc(label),
                    wallet(key)
                ));
            }
            j.push_str("]}");
        }
        j.push_str("],\"deeds\":[");
        for (i, (k, what)) in self.deeds.iter().enumerate() {
            if i > 0 {
                j.push(',');
            }
            let who = self
                .label_of(k)
                .map_or_else(|| short_label(k), str::to_string);
            j.push_str(&format!(
                "{{\"name\":\"{}\",\"did\":\"{}\"{}}}",
                esc(&who),
                esc(what),
                wallet(Some(k))
            ));
        }
        j.push_str("],\"hall\":[");
        for (i, l) in self.hall.podiums(Self::HALL_WIPES).into_iter().enumerate() {
            if i > 0 {
                j.push(',');
            }
            j.push_str(&format!(
                "{{\"wipe\":{},\"board\":\"{}\",\"rank\":{},\"score\":{},\"name\":\"{}\"{},\"agent\":{}}}",
                l.wipe,
                BOARD_NAMES[l.board as usize],
                l.rank,
                l.score,
                esc(&l.label),
                wallet(Some(&l.key)),
                self.hall.was_agent(l.wipe, &l.key)
            ));
        }
        j.push_str("]}");
        j
    }

    /// Take the file as it stands, beside a world save: held until that
    /// save lands ([`Self::release`]), so the two roll back together.
    pub fn snapshot(&mut self) {
        if self.out.is_none() || self.flushed == Some(self.gen) {
            return;
        }
        self.flushed = Some(self.gen);
        self.pending = Some(Flush {
            text: self.text(),
            json: self.json(),
        });
    }

    /// The world save the snapshot rode with has landed: hand the file to
    /// the writer thread. A full ring skips this one; the next is newer.
    pub fn release(&mut self) {
        if let (Some(f), Some(out)) = (self.pending.take(), self.out.as_mut()) {
            let _ = out.push(f);
        }
    }

    /// Write the file now, on this thread: the shutdown's last word.
    pub fn save_now(&self) -> std::io::Result<()> {
        let Some(base) = self.base.as_deref() else {
            return Ok(());
        };
        write_atomic(&standings_path(base), &self.text())?;
        write_atomic(&json_path(base), &self.json())
    }

    // ---- what players are told --------------------------------------------
    //
    // Chat lines are short (`protocol::CHAT_MAX_BYTES`, less the server's
    // prefix): one fact a line. The STANDINGS page carries the boards.

    /// `WORKS ALICE 1,240` — a board's leader.
    pub fn leader_line(&self, board: u8) -> String {
        let name = SHORT_NAMES[board as usize];
        match self.board(board).top.first() {
            Some((l, s)) => format!("{name} {l} {}", thousands(*s)),
            None => format!("{name}: nobody yet"),
        }
    }

    /// `island #4` or `island -`.
    fn place(&self, key: &str, board: u8) -> String {
        let name = SHORT_NAMES[board as usize].to_ascii_lowercase();
        match self.board(board).of(key) {
            Some((rank, _)) => format!("{name} #{rank}"),
            None => format!("{name} -"),
        }
    }

    /// What a joiner hears a few seconds in: last wipe, their hall, then
    /// where they stand in this one.
    pub fn join_lines(&self, key: Option<&str>) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(w) = self.hall.last_winner(BOARD_ISLAND) {
            out.push(format!("wipe {} went to {}", w.wipe, w.label));
            if let Some((rank, _)) = key.and_then(|k| self.hall.last.get(k)) {
                out.push(format!("you finished #{rank} of {}", self.hall.last_of));
            }
            if let Some(n) = key.and_then(|k| self.hall.last_extracted.get(k)) {
                out.push(format!("{} JUNK extracted last wipe", thousands(*n)));
            }
            if let Some(paid) = key.and_then(|k| self.hall.last_paid.get(k)) {
                out.push(format!(
                    "you won {} {} - check your wallet",
                    thousands(*paid),
                    self.hall.last_ticker
                ));
            }
        }
        let Some(key) = key else {
            return out;
        };
        let (podiums, best) = self.hall.career(key);
        if let Some((rank, _, _)) = best {
            out.push(format!(
                "your hall: {podiums} podium{}, best #{rank}",
                if podiums == 1 { "" } else { "s" }
            ));
        }
        if self.prizes.armed() {
            let places = self.prizes.places.iter().map(Vec::len).max().unwrap_or(0);
            out.push(format!(
                "{} to the top {places} of each board",
                self.prizes.ticker
            ));
        }
        let island = self.board(BOARD_ISLAND);
        out.push(match island.of(key) {
            Some((rank, _)) => format!("you are #{rank} of {} - /top", island.ranked),
            None => "standings: O then STANDINGS, or /top".into(),
        });
        out
    }

    /// `/top`: each board's leader and where the asker stands.
    pub fn top_lines(&self, key: Option<&str>) -> Vec<String> {
        let mut out: Vec<String> = (0..BOARDS as u8).map(|b| self.leader_line(b)).collect();
        if let Some(key) = key {
            out.push(format!(
                "you: {} - {}",
                self.place(key, BOARD_ISLAND),
                self.place(key, BOARD_WORKS)
            ));
            out.push(format!(
                "you: {} - {}",
                self.place(key, BOARD_HOARD),
                self.place(key, BOARD_FIGHT)
            ));
        }
        out
    }

    /// The wipe's last word, to everyone: each board's winner.
    pub fn final_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for board in 0..BOARDS as u8 {
            if let Some((l, s)) = self.board(board).top.first() {
                out.push(format!(
                    "{} goes to {l} {}",
                    SHORT_NAMES[board as usize].to_ascii_lowercase(),
                    thousands(*s)
                ));
            }
        }
        out
    }
}

/// Board names short enough for a chat line.
pub const SHORT_NAMES: [&str; BOARDS + 1] = ["ISLAND", "WORKS", "HOARD", "FIGHT", "LAST"];

/// `12,345`.
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CAROL: &str = "0xcccccccccccccccccccccccccccccccccccccccc";
    const BOT: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn rules() -> Rules {
        Rules {
            given_pct: 200,
            lit_points: 300,
            rekindled_points: 50,
            solved_points: 200,
            kill_points: 30,
            hoard_pct: 100,
            worth: vec![100, 250],
        }
    }

    #[test]
    fn the_boards_rank_four_ways_and_the_hoard_is_a_snapshot() {
        let mut s = Standings {
            rules: rules(),
            ..Standings::off()
        };
        // A gives 10 of item 1 (2.5 a unit, doubled): 50 points.
        s.gave("0xaaaa", "ALICE", 1, 10);
        // B holds a base worth 120, and killed A twice.
        s.set_hoards(&[("0xbbbb".into(), "BOB".into(), 12_000)]);
        s.kill(Some(("0xbbbb", "")), Some(("0xaaaa", "")));
        s.kill(Some(("0xbbbb", "")), Some(("0xaaaa", "")));
        assert!(s.rebuild());
        assert_eq!(s.board(BOARD_WORKS).top, vec![("ALICE".to_string(), 50)]);
        assert_eq!(s.board(BOARD_HOARD).top, vec![("BOB".to_string(), 120)]);
        assert_eq!(s.board(BOARD_FIGHT).of("0xbbbb"), Some((1, 2)));
        // Island: Bob 120 + 2 × 30 = 180, Alice 50.
        assert_eq!(s.board(BOARD_ISLAND).of("0xbbbb"), Some((1, 180)));
        assert_eq!(s.board(BOARD_ISLAND).of("0xaaaa"), Some((2, 50)));
        // Raided: the hoard is gone, the giving is not.
        s.set_hoards(&[]);
        s.rebuild();
        assert_eq!(s.board(BOARD_ISLAND).of("0xbbbb"), Some((1, 60)));
        assert_eq!(s.board(BOARD_HOARD).ranked, 0);
        assert!(!s.rebuild(), "nothing moved, nothing rebuilt");
    }

    #[test]
    fn a_saved_wipe_reads_back_and_closes_into_the_hall() {
        let dir = std::env::temp_dir().join(format!("gates-standings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("world.bin");
        let prizes = Prizes {
            ticker: "ELO".into(),
            places: [vec![500, 300], vec![200], vec![], vec![50]],
            min_minutes: 2,
            agents: false,
        };
        let mut s = Standings::open(rules(), prizes.clone(), Some(&base), 3);
        s.gave(ALICE, "ALICE", 0, 500);
        s.deed(ALICE, "", 600, Some("lit THE CRUCIBLE"));
        s.kill(Some((CAROL, "CAROL")), None);
        // An agent outscores Carol on the fight and is ranked, never paid.
        s.kill(Some((BOT, "BOT")), None);
        s.kill(Some((BOT, "BOT")), None);
        for _ in 0..2 {
            s.minute(ALICE, "", false);
            s.minute(CAROL, "", false);
            s.minute(BOT, "", true);
        }
        // Carol puts 100 JUNK through the exchange at a 2% fee, cap 150.
        s.exits = Exits {
            cap: 150,
            fee_pct: 2,
            min_minutes: 0,
        };
        assert_eq!(s.allowance(CAROL, 100), 150);
        s.extracted(CAROL, "", 100);
        assert_eq!(s.allowance(CAROL, 100), 50);
        assert_eq!(
            s.allowance(CAROL, 150),
            125,
            "the burning exchange lets more out"
        );
        assert_eq!(s.allowance("dev-key", 100), 0, "no wallet, nothing leaves");
        s.rebuild();
        assert_eq!(s.would_win(BOARD_ISLAND, ALICE), 500);
        assert_eq!(
            s.would_win(BOARD_FIGHT, CAROL),
            50,
            "first among the payable"
        );
        assert_eq!(s.would_win(BOARD_FIGHT, BOT), 0);
        s.save_now().unwrap();

        // A restart mid-wipe reads the same rows back.
        let again = Standings::open(rules(), prizes.clone(), Some(&base), 3);
        assert_eq!(again.row_of(ALICE), s.row_of(ALICE));
        assert!(again.row_of(BOT).unwrap().agent);
        assert_eq!(again.deeds(), s.deeds());
        // The next wipe's boot does not inherit them.
        let next = Standings::open(rules(), prizes.clone(), Some(&base), 4);
        assert!(next.row_of(ALICE).is_none());

        assert!(!is_closed(&base, 3));
        let closed = close_wipe(&base, 3, &prizes, &s.exits).unwrap();
        assert!(is_closed(&base, 3), "a closed wipe is not closed twice");
        assert!(!is_closed(&base, 4));
        assert_eq!(closed.winner.as_deref(), Some("ALICE (1600)"));
        // Alice: island #1 (500) and works #1 (200). Carol: island #2 among
        // the payable (300) and the fight's first payable place (50).
        // And Carol's 98 extracted JUNK, in its own coin.
        assert_eq!(
            closed.paid,
            vec![
                (ALICE.to_string(), "ALICE".into(), 700, "ELO".into()),
                (CAROL.to_string(), "CAROL".into(), 350, "ELO".into()),
                (CAROL.to_string(), "CAROL".into(), 98, "JUNK".into()),
            ]
        );
        let csv = std::fs::read_to_string(payout_path(&base, 3, "csv")).unwrap();
        assert!(csv.contains(&format!("{ALICE},700,ELO,ISLAND #1 500 | WORKS #1 200")));
        assert!(csv.contains(&format!("{CAROL},98,JUNK,EXTRACTED 98")));
        assert!(!csv.contains(BOT), "an agent is not paid");
        let text = discord_text(3, &closed, Some("Thu 15 Oct 19:00 UTC"));
        assert!(text.contains("ALICE 700 ELO") && !text.contains("0x"));
        // A podium name like `@everyone` must not ping anyone.
        let body = discord_body("@everyone \"won\"\n");
        assert!(
            body.ends_with(r#","allowed_mentions":{"parse":[]}}"#),
            "{body}"
        );
        assert!(
            body.starts_with(r#"{"content":"@everyone \"won\"\n""#),
            "{body}"
        );
        let hall = Hall::load(&base);
        assert_eq!(hall.last_wipe, 3);
        assert_eq!(hall.last.get(CAROL), Some(&(3, 30)));
        assert_eq!(hall.last_winner(BOARD_FIGHT).unwrap().label, "BOT");
        assert_eq!(hall.career(ALICE).1, Some((1, BOARD_ISLAND, 3)));
        assert_eq!(hall.last_board().top[0], ("ALICE".to_string(), 1600));
        assert_eq!(hall.last_paid.get(CAROL), Some(&350));
        let lines = Standings::open(rules(), prizes, Some(&base), 4).join_lines(Some(CAROL));
        assert_eq!(lines[0], "wipe 3 went to ALICE");
        assert_eq!(lines[1], "you finished #3 of 3");
        assert_eq!(lines[2], "98 JUNK extracted last wipe");
        assert_eq!(lines[3], "you won 350 ELO - check your wallet");
        assert_eq!(lines[4], "your hall: 2 podiums, best #2");
        assert_eq!(lines[5], "ELO to the top 2 of each board");
        for l in &lines {
            assert!(
                l.len() + 9 <= protocol::CHAT_MAX_BYTES,
                "`{l}` fits a chat line"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A wallet below `extract_min_minutes` is left out of the payout file,
    /// though the hall still records what it took out; and the fee rounds up,
    /// so 40 at 2% burns 1 rather than nothing.
    #[test]
    fn extraction_pays_only_wallets_that_played_and_the_fee_rounds_up() {
        let dir = std::env::temp_dir().join(format!("gates-extract-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("world.bin");
        let prizes = Prizes::default();
        let mut s = Standings::open(rules(), prizes.clone(), Some(&base), 3);
        s.exits = Exits {
            cap: 1000,
            fee_pct: 2,
            min_minutes: 2,
        };
        for _ in 0..2 {
            s.minute(ALICE, "", false);
        }
        s.minute(CAROL, "", false);
        s.extracted(ALICE, "ALICE", 100);
        s.extracted(CAROL, "CAROL", 40);
        assert_eq!(s.row_of(CAROL).unwrap().extracted, 39, "the fee rounds up");
        s.rebuild();
        s.save_now().unwrap();
        let closed = close_wipe(&base, 3, &prizes, &s.exits).unwrap();
        assert_eq!(
            closed.paid,
            vec![(ALICE.to_string(), "ALICE".into(), 98, "JUNK".into())]
        );
        let csv = std::fs::read_to_string(payout_path(&base, 3, "csv")).unwrap();
        assert!(!csv.contains(CAROL), "a minute's play is not paid");
        assert_eq!(Hall::load(&base).last_extracted.get(CAROL), Some(&39));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `/standings.json` is a website's read: it parses as JSON, a wallet
    /// row names its wallet and a guest or dev key's does not, every board
    /// row says whether it is an agent, and the hall carries the podiums.
    #[test]
    fn the_json_names_wallets_and_agents_and_carries_the_hall() {
        const GUEST: &str = "0x0000000000000000000000000000000000000000";
        let dir = std::env::temp_dir().join(format!("gates-standings-json-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("world.bin");
        let prizes = Prizes::default();
        let mut s = Standings::open(rules(), prizes.clone(), Some(&base), 3);
        s.gave(ALICE, "ALICE", 0, 500);
        s.deed(ALICE, "", 600, Some("lit THE CRUCIBLE"));
        s.deed("dev-key", "DEV", 10, Some("solved \"THE LOCK\""));
        s.kill(Some((BOT, "BOT")), None);
        s.minute(BOT, "", true);
        s.kill(Some((GUEST, "")), None);
        s.rebuild();
        s.save_now().unwrap();
        close_wipe(&base, 3, &prizes, &Exits::default()).unwrap();

        let mut s = Standings::open(rules(), prizes, Some(&base), 4);
        s.gave(CAROL, "CAROL", 1, 4);
        s.kill(Some((BOT, "BOT")), None);
        s.minute(BOT, "", true);
        s.gave("dev-key", "DEV", 0, 1);
        s.rebuild();
        let v: serde_json::Value = serde_json::from_str(&s.json()).expect("valid JSON");
        assert_eq!(v["wipe"], 4);
        assert_eq!(v["last_wipe"], 3);

        let boards = v["boards"].as_array().unwrap();
        assert_eq!(boards.len(), BOARDS + 1);
        let row = |b: usize, name: &str| {
            boards[b]["top"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["name"] == name)
                .unwrap_or_else(|| panic!("{name} on board {b}"))
                .clone()
        };
        for b in boards {
            for r in b["top"].as_array().unwrap() {
                assert!(r["name"].is_string() && r["score"].is_u64() && r["agent"].is_boolean());
            }
        }
        let island = BOARD_ISLAND as usize;
        assert_eq!(row(island, "CAROL")["wallet"], CAROL);
        assert_eq!(row(island, "CAROL")["agent"], false);
        assert_eq!(row(island, "BOT")["agent"], true);
        assert!(
            row(island, "DEV").get("wallet").is_none(),
            "a dev key is no wallet"
        );

        // LAST WIPE reads the hall: names, wallets and who was an agent.
        let last = BOARD_LAST as usize;
        assert_eq!(boards[last]["name"], "LAST WIPE");
        assert_eq!(row(last, "ALICE")["wallet"], ALICE);
        assert_eq!(row(last, "ALICE")["agent"], false);
        assert_eq!(row(last, "BOT")["agent"], true);
        assert!(row(last, "DEV").get("wallet").is_none());
        let guest = row(last, &short_label(GUEST));
        assert!(guest.get("wallet").is_none(), "the guest is no wallet");

        // This wipe's deeds are none yet; last wipe's are in the hall only.
        assert_eq!(v["deeds"].as_array().unwrap().len(), 0);
        let hall = v["hall"].as_array().unwrap();
        assert!(!hall.is_empty());
        for h in hall {
            assert_eq!(h["wipe"], 3);
            assert!(h["rank"].as_u64().unwrap() <= 3);
            assert!(BOARD_NAMES.contains(&h["board"].as_str().unwrap()));
            assert!(h["score"].is_u64() && h["name"].is_string() && h["agent"].is_boolean());
        }
        let fight = |name: &str| {
            hall.iter()
                .find(|h| h["board"] == "THE FIGHT" && h["name"] == name)
                .unwrap_or_else(|| panic!("{name} on the fight's podium"))
        };
        assert_eq!(fight("BOT")["wallet"], BOT);
        assert_eq!(fight("BOT")["agent"], true);
        assert!(fight(&short_label(GUEST)).get("wallet").is_none());

        // The deeds of a wipe in play: a wallet's names it, a dev key's not,
        // and a quote in a deed survives the escaping.
        let mut s = Standings::off();
        s.deed(ALICE, "ALICE", 1, Some("lit THE CRUCIBLE"));
        s.deed("dev-key", "DEV", 1, Some("solved \"THE LOCK\""));
        let v: serde_json::Value = serde_json::from_str(&s.json()).expect("valid JSON");
        let deeds = v["deeds"].as_array().unwrap();
        assert_eq!(deeds[0]["wallet"], ALICE);
        assert!(deeds[1].get("wallet").is_none());
        assert_eq!(deeds[1]["did"], "solved \"THE LOCK\"");
        assert_eq!(v["last_wipe"], 0);
        assert_eq!(v["hall"].as_array().unwrap().len(), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The hall a website sees is podiums only, newest wipes first.
    #[test]
    fn the_hall_shows_the_newest_podiums() {
        let mut hall = Hall::default();
        for wipe in 1..=10 {
            for board in 0..BOARDS as u8 {
                for rank in 1..=TOP as u32 {
                    hall.lines.push(HallLine {
                        wipe,
                        board,
                        rank,
                        score: 100 - rank as u64,
                        key: ALICE.into(),
                        label: "ALICE".into(),
                    });
                }
            }
        }
        let p = hall.podiums(Standings::HALL_WIPES);
        assert_eq!(p.len(), Standings::HALL_WIPES * BOARDS * 3);
        assert_eq!((p[0].wipe, p[0].board, p[0].rank), (10, 0, 1));
        assert_eq!(p.last().unwrap().wipe, 3);
        assert!(p.iter().all(|l| l.rank <= 3));
    }

    #[test]
    fn labels_and_numbers_read_like_a_scoreboard() {
        assert_eq!(
            short_label("0x1234567890abcdef1234567890abcdef12345678"),
            "0x1234..5678"
        );
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(1_240), "1,240");
        assert_eq!(thousands(1_000_000), "1,000,000");
        assert_eq!(clean_label("  tab\there  "), "tabhere");
    }
}
