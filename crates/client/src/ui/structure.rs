//! Which built thing `L`, `U`, `R` and the raid verb are pointed at.
//!
//! Separate from [`super::interact`] because the two picks disagree about
//! every term. `E` ranks an aim radius against a nearby rank over the DEPLOY
//! records and measures to a cell centre; these four address a **structure**
//! — either store — and measure to the piece ANCHOR, which is the corner
//! `build.rs`'s own reach checks use. Folding them would have to pick one
//! metric and would then advertise a verb on the wrong terms.
//!
//! ## The store bit is the trap
//!
//! `CLAUDE.md`'s positional-payload entry, in its sharpest form: **a built
//! piece and a deployable can share one address exactly.** `place_deploy`
//! requires the doorway piece at the *identical* `(cx, cz, level, loc)`, so a
//! door and its doorway are the same four numbers and the leading store bit
//! is the only thing that tells them apart. Send the wrong bit and the
//! address is still valid, every field is still in range, the encoder is
//! untouched and no golden moves — the server just mends the other thing, or
//! answers "not damaged" for a wall the player is watching burn.
//!
//! So [`Target`] carries [`Store`] as a typed enum rather than a `bool`, and
//! the call site converts once, at the encoder, where the argument is named.

use sim_core::build::{
    anchor, build_cell_of, shape_has_facing, soft_side, BuildContent, PieceRec, BUILD_CELL_M,
    BUILD_REACH_M, MAT_METAL,
};
use sim_core::deploy::{DeployContent, DeployRec};

/// Which store an address names — `encode_action_repair`'s leading argument
/// and `encode_action_throw`'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Piece,
    Deploy,
}

impl Store {
    /// The wire's bit. The ONE place this conversion happens, so a
    /// transposition has one site to be wrong at instead of five.
    pub fn is_deploy(self) -> bool {
        self == Store::Deploy
    }
}

/// A resolved structure address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub store: Store,
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub loc: u8,
    pub row: u8,
    /// The damage band the server last stated for this address
    /// (`build::damage_band`, 0 = untouched), so a prompt can say whether
    /// repair has anything to buy.
    ///
    /// **This was `hp`/`hp_max` until wire v44, and the pair was a lie.**
    /// Piece and deploy hp were not on the wire then (they are since v102,
    /// [`Self::hp`]) — `write_piece_rec` wrote address and row and stopped —
    /// so the mirror's `hp` was a permanent 0
    /// and `damaged()` answered TRUE for every structure in the world. The
    /// one thing it gates, `hammer.rs`'s "not damaged" refusal, was
    /// therefore unreachable: every repair swing at an intact wall took a
    /// server round trip to come back refused, by a client-side check that
    /// had never once fired. The band fixed that, and since v102 it is the
    /// fallback: `damaged()` reads [`Self::hp`] whenever the row is known.
    pub dmg: u8,
    /// The exact hp the server last stated (wire v102: every record carries
    /// it, `StructHit` and `PieceRepaired` move it). What the hammer prices
    /// a repair from (`hammer::repair_rows`); 0 means not known, since a
    /// standing structure is never at 0.
    pub hp: u16,
    /// The row's baked maximum hp, 0 until that def row has dripped in.
    /// Kept because the repair prompt prices against it; it is content the
    /// client already holds, not a per-piece fact off the wire.
    pub hp_max: u16,
    /// Which face of a sided piece the player stands on — `Some(true)` is
    /// the SOFT side, `Some(false)` the hard one, `None` a shape with no
    /// sides (or the other store). Computed by `sim_core::build::soft_side`,
    /// the same comparison `World::chip` prices the swing with, so the
    /// label can never disagree with the bill (hard/soft v0).
    pub side: Option<bool>,
}

impl Target {
    /// Is there damage to pay for? `build.rs` refuses a repair on an intact
    /// piece (`REFUSE_B_INTACT`), so a client that offered one would be
    /// advertising a refusal.
    ///
    /// **The exact hp decides once the def row is known** (v102): `hp <
    /// hp_max` is `repair`'s own test. The band is a 3-bit rounding of it,
    /// and only as good as the maximum the banding side held: a `StructHit`
    /// that lands before the def drip bands to 0 (`damage_band`'s
    /// unknown-maximum rule), and that wall must not read "not damaged"
    /// with its hp below full. The band answers while the row or the hp is
    /// unknown, which is the case it was made for.
    ///
    /// Both answer for what the client was told. Decay drains hp with no
    /// event, so a wall whose mirror reads full can have rotted since and
    /// reads "not damaged" until a record, a hit or a repair moves it (the
    /// band shared that blind spot).
    pub fn damaged(&self) -> bool {
        if self.hp > 0 && self.hp_max > 0 {
            self.hp < self.hp_max
        } else {
            self.dmg > 0
        }
    }
}

/// How many build cells either side of the feet's own a piece in reach can
/// stand in, per axis.
///
/// An anchor lies in `[x0, x0 + 2/3 cell]` of its cell on each axis
/// (`build::anchor`: an edge's corner side, a centre, a triangle's thirds),
/// so a piece `k` cells up an axis from the feet's cell is more than
/// `k - 1` cells away and one `k` cells down it at least `k - 2/3`. Both
/// pass the reach by `k = floor(reach / cell) + 2`, so one less holds every
/// piece in reach: two at 5 m and 3 m, with a metre to spare up the axis and
/// two down it — far more than the feet's own `floor` can round by.
/// `tests::the_cell_window_drops_no_piece_in_reach` holds it against the
/// unfiltered walk.
const REACH_CELLS: i32 = (BUILD_REACH_M / BUILD_CELL_M) as i32 + 1;

/// The nearest structure within `BUILD_REACH_M` of the feet, over BOTH
/// stores.
///
/// Reach is measured to the anchor via `sim_core::build::anchor` — the sim's
/// own function, not a copy — because that is what `repair` and `upgrade`
/// gate on. Quantize both sides: the client picks inside the radius the
/// server will accept.
///
/// A tie between the two stores at one address goes to the **deployable**,
/// which is the door rather than its doorway. That is the thing a player is
/// looking at and the thing a raider means; the doorway behind it is still
/// reachable by breaking the door first.
///
/// **Runs every frame over the whole piece mirror** (`verbs::resolve`, up to
/// `MAX_PIECES`), so a piece outside the [`REACH_CELLS`] window around the
/// feet is dropped on two integer compares before any anchor is measured
/// (NOW §0pf 3). The compares are on wrapped `u16` offsets: a piece in reach
/// is at most `REACH_CELLS` cells off per axis, so its offset from the
/// window's low corner is `0..=2 * REACH_CELLS` however the feet's cell
/// wraps, and anything else the window lets through is still measured.
pub fn nearest(
    at: (f32, f32),
    pieces: &[PieceRec],
    piece_defs: &BuildContent,
    piece_have: u16,
    deploys: &[DeployRec],
    deploy_defs: &DeployContent,
    deploy_have: u16,
) -> Option<Target> {
    let mut best: Option<Target> = None;
    let mut best_d2 = BUILD_REACH_M * BUILD_REACH_M;

    let span = 2 * REACH_CELLS as u16;
    let x0 = build_cell_of(at.0).wrapping_sub(REACH_CELLS) as u16;
    let z0 = build_cell_of(at.1).wrapping_sub(REACH_CELLS) as u16;
    for rec in pieces {
        if rec.cx.wrapping_sub(x0) > span || rec.cz.wrapping_sub(z0) > span {
            continue;
        }
        let (ax, az) = anchor(rec.cx, rec.cz, rec.loc);
        let (dx, dz) = (ax - at.0, az - at.1);
        let d2 = dx * dx + dz * dz;
        if d2 > best_d2 {
            continue;
        }
        best_d2 = d2;
        let sided = (rec.row as u16) < piece_have
            && shape_has_facing(piece_defs.pieces[rec.row as usize].shape);
        best = Some(Target {
            store: Store::Piece,
            cx: rec.cx,
            cz: rec.cz,
            level: rec.level,
            loc: rec.loc,
            row: rec.row,
            dmg: rec.dmg,
            hp: rec.hp,
            hp_max: if (rec.row as u16) < piece_have {
                piece_defs.pieces[rec.row as usize].hp
            } else {
                0
            },
            side: sided.then(|| soft_side(rec, at.0, at.1)),
        });
    }

    for rec in deploys {
        // The deploy store's own anchor: a door's doorway middle, a
        // free-placed deployable's own centre (`deploy::rec_anchor`, what
        // the sim's repair measures to).
        let (ax, az) = sim_core::deploy::rec_anchor(rec);
        let (dx, dz) = (ax - at.0, az - at.1);
        let d2 = dx * dx + dz * dz;
        // `<=` rather than `<`: a tie goes to the deployable. See the header.
        if d2 > best_d2 {
            continue;
        }
        best_d2 = d2;
        best = Some(deploy_target(rec, deploy_defs, deploy_have));
    }

    best
}

/// The free-placed deployable the crosshair is on, within reach of the
/// feet — what the hammer and the charge mean before [`nearest`] is asked.
///
/// Free placement broke the feet metric for the deploy store: a box stands
/// anywhere in its cell, the foundation under it is nearer from most of the
/// room, and a box against a wall could not be picked from anywhere a body
/// can stand. `E` answered the same problem with the eye's ray
/// ([`super::interact::Sight`]); this is that ray, plus the one thing `E`
/// never needed: a box behind a wall is not the one a raider at the wall
/// means, so a piece crossing the ray first hides it.
#[allow(clippy::too_many_arguments)]
pub fn seen_deploy(
    seed: u64,
    haven: &sim_core::terrain::Haven,
    cols: &sim_core::collide::ColIndex,
    at: (f32, f32),
    sight: &super::interact::Sight<'_>,
    deploys: &[DeployRec],
    deploy_defs: &DeployContent,
    deploy_have: u16,
) -> Option<Target> {
    let mut best: Option<(f32, &DeployRec)> = None;
    for rec in deploys {
        if sim_core::deploy::is_edge_loc(rec.loc)
            || (rec.row as u16) >= deploy_have.min(deploy_defs.def_count)
        {
            continue;
        }
        let arch = deploy_defs.defs[rec.row as usize].arch;
        let Some(rect) = rec.rect(arch) else {
            continue;
        };
        let (ax, az) = sim_core::deploy::rec_anchor(rec);
        let (dx, dz) = (ax - at.0, az - at.1);
        if dx * dx + dz * dz > BUILD_REACH_M * BUILD_REACH_M {
            continue;
        }
        let (bottom, top) = (sight.span)(rec, arch);
        let Some(t) = super::interact::sight_hit(sight, &rect, bottom, top) else {
            continue;
        };
        if best.is_none_or(|(b, _)| t < b) {
            best = Some((t, rec));
        }
    }
    let (t, rec) = best?;
    if piece_crosses(seed, haven, cols, sight.eye, sight.dir, t) {
        return None;
    }
    Some(deploy_target(rec, deploy_defs, deploy_have))
}

/// Whether a built piece stops the look ray before `t` metres — the shot
/// walk (`collide::shot_blocked`) sampled along it, stopping just short of
/// `t` so the floor a box stands on never counts against the box.
fn piece_crosses(
    seed: u64,
    haven: &sim_core::terrain::Haven,
    cols: &sim_core::collide::ColIndex,
    eye: [f32; 3],
    dir: [f32; 3],
    t: f32,
) -> bool {
    const STEP_M: f32 = 0.2;
    let end = t - 0.05;
    let (mut px, mut pz) = (eye[0], eye[2]);
    let mut s = STEP_M.min(end);
    while s > 0.0 {
        let (x, y, z) = (
            eye[0] + dir[0] * s,
            eye[1] + dir[1] * s,
            eye[2] + dir[2] * s,
        );
        if sim_core::collide::shot_blocked(seed, haven, cols, px, pz, x, z, y, 0.0) {
            return true;
        }
        (px, pz) = (x, z);
        if s >= end {
            break;
        }
        s = (s + STEP_M).min(end);
    }
    false
}

fn deploy_target(rec: &DeployRec, deploy_defs: &DeployContent, deploy_have: u16) -> Target {
    Target {
        store: Store::Deploy,
        cx: rec.cx,
        cz: rec.cz,
        level: rec.level,
        loc: rec.loc,
        row: rec.row,
        dmg: rec.dmg,
        hp: rec.hp,
        hp_max: if (rec.row as u16) < deploy_have {
            deploy_defs.defs[rec.row as usize].hp
        } else {
            0
        },
        side: None,
    }
}

/// Which deployable row a held item places, if any.
///
/// **This is what makes a box placeable at all.** `DeployDef::item` is the
/// crafted item placement consumes one unit of, so "the thing in my hand" is
/// a row lookup rather than a menu — which is how the reference does it, and
/// it needs no new wheel. `None` means the held item is not a deployable, or
/// the def table has not dripped far enough to say.
pub fn row_for_item(defs: &DeployContent, have: u16, item: u16) -> Option<u8> {
    if item == 0 {
        return None;
    }
    (0..have.min(defs.def_count))
        .find(|&i| defs.defs[i as usize].item == item)
        .map(|i| i as u8)
}

/// The material one rung above this piece's, or `None` at the top of the
/// ladder.
///
/// The sim refuses an upgrade that is not to the next rung
/// (`REFUSE_B_TIER`), so picking the rung client-side is what turns `U` into
/// one press instead of a guess between three.
pub fn next_material(piece_defs: &BuildContent, have: u16, row: u8) -> Option<u8> {
    if (row as u16) >= have {
        return None;
    }
    let current = piece_defs.pieces[row as usize].material;
    if current >= MAT_METAL {
        return None;
    }
    Some(current + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::build::{damage_band, PieceDef, LOC_EDGE_XLO, LOC_PLANE, MAT_STONE, MAT_WOOD};
    use sim_core::deploy::{DeployDef, ARCH_DOOR};
    use sim_core::limits::MAX_PIECE_COSTS;

    fn piece_table(materials: &[u8]) -> (BuildContent, u16) {
        let mut c = BuildContent::EMPTY;
        for (i, &material) in materials.iter().enumerate() {
            c.pieces[i] = PieceDef {
                shape: 1,
                material,
                hp: 500,
                n_costs: 0,
                costs: [(0, 0); MAX_PIECE_COSTS],
            };
        }
        c.piece_count = materials.len() as u16;
        (c, materials.len() as u16)
    }

    fn deploy_table() -> (DeployContent, u16) {
        let mut c = DeployContent::EMPTY;
        c.defs[0] = DeployDef {
            arch: ARCH_DOOR,
            hp: 200,
            ..DeployDef::INERT
        };
        c.def_count = 1;
        (c, 1)
    }

    /// A piece still stated in hp and banded the way the SERVER bands it
    /// (wire v44) — against `piece_table`'s own 500, so the fixture and the
    /// def it is measured against cannot drift apart.
    fn piece(cx: u16, cz: u16, loc: u8, hp: u16) -> PieceRec {
        PieceRec {
            cx,
            cz,
            level: 0,
            loc,
            row: 0,
            facing: 0,
            hp,
            uh: 0,
            dmg: damage_band(hp, 500),
            plate: 0,
        }
    }

    fn door(cx: u16, cz: u16, loc: u8, hp: u16) -> DeployRec {
        DeployRec {
            cx,
            cz,
            level: 0,
            loc,
            pose: Default::default(),
            row: 0,
            owner: 1,
            hp,
            uh: 0,
            open: false,
            locked: false,
            has_lock: false,
            // `deploy_table`'s own 200 — see `piece`.
            dmg: damage_band(hp, 200),
            grow: 0,
            lit: false,
        }
    }

    /// Free placement: a box near its cell's edge is the hammer's when the
    /// crosshair is on it, though the foundation under it is nearer the
    /// feet — and a wall between the eye and the box hides it again.
    #[test]
    fn the_crosshair_picks_a_free_box_and_a_wall_hides_it() {
        use sim_core::build::{BUILD_CELL_M, SHAPE_FOUNDATION, SHAPE_WALL};
        use sim_core::deploy::{body_base_y, ARCH_BOX};
        use sim_core::footprint::Pose;
        let seed = 20260731;
        let haven = sim_core::terrain::haven(seed);
        // A land cell clear of the authored sites.
        let (cx, cz) = (300..600)
            .step_by(7)
            .flat_map(|cx| (300..600).step_by(7).map(move |cz| (cx as u16, cz as u16)))
            .find(|&(cx, cz)| {
                let (x, z) = (
                    (cx as f32 + 0.5) * BUILD_CELL_M,
                    (cz as f32 + 0.5) * BUILD_CELL_M,
                );
                sim_core::terrain::height(seed, x, z) > sim_core::terrain::SEA_LEVEL + 3.0
                    && !sim_core::terrain::build_reserved(&haven, x, z, 3.0 * BUILD_CELL_M)
            })
            .expect("a land cell");
        let mut cols = sim_core::collide::ColIndex::new();
        cols.add(cx, cz, 0, LOC_PLANE, SHAPE_FOUNDATION, 0);
        let mut defs = DeployContent::EMPTY;
        defs.defs[0] = DeployDef {
            arch: ARCH_BOX,
            hp: 100,
            ..DeployDef::INERT
        };
        defs.def_count = 1;
        // Half a metre west of the cell's centre, wholly inside its cell.
        let boxed = DeployRec {
            cx,
            cz,
            level: 0,
            loc: LOC_PLANE,
            pose: Pose {
                ox: -50,
                oz: 0,
                yaw: 0,
            },
            row: 0,
            ..door(cx, cz, LOC_PLANE, 100)
        };
        let (bx, bz) = boxed.xz();
        let floor = body_base_y(seed, &haven, &cols, &boxed, ARCH_BOX);
        let span = |rec: &DeployRec, arch: u8| {
            let b = body_base_y(seed, &haven, &cols, rec, arch);
            (b, b + 0.7)
        };
        let look = |from: (f32, f32)| {
            let eye = [from.0, floor + 1.6, from.1];
            let d = [bx - eye[0], floor + 0.35 - eye[1], bz - eye[2]];
            let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            (eye, [d[0] / n, d[1] / n, d[2] / n])
        };
        let (dd, dh) = (defs, 1);
        let (pd, ph) = piece_table(&[MAT_WOOD]);
        let pieces = [piece(cx, cz, LOC_PLANE, 500)];

        // Inside, east of the box: the foundation is nearer the feet.
        let feet = (bx + 1.8, bz);
        let (eye, dir) = look(feet);
        let sight = super::super::interact::Sight {
            eye,
            dir,
            span: &span,
        };
        let seen = seen_deploy(seed, &haven, &cols, feet, &sight, &[boxed], &dd, dh)
            .expect("the box the crosshair is on");
        assert_eq!((seen.store, seen.loc), (Store::Deploy, LOC_PLANE));
        let by_feet = nearest(feet, &pieces, &pd, ph, &[boxed], &dd, dh).expect("in reach");
        assert_eq!(
            by_feet.store,
            Store::Piece,
            "the premise: by the feet, the foundation"
        );

        // Outside the cell's west wall, looking at the box through it.
        let mut walled = sim_core::collide::ColIndex::new();
        walled.add(cx, cz, 0, LOC_PLANE, SHAPE_FOUNDATION, 0);
        walled.add(cx, cz, 0, LOC_EDGE_XLO, SHAPE_WALL, 0);
        let feet = (cx as f32 * BUILD_CELL_M - 1.5, bz);
        let (eye, dir) = look(feet);
        let sight = super::super::interact::Sight {
            eye,
            dir,
            span: &span,
        };
        assert_eq!(
            seen_deploy(seed, &haven, &walled, feet, &sight, &[boxed], &dd, dh),
            None,
            "a box behind a wall is not the raider's target"
        );
    }

    #[test]
    fn nothing_in_reach_is_no_target() {
        let (pd, ph) = piece_table(&[MAT_WOOD]);
        let (dd, dh) = deploy_table();
        let far = [piece(200, 200, LOC_PLANE, 10)];
        assert!(nearest((0.0, 0.0), &far, &pd, ph, &[], &dd, dh).is_none());
    }

    /// The trap, stated as a test: one address, two stores, and the pick must
    /// name the door rather than the doorway behind it.
    #[test]
    fn a_door_and_its_doorway_share_an_address_and_the_door_wins() {
        let (pd, ph) = piece_table(&[MAT_WOOD]);
        let (dd, dh) = deploy_table();
        let addr = (5u16, 5u16, LOC_EDGE_XLO);
        let pieces = [piece(addr.0, addr.1, addr.2, 100)];
        let deploys = [door(addr.0, addr.1, addr.2, 100)];
        let (ax, az) = anchor(addr.0, addr.1, addr.2);
        let t = nearest((ax, az), &pieces, &pd, ph, &deploys, &dd, dh).expect("in reach");
        assert_eq!(t.store, Store::Deploy);
        assert!(t.store.is_deploy(), "the wire bit must follow the store");
        // Same four numbers either way — which is exactly why the bit is the
        // only thing distinguishing them.
        assert_eq!((t.cx, t.cz, t.loc), addr);
    }

    #[test]
    fn hp_max_comes_from_the_right_table_for_each_store() {
        let (pd, ph) = piece_table(&[MAT_WOOD]);
        let (dd, dh) = deploy_table();
        let (ax, az) = anchor(5, 5, LOC_PLANE);
        let t = nearest(
            (ax, az),
            &[piece(5, 5, LOC_PLANE, 250)],
            &pd,
            ph,
            &[],
            &dd,
            dh,
        )
        .unwrap();
        assert_eq!((t.dmg, t.hp, t.hp_max), (damage_band(250, 500), 250, 500));
        assert!(t.damaged());

        let (ax, az) = anchor(6, 6, LOC_EDGE_XLO);
        let t = nearest(
            (ax, az),
            &[],
            &pd,
            ph,
            &[door(6, 6, LOC_EDGE_XLO, 200)],
            &dd,
            dh,
        )
        .unwrap();
        assert_eq!((t.dmg, t.hp, t.hp_max), (0, 200, 200));
        assert!(!t.damaged(), "an intact door has nothing to buy");
    }

    /// A `StructHit` that lands before the def drip bands to 0 (an unknown
    /// maximum), but the hp beside it is exact: once the row is known the
    /// hp decides, so a wall below full never reads "not damaged". The band
    /// answers only while the row or the hp is unknown.
    #[test]
    fn the_exact_hp_decides_damage_once_the_row_is_known() {
        let t = Target {
            store: Store::Piece,
            cx: 5,
            cz: 5,
            level: 0,
            loc: LOC_PLANE,
            row: 0,
            dmg: 0,
            hp: 499,
            hp_max: 500,
            side: None,
        };
        assert!(t.damaged(), "one point down under a zero band is damage");
        assert!(
            !Target {
                dmg: 1,
                hp: 500,
                ..t
            }
            .damaged(),
            "a band does not outvote a full hp"
        );
        assert!(
            Target {
                hp_max: 0,
                dmg: 1,
                ..t
            }
            .damaged(),
            "undripped row"
        );
        assert!(!Target { hp_max: 0, ..t }.damaged());
        assert!(Target { hp: 0, dmg: 1, ..t }.damaged(), "unknown hp");
        assert!(!Target { hp: 0, ..t }.damaged());
    }

    /// A row the def table has not dripped reports max 0 — and since wire
    /// v44 it still reports **damage**, which is the improvement rather than
    /// a regression.
    ///
    /// This asserted `!damaged()` until 2026-08-16, and that was a property
    /// of the arithmetic rather than of the system: `damaged` divided hp by
    /// a maximum the client did not have, so an undripped row could only
    /// answer false. The band is the SERVER's division now — it knows every
    /// maximum — so it survives a def table that has not arrived, and a
    /// player who walks into a raided base before the drip finishes sees the
    /// damage rather than a clean wall.
    ///
    /// **What has not changed is the behaviour that mattered**: `hammer`'s
    /// repair guard is `!damaged() && hp_max > 0`, so an undripped row still
    /// sends and lets the sim answer. That is gated where it belongs, on
    /// `hammer::act` itself (`tests/ui.rs` §K), not inferred from this.
    #[test]
    fn an_undripped_row_reports_no_maximum() {
        let (pd, _) = piece_table(&[MAT_WOOD]);
        let (dd, dh) = deploy_table();
        let (ax, az) = anchor(5, 5, LOC_PLANE);
        let t = nearest(
            (ax, az),
            &[piece(5, 5, LOC_PLANE, 100)],
            &pd,
            0,
            &[],
            &dd,
            dh,
        )
        .unwrap();
        assert_eq!(t.hp_max, 0, "an undripped row has no known maximum");
        // The fixture bands against `piece_table`'s own 500, which is what
        // the server would have sent: hp 100 of 500 is heavy damage, and it
        // reaches the client whole even with the def table empty.
        assert_eq!(t.dmg, damage_band(100, 500));
        assert!(t.damaged(), "the server's band survives an undripped table");
    }

    #[test]
    fn reach_is_the_sims_reach_measured_to_the_anchor() {
        let (pd, ph) = piece_table(&[MAT_WOOD]);
        let (dd, dh) = deploy_table();
        let pieces = [piece(9, 9, LOC_EDGE_XLO, 10)];
        let (ax, az) = anchor(9, 9, LOC_EDGE_XLO);
        assert!(nearest(
            (ax, az - BUILD_REACH_M + 0.2),
            &pieces,
            &pd,
            ph,
            &[],
            &dd,
            dh
        )
        .is_some());
        assert!(nearest(
            (ax, az - BUILD_REACH_M - 0.2),
            &pieces,
            &pd,
            ph,
            &[],
            &dd,
            dh
        )
        .is_none());
    }

    #[test]
    fn the_ladder_climbs_once_and_stops_at_metal() {
        let (pd, ph) = piece_table(&[MAT_WOOD, MAT_STONE, MAT_METAL]);
        assert_eq!(next_material(&pd, ph, 0), Some(MAT_STONE));
        assert_eq!(next_material(&pd, ph, 1), Some(MAT_METAL));
        assert_eq!(next_material(&pd, ph, 2), None);
        // An undripped row has no known material, so no rung either.
        assert_eq!(next_material(&pd, 0, 0), None);
    }

    #[test]
    fn a_held_item_finds_the_row_that_places_it() {
        let mut c = DeployContent::EMPTY;
        c.defs[0] = DeployDef {
            arch: ARCH_DOOR,
            item: 41,
            ..DeployDef::INERT
        };
        c.defs[1] = DeployDef {
            arch: sim_core::deploy::ARCH_BOX,
            item: 42,
            ..DeployDef::INERT
        };
        c.def_count = 2;
        assert_eq!(row_for_item(&c, 2, 42), Some(1));
        assert_eq!(row_for_item(&c, 2, 41), Some(0));
        assert_eq!(row_for_item(&c, 2, 99), None);
        // Item 0 is "empty slot", never a deployable.
        assert_eq!(row_for_item(&c, 2, 0), None);
        // A row past what has dripped is not searched — its `item` is
        // INERT's and would match the wrong hand.
        assert_eq!(row_for_item(&c, 1, 42), None);
    }

    /// **`nearest`'s cell window drops nothing the unfiltered walk reaches**
    /// (NOW §0pf 3). One piece of every anchor kind, the feet swept on a
    /// quarter-metre grid over the cells around it — cell edges and the
    /// reach circle's rim included — and the address compared with the walk
    /// the window replaced, kept here as it was. Then every piece at once,
    /// for the tie rule. Near cell 0 too, where the window's low corner
    /// wraps.
    ///
    /// It must also REACH the window's outermost cells both ways, or a
    /// window one cell too narrow would agree with the walk everywhere this
    /// sweep happened to look.
    #[test]
    fn the_cell_window_drops_no_piece_in_reach() {
        use sim_core::build::{
            LOC_DIAG_A, LOC_EDGE_ZLO, LOC_RISER, LOC_TRI_XHI_ZHI, LOC_TRI_XHI_ZLO, LOC_TRI_XLO_ZHI,
            LOC_TRI_XLO_ZLO,
        };
        let walk = |at: (f32, f32), pieces: &[PieceRec]| {
            let mut best = None;
            let mut best_d2 = BUILD_REACH_M * BUILD_REACH_M;
            for r in pieces {
                let (ax, az) = anchor(r.cx, r.cz, r.loc);
                let (dx, dz) = (ax - at.0, az - at.1);
                let d2 = dx * dx + dz * dz;
                if d2 <= best_d2 {
                    best_d2 = d2;
                    best = Some((r.cx, r.cz, r.loc));
                }
            }
            best
        };
        let (pd, ph) = piece_table(&[MAT_WOOD]);
        let (dd, dh) = deploy_table();
        let pick = |at: (f32, f32), pieces: &[PieceRec]| {
            nearest(at, pieces, &pd, ph, &[], &dd, dh).map(|t| (t.cx, t.cz, t.loc))
        };
        let locs = [
            LOC_PLANE,
            LOC_RISER,
            LOC_EDGE_XLO,
            LOC_EDGE_ZLO,
            LOC_TRI_XLO_ZLO,
            LOC_TRI_XHI_ZLO,
            LOC_TRI_XLO_ZHI,
            LOC_TRI_XHI_ZHI,
            LOC_DIAG_A,
        ];
        let (mut up, mut down, mut reached) = (false, false, 0usize);
        for (cx, cz) in [(100u16, 100u16), (0, 1)] {
            let all: Vec<PieceRec> = locs.iter().map(|&l| piece(cx, cz, l, 500)).collect();
            let (x0, z0) = (cx as f32 * BUILD_CELL_M, cz as f32 * BUILD_CELL_M);
            for i in 0..=96 {
                for j in 0..=96 {
                    let at = (x0 - 10.5 + i as f32 * 0.25, z0 - 10.5 + j as f32 * 0.25);
                    for rec in &all {
                        let one = std::slice::from_ref(rec);
                        let got = pick(at, one);
                        assert_eq!(got, walk(at, one), "{rec:?} from {at:?}");
                        if got.is_some() {
                            reached += 1;
                            let off = cx as i32 - build_cell_of(at.0);
                            up |= off == REACH_CELLS;
                            down |= off == -REACH_CELLS;
                        }
                    }
                    assert_eq!(pick(at, &all), walk(at, &all), "every piece from {at:?}");
                }
            }
        }
        assert!(reached > 0, "the sweep never reached a piece");
        assert!(
            up && down,
            "no piece in reach {REACH_CELLS} cells off (up {up}, down {down})"
        );
    }
}
