//! The world's own effects: what a piece throws going up and coming down, a
//! footstep's puff, and a fire's flames, embers and smoke.
//!
//! Like the rest of `fx`, nothing here decides anything — every burst is a
//! fact the feed or the mirror already holds (`feed.placed()`,
//! `feed.removed()`, the lit set behind `structures::FireLight`).

use bevy::prelude::*;

use super::super::audio::Sound;
use super::super::decal::Marks;
use super::super::feed::Feed;
use super::super::impact::{Burst, Chips, Contact, ContactKind, Matter, Weapon};
use super::super::{structures, surface, Eye, Net, WorldId};
use super::pool::{Particle, Pool};
use super::table::{emit, lod, scaled, Layer};
use super::{atlas, Fx};
use crate::sound::mixer::Request;
use crate::sound::Cue;
use sim_core::build::{LEVEL_H_M, LOC_DIAG_A, LOC_DIAG_B, LOC_EDGE_XLO, LOC_EDGE_ZLO};

/// Is this address a wall — a standing slab — rather than a floor, a
/// triangle or a stair?
pub fn is_wall(loc: u8) -> bool {
    matches!(loc, LOC_EDGE_XLO | LOC_EDGE_ZLO | LOC_DIAG_A | LOC_DIAG_B)
}

/// Where a coming-down piece breaks, as offsets from its base in its own
/// frame (x across, y up, z along): a wall across its face, anything else
/// across its footprint.
pub fn break_points(wall: bool) -> [Vec3; 5] {
    if wall {
        [
            Vec3::new(0.0, 0.5, -1.0),
            Vec3::new(0.0, 1.5, 0.0),
            Vec3::new(0.0, 2.5, 1.0),
            Vec3::new(0.0, 0.6, 1.1),
            Vec3::new(0.0, 2.3, -1.1),
        ]
    } else {
        [
            Vec3::new(-0.9, 0.1, -0.9),
            Vec3::new(0.9, 0.1, -0.9),
            Vec3::new(0.0, 0.1, 0.0),
            Vec3::new(-0.9, 0.1, 0.9),
            Vec3::new(0.9, 0.1, 0.9),
        ]
    }
}

/// How far from a gone piece's middle its marks go with it, metres: past
/// the corner of a 3 m wall or floor (1.5·√2 ≈ 2.12 m from its middle). At
/// 1.8 a wall's four corners kept their holes after it came down.
pub const FORGET_R_M: f32 = 2.2;

/// The matter a piece of `row` is built of.
fn piece_matter(core: &client_core::core::ClientCore, row: u8) -> Matter {
    if (row as u16) < core.piece_defs_have {
        Matter::of_piece(core.piece_defs.pieces[row as usize].material)
    } else {
        Matter::Wood
    }
}

/// The matter a deployable of `row` is made of.
fn deploy_matter(core: &client_core::core::ClientCore, row: u8) -> Matter {
    if (row as u16) < core.deploy_defs_have {
        let d = &core.deploy_defs.defs[row as usize];
        surface::arch_matter(d.arch, d.hp)
    } else {
        Matter::Wood
    }
}

/// Pieces going up and coming down.
///
/// **Up**: a skirt of dust along the foot of what was set down — the
/// hammer's thump already plays (`audio::place`). **Down** (decay, a raid,
/// a hammer): the piece breaks where it stood — chips and dust of what it
/// was made of at five points across it, a cloud over the whole, the
/// collapse (a piece's; a deployable just goes), and every mark that was on
/// it goes with it.
#[allow(clippy::too_many_arguments)]
pub fn built(
    feed: Res<Feed>,
    net: Option<NonSend<Net>>,
    world: Option<Res<WorldId>>,
    eye: Res<Eye>,
    mut fx: ResMut<Fx>,
    mut chips: ResMut<Chips>,
    mut marks: ResMut<Marks>,
    mut sound: ResMut<Sound>,
) {
    if feed.placed().is_empty() && feed.removed().is_empty() {
        return;
    }
    let (Some(net), Some(world)) = (net, world) else {
        return;
    };
    let core = &net.session.core;
    for &(cx, cz, level, loc, deploy) in feed.placed() {
        let plate = core.pieces.cols().plate(cx, cz).unwrap_or(0);
        let tf = structures::base_transform(world.seed, &world.haven, (cx, cz, level, loc), plate);
        let k = lod(tf.translation.distance(eye.pos));
        if k <= 0.0 {
            continue;
        }
        let matter = if deploy {
            core.deploys
                .entries()
                .iter()
                .find(|r| (r.cx, r.cz, r.level, r.loc) == (cx, cz, level, loc))
                .map_or(Matter::Wood, |r| deploy_matter(core, r.row))
        } else {
            core.pieces
                .entries()
                .iter()
                .find(|r| (r.cx, r.cz, r.level, r.loc) == (cx, cz, level, loc))
                .map_or(Matter::Wood, |r| piece_matter(core, r.row))
        };
        let foot = if deploy || is_wall(loc) {
            [Vec3::new(0.0, 0.05, -1.1), Vec3::new(0.0, 0.05, 1.1)]
        } else {
            [Vec3::new(-1.3, 0.05, 0.0), Vec3::new(1.3, 0.05, 0.0)]
        };
        let n = scaled(if deploy { 2 } else { 4 }, k);
        for f in foot {
            let at = tf.transform_point(f);
            emit(&mut fx.soft, Layer::Dust, n, at, Vec3::Y, matter);
        }
    }
    for r in feed.removed() {
        let tf = structures::base_transform(
            world.seed,
            &world.haven,
            (r.cx, r.cz, r.level, r.loc),
            r.plate,
        );
        let wall = !r.deploy && is_wall(r.loc);
        let middle = tf.translation + Vec3::Y * if wall { LEVEL_H_M * 0.5 } else { 0.3 };
        marks.forget_later(middle, FORGET_R_M);
        // A piece crashes down; a deployable going (picked up, decayed) is
        // dust and no more — a charge on a door already has its blast.
        if !r.deploy {
            sound.play(Request::at(Cue::Collapse, middle.to_array()));
        }
        let d = middle.distance(eye.pos);
        if lod(d) <= 0.0 {
            continue;
        }
        let matter = if r.deploy {
            deploy_matter(core, r.row)
        } else {
            piece_matter(core, r.row)
        };
        // The break itself — chips and, off metal, sparks — is a piece's
        // alone. Run for a deployable too, it threw a spark shower every
        // time somebody picked up a lock or a bench.
        if !r.deploy {
            let across = tf.rotation * Vec3::X;
            for (i, p) in break_points(wall).into_iter().enumerate() {
                let at = tf.transform_point(p);
                // A wall breaks out of both faces; a floor up out of itself.
                let away = if !wall {
                    Vec3::Y
                } else if i % 2 == 0 {
                    across
                } else {
                    -across
                };
                let c = Contact {
                    at,
                    away,
                    normal: away,
                    surf: sim_core::ranged::SURF_BUILT,
                    matter,
                    kind: ContactKind::Impact,
                    weapon: Weapon::Melee,
                    mark: false,
                };
                let n = fx.impact(&c, eye.pos);
                if n > 0 {
                    chips.ignite_n(&Burst { at, away, matter }, n);
                }
            }
        }
        // The cloud the whole thing leaves hanging.
        let n = scaled(if r.deploy { 3 } else { 8 }, lod(d));
        emit(&mut fx.soft, Layer::Dust, n, middle, Vec3::Y, matter);
    }
}

/// A felled trunk hitting the ground: dust along its length where it lands
/// and needles thrown off the crown. `base` is where it stood, `bearing` the
/// way it fell (`props::fell_bearing`), `scale` its drawn size. On the
/// graded ground the trunk is drawn lying on (`terrain::ground`), never the
/// raw heightfield.
pub fn landing(fx: &mut Fx, world: &WorldId, base: Vec3, bearing: f32, scale: f32, eye: Vec3) {
    let k = lod(base.distance(eye));
    if k <= 0.0 {
        return;
    }
    let dir = Vec3::new(bearing.sin(), 0.0, bearing.cos());
    let len = super::super::props::PINE_H * scale;
    let ground = |p: Vec3| {
        let y = sim_core::terrain::ground(world.seed, &world.haven, p.x, p.z);
        Vec3::new(p.x, y + 0.1, p.z)
    };
    for i in 1..=5 {
        let at = ground(base + dir * (len * i as f32 / 6.0));
        emit(
            &mut fx.soft,
            Layer::Dust,
            scaled(3, k),
            at,
            Vec3::Y,
            Matter::Dirt,
        );
    }
    let crown = ground(base + dir * (len * 0.7)) + Vec3::Y * 0.6;
    emit(
        &mut fx.soft,
        Layer::Leaves,
        scaled(12, k),
        crown,
        Vec3::Y,
        Matter::Plant,
    );
}

/// Remote footsteps farther than this throw nothing, metres.
pub const STEP_FX_M: f32 = 30.0;

/// A step's puff: sand kicks a little up when running, water rings and
/// splashes, nothing else shows. Rust's own rule — footstep particles on
/// water, sand and snow and nowhere else.
pub fn footstep(fx: &mut Fx, cue: Cue, at: Vec3, gain: f32) {
    match cue {
        Cue::StepSand | Cue::RemoteStepSand if gain > 0.8 => {
            emit(
                &mut fx.soft,
                Layer::Dust,
                1,
                at + Vec3::Y * 0.05,
                Vec3::Y,
                Matter::Sand,
            );
        }
        Cue::StepWater | Cue::RemoteStepWater => {
            let at = Vec3::new(at.x, sim_core::terrain::SEA_LEVEL + 0.02, at.z);
            emit(&mut fx.soft, Layer::Ring, 1, at, Vec3::Y, Matter::Water);
            if gain > 0.6 {
                emit(&mut fx.soft, Layer::Droplets, 3, at, Vec3::Y, Matter::Water);
            }
        }
        _ => {}
    }
}

/// A burning thing's flames and smoke, carried beside its
/// `structures::FireLight`: whether it shows open flame (a fire pit does, a
/// furnace keeps it inside), and where the flames and the smoke leave from,
/// as heights above the light.
#[derive(Component, Clone, Copy, Debug)]
pub struct FireFx {
    pub flames: bool,
    pub flame_dy: f32,
    pub smoke_dy: f32,
    /// How big the fire is: 1 for a fire pit, a fraction for a torch in a
    /// hand — its tongues' spread, their size and how many there are.
    pub scale: f32,
}

/// The torch's fire, as a fraction of a fire pit's.
pub const TORCH_FIRE_SCALE: f32 = 0.35;

/// Fires get flames, embers and smoke within this range, metres.
pub const FIRE_FX_M: f32 = 40.0;
/// Fires drawn at once, the nearest — a base with a row of furnaces does
/// not get to spend the pools.
pub const FIRE_FX_MAX: usize = 6;
/// Particles a second: flame tongues, embers, smoke.
pub const FLAME_RATE: f32 = 26.0;
pub const EMBER_RATE: f32 = 3.0;
pub const SMOKE_RATE: f32 = 2.2;

/// How many of a `rate`-per-second stream fall in `dt`, rounding the
/// remainder by a roll so a slow stream still flows.
fn count(pool: &mut Pool, rate: f32, dt: f32) -> usize {
    let x = rate * dt;
    let whole = x.floor();
    whole as usize + usize::from(pool.roll() < x - whole)
}

/// Keep every lit fire near the eye burning: flame tongues licking up off
/// the logs, embers riding the heat, and smoke leaving above.
pub fn fires(
    time: Res<Time>,
    eye: Res<Eye>,
    mut fx: ResMut<Fx>,
    q: Query<(&FireFx, &PointLight, &GlobalTransform)>,
) {
    let dt = time.delta_secs().min(0.1);
    if dt <= 0.0 {
        return;
    }
    // The nearest few lit fires, by insertion into a fixed list.
    let mut near = [(f32::MAX, Vec3::ZERO, None::<FireFx>); FIRE_FX_MAX];
    for (f, light, gt) in &q {
        if light.intensity <= 0.0 {
            continue;
        }
        let at = gt.translation();
        let d = at.distance(eye.pos);
        if d > FIRE_FX_M {
            continue;
        }
        if let Some(i) = near.iter().position(|n| d < n.0) {
            near.copy_within(i..FIRE_FX_MAX - 1, i + 1);
            near[i] = (d, at, Some(*f));
        }
    }
    let Fx { glow, soft, .. } = &mut *fx;
    for (_, at, f) in near {
        let Some(f) = f else { break };
        if f.flames {
            let base = at + Vec3::Y * f.flame_dy;
            let k = f.scale;
            for _ in 0..count(glow, FLAME_RATE * k.max(0.5), dt) {
                let off = Vec3::new(glow.signed() * 0.16 * k, 0.0, glow.signed() * 0.16 * k);
                let life = 0.35 + 0.3 * glow.roll();
                let size = (0.14 + 0.08 * glow.roll()) * k;
                let rise = (0.9 + 0.6 * glow.roll()) * k.sqrt();
                let roll = glow.roll() * std::f32::consts::TAU;
                let spin = glow.signed() * 1.5;
                let cell = atlas::PUFF + ((glow.roll() * 3.0) as u8).min(2);
                glow.spawn(Particle {
                    left: life,
                    life,
                    pos: base + off,
                    vel: Vec3::new(-off.x * 0.8, rise, -off.z * 0.8),
                    size0: size,
                    size1: size * 0.25,
                    c0: [2.6, 1.05, 0.28, 0.9],
                    c1: [0.9, 0.18, 0.03, 0.0],
                    gravity: -0.6,
                    drag: 1.2,
                    roll,
                    spin,
                    cell,
                    ..Particle::default()
                });
            }
        }
        for _ in 0..count(
            glow,
            if f.flames {
                EMBER_RATE
            } else {
                EMBER_RATE * 0.4
            },
            dt,
        ) {
            let from = at + Vec3::Y * if f.flames { f.flame_dy } else { f.smoke_dy };
            let life = 1.2 + glow.roll();
            let pos = from + Vec3::new(glow.signed() * 0.12, 0.0, glow.signed() * 0.12);
            let vel = Vec3::new(
                glow.signed() * 0.35,
                1.4 + 1.2 * glow.roll(),
                glow.signed() * 0.35,
            );
            glow.spawn(Particle {
                left: life,
                life,
                pos,
                vel,
                size0: 0.012,
                size1: 0.006,
                c0: [7.0, 2.6, 0.5, 1.0],
                c1: [2.0, 0.3, 0.05, 0.0],
                gravity: -0.3,
                drag: 0.9,
                cell: atlas::GLOW,
                ..Particle::default()
            });
        }
        for _ in 0..count(soft, SMOKE_RATE, dt) {
            let from = at + Vec3::Y * f.smoke_dy;
            emit(soft, Layer::Smoke, 1, from, Vec3::Y, Matter::Stone);
        }
    }
}
