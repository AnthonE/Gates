//! THE GATE's signs, drawn on top of the dressed town: a name board over
//! each kiosk (the stall's name, as the server dripped it), Rust's posted
//! rules beside every gate, and "THE GATE" on the inner face of each gate
//! beam so it reads from inside too (`ci/site_kit.py` letters only the
//! outer faces).
//!
//! Block letters, as the dresser draws the gate's own name: a 5×7 cell
//! font, one small box per lit cell. Drawn, never collided — every board
//! sits above head height or flat on a wall.

use bevy::prelude::*;
use sim_core::town;

use super::props::{boxes_mesh_with, linear};
use super::{Net, WorldEntity, WorldId};

/// The 5×7 font: seven rows, five bits each, the high bit leftmost.
const GLYPHS: [(char, [u8; 7]); 28] = [
    ('A', [14, 17, 17, 31, 17, 17, 17]),
    ('B', [30, 17, 17, 30, 17, 17, 30]),
    ('C', [14, 17, 16, 16, 16, 17, 14]),
    ('D', [28, 18, 17, 17, 17, 18, 28]),
    ('E', [31, 16, 16, 30, 16, 16, 31]),
    ('F', [31, 16, 16, 30, 16, 16, 16]),
    ('G', [15, 16, 16, 23, 17, 17, 15]),
    ('H', [17, 17, 17, 31, 17, 17, 17]),
    ('I', [14, 4, 4, 4, 4, 4, 14]),
    ('J', [7, 2, 2, 2, 2, 18, 12]),
    ('K', [17, 18, 20, 24, 20, 18, 17]),
    ('L', [16, 16, 16, 16, 16, 16, 31]),
    ('M', [17, 27, 21, 21, 17, 17, 17]),
    ('N', [17, 17, 25, 21, 19, 17, 17]),
    ('O', [14, 17, 17, 17, 17, 17, 14]),
    ('P', [30, 17, 17, 30, 16, 16, 16]),
    ('Q', [14, 17, 17, 17, 21, 18, 13]),
    ('R', [30, 17, 17, 30, 20, 18, 17]),
    ('S', [15, 16, 16, 14, 1, 1, 30]),
    ('T', [31, 4, 4, 4, 4, 4, 4]),
    ('U', [17, 17, 17, 17, 17, 17, 14]),
    ('V', [17, 17, 17, 17, 17, 10, 4]),
    ('W', [17, 17, 17, 21, 21, 21, 10]),
    ('X', [17, 17, 10, 4, 10, 17, 17]),
    ('Y', [17, 17, 17, 10, 4, 4, 4]),
    ('Z', [31, 1, 2, 4, 8, 16, 31]),
    ('-', [0, 0, 0, 31, 0, 0, 0]),
    (' ', [0, 0, 0, 0, 0, 0, 0]),
];

fn glyph(ch: char) -> [u8; 7] {
    let up = ch.to_ascii_uppercase();
    GLYPHS
        .iter()
        .find(|(c, _)| *c == up)
        .map_or([0; 7], |(_, g)| *g)
}

/// A box list: `(centre, half-extent, hex)`, the props builder's shape.
type Boxes = Vec<([f32; 3], [f32; 3], u32)>;

/// A face: its centre (kit-local metres), the way it faces (the reader is
/// out along it), and its up.
#[derive(Clone, Copy)]
struct Face {
    at: Vec3,
    normal: Vec3,
}

impl Face {
    /// The reader's right, looking at the face.
    fn right(self) -> Vec3 {
        (-self.normal).cross(Vec3::Y)
    }

    /// A box on the face, `along` the reader's right and `up`, standing
    /// `out` off it, with half-extents across the face and through it.
    fn put(self, out: &mut Boxes, along: f32, up: f32, off: f32, half: (f32, f32, f32), hex: u32) {
        let r = self.right();
        let c = self.at + r * along + Vec3::Y * up + self.normal * off;
        let h = r.abs() * half.0 + Vec3::Y * half.1 + self.normal.abs() * half.2;
        out.push((c.to_array(), h.to_array(), hex));
    }
}

/// Lay `lines` of block letters on `face`, centred, `cell` metres a cell,
/// standing `depth` off it.
fn letters(out: &mut Boxes, face: Face, lines: &[(&str, u32)], cell: f32, depth: f32) {
    let n = lines.len() as f32;
    for (j, (text, hex)) in lines.iter().enumerate() {
        let chars: Vec<char> = text.chars().collect();
        let width = chars.len() as f32 * 6.0 * cell - cell;
        let line_y = ((n - 1.0) * 0.5 - j as f32) * 9.0 * cell;
        for (i, ch) in chars.iter().enumerate() {
            let rows = glyph(*ch);
            for (r, bits) in rows.iter().enumerate() {
                for c in 0..5 {
                    if bits & (1 << (4 - c)) == 0 {
                        continue;
                    }
                    let along = -width * 0.5 + (i as f32 * 6.0 + c as f32 + 0.5) * cell;
                    let up = line_y + (3.0 - r as f32) * cell;
                    let half = (cell * 0.46, cell * 0.46, depth * 0.5);
                    face.put(out, along, up, depth * 0.5, half, *hex);
                }
            }
        }
    }
}

/// The stall boards' colours, by kiosk.
const STALL_BOARDS: [u32; 6] = [0x2f5d3a, 0x6b4a2b, 0x2e4a66, 0x3e5f5c, 0x6e3226, 0x4a2323];
const STALL_INK: u32 = 0xf2d38a;
const RULES_BOARD: u32 = 0x1d1f22;
const RULES_HEAD: u32 = 0x7fd38a;
const RULES_INK: u32 = 0xe8e2cf;
const GILT: u32 = 0xf0c060;

/// Rust's posted rules (the Compound devblog: "No Weapons, No Looting, No
/// Killing, No Sleeping"), under the zone's name.
const RULES: [(&str, u32); 5] = [
    ("SAFE ZONE", RULES_HEAD),
    ("NO WEAPONS", RULES_INK),
    ("NO KILLING", RULES_INK),
    ("NO LOOTING", RULES_INK),
    ("NO SLEEPING", RULES_INK),
];

/// The gates' outer walls, beside each opening: (board centre, facing).
const RULE_BOARDS: [([f32; 3], [f32; 3]); 4] = [
    ([9.0, 2.2, 42.0], [0.0, 0.0, 1.0]),
    ([-9.0, 2.2, -42.0], [0.0, 0.0, -1.0]),
    ([42.0, 2.2, -9.0], [1.0, 0.0, 0.0]),
    ([-42.0, 2.2, 9.0], [-1.0, 0.0, 0.0]),
];

/// The gate beams' inner faces (`town::PARTS`' sign beams).
const BEAM_FACES: [([f32; 3], [f32; 3]); 4] = [
    ([0.0, 6.8, 40.2], [0.0, 0.0, -1.0]),
    ([0.0, 6.8, -40.2], [0.0, 0.0, 1.0]),
    ([40.2, 6.8, 0.0], [-1.0, 0.0, 0.0]),
    ([-40.2, 6.8, 0.0], [1.0, 0.0, 0.0]),
];

/// The rules boards and the beams' inner names: everything that does not
/// wait for the server.
fn fixed_boxes() -> (Boxes, Boxes) {
    let (mut boards, mut ink) = (Boxes::new(), Boxes::new());
    for (at, n) in RULE_BOARDS {
        let face = Face {
            at: Vec3::from_array(at),
            normal: Vec3::from_array(n),
        };
        face.put(&mut boards, 0.0, 0.0, 0.04, (2.05, 1.38, 0.04), RULES_BOARD);
        let board_face = Face {
            at: face.at + face.normal * 0.08,
            ..face
        };
        letters(&mut ink, board_face, &RULES, 0.055, 0.025);
    }
    for (at, n) in BEAM_FACES {
        let face = Face {
            at: Vec3::from_array(at),
            normal: Vec3::from_array(n),
        };
        letters(&mut ink, face, &[("THE GATE", GILT)], 0.11, 0.04);
    }
    (boards, ink)
}

/// A name board over each kiosk, on the street edge of its awning.
fn stall_boxes(names: &[String]) -> (Boxes, Boxes) {
    let (mut boards, mut ink) = (Boxes::new(), Boxes::new());
    for (k, &(kx, kz)) in town::KIOSKS.iter().enumerate() {
        let Some(name) = names.get(k).filter(|n| !n.is_empty()) else {
            continue;
        };
        let west = kx < 0.0;
        let face = Face {
            at: Vec3::new(if west { -4.12 } else { 4.12 }, 3.25, kz),
            normal: if west { Vec3::X } else { Vec3::NEG_X },
        };
        face.put(
            &mut boards,
            0.0,
            0.0,
            0.0,
            (2.5, 0.36, 0.04),
            STALL_BOARDS[k % STALL_BOARDS.len()],
        );
        let board_face = Face {
            at: face.at + face.normal * 0.04,
            ..face
        };
        let name: String = name.chars().take(10).collect();
        letters(&mut ink, board_face, &[(&name, STALL_INK)], 0.075, 0.03);
    }
    (boards, ink)
}

/// The fixed signs and the stall boards, each once per world.
#[derive(Component)]
pub struct TownSign;
#[derive(Component)]
pub struct StallSign;

#[allow(clippy::too_many_arguments)]
pub fn build(
    mut commands: Commands,
    net: NonSend<Net>,
    world: Option<Res<WorldId>>,
    fixed: Query<(), With<TownSign>>,
    stalls: Query<(), With<StallSign>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut mats: Local<Option<(Handle<StandardMaterial>, Handle<StandardMaterial>)>>,
) {
    let Some(world) = world else { return };
    let t = world.haven.town;
    if !t.live || (!fixed.is_empty() && !stalls.is_empty()) {
        return;
    }
    let (board_mat, ink_mat) = mats
        .get_or_insert_with(|| {
            (
                materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    perceptual_roughness: 0.8,
                    ..default()
                }),
                // Painted letters catch the lamps; a little glow keeps them
                // legible across the yard at night.
                materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    perceptual_roughness: 0.5,
                    emissive: LinearRgba::rgb(0.18, 0.15, 0.09),
                    ..default()
                }),
            )
        })
        .clone();
    let tf = super::town::transform(&t);
    let mut spawn =
        |commands: &mut Commands, boxes: Boxes, mat: &Handle<StandardMaterial>, tag: bool| {
            if boxes.is_empty() {
                return;
            }
            let mesh = meshes.add(boxes_mesh_with(&boxes, linear, 1.0));
            let mut e =
                commands.spawn((WorldEntity, Mesh3d(mesh), MeshMaterial3d(mat.clone()), tf));
            if tag {
                e.insert(TownSign);
            } else {
                e.insert(StallSign);
            }
        };
    if fixed.is_empty() {
        let (boards, ink) = fixed_boxes();
        spawn(&mut commands, boards, &board_mat, true);
        spawn(&mut commands, ink, &ink_mat, true);
    }
    if stalls.is_empty() {
        let core = &net.session.core;
        let names: Vec<String> = (0..town::KIOSKS.len())
            .map(|k| String::from_utf8_lossy(core.vendor_name(k)).into_owned())
            .collect();
        // The offers drip after the welcome; the boards wait for all six.
        if names.iter().all(|n| !n.is_empty()) {
            let (boards, ink) = stall_boxes(&names);
            spawn(&mut commands, boards, &board_mat, false);
            spawn(&mut commands, ink, &ink_mat, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stall_and_rule_letter_has_a_glyph() {
        for text in [
            "RATIONS", "BUILDERS", "TOOLS", "PARTS", "SALVAGE", "ARMS", "THE GATE",
        ]
        .into_iter()
        .chain(RULES.iter().map(|(t, _)| *t))
        {
            for ch in text.chars().filter(|c| *c != ' ') {
                assert_ne!(glyph(ch), [0; 7], "{ch:?} in {text:?} would be blank");
            }
        }
    }

    #[test]
    fn a_face_reads_left_to_right_for_the_reader() {
        // Facing +Z, the reader looks along -Z and their right is +X.
        let f = Face {
            at: Vec3::ZERO,
            normal: Vec3::Z,
        };
        assert!((f.right() - Vec3::X).length() < 1e-6);
        // Facing +X, their right is -Z.
        let f = Face {
            at: Vec3::ZERO,
            normal: Vec3::X,
        };
        assert!((f.right() - Vec3::NEG_Z).length() < 1e-6);
    }
}

/// THE GATE's shopkeepers: one beside each stall, facing the street —
/// drawn, never simulated (Rust's Bandit Town has shopkeepers, and a market
/// of six empty counters reads as abandoned). The player rig at idle.
#[derive(Component)]
pub struct Shopkeeper;

/// Where kiosk `k`'s keeper stands (kit-local x, z): in the gap on the
/// stall's south side, under nothing, out of the street.
pub fn keeper_at(k: usize) -> Option<(f32, f32)> {
    let &(kx, kz) = town::KIOSKS.get(k)?;
    // `town::PARTS`' kiosks run 6.1 m along the street, centred on their
    // trading spot: the keeper stands 0.6 m past the south end, level with
    // the container's middle.
    Some((if kx < 0.0 { -7.2 } else { 7.2 }, kz - 3.65))
}

pub fn shopkeepers(
    mut commands: Commands,
    world: Option<Res<WorldId>>,
    rig: Res<super::anim::Rig>,
    drawn: Query<(), With<Shopkeeper>>,
) {
    let Some(world) = world else { return };
    let t = world.haven.town;
    if !t.live || !drawn.is_empty() || !rig.ready() || !rig.shaded() {
        return;
    }
    let Some(scene) = rig.scene.clone() else {
        return;
    };
    let tf = super::town::transform(&t);
    for k in 0..town::KIOSKS.len() {
        let Some((lx, lz)) = keeper_at(k) else {
            continue;
        };
        // Facing the street: +X from the west row, −X from the east.
        let face = if lx < 0.0 {
            std::f32::consts::FRAC_PI_2
        } else {
            -std::f32::consts::FRAC_PI_2
        };
        let local = Transform::from_xyz(lx, 0.0, lz).with_rotation(Quat::from_rotation_y(face));
        let world_tf = tf.mul_transform(local).with_scale(Vec3::splat(rig.scale));
        commands.spawn((
            WorldEntity,
            Shopkeeper,
            super::anim::BodyAnim::default(),
            super::anim::Reshade(super::anim::Shade::Keeper),
            SceneRoot(scene.clone()),
            world_tf,
        ));
    }
}
