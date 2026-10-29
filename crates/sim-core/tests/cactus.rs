//! The desert's cactus (`terrain::Occupant::Cactus`): drawn only out of a
//! desert shrub, solid, and it pricks a body pressed against it once a second
//! (`World::prick_cacti`) while one standing clear of it is left alone.

use sim_core::collide::CAPSULE_RADIUS_M;
use sim_core::gather::GatherContent;
use sim_core::terrain::{self, Biome, Occupant, ScatterTable, Slot, CELLS_PER_SIDE};
use sim_core::world::{Command, World, CACTUS_HP, CACTUS_PERIOD_TICKS, EV_HURT};

const SEED: u64 = 0x0047_4154_4553; // the golden seed

/// Every cactus on a stride over the island, with the biome it stands in.
fn cacti(seed: u64) -> Vec<(Slot, Biome)> {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    let mut out = Vec::new();
    for cz in (0..CELLS_PER_SIDE).step_by(2) {
        for cx in (0..CELLS_PER_SIDE).step_by(2) {
            let s = terrain::scatter(seed, &table, &haven, cx, cz);
            if s.occupant == Occupant::Cactus {
                let b = terrain::biome(
                    s.y,
                    terrain::moisture(seed, s.x, s.z),
                    terrain::fel(seed, s.x, s.z),
                    terrain::cold(seed, s.x, s.z, s.y),
                );
                out.push((s, b));
            }
        }
    }
    out
}

#[test]
fn cacti_stand_in_the_desert_and_nowhere_else() {
    let found = cacti(SEED);
    assert!(
        found.len() > 20,
        "only {} cacti on the golden seed's stride — the desert grows none",
        found.len()
    );
    for (s, b) in &found {
        assert_eq!(*b, Biome::Desert, "a cactus at ({}, {}) in {b:?}", s.x, s.z);
    }
}

/// A body against the trunk loses `CACTUS_HP` a second; the same body two
/// metres off loses nothing.
#[test]
fn a_cactus_pricks_whoever_is_pressed_against_it() {
    let (s, _) = *cacti(SEED)
        .iter()
        .find(|(s, _)| {
            // Flat ground on the west side, so the spawn stands where it is put.
            let d = terrain::height(SEED, s.x - 1.0, s.z) - s.y;
            d * d < 0.09
        })
        .expect("a cactus on flat ground");
    let touching = CAPSULE_RADIUS_M + terrain::CACTUS_R_M * s.scale + 0.02;
    for (gap, pricked) in [(touching, true), (touching + 2.0, false)] {
        let mut w = Box::new(World::new(SEED));
        w.gather = GatherContent::probe_fixture();
        w.dev_spawn = Some((s.x - gap, s.z));
        w.tick(&[Command::Join { id: 1 }]);
        // The bare world loads no combat content, so no health to lose:
        // give the body a naked player's, outside any safe zone.
        (w.players[0].hp, w.players[0].hp_max) = (100, 100);
        w.players[0].safe = false;
        let hp = w.players[0].hp;
        let mut hurt = 0;
        for _ in 0..CACTUS_PERIOD_TICKS * 2 {
            w.tick(&[]);
            hurt += w
                .events
                .entries()
                .iter()
                .filter(|e| e.code == EV_HURT && e.a == 1)
                .count();
        }
        if pricked {
            assert!(
                hurt >= 1,
                "pressed against a cactus for two seconds, unhurt"
            );
            assert!(
                w.players[0].hp <= hp - CACTUS_HP,
                "hp {} → {}: the prick did not land",
                hp,
                w.players[0].hp
            );
        } else {
            assert_eq!(hurt, 0, "pricked from {gap:.2} m off");
            assert_eq!(w.players[0].hp, hp);
        }
    }
}

/// A body joined on the golden seed, walked by held input.
struct Walker {
    w: Box<World>,
    seq: u16,
}

impl Walker {
    fn join(x: f32, z: f32) -> Self {
        let mut w = Box::new(World::new(SEED));
        w.gather = GatherContent::probe_fixture();
        w.dev_spawn = Some((x, z));
        w.tick(&[Command::Join { id: 1 }]);
        Self { w, seq: 0 }
    }

    fn walk(&mut self, yaw: u16, ticks: u32) {
        use sim_core::input::InputFrame;
        for _ in 0..ticks {
            self.seq = self.seq.wrapping_add(1);
            let frame = InputFrame {
                seq: self.seq,
                yaw,
                move_z: 127,
                ..Default::default()
            };
            self.w.tick(&[Command::Input {
                id: 1,
                frame,
                favour: 0,
            }]);
        }
    }

    fn pos(&self) -> (f32, f32) {
        let b = &self.w.players[0].body;
        (
            b.qx as f32 * sim_core::movement::POS_XZ_Q,
            b.qz as f32 * sim_core::movement::POS_XZ_Q,
        )
    }

    /// The quarter-turn that walks along the cardinal (`ux`, `uz`): the yaw
    /// table is the sim's own, so it is found by trying the four.
    fn toward(&mut self, (ux, uz): (f32, f32)) -> u16 {
        for q in 0..4u16 {
            let a = self.pos();
            self.walk(q << 14, 2);
            let b = self.pos();
            self.walk((q << 14).wrapping_add(0x8000), 2);
            let (dx, dz) = (b.0 - a.0, b.1 - a.1);
            let side = dx * uz - dz * ux;
            if dx * ux + dz * uz > 0.1 && side * side < 0.0025 {
                return q << 14;
            }
        }
        panic!("no quarter-turn walks along ({ux}, {uz})");
    }
}

/// Whether the ground is flat 3 m either way of (`x`, `z`) along a cardinal.
fn flat_along(s: &Slot, (ux, uz): (f32, f32)) -> bool {
    [-3.0f32, -1.5, 1.5, 3.0].iter().all(|d| {
        let h = terrain::height(SEED, s.x + ux * d, s.z + uz * d) - s.y;
        h * h < 0.04
    })
}

/// Walked into near head-on, a round cactus is slid round, not stood
/// against: both axis slides fail on a round volume, and the body used to
/// stop dead there while it pricked them. The ball cactus, because the
/// saguaro's arms (below) stop a body as a wall does.
#[test]
fn walking_into_a_cactus_slides_round_it() {
    let (s, _) = *cacti(SEED)
        .iter()
        .find(|(s, _)| s.species == terrain::BALL_CACTUS_SPECIES && flat_along(s, (1.0, 0.0)))
        .expect("a ball cactus on flat ground");
    let mut b = Walker::join(s.x - 2.5, s.z + 0.2);
    let east = b.toward((1.0, 0.0));
    b.walk(east, 120);
    let (x, _) = b.pos();
    assert!(
        x > s.x + terrain::BALL_CACTUS_R_M * s.scale,
        "stopped at x {x:.2} against the cactus at {:.2}",
        s.x
    );
}

/// Walked straight at a saguaro's low arm, a body stops at the arm's tip:
/// it used to walk through both arms to the trunk.
#[test]
fn walking_into_a_saguaros_arm_stops_at_it() {
    // A saguaro whose yaw is within 4° of a quarter-turn, and the cardinal
    // its low arm (local −X, (−cos, sin) of the yaw) then points along.
    let arm = |s: &Slot| {
        let off = s.yaw.wrapping_add(3) % 64;
        let q = s.yaw.wrapping_add(32) / 64;
        (off <= 6).then_some([(-1.0f32, 0.0f32), (0.0, 1.0), (1.0, 0.0), (0.0, -1.0)][q as usize])
    };
    let (s, _) = *cacti(SEED)
        .iter()
        .find(|(s, _)| {
            s.species != terrain::BALL_CACTUS_SPECIES && arm(s).is_some_and(|u| flat_along(s, u))
        })
        .expect("a saguaro on flat ground with its arm along a cardinal");
    let (ux, uz) = arm(&s).unwrap();
    let mut b = Walker::join(s.x + ux * 2.5, s.z + uz * 2.5);
    let yaw = b.toward((-ux, -uz));
    b.walk(yaw, 120);
    let (x, z) = b.pos();
    let d = (x - s.x) * ux + (z - s.z) * uz;
    assert!(
        d > 0.65 * s.scale + CAPSULE_RADIUS_M,
        "walked {d:.2} m from the trunk, into the arm"
    );
    assert!(d < 1.5, "stopped {d:.2} m off, short of the arm");
}

/// A saguaro stops a standing body at its arms, not only at its trunk:
/// the arms are at eye height and a body walked through them. Across the
/// arms the trunk is all there is, and a crouch passes under the low one.
#[test]
fn a_saguaros_arms_are_solid() {
    use sim_core::collide::{CAPSULE_HEIGHT_M, CROUCH_HEIGHT_M};
    let r = CAPSULE_RADIUS_M;
    // Local −X (the low arm) and local +Z, in the world, at the slot's yaw
    // (`terrain::SAGUARO_ARM_BOXES`: local +X is (cos, −sin)).
    for (yaw, arm, across) in [
        (0u8, (-1.0f32, 0.0f32), (0.0f32, 1.0f32)),
        (64, (0.0, 1.0), (1.0, 0.0)),
    ] {
        let s = Slot {
            occupant: Occupant::Cactus,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            yaw,
            scale: 1.0,
            species: 0,
        };
        let blocks = |d: f32, (ux, uz): (f32, f32), h: f32| {
            terrain::slot_blocks(&s, ux * d, uz * d, 0.0, r, h)
        };
        assert!(
            blocks(0.65 + r, arm, CAPSULE_HEIGHT_M),
            "yaw {yaw}: walked into the arm"
        );
        assert!(
            !blocks(0.72 + r, arm, CAPSULE_HEIGHT_M),
            "yaw {yaw}: stopped past the arm"
        );
        assert!(
            !blocks(0.65 + r, arm, CROUCH_HEIGHT_M),
            "yaw {yaw}: a crouch under the arm"
        );
        assert!(
            !blocks(0.65 + r, across, CAPSULE_HEIGHT_M),
            "yaw {yaw}: stopped across the arms"
        );
        assert!(blocks(
            terrain::CACTUS_R_M + r - 0.02,
            across,
            CAPSULE_HEIGHT_M
        ));
    }
}
