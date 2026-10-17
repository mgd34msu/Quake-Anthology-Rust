use super::*;

fn color_files() -> Vec<(String, Vec<u8>)> {
    let mut files = replacement_files(RuleSetId::Quake);
    let bytes = &mut files
        .iter_mut()
        .find(|(name, _)| name == "models/paired.mdl")
        .unwrap()
        .1;
    let ranges = {
        let model = Model::parse(bytes).unwrap();
        model
            .skins
            .iter()
            .map(|skin| {
                let Image::Indexed { pixels, .. } = skin else {
                    panic!("embedded MDL skin")
                };
                let start = pixels.as_ptr().addr() - bytes.as_ptr().addr();
                start..start + pixels.len()
            })
            .collect::<Vec<_>>()
    };
    for (ordinal, range) in ranges.into_iter().enumerate() {
        bytes[range].fill(16 + ordinal as u8);
    }
    for (name, bytes) in &mut files {
        if name.ends_with(".lmp") {
            let value = if name.contains("/first_") { 16 } else { 96 };
            bytes[8..].fill(value);
        }
    }
    files
}

fn image_for(owner: &ModelAssets, shader: ShaderBindingId) -> &qa_render::assets::Image {
    let material = owner.assets().shader_binding(shader).unwrap().material;
    let StageTexture::Image(image) = owner.assets().material(material).unwrap().stages[0].texture
    else {
        panic!("colored alias texture")
    };
    owner.assets().image(image).unwrap()
}

fn group_images(owner: &ModelAssets, choice: SelectedModelChoice<'_>) -> Vec<(u8, u8)> {
    choice
        .shaders()
        .map(|shader| {
            let binding = choice
                .resolve_shader(shader, ShaderView::direct(), None)
                .unwrap()
                .source(&())
                .unwrap();
            let image = image_for(owner, binding);
            (
                image.rgba[0],
                image.indexed.as_ref().unwrap().mip(0).unwrap().indices()[0],
            )
        })
        .collect()
}

fn packet(
    owner: &ModelAssets,
    registration: ModelRegistrationId,
    skin: SkinSelection,
    replacement: bool,
) -> qa_render::scene::CommandList {
    let choices = owner
        .catalog()
        .lod_bindings(
            registration,
            3,
            ModelMaterialPolicy {
                fullbright: true,
                ..Default::default()
            },
        )
        .unwrap()
        .with_skin(owner.catalog(), skin)
        .unwrap();
    let mut front = qa_render::FrontEnd::load(
        qa_render::Limits::default()
            .for_assets(owner.assets())
            .unwrap()
            .for_model_choices(owner.catalog().max_choice_nodes())
            .unwrap(),
    )
    .unwrap();
    let mut frame = front.begin_frame([3, 5, 7, 255]).unwrap();
    assert!(choices.capture(
        &mut frame,
        qa_render::SceneEntity {
            frame: 1,
            old_frame: 3,
            back_lerp: 0.75,
            ..Default::default()
        },
        ShaderView::direct(),
        None,
        None
    ));
    let mut view = qa_render::Refdef {
        viewport: qa_render::Viewport {
            width: 64,
            height: 64,
            ..Default::default()
        },
        origin: Vec3([-8., 0., 0.]),
        near: 1.,
        fov: [90.; 2],
        model_time: Some(qa_core::primitives::ThinkTime::Seconds(0.125)),
        ..Default::default()
    };
    view.model_replacements[RuleSetId::Quake as usize].enabled = replacement;
    assert!(frame.render_scene(view, &[], owner.assets(), ShaderView::direct()));
    frame.finish()
}

#[test]
fn model_colors_keep_every_native_group_and_independent_skeletal_sidecars() {
    let files = color_files();
    let fixture = from_owned_files(&files);
    let mut assets = Assets::load().unwrap();
    let profile = replacement_profile(&mut assets, RuleSetId::Quake);
    let mut owner = replacement_owner(assets, profile);
    let handle = owner
        .register_model(&fixture.vfs, b"models/paired.mdl", profile.source_rules)
        .unwrap();
    let mut raster = cpu(&mut owner);
    let registration = owner
        .catalog()
        .renderer_registration(handle, profile)
        .unwrap();
    let counts = owner.assets().counts();
    let capacity = owner.catalog().capacity();
    let colors = ModelSkinColors {
        indexed: [2, 3],
        rgba: [4, 7],
        upload_extent: Some([32, 16]),
        rgba_crop: Some([296, 194]),
    };
    assert!(
        owner
            .prepare_model_colors(&fixture.vfs, registration, colors, false, |_, _| Err(
                "reject derived colors".to_owned()
            ))
            .is_err()
    );
    assert_eq!(owner.assets().counts(), counts);
    assert_eq!(owner.catalog().capacity().skins, capacity.skins);
    assert!(owner.catalog().skins.iter().all(|skin| {
        skin.profiles
            .iter()
            .all(|row| !matches!(row.request, SkinRequest::ModelColors(_)))
    }));
    let selection = owner
        .prepare_model_colors(&fixture.vfs, registration, colors, false, |assets, _| {
            raster.prepare_assets(assets, None).map_err(str::to_owned)
        })
        .unwrap();
    let colored = owner
        .catalog()
        .lod_bindings(registration, 3, Default::default())
        .unwrap()
        .with_skin(owner.catalog(), selection)
        .unwrap();
    assert!(colored.replacement().is_some());
    assert_eq!(colored.len(), 2);
    let native = colored.base().unwrap().choices().next().unwrap();
    assert_eq!(
        group_images(&owner, native),
        (6..12)
            .map(|ordinal| (64 + ordinal, 32 + ordinal))
            .collect::<Vec<_>>()
    );
    let (replacement, _, _) = colored.replacement().unwrap();
    let groups = replacement
        .choices()
        .map(|part| group_images(&owner, part))
        .collect::<Vec<_>>();
    assert_eq!(groups, vec![vec![(64, 32); 6], vec![(112, 48); 6]]);
    assert_eq!(owner.assets().counts().models, counts.models);
    let native_packet = packet(&owner, registration, selection, false);
    let native_draws = captured_draws(&native_packet);
    assert_eq!(native_draws.len(), 1);
    assert_eq!(
        image_for(&owner, native_draws[0].shader.unwrap().source(&()).unwrap()).rgba[0],
        71
    );
    assert_eq!(raster.render(&native_packet, owner.assets()).rejected, 0);
    assert!(raster.inverse_depth().iter().any(|&depth| depth > 0.));
    let replacement_packet = packet(&owner, registration, selection, true);
    let replacement_draws = captured_draws(&replacement_packet);
    assert_eq!(replacement_draws.len(), 2);
    let mut channels = replacement_draws
        .iter()
        .map(|draw| image_for(&owner, draw.shader.unwrap().source(&()).unwrap()).rgba[0])
        .collect::<Vec<_>>();
    channels.sort_unstable();
    assert_eq!(channels, [64, 112]);
    assert_eq!(
        raster.render(&replacement_packet, owner.assets()).rejected,
        0
    );
    assert!(raster.inverse_depth().iter().any(|&depth| depth > 0.));
    let fallback = owner
        .catalog()
        .lod_bindings(registration, u32::MAX, Default::default())
        .unwrap()
        .with_skin(owner.catalog(), selection)
        .unwrap();
    assert_eq!(
        group_images(&owner, fallback.base().unwrap().choices().next().unwrap()),
        [(64, 32)]
    );
    for (name, _) in &files {
        fixture.remove(name);
    }
    fixture.vfs.take_lookup_count();
    assert_eq!(
        owner
            .prepare_model_colors(&fixture.vfs, registration, colors, false, |_, _| panic!(
                "cached colors prepared twice"
            ))
            .unwrap(),
        selection
    );
    let second_colors = ModelSkinColors {
        indexed: [5, 6],
        rgba: [5, 6],
        ..colors
    };
    let second = owner
        .prepare_model_colors(
            &fixture.vfs,
            registration,
            second_colors,
            false,
            |assets, _| raster.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_ne!(selection, second);
    let second_group = owner
        .catalog()
        .lod_bindings(registration, 0, Default::default())
        .unwrap()
        .with_skin(owner.catalog(), second)
        .unwrap()
        .base()
        .unwrap()
        .choices()
        .next()
        .unwrap();
    assert_eq!(group_images(&owner, second_group), [(80, 80)]);
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
}

#[test]
fn model_color_sources_and_exact_selections_restore_after_profile_and_reload_rejection() {
    let files = color_files();
    let mut fixture = from_owned_files(&files);
    let mut assets = Assets::load().unwrap();
    let profile = replacement_profile(&mut assets, RuleSetId::Quake);
    let mut owner = replacement_owner(assets, profile);
    let handle = owner
        .register_model(&fixture.vfs, b"models/paired.mdl", profile.source_rules)
        .unwrap();
    let mut raster = cpu(&mut owner);
    let registration = owner
        .catalog()
        .renderer_registration(handle, profile)
        .unwrap();
    let colors = ModelSkinColors {
        indexed: [2, 3],
        rgba: [4, 7],
        upload_extent: None,
        rgba_crop: None,
    };
    let selection = owner
        .prepare_model_colors(&fixture.vfs, registration, colors, false, |assets, _| {
            raster.prepare_assets(assets, None).map_err(str::to_owned)
        })
        .unwrap();
    let original = packet(&owner, registration, selection, true);
    assert_eq!(raster.render(&original, owner.assets()).rejected, 0);
    let pixels = raster.pixels().to_vec();
    let prefix = owner.assets().counts();
    let ModelFile::Ready { lods, .. } =
        owner.catalog.files[owner.catalog.registrations[registration.0 as usize].file]
    else {
        panic!()
    };
    let source = lods.source(0).unwrap();
    let (raw, spans) = match &owner.catalog.sources[source] {
        ModelSourceFile::Ready {
            bytes,
            indexed_skins,
            ..
        } => (Arc::clone(bytes), Arc::clone(indexed_skins)),
        _ => panic!(),
    };
    for (name, _) in &files {
        fixture.remove(name);
    }
    fixture.vfs.take_lookup_count();
    let finish = MaterialFinishPolicy {
        detail_textures: false,
        ..profile.finish
    };
    let mut next = profile;
    let mut next_registration = registration;
    let mut next_selection = selection;
    for reject in [true, false] {
        let result = owner.image_transaction(
            &fixture.vfs,
            None,
            ImageRefreshMode::Reconfigure,
            |owner, _| {
                let refresh = owner.refresh_images(
                    &fixture.vfs,
                    &[profile],
                    &[],
                    [None; 5],
                    finish,
                    false,
                    |_| Ok(()),
                )?;
                let new_registration = refresh.registration(registration, owner.catalog())?;
                let new_selection = owner.catalog().refresh_skin(selection, &refresh)?;
                let mut candidate =
                    CpuBackend::load_with_assets(64, 64, owner.assets()).map_err(str::to_owned)?;
                assert_eq!(
                    candidate
                        .render(
                            &packet(owner, new_registration, new_selection, true),
                            owner.assets()
                        )
                        .rejected,
                    0
                );
                assert_eq!(candidate.pixels(), pixels);
                if reject {
                    return Err("reject derived profile publication".to_owned());
                }
                next = refresh.profile(profile);
                next_registration = new_registration;
                next_selection = new_selection;
                raster = candidate;
                Ok(())
            },
        );
        assert_eq!(result.is_err(), reject);
        if reject {
            assert!(
                result
                    .as_ref()
                    .unwrap_err()
                    .contains("reject derived profile publication")
            );
        }

        if reject {
            assert_eq!(owner.assets().counts(), prefix);
            assert!(
                owner
                    .catalog()
                    .lod_bindings(registration, 0, Default::default())
                    .unwrap()
                    .with_skin(owner.catalog(), selection)
                    .is_some()
            );
        }
    }
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    let mount = fixture.vfs.mounts().next().unwrap().id;
    let root = fixture.vfs.directory_path(mount).unwrap().to_path_buf();
    for (name, bytes) in &files {
        if name.ends_with(".lmp") {
            qa_platform::files::write(root.join(name), bytes).unwrap();
        }
    }
    fixture.vfs.refresh_directory_prefix(mount, b"").unwrap();
    let prefix = owner.assets().counts();
    let capacity = owner.catalog().capacity();
    for reject in [true, false, false] {
        let result =
            owner.image_transaction(&fixture.vfs, None, ImageRefreshMode::Reload, |owner, _| {
                let refresh = owner.refresh_images(
                    &fixture.vfs,
                    &[profile, next],
                    &[],
                    [None; 5],
                    finish,
                    false,
                    |_| Ok(()),
                )?;
                let selected = owner.catalog().refresh_skin(next_selection, &refresh)?;
                let mut candidate =
                    CpuBackend::load_with_assets(64, 64, owner.assets()).map_err(str::to_owned)?;
                assert_eq!(
                    candidate
                        .render(
                            &packet(owner, next_registration, selected, true),
                            owner.assets()
                        )
                        .rejected,
                    0
                );
                assert_eq!(candidate.pixels(), pixels);
                if reject {
                    return Err("reject model color reload publication".to_owned());
                }
                next_selection = selected;
                raster = candidate;
                Ok(())
            });
        assert_eq!(result.is_err(), reject);
        if reject {
            assert!(
                result
                    .as_ref()
                    .unwrap_err()
                    .contains("reject model color reload publication")
            );
        }

        if reject {
            assert_eq!(owner.assets().counts(), prefix);
        }
        assert_eq!(owner.assets().counts().models, prefix.models);
        assert_eq!(owner.catalog().capacity(), capacity);
        let ModelSourceFile::Ready {
            bytes,
            indexed_skins,
            ..
        } = &owner.catalog.sources[source]
        else {
            panic!()
        };
        assert!(Arc::ptr_eq(bytes, &raw));
        assert!(Arc::ptr_eq(indexed_skins, &spans));
        assert_eq!(
            raster
                .render(
                    &packet(&owner, next_registration, next_selection, true),
                    owner.assets()
                )
                .rejected,
            0
        );
        assert_eq!(raster.pixels(), pixels);
    }
}
