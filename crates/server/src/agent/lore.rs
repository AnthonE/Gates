//! The ancients' stones and their locks, and the Black Ziggurat's card
//! doors, as a player meets them (`ARC.md` F6, F7; `sim_core::monument`).
//!
//! **Stones.** Where every stone stands is what its client was sent (the
//! arc drip); what one says arrives only when it is read there
//! ([`ReadJob`]: walked up to, `E`, the server's text). Reading teaches the
//! glyphs a stone names, and what it can make of a stone is only the glyphs
//! it knows ([`seen`]): the rest stay shapes, as they do on a player's
//! READ panel.
//!
//! **Locks.** A lock's answer is written on a stone somewhere else, digit
//! by digit ([`answer`]: a stone it read whose words end on as many digits
//! as the lock has dials, the digits glyphs it reads). With the answer, a
//! solve ([`SolveJob`]) walks to each dial in turn and turns it a notch a
//! press until it shows the answer's digit; the last turn pays the turner.
//! Anyone may turn any dial, so it reads the dials again before each press.
//!
//! **Card doors.** The ziggurat's green, blue and red doors each take the
//! card of their colour at the reader beside them (`sim_core::monument`).
//! A swipe ([`SwipeJob`]) walks to the reader, swipes, and walks in while
//! the door stands open; the room's crates are then the loot run's. A room
//! whose door has shut is left by its lever ([`ExitJob`]), which opens the
//! door with no card.

use crate::agent::build::item_named;
use crate::agent::intent::{yaw_toward, Intent, Look};
use crate::agent::route::{Route, Step};
use crate::agent::stash::carried;
use crate::mind::Why;
use client_core::arc::LoreView;
use client_core::core::ClientCore;
use protocol::{EntityState, ARC_INSCRIPTION, ARC_TEXT_BYTES};
use sim_core::limits::{MAX_DIALS, TICK_HZ};
use sim_core::lore::{LORE_REACH_M, OP_READ};
use sim_core::mech::{dial_spot, MechDef, DIAL_REACH_M, OP_TURN};
use sim_core::monument::{reader_world, Ziggurat, CARD_DOORS, SWIPE_REACH_M};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::terrain::Haven;

/// A stone this near is worth the walk to read it.
pub const READ_TRIP_M: f32 = 500.0;
/// Where it reads a stone from: this far inside the reach.
pub const READ_STAND_M: f32 = LORE_REACH_M - 1.5;
/// Where it turns a dial from: nearer it than to either neighbour.
pub const DIAL_STAND_M: f32 = 0.6;
/// Where it swipes from, and pulls a lever from.
pub const SWIPE_STAND_M: f32 = 1.0;
/// In a room, it stands this near the lever's spot.
pub const ROOM_STAND_M: f32 = 1.5;
/// Under this far, the last steps are walked straight.
const STRAIGHT_M: f32 = 2.5;
/// Turns one solve presses at most: every dial all the way round twice.
pub const MAX_TURNS: u16 = 2 * (MAX_DIALS as u16) * sim_core::mech::MAX_VALUES as u16;
/// An answer to a press comes within this long.
const VERDICT_TICKS: u32 = 3 * TICK_HZ;

/// The card each door takes, by its name in the catalog: what the cards
/// say of themselves ("Opens the green doors").
pub const CARD_NAMES: [&str; CARD_DOORS] = ["Green Keycard", "Blue Keycard", "Red Keycard"];

/// Stone `k` as it reads it, into `out`: a glyph it knows as its letter,
/// one it does not as `?`. The bytes filled; none before it was read.
pub fn seen(lore: &LoreView, k: usize, out: &mut [u8; ARC_TEXT_BYTES]) -> usize {
    let Some(i) = lore.inscriptions.get(k).filter(|i| i.known) else {
        return 0;
    };
    let n = usize::from(i.len).min(ARC_TEXT_BYTES);
    for (o, &ch) in out.iter_mut().zip(&i.text[..n]) {
        *o = if lore.reads(ch) { ch } else { b'?' };
    }
    n
}

/// This wipe's answer to lock `m`, as the stones it has read give it: one
/// whose words hold a run of exactly as many single digits as the lock has
/// dials, each a digit it reads and a notch the dial has. Two such stones
/// that disagree are settled by the one that names the lock (a word of its
/// name, read); otherwise there is no answer yet.
pub fn answer(lore: &LoreView, m: usize) -> Option<[u8; MAX_DIALS]> {
    let mech = lore.mechs.get(m).filter(|x| x.known)?;
    let n = usize::from(mech.n_dials);
    if n == 0 || n > MAX_DIALS {
        return None;
    }
    let name = mech.name().as_bytes();
    let mut found: Option<([u8; MAX_DIALS], bool)> = None;
    let mut clash = false;
    for (k, _) in lore.known_inscriptions() {
        let mut text = [0u8; ARC_TEXT_BYTES];
        let len = seen(lore, k, &mut text);
        let words = || text[..len].split(|&c| c == b' ').filter(|w| !w.is_empty());
        let mut run = [0u8; MAX_DIALS];
        let mut at = 0usize;
        let mut digits: Option<[u8; MAX_DIALS]> = None;
        let mut runs = 0;
        let mut close = |at: &mut usize, run: &[u8; MAX_DIALS]| {
            if *at == n {
                digits = Some(*run);
                runs += 1;
            }
            *at = 0;
        };
        for w in words() {
            match w {
                [d] if d.is_ascii_digit() && d - b'0' < mech.values => {
                    if at < MAX_DIALS {
                        run[at] = d - b'0';
                    }
                    at += 1;
                }
                _ => close(&mut at, &run),
            }
        }
        close(&mut at, &run);
        let Some(digits) = digits.filter(|_| runs == 1) else {
            continue;
        };
        let names = words().any(|w| w.len() >= 4 && name.split(|&c| c == b' ').any(|nw| nw == w));
        match found {
            None => found = Some((digits, names)),
            Some((d, _)) if d == digits => {}
            Some((_, false)) if names => {
                found = Some((digits, true));
                clash = false;
            }
            Some((_, true)) if !names => {}
            Some(_) => clash = true,
        }
    }
    found.filter(|_| !clash).map(|(d, _)| d)
}

/// The nearest stone it has not read, within `within`: `(k, where it
/// stands)`. A read goes to one within [`READ_TRIP_M`].
pub fn unread(core: &ClientCore, x: f32, z: f32, within: f32) -> Option<(u8, [f32; 3])> {
    let mut best: Option<(f32, u8, [f32; 3])> = None;
    for (k, i) in core.lore.known_inscriptions() {
        if i.len > 0 {
            continue;
        }
        let Some((sx, sy, sz)) = sim_core::spot::world(core.haven(), &i.spot) else {
            continue;
        };
        let d = (sx - x).hypot(sz - z);
        if d <= within && best.is_none_or(|b| d < b.0) {
            best = Some((d, k as u8, [sx, sy, sz]));
        }
    }
    best.map(|(_, k, p)| (k, p))
}

/// Stones it has read, and how many it knows of.
pub fn stones(core: &ClientCore) -> (u8, u8) {
    let mut read = 0u8;
    let mut all = 0u8;
    for (_, i) in core.lore.known_inscriptions() {
        all = all.saturating_add(1);
        if i.len > 0 {
            read = read.saturating_add(1);
        }
    }
    (read, all)
}

/// Where dial `d` of lock `m` stands, per what its client was sent.
pub fn dial_at(core: &ClientCore, m: usize, d: usize) -> Option<[f32; 3]> {
    let mech = core.lore.mechs.get(m).filter(|x| x.known)?;
    let def = MechDef {
        spot: mech.spot,
        dials: mech.n_dials,
        ..Default::default()
    };
    sim_core::spot::world(core.haven(), &dial_spot(&def, d)).map(|(x, y, z)| [x, y, z])
}

/// A lock worth solving now: known, armed, its answer read and its dials
/// not showing it. The nearest: `(m, answer)`.
pub fn solvable(core: &ClientCore, x: f32, z: f32) -> Option<(u8, [u8; MAX_DIALS])> {
    let mut best: Option<(f32, u8, [u8; MAX_DIALS])> = None;
    for (m, mech) in core.lore.known_mechs() {
        if mech.resting {
            continue;
        }
        let Some(want) = answer(&core.lore, m) else {
            continue;
        };
        let n = usize::from(mech.n_dials);
        if mech.dials[..n] == want[..n] {
            continue;
        }
        let Some(at) = dial_at(core, m, 0) else {
            continue;
        };
        let d = (at[0] - x).hypot(at[2] - z);
        if best.is_none_or(|b| d < b.0) {
            best = Some((d, m as u8, want));
        }
    }
    best.map(|(_, m, a)| (m, a))
}

/// What `E` would use among the people, stones and dials from here: the
/// human client's own rule (`client::ui::interact::resolve_lore`), the
/// nearest within the sim's reach, a dial first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aimed {
    None,
    Turn(u8, u8),
    Talk(u8),
    Read(u8),
}

pub fn e_lore(core: &ClientCore, x: f32, feet: f32, z: f32) -> Aimed {
    let haven = core.haven();
    let lore = &core.lore;
    let near = |spot: &sim_core::spot::Spot, reach: f32| -> Option<f32> {
        if !sim_core::spot::within(haven, spot, x, feet, z, reach) {
            return None;
        }
        let (wx, _, wz) = sim_core::spot::world(haven, spot)?;
        Some((wx - x) * (wx - x) + (wz - z) * (wz - z))
    };
    let mut dial: Option<(f32, u8, u8)> = None;
    for (m, mech) in lore.known_mechs() {
        let def = MechDef {
            spot: mech.spot,
            dials: mech.n_dials,
            ..Default::default()
        };
        for d in 0..usize::from(mech.n_dials) {
            if let Some(d2) = near(&dial_spot(&def, d), DIAL_REACH_M) {
                if dial.is_none_or(|b| d2 < b.0) {
                    dial = Some((d2, m as u8, d as u8));
                }
            }
        }
    }
    if let Some((_, m, d)) = dial {
        return Aimed::Turn(m, d);
    }
    let mut talk: Option<(f32, u8)> = None;
    for (k, s) in lore.known_speakers() {
        if let Some(d2) = near(&s.spot, LORE_REACH_M) {
            if talk.is_none_or(|b| d2 < b.0) {
                talk = Some((d2, k as u8));
            }
        }
    }
    let mut read: Option<(f32, u8)> = None;
    for (k, s) in lore.known_inscriptions() {
        if let Some(d2) = near(&s.spot, LORE_REACH_M) {
            if read.is_none_or(|b| d2 < b.0) {
                read = Some((d2, k as u8));
            }
        }
    }
    match (talk, read) {
        (Some((d2, k)), r) if r.is_none_or(|(rd, _)| d2 <= rd) => Aimed::Talk(k),
        (_, Some((_, k))) => Aimed::Read(k),
        _ => Aimed::None,
    }
}

/// The reader or lever `E` would use from here: the human client's rule
/// (`client::ui::interact::resolve_swipe`), the nearest within the sim's
/// reach on its own floor. `(door, lever)`.
pub fn e_swipe(zig: &Ziggurat, x: f32, feet: f32, z: f32) -> Option<(u8, bool)> {
    if !zig.live {
        return None;
    }
    let mut best: Option<(f32, u8, bool)> = None;
    for d in 0..CARD_DOORS {
        for inside in [false, true] {
            let Some((rx, ry, rz)) = reader_world(zig, d, inside) else {
                continue;
            };
            let d2 = (rx - x) * (rx - x) + (rz - z) * (rz - z);
            if d2 > SWIPE_REACH_M * SWIPE_REACH_M || (ry - feet).abs() > 2.0 {
                continue;
            }
            if best.is_none_or(|b| d2 < b.0) {
                best = Some((d2, d as u8, inside));
            }
        }
    }
    best.map(|(_, d, lever)| (d, lever))
}

/// Door `d` stands open, per what its client was sent.
pub fn door_open(core: &ClientCore, d: usize) -> bool {
    core.card_doors & (1 << d) != 0
}

/// The card door the pack holds the card for, the highest first: a card
/// is not used up, and each room's crate holds the next card, so the one
/// worth opening is the next one up the chain (green, blue, red), not the
/// room it has been in already. `(door, reader)`.
pub fn swipeable(core: &ClientCore) -> Option<(u8, [f32; 3])> {
    let zig = core.haven().ziggurat;
    (0..CARD_DOORS).rev().find_map(|d| {
        let held = item_named(core, CARD_NAMES[d]).is_some_and(|item| carried(core, item) > 0);
        if !held || door_open(core, d) {
            return None;
        }
        reader_world(&zig, d, false).map(|(x, y, z)| (d as u8, [x, y, z]))
    })
}

/// Up on the terraces on the level of door `d`'s path: a climb to it goes
/// on from here; anywhere else up there, down first.
pub fn on_path(zig: &Ziggurat, d: u8, x: f32, feet: f32, z: f32) -> bool {
    PATHS.get(usize::from(d)).is_some_and(|path| {
        path.iter()
            .skip(1)
            .any(|&w| (at_zig(zig, w)[1] - feet).abs() <= 2.5)
    }) && aloft(zig, x, feet, z)
        && room_of(zig, x, feet, z).is_none()
}

/// The card rooms in the ziggurat's own metres (`monument::BODY`): `(x0,
/// z0, x1, z1)` boxes, each room's hall and the passage to its door, and
/// the floor the room stands on.
type Room = ([(f32, f32, f32, f32); 2], f32);
const ROOMS: [Room; CARD_DOORS] = [
    ([(-8.0, -8.0, 8.0, 10.6), (-1.5, 10.0, 1.5, 10.6)], 0.0),
    ([(-6.0, -6.0, 6.0, 6.0), (6.0, -1.5, 22.6, 1.5)], 5.0),
    ([(-13.0, -7.0, -3.0, 7.0), (-9.5, 7.0, -6.5, 12.6)], 15.0),
];

/// The card room it stands in, if any.
pub fn room_of(zig: &Ziggurat, x: f32, feet: f32, z: f32) -> Option<u8> {
    if !zig.live {
        return None;
    }
    let (lx, lz) = sim_core::kit::to_local(&zig.placed(), x, z);
    let up = feet - zig.floor_y;
    ROOMS
        .iter()
        .position(|(boxes, floor)| {
            (up - floor).abs() <= 2.5
                && boxes
                    .iter()
                    .any(|&(x0, z0, x1, z1)| lx > x0 && lx < x1 && lz > z0 && lz < z1)
        })
        .map(|d| d as u8)
}

/// What a stone, a lock or a card door wants next. `explorer.rs` sends
/// the verbs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Visit {
    Go(Intent),
    /// The arc verb: a read, or a turn.
    Arc {
        op: u8,
        target: u8,
        arg: u8,
    },
    /// Swipe at card door `door` (a reader, or from inside its lever).
    Swipe(u8),
    Done,
    Fail(Why),
}

/// Walk to `to` and stop within `stop` (straight for the last metres).
/// `None` once there.
fn approach(
    core: &mut ClientCore,
    body: &EntityState,
    route: &mut Route,
    to: [f32; 3],
    stop: f32,
    tick: u32,
) -> Option<Result<Intent, Why>> {
    let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
    let d = (to[0] - x).hypot(to[2] - z);
    if d <= stop {
        return None;
    }
    let straight = Intent::walk(yaw_toward(to[0] - x, to[2] - z));
    if d <= STRAIGHT_M.max(stop + 0.5) {
        return Some(Ok(straight));
    }
    Some(
        match route.to(core, body, [to[0], to[2]], STRAIGHT_M - 0.5, false, tick) {
            step @ Step::Walk { .. } => Ok(step.walk().unwrap_or(straight)),
            Step::Wait | Step::Arrived => Ok(straight),
            Step::Blocked => Err(Why::Stuck),
        },
    )
}

/// Eyes on a point, standing.
fn look(p: [f32; 3]) -> Intent {
    Intent {
        look: Look::Point([p[0], p[1] + 1.0, p[2]]),
        ..Intent::IDLE
    }
}

fn feet(body: &EntityState) -> (f32, f32, f32) {
    (
        body.qx as f32 * POS_XZ_Q,
        body.qy as f32 * POS_Y_Q,
        body.qz as f32 * POS_XZ_Q,
    )
}

/// One stone read: walked to, `E`, its text.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReadJob {
    stone: Option<u8>,
    sent: Option<u32>,
    read: bool,
    refused: bool,
    /// Stones read.
    pub reads: u64,
}

impl ReadJob {
    /// A new read; the count of reads is kept.
    pub fn reset(&mut self) {
        *self = Self {
            reads: self.reads,
            ..Self::default()
        };
    }

    /// The server's words for a stone: the one read, if it is this one.
    pub fn on_text(&mut self, kind: u8, index: u8) {
        if kind == ARC_INSCRIPTION && self.stone == Some(index) && self.sent.is_some() {
            self.read = true;
        }
    }

    pub fn on_refused(&mut self, op: u8, target: u8) {
        if op == OP_READ && self.stone == Some(target) && self.sent.is_some() {
            self.refused = true;
        }
    }

    pub fn sent(&mut self, tick: u32) {
        self.sent = Some(tick);
    }

    pub fn step(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        route: &mut Route,
        tick: u32,
    ) -> Visit {
        if self.read {
            self.reads += 1;
            return Visit::Done;
        }
        if self.refused {
            return Visit::Fail(Why::Refused);
        }
        if let Some(sent) = self.sent {
            return if tick.wrapping_sub(sent) >= VERDICT_TICKS {
                Visit::Fail(Why::NoAnswer)
            } else {
                Visit::Go(Intent::IDLE)
            };
        }
        let (x, y, z) = feet(body);
        let (k, at) = match self.stone {
            Some(k) => {
                let Some(i) = core.lore.inscriptions.get(usize::from(k)) else {
                    return Visit::Fail(Why::NotFound);
                };
                match sim_core::spot::world(core.haven(), &i.spot) {
                    Some((sx, sy, sz)) => (k, [sx, sy, sz]),
                    None => return Visit::Fail(Why::NotFound),
                }
            }
            None => match unread(core, x, z, READ_TRIP_M) {
                Some((k, at)) => {
                    self.stone = Some(k);
                    (k, at)
                }
                None => return Visit::Fail(Why::NotFound),
            },
        };
        // Near, but another stone or a speaker is what `E` would take:
        // closer.
        let stand = if (at[0] - x).hypot(at[2] - z) <= READ_STAND_M {
            if e_lore(core, x, y, z) == Aimed::Read(k) {
                return Visit::Arc {
                    op: OP_READ,
                    target: k,
                    arg: 0,
                };
            }
            DIAL_STAND_M
        } else {
            READ_STAND_M
        };
        match approach(core, body, route, at, stand, tick) {
            Some(Ok(intent)) => Visit::Go(intent),
            Some(Err(why)) => Visit::Fail(why),
            None => Visit::Go(look(at)),
        }
    }
}

/// One lock solved: each dial in turn walked to and turned a notch a press
/// until it shows the answer's digit; the turn that shows the whole answer
/// pays the turner.
#[derive(Clone, Copy, Debug, Default)]
pub struct SolveJob {
    lock: Option<(u8, [u8; MAX_DIALS])>,
    /// A turn of dial `.1` went out at `.0`, the dial showing `.2` then.
    sent: Option<(u32, u8, u8)>,
    /// Seen resting since this tick, with no word it solved it.
    resting: Option<u32>,
    solved: bool,
    refused: bool,
    turns: u16,
    /// Locks solved.
    pub solves: u64,
}

impl SolveJob {
    /// A new solve; the count of solves is kept.
    pub fn reset(&mut self) {
        *self = Self {
            solves: self.solves,
            ..Self::default()
        };
    }

    /// The island heard a lock solved; this one, by this body?
    pub fn on_solved(&mut self, mech: u8, by: u32, me: u32) {
        if self.lock.is_some_and(|(m, _)| m == mech) && by == me {
            self.solved = true;
        }
    }

    pub fn on_refused(&mut self, op: u8, target: u8) {
        if op == OP_TURN && self.lock.is_some_and(|(m, _)| m == target) && self.sent.is_some() {
            self.refused = true;
        }
    }

    pub fn sent(&mut self, core: &ClientCore, dial: u8, tick: u32) {
        let shows = self
            .lock
            .and_then(|(m, _)| core.lore.mechs.get(usize::from(m)))
            .map_or(0, |m| m.dials[usize::from(dial)]);
        self.sent = Some((tick, dial, shows));
        self.turns = self.turns.saturating_add(1);
    }

    pub fn step(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        route: &mut Route,
        tick: u32,
    ) -> Visit {
        if self.solved {
            self.solves += 1;
            return Visit::Done;
        }
        if self.refused {
            return Visit::Fail(Why::Refused);
        }
        let (x, y, z) = feet(body);
        if self.lock.is_none() {
            self.lock = solvable(core, x, z);
        }
        let Some((m, want)) = self.lock else {
            return Visit::Fail(Why::NotFound);
        };
        let Some(mech) = core.lore.mechs.get(usize::from(m)).copied() else {
            return Visit::Fail(Why::NotFound);
        };
        // A turn in flight: its dial moves, or the answer is late.
        if let Some((sent, d, was)) = self.sent {
            if mech.dials[usize::from(d)] != was || mech.resting {
                self.sent = None;
            } else if tick.wrapping_sub(sent) >= VERDICT_TICKS {
                return Visit::Fail(Why::NoAnswer);
            } else {
                return Visit::Go(Intent::IDLE);
            }
        }
        // Resting: the last turn solved it and the word of it is on its
        // way, or someone else solved it.
        if mech.resting {
            let since = *self.resting.get_or_insert(tick);
            return if tick.wrapping_sub(since) < VERDICT_TICKS {
                Visit::Go(Intent::IDLE)
            } else {
                Visit::Fail(Why::Refused)
            };
        }
        self.resting = None;
        if self.turns >= MAX_TURNS {
            return Visit::Fail(Why::Stuck);
        }
        let n = usize::from(mech.n_dials).min(MAX_DIALS);
        let Some(d) = (0..n).find(|&d| mech.dials[d] != want[d]) else {
            // Showing it, and no word yet of who solved it.
            return Visit::Go(Intent::IDLE);
        };
        let Some(at) = dial_at(core, usize::from(m), d) else {
            return Visit::Fail(Why::NotFound);
        };
        match approach(core, body, route, at, DIAL_STAND_M, tick) {
            Some(Ok(intent)) => Visit::Go(intent),
            Some(Err(why)) => Visit::Fail(why),
            None if e_lore(core, x, y, z) == Aimed::Turn(m, d as u8) => Visit::Arc {
                op: OP_TURN,
                target: m,
                arg: d as u8,
            },
            None => Visit::Go(look(at)),
        }
    }
}

/// The walk up to each card door's reader that the route's metre grid does
/// not plan (it knows the ground, not the terraces): from the foot of the
/// front stairs, straight up the flights, and round the ledge to the
/// reader. The green door's is on the floor, through the rear portal, and
/// the route finds it. In the ziggurat's own metres: (x, feet y, z).
const PATHS: [&[(f32, f32, f32)]; CARD_DOORS] = [
    &[],
    // Up the first flight to its landing, off it onto the first ledge,
    // east along it, and north along its east side.
    &[
        (0.0, 0.0, -39.0),
        (0.0, 5.0, -31.5),
        (5.0, 5.0, -29.0),
        (29.0, 5.0, -29.0),
        (29.0, 5.0, 2.2),
    ],
    // Up three flights to the third ledge, off the landing onto it, west
    // along it, north along its west side, and east along the rear.
    &[
        (0.0, 0.0, -39.0),
        (0.0, 15.0, -19.5),
        (-5.0, 15.0, -17.0),
        (-17.0, 15.0, -17.0),
        (-17.0, 15.0, 17.0),
        (-5.8, 15.0, 17.0),
    ],
];

/// From the reader to square in front of the doorway, then through it to
/// the lever; out, the other way. The green door's is the route's.
const INS: [&[(f32, f32, f32)]; CARD_DOORS] = [&[], &[(25.0, 5.0, 0.0)], &[(-8.0, 15.0, 14.5)]];

/// The mouth of each card room's passage, inside: the way between the
/// hall and the lever goes by it. The green room's is the route's.
const MOUTHS: [Option<(f32, f32, f32)>; CARD_DOORS] =
    [None, Some((7.0, 5.0, 0.0)), Some((-8.0, 15.0, 6.0))];

/// In card room `d`'s hall, not its passage.
fn in_hall(zig: &Ziggurat, d: usize, x: f32, z: f32) -> bool {
    let (lx, lz) = sim_core::kit::to_local(&zig.placed(), x, z);
    let inside = |(x0, z0, x1, z1): (f32, f32, f32, f32)| lx > x0 && lx < x1 && lz > z0 && lz < z1;
    let [hall, passage] = ROOMS[d].0;
    inside(hall) && !inside(passage)
}

/// A point of the ziggurat's own, in the world.
fn at_zig(zig: &Ziggurat, (x, y, z): (f32, f32, f32)) -> [f32; 3] {
    let (wx, wz) = sim_core::kit::to_world(&zig.placed(), x, z);
    [wx, zig.floor_y + y, wz]
}

/// Up on the ziggurat's terraces, above its floor, where the route's grid
/// does not reach: walked straight, as its flights and ledges are.
pub fn aloft(zig: &Ziggurat, x: f32, feet: f32, z: f32) -> bool {
    if !zig.live {
        return false;
    }
    let (lx, lz) = sim_core::kit::to_local(&zig.placed(), x, z);
    lx.abs() <= 33.0 && lz.abs() <= 33.0 && feet - zig.floor_y > 2.5
}

/// Straight to `to`, within `stop`; `None` once there.
fn straight_to(x: f32, z: f32, to: [f32; 3], stop: f32) -> Option<Intent> {
    ((to[0] - x).hypot(to[2] - z) > stop).then(|| Intent::walk(yaw_toward(to[0] - x, to[2] - z)))
}

/// A straight walk that has come no closer by [`STALL_M`] in
/// [`STALL_TICKS`] is up against something: given up.
const STALL_M: f32 = 0.3;
const STALL_TICKS: u32 = 5 * TICK_HZ;
/// Falls off a flight or a ledge before a climb is given up.
const MAX_FALLS: u8 = 2;

/// The closest a straight walk has come to its point, and when.
#[derive(Clone, Copy, Debug, Default)]
struct Progress(Option<([f32; 3], f32, u32)>);

impl Progress {
    /// Walking at `to` from `left` metres off: false once it has stalled.
    fn going(&mut self, to: [f32; 3], left: f32, tick: u32) -> bool {
        match self.0 {
            Some((p, best, at)) if p == to => {
                if left < best - STALL_M {
                    self.0 = Some((to, left, tick));
                    true
                } else {
                    tick.wrapping_sub(at) < STALL_TICKS
                }
            }
            _ => {
                self.0 = Some((to, left, tick));
                true
            }
        }
    }
}

/// [`straight_to`], given up once it stalls.
fn straight_on(
    progress: &mut Progress,
    x: f32,
    z: f32,
    to: [f32; 3],
    stop: f32,
    tick: u32,
) -> Option<Result<Intent, Why>> {
    let intent = straight_to(x, z, to, stop)?;
    let left = (to[0] - x).hypot(to[2] - z);
    Some(if progress.going(to, left, tick) {
        Ok(intent)
    } else {
        Err(Why::Stuck)
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Leg {
    /// Up the flights to waypoint `.0` of the door's path.
    #[default]
    Path,
    Reader,
    /// The swipe went out at this tick.
    Swiped(u32),
    In,
}

/// One card door opened: to its reader (up the flights and round the
/// ledge for the blue and the red), the swipe, and in to the lever while
/// the door stands open.
#[derive(Clone, Copy, Debug, Default)]
pub struct SwipeJob {
    door: Option<u8>,
    leg: Leg,
    /// The waypoint walked to next, on the path up or the way in; how a
    /// straight walk is going; falls off the flights so far.
    next: u8,
    progress: Progress,
    falls: u8,
    refused: Option<u8>,
    /// Doors opened with a card.
    pub swipes: u64,
}

impl SwipeJob {
    pub fn reset(&mut self) {
        *self = Self {
            swipes: self.swipes,
            ..Self::default()
        };
    }

    pub fn on_refused(&mut self, code: u8, door: u8) {
        if self.door == Some(door) && matches!(self.leg, Leg::Swiped(_)) {
            self.refused = Some(code);
        }
    }

    pub fn sent(&mut self, tick: u32) {
        self.leg = Leg::Swiped(tick);
    }

    /// Under way: climbing, at the reader, or going in. Its own walk is
    /// the way, wherever on the ziggurat that has it.
    pub fn under_way(&self) -> bool {
        self.door.is_some() && (self.leg != Leg::Path || self.next > 0)
    }

    pub fn step(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        route: &mut Route,
        tick: u32,
    ) -> Visit {
        if let Some(code) = self.refused {
            return Visit::Fail(if u32::from(code) == sim_core::monument::REFUSE_S_CARD {
                Why::MissingInputs
            } else {
                Why::Refused
            });
        }
        let (x, y, z) = feet(body);
        let zig = core.haven().ziggurat;
        let d = match self.door {
            Some(d) => d,
            None => match swipeable(core) {
                Some((d, _)) => {
                    self.door = Some(d);
                    // Already up there: from the nearest waypoint on.
                    if aloft(&zig, x, y, z) {
                        self.next = nearest_waypoint(&zig, PATHS[usize::from(d)], x, y, z);
                    }
                    d
                }
                None => return Visit::Fail(Why::NotFound),
            },
        };
        let i = usize::from(d);
        let (Some(reader), Some(lever)) =
            (reader_world(&zig, i, false), reader_world(&zig, i, true))
        else {
            return Visit::Fail(Why::NotFound);
        };
        let (reader, lever) = ([reader.0, reader.1, reader.2], [lever.0, lever.1, lever.2]);
        let up = aloft(&zig, x, y, z);
        match self.leg {
            Leg::Path => {
                let path = PATHS[i];
                // Off the side of a flight or a ledge: from the foot again.
                if self.next >= 2 && !up {
                    self.falls += 1;
                    if self.falls > MAX_FALLS {
                        return Visit::Fail(Why::Stuck);
                    }
                    self.next = 0;
                }
                let Some(&w) = path.get(usize::from(self.next)) else {
                    self.leg = Leg::Reader;
                    return Visit::Go(Intent::IDLE);
                };
                let w = at_zig(&zig, w);
                // The foot of the stairs by the route; the flights and the
                // ledges straight, to the half metre.
                let step = if self.next == 0 && !up {
                    approach(core, body, route, w, SWIPE_STAND_M, tick)
                } else {
                    straight_on(&mut self.progress, x, z, w, HOP_M, tick)
                };
                match step {
                    Some(Ok(intent)) => Visit::Go(intent),
                    Some(Err(why)) => Visit::Fail(why),
                    None => {
                        self.next += 1;
                        Visit::Go(Intent::IDLE)
                    }
                }
            }
            Leg::Reader => {
                if door_open(core, i) {
                    self.leg = Leg::In;
                    self.next = 0;
                    return Visit::Go(Intent::IDLE);
                }
                let step = if up {
                    straight_on(&mut self.progress, x, z, reader, SWIPE_STAND_M, tick)
                } else {
                    approach(core, body, route, reader, SWIPE_STAND_M, tick)
                };
                match step {
                    Some(Ok(intent)) => Visit::Go(intent),
                    Some(Err(why)) => Visit::Fail(why),
                    None if e_swipe(&zig, x, y, z) == Some((d, false)) => Visit::Swipe(d),
                    None => Visit::Go(look(reader)),
                }
            }
            Leg::Swiped(at) => {
                if door_open(core, i) {
                    self.swipes += 1;
                    self.leg = Leg::In;
                    self.next = 0;
                    Visit::Go(Intent::IDLE)
                } else if tick.wrapping_sub(at) >= VERDICT_TICKS {
                    Visit::Fail(Why::NoAnswer)
                } else {
                    Visit::Go(look(reader))
                }
            }
            Leg::In => {
                if room_of(&zig, x, y, z) == Some(d)
                    && (lever[0] - x).hypot(lever[2] - z) <= ROOM_STAND_M
                {
                    return Visit::Done;
                }
                if !door_open(core, i) && room_of(&zig, x, y, z) != Some(d) {
                    // Shut again before it was through.
                    return Visit::Fail(Why::Stuck);
                }
                let step = match INS[i].get(usize::from(self.next)) {
                    Some(&w) => {
                        match straight_on(&mut self.progress, x, z, at_zig(&zig, w), HOP_M, tick) {
                            None => {
                                self.next += 1;
                                return Visit::Go(Intent::IDLE);
                            }
                            some => some,
                        }
                    }
                    None if up => straight_on(&mut self.progress, x, z, lever, ROOM_STAND_M, tick),
                    None => approach(core, body, route, lever, ROOM_STAND_M, tick),
                };
                match step {
                    Some(Ok(intent)) => Visit::Go(intent),
                    Some(Err(why)) => Visit::Fail(why),
                    None => Visit::Done,
                }
            }
        }
    }
}

/// Where a waypoint counts as reached on the way in and out of a doorway.
const HOP_M: f32 = 0.5;

/// The waypoint of `path` nearest (`x`, `feet`, `z`) on its own level.
fn nearest_waypoint(zig: &Ziggurat, path: &[(f32, f32, f32)], x: f32, feet: f32, z: f32) -> u8 {
    let mut best: Option<(f32, u8)> = None;
    for (k, &w) in path.iter().enumerate() {
        let p = at_zig(zig, w);
        if (p[1] - feet).abs() > 2.5 {
            continue;
        }
        let d = (p[0] - x).hypot(p[2] - z);
        if best.is_none_or(|b| d < b.0) {
            best = Some((d, k as u8));
        }
    }
    best.map_or(0, |b| b.1)
}

/// Out of a card room, its lever pulled if the door has shut, and down off
/// the ziggurat's terraces the way up came.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExitJob {
    /// The lever went out at this tick.
    pulled: Option<u32>,
    /// Going down door `.0`'s path, to waypoint `.1` next; going out of
    /// door `.0`'s room, to its doorway waypoint `.1` next (`out`).
    down: Option<(u8, u8)>,
    /// Going out of door `.0`'s room, to point `.1` of the way out next,
    /// and whether the way started in its hall.
    out: Option<(u8, u8, bool)>,
    progress: Progress,
    /// A lever or a walk out that came to nothing, at this tick.
    pub failed: Option<u32>,
    /// Levers pulled.
    pub pulls: u64,
}

impl ExitJob {
    pub fn sent(&mut self, tick: u32) {
        self.pulled = Some(tick);
        self.pulls += 1;
    }

    /// The way out of the room it stands in and, with `descend`, down to
    /// the ground; `None` once there (or in no room and on the ground).
    pub fn step(
        &mut self,
        core: &mut ClientCore,
        haven: &Haven,
        body: &EntityState,
        route: &mut Route,
        descend: bool,
        tick: u32,
    ) -> Option<Visit> {
        let (x, y, z) = feet(body);
        let zig = haven.ziggurat;
        let up = aloft(&zig, x, y, z);
        let Some(d) = room_of(&zig, x, y, z) else {
            self.out = None;
            if !up || !descend {
                self.down = None;
                return None;
            }
            // Down the flights the way up came: the nearest waypoint of
            // the door's path on this level, then each before it.
            let (door, k) = *self.down.get_or_insert_with(|| {
                let door = (1..CARD_DOORS)
                    .min_by(|&a, &b| {
                        let dist = |d: usize| {
                            let k = nearest_waypoint(&zig, PATHS[d], x, y, z);
                            let p = at_zig(&zig, PATHS[d][usize::from(k)]);
                            (p[0] - x).hypot(p[2] - z) + (p[1] - y).abs() * 4.0
                        };
                        dist(a).total_cmp(&dist(b))
                    })
                    .unwrap_or(1);
                (door as u8, nearest_waypoint(&zig, PATHS[door], x, y, z))
            });
            let w = at_zig(&zig, PATHS[usize::from(door)][usize::from(k)]);
            return Some(
                match straight_on(&mut self.progress, x, z, w, HOP_M, tick) {
                    Some(Ok(intent)) => Visit::Go(intent),
                    Some(Err(why)) => {
                        self.down = None;
                        Visit::Fail(why)
                    }
                    None if k == 0 => {
                        self.down = None;
                        Visit::Done
                    }
                    None => {
                        self.down = Some((door, k - 1));
                        Visit::Go(Intent::IDLE)
                    }
                },
            );
        };
        let i = usize::from(d);
        let (Some(reader), Some(lever)) =
            (reader_world(&zig, i, false), reader_world(&zig, i, true))
        else {
            return None;
        };
        let (reader, lever) = ([reader.0, reader.1, reader.2], [lever.0, lever.1, lever.2]);
        let hall = in_hall(&zig, i, x, z);
        if door_open(core, i) {
            self.pulled = None;
            // Out: by the passage's mouth from the hall, through the
            // doorway's points backwards, then to the reader, outside.
            let (_, k, from_hall) = *self.out.get_or_insert((d, 0, hall));
            let mut way = [reader; 4];
            let mut n = 0;
            if let Some(m) = MOUTHS[i].filter(|_| from_hall) {
                way[n] = at_zig(&zig, m);
                n += 1;
            }
            for &w in INS[i].iter().rev() {
                way[n] = at_zig(&zig, w);
                n += 1;
            }
            way[n] = reader;
            let to = way[usize::from(k).min(n)];
            let step = if n == 0 && !up {
                approach(core, body, route, reader, SWIPE_STAND_M, tick)
            } else {
                straight_on(&mut self.progress, x, z, to, HOP_M, tick)
            };
            return Some(match step {
                Some(Ok(intent)) => Visit::Go(intent),
                Some(Err(why)) => Visit::Fail(why),
                None => {
                    self.out = Some((d, k.saturating_add(1), from_hall));
                    Visit::Go(Intent::IDLE)
                }
            });
        }
        if let Some(at) = self.pulled {
            if tick.wrapping_sub(at) < VERDICT_TICKS {
                return Some(Visit::Go(look(lever)));
            }
            self.pulled = None;
            return Some(Visit::Fail(Why::NoAnswer));
        }
        // To the lever, by the passage's mouth from the hall.
        if let Some(m) = MOUTHS[i].filter(|_| hall) {
            return Some(
                match straight_on(&mut self.progress, x, z, at_zig(&zig, m), HOP_M, tick) {
                    Some(Ok(intent)) => Visit::Go(intent),
                    Some(Err(why)) => Visit::Fail(why),
                    // Into the passage: the lever is straight ahead.
                    None => Visit::Go(Intent::walk(yaw_toward(lever[0] - x, lever[2] - z))),
                },
            );
        }
        let step = if up {
            straight_on(&mut self.progress, x, z, lever, SWIPE_STAND_M, tick)
        } else {
            approach(core, body, route, lever, SWIPE_STAND_M, tick)
        };
        Some(match step {
            Some(Ok(intent)) => Visit::Go(intent),
            Some(Err(why)) => Visit::Fail(why),
            None if e_swipe(&zig, x, y, z) == Some((d, true)) => Visit::Swipe(d),
            None => Visit::Go(look(lever)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lore_with(text: &str, glyphs: &str, dials: u8) -> LoreView {
        let mut lore = LoreView::default();
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        lore.alphabet[..alphabet.len()].copy_from_slice(alphabet);
        lore.alphabet_len = alphabet.len() as u8;
        for ch in glyphs.bytes() {
            if let Some(g) = lore.glyph_of(ch) {
                lore.glyphs |= 1 << g;
            }
        }
        lore.n_inscriptions = 1;
        lore.inscriptions[0].known = true;
        lore.inscriptions[0].text[..text.len()].copy_from_slice(text.as_bytes());
        lore.inscriptions[0].len = text.len() as u16;
        lore.n_mechs = 1;
        let m = &mut lore.mechs[0];
        m.known = true;
        let name = b"THE RING LOCK";
        m.name[..name.len()].copy_from_slice(name);
        m.name_len = name.len() as u8;
        m.n_dials = dials;
        m.values = 8;
        lore
    }

    #[test]
    fn a_lock_s_answer_is_the_digits_it_can_read() {
        let text = "AT THE RING OF STONES TURN THE THREE TO 3 5 1";
        // The digits are shapes to a player who never read the count stone.
        assert_eq!(answer(&lore_with(text, "AEIORSTNHL", 3), 0), None);
        let lore = lore_with(text, "0123456789", 3);
        assert_eq!(answer(&lore, 0), Some([3, 5, 1, 0]));
        let mut out = [0u8; ARC_TEXT_BYTES];
        let n = seen(&lore, 0, &mut out);
        assert_eq!(&out[..n], b"?? ??? ???? ?? ?????? ???? ??? ????? ?? 3 5 1");
        // A count of ten digits is no three-dial answer; nor is a notch the
        // dials do not have.
        assert_eq!(
            answer(&lore_with("COUNT 0 1 2 3 4 5 6 7 8 9", "0123456789", 3), 0),
            None
        );
        assert_eq!(answer(&lore_with("TO 3 9 1", "0123456789", 3), 0), None);
    }
}
