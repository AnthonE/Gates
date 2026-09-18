//! Road coverage is independent of biome splats and stays on the sim's bands.
#![cfg(feature = "render")]

use bevy::mesh::{Mesh, VertexAttributeValues};
use client::render::{ground_splat, terrain_mesh};
use sim_core::terrain::{self, RoadBand};

fn masks(mesh: &Mesh) -> &[[f32; 2]] {
    let Some(VertexAttributeValues::Float32x2(values)) =
        mesh.attribute(ground_splat::ATTRIBUTE_ROAD)
    else {
        panic!("ground mesh lost its independent road coverage");
    };
    values
}

#[test]
fn junctions_keep_full_pavement_and_never_double_cover() {
    let bands = [RoadBand::Off, RoadBand::Shoulder, RoadBand::Carriageway];
    for ring in bands {
        for side in bands {
            let [pavement, dirt] = ground_splat::road_coverage(ring, side);
            assert!((0.0..=1.0).contains(&pavement));
            assert!((0.0..=1.0).contains(&dirt));
            assert!(pavement + dirt <= 1.0);
            if ring == RoadBand::Carriageway {
                assert_eq!([pavement, dirt], [1.0, 0.0]);
            } else {
                assert_eq!(pavement, 0.0, "a shoulder is not paved");
            }
        }
    }
    assert_eq!(
        ground_splat::road_coverage(RoadBand::Off, RoadBand::Off),
        [0.0; 2]
    );
    assert_eq!(
        ground_splat::road_coverage(RoadBand::Off, RoadBand::Shoulder),
        [0.0, terrain::ROAD_WEAR_SHOULDER],
    );
}

#[test]
fn real_junction_mesh_carries_both_surfaces_only_on_their_bands() {
    let seed = 20260731;
    let haven = terrain::haven(seed);
    let road = haven.roads.iter().find(|r| r.live).expect("live side road");
    let n = terrain_mesh::NEAR_N;
    let ox = road.rx - terrain_mesh::CHUNK_M * 0.5;
    let oz = road.rz - terrain_mesh::CHUNK_M * 0.5;
    let mesh = terrain_mesh::heightfield(seed, &haven, ox, oz, n, 1.0, 0.0);
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("positions");
    };
    assert_eq!(masks(&mesh).len(), positions.len());
    let (mut paved, mut dirt, mut clear) = (0, 0, 0);
    for (&[x, _, z], &[p, d]) in positions.iter().zip(masks(&mesh)) {
        let ring = terrain::ring_band(&haven.ring, x, z);
        let side = terrain::side_band(&haven, x, z);
        if p > 0.0 {
            assert_eq!(ring, RoadBand::Carriageway);
            paved += 1;
        }
        if d > 0.0 {
            assert_ne!(side, RoadBand::Off);
            assert_ne!(ring, RoadBand::Carriageway);
            dirt += 1;
        }
        if ring == RoadBand::Off && side == RoadBand::Off {
            assert_eq!([p, d], [0.0; 2]);
            clear += 1;
        }
    }
    assert!(
        paved > 0 && dirt > 0 && clear > 0,
        "fixture must cross all three surfaces"
    );
    let far = terrain_mesh::heightfield(seed, &haven, ox, oz, n, terrain_mesh::FAR_STEP, 0.0);
    assert!(
        masks(&far).iter().all(|m| *m == [0.0; 2]),
        "coarse lattice must not alias a narrow road"
    );
}

#[test]
fn pavement_aggregate_uses_its_registered_physical_size() {
    let p = ground_splat::GroundSplatParams::new();
    let repeats =
        ground_splat::ROAD_AGGREGATE_TILE_M * terrain_mesh::UV_PER_M * p.tile.w * p.pavement.w;
    assert!((repeats - 1.0).abs() < f32::EPSILON);
}

#[test]
fn surface_fades_before_any_near_ring_edge_can_expose_the_coarse_mesh() {
    // Sweep camera positions within its centre chunk, independently deriving
    // the four outer edges the streamer keeps resident.
    let chunk = terrain_mesh::CHUNK_M;
    let radius = terrain_mesh::NEAR_RADIUS as f32;
    let fade = ground_splat::GroundSplatParams::new().road_lod;
    for sub in 0..=64 {
        let camera = chunk * sub as f32 / 64.0;
        let negative_edge_distance = camera + radius * chunk;
        let positive_edge_distance = (radius + 1.0) * chunk - camera;
        assert!(fade.y <= negative_edge_distance);
        assert!(fade.y <= positive_edge_distance);
    }
    assert!(fade.x > 0.0);
    assert_eq!(fade.y - fade.x, chunk);
}
