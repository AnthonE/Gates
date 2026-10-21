//! The lore, drawn (`ARC.md` F5–F7): the speakers as people (the player
//! rig at idle, the shopkeepers' shade), the inscriptions as black slabs
//! ([`SLAB_GLB`]) cut with the ancients' glyphs and a gold ring on top, and
//! each mechanism's dials as
//! stone drums ([`DIAL_GLB`]) whose notch turns as the dials do. Where each
//! stands is `sim_core::spot`'s answer, so it is drawn where the sim
//! measures reach from.

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use client_core::arc::LoreView;

use super::{Net, WorldEntity, WorldId};
use crate::ui::glyphs::{self, ADVANCE, GLYPH_COLS, GLYPH_ROWS, LEADING};

/// An inscription's stone: a blank basalt stele on a footing, 1.31 m wide,
/// 2.0 m tall, generated. The ring on top stays an exact torus.
pub const SLAB_GLB: &str = "models/site/lore_slab.glb";
/// How far the stele stands in the ground (`ART.md` rule 2).
const SLAB_SINK_M: f32 = 0.1;
/// A dial's drum: a basalt drum 1.1 m across, its top at 1.0 m, generated.
pub const DIAL_GLB: &str = "models/site/lore_dial.glb";

/// The stele's broad faces, measured off [`SLAB_GLB`]: front +Z at 0.120 m,
/// back −Z at −0.114 m, flat to ±3 mm over the carved field. The carving
/// stands 6 mm proud of each so it never fights the stone.
const FACE_FRONT_Z: f32 = 0.126;
const FACE_BACK_Z: f32 = -0.120;
/// The carved field on each face, in the slab's frame: 1.0 m by 1.2 m,
/// its middle 1.125 m up, inside the chipped edges.
const CARVE_W_M: f32 = 1.0;
const CARVE_H_M: f32 = 1.2;
const CARVE_MID_M: f32 = 1.125;
/// The most a glyph's cell is cut, so a short text is not a billboard.
const CARVE_MAX_CELL_M: f32 = 0.035;
/// How much of a cell's top the groove's upper wall shades, and how dark.
const LIP: f32 = 0.3;
const LIP_SHADE: f32 = 0.05;
/// How much of a stone not yet read has worn away, and how dull what is
/// left is.
const WORN_SHARE: f32 = 0.2;
const WORN_SHADE: f32 = 0.7;
/// A stone this player has read burns its glyphs the Verdigris' green: the
/// spires' veins, linear, measured off their emissive map, and lit by the
/// hour as they are (`boulders::spire_glow`).
const RUNE_GREEN: [f32; 3] = [0.08, 0.66, 0.25];

/// The lore's gold, rings and notches and the works' rings: old and worn,
/// a satin sheen rather than a mirror.
pub fn worn_gold() -> StandardMaterial {
    StandardMaterial {
        base_color: Color::srgb(0.70, 0.55, 0.27),
        metallic: 0.9,
        perceptual_roughness: 0.6,
        ..default()
    }
}

/// A drawn speaker, inscription or dial; what it is and its index.
#[derive(Component)]
pub struct LoreMark {
    pub kind: u8,
    pub index: u16,
}

/// An inscription's carving: which stone, what it was last cut from (the
/// text's length, 0 until this player reads it, and the alphabet's), the
/// cut drawn now, and its two looks: bare stone, and burning once read.
/// Read is this client's word alone: the text arrives only at the stone.
#[derive(Component)]
pub struct Carving {
    index: u16,
    key: Option<(u16, u8)>,
    cut: Option<Entity>,
    bare: Handle<StandardMaterial>,
    lit: Handle<StandardMaterial>,
}

/// A dial's notch: mechanism, dial.
#[derive(Component)]
pub struct DialNotch(pub u8, pub u8);

/// The mark over a speaker with an offer this player can still take
/// (`ui::offer::open_count`): a lit gold diamond turning over their head,
/// shown while there is something to claim or buy and gone once it is
/// theirs or minting. Its own entity, not the rig's child, so the rig's
/// scale does not size it.
#[derive(Component)]
pub struct OfferMark {
    speaker: u8,
    base: Vec3,
}

/// How far over a speaker's feet the mark turns, metres.
const MARK_UP_M: f32 = 2.45;
/// Its half height and half width, metres.
const MARK_HALF_H: f32 = 0.17;
const MARK_HALF_W: f32 = 0.11;
/// How fast it turns, rad/s, and how far it bobs, metres, how often.
const MARK_SPIN: f32 = 1.6;
const MARK_BOB_M: f32 = 0.06;
const MARK_BOB_HZ: f32 = 0.7;

fn drawn(have: &Query<&LoreMark>, kind: u8, index: u16) -> bool {
    have.iter().any(|m| m.kind == kind && m.index == index)
}

/// Feet height and position of a spot on the island.
fn at(world: &WorldId, spot: &sim_core::spot::Spot) -> Option<Vec3> {
    let (x, _, z) = sim_core::spot::world(&world.haven, spot)?;
    Some(Vec3::new(
        x,
        sim_core::terrain::ground(world.seed, &world.haven, x, z),
        z,
    ))
}

/// Stand up every speaker, stone and dial whose place has arrived.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    mut commands: Commands,
    net: NonSend<Net>,
    world: Option<Res<WorldId>>,
    rig: Res<super::anim::Rig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
    have: Query<&LoreMark>,
) {
    let Some(world) = world else { return };
    let lore = &net.session.core.lore;
    // People, once the rig has loaded (the shopkeepers' rule).
    if rig.ready() && rig.shaded() {
        if let Some(scene) = rig.scene.clone() {
            for (k, s) in lore.known_speakers() {
                if drawn(&have, protocol::ARC_SPEAKER, k as u16) {
                    continue;
                }
                let Some(p) = at(&world, &s.spot) else {
                    continue;
                };
                // Facing the middle of the site they stand at.
                let mid = match sim_core::spot::world(
                    &world.haven,
                    &sim_core::spot::Spot {
                        x_cm: 0,
                        y_cm: 0,
                        z_cm: 0,
                        ..s.spot
                    },
                ) {
                    Some((x, _, z)) => Vec3::new(x, p.y, z),
                    None => p + Vec3::Z,
                };
                let d = mid - p;
                let yaw = if d.length_squared() > 1e-4 {
                    d.x.atan2(d.z)
                } else {
                    0.0
                };
                commands.spawn((
                    WorldEntity,
                    LoreMark {
                        kind: protocol::ARC_SPEAKER,
                        index: k as u16,
                    },
                    super::anim::BodyAnim {
                        clip: Some(super::anim::Clip::Idle),
                        ..default()
                    },
                    super::anim::Reshade(super::anim::Shade::Keeper),
                    SceneRoot(scene.clone()),
                    Transform::from_translation(p)
                        .with_rotation(Quat::from_rotation_y(yaw))
                        .with_scale(Vec3::splat(rig.scale)),
                ));
                let base = p + Vec3::Y * MARK_UP_M;
                commands.spawn((
                    WorldEntity,
                    OfferMark {
                        speaker: k as u8,
                        base,
                    },
                    Mesh3d(meshes.add(diamond())),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        emissive: LinearRgba::rgb(1.8, 1.25, 0.4),
                        ..worn_gold()
                    })),
                    Transform::from_translation(base),
                    Visibility::Hidden,
                    NotShadowCaster,
                ));
            }
        }
    }
    for (k, ins) in lore.known_inscriptions() {
        if drawn(&have, protocol::ARC_INSCRIPTION, k as u16) {
            continue;
        }
        let Some(p) = at(&world, &ins.spot) else {
            continue;
        };
        let (slab, slab_material) = super::mobs::model(&server, SLAB_GLB);
        let ring = materials.add(worn_gold());
        // Fresh-cut basalt: the grey under the black.
        let bare = materials.add(StandardMaterial {
            base_color: Color::srgb(0.66, 0.65, 0.61),
            perceptual_roughness: 0.95,
            ..default()
        });
        let lit = materials.add(StandardMaterial {
            base_color: Color::srgb(0.05, 0.12, 0.07),
            perceptual_roughness: 0.6,
            ..default()
        });
        commands
            .spawn((
                WorldEntity,
                LoreMark {
                    kind: protocol::ARC_INSCRIPTION,
                    index: k as u16,
                },
                Carving {
                    index: k as u16,
                    key: None,
                    cut: None,
                    bare,
                    lit,
                },
                Transform::from_translation(p),
                Visibility::default(),
            ))
            .with_children(|c| {
                c.spawn((
                    Mesh3d(slab),
                    MeshMaterial3d(slab_material),
                    Transform::from_xyz(0.0, -SLAB_SINK_M, 0.0),
                ));
                c.spawn((
                    Mesh3d(meshes.add(Torus::new(0.28, 0.36))),
                    MeshMaterial3d(ring),
                    Transform::from_xyz(0.0, 2.15, 0.0)
                        .with_rotation(Quat::from_rotation_x(core::f32::consts::FRAC_PI_2)),
                ));
            });
    }
    for (k, m) in lore.known_mechs() {
        let def = sim_core::mech::MechDef {
            spot: m.spot,
            dials: m.n_dials,
            ..Default::default()
        };
        for d in 0..m.n_dials as usize {
            let index = (k as u16) << 8 | d as u16;
            if drawn(&have, protocol::ARC_MECH, index) {
                continue;
            }
            let Some(p) = at(&world, &sim_core::mech::dial_spot(&def, d)) else {
                continue;
            };
            let (drum, drum_material) = super::mobs::model(&server, DIAL_GLB);
            let notch = materials.add(worn_gold());
            commands
                .spawn((
                    WorldEntity,
                    LoreMark {
                        kind: protocol::ARC_MECH,
                        index,
                    },
                    Transform::from_translation(p),
                    Visibility::default(),
                ))
                .with_children(|c| {
                    c.spawn((Mesh3d(drum), MeshMaterial3d(drum_material)));
                    c.spawn((
                        DialNotch(k as u8, d as u8),
                        Transform::from_xyz(0.0, 1.0, 0.0),
                        Visibility::default(),
                    ))
                    .with_children(|n| {
                        n.spawn((
                            Mesh3d(meshes.add(Cuboid::new(0.08, 0.06, 0.36))),
                            MeshMaterial3d(notch),
                            Transform::from_xyz(0.0, 0.03, 0.2),
                        ));
                    });
                });
        }
    }
}

/// Show each speaker's mark while they have an offer this player can still
/// take, turning and bobbing.
pub fn offer_marks(
    net: NonSend<Net>,
    time: Res<Time>,
    mut q: Query<(&OfferMark, &mut Transform, &mut Visibility)>,
) {
    let core = &net.session.core;
    let t = time.elapsed_secs();
    for (m, mut tf, mut vis) in q.iter_mut() {
        let open = core.lore.speakers.get(m.speaker as usize).is_some_and(|s| {
            crate::ui::offer::open_count(s, &core.skins, &core.skins_owned, |item| {
                sim_core::craft::inv_count(&core.inv, item)
            }) > 0
        });
        let want = if open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        vis.set_if_neq(want);
        if open {
            let bob = (t * MARK_BOB_HZ * core::f32::consts::TAU).sin() * MARK_BOB_M;
            tf.translation = m.base + Vec3::Y * bob;
            tf.rotation = Quat::from_rotation_y(t * MARK_SPIN);
        }
    }
}

/// The mark's diamond: an octahedron, flat-shaded, its points up and down.
fn diamond() -> Mesh {
    let (h, w) = (MARK_HALF_H, MARK_HALF_W);
    let ring = [
        Vec3::new(w, 0.0, 0.0),
        Vec3::new(0.0, 0.0, w),
        Vec3::new(-w, 0.0, 0.0),
        Vec3::new(0.0, 0.0, -w),
    ];
    let (mut pos, mut nor) = (Vec::with_capacity(24), Vec::with_capacity(24));
    for i in 0..4 {
        let (a, b) = (ring[i], ring[(i + 1) % 4]);
        for tri in [[Vec3::Y * h, b, a], [Vec3::NEG_Y * h, a, b]] {
            let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize();
            for v in tri {
                pos.push(v.to_array());
                nor.push(n.to_array());
            }
        }
    }
    let n = pos.len();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; n]);
    mesh.insert_indices(Indices::U32((0..n as u32).collect()));
    mesh
}

/// Turn each dial's notch to the notch it shows.
pub fn dials(net: NonSend<Net>, mut q: Query<(&DialNotch, &mut Transform)>) {
    let lore = &net.session.core.lore;
    for (n, mut tf) in q.iter_mut() {
        let Some(m) = lore.mechs.get(n.0 as usize) else {
            continue;
        };
        let values = m.values.max(1) as f32;
        let v = m.dials.get(n.1 as usize).copied().unwrap_or(0) as f32;
        let want = Quat::from_rotation_y(-v / values * core::f32::consts::TAU);
        if tf.rotation.angle_between(want) > 1e-3 {
            tf.rotation = want;
        }
    }
}

/// Cut each stone's glyphs, and cut them again when what they are cut from
/// changes: the alphabet arriving, or this player reading the stone. A read
/// stone's glyphs burn by the hour, in the spires' steps of 1/32 of the day.
pub fn carve(
    mut commands: Commands,
    net: NonSend<Net>,
    weather: Res<super::weather::WeatherNow>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(Entity, &mut Carving)>,
) {
    let lore = &net.session.core.lore;
    let day = (weather.sun_lux.clamp(0.0, 1.0) * 32.0).round() / 32.0;
    let (night, noon) = (
        super::boulders::SPIRE_GLOW_NIGHT,
        super::boulders::SPIRE_GLOW_DAY,
    );
    let glow = night + (noon - night) * day;
    let [r, g, b] = RUNE_GREEN.map(|v| v * glow);
    let burn = LinearRgba::rgb(r, g, b);
    for (stone, mut c) in q.iter_mut() {
        if materials.get(&c.lit).is_some_and(|m| m.emissive != burn) {
            if let Some(m) = materials.get_mut(&c.lit) {
                m.emissive = burn;
            }
        }
        let Some(ins) = lore.inscriptions.get(c.index as usize) else {
            continue;
        };
        let key = (ins.len, lore.alphabet_len);
        if c.key == Some(key) {
            continue;
        }
        c.key = Some(key);
        if let Some(old) = c.cut.take() {
            commands.entity(old).despawn();
        }
        let Some(mesh) = carving_mesh(lore, c.index as usize) else {
            continue;
        };
        let material = if ins.len > 0 {
            c.lit.clone()
        } else {
            c.bare.clone()
        };
        c.cut = Some(
            commands
                .spawn((
                    ChildOf(stone),
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(material),
                    Transform::from_xyz(0.0, -SLAB_SINK_M, 0.0),
                    NotShadowCaster,
                ))
                .id(),
        );
    }
}

/// What stone `k` shows before this player has read it: words of the
/// alphabet's glyphs, fixed per stone, and worn (`WORN_SHARE`) past reading.
fn worn_text(lore: &LoreView, k: usize) -> String {
    let abc = lore.alphabet();
    let mut out = String::new();
    if abc.is_empty() {
        return out;
    }
    let mut n = 0u32;
    let mut roll = |m: usize| {
        n += 1;
        (super::props::hash2(k as u32 ^ 0x57_0E, n) as usize) % m
    };
    while out.len() < 48 {
        if !out.is_empty() {
            out.push(' ');
        }
        for _ in 0..2 + roll(6) {
            out.push(abc[roll(abc.len())] as char);
        }
    }
    out
}

/// Stone `k`'s carving, both faces, or `None` with nothing to cut. Each lit
/// cell of each glyph is a quad. Unread, it is pale where the chisel cut and
/// dark along its top where the groove's upper wall shades it (vertex colour
/// over the bare material); read, it burns whole, so it has no shade.
fn carving_mesh(lore: &LoreView, k: usize) -> Option<Mesh> {
    let read = lore.inscriptions[k].len > 0;
    let text = if read {
        lore.inscriptions[k].text().to_string()
    } else {
        worn_text(lore, k)
    };
    let (lines, cell) = glyphs::fit(&text, CARVE_W_M, CARVE_H_M, CARVE_MAX_CELL_M);
    let shade = if read { 1.0 } else { WORN_SHADE };
    let (cut, lip) = (
        [shade, shade, shade, 1.0],
        [LIP_SHADE, LIP_SHADE, LIP_SHADE, 1.0],
    );
    let mut m = Cut::default();
    let top = CARVE_MID_M + (lines.len() as f32 * LEADING as f32 - 2.0) * cell / 2.0;
    for (j, line) in lines.iter().enumerate() {
        let left = -(line.len() as f32 * ADVANCE as f32 - 1.0) * cell / 2.0;
        let y0 = top - (j as u32 * LEADING) as f32 * cell;
        for (i, ch) in line.bytes().enumerate() {
            let Some(g) = lore.glyph_of(ch) else {
                continue;
            };
            for r in 0..GLYPH_ROWS {
                for c in 0..GLYPH_COLS {
                    let at = ((j * 64 + i) as u32) * 16 + r * GLYPH_COLS + c;
                    if !glyphs::lit(g, c, r)
                        || (!read && super::props::hash01(k as u32, at) < WORN_SHARE)
                    {
                        continue;
                    }
                    let x = left + (i as u32 * ADVANCE + c) as f32 * cell;
                    let y = y0 - r as f32 * cell;
                    if !read && (r == 0 || !glyphs::lit(g, c, r - 1)) {
                        m.quad(x, x + cell, y - LIP * cell, y, lip);
                        m.quad(x, x + cell, y - cell, y - LIP * cell, cut);
                    } else {
                        m.quad(x, x + cell, y - cell, y, cut);
                    }
                }
            }
        }
    }
    if m.idx.is_empty() {
        return None;
    }
    let n = m.pos.len();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, m.nor);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, m.col);
    mesh.insert_indices(Indices::U32(m.idx));
    Some(mesh)
}

#[derive(Default)]
struct Cut {
    pos: Vec<[f32; 3]>,
    nor: Vec<[f32; 3]>,
    col: Vec<[f32; 4]>,
    idx: Vec<u32>,
}

impl Cut {
    /// One rectangle of the carving on both faces, mirrored behind so the
    /// back reads left to right too.
    fn quad(&mut self, x0: f32, x1: f32, y0: f32, y1: f32, col: [f32; 4]) {
        for (z, s) in [(FACE_FRONT_Z, 1.0), (FACE_BACK_Z, -1.0)] {
            let b = self.pos.len() as u32;
            for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
                self.pos.push([x * s, y, z]);
                self.nor.push([0.0, 0.0, s]);
                self.col.push(col);
            }
            self.idx
                .extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
        }
    }
}
