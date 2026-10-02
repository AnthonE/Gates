//! Authored kits: the box tables the town (`town.rs`) and the monuments
//! (`monument.rs`) are built from. The depot's and the landmarks' shape —
//! the boxes are the collision, the client draws the same boxes (or a
//! Blender dressing of them, `ci/site_kit.py`) — generalised once so the
//! big sites share one `blocks`/`ground`/`roofed` and one frame.
//!
//! **Quarter turns, as integers.** A kit's rotation is `rot` in 0..4 and the
//! frame maps by swapping and negating, never through the yaw LUT: index 64
//! of the LUT is (1.0, 6.1e-17), not (1, 0), and a kit whose station anchors
//! must land exactly on 3 m build cells cannot afford that tail. Handedness
//! matches `depot::to_world` (local +Z is yaw index `rot * 64`).

use crate::movement::STEP_UP;

/// What a part is made of — the client picks a surface by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum KitMat {
    Yard = 0,
    Concrete = 1,
    Sheet = 2,
    Cargo = 3,
    Timber = 4,
    Steel = 5,
    /// Dark ancient stone (the gate's pylons, the ziggurat).
    Obsidian = 6,
    /// Tarnished gold trim; metallic.
    Gilt = 7,
    /// Awnings and tarps.
    Canvas = 8,
    /// Glowing conduits (the ziggurat's lapis lines).
    Lapis = 9,
}

impl KitMat {
    /// The material role name the Blender dressing and the client share.
    pub const fn role(self) -> &'static str {
        match self {
            KitMat::Yard => "yard",
            KitMat::Concrete => "concrete",
            KitMat::Sheet => "sheet",
            KitMat::Cargo => "cargo",
            KitMat::Timber => "timber",
            KitMat::Steel => "steel",
            KitMat::Obsidian => "obsidian",
            KitMat::Gilt => "gilt",
            KitMat::Canvas => "canvas",
            KitMat::Lapis => "lapis",
        }
    }
}

/// A part a body walks under: rain stops here (`roofed`).
pub const ROOF: u8 = 1;
/// Drawn, never collided: awnings, the gate's ring overhead, trim.
pub const DECOR: u8 = 2;
/// Style hint for the dresser: a container wall drawn as stacked boxes.
pub const STACK: u8 = 4;

/// No card door.
pub const NO_DOOR: u8 = 0;

/// One box: `[x0, y0, z0, x1, y1, z1]` metres in the kit's frame, `y` from
/// the kit's floor. `door` is 0, or 1 + the index of the card door this part
/// is (`monument.rs`); an open door is skipped by `blocks`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KitPart {
    pub b: [f32; 6],
    pub mat: KitMat,
    pub flags: u8,
    pub door: u8,
}

pub const fn part(b: [f32; 6], mat: KitMat) -> KitPart {
    KitPart {
        b,
        mat,
        flags: 0,
        door: NO_DOOR,
    }
}

pub const fn part_f(b: [f32; 6], mat: KitMat, flags: u8) -> KitPart {
    KitPart {
        b,
        mat,
        flags,
        door: NO_DOOR,
    }
}

/// Where a kit stands: its centre, floor and quarter turn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub x: f32,
    pub z: f32,
    pub floor_y: f32,
    pub rot: u8,
}

/// Local (x, z) to world.
pub fn to_world(p: &Placed, x: f32, z: f32) -> (f32, f32) {
    let (wx, wz) = match p.rot & 3 {
        0 => (x, z),
        1 => (z, -x),
        2 => (-x, -z),
        _ => (-z, x),
    };
    (p.x + wx, p.z + wz)
}

/// World (x, z) to local — `to_world`'s exact inverse.
pub fn to_local(p: &Placed, x: f32, z: f32) -> (f32, f32) {
    let (dx, dz) = (x - p.x, z - p.z);
    match p.rot & 3 {
        0 => (dx, dz),
        1 => (-dz, dx),
        2 => (-dx, -dz),
        _ => (dz, -dx),
    }
}

/// The world yaw-LUT index (high byte) a local +Z faces.
pub const fn yaw_byte(rot: u8) -> u8 {
    (rot & 3).wrapping_mul(64)
}

fn solid(part: &KitPart, doors_open: u32) -> bool {
    if part.flags & DECOR != 0 {
        return false;
    }
    part.door == NO_DOOR || doors_open & (1 << ((part.door - 1) & 31)) == 0
}

/// Whether a kit's boxes stop a volume of radius `r`, height `h`, feet at
/// `feet` — `landmark::blocks`'s semantics: a top within a step of the feet
/// is ground (`ground` answers it), not a wall. `lx`/`lz` are local.
#[allow(clippy::too_many_arguments)]
pub fn blocks_local(
    parts: &[KitPart],
    floor_y: f32,
    doors_open: u32,
    lx: f32,
    lz: f32,
    feet: f32,
    r: f32,
    h: f32,
) -> bool {
    for part in parts {
        if !solid(part, doors_open) {
            continue;
        }
        let b = part.b;
        if feet + STEP_UP >= floor_y + b[4] || feet + h <= floor_y + b[1] {
            continue;
        }
        let dx = lx - lx.clamp(b[0], b[3]);
        let dz = lz - lz.clamp(b[2], b[5]);
        if dx * dx + dz * dz < r * r {
            return true;
        }
    }
    false
}

/// The highest kit top within a step of `feet` under a disc of radius `r`
/// at local (`lx`, `lz`) — `r` 0 is the point under it.
///
/// A body passes the capsule radius, the footprint `blocks_local` stops it
/// with. A point footprint let a body step off a ledge with its centre past
/// the edge and the rest of it still over the stone: it fell beside the
/// face overlapping it, the "already inside" lift in `movement::step` let it
/// walk on into the solid, and it came out inside a ziggurat terrace.
pub fn ground_local(
    parts: &[KitPart],
    floor_y: f32,
    doors_open: u32,
    lx: f32,
    lz: f32,
    feet: f32,
    r: f32,
) -> f32 {
    let mut best = crate::collide::NO_SURFACE;
    for part in parts {
        if !solid(part, doors_open) {
            continue;
        }
        let b = part.b;
        let top = floor_y + b[4];
        let dx = lx - lx.clamp(b[0], b[3]);
        let dz = lz - lz.clamp(b[2], b[5]);
        if dx * dx + dz * dz <= r * r && top <= feet + STEP_UP {
            best = best.max(top);
        }
    }
    best
}

/// Whether something overhead keeps the rain off a body at local (`lx`,
/// `lz`) with feet at `feet`: any solid or roof part above the head.
pub fn roofed_local(parts: &[KitPart], floor_y: f32, lx: f32, lz: f32, feet: f32) -> bool {
    for part in parts {
        let b = part.b;
        if part.flags & DECOR != 0 && part.flags & ROOF == 0 {
            continue;
        }
        if lx >= b[0] && lx <= b[3] && lz >= b[2] && lz <= b[5] && floor_y + b[1] >= feet + 1.0 {
            return true;
        }
    }
    false
}

/// The half-extent of the square every part fits inside, metres.
pub const fn envelope(parts: &[KitPart]) -> f32 {
    let mut m = 0.0f32;
    let mut i = 0;
    while i < parts.len() {
        let b = parts[i].b;
        let mut k = 0;
        while k < 6 {
            if k != 1 && k != 4 {
                let v = if b[k] < 0.0 { -b[k] } else { b[k] };
                if v > m {
                    m = v;
                }
            }
            k += 1;
        }
        i += 1;
    }
    m
}

/// Every part well-formed: positive extent on all three axes.
pub const fn well_formed(parts: &[KitPart]) -> bool {
    let mut i = 0;
    while i < parts.len() {
        let b = parts[i].b;
        if !(b[0] < b[3] && b[1] < b[4] && b[2] < b[5]) {
            return false;
        }
        i += 1;
    }
    true
}

/// Write a kit as JSON for the Blender dresser (`ci/site_kit.py`) and the
/// drift test. `sim-core` has no `String`, so it writes into the caller's
/// `fmt::Write`.
pub fn dump(
    name: &str,
    parts: &[KitPart],
    anchors: &[(&str, f32, f32, f32)],
    w: &mut impl core::fmt::Write,
) -> core::fmt::Result {
    write!(w, "{{\"name\":\"{name}\",\"parts\":[")?;
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            w.write_str(",")?;
        }
        let b = p.b;
        write!(
            w,
            "\n{{\"b\":[{},{},{},{},{},{}],\"mat\":\"{}\",\"flags\":{},\"door\":{}}}",
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            p.mat.role(),
            p.flags,
            p.door
        )?;
    }
    w.write_str("],\"anchors\":[")?;
    for (i, (kind, x, y, z)) in anchors.iter().enumerate() {
        if i > 0 {
            w.write_str(",")?;
        }
        write!(w, "\n{{\"kind\":\"{kind}\",\"at\":[{x},{y},{z}]}}")?;
    }
    w.write_str("]}\n")
}
