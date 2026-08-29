//! Which way is right — the one place in the client that answers it.
//!
//! Pure and unconditional (no Bevy), because the answer is arithmetic and
//! the arithmetic is what was wrong. It is a module rather than four lines
//! inside `render::input` for the reason `ui/` exists: a mapping that lives
//! inside a system is a mapping no test in the code tier can call.
//!
//! ## The bug this exists to have fixed
//!
//! Reported by the operator, 2026-08-06: *"mouse camera movement in the old
//! web build was all inverse even left to right"*. It was, on both clients,
//! and the half nobody had noticed is that **the strafe was mirrored too**.
//!
//! The world is right-handed and Y-up, so for a body facing `f = (fx, 0, fz)`
//! the body's own right `r` is fixed by `r × up = −f` — the identity that
//! also holds for Bevy's camera basis (`right × up = backward`). Solving it:
//!
//! ```text
//! r = (−fz, 0, fx)
//! ```
//!
//! Two places disagreed with that, and they disagreed *consistently*, which
//! is exactly why the client felt merely "wrong" rather than broken:
//!
//! 1. **`sim_core::movement::step` calls `(fz, −fx)` the right vector.** It
//!    is the LEFT vector — negate the expression above. So the wire's
//!    `move_x` is a strafe-LEFT axis, and a client that sends `D → +127`
//!    walks the player left.
//! 2. **`yaw` increasing rotates `f` toward `+X`** (`yaw_lut.rs`), and `+X`
//!    is toward the body's left by the identity above. So a client that
//!    adds a positive mouse `dx` to yaw turns the view left.
//!
//! Both are inside this crate's gift to fix and **neither costs a sim, wire
//! or golden change**: the sim's `yaw` is a bearing and its `move_x` is a
//! signed axis, and what those two mean physically is the client's half of
//! the contract. Renaming the sim's `(rx, rz)` would be honest and is not
//! this slice's business; correcting the sim's *basis* would move every
//! replay hash for a cosmetic naming error, which is the wrong trade.
//!
//! The browser client is left as it is. It is being retired
//! (`CLAUDE.md` §what this is), it is inverted *consistently*, and a
//! half-fixed second client is worse than a uniformly mirrored one.

use sim_core::yaw_dir;

/// Radians per pixel of mouse travel (`DECISIONS.md` §open, client cosmetics).
pub const MOUSE_RAD_PER_PX: f32 = 0.0022;

/// How close to straight up or down the view may get, radians from level.
/// The browser's `PITCH_LIMIT`; a pitch that reaches the pole makes the yaw
/// basis degenerate.
pub const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.02;

/// Wire yaw from a free-running radian yaw.
///
/// The sim's movement only ever reads the top 8 bits (`yaw_lut.rs`), so a
/// client that sent a finer bearing would be predicting on a heading the
/// server cannot face — and **both sides must quantize or prediction drifts
/// by rounding** (`CLAUDE.md` traps).
pub fn yaw_u16(yaw: f32) -> u16 {
    let t = (yaw / std::f32::consts::TAU).rem_euclid(1.0);
    (t * 65536.0) as u32 as u16
}

/// Wire pitch: 128 level, 255 straight up.
pub fn pitch_u8(pitch: f32) -> u8 {
    let v = ((pitch / std::f32::consts::PI + 0.5) * 255.0).round();
    v.clamp(0.0, 255.0) as u8
}

/// Degrees clockwise from north for a world-XZ direction — **the one place
/// that answers which compass point a world axis is.**
///
/// **North is `+Z` and east is `-X`** (`DECISIONS.md` 2026-08-15). That is
/// not a taste call, it is forced by the header above: the world is
/// right-handed and Y-up, so a body facing `+Z` has its right at `-X`, and a
/// compass card's east is the right hand of a body facing north. The tree
/// used to compute `atan2(fx, fz)` and call the result a bearing, which
/// labels `+X` east — a REFLECTION of the truth, not an offset, so all four
/// cardinals sat 180° off and **a player turning right watched the number
/// fall** (probed: 22.5° → 9.8° → 357.2° over three 100 px pushes right).
///
/// What this spends is the old convention's one property, `bearing == wire
/// yaw`. The row above says why that was the right side to spend: the
/// identity was *already* false in the tree, because the raid readout
/// computed its own `atan2` and never came through here. It cannot be false
/// again — that readout is a caller of this function now, so there is one
/// conversion rather than two that must agree.
///
/// **The `-0.0` trap, paid for once.** The obvious spelling `(-dx).atan2(dz)`
/// yields `-0.0` due north, and `f32::rem_euclid`'s guard is `r < 0.0`, which
/// `-0.0` does not satisfy — so north printed **`-00°`** at both `{:03.0}`
/// sites. Subtracting from 360 first cannot produce it.
pub fn bearing_of(dx: f32, dz: f32) -> f32 {
    // Trig is fine here: this is the client, not `sim-core`.
    (360.0 - dx.atan2(dz).to_degrees()).rem_euclid(360.0)
}

/// The compass bearing this client reports, degrees clockwise from north.
///
/// **One sample, one fact.** The compass strip and the map both print a
/// heading, and taking them from two computations is how they come to
/// disagree by a degree at a boundary — `web/src/main.js` asserts it calls
/// `yawU16()` exactly once per frame for precisely this reason, and names the
/// alternative a temporal seam. This is that assertion made structural: there
/// is one function, both callers use it, and there is nothing to keep in step.
///
/// Quantized through the sim's own LUT first, so the number on screen is a
/// bearing the server can actually face rather than the free-running float
/// between two of them. The axes are [`bearing_of`]'s, which is also what
/// `ui::map`'s hillshade, column lettering and x term rest on.
pub fn bearing_deg(yaw: f32) -> f32 {
    let (fx, fz) = yaw_dir(yaw_u16(yaw));
    bearing_of(fx, fz)
}

/// The body's right, in world XZ, for a wire yaw. `r × up = −f`.
///
/// This is the function the whole module exists for: everything else here
/// is a sign chosen to agree with it, and `crates/client/tests/look.rs`
/// checks it against Bevy's own `Transform::right()` rather than against
/// the derivation above — an identity restated is not an identity checked.
pub fn right_dir(yaw: u16) -> (f32, f32) {
    let (fx, fz) = yaw_dir(yaw);
    (-fz, fx)
}

/// The wire's `(move_x, move_z)` for a physical intent.
///
/// `fwd` is `+1` for `W` and `-1` for `S`; `right` is `+1` for `D` and `-1`
/// for `A`. Both are clamped to `-1..=1` so a caller that sums two keys
/// cannot overrun the axis.
///
/// **`move_x` comes back negated**, and that is the fix rather than a typo:
/// the sim's `move_x` axis points along the body's LEFT (see the header), so
/// the client that wants to strafe right asks for a negative one.
pub fn move_axes(fwd: i32, right: i32) -> (i8, i8) {
    let f = fwd.clamp(-1, 1);
    let r = right.clamp(-1, 1);
    ((-r * 127) as i8, (f * 127) as i8)
}

/// Free-running yaw after a mouse move of `dx` pixels.
///
/// **Subtracted**, for the second half of the header: yaw increasing turns
/// the view left, and a mouse pushed right means turn right.
pub fn yaw_after(yaw: f32, dx_px: f32, rad_per_px: f32) -> f32 {
    yaw - dx_px * rad_per_px
}

/// Free-running pitch after a mouse move of `dy` pixels, clamped.
///
/// Here for symmetry with [`yaw_after`] rather than because it was wrong —
/// it was not. `dy` is positive DOWNWARD (both Bevy's `AccumulatedMouseMotion`
/// and the browser's `movementY`), and subtracting it lowers the view, which
/// is the uninverted convention every reference ships as the default.
/// `invert` flips exactly this and nothing else; a yaw inversion is not a
/// setting any reference offers.
pub fn pitch_after(pitch: f32, dy_px: f32, rad_per_px: f32, invert: bool, limit: f32) -> f32 {
    let dy = if invert { -dy_px } else { dy_px };
    (pitch - dy * rad_per_px).clamp(-limit, limit)
}

/// How far the head may turn from the body while free look is held, radians.
///
/// **Ours, and not measured off the reference** (`DECISIONS.md` §open, "free
/// look v0"): the reference offers the verb and publishes no number for it,
/// and `BALANCE.md` §6 governs *sim* values rather than a client cosmetic, so
/// this is a documented default rather than a taken one. 120° is chosen to be
/// wider than a shoulder check and short of the full turn that would let the
/// camera face the body's back — at which point the player is walking
/// backwards with no way to tell.
pub const FREE_LOOK_YAW_LIMIT: f32 = 2.094_395_1;

/// The head's yaw OFFSET from the body after a mouse move, while free look
/// is held.
///
/// Subtracted for exactly [`yaw_after`]'s reason, and that shared sign is the
/// property worth gating: a free look that turned the opposite way from an
/// ordinary look would be the header's original bug back again, in a mode
/// nobody re-tests.
pub fn free_yaw_after(offset: f32, dx_px: f32, rad_per_px: f32, limit: f32) -> f32 {
    (offset - dx_px * rad_per_px).clamp(-limit, limit)
}

/// The head's pitch OFFSET from the body after a mouse move, while free look
/// is held.
///
/// Clamped against the BODY's own pitch rather than against zero, so the
/// summed view still stops short of the pole. Clamping the offset alone would
/// let a body already pitched up carry the camera past vertical, where the
/// yaw basis degenerates — [`PITCH_LIMIT`]'s whole reason.
pub fn free_pitch_after(
    offset: f32,
    body: f32,
    dy_px: f32,
    rad_per_px: f32,
    invert: bool,
    limit: f32,
) -> f32 {
    let dy = if invert { -dy_px } else { dy_px };
    (offset - dy * rad_per_px).clamp(-limit - body, limit - body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sim's own wish-direction arithmetic (`movement::step`), so the
    /// test asks what the SIM would do with our axes rather than what we
    /// hoped it would do.
    fn wish(yaw: u16, move_x: i8, move_z: i8) -> (f32, f32) {
        let (fx, fz) = yaw_dir(yaw);
        let (rx, rz) = (fz, -fx);
        let mf = move_z as f32 * (1.0 / 127.0);
        let ms = move_x as f32 * (1.0 / 127.0);
        (fx * mf + rx * ms, fz * mf + rz * ms)
    }

    /// `r × up = −f`, reduced to the 2D determinant it implies.
    ///
    /// Expanding `(rx, 0, rz) × (0, 1, 0) = (−rz, 0, rx)` and matching it to
    /// `(−fx, 0, −fz)` gives `rz = fx` and `rx = −fz`, so the cross term
    /// `fx·rz − fz·rx` comes to `fx² + fz²`, which is **+1** for a unit
    /// facing. This helper had that sign backwards on the first cut and both
    /// tests using it went red against correct code — worth the paragraph,
    /// since the whole module is about a sign.
    fn is_right_of(f: (f32, f32), r: (f32, f32)) -> bool {
        (f.0 * r.1 - f.1 * r.0 - 1.0).abs() < 1e-3
    }

    #[test]
    fn the_right_vector_is_the_right_vector() {
        for i in 0..256u32 {
            let yaw = (i << 8) as u16;
            let f = yaw_dir(yaw);
            let r = right_dir(yaw);
            assert!(is_right_of(f, r), "yaw index {i}: f {f:?} r {r:?}");
        }
    }

    /// The sim's `(fz, -fx)` is the LEFT vector. Pinned as a test rather
    /// than left in a comment, because the day it stops being true is the
    /// day `move_axes` must stop negating.
    #[test]
    fn the_sims_strafe_basis_points_left() {
        for i in 0..256u32 {
            let yaw = (i << 8) as u16;
            let (fx, fz) = yaw_dir(yaw);
            let sim_r = (fz, -fx);
            let left = (-sim_r.0, -sim_r.1);
            assert!(
                is_right_of((fx, fz), left),
                "yaw index {i}: the sim's basis stopped being the left vector"
            );
        }
    }

    #[test]
    fn d_strafes_toward_the_bodys_right() {
        for i in 0..256u32 {
            let yaw = (i << 8) as u16;
            let (mx, mz) = move_axes(0, 1);
            let w = wish(yaw, mx, mz);
            let r = right_dir(yaw);
            // Same direction, not merely non-opposite: the dot of two unit
            // vectors is 1 only when they agree.
            assert!(
                (w.0 * r.0 + w.1 * r.1 - 1.0).abs() < 1e-3,
                "yaw index {i}: D moved {w:?}, right is {r:?}"
            );
        }
    }

    #[test]
    fn w_walks_toward_the_facing() {
        for i in 0..256u32 {
            let yaw = (i << 8) as u16;
            let (mx, mz) = move_axes(1, 0);
            let w = wish(yaw, mx, mz);
            let f = yaw_dir(yaw);
            assert!((w.0 * f.0 + w.1 * f.1 - 1.0).abs() < 1e-3, "yaw index {i}");
        }
    }

    #[test]
    fn a_and_s_are_the_opposites_of_d_and_w() {
        let (dx, _) = move_axes(0, 1);
        let (ax, _) = move_axes(0, -1);
        assert_eq!(dx, -ax);
        let (_, wz) = move_axes(1, 0);
        let (_, sz) = move_axes(-1, 0);
        assert_eq!(wz, -sz);
    }

    /// Diagonals must not overflow the axis, which is what the clamp is for
    /// — a caller summing `W+W` from two bindings would otherwise send 254
    /// through an `i8` and arrive as −2.
    #[test]
    fn the_axes_are_bounded() {
        for f in -3..=3 {
            for r in -3..=3 {
                let (mx, mz) = move_axes(f, r);
                assert!(mx.unsigned_abs() <= 127 && mz.unsigned_abs() <= 127);
            }
        }
    }

    #[test]
    fn a_mouse_pushed_right_turns_the_view_right() {
        // Ten degrees of travel, at four bearings around the circle.
        for start in [0.0f32, 1.3, 3.0, 5.2] {
            let rad = 0.0022;
            let after = yaw_after(start, 100.0, rad);
            let f0 = yaw_dir(yaw_u16(start));
            let f1 = yaw_dir(yaw_u16(after));
            let r0 = right_dir(yaw_u16(start));
            // The facing moved toward the old right, not away from it.
            let d = (f1.0 - f0.0, f1.1 - f0.1);
            assert!(
                d.0 * r0.0 + d.1 * r0.1 > 0.0,
                "start {start}: the view turned the wrong way"
            );
        }
    }

    /// North is `+Z` and the card runs clockwise, so **east is `-X`** — the
    /// body's right when it faces north, which is what a compass card means
    /// by east (`DECISIONS.md` 2026-08-15).
    #[test]
    fn the_bearing_reads_clockwise_from_north() {
        // Straight down each world axis, stated as a direction rather than a
        // yaw, so this reads as the convention and not as arithmetic. North
        // is exact — see `due_north_prints_without_a_sign` for why that
        // matters; the rest carry `to_degrees`' rounding.
        assert_eq!(bearing_of(0.0, 1.0), 0.0, "+Z is north");
        for (dx, dz, want, name) in [
            (-1.0, 0.0, 90.0, "-X is east"),
            (0.0, -1.0, 180.0, "-Z is south"),
            (1.0, 0.0, 270.0, "+X is west"),
        ] {
            let got = bearing_of(dx, dz);
            assert!((got - want).abs() < 0.01, "{name}: got {got}");
        }
    }

    /// Yaw turns the view LEFT (the module header's second defect, kept as a
    /// basis fact), so the bearing a yaw reports runs the other way: it is
    /// `(360 − yaw°) mod 360`, which is the whole of what the 2026-08-15 row
    /// spent to get the card right.
    #[test]
    fn the_bearing_is_the_yaw_reflected() {
        for i in 0..64 {
            let yaw = i as f32 * 0.1;
            let want = (360.0 - yaw.to_degrees()).rem_euclid(360.0);
            let got = bearing_deg(yaw);
            // Quantized through the LUT — 256 steps, 1.40625° each — so a
            // step's worth of slack and no more.
            let d = (got - want).abs().min(360.0 - (got - want).abs());
            assert!(d < 2.0, "yaw {yaw}: bearing {got}, wanted {want}");
        }
    }

    /// **The operator's own probe, as a gate.** The reported defect was
    /// physical rather than numeric: *a player turning right watches the
    /// number fall.* A mouse pushed right must raise the bearing, all the way
    /// round, including across the wrap.
    #[test]
    fn a_mouse_pushed_right_raises_the_bearing() {
        for i in 0..48 {
            let yaw = -std::f32::consts::PI + i as f32 * 0.13;
            let before = bearing_deg(yaw);
            // 100 px right, the probe's own push — comfortably past one LUT
            // step, so the quantized number has to move.
            let after = bearing_deg(yaw_after(yaw, 100.0, MOUSE_RAD_PER_PX));
            let step = (after - before).rem_euclid(360.0);
            assert!(
                (1.0..180.0).contains(&step),
                "yaw {yaw}: {before}° → {after}° — the card ran backwards"
            );
        }
    }

    /// Due north prints `000`, never `-00`: `(-dx).atan2(dz)` is `-0.0` at
    /// yaw 0 and `rem_euclid`'s guard is `r < 0.0`, which `-0.0` slips past.
    /// Both `{:03.0}` sites (the compass strip and the map's heading) would
    /// have shown it, so it is gated on the value they both read.
    #[test]
    fn due_north_prints_without_a_sign() {
        let d = bearing_deg(0.0);
        assert!(
            d.is_sign_positive(),
            "north came back as {d} (a signed zero)"
        );
        assert_eq!(format!("{d:03.0}"), "000");
    }

    /// It never returns 360, which would print as `360°` beside a card that
    /// says `N`.
    #[test]
    fn the_bearing_wraps_rather_than_reaching_360() {
        for i in 0..64 {
            let yaw = -std::f32::consts::TAU + i as f32 * 0.2;
            let d = bearing_deg(yaw);
            assert!((0.0..360.0).contains(&d), "yaw {yaw} gave {d}");
        }
    }

    /// The bearing is quantized, so it names a heading the SERVER can face —
    /// the same discipline `yaw_u16` exists for. Two yaws inside one LUT step
    /// must report the same number.
    #[test]
    fn the_bearing_is_quantized_like_the_wire() {
        let step = std::f32::consts::TAU / 256.0;
        for i in 0..16 {
            let base = i as f32 * step;
            assert_eq!(
                bearing_deg(base + step * 0.1),
                bearing_deg(base + step * 0.4),
                "two yaws inside one LUT step disagreed",
            );
        }
    }

    #[test]
    fn a_mouse_pushed_down_lowers_the_view() {
        let limit = 1.55;
        assert!(pitch_after(0.0, 100.0, 0.0022, false, limit) < 0.0);
        assert!(pitch_after(0.0, 100.0, 0.0022, true, limit) > 0.0);
        assert!(pitch_after(0.0, 100_000.0, 0.0022, false, limit) >= -limit);
        assert!(pitch_after(0.0, -100_000.0, 0.0022, false, limit) <= limit);
    }

    /// The head turns the same way the body does. A free look that answered a
    /// rightward push by turning left would be the module header's original
    /// defect surviving inside a mode, which is the one place it could hide.
    #[test]
    fn free_look_turns_the_same_way_an_ordinary_look_does() {
        let rad = MOUSE_RAD_PER_PX;
        for dx in [30.0f32, 120.0, -75.0] {
            let ordinary = yaw_after(0.0, dx, rad);
            let free = free_yaw_after(0.0, dx, rad, FREE_LOOK_YAW_LIMIT);
            assert_eq!(
                ordinary.signum(),
                free.signum(),
                "dx {dx}: the head and the body disagree about right",
            );
            assert!(
                (ordinary - free).abs() < 1e-6,
                "dx {dx}: same push, different travel"
            );
        }
    }

    #[test]
    fn the_head_cannot_turn_past_its_limit() {
        let rad = MOUSE_RAD_PER_PX;
        let lim = FREE_LOOK_YAW_LIMIT;
        // Push far enough to lap the circle several times, both ways.
        assert!(free_yaw_after(0.0, 100_000.0, rad, lim) >= -lim);
        assert!(free_yaw_after(0.0, -100_000.0, rad, lim) <= lim);
        // And it saturates rather than wrapping — a head that wrapped would
        // face the body's back with the player still walking forward.
        assert!((free_yaw_after(0.0, -100_000.0, rad, lim) - lim).abs() < 1e-6);
    }

    /// The offset is clamped against the BODY's pitch, so the summed view
    /// stops at the same pole an ordinary look does — whatever the body was
    /// already doing when free look started.
    #[test]
    fn the_summed_free_pitch_never_passes_the_pole() {
        let rad = MOUSE_RAD_PER_PX;
        for body in [-1.4f32, -0.5, 0.0, 0.5, 1.4] {
            for dy in [100_000.0f32, -100_000.0] {
                let off = free_pitch_after(0.0, body, dy, rad, false, PITCH_LIMIT);
                let total = body + off;
                assert!(
                    (-PITCH_LIMIT - 1e-4..=PITCH_LIMIT + 1e-4).contains(&total),
                    "body {body}, dy {dy}: view reached {total}",
                );
            }
        }
    }
}
