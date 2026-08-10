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

/// Every stem `assets/icons/` ships.
///
/// A const list rather than a directory walk, because the render path must
/// not do I/O to find out what it has — and because a gate can then compare
/// it against the directory and fail on either half drifting
/// (`tests/ui.rs` §G).
pub const STEMS: [&str; 64] = [
    // the shape wheel
    "shape_foundation",
    "shape_wall",
    "shape_doorway",
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
    use super::stem;

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
}
