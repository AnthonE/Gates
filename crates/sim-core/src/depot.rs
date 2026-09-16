//! The inland depot's authored kit. Placement varies with the seed; every
//! rendered mass and every collision query reads this same bounded table.
use crate::terrain::{Haven, SiteFootprint, SiteKind, Waystation};

/// Semantic materials, not independent collision shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PartKind {
    Concrete,
    Wall,
    Roof,
    Steel,
    Cargo,
    Fence,
}

#[derive(Clone, Copy, Debug)]
pub struct Part {
    /// Local min x/y/z, max x/y/z, metres above the site's floor datum.
    pub bounds: [f32; 6],
    pub kind: PartKind,
}

const fn part(kind: PartKind, bounds: [f32; 6]) -> Part {
    Part { bounds, kind }
}

/// The gate centres and approach ports share the unobstructed local Z axis.
pub const PORT_Z: f32 = 24.0;
pub const YARD_HALF_X: f32 = 20.0;
pub const YARD_HALF_Z: f32 = 22.0;
pub const GATE_HALF_W: f32 = 4.0;
/// Best raw-terrain candidates tried as complete depot/road layouts.
pub const CANDIDATE_TRIES: usize = 16;
/// Conservative circle enclosing the rectangular yard and both approaches.
pub const FOOTPRINT: SiteFootprint = SiteFootprint {
    swept_m: 30.0,
    stamp_m: 32.0,
    scatter_m: 36.0,
    blend_m: 52.0,
};

/// Intact flat roofs; the warehouse's east doorway is 6 m wide, 4 m high.
/// Concrete slabs finish at the datum and require no invisible step.
pub const DEPOT_PARTS: [Part; 29] = [
    part(PartKind::Concrete, [-20.0, -0.3, -22.0, 20.0, 0.0, 22.0]),
    part(PartKind::Wall, [-18.0, 0.0, -12.0, -17.6, 6.0, 12.0]),
    part(PartKind::Wall, [-18.0, 0.0, -12.0, -4.0, 6.0, -11.6]),
    part(PartKind::Wall, [-18.0, 0.0, 11.6, -4.0, 6.0, 12.0]),
    part(PartKind::Wall, [-4.4, 0.0, -12.0, -4.0, 6.0, -3.0]),
    part(PartKind::Wall, [-4.4, 0.0, 3.0, -4.0, 6.0, 12.0]),
    part(PartKind::Wall, [-4.4, 4.0, -3.0, -4.0, 6.0, 3.0]),
    part(PartKind::Roof, [-18.0, 6.0, -12.0, -4.0, 6.35, 12.0]),
    part(PartKind::Steel, [-13.0, 6.35, -6.0, -9.0, 8.0, 6.0]),
    part(PartKind::Roof, [-13.3, 8.0, -6.3, -8.7, 8.3, 6.3]),
    part(PartKind::Roof, [6.0, 3.8, -10.0, 17.0, 4.1, 4.0]),
    part(PartKind::Steel, [6.0, 0.0, -10.0, 6.4, 3.8, -9.6]),
    part(PartKind::Steel, [16.6, 0.0, -10.0, 17.0, 3.8, -9.6]),
    part(PartKind::Steel, [6.0, 0.0, 3.6, 6.4, 3.8, 4.0]),
    part(PartKind::Steel, [16.6, 0.0, 3.6, 17.0, 3.8, 4.0]),
    part(PartKind::Cargo, [11.0, 0.0, -8.0, 16.0, 2.5, -2.0]),
    part(PartKind::Cargo, [7.0, 0.0, 8.0, 13.0, 2.5, 10.5]),
    part(PartKind::Cargo, [7.0, 0.0, 11.0, 13.0, 5.0, 13.5]),
    part(PartKind::Cargo, [-16.0, 0.0, 16.0, -10.0, 2.5, 18.5]),
    part(PartKind::Fence, [-20.0, 0.0, -22.0, -19.6, 2.4, 22.0]),
    part(PartKind::Fence, [19.6, 0.0, -22.0, 20.0, 2.4, 22.0]),
    part(PartKind::Fence, [-20.0, 0.0, -22.0, -4.0, 2.4, -21.6]),
    part(PartKind::Fence, [4.0, 0.0, -22.0, 20.0, 2.4, -21.6]),
    part(PartKind::Fence, [-20.0, 0.0, 21.6, -4.0, 2.4, 22.0]),
    part(PartKind::Fence, [4.0, 0.0, 21.6, 20.0, 2.4, 22.0]),
    part(PartKind::Steel, [-4.5, 0.0, -22.3, -4.0, 3.2, -21.3]),
    part(PartKind::Steel, [4.0, 0.0, -22.3, 4.5, 3.2, -21.3]),
    part(PartKind::Steel, [-4.5, 0.0, 21.3, -4.0, 3.2, 22.3]),
    part(PartKind::Steel, [4.0, 0.0, 21.3, 4.5, 3.2, 22.3]),
];

const _: () = {
    let slab = DEPOT_PARTS[0].bounds;
    assert!(slab[0] == -YARD_HALF_X && slab[3] == YARD_HALF_X);
    assert!(slab[2] == -YARD_HALF_Z && slab[5] == YARD_HALF_Z);
    assert!(slab[4] == 0.0);
    assert!(DEPOT_PARTS[21].bounds[3] == -GATE_HALF_W);
    assert!(DEPOT_PARTS[22].bounds[0] == GATE_HALF_W);
    assert!(DEPOT_PARTS[23].bounds[3] == -GATE_HALF_W);
    assert!(DEPOT_PARTS[24].bounds[0] == GATE_HALF_W);
    assert!(DEPOT_PARTS[25].bounds[3] == -GATE_HALF_W);
    assert!(DEPOT_PARTS[26].bounds[0] == GATE_HALF_W);
    assert!(DEPOT_PARTS[27].bounds[3] == -GATE_HALF_W);
    assert!(DEPOT_PARTS[28].bounds[0] == GATE_HALF_W);
    assert!(FOOTPRINT.swept_m < FOOTPRINT.stamp_m);
    assert!(FOOTPRINT.stamp_m < FOOTPRINT.scatter_m);
    assert!(FOOTPRINT.scatter_m < FOOTPRINT.blend_m);
    assert!(
        CANDIDATE_TRIES
            <= (crate::terrain::INLAND_CANDIDATES * crate::terrain::INLAND_RADII) as usize
    );
};

pub fn is_depot(site: &Waystation) -> bool {
    site.live && site.kind == SiteKind::Inland
}

/// Local +Z is yaw_dir; local +X is (cos, -sin), as for terrain slots.
pub fn to_world(site: &Waystation, x: f32, z: f32) -> (f32, f32) {
    let (s, c) = crate::yaw_dir((site.phase as u16) << 8);
    (site.x + x * c + z * s, site.z - x * s + z * c)
}

pub fn to_local(site: &Waystation, x: f32, z: f32) -> (f32, f32) {
    let (s, c) = crate::yaw_dir((site.phase as u16) << 8);
    let (dx, dz) = (x - site.x, z - site.z);
    (dx * c - dz * s, dx * s + dz * c)
}

fn near(site: &Waystation, x: f32, z: f32, r: f32) -> bool {
    let (dx, dz) = (x - site.x, z - site.z);
    dx * dx + dz * dz <= (FOOTPRINT.stamp_m + r) * (FOOTPRINT.stamp_m + r)
}

/// Bounded by the roster and 29 boxes, with one circle rejection per depot.
pub fn blocks(haven: &Haven, x: f32, z: f32, feet: f32, r: f32, h: f32) -> bool {
    for site in &haven.minor {
        if !is_depot(site) || !near(site, x, z, r) {
            continue;
        }
        let (lx, lz) = to_local(site, x, z);
        for part in &DEPOT_PARTS {
            // The slab finishes at carved ground. Like the existing shelter
            // floor it supplies support, not a wall at quantized foot height.
            if part.bounds[4] <= 0.0 {
                continue;
            }
            let b = part.bounds;
            if feet >= site.floor_y + b[4] || feet + h <= site.floor_y + b[1] {
                continue;
            }
            let dx = lx - lx.clamp(b[0], b[3]);
            let dz = lz - lz.clamp(b[2], b[5]);
            if dx * dx + dz * dz < r * r {
                return true;
            }
        }
    }
    false
}

pub fn ground(haven: &Haven, x: f32, z: f32, feet: f32) -> f32 {
    let mut best = crate::collide::NO_SURFACE;
    for site in &haven.minor {
        if !is_depot(site) || !near(site, x, z, 0.0) {
            continue;
        }
        let (lx, lz) = to_local(site, x, z);
        for part in &DEPOT_PARTS {
            let b = part.bounds;
            let top = site.floor_y + b[4];
            if lx >= b[0]
                && lx <= b[3]
                && lz >= b[2]
                && lz <= b[5]
                && top <= feet + crate::movement::STEP_UP
            {
                best = best.max(top);
            }
        }
    }
    best
}

/// The whole depot is reserved, including gate approaches. Expand by a
/// build-cell diagonal so an outside anchor cannot overhang its reservation.
pub fn reserves(haven: &Haven, x: f32, z: f32, margin: f32) -> bool {
    haven.minor.iter().any(|site| {
        if !is_depot(site) {
            return false;
        }
        let (dx, dz) = (x - site.x, z - site.z);
        let r = FOOTPRINT.scatter_m + margin;
        dx * dx + dz * dz <= r * r
    })
}
