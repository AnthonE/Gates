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
//! No damage falloff — the schema has no curve to read.
//!
//! **A bullet is lag-compensated and an arrow is not, since 2026-08-30**, and
//! the asymmetry is the design rather than a gap: `hitscan` resolves on the
//! tick its trigger came down, so its bodies are put back where the shooter's
//! screen had them (`Pose::Rewound`), while `step` is one tick of a flight
//! launched earlier and stays present-tick (`Pose::Live`). `Pose`'s own doc
//! carries the argument and `DECISIONS.md` §open carries the launch half.
//!
//! **"No headshots" stood here until 2026-08-30** and is the third clause
//! of this header to outlive its truth, after the bow and after the wall
//! chip. A head is a band off the top of the body cylinder
//! (`collide::HEAD_BAND_M`) and a hit whose line crosses it pays the
//! weapon's `headshot_mult`, which had been priced, banded and
//! content-hashed since the content crate and dropped at the bake every
//! time (`reference/PROJECTILES.md` §9.4). The rule is §7's — the most
//! significant part **along the segment**, so `nearest_body` carries the
//! span the shot spent inside the body and `head_crossed` is the overlap.
//!
//! **Melee still has none**, and that half of the old sentence stays true
//! for the reason it always gave: `combat::strike` resolves feet-to-feet in
//! a plane, so there is no height for a band to test. `MeleeDef` has no
//! such field.
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

use crate::collide::{self, ColIndex, CAPSULE_HEIGHT_M, CAPSULE_RADIUS_M, HEAD_BAND_M};
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
use crate::rewind::{Rewind, RewindPose};
use crate::spent::{SpentArrows, SpentRec};
use crate::terrain;
use crate::world::{EventQueue, Player, EV_DEATH, EV_HEALTH, EV_HIT, EV_HURT, EV_IMPACT, EV_SHOT};
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
    /// What this arrow is multiplied by if its line crosses the head band
    /// — the bow's `headshot_mult`, copied at the draw beside `damage` and
    /// `structure`, for the same reason those two are copied: an arrow
    /// already in the air should not change what it does because content
    /// was rebaked under it.
    ///
    /// The **bow's** number, like `damage` and `structure`, and not the
    /// round's. `weapons.toml`'s `[[ammo]]` rows carry ballistics and
    /// nothing else; a high-velocity arrow flies flatter and hits for what
    /// the bow says.
    pub head_mult: u16,
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
            // The identity, so an unfilled slot cannot delete a hit.
            head_mult: 1,
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
        head_mult: def.headshot_mult,
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
        //
        // **`Pose::Live`, and it is a refusal rather than an omission**
        // (lag comp, the gun's slice). This shaft was launched on an
        // earlier tick and has been flying since; the client was paid for
        // its aim at the draw, and rewinding 1.3 m of a four-second journey
        // against a quarter-second-old world would hit people the arrow
        // visibly flew past. `Pose`'s own doc has the whole argument and
        // `DECISIONS.md` §open carries the launch half, which is the part
        // this does NOT decide.
        let best = nearest_body(
            players,
            (ox, oy, oz),
            (sx, sy, sz),
            stop_t,
            a.owner,
            Pose::Live,
        );

        if let Some(BodyHit {
            t,
            slot: j,
            enter,
            exit,
            qy: feet_q,
        }) = best
        {
            let range_cm = ((a.flown as f32 + len_mm * t) / 10.0) as u16;
            let v = &mut players[j];
            // Did the shaft cross the head on its way through? The span is
            // clipped against the world's stop first, so an arrow that
            // buries itself in a windowsill at chest height is not credited
            // with the skull that was on the far side of it.
            // The feet the SCAN resolved, which for an arrow is the live
            // body and is spelled that way anyway: one rule for both
            // shots — the band is always measured off the cylinder the hit
            // was decided against (`BodyHit::qy`).
            let feet_mm = feet_q as f32 * (POS_Y_Q * MM_PER_M);
            let dmg = if head_crossed(oy, sy, feet_mm, enter, exit.min(stop_t)) {
                crate::combat::headshot(a.damage, a.head_mult)
            } else {
                a.damage
            };
            // The funnel, reduced: an arrow is a hit like any other.
            let h = crate::combat::hurt(cc, v, dmg);
            let died = h.died;
            let (vid, left, vmax) = (v.id, h.left as u32, v.hp_max as u32);
            // The scaled number on both events, not the bow's column.
            // `EV_HIT` is the attacker's hitmarker and `EV_HURT` the
            // victim's arc, and each is answering "how hard was that",
            // which is the blow that arrived and not the one on the row.
            events.push(EV_HIT, a.owner, vid, dmg as u32);
            // Where it came FROM, which for something still in flight is the
            // reverse of where it was going — truer than the archer's current
            // position, because the archer has had the whole flight to move.
            events.push(
                EV_HURT,
                vid,
                crate::combat::bearing_sector(-(dx as i64), -(dz as i64)) as u32,
                dmg as u32,
            );
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

/// How a body scan answers *where was this body*.
///
/// The two shots in this module need different answers and the difference
/// is not a depth one caller happens to pass zero for — it is a rule about
/// each shot's own chronology — so it is a **type**. `Pose::Live` cannot be
/// given a depth, which is what stops the arrow acquiring one by accident
/// the next time this signature is edited.
///
/// [`Pose::Live`] is the arrow's. `findings/lagcomp-design-20260818.md`
/// §5.1 is the reasoning and it survives intact: an arrow in the store was
/// launched on an *earlier* tick and has been travelling ever since, so
/// rewinding this tick of its flight would resolve 1.3 m of a four-second
/// journey against a quarter-second-old world — hitting people the shaft
/// visibly flew past, and doing it to a shooter who has already been paid
/// for their aim once. The **launch** direction is the open question there
/// and it is refused deliberately now rather than by omission
/// (`DECISIONS.md` §open, lag comp — the arrow's launch aim).
///
/// [`Pose::Rewound`] is the bullet's, and it is the case lag compensation
/// exists for: a hitscan resolves on the tick its trigger came down, which
/// is the tick the client aimed on, against bodies that client was drawing
/// `INTERP_DELAY_TICKS` in the past.
#[derive(Clone, Copy)]
enum Pose<'a> {
    /// Present tick. Bit-identical to the scan before this type existed.
    Live,
    /// `back` ticks before `tick`, out of the ring, per slot.
    Rewound {
        rewind: &'a Rewind,
        tick: u64,
        back: u8,
    },
}

impl Pose<'_> {
    /// Where slot `slot` was, for this resolver's definition of *was*.
    ///
    /// The live pose is built here and handed to `pose_at` as the fallback,
    /// so every honest-answer refusal in the ring (cold row, wrong stamp,
    /// empty slot, a stranger's id) lands on the body the scan would have
    /// read anyway. `back == 0` short-circuits inside `pose_at`, so
    /// `Rewound { back: 0 }` and `Live` are the same bytes as well as the
    /// same idea.
    #[inline]
    fn of(self, slot: usize, p: &Player) -> RewindPose {
        let live = RewindPose::live(p.id, &p.body);
        match self {
            Self::Live => live,
            Self::Rewound { rewind, tick, back } => rewind.pose_at(tick, slot, back, live),
        }
    }
}

/// One body the shot reached, and everything the two resolvers need to know
/// about how it reached it.
///
/// **A named struct rather than the `(f32, usize)` tuple this used to
/// return**, and the reason is in `CLAUDE.md`'s trap list twice over: a
/// positional payload is where the reference ecosystem actually bled, and a
/// gate that re-derives a tuple's layout is checking the layout against
/// itself. Four values in a tuple would be four chances to swap two of them
/// at a call site, in a file where `t`, `enter` and `exit` are all segment
/// fractions of the same segment and all interchangeable to the compiler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyHit {
    /// Segment fraction of the closest approach, clamped to `[0, 1]`. This
    /// is what decides *which* body was hit and where the arrow lodges.
    pub t: f32,
    /// Slot in `players`, not the network id — the caller already holds the
    /// array and wants `&mut players[slot]`.
    pub slot: usize,
    /// Segment fraction where the line **enters** that body's radius, and
    /// where it **leaves** it, clamped to `[0, 1]`. Together they are the
    /// span the shot spent inside the body, which is what [`head_crossed`]
    /// needs and what `t` alone cannot give: `t` is one point, and §7's
    /// rule is about the whole crossing.
    ///
    /// The caller clips `exit` against the world's stop before using it. It
    /// is not clipped here because `hitscan` asks this question **before**
    /// it knows where the world stopped — that is the whole of its
    /// truncation optimization — so a stop clipped in would be the wrong
    /// one on one of the two paths.
    pub enter: f32,
    pub exit: f32,
    /// The victim's **feet**, in `Body`'s own y quanta, as the scan
    /// resolved them — not as they are now.
    ///
    /// Carried rather than re-read at the call site, and that is the whole
    /// of what makes a rewound headshot coherent. The head band is an
    /// offset off the top of the same cylinder this scan solved against
    /// ([`head_crossed`]), so reading `players[slot].body.qy` afterwards
    /// would test a **present-tick crown** against a **past-tick
    /// horizontal solve** — a victim who jumped, fell or walked downhill in
    /// the last quarter-second gets a head floating away from the body the
    /// bullet was decided against.
    ///
    /// At [`Pose::Live`] this is exactly `players[slot].body.qy`, so the
    /// arrow's arithmetic is unchanged to the bit.
    pub qy: i32,
}

/// Did the shot cross the victim's head band while it was inside their body?
///
/// **The two-part reduction of `reference/PROJECTILES.md` §7's rule**, which
/// is *damage the most significant body part along the line of sight*, not
/// the first one intersected. §9.4 states the reduction: with a head and a
/// body and nothing else, "most significant part crossed" is "was the head
/// interval crossed at all". So this is an interval overlap and not a
/// raycast — a shot that clips a shoulder on its way into a skull is a
/// headshot, which is the inversion §7 says they had to make on purpose (a
/// limb in front of a torso must not save the torso).
///
/// Everything is **millimetres**, matching the two call sites' own units, so
/// nothing here converts and nothing rounds: `oy + sy * t` is the shot's
/// altitude at fraction `t`, and `feet_mm` is the victim's feet. `t_lo` and
/// `t_hi` are [`BodyHit::enter`] and [`BodyHit::exit`], the second already
/// clipped against the world's stop by the caller — a wall between the
/// chest and the head means the head was never reached.
///
/// `y` is linear in `t`, so the altitudes at the two ends **are** the range
/// over the span and there is nothing to sample. That is the whole reason
/// this is four adds and a compare rather than a walk.
///
/// Wall 1: `+ − × min max` only.
#[inline]
pub fn head_crossed(oy: f32, sy: f32, feet_mm: f32, t_lo: f32, t_hi: f32) -> bool {
    // A stop before the entry means the shot never got inside this body at
    // all on the part of the segment that survived the world.
    if t_hi < t_lo {
        return false;
    }
    let (a, b) = (oy + sy * t_lo, oy + sy * t_hi);
    let (lo, hi) = (a.min(b), a.max(b));
    let head_lo = feet_mm + (CAPSULE_HEIGHT_M - HEAD_BAND_M) * MM_PER_M;
    let head_hi = feet_mm + CAPSULE_HEIGHT_M * MM_PER_M;
    hi >= head_lo && lo <= head_hi
}

/// The nearest body whose closest approach to the segment `o + s·t` comes at
/// or before `stop_t`, skipping the shooter. Solved rather than sampled, so
/// a body is never missed between two taps — and compared against the
/// world's stop, so a body behind a trunk is never reached.
///
/// **The hit decision is unchanged by headshots and that is deliberate.**
/// What the winner now carries as well is the span it spent inside the
/// body's radius — the same planar quadratic the closest approach already
/// half-solves, finished. Who is hit is still decided at the closest
/// approach, exactly as it was before there was a head, so this landed
/// without moving a single existing hit/miss assertion in `tests/shoot.rs`
/// or `tests/gun.rs`. A headshot is a question asked **of a hit**, never a
/// second way to score one.
///
/// **`pose` decides where each candidate stood** (lag comp, the gun's
/// slice). The liveness tests below stay on the LIVE record and that is
/// `combat::strike`'s rule word for word: the ring stores a position, not a
/// life, and a body that has since died or left is not a target however
/// solid it looked `back` ticks ago.
fn nearest_body(
    players: &[Player; MAX_PLAYERS],
    o: (f32, f32, f32),
    s: (f32, f32, f32),
    stop_t: f32,
    owner: u32,
    pose: Pose<'_>,
) -> Option<BodyHit> {
    let (ox, oy, oz) = o;
    let (sx, sy, sz) = s;
    let mut best: Option<BodyHit> = None;
    let planar2 = sx * sx + sz * sz;
    for (j, v) in players.iter().enumerate() {
        if !v.active || v.dead || v.hp == 0 || v.id == owner {
            continue;
        }
        let at = pose.of(j, v);
        let (bx, by, bz) = (
            at.qx as f32 * POS_XZ_Q,
            at.qy as f32 * POS_Y_Q,
            at.qz as f32 * POS_XZ_Q,
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
        // shape. The head is a band off the top of this same cylinder
        // (`collide::HEAD_BAND_M`), never a second collider, so this test
        // is what it always was.
        if ddx * ddx + ddz * ddz > CAPSULE_RADIUS_M * CAPSULE_RADIUS_M {
            continue;
        }
        if cy < by || cy > by + CAPSULE_HEIGHT_M {
            continue;
        }
        // The rest of the quadratic the closest approach is the vertex of:
        // `|w + v·t|² = R²`, with `w` the muzzle-to-body planar offset. The
        // discriminant cannot be negative here — the compare above already
        // proved the line comes within `R` — but it is `max`ed at zero
        // anyway, because a `sqrt` of a float that is -1e-9 by rounding is
        // a NaN, and a NaN would spread through the span into
        // `head_crossed` and out of it as a silent false.
        let vv = ux * ux + uz * uz;
        let (enter, exit) = if vv <= 0.0 {
            // A purely vertical shot never leaves its own column, so the
            // span is the point the hit was decided at — the same `t = 0`
            // pin above, for the same reason.
            (t, t)
        } else {
            let (wx, wz) = (ax - bx, az - bz);
            let wv = wx * ux + wz * uz;
            let ww = wx * wx + wz * wz;
            let disc = (wv * wv - vv * (ww - CAPSULE_RADIUS_M * CAPSULE_RADIUS_M)).max(0.0);
            let half = disc.sqrt() / vv;
            let centre = -wv / vv;
            (
                (centre - half).clamp(0.0, 1.0),
                (centre + half).clamp(0.0, 1.0),
            )
        };
        if best.is_none_or(|b| t < b.t) {
            best = Some(BodyHit {
                t,
                slot: j,
                enter,
                exit,
                qy: at.qy,
            });
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
/// charges the revolver's `structure` against it. No falloff —
/// `content/weapons.toml` has no curve to read.
///
/// **It pays the head band exactly as an arrow does**, from the same two
/// functions and the same clip against the world's stop; this line said
/// "and no headshot" until headshot v0. The one difference is where the
/// multiplier is read from — a bullet takes it off `def` because a beam
/// has no flight to outlive a rebake, while an arrow copies it onto the
/// shaft at the draw.
///
/// # It is lag-compensated, and it is the only fight that now is
///
/// Melee rewound on 2026-08-29 (`combat::strike`, slice 4) and this pass
/// did not, which made the gun the **only** weapon on the shard decided by
/// ping — the asymmetry being worse than the uniform gap it replaced,
/// because lead error is largest exactly where the weapon is ranged. The
/// body scan now resolves against [`Pose::Rewound`] at the tick's granted
/// `favour`, and the head band is measured off the same rewound feet
/// ([`BodyHit::qy`]), so a crown is never solved at present-tick altitude
/// against a past-tick horizontal solve.
///
/// `favour` is indexed by **slot**, minted per tick by `World::tick` from
/// the shooter's own `Command::Input` and already clamped to
/// `Rewind::max_back()`. Zero — a slot nobody sent an input for, and every
/// non-server construction — makes `pose_at` return the live body, so a
/// zero favour is bit-identical to this pass before the parameter existed.
///
/// **The hurt bearing stays live**, `strike`'s rule and `strike`'s reason:
/// `EV_HURT` is an instruction to the victim (*turn this way*) and they are
/// at their present position when the arc appears. `range_cm` does rewind,
/// because it is a fact about the blow and is measured on the geometry the
/// hit was decided on.
#[allow(clippy::too_many_arguments)]
pub fn hitscan(
    seed: u64,
    haven: &terrain::Haven,
    cols: &ColIndex,
    occ: &mut Occupants,
    tick: u64,
    rewind: &Rewind,
    favour: &[u8; MAX_PLAYERS],
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
        //
        // **Rewound** (lag comp, the gun's slice): this shot was fired on
        // this tick, so the bodies it is asked about are put back where the
        // shooter's screen had them. The truncation argument above is
        // untouched by that — it is about which `t` is the minimum, and the
        // scan still returns the minimum of whatever positions it resolved.
        let seen = nearest_body(
            players,
            (ox, oy, oz),
            (sx, sy, sz),
            1.0,
            id,
            Pose::Rewound {
                rewind,
                tick,
                back: favour[i],
            },
        );
        let upto = match seen {
            Some(b) => (b.t * n as f32) as usize + 1,
            None => MAX_HITSCAN_MARK_SAMPLES,
        };
        let (stop_t, surf, built) =
            world_stop(seed, haven, cols, occ, (ox, oy, oz), (sx, sy, sz), n, upto);
        let best = seen.filter(|b| b.t <= stop_t);

        if let Some(BodyHit {
            t,
            slot: j,
            enter,
            exit,
            qy: feet_q,
        }) = best
        {
            let range_cm = (reach * t / 10.0) as u16;
            let v = &mut players[j];
            // Same question the arrow asks, from the same two functions —
            // the head band is a property of the body, not of what is
            // travelling towards it. Clipped against the world's stop for
            // the same reason: a beam that dies in a wall did not reach
            // what was standing behind the wall.
            //
            // **The rewound feet, not the live ones**, and this is the
            // half a rewind that stopped at the scan would have got wrong:
            // the band is an offset off the top of the cylinder the hit was
            // solved against, so a victim who jumped or walked downhill in
            // the last quarter-second would otherwise have a head floating
            // clear of the body the bullet met. At favour 0 this is the
            // live `qy` to the bit.
            let feet_mm = feet_q as f32 * (POS_Y_Q * MM_PER_M);
            let dmg = if head_crossed(oy, sy, feet_mm, enter, exit.min(stop_t)) {
                crate::combat::headshot(def.damage, def.headshot_mult)
            } else {
                def.damage
            };
            // The funnel, reduced: a bullet is a hit like any other, and
            // armor blunts it (armor v0, 2026-08-19 — this said "the day
            // armor lands" for exactly one day).
            let h = crate::combat::hurt(cc, v, dmg);
            let died = h.died;
            let (vid, left, vmax) = (v.id, h.left as u32, v.hp_max as u32);
            let sector = crate::combat::bearing_sector(
                qx as i64 - v.body.qx as i64,
                qz as i64 - v.body.qz as i64,
            );
            events.push(EV_HIT, id, vid, dmg as u32);
            // A beam has no flight to reverse, so this is the muzzle itself —
            // the shooter's body this tick, which is where they still are.
            // **And the victim's live body, not the rewound one**, which is
            // `strike`'s rule verbatim: this bearing is an instruction to
            // the person it is sent to, who is at their present position
            // when the arc appears. At a favour of 7 the rewound bearing
            // would describe neither player's situation and is easily a
            // whole sector of the sixteen out.
            events.push(EV_HURT, vid, sector as u32, dmg as u32);
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
