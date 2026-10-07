use qa_core::primitives::{Plane, Vec3};
use qa_world::collision::brushes::{Brush, BrushMap};
use qa_world::collision::{CollisionWorld, Contents};

fn box_map() -> BrushMap {
    let planes = (0..6)
        .map(|index| {
            let mut normal = [0.0; 3];
            normal[index / 2] = if index % 2 == 0 { 1.0 } else { -1.0 };
            Plane {
                normal: Vec3(normal),
                distance: 10.0,
                axis: None,
            }
        })
        .collect();
    BrushMap::load(
        planes,
        vec![Brush {
            first_plane: 0,
            plane_count: 6,
            contents: Contents::SOLID,
        }],
    )
    .unwrap()
}

#[test]
fn player_bounds_trace_against_both_brush_formats_without_family_tags() {
    for world in [
        CollisionWorld::Q2Brushes(box_map()),
        CollisionWorld::Q3Brushes(box_map()),
    ] {
        let epsilon = if matches!(&world, CollisionWorld::Q2Brushes(_)) {
            0.03125
        } else {
            0.125
        };
        for half_width in [16.0, 16.0, 15.0] {
            let trace = world.trace(
                Vec3([100.0, 0.0, 0.0]),
                Vec3::default(),
                Vec3([-half_width, -half_width, -24.0]),
                Vec3([half_width, half_width, 32.0]),
                Contents::SOLID,
            );
            assert_eq!(
                trace.fraction,
                (100.0 - 10.0 - half_width - epsilon) / 100.0
            );
            assert_eq!(trace.plane.normal, Vec3([1.0, 0.0, 0.0]));
            assert_eq!(trace.contents, Contents::SOLID);
            assert!(!trace.start_solid);
        }
        assert_eq!(world.point_contents(Vec3::default()), Contents::SOLID);
        assert_eq!(
            world.point_contents(Vec3([20.0, 0.0, 0.0])),
            Contents::EMPTY
        );
        assert_eq!(
            world
                .trace(
                    Vec3([100.0, 0.0, 0.0]),
                    Vec3::default(),
                    Vec3::default(),
                    Vec3::default(),
                    Contents::WATER
                )
                .fraction,
            1.0
        );
    }
}

#[test]
fn embedded_motion_keeps_original_brush_trace_semantics() {
    for (world, fraction) in [
        (CollisionWorld::Q2Brushes(box_map()), 1.0),
        (CollisionWorld::Q3Brushes(box_map()), 0.0),
    ] {
        let trace = world.trace(
            Vec3::default(),
            Vec3([1.0, 0.0, 0.0]),
            Vec3::default(),
            Vec3::default(),
            Contents::SOLID,
        );
        assert!(trace.start_solid && trace.all_solid);
        assert_eq!(trace.fraction, fraction);
        let still = world.trace(
            Vec3::default(),
            Vec3::default(),
            Vec3::default(),
            Vec3::default(),
            Contents::SOLID,
        );
        assert_eq!(still.fraction, 0.0);
    }
}
