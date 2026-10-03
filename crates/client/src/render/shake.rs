//! Camera shake: the frame moves when something big happens near you.
//!
//! Two parts. **Trauma** (Eiserloh, GDC 2016, "Juicing Your Cameras With
//! Math"): events add trauma in `0..=1`, it decays linearly, and trauma²
//! drives smooth noise on pitch, yaw and roll plus a few centimetres of
//! translation — a blast, a wall coming down, a tree hitting the ground, a
//! close thunderclap, a helicopter overhead. **Kicks** are directional and
//! ride a damped spring on top: your own shot's recoil, a flinch away from a
//! blow, a jolt when a swing connects, a nudge on a reload, the dip of a
//! landing.
//!
//! **No roll and no field-of-view pulse.** Rust's devblogs: their first
//! head-shake pass was "over-done", and FOV bounce and camera roll were what
//! they suspected of making players sick while a plain bounce was fine — so
//! this pitches, yaws and translates, and one slider turns it all down.
//!
//! Purely visual. [`apply`] runs after `rig::follow_eye` has rewritten the
//! camera from `Eye`, and only the camera's `Transform` moves: aim reads
//! `Look`, the listener reads `Eye`, and nothing sent to the server reads the
//! camera. The viewmodel is a child of the camera, so it rides the shake.

use bevy::prelude::*;

use super::feed::Feed;
use super::impact::{ContactKind, Contacts, Weapon};
use super::input::Look;
use super::rig::EyeCam;
use super::settings::Settings;
use super::weather::WeatherNow;
use super::{Eye, Net};

/// Largest rotation trauma 1 reaches, radians: pitch, yaw, roll (none).
const MAX_ROT: Vec3 = Vec3::new(0.028, 0.016, 0.0);
/// Largest translation trauma 1 reaches, metres.
const MAX_OFF_M: f32 = 0.05;
/// How fast the noise moves, samples per second.
const NOISE_HZ: f32 = 17.0;
/// Trauma lost per second. A full blast shakes for about a second; the
/// visible part (trauma²) for about half of that.
const DECAY_PER_S: f32 = 1.1;

/// The kick spring: ~5 Hz, a little under critically damped, so a kick
/// snaps out and settles back in about a fifth of a second.
const KICK_HZ: f32 = 5.0;
const KICK_DAMP: f32 = 0.72;
/// The landing spring: slower and bouncier.
const DIP_HZ: f32 = 3.2;
const DIP_DAMP: f32 = 0.55;

/// Your own gunshot: recoil up, a little sideways, and a touch of trauma.
const SHOT_KICK_RAD: f32 = 0.016;
const SHOT_SIDE_RAD: f32 = 0.005;
const SHOT_TRAUMA: f32 = 0.10;
/// Your own bow: a much smaller kick.
const BOW_KICK_RAD: f32 = 0.006;
/// Your swing connected with something: a short jolt down.
const SWING_KICK_RAD: f32 = 0.006;

/// A blow landed on you: trauma for the hurt, a flinch away from it.
const HURT_TRAUMA_BASE: f32 = 0.10;
const HURT_TRAUMA_PER_HP: f32 = 0.012;
const HURT_TRAUMA_MAX: f32 = 0.55;
const FLINCH_PITCH_RAD: f32 = 0.020;
const FLINCH_YAW_RAD: f32 = 0.022;
/// Your own reload: the camera moves a little with the hands.
const RELOAD_KICK: Vec3 = Vec3::new(-0.005, 0.004, 0.0);

/// A blast: full trauma inside `BLAST_FULL_M`, none past `BLAST_ZERO_M`.
const BLAST_FULL_M: f32 = 6.0;
const BLAST_ZERO_M: f32 = 75.0;
/// A built piece coming down.
pub const COLLAPSE_TRAUMA: f32 = 0.5;
pub const COLLAPSE_FULL_M: f32 = 4.0;
pub const COLLAPSE_ZERO_M: f32 = 32.0;
/// A felled tree hitting the ground, scaled by the tree's size.
pub const TREE_TRAUMA: f32 = 0.35;
pub const TREE_FULL_M: f32 = 3.0;
pub const TREE_ZERO_M: f32 = 28.0;
/// The helicopter's rotor wash: a floor the trauma cannot fall below.
const HELI_FLOOR: f32 = 0.38;
const HELI_FULL_M: f32 = 12.0;
const HELI_ZERO_M: f32 = 80.0;
/// A thunderclap shakes only when the bolt was close (gain near 1).
const THUNDER_TRAUMA: f32 = 0.3;
const THUNDER_FROM_GAIN: f32 = 0.6;

/// Landing: a fall faster than this (m/s) dips the camera.
const LAND_MIN_MPS: f32 = 3.5;
/// Dip at the minimum and how much more per m/s past it, metres.
const LAND_DIP_M: f32 = 0.012;
const LAND_DIP_PER_MPS: f32 = 0.007;
const LAND_DIP_MAX_M: f32 = 0.11;
/// A landing past this speed is a hard one and shakes.
const LAND_HARD_MPS: f32 = 10.0;

/// The shake state. Pure: [`Shake::step`] advances it and returns the
/// offsets, so it can be driven without a world.
#[derive(Resource, Clone, Debug)]
pub struct Shake {
    trauma: f32,
    /// The trauma floor this frame (the heli); it does not decay, it is
    /// re-measured.
    floor: f32,
    /// Noise time.
    t: f32,
    /// Kick spring: pitch, yaw, roll, radians, and their rates.
    kick: Vec3,
    kick_vel: Vec3,
    /// Landing dip, metres down, and its rate.
    dip: f32,
    dip_vel: f32,
    /// The body's last airborne vertical speed, for the landing.
    was_grounded: bool,
    air_vy: f32,
    /// A cheap generator for the shot's sideways kick.
    rng: u32,
}

impl Default for Shake {
    fn default() -> Self {
        Self {
            trauma: 0.0,
            floor: 0.0,
            t: 0.0,
            kick: Vec3::ZERO,
            kick_vel: Vec3::ZERO,
            dip: 0.0,
            dip_vel: 0.0,
            was_grounded: true,
            air_vy: 0.0,
            rng: 0x9e37_79b9,
        }
    }
}

/// What [`Shake::step`] hands the camera this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Offsets {
    /// Pitch, yaw, roll, radians.
    pub rot: Vec3,
    /// Camera-local translation, metres.
    pub local: Vec3,
    /// World-down dip, metres.
    pub dip: f32,
}

impl Shake {
    /// Add trauma, clamped to 1.
    pub fn add(&mut self, trauma: f32) {
        self.trauma = (self.trauma + trauma.max(0.0)).min(1.0);
    }

    /// Add trauma for something that happened at `at`, heard from `eye`:
    /// all of it inside `full_m`, none past `zero_m`, squared between.
    pub fn add_at(&mut self, trauma: f32, at: Vec3, eye: Vec3, full_m: f32, zero_m: f32) {
        self.add(trauma * near(at.distance(eye), full_m, zero_m));
    }

    /// Kick the view: pitch (up +), yaw (left +), roll, radians at the
    /// spring's peak.
    pub fn kick(&mut self, peak: Vec3) {
        self.kick_vel += peak * peak_to_speed(KICK_HZ, KICK_DAMP);
    }

    /// Dip the camera down by about `metres` at the spring's peak.
    pub fn dip(&mut self, metres: f32) {
        self.dip_vel += metres * peak_to_speed(DIP_HZ, DIP_DAMP);
    }

    /// The trauma right now (before the floor).
    pub fn trauma(&self) -> f32 {
        self.trauma
    }

    /// Feed the body's vertical state; a landing dips and, if hard, shakes.
    pub fn body(&mut self, grounded: bool, vy: f32) {
        if !grounded {
            self.air_vy = vy;
        } else if !self.was_grounded {
            let v = -self.air_vy;
            if v > LAND_MIN_MPS {
                let dip = (LAND_DIP_M + (v - LAND_MIN_MPS) * LAND_DIP_PER_MPS).min(LAND_DIP_MAX_M);
                self.dip(dip);
                self.add(((v - LAND_HARD_MPS) / 8.0).clamp(0.0, 1.0) * 0.35);
            }
            self.air_vy = 0.0;
        }
        self.was_grounded = grounded;
    }

    fn rand(&mut self) -> f32 {
        // xorshift32, mapped to -1..1.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Advance by `dt` seconds and return the offsets at strength `scale`
    /// (the player's setting; 0 is off).
    pub fn step(&mut self, dt: f32, scale: f32) -> Offsets {
        let dt = dt.clamp(0.0, 0.1);
        self.trauma = (self.trauma - DECAY_PER_S * dt).max(0.0);
        self.t += dt * NOISE_HZ;
        // The springs, stepped exactly: a slow frame lands where a fast one
        // would have, and nothing can blow up.
        for i in 0..3 {
            spring(
                &mut self.kick[i],
                &mut self.kick_vel[i],
                KICK_HZ,
                KICK_DAMP,
                dt,
            );
        }
        spring(&mut self.dip, &mut self.dip_vel, DIP_HZ, DIP_DAMP, dt);
        let s = self.trauma.max(self.floor);
        let s = s * s;
        let t = self.t;
        let rot = MAX_ROT * s * Vec3::new(fbm(t, 11), fbm(t, 23), fbm(t, 37)) + self.kick;
        let local = Vec3::new(fbm(t, 41), fbm(t, 53), fbm(t, 67)) * MAX_OFF_M * s;
        let scale = scale.clamp(0.0, 1.0);
        Offsets {
            rot: rot * scale,
            local: local * scale,
            dip: self.dip * scale,
        }
    }
}

/// How much of an event at `d` metres reaches you: 1 inside `full`, 0 past
/// `zero`, falling off as a square between.
pub fn near(d: f32, full: f32, zero: f32) -> f32 {
    let k = ((zero - d) / (zero - full).max(1e-3)).clamp(0.0, 1.0);
    k * k
}

/// Advance an underdamped spring (`damp` < 1) at rest length 0 by `h`
/// seconds, exactly: the closed-form solution, not an integrator.
fn spring(x: &mut f32, v: &mut f32, hz: f32, damp: f32, h: f32) {
    let w = std::f32::consts::TAU * hz;
    let wd = w * (1.0 - damp * damp).sqrt();
    let e = (-damp * w * h).exp();
    let (sn, cs) = (wd * h).sin_cos();
    let a = *x;
    let b = (*v + damp * w * a) / wd;
    let pos = a * cs + b * sn;
    *x = e * pos;
    *v = e * (-damp * w * pos + wd * (b * cs - a * sn));
}

/// The initial speed that makes a damped spring at `hz`/`damp` peak at 1.
fn peak_to_speed(hz: f32, damp: f32) -> f32 {
    let w = std::f32::consts::TAU * hz;
    let wd = w * (1.0 - damp * damp).sqrt();
    let tp = ((1.0 - damp * damp).sqrt() / damp).atan() / wd;
    let peak = (-damp * w * tp).exp() * (wd * tp).sin() / wd;
    1.0 / peak
}

/// Smooth value noise in -1..1, two octaves.
fn fbm(t: f32, seed: u32) -> f32 {
    value(t, seed) * 0.7 + value(t * 2.17 + 13.1, seed ^ 0x5bd1) * 0.3
}

fn value(t: f32, seed: u32) -> f32 {
    let i = t.floor();
    let f = t - i;
    let a = hash(i as i32, seed);
    let b = hash(i as i32 + 1, seed);
    let u = f * f * (3.0 - 2.0 * f);
    a + (b - a) * u
}

fn hash(i: i32, seed: u32) -> f32 {
    let mut x = (i as u32).wrapping_mul(0x27d4_eb2d) ^ seed.wrapping_mul(0x1656_67b1);
    x ^= x >> 15;
    x = x.wrapping_mul(0x85eb_ca6b);
    x ^= x >> 13;
    x = x.wrapping_mul(0xc2b2_ae35);
    x ^= x >> 16;
    (x as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// Read this frame's events into the shake and move the camera.
///
/// After `rig::follow_eye` (which rewrote the camera from `Eye`) and after
/// the contact resolver; before the audio chain, so a thunderclap due this
/// frame is still in the queue `audio::bed` takes it from.
#[allow(clippy::too_many_arguments)]
pub fn apply(
    time: Res<Time>,
    settings: Res<Settings>,
    feed: Res<Feed>,
    contacts: Option<Res<Contacts>>,
    net: Option<NonSend<Net>>,
    eye: Res<Eye>,
    look: Res<Look>,
    weather: Option<Res<WeatherNow>>,
    heli: Query<&GlobalTransform, With<super::heli::HeliBody>>,
    mut shake: ResMut<Shake>,
    mut cam: Query<&mut Transform, With<EyeCam>>,
) {
    if let Some(net) = net.as_deref() {
        let core = &net.session.core;
        // Your own shots, off the same broadcast the tracer and the report
        // read: a firearm's speed on the wire is zero.
        for &(shooter, _, _, speed, _) in feed.shots() {
            if shooter != core.player_id {
                continue;
            }
            if protocol::shot_is_instant(speed) {
                let side = shake.rand() * SHOT_SIDE_RAD;
                shake.kick(Vec3::new(SHOT_KICK_RAD, side, 0.0));
                shake.add(SHOT_TRAUMA);
            } else {
                shake.kick(Vec3::new(BOW_KICK_RAD, 0.0, 0.0));
            }
        }
        let body = &core.predict.body;
        shake.body(body.grounded, body.qvy as f32 * sim_core::movement::VEL_Q);
    }

    // Blows on you: trauma by damage, and a flinch away from where each
    // came from (`Hurt::from` is a compass bearing, like the hurt arc's).
    if feed.hurts > 0 {
        shake.add(
            (HURT_TRAUMA_BASE + feed.hurt_damage as f32 * HURT_TRAUMA_PER_HP).min(HURT_TRAUMA_MAX),
        );
        let facing = crate::look::bearing_deg(look.yaw);
        for h in feed.hurt_from() {
            let bearing = h.from as f32 * (360.0 / sim_core::combat::HURT_SECTORS as f32);
            let rel = (bearing - facing).to_radians();
            // From the front the head snaps back (pitch up); from the left
            // it turns right (yaw is left-positive).
            shake.kick(Vec3::new(
                rel.cos() * FLINCH_PITCH_RAD,
                rel.sin() * FLINCH_YAW_RAD,
                0.0,
            ));
        }
    }
    if feed.reloaded > 0 {
        shake.kick(RELOAD_KICK);
    }

    // Blasts anywhere in reach, and your own swing landing.
    if let Some(contacts) = contacts {
        let mut swung = false;
        for c in contacts.iter() {
            if c.weapon == Weapon::Blast {
                shake.add_at(1.0, c.at, eye.pos, BLAST_FULL_M, BLAST_ZERO_M);
            } else if c.kind == ContactKind::Swing && !swung {
                swung = true;
                shake.kick(Vec3::new(-SWING_KICK_RAD, 0.0, 0.0));
            }
        }
    }

    // A close thunderclap, while it is still queued for `audio::bed`.
    if let Some(w) = weather.as_deref() {
        let now_s = feed.server_tick_est / sim_core::limits::TICK_HZ as f64;
        for &(at, gain) in &w.thunder_due[..w.thunder_n] {
            if at <= now_s {
                let k = ((gain - THUNDER_FROM_GAIN) / (1.0 - THUNDER_FROM_GAIN)).clamp(0.0, 1.0);
                shake.add(THUNDER_TRAUMA * k);
            }
        }
    }

    // The heli's rotor wash, re-measured every frame.
    shake.floor = heli
        .iter()
        .map(|g| HELI_FLOOR * near(g.translation().distance(eye.pos), HELI_FULL_M, HELI_ZERO_M))
        .fold(0.0, f32::max);

    let o = shake.step(time.delta_secs(), settings.shake);
    let Ok(mut t) = cam.single_mut() else {
        return;
    };
    if o == Offsets::default() {
        return;
    }
    let local = t.rotation * o.local;
    t.translation += local - Vec3::Y * o.dip;
    t.rotate_local_x(o.rot.x);
    t.rotate_local_y(o.rot.y);
    t.rotate_local_z(o.rot.z);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kick_peaks_where_it_was_asked_to_and_settles() {
        let mut s = Shake::default();
        s.kick(Vec3::new(0.02, 0.0, 0.0));
        let mut peak = 0.0f32;
        for _ in 0..120 {
            peak = peak.max(s.step(1.0 / 120.0, 1.0).rot.x);
        }
        assert!((peak - 0.02).abs() < 0.002, "peaked at {peak}");
        let rest = s.step(1.0 / 60.0, 1.0).rot.x.abs();
        assert!(rest < 0.001, "still {rest} rad off after a second");
    }

    #[test]
    fn trauma_decays_to_stillness_and_the_setting_turns_it_off() {
        let mut s = Shake::default();
        s.add(1.0);
        assert_eq!(s.clone().step(1.0 / 60.0, 0.0), Offsets::default());
        let moved = s.clone().step(1.0 / 60.0, 1.0);
        assert!(moved.rot.length() > 0.0);
        for _ in 0..120 {
            s.step(1.0 / 60.0, 1.0);
        }
        assert_eq!(s.trauma(), 0.0);
        assert_eq!(s.step(1.0 / 60.0, 1.0).rot, Vec3::ZERO);
    }

    #[test]
    fn only_a_real_fall_dips() {
        let mut s = Shake::default();
        s.body(false, -2.0);
        s.body(true, 0.0);
        assert_eq!(s.step(0.05, 1.0).dip, 0.0, "a step down is not a landing");
        s.body(false, -7.0);
        s.body(true, 0.0);
        let mut deepest = 0.0f32;
        for _ in 0..60 {
            deepest = deepest.max(s.step(1.0 / 120.0, 1.0).dip);
        }
        assert!(deepest > LAND_DIP_M, "a jump's landing dipped {deepest} m");
        assert!(deepest < LAND_DIP_MAX_M);
    }

    #[test]
    fn distance_falls_off_to_nothing() {
        assert_eq!(near(0.0, 6.0, 75.0), 1.0);
        assert_eq!(near(80.0, 6.0, 75.0), 0.0);
        let mid = near(40.0, 6.0, 75.0);
        assert!(mid > 0.0 && mid < 0.5);
    }
}
