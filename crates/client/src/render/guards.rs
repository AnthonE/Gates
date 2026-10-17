//! THE GATE's guards: the green figures, a revolver in the hand, walking the
//! beats `town::GUARD_BEATS` lays out — drawn, never simulated; the sentries
//! are the town's teeth and these are its life. Every client walks them off
//! the server's clock (`Feed::server_tick_est`), so two players see the same
//! guard at the same corner, and their boots sound through the remote
//! footsteps (`audio::remote_steps`).

use bevy::prelude::*;
use sim_core::town;

use super::{WorldEntity, WorldId};

/// One guard, and the beat it walks.
#[derive(Component)]
pub struct Guard(pub usize);

/// The held row a guard carries (`ui::hold::HELD_MODELS`).
const GUN: &str = "revolver";

fn wrap_pi(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    a - tau * ((a + std::f32::consts::PI) / tau).floor()
}

/// Where along beat `k` its guard is at `t_s` seconds of the server's
/// clock: local (x, z), the way it faces (sim yaw: 0 is +Z, toward +X), and
/// whether it is walking. At a corner it looks back down the leg it came
/// by, turns to the next over the middle of the pause, and goes.
pub fn beat_pose(k: usize, t_s: f64) -> Option<((f32, f32), f32, bool)> {
    let beat = town::GUARD_BEATS.get(k)?;
    let n = beat.len();
    let leg = |i: usize| {
        let (a, b) = (beat[i], beat[(i + 1) % n]);
        let (dx, dz) = (b.0 - a.0, b.1 - a.1);
        (a, (dx, dz), (dx * dx + dz * dz).sqrt())
    };
    let walk = |len: f32| f64::from(len / town::GUARD_WALK_MPS);
    let pause = f64::from(town::GUARD_PAUSE_S);
    let lap: f64 = (0..n).map(|i| walk(leg(i).2) + pause).sum();
    // Each beat starts somewhere else round its loop.
    let mut u = (t_s + lap * (k as f64 * 0.37).fract()).rem_euclid(lap);
    for i in 0..n {
        let (a, (dx, dz), len) = leg(i);
        let yaw = dx.atan2(dz);
        let w = walk(len);
        if u < w {
            let f = (u / w) as f32;
            return Some(((a.0 + dx * f, a.1 + dz * f), yaw, true));
        }
        u -= w;
        if u < pause {
            let (_, (nx, nz), _) = leg((i + 1) % n);
            let turn = ((u / pause) as f32 * 2.0 - 0.5).clamp(0.0, 1.0);
            let yaw = yaw + wrap_pi(nx.atan2(nz) - yaw) * turn;
            return Some(((a.0 + dx, a.1 + dz), yaw, false));
        }
        u -= pause;
    }
    let (a, (dx, dz), _) = leg(0);
    Some((a, dx.atan2(dz), false))
}

/// Stand the guards up once per world, when the rig has loaded.
pub fn spawn(
    mut commands: Commands,
    world: Option<Res<WorldId>>,
    rig: Res<super::anim::Rig>,
    drawn: Query<(), With<Guard>>,
) {
    let Some(world) = world else { return };
    if !world.haven.town.live || !drawn.is_empty() || !rig.ready() || !rig.shaded() {
        return;
    }
    let Some(scene) = rig.scene.clone() else {
        return;
    };
    for k in 0..town::GUARD_BEATS.len() {
        commands.spawn((
            WorldEntity,
            Guard(k),
            super::anim::BodyAnim {
                clip: Some(super::anim::Clip::Idle),
                ..default()
            },
            super::anim::Reshade(super::anim::Shade::Keeper),
            super::audio::RemoteSteps::default(),
            SceneRoot(scene.clone()),
            Transform::from_scale(Vec3::splat(rig.scale)),
        ));
    }
}

/// Walk every guard to where the clock says it is on its beat.
pub fn walk(
    feed: Res<super::feed::Feed>,
    world: Option<Res<WorldId>>,
    time: Res<Time>,
    rig: Res<super::anim::Rig>,
    mut guards: Query<(&Guard, &mut Transform, &mut super::anim::BodyAnim)>,
) {
    let Some(world) = world else { return };
    let tf = super::town::transform(&world.haven.town);
    let t_s = feed.server_tick_est / f64::from(sim_core::limits::TICK_HZ);
    for (guard, mut at, mut anim) in &mut guards {
        let Some(((x, z), yaw, _)) = beat_pose(guard.0, t_s) else {
            continue;
        };
        let local = Transform::from_xyz(x, 0.0, z)
            .with_rotation(Quat::from_rotation_y(yaw))
            .with_scale(Vec3::splat(rig.scale));
        *at = tf.mul_transform(local);
        let fwd = at.rotation * Vec3::Z;
        anim.facing = fwd.x.atan2(fwd.z);
        anim.observe(at.translation, time.delta_secs(), false, false, false);
    }
}

/// Put the revolver in each guard's right hand once its skeleton exists
/// (`bodies::bind_hands`' moment and walk, for a guard).
#[allow(clippy::too_many_arguments)]
pub fn arm(
    mut commands: Commands,
    added: Query<Entity, Added<AnimationPlayer>>,
    parents: Query<&ChildOf>,
    children: Query<&Children>,
    names: Query<&Name>,
    guards: Query<(), With<Guard>>,
    models: Option<Res<super::viewmodel::Models>>,
    rig: Res<super::anim::Rig>,
) {
    let Some(models) = models else { return };
    let Some(row) = crate::ui::hold::HELD_MODELS
        .iter()
        .position(|m| m.key == GUN)
    else {
        return;
    };
    for player in &added {
        let mut at = player;
        let mut guard = None;
        for _ in 0..16 {
            if guards.get(at).is_ok() {
                guard = Some(at);
                break;
            }
            match parents.get(at) {
                Ok(p) => at = p.0,
                Err(_) => break,
            }
        }
        let Some(guard) = guard else { continue };
        let mut stack = vec![guard];
        let mut seen = 0usize;
        let mut bone = None;
        while let Some(e) = stack.pop() {
            seen += 1;
            if seen > 512 {
                break;
            }
            if names
                .get(e)
                .is_ok_and(|n| n.as_str() == super::anim::HAND_BONE)
            {
                bone = Some(e);
                break;
            }
            if let Ok(kids) = children.get(e) {
                stack.extend(kids.iter());
            }
        }
        let Some(bone) = bone else { continue };
        let (mesh, mat) = models.row(row);
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            super::bodies::hand_pose(row, rig.scale),
            ChildOf(bone),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A beat is walked round and back to where it started, at the walk's
    /// pace, never jumping: two instants a frame apart are a frame's walk
    /// apart, corners and the lap's wrap included.
    #[test]
    fn a_guard_walks_its_beat_without_jumping() {
        for k in 0..town::GUARD_BEATS.len() {
            let mut last = beat_pose(k, 0.0).unwrap().0;
            let mut walked = false;
            for i in 1..20_000 {
                let ((x, z), _, moving) = beat_pose(k, f64::from(i) / 20.0).unwrap();
                let d = ((x - last.0).powi(2) + (z - last.1).powi(2)).sqrt();
                assert!(
                    d <= town::GUARD_WALK_MPS / 20.0 + 1e-3,
                    "beat {k} jumps {d} m at {i}"
                );
                walked |= moving;
                last = (x, z);
            }
            assert!(walked, "beat {k} never walks");
        }
    }
}
