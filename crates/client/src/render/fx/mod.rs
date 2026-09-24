//! Particles: every spark, puff, fleck, droplet and leaf an effect throws.
//!
//! Two pools drawn as two meshes — two draw calls for the whole island, the
//! same on WebGL2 and on the desktop:
//!
//! - **glow** ([`GLOW_POOL`]): unlit, additive, HDR vertex colour. Sparks,
//!   flashes, fire. A spark's sprite carries its own soft halo and a streak
//!   is never drawn under ~1.5 px wide, so it reads as light with bloom off —
//!   which is the browser's default.
//! - **soft** ([`SOFT_POOL`]): lit like the ground under it, alpha-blended,
//!   sorted back to front. Dust, grit, smoke, splash, blood, leaves.
//!
//! Both meshes are written in place every frame something is alive and are
//! anchored at the eye, so the renderer's transparent sort draws them after
//! the marks (`decal.rs`) and the additive one after the soft one.
//!
//! What a blow throws is [`table::effect`]'s, by weapon × matter; this module
//! only runs it. It decides nothing — every burst comes from an
//! `impact::Contact` the sim already announced.

pub mod atlas;
pub mod pool;
pub mod table;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use super::impact::{Contact, Weapon};
use super::rig::EyeCam;
use pool::{pool_mesh, Cam, Pool};

/// Additive particles drawable at once. A spark lives under half a second, so
/// this is dozens of simultaneous metal hits.
pub const GLOW_POOL: usize = 512;
/// Alpha particles drawable at once — dust lives longer and is larger, and
/// every one of these is sorted each frame.
pub const SOFT_POOL: usize = 256;

/// Light flashes that can burn at once — a blast, a muzzle. Pooled point
/// lights, hidden when idle and never despawned.
pub const FLASH_LIGHTS: usize = 4;

/// A light flash: peak intensity (lumens), range (metres), life (seconds),
/// linear colour.
#[derive(Clone, Copy, Debug)]
pub struct FlashDef {
    pub lumens: f32,
    pub range: f32,
    pub life: f32,
    pub color: [f32; 3],
}

/// A satchel going off: a hard orange light over the whole base.
pub const BLAST_FLASH: FlashDef = FlashDef {
    lumens: 400_000.0,
    range: 30.0,
    life: 0.45,
    color: [1.0, 0.62, 0.3],
};

/// One burning flash.
#[derive(Clone, Copy, Debug, Default)]
pub struct Flash {
    pub at: Vec3,
    pub left: f32,
    pub life: f32,
    pub lumens: f32,
    pub range: f32,
    pub color: [f32; 3],
}

/// The two pools and the meshes they draw into, and the flash lights.
#[derive(Resource)]
pub struct Fx {
    pub glow: Pool,
    pub soft: Pool,
    pub flashes: [Flash; FLASH_LIGHTS],
    glow_mesh: Handle<Mesh>,
    soft_mesh: Handle<Mesh>,
}

impl Default for Fx {
    fn default() -> Self {
        Self {
            glow: Pool::new(GLOW_POOL, false),
            soft: Pool::new(SOFT_POOL, true),
            flashes: [Flash::default(); FLASH_LIGHTS],
            glow_mesh: Handle::default(),
            soft_mesh: Handle::default(),
        }
    }
}

impl Fx {
    /// Throw what contact `c` throws, thinned by its distance from `eye`.
    /// Returns how many lit chips go with it (`impact::Chips` owns those).
    pub fn impact(&mut self, c: &Contact, eye: Vec3) -> usize {
        let def = table::effect(c.weapon, c.matter);
        let dist = c.at.distance(eye);
        let lod = if c.weapon == Weapon::Blast {
            self.flash(c.at + Vec3::Y * 0.5, BLAST_FLASH);
            table::lod_far(dist)
        } else {
            table::lod(dist)
        };
        for (layer, n) in def.layers {
            let n = table::scaled(n, lod);
            if n == 0 {
                continue;
            }
            let pool = if layer.glow() {
                &mut self.glow
            } else {
                &mut self.soft
            };
            table::emit(pool, layer, n, c.at, c.away, c.matter);
        }
        table::scaled(def.chips, lod)
    }

    /// Light a flash at `at` — the free light, or the one closest to out.
    pub fn flash(&mut self, at: Vec3, def: FlashDef) {
        let i = self
            .flashes
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.left.total_cmp(&b.1.left))
            .map_or(0, |(i, _)| i);
        self.flashes[i] = Flash {
            at,
            left: def.life,
            life: def.life,
            lumens: def.lumens,
            range: def.range,
            color: def.color,
        };
    }

    /// Forget every particle — the world they were thrown in has gone.
    pub fn clear(&mut self) {
        self.glow.clear();
        self.soft.clear();
        self.flashes = [Flash::default(); FLASH_LIGHTS];
    }
}

/// A flash light's entity; `0` is its index into [`Fx::flashes`].
#[derive(Component)]
pub struct FlashLight(pub usize);

/// One of the two particle meshes.
#[derive(Component)]
pub struct FxMesh {
    glow: bool,
}

/// Build both meshes and their materials over the generated sprite sheet.
/// The entities are always drawn — collapsed quads until something flies —
/// so their pipelines compile at load and not on the first spark.
pub fn setup(
    mut commands: Commands,
    mut fx: ResMut<Fx>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
) {
    let sheet = images.add(atlas::sprite_image());
    fx.glow_mesh = meshes.add(pool_mesh(GLOW_POOL));
    fx.soft_mesh = meshes.add(pool_mesh(SOFT_POOL));
    let glow = standard.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(sheet.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let soft = standard.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(sheet),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    for i in 0..FLASH_LIGHTS {
        commands.spawn((
            FlashLight(i),
            PointLight {
                intensity: 0.0,
                range: 1.0,
                shadows_enabled: false,
                ..default()
            },
            Transform::default(),
            Visibility::Hidden,
        ));
    }
    for (is_glow, mesh, mat) in [
        (false, fx.soft_mesh.clone(), soft),
        (true, fx.glow_mesh.clone(), glow),
    ] {
        commands.spawn((
            FxMesh { glow: is_glow },
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            Transform::IDENTITY,
            Visibility::Visible,
            NoFrustumCulling,
            NotShadowCaster,
        ));
    }
}

/// Advance both pools and copy them into their meshes, after the camera has
/// its final transform for the frame (a billboard faces the camera the frame
/// sees, not the one before it).
pub fn draw(
    time: Res<Time>,
    mut fx: ResMut<Fx>,
    cams: Query<&GlobalTransform, (With<EyeCam>, Without<FxMesh>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut q: Query<(&FxMesh, &mut Transform, &mut GlobalTransform)>,
) {
    // A hitch must not fling everything a hundred metres.
    let dt = time.delta_secs().min(0.1);
    fx.glow.step(dt);
    fx.soft.step(dt);
    let Ok(cam_gt) = cams.single() else {
        return;
    };
    let cam = Cam {
        pos: cam_gt.translation(),
        right: cam_gt.right().into(),
        up: cam_gt.up().into(),
    };
    let ahead: Vec3 = cam_gt.forward().into();
    let Fx {
        glow,
        soft,
        glow_mesh,
        soft_mesh,
        ..
    } = &mut *fx;
    for (m, mut tf, mut gt) in &mut q {
        // The additive mesh sorts at the eye, the soft one a metre ahead of
        // it: transparent draws go far to near, so sparks land over dust and
        // both over the marks.
        let (pool, handle, anchor) = if m.glow {
            (&mut *glow, &*glow_mesh, cam.pos)
        } else {
            (&mut *soft, &*soft_mesh, cam.pos + ahead)
        };
        if tf.translation != anchor {
            tf.translation = anchor;
            *gt = GlobalTransform::from_translation(anchor);
        }
        if !pool.needs_write() {
            continue;
        }
        if let Some(mesh) = meshes.get_mut(handle) {
            pool.write(mesh, &cam, anchor);
        }
    }
}

/// Retire every particle on the way out of a world.
pub fn forget(mut fx: ResMut<Fx>) {
    fx.clear();
}

/// Burn the flashes down: a hard attack and a quadratic fall, hidden when out.
pub fn flash(
    time: Res<Time>,
    mut fx: ResMut<Fx>,
    mut q: Query<(
        &FlashLight,
        &mut PointLight,
        &mut Transform,
        &mut Visibility,
    )>,
) {
    let dt = time.delta_secs().min(0.1);
    for (FlashLight(i), mut light, mut tf, mut vis) in &mut q {
        let Some(f) = fx.flashes.get_mut(*i) else {
            continue;
        };
        if f.left <= 0.0 {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        }
        f.left = (f.left - dt).max(0.0);
        let k = (f.left / f.life.max(1e-4)).powi(2);
        light.intensity = f.lumens * k;
        light.range = f.range;
        light.color = Color::linear_rgb(f.color[0], f.color[1], f.color[2]);
        tf.translation = f.at;
        *vis = Visibility::Visible;
    }
}
