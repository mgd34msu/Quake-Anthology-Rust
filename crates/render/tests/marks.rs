#[path = "support/world_surfaces.rs"]
mod world_surfaces;
use qa_core::primitives::{Bounds, IndexSpan, Plane, Vec3};
use qa_render::{
    Assets, Vertex,
    assets::MaterialSettings,
    marks::MarkProjection,
    world::{
        SurfaceMaterial, WorldView,
        geometry::{GeometryKind, WorldModel, WorldVertex},
    },
};
use qa_world::visibility::{PvsRows, VisLeaf, VisibilityWorld};

fn planar(count: usize) -> qa_render::world::geometry::WorldGeometry {
    let (mut geometry, _) = world_surfaces::inputs(count);
    for surface in &mut geometry.surfaces {
        surface.kind = GeometryKind::Polygon;
        surface.plane = Some(Plane::oriented(Vec3([-1., 0., 0.]), -1.));
        surface.source_flags = 0x30; // Legacy bits cannot become Q3 shader filters.
    }
    geometry.models.push(WorldModel {
        bounds: geometry.surfaces[0].bounds,
        origin: Vec3::default(),
        surfaces: IndexSpan {
            first: 0,
            count: count as u32,
        },
    });
    geometry
}
fn visibility(surfaces: usize, references: Vec<u32>) -> VisibilityWorld {
    VisibilityWorld::load(
        vec![],
        vec![],
        vec![VisLeaf {
            selector: None,
            area: None,
            solid: false,
            bounds: Bounds {
                mins: Vec3([-100.; 3]),
                maxs: Vec3([100.; 3]),
            },
            surfaces: IndexSpan {
                first: 0,
                count: references.len() as u32,
            },
        }],
        references,
        surfaces,
        -1,
        PvsRows::all_visible(0),
    )
    .unwrap()
}
fn projection(radius: f32) -> MarkProjection {
    let points = [
        [0., -radius, -radius],
        [0., radius, -radius],
        [0., radius, radius],
        [0., -radius, radius],
    ]
    .map(Vec3);
    MarkProjection::prepare(4, Vec3([2., 0., 0.]), |index| Ok::<_, ()>(points[index])).unwrap()
}
fn query(
    assets: &Assets,
    world: qa_render::world::WorldId,
    recipe: usize,
    projection: &MarkProjection,
    max_points: usize,
    max_fragments: usize,
) -> Vec<Vec<Vec3>> {
    let world = assets.world(world).unwrap();
    let mut view = WorldView::load(world);
    let mut output = vec![];
    let mut first = 0;
    let count = view
        .mark_fragments(
            world,
            assets
                .world_presentation(world.presentations()[recipe])
                .unwrap(),
            assets,
            projection,
            max_points,
            max_fragments,
            &mut |span, points| {
                assert_eq!(
                    span,
                    IndexSpan {
                        first,
                        count: points.len() as u32
                    }
                );
                first += span.count;
                output.push(points.to_vec());
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    assert_eq!(count, output.len());
    output
}
#[test]
fn selected_compiled_flags_and_world_membership_filter_shared_leaf_references() {
    let mut assets = Assets::load().unwrap();
    let blocked = assets
        .register_material(
            "blocked",
            &[Default::default()],
            MaterialSettings {
                surface_flags: 0x20,
                ..Default::default()
            },
        )
        .unwrap();
    let mut geometry = planar(2);
    geometry.models[0].surfaces.count = 1;
    geometry.models.push(WorldModel {
        bounds: geometry.surfaces[1].bounds,
        origin: Vec3::default(),
        surfaces: IndexSpan { first: 1, count: 1 },
    });
    let open = [SurfaceMaterial::default(); 2];
    let hidden = [SurfaceMaterial {
        material: blocked,
        ..Default::default()
    }; 2];
    let world = assets
        .register_world(geometry, visibility(2, vec![1, 0, 0, 1]), &[&open, &hidden])
        .unwrap();
    let marks = query(&assets, world, 0, &projection(2.), 64, 8);
    assert_eq!(
        marks,
        [vec![
            Vec3([1., -1., -1.]),
            Vec3([1., 1., -1.]),
            Vec3([1., 0., 1.])
        ]]
    );
    assert!(query(&assets, world, 1, &projection(2.), 64, 8).is_empty());
}
fn small_last(geometry: &mut qa_render::world::geometry::WorldGeometry, surface: usize) {
    let first = geometry.vertices.len() as u32;
    for vertex in geometry.vertices[..3].to_vec() {
        let mut vertex = vertex;
        vertex.vertex.position.0[1] *= 0.1;
        vertex.vertex.position.0[2] *= 0.1;
        geometry.vertices.push(vertex);
    }
    let index = geometry.indices.len() as u32;
    geometry.indices.extend([first, first + 1, first + 2]);
    geometry.surfaces[surface].indices = IndexSpan {
        first: index,
        count: 3,
    };
}
#[test]
fn point_budget_skips_whole_fragment_but_candidate_cap_keeps_ordered_prefix() {
    let mut assets = Assets::load().unwrap();
    let blocked = assets
        .register_material(
            "noimpact",
            &[Default::default()],
            MaterialSettings {
                surface_flags: 0x10,
                ..Default::default()
            },
        )
        .unwrap();
    let mut geometry = planar(65);
    small_last(&mut geometry, 64);
    for vertex in &mut geometry.vertices[..3] {
        vertex.vertex.position.0[1] *= 4.0;
        vertex.vertex.position.0[2] *= 4.0;
    }
    let open = vec![SurfaceMaterial::default(); 65];
    let mut skip_first = open.clone();
    skip_first[0].material = blocked;
    let world = assets
        .register_world(
            geometry,
            visibility(65, (0..65).collect()),
            &[&open, &skip_first],
        )
        .unwrap();
    assert!(query(&assets, world, 0, &projection(0.75), 3, 8).is_empty());
    let marks = query(&assets, world, 1, &projection(0.75), 3, 8);
    assert_eq!(marks.len(), 1);
    assert_eq!(
        marks[0],
        [
            Vec3([1., -0.1, -0.1]),
            Vec3([1., 0.1, -0.1]),
            Vec3([1., 0., 0.1])
        ]
    );
    assert_eq!(query(&assets, world, 1, &projection(2.), 64, 1).len(), 1);
}
#[test]
fn retained_patch_triangles_are_projected_in_full_grid_order_and_soups_are_ignored() {
    let mut geometry = planar(2);
    geometry.vertices = [[1., -1., -1.], [1., -1., 1.], [1., 1., -1.], [1., 1., 1.]]
        .map(|position| WorldVertex {
            vertex: Vertex {
                position: Vec3(position),
                normal: Vec3([-1., 0., 0.]),
                ..Default::default()
            },
        })
        .into();
    geometry.indices = vec![0, 2, 1, 1, 2, 3];
    geometry.surfaces[0].kind = GeometryKind::Patch;
    geometry.surfaces[0].plane = None;
    geometry.surfaces[0].vertices.count = 4;
    geometry.surfaces[0].indices = IndexSpan { first: 0, count: 6 };
    geometry.surfaces[1].kind = GeometryKind::Triangles;
    let mut assets = Assets::load().unwrap();
    let world = assets
        .register_world(
            geometry,
            visibility(2, vec![0, 1]),
            &[&[SurfaceMaterial::default(); 2]],
        )
        .unwrap();
    let marks = query(&assets, world, 0, &projection(2.), 64, 8);
    assert_eq!(
        marks,
        [
            vec![
                Vec3([1., -1., -1.]),
                Vec3([1., 1., -1.]),
                Vec3([1., -1., 1.])
            ],
            vec![Vec3([1., -1., 1.]), Vec3([1., 1., -1.]), Vec3([1., 1., 1.])]
        ]
    );
}

#[test]
fn native_clip_emits_current_first_and_drops_all_on_the_epsilon_band() {
    let mut geometry = planar(1);
    for (vertex, position) in
        geometry
            .vertices
            .iter_mut()
            .zip([[1., -1., 0.], [1., 0., 1.], [1., 1., 0.]])
    {
        vertex.vertex.position = Vec3(position);
    }
    let points = [[0., 0., -2.], [0., 2., -2.], [0., 2., 2.], [0., 0., 2.]].map(Vec3);
    let projection =
        MarkProjection::prepare(4, Vec3([2., 0., 0.]), |index| Ok::<_, ()>(points[index])).unwrap();
    let mut assets = Assets::load().unwrap();
    let world = assets
        .register_world(
            geometry,
            visibility(1, vec![0]),
            &[&[SurfaceMaterial::default()]],
        )
        .unwrap();
    assert_eq!(
        query(&assets, world, 0, &projection, 64, 8),
        [vec![
            Vec3([1., 0., 1.]),
            Vec3([1., 1., 0.]),
            Vec3([1., 0., 0.]),
        ]]
    );

    let mut geometry = planar(1);
    for (vertex, position) in
        geometry
            .vertices
            .iter_mut()
            .zip([[1., 0.49, 0.], [1., 0.5, 1.], [1., 0.48, 0.]])
    {
        vertex.vertex.position = Vec3(position);
    }
    let world = assets
        .register_world(
            geometry,
            visibility(1, vec![0]),
            &[&[SurfaceMaterial::default()]],
        )
        .unwrap();
    assert!(query(&assets, world, 0, &projection, 64, 8).is_empty());
}
