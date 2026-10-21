//! Farming v2 on the client, the pure half: which bed spot a seed in hand
//! would go into, which planter a bed plant stands in, and whether the hand
//! holds water. Bevy-free, so the arithmetic is tested headless; the ghost
//! and the click are `render::farm_ghost`'s.

use protocol::WirePlant;
use sim_core::deploy::{DeployContent, DeployRec, ARCH_PLANTER, DEPLOY_VOL};
use sim_core::farm::{spot_xz, BED_SOIL_DROP_M, PLANTER_SPOTS};

/// How far from a spot's centre the crosshair may land on the soil and
/// still name that spot, metres.
pub const SPOT_PICK_M: f32 = 0.35;
/// A plant this close to a spot's centre (xz) is the one in it, metres:
/// wider than a position quantum, narrower than the spots' pitch.
pub const SPOT_TAKEN_M: f32 = 0.05;

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
            let (px, _, pz) = p.pos();
            p.in_bed() && (px - sx).powi(2) + (pz - sz).powi(2) <= SPOT_TAKEN_M * SPOT_TAKEN_M
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
/// one of whose spot centres is where the plant is (the wire record names
/// its spot, not its host). `have` is `ClientCore::deploy_defs_have`.
pub fn planter_of<'a>(
    p: &WirePlant,
    deploys: &'a [DeployRec],
    defs: &DeployContent,
    have: u16,
) -> Option<&'a DeployRec> {
    if !p.in_bed() {
        return None;
    }
    let (px, _, pz) = p.pos();
    deploys.iter().find(|d| {
        (d.row as u16) < have && defs.defs[d.row as usize].arch == ARCH_PLANTER && {
            let (sx, sz) = spot_xz(d.cx, d.cz, d.pose, p.spot);
            (px - sx).powi(2) + (pz - sz).powi(2) <= SPOT_TAKEN_M * SPOT_TAKEN_M
        }
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

/// Whether a stack is a full waterskin: at a planter, the full skin is the
/// only fuel (`CookContent::roles`, `ROLE_FUEL` is `water_full` alone).
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
        let p = plant_at(3, sx, 10.0, sz);
        let deploys = [a, b];
        assert_eq!(planter_of(&p, &deploys, &defs, 1).map(|d| d.cx), Some(101));
        assert!(
            planter_of(&p, &deploys, &defs, 0).is_none(),
            "an undripped row"
        );
        let ground = WirePlant {
            spot: sim_core::farm::SPOT_GROUND,
            ..p
        };
        assert!(planter_of(&ground, &deploys, &defs, 1).is_none());
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
