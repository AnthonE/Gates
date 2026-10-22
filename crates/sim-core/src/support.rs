//! What holds a piece up and what may share its space, on its building's
//! lattice (freeform W3).
//!
//! Every rule here is an exact lookup: a footprint's sides are edge keys
//! (`sock::Sock::sides`), the bodies on either side of an edge are keys
//! (`sock::beside`), and "is it taken" is `build::Pieces::find_addr`. So a
//! wall on a triangle's side is borne by that triangle exactly as a wall on
//! a square's side is borne by the square, and nothing depends on the grid.
//!
//! [`supported`] and [`dependents`] are one rule read two ways — the second
//! lists every address whose support the first reads at a piece — and must
//! change together; `build`'s collapse cascade walks [`dependents`], the
//! support sweep asks [`supported`], and the naive fixed-point test holds
//! the pair to each other.

use crate::build::*;
use crate::lattice::{overlaps, LPt};
use crate::sock::{
    beside, PieceAddr, Sock, FOOT_EDGE, FOOT_RISER, FOOT_SQUARE, FOOT_TRI, FOOT_TRI_RISER,
};

/// The footprint kind a shape takes.
pub const fn foot_of(shape: u8) -> u8 {
    match shape {
        SHAPE_FOUNDATION | SHAPE_FLOOR | SHAPE_ROOF | SHAPE_FLOOR_FRAME => FOOT_SQUARE,
        SHAPE_TRI_FOUNDATION | SHAPE_TRI_FLOOR | SHAPE_TRI_ROOF | SHAPE_TRI_FLOOR_FRAME => FOOT_TRI,
        SHAPE_STAIRS_TRI_SPIRAL => FOOT_TRI_RISER,
        SHAPE_STAIRS
        | SHAPE_FOUNDATION_STEPS
        | SHAPE_RAMP
        | SHAPE_STAIRS_L
        | SHAPE_STAIRS_U
        | SHAPE_STAIRS_SPIRAL => FOOT_RISER,
        _ => FOOT_EDGE,
    }
}

/// Foundations, and the flight that is one: borne by the ground.
pub const fn grounded(shape: u8) -> bool {
    matches!(
        shape,
        SHAPE_FOUNDATION | SHAPE_TRI_FOUNDATION | SHAPE_FOUNDATION_STEPS
    )
}

/// A body with no hole in it: what a flight may stand on.
const fn solid_body(shape: u8) -> bool {
    matches!(
        shape,
        SHAPE_FOUNDATION
            | SHAPE_FLOOR
            | SHAPE_ROOF
            | SHAPE_TRI_FOUNDATION
            | SHAPE_TRI_FLOOR
            | SHAPE_TRI_ROOF
    )
}

/// The shape of the piece at `a`, if one stands there.
#[inline]
fn shape_at(p: &Pieces, bc: &BuildContent, a: &PieceAddr) -> Option<u8> {
    p.find_addr(a)
        .map(|i| bc.pieces[p.entries()[i].row as usize].shape)
}

#[inline]
fn at(bld: u16, level: u8, sock: Sock) -> PieceAddr {
    PieceAddr { bld, level, sock }
}

/// Does an edge piece below bear a socket on `edge` at `level` — a half
/// wall half a storey down, or a full-height edge (not a low wall) a whole
/// storey down?
fn edge_bears(p: &Pieces, bc: &BuildContent, bld: u16, level: u8, edge: Sock) -> bool {
    if let Some(below) = level_step(level, -1) {
        if shape_at(p, bc, &at(bld, below, edge)) == Some(SHAPE_HALF_WALL) {
            return true;
        }
    }
    level_step(level, -2).is_some_and(|below| {
        shape_at(p, bc, &at(bld, below, edge))
            .is_some_and(|s| !matches!(s, SHAPE_HALF_WALL | SHAPE_LOW_WALL))
    })
}

/// The flight on the square on `side` of `edge` that climbs toward it: the
/// one whose HIGH side the edge is.
fn climbs_to(edge: &Sock, side: i32) -> Sock {
    let (sq, _) = beside(edge, side);
    // The square lies along `n` from the edge; the edge along `n + 6` from
    // it.
    let n = if side == 1 {
        edge.dir + 3
    } else {
        edge.dir + 9
    };
    Sock::riser(sq.at, n + 6)
}

/// Does a body at `level` have `edge` for a side — or a stepped foundation
/// climb to it?
fn edge_borne_beside(p: &Pieces, bc: &BuildContent, bld: u16, level: u8, edge: Sock) -> bool {
    for side in [1, -1] {
        let (sq, tri) = beside(&edge, side);
        if p.find_addr(&at(bld, level, sq)).is_some() || p.find_addr(&at(bld, level, tri)).is_some()
        {
            return true;
        }
        if shape_at(p, bc, &at(bld, level, climbs_to(&edge, side))) == Some(SHAPE_FOUNDATION_STEPS)
        {
            return true;
        }
    }
    false
}

/// The flight directions a body's footprint takes: a square's four, a
/// triangle's three turns.
pub fn flights(body: &Sock) -> ([Sock; 4], usize) {
    let mut out = [Sock::default(); 4];
    if body.foot == FOOT_TRI {
        for (k, o) in out.iter_mut().take(3).enumerate() {
            *o = Sock::tri_riser(body.at, body.dir + 4 * k as u8);
        }
        (out, 3)
    } else {
        for (k, o) in out.iter_mut().enumerate() {
            *o = Sock::riser(body.at, body.dir + 3 * k as u8);
        }
        (out, 4)
    }
}

/// Support for a piece of `shape` at `a` (support v0's binary rule on the
/// lattice). Foundations carry none — terrain is their check.
pub fn supported(p: &Pieces, bc: &BuildContent, shape: u8, a: &PieceAddr) -> bool {
    if grounded(shape) {
        return true;
    }
    let (bld, level, sock) = (a.bld, a.level, a.sock);
    match sock.foot {
        // A floor, roof or frame: an edge piece under any of its sides.
        FOOT_SQUARE | FOOT_TRI => {
            let (sides, n) = sock.sides();
            level >= 1 && sides[..n].iter().any(|&e| edge_bears(p, bc, bld, level, e))
        }
        // A flight: a solid body under it, or (a spiral) the flight it
        // continues half a storey down.
        FOOT_RISER | FOOT_TRI_RISER => {
            let body = sock.body();
            let stacked = matches!(shape, SHAPE_STAIRS_SPIRAL | SHAPE_STAIRS_TRI_SPIRAL)
                && level_step(level, -1).is_some_and(|below| {
                    let (fl, n) = flights(&body);
                    fl[..n].iter().any(|&f| {
                        (shape == SHAPE_STAIRS_TRI_SPIRAL || f.dir == (sock.dir + 6) % 12)
                            && shape_at(p, bc, &at(bld, below, f)) == Some(shape)
                    })
                });
            stacked || shape_at(p, bc, &at(bld, level, body)).is_some_and(solid_body)
        }
        // An edge: a body beside it at its own level, or an edge below
        // whose top is its bottom.
        _ => edge_borne_beside(p, bc, bld, level, sock) || edge_bears(p, bc, bld, level, sock),
    }
}

/// Most addresses one piece's support can be read at: an edge's stacked
/// edge and four bodies, at each of two heights.
pub const MAX_DEPENDENTS: usize = 10;

/// Every address whose [`supported`] reads the piece at `a` — a superset,
/// in a fixed order (the collapse cascade's order).
pub fn dependents(a: &PieceAddr, out: &mut [PieceAddr; MAX_DEPENDENTS]) -> usize {
    let (bld, level, sock) = (a.bld, a.level, a.sock);
    let mut n = 0;
    let mut push = |x: PieceAddr, n: &mut usize| {
        out[*n] = x;
        *n += 1;
    };
    match sock.foot {
        // A body: the edges on its sides read it (`edge_borne_beside`), the
        // flights on it read it (`supported`'s body probe).
        FOOT_SQUARE | FOOT_TRI => {
            let (sides, k) = sock.sides();
            for &e in &sides[..k] {
                push(at(bld, level, e), &mut n);
            }
            // Room for the flights: a square has four sides and four
            // flights, which is eight; a triangle three and three.
            let (fl, m) = flights(&sock);
            for &f in &fl[..m] {
                if n < MAX_DEPENDENTS {
                    push(at(bld, level, f), &mut n);
                }
            }
        }
        // A flight: stepped foundations bear their high side; a spiral
        // bears the flight that continues it half a storey up.
        FOOT_RISER | FOOT_TRI_RISER => {
            if sock.foot == FOOT_RISER {
                let edge = Sock::edge(
                    sock.at.add(LPt::new(3, 0, 0, 0).rot(sock.dir)),
                    sock.dir + 3,
                );
                push(at(bld, level, edge), &mut n);
            }
            if let Some(up) = level_step(level, 1) {
                let (fl, m) = flights(&sock.body());
                for &f in &fl[..m] {
                    push(at(bld, up, f), &mut n);
                }
            }
        }
        // An edge: at each height its top can be, the edge stacked on it
        // and the bodies it is a side of.
        _ => {
            for halves in [1, 2] {
                let Some(up) = level_step(level, halves) else {
                    continue;
                };
                push(at(bld, up, sock), &mut n);
                for side in [1, -1] {
                    let (sq, tri) = beside(&sock, side);
                    push(at(bld, up, sq), &mut n);
                    push(at(bld, up, tri), &mut n);
                }
            }
        }
    }
    n
}

/// The footprint a piece takes at its own level for overlap: a body's
/// corners, a flight's body, an edge's two ends.
pub fn outline(sock: &Sock) -> ([LPt; 4], usize) {
    sock.corners()
}

/// Do two footprints of one building at one level overlap in area — or an
/// edge cross a body? Exact (`lattice::overlaps`); sharing a side or a
/// corner is not overlapping.
pub fn crosses(a: &Sock, b: &Sock) -> bool {
    let (ca, na) = outline(a);
    let (cb, nb) = outline(b);
    overlaps(&ca[..na], &cb[..nb])
}

/// Whether a footprint is one the building's lattice can hold: its
/// canonical direction, every corner on a 3 m vertex of the frame (every
/// coefficient a multiple of six, the origin being a vertex), all inside
/// the coefficient bound.
pub fn on_lattice(s: &Sock) -> bool {
    let canonical = match s.foot {
        FOOT_SQUARE => s.dir < 3,
        FOOT_TRI => s.dir < 4,
        FOOT_EDGE => s.dir < 6,
        FOOT_RISER | FOOT_TRI_RISER => s.dir < 12,
        _ => false,
    };
    let (c, n) = s.corners();
    canonical
        && c[..n]
            .iter()
            .all(|p| p.fits() && p.a % 6 == 0 && p.b % 6 == 0 && p.c % 6 == 0 && p.d % 6 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_footprint_is_on_the_lattice_only_on_its_vertices() {
        assert!(on_lattice(&Sock::square_from(LPt::ZERO, 0)));
        assert!(on_lattice(&Sock::tri_from(LPt::edge(4), 7)));
        assert!(on_lattice(&Sock::edge_from(LPt::edge(1), 5)));
        // Half a side off.
        assert!(!on_lattice(&Sock::square_from(LPt::new(3, 0, 0, 0), 0)));
        // Not canonical.
        let mut s = Sock::square_from(LPt::ZERO, 0);
        s.dir = 4;
        assert!(!on_lattice(&s));
    }

    /// The flight that climbs to an edge has that edge for its high side.
    #[test]
    fn a_flight_climbs_to_its_high_side() {
        let body = Sock::square_from(LPt::edge(2), 2);
        let (sides, n) = body.sides();
        for e in &sides[..n] {
            let side = e.side_of(body.at);
            let f = climbs_to(e, side);
            assert_eq!(f.body(), body);
            let edge = Sock::edge(f.at.add(LPt::new(3, 0, 0, 0).rot(f.dir)), f.dir + 3);
            assert_eq!(edge, *e);
        }
    }
}
