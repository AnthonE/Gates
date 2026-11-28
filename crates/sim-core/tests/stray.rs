//! The strays (`mob::MOB_STRAY`, `ARC.md` §2 act III): nothing stands up
//! before THE SEVERED GATE opens; once it does they come out at the
//! landmarks' vault doors, never more than the roster's block holds, and a
//! hammer on a wall going up draws one in from much further than a wolf
//! hears it.
//!
//! Numbers come from `MobContent::probe_fixture`, whose stray row is the
//! shipped one's leash and ear.

use sim_core::brain::AiState;
use sim_core::build::{BuildContent, LOC_PLANE};
use sim_core::combat::CombatContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::InputFrame;
use sim_core::limits::{MAX_MOBS, MAX_STRAYS, MOB_THINK_TICKS};
use sim_core::mob::{self, MobContent, MOB_STRAY};
use sim_core::movement::{quant_xz, Body, POS_XZ_Q};
use sim_core::terrain::Occupant;
use sim_core::world::{Command, World, EV_PIECE_PLACED};

const SEED: u64 = 11;

/// Light a work past act II: THE SEVERED GATE is open, and its effect
/// rouses the island to `pct` per cent (`works::KNOB_NOTICE_PCT`).
fn open_gate(w: &mut World, pct: u16) {
    use sim_core::works::{ArcEffect, WorksContent, KNOB_NOTICE_PCT, WORK_LIT};
    let mut wc = WorksContent::EMPTY;
    wc.count = 1;
    wc.defs[0].act = 3;
    wc.defs[0].floor = 1;
    wc.n_effects = 1;
    wc.effects[0] = ArcEffect {
        unlock: 1,
        knob: KNOB_NOTICE_PCT,
        pct,
    };
    w.works_def = wc;
    w.works.w[0].state = WORK_LIT;
    w.works.refresh(&w.works_def);
    assert!(w.works.gate_open(&w.works_def), "the Gate is not open");
}

/// The roaming strays: the stray block less the lab's dwellers, who keep
/// the lab (`tests/lab.rs`).
fn strays(w: &World, f: impl Fn(&mob::Mob) -> bool) -> usize {
    w.mobs
        .m
        .iter()
        .enumerate()
        .filter(|(s, m)| m.kind == MOB_STRAY && !mob::is_lab_slot(*s) && f(m))
        .count()
}

/// Every vault door on the island and the way it faces.
fn doors(w: &World) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    for m in w.haven.marks.iter().filter(|m| m.live) {
        let mut k = 0;
        while let Some((x, z, _, occ)) = sim_core::landmark::anchor(m, k) {
            k += 1;
            if occ == Occupant::Vault {
                out.push((x, z));
            }
        }
    }
    out
}

/// **Sealed until the Gate opens.** A world runs a minute with the strays'
/// slots enrolled and not one stands up; the tick the Gate opens every
/// enrolled slot comes out, each a few metres from a landmark's vault door,
/// all of them inside the roster's block.
#[test]
fn no_stray_stands_up_until_the_gate_opens_and_then_at_a_vault_door() {
    let mut w = World::new(SEED);
    w.mob = MobContent::probe_fixture();
    for _ in 0..60 * sim_core::limits::TICK_HZ as u64 {
        w.tick(&[]);
        assert_eq!(strays(&w, |m| m.alive), 0, "a stray before the Gate");
    }
    let homed = strays(&w, |m| m.homed);
    assert!(homed > 0, "the fixture enrols no stray at all");
    assert!(homed <= MAX_STRAYS);
    assert_eq!(mob::capacity(MOB_STRAY), MAX_STRAYS);
    assert!((0..MAX_MOBS)
        .filter(|&s| mob::kind_of(s) == MOB_STRAY)
        .all(mob::is_stray_slot));

    open_gate(&mut w, 150);
    w.tick(&[]);
    assert_eq!(
        strays(&w, |m| m.alive),
        homed,
        "the Gate opened and none came out"
    );
    let doors = doors(&w);
    assert!(!doors.is_empty());
    for (_, m) in w
        .mobs
        .m
        .iter()
        .enumerate()
        .filter(|(s, m)| m.alive && m.kind == MOB_STRAY && !mob::is_lab_slot(*s))
    {
        let (x, z) = (m.home_qx as f32 * POS_XZ_Q, m.home_qz as f32 * POS_XZ_Q);
        let near = doors
            .iter()
            .map(|&(dx, dz)| ((x - dx) * (x - dx) + (z - dz) * (z - dz)).sqrt())
            .fold(f32::MAX, f32::min);
        assert!(
            near <= 8.5,
            "a stray came out {near:.1} m from the nearest vault door"
        );
    }

    // With no Gate (no work past act II), a dead stray stays dead.
    let slot = (0..MAX_MOBS)
        .find(|&s| w.mobs.m[s].alive && w.mobs.m[s].kind == MOB_STRAY && !mob::is_lab_slot(s))
        .unwrap();
    w.works_def = sim_core::works::WorksContent::EMPTY;
    w.works.refresh(&w.works_def);
    assert!(!w.works.gate_open(&w.works_def));
    w.mobs.m[slot].alive = false;
    w.mobs.m[slot].respawn_at = w.tick;
    for _ in 0..90 {
        w.tick(&[]);
    }
    assert!(!w.mobs.m[slot].alive, "a stray came out of a shut Gate");
}

/// **The cap holds.** A row asking for fifty per km² enrols the block and no
/// more, and killing them over and over never stands up a thirteenth.
#[test]
fn the_strays_never_pass_their_block() {
    let mut mc = MobContent::probe_fixture();
    mc.defs[MOB_STRAY as usize].per_km2_milli = 50_000;
    mc.defs[MOB_STRAY as usize].respawn_ticks = 1;
    let mut w = World::new(SEED);
    w.mob = mc;
    open_gate(&mut w, 100);
    for t in 0..600u64 {
        w.tick(&[]);
        assert!(strays(&w, |m| m.homed) <= MAX_STRAYS);
        assert!(strays(&w, |m| m.alive) <= MAX_STRAYS);
        if t % 7 == 0 {
            for m in w.mobs.m.iter_mut().filter(|m| m.kind == MOB_STRAY) {
                m.alive = false;
                m.respawn_at = w.tick + 1;
            }
        }
    }
    assert_eq!(strays(&w, |m| m.homed), MAX_STRAYS);
    // Enrolment past the cap would have taken another species' slot.
    assert!(w
        .mobs
        .m
        .iter()
        .enumerate()
        .all(|(s, m)| (m.kind == MOB_STRAY) == mob::is_stray_slot(s)));
}

/// A cell a foundation can go on, and a bearing from its middle over dry
/// ground nothing stands on for `r` metres (`tests/brain.rs`'s search).
fn build_site(r: f32) -> ((u16, u16), (f32, f32)) {
    use sim_core::build::{foundation_terrain_ok, BUILD_CELL_M};
    let h = sim_core::terrain::haven(SEED);
    let mut scratch = sim_core::occupy::Scratch::live(SEED);
    let mut occ = scratch.occupants();
    let (hx, hz) = sim_core::find::build_hint(SEED, 512, 512);
    for ring in 0..64i32 {
        for dz in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs() != ring && dz.abs() != ring {
                    continue;
                }
                let (cx, cz) = ((hx + dx) as u16, (hz + dz) as u16);
                let at = (
                    (cx as f32 + 0.5) * BUILD_CELL_M,
                    (cz as f32 + 0.5) * BUILD_CELL_M,
                );
                if !foundation_terrain_ok(SEED, &h, at.0, at.1) {
                    continue;
                }
                let open = (0..16u16)
                    .map(|k| sim_core::yaw_dir(k << 12))
                    .find(|&(bx, bz)| {
                        (1..=r as u32).all(|i| {
                            let (x, z) = (at.0 + bx * i as f32, at.1 + bz * i as f32);
                            let feet = sim_core::terrain::ground(SEED, &h, x, z);
                            sim_core::terrain::height(SEED, x, z) > sim_core::terrain::BEACH_MAX_H
                                && !occ.blocks(SEED, x, z, feet)
                        })
                    });
                if let Some(b) = open {
                    return ((cx, cz), b);
                }
            }
        }
    }
    panic!("no buildable cell with open ground round it");
}

/// **A hammer draws a stray in.** A player stands at a buildable cell with
/// the Gate open; a stray stands 50 m off, which is past what any wolf
/// hears a hammer at even roused (25 m × 1.5) and inside the stray's
/// (60 m × 1.5). It sees no further than 5 m here, so whatever it does it
/// heard. With a foundation going up it goes to look and closes most of the
/// gap; with nobody building it never looks.
#[test]
fn a_stray_walks_to_a_hammer_strike() {
    const R: f32 = 50.0;
    let ((cx, cz), (bx, bz)) = build_site(R);
    let at = sim_core::build::anchor(cx, cz, LOC_PLANE);
    let mut mc = MobContent::probe_fixture();
    mc.defs[MOB_STRAY as usize].spook_cm = 500;
    mc.defs[MOB_STRAY as usize].night_spook_cm = 500;

    let run = |build: bool| -> (bool, f32) {
        let mut w = World::new(SEED);
        w.env = sim_core::weather::Env::CLEAR;
        w.combat = CombatContent::probe_fixture();
        w.gather = GatherContent::probe_fixture();
        w.build = BuildContent::probe_fixture();
        w.mob = mc;
        open_gate(&mut w, 150);
        w.dev_spawn = Some(at);
        w.tick(&[Command::Join { id: 1 }]);
        let slot = (0..MAX_MOBS)
            .find(|&s| w.mobs.m[s].alive && w.mobs.m[s].kind == MOB_STRAY && !mob::is_lab_slot(s))
            .expect("the open Gate stood up no stray");
        for (i, m) in w.mobs.m.iter_mut().enumerate() {
            if i != slot {
                m.alive = false;
                m.respawn_at = u64::MAX;
            }
        }
        let m = &mut w.mobs.m[slot];
        m.body = Body::at(SEED, &w.haven, at.0 + bx * R, at.1 + bz * R);
        m.home_qx = m.body.qx;
        m.home_qz = m.body.qz;
        m.path.clear();
        m.state = AiState::Idle;
        m.state_until = u64::MAX;
        m.gait = 0;
        m.last_qx = m.body.qx;
        m.last_qz = m.body.qz;
        w.players[0].inv[0] = ItemStack {
            item: 0,
            count: 50,
            ..ItemStack::EMPTY
        };
        if build {
            w.tick(&[Command::Place {
                id: 1,
                row: 0,
                cx,
                cz,
                level: 0,
                loc: LOC_PLANE,
                freehand: false,
                plate: 0,
            }]);
            assert!(
                w.events.entries().iter().any(|e| e.code == EV_PIECE_PLACED),
                "the foundation did not go up"
            );
        }
        let base = (quant_xz(at.0), quant_xz(at.1));
        let gap = |w: &World| {
            let b = w.mobs.m[slot].body;
            let (dx, dz) = ((b.qx - base.0) as f32, (b.qz - base.1) as f32);
            (dx * dx + dz * dz).sqrt() * POS_XZ_Q
        };
        let mut looked = false;
        for seq in 0..40 * MOB_THINK_TICKS as u16 {
            w.tick(&[Command::Input {
                id: 1,
                frame: InputFrame {
                    seq,
                    ..InputFrame::default()
                },
                favour: 0,
            }]);
            looked |= w.mobs.m[slot].state == AiState::MoveTowards;
        }
        (looked, gap(&w))
    };
    let (looked, gap) = run(false);
    assert!(!looked, "nobody built and the stray went to look");
    assert!(
        gap > R * 0.6,
        "nobody built and the stray came in to {gap:.1} m"
    );
    let (looked, gap) = run(true);
    assert!(looked, "the stray did not go to look at the hammer");
    assert!(
        gap < R * 0.4,
        "the stray heard the hammer and only came in to {gap:.1} m"
    );
}

/// **Deterministic.** Two worlds with the Gate open run the strays' hatch,
/// roam and think alike, tick for tick.
#[test]
fn two_islands_with_the_gate_open_agree() {
    let make = || {
        let mut w = World::new(SEED);
        w.mob = MobContent::probe_fixture();
        open_gate(&mut w, 150);
        w.dev_spawn = Some(w.spawn_pos(1));
        w.tick(&[Command::Join { id: 1 }]);
        w
    };
    let (mut a, mut b) = (make(), make());
    for _ in 0..900 {
        a.tick(&[]);
        b.tick(&[]);
        assert_eq!(a.state_hash(), b.state_hash());
    }
    assert!(strays(&a, |m| m.alive) > 0);
}
