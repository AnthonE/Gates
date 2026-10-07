//! The arena (lane A): the agent against test-only seats that may read the
//! world (`arena/rig.rs`), in lockstep against `ShardCore` with the
//! shipped content and a synthetic clock, so a run is a function of its
//! inputs and nothing here is flaky.
//!
//! [`the_arena_holds_its_floor_and_its_ceiling`] is the gate: a slice of
//! the sweep (seats × loadouts × distances × latency over fixed seeds),
//! its results table printed, a GOOD-handed opportunist pinned to beat the
//! rusher and the strafer and to stay human (it loses to the aimbot, misses
//! a strafer from 30 m, reacts no faster than its hands, never turns faster
//! than they allow, never allocates, and replays). `full_sweep` runs the
//! whole matrix and `trace` one bout tick by tick; both are `#[ignore]`.
//!
//! The rest are scenes: it fights back when rushed, a spear beats a rock,
//! it takes an opening a defender would not, it does not fight what is not
//! a fight (a gatherer beside it, a body it cannot reach, a swing beside
//! it), it heals, hunts, loots, reloads, and takes cover from an archer.

mod common;
#[path = "arena/rig.rs"]
mod rig;

use common::{content, stack, SEED};
use rig::*;
use server::agent::combat::{Mode, Temperament};
use server::agent::hands::Preset;
use server::agent::intent::yaw_toward;
use server::mind::{Mind, MindConfig};
use sim_core::input::BTN_PRIMARY;
use sim_core::limits::{HOTBAR_SLOTS, TICK_HZ};
use sim_core::movement::{Body, POS_XZ_Q};
use std::time::Duration;

/// A meeting on the scene ground: the agent holds `mine`; another body holding `theirs` (if
/// anything) stands 8 m off, in front of it or behind, and rushes it or
/// stands still. Returns the arena when either side is down or a minute
/// has passed.
fn meet(
    temperament: Temperament,
    mine: &str,
    theirs: Option<&str>,
    ahead: bool,
    rush: bool,
) -> Arena {
    let content = content();
    let mut a = Arena::new(temperament);
    // The agent settles in: it has looked around and chosen a goal…
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    // …and has turned to it. "In front" is where it is looking, so the other
    // body is placed once the heading holds: staged mid-turn, toward a goal
    // behind it, "ahead" is a place it is about to turn its back on.
    let mut last = a.player(ID).frame.yaw;
    let mut still = 0;
    for _ in 0..300 {
        a.step();
        let yaw = a.player(ID).frame.yaw;
        still = if yaw == last { still + 1 } else { 0 };
        last = yaw;
        if still >= 15 {
            break;
        }
    }
    let weapon = stack(&content, mine);
    a.stage(ID, |p| {
        p.inv[0] = weapon;
        p.hp = p.hp_max;
    });
    assert!(a.shard.connect(1, RUSHER));
    a.step();
    let (at, facing) = (a.player(ID).body, a.player(ID).frame.yaw);
    let (fx, fz) = sim_core::yaw_dir(facing);
    let off = if ahead { 8.0 } else { -8.0 };
    let (x, z) = (
        at.qx as f32 * POS_XZ_Q + fx * off,
        at.qz as f32 * POS_XZ_Q + fz * off,
    );
    let theirs = theirs.map(|id| stack(&content, id));
    let body = Body::at(SEED, &a.haven, x, z);
    a.stage(RUSHER, |p| {
        p.body = body;
        p.inv[0] = theirs.unwrap_or_default();
    });
    if rush {
        a.sit(Seat::Rusher, 0);
    }
    a.until(60 * TICK_HZ, Arena::decided);
    a.seat = None;
    a
}

/// Rushed at equal gear, the agent fights back: it engages, lands blows,
/// and does it with a person's hands and without allocating.
#[test]
fn a_rushed_survivor_fights_back_at_equal_gear() {
    let a = meet(
        Temperament::Opportunist,
        "item.rock",
        Some("item.rock"),
        false,
        true,
    );
    println!("rock v rock: {}", a.explain());
    let (me, it) = (a.player(ID), a.player(RUSHER));
    println!(
        "agent hp {} wounded {} dead {}; rusher hp {} wounded {} dead {}",
        me.hp, me.wounded, me.dead, it.hp, it.wounded, it.dead
    );
    assert!(a.bot.combat().stats.engages > 0, "{}", a.explain());
    assert!(a.bot.stats.landed > 0, "it only fled: {}", a.explain());
    assert!(a.engaged_ticks > 0);
    // Even gear: it wins the race of blows (the arena's floor), or,
    // losing it, it runs.
    assert!(
        it.wounded || it.dead || a.bot.combat().stats.escapes > 0,
        "{}",
        a.explain()
    );
    assert_eq!(
        a.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    assert!(a.max_turn > 0);
}

/// With the longer reach, the agent beats a rock.
#[test]
fn a_spear_beats_a_rock() {
    let a = meet(
        Temperament::Opportunist,
        "item.spear_wood",
        Some("item.rock"),
        true,
        true,
    );
    println!("spear v rock: {}", a.explain());
    let (me, it) = (a.player(ID), a.player(RUSHER));
    println!(
        "agent hp {} wounded {} dead {}; rusher hp {} wounded {} dead {}",
        me.hp, me.wounded, me.dead, it.hp, it.wounded, it.dead
    );
    assert!(a.bot.stats.landed > 0, "{}", a.explain());
    assert!(
        it.wounded || it.dead,
        "the rusher is still up: {}",
        a.explain()
    );
    assert!(!me.wounded && !me.dead, "{}", a.explain());
    assert_eq!(a.heap_ops, 0);
}

/// An unarmed body standing in front of it is an opening an opportunist
/// takes, and a defensive body leaves alone.
#[test]
fn an_opportunist_takes_an_opening_a_defender_does_not() {
    let a = meet(Temperament::Opportunist, "item.rock", None, true, false);
    println!("opportunist: {}", a.explain());
    let it = a.player(RUSHER);
    assert!(a.bot.stats.landed > 0, "{}", a.explain());
    assert!(it.wounded || it.dead, "{}", a.explain());
    assert_eq!(a.bot.stats.hurts, 0);
    assert_eq!(a.heap_ops, 0);
    let a = {
        let mut a = meet(Temperament::Defensive, "item.rock", None, true, false);
        a.until(20 * TICK_HZ, |_| false);
        a
    };
    println!("defender: {}", a.explain());
    assert_eq!(a.bot.combat().stats.engages, 0, "{}", a.explain());
    assert_eq!(a.bot.stats.landed, 0);
}

/// Two agents at one spawn, both armed and of the shipped temperament, go
/// about their gathering side by side for a minute and a half: a harvest
/// swing beside a body is not an attack on it, nor is walking past it to a
/// tree, neither takes the other for an attacker, and neither takes the
/// other, busy at the trees, as an opening (a spear against a rock is an
/// edge, but not on someone working beside me).
#[test]
fn two_gatherers_side_by_side_do_not_fight() {
    use server::explorer::Phase;
    let mut a = Arena::new(Temperament::Opportunist);
    a.with_peer(Temperament::Opportunist);
    let mut closest = f32::MAX;
    for _ in 0..90 * TICK_HZ {
        a.step();
        let peer = &a.peer.as_ref().unwrap().bot;
        if a.bot.stats.phase == Phase::Harvesting && peer.stats.phase == Phase::Harvesting {
            let (me, it) = (a.player(ID).body, a.player(RUSHER).body);
            let d = ((me.qx - it.qx) as f32 * POS_XZ_Q).hypot((me.qz - it.qz) as f32 * POS_XZ_Q);
            closest = closest.min(d);
        }
    }
    let peer = &a.peer.as_ref().unwrap().bot;
    println!(
        "closest while both harvest {closest:.2} m\nagent: {}\npeer: {:?} {:?}",
        a.explain(),
        peer.combat().stats,
        peer.stats
    );
    assert!(a.bot.stats.gather_awards > 0 && peer.stats.gather_awards > 0);
    for (who, bot) in [("agent", &a.bot), ("peer", peer)] {
        assert_eq!(bot.combat().stats.engages, 0, "{who} picked a fight");
        assert_eq!(bot.stats.hurts, 0, "{who} was struck");
        assert_eq!(bot.stats.landed, 0, "{who} struck a body");
    }
    assert_eq!(a.heap_ops, 0);
}

/// An unarmed body is an opening an opportunist takes; one it never gets
/// any nearer to (the test keeps it a fixed way off, however the agent
/// walks) is let go in seconds and not taken again straight away.
#[test]
fn a_foe_that_cannot_be_reached_is_let_go() {
    let content = content();
    let mut a = Arena::new(Temperament::Opportunist);
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let rock = stack(&content, "item.rock");
    a.stage(ID, |p| {
        p.inv[0] = rock;
        p.hp = p.hp_max;
    });
    assert!(a.shard.connect(1, RUSHER));
    a.step();
    a.stage(RUSHER, |p| p.inv[0] = Default::default());
    let facing = a.player(ID).frame.yaw;
    a.keep_off = Some((6.0, facing));
    a.step();
    assert!(
        a.until(10 * TICK_HZ, |a| a.bot.combat().stats.engages > 0),
        "never took the opening: {}",
        a.explain()
    );
    a.until(20 * TICK_HZ, |_| false);
    println!("unreachable: {}", a.explain());
    let stats = a.bot.combat().stats;
    assert_eq!(stats.engages, 1, "it went back for it: {}", a.explain());
    assert!(stats.parted > 0, "{}", a.explain());
    assert_ne!(a.bot.combat().mode(), Mode::Engage, "{}", a.explain());
    assert!(
        a.engaged_ticks < 6 * TICK_HZ,
        "stood there {} ticks",
        a.engaged_ticks
    );
    assert_eq!(a.bot.stats.landed, 0);
    assert_eq!(a.heap_ops, 0);
}

/// A body swinging a rock just ahead of the agent while it harvests, its
/// look passing a step to one side of it (the swing a body chopping beside
/// it makes), is not attacking it: the agent keeps at its work. Beyond
/// the margin a harvest swing clears a body by, and inside the metre a
/// look ray was once called "at me" by.
#[test]
fn a_swing_beside_me_is_not_an_attack() {
    use server::explorer::Phase;
    const AHEAD_M: f32 = 1.0;
    const RIGHT_M: f32 = 1.0;
    const ASIDE_M: f32 = 0.8;
    let content = content();
    let mut a = Arena::new(Temperament::Defensive);
    let rock = stack(&content, "item.rock");
    assert!(a.shard.connect(1, RUSHER));
    a.step();
    a.stage(RUSHER, |p| p.inv[0] = rock);
    assert!(
        a.until(60 * TICK_HZ, |a| a.bot.stats.phase == Phase::Harvesting),
        "{}",
        a.explain()
    );
    // Ahead and to the right of it, looking back past its right side.
    let me = a.player(ID);
    let (x, z) = (me.body.qx as f32 * POS_XZ_Q, me.body.qz as f32 * POS_XZ_Q);
    let (fx, fz) = sim_core::yaw_dir(me.frame.yaw);
    let (rx, rz) = (fz, -fx);
    let (cx, cz) = (
        x + fx * AHEAD_M + rx * RIGHT_M,
        z + fz * AHEAD_M + rz * RIGHT_M,
    );
    let gap = (x - cx).hypot(z - cz);
    let (ux, uz) = ((x - cx) / gap, (z - cz) / gap);
    let (tx, tz) = (x - uz * ASIDE_M, z + ux * ASIDE_M);
    let body = Body::at(SEED, &a.haven, cx, cz);
    a.stage(RUSHER, |p| p.body = body);
    a.chopper = Some(Chopper {
        yaw: yaw_toward(tx - cx, tz - cz),
    });
    let mut beside = 0;
    for _ in 0..10 * TICK_HZ {
        a.step();
        if a.bot.stats.phase != Phase::Harvesting {
            break;
        }
        beside += 1;
    }
    a.chopper = None;
    println!("chopper: {beside} ticks beside it; {}", a.explain());
    assert!(beside >= 2 * TICK_HZ, "left its work: {}", a.explain());
    assert_eq!(a.bot.stats.hurts, 0, "the swing landed: {}", a.explain());
    assert_eq!(
        a.bot.combat().stats.engages,
        0,
        "took a swing beside it for an attack: {}",
        a.explain()
    );
    assert_eq!(a.heap_ops, 0);
}

/// Hurt with nobody about, it heals, and counts what the bandages already
/// used are still delivering (the server folds one's remainder into the
/// next): from 40 hp it takes three of five, not the lot, to reach 90 %.
#[test]
fn a_heal_counts_the_bandages_still_working() {
    let content = content();
    let mut a = Arena::new(Temperament::Opportunist);
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let mut bandages = stack(&content, "item.bandage");
    bandages.count = 5;
    let slot = sim_core::limits::HOTBAR_SLOTS + 2;
    a.stage(ID, |p| {
        p.inv[slot] = bandages;
        p.hp = 40;
    });
    let full = |a: &Arena| {
        let p = a.player(ID);
        u32::from(p.hp) * 100 >= u32::from(p.hp_max) * 90 && p.heal_rem == 0
    };
    assert!(a.until(40 * TICK_HZ, full), "{}", a.explain());
    // Wherever they are now: meds go on the belt between goals.
    let left: u16 = a
        .player(ID)
        .inv
        .iter()
        .filter(|s| s.count > 0 && s.item == bandages.item)
        .map(|s| s.count)
        .sum();
    println!(
        "healed to {} with {left} left: {}",
        a.player(ID).hp,
        a.explain()
    );
    assert_eq!(left, 2, "used {} bandages", 5 - left);
    assert_eq!(a.heap_ops, 0);
}

/// A mind that heals whenever healing is on offer and otherwise waits.
struct Healer;

impl server::mind::DecisionSource for Healer {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        use server::mind::Goal;
        let goal = if s.offers(Goal::Heal) {
            Goal::Heal
        } else {
            Goal::Wait
        };
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::from_text("test: heal when offered"),
            input_tokens: 0,
            output_tokens: 0,
        })
    }
}

/// The heal goal on its own, above where the reflex bandages: told to
/// heal at 85 %, one bandage takes it past 90 % and the goal ends done
/// with that one counted, the rest left in the pack.
#[test]
fn a_heal_goal_uses_what_it_needs_and_ends_done() {
    use server::mind::{Goal, Outcome};
    let content = content();
    let cfg = MindConfig {
        heartbeat: Duration::from_secs(1),
        per_hour: 100_000,
        per_day: 100_000,
        ..MindConfig::default()
    };
    let mut a = Arena::with(
        Temperament::Opportunist,
        Mind::inline(Healer, cfg).unwrap(),
        false,
    );
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let mut bandages = stack(&content, "item.bandage");
    bandages.count = 5;
    let slot = HOTBAR_SLOTS + 2;
    a.stage(ID, |p| {
        p.inv[slot] = bandages;
        p.hp = p.hp_max * 85 / 100;
    });
    let healed = |a: &Arena| {
        a.bot
            .memory()
            .last
            .is_some_and(|r| r.goal == Goal::Heal && r.outcome != Outcome::Running)
    };
    assert!(a.until(30 * TICK_HZ, healed), "{}", a.explain());
    let report = a.bot.memory().last.unwrap();
    let p = a.player(ID);
    println!(
        "heal goal: {report:?}, {} hp, {} left; {}",
        p.hp,
        p.inv[slot].count,
        a.explain()
    );
    assert_eq!(report.outcome, Outcome::Done, "{}", a.explain());
    assert_eq!(report.gained, 1, "{}", a.explain());
    assert_eq!(p.inv[slot].count, 4);
    assert!(u32::from(p.hp) * 100 >= u32::from(p.hp_max) * 90);
    assert_eq!(a.bot.stats.reflex_heals, 0, "the reflex took it");
    assert_eq!(a.heap_ops, 0);
}

/// A mind that hunts whatever animal is on offer and otherwise waits,
/// looking about: the test's way of pointing the agent at the pig.
struct Hunter;

impl server::mind::DecisionSource for Hunter {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        use server::mind::Goal;
        let goal = if s.offers(Goal::Hunt) {
            Goal::Hunt
        } else {
            Goal::Wait
        };
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::from_text("test: hunt what is in view"),
            input_tokens: 0,
            output_tokens: 0,
        })
    }
}

/// A hunter on the island with one pig: every other animal slot is
/// emptied, and the pig stands `off` metres ahead of the agent, grazing.
/// The agent holds a rock (slot 0) and `weapon` (slot 1), with `rounds` of
/// `round` in the pack.
fn hunt(weapon: &str, round: &str, rounds: u16, off: f32) -> (Arena, usize) {
    let content = content();
    // Asked every second, so a pig glimpsed while looking about is hunted
    // while it is still fresh in the mind's census.
    let cfg = MindConfig {
        heartbeat: Duration::from_secs(1),
        per_hour: 100_000,
        per_day: 100_000,
        ..MindConfig::default()
    };
    let mind = Mind::inline(Hunter, cfg).unwrap();
    let mut a = Arena::with(Temperament::Opportunist, mind, true);
    a.step();
    for m in a.shard.world.mobs.m.iter_mut() {
        m.alive = false;
        m.homed = false;
    }
    let pig = (0..sim_core::limits::MAX_MOBS)
        .find(|&s| sim_core::mob::kind_of(s) == sim_core::mob::MOB_PIG)
        .unwrap();
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let (rock, gun) = (stack(&content, "item.rock"), stack(&content, weapon));
    let mut ammo = stack(&content, round);
    ammo.count = rounds;
    a.stage(ID, |p| {
        p.inv[0] = rock;
        p.inv[1] = gun;
        p.inv[HOTBAR_SLOTS + 3] = ammo;
        p.hp = p.hp_max;
    });
    let me = a.player(ID);
    let (fx, fz) = sim_core::yaw_dir(me.frame.yaw);
    let (x, z) = (
        me.body.qx as f32 * POS_XZ_Q + fx * off,
        me.body.qz as f32 * POS_XZ_Q + fz * off,
    );
    let body = Body::at(SEED, &a.haven, x, z);
    let hp = a.shard.world.mob.def(sim_core::mob::MOB_PIG).hp;
    let m = &mut a.shard.world.mobs.m[pig];
    m.homed = true;
    m.alive = true;
    m.hp = hp;
    m.body = body;
    m.home_qx = body.qx;
    m.home_qz = body.qz;
    m.respawn_at = u64::MAX;
    (a, pig)
}

fn count_of(a: &Arena, id: &str) -> u32 {
    let item = content().item_index(id).unwrap();
    a.player(ID)
        .inv
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| u32::from(s.count))
        .sum()
}

/// With a bow and arrows, the agent stalks a pig crouched, shoots it dead
/// from range (drawing, leading, loosing only once settled), then walks
/// to the carcass bag and loots it, and picks up the arrows lying about.
#[test]
fn the_agent_kills_a_pig_with_a_bow_and_loots_it() {
    use sim_core::input::{BTN_AIM, BTN_CROUCH};
    let (mut a, pig) = hunt("item.bow", "item.arrow_wood", 20, 24.0);
    let mut pressed = 0u8;
    // The rock is in slot 0: a kill with it would be no bow kill.
    let mut clubbed = 0u32;
    for _ in 0..90 * TICK_HZ {
        if !a.shard.world.mobs.m[pig].alive {
            break;
        }
        let f = a.player(ID).frame;
        pressed |= f.buttons;
        if f.buttons & BTN_PRIMARY != 0 && f.sel == 0 {
            clubbed += 1;
        }
        a.step();
    }
    assert!(
        !a.shard.world.mobs.m[pig].alive,
        "the pig lives: {}",
        a.explain()
    );
    let arrows = count_of(&a, "item.arrow_wood");
    let looted = a.until(30 * TICK_HZ, |a| {
        count_of(a, "item.raw_meat") > 0 && !a.bot.combat().engaged()
    });
    let c = a.bot.combat().stats;
    println!(
        "bow hunt: arrows {arrows} after the kill, {} after the pickups; {}",
        count_of(&a, "item.arrow_wood"),
        a.explain()
    );
    assert!(pressed & BTN_AIM != 0, "never drew the bow");
    assert_eq!(clubbed, 0, "swung the rock at the pig: {}", a.explain());
    assert!(pressed & BTN_CROUCH != 0, "never crouched to stalk");
    assert!(c.shots >= 1 && c.won >= 1, "{}", a.explain());
    assert!(looted, "never looted the carcass: {}", a.explain());
    assert!(c.loots >= 1 && c.pickups >= 1, "{}", a.explain());
    assert!(
        count_of(&a, "item.arrow_wood") > arrows,
        "no arrow picked back up: {}",
        a.explain()
    );
    assert_eq!(a.heap_ops, 0);
}

/// With a revolver (bought dry, as it comes off the bench), the agent
/// loads it before it can shoot, kills the pig with it, and tops the
/// cylinder up once things are quiet.
#[test]
fn a_dry_revolver_is_loaded_fired_and_topped_up() {
    let (mut a, pig) = hunt("item.revolver", "item.pistol_ammo", 24, 20.0);
    let gun = content().item_index("item.revolver").unwrap();
    let def = a.shard.world.combat.held_ranged(gun).unwrap();
    let mag = |a: &Arena| a.player(ID).mag[usize::from(def.mag_slot)];
    assert_eq!(mag(&a), 0, "the revolver comes dry");
    // The server's own count: loaded (accepted, not just asked for)
    // before the first shot.
    let mut loaded_first = None;
    for _ in 0..90 * TICK_HZ {
        if !a.shard.world.mobs.m[pig].alive {
            break;
        }
        if mag(&a) > 0 && loaded_first.is_none() {
            loaded_first = Some(a.bot.combat().stats.shots);
        }
        a.step();
    }
    assert!(
        !a.shard.world.mobs.m[pig].alive,
        "the pig lives: {}",
        a.explain()
    );
    assert_eq!(loaded_first, Some(0), "no load before the first shot");
    let after_kill = mag(&a);
    a.until(20 * TICK_HZ, |_| false);
    let c = a.bot.combat().stats;
    println!(
        "revolver: mag {after_kill} after the kill, {} now, rounds {}; {}",
        mag(&a),
        count_of(&a, "item.pistol_ammo"),
        a.explain()
    );
    assert!(c.shots >= 1 && c.won >= 1, "{}", a.explain());
    assert!(after_kill < def.magazine, "no shot spent: {}", a.explain());
    assert_eq!(mag(&a), def.magazine, "never topped up: {}", a.explain());
    assert_eq!(a.heap_ops, 0);
}

/// Hurt in a fight it wins, the agent bandages itself once the foe is dead
/// and its bag taken: under whatever goal it had, with no heal goal asked
/// for. The bandages reach its pack only once the fight is under way, so
/// none goes on before it.
#[test]
fn the_agent_heals_after_a_fight() {
    let content = content();
    let mut a = Arena::new(Temperament::Opportunist);
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let spear = stack(&content, "item.spear_wood");
    // Already knocked about: the spear can win without taking a blow (it
    // outreaches the rock), and a fight it walks out of untouched leaves
    // nothing for this test to see healed.
    a.stage(ID, |p| {
        p.inv[0] = spear;
        p.hp = p.hp_max * 3 / 5;
    });
    assert!(a.shard.connect(1, RUSHER));
    a.step();
    let (at, facing) = (a.player(ID).body, a.player(ID).frame.yaw);
    let (fx, fz) = sim_core::yaw_dir(facing);
    let (x, z) = (
        at.qx as f32 * POS_XZ_Q + fx * 8.0,
        at.qz as f32 * POS_XZ_Q + fz * 8.0,
    );
    let rock = stack(&content, "item.rock");
    let body = Body::at(SEED, &a.haven, x, z);
    a.stage(RUSHER, |p| {
        p.body = body;
        p.inv[0] = rock;
    });
    a.sit(Seat::Rusher, 0);
    assert!(
        a.until(20 * TICK_HZ, |a| a.bot.combat().mode() == Mode::Engage),
        "{}",
        a.explain()
    );
    let slot = HOTBAR_SLOTS + 2;
    let mut bandages = stack(&content, "item.bandage");
    bandages.count = 3;
    a.stage(ID, |p| p.inv[slot] = bandages);
    let won = a.until(60 * TICK_HZ, |a| {
        a.player(RUSHER).dead || a.player(ID).wounded || a.player(ID).dead
    });
    a.seat = None;
    assert!(won && a.player(RUSHER).dead, "{}", a.explain());
    assert_eq!(a.player(ID).inv[slot].count, 3, "a bandage mid-fight");
    let over = a.until(30 * TICK_HZ, |a| !a.bot.combat().engaged());
    let hp = a.player(ID).hp;
    assert!(over, "{}", a.explain());
    assert!(
        u32::from(hp) * 100 < u32::from(a.player(ID).hp_max) * 80,
        "not hurt enough to need a bandage: {hp}"
    );
    let healed = a.until(20 * TICK_HZ, |a| {
        let p = a.player(ID);
        p.hp > hp && p.inv[slot].count < 3
    });
    println!(
        "heal: {hp} hp after the fight, {} now, {} bandages left; {}",
        a.player(ID).hp,
        a.player(ID).inv[slot].count,
        a.explain()
    );
    assert!(healed, "no bandage after the fight: {}", a.explain());
    assert!(a.bot.stats.reflex_heals >= 1, "{}", a.explain());
    assert_eq!(a.heap_ops, 0);
}

/// A body aiming a bow at the agent from 25 m is sidestepped, eyes on it;
/// once it looses arrows, the agent (nothing to shoot back with) runs for
/// a trunk or a rock between them, weaving, rather than across open ground.
#[test]
fn an_archer_is_sidestepped_then_run_from_to_cover() {
    let content = content();
    // A body standing about (the hunter's mind waits with nothing to
    // hunt), so the test is about the archer and not about exploring.
    let mind = Mind::inline(Hunter, MindConfig::default()).unwrap();
    let mut a = Arena::with(Temperament::Opportunist, mind, false);
    // The archer joins with the agent, far off with its bow already up:
    // first seen holding it.
    assert!(a.shard.connect(1, RUSHER));
    a.step();
    let at = a.player(ID).body;
    let bow = stack(&content, "item.bow");
    let far = Body::at(
        SEED,
        &a.haven,
        at.qx as f32 * POS_XZ_Q + 150.0,
        at.qz as f32 * POS_XZ_Q,
    );
    a.stage(RUSHER, |p| {
        p.inv[0] = bow;
        p.body = far;
    });
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let spear = stack(&content, "item.spear_wood");
    a.stage(ID, |p| {
        p.inv[0] = spear;
        p.hp = p.hp_max;
    });
    a.step();
    let (at, facing) = (a.player(ID).body, a.player(ID).frame.yaw);
    let (fx, fz) = sim_core::yaw_dir(facing);
    let (x, z) = (
        at.qx as f32 * POS_XZ_Q + fx * 18.0,
        at.qz as f32 * POS_XZ_Q + fz * 18.0,
    );
    let body = Body::at(SEED, &a.haven, x, z);
    a.stage(RUSHER, |p| p.body = body);
    a.archer = Some(false);
    let dodged = a.until(10 * TICK_HZ, |a| a.bot.combat().stats.evades > 0);
    assert!(dodged, "never sidestepped the bow: {}", a.explain());
    // Side to side: the path is long, the ground covered short.
    let mut moved = 0.0;
    for _ in 0..2 * TICK_HZ {
        let from = a.player(ID).body;
        a.step();
        let to = a.player(ID).body;
        moved += ((to.qx - from.qx) as f32 * POS_XZ_Q).hypot((to.qz - from.qz) as f32 * POS_XZ_Q);
    }
    assert!(moved > 4.0, "stood still under a drawn bow: {moved:.2} m");
    assert_eq!(a.bot.combat().stats.engages, 0, "{}", a.explain());
    assert_eq!(a.bot.combat().mode(), Mode::Idle, "a dodge is not a fight");
    let mut arrows = stack(&content, "item.arrow_wood");
    arrows.count = 50;
    a.stage(RUSHER, |p| p.inv[HOTBAR_SLOTS] = arrows);
    a.archer = Some(true);
    let ran = a.until(20 * TICK_HZ, |a| a.bot.combat().stats.covers > 0);
    a.until(5 * TICK_HZ, |_| false);
    a.archer = None;
    println!("archer: {}", a.explain());
    assert!(ran, "never made for cover: {}", a.explain());
    assert!(a.bot.combat().stats.escapes > 0, "{}", a.explain());
    assert_eq!(a.heap_ops, 0);
}

/// A defensive agent, bow or rock in hand, and a body carrying a bow
/// walking straight up to it from 30 m, eyes level, never drawing or
/// shooting, and stopping 3 m off: a bow carried my way is not an attack
/// (the wire says what is in hand and where it looks, not that it is
/// drawn), so there is no fight and nothing to run from.
#[test]
fn a_bow_carried_my_way_is_not_an_attack() {
    let content = content();
    for mine in ["item.bow", "item.rock"] {
        let mind = Mind::inline(Sentry, MindConfig::default()).unwrap();
        let opts = server::explorer::SurvivorOpts {
            temperament: Temperament::Defensive,
            ..Default::default()
        };
        let mut a = Arena::seated(opts, mind, false, ID, arena_ground(), false);
        assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
        let weapon = stack(&content, mine);
        let mut arrows = stack(&content, "item.arrow_wood");
        arrows.count = 20;
        a.stage(ID, |p| {
            p.inv[0] = weapon;
            p.inv[HOTBAR_SLOTS + 3] = arrows;
            p.hp = p.hp_max;
        });
        a.connect_foe();
        a.step();
        let (at, facing) = (a.player(ID).body, a.player(ID).frame.yaw);
        let (fx, fz) = sim_core::yaw_dir(facing);
        let body = Body::at(
            SEED,
            &a.haven,
            at.qx as f32 * POS_XZ_Q + fx * 30.0,
            at.qz as f32 * POS_XZ_Q + fz * 30.0,
        );
        let bow = stack(&content, "item.bow");
        a.stage(RUSHER, |p| {
            p.inv[0] = bow;
            p.body = body;
        });
        a.walker = Some(3.0);
        a.until(25 * TICK_HZ, |_| false);
        println!(
            "{mine} v a bow walking up: gap {:.1} m, {}",
            a.gap(),
            a.explain()
        );
        let stats = a.bot.combat().stats;
        assert_eq!(stats.engages, 0, "{mine}: {}", a.explain());
        assert_eq!(stats.escapes, 0, "{mine}: {}", a.explain());
        assert!(a.gap() < 4.0, "the walker never arrived: {:.1} m", a.gap());
        assert_eq!(a.bot.stats.landed, 0);
        assert!(!a.player(RUSHER).wounded && !a.player(RUSHER).dead);
        assert_eq!(a.heap_ops, 0);
    }
}

const DISTANCES: [f32; 3] = [5.0, 15.0, 30.0];

/// Mine against theirs.
const LOADOUTS: [(Gear, Gear); 5] = [
    (Gear::ROCK, Gear::ROCK),
    (Gear::SPEAR, Gear::ROCK),
    (Gear::SPEAR, Gear::SPEAR),
    (Gear::BOW, Gear::ROCK),
    (Gear::BOW, Gear::BOW),
];

/// Floors, pinned under what the sweep measured (`full_sweep` prints the
/// rest). Per cent of bouts won.
///
/// Melee at equal gear against the rusher and the strafer, no lag:
/// measured 42/48 when pinned.
const FLOOR_MELEE_PCT: u32 = 80;
/// Bow against bow, the rusher: measured 9/12, then 8/12, then 7/12 once
/// arrows flew at Rust's speeds and drop (2026-10) — both sides' arrows
/// land more and the 15 m bouts flipped to the rusher. Floored at half: it
/// wins the bow fight at least as often as it loses it. (The strafer
/// outshoots it: its aim is perfect and its eyes have no lag; reported,
/// not floored.)
const FLOOR_BOW_PCT: u32 = 50;
/// Melee at equal gear against the strafer, ~100 ms round trip: measured
/// 10/12. (The rusher at 100 ms is reported, not floored: a body with no
/// lag at all walking into a lagged one wins most first blows, as it
/// would against a person.)
const FLOOR_LAG_PCT: u32 = 60;
/// Ceilings. The aimbot wins at least this share at equal melee gear...
const AIMBOT_MIN_PCT: u32 = 75;
/// ...and the agent's arrows loosed from 25 m or further land no more than
/// this share on a strafing archer. The agent is held where it stands 30 m
/// off (left alone it closes to `DUEL_M` before it shoots at a player),
/// and at least this many such shots are taken, so the share is measured
/// and not passed by shooting nothing.
const BOW_30M_MAX_HIT_PCT: u32 = 50;
const BOW_30M_MIN_SHOTS: u32 = 12;

/// The arena's gates, over one sweep (run on every core):
/// - **floor**: a GOOD-handed opportunist beats the rusher and the
///   strafer at equal gear (melee, and bow against the rusher; melee
///   against the strafer at ~100 ms too);
/// - **ceiling**: it loses to the aimbot; held 30 m off a strafer, it
///   lands few of its arrows loosed from 25 m or further; its first blow comes no sooner than its reaction after the
///   foe is first on its screen, ambushed or not; its view never turns
///   faster than its hands allow; its frames never touch the heap;
/// - and one bout replays to the same world hash.
#[test]
fn the_arena_holds_its_floor_and_its_ceiling() {
    let started = std::time::Instant::now();
    let melee = [(Gear::ROCK, Gear::ROCK), (Gear::SPEAR, Gear::SPEAR)];
    let mut setups: Vec<(&str, Setup)> = Vec::new();
    for seat in [Seat::Rusher, Seat::Strafer] {
        for (mine, theirs) in melee {
            for dist in DISTANCES {
                for seed in 0..4 {
                    setups.push(("floor", Setup::new(seat, mine, theirs, dist, seed)));
                }
            }
        }
    }
    for dist in DISTANCES {
        for seed in 0..4 {
            let s = Setup::new(Seat::Rusher, Gear::BOW, Gear::BOW, dist, seed);
            setups.push(("bow floor", s));
        }
    }
    for (mine, theirs) in melee {
        for dist in DISTANCES {
            for seed in 0..2 {
                let mut s = Setup::new(Seat::Strafer, mine, theirs, dist, seed);
                s.latency = Latency::RTT_100MS;
                setups.push(("lag floor", s));
                setups.push(("aimbot", Setup::new(Seat::Aimbot, mine, theirs, dist, seed)));
            }
        }
    }
    for seed in 0..8 {
        let mut s = Setup::new(Seat::Strafer, Gear::BOW, Gear::BOW, 30.0, seed);
        if seed < 4 {
            setups.push(("bow strafer", s));
        }
        // Held there, it dies in a few of the seat's arrows: twice the
        // seeds for enough of its own.
        s.pinned = true;
        setups.push(("bow 30m", s));
    }
    for seed in 0..4 {
        // Placed on its screen within arm's reach, coming on at once.
        let mut s = Setup::new(Seat::Rusher, Gear::ROCK, Gear::ROCK, 2.0, seed);
        s.ambush = true;
        setups.push(("ambush", s));
    }
    let replay = Setup::new(Seat::Strafer, Gear::SPEAR, Gear::SPEAR, 15.0, 1);
    setups.push(("replay", replay));
    setups.push(("replay", replay));
    let plain: Vec<Setup> = setups.iter().map(|(_, s)| *s).collect();
    let records = sweep(&plain);
    table("arena", &plain, &records);
    let tally = |gate: &str| {
        let mut t = Tally::default();
        for ((g, _), r) in setups.iter().zip(&records) {
            if *g == gate {
                t.add(r);
            }
        }
        t
    };
    let (floor, bow, lag, aimbot, far, ambush) = (
        tally("floor"),
        tally("bow floor"),
        tally("lag floor"),
        tally("aimbot"),
        tally("bow 30m"),
        tally("ambush"),
    );
    let strafer = tally("bow strafer");
    println!("\n== gates ==");
    println!("{}", row("floor: melee v rusher, strafer", &floor));
    println!("{}", row("floor: bow v bow rusher", &bow));
    println!("{}", row("floor: melee v strafer 100ms", &lag));
    println!("{}", row("ceiling: melee v aimbot", &aimbot));
    println!(
        "{}  from 25 m+: {}/{} hit",
        row("ceiling: bow v strafer 30m pinned", &far),
        far.far_hits,
        far.far_shots
    );
    println!("{}", row("ceiling: ambushed at 2m", &ambush));
    // Not gated, and failing: the agent's bow against a strafing archer
    // (perfect aim, no lag in its eyes) at 30 m, left free to close.
    println!("{}", row("REPORTED (failing): bow v strafer", &strafer));
    let quickest = setups
        .iter()
        .zip(&records)
        .filter(|((g, _), _)| *g == "ambush")
        .filter_map(|(_, r)| r.sight_to_hit)
        .min();
    println!(
        "ambushed, the first blow landed {quickest:?} ticks after the foe was first on screen (reaction {} ticks)",
        Preset::Good.skill().react_frames
    );
    println!("{:.1}s", started.elapsed().as_secs_f32());
    for ((gate, s), r) in setups.iter().zip(&records) {
        let what = format!("{gate}: {} seed {}: {r:?}", label(s), s.seed);
        assert_eq!(r.heap_ops, 0, "the agent's frames touched the heap: {what}");
        assert!(r.max_turn <= r.turn_cap, "turned past its hands: {what}");
        if let Some(ticks) = r.sight_to_hit {
            assert!(
                ticks >= r.react_floor,
                "its first blow came {ticks} ticks after the foe was first on its screen: {what}"
            );
        }
    }
    assert!(
        floor.win_pct() >= FLOOR_MELEE_PCT,
        "{}",
        row("floor", &floor)
    );
    assert!(bow.win_pct() >= FLOOR_BOW_PCT, "{}", row("bow floor", &bow));
    assert!(lag.win_pct() >= FLOOR_LAG_PCT, "{}", row("lag floor", &lag));
    assert!(
        aimbot.losses * 100 >= AIMBOT_MIN_PCT * aimbot.bouts,
        "{}",
        row("aimbot", &aimbot)
    );
    assert!(
        far.far_shots >= BOW_30M_MIN_SHOTS,
        "only {} shots from 25 m+: {}",
        far.far_shots,
        row("bow 30m", &far)
    );
    assert!(
        far.far_hit_pct() <= BOW_30M_MAX_HIT_PCT,
        "{}/{} from 25 m+: {}",
        far.far_hits,
        far.far_shots,
        row("bow 30m", &far)
    );
    // Ambushed, it still answers: it lands a blow, after its reaction.
    assert!(ambush.hits > 0, "{}", row("ambush", &ambush));
    let (a, b) = (records[records.len() - 2], records[records.len() - 1]);
    assert_eq!(
        (a.hash, a.ticks, a.hits, a.their_hits),
        (b.hash, b.ticks, b.hits, b.their_hits),
        "one bout, two different worlds"
    );
}

/// The full sweep, printed: every seat, loadout, distance, preset and
/// latency over four seeds. `ARENA_ONLY="rock v rock"` keeps the rows
/// whose label has it. Not a gate: the gates run a slice of it.
#[test]
#[ignore]
fn full_sweep() {
    let started = std::time::Instant::now();
    let mut setups = Vec::new();
    for preset in Preset::ALL {
        for latency in [Latency::NONE, Latency::RTT_100MS] {
            for seat in Seat::ALL {
                for (mine, theirs) in LOADOUTS {
                    for dist in DISTANCES {
                        for seed in 0..4 {
                            let mut s = Setup::new(seat, mine, theirs, dist, seed);
                            s.preset = preset;
                            s.latency = latency;
                            setups.push(s);
                        }
                    }
                }
            }
        }
    }
    if let Ok(only) = std::env::var("ARENA_ONLY") {
        setups.retain(|s| label(s).contains(&only));
    }
    let records = sweep(&setups);
    table("full sweep", &setups, &records);
    if std::env::var("ARENA_BOUTS").is_ok() {
        for (s, r) in setups.iter().zip(&records) {
            println!("{} seed {}: {r:?}", label(s), s.seed);
        }
    }
    println!("{:.1}s", started.elapsed().as_secs_f32());
}

/// One bout, every tick printed: `ARENA="rusher rock 5 0"` (seat, gear or
/// `mine/theirs`, metres, seed, and optionally preset and `100` for the
/// round trip).
#[test]
#[ignore]
fn trace() {
    let spec = std::env::var("ARENA").unwrap_or_else(|_| "rusher rock 5 0".into());
    let w: Vec<&str> = spec.split_whitespace().collect();
    let seat = *Seat::ALL.iter().find(|s| s.name() == w[0]).unwrap();
    let gear = |name: &str| match name {
        "rock" => Gear::ROCK,
        "spear" => Gear::SPEAR,
        _ => Gear::BOW,
    };
    // `bow/rock`: mine against theirs.
    let (mine, theirs) = w[1].split_once('/').unwrap_or((w[1], w[1]));
    let mut s = Setup::new(
        seat,
        gear(mine),
        gear(theirs),
        w[2].parse().unwrap(),
        w[3].parse().unwrap(),
    );
    // Then, optionally, the preset and the round trip in ms.
    if let Some(p) = w.get(4) {
        s.preset = Preset::parse(p).unwrap();
    }
    if w.get(5).is_some_and(|ms| *ms != "0") {
        s.latency = Latency::RTT_100MS;
    }
    s.trace = true;
    let (a, r) = rig::bout(s);
    println!("{r:?}\n{}", a.explain());
}
