//! The animal brain (`brain.rs`): the behaviours the reference's AI has and
//! the old one-timer animal did not — the crouch blind spot, the pack that
//! answers one wolf, the cap on how many close in at once, and the torch.
//!
//! Numbers come from `MobContent::probe_fixture`; positions are set, not
//! walked to, and every assertion reads sim state.

use sim_core::brain::{AiState, NO_TARGET, PACK_BITERS};
use sim_core::combat::CombatContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_CROUCH, BTN_LIGHT, BTN_SPRINT};
use sim_core::limits::{MAX_MOBS, MOB_THINK_TICKS};
use sim_core::mob::{self, MobContent, MOB_PIG, MOB_WOLF};
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::nav::yaw_toward;
use sim_core::world::{Command, World};

/// `tests/mob.rs`'s memo, for its reason: `terrain::haven` is thousands of
/// height taps and a pure function of the seed.
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

const SEED: u64 = 11;

/// A joined player and a world holding only the named roster slots, each
/// stood at an offset (metres, planar) from the player, homed there (so the
/// leash does not walk it off before it has had a chance to sense), and
/// calmed: no route, no timer.
fn world_with(animals: &[(usize, f32, f32)]) -> World {
    world_at(None, animals)
}

/// `world_with`, with the player joined at `spawn` rather than on the beach.
fn world_at(spawn: Option<(f32, f32)>, animals: &[(usize, f32, f32)]) -> World {
    let mut w = World::new(SEED);
    // Clear skies held: these measure notice ranges, and the weather
    // schedule would otherwise put fog on some test tick (weather v0).
    w.env = sim_core::weather::Env::CLEAR;
    w.combat = CombatContent::probe_fixture();
    w.mob = MobContent::probe_fixture();
    w.gather = GatherContent::probe_fixture();
    w.dev_spawn = Some(spawn.unwrap_or_else(|| w.spawn_pos(1)));
    w.tick(&[Command::Join { id: 1 }]);
    let keep: Vec<usize> = animals.iter().map(|a| a.0).collect();
    for (i, m) in w.mobs.m.iter_mut().enumerate() {
        if !keep.contains(&i) {
            m.alive = false;
        }
    }
    let b = w.players[0].body;
    let (px, pz) = (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q);
    for &(slot, dx, dz) in animals {
        assert!(w.mobs.m[slot].alive, "slot {slot} is not a live animal");
        let m = &mut w.mobs.m[slot];
        m.body = Body::at(SEED, hv(SEED), px + dx, pz + dz);
        m.home_qx = m.body.qx;
        m.home_qz = m.body.qz;
        m.path.clear();
        m.state = AiState::Idle;
        m.state_until = u64::MAX;
        m.gait = 0;
        m.last_qx = m.body.qx;
        m.last_qz = m.body.qz;
    }
    w
}

/// Point a slot's head (and the heading it wants) at or away from the
/// player.
fn face(w: &mut World, slot: usize, toward: bool) {
    let (m, p) = (w.mobs.m[slot].body, w.players[0].body);
    let (dx, dz) = ((p.qx - m.qx) as f32, (p.qz - m.qz) as f32);
    let yaw = if toward {
        yaw_toward(dx, dz, 0)
    } else {
        yaw_toward(-dx, -dz, 0)
    };
    w.mobs.m[slot].yaw = yaw;
    w.mobs.m[slot].want_yaw = yaw;
}

/// Tick with the player holding `buttons` and never moving.
fn hold(w: &mut World, buttons: u8, ticks: u32, mut each: impl FnMut(&World)) {
    for seq in 0..ticks {
        let frame = InputFrame {
            seq: seq as u16,
            buttons,
            sel: 0,
            ..InputFrame::default()
        };
        w.tick(&[Command::Input {
            id: 1,
            frame,
            favour: 0,
        }]);
        each(w);
    }
}

fn first(w: &World, kind: u8) -> usize {
    w.mobs
        .m
        .iter()
        .position(|m| m.alive && m.kind == kind)
        .expect("a live animal of that kind")
}

/// The free wolves of the first pack: guards take the first
/// `SITE_GUARDS` predator slots, and the next `PACK_SIZE` den together.
fn free_pack() -> [usize; 3] {
    let leader = (0..MAX_MOBS)
        .find(|&s| mob::pack_leader_of(s) == Some(s))
        .expect("a free wolf leads a pack");
    let mates: Vec<usize> = (0..MAX_MOBS)
        .filter(|&s| mob::pack_leader_of(s) == Some(leader))
        .collect();
    assert_eq!(mates.len(), mob::PACK_SIZE, "the first pack is whole");
    [mates[0], mates[1], mates[2]]
}

/// **The blind spot.** A crouched player five metres behind a pig is not
/// noticed; the same player in front of it is, and so is a player standing
/// up behind it. The first is the reference's `IgnoreNonVisionSneakers` —
/// the one way to get a swing in on a boar before it turns.
#[test]
fn a_crouched_player_behind_a_pig_is_not_noticed() {
    let probe = |toward: bool, buttons: u8| -> bool {
        let slot = {
            let w = World::new(SEED);
            let mut w2 = w;
            w2.mob = MobContent::probe_fixture();
            w2.tick(&[]);
            first(&w2, MOB_PIG)
        };
        let mut w = world_with(&[(slot, 5.0, 0.0)]);
        face(&mut w, slot, toward);
        hold(&mut w, buttons, MOB_THINK_TICKS as u32 + 1, |_| {});
        w.mobs.m[slot].roused_until > 0
    };
    assert!(
        !probe(false, BTN_CROUCH),
        "a pig noticed a crouched player in its blind spot"
    );
    assert!(
        probe(true, BTN_CROUCH),
        "a pig looking at a crouched player five metres off did not see them"
    );
    assert!(
        probe(false, 0),
        "a pig did not hear a player standing up behind it"
    );
}

/// **The pack answers.** One wolf of a pack notices the player; its
/// pack-mate 45 m from the player — outside its own 30 m senses, inside the
/// 40 m call — takes the same target. With the call disarmed in content the
/// same mate never does.
#[test]
fn a_wolf_outside_its_own_senses_answers_its_pack() {
    let [a, b, _] = free_pack();
    for (armed, joins) in [(true, true), (false, false)] {
        let mut w = world_with(&[(a, 10.0, 0.0), (b, 45.0, 0.0)]);
        if !armed {
            w.mob.defs[MOB_WOLF as usize].pack_cm = 0;
        }
        hold(&mut w, 0, 2 * MOB_THINK_TICKS as u32 + 1, |_| {});
        assert_ne!(
            w.mobs.m[a].target, NO_TARGET,
            "the near wolf noticed nobody"
        );
        let got = w.mobs.m[b].target == 0 && w.mobs.m[b].roused_until > w.tick;
        assert_eq!(
            got, joins,
            "pack call armed={armed}: the far wolf's target is {} (roused until {})",
            w.mobs.m[b].target, w.mobs.m[b].roused_until
        );
    }
}

/// **They take turns.** Three wolves round one player: never more than
/// `PACK_BITERS` are biting at once, and the one left over circles.
#[test]
fn no_more_than_two_wolves_bite_one_player_at_once() {
    let pack = free_pack();
    let mut w = world_with(&[
        (pack[0], 3.0, 0.0),
        (pack[1], -3.0, 0.0),
        (pack[2], 0.0, 3.0),
    ]);
    let mut most = 0;
    let mut circled = false;
    hold(&mut w, 0, 8 * MOB_THINK_TICKS as u32, |w| {
        let biting = pack
            .iter()
            .filter(|&&s| w.mobs.m[s].state == AiState::Attack && w.mobs.m[s].target == 0)
            .count();
        most = most.max(biting);
        circled |= pack.iter().any(|&s| w.mobs.m[s].state == AiState::Orbit);
    });
    assert!(most >= 1, "nobody ever bit");
    assert!(
        most <= PACK_BITERS,
        "{most} wolves were biting one player at once"
    );
    assert!(circled, "the wolf that had to wait never circled");
}

/// **A lit torch holds a wolf off.** Ten seconds five metres from a wolf
/// with a burning torch in hand costs nothing; the same ten seconds with
/// the torch out is a mauling.
#[test]
fn a_lit_torch_keeps_a_wolf_from_biting() {
    let [a, _, _] = free_pack();
    for (buttons, safe) in [(BTN_LIGHT, true), (0, false)] {
        let mut w = world_with(&[(a, 5.0, 0.0)]);
        // The gather fixture's item 0 is its light, full of fuel.
        w.players[0].inv[0] = ItemStack {
            item: 0,
            count: 1,
            cond: 400,
            skin: 0,
        };
        let full = w.players[0].hp;
        let mut circled = false;
        hold(&mut w, buttons, 300, |w| {
            circled |= w.mobs.m[a].state == AiState::Orbit;
        });
        let unhurt = w.players[0].hp == full;
        assert_eq!(
            unhurt,
            safe,
            "torch lit={}: hp {} of {full}",
            buttons != 0,
            w.players[0].hp
        );
        if safe {
            assert!(circled, "a wolf held off by fire should circle it");
        }
    }
}

/// Planar distance², cm², from the player to a roster slot.
fn gap2(w: &World, slot: usize) -> i64 {
    let (m, p) = (w.mobs.m[slot].body, w.players[0].body);
    let (dx, dz) = ((m.qx - p.qx) as i64 * 3, (m.qz - p.qz) as i64 * 3);
    dx * dx + dz * dz
}

/// **A gunshot sends a pig running and brings a wolf to look.** Both stand
/// well outside their notice radius (the pig's 12 m, the wolf's 30 m), so
/// neither would ever react to a player standing still there. A shot at the
/// player is heard inside 100 m: the pig bolts away from the spot, and the
/// wolf jogs toward it until it notices the player and the hunt starts.
/// The control is the same stand-off with no shot, and nobody moves toward
/// or away on purpose.
#[test]
fn a_gunshot_sends_a_pig_running_and_brings_a_wolf_to_look() {
    use sim_core::noise::{Noise, NOISE_GUN_CM};
    let pig = {
        let mut w = World::new(SEED);
        w.mob = MobContent::probe_fixture();
        w.tick(&[]);
        first(&w, MOB_PIG)
    };
    let [wolf, _, _] = free_pack();
    let shoot = |w: &mut World| {
        let p = w.players[0].body;
        let at = w.tick;
        w.noises.push(Noise {
            qx: p.qx,
            qz: p.qz,
            radius_cm: NOISE_GUN_CM,
            at,
        });
    };

    // Staged inland, at a pig's home, with each animal placed along a
    // bearing that is dry land all the way out: the leash sends anything
    // standing on the beach home before it will do anything else, which is
    // a different test.
    let w0 = {
        let mut w = World::new(SEED);
        w.mob = MobContent::probe_fixture();
        w
    };
    let home = (
        w0.mobs.m[pig].home_qx as f32 * POS_XZ_Q,
        w0.mobs.m[pig].home_qz as f32 * POS_XZ_Q,
    );
    let inland = |r: f32| -> (f32, f32) {
        (0..16u16)
            .map(|k| sim_core::yaw_dir(k << 12))
            .find(|&(dx, dz)| {
                (1..=10).all(|i| {
                    let t = r * i as f32 / 10.0;
                    let (x, z) = (home.0 + dx * t, home.1 + dz * t);
                    sim_core::terrain::height(SEED, x, z) > sim_core::terrain::BEACH_MAX_H
                })
            })
            .map(|(dx, dz)| (dx * r, dz * r))
            .expect("no dry bearing from the pig's home")
    };

    // The pig, shot at and not.
    for shot in [true, false] {
        let (dx, dz) = inland(40.0);
        let mut w = world_at(Some(home), &[(pig, dx, dz)]);
        let before = gap2(&w, pig);
        if shot {
            shoot(&mut w);
        }
        let mut fled = false;
        hold(&mut w, 0, 4 * MOB_THINK_TICKS as u32, |w| {
            fled |= w.mobs.m[pig].state == AiState::Flee;
        });
        assert_eq!(fled, shot, "shot={shot}: the pig's flight");
        if shot {
            assert!(
                gap2(&w, pig) > before,
                "a pig that heard the shot did not open distance from it"
            );
        }
    }

    // The wolf, shot near and not.
    for shot in [true, false] {
        let (dx, dz) = inland(55.0);
        let mut w = world_at(Some(home), &[(wolf, dx, dz)]);
        if shot {
            shoot(&mut w);
        }
        let mut looked = false;
        let mut hunted = false;
        hold(&mut w, 0, 20 * MOB_THINK_TICKS as u32, |w| {
            looked |= w.mobs.m[wolf].state == AiState::MoveTowards;
            hunted |= w.mobs.m[wolf].target == 0 && w.mobs.m[wolf].roused_until > w.tick;
        });
        assert_eq!(looked, shot, "shot={shot}: the wolf going to look");
        assert_eq!(hunted, shot, "shot={shot}: the look turning into a hunt");
    }
}

/// **Out of combat, a wound heals** — after a minute unstruck with nobody
/// remembered, and not a tick before. A pig that got away comes back
/// whole, which is the reference's reworked wolf.
#[test]
fn a_wounded_animal_left_alone_heals_after_a_minute() {
    let pig = {
        let mut w = World::new(SEED);
        w.mob = MobContent::probe_fixture();
        w.tick(&[]);
        first(&w, MOB_PIG)
    };
    let mut w = world_with(&[(pig, 100.0, 0.0)]);
    let full = w.mob.def(MOB_PIG).hp;
    w.mobs.m[pig].hp = full / 8;
    hold(&mut w, 0, 1_700, |_| {});
    assert_eq!(
        w.mobs.m[pig].hp,
        full / 8,
        "the pig healed inside its first out-of-combat minute"
    );
    hold(&mut w, 0, 60 * MOB_THINK_TICKS as u32, |_| {});
    assert_eq!(w.mobs.m[pig].hp, full, "the pig never healed back to whole");
}

/// **One howl a hunt.** The pack-mate that found the player howls; the one
/// that answered the call does not howl back, and the finder does not howl
/// again while the hunt runs (`brain::HOWL_COOLDOWN_TICKS`). The events are
/// what reaches clients (`EV_HOWL`), so they are what is counted.
#[test]
fn a_pack_howls_once_when_it_finds_someone() {
    use sim_core::world::EV_HOWL;
    let [a, b, _] = free_pack();
    let mut w = world_with(&[(a, 10.0, 0.0), (b, 45.0, 0.0)]);
    let mut from_a = 0;
    let mut from_b = 0;
    hold(&mut w, 0, 10 * MOB_THINK_TICKS as u32, |w| {
        for e in w.events.entries().iter().filter(|e| e.code == EV_HOWL) {
            if e.a == mob::mob_id(a) {
                from_a += 1;
            } else if e.a == mob::mob_id(b) {
                from_b += 1;
            }
        }
    });
    assert_eq!(
        from_a, 1,
        "the wolf that found the player howled {from_a} times"
    );
    assert_eq!(from_b, 0, "the wolf that answered the call howled too");
    assert_eq!(w.mobs.m[b].target, 0, "the call went unanswered");
}

/// Land one blow from the player on a roster slot, the way a swing that
/// found it does (`mob::strike_slot`), with the combat fixture's spear.
fn strike(w: &mut World, slot: usize) {
    w.players[0].inv[0] = ItemStack {
        item: 0,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.players[0].frame.sel = 0;
    let tick = w.tick;
    assert!(
        mob::strike_slot(
            &w.combat,
            &w.backpack,
            &w.mob,
            tick,
            0,
            &w.players,
            &mut w.mobs,
            &mut w.backpacks,
            &mut w.events,
            slot,
        ),
        "the blow did not land"
    );
}

/// **Hit a wolf before it has seen you and it backs off, calls its pack, and
/// comes back with it** (the reference's reworked wolf). A crouched player
/// in the blind spot strikes the pack's leader; it retreats, howls, the
/// mate 38 m off — inside the 40 m call, and deaf to a crouched player at
/// that range — answers, and the retreat's timer turns the leader round
/// into a charge.
#[test]
fn an_ambushed_wolf_backs_off_calls_its_pack_and_comes_back() {
    use sim_core::world::EV_HOWL;
    let [a, b, _] = free_pack();
    let mut w = world_with(&[(a, 1.5, 0.0), (b, 38.0, 0.0)]);
    face(&mut w, a, false);
    // A crouched, still player behind it: unnoticed until the blow.
    hold(&mut w, BTN_CROUCH, MOB_THINK_TICKS as u32 + 1, |_| {});
    assert_eq!(
        w.mobs.m[a].roused_until, 0,
        "the wolf saw the ambush coming"
    );
    strike(&mut w, a);
    assert!(w.mobs.m[a].ambushed, "a blow out of nowhere is an ambush");

    let (mut retreated, mut howled, mut came_back, mut answered) = (false, false, false, false);
    hold(&mut w, BTN_CROUCH, 10 * MOB_THINK_TICKS as u32, |w| {
        retreated |= w.mobs.m[a].state == AiState::Flee;
        answered |= w.mobs.m[b].target == 0 && w.mobs.m[b].roused_until > w.tick;
        howled |= w
            .events
            .entries()
            .iter()
            .any(|e| e.code == EV_HOWL && e.a == mob::mob_id(a));
        came_back |= retreated && matches!(w.mobs.m[a].state, AiState::Chase | AiState::Attack);
    });
    assert!(retreated, "the ambushed wolf never backed off");
    assert!(howled, "the ambushed wolf never called its pack");
    assert!(answered, "the pack never answered the call");
    assert!(came_back, "the retreat never turned into a charge");
    assert!(
        !w.mobs.m[a].ambushed,
        "back in the fight, it is no longer ambushed"
    );
}

/// **A blow on one wolf makes its charging pack-mates break off and circle**
/// ("a pack-mate getting hit makes the others stop charging").
#[test]
fn a_hit_on_one_wolf_makes_its_charging_mate_circle() {
    let [a, b, _] = free_pack();
    let mut w = world_with(&[(a, 3.0, 0.0), (b, 20.0, 0.0)]);
    hold(&mut w, 0, MOB_THINK_TICKS as u32 + 1, |_| {});
    assert_eq!(
        w.mobs.m[b].state,
        AiState::Chase,
        "the far wolf is not charging yet"
    );
    strike(&mut w, a);
    let mut circled = false;
    hold(&mut w, 0, MOB_THINK_TICKS as u32 + 1, |w| {
        circled |= w.mobs.m[b].state == AiState::Orbit;
    });
    assert!(
        circled,
        "the charging mate kept coming after its pack-mate was hit"
    );
}

/// Tick with the player holding `buttons` and pushing forward.
fn hold_moving(w: &mut World, buttons: u8, ticks: u32) {
    for seq in 0..ticks {
        let frame = InputFrame {
            seq: seq as u16,
            buttons,
            move_z: 127,
            sel: 0,
            ..InputFrame::default()
        };
        w.tick(&[Command::Input {
            id: 1,
            frame,
            favour: 0,
        }]);
    }
}

/// **A crouch-sprint is a sprint.** `movement::step` reads no crouch, so a
/// player holding both runs at full speed; the pig behind them hears a
/// runner, not a stalker. Crouch was asked before sprint, which made
/// holding both the quietest way to cross the island at a run.
#[test]
fn a_crouch_sprint_behind_a_pig_is_heard() {
    let slot = {
        let mut w = World::new(SEED);
        w.mob = MobContent::probe_fixture();
        w.tick(&[]);
        first(&w, MOB_PIG)
    };
    let mut w = world_with(&[(slot, 5.0, 0.0)]);
    face(&mut w, slot, false);
    hold_moving(&mut w, BTN_CROUCH | BTN_SPRINT, MOB_THINK_TICKS as u32 + 1);
    assert!(
        w.mobs.m[slot].roused_until > 0,
        "a pig did not hear a player crouch-sprinting past behind it"
    );
}

/// **Dormancy ends in an Idle that ends.** An animal nobody can see goes
/// dormant as Idle; the state it left may have had no timer (a chase, a
/// patrol, the walk home), and an Idle that kept `u64::MAX` never timed
/// out, so the animal stood still once woken until somebody came close —
/// guards stopped their rounds for good once their site emptied.
#[test]
fn an_animal_woken_from_dormancy_does_not_stand_forever() {
    let slot = {
        let mut w = World::new(SEED);
        w.mob = MobContent::probe_fixture();
        w.tick(&[]);
        first(&w, MOB_PIG)
    };
    // 300 m off: past the 240 m wake radius, so the first think is dormant.
    let mut w = world_with(&[(slot, 300.0, 0.0)]);
    w.mobs.m[slot].state = AiState::Chase;
    w.mobs.m[slot].state_until = u64::MAX;
    hold(&mut w, 0, MOB_THINK_TICKS as u32 + 1, |_| {});
    let m = &w.mobs.m[slot];
    assert!(!m.awake, "the pig 300 m off was awake");
    assert_eq!(m.state, AiState::Idle, "dormancy is Idle");
    assert_ne!(
        m.state_until,
        u64::MAX,
        "dormancy left an Idle that can never time out"
    );

    // Now bring it within the wake radius but well outside its senses.
    let b = w.players[0].body;
    let (px, pz) = (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q);
    {
        let m = &mut w.mobs.m[slot];
        m.body = Body::at(SEED, hv(SEED), px + 60.0, pz);
        m.home_qx = m.body.qx;
        m.home_qz = m.body.qz;
        m.last_qx = m.body.qx;
        m.last_qz = m.body.qz;
    }
    let mut left_idle = false;
    hold(&mut w, 0, 20 * MOB_THINK_TICKS as u32, |w| {
        left_idle |= w.mobs.m[slot].state != AiState::Idle;
    });
    assert!(
        left_idle,
        "a woken pig stood in Idle for ten seconds with nobody near"
    );
}
