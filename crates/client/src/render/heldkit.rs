//! The prop kit: every item with no model file of its own, built out of
//! primitives — the hand that holds it, the ground it is dropped on, and its
//! inventory icon all draw the same object (`ui::hold`'s `HeldSrc::Gen` rows
//! fall through `heldgen::mesh` to [`baked`]; `iconbake` photographs the
//! same mesh).
//!
//! A prop is authored as a list of [`Part`]s, each a mesh, a [`Mat`] and where
//! it sits, in metres with +Y up. Long things are authored standing, like the
//! held models. [`baked`] then makes it one mesh for one material, which is
//! what a held row is (`tests/held_assets.rs`), the way Before Colony's
//! `bc-model` bakes a suit's bone:
//!
//! - **The part's paint rides in the vertices.** Colour in the vertex colour;
//!   the surface's detail (grain, brushing, weave…, [`Tex`]) in a small atlas
//!   on UV0; its roughness and metalness as a texel of a 16 × 16 table on UV1.
//!   One [`material`] serves every prop, so wood and steel on one gun keep
//!   their own response instead of sharing the dominant one.
//! - **Edges catch the light.** Boxes are chamfered ([`cuboid`]) and the
//!   chamfer is a touch brighter, worn; a turned shape is smooth across a
//!   gentle bend and crisp at a corner ([`lathe`]). A sharp-edged primitive
//!   under a key light is the tell of a programmer's prop.
//! - **Crevices hold shadow.** Each part leaves a rough volume and every
//!   vertex looks out along its normal into its neighbours' (the
//!   signed-distance estimate `bc-model/src/ao.rs` uses), so the gap between
//!   two logs, the inside of a coil and the seat of a cylinder in its frame
//!   stay dark however the prop is lit.
//!
//! The materials' detail maps exist because a flat colour under three lights
//! reads as plastic, and plastic is the opposite of the reference's icons.

use std::f32::consts::{PI, TAU};

use bevy::asset::uuid_handle;
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// A surface's detail: a grey map the base colour multiplies.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Tex {
    Plain,
    Grain,
    Bark,
    Brushed,
    Weave,
    Speckle,
}

impl Tex {
    const ALL: [Tex; 6] = [
        Tex::Plain,
        Tex::Grain,
        Tex::Bark,
        Tex::Brushed,
        Tex::Weave,
        Tex::Speckle,
    ];
}

/// What a part is made of.
#[derive(Clone, Copy, Debug)]
struct Mat {
    /// sRGB, 0xRRGGBB.
    rgb: u32,
    rough: f32,
    metal: f32,
    tex: Tex,
}

const fn m(rgb: u32, rough: f32, metal: f32, tex: Tex) -> Mat {
    Mat {
        rgb,
        rough,
        metal,
        tex,
    }
}

impl Mat {
    fn color(&self) -> Color {
        Color::srgb_u8(
            (self.rgb >> 16) as u8,
            (self.rgb >> 8 & 0xff) as u8,
            (self.rgb & 0xff) as u8,
        )
    }
}

// ---- the palette ----------------------------------------------------------

const WOOD: Mat = m(0x9a6a3e, 0.72, 0.0, Tex::Grain);
const WOOD_LIGHT: Mat = m(0xc49a64, 0.72, 0.0, Tex::Grain);
const WOOD_DARK: Mat = m(0x6e4a2c, 0.75, 0.0, Tex::Grain);
const BARK: Mat = m(0x6a5038, 0.92, 0.0, Tex::Bark);
const STEEL: Mat = m(0xc4cad2, 0.32, 1.0, Tex::Brushed);
const DARK_STEEL: Mat = m(0x5a6068, 0.42, 1.0, Tex::Brushed);
const GUNMETAL: Mat = m(0x50565e, 0.38, 0.9, Tex::Brushed);
const BRASS: Mat = m(0xd4aa50, 0.3, 1.0, Tex::Brushed);
const COPPER: Mat = m(0xc06c40, 0.35, 1.0, Tex::Plain);
const CLOTH: Mat = m(0xd8c8a0, 0.95, 0.0, Tex::Weave);
const BURLAP: Mat = m(0xbca070, 0.95, 0.0, Tex::Weave);
const CHARRED_CLOTH: Mat = m(0x5a4636, 0.95, 0.0, Tex::Weave);
const ROPE: Mat = m(0xccaa70, 0.95, 0.0, Tex::Weave);
const LEATHER: Mat = m(0x94603a, 0.6, 0.0, Tex::Speckle);
const HIDE: Mat = m(0xa8784c, 0.75, 0.0, Tex::Speckle);
const BONE: Mat = m(0xece2c8, 0.55, 0.0, Tex::Speckle);
const STONE: Mat = m(0x8f8a84, 0.85, 0.0, Tex::Speckle);
const FLINT: Mat = m(0x6e6c72, 0.38, 0.0, Tex::Speckle);
const CHARCOAL: Mat = m(0x2c2c30, 0.55, 0.15, Tex::Speckle);
const RED_PAINT: Mat = m(0xb8362a, 0.45, 0.25, Tex::Speckle);
const YELLOW_PAINT: Mat = m(0xdcb82e, 0.45, 0.25, Tex::Speckle);
const WHITE_PAINT: Mat = m(0xdcdfe0, 0.4, 0.3, Tex::Speckle);
const BLUE_PAINT: Mat = m(0x3a6ab0, 0.45, 0.2, Tex::Speckle);
const PAPER_BLUE: Mat = m(0x4676b8, 0.85, 0.0, Tex::Speckle);
const PAPER_PALE: Mat = m(0xa8c4e8, 0.85, 0.0, Tex::Speckle);
const GLASS: Mat = m(0xc4dee6, 0.08, 0.0, Tex::Plain);
const PCB: Mat = m(0x2e6e3a, 0.5, 0.0, Tex::Speckle);
const BLACK: Mat = m(0x1e1e22, 0.45, 0.0, Tex::Plain);
const HOLE: Mat = m(0x0c0c0e, 0.9, 0.0, Tex::Plain);
const FEATHER: Mat = m(0xece6d8, 0.8, 0.0, Tex::Weave);
const FEATHER_RED: Mat = m(0xb83a30, 0.8, 0.0, Tex::Weave);
const FEATHER_BLUE: Mat = m(0x3a6ab8, 0.8, 0.0, Tex::Weave);
const SOIL: Mat = m(0x4a3524, 1.0, 0.0, Tex::Speckle);
const LEAF: Mat = m(0x5f9a3a, 0.55, 0.0, Tex::Plain);
const KERNEL: Mat = m(0xeec44a, 0.45, 0.0, Tex::Plain);
const HEMP: Mat = m(0x8c7a4c, 0.6, 0.0, Tex::Speckle);
const THREAD_RED: Mat = m(0xb03a32, 0.9, 0.0, Tex::Weave);
const THREAD_BLUE: Mat = m(0x3a5ea8, 0.9, 0.0, Tex::Weave);
// An LED and an ember are bright paint and nothing more: a carried emissive
// is a lamp in the dark (`tests/held_assets.rs::nothing_generated_glows_either`).
const LED_RED: Mat = m(0xff3a2a, 0.3, 0.0, Tex::Plain);
const LED_GREEN: Mat = m(0x58e070, 0.3, 0.0, Tex::Plain);
const EMBER: Mat = m(0xff8a2a, 0.6, 0.0, Tex::Speckle);

// ---- parts and placement ---------------------------------------------------

/// One piece of a prop.
struct Part {
    mesh: Mesh,
    mat: Mat,
    at: Transform,
}

fn p(mesh: Mesh, mat: Mat, at: Transform) -> Part {
    Part { mesh, mat, at }
}

/// Placed at `t`, turned by Euler degrees `r` (X, then Y, then Z).
fn xf(t: [f32; 3], r: [f32; 3]) -> Transform {
    Transform::from_xyz(t[0], t[1], t[2]).with_rotation(Quat::from_euler(
        EulerRot::XYZ,
        r[0].to_radians(),
        r[1].to_radians(),
        r[2].to_radians(),
    ))
}

/// [`xf`], scaled.
fn xfs(t: [f32; 3], r: [f32; 3], s: [f32; 3]) -> Transform {
    xf(t, r).with_scale(Vec3::from_array(s))
}

// ---- meshes ----------------------------------------------------------------

/// A cylinder standing on Y, centred.
fn cyl(r: f32, h: f32) -> Mesh {
    Cylinder::new(r, h).mesh().resolution(28).build()
}

/// A box, centred, its edges chamfered: `bc-model`'s `hexa`. Each face is
/// inset along its own edges, a strip joins two faces at every edge and a
/// small triangle closes every corner, each flat. The strips carry a vertex
/// colour a little over 1 ([`WORN`]): an edge is where paint wears and
/// steel is polished by hands, and it catches the key light as a line.
///
/// The chamfer is a fraction of the thinnest side, up to [`CHAMFER_MAX`]:
/// big enough to read at arm's length, never big enough to round a plate.
fn cuboid(x: f32, y: f32, z: f32) -> Mesh {
    let h = Vec3::new(x, y, z) * 0.5;
    let ch = (x.min(y).min(z) * 0.2).min(CHAMFER_MAX);
    // Corner `i` (bit 0 x, bit 1 y, bit 2 z: 1 the high side) moved in by the
    // chamfer along the two axes other than `a`, which keeps it on face `a`.
    let corner = |i: usize| {
        Vec3::new(
            if i & 1 != 0 { h.x } else { -h.x },
            if i & 2 != 0 { h.y } else { -h.y },
            if i & 4 != 0 { h.z } else { -h.z },
        )
    };
    let inset = |i: usize, a: usize| {
        let c = corner(i);
        let mut v = c;
        for b in 0..3 {
            if b != a {
                v[b] -= c[b].signum() * ch;
            }
        }
        v
    };
    let (mut pos, mut nrm, mut uv, mut col, mut idx) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    // A flat convex polygon, wound to face away from the centre.
    let mut poly = |verts: &[Vec3], shade: f32| {
        let c = verts.iter().copied().sum::<Vec3>() / verts.len() as f32;
        let mut n = Vec3::ZERO;
        for k in 0..verts.len() {
            n += verts[k].cross(verts[(k + 1) % verts.len()]);
        }
        let Some(mut n) = n.try_normalize() else {
            return;
        };
        let flip = n.dot(c) < 0.0;
        if flip {
            n = -n;
        }
        // Planar UVs off the face's own two widest axes, 0..1 across it.
        let (ax, bx) = match n.abs().max_position() {
            0 => (2, 1),
            1 => (0, 2),
            _ => (0, 1),
        };
        let base = pos.len() as u32;
        for v in verts {
            pos.push(v.to_array());
            nrm.push(n.to_array());
            uv.push([0.5 + v[ax] / (2.0 * h[ax]), 0.5 - v[bx] / (2.0 * h[bx])]);
            col.push([shade, shade, shade, 1.0]);
        }
        for k in 1..verts.len() as u32 - 1 {
            if flip {
                idx.extend([base, base + k + 1, base + k]);
            } else {
                idx.extend([base, base + k, base + k + 1]);
            }
        }
    };
    for a in 0..3 {
        let (b, d) = ((a + 1) % 3, (a + 2) % 3);
        for s in 0..2 {
            let q = [(0, 0), (1, 0), (1, 1), (0, 1)]
                .map(|(u, v)| inset((s << a) | (u << b) | (v << d), a));
            // In tiles, so the occlusion baked into the vertices has
            // somewhere to vary: a window pane with only its four corners,
            // all tucked under the frame, went grey edge to edge.
            let tiles = |l: f32| ((l / FACE_TILE).ceil() as usize).clamp(1, 8);
            let (nu, nv) = (tiles(q[0].distance(q[1])), tiles(q[0].distance(q[3])));
            let at = |i: usize, j: usize| {
                let (u, v) = (i as f32 / nu as f32, j as f32 / nv as f32);
                q[0].lerp(q[1], u).lerp(q[3].lerp(q[2], u), v)
            };
            for j in 0..nv {
                for i in 0..nu {
                    poly(
                        &[at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)],
                        1.0,
                    );
                }
            }
        }
    }
    if ch > 0.0 {
        for e in 0..3 {
            let (b, d) = ((e + 1) % 3, (e + 2) % 3);
            for i in 0..8usize {
                if i & (1 << e) != 0 {
                    continue;
                }
                let j = i | (1 << e);
                poly(&[inset(i, b), inset(j, b), inset(j, d), inset(i, d)], WORN);
            }
        }
        for i in 0..8 {
            poly(&[inset(i, 0), inset(i, 1), inset(i, 2)], WORN);
        }
    }
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    m.insert_indices(Indices::U32(idx));
    m
}

/// The largest chamfer on a [`cuboid`], metres.
const CHAMFER_MAX: f32 = 0.004;

/// The largest tile a [`cuboid`]'s face is cut into, metres.
const FACE_TILE: f32 = 0.04;

/// How much brighter a chamfer is than the faces it joins.
const WORN: f32 = 1.12;

fn sphere(r: f32) -> Mesh {
    Sphere::new(r).mesh().uv(28, 18)
}

/// A cone standing on Y, point up, centred.
fn cone(r: f32, h: f32) -> Mesh {
    Cone::new(r, h).mesh().resolution(28).build()
}

/// A frustum standing on Y, centred.
fn frustum(top: f32, bottom: f32, h: f32) -> Mesh {
    ConicalFrustum {
        radius_top: top,
        radius_bottom: bottom,
        height: h,
    }
    .mesh()
    .resolution(28)
    .build()
}

/// A torus lying in XZ.
fn torus(minor: f32, major: f32) -> Mesh {
    Torus {
        minor_radius: minor,
        major_radius: major,
    }
    .mesh()
    .minor_resolution(14)
    .major_resolution(36)
    .build()
}

/// A prism with `n` sides, extruded along Z.
fn prism(n: u32, r: f32, depth: f32) -> Mesh {
    Extrusion::new(RegularPolygon::new(r, n), depth)
        .mesh()
        .build()
}

/// A hollow tube along Z.
fn pipe(inner: f32, outer: f32, len: f32) -> Mesh {
    Extrusion::new(Annulus::new(inner, outer), len)
        .mesh()
        .build()
}

fn mesh(pos: Vec<[f32; 3]>, uv: Vec<[f32; 2]>, idx: Vec<u32>) -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_indices(Indices::U32(idx));
    m.compute_smooth_normals();
    m
}

/// A tube swept along `path`, its radius `r(t)` over `t` in 0..1, capped.
/// Frames are parallel-transported, so a coil does not twist.
///
/// A long path is resampled to [`TUBE_MAX`] rings: past a few hundred the
/// tube came out black under lavapipe whatever its material (measured: a
/// 221-ring coil black, the same coil at 61 rings lit), and a coil reads
/// the same at either.
fn tube(path: &[Vec3], r: impl Fn(f32) -> f32, sides: u32) -> Mesh {
    let owned: Vec<Vec3>;
    let path = if path.len() > TUBE_MAX {
        owned = (0..TUBE_MAX)
            .map(|i| path[i * (path.len() - 1) / (TUBE_MAX - 1)])
            .collect();
        &owned[..]
    } else {
        path
    };
    let n = path.len();
    let tangent = |i: usize| {
        let a = path[i.saturating_sub(1)];
        let b = path[(i + 1).min(n - 1)];
        (b - a).normalize_or(Vec3::Y)
    };
    let t0 = tangent(0);
    let mut nrm = t0.any_orthonormal_vector();
    let (mut pos, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    let s = sides as usize;
    for (i, c) in path.iter().enumerate() {
        let t = tangent(i);
        nrm = (nrm - t * nrm.dot(t)).normalize_or(t.any_orthonormal_vector());
        let bin = t.cross(nrm);
        let rad = r(i as f32 / (n - 1).max(1) as f32);
        for k in 0..=s {
            let a = k as f32 / s as f32 * TAU;
            let d = nrm * a.cos() + bin * a.sin();
            pos.push((*c + d * rad).to_array());
            uv.push([k as f32 / s as f32, i as f32 / (n - 1).max(1) as f32]);
        }
    }
    let w = (s + 1) as u32;
    for i in 0..(n as u32 - 1) {
        for k in 0..s as u32 {
            let a = i * w + k;
            let b = a + w;
            idx.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    // Caps, as fans on their own vertices so the rim stays sharp.
    for (end, t) in [(0usize, -t0), (n - 1, tangent(n - 1))] {
        let centre = pos.len() as u32;
        pos.push(path[end].to_array());
        uv.push([0.5, 0.5]);
        let ring = end as u32 * w;
        for k in 0..s as u32 {
            let base = pos.len() as u32;
            pos.push(pos[(ring + k) as usize]);
            pos.push(pos[(ring + k + 1) as usize]);
            uv.extend([[0.0, 0.0], [1.0, 0.0]]);
            // The ring turns counter-clockwise about the tangent, so the far
            // end's fan faces out as wound and the near end's is reversed.
            let _ = t;
            if end == n - 1 {
                idx.extend([centre, base, base + 1]);
            } else {
                idx.extend([centre, base + 1, base]);
            }
        }
    }
    let mut m = mesh(pos, uv, idx);
    // A tube's caps want flat normals and its sides smooth ones; the side
    // smoothing above is right, and a cap seen at 48 px is a dot.
    m.compute_smooth_normals();
    m
}

/// The most rings a [`tube`] is built with.
const TUBE_MAX: usize = 96;

/// A helix around Y: `turns` loops of `radius`, rising `pitch` a loop.
fn helix(radius: f32, turns: f32, pitch: f32, steps: usize) -> Vec<Vec3> {
    (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            let a = t * turns * TAU;
            Vec3::new(radius * a.cos(), t * turns * pitch, radius * a.sin())
        })
        .collect()
}

/// An arc in the XY plane from `a0` to `a1` radians, centred on `c`.
fn arc(c: Vec3, radius: f32, a0: f32, a1: f32, steps: usize) -> Vec<Vec3> {
    (0..=steps)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / steps as f32;
            c + Vec3::new(radius * a.cos(), radius * a.sin(), 0.0)
        })
        .collect()
}

/// A slab cut from `outline` (XY, star-shaped about its centroid), `depth`
/// thick on Z, its front domed by `puff` — a hide, a blade, a shirt laid
/// flat. Fans from the centroid, so the outline must see its centre.
fn slab(outline: &[Vec2], depth: f32, puff: f32) -> Mesh {
    let n = outline.len();
    let c = outline.iter().copied().sum::<Vec2>() / n as f32;
    let (lo, hi) = outline.iter().fold((Vec2::MAX, Vec2::MIN), |(lo, hi), v| {
        (lo.min(*v), hi.max(*v))
    });
    let span = (hi - lo).max(Vec2::splat(1e-4));
    let uvf = |v: Vec2| ((v - lo) / span).to_array();
    let (mut pos, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    let half = depth * 0.5;
    // Front: centre, an inner ring at 55%, the outline; domed.
    for (sign, z_ring, z_mid, z_c) in [
        (1.0f32, half, half + puff * 0.75, half + puff),
        (-1.0, -half, -half - puff * 0.3, -half - puff * 0.4),
    ] {
        let base = pos.len() as u32;
        pos.push([c.x, c.y, z_c]);
        uv.push(uvf(c));
        for v in outline {
            let mid = c + (*v - c) * 0.55;
            pos.push([mid.x, mid.y, z_mid]);
            uv.push(uvf(mid));
        }
        for v in outline {
            pos.push([v.x, v.y, z_ring]);
            uv.push(uvf(*v));
        }
        let n32 = n as u32;
        for k in 0..n32 {
            let k1 = (k + 1) % n32;
            let (m0, m1) = (base + 1 + k, base + 1 + k1);
            let (o0, o1) = (base + 1 + n32 + k, base + 1 + n32 + k1);
            let tris = [[base, m0, m1], [m0, o0, o1], [m0, o1, m1]];
            for t in tris {
                if sign > 0.0 {
                    idx.extend(t);
                } else {
                    idx.extend([t[0], t[2], t[1]]);
                }
            }
        }
    }
    // The wall, on its own vertices.
    for k in 0..n {
        let (a, b) = (outline[k], outline[(k + 1) % n]);
        let base = pos.len() as u32;
        pos.extend([
            [a.x, a.y, half],
            [b.x, b.y, half],
            [b.x, b.y, -half],
            [a.x, a.y, -half],
        ]);
        uv.extend([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        idx.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    let mut m = mesh(pos, uv, idx);
    // The CCW outline's front faces +Z; a clockwise one was handed in.
    let area: f32 = (0..n)
        .map(|k| {
            let (a, b) = (outline[k], outline[(k + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    if area < 0.0 {
        if let Some(Indices::U32(ix)) = m.indices_mut() {
            for t in ix.chunks_mut(3) {
                t.swap(1, 2);
            }
        }
        m.compute_smooth_normals();
    }
    m
}

/// A shape turned on Y from a `(radius, height)` profile, bottom to top. An
/// open foot is closed on the axis, so a pile held up to the eye has a floor.
///
/// Normals are `bc-model`'s rule: across a bend gentler than [`SMOOTH_BEND`]
/// the two segments share one, so a dome or a tank's shoulder shades as one
/// curve; at a sharper corner (a rim, a foot) each keeps its own and the
/// edge stays crisp. One smoothing over the whole profile rounded every rim
/// into a blur.
fn lathe(profile: &[(f32, f32)], sides: u32) -> Mesh {
    let mut prof: Vec<(f32, f32)> = profile.to_vec();
    if prof.first().is_some_and(|p| p.0 > 0.0) {
        prof.insert(0, (0.0, prof[0].1));
    }
    // Each segment's outward normal in the (r, y) half-plane.
    let normals: Vec<Option<Vec2>> = prof
        .windows(2)
        .map(|w| Vec2::new(w[1].1 - w[0].1, -(w[1].0 - w[0].0)).try_normalize())
        .collect();
    let at_end = |own: Vec2, other: Option<Vec2>| match other {
        Some(o) if own.angle_to(o).abs() < SMOOTH_BEND => (own + o).normalize_or(own),
        _ => own,
    };
    // V by arc length, so a weave does not bunch up on a short segment.
    let mut arc = vec![0.0f32];
    for w in prof.windows(2) {
        let d = Vec2::new(w[1].0 - w[0].0, w[1].1 - w[0].1).length();
        arc.push(arc.last().copied().unwrap_or(0.0) + d);
    }
    let total = arc.last().copied().unwrap_or(1.0).max(1e-6);
    let s = sides as usize;
    let (mut pos, mut nrm, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (i, w) in prof.windows(2).enumerate() {
        let Some(own) = normals[i] else { continue };
        let prev = i.checked_sub(1).and_then(|j| normals[j]);
        let next = normals.get(i + 1).copied().flatten();
        let ends = [
            (w[0], at_end(own, prev), arc[i]),
            (w[1], at_end(own, next), arc[i + 1]),
        ];
        let base = pos.len() as u32;
        for k in 0..=s {
            let a = k as f32 / s as f32 * TAU;
            let (sin, cos) = a.sin_cos();
            for ((r, y), m, len) in ends {
                pos.push([r * cos, y, r * sin]);
                nrm.push(
                    Vec3::new(m.x * cos, m.y, m.x * sin)
                        .normalize_or(Vec3::Y)
                        .to_array(),
                );
                uv.push([k as f32 / s as f32, len / total]);
            }
        }
        let (r0, r1) = (w[0].0, w[1].0);
        for k in 0..s as u32 {
            let (a, b, c, d) = (
                base + 2 * k,
                base + 2 * k + 1,
                base + 2 * k + 3,
                base + 2 * k + 2,
            );
            // Ring k's foot `a` and head `b`, the next angle's head `c` and
            // foot `d`; a zero radius is a pole and its half is skipped.
            if r1 > 0.0 {
                idx.extend([a, b, c]);
            }
            if r0 > 0.0 {
                idx.extend([a, c, d]);
            }
        }
    }
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_indices(Indices::U32(idx));
    m
}

/// Where a turned profile bends by less than this, radians, it shades
/// smooth (`bc-model/src/kit.rs`'s `SMOOTH_BEND`).
const SMOOTH_BEND: f32 = 0.6;

/// A stone, a lump of charcoal, a shard: an icosphere pushed in and out by
/// a hash of where each corner is, faceted.
fn lump(seed: u32, rough: f32) -> Mesh {
    let mut m = Sphere::new(1.0).mesh().ico(2).expect("ico(2) is in range");
    if let Some(bevy::mesh::VertexAttributeValues::Float32x3(ps)) =
        m.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    {
        for v in ps.iter_mut() {
            let p = Vec3::from_array(*v);
            let k = hash3(p * 2.3, seed) * 0.6 + hash3(p * 5.1, seed ^ 0x9e37) * 0.4;
            *v = (p * (1.0 + rough * (k - 0.5))).to_array();
        }
    }
    m.duplicate_vertices();
    m.compute_flat_normals();
    m
}

/// [`lump`], smooth: fat, a waterskin's belly.
fn blob(seed: u32, rough: f32) -> Mesh {
    blob_n(seed, rough, 3)
}

/// [`blob`] at `subdiv` icosphere subdivisions: a seed a centimetre across
/// needs 80 faces, not the 1,280 a waterskin's belly does.
fn blob_n(seed: u32, rough: f32, subdiv: u32) -> Mesh {
    let mut m = Sphere::new(1.0)
        .mesh()
        .ico(subdiv)
        .expect("subdivisions in range");
    if let Some(bevy::mesh::VertexAttributeValues::Float32x3(ps)) =
        m.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    {
        for v in ps.iter_mut() {
            let p = Vec3::from_array(*v);
            let k = hash3(p * 1.6, seed);
            *v = (p * (1.0 + rough * (k - 0.5))).to_array();
        }
    }
    m.compute_smooth_normals();
    m
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// A value in 0..1 for a point, stable per `seed`, smooth enough between
/// nearby points (trilinear over a unit lattice).
fn hash3(p: Vec3, seed: u32) -> f32 {
    let f = p.floor();
    let t = p - f;
    let t = t * t * (Vec3::splat(3.0) - 2.0 * t);
    let at = |dx: i32, dy: i32, dz: i32| {
        let h = hash(
            (f.x as i32 + dx) as u32
                ^ hash((f.y as i32 + dy) as u32 ^ hash((f.z as i32 + dz) as u32 ^ seed)),
        );
        (h & 0xffff) as f32 / 65535.0
    };
    let x00 = at(0, 0, 0) + (at(1, 0, 0) - at(0, 0, 0)) * t.x;
    let x10 = at(0, 1, 0) + (at(1, 1, 0) - at(0, 1, 0)) * t.x;
    let x01 = at(0, 0, 1) + (at(1, 0, 1) - at(0, 0, 1)) * t.x;
    let x11 = at(0, 1, 1) + (at(1, 1, 1) - at(0, 1, 1)) * t.x;
    let y0 = x00 + (x10 - x00) * t.y;
    let y1 = x01 + (x11 - x01) * t.y;
    y0 + (y1 - y0) * t.z
}

/// A deterministic jitter in -1..1 for prop layout.
fn jit(i: u32, salt: u32) -> f32 {
    (hash(i.wrapping_mul(0x9e37_79b9) ^ salt) & 0xffff) as f32 / 32767.5 - 1.0
}

// ---- the detail maps ---------------------------------------------------------

/// The detail map for a [`Tex`] at `(u, v)` in 0..1: a grey the base colour
/// multiplies, mean near 0.85 (sRGB) so the colour keeps its value.
fn detail(tex: Tex, u: f32, v: f32) -> f32 {
    let fbm = |x: f32, y: f32, seed: u32| {
        let mut v = 0.0;
        let mut amp = 0.5;
        let mut f = 1.0;
        for o in 0..4 {
            v += amp * hash3(Vec3::new(x * f, y * f, 0.37), seed + o);
            amp *= 0.5;
            f *= 2.0;
        }
        v
    };
    let g = match tex {
        Tex::Plain => 0.92 + 0.08 * fbm(u * 6.0, v * 6.0, 1),
        // Rings run around a cylinder's U; the grain runs along V.
        Tex::Grain => {
            let w = fbm(u * 3.0, v * 0.8, 2) * 9.0;
            let ring = 0.5 + 0.5 * (u * 46.0 + w).sin();
            0.74 + 0.18 * ring + 0.08 * fbm(u * 40.0, v * 3.0, 3)
        }
        Tex::Bark => {
            let ridge = 1.0 - (fbm(u * 10.0, v * 2.5, 4) * 2.0 - 1.0).abs();
            0.45 + 0.5 * ridge.powf(2.0) + 0.1 * fbm(u * 30.0, v * 30.0, 5)
        }
        Tex::Brushed => 0.84 + 0.12 * fbm(u * 2.0, v * 90.0, 6) + 0.04 * fbm(u * 20.0, v * 20.0, 7),
        Tex::Weave => {
            let k = 72.0 * PI;
            let w = (u * k).sin() * (v * k).sin();
            0.8 + 0.1 * w + 0.1 * fbm(u * 12.0, v * 12.0, 8)
        }
        Tex::Speckle => {
            0.72 + 0.2 * fbm(u * 24.0, v * 24.0, 9) + 0.08 * fbm(u * 90.0, v * 90.0, 10)
        }
    };
    g.clamp(0.0, 1.0)
}

// ---- the props -------------------------------------------------------------

/// Every prop in the kit, by the item stem it draws.
pub const NAMES: &[&str] = &[
    "wooden_arrow",
    "bone_arrow",
    "metal_arrow",
    "high_velocity_arrow",
    "fire_arrow",
    "stone_spear",
    "torch",
    "revolver",
    "crossbow",
    "hunting_bow",
    "gears",
    "rope",
    "metal_blade",
    "empty_propane_tank",
    "sewing_kit",
    "metal_spring",
    "sheet_metal",
    "metal_pipe",
    "road_signs",
    "tech_trash",
    "electric_fuse",
    "blueprint",
    "key_lock",
    "code_lock",
    "pistol_round",
    "medical_syringe",
    "corn_seed",
    "hemp_seed",
    "burlap_tunic",
    "burlap_hood",
    "hide_poncho",
    "bone_helmet",
    "roadsign_vest",
    "planter_box",
    "auto_turret",
    "metal_window_bars",
    "garage_door",
    "glass_window",
    "camp_fire",
    "wood_shutters",
    // Rust's tree items (2026-10-17).
    "salvaged_hammer",
    "salvaged_sword",
    "mace",
    "salvaged_cleaver",
    "longsword",
    "salvaged_axe",
    "salvaged_icepick",
    "coffee_can_helmet",
    "tank_top",
    "beancan_grenade",
];

/// The parts of the prop called `name`, or `None` for a name with none.
fn parts(name: &str) -> Option<Vec<Part>> {
    Some(match name {
        "wooden_arrow" => arrow(WOOD_DARK, Head::Wood, FEATHER),
        "bone_arrow" => arrow(WOOD, Head::Bone, FEATHER),
        "metal_arrow" => arrow(WOOD, Head::Metal, FEATHER_RED),
        "high_velocity_arrow" => arrow(WOOD_LIGHT, Head::Needle, FEATHER_BLUE),
        "fire_arrow" => arrow(WOOD, Head::Fire, FEATHER_RED),
        "stone_spear" => stone_spear(),
        "torch" => torch(),
        "revolver" => revolver(),
        "crossbow" => crossbow(),
        "hunting_bow" => bow(),
        "gears" => gears(),
        "rope" => rope(),
        "metal_blade" => blade(),
        "empty_propane_tank" => propane(),
        "sewing_kit" => sewing(),
        "metal_spring" => spring(),
        "sheet_metal" => sheets(),
        "metal_pipe" => metal_pipe(),
        "road_signs" => road_signs(),
        "tech_trash" => tech_trash(),
        "electric_fuse" => fuse(),
        "blueprint" => blueprint(),
        "key_lock" => padlock(),
        "code_lock" => code_lock(),
        "pistol_round" => rounds(),
        "medical_syringe" => syringe(),
        "corn_seed" => seeds(KERNEL, 0.012, 21),
        "hemp_seed" => seeds(HEMP, 0.009, 22),
        "burlap_tunic" => stood(shirt(BURLAP)),
        "burlap_hood" => hood(),
        "hide_poncho" => stood(poncho()),
        "bone_helmet" => bone_helmet(),
        "roadsign_vest" => stood(roadsign_vest()),
        "planter_box" => planter(),
        "auto_turret" => turret(),
        // The window inserts: the game's own solids, in a material here.
        "metal_window_bars" => insert(sim_core::deploy::ARCH_WINDOW_BARS, DARK_STEEL),
        "garage_door" => garage_door(),
        "glass_window" => glass_window(),
        "camp_fire" => fire_pit(),
        "wood_shutters" => insert(sim_core::deploy::ARCH_WINDOW_SHUTTER, WOOD),
        "salvaged_hammer" => salvaged_hammer(),
        "salvaged_sword" => sword(0.78, 0.035, 0.13, CHARRED_CLOTH),
        "mace" => mace(),
        "salvaged_cleaver" => cleaver(),
        "longsword" => sword(1.1, 0.03, 0.2, LEATHER),
        "salvaged_axe" => salvaged_axe(),
        "salvaged_icepick" => salvaged_icepick(),
        "coffee_can_helmet" => coffee_can(),
        "tank_top" => stood(tank_top()),
        "beancan_grenade" => beancan(),
        _ => return None,
    })
}

#[derive(Clone, Copy)]
enum Head {
    Wood,
    Bone,
    Metal,
    Needle,
    Fire,
}

/// An arrow standing on Y: shaft, head, three vanes, a nock. The shaft is
/// drawn thicker than an arrow's (a third again) so it holds at 48 px.
fn arrow(shaft: Mat, head: Head, vane: Mat) -> Vec<Part> {
    let len = 0.72;
    let r = 0.016;
    let mut out = vec![
        p(cyl(r, len), shaft, xf([0.0, len * 0.5, 0.0], [0.0; 3])),
        p(cyl(r * 1.2, 0.02), BLACK, xf([0.0, 0.01, 0.0], [0.0; 3])),
    ];
    match head {
        Head::Wood => out.push(p(
            cone(r * 1.15, 0.07),
            WOOD_DARK,
            xf([0.0, len + 0.035, 0.0], [0.0; 3]),
        )),
        Head::Bone => out.push(p(
            cone(0.03, 0.11),
            BONE,
            xf([0.0, len + 0.05, 0.0], [0.0; 3]),
        )),
        Head::Metal => {
            let blade = [
                Vec2::new(0.0, 0.15),
                Vec2::new(-0.045, 0.0),
                Vec2::new(-0.012, 0.012),
                Vec2::new(0.0, -0.01),
                Vec2::new(0.012, 0.012),
                Vec2::new(0.045, 0.0),
            ];
            out.push(p(
                slab(&blade, 0.006, 0.004),
                STEEL,
                xf([0.0, len - 0.01, 0.0], [0.0; 3]),
            ));
        }
        Head::Needle => out.push(p(
            cone(r * 1.1, 0.14),
            DARK_STEEL,
            xf([0.0, len + 0.07, 0.0], [0.0; 3]),
        )),
        Head::Fire => {
            out.push(p(
                cone(0.02, 0.07),
                DARK_STEEL,
                xf([0.0, len + 0.035, 0.0], [0.0; 3]),
            ));
            out.push(p(
                frustum(0.024, 0.026, 0.07),
                CHARRED_CLOTH,
                xf([0.0, len - 0.04, 0.0], [0.0; 3]),
            ));
            out.push(p(
                frustum(0.02, 0.024, 0.02),
                EMBER,
                xf([0.0, len + 0.0, 0.0], [0.0; 3]),
            ));
        }
    }
    let vane_outline = [
        Vec2::new(0.0, 0.0),
        Vec2::new(0.05, 0.03),
        Vec2::new(0.05, 0.15),
        Vec2::new(0.0, 0.17),
    ];
    for k in 0..3 {
        let a = k as f32 * 120.0;
        let rot = Quat::from_rotation_y(a.to_radians());
        out.push(p(
            slab(&vane_outline, 0.003, 0.0),
            vane,
            Transform::from_translation(rot * Vec3::new(r * 0.6, 0.03, 0.0)).with_rotation(rot),
        ));
    }
    out
}

fn stone_spear() -> Vec<Part> {
    let len = 1.5;
    vec![
        p(cyl(0.02, len), WOOD, xf([0.0, len * 0.5, 0.0], [0.0; 3])),
        p(
            lump(31, 0.35),
            FLINT,
            xfs(
                [0.0, len + 0.07, 0.0],
                [0.0, 20.0, 0.0],
                [0.045, 0.13, 0.022],
            ),
        ),
        p(cyl(0.026, 0.07), ROPE, xf([0.0, len - 0.03, 0.0], [0.0; 3])),
    ]
}

/// A torch, unlit: the reference draws it cold, a wrapped head on a stick.
fn torch() -> Vec<Part> {
    vec![
        p(cyl(0.024, 0.46), WOOD, xf([0.0, 0.23, 0.0], [0.0; 3])),
        p(
            frustum(0.05, 0.036, 0.15),
            CHARRED_CLOTH,
            xf([0.0, 0.5, 0.0], [0.0; 3]),
        ),
        p(cyl(0.049, 0.012), CHARCOAL, xf([0.0, 0.58, 0.0], [0.0; 3])),
        p(torus(0.008, 0.042), ROPE, xf([0.0, 0.47, 0.0], [0.0; 3])),
        p(torus(0.008, 0.046), ROPE, xf([0.0, 0.53, 0.0], [0.0; 3])),
    ]
}

/// A revolver along +X, muzzle at +X, its side to +Z.
fn revolver() -> Vec<Part> {
    let mut out = vec![
        // Barrel, its rib, and the muzzle.
        p(
            cyl(0.011, 0.15),
            GUNMETAL,
            xf([0.115, 0.06, 0.0], [0.0, 0.0, 90.0]),
        ),
        p(
            cuboid(0.15, 0.008, 0.008),
            GUNMETAL,
            xf([0.115, 0.072, 0.0], [0.0; 3]),
        ),
        p(
            cyl(0.006, 0.004),
            HOLE,
            xf([0.191, 0.06, 0.0], [0.0, 0.0, 90.0]),
        ),
        // The cylinder and its pin.
        p(
            cyl(0.027, 0.048),
            DARK_STEEL,
            xf([0.02, 0.05, 0.0], [0.0, 0.0, 90.0]),
        ),
        p(
            cyl(0.006, 0.054),
            GUNMETAL,
            xf([0.02, 0.05, 0.0], [0.0, 0.0, 90.0]),
        ),
        // Frame and top strap.
        p(
            cuboid(0.09, 0.05, 0.022),
            GUNMETAL,
            xf([0.0, 0.04, 0.0], [0.0; 3]),
        ),
        p(
            cuboid(0.07, 0.012, 0.018),
            GUNMETAL,
            xf([0.03, 0.084, 0.0], [0.0; 3]),
        ),
        // Hammer, trigger, grip.
        p(
            cuboid(0.016, 0.03, 0.01),
            DARK_STEEL,
            xf([-0.045, 0.078, 0.0], [0.0, 0.0, 25.0]),
        ),
        p(
            cuboid(0.006, 0.026, 0.008),
            DARK_STEEL,
            xf([-0.012, 0.0, 0.0], [0.0, 0.0, -10.0]),
        ),
        p(
            cuboid(0.04, 0.115, 0.03),
            WOOD_DARK,
            xf([-0.06, -0.035, 0.0], [0.0, 0.0, 22.0]),
        ),
    ];
    // The trigger guard.
    out.push(p(
        tube(
            &arc(Vec3::new(-0.01, 0.0, 0.0), 0.022, PI, TAU, 16),
            |_| 0.004,
            8,
        ),
        GUNMETAL,
        Transform::IDENTITY,
    ));
    out
}

/// A crossbow standing on Y, the bow across X at the top.
fn crossbow() -> Vec<Part> {
    let limb: Vec<Vec3> = (0..=24)
        .map(|i| {
            let x = -0.3 + 0.6 * i as f32 / 24.0;
            let k = x / 0.3;
            Vec3::new(x, 0.56 - 0.07 * k * k, 0.0)
        })
        .collect();
    let (l, r) = (limb[0], limb[24]);
    let nock = Vec3::new(0.0, 0.4, 0.0);
    vec![
        p(
            cuboid(0.05, 0.62, 0.05),
            WOOD,
            xf([0.0, 0.29, 0.0], [0.0; 3]),
        ),
        p(
            cuboid(0.055, 0.2, 0.06),
            WOOD_DARK,
            xf([0.0, 0.05, 0.0], [0.0, 0.0, 0.0]),
        ),
        p(
            tube(&limb, |t| 0.016 - 0.008 * (2.0 * t - 1.0).abs(), 10),
            DARK_STEEL,
            Transform::IDENTITY,
        ),
        p(
            tube(&[l, nock, r], |_| 0.003, 6),
            CLOTH,
            Transform::IDENTITY,
        ),
        p(
            torus(0.006, 0.035),
            DARK_STEEL,
            xf([0.0, 0.64, 0.0], [90.0, 0.0, 0.0]),
        ),
        p(
            cuboid(0.012, 0.04, 0.012),
            DARK_STEEL,
            xf([0.0, 0.16, 0.03], [20.0, 0.0, 0.0]),
        ),
    ]
}

/// A hunting bow standing on Y: a recurved stave, the grip wrapped, and the
/// string — the reference's bow is a whole bow, string and all.
fn bow() -> Vec<Part> {
    let h = 1.2;
    let stave: Vec<Vec3> = (0..=40)
        .map(|i| {
            let t = i as f32 / 40.0;
            let y = t * h;
            let k = 2.0 * t - 1.0;
            // The belly bows out to +X; the tips recurve back.
            let x = 0.16 * (1.0 - k * k) - 0.04 * k.powi(8);
            Vec3::new(x, y, 0.0)
        })
        .collect();
    let (a, b) = (stave[0], stave[40]);
    vec![
        p(
            tube(&stave, |t| 0.036 - 0.02 * (2.0 * t - 1.0).abs(), 12),
            WOOD_LIGHT,
            Transform::IDENTITY,
        ),
        p(cyl(0.04, 0.15), LEATHER, xf([0.16, h * 0.5, 0.0], [0.0; 3])),
        p(tube(&[a, b], |_| 0.006, 6), CLOTH, Transform::IDENTITY),
    ]
}

/// A heap of `n` faceted lumps of `mat`, about `r` each.
fn heap(mat: Mat, n: u32, r: f32, seed: u32) -> Vec<Part> {
    (0..n)
        .map(|i| {
            let ring = if i == 0 { 0.0 } else { r * 1.6 };
            let a = i as f32 / (n - 1).max(1) as f32 * TAU + jit(i, seed) * 0.4;
            let y = if i == 0 { r * 1.6 } else { r * 0.7 };
            let s = r * (0.85 + 0.3 * jit(i, seed + 1).abs());
            p(
                lump(seed * 31 + i, 0.55),
                mat,
                xfs(
                    [ring * a.cos(), y, ring * a.sin()],
                    [jit(i, seed + 2) * 90.0, jit(i, seed + 3) * 180.0, 0.0],
                    [s, s * 0.8, s * 0.9],
                ),
            )
        })
        .collect()
}

/// A garment authored laid back 20° for the icon's camera, stood up straight
/// so a hand carries it by the shoulders; `iconbake` tips it back again.
fn stood(parts: Vec<Part>) -> Vec<Part> {
    place(parts, xf([0.0; 3], [20.0, 0.0, 0.0])).collect()
}

fn place(parts: Vec<Part>, at: Transform) -> impl Iterator<Item = Part> {
    parts.into_iter().map(move |mut part| {
        part.at = at * part.at;
        part
    })
}

/// A gear in XY: a disc, `teeth` teeth, a hub and its bore.
fn gear(r: f32, teeth: u32, mat: Mat) -> Vec<Part> {
    let t = r * 0.32;
    let mut out = vec![
        p(cyl(r, t), mat, xf([0.0; 3], [90.0, 0.0, 0.0])),
        p(
            cyl(r * 0.38, t * 1.25),
            DARK_STEEL,
            xf([0.0; 3], [90.0, 0.0, 0.0]),
        ),
        p(cyl(r * 0.16, t * 1.3), HOLE, xf([0.0; 3], [90.0, 0.0, 0.0])),
    ];
    for k in 0..teeth {
        let a = k as f32 / teeth as f32 * TAU;
        out.push(p(
            cuboid(r * 0.26, r * 0.26, t),
            mat,
            Transform::from_xyz(a.cos() * r * 1.05, a.sin() * r * 1.05, 0.0)
                .with_rotation(Quat::from_rotation_z(a)),
        ));
    }
    out
}

fn gears() -> Vec<Part> {
    let mut out: Vec<Part> = gear(0.1, 12, STEEL);
    out.extend(place(
        gear(0.065, 9, DARK_STEEL),
        xf([0.15, -0.07, 0.03], [0.0, 0.0, 12.0]),
    ));
    out
}

fn rope() -> Vec<Part> {
    let coil = helix(0.11, 3.6, 0.03, 220);
    let last = *coil.last().expect("a coil");
    let tail = [
        last,
        last + Vec3::new(0.05, -0.02, 0.04),
        last + Vec3::new(0.1, -0.08, 0.1),
        last + Vec3::new(0.12, -0.11, 0.16),
    ];
    vec![
        p(tube(&coil, |_| 0.017, 10), ROPE, Transform::IDENTITY),
        p(tube(&tail, |_| 0.017, 10), ROPE, Transform::IDENTITY),
    ]
}

fn blade() -> Vec<Part> {
    let outline = [
        Vec2::new(0.0, 0.42),
        Vec2::new(-0.03, 0.34),
        Vec2::new(-0.035, 0.1),
        Vec2::new(-0.012, 0.08),
        Vec2::new(-0.012, 0.0),
        Vec2::new(0.012, 0.0),
        Vec2::new(0.012, 0.08),
        Vec2::new(0.035, 0.1),
        Vec2::new(0.035, 0.3),
    ];
    vec![
        p(slab(&outline, 0.008, 0.004), STEEL, Transform::IDENTITY),
        p(cyl(0.016, 0.07), LEATHER, xf([0.0, 0.035, 0.0], [0.0; 3])),
    ]
}

fn propane() -> Vec<Part> {
    vec![
        p(
            lathe(
                &[
                    (0.0, 0.0),
                    (0.09, 0.0),
                    (0.1, 0.03),
                    (0.1, 0.22),
                    (0.085, 0.27),
                    (0.04, 0.3),
                    (0.0, 0.305),
                ],
                36,
            ),
            WHITE_PAINT,
            Transform::IDENTITY,
        ),
        p(
            torus(0.008, 0.07),
            DARK_STEEL,
            xf([0.0, 0.33, 0.0], [0.0; 3]),
        ),
        p(cyl(0.012, 0.06), BRASS, xf([0.0, 0.33, 0.0], [0.0; 3])),
        p(cyl(0.02, 0.012), BRASS, xf([0.0, 0.36, 0.0], [0.0; 3])),
        p(
            torus(0.01, 0.09),
            DARK_STEEL,
            xf([0.0, 0.01, 0.0], [0.0; 3]),
        ),
    ]
}

fn spool(thread: Mat) -> Vec<Part> {
    vec![
        p(cyl(0.032, 0.07), thread, Transform::IDENTITY),
        p(
            cyl(0.05, 0.012),
            WOOD_LIGHT,
            xf([0.0, 0.041, 0.0], [0.0; 3]),
        ),
        p(
            cyl(0.05, 0.012),
            WOOD_LIGHT,
            xf([0.0, -0.041, 0.0], [0.0; 3]),
        ),
    ]
}

fn sewing() -> Vec<Part> {
    let mut out: Vec<Part> = place(spool(THREAD_RED), xf([0.0, 0.047, 0.0], [0.0; 3])).collect();
    out.extend(place(
        spool(THREAD_BLUE),
        xf([0.1, 0.03, 0.04], [90.0, 30.0, 0.0]),
    ));
    out.push(p(
        cyl(0.003, 0.16),
        STEEL,
        xf([0.02, 0.012, 0.09], [90.0, 0.0, 70.0]),
    ));
    out
}

fn spring() -> Vec<Part> {
    let coil = helix(0.045, 6.0, 0.04, 300);
    vec![p(tube(&coil, |_| 0.008, 10), STEEL, Transform::IDENTITY)]
}

fn sheets() -> Vec<Part> {
    (0..3)
        .map(|i| {
            let mat = if i == 1 { STEEL } else { DARK_STEEL };
            p(
                cuboid(0.32, 0.01, 0.22),
                mat,
                xf(
                    [i as f32 * 0.015, i as f32 * 0.016, -(i as f32) * 0.01],
                    [0.0, jit(i, 81) * 10.0, 0.0],
                ),
            )
        })
        .collect()
}

fn metal_pipe() -> Vec<Part> {
    vec![
        p(
            pipe(0.03, 0.04, 0.42),
            DARK_STEEL,
            xf([0.0, 0.21, 0.0], [90.0, 0.0, 0.0]),
        ),
        p(
            pipe(0.036, 0.048, 0.04),
            STEEL,
            xf([0.0, 0.405, 0.0], [90.0, 0.0, 0.0]),
        ),
    ]
}

fn road_signs() -> Vec<Part> {
    vec![
        p(
            cuboid(0.3, 0.14, 0.008),
            YELLOW_PAINT,
            xf([0.05, 0.0, -0.02], [0.0, 0.0, -8.0]),
        ),
        p(
            prism(8, 0.13, 0.008),
            WHITE_PAINT,
            xf([-0.02, 0.05, 0.0], [0.0, 0.0, 22.5]),
        ),
        p(
            prism(8, 0.115, 0.01),
            RED_PAINT,
            xf([-0.02, 0.05, 0.002], [0.0, 0.0, 22.5]),
        ),
    ]
}

fn tech_trash() -> Vec<Part> {
    let mut out = vec![p(cuboid(0.24, 0.012, 0.16), PCB, Transform::IDENTITY)];
    for (x, z, w, d) in [
        (-0.05, -0.02, 0.06, 0.06),
        (0.05, 0.03, 0.04, 0.03),
        (0.07, -0.04, 0.03, 0.03),
    ] {
        out.push(p(cuboid(w, 0.01, d), BLACK, xf([x, 0.011, z], [0.0; 3])));
    }
    for (i, (x, z)) in [(-0.09, 0.05), (-0.07, 0.05), (0.0, -0.05)]
        .iter()
        .enumerate()
    {
        let mat = if i == 2 { BLUE_PAINT } else { STEEL };
        out.push(p(cyl(0.01, 0.03), mat, xf([*x, 0.021, *z], [0.0; 3])));
    }
    for k in 0..8 {
        out.push(p(
            cuboid(0.004, 0.004, 0.02),
            BRASS,
            xf([-0.115, 0.007, -0.06 + k as f32 * 0.016], [0.0, 0.0, 0.0]),
        ));
    }
    out.push(p(
        sphere(0.006),
        LED_GREEN,
        xf([0.1, 0.012, 0.06], [0.0; 3]),
    ));
    out
}

fn fuse() -> Vec<Part> {
    vec![
        p(cyl(0.022, 0.12), GLASS, xf([0.0, 0.08, 0.0], [0.0; 3])),
        p(cyl(0.0025, 0.12), COPPER, xf([0.0, 0.08, 0.0], [0.0; 3])),
        p(cyl(0.025, 0.026), BRASS, xf([0.0, 0.013, 0.0], [0.0; 3])),
        p(cyl(0.025, 0.026), BRASS, xf([0.0, 0.147, 0.0], [0.0; 3])),
    ]
}

/// A rolled blueprint, along X, tied.
fn blueprint() -> Vec<Part> {
    vec![
        p(cyl(0.042, 0.4), PAPER_BLUE, xf([0.0; 3], [0.0, 0.0, 90.0])),
        p(
            pipe(0.012, 0.043, 0.004),
            PAPER_PALE,
            xf([0.201, 0.0, 0.0], [0.0, 90.0, 0.0]),
        ),
        p(
            pipe(0.012, 0.043, 0.004),
            PAPER_PALE,
            xf([-0.201, 0.0, 0.0], [0.0, 90.0, 0.0]),
        ),
        p(
            torus(0.006, 0.044),
            ROPE,
            xf([0.04, 0.0, 0.0], [0.0, 0.0, 90.0]),
        ),
    ]
}

fn padlock() -> Vec<Part> {
    vec![
        p(
            cuboid(0.11, 0.09, 0.04),
            BRASS,
            xf([0.0, 0.045, 0.0], [0.0; 3]),
        ),
        p(
            tube(
                &arc(Vec3::new(0.0, 0.09, 0.0), 0.036, 0.0, PI, 20),
                |_| 0.01,
                10,
            ),
            STEEL,
            Transform::IDENTITY,
        ),
        p(
            cyl(0.01, 0.006),
            HOLE,
            xf([0.0, 0.055, 0.021], [90.0, 0.0, 0.0]),
        ),
        p(
            cuboid(0.006, 0.02, 0.006),
            HOLE,
            xf([0.0, 0.04, 0.021], [0.0; 3]),
        ),
    ]
}

fn code_lock() -> Vec<Part> {
    let mut out = vec![
        p(
            cuboid(0.12, 0.17, 0.04),
            DARK_STEEL,
            xf([0.0, 0.085, 0.0], [0.0; 3]),
        ),
        p(
            cuboid(0.08, 0.026, 0.006),
            GLASS,
            xf([0.0, 0.145, 0.021], [0.0; 3]),
        ),
        p(sphere(0.007), LED_RED, xf([0.042, 0.145, 0.022], [0.0; 3])),
    ];
    for r in 0..4 {
        for c in 0..3 {
            out.push(p(
                cuboid(0.022, 0.018, 0.008),
                GUNMETAL,
                xf(
                    [-0.028 + c as f32 * 0.028, 0.105 - r as f32 * 0.025, 0.022],
                    [0.0; 3],
                ),
            ));
        }
    }
    out
}

fn rounds() -> Vec<Part> {
    let mut out = Vec::new();
    for i in 0..3 {
        let x = (i as f32 - 1.0) * 0.032;
        let at = xf(
            [x, 0.0, (i % 2) as f32 * 0.01],
            [0.0, 0.0, -8.0 + i as f32 * 8.0],
        );
        out.extend(place(
            vec![
                p(cyl(0.012, 0.05), BRASS, xf([0.0, 0.025, 0.0], [0.0; 3])),
                p(cyl(0.0105, 0.016), COPPER, xf([0.0, 0.058, 0.0], [0.0; 3])),
                p(sphere(0.0105), COPPER, xf([0.0, 0.066, 0.0], [0.0; 3])),
                p(cyl(0.013, 0.004), BRASS, xf([0.0, 0.002, 0.0], [0.0; 3])),
            ],
            at,
        ));
    }
    out
}

fn syringe() -> Vec<Part> {
    vec![
        p(cyl(0.016, 0.13), GLASS, xf([0.0, 0.1, 0.0], [0.0; 3])),
        p(
            cyl(0.012, 0.09),
            m(0xc83a3a, 0.3, 0.0, Tex::Plain),
            xf([0.0, 0.08, 0.0], [0.0; 3]),
        ),
        p(
            cuboid(0.06, 0.006, 0.02),
            WHITE_PAINT,
            xf([0.0, 0.168, 0.0], [0.0; 3]),
        ),
        p(cyl(0.005, 0.07), WHITE_PAINT, xf([0.0, 0.2, 0.0], [0.0; 3])),
        p(
            cyl(0.016, 0.006),
            WHITE_PAINT,
            xf([0.0, 0.236, 0.0], [0.0; 3]),
        ),
        p(
            frustum(0.004, 0.009, 0.015),
            WHITE_PAINT,
            xf([0.0, 0.028, 0.0], [0.0; 3]),
        ),
        p(cyl(0.0018, 0.06), STEEL, xf([0.0, -0.01, 0.0], [0.0; 3])),
    ]
}

fn seeds(mat: Mat, r: f32, seed: u32) -> Vec<Part> {
    (0..14)
        .map(|i| {
            let ring = if i == 0 {
                0.0
            } else {
                r * (2.0 + 1.8 * (i / 6) as f32)
            };
            let a = i as f32 * 1.1 + jit(i, seed) * 0.3;
            p(
                blob_n(seed * 7 + i, 0.2, 1),
                mat,
                xfs(
                    [
                        ring * a.cos(),
                        r * (if i == 0 { 1.6 } else { 0.8 }),
                        ring * a.sin(),
                    ],
                    [jit(i, seed + 1) * 40.0, a.to_degrees(), 0.0],
                    [r * 1.3, r * 0.8, r],
                ),
            )
        })
        .collect()
}

/// A tunic laid flat, facing the camera.
fn shirt(mat: Mat) -> Vec<Part> {
    let outline = [
        (0.0, 0.17),
        (0.06, 0.2),
        (0.13, 0.18),
        (0.24, 0.08),
        (0.19, 0.0),
        (0.13, 0.06),
        (0.13, -0.22),
        (0.0, -0.24),
        (-0.13, -0.22),
        (-0.13, 0.06),
        (-0.19, 0.0),
        (-0.24, 0.08),
        (-0.13, 0.18),
        (-0.06, 0.2),
    ]
    .iter()
    .rev()
    .map(|&(x, y)| Vec2::new(x, y))
    .collect::<Vec<_>>();
    vec![
        p(
            slab(&outline, 0.03, 0.035),
            mat,
            xf([0.0; 3], [-20.0, 0.0, 0.0]),
        ),
        p(
            cyl(0.05, 0.01),
            m(0x6a5638, 0.95, 0.0, Tex::Weave),
            xf([0.0, 0.18, 0.02], [70.0, 0.0, 0.0]),
        ),
    ]
}

fn hood() -> Vec<Part> {
    vec![
        p(
            lathe(
                &[
                    (0.13, 0.0),
                    (0.12, 0.06),
                    (0.115, 0.13),
                    (0.1, 0.19),
                    (0.07, 0.235),
                    (0.0, 0.25),
                ],
                32,
            ),
            BURLAP,
            Transform::IDENTITY,
        ),
        p(
            blob(61, 0.1),
            HOLE,
            xfs([0.0, 0.11, 0.095], [0.0; 3], [0.06, 0.07, 0.03]),
        ),
        p(
            cyl(0.01, 0.03),
            m(0x5a4a30, 0.9, 0.0, Tex::Weave),
            xf([0.0, 0.03, 0.1], [90.0, 0.0, 0.0]),
        ),
    ]
}

fn poncho() -> Vec<Part> {
    let mut outline = vec![
        Vec2::new(0.0, 0.16),
        Vec2::new(0.16, 0.12),
        Vec2::new(0.26, 0.0),
    ];
    for k in 0..=8 {
        let x = 0.24 - k as f32 * 0.06;
        let y = -0.2 - if k % 2 == 0 { 0.0 } else { 0.04 };
        outline.push(Vec2::new(x, y));
    }
    outline.extend([Vec2::new(-0.26, 0.0), Vec2::new(-0.16, 0.12)]);
    outline.reverse();
    vec![
        p(
            slab(&outline, 0.025, 0.03),
            HIDE,
            xf([0.0; 3], [-20.0, 0.0, 0.0]),
        ),
        p(
            cyl(0.045, 0.01),
            HOLE,
            xf([0.0, 0.12, 0.03], [70.0, 0.0, 0.0]),
        ),
    ]
}

fn bone_helmet() -> Vec<Part> {
    let horn = |side: f32| -> Part {
        let path: Vec<Vec3> = (0..=16)
            .map(|i| {
                let t = i as f32 / 16.0;
                Vec3::new(side * (0.1 + 0.12 * t), 0.14 + 0.16 * t * t, -0.02 * t)
            })
            .collect();
        p(
            tube(&path, |t| 0.026 * (1.0 - t) + 0.003, 10),
            BONE,
            Transform::IDENTITY,
        )
    };
    vec![
        p(
            lathe(
                &[
                    (0.13, 0.0),
                    (0.128, 0.05),
                    (0.12, 0.1),
                    (0.1, 0.15),
                    (0.065, 0.185),
                    (0.0, 0.2),
                ],
                32,
            ),
            BONE,
            Transform::IDENTITY,
        ),
        p(torus(0.012, 0.13), LEATHER, xf([0.0, 0.01, 0.0], [0.0; 3])),
        horn(1.0),
        horn(-1.0),
    ]
}

fn roadsign_vest() -> Vec<Part> {
    let outline = [
        (0.0, 0.1),
        (0.07, 0.2),
        (0.14, 0.2),
        (0.16, 0.06),
        (0.15, -0.22),
        (0.0, -0.25),
        (-0.15, -0.22),
        (-0.16, 0.06),
        (-0.14, 0.2),
        (-0.07, 0.2),
    ]
    .iter()
    .rev()
    .map(|&(x, y)| Vec2::new(x, y))
    .collect::<Vec<_>>();
    vec![
        p(
            slab(&outline, 0.03, 0.03),
            YELLOW_PAINT,
            xf([0.0; 3], [-20.0, 0.0, 0.0]),
        ),
        p(
            cuboid(0.3, 0.035, 0.012),
            RED_PAINT,
            xf([0.0, -0.08, 0.06], [-20.0, 0.0, 0.0]),
        ),
        p(
            cuboid(0.03, 0.06, 0.012),
            LEATHER,
            xf([0.1, 0.17, 0.02], [-20.0, 0.0, 0.0]),
        ),
        p(
            cuboid(0.03, 0.06, 0.012),
            LEATHER,
            xf([-0.1, 0.17, 0.02], [-20.0, 0.0, 0.0]),
        ),
    ]
}

fn planter() -> Vec<Part> {
    let mut out = vec![
        p(cuboid(0.4, 0.12, 0.2), WOOD, xf([0.0, 0.06, 0.0], [0.0; 3])),
        p(
            cuboid(0.37, 0.01, 0.17),
            SOIL,
            xf([0.0, 0.121, 0.0], [0.0; 3]),
        ),
        p(
            cuboid(0.41, 0.02, 0.21),
            WOOD_DARK,
            xf([0.0, 0.0, 0.0], [0.0; 3]),
        ),
    ];
    for (i, x) in [-0.12f32, 0.0, 0.12].iter().enumerate() {
        out.push(p(cyl(0.004, 0.06), LEAF, xf([*x, 0.15, 0.0], [0.0; 3])));
        for side in [-1.0, 1.0] {
            let leaf = [
                Vec2::new(0.0, 0.0),
                Vec2::new(0.02, 0.02),
                Vec2::new(0.05, 0.03),
                Vec2::new(0.02, 0.045),
                Vec2::new(0.0, 0.01),
            ];
            out.push(p(
                slab(&leaf, 0.003, 0.003),
                LEAF,
                xf(
                    [*x, 0.17 + 0.01 * i as f32, 0.0],
                    [0.0, if side > 0.0 { 0.0 } else { 180.0 }, 15.0],
                ),
            ));
        }
    }
    out
}

/// A garage door: a steel panel ribbed across, a handle at its foot.
fn garage_door() -> Vec<Part> {
    let mut out = vec![p(
        cuboid(0.05, 1.0, 1.0),
        WHITE_PAINT,
        xf([0.0, 0.5, 0.0], [0.0; 3]),
    )];
    for k in 0..6 {
        out.push(p(
            cuboid(0.07, 0.03, 0.98),
            m(0xb8bcc0, 0.45, 0.3, Tex::Brushed),
            xf([0.0, 0.1 + k as f32 * 0.16, 0.0], [0.0; 3]),
        ));
    }
    out.push(p(
        cuboid(0.08, 0.04, 0.2),
        DARK_STEEL,
        xf([0.0, 0.08, 0.0], [0.0; 3]),
    ));
    out
}

/// A window: a wooden frame, four panes, the glass a cold blue.
fn glass_window() -> Vec<Part> {
    let mut out = vec![p(
        cuboid(0.02, 0.9, 0.9),
        GLASS,
        xf([0.0, 0.5, 0.0], [0.0; 3]),
    )];
    for (y, h, z, w) in [
        (0.06, 0.08, 0.0, 1.0),
        (0.94, 0.08, 0.0, 1.0),
        (0.5, 0.05, 0.0, 0.9),
        (0.5, 0.96, -0.47, 0.08),
        (0.5, 0.96, 0.47, 0.08),
        (0.5, 0.9, 0.0, 0.05),
    ] {
        out.push(p(cuboid(0.06, h, w), WOOD, xf([0.0, y, z], [0.0; 3])));
    }
    out
}

/// A camp fire, cold: a ring of stones round crossed logs, embers in the ash.
fn fire_pit() -> Vec<Part> {
    let mut out = Vec::new();
    for k in 0..9u32 {
        let a = k as f32 / 9.0 * TAU;
        let s = 0.06 + 0.012 * jit(k, 101);
        out.push(p(
            lump(110 + k, 0.45),
            STONE,
            xfs(
                [0.2 * a.cos(), 0.035, 0.2 * a.sin()],
                [0.0, a.to_degrees(), 0.0],
                [s, s * 0.7, s * 0.9],
            ),
        ));
    }
    for (k, a) in [20.0f32, 80.0, 140.0].iter().enumerate() {
        out.push(p(
            cyl(0.03, 0.3),
            BARK,
            xf([0.0, 0.06 + k as f32 * 0.025, 0.0], [0.0, *a, 75.0]),
        ));
    }
    out.push(p(cyl(0.13, 0.02), CHARCOAL, xf([0.0, 0.01, 0.0], [0.0; 3])));
    out.extend(heap(EMBER, 4, 0.02, 120).into_iter().map(|mut part| {
        part.at.translation.y += 0.01;
        part
    }));
    out
}

fn turret() -> Vec<Part> {
    vec![
        p(cyl(0.12, 0.04), DARK_STEEL, xf([0.0, 0.02, 0.0], [0.0; 3])),
        p(cyl(0.03, 0.12), GUNMETAL, xf([0.0, 0.1, 0.0], [0.0; 3])),
        p(
            cuboid(0.16, 0.12, 0.2),
            GUNMETAL,
            xf([0.0, 0.21, 0.0], [0.0; 3]),
        ),
        p(
            cyl(0.014, 0.2),
            DARK_STEEL,
            xf([0.0, 0.23, 0.19], [90.0, 0.0, 0.0]),
        ),
        p(
            cuboid(0.06, 0.08, 0.08),
            m(0x55603a, 0.6, 0.2, Tex::Speckle),
            xf([0.11, 0.18, 0.0], [0.0; 3]),
        ),
        p(sphere(0.018), LED_RED, xf([-0.04, 0.25, 0.1], [0.0; 3])),
    ]
}

fn insert(arch: u8, mat: Mat) -> Vec<Part> {
    vec![p(
        super::structures::insert_mesh(arch),
        mat,
        Transform::IDENTITY,
    )]
}

// ---- Rust's tree items (2026-10-17) -----------------------------------------
//
// Their salvaged weapons are scrap made sharp: pipe hafts, rag-wrapped grips,
// blades ground out of saw steel and road signs. Each stands on Y with its
// grip on the axis, as every held prop does.

/// A scrap haft: a pipe from the foot to `len`, a rag wound round its grip.
fn scrap_haft(len: f32, wrap: f32) -> Vec<Part> {
    vec![
        p(
            pipe(0.011, 0.016, len),
            DARK_STEEL,
            xf([0.0, len * 0.5, 0.0], [90.0, 0.0, 0.0]),
        ),
        p(
            cyl(0.02, wrap),
            CHARRED_CLOTH,
            xf([0.0, 0.02 + wrap * 0.5, 0.0], [0.0; 3]),
        ),
    ]
}

/// A lump of scrap welded across a pipe, a bolt through it.
fn salvaged_hammer() -> Vec<Part> {
    let mut out = scrap_haft(0.44, 0.14);
    out.extend([
        p(
            cuboid(0.17, 0.08, 0.08),
            GUNMETAL,
            xf([0.02, 0.47, 0.0], [0.0; 3]),
        ),
        p(
            cuboid(0.05, 0.09, 0.09),
            RED_PAINT,
            xf([-0.075, 0.465, 0.0], [0.0; 3]),
        ),
        p(
            cyl(0.012, 0.1),
            STEEL,
            xf([0.0, 0.47, 0.0], [90.0, 0.0, 0.0]),
        ),
    ]);
    out
}

/// A straight blade on a wrapped grip: `len` tip to pommel, the blade
/// `half` wide each side, the guard `guard` across.
fn sword(len: f32, half: f32, guard: f32, grip: Mat) -> Vec<Part> {
    let base = len * 0.22;
    let outline = [
        Vec2::new(0.0, len),
        Vec2::new(-half, len - half * 3.0),
        Vec2::new(-half * 1.1, base + 0.03),
        Vec2::new(-half * 0.8, base),
        Vec2::new(half * 0.8, base),
        Vec2::new(half * 1.1, base + 0.03),
        Vec2::new(half, len - half * 3.0),
    ];
    vec![
        p(slab(&outline, 0.008, 0.004), STEEL, Transform::IDENTITY),
        p(
            cuboid(guard, 0.022, 0.034),
            DARK_STEEL,
            xf([0.0, base - 0.01, 0.0], [0.0; 3]),
        ),
        p(
            cyl(0.017, base - 0.04),
            grip,
            xf([0.0, 0.02 + (base - 0.04) * 0.5, 0.0], [0.0; 3]),
        ),
        p(
            cyl(0.024, 0.022),
            DARK_STEEL,
            xf([0.0, 0.011, 0.0], [0.0; 3]),
        ),
    ]
}

/// A spiked iron ball on a wooden haft.
fn mace() -> Vec<Part> {
    let mut out = vec![
        p(cyl(0.018, 0.58), WOOD_DARK, xf([0.0, 0.29, 0.0], [0.0; 3])),
        p(cyl(0.021, 0.13), LEATHER, xf([0.0, 0.085, 0.0], [0.0; 3])),
        p(sphere(0.06), GUNMETAL, xf([0.0, 0.6, 0.0], [0.0; 3])),
        p(cone(0.014, 0.04), STEEL, xf([0.0, 0.678, 0.0], [0.0; 3])),
    ];
    for k in 0..8 {
        let a = k as f32 / 8.0 * TAU;
        let dir = Vec3::new(a.cos(), 0.0, a.sin());
        out.push(p(
            cone(0.013, 0.045),
            STEEL,
            Transform::from_translation(Vec3::new(0.0, 0.6, 0.0) + dir * 0.07)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, dir)),
        ));
    }
    out
}

/// A road sign cut into a broad blade, riveted to a wrapped pipe.
fn cleaver() -> Vec<Part> {
    let mut out = scrap_haft(0.32, 0.18);
    out.extend([
        p(
            cuboid(0.15, 0.36, 0.006),
            YELLOW_PAINT,
            xf([0.06, 0.48, 0.0], [0.0; 3]),
        ),
        p(
            cuboid(0.151, 0.05, 0.007),
            RED_PAINT,
            xf([0.06, 0.6, 0.0], [0.0; 3]),
        ),
        p(
            cuboid(0.012, 0.36, 0.004),
            STEEL,
            xf([0.14, 0.48, 0.0], [0.0; 3]),
        ),
    ]);
    for y in [0.32, 0.4] {
        out.push(p(
            cyl(0.008, 0.02),
            DARK_STEEL,
            xf([0.0, y, 0.0], [90.0, 0.0, 0.0]),
        ));
    }
    out
}

/// A saw blade bent into an axe head, clamped to a pipe.
fn salvaged_axe() -> Vec<Part> {
    let mut out = scrap_haft(0.58, 0.14);
    let head = [
        Vec2::new(0.02, 0.44),
        Vec2::new(0.1, 0.42),
        Vec2::new(0.19, 0.4),
        Vec2::new(0.22, 0.5),
        Vec2::new(0.2, 0.62),
        Vec2::new(0.1, 0.58),
        Vec2::new(0.02, 0.56),
    ];
    out.extend([
        p(slab(&head, 0.008, 0.003), STEEL, Transform::IDENTITY),
        p(
            cuboid(0.05, 0.13, 0.032),
            GUNMETAL,
            xf([0.0, 0.5, 0.0], [0.0; 3]),
        ),
    ]);
    out
}

/// Two blades bent into a pick, clamped across a pipe.
fn salvaged_icepick() -> Vec<Part> {
    let mut out = scrap_haft(0.56, 0.14);
    let path: Vec<Vec3> = (0..=16)
        .map(|i| {
            let t = i as f32 / 16.0;
            let x = (t - 0.5) * 0.36;
            Vec3::new(x, 0.56 - x * x * 1.4, 0.0)
        })
        .collect();
    out.extend([
        p(
            tube(&path, |t| 0.004 + 0.016 * (1.0 - (2.0 * t - 1.0).abs()), 10),
            DARK_STEEL,
            Transform::IDENTITY,
        ),
        p(
            cuboid(0.05, 0.08, 0.034),
            GUNMETAL,
            xf([0.0, 0.54, 0.0], [0.0; 3]),
        ),
    ]);
    out
}

/// A big coffee can worn upside down, an eye slot cut in its front.
fn coffee_can() -> Vec<Part> {
    vec![
        p(
            lathe(
                &[(0.115, 0.0), (0.115, 0.22), (0.11, 0.225), (0.0, 0.225)],
                32,
            ),
            STEEL,
            Transform::IDENTITY,
        ),
        p(cyl(0.117, 0.07), RED_PAINT, xf([0.0, 0.16, 0.0], [0.0; 3])),
        p(
            cuboid(0.13, 0.024, 0.03),
            HOLE,
            xf([0.0, 0.1, 0.104], [0.0; 3]),
        ),
        p(
            torus(0.006, 0.115),
            DARK_STEEL,
            xf([0.0, 0.004, 0.0], [0.0; 3]),
        ),
    ]
}

/// A tank top, laid back for the icon as a shirt is.
fn tank_top() -> Vec<Part> {
    let outline = [
        (0.0, 0.12),
        (0.05, 0.2),
        (0.09, 0.2),
        (0.1, 0.12),
        (0.14, 0.04),
        (0.13, -0.22),
        (0.0, -0.24),
        (-0.13, -0.22),
        (-0.14, 0.04),
        (-0.1, 0.12),
        (-0.09, 0.2),
        (-0.05, 0.2),
    ]
    .iter()
    .rev()
    .map(|&(x, y)| Vec2::new(x, y))
    .collect::<Vec<_>>();
    vec![p(
        slab(&outline, 0.03, 0.035),
        m(0xd6d0c2, 0.95, 0.0, Tex::Weave),
        xf([0.0; 3], [-20.0, 0.0, 0.0]),
    )]
}

/// A bean can full of gunpowder, taped round, a fuse out of its lid.
fn beancan() -> Vec<Part> {
    vec![
        p(
            lathe(
                &[(0.036, 0.0), (0.036, 0.11), (0.032, 0.115), (0.0, 0.115)],
                28,
            ),
            STEEL,
            Transform::IDENTITY,
        ),
        p(
            cyl(0.0365, 0.06),
            m(0xc8642a, 0.6, 0.0, Tex::Speckle),
            xf([0.0, 0.055, 0.0], [0.0; 3]),
        ),
        p(cyl(0.037, 0.014), BLACK, xf([0.0, 0.095, 0.0], [0.0; 3])),
        p(cyl(0.004, 0.05), ROPE, xf([0.01, 0.135, 0.0], [0.0; 3])),
    ]
}

// ---- the bake ----------------------------------------------------------------

/// The detail atlas: one [`CELL`]² cell per [`Tex`], in `Tex::ALL` order,
/// [`ATLAS_COLS`] across. Built in code at startup ([`install`]) and so
/// addressed by a fixed handle, which lets [`material`] stay a pure function
/// the tests can call.
pub const DETAIL: Handle<Image> = uuid_handle!("6f0b3f4e-9a3c-4d2b-8e51-1c7a2b9d4e60");
/// The surface table: roughness across, metalness down, [`LEVELS`] each, in
/// the green and blue channels glTF reads them from. A part's UV1 is the
/// centre of its texel.
pub const SURFACE: Handle<Image> = uuid_handle!("a2d94c71-5e08-4b6f-9c3a-7e1f0d8b2c45");

/// A detail cell's edge, pixels. A held item is a hand's width; 128 texels
/// across one part is finer than the screen shows it.
const CELL: u32 = 128;
const ATLAS_COLS: u32 = 4;
const ATLAS_ROWS: u32 = 2;
/// How far inside its cell a part's UVs stay, so the filter and the first
/// mips read its own pattern and not a neighbour's.
const CELL_MARGIN: f32 = 4.0 / CELL as f32;
/// Steps in the surface table, per axis.
const LEVELS: u32 = 16;
/// Metalness as stored, against the kit's 0..1: the hand has the island's
/// hemisphere fill to reflect and little else, and a fully metallic part
/// with nothing to mirror draws near black — the revolver's row settled on
/// 0.55 for the same reason.
pub const METAL_GAIN: f32 = 0.55;

/// How far each occlusion probe steps out along the normal, as a fraction of
/// the prop's reach, and how much it counts (`bc-model/src/ao.rs`'s steps,
/// scaled from an 18 m suit to a thing in a hand).
const AO_STEPS: [(f32, f32); 5] = [
    (0.06, 1.0),
    (0.16, 0.75),
    (0.32, 0.55),
    (0.55, 0.4),
    (0.8, 0.3),
];
/// How much a fully occluded vertex darkens. Less than `bc-model`'s 1.1,
/// because there the shader puts it on the ambient term alone and here it
/// multiplies the colour, so the key light sees it too.
const AO_STRENGTH: f32 = 0.6;
/// How round a part's volume is, as a fraction of its thinnest half-extent:
/// a log's box with its long edges rounded off, so the gap between two logs
/// is not filled in by their corners.
const VOL_ROUND: f32 = 0.6;

/// Put the kit's two textures in the store, under [`DETAIL`] and
/// [`SURFACE`]. Once, at startup, before any held model draws.
pub fn install(images: &mut Assets<Image>) {
    let _ = images.insert(DETAIL.id(), detail_atlas());
    let _ = images.insert(SURFACE.id(), surface_table());
}

/// The one material every baked prop draws with. The colour is in the
/// vertices, so the base colour is white and both factors are 1: the
/// textures say everything. Nothing glows (`tests/held_assets.rs`), not even
/// an LED — a carried emissive is a lamp in the dark.
pub fn material() -> StandardMaterial {
    StandardMaterial {
        base_color_texture: Some(DETAIL),
        metallic_roughness_texture: Some(SURFACE),
        metallic_roughness_channel: bevy::pbr::UvChannel::Uv1,
        perceptual_roughness: 1.0,
        metallic: 1.0,
        reflectance: super::fresnel::DIELECTRIC,
        ..default()
    }
}

fn detail_atlas() -> Image {
    let (w, h) = (CELL * ATLAS_COLS, CELL * ATLAS_ROWS);
    // The spare cells a mid grey, so nothing dark bleeds into a used one.
    let mut px = [217u8, 217, 217, 255].repeat((w * h) as usize);
    for (k, &tex) in Tex::ALL.iter().enumerate() {
        let (cx, cy) = (k as u32 % ATLAS_COLS, k as u32 / ATLAS_COLS);
        for j in 0..CELL {
            for i in 0..CELL {
                let (u, v) = (
                    (i as f32 + 0.5) / CELL as f32,
                    (j as f32 + 0.5) / CELL as f32,
                );
                let b = (detail(tex, u, v) * 255.0).round() as u8;
                let o = (((cy * CELL + j) * w + cx * CELL + i) * 4) as usize;
                px[o..o + 4].copy_from_slice(&[b, b, b, 255]);
            }
        }
    }
    let chain = super::mipmap::chain(&px, w, h, super::mipmap::Filter::Srgb);
    let mut img = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.texture_descriptor.mip_level_count = super::mipmap::levels(w, h);
    img.data = Some(chain);
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    img
}

fn surface_table() -> Image {
    let level = |k: u32| (k * 255 / (LEVELS - 1)) as u8;
    let mut px = Vec::with_capacity((LEVELS * LEVELS * 4) as usize);
    for metal in 0..LEVELS {
        for rough in 0..LEVELS {
            px.extend([0, level(rough), level(metal), 255]);
        }
    }
    let mut img = Image::new(
        Extent3d {
            width: LEVELS,
            height: LEVELS,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    // One texel per part, read exactly: a filtered tap between two would
    // hand a wooden grip half the barrel's metal.
    img.sampler = ImageSampler::nearest();
    img
}

/// A material's texel in the surface table.
fn surface_uv(mat: &Mat) -> Vec2 {
    let q = |x: f32| (x.clamp(0.0, 1.0) * (LEVELS - 1) as f32).round() + 0.5;
    Vec2::new(q(mat.rough), q(mat.metal * METAL_GAIN)) / LEVELS as f32
}

/// A part's volume for the occlusion bake: its own box, placed, its long
/// edges rounded by [`VOL_ROUND`].
struct Vol {
    centre: Vec3,
    axes: [Vec3; 3],
    half: Vec3,
}

impl Vol {
    /// Signed distance to the surface, negative inside.
    fn distance(&self, p: Vec3) -> f32 {
        let d = p - self.centre;
        let q = Vec3::new(
            self.axes[0].dot(d).abs(),
            self.axes[1].dot(d).abs(),
            self.axes[2].dot(d).abs(),
        );
        let r = VOL_ROUND * self.half.min_element();
        let q = q - (self.half - Vec3::splat(r));
        q.max(Vec3::ZERO).length() + q.max_element().min(0.0) - r
    }
}

fn bounds(ps: impl Iterator<Item = Vec3>) -> (Vec3, Vec3) {
    ps.fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)))
}

/// The prop `name` as the one mesh a held row draws, or `None` for a name
/// with no prop. Stood on its own feet with its authored origin on the Y
/// axis — the held models' convention, so the grip fraction means what it
/// does for a `.glb`.
pub fn baked(name: &str) -> Option<Mesh> {
    parts(name).map(|p| bake(&p))
}

fn bake(parts: &[Part]) -> Mesh {
    use bevy::mesh::VertexAttributeValues as V;
    let (mut pos, mut nrm, mut uv0, mut uv1, mut col, mut idx, mut owner) = (
        Vec::<Vec3>::new(),
        Vec::<Vec3>::new(),
        Vec::<[f32; 2]>::new(),
        Vec::<[f32; 2]>::new(),
        Vec::<Vec3>::new(),
        Vec::<u32>::new(),
        Vec::<usize>::new(),
    );
    let mut vols = Vec::with_capacity(parts.len());
    for (k, part) in parts.iter().enumerate() {
        let m = &part.mesh;
        let Some(V::Float32x3(ps)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else {
            continue;
        };
        let ns = match m.attribute(Mesh::ATTRIBUTE_NORMAL) {
            Some(V::Float32x3(n)) => Some(n),
            _ => None,
        };
        let uvs = match m.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(V::Float32x2(u)) => Some(u),
            _ => None,
        };
        let cs = match m.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(V::Float32x4(c)) => Some(c),
            _ => None,
        };
        let at = part.at;
        let (lo, hi) = bounds(ps.iter().map(|p| Vec3::from(*p)));
        vols.push(Vol {
            centre: at.transform_point((lo + hi) * 0.5),
            axes: [
                at.rotation * Vec3::X,
                at.rotation * Vec3::Y,
                at.rotation * Vec3::Z,
            ],
            half: ((hi - lo) * 0.5 * at.scale).abs(),
        });
        // The part's UVs, whatever range its primitive wrote, onto its
        // material's cell.
        let (ulo, uhi) = uvs.map_or((Vec2::ZERO, Vec2::ONE), |u| {
            u.iter().fold((Vec2::MAX, Vec2::MIN), |(lo, hi), t| {
                (lo.min(Vec2::from(*t)), hi.max(Vec2::from(*t)))
            })
        });
        let cell = Tex::ALL
            .iter()
            .position(|t| *t == part.mat.tex)
            .unwrap_or(0) as u32;
        let size = Vec2::new(ATLAS_COLS as f32, ATLAS_ROWS as f32).recip();
        let c0 = Vec2::new((cell % ATLAS_COLS) as f32, (cell / ATLAS_COLS) as f32) * size
            + size * CELL_MARGIN;
        let span = size * (1.0 - 2.0 * CELL_MARGIN);
        let surf = surface_uv(&part.mat).to_array();
        let lin = part.mat.color().to_linear();
        let paint = Vec3::new(lin.red, lin.green, lin.blue);
        let inv_scale = at.scale.recip();
        let base = pos.len() as u32;
        for (i, p) in ps.iter().enumerate() {
            pos.push(at.transform_point(Vec3::from(*p)));
            let n = ns.map_or(Vec3::Y, |n| Vec3::from(n[i]));
            nrm.push((at.rotation * (n * inv_scale)).normalize_or(Vec3::Y));
            let t = uvs.map_or(Vec2::splat(0.5), |u| {
                (Vec2::from(u[i]) - ulo) / (uhi - ulo).max(Vec2::splat(1e-6))
            });
            uv0.push((c0 + span * t.clamp(Vec2::ZERO, Vec2::ONE)).to_array());
            uv1.push(surf);
            let shade = cs.map_or(1.0, |c| (c[i][0] + c[i][1] + c[i][2]) / 3.0);
            col.push(paint * shade);
            owner.push(k);
        }
        match m.indices() {
            Some(ix) => idx.extend(ix.iter().map(|i| base + i as u32)),
            None => idx.extend(base..base + ps.len() as u32),
        }
    }

    // On its own feet. Across, it stays where it was authored: the origin is
    // where the hand goes (a coil's middle, the big gear's hub, an arrow's
    // shaft), which a bounding box's centre is not — `import_meshy.py`
    // centring the box is how the hatchet once hung beside the fist.
    let (lo, hi) = bounds(pos.iter().copied());
    let shift = Vec3::new(0.0, -lo.y, 0.0);
    for p in &mut pos {
        *p += shift;
    }
    for v in &mut vols {
        v.centre += shift;
    }

    // The occlusion: each vertex looks out along its normal and adds up how
    // deep its steps land inside the OTHER parts' volumes. A part never
    // shades itself — a coil's own box would fill its middle in.
    let reach = ((hi - lo).length() * 0.12).clamp(0.012, 0.06);
    let total: f32 = AO_STEPS.iter().map(|s| s.1).sum();
    let near: Vec<(f32, &Vol)> = vols.iter().map(|v| (v.half.length() + reach, v)).collect();
    let mut close: Vec<&Vol> = Vec::new();
    for i in 0..pos.len() {
        let (p, n) = (pos[i], nrm[i]);
        close.clear();
        close.extend(
            near.iter()
                .enumerate()
                .filter(|(k, (r, v))| *k != owner[i] && v.centre.distance(p) < *r)
                .map(|(_, (_, v))| *v),
        );
        if close.is_empty() {
            continue;
        }
        let mut occ = 0.0;
        for (step, weight) in AO_STEPS {
            let h = step * reach;
            let q = p + n * h;
            let d = close.iter().map(|v| v.distance(q)).fold(f32::MAX, f32::min);
            occ += (h - d).clamp(0.0, h) / h * weight;
        }
        col[i] *= 1.0 - AO_STRENGTH * (occ / total).min(1.0);
    }

    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    m.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        pos.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
    );
    m.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        nrm.iter().map(|n| n.to_array()).collect::<Vec<_>>(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv0);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
    m.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        col.iter().map(|c| [c.x, c.y, c.z, 1.0]).collect::<Vec<_>>(),
    );
    m.insert_indices(Indices::U32(idx));
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name the kit lists bakes, and into what a held row draws: one
    /// mesh carrying its paint (colour, both UV sets) on its own feet, with
    /// the occlusion actually reaching some vertex.
    #[test]
    fn every_prop_bakes_to_one_painted_mesh() {
        use bevy::mesh::VertexAttributeValues as V;
        for name in NAMES {
            let m = baked(name).unwrap_or_else(|| panic!("{name} is listed and has no parts"));
            for attr in [
                Mesh::ATTRIBUTE_NORMAL,
                Mesh::ATTRIBUTE_UV_0,
                Mesh::ATTRIBUTE_UV_1,
                Mesh::ATTRIBUTE_COLOR,
            ] {
                assert!(m.attribute(attr).is_some(), "{name}: no {attr:?}");
            }
            let Some(V::Float32x3(ps)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else {
                panic!("{name}: no positions")
            };
            let low = ps.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
            assert!(low.abs() < 1e-5, "{name} stands at {low}, not on 0");
        }
        // A cold fire pit's logs shade each other and the ash where they cross.
        let paint = BARK.color().to_linear();
        let pit = baked("camp_fire").expect("camp_fire");
        let Some(V::Float32x4(cs)) = pit.attribute(Mesh::ATTRIBUTE_COLOR) else {
            panic!("no colours")
        };
        let darkest = cs.iter().map(|c| c[0] / paint.red).fold(f32::MAX, f32::min);
        assert!(
            darkest < 0.8,
            "no crevice in the fire pit is shaded ({darkest})"
        );
    }

    /// A tube's normals point out of it and agree with its winding: a coil
    /// lit from inside renders black, and that is how this was found.
    #[test]
    fn a_coil_faces_out() {
        let path = helix(0.11, 3.6, 0.03, TUBE_MAX - 1);
        let m = tube(&path, |_| 0.017, 10);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(ps)) =
            m.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!()
        };
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(ns)) =
            m.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            panic!()
        };
        let (mut bad, mut nan) = (0, 0);
        let ring = path.len() * 11;
        for i in 0..ring {
            let p = Vec3::from_array(ps[i]);
            let n = Vec3::from_array(ns[i]);
            if n.is_nan() {
                nan += 1;
                continue;
            }
            let c = path[i / 11];
            if n.dot(p - c) < 0.0 {
                bad += 1;
            }
        }
        let Some(Indices::U32(ix)) = m.indices() else {
            panic!()
        };
        let mut flipped = 0;
        for t in ix.chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from_array(ps[i as usize]));
            let f = (b - a).cross(c - a);
            let n = Vec3::from_array(ns[t[0] as usize]);
            if f.dot(n) < 0.0 {
                flipped += 1;
            }
        }
        assert_eq!(
            (bad, nan, flipped),
            (0, 0, 0),
            "of {ring} / {}",
            ix.len() / 3
        );
    }
}
