//! **Mints the shard owes**: what THE OUTFITTER sells (`content/arc.toml`, a
//! priced gift, the Abyss Rug's unlock) goes to the buyer's wallet as an
//! `EloItem:GATES` token, and that token is the only proof of owning it
//! (`skins.rs` reads `/api/items/of`): sold on, the unlock goes with it.
//!
//! The shard takes the price in the sim (`Command::Pay`) and writes the sale
//! here, `<world>.mints`, one line each: `ref wallet catalog`. The minter
//! (`ops/mint_pending.py`, a timer on this box) mints each one on morr,
//! where the key is, and appends the same line and the transaction to
//! `<world>.minted`. The accept loop watches that file and reads the
//! wallet's items again (`net.rs`), which is when the unlock arrives. A sale
//! stays here until the wallet's items show its catalog, so asking again in
//! the meantime charges nothing.
//!
//! `ref` is the sale's own: the minter's `mintRef` is keccak of
//! `gates:buy:<catalog>:<wallet>:<ref>`, so a retry can never mint a sale
//! twice, and a wallet that sold its token and buys again is a new sale.
//!
//! Beside the gifts and the hall: a wipe leaves both files alone.

use sim_core::skin::{SkinContent, SkinSet};
use std::path::{Path, PathBuf};

pub fn mints_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.mints", base.display()))
}

pub fn minted_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.minted", base.display()))
}

/// One sale waiting for its token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sale {
    pub reference: String,
    pub wallet: String,
    pub catalog: u16,
}

#[derive(Clone, Debug, Default)]
pub struct Mints {
    path: Option<PathBuf>,
    sales: Vec<Sale>,
    /// A sale came or went since the queue was last handed to the store
    /// thread.
    dirty: bool,
}

impl Mints {
    /// Remembers nothing past a restart. Tests and a shard with no save.
    pub fn off() -> Self {
        Self::default()
    }

    /// The queue beside `base`, read if it exists. A line that does not
    /// parse is skipped and logged: the token it was owed is then the
    /// operator's to mint by hand from the log.
    pub fn open(base: Option<&Path>) -> Self {
        let Some(base) = base else {
            return Self::off();
        };
        let path = mints_path(base);
        let mut m = Self {
            path: Some(path.clone()),
            ..Self::default()
        };
        if let Ok(text) = std::fs::read_to_string(&path) {
            for (n, line) in text.lines().enumerate() {
                match parse(line) {
                    Some(Some(sale)) => m.sales.push(sale),
                    Some(None) => {}
                    None => eprintln!("{}:{}: skipped `{line}`", path.display(), n + 1),
                }
            }
        }
        m
    }

    /// Is a sale of any row of `set` to `wallet` still waiting for its
    /// token?
    pub fn waiting(&self, wallet: &str, set: SkinSet, sc: &SkinContent) -> bool {
        self.sales
            .iter()
            .any(|s| s.wallet == wallet && sc.row_of(s.catalog).is_some_and(|r| set.has(r)))
    }

    /// Owe `wallet` a token for every row of `set`, sold at `unix` (the
    /// sale's ref). How many sales that made.
    pub fn sell(&mut self, wallet: &str, set: SkinSet, sc: &SkinContent, unix: u64) -> usize {
        let before = self.sales.len();
        for (i, d) in sc.rows().iter().enumerate() {
            if set.has(i) {
                self.sales.push(Sale {
                    reference: unix.to_string(),
                    wallet: wallet.to_string(),
                    catalog: d.catalog,
                });
            }
        }
        let made = self.sales.len() - before;
        self.dirty |= made > 0 && self.path.is_some();
        made
    }

    /// The platform says what `wallet` holds: every sale whose row it now
    /// holds is settled, the token being the proof from here on.
    pub fn settle(&mut self, wallet: &str, owned: SkinSet, sc: &SkinContent) {
        let before = self.sales.len();
        self.sales.retain(|s| {
            !(s.wallet == wallet && sc.row_of(s.catalog).is_some_and(|r| owned.has(r)))
        });
        self.dirty |= self.sales.len() != before && self.path.is_some();
    }

    /// Write the whole queue again (a side-file write failed).
    pub fn mark_dirty(&mut self) {
        self.dirty = self.path.is_some();
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The whole queue for the store thread, if it changed since the last
    /// take (`gifts::Gifts::take_write`'s shape).
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
        self.sales.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sales.is_empty()
    }

    fn text(&self) -> String {
        let mut out = String::from(
            "# Tokens this shard sold and owes (server/src/mints.rs): ref, wallet, catalog id.\n",
        );
        for s in &self.sales {
            out.push_str(&format!("{} {} {}\n", s.reference, s.wallet, s.catalog));
        }
        out
    }
}

/// One `ref wallet catalog [anything]` line: `Some(None)` for a blank or a
/// comment, `None` for a line that does not parse.
pub fn parse(line: &str) -> Option<Option<Sale>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Some(None);
    }
    let mut it = line.split_whitespace();
    let reference = it.next()?;
    let wallet = it.next()?;
    let catalog = it.next()?.parse().ok()?;
    let ok_ref = reference
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    // A wallet, or an Elo Pros account with none (v151), whose sale the
    // minter sends to the platform's locker instead of the chain.
    (ok_ref && (crate::entitle::is_wallet(wallet) || crate::auth::is_account_str(wallet))).then(
        || {
            Some(Sale {
                reference: reference.to_string(),
                wallet: wallet.to_ascii_lowercase(),
                catalog,
            })
        },
    )
}

/// The wallets of `<world>.minted`'s lines past the first `seen`, and how
/// many lines there are now. The file only grows (the minter appends), so a
/// count is the whole of the reader's memory.
pub fn minted_since(text: &str, seen: usize) -> (Vec<String>, usize) {
    let lines: Vec<&str> = text.lines().collect();
    let wallets = lines
        .iter()
        .skip(seen)
        .filter_map(|l| parse(l).flatten())
        .map(|s| s.wallet)
        .collect();
    (wallets, lines.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::skin::SkinDef;

    const DAVE: &str = "0x00000000000000000000000000000000000000d4";

    fn content() -> SkinContent {
        let mut sc = SkinContent::EMPTY;
        for (i, catalog) in [7u16, 10].into_iter().enumerate() {
            sc.defs[i] = SkinDef {
                catalog,
                covers: 0,
                unlock: i == 1,
            };
        }
        sc.count = 2;
        sc
    }

    /// A sale waits until the wallet holds its row, survives a restart, and
    /// a second sale after the first settled is its own line.
    #[test]
    fn a_sale_waits_for_its_token_and_outlives_a_restart() {
        let dir = std::env::temp_dir().join(format!("mints-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("world");
        let _ = std::fs::remove_file(mints_path(&base));
        let sc = content();
        let mut rug = SkinSet::EMPTY;
        rug.insert(1);
        let mut m = Mints::open(Some(&base));
        assert!(!m.waiting(DAVE, rug, &sc));
        assert_eq!(m.sell(DAVE, rug, &sc, 1_792_400_000), 1);
        assert!(m.waiting(DAVE, rug, &sc));
        let msg = m.take_write().expect("a sale is written");
        crate::gifts::write_atomic(&msg.path, &msg.text).unwrap();
        assert!(msg.text.contains(&format!("1792400000 {DAVE} 10\n")));
        let mut again = Mints::open(Some(&base));
        assert!(again.waiting(DAVE, rug, &sc), "kept across a restart");
        // The platform answers without it: still owed.
        again.settle(DAVE, SkinSet::EMPTY, &sc);
        assert!(again.waiting(DAVE, rug, &sc));
        again.settle(DAVE, rug, &sc);
        assert!(again.is_empty(), "the token is the proof now");
        assert!(again.take_write().is_some());
        assert_eq!(again.sell(DAVE, rug, &sc, 1_792_500_000), 1);
        assert_eq!(again.sales[0].reference, "1792500000");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The minted log is read past what was seen, a bad line costs only
    /// itself, and the count moves with the file.
    #[test]
    fn the_minted_log_is_read_from_where_it_was_left() {
        let text = format!(
            "1 {DAVE} 10 0xabc\nnot a line\n2 {} 10\n",
            DAVE.to_uppercase()
        );
        let (wallets, n) = minted_since(&text, 0);
        assert_eq!(n, 3);
        assert_eq!(wallets.len(), 1, "an upper-case 0X is not a wallet line");
        let text = format!("1 {DAVE} 10 0xabc\n3 {DAVE} 10 0xdef\n");
        assert_eq!(minted_since(&text, 1), (vec![DAVE.to_string()], 2));
        assert_eq!(minted_since(&text, 2), (vec![], 2));
    }
}
