//! The plants the bush column grows (`sim_core::terrain::plant_of`): a shrub
//! that is only scenery, a berry bush, and hemp.
//!
//! Every plant is meshes in one **ground-rooted** frame (y = 0 is the slot's
//! ground): its leaves, alpha-cut cards; its wood, an opaque vertex-coloured
//! mesh holding the twigs or the hemp stalk; and a berry bush's berries,
//! which shine where the wood does not. All of them sway on the same bend
//! (`foliage::Kind::BushLeaf`, `HempLeaf` and `Stem` share it), so a berry
//! stays on its branch. Pure functions of a variant, so a chunk streams back
//! bit-identical and two clients grow the same plant — the law `tree.rs` and
//! `props.rs` keep.
//!
//! **What this replaced.** A bush was three crossed leaf cards wrapped round
//! an opaque vertex-green icosphere, "the interior mass a bush needs to not
//! read as a flat billboard". The tall bush stretched that blob upward and
//! drew it at half brightness, and through every gap in the leaves it read as
//! a dark green lump (operator, 2026-10-06). A leaf mass is leaves all the
//! way in: the interior is now more cards, darker the deeper they sit, over
//! a few twigs that root the plant in the ground.

use std::f32::consts::{PI, TAU};

use bevy::prelude::*;

use super::props::{hash01, Soup, BUSH_CARD_CELLS, BUSH_CARD_COLS, BUSH_CARD_ROWS};

/// Distinct meshes per plant, indexed by the slot's yaw (`props::spawn_slot`)
/// exactly as the conifer pool is, so two plants side by side do not present
/// the same cards.
pub const PLANT_POOL: usize = 4;

/// A shrub's height and half-width at scale 1, metres. The short one comes
/// to a standing player's chest; the tall one is over their head, the
/// reference's Glaucous Willow and Spicebush: *"taller than the player, which
/// means you should be able to use them as cover and hide within the
/// canopy"* (devblog 198). Which one a cell grows is [`super::props::tall_bush`].
pub const SHRUB_H: f32 = 1.3;
pub const TALL_SHRUB_H: f32 = 2.15;

/// A berry bush is the small round one, waist-high, so it reads as a
/// different plant from the shrubs around it before its berries do.
pub const BERRY_BUSH_H: f32 = 1.0;

/// Hemp's height at scale 1, metres: one stalk to about the chest.
pub const HEMP_H: f32 = 1.3;

/// How dark the deepest leaves are against the outermost, as a vertex
/// value. **(knob)** This is the whole of what the old blob was for: the
/// inside of a bush is shaded by the outside, so the cards near the heart
/// draw darker and a gap in the outer leaves shows leaves in shadow rather
/// than sky or a green lump.
pub const HEART_SHADE: f32 = 0.62;

/// The value at a plant's root against its crown — the light a leaf mass
/// keeps off its own lower half. **(knob)**
pub const ROOT_SHADE: f32 = 0.72;

/// How far a leaf card's normal is pulled toward a sphere centred on its
/// plant: a leaf mass scatters light as a rough sphere, not as plates
/// (`clutter::BLADE_TIP_BLEND`'s "pile of foil"). A little facet is kept so
/// two cards at different angles do not shade identically.
pub const LEAF_VOLUME: f32 = 0.85;

/// Berry clusters on one bush, and berries in a cluster (inclusive range).
pub const BERRY_CLUSTERS: u32 = 12;
pub const BERRIES_PER_CLUSTER: (u32, u32) = (4, 7);
/// A berry's radius, metres. Larger than a real one on purpose: the berries
/// are how a player tells this bush from scenery at ten metres.
pub const BERRY_R: f32 = 0.026;

/// How far out a berry cluster hangs, as a share of the leaf mass's radius at
/// its height: inside the outer leaves, so the berries show through the gaps
/// and at the edges rather than hanging in the air beside the bush.
pub const BERRY_DEPTH: (f32, f32) = (0.55, 0.85);

/// Berry colours, linear: ripe, and the odd darker one, for the two kinds a
/// bush can carry. Which one a bush grows is off its cell key
/// ([`super::props::red_berries`]) — cosmetic, both pay the same berries.
const RED_RIPE: [f32; 3] = [0.40, 0.008, 0.016];
const RED_DARK: [f32; 3] = [0.17, 0.005, 0.012];
const BLUE_RIPE: [f32; 3] = [0.024, 0.030, 0.11];
const BLUE_DARK: [f32; 3] = [0.010, 0.012, 0.045];

/// Bark on a twig, and the hemp stalk at its root and its tip, linear.
const TWIG: [f32; 3] = [0.065, 0.042, 0.024];
const STALK_ROOT: [f32; 3] = [0.06, 0.075, 0.025];
const STALK_TIP: [f32; 3] = [0.11, 0.20, 0.045];

fn smooth01(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// How a plant's leaves are lit by their own position: darker toward the
/// root ([`ROOT_SHADE`]) and toward the heart ([`HEART_SHADE`]).
#[derive(Clone, Copy)]
struct Shade {
    h: f32,
    r: f32,
}

impl Shade {
    fn at(self, v: Vec3) -> f32 {
        let up = ROOT_SHADE + (1.0 - ROOT_SHADE) * smooth01(v.y / self.h);
        let out = Vec2::new(v.x, v.z).length() / self.r;
        up * (HEART_SHADE + (1.0 - HEART_SHADE) * smooth01(out * 1.3))
    }
}

/// One photographed leaf cluster from the bush atlas: a square card of
/// half-size `half` centred at `c`, spanned by `right` and `up` (unit,
/// orthogonal), sampling atlas cell `cell`, its normals pulled toward `dome`.
///
/// Square for `props::BUSH_CARD_ATLAS`'s reason: a card samples a whole cell
/// and the cells are square, so any other quad stretches every leaf on it.
#[allow(clippy::too_many_arguments)]
fn card(
    s: &mut Soup,
    c: Vec3,
    right: Vec3,
    up: Vec3,
    half: f32,
    cell: u32,
    value: f32,
    shade: Shade,
    dome: Vec3,
) {
    let (du, dv) = (1.0 / BUSH_CARD_COLS as f32, 1.0 / BUSH_CARD_ROWS as f32);
    let (cu, cv) = (
        (cell % BUSH_CARD_COLS) as f32 * du,
        (cell / BUSH_CARD_COLS) as f32 * dv,
    );
    // A 3×3 grid of vertices rather than four corners: a heart card crosses
    // the plant's axis, and only a vertex ON the axis can carry the dark the
    // heart is for — four corners would interpolate their own brightness
    // straight across it.
    let at = |i: usize, j: usize| -> (Vec3, [f32; 2]) {
        let (x, y) = (i as f32 - 1.0, j as f32 - 1.0);
        // V grows downward in image space: the card's TOP takes the cell's
        // smallest v. `tests/bush_card.rs` asserts it rather than trusting it.
        (
            c + right * (half * x) + up * (half * y),
            [cu + du * i as f32 * 0.5, cv + dv * (1.0 - j as f32 * 0.5)],
        )
    };
    let col = move |v: Vec3| {
        let k = value * shade.at(v);
        [k, k, k, 1.0]
    };
    let blend = |_: Vec3| LEAF_VOLUME;
    for i in 0..2 {
        for j in 0..2 {
            let (b0, t0, b1, t1) = (at(i, j), at(i, j + 1), at(i + 1, j), at(i + 1, j + 1));
            s.tri_uv([b0, t0, b1], col, Some(dome), blend);
            s.tri_uv([b1, t0, t1], col, Some(dome), blend);
        }
    }
}

/// `n` cards crossed about a vertical axis through `c`, spread evenly and
/// jittered, each leant off vertical by up to `tilt` radians toward or away
/// from the plant's heart, so a cluster is a tuft rather than a picket.
#[allow(clippy::too_many_arguments)]
fn cluster(
    s: &mut Soup,
    seed: u32,
    c: Vec3,
    half: f32,
    n: u32,
    tilt: f32,
    value: f32,
    shade: Shade,
    dome: Vec3,
) {
    let yaw0 = hash01(seed, 1) * PI;
    for i in 0..n {
        let a = yaw0 + i as f32 * PI / n as f32 + (hash01(seed, 10 + i) - 0.5) * 0.45;
        let right = Vec3::new(a.cos(), 0.0, -a.sin());
        let face = Vec3::new(a.sin(), 0.0, a.cos());
        let lean = (hash01(seed, 20 + i) - 0.5) * 2.0 * tilt;
        let up = Vec3::Y * lean.cos() + face * lean.sin();
        let h = half * (0.86 + 0.28 * hash01(seed, 30 + i));
        let cell = (hash01(seed, 40 + i) * BUSH_CARD_CELLS as f32) as u32 % BUSH_CARD_CELLS;
        let v = value * (0.92 + 0.16 * hash01(seed, 50 + i));
        card(s, c, right, up, h, cell, v, shade, dome);
    }
}

/// A ring of `n` two-card clusters round the plant's axis at height `y` and
/// radius `r`, jittered, each brighter than the heart by `value`.
#[allow(clippy::too_many_arguments)]
fn ring(
    s: &mut Soup,
    seed: u32,
    n: u32,
    r: f32,
    y: f32,
    half: f32,
    value: f32,
    shade: Shade,
    dome: Vec3,
) {
    let a0 = hash01(seed, 3) * TAU;
    for k in 0..n {
        let a = a0 + k as f32 * TAU / n as f32 + (hash01(seed, 60 + k) - 0.5) * 0.6;
        let rr = r * (0.85 + 0.3 * hash01(seed, 70 + k));
        let yy = y + (hash01(seed, 80 + k) - 0.5) * 0.16;
        let c = Vec3::new(a.sin() * rr, yy, a.cos() * rr);
        cluster(s, seed ^ (0x100 + k), c, half, 2, 0.35, value, shade, dome);
    }
}

/// A tapered square twig from `a` to `b`, radius `r0` at `a` and `r1` at `b`.
/// Four sides: at a centimetre across a twig's shading is mostly its colour,
/// and its normals are pulled toward its own middle so the faces read as a
/// round stem. Wound outward — the wood material culls back faces.
fn twig(s: &mut Soup, a: Vec3, b: Vec3, r0: f32, r1: f32, col: [f32; 3]) {
    let axis = (b - a).normalize_or_zero();
    let side = axis.cross(Vec3::X).normalize_or(Vec3::Z);
    let other = axis.cross(side);
    let ring = |p: Vec3, r: f32| -> [Vec3; 4] {
        [p + side * r, p + other * r, p - side * r, p - other * r]
    };
    let (ra, rb) = (ring(a, r0), ring(b, r1.max(0.002)));
    let c = [col[0], col[1], col[2], 1.0];
    let mid = (a + b) * 0.5;
    for i in 0..4 {
        let j = (i + 1) % 4;
        s.tri(ra[i], ra[j], rb[i], |_| c, Some(mid), 0.6);
        s.tri(ra[j], rb[j], rb[i], |_| c, Some(mid), 0.6);
    }
}

/// `n` twigs rising from the root into a leaf mass `h` tall: each leaves the
/// ground near the axis and ends inside the leaves, bent once, so the bottom
/// of the plant is stems and not leaves sitting on the grass.
fn twigs(s: &mut Soup, seed: u32, n: u32, h: f32, spread: f32, r0: f32) {
    let a0 = hash01(seed, 5) * TAU;
    for k in 0..n {
        let a = a0 + k as f32 * TAU / n as f32 + (hash01(seed, 90 + k) - 0.5) * 0.8;
        let dir = Vec3::new(a.sin(), 0.0, a.cos());
        let root = dir * (0.03 + 0.05 * hash01(seed, 100 + k)) - Vec3::Y * 0.05;
        let top_h = h * (0.45 + 0.25 * hash01(seed, 110 + k));
        let out = spread * (0.6 + 0.4 * hash01(seed, 120 + k));
        let knee = root + dir * out * 0.35 + Vec3::Y * top_h * 0.5;
        let tip = root + dir * out + Vec3::Y * top_h;
        twig(s, root, knee, r0, r0 * 0.7, TWIG);
        twig(s, knee, tip, r0 * 0.7, r0 * 0.35, TWIG);
        // One side shoot off the knee.
        let side = Vec3::new(-dir.z, 0.0, dir.x) * if k % 2 == 0 { 1.0 } else { -1.0 };
        let shoot = knee + (side * 0.6 + dir * 0.4) * out * 0.5 + Vec3::Y * top_h * 0.3;
        twig(s, knee, shoot, r0 * 0.45, r0 * 0.25, TWIG);
    }
}

/// A berry: an icosahedron with smooth normals, which at three centimetres
/// is as round as a sphere is.
fn berry(s: &mut Soup, c: Vec3, r: f32, col: [f32; 3]) {
    let t = (1.0 + 5.0f32.sqrt()) * 0.5;
    let v = [
        Vec3::new(-1.0, t, 0.0),
        Vec3::new(1.0, t, 0.0),
        Vec3::new(-1.0, -t, 0.0),
        Vec3::new(1.0, -t, 0.0),
        Vec3::new(0.0, -1.0, t),
        Vec3::new(0.0, 1.0, t),
        Vec3::new(0.0, -1.0, -t),
        Vec3::new(0.0, 1.0, -t),
        Vec3::new(t, 0.0, -1.0),
        Vec3::new(t, 0.0, 1.0),
        Vec3::new(-t, 0.0, -1.0),
        Vec3::new(-t, 0.0, 1.0),
    ]
    .map(|p| c + p.normalize() * r);
    const F: [[usize; 3]; 20] = [
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    let k = [col[0], col[1], col[2], 1.0];
    for f in F {
        s.tri(v[f[0]], v[f[1]], v[f[2]], |_| k, Some(c), 1.0);
    }
}

fn seed_of(base: u32, variant: u32) -> u32 {
    base ^ variant.wrapping_mul(2_654_435_761)
}

/// A shrub's leaves: a dark heart of big cards, rings of smaller clusters
/// round it, brighter the further out and up they sit, and a tuft on top.
/// `tall` stacks a second heart and two more rings over the first.
pub fn shrub_leaves(variant: u32, tall: bool) -> Mesh {
    let mut s = Soup::default();
    let seed = seed_of(if tall { 0x7a11_5b0b } else { 0x5b0b }, variant);
    if tall {
        let (h, r) = (TALL_SHRUB_H, 0.8);
        let shade = Shade { h, r };
        let dome = Vec3::new(0.0, h * 0.5, 0.0);
        let lean = Vec3::new(
            (hash01(seed, 7) - 0.5) * 0.3,
            0.0,
            (hash01(seed, 8) - 0.5) * 0.3,
        );
        cluster(
            &mut s,
            seed,
            Vec3::new(0.0, 0.62, 0.0),
            0.55,
            3,
            0.1,
            1.0,
            shade,
            dome,
        );
        cluster(
            &mut s,
            seed ^ 0x2,
            Vec3::Y * 1.38 + lean,
            0.5,
            3,
            0.1,
            1.0,
            shade,
            dome,
        );
        ring(&mut s, seed ^ 0x3, 5, 0.34, 0.6, 0.4, 1.0, shade, dome);
        ring(&mut s, seed ^ 0x4, 4, 0.30, 1.18, 0.38, 1.04, shade, dome);
        ring(&mut s, seed ^ 0x5, 3, 0.22, 1.6, 0.33, 1.08, shade, dome);
        cluster(
            &mut s,
            seed ^ 0x6,
            Vec3::Y * 1.86 + lean,
            0.3,
            2,
            0.6,
            1.12,
            shade,
            dome,
        );
    } else {
        let (h, r) = (SHRUB_H, 0.75);
        let shade = Shade { h, r };
        let dome = Vec3::new(0.0, h * 0.45, 0.0);
        cluster(
            &mut s,
            seed,
            Vec3::new(0.0, 0.56, 0.0),
            0.56,
            3,
            0.1,
            1.0,
            shade,
            dome,
        );
        ring(&mut s, seed ^ 0x3, 5, 0.32, 0.5, 0.38, 1.0, shade, dome);
        ring(&mut s, seed ^ 0x4, 3, 0.22, 0.84, 0.35, 1.06, shade, dome);
        cluster(
            &mut s,
            seed ^ 0x6,
            Vec3::new(0.05, 1.0, -0.04),
            0.3,
            2,
            0.6,
            1.12,
            shade,
            dome,
        );
    }
    s.mesh()
}

/// A shrub's wood: the twigs under its leaves.
pub fn shrub_wood(variant: u32, tall: bool) -> Mesh {
    let mut s = Soup::default();
    let seed = seed_of(if tall { 0x7a11_0d0d } else { 0x0d0d }, variant);
    if tall {
        twigs(&mut s, seed, 6, 1.5, 0.45, 0.022);
    } else {
        twigs(&mut s, seed, 5, 0.85, 0.4, 0.016);
    }
    s.mesh()
}

/// A berry bush's leaves: the shrub's build, small and round.
pub fn berry_leaves(variant: u32) -> Mesh {
    let mut s = Soup::default();
    let seed = seed_of(0xbe44_1eaf, variant);
    let (h, r) = (BERRY_BUSH_H, 0.5);
    let shade = Shade { h, r };
    let dome = Vec3::new(0.0, h * 0.42, 0.0);
    cluster(
        &mut s,
        seed,
        Vec3::new(0.0, 0.42, 0.0),
        0.42,
        3,
        0.1,
        1.0,
        shade,
        dome,
    );
    ring(&mut s, seed ^ 0x3, 6, 0.2, 0.36, 0.3, 1.0, shade, dome);
    ring(&mut s, seed ^ 0x4, 4, 0.13, 0.64, 0.27, 1.06, shade, dome);
    cluster(
        &mut s,
        seed ^ 0x6,
        Vec3::new(-0.03, 0.8, 0.02),
        0.22,
        2,
        0.6,
        1.1,
        shade,
        dome,
    );
    s.mesh()
}

/// A berry bush's wood: the twigs under its leaves.
pub fn berry_wood(variant: u32) -> Mesh {
    let mut s = Soup::default();
    twigs(&mut s, seed_of(0xbe44_0d0d, variant), 4, 0.6, 0.26, 0.012);
    s.mesh()
}

/// A berry bush's berries, hung in clusters among the outer leaves where a
/// player sees them. `red` picks the colour. Their own mesh, because a berry
/// shines and a twig does not, and one material has one roughness.
pub fn berries(variant: u32, red: bool) -> Mesh {
    let mut s = Soup::default();
    let seed = seed_of(0xbe44_f00d, variant);
    let (ripe, dark) = if red {
        (RED_RIPE, RED_DARK)
    } else {
        (BLUE_RIPE, BLUE_DARK)
    };
    // The leaf mass's outline, as an ellipsoid ([`berry_leaves`]'s).
    let (cy, ry, rr) = (0.45, 0.42, 0.48);
    let a0 = hash01(seed, 9) * TAU;
    for k in 0..BERRY_CLUSTERS {
        let a = a0 + k as f32 * TAU / BERRY_CLUSTERS as f32 + (hash01(seed, 200 + k) - 0.5) * 0.4;
        let y = 0.22 + 0.58 * hash01(seed, 210 + k);
        let e = ((y - cy) / ry).clamp(-1.0, 1.0);
        let (d0, d1) = BERRY_DEPTH;
        let out = rr * (1.0 - e * e).max(0.0).sqrt() * (d0 + (d1 - d0) * hash01(seed, 220 + k));
        let c = Vec3::new(a.sin() * out, y, a.cos() * out);
        let (lo, hi) = BERRIES_PER_CLUSTER;
        let n = lo + (hash01(seed, 230 + k) * (hi - lo + 1) as f32) as u32 % (hi - lo + 1);
        for j in 0..n {
            let q = seed ^ (k * 31 + j);
            let off = Vec3::new(hash01(q, 1) - 0.5, hash01(q, 2) - 0.65, hash01(q, 3) - 0.5) * 0.07;
            let r = BERRY_R * (0.8 + 0.4 * hash01(q, 4));
            let col = lerp3(
                ripe,
                dark,
                if hash01(q, 5) < 0.2 {
                    1.0
                } else {
                    0.3 * hash01(q, 6)
                },
            );
            berry(&mut s, c + off, r, col);
        }
    }
    s.mesh()
}

/// Hemp: a stalk with leaves in opposite pairs, each pair a quarter turn on
/// from the last, broad and drooping at the bottom and small and raised at
/// the top. `(stalk, leaves)` — the stalk is opaque, the leaves are cards
/// wearing [`hemp_leaf_image`].
pub fn hemp(variant: u32) -> (Mesh, Mesh) {
    let seed = seed_of(0x4e4d_9000, variant);
    let h = HEMP_H * (0.92 + 0.16 * hash01(seed, 1));
    // A gentle bow, so the stalk is not a rod.
    let bow = Vec3::new(hash01(seed, 2) - 0.5, 0.0, hash01(seed, 3) - 0.5) * 0.12;
    let at = |t: f32| Vec3::Y * (t * h) + bow * (t * PI).sin();

    let mut stalk = Soup::default();
    const SEGS: usize = 6;
    for i in 0..SEGS {
        let (t0, t1) = (i as f32 / SEGS as f32, (i + 1) as f32 / SEGS as f32);
        let r = |t: f32| 0.014 * (1.0 - t) + 0.004 * t;
        let col = lerp3(STALK_ROOT, STALK_TIP, smooth01(t0 * 1.4));
        let a = if i == 0 {
            at(t0) - Vec3::Y * 0.05
        } else {
            at(t0)
        };
        twig(&mut stalk, a, at(t1), r(t0), r(t1), col);
    }

    let mut leaves = Soup::default();
    // Below the ground, so every leaf's normal leans up toward the sky: a
    // hemp leaf is a plate seen from both sides, and a mesh with tangents and
    // no normal map does not have its back face's normal flipped by Bevy's
    // PBR, so a facet normal would light the top of every drooping leaf as
    // its underside. The bush's cards dodge it the same way, toward a dome.
    let dome = Vec3::new(0.0, -h, 0.0);
    const NODES: u32 = 9;
    let yaw0 = hash01(seed, 4) * TAU;
    for k in 0..NODES {
        let f = k as f32 / (NODES - 1) as f32;
        let t = 0.16 + 0.74 * f;
        let base = at(t);
        let len = (0.4 - 0.22 * f) * (0.9 + 0.2 * hash01(seed, 10 + k));
        // Lower leaves droop, upper ones lift: a hemp plant is a cone of
        // leaves, widest at the bottom.
        let droop = -0.55 + 0.75 * f + (hash01(seed, 20 + k) - 0.5) * 0.2;
        let yaw = yaw0 + k as f32 * PI * 0.5 + (hash01(seed, 30 + k) - 0.5) * 0.4;
        let value = 0.95 + 0.2 * f;
        for side in [0.0, PI] {
            let a = yaw + side;
            let out = Vec3::new(a.sin(), 0.0, a.cos());
            let roll = (hash01(seed, 40 + k + side as u32) - 0.5) * 0.5;
            hemp_leaf(
                &mut leaves,
                base + out * 0.02,
                out,
                droop,
                roll,
                len,
                value,
                dome,
            );
            // A side shoot's smaller leaf a quarter turn round, on the lower
            // two thirds: what makes a hemp plant a bush and not a pole.
            if f < 0.7 {
                let a = a + PI * 0.5 + (hash01(seed, 50 + k + side as u32) - 0.5) * 0.5;
                let out = Vec3::new(a.sin(), 0.0, a.cos());
                let at = base + out * 0.03 + Vec3::Y * 0.04;
                hemp_leaf(
                    &mut leaves,
                    at,
                    out,
                    droop + 0.25,
                    -roll,
                    len * 0.62,
                    value,
                    dome,
                );
            }
        }
    }
    // The top: a tuft of small leaves turned up round the tip.
    let tip = at(1.0);
    for k in 0..5u32 {
        let a = yaw0 + k as f32 * TAU / 5.0 + 0.3;
        let out = Vec3::new(a.sin(), 0.0, a.cos());
        let len = 0.13 * (0.85 + 0.3 * hash01(seed, 60 + k));
        let lift = 0.75 + 0.4 * hash01(seed, 70 + k);
        hemp_leaf(
            &mut leaves,
            tip - Vec3::Y * 0.06,
            out,
            lift,
            0.0,
            len,
            1.05,
            dome,
        );
    }
    (stalk.mesh(), leaves.mesh())
}

/// One hemp leaf: a [`hemp_leaf_image`] card whose petiole (the image's
/// bottom-centre) sits at `base` and whose tip points `out`, pitched by
/// `pitch` radians (negative droops) and rolled about its own midrib by
/// `roll`.
#[allow(clippy::too_many_arguments)]
fn hemp_leaf(
    s: &mut Soup,
    base: Vec3,
    out: Vec3,
    pitch: f32,
    roll: f32,
    len: f32,
    value: f32,
    dome: Vec3,
) {
    let along = (out * pitch.cos() + Vec3::Y * pitch.sin()).normalize();
    let flat = Vec3::new(-out.z, 0.0, out.x);
    // Roll the blade about its midrib so a pair is not two coplanar plates.
    let width = (flat * roll.cos() + along.cross(flat) * roll.sin()).normalize() * (len * 0.5);
    let tip = base + along * len;
    let (b0, b1, t0, t1) = (base - width, base + width, tip - width, tip + width);
    let col = move |v: Vec3| {
        // Darker where the leaf meets the stalk, under the leaves above it.
        let k = value * (0.8 + 0.2 * smooth01((v - base).length() / len.max(1e-3)));
        [k, k, k, 1.0]
    };
    let blend = |_: Vec3| 0.8;
    s.tri_uv(
        [(b0, [0.0, 1.0]), (t0, [0.0, 0.0]), (b1, [1.0, 1.0])],
        col,
        Some(dome),
        blend,
    );
    s.tri_uv(
        [(b1, [1.0, 1.0]), (t0, [0.0, 0.0]), (t1, [1.0, 0.0])],
        col,
        Some(dome),
        blend,
    );
}

/// The hemp leaf card's edge, texels: power of two, for the mip chain.
pub const HEMP_LEAF_PX: u32 = 256;

/// Hemp's leaf, drawn rather than photographed: seven narrow serrated
/// leaflets fanned from a petiole at the bottom-centre, the silhouette that
/// says hemp at a glance. Coverage is supersampled 4×4 so the mask's edge is
/// a real coverage value, which is what the coverage-preserving chain
/// (`mipmap::Filter::Mask`) keeps as the card shrinks.
pub fn hemp_leaf_image() -> Image {
    super::tree::alpha_card(hemp_leaf_rgba(HEMP_LEAF_PX), HEMP_LEAF_PX)
}

/// [`hemp_leaf_image`]'s level 0, RGBA8 sRGB. Public so a gate can count it.
pub fn hemp_leaf_rgba(n: u32) -> Vec<u8> {
    // Leaflets: angle off straight up (radians) and length as a share of the
    // longest, outermost last.
    const LEAFLETS: [(f32, f32); 7] = [
        (0.0, 1.0),
        (0.46, 0.86),
        (-0.46, 0.86),
        (0.95, 0.64),
        (-0.95, 0.64),
        (1.42, 0.38),
        (-1.42, 0.38),
    ];
    let nf = n as f32;
    let base = Vec2::new(0.5, 0.965);
    // Where the leaflets leave the petiole.
    let hub = Vec2::new(0.5, 0.9);
    let longest = 0.86;
    // sRGB: the blade, its lighter midrib, and its darker edge.
    // A light, yellowish green: hemp is spotted across a meadow by its
    // colour as much as its shape.
    let blade = Vec3::new(104.0, 168.0, 60.0);
    let rib = Vec3::new(160.0, 205.0, 110.0);
    let rim = Vec3::new(72.0, 126.0, 44.0);

    // Half-width of a leaflet at fraction `t` of its length, as a share of
    // its length: lanceolate, widest a third of the way out, serrated.
    let width = |t: f32| -> f32 {
        if !(0.0..=1.0).contains(&t) {
            return -1.0;
        }
        let body = (t * PI).sin().powf(1.1) * (1.0 - 0.35 * t);
        let teeth = 1.0 - 0.28 * (t * 13.0).fract();
        0.105 * body * teeth
    };
    // Which leaflet (if any) covers the point `p`, and how far across it.
    let sample = |p: Vec2| -> Option<(f32, f32, usize)> {
        // The petiole: a short stalk from the base to the hub.
        let d = hub - base;
        let t = (p - base).dot(d) / d.length_squared();
        if (0.0..=1.0).contains(&t) {
            let q = base + d * t;
            if (p - q).length() < 0.012 {
                return Some((0.0, 0.0, 99));
            }
        }
        let mut best: Option<(f32, f32, usize)> = None;
        for (i, &(ang, share)) in LEAFLETS.iter().enumerate() {
            let len = longest * share;
            let dir = Vec2::new(ang.sin(), -ang.cos());
            let rel = p - hub;
            let along = rel.dot(dir) / len;
            let across = (rel.x * dir.y - rel.y * dir.x).abs() / len;
            let w = width(along);
            if w > 0.0 && across <= w {
                let k = across / w;
                if best.is_none_or(|b| k < b.1) {
                    best = Some((along, k, i));
                }
            }
        }
        best
    };

    const SS: u32 = 4;
    let mut out = vec![0u8; (n * n * 4) as usize];
    for y in 0..n {
        for x in 0..n {
            let mut cover = 0u32;
            let mut hit: Option<(f32, f32, usize)> = None;
            for sy in 0..SS {
                for sx in 0..SS {
                    let p = Vec2::new(
                        (x as f32 + (sx as f32 + 0.5) / SS as f32) / nf,
                        (y as f32 + (sy as f32 + 0.5) / SS as f32) / nf,
                    );
                    if let Some(h) = sample(p) {
                        cover += 1;
                        if hit.is_none_or(|b| h.1 < b.1) {
                            hit = Some(h);
                        }
                    }
                }
            }
            let i = ((y * n + x) * 4) as usize;
            let Some((along, k, leaflet)) = hit else {
                // A transparent texel carries the blade's colour, so the
                // filtered edge never fringes dark.
                out[i..i + 4].copy_from_slice(&[blade.x as u8, blade.y as u8, blade.z as u8, 0]);
                continue;
            };
            let c = if leaflet == 99 {
                rib * 0.8
            } else {
                // Each leaflet a touch different, lighter toward its tip.
                let v = 0.94 + 0.12 * hash01(0x4e4d, leaflet as u32) + 0.08 * along;
                let c = if k < 0.12 {
                    rib
                } else {
                    blade.lerp(rim, smooth01((k - 0.55) / 0.45))
                };
                // A faint vein ladder off the midrib.
                let vein = ((along * 22.0 - k * 6.0).fract() - 0.5).abs() < 0.06 && k < 0.8;
                let c = if vein { c.lerp(rib, 0.25) } else { c };
                c * v
            };
            let a = (cover * 255 / (SS * SS)) as u8;
            out[i..i + 4].copy_from_slice(&[
                c.x.min(255.0) as u8,
                c.y.min(255.0) as u8,
                c.z.min(255.0) as u8,
                a,
            ]);
        }
    }
    out
}
