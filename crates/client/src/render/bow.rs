//! The bow in your own hand: its string, and the arrow on it.
//!
//! The hunting bow is one rigid mesh with its string baked in, so a draw could
//! only move the whole bow. The first-person copy has the string taken out at
//! load ([`strip_string`]) and drawn here instead: two segments from where the
//! string is tied on to a nock that follows the draw, and an arrow on the string
//! while the bow is raised with arrows to loose. Other players' bows keep the
//! baked string, since nothing on the wire says they are drawing.
//!
//! Everything here hangs under `viewmodel::HeldModel`, so it is in the bow
//! model's own frame (metres before the row's in-hand scale) and inherits the
//! hold, the raise and every sway for free. The draw itself is `viewmodel`'s
//! (`Motion::bow`); this only draws it.

use std::collections::HashMap;

use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;

use super::viewmodel::{HeldModel, Motion};

/// The `HELD_MODELS` key of the one row that has a string.
pub const BOW_KEY: &str = "hunting_bow";

/// A connected piece of the model is the string when it runs further than
/// this up the bow and is thinner than [`STRAND_THIN_M`] through it, model
/// metres. The shipped file's string is two such pieces (1.32 m and 0.24 m
/// long, 7 and 8 mm through); nothing else in it is under 38 mm through.
const STRAND_LONG_M: f32 = 0.2;
const STRAND_THIN_M: f32 = 0.012;
/// Vertices closer than this are one vertex when finding what is connected:
/// a glTF splits a vertex at every UV seam.
const WELD_M: f32 = 1e-5;

/// How far up the bow the arrow crosses the string, model metres: on the
/// fist, whose centre is the row's grip (0.5 of 1.687 m) and whose top is
/// about 6 cm above it at the row's 0.8 scale.
pub const NOCK_Y_M: f32 = 0.93;
/// How far a full draw pulls the nock back off the braced string, model
/// metres. Brace to full draw is 0.2 m to 0.7 m from the riser, a real draw
/// length; drawn in hand that puts the nock at the cheek, just past the
/// near plane, so the string at full draw runs out of the frame.
pub const DRAW_M: f32 = 0.5;
/// The string's radius, model metres.
pub const STRING_R_M: f32 = 0.0022;
/// The arrow on the string: its length, and how far to the side of the string
/// it passes the riser (the riser's half-thickness plus the shaft's radius),
/// model metres. Negative is the side that faces your eye once the bow is
/// raised (`viewmodel::bow_aim`).
pub const ARROW_LEN_M: f32 = 0.8;
pub const ARROW_SIDE_M: f32 = -0.027;
/// The loose: how far the string shivers past brace, model metres, and how
/// fast, per second. It dies out over `viewmodel::VIEWMODEL_LOOSE_S`.
pub const SHIVER_M: f32 = 0.03;
pub const SHIVER_HZ: f32 = 17.0;

/// The first-person bow without its string, and where the string was tied on.
#[derive(Resource, Default)]
pub struct FpBow {
    /// `None` until the file has loaded, and for good if it had no string to
    /// take out (the bow then keeps the baked one and no live one is drawn).
    pub mesh: Option<Handle<Mesh>>,
    /// The string's two ends in model space, bottom then top.
    pub ends: [Vec3; 2],
}

/// One half of the string: from the bottom tie to the nock, or from the nock
/// to the top tie.
#[derive(Component)]
pub struct BowString {
    pub upper: bool,
}

/// The arrow on the string.
#[derive(Component)]
pub struct NockedArrow;

/// The `HELD_MODELS` row this module draws a string on.
pub fn bow_row() -> Option<usize> {
    crate::ui::hold::HELD_MODELS
        .iter()
        .position(|m| m.key == BOW_KEY)
}

/// Take the string out of a bow's triangles.
///
/// Returns the index buffer without it, and the string's lowest and highest
/// points (where it is tied on), or `None` if nothing in the mesh is shaped
/// like a string. The string is found by shape rather than by index range, so
/// a re-exported file still splits: every connected piece that is long up the
/// bow and thin through it goes.
pub fn strip_string(pos: &[[f32; 3]], idx: &[u32]) -> Option<(Vec<u32>, Vec3, Vec3)> {
    // Weld, then union the three corners of every triangle.
    let mut weld: HashMap<[i64; 3], usize> = HashMap::new();
    let id: Vec<usize> = pos
        .iter()
        .map(|p| {
            let k = p.map(|c| (c / WELD_M).round() as i64);
            let n = weld.len();
            *weld.entry(k).or_insert(n)
        })
        .collect();
    let mut parent: Vec<usize> = (0..weld.len()).collect();
    fn root(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for t in idx.chunks_exact(3) {
        let a = root(&mut parent, id[t[0] as usize]);
        for &v in &t[1..] {
            let b = root(&mut parent, id[v as usize]);
            parent[b] = a;
        }
    }
    // Each piece's bounds.
    let mut bounds: HashMap<usize, (Vec3, Vec3)> = HashMap::new();
    for &v in idx {
        let p = Vec3::from(pos[v as usize]);
        let r = root(&mut parent, id[v as usize]);
        let b = bounds.entry(r).or_insert((p, p));
        b.0 = b.0.min(p);
        b.1 = b.1.max(p);
    }
    let strand = |r: usize| {
        let (lo, hi) = bounds[&r];
        hi.y - lo.y > STRAND_LONG_M && hi.z - lo.z < STRAND_THIN_M
    };
    let mut keep = Vec::with_capacity(idx.len());
    let mut ends: Option<(Vec3, Vec3)> = None;
    for t in idx.chunks_exact(3) {
        if strand(root(&mut parent, id[t[0] as usize])) {
            for &v in t {
                let p = Vec3::from(pos[v as usize]);
                let e = ends.get_or_insert((p, p));
                if p.y < e.0.y {
                    e.0 = p;
                }
                if p.y > e.1.y {
                    e.1 = p;
                }
            }
        } else {
            keep.extend_from_slice(t);
        }
    }
    ends.map(|(lo, hi)| (keep, lo, hi))
}

/// Build the stringless first-person bow once its file has loaded.
pub fn prepare(
    mut fp: ResMut<FpBow>,
    mut done: Local<bool>,
    models: Option<Res<super::viewmodel::Models>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    if *done {
        return;
    }
    let (Some(models), Some(row)) = (models, bow_row()) else {
        return;
    };
    let Some(mesh) = meshes.get(&models.row(row).0) else {
        return;
    };
    *done = true;
    let (Some(VertexAttributeValues::Float32x3(pos)), Some(ind)) =
        (mesh.attribute(Mesh::ATTRIBUTE_POSITION), mesh.indices())
    else {
        warn!("bow: the hunting bow has no readable positions; its string stays baked");
        return;
    };
    let idx: Vec<u32> = ind.iter().map(|i| i as u32).collect();
    let Some((keep, lo, hi)) = strip_string(pos, &idx) else {
        warn!("bow: found no string in the hunting bow; its string stays baked and does not draw");
        return;
    };
    let mut own = mesh.clone();
    own.insert_indices(Indices::U32(keep));
    fp.mesh = Some(meshes.add(own));
    fp.ends = [lo, hi];
}

/// The string and the arrow, spawned hidden under the held model once.
pub(super) fn spawn_parts(
    model: &mut ChildSpawnerCommands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let cord = meshes.add(Cylinder::new(STRING_R_M, 1.0).mesh().resolution(6));
    let waxed = materials.add(StandardMaterial {
        base_color: Color::srgb(0.80, 0.75, 0.63),
        perceptual_roughness: 0.7,
        reflectance: super::fresnel::DIELECTRIC,
        ..default()
    });
    for upper in [false, true] {
        model.spawn((
            BowString { upper },
            Mesh3d(cord.clone()),
            MeshMaterial3d(waxed.clone()),
            Transform::default(),
            Visibility::Hidden,
        ));
    }
    model.spawn((
        NockedArrow,
        Mesh3d(meshes.add(super::heldgen::arrow_mesh(ARROW_LEN_M))),
        MeshMaterial3d(materials.add(super::heldgen::arrow_material())),
        Transform::default(),
        Visibility::Hidden,
    ));
}

/// Where the nock is, model space: on the braced string at [`NOCK_Y_M`],
/// pulled `back` metres toward the archer (+X, the string side).
pub fn nock_at(ends: [Vec3; 2], back: f32) -> Vec3 {
    let [lo, hi] = ends;
    let t = ((NOCK_Y_M - lo.y) / (hi.y - lo.y)).clamp(0.0, 1.0);
    lo.lerp(hi, t) + Vec3::X * back
}

/// How far back the nock sits for a draw state: the draw eased so it leaves
/// the frame early (there is no drawing hand to hold it), plus the shiver
/// after a loose. `loose` counts down from 1 to 0 across the loose.
pub fn nock_back(pull: f32, loose: f32) -> f32 {
    let p = pull.clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - p) * (1.0 - p);
    let t = (1.0 - loose) * super::viewmodel::VIEWMODEL_LOOSE_S;
    let shiver = SHIVER_M * loose * loose * (std::f32::consts::TAU * SHIVER_HZ * t).sin();
    DRAW_M * eased + shiver
}

/// A unit cylinder along +Y stretched from `a` to `b`. The string runs up the
/// bow, so `b - a` is never near −Y and the arc is well defined.
fn span(a: Vec3, b: Vec3) -> Transform {
    let d = b - a;
    Transform {
        translation: (a + b) * 0.5,
        rotation: Quat::from_rotation_arc(Vec3::Y, d.normalize_or(Vec3::Y)),
        scale: Vec3::new(1.0, d.length(), 1.0),
    }
}

/// Draw the string and the arrow for this frame's draw.
#[allow(clippy::type_complexity)]
pub fn drive(
    fp: Res<FpBow>,
    motion: Res<Motion>,
    held: Query<&HeldModel>,
    mut strings: Query<(&BowString, &mut Transform, &mut Visibility), Without<NockedArrow>>,
    mut arrow: Query<(&mut Transform, &mut Visibility), (With<NockedArrow>, Without<BowString>)>,
) {
    let shown = held.iter().next().and_then(HeldModel::shown);
    let on = fp.mesh.is_some() && shown.is_some() && shown == bow_row();
    let want = if on {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    let pose = motion.bow();
    let nock = on.then(|| nock_at(fp.ends, nock_back(pose.pull, pose.loose)));
    for (s, mut t, mut v) in &mut strings {
        v.set_if_neq(want);
        if let Some(nock) = nock {
            let [lo, hi] = fp.ends;
            *t = if s.upper {
                span(nock, hi)
            } else {
                span(lo, nock)
            };
        }
    }
    let nocked = nock.filter(|_| pose.nocked);
    for (mut t, mut v) in &mut arrow {
        v.set_if_neq(if nocked.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if let Some(nock) = nocked {
            // Off the nock and past the riser on the arrow shelf, so the
            // shaft slides over the rest as the string comes back.
            let rest = Vec3::new(0.0, NOCK_Y_M, ARROW_SIDE_M);
            let dir = (rest - nock).normalize_or(Vec3::NEG_X);
            *t = Transform {
                translation: nock,
                rotation: Quat::from_rotation_arc(Vec3::Y, dir),
                scale: Vec3::ONE,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A thin strand beside a fat block: the strand goes, the block stays,
    /// and the strand's two ends come back.
    #[test]
    fn the_strand_is_taken_and_the_rest_kept() {
        let mut pos = Vec::new();
        let mut idx = Vec::new();
        let mut quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| {
            let n = pos.len() as u32;
            pos.extend([a, b, c, d]);
            idx.extend([n, n + 1, n + 2, n, n + 2, n + 3]);
        };
        // The block: 0.1 thick in z.
        quad(
            [0.0, 0.0, -0.05],
            [0.1, 0.0, -0.05],
            [0.1, 1.0, 0.05],
            [0.0, 1.0, 0.05],
        );
        // The strand: 1.5 long, flat in z.
        quad(
            [0.2, 0.05, 0.0],
            [0.21, 0.05, 0.0],
            [0.21, 1.55, 0.0],
            [0.2, 1.55, 0.0],
        );
        let (keep, lo, hi) = strip_string(&pos, &idx).expect("a strand");
        assert_eq!(keep, vec![0, 1, 2, 0, 2, 3]);
        assert!((lo.y - 0.05).abs() < 1e-6 && (hi.y - 1.55).abs() < 1e-6);
    }

    #[test]
    fn no_strand_is_none() {
        let pos = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.5]];
        assert!(strip_string(&pos, &[0, 1, 2]).is_none());
    }

    /// The nock is on the braced string at rest, pulled straight back at full
    /// draw, and the shiver is gone once the loose is over.
    #[test]
    fn the_nock_follows_the_draw() {
        let ends = [Vec3::new(0.19, 0.05, 0.0), Vec3::new(0.19, 1.6, 0.0)];
        let rest = nock_at(ends, nock_back(0.0, 0.0));
        assert!((rest - Vec3::new(0.19, NOCK_Y_M, 0.0)).length() < 1e-5);
        let full = nock_at(ends, nock_back(1.0, 0.0));
        assert!((full - rest - Vec3::X * DRAW_M).length() < 1e-5);
        assert!(nock_back(0.5, 0.0) > 0.5 * DRAW_M, "the draw is eased out");
        assert!(
            nock_back(0.0, 1.0).abs() < 1e-6,
            "the shiver starts at brace"
        );
    }
}
