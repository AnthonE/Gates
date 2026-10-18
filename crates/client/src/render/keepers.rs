//! THE GATE's shopkeepers: one beside each kiosk's machine
//! (`town::KEEPER_SPOTS`), Rust's `NPCShopKeeper` (decompiled server code,
//! research 2026-10-18).
//!
//! What Rust's does, and so what these do:
//!
//! - **Greet.** Every five seconds and up to two more, it picks a visible
//!   player within 10 m in front of it, skipping the last one it waved at,
//!   waves and turns to face them; with nobody, it turns back to its post.
//! - **Serve.** It faces whoever has its shop open, and after a sale it
//!   plays a victory gesture at the buyer.
//! - **Cannot be hurt**, and never leaves its post.
//!
//! The rig has no wave and no victory clip, so the talking loop
//! (`anim::Clip::Talk`) stands in for both. Drawn, never simulated, like the
//! guards: a gesture is a picture of a shop, not a fact anyone acts on, so
//! each client greets its own player and the sim carries nothing but the
//! body's box (`town::KEEPERS`).

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::prelude::*;
use sim_core::town;

use super::panels::{Panel, Ui};
use super::{Net, WorldEntity, WorldId};

/// How far in front a keeper sees a customer, metres (Rust's 10 m).
const GREET_M: f32 = 10.0;
/// A customer this far off is gone, and will be greeted again.
const FORGET_M: f32 = 13.0;
/// Rust's think: every five seconds, and up to two more.
const THINK_S: f32 = 5.0;
const THINK_JITTER_S: f32 = 2.0;
/// How long a wave and a thank-you run, seconds.
const WAVE_S: f32 = 2.6;
const THANKS_S: f32 = 2.0;
/// How fast a keeper turns, rad/s.
const TURN_RADS: f32 = 3.0;

/// Kiosk `k`'s keeper, and what it is doing.
#[derive(Component)]
pub struct Keeper {
    pub k: usize,
    /// Its own facing, sim yaw (0 is +Z, toward +X).
    yaw: f32,
    /// When it next looks for someone to greet, and whether the player has
    /// been greeted since they last walked off.
    next_think: f32,
    greeted: bool,
    /// While this runs, it gestures at the player.
    talk_until: f32,
}

/// The way keeper `k` faces at its post: out across the street.
fn home_yaw(k: usize) -> f32 {
    if town::KEEPER_SPOTS[k].0 < 0.0 {
        FRAC_PI_2
    } else {
        -FRAC_PI_2
    }
}

fn wrap_pi(a: f32) -> f32 {
    a - TAU * ((a + PI) / TAU).floor()
}

/// Stand the keepers up once per world, when the rig has loaded.
pub fn spawn(
    mut commands: Commands,
    world: Option<Res<WorldId>>,
    rig: Res<super::anim::Rig>,
    drawn: Query<(), With<Keeper>>,
) {
    let Some(world) = world else { return };
    if !world.haven.town.live || !drawn.is_empty() || !rig.ready() || !rig.shaded() {
        return;
    }
    let Some(scene) = rig.scene.clone() else {
        return;
    };
    for k in 0..town::KEEPER_SPOTS.len() {
        commands.spawn((
            WorldEntity,
            Keeper {
                k,
                yaw: home_yaw(k),
                // Six keepers do not all think on the same frame.
                next_think: k as f32 * 0.8,
                greeted: false,
                talk_until: 0.0,
            },
            super::anim::BodyAnim {
                clip: Some(super::anim::Clip::Idle),
                ..default()
            },
            super::anim::Reshade(super::anim::Shade::Keeper),
            SceneRoot(scene.clone()),
            Transform::from_scale(Vec3::splat(rig.scale)),
        ));
    }
}

/// Greet, serve and thank, then stand the body where it is and pose it.
pub fn act(
    net: NonSend<Net>,
    feed: Res<super::feed::Feed>,
    ui: Option<Res<Ui>>,
    world: Option<Res<WorldId>>,
    time: Res<Time>,
    rig: Res<super::anim::Rig>,
    mut keepers: Query<(&mut Keeper, &mut Transform, &mut super::anim::BodyAnim)>,
) {
    let Some(world) = world else { return };
    let t = world.haven.town;
    if !t.live {
        return;
    }
    let core = &net.session.core;
    let tf = super::town::transform(&t);
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let [px, _, pz] = core.predict.render_position();
    let (lx, lz) = sim_core::kit::to_local(&t.placed(), px, pz);
    // A sale this frame, by kiosk.
    let mut sold = [false; town::KIOSKS.len()];
    for &(offer, _) in feed.traded() {
        if let Some(o) = core.vend.get(offer as usize) {
            if let Some(s) = sold.get_mut(o.vendor as usize) {
                *s = true;
            }
        }
    }
    let serving = ui
        .as_deref()
        .filter(|u| u.panel == Panel::Vendor)
        .map(|u| u.vendor as usize);
    for (mut keeper, mut at, mut anim) in &mut keepers {
        let k = keeper.k;
        let (x, z) = town::KEEPER_SPOTS[k];
        let (dx, dz) = (lx - x, lz - z);
        let dist = (dx * dx + dz * dz).sqrt();
        let toward = dx.atan2(dz);
        let home = home_yaw(k);
        if dist > FORGET_M {
            keeper.greeted = false;
        }
        if now >= keeper.next_think {
            let jitter = ((k as f32 * 1.618 + now).fract()) * THINK_JITTER_S;
            keeper.next_think = now + THINK_S + jitter;
            let in_front = wrap_pi(toward - home).abs() < FRAC_PI_2;
            if !keeper.greeted && dist <= GREET_M && in_front && !core.dead {
                keeper.greeted = true;
                keeper.talk_until = now + WAVE_S;
            }
        }
        if sold[k] {
            keeper.talk_until = now + THANKS_S;
        }
        let talking = now < keeper.talk_until;
        let want = if talking || serving == Some(k) {
            toward
        } else {
            home
        };
        let turn = wrap_pi(want - keeper.yaw);
        keeper.yaw = wrap_pi(keeper.yaw + turn.clamp(-TURN_RADS * dt, TURN_RADS * dt));

        let local = Transform::from_xyz(x, 0.0, z)
            .with_rotation(Quat::from_rotation_y(keeper.yaw))
            .with_scale(Vec3::splat(rig.scale));
        let want_tf = tf.mul_transform(local);
        if *at != want_tf {
            *at = want_tf;
        }
        let fwd = at.rotation * Vec3::Z;
        anim.facing = fwd.x.atan2(fwd.z);
        anim.observe(at.translation, dt, false, false, false);
        if talking {
            anim.clip = Some(super::anim::Clip::Talk);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A keeper at its post looks out across the street, at the trading spot
    /// it serves.
    #[test]
    fn a_keeper_faces_its_customers() {
        for k in 0..town::KEEPER_SPOTS.len() {
            let (x, z) = town::KEEPER_SPOTS[k];
            let (kx, kz) = town::KIOSKS[k];
            let toward = (kx - x).atan2(kz - z);
            assert!(
                wrap_pi(toward - home_yaw(k)).abs() < FRAC_PI_2,
                "keeper {k} turns its back on its counter"
            );
        }
    }
}
