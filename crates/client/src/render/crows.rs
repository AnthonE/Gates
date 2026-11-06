//! Crows over the dead: a flock circling every fresh corpse near you.
//!
//! The reference's Nov 2026 "Crows and Seagulls": crow flocks find corpses of
//! any kind and circle above them, and an explosion scares them off. Here it
//! is drawn, never decided — the corpses are the bags the client already
//! mirrors (a player's body, an animal's carcass: [`WireBag::kind`] past
//! `BAG_KIND_PACK`), the blasts are the impacts it already hears, and no crow
//! is sim state. What it buys is a tell: a ring of black birds over the
//! treeline is a kill somebody made there, readable from further than the
//! body is.
//!
//! **A fixed pool.** [`FLOCKS`] × [`CROWS`] birds, spawned once on the first
//! frame in a world (and again after a teardown took them), each a body and
//! two wings from [`props::boxes_mesh_with`] — no asset, no per-frame spawn,
//! no allocation. A flock is a small state machine over one number, `out`:
//! 0 circling its corpse, 1 gone (hidden, far off and high), and every crow
//! is drawn at the blend of its two places, so arriving, scattering and
//! leaving are one motion.
//!
//! [`WireBag::kind`]: protocol::event::WireBag

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use super::props::{boxes_mesh_with, linear};
use super::Net;

/// Flocks at once, and birds in each. **(knob)**
pub const FLOCKS: usize = 3;
pub const CROWS: usize = 6;

/// How near you a corpse draws a flock, and how far you go before the flock
/// gives it up for a nearer one, metres.
const NOTICE_M: f32 = 120.0;
const FORGET_M: f32 = 160.0;
/// The circle: radius and height over the corpse, metres, each crow jittered
/// inside the band; one lap's angular speed, radians a second.
const RING_M: (f32, f32) = (6.0, 11.0);
const HIGH_M: (f32, f32) = (13.0, 19.0);
const LAP_RAD_S: f32 = 0.38;
/// Where a crow is when it is not here: this far out along its bearing and
/// this much higher than its circle.
const AWAY_M: f32 = 90.0;
const AWAY_UP_M: f32 = 30.0;
/// Seconds to fly in, to scatter, and to leave a corpse that is gone.
const ARRIVE_S: f32 = 9.0;
const SCATTER_S: f32 = 2.5;
const LEAVE_S: f32 = 6.0;
/// A blast this near a flock's corpse scatters it, and it stays off this
/// long before it comes back.
const SCARE_M: f32 = 45.0;
const SCARED_S: f32 = 25.0;
/// Wingbeats a second, and the flap's swing, radians.
const FLAP_HZ: f32 = 4.2;
const FLAP_RAD: f32 = 0.75;

/// Near-black with a little blue, the crow's own.
const FEATHER: u32 = 0x15161a;

/// The bird, nose down +Z, about a metre across the wings: body, head, tail.
const BODY: &[([f32; 3], [f32; 3], u32)] = &[
    ([0.0, 0.0, 0.0], [0.07, 0.06, 0.2], FEATHER),
    ([0.0, 0.03, 0.24], [0.045, 0.045, 0.05], FEATHER),
    ([0.0, 0.0, -0.27], [0.08, 0.012, 0.09], FEATHER),
];
/// One wing, its root at the body's side so it pivots there: the right one,
/// and the left is the same mesh turned half a turn about Y.
const WING: &[([f32; 3], [f32; 3], u32)] = &[([0.26, 0.0, 0.02], [0.24, 0.01, 0.11], FEATHER)];

#[derive(Resource)]
pub struct CrowAssets {
    body: Handle<Mesh>,
    wing: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

/// One bird: which flock, which of it, where it was drawn last frame (its
/// heading is the difference), and its two wings.
#[derive(Component)]
pub struct Crow {
    flock: u8,
    i: u8,
    prev: Vec3,
    wings: [Entity; 2],
}

/// A wing, flapped about the bird's forward axis; `side` is +1 right, −1 left.
#[derive(Component)]
pub struct Wing {
    side: f32,
}

/// What a flock is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Mode {
    /// No corpse; hidden.
    #[default]
    Free,
    /// Coming in to, or circling, its corpse.
    Here,
    /// Off after a blast, for `away` more seconds.
    Scared,
    /// Its corpse is gone (looted out, rotted, out of range): flying off.
    Leaving,
}

#[derive(Clone, Copy, Debug, Default)]
struct Flock {
    mode: Mode,
    /// The bag it is over (0 none) and where that lies.
    bag: u32,
    at: Vec3,
    /// 0 circling … 1 gone.
    out: f32,
    /// Seconds left of a scare.
    away: f32,
    /// Seconds since it took this corpse: the circle's clock.
    t: f32,
}

/// Every flock's state. Reset when the birds are (re)spawned.
#[derive(Resource, Default)]
pub struct Flocks {
    f: [Flock; FLOCKS],
}

pub fn load(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(CrowAssets {
        body: meshes.add(boxes_mesh_with(BODY, linear, 1.0)),
        wing: meshes.add(boxes_mesh_with(WING, linear, 1.0)),
        material: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.85,
            ..default()
        }),
    });
    commands.init_resource::<Flocks>();
}

/// A crow's fixed draw in `[0, 1)`: its place in the ring, height and pace,
/// from the bag it circles and which bird it is, so one corpse's flock
/// always wheels the same way.
fn roll(bag: u32, i: u8, k: u32) -> f32 {
    let mut h = bag
        .wrapping_mul(0x9e37_79b9)
        .wrapping_add((i as u32) << 8 | k)
        .wrapping_mul(0x85eb_ca6b);
    h ^= h >> 15;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 13;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Where crow `i` of a flock over `bag` at `at` is drawn, `t` seconds into
/// its circle and `out` of the way gone.
pub fn place(bag: u32, at: Vec3, i: u8, t: f32, out: f32) -> Vec3 {
    let r = RING_M.0 + (RING_M.1 - RING_M.0) * roll(bag, i, 1);
    let y = HIGH_M.0 + (HIGH_M.1 - HIGH_M.0) * roll(bag, i, 2);
    // Every bird the same way round (a flock wheels), each at its own pace
    // and place in the ring, bobbing a little.
    let pace = LAP_RAD_S * (0.85 + 0.3 * roll(bag, i, 3));
    let a = std::f32::consts::TAU * roll(bag, i, 4) + pace * t;
    let bob = 0.6 * (t * 0.9 + 6.0 * roll(bag, i, 5)).sin();
    let ring = at + Vec3::new(r * a.cos(), y + bob, r * a.sin());
    // Away: out along a bearing shared by the flock, spread a little.
    let b = std::f32::consts::TAU * roll(bag, 0, 6) + 0.6 * (roll(bag, i, 7) - 0.5);
    let far = at + Vec3::new(AWAY_M * b.cos(), y + AWAY_UP_M, AWAY_M * b.sin());
    ring.lerp(far, smooth(out))
}

/// Is `k` a corpse — a body or a carcass, not a pack?
fn is_corpse(kind: u8) -> bool {
    kind != protocol::event::BAG_KIND_PACK
}

#[allow(clippy::too_many_arguments)]
pub fn drive(
    mut commands: Commands,
    assets: Option<Res<CrowAssets>>,
    flocks: Option<ResMut<Flocks>>,
    feed: Res<super::feed::Feed>,
    time: Res<Time>,
    net: NonSend<Net>,
    mut crows: Query<(&mut Crow, &mut Transform, &mut Visibility), Without<Wing>>,
    mut wings: Query<(&Wing, &mut Transform), Without<Crow>>,
) {
    let (Some(assets), Some(mut flocks)) = (assets, flocks) else {
        return; // Startup has not run yet.
    };
    // The pool: spawned on the first frame in a world, and again after a
    // teardown despawned it (the birds are `WorldEntity` roots).
    if crows.is_empty() {
        flocks.f = [Flock::default(); FLOCKS];
        for flock in 0..FLOCKS as u8 {
            for i in 0..CROWS as u8 {
                let wing = |parent: &mut Commands, side: f32| {
                    let turn = if side > 0.0 {
                        0.0
                    } else {
                        std::f32::consts::PI
                    };
                    parent
                        .spawn((
                            Wing { side },
                            Mesh3d(assets.wing.clone()),
                            MeshMaterial3d(assets.material.clone()),
                            Transform::from_rotation(Quat::from_rotation_y(turn)),
                            NotShadowCaster,
                        ))
                        .id()
                };
                let right = wing(&mut commands, 1.0);
                let left = wing(&mut commands, -1.0);
                commands
                    .spawn((
                        super::WorldEntity,
                        Crow {
                            flock,
                            i,
                            prev: Vec3::ZERO,
                            wings: [right, left],
                        },
                        Mesh3d(assets.body.clone()),
                        MeshMaterial3d(assets.material.clone()),
                        Transform::default(),
                        Visibility::Hidden,
                        NotShadowCaster,
                    ))
                    .add_children(&[right, left]);
            }
        }
        return;
    }

    let core = &net.session.core;
    let dt = time.delta_secs().min(0.1);
    let [px, _, pz] = core.predict.render_position();
    let bag_at = |b: &protocol::event::WireBag| {
        Vec3::new(
            b.qx as f32 * sim_core::movement::POS_XZ_Q,
            b.qy as f32 * sim_core::movement::POS_Y_Q,
            b.qz as f32 * sim_core::movement::POS_XZ_Q,
        )
    };
    let far = |at: Vec3, m: f32| (at.x - px).powi(2) + (at.z - pz).powi(2) > m * m;

    // A blast near a flock scatters it.
    for imp in feed.impacts() {
        if imp.kind != sim_core::ranged::IMPACT_BLAST {
            continue;
        }
        let bx = imp.qx as f32 * sim_core::movement::POS_XZ_Q;
        let bz = imp.qz as f32 * sim_core::movement::POS_XZ_Q;
        for f in flocks.f.iter_mut() {
            if matches!(f.mode, Mode::Here | Mode::Scared)
                && (f.at.x - bx).powi(2) + (f.at.z - bz).powi(2) < SCARE_M * SCARE_M
            {
                f.mode = Mode::Scared;
                f.away = SCARED_S;
            }
        }
    }

    // Each flock's corpse still lies there, near enough to bother with?
    let bags = core.bags.entries();
    for f in flocks.f.iter_mut() {
        if f.bag == 0 || f.mode == Mode::Leaving {
            continue;
        }
        let kept = bags.iter().any(|b| b.id == f.bag && is_corpse(b.kind));
        if !kept || far(f.at, FORGET_M) {
            f.mode = Mode::Leaving;
        }
    }
    // A free flock takes the nearest corpse no flock has.
    for k in 0..FLOCKS {
        if flocks.f[k].mode != Mode::Free {
            continue;
        }
        let mut best: Option<(f32, u32, Vec3)> = None;
        for b in bags.iter().filter(|b| is_corpse(b.kind)) {
            if flocks.f.iter().any(|f| f.bag == b.id) {
                continue;
            }
            let at = bag_at(b);
            let d = (at.x - px).powi(2) + (at.z - pz).powi(2);
            if d < NOTICE_M * NOTICE_M && best.is_none_or(|(bd, _, _)| d < bd) {
                best = Some((d, b.id, at));
            }
        }
        if let Some((_, id, at)) = best {
            flocks.f[k] = Flock {
                mode: Mode::Here,
                bag: id,
                at,
                out: 1.0,
                away: 0.0,
                t: 0.0,
            };
        }
    }
    // Step each flock's one number.
    for f in flocks.f.iter_mut() {
        f.t += dt;
        match f.mode {
            Mode::Free => f.out = 1.0,
            Mode::Here => f.out = (f.out - dt / ARRIVE_S).max(0.0),
            Mode::Scared => {
                f.out = (f.out + dt / SCATTER_S).min(1.0);
                f.away -= dt;
                if f.away <= 0.0 {
                    f.mode = Mode::Here;
                }
            }
            Mode::Leaving => {
                f.out = (f.out + dt / LEAVE_S).min(1.0);
                if f.out >= 1.0 {
                    *f = Flock {
                        out: 1.0,
                        ..Flock::default()
                    };
                }
            }
        }
    }

    // Draw every bird at its blend, facing the way it moved.
    for (mut crow, mut tf, mut vis) in crows.iter_mut() {
        let f = flocks.f[crow.flock as usize];
        if f.mode == Mode::Free || f.out >= 1.0 {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            crow.prev = Vec3::ZERO;
            continue;
        }
        let pos = place(f.bag, f.at, crow.i, f.t, f.out);
        let step = pos - crow.prev;
        if crow.prev != Vec3::ZERO && step.length_squared() > 1e-8 {
            let flat = Vec3::new(step.x, 0.0, step.z);
            let yaw = flat.x.atan2(flat.z);
            // A bank into the turn: every bird circles the same way.
            tf.rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_z(-0.35 * (1.0 - f.out));
        }
        tf.translation = pos;
        crow.prev = pos;
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }
        // Flap flying in and out; on the circle, glide with a few beats now
        // and then.
        let beat = f.t * FLAP_HZ * std::f32::consts::TAU + 7.0 * roll(f.bag, crow.i, 8);
        let gliding = f.out <= 0.0 && (f.t * 0.5 + 3.0 * roll(f.bag, crow.i, 9)).sin() < 0.55;
        let swing = if gliding { 0.08 } else { FLAP_RAD * beat.sin() };
        for e in crow.wings {
            if let Ok((w, mut wt)) = wings.get_mut(e) {
                let turn = if w.side > 0.0 {
                    0.0
                } else {
                    std::f32::consts::PI
                };
                wt.rotation = Quat::from_rotation_y(turn) * Quat::from_rotation_z(swing);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_circling_crow_stays_in_its_band_over_the_corpse() {
        let at = Vec3::new(100.0, 5.0, 200.0);
        for i in 0..CROWS as u8 {
            for t in [0.0, 3.0, 40.0] {
                let p = place(7, at, i, t, 0.0);
                let r = Vec2::new(p.x - at.x, p.z - at.z).length();
                assert!((RING_M.0 - 0.01..=RING_M.1 + 0.01).contains(&r), "ring {r}");
                let h = p.y - at.y;
                assert!((HIGH_M.0 - 1.0..=HIGH_M.1 + 1.0).contains(&h), "height {h}");
            }
        }
    }

    #[test]
    fn a_gone_crow_is_far_off_and_high() {
        let at = Vec3::ZERO;
        let p = place(7, at, 2, 5.0, 1.0);
        assert!(Vec2::new(p.x, p.z).length() > AWAY_M * 0.9);
        assert!(p.y > HIGH_M.0 + AWAY_UP_M - 1.0);
    }
}
