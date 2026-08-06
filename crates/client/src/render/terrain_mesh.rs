//! The ground: a heightfield meshed straight out of `sim_core::terrain`.
//!
//! This is meshing, not design. `terrain::height` is a pure function of the
//! seed, the server sims on it, the browser drew it, and the three agree
//! bit-for-bit — so the only questions here are resolution, streaming budget
//! and what the vertices carry.
//!
//! The scheme is the browser's, because it was measured rather than guessed
//! (`web/src/terrain.js`): one far mesh of the whole island at 8 m built
//! once, plus a ring of 64 m near chunks at 1 m streamed around the player,
//! the far mesh dropped 0.15 m so the near↔far boundary cannot z-fight.
//! Adjacent near chunks share exact edge heights — they sample the same
//! function at the same coordinates — so same-LOD seams cannot crack.
//!
//! **Normals are analytic, never from the triangulation.** A heightfield that
//! takes its normal from its own triangles renders its own tessellation:
//! every quad's diagonal shows up as a shading crease that moves when the LOD
//! does. `ci/bump_basis.mjs` holds that arithmetic as a gate for the browser
//! and the property it asserts — the world-XZ gradient is continuous across a
//! triangle edge — is a property of central differences, which is what this
//! takes. The gate is language-agnostic; only its caller is JS.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sim_core::terrain::{self, SEA_LEVEL};

use super::textures::GroundMaps;
use super::{Eye, WorldId};

/// Near-chunk edge, metres.
pub const CHUNK_M: f32 = 64.0;
/// Vertices per near-chunk side: 64 m at 1 m plus the shared edge.
pub const NEAR_N: usize = 65;
/// Near ring radius in chunks — a 5×5 ring, 160 m to the corner.
pub const NEAR_RADIUS: i32 = 2;
/// Vertices per far-mesh side: 2048 m at 8 m plus the edge.
pub const FAR_N: usize = 257;
/// Far-mesh sample step, metres.
pub const FAR_STEP: f32 = 8.0;
/// How far the far mesh sits below the near ring so the boundary cannot
/// z-fight, metres.
pub const FAR_DROP: f32 = 0.15;
/// Chunks built per frame, and chunks torn down per frame. Stream-in AND
/// stream-out are budgeted: the teardown spike is the half everyone forgets
/// (`CLAUDE.md` traps).
pub const CHUNK_BUILDS_PER_FRAME: usize = 1;

/// The four ground identities' albedo, LINEAR, in the order `terrain::splat`
/// returns them: sand · grass · forest litter · rock.
///
/// Derived from `ART.md` §3's measured surfaces — the hue and saturation are
/// the reference's, the value is the *reflectance* that produces the
/// reference's lit luma under a midday sun, not the lit luma itself. All four
/// sit inside `ALBEDO_LUMA_BAND = [0.05, 0.55]`, and the two facts §3 states
/// plainly are visible in the table: granite is warm grey and roughly 2×
/// turf's value, and grass is the darkest thing on the island.
pub const GROUND_ALBEDO: [[f32; 3]; 4] = [
    // beach sand — hue 42°, sat 10%
    [0.30, 0.272, 0.225],
    // grass — hue 63–74°, sat 29–33%. The darkest identity, deliberately.
    // Blue lifted off the first capture: 42% measured near-band saturation
    // against the reference's 33%, and a green with no blue in it is where
    // most of that came from.
    [0.095, 0.116, 0.068],
    // forest litter — red-leaning, needles and mud
    [0.093, 0.068, 0.046],
    // granite — hue 35–43° warm grey, ~2× turf
    [0.235, 0.222, 0.198],
];

/// `1 / linear mean` of `rock_albedo.jpg`, per channel — the mean-placing
/// correction of `ART.md` §7, measured off the shipped file rather than
/// guessed. Its span (max/min) is 1.054, i.e. the correction stretches the
/// source's colour deviation by 5%, which is what the ×1 rule permits.
pub const ROCK_GAIN: [f32; 3] = [3.659, 3.713, 3.855];

/// One built ground chunk, so teardown can find it.
#[derive(Component)]
pub struct Chunk(pub i32, pub i32);

/// The static far mesh and the water plane, which never stream.
#[derive(Component)]
pub struct Static;

/// What the ring has built. A `HashMap` here is fine and would not be in
/// `sim-core`: the no-`HashMap`-iteration wall is about the deterministic
/// tick, and nothing in this file feeds one.
#[derive(Resource, Default)]
pub struct Ring {
    built: HashMap<(i32, i32), Entity>,
    ground: Option<Handle<StandardMaterial>>,
    far_done: bool,
}

/// Chunks in a full near ring — what `is_full` is asserting against, and the
/// number a capture settles on rather than on a clock.
pub const RING_CHUNKS: usize = ((2 * NEAR_RADIUS + 1) * (2 * NEAR_RADIUS + 1)) as usize;

impl Ring {
    pub fn len(&self) -> usize {
        self.built.len()
    }
    pub fn is_empty(&self) -> bool {
        self.built.is_empty()
    }
    /// The far mesh is up and every near chunk is resident.
    pub fn is_full(&self) -> bool {
        self.far_done && self.built.len() >= RING_CHUNKS
    }
}

/// The ground's one material. Shared by every chunk so Bevy batches them into
/// one draw per pipeline; the identity variation rides the vertices and the
/// near-field grain rides the photograph.
///
/// **One map set, not four, and WHICH one was a measurement rather than a
/// taste.** A `StandardMaterial` has one base-colour slot, so the four
/// identities `terrain::splat` resolves cannot each carry their own
/// photograph here. The first cut picked `grass` because `MANIFEST.md`
/// records it owning ~99% of the near ring — and the capture measured
/// near-band saturation falling 32.5% → 15.0% against a reference of 33.2%,
/// because `base_color_texture` MULTIPLIES the authored colour by the
/// photograph's own colour, which is `ART.md` §7's named failure: a modifier
/// that must set a colour multiplies the surface's mean-1 LUMINANCE field, not
/// its chroma.
///
/// The fix §7 prescribes is a per-channel gain that places the source's mean,
/// bounded by the rule that **a sourced map's colour deviation may not be
/// stretched by more than ×1 by that correction**. Measured over the four
/// ground sources (linear means, gain = 1/mean, span = max gain / min gain):
///
/// | source | linear mean rgb | gain span | albedo sd |
/// |---|---|---|---|
/// | grass | 0.291 0.249 0.119 | **2.454** | 0.0743 |
/// | sand | 0.228 0.174 0.110 | **2.073** | 0.0480 |
/// | litter | 0.139 0.099 0.039 | **3.586** | 0.0527 |
/// | rock | 0.273 0.269 0.259 | **1.054** | 0.0924 |
///
/// Only `rock` clears the rule, and it is also the source carrying the most
/// detail. That is not a coincidence: `MANIFEST.md` records `rock` being
/// chosen by scoring 74 CC0 candidates on exactly gain span and albedo sd.
/// So the ground's grain is granite's, mean-corrected, and its colour is
/// entirely the authored splat's — which is the construction §7 asks for.
///
/// Four maps blended by the splat weights needs a custom material, and in
/// 0.18 `StandardMaterial` is `#[bindless(index_table(range(0..31)))]`, so
/// that is a slice with a shader in it rather than a knob. `RENDER.md` §8.
fn ground_material(
    materials: &mut Assets<StandardMaterial>,
    maps: &GroundMaps,
) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        // White, because the albedo IS the vertex colour times the map. A
        // base colour here would multiply every identity by one tint and
        // flatten the splat.
        // The reciprocal of the source's own linear mean, so the delivered
        // mean is the authored vertex colour and the map contributes only its
        // relief. Values above 1 are correct and intended here — `LinearRgba`
        // is unclamped, and a gain under 1 would be darkening rather than
        // normalizing.
        base_color: Color::linear_rgb(
            super::textures::GROUND_DETAIL_GAIN,
            super::textures::GROUND_DETAIL_GAIN,
            super::textures::GROUND_DETAIL_GAIN,
        ),
        base_color_texture: Some(maps.detail.clone()),
        normal_map_texture: Some(maps.grass.normal.clone()),
        // **The roughness map is deliberately NOT wired.**
        // `metallic_roughness_texture` is a glTF-packed ORM slot — G is
        // roughness and **B is metallic** — and these sources are GREYSCALE
        // roughness jpgs, so B would carry the roughness value into the
        // metallic channel and make the ground a half-metal. Wiring it needs
        // the maps packed into one ORM texture (an asset step), not a slot
        // assignment. That is a reason from the format, not a measurement:
        // when this was first blamed for a measured saturation crash the real
        // cause was elsewhere and the log said so.
        perceptual_roughness: 0.92,
        metallic: 0.0,
        reflectance: 0.18,
        ..default()
    })
}

/// Build one heightfield patch. `n` vertices a side, `step` metres apart,
/// origin at its minimum corner.
pub fn heightfield(seed: u64, ox: f32, oz: f32, n: usize, step: f32, drop: f32) -> Mesh {
    let count = n * n;
    let mut positions = Vec::with_capacity(count);
    let mut normals = Vec::with_capacity(count);
    let mut colors = Vec::with_capacity(count);
    let mut uvs = Vec::with_capacity(count);

    // The central-difference arm. Half a step keeps the gradient local to the
    // quad it shades without sampling inside its own vertex.
    let d = (step * 0.5).max(0.5);

    for iz in 0..n {
        for ix in 0..n {
            let x = ox + ix as f32 * step;
            let z = oz + iz as f32 * step;
            let y = terrain::height(seed, x, z);
            positions.push([x, y - drop, z]);

            // Analytic normal: the surface gradient, not the triangulation.
            let hx = terrain::height(seed, x + d, z) - terrain::height(seed, x - d, z);
            let hz = terrain::height(seed, x, z + d) - terrain::height(seed, x, z - d);
            let n_v = Vec3::new(-hx, 2.0 * d, -hz).normalize();
            normals.push([n_v.x, n_v.y, n_v.z]);

            // The identity mix, from the same `splat` the browser's material
            // was fed by — four weights summing to ~255.
            let w = terrain::splat(seed, x, z);
            let inv = 1.0 / 255.0;
            let mut c = [0.0f32; 3];
            for (k, wk) in w.iter().enumerate() {
                let f = *wk as f32 * inv;
                for ch in 0..3 {
                    c[ch] += GROUND_ALBEDO[k][ch] * f;
                }
            }
            // Rule 1: no surface may be one flat value. This is the MACRO
            // break-up (0.5–1 m) — the near ring's vertices are 1 m apart, so
            // one hash per vertex is exactly that scale. The near-field grain
            // under 5 cm is a texture's job and belongs to the materials
            // slice; this is the half geometry can carry.
            let v = 0.88 + 0.24 * super::props::hash01(x.to_bits(), z.to_bits());
            colors.push([c[0] * v, c[1] * v, c[2] * v, 1.0]);
            uvs.push([x * 0.25, z * 0.25]);
        }
    }

    let mut indices = Vec::with_capacity((n - 1) * (n - 1) * 6);
    for iz in 0..n - 1 {
        for ix in 0..n - 1 {
            let a = (iz * n + ix) as u32;
            let b = a + 1;
            let c = a + n as u32;
            let dd = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, dd]);
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    // Tangents, because a normal map without them is not a normal map.
    // Bevy's PBR shader needs `ATTRIBUTE_TANGENT` to build the tangent frame;
    // without it the map is ignored and the surface silently stays flat —
    // the failure that looks like "the texture did not load".
    mesh.generate_tangents()
        .expect("terrain mesh has UVs and normals");
    mesh
}

/// The sea. One translucent plane at `SEA_LEVEL`; nothing simulates
/// (`TERRAIN.md` §4).
pub fn setup_water(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let size = terrain::ISLAND_SIZE * 1.5;
    commands.spawn((
        Static,
        Mesh3d(meshes.add(Plane3d::default().mesh().size(size, size))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.055, 0.14, 0.18, 0.86),
            perceptual_roughness: 0.06,
            reflectance: 0.55,
            alpha_mode: AlphaMode::Blend,
            ..default()
        })),
        Transform::from_xyz(
            terrain::ISLAND_SIZE * 0.5,
            SEA_LEVEL,
            terrain::ISLAND_SIZE * 0.5,
        ),
    ));
}

/// Stream the near ring, and build the far mesh once.
pub fn stream(
    mut commands: Commands,
    mut ring: ResMut<Ring>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    world: Res<WorldId>,
    maps: Res<GroundMaps>,
    eye: Res<Eye>,
) {
    let ground = ring
        .ground
        .get_or_insert_with(|| ground_material(&mut materials, &maps))
        .clone();

    // The far mesh, once. It is 66 k vertices and it is the whole island, so
    // it is built on the first streaming frame rather than in `Startup` —
    // that keeps the window up and the session pumping while it happens.
    if !ring.far_done {
        ring.far_done = true;
        let mesh = heightfield(world.seed, 0.0, 0.0, FAR_N, FAR_STEP, FAR_DROP);
        commands.spawn((
            Static,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(ground.clone()),
            Transform::IDENTITY,
        ));
        return;
    }

    let cx = (eye.pos.x / CHUNK_M).floor() as i32;
    let cz = (eye.pos.z / CHUNK_M).floor() as i32;

    // Stream out first: a ring that grows before it shrinks peaks at both
    // rings resident, which is the teardown spike in its other form.
    let mut dropped = 0usize;
    ring.built.retain(|(bx, bz), e| {
        if dropped >= CHUNK_BUILDS_PER_FRAME
            || ((*bx - cx).abs() <= NEAR_RADIUS && (*bz - cz).abs() <= NEAR_RADIUS)
        {
            return true;
        }
        dropped += 1;
        commands.entity(*e).despawn();
        false
    });

    let mut built = 0usize;
    for dz in -NEAR_RADIUS..=NEAR_RADIUS {
        for dx in -NEAR_RADIUS..=NEAR_RADIUS {
            if built >= CHUNK_BUILDS_PER_FRAME {
                return;
            }
            let key = (cx + dx, cz + dz);
            if ring.built.contains_key(&key) {
                continue;
            }
            let ox = key.0 as f32 * CHUNK_M;
            let oz = key.1 as f32 * CHUNK_M;
            let step = CHUNK_M / (NEAR_N - 1) as f32;
            let mesh = heightfield(world.seed, ox, oz, NEAR_N, step, 0.0);
            let e = commands
                .spawn((
                    Chunk(key.0, key.1),
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(ground.clone()),
                    Transform::IDENTITY,
                ))
                .id();
            ring.built.insert(key, e);
            built += 1;
        }
    }
}
