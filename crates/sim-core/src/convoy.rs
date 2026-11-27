//! Convoys (`ARC.md` §5 M2): the war effort's columns. While THE SEVERED
//! GATE takes deposits, a column of pack beasts (`mob::MOB_CARRIER`) comes
//! in off the ring road every so often, laden with salvage for the Gate's
//! quota, and walks one of the town's two roads to its gate. A carrier that
//! gets there puts its load on the quota (`works::deliver`); one that is
//! killed on the way leaves its load in its carcass for whoever killed it,
//! exactly as any animal's loot (`mob::hurt_slot`). So a convoy is a thing
//! to see in or to raid, and either way it is heard across the island
//! (`works::WORK_EV_CONVOY`).
//!
//! **Its timetable is the arc clock.** Column `n` (from 1) sets out
//! `n × convoy_every` ticks after the work opens, if the work is still
//! taking deposits then, and the road it takes is drawn from the seed and
//! `n` ([`road_of`]). Nothing about it is remembered but the carriers
//! themselves: the column out is always the latest one, so its road is
//! re-derived from the tick, and a carrier's next road node is its
//! `Mob::leg`. A column still out when the next is due is given up for lost
//! and goes, so a carrier stuck against a wall never outlives its slot's
//! turn, and only one column is ever out ([`MAX_CONVOY`] slots).
//!
//! **It walks; it does not think.** A carrier is never homed, so
//! `mob::step` leaves it alone; here it is stepped every tick, awake or
//! not, through the same capsule every animal walks in
//! (`movement::step_scaled`), routed on the animals' grid (`nav.rs`, the
//! roster's search scratch and budget) to its next road node. Struck, it
//! runs on along the road for its row's `flee_seconds`. Alloc-free: the
//! block is fixed and the road is the haven's.

use crate::collide::ColIndex;
use crate::input::{InputFrame, BTN_SPRINT};
use crate::limits::{MAX_CONVOY, MOB_THINK_TICKS};
use crate::mob::{self, MobContent, Mobs, CARRIER_SLOT0, MOB_CARRIER};
use crate::movement::{self, POS_XZ_Q};
use crate::nav::{self, Nav};
use crate::occupy::Occupants;
use crate::rng::cell_hash;
use crate::terrain::{Haven, SideRoad, DEPOT_ROADS, SIDE_ROAD_POINTS};
use crate::town::TOWN_ROADS;
use crate::works::{
    self, Works, WorksContent, CONVOY_IN, CONVOY_OUT, NO_CONVOY, WORK_EV_CONVOY, WORK_OPEN,
};
use crate::world::{EventQueue, EV_WORK};

/// Which road a column takes.
const CH_CONVOY: u32 = 2100;
/// A road node is reached inside this, metres.
pub const ARRIVE_M: f32 = 3.0;
/// The column's carriers stand up this far apart, metres, the first at the
/// road's ring end and the rest strung out behind it.
pub const GAP_M: f32 = 6.0;
/// How near a road node a route steers, centimetres: inside [`ARRIVE_M`].
const STOP_CM: u16 = 200;
/// Less than this walked between two thinks, centimetres, and a carrier is
/// pinned (a walk covers ~110 between thinks).
const STUCK_CM: i64 = 25;
/// How long a pinned carrier steps off its way, ticks: one think.
const DETOUR_TICKS: u64 = MOB_THINK_TICKS;
/// Further off its heading than this a carrier turns where it stands
/// (`mob.rs`'s rule for every animal).
const TURN_IN_PLACE: u16 = 64 << 8;

/// The column number `tick` sends out, if it is a departure tick: `n ≥ 1`,
/// `n × convoy_every` ticks after the convoy's work opened.
pub fn departs(wc: &WorksContent, tick: u64) -> Option<u64> {
    let (def, _) = convoy_def(wc)?;
    let since = tick.checked_sub(def.opens_at)?;
    (since > 0 && since.is_multiple_of(wc.convoy_every)).then_some(since / wc.convoy_every)
}

/// The column that is (or was last) out on `tick`: the latest that set out.
pub fn current(wc: &WorksContent, tick: u64) -> Option<u64> {
    let (def, _) = convoy_def(wc)?;
    let n = tick.checked_sub(def.opens_at)? / wc.convoy_every;
    (n > 0).then_some(n)
}

fn convoy_def(wc: &WorksContent) -> Option<(&works::WorkDef, usize)> {
    if wc.convoy_work == NO_CONVOY || wc.convoy_carriers == 0 || wc.convoy_every == 0 {
        return None;
    }
    let k = wc.convoy_work as usize;
    wc.get(k).map(|d| (d, k))
}

/// The road column `n` walks, drawn among the town's live roads: an index
/// into `Haven::roads`. `None` on an island with no town or no road to it.
pub fn road_of(seed: u64, haven: &Haven, n: u64) -> Option<usize> {
    if !haven.town.live {
        return None;
    }
    let town = DEPOT_ROADS..DEPOT_ROADS + TOWN_ROADS;
    let live = haven.roads[town.clone()].iter().filter(|r| r.live).count();
    if live == 0 {
        return None;
    }
    let pick = (cell_hash(seed, n as i32, (n >> 31) as i32, CH_CONVOY) % live as u64) as usize;
    town.filter(|&i| haven.roads[i].live).nth(pick)
}

/// Where carrier `i` of a column stands up on `road`: the ring end, and
/// each one after it [`GAP_M`] further back along the road's last leg.
pub fn stand_at(road: &SideRoad, i: usize) -> (f32, f32) {
    let (rx, rz) = road.node(SIDE_ROAD_POINTS - 1);
    let (ax, az) = road.node(SIDE_ROAD_POINTS - 2);
    let (dx, dz) = (rx - ax, rz - az);
    let len = (dx * dx + dz * dz).sqrt();
    if len <= 0.0 {
        return (rx, rz);
    }
    let back = GAP_M * i as f32 / len;
    (rx + dx * back, rz + dz * back)
}

/// One tick of the island's convoy: send a column out on its hour, walk the
/// one that is out, and hand in what reaches the town's gate.
#[allow(clippy::too_many_arguments)]
pub fn step(
    seed: u64,
    haven: &Haven,
    tick: u64,
    wc: &WorksContent,
    works: &mut Works,
    mc: &MobContent,
    mobs: &mut Mobs,
    nav: &mut Nav,
    cols: &ColIndex,
    occ: &mut Occupants,
    events: &mut EventQueue,
) {
    let Some((_, k)) = convoy_def(wc) else {
        return;
    };
    let def = mc.def(MOB_CARRIER);
    if def.hp == 0 {
        return;
    }
    let carriers = (wc.convoy_carriers as usize).min(MAX_CONVOY);
    if let Some(n) = departs(wc, tick) {
        // The last column, if any of it is still on the road, is lost.
        for m in mobs.m[CARRIER_SLOT0..CARRIER_SLOT0 + MAX_CONVOY].iter_mut() {
            m.alive = false;
        }
        if works.w[k].state == WORK_OPEN {
            if let Some(r) = road_of(seed, haven, n) {
                let road = haven.roads[r];
                let (ax, az) = road.node(SIDE_ROAD_POINTS - 2);
                for i in 0..carriers {
                    let m = &mut mobs.m[CARRIER_SLOT0 + i];
                    let (x, z) = stand_at(&road, i);
                    m.yaw = nav::yaw_toward(ax - x, az - z, m.yaw);
                    mob::stand_up(seed, haven, m, &def, x, z);
                    m.awake = true;
                    m.state = crate::brain::AiState::Roam;
                    m.leg = (SIDE_ROAD_POINTS - 2) as u8;
                }
                events.push(EV_WORK, k as u32, WORK_EV_CONVOY, CONVOY_OUT);
            }
        }
    }
    let Some(road) = current(wc, tick).and_then(|n| road_of(seed, haven, n)) else {
        return;
    };
    let road = haven.roads[road];
    for i in 0..carriers {
        let slot = CARRIER_SLOT0 + i;
        if !mobs.m[slot].alive {
            continue;
        }
        let m = &mut mobs.m[slot];
        let (x, z) = (m.body.qx as f32 * POS_XZ_Q, m.body.qz as f32 * POS_XZ_Q);
        let (mut tx, mut tz) = road.node(m.leg as usize);
        if (tx - x) * (tx - x) + (tz - z) * (tz - z) <= ARRIVE_M * ARRIVE_M {
            if m.leg == 0 {
                // In at the town's gate: the load goes on the quota, and the
                // beast goes in.
                m.alive = false;
                m.gait = 0;
                works::deliver(wc, works, k, &def.loot, tick, events);
                let out = mobs.m[CARRIER_SLOT0..CARRIER_SLOT0 + MAX_CONVOY]
                    .iter()
                    .any(|m| m.alive);
                if !out {
                    events.push(EV_WORK, k as u32, WORK_EV_CONVOY, CONVOY_IN);
                }
                continue;
            }
            m.leg -= 1;
            (tx, tz) = road.node(m.leg as usize);
            m.path.clear();
        }
        // Routed to the node on the animals' own grid, so a rock, a tree
        // or a wall on the road is walked round rather than into: a fresh
        // route each think (phase-offset by slot, as the roster thinks), or
        // at once when the last one is done.
        let think = tick % MOB_THINK_TICKS == slot as u64 % MOB_THINK_TICKS;
        if think && tick >= m.state_until {
            // Pinned: it walked and hardly moved since the last think. It
            // steps off square to its way for a think, to one side or the
            // other by the think's parity, and is routed again from there.
            let moved = nav::dist2_cm(m.body.qx - m.last_qx, m.body.qz - m.last_qz);
            if m.gait != 0 && moved < STUCK_CM * STUCK_CM {
                let side: u16 = if (tick / MOB_THINK_TICKS).is_multiple_of(2) {
                    64 << 8
                } else {
                    192 << 8
                };
                m.want_yaw = m.yaw.wrapping_add(side);
                m.state_until = tick + DETOUR_TICKS;
            }
            m.last_qx = m.body.qx;
            m.last_qz = m.body.qz;
        }
        let detour = tick < m.state_until;
        if !detour && (!m.path.active() || think) {
            let y = m.body.qy as f32 * movement::POS_Y_Q;
            let mut ground = nav::Ground {
                seed,
                haven,
                cols,
                occ: &mut *occ,
            };
            nav.plan(&mut ground, x, y, z, tx, tz, STOP_CM, &mut m.path);
        }
        let want = if detour {
            m.want_yaw
        } else {
            let goal = if m.path.active() {
                nav::waypoint(&mut m.path, m.body.qx, m.body.qz)
                    .map(|(qx, qz)| (qx as f32 * POS_XZ_Q, qz as f32 * POS_XZ_Q))
            } else {
                None
            };
            let (gx, gz) = goal.unwrap_or((tx, tz));
            nav::yaw_toward(gx - x, gz - z, m.yaw)
        };
        m.yaw = nav::turn_toward(m.yaw, want, mob::MOB_TURN_STEP);
        let hurried = m.roused_until > tick;
        m.gait = if nav::yaw_gap(m.yaw, want) > TURN_IN_PLACE {
            0
        } else if hurried {
            def.flee_gait as i8
        } else {
            def.gait as i8
        };
        m.state = if hurried {
            crate::brain::AiState::Flee
        } else {
            crate::brain::AiState::Roam
        };
        let frame = InputFrame {
            yaw: m.yaw,
            move_z: m.gait,
            buttons: if hurried { BTN_SPRINT } else { 0 },
            ..InputFrame::default()
        };
        movement::step_scaled(seed, haven, cols, occ, &mut m.body, &frame, 1.0);
    }
}
