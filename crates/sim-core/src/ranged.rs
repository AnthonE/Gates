//! Ranged v0 — the bow fires, the arrow flies, the trunk stops it.
//!
//! Until this module `combat.rs` exposed `strike` and `raid` and nothing
//! else, so every fight on the island was a walk-up club fight while
//! `content/weapons.toml` had carried a bow, a crossbow, their ammo and
//! their ballistics since the content crate was written — validated,
//! balance-checked, hashed into the content hash, and thrown away at
//! `bake_combat`'s `if w.kind != WeaponKind::Melee { continue; }`. The data
//! was armed and the sim could not read it.
//!
//! # Everything here is an integer
//!
//! An arrow's position is millimetres and its velocity is **millimetres per
//! tick**, both `i32`, and the whole integration is integer addition. The
//! conversion out of `content`'s metres-per-second happens once, at bake,
//! and never again — which is the quantize-both-sides law (CLAUDE.md's trap
//! list) applied to a projectile. Two things fall out of it: there is no
//! per-tick rounding to accumulate over a second of flight, and native and
//! wasm cannot disagree about a path, because there is no float in it. The
//! only floats are at the two ends — the launch direction, read from the two
//! angle LUTs, and the collision queries, which take metres because
//! `terrain` and `collide` do.
//!
//! # What stops an arrow, and in what order
//!
//! Ground, occupants and building pieces are **point samples** along the
//! tick's segment, `ARROW_STEP_MM` apart; bodies are an exact planar
//! closest-approach against the same segment. That split is deliberate. The
//! world's stop is a *distance along the segment*, and a body is only hit if
//! its closest approach comes **before** that distance — which is the whole
//! of the judge's ask, that a shot stopped by a trunk does not reach the
//! body behind it. Sampling the world and solving for the body is what makes
//! the comparison exact rather than a race between two samplers.
//!
//! # What v0 does not do
//!
//! No headshots (melee has none either — `frame.pitch` aims the shot but no
//! part of a body is worth more than another), no damage falloff (the schema
//! has no curve to read), and no structure damage
//! — an arrow that reaches a wall stops dead rather than chipping it.
//!
//! **An arrow flies at its own size now** (catalogue v1). Pieces stop it
//! through `collide::shot_blocked` — a point at the arrow's altitude,
//! inflated by `ARROW_R_M` — where it used to ride `collide::blocked` and
//! be a 1.7 m capsule that could not thread a gap a body could not walk.
//! That was stated here as a debt ("the honest fix is a radius parameter
//! on `collide`"), and the window is what called it in: a shape whose
//! whole contract is *blocks a body and not a shot* is unbuildable while
//! every shot is shaped like a body. Trees and bodies still stop arrows
//! exactly as before; only the piece query changed profile.

use crate::collide::{self, ColIndex, CAPSULE_HEIGHT_M, CAPSULE_RADIUS_M};
use crate::combat::{held_item, CombatContent};
use crate::craft::{inv_count, inv_take};
use crate::gather::NO_ITEM;
use crate::input::BTN_PRIMARY;
use crate::limits::{
    ARROW_STEP_MM, MAX_ARROWS, MAX_ARROW_LIFE_TICKS, MAX_ARROW_SUBSTEPS, MAX_PLAYERS,
};
use crate::movement::{POS_XZ_Q, POS_Y_Q};
use crate::occupy::Occupants;
use crate::pitch_lut::pitch_dir;
use crate::terrain;
use crate::world::{EventQueue, Player, EV_DEATH, EV_HEALTH, EV_HIT, EV_IMPACT, EV_SHOT};
use crate::yaw_lut::yaw_dir;

/// Height above the feet an arrow leaves from, millimetres. The eye, not
/// the chest: a shot that started at the navel would clear cover the
/// shooter cannot see over. `CAPSULE_HEIGHT_M` is 1.7 m and this is 10 cm
/// under it, the same relationship the client's camera has.
/// Proposed default, DECISIONS.md §open (ranged v0).
pub const ARROW_EYE_MM: i32 = 1600;

/// The arrowhead's collision extent, metres — used as both radius and
/// height, because an arrow is a point that needs just enough extent to
/// survive `terrain::slot_blocks`'s half-open vertical interval (a zero
/// height is rejected at a slot's exact base). Small enough that what stops
/// an arrow is the sample spacing, never the head.
/// Proposed default, DECISIONS.md §open (ranged v0).
pub const ARROW_R_M: f32 = 0.05;

/// Millimetres in a metre — the one conversion this module makes, named so
/// the collision calls read as unit changes rather than magic scaling.
const MM_PER_M: f32 = 1000.0;

/// What an arrow stopped on (`EV_IMPACT`'s surface field) — the ground, a
/// thing worldgen put there, or a thing a player built.
///
/// **Three, because three is what the stop test can answer.** `step`'s
/// pass one asks exactly three questions in order — terrain height, then
/// occupants, then pieces — so these are not a taxonomy anyone chose but a
/// readout of the ladder that already existed. A finer one (pine vs
/// boulder vs barrel) is a fourth question nothing currently asks, and
/// `occupy` would have to answer it on the hot path to say so.
///
/// They cross the wire because the alternative is the client re-deciding
/// what it hit, which is `collide` and `terrain` re-implemented on the
/// draw side and drifting from the sim's copy the first time either moves
/// — the failure `column_floor_y` was written to end. A position plus this
/// byte is a *statement*; a position alone is a riddle.
pub const SURF_GROUND: u8 = 0;
/// An occupant: a trunk, a boulder, a barrel — anything worldgen stood in
/// a slot (`occupy::Occupants::blocks_volume`).
pub const SURF_WORLD: u8 = 1;
/// A built piece: a wall, a floor, a door (`collide::shot_blocked`).
pub const SURF_BUILT: u8 = 2;

/// One arrow in flight. `life == 0` ⇔ the slot is free, which is also what
/// keeps it out of `state_hash` (see `World::state_hash`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Arrow {
    /// Position, millimetres. Not the body quanta: those are 3 cm in x/z
    /// and 1 cm in y, and an arrow crossing 1.3 m a tick wants one unit in
    /// both axes and a finer one than the body's.
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
    /// Velocity, millimetres **per tick** — already divided by the rate at
    /// bake time, so a step is `q += v` and nothing else.
    pub vx: i32,
    pub vy: i32,
    pub vz: i32,
    /// Gravity carried on the arrow rather than looked up per tick: the
    /// flight step has no reason to hold the content table, and an arrow
    /// already in the air should not change behaviour if content is
    /// rebaked under it.
    pub drop: u16,
    /// Network id of the shooter — what `EV_HIT` reports and what the
    /// self-hit skip compares. A slot index would go stale across a
    /// respawn; the id does not.
    pub owner: u32,
    /// The weapon that fired it, for the death screen's "with <weapon>".
    pub item: u16,
    pub damage: u16,
    /// Ticks of flight left. Zero means the slot is free.
    pub life: u16,
    /// Millimetres flown so far — arc length, summed per tick, which is
    /// what the death screen reports as the range. It differs from the
    /// straight line by the sag of the drop, and over an arrow's life that
    /// is centimetres.
    pub flown: u32,
}

impl Arrow {
    #[inline]
    fn active(&self) -> bool {
        self.life > 0
    }
}

/// A death an arrow caused, handed back to `World` because laying the body
/// down (`World::die`) needs the whole world and this step has half of it
/// borrowed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Kill {
    pub victim: usize,
    pub by: u32,
    pub item: u16,
    pub range_cm: u16,
}

/// Every arrow in the air on the shard. A flat array with a free-slot scan
/// rather than a free list: `MAX_ARROWS` is 128, the scan stops at the
/// first hole, and a free list is one more thing `state_hash` would have to
/// answer for.
#[derive(Clone, Copy, Debug)]
pub struct Arrows {
    a: [Arrow; MAX_ARROWS],
}

impl Arrows {
    pub const EMPTY: Self = Self {
        a: [Arrow {
            qx: 0,
            qy: 0,
            qz: 0,
            vx: 0,
            vy: 0,
            vz: 0,
            drop: 0,
            owner: 0,
            item: 0,
            damage: 0,
            life: 0,
            flown: 0,
        }; MAX_ARROWS],
    };

    pub fn new() -> Self {
        Self::EMPTY
    }

    /// The arrows actually in the air, in slot order. The only iteration
    /// anything outside this module does, and the order is an array index,
    /// so it is the same on every box (CLAUDE.md wall 1).
    pub fn entries(&self) -> impl Iterator<Item = &Arrow> {
        self.a.iter().filter(|a| a.active())
    }

    pub fn len(&self) -> usize {
        self.a.iter().filter(|a| a.active()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The first free slot, or `None` when the store is full — the refusal
    /// `MAX_ARROWS` documents. Checked before the ammo is spent.
    fn free(&mut self) -> Option<usize> {
        self.a.iter().position(|a| !a.active())
    }
}

impl Default for Arrows {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// Draw, and fire if everything is in hand. Returns whether the **bow took
/// the arm** — not whether an arrow left it.
///
/// That distinction is the point of the return value. A drawn bow does not
/// swing at anything: `gather::swing` scans the 3×3 cell ring for a node
/// and would eat the shot of anyone standing next to a tree, which is
/// exactly where an archer stands. So `World::tick` asks this first and
/// skips the gather-and-melee path entirely when it answers `true`, whether
/// the shot was refused for cadence, for an empty quiver or for a full
/// store.
pub fn draw(
    tick: u64,
    cc: &CombatContent,
    arrows: &mut Arrows,
    events: &mut EventQueue,
    p: &mut Player,
) -> bool {
    let Some(def) = cc.held_ranged(held_item(p)) else {
        return false;
    };
    // From here the answer is `true` on every path: the hand holds a bow,
    // so nothing below hands the arm back to the club.
    if p.frame.buttons & BTN_PRIMARY == 0 || tick < p.next_swing {
        return true;
    }
    // The cadence is the weapon's, and it is paid on the draw rather than
    // on the hit — the same rule `gather::swing` uses, for the same reason:
    // a refused shot must not be re-attempted every tick.
    p.next_swing = tick + def.rate_ticks.max(1) as u64;

    // The first round in the weapon's preference order the shooter is
    // actually carrying — and it must have ballistics to fly by, which
    // `validate` guarantees for every listed round, so the `find_map` is a
    // re-check rather than the first check.
    //
    // Walking the list here rather than baking one round is the whole of
    // §9.3's payoff at the sim end: a bow with wooden and high-velocity
    // arrows listed fires wood until the wood runs out and then keeps
    // firing, at the other arrow's speed and drop. There is no verb to
    // choose, so the list order is the choice.
    let Some((round, ball)) = def
        .ammo
        .iter()
        .copied()
        .take_while(|&a| a != NO_ITEM)
        .filter(|&a| inv_count(&p.inv, a) > 0)
        .find_map(|a| cc.ammo_def(a).map(|b| (a, b)))
    else {
        return true;
    };
    // Space before ammo, always. A full store must cost the shooter
    // nothing — spending the arrow first and finding no slot for it is how
    // a cap turns into a bug report about vanishing ammunition.
    let Some(ix) = arrows.free() else {
        return true;
    };
    inv_take(&mut p.inv, round, 1);

    let (fx, fz) = yaw_dir(p.frame.yaw);
    let (ch, sv) = pitch_dir(p.frame.pitch);
    let speed = ball.speed_mmpt as f32;
    // Flight time is the weapon's reach over this round's speed, so a fast
    // arrow and a slow one out of the same bow both expire at the range
    // the bow claims instead of at a baked tick count that only one of
    // them earned. Integer division, once per shot. `speed_mmpt > 0` is
    // `ammo_def`'s filter, so this cannot divide by zero.
    let life = (def.range_mm / ball.speed_mmpt as u32).clamp(1, MAX_ARROW_LIFE_TICKS as u32) as u16;
    arrows.a[ix] = Arrow {
        qx: p.body.qx * (POS_XZ_Q * MM_PER_M) as i32,
        qy: p.body.qy * (POS_Y_Q * MM_PER_M) as i32 + ARROW_EYE_MM,
        qz: p.body.qz * (POS_XZ_Q * MM_PER_M) as i32,
        // The one place a float becomes an integer. After this the path is
        // exact, so this rounding happens once per shot and never per tick.
        vx: crate::fmath::floor_i32(fx * ch * speed),
        vy: crate::fmath::floor_i32(sv * speed),
        vz: crate::fmath::floor_i32(fz * ch * speed),
        drop: ball.drop_mmpt2,
        owner: p.id,
        item: held_item(p),
        damage: def.damage,
        life,
        flown: 0,
    };
    // Announced only where an arrow actually left the bow — after the
    // cadence, the quiver and the store have all had their say. Every one
    // of those paths returns above, so a client can treat `EV_SHOT` as
    // "an arrow exists" rather than "someone pressed the button", which
    // is the difference between a tracer and a phantom.
    events.push(
        EV_SHOT,
        p.id,
        (p.frame.yaw as u32) << 8 | p.frame.pitch as u32,
        (ball.speed_mmpt as u32) << 16 | ball.drop_mmpt2 as u32,
    );
    true
}

/// Fly every arrow one tick and resolve what it reached. Returns how many
/// entries of `kills` were written.
///
/// Ordering inside a tick is slot order over the arrow store, which is
/// allocation order, which is deterministic — two arrows arriving at the
/// same body in the same tick both land, and the second finds the hp the
/// first left.
#[allow(clippy::too_many_arguments)]
pub fn step(
    seed: u64,
    haven: &terrain::Haven,
    cols: &ColIndex,
    occ: &mut Occupants,
    arrows: &mut Arrows,
    players: &mut [Player; MAX_PLAYERS],
    events: &mut EventQueue,
    kills: &mut [Kill; MAX_ARROWS],
) -> usize {
    let mut n_kills = 0usize;
    for ix in 0..MAX_ARROWS {
        let mut a = arrows.a[ix];
        if !a.active() {
            continue;
        }
        // Gravity before the step: the velocity an arrow flies this tick is
        // the one it ends the tick with, which is the convention
        // `movement::step` already uses for a falling body.
        a.vy -= a.drop as i32;

        let (dx, dy, dz) = (a.vx, a.vy, a.vz);
        let len_mm = {
            let (fx, fy, fz) = (dx as f32, dy as f32, dz as f32);
            (fx * fx + fy * fy + fz * fz).sqrt()
        };

        // How many samples this tick's segment needs, and the refusal when
        // it needs more than it may have. That case is unreachable with
        // shipped content — `bake_combat` refuses a muzzle speed past the
        // sampler, and a derived life expires an arrow long before gravity
        // could carry it there — so this is the backstop that lets the
        // sample spacing be a guarantee rather than a hope. An arrow moving
        // faster than the sim can honestly trace stops existing; it does
        // not fly untraced.
        let need = (len_mm / ARROW_STEP_MM as f32) as usize + 1;
        if need > MAX_ARROW_SUBSTEPS {
            arrows.a[ix].life = 0;
            continue;
        }
        let n = need.max(1);

        let (ox, oy, oz) = (a.qx as f32, a.qy as f32, a.qz as f32);
        let (sx, sy, sz) = (dx as f32, dy as f32, dz as f32);

        // Pass one: how far along the segment the world stops it, if it
        // does. Pure point sampling — ground, then occupants, then pieces.
        //
        // The three predicates used to be one `||` chain and are now an
        // `if`/`else if` ladder in the same order, which short-circuits
        // identically — the arrow stops on exactly the sample it always
        // stopped on. What the ladder adds is **which** of the three
        // answered, because that is the difference between a puff of dirt,
        // a chip of bark and a splintered plank, and it is knowable here
        // and nowhere else (`SURF_*`).
        let mut stop_t = 1.0f32;
        let mut surf: Option<u8> = None;
        let mut prev = (ox / MM_PER_M, oz / MM_PER_M);
        for k in 1..=n {
            let t = k as f32 / n as f32;
            let px = (ox + sx * t) / MM_PER_M;
            let py = (oy + sy * t) / MM_PER_M;
            let pz = (oz + sz * t) / MM_PER_M;
            // `ground`, not `height`: an arrow stops on the surface a player
            // stands on, and on an authored site that is the carved floor. A
            // raw read here puts the impact — and now the decal main added
            // with it — several metres under the pad.
            let what = if py <= terrain::ground(seed, haven, px, pz) {
                Some(SURF_GROUND)
            } else if occ.blocks_volume(seed, px, pz, py, ARROW_R_M, ARROW_R_M) {
                Some(SURF_WORLD)
            } else if collide::shot_blocked(
                seed, haven, cols, prev.0, prev.1, px, pz, py, ARROW_R_M,
            ) {
                Some(SURF_BUILT)
            } else {
                None
            };
            if what.is_some() {
                stop_t = t;
                surf = what;
                break;
            }
            prev = (px, pz);
        }

        // Pass two: the nearest body whose closest approach to the segment
        // comes at or before the world's stop. Solved rather than sampled,
        // so a body is never missed between two taps — and compared against
        // `stop_t`, so a body behind a trunk is never reached.
        let mut best: Option<(f32, usize)> = None;
        let planar2 = sx * sx + sz * sz;
        for (j, v) in players.iter().enumerate() {
            if !v.active || v.dead || v.hp == 0 || v.id == a.owner {
                continue;
            }
            let (bx, by, bz) = (
                v.body.qx as f32 * POS_XZ_Q,
                v.body.qy as f32 * POS_Y_Q,
                v.body.qz as f32 * POS_XZ_Q,
            );
            let (ax, ay, az) = (ox / MM_PER_M, oy / MM_PER_M, oz / MM_PER_M);
            let (ux, uy, uz) = (sx / MM_PER_M, sy / MM_PER_M, sz / MM_PER_M);
            // Closest approach in the plane, clamped to the segment. A
            // degenerate (purely vertical) shot pins to its start.
            let t = if planar2 <= 0.0 {
                0.0
            } else {
                (((bx - ax) * ux + (bz - az) * uz) / (ux * ux + uz * uz)).clamp(0.0, 1.0)
            };
            if t > stop_t {
                continue;
            }
            let (cx, cy, cz) = (ax + ux * t, ay + uy * t, az + uz * t);
            let (ddx, ddz) = (cx - bx, cz - bz);
            // A cylinder, exactly like `terrain::slot_blocks` — the house
            // shape. No headshot, so the whole body is one target.
            if ddx * ddx + ddz * ddz > CAPSULE_RADIUS_M * CAPSULE_RADIUS_M {
                continue;
            }
            if cy < by || cy > by + CAPSULE_HEIGHT_M {
                continue;
            }
            if best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, j));
            }
        }

        if let Some((t, j)) = best {
            let range_cm = ((a.flown as f32 + len_mm * t) / 10.0) as u16;
            let v = &mut players[j];
            // The funnel, reduced: an arrow is a hit like any other.
            let h = crate::combat::hurt(v, a.damage);
            let died = h.died;
            let (vid, left, vmax) = (v.id, h.left as u32, v.hp_max as u32);
            events.push(EV_HIT, a.owner, vid, a.damage as u32);
            events.push(EV_HEALTH, vid, left, vmax);
            if died {
                events.push(EV_DEATH, vid, a.owner, 0);
                kills[n_kills] = Kill {
                    victim: j,
                    by: a.owner,
                    item: a.item,
                    range_cm,
                };
                n_kills += 1;
            }
            arrows.a[ix].life = 0;
            continue;
        }

        if let Some(kind) = surf {
            // Where it actually landed, in the body's own quanta.
            //
            // **Reached only here, which is the whole reason this is
            // trustworthy.** The body branch above `continue`s, so an arrow
            // that found flesh never arrives — a hit is `EV_HIT` and draws a
            // hitmarker, not a chip out of the scenery. This is the other
            // outcome: the arrow met the world and stopped.
            //
            // `stop_t` is the fraction the sample loop broke on, so this is
            // the sim's own stop point rather than a re-solve of it. The
            // quanta are `Body`'s (3 cm x/z, 1 cm y) and not the arrow's
            // millimetres, because the wire already has windows and a range
            // check for those and a decal is 20 cm across — a unit no eye
            // can find is a unit not worth three bits an axis.
            let qx = crate::fmath::floor_i32((ox + sx * stop_t) / (POS_XZ_Q * MM_PER_M));
            let qy = crate::fmath::floor_i32((oy + sy * stop_t) / (POS_Y_Q * MM_PER_M));
            let qz = crate::fmath::floor_i32((oz + sz * stop_t) / (POS_XZ_Q * MM_PER_M));
            events.push(
                EV_IMPACT,
                (kind as u32) << 24 | qx as u32,
                qz as u32,
                // Signed, and the only field in the lane that is: an arrow
                // can stop below sea level and `qy` is negative there. It
                // crosses as the two's-complement bit pattern and the
                // encoder reads it back with `as i32` before biasing it into
                // the wire's window — a reinterpretation, never a cast that
                // loses anything. `a`'s `qx` needs no such note: the island
                // starts at zero.
                qy as u32,
            );
            arrows.a[ix].life = 0;
            continue;
        }

        a.qx += dx;
        a.qy += dy;
        a.qz += dz;
        a.flown = a.flown.saturating_add(len_mm as u32);
        a.life -= 1;
        arrows.a[ix] = a;
    }
    n_kills
}
