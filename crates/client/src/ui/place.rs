//! Where a placement would go, and whether the sim would take it.
//!
//! The build wheel latched a piece and could not place one: `NOW.md` §0w
//! item 1 — *"nothing draws the cell being aimed at or colours it by whether
//! the sim would accept it, and `encode_action_place` needs a cell, a level
//! and a location this client cannot aim."* This is that aiming, and the
//! acceptance guess that colours the ghost.
//!
//! Pure, in `ui/`, gated in the code tier — the usual reason, plus one
//! specific to this file: **the anchor is a positional payload**. The reach
//! both `place` and `repair` gate on is measured to a corner of the cell that
//! depends on `loc`, and swapping two terms leaves every byte-golden green
//! while the client refuses at a distance the server would accept. So the
//! anchor is not reimplemented here at all — `sim_core::build::anchor` is
//! called directly, which is why that function stopped being `pub(crate)` in
//! the same commit.
//!
//! ## What the verdict is, and what it is not
//!
//! [`verdict`] is a **local guess that saves a round trip**, never an
//! authority. It answers the four refusals a client can see for itself —
//! the spot is taken, out of reach, cannot afford it, the ground will not
//! hold a foundation — and says nothing about support, hearth claims or
//! world capacity, which are the server's alone. A ghost that draws green
//! and then refuses is a ghost that was honest about what it knew; a ghost
//! that draws red on something the sim would have taken is the failure to
//! avoid, so every check here is one the sim runs the same way.

use sim_core::build::{
    anchor, build_cell_of, foundation_terrain_ok, plate_for, BuildContent, PieceRec, BUILD_CELL_M,
    BUILD_REACH_M, LOC_DIAG_A, LOC_DIAG_B, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE, LOC_RISER,
    LOC_TRI_XHI_ZHI, LOC_TRI_XHI_ZLO, LOC_TRI_XLO_ZHI, LOC_TRI_XLO_ZLO, SHAPE_DOORWAY,
    SHAPE_FOUNDATION, SHAPE_FRAME, SHAPE_STAIRS, SHAPE_TRI_FLOOR, SHAPE_TRI_FOUNDATION,
    SHAPE_TRI_ROOF, SHAPE_WALL, SHAPE_WINDOW,
};
use sim_core::craft::inv_count;
use sim_core::deploy::{
    box_key, cell_center, loc_fits_placement, lockable, DeployContent, DeployRec, ARCH_BOX,
    ARCH_LOCK, PLACE_ANY, PLACE_DOOR, PLACE_DOORWAY, PLACE_FOUNDATION, PLACE_GROUND,
};
use sim_core::gather::ItemStack;
use sim_core::limits::{INV_SLOTS, MAX_BUILD_COORD, MAX_BUILD_LEVELS};

use super::build::affordable;

/// How far ahead of the feet the ghost is aimed when the LOOK ray finds
/// nothing to land on (sky, sea, past range), metres.
///
/// PROPOSED — `DECISIONS.md` §open ("build aim distance v0"), carried over
/// from `web/src/main.js`'s `buildTarget`. Comfortably inside
/// `BUILD_REACH_M` (5 m) so an aimed cell is one the sim will accept on
/// reach, with room for the anchor being up to half a cell further than the
/// centre.
///
/// Until 2026-08-15 this was the ONLY aim: the ghost sat at a fixed 3.5 m
/// from the feet along the yaw, pitch ignored, so the crosshair pointed at
/// one cell and the ghost stood in another — the player in that day's
/// playtest was reading `SPOT TAKEN` off a ghost parked behind the gap
/// they were looking at. [`aim_from_look`] is the aim now; this is its
/// fallback.
pub const AIM_AHEAD_M: f32 = 3.5;

/// The look-ray march step, metres. At 0.25 the resolved point is well
/// under a tenth of a cell off the true intersection, and the march is at
/// most 32 terrain samples on the frame path — the same order the movement
/// step already pays. Proposed default, `DECISIONS.md` §open ("build base
/// lattice v0").
pub const AIM_STEP_M: f32 = 0.25;

/// How far past build reach the march bothers looking before falling back,
/// metres. Same §open row.
pub const AIM_RANGE_M: f32 = BUILD_REACH_M + 3.0;

/// Where the LOOK ray meets the world, as a planar aim point for
/// [`target_at`] — clamped to [`BUILD_REACH_M`] around the feet so the
/// resolved cell is one the player could plausibly place in.
///
/// The march tests the ray against the same two grounds the sim stands
/// players on: raw terrain, and the piece surfaces in the predictor's own
/// collision index (`ClientCore::cols`) via `collide::piece_ground` — so
/// aiming at your own floor extends the floor, at the working level or
/// not. What it deliberately does NOT test is wall faces: a ray into a
/// wall lands on the ground behind it, and the verdict says what the sim
/// would say about that cell. `eye`/`dir` are the camera's own (the
/// tracer's ray convention, so the ghost agrees with where a shot goes);
/// pure and bevy-free so the whole aim is testable headless.
pub fn aim_from_look(
    seed: u64,
    haven: &sim_core::terrain::Haven,
    cols: &sim_core::collide::ColIndex,
    eye: [f32; 3],
    dir: [f32; 3],
    feet: (f32, f32),
) -> (f32, f32) {
    let mut t = AIM_STEP_M;
    while t <= AIM_RANGE_M {
        let px = eye[0] + dir[0] * t;
        let py = eye[1] + dir[1] * t;
        let pz = eye[2] + dir[2] * t;
        let ground = sim_core::terrain::ground(seed, haven, px, pz).max(
            sim_core::collide::piece_ground(seed, haven, cols, px, pz, py),
        );
        if py <= ground {
            return clamp_to_reach(feet, (px, pz));
        }
        t += AIM_STEP_M;
    }
    let planar = (dir[0] * dir[0] + dir[2] * dir[2]).sqrt().max(1e-3);
    clamp_to_reach(
        feet,
        (
            feet.0 + dir[0] / planar * AIM_AHEAD_M,
            feet.1 + dir[2] / planar * AIM_AHEAD_M,
        ),
    )
}

/// Pull an aim point back onto the reach circle around the feet. The ghost
/// never parks past what the sim could accept; at the rim the verdict
/// still speaks (an anchor can sit past the point that resolved it).
fn clamp_to_reach(feet: (f32, f32), p: (f32, f32)) -> (f32, f32) {
    let (dx, dz) = (p.0 - feet.0, p.1 - feet.1);
    let d2 = dx * dx + dz * dz;
    if d2 <= BUILD_REACH_M * BUILD_REACH_M {
        return p;
    }
    let s = BUILD_REACH_M / d2.sqrt();
    (feet.0 + dx * s, feet.1 + dz * s)
}

/// A resolved build address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub loc: u8,
}

/// Which cell, level and location a shape would occupy, aimed from the feet
/// along the yaw at the fixed fallback distance. The projection half of
/// [`target_at`], kept because a caller with no look ray (and the tests
/// that pin the addressing rules) still wants the old behaviour.
pub fn target(x: f32, z: f32, fx: f32, fz: f32, shape: u8, level: u8) -> Target {
    target_at(x + fx * AIM_AHEAD_M, z + fz * AIM_AHEAD_M, shape, level)
}

/// Which cell, level and location a shape would occupy for an aim POINT —
/// wherever that point came from (the look-ray march, or [`target`]'s
/// fixed projection).
///
/// The `loc` rule is the grid's, not a preference: **edge pieces are
/// canonical to a cell's low-x or low-z boundary** (`build.rs`), so the same
/// physical edge is never addressable twice. Aiming at the +x edge of cell
/// N means the low-x edge of cell N+1, which is what the `cx += 1` arm below
/// is doing — it is a re-address, not an off-by-one.
///
/// A foundation is pinned to level 0 whatever the working level says: it is
/// the piece that stands on the ground by definition, and letting the level
/// stepper lift one is how a player spends materials on a refusal.
pub fn target_at(ax: f32, az: f32, shape: u8, level: u8) -> Target {
    let max = (MAX_BUILD_COORD - 1) as i32;
    let mut cx = build_cell_of(ax).clamp(0, max);
    let mut cz = build_cell_of(az).clamp(0, max);

    let mut loc = LOC_PLANE;
    if matches!(
        shape,
        SHAPE_WALL | SHAPE_DOORWAY | SHAPE_WINDOW | SHAPE_FRAME
    ) {
        // Which boundary of this cell the aim point is nearest — and for
        // the WALL alone, the two diagonals compete on the same terms
        // (triangles v0): their perpendicular distances, in the same
        // cell-fraction units, so aiming at the middle of a cell reaches
        // for a diagonal and aiming at a boundary reaches for it exactly
        // as before. The √2 is the projection, precomputed.
        let fxc = ax / BUILD_CELL_M - cx as f32;
        let fzc = az / BUILD_CELL_M - cz as f32;
        let m = fxc.min(1.0 - fxc).min(fzc).min(1.0 - fzc);
        if m == fxc {
            loc = LOC_EDGE_XLO;
        } else if m == 1.0 - fxc {
            cx = (cx + 1).min(max);
            loc = LOC_EDGE_XLO;
        } else if m == fzc {
            loc = LOC_EDGE_ZLO;
        } else {
            cz = (cz + 1).min(max);
            loc = LOC_EDGE_ZLO;
        }
        if shape == SHAPE_WALL {
            const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;
            let da = (fzc - fxc).abs() * INV_SQRT2;
            let db = (fxc + fzc - 1.0).abs() * INV_SQRT2;
            if da < m && da <= db {
                loc = LOC_DIAG_A;
                cx = build_cell_of(ax).clamp(0, max);
                cz = build_cell_of(az).clamp(0, max);
            } else if db < m {
                loc = LOC_DIAG_B;
                cx = build_cell_of(ax).clamp(0, max);
                cz = build_cell_of(az).clamp(0, max);
            }
        }
    } else if matches!(
        shape,
        SHAPE_TRI_FOUNDATION | SHAPE_TRI_FLOOR | SHAPE_TRI_ROOF
    ) {
        // The half whose centroid is nearest the aim point — the anchors
        // are the sim's own (`build::anchor` at thirds), so the ghost's
        // pick and the reach the server measures agree by construction.
        let mut best = f32::MAX;
        for cand in [
            LOC_TRI_XLO_ZLO,
            LOC_TRI_XHI_ZLO,
            LOC_TRI_XLO_ZHI,
            LOC_TRI_XHI_ZHI,
        ] {
            let (tx, tz) = anchor(cx as u16, cz as u16, cand);
            let d2 = (ax - tx) * (ax - tx) + (az - tz) * (az - tz);
            if d2 < best {
                best = d2;
                loc = cand;
            }
        }
    } else if shape == SHAPE_STAIRS {
        loc = LOC_RISER;
    }

    Target {
        cx: cx as u16,
        cz: cz as u16,
        level: if shape == SHAPE_FOUNDATION {
            0
        } else {
            level.min(MAX_BUILD_LEVELS as u8 - 1)
        },
        loc,
    }
}

/// What the client can tell about a placement before sending it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing this client can see refuses it.
    Ok,
    /// One of the four the client can check. The string is the same sentence
    /// the server's refusal would carry (`ui::refusals`), so a ghost that
    /// guessed and a refusal that arrived read identically.
    No(&'static str),
}

/// A verdict nobody has computed yet is a refusal with nothing to say —
/// **not** `Ok`. A `Default` that meant "the sim would take this" would draw
/// a green ghost on the frame before anything had been checked.
impl Default for Verdict {
    fn default() -> Self {
        Verdict::No("")
    }
}

impl Verdict {
    pub fn ok(self) -> bool {
        self == Verdict::Ok
    }

    pub fn why(self) -> &'static str {
        match self {
            Verdict::Ok => "",
            Verdict::No(s) => s,
        }
    }
}

/// Everything the local pre-check reads.
pub struct Site<'a> {
    pub seed: u64,
    /// The solved authored sites, for `terrain::ground`. Beside `seed`
    /// because the pair is what names an island's *carved* surface, and the
    /// ghost has to predict against the same surface `build::place` will
    /// validate on — one of them reading raw terrain is a ghost that says
    /// yes where the server says no.
    pub haven: &'a sim_core::terrain::Haven,
    /// The player's feet, world XZ.
    pub at: (f32, f32),
    /// Addresses already holding a piece.
    pub taken: &'a [sim_core::build::PieceRec],
    /// The predictor's column index — the client's own mirror of the store
    /// `build::place` reads. Here for the plate (build plate v1): the height
    /// a placement takes is no longer a function of (seed, cell), so the
    /// ghost has to ask the same question the sim will.
    pub cols: &'a sim_core::collide::ColIndex,
    pub content: &'a BuildContent,
    pub inv: &'a [ItemStack; INV_SLOTS],
}

/// The four refusals a client can see for itself. See the module header for
/// what this deliberately does not answer.
pub fn verdict(t: Target, row: u16, shape: u8, site: &Site<'_>, freehand: bool) -> Verdict {
    // Spot taken. `deploy.rs` and `build.rs` both key on the full address, so
    // this is the same comparison the sim's `find` makes.
    if site
        .taken
        .iter()
        .any(|r| r.cx == t.cx && r.cz == t.cz && r.level == t.level && r.loc == t.loc)
    {
        return Verdict::No("spot taken");
    }

    // Reach, measured to the ANCHOR — the sim's own corner, via the sim's own
    // function. See the header.
    let (ax, az) = anchor(t.cx, t.cz, t.loc);
    let (dx, dz) = (ax - site.at.0, az - site.at.1);
    if dx * dx + dz * dz > BUILD_REACH_M * BUILD_REACH_M {
        return Verdict::No("out of reach");
    }

    // Ground, for a foundation only — every other shape stands on structure
    // and the sim asks a different question of it.
    if shape == SHAPE_FOUNDATION && !foundation_terrain_ok(site.seed, site.haven, ax, az) {
        return Verdict::No("bad ground");
    }

    // The plate (build plate v1), by the sim's own rule against the client's
    // own mirror — after ground and before cost, which is `build::place`'s
    // own order, so the sentence the ghost shows is the one the server would
    // have said and not a later one.
    //
    // **Mirrored rather than left Unknown**, unlike the claim: the two plate
    // refusals are computed from the piece index and the terrain, and the
    // client holds both. Leaving them out would put a green ghost on the
    // commonest refusal a hillside can produce.
    if let Err(why) = plate_for(site.cols, site.seed, site.haven, t.cx, t.cz, freehand) {
        // Indexed rather than formatted: `Verdict::No` carries a
        // `&'static str`, and the table row IS that string — the same one
        // the server's refusal would arrive with (`DeployVerdict`'s
        // discipline, one table over). `plate_for` returns only these two
        // codes, so an out-of-range index here is a loud bug and not a
        // case to clamp away.
        return Verdict::No(super::refusals::BUILD[why as usize]);
    }

    if !affordable(site.content, row, site.inv) {
        return Verdict::No("missing materials");
    }

    Verdict::Ok
}

/// The plate a placement at `t` would take, or 0 where the rule refuses —
/// what the GHOST draws at.
///
/// A refusal still has to draw something: the ghost's whole job on a hillside
/// is to show you the base is too high or too low before you press, and a
/// hidden ghost says nothing. Zero is the column's own ground, which is where
/// an unlatched foundation would stand — so a red ghost on `too far below the
/// floor` sits on the terrain under it with the base it could not reach
/// visibly above, which is the picture that explains the refusal.
pub fn ghost_plate(site: &Site<'_>, t: Target, freehand: bool) -> i8 {
    plate_for(site.cols, site.seed, site.haven, t.cx, t.cz, freehand).unwrap_or(0)
}

/// How near the shared edge with a BUILT neighbour the aim has to fall for
/// the placement to take that neighbour's floor — the snap band, and the
/// whole user interface of freehand placement v0 (`DECISIONS.md` §open).
///
/// **The reference has no freehand key, and that is the finding this is built
/// on** (2026-08-22, `reference/BUILDING.md` §7c.3). Placement there is
/// continuous: a piece is ATTRACTED to a socket when you aim near one, and
/// you get a freehand piece by aiming where no socket catches it. There is no
/// button to hold, which is exactly why their own guides call the technique
/// "tricky and non-intuitive" and teach it with the logs on a twig foundation
/// and the compass tics as visual guides.
///
/// That does not port as-is, because **ours is address-based**: `Place`
/// carries a cell, so there is no "near" — a cell is either the neighbour's
/// or the next one over, and `plate_for`'s latch fires on exact adjacency.
/// What survives is that the continuous aim point is still there:
/// [`aim_from_look`] marches the look ray to a real `(f32, f32)` and
/// [`target_at`] quantizes it away. The sub-cell remainder is the freehand
/// input the model was missing, so the bit is aimed rather than typed and the
/// ghost shows it — which is the operator's own memory of the reference
/// (2026-08-22) expressed against a grid.
///
/// **Two thirds of a cell, so snapping is what HAPPENS and freehand is what
/// you DO.** A base that is one plate is the common case build plate v1
/// landed for, and a placement declining it is advanced tech; the default has
/// to favour the first. At `BUILD_CELL_M` 3 m the near 2 m snap and the far
/// 1 m is freehand, so a player not thinking about it never leaves the plate
/// and a player who is aims at the far edge and watches the ghost drop.
/// Derived rather than typed, so a cell resize moves both — which is also
/// why the KNOB is the fraction and the metres are arithmetic off it
/// (`DECISIONS.md` §open pins `SNAP_BAND_FRAC`; a registry cannot pin a
/// constant whose initializer is an expression, and making the metres the
/// knob would have meant typing 2.0 and letting it drift from the cell).
pub const SNAP_BAND_FRAC: f32 = 2.0 / 3.0;

/// [`SNAP_BAND_FRAC`] in metres, against the cell it is a fraction of.
pub const SNAP_BAND_M: f32 = BUILD_CELL_M * SNAP_BAND_FRAC;

/// Whether a placement at `t` aimed at `aim` declines the plate latch.
///
/// True when the aim lands further than [`SNAP_BAND_M`] from the shared edge
/// of EVERY built orthogonal neighbour — you did not reach toward the base,
/// so you get your own ground. False when no neighbour is built at all,
/// because there is then no latch to decline and a bit flipping with the
/// crosshair over open ground would be noise on the wire and in the replay.
///
/// **A cell wedged between two built columns can never go freehand**: the
/// band is two thirds measured from each side, so the two overlap and no
/// point in the cell clears both. Deliberate — an interior cell of somebody's
/// base is the one place a second floor height has nothing to mean.
///
/// Pure, and mirrors nothing the sim computes: the server cannot re-derive
/// this because it never sees the aim, which is why the bit crosses the wire
/// (`protocol::ActionMsg::Place`).
pub fn freehand_from_aim(site: &Site<'_>, t: Target, aim: (f32, f32)) -> bool {
    let x0 = t.cx as f32 * BUILD_CELL_M;
    let z0 = t.cz as f32 * BUILD_CELL_M;
    // Each entry is a neighbour address and the aim's distance to the edge
    // it shares with this cell. Same checked arithmetic and same range guard
    // as `plate_for`'s own scan, because this runs on an address a look ray
    // produced and `u16::MAX + 1` is a debug panic on a path a player aims at.
    let edges = [
        (t.cx.checked_sub(1), Some(t.cz), aim.0 - x0),
        (t.cx.checked_add(1), Some(t.cz), x0 + BUILD_CELL_M - aim.0),
        (Some(t.cx), t.cz.checked_sub(1), aim.1 - z0),
        (Some(t.cx), t.cz.checked_add(1), z0 + BUILD_CELL_M - aim.1),
    ];
    let mut latchable = false;
    for (nx, nz, d) in edges {
        let (Some(nx), Some(nz)) = (nx, nz) else {
            continue;
        };
        if (nx as usize) >= MAX_BUILD_COORD || (nz as usize) >= MAX_BUILD_COORD {
            continue;
        }
        if site.cols.plate(nx, nz).is_none() {
            continue;
        }
        latchable = true;
        if d <= SNAP_BAND_M {
            return false;
        }
    }
    latchable
}

// ---------------------------------------------------------------------------
// The deploy half — WHETHER, not just WHERE (`NOW.md` §0u item 2)
// ---------------------------------------------------------------------------

/// What the client can tell about a deploy placement before sending it.
///
/// Two states, not three, and the missing `Ok` is the point: `REFUSE_D_CLAIM`
/// needs the hearth crew lists and the claim walk over the sim's own stores,
/// and neither is client-visible (the wire's deploy record carries no owner
/// and no crew — `protocol::event::write_deploy_rec`). So "nothing this
/// client can check refuses it" can never be promoted to "the sim will take
/// it", and a variant that said so would be a green ghost on a claim refusal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeployVerdict {
    /// One of the refusals the client can mirror, computed by the sim's own
    /// predicates on the client's own mirror. The sentence is the refusal
    /// table's row for the sim's constant (`ui::refusals::DEPLOY`), so the
    /// guess and the refusal that would follow it read identically.
    ///
    /// A red is always a true refusal — every check here is one the sim runs
    /// the same way — but not always the FIRST sentence the server would say:
    /// the sim refuses a foreign claim before support, and the claim is the
    /// one rung this client cannot see.
    No(&'static str),
    /// Nothing this client can check refuses it — **not** "the sim would
    /// take it" (see the type doc). Also the state nobody has computed yet:
    /// the default that coloured would be a verdict nobody earned.
    #[default]
    Unknown,
}

impl DeployVerdict {
    pub fn refused(self) -> bool {
        matches!(self, DeployVerdict::No(_))
    }

    pub fn why(self) -> &'static str {
        match self {
            DeployVerdict::No(s) => s,
            DeployVerdict::Unknown => "",
        }
    }
}

/// Which address the held deployable is aimed at, from its placement class.
///
/// A doorway-class deployable (the door) resolves an EDGE the way a wall
/// does — `place_deploy` requires the doorway piece at the identical
/// address, so aiming it at the cell body could only ever refuse
/// (`loc_fits_placement`). Everything else stands on the cell body at
/// level 0, which is `deploy_key`'s original plane-shape target. `level`
/// is the ghost's working-level latch and only the doorway class reads it:
/// a ground/foundation body deploy is sent at level 0 exactly as before.
pub fn deploy_target(x: f32, z: f32, fx: f32, fz: f32, placement: u8, level: u8) -> Target {
    if placement == PLACE_DOORWAY {
        target(x, z, fx, fz, SHAPE_DOORWAY, level)
    } else {
        target(x, z, fx, fz, SHAPE_FOUNDATION, 0)
    }
}

/// [`deploy_target`] for an aim POINT — the look-ray variant, split the
/// way [`target_at`] is and for the same caller.
pub fn deploy_target_at(ax: f32, az: f32, placement: u8, level: u8) -> Target {
    if placement == PLACE_DOORWAY {
        target_at(ax, az, SHAPE_DOORWAY, level)
    } else {
        target_at(ax, az, SHAPE_FOUNDATION, 0)
    }
}

/// Everything the deploy pre-check reads — the client's mirror, whole.
pub struct DeploySite<'a> {
    pub seed: u64,
    /// The solved authored sites — see `Site::haven`, same reason.
    pub haven: &'a sim_core::terrain::Haven,
    /// The player's feet, world XZ.
    pub at: (f32, f32),
    /// The placed-piece mirror (support lives here).
    pub pieces: &'a [PieceRec],
    pub piece_defs: &'a BuildContent,
    /// Rows of `piece_defs` that have dripped in.
    pub piece_have: u16,
    /// The placed-deployable mirror (occupancy and lock targets live here).
    pub deploys: &'a [DeployRec],
    pub deploy_defs: &'a DeployContent,
    /// Rows of `deploy_defs` that have dripped in.
    pub deploy_have: u16,
    pub inv: &'a [ItemStack; INV_SLOTS],
}

impl DeploySite<'_> {
    fn deploy_at(&self, cx: u16, cz: u16, level: u8, loc: u8) -> Option<&DeployRec> {
        self.deploys
            .iter()
            .find(|r| r.cx == cx && r.cz == cz && r.level == level && r.loc == loc)
    }

    fn piece_at(&self, cx: u16, cz: u16, level: u8, loc: u8) -> Option<&PieceRec> {
        self.pieces
            .iter()
            .find(|r| r.cx == cx && r.cz == cz && r.level == level && r.loc == loc)
    }
}

/// The deploy refusals a client can see for itself, in `place_deploy`'s own
/// order, each computed the way the sim computes it.
///
/// ## The split, stated (the module's honesty rule)
///
/// **Mirrored** — a pure function of state the mirror holds, answered by the
/// sim's own functions (`loc_fits_placement`, `box_key`, `cell_center`,
/// `foundation_terrain_ok`, `lockable`, `lock::holds`, `inv_count`) plus the
/// address lookups the mirror exists for: `REFUSE_D_KIND`, `REFUSE_D_SPOT`,
/// `REFUSE_D_REACH`, `REFUSE_D_SUPPORT`, `REFUSE_D_TERRAIN`,
/// `REFUSE_D_HAS_LOCK`, `REFUSE_D_COST`.
///
/// **Unknown, and why** — the verdict stays [`DeployVerdict::Unknown`] and
/// the ghost stays neutral for: `REFUSE_D_CLAIM` (needs every hearth's crew
/// list, which the wire never carries) and `REFUSE_D_OVERLAP` (the claim
/// walk is `claim::reach` over the sim's own `Pieces` store — calling it on
/// the mirror would mean either duplicating the flood fill here or reshaping
/// sim code, both forbidden); `REFUSE_D_BAG_CAP` (counts bags by owner, and
/// the wire's deploy record carries no owner); `REFUSE_D_FULL` (the server's
/// store lengths — the same "world capacity is the server's alone" the build
/// verdict declares). A support answer that hangs on an undripped def row is
/// Unknown too, for the reason `structures::stream` skips those rows.
pub fn deploy_verdict(t: Target, row: u8, site: &DeploySite<'_>) -> DeployVerdict {
    // KIND. A row past the declared table is refused outright; a declared
    // row that has not dripped in yet is one this client cannot judge.
    if (row as u16) >= site.deploy_defs.def_count {
        return DeployVerdict::No("no such deployable");
    }
    let have = site.deploy_have.min(site.deploy_defs.def_count);
    if (row as u16) >= have {
        return DeployVerdict::Unknown;
    }
    let def = &site.deploy_defs.defs[row as usize];
    if def.hp == 0 {
        return DeployVerdict::No("no such deployable");
    }

    // SPOT. The loc rule is the sim's own function; occupancy is the same
    // address comparison its `find` makes (every class but the lock wants
    // the address empty); the reserved box address is the sim's own key.
    if !loc_fits_placement(def.placement, t.loc)
        || (def.placement != PLACE_DOOR && site.deploy_at(t.cx, t.cz, t.level, t.loc).is_some())
        || (def.arch == ARCH_BOX && box_key(t.cx, t.cz, t.level) == 0)
    {
        return DeployVerdict::No("spot taken");
    }

    // REACH, measured to the sim's own point via the sim's own function —
    // the cell CENTRE for every loc, where build reach uses the anchor.
    let (ax, az) = cell_center(t.cx, t.cz);
    let (dx, dz) = (ax - site.at.0, az - site.at.1);
    if dx * dx + dz * dz > BUILD_REACH_M * BUILD_REACH_M {
        return DeployVerdict::No("out of reach");
    }

    // REFUSE_D_CLAIM sits here in the sim's ladder and is Unknown (module
    // doc) — so every red below is still a true refusal, just possibly not
    // the first sentence the server would say.

    // SUPPORT / TERRAIN, per placement class — the sim's `supported` match,
    // its store lookups answered by the mirror.
    let ground_ok = site.piece_at(t.cx, t.cz, 0, LOC_PLANE).is_none()
        && foundation_terrain_ok(site.seed, site.haven, ax, az);
    match def.placement {
        PLACE_GROUND => {
            if t.level != 0 {
                return DeployVerdict::No("needs support");
            }
            if !ground_ok {
                return DeployVerdict::No("bad ground");
            }
        }
        PLACE_FOUNDATION => {
            if site.piece_at(t.cx, t.cz, t.level, LOC_PLANE).is_none() {
                return DeployVerdict::No("needs support");
            }
        }
        PLACE_ANY => {
            if site.piece_at(t.cx, t.cz, t.level, LOC_PLANE).is_none()
                && !(t.level == 0 && ground_ok)
            {
                return DeployVerdict::No("needs support");
            }
        }
        PLACE_DOORWAY => match site.piece_at(t.cx, t.cz, t.level, t.loc) {
            None => return DeployVerdict::No("needs support"),
            Some(r) => {
                if (r.row as u16) >= site.piece_have.min(site.piece_defs.piece_count) {
                    return DeployVerdict::Unknown; // shape not dripped yet
                }
                if site.piece_defs.pieces[r.row as usize].shape != SHAPE_DOORWAY {
                    return DeployVerdict::No("needs support");
                }
            }
        },
        PLACE_DOOR => match site.deploy_at(t.cx, t.cz, t.level, t.loc) {
            None => return DeployVerdict::No("needs support"),
            Some(r) => {
                if (r.row as u16) >= have {
                    return DeployVerdict::Unknown; // target's arch not dripped
                }
                if !lockable(site.deploy_defs.defs[r.row as usize].arch) {
                    return DeployVerdict::No("needs support");
                }
            }
        },
        // The sim's `_ => false` arm: an unknown class never stands.
        _ => return DeployVerdict::No("needs support"),
    }

    // The lock's extra rungs, in the sim's order. Its store is not
    // mirrored, but the `has_lock` bit on the record it bolts to is kept in
    // lockstep by every lock verb — that bit IS the wire's view of the
    // store. `MAX_LOCKS` (REFUSE_D_FULL) stays Unknown.
    if def.arch == ARCH_LOCK {
        if site
            .deploy_at(t.cx, t.cz, t.level, t.loc)
            .is_some_and(|r| r.has_lock)
        {
            return DeployVerdict::No("that door already has a lock");
        }
        if !sim_core::lock::holds(site.inv, def.item) {
            return DeployVerdict::No("item not in inventory");
        }
        return DeployVerdict::Unknown;
    }

    // REFUSE_D_OVERLAP, the hearth cap, the bag cap and the box-store cap
    // sit here in the sim's ladder and are Unknown (module doc).

    // COST: the deployable's own item, counted the way the sim counts it.
    if inv_count(site.inv, def.item) < 1 {
        return DeployVerdict::No("item not in inventory");
    }

    DeployVerdict::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::build::{PieceDef, PieceRec, BUILD_CELL_M, MAT_WOOD};

    fn empty_inv() -> [ItemStack; INV_SLOTS] {
        [ItemStack::default(); INV_SLOTS]
    }

    /// A table with one free piece at row 0, so `affordable` is not the thing
    /// under test unless a test means it to be.
    fn free_table(shape: u8) -> BuildContent {
        let mut c = BuildContent::EMPTY;
        c.pieces[0] = PieceDef {
            shape,
            material: MAT_WOOD,
            hp: 100,
            n_costs: 0,
            costs: [(0, 0); sim_core::limits::MAX_PIECE_COSTS],
        };
        c.piece_count = 1;
        c
    }

    #[test]
    fn a_plane_lands_in_the_cell_the_aim_falls_in() {
        // Facing +Z from the origin: 3.5 m ahead is inside cell z = 1.
        let t = target(1.5, 1.5, 0.0, 1.0, SHAPE_FOUNDATION, 0);
        assert_eq!(t.loc, LOC_PLANE);
        assert_eq!(t.cz, ((1.5 + AIM_AHEAD_M) / BUILD_CELL_M) as u16);
    }

    /// The re-address, and the reason the grid has it: the +x edge of one
    /// cell IS the low-x edge of the next, and only one of the two names it.
    #[test]
    fn a_plus_x_edge_is_addressed_as_the_next_cells_low_x_edge() {
        // Stand just below a cell boundary in x and aim +x across it.
        let boundary = 6.0f32; // cell 2's low-x edge
        let t = target(boundary - AIM_AHEAD_M + 0.1, 1.5, 1.0, 0.0, SHAPE_WALL, 0);
        assert_eq!(t.loc, LOC_EDGE_XLO);
        // Whatever cell it picked, the anchor must sit on a cell boundary in
        // x — which is what LOC_EDGE_XLO means.
        let (ax, _) = anchor(t.cx, t.cz, t.loc);
        assert!(
            (ax / BUILD_CELL_M).fract().abs() < 1e-4,
            "anchor {ax} is not on a boundary"
        );
    }

    #[test]
    fn a_wall_always_lands_on_a_canonical_edge_or_a_diagonal() {
        // Sweep the circle; every wall address must be canonical — a west
        // or north edge, or (triangles v0) one of the cell's own two
        // diagonals, which are unshared by construction. What must never
        // appear is an east/south alias of a neighbour's edge.
        for i in 0..64 {
            let a = i as f32 * std::f32::consts::TAU / 64.0;
            let t = target(40.0, 40.0, a.sin(), a.cos(), SHAPE_WALL, 2);
            assert!(
                matches!(t.loc, LOC_EDGE_XLO | LOC_EDGE_ZLO | LOC_DIAG_A | LOC_DIAG_B),
                "bearing {i} gave loc {}",
                t.loc
            );
        }
        // Aimed at a cell's middle, the wall reaches for a diagonal —
        // the aim point (10.5, 10.4) sits a hair off cell (3, 3)'s centre,
        // far from every boundary and nearly on diagonal A's line.
        let t = target(10.5 - AIM_AHEAD_M, 10.4, 1.0, 0.0, SHAPE_WALL, 0);
        assert!(
            t.loc == LOC_DIAG_A || t.loc == LOC_DIAG_B,
            "mid-cell aim gave loc {}",
            t.loc
        );
        // Aimed at a boundary, the straight edge still wins.
        let t = target(9.05 - AIM_AHEAD_M, 10.5, 1.0, 0.0, SHAPE_WALL, 0);
        assert_eq!(
            t.loc, LOC_EDGE_XLO,
            "boundary aim must stay a straight edge"
        );
    }

    #[test]
    fn a_triangle_lands_on_the_half_the_aim_is_inside() {
        // Aim near each quarter of cell (3, 3) (metres 9..12): the pick is
        // the half whose centroid is nearest, which contains the aim.
        let cases = [
            ((10.0, 10.0), LOC_TRI_XLO_ZLO),
            ((11.0, 10.0), LOC_TRI_XHI_ZLO),
            ((10.0, 11.0), LOC_TRI_XLO_ZHI),
            ((11.0, 11.0), LOC_TRI_XHI_ZHI),
        ];
        for ((ax, az), want) in cases {
            let t = target(ax - AIM_AHEAD_M, az, 1.0, 0.0, SHAPE_TRI_FOUNDATION, 0);
            assert_eq!(
                t.loc, want,
                "aim ({ax}, {az}) picked loc {} not {want}",
                t.loc
            );
            assert_eq!((t.cx, t.cz), (3, 3));
        }
    }

    #[test]
    fn stairs_take_the_riser_and_planes_take_the_plane() {
        assert_eq!(target(9.0, 9.0, 0.0, 1.0, SHAPE_STAIRS, 1).loc, LOC_RISER);
        assert_eq!(
            target(9.0, 9.0, 0.0, 1.0, SHAPE_FOUNDATION, 1).loc,
            LOC_PLANE
        );
    }

    /// A foundation is the piece that stands on the ground. Letting the level
    /// stepper lift one is how a player spends materials on a refusal.
    #[test]
    fn a_foundation_ignores_the_working_level() {
        assert_eq!(target(9.0, 9.0, 0.0, 1.0, SHAPE_FOUNDATION, 5).level, 0);
        assert_eq!(target(9.0, 9.0, 0.0, 1.0, SHAPE_WALL, 5).level, 5);
    }

    /// Aiming off the island must not wrap into a cell that names somewhere
    /// real — the `box_key` shift trap, in the other place this client turns
    /// a coordinate into an address.
    #[test]
    fn the_grid_is_clamped_at_both_ends() {
        let t = target(0.0, 0.0, -1.0, -1.0, SHAPE_FOUNDATION, 0);
        assert_eq!((t.cx, t.cz), (0, 0));
        let far = MAX_BUILD_COORD as f32 * BUILD_CELL_M * 2.0;
        let t = target(far, far, 1.0, 1.0, SHAPE_FOUNDATION, 0);
        assert!((t.cx as usize) < MAX_BUILD_COORD && (t.cz as usize) < MAX_BUILD_COORD);
    }

    #[test]
    fn the_level_is_clamped_to_the_grids_ceiling() {
        let t = target(9.0, 9.0, 0.0, 1.0, SHAPE_WALL, 250);
        assert!((t.level as usize) < MAX_BUILD_LEVELS);
    }

    #[test]
    fn a_taken_spot_is_refused_before_anything_else_is_asked() {
        // seed 1's solved sites: the ghost predicts against the carved
        // surface, so it needs the same island the sim would build.
        let haven1 = sim_core::terrain::haven(1);
        let content = free_table(SHAPE_WALL);
        let inv = empty_inv();
        let t = Target {
            cx: 3,
            cz: 3,
            level: 0,
            loc: LOC_EDGE_XLO,
        };
        let taken = [PieceRec {
            cx: 3,
            cz: 3,
            level: 0,
            loc: LOC_EDGE_XLO,
            row: 0,
            facing: 0,
            hp: 10,
            uh: 0,
            dmg: 0,
            plate: 0,
        }];
        let (ax, az) = anchor(t.cx, t.cz, t.loc);
        // Nothing built anywhere: `plate_for` answers 0 and refuses nothing,
        // so these two tests keep asking exactly what they asked before.
        let empty_cols = sim_core::collide::ColIndex::new();
        let site = Site {
            seed: 1,
            haven: &haven1,
            at: (ax, az),
            taken: &taken,
            cols: &empty_cols,
            content: &content,
            inv: &inv,
        };
        assert_eq!(
            verdict(t, 0, SHAPE_WALL, &site, false),
            Verdict::No("spot taken")
        );
    }

    /// Reach is measured to the anchor, so a cell whose CENTRE is in range
    /// but whose edge is not must refuse — this is the check the browser's
    /// hand-copied anchor could get silently wrong.
    #[test]
    fn reach_is_measured_to_the_anchor() {
        // seed 1's solved sites: the ghost predicts against the carved
        // surface, so it needs the same island the sim would build.
        let haven1 = sim_core::terrain::haven(1);
        let content = free_table(SHAPE_WALL);
        let inv = empty_inv();
        let t = Target {
            cx: 10,
            cz: 10,
            level: 0,
            loc: LOC_EDGE_XLO,
        };
        let (ax, az) = anchor(t.cx, t.cz, t.loc);
        let empty_cols = sim_core::collide::ColIndex::new();
        let site_near = Site {
            seed: 1,
            haven: &haven1,
            at: (ax, az - BUILD_REACH_M + 0.2),
            taken: &[],
            cols: &empty_cols,
            content: &content,
            inv: &inv,
        };
        assert!(verdict(t, 0, SHAPE_WALL, &site_near, false).ok());
        let site_far = Site {
            seed: 1,
            haven: &haven1,
            at: (ax, az - BUILD_REACH_M - 0.2),
            taken: &[],
            cols: &empty_cols,
            content: &content,
            inv: &inv,
        };
        assert_eq!(
            verdict(t, 0, SHAPE_WALL, &site_far, false),
            Verdict::No("out of reach")
        );
    }

    /// Every sentence the verdict can say must be one the server's own
    /// refusal table says too, or a ghost's guess and the refusal that
    /// follows it read as two different problems.
    #[test]
    fn every_verdict_sentence_is_one_the_sim_also_says() {
        for s in [
            "spot taken",
            "out of reach",
            "bad ground",
            "missing materials",
        ] {
            assert!(
                super::super::refusals::BUILD.contains(&s),
                "{s:?} is not in the build refusal table"
            );
        }
    }

    /// The deploy verdict's sentences, bound to the sim's own CONSTANTS
    /// through the refusal table — the binding no transposition survives
    /// (`refusals.rs`'s own discipline). A verdict sentence that drifted
    /// from the table would make the guess and the refusal that follows it
    /// read as two different problems.
    #[test]
    fn every_deploy_verdict_sentence_is_the_tables_row_for_its_code() {
        use sim_core::deploy::{
            REFUSE_D_COST, REFUSE_D_HAS_LOCK, REFUSE_D_KIND, REFUSE_D_REACH, REFUSE_D_SPOT,
            REFUSE_D_SUPPORT, REFUSE_D_TERRAIN,
        };
        for (code, said) in [
            (REFUSE_D_KIND, "no such deployable"),
            (REFUSE_D_SPOT, "spot taken"),
            (REFUSE_D_REACH, "out of reach"),
            (REFUSE_D_SUPPORT, "needs support"),
            (REFUSE_D_TERRAIN, "bad ground"),
            (REFUSE_D_HAS_LOCK, "that door already has a lock"),
            (REFUSE_D_COST, "item not in inventory"),
        ] {
            assert_eq!(
                super::super::refusals::deploy(code as u8),
                said,
                "the verdict's sentence for code {code} is not the table's"
            );
        }
    }

    /// A doorway-class deployable aims at an EDGE, everything else at the
    /// cell body on level 0 — the split `deploy_target` exists for, and the
    /// reason a door stopped previewing on the plane (`NOW.md` §0u item 3).
    #[test]
    fn a_door_aims_at_an_edge_and_a_box_at_the_cell_body() {
        for i in 0..16 {
            let a = i as f32 * std::f32::consts::TAU / 16.0;
            let door = deploy_target(40.0, 40.0, a.sin(), a.cos(), PLACE_DOORWAY, 1);
            assert!(
                door.loc == LOC_EDGE_XLO || door.loc == LOC_EDGE_ZLO,
                "bearing {i}: a door resolved loc {}",
                door.loc
            );
            assert_eq!(door.level, 1, "a door follows the working level");
            let body = deploy_target(40.0, 40.0, a.sin(), a.cos(), PLACE_GROUND, 1);
            assert_eq!(body.loc, LOC_PLANE);
            assert_eq!(body.level, 0, "a body deploy is sent at level 0");
        }
    }

    /// The default verdict is the NEUTRAL one. A default that refused would
    /// draw red on the frame before anything had been checked — the exact
    /// failure direction the module forbids (red on something the sim would
    /// have accepted), inverted from the build verdict's default for the
    /// inverted reason.
    #[test]
    fn a_deploy_verdict_nobody_computed_is_unknown() {
        assert_eq!(DeployVerdict::default(), DeployVerdict::Unknown);
        assert!(!DeployVerdict::default().refused());
        assert_eq!(DeployVerdict::No("spot taken").why(), "spot taken");
    }

    /// The aim is the LOOK ray. Three contracts, on the shipped island's
    /// own terrain: looking straight down aims at your own feet; looking
    /// steeply up finds no ground and falls back to the fixed projection;
    /// and no direction at all — a 16×5 sweep of bearings and pitches —
    /// parks the aim past build reach.
    #[test]
    fn the_aim_is_the_look_ray_and_never_leaves_reach() {
        // Boxed: `ColIndex` is a large fixed array, and a big struct built
        // on a test thread's stack is the wasm-shadow-stack trap's native
        // cousin (`CLAUDE.md`).
        let cols = Box::new(sim_core::collide::ColIndex::new());
        let seed = 20260731u64;
        // The carved surface the aim marches against, for this seed. Built
        // here rather than shared, so the ghost is tested against the same
        // island `terrain::ground` would give the server.
        let haven = sim_core::terrain::haven(seed);
        let (fx, fz) = (1024.5f32, 1024.5f32);
        let eye_y = sim_core::terrain::ground(seed, &haven, fx, fz) + 1.6;

        // Straight down: the ray meets the ground under the eye.
        let (ax, az) = aim_from_look(
            seed,
            &haven,
            &cols,
            [fx, eye_y, fz],
            [0.0, -1.0, 0.0],
            (fx, fz),
        );
        assert!(
            (ax - fx).abs() < 1e-3 && (az - fz).abs() < 1e-3,
            "straight down aimed at ({ax}, {az}), not the feet"
        );

        // Steeply up: no ground inside the march (the climb outruns any
        // slope), so the old fixed projection answers.
        let (ax, az) = aim_from_look(
            seed,
            &haven,
            &cols,
            [fx, eye_y, fz],
            [0.1, 1.0, 0.0],
            (fx, fz),
        );
        assert!(
            (ax - (fx + AIM_AHEAD_M)).abs() < 1e-3 && (az - fz).abs() < 1e-3,
            "skyward aim fell back to ({ax}, {az}), not {AIM_AHEAD_M} ahead"
        );

        // Nothing escapes the reach circle, whatever the direction.
        for b in 0..16 {
            let a = b as f32 * std::f32::consts::TAU / 16.0;
            for p in [-1.0f32, -0.5, -0.1, 0.2, 0.8] {
                let (ax, az) = aim_from_look(
                    seed,
                    &haven,
                    &cols,
                    [fx, eye_y, fz],
                    [a.cos(), p, a.sin()],
                    (fx, fz),
                );
                let d = ((ax - fx).powi(2) + (az - fz).powi(2)).sqrt();
                assert!(
                    d <= BUILD_REACH_M + 1e-3,
                    "bearing {b} pitch {p}: aim parked {d} out, past reach"
                );
            }
        }
    }
}
