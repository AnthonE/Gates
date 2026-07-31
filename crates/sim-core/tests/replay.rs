//! `test_replay` (DESIGN.md §7/§12): same build + same seed + same command
//! log → the same state hashes, every stamp. Until the server's WAL exists
//! this drives a deterministic in-memory command script — the contract is
//! identical, the fixture just isn't a file yet. The final hash is also
//! pinned: any accidental drift in sim behavior reddens this gate.

use sim_core::bots::bot_frame;
use sim_core::craft::CraftContent;
use sim_core::gather::GatherContent;
use sim_core::limits::STATE_HASH_INTERVAL;
use sim_core::rng::Pcg32;
use sim_core::world::{Command, World};

const SEED: u64 = 0x0047_4154_4553; // "GATES"
const TICKS: u64 = 900;

/// Pinned end-state hash for (SEED, the script below). Regenerates only
/// with an intentional sim change, in the same commit (CLAUDE.md wall 5).
/// Regenerated this commit: the craft verb landed — state_hash covers the
/// per-player craft queue, and the script drives Craft/CraftCancel
/// commands (enqueue, completion, refusal, cancel) through the log.
const GOLDEN_FINAL_HASH: u64 = 0xECCF_F5D7_4B0E_4664;

fn run(seed: u64) -> (Vec<u64>, u64) {
    let mut world = World::new(seed);
    world.gather = GatherContent::probe_fixture();
    world.craft = CraftContent::probe_fixture();
    let mut rng = Pcg32::new(seed, 11);
    let mut yaws = [0u16; 64];
    let mut joined: u32 = 0;
    let mut hashes = Vec::new();

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
        }
        world.tick(&cmds);
        if world.tick.is_multiple_of(STATE_HASH_INTERVAL) {
            hashes.push(world.last_hash);
        }
    }
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
