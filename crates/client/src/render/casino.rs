//! THE GATE's casino, drawn (`sim_core::casino`, `town::CASINO`): Rust's
//! Bandit Camp big wheel on a timber stand under its sign, the betting
//! terminals before it, and the slot machines under a lit marquee, all on a
//! plank deck.
//!
//! The terminals and the cabinets are generated models
//! (`assets/models/site/`, `MANIFEST.md`) sized to the sim's own boxes. The
//! wheel is drawn here: a disc of Rust's twenty-five coloured segments with
//! their multipliers on them, turned when the shard says it spins
//! (`EventMsg::WheelSpin`) and brought to rest on the segment it drew, on the
//! shard's clock, so every screen in the yard sees it stop together. The
//! bulbs round it chase while it turns and flash when it lands.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::prelude::*;
use sim_core::casino::{WHEEL, WHEEL_OPEN_TICKS, WHEEL_PAYS, WHEEL_ROUND_TICKS, WHEEL_SEGMENTS};
use sim_core::limits::TICK_HZ;
use sim_core::town::{
    CABINET_D, CABINET_H, CASINO, CASINO_GAP_M, SLOT_SPOTS, TERMINAL_D, WHEEL_HUB, WHEEL_RADIUS_M,
    WHEEL_TERMINALS,
};

use super::props::{boxes_mesh_with, linear, Soup};
use super::town_signs::{letters, Boxes, Face};
use super::{Net, WorldEntity, WorldId};

/// The generated cabinet and terminal, each facing +Z at its base.
pub const SLOT_GLB: &str = "models/site/slot_machine.glb";
pub const TERMINAL_GLB: &str = "models/site/bet_terminal.glb";

/// Whole turns the wheel makes before it settles.
const TURNS: f32 = 5.0;
/// How far in front of its board the disc hangs, metres.
const DISC_OFF_M: f32 = 0.14;
/// The bulbs round the disc, and how far out.
const RING_BULBS: usize = 40;
const RING_OUT_M: f32 = 0.2;
/// Seconds the bulbs flash after the wheel lands.
const LANDED_S: f32 = 5.0;

const WOOD: u32 = 0x5a3d26;
const WOOD_DARK: u32 = 0x3b2819;
const PLANK_A: u32 = 0x6b4c31;
const PLANK_B: u32 = 0x5c4029;
const STEEL: u32 = 0x2c2f33;
const BOARD: u32 = 0x23180f;
const SIGN_RED: u32 = 0x7a1f1a;
const CARPET: u32 = 0x6e1b1b;
const GILT: u32 = 0xe2b551;
const TIP: u32 = 0xc0241c;
const INK_LIGHT: u32 = 0xf6efdc;
const INK_DARK: u32 = 0x2a1d12;

/// The bulbs' glow at full brightness, linear; the yard's own bulbs'.
const BULB_GLOW: LinearRgba = LinearRgba::rgb(6.0, 4.5, 2.2);
/// How much of the night eye's gain (`rig::FlameGain`) the bulbs take.
const NIGHT_SHARE: f32 = 0.04;
/// The casino's two lamps at night: over the wheel, over the slots.
const LAMPS: [(f32, f32, f32); 2] = [(27.0, 5.6, 14.0), (20.0, 3.6, 18.8)];
const LAMP_LUMENS: f32 = 2600.0;
const LAMP_RANGE_M: f32 = 14.0;

#[derive(Component)]
pub struct CasinoVisual;

/// A casino lamp's light, dark by day.
#[derive(Component)]
pub struct CasinoLamp;

/// The two bulb sets' materials, which [`lights`] chases.
#[derive(Resource)]
pub struct CasinoBulbs {
    even: Handle<StandardMaterial>,
    odd: Handle<StandardMaterial>,
}

/// The turning disc: its rest transform, and the spin it is drawing (the
/// round, the angle it started from and the one it stops at).
#[derive(Component)]
pub struct WheelDisc {
    base: Transform,
    round: Option<u32>,
    from: f32,
    to: f32,
}

fn put(out: &mut Boxes, b: [f32; 6], hex: u32) {
    out.push((
        [
            (b[0] + b[3]) * 0.5,
            (b[1] + b[4]) * 0.5,
            (b[2] + b[5]) * 0.5,
        ],
        [
            (b[3] - b[0]) * 0.5,
            (b[4] - b[1]) * 0.5,
            (b[5] - b[2]) * 0.5,
        ],
        hex,
    ));
}

/// The stand's sign board, in the town's frame: `[x0, y0, z0, x1, y1, z1]`.
fn wheel_sign() -> [f32; 6] {
    let (hx, _, hz) = WHEEL_HUB;
    [hx + 0.02, 6.05, hz - 2.25, hx + 0.16, 6.95, hz + 2.25]
}

/// The slot row's marquee, on the cabinets' backs.
fn slot_marquee() -> [f32; 6] {
    let z0 = SLOT_SPOTS[0].1 + CASINO_GAP_M + CABINET_D - 0.22;
    let (x0, x1) = (
        SLOT_SPOTS[0].0 - 0.75,
        SLOT_SPOTS[SLOT_SPOTS.len() - 1].0 + 0.75,
    );
    [x0, CABINET_H + 0.02, z0, x1, CABINET_H + 0.72, z0 + 0.14]
}

/// Everything that stands still and is not a bulb, in the town's frame: the
/// deck, the stand, its sign, the pointer, the marquee.
fn fixed_boxes() -> Boxes {
    let mut out = Boxes::new();
    // The deck: planks across the wheel's floor and along the slot row, a
    // carpet runner before the terminals.
    let (tx, _) = WHEEL_TERMINALS[0];
    let (z0, z1) = (
        WHEEL_TERMINALS[0].1 - 1.4,
        WHEEL_TERMINALS[WHEEL_TERMINALS.len() - 1].1 + 1.0,
    );
    let mut x = tx - 1.4;
    let mut k = 0;
    while x < WHEEL_HUB.0 {
        let x1 = (x + 0.3).min(WHEEL_HUB.0);
        let hex = if k % 2 == 0 { PLANK_A } else { PLANK_B };
        put(&mut out, [x, 0.0, z0, x1 - 0.01, 0.035, z1], hex);
        x += 0.3;
        k += 1;
    }
    put(
        &mut out,
        [tx - 0.55, 0.035, z0 + 0.4, tx + 0.45, 0.045, z1 - 0.4],
        CARPET,
    );
    let m = slot_marquee();
    let mut z = z1;
    let mut k = 0;
    while z < m[5] + 0.2 {
        let hex = if k % 2 == 0 { PLANK_B } else { PLANK_A };
        put(&mut out, [m[0], 0.0, z, m[3], 0.035, z + 0.29], hex);
        z += 0.3;
        k += 1;
    }

    // The stand: a steel foot, a dark board behind the disc framed in gilt,
    // two timber posts carrying the sign, a cabinet behind.
    let (hx, hy, hz) = WHEEL_HUB;
    let s = CASINO[0].b;
    let half = WHEEL_RADIUS_M + RING_OUT_M + 0.18;
    put(&mut out, [s[0], 0.0, s[2], s[3], 0.45, s[5]], STEEL);
    put(
        &mut out,
        [s[0] + 0.15, 0.45, s[2] + 0.3, s[3], 2.2, s[5] - 0.3],
        WOOD_DARK,
    );
    let board = [hx, hy - half, hz - half, hx + 0.12, hy + half, hz + half];
    put(&mut out, board, BOARD);
    for (y0, y1, z0, z1) in [
        (board[1] - 0.08, board[1], board[2], board[5]),
        (board[4], board[4] + 0.08, board[2], board[5]),
        (board[1], board[4], board[2] - 0.08, board[2]),
        (board[1], board[4], board[5], board[5] + 0.08),
    ] {
        put(&mut out, [hx - 0.02, y0, z0, hx + 0.13, y1, z1], GILT);
    }
    for pz in [s[2] + 0.02, s[5] - 0.28] {
        put(
            &mut out,
            [hx + 0.14, 0.0, pz, hx + 0.4, 7.2, pz + 0.26],
            WOOD,
        );
        put(
            &mut out,
            [hx + 0.1, 7.2, pz - 0.03, hx + 0.44, 7.32, pz + 0.29],
            GILT,
        );
    }
    let sign = wheel_sign();
    put(&mut out, sign, SIGN_RED);
    put(
        &mut out,
        [
            sign[0] - 0.02,
            sign[1] - 0.06,
            sign[2] - 0.06,
            sign[3],
            sign[1],
            sign[5] + 0.06,
        ],
        GILT,
    );
    put(
        &mut out,
        [
            sign[0] - 0.02,
            sign[4],
            sign[2] - 0.06,
            sign[3],
            sign[4] + 0.06,
            sign[5] + 0.06,
        ],
        GILT,
    );
    // The pointer: a gilt flapper at the top of the disc, a red tip.
    let top = hy + WHEEL_RADIUS_M;
    put(
        &mut out,
        [
            hx - DISC_OFF_M - 0.2,
            top - 0.02,
            hz - 0.07,
            hx,
            top + 0.34,
            hz + 0.07,
        ],
        GILT,
    );
    put(
        &mut out,
        [
            hx - DISC_OFF_M - 0.18,
            top - 0.16,
            hz - 0.05,
            hx - DISC_OFF_M - 0.06,
            top - 0.02,
            hz + 0.05,
        ],
        TIP,
    );

    // The marquee over the slot row, gilt-edged.
    put(&mut out, m, SIGN_RED);
    put(
        &mut out,
        [
            m[0] - 0.04,
            m[1] - 0.04,
            m[2] - 0.02,
            m[3] + 0.04,
            m[1],
            m[5],
        ],
        GILT,
    );
    put(
        &mut out,
        [
            m[0] - 0.04,
            m[4],
            m[2] - 0.02,
            m[3] + 0.04,
            m[4] + 0.04,
            m[5],
        ],
        GILT,
    );
    out
}

/// The signs' letters, a little lit (`town_signs`' block font).
fn ink_boxes() -> Boxes {
    let mut out = Boxes::new();
    let s = wheel_sign();
    letters(
        &mut out,
        Face {
            at: Vec3::new(s[0], (s[1] + s[4]) * 0.5, (s[2] + s[5]) * 0.5),
            normal: Vec3::NEG_X,
        },
        &[("BIG WHEEL", GILT)],
        0.075,
        0.03,
    );
    let m = slot_marquee();
    letters(
        &mut out,
        Face {
            at: Vec3::new((m[0] + m[3]) * 0.5, (m[1] + m[4]) * 0.5, m[2]),
            normal: Vec3::NEG_Z,
        },
        &[("SLOTS", GILT)],
        0.075,
        0.03,
    );
    out
}

/// The bulbs: round the disc on the board, along the stand's sign and the
/// marquee. Two sets, alternating, so they can chase.
fn bulb_boxes() -> (Boxes, Boxes) {
    let (mut even, mut odd) = (Boxes::new(), Boxes::new());
    let mut n = 0usize;
    let mut bulb = |c: Vec3| {
        let set = if n.is_multiple_of(2) {
            &mut even
        } else {
            &mut odd
        };
        set.push((c.to_array(), [0.05, 0.05, 0.05], 0xfff1c8));
        n += 1;
    };
    let (hx, hy, hz) = WHEEL_HUB;
    let r = WHEEL_RADIUS_M + RING_OUT_M;
    for i in 0..RING_BULBS {
        let a = i as f32 * TAU / RING_BULBS as f32;
        bulb(Vec3::new(hx - 0.05, hy + r * a.cos(), hz + r * a.sin()));
    }
    let s = wheel_sign();
    for i in 0..18 {
        let z = s[2] + (s[5] - s[2]) * (i as f32 + 0.5) / 18.0;
        bulb(Vec3::new(s[0] - 0.06, s[4] + 0.12, z));
        bulb(Vec3::new(s[0] - 0.06, s[1] - 0.12, z));
    }
    let m = slot_marquee();
    for i in 0..30 {
        let x = m[0] + (m[3] - m[0]) * (i as f32 + 0.5) / 30.0;
        bulb(Vec3::new(x, m[4] + 0.1, m[2] - 0.06));
        bulb(Vec3::new(x, m[1] - 0.08, m[2] - 0.06));
    }
    (even, odd)
}

/// A 3×5 digit, rows top to bottom, the high bit leftmost.
fn digit(d: u32) -> [u8; 5] {
    match d {
        0 => [7, 5, 5, 5, 7],
        1 => [2, 6, 2, 2, 7],
        2 => [7, 1, 7, 4, 7],
        3 => [7, 1, 7, 1, 7],
        5 => [7, 4, 7, 1, 7],
        _ => [0; 5],
    }
}

/// The disc in its own frame: facing -X, segment `i` centred at angle
/// `i × 2π/25` from straight up toward +Z, its multiplier on it reading
/// outward, so the segment under the pointer reads upright.
fn disc_mesh() -> Mesh {
    let mut s = Soup::tiling(1.0);
    let r = WHEEL_RADIUS_M;
    let at = |phi: f32, rad: f32, x: f32| Vec3::new(x, rad * phi.cos(), rad * phi.sin());
    let w = TAU / WHEEL_SEGMENTS as f32;
    let flat = |c: [f32; 3]| move |_: Vec3| [c[0], c[1], c[2], 1.0];
    let srgb = |c: [f32; 3]| [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2)];
    // A quad facing -X: centre, its right and its up (both in the YZ plane).
    let quad = |s: &mut Soup, c: Vec3, right: Vec3, up: Vec3, col: [f32; 3]| {
        let (a, b) = (c - right - up, c + right - up);
        let (cc, d) = (c + right + up, c - right + up);
        s.tri(a, b, cc, flat(col), None, 0.0);
        s.tri(a, cc, d, flat(col), None, 0.0);
    };
    let back = linear(BOARD);
    let gilt = linear(GILT);
    for i in 0..64 {
        let (a, b) = (i as f32 * TAU / 64.0, (i + 1) as f32 * TAU / 64.0);
        s.tri(
            Vec3::ZERO,
            at(b, r, 0.0),
            at(a, r, 0.0),
            flat(back),
            None,
            0.0,
        );
        // The rim: a gilt band, and its edge so the disc has a thickness.
        s.tri(
            at(a, r - 0.1, -0.03),
            at(b, r, -0.03),
            at(a, r, -0.03),
            flat(gilt),
            None,
            0.0,
        );
        s.tri(
            at(a, r - 0.1, -0.03),
            at(b, r - 0.1, -0.03),
            at(b, r, -0.03),
            flat(gilt),
            None,
            0.0,
        );
        let (p, q) = (at(a, r, 0.0), at(b, r, 0.0));
        let (p1, q1) = (at(a, r, -0.06), at(b, r, -0.06));
        s.tri(p, p1, q1, flat(gilt), None, 0.0);
        s.tri(p, q1, q, flat(gilt), None, 0.0);
    }
    let gap = 0.006;
    for (i, &c) in WHEEL.iter().enumerate() {
        let ink = super::panels::casino::COLOURS[c as usize].1;
        let col = srgb(ink);
        let (a0, a1) = (i as f32 * w - w * 0.5 + gap, i as f32 * w + w * 0.5 - gap);
        for k in 0..3 {
            let a = a0 + (a1 - a0) * k as f32 / 3.0;
            let b = a0 + (a1 - a0) * (k + 1) as f32 / 3.0;
            let (ri, ro) = (0.36, r - 0.1);
            let (p0, p1) = (at(a, ri, -0.02), at(b, ri, -0.02));
            let (q0, q1) = (at(a, ro, -0.02), at(b, ro, -0.02));
            s.tri(p0, q1, q0, flat(col), None, 0.0);
            s.tri(p0, p1, q1, flat(col), None, 0.0);
        }
        // The multiplier, near the rim, reading outward.
        let phi = i as f32 * w;
        let up = Vec3::new(0.0, phi.cos(), phi.sin());
        let right = Vec3::new(0.0, -phi.sin(), phi.cos());
        let text = WHEEL_PAYS[c as usize];
        let digits: Vec<u32> = if text >= 10 {
            vec![text / 10, text % 10]
        } else {
            vec![text]
        };
        let cell = 0.062;
        let width = digits.len() as f32 * 4.0 * cell - cell;
        let ink = if c == 0 {
            linear(INK_DARK)
        } else {
            linear(INK_LIGHT)
        };
        let centre = r - 0.42;
        for (j, &d) in digits.iter().enumerate() {
            for (row, bits) in digit(d).iter().enumerate() {
                for col in 0..3 {
                    if bits & (4 >> col) == 0 {
                        continue;
                    }
                    let along = -width * 0.5 + (j as f32 * 4.0 + col as f32 + 0.5) * cell;
                    let rise = (2.0 - row as f32) * cell;
                    let c = up * (centre + rise) + right * along + Vec3::new(-0.03, 0.0, 0.0);
                    quad(&mut s, c, right * cell * 0.47, up * cell * 0.47, ink);
                }
            }
        }
        // A gilt peg between this segment and the next.
        let b = phi + w * 0.5;
        let (pu, pr) = (
            Vec3::new(0.0, b.cos(), b.sin()),
            Vec3::new(0.0, -b.sin(), b.cos()),
        );
        quad(
            &mut s,
            pu * (r - 0.05) + Vec3::new(-0.07, 0.0, 0.0),
            pr * 0.035,
            pu * 0.035,
            gilt,
        );
    }
    // The hub: gilt, a dark boss, a gilt cap.
    for i in 0..32 {
        let (a, b) = (i as f32 * TAU / 32.0, (i + 1) as f32 * TAU / 32.0);
        let c = Vec3::new(-0.05, 0.0, 0.0);
        s.tri(
            c,
            at(b, 0.38, -0.05),
            at(a, 0.38, -0.05),
            flat(gilt),
            None,
            0.0,
        );
        let c = Vec3::new(-0.07, 0.0, 0.0);
        s.tri(
            c,
            at(b, 0.2, -0.07),
            at(a, 0.2, -0.07),
            flat(back),
            None,
            0.0,
        );
        let c = Vec3::new(-0.09, 0.0, 0.0);
        s.tri(
            c,
            at(b, 0.08, -0.09),
            at(a, 0.08, -0.09),
            flat(gilt),
            None,
            0.0,
        );
    }
    s.mesh()
}

/// The disc's angle with segment `seg` under the pointer (straight up).
fn rest_angle(seg: u8) -> f32 {
    -(seg as f32) * TAU / WHEEL_SEGMENTS as f32
}

#[allow(clippy::too_many_arguments)]
pub fn spawn(
    mut commands: Commands,
    world: Option<Res<WorldId>>,
    drawn: Query<(), With<CasinoVisual>>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(world) = world else { return };
    let t = world.haven.town;
    if !t.live || !drawn.is_empty() {
        return;
    }
    let tf = super::town::transform(&t);
    let mut solid = |commands: &mut Commands, mesh: Mesh, mat: StandardMaterial| {
        commands.spawn((
            WorldEntity,
            CasinoVisual,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(mat)),
            tf,
        ));
    };
    solid(
        &mut commands,
        boxes_mesh_with(&fixed_boxes(), linear, 1.0),
        StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.75,
            ..default()
        },
    );
    // Painted letters catch the lamps; a little glow keeps them legible at
    // night, as the town's own signs do.
    solid(
        &mut commands,
        boxes_mesh_with(&ink_boxes(), linear, 1.0),
        StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.4,
            metallic: 0.3,
            emissive: LinearRgba::rgb(0.5, 0.38, 0.12),
            ..default()
        },
    );
    let (even, odd) = bulb_boxes();
    let bulb = || StandardMaterial {
        base_color: Color::srgb(1.0, 0.95, 0.8),
        emissive: BULB_GLOW,
        ..default()
    };
    let (even_mat, odd_mat) = (materials.add(bulb()), materials.add(bulb()));
    for (boxes, mat) in [(even, &even_mat), (odd, &odd_mat)] {
        commands.spawn((
            WorldEntity,
            CasinoVisual,
            Mesh3d(meshes.add(boxes_mesh_with(&boxes, linear, 1.0))),
            MeshMaterial3d(mat.clone()),
            tf,
        ));
    }
    commands.insert_resource(CasinoBulbs {
        even: even_mat,
        odd: odd_mat,
    });

    // The generated cabinets and terminals, each turned to face its player.
    let place = |commands: &mut Commands, glb: &'static str, x: f32, z: f32, yaw: f32| {
        let (mesh, mat) = super::mobs::model(&server, glb);
        commands.spawn((
            WorldEntity,
            CasinoVisual,
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            tf.mul_transform(
                Transform::from_xyz(x, 0.0, z).with_rotation(Quat::from_rotation_y(yaw)),
            ),
        ));
    };
    for &(x, z) in SLOT_SPOTS.iter() {
        place(
            &mut commands,
            SLOT_GLB,
            x,
            z + CASINO_GAP_M + CABINET_D * 0.5,
            PI,
        );
    }
    for &(x, z) in WHEEL_TERMINALS.iter() {
        place(
            &mut commands,
            TERMINAL_GLB,
            x + CASINO_GAP_M + TERMINAL_D * 0.5,
            z,
            -FRAC_PI_2,
        );
    }

    // The disc catches the yard's lamps a little, so it reads at night.
    let lit = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.35,
        emissive: LinearRgba::rgb(0.1, 0.09, 0.07),
        ..default()
    });
    let (hx, hy, hz) = WHEEL_HUB;
    let base = tf.mul_transform(Transform::from_xyz(hx - DISC_OFF_M, hy, hz));
    commands.spawn((
        WorldEntity,
        CasinoVisual,
        WheelDisc {
            base,
            round: None,
            from: 0.0,
            to: 0.0,
        },
        Mesh3d(meshes.add(disc_mesh())),
        MeshMaterial3d(lit),
        base,
    ));
    for (x, y, z) in LAMPS {
        commands.spawn((
            WorldEntity,
            CasinoVisual,
            CasinoLamp,
            PointLight {
                color: super::town::LAMP_COLOR,
                intensity: 0.0,
                range: LAMP_RANGE_M,
                shadows_enabled: false,
                ..default()
            },
            tf.mul_transform(Transform::from_xyz(x, y, z)),
        ));
    }
}

/// Where the wheel's latest spin stands: `Some(u)` while it turns, `u` its
/// progress 0 to 1; `None` before it starts.
fn spin_progress(round: u32, now: f64) -> Option<f32> {
    let start = round as f64 * WHEEL_ROUND_TICKS as f64 + WHEEL_OPEN_TICKS as f64;
    let len = (WHEEL_ROUND_TICKS - WHEEL_OPEN_TICKS - 1) as f64;
    (now >= start).then(|| ((now - start) / len).min(1.0) as f32)
}

/// Turn the disc: still between spins, and through a spin from where it
/// rested to `TURNS` turns and the drawn segment, slowing as Rust's does.
pub fn spin(net: NonSend<Net>, mut q: Query<(&mut WheelDisc, &mut Transform)>) {
    let core = &net.session.core;
    let now = core.server_tick_est().max(0.0);
    for (mut d, mut tf) in q.iter_mut() {
        let mut angle = d.to;
        if let Some((round, seg)) = core.casino.spin {
            if d.round != Some(round) {
                // A new spin starts from wherever the disc stands.
                let from = d.to.rem_euclid(TAU);
                let land = from - TURNS * TAU;
                let to = land - (land - rest_angle(seg)).rem_euclid(TAU);
                *d = WheelDisc {
                    base: d.base,
                    round: Some(round),
                    from,
                    to,
                };
            }
            let u = spin_progress(round, now).unwrap_or(0.0);
            let ease = 1.0 - (1.0 - u).powi(3);
            angle = d.from + (d.to - d.from) * ease;
        }
        let want = d
            .base
            .mul_transform(Transform::from_rotation(Quat::from_rotation_x(angle)));
        if *tf != want {
            *tf = want;
        }
    }
}

/// The bulbs and the lamps: the lamps follow the dark as the yard's do; the
/// bulbs glow all day, chase while the wheel turns, flash as it lands, and
/// breathe otherwise.
pub fn lights(
    net: NonSend<Net>,
    time: Res<Time>,
    glow: Option<Res<super::town::TownGlow>>,
    gain: Res<super::rig::FlameGain>,
    bulbs: Option<Res<CasinoBulbs>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut lamps: Query<&mut PointLight, With<CasinoLamp>>,
) {
    let Some(bulbs) = bulbs else { return };
    let night = glow.as_ref().map_or(0.0, |g| g.night());
    let g = gain.0;
    for mut l in lamps.iter_mut() {
        let want = LAMP_LUMENS * night * g;
        if (l.intensity - want).abs() > 1.0 {
            l.intensity = want;
        }
    }
    let core = &net.session.core;
    let now = core.server_tick_est().max(0.0);
    let t = time.elapsed_secs();
    // Lit by day; by night a share of the eye's gain, not all of it: the
    // yard's lone bulbs take the whole of it, and a hundred and forty of
    // them a hand apart at that bloom into one glare.
    let base = 0.35 * (1.0 - night) + NIGHT_SHARE * night * g;
    let landed_ticks = (LANDED_S * TICK_HZ as f32) as f64;
    let (e, o) = match core.casino.spin.and_then(|(r, _)| {
        let u = spin_progress(r, now)?;
        let end = r as f64 * WHEEL_ROUND_TICKS as f64 + WHEEL_ROUND_TICKS as f64;
        (u < 1.0 || now < end + landed_ticks).then_some(u)
    }) {
        Some(u) if u < 1.0 => {
            if ((t * 8.0) as u32).is_multiple_of(2) {
                (1.0, 0.1)
            } else {
                (0.1, 1.0)
            }
        }
        Some(_) => {
            let f = if ((t * 3.0) as u32).is_multiple_of(2) {
                1.0
            } else {
                0.2
            };
            (f, f)
        }
        None => (
            0.75 + 0.25 * (t * 1.5).sin(),
            0.75 + 0.25 * (t * 1.5 + PI).sin(),
        ),
    };
    for (h, k) in [(&bulbs.even, e), (&bulbs.odd, o)] {
        if let Some(m) = materials.get_mut(h) {
            m.emissive = BULB_GLOW * (base * k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A segment at rest sits under the pointer: rotating its centre by its
    /// rest angle brings it straight up.
    #[test]
    fn a_resting_segment_is_under_the_pointer() {
        for seg in 0..WHEEL_SEGMENTS as u8 {
            let phi = seg as f32 * TAU / WHEEL_SEGMENTS as f32;
            let p = Vec3::new(0.0, phi.cos(), phi.sin());
            let up = Quat::from_rotation_x(rest_angle(seg)) * p;
            assert!(
                (up - Vec3::Y).length() < 1e-4,
                "segment {seg} rests at {up}"
            );
        }
    }

    /// Every multiplier the wheel pays has its digits.
    #[test]
    fn every_multiplier_has_digits() {
        for p in WHEEL_PAYS {
            let digits = if p >= 10 {
                vec![p / 10, p % 10]
            } else {
                vec![p]
            };
            for d in digits {
                assert_ne!(digit(d), [0; 5], "{p}");
            }
        }
    }

    /// The board, its gilt frame and the bulbs stay inside the box the sim
    /// stops a body at, so nobody walks through the stand.
    #[test]
    fn the_board_fits_the_stand() {
        let s = CASINO[0].b;
        let half = WHEEL_RADIUS_M + RING_OUT_M + 0.18 + 0.08;
        assert!(
            WHEEL_HUB.2 - half >= s[2],
            "{} < {}",
            WHEEL_HUB.2 - half,
            s[2]
        );
        assert!(
            WHEEL_HUB.2 + half <= s[5],
            "{} > {}",
            WHEEL_HUB.2 + half,
            s[5]
        );
    }
}
