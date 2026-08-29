//! Gate: the near-ground population is an object, not a decal painted on the
//! ground — measured on the geometry, headless, with no GPU and no frame.
//!
//! **Why this is arithmetic and not a screenshot.** `CLAUDE.md` says there is
//! no visual gate and none may be written; what may be gated about a frame is
//! arithmetic. Every claim here is a property of the vertices `clutter.rs`
//! emits — normals, and where the mesh sits relative to the ground it was
//! placed on — so it holds on this box, on a GPU, and on the reference VPS
//! identically.
//!
//! Three things it closes, all from `pass-20260815-042118-03-visual.md`:
//!
//! 1. The chips were four hard facets. The blind reader called them "stray
//!    flat blue triangles poking through it — an engine test surface", and
//!    the ranked gap asked for "deletion or texturing of the flat-shaded
//!    pebble primitives". Blue is the tell: a facet carrying little sun is lit
//!    almost entirely by `fill.rs`'s sky half, so a grey stone comes back
//!    blue-grey and does it in four discrete steps.
//! 2. The chips met the ground on a straight seam — `ART.md` rule 2's "a clean
//!    intersection edge reads as a decal", which the judge named on three
//!    frames.
//! 3. A comment in `clutter.rs` justified forcing every blade normal fully
//!    vertical with a claim about triangle winding. The claim is false, and
//!    §the_blade_quad below is the computation that says so. It matters
//!    because the false reason hid the real cost: a fully vertical normal is
//!    the GROUND's normal, so a blade is shaded identically to the dirt it
//!    stands in.

#![cfg(feature = "render")]

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::clutter::{
    element_mesh, CHIP_SINK, CHIP_VOLUME_BLEND, FRONDS_PER_CLUMP, TUFT_H,
};
use sim_core::terrain::{Clutter, ClutterElem};

/// The three chip-bearing kinds. Tufts are the fourth `Clutter` variant and
/// carry no chip; the last test covers the quad they are built from instead.
const CHIPS: [Clutter; 3] = [Clutter::Pebble, Clutter::Twig, Clutter::Shard];

/// Vertices one chip contributes: four triangles.
const CHIP_VERTS: usize = 12;

/// Vertices a whole element mesh holds, per kind.
///
/// A litter clump is its fallen chip PLUS standing stalks (2026-08-15,
/// `NOW.md` §0gp — the growing channel was drawn with the flattest mesh in the
/// file); pebble and shard are the chip alone. This is asserted rather than
/// assumed so that BOTH halves of a clump are gated: deleting the stalks and
/// deleting the chip they stand in are each red here.
fn verts(kind: Clutter) -> usize {
    match kind {
        Clutter::Twig => CHIP_VERTS + FRONDS_PER_CLUMP as usize * 6,
        _ => CHIP_VERTS,
    }
}

/// The chip's own twelve vertices out of an element mesh.
///
/// Every assertion in this file that names a chip runs through here, so a kind
/// that grew a second population around its chip is still measured on the chip
/// at full strength — the four tests below check the same twelve vertices of a
/// litter clump's stick that they checked when the stick was the whole mesh.
/// `clutter.rs::litter` emits the chip first and says that this is why.
fn chip_span(m: &Mesh, kind: Clutter) -> (Vec<Vec3>, Vec<Vec3>) {
    let (p, n) = (positions(m), normals(m));
    assert_eq!(
        p.len(),
        verts(kind),
        "{kind:?}: element mesh is not the population it is supposed to be"
    );
    (p[..CHIP_VERTS].to_vec(), n[..CHIP_VERTS].to_vec())
}

fn elem(kind: Clutter, yaw: u8, scale: f32) -> ClutterElem {
    ClutterElem {
        kind,
        x: 3.0,
        y: 0.0,
        z: -7.0,
        yaw,
        scale,
    }
}

fn normals(m: &Mesh) -> Vec<Vec3> {
    match m.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(v)) => {
            v.iter().map(|n| Vec3::from_array(*n)).collect()
        }
        _ => panic!("clutter mesh has float3 normals"),
    }
}

fn positions(m: &Mesh) -> Vec<Vec3> {
    match m.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(v)) => {
            v.iter().map(|p| Vec3::from_array(*p)).collect()
        }
        _ => panic!("clutter mesh has float3 positions"),
    }
}

/// A chip is four triangles. With a pure facet normal every one of its twelve
/// vertices carries one of exactly FOUR directions, and the surface reads as
/// four flat values with hard steps between them — which is what a 5 cm stone
/// must not do.
///
/// The measure is the count of distinct normal directions. Facets give 4;
/// a volume field gives one per vertex, because each vertex sits at its own
/// angle from the centroid.
#[test]
fn a_chip_is_a_volume_and_not_four_flat_facets() {
    for kind in CHIPS {
        let m = element_mesh(&elem(kind, 40, 1.0));
        let (_, n) = chip_span(&m, kind);

        // Distinct directions, at 1° resolution. Anything that still reads as
        // a facet pair lands in the same bucket.
        let mut distinct: Vec<Vec3> = Vec::new();
        for v in &n {
            if !distinct.iter().any(|d| d.dot(*v) > 0.9998) {
                distinct.push(*v);
            }
        }
        assert!(
            distinct.len() > 4,
            "{kind:?}: {} distinct normals — still a facet set, so the four \
             discrete values the judge read as an engine test surface survive",
            distinct.len()
        );
    }
}

/// The blend has to be partial in BOTH directions: 0.0 is the facet set above,
/// and 1.0 would erase the chip's own shape into a sphere's normal field —
/// a pebble is angular and `ART.md` rule 1 wants that grain kept.
#[test]
fn the_chip_keeps_most_of_its_own_facing() {
    // A const block, so this one refuses at COMPILE time rather than at run
    // time — clippy asked for it and it is the stronger gate: the constant
    // cannot be edited out of range in any build, including one nobody runs.
    const {
        assert!(
            CHIP_VOLUME_BLEND > 0.0 && CHIP_VOLUME_BLEND < 1.0,
            "a chip's normal is neither a pure facet nor a pure sphere"
        )
    };

    // Measured rather than asserted by construction: the angle between a
    // vertex normal and its own triangle's facet must be real but bounded.
    for kind in CHIPS {
        let m = element_mesh(&elem(kind, 200, 1.0));
        let (p, n) = chip_span(&m, kind);
        let mut worst: f32 = 0.0;
        for t in 0..4 {
            let (a, b, c) = (p[t * 3], p[t * 3 + 1], p[t * 3 + 2]);
            let facet = (b - a).cross(c - a).normalize();
            for v in 0..3 {
                let d = n[t * 3 + v].dot(facet).clamp(-1.0, 1.0).acos();
                worst = worst.max(d);
            }
        }
        assert!(
            worst > 0.05,
            "{kind:?}: normals sit {worst} rad off their facet — the volume \
             blend is not reaching the vertices"
        );
        assert!(
            worst < std::f32::consts::FRAC_PI_2,
            "{kind:?}: a normal turned {worst} rad off its facet and has \
             crossed into the surface it is supposed to face out of"
        );
    }
}

/// `ART.md` rule 2: a clean intersection edge reads as a decal. A chip whose
/// base ring is coplanar with the ground IS that edge, so the ring is pushed
/// under the terrain and only the taper crosses it.
#[test]
fn a_chip_sinks_into_the_ground_rather_than_standing_on_it() {
    const {
        assert!(
            CHIP_SINK > 0.0,
            "a chip that does not sink meets the ground on a straight seam"
        )
    };
    for kind in CHIPS {
        for scale in [0.75_f32, 1.0, 1.25] {
            let e = elem(kind, 91, scale);
            let m = element_mesh(&e);
            // The CHIP's extent, not the element's: a litter clump's stalks
            // stand well above its stick, and this test is about where the
            // stick meets the ground.
            let (p, _) = chip_span(&m, kind);
            let lo = p.iter().fold(f32::INFINITY, |a, v| a.min(v.y));
            let hi = p.iter().fold(f32::NEG_INFINITY, |a, v| a.max(v.y));
            assert!(
                lo < e.y,
                "{kind:?} @{scale}: lowest vertex {lo} is not below the ground \
                 plane {} it was placed on",
                e.y
            );
            // The sink is a fraction of the chip's own height, so it scales
            // with the element and never buries a small one whole. The chip
            // must still stand proud of the ground it is pushed into.
            assert!(
                hi > e.y,
                "{kind:?} @{scale}: buried — apex {hi} is at or under the ground"
            );
            let visible = hi - e.y;
            let sunk = e.y - lo;
            assert!(
                sunk < visible,
                "{kind:?} @{scale}: {sunk} m under ground against {visible} m \
                 above it — more of the chip is hidden than drawn"
            );
            // **A magnitude, not a sign** — and this assertion exists because
            // the first cut of this test did not have it. Proving the gate by
            // inversion found that `CHIP_SINK = 1e-9` passed everything above:
            // `lo < e.y` is true for any positive sink, so the suite was
            // green on a chip whose base ring is coplanar with the ground to
            // within a float, which is the exact defect it is named after.
            let buried = sunk / (hi - lo);
            assert!(
                (0.05..0.50).contains(&buried),
                "{kind:?} @{scale}: {buried} of the chip's height is under \
                 ground — under 0.05 the seam is still a straight line, over \
                 0.50 the chip is mostly a hole"
            );
        }
    }
}

/// **The correction.** `clutter.rs` justified forcing every blade normal fully
/// vertical by saying a blade's two triangles wind opposite ways, "so one of
/// them took the sun and the other went black". They do not wind opposite
/// ways, and this is the computation.
///
/// It matters because the false reason concealed the true cost. A fully
/// vertical normal is the ground's own normal, so every blade takes the same
/// sun cosine and the same hemisphere sample as the dirt beneath it — the
/// visual judge's "reads as paint", stated as arithmetic. The real mechanism
/// behind a black half-tuft is the material's `double_sided` flip, which is a
/// different problem with a different fix (a per-vertex root-to-tip normal
/// ramp), and it is not solved by throwing the facet away.
#[test]
fn a_blades_two_triangles_do_not_wind_opposite_ways() {
    // The quad `blade()` emits, rebuilt here from the same corner algebra, and
    // swept over the lean and yaw range the tuft actually draws.
    let (half_base, half_tip) = (0.030_f32, 0.008_f32);
    for yaw_step in 0..16 {
        let a = yaw_step as f32 / 16.0 * std::f32::consts::TAU;
        let dir = Vec2::new(a.sin(), a.cos());
        for lean_step in 0..8 {
            // `blade()` draws lean from 0.22..0.56 and height from
            // 0.55..1.25 of TUFT_H.
            let lean = 0.22 + 0.34 * (lean_step as f32 / 7.0);
            let h = TUFT_H * (0.55 + 0.7 * (lean_step as f32 / 7.0));

            let base = Vec3::new(0.0, 0.0, 0.0);
            let side = Vec3::new(-dir.y, 0.0, dir.x);
            let tip = base + Vec3::new(dir.x * lean * h, h, dir.y * lean * h);
            let (b0, b1) = (base - side * half_base, base + side * half_base);
            let (t0, t1) = (tip - side * half_tip, tip + side * half_tip);

            // The two calls `blade()` makes, in order.
            let f0 = (t0 - b0).cross(b1 - b0).normalize();
            let f1 = (t0 - b1).cross(t1 - b1).normalize();

            assert!(
                f0.dot(f1) > 0.99,
                "yaw {a} lean {lean}: the quad's facets point {f0} and {f1} \
                 — dot {}, which is the opposite-winding claim being true",
                f0.dot(f1)
            );
            // And neither of them is the ground's normal. The quad's normal
            // works out in closed form: `(h·dir.x, −lean·h, h·dir.y)`, so its
            // vertical component is `lean/√(1+lean²)` exactly, independent of
            // height — 0.215 at the minimum lean this draws and 0.489 at the
            // maximum. A blade's own facing is therefore predominantly
            // HORIZONTAL over the whole drawn range, and forcing the normal to
            // +Y throws the dominant component away. That is the cost the
            // false winding claim was hiding.
            let closed_form = lean / (1.0 + lean * lean).sqrt();
            assert!(
                (f0.y.abs() - closed_form).abs() < 1e-4,
                "yaw {a} lean {lean}: facet y {} is not the closed form {closed_form}",
                f0.y
            );
            assert!(
                f0.y.abs() < std::f32::consts::FRAC_1_SQRT_2,
                "yaw {a} lean {lean}: facet {f0} is more vertical than \
                 horizontal, so the blend below would be discarding little"
            );
        }
    }
}

/// **Gate: a blade shades like a blade at the tip and like the ground at the
/// root.**
///
/// The defect this closes is the one the test above measured the cost of
/// without fixing: `blade()` blended every vertex fully to the volume normal,
/// which over a 34 cm blade about a centre 2 m below it is within ~3° of
/// straight up — the GROUND's own normal. So a blade took the same sun cosine
/// and the same hemisphere sample as the dirt it stood in, on the layer that
/// fills the bottom half of every frame, and albedo was the only thing telling
/// grass from ground. `ART.md` §5 asks for the opposite in as many words:
/// "blades catch a rim of sun at their tips".
///
/// **Both ends are asserted, because both ends are load-bearing.** A root that
/// stopped being vertical would be `ART.md` rule 2 broken — the blade would
/// stop being bedded in the turf and start reading as a decal standing on it.
/// A tip that stays vertical is the paint defect. One number cannot satisfy
/// both, which is why `Soup::tri_ramp` exists.
#[test]
fn a_blade_separates_from_the_ground_it_grows_out_of() {
    let m = element_mesh(&elem(Clutter::Tuft, 91, 1.0));
    let (p, n) = (positions(&m), normals(&m));
    assert_eq!(p.len(), n.len());

    let y_max = p.iter().fold(f32::MIN, |a, v| a.max(v.y));
    let y_min = p.iter().fold(f32::MAX, |a, v| a.min(v.y));
    let span = y_max - y_min;
    assert!(span > 0.05, "a tuft with no height in it: span {span}");

    // Root band and tip band, by height. A blade is two triangles, so its four
    // corners appear six times between them; taking bands rather than indices
    // keeps this independent of the emit order.
    let band = |lo: f32, hi: f32| -> Vec<f32> {
        p.iter()
            .zip(&n)
            .filter(|(v, _)| {
                let t = (v.y - y_min) / span;
                t >= lo && t <= hi
            })
            .map(|(_, nn)| nn.y)
            .collect()
    };
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;

    let root = band(0.0, 0.10);
    let tip = band(0.90, 1.0);
    assert!(
        !root.is_empty() && !tip.is_empty(),
        "no vertices in one of the bands — root {} tip {}",
        root.len(),
        tip.len()
    );
    let (rm, tm) = (mean(&root), mean(&tip));

    // The root is bedded: essentially the ground's own normal.
    assert!(
        rm > 0.99,
        "a blade's ROOT normal averages y={rm:.4} — it must stay with the \
         ground it grows out of (ART.md rule 2), or the blade reads as a decal \
         standing on the turf rather than as something in it"
    );
    // The tip is not. This is the whole fix, and it is the assertion that goes
    // red on the shipped `blend = 1.0`.
    assert!(
        tm < 0.985,
        "a blade's TIP normal averages y={tm:.4} — that is the GROUND's normal, \
         so every blade takes the same sun cosine and the same hemisphere \
         sample as the dirt under it and only albedo separates them. This is \
         the visual judge's 'reads as paint' as arithmetic"
    );
    // …and the separation is a real one rather than float noise.
    assert!(
        rm - tm > 0.008,
        "root {rm:.4} and tip {tm:.4} differ by {:.4} — under this the ramp is \
         present in the code and absent from the mesh",
        rm - tm
    );
}
