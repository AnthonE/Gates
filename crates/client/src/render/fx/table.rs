//! What a blow throws, by weapon × matter — the one table that decides
//! sparks or dust, splash or blood, the way the reference picks an impact
//! effect by hit type and physic material. Numbers live here, in code.

use bevy::prelude::*;

use super::atlas::{DROPLET, GLOW, LEAF, PUFF, RING, SMOKE, SPECK, STAR, STREAK};
use super::pool::{Orient, Particle, Pool};
use crate::render::impact::{Matter, Weapon};

/// One kind of thing an effect throws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
    /// Hot fragments, drawn as streaks along their flight; they cool from
    /// white to a red ember and bounce once off what they came from.
    Sparks,
    /// The instant flash where a round meets metal or stone.
    Flash,
    /// A soft cloud in the struck matter's colour.
    Dust,
    /// Specks of the struck matter thrown out and falling.
    Grit,
    Droplets,
    /// A ring spreading on the water.
    Ring,
    Mist,
    Leaves,
    BloodMist,
    BloodDrops,
    Smoke,
    Fireball,
}

impl Layer {
    /// Whether this layer is light (the additive pool) or matter (the
    /// alpha pool, lit like the ground).
    pub fn glow(self) -> bool {
        matches!(self, Layer::Sparks | Layer::Flash | Layer::Fireball)
    }
}

/// What one blow throws: up to five layers with a count each, and how many
/// lit chips (`impact::Chips`) fly with them.
#[derive(Clone, Copy, Debug)]
pub struct EffectDef {
    pub layers: [(Layer, u8); 5],
    pub chips: u8,
}

impl EffectDef {
    fn of(layers: &[(Layer, u8)], chips: u8) -> Self {
        let mut out = [(Layer::Dust, 0u8); 5];
        out[..layers.len()].copy_from_slice(layers);
        Self { layers: out, chips }
    }

    /// How many of `layer` this effect throws.
    pub fn count(&self, layer: Layer) -> u8 {
        self.layers
            .iter()
            .filter(|(l, _)| *l == layer)
            .map(|(_, n)| *n)
            .sum()
    }
}

/// The effect of a blow. Metal showers sparks and never dust clouds; stone
/// flashes a few sparks in a grey cloud; wood and soil never spark; water
/// splashes; a body bleeds; a bush sheds leaves.
pub fn effect(weapon: Weapon, matter: Matter) -> EffectDef {
    use Layer::*;
    use Matter::*;
    match (weapon, matter) {
        (Weapon::Blast, _) => EffectDef::of(
            &[
                (Flash, 1),
                (Fireball, 10),
                (Smoke, 8),
                (Dust, 8),
                (Sparks, 24),
            ],
            16,
        ),
        (_, Water) => EffectDef::of(&[(Droplets, 10), (Ring, 1), (Mist, 2)], 0),
        (_, Flesh) => EffectDef::of(&[(BloodMist, 3), (BloodDrops, 8)], 0),
        (_, Plant) => EffectDef::of(&[(Leaves, 7), (Dust, 1)], 0),
        (Weapon::Melee, Metal) => EffectDef::of(&[(Sparks, 18), (Dust, 1)], 3),
        (Weapon::Melee, Stone) => EffectDef::of(&[(Sparks, 6), (Dust, 3), (Grit, 4)], 8),
        (Weapon::Melee, Wood) => EffectDef::of(&[(Dust, 2)], 8),
        (Weapon::Melee, Dirt | Sand | Grass) => EffectDef::of(&[(Dust, 3), (Grit, 6)], 3),
        (Weapon::Arrow, Metal) => EffectDef::of(&[(Sparks, 5), (Dust, 1)], 1),
        (Weapon::Arrow, Stone) => EffectDef::of(&[(Sparks, 2), (Dust, 2), (Grit, 3)], 3),
        (Weapon::Arrow, Wood) => EffectDef::of(&[(Dust, 1), (Grit, 2)], 4),
        (Weapon::Arrow, Dirt | Sand | Grass) => EffectDef::of(&[(Dust, 2), (Grit, 4)], 0),
        (Weapon::Bullet, Metal) => EffectDef::of(&[(Sparks, 12), (Flash, 1), (Dust, 1)], 2),
        (Weapon::Bullet, Stone) => {
            EffectDef::of(&[(Sparks, 4), (Flash, 1), (Dust, 3), (Grit, 6)], 4)
        }
        (Weapon::Bullet, Wood) => EffectDef::of(&[(Dust, 2), (Grit, 4)], 6),
        (Weapon::Bullet, Sand) => EffectDef::of(&[(Dust, 4), (Grit, 8)], 0),
        (Weapon::Bullet, Dirt | Grass) => EffectDef::of(&[(Dust, 3), (Grit, 8)], 0),
    }
}

/// How much of an effect is thrown at `dist` metres from the eye: all of it
/// up close, less with distance, nothing past 100 m (the reference's own
/// cap on impact particles).
pub fn lod(dist: f32) -> f32 {
    if dist <= 25.0 {
        1.0
    } else if dist <= 50.0 {
        0.5
    } else if dist <= 100.0 {
        0.25
    } else {
        0.0
    }
}

/// `count` scaled by `lod`, never rounding a non-empty layer to nothing
/// while it is in range at all.
pub fn scaled(count: u8, lod: f32) -> usize {
    if count == 0 || lod <= 0.0 {
        0
    } else {
        ((count as f32 * lod).ceil() as usize).max(1)
    }
}

/// A dust cloud's colour per matter, sRGB.
pub fn dust_tint(m: Matter) -> Color {
    match m {
        Matter::Wood => Color::srgb(0.66, 0.56, 0.40),
        Matter::Stone => Color::srgb(0.62, 0.60, 0.56),
        Matter::Metal => Color::srgb(0.52, 0.49, 0.44),
        Matter::Flesh => Color::srgb(0.40, 0.05, 0.05),
        Matter::Plant => Color::srgb(0.52, 0.56, 0.36),
        Matter::Dirt => Color::srgb(0.50, 0.42, 0.30),
        Matter::Sand => Color::srgb(0.80, 0.73, 0.58),
        Matter::Grass => Color::srgb(0.50, 0.48, 0.34),
        Matter::Water => Color::srgb(0.86, 0.90, 0.94),
    }
}

/// One layer's recipe.
struct Spec {
    orient: Orient,
    cell: u8,
    cells: u8,
    speed: (f32, f32),
    /// Share of the launch along the thrown direction rather than scattered.
    spray: f32,
    /// Extra upward speed, m/s.
    up: f32,
    gravity: f32,
    drag: f32,
    life: (f32, f32),
    size: (f32, f32),
    /// End size over start size.
    grow: f32,
    c0: [f32; 4],
    c1: [f32; 4],
    /// Multiply the colours by the matter's dust tint.
    tinted: bool,
    spin: f32,
    stretch: f32,
    bounce: bool,
}

const WATER: [f32; 3] = [0.72, 0.8, 0.86];

fn spec(layer: Layer) -> Spec {
    let base = Spec {
        orient: Orient::Billboard,
        cell: PUFF,
        cells: 1,
        speed: (0.0, 0.0),
        spray: 0.5,
        up: 0.0,
        gravity: 0.0,
        drag: 0.0,
        life: (0.5, 0.5),
        size: (0.05, 0.05),
        grow: 1.0,
        c0: [1.0; 4],
        c1: [1.0, 1.0, 1.0, 0.0],
        tinted: false,
        spin: 0.0,
        stretch: 0.0,
        bounce: false,
    };
    match layer {
        Layer::Sparks => Spec {
            orient: Orient::Stretch,
            cell: STREAK,
            speed: (3.0, 9.0),
            spray: 0.65,
            up: 0.8,
            gravity: 9.81,
            drag: 2.4,
            life: (0.16, 0.42),
            size: (0.006, 0.008),
            grow: 0.5,
            c0: [22.0, 13.0, 5.0, 1.0],
            c1: [2.2, 0.35, 0.05, 0.0],
            stretch: 0.016,
            bounce: true,
            ..base
        },
        Layer::Flash => Spec {
            cell: STAR,
            cells: 3,
            life: (0.05, 0.08),
            size: (0.06, 0.1),
            grow: 1.4,
            c0: [7.0, 5.5, 3.0, 1.0],
            c1: [1.5, 0.8, 0.3, 0.0],
            ..base
        },
        Layer::Dust => Spec {
            cells: 3,
            speed: (0.4, 1.6),
            spray: 0.55,
            up: 0.2,
            gravity: -0.25,
            drag: 2.8,
            life: (0.8, 1.4),
            size: (0.05, 0.08),
            grow: 6.0,
            c0: [0.95, 0.95, 0.95, 0.55],
            c1: [1.05, 1.05, 1.05, 0.0],
            tinted: true,
            spin: 0.8,
            ..base
        },
        Layer::Grit => Spec {
            cell: SPECK,
            speed: (2.0, 4.5),
            spray: 0.6,
            up: 1.0,
            gravity: 9.81,
            drag: 0.6,
            life: (0.35, 0.7),
            size: (0.010, 0.014),
            grow: 0.8,
            c0: [0.55, 0.55, 0.55, 1.0],
            c1: [0.55, 0.55, 0.55, 0.7],
            tinted: true,
            bounce: true,
            ..base
        },
        Layer::Droplets => Spec {
            cell: DROPLET,
            speed: (1.5, 4.0),
            spray: 0.85,
            up: 1.5,
            gravity: 9.81,
            drag: 0.4,
            life: (0.45, 0.8),
            size: (0.02, 0.03),
            grow: 0.8,
            c0: [WATER[0], WATER[1], WATER[2], 0.85],
            c1: [WATER[0], WATER[1], WATER[2], 0.3],
            ..base
        },
        Layer::Ring => Spec {
            orient: Orient::Flat,
            cell: RING,
            life: (0.55, 0.7),
            size: (0.05, 0.06),
            grow: 10.0,
            c0: [WATER[0], WATER[1], WATER[2], 0.55],
            c1: [WATER[0], WATER[1], WATER[2], 0.0],
            ..base
        },
        Layer::Mist => Spec {
            cells: 3,
            speed: (0.3, 0.9),
            spray: 0.9,
            gravity: -0.1,
            drag: 2.0,
            life: (0.5, 0.8),
            size: (0.08, 0.1),
            grow: 5.0,
            c0: [WATER[0], WATER[1], WATER[2], 0.35],
            c1: [WATER[0], WATER[1], WATER[2], 0.0],
            spin: 0.5,
            ..base
        },
        Layer::Leaves => Spec {
            cell: LEAF,
            speed: (1.0, 2.5),
            spray: 0.4,
            up: 1.0,
            gravity: 2.2,
            drag: 2.2,
            life: (0.9, 1.6),
            size: (0.03, 0.04),
            c0: [0.10, 0.20, 0.04, 1.0],
            c1: [0.10, 0.18, 0.04, 0.0],
            spin: 7.0,
            ..base
        },
        Layer::BloodMist => Spec {
            cells: 3,
            speed: (0.5, 1.8),
            spray: 0.6,
            gravity: 0.6,
            drag: 4.0,
            life: (0.3, 0.5),
            size: (0.04, 0.06),
            grow: 5.0,
            c0: [0.20, 0.004, 0.004, 0.85],
            c1: [0.24, 0.01, 0.01, 0.0],
            spin: 1.0,
            ..base
        },
        Layer::BloodDrops => Spec {
            cell: DROPLET,
            speed: (1.5, 3.5),
            spray: 0.5,
            up: 0.8,
            gravity: 9.81,
            drag: 0.5,
            life: (0.35, 0.7),
            size: (0.010, 0.014),
            grow: 0.8,
            c0: [0.16, 0.002, 0.002, 1.0],
            c1: [0.16, 0.002, 0.002, 0.6],
            ..base
        },
        Layer::Smoke => Spec {
            cell: SMOKE,
            cells: 2,
            speed: (0.3, 1.0),
            spray: 0.8,
            gravity: -0.6,
            drag: 1.2,
            life: (2.5, 4.5),
            size: (0.3, 0.45),
            grow: 4.0,
            c0: [0.25, 0.24, 0.23, 0.55],
            c1: [0.35, 0.35, 0.35, 0.0],
            spin: 0.3,
            ..base
        },
        Layer::Fireball => Spec {
            cell: GLOW,
            speed: (1.0, 4.0),
            spray: 0.3,
            gravity: -1.0,
            drag: 3.0,
            life: (0.25, 0.5),
            size: (0.3, 0.45),
            grow: 3.0,
            c0: [14.0, 6.0, 1.5, 1.0],
            c1: [1.2, 0.2, 0.02, 0.0],
            spin: 0.5,
            ..base
        },
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Throw `n` particles of `layer` from `at`, around `dir`, coloured for
/// `matter`. Sparks and grit bounce once off the plane through `at` facing
/// `dir` — the surface they came off.
pub fn emit(pool: &mut Pool, layer: Layer, n: usize, at: Vec3, dir: Vec3, matter: Matter) {
    let s = spec(layer);
    let tint = if s.tinted {
        dust_tint(matter).to_linear().to_f32_array_no_alpha()
    } else {
        [1.0; 3]
    };
    let paint = |c: [f32; 4]| [c[0] * tint[0], c[1] * tint[1], c[2] * tint[2], c[3]];
    let dir = dir.normalize_or(Vec3::Y);
    for _ in 0..n {
        let scatter = Vec3::new(pool.signed(), pool.signed(), pool.signed()).normalize_or(Vec3::Y);
        let v = (dir * s.spray + scatter * (1.0 - s.spray)).normalize_or(dir);
        let speed = lerp(s.speed.0, s.speed.1, pool.roll());
        let life = lerp(s.life.0, s.life.1, pool.roll());
        let size = lerp(s.size.0, s.size.1, pool.roll());
        let cell = s.cell + ((pool.roll() * s.cells as f32) as u8).min(s.cells - 1);
        let roll = pool.roll() * std::f32::consts::TAU;
        let spin = pool.signed() * s.spin;
        pool.spawn(Particle {
            left: life,
            life,
            // A couple of centimetres off the surface, so nothing is born
            // inside it.
            pos: at + v * 0.02,
            vel: v * speed + Vec3::Y * s.up,
            size0: size,
            size1: size * s.grow,
            c0: paint(s.c0),
            c1: paint(s.c1),
            gravity: s.gravity,
            drag: s.drag,
            roll,
            spin,
            cell,
            orient: s.orient,
            stretch: s.stretch,
            plane_p: at,
            plane_n: if s.bounce { dir } else { Vec3::ZERO },
        });
    }
}
