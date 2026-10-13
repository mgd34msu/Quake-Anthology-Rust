use super::*;
use crate::scene::{SceneEntity, SceneModel};
use crate::shader::{AlphaGen, RgbGen};
use crate::surface_cache::{IndexedLighting, IndexedTexture};

fn instance(assets: &Assets, world: WorldId) -> SceneEntity {
    SceneEntity {
        model: SceneModel::World {
            world,
            presentation: only_presentation(assets, world),
            surfaces: IndexSpan { first: 0, count: 1 },
        },
        ..SceneEntity::default()
    }
}

pub(super) fn inline_models(geometry: &mut WorldGeometry) {
    let model = WorldModel {
        bounds: geometry.surfaces[0].bounds,
        origin: Vec3::default(),
        surfaces: IndexSpan {
            first: 0,
            count: geometry.surfaces.len() as u32,
        },
    };
    geometry.models = vec![model, model];
}

fn instance_packet(assets: &Assets, entities: &[SceneEntity], view: Refdef) -> CommandList {
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    for entity in entities {
        assert!(frame.add_entity(*entity));
    }
    assert!(frame.render_scene(view, &[], assets));
    frame.finish()
}

pub(super) fn assert_frame(actual: &CpuBackend, expected: &CpuBackend) {
    assert_eq!(actual.frame.pixels, expected.frame.pixels);
    assert_eq!(actual.frame.indices, expected.frame.indices);
    assert_eq!(actual.frame.palettes, expected.frame.palettes);
    assert_eq!(
        actual
            .frame
            .inverse_depth
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        expected
            .frame
            .inverse_depth
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
    );
}

#[test]
fn indexed_world_instances_retain_native_coordinates_and_share_cached_texels() {
    let mut assets = Assets::load().unwrap();
    let palette = indexed_palette(&mut assets);
    let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| {
        let size = 16 >> mip;
        (0..size * size).map(|i| (17 + i % 191) as u8).collect()
    });
    let image = assets
        .register_indexed_image(
            IndexedTexture::load(
                16,
                16,
                std::array::from_fn(|mip| mips[mip].as_slice()),
                false,
            )
            .unwrap(),
            palette,
        )
        .unwrap();
    let material = stages(
        &mut assets,
        &[Stage {
            texture: StageTexture::Image(image),
            ..Stage::default()
        }],
        MaterialSettings {
            cull: Cull::None,
            ..MaterialSettings::default()
        },
    );
    let worlds = [(4.0, 1.0), (8.0, 0.5)].map(|(depth, extent)| {
        fixture_world(
            &mut assets,
            depth,
            extent,
            SurfaceMaterial {
                material,
                ..SurfaceMaterial::default()
            },
            Some(137),
            GeometryPartition::SplitBsp,
            inline_models,
        )
    });
    let view = Refdef {
        cpu_presentation: CpuPresentation::Indexed {
            palette,
            lighting: IndexedLighting::Gray,
            ambient: 0,
            fullbright: false,
        },
        ..test_view()
    };
    let mut expected = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    let mut actual = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    assert_eq!(
        expected
            .render(&packet(&assets, &[worlds[0]], view), &assets)
            .rejected,
        0
    );
    assert_eq!(
        actual
            .render(
                &instance_packet(&assets, &[instance(&assets, worlds[0])], view),
                &assets
            )
            .rejected,
        0
    );
    assert_frame(&actual, &expected);
    assert!(actual.world_stats().indexed_pixels > 0);
    let fills = actual.world_stats().cache.fills;
    let rotated = SceneEntity {
        origin: Vec3([0.0, 4.0, 0.0]),
        axes: [
            Vec3([0.0, 1.0, 0.0]),
            Vec3([-1.0, 0.0, 0.0]),
            Vec3([0.0, 0.0, 1.0]),
        ],
        ..instance(&assets, worlds[0])
    };
    assert_eq!(
        actual
            .render(
                &instance_packet(
                    &assets,
                    &[rotated],
                    Refdef {
                        origin: rotated.origin,
                        axes: rotated.axes,
                        ..view
                    }
                ),
                &assets
            )
            .rejected,
        0
    );
    assert_frame(&actual, &expected);
    assert_eq!(actual.world_stats().cache.fills, fills);
    assert!(actual.world_stats().cache.hits > 0);
    let translated = SceneEntity {
        origin: Vec3([4.0, 0.0, 0.0]),
        ..instance(&assets, worlds[0])
    };
    assert_eq!(
        expected
            .render(&packet(&assets, &[worlds[1]], view), &assets)
            .rejected,
        0
    );
    assert_eq!(
        actual
            .render(&instance_packet(&assets, &[translated], view), &assets)
            .rejected,
        0
    );
    assert_frame(&actual, &expected);
}

#[test]
fn mixed_static_and_instanced_bsp_surfaces_use_plane_depth_order() {
    let mut assets = Assets::load().unwrap();
    let material = stages(
        &mut assets,
        &[Stage::default()],
        MaterialSettings {
            cull: Cull::None,
            ..MaterialSettings::default()
        },
    );
    let world = fixture_world(
        &mut assets,
        4.0,
        1.0,
        SurfaceMaterial {
            material,
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::SplitBsp,
        inline_models,
    );
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    assert!(frame.add_world(
        world,
        only_presentation(&assets, world),
        &[VisibleSurface {
            surface: 0,
            depth_key: 0
        }]
    ));
    assert!(frame.add_entity(SceneEntity {
        origin: Vec3([-2.0, 0.0, 0.0]),
        ..instance(&assets, world)
    }));
    assert!(frame.render_scene(test_view(), &[], &assets));
    let mut cpu = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    assert_eq!(cpu.render(&frame.finish(), &assets).rejected, 0);
    assert_eq!(
        cpu.world.as_ref().unwrap().prepare.policy,
        DepthPolicy::PlaneDepth
    );
    let center = cpu.frame.width as usize * 9 + 13;
    assert_eq!(cpu.frame.inverse_depth[center].to_bits(), 0.5_f32.to_bits());
}

#[test]
fn parallel_instance_chunks_share_packet_poses_and_disable_bsp_certification() {
    let mut assets = Assets::load().unwrap();
    let material = stages(
        &mut assets,
        &[Stage::default()],
        MaterialSettings {
            cull: Cull::None,
            ..MaterialSettings::default()
        },
    );
    let world = fixture_world(
        &mut assets,
        4.0,
        1.0,
        SurfaceMaterial {
            material,
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::SplitBsp,
        inline_models,
    );
    let mut entities = vec![instance(&assets, world); 128];
    entities[64].origin = Vec3([-2.0, 0.0, 0.0]);
    let list = instance_packet(&assets, &entities, test_view());
    let scene = Limits {
        entities: 128,
        surfaces: 128,
        ..Limits::default()
    };
    let mut serial = CpuBackend::load_with_limits(
        29,
        19,
        &assets,
        CpuLimits {
            scene,
            ..CpuLimits::default()
        },
    )
    .unwrap();
    let mut parallel = CpuBackend::load_with_limits(
        29,
        19,
        &assets,
        CpuLimits {
            scene,
            bands: RasterBands::Two,
            ..CpuLimits::default()
        },
    )
    .unwrap();
    let expected = serial.render(&list, &assets);
    let mut prepared = false;
    let actual = parallel
        .render_with_dispatch(&list, &assets, |jobs| {
            prepared |= jobs[0].kind() == JobKind::Prepare;
            for job in jobs.iter_mut().rev() {
                run_cpu_job(job);
            }
            Ok::<_, ()>(())
        })
        .unwrap();
    assert!(prepared);
    assert_eq!(expected.rejected, 0);
    assert_eq!(actual.rejected, 0);
    assert_frame(&parallel, &serial);
    assert_eq!(
        parallel.world.as_ref().unwrap().prepare.policy,
        DepthPolicy::PlaneDepth
    );
    let center = parallel.frame.width as usize * 9 + 13;
    assert_eq!(
        parallel.frame.inverse_depth[center].to_bits(),
        0.5_f32.to_bits()
    );
}

#[test]
fn world_instances_use_copied_entity_shader_inputs_and_material_override() {
    let mut assets = Assets::load().unwrap();
    let base = stages(
        &mut assets,
        &[Stage::default()],
        MaterialSettings {
            cull: Cull::None,
            ..MaterialSettings::default()
        },
    );
    let image = assets
        .register_image(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255])
        .unwrap();
    let replacement = assets
        .register_material(
            "inline entity stage",
            &[Stage {
                texture: StageTexture::Image(image),
                rgb_gen: RgbGen::Entity,
                alpha_gen: AlphaGen::Entity,
                sampler: Sampler {
                    filter: Filter::Nearest,
                    mipmaps: false,
                    ..Sampler::default()
                },
                tcmods: [
                    Some(crate::assets::TcMod::Script(TexMod::EntityTranslate)),
                    Some(crate::assets::TcMod::Script(TexMod::Scroll([0.5, 0.0]))),
                    None,
                    None,
                ],
                ..Stage::default()
            }],
            MaterialSettings {
                cull: Cull::None,
                ..MaterialSettings::default()
            },
        )
        .unwrap();
    let world = fixture_world(
        &mut assets,
        4.0,
        1.0,
        SurfaceMaterial {
            material: base,
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::Unpartitioned,
        inline_models,
    );
    let view = Refdef {
        time_ms: 2000,
        ..test_view()
    };
    let entity = SceneEntity {
        material: Some(replacement),
        color: [101, 203, 47, 255],
        ..instance(&assets, world)
    };
    let mut cpu = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    let list = instance_packet(&assets, &[entity], view);
    assert_eq!(cpu.render(&list, &assets).rejected, 0);
    let original = cpu.frame.pixels.clone();
    assert!(
        original
            .iter()
            .any(|&pixel| pixel.to_le_bytes() == [101, 0, 0, 255])
    );
    assert!(
        original
            .iter()
            .any(|&pixel| pixel.to_le_bytes() == [0, 0, 47, 255])
    );
    let shifted = SceneEntity {
        shader_texcoord: [0.5, 0.0],
        ..entity
    };
    assert_eq!(
        cpu.render(&instance_packet(&assets, &[shifted], view), &assets)
            .rejected,
        0
    );
    let shifted_pixels = cpu.frame.pixels.clone();
    assert_ne!(shifted_pixels, original);
    let timed = SceneEntity {
        shader_time: 1.0,
        ..entity
    };
    assert_eq!(
        cpu.render(&instance_packet(&assets, &[timed], view), &assets)
            .rejected,
        0
    );
    assert_eq!(cpu.frame.pixels, shifted_pixels);
}

#[test]
fn unsupported_world_instance_transform_drops_only_affected_submission() {
    let mut assets = Assets::load().unwrap();
    let material = stages(
        &mut assets,
        &[Stage::default()],
        MaterialSettings {
            cull: Cull::None,
            ..MaterialSettings::default()
        },
    );
    let world = fixture_world(
        &mut assets,
        4.0,
        1.0,
        SurfaceMaterial {
            material,
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::Unpartitioned,
        inline_models,
    );
    let good = instance(&assets, world);
    for bad in [
        SceneEntity {
            axes: [Vec3([2.0, 0.0, 0.0]), good.axes[1], good.axes[2]],
            ..good
        },
        SceneEntity {
            non_normalized_axes: true,
            ..good
        },
        SceneEntity {
            depth_hack: true,
            ..good
        },
        SceneEntity {
            origin: Vec3([f32::INFINITY, 0.0, 0.0]),
            ..good
        },
    ] {
        let mut cpu = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
        assert_eq!(
            cpu.render(
                &instance_packet(&assets, &[bad, good], test_view()),
                &assets
            )
            .rejected,
            1
        );
        assert!(cpu.frame.pixels.contains(&u32::MAX));
    }
}
