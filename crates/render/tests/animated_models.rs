use qa_core::primitives::Vec3;
use qa_render::{
    Assets, FrontEnd, Limits, MaterialId, ModelId, Refdef, SceneEntity, Vertex, Viewport,
    assets::{Cull, MaterialSettings, Stage},
    cpu::CpuBackend,
    shader::RgbGen,
    stage::EntityLighting,
};

fn vertex(position: [f32; 3], normal: [f32; 3]) -> Vertex {
    Vertex {
        position: Vec3(position),
        normal: Vec3(normal),
        texcoord: [0.25, 0.75],
        lightmap_coord: [0.5, 0.125],
        color: [71, 139, 211, 255],
    }
}

// Q3 tr_surface.c LerpMeshVertexes/VectorArrayNormalize, on decoded vertices.
// Keep this scalar oracle independent of ModelFrames and core vector helpers.
fn reference(current: Vertex, previous: Vertex, back_lerp: f32, same_frame: bool) -> Vertex {
    if same_frame || back_lerp == 0.0 {
        return current;
    }
    let front_lerp = 1.0 - back_lerp;
    let mut output = current;
    for axis in 0..3 {
        output.position.0[axis] =
            previous.position.0[axis] * back_lerp + current.position.0[axis] * front_lerp;
        output.normal.0[axis] =
            previous.normal.0[axis] * back_lerp + current.normal.0[axis] * front_lerp;
    }
    let [x, y, z] = output.normal.0;
    let squared = x * x + y * y + z * z;
    let estimate = f32::from_bits(0x5f3759df - (squared.to_bits() >> 1));
    let scale = estimate * (1.5 - squared * 0.5 * estimate * estimate);
    for axis in 0..3 {
        output.normal.0[axis] *= scale;
    }
    output
}

fn assert_vertex_bits(actual: Vertex, expected: Vertex) {
    assert_eq!(
        actual.position.0.map(f32::to_bits),
        expected.position.0.map(f32::to_bits)
    );
    assert_eq!(
        actual.normal.0.map(f32::to_bits),
        expected.normal.0.map(f32::to_bits)
    );
    assert_eq!(
        actual.texcoord.map(f32::to_bits),
        expected.texcoord.map(f32::to_bits)
    );
    assert_eq!(
        actual.lightmap_coord.map(f32::to_bits),
        expected.lightmap_coord.map(f32::to_bits)
    );
    assert_eq!(actual.color, expected.color);
}

#[test]
fn registration_rejects_malformed_frame_tables_without_consuming_handles() {
    let mut assets = Assets::load().unwrap();
    let vertices = [vertex([8.0, 1.0, 2.0], [0.0, 0.0, 1.0]); 6];
    let count = assets.models().len();
    for (data, frame_size, indices, material) in [
        (&vertices[..], 0, &[0, 1, 2][..], MaterialId(0)),
        (&vertices[..5], 3, &[0, 1, 2][..], MaterialId(0)),
        (&vertices[..], 7, &[0, 1, 2][..], MaterialId(0)),
        (&vertices[..0], 3, &[][..], MaterialId(0)),
        (&vertices[..], 3, &[0, 1, 3][..], MaterialId(0)),
        (&vertices[..], 3, &[0, 1][..], MaterialId(0)),
        (&vertices[..], 3, &[0, 1, 2][..], MaterialId(u32::MAX)),
    ] {
        assert!(
            assets
                .register_animated_model(data, frame_size, indices, material)
                .is_err()
        );
        assert_eq!(assets.models().len(), count);
    }
    let mut nonfinite = vertices;
    nonfinite[5].normal.0[2] = f32::NAN;
    assert!(
        assets
            .register_animated_model(&nonfinite, 3, &[0, 1, 2], MaterialId(0))
            .is_err()
    );
    let id = assets
        .register_animated_model(&vertices, 3, &[0, 1, 2], MaterialId(0))
        .unwrap();
    assert_eq!(id.0 as usize, count);
    assert_eq!(assets.model(id).unwrap().frame_count(), 2);
    assert_eq!(assets.model(id).unwrap().vertices_per_frame, 3);
    let empty = assets.register_model(&[], &[], MaterialId(0)).unwrap();
    assert_eq!(assets.model(empty).unwrap().frame_count(), 1);
    assert!(
        assets
            .model(empty)
            .unwrap()
            .frames(0, 0, 0.0)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn frame_sampling_matches_native_weighted_order_and_retains_current_attributes() {
    let current = vertex([8.125, -3.75, 1.125], [0.0, 0.6, 0.8]);
    let previous = Vertex {
        texcoord: [1.0, 1.0],
        color: [255, 0, 0, 255],
        ..vertex([16.5, 7.25, -4.875], [0.8, 0.0, 0.6])
    };
    let mut assets = Assets::load().unwrap();
    let id = assets
        .register_animated_model(&[previous, current], 1, &[], MaterialId(0))
        .unwrap();
    let model = assets.model(id).unwrap();
    for back in [0.0, 0.125, 0.3, 0.5, 0.875, 1.0] {
        assert_vertex_bits(
            model.frames(1, 0, back).unwrap().vertex(0),
            reference(current, previous, back, false),
        );
        assert_vertex_bits(
            model.frames(0, 1, back).unwrap().vertex(0),
            reference(previous, current, back, false),
        );
        assert_vertex_bits(model.frames(1, 1, back).unwrap().vertex(0), current);
    }
    for (frame, old_frame, back) in [
        (2, 0, 0.0),
        (0, 2, 0.0),
        (u32::MAX, 0, 0.0),
        (1, 0, -0.1),
        (1, 0, 1.1),
        (1, 0, f32::NAN),
        (1, 0, f32::INFINITY),
    ] {
        assert!(model.frames(frame, old_frame, back).is_none());
    }
    let signed_zero = vertex([-0.0, 0.0, -0.0], [-0.0, 0.0, 1.0]);
    let static_id = assets
        .register_model(&[signed_zero], &[], MaterialId(0))
        .unwrap();
    assert_vertex_bits(
        assets
            .model(static_id)
            .unwrap()
            .frames(0, 0, 1.0)
            .unwrap()
            .vertex(0),
        signed_zero,
    );
}

fn render(assets: &Assets, entity: SceneEntity) -> (Vec<u32>, Vec<u32>, qa_render::BackendStats) {
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    assert!(frame.add_entity(entity));
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                width: 32,
                height: 24,
                ..Viewport::default()
            },
            fov: [90.0, 75.0],
            ..Refdef::default()
        },
        &[],
        assets
    ));
    let mut cpu = CpuBackend::load(32, 24).unwrap();
    let stats = cpu.render(&frame.finish(), assets);
    (
        cpu.pixels().to_vec(),
        cpu.inverse_depth().iter().map(|v| v.to_bits()).collect(),
        stats,
    )
}

#[test]
fn cpu_animated_images_and_depth_match_explicit_interpolated_meshes() {
    let mut assets = Assets::load().unwrap();
    let material = assets
        .register_material(
            "animated diffuse",
            &[Stage {
                rgb_gen: RgbGen::LightingDiffuse,
                ..Stage::default()
            }],
            MaterialSettings {
                cull: Cull::None,
                ..MaterialSettings::default()
            },
        )
        .unwrap();
    let previous = [
        [8.0, 4.0, -3.0],
        [8.0, -4.0, -3.0],
        [8.0, -4.0, 3.0],
        [8.0, 4.0, 3.0],
    ]
    .map(|position| vertex(position, [0.0, 0.0, 1.0]));
    let current = [
        [10.0, 2.0, -4.0],
        [10.0, -6.0, -4.0],
        [10.0, -6.0, 2.0],
        [10.0, 2.0, 2.0],
    ]
    .map(|position| vertex(position, [0.6, 0.0, 0.8]));
    let frames: Vec<Vertex> = previous.into_iter().chain(current).collect();
    let indices = [0, 1, 2, 0, 2, 3];
    let animated = assets
        .register_animated_model(&frames, 4, &indices, material)
        .unwrap();
    for back in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let expected = std::array::from_fn::<_, 4, _>(|index| {
            reference(current[index], previous[index], back, false)
        });
        let fixed = assets
            .register_model(&expected, &indices, material)
            .unwrap();
        let entity = SceneEntity {
            model: animated,
            frame: 1,
            old_frame: 0,
            back_lerp: back,
            origin: Vec3([0.0, 0.25, -0.5]),
            lighting: Some(EntityLighting {
                ambient: [23.0, 31.0, 47.0],
                directed: [191.0, 173.0, 151.0],
                direction: Vec3([0.0, 0.0, 1.0]),
            }),
            ..SceneEntity::default()
        };
        let animated_image = render(&assets, entity);
        let fixed_image = render(
            &assets,
            SceneEntity {
                model: fixed,
                frame: 0,
                old_frame: 0,
                back_lerp: 0.0,
                ..entity
            },
        );
        assert_eq!(animated_image, fixed_image);
        assert_eq!(animated_image.2.rejected, 0);
        assert!(animated_image.2.triangles > 0);
        assert!(
            animated_image
                .0
                .iter()
                .any(|&pixel| pixel != u32::from_le_bytes([3, 5, 7, 255]))
        );
    }
}

#[test]
fn cpu_rejects_bad_frame_selection_once_before_drawing() {
    let assets = Assets::load().unwrap();
    for entity in [
        SceneEntity {
            frame: 1,
            ..SceneEntity::default()
        },
        SceneEntity {
            old_frame: 1,
            ..SceneEntity::default()
        },
        SceneEntity {
            back_lerp: f32::NAN,
            ..SceneEntity::default()
        },
        SceneEntity {
            model: ModelId(u32::MAX),
            ..SceneEntity::default()
        },
    ] {
        let (pixels, _, stats) = render(&assets, entity);
        assert_eq!(stats.rejected, 1);
        assert_eq!(stats.triangles, 0);
        assert_eq!(stats.stages, 0);
        assert!(
            pixels
                .iter()
                .all(|&pixel| pixel == u32::from_le_bytes([3, 5, 7, 255]))
        );
    }
}
