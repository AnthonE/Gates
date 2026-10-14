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
            rows: BTreeMap::new(),
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

    /// Give `wallet` every row of `set`. Whether anything was new; a new
    /// gift is on disk before this returns (a few hundred bytes, once per
    /// wallet, so the sim thread can afford the write). A failed write is
    /// logged and the gift still stands for this run.
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
        if let Some(path) = &self.path {
            if let Err(e) = write_atomic(path, &self.text()) {
                eprintln!("gifts: writing {}: {e}", path.display());
            }
        }
        true
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

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
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
            sc.defs[i] = SkinDef { catalog, covers: 0 };
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
        assert!(!g.give(CAROL, founder, &sc), "a second ask gives nothing");

        let again = Gifts::open(Some(&base));
        assert_eq!(again.of(CAROL, &sc), founder);
        assert!(again
            .of("0x00000000000000000000000000000000000000c4", &sc)
            .is_empty());

        // The catalog reordered: ids 7 and 8 are now rows 0 and 1.
        let mut moved = SkinContent::EMPTY;
        for (i, catalog) in [7u16, 8, 1].into_iter().enumerate() {
            moved.defs[i] = SkinDef { catalog, covers: 0 };
        }
        moved.count = 3;
        let set = again.of(CAROL, &moved);
        assert!(set.has(0) && set.has(1) && !set.has(2));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
