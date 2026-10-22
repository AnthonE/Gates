//! The column index's own walks, kept as the oracle `hull.rs`'s shadow
//! test holds the hulls to: what every body, ground and shot query
//! answered before the hulls (freeform W1). Test-only; it goes when the
//! masks do.
#![allow(clippy::too_many_arguments, dead_code)]

use crate::build::{
    BUILD_CELL_M, LOC_DIAG_A, LOC_DIAG_B, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE, LOC_TRI_XHI_ZHI,
    LOC_TRI_XHI_ZLO, LOC_TRI_XLO_ZHI, LOC_TRI_XLO_ZLO,
};
use crate::collide::{
    col_base_y, doorway_solid_at, edge_foot_drop, floor_frame_solid, for_near_solids,
    frame_solid_at, tri_frame_solid, tri_half_has, window_solid_at, ColIndex, PieceHit,
    CAPSULE_HEIGHT_M, CAPSULE_RADIUS_M, DOOR_HEAD_M, FRAME_RIM_M, NO_SURFACE, PLANE_THICKNESS_M,
    WALL_THICKNESS_M,
};
use crate::fmath::fabs;
use crate::limits::{MAX_BUILD_COORD, MAX_BUILD_SOCKETS};
use crate::movement::STEP_UP;

const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;
const DIAG_LEN_M: f32 = BUILD_CELL_M * std::f32::consts::SQRT_2;

pub(crate) fn floor_under(
    seed: u64,
    haven: &crate::terrain::Haven,
    cols: &ColIndex,
    x: f32,
    z: f32,
    feet_y: f32,
) -> Option<(u16, u16, u8)> {
    // A hand, and a little over: another body's feet are an interpolated
    // transform, not the predictor's.
    const HAND: f32 = 0.25;
    let r = CAPSULE_RADIUS_M;
    for (px, pz) in [(x, z), (x - r, z), (x + r, z), (x, z - r), (x, z + r)] {
        let bx = crate::build::build_cell_of(px);
        let bz = crate::build::build_cell_of(pz);
        if bx < 0 || bz < 0 || bx >= MAX_BUILD_COORD as i32 || bz >= MAX_BUILD_COORD as i32 {
            continue;
        }
        let m = cols.get(bx as u16, bz as u16);
        let tris = m.tri_xlo_zlo | m.tri_xhi_zlo | m.tri_xlo_zhi | m.tri_xhi_zhi;
        let flats = m.planes | m.floor_frames | tris;
        let stairs = m.stairs | m.stairs_xhi | m.stairs_zlo | m.stairs_xlo;
        if flats | stairs == 0 {
            continue;
        }
        let base = col_base_y(seed, haven, cols, bx as u16, bz as u16);
        for level in 0..MAX_BUILD_SOCKETS {
            let bit = 1u16 << level;
            let y = base + crate::build::level_y(level as u8);
            let on_flat = flats & bit != 0 && fabs(y - feet_y) <= HAND;
            let on_stairs = stairs & bit != 0
                && feet_y > y - HAND
                && feet_y < y + crate::build::LEVEL_H_M + HAND;
            if on_flat || on_stairs {
                return Some((bx as u16, bz as u16, level as u8));
            }
        }
    }
    None
}

pub(crate) fn piece_ground(
    seed: u64,
    haven: &crate::terrain::Haven,
    cols: &ColIndex,
    x: f32,
    z: f32,
    feet_y: f32,
) -> f32 {
    let bx = crate::build::build_cell_of(x);
    let bz = crate::build::build_cell_of(z);
    if bx < 0 || bz < 0 || bx >= MAX_BUILD_COORD as i32 || bz >= MAX_BUILD_COORD as i32 {
        return NO_SURFACE;
    }
    let lid = feet_y + STEP_UP;
    // Solid-deploy tops are standable ground (deploy collision v0): the
    // reference's box-stair. The footprint is the volume's own — not
    // inflated by the capsule — so a body stands on a furnace only with
    // its centre over the furnace, the same rule a cell boundary already
    // applies to a floor slab. The lid keeps a tall top from teleporting
    // anyone up: a box (0.65) needs the jump, which clears it. Since free
    // placement a box may stand across a cell boundary, so its top is read
    // from the solid table around the point rather than off this column.
    let mut best = NO_SURFACE;
    for_near_solids(seed, haven, cols, x, z, 0.0, |rect, bottom, h, _| {
        let top = bottom + h;
        if rect.contains(x, z) && top <= lid && top > best {
            best = top;
        }
        false
    });
    let m = cols.get(bx as u16, bz as u16);
    let tris = m.tri_xlo_zlo | m.tri_xhi_zlo | m.tri_xlo_zhi | m.tri_xhi_zhi;
    if m.planes | m.floor_frames | m.stairs | m.stairs_xhi | m.stairs_zlo | m.stairs_xlo | tris == 0
    {
        return best;
    }
    let base = col_base_y(seed, haven, cols, bx as u16, bz as u16);
    // A triangle plane is ground over its own half of the cell and air
    // over the other (triangles v0) — [`tri_half_has`].
    let dx = x - bx as f32 * BUILD_CELL_M;
    let dz = z - bz as f32 * BUILD_CELL_M;
    let in_half = |loc: u8| tri_half_has(loc, dx, dz);
    for level in 0..MAX_BUILD_SOCKETS {
        let bit = 1u16 << level;
        let floor = base + crate::build::level_y(level as u8);
        if (m.planes & bit != 0 || (m.floor_frames & bit != 0 && floor_frame_solid(dx, dz, 0.0)))
            && floor <= lid
            && floor > best
        {
            best = floor;
        }
        if tris & bit != 0 && floor <= lid && floor > best {
            let on = (m.tri_xlo_zlo & bit != 0
                && in_half(LOC_TRI_XLO_ZLO)
                && (!m.tri_is_frame(level as u8, LOC_TRI_XLO_ZLO)
                    || tri_frame_solid(LOC_TRI_XLO_ZLO, dx, dz, 0.0)))
                || (m.tri_xhi_zlo & bit != 0
                    && in_half(LOC_TRI_XHI_ZLO)
                    && (!m.tri_is_frame(level as u8, LOC_TRI_XHI_ZLO)
                        || tri_frame_solid(LOC_TRI_XHI_ZLO, dx, dz, 0.0)))
                || (m.tri_xlo_zhi & bit != 0
                    && in_half(LOC_TRI_XLO_ZHI)
                    && (!m.tri_is_frame(level as u8, LOC_TRI_XLO_ZHI)
                        || tri_frame_solid(LOC_TRI_XLO_ZHI, dx, dz, 0.0)))
                || (m.tri_xhi_zhi & bit != 0
                    && in_half(LOC_TRI_XHI_ZHI)
                    && (!m.tri_is_frame(level as u8, LOC_TRI_XHI_ZHI)
                        || tri_frame_solid(LOC_TRI_XHI_ZHI, dx, dz, 0.0)));
            if on {
                best = floor;
            }
        }
        for (mask, loc) in m.stair_masks().into_iter().zip(crate::build::STAIR_LOCS) {
            if mask & bit != 0 {
                if let Some(y) =
                    crate::circulation::surface(m.riser_shape(level as u8), loc, dx, dz)
                {
                    let ramp = floor + y;
                    if ramp <= lid && ramp > best {
                        best = ramp;
                    }
                }
            }
        }
    }
    best
}

pub(crate) fn body_ground(
    seed: u64,
    haven: &crate::terrain::Haven,
    cols: &ColIndex,
    x: f32,
    z: f32,
    feet_y: f32,
) -> f32 {
    let lid = feet_y + STEP_UP;
    let mut best = NO_SURFACE;
    // Planes, frames, triangles and stair ramps: the walk `plane_blocked` and
    // `piece_ceiling` take, so a top is ground exactly where its flank and
    // its underside would be solid.
    any_body_plane(seed, haven, cols, x, z, |_, top| {
        if top <= lid && top > best {
            best = top;
        }
        false
    });
    // Solid-deploy tops at `deploy_blocked`'s footprint, closed where that
    // one is open, over the same walk.
    let r2 = CAPSULE_RADIUS_M * CAPSULE_RADIUS_M;
    for_near_solids(
        seed,
        haven,
        cols,
        x,
        z,
        CAPSULE_RADIUS_M,
        |rect, bottom, h, _| {
            let top = bottom + h;
            if top <= lid && top > best && rect.dist2(x, z) <= r2 {
                best = top;
            }
            false
        },
    );
    best
}

pub(crate) fn plane_blocked(
    seed: u64,
    haven: &crate::terrain::Haven,
    cols: &ColIndex,
    x: f32,
    z: f32,
    feet_y: f32,
) -> bool {
    let head = feet_y + CAPSULE_HEIGHT_M;
    any_body_plane(seed, haven, cols, x, z, |level, top| {
        feet_y + STEP_UP < top && (level == 0 || head > top - PLANE_THICKNESS_M)
    })
}

pub(crate) fn piece_ceiling(
    seed: u64,
    haven: &crate::terrain::Haven,
    cols: &ColIndex,
    x: f32,
    z: f32,
    feet_y: f32,
) -> Option<f32> {
    let head = feet_y + CAPSULE_HEIGHT_M;
    let mut lowest: Option<f32> = None;
    any_body_plane(seed, haven, cols, x, z, |level, top| {
        let underside = top - PLANE_THICKNESS_M;
        if level > 0 && head <= underside {
            lowest = Some(lowest.map_or(underside, |old| old.min(underside)));
        }
        false
    });
    lowest
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn blocked(
    seed: u64,
    haven: &crate::terrain::Haven,
    cols: &ColIndex,
    x: f32,
    z: f32,
    nx: f32,
    nz: f32,
    feet_y: f32,
) -> bool {
    let (bx0, bz0) = (
        crate::build::build_cell_of(x),
        crate::build::build_cell_of(z),
    );
    let (bx1, bz1) = (
        crate::build::build_cell_of(nx),
        crate::build::build_cell_of(nz),
    );
    if cell_edges_block(seed, haven, cols, bx1, bz1, x, z, nx, nz, feet_y) {
        return true;
    }
    if (bx0 != bx1 || bz0 != bz1)
        && cell_edges_block(seed, haven, cols, bx0, bz0, x, z, nx, nz, feet_y)
    {
        return true;
    }
    let (lo, hi) = (feet_y, feet_y + CAPSULE_HEIGHT_M);
    // `.is_some()`: the body walk asks only whether it is stopped. The
    // address the diagonal now names is the shot walk's business
    // ([`shot_stop`]) — a mover that cannot damage a wall has no use for it.
    if cell_diags_block(
        seed,
        haven,
        cols,
        bx1,
        bz1,
        x,
        z,
        nx,
        nz,
        lo,
        hi,
        false,
        CAPSULE_RADIUS_M,
    )
    .is_some()
    {
        return true;
    }
    if (bx0 != bx1 || bz0 != bz1)
        && cell_diags_block(
            seed,
            haven,
            cols,
            bx0,
            bz0,
            x,
            z,
            nx,
            nz,
            lo,
            hi,
            false,
            CAPSULE_RADIUS_M,
        )
        .is_some()
    {
        return true;
    }
    false
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn shot_hit(
    seed: u64,
    haven: &crate::terrain::Haven,
    cols: &ColIndex,
    x: f32,
    z: f32,
    nx: f32,
    nz: f32,
    y: f32,
    r: f32,
) -> Option<(PieceHit, bool)> {
    let (bx0, bz0) = (
        crate::build::build_cell_of(x),
        crate::build::build_cell_of(z),
    );
    let (bx1, bz1) = (
        crate::build::build_cell_of(nx),
        crate::build::build_cell_of(nz),
    );
    let hit = cell_edges_stop_shot(seed, haven, cols, bx1, bz1, x, z, nx, nz, y, r);
    if hit.is_some() {
        return hit;
    }
    if bx0 != bx1 || bz0 != bz1 {
        let hit = cell_edges_stop_shot(seed, haven, cols, bx0, bz0, x, z, nx, nz, y, r);
        if hit.is_some() {
            return hit;
        }
    }
    // The diagonals, with the point's own band: containment of `y`, not a
    // capsule's overlap.
    let hit = cell_diags_block(seed, haven, cols, bx1, bz1, x, z, nx, nz, y, y, true, r);
    if hit.is_some() {
        return hit.map(|at| (at, false));
    }
    if bx0 != bx1 || bz0 != bz1 {
        let hit = cell_diags_block(seed, haven, cols, bx0, bz0, x, z, nx, nz, y, y, true, r);
        if hit.is_some() {
            return hit.map(|at| (at, false));
        }
    }
    // The planes, at the sample's own point rather than over the sweep.
    // **A point, deliberately, where the edges above are swept**: an edge is
    // crossed horizontally and a plane is crossed vertically, and the
    // vertical step between two taps is at most `ARROW_STEP_MM` — under the
    // band a slab presents, so the sweep an edge needs a plane does not
    // ([`cell_planes_stop_shot`]'s doc carries the arithmetic). It reads the
    // destination `(nx, nz)`, which is the sample `ranged::world_stop` is
    // asking about and the same point it hands `Occupants::blocks_volume`.
    cell_planes_stop_shot(seed, haven, cols, nx, nz, y, r).map(|at| (at, false))
}

pub(crate) fn any_body_plane(
    seed: u64,
    haven: &crate::terrain::Haven,
    cols: &ColIndex,
    x: f32,
    z: f32,
    mut visit: impl FnMut(usize, f32) -> bool,
) -> bool {
    let r = CAPSULE_RADIUS_M;
    // **Up to FOUR cells, not one, and that is the difference from
    // [`deploy_blocked`].** That function is complete over the candidate's own
    // cell because `deploy::DEPLOY_VOL`'s const block proves no volume,
    // inflated by the capsule, reaches past the half-cell. A plane IS the
    // cell, so inflated by the capsule it reaches past every boundary — the
    // first draft looked only at `build_cell_of(x, z)` and let a body stand a
    // finger's width from a stilted plate, because the point was in the empty
    // cell NEXT to it. A 3 m cell against a 0.4 m radius means the reach is at
    // most one cell each way, so the corners `x ± r` and `z ± r` name every
    // cell that can matter and the pair collapses to one when the point is
    // not near a boundary.
    let (x0, x1) = (
        crate::build::build_cell_of(x - r),
        crate::build::build_cell_of(x + r),
    );
    let (z0, z1) = (
        crate::build::build_cell_of(z - r),
        crate::build::build_cell_of(z + r),
    );
    let half = BUILD_CELL_M * 0.5;
    for bx in x0..=x1 {
        for bz in z0..=z1 {
            if bx < 0 || bz < 0 || bx >= MAX_BUILD_COORD as i32 || bz >= MAX_BUILD_COORD as i32 {
                continue;
            }
            let m = cols.get(bx as u16, bz as u16);
            let tris = m.tri_xlo_zlo | m.tri_xhi_zlo | m.tri_xlo_zhi | m.tri_xhi_zhi;
            if m.planes
                | m.floor_frames
                | tris
                | m.stairs
                | m.stairs_xhi
                | m.stairs_zlo
                | m.stairs_xlo
                == 0
            {
                continue;
            }
            // The cell, at the capsule's radius: the clamp-to-rectangle circle
            // distance [`deploy_blocked`] uses, for its reason — growing the
            // rectangle by the radius rounds the corners the wrong way.
            let (cxm, czm) = (
                bx as f32 * BUILD_CELL_M + half,
                bz as f32 * BUILD_CELL_M + half,
            );
            let ex = x - cxm - (x - cxm).clamp(-half, half);
            let ez = z - czm - (z - czm).clamp(-half, half);
            if ex * ex + ez * ez >= r * r {
                continue;
            }
            let base = col_base_y(seed, haven, cols, bx as u16, bz as u16);
            let dx = x - bx as f32 * BUILD_CELL_M;
            let dz = z - bz as f32 * BUILD_CELL_M;
            // `piece_ground`'s own half tests, boundary-inclusive on both
            // sides for its reason: the seam of a NW+SE pair must read the
            // same from either.
            let in_half = |loc: u8| match loc {
                LOC_TRI_XLO_ZLO => dx + dz <= BUILD_CELL_M,
                LOC_TRI_XHI_ZHI => dx + dz >= BUILD_CELL_M,
                LOC_TRI_XHI_ZLO => dz <= dx,
                _ => dz >= dx,
            };
            for level in 0..MAX_BUILD_SOCKETS {
                let bit = 1u16 << level;
                let here = m.planes & bit != 0
                    || (m.floor_frames & bit != 0 && floor_frame_solid(dx, dz, r))
                    || (m.tri_xlo_zlo & bit != 0
                        && in_half(LOC_TRI_XLO_ZLO)
                        && (!m.tri_is_frame(level as u8, LOC_TRI_XLO_ZLO)
                            || tri_frame_solid(LOC_TRI_XLO_ZLO, dx, dz, r)))
                    || (m.tri_xhi_zlo & bit != 0
                        && in_half(LOC_TRI_XHI_ZLO)
                        && (!m.tri_is_frame(level as u8, LOC_TRI_XHI_ZLO)
                            || tri_frame_solid(LOC_TRI_XHI_ZLO, dx, dz, r)))
                    || (m.tri_xlo_zhi & bit != 0
                        && in_half(LOC_TRI_XLO_ZHI)
                        && (!m.tri_is_frame(level as u8, LOC_TRI_XLO_ZHI)
                            || tri_frame_solid(LOC_TRI_XLO_ZHI, dx, dz, r)))
                    || (m.tri_xhi_zhi & bit != 0
                        && in_half(LOC_TRI_XHI_ZHI)
                        && (!m.tri_is_frame(level as u8, LOC_TRI_XHI_ZHI)
                            || tri_frame_solid(LOC_TRI_XHI_ZHI, dx, dz, r)));
                let floor = base + crate::build::level_y(level as u8);
                if here && visit(level, floor) {
                    return true;
                }
                for (mask, loc) in m.stair_masks().into_iter().zip(crate::build::STAIR_LOCS) {
                    if mask & bit == 0 {
                        continue;
                    }
                    let shape = m.riser_shape(level as u8);
                    if let Some(y) = crate::circulation::surface(shape, loc, dx, dz) {
                        let slab_level = if shape == crate::build::SHAPE_FOUNDATION_STEPS {
                            0
                        } else {
                            level.max(1)
                        };
                        if visit(slab_level, floor + y) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

pub(crate) fn cell_planes_stop_shot(
    seed: u64,
    haven: &crate::terrain::Haven,
    cols: &ColIndex,
    x: f32,
    z: f32,
    y: f32,
    r: f32,
) -> Option<PieceHit> {
    // The same four-cell reach [`plane_blocked`] takes, and for its reason:
    // a plane IS the cell, so inflated by the mover's radius it can reach
    // past a boundary. The arrowhead is 0.05 m against a 3 m cell, so the
    // pair collapses to one except within a head's width of a seam.
    let (x0, x1) = (
        crate::build::build_cell_of(x - r),
        crate::build::build_cell_of(x + r),
    );
    let (z0, z1) = (
        crate::build::build_cell_of(z - r),
        crate::build::build_cell_of(z + r),
    );
    let half = BUILD_CELL_M * 0.5;
    for bx in x0..=x1 {
        for bz in z0..=z1 {
            if bx < 0 || bz < 0 || bx >= MAX_BUILD_COORD as i32 || bz >= MAX_BUILD_COORD as i32 {
                continue;
            }
            let m = cols.get(bx as u16, bz as u16);
            let tris = m.tri_xlo_zlo | m.tri_xhi_zlo | m.tri_xlo_zhi | m.tri_xhi_zhi;
            if m.planes
                | m.floor_frames
                | tris
                | m.stairs
                | m.stairs_xhi
                | m.stairs_zlo
                | m.stairs_xlo
                == 0
            {
                continue;
            }
            let (cxm, czm) = (
                bx as f32 * BUILD_CELL_M + half,
                bz as f32 * BUILD_CELL_M + half,
            );
            let ex = x - cxm - (x - cxm).clamp(-half, half);
            let ez = z - czm - (z - czm).clamp(-half, half);
            if ex * ex + ez * ez >= r * r {
                continue;
            }
            let base = col_base_y(seed, haven, cols, bx as u16, bz as u16);
            let dx = x - bx as f32 * BUILD_CELL_M;
            let dz = z - bz as f32 * BUILD_CELL_M;
            // `piece_ground`'s own half tests, boundary-inclusive on both
            // sides for its reason: the seam of a NW+SE pair must read the
            // same from either.
            let in_half = |loc: u8| match loc {
                LOC_TRI_XLO_ZLO => dx + dz <= BUILD_CELL_M,
                LOC_TRI_XHI_ZHI => dx + dz >= BUILD_CELL_M,
                LOC_TRI_XHI_ZLO => dz <= dx,
                _ => dz >= dx,
            };
            for level in 0..MAX_BUILD_SOCKETS {
                let bit = 1u16 << level;
                // Which slab, not just whether one — the address is what
                // `ranged.rs` charges structure damage against, and the OR
                // this replaced could not say.
                //
                // **The order is a real tie-break and not just insurance.**
                // A plane and a triangle cannot share a (cell, level) —
                // `build::body_overlaps` refuses that pair — but two
                // COMPLEMENTARY triangles can, and are meant to: NW+SE and
                // NE+SW are how two halves make a cell. Their `in_half`
                // tests are boundary-inclusive on both sides on purpose
                // (the comment above says why: the seam must read the same
                // from either), so a sample landing exactly on `dx + dz ==
                // BUILD_CELL_M` satisfies both halves and the first arm
                // wins. That is a whole answer either way — both halves are
                // real pieces standing at that point, and a raider on the
                // seam of a split floor has no claim about which one the
                // arrow found. What the sim does require is that the pick
                // be a rule rather than an accident, which an `||` chain
                // read for its address would not have been.
                let here = if m.planes & bit != 0
                    || (m.floor_frames & bit != 0 && floor_frame_solid(dx, dz, r))
                {
                    Some(LOC_PLANE)
                } else if m.tri_xlo_zlo & bit != 0
                    && in_half(LOC_TRI_XLO_ZLO)
                    && (!m.tri_is_frame(level as u8, LOC_TRI_XLO_ZLO)
                        || tri_frame_solid(LOC_TRI_XLO_ZLO, dx, dz, r))
                {
                    Some(LOC_TRI_XLO_ZLO)
                } else if m.tri_xhi_zlo & bit != 0
                    && in_half(LOC_TRI_XHI_ZLO)
                    && (!m.tri_is_frame(level as u8, LOC_TRI_XHI_ZLO)
                        || tri_frame_solid(LOC_TRI_XHI_ZLO, dx, dz, r))
                {
                    Some(LOC_TRI_XHI_ZLO)
                } else if m.tri_xlo_zhi & bit != 0
                    && in_half(LOC_TRI_XLO_ZHI)
                    && (!m.tri_is_frame(level as u8, LOC_TRI_XLO_ZHI)
                        || tri_frame_solid(LOC_TRI_XLO_ZHI, dx, dz, r))
                {
                    Some(LOC_TRI_XLO_ZHI)
                } else if m.tri_xhi_zhi & bit != 0
                    && in_half(LOC_TRI_XHI_ZHI)
                    && (!m.tri_is_frame(level as u8, LOC_TRI_XHI_ZHI)
                        || tri_frame_solid(LOC_TRI_XHI_ZHI, dx, dz, r))
                {
                    Some(LOC_TRI_XHI_ZHI)
                } else {
                    None
                };
                let floor = base + crate::build::level_y(level as u8);
                if let Some(loc) = here {
                    if y - r < floor && (level == 0 || y + r > floor - PLANE_THICKNESS_M) {
                        return Some(PieceHit {
                            cx: bx as u16,
                            cz: bz as u16,
                            level: level as u8,
                            loc,
                        });
                    }
                }
                for (mask, loc) in m.stair_masks().into_iter().zip(crate::build::STAIR_LOCS) {
                    if mask & bit == 0 {
                        continue;
                    }
                    let shape = m.riser_shape(level as u8);
                    if let Some(dy) = crate::circulation::surface(shape, loc, dx, dz) {
                        let top = floor + dy;
                        if y - r < top
                            && (shape == crate::build::SHAPE_FOUNDATION_STEPS
                                || y + r > top - PLANE_THICKNESS_M)
                        {
                            return Some(PieceHit {
                                cx: bx as u16,
                                cz: bz as u16,
                                level: level as u8,
                                loc,
                            });
                        }
                    }
                }
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cell_edges_block(
    seed: u64,
    haven: &crate::terrain::Haven,
    cols: &ColIndex,
    bx: i32,
    bz: i32,
    x: f32,
    z: f32,
    nx: f32,
    nz: f32,
    feet_y: f32,
) -> bool {
    // (column cx, cz, x-plane?) — the four boundaries of cell (bx, bz).
    let edges = [
        (bx, bz, true),
        (bx + 1, bz, true),
        (bx, bz, false),
        (bx, bz + 1, false),
    ];
    for (ecx, ecz, x_plane) in edges {
        if ecx < 0 || ecz < 0 || ecx >= MAX_BUILD_COORD as i32 || ecz >= MAX_BUILD_COORD as i32 {
            continue;
        }
        let m = cols.get(ecx as u16, ecz as u16);
        // Windows ride the wall mask here on purpose: to a moving body a
        // window IS a wall (the aperture is above a capsule's reach and
        // under its head — `WINDOW_SILL_M`'s doc). A frame blocks only its
        // thin rim jambs, and its top beam not at all — the lintel
        // precedent: nothing at capsule height until a jump exists. The
        // shot walk (`shot_blocked`) is where the shapes fully diverge.
        let (walls, doors, frames, shuts) = if x_plane {
            (
                m.walls_xlo | m.half_xlo | m.low_xlo | m.wins_xlo,
                m.doors_xlo,
                m.frames_xlo,
                m.shut_xlo,
            )
        } else {
            (
                m.walls_zlo | m.half_zlo | m.low_zlo | m.wins_zlo,
                m.doors_zlo,
                m.frames_zlo,
                m.shut_zlo,
            )
        };
        if walls | doors | frames == 0 {
            continue;
        }
        let base = col_base_y(seed, haven, cols, ecx as u16, ecz as u16);
        for level in 0..MAX_BUILD_SOCKETS {
            let bit = 1u16 << level;
            let has_wall = walls & bit != 0;
            let has_door = doors & bit != 0;
            let has_frame = frames & bit != 0;
            if !has_wall && !has_door && !has_frame {
                continue;
            }
            let bottom = base + crate::build::level_y(level as u8);
            let foot = bottom
                - edge_foot_drop(
                    seed,
                    haven,
                    cols,
                    ecx as u16,
                    ecz as u16,
                    level as u8,
                    if x_plane { LOC_EDGE_XLO } else { LOC_EDGE_ZLO },
                    m.plate,
                );
            if feet_y
                >= bottom
                    + m.edge_height(
                        if x_plane { LOC_EDGE_XLO } else { LOC_EDGE_ZLO },
                        level as u8,
                    )
                || feet_y + CAPSULE_HEIGHT_M <= foot
            {
                continue; // that storey is above the head or below the feet
            }
            let meet = if x_plane {
                // x-plane at ecx·3, spanning z from ecz·3.
                let px = ecx as f32 * BUILD_CELL_M;
                let s0 = ecz as f32 * BUILD_CELL_M;
                edge_meet(px, s0, x, nx, nz, CAPSULE_RADIUS_M)
            } else {
                // z-plane at ecz·3, spanning x from ecx·3.
                let pz = ecz as f32 * BUILD_CELL_M;
                let s0 = ecx as f32 * BUILD_CELL_M;
                edge_meet(pz, s0, z, nz, nx, CAPSULE_RADIUS_M)
            };
            let Some(t) = meet else { continue };
            let hit = if has_wall || (feet_y < bottom && foot < bottom) || shuts & bit != 0 {
                (0.0..=BUILD_CELL_M).contains(&t)
            } else if has_door {
                // A closed door seals the doorway: full span, like a wall.
                if shuts & bit != 0 {
                    (0.0..=BUILD_CELL_M).contains(&t)
                } else {
                    doorway_solid_at(t)
                }
            } else {
                // The frame's jambs, at any capsule height in the storey.
                (0.0..=FRAME_RIM_M).contains(&t)
                    || (BUILD_CELL_M - FRAME_RIM_M..=BUILD_CELL_M).contains(&t)
            };
            if hit {
                return true;
            }
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cell_diags_block(
    seed: u64,
    haven: &crate::terrain::Haven,
    cols: &ColIndex,
    bx: i32,
    bz: i32,
    x: f32,
    z: f32,
    nx: f32,
    nz: f32,
    lo_y: f32,
    hi_y: f32,
    point: bool,
    r_extra: f32,
) -> Option<PieceHit> {
    if bx < 0 || bz < 0 || bx >= MAX_BUILD_COORD as i32 || bz >= MAX_BUILD_COORD as i32 {
        return None;
    }
    let m = cols.get(bx as u16, bz as u16);
    if m.diag_a | m.diag_b | m.half_diag_a | m.half_diag_b | m.low_diag_a | m.low_diag_b == 0 {
        return None;
    }
    let base = col_base_y(seed, haven, cols, bx as u16, bz as u16);
    for level in 0..MAX_BUILD_SOCKETS {
        let bit = 1u16 << level;
        if (m.diag_a | m.diag_b | m.half_diag_a | m.half_diag_b | m.low_diag_a | m.low_diag_b) & bit
            == 0
        {
            continue;
        }
        let bottom = base + crate::build::level_y(level as u8);
        for (mask, diag_b) in [
            (m.diag_a | m.half_diag_a | m.low_diag_a, false),
            (m.diag_b | m.half_diag_b | m.low_diag_b, true),
        ] {
            let top =
                bottom + m.edge_height(if diag_b { LOC_DIAG_B } else { LOC_DIAG_A }, level as u8);
            let outside = if point {
                lo_y < bottom || lo_y >= top
            } else {
                lo_y >= top || hi_y <= bottom
            };
            if mask & bit == 0 || outside {
                continue;
            }
            if let Some(t) = diag_meet(bx, bz, diag_b, x, z, nx, nz, r_extra) {
                if (0.0..=DIAG_LEN_M).contains(&t) {
                    return Some(PieceHit {
                        cx: bx as u16,
                        cz: bz as u16,
                        level: level as u8,
                        loc: if diag_b { LOC_DIAG_B } else { LOC_DIAG_A },
                    });
                }
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cell_edges_stop_shot(
    seed: u64,
    haven: &crate::terrain::Haven,
    cols: &ColIndex,
    bx: i32,
    bz: i32,
    x: f32,
    z: f32,
    nx: f32,
    nz: f32,
    y: f32,
    r: f32,
) -> Option<(PieceHit, bool)> {
    let edges = [
        (bx, bz, true),
        (bx + 1, bz, true),
        (bx, bz, false),
        (bx, bz + 1, false),
    ];
    for (ecx, ecz, x_plane) in edges {
        if ecx < 0 || ecz < 0 || ecx >= MAX_BUILD_COORD as i32 || ecz >= MAX_BUILD_COORD as i32 {
            continue;
        }
        let m = cols.get(ecx as u16, ecz as u16);
        let (walls, doors, wins, frames, shuts) = if x_plane {
            (
                m.walls_xlo | m.half_xlo | m.low_xlo,
                m.doors_xlo,
                m.wins_xlo,
                m.frames_xlo,
                m.shut_xlo,
            )
        } else {
            (
                m.walls_zlo | m.half_zlo | m.low_zlo,
                m.doors_zlo,
                m.wins_zlo,
                m.frames_zlo,
                m.shut_zlo,
            )
        };
        if walls | doors | wins | frames == 0 {
            continue;
        }
        let base = col_base_y(seed, haven, cols, ecx as u16, ecz as u16);
        for level in 0..MAX_BUILD_SOCKETS {
            let bit = 1u16 << level;
            if (walls | doors | wins | frames) & bit == 0 {
                continue;
            }
            let bottom = base + crate::build::level_y(level as u8);
            // A point is inside exactly one storey; half-open so the
            // boundary altitude resolves the same on both targets.
            let foot = bottom
                - edge_foot_drop(
                    seed,
                    haven,
                    cols,
                    ecx as u16,
                    ecz as u16,
                    level as u8,
                    if x_plane { LOC_EDGE_XLO } else { LOC_EDGE_ZLO },
                    m.plate,
                );
            if y < foot
                || y >= bottom
                    + m.edge_height(
                        if x_plane { LOC_EDGE_XLO } else { LOC_EDGE_ZLO },
                        level as u8,
                    )
            {
                continue;
            }
            let meet = if x_plane {
                let px = ecx as f32 * BUILD_CELL_M;
                let s0 = ecz as f32 * BUILD_CELL_M;
                edge_meet(px, s0, x, nx, nz, r)
            } else {
                let pz = ecz as f32 * BUILD_CELL_M;
                let s0 = ecx as f32 * BUILD_CELL_M;
                edge_meet(pz, s0, z, nz, nx, r)
            };
            let Some(t) = meet else { continue };
            let span = (0.0..=BUILD_CELL_M).contains(&t);
            let solid = span
                && (y < bottom
                    || if walls & bit != 0 {
                        true
                    } else if doors & bit != 0 {
                        doorway_solid_at(t) || y - bottom >= DOOR_HEAD_M
                    } else if wins & bit != 0 {
                        window_solid_at(t, y - bottom)
                    } else {
                        frame_solid_at(t, y - bottom)
                    });
            let arch = if doors & bit != 0 {
                crate::deploy::ARCH_DOOR
            } else if wins & bit != 0 {
                let panes = if x_plane { m.panes_xlo } else { m.panes_zlo };
                if panes & bit != 0 {
                    crate::deploy::ARCH_WINDOW_GLASS
                } else {
                    crate::deploy::ARCH_WINDOW_BARS
                }
            } else {
                crate::deploy::ARCH_GARAGE_DOOR
            };
            let insert = !solid
                && shuts & bit != 0
                && crate::deploy::insert_solid_at(arch, t, y - bottom, r);
            if solid || insert {
                return Some((
                    PieceHit {
                        cx: ecx as u16,
                        cz: ecz as u16,
                        level: level as u8,
                        loc: if x_plane { LOC_EDGE_XLO } else { LOC_EDGE_ZLO },
                    },
                    insert,
                ));
            }
        }
    }
    None
}

pub(crate) fn edge_meet(
    px: f32,
    s0: f32,
    a0: f32,
    a1: f32,
    along: f32,
    r_extra: f32,
) -> Option<f32> {
    let r = WALL_THICKNESS_M * 0.5 + r_extra;
    let d0 = a0 - px;
    let d1 = a1 - px;
    let crosses = (d0 < 0.0) != (d1 < 0.0);
    let pushes_in = fabs(d1) < r && fabs(d1) < fabs(d0);
    if !crosses && !pushes_in {
        return None;
    }
    Some(along - s0)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn diag_meet(
    bx: i32,
    bz: i32,
    diag_b: bool,
    x: f32,
    z: f32,
    nx: f32,
    nz: f32,
    r_extra: f32,
) -> Option<f32> {
    let x0 = bx as f32 * BUILD_CELL_M;
    let z0 = bz as f32 * BUILD_CELL_M;
    let (d0, d1, along) = if diag_b {
        // Line dx + dz = cell: distance grows toward SE.
        let d = |px: f32, pz: f32| ((px - x0) + (pz - z0) - BUILD_CELL_M) * INV_SQRT2;
        let s = ((nx - x0) - (nz - z0) + BUILD_CELL_M) * INV_SQRT2;
        (d(x, z), d(nx, nz), s)
    } else {
        // Line dz = dx: distance grows toward SW.
        let d = |px: f32, pz: f32| ((pz - z0) - (px - x0)) * INV_SQRT2;
        let s = ((nx - x0) + (nz - z0)) * INV_SQRT2;
        (d(x, z), d(nx, nz), s)
    };
    let r = WALL_THICKNESS_M * 0.5 + r_extra;
    // Half-open, [`edge_meet`]'s rule and reason: on the line is one side.
    let crosses = (d0 < 0.0) != (d1 < 0.0);
    let pushes_in = fabs(d1) < r && fabs(d1) < fabs(d0);
    if !crosses && !pushes_in {
        return None;
    }
    Some(along)
}
