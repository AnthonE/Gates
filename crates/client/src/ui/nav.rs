//! Which page a key or a tab opens — Rust's inventory and crafting split.
//!
//! Rust keeps the two apart: **Tab** is the inventory (your pack, your body,
//! whatever you are looting), **Q** is the crafting menu, and each key
//! toggles its own page — `inventory.toggle` and `inventory.togglecrafting`
//! in its console. Ours was one screen with the recipe browser stacked over
//! the grids, which is why it had to be squeezed into 720 px and why a
//! container pushed crafting off it entirely.
//!
//! This is the whole rule, kept out of the Bevy system so it can be tested:
//! a page's own key closes it, any other page's key switches to that page,
//! and a button in the strip is a switch that never closes. The strip never
//! offers the page you are on — Rust's has no lit tab; its inventory button
//! "moves around to replace the opened menu button".

/// The screens a page key can land on. `Other` is everything that is not
/// one of the two pages — the build wheel, the tech tree — which a page key
/// replaces rather than stacks on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Closed,
    Inventory,
    Crafting,
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

/// The buttons across the top while `open` is up: the other page, never
/// this one.
pub fn strip(open: Page) -> &'static [Page] {
    match open {
        Page::Inventory => &[Page::Crafting],
        Page::Crafting => &[Page::Inventory],
        Page::Closed | Page::Other => &[],
    }
}

/// A page's name on its button.
pub fn label(page: Page) -> &'static str {
    match page {
        Page::Inventory => "INVENTORY",
        Page::Crafting => "CRAFTING",
        Page::Closed | Page::Other => "",
    }
}

/// The key that opens a page, Rust's own binding — in the button's
/// tooltip, where Rust puts it.
pub fn key_hint(page: Page) -> &'static str {
    match page {
        Page::Inventory => "Tab",
        Page::Crafting => "Q",
        Page::Closed | Page::Other => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pages_own_key_closes_it_and_the_other_key_switches() {
        assert_eq!(press(Page::Closed, Page::Inventory), Page::Inventory);
        assert_eq!(press(Page::Closed, Page::Crafting), Page::Crafting);
        assert_eq!(press(Page::Inventory, Page::Inventory), Page::Closed);
        assert_eq!(press(Page::Crafting, Page::Crafting), Page::Closed);
        assert_eq!(press(Page::Inventory, Page::Crafting), Page::Crafting);
        assert_eq!(press(Page::Crafting, Page::Inventory), Page::Inventory);
        // The tech tree is not a page: a page key replaces it.
        assert_eq!(press(Page::Other, Page::Inventory), Page::Inventory);
    }

    #[test]
    fn a_tab_click_never_closes() {
        assert_eq!(click(Page::Inventory), Page::Inventory);
        assert_eq!(click(Page::Crafting), Page::Crafting);
    }

    #[test]
    fn the_strip_offers_the_other_page_and_never_this_one() {
        for open in [Page::Inventory, Page::Crafting] {
            let offered = strip(open);
            assert!(!offered.is_empty(), "{open:?} has a way to the other page");
            assert!(!offered.contains(&open), "{open:?} offers itself");
            for page in offered {
                assert!(!label(*page).is_empty() && !key_hint(*page).is_empty());
            }
        }
        assert!(strip(Page::Closed).is_empty() && strip(Page::Other).is_empty());
    }
}
