//! The island's public places as a player uses them: THE GATE's kiosks
//! (`sim_core::vend`, `content/sites.toml`) and the works (`sim_core::works`,
//! `ARC.md`).
//!
//! **Trading.** The stalls take junk for food, building kit, tools, parts and
//! arms, and the SALVAGE stall buys surplus cloth, fragments, wood and stone
//! for junk. What it knows of them is what its client was sent: the offer
//! table (`ClientCore::vend`) and the island's unlocks (`ClientCore::arc`).
//! A trade ([`TradeJob`]) walks to the stall's counter, sends the human
//! client's vend verb once, and reads the answer. It never walks in while
//! it is marked hostile (`ClientCore::hostile_left`): the gate's guns shoot
//! anyone who attacked a player lately.
//!
//! **The works.** Each work takes a quota of goods while open (deposited at
//! its terminal), lights when the quota is met, unlocking things for the
//! whole island, and burns fuel while lit. A feed ([`WorkJob`]) walks to the
//! nearest work that takes something the pack holds and puts it all in, or
//! tops a lit one's tank up; credit for it is the depositor's.

use crate::agent::intent::{yaw_toward, Intent};
use crate::agent::route::{Route, Step};
use crate::agent::stash::carried;
use crate::mind::Why;
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::limits::{MAX_WORK_INPUTS, TICK_HZ};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::terrain::Haven;
use sim_core::vend::{VEND_REACH_M, VEND_TIMES_MAX};
use sim_core::works::{
    rekindle_min, ARG_ALL, OP_DEPOSIT, OP_FUEL, WORK_LIT, WORK_OPEN, WORK_REACH_M,
};

/// A sale keeps this many lots of what it sells.
pub const SELL_KEEP_LOTS: u32 = 2;
/// A purchase buys at most this many lots at once, and one of anything
/// that does not stack.
pub const BUY_LOTS_MAX: u32 = 5;
/// Where it stands to trade or feed: this far inside the reach.
pub const STAND_SLACK_M: f32 = 1.0;
/// Trades a summary offers, at most.
pub const TRADE_ROWS: usize = 8;
/// An answer comes within this long.
const VERDICT_TICKS: u32 = 3 * TICK_HZ;

/// The item the stalls are paid in.
pub const JUNK: &str = "Junk";

fn junk(core: &ClientCore) -> Option<u16> {
    crate::agent::build::item_named(core, JUNK)
}

/// The offer and how many times over, as the pack and the stalls stand
/// now: buying `item` with junk (as many lots as it can pay for, up to
/// [`BUY_LOTS_MAX`], one of what does not stack), or selling it for junk
/// (all but [`SELL_KEEP_LOTS`] lots). `None` when no such trade can be made.
pub fn trade_for(core: &ClientCore, item: u16, sell: bool) -> Option<(u8, u8)> {
    let junk = junk(core)?;
    (0..usize::from(core.vend.count)).find_map(|k| {
        let o = core.vend.get(k)?;
        if !core.arc.holds(o.unlock) || o.pay_n == 0 || o.get_n == 0 {
            return None;
        }
        let lots = if sell {
            if o.pay != item || o.get != junk {
                return None;
            }
            (carried(core, item) / u32::from(o.pay_n)).saturating_sub(SELL_KEEP_LOTS)
        } else {
            if o.get != item || o.pay != junk {
                return None;
            }
            let single = core.catalog.row(usize::from(item)).stack_max <= 1;
            if single && carried(core, item) > 0 {
                return None;
            }
            let cap = if single { 1 } else { BUY_LOTS_MAX };
            (carried(core, junk) / u32::from(o.pay_n)).min(cap)
        };
        let lots = lots.min(u32::from(VEND_TIMES_MAX));
        (lots > 0).then_some((k as u8, lots as u8))
    })
}

/// The trades it could make now, into `out` as `(item, sell)`: what it
/// `wants` (its base's shortfall) bought first, then what it can sell, then
/// what it can buy, dearest first. The rows filled. With forty stalls and
/// eight rows, a fragment it needed was never on the list.
pub fn trades(
    core: &ClientCore,
    wants: impl Iterator<Item = u16>,
    out: &mut [(u16, bool); TRADE_ROWS],
) -> usize {
    let mut n = 0;
    let Some(junk) = junk(core) else {
        return 0;
    };
    let mut push = |out: &mut [(u16, bool); TRADE_ROWS], row: (u16, bool)| {
        if n < TRADE_ROWS && !out[..n].contains(&row) && trade_for(core, row.0, row.1).is_some() {
            out[n] = row;
            n += 1;
        }
    };
    for item in wants {
        push(out, (item, false));
    }
    for k in 0..usize::from(core.vend.count) {
        if let Some(o) = core.vend.get(k).filter(|o| o.get == junk) {
            push(out, (o.pay, true));
        }
    }
    // Dearest first: the summary has room for a few, and a metal hatchet
    // says more than a third handful of berries.
    let mut buys = [(0u16, 0u16); sim_core::limits::MAX_VEND_OFFERS];
    let mut m = 0;
    for k in 0..usize::from(core.vend.count) {
        if let Some(o) = core.vend.get(k).filter(|o| o.pay == junk) {
            buys[m] = (o.pay_n, o.get);
            m += 1;
        }
    }
    buys[..m].sort_unstable_by_key(|b| std::cmp::Reverse(b.0));
    for &(_, item) in &buys[..m] {
        push(out, (item, false));
    }
    n
}

/// Where kiosk `vendor` is traded at.
fn kiosk(core: &ClientCore, vendor: u8) -> Option<[f32; 2]> {
    sim_core::town::kiosk_world(&core.haven().town, usize::from(vendor)).map(|(x, z)| [x, z])
}

/// THE GATE: where its first stall stands, while the town is up.
pub fn gate(core: &ClientCore) -> Option<[f32; 2]> {
    if !core.haven().town.live {
        return None;
    }
    kiosk(core, 0)
}

/// Walk to THE GATE, stopping within `stop` of its first stall: where an
/// agent waits for its friend (`Goal::Meet`). `None` once there; a town
/// that is down is somewhere it cannot go.
pub fn approach(
    core: &mut ClientCore,
    body: &EntityState,
    route: &mut Route,
    stop: f32,
    tick: u32,
) -> Option<Result<Intent, Why>> {
    let Some(to) = gate(core) else {
        return Some(Err(Why::NotFound));
    };
    walk(core, body, route, to, stop, tick)
}

/// THE GATE as a travel goal names it (`Goal::Go`).
pub const GATE_NAME: &str = "THE GATE";

/// Where a public place stands, by the name its summary gives it (the
/// travel goal, `Goal::Go`): THE GATE while the town is up, a work its
/// client knows, or a lock it has found.
pub fn place(core: &ClientCore, name: &str) -> Option<[f32; 2]> {
    if name == GATE_NAME {
        return gate(core);
    }
    if let Some((_, w)) = core.arc.known().find(|(_, w)| w.name() == name) {
        return sim_core::spot::world(core.haven(), &w.def.spot).map(|(x, _, z)| [x, z]);
    }
    let (m, mech) = core.lore.known_mechs().find(|(_, m)| m.name() == name)?;
    let middle = usize::from(mech.n_dials.saturating_sub(1) / 2);
    crate::agent::lore::dial_at(core, m, middle).map(|p| [p[0], p[2]])
}

/// Walk to `to`, stopping within `stop`. `None` once there.
pub fn walk(
    core: &mut ClientCore,
    body: &EntityState,
    route: &mut Route,
    to: [f32; 2],
    stop: f32,
    tick: u32,
) -> Option<Result<Intent, Why>> {
    let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
    if (to[0] - x).hypot(to[1] - z) <= stop {
        return None;
    }
    Some(match route.to(core, body, to, stop - 0.2, false, tick) {
        step @ Step::Walk { .. } => Ok(step.walk().unwrap_or(Intent::IDLE)),
        Step::Wait | Step::Arrived => Ok(Intent::walk(yaw_toward(to[0] - x, to[1] - z))),
        Step::Blocked => Err(Why::Stuck),
    })
}

/// What a trade or a feed wants next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Visit {
    Go(Intent),
    /// Trade offer `offer`, `times` over.
    Vend {
        offer: u8,
        times: u8,
    },
    /// The arc verb at work `target`.
    Arc {
        op: u8,
        target: u8,
        arg: u8,
    },
    Done,
    Fail(Why),
}

/// One trade at THE GATE.
#[derive(Clone, Copy, Debug, Default)]
pub struct TradeJob {
    /// The verb went out at this tick, and its answer.
    sent: Option<u32>,
    answer: Option<bool>,
    pub trades: u64,
}

impl TradeJob {
    /// A new trade; the count of trades made is kept.
    pub fn reset(&mut self) {
        self.sent = None;
        self.answer = None;
    }

    pub fn sent(&mut self, tick: u32) {
        self.sent = Some(tick);
        self.answer = None;
    }

    /// The stall's answer: taken, or refused.
    pub fn on_vend(&mut self, refused: bool) {
        if self.sent.is_some() {
            self.answer = Some(!refused);
        }
    }

    pub fn step(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        route: &mut Route,
        item: u16,
        sell: bool,
        tick: u32,
    ) -> Visit {
        if let Some(sent) = self.sent {
            return match self.answer {
                Some(true) => {
                    self.trades += 1;
                    Visit::Done
                }
                Some(false) => Visit::Fail(Why::Refused),
                None if tick.wrapping_sub(sent) >= VERDICT_TICKS => Visit::Fail(Why::NoAnswer),
                None => Visit::Go(Intent::IDLE),
            };
        }
        // The gate's guns shoot whoever fought a player lately.
        if core.hostile_left() > 0 {
            return Visit::Fail(Why::Wanted);
        }
        let Some((offer, times)) = trade_for(core, item, sell) else {
            return Visit::Fail(Why::MissingInputs);
        };
        let Some(at) = core
            .vend
            .get(usize::from(offer))
            .and_then(|o| kiosk(core, o.vendor))
        else {
            return Visit::Fail(Why::NotFound);
        };
        match walk(core, body, route, at, VEND_REACH_M - STAND_SLACK_M, tick) {
            Some(Ok(intent)) => Visit::Go(intent),
            Some(Err(why)) => Visit::Fail(why),
            None => Visit::Vend { offer, times },
        }
    }
}

/// The work a feed goes to, and what it does there: the nearest that
/// takes something the pack holds, an open one's quota or a lit one's
/// fuel. `(work, op, where its terminal stands)`.
pub fn pick_work(core: &ClientCore, haven: &Haven, x: f32, z: f32) -> Option<(u8, u8, [f32; 3])> {
    let mut best: Option<(f32, u8, u8, [f32; 3])> = None;
    for (k, w) in core.arc.known() {
        let def = &w.def;
        let op = if w.state == WORK_OPEN
            && def
                .inputs
                .iter()
                .zip(w.got.iter())
                .take(usize::from(def.n_inputs))
                .any(|(i, got)| *got < i.need && carried(core, i.item) > 0)
        {
            OP_DEPOSIT
        } else if w.state == WORK_LIT
            && def.fuel_max > 0
            && w.fuel < def.fuel_max
            // A dry tank refuses less than a tenth of a full one.
            && carried(core, def.fuel) >= if w.fuel == 0 { rekindle_min(def) } else { 1 }
        {
            OP_FUEL
        } else {
            continue;
        };
        let Some((sx, sy, sz)) = sim_core::spot::world(haven, &def.spot) else {
            continue;
        };
        let d = (sx - x).hypot(sz - z);
        if best.is_none_or(|b| d < b.0) {
            best = Some((d, k as u8, op, [sx, sy, sz]));
        }
    }
    best.map(|(_, k, op, at)| (k, op, at))
}

/// What a work still takes, item by item, into `out`: its quota left while
/// open. The rows filled.
pub fn work_needs(
    w: &client_core::arc::WorkView,
    out: &mut [(u16, u32); MAX_WORK_INPUTS],
) -> usize {
    let mut n = 0;
    if w.state != WORK_OPEN {
        return 0;
    }
    for (i, got) in w
        .def
        .inputs
        .iter()
        .zip(w.got.iter())
        .take(usize::from(w.def.n_inputs))
    {
        if *got < i.need && n < out.len() {
            out[n] = (i.item, i.need - got);
            n += 1;
        }
    }
    n
}

/// One feed at a work.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorkJob {
    /// The verb went out at this tick, to this work with this op, and what
    /// the pack held of what it gives then; the work's answer if it refused
    /// (`sim_core::works::REFUSE_A_*`).
    sent: Option<(u32, u8, u8, u32)>,
    refused: Option<u8>,
    /// Units given.
    pub gave: u32,
}

impl WorkJob {
    /// An arc verb refused: this feed's, if it was its own op at its work.
    pub fn on_refused(&mut self, code: u8, op: u8, target: u8) {
        if self
            .sent
            .is_some_and(|(_, k, sent_op, _)| (k, sent_op) == (target, op))
        {
            self.refused = Some(code);
        }
    }

    /// What the pack holds of what work `k` takes now.
    fn holding(core: &ClientCore, k: u8, op: u8) -> u32 {
        let Some(w) = core.arc.works.get(usize::from(k)) else {
            return 0;
        };
        if op == OP_FUEL {
            return carried(core, w.def.fuel);
        }
        w.def
            .inputs
            .iter()
            .take(usize::from(w.def.n_inputs))
            .map(|i| carried(core, i.item))
            .sum()
    }

    pub fn sent(&mut self, core: &ClientCore, k: u8, op: u8, tick: u32) {
        self.sent = Some((tick, k, op, Self::holding(core, k, op)));
        self.refused = None;
    }

    pub fn step(
        &mut self,
        core: &mut ClientCore,
        haven: &Haven,
        body: &EntityState,
        route: &mut Route,
        tick: u32,
    ) -> Visit {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        if let Some(code) = self.refused {
            return Visit::Fail(Why::ArcRefused(code));
        }
        // A verb in flight: what it gave is what left the pack.
        if let Some((sent, k, op, before)) = self.sent {
            let now = Self::holding(core, k, op);
            if now < before {
                self.gave = self.gave.saturating_add(before - now);
                self.sent = None;
                return Visit::Done;
            }
            if tick.wrapping_sub(sent) >= VERDICT_TICKS {
                return Visit::Fail(Why::NoAnswer);
            }
            return Visit::Go(Intent::IDLE);
        }
        let Some((k, op, at)) = pick_work(core, haven, x, z) else {
            return Visit::Fail(Why::NotFound);
        };
        match walk(
            core,
            body,
            route,
            [at[0], at[2]],
            WORK_REACH_M - STAND_SLACK_M,
            tick,
        ) {
            Some(Ok(intent)) => Visit::Go(intent),
            Some(Err(why)) => Visit::Fail(why),
            None => {
                // The terminal's reach is a cylinder: a floor above or
                // below it is out of it however near.
                let feet = body.qy as f32 * POS_Y_Q;
                if (feet - at[1]).abs() > WORK_REACH_M {
                    return Visit::Fail(Why::Stuck);
                }
                let arg = if op == OP_DEPOSIT { ARG_ALL } else { 0 };
                Visit::Arc { op, target: k, arg }
            }
        }
    }
}
