use super::*;

struct IndexedFixture {
    files: support::Fixture,
    owner: ModelAssets,
    cpu: CpuBackend,
    profile: ModelProfile,
    registration: ModelRegistrationId,
    colors: SkinSelection,
    script: i32,
    shader: i32,
    world: WorldId,
    layout: IndexedSkinLayout,
    translation: IndexedSkinTranslation,
    selections: [SkinSelection; 3],
}
fn selected(owner: &ModelAssets, name: &str) -> SkinSelection {
    let (_, skin) = find_skin_index(
        &owner.catalog.names,
        &owner.catalog.skin_indices,
        name.as_bytes(),
    )
    .unwrap();
    let skin = skin.unwrap();
    let row = owner.catalog.skins[skin as usize]
        .profiles
        .iter()
        .position(|row| matches!(row.request, SkinRequest::Indexed(_)))
        .unwrap();
    SkinSelection {
        skin,
        row: row as u32,
    }
}
fn indexed_shader(owner: &ModelAssets, selection: SkinSelection) -> Option<ShaderBindingId> {
    owner
        .catalog
        .indexed_skin(
            selection,
            ModelMaterialPolicy {
                fullbright: true,
                ..Default::default()
            },
        )?
        .source(&())
}
fn texture(owner: &ModelAssets, shader: ShaderBindingId) -> ImageId {
    let binding = owner.assets.shader_binding(shader).unwrap();
    let StageTexture::Image(image) =
        owner.assets.material(binding.material).unwrap().stages[0].texture
    else {
        panic!("indexed texture body")
    };
    image
}
impl IndexedFixture {
    fn new() -> Self {
        let mut bytes = support::mdl();
        bytes[88..90].copy_from_slice(&[16, 96]);
        let files = support::Fixture::new(&[
            ("progs/player.mdl", bytes),
            ("skins/current.pcx", support::pcx_dimensions(2, 1, 16)),
            ("skins/corrupt.pcx", b"bad PCX".to_vec()),
            ("skins/unused.pcx", b"must remain unattempted".to_vec()),
            ("models/player.skin", b"body,textures/plain\n".to_vec()),
            (
                "textures/plain.tga",
                support::tga(2, 2, &[[90, 20, 10, 255]; 4]),
            ),
        ]);
        let mut assets = Assets::load().unwrap();
        let palette = support::palette(&mut assets);
        let world = support::world(&mut assets, 0.0);
        let profile = ModelRequest::native("", RuleSetId::QuakeWorld, Some(palette)).profile();
        let mut owner =
            ModelAssets::new(&files.vfs, assets, [profile], [], Default::default()).unwrap();
        let model = owner
            .register_model(&files.vfs, b"progs/player.mdl", profile.source_rules)
            .unwrap();
        let script = owner
            .register_skin(&files.vfs, b"models/player.skin", profile.source_rules)
            .unwrap();
        let shader = owner
            .register_shader(
                &files.vfs,
                b"textures/plain.tga",
                profile.source_rules,
                false,
                ShaderLighting::TwoDimensional,
            )
            .unwrap();
        let (mut owner, mut cpu) = publish(owner);
        let registration = owner.catalog.renderer_registration(model, profile).unwrap();
        let colors = owner
            .prepare_model_colors(
                &files.vfs,
                registration,
                ModelSkinColors {
                    indexed: [2, 3],
                    rgba: [4, 5],
                    upload_extent: None,
                    rgba_crop: None,
                },
                false,
                |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
            )
            .unwrap();
        let layout = IndexedSkinLayout::default();
        let translation = IndexedSkinTranslation {
            palette,
            target: Some(palette),
            indexed_colors: Some([2, 3]),
            rgba_colors: Some([4, 5]),
            crop: None,
            coordinate_extent: [2, 1],
            upload_extent: None,
            usage: ImageUse {
                role: ImageRole::Skin,
                allow_picmip: false,
                sampler: qa_render::assets::Sampler {
                    mipmaps: false,
                    ..Default::default()
                },
            },
        };
        owner
            .prepare_indexed_skin(
                &files.vfs,
                "skins/current.pcx",
                profile,
                false,
                layout,
                translation,
                |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
            )
            .unwrap();
        assert!(matches!(
            owner.prepare_indexed_skin(
                &files.vfs,
                "skins/corrupt.pcx",
                profile,
                false,
                layout,
                translation,
                |_, _| panic!("invalid source prepared")
            ),
            Err(IndexedSkinError::InvalidSource(_))
        ));
        assert!(matches!(
            owner.prepare_indexed_skin(
                &files.vfs,
                "skins/missing.pcx",
                profile,
                false,
                layout,
                translation,
                |_, _| panic!("missing source prepared")
            ),
            Err(IndexedSkinError::Missing)
        ));
        let selections = [
            "skins/current.pcx",
            "skins/corrupt.pcx",
            "skins/missing.pcx",
        ]
        .map(|path| selected(&owner, path));
        let unused = owner.ensure_skin_row("skins/unused.pcx", false).unwrap();
        owner
            .skin_variant(
                unused,
                profile,
                false,
                SkinRequest::Indexed(IndexedSkinRequest {
                    layout,
                    translation,
                }),
            )
            .unwrap();
        Self {
            files,
            owner,
            cpu,
            profile,
            registration,
            colors,
            script,
            shader,
            world,
            layout,
            translation,
            selections,
        }
    }
    fn replace_sources(&mut self) {
        let mount = self.files.vfs.mounts().next().unwrap();
        let root = mount.path.clone();
        let mount = mount.id;
        // The changed source also changes dimensions, so its old decoded-key
        // recipe cannot be reused merely because the colors are unchanged.
        for (name, width, index) in [("current", 4, 96), ("corrupt", 2, 32), ("missing", 2, 48)] {
            qa_platform::files::write(
                root.join("replacement"),
                support::pcx_dimensions(width, 1, index),
            )
            .unwrap();
            qa_platform::files::rename(
                root.join("replacement"),
                root.join(format!("skins/{name}.pcx")),
            )
            .unwrap();
        }
        self.files
            .vfs
            .refresh_directory_prefix(mount, b"skins")
            .unwrap();
    }
}

#[test]
fn indexed_skin_reload_retries_sources_and_restores_every_old_body_on_late_rejection() {
    let mut fixture = IndexedFixture::new();
    let owner = &fixture.owner;
    let prefix = owner.assets.counts();
    let capacity = owner.catalog.capacity();
    let binding_count = owner.catalog.bindings.len();
    let surface_count = owner.catalog.surfaces.len();
    let world_mesh = owner.assets.world(fixture.world).unwrap().mesh();
    let native: Vec<_> = owner
        .catalog
        .bindings(fixture.registration, 0, Default::default())
        .unwrap()
        .collect();
    let script = owner
        .catalog
        .renderer_registrations(RendererRegistration::Skin, fixture.script, fixture.profile)
        .unwrap()
        .to_vec();
    let named = owner
        .catalog
        .shader_binding(fixture.shader, fixture.profile)
        .unwrap();
    let colored: Vec<_> = owner
        .catalog
        .lod_bindings(fixture.registration, 0, Default::default())
        .unwrap()
        .with_skin(owner.catalog(), fixture.colors)
        .unwrap()
        .base()
        .unwrap()
        .collect();
    let old_image = texture(owner, indexed_shader(owner, fixture.selections[0]).unwrap());
    let old_pixels = owner.assets.image(old_image).unwrap().rgba.to_vec();
    let old_revision = owner.assets.image(old_image).unwrap().preparation_revision;
    let old_materials = owner.catalog.skins[fixture.selections[0].skin as usize].profiles
        [fixture.selections[0].row as usize]
        .state
        .ready()
        .unwrap()
        .0
        .clone();
    let old_source = match &owner.catalog.skins[fixture.selections[0].skin as usize].source {
        Some(SkinSource::Indexed { source, .. }) => Arc::clone(source),
        _ => panic!("retained positive source"),
    };
    let old_rendered = paint_shader(
        &mut fixture.cpu,
        owner.assets(),
        indexed_shader(owner, fixture.selections[0]).unwrap(),
    );
    fixture.replace_sources();
    let IndexedFixture {
        files,
        owner,
        cpu,
        selections,
        profile,
        layout,
        translation,
        registration,
        colors,
        world,
        script: script_handle,
        shader: shader_handle,
    } = &mut fixture;
    let rejected: Result<(), String> = owner.image_transaction(
        &files.vfs,
        None,
        ImageRefreshMode::IndexedSkins,
        |owner, worlds| {
            assert!(worlds.is_empty());
            for &selection in selections.iter() {
                assert!(indexed_shader(owner, selection).is_some());
            }
            let shader = indexed_shader(owner, selections[0]).unwrap();
            assert_eq!(texture(owner, shader), old_image);
            assert_eq!(
                owner.assets.image(old_image).unwrap().preparation_revision,
                old_revision + 1
            );
            assert_eq!(
                owner
                    .assets
                    .image(texture(owner, shader))
                    .unwrap()
                    .logical_size(),
                [4, 1]
            );
            let mut candidate =
                CpuBackend::load_with_assets(8, 8, owner.assets()).map_err(str::to_owned)?;
            assert_ne!(
                paint_shader(&mut candidate, owner.assets(), shader),
                old_rendered
            );
            Err("reject final indexed backend publication".to_owned())
        },
    );
    assert!(
        rejected
            .unwrap_err()
            .contains("reject final indexed backend")
    );
    assert_eq!(owner.assets.counts(), prefix);
    assert_eq!(
        owner.assets.image(old_image).unwrap().preparation_revision,
        old_revision
    );
    assert_eq!(&*owner.assets.image(old_image).unwrap().rgba, old_pixels);
    assert_eq!(owner.catalog.capacity(), capacity);
    assert_eq!(
        texture(owner, indexed_shader(owner, selections[0]).unwrap()),
        old_image
    );
    assert!(indexed_shader(owner, selections[1]).is_none());
    assert!(indexed_shader(owner, selections[2]).is_none());
    assert!(
        matches!(&owner.catalog.skins[selections[0].skin as usize].source,
        Some(SkinSource::Indexed { source, .. }) if Arc::ptr_eq(source, &old_source))
    );
    assert!(matches!(
        &owner.catalog.skins[selections[1].skin as usize].source,
        Some(SkinSource::IndexedFailure { missing: false, .. })
    ));
    assert!(matches!(
        &owner.catalog.skins[selections[2].skin as usize].source,
        Some(SkinSource::IndexedFailure { missing: true, .. })
    ));
    let candidate = owner
        .image_transaction(
            &files.vfs,
            None,
            ImageRefreshMode::IndexedSkins,
            |owner, _| CpuBackend::load_with_assets(8, 8, owner.assets()).map_err(str::to_owned),
        )
        .unwrap();
    *cpu = candidate;
    assert_eq!(owner.catalog.capacity().skins, capacity.skins + 2);
    assert_eq!(owner.catalog.bindings.len(), binding_count);
    assert_eq!(owner.catalog.surfaces.len(), surface_count);
    assert_eq!(owner.assets.counts().models, prefix.models);
    assert_eq!(owner.assets.counts().worlds, prefix.worlds);
    assert_eq!(owner.assets.counts().presentations, prefix.presentations);
    assert_eq!(owner.assets.world(*world).unwrap().mesh(), world_mesh);
    assert_eq!(
        owner
            .catalog
            .bindings(*registration, 0, Default::default())
            .unwrap()
            .collect::<Vec<_>>(),
        native
    );
    assert_eq!(
        owner
            .catalog
            .renderer_registrations(RendererRegistration::Skin, *script_handle, *profile)
            .unwrap(),
        script
    );
    assert_eq!(
        owner.catalog.shader_binding(*shader_handle, *profile),
        Some(named)
    );
    assert_eq!(
        owner
            .catalog
            .lod_bindings(*registration, 0, Default::default())
            .unwrap()
            .with_skin(owner.catalog(), *colors)
            .unwrap()
            .base()
            .unwrap()
            .collect::<Vec<_>>(),
        colored
    );
    assert_ne!(&*owner.assets.image(old_image).unwrap().rgba, old_pixels);
    assert_eq!(
        owner.assets.image(old_image).unwrap().preparation_revision,
        old_revision + 1
    );
    assert_eq!(owner.assets.counts().images, prefix.images + 2);
    assert_eq!(owner.assets.counts().materials, prefix.materials + 8);
    assert_eq!(
        owner.assets.counts().shader_bindings,
        prefix.shader_bindings + 8
    );
    assert_eq!(
        owner.catalog.skins[selections[0].skin as usize].profiles[selections[0].row as usize]
            .state
            .ready()
            .unwrap()
            .0,
        &old_materials
    );
    let unused = selected(owner, "skins/unused.pcx");
    assert!(matches!(
        owner.catalog.skins[unused.skin as usize].profiles[unused.row as usize].state,
        RegistrationState::Unattempted
    ));
    assert!(owner.catalog.skins[unused.skin as usize].source.is_none());
    let current = owner.assets.counts();
    for name in ["current", "corrupt", "missing"] {
        files.remove(&format!("skins/{name}.pcx"));
    }
    let mount = files.vfs.mounts().next().unwrap().id;
    files.vfs.refresh_directory_prefix(mount, b"skins").unwrap();
    for _ in 0..2 {
        owner
            .image_transaction(
                &files.vfs,
                None,
                ImageRefreshMode::IndexedSkins,
                |owner, _| {
                    for &selection in selections.iter() {
                        assert!(indexed_shader(owner, selection).is_none());
                    }
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(owner.assets.counts(), current);
        assert_eq!(owner.catalog.capacity().skins, capacity.skins - 1);
        assert_eq!(owner.catalog.bindings.len(), binding_count);
        assert_eq!(owner.catalog.surfaces.len(), surface_count);
    }
    // Negative results remain cached until another explicit refresh.
    files.vfs.take_lookup_count();
    assert!(matches!(
        owner.prepare_indexed_skin(
            &files.vfs,
            "skins/current.pcx",
            *profile,
            false,
            *layout,
            *translation,
            |_, _| panic!("negative source prepared")
        ),
        Err(IndexedSkinError::Missing)
    ));
    assert_eq!(files.vfs.take_lookup_count(), 0);
    let retained_materials: Vec<_> = selections
        .iter()
        .map(|selection| {
            owner.catalog.skins[selection.skin as usize].profiles[selection.row as usize]
                .reserved
                .clone()
                .unwrap()
        })
        .collect();
    let root = files.vfs.mounts().next().unwrap().path.clone();
    for width in [3, 4, 2] {
        for (name, index) in [("current", 16), ("corrupt", 32), ("missing", 48)] {
            qa_platform::files::write(
                root.join("replacement"),
                support::pcx_dimensions(width, 1, index),
            )
            .unwrap();
            qa_platform::files::rename(
                root.join("replacement"),
                root.join(format!("skins/{name}.pcx")),
            )
            .unwrap();
        }
        files.vfs.refresh_directory_prefix(mount, b"skins").unwrap();
        owner
            .image_transaction(
                &files.vfs,
                None,
                ImageRefreshMode::IndexedSkins,
                |owner, _| {
                    cpu.prepare_assets(owner.assets(), None)
                        .map_err(str::to_owned)
                },
            )
            .unwrap();
        assert_eq!(owner.assets.counts(), current);
        for (&selection, retained) in selections.iter().zip(&retained_materials) {
            let row =
                &owner.catalog.skins[selection.skin as usize].profiles[selection.row as usize];
            assert_eq!(row.state.ready().unwrap().0, retained);
            assert!(row.reserved.is_none());
            assert_eq!(
                owner
                    .assets
                    .image(texture(owner, indexed_shader(owner, selection).unwrap()))
                    .unwrap()
                    .logical_size(),
                [u32::from(width), 1]
            );
        }
    }
}

#[test]
fn indexed_skin_reload_capacity_failure_restores_earlier_rebuilt_rows() {
    let mut fixture = IndexedFixture::new();
    let prefix = fixture.owner.assets.counts();
    let capacity = fixture.owner.catalog.capacity();
    let old = indexed_shader(&fixture.owner, fixture.selections[0]).unwrap();
    fixture.replace_sources();
    fixture.owner.catalog.limits.skins = capacity.skins + 1;
    let result: Result<(), String> = fixture.owner.image_transaction(
        &fixture.files.vfs,
        None,
        ImageRefreshMode::IndexedSkins,
        |_, _| panic!("over-capacity replay published"),
    );
    assert!(result.unwrap_err().contains("indexed skin capacity"));
    assert_eq!(fixture.owner.assets.counts(), prefix);
    assert_eq!(fixture.owner.catalog.capacity(), capacity);
    assert_eq!(
        indexed_shader(&fixture.owner, fixture.selections[0]),
        Some(old)
    );
    assert!(indexed_shader(&fixture.owner, fixture.selections[1]).is_none());
    assert!(indexed_shader(&fixture.owner, fixture.selections[2]).is_none());
}
