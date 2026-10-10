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
//! ## Where this runs
//!
//! Server-side only: nothing here is sim state or hashed. The sim's events
//! (`EV_GAVE`, `EV_WORK`, `EV_MECH_SOLVED`, `EV_DEATH`) are tallied per
//! **wallet** by `ShardCore`, because a player id is minted per connection.
//! The file is rewritten once a minute by a writer thread fed through a ring
//! (`<world>.standings`, plus `.standings.json` for the status endpoint), and
//! once more, synchronously, at shutdown, after a last count of the hoards.

use std::collections::BTreeMap;
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
    /// `(label, score)`, best first.
    pub top: Vec<(String, u64)>,
    /// Wallets with a score above zero.
    pub ranked: u32,
    ranks: BTreeMap<String, (u32, u64)>,
}

impl Board {
    /// Rank (1-based) and score of `key`, if it scored.
    pub fn of(&self, key: &str) -> Option<(u32, u64)> {
        self.ranks.get(key).copied()
    }

    /// Rank `(key, label, score)` rows: best first, ties to the earlier key.
    fn rank(mut rows: Vec<(&str, &str, u64)>) -> Board {
        rows.retain(|r| r.2 > 0);
        rows.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(b.0)));
        let mut b = Board {
            ranked: rows.len() as u32,
            ..Board::default()
        };
        for (i, (key, label, score)) in rows.into_iter().enumerate() {
            if i < TOP {
                b.top.push((label.to_string(), score));
            }
            b.ranks.insert(key.to_string(), (i as u32 + 1, score));
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
}

impl Hall {
    pub fn load(base: &Path) -> Hall {
        let mut h = Hall::default();
        if let Ok(text) = std::fs::read_to_string(hall_path(base)) {
            for line in text.lines() {
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
        }
        b.ranks = self.last.clone();
        b
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
}

fn read_saved(text: &str) -> Vec<Saved> {
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.splitn(10, '\t').collect();
        if f.len() < 9 || f[0] != "row" {
            continue;
        }
        let n = |i: usize| f[i].parse::<u64>().unwrap_or(0);
        let (given, deeds, kills, hoard, island) = (n(2), n(3), n(4), n(6), n(7));
        out.push(Saved {
            key: f[1].to_string(),
            label: f.get(9).copied().unwrap_or("").to_string(),
            scores: [island, (given + deeds) / 100, hoard / 100, kills],
        });
    }
    out
}

/// **The wipe's last word**, on the closed files (`bin/shard.rs`'s
/// `end_world`, before `wipe::apply` archives the standings): each board's
/// podium into the hall, everyone's island placing into `.lastwipe`.
/// Returns the island's winner, for the boot log.
pub fn close_wipe(base: &Path, wipe: u32) -> Result<Option<String>, String> {
    let text = match std::fs::read_to_string(standings_path(base)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("standings {}: {e}", base.display())),
    };
    let rows = read_saved(&text);
    let mut hall = String::new();
    let mut winner = None;
    for board in 0..BOARDS as u8 {
        let b = Board::rank(
            rows.iter()
                .map(|r| (r.key.as_str(), r.label.as_str(), r.scores[board as usize]))
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
            let label = rows
                .iter()
                .find(|r| r.key == key)
                .map_or(String::new(), |r| r.label.clone());
            if board == BOARD_ISLAND && rank == 1 {
                winner = Some(format!("{label} ({score})"));
            }
            hall.push_str(&format!(
                "podium\t{wipe}\t{board}\t{rank}\t{score}\t{key}\t{label}\n"
            ));
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
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(hall_path(base))
        .map_err(|e| format!("hall {}: {e}", base.display()))?;
    f.write_all(hall.as_bytes())
        .and_then(|_| f.sync_data())
        .map_err(|e| format!("hall {}: {e}", base.display()))?;
    Ok(winner)
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
    pub fn open(rules: Rules, base: Option<&Path>, wipe: u32) -> Self {
        let mut s = Standings {
            rules,
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
            let f: Vec<&str> = line.splitn(10, '\t').collect();
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
                Some(&"row") if f.len() >= 9 && filable(f[1]) => {
                    let n = |i: usize| f[i].parse::<u64>().unwrap_or(0);
                    self.rows.insert(
                        f[1].to_string(),
                        Row {
                            label: f.get(9).copied().unwrap_or("").to_string(),
                            given: n(2),
                            deeds: n(3),
                            kills: n(4) as u32,
                            deaths: n(5) as u32,
                            hoard: n(6),
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

    /// Rebuild the boards if anything moved. True when they did.
    pub fn rebuild(&mut self) -> bool {
        if !self.dirty {
            return false;
        }
        self.dirty = false;
        for board in 0..BOARDS as u8 {
            let rules = &self.rules;
            self.boards[board as usize] = Board::rank(
                self.rows
                    .iter()
                    .map(|(k, r)| (k.as_str(), r.label.as_str(), r.score(board, rules)))
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
             # kills, deaths and island: row key given deeds kills deaths hoard island 0 label\n\
             wipe\t{}\n",
            self.wipe
        );
        for (k, r) in &self.rows {
            t.push_str(&format!(
                "row\t{k}\t{}\t{}\t{}\t{}\t{}\t{}\t0\t{}\n",
                r.given,
                r.deeds,
                r.kills,
                r.deaths,
                r.hoard,
                r.score(BOARD_ISLAND, &self.rules),
                r.label
            ));
        }
        for (k, what) in &self.deeds {
            t.push_str(&format!("deed\t{k}\t{what}\n"));
        }
        t
    }

    /// What `/standings.json` serves: the wipe, each board's top, the deeds
    /// and last wipe's champions. Labels are printable ASCII (`clean_label`),
    /// so escaping `"` and `\` is the whole of JSON's string rule here.
    pub fn json(&self) -> String {
        fn esc(s: &str) -> String {
            s.replace('\\', "\\\\").replace('"', "\\\"")
        }
        let mut j = format!("{{\"wipe\":{},\"boards\":[", self.wipe);
        for b in 0..=BOARDS as u8 {
            let board = self.board(b);
            if b > 0 {
                j.push(',');
            }
            j.push_str(&format!(
                "{{\"name\":\"{}\",\"ranked\":{},\"top\":[",
                BOARD_NAMES[b as usize], board.ranked
            ));
            for (i, (label, score)) in board.top.iter().enumerate() {
                if i > 0 {
                    j.push(',');
                }
                j.push_str(&format!(
                    "{{\"name\":\"{}\",\"score\":{score}}}",
                    esc(label)
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
                "{{\"name\":\"{}\",\"did\":\"{}\"}}",
                esc(&who),
                esc(what)
            ));
        }
        j.push_str("]}");
        j
    }

    /// Hand the file to the writer thread. A full ring skips this one: the
    /// next minute's is newer anyway.
    pub fn flush(&mut self) {
        if self.out.is_none() || self.flushed == Some(self.gen) {
            return;
        }
        self.flushed = Some(self.gen);
        let f = Flush {
            text: self.text(),
            json: self.json(),
        };
        if let Some(out) = self.out.as_mut() {
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
        let mut s = Standings::open(rules(), Some(&base), 3);
        s.gave("0xaaaa", "ALICE", 0, 500);
        s.deed("0xaaaa", "", 600, Some("lit THE CRUCIBLE"));
        s.kill(Some(("0xcccc", "CAROL")), None);
        s.rebuild();
        s.save_now().unwrap();

        // A restart mid-wipe reads the same rows back.
        let again = Standings::open(rules(), Some(&base), 3);
        assert_eq!(again.row_of("0xaaaa"), s.row_of("0xaaaa"));
        assert_eq!(again.deeds(), s.deeds());
        // The next wipe's boot does not inherit them.
        let next = Standings::open(rules(), Some(&base), 4);
        assert!(next.row_of("0xaaaa").is_none());

        let winner = close_wipe(&base, 3).unwrap();
        assert_eq!(winner.as_deref(), Some("ALICE (1600)"));
        let hall = Hall::load(&base);
        assert_eq!(hall.last_wipe, 3);
        assert_eq!(hall.last.get("0xcccc"), Some(&(2, 30)));
        assert_eq!(hall.last_winner(BOARD_FIGHT).unwrap().label, "CAROL");
        assert_eq!(hall.career("0xaaaa").1, Some((1, BOARD_ISLAND, 3)));
        assert_eq!(hall.last_board().top[0], ("ALICE".to_string(), 1600));
        let lines = Standings::open(rules(), Some(&base), 4).join_lines(Some("0xcccc"));
        assert_eq!(lines[0], "wipe 3 went to ALICE");
        assert_eq!(lines[1], "you finished #2 of 2");
        assert_eq!(lines[2], "your hall: 2 podiums, best #1");
        for l in &lines {
            assert!(
                l.len() + 9 <= protocol::CHAT_MAX_BYTES,
                "`{l}` fits a chat line"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
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
