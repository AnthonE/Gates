//! `hunt` — a naked spawn can actually catch and kill a pig, and the kill
//! leaves a corpse bag whose loot is a raw food a campfire will take. **And
//! the inverse**: a wolf can catch and kill a player who does not react.
//!
//! The two directions are one file because they are one claim measured from
//! both ends — that the roster is a thing a player has an encounter with
//! rather than scenery with hit points. The first test is the encounter the
//! player wins by choosing to; the second is the one they lose by not
//! choosing at all.
//!
//! **This is the gate for a defect no other gate could see.** The pig's
//! flight speed shipped at 100% of `SPRINT_SPEED` for one build: every
//! automated check was green, the animal was correct in every measurable
//! way, and it simply could not be caught — a sprinting player never closed
//! the gap, so the first thing a naked spawn was supposed to be able to
//! kill was unkillable forever. It was found by booting the game and
//! looking, which does not scale and does not run in CI.
//!
//! What makes it gateable is that "catchable" is not a feeling: it is a
//! chase, run in the sim, against the shipped content, with the same
//! `movement::step` both bodies use. The player here does exactly what a
//! player does — sprint at it and swing — and the assertion is that the pig
//! is dead at the end. `tests/content.rs` gates the ratio that makes it
//! possible; this gates the outcome.
//!
//! No clock, no sockets: everything is ticks and observable state.

use server::core::ShardCore;
use server::stats::ShardStats;
use sim_core::input::{InputFrame, BTN_PRIMARY, BTN_SPRINT};
use sim_core::limits::TICK_HZ;
use sim_core::mob::{MOB_PIG, MOB_WOLF};
use sim_core::movement::POS_XZ_Q;
use sim_core::world::{Command, DEATH_BY_MOB};

/// The wire yaw whose LUT entry points closest to `(dx, dz)` — the same
/// pick `mob::think` makes, done here off the public `yaw_dir` so the test
/// steers on the sim's own direction space rather than an angle it computed
/// some other way.
fn yaw_toward(dx: f32, dz: f32) -> u16 {
    let (mut best, mut best_dot) = (0u16, f32::NEG_INFINITY);
    for i in 0..256u16 {
        let (ex, ez) = sim_core::yaw_dir(i << 8);
        let dot = ex * dx + ez * dz;
        if dot > best_dot {
            best_dot = dot;
            best = i << 8;
        }
    }
    best
}

/// Ticks a chase may take before the hunt is called a failure — 60 s.
///
/// Generous on purpose. The claim being gated is *possible*, not *quick*:
/// a number tight enough to measure the fun would redden on a rebalance
/// that is nobody's bug, and a chase that takes longer than a minute of
/// open-ground sprinting is not a chase, it is the 100%-flee defect back.
const HUNT_LIMIT_TICKS: u32 = 60 * TICK_HZ;

#[test]
fn a_sprinting_player_can_catch_and_kill_a_pig() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    let stats = ShardStats::default();
    let mut core = ShardCore::new(20260731);
    core.world.mob = c.bake_mobs().expect("animals bake");
    core.world.combat = c.bake_combat().expect("weapons bake");
    core.world.gather = c.bake_gather().expect("gather bakes");
    core.world.spawn_kit = c.bake_spawn_kit().expect("the kit bakes");
    // The bag ladder, because the kill's payout IS a bag now: a killed pig
    // stands its loot up as a ground container at the death position, and
    // a shipped content set that disarmed backpacks would make every hunt
    // pay nothing — this line is where that regression reddens.
    core.world.backpack = c.bake_backpack().expect("the bag ladder bakes");
    core.tick_bare(&stats, |_, _, _| true);

    // **A pig, by name.** The roster holds two species since predator v0
    // and `mob::kind_of` puts a wolf in slot 0, so the `position(|m|
    // m.alive)` this line used to be now hands back the animal that hunts
    // instead of the one that runs — a test about catching prey, quietly
    // measuring a predator that was coming anyway.
    let slot = core
        .world
        .mobs
        .m
        .iter()
        .position(|m| m.alive && m.kind == MOB_PIG)
        .expect("the roster hatched a pig");
    let (mx, mz) = (
        core.world.mobs.m[slot].body.qx as f32 * POS_XZ_Q,
        core.world.mobs.m[slot].body.qz as f32 * POS_XZ_Q,
    );
    // Twelve metres: exactly the fright radius, so the pig is running from
    // the first tick and the chase is the whole test.
    core.world.dev_spawn = Some((mx + 12.0, mz));
    assert!(core.connect(0, 0x100), "connect");
    core.tick_bare(&stats, |_, _, _| true);
    let p = core
        .world
        .players
        .iter()
        .position(|p| p.active)
        .expect("seated");

    // **What the spawn kit actually holds**, found rather than assumed.
    //
    // An earlier cut of this test hunted a pig with a *building plan* for
    // sixty seconds and blamed the animal — the plan has no damage by
    // design (`DECISIONS.md` 2026-08-07, the held-item modal mouse). The
    // next cut put a crafted spear in hand, because at the time
    // `weapons.toml` armed six things and none of them was a tool, so a
    // fresh character genuinely could not hunt. Tools are armed now
    // (2026-08-08), which is what makes this loop reachable from the beach:
    // the assertion is that SOME pocket a fresh character owns can kill,
    // not which one.
    let combat = c.bake_combat().expect("weapons bake");
    let frame_sel = (0..sim_core::limits::HOTBAR_SLOTS as u8)
        .find(|&s| {
            let held = core.world.players[p].inv[s as usize];
            held.count > 0 && combat.held_melee(held.item).is_some()
        })
        .expect(
            "the spawn kit arms a fresh character with nothing that swings — \
             a naked spawn cannot hunt, and the food loop starts at a kill",
        );
    let mut frame = core.world.players[p].frame;
    frame.sel = frame_sel;
    let mut caught = None;
    for t in 0..HUNT_LIMIT_TICKS {
        let (px, pz) = (core.world.players[p].body.qx, core.world.players[p].body.qz);
        let (bx, bz) = (
            core.world.mobs.m[slot].body.qx,
            core.world.mobs.m[slot].body.qz,
        );
        frame.seq = frame.seq.wrapping_add(1);
        frame.yaw = yaw_toward((bx - px) as f32 * POS_XZ_Q, (bz - pz) as f32 * POS_XZ_Q);
        frame.move_z = 127;
        frame.buttons = BTN_SPRINT | BTN_PRIMARY;
        core.world.tick(&[Command::Input {
            id: 0x100,
            frame,
            favour: 0,
        }]);
        if !core.world.mobs.m[slot].alive {
            // (bx, bz) was read before the killing tick and the strike
            // resolves before the roster steps, so it is the death address.
            caught = Some((t, bx, bz));
            break;
        }
    }

    let (t, bx, bz) = caught.unwrap_or_else(|| {
        let (px, pz) = (core.world.players[p].body.qx, core.world.players[p].body.qz);
        let (bx, bz) = (
            core.world.mobs.m[slot].body.qx,
            core.world.mobs.m[slot].body.qz,
        );
        let d = (((bx - px) as f64 * 0.03).powi(2) + ((bz - pz) as f64 * 0.03).powi(2)).sqrt();
        panic!(
            "60 s of sprinting and the pig is still alive at {} hp, {d:.1} m away — \
             this is the flee-speed defect: an animal at or above the player's \
             sprint can never be caught, and every other gate stays green",
            core.world.mobs.m[slot].hp
        )
    });
    println!("caught in {t} ticks ({:.1} s)", t as f32 / TICK_HZ as f32);

    // The kill left a body, not a payment: the loot stands in a ground bag
    // at the death position, and the killer's pockets hold none of it yet.
    let raw = c.item_index("item.raw_meat").expect("raw meat is an item");
    let carried = |core: &ShardCore, item: u16| -> u32 {
        core.world.players[p]
            .inv
            .iter()
            .filter(|s| s.item == item && s.count > 0)
            .map(|s| s.count as u32)
            .sum()
    };
    assert_eq!(
        carried(&core, raw),
        0,
        "the blow itself paid raw meat into the killer — the corpse bag \
         was bypassed"
    );
    assert_eq!(
        core.world.backpacks.len(),
        1,
        "the kill stood up exactly one corpse bag"
    );
    let bag = core.world.backpacks.entries()[0];
    assert_eq!(
        (bag.qx, bag.qz),
        (bx, bz),
        "the bag is where the pig died, not where the killer stood"
    );

    // E — the same loot verb every ground bag answers. The chase ends with
    // the killer on top of the body (melee reach 2 m < loot reach 5 m), so
    // no walk is owed; the take pays exactly the mobs.toml rows.
    core.world.tick(&[Command::Loot { id: 0x100 }]);
    let def = core.world.mob.def(MOB_PIG);
    for row in def.loot.iter() {
        if row.item == sim_core::gather::NO_ITEM || row.count == 0 {
            continue;
        }
        assert_eq!(
            carried(&core, row.item),
            row.count as u32,
            "looting the corpse must pay exactly the content row for item {}",
            row.item
        );
    }
    assert!(carried(&core, raw) > 0, "the hunt paid no raw meat");
    assert!(
        core.world.backpacks.is_empty(),
        "an emptied corpse bag leaves the world"
    );
}

/// **The inverse: a wolf kills a player who does not react.**
///
/// The judge's ranked gap for 2026-08-13 was that the island held one animal
/// and it was prey — so outside a PvP encounter nothing on the map ever
/// chose the player as a target, and night was darker and nothing else. This
/// is that claim measured from the other side, against the *shipped* content
/// rather than a fixture: `content/mobs.toml`'s own wolf, on the real
/// terrain, driving the same `movement::step` a player does.
///
/// The player here does nothing on purpose. Not "plays badly" — sends idle
/// frames, which is what an AFK body or a player in a menu is, and the
/// weakest possible thing to ask of an animal that is supposed to be
/// dangerous. Every earlier version of this roster passed that bar by
/// leaving the player alone.
///
/// No clock and no sockets, like its sibling above: ticks and observable
/// state.
#[test]
fn a_wolf_kills_a_player_who_never_reacts() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    let stats = ShardStats::default();
    let mut core = ShardCore::new(20260731);
    core.world.mob = c.bake_mobs().expect("animals bake");
    core.world.combat = c.bake_combat().expect("weapons bake");
    core.world.spawn_kit = c.bake_spawn_kit().expect("the kit bakes");
    core.world.backpack = c.bake_backpack().expect("the bag ladder bakes");
    core.tick_bare(&stats, |_, _, _| true);

    let slot = core
        .world
        .mobs
        .m
        .iter()
        .position(|m| m.alive && m.kind == MOB_WOLF)
        .expect("the roster hatched a wolf");
    // Every other animal out of the world: this test asserts on the
    // player's hp, and a second animal in bite reach would move the same
    // number. Nothing is retuned — one animal is simply the only one there.
    for (i, m) in core.world.mobs.m.iter_mut().enumerate() {
        if i != slot {
            m.alive = false;
        }
    }
    let (mx, mz) = (
        core.world.mobs.m[slot].body.qx as f32 * POS_XZ_Q,
        core.world.mobs.m[slot].body.qz as f32 * POS_XZ_Q,
    );
    // 25 m: inside the wolf's 30 m notice radius and more than twice the
    // pig's 12 m, so what happens next is the hunt and not a flinch.
    core.world.dev_spawn = Some((mx + 25.0, mz));
    assert!(core.connect(0, 0x100), "connect");
    core.tick_bare(&stats, |_, _, _| true);
    let p = core
        .world
        .players
        .iter()
        .position(|p| p.active)
        .expect("seated");
    let full = core.world.players[p].hp;
    assert!(full > 0, "a fresh spawn has hit points");

    let frame = InputFrame {
        yaw: core.world.players[p].frame.yaw,
        ..InputFrame::default()
    };
    let mut killed = None;
    for t in 0..HUNT_LIMIT_TICKS {
        let mut f = frame;
        f.seq = (t % 65_536) as u16;
        core.world.tick(&[Command::Input {
            id: 0x100,
            frame: f,
            favour: 0,
        }]);
        if core.world.players[p].dead {
            killed = Some(t);
            break;
        }
    }

    let t = killed.unwrap_or_else(|| {
        let (px, pz) = (core.world.players[p].body.qx, core.world.players[p].body.qz);
        let (bx, bz) = (
            core.world.mobs.m[slot].body.qx,
            core.world.mobs.m[slot].body.qz,
        );
        let d = (((bx - px) as f64 * 0.03).powi(2) + ((bz - pz) as f64 * 0.03).powi(2)).sqrt();
        panic!(
            "60 s standing still 25 m from a wolf and the player is alive at {} of {full} hp, \
             {d:.1} m away — the island's only predator does not hunt, which is the whole \
             of what it is for",
            core.world.players[p].hp
        )
    });
    println!("killed in {t} ticks ({:.1} s)", t as f32 / TICK_HZ as f32);
    assert_eq!(
        core.world.players[p].death_cause, DEATH_BY_MOB,
        "the death has to name the animal, not a fall or a bleed"
    );
    assert_ne!(
        core.world.players[p].death_by & sim_core::limits::MOB_ID_TAG,
        0,
        "the killer is the tagged roster id, not a player number"
    );
    // And the wolf is still standing: nothing in this test hurt it, so a
    // dead player here is the animal's doing and not a mutual collapse.
    assert!(
        core.world.mobs.m[slot].alive,
        "the wolf died too — something other than the hunt happened here"
    );
}

/// **The same wolf, the same 25 m, the same shipped content — and the hour
/// decides whether it is an encounter at all.**
///
/// The A/B its sim-core sibling cannot run: `sim-core/tests/mob.rs` proves
/// the radius against `probe_fixture`, and this proves the number in
/// `content/mobs.toml` survives the whole path into a running shard core.
/// Everything is identical between the two runs but `World::tick`'s value,
/// so a difference in outcome has exactly one cause.
///
/// The direction is the reference game's, not the obvious one: it shipped a
/// predator that saw *further* in the dark and removed it, because being
/// hunted by something the player cannot see is an ambush rather than a
/// difficulty (`content/mobs.toml`, `DECISIONS.md` §open "nocturnal
/// senses"). What the player gets is the first tactic the clock sells them.
///
/// **The window is 5 s and that is load-bearing, not timidity.** The first
/// draft of this test stood still for 60 s and asserted the night player
/// survived; it failed, and the failure was the honest kind — an unroused
/// wolf *ambles*, and over a minute this seed's random walk carried it from
/// 25 m to 0.7 m, at which point it could see the player perfectly well and
/// the run had stopped being about the clock. Five seconds is bounded by
/// arithmetic instead: an ambling wolf cannot cross the 10 m between the
/// stand-off and its night radius in that time, and the test asserts the
/// distance it actually kept so a future retune cannot quietly invalidate
/// the margin. What that costs is the kill — `a_wolf_kills_a_player_who
/// _never_reacts` still owns that claim, by day.
///
/// No clock and no sockets, like its siblings: ticks and observable state.
#[test]
fn the_hour_decides_whether_the_wolf_finds_you_at_all() {
    /// Long enough for every slot to think several times over, short enough
    /// that an amble cannot close the 10 m of margin.
    const WINDOW_TICKS: u32 = 5 * TICK_HZ;

    let day = (0..sim_core::limits::DAY_TICKS)
        .find(|&t| !sim_core::world::is_night(t))
        .expect("the cycle has a day");
    let night = (0..sim_core::limits::DAY_TICKS)
        .find(|&t| {
            sim_core::world::is_night(t) && sim_core::world::is_night(t + WINDOW_TICKS as u64)
        })
        .expect("night must be longer than the window");

    let by_day = stand_off_at(day, WINDOW_TICKS);
    let by_night = stand_off_at(night, WINDOW_TICKS);

    assert!(
        by_day.noticed,
        "at noon a wolf 25 m away is inside its 30 m notice radius and must \
         have roused: it stayed {:.1} m off and never did",
        by_day.closest_m
    );
    assert!(
        !by_night.noticed,
        "at midnight the same wolf notices from 15 m and must not have \
         roused at 25 m: it did, closest approach {:.1} m",
        by_night.closest_m
    );
    // The reason has to be the radius and not the animal having gone
    // somewhere: if the night amble ever carried it inside the 15 m it
    // *can* see from, this run no longer isolates the hour and says so
    // rather than passing quietly.
    assert!(
        by_night.closest_m > 15.0,
        "the night wolf came within {:.1} m — inside the radius it notices \
         from, so this window no longer isolates the hour",
        by_night.closest_m
    );
    // And by day it is not merely roused but coming: the rousing has to be
    // a hunt rather than a flag, or this passes on a predator that stands
    // and thinks about it.
    assert!(
        by_day.closest_m < 25.0 - 1.0,
        "a roused wolf must have closed ground: closest {:.1} m of 25",
        by_day.closest_m
    );
}

struct StandOff {
    /// Did the wolf ever rouse inside the window?
    noticed: bool,
    /// Nearest it ever got, metres.
    closest_m: f32,
}

/// One stand-still at 25 m from the island's only wolf, starting at `hour`
/// and lasting `ticks`. The player sends idle frames and never moves.
fn stand_off_at(hour: u64, ticks: u32) -> StandOff {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    let stats = ShardStats::default();
    let mut core = ShardCore::new(20260731);
    core.world.mob = c.bake_mobs().expect("animals bake");
    core.world.combat = c.bake_combat().expect("weapons bake");
    core.world.spawn_kit = c.bake_spawn_kit().expect("the kit bakes");
    core.world.backpack = c.bake_backpack().expect("the bag ladder bakes");
    core.tick_bare(&stats, |_, _, _| true);

    let slot = core
        .world
        .mobs
        .m
        .iter()
        .position(|m| m.alive && m.kind == MOB_WOLF)
        .expect("the roster hatched a wolf");
    // Every other animal out of the world: a second one in reach would
    // rouse against the same player and muddy which animal answered.
    for (i, m) in core.world.mobs.m.iter_mut().enumerate() {
        if i != slot {
            m.alive = false;
        }
    }
    let (mx, mz) = (
        core.world.mobs.m[slot].body.qx as f32 * POS_XZ_Q,
        core.world.mobs.m[slot].body.qz as f32 * POS_XZ_Q,
    );
    core.world.dev_spawn = Some((mx + 25.0, mz));
    assert!(core.connect(0, 0x100), "connect");
    core.tick_bare(&stats, |_, _, _| true);
    let p = core
        .world
        .players
        .iter()
        .position(|p| p.active)
        .expect("seated");

    // Set the hour only now: both runs are identical up to this line and
    // differ in nothing afterwards but the clock they keep.
    core.world.tick = hour;

    let frame = InputFrame {
        yaw: core.world.players[p].frame.yaw,
        ..InputFrame::default()
    };
    let mut noticed = false;
    let mut closest_m = f32::MAX;
    for t in 0..ticks {
        let mut f = frame;
        f.seq = (t % 65_536) as u16;
        core.world.tick(&[Command::Input {
            id: 0x100,
            frame: f,
            favour: 0,
        }]);
        let (px, pz) = (core.world.players[p].body.qx, core.world.players[p].body.qz);
        let (bx, bz) = (
            core.world.mobs.m[slot].body.qx,
            core.world.mobs.m[slot].body.qz,
        );
        let dx = (bx - px) as f32 * POS_XZ_Q;
        let dz = (bz - pz) as f32 * POS_XZ_Q;
        closest_m = closest_m.min((dx * dx + dz * dz).sqrt());
        noticed |= core.world.mobs.m[slot].roused_until > core.world.tick;
    }
    StandOff { noticed, closest_m }
}
