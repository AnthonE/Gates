//! What a shot looks like: the muzzle flash, the tracer, and the dust and
//! mark of a miss the shard did not mark.
//!
//! The shard walks a missed shot only [`MAX_HITSCAN_MARK_SAMPLES`] (10.9 m)
//! for its mark — a decal must not own the tick — so a round into a hillside
//! at thirty metres used to land in silence: no dust, no hole. The client
//! walks the rest itself, with the sim's own ladder (`ranged::beam_stop`),
//! and draws a mark only past where the shard would have (no double mark),
//! and only when no drawn body stood in the line (a hit is `EV_HIT`, not
//! dust behind the victim). Cosmetic: it decides nothing.

use bevy::prelude::*;

use super::super::bodies::Body;
use super::super::feed::Feed;
use super::super::impact::{Contact, ContactKind, Contacts, Matter, Weapon};
use super::super::rig::EyeCam;
use super::super::{surface, Net, WorldId};
use super::pool::{Orient, Particle};
use super::table::{emit, Layer};
use super::{FlashDef, Fx};
use sim_core::limits::MAX_HITSCAN_MARK_SAMPLES;
use sim_core::occupy::{Occupants, SlotCache};
use sim_core::ranged::{beam_stop, ARROW_EYE_MM, MM_PER_M};

/// Client traces per frame — a volley is bounded by the feed, and this keeps
/// a frame's cost to a handful of walks (~0.1 ms each natively).
pub const MAX_TRACES_PER_FRAME: usize = 4;

/// Shooters further than this get no trace, metres: their dust would be a
/// few pixels, and the walk is not free.
pub const TRACE_RANGE_M: f32 = 150.0;

/// A muzzle flash's light: bright, short, warm.
pub const MUZZLE_FLASH: FlashDef = FlashDef {
    lumens: 9_000.0,
    range: 7.0,
    life: 0.07,
    color: [1.0, 0.72, 0.42],
};

/// A tracer's speed, m/s — slow enough to be seen crossing a frame.
pub const TRACER_MPS: f32 = 280.0;

/// Whether the client draws a miss the shard did not: the beam stopped on
/// something, past the shard's own mark walk, with no drawn body in its line.
pub fn cosmetic_mark(k: usize, stopped: bool, body_in_line: bool) -> bool {
    stopped && k > MAX_HITSCAN_MARK_SAMPLES && !body_in_line
}

/// Distance from `p` to the segment `a`→`b`.
pub fn seg_dist(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// Draw every shot this frame: its flash at the muzzle, its tracer, and —
/// for a hitscan miss past the shard's mark range — the contact its dust,
/// mark and sound are thrown from.
#[allow(clippy::too_many_arguments)]
pub fn shots(
    feed: Res<Feed>,
    net: Option<NonSend<Net>>,
    world: Option<Res<WorldId>>,
    cams: Query<&GlobalTransform, With<EyeCam>>,
    bodies: Query<(&Body, &GlobalTransform)>,
    mut contacts: ResMut<Contacts>,
    mut fx: ResMut<Fx>,
    mut cache: Local<Box<SlotCache>>,
) {
    if feed.shots().is_empty() {
        return;
    }
    let (Some(net), Some(world)) = (net, world) else {
        return;
    };
    let core = &net.session.core;
    let Ok(cam) = cams.single() else {
        return;
    };
    let eye_pos = cam.translation();
    let at_tick = core.render_tick();
    let mut rs = client_core::interp::RemoteState::default();
    let mut traces = 0usize;

    for &(shooter, yaw, pitch, speed_mmpt, reach) in feed.shots() {
        // Arrows fly (`tracer.rs`) and land as `EV_IMPACT`; only a firearm's
        // instantaneous shot flashes and needs a trace.
        if !protocol::shot_is_instant(speed_mmpt) {
            continue;
        }
        let own = shooter == core.player_id;
        let feet = if own {
            let p = core.predict.position();
            Vec3::new(p[0], p[1], p[2])
        } else if core.interp.sample(shooter, at_tick, &mut rs) {
            Vec3::new(rs.x, rs.y, rs.z)
        } else {
            continue;
        };
        let (fx_, fz) = sim_core::yaw_dir(yaw);
        let (ch, sv) = sim_core::pitch_dir(pitch);
        let dir = Vec3::new(fx_ * ch, sv, fz * ch).normalize_or(Vec3::Z);
        let eye_at = feet + Vec3::Y * (ARROW_EYE_MM as f32 / MM_PER_M);

        // The muzzle: your own is just ahead of the camera, down and to the
        // right where the held gun is; anyone else's at their drawn body's
        // hands, along their aim.
        let muzzle = if own {
            eye_pos + Vec3::from(cam.forward()) * 0.7 + Vec3::from(cam.right()) * 0.12
                - Vec3::from(cam.up()) * 0.1
        } else {
            let drawn = bodies
                .iter()
                .find(|(b, _)| b.0 == shooter)
                .map_or(feet, |(_, gt)| gt.translation());
            drawn + Vec3::Y * 1.42 + dir * 0.55
        };
        emit(&mut fx.glow, Layer::Flash, 2, muzzle, dir, Matter::Metal);
        emit(&mut fx.soft, Layer::GunSmoke, 2, muzzle, dir, Matter::Metal);
        fx.flash(muzzle, MUZZLE_FLASH);

        // Where the beam stops: the sim's own walk over the whole reach,
        // for shooters near enough to see.
        let range_mm = reach as u32 * 100;
        let traced = traces < MAX_TRACES_PER_FRAME && feet.distance(eye_pos) <= TRACE_RANGE_M;
        let stop = traced.then(|| {
            traces += 1;
            let mut occ = Occupants {
                table: &world.table,
                haven: &world.haven,
                harvested: &core.harvested,
                cache: &mut cache,
            };
            beam_stop(
                world.seed,
                &world.haven,
                core.pieces.cols(),
                &mut occ,
                (
                    eye_at.x * MM_PER_M,
                    eye_at.y * MM_PER_M,
                    eye_at.z * MM_PER_M,
                ),
                yaw,
                pitch,
                range_mm,
            )
        });
        let end = stop.map_or(eye_at + dir * (range_mm as f32 / MM_PER_M), |b| {
            Vec3::new(b.at_mm.0, b.at_mm.1, b.at_mm.2) / MM_PER_M
        });

        // The tracer, from a little past the muzzle (not through your own
        // gun) to where the round stopped.
        let start = muzzle + dir * if own { 1.5 } else { 0.3 };
        let dist = (end - start).dot(dir).max(0.0);
        if dist > 2.0 {
            let life = (dist / TRACER_MPS).clamp(0.03, 0.3);
            fx.glow.spawn(Particle {
                left: life,
                life,
                pos: start,
                vel: dir * TRACER_MPS,
                size0: 0.012,
                size1: 0.010,
                c0: [9.0, 6.0, 2.4, 0.9],
                c1: [5.0, 3.0, 1.0, 0.5],
                cell: super::atlas::STREAK,
                orient: Orient::Stretch,
                stretch: 0.012,
                len_max: 4.0,
                ..Particle::default()
            });
        }

        // The miss the shard did not mark.
        let Some(b) = stop else { continue };
        let body_in_line = if own {
            // Your own hit arrives as `EV_HIT` on the same frame.
            !feed.hit_victims().is_empty()
        } else {
            bodies.iter().any(|(bd, gt)| {
                bd.0 != shooter && seg_dist(gt.translation() + Vec3::Y * 1.0, eye_at, end) < 0.5
            })
        };
        let Some(surf) = b.surf else { continue };
        if !cosmetic_mark(b.k, true, body_in_line) {
            continue;
        }
        let n = surface::normal_at(&world, core.pieces.cols(), end.x, end.y, end.z, surf);
        let mut matter = surface::matter_at(&world, core, end, surf, n);
        let mut at = end;
        let mut normal = n;
        if matter == Matter::Water {
            at.y = sim_core::terrain::SEA_LEVEL;
            normal = Vec3::Y;
        }
        if matter == Matter::Flesh {
            matter = Matter::Dirt;
        }
        // Debris leaves between the surface normal and the bounce off it.
        let reflect = dir - 2.0 * dir.dot(normal) * normal;
        contacts.push(Contact {
            at,
            away: (normal + reflect * 0.6).normalize_or(normal),
            normal,
            surf,
            matter,
            kind: ContactKind::Impact,
            weapon: Weapon::Bullet,
            mark: !matches!(matter, Matter::Water | Matter::Plant),
        });
    }
}
