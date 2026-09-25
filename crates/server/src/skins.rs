//! **What skins does this player own?** Rust asks the player's Steam
//! inventory (the client sends a Steam-signed snapshot, the server checks it
//! belongs to that SteamID and answers `HasItem`). A Gates shard asks the
//! platform instead, over the route elo already serves for its inventory
//! page: `GET {origin}/api/items/of/{wallet}` (`meter/items.py` in
//! `AnthonE/scry-forge`), which lists the wallet's item-contract tokens and
//! re-reads each one's `catalogId` off the chain. A catalog id is exactly
//! what `content/skins.toml` calls `catalog`, so the answer maps straight
//! onto the baked rows (`sim_core::skin`).
//!
//! ## Two answers that matter, and the policy runs the other way to the
//! ## ticket door's
//!
//! [`Owned::Known`] replaces what the sim holds; [`Owned::Unknown`] (a
//! timeout, a non-2xx, `reachable: false`, a body we cannot read) changes
//! nothing. `entitle.rs` fails *open* on its unknown, because the game is
//! the player's and an outage must not lock them out. Here the thing at
//! stake is a sale: "we could not look" never grants a skin, and it never
//! strips one either, because the sim keeps the last set it was told.
//!
//! ## Asked at the door and on request, never on a timer
//!
//! Once when a player joins, and again when their client says it bought
//! something (`ACT_SKINS_REFRESH`), at most once per [`REFRESH_COOLDOWN`].
//! Not swept: every read of that route is a block-explorer call elo pays
//! for (its `CLAUDE.md`: *the explorer door is METERED*), and a 100-player
//! sweep would spend that budget around the clock for purchases that happen
//! a few times a day. Rust's own client pushes a fresh inventory on Steam's
//! "inventory changed" callback for the same reason.
//!
//! ## Where this runs
//!
//! On a blocking thread spawned by the accept loop (`net.rs`), never on the
//! sim thread. What it learns enters the world as `Command::SkinsOwned`, a
//! command only the server mints, so a replay owns what the session owned.

use sim_core::skin::{SkinContent, SkinSet};
use std::time::Duration;

/// How long one read may take. The route re-reads every token off the chain
/// (`instanceOf` per item), so it is slower than the ticket check's one
/// `balanceOf`.
pub const DEFAULT_TIMEOUT_SECS: u64 = 8;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(DEFAULT_TIMEOUT_SECS);

/// The least time between two reads for one player. A client that asks
/// sooner is ignored, not queued: the answer it wants is at most this old.
pub const REFRESH_COOLDOWN: Duration = Duration::from_secs(15);

/// The far end's own ceiling on tokens re-read per wallet
/// (`SCRY_ITEM_MAX_READ`, 200), asked for explicitly so a smaller default
/// there cannot silently hide skins here.
pub const ITEMS_LIMIT: u32 = 200;

/// Wall 4 on the read. A full page of 200 items with their derived fields
/// is well under this; a body past it is refused rather than truncated,
/// because a truncated list parses into fewer skins and reads as a sale
/// that never happened.
pub const MAX_RESPONSE_BYTES: usize = 512 * 1024;

/// Where to ask, from `shard.toml`. Absent ⇒ nobody owns anything, which is
/// every test's and every unarmed shard's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// `skins_origin`: the platform, no trailing slash.
    pub origin: Option<String>,
    /// `skins_all`: every player owns every skin. **A dev and test knob**:
    /// it is how a look is seen before anything is sold, and it is refused
    /// on a shard with a ticket door (`config.rs`), because an official
    /// shard lending paid skins to everyone is the one thing Rust's server
    /// rules forbid community servers too.
    pub all: bool,
    pub timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self::off()
    }
}

impl Config {
    pub fn off() -> Self {
        Self {
            origin: None,
            all: false,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// What a read said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owned {
    /// The set, over this shard's baked rows. Replaces the sim's.
    Known(SkinSet),
    /// We could not look. Changes nothing.
    Unknown,
}

/// One player's owned set. `wallet` is `None` for a guest, who owns
/// nothing. Blocking; the caller is a `spawn_blocking` task.
pub fn owned_of(cfg: &Config, wallet: Option<&str>, sc: &SkinContent) -> Owned {
    if cfg.all {
        return Owned::Known(SkinSet::all(sc.count as usize));
    }
    let (Some(origin), Some(wallet)) = (cfg.origin.as_deref(), wallet) else {
        return Owned::Known(SkinSet::EMPTY);
    };
    if sc.count == 0 {
        // A catalog with no rows maps every answer to the empty set, so
        // there is nothing a read could change.
        return Owned::Known(SkinSet::EMPTY);
    }
    if !crate::entitle::is_wallet(wallet) {
        // Our bug, not the chain's answer: the same `Unknown` the ticket
        // door gives it.
        return Owned::Unknown;
    }
    let url = format!("{origin}/api/items/of/{wallet}?limit={ITEMS_LIMIT}");
    match crate::entitle::get_capped(&url, cfg.timeout, MAX_RESPONSE_BYTES) {
        Some(body) => parse(&body, sc),
        None => Owned::Unknown,
    }
}

/// An `items/of` body → the owned set over `sc`'s rows.
///
/// Hand-scanned, `entitle.rs`'s reasoning: the only facts this may act on
/// are the two flags that say *could not look* and *no contract*, and the
/// `catalog_id` of each listed item. An id this shard's content does not
/// know is skipped, as is an item whose instance could not be read (it has
/// no `catalog_id`): both fail toward owning less.
pub fn parse(body: &str, sc: &SkinContent) -> Owned {
    if flag(body, "\"reachable\"") == Some(false) {
        return Owned::Unknown;
    }
    if flag(body, "\"configured\"") == Some(false) {
        // No item contract on the platform yet: nobody can hold a skin.
        return Owned::Known(SkinSet::EMPTY);
    }
    let Some(items_at) = body.find("\"items\"") else {
        return Owned::Unknown;
    };
    let mut set = SkinSet::EMPTY;
    let key = "\"catalog_id\"";
    let mut rest = &body[items_at..];
    while let Some(at) = rest.find(key) {
        rest = &rest[at + key.len()..];
        let Some(value) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let value = value.trim_start();
        let digits = value.bytes().take_while(u8::is_ascii_digit).count();
        let Ok(id) = value[..digits].parse::<u64>() else {
            continue;
        };
        if let Some(row) = u16::try_from(id).ok().and_then(|id| sc.row_of(id)) {
            set.insert(row);
        }
    }
    Owned::Known(set)
}

// ── what the store charges ───────────────────────────────────────────────────
//
// The price a store screen shows is the one the platform's item store posts
// on chain (`EloItemStore`, read back by `GET {origin}/api/items/store/gates`,
// `meter/itemstore.py`), not `content/skins.toml`'s. Two reasons: the sale
// happens on the platform's page, so a content price is a second number that
// can disagree with the one charged; and content prices are in the content
// hash, so repricing from here would refuse every save on the next boot.
// Content prices stand only on a shard with no `skins_origin`.

/// The title's slug on the platform (`/api/items/store/{slug}`).
pub const STORE_TITLE: &str = "gates";

/// How often the shard re-reads the store. That route answers from
/// `eth_call`s behind a short cache — not the metered explorer — and a price
/// moves only when the store's owner re-lists, so minutes are plenty.
pub const PRICES_EVERY: Duration = Duration::from_secs(300);

/// At most `MAX_SKINS` rows of well under 1 KiB each.
pub const MAX_PRICES_BYTES: usize = 256 * 1024;

/// One row's price as the store posts it: `(protocol::COIN_*, whole coins)`.
pub type Price = (u8, u32);

/// What a store read said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prices {
    /// One entry per baked row: `Some` on sale, `None` not. Replaces the
    /// rows' prices. Boxed because it crosses a channel beside `Unknown`;
    /// the accept loop copies it into the ring's fixed-size message.
    Known(Box<[Option<Price>; sim_core::limits::MAX_SKINS]>),
    /// We could not look. Changes nothing.
    Unknown,
}

/// The store's prices over `sc`'s rows. Blocking; the caller is a
/// `spawn_blocking` task. An unarmed shard or the dev knob never asks.
pub fn prices_of(cfg: &Config, sc: &SkinContent) -> Prices {
    let Some(origin) = cfg.origin.as_deref() else {
        return Prices::Unknown;
    };
    if cfg.all || sc.count == 0 {
        return Prices::Unknown;
    }
    let url = format!("{origin}/api/items/store/{STORE_TITLE}");
    match crate::entitle::get_capped(&url, cfg.timeout, MAX_PRICES_BYTES) {
        Some(body) => parse_prices(&body, sc),
        None => Prices::Unknown,
    }
}

/// An `items/store` body → a price per row. No store on the platform yet
/// (`configured: false`) is an answer — nothing is on sale. A store we could
/// not read is not. A row the store sells in a coin the wire cannot name, or
/// at a price past `u32` whole coins, shows as not on sale here; the
/// platform's own page still sells it.
pub fn parse_prices(body: &str, sc: &SkinContent) -> Prices {
    use serde_json::Value;
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return Prices::Unknown;
    };
    let mut out = Box::new([None; sim_core::limits::MAX_SKINS]);
    if v.get("configured") == Some(&Value::Bool(false)) {
        return Prices::Known(out);
    }
    if v.get("reachable") != Some(&Value::Bool(true)) {
        return Prices::Unknown;
    }
    let Some(skins) = v.get("skins").and_then(Value::as_array) else {
        return Prices::Unknown;
    };
    for s in skins {
        let Some(row) = s
            .get("catalog_id")
            .and_then(Value::as_u64)
            .and_then(|id| u16::try_from(id).ok())
            .and_then(|id| sc.row_of(id))
        else {
            continue;
        };
        if s.get("on_sale") != Some(&Value::Bool(true)) {
            continue;
        }
        let coin = match s
            .get("coin")
            .and_then(|c| c.get("symbol"))
            .and_then(Value::as_str)
        {
            Some("ORBS") => protocol::COIN_ORBS,
            Some("ELO") => protocol::COIN_ELO,
            _ => continue,
        };
        let Some(price) = s
            .get("price_whole")
            .and_then(Value::as_u64)
            .and_then(|p| u32::try_from(p).ok())
            .filter(|&p| p > 0)
        else {
            continue;
        };
        out[row] = Some((coin, price));
    }
    Prices::Known(out)
}

/// The boolean after `key:`, if it is one.
fn flag(body: &str, key: &str) -> Option<bool> {
    let at = body.find(key)? + key.len();
    let rest = body[at..].trim_start().strip_prefix(':')?.trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::skin::SkinDef;

    /// Catalog ids 1, 2 and 900, in that row order.
    fn catalog() -> SkinContent {
        let mut sc = SkinContent::EMPTY;
        for (i, catalog) in [1u16, 2, 900].into_iter().enumerate() {
            sc.defs[i] = SkinDef { catalog, covers: 3 };
        }
        sc.count = 3;
        sc
    }

    /// The live route's success shape (`meter/items.py::_items_of_blocking`),
    /// trimmed to the keys that matter plus one that must not.
    const OWNS_TWO: &str = r#"{"wallet":"0xAb","collection":"0x9","items":[
        {"token_id":4,"catalog_id":900,"mint_kind":2,"mint_kind_name":"store",
         "mint_ref":"0x00","derived":null},
        {"token_id":7,"catalog_id": 1,"mint_kind":2,"derived":null},
        {"token_id":8,"catalog_id":77,"mint_kind":2},
        {"token_id":9,"error":"instanceOf could not be read for this token"}
      ],"count":4,"balance_onchain":4,"index_disagrees":false,"truncated":null}"#;

    #[test]
    fn a_listed_item_owns_its_row_and_an_unknown_id_owns_nothing() {
        let Owned::Known(set) = parse(OWNS_TWO, &catalog()) else {
            panic!("a readable page is an answer");
        };
        assert!(set.has(2), "catalog 900 is row 2");
        assert!(set.has(0), "catalog 1 is row 0, whitespace and all");
        assert!(!set.has(1), "catalog 2 was not listed");
        assert_eq!(set.count(), 2, "77 is not a skin this shard knows");
    }

    #[test]
    fn could_not_look_changes_nothing_and_no_contract_owns_nothing() {
        let failed = r#"{"wallet":"0xAb","items":[],"count":0,"reachable":false,
            "error":"could not enumerate this wallet's items"}"#;
        assert_eq!(parse(failed, &catalog()), Owned::Unknown);
        let dark = r#"{"configured":false,"deployed":false,"why":"no item contract",
            "wallet":"0xAb","items":[],"count":0}"#;
        assert_eq!(parse(dark, &catalog()), Owned::Known(SkinSet::EMPTY));
        assert_eq!(parse("<html>502</html>", &catalog()), Owned::Unknown);
    }

    #[test]
    fn an_unarmed_shard_and_a_guest_own_nothing_and_the_dev_knob_owns_all() {
        let sc = catalog();
        let wallet = Some("0x00000000000000000000000000000000000000a1");
        assert_eq!(
            owned_of(&Config::off(), wallet, &sc),
            Owned::Known(SkinSet::EMPTY)
        );
        let armed = Config {
            origin: Some("https://origin.test".into()),
            ..Config::off()
        };
        assert_eq!(owned_of(&armed, None, &sc), Owned::Known(SkinSet::EMPTY));
        assert_eq!(
            owned_of(&armed, Some("not a wallet"), &sc),
            Owned::Unknown,
            "our bug is could-not-look, never a no"
        );
        let all = Config {
            all: true,
            ..Config::off()
        };
        assert_eq!(owned_of(&all, None, &sc), Owned::Known(SkinSet::all(3)));
    }

    /// The live store's shape (`meter/itemstore.py::card`), trimmed.
    const STORE: &str = r#"{"game":"gates","configured":true,"reachable":true,
      "store":"0x5","item":"0xa","store_can_mint":true,"skins":[
        {"catalog_id":900,"name":"Bone Bow","on_sale":true,"why_not":null,
         "coin":{"address":"0xc","symbol":"ORBS","decimals":18},
         "price":"2500000000000000000","price_text":"2.5 ORBS","price_whole":3,"burns":true},
        {"catalog_id":1,"name":"Obsidian Rock","on_sale":false,"why_not":"not on sale",
         "coin":null,"price":null,"price_whole":null},
        {"catalog_id":2,"name":"Ember Hatchet","on_sale":true,
         "coin":{"symbol":"ELO","decimals":18},"price_whole":10},
        {"catalog_id":77,"name":"not ours","on_sale":true,
         "coin":{"symbol":"ORBS"},"price_whole":1}
      ]}"#;

    fn known(p: Prices) -> [Option<Price>; sim_core::limits::MAX_SKINS] {
        match p {
            Prices::Known(rows) => *rows,
            Prices::Unknown => panic!("a readable store is an answer"),
        }
    }

    #[test]
    fn a_listed_skin_takes_the_store_price_and_an_unlisted_one_none() {
        let rows = known(parse_prices(STORE, &catalog()));
        assert_eq!(
            rows[2],
            Some((protocol::COIN_ORBS, 3)),
            "catalog 900, rounded up"
        );
        assert_eq!(rows[0], None, "catalog 1 is not on sale");
        assert_eq!(rows[1], Some((protocol::COIN_ELO, 10)));
        assert!(
            rows[3..].iter().all(Option::is_none),
            "77 is no row of ours"
        );
    }

    #[test]
    fn no_store_is_nothing_on_sale_and_a_failed_read_changes_nothing() {
        let dark = r#"{"game":"gates","configured":false,"why":"no item store",
            "skins":[{"catalog_id":1,"on_sale":false}]}"#;
        assert_eq!(
            parse_prices(dark, &catalog()),
            Prices::Known(Box::new([None; sim_core::limits::MAX_SKINS]))
        );
        let unread = r#"{"game":"gates","configured":true,"reachable":false,"skins":[]}"#;
        assert_eq!(parse_prices(unread, &catalog()), Prices::Unknown);
        assert_eq!(
            parse_prices("<html>502</html>", &catalog()),
            Prices::Unknown
        );
        assert_eq!(
            parse_prices(r#"{"configured":true,"reachable":true}"#, &catalog()),
            Prices::Unknown
        );
    }

    #[test]
    fn a_coin_the_wire_cannot_name_or_a_price_it_cannot_carry_is_not_on_sale_here() {
        let odd = r#"{"configured":true,"reachable":true,"skins":[
            {"catalog_id":1,"on_sale":true,"coin":{"symbol":"JUNK"},"price_whole":5},
            {"catalog_id":2,"on_sale":true,"coin":{"symbol":"ORBS"},"price_whole":4294967296},
            {"catalog_id":900,"on_sale":true,"coin":{"symbol":"ORBS"},"price_whole":0}]}"#;
        let rows = known(parse_prices(odd, &catalog()));
        assert!(rows.iter().all(Option::is_none));
    }

    #[test]
    fn an_unarmed_shard_and_the_dev_knob_never_ask_the_store() {
        let sc = catalog();
        assert_eq!(prices_of(&Config::off(), &sc), Prices::Unknown);
        let all = Config {
            origin: Some("https://origin.test".into()),
            all: true,
            ..Config::off()
        };
        assert_eq!(prices_of(&all, &sc), Prices::Unknown);
    }
}
