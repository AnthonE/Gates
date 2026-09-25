//! Skins on screen (skins v0): the arithmetic the craft panel's skin picker,
//! the inventory's re-skin key and the renderer's tint share. Pure, like the
//! rest of `ui`, so all of it is tested without a window.
//!
//! The shape is Rust's. A skin is picked on the craft menu or changed on an
//! item you hold at a bench; you choose among the skins **you own** for that
//! item; and a skin the platform has not said you own is shown with its
//! price and where it is sold, never as a choice.

use protocol::{SkinCatalog, SkinRow, COIN_ELO, COIN_ORBS};
use sim_core::skin::{SkinSet, NO_SKIN};

/// What a skin costs, as a list says it: `250 ELO`, `12 ORBS`, or that it
/// is not on sale yet. Bare tickers, never a `$`.
pub fn price_label(row: &SkinRow) -> String {
    match row.coin {
        COIN_ELO => format!("{} ELO", row.price),
        COIN_ORBS => format!("{} ORBS", row.price),
        _ => "not on sale yet".to_string(),
    }
}

/// A skin's display name, or `None` if this client's catalog does not have
/// it (an item wearing an id the shard's content no longer names).
pub fn name(cat: &SkinCatalog, catalog: u16) -> Option<String> {
    let i = cat.index_of(catalog)?;
    Some(String::from_utf8_lossy(cat.name(i)).into_owned())
}

/// The skin the re-skin key moves `item` to from `current`: the next skin
/// in catalog order that fits the item and that this player owns, wrapping
/// through the item's own look. With nothing owned it is always the plain
/// look, which is also how a skin comes off.
pub fn next_skin(cat: &SkinCatalog, owned: &SkinSet, item: u16, current: u16) -> u16 {
    let mut choices = [NO_SKIN; sim_core::limits::MAX_SKINS + 1];
    let mut n = 1; // the plain look is always a choice
    for (i, row) in cat.rows().iter().enumerate() {
        if row.covers == item && owned.has(i) {
            choices[n] = row.catalog;
            n += 1;
        }
    }
    let at = choices[..n].iter().position(|c| *c == current);
    match at {
        Some(i) => choices[(i + 1) % n],
        // Wearing something we do not own or no longer know: the next pick
        // is the first owned skin, or the plain look.
        None => choices[1 % n],
    }
}

/// The tint as linear-ish colour multipliers in `0.0..=1.0`, for a
/// material's base colour or an icon's.
pub fn tint_factors(tint: [u8; 3]) -> [f32; 3] {
    [
        tint[0] as f32 / 255.0,
        tint[1] as f32 / 255.0,
        tint[2] as f32 / 255.0,
    ]
}

/// The tint an item wearing `catalog` is drawn with, if the catalog has
/// the row. `None` draws the item's own look.
pub fn tint_of(cat: &SkinCatalog, catalog: u16) -> Option<[f32; 3]> {
    let i = cat.index_of(catalog)?;
    Some(tint_factors(cat.rows[i].tint))
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::COIN_NONE;

    fn cat() -> SkinCatalog {
        let mut c = SkinCatalog::EMPTY;
        let rows = [
            (7u16, 3u16, COIN_ELO, 250u32),
            (8, 4, COIN_NONE, 0),
            (9, 3, COIN_ORBS, 12),
        ];
        for (i, (catalog, covers, coin, price)) in rows.into_iter().enumerate() {
            c.set(
                i,
                format!("Skin {catalog}").as_bytes(),
                SkinRow {
                    catalog,
                    covers,
                    tint: [255, 128, 0],
                    coin,
                    price,
                },
            )
            .unwrap();
        }
        c.count = 3;
        c
    }

    #[test]
    fn a_price_says_its_coin_bare_or_that_it_is_not_for_sale() {
        let c = cat();
        assert_eq!(price_label(&c.rows[0]), "250 ELO");
        assert_eq!(price_label(&c.rows[1]), "not on sale yet");
        assert_eq!(price_label(&c.rows[2]), "12 ORBS");
        assert!(!price_label(&c.rows[0]).contains('$'));
    }

    #[test]
    fn the_reskin_key_cycles_owned_skins_through_the_plain_look() {
        let c = cat();
        let mut owned = SkinSet::EMPTY;
        assert_eq!(next_skin(&c, &owned, 3, NO_SKIN), NO_SKIN, "nothing owned");
        assert_eq!(
            next_skin(&c, &owned, 3, 7),
            NO_SKIN,
            "an unowned skin comes off"
        );
        owned.insert(0); // catalog 7, item 3
        owned.insert(1); // catalog 8, item 4 — not this item's
        owned.insert(2); // catalog 9, item 3
        assert_eq!(next_skin(&c, &owned, 3, NO_SKIN), 7);
        assert_eq!(next_skin(&c, &owned, 3, 7), 9);
        assert_eq!(next_skin(&c, &owned, 3, 9), NO_SKIN, "and round again");
        assert_eq!(next_skin(&c, &owned, 3, 555), 7, "an unknown id restarts");
    }

    #[test]
    fn names_and_tints_come_from_the_catalog_or_not_at_all() {
        let c = cat();
        assert_eq!(name(&c, 9).as_deref(), Some("Skin 9"));
        assert_eq!(name(&c, 1), None);
        assert_eq!(tint_of(&c, NO_SKIN), None, "the plain look has no tint");
        let t = tint_of(&c, 7).unwrap();
        assert!((t[0] - 1.0).abs() < 1e-6 && t[2] == 0.0);
    }
}
