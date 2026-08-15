//! Item, building-shape and hammer-verb icons.
//!
//! **Why this exists.** Every cell in the reference `crafting.png` is a picture
//! of the thing; every cell of ours was a clipped word (`Gunpowde`,
//! `Workbenc`). That is the difference between a screen you scan and a screen
//! you read, and it is why the panels looked like a spreadsheet no matter
//! what the palette or the typeface did.
//!
//! **Where they come from.** `game-icons.net`, CC BY 3.0, rasterised into
//! `assets/icons/` by `ci/` tooling from the project's own SVG archive.
//! Nothing is traced from the reference game — that is the IP rail, and an
//! icon set anyone may redistribute with credit is not the rail's business.
//! `assets/icons/CREDITS.md` carries the attribution the licence requires and
//! `crates/client/tests/ui.rs` §G fails if it stops shipping.
//!
//! **White on transparent, tinted at the draw.** Every PNG is a white
//! silhouette, so one file serves every state: `ImageNode::color` multiplies
//! it, and a chosen wedge, a dimmed unaffordable recipe and an ordinary cell
//! are three tints of one texture rather than three files.
//!
//! ## The key is the item's NAME, and that is forced
//!
//! `protocol::ItemCatalog` carries display names and nothing else — there is
//! no content id on the wire. So a cell finds its icon by normalising the
//! name the server sent (`"Low Grade Fuel"` → `low_grade_fuel`) and the baker
//! writes its files under the same normalisation, derived from
//! `content/items.toml` rather than typed twice. 21 of the 48 items have a
//! name that differs from their id, which is why this is worth a paragraph.
//!
//! A miss is not a defect and must not draw an empty cell: [`Icons::item`]
//! returns `None` and the caller falls back to the text it drew before. A
//! shard whose content has an item we baked no icon for is a normal thing.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

pub use crate::ui::icons::{stem, STEMS};

/// Every icon, by file stem, loaded once.
///
/// Loaded through the `AssetServer` rather than embedded like the fonts, and
/// the difference is what a miss costs: an unresolved *font* draws no text at
/// all and the client cannot report its own state, while an unresolved icon
/// draws a cell that still has a border, a name in the detail pane, and a
/// fallback label. Icons are also content-shaped — a shard may add items —
/// and `assets/` is what the depot already ships and `--features hot`
/// already watches.
#[derive(Resource, Default)]
pub struct Icons {
    by_name: HashMap<String, Handle<Image>>,
}

impl Icons {
    /// Every handle this set asked the asset server for.
    ///
    /// Exists for the boot splash, which is the one caller that cares about
    /// the *set* rather than about any icon in it: it lifts when every handle
    /// has settled (`render/boot.rs`). Order-independent by construction — it
    /// is folded with `all`, never indexed — so iterating the map is safe
    /// here in a way wall 1 forbids inside the sim.
    pub fn handles(&self) -> impl Iterator<Item = &Handle<Image>> {
        self.by_name.values()
    }

    /// The icon for an item's display name, if one was baked.
    pub fn item(&self, name: &str) -> Option<Handle<Image>> {
        self.by_name.get(&stem(name)).cloned()
    }

    /// A building shape's icon — `shape_wall`, `shape_foundation`, …
    pub fn shape(&self, key: &str) -> Option<Handle<Image>> {
        self.by_name.get(key).cloned()
    }

    /// A hammer verb's icon — `verb_upgrade`, `verb_demolish`, …
    ///
    /// The same lookup [`Self::shape`] does and deliberately not the same
    /// function: these are keyed by a literal stem rather than by a name off
    /// the wire, and which map a call site is reading is worth being able to
    /// grep for.
    pub fn verb(&self, key: &str) -> Option<Handle<Image>> {
        self.by_name.get(key).cloned()
    }

    /// A UI glyph borrowed from the item set by literal stem — the tech
    /// tree's padlock is `code_lock` worn as an overlay (tech tree v0).
    /// [`Self::shape`]'s lookup, its own name for its own grep.
    pub fn glyph(&self, key: &str) -> Option<Handle<Image>> {
        self.by_name.get(key).cloned()
    }
}

/// Kick every icon off at `Startup`, beside the terrain textures and for the
/// same reason: they are wanted whichever screen comes first, and warming
/// them while a player reads the menu is free time.
pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    let mut by_name = HashMap::default();
    for s in STEMS {
        by_name.insert(s.to_string(), server.load(format!("icons/{s}.png")));
    }
    commands.insert_resource(Icons { by_name });
}
