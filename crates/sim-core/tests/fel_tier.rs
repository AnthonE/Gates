//! Gate: the fel tiers (`mob::FelTier`, `works::fel_tier_of`) — which tier
//! the island is in, and what a tier does to a species' row.

use sim_core::mob::{FelTier, MobContent, MOB_PIG, MOB_STAG, MOB_WOLF};
use sim_core::works::{fel_tier_of, WORK_LIT, WORK_OPEN, WORK_SEALED};

#[test]
fn the_tier_follows_the_works() {
    let t = |w: &[(u8, u8)]| fel_tier_of(w.iter().copied());
    assert_eq!(t(&[]), 0, "no works is act I");
    assert_eq!(t(&[(2, WORK_SEALED), (2, WORK_SEALED)]), 0);
    assert_eq!(t(&[(2, WORK_OPEN), (2, WORK_SEALED)]), 1, "act II opens");
    assert_eq!(t(&[(2, WORK_LIT), (2, WORK_OPEN)]), 1);
    assert_eq!(
        t(&[(2, WORK_LIT), (2, WORK_LIT)]),
        2,
        "every act-II work burns"
    );
    assert_eq!(t(&[(2, WORK_OPEN), (3, WORK_OPEN)]), 2, "act III");
}

fn tiered() -> MobContent {
    let mut mc = MobContent::probe_fixture();
    mc.tiers[0] = FelTier {
        hp_pct: 60,
        attack_pct: 50,
        notice_pct: 50,
        brave_min: 35,
    };
    mc.tiers[2] = FelTier {
        hp_pct: 150,
        attack_pct: 140,
        notice_pct: 125,
        brave_min: 0,
    };
    mc
}

#[test]
fn a_tier_moves_every_row_by_its_percentages() {
    let mc = tiered();
    let base = mc.def(MOB_WOLF);
    let a = mc.at_tier(0).def(MOB_WOLF);
    assert_eq!(a.hp, base.hp * 60 / 100);
    assert_eq!(a.attack, base.attack / 2);
    assert_eq!(a.spook_cm, (base.spook_cm / 2).max(base.attack_range_cm));
    assert_eq!(a.hear.gun_cm, base.hear.gun_cm / 2);
    assert_eq!(a.brave_pct, 35, "the predator learns to break off");
    // What a tier does not touch stays the species'.
    assert_eq!(a.body_r_cm, base.body_r_cm);
    assert_eq!(a.attack_range_cm, base.attack_range_cm);
    assert_eq!(a.pack_cm, base.pack_cm);

    let c = mc.at_tier(2).def(MOB_WOLF);
    assert_eq!(c.hp, base.hp * 3 / 2);
    assert_eq!(c.attack, base.attack * 140 / 100);
    assert!(c.spook_cm > base.spook_cm);
    assert_eq!(c.brave_pct, base.brave_pct);

    assert_eq!(mc.at_tier(1), mc, "the middle tier is the rows as written");
    assert_eq!(
        mc.at_tier(9),
        mc.at_tier(2),
        "past the last tier is the last tier"
    );
}

#[test]
fn a_courage_floor_never_lowers_one_and_a_pacifist_stays_one() {
    let mc = tiered();
    // The boar's own 50 is over the floor of 35.
    assert_eq!(
        mc.at_tier(0).def(MOB_PIG).brave_pct,
        mc.def(MOB_PIG).brave_pct.max(35)
    );
    // A species that never bites is not given courage it has no use for.
    let stag = mc.at_tier(0).def(MOB_STAG);
    assert_eq!(stag.attack, 0);
    assert_eq!(stag.brave_pct, mc.def(MOB_STAG).brave_pct);
}

#[test]
fn no_tier_shrinks_a_hunter_under_its_own_bite() {
    let mut mc = MobContent::probe_fixture();
    mc.tiers[0].notice_pct = 10;
    for kind in [MOB_PIG, MOB_WOLF] {
        let d = mc.at_tier(0).def(kind);
        if d.attack > 0 {
            assert!(
                d.spook_cm >= d.attack_range_cm,
                "kind {kind} notices inside its bite"
            );
            assert!(d.night_spook_cm >= d.attack_range_cm);
        }
    }
}
