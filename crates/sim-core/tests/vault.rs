//! Sealed steel doors planted everywhere (`ARC.md` §5 M2): a vault at every
//! landmark, one outside the town's walls and one on the ziggurat, each on
//! ground a body walks to with its door clear, outside the safe zone, and
//! on ground nobody may build round.
// Host-side test: labels are Strings, and `abs` measures, nothing simulates.
#![allow(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods
)]

use sim_core::landmark::{self, LandmarkKind};
use sim_core::limits::MAX_VAULTS;
use sim_core::occupy::Scratch;
use sim_core::terrain::{
    self, Lattice, Occupant, ScatterTable, Slot, CELLS_PER_SIDE, CELL_SIZE, CLIFF_SLOPE_RATIO,
    LAND_MIN_H,
};

/// The shipped seed (`shard-public.toml`).
const SHIPPED: u64 = 20_260_731;
const SEEDS: [u64; 6] = [SHIPPED, 0x0047_4154_4553, 0xDEAD_BEEF, 42, 7, 31337];

/// How far in front of a vault's centre a body stands to open it: the
/// block's half-depth, a capsule and a hand's breadth.
const FRONT_M: f32 = 1.6;

fn slot_at(seed: u64, table: &ScatterTable, h: &terrain::Haven, x: f32, z: f32) -> Slot {
    terrain::scatter(
        seed,
        table,
        h,
        (x / CELL_SIZE) as i32,
        (z / CELL_SIZE) as i32,
    )
}

/// Every vault this seed authors, as scatter seats it, with what it stands
/// at: the landmarks' (one each), the town's and the ziggurat's.
fn authored(seed: u64, h: &terrain::Haven) -> Vec<(Option<LandmarkKind>, String, Slot)> {
    let table = ScatterTable::alpha_default();
    let mut out = Vec::new();
    for m in h.marks.iter().filter(|m| m.live) {
        let mut k = 0;
        while let Some((x, z, _, occ)) = landmark::anchor(m, k) {
            k += 1;
            if occ == Occupant::Vault {
                out.push((
                    Some(m.kind),
                    format!("{:?}", m.kind),
                    slot_at(seed, &table, h, x, z),
                ));
            }
        }
    }
    if let Some((x, z, _)) = sim_core::town::vault_world(&h.town) {
        out.push((None, "town".into(), slot_at(seed, &table, h, x, z)));
    }
    for k in 0..sim_core::monument::CRATES.len() {
        let (x, _, z, _, occ) = sim_core::monument::crate_world(&h.ziggurat, k).unwrap();
        if occ == Occupant::Vault && h.ziggurat.live {
            out.push((None, "ziggurat".into(), slot_at(seed, &table, h, x, z)));
        }
    }
    out
}

/// On the shipped seed: one vault at every landmark of every kind, the
/// town's and the ziggurat's, and none anywhere else on the island — a
/// whole-island scan finds exactly those, under the cap.
#[test]
fn the_shipped_island_has_a_vault_at_every_landmark_the_town_and_the_ziggurat() {
    let h = terrain::haven(SHIPPED);
    assert!(h.town.live && h.ziggurat.live);
    let table = ScatterTable::alpha_default();
    let mut lat = Lattice::new();
    let mut scanned = Vec::new();
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let s = terrain::scatter_memo(&mut lat, SHIPPED, &table, &h, cx, cz);
            if s.occupant == Occupant::Vault {
                scanned.push((s.x, s.z));
            }
        }
    }
    let placed = authored(SHIPPED, &h);
    let mut per_kind = [0u32; 10];
    let mut marks = [0u32; 10];
    for m in h.marks.iter().filter(|m| m.live) {
        marks[m.kind as usize] += 1;
    }
    for (kind, what, s) in &placed {
        assert_eq!(
            s.occupant,
            Occupant::Vault,
            "the {what} vault lost its cell"
        );
        if let Some(k) = kind {
            per_kind[*k as usize] += 1;
        }
    }
    // Every live landmark keeps its door, whatever kind it is.
    assert_eq!(per_kind, marks, "a landmark without its vault");
    // The shipped seed's count by kind: Mast, Ruin, Tower, Stones, Yard,
    // Anvil, Arch, Spires, Quarry, Relay. It moves with worldgen (a wipe).
    assert_eq!(
        per_kind, GOLDEN_PER_KIND,
        "the shipped island's vaults by kind"
    );
    assert_eq!(
        placed.len(),
        scanned.len(),
        "a vault scatter did not author"
    );
    for (_, _, s) in &placed {
        assert!(scanned.contains(&(s.x, s.z)));
    }
    assert!(scanned.len() <= MAX_VAULTS, "{} vaults", scanned.len());
}

const GOLDEN_PER_KIND: [u32; 10] = [0, 5, 0, 3, 1, 3, 1, 2, 4, 0];

/// Every vault on a handful of seeds stands on dry, walkable ground and in
/// no wall; its door faces ground a body can stand on, at the vault's own
/// level, with nothing in the way; nobody is safe at it, and nobody may
/// build there.
#[test]
fn every_vault_stands_on_walkable_ground_with_its_door_clear() {
    let mut kinds = [0u32; 10];
    let mut sites = 0;
    for seed in SEEDS {
        let mut scratch = Scratch::live(seed);
        let h = scratch.haven;
        let placed = authored(seed, &h);
        assert!(placed.len() >= 8, "seed {seed:#x}: {} vaults", placed.len());
        let mut occ = scratch.occupants();
        for (kind, what, s) in &placed {
            match kind {
                Some(k) => kinds[*k as usize] += 1,
                None => sites += 1,
            }
            let at = format!(
                "seed {seed:#x}: the {what} vault at ({:.1}, {:.1}, {:.1})",
                s.x, s.y, s.z
            );
            assert_eq!(s.occupant, Occupant::Vault, "{at} lost its cell");
            assert!(s.y > LAND_MIN_H, "{at} is in the sea");
            assert!(
                !sim_core::town::safe(&h.town, s.x, s.z),
                "{at} is in the safe zone"
            );
            assert!(
                terrain::build_reserved(&h, s.x, s.z, 0.0),
                "{at} stands where a player may build"
            );
            // Not set into a wall: no structure stops the block itself.
            let (r, top) = terrain::occupant_volume(Occupant::Vault);
            let walled = landmark::blocks(&h.marks, s.x, s.z, s.y, r, top)
                || sim_core::town::blocks(&h.town, s.x, s.z, s.y, r, top)
                || sim_core::monument::blocks(&h.ziggurat, 0, s.x, s.z, s.y, r, top);
            assert!(!walled, "{at} is set into a wall");
            // The door's front.
            let (dx, dz) = sim_core::yaw_dir((s.yaw as u16) << 8);
            let (fx, fz) = (s.x + dx * FRONT_M, s.z + dz * FRONT_M);
            let ground = terrain::ground(seed, &h, fx, fz);
            let feet = ground.max(occ.body_ground(seed, fx, fz, s.y));
            assert!(feet > LAND_MIN_H, "{at}: its door opens on the sea");
            assert!(
                (feet - s.y).abs() <= 1.0,
                "{at}: its door opens {:.2} m off its own floor",
                feet - s.y
            );
            if feet == ground {
                let sl = terrain::ground_slope(seed, &h, fx, fz);
                assert!(
                    sl < CLIFF_SLOPE_RATIO,
                    "{at}: its door opens on a cliff ({sl:.2})"
                );
            }
            assert!(!occ.blocks(seed, fx, fz, feet), "{at}: its door is blocked");
            assert!(
                terrain::build_reserved(&h, fx, fz, 0.0),
                "{at}: a player may build across its door"
            );
            assert!(
                !sim_core::town::safe(&h.town, fx, fz),
                "{at}: its door is safe"
            );
        }
    }
    // Every kit's door, and the town's and the ziggurat's, walked up to.
    assert!(kinds.iter().all(|&n| n > 0), "kinds unchecked: {kinds:?}");
    assert!(
        sites >= 2 * SEEDS.len() - 2,
        "{sites} town and ziggurat vaults"
    );
}
