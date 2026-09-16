//! The inland freight depot, drawn from the sim's exact part envelopes.
//!
//! The kit adds oriented photographed cladding, plinths, door hardware,
//! clearance paint and signage inside those envelopes. It never adds a
//! second layout, collision shape, loot prop or light.

use bevy::prelude::*;
use sim_core::depot::{self as layout, Part, PartKind};
use sim_core::terrain::Waystation;

use super::props::Soup;
use super::textures::MapSet;
use super::{WorldEntity, WorldId};

/// Proposed art defaults, DECISIONS.md depot visual kit v1.
pub const DEPOT_FACE_GRID_M: f32 = 1.0;
pub const DEPOT_WEATHER_CELL_M: f32 = 1.0;
pub const DEPOT_FRAME_BAY_M: f32 = 3.0;
pub const DEPOT_SIGN_WIDTH_FRACTION: f32 = 0.85;
pub const DEPOT_SIGN_HEIGHT_FRACTION: f32 = 0.6;
pub const DEPOT_GATE_BANDS: usize = 4;
pub const DEPOT_GATE_BAND_FRACTION: f32 = 0.4;
pub const DEPOT_WEATHER_STRENGTH: f32 = 0.12;
pub const DEPOT_CONTACT_HEIGHT_M: f32 = 0.65;
pub const DEPOT_CONTACT_KEEP: f32 = 0.76;
pub const DEPOT_PLINTH_M: f32 = 0.85;
pub const DEPOT_FRAME_M: f32 = 0.08;
pub const DEPOT_CONCRETE_TILE_M: f32 = 3.0;
pub const DEPOT_SHEET_TILES_PER_M: f32 = 0.55;
pub const DEPOT_YARD_TILE_M: f32 = 1.0;
pub const DEPOT_SHEET_TINT: [f32; 3] = [0.62, 0.78, 0.80];
pub const DEPOT_ROOF_TINT: [f32; 3] = [0.62, 0.66, 0.68];
pub const DEPOT_STEEL_TINT: [f32; 3] = [0.37, 0.42, 0.43];
pub const DEPOT_YARD_TINT: [f32; 3] = [0.56, 0.59, 0.62];
pub const DEPOT_CARGO_TINTS: [[f32; 3]; 3] =
    [[0.84, 0.57, 0.40], [0.47, 0.62, 0.59], [0.64, 0.65, 0.61]];
pub const DEPOT_FENCE_TINT: [f32; 3] = [0.70, 0.73, 0.70];
pub const DEPOT_MONITOR_TINT: [f32; 3] = [0.76, 0.80, 0.80];
pub const DEPOT_SAFETY_TINT: [f32; 3] = [1.0, 0.76, 0.28];
/// Raster depth bias, not a physical displacement. The yard remains exactly
/// at the sim datum and painted trim stays on the collision envelope.
pub const DEPOT_SURFACE_DEPTH_BIAS: f32 = 1.0;
/// Dedicated raster bias for the coplanar yard/terrain draw. Bevy forwards
/// this integer to wgpu's constant depth bias; positive draws toward camera.
/// Biases 1 and 16 left stripes across independently transformed meshes.
/// 1024 is the proposed precision margin, not a physical height offset.
pub const DEPOT_YARD_DEPTH_BIAS: f32 = 1024.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Surface {
    Yard,
    Concrete,
    Sheet,
    Roof,
    Steel,
    Cargo,
    Paint,
    Timber,
}

pub const SURFACES: [Surface; 8] = [
    Surface::Yard,
    Surface::Concrete,
    Surface::Sheet,
    Surface::Roof,
    Surface::Steel,
    Surface::Cargo,
    Surface::Paint,
    Surface::Timber,
];

impl Surface {
    fn tiles(self) -> f32 {
        match self {
            Self::Yard => 1.0 / DEPOT_YARD_TILE_M,
            Self::Concrete | Self::Paint => 1.0 / DEPOT_CONCRETE_TILE_M,
            Self::Timber => super::structures::tier(sim_core::build::MAT_WOOD).tiles_per_m,
            _ => DEPOT_SHEET_TILES_PER_M,
        }
    }

    fn role(self) -> &'static str {
        match self {
            Self::Yard => "gravel",
            Self::Concrete | Self::Paint => "concrete",
            Self::Timber => "wood",
            _ => "metal",
        }
    }

    fn sheet(self) -> bool {
        matches!(self, Self::Sheet | Self::Roof | Self::Steel | Self::Cargo)
    }
}

#[derive(Component)]
pub struct DepotVisual;

fn kit() -> [Soup; SURFACES.len()] {
    std::array::from_fn(|_| Soup::default())
}

/// All materials are nonmetallic: oxidation/paint/concrete are dielectrics.
/// The source's linear greyscale roughness occupies G; B cannot introduce
/// metal because StandardMaterial multiplies it by the explicit zero below.
fn material(surface: Surface, server: &AssetServer) -> StandardMaterial {
    let maps = MapSet::load(server, surface.role());
    StandardMaterial {
        base_color_texture: Some(maps.albedo),
        normal_map_texture: Some(maps.normal),
        metallic_roughness_texture: Some(maps.rough),
        occlusion_texture: maps.ao,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: super::fresnel::DIELECTRIC,
        depth_bias: match surface {
            Surface::Yard => DEPOT_YARD_DEPTH_BIAS,
            Surface::Steel | Surface::Timber => DEPOT_SURFACE_DEPTH_BIAS,
            Surface::Paint => DEPOT_SURFACE_DEPTH_BIAS * 2.0,
            _ => 0.0,
        },
        ..default()
    }
}

/// Build once per world; separate material meshes are shared by any placed
/// depot. Frustum culling remains Bevy's; no new per-frame site walk is added.
pub fn spawn(
    mut commands: Commands,
    world: Res<WorldId>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    drawn: Query<(), With<DepotVisual>>,
) {
    if !drawn.is_empty() {
        return;
    }
    let groups: Vec<_> = depot_meshes()
        .into_iter()
        .map(|(surface, mesh)| (meshes.add(mesh), materials.add(material(surface, &server))))
        .collect();
    let root = commands
        .spawn((
            DepotVisual,
            WorldEntity,
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .id();
    for site in world
        .haven
        .minor
        .iter()
        .filter(|site| layout::is_depot(site))
    {
        for (mesh, material) in &groups {
            commands.spawn((
                ChildOf(root),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                site_transform(site),
            ));
        }
    }
}

/// The same builder used by arithmetic tests, not a parallel model.
pub fn depot_meshes() -> Vec<(Surface, Mesh)> {
    let mut soups = kit();
    for (index, part) in layout::DEPOT_PARTS.iter().enumerate() {
        draw_part(&mut soups, part, index);
    }
    finish(soups)
}

pub fn part_meshes(part: &Part, index: usize) -> Vec<(Surface, Mesh)> {
    let mut soups = kit();
    draw_part(&mut soups, part, index);
    finish(soups)
}

fn finish(soups: [Soup; SURFACES.len()]) -> Vec<(Surface, Mesh)> {
    soups
        .into_iter()
        .zip(SURFACES)
        .filter_map(|(s, surface)| (!s.is_empty()).then(|| (surface, s.mesh())))
        .collect()
}

fn draw_part(s: &mut [Soup; SURFACES.len()], p: &Part, index: usize) {
    let b = p.bounds;
    match p.kind {
        PartKind::Concrete => cuboid(s, b, Surface::Yard, DEPOT_YARD_TINT),
        PartKind::Wall => {
            if b[1] < DEPOT_PLINTH_M {
                let mut foot = b;
                foot[4] = b[4].min(DEPOT_PLINTH_M);
                cuboid(s, foot, Surface::Concrete, [1.0; 3]);
                let mut upper = b;
                upper[1] = foot[4];
                cuboid(s, upper, Surface::Sheet, DEPOT_SHEET_TINT);
            } else {
                cuboid(s, b, Surface::Sheet, DEPOT_SHEET_TINT);
                // The one elevated wall spans the warehouse doorway. Its
                // east-facing header carries original geometric lettering.
                sign(s, b, "FREIGHT");
            }
            wall_frames(s, b);
        }
        PartKind::Roof => {
            cuboid(s, b, Surface::Roof, DEPOT_ROOF_TINT);
            let mut rim = b;
            rim[1] = (b[4] - DEPOT_FRAME_M).max(b[1]);
            for axis in [0, 2] {
                for upper in [false, true] {
                    let mut edge = rim;
                    if upper {
                        edge[axis] = (b[axis + 3] - DEPOT_FRAME_M).max(b[axis]);
                    } else {
                        edge[axis + 3] = (b[axis] + DEPOT_FRAME_M).min(b[axis + 3]);
                    }
                    cuboid(s, edge, Surface::Steel, DEPOT_STEEL_TINT);
                }
            }
        }
        PartKind::Steel => {
            cuboid(s, b, Surface::Steel, DEPOT_STEEL_TINT);
            if b[1] > 0.0 {
                monitor(s, b);
            } else if b[2].abs() > layout::YARD_HALF_Z - 1.0 {
                gate_paint(s, b);
            }
        }
        PartKind::Fence => {
            cuboid(s, b, Surface::Sheet, DEPOT_FENCE_TINT);
            wall_frames(s, b);
        }
        PartKind::Cargo => {
            let tint = DEPOT_CARGO_TINTS[index % DEPOT_CARGO_TINTS.len()];
            cuboid(s, b, Surface::Cargo, tint);
            cargo_frames(s, b);
        }
    }
}

fn weather(v: Vec3, tint: [f32; 3]) -> [f32; 4] {
    let cell = |f: f32| (f / DEPOT_WEATHER_CELL_M).floor() as i32 as u32;
    let grain = super::props::hash01(
        cell(v.x).wrapping_add(cell(v.y).wrapping_mul(31)),
        cell(v.z),
    );
    let macro_keep = 1.0 - DEPOT_WEATHER_STRENGTH * grain;
    let base_keep = DEPOT_CONTACT_KEEP
        + (1.0 - DEPOT_CONTACT_KEEP) * (v.y / DEPOT_CONTACT_HEIGHT_M).clamp(0.0, 1.0);
    [
        tint[0] * macro_keep * base_keep,
        tint[1] * macro_keep * base_keep,
        tint[2] * macro_keep * base_keep,
        1.0,
    ]
}

fn uv(v: Vec3, n: Vec3, surface: Surface) -> [f32; 2] {
    let a = n.abs();
    let (u, w) = if a.y > a.x && a.y > a.z {
        (v.z, v.x) // roof ribs run in local Z, toward the short ends
    } else {
        let horizontal = if a.x > a.z { v.z } else { v.x };
        if surface.sheet() {
            (v.y, horizontal)
        } else {
            (horizontal, v.y)
        }
    };
    [u * surface.tiles(), w * surface.tiles()]
}

fn quad(
    s: &mut [Soup; SURFACES.len()],
    surface: Surface,
    q: [Vec3; 4],
    tint: [f32; 3],
    subdivide: bool,
) {
    let normal = (q[1] - q[0]).cross(q[2] - q[0]).normalize_or_zero();
    let nu = if subdivide {
        ((q[1] - q[0]).length() / DEPOT_FACE_GRID_M).ceil().max(1.0) as usize
    } else {
        1
    };
    let nv = if subdivide {
        ((q[3] - q[0]).length() / DEPOT_FACE_GRID_M).ceil().max(1.0) as usize
    } else {
        1
    };
    for y in 0..nv {
        for x in 0..nu {
            let at = |i: usize, j: usize| {
                q[0] + (q[1] - q[0]) * (i as f32 / nu as f32)
                    + (q[3] - q[0]) * (j as f32 / nv as f32)
            };
            let p = [at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1)];
            for ids in [[0, 1, 2], [0, 2, 3]] {
                s[surface as usize].tri_uv(
                    ids.map(|i| (p[i], uv(p[i], normal, surface))),
                    |v| weather(v, tint),
                    None,
                    |_| 0.0,
                );
            }
        }
    }
}

fn cuboid(s: &mut [Soup; SURFACES.len()], b: [f32; 6], surface: Surface, tint: [f32; 3]) {
    if b[0] >= b[3] || b[1] >= b[4] || b[2] >= b[5] {
        return;
    }
    let v = |x: bool, y: bool, z: bool| {
        Vec3::new(
            b[usize::from(x) * 3],
            b[1 + usize::from(y) * 3],
            b[2 + usize::from(z) * 3],
        )
    };
    let faces = [
        [
            v(false, false, true),
            v(true, false, true),
            v(true, true, true),
            v(false, true, true),
        ],
        [
            v(true, false, false),
            v(false, false, false),
            v(false, true, false),
            v(true, true, false),
        ],
        [
            v(true, false, true),
            v(true, false, false),
            v(true, true, false),
            v(true, true, true),
        ],
        [
            v(false, false, false),
            v(false, false, true),
            v(false, true, true),
            v(false, true, false),
        ],
        [
            v(false, true, true),
            v(true, true, true),
            v(true, true, false),
            v(false, true, false),
        ],
        [
            v(false, false, false),
            v(true, false, false),
            v(true, false, true),
            v(false, false, true),
        ],
    ];
    for q in faces {
        quad(s, surface, q, tint, true);
    }
}

fn wall_frames(s: &mut [Soup; SURFACES.len()], b: [f32; 6]) {
    let long_x = b[3] - b[0] > b[5] - b[2];
    let axis = if long_x { 0 } else { 2 };
    // Bay rhythm follows the existing published part, not random box placement.
    let bays = ((b[axis + 3] - b[axis]) / DEPOT_FRAME_BAY_M).ceil() as usize;
    for k in 0..=bays {
        let mut post = b;
        let p = b[axis] + (b[axis + 3] - b[axis]) * k as f32 / bays as f32;
        post[axis] = (p - DEPOT_FRAME_M * 0.5).max(b[axis]);
        post[axis + 3] = (p + DEPOT_FRAME_M * 0.5).min(b[axis + 3]);
        cuboid(s, post, Surface::Steel, DEPOT_STEEL_TINT);
    }
    let mut beam = b;
    beam[1] = (b[4] - DEPOT_FRAME_M).max(b[1]);
    cuboid(s, beam, Surface::Steel, DEPOT_STEEL_TINT);
}

fn cargo_frames(s: &mut [Soup; SURFACES.len()], b: [f32; 6]) {
    let mut pallet = b;
    pallet[4] = (b[1] + DEPOT_FRAME_M * 2.0).min(b[4]);
    cuboid(s, pallet, Surface::Timber, [1.0; 3]);
    let tier_height = layout::DEPOT_PARTS
        .iter()
        .filter(|p| p.kind == PartKind::Cargo)
        .map(|p| p.bounds[4] - p.bounds[1])
        .fold(f32::INFINITY, f32::min);
    let tiers = ((b[4] - b[1]) / tier_height).round().max(1.0) as usize;
    for i in 0..=tiers {
        let y = b[1] + (b[4] - b[1]) * i as f32 / tiers as f32;
        let mut band = b;
        band[1] = (y - DEPOT_FRAME_M * 0.5).max(b[1]);
        band[4] = (y + DEPOT_FRAME_M * 0.5).min(b[4]);
        cuboid(s, band, Surface::Steel, DEPOT_STEEL_TINT);
    }
    // Framed double doors face the through-lane. All hardware is recessed
    // into the same freight mass, never a loot-container proxy.
    for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let mut bar = b;
        let z = b[2] + (b[5] - b[2]) * t;
        bar[2] = (z - DEPOT_FRAME_M * 0.5).max(b[2]);
        bar[5] = (z + DEPOT_FRAME_M * 0.5).min(b[5]);
        if b[0] + b[3] > 0.0 {
            bar[3] = (b[0] + DEPOT_FRAME_M).min(b[3]);
        } else {
            bar[0] = (b[3] - DEPOT_FRAME_M).max(b[0]);
        }
        cuboid(s, bar, Surface::Steel, DEPOT_STEEL_TINT);
    }
}

fn monitor(s: &mut [Soup; SURFACES.len()], b: [f32; 6]) {
    for t in [0.25, 0.5, 0.75] {
        let mut louver = b;
        let y = b[1] + (b[4] - b[1]) * t;
        louver[1] = y;
        louver[4] = (y + DEPOT_FRAME_M).min(b[4]);
        cuboid(s, louver, Surface::Paint, DEPOT_MONITOR_TINT);
    }
}

fn gate_paint(s: &mut [Soup; SURFACES.len()], b: [f32; 6]) {
    // Painted clearance bands live on the post faces, not across the opening.
    for k in 0..DEPOT_GATE_BANDS {
        let mut band = b;
        band[1] = b[1]
            + (b[4] - b[1]) * (k as f32 + (1.0 - DEPOT_GATE_BAND_FRACTION) * 0.5)
                / DEPOT_GATE_BANDS as f32;
        band[4] = b[1]
            + (b[4] - b[1]) * (k as f32 + (1.0 + DEPOT_GATE_BAND_FRACTION) * 0.5)
                / DEPOT_GATE_BANDS as f32;
        cuboid(s, band, Surface::Paint, DEPOT_SAFETY_TINT);
    }
}

fn glyph(c: u8) -> [u8; 7] {
    match c {
        b'F' => [31, 16, 16, 30, 16, 16, 16],
        b'R' => [30, 17, 17, 30, 20, 18, 17],
        b'E' => [31, 16, 16, 30, 16, 16, 31],
        b'I' => [31, 4, 4, 4, 4, 4, 31],
        b'G' => [15, 16, 16, 23, 17, 17, 15],
        b'H' => [17, 17, 17, 31, 17, 17, 17],
        b'T' => [31, 4, 4, 4, 4, 4, 4],
        _ => [0; 7],
    }
}

fn sign(s: &mut [Soup; SURFACES.len()], b: [f32; 6], text: &str) {
    let cells = text.len() * 6 - 1;
    let step = ((b[5] - b[2]) * DEPOT_SIGN_WIDTH_FRACTION / cells as f32)
        .min((b[4] - b[1]) * DEPOT_SIGN_HEIGHT_FRACTION / 7.0);
    let left = (b[2] + b[5] + cells as f32 * step) * 0.5;
    let top = (b[1] + b[4] + 7.0 * step) * 0.5;
    for (i, c) in text.bytes().enumerate() {
        for (j, row) in glyph(c).into_iter().enumerate() {
            for k in 0..5 {
                if row & (1 << (4 - k)) == 0 {
                    continue;
                }
                let z = left - (i * 6 + k + 1) as f32 * step;
                let y = top - j as f32 * step;
                quad(
                    s,
                    Surface::Paint,
                    [
                        Vec3::new(b[3], y - step, z + step),
                        Vec3::new(b[3], y - step, z),
                        Vec3::new(b[3], y, z),
                        Vec3::new(b[3], y, z + step),
                    ],
                    [1.0; 3],
                    false,
                );
            }
        }
    }
}

/// The actual instance transform; tests compare Bevy's quaternion transform
/// against the sim's local-to-world arithmetic over every quantized phase.
pub fn site_transform(site: &Waystation) -> Transform {
    let (s, c) = sim_core::yaw_dir((site.phase as u16) << 8);
    let rotation = Quat::from_mat3(&Mat3::from_cols(
        Vec3::new(c, 0.0, -s),
        Vec3::Y,
        Vec3::new(s, 0.0, c),
    ));
    Transform::from_xyz(site.x, site.floor_y, site.z).with_rotation(rotation)
}
