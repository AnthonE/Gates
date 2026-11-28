//! The lab under the island, v0 (`ARC.md` §2 act IV, "raid the labs under
//! the island"; §5 M3, "prefab interiors under the terrain behind those
//! doors"): one prefab of three rooms cut under one broken keep, entered
//! down a stair from a blockhouse beside the keep's sealed vault. Its steel
//! door is shut until THE SEVERED GATE opens (`works::gate_open_of`); behind
//! it are two elite crates and the strays kept down there
//! (`mob::LAB_SLOT0`).
//!
//! **No new mechanism.** The island is a heightfield and a body never
//! stands under it, so a room below the ground is ground carved out of it
//! with a roof over the cut, which is how the ziggurat's rooms and the
//! quarry's faces already stand:
//! - **the cut** is a carve over the host's own (`carve`, after
//!   `landmark::stamp` in `terrain::ground`): two hard-edged pits to
//!   [`PIT_Y`], one under the keep's court and one for the stair, and a
//!   level apron for the blockhouse;
//! - **the roof** over the court's pit is the keep's own: its flagged court
//!   and its walls (`landmark::RUIN`), whose footings stop at
//!   `-FOOTING_M`, above the lab's ceiling. The blockhouse and two sills in
//!   the keep's gaps cover the rest;
//! - **the rooms** are `kit.rs` boxes in the host's frame ([`PARTS`]), the
//!   collision and the drawing at once, with the door a kit door: bit
//!   [`LAB_DOOR_BIT`] of the card-door mask, set while the Gate is open
//!   (`World::door_bits`);
//! - **the crates** are scatter slots seated on the lab's floor
//!   (`terrain::scatter`), so looting, refills and the wire are every
//!   crate's; the host's court crates went down with it.
//!
//! A drawn heightfield bends between its 1 m vertices, so each pit's edge
//! dents the ground up to a diagonal (√2 m) outside it and ramps up to one
//! inside it. Every wall standing on a pit's edge is that thick on each
//! side of it, and the court's pit stops that far inside the keep's flags
//! and walls: neither shows. The far terrain's coarser lattice is not
//! hidden that way (follow-up).
//!
//! Every number here is a first cut.

use crate::kit::{self, KitMat as M, KitPart, DECOR, NO_DOOR};
use crate::landmark::{self, Landmark, LandmarkKind};
use crate::terrain::{self, Occupant};

/// The kind the lab is cut under: the broken keep, the commonest kind on
/// the islands we ship (`examples/lab_probe.rs`).
pub const HOST_KIND: LandmarkKind = LandmarkKind::Ruin;

/// The pits' floor, metres from the host's base.
pub const PIT_Y: f32 = -8.2;
/// The lab's floor, on its slab over the pit.
pub const FLOOR_Y: f32 = -8.0;
/// The lab's ceiling (its underside).
pub const CEIL_Y: f32 = -4.3;
/// The lowest base a host may stand on: the pit stays a metre over the
/// beach, so nothing down there wades or draws the sea.
pub const MIN_BASE_Y: f32 = terrain::BEACH_MAX_H + 1.0 - PIT_Y;
/// How far past the host's disc the carve reaches, metres.
pub const REACH_M: f32 = 12.0;

/// The lab's door: kit door `LAB_DOOR` (the card doors' numbering, after
/// theirs), so bit [`LAB_DOOR_BIT`] of the open mask.
pub const LAB_DOOR: u8 = crate::monument::CARD_DOORS as u8 + 1;
pub const LAB_DOOR_BIT: u32 = 1 << (LAB_DOOR - 1);
/// The open mask's width: the card doors and the lab's (the wire's, too).
pub const DOOR_BITS: usize = crate::monument::CARD_DOORS + 1;

/// A dweller's leash, centimetres: the lab and its stair from any room.
pub const LEASH_CM: i64 = 1_400;

/// The pits, `[x0, z0, x1, z1]` in the host's frame: the court's, and the
/// stair's, which runs south under the keep's wall to the blockhouse.
pub const PITS: [[f32; 4]; 2] = [[-8.5, -8.5, 8.5, 8.5], [3.0, -18.5, 8.5, -8.5]];
/// Where the ground is levelled to the host's base for the blockhouse and
/// the walk up to its door, and how far it eases back to the wild past each
/// side: a keep's ground falls away up to `FOOTING_M` round it
/// (`landmark::try_site`), and eight metres keeps that a walk.
const APRON: [f32; 4] = [1.5, -24.0, 10.0, -10.0];
const APRON_BAND_M: f32 = 8.0;
/// How far past a pit's edge nothing grows and no rock face is drawn: the
/// dent and the 5×5 stencil the faces read round it.
const SWEEP_PAD_M: f32 = 2.5;

const fn b(x0: f32, y0: f32, z0: f32, x1: f32, y1: f32, z1: f32, mat: M) -> KitPart {
    KitPart {
        b: [x0, y0, z0, x1, y1, z1],
        mat,
        flags: 0,
        door: NO_DOOR,
    }
}

const fn glow(x0: f32, z0: f32, x1: f32, z1: f32) -> KitPart {
    KitPart {
        b: [x0, CEIL_Y - 0.15, z0, x1, CEIL_Y, z1],
        mat: M::Lapis,
        flags: DECOR,
        door: NO_DOOR,
    }
}

/// One stair tread `i` of 16: 0.5 m risers on 0.5 m treads from the
/// landing's 0.1 m sill down north to the floor.
///
/// **The ceilings over the stair are set by the treads, not the reverse.**
/// A body stands on the highest tread its disc overlaps and asks its next
/// spot at the height it stands at, so a head reaches a capsule's radius
/// and a tick's walk past its tread's end at that tread's height. Each
/// ceiling starts where no tread that high reaches (`STAIR_REACH_M`; the
/// const block holds it): a head that touched one would be inside it, and
/// inside is where `movement::step` lifts every wall.
const fn step(i: u32) -> KitPart {
    let z0 = STAIR_Z0 + 0.5 * i as f32;
    b(
        4.4,
        PIT_Y,
        z0,
        7.0,
        0.1 - 0.5 * (i + 1) as f32,
        z0 + 0.5,
        M::Concrete,
    )
}

pub const STEPS: usize = 16;
/// Where the stair's first tread starts, local z.
const STAIR_Z0: f32 = -17.0;
/// How far past its tread's end a head reaches at that tread's height: the
/// disc stands on a tread while it overlaps it (a radius), reaches a radius
/// on, and asks a sprinting tick's walk further, rounded up.
const STAIR_REACH_M: f32 = 1.0;

/// The rooms, their stair and its blockhouse, in the host's frame.
pub const PARTS: [KitPart; 31 + STEPS] = [
    // The floors, on the pits.
    b(-7.0, PIT_Y, -7.0, 7.0, FLOOR_Y, 7.0, M::Concrete),
    b(4.4, PIT_Y, -9.0, 7.0, FLOOR_Y, -7.0, M::Concrete),
    // The ceilings: the court's, and the stair's under the keep's wall,
    // thick enough to meet the blockhouse's north wall.
    b(-8.5, CEIL_Y, -8.5, 8.5, -4.0, 8.5, M::Concrete),
    b(3.0, CEIL_Y, -9.8, 8.5, -3.4, -8.5, M::Concrete),
    // The outer walls, 1.5 m inside each pit's edge.
    b(-8.5, PIT_Y, -8.5, -7.0, CEIL_Y, 8.5, M::Concrete),
    b(-7.0, PIT_Y, 7.0, 8.5, CEIL_Y, 8.5, M::Concrete),
    b(7.0, PIT_Y, -10.0, 8.5, CEIL_Y, 7.0, M::Concrete),
    b(-7.0, PIT_Y, -8.5, 4.4, CEIL_Y, -7.0, M::Concrete),
    b(3.0, PIT_Y, -10.0, 4.4, -3.4, -8.5, M::Concrete),
    // The hall across the south, and the two rooms north of it, each
    // through its own doorway.
    b(-7.0, FLOOR_Y, -1.2, -4.6, CEIL_Y, -0.8, M::Concrete),
    b(-3.0, FLOOR_Y, -1.2, 3.0, CEIL_Y, -0.8, M::Concrete),
    b(4.6, FLOOR_Y, -1.2, 7.0, CEIL_Y, -0.8, M::Concrete),
    b(-0.2, FLOOR_Y, -0.8, 0.2, CEIL_Y, 7.0, M::Concrete),
    // What stands in the hall: a tank in its corner and a bench.
    b(-6.6, FLOOR_Y, -6.6, -4.6, -5.6, -4.6, M::Steel),
    b(-1.2, FLOOR_Y, -6.6, 1.2, -7.1, -5.4, M::Steel),
    // The lapis lines along the ceilings, drawn and never struck.
    glow(-6.8, -4.1, 6.8, -3.9),
    glow(-6.8, 2.9, -0.4, 3.1),
    glow(0.4, 2.9, 6.8, 3.1),
    // The blockhouse: its walls stand on the stair pit's edges, its roof,
    // the wall that closes the stair's ceiling, and the landing inside its
    // door.
    b(1.5, PIT_Y, -20.0, 4.4, 3.0, -10.0, M::Concrete),
    b(7.0, PIT_Y, -20.0, 10.0, 3.0, -10.0, M::Concrete),
    b(4.4, 2.6, -20.0, 7.0, 3.0, -10.3, M::Concrete),
    b(4.4, -3.4, -10.3, 7.0, 3.0, -9.8, M::Concrete),
    b(4.4, PIT_Y, -20.0, 7.0, 0.1, STAIR_Z0, M::Concrete),
    // The door's frame and the door.
    b(4.4, 0.1, -20.0, 4.75, 2.6, -19.4, M::Concrete),
    b(6.65, 0.1, -20.0, 7.0, 2.6, -19.4, M::Concrete),
    b(4.75, 2.3, -20.0, 6.65, 2.6, -19.4, M::Concrete),
    KitPart {
        b: [4.75, 0.1, -19.85, 6.65, 2.3, -19.55],
        mat: M::Steel,
        flags: 0,
        door: LAB_DOOR,
    },
    // Sills over the court pit's dent where the keep's walls are open: the
    // south gate and the east breach (`landmark::RUIN`).
    b(-3.0, CEIL_Y, -10.0, 3.0, 0.06, -8.8, M::Sandstone),
    b(8.8, CEIL_Y, -1.5, 10.0, 0.06, 3.5, M::Sandstone),
    // The tank's pipe up to the ceiling, and the stair's east wall from
    // the lab's ceiling up to the stair's.
    b(-5.8, -5.6, -5.8, -5.4, CEIL_Y, -5.4, M::Steel),
    b(7.0, CEIL_Y, -10.0, 8.5, -3.4, -8.5, M::Concrete),
    step(0),
    step(1),
    step(2),
    step(3),
    step(4),
    step(5),
    step(6),
    step(7),
    step(8),
    step(9),
    step(10),
    step(11),
    step(12),
    step(13),
    step(14),
    step(15),
];

/// The lab's containers: local (x, z), seated on the floor, one in each of
/// the north rooms' far corners.
pub const CRATES: [(f32, f32, Occupant); crate::limits::MAX_LAB_CRATES] = [
    (-6.0, 6.0, Occupant::EliteCrate),
    (6.0, 6.0, Occupant::EliteCrate),
];

/// Where each dweller is homed, local (x, z): two in the hall, one in
/// each room.
pub const DWELL: [(f32, f32); crate::limits::MAX_LAB_DWELLERS] =
    [(-3.5, -4.0), (3.5, -3.5), (-3.5, 3.5), (3.5, 3.0)];

/// Where the lab's lights hang, local (x, y, z) — the client's.
pub const LIGHTS: [(f32, f32, f32); 4] = [
    (0.0, -4.8, -4.0),
    (-3.5, -4.8, 3.5),
    (3.5, -4.8, 3.5),
    (5.7, -2.0, -13.0),
];

const fn in_rect(r: [f32; 4], x: f32, z: f32, pad: f32) -> bool {
    x >= r[0] - pad && x <= r[2] + pad && z >= r[1] - pad && z <= r[3] + pad
}

/// Whether local (x, z) is over one of the pits.
pub const fn over_pit(lx: f32, lz: f32) -> bool {
    in_rect(PITS[0], lx, lz, 0.0) || in_rect(PITS[1], lx, lz, 0.0)
}

const _: () = {
    assert!(kit::well_formed(&PARTS));
    // The lab's floor over the beach on every host (`choose`).
    assert!(MIN_BASE_Y + PIT_Y > terrain::BEACH_MAX_H);
    assert!(PIT_Y < FLOOR_Y && FLOOR_Y < CEIL_Y);
    // Under the keep's footings: the court's walls and flags stop above the
    // ceiling, so nothing of the keep hangs into a room.
    assert!(CEIL_Y < -landmark::FOOTING_M);
    // A body stands up in a room.
    assert!(CEIL_Y - FLOOR_Y > crate::collide::CAPSULE_HEIGHT_M + 1.0);
    // The wire's door mask fits its byte (`protocol::encode_event_card_doors`).
    assert!(DOOR_BITS <= 8 && LAB_DOOR as usize == DOOR_BITS);
    // Everything inside the host's disc, so the scatter veto round it
    // (`landmark::at`) keeps trees out of it and nothing is built over it
    // (`terrain::build_reserved`).
    let r = landmark::LANDMARK_R_M;
    let mut i = 0;
    while i < PARTS.len() {
        let p = PARTS[i].b;
        assert!(p[0] * p[0] + p[2] * p[2] <= r * r && p[3] * p[3] + p[5] * p[5] <= r * r);
        assert!(p[0] * p[0] + p[5] * p[5] <= r * r && p[3] * p[3] + p[2] * p[2] <= r * r);
        i += 1;
    }
    // The apron's band inside the reach the carve is looked up by, and that
    // inside the landmark's own cell (`landmark::at` asks only that one).
    let reach = r + REACH_M;
    assert!(reach < landmark::LANDMARK_R_M + 40.0);
    let (ax, az) = (APRON[2] + APRON_BAND_M, APRON[1] - APRON_BAND_M);
    assert!(ax * ax + az * az <= reach * reach);
    let mut k = 0;
    while k < PITS.len() {
        let p = PITS[k];
        assert!(p[0] * p[0] + p[1] * p[1] <= r * r && p[2] * p[2] + p[3] * p[3] <= r * r);
        k += 1;
    }
    // The crates: on the floor, clear of the walls, and each in its own
    // scatter cell at any bearing — more than a cell's diagonal from each
    // other and from every host anchor that stays (the ones over a pit went
    // down with the court, `terrain::scatter`). The vault's proof
    // (`landmark.rs`), for these.
    let cell2 = 2.0 * terrain::CELL_SIZE * terrain::CELL_SIZE + 1.0;
    let host = landmark::anchors(HOST_KIND);
    let mut c = 0;
    while c < CRATES.len() {
        let (x, z, _) = CRATES[c];
        assert!(x > -6.5 && x < 6.5 && z > -6.5 && z < 6.5);
        let mut d = 0;
        while d < CRATES.len() {
            let (dx, dz) = (x - CRATES[d].0, z - CRATES[d].1);
            assert!(d == c || dx * dx + dz * dz > cell2);
            d += 1;
        }
        let mut a = 0;
        while a < host.len() {
            let (hx, hz, _) = host[a];
            let (dx, dz) = (x - hx, z - hz);
            assert!(over_pit(hx, hz) || dx * dx + dz * dz > cell2);
            a += 1;
        }
        c += 1;
    }
    // The host keeps its vault: it does not stand over a pit.
    let mut a = 0;
    let mut vaults = 0;
    while a < host.len() {
        if matches!(host[a].2, Occupant::Vault) {
            assert!(!over_pit(host[a].0, host[a].1));
            vaults += 1;
        }
        a += 1;
    }
    assert!(vaults == 1);
    // The stair's headroom (`step`'s note): every tread a head reaches a
    // ceiling from stands low enough under it. The ceilings over the stair
    // are the stair's own (`PARTS[3]`), the blockhouse's north wall
    // (`PARTS[21]`) and the keep's south wall's footing (`-FOOTING_M` from
    // local z −10).
    let ceilings = [
        (PARTS[3].b[2], PARTS[3].b[1]),
        (PARTS[21].b[2], PARTS[21].b[1]),
        (-10.0, -landmark::FOOTING_M),
        (PARTS[2].b[2], PARTS[2].b[1]),
    ];
    let mut t = 0;
    while t < STEPS {
        let s = step(t as u32).b;
        let mut c = 0;
        while c < ceilings.len() {
            let (z0, under) = ceilings[c];
            assert!(
                s[5] + STAIR_REACH_M < z0 || s[4] + crate::collide::CAPSULE_HEIGHT_M + 0.05 < under
            );
            c += 1;
        }
        t += 1;
    }
    assert!(step(STEPS as u32 - 1).b[5] == PARTS[1].b[2]);
    assert!(step(0).b[2] == PARTS[22].b[5]);
    // The dwellers' homes on the floor, a body clear of every wall.
    let mut h = 0;
    while h < DWELL.len() {
        let (x, z) = DWELL[h];
        assert!(x > -6.4 && x < 6.4 && z > -6.4 && z < 6.4);
        h += 1;
    }
};

/// Mark the lab's host among `marks`: the live keep nearest the island's
/// middle that stands high enough ([`MIN_BASE_Y`]). An island without one
/// has no lab. Called once, from `terrain::haven`, after the landmarks.
pub fn choose(marks: &mut [Landmark]) {
    let mid = terrain::ISLAND_SIZE * 0.5;
    let mut best: Option<(usize, f32)> = None;
    for (i, m) in marks.iter().enumerate() {
        if !m.live || m.kind != HOST_KIND || m.y < MIN_BASE_Y {
            continue;
        }
        let (dx, dz) = (m.x - mid, m.z - mid);
        let d2 = dx * dx + dz * dz;
        if best.is_none_or(|(_, b)| d2 < b) {
            best = Some((i, d2));
        }
    }
    if let Some((i, _)) = best {
        marks[i].lab = true;
    }
}

/// The lab's host, if this island has one.
pub fn host(marks: &[Landmark]) -> Option<&Landmark> {
    marks.iter().find(|m| m.live && m.lab)
}

/// The host whose disc, padded by `pad`, holds (`x`, `z`).
#[inline]
fn host_at(marks: &[Landmark], x: f32, z: f32, pad: f32) -> Option<&Landmark> {
    landmark::at(marks, x, z, pad).filter(|m| m.lab)
}

/// The lab's carve as a height delta over `g` (the ground the roads, the
/// sites and the host's own stamps already shaped): its pits, hard-edged to
/// [`PIT_Y`], and the blockhouse's level apron. `0.0` off them, so
/// `terrain::ground` keeps its bits everywhere else.
pub fn carve(marks: &[Landmark], g: f32, x: f32, z: f32) -> f32 {
    let Some(m) = host_at(marks, x, z, REACH_M) else {
        return 0.0;
    };
    let (lx, lz) = landmark::to_local(m, x, z);
    if over_pit(lx, lz) {
        return (m.y + PIT_Y) - g;
    }
    let a = APRON;
    let tx = if lx < a[0] {
        (a[0] - lx) / APRON_BAND_M
    } else if lx > a[2] {
        (lx - a[2]) / APRON_BAND_M
    } else {
        0.0
    };
    let tz = if lz < a[1] {
        (a[1] - lz) / APRON_BAND_M
    } else if lz > a[3] {
        (lz - a[3]) / APRON_BAND_M
    } else {
        0.0
    };
    let t2 = tx * tx + tz * tz;
    if t2 >= 1.0 {
        return 0.0;
    }
    let t = 1.0 - t2.sqrt();
    let w = t * t * (3.0 - 2.0 * t);
    (m.y - g) * w
}

/// 1.0 over the pits and a pad round them, else 0.0: nothing grows down
/// there, and no rock face is drawn on a pit's wall (`face::cliff_relief`,
/// `cliff::open_ground`).
pub fn sweep(marks: &[Landmark], x: f32, z: f32) -> f32 {
    let Some(m) = host_at(marks, x, z, REACH_M) else {
        return 0.0;
    };
    let (lx, lz) = landmark::to_local(m, x, z);
    if in_rect(PITS[0], lx, lz, SWEEP_PAD_M) || in_rect(PITS[1], lx, lz, SWEEP_PAD_M) {
        1.0
    } else {
        0.0
    }
}

/// Whether the lab's boxes stop a volume (`landmark::blocks`'s shape), its
/// door as `doors` has it.
#[allow(clippy::too_many_arguments)]
pub fn blocks(marks: &[Landmark], doors: u32, x: f32, z: f32, feet: f32, r: f32, h: f32) -> bool {
    let Some(m) = host_at(marks, x, z, r) else {
        return false;
    };
    let (lx, lz) = landmark::to_local(m, x, z);
    kit::blocks_local(&PARTS, m.y, doors, lx, lz, feet, r, h)
}

/// How far apart [`walled`] samples its line, metres: under half the
/// thinnest wall ([`PARTS`]' 0.4 m partitions), so no sample steps over one.
const WALL_SAMPLE_M: f32 = 0.15;
/// The most samples [`walled`] takes: a bite's reach and some.
const WALL_SAMPLES: usize = 32;

/// Whether a lab wall (or its shut door) stands on the planar line from
/// (`x0`, `z0`) to (`x1`, `z1`) for a body with its feet at `feet`: a point
/// walk at [`WALL_SAMPLE_M`], so a bite never crosses a partition. A tread
/// within a step of `feet` is floor, not wall (`kit::blocks_local`).
#[allow(clippy::too_many_arguments)]
pub fn walled(
    marks: &[Landmark],
    doors: u32,
    x0: f32,
    z0: f32,
    x1: f32,
    z1: f32,
    feet: f32,
) -> bool {
    let (dx, dz) = (x1 - x0, z1 - z0);
    let len = (dx * dx + dz * dz).sqrt();
    if host_at(marks, x0, z0, len + 1.0).is_none() {
        return false;
    }
    let n = ((len / WALL_SAMPLE_M) as usize + 1).min(WALL_SAMPLES);
    let h = crate::collide::CAPSULE_HEIGHT_M;
    (1..n).any(|k| {
        let t = k as f32 / n as f32;
        blocks(
            marks,
            doors,
            x0 + dx * t,
            z0 + dz * t,
            feet,
            crate::ranged::ARROW_R_M,
            h,
        )
    })
}

/// The highest lab surface within a step of `feet` under a disc of radius
/// `r` at (`x`, `z`) — `kit::ground_local`'s footprint.
pub fn ground(marks: &[Landmark], doors: u32, x: f32, z: f32, feet: f32, r: f32) -> f32 {
    let Some(m) = host_at(marks, x, z, r) else {
        return crate::collide::NO_SURFACE;
    };
    let (lx, lz) = landmark::to_local(m, x, z);
    kit::ground_local(&PARTS, m.y, doors, lx, lz, feet, r)
}

/// Whether the lab's ceiling or the blockhouse's roof keeps the rain off.
pub fn roofed(marks: &[Landmark], x: f32, z: f32, feet: f32) -> bool {
    let Some(m) = host_at(marks, x, z, 0.0) else {
        return false;
    };
    let (lx, lz) = landmark::to_local(m, x, z);
    kit::roofed_local(&PARTS, m.y, lx, lz, feet)
}

/// Crate `k` of the lab in world space: `(x, y, z, yaw, occupant)`, seated
/// on the floor.
pub fn crate_world(m: &Landmark, k: usize) -> Option<(f32, f32, f32, u8, Occupant)> {
    let &(x, z, occ) = CRATES.get(k)?;
    let (wx, wz) = landmark::to_world(m, x, z);
    let yaw = m
        .yaw
        .wrapping_add((k as u8).wrapping_mul(64).wrapping_add(32));
    Some((wx, m.y + FLOOR_Y, wz, yaw, occ))
}

/// Dweller `k`'s home in world space, on this island's lab.
pub fn dweller_home(marks: &[Landmark], k: usize) -> Option<(f32, f32)> {
    let m = host(marks)?;
    let &(x, z) = DWELL.get(k)?;
    Some(landmark::to_world(m, x, z))
}

/// The lab's bit of the open mask: its door stands open while THE SEVERED
/// GATE does (`works::gate_open_of`), the rule the vaults open by.
pub const fn door_bits(gate_open: bool) -> u32 {
    if gate_open {
        LAB_DOOR_BIT
    } else {
        0
    }
}
