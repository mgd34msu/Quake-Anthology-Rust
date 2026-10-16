use super::super::*;
use qa_app::{map, renderer::SceneComposer};
use qa_render::{
    cpu::{CpuBackend, CpuLimits},
    material::{resources::ImageSettings, world_load::LoadedWorldPresentation},
    world::WorldView,
};
use qa_world::collision::{CollisionStore, TraceQuery, trace_policy};

fn draw_world(
    cpu: &mut CpuBackend,
    assets: &Assets,
    world: WorldId,
    presentation: LoadedWorldPresentation,
    view: &mut WorldView,
) -> (Vec<u32>, qa_render::BackendStats) {
    let mut frontend = qa_render::FrontEnd::load(Default::default()).unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    let mut refdef = qa_render::Refdef {
        viewport: qa_render::Viewport {
            width: 64,
            height: 48,
            ..Default::default()
        },
        origin: Vec3([32., 0., 0.]),
        axes: qa_core::math::angle_vectors(Vec3([0., 180., 0.])).axes(),
        fov: [90., 75.],
        ..Default::default()
    };
    presentation.defaults.apply(&mut refdef);
    let surfaces = view
        .query(assets.world(world).unwrap(), refdef, None, &[])
        .unwrap();
    assert_eq!(surfaces.len(), 1);
    assert!(frame.add_world(world, presentation.id, surfaces));
    assert!(frame.render_scene(
        refdef,
        &[],
        assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let stats = cpu.render(&frame.finish(), assets);
    (cpu.pixels().to_vec(), stats)
}

#[test]
fn older_world_adds_exact_presentations_through_one_compiler_without_bsp_io_or_new_geometry() {
    let mut entries = super::native_palettes();
    entries.extend([
        ("maps/first.bsp", support::world_map(8.)),
        ("maps/second.bsp", support::world_map(8.)),
    ]);
    let fixture = support::Fixture::new(&entries);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = native_profile(RuleSetId::Quake, palette);
    let mut geometry = CollisionStore::new();
    let first = map::read(&fixture.vfs, "first")
        .unwrap()
        .load(
            &fixture.vfs,
            &mut assets,
            &mut geometry,
            Some(palette),
            &[world_registration(profile).presentation.into()],
            Default::default(),
        )
        .unwrap();
    let second = map::read(&fixture.vfs, "second")
        .unwrap()
        .load(
            &fixture.vfs,
            &mut assets,
            &mut geometry,
            Some(palette),
            &[world_registration(profile).presentation.into()],
            Default::default(),
        )
        .unwrap();
    let world = first.render.world;
    let other_world = second.render.world;
    let first_presentation = first.render.presentations[0];
    let other_presentation = second.render.presentations[0];
    let mesh = assets.world(world).unwrap().mesh();
    let other_mesh = assets.world(other_world).unwrap().mesh();
    let collision = first.collision;
    let mut view = WorldView::load(assets.world(world).unwrap());
    let mut other_view = WorldView::load(assets.world(other_world).unwrap());
    let (mut scene, _) = SceneComposer::load(
        Default::default(),
        &mut assets,
        vec![first.render, second.render],
    )
    .unwrap();
    let mut owner =
        ModelAssets::new(assets, [profile], [world, other_world], Default::default()).unwrap();
    owner
        .adopt_palettes(
            qa_app::render_settings::PaletteRegistrations::from_registered([
                Some(palette),
                Some(palette),
                None,
                None,
                None,
            ]),
        )
        .unwrap();
    owner
        .bind_loaded_world(
            &first.virtual_path,
            world,
            Some(collision),
            first.collision_bounds,
        )
        .unwrap();
    owner
        .bind_loaded_world(
            &second.virtual_path,
            other_world,
            Some(second.collision),
            second.collision_bounds,
        )
        .unwrap();
    let inline = owner.register_inline(world, 1).unwrap();
    let mut cpu = CpuBackend::load_with_limits(
        64,
        48,
        owner.assets(),
        CpuLimits {
            cache_bytes: 1_048_576,
            ..Default::default()
        },
    )
    .unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    assert_eq!(
        owner.view().world_presentation(world, Some(profile), None),
        Some(first_presentation)
    );
    assert!(
        owner
            .view()
            .world_presentation(world, None, Some(other_presentation.id))
            .is_none()
    );
    let original = draw_world(
        &mut cpu,
        owner.assets(),
        world,
        first_presentation,
        &mut view,
    );
    assert!(
        original
            .0
            .iter()
            .any(|&pixel| pixel != u32::from_le_bytes([3, 5, 7, 255]))
    );
    let peer = draw_world(
        &mut cpu,
        owner.assets(),
        other_world,
        other_presentation,
        &mut other_view,
    );
    let old_geometries = geometry.geometries().collect::<Vec<_>>();
    let (rules, models) = trace_policy(RuleSetId::Quake3);
    let query = TraceQuery::point(Vec3([3., 0., 0.]), Vec3::default(), rules, models);
    let mut scratch = geometry.scratch();
    let old_trace = geometry
        .trace_model(collision, 0, query, &mut scratch)
        .unwrap();
    assert!(old_trace.fraction < 1.);
    let trace_fields = |trace: qa_world::collision::Trace| {
        (
            (
                trace.fraction.to_bits(),
                trace.end,
                trace.plane,
                trace.start_solid,
                trace.all_solid,
                trace.in_open,
                trace.in_water,
                trace.contents,
            ),
            (
                trace.entity,
                trace.surface,
                trace.surface_id,
                trace.secondary_plane,
                trace.secondary_surface_id,
                trace.brush_solid,
            ),
        )
    };
    let initial = owner.assets().counts();
    let mut new_ids = Vec::new();
    for target in [
        RuleSetId::Quake3,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::QuakeWorld,
    ] {
        let (selected, profile_counts) = owner
            .prepare_profile(
                &fixture.vfs,
                RuleSetId::Quake,
                target,
                ImageSettings::native(target),
                |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
            )
            .unwrap();
        let presentation = world_registration(selected).presentation;
        fixture.vfs.take_lookup_count();
        let (loaded, append, counts) = owner
            .prepare_world_presentation(
                &fixture.vfs,
                world,
                selected,
                presentation,
                |view, loaded| {
                    assert_eq!(
                        view.assets
                            .world_presentation_registration(loaded.presentations[0].id),
                        Some(loaded.presentations[0])
                    );
                    assert!(
                        view.world_presentation(world, Some(selected), None)
                            .is_none()
                    );
                    assert!(
                        view.world_presentation(world, None, Some(loaded.presentations[0].id))
                            .is_none()
                    );
                    assert_eq!(
                        view.world_presentation(world, None, Some(first_presentation.id)),
                        Some(first_presentation)
                    );
                    scene.stage_world(view.assets, loaded)
                },
                |assets, append| {
                    cpu.prepare_assets(assets, Some(append.limits()))
                        .map_err(str::to_owned)
                },
            )
            .unwrap();
        append.commit();
        assert_eq!(
            fixture.vfs.take_lookup_count(),
            0,
            "late {target:?} reread a resource"
        );
        let published = loaded.presentations[0];
        assert_eq!(
            owner.view().world_presentation(world, Some(selected), None),
            Some(published)
        );
        assert_eq!(published.id.0 as usize, profile_counts.presentations);
        new_ids.push(published.id);
        assert_eq!(owner.assets().counts().models, initial.models);
        assert_eq!(counts.worlds, initial.worlds);
        assert_eq!(owner.assets().world(world).unwrap().mesh(), mesh);
        assert_eq!(
            owner.assets().world(other_world).unwrap().mesh(),
            other_mesh
        );
        assert_eq!(
            owner.assets().world(other_world).unwrap().presentations(),
            [other_presentation.id]
        );
        assert_eq!(owner.catalog().inline_handle(world, 1), Some(inline));
        assert_eq!(
            owner
                .catalog()
                .loaded_world(b"maps/first.bsp")
                .unwrap()
                .collision,
            Some(collision)
        );
        let pixels = draw_world(&mut cpu, owner.assets(), world, published, &mut view);
        assert!(
            pixels
                .0
                .iter()
                .any(|&pixel| pixel != u32::from_le_bytes([3, 5, 7, 255])),
            "{target:?}"
        );
        assert_eq!(
            draw_world(
                &mut cpu,
                owner.assets(),
                world,
                first_presentation,
                &mut view
            ),
            original
        );
        assert_eq!(
            draw_world(
                &mut cpu,
                owner.assets(),
                other_world,
                other_presentation,
                &mut other_view
            ),
            peer
        );
        assert_eq!(geometry.geometries().collect::<Vec<_>>(), old_geometries);
        assert_eq!(
            trace_fields(
                geometry
                    .trace_model(collision, 0, query, &mut scratch)
                    .unwrap()
            ),
            trace_fields(old_trace)
        );
        let (cached, append, cached_counts) = owner
            .prepare_world_presentation(
                &fixture.vfs,
                world,
                selected,
                presentation,
                |view, loaded| scene.stage_world(view.assets, loaded),
                |assets, append| {
                    cpu.prepare_assets(assets, Some(append.limits()))
                        .map_err(str::to_owned)
                },
            )
            .unwrap();
        append.commit();
        assert_eq!(cached.presentations[0].id, published.id);
        assert_eq!(cached_counts, counts);
        assert_eq!(fixture.vfs.take_lookup_count(), 0);
    }
    let expected = std::iter::once(first_presentation.id)
        .chain(new_ids)
        .collect::<Vec<_>>();
    assert_eq!(
        owner.assets().world(world).unwrap().presentations(),
        expected
    );
}

#[test]
fn failed_presentation_staging_and_cpu_admission_restore_old_membership_and_recipe_for_retry() {
    let fixture = support::Fixture::new(&[("maps/first.bsp", support::world_map(8.))]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let original_profile = native_profile(RuleSetId::Quake, palette);
    let mut geometry = CollisionStore::new();
    let loaded = map::read(&fixture.vfs, "first")
        .unwrap()
        .load(
            &fixture.vfs,
            &mut assets,
            &mut geometry,
            Some(palette),
            &[world_registration(original_profile).presentation.into()],
            Default::default(),
        )
        .unwrap();
    let world = loaded.render.world;
    let collision = loaded.collision;
    let original = loaded.render.presentations[0];
    let mut view = WorldView::load(assets.world(world).unwrap());
    let (mut scene, _) =
        SceneComposer::load(Default::default(), &mut assets, vec![loaded.render]).unwrap();
    let mut owner =
        ModelAssets::new(assets, [original_profile], [world], Default::default()).unwrap();
    owner
        .adopt_palettes(
            qa_app::render_settings::PaletteRegistrations::from_registered([
                Some(palette),
                Some(palette),
                None,
                None,
                None,
            ]),
        )
        .unwrap();
    let mut cpu = CpuBackend::load_with_limits(
        64,
        48,
        owner.assets(),
        CpuLimits {
            cache_bytes: 1_048_576,
            ..Default::default()
        },
    )
    .unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    let (profile, before) = owner
        .prepare_profile(
            &fixture.vfs,
            RuleSetId::Quake,
            RuleSetId::Quake3,
            ImageSettings::native(RuleSetId::Quake3),
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let expected = draw_world(&mut cpu, owner.assets(), world, original, &mut view);
    // Rejection inside the retained-source compiler must return its recipe too.
    let mut invalid = world_registration(profile).presentation;
    invalid.q2_modulate = f32::NAN;
    assert!(
        owner
            .prepare_world_presentation(
                &fixture.vfs,
                world,
                profile,
                invalid,
                |_, _| -> Result<(), String> { panic!("invalid presentation reached stage") },
                |_, _| panic!("invalid presentation reached CPU")
            )
            .is_err()
    );
    let presentation = world_registration(profile).presentation;
    assert!(scene.begin_frame());
    assert!(
        owner
            .prepare_world_presentation(
                &fixture.vfs,
                world,
                profile,
                presentation,
                |view, loaded| scene.stage_world(view.assets, loaded),
                |_, _| panic!("active scene reached CPU")
            )
            .is_err()
    );
    scene.discard_frame();
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(
        owner.assets().world(world).unwrap().presentations(),
        [original.id]
    );
    let error = owner
        .prepare_world_presentation(
            &fixture.vfs,
            world,
            profile,
            presentation,
            |view, loaded| scene.stage_world(view.assets, loaded),
            |assets, append| {
                let mut incompatible = append.limits();
                incompatible.surfaces = 0;
                cpu.prepare_assets(assets, Some(incompatible))
                    .map_err(str::to_owned)
            },
        )
        .err()
        .unwrap();
    assert!(error.contains("scene capacity"), "{error}");
    assert_eq!(cpu.admitted_assets(), Some(before));
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(
        owner.assets().world(world).unwrap().presentations(),
        [original.id]
    );
    assert_eq!(geometry.geometries().collect::<Vec<_>>(), [collision]);
    assert_eq!(
        draw_world(&mut cpu, owner.assets(), world, original, &mut view),
        expected
    );
    fixture.vfs.take_lookup_count();
    let (ready, append, counts) = owner
        .prepare_world_presentation(
            &fixture.vfs,
            world,
            profile,
            presentation,
            |view, loaded| scene.stage_world(view.assets, loaded),
            |assets, append| {
                cpu.prepare_assets(assets, Some(append.limits()))
                    .map_err(str::to_owned)
            },
        )
        .unwrap();
    append.commit();
    assert_eq!(ready.presentations[0].id.0 as usize, before.presentations);
    assert_eq!(counts.presentations, before.presentations + 1);
    assert_eq!(
        owner.assets().world(world).unwrap().presentations(),
        [original.id, ready.presentations[0].id]
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    assert_eq!(cpu.admitted_assets(), Some(counts));
    assert_eq!(
        draw_world(&mut cpu, owner.assets(), world, original, &mut view),
        expected
    );
    assert!(
        draw_world(
            &mut cpu,
            owner.assets(),
            world,
            ready.presentations[0],
            &mut view
        )
        .0
        .iter()
        .any(|&pixel| pixel != u32::from_le_bytes([3, 5, 7, 255]))
    );
}
