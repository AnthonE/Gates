//! Kinematic capsule vs terrain and placed pieces (TERRAIN.md §3), shared
//! verbatim by server and wasm prediction. Quantize-both-sides: positions
//! and vertical velocity live as integer quanta; each step decodes,
//! integrates with walled float ops, and re-quantizes — the sim runs on
//! the values it transmits (NETCODE.md §3), so prediction can never drift
//! by rounding. Piece collision comes in through `cols` (collide.rs): the
//! server passes its piece store's index, the predictor the client
//! mirror's — same code, same bits.
//!
//! Movement constants are proposed defaults registered in DECISIONS.md
//! §open (sim movement constants row); capsule and piece-slab dimensions
//! in the piece collision v0 row.

use crate::collide::{self, ColIndex};
use crate::fmath::floor_i32;
use crate::input::{InputFrame, BTN_AIM, BTN_CROUCH, BTN_JUMP, BTN_SPRINT};
use crate::occupy::Occupants;
use crate::terrain::{self, CLIFF_SLOPE_RATIO, ISLAND_SIZE, SEA_LEVEL};
use crate::yaw_lut::yaw_dir;

/// Position quantum, x/z: 3 cm (DESIGN.md §5.5).
pub const POS_XZ_Q: f32 = 0.03;
/// Position quantum, y: 1 cm (DESIGN.md §5.5).
pub const POS_Y_Q: f32 = 0.01;
/// Velocity quantum: 1 cm/s (DECISIONS.md §open).
pub const VEL_Q: f32 = 0.01;

/// Fixed tick delta — the only dt the sim knows (30 Hz).
pub const DT: f32 = 1.0 / 30.0;

pub const WALK_SPEED: f32 = 3.0;
pub const SPRINT_SPEED: f32 = 5.5;
/// A crouched walk, m/s: the reference's duck speed (walk 2.8, run 5.5,
/// duck 1.7). Crouch beats sprint and the drawn-bow walk — a body holding
/// both keys is sneaking, and `brain::sense` agrees.
pub const CROUCH_SPEED: f32 = 1.7;
pub const GRAVITY: f32 = 20.0;
pub const TERMINAL_VELOCITY: f32 = 50.0;
pub const STEP_UP: f32 = 0.6;
/// Launch velocity of a jump, m/s (DECISIONS.md §open, jump verb v0).
///
/// Derived from the two constants it sits between, not chosen for feel. The
/// apex is `JUMP_SPEED²/(2·GRAVITY)` = 1.22 m, and it has to clear one bound
/// and stay under the other:
///
/// - **Above `STEP_UP` (0.6 m)**, or the verb does nothing walking cannot
///   already do — the capsule steps 0.6 m for free, so a jump that peaks
///   below it is an animation. 1.22 m is 2× it, and clears the 0.6 m step
///   plus a foundation's `PIECE_LIFT_M` (0.3 m) together.
/// - **Well under `build::LEVEL_H_M` (3.0 m)**, or a wall stops being a wall
///   and the entire build/raid premise goes with it. 1.22 m is 41% of a
///   storey, so a wall is not cleared by jumping. The capsule's head CAN
///   reach an interior ceiling; `step` stops that upward motion separately.
///
/// Both bounds are asserted against those constants in `tests/jump.rs`
/// rather than left to this comment, so moving `STEP_UP` or `LEVEL_H_M`
/// without re-deriving this reddens rather than drifts.
pub const JUMP_SPEED: f32 = 7.0;

/// A walk into a round occupant that is at most this share of the move
/// sideways (about 9°) is square on and stops, as against a wall; past it the
/// body slides round at no less than [`ROUND_SLIDE_MIN`] of its walk.
pub const ROUND_SLIDE_SQUARE: f32 = 0.15;
pub const ROUND_SLIDE_MIN: f32 = 0.6;
pub const WADE_SPEED_MULT: f32 = 0.5;
/// Ground at or below this height counts as wading (alpha swim = slow
/// wade, TERRAIN.md §3).
pub const WADE_GROUND_MAX: f32 = SEA_LEVEL + 0.4;

// ── Swimming (water pass, 2026-10-21) ─────────────────────────────────────
//
// The reference floats a body once the water reaches its chest and lets it
// swim where it looks (`reference/WATER.md`). Every water surface here is
// `SEA_LEVEL` — the sea and the carved lakes alike — and the ground is always
// under the feet, so feet this far under the line ARE in water this deep: the
// state is a function of `qy` alone, with nothing new on the body or the wire.

/// Water over the feet at which a body swims instead of wading, metres.
pub const SWIM_DEPTH_M: f32 = 1.15;
/// Where a floating body's feet hang under the surface: the head out.
pub const FLOAT_DEPTH_M: f32 = 1.3;
/// The mouth over the feet: under the surface past this, a body holds its
/// breath (`World::breathe`).
pub const MOUTH_M: f32 = 1.5;
/// Swimming, m/s: slower than a walk, as the reference's is.
pub const SWIM_SPEED: f32 = 1.6;
pub const SWIM_SPRINT_SPEED: f32 = 2.4;
/// Diving and rising, m/s: crouch, jump, or swimming forward looking down
/// or up past [`SWIM_PITCH_DEAD`].
pub const SWIM_VERT_SPEED: f32 = 1.8;
/// The drift back up to [`FLOAT_DEPTH_M`] when nothing holds a body down.
pub const BUOY_SPEED: f32 = 1.2;
/// The share of the gap to the wanted vertical speed water closes a tick:
/// a dive into a lake slows instead of hitting the bed at fall speed.
pub const WATER_DRAG: f32 = 0.15;
/// The share of a full look down (or up) under which swimming forward stays
/// level, about 30°, so a swimmer eyeing the shore does not sink toward it.
pub const SWIM_PITCH_DEAD: f32 = 0.35;
/// How high above its feet a swimmer hauls out onto an edge: a dock, a
/// foundation's lip, a bank. Walking keeps [`STEP_UP`].
pub const SWIM_CLIMB_M: f32 = 1.8;

/// Feet at `feet_y` are swimming.
#[inline]
pub fn swimming_at(feet_y: f32) -> bool {
    feet_y < SEA_LEVEL - SWIM_DEPTH_M
}

/// A body is swimming: in water too deep to stand in with its head out.
/// Guns, bows and building wait for dry feet (the reference's rule).
#[inline]
pub fn swimming(body: &Body) -> bool {
    swimming_at(body.qy as f32 * POS_Y_Q)
}

/// The mouth of feet at `feet_y` is under water. The one hook a sealed,
/// air-filled room would answer to, when there is one to build.
#[inline]
pub fn mouth_under(feet_y: f32) -> bool {
    feet_y + MOUTH_M < SEA_LEVEL
}
/// Hard world border: the sea ring beyond the island is not walkable v1.
pub const BORDER_MARGIN: f32 = 8.0;

#[inline]
pub fn quant_xz(m: f32) -> i32 {
    floor_i32(m / POS_XZ_Q + 0.5)
}
#[inline]
pub fn quant_y(m: f32) -> i32 {
    floor_i32(m / POS_Y_Q + 0.5)
}
#[inline]
pub fn quant_vel(v: f32) -> i32 {
    floor_i32(v / VEL_Q + 0.5)
}

/// The capsule's mutable state, all quantized. Lives inside `Player`.
///
/// `Eq` is derivable **because** every field is quantized — there is no
/// float in here to make equality a tolerance question, which is the same
/// property the wire and `state_hash` rest on. `persist.rs` compares two
/// bodies for exact identity (the autosave sweep skips a player who has not
/// moved), and that comparison is only meaningful for this reason.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Body {
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
    pub qvy: i32,
    pub grounded: bool,
}

impl Body {
    pub fn at(seed: u64, haven: &terrain::Haven, x: f32, z: f32) -> Self {
        let y = terrain::ground(seed, haven, x, z);
        Self {
            qx: quant_xz(x),
            qy: quant_y(y),
            qz: quant_xz(z),
            qvy: 0,
            grounded: true,
        }
    }
}

/// Can the capsule move from height `from_y` onto ground at `to_ground`,
/// covering planar distance `run`? Step-up bounds any single rise; the
/// cliff ratio keeps >50° terrain unwalkable (TERRAIN.md §3).
#[inline]
fn climbable(from_y: f32, to_ground: f32, run: f32) -> bool {
    let rise = to_ground - from_y;
    rise <= STEP_UP && rise <= run * CLIFF_SLOPE_RATIO
}

/// How fast a body slides down a face, m/s along the fall line: nobody
/// stands on ground too steep to climb. Until 2026-10-17 the cliff ratio
/// held only a step's own rise, so a body walked along a face, or up it on a
/// diagonal, and stood inside the rock the client draws proud of every face
/// (`render/cliff_skin.rs`).
pub const SLIDE_SPEED: f32 = 4.0;

/// The fall line at (`x`, `z`), a unit vector downhill, where the solid
/// surface there is a face: too steep to climb at the nearest whole-metre
/// point and at all eight round it, the gentlest of the 3×3 that the skin's
/// own mask reads (`face.rs`). `None` where a body may stand. Read off
/// [`crate::face::surface`], so a shelf in the drawn rock is ground and a
/// riser is a face; off the faces that is the sim's ground.
pub fn face_fall(
    occ: &mut Occupants,
    seed: u64,
    haven: &terrain::Haven,
    x: f32,
    z: f32,
) -> Option<(f32, f32)> {
    occ.cache
        .fall(seed, haven, floor_i32(x + 0.5), floor_i32(z + 0.5))
}

/// [`face_fall`] at whole-metre (`ix`, `iz`), from the surface's heights
/// there (`surf`).
pub(crate) fn fall_at(
    ix: i32,
    iz: i32,
    mut surf: impl FnMut(i32, i32) -> f32,
) -> Option<(f32, f32)> {
    let mut grade = |i: i32, j: i32| {
        let sx = (surf(i + 1, j) - surf(i - 1, j)) * 0.5;
        let sz = (surf(i, j + 1) - surf(i, j - 1)) * 0.5;
        (sx, sz)
    };
    let (sx, sz) = grade(ix, iz);
    let mut gentlest = sx * sx + sz * sz;
    for dz in -1..=1 {
        for dx in -1..=1 {
            if dx != 0 || dz != 0 {
                let (gx, gz) = grade(ix + dx, iz + dz);
                gentlest = gentlest.min(gx * gx + gz * gz);
            }
        }
    }
    if gentlest < CLIFF_SLOPE_RATIO * CLIFF_SLOPE_RATIO {
        return None;
    }
    let s = (sx * sx + sz * sz).sqrt();
    Some((-sx / s, -sz / s))
}

/// Highest quantized feet position that keeps the head below a slab. Round
/// down, then check the decoded sum too: float subtraction/division must not
/// round a contact back inside the ceiling on the next tick.
fn ceiling_limit(underside: f32) -> i32 {
    let mut q = floor_i32((underside - collide::CAPSULE_HEIGHT_M) / POS_Y_Q);
    if q as f32 * POS_Y_Q + collide::CAPSULE_HEIGHT_M > underside {
        q -= 1;
    }
    q
}

/// One fixed-timestep step of one capsule. Pure: same inputs, same result,
/// native or wasm. `cols` is the placed-piece collision index — the
/// server's store and the client's mirror step through this same code.
/// `occ` is the scattered-world collision bundle (occupy.rs) and is here for
/// the same reason `cols` is: a wall only one side knows about is a
/// misprediction every time a player leans on a tree.
pub fn step(
    seed: u64,
    haven: &terrain::Haven,
    cols: &ColIndex,
    occ: &mut Occupants,
    body: &mut Body,
    frame: &InputFrame,
) {
    step_scaled(seed, haven, cols, occ, body, frame, 1.0);
}

/// [`step`] with the top speed times `scale`: an animal that outruns a
/// sprinting player (`MobDef::run_pct`). A player always steps at 1, and
/// `x * 1.0` is `x` bit for bit, so prediction is untouched.
pub fn step_scaled(
    seed: u64,
    haven: &terrain::Haven,
    cols: &ColIndex,
    occ: &mut Occupants,
    body: &mut Body,
    frame: &InputFrame,
    scale: f32,
) {
    let x = body.qx as f32 * POS_XZ_Q;
    let y = body.qy as f32 * POS_Y_Q;
    let z = body.qz as f32 * POS_XZ_Q;
    let mut vy = body.qvy as f32 * VEL_Q;

    // Wish direction: move vec rotated by the yaw LUT, magnitude clamped.
    let (fx, fz) = yaw_dir(frame.yaw);
    let (rx, rz) = (fz, -fx);
    let mf = frame.move_z as f32 * (1.0 / 127.0);
    let ms = frame.move_x as f32 * (1.0 / 127.0);
    let mut wx = fx * mf + rx * ms;
    let mut wz = fz * mf + rz * ms;
    let len2 = wx * wx + wz * wz;
    if len2 > 1.0 {
        let inv = 1.0 / len2.sqrt();
        wx *= inv;
        wz *= inv;
    }

    // Wade on the effective ground (a floor over shallows stays dry).
    // `body_ground` covers built planes, ramps and solid-deploy tops;
    // `occ.body_ground` covers scattered tops — the crate, the boulder, the
    // shelter plinth, the town's and the ziggurat's stone (deploy collision
    // v0 / `slot_ground`). Without the occupant term here, a body standing
    // on one would read the terrain under it as ground and fall inside on
    // the next vertical pass.
    //
    // Both are the BODY's footprint — every top its disc overlaps — not the
    // point under its centre, because that disc is what every veto below
    // stops it with. A point here let a body stand past a ledge with its
    // centre over air and fall down the face half inside it, which the
    // veto lifts then treated as "already inside" (`body_ground`'s doc).
    // The solid ground (`face::surface`): the sim's, or on a face the rock
    // drawn over it.
    let terrain_here = crate::face::surface(occ, seed, haven, x, z);
    let hard_here =
        collide::body_ground(seed, haven, cols, x, z, y).max(occ.body_ground(seed, x, z, y));
    let ground_here = terrain_here.max(hard_here);
    // Off its feet over a face, a body is coming down it.
    let slide = if !body.grounded && hard_here <= terrain_here && y - terrain_here <= STEP_UP {
        face_fall(occ, seed, haven, x, z)
    } else {
        None
    };
    // A drawn bow walks (`BTN_AIM`, `reference/PROJECTILES.md` §6): the
    // draw costs the legs as well as the time. Crouch is read off the frame
    // alone, like the draw, so the predictor needs nothing else to agree.
    let swim = swimming_at(y);
    let mut speed = if swim {
        if frame.buttons & BTN_SPRINT != 0 {
            SWIM_SPRINT_SPEED
        } else {
            SWIM_SPEED
        }
    } else if frame.buttons & BTN_CROUCH != 0 {
        CROUCH_SPEED
    } else if frame.buttons & BTN_SPRINT != 0 && frame.buttons & BTN_AIM == 0 {
        SPRINT_SPEED
    } else {
        WALK_SPEED
    };
    if !swim && ground_here <= WADE_GROUND_MAX {
        speed *= WADE_SPEED_MULT;
    }
    speed *= scale;
    // A swimmer hauls out onto what a walker would have to climb.
    let step_up = if swim { SWIM_CLIMB_M } else { STEP_UP };

    // Horizontal, axis-resolved: full move, then x-only, then z-only.
    // Walls veto a candidate outright; a built surface accepts on the
    // step rule alone (a stair-step, not a cliff — the ramp and slab
    // rises are bounded by design), terrain keeps the cliff ratio.
    let mut dx = wx * speed * DT;
    let mut dz = wz * speed * DT;
    if let Some((fall_x, fall_z)) = slide {
        // Steered across the face, and down it at the slide's own pace.
        let along = dx * fall_x + dz * fall_z;
        dx += fall_x * (SLIDE_SPEED * DT - along);
        dz += fall_z * (SLIDE_SPEED * DT - along);
    }
    let mut nx = x;
    let mut nz = z;
    // The effective ground under wherever the body ends up. It starts as the
    // ground under where it started, because that is what "no candidate was
    // accepted" resolves to, and the accepted candidate overwrites it with
    // the pair that candidate already had to compute.
    //
    // **This is a saved terrain fan per step, not a tidy-up.** The vertical
    // pass below used to re-derive this from (nx, nz), which is by
    // construction either (x, z) — the identical expression on the line
    // above — or a candidate whose `g` and `pg` were computed inside the
    // loop and thrown away. `terrain::height` is ~12 `noise2` evaluations
    // (a 2-octave warp on each axis, a 5-octave relief, the ridge tap, and
    // the coast fBm inside `continent`), and it was the single largest line
    // item in the shard profile: at `MAX_PLAYERS` bodies plus the roster,
    // 30 times a second, the duplicate was ~30 % of every `noise2` the sim
    // ran. A body standing still — a sleeper, a corpse on the death screen,
    // anybody not pressing a key — paid for the whole fan twice.
    //
    // Arithmetically identical by construction, which is the only reason it
    // may land: same `max` of the same two calls at the same coordinates, so
    // `test_replay`, `test_terrain_golden` and `test_parity_wasm` are the
    // proof rather than the risk.
    let mut ground_sel = ground_here;
    // Whether that ground is bare terrain, and, if the accepted candidate
    // already asked, whether it is a face.
    let mut bare_sel = hard_here <= terrain_here;
    let mut face_sel = None;
    // Cache a queried destination ceiling (including "none") for the vertical
    // pass. A rising step must not sample each column's terrain band twice.
    let mut ceiling_sel = None;
    // Whether the body is ALREADY inside an occupant, resolved at most once
    // and only if a candidate is vetoed by one. See the veto below.
    let mut inside: Option<bool> = None;
    // The same latch for solid deployables, separately: being stuck in a
    // pine must not license walking through a furnace.
    let mut inside_dep: Option<bool> = None;
    // ...and for plane flanks, third and apart for the same reason.
    let mut inside_plane: Option<bool> = None;
    if len2 > 0.0 || slide.is_some() {
        let clamp_x = |v: f32| v.clamp(BORDER_MARGIN, ISLAND_SIZE - BORDER_MARGIN);
        // The full move, then each axis alone, then — only once all three
        // fail against a round occupant — the move's part tangent to it:
        // walked into head-on, a trunk or a cactus stops both axis slides,
        // and the body stood pinned to it (2026-10-19, *"you kinda get
        // stuck"* against a cactus pricking you once a second).
        let mut round = false;
        for i in 0..4 {
            let (cx, cz) = match i {
                0 => (x + dx, z + dz),
                1 => (x + dx, z),
                2 => (x, z + dz),
                _ => {
                    let Some((sx, sz)) = round
                        .then(|| occ.round_blocker(seed, x + dx, z + dz, y))
                        .flatten()
                    else {
                        break;
                    };
                    let (ox, oz) = (x - sx, z - sz);
                    let d2 = ox * ox + oz * oz;
                    let into = dx * ox + dz * oz;
                    if d2 <= 0.0 || into >= 0.0 {
                        break;
                    }
                    // The radial part removed. Square on is still a stop, as
                    // a wall is; glancing off it keeps at least
                    // `ROUND_SLIDE_MIN` of the walk, so the slide round does
                    // not crawl.
                    let (tx, tz) = (dx - ox * into / d2, dz - oz * into / d2);
                    let (t2, m2) = (tx * tx + tz * tz, dx * dx + dz * dz);
                    if t2 < m2 * ROUND_SLIDE_SQUARE * ROUND_SLIDE_SQUARE {
                        break;
                    }
                    let k = (m2 * ROUND_SLIDE_MIN * ROUND_SLIDE_MIN / t2)
                        .sqrt()
                        .max(1.0);
                    // Plus a quantum outward: quantizing the step below can
                    // round it 2 cm back in, inside a wide volume, and the
                    // body stuck halfway round a ball cactus (2026-10-20).
                    let out = POS_XZ_Q / d2.sqrt();
                    (x + tx * k + ox * out, z + tz * k + oz * out)
                }
            };
            // ⚠ **Tested where the body will actually stand: quantized.** The
            // body is stored in `POS_XZ_Q` steps, so an unquantized candidate
            // that clears a wall by less than half a step can be stored
            // inside it — and inside is exactly where the veto below lifts,
            // so the next tick walks straight through. The 4,096 m island's
            // seed 1 found it at the haven shelter's back wall.
            let cx = quant_xz(clamp_x(cx)) as f32 * POS_XZ_Q;
            let cz = quant_xz(clamp_x(cz)) as f32 * POS_XZ_Q;
            let run2 = (cx - x) * (cx - x) + (cz - z) * (cz - z);
            if run2 <= 0.0 {
                continue;
            }
            if collide::blocked(seed, haven, cols, x, z, cx, cz, y) {
                continue;
            }
            // Trees, boulders, barrels, crates and the shelter's walls
            // (occupy.rs). A destination test, not a swept one, which the
            // const block there proves cannot tunnel at sprint speed.
            //
            // The veto lifts when the body is already inside an occupant —
            // a node can respawn around a standing player, and a rule that
            // vetoed every candidate would then pin them there forever with
            // no verb that frees them. Walking out is the only escape a
            // capsule has, so being stuck must never be absorbing.
            if occ.blocks(seed, cx, cz, y) {
                round |= i == 0;
                if inside.is_none() {
                    inside = Some(occ.blocks(seed, x, z, y));
                }
                if inside != Some(true) {
                    continue;
                }
            }
            // Solid deployables — the furnace, the box, the hearth
            // (collide::deploy_blocked, deploy collision v0). The same
            // destination test and the same veto-lift, latched apart: a
            // deploy CAN be placed around a standing body, and walking out
            // must stay the escape.
            if collide::deploy_blocked(seed, haven, cols, cx, cz, y) {
                if inside_dep.is_none() {
                    inside_dep = Some(collide::deploy_blocked(seed, haven, cols, x, z, y));
                }
                if inside_dep != Some(true) {
                    continue;
                }
            }
            // The flank of a foundation, a floor or a roof (piece flanks
            // v0, `collide::plane_blocked`). Planes were ground and nothing
            // else until 2026-08-21, so a body walked into the side of a
            // base and stood inside the slab and the skirt under it — the
            // camera in a wall of earth. Third latch, and it is separate
            // from the other two for their reason: being stuck in a pine
            // must not license walking through a furnace, and being stuck
            // in either must not license walking into a foundation.
            //
            // A foundation CAN be placed around a standing body — `place`
            // asks nothing about who is there — so the lift is not
            // optional. It is the whole of why a base built over somebody
            // is a thing they walk out of rather than a grave.
            if collide::plane_blocked(seed, haven, cols, cx, cz, y) {
                if inside_plane.is_none() {
                    inside_plane = Some(collide::plane_blocked(seed, haven, cols, x, z, y));
                }
                if inside_plane != Some(true) {
                    continue;
                }
            }
            let g = crate::face::surface(occ, seed, haven, cx, cz);
            let pg = collide::body_ground(seed, haven, cols, cx, cz, y);
            let og = occ.body_ground(seed, cx, cz, y);
            // A built or hard surface accepts on the step rule alone; bare
            // terrain keeps the cliff ratio. Occupant tops count as hard
            // for the reason piece tops do — the rise onto a plinth is
            // bounded by the lid, not by a slope that does not exist.
            let hard = pg.max(og);
            let mut face = None;
            let ok = if hard > g {
                hard - y <= step_up
            } else if !climbable(y, g, run2.sqrt()) {
                false
            } else if g > y {
                // Up onto a face from any bearing: the ratio above holds
                // only the rise along the step, which a diagonal keeps low.
                let f = face_fall(occ, seed, haven, cx, cz).is_some();
                face = Some(f);
                !f
            } else {
                true
            };
            if ok {
                let next_ground = g.max(hard);
                if next_ground > y {
                    let ceiling = collide::piece_ceiling(
                        seed,
                        haven,
                        cols,
                        quant_xz(cx) as f32 * POS_XZ_Q,
                        quant_xz(cz) as f32 * POS_XZ_Q,
                        y,
                    );
                    // A ramp must stop horizontally when its next step has
                    // no headroom; clamping only y would walk the body into
                    // the ramp. Test where quantization puts it.
                    if ceiling.is_some_and(|c| quant_y(next_ground) > ceiling_limit(c)) {
                        continue;
                    }
                    ceiling_sel = Some(ceiling);
                }
                nx = cx;
                nz = cz;
                ground_sel = next_ground;
                bare_sel = hard <= g;
                face_sel = face;
                break;
            }
        }
    }

    // Whether the body ends the step over a face, which it slides down
    // rather than stands on. A grounded body that did not move was asked
    // when it arrived.
    let moved = nx != x || nz != z;
    let on_face = bare_sel
        && (moved || !body.grounded)
        && face_sel.unwrap_or_else(|| {
            (!moved && slide.is_some()) || face_fall(occ, seed, haven, nx, nz).is_some()
        });

    // Vertical: snap within step range, otherwise fall. Built surfaces
    // count as ground only when the step rule already admitted them
    // (piece_ground's lid), so a floor overhead is a ceiling, not a lift.
    let ground = ground_sel;
    let mut ny = y;
    // A jump is the one thing that takes a *grounded* body off the snap and
    // onto the ballistic path. `grounded` is the whole gate: it is recomputed
    // at the bottom of every step and is false for the entire arc, so a held
    // button cannot double-jump — the second press is simply not grounded.
    // Landing re-arms it, which makes a held button a hop on every touchdown.
    // That is the genre's behaviour and it buys no speed here, because this
    // capsule has no air-control penalty and no ground friction to escape:
    // horizontal speed is `WALK_SPEED`/`SPRINT_SPEED` in the air exactly as
    // it is on the ground.
    let jumping = !swim && body.grounded && frame.buttons & BTN_JUMP != 0;
    if jumping {
        vy = JUMP_SPEED;
    }
    // A slide keeps to the face like a walk keeps to the ground, so its foot
    // is not a fall.
    let held = body.grounded || slide.is_some();
    if swim {
        // In the water: jump rises, crouch dives, swimming forward looking
        // well down or up steers; otherwise the body drifts back up to float
        // with its head out. Water drag closes on the wanted speed rather
        // than gravity adding to it, so a dive off a cliff slows in the lake.
        let float_y = SEA_LEVEL - FLOAT_DEPTH_M;
        let up = frame.buttons & BTN_JUMP != 0;
        let down = frame.buttons & BTN_CROUCH != 0;
        let mut want = match (up, down) {
            (true, false) => SWIM_VERT_SPEED,
            (false, true) => -SWIM_VERT_SPEED,
            _ => 0.0,
        };
        if want == 0.0 && mf > 0.0 {
            // The look as a share of straight up (1) or down (-1): linear in
            // the angle, which is all an intent needs and costs no trig.
            let look = frame.pitch as f32 * (2.0 / 255.0) - 1.0;
            if !(-SWIM_PITCH_DEAD..=SWIM_PITCH_DEAD).contains(&look) {
                want = mf * look * SWIM_VERT_SPEED;
            }
        }
        if want == 0.0 && y < float_y {
            want = BUOY_SPEED;
        }
        vy += (want - vy) * WATER_DRAG;
        ny += vy * DT;
        // Swimming does not lift a body out of the water: the surface holds
        // it at the float line. Out is a haul onto an edge, above.
        if vy > 0.0 && ny > float_y {
            ny = float_y.max(y);
            vy = 0.0;
        }
        if ny <= ground {
            ny = ground;
            vy = vy.max(0.0);
        }
    } else if !jumping && held && ground - y <= STEP_UP && y - ground <= STEP_UP {
        ny = ground;
        vy = 0.0;
    } else {
        vy = (vy - GRAVITY * DT).max(-TERMINAL_VELOCITY);
        ny += vy * DT;
        if ny <= ground {
            ny = ground;
            vy = 0.0;
        }
    }
    if ny > y {
        if let Some(ceiling) = ceiling_sel.unwrap_or_else(|| {
            collide::piece_ceiling(
                seed,
                haven,
                cols,
                quant_xz(nx) as f32 * POS_XZ_Q,
                quant_xz(nz) as f32 * POS_XZ_Q,
                y,
            )
        }) {
            let limit = ceiling_limit(ceiling);
            if quant_y(ny) > limit {
                ny = limit as f32 * POS_Y_Q;
                vy = 0.0;
            }
        }
    }
    body.grounded = ny <= ground + 0.001 && !on_face;

    body.qx = quant_xz(nx);
    body.qy = quant_y(ny);
    body.qz = quant_xz(nz);
    body.qvy = quant_vel(vy);
}
