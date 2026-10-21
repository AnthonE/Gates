//! Which part of its building's tiling a piece takes (freeform W1): a
//! square, an equilateral triangle or one 3 m edge, as an exact key in the
//! building's lattice (`lattice.rs`).
//!
//! A footprint is named by one point and one direction:
//!
//! - a **square** by its centroid and the direction of one side, mod 3
//!   (a quarter turn is the same square);
//! - a **triangle** by its centroid and the direction of the side that
//!   starts at its "first" corner, mod 4 (a third of a turn is the same
//!   triangle);
//! - an **edge** by its midpoint and its direction, mod 6 (a half turn is
//!   the same edge).
//!
//! Every one of those points lies in Z[ζ]/2, so two pieces take the same
//! footprint exactly when their keys are equal, and what stands beside an
//! edge is an integer lookup ([`beside`]).

use crate::lattice::{square, triangle, LPt};

/// A square body: foundation, floor, roof, floor frame, a square flight.
pub const FOOT_SQUARE: u8 = 0;
/// An equilateral triangle body.
pub const FOOT_TRI: u8 = 1;
/// One 3 m edge: the wall family.
pub const FOOT_EDGE: u8 = 2;

/// Half a side, along a side: 1.5 m in the lattice's half-metre units.
const HALF_SIDE: LPt = LPt::new(3, 0, 0, 0);
/// A triangle's apothem, √3/2 m, along +x: √3 = ζ + ζ⁻¹ = 2ζ − ζ³.
const TRI_APOTHEM: LPt = LPt::new(0, 2, 0, -1);

/// One footprint in a building's lattice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sock {
    pub foot: u8,
    /// Centroid of a body, midpoint of an edge.
    pub at: LPt,
    /// Canonical: mod 3 for a square, mod 4 for a triangle, mod 6 for an
    /// edge.
    pub dir: u8,
}

/// `v` along direction `k` (ζᵏ).
const fn along(v: LPt, k: u8) -> LPt {
    v.rot(k % 12)
}

impl Sock {
    /// The square whose centroid is `at`, one side along `dir`.
    pub const fn square(at: LPt, dir: u8) -> Sock {
        Sock {
            foot: FOOT_SQUARE,
            at,
            dir: dir % 3,
        }
    }

    /// The triangle whose centroid is `at` and whose corners are
    /// `lattice::triangle(start, dir)` for the start that puts the centroid
    /// there.
    pub const fn tri(at: LPt, dir: u8) -> Sock {
        Sock {
            foot: FOOT_TRI,
            at,
            dir: dir % 4,
        }
    }

    /// The edge whose midpoint is `at`, running along `dir`.
    pub const fn edge(at: LPt, dir: u8) -> Sock {
        Sock {
            foot: FOOT_EDGE,
            at,
            dir: dir % 6,
        }
    }

    /// The square with corners `lattice::square(start, dir)`.
    pub const fn square_from(start: LPt, dir: u8) -> Sock {
        let c = start
            .add(along(HALF_SIDE, dir))
            .add(along(HALF_SIDE, dir + 3));
        Sock::square(c, dir)
    }

    /// The triangle with corners `lattice::triangle(start, dir)`: its
    /// centroid is `start + (e(dir) + e(dir + 2)) / 3`, exactly, because a
    /// side is six half-metres.
    pub const fn tri_from(start: LPt, dir: u8) -> Sock {
        let e = LPt::edge(dir % 12);
        let f = LPt::edge((dir + 2) % 12);
        let s = e.add(f);
        let c = start.add(LPt::new(s.a / 3, s.b / 3, s.c / 3, s.d / 3));
        Sock::tri(c, dir)
    }

    /// The edge from `start` along `dir`.
    pub const fn edge_from(start: LPt, dir: u8) -> Sock {
        Sock::edge(start.add(along(HALF_SIDE, dir)), dir)
    }

    pub const fn is_edge(&self) -> bool {
        self.foot == FOOT_EDGE
    }

    /// A body's corners, in order round it (anticlockwise, +x toward +z),
    /// or an edge's two ends. The count is live.
    pub const fn corners(&self) -> ([LPt; 4], usize) {
        match self.foot {
            FOOT_SQUARE => {
                let start = self
                    .at
                    .sub(along(HALF_SIDE, self.dir))
                    .sub(along(HALF_SIDE, self.dir + 3));
                (square(start, self.dir), 4)
            }
            FOOT_TRI => {
                let e = LPt::edge(self.dir);
                let f = LPt::edge(self.dir + 2);
                let s = e.add(f);
                let start = self.at.sub(LPt::new(s.a / 3, s.b / 3, s.c / 3, s.d / 3));
                let t = triangle(start, self.dir);
                ([t[0], t[1], t[2], LPt::ZERO], 3)
            }
            _ => {
                let h = along(HALF_SIDE, self.dir);
                ([self.at.sub(h), self.at.add(h), LPt::ZERO, LPt::ZERO], 2)
            }
        }
    }

    /// A body's sides as edge sockets, in corner order. An edge has none.
    pub const fn sides(&self) -> ([Sock; 4], usize) {
        let (c, n) = self.corners();
        let mut out = [Sock::edge(LPt::ZERO, 0); 4];
        if self.foot == FOOT_EDGE {
            return (out, 0);
        }
        let mut i = 0;
        while i < n {
            out[i] = edge_between(c[i], c[(i + 1) % n]);
            i += 1;
        }
        (out, n)
    }

    /// Exact: does this footprint have `edge` as one of its sides?
    pub fn has_side(&self, edge: &Sock) -> bool {
        let (s, n) = self.sides();
        s[..n].iter().any(|e| e == edge)
    }

    /// Which side of this edge a lattice point lies on: 1 on the
    /// anticlockwise side (direction `dir + 3`), −1 on the other, 0 on the
    /// line. Exact.
    pub const fn side_of(&self, p: LPt) -> i32 {
        crate::lattice::cmp_along(p, self.at, (self.dir + 3) % 12)
    }
}

/// The edge socket between two adjacent corners.
pub const fn edge_between(p: LPt, q: LPt) -> Sock {
    let d = q.sub(p);
    let mut k = 0u8;
    while k < 12 {
        let e = LPt::edge(k);
        if e.a == d.a && e.b == d.b && e.c == d.c && e.d == d.d {
            break;
        }
        k += 1;
    }
    debug_assert!(k < 12, "corners are not one side apart");
    // Midpoint: corners of a building are sums of 3 m sides, so every
    // coefficient is a multiple of six and the halving is exact.
    let s = p.add(q);
    Sock::edge(LPt::new(s.a / 2, s.b / 2, s.c / 2, s.d / 2), k)
}

/// The two bodies that can stand on `side` of an edge: 1 is the
/// anticlockwise side (direction `edge.dir + 3`), anything else the other.
/// Returns (square, triangle), each the one footprint of that kind which
/// has the edge as a side and lies on that side of it.
pub const fn beside(edge: &Sock, side: i32) -> (Sock, Sock) {
    // The normal pointing into the body.
    let n = if side == 1 {
        edge.dir + 3
    } else {
        edge.dir + 9
    };
    let sq = Sock::square(edge.at.add(along(HALF_SIDE, n)), edge.dir);
    let c = edge.at.add(along(TRI_APOTHEM, n));
    // The triangle whose first side runs along the edge with the interior
    // on its anticlockwise side: from the edge's start (as seen walking
    // with the interior on the left), direction `n − 3`.
    let d = (n + 9) % 12;
    let tri = Sock::tri(c, d);
    (sq, tri)
}

/// The square address a cell takes in a building whose lattice origin is
/// the low corner of cell (`ox`, `oz`), facing +x: the bridge from today's
/// grid addresses. `None` when the cell is behind the origin past what
/// the lattice holds.
pub fn grid_cell(ox: u16, oz: u16, cx: u16, cz: u16) -> Option<LPt> {
    let dx = cx as i32 - ox as i32;
    let dz = cz as i32 - oz as i32;
    let p = LPt::new(6 * dx, 0, 0, 6 * dz);
    p.fits().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::overlaps;

    fn close(p: LPt, x: f32, z: f32) -> bool {
        let (px, pz) = p.to_xz();
        (px - x).abs() < 1e-4 && (pz - z).abs() < 1e-4
    }

    #[test]
    fn a_square_is_where_its_corners_are() {
        for dir in 0..12 {
            let start = LPt::edge((dir + 5) % 12).add(LPt::edge(7));
            let s = Sock::square_from(start, dir);
            let (c, n) = s.corners();
            assert_eq!(n, 4);
            let want = square(start, dir);
            let mut got: Vec<LPt> = c[..4].to_vec();
            let mut want: Vec<LPt> = want.to_vec();
            got.sort();
            want.sort();
            assert_eq!(got, want, "dir {dir}");
        }
        // The cell at the origin: centroid (1.5, 1.5) m.
        let s = Sock::square_from(LPt::ZERO, 0);
        assert!(close(s.at, 1.5, 1.5));
        assert_eq!(s, Sock::square_from(LPt::edge(0), 3));
        assert_eq!(s, Sock::square_from(LPt::edge(0).add(LPt::edge(3)), 6));
    }

    #[test]
    fn a_triangle_is_one_triangle_whichever_corner_names_it() {
        for dir in 0..12u8 {
            let start = LPt::edge((dir * 5) % 12);
            let t = Sock::tri_from(start, dir);
            let (c, n) = t.corners();
            assert_eq!(n, 3);
            let mut got = c[..3].to_vec();
            let mut want = triangle(start, dir).to_vec();
            got.sort();
            want.sort();
            assert_eq!(got, want, "dir {dir}");
            // Named from its second corner it is the same key.
            assert_eq!(t, Sock::tri_from(start.add(LPt::edge(dir)), dir + 4));
            // The centroid is a third of the way along each median.
            let (x, z) = t.at.to_xz();
            let (cx, cz) = c[..3]
                .iter()
                .map(|p| p.to_xz())
                .fold((0.0, 0.0), |a, b| (a.0 + b.0 / 3.0, a.1 + b.1 / 3.0));
            assert!((x - cx).abs() < 1e-4 && (z - cz).abs() < 1e-4);
        }
    }

    #[test]
    fn sides_are_the_edges_between_corners_and_beside_inverts_them() {
        let bodies = [
            Sock::square_from(LPt::ZERO, 0),
            Sock::square_from(LPt::edge(4), 1),
            Sock::tri_from(LPt::ZERO, 0),
            Sock::tri_from(LPt::edge(9), 3),
            Sock::tri_from(LPt::edge(2), 6),
        ];
        for b in bodies {
            let (sides, n) = b.sides();
            for e in &sides[..n] {
                let (lo, hi) = e.corners();
                let _ = hi;
                // Each side is 3 m.
                let (x0, z0) = lo[0].to_xz();
                let (x1, z1) = lo[1].to_xz();
                let len2 = (x1 - x0) * (x1 - x0) + (z1 - z0) * (z1 - z0);
                assert!((len2 - 9.0).abs() < 1e-3, "{b:?} {e:?}");
                // The body is on exactly one side, and `beside` finds it.
                let side = e.side_of(b.at);
                assert!(side != 0);
                let (sq, tri) = beside(e, side);
                let want = if b.foot == FOOT_SQUARE { sq } else { tri };
                assert_eq!(want, b, "{e:?} side {side}");
                assert!(want.has_side(e));
                // And the other side's bodies do not overlap it.
                let (osq, otri) = beside(e, -side);
                let (bc, bn) = b.corners();
                for o in [osq, otri] {
                    let (oc, on) = o.corners();
                    assert!(!overlaps(&bc[..bn], &oc[..on]), "{b:?} vs {o:?}");
                    assert!(o.has_side(e));
                }
            }
        }
    }

    /// Rust's honeycomb: a square with a triangle on every side, and the
    /// triangles' outer sides take 3 m walls like any other.
    #[test]
    fn a_square_ringed_with_triangles_tiles() {
        let core = Sock::square_from(LPt::ZERO, 0);
        let (sides, n) = core.sides();
        let mut pieces = vec![core];
        for e in &sides[..n] {
            let out = -e.side_of(core.at);
            let (_, tri) = beside(e, out);
            pieces.push(tri);
        }
        for i in 0..pieces.len() {
            for j in i + 1..pieces.len() {
                let (a, an) = pieces[i].corners();
                let (b, bn) = pieces[j].corners();
                assert!(!overlaps(&a[..an], &b[..bn]));
            }
        }
        // Twelve distinct edges round the outside of a square and four
        // triangles: 4 core sides + 8 outer.
        let mut edges: Vec<Sock> = pieces
            .iter()
            .flat_map(|p| {
                let (s, n) = p.sides();
                s[..n].to_vec()
            })
            .collect();
        edges.sort();
        edges.dedup();
        assert_eq!(edges.len(), 12);
    }

    #[test]
    fn the_grid_cell_is_a_square_from_its_low_corner() {
        let p = grid_cell(100, 100, 103, 98).unwrap();
        assert!(close(p, 9.0, -6.0));
        let s = Sock::square_from(p, 0);
        assert!(close(s.at, 10.5, -4.5));
    }
}
