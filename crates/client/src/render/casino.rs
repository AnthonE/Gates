//! THE GATE's casino, drawn (`sim_core::casino`, `town::CASINO`): Rust's
//! Bandit Camp big wheel on a timber stand under its sign, the betting
//! terminals before it, and the slot machines under a lit marquee, all on a
//! plank deck under a timber-trussed tin roof (`town::CASINO_ROOF`), with a
//! stool at every machine and a lounge of armchairs facing the wheel
//! (`town::SEATS`, `sim_core::seat`).
//!
//! The terminals, the cabinets and the seats are generated models
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
    self, SeatKind, Town, CABINET_D, CABINET_H, CASINO, CASINO_GAP_M, CASINO_ROOF, ROOF_EAVES_M,
    ROOF_OVERHANG_M, ROOF_RIDGE_M, ROOF_X, ROOF_Z, SEATS, SLOT_SPOTS, TERMINAL_D, WHEEL_HUB,
    WHEEL_RADIUS_M, WHEEL_TERMINALS,
};

use super::depot::{self, Surface};
use super::props::{boxes_mesh_with, linear, Soup};
use super::town_signs::{letters, Boxes, Face};
use super::{Net, WorldEntity, WorldId};

/// The generated cabinet and terminal, each facing +Z at its base.
pub const SLOT_GLB: &str = "models/site/slot_machine.glb";
pub const TERMINAL_GLB: &str = "models/site/bet_terminal.glb";
/// The generated seats, each facing +Z at its base, the seat 0.46 m up:
/// where `Sitting_Idle_Loop` sits (`anim::SIT_LIFT_M`).
pub const STOOL_GLB: &str = "models/site/casino_stool.glb";
pub const ARMCHAIR_GLB: &str = "models/site/casino_armchair.glb";

/// From a sitter's feet back to the middle of its seat, metres: where
/// `Sitting_Idle_Loop` puts the hips (`stumpy.glb`, skinned at the loop's
/// first frame, the seat of the pants 0.13–0.45 m behind the feet). A
/// stool goes under them; an armchair's back goes behind them, its inside
/// face 0.125 m behind the chair's middle.
fn seat_back_m(kind: SeatKind) -> f32 {
    match kind {
        SeatKind::Stool => 0.27,
        SeatKind::Armchair => 0.32,
    }
}
/// How near its spot a sitter's feet must be to be on a seat, metres: the
/// server puts them on it exactly, to the quantum (`sim_core::seat`).
const SEAT_SNAP_M: f32 = 0.3;

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
const PLANK_C: u32 = 0x634630;
const SHADE: u32 = 0x1f2326;

/// The deck's planks, metres: a board's width and its length, the joints
/// staggered by a third of it.
const PLANK_W: f32 = 0.3;
const PLANK_L: f32 = 2.9;
/// The lounge's rug, under the armchairs: `[x0, z0, x1, z1]`.
const RUG: [f32; 4] = [18.7, 10.1, 22.5, 17.9];
/// Section of the roof's timbers, metres.
const TIMBER_W: f32 = 0.2;
const TIMBER_H: f32 = 0.28;
/// Where the purlins carry the sheets, local z, both slopes.
const PURLINS_Z: [f32; 6] = [8.4, 10.6, 12.8, 17.0, 19.2, 21.4];
/// Bulbs along each eave.
const EAVE_BULBS: usize = 44;

/// The bulbs' glow at full brightness, linear; the yard's own bulbs'.
const BULB_GLOW: LinearRgba = LinearRgba::rgb(6.0, 4.5, 2.2);
/// How much of the night eye's gain (`rig::FlameGain`) the bulbs take.
const NIGHT_SHARE: f32 = 0.04;
/// The casino's lamps, hung from the roof: before the wheel, over the
/// lounge, over the slots.
const LAMPS: [(f32, f32, f32); 5] = [
    (27.0, 5.6, 14.0),
    (20.6, 4.4, 12.0),
    (20.6, 4.4, 16.0),
    (18.0, 3.9, 19.2),
    (22.0, 3.9, 19.2),
];
const LAMP_LUMENS: f32 = 2600.0;
const LAMP_RANGE_M: f32 = 14.0;
/// The share of their night light the lamps give by day: it is shade under
/// the roof.
const LAMP_DAY: f32 = 0.45;
/// The lamps' bulbs, lit, linear.
const LAMP_GLOW: LinearRgba = LinearRgba::rgb(9.0, 6.8, 3.4);

#[derive(Component)]
pub struct CasinoVisual;

/// A casino lamp's light, dark by day.
#[derive(Component)]
pub struct CasinoLamp;

/// The two bulb sets' materials, which [`lights`] chases, and the lamps'.
#[derive(Resource)]
pub struct CasinoBulbs {
    even: Handle<StandardMaterial>,
    odd: Handle<StandardMaterial>,
    lamps: Handle<StandardMaterial>,
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
    // The deck: staggered planks over the whole floor under the roof, a rug
    // with a gilt border under the lounge, and a carpet runner before the
    // terminals.
    let (fx0, fx1) = (ROOF_X[0], ROOF_X[ROOF_X.len() - 1]);
    let (fz0, fz1) = (ROOF_Z[0], ROOF_Z[1]);
    let mut x = fx0;
    let mut k = 0;
    while x < fx1 - 0.01 {
        let x1 = (x + PLANK_W).min(fx1);
        let mut z = fz0 - (k % 3) as f32 * PLANK_L / 3.0;
        let mut j = 0;
        while z < fz1 {
            let (a, b) = (z.max(fz0), (z + PLANK_L).min(fz1));
            if b - a > 0.05 {
                let hex = [PLANK_A, PLANK_B, PLANK_C][(k + j) % 3];
                put(
                    &mut out,
                    [x, 0.0, a + 0.004, x1 - 0.012, 0.035, b - 0.004],
                    hex,
                );
            }
            z += PLANK_L;
            j += 1;
        }
        x += PLANK_W;
        k += 1;
    }
    let [rx0, rz0, rx1, rz1] = RUG;
    put(&mut out, [rx0, 0.035, rz0, rx1, 0.043, rz1], CARPET);
    for b in [
        [rx0, rz0, rx1, rz0 + 0.08],
        [rx0, rz1 - 0.08, rx1, rz1],
        [rx0, rz0, rx0 + 0.08, rz1],
        [rx1 - 0.08, rz0, rx1, rz1],
    ] {
        put(&mut out, [b[0], 0.043, b[1], b[2], 0.046, b[3]], GILT);
    }
    let (tx, _) = WHEEL_TERMINALS[0];
    let (z0, z1) = (
        WHEEL_TERMINALS[0].1 - 1.4,
        WHEEL_TERMINALS[WHEEL_TERMINALS.len() - 1].1 + 1.0,
    );
    put(
        &mut out,
        [tx - 0.55, 0.035, z0 + 0.4, tx + 0.45, 0.045, z1 - 0.4],
        CARPET,
    );

    // The roof's painted fascia along both eaves, gilt-edged, and the
    // lamps: a steel shade on a rod down from the roof.
    for z in [ROOF_Z[0] - ROOF_OVERHANG_M, ROOF_Z[1] + ROOF_OVERHANG_M] {
        let y = roof_y(z);
        let (xa, xb) = (fx0 - ROOF_OVERHANG_M, fx1 + ROOF_OVERHANG_M);
        put(
            &mut out,
            [xa, y - 0.34, z - 0.05, xb, y + 0.04, z + 0.05],
            SIGN_RED,
        );
        put(
            &mut out,
            [xa, y - 0.38, z - 0.07, xb, y - 0.34, z + 0.07],
            GILT,
        );
        put(
            &mut out,
            [xa, y + 0.04, z - 0.07, xb, y + 0.08, z + 0.07],
            GILT,
        );
    }
    for (lx, ly, lz) in LAMPS {
        put(
            &mut out,
            [
                lx - 0.02,
                ly + 0.2,
                lz - 0.02,
                lx + 0.02,
                roof_y(lz),
                lz + 0.02,
            ],
            STEEL,
        );
        put(
            &mut out,
            [
                lx - 0.24,
                ly + 0.06,
                lz - 0.24,
                lx + 0.24,
                ly + 0.14,
                lz + 0.24,
            ],
            SHADE,
        );
        put(
            &mut out,
            [lx - 0.1, ly + 0.14, lz - 0.1, lx + 0.1, ly + 0.22, lz + 0.1],
            SHADE,
        );
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
    let m = slot_marquee();
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

/// The roof's height over the floor at local `z`, metres: the eaves over the
/// posts, the ridge midway, and on down over the overhang.
fn roof_y(z: f32) -> f32 {
    let mid = (ROOF_Z[0] + ROOF_Z[1]) * 0.5;
    let half = (ROOF_Z[1] - ROOF_Z[0]) * 0.5;
    ROOF_RIDGE_M - (ROOF_RIDGE_M - ROOF_EAVES_M) * (z - mid).abs() / half
}

/// A timber of `w` × `h` from `a` to `b`, its width level.
fn beam(soups: &mut [Soup; depot::SURFACES.len()], a: Vec3, b: Vec3, w: f32, h: f32) {
    let along = (b - a).normalize_or_zero();
    let side = along.cross(Vec3::Y).normalize_or_zero() * (w * 0.5);
    let up = side.cross(along).normalize_or_zero() * (h * 0.5);
    let c = |p: Vec3, sx: f32, sy: f32| p + side * sx + up * sy;
    let (tint, sf) = ([1.0; 3], Surface::Timber);
    for (p, q) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .iter()
        .zip([(1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0)].iter())
    {
        depot::quad(
            soups,
            sf,
            [
                c(a, p.0, p.1),
                c(b, p.0, p.1),
                c(b, q.0, q.1),
                c(a, q.0, q.1),
            ],
            tint,
            true,
        );
    }
}

/// The roof in the town's frame, in the photographed surfaces: the posts,
/// the wall plates on them, a king-post truss over each pair, the purlins
/// and the ridge beam across the trusses, and the corrugated sheets on top.
fn roof_meshes() -> Vec<(Surface, Mesh)> {
    let mut soups = depot::kit();
    let wood = [1.0; 3];
    for p in CASINO_ROOF
        .iter()
        .filter(|p| p.flags & sim_core::kit::DECOR == 0)
    {
        depot::cuboid(&mut soups, p.b, Surface::Timber, wood);
    }
    let (x0, x1) = (ROOF_X[0], ROOF_X[ROOF_X.len() - 1]);
    let (z0, z1) = (ROOF_Z[0], ROOF_Z[1]);
    let zr = (z0 + z1) * 0.5;
    let e = ROOF_EAVES_M;
    let (w, h) = (TIMBER_W, TIMBER_H);
    for z in ROOF_Z {
        depot::cuboid(
            &mut soups,
            [x0 - 0.2, e - h, z - w * 0.5, x1 + 0.2, e, z + w * 0.5],
            Surface::Timber,
            wood,
        );
    }
    for x in ROOF_X {
        // The tie beam, the king post, the rafters, two struts.
        depot::cuboid(
            &mut soups,
            [x - w * 0.5, e - h, z0, x + w * 0.5, e, z1],
            Surface::Timber,
            wood,
        );
        depot::cuboid(
            &mut soups,
            [
                x - w * 0.5,
                e,
                zr - w * 0.5,
                x + w * 0.5,
                ROOF_RIDGE_M - h,
                zr + w * 0.5,
            ],
            Surface::Timber,
            wood,
        );
        for (za, zq) in [(z0, (z0 + zr) * 0.5), (z1, (z1 + zr) * 0.5)] {
            let a = Vec3::new(x, roof_y(za) - h * 0.5, za);
            let r = Vec3::new(x, ROOF_RIDGE_M - h * 0.5, zr);
            beam(&mut soups, a, r, w, h);
            let foot = Vec3::new(x, e + 0.1, zr);
            beam(
                &mut soups,
                foot,
                Vec3::new(x, roof_y(zq) - h, zq),
                w * 0.8,
                h * 0.7,
            );
        }
    }
    for z in PURLINS_Z {
        let y = roof_y(z) - 0.02;
        depot::cuboid(
            &mut soups,
            [x0 - 0.3, y - 0.18, z - 0.07, x1 + 0.3, y, z + 0.07],
            Surface::Timber,
            wood,
        );
    }
    depot::cuboid(
        &mut soups,
        [
            x0 - 0.3,
            ROOF_RIDGE_M - h,
            zr - 0.09,
            x1 + 0.3,
            ROOF_RIDGE_M,
            zr + 0.09,
        ],
        Surface::Timber,
        wood,
    );
    // The sheets: one face a slope, both sides lit, and a cap on the ridge.
    let (xa, xb) = (x0 - ROOF_OVERHANG_M, x1 + ROOF_OVERHANG_M);
    for zo in [z0 - ROOF_OVERHANG_M, z1 + ROOF_OVERHANG_M] {
        let (yo, yr) = (roof_y(zo), ROOF_RIDGE_M);
        depot::quad(
            &mut soups,
            Surface::Sheet,
            [
                Vec3::new(xa, yo, zo),
                Vec3::new(xa, yr, zr),
                Vec3::new(xb, yr, zr),
                Vec3::new(xb, yo, zo),
            ],
            depot::DEPOT_ROOF_TINT,
            true,
        );
    }
    depot::cuboid(
        &mut soups,
        [
            xa,
            ROOF_RIDGE_M - 0.02,
            zr - 0.22,
            xb,
            ROOF_RIDGE_M + 0.1,
            zr + 0.22,
        ],
        Surface::Steel,
        depot::DEPOT_STEEL_TINT,
    );
    depot::finish(soups)
}

/// The way a seat facing `face` quarter turns faces in the world, radians
/// about up in the rig's sense (`rotation_y`, 0 toward +Z).
fn face_yaw(t: &Town, face: u8) -> f32 {
    (face.wrapping_add(t.rot) & 3) as f32 * FRAC_PI_2
}

/// The seat whose spot (world) `x`, `z` stands on, if any.
fn seat_at(t: &Town, x: f32, z: f32) -> Option<usize> {
    (0..SEATS.len()).find(|&k| {
        town::seat_world(t, k).is_some_and(|(sx, sz)| {
            (sx - x) * (sx - x) + (sz - z) * (sz - z) < SEAT_SNAP_M * SEAT_SNAP_M
        })
    })
}

/// The way a sitter at world `x`, `z` faces: its seat's way, since the
/// server puts a sitter's feet on its seat's spot.
pub fn seat_yaw(t: &Town, x: f32, z: f32) -> Option<f32> {
    seat_at(t, x, z).map(|k| face_yaw(t, SEATS[k].face))
}

/// Whether somebody other than this client sits in seat `k`, off the wire's
/// `seated` bits.
pub fn seat_taken(core: &client_core::core::ClientCore, k: usize) -> bool {
    let t = &core.haven().town;
    core.view.entities.iter().any(|(id, e)| {
        *id != core.player_id
            && e.seated
            && seat_at(
                t,
                e.qx as f32 * sim_core::movement::POS_XZ_Q,
                e.qz as f32 * sim_core::movement::POS_XZ_Q,
            ) == Some(k)
    })
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
    // Along both eaves, under the fascia.
    let (xa, xb) = (
        ROOF_X[0] - ROOF_OVERHANG_M,
        ROOF_X[ROOF_X.len() - 1] + ROOF_OVERHANG_M,
    );
    for (z, out) in [
        (ROOF_Z[0] - ROOF_OVERHANG_M, -0.1),
        (ROOF_Z[1] + ROOF_OVERHANG_M, 0.1),
    ] {
        for i in 0..EAVE_BULBS {
            let x = xa + (xb - xa) * (i as f32 + 0.5) / EAVE_BULBS as f32;
            bulb(Vec3::new(x, roof_y(z) - 0.2, z + out));
        }
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
    // The roof, in the town's own photographed surfaces, lit both sides:
    // the sheets are single faces seen from under and over.
    for (surface, mesh) in roof_meshes() {
        let mut m = depot::material(surface, &server);
        m.double_sided = true;
        m.cull_mode = None;
        solid(&mut commands, mesh, m);
    }
    let (even, odd) = bulb_boxes();
    let bulb = || StandardMaterial {
        base_color: Color::srgb(1.0, 0.95, 0.8),
        emissive: BULB_GLOW,
        ..default()
    };
    let (even_mat, odd_mat) = (materials.add(bulb()), materials.add(bulb()));
    // The lamps' bulbs: one under each shade, steady.
    let mut lamp_bulbs = Boxes::new();
    for (x, y, z) in LAMPS {
        lamp_bulbs.push(([x, y, z], [0.09, 0.07, 0.09], 0xfff1c8));
    }
    let lamp_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.95, 0.8),
        emissive: LAMP_GLOW,
        ..default()
    });
    for (boxes, mat) in [(even, &even_mat), (odd, &odd_mat), (lamp_bulbs, &lamp_mat)] {
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
        lamps: lamp_mat,
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
    // Every seat behind its sitter's feet, facing the way they do.
    for seat in SEATS {
        let yaw = seat.face as f32 * FRAC_PI_2;
        let back = Quat::from_rotation_y(yaw) * Vec3::Z * seat_back_m(seat.kind);
        let glb = match seat.kind {
            SeatKind::Stool => STOOL_GLB,
            SeatKind::Armchair => ARMCHAIR_GLB,
        };
        place(&mut commands, glb, seat.x - back.x, seat.z - back.z, yaw);
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

/// The bulbs and the lamps: the lamps burn low by day, under the roof, and
/// follow the dark as the yard's do by night; the bulbs glow all day, chase
/// while the wheel turns, flash as it lands, and breathe otherwise.
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
        let want = LAMP_LUMENS * (LAMP_DAY * (1.0 - night) + night * g);
        if (l.intensity - want).abs() > 1.0 {
            l.intensity = want;
        }
    }
    if let Some(m) = materials.get_mut(&bulbs.lamps) {
        let want = LAMP_GLOW * (0.4 * (1.0 - night) + NIGHT_SHARE * 2.0 * night * g);
        if m.emissive != want {
            m.emissive = want;
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
