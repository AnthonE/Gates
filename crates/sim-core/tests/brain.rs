//! The animal brain (`brain.rs`): the behaviours the reference's AI has and
//! the old one-timer animal did not — the crouch blind spot, the pack that
//! answers one wolf, the cap on how many close in at once, and the torch.
//!
//! Numbers come from `MobContent::probe_fixture`; positions are set, not
//! walked to, and every assertion reads sim state.

use sim_core::brain::{AiState, NO_TARGET, PACK_BITERS};
use sim_core::combat::CombatContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_CROUCH, BTN_LIGHT};
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
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.mob = MobContent::probe_fixture();
    w.gather = GatherContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(1));
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
