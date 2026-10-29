//! A player's auto turret (`turret.rs`, `NOW.md` §0aa 1): it shoots a
//! stranger in reach and in sight, spares its owner, spends a round out of
//! its own box for every shot, and holds fire dry.

use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::combat::CombatContent;
use sim_core::deploy::{box_key, DeployContent, DeployDef, ARCH_TURRET, PLACE_ANY};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::limits::BOX_SLOTS;
use sim_core::mob;
use sim_core::movement::Body;
use sim_core::sentry::SentryDef;
use sim_core::turret::{is_turret_slot, TurretDef, TurretGun};
use sim_core::world::{Command, World, EV_HURT, EV_SENTRY_LOCK, EV_SHOT};

const SEED: u64 = 0x0FEE_0FEE;
const OWNER: u32 = 3;
const STRANGER: u32 = 4;
const TURRET_ITEM: u16 = 40;
const ROUND: u16 = 41;
/// The gun a test turret is given, which fires [`ROUND`]s.
const GUN: u16 = 42;
/// A second gun and its own round, for the swap.
const RIFLE: u16 = 43;
const RIFLE_ROUND: u16 = 44;

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

fn buildable_cell(w: &World) -> (u16, u16) {
    let (hx, hz) = sim_core::find::build_hint(SEED, 512, 512);
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (hx + dx) as u16;
                let cz = (hz + dz) as u16;
                let (x, z) = cell_center(cx, cz);
                // Flat ground on both sides of it, so the stranger 9 m off
                // stands in sight.
                if (-3..=3).all(|k| {
                    let (x, z) = cell_center((cx as i32 + k) as u16, cz);
                    foundation_terrain_ok(SEED, &w.haven, x, z)
                }) && foundation_terrain_ok(SEED, &w.haven, x, z)
                {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no flat run of cells near the middle");
}

/// A world with the owner's turret placed and the stranger standing 9 m
/// from it; a [`GUN`] in its weapon slot and `rounds` pistol rounds in its
/// first ammo slot. Returns the world and the turret's box key.
fn turret_world(rounds: u16) -> (World, u32) {
    turret_world_with(rounds, Some(GUN))
}

/// [`turret_world`] with `gun` (or none) in the weapon slot.
fn turret_world_with(rounds: u16, gun: Option<u16>) -> (World, u32) {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.gather.stack_max[TURRET_ITEM as usize] = 1;
    w.gather.stack_max[ROUND as usize] = 128;
    w.gather.stack_max[GUN as usize] = 1;
    w.gather.stack_max[RIFLE as usize] = 1;
    w.gather.stack_max[RIFLE_ROUND as usize] = 128;
    w.gather.item_count = w.gather.item_count.max(RIFLE_ROUND + 1);
    let row = w.deploy.def_count;
    w.deploy.defs[row as usize] = DeployDef {
        arch: ARCH_TURRET,
        placement: PLACE_ANY,
        hp: 1000,
        matter: sim_core::deploy::MATTER_METAL,
        item: TURRET_ITEM,
        n_costs: 0,
        costs: [(0, 0); 4],
    };
    w.deploy.def_count += 1;
    let def = |damage| SentryDef {
        range_mm: 30_000,
        damage,
        burst: 8,
        rate_ticks: 12,
        gap_ticks: 100,
        lock_ticks: 30,
        lose_ticks: 90,
        spread_pm: 0,
    };
    w.turret_def = TurretDef::INERT;
    w.turret_def.guns[0] = TurretGun {
        item: GUN,
        def: def(20),
        ammo: ROUND,
    };
    w.turret_def.guns[1] = TurretGun {
        item: RIFLE,
        def: def(50),
        ammo: RIFLE_ROUND,
    };
    w.turret_def.n = 2;

    let (cx, cz) = buildable_cell(&w);
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: OWNER }, Command::Join { id: STRANGER }]);
    let o = w.players.iter().position(|p| p.id == OWNER).unwrap();
    let s = w.players.iter().position(|p| p.id == STRANGER).unwrap();
    // The owner stands a cell off the turret; the stranger three.
    let (ox, oz) = cell_center(cx - 1, cz);
    let (sx, sz) = cell_center(cx + 3, cz);
    w.players[o].body = Body::at(SEED, &w.haven, ox, oz);
    w.players[s].body = Body::at(SEED, &w.haven, sx, sz);
    w.players[o].inv[3] = ItemStack {
        item: TURRET_ITEM,
        count: 1,
        cond: 0,
        skin: 0,
        ..ItemStack::EMPTY
    };
    w.players[o].frame.sel = 3;
    w.tick(&[Command::PlaceDeploy {
        id: OWNER,
        row,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        pose: sim_core::footprint::Pose::CENTRE,
    }]);
    let key = box_key(cx, cz, 0, LOC_PLANE);
    let i = w
        .deploys
        .box_index(key)
        .expect("a turret is a container: placing one stands a box up");
    if rounds > 0 {
        w.deploys.set_box_slot(
            i,
            1,
            ItemStack {
                item: ROUND,
                count: rounds,
                cond: 0,
                skin: 0,
                ..ItemStack::EMPTY
            },
        );
    }
    if let Some(item) = gun {
        w.deploys.set_box_slot(
            i,
            0,
            ItemStack {
                item,
                count: 1,
                cond: 0,
                skin: 0,
                ..ItemStack::EMPTY
            },
        );
    }
    (w, key)
}

fn count_of(w: &World, key: u32, item: u16) -> u32 {
    let i = w.deploys.box_index(key).unwrap();
    (0..BOX_SLOTS)
        .map(|s| w.deploys.box_slot(i, s))
        .filter(|s| s.item == item)
        .map(|s| s.count as u32)
        .sum()
}

fn turret_shots(w: &mut World, ticks: usize) -> u32 {
    let mut shots = 0;
    for _ in 0..ticks {
        w.tick(&[]);
        shots += w
            .events
            .entries()
            .iter()
            .filter(|e| e.code == EV_SHOT && is_turret(e.a))
            .count() as u32;
    }
    shots
}

/// As in Rust, a turret with rounds and no gun has nothing to fire them
/// from.
#[test]
fn a_turret_with_no_gun_holds_fire() {
    let (mut w, key) = turret_world_with(64, None);
    assert_eq!(turret_shots(&mut w, 300), 0, "a turret with no gun fired");
    assert_eq!(count_of(&w, key, ROUND), 64);
    assert!(w.turrets.iter().all(|t| t.carried.is_none()));
}

/// **It fires the gun it is given, and that gun's round**: a rifle in the
/// box spends rifle rounds and leaves the pistol rounds beside it alone,
/// and the gun rides the table for the wire to mount.
#[test]
fn a_turret_fires_the_gun_in_its_box_with_that_guns_round() {
    let (mut w, key) = turret_world_with(64, Some(RIFLE));
    let i = w.deploys.box_index(key).unwrap();
    w.deploys.set_box_slot(
        i,
        2,
        ItemStack {
            item: RIFLE_ROUND,
            count: 64,
            cond: 0,
            skin: 0,
            ..ItemStack::EMPTY
        },
    );
    let shots = turret_shots(&mut w, 300);
    assert!(shots > 0, "the rifle turret never fired");
    assert_eq!(count_of(&w, key, RIFLE_ROUND), 64 - shots);
    assert_eq!(
        count_of(&w, key, ROUND),
        64,
        "it spent the wrong gun's round"
    );
    assert!(
        w.turrets.iter().any(|t| t.live && t.carried == Some(RIFLE)),
        "the table does not say which gun it carries"
    );
}

fn rounds_left(w: &World, key: u32) -> u32 {
    let i = w.deploys.box_index(key).unwrap();
    (0..BOX_SLOTS)
        .map(|s| w.deploys.box_slot(i, s))
        .filter(|s| s.item == ROUND)
        .map(|s| s.count as u32)
        .sum()
}

fn is_turret(id: u32) -> bool {
    mob::slot_of_id(id).is_some_and(is_turret_slot)
}

#[test]
fn a_turret_shoots_a_stranger_spares_its_owner_and_spends_its_rounds() {
    let (mut w, key) = turret_world(64);
    let (mut locks, mut shots, mut hurt_stranger, mut hurt_owner) = (0, 0, 0, 0);
    for _ in 0..300 {
        w.tick(&[]);
        for e in w.events.entries() {
            match e.code {
                EV_SENTRY_LOCK if is_turret(e.a) => {
                    assert_eq!(e.b, STRANGER, "it locked on to its owner");
                    locks += 1;
                }
                EV_SHOT if is_turret(e.a) => shots += 1,
                EV_HURT if e.a == STRANGER => hurt_stranger += 1,
                EV_HURT if e.a == OWNER => hurt_owner += 1,
                _ => {}
            }
        }
    }
    assert!(locks > 0, "the turret never locked on to the stranger");
    assert!(shots > 0, "it locked on and never fired");
    assert!(hurt_stranger > 0, "{shots} rounds and none landed");
    assert_eq!(hurt_owner, 0, "it shot its owner");
    assert_eq!(
        rounds_left(&w, key),
        64 - shots,
        "every round fired came out of the box"
    );
}

#[test]
fn a_dry_turret_holds_fire() {
    let (mut w, _) = turret_world(0);
    for _ in 0..300 {
        w.tick(&[]);
        assert!(
            !w.events
                .entries()
                .iter()
                .any(|e| (e.code == EV_SHOT || e.code == EV_SENTRY_LOCK) && is_turret(e.a)),
            "an empty turret fired"
        );
    }
}

/// **A turret still knows its owner after they log back in.** The table
/// caches its deployable's owner, and a reconnect hands the owner a new id
/// (`World::take_over`): a cache left behind turns the owner's own turret on
/// them the moment they walk back into their base.
#[test]
fn a_turret_spares_its_owner_after_they_reconnect() {
    let (mut w, _) = turret_world(64);
    const BACK: u32 = 0x0203;
    w.tick(&[Command::Leave { id: OWNER }]);
    w.tick(&[Command::Wake {
        id: BACK,
        sleeper: OWNER,
    }]);
    let (mut locks, mut hurt_owner) = (0, 0);
    for _ in 0..300 {
        w.tick(&[]);
        for e in w.events.entries() {
            match e.code {
                EV_SENTRY_LOCK if is_turret(e.a) => {
                    assert_eq!(e.b, STRANGER, "it locked on to its owner's new id");
                    locks += 1;
                }
                EV_HURT if e.a == BACK => hurt_owner += 1,
                _ => {}
            }
        }
    }
    assert!(locks > 0, "the turret never locked on to the stranger");
    assert_eq!(hurt_owner, 0, "it shot its owner after a reconnect");
}

/// **Nobody the gun would shoot may unload it by hand.** A stranger in reach
/// moving the turret's rounds out is refused like a locked box and takes
/// nothing; its owner's same move empties it.
#[test]
fn a_stranger_cannot_unload_a_turret_and_its_owner_can() {
    use sim_core::deploy::REFUSE_D_OWNER;
    use sim_core::inventory::{CONT_BOX, CONT_SELF};
    use sim_core::world::EV_DEPLOY_REFUSED;
    let (mut w, key) = turret_world(64);
    let i = w.deploys.box_index(key).unwrap();
    let (bx, bz) = (w.deploys.boxes()[i].cx, w.deploys.boxes()[i].cz);
    let s = w.players.iter().position(|p| p.id == STRANGER).unwrap();
    let (sx, sz) = cell_center(bx + 1, bz);
    w.players[s].body = Body::at(SEED, &w.haven, sx, sz);
    let take = |id| Command::Move {
        id,
        cont: key,
        from_kind: CONT_BOX,
        from_slot: 1,
        to_kind: CONT_SELF,
        to_slot: 10,
        count: 64,
    };
    w.tick(&[take(STRANGER)]);
    assert!(
        w.events
            .entries()
            .iter()
            .any(|e| e.code == EV_DEPLOY_REFUSED && e.a == STRANGER && e.b == REFUSE_D_OWNER),
        "a stranger's unload was not refused as the owner's"
    );
    assert!(
        !w.players[s].inv.iter().any(|st| st.item == ROUND),
        "the stranger walked off with rounds"
    );
    assert_eq!(rounds_left(&w, key), 64);
    w.tick(&[take(OWNER)]);
    assert_eq!(rounds_left(&w, key), 0, "the owner unloads their own gun");
}

/// **Only the hands it spares open its box** (exploit sweep 2026-10). A
/// turret carries no lock, and its box was anybody's: a stranger in reach
/// could empty its rounds, or stuff it, with the move verb. The reference
/// loots a turret only for its authorised; ours are the owner and the
/// claim's crew (`turret::spares`), the vending machine's rule.
#[test]
fn only_the_hands_a_turret_spares_move_its_rounds() {
    use sim_core::deploy::REFUSE_D_OWNER;
    use sim_core::inventory::{CONT_BOX, CONT_SELF};
    use sim_core::world::{EV_DEPLOY_REFUSED, EV_MOVED};
    let (mut w, key) = turret_world(64);
    // Both within arm's reach of it, either side.
    let i = w.deploys.box_index(key).unwrap();
    let (x, z) = w.deploys.boxes()[i].xz();
    let o = w.players.iter().position(|p| p.id == OWNER).unwrap();
    let s = w.players.iter().position(|p| p.id == STRANGER).unwrap();
    w.players[o].body = Body::at(SEED, &w.haven, x - 1.5, z);
    w.players[s].body = Body::at(SEED, &w.haven, x + 1.5, z);
    w.players[s].inv[5] = ItemStack {
        item: ROUND,
        count: 10,
        cond: 0,
        skin: 0,
        ..ItemStack::EMPTY
    };
    let mv = |w: &mut World, id, from_kind, from_slot, to_kind, to_slot, count| {
        w.tick(&[Command::Move {
            id,
            cont: key,
            from_kind,
            from_slot,
            to_kind,
            to_slot,
            count,
        }]);
        w.events
            .entries()
            .iter()
            .filter(|e| (e.code == EV_MOVED || e.code == EV_DEPLOY_REFUSED) && e.a == id)
            .map(|e| (e.code, e.b))
            .next()
    };
    let refused = Some((EV_DEPLOY_REFUSED, REFUSE_D_OWNER));
    assert_eq!(mv(&mut w, STRANGER, CONT_BOX, 1, CONT_SELF, 6, 64), refused);
    assert_eq!(mv(&mut w, STRANGER, CONT_SELF, 5, CONT_BOX, 1, 10), refused);
    assert_eq!(rounds_left(&w, key), 64, "a stranger moved its rounds");
    assert_eq!(w.players[s].inv[5].count, 10);
    // Its owner still loads and unloads it.
    let r = mv(&mut w, OWNER, CONT_BOX, 1, CONT_SELF, 6, 64);
    assert_eq!(r.map(|r| r.0), Some(EV_MOVED), "its owner was refused");
    assert_eq!(rounds_left(&w, key), 0);
}

fn access(id: u32, key: u32, op: u8) -> Command {
    let (cx, cz, level, loc) = sim_core::deploy::box_addr(key);
    Command::Access {
        id,
        cx,
        cz,
        level,
        loc,
        op,
        code: 0,
    }
}

/// Who the turret hurt in `ticks`: (owner hits, stranger hits).
fn hurt_in(w: &mut World, ticks: usize) -> (u32, u32) {
    let (mut owner, mut stranger) = (0, 0);
    for _ in 0..ticks {
        w.tick(&[]);
        for e in w.events.entries() {
            match e.code {
                EV_HURT if e.a == OWNER => owner += 1,
                EV_HURT if e.a == STRANGER => stranger += 1,
                _ => {}
            }
        }
    }
    (owner, stranger)
}

/// **The turret spares its own list, Rust's model.** A stranger may not
/// put themselves on it; its owner's clear empties it, owner and all, so it
/// shoots its owner; the owner putting themselves back on is spared again;
/// and the list rides a save.
#[test]
fn a_turret_spares_its_own_list_and_only_its_privileged_may_join_it() {
    use sim_core::deploy::{
        ACCESS_OP_TURRET_AUTH, ACCESS_OP_TURRET_CLEAR, ACCESS_OP_TURRET_DEAUTH, REFUSE_D_OWNER,
    };
    use sim_core::world::EV_DEPLOY_REFUSED;
    let (mut w, key) = turret_world(128);
    // Both in arm's reach of it, the stranger in its line too.
    let i = w.deploys.box_index(key).unwrap();
    let (x, z) = w.deploys.boxes()[i].xz();
    let o = w.players.iter().position(|p| p.id == OWNER).unwrap();
    let s = w.players.iter().position(|p| p.id == STRANGER).unwrap();
    w.players[o].body = Body::at(SEED, &w.haven, x - 1.5, z);
    w.players[s].body = Body::at(SEED, &w.haven, x + 1.5, z);

    w.tick(&[access(STRANGER, key, ACCESS_OP_TURRET_AUTH)]);
    assert!(
        w.events
            .entries()
            .iter()
            .any(|e| e.code == EV_DEPLOY_REFUSED && e.a == STRANGER && e.b == REFUSE_D_OWNER),
        "a stranger put themselves on the turret's list"
    );
    // Taking yourself off a list you are not on is nothing, and says nothing.
    w.tick(&[access(STRANGER, key, ACCESS_OP_TURRET_DEAUTH)]);
    assert!(w.deploys.turret_recs().is_empty(), "a no-op kept a record");
    assert!(!w.deploys.turret_recs().authorized(key, OWNER, STRANGER));
    let (owner_hits, stranger_hits) = hurt_in(&mut w, 300);
    assert_eq!(owner_hits, 0, "it shot the owner on its list");
    assert!(stranger_hits > 0, "it spared a stranger off its list");

    // The clear: nobody is on it, and it shoots its owner too.
    w.tick(&[access(OWNER, key, ACCESS_OP_TURRET_CLEAR)]);
    assert_eq!(w.deploys.turret_recs().get(key, OWNER).auth.len(), 0);
    let mut blob = vec![0; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = sim_core::worldsave::encode(&w, &mut blob).unwrap();
    let mut back = World::new(SEED);
    back.gather = w.gather;
    back.build = w.build;
    back.deploy = w.deploy;
    back.combat = w.combat;
    back.turret_def = w.turret_def;
    sim_core::worldsave::decode_into(&mut back, &blob[..n]).unwrap();
    assert_eq!(
        back.deploys.turret_recs().entries(),
        w.deploys.turret_recs().entries(),
        "the cleared list did not ride the save"
    );
    // Its owner's privilege puts them back on, and the record goes: the
    // list is its owner alone again.
    w.tick(&[access(OWNER, key, ACCESS_OP_TURRET_AUTH)]);
    assert!(w.deploys.turret_recs().is_empty());
    assert!(w.deploys.turret_recs().authorized(key, OWNER, OWNER));

    // Cleared again, it turns on its owner.
    w.tick(&[access(OWNER, key, ACCESS_OP_TURRET_CLEAR)]);
    let (owner_hits, _) = hurt_in(&mut w, 400);
    assert!(owner_hits > 0, "a cleared turret still spared its owner");
}

/// **Peacekeeper shoots only the hostile** (Rust's mode): a stranger minding
/// their own business is spared, the same stranger hostile is shot, and a
/// blow on the turret makes its striker hostile. Only its privilege sets it.
#[test]
fn a_peacekeeper_turret_shoots_only_the_hostile() {
    use sim_core::deploy::{
        ACCESS_OP_TURRET_ATTACK_ALL, ACCESS_OP_TURRET_PEACEKEEPER, REFUSE_D_OWNER,
    };
    use sim_core::world::EV_DEPLOY_REFUSED;
    let (mut w, key) = turret_world(128);
    let i = w.deploys.box_index(key).unwrap();
    let (x, z) = w.deploys.boxes()[i].xz();
    let o = w.players.iter().position(|p| p.id == OWNER).unwrap();
    let s = w.players.iter().position(|p| p.id == STRANGER).unwrap();
    w.players[o].body = Body::at(SEED, &w.haven, x - 1.5, z);
    w.players[s].body = Body::at(SEED, &w.haven, x + 1.5, z);

    w.tick(&[access(STRANGER, key, ACCESS_OP_TURRET_PEACEKEEPER)]);
    assert!(
        w.events
            .entries()
            .iter()
            .any(|e| e.code == EV_DEPLOY_REFUSED && e.a == STRANGER && e.b == REFUSE_D_OWNER),
        "a stranger set the turret's mode"
    );
    w.tick(&[access(OWNER, key, ACCESS_OP_TURRET_PEACEKEEPER)]);
    assert!(w.deploys.turret_recs().get(key, OWNER).peacekeeper);
    w.players[s].hostile = 0;
    let (_, stranger_hits) = hurt_in(&mut w, 300);
    assert_eq!(stranger_hits, 0, "a peacekeeper shot a peaceful stranger");

    w.players[s].hostile = sim_core::combat::HOSTILE_TICKS;
    let (owner_hits, stranger_hits) = hurt_in(&mut w, 300);
    assert!(stranger_hits > 0, "a peacekeeper spared a hostile stranger");
    assert_eq!(owner_hits, 0, "it shot its owner");

    // Back to attack all, and the record goes: owner alone, attacking all.
    w.tick(&[access(OWNER, key, ACCESS_OP_TURRET_ATTACK_ALL)]);
    assert!(w.deploys.turret_recs().is_empty());
}

/// **The turret panel's slots** (Rust's): its first takes a gun it fires
/// and nothing else, the next six that gun's rounds, the rest nothing; a
/// gun anywhere but the weapon slot is not fired.
#[test]
fn a_turrets_slots_take_what_they_are_for() {
    use sim_core::inventory::{CONT_BOX, CONT_SELF, REFUSE_M_TURRET};
    use sim_core::world::{EV_MOVED, EV_MOVE_REFUSED};
    let (mut w, key) = turret_world_with(0, None);
    let i = w.deploys.box_index(key).unwrap();
    let (x, z) = w.deploys.boxes()[i].xz();
    let o = w.players.iter().position(|p| p.id == OWNER).unwrap();
    w.players[o].body = Body::at(SEED, &w.haven, x - 1.5, z);
    let stack = |item, count| ItemStack {
        item,
        count,
        cond: 0,
        skin: 0,
        ..ItemStack::EMPTY
    };
    w.players[o].inv[5] = stack(ROUND, 20);
    w.players[o].inv[6] = stack(GUN, 1);
    let put = |w: &mut World, from_slot: u8, to_slot| {
        let count = w.players[o].inv[from_slot as usize].count;
        w.tick(&[Command::Move {
            id: OWNER,
            cont: key,
            from_kind: CONT_SELF,
            from_slot,
            to_kind: CONT_BOX,
            to_slot,
            count,
        }]);
        w.events
            .entries()
            .iter()
            .filter(|e| (e.code == EV_MOVED || e.code == EV_MOVE_REFUSED) && e.a == OWNER)
            .map(|e| (e.code, e.b))
            .next()
    };
    let refused = Some((EV_MOVE_REFUSED, REFUSE_M_TURRET));
    assert_eq!(put(&mut w, 5, 0), refused, "rounds went in the weapon slot");
    assert_eq!(put(&mut w, 6, 3), refused, "a gun went in an ammo slot");
    assert_eq!(
        put(&mut w, 5, 9),
        refused,
        "rounds went past the ammo slots"
    );
    assert_eq!(put(&mut w, 6, 0).map(|r| r.0), Some(EV_MOVED));
    assert_eq!(put(&mut w, 5, 4).map(|r| r.0), Some(EV_MOVED));
    let shots = turret_shots(&mut w, 300);
    assert!(shots > 0, "a gun in its slot with rounds never fired");
}

/// **A turret that needs power holds fire until it has it** (Rust's 10 rW):
/// loaded and in reach of a stranger it stands dark; a solar panel at noon,
/// wired to it with the wire tool, wakes it.
#[test]
fn a_turret_fires_only_with_its_power() {
    use sim_core::deploy::{ARCH_SOLAR, PLACE_ANY};
    use sim_core::limits::{DAY_PHASE_TICKS, DAY_PORTION, DAY_TICKS};
    const SOLAR_ITEM: u16 = 45;
    const WIRE_ITEM: u16 = 46;
    let (mut w, key) = turret_world(128);
    w.gather.stack_max[SOLAR_ITEM as usize] = 1;
    w.gather.stack_max[WIRE_ITEM as usize] = 1;
    w.gather.item_count = w.gather.item_count.max(WIRE_ITEM + 1);
    let solar_row = w.deploy.def_count;
    w.deploy.defs[solar_row as usize] = sim_core::deploy::DeployDef {
        arch: ARCH_SOLAR,
        placement: PLACE_ANY,
        hp: 250,
        matter: sim_core::deploy::MATTER_METAL,
        item: SOLAR_ITEM,
        n_costs: 0,
        costs: [(0, 0); 4],
    };
    w.deploy.def_count += 1;
    let (tcx, tcz, tlevel, tloc) = sim_core::deploy::box_addr(key);
    let turret_row = w
        .deploys
        .find(tcx, tcz, tlevel, tloc)
        .expect("the turret's record")
        .row;
    w.power_def.parts[turret_row as usize].draw_rw = 10;
    w.power_def.parts[solar_row as usize].out_rw = 20;
    w.power_def.wire_tool = WIRE_ITEM;
    w.power_def.wire_max_mm = 30_000;
    // Noon, so the panel gives its whole rating.
    let noon = ((DAY_TICKS as f64 * DAY_PORTION as f64) as u64) / 2 + DAY_TICKS - DAY_PHASE_TICKS;
    w.tick = noon - noon % 15;

    assert_eq!(turret_shots(&mut w, 300), 0, "an unpowered turret fired");

    let i = w.deploys.box_index(key).unwrap();
    let (bx, bz) = (w.deploys.boxes()[i].cx, w.deploys.boxes()[i].cz);
    let o = w.players.iter().position(|p| p.id == OWNER).unwrap();
    let stack = |item| ItemStack {
        item,
        count: 1,
        cond: 0,
        skin: 0,
        ..ItemStack::EMPTY
    };
    w.players[o].inv[3] = stack(SOLAR_ITEM);
    w.players[o].frame.sel = 3;
    w.tick(&[Command::PlaceDeploy {
        id: OWNER,
        row: solar_row,
        cx: bx - 2,
        cz: bz,
        level: 0,
        loc: LOC_PLANE,
        pose: sim_core::footprint::Pose::CENTRE,
    }]);
    let panel = box_key(bx - 2, bz, 0, LOC_PLANE);
    assert!(
        w.deploys.find(bx - 2, bz, 0, LOC_PLANE).is_some(),
        "the panel did not place"
    );
    w.players[o].inv[3] = stack(WIRE_ITEM);
    w.tick(&[Command::Wire {
        id: OWNER,
        wire: sim_core::power::WireRec {
            from: panel,
            out: 0,
            to: key,
            inp: 0,
        },
    }]);
    assert_eq!(w.deploys.power().wires().len(), 1, "the wire was not run");
    assert!(
        turret_shots(&mut w, 300) > 0,
        "a powered turret never fired"
    );
}
