use super::*;
use crate::assets::PaletteId;
use crate::surface_cache::{IndexedLighting, IndexedTexture, PaletteLighting};
use crate::world::WorldPresentationId;

fn variants() -> (Assets, WorldId, [WorldPresentationId; 2], [PaletteId; 2]) {
    let mut assets = Assets::load().unwrap();
    let palettes = [0, 1].map(|channel| {
        let colors: Vec<_> = (0..=255u8)
            .flat_map(|value| {
                let mut rgb = [0; 3];
                rgb[channel] = value;
                rgb
            })
            .collect();
        let colormap: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
        assets
            .register_palette(PaletteLighting::load(&colors, &colormap, None, 256).unwrap())
            .unwrap()
    });
    let bindings = std::array::from_fn::<_, 2, _>(|variant| {
        let mips: [Vec<u8>; 4] =
            std::array::from_fn(|mip| vec![[27, 99][variant]; (16 >> mip) * (16 >> mip)]);
        let image = assets
            .register_indexed_image(
                IndexedTexture::load(16, 16, mips.each_ref().map(Vec::as_slice), false).unwrap(),
                palettes[variant],
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
        SurfaceMaterial {
            shader: *assets.shader_binding(material).unwrap(),
            ..SurfaceMaterial::default()
        }
    });
    let world = fixture_world_variants(
        &mut assets,
        4.0,
        1.0,
        &bindings,
        Some(137),
        GeometryPartition::SplitBsp,
        instances::inline_models,
    );
    let span = assets.world(world).unwrap().presentations();
    assert_eq!(span.len(), 2);
    let ids = [span[0], span[1]];
    (assets, world, ids, palettes)
}

fn selected_view(palette: PaletteId, x: u32) -> Refdef {
    Refdef {
        viewport: Viewport {
            x,
            y: 0,
            width: 16,
            height: 16,
        },
        cpu_presentation: CpuPresentation::Indexed {
            palette,
            lighting: IndexedLighting::Gray,
            ambient: 0,
            fullbright: false,
        },
        ..test_view()
    }
}

fn selected_packet(
    assets: &Assets,
    world: WorldId,
    presentation: WorldPresentationId,
    view: Refdef,
    inline: bool,
) -> CommandList {
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    if inline {
        assert!(frame.add_entity(crate::scene::SceneEntity {
            model: crate::scene::SceneModel::World {
                world,
                presentation,
                surfaces: IndexSpan { first: 0, count: 1 },
            },
            ..crate::scene::SceneEntity::default()
        }));
    } else {
        assert!(frame.add_world(
            world,
            presentation,
            &[VisibleSurface {
                surface: 0,
                depth_key: 0
            }]
        ));
    }
    assert!(frame.render_scene(view, &[], assets));
    frame.finish()
}

#[test]
fn presentation_sources_share_light_samples_and_reserve_distinct_mips() {
    let (assets, world, presentations, _) = variants();
    let cpu = CpuBackend::load_with_assets(16, 16, &assets).unwrap();
    let catalog = &cpu.world.as_ref().unwrap().prepare.catalog;
    assert_eq!(assets.worlds().len(), 1);
    assert_eq!(catalog.geometry.len(), 1);
    assert_eq!(catalog.surfaces.len(), 2);
    let infos = presentations.map(|id| *catalog.surface_info(id, 0).unwrap());
    assert_eq!(infos[0].geometry, infos[1].geometry);
    assert_ne!(infos[0].cache, infos[1].cache);
    let sources = infos.map(|info| catalog.surfaces_cache.surface(info.cache).unwrap());
    assert_eq!(
        sources[0].lightmap().unwrap().samples().as_ptr(),
        sources[1].lightmap().unwrap().samples().as_ptr()
    );
    let single = crate::surface_cache::SurfaceCatalog::load(vec![
        SurfaceSource::load(
            [0; 2],
            [16; 2],
            Some(Arc::new(
                LightGrid::rgb(2, 2, [0, 255, 255, 255], &[137; 12]).unwrap(),
            )),
            true,
            None,
        )
        .unwrap(),
    ])
    .unwrap();
    assert_eq!(
        catalog.surfaces_cache.working_set_bytes(),
        single.working_set_bytes() * 2
    );
    assert_eq!(
        assets.world_presentation(presentations[0]).unwrap().world(),
        world
    );
}

#[test]
fn same_world_presentation_a_b_a_reuses_its_own_resident_cache() {
    let (assets, world, presentations, palettes) = variants();
    let mut cpu = CpuBackend::load_with_assets(16, 16, &assets).unwrap();
    let packets = [0, 1].map(|i| {
        selected_packet(
            &assets,
            world,
            presentations[i],
            selected_view(palettes[i], 0),
            false,
        )
    });
    assert_eq!(cpu.render(&packets[0], &assets).rejected, 0);
    assert!(cpu.world_stats().indexed_pixels > 0);
    let a = cpu.pixels().to_vec();
    let first = cpu.world_stats().cache;
    assert!(first.fills > 0);
    assert_eq!(cpu.render(&packets[1], &assets).rejected, 0);
    let b = cpu.pixels().to_vec();
    let second = cpu.world_stats().cache;
    assert!(second.fills > first.fills);
    assert_ne!(a, b);
    assert_eq!(a[8 * 16 + 8], u32::from_le_bytes([27, 0, 0, 255]));
    assert_eq!(b[8 * 16 + 8], u32::from_le_bytes([0, 99, 0, 255]));
    assert_eq!(cpu.render(&packets[0], &assets).rejected, 0);
    assert_eq!(cpu.pixels(), a);
    let third = cpu.world_stats().cache;
    assert_eq!(third.fills, second.fills);
    assert!(third.hits > second.hits);
    assert_eq!(third.evictions, 0);
}

#[test]
fn two_views_of_one_world_and_inline_entities_select_independent_presentations() {
    let (assets, world, presentations, palettes) = variants();
    for inline in [false, true] {
        let mut frontend = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
        for i in 0..2 {
            frame.clear_scene();
            if inline {
                assert!(frame.add_entity(crate::scene::SceneEntity {
                    model: crate::scene::SceneModel::World {
                        world,
                        presentation: presentations[i],
                        surfaces: IndexSpan { first: 0, count: 1 },
                    },
                    ..crate::scene::SceneEntity::default()
                }));
            } else {
                assert!(frame.add_world(
                    world,
                    presentations[i],
                    &[VisibleSurface {
                        surface: 0,
                        depth_key: 0
                    }]
                ));
            }
            assert!(frame.render_scene(selected_view(palettes[i], i as u32 * 16), &[], &assets));
        }
        let packet = frame.finish();
        let mut cpu = CpuBackend::load_with_assets(32, 16, &assets).unwrap();
        assert_eq!(cpu.render(&packet, &assets).rejected, 0);
        assert!(cpu.world_stats().indexed_pixels > 0);
        for i in 0..2 {
            let mut expected = CpuBackend::load_with_assets(16, 16, &assets).unwrap();
            assert_eq!(
                expected
                    .render(
                        &selected_packet(
                            &assets,
                            world,
                            presentations[i],
                            selected_view(palettes[i], 0),
                            inline
                        ),
                        &assets
                    )
                    .rejected,
                0
            );
            for row in 0..16 {
                assert_eq!(
                    &cpu.pixels()[row * 32 + i * 16..row * 32 + i * 16 + 16],
                    &expected.pixels()[row * 16..row * 16 + 16]
                );
            }
        }
    }
}

#[test]
fn mismatched_world_presentation_is_rejected_without_affecting_a_healthy_surface() {
    let (mut assets, world, presentations, palettes) = variants();
    let material = assets.surface_binding(presentations[1], 0).unwrap().shader;
    let shader = *assets.shader_binding(material).unwrap();
    let other = fixture_world(
        &mut assets,
        2.0,
        1.0,
        SurfaceMaterial {
            shader,
            ..SurfaceMaterial::default()
        },
        Some(137),
        GeometryPartition::SplitBsp,
        instances::inline_models,
    );
    let wrong_presentation = only_presentation(&assets, other);
    assert_ne!(world, other);
    for inline in [false, true] {
        let mut healthy = CpuBackend::load_with_assets(16, 16, &assets).unwrap();
        assert_eq!(
            healthy
                .render(
                    &selected_packet(
                        &assets,
                        world,
                        presentations[0],
                        selected_view(palettes[0], 0),
                        inline,
                    ),
                    &assets,
                )
                .rejected,
            0
        );
        assert!(healthy.world_stats().indexed_pixels > 0);

        let mut rejected = CpuBackend::load_with_assets(16, 16, &assets).unwrap();
        rejected.render(
            &selected_packet(
                &assets,
                world,
                wrong_presentation,
                selected_view(palettes[0], 0),
                inline,
            ),
            &assets,
        );
        assert_eq!(rejected.world_stats().polygons, 0);
        assert!(
            rejected
                .pixels()
                .iter()
                .all(|&pixel| pixel == u32::from_le_bytes([3, 5, 7, 255]))
        );

        for bad_first in [false, true] {
            let mut frontend = FrontEnd::load(Limits::default()).unwrap();
            let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
            for bad in [bad_first, !bad_first] {
                let presentation = if bad {
                    wrong_presentation
                } else {
                    presentations[0]
                };
                if inline {
                    assert!(frame.add_entity(crate::scene::SceneEntity {
                        model: crate::scene::SceneModel::World {
                            world,
                            presentation,
                            surfaces: IndexSpan { first: 0, count: 1 },
                        },
                        ..crate::scene::SceneEntity::default()
                    }));
                } else {
                    assert!(frame.add_world(
                        world,
                        presentation,
                        &[VisibleSurface {
                            surface: 0,
                            depth_key: 0
                        }],
                    ));
                }
            }
            assert!(frame.render_scene(selected_view(palettes[0], 0), &[], &assets));
            let mut combined = CpuBackend::load_with_assets(16, 16, &assets).unwrap();
            combined.render(&frame.finish(), &assets);
            instances::assert_frame(&combined, &healthy);
            assert_eq!(
                combined.world_stats().polygons,
                healthy.world_stats().polygons
            );
        }
    }
}
