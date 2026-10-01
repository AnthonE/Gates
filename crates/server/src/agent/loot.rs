//! Loot runs: the road's barrels, the crates at the landmarks and the
//! haven, the caches at the waystations — where a player gets the metal
//! fragments, fuel and parts nothing else gives before a furnace stands.
//!
//! A barrel is smashed like a node, a few swings of whatever is in hand,
//! and what falls out is picked up off the ground one stack per press
//! (`encode_action_pickup`, the take key). A crate or a cache is opened
//! with `E` (a world container, `CONT_WORLD`) and emptied into the pack
//! stack by stack (`encode_action_move` with its panel open), then shut.
//!
//! Where they are is what the eyes found ([`Loot::saw`], from the
//! explorer's cell scan: in the cone, with a clear line) and the map every
//! player holds: the haven, the waystations and the landmarks are drawn on
//! it, and the crates stand where they always stand there ([`places`]).
//! A container it emptied and a place it found bare are remembered for the
//! refill window, so a run does not walk back to an empty crate.
//! [`LootJob`] is one run; `explorer.rs` drives it and sends the verbs.

use client_core::core::ClientCore;
use sim_core::gather::{cell_key, RESPAWN_MIN_TICKS};
use sim_core::inventory::CONT_WORLD;
use sim_core::landmark;
use sim_core::limits::{HOTBAR_SLOTS, INV_SLOTS, TICK_HZ};
use sim_core::terrain::{Haven, Occupant, Slot};

/// Containers seen and remembered.
pub const SPOT_ROWS: usize = 16;
/// Containers emptied, and places found bare, remembered.
pub const EMPTIED_ROWS: usize = 32;
pub const VISITED_ROWS: usize = 8;
/// An emptied container, or a place found bare, is worth another walk
/// after the shortest refill (`gather::RESPAWN_MIN_TICKS`).
pub const EMPTY_TICKS: u32 = RESPAWN_MIN_TICKS as u32;
/// How long a container seen is remembered.
pub const SPOT_KEEP_TICKS: u32 = 10 * 60 * TICK_HZ;
/// A place on the map this near is worth a loot run.
pub const TRIP_M: f32 = 600.0;
/// Within this of a place's middle, it has been reached.
pub const PLACE_NEAR_M: f32 = 16.0;
/// A run goes on to the next container this near; farther is another run.
pub const NEXT_SPOT_M: f32 = 80.0;
/// Free pack slots a run wants before it sets out.
pub const MIN_FREE_SLOTS: u8 = 2;
/// A run that came to nothing is not offered again for this long.
pub const RETRY_TICKS: u32 = 60 * TICK_HZ;
/// Containers one run takes before it reports back.
pub const MAX_STOPS: u8 = 6;
/// Ground items passed over in one run (presses that took nothing).
pub const SKIP_ROWS: usize = 6;

/// What a container is to a player: smashed or opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prize {
    Barrel,
    /// A crate or a cache: a world container with a lid.
    Crate,
}

impl Prize {
    pub fn of(occupant: Occupant) -> Option<Prize> {
        match occupant {
            Occupant::BarrelSlot => Some(Prize::Barrel),
            Occupant::CrateSlot | Occupant::CacheSlot => Some(Prize::Crate),
            _ => None,
        }
    }
}

/// One container the eyes found.
#[derive(Clone, Copy, Debug)]
pub struct Spot {
    pub cx: u16,
    pub cz: u16,
    pub slot: Slot,
    pub prize: Prize,
    /// When it was last in sight.
    pub seen: u32,
}

impl Spot {
    /// Its scatter cell's key: the barrel's harvested bit, the crate's
    /// container handle.
    pub fn key(&self) -> u32 {
        cell_key(self.cx, self.cz)
    }

    pub fn at(&self) -> [f32; 2] {
        [self.slot.x, self.slot.z]
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LootStats {
    pub runs: u64,
    pub barrels: u64,
    pub opened: u64,
    pub moves: u64,
    pub pickups: u64,
    pub emptied: u64,
    pub places: u64,
}

/// What this body knows of the island's loot.
#[derive(Clone, Copy, Debug)]
pub struct Loot {
    spots: [Option<Spot>; SPOT_ROWS],
    /// Containers (by key) and places (by `PLACE_KEY | index`) emptied or
    /// found bare, and when; a ring.
    emptied: [Option<(u32, u32)>; EMPTIED_ROWS],
    head: usize,
    failed: Option<u32>,
    pub stats: LootStats,
}

/// Places share the emptied ring with containers, keyed apart: a scatter
/// cell key never sets this bit (`gather::cell_key` packs two 16-bit
/// cells, and the island is far smaller than that).
const PLACE_KEY: u32 = 1 << 31;

impl Default for Loot {
    fn default() -> Self {
        Self::new()
    }
}

impl Loot {
    pub const fn new() -> Self {
        Self {
            spots: [None; SPOT_ROWS],
            emptied: [None; EMPTIED_ROWS],
            head: 0,
            failed: None,
            stats: LootStats {
                runs: 0,
                barrels: 0,
                opened: 0,
                moves: 0,
                pickups: 0,
                emptied: 0,
                places: 0,
            },
        }
    }

    /// A new session: the island is seen afresh.
    pub fn clear(&mut self) {
        let stats = self.stats;
        *self = Self::new();
        self.stats = stats;
    }

    /// A container in sight now.
    pub fn saw(&mut self, spot: Spot) {
        let key = spot.key();
        let row = self
            .spots
            .iter()
            .position(|s| s.is_some_and(|s| s.key() == key))
            .or_else(|| self.spots.iter().position(Option::is_none))
            .unwrap_or_else(|| {
                // Full: the one seen longest ago goes.
                let mut oldest = 0;
                for (i, s) in self.spots.iter().enumerate() {
                    if let (Some(s), Some(o)) = (s, self.spots[oldest]) {
                        if s.seen.wrapping_sub(o.seen) > u32::MAX / 2 {
                            oldest = i;
                        }
                    }
                }
                oldest
            });
        self.spots[row] = Some(spot);
    }

    fn remember(&mut self, key: u32, tick: u32) {
        if let Some(row) = self
            .emptied
            .iter_mut()
            .find(|e| e.is_some_and(|(k, _)| k == key))
        {
            *row = Some((key, tick));
            return;
        }
        self.emptied[self.head] = Some((key, tick));
        self.head = (self.head + 1) % EMPTIED_ROWS;
    }

    fn recalls(&self, key: u32, tick: u32) -> bool {
        self.emptied
            .iter()
            .flatten()
            .any(|&(k, at)| k == key && tick.wrapping_sub(at) < EMPTY_TICKS)
    }

    /// This container is empty, or not worth more of this run: not again
    /// until it may have refilled.
    pub fn emptied(&mut self, key: u32, tick: u32) {
        self.remember(key, tick);
        self.stats.emptied += 1;
    }

    /// A place on the map was reached and looked over.
    pub fn visited(&mut self, place: u16, tick: u32) {
        self.remember(PLACE_KEY | u32::from(place), tick);
        self.stats.places += 1;
    }

    /// A run came to nothing.
    pub fn failed(&mut self, tick: u32) {
        self.failed = Some(tick);
    }

    /// A run came to nothing a moment ago.
    pub fn held(&self, tick: u32) -> bool {
        self.failed
            .is_some_and(|at| tick.wrapping_sub(at) < RETRY_TICKS)
    }

    /// The containers known and worth a walk: seen lately, not emptied,
    /// and (a barrel) still standing.
    pub fn live<'a>(&'a self, core: &'a ClientCore, tick: u32) -> impl Iterator<Item = Spot> + 'a {
        self.spots.iter().flatten().copied().filter(move |s| {
            tick.wrapping_sub(s.seen) < SPOT_KEEP_TICKS
                && !self.recalls(s.key(), tick)
                && !(s.prize == Prize::Barrel && core.harvested.contains(s.key()))
        })
    }

    /// The nearest container worth a walk from `at`, within `within`.
    pub fn nearest(&self, core: &ClientCore, at: [f32; 2], within: f32, tick: u32) -> Option<Spot> {
        let mut best: Option<(f32, Spot)> = None;
        for s in self.live(core, tick) {
            let d = (s.slot.x - at[0]).hypot(s.slot.z - at[1]);
            if d <= within && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, s));
            }
        }
        best.map(|(_, s)| s)
    }

    /// The nearest place on the map within [`TRIP_M`] not found bare
    /// lately: `(index, middle)`.
    pub fn place(&self, haven: &Haven, at: [f32; 2], tick: u32) -> Option<(u16, [f32; 2])> {
        let mut best: Option<(f32, u16, [f32; 2])> = None;
        for (i, p) in places(haven) {
            let d = (p[0] - at[0]).hypot(p[1] - at[1]);
            if d <= TRIP_M
                && best.is_none_or(|(b, ..)| d < b)
                && !self.recalls(PLACE_KEY | u32::from(i), tick)
            {
                best = Some((d, i, p));
            }
        }
        best.map(|(_, i, p)| (i, p))
    }
}

/// The places on the map where crates stand, `(index, middle)`: the haven
/// (its pad's crates), the waystations and inland sites (their caches),
/// and the landmarks whose kind keeps crates. A player's map draws every
/// one of them; it is the island's layout, not anyone's doing.
pub fn places(haven: &Haven) -> impl Iterator<Item = (u16, [f32; 2])> + '_ {
    let pad = std::iter::once((0u16, [haven.x, haven.z]));
    let minor = haven
        .minor
        .iter()
        .enumerate()
        .filter(|(_, w)| w.live)
        .map(|(i, w)| (1 + i as u16, [w.x, w.z]));
    let base = 1 + haven.minor.len() as u16;
    let marks = haven
        .marks
        .iter()
        .enumerate()
        .filter(|(_, m)| m.live && !landmark::anchors(m.kind).is_empty())
        .map(move |(i, m)| (base + i as u16, [m.x, m.z]));
    pad.chain(minor).chain(marks)
}

/// The next stack to take out of the open world container: its slot, the
/// pack slot it goes to (the pack before the belt; onto a stack of the
/// same item with room, else an empty slot) and how many.
pub fn take_plan(core: &ClientCore) -> Option<(u8, u8, u16)> {
    if core.cont_kind != CONT_WORLD {
        return None;
    }
    let order = (HOTBAR_SLOTS..INV_SLOTS).chain(0..HOTBAR_SLOTS);
    for (from, stack) in core.cont.iter().enumerate() {
        if stack.count == 0 {
            continue;
        }
        let max = core.catalog.row(usize::from(stack.item)).stack_max.max(1);
        let onto = order.clone().find(|&i| {
            let s = core.inv[i];
            s.count > 0 && s.item == stack.item && s.count < max
        });
        let to = onto.or_else(|| order.clone().find(|&i| core.inv[i].count == 0));
        if let Some(to) = to {
            let room = max - core.inv[to].count.min(max);
            return Some((from as u8, to as u8, stack.count.min(room)));
        }
    }
    None
}

/// One stack more of `item` fits the pack.
pub fn room_for(core: &ClientCore, item: u16) -> bool {
    let max = core.catalog.row(usize::from(item)).stack_max;
    core.inv[..INV_SLOTS]
        .iter()
        .any(|s| s.count == 0 || (s.item == item && s.count < max))
}

/// Empty pack and belt slots.
pub fn free_slots(core: &ClientCore) -> u8 {
    core.inv[..INV_SLOTS]
        .iter()
        .filter(|s| s.count == 0)
        .count() as u8
}

/// What an open press is waiting on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lid {
    /// Eyes on it since.
    Aiming(u32),
    /// The open went out; the panel has not shown it yet.
    Opening(u32),
    /// Its panel is open.
    Open,
    /// A move of this many went out, and its answer is awaited.
    Moving(u32, u16),
    /// A move was answered: the panel catches up before the next.
    Settling(u32),
}

/// One loot run: the container in hand, the place on the map walked to,
/// and the presses in flight.
#[derive(Clone, Copy, Debug, Default)]
pub struct LootJob {
    /// The container being taken.
    pub target: Option<Spot>,
    /// The place on the map walked to while no container is known.
    pub place: Option<(u16, [f32; 2])>,
    /// When the place was reached; the look round there ends the run.
    pub arrived: Option<u32>,
    /// The barrel that broke and when: its stacks are on the ground.
    pub smashed: Option<(Spot, u32)>,
    /// The ground stack last pressed for, when, and how often.
    pub pick: Option<(u32, u32, u8)>,
    pub skip: [u32; SKIP_ROWS],
    pub skipped: usize,
    /// The nearest the walk to the target has come, and when.
    pub best: Option<(f32, u32)>,
    /// Swinging at the barrel since.
    pub swinging: Option<u32>,
    pub lid: Option<Lid>,
    /// The panel showed this container since the open went out.
    pub fresh: bool,
    /// A move's answer: refused or not.
    pub moved: Option<bool>,
    pub tries: u8,
    pub stops: u8,
    /// Units this run put in the pack by moves (pickups count as toasts).
    pub taken: u32,
}

impl LootJob {
    /// The world container `handle`'s panel showed it.
    pub fn on_panel(&mut self, handle: u32) {
        if self.target.is_some_and(|t| t.key() == handle) {
            self.fresh = true;
        }
    }

    /// A move out of the panel was answered.
    pub fn on_moved(&mut self, refused: bool) {
        if matches!(self.lid, Some(Lid::Moving(..))) {
            self.moved = Some(refused);
        }
    }

    /// Done with the container in hand: the next one, or the place.
    pub fn next(&mut self) {
        self.target = None;
        self.smashed = None;
        self.pick = None;
        self.best = None;
        self.swinging = None;
        self.lid = None;
        self.fresh = false;
        self.moved = None;
        self.tries = 0;
        self.stops = self.stops.saturating_add(1);
    }

    /// Passed over for the rest of the run.
    pub fn skips(&self, id: u32) -> bool {
        self.skip[..self.skipped.min(SKIP_ROWS)].contains(&id)
    }

    pub fn skip_item(&mut self, id: u32) {
        self.skip[self.skipped % SKIP_ROWS] = id;
        self.skipped += 1;
        self.pick = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 20260731;

    fn spot(cx: u16, cz: u16, prize: Prize, seen: u32) -> Spot {
        Spot {
            cx,
            cz,
            slot: Slot {
                occupant: match prize {
                    Prize::Barrel => Occupant::BarrelSlot,
                    Prize::Crate => Occupant::CrateSlot,
                },
                x: f32::from(cx) * 8.0,
                y: 0.0,
                z: f32::from(cz) * 8.0,
                yaw: 0,
                scale: 1.0,
                species: 0,
            },
            prize,
            seen,
        }
    }

    #[test]
    fn emptied_containers_and_bare_places_wait_out_the_refill() {
        let core = ClientCore::new(SEED, 1, 0);
        let mut loot = Loot::new();
        loot.saw(spot(10, 10, Prize::Crate, 0));
        loot.saw(spot(12, 10, Prize::Barrel, 0));
        let near = loot.nearest(&core, [80.0, 80.0], 100.0, 1).unwrap();
        assert_eq!((near.cx, near.cz), (10, 10));
        loot.emptied(near.key(), 1);
        let next = loot.nearest(&core, [80.0, 80.0], 100.0, 2).unwrap();
        assert_eq!((next.cx, next.cz), (12, 10));
        assert!(
            loot.nearest(&core, [80.0, 80.0], 10.0, 2).is_none(),
            "too far"
        );
        // The crate is worth a look again once it may have refilled; a
        // container not seen for long is forgotten.
        assert_eq!(loot.live(&core, SPOT_KEEP_TICKS / 2).count(), 1);
        loot.saw(spot(10, 10, Prize::Crate, EMPTY_TICKS));
        assert_eq!(loot.live(&core, EMPTY_TICKS + 1).count(), 1);
        assert_eq!(loot.live(&core, EMPTY_TICKS + SPOT_KEEP_TICKS).count(), 0);

        let haven = sim_core::terrain::haven(SEED);
        let (i, p) = loot.place(&haven, [haven.x, haven.z], 0).unwrap();
        assert_eq!((i, p), (0, [haven.x, haven.z]), "the haven's own pad");
        loot.visited(i, 0);
        let (j, _) = loot.place(&haven, [haven.x, haven.z], 10).unwrap();
        assert_ne!(j, 0);
        assert_eq!(
            loot.place(&haven, [haven.x, haven.z], EMPTY_TICKS)
                .unwrap()
                .0,
            0
        );
        assert!(places(&haven).count() > 3, "the map has places on it");
    }
}
