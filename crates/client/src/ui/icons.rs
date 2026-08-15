//! Which picture belongs to which item — the pure half.
//!
//! Lives here rather than in `render::icons` for this crate's standing rule:
//! **everything with arithmetic in it is in `crate::ui`**, headless and gated
//! by `crates/client/tests/ui.rs`, and everything under `render` is nodes and
//! handles. The name→stem normalisation is arithmetic in the sense that
//! matters — it can be silently wrong, and being wrong draws an empty cell —
//! so it is testable without a window, and `tests/ui.rs` §G drives it against
//! `content/items.toml` and against what `assets/icons/` actually ships.

/// Normalise a display name to a file stem, the same rule the baker uses.
///
/// Lowercase, and every run of non-alphanumerics becomes one underscore.
/// `"Low Grade Fuel"` → `low_grade_fuel`, `"Metal Fragments"` →
/// `metal_fragments`.
pub fn stem(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if pending && !out.is_empty() {
                out.push('_');
            }
            pending = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending = true;
        }
    }
    out
}

/// The icon a stack draws in an inventory cell, or `None` for an empty slot
/// and for an item whose picture we do not ship.
///
/// **`None` for a missing icon is deliberate and is not a fallback.** A cell
/// showing the wrong picture is worse than a cell showing none: the count is
/// still drawn, so the player can see they are carrying *something*, and an
/// unfamiliar blank reads as "no art yet" where a wrong silhouette reads as
/// the wrong item. `tests/ui.rs` §G already fails if `STEMS` and the shipped
/// directory drift, so a `Some` here is a file that exists.
///
/// Keyed by normalised display name, the same rule [`stem`] states and the
/// same one `ui::hold` resolves models by — one rename, one breakage.
pub fn icon_stem(
    catalog: &protocol::ItemCatalog,
    stack: sim_core::gather::ItemStack,
) -> Option<&'static str> {
    if stack.count == 0 {
        return None;
    }
    let name = core::str::from_utf8(catalog.name(stack.item as usize)).ok()?;
    let s = stem(name);
    STEMS.iter().copied().find(|k| *k == s)
}

/// Every stem `assets/icons/` ships.
///
/// A const list rather than a directory walk, because the render path must
/// not do I/O to find out what it has — and because a gate can then compare
/// it against the directory and fail on either half drifting
/// (`tests/ui.rs` §G).
pub const STEMS: [&str; 67] = [
    // the shape wheel
    "shape_foundation",
    "shape_wall",
    "shape_doorway",
    "shape_window",
    "shape_wall_frame",
    "shape_floor",
    "shape_stairs",
    "shape_roof",
    // the hammer wheel's verbs (`ui::hammer::verb_icon`)
    "verb_upgrade",
    "verb_repair",
    "verb_demolish",
    "verb_pick_up",
    // items, by normalised display name
    "wood",
    "stone",
    "metal_ore",
    "sulfur_ore",
    "cloth",
    "animal_fat",
    "charcoal",
    "metal_fragments",
    "sulfur",
    "gunpowder",
    "low_grade_fuel",
    "gears",
    "rope",
    "tarp",
    "obol",
    "rock",
    "torch",
    "wooden_spear",
    "stone_hatchet",
    "stone_pickaxe",
    "hunting_bow",
    "wooden_arrow",
    "bandage",
    "sleeping_bag",
    "small_box",
    "fire_pit",
    "workbench",
    "hearth",
    "metal_hatchet",
    "metal_pickaxe",
    "metal_spear",
    "furnace",
    "large_box",
    "code_lock",
    "recycler",
    "research_table",
    "wooden_door",
    "building_plan",
    "hammer",
    "burlap_hood",
    "burlap_tunic",
    "metal_arrow",
    "crossbow",
    "revolver",
    "pistol_round",
    "satchel_charge",
    "metal_door",
    "roadsign_vest",
    "medkit",
    "berries",
    "mushrooms",
    "corn",
    // The food loop's states. `burnt_meat` is the one PNG in
    // `assets/icons/` that is not game-icons.net — the archive has none, so
    // it is ours. The other two were briefly ours as well and are archive
    // icons again; `CREDITS.md` draws the line, and this comment claimed all
    // three until 2026-08-09.
    "raw_meat",
    "cooked_meat",
    "burnt_meat",
];

#[cfg(test)]
mod tests {
    use super::{icon_stem, stem};

    #[test]
    fn the_stem_matches_the_bakers_rule() {
        assert_eq!(stem("Low Grade Fuel"), "low_grade_fuel");
        assert_eq!(stem("Metal Fragments"), "metal_fragments");
        assert_eq!(stem("Wood"), "wood");
        // Runs of punctuation collapse to one separator, and a leading or
        // trailing run produces none — the baker's `strip('_')`.
        assert_eq!(stem("  Animal   Fat  "), "animal_fat");
        assert_eq!(stem("Workbench-1"), "workbench_1");
    }

    fn catalog_with(names: &[&str]) -> protocol::ItemCatalog {
        let mut c = protocol::ItemCatalog::EMPTY;
        for (i, n) in names.iter().enumerate() {
            c.set(i, n.as_bytes()).expect("a short name fits");
        }
        c.count = names.len() as u16;
        c
    }

    #[test]
    fn an_empty_slot_draws_no_icon() {
        let c = catalog_with(&["Wood"]);
        let s = |count| icon_stem(&c, sim_core::gather::ItemStack { item: 0, count });
        assert_eq!(s(0), None, "an empty slot must draw nothing at all");
        assert_eq!(s(1), Some("wood"));
    }

    #[test]
    fn an_item_we_ship_no_picture_for_is_none_and_never_a_stand_in() {
        // The cell then shows its COUNT over an empty square, which reads as
        // "no art yet". A fallback silhouette would read as the wrong item,
        // which is strictly worse than blank.
        let c = catalog_with(&["Nonexistent Widget"]);
        assert_eq!(
            icon_stem(&c, sim_core::gather::ItemStack { item: 0, count: 3 }),
            None
        );
        // An id past the catalog is the same answer and not a panic: the id
        // arrives from the wire and the slot index from a keypress.
        assert_eq!(
            icon_stem(
                &c,
                sim_core::gather::ItemStack {
                    item: 999,
                    count: 1
                }
            ),
            None
        );
    }

    #[test]
    fn the_icon_and_the_model_key_off_one_name() {
        // The property that makes a rename break in one place: both lookups
        // normalise the same string with the same function.
        let c = catalog_with(&["Stone Hatchet"]);
        let st = sim_core::gather::ItemStack { item: 0, count: 1 };
        assert_eq!(icon_stem(&c, st), Some("stone_hatchet"));
        assert_eq!(
            crate::ui::hold::held_model(&c, st),
            Some(
                crate::ui::hold::HELD_MODELS
                    .iter()
                    .position(|m| m.key == "stone_hatchet")
                    .unwrap()
            )
        );
    }
}
