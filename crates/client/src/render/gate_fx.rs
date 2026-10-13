//! THE GATE, alive. The dressing (`town.rs`) is a still life — a broken gilt
//! ring between two obsidian pylons — and this is what the ring still does.
//! `WORLD.md` §3 names it: *debris still hanging*.
//!
//! - **The fallen arc hangs where it broke**: three curved fragments of the
//!   ring's lower arc and a scatter of gilt and obsidian chips, swaying and
//!   turning in the gap they fell out of.
//! - **Power crawls across the breaks**: arcs jump between the ring's broken
//!   ends and the fragments, a few times a second; every so often the whole
//!   chain lights at once and strikes the dais (a *surge*), and the seams,
//!   the dais lights and a flash go with it.
//! - **The lapis breathes**: the seams' glow swells and settles on a slow
//!   breath, burns brighter after dark, and spikes on a surge.
//! - **Motes**: lapis sparks rise off the dais and are drawn round and into
//!   the ring, a slow vortex that never opens anything; gilt glints in the sun.
//! - **Beacons**: after dusk a shaft of pale light stands off each gilt cap,
//!   so the town is found from anywhere on the island.
//!
//! Drawn and never decided: nothing collides, nothing reaches the sim, and
//! the clock is the client's own. One extra draw call carries every mote and
//! arc (the mesh is the fx pools' shape, anchored at the eye).

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::ImageSampler;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::f32::consts::TAU;

use super::depot::Surface;
use super::fx::atlas::{self, SPRITE_COLS, SPRITE_ROWS, SPRITE_TEX};
use super::fx::pool::{pool_mesh, Cam, Orient, Particle, Pool, MIN_PX};
use super::fx::Fx;
use super::rig::{EyeCam, FlameGain};
use super::town::{self, TownGlow};
use super::{Eye, WorldEntity, WorldId};

// ------------------------------------------------------------- the ring ---

/// The ring's circle, kit-local: `ci/site_kit.py` fits it to `town::PARTS`'
/// gilt decor — centre (0, 18) in the XY plane, radius 7.6.
const RING_C: Vec3 = Vec3::new(0.0, 18.0, 0.0);
const RING_R: f32 = 7.6;
/// Half the ring's squared section (the dresser's `half`).
const RING_HALF: f32 = 0.6;
/// Where the standing arc ends, degrees round the circle: the dresser's run
/// of decor boxes from −30° to 210°, padded 12° either side.
const BREAK_LO: f32 = 222.0;
const BREAK_HI: f32 = 318.0;
/// The standing arc, for glints: from −42° round through the top to 222°.
const STAND_FROM: f32 = -42.0;
const STAND_TO: f32 = 222.0;

/// A point on the ring's circle at `deg` degrees, radius `r`, depth `z`,
/// relative to [`RING_C`].
fn on_ring(deg: f32, r: f32, z: f32) -> Vec3 {
    let (s, c) = deg.to_radians().sin_cos();
    Vec3::new(r * c, r * s, z)
}

/// One hanging fragment of the fallen arc: the degrees of ring it was, where
/// it hangs from where it broke, how it is tilted, and how it sways.
struct Frag {
    a0: f32,
    a1: f32,
    drift: Vec3,
    tilt: Vec3,
    bob: f32,
    hz: f32,
    phase: f32,
}

/// The lower arc in three pieces, each fallen a little out of line: the
/// gaps between them are what the arcs jump.
const FRAGS: [Frag; 3] = [
    Frag {
        a0: 226.0,
        a1: 250.0,
        drift: Vec3::new(-0.5, -0.9, 0.6),
        tilt: Vec3::new(0.12, 0.22, 0.16),
        bob: 0.22,
        hz: 0.13,
        phase: 0.0,
    },
    Frag {
        a0: 255.0,
        a1: 285.0,
        drift: Vec3::new(0.0, -1.9, -0.45),
        tilt: Vec3::new(-0.14, -0.08, -0.07),
        bob: 0.3,
        hz: 0.1,
        phase: 2.1,
    },
    Frag {
        a0: 290.0,
        a1: 314.0,
        drift: Vec3::new(0.6, -1.1, 0.5),
        tilt: Vec3::new(0.16, -0.26, -0.2),
        bob: 0.2,
        hz: 0.15,
        phase: 4.0,
    },
];

/// Gilt and obsidian chips hanging in the break, beside the fragments.
const CHIPS: usize = 14;

/// What a hanging piece is and how it moves. `home` is kit-local.
#[derive(Component, Clone, Copy)]
pub struct Hanging {
    home: Vec3,
    base: Quat,
    /// Sway: amplitude (m), frequency (Hz), phase.
    bob: f32,
    hz: f32,
    phase: f32,
    /// A chip turns about its own axis; a fragment rocks on it.
    axis: Vec3,
    spin: f32,
    rock: bool,
    /// The fragment's ends in its own frame (arc anchors), or zero.
    ends: [Vec3; 2],
    /// Which fragment, or `u8::MAX` for a chip.
    frag: u8,
}

// -------------------------------------------------------- the discharge ---

/// Points per bolt (16 segments by midpoint displacement).
const BOLT_PTS: usize = 17;
/// Bolts burning at once: a surge lights the whole chain, its branches and
/// two ground strikes, with room for the crackle around it.
const BOLTS: usize = 22;
/// Two quads per segment: the core and the glow round it.
const BOLT_QUADS: usize = BOLTS * (BOLT_PTS - 1) * 2;
/// Motes alive at once.
const MOTES: usize = 260;
/// Everything the gate's one mesh can draw.
const QUADS: usize = BOLT_QUADS + MOTES;

#[derive(Clone, Copy)]
struct Bolt {
    pts: [Vec3; BOLT_PTS],
    /// Points used (a branch is shorter than a bolt).
    n: usize,
    left: f32,
    life: f32,
    width: f32,
}

impl Default for Bolt {
    fn default() -> Self {
        Self {
            pts: [Vec3::ZERO; BOLT_PTS],
            n: 0,
            left: 0.0,
            life: 1.0,
            width: 0.0,
        }
    }
}

/// A discharge across one gap: it strikes, then re-strikes a few times on a
/// fresh path, which is what makes a spark read as electric.
#[derive(Clone, Copy, Default)]
struct Discharge {
    /// Which gap (`0..GAPS`), or a ground strike (`GAPS`, `GAPS + 1`).
    gap: u8,
    strikes: u8,
    next: f32,
    width: f32,
}

/// The ring's two broken ends and the fragments between them: four gaps.
const GAPS: usize = FRAGS.len() + 1;
const DISCHARGES: usize = 10;

/// Seconds between crackles across one gap, and between surges.
const CRACKLE_S: (f32, f32) = (0.2, 1.0);
const SURGE_S: (f32, f32) = (16.0, 30.0);
/// How long a surge's glow takes to settle, seconds.
const SURGE_FALL_S: f32 = 1.6;

/// The arcs' core and glow, linear HDR: lapis-white.
const ARC_CORE: [f32; 3] = [2.6, 4.2, 9.0];
const ARC_GLOW: [f32; 3] = [0.25, 0.6, 1.8];
/// The flash an arc lights, lumens: a crackle and a surge, at the night
/// eye's gain, never under the surge's daylight floor.
const ARC_LUMENS: f32 = 700.0;
const SURGE_LUMENS: f32 = 9_000.0;
const SURGE_DAY_LUMENS: f32 = 160_000.0;
const ARC_COLOR: Color = Color::srgb(0.55, 0.75, 1.0);

// --------------------------------------------------------------- motes ---

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Kind {
    /// Drawn round the ring's inside and in toward its middle.
    #[default]
    Vortex,
    /// Off the dais and the fallen blocks, rising and wandering.
    Rise,
    /// A sun glint on the gilt.
    Glint,
    /// A surge's burst out of the ring.
    Burst,
    /// Cold light pooling on the dais after dark.
    Mist,
}

#[derive(Clone, Copy, Default)]
struct Mote {
    left: f32,
    life: f32,
    kind: Kind,
    /// Vortex: angle (rad), radius, depth, angular speed, inward speed.
    theta: f32,
    r: f32,
    z: f32,
    omega: f32,
    vr: f32,
    /// Rise / Glint / Burst: kit-local position and velocity.
    pos: Vec3,
    vel: Vec3,
    size: f32,
    phase: f32,
}

/// Motes a second, by kind. **(knob)**
const VORTEX_RATE: f32 = 11.0;
const RISE_RATE: f32 = 9.0;
const GLINT_RATE: f32 = 5.0;
const MIST_RATE: f32 = 3.0;
/// The lapis of a mote, linear HDR.
const MOTE_LAPIS: [f32; 3] = [0.35, 0.85, 2.6];
const MOTE_GILT: [f32; 3] = [3.2, 2.4, 1.1];
const MIST_LAPIS: [f32; 3] = [0.25, 0.5, 1.4];

/// Motes and arcs are drawn within this of the town's centre, metres; past
/// it a mote is under a pixel and an arc a flicker nobody can place.
const NEAR_M: f32 = 260.0;
const ARC_FAR_M: f32 = 600.0;

// -------------------------------------------------------------- beacons ---

/// A shaft of light standing off a gilt cap at night: its height, width at
/// the cap and at the top, metres. **(knob)**
const BEACON_LEN_M: f32 = 320.0;
const BEACON_W0: f32 = 2.4;
const BEACON_W1: f32 = 15.0;
/// Its colour at full night, linear: pale lapis.
const BEACON_RGB: [f32; 3] = [0.26, 0.46, 1.0];
/// The halo on the cap.
const HALO_M: f32 = 3.4;
/// The caps' tops, kit-local (`town::PARTS`' gilt caps).
const CAPS: [Vec3; 2] = [Vec3::new(-9.5, 31.05, 0.0), Vec3::new(9.5, 31.05, 0.0)];

/// The dais lights: lapis pools at the pylons' feet. Lumens at full night.
const DAIS_LUMENS: f32 = 1_100.0;
const DAIS_RANGE_M: f32 = 15.0;
const DAIS: [Vec3; 2] = [Vec3::new(-9.5, 2.2, 0.0), Vec3::new(9.5, 2.2, 0.0)];
const DAIS_COLOR: Color = Color::srgb(0.35, 0.6, 1.0);
/// The glow in the break, under the ring's middle, and its reach.
const GAP_AT: Vec3 = Vec3::new(0.0, 11.0, 0.0);
const GAP_LUMENS: f32 = 600.0;
const GAP_RANGE_M: f32 = 9.0;

/// The seams' glow by day and at full night (linear), and what a surge adds.
const LAPIS_DAY: LinearRgba = LinearRgba::rgb(0.25, 0.55, 1.6);
const LAPIS_NIGHT: LinearRgba = LinearRgba::rgb(1.0, 2.3, 6.5);
const LAPIS_SURGE: LinearRgba = LinearRgba::rgb(2.0, 4.0, 10.0);
/// One breath of the seams, seconds, and how deep it goes.
const BREATH_S: f32 = 5.5;
const BREATH_DEPTH: f32 = 0.35;

// ----------------------------------------------------------- the state ---

/// The gate's moving parts.
#[derive(Resource)]
pub struct GateFx {
    tf: Transform,
    mesh: Handle<Mesh>,
    motes: Vec<Mote>,
    bolts: Vec<Bolt>,
    discharges: [Discharge; DISCHARGES],
    /// The gaps' ends this frame, kit-local: [lo, hi] for each.
    gaps: [[Vec3; 2]; GAPS],
    rng: u32,
    clock: f32,
    next_crackle: f32,
    next_surge: f32,
    /// The surge's glow, 1 at the strike, settling to 0.
    surge: f32,
    flash_left: f32,
    flash_life: f32,
    flash_lumens: f32,
    flash_at: Vec3,
    /// The seams' emissive last written.
    lapis_at: Option<LinearRgba>,
    /// Quads the last write filled, so the next clears only what it used.
    drawn: usize,
    /// Whether the eye is near enough for motes and arcs.
    near: bool,
    arcs_seen: bool,
    pub beacon_mat: Handle<StandardMaterial>,
    /// The beam profile, shared with `town_fx`'s searchlights.
    pub beam_tex: Handle<Image>,
}

impl GateFx {
    fn roll(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1 << 24) as f32
    }

    fn signed(&mut self) -> f32 {
        self.roll() * 2.0 - 1.0
    }

    fn range(&mut self, r: (f32, f32)) -> f32 {
        r.0 + (r.1 - r.0) * self.roll()
    }

    /// How many of a `rate`-per-second stream fall in `dt`.
    fn count(&mut self, rate: f32, dt: f32) -> usize {
        let x = rate * dt;
        let whole = x.floor();
        whole as usize + usize::from(self.roll() < x - whole)
    }

    fn mote(&mut self, m: Mote) {
        if let Some(slot) = self.motes.iter_mut().find(|s| s.left <= 0.0) {
            *slot = m;
        }
    }

    /// A bolt from `a` to `b` (kit-local), jagged by `jag` of its length.
    fn bolt(&mut self, a: Vec3, b: Vec3, jag: f32, width: f32, life: f32, n: usize) -> usize {
        let Some(i) = self.bolts.iter().position(|s| s.left <= 0.0) else {
            return usize::MAX;
        };
        let n = n.clamp(3, BOLT_PTS);
        let mut pts = [Vec3::ZERO; BOLT_PTS];
        let last = n - 1;
        for (k, p) in pts.iter_mut().enumerate().take(n) {
            *p = a.lerp(b, k as f32 / last as f32);
        }
        // Midpoint displacement, coarse to fine, perpendicular to the bolt.
        let dir = (b - a).normalize_or(Vec3::Y);
        let mut step = last.next_power_of_two() / 2;
        let mut amp = (b - a).length() * jag;
        while step >= 1 {
            let mut k = step;
            while k < last {
                let r = Vec3::new(self.signed(), self.signed(), self.signed());
                let off = (r - dir * r.dot(dir)) * amp;
                pts[k] += off;
                k += step * 2;
            }
            step /= 2;
            amp *= 0.55;
        }
        self.bolts[i] = Bolt {
            pts,
            n,
            left: life,
            life,
            width,
        };
        i
    }

    /// One strike across `gap`: a bolt, maybe a branch, sparks off its ends.
    fn strike(&mut self, gap: usize, width: f32, sparks: Option<&mut Pool>) {
        let (a, b) = match gap {
            g if g < GAPS => (self.gaps[g][0], self.gaps[g][1]),
            // A ground strike: off a broken end, down to the dais under it.
            g => {
                let lo = g == GAPS;
                let from = if lo {
                    self.gaps[0][0]
                } else {
                    self.gaps[GAPS - 1][1]
                };
                let x = if lo { -7.5 } else { 7.5 } + self.signed() * 1.5;
                (from, Vec3::new(x, 0.5, self.signed() * 3.0))
            }
        };
        let life = self.range((0.09, 0.2));
        let i = self.bolt(a, b, 0.22, width, life, BOLT_PTS);
        if i == usize::MAX {
            return;
        }
        if self.roll() < 0.45 {
            let k = 5 + (self.roll() * 7.0) as usize;
            let from = self.bolts[i].pts[k];
            let len = (b - a).length() * self.range((0.25, 0.45));
            let away = Vec3::new(self.signed(), self.signed() - 0.4, self.signed())
                .normalize_or(Vec3::NEG_Y);
            self.bolt(from, from + away * len, 0.3, width * 0.6, life * 0.8, 9);
        }
        let mid = a.lerp(b, 0.5);
        self.flash_at = mid;
        if self.flash_left <= 0.0 || self.flash_lumens <= ARC_LUMENS {
            self.flash_left = 0.09;
            self.flash_life = 0.09;
            self.flash_lumens = ARC_LUMENS;
        }
        if let Some(pool) = sparks {
            let tf = self.tf;
            let n = 2 + (self.roll() * 4.0) as usize;
            for _ in 0..n {
                let at = if self.roll() < 0.5 { a } else { b };
                let v = Vec3::new(self.signed(), self.signed() * 0.6 + 0.3, self.signed())
                    .normalize_or(Vec3::Y)
                    * self.range((1.5, 4.0));
                let life = self.range((0.35, 0.8));
                pool.spawn(Particle {
                    left: life,
                    life,
                    pos: tf.transform_point(at),
                    vel: tf.rotation * v,
                    size0: 0.012,
                    size1: 0.006,
                    c0: [4.0, 5.5, 9.0, 1.0],
                    c1: [0.6, 1.0, 2.4, 0.0],
                    gravity: 9.8,
                    drag: 0.6,
                    cell: atlas::GLOW,
                    orient: Orient::Stretch,
                    stretch: 0.04,
                    ..Particle::default()
                });
            }
        }
    }

    fn discharge(&mut self, gap: u8, strikes: u8, width: f32) {
        if let Some(d) = self.discharges.iter_mut().find(|d| d.strikes == 0) {
            *d = Discharge {
                gap,
                strikes,
                next: 0.0,
                width,
            };
        }
    }
}

/// The gate's root, so it is built once per world.
#[derive(Component)]
pub struct GateRoot;

/// The one mesh every mote and arc is written into.
#[derive(Component)]
pub struct GateMesh;

/// The arcs' flash.
#[derive(Component)]
pub struct ArcLight;

/// A lapis light that breathes with the seams: the pools at the pylons'
/// feet, and the glow in the break that lights the hanging fragments. Its
/// lumens at full night.
#[derive(Component)]
pub struct DaisLight(f32);

/// A shaft of light: drawn from `from` along `dir` (world), turned about its
/// own axis to face the eye. `town_fx`'s searchlights are beams too.
#[derive(Component, Clone, Copy)]
pub struct Beam {
    pub from: Vec3,
    pub dir: Vec3,
}

/// A billboard that faces the eye square on (a beacon's halo).
#[derive(Component)]
pub struct Halo;

/// A beacon's shaft or halo, shown only after dusk.
#[derive(Component)]
pub struct Beacon;

// ------------------------------------------------------------- meshing ---

/// A flat-shaded triangle soup with box-projected metre UVs — the dresser's
/// faces, built here.
#[derive(Default)]
pub(super) struct Soup {
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
}

impl Soup {
    /// A triangle facing `out`, whichever way it was wound.
    pub(super) fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, out: Vec3) {
        let n = (b - a).cross(c - a);
        let (b, c, n) = if n.dot(out) < 0.0 {
            (c, b, -n)
        } else {
            (b, c, n)
        };
        let n = n.normalize_or(out.normalize_or(Vec3::Y));
        let ax = n.abs();
        for p in [a, b, c] {
            let uv = if ax.x >= ax.y && ax.x >= ax.z {
                [p.z, p.y]
            } else if ax.y >= ax.z {
                [p.x, p.z]
            } else {
                [p.x, p.y]
            };
            self.pos.push(p.to_array());
            self.nrm.push(n.to_array());
            self.uv.push(uv);
        }
    }

    pub(super) fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, out: Vec3) {
        self.tri(a, b, c, out);
        self.tri(a, c, d, out);
    }

    pub(super) fn mesh(self) -> Mesh {
        let n = self.pos.len() as u32;
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.pos);
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.nrm);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uv);
        m.insert_indices(Indices::U32((0..n).collect()));
        m
    }
}

/// A fragment of the ring from `a0` to `a1` degrees, about its own middle:
/// the gilt section, the lapis line round its inner face, and jagged broken
/// ends. Returns the gilt, the lapis, and where its middle sat on the ring.
fn fragment(a0: f32, a1: f32, seed: u32) -> (Mesh, Mesh, Vec3) {
    let pivot = on_ring((a0 + a1) * 0.5, RING_R, 0.0);
    let (ri, ro, h) = (RING_R - RING_HALF, RING_R + RING_HALF, RING_HALF);
    let p = |t: f32, r: f32, z: f32| on_ring(t, r, z) - pivot;
    let (mut gilt, mut lapis) = (Soup::default(), Soup::default());
    let n = ((a1 - a0) / 5.0).ceil().max(2.0) as usize;
    for i in 0..n {
        let t0 = a0 + (a1 - a0) * i as f32 / n as f32;
        let t1 = a0 + (a1 - a0) * (i + 1) as f32 / n as f32;
        let rad = on_ring((t0 + t1) * 0.5, 1.0, 0.0);
        gilt.quad(
            p(t0, ro, -h),
            p(t1, ro, -h),
            p(t1, ro, h),
            p(t0, ro, h),
            rad,
        );
        gilt.quad(
            p(t0, ri, h),
            p(t1, ri, h),
            p(t1, ri, -h),
            p(t0, ri, -h),
            -rad,
        );
        gilt.quad(
            p(t0, ri, h),
            p(t0, ro, h),
            p(t1, ro, h),
            p(t1, ri, h),
            Vec3::Z,
        );
        gilt.quad(
            p(t1, ri, -h),
            p(t1, ro, -h),
            p(t0, ro, -h),
            p(t0, ri, -h),
            Vec3::NEG_Z,
        );
        let li = ri - 0.02;
        lapis.quad(
            p(t0, li, 0.12),
            p(t1, li, 0.12),
            p(t1, li, -0.12),
            p(t0, li, -0.12),
            -rad,
        );
    }
    // The breaks: each end a fan to a tip pushed out of the section, so a
    // fragment reads snapped rather than sawn.
    for (k, (t, sign)) in [(a0, -1.0f32), (a1, 1.0)].into_iter().enumerate() {
        let (s, c) = t.to_radians().sin_cos();
        let tangent = Vec3::new(-s, c, 0.0) * sign;
        let corners = [p(t, ri, -h), p(t, ro, -h), p(t, ro, h), p(t, ri, h)];
        let j = |x: u32| super::props::hash01(seed.wrapping_add(k as u32 * 7), x) * 2.0 - 1.0;
        // Off-centre and shallow: a slanted snap, not a gable.
        let tip = p(t, RING_R + 0.45 * j(1), 0.45 * j(2)) + tangent * (0.06 + 0.1 * (j(3) + 1.0));
        for e in 0..4 {
            gilt.tri(corners[e], corners[(e + 1) % 4], tip, tangent);
        }
    }
    (gilt.mesh(), lapis.mesh(), pivot)
}

/// A chip: an irregular octahedron `size` across.
fn chip(size: f32, seed: u32) -> Mesh {
    let j = |x: u32| 0.65 + 0.7 * super::props::hash01(seed, x);
    let v = [
        Vec3::X * size * j(1),
        Vec3::NEG_X * size * j(2),
        Vec3::Y * size * j(3) * 1.4,
        Vec3::NEG_Y * size * j(4) * 1.2,
        Vec3::Z * size * j(5) * 0.7,
        Vec3::NEG_Z * size * j(6) * 0.7,
    ];
    let mut s = Soup::default();
    for &(a, b, c) in &[
        (0, 2, 4),
        (4, 2, 1),
        (1, 2, 5),
        (5, 2, 0),
        (0, 4, 3),
        (4, 1, 3),
        (1, 5, 3),
        (5, 0, 3),
    ] {
        let (a, b, c) = (v[a], v[b], v[c]);
        s.tri(a, b, c, (a + b + c) / 3.0);
    }
    s.mesh()
}

/// A beam's profile: soft across, a hot line down the middle. White, the
/// shape in the alpha, like the sprite sheet.
pub fn beam_image() -> Image {
    const W: u32 = 64;
    const H: u32 = 4;
    let mut data = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let d = ((x as f32 + 0.5) / W as f32 * 2.0 - 1.0).abs();
            let soft = (1.0 - d).max(0.0).powf(2.6);
            let core = (1.0 - d / 0.14).clamp(0.0, 1.0).powi(2);
            let a = (soft * 0.85 + core * 0.15).min(1.0);
            let i = ((y * W + x) * 4) as usize;
            data[i..i + 4].copy_from_slice(&[255, 255, 255, (a * 255.0 + 0.5) as u8]);
        }
    }
    let mut img = Image::new(
        Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::linear();
    img
}

/// A beam `len` long along local +Y, `w0` wide at its foot and `w1` at its
/// end, fading from `a0` to nothing along it: the mesh a [`Beam`] turns.
pub fn beam_mesh(len: f32, w0: f32, w1: f32, a0: f32, fall: f32) -> Mesh {
    const ROWS: usize = 12;
    let mut pos = Vec::with_capacity(ROWS * 2 + 2);
    let mut uv = Vec::with_capacity(ROWS * 2 + 2);
    let mut col = Vec::with_capacity(ROWS * 2 + 2);
    let mut idx = Vec::with_capacity(ROWS * 6);
    for k in 0..=ROWS {
        // Rows close together at the foot, where the fade is steepest.
        let t = (k as f32 / ROWS as f32).powf(1.6);
        let w = w0 + (w1 - w0) * t;
        let a = a0 * (1.0 - t).powf(fall);
        for side in [-0.5f32, 0.5] {
            pos.push([side * w, t * len, 0.0]);
            uv.push([side + 0.5, t]);
            col.push([1.0, 1.0, 1.0, a]);
        }
        if k < ROWS {
            let b = (k * 2) as u32;
            idx.extend_from_slice(&[b, b + 1, b + 3, b, b + 3, b + 2]);
        }
    }
    let n = pos.len();
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 0.0, 1.0]; n]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    m.insert_indices(Indices::U32(idx));
    m
}

/// A unit billboard quad over the whole of the sprite sheet's glow cell.
fn halo_mesh(size: f32) -> Mesh {
    let h = size * 0.5;
    let cell = |u: f32, v: f32| {
        let inset = 0.5 / SPRITE_TEX as f32;
        [
            (inset + u * (1.0 - 2.0 * inset)) / SPRITE_COLS as f32,
            (inset + v * (1.0 - 2.0 * inset)) / SPRITE_ROWS as f32,
        ]
    };
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    m.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[-h, -h, 0.0], [h, -h, 0.0], [h, h, 0.0], [-h, h, 0.0]],
    );
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 0.0, 1.0]; 4]);
    m.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![
            cell(0.0, 1.0),
            cell(1.0, 1.0),
            cell(1.0, 0.0),
            cell(0.0, 0.0),
        ],
    );
    m.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    m
}

/// An unlit additive material over `tex`: light, not matter. Fog is off —
/// fog over an additive draw adds the fog's colour, a bright slab at range.
pub fn light_material(tex: Handle<Image>, rgb: [f32; 3]) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::linear_rgb(rgb[0], rgb[1], rgb[2]),
        base_color_texture: Some(tex),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        double_sided: true,
        cull_mode: None,
        fog_enabled: false,
        ..default()
    }
}

// ------------------------------------------------------------- systems ---

/// Build the gate's moving parts once per world, after the town.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    mut commands: Commands,
    world: Option<Res<WorldId>>,
    server: Res<AssetServer>,
    glow: Option<Res<TownGlow>>,
    fx: Res<Fx>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    drawn: Query<(), With<GateRoot>>,
) {
    let (Some(world), Some(glow)) = (world, glow) else {
        return;
    };
    let t = world.haven.town;
    if !t.live || !drawn.is_empty() {
        return;
    }
    let Some(lapis) = glow.lapis().first().cloned() else {
        return;
    };
    let tf = town::transform(&t);
    let root = commands
        .spawn((GateRoot, WorldEntity, tf, Visibility::default()))
        .id();
    let gilt = materials.add(town::dressed_material(Surface::Gilt, &server));
    let obsidian = materials.add(town::dressed_material(Surface::Obsidian, &server));

    let mut seed = (world.seed as u32) ^ 0x6A7E_0001;
    let mut roll = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed >> 8) as f32 / (1 << 24) as f32
    };

    for (i, f) in FRAGS.iter().enumerate() {
        let (g, l, pivot) = fragment(f.a0, f.a1, 0x51A7 + i as u32 * 31);
        let home = RING_C + pivot + f.drift;
        let base = Quat::from_euler(EulerRot::XYZ, f.tilt.x, f.tilt.y, f.tilt.z);
        let ends = [
            on_ring(f.a0, RING_R, 0.0) - pivot,
            on_ring(f.a1, RING_R, 0.0) - pivot,
        ];
        let tangent = on_ring((f.a0 + f.a1) * 0.5 + 90.0, 1.0, 0.0);
        let hang = Hanging {
            home,
            base,
            bob: f.bob,
            hz: f.hz,
            phase: f.phase,
            axis: tangent,
            spin: 0.07,
            rock: true,
            ends,
            frag: i as u8,
        };
        let piece = commands
            .spawn((
                ChildOf(root),
                hang,
                Transform::from_translation(home).with_rotation(base),
                Visibility::default(),
            ))
            .id();
        commands.spawn((
            ChildOf(piece),
            Mesh3d(meshes.add(g)),
            MeshMaterial3d(gilt.clone()),
            Transform::IDENTITY,
        ));
        commands.spawn((
            ChildOf(piece),
            Mesh3d(meshes.add(l)),
            MeshMaterial3d(lapis.clone()),
            Transform::IDENTITY,
            NotShadowCaster,
        ));
    }
    for k in 0..CHIPS {
        // Round the break and a little in front of and behind it, fallen a
        // few metres at most.
        let deg = 228.0 + 84.0 * roll();
        let r = RING_R + (roll() * 2.0 - 1.0) * 1.6;
        let home =
            RING_C + on_ring(deg, r, (roll() * 2.0 - 1.0) * 1.8) + Vec3::Y * (-0.4 - 2.2 * roll());
        let is_gilt = k % 3 != 2;
        let size = if is_gilt {
            0.12 + 0.22 * roll()
        } else {
            0.2 + 0.3 * roll()
        };
        let axis = Vec3::new(roll() - 0.5, roll() - 0.5, roll() - 0.5).normalize_or(Vec3::Y);
        let base = Quat::from_euler(EulerRot::XYZ, roll() * TAU, roll() * TAU, roll() * TAU);
        let hang = Hanging {
            home,
            base,
            bob: 0.08 + 0.18 * roll(),
            hz: 0.12 + 0.2 * roll(),
            phase: roll() * TAU,
            axis,
            spin: (0.15 + 0.45 * roll()) * if roll() < 0.5 { -1.0 } else { 1.0 },
            rock: false,
            ends: [Vec3::ZERO; 2],
            frag: u8::MAX,
        };
        commands.spawn((
            ChildOf(root),
            hang,
            Mesh3d(meshes.add(chip(size, 0xC41F + k as u32))),
            MeshMaterial3d(if is_gilt {
                gilt.clone()
            } else {
                obsidian.clone()
            }),
            Transform::from_translation(home).with_rotation(base),
        ));
    }

    // The arcs' flash and the dais lights, all dark until something lights
    // them.
    commands.spawn((
        WorldEntity,
        ArcLight,
        PointLight {
            color: ARC_COLOR,
            intensity: 0.0,
            range: 26.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_translation(tf.transform_point(RING_C)),
    ));
    for (at, lumens, range) in [
        (DAIS[0], DAIS_LUMENS, DAIS_RANGE_M),
        (DAIS[1], DAIS_LUMENS, DAIS_RANGE_M),
        (GAP_AT, GAP_LUMENS, GAP_RANGE_M),
    ] {
        commands.spawn((
            WorldEntity,
            DaisLight(lumens),
            PointLight {
                color: DAIS_COLOR,
                intensity: 0.0,
                range,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_translation(tf.transform_point(at)),
        ));
    }

    // The beacons: a shaft and a halo on each cap.
    let beam_tex = images.add(beam_image());
    let sheet = materials
        .get(&fx.glow_mat)
        .and_then(|m| m.base_color_texture.clone())
        .unwrap_or_default();
    let beacon_mat = materials.add(light_material(beam_tex.clone(), [0.0; 3]));
    let halo_mat = materials.add(light_material(sheet, [0.0; 3]));
    let shaft = meshes.add(beam_mesh(BEACON_LEN_M, BEACON_W0, BEACON_W1, 0.42, 2.4));
    let halo = meshes.add(halo_mesh(HALO_M));
    for cap in CAPS {
        let from = tf.transform_point(cap);
        commands.spawn((
            WorldEntity,
            Beacon,
            Beam { from, dir: Vec3::Y },
            Mesh3d(shaft.clone()),
            MeshMaterial3d(beacon_mat.clone()),
            Transform::from_translation(from),
            Visibility::Hidden,
            // Turned after the visibility pass (`aim`), so its bounds lag.
            NoFrustumCulling,
            NotShadowCaster,
        ));
        commands.spawn((
            WorldEntity,
            Beacon,
            Halo,
            Mesh3d(halo.clone()),
            MeshMaterial3d(halo_mat.clone()),
            Transform::from_translation(from + Vec3::Y * 0.4),
            Visibility::Hidden,
            NoFrustumCulling,
            NotShadowCaster,
        ));
    }

    // The motes' and arcs' one mesh, on the fx glow material.
    let mesh = meshes.add(pool_mesh(QUADS));
    commands.spawn((
        WorldEntity,
        GateMesh,
        Mesh3d(mesh.clone()),
        MeshMaterial3d(fx.glow_mat.clone()),
        Transform::IDENTITY,
        Visibility::Visible,
        NoFrustumCulling,
        NotShadowCaster,
    ));

    let rng = (world.seed as u32 | 1) ^ 0x2545_F491;
    let gaps = [[Vec3::ZERO; 2]; GAPS];
    commands.insert_resource(GateFx {
        tf,
        mesh,
        motes: vec![Mote::default(); MOTES],
        bolts: vec![Bolt::default(); BOLTS],
        discharges: [Discharge::default(); DISCHARGES],
        gaps,
        rng,
        clock: 0.0,
        next_crackle: 0.5,
        next_surge: 6.0,
        surge: 0.0,
        flash_left: 0.0,
        flash_life: 1.0,
        flash_lumens: 0.0,
        flash_at: RING_C,
        lapis_at: None,
        drawn: QUADS,
        near: false,
        arcs_seen: false,
        beacon_mat,
        beam_tex,
    });
    info!("gate: {} fragments and {CHIPS} chips hanging", FRAGS.len());
}

/// Sway the hanging pieces, run the discharge and the motes, breathe the
/// seams, and light what is lit.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn animate(
    time: Res<Time>,
    eye: Res<Eye>,
    gain: Res<FlameGain>,
    glow: Option<Res<TownGlow>>,
    gate: Option<ResMut<GateFx>>,
    mut fx: ResMut<Fx>,
    shake: Option<ResMut<super::shake::Shake>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut pieces: Query<(&Hanging, &mut Transform)>,
    mut arc_light: Query<
        (&mut PointLight, &mut Transform),
        (With<ArcLight>, Without<Hanging>, Without<DaisLight>),
    >,
    mut dais: Query<(&DaisLight, &mut PointLight), Without<ArcLight>>,
    mut beacons: Query<&mut Visibility, With<Beacon>>,
) {
    let (Some(mut gate), Some(glow)) = (gate, glow) else {
        return;
    };
    let dt = time.delta_secs().min(0.1);
    if dt <= 0.0 {
        return;
    }
    gate.clock += dt;
    let t = gate.clock;
    let night = glow.night();
    let g = gain.0;
    let centre = gate.tf.translation;
    let dist = eye.pos.distance(centre);
    gate.near = dist < NEAR_M;
    gate.arcs_seen = dist < ARC_FAR_M;

    // The hanging pieces, and the gaps between them this frame.
    let mut ends = [[Vec3::ZERO; 2]; FRAGS.len()];
    for (h, mut tf) in &mut pieces {
        let s = (t * h.hz * TAU + h.phase).sin();
        let pos = h.home + Vec3::Y * h.bob * s;
        let rot = if h.rock {
            Quat::from_axis_angle(h.axis, h.spin * (t * h.hz * 0.7 * TAU + h.phase).sin()) * h.base
        } else {
            Quat::from_axis_angle(h.axis, h.spin * t) * h.base
        };
        tf.translation = pos;
        tf.rotation = rot;
        if let Some(e) = ends.get_mut(h.frag as usize) {
            *e = [pos + rot * h.ends[0], pos + rot * h.ends[1]];
        }
    }
    let lo = RING_C + on_ring(BREAK_LO, RING_R, 0.0);
    let hi = RING_C + on_ring(BREAK_HI, RING_R, 0.0);
    gate.gaps[0] = [lo, ends[0][0]];
    for k in 1..FRAGS.len() {
        gate.gaps[k] = [ends[k - 1][1], ends[k][0]];
    }
    gate.gaps[GAPS - 1] = [ends[FRAGS.len() - 1][1], hi];

    // The discharge: crackle across a gap now and then, and a surge.
    gate.next_crackle -= dt;
    if gate.next_crackle <= 0.0 {
        gate.next_crackle = gate.range(CRACKLE_S);
        let gap = (gate.roll() * GAPS as f32) as u8;
        let strikes = 1 + (gate.roll() * 3.0) as u8;
        gate.discharge(gap.min(GAPS as u8 - 1), strikes, 0.055);
    }
    gate.next_surge -= dt;
    if gate.next_surge <= 0.0 {
        gate.next_surge = gate.range(SURGE_S);
        for gap in 0..GAPS as u8 {
            let strikes = 3 + (gate.roll() * 2.0) as u8;
            gate.discharge(gap, strikes, 0.1);
        }
        gate.discharge(GAPS as u8, 3, 0.09);
        gate.discharge(GAPS as u8 + 1, 3, 0.09);
        gate.surge = 1.0;
        gate.flash_left = 0.45;
        gate.flash_life = 0.45;
        gate.flash_lumens = SURGE_LUMENS;
        gate.flash_at = RING_C + Vec3::Y * -6.0;
        // Close enough and you feel it in your feet.
        if let Some(mut shake) = shake {
            let at = gate.tf.transform_point(RING_C - Vec3::Y * 10.0);
            shake.add_at(0.2, at, eye.pos, 10.0, 32.0);
        }
        if gate.near {
            // A ring of sparks thrown out of the break.
            for _ in 0..40 {
                let deg = gate.range((BREAK_LO, BREAK_HI));
                let pos = RING_C + on_ring(deg, RING_R + gate.signed() * 0.8, gate.signed());
                let vel = (on_ring(deg, 1.0, gate.signed() * 0.5) * gate.range((1.0, 3.5)))
                    + Vec3::Y * gate.range((0.0, 1.5));
                let life = gate.range((1.2, 2.4));
                let m = Mote {
                    left: life,
                    life,
                    kind: Kind::Burst,
                    pos,
                    vel,
                    size: gate.range((0.04, 0.09)),
                    ..Mote::default()
                };
                gate.mote(m);
            }
        }
    }
    gate.surge = (gate.surge - dt / SURGE_FALL_S).max(0.0);
    let seen = gate.arcs_seen;
    for i in 0..DISCHARGES {
        let mut d = gate.discharges[i];
        if d.strikes == 0 {
            continue;
        }
        d.next -= dt;
        if d.next <= 0.0 {
            d.strikes -= 1;
            d.next = gate.range((0.035, 0.11));
            if seen {
                let sparks = gate.near.then_some(&mut fx.glow);
                gate.strike(d.gap as usize, d.width, sparks);
            }
        }
        gate.discharges[i] = d;
    }
    for b in gate.bolts.iter_mut() {
        b.left -= dt;
    }

    // The motes.
    if gate.near {
        for _ in 0..gate.count(VORTEX_RATE, dt) {
            let life = gate.range((5.0, 8.0));
            let r = gate.range((6.4, 7.1));
            let m = Mote {
                left: life,
                life,
                kind: Kind::Vortex,
                theta: gate.roll() * TAU,
                r,
                z: gate.signed() * 0.7,
                omega: gate.range((0.22, 0.5)),
                vr: r / life * gate.range((0.3, 0.75)),
                size: gate.range((0.045, 0.1)),
                phase: gate.roll() * TAU,
                ..Mote::default()
            };
            gate.mote(m);
        }
        for _ in 0..gate.count(RISE_RATE, dt) {
            let life = gate.range((5.0, 8.0));
            // Off the dais either side of the street, or out of a fallen
            // block's cracks.
            let pos = match (gate.roll() * 5.0) as u32 {
                0 => Vec3::new(gate.range((16.5, 21.5)), 1.9, gate.range((-11.5, -7.5))),
                1 => Vec3::new(gate.range((-21.5, -16.5)), 2.3, gate.range((-9.5, -6.5))),
                k => {
                    let side = if k % 2 == 0 { -1.0 } else { 1.0 };
                    Vec3::new(
                        side * gate.range((5.8, 13.6)),
                        0.55,
                        gate.range((-5.6, 5.6)),
                    )
                }
            };
            let m = Mote {
                left: life,
                life,
                kind: Kind::Rise,
                pos,
                vel: Vec3::Y * gate.range((0.45, 1.05)),
                size: gate.range((0.03, 0.065)),
                phase: gate.roll() * TAU,
                ..Mote::default()
            };
            gate.mote(m);
        }
        // After dark, cold light pools on the dais and creeps off it.
        for _ in 0..gate.count(MIST_RATE * night, dt) {
            let life = gate.range((6.0, 9.0));
            let side = if gate.roll() < 0.5 { -1.0 } else { 1.0 };
            let pos = Vec3::new(side * gate.range((6.0, 13.5)), 0.8, gate.range((-5.5, 5.5)));
            let vel = Vec3::new(side * gate.range((0.05, 0.2)), 0.02, gate.signed() * 0.12);
            let m = Mote {
                left: life,
                life,
                kind: Kind::Mist,
                pos,
                vel,
                size: gate.range((1.4, 2.6)),
                phase: gate.roll() * TAU,
                ..Mote::default()
            };
            gate.mote(m);
        }
        // Glints follow the sun: many at noon, a few at night off the seams.
        let glints = GLINT_RATE * (1.0 - 0.8 * night);
        for _ in 0..gate.count(glints, dt) {
            let deg = gate.range((STAND_FROM, STAND_TO));
            let face = gate.roll();
            let pos = if face < 0.5 {
                RING_C + on_ring(deg, RING_R + RING_HALF + 0.02, gate.signed() * 0.5)
            } else {
                let z = if face < 0.75 {
                    RING_HALF + 0.02
                } else {
                    -RING_HALF - 0.02
                };
                RING_C + on_ring(deg, RING_R + gate.signed() * 0.5, z)
            };
            let life = gate.range((0.25, 0.5));
            let m = Mote {
                left: life,
                life,
                kind: Kind::Glint,
                pos,
                size: gate.range((0.18, 0.42)),
                phase: gate.roll() * TAU,
                ..Mote::default()
            };
            gate.mote(m);
        }
    }
    for m in gate.motes.iter_mut() {
        if m.left <= 0.0 {
            continue;
        }
        m.left -= dt;
        match m.kind {
            Kind::Vortex => {
                m.theta += m.omega * dt * (1.0 + 1.2 * (1.0 - m.r / RING_R).max(0.0));
                m.r = (m.r - m.vr * dt).max(0.05);
                m.z *= (-0.6 * dt).exp();
            }
            Kind::Rise => {
                let age = m.life - m.left;
                m.vel.x = 0.28 * (m.phase + age * 1.7).sin();
                m.vel.z = 0.28 * (m.phase * 1.3 + age * 1.3).cos();
                // High enough and it is drawn toward the ring.
                if m.pos.y > 5.0 {
                    let to = (RING_C - m.pos).normalize_or(Vec3::Y);
                    m.vel += to * 0.6;
                }
                m.pos += m.vel * dt;
            }
            Kind::Burst => {
                m.vel *= (-1.2 * dt).exp();
                m.vel.y -= 0.6 * dt;
                m.pos += m.vel * dt;
            }
            Kind::Mist => m.pos += m.vel * dt,
            Kind::Glint => {}
        }
    }

    // The seams breathe, and burn on a surge.
    let b = 0.5 + 0.5 * (t * TAU / BREATH_S).sin();
    let breath = 1.0 - BREATH_DEPTH + BREATH_DEPTH * b * b;
    let mix = |a: LinearRgba, b: LinearRgba, k: f32| {
        LinearRgba::rgb(
            a.red + (b.red - a.red) * k,
            a.green + (b.green - a.green) * k,
            a.blue + (b.blue - a.blue) * k,
        )
    };
    let base = mix(LAPIS_DAY, LAPIS_NIGHT, night);
    let s = gate.surge * gate.surge;
    let want = LinearRgba::rgb(
        base.red * breath + LAPIS_SURGE.red * s,
        base.green * breath + LAPIS_SURGE.green * s,
        base.blue * breath + LAPIS_SURGE.blue * s,
    );
    let moved = gate
        .lapis_at
        .is_none_or(|a| (a.blue - want.blue).abs() > 0.004 * want.blue.max(0.1));
    if moved {
        gate.lapis_at = Some(want);
        for h in glow.lapis() {
            if let Some(m) = materials.get_mut(h) {
                m.emissive = want;
            }
        }
    }

    // The flash, the dais pools and the beacons.
    gate.flash_left = (gate.flash_left - dt).max(0.0);
    if let Ok((mut light, mut tf)) = arc_light.single_mut() {
        let k = (gate.flash_left / gate.flash_life.max(1e-4)).powi(2);
        let lumens = if gate.flash_lumens >= SURGE_LUMENS {
            (gate.flash_lumens * g).max(SURGE_DAY_LUMENS)
        } else {
            gate.flash_lumens * g
        };
        let want = if seen { lumens * k } else { 0.0 };
        if light.intensity != want {
            light.intensity = want;
        }
        let at = gate.tf.transform_point(gate.flash_at);
        if tf.translation != at {
            tf.translation = at;
        }
    }
    for (d, mut l) in &mut dais {
        let want = d.0 * g * night * breath * (1.0 + 3.0 * s);
        if (l.intensity - want).abs() > 0.01 * want.max(1.0) {
            l.intensity = want;
        }
    }
    let shown = night > 0.02;
    for mut v in &mut beacons {
        let want = if shown {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    if shown {
        let k = night * (0.85 + 0.15 * breath) * (1.0 + 0.8 * s);
        if let Some(m) = materials.get_mut(&gate.beacon_mat) {
            m.base_color =
                Color::linear_rgb(BEACON_RGB[0] * k, BEACON_RGB[1] * k, BEACON_RGB[2] * k);
        }
    }
}

/// Turn every beam about its axis to face the eye, and every halo square on
/// to it — after the camera's transform is final for the frame.
#[allow(clippy::type_complexity)]
pub fn aim(
    cams: Query<&GlobalTransform, With<EyeCam>>,
    mut beams: Query<
        (&Beam, &mut Transform, &mut GlobalTransform),
        (Without<EyeCam>, Without<Halo>),
    >,
    mut halos: Query<(&mut Transform, &mut GlobalTransform), (With<Halo>, Without<EyeCam>)>,
) {
    let Ok(cam) = cams.single() else { return };
    let eye = cam.translation();
    for (b, mut tf, mut gt) in &mut beams {
        let y = b.dir.normalize_or(Vec3::Y);
        let mid = b.from + y * 4.0;
        let x = y.cross(eye - mid).normalize_or(Vec3::X);
        let z = x.cross(y);
        let rot = Quat::from_mat3(&Mat3::from_cols(x, y, z));
        if tf.rotation != rot || tf.translation != b.from {
            tf.translation = b.from;
            tf.rotation = rot;
            *gt = GlobalTransform::from(*tf);
        }
    }
    let rot = cam.compute_transform().rotation;
    for (mut tf, mut gt) in &mut halos {
        if tf.rotation != rot {
            tf.rotation = rot;
            *gt = GlobalTransform::from(*tf);
        }
    }
}

/// Write every live mote and arc into the gate's mesh, anchored at the eye.
pub fn draw(
    gate: Option<ResMut<GateFx>>,
    glow: Option<Res<TownGlow>>,
    cams: Query<&GlobalTransform, (With<EyeCam>, Without<GateMesh>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut q: Query<(&mut Transform, &mut GlobalTransform), With<GateMesh>>,
) {
    let Some(mut gate) = gate else { return };
    let Ok(cam_gt) = cams.single() else { return };
    let cam = Cam {
        pos: cam_gt.translation(),
        right: cam_gt.right().into(),
        up: cam_gt.up().into(),
    };
    let anchor = cam.pos;
    if let Ok((mut tf, mut gt)) = q.single_mut() {
        if tf.translation != anchor {
            tf.translation = anchor;
            *gt = GlobalTransform::from_translation(anchor);
        }
    }
    let night = glow.map_or(0.0, |g| g.night());
    let Some(mesh) = meshes.get_mut(&gate.mesh) else {
        return;
    };
    let (mut pos, mut uv, mut col) = (None, None, None);
    for (attr, values) in mesh.attributes_mut() {
        let id = attr.id;
        match values {
            VertexAttributeValues::Float32x3(v) if id == Mesh::ATTRIBUTE_POSITION.id => {
                pos = Some(v);
            }
            VertexAttributeValues::Float32x2(v) if id == Mesh::ATTRIBUTE_UV_0.id => {
                uv = Some(v);
            }
            VertexAttributeValues::Float32x4(v) if id == Mesh::ATTRIBUTE_COLOR.id => {
                col = Some(v);
            }
            _ => {}
        }
    }
    let (Some(pos), Some(uv), Some(col)) = (pos, uv, col) else {
        return;
    };
    let tf = gate.tf;
    let mut n = 0usize;
    let mut put = |q: ([[f32; 3]; 4], [[f32; 2]; 4], [f32; 4])| {
        if n >= QUADS {
            return;
        }
        let b = n * 4;
        for k in 0..4 {
            pos[b + k] = q.0[k];
            uv[b + k] = q.1[k];
            col[b + k] = q.2;
        }
        n += 1;
    };

    // The arcs: each segment a ribbon facing the eye, a glow under a core.
    // The glow cell's middle column carries the profile across.
    let inset = 0.5 / SPRITE_TEX as f32;
    let across = |v: f32| {
        [
            (0.5) / SPRITE_COLS as f32,
            (inset + v * (1.0 - 2.0 * inset)) / SPRITE_ROWS as f32,
        ]
    };
    let ribbon_uv = [across(0.0), across(0.0), across(1.0), across(1.0)];
    let bright = 1.0 + 0.6 * night;
    for b in gate.bolts.iter().filter(|b| b.left > 0.0 && b.n > 1) {
        let k = (b.left / b.life.max(1e-4)).clamp(0.0, 1.0).sqrt() * bright;
        for s in 0..b.n - 1 {
            let p0 = tf.transform_point(b.pts[s]);
            let p1 = tf.transform_point(b.pts[s + 1]);
            let mid = (p0 + p1) * 0.5;
            let dist = mid.distance(cam.pos);
            let view = (mid - cam.pos).normalize_or(Vec3::Z);
            let side = view.cross(p1 - p0).normalize_or(cam.right);
            for (w, rgb, a) in [
                (b.width * 7.0, ARC_GLOW, 0.55),
                (b.width.max(dist * MIN_PX * 0.8), ARC_CORE, 1.0),
            ] {
                let o = side * w;
                let c = [rgb[0] * k, rgb[1] * k, rgb[2] * k, a * k.min(1.0)];
                put((
                    [
                        (p0 - o - anchor).to_array(),
                        (p1 - o - anchor).to_array(),
                        (p1 + o - anchor).to_array(),
                        (p0 + o - anchor).to_array(),
                    ],
                    ribbon_uv,
                    c,
                ));
            }
        }
    }

    // The motes, as the fx pools draw a particle.
    let lapis_k = 0.7 + 0.8 * night + 1.5 * gate.surge;
    for m in gate.motes.iter().filter(|m| m.left > 0.0) {
        let age = 1.0 - m.left / m.life.max(1e-4);
        let fade = (age / 0.15).min(1.0) * ((1.0 - age) / 0.35).min(1.0);
        // A vortex mote streaks along its orbit, so the ring reads as a
        // slow swirl rather than a second starfield.
        let mut streak = None;
        let (local, rgb, cell, size, a) = match m.kind {
            Kind::Vortex => {
                let (s, c) = m.theta.sin_cos();
                let twinkle = 0.75 + 0.25 * (m.phase + age * 31.0).sin();
                streak = Some(Vec3::new(-s, c, 0.0) * (m.omega * m.r).max(0.4));
                (
                    RING_C + Vec3::new(m.r * c, m.r * s, m.z),
                    MOTE_LAPIS,
                    atlas::STREAK,
                    m.size * 0.45,
                    fade * twinkle * lapis_k,
                )
            }
            Kind::Rise | Kind::Burst => (m.pos, MOTE_LAPIS, atlas::GLOW, m.size, fade * lapis_k),
            Kind::Mist => (
                m.pos,
                MIST_LAPIS,
                atlas::PUFF + (m.phase as u8 % 3),
                m.size * (0.8 + 0.4 * age),
                fade * 0.08 * night,
            ),
            Kind::Glint => {
                // Up and gone: a flash that peaks a third of the way in.
                let k = (1.0 - (age - 0.33).abs() / 0.67).max(0.0);
                (
                    m.pos,
                    MOTE_GILT,
                    atlas::STAR + (m.phase as u8 % 3),
                    m.size * (0.4 + 0.6 * k),
                    k * (1.0 - 0.6 * night),
                )
            }
        };
        if a <= 0.0 {
            continue;
        }
        let (orient, vel) = match streak {
            Some(v) => (Orient::Stretch, tf.rotation * v),
            None => (Orient::Billboard, Vec3::ZERO),
        };
        let p = Particle {
            left: 1.0,
            life: 1.0,
            pos: tf.transform_point(local),
            vel,
            size0: size,
            size1: size,
            c0: [rgb[0] * a, rgb[1] * a, rgb[2] * a, a.min(1.0)],
            c1: [rgb[0] * a, rgb[1] * a, rgb[2] * a, a.min(1.0)],
            roll: m.phase,
            cell,
            orient,
            stretch: 0.6,
            len_max: 0.9,
            ..Particle::default()
        };
        if let Some(q) = Pool::quad(&p, &cam, anchor) {
            put(q);
        }
    }

    // Collapse whatever the last write drew and this one did not.
    let used = n;
    for slot in used..gate.drawn.max(used) {
        let b = slot * 4;
        for k in 0..4 {
            pos[b + k] = [0.0; 3];
            col[b + k] = [0.0; 4];
        }
    }
    gate.drawn = used;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fragments_fill_the_break_in_order() {
        let mut at = BREAK_LO;
        for f in &FRAGS {
            assert!(f.a0 > at && f.a1 > f.a0, "fragments run round the break");
            at = f.a1;
        }
        assert!(at < BREAK_HI);
    }

    #[test]
    fn a_bolt_runs_end_to_end() {
        let mut g = GateFx {
            tf: Transform::IDENTITY,
            mesh: Handle::default(),
            motes: vec![Mote::default(); 4],
            bolts: vec![Bolt::default(); 2],
            discharges: [Discharge::default(); DISCHARGES],
            gaps: [[Vec3::ZERO; 2]; GAPS],
            rng: 7,
            clock: 0.0,
            next_crackle: 0.0,
            next_surge: 0.0,
            surge: 0.0,
            flash_left: 0.0,
            flash_life: 1.0,
            flash_lumens: 0.0,
            flash_at: Vec3::ZERO,
            lapis_at: None,
            drawn: 0,
            near: true,
            arcs_seen: true,
            beacon_mat: Handle::default(),
            beam_tex: Handle::default(),
        };
        let (a, b) = (Vec3::new(-1.0, 2.0, 0.0), Vec3::new(3.0, 1.0, 0.5));
        let i = g.bolt(a, b, 0.2, 0.05, 0.1, BOLT_PTS);
        let bolt = g.bolts[i];
        assert_eq!(bolt.pts[0], a);
        assert_eq!(bolt.pts[BOLT_PTS - 1], b);
        // Jagged, but never further from the line than its own length.
        for p in &bolt.pts[..bolt.n] {
            assert!(p.distance(a.lerp(b, 0.5)) < (b - a).length() * 1.5);
        }
        // Every slot taken: the next bolt is refused, not stolen.
        g.bolt(a, b, 0.2, 0.05, 0.1, BOLT_PTS);
        assert_eq!(g.bolt(a, b, 0.2, 0.05, 0.1, BOLT_PTS), usize::MAX);
    }
}
