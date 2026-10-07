//! The right hand's finger bones (`ci/rig_fingers.py`), closed on what it holds.
//!
//! **At rest they change nothing.** Each bone is bound exactly where
//! `ci/curl_hands.py` bent that point of the digit, so a hand at its bones'
//! rest rotations is the shipped half-curl to the vertex — a body holding
//! nothing, the left hand, and every gate that reads the mesh all see what
//! they saw before the bones existed.
//!
//! **Holding a row that closes the fist** (`HeldModelDef::grip_roll`), the
//! hand turns each bone by [`closed`] on top of its rest: the fingers in to a
//! 3.4 cm handle on `viewmodel::VIEWMODEL_SEAT`'s grip line, and the thumb
//! round over the front of it. Before the bones the thumb stood out from the
//! fist and the fingers rested a centimetre off the handle, which is the
//! hand that read as "kinda odd" (operator, 2026-10-06).
//!
//! The first-person arms ease with the hand fit ([`Motion::grip`]); a remote
//! body snaps on the transition its held row changes on ([`Grip`]).

use bevy::prelude::*;

use super::viewmodel::{Motion, ViewArms};

/// How closed a remote body's right hand is: 0 at the rest curl, 1 closed on
/// a handle. On the body root, written by `bodies::update_hand`.
#[derive(Component, Default, Clone, Copy, PartialEq, Debug)]
pub struct Grip(pub f32);

/// One right-hand finger bone, with the rotation it was bound at.
#[derive(Component)]
pub struct FingerBone {
    digit: usize,
    joint: usize,
    rest: Quat,
    /// The rig root it closes with — the first-person arms or a body —
    /// found once by climbing, `bodies::bind_hands`' way.
    owner: Option<Entity>,
}

/// The digits in the order `ci/rig_fingers.py` names them.
pub const DIGITS: [&str; 5] = ["Thumb", "Index", "Middle", "Ring", "Pinky"];
/// The bone the finger bones hang under.
const HAND: &str = super::anim::HAND_BONE;

/// How far each finger closes past its rest curl, degrees over its three
/// bones, Index to Pinky. Measured off the rigged file against the grip line:
/// the index already touches a 3.4 cm handle at rest, the middle and ring
/// need 12° and 24° to reach it, and the pinky sits past the end of the
/// handle's diagonal, so it closes into the palm as a fist does.
const FINGER_DEG: [f32; 4] = [0.0, 12.0, 24.0, 45.0];
/// How a finger's closing splits over its knuckle, middle and last joints.
const FINGER_SPLIT: [f32; 3] = [0.35, 0.40, 0.25];
/// The thumb's base turn, about this axis in its own bone frame: mostly the
/// curl run backwards and a little across, which swings it off the side of
/// the fist and round over the front of the handle. A pure curl cannot get
/// there — the rest thumb curls into the palm, away from the handle — and
/// this was the closest of 2,000 sampled turns to touching it (2.5 cm off
/// the handle's axis, never inside it).
const THUMB_AXIS: Vec3 = Vec3::new(-0.986, -0.031, -0.162);
const THUMB_TURN_DEG: f32 = 110.0;
/// Then the thumb's two outer joints close down onto the handle.
const THUMB_CURL_DEG: [f32; 2] = [40.0, 32.0];

/// The closed grip's turn for one bone, on top of its rest rotation. A
/// positive turn about a finger bone's local +X closes it — the axis
/// `ci/rig_fingers.py` builds each frame on.
pub fn closed(digit: usize, joint: usize) -> Quat {
    if digit == 0 {
        return match joint {
            0 => Quat::from_axis_angle(THUMB_AXIS.normalize(), THUMB_TURN_DEG.to_radians()),
            j => Quat::from_rotation_x(THUMB_CURL_DEG[j - 1].to_radians()),
        };
    }
    Quat::from_rotation_x((FINGER_DEG[digit - 1] * FINGER_SPLIT[joint]).to_radians())
}

/// The bone a scene node's name is, if it is one of the right hand's.
pub fn parse(name: &str) -> Option<(usize, usize)> {
    let rest = name.strip_prefix(HAND)?;
    let (digit, n) = rest.split_at(rest.len().checked_sub(1)?);
    let digit = DIGITS.iter().position(|d| *d == digit)?;
    let joint = n.parse::<usize>().ok()?.checked_sub(1)?;
    (joint < 3).then_some((digit, joint))
}

/// A scene node as it spawns, not yet bound.
type Spawned<'w, 's> =
    Query<'w, 's, (Entity, &'static Name, &'static Transform), (Added<Name>, Without<FingerBone>)>;

/// Record every right-hand finger bone as its scene spawns, with the
/// rotation it was bound at.
pub fn bind(mut commands: Commands, added: Spawned) {
    for (e, name, t) in &added {
        if let Some((digit, joint)) = parse(name.as_str()) {
            commands.entity(e).insert(FingerBone {
                digit,
                joint,
                rest: t.rotation,
                owner: None,
            });
        }
    }
}

/// Close each right hand as far as its rig's grip says. Between the
/// animation and the propagation, `viewmodel::pose_hand`'s window: no clip
/// animates a finger, so the rest rotation is the bone's own and this is
/// the only writer.
pub fn pose(
    m: Res<Motion>,
    grips: Query<&Grip>,
    view: Query<(), With<ViewArms>>,
    parents: Query<&ChildOf>,
    mut bones: Query<(Entity, &mut FingerBone, &mut Transform)>,
) {
    for (e, mut bone, mut t) in &mut bones {
        let owner = match bone.owner {
            Some(o) => o,
            None => {
                let mut at = e;
                let mut found = None;
                for _ in 0..32 {
                    match parents.get(at) {
                        Ok(p) => at = p.0,
                        Err(_) => break,
                    }
                    if view.contains(at) || grips.contains(at) {
                        found = Some(at);
                        break;
                    }
                }
                let Some(o) = found else { continue };
                bone.owner = Some(o);
                o
            }
        };
        let k = if view.contains(owner) {
            m.grip()
        } else {
            grips.get(owner).map_or(0.0, |g| g.0)
        };
        let want = bone.rest * Quat::IDENTITY.slerp(closed(bone.digit, bone.joint), k);
        if t.rotation != want {
            t.rotation = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rig_s_names_parse_and_nothing_else_does() {
        assert_eq!(parse("RightHandThumb1"), Some((0, 0)));
        assert_eq!(parse("RightHandPinky3"), Some((4, 2)));
        assert_eq!(parse("RightHand"), None);
        assert_eq!(parse("RightHandIndex4"), None);
        assert_eq!(parse("LeftHandIndex1"), None);
        assert_eq!(parse("RightForeArm"), None);
    }

    #[test]
    fn closing_turns_every_finger_inward_and_the_index_not_at_all() {
        for d in 1..5 {
            for j in 0..3 {
                let (axis, ang) = closed(d, j).to_axis_angle();
                assert!(
                    ang.abs() < 1e-6 || axis.x * ang > 0.0,
                    "digit {d} joint {j}"
                );
            }
        }
        assert_eq!(closed(1, 0), Quat::IDENTITY);
    }
}
