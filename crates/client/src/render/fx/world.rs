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

/// A free-placed deployable's half footprint `(hw, hd)`, or `None` for an
/// edge insert or a row whose archetype has not dripped in.
fn body_half_foot(core: &client_core::core::ClientCore, loc: u8, row: u8) -> Option<(f32, f32)> {
    if sim_core::deploy::is_edge_loc(loc)
        || (row as u16) >= core.deploy_defs_have.min(core.deploy_defs.def_count)
    {
        return None;
    }
    sim_core::footprint::half_foot(core.deploy_defs.defs[row as usize].arch)
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
    mut shake: Option<ResMut<super::super::shake::Shake>>,
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
        // A deployable puffs where it stands (free placement), along its
        // own depth; a piece, and anything the mirror lost, at its address.
        let rec = if deploy {
            core.deploys
                .entries()
                .iter()
                .find(|r| (r.cx, r.cz, r.level, r.loc) == (cx, cz, level, loc))
        } else {
            None
        };
        let tf = match rec {
            Some(r) => structures::deploy_fx_feet(
                world.seed,
                &world.haven,
                core.pieces.cols(),
                &core.deploy_defs,
                core.deploy_defs_have,
                r,
            ),
            None => {
                structures::base_transform(world.seed, &world.haven, (cx, cz, level, loc), plate)
            }
        };
        let k = lod(tf.translation.distance(eye.pos));
        if k <= 0.0 {
            continue;
        }
        let matter = if deploy {
            rec.map_or(Matter::Wood, |r| deploy_matter(core, r.row))
        } else {
            core.pieces
                .entries()
                .iter()
                .find(|r| (r.cx, r.cz, r.level, r.loc) == (cx, cz, level, loc))
                .map_or(Matter::Wood, |r| piece_matter(core, r.row))
        };
        let depth = rec
            .and_then(|r| body_half_foot(core, r.loc, r.row))
            .map(|(_, hd)| hd);
        let foot = if let Some(hd) = depth {
            [Vec3::new(0.0, 0.05, -hd), Vec3::new(0.0, 0.05, hd)]
        } else if deploy || is_wall(loc) {
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
        // A free-placed deployable went from where it stood, and only its
        // own marks go with it — not every wall mark within a cell's reach.
        let body = if r.deploy {
            body_half_foot(core, r.loc, r.row)
        } else {
            None
        };
        let tf = match body {
            Some(_) => structures::body_feet(
                world.seed,
                &world.haven,
                core.pieces.cols(),
                &sim_core::deploy::DeployRec {
                    cx: r.cx,
                    cz: r.cz,
                    level: r.level,
                    loc: r.loc,
                    pose: r.pose,
                    row: r.row,
                    ..Default::default()
                },
                core.deploy_defs.defs[r.row as usize].arch,
            ),
            None => structures::base_transform(
                world.seed,
                &world.haven,
                (r.cx, r.cz, r.level, r.loc),
                r.plate,
            ),
        };
        let wall = !r.deploy && is_wall(r.loc);
        let middle = tf.translation + Vec3::Y * if wall { LEVEL_H_M * 0.5 } else { 0.3 };
        let forget = body.map_or(FORGET_R_M, |(hw, hd)| hw.max(hd) + 0.3);
        marks.forget_later(middle, forget);
        // A piece crashes down; a deployable going (picked up, decayed) is
        // dust and no more — a charge on a door already has its blast.
        if !r.deploy {
            // Planks splinter; stone and sheet metal crash.
            let have = core.piece_defs_have.min(core.piece_defs.piece_count);
            let wooden = (r.row as u16) < have
                && core.piece_defs.pieces[r.row as usize].material <= sim_core::build::MAT_WOOD;
            let cue = if wooden {
                Cue::CollapseWood
            } else {
                Cue::Collapse
            };
            sound.play(Request::at(cue, middle.to_array()));
            if let Some(shake) = shake.as_deref_mut() {
                use super::super::shake::{COLLAPSE_FULL_M, COLLAPSE_TRAUMA, COLLAPSE_ZERO_M};
                shake.add_at(
                    COLLAPSE_TRAUMA,
                    middle,
                    eye.pos,
                    COLLAPSE_FULL_M,
                    COLLAPSE_ZERO_M,
                );
            }
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
                    skin: None,
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

/// A body breaking the surface: rings spread from it and spray goes up, more
/// of both the harder it went in (`gain` is the splash cue's own, `0..1`).
pub fn splash(fx: &mut Fx, at: Vec3, gain: f32) {
    let at = Vec3::new(at.x, sim_core::terrain::SEA_LEVEL + 0.02, at.z);
    emit(&mut fx.soft, Layer::Ring, 2, at, Vec3::Y, Matter::Water);
    let n = scaled(8, gain.clamp(0.3, 1.0));
    emit(&mut fx.soft, Layer::Droplets, n, at, Vec3::Y, Matter::Water);
}

/// A body landing from a jump or a drop: the ground it hits kicks up — sand
/// most, soil a little, rock and water not at all (the water's splash is its
/// own crossing).
pub fn body_landing(fx: &mut Fx, cue: Cue, at: Vec3) {
    let (n, matter) = match cue {
        Cue::StepSand => (3, Matter::Sand),
        Cue::StepGrass | Cue::StepLitter => (1, Matter::Dirt),
        _ => return,
    };
    emit(
        &mut fx.soft,
        Layer::Dust,
        n,
        at + Vec3::Y * 0.05,
        Vec3::Y,
        matter,
    );
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
    /// hand — how far its embers spread.
    pub scale: f32,
    /// What its flame is made of.
    pub tongues: Tongues,
}

/// The flame a burning thing shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tongues {
    /// A pit's: ragged puffs licking up off the logs.
    Pit,
    /// A torch's: small tongues climbing to a tip off its head
    /// ([`torch_flame`]) — another player's, in the world.
    Torch,
    /// Drawn by the emitter's owner in its own frame (your own torch,
    /// `viewmodel::hand_fire`, with [`torch_flame`] too), so this throws
    /// only the embers and the smoke. World-space tongues on a fire carried
    /// half a metre from the eye are left behind by every step and turn.
    Owned,
}

/// The torch's embers, as a fraction of a fire pit's spread.
pub const TORCH_FIRE_SCALE: f32 = 0.35;

/// A torch flame's particles a second: its orange tongues, its yellow heart,
/// the glow under them, and the smoulder and the embers in the wrap.
pub const TORCH_TONGUES: f32 = 40.0;
pub const TORCH_CORES: f32 = 34.0;
pub const TORCH_GLOWS: f32 = 24.0;
pub const TORCH_SMOULDER: f32 = 14.0;
pub const TORCH_EMBERS: f32 = 16.0;

/// `dt` of a torch's flame into `pool`, off a head whose crown is at
/// `crown`, rising along `up` — in whatever frame those are in: the world's
/// for another player's torch (`fires`), the hand's for your own
/// (`viewmodel::hand_fire`). So a torch burns the same in every hand.
///
/// Small tongues out of the whole wrap that climb, speed up and meet in a
/// tip, a hot yellow heart low in the middle, the glow they sit in, and —
/// with `wrap`, where the head is close enough to see — the cloth itself
/// smouldering, which a point light on the axis cannot draw: it never
/// reaches cloth that faces out.
pub fn torch_flame(pool: &mut Pool, crown: Vec3, up: Vec3, dt: f32, wrap: bool) {
    for _ in 0..count(pool, TORCH_TONGUES, dt) {
        let off = Vec3::new(pool.signed(), 0.0, pool.signed()) * 0.018;
        let life = 0.28 + 0.16 * pool.roll();
        let dy = 0.01 * pool.roll() - 0.008;
        let rise = 0.12 + 0.1 * pool.roll();
        let size = 0.032 + 0.014 * pool.roll();
        let (roll, spin) = (pool.signed() * 0.14, pool.signed() * 0.5);
        pool.spawn(Particle {
            left: life,
            life,
            pos: crown + off + Vec3::Y * dy,
            // Up, and in toward the middle: the tongues meet in a tip.
            vel: up * rise - off * 1.4,
            size0: size,
            size1: 0.014,
            c0: [1.6, 0.58, 0.11, 1.0],
            c1: [0.9, 0.16, 0.02, 0.0],
            // Buoyant: a tongue speeds up as it climbs.
            drag: -1.0,
            roll,
            spin,
            cell: atlas::FLAME,
            ..Particle::default()
        });
    }
    for _ in 0..count(pool, TORCH_CORES, dt) {
        let off = Vec3::new(pool.signed(), 0.0, pool.signed()) * 0.008;
        let life = 0.16 + 0.1 * pool.roll();
        let rise = 0.1 + 0.08 * pool.roll();
        let size = 0.02 + 0.006 * pool.roll();
        let roll = pool.signed() * 0.1;
        pool.spawn(Particle {
            left: life,
            life,
            pos: crown + off + Vec3::Y * 0.004,
            vel: up * rise,
            size0: size,
            size1: 0.01,
            c0: [2.4, 1.55, 0.62, 1.0],
            c1: [1.8, 0.7, 0.14, 0.0],
            drag: -1.0,
            roll,
            cell: atlas::FLAME,
            ..Particle::default()
        });
    }
    for _ in 0..count(pool, TORCH_GLOWS, dt) {
        let life = 0.12 + 0.08 * pool.roll();
        pool.spawn(Particle {
            left: life,
            life,
            pos: crown + Vec3::Y * 0.012,
            vel: up * 0.05,
            size0: 0.05,
            size1: 0.04,
            c0: [1.1, 0.48, 0.1, 0.6],
            c1: [0.7, 0.2, 0.03, 0.0],
            cell: atlas::GLOW,
            ..Particle::default()
        });
    }
    if !wrap {
        return;
    }
    for _ in 0..count(pool, TORCH_SMOULDER, dt) {
        let a = pool.roll() * std::f32::consts::TAU;
        let y = -0.075 * pool.roll() - 0.006;
        let life = 0.4 + 0.3 * pool.roll();
        let size = 0.007 + 0.004 * pool.roll();
        pool.spawn(Particle {
            left: life,
            life,
            pos: crown + Vec3::new(a.cos() * 0.027, y, a.sin() * 0.027),
            size0: size,
            size1: 0.005,
            c0: [0.8, 0.2, 0.03, 0.45],
            c1: [0.4, 0.06, 0.01, 0.0],
            cell: atlas::GLOW,
            ..Particle::default()
        });
    }
    for _ in 0..count(pool, TORCH_EMBERS, dt) {
        let a = pool.roll() * std::f32::consts::TAU;
        let y = -0.085 * pool.roll() - 0.004;
        let life = 0.5 + 0.5 * pool.roll();
        let size = 0.003 + 0.002 * pool.roll();
        pool.spawn(Particle {
            left: life,
            life,
            pos: crown + Vec3::new(a.cos() * 0.026, y, a.sin() * 0.026),
            vel: up * 0.01,
            size0: size,
            size1: 0.0015,
            c0: [3.2, 1.2, 0.25, 1.0],
            c1: [1.4, 0.25, 0.03, 0.0],
            cell: atlas::GLOW,
            ..Particle::default()
        });
    }
}

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
        match f.tongues {
            Tongues::Pit if f.flames => {
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
            Tongues::Torch if f.flames => {
                torch_flame(glow, at + Vec3::Y * f.flame_dy, Vec3::Y, dt, false);
            }
            _ => {}
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
            // Out of the fire, not out of the air round it: a torch's head
            // is a hand's width, a pit's logs a stride.
            let spread = 0.12 * f.scale.clamp(0.25, 1.0);
            let pos = from + Vec3::new(glow.signed() * spread, 0.0, glow.signed() * spread);
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
