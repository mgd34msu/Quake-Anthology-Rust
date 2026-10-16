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
    assert!(frame.render_scene(
        view,
        &[],
        assets,
        crate::shader::reference::ShaderView::direct()
    ));
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
        let shader_input_0 = *assets.shader_binding(material).unwrap();
        fixture_world(
            &mut assets,
            depth,
            extent,
            SurfaceMaterial {
                shader: shader_input_0,
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
    let world = {
        let shader_input_0 = *assets.shader_binding(material).unwrap();
        fixture_world(
            &mut assets,
            4.0,
            1.0,
            SurfaceMaterial {
                shader: shader_input_0,
                ..SurfaceMaterial::default()
            },
            None,
            GeometryPartition::SplitBsp,
            inline_models,
        )
    };
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
    assert!(frame.render_scene(
        test_view(),
        &[],
        &assets,
        crate::shader::reference::ShaderView::direct()
    ));
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
    let world = {
        let shader_input_0 = *assets.shader_binding(material).unwrap();
        fixture_world(
            &mut assets,
            4.0,
            1.0,
            SurfaceMaterial {
                shader: shader_input_0,
                ..SurfaceMaterial::default()
            },
            None,
            GeometryPartition::SplitBsp,
            inline_models,
        )
    };
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
    let replacement = {
        let material = assets
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
        assets
            .register_shader_binding(crate::ShaderBinding::material(
                material,
                crate::world::geometry::LightSource::TwoDimensional,
            ))
            .unwrap()
    };
    let world = {
        let shader_input_0 = *assets.shader_binding(base).unwrap();
        fixture_world(
            &mut assets,
            4.0,
            1.0,
            SurfaceMaterial {
                shader: shader_input_0,
                ..SurfaceMaterial::default()
            },
            None,
            GeometryPartition::Unpartitioned,
            inline_models,
        )
    };
    let view = Refdef {
        time_ms: 2000,
        ..test_view()
    };
    let entity = SceneEntity {
        shader: Some(replacement.into()),
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
    let world = {
        let shader_input_0 = *assets.shader_binding(material).unwrap();
        fixture_world(
            &mut assets,
            4.0,
            1.0,
            SurfaceMaterial {
                shader: shader_input_0,
                ..SurfaceMaterial::default()
            },
            None,
            GeometryPartition::Unpartitioned,
            inline_models,
        )
    };
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

fn opacity_fixture(extra: &[Stage]) -> (Assets, WorldId) {
    use crate::shader::{BlendFactor, TexCoordGen};
    let mut assets = Assets::load().unwrap();
    let image = assets.register_image(1, 1, &[200, 200, 200, 255]).unwrap();
    let light = assets.register_image(1, 1, &[128, 128, 128, 255]).unwrap();
    let mut sequence = vec![
        Stage {
            texture: StageTexture::Image(image),
            ..Stage::default()
        },
        Stage {
            texture: StageTexture::Lightmap,
            is_lightmap: true,
            texgen: TexCoordGen::Lightmap,
            depth_write: false,
            depth_func: DepthFunc::Equal,
            blend: Some(StageBlend {
                source: BlendFactor::DestinationColor,
                destination: BlendFactor::Zero,
            }),
            ..Stage::default()
        },
    ];
    sequence.extend_from_slice(extra);
    let material = stages(
        &mut assets,
        &sequence,
        MaterialSettings {
            cull: Cull::None,
            ..Default::default()
        },
    );
    let world = {
        let shader_input_0 = *assets.shader_binding(material).unwrap();
        fixture_world(
            &mut assets,
            4.,
            1.,
            SurfaceMaterial {
                shader: crate::ShaderBinding {
                    lightmap: light,
                    ..shader_input_0
                },
                ..Default::default()
            },
            None,
            GeometryPartition::SplitBsp,
            inline_models,
        )
    };
    (assets, world)
}

fn opacity_packet(assets: &Assets, entities: &[SceneEntity], view: Refdef) -> CommandList {
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([40, 40, 40, 255]).unwrap();
    assert!(frame.add_entities(entities.iter().copied()));
    assert!(frame.render_scene(
        view,
        &[],
        assets,
        crate::shader::reference::ShaderView::direct()
    ));
    frame.finish()
}

#[test]
fn completed_lightmap_material_opacity_preserves_scene_depth_and_cache_identity() {
    use crate::scene::MaterialOpacity;
    let (assets, world) = opacity_fixture(&[]);
    let view = test_view();
    let mut cpu = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    let mut baseline = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    baseline.render(&opacity_packet(&assets, &[], view), &assets);
    let center = cpu.frame.width as usize * 9 + 13;
    for (amount, expected) in [(0., 40), (0.25, 55), (0.5, 70), (1., 100)] {
        let entity = SceneEntity {
            material_opacity: MaterialOpacity::new(amount),
            ..instance(&assets, world)
        };
        let stats = cpu.render(&opacity_packet(&assets, &[entity], view), &assets);
        assert_eq!(stats.rejected, 0);
        assert_eq!(&cpu.frame.pixels[center].to_le_bytes()[..3], &[expected; 3]);
        assert_eq!(cpu.frame.inverse_depth, baseline.frame.inverse_depth);
        assert_eq!(cpu.frame.depth_ranks, baseline.frame.depth_ranks);
    }
    assert!(cpu.world_stats().cache.hits > 0);
    let entity = SceneEntity {
        material_opacity: MaterialOpacity::new(0.25),
        ..instance(&assets, world)
    };
    let mut serial = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    let packet = opacity_packet(&assets, &[entity], view);
    serial.render(&packet, &assets);
    for bands in [RasterBands::Two, RasterBands::Four, RasterBands::Eight] {
        let mut parallel = CpuBackend::load_with_limits(
            29,
            19,
            &assets,
            CpuLimits {
                bands,
                ..Default::default()
            },
        )
        .unwrap();
        parallel.render(&packet, &assets);
        assert_frame(&parallel, &serial);
        assert_eq!(parallel.frame.depth_ranks, serial.frame.depth_ranks);
    }
}

#[test]
fn opacity_runs_destination_dependent_stages_against_original_scene_once() {
    use crate::scene::MaterialOpacity;
    use crate::shader::BlendFactor;
    let (assets, world) = opacity_fixture(&[Stage {
        texture: StageTexture::default(),
        blend: Some(StageBlend {
            source: BlendFactor::DestinationColor,
            destination: BlendFactor::One,
        }),
        depth_write: false,
        depth_func: DepthFunc::Equal,
        ..Default::default()
    }]);
    let entity = SceneEntity {
        material_opacity: MaterialOpacity::new(0.25),
        ..instance(&assets, world)
    };
    let mut cpu = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    assert_eq!(
        cpu.render(&opacity_packet(&assets, &[entity], test_view()), &assets)
            .rejected,
        0
    );
    let center = cpu.frame.width as usize * 9 + 13;
    // Base 200 * light 128/255 -> 100, then source*destination + destination -> 200.
    assert_eq!(&cpu.frame.pixels[center].to_le_bytes()[..3], &[80; 3]);
}

#[test]
fn opacity_keeps_clipped_pixels_and_rejects_invalid_values() {
    use crate::scene::MaterialOpacity;
    use qa_core::primitives::Plane;
    assert!(MaterialOpacity::new(f32::NAN).is_none());
    assert!(MaterialOpacity::new(-0.1).is_none());
    assert!(MaterialOpacity::new(1.1).is_none());
    let (assets, world) = opacity_fixture(&[]);
    let entity = SceneEntity {
        material_opacity: MaterialOpacity::new(0.25),
        ..instance(&assets, world)
    };
    let packet = opacity_packet(
        &assets,
        &[entity],
        Refdef {
            clip_plane: Some(Plane::oriented(Vec3([0., 1., 0.]), 0.)),
            ..test_view()
        },
    );
    let mut cpu = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    assert_eq!(cpu.render(&packet, &assets).rejected, 0);
    assert!(cpu.frame.pixels.iter().any(|&p| p.to_le_bytes()[0] == 55));
    assert!(cpu.frame.pixels.iter().any(|&p| p.to_le_bytes()[0] == 40));
    assert!(
        cpu.frame
            .pixels
            .iter()
            .all(|&p| matches!(p.to_le_bytes()[0], 40 | 55))
    );
}

#[test]
fn animated_opacity_is_outside_material_time_and_keeps_alpha_test_holes() {
    use crate::scene::MaterialOpacity;
    let (mut assets, world) = opacity_fixture(&[]);
    let red = assets.register_image(1, 1, &[200, 0, 0, 255]).unwrap();
    let blue = assets.register_image(1, 1, &[0, 0, 200, 255]).unwrap();
    let hole = assets.register_image(1, 1, &[200, 200, 200, 0]).unwrap();
    let mut images = [red; 8];
    images[1] = blue;
    let light = assets.register_image(1, 1, &[128, 128, 128, 255]).unwrap();
    let material = stages(
        &mut assets,
        &[
            Stage {
                texture: StageTexture::Animation {
                    images,
                    count: 2,
                    frequency: 1.,
                },
                alpha_test: AlphaFunc::AtLeastHalf,
                ..Default::default()
            },
            Stage {
                texture: StageTexture::Image(light),
                blend: Some(StageBlend {
                    source: crate::shader::BlendFactor::DestinationColor,
                    destination: crate::shader::BlendFactor::Zero,
                }),
                depth_func: DepthFunc::Equal,
                depth_write: false,
                ..Default::default()
            },
        ],
        MaterialSettings {
            cull: Cull::None,
            ..Default::default()
        },
    );
    let hole_material = stages(
        &mut assets,
        &[
            Stage {
                texture: StageTexture::Image(hole),
                alpha_test: AlphaFunc::AtLeastHalf,
                ..Default::default()
            },
            Stage {
                texture: StageTexture::Image(light),
                blend: Some(StageBlend {
                    source: crate::shader::BlendFactor::One,
                    destination: crate::shader::BlendFactor::One,
                }),
                depth_func: DepthFunc::Equal,
                depth_write: false,
                ..Default::default()
            },
        ],
        MaterialSettings {
            cull: Cull::None,
            ..Default::default()
        },
    );
    let mut cpu = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    let center = 29 * 9 + 13;
    let entity = SceneEntity {
        shader: Some(material.into()),
        material_opacity: MaterialOpacity::new(0.25),
        ..instance(&assets, world)
    };
    for (time_ms, color) in [(0, [55, 30, 30]), (1000, [30, 30, 55])] {
        assert_eq!(
            cpu.render(
                &opacity_packet(
                    &assets,
                    &[entity],
                    Refdef {
                        time_ms,
                        ..test_view()
                    }
                ),
                &assets
            )
            .rejected,
            0
        );
        assert_eq!(&cpu.frame.pixels[center].to_le_bytes()[..3], &color);
    }
    let entity = SceneEntity {
        shader: Some(hole_material.into()),
        ..entity
    };
    assert_eq!(
        cpu.render(&opacity_packet(&assets, &[entity], test_view()), &assets)
            .rejected,
        0
    );
    assert!(
        cpu.frame
            .pixels
            .iter()
            .all(|&p| p == u32::from_le_bytes([40, 40, 40, 255]))
    );
}

#[test]
fn material_transaction_restores_framebuffer_after_dispatch_error() {
    use crate::scene::MaterialOpacity;
    let (assets, world) = opacity_fixture(&[]);
    let entity = SceneEntity {
        material_opacity: MaterialOpacity::new(0.25),
        ..instance(&assets, world)
    };
    let packet = opacity_packet(&assets, &[entity], test_view());
    let mut cpu = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    let mut raster_passes = 0;
    let result = cpu.render_with_dispatch(&packet, &assets, |jobs| {
        if jobs.iter().any(|job| job.kind() == JobKind::Raster) {
            raster_passes += 1;
        }
        for job in jobs {
            run_cpu_job(job);
        }
        if raster_passes == 2 {
            Err("fixture dispatch interruption")
        } else {
            Ok(())
        }
    });
    assert_eq!(result, Err("fixture dispatch interruption"));
    assert!(
        cpu.frame
            .pixels
            .iter()
            .all(|&p| p == u32::from_le_bytes([40, 40, 40, 255]))
    );
    assert_eq!(cpu.render(&packet, &assets).rejected, 0);
    assert_eq!(&cpu.frame.pixels[29 * 9 + 13].to_le_bytes()[..3], &[55; 3]);
}

#[test]
fn indexed_completed_material_uses_the_authored_blend_table_without_depth_writes() {
    use crate::scene::MaterialOpacity;
    let mut assets = Assets::load().unwrap();
    let rgb = (0..=255u8).flat_map(|value| [value; 3]).collect::<Vec<_>>();
    let colormap = (0..64).flat_map(|_| 0..=255u8).collect::<Vec<_>>();
    let table = (0..65536usize)
        .map(|i| ((i / 256 * 7 + i % 256 * 3) % 256) as u8)
        .collect::<Vec<_>>();
    let palette = assets
        .register_palette(
            crate::surface_cache::PaletteLighting::load(&rgb, &colormap, Some(&table), 256)
                .unwrap(),
        )
        .unwrap();
    let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| vec![200; (16 >> mip) * (16 >> mip)]);
    let image = assets
        .register_indexed_image(
            IndexedTexture::load(16, 16, mips.each_ref().map(Vec::as_slice), false).unwrap(),
            palette,
        )
        .unwrap();
    let material = stages(
        &mut assets,
        &[Stage {
            texture: StageTexture::Image(image),
            ..Default::default()
        }],
        MaterialSettings {
            cull: Cull::None,
            ..Default::default()
        },
    );
    let world = {
        let shader_input_0 = *assets.shader_binding(material).unwrap();
        fixture_world(
            &mut assets,
            4.,
            1.,
            SurfaceMaterial {
                shader: shader_input_0,
                ..Default::default()
            },
            Some(137),
            GeometryPartition::SplitBsp,
            inline_models,
        )
    };
    let view = Refdef {
        cpu_presentation: CpuPresentation::Indexed {
            palette,
            lighting: IndexedLighting::Gray,
            ambient: 0,
            fullbright: true,
        },
        ..test_view()
    };
    let mut cpu = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    let mut baseline = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    baseline.render(&opacity_packet(&assets, &[], view), &assets);
    for (amount, major, minor) in [(0.25, 40usize, 200usize), (0.5, 200, 40), (1., 200, 40)] {
        let entity = SceneEntity {
            material_opacity: MaterialOpacity::new(amount),
            ..instance(&assets, world)
        };
        assert_eq!(
            cpu.render(&opacity_packet(&assets, &[entity], view), &assets)
                .rejected,
            0
        );
        let expected = if amount == 1. {
            200
        } else {
            table[major * 256 + minor]
        };
        assert_eq!(cpu.frame.indices[29 * 9 + 13], expected);
        assert_eq!(cpu.frame.palettes[29 * 9 + 13], palette.0);
        assert_eq!(cpu.frame.inverse_depth, baseline.frame.inverse_depth);
        assert_eq!(cpu.frame.depth_ranks, baseline.frame.depth_ranks);
    }
}
