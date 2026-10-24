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
use sim_core::turret::{is_turret_slot, TurretDef};
use sim_core::world::{Command, World, EV_HURT, EV_SENTRY_LOCK, EV_SHOT};

const SEED: u64 = 0x0FEE_0FEE;
const OWNER: u32 = 3;
const STRANGER: u32 = 4;
const TURRET_ITEM: u16 = 40;
const ROUND: u16 = 41;

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

fn buildable_cell(w: &World) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (512 + dx) as u16;
                let cz = (512 + dz) as u16;
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
/// from it; `rounds` pistol rounds in its box. Returns the world, the
/// turret's box key and the two players' slots.
fn turret_world(rounds: u16) -> (World, u32) {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.gather.stack_max[TURRET_ITEM as usize] = 1;
    w.gather.stack_max[ROUND as usize] = 128;
    w.gather.item_count = w.gather.item_count.max(ROUND + 1);
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
    w.turret_def = TurretDef {
        gun: SentryDef {
            range_mm: 30_000,
            damage: 20,
            burst: 8,
            rate_ticks: 12,
            gap_ticks: 100,
            lock_ticks: 30,
            lose_ticks: 90,
            spread_pm: 0,
        },
        ammo: ROUND,
    };

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
            0,
            ItemStack {
                item: ROUND,
                count: rounds,
                cond: 0,
                skin: 0,
                ..ItemStack::EMPTY
            },
        );
    }
    (w, key)
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
        from_slot: 0,
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
    assert_eq!(mv(&mut w, STRANGER, CONT_BOX, 0, CONT_SELF, 6, 64), refused);
    assert_eq!(mv(&mut w, STRANGER, CONT_SELF, 5, CONT_BOX, 1, 10), refused);
    assert_eq!(rounds_left(&w, key), 64, "a stranger moved its rounds");
    assert_eq!(w.players[s].inv[5].count, 10);
    // Its owner still loads and unloads it.
    let r = mv(&mut w, OWNER, CONT_BOX, 0, CONT_SELF, 6, 64);
    assert_eq!(r.map(|r| r.0), Some(EV_MOVED), "its owner was refused");
    assert_eq!(rounds_left(&w, key), 0);
}
