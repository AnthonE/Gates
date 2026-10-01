//! The Black Ziggurat — the island's one monument: five black-stone
//! terraces, a broken gilt crown, glowing lapis seams, and three sealed rooms
//! opened by keycards (green → blue → red, Rust's card loop; research
//! 2026-09-30).
//!
//! The route: the rear portal into the entry hall; the **green door** to the
//! green room under the first terrace; up the front stairs and round the
//! first ledge to the **blue door** in the second terrace's east face; up to
//! the third ledge and the **red door** in the fourth terrace's rear face,
//! the sanctum. Each room's crate guarantees the next card
//! (`content/loot.toml`), and green cards turn up out in the world.
//!
//! The kit is `kit.rs` boxes: the same parts collide and draw. Placement is
//! `terrain::solve_ziggurat`: after the town, on the town's 24 m lattice with
//! an exact quarter turn, so every crate anchor (local ≡ 4 mod 8) is a
//! scatter-cell centre in the world whatever the rotation.

use crate::kit::{self, part, part_f, KitMat as M, KitPart, Placed, DECOR, NO_DOOR, ROOF};
use crate::terrain::{Occupant, SiteFootprint};

/// Its lattice, metres (the town's: lcm of the build and scatter cells).
pub const ZIG_SNAP_M: f32 = 24.0;
/// The nearest it may stand to the town, metres.
pub const ZIG_TOWN_CLEAR_M: f32 = 700.0;
/// The card doors: green, blue, red. Door part `door` is 1 + this index.
pub const CARD_DOORS: usize = 3;
/// How long a swiped door stays open, ticks (60 s).
pub const DOOR_OPEN_TICKS: u64 = 60 * crate::limits::TICK_HZ as u64;
/// How close to a door's reader a swipe must be, metres.
pub const SWIPE_REACH_M: f32 = 3.0;

/// The masks. The kit's corners reach 32·√2 ≈ 45.3 m.
pub const ZIG_FOOTPRINT: SiteFootprint = SiteFootprint {
    swept_m: 52.0,
    stamp_m: 54.0,
    scatter_m: 62.0,
    blend_m: 100.0,
};

/// One placed ziggurat; `live` false on a seed with nowhere to put it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ziggurat {
    pub x: f32,
    pub z: f32,
    pub y: f32,
    /// The carved floor, a multiple of 0.5 m.
    pub floor_y: f32,
    /// Quarter turns: local +Z (the rear portal) faces yaw index `rot * 64`.
    pub rot: u8,
    pub live: bool,
}

impl Ziggurat {
    pub const NONE: Ziggurat = Ziggurat {
        x: 0.0,
        z: 0.0,
        y: 0.0,
        floor_y: 0.0,
        rot: 0,
        live: false,
    };

    pub fn placed(&self) -> Placed {
        Placed {
            x: self.x,
            z: self.z,
            floor_y: self.floor_y,
            rot: self.rot,
        }
    }
}

const F: f32 = -2.0; // footing below the floor

const fn door(b: [f32; 6], which: u8) -> KitPart {
    KitPart {
        b,
        mat: M::Steel,
        flags: 0,
        door: which,
    }
}

/// One step of a front flight: terrace `k`'s face at `z = -face`, rising
/// from `5k` in ten 0.5 m risers on 0.5 m treads. The flight runs 5 m of
/// the 6 m ledge, leaving a 1 m landing at its foot to step off onto the
/// ledge from.
const fn step(k: u32, i: u32, face: f32) -> KitPart {
    let y0 = 5.0 * k as f32;
    let z0 = -face - 5.0 + 0.5 * i as f32;
    let base = if k == 0 { F } else { y0 - 0.5 };
    part(
        [-3.0, base, z0, 3.0, y0 + 0.5 * (i + 1) as f32, -face],
        M::Obsidian,
    )
}

macro_rules! flight {
    ($k:expr, $face:expr) => {
        [
            step($k, 0, $face),
            step($k, 1, $face),
            step($k, 2, $face),
            step($k, 3, $face),
            step($k, 4, $face),
            step($k, 5, $face),
            step($k, 6, $face),
            step($k, 7, $face),
            step($k, 8, $face),
            step($k, 9, $face),
        ]
    };
}

const STAIRS: [[KitPart; 10]; 5] = [
    flight!(0, 32.0),
    flight!(1, 26.0),
    flight!(2, 20.0),
    flight!(3, 14.0),
    flight!(4, 8.0),
];

/// The massing and the rooms. Local metres; +Z the rear portal, −Z the
/// front stairs, +X the blue door's face.
const BODY: &[KitPart] = &[
    // ── Terrace 0 (y 0–5, half 32): the entry hall and the green room ──
    part([-32.0, F, -32.0, -8.0, 5.0, 32.0], M::Obsidian),
    part([8.0, F, -32.0, 32.0, 5.0, 32.0], M::Obsidian),
    part([-8.0, F, -32.0, 8.0, 5.0, -8.0], M::Obsidian),
    part([-8.0, F, 12.0, -6.0, 5.0, 32.0], M::Obsidian),
    part([6.0, F, 12.0, 8.0, 5.0, 32.0], M::Obsidian),
    // The rear wall and the portal's lintel.
    part([-6.0, F, 30.0, -2.0, 5.0, 32.0], M::Obsidian),
    part([2.0, F, 30.0, 6.0, 5.0, 32.0], M::Obsidian),
    part([-2.0, 3.5, 30.0, 2.0, 5.0, 32.0], M::Gilt),
    // The partition, its lintel, and the green door.
    part([-8.0, F, 10.0, -1.5, 5.0, 12.0], M::Obsidian),
    part([1.5, F, 10.0, 8.0, 5.0, 12.0], M::Obsidian),
    part([-1.5, 3.0, 10.0, 1.5, 5.0, 12.0], M::Obsidian),
    door([-1.5, 0.0, 10.6, 1.5, 3.0, 11.4], 1),
    // The hollow's floor and roof.
    part([-8.0, -0.3, -8.0, 8.0, 0.0, 30.0], M::Concrete),
    part_f([-8.0, 4.0, -8.0, 8.0, 5.0, 10.0], M::Obsidian, ROOF),
    part_f([-6.0, 4.0, 12.0, 6.0, 5.0, 30.0], M::Obsidian, ROOF),
    // ── Terrace 1 (y 5–10, half 26): the blue room, east door ──
    part([-26.0, 4.5, -26.0, -6.0, 10.0, 26.0], M::Obsidian),
    part([-6.0, 4.5, 6.0, 26.0, 10.0, 26.0], M::Obsidian),
    part([-6.0, 4.5, -26.0, 26.0, 10.0, -6.0], M::Obsidian),
    part([6.0, 4.5, 1.5, 26.0, 10.0, 6.0], M::Obsidian),
    part([6.0, 4.5, -6.0, 26.0, 10.0, -1.5], M::Obsidian),
    part_f([6.0, 8.0, -1.5, 26.0, 10.0, 1.5], M::Obsidian, ROOF),
    part_f([-6.0, 9.0, -6.0, 6.0, 10.0, 6.0], M::Obsidian, ROOF),
    door([22.6, 5.0, -1.5, 23.4, 8.0, 1.5], 2),
    // ── Terrace 2 (y 10–15, half 20): solid ──
    part([-20.0, 9.5, -20.0, 20.0, 15.0, 20.0], M::Obsidian),
    // ── Terrace 3 (y 15–20, half 14): the sanctum, rear door ──
    part([-14.0, 14.5, -14.0, -13.0, 20.0, 14.0], M::Obsidian),
    part([-3.0, 14.5, -14.0, 14.0, 20.0, 14.0], M::Obsidian),
    part([-13.0, 14.5, -14.0, -3.0, 20.0, -7.0], M::Obsidian),
    part([-13.0, 14.5, 7.0, -9.5, 20.0, 14.0], M::Obsidian),
    part([-6.5, 14.5, 7.0, -3.0, 20.0, 14.0], M::Obsidian),
    part_f([-9.5, 18.0, 7.0, -6.5, 20.0, 14.0], M::Obsidian, ROOF),
    part_f([-13.0, 19.0, -7.0, -3.0, 20.0, 7.0], M::Obsidian, ROOF),
    door([-9.5, 15.0, 12.6, -6.5, 18.0, 13.4], 3),
    // ── Terrace 4 (y 20–25, half 8): solid, the crown broken ──
    part([-8.0, 19.5, -8.0, 8.0, 25.0, 8.0], M::Obsidian),
    part([-8.0, 25.0, 4.0, -4.0, 28.0, 8.0], M::Obsidian),
    part([4.0, 25.0, 5.0, 8.0, 26.5, 8.0], M::Obsidian),
    part([-8.0, 25.0, -8.0, -5.0, 27.0, -5.0], M::Obsidian),
    // The crown: gilt fragments, drawn only.
    part_f([-6.0, 28.0, 4.5, -4.5, 33.0, 7.5], M::Gilt, DECOR),
    part_f([5.0, 26.5, 5.5, 7.5, 29.0, 7.5], M::Gilt, DECOR),
    part_f([-7.5, 27.0, -7.5, -5.5, 30.5, -5.5], M::Gilt, DECOR),
    // Lapis seams along each terrace's front lip, drawn only.
    part_f([-32.0, 4.6, -32.2, -3.2, 4.9, -31.9], M::Lapis, DECOR),
    part_f([3.2, 4.6, -32.2, 32.0, 4.9, -31.9], M::Lapis, DECOR),
    part_f([-26.0, 9.6, -26.2, -3.2, 9.9, -25.9], M::Lapis, DECOR),
    part_f([3.2, 9.6, -26.2, 26.0, 9.9, -25.9], M::Lapis, DECOR),
    part_f([-20.0, 14.6, -20.2, -3.2, 14.9, -19.9], M::Lapis, DECOR),
    part_f([3.2, 14.6, -20.2, 20.0, 14.9, -19.9], M::Lapis, DECOR),
    part_f([-14.0, 19.6, -14.2, -3.2, 19.9, -13.9], M::Lapis, DECOR),
    part_f([3.2, 19.6, -14.2, 14.0, 19.9, -13.9], M::Lapis, DECOR),
    // The card readers: a lapis plate beside each door, drawn only.
    part_f([1.8, 1.2, 12.0, 2.6, 2.0, 12.15], M::Lapis, DECOR),
    part_f([26.0, 6.2, 1.8, 26.15, 7.0, 2.6], M::Lapis, DECOR),
    part_f([-6.2, 16.2, 14.0, -5.4, 17.0, 14.15], M::Lapis, DECOR),
];

/// Every part: the body then the five flights.
pub const PARTS: [KitPart; BODY.len() + 50] = {
    let mut out = [part([0.0, 0.0, 0.0, 1.0, 1.0, 1.0], M::Obsidian); BODY.len() + 50];
    let mut i = 0;
    while i < BODY.len() {
        out[i] = BODY[i];
        i += 1;
    }
    let mut k = 0;
    while k < 5 {
        let mut s = 0;
        while s < 10 {
            out[BODY.len() + k * 10 + s] = STAIRS[k][s];
            s += 1;
        }
        k += 1;
    }
    out
};

/// A card door: which card opens it, where its reader is outside, and where
/// the exit lever is inside (local x, feet y, z — where a body stands). The
/// lever opens the door with no card, so a room is never a cell.
pub struct CardDoor {
    pub card: &'static str,
    pub reader: (f32, f32, f32),
    pub lever: (f32, f32, f32),
}

pub const DOORS: [CardDoor; CARD_DOORS] = [
    CardDoor {
        card: "item.keycard_green",
        reader: (2.2, 0.0, 13.0),
        lever: (2.2, 0.0, 9.0),
    },
    CardDoor {
        card: "item.keycard_blue",
        reader: (27.0, 5.0, 2.2),
        lever: (21.0, 5.0, 0.0),
    },
    CardDoor {
        card: "item.keycard_red",
        reader: (-5.8, 15.0, 15.0),
        lever: (-8.0, 15.0, 11.0),
    },
];

/// The crates: local (x, y, z) and occupant. Each x and z ≡ 4 (mod 8), so
/// each is a scatter-cell centre and no two share a cell.
pub const CRATES: [(f32, f32, f32, Occupant); 8] = [
    // Entry hall.
    (-4.0, 0.0, 20.0, Occupant::CacheSlot),
    (4.0, 0.0, 28.0, Occupant::CrateSlot),
    // Green room.
    (-4.0, 0.0, -4.0, Occupant::GreenCrate),
    (4.0, 0.0, -4.0, Occupant::CrateSlot),
    // Blue room.
    (-4.0, 5.0, 4.0, Occupant::BlueCrate),
    (4.0, 5.0, 4.0, Occupant::CrateSlot),
    // Sanctum.
    (-12.0, 15.0, -4.0, Occupant::EliteCrate),
    (-12.0, 15.0, 4.0, Occupant::EliteCrate),
];

const _: () = {
    assert!(kit::well_formed(&PARTS));
    let e = kit::envelope(&PARTS);
    assert!(e * e * 2.0 <= ZIG_FOOTPRINT.stamp_m * ZIG_FOOTPRINT.stamp_m);
    assert!(ZIG_FOOTPRINT.swept_m <= ZIG_FOOTPRINT.stamp_m);
    assert!(ZIG_FOOTPRINT.stamp_m < ZIG_FOOTPRINT.scatter_m);
    assert!(ZIG_FOOTPRINT.scatter_m < ZIG_FOOTPRINT.blend_m);
    // Crate anchors on scatter-cell centres, pairwise in distinct cells.
    let mut i = 0;
    while i < CRATES.len() {
        let (x, _, z, _) = CRATES[i];
        let (cx, cz) = ((x - 4.0) / 8.0, (z - 4.0) / 8.0);
        assert!(cx == (cx as i32) as f32 && cz == (cz as i32) as f32);
        let mut j = 0;
        while j < i {
            assert!(!(CRATES[j].0 == x && CRATES[j].2 == z));
            j += 1;
        }
        i += 1;
    }
    // Every door part names a door that exists.
    let mut i = 0;
    while i < PARTS.len() {
        assert!(PARTS[i].door == NO_DOOR || PARTS[i].door as usize <= CARD_DOORS);
        i += 1;
    }
    assert!(ZIG_SNAP_M == 24.0);
};

fn near(z: &Ziggurat, x: f32, wz: f32, half: f32) -> bool {
    if !z.live {
        return false;
    }
    let (dx, dz) = (x - z.x, wz - z.z);
    dx >= -half && dx <= half && dz >= -half && dz <= half
}

/// The doors open now, as `kit::blocks_local`'s bit mask: bit `d` for card
/// door `d` (`World::card_doors` holds the open-until ticks).
pub fn open_bits(until: &[u64; CARD_DOORS], tick: u64) -> u32 {
    let mut bits = 0;
    for (d, &u) in until.iter().enumerate() {
        if tick < u {
            bits |= 1 << d;
        }
    }
    bits
}

/// Whether the ziggurat's boxes stop a volume.
#[allow(clippy::too_many_arguments)]
pub fn blocks(z: &Ziggurat, doors: u32, x: f32, wz: f32, feet: f32, r: f32, h: f32) -> bool {
    if !near(z, x, wz, 39.0 + r) {
        return false;
    }
    let (lx, lz) = kit::to_local(&z.placed(), x, wz);
    kit::blocks_local(&PARTS, z.floor_y, doors, lx, lz, feet, r, h)
}

/// The highest ziggurat surface under (`x`, `wz`) within a step of `feet`.
pub fn ground(z: &Ziggurat, doors: u32, x: f32, wz: f32, feet: f32) -> f32 {
    if !near(z, x, wz, 39.0) {
        return crate::collide::NO_SURFACE;
    }
    let (lx, lz) = kit::to_local(&z.placed(), x, wz);
    kit::ground_local(&PARTS, z.floor_y, doors, lx, lz, feet)
}

/// Whether stone overhead keeps the rain off a body here.
pub fn roofed(z: &Ziggurat, x: f32, wz: f32, feet: f32) -> bool {
    if !near(z, x, wz, 39.0) {
        return false;
    }
    let (lx, lz) = kit::to_local(&z.placed(), x, wz);
    kit::roofed_local(&PARTS, z.floor_y, lx, lz, feet)
}

/// Whether building at (`x`, `wz`) with `margin` would touch it.
pub fn reserves(z: &Ziggurat, x: f32, wz: f32, margin: f32) -> bool {
    near(z, x, wz, 46.0 + margin)
}

/// Whether (`x`, `wz`) is inside its scatter mask (nothing grows).
pub fn covers(z: &Ziggurat, x: f32, wz: f32, pad: f32) -> bool {
    if !z.live {
        return false;
    }
    let (dx, dz) = (x - z.x, wz - z.z);
    let r = ZIG_FOOTPRINT.scatter_m + pad;
    dx * dx + dz * dz <= r * r
}

/// Crate `k` in world space: (x, y, z, yaw byte, occupant).
pub fn crate_world(z: &Ziggurat, k: usize) -> Option<(f32, f32, f32, u8, Occupant)> {
    let &(x, y, lz, occ) = CRATES.get(k)?;
    let (wx, wz) = kit::to_world(&z.placed(), x, lz);
    let yaw = kit::yaw_byte(z.rot).wrapping_add((k as u8).wrapping_mul(64));
    Some((wx, z.floor_y + y, wz, yaw, occ))
}

/// Door `d`'s reader (`inside` false) or lever in world space: (x, feet
/// y, z).
pub fn reader_world(z: &Ziggurat, d: usize, inside: bool) -> Option<(f32, f32, f32)> {
    let door = DOORS.get(d)?;
    let (x, y, lz) = if inside { door.lever } else { door.reader };
    let (wx, wz) = kit::to_world(&z.placed(), x, lz);
    Some((wx, z.floor_y + y, wz))
}

/// The local box of card door `d`'s leaf.
pub fn door_box(d: usize) -> Option<[f32; 6]> {
    PARTS.iter().find(|p| p.door as usize == d + 1).map(|p| p.b)
}

/// Whether a body (feet at `feet`) stands in door `d`'s doorway — a door
/// does not close on somebody.
pub fn in_doorway(z: &Ziggurat, d: usize, x: f32, wz: f32, feet: f32) -> bool {
    let Some(b) = door_box(d) else { return false };
    if !z.live {
        return false;
    }
    let (lx, lz) = kit::to_local(&z.placed(), x, wz);
    let pad = 0.6;
    lx >= b[0] - pad
        && lx <= b[3] + pad
        && lz >= b[2] - pad
        && lz <= b[5] + pad
        && feet < z.floor_y + b[4]
        && feet + 1.8 > z.floor_y + b[1]
}

/// The kit as JSON for `ci/site_kit.py` (`examples/kit_dump.rs`).
pub fn dump(w: &mut impl core::fmt::Write) -> core::fmt::Result {
    let mut anchors = [("", 0.0, 0.0, 0.0); CRATES.len() + 2 * CARD_DOORS];
    let mut n = 0;
    for (x, y, z, occ) in CRATES {
        let kind = match occ {
            Occupant::GreenCrate => "crate_green",
            Occupant::BlueCrate => "crate_blue",
            Occupant::EliteCrate => "crate_elite",
            _ => "crate",
        };
        anchors[n] = (kind, x, y, z);
        n += 1;
    }
    for d in DOORS {
        let (x, y, z) = d.reader;
        anchors[n] = ("reader", x, y, z);
        let (x, y, z) = d.lever;
        anchors[n + 1] = ("lever", x, y, z);
        n += 2;
    }
    kit::dump("ziggurat", &PARTS, &anchors, w)
}

/// A swipe refused: no such door, or this body cannot act.
pub const REFUSE_S_KIND: u32 = 1;
/// Not at the door's reader or its lever.
pub const REFUSE_S_REACH: u32 = 2;
/// At the reader without the door's card.
pub const REFUSE_S_CARD: u32 = 3;
pub const REFUSE_S_MAX: u32 = REFUSE_S_CARD;

/// What one swipe wears off a card, in condition points: a 400 card opens
/// four doors, a 200 card two (Rust's green/blue 4, red 2).
pub const SWIPE_WEAR: u16 = 100;

/// Swipe at door `d`'s reader with its card, or pull its lever from inside.
/// Opens the door for `DOOR_OPEN_TICKS`; refusals are events.
#[allow(clippy::too_many_arguments)]
pub fn swipe(
    z: &Ziggurat,
    cards: &[u16; CARD_DOORS],
    doors: &mut [u64; CARD_DOORS],
    tick: u64,
    p: &mut crate::world::Player,
    d: usize,
    events: &mut crate::world::EventQueue,
) {
    use crate::world::{EV_SWIPE, EV_SWIPE_REFUSED};
    let refuse = |events: &mut crate::world::EventQueue, code: u32| {
        events.push(EV_SWIPE_REFUSED, p.id, code, d as u32);
    };
    if !z.live || d >= CARD_DOORS || p.dead || p.sleeping || p.wounded {
        refuse(events, REFUSE_S_KIND);
        return;
    }
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    let feet = p.body.qy as f32 * crate::movement::POS_Y_Q;
    let at = |inside: bool| {
        reader_world(z, d, inside).is_some_and(|(x, y, wz)| {
            let (dx, dz) = (px - x, pz - wz);
            dx * dx + dz * dz <= SWIPE_REACH_M * SWIPE_REACH_M
                && crate::fmath::fabs(feet - y) <= 2.0
        })
    };
    let lever = at(true);
    if !lever {
        if !at(false) {
            refuse(events, REFUSE_S_REACH);
            return;
        }
        let card = cards[d];
        let Some(s) = p
            .inv
            .iter_mut()
            .find(|s| card != crate::gather::NO_ITEM && s.count > 0 && s.item == card)
        else {
            refuse(events, REFUSE_S_CARD);
            return;
        };
        if s.cond <= SWIPE_WEAR {
            *s = crate::gather::ItemStack::default();
        } else {
            s.cond -= SWIPE_WEAR;
        }
    }
    doors[d] = tick + DOOR_OPEN_TICKS;
    events.push(EV_SWIPE, p.id, d as u32, lever as u32);
}
