//! A speaker's offers (wire v130): the gift topics THE OUTFITTER hands skins
//! over on, free or for a price. The talk panel draws each as a card with
//! the skins it gives, what it costs and one button, and the speaker wears a
//! mark over their head while one is still to be had. Pure, like the rest of
//! `ui`, so all of it is tested without a window.
//!
//! Asking a gift topic *is* taking it: on the ask the server gives the
//! skins, or takes the price and mints the token to the wallet
//! (`server/src/mints.rs`). So walking up never asks one
//! (`SpeakerView::greeting`), and the button says what the press does and
//! what it costs.

use client_core::arc::SpeakerView;
use protocol::{SkinCatalog, TopicOffer};
use sim_core::skin::SkinSet;

/// Where this player stands with one offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Every row is already theirs.
    Owned,
    /// Paid for, and the token is on its way to the wallet.
    Minting,
    /// Signed out: no wallet to give it to.
    Guest,
    /// Priced, and the pack holds `have`, less than the price.
    Short { have: u32 },
    /// One press takes it.
    Ready,
}

impl Standing {
    /// Still to be had, so worth the mark over the speaker's head.
    pub fn open(self) -> bool {
        matches!(
            self,
            Standing::Guest | Standing::Short { .. } | Standing::Ready
        )
    }
}

/// Whether every row `offer` hands over is in `owned`. A row this client's
/// catalog does not know is not owned.
pub fn owns_all(offer: &TopicOffer, cat: &SkinCatalog, owned: &SkinSet) -> bool {
    offer
        .catalogs()
        .iter()
        .all(|&c| cat.index_of(c).is_some_and(|i| owned.has(i)))
}

/// Where this player stands: owning wins, then a payment on its way, then
/// signing in, then the price.
pub fn standing(offer: &TopicOffer, owns: bool, minting: bool, guest: bool, have: u32) -> Standing {
    if owns {
        Standing::Owned
    } else if minting {
        Standing::Minting
    } else if guest {
        Standing::Guest
    } else if have < offer.count as u32 {
        Standing::Short { have }
    } else {
        Standing::Ready
    }
}

/// Speaker `s`'s offers by topic, with where this player stands on each.
/// `have` counts an item in the pack.
pub fn offers<'a>(
    s: &'a SpeakerView,
    cat: &'a SkinCatalog,
    owned: &'a SkinSet,
    guest: bool,
    have: impl Fn(u16) -> u32 + 'a,
) -> impl Iterator<Item = (u8, TopicOffer, Standing)> + 'a {
    (0..s.n_topics).filter_map(move |t| {
        let o = s.offer(t as usize);
        if !o.gift {
            return None;
        }
        let minting = s.minting & (1 << t) != 0;
        let held = if o.priced() { have(o.item) } else { 0 };
        let st = standing(&o, owns_all(&o, cat, owned), minting, guest, held);
        Some((t, o, st))
    })
}

/// How many of speaker `s`'s offers this player can still take: what the
/// prompt, the panel's head and the mark over the speaker count.
pub fn open_count(
    s: &SpeakerView,
    cat: &SkinCatalog,
    owned: &SkinSet,
    have: impl Fn(u16) -> u32,
) -> u8 {
    // Signing in moves an offer between two open standings, never out of
    // them, so the count does not need to know.
    offers(s, cat, owned, false, have)
        .filter(|(_, _, st)| st.open())
        .count() as u8
}

/// What the price chip says: `FREE`, or `100 JUNK`.
pub fn price_label(offer: &TopicOffer, item: &str) -> String {
    if offer.priced() {
        format!("{} {}", offer.count, item.to_uppercase())
    } else {
        "FREE".to_string()
    }
}

/// What the card's button says, or `None` where there is nothing to press.
pub fn button_label(st: Standing, offer: &TopicOffer, price: &str) -> Option<String> {
    match st {
        Standing::Owned | Standing::Minting => None,
        Standing::Guest => Some("SIGN IN TO CLAIM".to_string()),
        Standing::Short { .. } | Standing::Ready if offer.priced() => {
            Some(format!("BUY · {price}"))
        }
        Standing::Short { .. } | Standing::Ready => Some("CLAIM".to_string()),
    }
}

/// The line under the card's skins: what pressing does, or where it stands.
pub fn note(st: Standing, offer: &TopicOffer, price: &str) -> String {
    match st {
        Standing::Owned => "yours — kept past every wipe".to_string(),
        Standing::Minting => "paid — minting to your wallet, a minute or two".to_string(),
        Standing::Guest => "it goes to a wallet: sign in to take it".to_string(),
        Standing::Short { have } => format!("costs {price} — you carry {have}"),
        Standing::Ready if offer.priced() => {
            "minted to your wallet as a token — sell it and it goes with it".to_string()
        }
        Standing::Ready => "free, once — kept past every wipe".to_string(),
    }
}

/// One skin an offer hands over, as its card lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferRow {
    pub name: String,
    /// What it is: a look for an item, or the unlock to craft one.
    pub what: String,
    /// The item row it fits, whose picture the card wears in `tint`.
    pub covers: u16,
    pub tint: [u8; 3],
}

/// The skins `offer` hands over that this client's catalog names. `item`
/// names an item row.
pub fn rows(offer: &TopicOffer, cat: &SkinCatalog, item: impl Fn(u16) -> String) -> Vec<OfferRow> {
    offer
        .catalogs()
        .iter()
        .filter_map(|&c| {
            let i = cat.index_of(c)?;
            let r = cat.rows[i];
            Some(OfferRow {
                name: String::from_utf8_lossy(cat.name(i)).into_owned(),
                what: if r.unlock {
                    format!("unlock: lets you craft the {}", item(r.covers))
                } else {
                    format!("a look for the {}", item(r.covers))
                },
                covers: r.covers,
                tint: r.tint,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{SkinRow, ARC_GIFT_GIVEN, ARC_GIFT_NONE, ARC_GIFT_OWNED, ARC_GIFT_WAITING};

    fn cat() -> SkinCatalog {
        let mut c = SkinCatalog::EMPTY;
        for (i, (catalog, unlock)) in [(7u16, false), (8, false), (10, true)]
            .into_iter()
            .enumerate()
        {
            c.set(
                i,
                format!("Skin {catalog}").as_bytes(),
                SkinRow {
                    catalog,
                    covers: 3,
                    tint: [1, 2, 3],
                    coin: protocol::COIN_NONE,
                    price: 0,
                    texture: [0; 8],
                    unlock,
                },
            )
            .unwrap();
        }
        c.count = 3;
        c
    }

    fn free() -> TopicOffer {
        TopicOffer {
            gift: true,
            n: 2,
            catalogs: [7, 8, 0, 0],
            ..TopicOffer::default()
        }
    }

    fn priced() -> TopicOffer {
        TopicOffer {
            gift: true,
            n: 1,
            catalogs: [10, 0, 0, 0],
            item: 41,
            count: 100,
        }
    }

    /// The Outfitter as the shard drips it: a free gift first, a priced one,
    /// then small talk.
    fn outfitter() -> SpeakerView {
        let mut s = SpeakerView {
            known: true,
            n_topics: 3,
            ..SpeakerView::default()
        };
        s.offers[0] = free();
        s.offers[1] = priced();
        s
    }

    #[test]
    fn owning_wins_then_minting_then_signing_in_then_the_price() {
        let o = priced();
        assert_eq!(standing(&o, true, true, true, 0), Standing::Owned);
        assert_eq!(standing(&o, false, true, true, 0), Standing::Minting);
        assert_eq!(standing(&o, false, false, true, 0), Standing::Guest);
        assert_eq!(
            standing(&o, false, false, false, 99),
            Standing::Short { have: 99 }
        );
        assert_eq!(standing(&o, false, false, false, 100), Standing::Ready);
        assert_eq!(standing(&free(), false, false, false, 0), Standing::Ready);
        assert!(!Standing::Owned.open() && !Standing::Minting.open());
        assert!(Standing::Ready.open() && Standing::Guest.open());
    }

    #[test]
    fn owning_is_every_row_and_an_unknown_row_is_not_owned() {
        let c = cat();
        let mut owned = SkinSet::EMPTY;
        owned.insert(0);
        assert!(!owns_all(&free(), &c, &owned), "one of two");
        owned.insert(1);
        assert!(owns_all(&free(), &c, &owned));
        let stranger = TopicOffer {
            catalogs: [99, 0, 0, 0],
            n: 1,
            ..free()
        };
        assert!(!owns_all(&stranger, &c, &SkinSet::all(3)));
    }

    #[test]
    fn the_button_says_what_the_press_does_and_costs() {
        let p = price_label(&priced(), "Junk");
        assert_eq!(p, "100 JUNK");
        assert_eq!(price_label(&free(), "Junk"), "FREE");
        assert_eq!(
            button_label(Standing::Ready, &priced(), &p).as_deref(),
            Some("BUY · 100 JUNK")
        );
        assert_eq!(
            button_label(Standing::Short { have: 3 }, &priced(), &p).as_deref(),
            Some("BUY · 100 JUNK")
        );
        assert_eq!(
            button_label(Standing::Ready, &free(), "FREE").as_deref(),
            Some("CLAIM")
        );
        assert_eq!(
            button_label(Standing::Guest, &priced(), &p).as_deref(),
            Some("SIGN IN TO CLAIM")
        );
        assert_eq!(button_label(Standing::Owned, &free(), "FREE"), None);
        assert_eq!(button_label(Standing::Minting, &priced(), &p), None);
        assert!(note(Standing::Short { have: 34 }, &priced(), &p).contains("34"));
    }

    #[test]
    fn walking_up_greets_with_small_talk_never_a_gift() {
        let mut s = outfitter();
        assert_eq!(s.greeting(), Some(2));
        s.offers[2] = free();
        assert_eq!(s.greeting(), None, "all gifts: nothing is asked");
    }

    #[test]
    fn a_paid_offer_is_minting_until_an_answer_says_otherwise() {
        let mut s = outfitter();
        s.heard(1, ARC_GIFT_GIVEN);
        assert_eq!(s.minting, 0b010, "paid: on its way");
        s.heard(0, ARC_GIFT_GIVEN);
        assert_eq!(s.minting, 0b010, "a free gift is given, not minted");
        s.heard(2, ARC_GIFT_NONE);
        assert_eq!(s.minting, 0b010, "small talk changes nothing");
        s.heard(1, ARC_GIFT_OWNED);
        assert_eq!(s.minting, 0);
        s.heard(1, ARC_GIFT_WAITING);
        assert_eq!(s.minting, 0b010);
    }

    #[test]
    fn offers_lists_gifts_only_with_the_pack_counted_for_the_priced() {
        let s = outfitter();
        let c = cat();
        let got: Vec<_> = offers(&s, &c, &SkinSet::EMPTY, false, |item| {
            assert_eq!(item, 41, "only a priced offer counts the pack");
            34
        })
        .collect();
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].0, got[0].2), (0, Standing::Ready));
        assert_eq!((got[1].0, got[1].2), (1, Standing::Short { have: 34 }));
        assert_eq!(open_count(&s, &c, &SkinSet::EMPTY, |_| 34), 2);
        assert_eq!(
            open_count(&s, &c, &SkinSet::all(2), |_| 34),
            1,
            "the gift is owned"
        );
        let names = rows(&got[1].1, &c, |_| "Rug".into());
        assert_eq!(names[0].name, "Skin 10");
        assert!(names[0].what.contains("craft"));
        assert!(rows(&got[0].1, &c, |_| "Bat".into())[1]
            .what
            .contains("look"));
    }
}
