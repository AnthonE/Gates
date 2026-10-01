//! Other people's bases as a raider knows them, and the raid.
//!
//! What it knows is what its eyes found ([`Bases::look`]): a wall, a
//! doorway or a door at ground level that stood in its view cone, in
//! range, with a clear line to its near face; a box seen the same way; and
//! who was about (players seen near the place, awake or asleep). Never the
//! island's list of bases. Each face is kept with what a player reads off
//! it: its material (the row it looks like), how damaged it looks (the
//! band the client draws) and which side is the soft one. The game's
//! numbers (a satchel's structure damage, a weapon's per-blow damage on a
//! structure, a piece's full hp) are the wiki's and the tables'.
//!
//! The cost of a way in is the weakest face's ([`breach`]): the satchels
//! its hp takes, or the blows of the belt's weapon when that is few enough
//! (a wooden door, a twig wall). Which bases are worth it is the
//! temperament's ([`Bases::best`]): an opportunist goes only for a weak or
//! an offline-looking one, a killer-on-sight for any it can pay for, the
//! others never.
//!
//! The raid ([`RaidJob`]) walks to the face from outside, plants each
//! satchel with it in hand (`encode_action_throw`, aimed the way the human
//! client's `X` aims: the structure nearest the feet), stands clear of the
//! blast, and once the face is down walks in, opens each box it sees with
//! the panel (`encode_action_container`) and takes what fits
//! (`encode_action_move`). Then the goal ends, and home is the mind's.

use crate::agent::build::{aim_point, e_picks_by, look_point, nearest_structure};
use crate::agent::combat::Temperament;
use crate::agent::hands::Hands;
use crate::agent::home::{belt_move, HOLD_TICKS};
use crate::agent::intent::{yaw_toward, Intent, Look};
use crate::agent::route::{Route, Step};
use crate::agent::tracks::{clear_line, Sight, Species, Tracks};
use crate::agent::wiki::Throw;
use crate::mind::Why;
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::bots::OpAddr;
use sim_core::build::{anchor, facing_of, shape_has_facing, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE};
use sim_core::deploy::{arch_is_door, box_key, cell_center, ARCH_BOX, ARCH_HEARTH};
use sim_core::input::BTN_PRIMARY;
use sim_core::inventory::CONT_BOX;
use sim_core::limits::{HOTBAR_SLOTS, INV_SLOTS, TICK_HZ};
use sim_core::movement::POS_XZ_Q;
use sim_core::terrain::Haven;
use sim_core::yaw_dir;

/// The item a raid plants, by its catalog name.
pub const SATCHEL_ITEM: &str = "Satchel Charge";
/// Bases remembered, and the faces and boxes kept of each.
pub const BASE_ROWS: usize = 8;
pub const FACE_ROWS: usize = 6;
pub const BOX_ROWS: usize = 3;
/// A face is made out this far off; a box, only this near.
pub const RAID_SIGHT_M: f32 = 60.0;
pub const BOX_SIGHT_M: f32 = 24.0;
/// Mirror records examined per frame, and the sight lines they may cost.
pub const RAID_SCAN_PER_FRAME: usize = 8;
pub const RAID_RAYS_PER_FRAME: usize = 1;
/// Records this close to a remembered base are that base.
pub const BASE_MERGE_M: f32 = 15.0;
/// A player seen this near a base is about it.
pub const NEAR_BASE_M: f32 = 25.0;
/// Watched this long with nobody awake seen about it: offline-looking.
pub const OFFLINE_TICKS: u32 = 300 * TICK_HZ;
/// At most this many satchels is a weak face; at most this many blows of
/// the belt's weapon is a way in without any.
pub const WEAK_SATCHELS: u32 = 2;
pub const MELEE_SWINGS_MAX: u32 = 60;
/// Satchels planted on one face before the raid gives up on it.
pub const MAX_CHARGES: u8 = 6;
/// A base further than this is not worth the trip.
pub const RAID_TRIP_M: f32 = 400.0;
/// A base raided, or a raid on it that came to nothing, is let be this long.
pub const RAID_RETRY_TICKS: u32 = 600 * TICK_HZ;
/// Where it stands to plant a charge or swing: this far out from the
/// face's middle, and on the last steps within this of the spot.
pub const STAND_OUT_M: f32 = 1.0;
pub const STAND_NEAR_M: f32 = 0.15;
pub const STALL_TICKS: u32 = 2 * TICK_HZ;
/// Clear of a charge's blast by this much more than its radius.
pub const CLEAR_SLACK_M: f32 = 4.0;
/// A charge goes off within this long past its fuse.
pub const FUSE_SLACK_TICKS: u32 = 2 * TICK_HZ;
/// An action's answer.
pub const VERDICT_TICKS: u32 = 3 * TICK_HZ;
/// Through the hole: this far past where the face stood, within this long.
pub const ENTER_M: f32 = 1.0;
pub const ENTER_TICKS: u32 = 20 * TICK_HZ;
/// Inside, it looks round this long for boxes before giving the base up.
pub const LOOK_TICKS: u32 = 6 * TICK_HZ;
/// Presses answered by nothing before a box is let be.
pub const BOX_TRIES: u8 = 3;
/// It stands this near a box to open it.
pub const BOX_STAND_M: f32 = 1.4;

/// One face of a base, as a player reads it off the wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    pub deploy: bool,
    pub at: OpAddr,
    pub hp_max: u16,
    /// The damage band drawn on it when last seen.
    pub dmg: u8,
    /// It has a hard face and a soft one, and which way the soft one faces
    /// (`PieceRec::facing`).
    pub sided: bool,
    pub facing: u8,
    /// Out of the base: the side the eyes saw it from, a unit vector.
    pub out: [f32; 2],
    pub seen: u32,
}

impl Face {
    /// Its hp, at most: the band says how much is gone, in sevenths.
    pub fn hp(&self) -> u32 {
        let max = u32::from(self.hp_max);
        let gone = u32::from(self.dmg.saturating_sub(1));
        max.saturating_sub(gone * max / 7).max(1)
    }

    /// Where a body stands outside it, this far out.
    pub fn outside(&self, m: f32) -> [f32; 2] {
        let (ax, az) = anchor(self.at.cx, self.at.cz, self.at.loc);
        [ax + self.out[0] * m, az + self.out[1] * m]
    }
}

/// One base this body has seen.
#[derive(Clone, Copy, Debug, Default)]
pub struct Base {
    pub used: bool,
    pub at: [f32; 2],
    pub first: u32,
    pub last: u32,
    /// A player seen about it awake, or asleep in it, last.
    pub awake: Option<u32>,
    pub sleeper: Option<u32>,
    pub faces: [Option<Face>; FACE_ROWS],
    pub boxes: [Option<OpAddr>; BOX_ROWS],
    /// A raid on it began, or came to nothing.
    pub tried: Option<u32>,
}

impl Base {
    /// Nobody about it awake for a long watch, or somebody asleep in it.
    pub fn offline(&self, tick: u32) -> bool {
        let watched = tick.wrapping_sub(self.first) >= OFFLINE_TICKS;
        let quiet = self
            .awake
            .is_none_or(|at| tick.wrapping_sub(at) >= OFFLINE_TICKS);
        self.sleeper.is_some() || (watched && quiet)
    }
}

/// What it would raid with.
#[derive(Clone, Copy, Debug, Default)]
pub struct Means {
    /// The satchel (wire item) and its page, and how many the pack holds.
    pub satchel: Option<(u16, Throw)>,
    pub satchels: u32,
    /// The belt's swung weapon: wire item, structure damage per blow, and
    /// ticks between blows.
    pub melee: Option<(u16, u16, u16)>,
}

/// A way in, and its price.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Breach {
    Satchels(u8),
    Swings(u16),
}

/// The cheapest way through this face with these means, from outside it.
/// Blows when few enough (a door or a soft face takes the weapon's own
/// number, a hard face the game's floor); else satchels, whose blast takes
/// the same off either face.
pub fn breach(face: &Face, means: &Means) -> Option<Breach> {
    let hp = face.hp();
    if let Some((_, structure, _)) = means.melee.filter(|m| m.1 > 0) {
        let [px, pz] = face.outside(STAND_OUT_M);
        let soft =
            !face.sided || facing_of(face.at.loc, face.at.cx, face.at.cz, px, pz) == face.facing;
        let per = if soft {
            u32::from(structure)
        } else {
            u32::from(sim_core::combat::HARD_SIDE_STRUCTURE)
        };
        let swings = hp.div_ceil(per.max(1));
        if swings <= MELEE_SWINGS_MAX {
            return Some(Breach::Swings(swings as u16));
        }
    }
    let (_, throw) = means.satchel?;
    let n = hp.div_ceil(u32::from(throw.structure).max(1));
    (n <= u32::from(MAX_CHARGES) && n <= means.satchels).then_some(Breach::Satchels(n as u8))
}

/// The cheapest face of a base and its way in, cheapest first: blows
/// before satchels, fewer before more.
fn cheapest(base: &Base, means: &Means) -> Option<(Face, Breach)> {
    let cost = |b: Breach| match b {
        Breach::Swings(n) => u32::from(n),
        Breach::Satchels(n) => 10_000 + u32::from(n),
    };
    base.faces
        .iter()
        .flatten()
        .filter_map(|f| breach(f, means).map(|b| (*f, b)))
        .min_by_key(|&(_, b)| cost(b))
}

/// A way in the temperament takes: weak (blows, or a satchel or two) or
/// the base offline-looking, for an opportunist; any it can pay for, for a
/// killer-on-sight; none otherwise.
pub fn worth(temperament: Temperament, breach: Breach, offline: bool) -> bool {
    let weak = match breach {
        Breach::Swings(_) => true,
        Breach::Satchels(n) => u32::from(n) <= WEAK_SATCHELS,
    };
    match temperament {
        Temperament::Kos => true,
        Temperament::Opportunist => weak || offline,
        Temperament::Passive | Temperament::Defensive => false,
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RaidStats {
    pub examined: u64,
    pub rays: u64,
    pub faces: u64,
    pub boxes_seen: u64,
    pub raids: u64,
    pub charges: u64,
    pub breached: u64,
    pub boxes: u64,
    pub taken: u64,
}

/// The bases this body has seen, by its own eyes.
pub struct Bases {
    bases: [Base; BASE_ROWS],
    piece: usize,
    deploy: usize,
    pub stats: RaidStats,
}

impl Default for Bases {
    fn default() -> Self {
        Self::new()
    }
}

impl Bases {
    pub fn new() -> Self {
        Self {
            bases: [Base::default(); BASE_ROWS],
            piece: 0,
            deploy: 0,
            stats: RaidStats::default(),
        }
    }

    /// Forget them (a new session).
    pub fn clear(&mut self) {
        let stats = self.stats;
        *self = Self::new();
        self.stats = stats;
    }

    pub fn get(&self, i: usize) -> Option<&Base> {
        self.bases.get(i).filter(|b| b.used)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Base> + '_ {
        self.bases.iter().filter(|b| b.used)
    }

    /// The base a point belongs to, if one is remembered there.
    fn base_at(&self, x: f32, z: f32) -> Option<usize> {
        self.bases
            .iter()
            .position(|b| b.used && (b.at[0] - x).hypot(b.at[1] - z) <= BASE_MERGE_M)
    }

    fn base_for(&mut self, x: f32, z: f32, tick: u32) -> usize {
        if let Some(i) = self.base_at(x, z) {
            self.bases[i].last = tick;
            return i;
        }
        let i = self.bases.iter().position(|b| !b.used).unwrap_or_else(|| {
            // Full: the one longest out of sight goes.
            (0..BASE_ROWS)
                .max_by_key(|&i| tick.wrapping_sub(self.bases[i].last))
                .unwrap_or(0)
        });
        self.bases[i] = Base {
            used: true,
            at: [x, z],
            first: tick,
            last: tick,
            ..Base::default()
        };
        i
    }

    /// A raid on base `i` began or came to nothing at `tick`.
    pub fn tried(&mut self, i: usize, tick: u32) {
        if let Some(b) = self.bases.get_mut(i) {
            b.tried = Some(tick);
        }
    }

    /// Who is about each base, as the eyes have them now.
    pub fn bodies(&mut self, tracks: &Tracks, tick: u32) {
        for t in tracks.seen() {
            if !t.visible || t.species != Species::Player || t.dead {
                continue;
            }
            for b in self.bases.iter_mut().filter(|b| b.used) {
                if (b.at[0] - t.pos[0]).hypot(b.at[1] - t.pos[2]) > NEAR_BASE_M {
                    continue;
                }
                if t.sleeping {
                    b.sleeper = Some(tick);
                } else {
                    b.awake = Some(tick);
                }
            }
        }
    }

    /// Look at the next few mirror records: a wall, doorway or door at
    /// ground level, or a box, in the view cone and range with a clear
    /// line to its near side, is remembered with its base; a cupboard
    /// marks where a base is. `own` names what this body built. Bounded per
    /// frame and allocation-free.
    #[allow(clippy::too_many_arguments)]
    pub fn look(
        &mut self,
        core: &mut ClientCore,
        haven: &Haven,
        eye: [f32; 3],
        yaw: u16,
        tick: u32,
        own: impl Fn(u16, u16, u8, u8) -> bool,
    ) {
        let mut rays = RAID_RAYS_PER_FRAME;
        let (fx, fz) = yaw_dir(yaw);
        for k in 0..RAID_SCAN_PER_FRAME {
            let (np, nd) = (core.pieces.entries().len(), core.deploys.entries().len());
            if np == 0 && nd == 0 {
                return;
            }
            self.stats.examined += 1;
            let piece_turn = (k % 2 == 0 && np > 0) || nd == 0;
            // What it is: a face (and its shape's sides), a box, a cupboard.
            let (deploy, at, row, facing, dmg, what) = if piece_turn {
                self.piece = (self.piece + 1) % np;
                let r = core.pieces.entries()[self.piece];
                let def = (u16::from(r.row) < core.piece_defs_have)
                    .then(|| core.piece_defs.pieces[usize::from(r.row)]);
                let Some(def) = def.filter(|d| d.hp > 0 && shape_has_facing(d.shape)) else {
                    continue;
                };
                if r.level != 0 || !matches!(r.loc, LOC_EDGE_XLO | LOC_EDGE_ZLO) {
                    continue;
                }
                let at = OpAddr {
                    cx: r.cx,
                    cz: r.cz,
                    level: r.level,
                    loc: r.loc,
                };
                (false, at, def.hp, r.facing, r.dmg, What::Face(true))
            } else {
                self.deploy = (self.deploy + 1) % nd;
                let r = core.deploys.entries()[self.deploy];
                let have = core.deploy_defs_have.min(core.deploy_defs.def_count);
                if u16::from(r.row) >= have {
                    continue;
                }
                let def = core.deploy_defs.defs[usize::from(r.row)];
                let what = if arch_is_door(def.arch) && r.level == 0 {
                    What::Face(false)
                } else if def.arch == ARCH_BOX {
                    What::Box
                } else if def.arch == ARCH_HEARTH {
                    What::Hearth
                } else {
                    continue;
                };
                let at = OpAddr {
                    cx: r.cx,
                    cz: r.cz,
                    level: r.level,
                    loc: r.loc,
                };
                (true, at, def.hp, 0, r.dmg, what)
            };
            if own(at.cx, at.cz, at.level, at.loc) {
                continue;
            }
            let (ax, az) = if at.loc == LOC_PLANE {
                cell_center(at.cx, at.cz)
            } else {
                anchor(at.cx, at.cz, at.loc)
            };
            let (dx, dz) = (ax - eye[0], az - eye[2]);
            let d = dx.hypot(dz);
            let range = if what == What::Box {
                BOX_SIGHT_M
            } else {
                RAID_SIGHT_M
            };
            if d > range || d < 0.5 || dx * fx + dz * fz < d * std::f32::consts::FRAC_1_SQRT_2 {
                continue;
            }
            // Out of the base, for a face: the side of its edge with no
            // floor laid (a foundation shows), else the eye's side; a face
            // with floor on both sides is inside the base, and no way in.
            let out = match what {
                What::Face(_) => match outward(core, at, [eye[0], eye[2]]) {
                    Some(out) => out,
                    None => continue,
                },
                _ => [0.0, 0.0],
            };
            if rays == 0 {
                continue;
            }
            rays -= 1;
            self.stats.rays += 1;
            // A line to just short of its near side (or of the box): what
            // stands in front of it, not the thing itself.
            let short = if what == What::Box { 0.7 } else { 0.35 };
            let floor = eye[1] - 1.6;
            let target = [ax - dx / d * short, floor + 0.9, az - dz / d * short];
            if clear_line(core, haven, eye, target, range) != Sight::Clear {
                continue;
            }
            let b = self.base_for(ax, az, tick);
            let base = &mut self.bases[b];
            match what {
                What::Face(sided) => {
                    let face = Face {
                        deploy,
                        at,
                        hp_max: row,
                        dmg,
                        sided,
                        facing,
                        out,
                        seen: tick,
                    };
                    self.stats.faces += 1;
                    let same = |f: &Face| f.deploy == deploy && f.at == at;
                    let slot = base
                        .faces
                        .iter()
                        .position(|f| f.is_some_and(|f| same(&f)))
                        .or_else(|| base.faces.iter().position(Option::is_none))
                        .or_else(|| {
                            // Full: a weaker face takes the strongest one's row.
                            let (i, f) = base
                                .faces
                                .iter()
                                .enumerate()
                                .filter_map(|(i, f)| f.map(|f| (i, f)))
                                .max_by_key(|(_, f)| f.hp())?;
                            (face.hp() < f.hp()).then_some(i)
                        });
                    if let Some(i) = slot {
                        base.faces[i] = Some(face);
                    }
                }
                What::Box => {
                    self.stats.boxes_seen += 1;
                    if !base.boxes.contains(&Some(at)) {
                        if let Some(slot) = base.boxes.iter_mut().find(|s| s.is_none()) {
                            *slot = Some(at);
                        }
                    }
                }
                What::Hearth => {}
            }
        }
    }

    /// A face it planted on, or swung at, came down, or is gone from the
    /// mirror: forgotten.
    pub fn gone(&mut self, i: usize, face: &Face) {
        if let Some(b) = self.bases.get_mut(i) {
            for f in b.faces.iter_mut() {
                if f.is_some_and(|f| f.deploy == face.deploy && f.at == face.at) {
                    *f = None;
                }
            }
        }
    }

    /// The base to raid from `from` now, with its face and way in: one the
    /// temperament takes, the means pay for, not tried a moment ago and in
    /// a trip's reach; the cheapest, then the nearest.
    pub fn best(
        &self,
        means: &Means,
        temperament: Temperament,
        from: [f32; 2],
        tick: u32,
    ) -> Option<(usize, Face, Breach)> {
        let mut best: Option<(u32, f32, usize, Face, Breach)> = None;
        for (i, b) in self.bases.iter().enumerate() {
            if !b.used
                || b.tried
                    .is_some_and(|t| tick.wrapping_sub(t) < RAID_RETRY_TICKS)
            {
                continue;
            }
            let d = (b.at[0] - from[0]).hypot(b.at[1] - from[1]);
            if d > RAID_TRIP_M {
                continue;
            }
            let Some((face, way)) = cheapest(b, means) else {
                continue;
            };
            if !worth(temperament, way, b.offline(tick)) {
                continue;
            }
            let cost = match way {
                Breach::Swings(n) => u32::from(n),
                Breach::Satchels(n) => 10_000 + u32::from(n),
            };
            if best.is_none_or(|(c, bd, ..)| (cost, d) < (c, bd)) {
                best = Some((cost, d, i, face, way));
            }
        }
        best.map(|(_, _, i, f, b)| (i, f, b))
    }

    /// How many bases are worth raiding now, and where the nearest is.
    pub fn targets(
        &self,
        means: &Means,
        temperament: Temperament,
        from: [f32; 2],
        tick: u32,
    ) -> (u8, Option<[f32; 2]>) {
        let mut n = 0u8;
        let mut nearest: Option<(f32, [f32; 2])> = None;
        for (i, b) in self.bases.iter().enumerate() {
            let one = self.best_of(i, means, temperament, from, tick);
            if one {
                n = n.saturating_add(1);
                let d = (b.at[0] - from[0]).hypot(b.at[1] - from[1]);
                if nearest.is_none_or(|(m, _)| d < m) {
                    nearest = Some((d, b.at));
                }
            }
        }
        (n, nearest.map(|(_, at)| at))
    }

    fn best_of(
        &self,
        i: usize,
        means: &Means,
        temperament: Temperament,
        from: [f32; 2],
        tick: u32,
    ) -> bool {
        let b = &self.bases[i];
        b.used
            && b.tried
                .is_none_or(|t| tick.wrapping_sub(t) >= RAID_RETRY_TICKS)
            && (b.at[0] - from[0]).hypot(b.at[1] - from[1]) <= RAID_TRIP_M
            && cheapest(b, means).is_some_and(|(_, way)| worth(temperament, way, b.offline(tick)))
    }
}

/// Which way is out through an edge at `at`: the side whose cell has no
/// foundation, else (a wall in a field) the side `eye` is on; `None` with
/// floor on both sides.
fn outward(core: &ClientCore, at: OpAddr, eye: [f32; 2]) -> Option<[f32; 2]> {
    let (ax, az) = anchor(at.cx, at.cz, at.loc);
    let cols = core.pieces.cols();
    let (axis, before, after, eye_after) = match at.loc {
        LOC_EDGE_XLO => (
            [1.0, 0.0],
            at.cx.checked_sub(1).map(|x| (x, at.cz)),
            (at.cx, at.cz),
            eye[0] >= ax,
        ),
        LOC_EDGE_ZLO => (
            [0.0, 1.0],
            at.cz.checked_sub(1).map(|z| (at.cx, z)),
            (at.cx, at.cz),
            eye[1] >= az,
        ),
        _ => return None,
    };
    let floor = |c: Option<(u16, u16)>| c.is_some_and(|(x, z)| cols.plate(x, z).is_some());
    let neg = |v: [f32; 2]| [-v[0], -v[1]];
    match (floor(before), floor(Some(after))) {
        (true, true) => None,
        (true, false) => Some(axis),
        (false, true) => Some(neg(axis)),
        (false, false) => Some(if eye_after { axis } else { neg(axis) }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum What {
    /// A wall-like piece (sided) or a door.
    Face(bool),
    Box,
    Hearth,
}

/// What the raid wants next. `explorer.rs` sends the verbs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Raid {
    Go(Intent),
    /// Move a stack from the pack to the belt (`encode_action_move`).
    Belt {
        from: u8,
        to: u8,
        count: u16,
    },
    /// Plant the satchel in hand on this face (`encode_action_throw`).
    Throw {
        deploy: bool,
        at: OpAddr,
        intent: Intent,
    },
    /// Open this box (`encode_action_container`).
    Open {
        key: u32,
        intent: Intent,
    },
    /// Take a stack out of the open box (`encode_action_move`).
    Take {
        key: u32,
        from: u8,
        to: u8,
        count: u16,
        intent: Intent,
    },
    /// Shut the panel.
    Close(Intent),
    Done,
    Fail(Why),
}

/// Where the raid is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Leg {
    /// Walking to the face, then working it.
    #[default]
    Breach,
    /// A charge planted: clear of it until it goes off.
    Clear,
    /// Through the hole.
    Enter,
    /// Inside, at the boxes.
    Loot,
}

/// A box being emptied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lid {
    Aiming(u32),
    Opening(u32),
    Open,
    Moving(u32),
}

/// One raid, from the walk to the face to the last box.
#[derive(Clone, Copy, Debug, Default)]
pub struct RaidJob {
    target: Option<(usize, Face, Breach)>,
    leg: Leg,
    since: u32,
    held: Option<u32>,
    approach: Option<(f32, u32)>,
    belt: Option<u32>,
    /// A charge's plant in flight: when it went; the answer.
    planting: Option<u32>,
    planted_seen: bool,
    refused: Option<u8>,
    tries: u8,
    charges: u8,
    fires_at: u32,
    /// The box being opened, how, and how often.
    chest: Option<(OpAddr, Lid)>,
    box_tries: u8,
    moved: Option<bool>,
    done_boxes: [Option<OpAddr>; BOX_ROWS],
    /// Looking round inside since.
    looking: Option<(u32, u16)>,
    /// Units taken.
    pub taken: u32,
}

impl RaidJob {
    /// Between two actions: nothing waiting on its answer, no panel open.
    pub fn at_checkpoint(&self) -> bool {
        self.planting.is_none() && self.chest.is_none() && self.belt.is_none()
    }

    pub fn target(&self) -> Option<(usize, Face, Breach)> {
        self.target
    }

    /// The action asked for went out.
    pub fn sent(&mut self, tick: u32, what: &Raid) {
        match what {
            Raid::Throw { .. } => {
                self.planting = Some(tick);
                self.planted_seen = false;
                self.refused = None;
                self.held = None;
            }
            Raid::Belt { .. } => self.belt = Some(tick),
            Raid::Open { .. } => {
                if let Some((at, _)) = self.chest {
                    self.chest = Some((at, Lid::Opening(tick)));
                }
            }
            Raid::Take { .. } => {
                if let Some((at, _)) = self.chest {
                    self.chest = Some((at, Lid::Moving(tick)));
                    self.moved = None;
                }
            }
            Raid::Close(_) => {
                if let Some((at, _)) = self.chest.take() {
                    if let Some(s) = self.done_boxes.iter_mut().find(|s| s.is_none()) {
                        *s = Some(at);
                    }
                }
            }
            _ => {}
        }
    }

    /// A charge went on at this address (`ChargePlaced`, island-wide).
    pub fn on_charge(&mut self, deploy: bool, cx: u16, cz: u16, level: u8, loc: u8) {
        if let (Some(_), Some((_, face, _))) = (self.planting, self.target) {
            if face.deploy == deploy
                && (face.at.cx, face.at.cz, face.at.level, face.at.loc) == (cx, cz, level, loc)
            {
                self.planted_seen = true;
            }
        }
    }

    /// A build refusal: the plant's, while one is in flight.
    pub fn on_refused(&mut self, reason: u8) {
        if self.planting.is_some() {
            self.refused = Some(reason);
        }
    }

    /// A box's panel opened.
    pub fn on_panel(&mut self, handle: u32) {
        if let Some((at, Lid::Opening(_))) = self.chest {
            if box_key(at.cx, at.cz, at.level) == handle {
                self.chest = Some((at, Lid::Open));
            }
        }
    }

    /// A move's answer.
    pub fn on_moved(&mut self, refused: bool) {
        if let Some((at, Lid::Moving(_))) = self.chest {
            self.moved = Some(!refused);
            self.chest = Some((at, Lid::Open));
        }
    }

    /// Does the face still stand, per the mirror?
    fn stands(core: &ClientCore, face: &Face) -> bool {
        let at = face.at;
        let same = |cx: u16, cz: u16, level: u8, loc: u8| {
            (cx, cz, level, loc) == (at.cx, at.cz, at.level, at.loc)
        };
        if face.deploy {
            core.deploys
                .entries()
                .iter()
                .any(|r| same(r.cx, r.cz, r.level, r.loc))
        } else {
            core.pieces
                .entries()
                .iter()
                .any(|r| same(r.cx, r.cz, r.level, r.loc))
        }
    }

    /// The damage band drawn on the face now.
    fn dmg(core: &ClientCore, face: &Face) -> u8 {
        let at = face.at;
        let same = |cx: u16, cz: u16, level: u8, loc: u8| {
            (cx, cz, level, loc) == (at.cx, at.cz, at.level, at.loc)
        };
        if face.deploy {
            core.deploys
                .entries()
                .iter()
                .find(|r| same(r.cx, r.cz, r.level, r.loc))
                .map_or(face.dmg, |r| r.dmg)
        } else {
            core.pieces
                .entries()
                .iter()
                .find(|r| same(r.cx, r.cz, r.level, r.loc))
                .map_or(face.dmg, |r| r.dmg)
        }
    }

    /// Walk to a spot by a route, the last steps straight to within
    /// [`STAND_NEAR_M`] or until `there` says so. `None` once there.
    #[allow(clippy::too_many_arguments)]
    fn walk_to(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        route: &mut Route,
        p: [f32; 2],
        hurry: bool,
        there: bool,
        tick: u32,
    ) -> Option<Result<Intent, Why>> {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let left = (p[0] - x).hypot(p[1] - z);
        if there {
            self.approach = None;
            return None;
        }
        if left > 0.6 {
            self.approach = None;
            return Some(match route.to(core, body, p, 0.5, hurry, tick) {
                step @ Step::Walk { .. } => Ok(step.walk().unwrap_or(Intent::IDLE)),
                Step::Wait | Step::Arrived => Ok(Intent::walk(yaw_toward(p[0] - x, p[1] - z))),
                Step::Blocked => Err(Why::Stuck),
            });
        }
        if left <= STAND_NEAR_M {
            self.approach = None;
            return None;
        }
        match self.approach {
            Some((best, since)) if left > best - 0.02 => {
                if tick.wrapping_sub(since) >= STALL_TICKS {
                    self.approach = None;
                    return Some(Err(Why::Stuck));
                }
            }
            _ => self.approach = Some((left, tick)),
        }
        Some(Ok(Intent::walk(yaw_toward(p[0] - x, p[1] - z))))
    }

    /// Give the raid up: the base is let be a while.
    fn give_up(&mut self, bases: &mut Bases, why: Why, tick: u32) -> Raid {
        if let Some((i, ..)) = self.target {
            bases.tried(i, tick);
        }
        Raid::Fail(why)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        bases: &mut Bases,
        means: &Means,
        temperament: Temperament,
        tick: u32,
    ) -> Raid {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        if self.target.is_none() {
            let Some(t) = bases.best(means, temperament, [x, z], tick) else {
                return Raid::Fail(Why::NotFound);
            };
            self.target = Some(t);
            self.leg = Leg::Breach;
            self.since = tick;
            bases.stats.raids += 1;
        }
        let Some((base, face, _)) = self.target else {
            return Raid::Fail(Why::NotFound);
        };
        match self.leg {
            Leg::Breach | Leg::Clear if !Self::stands(core, &face) => {
                // Down: the way in is open.
                bases.stats.breached += 1;
                bases.gone(base, &face);
                self.leg = Leg::Enter;
                self.since = tick;
                self.planting = None;
                Raid::Go(Intent::IDLE)
            }
            Leg::Breach => self.work(core, seed, haven, body, hands, route, bases, means, tick),
            Leg::Clear => {
                let far = means
                    .satchel
                    .map_or(3.0, |(_, t)| f32::from(t.blast_cm) * 0.01)
                    + CLEAR_SLACK_M;
                let p = face.outside(far);
                let look = Intent {
                    look: Look::Point(look_point(seed, haven, core, face.at)),
                    ..Intent::IDLE
                };
                if tick.wrapping_sub(self.fires_at) < u32::MAX / 2 {
                    // Gone off and it stands: another, if the pack has one.
                    let dmg = Self::dmg(core, &face);
                    let left = Face { dmg, ..face };
                    match breach(&left, means) {
                        Some(next) if self.charges < MAX_CHARGES => {
                            self.target = Some((base, left, next));
                            self.leg = Leg::Breach;
                            self.since = tick;
                            Raid::Go(look)
                        }
                        _ => self.give_up(bases, Why::MissingInputs, tick),
                    }
                } else {
                    match self.walk_to(core, body, route, p, true, false, tick) {
                        Some(Ok(intent)) => Raid::Go(intent),
                        // Nowhere further to go: as far as it got.
                        Some(Err(_)) | None => Raid::Go(look),
                    }
                }
            }
            Leg::Enter => {
                // Just past where the face stood: inside, whatever stands
                // in the room.
                let p = face.outside(-ENTER_M);
                if tick.wrapping_sub(self.since) >= ENTER_TICKS {
                    return self.give_up(bases, Why::Stuck, tick);
                }
                let there = (p[0] - x).hypot(p[1] - z) <= 0.6;
                match self.walk_to(core, body, route, p, false, there, tick) {
                    Some(Ok(intent)) => Raid::Go(intent),
                    Some(Err(why)) => self.give_up(bases, why, tick),
                    None => {
                        self.leg = Leg::Loot;
                        self.since = tick;
                        self.looking = None;
                        Raid::Go(Intent::IDLE)
                    }
                }
            }
            Leg::Loot => self.loot(core, seed, haven, body, hands, route, bases, tick),
        }
    }

    /// At the face: to the spot outside it, then a charge planted with the
    /// satchel in hand, or blows with the weapon until it is down.
    #[allow(clippy::too_many_arguments)]
    fn work(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        bases: &mut Bases,
        means: &Means,
        tick: u32,
    ) -> Raid {
        let Some((_, face, way)) = self.target else {
            return Raid::Fail(Why::NotFound);
        };
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        // A plant in flight: the charge on the wall, a refusal, or nothing.
        if let Some(at) = self.planting {
            if self.planted_seen {
                self.planting = None;
                self.charges += 1;
                bases.stats.charges += 1;
                let fuse = means.satchel.map_or(0, |(_, t)| u32::from(t.fuse_ticks));
                self.fires_at = tick.wrapping_add(fuse + FUSE_SLACK_TICKS);
                self.leg = Leg::Clear;
                self.tries = 0;
                return Raid::Go(Intent::IDLE);
            }
            if self.refused.take().is_some() {
                self.planting = None;
                return self.give_up(bases, Why::Refused, tick);
            }
            if tick.wrapping_sub(at) < VERDICT_TICKS {
                return Raid::Go(Intent::IDLE);
            }
            self.planting = None;
            self.tries += 1;
            if self.tries >= 2 {
                return self.give_up(bases, Why::NoAnswer, tick);
            }
        }
        let p = face.outside(STAND_OUT_M);
        let picks = nearest_structure(core, x, z) == Some((face.deploy, face.at));
        let there = match way {
            Breach::Satchels(_) => picks,
            Breach::Swings(_) => false,
        };
        match self.walk_to(core, body, route, p, false, there, tick) {
            Some(Ok(intent)) => return Raid::Go(intent),
            Some(Err(why)) => return self.give_up(bases, why, tick),
            None => {}
        }
        let item = match way {
            Breach::Satchels(_) => means.satchel.map(|(item, _)| item),
            Breach::Swings(_) => means.melee.map(|(item, ..)| item),
        };
        let Some(item) = item else {
            return self.give_up(bases, Why::NoTool, tick);
        };
        // In hand: on the belt first.
        let slot = (0..HOTBAR_SLOTS).find(|&i| core.inv[i].count > 0 && core.inv[i].item == item);
        let Some(slot) = slot else {
            if let Some(at) = self.belt {
                if tick.wrapping_sub(at) < VERDICT_TICKS {
                    return Raid::Go(Intent::IDLE);
                }
                self.belt = None;
                return self.give_up(bases, Why::NoTool, tick);
            }
            return match belt_move(core, item) {
                Some((from, to, count)) => Raid::Belt { from, to, count },
                None => self.give_up(bases, Why::NoTool, tick),
            };
        };
        self.belt = None;
        let intent = Intent {
            look: Look::Point(look_point(seed, haven, core, face.at)),
            sel: Some(slot as u8),
            ..Intent::IDLE
        };
        let held = *self.held.get_or_insert(tick);
        let ready = tick.wrapping_sub(held) >= HOLD_TICKS && hands.settled();
        match way {
            Breach::Satchels(_) => {
                if !picks {
                    // Not what `X` takes from where the feet ended up.
                    return self.give_up(bases, Why::NoSpot, tick);
                }
                if ready {
                    return Raid::Throw {
                        deploy: face.deploy,
                        at: face.at,
                        intent,
                    };
                }
                Raid::Go(intent)
            }
            Breach::Swings(n) => {
                // Blows until it is down, for as long as they should take.
                let cadence = means.melee.map_or(TICK_HZ, |(.., c)| u32::from(c.max(1)));
                let budget = u32::from(n) * cadence * 2 + 10 * TICK_HZ;
                if tick.wrapping_sub(self.since) >= budget {
                    return self.give_up(bases, Why::Stuck, tick);
                }
                if ready && hands.on_target() {
                    return Raid::Go(Intent {
                        buttons: BTN_PRIMARY,
                        ..intent
                    });
                }
                Raid::Go(intent)
            }
        }
    }

    /// Inside: each box seen opened, emptied into the pack as far as it
    /// fits, and shut; a look round for boxes not seen yet; then done.
    #[allow(clippy::too_many_arguments)]
    fn loot(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        bases: &mut Bases,
        tick: u32,
    ) -> Raid {
        let Some((base, ..)) = self.target else {
            return Raid::Fail(Why::NotFound);
        };
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        if let Some((at, lid)) = self.chest {
            let key = box_key(at.cx, at.cz, at.level);
            let intent = Intent {
                look: Look::Point(aim_point(core, seed, haven, at, x, z)),
                ..Intent::IDLE
            };
            match lid {
                Lid::Aiming(since) => {
                    let gone =
                        !core.deploys.entries().iter().any(|r| {
                            (r.cx, r.cz, r.level, r.loc) == (at.cx, at.cz, at.level, at.loc)
                        });
                    if gone || self.box_tries >= BOX_TRIES {
                        self.chest = None;
                        if let Some(s) = self.done_boxes.iter_mut().find(|s| s.is_none()) {
                            *s = Some(at);
                        }
                        return Raid::Go(Intent::IDLE);
                    }
                    let (cx, cz) = cell_center(at.cx, at.cz);
                    match self.walk_to(
                        core,
                        body,
                        route,
                        [cx, cz],
                        false,
                        (cx - x).hypot(cz - z) <= BOX_STAND_M,
                        tick,
                    ) {
                        Some(Ok(walk)) => return Raid::Go(walk),
                        Some(Err(_)) => {
                            self.box_tries = BOX_TRIES;
                            return Raid::Go(Intent::IDLE);
                        }
                        None => {}
                    }
                    if tick.wrapping_sub(since) >= HOLD_TICKS
                        && hands.settled()
                        && e_picks_by(core, x, z, hands.view().0, at, 0.0)
                    {
                        return Raid::Open { key, intent };
                    }
                    if tick.wrapping_sub(since) >= VERDICT_TICKS {
                        self.box_tries += 1;
                        self.chest = Some((at, Lid::Aiming(tick)));
                    }
                    Raid::Go(intent)
                }
                Lid::Opening(sent) => {
                    if tick.wrapping_sub(sent) >= VERDICT_TICKS {
                        self.box_tries += 1;
                        self.chest = Some((at, Lid::Aiming(tick)));
                    }
                    Raid::Go(intent)
                }
                Lid::Moving(sent) => {
                    if tick.wrapping_sub(sent) >= VERDICT_TICKS {
                        self.chest = Some((at, Lid::Open));
                        self.moved = Some(false);
                    }
                    Raid::Go(intent)
                }
                Lid::Open => {
                    if core.cont_kind != CONT_BOX || core.cont_handle != key {
                        // The panel shut under it.
                        self.box_tries += 1;
                        self.chest = Some((at, Lid::Aiming(tick)));
                        return Raid::Go(intent);
                    }
                    let refused = self.moved.take() == Some(false);
                    match take_from(core).filter(|_| !refused) {
                        Some((from, to, count)) => {
                            self.taken = self.taken.saturating_add(u32::from(count));
                            bases.stats.taken += u64::from(count);
                            Raid::Take {
                                key,
                                from,
                                to,
                                count,
                                intent,
                            }
                        }
                        None => {
                            bases.stats.boxes += 1;
                            Raid::Close(intent)
                        }
                    }
                }
            }
        } else {
            let next = bases.get(base).and_then(|b| {
                b.boxes
                    .iter()
                    .flatten()
                    .find(|a| !self.done_boxes.contains(&Some(**a)))
                    .copied()
            });
            if let Some(at) = next {
                self.chest = Some((at, Lid::Aiming(tick)));
                self.box_tries = 0;
                self.looking = None;
                return Raid::Go(Intent::IDLE);
            }
            // None seen yet from in here: a slow turn round, then done.
            let (since, from) = *self.looking.get_or_insert((tick, hands.view().0));
            let t = tick.wrapping_sub(since);
            if t >= LOOK_TICKS {
                if let Some((i, ..)) = self.target {
                    bases.tried(i, tick);
                }
                return Raid::Done;
            }
            let turn = (t * 65536 / LOOK_TICKS) as u16;
            Raid::Go(Intent {
                look: Look::Heading(from.wrapping_add(turn)),
                ..Intent::IDLE
            })
        }
    }
}

/// The next stack to take out of the open box: its slot, the pack slot it
/// goes to (onto a stack of the same item with room, else an empty slot,
/// the pack before the belt) and how many.
pub fn take_from(core: &ClientCore) -> Option<(u8, u8, u16)> {
    if core.cont_kind != CONT_BOX {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn satchel(n: u32) -> Means {
        Means {
            satchel: Some((
                7,
                Throw {
                    damage: 475,
                    structure: 125,
                    fuse_ticks: 300,
                    reach_cm: 1000,
                    blast_cm: 300,
                },
            )),
            satchels: n,
            melee: Some((1, 1, 30)),
        }
    }

    fn face(hp_max: u16, dmg: u8, sided: bool) -> Face {
        Face {
            deploy: !sided,
            at: OpAddr {
                cx: 100,
                cz: 100,
                level: 0,
                loc: LOC_EDGE_ZLO,
            },
            hp_max,
            dmg,
            sided,
            // Soft toward +z, the way a base built from inside on the +z
            // side faces; seen from -z, outside.
            facing: 1,
            out: [0.0, -1.0],
            seen: 0,
        }
    }

    #[test]
    fn a_face_is_priced_in_satchels_or_blows_by_what_it_looks_like() {
        // A wooden door: two satchels, far too many blows of a rock.
        let door = face(200, 0, false);
        assert_eq!(breach(&door, &satchel(3)), Some(Breach::Satchels(2)));
        assert_eq!(breach(&door, &satchel(1)), None, "one is not enough");
        // Banded at 5 of 7 lost: at most 86 hp left, one satchel.
        let hurt = face(200, 5, false);
        assert_eq!(hurt.hp(), 200 - 4 * 200 / 7);
        assert_eq!(breach(&hurt, &satchel(1)), Some(Breach::Satchels(1)));
        // A twig wall from its hard side: the game's floor per blow, ten.
        let twig = face(10, 0, true);
        assert_eq!(breach(&twig, &satchel(0)), Some(Breach::Swings(10)));
        // A stone wall: four satchels, a strong face.
        let stone = face(500, 0, true);
        assert_eq!(breach(&stone, &satchel(6)), Some(Breach::Satchels(4)));
        assert!(!worth(Temperament::Opportunist, Breach::Satchels(4), false));
        assert!(worth(Temperament::Opportunist, Breach::Satchels(4), true));
        assert!(worth(Temperament::Opportunist, Breach::Satchels(2), false));
        assert!(worth(Temperament::Kos, Breach::Satchels(4), false));
        for t in [Temperament::Passive, Temperament::Defensive] {
            assert!(!worth(t, Breach::Swings(1), true));
        }
    }

    #[test]
    fn the_cheapest_base_worth_it_is_chosen_and_one_tried_is_let_be() {
        let mut bases = Bases::new();
        let a = bases.base_for(100.0, 100.0, 0);
        let b = bases.base_for(200.0, 100.0, 0);
        assert_ne!(a, b);
        assert_eq!(bases.base_for(105.0, 100.0, 5), a, "the same base");
        bases.bases[a].faces[0] = Some(face(500, 0, true));
        bases.bases[b].faces[0] = Some(face(500, 0, true));
        bases.bases[b].faces[1] = Some(face(200, 0, false));
        let means = satchel(4);
        let from = [150.0, 100.0];
        // Base a is stone all round: four satchels, not weak and somebody
        // was about it; base b has a wooden door.
        bases.bases[a].awake = Some(0);
        let pick = bases.best(&means, Temperament::Opportunist, from, 10);
        assert_eq!(pick.map(|(i, _, w)| (i, w)), Some((b, Breach::Satchels(2))));
        assert_eq!(
            bases.targets(&means, Temperament::Opportunist, from, 10).0,
            1
        );
        assert_eq!(bases.targets(&means, Temperament::Kos, from, 10).0, 2);
        assert_eq!(bases.targets(&means, Temperament::Defensive, from, 10).0, 0);
        // Asleep in it: offline-looking, and worth it after all.
        bases.bases[a].sleeper = Some(20);
        assert_eq!(
            bases.targets(&means, Temperament::Opportunist, from, 20).0,
            2
        );
        bases.tried(b, 30);
        let pick = bases.best(&means, Temperament::Opportunist, from, 31);
        assert_eq!(pick.map(|(i, ..)| i), Some(a));
        let later = 30 + RAID_RETRY_TICKS;
        assert_eq!(
            bases
                .targets(&means, Temperament::Opportunist, from, later)
                .0,
            2
        );
        // Out of a trip's reach.
        let far = [100.0 + RAID_TRIP_M + 150.0, 100.0];
        assert_eq!(bases.targets(&means, Temperament::Kos, far, later).0, 0);
    }
}
