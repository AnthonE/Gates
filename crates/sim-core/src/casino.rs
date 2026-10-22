//! THE GATE's games of chance (`town::CASINO`): Rust's Bandit Camp big wheel
//! and slot machines, staked in the island's junk.
//!
//! Both are junk sinks. Every rule below that Rust's server code states is
//! Rust's (decompiled `BigWheelGame`, `BigWheelBettingTerminal`,
//! `SlotMachine`, research 2026-10-18), and the house keeps an edge on every
//! bet: [`wheel_return_pm`] and [`slot_return_pm`] are the exact returns, and
//! the tests hold both under 100%. A stake is burned when it is placed; a win
//! is minted when it is paid. There is no pot to drain.
//!
//! **The wheel** is shared: one round a minute for the whole island, and it
//! turns in every round someone has junk on. Bets are open for the first [`WHEEL_OPEN_TICKS`] of each round, at any of the
//! terminals; then the wheel spins for [`WHEEL_SPIN_TICKS`] and pays at the
//! round's last tick. Where it lands is drawn when it starts to spin and is
//! told to everyone then (`EV_WHEEL_SPIN`), so every screen draws the same
//! wheel coming to rest; the bets are already shut.
//!
//! **A slot machine** is one player's at a time: a spin costs
//! [`SLOT_COST`] × a multiplier of 1 to [`SLOT_MULT_MAX`], the reels are
//! drawn as it starts (`EV_SLOTS` tells the spinner), and it pays when it
//! stops [`SLOT_SPIN_TICKS`] later. Only three of a kind pays; any other
//! spin pays nothing.
//!
//! Every draw mixes in the world's secret (`mech::ArcState::salt`), which no
//! client is ever sent, so nobody can see a spin coming from the seed.
//!
//! Stakes on the wheel and a spin in progress are held here, hashed, and not
//! saved: a restart mid-round loses them to the house.

use crate::craft::{inv_count, inv_take};
use crate::limits::{MAX_PLAYERS, TICK_HZ};
use crate::rng::splitmix64;
use crate::town::{self, Town};
use crate::world::{
    EventQueue, Player, EV_SLOTS, EV_SLOTS_REFUSED, EV_WHEEL_BET, EV_WHEEL_REFUSED,
};

/// How far from a terminal's or a machine's spot a player may play, metres
/// (planar).
pub const CASINO_REACH_M: f32 = 2.0;

/// The wheel's colours, as Rust names them (`HitNumber.ColorToMultiplier`):
/// yellow, green, blue, purple, red.
pub const WHEEL_COLOURS: usize = 5;
/// What a win on each colour pays over the stake: the stake comes back with
/// this many times it on top (`slot.amount += slot.amount * multiplier`).
pub const WHEEL_PAYS: [u32; WHEEL_COLOURS] = [1, 3, 5, 10, 20];
/// The wheel's segments in order round the rim, each a colour: Rust's
/// 25, twelve yellow, six green, four blue, two purple and one red. Every
/// segment is equally likely.
pub const WHEEL: [u8; 25] = [
    4, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 2, 0, 1, 0, 2, 0, 3, 0, 1, 0, 2, 1, 0, 0,
];
pub const WHEEL_SEGMENTS: usize = WHEEL.len();
/// Rust's betting window: the terminals open 45 s before each spin.
pub const WHEEL_OPEN_TICKS: u64 = 45 * TICK_HZ as u64;
/// How long the wheel turns before it pays: Rust's 13–18 s at speed and its
/// slow-down, as one fixed time so every screen lands together.
pub const WHEEL_SPIN_TICKS: u64 = 15 * TICK_HZ as u64;
pub const WHEEL_ROUND_TICKS: u64 = WHEEL_OPEN_TICKS + WHEEL_SPIN_TICKS;
/// The most junk one player may have on one colour in one round. Rust takes
/// a stack (1,000 scrap); a tenth of that keeps one lucky spin from minting
/// a fortune (100 on red pays 2,100).
pub const WHEEL_STAKE_MAX: u32 = 100;

/// Rust's slot machine: a spin costs 2 scrap at ×1, and the multiplier runs
/// 1 to 5 (`Mathf.Clamp(..., 1, 5)`).
pub const SLOT_COST: u32 = 2;
pub const SLOT_MULT_MAX: u8 = 5;
/// Rust's `SpinDuration`.
pub const SLOT_SPIN_TICKS: u64 = 2 * TICK_HZ as u64;
/// Rust's reel symbols, in its order. Junk stands in for scrap and the
/// gate for the Rust logo.
pub const SYM_JUNK: u8 = 0;
pub const SYM_ROPE: u8 = 1;
pub const SYM_APPLE: u8 = 2;
pub const SYM_LOWGRADE: u8 = 3;
pub const SYM_WOOD: u8 = 4;
pub const SYM_BANDAGE: u8 = 5;
pub const SYM_CHARCOAL: u8 = 6;
pub const SYM_GUNPOWDER: u8 = 7;
pub const SYM_GATE: u8 = 8;
pub const SYM_MEAT: u8 = 9;
pub const SYM_HAMMER: u8 = 10;
pub const SYM_SULFUR: u8 = 11;
pub const SYM_GEARS: u8 = 12;
pub const SYM_FRAGS: u8 = 13;
pub const SYM_CLOTH: u8 = 14;
pub const SYM_SEVEN: u8 = 15;
pub const SLOT_SYMBOLS: usize = 16;
/// Each reel's stops by symbol (Rust's `VirtualFaces`; its own weights are
/// not public, so these are ours). All three reels share them. Charcoal and
/// wood are the common wins: some triple lands about one spin in 13.
pub const SLOT_WEIGHTS: [u32; SLOT_SYMBOLS] = [1, 7, 2, 3, 13, 2, 26, 1, 1, 1, 1, 2, 1, 1, 1, 1];
pub const SLOT_STOPS: u32 = {
    let mut sum = 0;
    let mut i = 0;
    while i < SLOT_SYMBOLS {
        sum += SLOT_WEIGHTS[i];
        i += 1;
    }
    sum
};
/// Three of a kind pays this much junk at ×1, by symbol; the rarer the
/// symbol, the more. Every symbol's triple pays, so three of anything wins.
pub const SLOT_TRIPLE: [u32; SLOT_SYMBOLS] = [
    500, 150, 300, 200, 60, 300, 15, 500, 500, 500, 500, 300, 500, 500, 500, 1000,
];
/// Bits a symbol takes in the packed reels (`EV_SLOTS.c`).
pub const REEL_BITS: u32 = 4;

/// Refusals, `EV_WHEEL_REFUSED.b` and `EV_SLOTS_REFUSED.b`.
/// No such colour, machine or stake, no coin, or no town.
pub const REFUSE_CA_KIND: u32 = 1;
/// Not at a terminal or the machine.
pub const REFUSE_CA_REACH: u32 = 2;
/// The wheel is spinning, or the machine is.
pub const REFUSE_CA_BUSY: u32 = 3;
/// The player carries less junk than the stake.
pub const REFUSE_CA_FUNDS: u32 = 4;
/// That colour already holds [`WHEEL_STAKE_MAX`] of the player's junk.
pub const REFUSE_CA_CAP: u32 = 5;
pub const REFUSE_CA_MAX: u32 = REFUSE_CA_CAP;

/// Draw channels, so a wheel spin and a reel never share a draw.
const CH_WHEEL: u64 = 0x5748_4545_4C00_0001;
const CH_SLOTS: u64 = 0x534C_4F54_5300_0002;

/// One player's stakes on this round of the wheel, by colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WheelSeat {
    pub id: u32,
    pub stakes: [u16; WHEEL_COLOURS],
}

impl WheelSeat {
    pub fn staked(&self) -> u32 {
        self.stakes.iter().map(|&s| s as u32).sum()
    }

    pub fn hash_bytes(&self) -> [u8; 4 + 2 * WHEEL_COLOURS] {
        let mut b = [0u8; 4 + 2 * WHEEL_COLOURS];
        b[..4].copy_from_slice(&self.id.to_le_bytes());
        for (c, s) in self.stakes.iter().enumerate() {
            b[4 + 2 * c..6 + 2 * c].copy_from_slice(&s.to_le_bytes());
        }
        b
    }
}

/// A slot machine's spin in progress: whose, what it pays, when it stops.
/// `id == 0` is an idle machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlotSpin {
    pub id: u32,
    pub pay: u32,
    pub done_at: u64,
}

impl SlotSpin {
    pub fn hash_bytes(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[..4].copy_from_slice(&self.id.to_le_bytes());
        b[4..8].copy_from_slice(&self.pay.to_le_bytes());
        b[8..].copy_from_slice(&self.done_at.to_le_bytes());
        b
    }
}

/// The wheel's seats and the machines' spins. One seat a player at most, so
/// the seats can never run out.
#[derive(Clone, Copy, Debug)]
pub struct Casino {
    pub seats: [WheelSeat; MAX_PLAYERS],
    pub slots: [SlotSpin; town::SLOT_SPOTS.len()],
}

impl Default for Casino {
    fn default() -> Self {
        Self {
            seats: [WheelSeat::default(); MAX_PLAYERS],
            slots: [SlotSpin::default(); town::SLOT_SPOTS.len()],
        }
    }
}

impl Casino {
    /// Everyone who has junk on this round.
    pub fn seated(&self) -> impl Iterator<Item = &WheelSeat> {
        self.seats.iter().filter(|s| s.id != 0)
    }
}

/// Where tick `tick` falls in its wheel round, and which round it is.
pub fn wheel_phase(tick: u64) -> (u64, u64) {
    (tick / WHEEL_ROUND_TICKS, tick % WHEEL_ROUND_TICKS)
}

/// Whether the terminals take bets at `tick`.
pub fn wheel_open(tick: u64) -> bool {
    wheel_phase(tick).1 < WHEEL_OPEN_TICKS
}

/// A spin's draw: a PRF of the salt (`rng::sip24`), not a splitmix mix of
/// it. Every wheel result goes to every client and a slot pull shows ~12
/// bits on a tick the spinner picks; splitmix inverts, so a mixed salt came
/// back out of enough spins — and the salt keys the cone, the loot and the
/// locks too. An unsalted world (the fixtures) draws as it always did.
fn draw(seed: u64, salt: u64, channel: u64, a: u64, b: u64) -> u64 {
    if salt == 0 {
        splitmix64(splitmix64(splitmix64(seed ^ channel) ^ a) ^ b)
    } else {
        crate::rng::sip24(salt, seed ^ channel, a, b)
    }
}

/// `[0, bound)` from a draw, without modulo bias (Lemire, `rng::next_bounded`).
fn bounded(h: u64, bound: u32) -> u32 {
    (((h >> 32) * bound as u64) >> 32) as u32
}

/// The segment round `round` lands on.
pub fn wheel_segment(seed: u64, salt: u64, round: u64) -> u8 {
    bounded(draw(seed, salt, CH_WHEEL, round, 0), WHEEL_SEGMENTS as u32) as u8
}

/// What a seat's stakes pay when the wheel lands on `segment`: the stake on
/// its colour, back with the colour's multiple on top. Everything else is
/// lost.
pub fn wheel_paid(seat: &WheelSeat, segment: u8) -> u32 {
    let c = WHEEL[segment as usize % WHEEL_SEGMENTS] as usize;
    seat.stakes[c] as u32 * (WHEEL_PAYS[c] + 1)
}

/// The share of a stake on colour `c` the wheel gives back on average, per
/// mille (Rust's: 960, 960, 960, 880, 840).
pub fn wheel_return_pm(c: usize) -> u32 {
    let hits = WHEEL.iter().filter(|&&s| s as usize == c).count() as u32;
    hits * (WHEEL_PAYS[c] + 1) * 1000 / WHEEL_SEGMENTS as u32
}

/// One reel's symbol for a draw.
fn reel(h: u64) -> u8 {
    let mut at = bounded(h, SLOT_STOPS);
    for (s, &w) in SLOT_WEIGHTS.iter().enumerate() {
        if at < w {
            return s as u8;
        }
        at -= w;
    }
    (SLOT_SYMBOLS - 1) as u8
}

/// The three reels of the spin `id` starts on machine `m` at `tick`.
pub fn slot_reels(seed: u64, salt: u64, tick: u64, m: usize, id: u32) -> [u8; 3] {
    let at = (m as u64) << 32 | id as u64;
    core::array::from_fn(|r| reel(draw(seed, salt, CH_SLOTS ^ r as u64, tick, at)))
}

/// What three reels pay at ×1: a triple's prize, and nothing else.
pub fn slot_pay(reels: [u8; 3]) -> u32 {
    let [a, b, c] = reels.map(|s| s as usize % SLOT_SYMBOLS);
    if a == b && b == c {
        SLOT_TRIPLE[a]
    } else {
        0
    }
}

/// The reels packed for the wire, low reel first.
pub fn pack_reels(reels: [u8; 3]) -> u32 {
    reels
        .iter()
        .enumerate()
        .map(|(i, &s)| (s as u32 & 0xF) << (REEL_BITS * i as u32))
        .sum()
}

pub fn unpack_reels(packed: u32) -> [u8; 3] {
    core::array::from_fn(|i| ((packed >> (REEL_BITS * i as u32)) & 0xF) as u8)
}

/// The share of what a slot machine takes that it gives back on average, per
/// mille — exact over every stop of the three reels.
pub fn slot_return_pm() -> u32 {
    let n = SLOT_STOPS as u64;
    let mut paid = 0u64;
    for (s, &w) in SLOT_WEIGHTS.iter().enumerate() {
        paid += (w as u64).pow(3) * SLOT_TRIPLE[s] as u64;
    }
    (paid * 1000 / (n * n * n * SLOT_COST as u64)) as u32
}

/// Whether the player stands at one of `spots`, within reach.
fn at_spot(
    p: &Player,
    town: &Town,
    spot: impl Fn(&Town, usize) -> Option<(f32, f32)>,
    k: usize,
) -> bool {
    let Some((sx, sz)) = spot(town, k) else {
        return false;
    };
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    let (dx, dz) = (px - sx, pz - sz);
    dx * dx + dz * dz <= CASINO_REACH_M * CASINO_REACH_M
}

/// Put `stake` junk on colour `colour` for this round. Refusals are events.
#[allow(clippy::too_many_arguments)]
pub fn bet(
    casino: &mut Casino,
    town: &Town,
    coin: u16,
    tick: u64,
    p: &mut Player,
    colour: u8,
    stake: u16,
    events: &mut EventQueue,
) {
    let pid = p.id;
    let refuse = |events: &mut EventQueue, code: u32| {
        events.push(
            EV_WHEEL_REFUSED,
            pid,
            code,
            (colour as u32) << 16 | stake as u32,
        );
    };
    let c = colour as usize;
    if c >= WHEEL_COLOURS
        || stake == 0
        || coin == crate::deploy::NO_SHOP_COIN
        || !town.live
        || p.dead
        || p.sleeping
        || p.wounded
    {
        return refuse(events, REFUSE_CA_KIND);
    }
    if !(0..town::WHEEL_TERMINALS.len()).any(|k| at_spot(p, town, town::wheel_terminal_world, k)) {
        return refuse(events, REFUSE_CA_REACH);
    }
    if !wheel_open(tick) {
        return refuse(events, REFUSE_CA_BUSY);
    }
    let seat = casino
        .seats
        .iter()
        .position(|s| s.id == pid)
        .or_else(|| casino.seats.iter().position(|s| s.id == 0));
    let Some(seat) = seat else {
        return refuse(events, REFUSE_CA_KIND);
    };
    let total = casino.seats[seat].stakes[c] as u32 + stake as u32;
    if total > WHEEL_STAKE_MAX {
        return refuse(events, REFUSE_CA_CAP);
    }
    if inv_count(&p.inv, coin) < stake as u32 {
        return refuse(events, REFUSE_CA_FUNDS);
    }
    inv_take(&mut p.inv, coin, stake as u32);
    let s = &mut casino.seats[seat];
    s.id = pid;
    s.stakes[c] = total as u16;
    events.push(EV_WHEEL_BET, pid, colour as u32, total);
}

/// Pull slot machine `m`'s lever at ×`mult`. Refusals are events.
#[allow(clippy::too_many_arguments)]
pub fn spin(
    casino: &mut Casino,
    town: &Town,
    coin: u16,
    seed: u64,
    salt: u64,
    tick: u64,
    p: &mut Player,
    m: u8,
    mult: u8,
    events: &mut EventQueue,
) {
    let pid = p.id;
    let refuse = |events: &mut EventQueue, code: u32| {
        events.push(EV_SLOTS_REFUSED, pid, code, (m as u32) << 8 | mult as u32);
    };
    let k = m as usize;
    if k >= town::SLOT_SPOTS.len()
        || mult == 0
        || mult > SLOT_MULT_MAX
        || coin == crate::deploy::NO_SHOP_COIN
        || !town.live
        || p.dead
        || p.sleeping
        || p.wounded
    {
        return refuse(events, REFUSE_CA_KIND);
    }
    if !at_spot(p, town, town::slot_world, k) {
        return refuse(events, REFUSE_CA_REACH);
    }
    if casino.slots[k].id != 0 {
        return refuse(events, REFUSE_CA_BUSY);
    }
    let cost = SLOT_COST * mult as u32;
    if inv_count(&p.inv, coin) < cost {
        return refuse(events, REFUSE_CA_FUNDS);
    }
    inv_take(&mut p.inv, coin, cost);
    let reels = slot_reels(seed, salt, tick, k, pid);
    let pay = slot_pay(reels) * mult as u32;
    casino.slots[k] = SlotSpin {
        id: pid,
        pay,
        done_at: tick + SLOT_SPIN_TICKS,
    };
    events.push(
        EV_SLOTS,
        pid,
        (m as u32) << 8 | mult as u32,
        pack_reels(reels) | pay << (3 * REEL_BITS),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wheel_is_rusts_twenty_five() {
        let count = |c: u8| WHEEL.iter().filter(|&&s| s == c).count();
        assert_eq!(
            [count(0), count(1), count(2), count(3), count(4)],
            [12, 6, 4, 2, 1]
        );
    }

    /// The house wins every bet on average, on both games — the economy's
    /// guard. A change to a weight or a prize that tips one over fails here.
    #[test]
    fn the_house_keeps_an_edge_on_every_bet() {
        let wheel: Vec<u32> = (0..WHEEL_COLOURS).map(wheel_return_pm).collect();
        assert_eq!(wheel, [960, 960, 960, 880, 840], "Rust's wheel");
        let slots = slot_return_pm();
        assert!((850..920).contains(&slots), "slots return {slots}‰");
        assert_eq!(SLOT_STOPS, 64);
    }

    /// A long run of seeded spins lands near the exact returns: the draw is
    /// as fair as the tables say, with no colour or symbol favoured.
    #[test]
    fn spins_land_as_often_as_the_tables_say() {
        let mut hits = [0u32; WHEEL_SEGMENTS];
        for round in 0..50_000 {
            hits[wheel_segment(7, 99, round) as usize] += 1;
        }
        for h in hits {
            assert!((1_800..2_200).contains(&h), "{hits:?}");
        }
        let (mut cost, mut paid) = (0u64, 0u64);
        for t in 0..400_000u64 {
            cost += SLOT_COST as u64;
            paid += slot_pay(slot_reels(7, 99, t, (t % 4) as usize, 5)) as u64;
        }
        let pm = paid * 1000 / cost;
        assert!(pm.abs_diff(slot_return_pm() as u64) < 30, "{pm}‰");
    }

    /// Only three of a kind pays, and every triple pays more than the spin
    /// cost. A low grade on one reel once paid on its own, and a spin with
    /// no match handed the player junk.
    #[test]
    fn only_three_of_a_kind_pays() {
        assert_eq!(slot_pay([SYM_SEVEN; 3]), 1000);
        assert_eq!(slot_pay([SYM_LOWGRADE, SYM_WOOD, SYM_CHARCOAL]), 0);
        assert_eq!(slot_pay([SYM_LOWGRADE, SYM_LOWGRADE, SYM_WOOD]), 0);
        for a in 0..SLOT_SYMBOLS as u8 {
            for b in 0..SLOT_SYMBOLS as u8 {
                for c in 0..SLOT_SYMBOLS as u8 {
                    let pay = slot_pay([a, b, c]);
                    if a == b && b == c {
                        assert!(pay > SLOT_COST, "three of {a} pays {pay}");
                    } else {
                        assert_eq!(pay, 0, "{a} {b} {c} is no match");
                    }
                }
            }
        }
        let r = [SYM_SEVEN, SYM_GATE, SYM_JUNK];
        assert_eq!(unpack_reels(pack_reels(r)), r);
    }

    #[test]
    fn the_salt_moves_every_spin() {
        let same = (0..64)
            .filter(|&r| wheel_segment(7, 1, r) == wheel_segment(7, 2, r))
            .count();
        assert!(same < 16, "{same} of 64 rounds ignored the salt");
    }
}
