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

use sim_core::skin::{SkinContent, SkinSet};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn gifts_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.gifts", base.display()))
}

/// The ledger: wallet → the catalog ids given to it, sorted.
#[derive(Clone, Debug, Default)]
pub struct Gifts {
    path: Option<PathBuf>,
    rows: BTreeMap<String, Vec<u16>>,
    /// A gift was given since the ledger was last handed to the store thread.
    dirty: bool,
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
            path: Some(path.clone()),
            ..Self::default()
        };
        if let Ok(text) = std::fs::read_to_string(&path) {
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
                        let row = g.rows.entry(wallet.to_string()).or_default();
                        row.extend(ids);
                        row.sort_unstable();
                        row.dedup();
                    }
                    _ => eprintln!("{}:{}: skipped `{line}`", path.display(), n + 1),
                }
            }
        }
        g
    }

    /// What `wallet` was given, over `sc`'s rows. An id this shard's
    /// content no longer knows owns nothing.
    pub fn of(&self, wallet: &str, sc: &SkinContent) -> SkinSet {
        let mut set = SkinSet::EMPTY;
        for id in self.rows.get(wallet).into_iter().flatten() {
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
}
