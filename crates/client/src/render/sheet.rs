//! The blueprint a deployable is held as.
//!
//! Rust holds a box, a bag, a furnace or a seed as a sheet of blue paper low
//! in the frame, under the belt, and it unrolls when the item is drawn, with
//! a rustle. Ours drew a shrunken copy of the box in the fist, which read as
//! carrying a toy (operator, 2026-10-03: *"it unfurls like a little
//! blueprint… in the lower bottom of the screen underneath the toolbar"*).
//!
//! The sheet hangs off the camera, not the arm: it runs off the bottom of the
//! frame, where the hands holding it would be, so the arm is lowered out of
//! view while it is up (`viewmodel::animate`'s stow) rather than left beside
//! it with an empty fist. It still takes the arm's bob and sway
//! ([`super::viewmodel::Motion::idle`]) so it reads as carried.
//!
//! **The unroll is a real roll, not a squash.** The mesh is a strip whose
//! upper part lies flat and whose lower part is wound on a spiral behind it;
//! the draw brings the roll up into frame and moves it down the sheet toward
//! the eye and off under the belt, so the paper is revealed rather than
//! stretched. Rebuilt only while it moves.
//!
//! What a deployable is comes from `ui::hold::Click::Deploy`, the same answer
//! the left click, the ghost and the HUD read. Other bodies still draw the
//! item's own model in their hand (`bodies`): this is the first-person view.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::ImageSampler;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::rig::EyeCam;
use super::viewmodel::Motion;
use super::Net;

/// The sheet's width and height, metres.
pub const SHEET_W: f32 = 0.26;
pub const SHEET_H: f32 = 0.22;
/// The top edge's centre, view space, metres: ndc y ≈ −0.53, a quarter of
/// the way up the screen. With [`SHEET_TILT`] the bottom edge is well below
/// the frame and the belt is drawn across the sheet's visible part.
pub const SHEET_TOP: Vec3 = Vec3::new(0.0, -0.215, -0.53);
/// How far the sheet leans back from upright, radians: the top edge away
/// from the eye, so the face looks up at it like a page being read.
pub const SHEET_TILT: f32 = 0.85;
/// The roll's outer radius, metres.
pub const SHEET_ROLL_R: f32 = 0.016;
/// How much the roll's radius shrinks per radian wound, as a fraction of
/// [`SHEET_ROLL_R`], so the turns nest instead of drawing over each other.
const ROLL_TIGHTEN: f32 = 0.11 / std::f32::consts::TAU;
/// Rows along the sheet's height. The roll needs them; the flat part would
/// not.
const ROWS: usize = 48;
/// The draw, seconds: up from below the frame, then unrolled.
pub const SHEET_DRAW_S: f32 = 0.5;
/// Where the sheet starts the draw, relative to where it ends: lower and a
/// little nearer, so the roll comes up into frame from just below it.
pub const SHEET_RISE: Vec3 = Vec3::new(0.0, -0.17, 0.04);
/// The point the sway turns the sheet about: roughly the chest.
const SWAY_PIVOT: Vec3 = Vec3::new(0.0, -0.25, 0.0);
/// The texture's side, texels.
const TEX: u32 = 512;

/// Is the selected stack held as a sheet? A deployable, out of the gate.
pub fn up(n: &Net) -> bool {
    let core = &n.session.core;
    !core.holstered()
        && crate::ui::hold::click_in_hand(
            &core.catalog,
            &core.research,
            &core.deploy_defs,
            core.deploy_defs_have,
            &core.inv,
            n.sel,
        ) == crate::ui::hold::Click::Deploy
}

/// The draw at progress `t` (0..=1 over [`SHEET_DRAW_S`]): how far the sheet
/// has risen and how much of it is unrolled, both 0..=1. The rise leads and
/// the unroll follows it, so the roll comes up into frame before it opens.
pub fn draw_at(t: f32) -> (f32, f32) {
    let t = t.clamp(0.0, 1.0);
    let rise = 1.0 - (1.0 - (t / 0.45).min(1.0)).powi(3);
    let u = ((t - 0.15) / 0.85).clamp(0.0, 1.0);
    let unroll = u * u * (3.0 - 2.0 * u);
    (rise, unroll)
}

/// One row of the strip at unroll `u`: its point and its normal as (distance
/// down the sheet from the top edge, height off the printed face). Rows
/// within `u · H` of the top lie flat; the rest wind backward onto a
/// tightening spiral whose start sits on that line.
pub fn rows(u: f32) -> [(Vec2, Vec2); ROWS + 1] {
    let s0 = u.clamp(0.0, 1.0) * SHEET_H;
    let mut out = [(Vec2::ZERO, Vec2::Y); ROWS + 1];
    let (mut phi, mut prev) = (0.0f32, s0);
    for (i, row) in out.iter_mut().enumerate() {
        let s = SHEET_H * i as f32 / ROWS as f32;
        if s <= s0 {
            *row = (Vec2::new(s, 0.0), Vec2::Y);
            continue;
        }
        let r = SHEET_ROLL_R * (1.0 - phi * ROLL_TIGHTEN).max(0.35);
        phi += (s - prev) / r;
        prev = s;
        let r = SHEET_ROLL_R * (1.0 - phi * ROLL_TIGHTEN).max(0.35);
        let (sin, cos) = phi.sin_cos();
        // Centre one radius behind the sheet at `s0`, so φ = 0 is the flat
        // part's last point and the paper leaves it heading straight on.
        *row = (
            Vec2::new(s0 + r * sin, -SHEET_ROLL_R + r * cos),
            Vec2::new(sin, cos),
        );
    }
    out
}

/// The strip's positions and normals at unroll `u`, two vertices a row, in
/// the sheet's frame: the top edge's centre at the origin, the sheet down
/// −y, the printed face +z.
fn shape(u: f32) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let mut pos = Vec::with_capacity((ROWS + 1) * 2);
    let mut nrm = Vec::with_capacity((ROWS + 1) * 2);
    for (p, n) in rows(u) {
        for x in [-0.5 * SHEET_W, 0.5 * SHEET_W] {
            pos.push([x, -p.x, p.y]);
            nrm.push([0.0, -n.x, n.y]);
        }
    }
    (pos, nrm)
}

/// The strip, rolled up. Kept in the main world as well so [`drive`] can
/// rewrite its points while it unrolls.
pub fn mesh() -> Mesh {
    let (pos, nrm) = shape(0.0);
    let mut uv = Vec::with_capacity(pos.len());
    let mut idx = Vec::with_capacity(ROWS * 6);
    for i in 0..=ROWS {
        let v = i as f32 / ROWS as f32;
        uv.push([0.0, v]);
        uv.push([1.0, v]);
    }
    for i in 0..ROWS as u32 {
        let (a, b, c, d) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
        idx.extend_from_slice(&[a, b, d, a, d, c]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
    .with_inserted_indices(Indices::U32(idx))
}

/// The paper: blueprint blue, a fine and a coarse grid, a double border, a
/// title block, and a crate drawn in isometric — the generic "something to
/// put down", as Rust's own sheet is generic. RGBA8 sRGB, `TEX` square,
/// level 0 only (the caller builds the chain).
pub fn pixels() -> Vec<u8> {
    let n = TEX as usize;
    let mut px = vec![0u8; n * n * 4];
    let paper = Vec3::new(0.11, 0.29, 0.55);
    let ink = Vec3::new(0.90, 0.95, 1.0);
    // Texels are not square on the sheet (it is wider than tall), so the
    // drawing's x offsets are scaled to keep the crate a crate.
    let ax = SHEET_H / SHEET_W;
    let segs = drawing(ax);
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut c = paper * (1.0 + 0.05 * (mottle(fx, fy) - 0.5));
            let line = |d: f32, half: f32| (half + 0.5 - d).clamp(0.0, 1.0);
            // The grids, over every texel.
            let g = |p: f32, step: f32| {
                let m = p % step;
                m.min(step - m)
            };
            let fine = line(g(fx, 12.0).min(g(fy, 12.0)), 0.35);
            let coarse = line(g(fx, 48.0).min(g(fy, 48.0)), 0.7);
            c = c.lerp(ink, 0.13 * fine);
            c = c.lerp(ink, 0.24 * coarse);
            // The drawing in ink.
            let mut a = 0.0f32;
            let at = Vec2::new(fx, fy);
            for (p, q, half, dash) in &segs {
                let pad = half + 1.0;
                if at.cmplt(p.min(*q) - pad).any() || at.cmpgt(p.max(*q) + pad).any() {
                    continue;
                }
                let (d, along) = seg_dist(at, *p, *q);
                if *dash && (along / 7.0) as i32 % 2 == 1 {
                    continue;
                }
                a = a.max(line(d, *half));
            }
            c = c.lerp(ink, 0.92 * a);
            let i = (y * n + x) * 4;
            px[i] = to_u8(c.x);
            px[i + 1] = to_u8(c.y);
            px[i + 2] = to_u8(c.z);
            px[i + 3] = 255;
        }
    }
    px
}

/// The paper's image, with its mip chain: the grid is a texel wide and
/// shimmers without one.
pub fn image() -> Image {
    let level0 = pixels();
    let mut img = Image::new(
        Extent3d {
            width: TEX,
            height: TEX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        super::mipmap::chain(&level0, TEX, TEX, super::mipmap::Filter::Srgb),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.texture_descriptor.mip_level_count = super::mipmap::levels(TEX, TEX);
    img.sampler = ImageSampler::linear();
    img
}

/// The ink, as segments in texels: `(from, to, half width, dashed)`.
fn drawing(ax: f32) -> Vec<(Vec2, Vec2, f32, bool)> {
    let t = TEX as f32;
    let mut s = Vec::new();
    let mut rect = |x0: f32, y0: f32, x1: f32, y1: f32, half: f32| {
        let (a, b, c, d) = (
            Vec2::new(x0, y0),
            Vec2::new(x1, y0),
            Vec2::new(x1, y1),
            Vec2::new(x0, y1),
        );
        for (p, q) in [(a, b), (b, c), (c, d), (d, a)] {
            s.push((p, q, half, false));
        }
    };
    // The border, twice, and a title block. Only the top third or so of the
    // sheet is on screen above the belt, so the drawing lives there.
    rect(18.0, 18.0, t - 18.0, t - 18.0, 1.6);
    rect(26.0, 26.0, t - 26.0, t - 26.0, 0.6);
    let (bx, by) = (t - 26.0 - 140.0, 40.0);
    rect(bx, by, t - 40.0, by + 58.0, 1.0);
    for (p, q, half) in [
        ((bx, by + 22.0), (t - 40.0, by + 22.0), 0.8),
        ((bx + 12.0, by + 11.0), (bx + 70.0, by + 11.0), 1.6),
        ((bx + 12.0, by + 34.0), (bx + 100.0, by + 34.0), 1.2),
        ((bx + 12.0, by + 46.0), (bx + 64.0, by + 46.0), 1.2),
    ] {
        s.push((Vec2::from(p), Vec2::from(q), half, false));
    }
    // A crate in isometric: the three faces you see in ink, the three edges
    // you do not dashed.
    let o = Vec2::new(t * 0.40, t * 0.21);
    let e = 60.0;
    let (ix, iz, iy) = (
        Vec2::new(0.866 * ax, 0.5) * e,
        Vec2::new(-0.866 * ax, 0.5) * e,
        Vec2::new(0.0, -1.0) * e,
    );
    let at = |x: f32, y: f32, z: f32| o + ix * x + iy * y + iz * z;
    let v = [
        at(0.0, 0.0, 0.0),
        at(1.0, 0.0, 0.0),
        at(1.0, 0.0, 1.0),
        at(0.0, 0.0, 1.0),
        at(0.0, 1.0, 0.0),
        at(1.0, 1.0, 0.0),
        at(1.0, 1.0, 1.0),
        at(0.0, 1.0, 1.0),
    ];
    for (a, b) in [
        (1, 2),
        (2, 3),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ] {
        s.push((v[a], v[b], 1.9, false));
    }
    for (a, b) in [(0, 1), (0, 3), (0, 4)] {
        s.push((v[a], v[b], 1.0, true));
    }
    // The lid's planks, and a dimension line under the near edge.
    for k in [0.33f32, 0.66] {
        s.push((at(k, 1.0, 0.0), at(k, 1.0, 1.0), 1.0, false));
    }
    let drop = Vec2::new(0.0, 16.0);
    s.push((v[3] + drop, v[2] + drop, 0.9, false));
    s.push((v[3] + drop * 0.4, v[3] + drop * 1.4, 0.7, false));
    s.push((v[2] + drop * 0.4, v[2] + drop * 1.4, 0.7, false));
    // Centre lines through the crate, long dashes past it.
    let mid = (v[3] + v[6]) * 0.5;
    s.push((mid - ix * 0.9, mid + ix * 0.9, 0.6, true));
    s
}

/// Distance from `p` to segment `a`–`b`, and how far along it the nearest
/// point is (texels from `a`), for the dashes.
fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> (f32, f32) {
    let ab = b - a;
    let len2 = ab.length_squared().max(1e-6);
    let k = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    ((p - (a + ab * k)).length(), k * len2.sqrt())
}

/// Smooth value noise in 0..1 with a 24-texel cell: the paper's fibre.
fn mottle(x: f32, y: f32) -> f32 {
    let (x, y) = (x / 24.0, y / 24.0);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let h = |i: f32, j: f32| {
        let mut k = (i as i32 as u32).wrapping_mul(0x9E37_79B1)
            ^ (j as i32 as u32).wrapping_mul(0x85EB_CA77);
        k ^= k >> 15;
        k = k.wrapping_mul(0x2C1B_3C6D);
        k ^= k >> 12;
        (k & 0xFFFF) as f32 / 65535.0
    };
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = h(x0, y0) + (h(x0 + 1.0, y0) - h(x0, y0)) * sx;
    let bot = h(x0, y0 + 1.0) + (h(x0 + 1.0, y0 + 1.0) - h(x0, y0 + 1.0)) * sx;
    top + (bot - top) * sy
}

fn to_u8(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The sheet's transform at rise `rise`, under the arm's sway `lag` and bob
/// `drift` this frame.
pub fn pose(rise: f32, lag: Quat, drift: Vec3) -> Transform {
    let at = SHEET_TOP + SHEET_RISE * (1.0 - rise);
    Transform {
        translation: SWAY_PIVOT + lag * (at - SWAY_PIVOT) + drift,
        rotation: lag * Quat::from_rotation_x(-SHEET_TILT),
        scale: Vec3::ONE,
    }
}

/// The sheet on screen.
#[derive(Component)]
pub struct Sheet {
    mesh: Handle<Mesh>,
    /// Draw progress, 0..=1; restarts when a different deployable is drawn.
    t: f32,
    /// The unroll the mesh was last written at, so a still sheet is not
    /// rebuilt every frame.
    shaped: f32,
    /// The (slot, item) the draw is for.
    drawn: Option<(u8, u16)>,
}

/// Spawn the sheet under the camera, once, hidden. A latch for
/// `viewmodel::spawn_item`'s reason: the camera is not there on the first
/// frames.
pub fn spawn(
    mut commands: Commands,
    mut done: Local<bool>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    cam: Query<Entity, With<EyeCam>>,
) {
    if *done {
        return;
    }
    let Ok(cam) = cam.single() else { return };
    *done = true;
    let mesh = meshes.add(mesh());
    let mat = materials.add(StandardMaterial {
        base_color_texture: Some(images.add(image())),
        perceptual_roughness: 0.9,
        reflectance: super::fresnel::DIELECTRIC,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    commands.entity(cam).with_children(|c| {
        c.spawn((
            Sheet {
                mesh: mesh.clone(),
                t: 0.0,
                shaped: 0.0,
                drawn: None,
            },
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            pose(0.0, Quat::IDENTITY, Vec3::ZERO),
            Visibility::Hidden,
            // The points move under a bounding box computed once.
            NoFrustumCulling,
            bevy::light::NotShadowCaster,
        ));
    });
}

/// Draw, unroll and carry the sheet. After `viewmodel::animate`, whose sway
/// and bob it takes.
pub fn drive(
    time: Res<Time>,
    net: Option<NonSend<Net>>,
    motion: Res<Motion>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut q: Query<(&mut Sheet, &mut Transform, &mut Visibility)>,
) {
    let Ok((mut sheet, mut tf, mut vis)) = q.single_mut() else {
        return;
    };
    let held = net.as_deref().filter(|n| up(n)).map(|n| {
        let core = &n.session.core;
        let i = usize::from(n.sel).min(core.inv.len().saturating_sub(1));
        (n.sel, core.inv.get(i).map_or(0, |s| s.item))
    });
    if held.is_none() {
        sheet.drawn = None;
        *vis = Visibility::Hidden;
        return;
    }
    if sheet.drawn != held {
        sheet.drawn = held;
        sheet.t = 0.0;
    }
    sheet.t = (sheet.t + time.delta_secs() / SHEET_DRAW_S).min(1.0);
    let (rise, unroll) = draw_at(sheet.t);
    if unroll != sheet.shaped || *vis == Visibility::Hidden {
        if let Some(m) = meshes.get_mut(&sheet.mesh) {
            let (pos, nrm) = shape(unroll);
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
            sheet.shaped = unroll;
        }
    }
    let (lag, drift) = motion.idle();
    *tf = pose(rise, lag, drift);
    *vis = Visibility::Inherited;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrolled_is_flat_and_full_height() {
        let r = rows(1.0);
        assert!(r.iter().all(|(p, n)| p.y == 0.0 && *n == Vec2::Y));
        assert!((r[ROWS].0.x - SHEET_H).abs() < 1e-6);
    }

    #[test]
    fn rolled_stays_inside_the_roll() {
        // Wound up, every point is within one roll of the held edge, behind
        // the face — a tube, not a sheet sticking out.
        for (p, _) in rows(0.0).iter().skip(1) {
            assert!(p.x.abs() <= SHEET_ROLL_R + 1e-5, "{p}");
            assert!(p.y <= 1e-5 && p.y >= -2.0 * SHEET_ROLL_R - 1e-5, "{p}");
        }
    }

    #[test]
    fn the_paper_does_not_stretch() {
        // Neighbouring rows stay one row apart along the paper wherever the
        // roll is, so the print is revealed and never squashed.
        let step = SHEET_H / ROWS as f32;
        for u in [0.0, 0.3, 0.71, 1.0] {
            let r = rows(u);
            for w in r.windows(2) {
                let d = (w[1].0 - w[0].0).length();
                assert!(d <= step * 1.001 && d >= step * 0.9, "u {u}: {d} vs {step}");
            }
        }
    }

    #[test]
    fn the_draw_ends_up_and_open() {
        assert_eq!(draw_at(0.0), (0.0, 0.0));
        assert_eq!(draw_at(1.0), (1.0, 1.0));
        let (rise, unroll) = draw_at(0.3);
        assert!(rise > unroll, "the roll comes up before it opens");
    }

    #[test]
    fn the_top_edge_sits_low_on_screen_and_the_rest_runs_off_it() {
        // ndc y of a view-space point, at the rig's FOV.
        let half = (super::super::rig::FOV_DEG.to_radians() * 0.5).tan();
        let ndc = |p: Vec3| p.y / (-p.z * half);
        let t = pose(1.0, Quat::IDENTITY, Vec3::ZERO);
        let top = t.transform_point(Vec3::ZERO);
        let bottom = t.transform_point(Vec3::new(0.0, -SHEET_H, 0.0));
        assert!((-0.6..-0.45).contains(&ndc(top)), "top at {}", ndc(top));
        assert!(ndc(bottom) < -1.0, "the bottom edge is below the frame");
        // Drawn from just below the frame.
        let start = pose(0.0, Quat::IDENTITY, Vec3::ZERO).transform_point(Vec3::ZERO);
        assert!(
            (-1.2..-1.0).contains(&ndc(start)),
            "starts at {}",
            ndc(start)
        );
        // The face looks back at the eye.
        let n = t.rotation * Vec3::Z;
        assert!(n.dot(-top) > 0.0);
    }
}
