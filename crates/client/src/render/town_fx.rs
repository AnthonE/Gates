//! THE GATE's yard, lived in: what makes a scrap town read as a place people
//! keep rather than a set of boxes.
//!
//! - **Festoons**: strings of coloured bulbs slung off the pylons to every
//!   gate and watchtower, a fairground canopy over the yard. They twinkle in
//!   three phases after dusk.
//! - **Braziers**: fire baskets on the gate posts, so every way in is
//!   flanked by flame — the fx pools' fire, and a light that flickers.
//! - **Searchlights**: one on each watchtower, sweeping the approaches at
//!   night with a beam you can see from the road.
//! - **Banners**: the town's colours — two pillars and the broken ring, in
//!   gilt on lapis — hanging from every gate beam and streaming off every
//!   tower roof, moving in the weather's wind.
//! - **A bad junction**: the street lamp by the spawn has a short in it and
//!   spits a shower of sparks every few seconds.
//!
//! Drawn, never collided: every piece sits above head height or on top of
//! something solid. The clock and the wind are the client's.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::f32::consts::TAU;

use super::fx::pool::{Orient, Particle};
use super::fx::table::{emit, Layer};
use super::fx::world::{FireFx, Tongues};
use super::fx::{atlas, Fx};
use super::gate_fx::{self, Beam, Soup};
use super::impact::Matter;
use super::rig::FlameGain;
use super::town::{self, TownGlow};
use super::weather::WeatherNow;
use super::{mipmap, Eye, WorldEntity, WorldId};

// ------------------------------------------------------------ festoons ---

/// A string of bulbs: two kit-local ends and how far it sags, metres.
type Festoon = ([f32; 3], [f32; 3], f32);

/// Off the pylons (`sim_core::town::GATE_SHAFT`: x ±22..30, z ±4.5) to the
/// gate beams, the posts of the street gates and the watchtower roofs.
const FESTOONS: [Festoon; 10] = [
    ([30.0, 14.0, 0.0], [40.2, 7.0, 0.0], 2.4),
    ([-30.0, 14.0, 0.0], [-40.2, 7.0, 0.0], 2.4),
    ([26.0, 14.0, 4.5], [5.5, 7.3, 39.2], 3.0),
    ([-26.0, 14.0, 4.5], [-5.5, 7.3, 39.2], 3.0),
    ([26.0, 14.0, -4.5], [5.5, 7.3, -39.2], 3.0),
    ([-26.0, 14.0, -4.5], [-5.5, 7.3, -39.2], 3.0),
    ([30.0, 18.0, 4.5], [32.2, 9.7, 32.2], 3.6),
    ([-30.0, 18.0, 4.5], [-32.2, 9.7, 32.2], 3.6),
    ([30.0, 18.0, -4.5], [32.2, 9.7, -32.2], 3.6),
    ([-30.0, 18.0, -4.5], [-32.2, 9.7, -32.2], 3.6),
];
/// A bulb every this many metres along a string.
const BULB_EVERY_M: f32 = 1.15;
/// The bulbs' colours, linear, in the order they hang: mostly warm white,
/// with the fairground's amber, rose and teal between.
const BULB_RGB: [[f32; 3]; 6] = [
    [1.0, 0.78, 0.48],
    [1.0, 0.55, 0.16],
    [1.0, 0.78, 0.48],
    [1.0, 0.32, 0.3],
    [1.0, 0.78, 0.48],
    [0.3, 0.85, 0.8],
];
/// Bunting between the bulbs, linear albedo: lapis, gilt, rust and bone.
const BUNTING_RGB: [[f32; 3]; 4] = [
    [0.04, 0.09, 0.36],
    [0.72, 0.48, 0.1],
    [0.42, 0.07, 0.04],
    [0.7, 0.66, 0.56],
];
/// A pennant of bunting every this many metres, its width and its drop.
const BUNTING_EVERY_M: f32 = 0.75;
const BUNTING_W: f32 = 0.38;
const BUNTING_DROP: f32 = 0.46;
/// Their brightness at noon (glass) and at full night (HDR, for the bloom).
const BULB_DAY: f32 = 0.55;
const BULB_NIGHT: f32 = 6.5;
/// Twinkle phases, and one twinkle's period, seconds.
const PHASES: usize = 3;
const TWINKLE_S: f32 = 2.7;

// ------------------------------------------------------------ braziers ---

/// A fire basket on the outer end of each gate post (top 7.4 m).
const BRAZIERS: [[f32; 2]; 8] = [
    [5.5, 41.9],
    [-5.5, 41.9],
    [5.5, -41.9],
    [-5.5, -41.9],
    [41.9, 4.5],
    [41.9, -4.5],
    [-41.9, 4.5],
    [-41.9, -4.5],
];
const POST_TOP: f32 = 7.4;
/// Lumens at the night eye's gain, and how far it reaches. **(knob)**
const BRAZIER_LUMENS: f32 = 1_500.0;
const BRAZIER_RANGE_M: f32 = 11.0;
const BRAZIER_COLOR: Color = Color::srgb(1.0, 0.6, 0.26);

#[derive(Component)]
pub struct Brazier {
    phase: f32,
}

/// The coals' glow, so it breathes with the flames.
#[derive(Component)]
pub struct Coals;

// --------------------------------------------------------- searchlights ---

/// Each watchtower's searchlight: on the deck's outer corner, under the
/// roof.
const SEARCH_AT: [[f32; 3]; 4] = [
    [36.9, 8.1, 36.9],
    [-36.9, 8.1, 36.9],
    [36.9, 8.1, -36.9],
    [-36.9, 8.1, -36.9],
];
/// The sweep: half its arc (radians), its period (seconds), and the beam's
/// tilt below level, with how far that nods.
const SWEEP_HALF: f32 = 1.0;
const SWEEP_S: f32 = 14.0;
const TILT: f32 = -0.21;
const NOD: f32 = 0.06;
/// The drawn beam: length, width at the lamp and at the end, metres.
const SEARCH_LEN_M: f32 = 60.0;
const SEARCH_W0: f32 = 0.55;
const SEARCH_W1: f32 = 12.0;
const SEARCH_RGB: [f32; 3] = [0.75, 0.74, 0.62];
/// The spot itself: lumens at the night eye's gain, reach and cone.
const SEARCH_LUMENS: f32 = 14_000.0;
const SEARCH_RANGE_M: f32 = 80.0;
const SEARCH_CONE: f32 = 0.085;
/// The lamp head, generated: a finned steel drum 0.5 m across, its lens up
/// +Y (the way [`SearchHead`] aims) and centred on the pivot.
pub const SEARCHLIGHT_GLB: &str = "models/site/searchlight.glb";
/// The glowing disc over the model's lens: its radius and height on +Y.
const LENS_R: f32 = 0.18;
const LENS_Y: f32 = 0.21;

#[derive(Component)]
pub struct Searchlight {
    k: usize,
    at: Vec3,
    yaw0: f32,
    phase: f32,
}

/// The lamp's drum and lens, turned with the beam.
#[derive(Component)]
pub struct SearchHead(usize);

#[derive(Component)]
pub struct SearchBeam(usize);

// -------------------------------------------------------------- banners ---

/// A banner's grid: across, and down (or along, for a pennant).
const BANNER_NU: usize = 6;
const BANNER_NV: usize = 10;
const BANNER_W: f32 = 1.3;
const BANNER_H: f32 = 3.0;
const PENNANT_NU: usize = 10;
const PENNANT_NV: usize = 3;
const PENNANT_L: f32 = 1.9;
const PENNANT_H: f32 = 0.7;
const POLE_TOP: f32 = 13.0;

/// The most vertices a cloth has: a banner's grid, which a pennant's fits in.
const CLOTH_VERTS: usize = (BANNER_NU + 1) * (BANNER_NV + 1);
const _: () = assert!((PENNANT_NU + 1) * (PENNANT_NV + 1) <= CLOTH_VERTS);

/// Banners and pennants move within this of the town's middle, metres.
const CLOTH_M: f32 = 180.0;

/// A hanging banner (`hang`) or a streaming pennant, and the frame it moves
/// in.
#[derive(Component)]
pub struct Cloth {
    mesh: Handle<Mesh>,
    pennant: bool,
    phase: f32,
    /// A banner's face, outward, world space (a pennant turns to the wind
    /// instead).
    normal: Vec3,
}

// ------------------------------------------------------------ the canteen ---

/// Stovepipes on the canteen's two containers (roofs 2.6 m): their feet,
/// kit-local, and how tall they stand.
const STOVEPIPES: [[f32; 2]; 2] = [[17.0, -32.4], [32.9, -32.6]];
const PIPE_H: f32 = 1.8;
/// Cooking smoke a second, per pipe.
const COOK_SMOKE_RATE: f32 = 1.4;

// --------------------------------------------------------- the junction ---

/// The street lamp with the short in it: the post at kit (5.8, −18), its
/// head at 5 m.
const SHORT_AT: [f32; 3] = [5.8, 4.85, -18.0];
/// Seconds between bursts.
const SHORT_S: (f32, f32) = (2.5, 8.0);

#[derive(Component)]
pub struct ShortLight;

/// The yard's moving parts.
#[derive(Resource)]
pub struct TownFx {
    tf: Transform,
    bulb_mats: [Handle<StandardMaterial>; PHASES],
    coals_mat: Handle<StandardMaterial>,
    beam_mat: Handle<StandardMaterial>,
    clock: f32,
    rng: u32,
    next_short: f32,
    short_flash: f32,
    /// The pennants' heading, smoothed: radians of yaw.
    heading: f32,
    lit_at: Option<f32>,
    /// The searchlight beams' brightness last written.
    beam_k: Option<f32>,
}

impl TownFx {
    fn roll(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1 << 24) as f32
    }

    fn signed(&mut self) -> f32 {
        self.roll() * 2.0 - 1.0
    }
}

#[derive(Component)]
pub struct TownFxRoot;

// ------------------------------------------------------------- meshing ---

/// A point on a slung string, `t` of the way from `a` to `b`.
fn sling(a: Vec3, b: Vec3, sag: f32, t: f32) -> Vec3 {
    a.lerp(b, t) - Vec3::Y * sag * 4.0 * t * (1.0 - t)
}

/// A thin square tube from `p` to `q` (a wire, a pole).
fn tube(s: &mut Soup, p: Vec3, q: Vec3, r: f32) {
    let d = (q - p).normalize_or(Vec3::Y);
    let a = d.any_orthonormal_vector();
    let b = d.cross(a);
    let c = [a * r, b * r, -a * r, -b * r];
    for k in 0..4 {
        let (u, v) = (c[k], c[(k + 1) % 4]);
        s.quad(p + u, q + u, q + v, p + v, u + v);
    }
}

/// A soup with a colour per vertex: positions, normals and colours.
type Tinted = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 4]>);

fn tinted_mesh((pos, nrm, col): Tinted) -> Mesh {
    let n = pos.len() as u32;
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    m.insert_indices(Indices::U32((0..n).collect()));
    m
}

/// The bulbs of every string, one mesh per twinkle phase, with each bulb's
/// colour in its vertices; the bunting between them; and the wires.
fn festoon_meshes() -> ([Mesh; PHASES], Mesh, Mesh) {
    let mut wires = Soup::default();
    let mut bulbs: [Tinted; PHASES] = Default::default();
    let mut bunting: Tinted = Default::default();
    let mut k = 0usize;
    for &(a, b, sag) in &FESTOONS {
        let (a, b) = (Vec3::from_array(a), Vec3::from_array(b));
        let len = a.distance(b) + sag;
        let segs = 24;
        for i in 0..segs {
            let p = sling(a, b, sag, i as f32 / segs as f32);
            let q = sling(a, b, sag, (i + 1) as f32 / segs as f32);
            tube(&mut wires, p, q, 0.018);
        }
        // Bunting: a pennant hanging off the wire every stride, its top edge
        // along the wire, its point straight down.
        let flags = (len / BUNTING_EVERY_M).floor() as usize;
        for i in 1..flags {
            let t = (i as f32 + 0.5) / flags as f32;
            let dt = BUNTING_W * 0.5 / len;
            let (l, r) = (sling(a, b, sag, t - dt), sling(a, b, sag, t + dt));
            let tip = (l + r) * 0.5 - Vec3::Y * BUNTING_DROP;
            let rgb = BUNTING_RGB[i % BUNTING_RGB.len()];
            let n = (r - l).cross(Vec3::NEG_Y).normalize_or(Vec3::Z);
            let (pos, nrm, col) = &mut bunting;
            for p in [l, r, tip] {
                pos.push(p.to_array());
                nrm.push(n.to_array());
                col.push([rgb[0], rgb[1], rgb[2], 1.0]);
            }
        }
        let n = (len / BULB_EVERY_M).floor() as usize;
        for i in 1..n {
            let at = sling(a, b, sag, i as f32 / n as f32) - Vec3::Y * 0.09;
            let rgb = BULB_RGB[k % BULB_RGB.len()];
            let (pos, nrm, col) = &mut bulbs[k % PHASES];
            // An octahedron, a little long: a bulb, at this size.
            let v = [
                Vec3::X * 0.07,
                Vec3::NEG_X * 0.07,
                Vec3::Y * 0.07,
                Vec3::NEG_Y * 0.11,
                Vec3::Z * 0.07,
                Vec3::NEG_Z * 0.07,
            ];
            for &(i0, i1, i2) in &[
                (0, 2, 4),
                (4, 2, 1),
                (1, 2, 5),
                (5, 2, 0),
                (0, 4, 3),
                (4, 1, 3),
                (1, 5, 3),
                (5, 0, 3),
            ] {
                let (p0, p1, p2) = (v[i0], v[i1], v[i2]);
                let n = (p1 - p0).cross(p2 - p0).normalize_or(Vec3::Y);
                for p in [p0, p1, p2] {
                    pos.push((at + p).to_array());
                    nrm.push(n.to_array());
                    col.push([rgb[0], rgb[1], rgb[2], 1.0]);
                }
            }
            k += 1;
        }
    }
    (bulbs.map(tinted_mesh), tinted_mesh(bunting), wires.mesh())
}

/// A fire basket: an eight-sided bowl on three legs, and its coals.
fn brazier_meshes() -> (Mesh, Mesh) {
    let mut iron = Soup::default();
    let mut coals = Soup::default();
    let ring = |r: f32, y: f32, k: usize| {
        let a = k as f32 / 8.0 * TAU;
        Vec3::new(r * a.cos(), y, r * a.sin())
    };
    for k in 0..8 {
        let (lo0, lo1) = (ring(0.2, 0.18, k), ring(0.2, 0.18, k + 1));
        let (hi0, hi1) = (ring(0.42, 0.55, k), ring(0.42, 0.55, k + 1));
        let mid = (lo0 + lo1 + hi0 + hi1) * 0.25;
        let out = Vec3::new(mid.x, 0.0, mid.z);
        iron.quad(lo0, lo1, hi1, hi0, out);
        iron.quad(lo0, hi0, hi1, lo1, -out);
        // The rim, a lip turned out.
        let (r0, r1) = (ring(0.47, 0.57, k), ring(0.47, 0.57, k + 1));
        iron.quad(hi0, hi1, r1, r0, Vec3::Y);
        // The bowl's floor and the coals heaped in it.
        iron.tri(Vec3::Y * 0.18, lo1, lo0, Vec3::NEG_Y);
        coals.tri(
            Vec3::Y * 0.5,
            ring(0.36, 0.42, k),
            ring(0.36, 0.42, k + 1),
            Vec3::Y,
        );
    }
    for k in 0..3 {
        let a = k as f32 / 3.0 * TAU;
        let foot = Vec3::new(0.3 * a.cos(), 0.0, 0.3 * a.sin());
        let top = Vec3::new(0.18 * a.cos(), 0.3, 0.18 * a.sin());
        tube(&mut iron, foot, top, 0.025);
    }
    (iron.mesh(), coals.mesh())
}

/// A cloth grid `nu` × `nv` cells, `w` × `h` metres, laid flat in its frame:
/// a banner hangs down from its top edge (−Y), a pennant streams along +X.
fn cloth_mesh(nu: usize, nv: usize, w: f32, h: f32, pennant: bool) -> Mesh {
    let mut pos = Vec::new();
    let mut uv = Vec::new();
    for j in 0..=nv {
        for i in 0..=nu {
            let (u, v) = (i as f32 / nu as f32, j as f32 / nv as f32);
            pos.push(cloth_rest(u, v, w, h, pennant).to_array());
            uv.push([u, v]);
        }
    }
    let mut idx = Vec::new();
    let row = (nu + 1) as u32;
    for j in 0..nv as u32 {
        for i in 0..nu as u32 {
            let a = j * row + i;
            idx.extend_from_slice(&[a, a + row, a + 1, a + 1, a + row, a + row + 1]);
        }
    }
    let n = pos.len();
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 0.0, 1.0]; n]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_indices(Indices::U32(idx));
    m
}

/// A cloth's point at rest.
fn cloth_rest(u: f32, v: f32, w: f32, h: f32, pennant: bool) -> Vec3 {
    if pennant {
        Vec3::new(u * w, (0.5 - v) * h, 0.0)
    } else {
        Vec3::new((u - 0.5) * w, -v * h, 0.0)
    }
}

/// Where the wind has a cloth's point at `t`: a banner billows from its
/// top edge and is pushed by the wind on its face; a pennant snaps along
/// its length from the pole. `push` is the wind on the face, `gust` its
/// strength, both `0..=1`.
pub fn cloth_at(u: f32, v: f32, t: f32, phase: f32, push: f32, gust: f32, pennant: bool) -> Vec3 {
    if pennant {
        let (w, h) = (PENNANT_L, PENNANT_H);
        let amp = (0.06 + 0.16 * gust) * u;
        let wave = (t * (5.0 + 5.0 * gust) - u * 7.0 + phase).sin();
        let droop = (1.0 - gust) * 0.35 * u * u;
        let rest = cloth_rest(u, v, w, h, true);
        Vec3::new(
            rest.x * (1.0 - 0.06 * wave.abs()),
            rest.y - droop,
            amp * wave,
        )
    } else {
        let (w, h) = (BANNER_W, BANNER_H);
        let free = v.powf(1.4);
        let wave = (t * (1.6 + 1.6 * gust) + v * 2.6 - u * 1.3 + phase).sin();
        let flutter = (t * 6.1 + v * 7.0 + u * 3.0 + phase * 2.0).sin();
        let out = free * ((0.05 + 0.12 * gust) * wave + 0.025 * gust * flutter + 0.55 * push);
        let rest = cloth_rest(u, v, w, h, false);
        // Pushed out, the hem rises a little: the cloth keeps its length.
        Vec3::new(
            rest.x + 0.04 * free * (t * 1.1 + phase).sin(),
            rest.y + 0.3 * free * push * push,
            out,
        )
    }
}

/// The town's colours: two pillars and the broken ring in gilt on lapis,
/// a gilt border, and a swallowtail hem, tattered.
fn banner_image() -> Image {
    const W: u32 = 64;
    const H: u32 = 128;
    let gold = [214u8, 170, 82];
    let field = [24u8, 38, 78];
    let mut data = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let (u, v) = ((x as f32 + 0.5) / W as f32, (y as f32 + 0.5) / H as f32);
            let tatter = 0.035 * super::props::hash01(x / 2, 0xBA77);
            let hem = 1.0 - 0.13 * (1.0 - (2.0 * u - 1.0).abs()) - tatter;
            let i = ((y * W + x) * 4) as usize;
            if v > hem {
                continue;
            }
            let weave = 0.9 + 0.1 * super::props::hash01(x * 7 + y * 131, 0x5EED);
            let mut c = field.map(|c| (c as f32 * weave) as u8);
            let border =
                (3..=5).contains(&x) || (W - 6..=W - 4).contains(&x) || (3..=5).contains(&y);
            let (cx, cy) = (32.0f32, 54.0f32);
            let (dx, dy) = (x as f32 + 0.5 - cx, cy - (y as f32 + 0.5));
            let r = (dx * dx + dy * dy).sqrt();
            let deg = dy.atan2(dx).to_degrees().rem_euclid(360.0);
            let ring = (r - 15.0).abs() < 2.2 && !(222.0..=318.0).contains(&deg);
            let pillar = ((dx.abs() - 22.0).abs() < 2.4) && dy.abs() < 25.0;
            let cap = ((dx.abs() - 22.0).abs() < 3.4) && (dy - 25.5).abs() < 1.6;
            if border || ring || pillar || cap {
                c = gold.map(|c| (c as f32 * weave) as u8);
            }
            data[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    cloth_image(data, W, H)
}

/// A pennant: gilt, a lapis stripe down its length, cut to a point.
fn pennant_image() -> Image {
    const W: u32 = 64;
    const H: u32 = 32;
    let mut data = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let (u, v) = ((x as f32 + 0.5) / W as f32, (y as f32 + 0.5) / H as f32);
            let half = 0.5 * (1.0 - u).powf(0.85);
            if (v - 0.5).abs() > half {
                continue;
            }
            let c = if (v - 0.5).abs() < 0.11 * (1.0 - u * 0.6) {
                [34u8, 62, 140]
            } else {
                [206u8, 160, 66]
            };
            let i = ((y * W + x) * 4) as usize;
            data[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    cloth_image(data, W, H)
}

fn cloth_image(level0: Vec<u8>, w: u32, h: u32) -> Image {
    let mut img = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        level0,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    if let Some(l0) = img.data.as_ref() {
        let chain = mipmap::chain(l0, w, h, mipmap::Filter::Srgb);
        img.texture_descriptor.mip_level_count = mipmap::levels(w, h);
        img.data = Some(chain);
    }
    img.sampler = ImageSampler::linear();
    img
}

fn cloth_material(tex: Handle<Image>) -> StandardMaterial {
    StandardMaterial {
        base_color_texture: Some(tex),
        alpha_mode: AlphaMode::Mask(0.5),
        perceptual_roughness: 0.92,
        reflectance: super::fresnel::DIELECTRIC,
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

/// The gate beams' faces (`town::PARTS`' sign beams, centred 40.8 m out,
/// 1.2 m thick): the beam's middle and its outward axis.
const GATE_BEAMS: [([f32; 2], [f32; 2]); 4] = [
    ([0.0, 40.8], [0.0, 1.0]),
    ([0.0, -40.8], [0.0, -1.0]),
    ([40.8, 0.0], [1.0, 0.0]),
    ([-40.8, 0.0], [-1.0, 0.0]),
];
/// The beams' underside, metres.
const BEAM_FOOT: f32 = 6.2;

// ------------------------------------------------------------- systems ---

/// Build the yard's moving parts once per world.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    mut commands: Commands,
    world: Option<Res<WorldId>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    server: Res<AssetServer>,
    drawn: Query<(), With<TownFxRoot>>,
) {
    let Some(world) = world else { return };
    let t = world.haven.town;
    if !t.live || !drawn.is_empty() {
        return;
    }
    let tf = town::transform(&t);
    commands.spawn((TownFxRoot, WorldEntity, Transform::IDENTITY));
    let at = |p: Vec3| tf.transform_point(p);
    let steel = materials.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.12, 0.12),
        metallic: 0.6,
        perceptual_roughness: 0.6,
        ..default()
    });

    // Festoons.
    let (bulbs, bunting, wires) = festoon_meshes();
    commands.spawn((
        WorldEntity,
        Mesh3d(meshes.add(wires)),
        MeshMaterial3d(steel.clone()),
        tf,
        NotShadowCaster,
    ));
    commands.spawn((
        WorldEntity,
        Mesh3d(meshes.add(bunting)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.9,
            reflectance: super::fresnel::DIELECTRIC,
            double_sided: true,
            cull_mode: None,
            ..default()
        })),
        tf,
        NotShadowCaster,
    ));
    let bulb_mats: [Handle<StandardMaterial>; PHASES] = std::array::from_fn(|_| {
        materials.add(StandardMaterial {
            base_color: Color::linear_rgb(BULB_DAY, BULB_DAY, BULB_DAY),
            unlit: true,
            ..default()
        })
    });
    for (k, mesh) in bulbs.into_iter().enumerate() {
        commands.spawn((
            WorldEntity,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(bulb_mats[k].clone()),
            tf,
            NotShadowCaster,
        ));
    }

    // Braziers.
    let iron = materials.add(StandardMaterial {
        base_color: Color::srgb(0.07, 0.06, 0.055),
        metallic: 0.5,
        perceptual_roughness: 0.75,
        ..default()
    });
    let coals_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.05, 0.03, 0.02),
        emissive: LinearRgba::rgb(4.0, 1.1, 0.15),
        perceptual_roughness: 1.0,
        ..default()
    });
    let (bowl, coals) = brazier_meshes();
    let (bowl, coals) = (meshes.add(bowl), meshes.add(coals));
    for (i, [x, z]) in BRAZIERS.into_iter().enumerate() {
        let foot = at(Vec3::new(x, POST_TOP, z));
        commands.spawn((
            WorldEntity,
            Mesh3d(bowl.clone()),
            MeshMaterial3d(iron.clone()),
            Transform::from_translation(foot),
        ));
        commands.spawn((
            WorldEntity,
            Coals,
            Mesh3d(coals.clone()),
            MeshMaterial3d(coals_mat.clone()),
            Transform::from_translation(foot),
            NotShadowCaster,
        ));
        commands.spawn((
            WorldEntity,
            Brazier {
                phase: i as f32 * 1.7,
            },
            FireFx {
                flames: true,
                flame_dy: -0.05,
                smoke_dy: 0.7,
                scale: 0.8,
                tongues: Tongues::Pit,
            },
            PointLight {
                color: BRAZIER_COLOR,
                intensity: BRAZIER_LUMENS,
                range: BRAZIER_RANGE_M,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_translation(foot + Vec3::Y * 0.6),
        ));
    }

    // Searchlights.
    let beam_tex = images.add(gate_fx::beam_image());
    let beam_mat = materials.add(gate_fx::light_material(beam_tex, [0.0; 3]));
    let beam_mesh = meshes.add(gate_fx::beam_mesh(
        SEARCH_LEN_M,
        SEARCH_W0,
        SEARCH_W1,
        0.32,
        1.6,
    ));
    let (drum, drum_material) = super::mobs::model(&server, SEARCHLIGHT_GLB);
    let lens = meshes.add(Cylinder::new(LENS_R, 0.01));
    let lens_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.9, 0.9, 0.85),
        emissive: LinearRgba::rgb(2.0, 2.0, 1.8),
        ..default()
    });
    for (k, p) in SEARCH_AT.into_iter().enumerate() {
        let local = Vec3::from_array(p);
        let at_w = at(local);
        let out = tf.rotation * Vec3::new(local.x, 0.0, local.z).normalize_or(Vec3::Z);
        let yaw0 = out.z.atan2(out.x);
        commands.spawn((
            WorldEntity,
            Searchlight {
                k,
                at: at_w,
                yaw0,
                phase: k as f32 * 1.9,
            },
            SpotLight {
                color: Color::srgb(0.95, 0.96, 0.9),
                intensity: 0.0,
                range: SEARCH_RANGE_M,
                outer_angle: SEARCH_CONE,
                inner_angle: SEARCH_CONE * 0.6,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_translation(at_w),
        ));
        let head = commands
            .spawn((
                WorldEntity,
                SearchHead(k),
                Mesh3d(drum.clone()),
                MeshMaterial3d(drum_material.clone()),
                Transform::from_translation(at_w),
            ))
            .id();
        commands.spawn((
            ChildOf(head),
            Mesh3d(lens.clone()),
            MeshMaterial3d(lens_mat.clone()),
            Transform::from_xyz(0.0, LENS_Y, 0.0),
            NotShadowCaster,
        ));
        commands.spawn((
            WorldEntity,
            SearchBeam(k),
            Beam {
                from: at_w,
                dir: out,
            },
            Mesh3d(beam_mesh.clone()),
            MeshMaterial3d(beam_mat.clone()),
            Transform::from_translation(at_w),
            Visibility::Hidden,
            // Turned after the visibility pass (`gate_fx::aim`).
            bevy::camera::visibility::NoFrustumCulling,
            NotShadowCaster,
        ));
    }

    // Banners on both faces of every gate beam, and pennants off the
    // towers.
    let banner_mat = materials.add(cloth_material(images.add(banner_image())));
    let pennant_mat = materials.add(cloth_material(images.add(pennant_image())));
    let mut phase = 0.0;
    for ([cx, cz], [nx, nz]) in GATE_BEAMS {
        let n = Vec3::new(nx, 0.0, nz);
        let along = Vec3::Y.cross(n);
        for face in [1.0f32, -1.0] {
            let normal = n * face;
            for side in [-3.3f32, 3.3] {
                let local = Vec3::new(cx, BEAM_FOOT, cz) + normal * 0.66 + along * side;
                let rot = tf.rotation
                    * Quat::from_mat3(&Mat3::from_cols(Vec3::Y.cross(normal), Vec3::Y, normal));
                let mesh = meshes.add(cloth_mesh(BANNER_NU, BANNER_NV, BANNER_W, BANNER_H, false));
                phase += 1.37;
                commands.spawn((
                    WorldEntity,
                    Cloth {
                        mesh: mesh.clone(),
                        pennant: false,
                        phase,
                        normal: tf.rotation * normal,
                    },
                    Mesh3d(mesh),
                    MeshMaterial3d(banner_mat.clone()),
                    Transform::from_translation(at(local)).with_rotation(rot),
                    NotShadowCaster,
                ));
            }
        }
    }
    for p in SEARCH_AT {
        let corner = Vec3::new(p[0], 0.0, p[2]).normalize_or(Vec3::X) * 2.4;
        let foot = Vec3::new(p[0].signum() * 35.0, 9.9, p[2].signum() * 35.0) - corner;
        let mut wood = Soup::default();
        tube(
            &mut wood,
            Vec3::ZERO,
            Vec3::Y * (POLE_TOP - foot.y + 0.05),
            0.035,
        );
        commands.spawn((
            WorldEntity,
            Mesh3d(meshes.add(wood.mesh())),
            MeshMaterial3d(steel.clone()),
            Transform::from_translation(at(foot)),
        ));
        let mesh = meshes.add(cloth_mesh(
            PENNANT_NU, PENNANT_NV, PENNANT_L, PENNANT_H, true,
        ));
        phase += 2.1;
        let hang = at(Vec3::new(foot.x, POLE_TOP - PENNANT_H * 0.5, foot.z));
        commands.spawn((
            WorldEntity,
            Cloth {
                mesh: mesh.clone(),
                pennant: true,
                phase,
                normal: Vec3::ZERO,
            },
            Mesh3d(mesh),
            MeshMaterial3d(pennant_mat.clone()),
            Transform::from_translation(hang),
            NotShadowCaster,
        ));
    }

    // The canteen's stovepipes.
    let pipe = meshes.add(Cylinder::new(0.1, PIPE_H));
    let cap = meshes.add(Cylinder::new(0.2, 0.05));
    for [x, z] in STOVEPIPES {
        let foot = at(Vec3::new(x, 2.6, z));
        commands.spawn((
            WorldEntity,
            Mesh3d(pipe.clone()),
            MeshMaterial3d(iron.clone()),
            Transform::from_translation(foot + Vec3::Y * PIPE_H * 0.5),
        ));
        commands.spawn((
            WorldEntity,
            Mesh3d(cap.clone()),
            MeshMaterial3d(iron.clone()),
            Transform::from_translation(foot + Vec3::Y * (PIPE_H + 0.12)),
        ));
    }

    // The bad junction's flash.
    commands.spawn((
        WorldEntity,
        ShortLight,
        PointLight {
            color: Color::srgb(0.75, 0.85, 1.0),
            intensity: 0.0,
            range: 9.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_translation(at(Vec3::from_array(SHORT_AT))),
    ));

    commands.insert_resource(TownFx {
        tf,
        bulb_mats,
        coals_mat,
        beam_mat,
        clock: 0.0,
        rng: (world.seed as u32 | 1) ^ 0x7A11_C0DE,
        next_short: 2.0,
        short_flash: 0.0,
        heading: 0.0,
        lit_at: None,
        beam_k: None,
    });
    let n: usize = FESTOONS.len();
    info!(
        "town fx: {n} festoons, {} braziers, searchlights and banners up",
        BRAZIERS.len()
    );
}

/// Twinkle the bulbs, flicker the fires, sweep the searchlights, move the
/// cloth in the wind and short the junction.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn animate(
    time: Res<Time>,
    eye: Res<Eye>,
    gain: Res<FlameGain>,
    glow: Option<Res<TownGlow>>,
    weather: Option<Res<WeatherNow>>,
    state: Option<ResMut<TownFx>>,
    mut fx: ResMut<Fx>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut braziers: Query<(&Brazier, &mut PointLight), Without<ShortLight>>,
    mut lights: Query<
        (&Searchlight, &mut SpotLight, &mut Transform),
        (Without<SearchHead>, Without<Cloth>),
    >,
    mut heads: Query<(&SearchHead, &mut Transform), (Without<Searchlight>, Without<Cloth>)>,
    mut beams: Query<(&SearchBeam, &mut Beam, &mut Visibility)>,
    mut cloth: Query<(&Cloth, &mut Transform), (Without<Searchlight>, Without<SearchHead>)>,
    mut short: Query<&mut PointLight, (With<ShortLight>, Without<Brazier>)>,
) {
    let (Some(mut st), Some(glow)) = (state, glow) else {
        return;
    };
    let dt = time.delta_secs().min(0.1);
    if dt <= 0.0 {
        return;
    }
    st.clock += dt;
    let t = st.clock;
    let night = glow.night();
    let g = gain.0;
    let near = eye.pos.distance(st.tf.translation) < CLOTH_M;

    // Bulbs: each phase dips in turn, so the strings shimmer.
    let level = BULB_DAY + (BULB_NIGHT - BULB_DAY) * night;
    let lit_key = (level * 100.0).round() + (t / 0.05).floor() * 1000.0;
    if st.lit_at != Some(lit_key) {
        st.lit_at = Some(lit_key);
        for (k, h) in st.bulb_mats.iter().enumerate() {
            let p = (t / TWINKLE_S + k as f32 / PHASES as f32).fract();
            let dip = 1.0 - 0.45 * night * (1.0 - (p * TAU).cos()).powi(2) * 0.25;
            if let Some(m) = materials.get_mut(h) {
                let v = level * dip;
                m.base_color = Color::linear_rgb(v, v, v);
            }
        }
        // The coals breathe with the fire.
        let c = 0.75 + 0.25 * (t * 2.3).sin() * (t * 3.7).sin();
        if let Some(m) = materials.get_mut(&st.coals_mat) {
            m.emissive = LinearRgba::rgb(4.0 * c, 1.1 * c, 0.15 * c);
        }
    }

    // Braziers flicker: two beats and a stutter.
    for (b, mut l) in &mut braziers {
        let f = 0.8
            + 0.1 * (t * 13.0 + b.phase).sin()
            + 0.07 * (t * 27.3 + b.phase * 2.0).sin()
            + 0.03 * (t * 51.0 + b.phase * 3.0).sin();
        l.intensity = BRAZIER_LUMENS * g * f;
    }

    // Searchlights sweep after dusk.
    let on = night > 0.05;
    let mut dirs = [Vec3::Z; SEARCH_AT.len()];
    for (s, mut spot, mut tf) in &mut lights {
        let yaw = s.yaw0 + SWEEP_HALF * (t * TAU / SWEEP_S + s.phase).sin();
        let tilt = TILT + NOD * (t * TAU / (SWEEP_S * 0.53) + s.phase * 1.3).sin();
        let dir = Vec3::new(yaw.cos() * tilt.cos(), tilt.sin(), yaw.sin() * tilt.cos());
        dirs[s.k] = dir;
        let want = if on { SEARCH_LUMENS * g * night } else { 0.0 };
        if spot.intensity != want {
            spot.intensity = want;
        }
        *tf = Transform::from_translation(s.at).looking_to(dir, Vec3::Y);
    }
    for (h, mut tf) in &mut heads {
        let dir = dirs[h.0];
        tf.rotation = Quat::from_rotation_arc(Vec3::Y, dir);
    }
    for (b, mut beam, mut vis) in &mut beams {
        // From the lens, not the drum's middle.
        beam.dir = dirs[b.0];
        let want = if on {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
    }
    // Only on a visible change: a material taken mutably is re-prepared.
    let k = night;
    if on && st.beam_k.is_none_or(|a| (a - k).abs() > 0.004 * k.max(0.1)) {
        st.beam_k = Some(k);
        if let Some(m) = materials.get_mut(&st.beam_mat) {
            m.base_color =
                Color::linear_rgb(SEARCH_RGB[0] * k, SEARCH_RGB[1] * k, SEARCH_RGB[2] * k);
        }
    }

    // Cloth in the wind.
    if near {
        let w = weather.map(|w| *w).unwrap_or_default();
        let wind_dir = if w.wind_dir.length_squared() > 0.5 {
            Vec3::new(w.wind_dir.x, 0.0, w.wind_dir.y)
        } else {
            Vec3::new(0.8, 0.0, 0.6)
        };
        let gust = (0.25 + 0.75 * w.wind) * (0.85 + 0.15 * (t * 0.37).sin());
        let want = wind_dir.z.atan2(wind_dir.x);
        let mut d = (want - st.heading).rem_euclid(TAU);
        if d > std::f32::consts::PI {
            d -= TAU;
        }
        st.heading += d * (1.0 - (-0.8 * dt).exp());
        let heading = st.heading;
        for (c, mut tf) in &mut cloth {
            let (nu, nv) = if c.pennant {
                (PENNANT_NU, PENNANT_NV)
            } else {
                (BANNER_NU, BANNER_NV)
            };
            let push = if c.pennant {
                tf.rotation = Quat::from_rotation_y(-heading);
                0.0
            } else {
                (wind_dir.dot(c.normal) * gust).max(0.0) * 0.6
            };
            let Some(mesh) = meshes.get_mut(&c.mesh) else {
                continue;
            };
            write_cloth(mesh, nu, nv, t, c.phase, push, gust, c.pennant);
        }
    }

    // Cooking smoke off the canteen's pipes.
    if near {
        for [x, z] in STOVEPIPES {
            let top = st.tf.transform_point(Vec3::new(x, 2.6 + PIPE_H + 0.05, z));
            let x = COOK_SMOKE_RATE * dt;
            let n = x.floor() as usize + usize::from(st.roll() < x - x.floor());
            emit(&mut fx.soft, Layer::Smoke, n, top, Vec3::Y, Matter::Stone);
        }
    }

    // The bad junction.
    st.next_short -= dt;
    st.short_flash = (st.short_flash - dt).max(0.0);
    if st.next_short <= 0.0 {
        st.next_short = SHORT_S.0 + (SHORT_S.1 - SHORT_S.0) * st.roll();
        st.short_flash = 0.35;
        if near {
            let from = st.tf.transform_point(Vec3::from_array(SHORT_AT));
            let floor = st.tf.translation.y;
            let n = 16 + (st.roll() * 18.0) as usize;
            for _ in 0..n {
                let v = Vec3::new(st.signed(), st.roll() * 0.6 - 0.1, st.signed()) * 2.2;
                let life = 0.7 + 0.9 * st.roll();
                fx.glow.spawn(Particle {
                    left: life,
                    life,
                    pos: from,
                    vel: v,
                    size0: 0.018,
                    size1: 0.008,
                    c0: [6.0, 4.6, 2.6, 1.0],
                    c1: [2.2, 0.5, 0.08, 0.0],
                    gravity: 9.8,
                    drag: 0.4,
                    cell: atlas::GLOW,
                    orient: Orient::Stretch,
                    stretch: 0.03,
                    plane_p: Vec3::new(from.x, floor + 0.02, from.z),
                    plane_n: Vec3::Y,
                    ..Particle::default()
                });
            }
            for _ in 0..3 {
                let life = 0.12;
                fx.glow.spawn(Particle {
                    left: life,
                    life,
                    pos: from,
                    size0: 0.35,
                    size1: 0.1,
                    c0: [2.5, 3.0, 4.0, 1.0],
                    c1: [0.5, 0.7, 1.2, 0.0],
                    roll: st.roll() * TAU,
                    cell: atlas::STAR,
                    ..Particle::default()
                });
            }
        }
    }
    if let Ok(mut l) = short.single_mut() {
        // Three stutters as it arcs.
        let k = if st.short_flash > 0.0 && ((st.short_flash * 40.0) as u32) % 3 != 1 {
            st.short_flash / 0.35
        } else {
            0.0
        };
        let want = 2_500.0 * g.max(20.0) * k;
        if l.intensity != want {
            l.intensity = want;
        }
    }
}

/// Write a cloth's pose at `t` into its mesh, with normals off its grid.
#[allow(clippy::too_many_arguments)]
fn write_cloth(
    mesh: &mut Mesh,
    nu: usize,
    nv: usize,
    t: f32,
    phase: f32,
    push: f32,
    gust: f32,
    pennant: bool,
) {
    let row = nu + 1;
    let mut grid = [Vec3::ZERO; CLOTH_VERTS];
    for j in 0..=nv {
        for i in 0..=nu {
            let (u, v) = (i as f32 / nu as f32, j as f32 / nv as f32);
            grid[j * row + i] = cloth_at(u, v, t, phase, push, gust, pennant);
        }
    }
    for (attr, values) in mesh.attributes_mut() {
        let id = attr.id;
        match values {
            VertexAttributeValues::Float32x3(v) if id == Mesh::ATTRIBUTE_POSITION.id => {
                for (k, p) in v.iter_mut().enumerate().take(row * (nv + 1)) {
                    *p = grid[k].to_array();
                }
            }
            VertexAttributeValues::Float32x3(v) if id == Mesh::ATTRIBUTE_NORMAL.id => {
                for j in 0..=nv {
                    for i in 0..=nu {
                        let at = |i: usize, j: usize| grid[j.min(nv) * row + i.min(nu)];
                        let du = at(i + 1, j) - at(i.saturating_sub(1), j);
                        let dv = at(i, j + 1) - at(i, j.saturating_sub(1));
                        let n = dv.cross(du).normalize_or(Vec3::Z);
                        v[j * row + i] = n.to_array();
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn festoons_clear_heads_and_roofs() {
        // Every bulb hangs above the tallest body's reach and above the
        // kiosks' awnings it crosses.
        for &(a, b, sag) in &FESTOONS {
            let (a, b) = (Vec3::from_array(a), Vec3::from_array(b));
            for i in 0..=40 {
                let p = sling(a, b, sag, i as f32 / 40.0);
                assert!(p.y > 5.5, "a string sags to {p:?}");
            }
        }
    }

    #[test]
    fn a_banner_hangs_from_its_top_edge() {
        for t in [0.0, 1.3, 7.7] {
            let top = cloth_at(0.3, 0.0, t, 0.4, 0.5, 1.0, false);
            assert!(
                top.z.abs() < 1e-5 && top.y.abs() < 1e-5,
                "the top edge stays nailed"
            );
            let hem = cloth_at(0.3, 1.0, t, 0.4, 0.0, 0.0, false);
            assert!(hem.y < -BANNER_H * 0.9);
        }
        // A pennant's hoist stays on the pole.
        let hoist = cloth_at(0.0, 0.5, 3.0, 0.0, 0.0, 1.0, true);
        assert!(hoist.x.abs() < 1e-5 && hoist.z.abs() < 1e-5);
    }

    #[test]
    fn braziers_sit_on_the_gate_posts() {
        for [x, z] in BRAZIERS {
            let held = sim_core::town::PARTS.iter().any(|p| {
                let b = p.b;
                (b[4] - POST_TOP).abs() < 1e-3 && b[0] < x && x < b[3] && b[2] < z && z < b[5]
            });
            assert!(held, "no post under the brazier at ({x}, {z})");
        }
    }
}
