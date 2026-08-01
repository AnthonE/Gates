//! `test_replay` (DESIGN.md §7/§12): same build + same seed + same command
//! log → the same state hashes, every stamp. Until the server's WAL exists
//! this drives a deterministic in-memory command script — the contract is
//! identical, the fixture just isn't a file yet. The final hash is also
//! pinned: any accidental drift in sim behavior reddens this gate.

use sim_core::bots::bot_frame;
use sim_core::build::BuildContent;
use sim_core::craft::CraftContent;
use sim_core::gather::GatherContent;
use sim_core::limits::STATE_HASH_INTERVAL;
use sim_core::rng::Pcg32;
use sim_core::world::{Command, World};

const SEED: u64 = 0x0047_4154_4553; // "GATES"
const TICKS: u64 = 900;

/// Pinned end-state hash for (SEED, the script below). Regenerates only
/// with an intentional sim change, in the same commit (CLAUDE.md wall 5).
/// Regenerated this commit: door locks landed — deploy records carry (and
/// hash) a lock bit, doors place locked to their placer, and this script
/// now locks and unlocks the door it was already toggling.
const GOLDEN_FINAL_HASH: u64 = 0x754A_E9E2_1A7C_8EA4;

fn run(seed: u64) -> (Vec<u64>, u64) {
    let mut world = World::new(seed);
    world.gather = GatherContent::probe_fixture();
    world.craft = CraftContent::probe_fixture();
    world.build = BuildContent::probe_fixture();
    world.deploy = sim_core::deploy::DeployContent::probe_fixture();
    let mut rng = Pcg32::new(seed, 11);
    let mut yaws = [0u16; 64];
    let mut joined: u32 = 0;
    let mut hashes = Vec::new();
    let (mut placed, mut deployed, mut decayed, mut doors) = (0u32, 0u32, 0u32, 0u32);
    let (mut locked_seen, mut unlocked_seen) = (false, false);
    let mut hearth_cell = (0u16, 0u16);

    for t in 0..TICKS {
        let mut cmds: Vec<Command> = Vec::new();
        if t % 9 == 0 && joined < 64 {
            joined += 1;
            cmds.push(Command::Join { id: joined });
        }
        if t == 450 {
            cmds.push(Command::Leave { id: 3 });
        }
        if t == 500 {
            cmds.push(Command::Join { id: 3 }); // slot reuse is part of the contract
        }
        for id in 1..=joined {
            if (450..500).contains(&t) && id == 3 {
                continue;
            }
            let f = bot_frame(&mut rng, yaws[id as usize - 1], t as u16);
            yaws[id as usize - 1] = f.yaw;
            cmds.push(Command::Input { id, frame: f });
            // The craft verb rides the same log: periodic enqueues (row 3
            // is out of range — the refusal path) and rarer cancels.
            if (t + id as u64).is_multiple_of(37) {
                cmds.push(Command::Craft {
                    id,
                    recipe: ((t / 37 + id as u64) % 4) as u16,
                    count: 1 + (id as u64 % 3) as u16,
                });
            }
            if (t + id as u64).is_multiple_of(149) {
                cmds.push(Command::CraftCancel {
                    id,
                    index: (t % 4) as u16,
                });
            }
            // The build verb rides the log too: places at the player's own
            // cell (successes once wood accrues) plus out-of-range rows and
            // mismatched locs (the refusal paths).
            if (t + id as u64).is_multiple_of(53) {
                let b = &world.players[(id as usize - 1) % 64].body;
                let cell = |q: i32| {
                    sim_core::build::build_cell_of(q as f32 * sim_core::movement::POS_XZ_Q)
                        .clamp(0, 1023) as u16
                };
                // Row 4 is out of range (the fixture is 4 rows since the
                // doorway joined it) — the refusal path stays in surface.
                cmds.push(Command::Place {
                    id,
                    row: ((t / 53 + id as u64) % 5) as u16,
                    cx: cell(b.qx),
                    cz: cell(b.qz),
                    level: ((t / 106) % 2) as u8,
                    loc: ((t / 53 + id as u64) % 4) as u8,
                });
            }
            // The deploy verb too: a bag and a workbench at the player's
            // feet (the success shapes, once the granted or gathered
            // items are there), a rotating junk request for the refusal
            // reasons, and a feed (mostly the no-hearth refusal).
            if (t + id as u64).is_multiple_of(71) {
                let b = &world.players[(id as usize - 1) % 64].body;
                let cell = |q: i32| {
                    sim_core::build::build_cell_of(q as f32 * sim_core::movement::POS_XZ_Q)
                        .clamp(0, 1023) as u16
                };
                let (cx, cz) = (cell(b.qx), cell(b.qz));
                cmds.push(Command::PlaceDeploy {
                    id,
                    row: 3,
                    cx,
                    cz,
                    level: 0,
                    loc: 0,
                });
                cmds.push(Command::PlaceDeploy {
                    id,
                    row: 1,
                    cx: (cx + 1).min(1023),
                    cz,
                    level: 0,
                    loc: 0,
                });
                cmds.push(Command::PlaceDeploy {
                    id,
                    row: ((t / 71 + id as u64) % 5) as u16,
                    cx,
                    cz,
                    level: ((t / 142) % 2) as u8,
                    loc: ((t / 71 + id as u64) % 4) as u8,
                });
                cmds.push(Command::Feed {
                    id,
                    cx,
                    cz,
                    level: 0,
                });
            }
        }
        // A scripted hearth: grant a kit to the first eight bots (a
        // fixture arrangement, like the wire tests' server-side grants —
        // identical on both runs, so the replay contract holds), then
        // bot 1 founds, hearths, and feeds one remembered cell. This
        // pins the pay path — everything unpaid decays by the leaps.
        if t == 149 {
            for w in 0..8usize {
                if world.players[w].active {
                    for (k, &(item, count)) in [(0u16, 200u16), (1, 200), (2, 50), (3, 50), (4, 50)]
                        .iter()
                        .enumerate()
                    {
                        world.players[w].inv[20 + k] = sim_core::gather::ItemStack { item, count };
                    }
                }
            }
        }
        if t == 150 {
            let b = &world.players[0].body;
            let cell = |q: i32| {
                sim_core::build::build_cell_of(q as f32 * sim_core::movement::POS_XZ_Q)
                    .clamp(0, 1023) as u16
            };
            hearth_cell = (cell(b.qx), cell(b.qz));
        }
        if (150..=160).contains(&t) {
            let (cx, cz) = hearth_cell;
            let id = world.players[0].id;
            match t {
                150 => cmds.push(Command::Place {
                    id,
                    row: 0,
                    cx,
                    cz,
                    level: 0,
                    loc: 0,
                }),
                151 => cmds.push(Command::PlaceDeploy {
                    id,
                    row: 0,
                    cx,
                    cz,
                    level: 0,
                    loc: 0,
                }),
                // A doorway on the same cell's west edge, a door in it,
                // then the door verbs' whole arc — placement seals the
                // edge locked, its owner's toggles open and reseal it,
                // and the lock verb rides both ways (a stranger's lock
                // attempt in between, refused) — so the bodies that walk
                // that edge afterwards feel each state. All of it before
                // the feeds, which hand the same wood to the hearth.
                152 => cmds.push(Command::Place {
                    id,
                    row: 3,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_W,
                }),
                153 => cmds.push(Command::PlaceDeploy {
                    id,
                    row: 2,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_W,
                }),
                154 | 157 => cmds.push(Command::Use {
                    id,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_W,
                }),
                155 | 156 | 158 => cmds.push(Command::Lock {
                    // 156 is a hand that does not own this door — the
                    // refusal path, on the replayed surface too.
                    id: if t == 156 { world.players[1].id } else { id },
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_W,
                    locked: t == 158,
                }),
                _ => cmds.push(Command::Feed {
                    id,
                    cx,
                    cz,
                    level: 0,
                }),
            }
        }
        // Leap the clock three upkeep periods on a cadence: over the run
        // that is ~40 periods, enough to decay unpaid fixture pieces
        // (100 hp − 5/period) all the way to removal — charge, decay,
        // and the removal cascade all inside the replayed surface.
        if t % 64 == 63 {
            world.tick += 3 * sim_core::deploy::UPKEEP_PERIOD_TICKS;
        }
        world.tick(&cmds);
        for e in world.events.entries() {
            match e.code {
                sim_core::world::EV_PIECE_PLACED => placed += 1,
                sim_core::world::EV_DEPLOY_PLACED => deployed += 1,
                sim_core::world::EV_PIECE_REMOVED | sim_core::world::EV_DEPLOY_REMOVED => {
                    decayed += 1
                }
                sim_core::world::EV_DOOR => {
                    doors += 1;
                    if e.b & 2 == 0 {
                        unlocked_seen = true;
                    } else {
                        locked_seen = true;
                    }
                }
                _ => {}
            }
        }
        if world.tick.is_multiple_of(STATE_HASH_INTERVAL) {
            hashes.push(world.last_hash);
        }
    }
    // Counted from events, not the final stores: decay legitimately
    // removes early placements before the run ends.
    assert!(
        placed > 0,
        "the script placed nothing — the build success path fell out of the replay surface"
    );
    assert!(
        deployed > 0,
        "the script deployed nothing — the deploy success path fell out of the replay surface"
    );
    assert!(
        decayed > 0,
        "nothing decayed away — the removal path fell out of the replay surface"
    );
    assert!(
        doors >= 4,
        "the scripted door never toggled — the use verb fell out of the replay surface"
    );
    assert!(
        locked_seen && unlocked_seen,
        "the scripted door never changed hands both ways — the lock verb fell out \
         of the replay surface"
    );
    (hashes, world.state_hash())
}

#[test]
fn test_replay() {
    let (hashes_a, final_a) = run(SEED);
    let (hashes_b, final_b) = run(SEED);

    assert_eq!(hashes_a.len() as u64, TICKS / STATE_HASH_INTERVAL);
    assert_eq!(
        hashes_a, hashes_b,
        "replay diverged: same seed + same commands must reproduce every stamped hash"
    );
    assert_eq!(final_a, final_b);
    assert_eq!(
        final_a, GOLDEN_FINAL_HASH,
        "sim behavior drifted from the pinned replay golden; if intentional, \
         regenerate the golden in this same commit"
    );

    // A different seed must actually change the world (guards against a
    // degenerate hash or a sim that ignores its inputs).
    let (_, final_other) = run(SEED ^ 1);
    assert_ne!(final_a, final_other);
}
