//! Dev probe: what a tick of **hitscan** costs, at population.
//!
//! **Why it exists.** `ranged::hitscan` walks a firearm's whole reach in one
//! tick — the shipped revolver is 50 m at `ARROW_STEP_MM`, which is 295
//! `terrain::ground` evaluations — where an arrow spreads sixteen samples
//! over each of forty ticks. That asymmetry is the design (a bullet has no
//! flight), and it means the shard-wide cost is a thing somebody has to
//! measure rather than reason about. It is what
//! `MAX_HITSCAN_MARK_SAMPLES` was sized off (`findings/hitscan-cost-20260819.md`).
//!
//! It is **not a gate**: a wall that waits on a clock is not a wall on this
//! box (`CLAUDE.md`), and a threshold here would be exactly that. The tick
//! budget is 33 ms and the numbers below are read by a person.
//!
//! ```text
//! cargo run -p sim-core --release --example shot_cost            # duelling grid
//! cargo run -p sim-core --release --example shot_cost -- --lonely # nobody in range
//! cargo run -p sim-core --release --example shot_cost -- --ground # standing on the island
//! ```
//!
//! `--lonely` is the worst case and the reason the mark is bounded: with no
//! body in the line, the only thing a walk can find is where to draw a
//! decal.

// Host-side timing probe: the clock and the printing are its job. The wall-1
// and wall-3 lists bind SIM code, and an example binary is not sim code —
// `terrain_stats.rs` carries the same allow for the same reason.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]

use sim_core::collide::ColIndex;
use sim_core::combat::{CombatContent, RangedDef};
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{MAX_ARROWS, MAX_PLAYERS};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::{Pristine, Scratch};
use sim_core::ranged::{self, Kill};
use sim_core::world::{EventQueue, Player};

/// The seed the shard ships.
const SEED: u64 = 20260731;
/// Item indices the fixture makes a revolver and its round.
const GUN: u16 = 5;
const ROUND: u16 = 6;

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

/// Where shooter `i` stands. The default is a 3 m grid, so everyone has a
/// neighbour ten cells down the barrel and the trace truncates at them;
/// `--lonely` is 60 m apart, so nobody is inside anybody's fifty.
fn spread(i: usize, lonely: bool) -> (f32, f32) {
    if lonely {
        (
            300.0 + (i % 10) as f32 * 60.0,
            300.0 + (i / 10) as f32 * 60.0,
        )
    } else {
        (900.0 + (i % 10) as f32 * 3.0, 900.0 + (i / 10) as f32 * 3.0)
    }
}

fn main() {
    let (lonely, on_ground) = (flag("--lonely"), flag("--ground"));
    let haven = sim_core::terrain::haven(SEED);
    let mut cc = CombatContent::EMPTY;
    cc.player_hp = 100;
    // The shipped revolver: 150 rounds/min at 30 Hz, 50 m, no ballistics.
    cc.ranged[GUN as usize] = RangedDef {
        damage: 1,
        ammo: [ROUND, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 12,
        hitscan: true,
        range_mm: 50_000,
    };
    let mut pristine = Scratch::with(SEED, Pristine);
    let mut barren = Scratch::barren();
    let cols = ColIndex::new();

    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    for (i, p) in players.iter_mut().enumerate() {
        let (x, z) = spread(i, lonely);
        let g = sim_core::terrain::ground(SEED, &haven, x, z);
        // Damage 1 and full hp so nobody dies inside the measurement — a
        // corpse stops shooting and would flatter the number.
        *p = Player {
            id: i as u32 + 1,
            active: true,
            hp: u16::MAX,
            hp_max: u16::MAX,
            ..Player::default()
        };
        p.body = Body {
            qx: (x / POS_XZ_Q) as i32,
            qy: ((g + if on_ground { 0.0 } else { 200.0 }) / POS_Y_Q) as i32,
            qz: (z / POS_XZ_Q) as i32,
            ..Body::default()
        };
        p.inv[0] = ItemStack {
            item: GUN,
            count: 1,
            cond: 0,
        };
        p.inv[7] = ItemStack {
            item: ROUND,
            count: u16::MAX,
            cond: 0,
        };
        p.frame = InputFrame {
            seq: 1,
            buttons: BTN_PRIMARY,
            yaw: 0,
            pitch: 128,
            sel: 0,
            ..InputFrame::default()
        };
    }

    let mut kills = [Kill::default(); MAX_ARROWS];
    let (mut worst, mut total, mut n) = (0u128, 0u128, 0u128);
    // Everyone fires on the same tick every twelfth, which is the aligned
    // volley — the case a cap has to survive, not the case play produces.
    for t in 0..(12 * 40u64) {
        let mut events = EventQueue::default();
        let start = std::time::Instant::now();
        ranged::hitscan(
            SEED,
            &haven,
            &cols,
            &mut if on_ground {
                pristine.occupants()
            } else {
                barren.occupants()
            },
            t,
            &cc,
            &mut players,
            &mut events,
            &mut kills,
        );
        let dt = start.elapsed().as_micros();
        if t % 12 == 0 {
            worst = worst.max(dt);
            total += dt;
            n += 1;
        }
    }
    println!(
        "{MAX_PLAYERS} shooters, {} occupants, {}: {n} firing ticks, mean {:.2} ms, worst {:.2} ms \
         (tick budget 33 ms)",
        if on_ground { "pristine" } else { "barren" },
        if lonely {
            "nobody in range"
        } else {
            "a body 3 m down every barrel"
        },
        total as f64 / n as f64 / 1000.0,
        worst as f64 / 1000.0,
    );
}
