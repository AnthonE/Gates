//! Give a sleeping bag to another player (`Command::GiveBag`, Rust's assign
//! bag): the bag becomes theirs to wake on and stops being the giver's.
//!
//! The team rule is the shard's (`server::team`), so it is not here: the sim
//! checks a standing giver, a seated receiver who is not the giver, the
//! giver's own bag at the address, and the receiver's `BAG_CAP`. What this
//! file owns is the consequence — whose death screen the bag answers, that
//! its cooldown travels with it, and that a refusal moves nothing.

use sim_core::build::{foundation_terrain_ok, BUILD_CELL_M, LOC_PLANE};
use sim_core::combat::CombatContent;
use sim_core::deploy::{
    box_key, BagAnchor, DeployContent, BAG_CAP, REFUSE_D_BAG_CAP, REFUSE_D_KIND, REFUSE_D_OWNER,
};
use sim_core::gather::ItemStack;
use sim_core::limits::TICK_HZ;
use sim_core::survival::SurvivalContent;
use sim_core::world::{Command, World, EV_BAG_GIVEN, EV_DEPLOY_REFUSED, EV_RESPAWN};

const SEED: u64 = 20260803;
const GIVER: u32 = 1;
const TAKER: u32 = 2;
/// Row 3 of `DeployContent::probe_fixture` is the ground-class bag, and it
/// costs one unit of fixture item 5 (`bag_respawn`'s fixture).
const BAG_ROW: u16 = 3;
const BAG_ITEM: u16 = 5;

fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// The `skip`th cell near (682, 682) that takes a ground deployable, by the
/// rule the sim refuses on (`bag_respawn`'s scan).
fn cell(skip: usize) -> (u16, u16) {
    let mut found = 0usize;
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (682 + dx) as u16;
                let cz = (682 + dz) as u16;
                let (x, z) = cell_center(cx, cz);
                if foundation_terrain_ok(SEED, hv(SEED), x, z) {
                    if found == skip {
                        return (cx, cz);
                    }
                    found += 1;
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this test");
}

/// Two players, hp from the combat fixture, the clock armed so an empty
/// meter kills, and the deploy table that knows what a bag is. The meters
/// never drain on their own, so only the body [`die`] empties falls.
fn pair_world() -> World {
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.survival.food_span_ticks = 1 << 30;
    w.survival.water_span_ticks = 1 << 30;
    w.deploy = DeployContent::probe_fixture();
    w.tick(&[Command::Join { id: GIVER }, Command::Join { id: TAKER }]);
    w
}

fn slot(w: &World, id: u32) -> usize {
    w.players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("seated")
}

fn stand(w: &mut World, id: u32, cx: u16, cz: u16) {
    let (x, z) = cell_center(cx, cz);
    let s = slot(w, id);
    w.players[s].body = sim_core::movement::Body::at(w.seed, &w.haven, x, z);
}

/// Place a bag for `id` through the real verb.
fn place_bag(w: &mut World, id: u32, cx: u16, cz: u16) {
    stand(w, id, cx, cz);
    let s = slot(w, id);
    w.players[s].inv[10] = ItemStack {
        item: BAG_ITEM,
        count: 1,
        ..ItemStack::EMPTY
    };
    let before = w.deploys.len();
    w.tick(&[Command::PlaceDeploy {
        id,
        row: BAG_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        pose: sim_core::footprint::Pose::CENTRE,
    }]);
    assert_eq!(
        w.deploys.len(),
        before + 1,
        "the bag did not place at ({cx}, {cz}) — the fixture, not the mechanic"
    );
}

fn give(w: &mut World, id: u32, at: (u16, u16), target: u32) {
    w.tick(&[Command::GiveBag {
        id,
        cx: at.0,
        cz: at.1,
        level: 0,
        target,
    }]);
}

fn bags_of(w: &World, id: u32) -> Vec<BagAnchor> {
    let mut out = [BagAnchor::default(); BAG_CAP];
    let n = w.deploys.own_bags(&w.deploy, id, w.tick, &mut out);
    out[..n].to_vec()
}

fn addrs(w: &World, id: u32) -> Vec<(u16, u16)> {
    bags_of(w, id).iter().map(|b| (b.cx, b.cz)).collect()
}

/// The tick's events of `code`, as (a, b, c).
fn events(w: &World, code: u8) -> Vec<(u32, u32, u32)> {
    w.events
        .entries()
        .iter()
        .filter(|e| e.code == code)
        .map(|e| (e.a, e.b, e.c))
        .collect()
}

/// Starve `id` until the clock takes them; the body waits on the death
/// screen.
fn die(w: &mut World, id: u32) {
    let s = slot(w, id);
    let before = w.players[s].deaths;
    w.players[s].food = 0;
    w.players[s].water = 0;
    for _ in 0..120 * TICK_HZ {
        w.tick(&[]);
        if w.players[s].deaths > before {
            assert!(w.players[s].dead, "the body woke by itself");
            return;
        }
    }
    panic!("the clock never killed the body — the survival fixture changed");
}

/// Die, answer the death screen with the bag at `at`, and say whether a bag
/// answered (`EV_RESPAWN.b`) and where the body stands.
fn die_and_wake_at(w: &mut World, id: u32, at: (u16, u16)) -> (bool, (i32, i32)) {
    die(w, id);
    wake_at(w, id, at)
}

/// Answer the death screen of a body already dead with the bag at `at`.
fn wake_at(w: &mut World, id: u32, at: (u16, u16)) -> (bool, (i32, i32)) {
    w.tick(&[Command::RespawnAt {
        id,
        cx: at.0,
        cz: at.1,
        level: 0,
    }]);
    let s = slot(w, id);
    assert!(!w.players[s].dead, "the answer did not wake the body");
    let woke = events(w, EV_RESPAWN)
        .into_iter()
        .find(|e| e.0 == id)
        .expect("a respawn was announced");
    (woke.1 == 1, (w.players[s].body.qx, w.players[s].body.qz))
}

fn body_on(w: &World, at: (u16, u16)) -> (i32, i32) {
    let (x, z) = cell_center(at.0, at.1);
    let b = sim_core::movement::Body::at(w.seed, &w.haven, x, z);
    (b.qx, b.qz)
}

/// **The whole item.** A bag given is the receiver's: it is on their list,
/// their death screen wakes them on it, and the giver's no longer does.
#[test]
fn a_given_bag_wakes_its_new_owner_and_not_the_giver() {
    let mut w = pair_world();
    let a = cell(0);
    place_bag(&mut w, GIVER, a.0, a.1);
    assert_eq!(addrs(&w, GIVER), vec![a]);

    give(&mut w, GIVER, a, TAKER);
    let d = w.deploys.entries()[0];
    assert_eq!(
        events(&w, EV_BAG_GIVEN),
        vec![(GIVER, TAKER, box_key(d.cx, d.cz, d.level, d.loc))],
        "a landed give is announced once, naming both and the bag"
    );
    assert!(events(&w, EV_DEPLOY_REFUSED).is_empty());
    assert_eq!(
        addrs(&w, TAKER),
        vec![a],
        "the bag is on the receiver's list"
    );
    assert!(addrs(&w, GIVER).is_empty(), "and off the giver's");

    // Both die somewhere else and ask for the bag by address: the giver
    // first, while the bag is still ready, so only ownership can turn the
    // giver away (a cooling bag would refuse anybody).
    let far = cell(40);
    stand(&mut w, GIVER, far.0, far.1);
    die(&mut w, GIVER);
    assert!(
        w.deploys.bag_ready()[0] <= w.tick,
        "the bag must be ready when the giver asks, or the check proves nothing"
    );
    let (on_bag, at) = wake_at(&mut w, GIVER, a);
    assert!(!on_bag, "the giver still woke on a bag they gave away");
    assert_ne!(at, body_on(&w, a));

    stand(&mut w, TAKER, far.0, far.1);
    let (on_bag, at) = die_and_wake_at(&mut w, TAKER, a);
    assert!(on_bag, "the receiver's death screen did not take the bag");
    assert_eq!(at, body_on(&w, a), "the receiver woke on the bag");
}

/// Only your own bag, and only a bag: a give at somebody else's bag or at
/// an address with none is refused and moves nothing, and a give to
/// yourself, to nobody seated, or from a corpse says nothing at all.
#[test]
fn only_your_own_bag_can_be_given() {
    let mut w = pair_world();
    let a = cell(0);
    place_bag(&mut w, GIVER, a.0, a.1);

    // The receiver reaches for the giver's bag.
    give(&mut w, TAKER, a, GIVER);
    assert_eq!(
        events(&w, EV_DEPLOY_REFUSED),
        vec![(TAKER, REFUSE_D_OWNER, 0)]
    );
    assert!(events(&w, EV_BAG_GIVEN).is_empty());
    // Nothing stands at the next cell over.
    give(&mut w, GIVER, (a.0 + 1, a.1), TAKER);
    assert_eq!(
        events(&w, EV_DEPLOY_REFUSED),
        vec![(GIVER, REFUSE_D_KIND, 0)]
    );
    // To yourself, and to an id nobody is seated under.
    give(&mut w, GIVER, a, GIVER);
    assert!(events(&w, EV_DEPLOY_REFUSED).is_empty());
    give(&mut w, GIVER, a, 99);
    assert!(events(&w, EV_DEPLOY_REFUSED).is_empty());
    assert!(events(&w, EV_BAG_GIVEN).is_empty());
    // A body on the death screen gives nothing.
    die(&mut w, GIVER);
    give(&mut w, GIVER, a, TAKER);
    assert!(events(&w, EV_BAG_GIVEN).is_empty());

    assert_eq!(addrs(&w, GIVER), vec![a], "the bag never moved");
    assert!(addrs(&w, TAKER).is_empty());
}

/// A receiver with `BAG_CAP` placed is refused: a bag given is never one
/// more than they could have put down.
#[test]
fn a_receiver_at_the_cap_is_refused() {
    let mut w = pair_world();
    for k in 0..BAG_CAP {
        let c = cell(k);
        place_bag(&mut w, TAKER, c.0, c.1);
    }
    assert_eq!(bags_of(&w, TAKER).len(), BAG_CAP);
    let a = cell(BAG_CAP);
    place_bag(&mut w, GIVER, a.0, a.1);

    give(&mut w, GIVER, a, TAKER);
    assert_eq!(
        events(&w, EV_DEPLOY_REFUSED),
        vec![(GIVER, REFUSE_D_BAG_CAP, 0)]
    );
    assert!(events(&w, EV_BAG_GIVEN).is_empty());
    assert_eq!(addrs(&w, GIVER), vec![a]);
    assert_eq!(bags_of(&w, TAKER).len(), BAG_CAP);
}

/// The cooldown travels with the bag: one the giver just woke on is spent
/// for its new owner too, or handing it over would reset it.
#[test]
fn a_spent_bag_stays_spent_for_its_new_owner() {
    let mut w = pair_world();
    let a = cell(0);
    place_bag(&mut w, GIVER, a.0, a.1);
    let (on_bag, _) = die_and_wake_at(&mut w, GIVER, a);
    assert!(on_bag, "the fixture: the giver spends the bag first");
    let ready = w.deploys.bag_ready()[0];
    assert!(ready > w.tick, "the bag is cooling down");

    give(&mut w, GIVER, a, TAKER);
    assert_eq!(w.deploys.bag_ready()[0], ready, "the cooldown did not move");
    let mine = bags_of(&w, TAKER);
    assert_eq!(mine.len(), 1);
    assert!(!mine[0].ready, "the new owner is told it is spent");

    let (on_bag, _) = die_and_wake_at(&mut w, TAKER, a);
    assert!(!on_bag, "a spent bag woke its new owner");
}

/// The give is sim state: two runs of the same log hash the same, and a run
/// without it does not.
#[test]
fn a_give_replays_bit_for_bit() {
    let run = |gives: bool| {
        let mut w = pair_world();
        let a = cell(0);
        place_bag(&mut w, GIVER, a.0, a.1);
        if gives {
            give(&mut w, GIVER, a, TAKER);
        } else {
            w.tick(&[]);
        }
        for _ in 0..30 {
            w.tick(&[]);
        }
        w.state_hash()
    };
    assert_eq!(run(true), run(true), "two replays of a give disagree");
    assert_ne!(run(true), run(false), "the bag's owner is not in the hash");
}
