use client::ui::place::{aim_from_look, standing_level, Met};
use sim_core::build::*;
use sim_core::collide::ColIndex;
use sim_core::terrain;
#[test]
fn half_wall_tops_and_half_floors_resolve_to_their_real_socket() {
    let seed = 20260731;
    let haven = terrain::haven(seed);
    let (cx, cz) = (341, 341);
    let base = column_floor_y(seed, &haven, cx, cz, 0);
    let x = cx as f32 * BUILD_CELL_M;
    let z = cz as f32 * BUILD_CELL_M;
    for level in [0, 8, 1, 9] {
        let mut cols = ColIndex::new();
        cols.add(cx, cz, level, LOC_EDGE_XLO, SHAPE_HALF_WALL, 0);
        cols.add(cx, cz, level, LOC_PLANE, SHAPE_FLOOR, 0);
        let y = base + level_y(level);
        let aim = aim_from_look(
            seed,
            &haven,
            &cols,
            [x + 1.0, y + 1.3, z + 1.5],
            [-1.0, 0.0, 0.0],
            [x + 1.0, y, z + 1.5],
        );
        assert_eq!(aim.met, Met::HalfWall(level));
        assert_eq!(aim.level_for(SHAPE_FLOOR), level_step(level, 1).unwrap());
        assert_eq!(
            aim.level_for(SHAPE_HALF_WALL),
            level_step(level, 1).unwrap()
        );
        assert_eq!(aim.level_for_deploy(), level);
        assert_eq!(
            standing_level(seed, &haven, &cols, [x + 1.0, y, z + 1.5]),
            level
        );
    }
}
#[cfg(feature = "render")]
#[test]
fn short_wall_meshes_end_inside_the_floor_they_bear() {
    use bevy::mesh::VertexAttributeValues;
    use bevy::prelude::*;
    use client::render::structures::{base_transform, part_mesh, parts_for};
    let seed = 20260731;
    let haven = terrain::haven(seed);
    for shape in [SHAPE_HALF_WALL, SHAPE_LOW_WALL] {
        for loc in [LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_DIAG_A, LOC_DIAG_B] {
            for level in [0, 8, 2, 10] {
                let root = base_transform(seed, &haven, (341, 341, level, loc), 0);
                let base = column_floor_y(seed, &haven, 341, 341, 0) + level_y(level);
                let (parts, n) = parts_for(shape, loc);
                for part in &parts[..n] {
                    let mesh = part_mesh(part);
                    let VertexAttributeValues::Float32x3(points) =
                        mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
                    else {
                        panic!("positions");
                    };
                    for v in points {
                        let world = (root * part.transform()).transform_point(Vec3::from(*v));
                        assert!(world.y < base + wall_height(shape));
                        assert!(world.y > base - 0.2);
                    }
                }
            }
        }
    }
}
