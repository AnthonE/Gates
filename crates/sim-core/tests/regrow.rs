//! Trees regrow from saplings (tree growth v0): the sapling grows in its
//! steps and never shrinks, a young tree pays and falls at its own size, a
//! body walks past a sapling the tree would have stopped, and a growing
//! tree survives a restart. The store's own life cycle is `gather.rs`'s
//! unit test.

use sim_core::gather::{
    grow_pm, GatherContent, GROW_STAGES, SAPLING_PM, SWING_INTERVAL_TICKS, TREE_GROW_TICKS,
};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::occupy::{Harvested, Scratch};
use sim_core::terrain::{self, Occupant, ScatterTable, Slot, CELL_SIZE};
use sim_core::world::{Command, World, EV_GATHER, EV_SLOT_HARVESTED, EV_SLOT_RESPAWNED};
use sim_core::worldsave::WORLD_SAVE_MAX_BYTES;
use sim_core::yaw_dir;

const SEED: u64 = 20_260_731;
/// Low on the trunk, so the same aim reaches a sapling and a tree.
const AIM_UP_M: f32 = 0.3;
const EYE_M: f32 = sim_core::ranged::ARROW_EYE_MM as f32 / 1000.0;

#[derive(Clone, Copy)]
struct Aim {
    yaw: u16,
    pitch: u8,
}

/// The pitch byte closest to `(run, rise)`, off the sim's own LUT.
fn pitch_toward(rise: f32, run: f32) -> u8 {
    let len = (rise * rise + run * run).sqrt();
    let mut best = (128u8, f32::MIN);
    for b in 0..=255u8 {
        let (ch, sv) = sim_core::pitch_dir(b);
        let dot = (ch * run + sv * rise) / len;
        if dot > best.1 {
            best = (b, dot);
        }
    }
    best.0
}

/// A tree with no other node within reach of the point 1.2 m west of it:
/// the tree, that point, and the aim at the trunk from there.
fn lone_tree() -> (Slot, (f32, f32), Aim) {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(SEED);
    for cz in 40..216i32 {
        for cx in 40..216i32 {
            let s = terrain::scatter(SEED, &table, &haven, cx, cz);
            if s.occupant != Occupant::Tree {
                continue;
            }
            let (px, pz) = (s.x - 1.2, s.z);
            let py = terrain::height(SEED, px, pz);
            if (s.y - py).max(py - s.y) > 1.0 || py < 1.0 {
                continue;
            }
            let (pcx, pcz) = ((px / CELL_SIZE) as i32, (pz / CELL_SIZE) as i32);
            let rival = (-1..=1).any(|dz| {
                (-1..=1).any(|dx| {
                    let n = terrain::scatter(SEED, &table, &haven, pcx + dx, pcz + dz);
                    let d2 = (n.x - px) * (n.x - px) + (n.z - pz) * (n.z - pz);
                    n.occupant != Occupant::None && d2 <= 6.25 && (n.x != s.x || n.z != s.z)
                })
            });
            if rival {
                continue;
            }
            // The tree is due east of the stand point.
            let yaw = (0..=255u16)
                .map(|hi| hi << 8)
                .max_by(|&a, &b| yaw_dir(a).0.total_cmp(&yaw_dir(b).0))
                .unwrap();
            let pitch = pitch_toward(s.y + AIM_UP_M - (py + EYE_M), 1.2);
            return (s, (px, pz), Aim { yaw, pitch });
        }
    }
    panic!("no lone tree on seed {SEED:#x}");
}

fn fixture() -> Box<World> {
    let mut w = Box::new(World::new(SEED));
    w.gather = GatherContent::probe_fixture();
    // The weak spot's bonus would move the pay this suite counts exactly.
    for n in w.gather.nodes.iter_mut() {
        n.weak_pct = 0;
    }
    w
}

fn world_at(pos: (f32, f32)) -> Box<World> {
    let mut w = fixture();
    w.dev_spawn = Some(pos);
    w.tick(&[Command::Join { id: 1 }]);
    w
}

fn input(aim: Aim, seq: u64, buttons: u8) -> Command {
    Command::Input {
        id: 1,
        frame: InputFrame {
            seq: seq as u16,
            buttons,
            yaw: aim.yaw,
            pitch: aim.pitch,
            move_x: 0,
            move_z: 0,
            sel: 0,
        },
        favour: 0,
    }
}

/// Swing until the node falls, then let go: (paying swings, wood paid).
fn chop(w: &mut World, aim: Aim) -> (u32, u32) {
    let (mut swings, mut paid) = (0, 0);
    for seq in 0..SWING_INTERVAL_TICKS * 12 {
        w.tick(&[input(aim, seq, BTN_PRIMARY)]);
        let mut felled = false;
        for e in w.events.entries() {
            match e.code {
                EV_GATHER => {
                    swings += 1;
                    paid += e.b & 0xFFFF;
                }
                EV_SLOT_HARVESTED => felled = true,
                _ => {}
            }
        }
        if felled {
            w.tick(&[input(aim, seq + 1, 0)]);
            return (swings, paid);
        }
    }
    panic!("the tree never fell");
}

/// Leap to the felled tree's timer and let it sprout: the grown-by tick.
fn sprout(w: &mut World, cx: u16, cz: u16) -> u64 {
    let due = w.slot_lives.find(cx, cz).expect("felled").respawn_at;
    w.tick = due - 1;
    for _ in 0..3 {
        w.tick(&[]);
        if let Some(e) = w
            .events
            .entries()
            .iter()
            .find(|e| e.code == EV_SLOT_RESPAWNED)
        {
            assert_eq!(e.c, 1, "a tree comes back as a sapling");
            return w.slot_lives.find(cx, cz).expect("growing").grown_at;
        }
    }
    panic!("the stump never sprouted");
}

fn cell(s: &Slot) -> (u16, u16) {
    ((s.x / CELL_SIZE) as u16, (s.z / CELL_SIZE) as u16)
}

#[test]
fn a_sapling_grows_in_its_steps_and_never_shrinks() {
    let grown_at = 1_000_000;
    let sprouted = grown_at - TREE_GROW_TICKS;
    assert_eq!(grow_pm(grown_at, sprouted), SAPLING_PM);
    assert_eq!(grow_pm(grown_at, grown_at), 1000);
    assert_eq!(grow_pm(0, 5), 1000, "a tree that is not growing is whole");
    let (mut last, mut steps) = (0, 0);
    for now in sprouted..grown_at {
        let pm = grow_pm(grown_at, now);
        assert!(pm >= last && pm < 1000, "tick {now}: {last} then {pm}");
        if pm != last {
            steps += 1;
            last = pm;
        }
    }
    assert_eq!(steps, GROW_STAGES);
}

#[test]
fn a_young_tree_pays_and_falls_at_its_own_size() {
    let (slot, pos, aim) = lone_tree();
    let (cx, cz) = cell(&slot);
    let mut w = world_at(pos);
    let tree = w.gather.nodes[0];
    assert_eq!(
        chop(&mut w, aim),
        (tree.hits as u32, (tree.hits * tree.hand_yield) as u32),
        "a grown tree pays its whole row"
    );
    sprout(&mut w, cx, cz);

    // Fresh, it is 15% of the tree: one swing, one swing's wood.
    assert_eq!(w.slot_lives.standing_pm(cx, cz), SAPLING_PM);
    let hits = (tree.hits as u32 * SAPLING_PM as u32).div_ceil(1000);
    assert_eq!(chop(&mut w, aim), (hits, hits * tree.hand_yield as u32));

    // Half grown, half the swings and half the wood, rounded up.
    let grown_at = sprout(&mut w, cx, cz);
    w.tick = grown_at - TREE_GROW_TICKS / 2;
    w.tick(&[]);
    let pm = w.slot_lives.standing_pm(cx, cz) as u32;
    assert!(pm > 500 && pm < 1000, "{pm}");
    let hits = (tree.hits as u32 * pm).div_ceil(1000);
    assert!(hits > 1 && hits < tree.hits as u32);
    assert_eq!(chop(&mut w, aim), (hits, hits * tree.hand_yield as u32));
}

/// One sapling at `pm`, everything else untouched.
struct OneSapling {
    at: (u16, u16),
    pm: u16,
}

impl Harvested for OneSapling {
    fn is_harvested(&self, cx: u16, cz: u16) -> bool {
        (cx, cz) == self.at && self.pm == 0
    }

    fn standing_pm(&self, cx: u16, cz: u16) -> u16 {
        if (cx, cz) == self.at {
            self.pm
        } else {
            1000
        }
    }
}

#[test]
fn a_body_walks_past_a_sapling_the_tree_would_stop() {
    let (slot, ..) = lone_tree();
    let at = cell(&slot);
    let blocked = |pm: u16, off: f32| {
        let mut scratch = Scratch::with(SEED, OneSapling { at, pm });
        [(off, 0.0), (-off, 0.0), (0.0, off), (0.0, -off)].map(|(dx, dz)| {
            scratch
                .occupants()
                .blocks(SEED, slot.x + dx, slot.z + dz, slot.y)
        })
    };
    assert_eq!(blocked(0, 0.5), [false; 4], "something else stands here");
    assert_eq!(blocked(1000, 0.5), [true; 4], "the tree stops a body");
    assert_eq!(blocked(SAPLING_PM, 0.5), [false; 4], "a sapling does not");
    assert_eq!(blocked(SAPLING_PM, 0.3), [true; 4], "but it is not a ghost");
}

#[test]
fn a_growing_tree_survives_a_restart() {
    let (slot, pos, aim) = lone_tree();
    let (cx, cz) = cell(&slot);
    let mut w = world_at(pos);
    chop(&mut w, aim);
    let grown_at = sprout(&mut w, cx, cz);
    w.tick(&[Command::Leave { id: 1 }]);
    let mut buf = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("encodes");
    let mut back = fixture();
    back.load(&buf[..n]).expect("loads");
    assert_eq!(back.state_hash(), w.state_hash());
    assert_eq!(
        back.slot_lives.find(cx, cz).map(|e| e.grown_at),
        Some(grown_at)
    );
}
