//! Paint coordinates must survive the actual terrain triangulation.
#![cfg(feature = "render")]

use bevy::mesh::{Indices, Mesh, VertexAttributeValues};
use client::render::{ground_splat as material, road_markings as paint, terrain_mesh};
use sim_core::terrain::{self, RoadBand};

fn coords(mesh: &Mesh) -> &[[f32; 4]] {
    let Some(VertexAttributeValues::Float32x4(v)) = mesh.attribute(material::ATTRIBUTE_MARKINGS)
    else {
        panic!("marking coordinates");
    };
    v
}

fn positions(mesh: &Mesh) -> &[[f32; 3]] {
    let Some(VertexAttributeValues::Float32x3(v)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        panic!("positions");
    };
    v
}

#[test]
fn triangle_validity_prevents_false_stripes_at_real_junctions() {
    let mut guarded_marks = 0;
    let mut unguarded_violations = 0;
    for (seed, ox, oz) in [(20260731, 1146.0, 173.0), (42, 844.0, 248.0)] {
        let haven = terrain::haven(seed);
        let chart = paint::RoadChart::build(seed, 1.0);
        let mut mesh = terrain_mesh::heightfield(seed, &haven, ox, oz, 13, 1.0, 0.0);
        terrain_mesh::apply_road_markings(&mut mesh, &chart, &haven);
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("indices");
        };
        let Some(VertexAttributeValues::Float32x2(roads)) =
            mesh.attribute(material::ATTRIBUTE_ROAD)
        else {
            panic!("roads");
        };
        for triangle in indices.chunks_exact(3) {
            for a in 0..=8 {
                for b in 0..=8 - a {
                    let weights = [a as f32 / 8.0, b as f32 / 8.0, (8 - a - b) as f32 / 8.0];
                    let mut m = [0.0; 4];
                    let mut p = [0.0; 3];
                    let mut pavement = 0.0;
                    for (&index, &w) in triangle.iter().zip(&weights) {
                        for (value, coordinate) in m.iter_mut().zip(coords(&mesh)[index as usize]) {
                            *value += coordinate * w;
                        }
                        for (value, coordinate) in
                            p.iter_mut().zip(positions(&mesh)[index as usize])
                        {
                            *value += coordinate * w;
                        }
                        pavement += roads[index as usize][0] * w;
                    }
                    let stripe = m[0].abs() <= paint::ROAD_PAINT_WIDTH_M * 0.5
                        || (m[0].abs() - paint::ROAD_EDGE_OFFSET_M).abs()
                            <= paint::ROAD_PAINT_WIDTH_M * 0.5;
                    if !stripe || pavement < 0.5 {
                        continue;
                    }
                    let on_road = terrain::ring_band(seed, p[0], p[2]) == RoadBand::Carriageway;
                    if m[3] >= 1.0 - material::ROAD_PAINT_VALID_EPS {
                        assert!(on_road, "paint escaped at seed={seed}, {},{}", p[0], p[2]);
                        guarded_marks += 1;
                    } else if !on_road {
                        // Control: removing validity must manufacture an off-road
                        // stripe in these fixtures even with pavement clipping.
                        unguarded_violations += 1;
                    }
                }
            }
        }
    }
    assert!(
        guarded_marks > 0,
        "fixture must actually exercise visible paint"
    );
    assert!(
        unguarded_violations > 0,
        "fixture must detect sentinel interpolation"
    );
}

#[test]
fn adjacent_chunks_share_exact_coordinates_across_the_phase_seam() {
    let seed = 20260731;
    let haven = terrain::haven(seed);
    let chart = paint::RoadChart::build(seed, 1.0);
    let mut a = terrain_mesh::heightfield(seed, &haven, 1856.0, 1020.0, 5, 1.0, 0.0);
    let mut b = terrain_mesh::heightfield(seed, &haven, 1856.0, 1024.0, 5, 1.0, 0.0);
    terrain_mesh::apply_road_markings(&mut a, &chart, &haven);
    terrain_mesh::apply_road_markings(&mut b, &chart, &haven);
    let mut live = 0;
    for k in 0..5 {
        assert_eq!(coords(&a)[20 + k], coords(&b)[k]);
        if coords(&b)[k][3] > 0.0 {
            live += 1;
        }
    }
    assert!(live > 0, "phase-seam fixture must carry markings");
}

#[test]
fn branches_and_coarse_terrain_never_receive_markings() {
    let seed = 20260731;
    let haven = terrain::haven(seed);
    let chart = paint::RoadChart::build(seed, 1.0);
    let road = haven.roads.iter().find(|r| r.live).unwrap();
    let mut near =
        terrain_mesh::heightfield(seed, &haven, road.rx - 16.0, road.rz - 16.0, 33, 1.0, 0.0);
    terrain_mesh::apply_road_markings(&mut near, &chart, &haven);
    let mut branch_vertices = 0;
    for (p, m) in positions(&near).iter().zip(coords(&near)) {
        if terrain::side_band(&haven, p[0], p[2]) != RoadBand::Off {
            assert_eq!(m[3], 0.0);
            branch_vertices += 1;
        }
    }
    assert!(branch_vertices > 0);
    let far = terrain_mesh::heightfield(
        seed,
        &haven,
        road.rx - 16.0,
        road.rz - 16.0,
        9,
        terrain_mesh::FAR_STEP,
        0.0,
    );
    assert!(coords(&far).iter().all(|m| *m == [0.0; 4]));
}
