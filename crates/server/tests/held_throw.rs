//! **The satchel in hand is the one planted** (NOW §0rc 2). A hotbar
//! switch rides the input lane, through the jitter buffer; `X` rides the
//! action lane, which skips it. So a plant pressed just after a switch used
//! to reach the sim while the cursor was still on an older frame — the old
//! slot in hand, and the plant refused as "you cannot pay"
//! (`REFUSE_B_COST`). The shard now holds a hand-reading action until the
//! frames buffered ahead of it have acted (`ClientNetState::hand_ready`).
//!
//! Through `ShardCore` on shipped content: frames go in as datagrams and the
//! plant as an action, exactly as the net thread hands them over. No
//! sockets, no clock: ticks and observable state.

mod common;

use common::SEED;
use protocol::{ActionMsg, InputDatagram};
use server::core::ShardCore;
use server::stats::ShardStats;
use sim_core::build::{
    build_cell_of, foundation_terrain_ok, BUILD_CELL_M, LOC_PLANE, REFUSE_B_COST,
};
use sim_core::gather::ItemStack;
use sim_core::input::InputFrame;
use sim_core::world::EV_BUILD_REFUSED;

const ID: u32 = 7;
/// Where the satchel sits. Slot 0 holds the wood, which is no charge.
const SATCHEL_SLOT: u8 = 2;

/// A cell a foundation may stand on, near a dry clearing: its address and
/// its centre.
fn buildable() -> (u16, u16, (f32, f32)) {
    let haven = sim_core::terrain::haven(SEED);
    let (x0, z0) = common::clearing(2);
    let (c0x, c0z) = (build_cell_of(x0), build_cell_of(z0));
    for r in 0..16i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let (cx, cz) = (c0x + dx, c0z + dz);
                let (x, z) = (
                    (cx as f32 + 0.5) * BUILD_CELL_M,
                    (cz as f32 + 0.5) * BUILD_CELL_M,
                );
                if foundation_terrain_ok(SEED, &haven, x, z) {
                    return (cx as u16, cz as u16, (x, z));
                }
            }
        }
    }
    panic!("no buildable cell near the clearing");
}

struct Raid {
    core: Box<ShardCore>,
    stats: ShardStats,
    cx: u16,
    cz: u16,
    seq: u16,
}

impl Raid {
    /// A raider on a twig foundation of its own, wood in slot 0 and
    /// `satchels` in `SATCHEL_SLOT`, holding slot 0 with an empty buffer.
    fn new(satchels: u16) -> Self {
        let content = common::content();
        let (cx, cz, at) = buildable();
        let core = common::shard(&content, at, false, ID);
        let mut r = Raid {
            core,
            stats: ShardStats::default(),
            cx,
            cz,
            seq: 0,
        };
        r.tick();
        let wood = content.item_index("item.wood").unwrap();
        let satchel = content.item_index("item.satchel_charge").unwrap();
        let twig = content.piece_index("build.foundation_twig").unwrap();
        let w = r.slot();
        let p = &mut r.core.world.players[w];
        p.inv[0] = ItemStack {
            item: wood,
            count: 1000,
            cond: 0,
            skin: 0,
        };
        p.inv[SATCHEL_SLOT as usize] = ItemStack {
            item: satchel,
            count: satchels,
            cond: 0,
            skin: 0,
        };
        r.core.push_action(
            0,
            ActionMsg::Place {
                row: twig,
                cx,
                cz,
                level: 0,
                loc: LOC_PLANE,
                freehand: false,
                plate: 0,
            },
        );
        r.tick();
        assert_eq!(r.core.world.pieces.len(), 1, "the twig foundation goes up");
        // Settled: a frame a tick, each acting the tick it lands.
        for _ in 0..4 {
            r.frames(&[0]);
            r.tick();
        }
        r
    }

    fn slot(&self) -> usize {
        let w = &self.core.world;
        w.players
            .iter()
            .position(|p| p.active && p.id == ID)
            .unwrap()
    }

    /// One datagram carrying the next frames, one per `sels` entry.
    fn frames(&mut self, sels: &[u8]) {
        let mut dg = InputDatagram::new(0, 0, sim_core::limits::INTERP_DELAY_TICKS);
        for &sel in sels {
            self.seq = self.seq.wrapping_add(1);
            dg.push(InputFrame {
                seq: self.seq,
                sel,
                ..InputFrame::default()
            })
            .unwrap();
        }
        self.core.push_input(0, &dg);
    }

    fn throw(&mut self) {
        assert!(self.core.wants_action(0), "the hand is open");
        self.core.push_action(
            0,
            ActionMsg::Throw {
                deploy: false,
                cx: self.cx,
                cz: self.cz,
                level: 0,
                loc: LOC_PLANE,
            },
        );
    }

    /// One tick; whether the raider was refused a plant on it.
    fn tick(&mut self) -> bool {
        self.core.tick_bare(&self.stats, |_, _, _| true);
        self.core
            .world
            .events
            .entries()
            .iter()
            .any(|e| e.code == EV_BUILD_REFUSED && e.a == ID && e.b == REFUSE_B_COST)
    }

    /// Tick until a charge is down or `ticks` run out: the tick it landed
    /// on (1-based), or `None`, panicking on a refused plant.
    fn plant_lands_within(&mut self, ticks: u32) -> Option<u32> {
        let before = self.core.world.charges.len();
        for t in 1..=ticks {
            let refused = self.tick();
            assert!(
                !refused,
                "the plant was refused as unpaid on tick {t}: it acted on a frame \
                 older than the slot switch the client made before pressing"
            );
            if self.core.world.charges.len() > before {
                return Some(t);
            }
        }
        None
    }
}

/// **The whole item.** The switch to the satchel and the plant arrive in
/// one tick, with the switch still two frames deep in the buffer. The plant
/// waits for those frames and lands with the satchel in hand — on the tick
/// the newest of them acts, and not later. Red without the wait: the plant
/// acts after frame one (slot 0, the wood) and is refused.
#[test]
fn a_plant_pressed_just_after_a_switch_plants_the_satchel() {
    let mut r = Raid::new(1);
    r.frames(&[0, SATCHEL_SLOT, SATCHEL_SLOT]);
    r.throw();
    assert_eq!(
        r.plant_lands_within(6),
        Some(3),
        "the plant lands on the tick the third buffered frame acts"
    );
    let w = r.slot();
    assert_eq!(r.core.world.players[w].frame.sel, SATCHEL_SLOT);
    assert_eq!(
        r.core.world.players[w].inv[SATCHEL_SLOT as usize].count, 0,
        "and the satchel left the hand"
    );
}

/// **No added latency when nothing is buffered behind.** A plant that
/// arrives with only its own frame (the steady lockstep a driven agent
/// keeps) lands on that tick; one that arrives on an empty buffer lands on
/// the next.
#[test]
fn a_plant_with_nothing_buffered_ahead_acts_at_once() {
    let mut r = Raid::new(2);
    r.frames(&[SATCHEL_SLOT]);
    r.throw();
    assert_eq!(
        r.plant_lands_within(1),
        Some(1),
        "frame and plant, one tick"
    );

    // Past the throw's pace (one a second), holding the satchel.
    for _ in 0..throw_pace() {
        r.frames(&[SATCHEL_SLOT]);
        r.tick();
    }
    r.throw();
    assert_eq!(
        r.plant_lands_within(1),
        Some(1),
        "an empty buffer, one tick"
    );
}

/// `pace::gap(Kind::Throw)` plus a margin, in ticks.
fn throw_pace() -> u32 {
    server::pace::gap(server::pace::Kind::Throw) as u32 + 2
}
