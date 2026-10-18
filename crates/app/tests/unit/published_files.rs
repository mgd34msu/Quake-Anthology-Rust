use super::*;
use crate::{client_resources::ClientResources, renderer::AssetPreparer};
use qa_core::primitives::{ClientId, ModelRef, ResourceNamespace};
use qa_network::commands::packet::Protocol;

struct Prepare<'a> {
    cpu: &'a mut CpuBackend,
    calls: usize,
}
impl AssetPreparer for Prepare<'_> {
    fn prepare_assets(
        &mut self,
        assets: &Assets,
        _: Option<qa_render::Limits>,
    ) -> Result<(), String> {
        self.calls += 1;
        self.cpu.prepare_assets(assets, None).map_err(str::to_owned)
    }
}

fn publish_file(files: &mut support::Fixture, name: &str, bytes: &[u8]) {
    let mount = files.vfs.mounts().next().unwrap().id;
    let root = files.vfs.directory_path(mount).unwrap();
    let mut file = qa_platform::files::StagedFile::create(root, name, 1024 * 1024, 256).unwrap();
    file.append(bytes).unwrap();
    assert_eq!(file.publish().unwrap(), bytes.len() as u64);
    let prefix = name.rsplit_once('/').map_or("", |(prefix, _)| prefix);
    files
        .vfs
        .refresh_directory_prefix(mount, prefix.as_bytes())
        .unwrap();
}

#[test]
fn published_model_retries_the_same_client_ordinal_without_replacing_ready_resources() {
    let mut files = support::Fixture::new(&[
        ("models/ready.md2", support::md2_skin(b"models/ready.pcx")),
        ("models/ready.pcx", support::pcx(11)),
    ]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = ModelRequest::native("", RuleSetId::Quake2, Some(palette)).profile();
    let mut owner =
        ModelAssets::new(&files.vfs, assets, [profile], [], Default::default()).unwrap();
    let ready = owner
        .register_model(&files.vfs, b"models/ready.md2", profile.source_rules)
        .unwrap();
    assert!(
        owner
            .register_model(&files.vfs, b"models/later.md2", profile.source_rules)
            .is_err()
    );
    let name = owner
        .catalog
        .names
        .find_path_bytes(b"models/later.md2")
        .unwrap();
    let ready_name = owner
        .cached_model_source(&files.vfs.default_view(), b"models/ready.md2")
        .unwrap();
    let ModelSourceFile::Ready { bytes, .. } = &owner.catalog.sources[ready_name] else {
        panic!("retained ready source");
    };
    let ready_source = Arc::clone(bytes);
    let (mut owner, mut cpu) = publish(owner);
    let ready_registration = owner.catalog.renderer_registration(ready, profile).unwrap();
    let ready_mesh = owner
        .catalog
        .bindings(ready_registration, 0, Default::default())
        .unwrap()
        .next()
        .unwrap()
        .model;
    let client = ClientId(3);
    let mut resources = ClientResources::load(
        client,
        Protocol::Quake2_34,
        RuleSetId::Quake2,
        &files.vfs,
        None,
    )
    .unwrap()
    .unwrap();
    let layout = resources.state.layout;
    let later_row = layout.resources[0].get(2).unwrap();
    resources
        .state
        .configs
        .set(later_row, b"MoDeLs\\LaTeR.Md2")
        .unwrap();
    resources
        .state
        .configs
        .set(layout.resources[0].get(3).unwrap(), b"models/ready.md2")
        .unwrap();
    assert_eq!(
        resources.state.models.bind_all(
            &resources.state.configs,
            owner.catalog(),
            profile,
            None,
            None
        ),
        1
    );
    let reference = ModelRef::native(ResourceNamespace::Client(client), 2).unwrap();
    let revision = resources.state.configs.config(later_row).unwrap().revision;
    let mut prepare = Prepare {
        cpu: &mut cpu,
        calls: 0,
    };
    assert_eq!(
        resources.state.models.prepare_pending(
            &resources.state.configs,
            &mut owner,
            &files.vfs,
            &mut prepare,
            false,
            None
        ),
        (1, true)
    );
    assert_eq!(prepare.calls, 0);
    publish_file(
        &mut files,
        "models/later.md2",
        &support::md2_skin(b"models/ready.pcx"),
    );
    assert_eq!(
        resources.state.models.prepare_pending(
            &resources.state.configs,
            &mut owner,
            &files.vfs,
            &mut prepare,
            false,
            None
        ),
        (0, false)
    );
    assert!(
        resources
            .state
            .models
            .resolve(reference, 0, owner.catalog(), Default::default())
            .is_none()
    );

    assert!(
        owner
            .downloaded(
                &files.vfs,
                b"MODELS\\LATER.MD2",
                |_, _| Ok(None::<()>),
                |_, _| panic!("failed source reset prepared")
            )
            .unwrap()
            .0
    );
    resources.retry_published_file(b"models/later.md2");
    assert_eq!(
        resources.state.models.prepare_pending(
            &resources.state.configs,
            &mut owner,
            &files.vfs,
            &mut prepare,
            false,
            None
        ),
        (0, true)
    );
    assert_eq!(prepare.calls, 1);
    assert!(
        resources
            .state
            .models
            .resolve(reference, 0, owner.catalog(), Default::default())
            .is_some()
    );
    assert_eq!(
        owner.catalog.file_handle(b"models/later.md2", None),
        Some(name.0 as i32)
    );
    assert_eq!(
        resources.state.configs.config(later_row).unwrap().revision,
        revision
    );
    assert_eq!(
        owner
            .catalog
            .bindings(ready_registration, 0, Default::default())
            .unwrap()
            .next()
            .unwrap()
            .model,
        ready_mesh
    );
    let ModelSourceFile::Ready { bytes, .. } = &owner.catalog.sources[ready_name] else {
        panic!("retained ready source");
    };
    assert!(Arc::ptr_eq(bytes, &ready_source));
    let counts = owner.assets.counts();
    assert!(
        !owner
            .downloaded(
                &files.vfs,
                b"models/later.md2",
                |_, _| Ok(None::<()>),
                |_, _| panic!("cached publication prepared")
            )
            .unwrap()
            .0
    );
    assert_eq!(owner.assets.counts(), counts);
}

#[test]
fn published_named_pcx_reuses_failed_skin_row_and_rolls_back_backend_rejection() {
    let mut files = support::Fixture::new(&[]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = ModelRequest::native("", RuleSetId::Quake2, Some(palette)).profile();
    let mut owner =
        ModelAssets::new(&files.vfs, assets, [profile], [], Default::default()).unwrap();
    let initial_skin_capacity = owner.catalog.capacity.skins;
    owner.catalog.limits.skins = initial_skin_capacity + 1;
    for _ in 0..2 {
        assert_eq!(
            owner.register_skin(&files.vfs, b"players/test/new.pcx", profile.source_rules),
            Ok(0)
        );
    }
    assert_eq!(owner.catalog.capacity.skins, initial_skin_capacity);
    let name = owner
        .catalog
        .names
        .find_path_bytes(b"players/test/new.pcx")
        .unwrap();
    let skin = owner
        .catalog
        .skin_indices
        .find(&owner.catalog.registration_vfs, name)
        .unwrap()
        .0;
    let rows = owner.catalog.skins[skin as usize].profiles.len();
    let (mut owner, mut cpu) = publish(owner);
    let before = owner.assets.counts();
    publish_file(&mut files, "players/test/new.pcx", &support::pcx(37));
    let request = request(
        b"players/test/new.pcx",
        profile.source_rules,
        RendererRegistration::Skin,
    );
    assert_eq!(
        owner
            .prepare_registration(
                &files.vfs,
                &request,
                profile,
                RegistrationVariant::Guard(false),
                |_| Ok(()),
                |_, _| panic!("cached failure prepared")
            )
            .unwrap()
            .0,
        0
    );
    assert!(
        owner
            .downloaded(
                &files.vfs,
                request.name,
                |_, _| Ok(None::<()>),
                |_, _| panic!("named failure reset prepared")
            )
            .unwrap()
            .0
    );
    assert_eq!(
        owner
            .prepare_registration(
                &files.vfs,
                &request,
                profile,
                RegistrationVariant::Guard(false),
                |_| Ok(()),
                |assets, _| {
                    let candidate =
                        CpuBackend::load_with_assets(8, 8, assets).map_err(str::to_owned)?;
                    assert_eq!(candidate.admitted_assets(), Some(assets.counts()));
                    Err("backend rejected published skin".to_owned())
                }
            )
            .unwrap_err(),
        "backend rejected published skin"
    );
    assert_eq!(owner.assets.counts(), before);
    assert_eq!(owner.catalog.capacity.skins, initial_skin_capacity);
    assert_eq!(
        owner
            .catalog
            .skin_indices
            .find(&owner.catalog.registration_vfs, name)
            .map(|index| index.0),
        Some(skin)
    );
    let (handle, (), _) = owner
        .prepare_registration(
            &files.vfs,
            &request,
            profile,
            RegistrationVariant::Guard(false),
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(handle, skin as i32 + 1);
    assert_eq!(owner.catalog.skins[skin as usize].profiles.len(), rows);
    assert_eq!(owner.catalog.capacity.skins, initial_skin_capacity + 1);
    let (ready, _) = owner
        .catalog
        .skin_profile(handle, profile)
        .unwrap()
        .state
        .ready()
        .unwrap();
    let ShaderChoice::Direct(shader) = ready.members[0][1] else {
        panic!("fullbright alias shader");
    };
    let pixels = paint_shader(&mut cpu, owner.assets(), shader);
    assert!(pixels.windows(2).all(|pair| pair[0] == pair[1]));
    assert_ne!(pixels[0], u32::from_ne_bytes([3, 5, 7, 255]));
    let counts = owner.assets.counts();
    assert!(
        !owner
            .downloaded(
                &files.vfs,
                request.name,
                |_, _| Ok(None::<()>),
                |_, _| panic!("named failure reset prepared")
            )
            .unwrap()
            .0
    );
    assert_eq!(owner.assets.counts(), counts);
}

#[test]
fn published_md2_skin_rebinds_only_missing_materials_after_backend_admission() {
    let mut files = support::Fixture::new(&[
        ("models/test/tris.md2", support::md2()),
        ("models/test/skin1.pcx", support::pcx(11)),
    ]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let native = ModelRequest::native("", RuleSetId::Quake2, Some(palette)).profile();
    let converted = ModelProfile {
        rules: RuleSetId::Quake3,
        ..native
    };
    let mut owner = ModelAssets::new(
        &files.vfs,
        assets,
        [native, converted],
        [],
        Default::default(),
    )
    .unwrap();
    let handle = owner
        .register_model(&files.vfs, b"models/test/tris.md2", native.source_rules)
        .unwrap();
    let registrations = [native, converted].map(|profile| {
        owner
            .catalog
            .renderer_registration(handle, profile)
            .unwrap()
    });
    let rows = registrations.map(|registration| {
        let ready = owner.catalog.registrations[registration.0 as usize]
            .state
            .ready()
            .unwrap()
            .0;
        let surface = owner.catalog.surfaces[ready.surfaces[0].unwrap().first as usize];
        assert_eq!(surface.skins.count, 2);
        (surface.skins.first as usize, ready.registrations.to_vec())
    });
    let before_rows = rows
        .clone()
        .map(|(row, _)| [owner.catalog.bindings[row], owner.catalog.bindings[row + 1]]);
    assert!(
        before_rows
            .iter()
            .all(|rows| rows[0].missing_image.is_some())
    );
    let meshes = before_rows.map(|rows| rows[0].variants[0].model);
    assert_eq!(meshes[0], meshes[1]);
    let ShaderChoice::Direct(old_fullbright) = before_rows[0][0].variants[1].shader else {
        panic!("alias fullbright material");
    };
    let StageTexture::Image(missing_image) = owner
        .assets
        .material(
            owner
                .assets
                .shader_binding(old_fullbright)
                .unwrap()
                .material,
        )
        .unwrap()
        .stages[0]
        .texture
    else {
        panic!("generated missing image");
    };
    let missing_rgba = owner.assets.image(missing_image).unwrap().rgba.to_vec();
    let retained_id = owner
        .cached_model_source(&files.vfs.default_view(), b"models/test/tris.md2")
        .unwrap();
    let ModelSourceFile::Ready { bytes, .. } = &owner.catalog.sources[retained_id] else {
        panic!("retained MD2 source");
    };
    let source = Arc::clone(bytes);
    let (mut owner, mut cpu) = publish(owner);
    let prefix = owner.assets.counts();
    let capacity = owner.catalog.capacity;
    let old_pixels = paint_shader(&mut cpu, owner.assets(), old_fullbright);
    files.remove("models/test/tris.md2");
    publish_file(&mut files, "models/test/skin0.pcx", &support::pcx(37));
    assert_eq!(
        owner
            .downloaded(
                &files.vfs,
                b"models/test/skin0.pcx",
                |_, _| Ok(None::<()>),
                |assets, _| {
                    let candidate =
                        CpuBackend::load_with_assets(8, 8, assets).map_err(str::to_owned)?;
                    assert_eq!(candidate.admitted_assets(), Some(assets.counts()));
                    Err("published material backend rejected".to_owned())
                }
            )
            .unwrap_err(),
        "published material backend rejected"
    );
    assert_eq!(owner.assets.counts(), prefix);
    assert_eq!(cpu.admitted_assets(), Some(prefix));
    for (index, (row, receipts)) in rows.iter().enumerate() {
        assert_eq!(&owner.catalog.bindings[*row..*row + 2], &before_rows[index]);
        assert_eq!(
            owner.catalog.registrations[registrations[index].0 as usize]
                .state
                .ready()
                .unwrap()
                .0
                .registrations
                .as_ref(),
            receipts
        );
    }
    assert_eq!(
        paint_shader(&mut cpu, owner.assets(), old_fullbright),
        old_pixels
    );
    assert!(
        owner
            .downloaded(
                &files.vfs,
                b"MoDeLs\\TeSt\\SkIn0.PcX",
                |_, _| Ok(None::<()>),
                |assets, _| { cpu.prepare_assets(assets, None).map_err(str::to_owned) }
            )
            .unwrap()
            .0
    );
    let admitted = owner.assets.counts();
    assert_eq!(admitted.models, prefix.models);
    assert_eq!(owner.catalog.capacity, capacity);
    assert_eq!(cpu.admitted_assets(), Some(admitted));
    for (index, (row, receipts)) in rows.iter().enumerate() {
        let refreshed = owner.catalog.bindings[*row];
        assert_eq!(refreshed.missing_image, None);
        assert_eq!(refreshed.variants[0].model, meshes[index]);
        assert_eq!(owner.catalog.bindings[*row + 1], before_rows[index][1]);
        let ShaderChoice::Occurrence(old_occurrence) = before_rows[index][0].variants[0].shader
        else {
            panic!("original model occurrence");
        };
        let ShaderChoice::Converted {
            binding,
            occurrence,
        } = refreshed.variants[0].shader
        else {
            panic!("refreshed generated body");
        };
        assert_eq!(occurrence, old_occurrence);
        let ready = owner.catalog.registrations[registrations[index].0 as usize]
            .state
            .ready()
            .unwrap()
            .0;
        assert_eq!(ready.registrations[occurrence.0 as usize].shader, binding);
        assert_eq!(
            ready.registrations[occurrence.0 as usize].key,
            receipts[occurrence.0 as usize].key
        );
        assert_eq!(ready.registrations[1], receipts[1]);
    }
    let ShaderChoice::Direct(new_fullbright) = owner.catalog.bindings[rows[0].0].variants[1].shader
    else {
        panic!("refreshed fullbright material");
    };
    let pixels = paint_shader(&mut cpu, owner.assets(), new_fullbright);
    // Native Q2 skin uploads apply the default intensity of two.
    assert_ne!(pixels, old_pixels);
    assert!(
        pixels
            .iter()
            .all(|&pixel| pixel == u32::from_ne_bytes([74, 255, 36, 255])),
        "first refreshed pixel: {:?}",
        pixels.first().map(|pixel| pixel.to_ne_bytes())
    );
    assert_eq!(
        owner.assets.image(missing_image).unwrap().rgba.as_ref(),
        missing_rgba
    );
    let ModelSourceFile::Ready { bytes, .. } = &owner.catalog.sources[retained_id] else {
        panic!("retained MD2 source");
    };
    assert!(Arc::ptr_eq(bytes, &source));
    files.vfs.take_lookup_count();
    assert!(
        !owner
            .downloaded(
                &files.vfs,
                b"models/test/skin0.pcx",
                |_, _| Ok(None::<()>),
                |_, _| {
                    panic!("already refreshed material prepared");
                }
            )
            .unwrap()
            .0
    );
    assert_eq!(files.vfs.take_lookup_count(), 0);
    assert_eq!(owner.assets.counts(), admitted);
}
