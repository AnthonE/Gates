//! Farming v2 on the client, the pure half: which bed spot a seed in hand
//! would go into, or else which patch of open ground and whether it is soft;
//! which planter a bed plant stands in, and whether the hand holds water.
//! Bevy-free, so the arithmetic is tested headless; the ghost and the click
//! are `render::farm_ghost`'s.

use protocol::WirePlant;
use sim_core::deploy::{DeployContent, DeployRec, ARCH_PLANTER, DEPLOY_VOL};
use sim_core::farm::{
    ground_site, in_sow_reach, spot_xz, Ground, BED_SOIL_DROP_M, GROUND_SPACING_M, PLANTER_SPOTS,
};
use sim_core::movement::{quant_xz, POS_XZ_Q};

/// What a seed in hand would do where the crosshair is: the planting
/// ghost's verdict, and the HUD's line for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlantAim {
    /// Nowhere to plant: the ghost hides.
    #[default]
    None,
    /// A free spot in a planter's bed.
    Bed,
    /// Open ground the ghost finds soft: the sim's own ground rules,
    /// its soft biomes (`farm::SOFT_SOIL`) and its scatter's included.
    Ground,
    /// Open ground the sim would refuse, and why, in the line's words.
    No(&'static str),
}

/// How a [`Ground`] refusal reads on the planting line.
pub fn ground_words(g: Ground) -> &'static str {
    use sim_core::terrain::Biome;
    match g {
        Ground::Road => "ON THE ROAD",
        Ground::Roofed => "UNDER A ROOF",
        Ground::Water => "TOO WET",
        Ground::Barren(Biome::Beach | Biome::Desert) => "TOO SANDY",
        Ground::Barren(Biome::Fel) => "TOO SOUR",
        Ground::Barren(Biome::Snow) => "TOO COLD",
        Ground::Occupied => "IN THE WAY",
        Ground::Crowded => "TOO CLOSE",
        Ground::Reserved | Ground::Claimed | Ground::Steep | Ground::Built | Ground::Barren(_) => {
            "TOO STONY"
        }
    }
}

/// A patch of open ground under the crosshair ([`ground_target`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundSpot {
    /// The point as the wire carries it (`movement::quant_xz`).
    pub q: (i32, i32),
    /// Its soil point, where the ghost stands.
    pub at: (f32, f32, f32),
    pub aim: PlantAim,
}

/// The open ground a seed in hand would go into: the look ray's hit on the
/// terrain at `hit` (planar), quantized as the wire carries it, with the
/// verdict — the sim's own ground rules ([`ground_site`]: road, reserve,
/// slope, the soft biomes, roof, what is built there, the island's scatter
/// in `occ`, with no claim, which the wire never carries) and the spacing
/// to the ground plants in `plants`. `None` past a hand's reach of `eye`.
/// `occ` carries `occupy::Pristine` (the scatter as the seed draws it).
pub fn ground_target(
    seed: u64,
    haven: &sim_core::terrain::Haven,
    cols: &sim_core::collide::ColIndex,
    occ: &mut sim_core::occupy::Occupants<'_>,
    eye: (f32, f32, f32),
    hit: (f32, f32),
    plants: &[WirePlant],
) -> Option<GroundSpot> {
    let q = (quant_xz(hit.0), quant_xz(hit.1));
    let (x, z) = (q.0 as f32 * POS_XZ_Q, q.1 as f32 * POS_XZ_Q);
    let site = ground_site(seed, haven, cols, occ, x, z, || false);
    let y = match site {
        Ok(y) => y,
        Err(_) => sim_core::terrain::ground(seed, haven, x, z),
    };
    if !in_sow_reach((x, y, z), eye) {
        return None;
    }
    let crowded = plants.iter().any(|p| {
        let (px, _, pz) = p.pos();
        !p.in_bed() && (px - x).powi(2) + (pz - z).powi(2) < GROUND_SPACING_M * GROUND_SPACING_M
    });
    let aim = match site {
        Err(g) => PlantAim::No(ground_words(g)),
        Ok(_) if crowded => PlantAim::No("TOO CLOSE"),
        Ok(_) => PlantAim::Ground,
    };
    Some(GroundSpot {
        q,
        at: (x, y, z),
        aim,
    })
}

/// How far from a spot's centre the crosshair may land on the soil and
/// still name that spot, metres.
pub const SPOT_PICK_M: f32 = 0.35;
/// A plant this close to a spot's centre (xz) is the one in it, metres:
/// wider than a position quantum, narrower than the spots' pitch.
pub const SPOT_TAKEN_M: f32 = 0.05;
/// And this close to that planter's soil (y), metres: wider than a height
/// quantum, far under the half storey between two planters stacked in one
/// column, whose spots share their xz.
pub const SPOT_SOIL_M: f32 = 0.1;

/// Whether a plant at height `py` stands in the soil of a planter whose
/// floor is `floor_y`. A floor the client cannot place yet (no island,
/// non-finite) rules nothing out.
fn on_soil(py: f32, floor_y: f32) -> bool {
    !floor_y.is_finite() || (py - floor_y - soil_rise()).abs() <= SPOT_SOIL_M
}

/// The planter's soil height over its floor, metres.
pub fn soil_rise() -> f32 {
    DEPLOY_VOL[ARCH_PLANTER as usize][1] - BED_SOIL_DROP_M
}

/// The free bed spot of planter `rec` nearest where the eye's ray meets
/// its soil: `(box key, spot, the spot's soil point)`. `floor_y` is the
/// planter's floor (its feet), `eye` and `dir` the eye's ray, `plants` the
/// mirror. `None` when the ray misses the soil, lands past
/// [`SPOT_PICK_M`] from every free spot, or every spot is taken.
pub fn plant_target(
    rec: &DeployRec,
    floor_y: f32,
    eye: (f32, f32, f32),
    dir: (f32, f32, f32),
    plants: &[WirePlant],
) -> Option<(u32, u8, (f32, f32, f32))> {
    let soil = floor_y + soil_rise();
    if dir.1 >= -1e-4 || eye.1 <= soil {
        return None;
    }
    let t = (soil - eye.1) / dir.1;
    let (hx, hz) = (eye.0 + dir.0 * t, eye.2 + dir.2 * t);
    let taken = |sx: f32, sz: f32| {
        plants.iter().any(|p| {
            let (px, py, pz) = p.pos();
            p.in_bed()
                && on_soil(py, floor_y)
                && (px - sx).powi(2) + (pz - sz).powi(2) <= SPOT_TAKEN_M * SPOT_TAKEN_M
        })
    };
    let mut best: Option<(u8, f32, f32, f32)> = None;
    for spot in 0..PLANTER_SPOTS {
        let (sx, sz) = spot_xz(rec.cx, rec.cz, rec.pose, spot);
        let d2 = (hx - sx).powi(2) + (hz - sz).powi(2);
        if d2 > SPOT_PICK_M * SPOT_PICK_M || taken(sx, sz) {
            continue;
        }
        if best.is_none_or(|b| d2 < b.1) {
            best = Some((spot, d2, sx, sz));
        }
    }
    let (spot, _, sx, sz) = best?;
    let key = sim_core::deploy::box_key(rec.cx, rec.cz, rec.level, rec.loc);
    (key != 0).then_some((key, spot, (sx, soil, sz)))
}

/// The planter bed plant `p` stands in, off the deploy mirror: the planter
/// one of whose spot centres is where the plant is, at its soil's height
/// (the wire record names its spot, not its host, and planters stacked a
/// storey apart share their spots' xz). `have` is
/// `ClientCore::deploy_defs_have`; `floor_y` a planter's floor (its feet,
/// `deploy::body_base_y`).
pub fn planter_of<'a>(
    p: &WirePlant,
    deploys: &'a [DeployRec],
    defs: &DeployContent,
    have: u16,
    floor_y: impl Fn(&DeployRec) -> f32,
) -> Option<&'a DeployRec> {
    if !p.in_bed() {
        return None;
    }
    let (px, py, pz) = p.pos();
    deploys.iter().find(|d| {
        (d.row as u16) < have
            && defs.defs[d.row as usize].arch == ARCH_PLANTER
            && {
                let (sx, sz) = spot_xz(d.cx, d.cz, d.pose, p.spot);
                (px - sx).powi(2) + (pz - sz).powi(2) <= SPOT_TAKEN_M * SPOT_TAKEN_M
            }
            && on_soil(py, floor_y(d))
    })
}

/// Whether the hand gets at plant `p` the way the shard asks it
/// (`World::plant_reachable`): a bed plant through its planter (`planter`,
/// off [`planter_of`]; one the mirror does not hold cannot be reached), a
/// ground plant where it lies. `deploy_clear` and `lying_clear` are the
/// line tests the deploy and loose-stack picks use (`sim_core::reach`).
pub fn plant_in_sight(
    p: &WirePlant,
    planter: Option<&DeployRec>,
    deploy_clear: impl Fn(&DeployRec) -> bool,
    lying_clear: impl Fn((i32, i32, i32)) -> bool,
) -> bool {
    if p.in_bed() {
        planter.is_some_and(deploy_clear)
    } else {
        lying_clear((p.qx, p.qy, p.qz))
    }
}

/// Whether a stack is the planter's waterskin (`CookContent::roles`,
/// `ROLE_FUEL` is `water_vessel` alone); its water is the server's to read.
pub fn is_water(roles_at_planter: Option<u8>) -> bool {
    roles_at_planter.is_some_and(|r| r & sim_core::oven::ROLE_FUEL != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::build::BUILD_CELL_M;
    use sim_core::footprint::Pose;
    use sim_core::movement::{quant_xz, quant_y};

    fn planter(yaw: u8) -> DeployRec {
        DeployRec {
            cx: 100,
            cz: 200,
            pose: Pose { ox: 0, oz: 0, yaw },
            row: 0,
            ..DeployRec::default()
        }
    }

    fn plant_at(spot: u8, x: f32, y: f32, z: f32) -> WirePlant {
        WirePlant {
            id: 1 + spot as u32,
            spot,
            qx: quant_xz(x),
            qy: quant_y(y),
            qz: quant_xz(z),
            ..WirePlant::default()
        }
    }

    /// A ray from 1.6 m over the floor, a metre back, down onto `(x, z)`.
    fn ray_to(x: f32, z: f32, floor: f32) -> ((f32, f32, f32), (f32, f32, f32)) {
        let eye = (x, floor + 1.6, z - 1.0);
        let to = (x, floor + soil_rise(), z);
        let d = (to.0 - eye.0, to.1 - eye.1, to.2 - eye.2);
        let n = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt();
        (eye, (d.0 / n, d.1 / n, d.2 / n))
    }

    #[test]
    fn the_crosshair_names_the_spot_it_lands_on_and_skips_a_taken_one() {
        let floor = 12.0;
        for yaw in [0u8, 64, 37] {
            let rec = planter(yaw);
            for spot in 0..PLANTER_SPOTS {
                let (sx, sz) = spot_xz(rec.cx, rec.cz, rec.pose, spot);
                let (eye, dir) = ray_to(sx + 0.05, sz - 0.04, floor);
                let (key, got, at) = plant_target(&rec, floor, eye, dir, &[]).expect("a spot");
                assert_eq!(got, spot, "yaw {yaw}");
                assert_eq!(key, sim_core::deploy::box_key(100, 200, 0, 0));
                assert!((at.0 - sx).abs() < 1e-4 && (at.2 - sz).abs() < 1e-4);
                assert!((at.1 - floor - soil_rise()).abs() < 1e-4);
                // Taken: the nearest free spot within reach of the hit, or none.
                let there = plant_at(spot, sx, floor + soil_rise(), sz);
                if let Some((_, other, _)) = plant_target(&rec, floor, eye, dir, &[there]) {
                    assert_ne!(other, spot);
                }
            }
        }
        // Off the bed: nothing.
        let rec = planter(0);
        let (x, z) = (
            (rec.cx as f32 + 0.5) * BUILD_CELL_M + 1.4,
            (rec.cz as f32 + 0.5) * BUILD_CELL_M,
        );
        let (eye, dir) = ray_to(x, z, floor);
        assert!(plant_target(&rec, floor, eye, dir, &[]).is_none());
        // Looking up: nothing.
        assert!(plant_target(&rec, floor, (0.0, 20.0, 0.0), (0.0, 1.0, 0.0), &[]).is_none());
    }

    #[test]
    fn a_bed_plant_finds_its_planter_by_its_spot() {
        let mut defs = DeployContent::EMPTY;
        defs.defs[0].arch = ARCH_PLANTER;
        defs.def_count = 1;
        let a = planter(0);
        let b = DeployRec {
            cx: 101,
            ..planter(0)
        };
        let (sx, sz) = spot_xz(b.cx, b.cz, b.pose, 3);
        let p = plant_at(3, sx, 10.0 + soil_rise(), sz);
        let deploys = [a, b];
        let floor = |_: &DeployRec| 10.0;
        assert_eq!(
            planter_of(&p, &deploys, &defs, 1, floor).map(|d| d.cx),
            Some(101)
        );
        assert!(
            planter_of(&p, &deploys, &defs, 0, floor).is_none(),
            "an undripped row"
        );
        let ground = WirePlant {
            spot: sim_core::farm::SPOT_GROUND,
            ..p
        };
        assert!(planter_of(&ground, &deploys, &defs, 1, floor).is_none());
    }

    /// Two planters stacked a storey apart share their spots' xz: a plant
    /// is the one whose soil it stands in, and a plant downstairs does not
    /// take the spot over it.
    #[test]
    fn stacked_planters_are_told_apart_by_their_soil() {
        let mut defs = DeployContent::EMPTY;
        defs.defs[0].arch = ARCH_PLANTER;
        defs.def_count = 1;
        let (low, high) = (10.0, 10.0 + sim_core::build::LEVEL_H_M);
        let down = planter(0);
        let up = DeployRec {
            level: 1,
            ..planter(0)
        };
        let floor = |d: &DeployRec| if d.level == 0 { low } else { high };
        let deploys = [down, up];
        let (sx, sz) = spot_xz(up.cx, up.cz, up.pose, 2);
        for (y, want) in [(low, 0u8), (high, 1)] {
            let p = plant_at(2, sx, y + soil_rise(), sz);
            let got = planter_of(&p, &deploys, &defs, 1, floor).map(|d| d.level);
            assert_eq!(got, Some(want), "the plant at {y}");
        }
        // A plant in the lower bed leaves the upper bed's spot free.
        let below = plant_at(2, sx, low + soil_rise(), sz);
        let (eye, dir) = ray_to(sx, sz, high);
        let got = plant_target(&up, high, eye, dir, &[below]).map(|t| t.1);
        assert_eq!(got, Some(2));
        let above = plant_at(2, sx, high + soil_rise(), sz);
        let got = plant_target(&up, high, eye, dir, &[above]).map(|t| t.1);
        assert_ne!(got, Some(2));
    }

    /// The open ground under the crosshair: in reach it is offered at the
    /// wire's quantized point, a neighbour inside the spacing reddens it,
    /// the road reddens it in its own words, and past reach it is nothing.
    #[test]
    fn the_ground_under_the_crosshair_is_offered_in_reach_and_reddens_by_rule() {
        use sim_core::terrain::{road_band, RoadBand, LAND_MIN_H};
        let seed = 0x0FEE_0FEE;
        let haven = sim_core::terrain::haven(seed);
        let cols = sim_core::collide::ColIndex::new();
        let table = sim_core::terrain::ScatterTable::alpha_default();
        let mut cache = Box::new(sim_core::occupy::SlotCache::new());
        let mut occ = sim_core::occupy::Occupants {
            table: &table,
            haven: &haven,
            harvested: &sim_core::occupy::Pristine,
            cache: &mut cache,
            doors: 0,
        };
        let find = |want: &mut dyn FnMut(f32, f32) -> bool| {
            (0..40_000)
                .map(|k| {
                    (
                        1500.0 + (k % 200) as f32 * 7.0,
                        1500.0 + (k / 200) as f32 * 7.0,
                    )
                })
                .find(|&(x, z)| want(x, z))
                .expect("a fixture point")
        };
        let (x, z) =
            find(&mut |x, z| ground_site(seed, &haven, &cols, &mut occ, x, z, || false).is_ok());
        let y = sim_core::terrain::ground(seed, &haven, x, z);
        let eye = (x, y + 1.6, z + 1.0);
        let GroundSpot { q, at, aim } =
            ground_target(seed, &haven, &cols, &mut occ, eye, (x, z), &[]).expect("in reach");
        assert_eq!(q, (quant_xz(x), quant_xz(z)));
        assert!((at.0 - x).abs() < POS_XZ_Q && (at.2 - z).abs() < POS_XZ_Q);
        assert_eq!(aim, PlantAim::Ground);
        // A ground plant 0.3 m off crowds it; a bed plant there does not.
        let near = WirePlant {
            spot: sim_core::farm::SPOT_GROUND,
            ..plant_at(0, x + 0.3, y, z)
        };
        let got = ground_target(seed, &haven, &cols, &mut occ, eye, (x, z), &[near]).map(|t| t.aim);
        assert_eq!(got, Some(PlantAim::No("TOO CLOSE")));
        let bed = plant_at(0, x + 0.3, y, z);
        let got = ground_target(seed, &haven, &cols, &mut occ, eye, (x, z), &[bed]).map(|t| t.aim);
        assert_eq!(got, Some(PlantAim::Ground));
        // Past a hand's reach: no ghost.
        let far = (x, y + 1.6, z + 4.0);
        assert!(ground_target(seed, &haven, &cols, &mut occ, far, (x, z), &[]).is_none());
        // The road says so.
        let (x, z) = find(&mut |x, z| {
            sim_core::terrain::ground(seed, &haven, x, z) >= LAND_MIN_H
                && road_band(seed, &haven, x, z) == RoadBand::Carriageway
        });
        let y = sim_core::terrain::ground(seed, &haven, x, z);
        let got = ground_target(
            seed,
            &haven,
            &cols,
            &mut occ,
            (x, y + 1.6, z + 1.0),
            (x, z),
            &[],
        );
        assert_eq!(got.map(|t| t.aim), Some(PlantAim::No("ON THE ROAD")));
        // The beach (the spawn zone) is no soft ground, as the sim says.
        let (x, z) = find(&mut |x, z| {
            let y = sim_core::terrain::ground(seed, &haven, x, z);
            y >= LAND_MIN_H
                && road_band(seed, &haven, x, z) == RoadBand::Off
                && sim_core::terrain::biome_at(seed, y, x, z) == sim_core::terrain::Biome::Beach
                && matches!(
                    ground_site(seed, &haven, &cols, &mut occ, x, z, || false),
                    Err(Ground::Barren(_))
                )
        });
        let y = sim_core::terrain::ground(seed, &haven, x, z);
        let got = ground_target(
            seed,
            &haven,
            &cols,
            &mut occ,
            (x, y + 1.6, z + 1.0),
            (x, z),
            &[],
        );
        assert_eq!(got.map(|t| t.aim), Some(PlantAim::No("TOO SANDY")));
    }

    /// A bed plant is in sight when its planter is, and never without one;
    /// a ground plant when the point it lies at is.
    #[test]
    fn a_crop_is_offered_only_when_the_hand_gets_at_it() {
        let bed = plant_at(2, 1.0, 2.0, 3.0);
        let rec = planter(0);
        let seen = |_: &DeployRec| true;
        let walled = |_: &DeployRec| false;
        let never = |_: (i32, i32, i32)| panic!("a bed plant asked the lying test");
        assert!(plant_in_sight(&bed, Some(&rec), seen, never));
        assert!(!plant_in_sight(&bed, Some(&rec), walled, never));
        assert!(!plant_in_sight(&bed, None, seen, never));
        let ground = WirePlant {
            spot: sim_core::farm::SPOT_GROUND,
            ..bed
        };
        let q = (bed.qx, bed.qy, bed.qz);
        let no_deploy = |_: &DeployRec| panic!("a ground plant asked its planter");
        assert!(plant_in_sight(&ground, None, no_deploy, |at| at == q));
        assert!(!plant_in_sight(&ground, None, no_deploy, |_| false));
    }
}
