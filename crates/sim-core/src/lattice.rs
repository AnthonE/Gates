//! Exact points of the square-and-triangle tiling — the address space
//! freeform building will stand on (freeform M8, unwired: nothing in the
//! sim reads this yet).
//!
//! A building is squares and equilateral triangles with 3 m edges, every
//! edge at a multiple of 30°. Every vertex, edge midpoint, centroid and
//! apothem such a tiling can name lies in Z[ζ]/2, ζ = e^{iπ/6}: four
//! integers (a, b, c, d) meaning (a + bζ + cζ² + dζ³)/2 metres in the
//! building's own frame, x along ζ⁰ and z along ζ³. ζ⁴ = ζ² − 1 folds every
//! higher power back into that basis, and the basis is independent over Q,
//! so a point has exactly one key: two paths round a honeycomb that meet,
//! meet exactly, and "is this socket taken" stays an integer lookup.
//!
//! A 30° turn is an integer shuffle, (a, b, c, d) → (−d, a, b + d, c), so
//! no float ever decides where a piece is. Floats appear only at the edge,
//! in [`LPt::to_xz`], in a fixed operation order. Overlap inside one
//! building is exact too ([`overlaps`]): every edge runs at a multiple of
//! 30°, a projection onto any of the six axes is (P + Q√3)/4, and the sign
//! of P + Q√3 is decided in integers.

/// Coefficients past this are refused by [`LPt::fits`]. Z[ζ] is dense in
/// the plane, so a coefficient is not a distance: it follows the conjugate
/// image of the build path as well as the path, and a long zig-zag can grow
/// one well past the metres it covers. 4096 keeps every product in
/// [`sign_p_q3`] far inside i64 and a coefficient inside 13 bits on a wire.
pub const LATTICE_MAX_COEF: i32 = 4096;

/// √3 / 4 — the weight of `b` and `c` in [`LPt::to_xz`].
const SQRT3_4: f32 = 0.433_012_7;

/// A point of the tiling: (a + bζ + cζ² + dζ³)/2 metres.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LPt {
    pub a: i32,
    pub b: i32,
    pub c: i32,
    pub d: i32,
}

impl LPt {
    pub const ZERO: LPt = LPt::new(0, 0, 0, 0);

    pub const fn new(a: i32, b: i32, c: i32, d: i32) -> Self {
        Self { a, b, c, d }
    }

    pub const fn add(self, o: LPt) -> LPt {
        LPt::new(self.a + o.a, self.b + o.b, self.c + o.c, self.d + o.d)
    }

    pub const fn sub(self, o: LPt) -> LPt {
        LPt::new(self.a - o.a, self.b - o.b, self.c - o.c, self.d - o.d)
    }

    pub const fn neg(self) -> LPt {
        LPt::new(-self.a, -self.b, -self.c, -self.d)
    }

    /// Times ζ: a quarter of a right angle more, anticlockwise from +x
    /// toward +z. ζ·(a + bζ + cζ² + dζ³) = −d + aζ + (b + d)ζ² + cζ³.
    pub const fn rot1(self) -> LPt {
        LPt::new(-self.d, self.a, self.b + self.d, self.c)
    }

    /// Turned `k` steps of 30° (any `k`; twelve is a whole turn).
    pub const fn rot(self, k: u8) -> LPt {
        let mut p = self;
        let mut n = k % 12;
        while n > 0 {
            p = p.rot1();
            n -= 1;
        }
        p
    }

    /// One 3 m edge along direction `k` (ζᵏ, in 30° steps from +x).
    pub const fn edge(k: u8) -> LPt {
        LPt::new(6, 0, 0, 0).rot(k)
    }

    /// Whether every coefficient is inside [`LATTICE_MAX_COEF`].
    pub const fn fits(self) -> bool {
        let m = LATTICE_MAX_COEF;
        self.a.abs() <= m && self.b.abs() <= m && self.c.abs() <= m && self.d.abs() <= m
    }

    /// Local metres, (x, z). ζ = (√3/2, ½), ζ² = (½, √3/2), ζ³ = (0, 1),
    /// halved: x = a/2 + c/4 + (√3/4)b, z = d/2 + b/4 + (√3/4)c, summed in
    /// this order on every target so native and wasm agree to the bit.
    pub fn to_xz(self) -> (f32, f32) {
        let (a, b, c, d) = (self.a as f32, self.b as f32, self.c as f32, self.d as f32);
        let x = 0.5 * a + 0.25 * c + SQRT3_4 * b;
        let z = 0.5 * d + 0.25 * b + SQRT3_4 * c;
        (x, z)
    }

    /// 4 × the projection onto direction `m` (ζᵐ), as (P, Q) meaning
    /// P + Q√3: Re(p·ζ⁻ᵐ) = (2a′ + c′ + b′√3)/4 once p is turned back by m.
    pub const fn proj4(self, m: u8) -> (i64, i64) {
        let p = self.rot(12 - m % 12);
        ((2 * p.a + p.c) as i64, p.b as i64)
    }
}

/// The sign of P + Q√3, exactly: −1, 0 or 1. √3 is irrational, so the sum
/// is zero only when both are; with mixed signs the larger of P² and 3Q²
/// decides.
pub const fn sign_p_q3(p: i64, q: i64) -> i32 {
    if p >= 0 && q >= 0 {
        return (p != 0 || q != 0) as i32;
    }
    if p <= 0 && q <= 0 {
        return -((p != 0 || q != 0) as i32);
    }
    let (pp, qq) = (p * p, 3 * q * q);
    if p > 0 {
        if pp > qq {
            1
        } else {
            -1
        }
    } else if pp > qq {
        -1
    } else {
        1
    }
}

/// Exact comparison of two points along direction `m`: the sign of
/// proj(p) − proj(q).
pub const fn cmp_along(p: LPt, q: LPt, m: u8) -> i32 {
    let (pp, qq) = p.sub(q).proj4(m);
    sign_p_q3(pp, qq)
}

/// A square's corners: from `at`, along direction `dir`, then a right
/// angle anticlockwise.
pub const fn square(at: LPt, dir: u8) -> [LPt; 4] {
    let dir = dir % 12;
    let e = LPt::edge(dir);
    let f = LPt::edge(dir + 3);
    [at, at.add(e), at.add(e).add(f), at.add(f)]
}

/// An equilateral triangle's corners: from `at`, along direction `dir`, and
/// along `dir` + 60°.
pub const fn triangle(at: LPt, dir: u8) -> [LPt; 3] {
    let dir = dir % 12;
    [at, at.add(LPt::edge(dir)), at.add(LPt::edge(dir + 2))]
}

/// The extreme corner of `poly` along direction `m`: `hi` for the
/// furthest, else the nearest.
fn extreme(poly: &[LPt], m: u8, hi: bool) -> LPt {
    let mut best = poly[0];
    for &v in &poly[1..] {
        let s = cmp_along(v, best, m);
        if (hi && s > 0) || (!hi && s < 0) {
            best = v;
        }
    }
    best
}

/// Do two convex pieces of one building overlap in area? Exact: a
/// separating-axis test on the six axes every tiling edge is normal to.
/// Pieces that only share an edge or a corner do not overlap.
pub fn overlaps(a: &[LPt], b: &[LPt]) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    for m in 0..6u8 {
        let (a_lo, a_hi) = (extreme(a, m, false), extreme(a, m, true));
        let (b_lo, b_hi) = (extreme(b, m, false), extreme(b, m, true));
        if cmp_along(a_hi, b_lo, m) <= 0 || cmp_along(b_hi, a_lo, m) <= 0 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Pcg32;

    fn random_pt(rng: &mut Pcg32) -> LPt {
        let mut c = || rng.next_bounded(201) as i32 - 100;
        LPt::new(c(), c(), c(), c())
    }

    /// Twelve turns are none, turns compose, and the integer shuffle is the
    /// rotation the floats say it is.
    #[test]
    fn a_turn_is_exact_and_is_the_rotation() {
        let mut rng = Pcg32::new(0x01A7_71CE, 1);
        for _ in 0..2_000 {
            let p = random_pt(&mut rng);
            assert_eq!(p.rot(12), p);
            let (j, k) = (rng.next_bounded(12) as u8, rng.next_bounded(12) as u8);
            assert_eq!(p.rot(j).rot(k), p.rot(j + k));
            let (x, z) = p.to_xz();
            let (rx, rz) = p.rot(k).to_xz();
            let t = k as f32 * (core::f32::consts::PI / 6.0);
            let (s, c) = crate::fmath::sin_cos(t);
            let tol = 1e-3 * (1.0 + x * x + z * z);
            assert!(
                (rx - (x * c - z * s)) * (rx - (x * c - z * s)) < tol,
                "{p:?} by {k}"
            );
            assert!(
                (rz - (x * s + z * c)) * (rz - (x * s + z * c)) < tol,
                "{p:?} by {k}"
            );
        }
    }

    /// The basis is the one the module header names.
    #[test]
    fn the_basis_points_where_it_says() {
        let close = |p: LPt, x: f32, z: f32| {
            let (px, pz) = p.to_xz();
            (px - x) * (px - x) + (pz - z) * (pz - z) < 1e-10
        };
        let h = 3f32.sqrt() / 2.0;
        assert!(close(LPt::new(2, 0, 0, 0), 1.0, 0.0));
        assert!(close(LPt::new(0, 2, 0, 0), h, 0.5));
        assert!(close(LPt::new(0, 0, 2, 0), 0.5, h));
        assert!(close(LPt::new(0, 0, 0, 2), 0.0, 1.0));
        assert!(close(LPt::edge(3), 0.0, 3.0));
        // ζ⁴ = ζ² − 1.
        assert_eq!(LPt::new(2, 0, 0, 0).rot(4), LPt::new(-2, 0, 2, 0));
    }

    /// Loops close exactly: a dodecagon of edges, a hexagon of every other
    /// direction, a square, a triangle.
    #[test]
    fn loops_close_exactly() {
        let walk = |dirs: &[u8]| dirs.iter().fold(LPt::ZERO, |p, &k| p.add(LPt::edge(k)));
        let all: Vec<u8> = (0..12).collect();
        assert_eq!(walk(&all), LPt::ZERO);
        assert_eq!(walk(&[0, 2, 4, 6, 8, 10]), LPt::ZERO);
        assert_eq!(walk(&[1, 4, 7, 10]), LPt::ZERO);
        assert_eq!(walk(&[5, 9, 1]), LPt::ZERO);
    }

    /// Pieces fanned round one corner, each starting where the last ended.
    fn fan(kinds: &[bool]) -> Vec<Vec<LPt>> {
        let mut dir = 0u8;
        kinds
            .iter()
            .map(|&sq| {
                let p = if sq {
                    square(LPt::ZERO, dir).to_vec()
                } else {
                    triangle(LPt::ZERO, dir).to_vec()
                };
                dir += if sq { 3 } else { 2 };
                p
            })
            .collect()
    }

    fn any_overlap(pieces: &[Vec<LPt>]) -> bool {
        (0..pieces.len()).any(|i| (i + 1..pieces.len()).any(|j| overlaps(&pieces[i], &pieces[j])))
    }

    /// 90 + 90 + 60 + 60 + 60 is a whole turn, and nothing overlaps;
    /// 90 + 90 + 90 + 60 + 60 is 390°, and the last piece runs into the
    /// first. Every corner key is distinct in both, so only the area test
    /// can tell them apart.
    #[test]
    fn a_full_turn_of_pieces_fits_and_one_more_does_not() {
        assert!(!any_overlap(&fan(&[true, true, false, false, false])));
        assert!(!any_overlap(&fan(&[true, false, true, false, false])));
        assert!(!any_overlap(&fan(&[false; 6])));
        assert!(any_overlap(&fan(&[true, true, true, false, false])));
        assert!(any_overlap(&fan(&[false; 7])));
    }

    /// Sharing an edge is not overlapping; half a square over is.
    #[test]
    fn touching_is_not_overlapping() {
        let a = square(LPt::ZERO, 1);
        let b = square(LPt::edge(1), 1);
        assert!(!overlaps(&a, &b));
        let half = LPt::new(3, 0, 0, 0).rot(1);
        assert!(overlaps(&a, &square(half, 1)));
        assert!(overlaps(&a, &a));
        let t = triangle(LPt::ZERO, 1);
        assert!(overlaps(&a, &t));
        assert!(!overlaps(&a, &triangle(LPt::ZERO, 4)));
    }

    #[test]
    fn the_sign_is_exact() {
        assert_eq!(sign_p_q3(0, 0), 0);
        assert_eq!(sign_p_q3(2, -1), 1); // 2 − 1.732…
        assert_eq!(sign_p_q3(1, -1), -1);
        assert_eq!(sign_p_q3(-7, 4), -1); // −7 + 6.928…
        assert_eq!(sign_p_q3(-6, 4), 1);
        assert!(LPt::new(LATTICE_MAX_COEF, 0, 0, 0).fits());
        assert!(!LPt::new(0, 0, -LATTICE_MAX_COEF - 1, 0).fits());
    }
}
