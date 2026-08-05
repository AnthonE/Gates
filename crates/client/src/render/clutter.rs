//! The near-ground population — `ART.md`'s single largest structural gap,
//! and the one no shader closes.
//!
//! "The ground is not a surface, it is a population": grass reads as
//! thousands of individual lit blades standing 20–40 cm, not as a textured
//! plane. The reference set's near-ground neighbour contrast is 6.3 luma per
//! pixel and the browser client's was 0.26 — a 24× gap that six visual passes
//! of shader work never moved, because the mechanism is geometry.
//!
//! Placement is `sim_core::terrain::clutter_fill`, which already exists,
//! already runs natively, and is already gated: `crates/sim-core/tests/
//! clutter.rs` measures the largest bare disc inside 15 m against rule 4's
//! bound. It has simply never been drawn by this client.
//!
//! **One mesh per tile, not one entity per element.** A tile peaks at 721
//! elements and the ring is 25 tiles; 18,000 entities would cost more in ECS
//! traversal than the triangles cost to draw. Baking each tile's elements
//! into one buffer is the same trick the browser's `ClutterField` used and it
//! keeps the whole ring at 25 draws.

use bevy::light::NotShadowCaster;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sim_core::terrain::{self, Clutter, ClutterElem, CLUTTER_PER_TILE, CLUTTER_TILE_M};

use super::props::{hash01, linear, Soup};
use super::{Eye, WorldId};

/// Tiles either side of the player's own — a 5×5 ring, 40 m to an edge.
pub const CLUTTER_RING: i32 = 2;
/// Tiles filled per frame. The fill is 721 hash draws and a few thousand
/// triangles; one a frame keeps the spike off the frame the player turns on.
pub const CLUTTER_FILLS_PER_FRAME: usize = 1;
/// A tuft's blade height at scale 1, metres — the browser's, unchanged, and
/// inside `ART.md` §1's measured 20–40 cm band.
pub const TUFT_H: f32 = 0.34;

/// Authored colour per kind, sRGB. Grass is the darkest thing on the island
/// and its shadowed side goes COOL (`ART.md` §3), which the blade ramp does
/// by lightening and warming toward the tip rather than by tinting the base.
const TUFT_LO: u32 = 0x354a2b;
const TUFT_HI: u32 = 0x778a4b;
const PEBBLE_C: u32 = 0x8a8880;
const TWIG_C: u32 = 0x5a4630;
const SHARD_C: u32 = 0x7d7a73;

#[derive(Resource, Default)]
pub struct ClutterRing {
    built: HashMap<(i32, i32), Entity>,
    material: Option<Handle<StandardMaterial>>,
}

/// Tiles in a full clutter ring.
pub const RING_TILES: usize = ((2 * CLUTTER_RING + 1) * (2 * CLUTTER_RING + 1)) as usize;

impl ClutterRing {
    pub fn len(&self) -> usize {
        self.built.len()
    }
    pub fn is_empty(&self) -> bool {
        self.built.is_empty()
    }
    pub fn is_full(&self) -> bool {
        self.built.len() >= RING_TILES
    }
}

/// One baked clutter tile.
#[derive(Component)]
pub struct Tile(pub i32, pub i32);

/// A blade: a tapered quad leaning off vertical, two triangles.
fn blade(s: &mut Soup, base: Vec3, dir: Vec2, h: f32, lean: f32, seed: u32, i: u32) {
    // Wider than the first cut's 0.022/0.004. At 2.4 elements per square
    // metre a narrow blade reads as a dark spike standing in mown lawn — the
    // first native capture's exact defect — because the eye is being shown
    // one thin silhouette rather than a mass.
    let half_base = 0.030;
    let half_tip = 0.008;
    let side = Vec3::new(-dir.y, 0.0, dir.x);
    let tip = base + Vec3::new(dir.x * lean * h, h, dir.y * lean * h);

    let b0 = base - side * half_base;
    let b1 = base + side * half_base;
    let t0 = tip - side * half_tip;
    let t1 = tip + side * half_tip;

    // The tip catches a rim of sun; the base sits in its own shade. A blade
    // that is one value is rule 1's flat surface at blade scale.
    let lo = linear(TUFT_LO);
    let hi = linear(TUFT_HI);
    let v = 0.85 + 0.3 * hash01(seed, i);
    let col = move |p: Vec3| {
        let t = ((p.y - base.y) / h).clamp(0.0, 1.0);
        [
            (lo[0] + (hi[0] - lo[0]) * t) * v,
            (lo[1] + (hi[1] - lo[1]) * t) * v,
            (lo[2] + (hi[2] - lo[2]) * t) * v,
            1.0,
        ]
    };
    // Grass scatters light as a mass, not as a set of plates. The first cut
    // blended only 0.72 of the way to vertical and left 0.28 of a FACET
    // normal in — and a blade's two triangles wind opposite ways, so one of
    // them took the sun and the other went black. Fully vertical: every blade
    // is lit by the sky above it whichever way it happens to face, which is
    // also what a real blade does once its neighbours have scattered into it.
    let up_volume = Some(base - Vec3::Y * 2.0);
    s.tri(b0, t0, b1, col, up_volume, 1.0);
    s.tri(b1, t0, t1, col, up_volume, 1.0);
}

/// Blades per tuft. Seven, not the three the first native capture shipped:
/// `terrain::clutter_fill` places ~2.4 elements per square metre, so a tuft
/// is what stands between one element and the next, and three blades of it is
/// a sprig. Seven is 5,000 blades a tile — still one draw, since the tile is
/// one baked mesh.
const BLADES_PER_TUFT: u32 = 7;

/// A tuft: a spray of blades out of one root.
fn tuft(s: &mut Soup, at: Vec3, yaw: f32, scale: f32, seed: u32) {
    let h = TUFT_H * scale;
    for i in 0..BLADES_PER_TUFT {
        let a = yaw + i as f32 * 0.897 + hash01(seed, i + 17) * 0.9;
        let dir = Vec2::new(a.sin(), a.cos());
        let lean = 0.22 + 0.34 * hash01(seed, i + 31);
        let spread = 0.03 + 0.05 * hash01(seed, i + 61);
        let root = at + Vec3::new(dir.x * spread, 0.0, dir.y * spread);
        blade(
            s,
            root,
            dir,
            h * (0.55 + 0.7 * hash01(seed, i + 47)),
            lean,
            seed,
            i,
        );
    }
}

/// A flat-ish chip: pebble, shard and twig are all one builder at different
/// proportions, which is also why none of them reads as a sphere.
fn chip(s: &mut Soup, at: Vec3, yaw: f32, size: Vec3, hex: u32, seed: u32) {
    let base = linear(hex);
    let (sy, cy) = (yaw.sin(), yaw.cos());
    let rot = |p: Vec3| Vec3::new(p.x * cy + p.z * sy, p.y, -p.x * sy + p.z * cy);
    // Four corners jittered in plan, one raised apex — an angular chip rather
    // than a box, so its silhouette is not four right angles.
    let mut c = [Vec3::ZERO; 4];
    for (i, cc) in c.iter_mut().enumerate() {
        let a = i as f32 * std::f32::consts::FRAC_PI_2 + 0.4;
        let r = 0.6 + 0.4 * hash01(seed, i as u32);
        *cc = at + rot(Vec3::new(a.cos() * size.x * r, 0.0, a.sin() * size.z * r));
    }
    let apex = at + Vec3::new(0.0, size.y, 0.0);
    let v = 0.8 + 0.4 * hash01(seed, 5);
    let col = move |_: Vec3| [base[0] * v, base[1] * v, base[2] * v, 1.0];
    for i in 0..4 {
        s.tri(c[i], apex, c[(i + 1) % 4], col, None, 0.0);
    }
}

fn element(s: &mut Soup, e: &ClutterElem) {
    let at = Vec3::new(e.x, e.y, e.z);
    let yaw = e.yaw as f32 / 256.0 * std::f32::consts::TAU;
    // The element's own cell coordinates would be a better hash key, but the
    // fill does not return them; the quantized position is stable for the
    // same reason and costs nothing.
    let seed = ((e.x * 64.0) as i32 as u32) ^ ((e.z * 64.0) as i32 as u32).rotate_left(13);
    match e.kind {
        Clutter::None => {}
        Clutter::Tuft => tuft(s, at, yaw, e.scale, seed),
        Clutter::Pebble => chip(
            s,
            at,
            yaw,
            Vec3::new(0.05, 0.035, 0.05) * e.scale,
            PEBBLE_C,
            seed,
        ),
        Clutter::Twig => chip(
            s,
            at,
            yaw,
            Vec3::new(0.16, 0.022, 0.03) * e.scale,
            TWIG_C,
            seed,
        ),
        Clutter::Shard => chip(
            s,
            at,
            yaw,
            Vec3::new(0.07, 0.06, 0.055) * e.scale,
            SHARD_C,
            seed,
        ),
    }
}

pub fn stream(
    mut commands: Commands,
    mut ring: ResMut<ClutterRing>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut buf: Local<Vec<ClutterElem>>,
    world: Res<WorldId>,
    eye: Res<Eye>,
) {
    if buf.is_empty() {
        buf.resize(CLUTTER_PER_TILE, terrain::CLUTTER_NONE);
    }
    let material = ring
        .material
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 0.92,
                reflectance: 0.12,
                // Blades are single-sided quads and a player walks all the way
                // around them.
                double_sided: true,
                cull_mode: None,
                ..default()
            })
        })
        .clone();

    let tx = (eye.pos.x / CLUTTER_TILE_M).floor() as i32;
    let tz = (eye.pos.z / CLUTTER_TILE_M).floor() as i32;

    let mut dropped = 0usize;
    ring.built.retain(|(bx, bz), e| {
        if dropped >= CLUTTER_FILLS_PER_FRAME
            || ((*bx - tx).abs() <= CLUTTER_RING && (*bz - tz).abs() <= CLUTTER_RING)
        {
            return true;
        }
        dropped += 1;
        commands.entity(*e).despawn();
        false
    });

    let mut filled = 0usize;
    for dz in -CLUTTER_RING..=CLUTTER_RING {
        for dx in -CLUTTER_RING..=CLUTTER_RING {
            if filled >= CLUTTER_FILLS_PER_FRAME {
                return;
            }
            let key = (tx + dx, tz + dz);
            if ring.built.contains_key(&key) {
                continue;
            }
            let n = terrain::clutter_fill(world.seed, key.0, key.1, &mut buf);
            let mut s = Soup::default();
            for e in buf.iter().take(n) {
                element(&mut s, e);
            }
            let e = if n == 0 {
                commands.spawn((
                    Tile(key.0, key.1),
                    Transform::IDENTITY,
                    Visibility::default(),
                ))
            } else {
                commands.spawn((
                    Tile(key.0, key.1),
                    Mesh3d(meshes.add(s.mesh())),
                    MeshMaterial3d(material.clone()),
                    // A blade is two triangles a few centimetres wide. Against
                    // a cascade sized for a 200 m world that is not a shadow,
                    // it is acne — the black wedges under every tuft in the
                    // first native capture. The ground's contact darkening
                    // comes from the blades' own dark bases instead.
                    NotShadowCaster,
                    Transform::IDENTITY,
                ))
            }
            .id();
            ring.built.insert(key, e);
            filled += 1;
        }
    }
}
