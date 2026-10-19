//! **Skins the shard gave away**: the alpha's Founder set, handed out by
//! THE OUTFITTER at THE GATE (a `gift` topic in `content/arc.toml`).
//!
//! Ownership is otherwise the platform's (`skins.rs`): a token on elo's item
//! contract. A gift is not a token. Minting is the operator's act, so the
//! shard writes down who took what in `<world>.gifts`, one wallet a line,
//! and that file is the list a later airdrop mints from. A player's owned
//! set is the platform's answer plus this file's row (`ShardCore::skins_owned`).
//!
//! The file sits beside `<world>.hall` and the wipe leaves it alone
//! (`wipe::apply` archives the world and the standings only), so a gift
//! outlives the world it was given in, the way a skin is meant to.
//!
//! Keyed by catalog id, never row index: a row moved in `skins.toml` must not
//! hand somebody a different look.
//!
//! **And what an airdrop gave**, in `<world>.airdrop`, the same line format.
//! The shard only reads that one: `ops/airdrop.py` writes it (who played a
//! week and held a coin, say), and the shard reads it again within a minute
//! of it changing ([`Gifts::reload_airdrop`]), so a drop lands without a
//! restart. It is how an unlock reaches a wallet (Rust's Frog Boots, given to
//! every account that played its early access; `sim_core::skin`).

use sim_core::skin::{SkinContent, SkinSet};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn gifts_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.gifts", base.display()))
}

pub fn airdrop_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.airdrop", base.display()))
}

/// The ledger: wallet → the catalog ids given to it, sorted.
#[derive(Clone, Debug, Default)]
pub struct Gifts {
    path: Option<PathBuf>,
    rows: BTreeMap<String, Vec<u16>>,
    /// A gift was given since the ledger was last handed to the store thread.
    dirty: bool,
    /// The airdrop list, read and never written, and what it was when read
    /// (modified time and length), so a changed file is read again.
    airdrop_path: Option<PathBuf>,
    airdrop_stamp: Option<(std::time::SystemTime, u64)>,
    airdrop: BTreeMap<String, Vec<u16>>,
}

impl Gifts {
    /// Remembers nothing past a restart. Tests and a shard with no save.
    pub fn off() -> Self {
        Self::default()
    }

    /// The ledger beside `base`, read if it exists. A line that does not
    /// parse is skipped and logged, never fatal: the worst it costs is a
    /// player asking the Outfitter again.
    pub fn open(base: Option<&Path>) -> Self {
        let Some(base) = base else {
            return Self::off();
        };
        let path = gifts_path(base);
        let mut g = Self {
            rows: read_rows(&path),
            path: Some(path),
            airdrop_path: Some(airdrop_path(base)),
            ..Self::default()
        };
        g.reload_airdrop();
        g
    }

    /// Read the airdrop list again if it changed since the last read (or
    /// appeared, or went). Whether what it gives changed. A stat a minute on
    /// the sim thread, and a read only when the file moved.
    pub fn reload_airdrop(&mut self) -> bool {
        let Some(path) = &self.airdrop_path else {
            return false;
        };
        let stamp = std::fs::metadata(path)
            .ok()
            .and_then(|m| Some((m.modified().ok()?, m.len())));
        if stamp == self.airdrop_stamp {
            return false;
        }
        self.airdrop_stamp = stamp;
        let rows = read_rows(path);
        let moved = rows != self.airdrop;
        self.airdrop = rows;
        moved
    }

    /// Wallets on the airdrop list.
    pub fn airdrop_len(&self) -> usize {
        self.airdrop.len()
    }

    /// What `wallet` was given, gifts and airdrops together, over `sc`'s
    /// rows. An id this shard's content no longer knows owns nothing.
    pub fn of(&self, wallet: &str, sc: &SkinContent) -> SkinSet {
        let mut set = SkinSet::EMPTY;
        let given = self.rows.get(wallet).into_iter().flatten();
        let dropped = self.airdrop.get(wallet).into_iter().flatten();
        for id in given.chain(dropped) {
            if let Some(row) = sc.row_of(*id) {
                set.insert(row);
            }
        }
        set
    }

    /// Give `wallet` every row of `set`. Whether anything was new. Touches
    /// no file: a new gift marks the ledger dirty, and the sim loop hands the
    /// whole ledger to the store thread after the tick ([`Self::take_write`],
    /// `net.rs`) — the sim thread does not wait on a disk. The gift stands in
    /// memory at once; a crash before the write lands costs a player asking
    /// the Outfitter again.
    pub fn give(&mut self, wallet: &str, set: SkinSet, sc: &SkinContent) -> bool {
        if self.of(wallet, sc).has_all(&set) {
            return false;
        }
        let row = self.rows.entry(wallet.to_string()).or_default();
        for (i, d) in sc.rows().iter().enumerate() {
            if set.has(i) {
                row.push(d.catalog);
            }
        }
        row.sort_unstable();
        row.dedup();
        self.dirty |= self.path.is_some();
        true
    }

    /// Write the whole ledger again (a side-file write failed).
    pub fn mark_dirty(&mut self) {
        self.dirty = self.path.is_some();
    }

    /// A gift is waiting to be written.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The whole ledger for the store thread, if a gift was given since the
    /// last take; `None` with no file or nothing new. The caller checks the
    /// ring for room first, so a taken ledger is never thrown away.
    pub fn take_write(&mut self) -> Option<crate::slot::FileMsg> {
        if !std::mem::take(&mut self.dirty) {
            return None;
        }
        Some(crate::slot::FileMsg {
            path: self.path.clone()?,
            text: self.text(),
        })
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    fn text(&self) -> String {
        let mut out = String::from(
            "# Skins this shard gave (server/src/gifts.rs): wallet, then catalog ids.\n",
        );
        for (wallet, ids) in &self.rows {
            out.push_str(wallet);
            for id in ids {
                out.push(' ');
                out.push_str(&id.to_string());
            }
            out.push('\n');
        }
        out
    }
}

/// A ledger file: `wallet id id …` a line, `#` comments. A line that does
/// not parse is skipped and logged, never fatal: the worst it costs is a
/// player asking the Outfitter again, or an airdrop line fixed and re-read.
fn read_rows(path: &Path) -> BTreeMap<String, Vec<u16>> {
    let mut rows: BTreeMap<String, Vec<u16>> = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return rows;
    };
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let wallet = it.next().unwrap_or_default();
        let ids: Option<Vec<u16>> = it.map(|t| t.parse().ok()).collect();
        match ids {
            Some(ids) if crate::entitle::is_wallet(wallet) => {
                let row = rows.entry(wallet.to_string()).or_default();
                row.extend(ids);
                row.sort_unstable();
                row.dedup();
            }
            _ => eprintln!("{}:{}: skipped `{line}`", path.display(), n + 1),
        }
    }
    rows
}

/// Temp, fsync, rename: a crash keeps the old file whole. The store thread's
/// writer for every side file (`net.rs` store_thread).
pub(crate) fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_data()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::skin::SkinDef;

    const CAROL: &str = "0x00000000000000000000000000000000000000c3";

    fn content() -> SkinContent {
        let mut sc = SkinContent::EMPTY;
        for (i, catalog) in [1u16, 7, 8].into_iter().enumerate() {
            sc.defs[i] = SkinDef {
                catalog,
                covers: 0,
                unlock: false,
            };
        }
        sc.count = 3;
        sc
    }

    /// Given once, kept across a restart, and keyed by catalog id: a row
    /// that moves in the catalog keeps the same look on the wallet.
    #[test]
    fn a_gift_is_given_once_and_outlives_a_restart() {
        let dir = std::env::temp_dir().join(format!("gifts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("world");
        let _ = std::fs::remove_file(gifts_path(&base));
        let sc = content();
        let mut founder = SkinSet::EMPTY;
        founder.insert(1);
        founder.insert(2);

        let mut g = Gifts::open(Some(&base));
        assert!(g.give(CAROL, founder, &sc));
        assert!(
            !gifts_path(&base).exists(),
            "give() touches no file: the store thread writes it"
        );
        assert!(!g.give(CAROL, founder, &sc), "a second ask gives nothing");
        let msg = g.take_write().expect("a gift was given");
        assert!(g.take_write().is_none(), "taken once");
        assert_eq!(msg.path, gifts_path(&base));
        write_atomic(&msg.path, &msg.text).unwrap();
        let mut off = Gifts::off();
        assert!(off.give(CAROL, founder, &sc));
        assert!(off.take_write().is_none(), "no file, nothing to write");

        let again = Gifts::open(Some(&base));
        assert_eq!(again.of(CAROL, &sc), founder);
        assert!(again
            .of("0x00000000000000000000000000000000000000c4", &sc)
            .is_empty());

        // The catalog reordered: ids 7 and 8 are now rows 0 and 1.
        let mut moved = SkinContent::EMPTY;
        for (i, catalog) in [7u16, 8, 1].into_iter().enumerate() {
            moved.defs[i] = SkinDef {
                catalog,
                covers: 0,
                unlock: false,
            };
        }
        moved.count = 3;
        let set = again.of(CAROL, &moved);
        assert!(set.has(0) && set.has(1) && !set.has(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The airdrop list is read, re-read when it changes, merged into what
    /// a wallet owns, and never written by the shard.
    #[test]
    fn an_airdrop_list_is_read_again_when_it_changes_and_never_written() {
        let dir = std::env::temp_dir().join(format!("airdrop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("world");
        let _ = std::fs::remove_file(airdrop_path(&base));
        let sc = content();
        let mut g = Gifts::open(Some(&base));
        assert!(g.of(CAROL, &sc).is_empty());
        assert!(!g.reload_airdrop(), "no file, nothing new");

        std::fs::write(
            airdrop_path(&base),
            format!("# a drop\n{CAROL} 8\nnot-a-wallet 8\n"),
        )
        .unwrap();
        assert!(g.reload_airdrop(), "the new list is read");
        assert_eq!(g.airdrop_len(), 1, "the bad line is skipped");
        assert!(g.of(CAROL, &sc).has(2), "catalog 8 is row 2");
        assert!(!g.reload_airdrop(), "unchanged, not read again");

        // A gift on top writes the gifts ledger alone.
        let mut one = SkinSet::EMPTY;
        one.insert(0);
        assert!(g.give(CAROL, one, &sc));
        let msg = g.take_write().expect("a gift was given");
        assert_eq!(msg.path, gifts_path(&base));
        assert!(
            !msg.text.contains(" 8"),
            "the airdrop stays in its own file"
        );

        std::fs::write(airdrop_path(&base), "# empty now, and longer than before\n").unwrap();
        assert!(g.reload_airdrop(), "a changed list is read again");
        assert!(
            !g.of(CAROL, &sc).has(2),
            "and what it no longer gives is gone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
