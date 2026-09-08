//! No gap between drawn pieces, and no two faces fighting for one plane
//! (gap v1, `DECISIONS.md` §open).
//!
//! The 2026-09-04 playtest: *"building pieces have a gap in them."* Three
//! defects, all in `render/structures.rs` and all geometric: a 4 cm seam every
//! piece kept from its cell (a see-through slit between any two walls in a
//! line and between any two floors), a bare notch at every outside corner
//! where two walls stopped short of each other, and — the moment a floor went
//! on a wall — a 12 cm strip where the wall's head and the slab's top shared a
//! plane and flickered. The fixes are three numbers and one rule
//! (`EDGE_DROP_M`, `POST_PROUD_M`, `DIAG_DROP_M`, `post_owner`), and this
//! file is what holds them.
//!
//! **Arithmetic, not pixels.** `CLAUDE.md` says the visual gate is a person
//! looking, and this is not one: it asks two questions a box can answer.
//! *Is every point along an edge piece's line and at its corners inside
//! something drawn?* — that is "no gap". *Do any two drawn faces with the
//! same normal share a plane over a positive area that nothing hides?* — that
//! is "no z-fight", stated the way the GPU decides it. Both are asked over a
//! base built through the sim's own verb, with the posts each piece actually
//! draws under [`post_owner`], not the ghost's both-posts preview.

use bevy::prelude::*;
use client::render::structures::{
    apron_parts, base_transform, corner_posts, foundation_part, is_edge_shape, parts_for,
    post_owner, post_w, Part, PostOwn, APRON_DEPTH_M, DIAG_DROP_M, EDGE_DROP_M,
};
use sim_core::build::{
    place, BuildContent, PieceRec, Pieces, BUILD_CELL_M, LEVEL_H_M, LOC_DIAG_A, LOC_DIAG_B,
    LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE, SHAPE_DOORWAY, SHAPE_FOUNDATION, SHAPE_WALL,
};
use sim_core::collide::{ColIndex, WALL_THICKNESS_M};
use sim_core::deploy::Deploys;
use sim_core::gather::ItemStack;
use sim_core::movement::Body;
use sim_core::terrain;
use sim_core::world::{EventQueue, Player, EV_BUILD_REFUSED, EV_PIECE_PLACED};

const SEED: u64 = 20260731;
const CX: u16 = 341;
const CZ: u16 = 341;
const ROW_FOUNDATION: u16 = 0;
const ROW_WALL: u16 = 1;
const ROW_FLOOR: u16 = 2;
const ROW_DOORWAY: u16 = 3;

fn hv() -> &'static terrain::Haven {
    use std::sync::OnceLock;
    static H: OnceLock<terrain::Haven> = OnceLock::new();
    H.get_or_init(|| terrain::haven(SEED))
}

struct Rig {
    bc: BuildContent,
    pieces: Pieces,
    deploys: Deploys,
}

impl Rig {
    fn new() -> Self {
        Self {
            bc: BuildContent::probe_fixture(),
            pieces: Pieces::new(),
            deploys: Deploys::new(),
        }
    }

    fn place(&mut self, row: u16, cx: u16, cz: u16, level: u8, loc: u8) -> Result<(), u32> {
        let (ax, az) = sim_core::build::anchor(cx, cz, loc);
        let mut p = Player {
            id: 7,
            active: true,
            body: Body::at(SEED, hv(), ax, az),
            ..Player::default()
        };
        for (i, slot) in p.inv.iter_mut().enumerate().take(2) {
            *slot = ItemStack {
                item: i as u16,
                count: 99,
                cond: 0,
            };
        }
        let mut ev = EventQueue::default();
        place(
            SEED,
            hv(),
            &self.bc,
            &self.deploys,
            &mut self.pieces,
            &mut p,
            0,
            row,
            cx,
            cz,
            level,
            loc,
            false,
            0,
            &mut ev,
        );
        match ev.entries().iter().find(|e| e.code == EV_BUILD_REFUSED) {
            Some(e) => Err(e.b),
            None => {
                assert!(
                    ev.entries().iter().any(|e| e.code == EV_PIECE_PLACED),
                    "the verb neither placed nor refused"
                );
                Ok(())
            }
        }
    }
}

/// A base with every join this file is about: a 2×2 of foundations, walls all
/// round and one down the middle, a doorway in the ring, floors over two
/// cells, walls stacked on the storey above, and a diagonal in a corner.
fn base() -> Rig {
    let mut r = Rig::new();
    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        r.place(ROW_FOUNDATION, CX + dx, CZ + dz, 0, LOC_PLANE)
            .expect("foundation");
    }
    // The ring: west and east edges of both rows, north and south of both
    // columns; the east edge of the north row is a doorway.
    for dz in 0..2 {
        r.place(ROW_WALL, CX, CZ + dz, 0, LOC_EDGE_XLO)
            .expect("west");
        if dz == 0 {
            r.place(ROW_DOORWAY, CX + 2, CZ + dz, 0, LOC_EDGE_XLO)
                .expect("east doorway");
        } else {
            r.place(ROW_WALL, CX + 2, CZ + dz, 0, LOC_EDGE_XLO)
                .expect("east");
        }
    }
    for dx in 0..2 {
        r.place(ROW_WALL, CX + dx, CZ, 0, LOC_EDGE_ZLO)
            .expect("north");
        r.place(ROW_WALL, CX + dx, CZ + 2, 0, LOC_EDGE_ZLO)
            .expect("south");
    }
    // A wall down the middle (a T at each end), a floor over the west
    // column, and walls stacked on the storey above along its west edge.
    r.place(ROW_WALL, CX + 1, CZ, 0, LOC_EDGE_XLO)
        .expect("middle");
    r.place(ROW_FLOOR, CX, CZ, 1, LOC_PLANE).expect("floor NW");
    r.place(ROW_FLOOR, CX, CZ + 1, 1, LOC_PLANE)
        .expect("floor SW");
    r.place(ROW_WALL, CX, CZ, 1, LOC_EDGE_XLO)
        .expect("upper west");
    r.place(ROW_WALL, CX, CZ + 1, 1, LOC_EDGE_XLO)
        .expect("upper west 2");
    r.place(ROW_WALL, CX, CZ, 1, LOC_EDGE_ZLO)
        .expect("upper north");
    // A diagonal across the SE cell.
    r.place(ROW_WALL, CX + 1, CZ + 1, 0, LOC_DIAG_A)
        .expect("diagonal");
    r
}

/// One drawn box in world space — every straight piece's parts are
/// axis-aligned under `base_transform`'s quarter-turns, so a box is its two
/// corners. Diagonals are not boxes and are handled apart.
#[derive(Clone, Copy, Debug)]
struct Aabb {
    lo: Vec3,
    hi: Vec3,
    /// Which piece and part, for the messages.
    tag: (u16, u16, u8, u8, usize),
}

impl Aabb {
    fn of(part: &Part, root: &Transform, tag: (u16, u16, u8, u8, usize)) -> Self {
        let m = (*root * part.transform()).to_matrix();
        let h = part.size * 0.5;
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for sx in [-1.0, 1.0] {
            for sy in [-1.0, 1.0] {
                for sz in [-1.0, 1.0] {
                    let p = m.transform_point3(Vec3::new(sx * h.x, sy * h.y, sz * h.z));
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
            }
        }
        Self { lo, hi, tag }
    }

    fn contains(&self, p: Vec3, slack: f32) -> bool {
        p.x >= self.lo.x - slack
            && p.x <= self.hi.x + slack
            && p.y >= self.lo.y - slack
            && p.y <= self.hi.y + slack
            && p.z >= self.lo.z - slack
            && p.z <= self.hi.z + slack
    }

    fn strictly_inside(&self, p: Vec3, margin: f32) -> bool {
        p.x > self.lo.x + margin
            && p.x < self.hi.x - margin
            && p.y > self.lo.y + margin
            && p.y < self.hi.y - margin
            && p.z > self.lo.z + margin
            && p.z < self.hi.z - margin
    }
}

/// What the standing pieces of `r` draw, as the parts `spawn_piece` would
/// spawn them: the foundations' footing, every straight edge piece's body,
/// the posts [`post_owner`] gives it and — on the ground storey — its apron
/// (part index 10 and up), and every other shape's own parts. Returns the
/// boxes and, separately, the diagonals' (top, root).
fn drawn(r: &Rig) -> (Vec<Aabb>, Vec<(f32, Transform)>) {
    let mut boxes = Vec::new();
    let mut diagonals = Vec::new();
    for rec in r.pieces.entries() {
        let shape = r.bc.pieces[rec.row as usize].shape;
        let addr = (rec.cx, rec.cz, rec.level, rec.loc);
        let root = base_transform(SEED, hv(), addr, rec.plate);
        if matches!(
            shape,
            SHAPE_FOUNDATION | sim_core::build::SHAPE_TRI_FOUNDATION
        ) {
            let part = foundation_part(
                SEED,
                hv(),
                rec.cx,
                rec.cz,
                shape == sim_core::build::SHAPE_TRI_FOUNDATION,
                rec.plate,
            );
            boxes.push(Aabb::of(
                &part,
                &root,
                (rec.cx, rec.cz, rec.level, rec.loc, 0),
            ));
            continue;
        }
        if shape == SHAPE_WALL && matches!(rec.loc, LOC_DIAG_A | LOC_DIAG_B) {
            let (parts, _) = parts_for(shape, rec.loc);
            let top = root.translation.y + parts[0].offset.y + parts[0].size.y * 0.5;
            diagonals.push((top, root));
            continue;
        }
        let own = if is_edge_shape(shape) {
            post_owner(r.pieces.cols(), rec.cx, rec.cz, rec.level, rec.loc)
        } else {
            PostOwn::default()
        };
        let (parts, n) = parts_for(shape, rec.loc);
        for (i, part) in parts[..n].iter().enumerate() {
            if !own.draws(part.role) {
                continue;
            }
            boxes.push(Aabb::of(
                part,
                &root,
                (rec.cx, rec.cz, rec.level, rec.loc, i),
            ));
        }
        if rec.level == 0 && is_edge_shape(shape) && matches!(rec.loc, LOC_EDGE_XLO | LOC_EDGE_ZLO)
        {
            let (parts, n) = apron_parts(own);
            for (i, part) in parts[..n].iter().enumerate() {
                boxes.push(Aabb::of(
                    part,
                    &root,
                    (rec.cx, rec.cz, rec.level, rec.loc, 10 + i),
                ));
            }
        }
    }
    (boxes, diagonals)
}

fn edge_recs(r: &Rig) -> Vec<PieceRec> {
    r.pieces
        .entries()
        .iter()
        .copied()
        .filter(|rec| {
            is_edge_shape(r.bc.pieces[rec.row as usize].shape)
                && matches!(rec.loc, LOC_EDGE_XLO | LOC_EDGE_ZLO)
        })
        .collect()
}

/// The world line of a straight edge piece: its two corners, and the storey
/// base it stands on.
fn edge_line(rec: &PieceRec) -> (Vec3, Vec3) {
    let root = base_transform(SEED, hv(), (rec.cx, rec.cz, rec.level, rec.loc), rec.plate);
    let m = root.to_matrix();
    (
        m.transform_point3(Vec3::new(0.0, 0.0, -BUILD_CELL_M * 0.5)),
        m.transform_point3(Vec3::new(0.0, 0.0, BUILD_CELL_M * 0.5)),
    )
}

// ---------------------------------------------------------------------------
// §A · No gap
// ---------------------------------------------------------------------------

/// Along every straight edge piece's line, corner to corner — the corners
/// themselves included — at every height the sim blocks, there is something
/// drawn. That is the seam and the notch, closed, asked of the base rather
/// than of a constant: a wall in a line with another, a wall meeting another
/// at a corner, a wall meeting a doorway, a wall at a T.
#[test]
fn every_edge_line_is_drawn_end_to_end() {
    let r = base();
    let (boxes, _) = drawn(&r);
    let mut checked = 0;
    for rec in edge_recs(&r) {
        let (a, b) = edge_line(&rec);
        let shape = r.bc.pieces[rec.row as usize].shape;
        for i in 0..=120 {
            let t = i as f32 / 120.0;
            // The doorway's opening is open on purpose; sample its posts.
            if shape == SHAPE_DOORWAY {
                let along = t * BUILD_CELL_M;
                if !sim_core::collide::doorway_solid_at(along) {
                    continue;
                }
            }
            for y in [0.05, 1.5, LEVEL_H_M - 0.05] {
                let p = a.lerp(b, t) + Vec3::Y * y;
                assert!(
                    boxes.iter().any(|bx| bx.contains(p, 1e-4)),
                    "piece {:?}: nothing drawn at {p:?} ({t:.3} of the way along, {y} m up)",
                    (rec.cx, rec.cz, rec.level, rec.loc)
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 1_000, "only {checked} points were checked");
}

/// At every corner an edge piece ends at, exactly one piece draws the post —
/// one, so no two coincide (coincident boxes z-fight over their whole
/// surface), and not zero, so no corner is bare. Asked of every corner in the
/// base, at both storeys, over T-joins, L-corners, in-line joins and free
/// ends alike.
#[test]
fn every_corner_has_exactly_one_post() {
    let r = base();
    let recs = edge_recs(&r);
    let mut corners: std::collections::HashMap<(u16, u16, u8), (u32, u32)> =
        std::collections::HashMap::new();
    for rec in &recs {
        let own = post_owner(r.pieces.cols(), rec.cx, rec.cz, rec.level, rec.loc);
        let (low, high) = match rec.loc {
            LOC_EDGE_XLO => ((rec.cx, rec.cz), (rec.cx, rec.cz + 1)),
            _ => ((rec.cx, rec.cz), (rec.cx + 1, rec.cz)),
        };
        for (corner, owns) in [(low, own.low), (high, own.high)] {
            let e = corners
                .entry((corner.0, corner.1, rec.level))
                .or_insert((0, 0));
            e.0 += 1;
            e.1 += owns as u32;
        }
    }
    assert!(
        corners.len() >= 12,
        "the base has only {} corners",
        corners.len()
    );
    let mut shared = 0;
    for (corner, (pieces, owners)) in corners {
        assert_eq!(
            owners, 1,
            "corner {corner:?}: {pieces} pieces meet and {owners} draw the post"
        );
        if pieces > 1 {
            shared += 1;
        }
    }
    assert!(
        shared >= 6,
        "only {shared} corners are shared, so the rule was barely asked"
    );
}

/// The rule is pure in the index: a piece placed later takes a post the
/// earlier piece drew, and `stream` redraws the earlier one because it
/// re-reads ownership every pass. Checked on the index directly.
#[test]
fn an_arriving_neighbour_takes_the_post_it_ranks_for() {
    let mut cols = ColIndex::new();
    cols.add(CX, CZ, 0, LOC_EDGE_ZLO, SHAPE_WALL, 0);
    // Alone, the north wall owns both its corners.
    assert_eq!(post_owner(&cols, CX, CZ, 0, LOC_EDGE_ZLO), PostOwn::BOTH);
    // A west wall arrives, sharing the low corner and ranking first there.
    cols.add(CX, CZ, 0, LOC_EDGE_XLO, SHAPE_WALL, 0);
    let north = post_owner(&cols, CX, CZ, 0, LOC_EDGE_ZLO);
    let west = post_owner(&cols, CX, CZ, 0, LOC_EDGE_XLO);
    assert!(
        !north.low && north.high,
        "the north wall kept the shared post"
    );
    assert!(west.low && west.high, "the west wall did not take it");
    // Take the west wall away and the north wall has it back.
    cols.del(CX, CZ, 0, LOC_EDGE_XLO, SHAPE_WALL);
    assert_eq!(post_owner(&cols, CX, CZ, 0, LOC_EDGE_ZLO), PostOwn::BOTH);
}

// ---------------------------------------------------------------------------
// §B · No two faces on one plane
// ---------------------------------------------------------------------------

/// Two boxes' faces with the SAME outward normal, on the same plane, over a
/// region of positive area: the overlap rectangle, if any.
fn coplanar_overlap(a: &Aabb, b: &Aabb, axis: usize, high: bool) -> Option<[(f32, f32); 2]> {
    let (pa, pb) = if high {
        (a.hi[axis], b.hi[axis])
    } else {
        (a.lo[axis], b.lo[axis])
    };
    if (pa - pb).abs() > 1e-4 {
        return None;
    }
    let others: [usize; 2] = match axis {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    };
    let mut rect = [(0.0, 0.0); 2];
    for (k, o) in others.into_iter().enumerate() {
        let lo = a.lo[o].max(b.lo[o]);
        let hi = a.hi[o].min(b.hi[o]);
        // Abutting is not overlapping: a shared EDGE is what neighbours
        // have, and it is fine.
        if hi - lo <= 1e-3 {
            return None;
        }
        rect[k] = (lo, hi);
    }
    Some(rect)
}

/// The first pair of same-normal coplanar faces over a positive area that
/// nothing hides, as a sentence — or `None` when the boxes have none.
fn first_fight(boxes: &[Aabb]) -> Option<String> {
    for (i, a) in boxes.iter().enumerate() {
        for b in &boxes[i + 1..] {
            for axis in 0..3 {
                for high in [false, true] {
                    let Some(rect) = coplanar_overlap(a, b, axis, high) else {
                        continue;
                    };
                    let plane = if high { a.hi[axis] } else { a.lo[axis] };
                    // Sample the shared region, inset a hair from its rim.
                    let inset = 1e-3;
                    for u in 0..=6 {
                        for v in 0..=6 {
                            let fu = inset + (rect[0].1 - rect[0].0 - 2.0 * inset) * u as f32 / 6.0;
                            let fv = inset + (rect[1].1 - rect[1].0 - 2.0 * inset) * v as f32 / 6.0;
                            let (s, t) = (rect[0].0 + fu, rect[1].0 + fv);
                            let p = match axis {
                                0 => Vec3::new(plane, s, t),
                                1 => Vec3::new(s, plane, t),
                                _ => Vec3::new(s, t, plane),
                            };
                            let covered = boxes
                                .iter()
                                .filter(|c| !std::ptr::eq(*c, a) && !std::ptr::eq(*c, b))
                                .any(|c| c.strictly_inside(p, 1e-4));
                            if !covered {
                                return Some(format!(
                                    "faces of {:?} and {:?} share the plane {}={plane:.4} over \
{rect:?} and nothing hides it — z-fighting",
                                    a.tag,
                                    b.tag,
                                    ["x", "y", "z"][axis]
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// No two drawn faces fight for a plane anyone can see: wherever two boxes'
/// same-normal faces share a plane over a positive area, every point of that
/// area is strictly inside a third box — hidden — or the pair is a defect.
///
/// This is the wall-head-under-a-floor strip, the coincident-post case, the
/// widened skirts' corner patches (which is how this test earned its keep
/// the day it was written) and every seam-era assumption, stated as what the
/// depth buffer decides. **Proven on a mutant in the same test**: the base
/// with one box drawn twice must be flagged, or a checker that matched
/// nothing would be reporting a base with no defects.
#[test]
fn no_two_visible_faces_share_a_plane() {
    let r = base();
    let (boxes, _) = drawn(&r);
    assert!(
        boxes.len() > 30,
        "the base draws only {} boxes",
        boxes.len()
    );
    if let Some(fight) = first_fight(&boxes) {
        panic!("{fight}");
    }
    // The mutant: the same wall drawn twice.
    let mut doubled = boxes.clone();
    let dup = doubled[5];
    doubled.push(dup);
    assert!(
        first_fight(&doubled).is_some(),
        "a box drawn twice was not flagged — the checker sees nothing"
    );
}

/// The wall's head is inside the slab above it and its foot inside the slab
/// below — the edge drop, measured on the base: for every edge piece with a
/// plane at its storey above, the head lies strictly within that slab's
/// thickness; for every one, the foot lies within the slab under it.
#[test]
fn an_edge_pieces_head_and_foot_are_inside_the_slabs() {
    let r = base();
    let (boxes, _) = drawn(&r);
    let slab_of = |cx: u16, cz: u16, level: u8| -> Option<Aabb> {
        boxes.iter().copied().find(|b| {
            b.tag.0 == cx
                && b.tag.1 == cz
                && b.tag.2 == level
                && b.tag.3 == LOC_PLANE
                && b.tag.4 == 0
        })
    };
    let mut checked = 0;
    for rec in edge_recs(&r) {
        // The piece's own parts — not its apron, which hangs below its foot
        // by design (part indices 10 and up).
        let bodies: Vec<&Aabb> = boxes
            .iter()
            .filter(|b| {
                (b.tag.0, b.tag.1, b.tag.2, b.tag.3) == (rec.cx, rec.cz, rec.level, rec.loc)
                    && b.tag.4 < 10
            })
            .collect();
        assert!(!bodies.is_empty(), "edge piece {:?} draws nothing", rec);
        let head = bodies.iter().map(|b| b.hi.y).fold(f32::MIN, f32::max);
        let foot = bodies.iter().map(|b| b.lo.y).fold(f32::MAX, f32::min);
        // The cells this edge adjoins.
        let cells: [(u16, u16); 2] = match rec.loc {
            LOC_EDGE_XLO => [(rec.cx, rec.cz), (rec.cx.wrapping_sub(1), rec.cz)],
            _ => [(rec.cx, rec.cz), (rec.cx, rec.cz.wrapping_sub(1))],
        };
        for (cx, cz) in cells {
            if let Some(above) = slab_of(cx, cz, rec.level + 1) {
                assert!(
                    head > above.lo.y + 1e-4 && head < above.hi.y - 1e-4,
                    "edge {:?}: head {head:.4} is not inside the slab above ({:.4}..{:.4})",
                    (rec.cx, rec.cz, rec.level, rec.loc),
                    above.lo.y,
                    above.hi.y
                );
                checked += 1;
            }
            if let Some(below) = slab_of(cx, cz, rec.level) {
                assert!(
                    foot > below.lo.y + 1e-4 && foot < below.hi.y - 1e-4,
                    "edge {:?}: foot {foot:.4} is not inside the slab under it ({:.4}..{:.4})",
                    (rec.cx, rec.cz, rec.level, rec.loc),
                    below.lo.y,
                    below.hi.y
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 10, "only {checked} slab joins were checked");
}

/// A wall stacked on a wall abuts it exactly — the upper's foot on the
/// lower's head, no slit and no overlap — because both dropped together.
#[test]
fn a_stacked_wall_meets_the_one_under_it() {
    let r = base();
    let (boxes, _) = drawn(&r);
    let mut checked = 0;
    for rec in edge_recs(&r) {
        if rec.level == 0 {
            continue;
        }
        let head_below = boxes
            .iter()
            .filter(|b| (b.tag.0, b.tag.1, b.tag.2, b.tag.3) == (rec.cx, rec.cz, 0, rec.loc))
            .map(|b| b.hi.y)
            .fold(f32::MIN, f32::max);
        let foot = boxes
            .iter()
            .filter(|b| {
                (b.tag.0, b.tag.1, b.tag.2, b.tag.3) == (rec.cx, rec.cz, rec.level, rec.loc)
            })
            .map(|b| b.lo.y)
            .fold(f32::MAX, f32::min);
        assert!(
            (foot - head_below).abs() < 1e-4,
            "upper wall {:?}: foot {foot:.4} against the lower's head {head_below:.4}",
            (rec.cx, rec.cz, rec.level, rec.loc)
        );
        checked += 1;
    }
    assert!(checked >= 2, "only {checked} stacked walls were checked");
}

/// The diagonal's head sits under every post's head (its end is inside the
/// post a straight wall owns at that corner), and its foot is the edge drop
/// like everyone else's.
#[test]
fn a_diagonal_keeps_its_head_under_the_posts() {
    let r = base();
    let (boxes, diagonals) = drawn(&r);
    assert!(!diagonals.is_empty(), "the base has no diagonal");
    let [lo_post, _] = corner_posts();
    let post_head = lo_post.offset.y + lo_post.size.y * 0.5;
    for (top, root) in diagonals {
        let expected = root.translation.y + post_head - DIAG_DROP_M;
        assert!(
            (top - expected).abs() < 1e-4,
            "diagonal head at {top:.4}, expected {expected:.4}"
        );
        // ...and below every post drawn at its storey.
        for b in boxes.iter().filter(|b| {
            (b.hi.y - (root.translation.y + post_head)).abs() < 1e-4
                && (b.hi.x - b.lo.x - post_w()).abs() < 1e-4
        }) {
            assert!(
                top < b.hi.y - 1e-4,
                "the diagonal's head reaches a post's at {:?}",
                b.tag
            );
        }
    }
}

/// The apron closes the undercut: under a ground-storey wall's outer half,
/// which hangs past the slab's edge, the wall's own plinth reaches down from
/// the wall's foot in the wall's own plane, as deep as the footing's ceiling
/// — so from outside a wall and the ground under it are one face.
#[test]
fn the_apron_closes_the_undercut() {
    let (wall, _) = parts_for(SHAPE_WALL, LOC_EDGE_XLO);
    let (apron, n) = apron_parts(PostOwn::BOTH);
    assert_eq!(
        n, 3,
        "an apron owning both posts is a strip and two post feet"
    );
    let strip = apron[0];
    // The wall's plane and thickness, exactly.
    assert!(
        (strip.size.x - wall[0].size.x).abs() < 1e-6,
        "the apron is not the wall's thickness"
    );
    assert!(
        (strip.size.z - wall[0].size.z).abs() < 1e-6,
        "the apron does not run between the posts"
    );
    // Its top is the wall's foot; its bottom the footing's ceiling below the
    // storey base.
    let top = strip.offset.y + strip.size.y * 0.5;
    let foot = wall[0].offset.y - wall[0].size.y * 0.5;
    assert!(
        (top - foot).abs() < 1e-6,
        "apron top {top} against the wall's foot {foot}"
    );
    let bottom = strip.offset.y - strip.size.y * 0.5;
    assert!(
        (bottom + APRON_DEPTH_M).abs() < 1e-6,
        "apron bottom {bottom}"
    );
    // The post feet continue the posts, at the posts' own places.
    let [lo, hi] = corner_posts();
    for (foot, post) in [(apron[1], lo), (apron[2], hi)] {
        assert_eq!(foot.role, post.role);
        assert!((foot.offset.z - post.offset.z).abs() < 1e-6);
        assert!((foot.size.x - post.size.x).abs() < 1e-6);
        assert!(
            (foot.offset.y + foot.size.y * 0.5 - (post.offset.y - post.size.y * 0.5)).abs() < 1e-6
        );
    }
    // And on the base, under every ground-storey edge piece the drawn solid
    // continues without a break from the wall's head to the footing's depth.
    let r = base();
    let (boxes, _) = drawn(&r);
    let mut checked = 0;
    for rec in edge_recs(&r).into_iter().filter(|rec| rec.level == 0) {
        let (a, b) = edge_line(&rec);
        // A quarter of the way along: inside a wall's body and inside a
        // doorway's post, never in its opening.
        let mid = a.lerp(b, 0.25);
        let mut y = EDGE_DROP_M * 0.5;
        while y > -SKIRT_PROBE_M {
            // Both halves of the wall's thickness.
            for dx in [-WALL_THICKNESS_M * 0.4, WALL_THICKNESS_M * 0.4] {
                let d = (b - a).normalize();
                let side = Vec3::new(d.z, 0.0, -d.x) * dx;
                let p = mid + side + Vec3::Y * y;
                assert!(
                    boxes.iter().any(|bx| bx.contains(p, 1e-4)),
                    "piece {:?}: nothing drawn under the wall at {p:?}",
                    (rec.cx, rec.cz, rec.level, rec.loc)
                );
            }
            y -= 0.05;
            checked += 1;
        }
    }
    assert!(
        checked > 100,
        "only {checked} points under walls were checked"
    );
}

/// How far under a ground-storey wall the undercut probe reaches, metres —
/// well past any footing the base's flat-ish cells need, and short of the
/// apron's ceiling.
const SKIRT_PROBE_M: f32 = 1.0;
