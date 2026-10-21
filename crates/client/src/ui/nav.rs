//! Which page a key or a tab opens — Rust's inventory, crafting and clan
//! pages, and the strip across the top that walks between them.
//!
//! Rust keeps the pages apart: **Tab** is the inventory (your pack, your
//! body, whatever you are looting), **Q** is the crafting menu, and each key
//! toggles its own page — `inventory.toggle` and `inventory.togglecrafting`
//! in its console. Across the top of every one of them sits the same strip,
//! CONTACTS · CRAFTING · CLAN, and the button for the page you are on gives
//! its seat to INVENTORY ("moves around to replace the opened menu button").
//! Ours is ISLAND · CRAFTING · TEAM: the island's works and journal (`O`)
//! where Rust has contacts, and the team (`M`) where it has the clan.
//!
//! This is the whole rule, kept out of the Bevy system so it can be tested:
//! a page's own key closes it, any other page's key switches to that page,
//! and a button in the strip is a switch that never closes.

/// The screens a page key can land on. `Other` is everything that is not
/// one of the pages — the build wheel, the tech tree, a kiosk — which a page
/// key replaces rather than stacks on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Closed,
    Inventory,
    Crafting,
    Island,
    Team,
    Other,
}

/// What pressing `key`'s own binding does while `open` is up.
pub fn press(open: Page, key: Page) -> Page {
    if open == key {
        Page::Closed
    } else {
        key
    }
}

/// What clicking `tab` in the strip does: always that page.
pub fn click(tab: Page) -> Page {
    tab
}

/// The strip's three seats, left to right, with nothing open.
const SEATS: [Page; 3] = [Page::Island, Page::Crafting, Page::Team];

/// The buttons across the top while `open` is up: the three seats, with
/// the open page's seat handed to INVENTORY — so the strip never offers the
/// page you are on and never moves a button you did not click.
pub fn strip(open: Page) -> [Page; 3] {
    let mut out = SEATS;
    match open {
        Page::Closed | Page::Other | Page::Inventory => {}
        _ => {
            for seat in out.iter_mut() {
                if *seat == open {
                    *seat = Page::Inventory;
                }
            }
        }
    }
    out
}

/// Whether `open` draws the strip at all.
pub fn has_strip(open: Page) -> bool {
    !matches!(open, Page::Closed | Page::Other)
}

/// A page's name on its button.
pub fn label(page: Page) -> &'static str {
    match page {
        Page::Inventory => "INVENTORY",
        Page::Crafting => "CRAFTING",
        Page::Island => "ISLAND",
        Page::Team => "TEAM",
        Page::Closed | Page::Other => "",
    }
}

/// The key that opens a page by default, as a bracketed hint the binds
/// relabel (`render::binds::Keyed`).
pub fn key_hint(page: Page) -> &'static str {
    match page {
        Page::Inventory => "[Tab]",
        Page::Crafting => "[Q]",
        Page::Island => "[O]",
        Page::Team => "[M]",
        Page::Closed | Page::Other => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGES: [Page; 4] = [Page::Inventory, Page::Crafting, Page::Island, Page::Team];

    #[test]
    fn a_pages_own_key_closes_it_and_the_other_key_switches() {
        assert_eq!(press(Page::Closed, Page::Inventory), Page::Inventory);
        assert_eq!(press(Page::Closed, Page::Crafting), Page::Crafting);
        assert_eq!(press(Page::Inventory, Page::Inventory), Page::Closed);
        assert_eq!(press(Page::Crafting, Page::Crafting), Page::Closed);
        assert_eq!(press(Page::Inventory, Page::Crafting), Page::Crafting);
        assert_eq!(press(Page::Crafting, Page::Inventory), Page::Inventory);
        assert_eq!(press(Page::Team, Page::Island), Page::Island);
        assert_eq!(press(Page::Island, Page::Island), Page::Closed);
        // The tech tree is not a page: a page key replaces it.
        assert_eq!(press(Page::Other, Page::Inventory), Page::Inventory);
    }

    #[test]
    fn a_tab_click_never_closes() {
        for p in PAGES {
            assert_eq!(click(p), p);
        }
    }

    #[test]
    fn the_strip_never_offers_this_page_and_keeps_its_seats() {
        assert_eq!(strip(Page::Inventory), SEATS);
        for open in PAGES {
            assert!(has_strip(open));
            let offered = strip(open);
            assert!(!offered.contains(&open), "{open:?} offers itself");
            // Every other page is one click away.
            for p in PAGES.iter().filter(|p| **p != open) {
                assert!(offered.contains(p), "{open:?} hides {p:?}");
            }
            // A seat only ever changes to INVENTORY.
            for (seat, now) in SEATS.iter().zip(offered) {
                assert!(now == *seat || now == Page::Inventory);
            }
            for page in offered {
                assert!(!label(page).is_empty() && !key_hint(page).is_empty());
            }
        }
        assert!(!has_strip(Page::Closed) && !has_strip(Page::Other));
    }
}
