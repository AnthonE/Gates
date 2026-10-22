//! Mechanisms (`ARC.md` F7): the ancients' puzzles, re-seeded every wipe.
//!
//! A mechanism is a row of **dials** at a spot. Each dial turns one notch per
//! press, wrapping at `values`, and anyone may turn any dial, so a rival can
//! scramble a lock you were halfway through. When every dial shows this
//! wipe's solution, the player who made the last turn takes the reward. The
//! mechanism then rests for its cooldown before it pays again.
//!
//! **The solution is this wipe's, and only the server can work it out.** It
//! is derived from the world's [`ArcState::salt`], which the server draws
//! when a world is made, saves with it, and never sends. A client holding
//! the seed can rebuild the island but not the answer. The answer is written
//! on the island instead, in glyphs, on a stone somewhere else
//! (`lore.rs`, an inscription whose text names the mechanism). Rust's
//! puzzles are on YouTube; these change every wipe.
//!
//! **A dial is also a target.** An arrow that strikes a dial's drum turns
//! it one notch, as a press does, from wherever it was loosed ([`shot`]),
//! and the archer whose arrow shows the answer takes the reward. The lock
//! can be worked from a sniper's perch, and scrambled from one.

use crate::limits::{MAX_DIALS, MAX_MECHS};
use crate::spot::Spot;
use crate::terrain::Haven;
use crate::works::{REFUSE_A_KIND, REFUSE_A_REACH, REFUSE_A_RESTING};
use crate::world::{EventQueue, Player, EV_ARC_DID, EV_ARC_REFUSED, EV_MECH_SOLVED};

/// `Command::Arc` op: turn dial `arg` of mechanism `target` one notch.
pub const OP_TURN: u8 = 4;

/// How close to a dial a turn must be, metres.
pub const DIAL_REACH_M: f32 = 2.0;
/// Metres between neighbouring dials, along the mechanism's x.
pub const DIAL_PITCH_M: f32 = 1.6;
/// The most notches a dial has.
pub const MAX_VALUES: u8 = 10;
/// A dial's drum as drawn (`lore_dial.glb`): 1.1 m across, its top 1.0 m
/// off the ground under it. What an arrow strikes.
pub const DRUM_R_M: f32 = 0.55;
pub const DRUM_H_M: f32 = 1.0;

/// One mechanism, baked from `content/arc.toml`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MechDef {
    /// The middle of the row of dials.
    pub spot: Spot,
    pub dials: u8,
    /// Notches per dial, 2..=`MAX_VALUES`.
    pub values: u8,
    /// What a solve pays the solver.
    pub reward: u16,
    pub reward_n: u16,
    /// How long it rests after a solve, ticks.
    pub rest_ticks: u32,
}

/// Every mechanism on the shard. `EMPTY` holds none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MechContent {
    pub defs: [MechDef; MAX_MECHS],
    pub count: u8,
}

impl MechContent {
    pub const EMPTY: MechContent = MechContent {
        defs: [MechDef {
            spot: Spot {
                site: 0,
                nth: 0,
                x_cm: 0,
                y_cm: 0,
                z_cm: 0,
            },
            dials: 0,
            values: 0,
            reward: 0,
            reward_n: 0,
            rest_ticks: 0,
        }; MAX_MECHS],
        count: 0,
    };

    /// One three-dial lock of four notches at the town's middle, paying two
    /// of item 1, resting a minute, for the gates.
    pub fn get(&self, k: usize) -> Option<&MechDef> {
        (k < self.count as usize).then(|| &self.defs[k])
    }

    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.defs[0] = MechDef {
            spot: Spot {
                site: crate::spot::SITE_TOWN,
                ..Spot::default()
            },
            dials: 3,
            values: 4,
            reward: 1,
            reward_n: 2,
            rest_ticks: 60 * crate::limits::TICK_HZ,
        };
        c.count = 1;
        c
    }
}

/// One mechanism's state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mech {
    pub dials: [u8; MAX_DIALS],
    /// The tick it pays again; 0 is armed.
    pub rest_until: u64,
    pub solves: u32,
}

/// Bytes a mechanism saves and hashes as.
pub const MECH_BYTES: usize = MAX_DIALS + 8 + 4;

impl Mech {
    pub fn to_bytes(&self) -> [u8; MECH_BYTES] {
        let mut b = [0u8; MECH_BYTES];
        b[..MAX_DIALS].copy_from_slice(&self.dials);
        b[MAX_DIALS..MAX_DIALS + 8].copy_from_slice(&self.rest_until.to_le_bytes());
        b[MAX_DIALS + 8..].copy_from_slice(&self.solves.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; MECH_BYTES]) -> Mech {
        let mut dials = [0u8; MAX_DIALS];
        dials.copy_from_slice(&b[..MAX_DIALS]);
        Mech {
            dials,
            rest_until: u64::from_le_bytes(b[MAX_DIALS..MAX_DIALS + 8].try_into().expect("8")),
            solves: u32::from_le_bytes(b[MAX_DIALS + 8..].try_into().expect("4")),
        }
    }
}

/// The world's secret and the mechanisms' state. Sim state: hashed once it
/// is not fresh, and saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArcState {
    /// This world's secret, drawn by the server when the world is made
    /// (`server::boot`) and never sent to a client. 0 on a world nobody
    /// salted, which still plays: its answers are just the seed's.
    pub salt: u64,
    pub mechs: [Mech; MAX_MECHS],
}

impl ArcState {
    pub fn is_fresh(&self) -> bool {
        *self == ArcState::default()
    }
}

/// This wipe's answer for dial `d` of mechanism `k`.
pub fn answer(salt: u64, seed: u64, k: usize, d: usize, values: u8) -> u8 {
    let h = crate::rng::cell_hash(seed ^ salt.rotate_left(17), k as i32, d as i32, 0x4D45_4348);
    (h % values.max(1) as u64) as u8
}

/// Every dial's answer for mechanism `k`, for the server's hint text.
pub fn answers(salt: u64, seed: u64, k: usize, def: &MechDef) -> [u8; MAX_DIALS] {
    let mut out = [0u8; MAX_DIALS];
    for (d, v) in out.iter_mut().enumerate().take(def.dials as usize) {
        *v = answer(salt, seed, k, d, def.values);
    }
    out
}

/// Where dial `d` of `def` stands: the row runs along the mechanism's local x,
/// centred on its spot.
pub fn dial_spot(def: &MechDef, d: usize) -> Spot {
    let off = (d as f32 - (def.dials as f32 - 1.0) * 0.5) * DIAL_PITCH_M;
    Spot {
        x_cm: def.spot.x_cm.saturating_add((off * 100.0) as i16),
        ..def.spot
    }
}

/// One dial's drum, where it stands: its foot on the ground, metres.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Drum {
    pub mech: u8,
    pub dial: u8,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Every drum on the island, `n` of them, stood up once a tick for the
/// arrows to meet (`ranged::Quarry`).
#[derive(Clone, Copy, Debug)]
pub struct Drums {
    pub d: [Drum; MAX_MECHS * MAX_DIALS],
    pub n: usize,
}

impl Drums {
    pub const NONE: Drums = Drums {
        d: [Drum {
            mech: 0,
            dial: 0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }; MAX_MECHS * MAX_DIALS],
        n: 0,
    };

    /// Each dial of `mc` on the ground at its spot, where the client draws
    /// its drum. A mechanism whose site is not on this island has none.
    pub fn stand(mc: &MechContent, seed: u64, haven: &Haven) -> Drums {
        let mut out = Drums::NONE;
        for (k, def) in mc.defs.iter().enumerate().take(mc.count as usize) {
            for d in 0..def.dials as usize {
                let Some((x, _, z)) = crate::spot::world(haven, &dial_spot(def, d)) else {
                    continue;
                };
                if out.n < out.d.len() {
                    out.d[out.n] = Drum {
                        mech: k as u8,
                        dial: d as u8,
                        x,
                        y: crate::terrain::ground(seed, haven, x, z),
                        z,
                    };
                    out.n += 1;
                }
            }
        }
        out
    }

    /// The first drum the segment `o + s·t` (millimetres) enters, as its
    /// index and `t`. An arrow's own radius widens each drum.
    pub fn first(&self, o: (f32, f32, f32), s: (f32, f32, f32)) -> Option<(usize, f32)> {
        const MM: f32 = crate::ranged::MM_PER_M;
        let o = (o.0 / MM, o.1 / MM, o.2 / MM);
        let u = (s.0 / MM, s.1 / MM, s.2 / MM);
        let r = DRUM_R_M + crate::ranged::ARROW_R_M;
        let mut best: Option<(usize, f32)> = None;
        for (i, dr) in self.d.iter().enumerate().take(self.n) {
            let span = crate::melee::cylinder_span(o, u, (dr.x, dr.z), r, dr.y, dr.y + DRUM_H_M);
            if let Some((t, _)) = span {
                if best.is_none_or(|b| t < b.1) {
                    best = Some((i, t));
                }
            }
        }
        best
    }
}

/// An arrow that struck a drum, handed to `World` to turn (`ranged::Quarry`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrumShot {
    pub mech: u8,
    pub dial: u8,
    /// The archer's player id.
    pub by: u32,
}

/// An arrow struck a dial: it turns one notch, from any range. `p` is the
/// archer when they are still standing on the island, and takes the reward
/// if the turn solves it; a solve with nobody to pay still rests the lock.
#[allow(clippy::too_many_arguments)]
pub fn shot(
    mc: &MechContent,
    state: &mut ArcState,
    seed: u64,
    tick: u64,
    gc: &crate::gather::GatherContent,
    s: DrumShot,
    p: Option<&mut Player>,
    events: &mut EventQueue,
) {
    let (k, d) = (s.mech as usize, s.dial as usize);
    if mc.get(k).is_some_and(|def| d < def.dials as usize) {
        turn(mc, state, seed, tick, gc, k, d, s.by, p, events);
    }
}

/// One turn from `p`. Refusals are events.
#[allow(clippy::too_many_arguments)]
pub fn act(
    mc: &MechContent,
    state: &mut ArcState,
    haven: &Haven,
    seed: u64,
    tick: u64,
    gc: &crate::gather::GatherContent,
    p: &mut Player,
    target: u8,
    arg: u8,
    events: &mut EventQueue,
) {
    let pid = p.id;
    let refuse = |events: &mut EventQueue, code: u32| {
        events.push(
            EV_ARC_REFUSED,
            pid,
            code,
            (OP_TURN as u32) << 8 | target as u32,
        );
    };
    let k = target as usize;
    let d = arg as usize;
    if k >= mc.count as usize || p.dead || p.sleeping || p.wounded {
        return refuse(events, REFUSE_A_KIND);
    }
    let def = mc.defs[k];
    if d >= def.dials as usize {
        return refuse(events, REFUSE_A_KIND);
    }
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    let feet = p.body.qy as f32 * crate::movement::POS_Y_Q;
    if !crate::spot::within(haven, &dial_spot(&def, d), px, feet, pz, DIAL_REACH_M) {
        return refuse(events, REFUSE_A_REACH);
    }
    turn(mc, state, seed, tick, gc, k, d, pid, Some(p), events);
}

/// Dial `d` of mechanism `k` turns one notch for player `who`, by hand or
/// by arrow. The turn that shows the answer pays `p`, if there is one.
#[allow(clippy::too_many_arguments)]
fn turn(
    mc: &MechContent,
    state: &mut ArcState,
    seed: u64,
    tick: u64,
    gc: &crate::gather::GatherContent,
    k: usize,
    d: usize,
    who: u32,
    p: Option<&mut Player>,
    events: &mut EventQueue,
) {
    let def = mc.defs[k];
    let m = &mut state.mechs[k];
    if m.rest_until > tick {
        events.push(
            EV_ARC_REFUSED,
            who,
            REFUSE_A_RESTING,
            (OP_TURN as u32) << 8 | k as u32,
        );
        return;
    }
    m.dials[d] = (m.dials[d] + 1) % def.values.max(1);
    events.push(EV_ARC_DID, who, OP_TURN as u32, (k as u32) << 8 | d as u32);
    let want = answers(state.salt, seed, k, &def);
    if m.dials[..def.dials as usize] != want[..def.dials as usize] {
        return;
    }
    // Solved. The reward goes into the pack (a full pack keeps what fits),
    // and the lock rests and scrambles.
    if let Some(p) = p {
        let cap = gc.stack_max_of(def.reward);
        let cond = gc.cond_max_of(def.reward);
        crate::gather::inv_add(&mut p.inv, def.reward, def.reward_n, cap, cond);
    }
    m.solves = m.solves.wrapping_add(1);
    m.rest_until = tick + def.rest_ticks as u64;
    for (j, v) in m.dials.iter_mut().enumerate().take(def.dials as usize) {
        *v = (want[j] + 1 + j as u8) % def.values.max(1);
    }
    events.push(EV_MECH_SOLVED, k as u32, who, 0);
}
