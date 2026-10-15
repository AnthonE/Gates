//! Shared held models resting in the world, with a tied canvas pouch for
//! items without a model. Placement is visual only; pickup uses the wire.

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;

use super::props::Soup;

/// Height of the near terrain's actual triangles. Small loot can disappear
/// under the chord between metre-spaced vertices even when its server height
/// is correct for the continuous heightfield.
pub fn surface_y(seed: u64, haven: &sim_core::terrain::Haven, x: f32, z: f32) -> f32 {
    let step = super::terrain_mesh::CHUNK_M / (super::terrain_mesh::NEAR_N - 1) as f32;
    let ox = (x / step).floor() * step;
    let oz = (z / step).floor() * step;
    let u = (x - ox) / step;
    let v = (z - oz) / step;
    let h = |dx, dz| sim_core::terrain::ground(seed, haven, ox + dx * step, oz + dz * step);
    // heightfield's [a,c,b], [b,c,d] diagonal.
    if u + v <= 1.0 {
        h(0.0, 0.0) * (1.0 - u - v) + h(1.0, 0.0) * u + h(0.0, 1.0) * v
    } else {
        h(1.0, 1.0) * (u + v - 1.0) + h(1.0, 0.0) * (1.0 - v) + h(0.0, 1.0) * (1.0 - u)
    }
}

/// A folded sack, rooted at zero, inside the existing loose-stack envelope.
/// The pinched neck and flared mouth distinguish it from a death backpack.
pub fn sack_mesh(size: [f32; 3]) -> Mesh {
    let mut s = Soup::tiling(1.0);
    let profile = [
        (0.0, 0.55),
        (0.12, 0.90),
        (0.40, 1.0),
        (0.65, 0.80),
        (0.80, 0.26),
        (0.88, 0.25),
        (1.0, 0.52),
    ];
    let point = |row: usize, side: usize| {
        let (y, radius) = profile[row];
        let a = side as f32 * std::f32::consts::TAU / 12.0;
        let fold = if side.is_multiple_of(2) { 1.0 } else { 0.88 };
        Vec3::new(
            a.cos() * radius * fold * size[0] * 0.5,
            (y - if row == profile.len() - 1 && !side.is_multiple_of(2) {
                0.08
            } else {
                0.0
            }) * size[1],
            a.sin() * radius * fold * size[2] * 0.5,
        )
    };
    for row in 0..profile.len() - 1 {
        for side in 0..12 {
            let a = point(row, side);
            let b = point(row + 1, side);
            let c = point(row + 1, side + 1);
            let d = point(row, side + 1);
            // Vertex value follows the fold; the material supplies canvas.
            let value = if row == 4 { 0.55 } else { 0.92 };
            let color = |_| [value, value, value, 1.0];
            let center = Some(Vec3::Y * size[1] * 0.4);
            let soften = if row < 3 { 0.5 } else { 0.0 };
            s.tri(a, b, c, color, center, soften);
            s.tri(a, c, d, color, center, soften);
        }
    }
    for side in 0..12 {
        s.tri(
            Vec3::ZERO,
            point(0, side),
            point(0, side + 1),
            |_| [0.7, 0.7, 0.7, 1.0],
            None,
            0.0,
        );
        let top = profile.len() - 1;
        s.tri(
            Vec3::Y * size[1] * 0.88,
            point(top, side + 1),
            point(top, side),
            |_| [0.8, 0.8, 0.8, 1.0],
            None,
            0.0,
        );
    }
    s.mesh()
}

/// The turn that rests a model on its broadest face: its thinnest axis up.
/// A hafted tool falls on its side, a coil of rope or a pile of ore stays
/// as it sits, and a shirt lies flat rather than standing like a sign.
fn lie_flat(positions: &[[f32; 3]]) -> Quat {
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for p in positions {
        lo = lo.min(Vec3::from(*p));
        hi = hi.max(Vec3::from(*p));
    }
    let e = hi - lo;
    if e.y <= e.x && e.y <= e.z {
        Quat::IDENTITY
    } else if e.x <= e.z {
        Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
    } else {
        Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)
    }
}

/// Lay a model on its broadest face and seat its transformed bounds on the
/// surface.
/// The hand's grip offset is deliberately absent: it has no meaning on dirt.
/// Sampling the footprint also covers wire-height quantization and slopes.
pub fn resting_transform(
    mesh: &Mesh,
    position: Vec3,
    id: u32,
    scale: f32,
    laid: bool,
    ground: impl Fn(f32, f32) -> f32,
) -> Transform {
    let yaw = Quat::from_rotation_y((id % 4) as f32 * std::f32::consts::FRAC_PI_2);
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return Transform::from_translation(position);
    };
    let mut rotation = yaw
        * if laid {
            lie_flat(positions)
        } else {
            Quat::IDENTITY
        };
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for p in positions {
        let p = rotation * (Vec3::from(*p) * scale);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let center = (lo + hi) * 0.5;
    // Follow the terrain's local plane, then lift only where curvature or
    // quantization puts a corner below it. A horizontal spear seated at the
    // highest corner of its world AABB would hover over a hillside.
    let radius = ((hi - lo).xz().length() * 0.5).max(f32::EPSILON);
    let dx = (ground(position.x + radius, position.z) - ground(position.x - radius, position.z))
        / (2.0 * radius);
    let dz = (ground(position.x, position.z + radius) - ground(position.x, position.z - radius))
        / (2.0 * radius);
    let tilt = Quat::from_rotation_arc(Vec3::Y, Vec3::new(-dx, 1.0, -dz).normalize());
    rotation = tilt * rotation;
    let mut translation = position - tilt * Vec3::new(center.x, lo.y, center.z);
    let mut lift = 0.0_f32;
    for x in [lo.x, center.x, hi.x] {
        for z in [lo.z, center.z, hi.z] {
            let p = translation + tilt * Vec3::new(x, lo.y, z);
            lift = lift.max(ground(p.x, p.z) - p.y);
        }
    }
    translation.y += lift;
    Transform {
        translation,
        rotation,
        scale: Vec3::splat(scale),
    }
}

/// Ticks a thrown stack spends after it touches down: a hop, then a short
/// roll into the pose it rests in.
pub const SETTLE_TICKS: f32 = 12.0;
/// How far it rolls, metres, and at most what share of the throw's length:
/// a stack dropped at the feet barely moves.
const ROLL_M: f32 = 0.25;
const ROLL_SHARE: f32 = 0.25;
/// The hop off the ground, metres, and the share of the settle it takes.
const HOP_M: f32 = 0.06;
const HOP_SHARE: f32 = 0.5;

/// What the wire said about a throw (v111), kept beside the stack so a
/// model that loads mid-flight takes the flight over rather than starting
/// it again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    /// The server tick it left the hand.
    pub born: f64,
    /// Ticks in the air (`sim_core::grounditem::toss_ticks`).
    pub ticks: u32,
    /// Where it left the hand, world metres.
    pub hand: Vec3,
}

impl Flight {
    /// The throw `g` is flying on, put back beside the server tick `now`;
    /// `None` when it carries none or its picture is already over.
    pub fn of(g: &protocol::WireGItem, now: f64) -> Option<Self> {
        use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
        let t = g.toss?;
        let f = Self {
            born: toss_born(t.born_lo, now),
            ticks: sim_core::grounditem::toss_ticks(t.from[1] as f32 * POS_Y_Q),
            hand: Vec3::new(
                (g.qx + t.from[0] as i32) as f32 * POS_XZ_Q,
                (g.qy + t.from[1] as i32) as f32 * POS_Y_Q,
                (g.qz + t.from[2] as i32) as f32 * POS_XZ_Q,
            ),
        };
        (!f.over(now)).then_some(f)
    }

    /// Whether the flight and the settle after it are done by `now`.
    pub fn over(&self, now: f64) -> bool {
        now >= self.born + self.ticks as f64 + SETTLE_TICKS as f64
    }
}

/// The server tick a throw left the hand, from the low byte the wire
/// carries: the tick nearest `now` with that byte. A flight is a few dozen
/// ticks and the byte spans 256, so the nearest one is the one.
pub fn toss_born(born_lo: u8, now: f64) -> f64 {
    let now = now.max(0.0) as i64;
    let d = (born_lo as i64 - now).rem_euclid(256);
    (now + if d >= 128 { d - 256 } else { d }) as f64
}

/// A loose stack still on its way out of a hand (wire v111): the flight the
/// sim keeps in closed form (`sim_core::grounditem::toss_at`) carries the
/// model's middle from the hand to just short of where it rests, tumbling,
/// then it hops and rolls the rest of the way into its resting pose. Only
/// the picture: the record already lies where this ends, and a take reads
/// the record (`reference/LOOT.md` §9.3).
#[derive(Component, Clone, Copy, Debug)]
pub struct Tossed {
    flight: Flight,
    /// The pose it ends in (`resting_transform`).
    rest: Transform,
    /// The model's own box, unscaled, and its middle.
    lo: Vec3,
    hi: Vec3,
    mid: Vec3,
    /// Where its middle touches down.
    land: Vec3,
    /// What it tumbles about, and how far before it rests, radians.
    axis: Vec3,
    spin: f32,
    /// The lowest its box goes at rest: nothing in flight goes under it.
    floor: f32,
}

/// Where a thrown stack is drawn at a moment ([`Tossed::pose`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pose {
    /// Not out of the hand yet on this clock: hidden.
    Waiting,
    Flying(Transform),
    Rested,
}

impl Tossed {
    /// The throw of the stack `id` drawn with `mesh`, to rest at `rest`.
    pub fn new(flight: Flight, rest: Transform, mesh: &Mesh, id: u32) -> Self {
        let (lo, hi) = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(ps)) if !ps.is_empty() => ps.iter().fold(
                (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                |(lo, hi), p| (lo.min(Vec3::from(*p)), hi.max(Vec3::from(*p))),
            ),
            _ => (Vec3::ZERO, Vec3::ZERO),
        };
        let mid = (lo + hi) * 0.5;
        let rest_mid = rest.transform_point(mid);
        let reach = (rest_mid - flight.hand).xz();
        let ahead = Vec3::new(reach.x, 0.0, reach.y).normalize_or(Vec3::X);
        let land = rest_mid - ahead * (reach.length() * ROLL_SHARE).min(ROLL_M);
        // Tumbling forward, as a thing thrown underhand does, about an
        // axis and through an angle drawn from the id so a pile of drops
        // does not turn in step.
        let h = id.wrapping_mul(0x9E37_79B9);
        let unit = |shift: u32| ((h >> shift) & 0xFF) as f32 / 255.0;
        let axis = Quat::from_rotation_y((unit(8) - 0.5) * 0.8)
            * Quat::from_axis_angle(ahead, (unit(16) - 0.5) * 0.8)
            * Vec3::Y.cross(ahead);
        let mut t = Self {
            flight,
            rest,
            lo,
            hi,
            mid,
            land,
            axis: axis.normalize_or(Vec3::Z),
            spin: std::f32::consts::TAU * (0.75 + 0.75 * unit(24)),
            floor: 0.0,
        };
        t.floor = rest_mid.y + t.low(rest.rotation);
        t
    }

    /// The pose it ends in.
    pub fn rest(&self) -> Transform {
        self.rest
    }

    /// The lowest point of the model's box turned by `rotation`, relative
    /// to its middle: each axis reaches down by its half extent times how
    /// far it leans.
    fn low(&self, rotation: Quat) -> f32 {
        let half = (self.hi - self.lo) * 0.5 * self.rest.scale;
        let lean = |axis: Vec3| (rotation * axis).y.abs();
        -(lean(Vec3::X) * half.x + lean(Vec3::Y) * half.y + lean(Vec3::Z) * half.z)
    }

    /// Where it is drawn at server tick `now`.
    pub fn pose(&self, now: f64) -> Pose {
        let t = (now - self.flight.born) as f32;
        let air = self.flight.ticks.max(1) as f32;
        if t < 0.0 {
            return Pose::Waiting;
        }
        if t >= air + SETTLE_TICKS {
            return Pose::Rested;
        }
        let rest_mid = self.rest.transform_point(self.mid);
        // The share of the spin done in the air, so it turns as fast as it
        // lands as it did flying, then slows to a stop.
        let share = 2.0 * air / (SETTLE_TICKS + 2.0 * air);
        let (mut at, done) = if t < air {
            let p = sim_core::grounditem::toss_at(
                self.flight.hand.into(),
                self.land.into(),
                self.flight.ticks,
                t,
            );
            (Vec3::from(p), share * t / air)
        } else {
            let v = (t - air) / SETTLE_TICKS;
            let ease = 1.0 - (1.0 - v) * (1.0 - v);
            let h = v / HOP_SHARE;
            let hop = if h < 1.0 {
                HOP_M * 4.0 * h * (1.0 - h)
            } else {
                0.0
            };
            (
                self.land.lerp(rest_mid, ease) + Vec3::Y * hop,
                share + (1.0 - share) * ease,
            )
        };
        let rotation =
            Quat::from_axis_angle(self.axis, -self.spin * (1.0 - done)) * self.rest.rotation;
        // A long thing landing on its end stands on it, then tips over.
        at.y = at.y.max(self.floor - self.low(rotation));
        Pose::Flying(Transform {
            translation: at - rotation * (self.mid * self.rest.scale),
            rotation,
            scale: self.rest.scale,
        })
    }
}

/// Fly every thrown stack to where it rests, then let it lie.
pub fn fly(
    mut commands: Commands,
    feed: Res<super::feed::Feed>,
    mut flying: Query<(Entity, &Tossed, &mut Transform, &mut Visibility)>,
) {
    for (entity, toss, mut transform, mut visibility) in &mut flying {
        let shown = match toss.pose(feed.server_tick_est) {
            Pose::Waiting => false,
            Pose::Flying(pose) => {
                *transform = pose;
                true
            }
            Pose::Rested => {
                *transform = toss.rest();
                commands.entity(entity).remove::<Tossed>();
                true
            }
        };
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}
