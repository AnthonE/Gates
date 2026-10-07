//! Arrows standing in bodies — the players and animals they hit.
//!
//! Rust's arrows stay in whoever they hit, and so do ours (`sim-core`'s
//! `spent.rs`): the arrow rides the body until the lodge runs out or the
//! body dies, and then falls where it stands. The server walks those arrows
//! to every client (`SUB_LODGED_SYNC`, wire v94) as the body each is in and
//! where in it it went in and which way, **in the body's own frame** — so
//! this draws each one off the body's drawn transform, and it turns, walks
//! and falls with the body without a byte more crossing.
//!
//! Bevy draws, it does not decide: nothing here is read back, and an arrow
//! whose body is not drawn (out of view, the local player's own) is simply
//! not drawn either.
//!
//! A fixed pool, spawned once, for `tracer.rs`'s reason: an arrow going into
//! a body in a fight must not spawn an entity.

use bevy::prelude::*;

use super::anim::BodyAnim;
use super::bodies::Body;
use super::mobs::Animal;
use super::Net;
use sim_core::collide::{CAPSULE_HEIGHT_M, CROUCH_HEIGHT_M};

/// Arrows drawn in bodies at once. A view cap, not the store's: past it the
/// rest are not drawn, which reads as nothing at all.
pub const LODGED_DRAWN: usize = 48;

/// The shaft, metres — the length and radius of an arrow standing in the
/// ground (`render::structures`), so one reads as the other.
const LEN_M: f32 = 0.8;
const RADIUS_M: f32 = 0.012;
/// How much of it is in the body.
const IN_M: f32 = 0.12;
const COLOR: Color = Color::srgb(0.71, 0.58, 0.40);

/// The pool.
#[derive(Resource, Default)]
pub struct Lodged {
    entities: Vec<Entity>,
}

/// Spawn the pool, hidden.
pub fn setup(
    mut commands: Commands,
    mut pool: ResMut<Lodged>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cylinder::new(RADIUS_M, LEN_M));
    let material = materials.add(StandardMaterial {
        base_color: COLOR,
        perceptual_roughness: 0.8,
        ..default()
    });
    pool.entities = (0..LODGED_DRAWN)
        .map(|_| {
            commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id()
        })
        .collect();
}

/// Where an arrow in a body stands, given the body's drawn root (its feet,
/// turned to its facing): the point it went in, `off` centimetres in the
/// body's frame — `height` of the way up it, 1 standing — and the shaft
/// back out against `dir`, the way it flew.
pub fn lodged_transform(root: &Transform, off: [i16; 3], dir: [i8; 3], height: f32) -> Transform {
    let local = Vec3::new(off[0] as f32, off[1] as f32 * height, off[2] as f32) / 100.0;
    let flew = Vec3::new(dir[0] as f32, dir[1] as f32, dir[2] as f32).normalize_or(Vec3::Z);
    let at = root.translation + root.rotation * local;
    let flew = root.rotation * flew;
    // The cylinder's +Y runs from the head to the nock.
    let rotation = Quat::from_rotation_arc(Vec3::Y, -flew);
    Transform::from_translation(at - flew * (LEN_M * 0.5 - IN_M)).with_rotation(rotation)
}

/// Stand every arrow the server says is in a drawn body in it.
#[allow(clippy::type_complexity)]
pub fn draw(
    pool: Res<Lodged>,
    net: NonSend<Net>,
    bodies: Query<(&Body, &Transform, Option<&BodyAnim>), Without<Animal>>,
    animals: Query<(&Animal, &Transform), Without<Body>>,
    mut q: Query<(&mut Transform, &mut Visibility), (Without<Body>, Without<Animal>)>,
) {
    let lodged = net.session.core.lodged();
    let mut shown = 0usize;
    for rec in lodged {
        if shown == pool.entities.len() {
            break;
        }
        // A crouched body is shorter, and an arrow in its chest sinks with
        // it — by the sim's own two hit heights.
        let root = if sim_core::mob::slot_of_id(rec.host).is_some() {
            animals
                .iter()
                .find(|(a, _)| a.0 == rec.host)
                .map(|(_, t)| (t, 1.0))
        } else {
            bodies
                .iter()
                .find(|(b, ..)| b.0 == rec.host)
                .map(|(_, t, anim)| {
                    let low = anim.is_some_and(|a| a.crouched);
                    (
                        t,
                        if low {
                            CROUCH_HEIGHT_M / CAPSULE_HEIGHT_M
                        } else {
                            1.0
                        },
                    )
                })
        };
        let Some((root, height)) = root else {
            continue;
        };
        let Ok((mut tf, mut vis)) = q.get_mut(pool.entities[shown]) else {
            continue;
        };
        *tf = lodged_transform(root, rec.off, rec.dir, height);
        if *vis != Visibility::Visible {
            *vis = Visibility::Visible;
        }
        shown += 1;
    }
    for &e in &pool.entities[shown..] {
        if let Ok((_, mut vis)) = q.get_mut(e) {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
        }
    }
}
