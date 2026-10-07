//! THE GATE's safe zone, on Rust's rules: who it protects and who it does
//! not, no weapon drawn inside, no looting, no sleeping, the sentries that
//! shoot anyone hostile inside, and the respawn point.
use sim_core::backpack::{BackpackContent, Backpacks};
use sim_core::combat::{self, CombatContent, RangedDef, HOSTILE_TICKS, NO_MAG};
use sim_core::gather::{GatherContent, ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::inventory::REFUSE_M_SAFE;
use sim_core::kit;
use sim_core::limits::{INV_SLOTS, MOB_ID_TAG, TICK_HZ};
use sim_core::mob;
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::sentry::{SentryDef, SENTRIES, SENTRY_SLOT0};
use sim_core::town;
use sim_core::world::{
    Command, EventQueue, Player, World, DEATH_BY_MOB, EV_HURT, EV_MOVE_REFUSED, EV_SENTRY_LOCK,
    EV_SHOT, GATE_SPAWN_COOLDOWN_TICKS, SAFE_SLEEP_TICKS,
};

const SEED: u64 = 20_260_731;
const ME: u32 = 1;
const GUN: u16 = 5;
const ROUND: u16 = 6;

fn body(safe: bool, hostile: u16) -> Player {
    Player {
        active: true,
        safe,
        hostile,
        ..Player::default()
    }
}

/// The shipped `[sentry]` at 30 Hz.
fn sentry_def() -> SentryDef {
    SentryDef {
        range_mm: 80_000,
        damage: 35,
        burst: 5,
        rate_ticks: 3,
        gap_ticks: 27,
        lock_ticks: 45,
        lose_ticks: 90,
        spread_pm: 12,
    }
}

/// A world with one body standing in the town at local (`lx`, `lz`), a
/// loaded gun in hotbar slot 0.
fn in_town(lx: f32, lz: f32) -> (Box<World>, usize) {
    let mut w = Box::new(World::new(SEED));
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.ranged[GUN as usize] = RangedDef {
        damage: 20,
        ammo: [ROUND, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 4,
        hitscan: true,
        range_mm: 50_000,
        structure: 0,
        head_pct: 200,
        limb_pct: 50,
        magazine: 0,
        reload_ticks: 0,
        mag_slot: NO_MAG,
        draw_ticks: 0,
    };
    w.combat = c;
    let t = w.haven.town;
    assert!(t.live);
    let (x, z) = kit::to_world(&t.placed(), lx, lz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: ME }]);
    // Every later wake is the world's own answer: a beach is a beach.
    w.dev_spawn = None;
    let slot = w
        .players
        .iter()
        .position(|p| p.active && p.id == ME)
        .unwrap();
    w.players[slot].body = Body::at(SEED, &w.haven, x, z);
    w.players[slot].inv[0] = ItemStack {
        item: GUN,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.players[slot].inv[1] = ItemStack {
        item: ROUND,
        count: 50,
        cond: 0,
        skin: 0,
    };
    w.tick(&[]);
    (w, slot)
}

fn fire(on: bool) -> Command {
    Command::Input {
        id: ME,
        frame: InputFrame {
            seq: 1,
            buttons: if on { BTN_PRIMARY } else { 0 },
            yaw: 0,
            pitch: 128,
            move_x: 0,
            move_z: 0,
            sel: 0,
        },
        favour: 0,
    }
}

/// Tick `n` times, counting events of `code` whose `a` is `a` (any `a`
/// when `None`).
fn count(w: &mut World, n: usize, cmd: &[Command], code: u8, a: Option<u32>) -> usize {
    let mut seen = 0;
    for _ in 0..n {
        w.tick(cmd);
        seen += w
            .events
            .entries()
            .iter()
            .filter(|e| e.code == code && a.is_none_or(|a| e.a == a))
            .count();
    }
    seen
}

#[test]
fn nobody_hurts_from_inside_and_nobody_inside_is_hurt_unless_hostile() {
    // Attacker outside, victim inside and peaceful: shielded.
    let mut a = body(false, 0);
    assert!(combat::shielded(&mut a, &body(true, 0)));
    // Trying counts: the attacker is hostile now, for Rust's five minutes.
    assert_eq!(a.hostile, HOSTILE_TICKS);
    assert_eq!(HOSTILE_TICKS as u32, 300 * TICK_HZ);
    // Attacker inside: shielded whoever the victim is.
    let mut a = body(true, 0);
    assert!(combat::shielded(&mut a, &body(false, 0)));
    assert!(combat::shielded(&mut a, &body(true, HOSTILE_TICKS)));
    // A hostile body that ran inside is fair game from outside.
    let mut a = body(false, 0);
    assert!(!combat::shielded(&mut a, &body(true, 5)));
    // Out in the open, nothing changes.
    let mut a = body(false, 0);
    assert!(!combat::shielded(&mut a, &body(false, 0)));
}

#[test]
fn weapons_holster_and_tools_stay_in_hand() {
    let c = CombatContent::probe_fixture();
    let g = GatherContent::probe_fixture();
    // The fixture's firearm and its throwable are weapons.
    assert!(combat::drawn_weapon(&c, &g, 6));
    assert!(combat::drawn_weapon(&c, &g, 3));
    // Item 0 swings and the gather fixture makes it a tool: it stays out.
    assert!(g.is_tool(0));
    assert!(!combat::drawn_weapon(&c, &g, 0));
    // An empty hand and a round are nothing to holster.
    assert!(!combat::drawn_weapon(&c, &g, NO_ITEM));
    assert!(!combat::drawn_weapon(&c, &g, 7));
}

#[test]
fn no_gun_fires_in_the_zone_and_it_fires_outside() {
    let (mut w, slot) = in_town(0.0, -24.0);
    assert!(w.players[slot].safe, "the market street is in the zone");
    let shots = count(&mut w, 60, &[fire(true)], EV_SHOT, Some(ME));
    assert_eq!(shots, 0, "a gun was drawn in the safe zone");

    // The same hand, out past the zone: it fires.
    let t = w.haven.town;
    let (x, z) = kit::to_world(&t.placed(), 0.0, -(town::SAFE_HALF_M + 20.0));
    w.players[slot].body = Body::at(SEED, &w.haven, x, z);
    let shots = count(&mut w, 60, &[fire(true)], EV_SHOT, Some(ME));
    assert!(shots > 0, "the gun never fired outside the zone either");
}

#[test]
fn the_sentries_shoot_a_hostile_player_in_the_zone() {
    let (mut w, slot) = in_town(0.0, -24.0);
    w.sentry_def = sentry_def();
    w.players[slot].hostile = HOSTILE_TICKS;
    let (mut locks, mut shots, mut hurt) = (0, 0, 0);
    for _ in 0..(10 * TICK_HZ) {
        w.tick(&[]);
        for e in w.events.entries() {
            if e.code == EV_SENTRY_LOCK && e.b == ME {
                assert_eq!(
                    mob::slot_of_id(e.a).map(mob::kind_of),
                    Some(mob::MOB_SENTRY)
                );
                locks += 1;
            }
            if e.code == EV_SHOT
                && mob::slot_of_id(e.a).is_some_and(sim_core::sentry::is_sentry_slot)
            {
                shots += 1;
            }
            if e.code == EV_HURT && e.a == ME {
                hurt += 1;
            }
        }
        if w.players[slot].dead {
            break;
        }
    }
    assert!(
        locks > 0,
        "no sentry locked on to a hostile player in the zone"
    );
    assert!(shots > 0, "the sentries locked on and never fired");
    assert!(hurt > 0, "{shots} sentry rounds and none landed");
    let p = &w.players[slot];
    assert!(
        p.dead,
        "a hostile player stood in the zone for ten seconds and lived"
    );
    assert_eq!(p.death_cause, DEATH_BY_MOB);
    assert!(
        mob::slot_of_id(p.death_by).is_some_and(sim_core::sentry::is_sentry_slot),
        "killed by {:#x}, not a sentry",
        p.death_by
    );
    // Rust's rule: hostility survives the death.
    assert!(p.hostile > 0, "the death forgave hostility");
}

#[test]
fn the_sentries_leave_the_peaceful_alone() {
    let (mut w, slot) = in_town(0.0, -24.0);
    w.sentry_def = sentry_def();
    let locks = count(&mut w, 300, &[], EV_SENTRY_LOCK, None);
    assert_eq!(locks, 0);
    assert_eq!(w.players[slot].hp, 100);
    for k in 0..SENTRIES {
        let m = &w.mobs.m[SENTRY_SLOT0 + k];
        assert!(m.alive, "sentry {k} is not standing");
        assert_eq!(m.kind, mob::MOB_SENTRY);
    }
}

#[test]
fn unarmed_the_posts_stay_empty() {
    let (mut w, _) = in_town(0.0, -24.0);
    for _ in 0..60 {
        w.tick(&[]);
    }
    for k in 0..SENTRIES {
        assert!(!w.mobs.m[SENTRY_SLOT0 + k].alive);
        assert_eq!(w.sentries[k], sim_core::sentry::Sentry::default());
    }
}

#[test]
fn the_tick_marks_the_zone_runs_hostility_down_and_kills_oversleepers() {
    let (mut w, slot) = in_town(0.0, 20.0);
    w.players[slot].hostile = 3;
    w.tick(&[]);
    assert!(w.players[slot].safe, "a body in the market is in the zone");
    assert_eq!(w.players[slot].hostile, 2);
    assert!(
        w.players[slot].gate_spawn,
        "reaching the town unlocks its respawn point"
    );
    // Asleep in the zone past the limit: killed where it lies (Rust's rule).
    let at = w.players[slot].body;
    w.players[slot].sleeping = true;
    w.players[slot].slept_at = 0;
    w.tick = SAFE_SLEEP_TICKS + 30 - (SAFE_SLEEP_TICKS + 30) % 30;
    w.tick(&[]);
    let p = &w.players[slot];
    assert!(p.dead, "a sleeper outlived the zone's twenty minutes");
    assert_eq!(p.death_cause, DEATH_BY_MOB);
    assert_eq!(p.death_by, mob::mob_id(SENTRY_SLOT0));
    assert_eq!((p.body.qx, p.body.qz), (at.qx, at.qz), "the body was moved");
    let (px, pz) = (p.body.qx as f32 * POS_XZ_Q, p.body.qz as f32 * POS_XZ_Q);
    assert!(town::safe(&w.haven.town, px, pz));
}

#[test]
fn the_gate_respawn_point_unlocks_rests_and_refuses_the_hostile() {
    let (mut w, slot) = in_town(0.0, -24.0);
    assert!(w.players[slot].gate_spawn);
    let near_gate = |w: &World, slot: usize| {
        let p = &w.players[slot];
        town::safe(
            &w.haven.town,
            p.body.qx as f32 * POS_XZ_Q,
            p.body.qz as f32 * POS_XZ_Q,
        )
    };

    // Dead and peaceful: wakes in the town, and the point rests.
    w.players[slot].dead = true;
    w.tick(&[Command::RespawnGate { id: ME }]);
    assert!(!w.players[slot].dead);
    assert!(near_gate(&w, slot), "woke outside THE GATE");
    let rest = w.players[slot].gate_spawn_at;
    assert!(rest >= w.tick + GATE_SPAWN_COOLDOWN_TICKS - 2);

    // Again inside the cooldown: a beach.
    w.players[slot].dead = true;
    w.tick(&[Command::RespawnGate { id: ME }]);
    assert!(!w.players[slot].dead);
    assert!(!near_gate(&w, slot), "the point did not rest");
    assert_eq!(
        w.players[slot].gate_spawn_at, rest,
        "a beach spent the point"
    );

    // Off the cooldown but hostile: a beach too.
    w.players[slot].gate_spawn_at = 0;
    w.players[slot].hostile = HOSTILE_TICKS;
    w.players[slot].dead = true;
    w.tick(&[Command::RespawnGate { id: ME }]);
    assert!(
        !near_gate(&w, slot),
        "a hostile player woke under the sentries"
    );
}

#[test]
fn another_players_bag_in_the_zone_cannot_be_looted() {
    let w = World::new(SEED);
    let t = w.haven.town;
    let bc = BackpackContent::probe_fixture();
    let gc = GatherContent::probe_fixture();
    let mut items = [ItemStack::default(); INV_SLOTS];
    items[0] = ItemStack {
        item: 1,
        count: 3,
        cond: 0,
        skin: 0,
    };
    let looter_at = |lz: f32| {
        let (x, z) = kit::to_world(&t.placed(), 0.0, lz);
        let mut p = Player {
            id: ME,
            active: true,
            ..Player::default()
        };
        p.body.qx = (x / POS_XZ_Q) as i32;
        p.body.qz = (z / POS_XZ_Q) as i32;
        p
    };
    let try_loot = |owner: u32, lz: f32| {
        let mut bp = Backpacks::new();
        let mut ev = EventQueue::default();
        let p0 = looter_at(lz);
        bp.stand_up(&bc, p0.body.qx, 0, p0.body.qz, owner, &items, 0, &mut ev)
            .expect("the fixture ladder is armed");
        let mut p = looter_at(lz);
        let mut ev = EventQueue::default();
        let took = bp.loot_nearest(&gc, &t, &mut p, &mut ev);
        let refused = ev
            .entries()
            .iter()
            .any(|e| e.code == EV_MOVE_REFUSED && e.b == REFUSE_M_SAFE);
        (took.is_some(), refused)
    };
    // Another player's bag in the market: refused, and the refusal says so.
    assert_eq!(try_loot(2, -24.0), (false, true));
    // Your own: yours.
    assert_eq!(try_loot(ME, -24.0), (true, false));
    // An animal's: anyone's.
    assert_eq!(try_loot(MOB_ID_TAG | 3, -24.0), (true, false));
    // Another player's bag out past the zone: fair game.
    assert_eq!(try_loot(2, -(town::SAFE_HALF_M + 20.0)), (true, false));
}
