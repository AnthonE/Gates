//! Where to build: the ground rules a plot must meet, what this body has
//! seen of other people's building, and the plot it would choose.
//!
//! The ground rules are the map's (`Haven`: terrain, the depot and landmark
//! reserves, the roads), which a player has. Other bases are not: the
//! client mirrors every piece on the island, but a player knows only the
//! buildings it has looked at, so [`Seen`] learns them through the view
//! cone and the terrain in between, a few records a frame. The server's
//! verdict on a placement stays the authority either way.
//!
//! The helpers the raid owner in `botclient.rs` settles with live here too,
//! so both builders read one set of ground rules.

use crate::view::ClientView;
use client_core::core::ClientCore;
use sim_core::bots::STARTER_FOOTPRINT;
use sim_core::build::{build_cell_of, BUILD_CELL_M};
use sim_core::deploy::ARCH_HEARTH;
use sim_core::limits::MAX_BUILD_COORD;
use sim_core::movement::POS_XZ_Q;
use sim_core::terrain::{self, Haven, RoadBand};
use sim_core::yaw_dir;

/// How far from another cupboard an owner settles, metres: its claim reaches
/// `claim::PRIV_CUSHION_M` past every cell of its building, and a base
/// spreads a couple of cells from its cupboard on either side.
pub(crate) const CLAIM_KEEP_M: f32 = sim_core::claim::PRIV_CUSHION_M + 4.0 * BUILD_CELL_M;

/// Other people's building this body can remember, as places.
pub const SEEN_ROWS: usize = 32;
/// A building is seen as far as a person is (`tracks::PLAYER_SIGHT_M`).
pub const BUILDING_SIGHT_M: f32 = 120.0;
/// Mirror records examined per frame, round robin, and the sight lines
/// they may cost: the scan is spread over frames rather than done at once.
pub const BUILDING_SCAN_PER_FRAME: usize = 16;
pub const BUILDING_RAYS_PER_FRAME: usize = 1;
/// Records this close to a remembered place are the same building.
pub const SEEN_MERGE_M: f32 = 12.0;
/// Terrain samples along a sight line to a building, metres apart.
const SIGHT_STEP_M: f32 = 2.0;
/// Build cells searched on each side of the chooser for a plot.
pub const PLOT_SEARCH_CELLS: i32 = 12;
/// A plot keeps this far from a landmark's edge: loot runs and raiders
/// both come through them.
pub const PLOT_LANDMARK_KEEP_M: f32 = 30.0;
/// A resource no nearer than this counts as this far for a plot's score.
pub const PLOT_RESOURCE_M: f32 = 60.0;

/// The middle of a build cell, metres.
pub(crate) fn cell_mid(cx: u16, cz: u16) -> (f32, f32) {
    let half = BUILD_CELL_M * 0.5;
    (
        cx as f32 * BUILD_CELL_M + half,
        cz as f32 * BUILD_CELL_M + half,
    )
}

/// The build cell a quantized body coordinate stands in.
///
/// `build_cell_of` is the sim's own function and the clamp is the one
/// `ui/place.rs:77` applies to a look-at point, so the bot addresses a cell
/// on the same grid the server validates against — the quantize-both-sides
/// law from the trap list, applied to a cell index. The island is
/// all-positive (`limits::MAX_BUILD_COORD`: a 2,048 m world over ~683 3 m
/// cells), so the clamp only ever bites on a body outside the playfield.
pub(crate) fn body_cell(q: i32) -> u16 {
    build_cell_of(q as f32 * POS_XZ_Q).clamp(0, MAX_BUILD_COORD as i32 - 1) as u16
}

/// Is another person (not an animal) standing within `r` cells of this one
/// (`r` = 0: in it), as far as the snapshots say? The raid owner's check:
/// it reads every body the snapshot carries. The agent asks its tracks.
pub(crate) fn someone_near(view: &ClientView, me: u32, cx: u16, cz: u16, r: u16) -> bool {
    view.entities.iter().any(|(id, e)| {
        *id != me
            && sim_core::mob::slot_of_id(*id).is_none()
            && body_cell(e.qx).abs_diff(cx) <= r
            && body_cell(e.qz).abs_diff(cz) <= r
    })
}

/// Would a foundation go on this cell? `build::place`'s ground rules: the
/// terrain, and every reserve (the depot, the town, the landmarks).
pub(crate) fn foundation_goes(seed: u64, hv: &Haven, cx: u16, cz: u16) -> bool {
    let (ax, az) = sim_core::build::anchor(cx, cz, sim_core::build::LOC_PLANE);
    let pad = BUILD_CELL_M * 1.5;
    sim_core::build::foundation_terrain_ok(seed, hv, ax, az)
        && !sim_core::terrain::build_reserved(hv, ax, az, pad)
}

#[derive(Clone, Copy, Debug, Default)]
struct Mark {
    x: f32,
    z: f32,
    /// When it was last in sight.
    tick: u32,
    used: bool,
}

/// What the building scan has cost.
#[derive(Clone, Copy, Debug, Default)]
pub struct SeenStats {
    pub examined: u64,
    pub rays: u64,
    pub hidden: u64,
}

/// Places this body has seen other people's building: a piece or a
/// cupboard that stood in its view cone, in range, with no terrain between.
/// A building is bigger than what hides a body, so trees and its own walls
/// do not hide it; a hill does.
pub struct Seen {
    marks: [Mark; SEEN_ROWS],
    piece: usize,
    deploy: usize,
    pub stats: SeenStats,
}

impl Default for Seen {
    fn default() -> Self {
        Self::new()
    }
}

impl Seen {
    pub fn new() -> Self {
        Self {
            marks: [Mark::default(); SEEN_ROWS],
            piece: 0,
            deploy: 0,
            stats: SeenStats::default(),
        }
    }

    /// Forget every building (a new session: a new island, maybe).
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// Remembered places, and when each was last in sight.
    pub fn places(&self) -> impl Iterator<Item = ([f32; 2], u32)> + '_ {
        self.marks
            .iter()
            .filter(|m| m.used)
            .map(|m| ([m.x, m.z], m.tick))
    }

    /// Nothing remembered within `keep_m` of the point.
    pub fn clear_of(&self, x: f32, z: f32, keep_m: f32) -> bool {
        self.places()
            .all(|([mx, mz], _)| (mx - x).hypot(mz - z) > keep_m)
    }

    /// Look at the next few mirror records. `own` names addresses this body
    /// built itself, which are not someone else's. Bounded per frame and
    /// allocation-free.
    #[allow(clippy::too_many_arguments)]
    pub fn look(
        &mut self,
        core: &ClientCore,
        seed: u64,
        haven: &Haven,
        eye: [f32; 3],
        yaw: u16,
        tick: u32,
        own: impl Fn(u16, u16, u8, u8) -> bool,
    ) {
        let mut rays = BUILDING_RAYS_PER_FRAME;
        let pieces = core.pieces.entries();
        let deploys = core.deploys.entries();
        let defs = &core.deploy_defs;
        let have = core.deploy_defs_have.min(defs.def_count);
        for _ in 0..BUILDING_SCAN_PER_FRAME {
            // Pieces and cupboards take turns; other deployables (a bag or
            // a box in a field) claim nothing and are passed over.
            if pieces.is_empty() && deploys.is_empty() {
                return;
            }
            let piece_turn = self.stats.examined.is_multiple_of(2) || deploys.is_empty();
            let at = if piece_turn && !pieces.is_empty() {
                self.piece = (self.piece + 1) % pieces.len();
                let r = pieces[self.piece];
                (r.cx, r.cz, r.level, r.loc)
            } else {
                self.deploy = (self.deploy + 1) % deploys.len();
                let r = deploys[self.deploy];
                let hearth =
                    u16::from(r.row) < have && defs.defs[r.row as usize].arch == ARCH_HEARTH;
                if !hearth {
                    self.stats.examined += 1;
                    continue;
                }
                (r.cx, r.cz, r.level, r.loc)
            };
            self.stats.examined += 1;
            let (cx, cz, level, loc) = at;
            if own(cx, cz, level, loc) {
                continue;
            }
            let (x, z) = cell_mid(cx, cz);
            let (dx, dz) = (x - eye[0], z - eye[2]);
            let d = dx.hypot(dz);
            let (fx, fz) = yaw_dir(yaw);
            if d > BUILDING_SIGHT_M || dx * fx + dz * fz < d * std::f32::consts::FRAC_1_SQRT_2 {
                continue;
            }
            if let Some(m) = self
                .marks
                .iter_mut()
                .find(|m| m.used && (m.x - x).hypot(m.z - z) <= SEEN_MERGE_M)
            {
                m.tick = tick;
                continue;
            }
            if rays == 0 {
                continue;
            }
            rays -= 1;
            self.stats.rays += 1;
            let y = terrain::ground(seed, haven, x, z) + 1.5;
            if terrain_hides(seed, haven, eye, [x, y, z]) {
                self.stats.hidden += 1;
                continue;
            }
            self.remember(x, z, tick);
        }
    }

    fn remember(&mut self, x: f32, z: f32, tick: u32) {
        let slot = match self.marks.iter().position(|m| !m.used) {
            Some(i) => i,
            None => {
                // Full: the place longest out of sight goes.
                let mut oldest = 0;
                for (i, m) in self.marks.iter().enumerate() {
                    if tick.wrapping_sub(m.tick) > tick.wrapping_sub(self.marks[oldest].tick) {
                        oldest = i;
                    }
                }
                oldest
            }
        };
        self.marks[slot] = Mark {
            x,
            z,
            tick,
            used: true,
        };
    }
}

/// Does the ground rise above the straight line between two points?
fn terrain_hides(seed: u64, haven: &Haven, from: [f32; 3], to: [f32; 3]) -> bool {
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let length = (d[0] * d[0] + d[2] * d[2]).sqrt();
    let steps = (length / SIGHT_STEP_M).ceil() as usize;
    (1..steps).any(|i| {
        let t = i as f32 / steps as f32;
        let (x, y, z) = (from[0] + d[0] * t, from[1] + d[1] * t, from[2] + d[2] * t);
        y < terrain::ground(seed, haven, x, z)
    })
}

/// Every column the starter's walls stand on, from the plot: its footprint
/// and the cells whose low edges close it (`sim_core::bots::STARTER`).
const PLOT_COLUMNS: [(u16, u16); 5] = [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1)];

/// Would the starter base go on this plot? Every cell of its footprint
/// takes a foundation, off the roads and clear of the landmarks, no
/// building this body has seen is within a claim's reach, and the ground
/// is even enough that every wall latches to the core's plate.
pub fn plot_goes(seed: u64, haven: &Haven, seen: &Seen, cx: u16, cz: u16) -> bool {
    if usize::from(cx) + 2 >= MAX_BUILD_COORD || usize::from(cz) + 1 >= MAX_BUILD_COORD {
        return false;
    }
    let band = sim_core::build::terrain_band(seed, haven, cx, cz);
    let even = PLOT_COLUMNS.iter().all(|&(dx, dz)| {
        (sim_core::build::terrain_band(seed, haven, cx + dx, cz + dz) - band).abs() <= 1
    });
    even && STARTER_FOOTPRINT.iter().all(|&(dx, dz)| {
        let (Some(x), Some(z)) = (
            cx.checked_add_signed(i16::from(dx)),
            cz.checked_add_signed(i16::from(dz)),
        ) else {
            return false;
        };
        let (mx, mz) = cell_mid(x, z);
        usize::from(x) < MAX_BUILD_COORD
            && usize::from(z) < MAX_BUILD_COORD
            && foundation_goes(seed, haven, x, z)
            && terrain::road_band(seed, haven, mx, mz) == RoadBand::Off
            && !sim_core::landmark::covers(&haven.marks, mx, mz, PLOT_LANDMARK_KEEP_M)
            && seen.clear_of(mx, mz, CLAIM_KEEP_M)
    })
}

/// The plot to build the starter on, near `from`: the one that goes
/// ([`plot_goes`]) and that the caller does not `avoid`, with the least
/// walking to it and from it to the wood and stone this body remembers.
/// Bounded (`PLOT_SEARCH_CELLS` on each side) and allocation-free; `None`
/// when nothing near goes.
pub fn pick_plot(
    seed: u64,
    haven: &Haven,
    seen: &Seen,
    from: [f32; 2],
    wood: Option<[f32; 2]>,
    stone: Option<[f32; 2]>,
    avoid: impl Fn(u16, u16) -> bool,
) -> Option<(u16, u16)> {
    let fx = build_cell_of(from[0]);
    let fz = build_cell_of(from[1]);
    let near = |to: Option<[f32; 2]>, x: f32, z: f32| {
        to.map_or(PLOT_RESOURCE_M, |[tx, tz]| {
            (tx - x).hypot(tz - z).min(PLOT_RESOURCE_M)
        })
    };
    let mut best: Option<(f32, u16, u16)> = None;
    for dz in -PLOT_SEARCH_CELLS..=PLOT_SEARCH_CELLS {
        for dx in -PLOT_SEARCH_CELLS..=PLOT_SEARCH_CELLS {
            let (cx, cz) = (fx + dx, fz + dz);
            if !(0..MAX_BUILD_COORD as i32).contains(&cx)
                || !(0..MAX_BUILD_COORD as i32).contains(&cz)
            {
                continue;
            }
            let (cx, cz) = (cx as u16, cz as u16);
            let (x, z) = cell_mid(cx, cz);
            let score = (x - from[0]).hypot(z - from[1]) + near(wood, x, z) + near(stone, x, z);
            // The cheap score first: the ground rules cost terrain samples.
            if best.is_some_and(|(b, ..)| score >= b)
                || !plot_goes(seed, haven, seen, cx, cz)
                || avoid(cx, cz)
            {
                continue;
            }
            best = Some((score, cx, cz));
        }
    }
    best.map(|(_, cx, cz)| (cx, cz))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::build::{PieceRec, LOC_PLANE};

    const SEED: u64 = 20260731;

    fn core() -> Box<ClientCore> {
        Box::new(ClientCore::new(SEED, 1, 0))
    }

    /// A plot that goes, near the island's middle.
    fn a_plot(haven: &Haven) -> (u16, u16) {
        let c = build_cell_of(terrain::ISLAND_SIZE * 0.5);
        for r in 0..200 {
            for (dx, dz) in [(r, 0), (-r, 0), (0, r), (0, -r)] {
                let (cx, cz) = ((c + dx) as u16, (c + dz) as u16);
                if plot_goes(SEED, haven, &Seen::new(), cx, cz) {
                    return (cx, cz);
                }
            }
        }
        panic!("no plot on the fixture island");
    }

    #[test]
    fn a_seen_building_moves_the_plot_and_an_unseen_one_does_not() {
        let haven = terrain::haven(SEED);
        let (cx, cz) = a_plot(&haven);
        let (x, z) = cell_mid(cx, cz);
        let mut core = core();
        // Somebody's foundation right on the plot, and this body standing
        // 20 m south of it, facing north (+z) or south.
        let rec = PieceRec {
            cx,
            cz,
            loc: LOC_PLANE,
            hp: 100,
            ..PieceRec::default()
        };
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_piece_sync(true, &[rec], &mut buf).unwrap();
        core.on_stream(&buf[..n]).unwrap();
        let eye = [
            x,
            terrain::ground(SEED, &haven, x, z - 20.0) + 1.6,
            z - 20.0,
        ];
        let mut seen = Seen::new();
        // Facing away: nothing learned, the plot still goes.
        for tick in 0..8 {
            seen.look(&core, SEED, &haven, eye, 1 << 15, tick, |_, _, _, _| false);
        }
        assert!(seen.places().next().is_none());
        assert_eq!(
            pick_plot(SEED, &haven, &seen, [x, z], None, None, |_, _| false),
            Some((cx, cz))
        );
        // Its own building is not someone else's.
        for tick in 0..8 {
            seen.look(&core, SEED, &haven, eye, 0, tick, |_, _, _, _| true);
        }
        assert!(seen.places().next().is_none());
        // Facing it: remembered, and the chooser walks off the claim.
        for tick in 0..8 {
            seen.look(&core, SEED, &haven, eye, 0, tick, |_, _, _, _| false);
        }
        assert_eq!(seen.places().count(), 1, "{:?}", seen.stats);
        assert!(!seen.clear_of(x, z, CLAIM_KEEP_M));
        let moved = pick_plot(SEED, &haven, &seen, [x, z], None, None, |_, _| false);
        if let Some((mx, mz)) = moved {
            let (px, pz) = cell_mid(mx, mz);
            assert!((px - x).hypot(pz - z) > CLAIM_KEEP_M);
            assert!(plot_goes(SEED, &haven, &seen, mx, mz));
        }
    }

    /// The blueprint read as data resolves to the rows the scripted owner
    /// is handed by id: a part's shape in twig, a kit's arch.
    #[test]
    fn every_starter_part_and_kit_resolves_in_the_shipped_tables() {
        use sim_core::bots::{kit_arch, part_shape, BaseOp, Kit, Part, STARTER};
        use sim_core::build::{row_of, MAT_TWIG};
        let content = content::Content::from_sources(&crate::agent::wiki::SOURCES).unwrap();
        let tables = crate::net::bake_all(&content).unwrap();
        let rows = crate::population::base_rows(&content).unwrap();
        for op in STARTER {
            match *op {
                BaseOp::Place(part, ..) => {
                    let want = match part {
                        Part::Foundation => rows.foundation,
                        Part::TriFoundation => rows.tri_foundation,
                        Part::Wall => rows.wall,
                        Part::Doorway => rows.doorway,
                        Part::Floor => rows.floor,
                        Part::Stairs => rows.stairs,
                        Part::Roof => rows.roof,
                        Part::TriRoof => rows.tri_roof,
                    };
                    assert_eq!(
                        row_of(&tables.build, part_shape(part), MAT_TWIG),
                        Some(want),
                        "{part:?}"
                    );
                }
                BaseOp::Deploy(kit, ..) => {
                    let row = match kit {
                        Kit::Hearth => rows.hearth,
                        Kit::Door => rows.door,
                        Kit::MetalDoor => rows.metal_door,
                        Kit::Lock => rows.lock,
                        Kit::Box => rows.container,
                    };
                    assert_eq!(
                        tables.deploy.defs[usize::from(row)].arch,
                        kit_arch(kit),
                        "{kit:?}"
                    );
                }
                _ => {}
            }
        }
    }

    #[test]
    fn the_plot_leans_toward_the_wood_and_stone() {
        let haven = terrain::haven(SEED);
        let (cx, cz) = a_plot(&haven);
        let (x, z) = cell_mid(cx, cz);
        let seen = Seen::new();
        let east = [x + 30.0, z];
        let (px, _) = cell_mid(
            pick_plot(
                SEED,
                &haven,
                &seen,
                [x, z],
                Some(east),
                Some(east),
                |_, _| false,
            )
            .expect("a plot near a plot that goes")
            .0,
            0,
        );
        assert!(px > x, "{px} not east of {x}");
    }
}
