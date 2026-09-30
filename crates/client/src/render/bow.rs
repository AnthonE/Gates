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
//! (`Motion::bow`); this only draws it, and puts the left hand on the string
//! to draw it ([`draw_arm`]).

use std::collections::HashMap;

use bevy::math::Affine3A;
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;

use super::viewmodel::{
    HeldModel, HeldRig, Motion, VIEWMODEL_HIDDEN_OFFSET, VIEWMODEL_HIDDEN_SCALE,
};

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
/// metres. Short of a real draw (0.5 would take it to the cheek): the
/// first-person shoulders sit in front of the eye, so a hand drawn that far
/// is a fist over half the frame. This keeps the drawing hand beside the bow
/// and out of the crosshair.
pub const DRAW_M: f32 = 0.25;
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

/// How far back the nock sits for a draw state: the draw eased in and out,
/// plus the shiver after a loose. `loose` counts down from 1 to 0 across the
/// loose.
pub fn nock_back(pull: f32, loose: f32) -> f32 {
    let p = pull.clamp(0.0, 1.0);
    let eased = p * p * (3.0 - 2.0 * p);
    let t = (1.0 - loose) * super::viewmodel::VIEWMODEL_LOOSE_S;
    let shiver = SHIVER_M * loose * loose * (std::f32::consts::TAU * SHIVER_HZ * t).sin();
    DRAW_M * eased + shiver
}

/// Where the drawing arm's shoulder joint is set while the bow is up, in the
/// viewmodel rig's frame (`viewmodel::HeldRig`), metres: below and left of
/// the frame. The arms rig's own left shoulder sits a hand's width from the
/// eye, and an arm from there fills the view; from here the forearm comes
/// in from the lower left, the way an archer's does.
pub const DRAW_SHOULDER: Vec3 = Vec3::new(0.121, -0.555, -0.187);
/// Which way the drawing elbow points, same frame: out, down and a little
/// back.
pub const DRAW_ELBOW: Vec3 = Vec3::new(-0.542, -0.824, 0.165);
/// Where the wrist sits from the nock, metres: back along the arrow and down,
/// so the string runs between the fingers (a 20 cm hand) and the arrow shows
/// above them rather than the fingertips reaching over the crosshair. The
/// fingers aim just under the nock.
pub const DRAW_WRIST_BACK_M: f32 = 0.12;
pub const DRAW_WRIST_DROP_M: f32 = 0.03;
pub const DRAW_FINGERS_DROP_M: f32 = 0.025;

/// The first-person arms' left arm, found when the arms are dressed
/// (`viewmodel::dress_arms`): shoulder, upper arm, forearm, hand.
#[derive(Resource, Default)]
pub struct DrawArm {
    pub bones: Option<[Entity; 4]>,
}

/// `e`'s transform in `root`'s frame, from this frame's local transforms.
fn in_frame(
    e: Entity,
    root: Entity,
    parents: &Query<&ChildOf>,
    xf: &Query<&mut Transform>,
) -> Option<Affine3A> {
    let mut m = Affine3A::IDENTITY;
    let mut at = e;
    for _ in 0..32 {
        if at == root {
            return Some(m);
        }
        m = xf.get(at).ok()?.compute_affine() * m;
        at = parents.get(at).ok()?.0;
    }
    None
}

fn rot(m: &Affine3A) -> Quat {
    m.to_scale_rotation_translation().1
}

/// The turn from one direction to another, or none if they are opposite
/// (where the arc has no axis).
fn arc(from: Vec3, to: Vec3) -> Quat {
    let (a, b) = (from.normalize_or_zero(), to.normalize_or_zero());
    if a.dot(b) < -0.999 {
        return Quat::IDENTITY;
    }
    Quat::from_rotation_arc(a, b)
}

/// Where a shoulder at `s`, with an upper arm `a` long and a forearm `b`
/// long, puts its elbow to reach toward `t`, bending toward `pole`; and the
/// point it actually reaches (`t`, or as near as the arm goes).
pub fn two_bone(s: Vec3, t: Vec3, a: f32, b: f32, pole: Vec3) -> (Vec3, Vec3) {
    let to = t - s;
    let u = to.normalize_or(Vec3::NEG_Z);
    let d = to.length().clamp((a - b).abs() + 1e-3, a + b - 1e-3);
    let v = (pole - u * pole.dot(u)).normalize_or(Vec3::NEG_Y);
    let cos = ((a * a + d * d - b * b) / (2.0 * a * d)).clamp(-1.0, 1.0);
    let sin = (1.0 - cos * cos).max(0.0).sqrt();
    (s + a * (u * cos + v * sin), s + u * d)
}

/// Put the left hand on the string while the bow is up.
///
/// The first-person arms hold everything in the right hand and fold the left
/// arm away (`viewmodel::VIEWMODEL_HIDDEN_ARM`). A raised bow brings it back:
/// the shoulder is set at [`DRAW_SHOULDER`] and the arm solved so the wrist is
/// just behind the nock, so a hand draws the string instead of the string
/// drawing itself. Between the animation and the propagation like
/// `anim::head_look`: it overrides this frame's hold pose on the left arm and
/// costs no second propagation.
#[allow(clippy::type_complexity)]
pub fn draw_arm(
    fp: Res<FpBow>,
    motion: Res<Motion>,
    arm: Res<DrawArm>,
    rig: Query<Entity, With<HeldRig>>,
    held: Query<(Entity, &HeldModel)>,
    parents: Query<&ChildOf>,
    mut xf: Query<&mut Transform>,
) {
    let Some([sh, up, fore, hand]) = arm.bones else {
        return;
    };
    let pose = motion.bow();
    let model = held
        .iter()
        .find(|(_, h)| h.shown().is_some() && h.shown() == bow_row())
        .map(|(e, _)| e);
    let solved = match (model, rig.single()) {
        (Some(model), Ok(rig)) if fp.mesh.is_some() && pose.raise > 0.5 => solve(
            &fp,
            pose.hand,
            sh,
            [up, fore, hand],
            model,
            rig,
            &parents,
            &xf,
        ),
        _ => None,
    };
    let Some((t_sh, [q_up, q_fore, q_hand])) = solved else {
        // Folded away, as `dress_arms` left it.
        if let Ok(mut t) = xf.get_mut(sh) {
            if t.translation != VIEWMODEL_HIDDEN_OFFSET {
                t.translation = VIEWMODEL_HIDDEN_OFFSET;
                t.scale = Vec3::splat(VIEWMODEL_HIDDEN_SCALE);
            }
        }
        return;
    };
    for (e, write) in [
        (sh, None),
        (up, Some(q_up)),
        (fore, Some(q_fore)),
        (hand, Some(q_hand)),
    ] {
        if let Ok(mut t) = xf.get_mut(e) {
            match write {
                None => {
                    t.translation = t_sh;
                    t.scale = Vec3::ONE;
                }
                Some(q) => t.rotation = q,
            }
        }
    }
}

/// The left arm's new locals: the shoulder's translation, then the upper
/// arm's, the forearm's and the hand's rotations. Everything is worked in the
/// viewmodel rig's frame.
#[allow(clippy::too_many_arguments)]
fn solve(
    fp: &FpBow,
    pull: f32,
    sh: Entity,
    [up, fore, hand]: [Entity; 3],
    model: Entity,
    rig: Entity,
    parents: &Query<&ChildOf>,
    xf: &Query<&mut Transform>,
) -> Option<(Vec3, [Quat; 3])> {
    let local = |e: Entity| xf.get(e).ok().copied();
    let (t_sh, t_up, t_fore, t_hand) = (local(sh)?, local(up)?, local(fore)?, local(hand)?);
    // The nock the fingers hold and the arrow's line through it.
    let m_model = in_frame(model, rig, parents, xf)?;
    let at = nock_at(fp.ends, nock_back(pull, 0.0));
    let nock = m_model.transform_point3(at);
    let fwd = m_model
        .transform_vector3(Vec3::new(0.0, NOCK_Y_M, ARROW_SIDE_M) - at)
        .normalize_or(Vec3::NEG_Z);
    let wrist = nock - fwd * DRAW_WRIST_BACK_M + Vec3::NEG_Y * DRAW_WRIST_DROP_M;
    let fingers = nock + Vec3::NEG_Y * DRAW_FINGERS_DROP_M;
    // The shoulder, moved so the upper arm starts at `DRAW_SHOULDER`.
    let m_p = in_frame(parents.get(sh).ok()?.0, rig, parents, xf)?;
    let t_sh_new = m_p.inverse().transform_point3(DRAW_SHOULDER) - t_sh.rotation * t_up.translation;
    let m_sh = m_p * Affine3A::from_rotation_translation(t_sh.rotation, t_sh_new);
    let m_up = m_sh * t_up.compute_affine();
    let m_fore = m_up * t_fore.compute_affine();
    let (s, e0) = (Vec3::from(m_up.translation), Vec3::from(m_fore.translation));
    let h0 = Vec3::from((m_fore * t_hand.compute_affine()).translation);
    let (elbow, reach) = two_bone(s, wrist, e0.distance(s), h0.distance(e0), DRAW_ELBOW);
    // The upper arm onto the elbow, the forearm onto the wrist, the hand's
    // fingers onto the string.
    let q_up = rot(&m_sh).inverse() * arc(e0 - s, elbow - s) * rot(&m_up);
    let m_up = m_sh * Affine3A::from_scale_rotation_translation(t_up.scale, q_up, t_up.translation);
    let m_fore = m_up * t_fore.compute_affine();
    let e1 = Vec3::from(m_fore.translation);
    let h1 = Vec3::from((m_fore * t_hand.compute_affine()).translation);
    let q_fore = rot(&m_up).inverse() * arc(h1 - e1, reach - e1) * rot(&m_fore);
    let m_fore =
        m_up * Affine3A::from_scale_rotation_translation(t_fore.scale, q_fore, t_fore.translation);
    let m_hand = m_fore * t_hand.compute_affine();
    let q_hand = rot(&m_fore).inverse()
        * arc(
            rot(&m_hand) * Vec3::Y,
            fingers - Vec3::from(m_hand.translation),
        )
        * rot(&m_hand);
    Some((t_sh_new, [q_up, q_fore, q_hand]))
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

    /// The elbow is an upper arm from the shoulder and a forearm from the
    /// wrist, on the pole's side; out of reach, the arm points straight at it.
    #[test]
    fn the_arm_reaches_and_bends_the_right_way() {
        let (s, t, pole) = (Vec3::ZERO, Vec3::new(0.0, 0.0, -0.4), Vec3::NEG_Y);
        let (e, r) = two_bone(s, t, 0.25, 0.25, pole);
        assert!((e.length() - 0.25).abs() < 1e-4 && (r.distance(e) - 0.25).abs() < 1e-4);
        assert!((r - t).length() < 1e-4 && e.y < 0.0, "elbow {e}");
        let (_, r) = two_bone(s, Vec3::new(0.0, 0.0, -2.0), 0.25, 0.25, pole);
        assert!(r.z < -0.49 && r.x.abs() < 1e-5, "reach {r}");
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
        assert!((nock_back(0.5, 0.0) - 0.5 * DRAW_M).abs() < 1e-6);
        assert!(
            nock_back(0.0, 1.0).abs() < 1e-6,
            "the shiver starts at brace"
        );
    }
}
