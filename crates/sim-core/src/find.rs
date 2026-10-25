//! Finding ground on a generated island: the fixture helpers.
//!
//! The world is made by the generator, so a test, a capture or a dev spawn
//! never types a place: it asks for the ground it needs and searches for it,
//! the way the reference places a monument by querying the terrain. A
//! fixture may start looking somewhere (its *hint*, kept so its picks stay
//! stable while the island does), but a hint is never a requirement. Every
//! search here also wants **mild** ground, which no climate reaches, so a
//! band of desert, a cold band or the Verdigris moving over a hint costs a
//! fixture a longer walk instead of a broken test.
//!
//! Pure functions of the seed and integers, like the rest of the crate.

use crate::terrain;

/// Ground no climate band reaches: not the desert, not the arctic, not the
/// Verdigris. Where a fixture tests a verb rather than a climate, the climate
/// stays out of it. (A snowcap is height, and a fixture's level ground is
/// never up on one.)
pub fn mild(seed: u64, x: f32, z: f32) -> bool {
    terrain::aridity(seed, x, z) == 0.0
        && terrain::arctic(seed, x, z) == 0.0
        && terrain::fel(seed, x, z) == 0.0
}

/// [`mild`] at a scatter cell's centre.
pub fn mild_cell(seed: u64, cx: i32, cz: i32) -> bool {
    let (x, z) = cell_centre(cx, cz);
    mild(seed, x, z)
}

/// A scatter cell's centre, metres.
pub fn cell_centre(cx: i32, cz: i32) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * terrain::CELL_SIZE,
        (cz as f32 + 0.5) * terrain::CELL_SIZE,
    )
}

/// Square rings of grid offsets around `(cx, cz)`, ring 0 (the point itself)
/// to ring `max_r` inclusive, each ring row by row from its south edge. The
/// order every fixture's hand-written spiral already walked, so a fixture
/// moved onto it picks what it picked before.
pub fn rings(cx: i32, cz: i32, max_r: i32) -> impl Iterator<Item = (i32, i32)> {
    (0..=max_r).flat_map(move |r| {
        (-r..=r).flat_map(move |dz| {
            (-r..=r)
                .filter(move |&dx| dx.abs() == r || dz.abs() == r)
                .map(move |dx| (cx + dx, cz + dz))
        })
    })
}

/// Every scatter cell on the island, ring by ring out from its centre: the
/// fallback after a fixture's hint runs dry.
pub fn island_cells() -> impl Iterator<Item = (i32, i32)> {
    let c = terrain::CELLS_PER_SIDE / 2;
    rings(c, c, c).filter(|&(cx, cz)| {
        (0..terrain::CELLS_PER_SIDE).contains(&cx) && (0..terrain::CELLS_PER_SIDE).contains(&cz)
    })
}

/// A fixture's scatter cells: its own `hint` order first, then
/// [`island_cells`], keeping only [`mild`] ones.
pub fn mild_cells(
    seed: u64,
    hint: impl Iterator<Item = (i32, i32)>,
) -> impl Iterator<Item = (i32, i32)> {
    hint.chain(island_cells())
        .filter(move |&(cx, cz)| mild_cell(seed, cx, cz))
}

/// The first point, in rings of `step` metres out from `hint` to `max_r`
/// rings, that is [`mild`] land and that `want` takes.
pub fn near(
    seed: u64,
    hint: (f32, f32),
    step: f32,
    max_r: i32,
    mut want: impl FnMut(f32, f32) -> bool,
) -> Option<(f32, f32)> {
    rings(0, 0, max_r)
        .map(|(dx, dz)| (hint.0 + dx as f32 * step, hint.1 + dz as f32 * step))
        .find(|&(x, z)| {
            (0.0..terrain::ISLAND_SIZE).contains(&x)
                && (0.0..terrain::ISLAND_SIZE).contains(&z)
                && terrain::height(seed, x, z) > terrain::LAND_MIN_H
                && mild(seed, x, z)
                && want(x, z)
        })
}

/// [`near`] from `hint`, then from the island's centre out to its edge: a
/// point is found wherever the climates have moved.
pub fn ground(
    seed: u64,
    hint: (f32, f32),
    step: f32,
    mut want: impl FnMut(f32, f32) -> bool,
) -> (f32, f32) {
    let c = terrain::ISLAND_SIZE * 0.5;
    let reach = (c / step) as i32;
    near(seed, hint, step, (reach / 4).max(1), &mut want)
        .or_else(|| near(seed, (c, c), step, reach, &mut want))
        .unwrap_or_else(|| {
            panic!("seed {seed}: no mild ground the fixture wants — the generator moved")
        })
}

/// A build cell to start a fixture's build-grid spiral from: `(cx, cz)` when
/// that cell is mild land, else the nearest one that is, one build cell at a
/// time. A spiral that starts on mild ground finds its foundation there.
pub fn build_hint(seed: u64, cx: i32, cz: i32) -> (i32, i32) {
    let m = crate::build::BUILD_CELL_M;
    let at = ((cx as f32 + 0.5) * m, (cz as f32 + 0.5) * m);
    let (x, z) = ground(seed, at, m, |_, _| true);
    (
        crate::build::build_cell_of(x),
        crate::build::build_cell_of(z),
    )
}

/// How far out of the island's centre a found stance starts: past the town
/// and the ground it blends, where nothing authored stands.
const CLEAR_OF_TOWN_M: f32 = crate::town::TOWN_R_MAX + 100.0;
/// Level enough to stand a camera and a base on.
const LEVEL_SLOPE: f32 = 0.2;

fn clear_of_town(x: f32, z: f32) -> bool {
    let c = terrain::ISLAND_SIZE * 0.5;
    let (dx, dz) = (x - c, z - c);
    dx * dx + dz * dz >= CLEAR_OF_TOWN_M * CLEAR_OF_TOWN_M
}

/// How far out [`spawn_ground`] stands, in units of the continent's radius:
/// the outer island, off the flat walkable interior where the roster's mob
/// homes concentrate (an unarmed capture probe died there three runs in four).
const SPAWN_OUT: f32 = 0.75;

/// A dev or capture stance asked for rather than typed (`dev_spawn =
/// "auto"`): mild, level meadow or forest on the outer island, the first the
/// rings out of the centre reach.
pub fn spawn_ground(seed: u64) -> (f32, f32) {
    let c = terrain::ISLAND_SIZE * 0.5;
    let r = SPAWN_OUT * terrain::CONTINENT_RADIUS;
    ground(seed, (c, c), 8.0, |x, z| {
        (x - c) * (x - c) + (z - c) * (z - c) >= r * r
            && terrain::slope(seed, x, z) < LEVEL_SLOPE
            && matches!(
                terrain::biome_at(seed, terrain::height(seed, x, z), x, z),
                terrain::Biome::Meadow | terrain::Biome::Forest
            )
    })
}

/// Level land of this biome, nearest the island's centre and clear of the
/// town (`dev_spawn = "biome:<name>"`): the way a capture is pointed at a
/// climate. Climate is the point here, so it need not be mild.
pub fn biome_ground(seed: u64, want: terrain::Biome) -> Option<(f32, f32)> {
    let c = terrain::ISLAND_SIZE * 0.5;
    let step = 8.0;
    rings(0, 0, (c / step) as i32)
        .map(|(dx, dz)| (c + dx as f32 * step, c + dz as f32 * step))
        .find(|&(x, z)| {
            let h = terrain::height(seed, x, z);
            h > terrain::BEACH_MAX_H
                && clear_of_town(x, z)
                && terrain::slope(seed, x, z) < LEVEL_SLOPE
                && terrain::biome_at(seed, h, x, z) == want
        })
}

/// A biome by the name content spells it (`terrain::BIOME_NAMES`).
pub fn biome_named(name: &str) -> Option<terrain::Biome> {
    terrain::BIOME_NAMES
        .iter()
        .position(|n| *n == name)
        .map(|i| terrain::ALL_BIOMES[i])
}

/// Ground nothing authored stands on or sweeps: no site's floor
/// (`terrain::site_sweep`) and no landmark within `pad` metres. A fixture
/// that wants open ground wants the wilderness, not a carved yard whose
/// flatness is a structure's.
pub fn unbuilt(haven: &terrain::Haven, x: f32, z: f32, pad: f32) -> bool {
    terrain::site_sweep(haven, x, z) == 0.0 && !crate::landmark::covers(&haven.marks, x, z, pad)
}
