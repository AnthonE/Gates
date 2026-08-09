//! `hunt` — a naked spawn can actually catch and kill a pig, and the kill
//! leaves a corpse bag whose loot is a raw food a campfire will take.
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
use sim_core::input::{BTN_PRIMARY, BTN_SPRINT};
use sim_core::limits::TICK_HZ;
use sim_core::movement::POS_XZ_Q;
use sim_core::world::Command;

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
    core.tick(&stats, |_, _, _| true);

    let slot = core
        .world
        .mobs
        .m
        .iter()
        .position(|m| m.alive)
        .expect("the roster hatched");
    let (mx, mz) = (
        core.world.mobs.m[slot].body.qx as f32 * POS_XZ_Q,
        core.world.mobs.m[slot].body.qz as f32 * POS_XZ_Q,
    );
    // Twelve metres: exactly the fright radius, so the pig is running from
    // the first tick and the chase is the whole test.
    core.world.dev_spawn = Some((mx + 12.0, mz));
    assert!(core.connect(0, 0x100), "connect");
    core.tick(&stats, |_, _, _| true);
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
        core.world.tick(&[Command::Input { id: 0x100, frame }]);
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
    let def = core.world.mob.def(0);
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
