//! Native half of `test_parity_wasm`: prints probe digests, one per line,
//! in exactly the format ci/parity.mjs prints from the wasm build.
//! ci/gates.sh diffs the two outputs; any difference is a red gate.

// Host-side parity driver: printing is its job. The L5 wall bans
// format/print in SIM code; an example binary is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::probe::{probe_bags, probe_combat, probe_parity, probe_terrain};

// Keep in lockstep with ci/parity.mjs — a mismatch shows up as a diff.
const TERRAIN_SEEDS: [u64; 3] = [0x0047_4154_4553, 0x1, 0xDEAD_BEEF];
const PARITY_MASTER_SEED: u64 = 0x0047_4154_4553;
const PARITY_SEQUENCES: u32 = 10_000;
const PARITY_TICKS: u32 = 16;
// Combat needs depth, not breadth: a kill is three landed hits and a
// swing is one per 38 ticks, so 16-tick sequences would never reach a
// death. Fewer, longer brawls instead.
const COMBAT_SEQUENCES: u32 = 500;
const COMBAT_TICKS: u32 = 256;
// Bags need a whole death cycle, not a brawl: the survival fixture's spans
// are ~10 s, so a body empties around tick 300 and dies of it a hundred-odd
// ticks later. 600 reaches two deaths per body, which is what puts a *spent*
// bag on the surface beside a fresh one.
const BAGS_SEQUENCES: u32 = 24;
const BAGS_TICKS: u32 = 600;

fn main() {
    for seed in TERRAIN_SEEDS {
        println!("terrain {seed:#018x} {:#018x}", probe_terrain(seed));
    }
    println!(
        "parity {PARITY_MASTER_SEED:#018x} {PARITY_SEQUENCES} {PARITY_TICKS} {:#018x}",
        probe_parity(PARITY_MASTER_SEED, PARITY_SEQUENCES, PARITY_TICKS)
    );
    println!(
        "combat {PARITY_MASTER_SEED:#018x} {COMBAT_SEQUENCES} {COMBAT_TICKS} {:#018x}",
        probe_combat(PARITY_MASTER_SEED, COMBAT_SEQUENCES, COMBAT_TICKS)
    );
    // Packed `wakes << 32 | digest32` — printed as its two halves so the
    // count is a field a gate can read, not a number buried in a hash.
    let bags = probe_bags(PARITY_MASTER_SEED, BAGS_SEQUENCES, BAGS_TICKS);
    println!(
        "bags {PARITY_MASTER_SEED:#018x} {BAGS_SEQUENCES} {BAGS_TICKS} {} {:#010x}",
        bags >> 32,
        bags & 0xFFFF_FFFF
    );
}
