//! The front end's INVENTORY and ITEM STORE: the skins the platform sells for
//! Gates, and the ones a wallet owns. Pure, so it is tested without a window.
//!
//! The reference draws both as grids of item pictures beside its nav (the
//! operator's frames, 2026-10-21): the store as cards with a price, the
//! inventory as tiles with a name. The browser page reads the same two
//! documents (`client-web/web/app.js`, `fillStore`/`fillOwned`), so the two
//! front ends show the same catalog off the same answers.
//!
//! Buying is never here: a card hands off to the launcher's store
//! (`ui::hub`'s rule about money).

/// What one store row says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreSkin {
    /// The id the item contract mints it under (`content/skins.toml` `catalog`).
    pub catalog: u32,
    pub name: String,
    /// The covered item's display name, which is also its icon's key
    /// (`ui::icons::stem`).
    pub covers: String,
    pub tint: [u8; 3],
    /// What the card's price chip says, or `None` when it is not on sale.
    pub price: Option<String>,
    /// How play gives a skin nobody sells (the store row's `earn`, e.g. "Free
    /// in game"): the card says it in place of "coming soon".
    pub earn: Option<String>,
}

/// The platform's slug for this title (`/api/items/store/{slug}`).
pub const TITLE: &str = "gates";

/// Byte caps on the two reads: the shard's own (`server/skins.rs`), so the
/// client never takes a body the shard would refuse.
pub const MAX_STORE_BYTES: usize = 256 * 1024;
pub const MAX_OWNED_BYTES: usize = 256 * 1024;

pub fn store_url(origin: &str) -> String {
    format!("{origin}/api/items/store/{TITLE}")
}

/// `title` keeps the answer to this title's rows, so another game's catalog
/// ids can never read as Gates skins.
pub fn owned_url(origin: &str, wallet: &str) -> String {
    format!("{origin}/api/items/of/{wallet}?limit=200&title={TITLE}")
}

/// `#rrggbb` → bytes. Anything else is `None`, and the card draws the item's
/// own colours.
pub fn parse_tint(s: &str) -> Option<[u8; 3]> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// An `items/store` body → its rows, in the store's order.
///
/// A store with no contract (`configured: false`) still lists its looks, all
/// off sale — the browser page draws that as "coming soon" and so does this.
pub fn parse_store(body: &[u8]) -> Result<Vec<StoreSkin>, String> {
    use serde_json::Value;
    let v: Value =
        serde_json::from_slice(body).map_err(|_| "the item store sent something unreadable")?;
    let skins = v
        .get("skins")
        .and_then(Value::as_array)
        .ok_or("the item store lists no skins")?;
    let mut out = Vec::with_capacity(skins.len());
    for s in skins {
        let Some(catalog) = s
            .get("catalog_id")
            .and_then(Value::as_u64)
            .and_then(|c| u32::try_from(c).ok())
        else {
            continue;
        };
        let text = |k: &str| s.get(k).and_then(Value::as_str).map(str::trim);
        let price = (s.get("on_sale") == Some(&Value::Bool(true)))
            .then(|| price_text(s))
            .flatten();
        out.push(StoreSkin {
            catalog,
            name: text("name")
                .filter(|n| !n.is_empty())
                .unwrap_or("Skin")
                .to_string(),
            covers: text("covers").unwrap_or("").to_string(),
            tint: text("tint").and_then(parse_tint).unwrap_or([255; 3]),
            price,
            earn: text("earn").filter(|e| !e.is_empty()).map(str::to_string),
        });
    }
    Ok(out)
}

/// The store's own `price_text`, or `{whole} {symbol}` built from the coin.
fn price_text(s: &serde_json::Value) -> Option<String> {
    use serde_json::Value;
    if let Some(t) = s
        .get("price_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        return Some(t.to_string());
    }
    let whole = s.get("price_whole").and_then(Value::as_u64)?;
    let sym = s
        .get("coin")
        .and_then(|c| c.get("symbol"))
        .and_then(Value::as_str)?;
    Some(format!("{whole} {sym}"))
}

/// An `items/of` body → `(catalog id, how many)`, first-seen order.
///
/// `reachable: false` is the platform saying it could not look, which is an
/// error and not an empty wallet; `configured: false` is no item contract
/// yet, which is an empty wallet.
pub fn parse_owned(body: &[u8]) -> Result<Vec<(u32, u32)>, String> {
    use serde_json::Value;
    let v: Value = serde_json::from_slice(body).map_err(|_| "your wallet read was unreadable")?;
    if v.get("reachable") == Some(&Value::Bool(false)) {
        return Err("couldn't read your wallet just now".into());
    }
    if v.get("configured") == Some(&Value::Bool(false)) {
        return Ok(Vec::new());
    }
    let items = v
        .get("items")
        .and_then(Value::as_array)
        .ok_or("your wallet read listed no items")?;
    let mut out: Vec<(u32, u32)> = Vec::new();
    for id in items.iter().filter_map(|i| {
        i.get("catalog_id")
            .and_then(Value::as_u64)
            .and_then(|c| u32::try_from(c).ok())
    }) {
        match out.iter_mut().find(|(c, _)| *c == id) {
            Some((_, n)) => *n += 1,
            None => out.push((id, 1)),
        }
    }
    Ok(out)
}

/// The store's tabs — the reference has LIMITED and GENERAL; ours split on
/// what a player can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shelf {
    #[default]
    All,
    OnSale,
    Owned,
}

impl Shelf {
    pub const ALL: [Shelf; 3] = [Shelf::All, Shelf::OnSale, Shelf::Owned];

    pub fn label(self) -> &'static str {
        match self {
            Shelf::All => "ALL SKINS",
            Shelf::OnSale => "ON SALE",
            Shelf::Owned => "OWNED",
        }
    }

    /// Which rows of `store` this tab shows, in the store's order.
    pub fn rows(self, store: &[StoreSkin], owned: &[(u32, u32)]) -> Vec<usize> {
        let has = |c: u32| owned.iter().any(|(o, _)| *o == c);
        (0..store.len())
            .filter(|&i| match self {
                Shelf::All => true,
                Shelf::OnSale => store[i].price.is_some(),
                Shelf::Owned => has(store[i].catalog),
            })
            .collect()
    }
}

/// How many of `owned` this wallet holds of `catalog`.
pub fn count_of(owned: &[(u32, u32)], catalog: u32) -> u32 {
    owned
        .iter()
        .find(|(c, _)| *c == catalog)
        .map_or(0, |(_, n)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STORE: &str = r##"{"game":"gates","configured":true,"reachable":true,"skins":[
        {"catalog_id":1,"name":"Obsidian Bat","covers":"Bat","tint":"#463e54","on_sale":false,
         "earn":"Free in game"},
        {"catalog_id":4,"name":"Bone Bow","covers":"Hunting Bow","tint":"#ECE2CC",
         "on_sale":true,"price_whole":250,"coin":{"symbol":"JUNK"}},
        {"catalog_id":7,"name":"  ","covers":"Rock","tint":"red","on_sale":true,
         "price_text":"400000 JUNK"},
        {"name":"no id"}
    ]}"##;

    #[test]
    fn the_store_reads_names_tints_and_prices() {
        let s = parse_store(STORE.as_bytes()).expect("a store");
        assert_eq!(s.len(), 3, "a row with no catalog id is skipped");
        assert_eq!(s[0].tint, [0x46, 0x3e, 0x54]);
        assert_eq!(s[0].price, None, "off sale draws no price");
        assert_eq!(s[0].earn.as_deref(), Some("Free in game"));
        assert_eq!(s[1].earn, None);
        assert_eq!(s[1].tint, [0xec, 0xe2, 0xcc], "hex case does not matter");
        assert_eq!(s[1].price.as_deref(), Some("250 JUNK"));
        assert_eq!(s[2].name, "Skin", "a blank name is not drawn blank");
        assert_eq!(
            s[2].tint, [255; 3],
            "an unreadable tint is the item's own colours"
        );
        assert_eq!(s[2].price.as_deref(), Some("400000 JUNK"));
        assert!(parse_store(b"<html>").is_err());
    }

    #[test]
    fn owned_counts_duplicates_and_tells_unknown_from_empty() {
        let body =
            br#"{"items":[{"catalog_id":4},{"catalog_id":1},{"catalog_id":4},{"error":"x"}]}"#;
        assert_eq!(parse_owned(body).unwrap(), vec![(4, 2), (1, 1)]);
        assert_eq!(parse_owned(br#"{"configured":false}"#).unwrap(), Vec::new());
        assert!(parse_owned(br#"{"reachable":false,"items":[]}"#).is_err());
    }

    #[test]
    fn shelves_filter_the_store() {
        let s = parse_store(STORE.as_bytes()).unwrap();
        let owned = [(1, 1)];
        assert_eq!(Shelf::All.rows(&s, &owned), vec![0, 1, 2]);
        assert_eq!(Shelf::OnSale.rows(&s, &owned), vec![1, 2]);
        assert_eq!(Shelf::Owned.rows(&s, &owned), vec![0]);
        assert_eq!(count_of(&owned, 1), 1);
        assert_eq!(count_of(&owned, 4), 0);
    }
}
