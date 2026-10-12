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
    render_presentation(assets, entity, qa_render::CpuPresentation::Rgb)
}

fn render_presentation(
    assets: &Assets,
    entity: SceneEntity,
    presentation: qa_render::CpuPresentation,
) -> (Vec<u32>, Vec<u32>, qa_render::BackendStats) {
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
            cpu_presentation: presentation,
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

fn software_palette() -> qa_render::surface_cache::PaletteLighting {
    let colors: Vec<_> = (0..=255u8)
        .flat_map(|value| [value, 255 - value, value])
        .collect();
    let shades: Vec<_> = (0..64u8)
        .flat_map(|shade| (0..=255u8).map(move |index| index.wrapping_add(shade)))
        .collect();
    qa_render::surface_cache::PaletteLighting::load(&colors, &shades, None, 224).unwrap()
}

#[test]
fn indexed_animated_models_use_selected_positions_current_normals_and_native_colormap() {
    let mut assets = Assets::load().unwrap();
    let palette = assets.register_palette(software_palette()).unwrap();
    let image = assets
        .register_indexed_image(
            qa_render::surface_cache::IndexedTexture::load_base(1, 1, &[9], None).unwrap(),
            palette,
        )
        .unwrap();
    let material = assets
        .register_material(
            "indexed alias",
            &[Stage {
                texture: qa_render::assets::StageTexture::Image(image),
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
    let vertices: Vec<_> = previous.into_iter().chain(current).collect();
    let indices = [0, 1, 2, 0, 2, 3];
    let model = assets
        .register_animated_model(&vertices, 4, &indices, material)
        .unwrap();
    for lighting in [
        qa_render::surface_cache::IndexedLighting::Gray,
        qa_render::surface_cache::IndexedLighting::BrightestRgb,
    ] {
        let presentation = qa_render::CpuPresentation::Indexed {
            palette,
            lighting,
            ambient: 0,
            fullbright: false,
        };
        for back in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let expected = std::array::from_fn::<_, 4, _>(|index| Vertex {
                normal: current[index].normal,
                ..reference(current[index], previous[index], back, false)
            });
            let fixed = assets
                .register_model(&expected, &indices, material)
                .unwrap();
            let entity = SceneEntity {
                model: model.into(),
                frame: 1,
                old_frame: 0,
                back_lerp: back,
                lighting: Some(EntityLighting {
                    ambient: [80.0; 3],
                    directed: [48.0; 3],
                    direction: Vec3([-1.0, 0.0, 0.0]),
                }),
                ..SceneEntity::default()
            };
            let actual = render_presentation(&assets, entity, presentation);
            let explicit = render_presentation(
                &assets,
                SceneEntity {
                    model: fixed.into(),
                    frame: 0,
                    old_frame: 0,
                    back_lerp: 0.0,
                    ..entity
                },
                presentation,
            );
            assert_eq!(actual, explicit);
            assert_eq!(actual.2.rejected, 0);
            assert!(actual.2.triangles > 0);
            let expected_color = assets.palette(palette).unwrap().color(45);
            assert!(actual.0.contains(&expected_color));
            assert!(actual.0.iter().all(
                |&pixel| pixel == expected_color || pixel == u32::from_le_bytes([3, 5, 7, 255])
            ));
        }
    }
}

#[test]
fn indexed_polygons_keep_native_indices_cutouts_and_final_palette_phase() {
    let mut assets = Assets::load().unwrap();
    let palette = assets.register_palette(software_palette()).unwrap();
    let image = assets
        .register_indexed_image(
            qa_render::surface_cache::IndexedTexture::load_base(2, 1, &[255, 9], Some(255))
                .unwrap(),
            palette,
        )
        .unwrap();
    let material = assets
        .register_material(
            "indexed fence",
            &[Stage {
                texture: qa_render::assets::StageTexture::Image(image),
                ..Stage::default()
            }],
            MaterialSettings {
                cull: Cull::None,
                ..MaterialSettings::default()
            },
        )
        .unwrap();
    let vertices = [
        (Vec3([8.0, 8.0, 8.0]), [0.0, 0.0]),
        (Vec3([8.0, -8.0, 8.0]), [1.0, 0.0]),
        (Vec3([8.0, -8.0, -8.0]), [1.0, 1.0]),
        (Vec3([8.0, 8.0, -8.0]), [0.0, 1.0]),
    ]
    .map(|(position, texcoord)| Vertex {
        position,
        texcoord,
        ..Vertex::default()
    });
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut cpu = CpuBackend::load(4, 4).unwrap();
    let background = [3, 5, 7, 255];
    for shifted in [false, true] {
        let mut frame = frontend.begin_frame(background).unwrap();
        assert!(frame.add_poly(material, &vertices));
        let mut transform = qa_render::PaletteTransform::default();
        transform.shifts[0] = qa_render::PaletteShift {
            destination: [255, 0, 0],
            percent: 128,
        };
        assert!(frame.render_scene(
            Refdef {
                viewport: Viewport {
                    width: 4,
                    height: 4,
                    ..Viewport::default()
                },
                fov: [90.0; 2],
                cpu_presentation: qa_render::CpuPresentation::Indexed {
                    palette,
                    lighting: qa_render::surface_cache::IndexedLighting::Gray,
                    ambient: 0,
                    fullbright: false
                },
                blend_phase: qa_render::BlendPhase::FinalPalette,
                palette_transform: shifted.then_some(transform),
                ..Refdef::default()
            },
            &[],
            &assets
        ));
        let packet = frame.finish();
        assert_eq!(cpu.render(&packet, &assets).rejected, 0);
        let mut color = assets.palette(palette).unwrap().color(9).to_le_bytes();
        if shifted {
            for (channel, destination) in color[..3].iter_mut().zip([255, 0, 0]) {
                let original = i32::from(*channel);
                *channel = (original + ((destination - original) * 128 >> 8)) as u8;
            }
        }
        for row in cpu.pixels().chunks_exact(4) {
            assert_eq!(&row[..2], &[u32::from_le_bytes(background); 2]);
            assert_eq!(&row[2..], &[u32::from_le_bytes(color); 2]);
        }
        assert!(frontend.recycle(packet).is_ok());
    }
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
            model: animated.into(),
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
                model: fixed.into(),
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
            model: ModelId(u32::MAX).into(),
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
