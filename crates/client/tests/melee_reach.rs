//! A melee row reaches as far as its weapon is drawn reaching.
//!
//! `content/weapons.toml`'s `reach_cm` is eye to the struck body's skin. The
//! drawn weapon reaches the rig's hand at the strike plus the item's length
//! ahead of the grip (`ui::hold::HELD_MODELS`). The two lived in different
//! crates with nothing between them, and every short weapon reached a flat
//! metre whether it was a 30 cm torch head or a 74 cm bat barrel.

use client::ui::hold::HELD_MODELS;
use client::ui::icons::stem;
use content::schema::WeaponKind;

/// Where the rig's hand is at the strike, metres out from the body's axis:
/// `assets/models/stumpy.glb`'s `RightHand` under `Sword_Attack` at 0.30 s,
/// its forward-most frame, 0.57 m at the rig's 1.8 m and 0.54 m at the drawn
/// 1.7 m, plus the palm's few centimetres to the grip.
const HAND_AT_STRIKE_M: f32 = 0.6;

/// How far a row may sit from its drawn weapon: the 5 cm the rows are
/// rounded to.
const SLACK_M: f32 = 0.05;

#[test]
fn every_melee_row_reaches_as_far_as_its_weapon_is_drawn() {
    let c = content::Content::load_dir(std::path::Path::new("../../content"))
        .unwrap_or_else(|e| panic!("content/ does not load: {e}"));
    let mut checked = 0;
    for w in c.weapons.iter().filter(|w| w.kind == WeaponKind::Melee) {
        let item = c
            .items
            .iter()
            .find(|i| i.id == w.id)
            .unwrap_or_else(|| panic!("`{}` arms no item", w.id));
        let key = stem(&item.name);
        let Some(m) = HELD_MODELS.iter().find(|m| m.key == key) else {
            panic!(
                "`{}` ({key}) has no held model to measure its reach by",
                w.id
            );
        };
        let ahead = m.height_m * m.scale * (1.0 - m.grip_frac);
        let drawn = HAND_AT_STRIKE_M + ahead;
        let reach = w.reach_cm as f32 / 100.0;
        assert!(
            (reach - drawn).abs() <= SLACK_M,
            "`{}` reaches {reach:.2} m but is drawn reaching {drawn:.2} m (the hand at \
             {HAND_AT_STRIKE_M} m plus {ahead:.2} m of {key} ahead of the grip)",
            w.id
        );
        checked += 1;
    }
    assert!(checked >= 6, "only {checked} melee rows were measured");
}
