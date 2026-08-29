//! Ranged v0 — the bow fires, the arrow flies, the trunk stops it.
//! **Hitscan v0 (2026-08-19) is the same module with the flight deleted**:
//! a firearm resolves its whole reach on the tick the trigger is pulled,
//! through the same two questions in the same order (`world_stop`, then
//! `nearest_body`), so a bullet and an arrow cannot disagree about what a
//! trunk is. Until it landed the revolver was a **charged dead end** — a
//! rare barrel drop with a recipe, a research rung and a craftable round,
//! all of which a player could pay for before pulling the trigger on
//! nothing, because `bake_combat` dropped every row that was not melee,
//! throwable or bow. It announces itself with `EV_SHOT` at speed zero —
//! see `hitscan` for why that pattern was free to spend.
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
//! part of a body is worth more than another) and no damage falloff (the
//! schema has no curve to read).
//!
//! **A shot chips the wall it stops on, since 2026-08-28** (ranged
//! structure damage v0) — this paragraph said it did not, for as long as
//! there has been a bow. `content/weapons.toml` has given the bow, the
//! crossbow and the revolver a `structure = 1` since the content crate was
//! written; the column was parsed, range-checked, balance-checked and
//! folded into the content hash, and `bake_ranged` dropped it one line
//! before `RangedDef` could hold it. The same "armed and unread" shape as
//! the whole bow before hitscan v0, one column down.
//!
//! It is not a second damage path. `world_stop` already knew a shot had
//! stopped on a piece — that is what `SURF_BUILT` means — and all that was
//! missing was **which** piece, so [`collide::shot_stop`] now names the
//! address the walk already had in hand and threw away. The write is
//! `deploy::damage_piece`, the same one a swing uses, through
//! `World::chip`; the sides, the removal budget and the `EV_STRUCT_HIT`
//! payload are `combat::raid`'s and are not restated here.
//!
//! **A deployable stops one now too** (2026-08-28, `NOW.md` §0mk item 2).
//! It was a different hole in a different walk — `shot_stop` reads edges,
//! diagonals and planes and no bit of `ColMasks::solid` — so an arrow
//! passed through a furnace rather than failing to damage one, and the
//! body walk had reached the same volume through a separate function
//! (`collide::deploy_blocked`) since deploy collision v0.
//! `collide::deploy_stop` is that function with a projectile's profile,
//! asked last in `world_stop`'s ladder, and `Struck` is what lets one
//! four-part address say which of the two stores it came out of. Arrows
//! still do not come back (`reference/PROJECTILES.md` §9.7).
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
//!
//! **A floor stops it too, since 2026-08-25** (shot planes v0). Until then
//! `shot_blocked` read edges and diagonals and no plane, so an arrow fired
//! down inside a base fell through every storey and landed on the dirt as
//! `SURF_GROUND` — and a roof was cover you could see through, which is the
//! half a raider notices. The body walk has read those bits since piece
//! flanks v0; the two movers now share the slab set, and where they
//! deliberately differ is written down at each shape rather than left to
//! whichever walk was written first. It costs the same walk, at the sample
//! point rather than over the sweep: a plane is crossed vertically, and the
//! vertical step between two taps is at most `ARROW_STEP_MM`, under the band
//! a slab presents.

use crate::collide::{self, ColIndex, CAPSULE_HEIGHT_M, CAPSULE_RADIUS_M};
use crate::combat::{held_item, CombatContent};
use crate::craft::{inv_count, inv_take};
use crate::gather::NO_ITEM;
use crate::input::BTN_PRIMARY;
use crate::limits::{
    ARROW_STEP_MM, MAX_ARROWS, MAX_ARROW_LIFE_TICKS, MAX_ARROW_SUBSTEPS, MAX_HITSCAN_MARK_SAMPLES,
    MAX_HITSCAN_SAMPLES, MAX_PLAYERS,
};
use crate::movement::{POS_XZ_Q, POS_Y_Q};
use crate::occupy::Occupants;
use crate::pitch_lut::pitch_dir;
use crate::spent::{SpentArrows, SpentRec};
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
///
/// ⚠ **"a floor" was a lie in this line until 2026-08-25.**
/// `collide::shot_blocked` walked edges and diagonals and consulted no plane
/// at all, so a floor, a roof and a foundation were transparent to a
/// projectile and this code could never mean one — a shot fired down inside a
/// base reported [`SURF_GROUND`] from the dirt underneath it. The doc was
/// right about the intent and wrong about the tree, which is the direction
/// that reads as covered while nothing checks it. `collide::
/// cell_planes_stop_shot` is what makes the sentence true; `tests/shoot.rs`'
/// floor block is what keeps it that way.
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
    /// The **round** it is, which is not the weapon that fired it
    /// (`reference/PROJECTILES.md` §1 fact 5: the arrow you pull out of a
    /// tree is the arrow you fired). Carried for recovery and for nothing
    /// else — until `spent.rs` there was no reason to remember which of a
    /// bow's listed rounds left the quiver, because the ballistics were
    /// already denormalized into `vx/vy/vz` and `drop`. A bow firing
    /// wooden arrows until they run out and then high-velocity ones gives
    /// back exactly what it spent, in the order it spent it.
    pub round: u16,
    pub damage: u16,
    /// What this arrow takes off a building piece if it stops on one —
    /// the bow's `structure` column, copied at the draw beside `damage`
    /// and for `drop`'s stated reason: an arrow already in the air should
    /// not change behaviour because content was rebaked under it.
    ///
    /// It is the **bow's** number and not the round's. `content/
    /// weapons.toml`'s `[[ammo]]` table carries no damage column of any
    /// kind (the file says so and says why), so every arrow out of one bow
    /// chips a wall by the same amount — the same rule already in force
    /// for flesh.
    pub structure: u16,
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
            round: 0,
            damage: 0,
            structure: 0,
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

/// Draw, and fire if everything is in hand. Returns whether the **weapon
/// took the arm** — not whether an arrow left it.
///
/// That distinction is the point of the return value. A drawn bow does not
/// swing at anything: `gather::swing` scans the 3×3 cell ring for a node
/// and would eat the shot of anyone standing next to a tree, which is
/// exactly where an archer stands. So `World::tick` asks this first and
/// skips the gather-and-melee path entirely when it answers `true`, whether
/// the shot was refused for cadence, for an empty quiver or for a full
/// store — and, since hitscan v0, whether it was a shot this function
/// resolves at all.
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
    //
    // **A firearm takes the arm here and fires somewhere else.** A hitscan
    // shot resolves against every body on the shard, and this function
    // holds exactly one of them — the shooter — because it is called from
    // inside `World::tick`'s player loop with that slot borrowed. So the
    // gun is answered by `hitscan` after the loop, on the same tick, and
    // all this does is refuse to hand the arm to `gather::swing`: without
    // it a revolver would chop the tree the shooter is standing next to,
    // which is the same defect the bow's `true` was written to prevent.
    // The cadence and the round are paid there, once, so nothing here may
    // charge for a shot it is not resolving.
    if def.hitscan {
        return true;
    }
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
        round,
        damage: def.damage,
        structure: def.structure,
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

/// Retire a landed arrow into the world: break it, or lay it down where a
/// player can take it back (`spent.rs`, `reference/PROJECTILES.md` §5).
///
/// `lodged` is the reference's own axis and it is *dealt damage* rather
/// than *what was hit*: an arrow in a body waits out the lodge, an arrow
/// in the scenery is takeable at once. §5 draws no distinction between a
/// tree, a wall and a hillside, so neither does this.
///
/// **The break roll happens here and only here**, so every path that ends
/// with an arrow on the ground pays the same odds and none of them can
/// forget to. The slot is part of the key, which is what makes two arrows
/// landing on one tick two independent draws.
///
/// ⚠ **A lodged arrow does not travel with the body it is in.** The
/// reference sticks it to the victim; ours lies at the point of impact,
/// so a hit player walking away leaves the arrow behind them. That is a
/// simplification and not an oversight — attaching it needs the arrow to
/// be a child of a moving entity, which is a second store and a second
/// set of rules about what happens when the body dies, sleeps or is
/// evicted. The lodge *timer* is what §5 says the mechanic is for, and
/// the timer is exact.
///
/// **The arrow's own position is where it lies**, so the caller advances
/// `a.q*` to the stop point before calling rather than passing the point
/// beside the arrow that is already carrying one. That is one fewer
/// argument than the obvious shape — clippy's limit is seven and the
/// obvious shape was eight — and it is also the truer statement: an arrow
/// that has stopped is at the place it stopped.
#[inline]
fn land(
    seed: u64,
    tick: u64,
    cc: &CombatContent,
    spent: &mut SpentArrows,
    slot: usize,
    a: &Arrow,
    lodged: bool,
) {
    if crate::spent::breaks(seed, tick, slot, cc.arrow_break_pct) {
        return;
    }
    spent.lodge(SpentRec {
        qx: a.qx,
        qy: a.qy,
        qz: a.qz,
        round: a.round,
        ready_at: if lodged {
            tick + u64::from(cc.arrow_lodge_ticks)
        } else {
            tick
        },
    });
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
    tick: u64,
    haven: &terrain::Haven,
    cols: &ColIndex,
    occ: &mut Occupants,
    cc: &CombatContent,
    arrows: &mut Arrows,
    spent: &mut SpentArrows,
    players: &mut [Player; MAX_PLAYERS],
    events: &mut EventQueue,
    kills: &mut [Kill; MAX_ARROWS],
    chips: &mut [Chip; MAX_ARROWS],
) -> (usize, usize) {
    let mut n_kills = 0usize;
    let mut n_chips = 0usize;
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
        let (stop_t, surf, built) =
            world_stop(seed, haven, cols, occ, (ox, oy, oz), (sx, sy, sz), n, n);

        // Pass two: the nearest body whose closest approach to the segment
        // comes at or before the world's stop. Solved rather than sampled,
        // so a body is never missed between two taps — and compared against
        // `stop_t`, so a body behind a trunk is never reached.
        let best = nearest_body(players, (ox, oy, oz), (sx, sy, sz), stop_t, a.owner);

        if let Some((t, j)) = best {
            let range_cm = ((a.flown as f32 + len_mm * t) / 10.0) as u16;
            let v = &mut players[j];
            // The funnel, reduced: an arrow is a hit like any other.
            let h = crate::combat::hurt(cc, v, a.damage);
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
            // Dealt damage, so the lodge timer applies — this is the
            // arrow you may not re-use during the fight you fired it in.
            // The point is the closest approach the solve already found,
            // which is the arrow's position at the instant it met the
            // body, not a re-solve of it.
            a.qx = crate::fmath::floor_i32(ox + sx * t);
            a.qy = crate::fmath::floor_i32(oy + sy * t);
            a.qz = crate::fmath::floor_i32(oz + sz * t);
            land(seed, tick, cc, spent, ix, &a, true);
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
            // …and the wall takes it, on `hitscan`'s ordering and for its
            // reason. `(ox, oz)` is the arrow's position at the start of
            // this tick — one tick of flight back from the wall, which is
            // the side it came from and what the hard/soft rule asks for.
            if let Some(hit) = built {
                if a.structure > 0 {
                    chips[n_chips] = Chip {
                        hit: hit.at,
                        deploy: hit.deploy,
                        structure: a.structure,
                        from_x: ox / MM_PER_M,
                        from_z: oz / MM_PER_M,
                    };
                    n_chips += 1;
                }
            }
            // Missed every body, so it is takeable at once. Same stop
            // point the impact event just reported, in millimetres rather
            // than in the body's coarser quanta: a decal is 20 cm across
            // and does not care, but the hand reaching for the arrow is
            // the thing `take_near` measures against.
            a.qx = crate::fmath::floor_i32(ox + sx * stop_t);
            a.qy = crate::fmath::floor_i32(oy + sy * stop_t);
            a.qz = crate::fmath::floor_i32(oz + sz * stop_t);
            land(seed, tick, cc, spent, ix, &a, false);
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
    (n_kills, n_chips)
}

/// What a shot walk stopped on, when what it stopped on was **built** —
/// the address and the store it lives in.
///
/// The two stores share one four-part address by design (`Deploys::
/// find_index` says so: a door and its doorway have one), so a walk that
/// reaches both has to say which. It is the same discriminator
/// `build::repair` takes as its `deploy` flag and the same one the wire
/// carries as `world::STRUCT_DEPLOY_BIT`; without it the shot path would
/// have to guess, and a guess here charges a wall for a furnace's hit or
/// silently drops the chip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Struck {
    pub at: collide::PieceHit,
    /// `true` <=> the address names a `DeployRec`, not a `PieceRec`.
    pub deploy: bool,
}

/// A chip a shot took out of a building piece — found here, applied by
/// `World`.
///
/// **Handed back rather than written, for `Kill`'s reason stated one type
/// up**: damaging a piece needs the store, the build and deploy content and
/// the tick's removal budget, and this pass holds none of them — it holds
/// the collision index and half a world. Taking `&mut Pieces` here would
/// also make the shot pass the only reader of that store that is not a
/// build verb, and would put a store-mutating parameter on the one function
/// `tests/shoot.rs` drives with a hand-built `ColIndex`.
///
/// The address travels, never a store index. `charge::detonate` states the
/// rule: the walk that finds a piece and the write that damages it are
/// separated — here by the body pass, the remaining arrows and every other
/// player's shot on the same tick — and any of those can drop a piece and
/// swap-remove another into its slot. `World::chip` re-resolves the address
/// at the moment it charges the damage, and a hit whose piece has gone is
/// simply no longer a hit: the shot still stopped and still drew its
/// impact, and only the chip is lost.
#[derive(Clone, Copy, Default)]
pub struct Chip {
    /// Which piece — `build`'s four-part address, the same one
    /// `combat::raid` picks and `deploy::damage_piece` writes against, so a
    /// shot and a swing name a wall identically.
    pub hit: collide::PieceHit,
    /// Which store that address names ([`Struck::deploy`], carried out).
    /// `World::chip` re-resolves through `Deploys::find_index` and
    /// `deploy::damage_deploy` when it is set, and pays no side price:
    /// a box has no facing, exactly as `combat::raid`'s own
    /// `Target::Deploy` arm and `charge::detonate` already have it.
    pub deploy: bool,
    /// What to take off it: the firing weapon's `structure` column. Never
    /// zero — a weapon with no structure column produces no `Chip` at all,
    /// so this array holds only hits that will be charged.
    pub structure: u16,
    /// Where the shot came from, metres, planar — what the hard/soft side
    /// rule reads. For an arrow that is its position one tick of flight
    /// back; for a bullet it is the muzzle. Both are the approach side,
    /// which is what the rule is actually about.
    pub from_x: f32,
    pub from_z: f32,
}

/// How far along `s` from `o` the **world** stops a shot, and what stopped
/// it — the ground, an occupant, or a built piece, asked in that order.
///
/// Point samples `1/n` of the segment apart; `1.0` and `None` mean nothing
/// did. Both ends are millimetres, because both callers hold millimetres and
/// the metre conversion the collision queries want happens once per sample
/// here rather than twice in two copies.
///
/// **`n` is the spacing and `upto` is how far to walk**, and they are two
/// parameters rather than one because a caller may know that nothing past
/// some fraction of the segment can change its answer. Shortening the walk
/// by shrinking `n` would move every sample; shortening it with `upto`
/// leaves the ones it does take exactly where they were, which is what makes
/// the truncation invisible to the result rather than a second sampler.
///
/// **Shared by the arrow and the bullet on purpose.** A tick of flight and
/// a whole hitscan reach are the same question over a different segment,
/// and two copies of this ladder would be two chances for a bullet and an
/// arrow to disagree about what a trunk is — the shape `CLAUDE.md`'s
/// event-payload trap warns about, one crate over.
#[allow(clippy::too_many_arguments)]
fn world_stop(
    seed: u64,
    haven: &terrain::Haven,
    cols: &ColIndex,
    occ: &mut Occupants,
    o: (f32, f32, f32),
    s: (f32, f32, f32),
    n: usize,
    upto: usize,
) -> (f32, Option<u8>, Option<Struck>) {
    let (ox, oy, oz) = o;
    let (sx, sy, sz) = s;
    let mut stop_t = 1.0f32;
    let mut surf: Option<u8> = None;
    let mut built: Option<Struck> = None;
    let mut prev = (ox / MM_PER_M, oz / MM_PER_M);
    for k in 1..=upto.min(n) {
        let t = k as f32 / n as f32;
        let px = (ox + sx * t) / MM_PER_M;
        let py = (oy + sy * t) / MM_PER_M;
        let pz = (oz + sz * t) / MM_PER_M;
        // `ground`, not `height`: a shot stops on the surface a player
        // stands on, and on an authored site that is the carved floor. A
        // raw read here puts the impact — and the decal drawn from it —
        // several metres under the pad.
        // `shot_stop` rather than `shot_blocked`: the address it names is
        // what the caller charges structure damage against. The ladder is
        // otherwise unchanged — ground, then occupants, then pieces, and
        // the first answer wins — so a shot stops exactly where it did.
        let mut hit = None;
        let what = if py <= terrain::ground(seed, haven, px, pz) {
            Some(SURF_GROUND)
        } else if occ.blocks_volume(seed, px, pz, py, ARROW_R_M, ARROW_R_M) {
            Some(SURF_WORLD)
        } else {
            // Pieces, then deployables — and this is not a tie-break, it is
            // an order two shapes can never both answer. A solid deployable
            // stands at its cell's centre and `DEPLOY_VOL`'s const block
            // proves its inflated volume never reaches the boundary; the
            // widest bench leaves 0.6 m of clear cell between its face and
            // the edge, against a 0.17 m step. So no sample can be both
            // crossing an edge and inside a box.
            //
            // The one place they do overlap is a 2*`ARROW_R_M` sliver where
            // a box's base meets the floor it stands on, and there the
            // plane answering first is the honest read: at that altitude
            // the slab is what the arrowhead is in.
            hit = collide::shot_stop(seed, haven, cols, prev.0, prev.1, px, pz, py, ARROW_R_M)
                .map(|at| Struck { at, deploy: false })
                .or_else(|| {
                    collide::deploy_stop(seed, haven, cols, px, pz, py, ARROW_R_M)
                        .map(|at| Struck { at, deploy: true })
                });
            hit.map(|_| SURF_BUILT)
        };
        if what.is_some() {
            stop_t = t;
            surf = what;
            built = hit;
            break;
        }
        prev = (px, pz);
    }
    (stop_t, surf, built)
}

/// The nearest body whose closest approach to the segment `o + s·t` comes at
/// or before `stop_t`, skipping the shooter. Solved rather than sampled, so
/// a body is never missed between two taps — and compared against the
/// world's stop, so a body behind a trunk is never reached.
fn nearest_body(
    players: &[Player; MAX_PLAYERS],
    o: (f32, f32, f32),
    s: (f32, f32, f32),
    stop_t: f32,
    owner: u32,
) -> Option<(f32, usize)> {
    let (ox, oy, oz) = o;
    let (sx, sy, sz) = s;
    let mut best: Option<(f32, usize)> = None;
    let planar2 = sx * sx + sz * sz;
    for (j, v) in players.iter().enumerate() {
        if !v.active || v.dead || v.hp == 0 || v.id == owner {
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
    best
}

/// A hitscan kill fits the arrow's kill array because there are never more
/// of them than there are players, and one shot is one body at most.
/// Asserted rather than assumed: `World::tick` hands both passes the same
/// `[Kill; MAX_ARROWS]`, and the day `MAX_PLAYERS` outgrows `MAX_ARROWS`
/// that array stops being a container and becomes an overflow.
///
/// **It bounds the chip array on the same terms** (ranged structure damage
/// v0): `World::tick` hands both passes one `[Chip; MAX_ARROWS]` too, and
/// the two passes fill it under the two rules this covers — `step` writes
/// at most one chip per arrow over `0..MAX_ARROWS`, and `hitscan` at most
/// one per player. So wall 4's cap on this write is the array's own length
/// and this line is the check, rather than a bound restated at each
/// `chips[n_chips]`.
const _: () = assert!(
    MAX_PLAYERS <= MAX_ARROWS,
    "the hitscan pass writes at most one Kill and one Chip per player into \
     the arrow store's arrays — widen MAX_ARROWS or give the pass its own"
);

/// Every firearm on the shard fires, resolves and is paid for, once, on the
/// tick its trigger is down. Returns how many entries of `kills` were
/// written.
///
/// # Why this is not `draw`
///
/// `draw` runs inside `World::tick`'s player loop with one slot borrowed,
/// which is enough for a bow — a launch touches the shooter and nobody else
/// — and is not enough for a hitscan, whose whole resolution is a question
/// about every other body. So the gun is answered here, after the loop, for
/// the two reasons the arrow's flight is: every body has already taken its
/// step, so a shot resolves against final positions rather than positions
/// that are final for the low slots and stale for the high ones; and
/// nothing about a hit may depend on the shooter's slot index.
///
/// # A bullet is an arrow with the flight deleted
///
/// The segment is the weapon's whole reach in one pass instead of one
/// tick's velocity, and the two questions asked over it are the *same two
/// functions* the arrow asks — `world_stop` and `nearest_body`, with the
/// same constants — so a bullet and an arrow cannot disagree about what a
/// trunk is. They are asked in the **opposite order**, and only because
/// this segment is 295 taps where the arrow's is sixteen; the comment at
/// the call site has the argument and why the answer is identical. There
/// is no `Arrow` record, no slot, and no `MAX_ARROWS` claim — a hitscan
/// shot is not a sim entity, so there is nothing to store and nothing to
/// refuse for lack of room. What bounds the work is
/// `MAX_HITSCAN_SAMPLES` per shot and one shot per player per tick.
///
/// A kill it reports is laid down as `DEATH_BY_ARROW` (`world.rs`), whose
/// name is the only thing about it that is wrong: a seventh cause is a
/// wire change and that constant's doc carries the refusal.
///
/// # It raises `EV_SHOT`, and the reading it uses is the cheap one
///
/// Until wire v54 it raised none, on a stated argument: the payload is a
/// muzzle speed and a drop that the client re-flies (`render/tracer.rs`),
/// a hitscan has neither, and a zero in both would hang a motionless
/// tracer at the muzzle for four seconds. The argument was sound and the
/// conclusion outlived it — the same doc named the fix (*a spoken reading
/// of `EV_SHOT`'s spare bit patterns*) and then declined to take it, so a
/// firearm announced itself only by what it *reached* and a gunfight was
/// a private event for twenty-four days.
///
/// **`speed == 0` means instantaneous** and the low half of `c` carries
/// the **reach in decimetres**. The tracer reads the zero and draws
/// nothing to fly; the mixer reads it and picks the gunshot cue over the
/// bowshot, which is the whole of how the wire tells them apart without a
/// field for the item. `world.rs`'s doc on the constant is the authority
/// and `DECISIONS.md` §open carries the proposal.
///
/// It chips a wall exactly as an arrow does (the module header): the shot
/// stops on a piece, `Chip` carries the address out and `World::chip`
/// charges the revolver's `structure` against it. No falloff
/// (`content/weapons.toml` has no curve to read) and no headshot.
#[allow(clippy::too_many_arguments)]
pub fn hitscan(
    seed: u64,
    haven: &terrain::Haven,
    cols: &ColIndex,
    occ: &mut Occupants,
    tick: u64,
    cc: &CombatContent,
    players: &mut [Player; MAX_PLAYERS],
    events: &mut EventQueue,
    kills: &mut [Kill; MAX_ARROWS],
    chips: &mut [Chip; MAX_ARROWS],
) -> (usize, usize) {
    let mut n_kills = 0usize;
    let mut n_chips = 0usize;
    for i in 0..MAX_PLAYERS {
        let p = &players[i];
        // A corpse and a sleeper do not shoot. `World::tick`'s player loop
        // has already refused them the arm; this pass runs outside that
        // loop, so it restates the rule rather than inheriting it.
        if !p.active || p.dead || p.sleeping || p.hp == 0 {
            continue;
        }
        let item = held_item(p);
        let Some(def) = cc.held_ranged(item) else {
            continue;
        };
        if !def.hitscan {
            continue;
        }
        if p.frame.buttons & BTN_PRIMARY == 0 || tick < p.next_swing {
            continue;
        }
        // The sampler backstop, `step`'s in a different clock: a reach this
        // pass cannot sample at `ARROW_STEP_MM` cannot be honest about what
        // it hits, so the weapon does not fire rather than firing untraced.
        // `bake_combat` refuses such content at boot, so this is unreachable
        // with anything shipped — which is what lets the spacing be a
        // guarantee instead of a hope. Checked before anything is spent.
        let n = def.range_mm as usize / ARROW_STEP_MM as usize + 1;
        if n > MAX_HITSCAN_SAMPLES {
            continue;
        }
        // The first round in the weapon's preference order the shooter is
        // carrying — `draw`'s rule, minus the ballistics lookup, because a
        // hitscan round has no flight to look up. Read before the cadence
        // is paid so that ordering stays visible, spent after it.
        let round = def
            .ammo
            .iter()
            .copied()
            .take_while(|&a| a != NO_ITEM)
            .find(|&a| inv_count(&p.inv, a) > 0);
        let (id, yaw, pitch) = (p.id, p.frame.yaw, p.frame.pitch);
        let (qx, qy, qz) = (p.body.qx, p.body.qy, p.body.qz);
        // The cadence is the weapon's and it is paid on the pull, not on
        // the hit — `draw`'s rule, for `draw`'s reason: a refused shot must
        // not be re-attempted every tick.
        players[i].next_swing = tick + def.rate_ticks.max(1) as u64;
        let Some(round) = round else {
            continue;
        };
        inv_take(&mut players[i].inv, round, 1);
        // **The report, and it is the same event a bow raises.** Announced
        // here for `draw`'s reason, one line later in the same order: the
        // cadence and the ammunition have both had their say, so `EV_SHOT`
        // means *a round left this barrel* rather than *someone pressed the
        // button*. Everything below only decides what it reached.
        //
        // `speed == 0` is the instantaneous reading (`world.rs`'s doc on the
        // constant): a projectile cannot leave the muzzle at rest, so the
        // pattern was unreachable, and it is now the one bit of state that
        // separates a flight from a beam. The low half then carries the
        // **reach in decimetres** instead of a drop, because a shot with no
        // flight has no gravity to describe and a beam does need a length —
        // `range_m` is at most 80 here and the field holds 6 553.
        events.push(
            EV_SHOT,
            id,
            (yaw as u32) << 8 | pitch as u32,
            def.range_mm / 100,
        );

        let (fx, fz) = yaw_dir(yaw);
        let (ch, sv) = pitch_dir(pitch);
        // Millimetres, and the same eye the bow leaves from — a gun fired
        // from the navel would clear cover the shooter cannot see over.
        let (ox, oy, oz) = (
            (qx * (POS_XZ_Q * MM_PER_M) as i32) as f32,
            (qy * (POS_Y_Q * MM_PER_M) as i32 + ARROW_EYE_MM) as f32,
            (qz * (POS_XZ_Q * MM_PER_M) as i32) as f32,
        );
        // The whole reach as one segment. The two LUTs give a unit
        // direction, so its length is `range_mm` and the sample count above
        // is the spacing `ARROW_STEP_MM` names.
        let reach = def.range_mm as f32;
        let (sx, sy, sz) = (fx * ch * reach, sv * reach, fz * ch * reach);

        // **The cheap question first, which is the opposite of the arrow's
        // order, and the reason is length.** `step` asks the world first
        // because a tick of flight is 1.3 m and at most sixteen taps, so
        // asking anything else first saves nothing. A bullet's segment is
        // its whole reach — 295 taps for the shipped revolver, each one a
        // terrain evaluation — while the body solve is a hundred compares
        // and no noise at all. So the body is found over the *whole*
        // segment, and the world is then walked only as far as that body:
        // past it, nothing the world could stop changes who was hit.
        //
        // It is a truncation and not a different rule. `nearest_body` picks
        // the **minimum** `t`, so if that one falls past where the world
        // stopped then no body is inside the stop, and the single compare
        // below is exactly what passing the real `stop_t` in would have
        // computed. With no body in the line the walk is the full one,
        // because then the only thing left to find is where the shot marked
        // the world — and that last walk is the one bounded by
        // `MAX_HITSCAN_MARK_SAMPLES`, because a decal must not own the
        // tick. Measured, at 100 shooters all firing on one tick: 20 ms
        // when every trace runs its full 50 m, 2.1 ms when a body
        // truncates it (`DECISIONS.md` §open, hitscan v0).
        let seen = nearest_body(players, (ox, oy, oz), (sx, sy, sz), 1.0, id);
        let upto = match seen {
            Some((t, _)) => (t * n as f32) as usize + 1,
            None => MAX_HITSCAN_MARK_SAMPLES,
        };
        let (stop_t, surf, built) =
            world_stop(seed, haven, cols, occ, (ox, oy, oz), (sx, sy, sz), n, upto);
        let best = seen.filter(|&(t, _)| t <= stop_t);

        if let Some((t, j)) = best {
            let range_cm = (reach * t / 10.0) as u16;
            let v = &mut players[j];
            // The funnel, reduced: a bullet is a hit like any other, and
            // armor blunts it (armor v0, 2026-08-19 — this said "the day
            // armor lands" for exactly one day).
            let h = crate::combat::hurt(cc, v, def.damage);
            let died = h.died;
            let (vid, left, vmax) = (v.id, h.left as u32, v.hp_max as u32);
            events.push(EV_HIT, id, vid, def.damage as u32);
            events.push(EV_HEALTH, vid, left, vmax);
            if died {
                events.push(EV_DEATH, vid, id, 0);
                kills[n_kills] = Kill {
                    victim: j,
                    by: id,
                    item,
                    range_cm,
                };
                n_kills += 1;
            }
            continue;
        }

        if let Some(kind) = surf {
            // Where it landed, in the body's own quanta — `step`'s
            // arithmetic and `step`'s reason for the units. Reached only
            // when no body was hit, so a decal is never drawn for a shot
            // that found flesh.
            let qx = crate::fmath::floor_i32((ox + sx * stop_t) / (POS_XZ_Q * MM_PER_M));
            let qy = crate::fmath::floor_i32((oy + sy * stop_t) / (POS_Y_Q * MM_PER_M));
            let qz = crate::fmath::floor_i32((oz + sz * stop_t) / (POS_XZ_Q * MM_PER_M));
            events.push(
                EV_IMPACT,
                (kind as u32) << 24 | qx as u32,
                qz as u32,
                qy as u32,
            );
            // …and the wall takes it. After the impact, so the order on the
            // wire is *where it hit* then *what that cost* — and so a piece
            // that falls to this shot has its `EV_PIECE_REMOVED` behind the
            // mark that explains it.
            if let Some(hit) = built {
                if def.structure > 0 {
                    chips[n_chips] = Chip {
                        hit: hit.at,
                        deploy: hit.deploy,
                        structure: def.structure,
                        from_x: ox / MM_PER_M,
                        from_z: oz / MM_PER_M,
                    };
                    n_chips += 1;
                }
            }
        }
    }
    (n_kills, n_chips)
}
