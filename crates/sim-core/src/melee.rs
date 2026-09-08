//! Melee aim — the swing follows the look ray.
//!
//! Until 2026-09-05 a swing was resolved in a plane: `gather::swing`,
//! `combat::strike`, `mob::strike` and `combat::raid` each scanned their
//! own store for the nearest thing inside a 30° cone about the body's yaw,
//! within a ±3 m vertical window, and pitch was never read. A spear driven
//! at the sky landed on the person in front of you, a swing at the ground
//! felled the tree behind it, and the hitmarker paid the identity rung on
//! every blow because there was no line to cross. The operator's call was
//! *"we need to step back and make this a PROPER VIDEO GAME"*, and this
//! module is that: **one cast from the eye along (yaw, pitch), for the
//! weapon's reach, and the nearest thing it enters is what the swing hit.**
//!
//! # It is the shot's own machinery, pointed at the arm
//!
//! Nothing here is a second hit model. Bodies are found by
//! `ranged::nearest_body` — the same planar quadratic, the same rewound
//! pose, the same span handed to `ranged::part_crossed` for the head and
//! limb bands, so a spear and an arrow cannot disagree about where a skull
//! is. The world is walked by `ranged::world_stop` — ground, then scenery,
//! then built pieces, at `ARROW_STEP_MM` — so a swing stops on exactly the
//! boulder an arrow stops on, and a wall it reaches comes back as the same
//! `Struck` a bullet hands `World::chip`. What is new is small: an exact
//! ray–cylinder solve for the scatter's own occupants ([`node_cast`]),
//! because a gather node has a *cell* to pay out of and the sampled walk
//! only knows that *something* blocked; the same solve for the animal
//! roster ([`mob_cast`]); and the arbitration in [`cast`].
//!
//! # Nearest along the ray wins
//!
//! The old order — node, then player, then animal, then structure — was a
//! rule about which store to ask first, needed because a planar cone could
//! not tell "in the way" from "in the sector". A ray can, so there is no
//! order: whatever the segment enters first is what the arm reached, and
//! everything behind it is shadowed. A player standing in front of a tree
//! takes the blow; a tree standing in front of a player is cover; a torch
//! swung at a stone node thunks into the stone and never reaches the wall
//! behind it. `Swing::Refused`'s whole fall-through contract retired with
//! this, because the case it existed for — a node in the cone eating a
//! swing meant for the person beside it — cannot happen to a ray.
//!
//! # Reach is per kind and the ray is the longest of them
//!
//! `gather::REACH_M` is the arm at a node for every tool (a spoken default);
//! the melee row's `reach_cm` is the arm at a body, an animal and a wall.
//! The ray is cast to the longest, each kind is accepted only inside its
//! own, and a target that is nearest but out of its own reach ends the
//! swing as a whiff — it is still in the way, so nothing behind it is hit
//! either. A bare hand therefore still *finds* the tree it cannot fell,
//! which is what lets the node refuse it with a reason.
//!
//! # What deliberately did not change
//!
//! The weak-spot mark is still a sector around the node judged by where the
//! swinger *stands* (`gather::WEAK_COS`), because that is a mechanic about
//! footwork and the HUD teaches it as one. The swing cadence, the
//! refusals, the payout schedule and the barrel are all `gather::land`'s
//! and untouched. And the sim still knows nothing of a chop or a thrust —
//! the client's `Stroke` is a picture, this is the law it now agrees with.
//!
//! # The client mirrors it by calling it
//!
//! `ui::interact::resolve_swing` used to restate the planar scan; it calls
//! [`node_cast`] now, over its own `Occupants`, with the same quantized yaw
//! and pitch the wire carries. One implementation on both sides is the
//! quantize-both-sides law applied to aiming: the prompt cannot offer a
//! tree the server's ray misses.
//!
//! Wall 1 throughout: `+ − × ÷ sqrt min max` and the two LUTs. Wall 2:
//! nothing here allocates. Wall 4: the node cast is nine cells, the mob
//! cast `MAX_MOBS`, the body cast `MAX_PLAYERS`, and the world walk is
//! bounded by the reach over `ARROW_STEP_MM`, capped at
//! `MAX_HITSCAN_SAMPLES` — all on a swing tick only.

use crate::combat::CombatContent;
use crate::fmath::floor_i32;
use crate::gather::{self, REACH_M};
use crate::limits::{ARROW_STEP_MM, MAX_HITSCAN_SAMPLES, MAX_PLAYERS};
use crate::mob::{MobContent, Mobs};
use crate::movement::{Body, POS_XZ_Q, POS_Y_Q};
use crate::occupy::Occupants;
use crate::pitch_lut::pitch_dir;
use crate::ranged::{self, BodyHit, Struck, ARROW_EYE_MM, MM_PER_M};
use crate::rewind::Rewind;
use crate::terrain::{self, Haven, Occupant, Slot, CELL_SIZE, SLOT_SCALE_MAX};
use crate::world::Player;
use crate::yaw_lut::yaw_dir;

/// The forgiveness a swing gets at a scatter occupant, metres — added to the
/// occupant's collision radius before the ray is tested against it. **(knob)**
/// `DECISIONS.md` §open, "melee aim v1".
///
/// The reference resolves melee as a sphere cast rather than a line for
/// exactly this: a line that grazes a trunk by a centimetre is a miss the
/// player did not make. Ten centimetres is the width of a hatchet's bit.
/// Bodies and animals get none of it — a 0.4 m capsule is already generous
/// (`collide::CAPSULE_RADIUS_M`), and an animal's volume is content's to
/// size. Pieces get it through `world_stop`'s probe, so a swing and a shot
/// stop on the same plank at the same depth.
pub const MELEE_PROBE_M: f32 = 0.10;

/// The volume a swing tests a **bush** against, `(radius, top)` metres at
/// slot scale 1 — because `terrain::occupant_volume` gives the bush none.
/// **(knob)** `DECISIONS.md` §open, "melee aim v1".
///
/// A bush blocks no body on purpose (you walk through it), so the movement
/// volume is zero and a ray through zero hits nothing: the one gatherable
/// you could always swing at would have become unswingable. These are read
/// off what the client draws — `render/props.rs`'s leaf cards put the mass
/// at ~1.5 m across and ~1.4 m tall (`BUSH_CARD_HALF`), so the radius is
/// three-quarters of that half-width and the top a little under the crown.
pub const BUSH_SWING_R_M: f32 = 0.55;
/// See [`BUSH_SWING_R_M`].
pub const BUSH_SWING_TOP_M: f32 = 1.2;

/// The volume a **swing** tests an occupant against, `(radius, top)` metres
/// at slot scale 1: `terrain::occupant_volume` — the same cylinder a body
/// and an arrow collide with — except for the bush, which has a swing volume
/// and no collision volume. One law with one stated exception, so a tree is
/// exactly as wide to a hatchet as it is to a shoulder.
pub const fn swing_volume(o: Occupant) -> (f32, f32) {
    match o {
        Occupant::Bush => (BUSH_SWING_R_M, BUSH_SWING_TOP_M),
        other => terrain::occupant_volume(other),
    }
}

// The node cast reads the 3×3 ring around the EYE's cell and nothing wider,
// so the ring has to contain every occupant a ray of the longest node reach
// can enter. The nearest edge of the ring is at least one cell away from any
// point of the centre cell; the furthest a swing can reach into an
// occupant's inflated cylinder is the reach plus the widest scaled radius
// plus the probe. This is `terrain::OCCUPANT_PROBE_CELLS`'s own argument,
// with the arm in place of the capsule.
const _: () = {
    let mut widest = 0.0f32;
    let mut i = 0;
    while i < terrain::OCCUPANT_R_M.len() {
        if terrain::OCCUPANT_R_M[i] > widest {
            widest = terrain::OCCUPANT_R_M[i];
        }
        i += 1;
    }
    if BUSH_SWING_R_M > widest {
        widest = BUSH_SWING_R_M;
    }
    assert!(
        REACH_M + widest * SLOT_SCALE_MAX + MELEE_PROBE_M < CELL_SIZE,
        "a node reach this long can enter an occupant outside the 3x3 ring"
    );
    assert!(terrain::OCCUPANT_PROBE_CELLS >= 1);
};

/// A swing's line of attack, in **millimetres** — `ranged`'s units, because
/// both solvers this module borrows are millimetre functions and a swing
/// must agree with a shot to the bit about where the world is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    /// The eye: the body's quanta as whole millimetres, plus
    /// [`ARROW_EYE_MM`] — the same origin a bow and a gun fire from, so a
    /// swing clears exactly the cover its owner can see over.
    pub o: (f32, f32, f32),
    /// The whole reach as one segment, so a fraction `t` of it is a distance
    /// `t · len_mm`.
    pub s: (f32, f32, f32),
    /// `|s|`.
    pub len_mm: f32,
}

impl Ray {
    /// The point at fraction `t`, in metres.
    #[inline]
    pub fn at_m(&self, t: f32) -> (f32, f32, f32) {
        (
            (self.o.0 + self.s.0 * t) / MM_PER_M,
            (self.o.1 + self.s.1 * t) / MM_PER_M,
            (self.o.2 + self.s.2 * t) / MM_PER_M,
        )
    }
}

/// The ray a body swings along: from its eye, the way it is looking, for
/// `len_mm`. `yaw` and `pitch` are the wire's bytes and go through the two
/// LUTs the shot path uses, so there is no trig and no second opinion about
/// which way a byte points.
pub fn ray(body: &Body, yaw: u16, pitch: u8, len_mm: f32) -> Ray {
    let (fx, fz) = yaw_dir(yaw);
    let (ch, sv) = pitch_dir(pitch);
    // `hitscan`'s own arithmetic for the muzzle, verbatim: quanta to whole
    // millimetres in integers, then the eye lift.
    let o = (
        (body.qx * (POS_XZ_Q * MM_PER_M) as i32) as f32,
        (body.qy * (POS_Y_Q * MM_PER_M) as i32 + ARROW_EYE_MM) as f32,
        (body.qz * (POS_XZ_Q * MM_PER_M) as i32) as f32,
    );
    Ray {
        o,
        s: (fx * ch * len_mm, sv * len_mm, fz * ch * len_mm),
        len_mm,
    }
}

/// How far the hand reaches each kind of target, millimetres. Zero means
/// the hand cannot reach that kind at all — it still *finds* it, and the
/// swing ends there as a whiff.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reaches {
    /// A gather node or a barrel: `gather::REACH_M`, whatever is in hand.
    pub node: f32,
    /// A player or an animal: the melee row's `reach_cm`.
    pub body: f32,
    /// A built piece or a solid deployable: the row's `reach_cm` again, but
    /// only for a row with a `structure` column.
    pub structure: f32,
}

impl Reaches {
    /// The reaches for whatever `held` is, off the combat table.
    pub fn for_hand(cc: &CombatContent, held: u16) -> Self {
        Self {
            node: REACH_M * MM_PER_M,
            body: cc
                .held_melee(held)
                .map_or(0.0, |d| f32::from(d.reach_cm) * 10.0),
            structure: cc
                .held_struct(held)
                .map_or(0.0, |d| f32::from(d.reach_cm) * 10.0),
        }
    }

    /// How long the ray has to be to find everything any kind can reach.
    pub fn longest(&self) -> f32 {
        self.node.max(self.body).max(self.structure)
    }
}

/// A scatter occupant the ray entered — any kind the caller asked for.
#[derive(Clone, Copy, Debug)]
pub struct OccupantHit {
    pub cx: u16,
    pub cz: u16,
    /// The slot as the cache resolved it — carried rather than re-queried,
    /// for `gather`'s old `Target` reason: a second lookup is a second
    /// chance to disagree with the thing that was hit.
    pub slot: Slot,
    /// Segment fraction where the ray enters the occupant's swing volume.
    pub t: f32,
}

/// A swingable occupant the ray entered: an [`OccupantHit`] with the row
/// `gather::land` pays out of.
#[derive(Clone, Copy, Debug)]
pub struct NodeHit {
    pub cx: u16,
    pub cz: u16,
    /// See [`OccupantHit::slot`].
    pub slot: Slot,
    /// `gather`'s target index: a node's gatherable row, or the barrel.
    pub ni: usize,
    /// Segment fraction where the ray enters the occupant's swing volume.
    pub t: f32,
}

impl NodeHit {
    /// Where the blow scuffs the occupant, metres: the entry point pulled
    /// back onto the occupant's **collision** skin, at the ray's own height
    /// clamped into the occupant's span. `None` for an occupant with no
    /// collision skin — the bush — which has a swing volume and nothing to
    /// draw a mark on.
    ///
    /// The entry point itself sits up to [`MELEE_PROBE_M`] outside the bark,
    /// because the forgiveness inflates the cylinder the ray is tested
    /// against; a decal floating a hand's width off the trunk would read as
    /// a miss drawn as a hit, so the point is projected in from the slot's
    /// axis to the true radius. The direction is slot→entry, which is what
    /// `render/decal.rs::facing` re-derives at the other end.
    pub fn skin(&self, ray: &Ray) -> Option<(f32, f32, f32)> {
        let (r, top) = terrain::occupant_volume(self.slot.occupant);
        if r <= 0.0 || top <= 0.0 {
            return None;
        }
        let (px, py, pz) = ray.at_m(self.t);
        let (dx, dz) = (px - self.slot.x, pz - self.slot.z);
        let d2 = dx * dx + dz * dz;
        let rr = r * self.slot.scale;
        let (mx, mz) = if d2 > rr * rr {
            let k = rr / d2.sqrt();
            (self.slot.x + dx * k, self.slot.z + dz * k)
        } else {
            (px, pz)
        };
        let my = py.max(self.slot.y).min(self.slot.y + top * self.slot.scale);
        Some((mx, my, mz))
    }
}

/// An animal the ray entered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MobHit {
    pub slot: usize,
    /// Segment fraction where the ray enters the animal's volume.
    pub t: f32,
}

/// Where a ray of `o + u·t` (metres) is inside a finite vertical cylinder —
/// `[t_in, t_out]` clipped to the segment, or `None` if it never is.
///
/// The planar half is the quadratic `ranged::nearest_body` finishes for a
/// player; the vertical half is a slab. A ray starting inside enters at 0,
/// so a swinger standing in a bush hits it at once. Wall 1 only.
pub fn cylinder_span(
    o: (f32, f32, f32),
    u: (f32, f32, f32),
    centre: (f32, f32),
    r: f32,
    base: f32,
    top: f32,
) -> Option<(f32, f32)> {
    let (wx, wz) = (o.0 - centre.0, o.2 - centre.1);
    let vv = u.0 * u.0 + u.2 * u.2;
    let ww = wx * wx + wz * wz;
    let (p_in, p_out) = if vv <= 0.0 {
        // Straight up or down: the ray is one column, inside or not.
        if ww >= r * r {
            return None;
        }
        (0.0, 1.0)
    } else {
        let wv = wx * u.0 + wz * u.2;
        let disc = wv * wv - vv * (ww - r * r);
        if disc < 0.0 {
            return None;
        }
        let half = disc.sqrt() / vv;
        let centre_t = -wv / vv;
        (centre_t - half, centre_t + half)
    };
    let (v_in, v_out) = if u.1 == 0.0 {
        if o.1 < base || o.1 >= top {
            return None;
        }
        (0.0, 1.0)
    } else {
        let a = (base - o.1) / u.1;
        let b = (top - o.1) / u.1;
        (a.min(b), a.max(b))
    };
    let t_in = p_in.max(v_in).max(0.0);
    let t_out = p_out.min(v_out).min(1.0);
    (t_in <= t_out).then_some((t_in, t_out))
}

/// The nearest swingable occupant the ray enters — a gather node, a bush or
/// a barrel — over the 3×3 cells around the eye, skipping harvested cells.
/// `None` when the ray enters none.
///
/// [`occupant_cast`] over `gather`'s own target set, so the sim and the
/// client's prompt (`ui::interact::resolve_swing` calls this) agree about
/// what a swing can land on by construction rather than by two predicates
/// kept in step.
pub fn node_cast(seed: u64, occ: &mut Occupants, ray: &Ray) -> Option<NodeHit> {
    let hit = occupant_cast(seed, occ, ray, |o| gather::target_index(o).is_some())?;
    Some(NodeHit {
        cx: hit.cx,
        cz: hit.cz,
        slot: hit.slot,
        // `Some` by the predicate above; the fallback is unreachable and
        // says so rather than unwrapping on the hot path.
        ni: gather::target_index(hit.slot.occupant).unwrap_or(0),
        t: hit.t,
    })
}

/// The nearest occupant of a kind `want` accepts that the ray enters, over
/// the 3×3 cells around the eye, skipping harvested cells. `None` when the
/// ray enters none.
///
/// Read through the same memo the movement step and the arrow read
/// (`Occupants::cache`), for the old `gather::swing`'s stated reason: a
/// swing reads the same nine cells the collision path just resolved, and
/// resolving them cold was an 8× tick spike when a hundred cooldowns lined
/// up (`server/bin/profile.rs`, 2026-08-11).
///
/// Public because the client calls it — for the swing prompt through
/// [`node_cast`], and for the open prompt (a crate, a cache) with its own
/// predicate and the container verbs' reach — which is what makes those
/// prompts a mirror of the sim's geometry rather than a restatement of it.
/// `want` is a plain function pointer: nothing here may allocate a closure.
pub fn occupant_cast(
    seed: u64,
    occ: &mut Occupants,
    ray: &Ray,
    want: fn(Occupant) -> bool,
) -> Option<OccupantHit> {
    let o = (ray.o.0 / MM_PER_M, ray.o.1 / MM_PER_M, ray.o.2 / MM_PER_M);
    let u = (ray.s.0 / MM_PER_M, ray.s.1 / MM_PER_M, ray.s.2 / MM_PER_M);
    let pcx = floor_i32(o.0 / CELL_SIZE);
    let pcz = floor_i32(o.2 / CELL_SIZE);
    let mut best: Option<OccupantHit> = None;
    let mut dz = -terrain::OCCUPANT_PROBE_CELLS;
    while dz <= terrain::OCCUPANT_PROBE_CELLS {
        let mut dx = -terrain::OCCUPANT_PROBE_CELLS;
        while dx <= terrain::OCCUPANT_PROBE_CELLS {
            let (cx, cz) = (pcx + dx, pcz + dz);
            dx += 1;
            let slot = occ.cache.slot(seed, occ.table, occ.haven, cx, cz);
            if !want(slot.occupant) {
                continue;
            }
            let (r, top) = swing_volume(slot.occupant);
            if r <= 0.0 || top <= 0.0 {
                continue;
            }
            let Some((t_in, _)) = cylinder_span(
                o,
                u,
                (slot.x, slot.z),
                r * slot.scale + MELEE_PROBE_M,
                slot.y,
                slot.y + top * slot.scale,
            ) else {
                continue;
            };
            if best.is_some_and(|b| t_in >= b.t) {
                continue;
            }
            // Last, and only for a slot the ray actually enters: the life
            // store is a linear scan (`Occupants::blocks_volume`'s order,
            // for its reason). An out-of-island cell resolved to `None`
            // above, so the casts are in range.
            if cx < 0 || cz < 0 || occ.harvested.is_harvested(cx as u16, cz as u16) {
                continue;
            }
            best = Some(OccupantHit {
                cx: cx as u16,
                cz: cz as u16,
                slot,
                t: t_in,
            });
        }
        dz += 1;
    }
    best
}

/// The nearest living animal the ray enters. An animal is the cylinder its
/// content row declares (`MobDef::body_r_cm` / `body_h_cm`), standing on
/// its feet — so a level swing from eye height passes clean over a pig, and
/// you look down at what you are hunting.
pub fn mob_cast(mc: &MobContent, mobs: &Mobs, ray: &Ray) -> Option<MobHit> {
    let o = (ray.o.0 / MM_PER_M, ray.o.1 / MM_PER_M, ray.o.2 / MM_PER_M);
    let u = (ray.s.0 / MM_PER_M, ray.s.1 / MM_PER_M, ray.s.2 / MM_PER_M);
    let mut best: Option<MobHit> = None;
    for (slot, m) in mobs.m.iter().enumerate() {
        if !m.alive || m.hp == 0 {
            continue;
        }
        let def = mc.def(m.kind);
        let (r, h) = (
            f32::from(def.body_r_cm) * 0.01,
            f32::from(def.body_h_cm) * 0.01,
        );
        if r <= 0.0 || h <= 0.0 {
            continue;
        }
        let base = m.body.qy as f32 * POS_Y_Q;
        let centre = (m.body.qx as f32 * POS_XZ_Q, m.body.qz as f32 * POS_XZ_Q);
        let Some((t_in, _)) = cylinder_span(o, u, centre, r, base, base + h) else {
            continue;
        };
        if best.is_none_or(|b| t_in < b.t) {
            best = Some(MobHit { slot, t: t_in });
        }
    }
    best
}

/// What a swing reached first along its ray.
#[derive(Clone, Copy, Debug)]
pub enum Reached {
    /// Air, for the whole reach — or something in the way that this hand
    /// cannot reach, which ends the swing the same way.
    Nothing,
    /// A gather node, a bush or a barrel.
    Node(NodeHit),
    /// Another player. `stop_t` is where the world would have stopped the
    /// ray, for `part_crossed`'s clip: a wall between the chest and the head
    /// means the head was never reached.
    Body { hit: BodyHit, stop_t: f32 },
    /// An animal.
    Mob(MobHit),
    /// The world: ground, scenery, or — with `built` — a piece or a solid
    /// deployable, addressed the way `World::chip` charges it. `surf` is the
    /// `ranged::SURF_*` class for the mark.
    World {
        t: f32,
        surf: u8,
        built: Option<Struck>,
    },
}

/// Resolve one already-taken swing: what did the ray from `attacker`'s eye
/// enter first, and is that inside the hand's reach for its kind?
///
/// Four questions, each bounded, none allocating: the body solve over
/// `MAX_PLAYERS` (rewound by `favour`, exactly as a bullet is), the node
/// solve over nine cells, the animal solve over `MAX_MOBS`, and the world
/// walk over the reach at `ARROW_STEP_MM`. Then the minimum entry fraction
/// wins, ties broken node → body → animal → world, and the winner is
/// accepted only inside its own reach ([`Reaches`]).
#[allow(clippy::too_many_arguments)]
pub fn cast(
    seed: u64,
    haven: &Haven,
    cols: &crate::collide::ColIndex,
    occ: &mut Occupants,
    players: &[Player; MAX_PLAYERS],
    mobs: &Mobs,
    mc: &MobContent,
    attacker: usize,
    rewind: &Rewind,
    tick: u64,
    favour: u8,
    ray: &Ray,
    reaches: &Reaches,
) -> Reached {
    if ray.len_mm <= 0.0 {
        return Reached::Nothing;
    }
    let owner = players[attacker].id;
    let body = ranged::nearest_body(
        players,
        ray.o,
        ray.s,
        1.0,
        owner,
        ranged::Pose::Rewound {
            rewind,
            tick,
            back: favour,
        },
    );
    let node = node_cast(seed, occ, ray);
    let mob = mob_cast(mc, mobs, ray);
    // The whole reach, sampled as a shot would be. A melee reach is metres,
    // so this is a dozen taps; the cap is the sampler's own and a reach it
    // would coarsen is a content row `validate` refuses.
    let n = ((ray.len_mm / ARROW_STEP_MM as f32) as usize + 1).min(MAX_HITSCAN_SAMPLES);
    let (stop_t, surf, built) =
        ranged::world_stop(seed, haven, cols, occ, ray.o, ray.s, n, n, MELEE_PROBE_M);

    let mut best_t = f32::INFINITY;
    let mut best = Reached::Nothing;
    if let Some(nh) = node {
        best_t = nh.t;
        best = Reached::Node(nh);
    }
    if let Some(b) = body {
        // Entry, not the closest approach: the arbitration is about what the
        // arm meets first, and a chord's midpoint is up to a body's radius
        // deeper than its skin.
        if b.enter < best_t {
            best_t = b.enter;
            best = Reached::Body { hit: b, stop_t };
        }
    }
    if let Some(m) = mob {
        if m.t < best_t {
            best_t = m.t;
            best = Reached::Mob(m);
        }
    }
    if let Some(kind) = surf {
        if stop_t < best_t {
            best = Reached::World {
                t: stop_t,
                surf: kind,
                built,
            };
        }
    }

    let within = |t: f32, reach_mm: f32| t * ray.len_mm <= reach_mm;
    match best {
        Reached::Node(nh) if !within(nh.t, reaches.node) => Reached::Nothing,
        Reached::Body { hit, .. } if !within(hit.enter, reaches.body) => Reached::Nothing,
        Reached::Mob(m) if !within(m.t, reaches.body) => Reached::Nothing,
        // A wall the hand cannot damage is still a wall the swing stopped
        // on: it is marked and not charged.
        Reached::World { t, surf, built } if built.is_some() && !within(t, reaches.structure) => {
            Reached::World {
                t,
                surf,
                built: None,
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fmath::fabs;

    /// The cylinder solve, walked from outside: a ray that crosses the axis
    /// enters at the near skin and leaves at the far one.
    #[test]
    fn a_ray_through_the_axis_enters_at_the_near_skin() {
        let span = cylinder_span((0.0, 1.0, 0.0), (4.0, 0.0, 0.0), (2.0, 0.0), 0.5, 0.0, 2.0);
        let (t_in, t_out) = span.expect("the ray crosses the cylinder");
        assert!(fabs(t_in - 1.5 / 4.0) < 1e-6, "entered at t={t_in}");
        assert!(fabs(t_out - 2.5 / 4.0) < 1e-6, "left at t={t_out}");
    }

    /// Passing beside it, or over it, is a miss; starting inside it is a hit
    /// at once.
    #[test]
    fn beside_and_over_miss_and_inside_hits_at_zero() {
        assert!(
            cylinder_span((0.0, 1.0, 0.6), (4.0, 0.0, 0.0), (2.0, 0.0), 0.5, 0.0, 2.0).is_none()
        );
        assert!(
            cylinder_span((0.0, 2.5, 0.0), (4.0, 0.0, 0.0), (2.0, 0.0), 0.5, 0.0, 2.0).is_none()
        );
        let (t_in, _) =
            cylinder_span((2.0, 1.0, 0.0), (4.0, 0.0, 0.0), (2.0, 0.0), 0.5, 0.0, 2.0).unwrap();
        assert_eq!(t_in, 0.0);
    }

    /// A ray aimed down onto a short cylinder's cap enters through the cap,
    /// which is the whole reason the vertical slab is clipped in rather than
    /// tested at the planar entry.
    #[test]
    fn a_downward_ray_enters_through_the_cap() {
        // From (0, 1.6) toward (2, 0.2): passes the planar skin (x = 1.5)
        // at y ≈ 0.55, which is above a 0.4 m top, then drops through the
        // cap at y = 0.4.
        let span = cylinder_span((0.0, 1.6, 0.0), (2.0, -1.4, 0.0), (2.0, 0.0), 0.5, 0.0, 0.4);
        let (t_in, t_out) = span.expect("the ray drops onto the cap");
        let y_in = 1.6 - 1.4 * t_in;
        assert!(fabs(y_in - 0.4) < 1e-5, "entered at y={y_in}, not the cap");
        assert!(t_out >= t_in);
    }

    /// The bush is the one occupant whose swing volume is not its collision
    /// volume, and everything else is the collision volume to the bit.
    #[test]
    fn the_swing_volume_is_the_collision_volume_except_for_the_bush() {
        assert_eq!(terrain::occupant_volume(Occupant::Bush), (0.0, 0.0));
        assert_eq!(
            swing_volume(Occupant::Bush),
            (BUSH_SWING_R_M, BUSH_SWING_TOP_M)
        );
        for o in [
            Occupant::Tree,
            Occupant::StoneNode,
            Occupant::MetalNode,
            Occupant::SulfurNode,
            Occupant::Rock,
            Occupant::BarrelSlot,
            Occupant::CrateSlot,
            Occupant::CacheSlot,
        ] {
            assert_eq!(swing_volume(o), terrain::occupant_volume(o), "{o:?}");
        }
    }

    /// The ray leaves the same eye a shot does and points the way the two
    /// LUTs say: level yaw 0 is +Z, and a down pitch has a negative rise.
    #[test]
    fn the_ray_leaves_the_eye_along_the_luts() {
        let body = Body {
            qx: 100,
            qy: 50,
            qz: -200,
            ..Body::default()
        };
        let r = ray(&body, 0, 128, 2000.0);
        assert_eq!(r.o.0, 3000.0);
        assert_eq!(r.o.1, 500.0 + ARROW_EYE_MM as f32);
        assert_eq!(r.o.2, -6000.0);
        let (fx, fz) = yaw_dir(0);
        let (ch, sv) = pitch_dir(128);
        assert_eq!(r.s, (fx * ch * 2000.0, sv * 2000.0, fz * ch * 2000.0));
        let down = ray(&body, 0, 64, 2000.0);
        assert!(down.s.1 < 0.0, "a pitch under level points down");
    }
}
