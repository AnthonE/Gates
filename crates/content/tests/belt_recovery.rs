//! The shipped medkit's recovery flag survives parsing, baking and hashing,
//! and reaches the same World::tick that resolves an actual failed roll.

use content::Content;
use sim_core::{
    gather::ItemStack,
    limits::HOTBAR_SLOTS,
    world::{Command, World, EV_CONSUMED, EV_RECOVERED},
    wound::{recovers, RECOVER_BASE_PM, WOUNDED_HP},
};
use std::path::Path;

fn sources() -> Vec<(&'static str, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    content::FILES
        .iter()
        .map(|&name| (name, std::fs::read_to_string(dir.join(name)).unwrap()))
        .collect()
}

fn build(src: &[(&str, String)]) -> Result<Content, String> {
    Content::from_sources(
        &src.iter()
            .map(|(name, text)| (*name, text.as_str()))
            .collect::<Vec<_>>(),
    )
}

#[test]
fn only_the_medkit_opts_in_and_omission_is_false() {
    let c = build(&sources()).unwrap();
    let table = c.bake_survival().unwrap();
    let medkit = c.item_index("item.medkit").unwrap() as usize;
    for (i, &flag) in table.belt_recovery.iter().enumerate() {
        assert_eq!(
            flag,
            i == medkit,
            "only the shipped medkit rescues: item {i}"
        );
    }
    // An absent field is exactly false, including the WAL's content hash.
    let mut absent = sources();
    let row = &mut absent
        .iter_mut()
        .find(|(name, _)| *name == "consumables.toml")
        .unwrap()
        .1;
    assert_eq!(row.matches("belt_recovery = true").count(), 1);
    *row = row.replace("belt_recovery = true", "");
    let unarmed = build(&absent).unwrap();
    assert!(unarmed
        .bake_survival()
        .unwrap()
        .belt_recovery
        .iter()
        .all(|&v| !v));
    let row = &mut absent
        .iter_mut()
        .find(|(name, _)| *name == "consumables.toml")
        .unwrap()
        .1;
    *row = row.replace(
        "id = \"item.medkit\"",
        "id = \"item.medkit\"\nbelt_recovery = false",
    );
    assert_eq!(unarmed.hash(), build(&absent).unwrap().hash());
    assert_ne!(unarmed.hash(), c.hash(), "replays must pin eligibility");
}

#[test]
fn the_flag_is_keyed_by_item_and_requires_a_healing_row() {
    let mut c = build(&sources()).unwrap();
    for con in &mut c.consumables {
        con.belt_recovery = con.id == "item.bandage";
    }
    let table = c.bake_survival().unwrap();
    let bandage = c.item_index("item.bandage").unwrap() as usize;
    for (i, &flag) in table.belt_recovery.iter().enumerate() {
        assert_eq!(
            flag,
            i == bandage,
            "the sim does not hardcode a medkit index"
        );
    }
    let mut invalid = sources();
    let row = &mut invalid
        .iter_mut()
        .find(|(name, _)| *name == "consumables.toml")
        .unwrap()
        .1;
    *row = row.replace("health = 60", "health = 0");
    assert!(build(&invalid)
        .unwrap_err()
        .contains("belt recovery needs health"));
    c.consumables
        .iter_mut()
        .find(|c| c.id == "item.bandage")
        .unwrap()
        .health = 0;
    assert!(c
        .bake_survival()
        .unwrap_err()
        .contains("belt recovery needs health"));
}

#[test]
fn the_shipped_medkit_rescues_from_the_belt_but_not_the_backpack() {
    let c = build(&sources()).unwrap();
    let medkit = c.item_index("item.medkit").unwrap();
    for inv in [HOTBAR_SLOTS - 1, HOTBAR_SLOTS] {
        let mut w = World::new(42);
        w.survival = c.bake_survival().unwrap();
        w.gather = c.bake_gather().unwrap();
        w.combat = c.bake_combat().unwrap();
        w.tick(&[Command::Join { id: 1 }]);
        let t = (w.tick + 2..w.tick + 100)
            .find(|&t| !recovers(42, 1, t, RECOVER_BASE_PM))
            .unwrap();
        let p = &mut w.players[0];
        p.hp = WOUNDED_HP;
        p.food = 0;
        p.water = 0;
        p.wounded = true;
        p.wound_until = t;
        p.inv[inv] = ItemStack {
            item: medkit,
            count: 1,
            cond: 0,
        };
        while w.tick <= t {
            w.tick(&[]);
        }
        let rescued = inv < HOTBAR_SLOTS;
        assert_eq!(!w.players[0].dead && !w.players[0].wounded, rescued);
        assert_eq!(
            w.events.entries().iter().any(|e| e.code == EV_RECOVERED),
            rescued
        );
        assert_eq!(
            w.events.entries().iter().any(|e| e.code == EV_CONSUMED),
            rescued
        );
        if rescued {
            assert_eq!(w.players[0].inv[inv], ItemStack::default());
            assert_eq!(w.players[0].hp, WOUNDED_HP);
            assert_eq!(w.players[0].heal_rem, 0);
        }
    }
}
