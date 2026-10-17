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
    let mut owner = ModelAssets::new(assets, [profile], [], Default::default()).unwrap();
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
    let ready_name = NameId(ready as u32);
    let ModelSourceFile::Ready { bytes, .. } = &owner.catalog.sources[ready_name.0 as usize] else {
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
    let mut resources = ClientResources::load(client, Protocol::Quake2_34, RuleSetId::Quake2)
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
        resources
            .state
            .models
            .bind_all(&resources.state.configs, owner.catalog(), profile, None),
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
            false
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
            false
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

    assert!(owner.retry_published_file(b"MODELS\\LATER.MD2"));
    resources.retry_published_file(b"models/later.md2");
    assert_eq!(
        resources.state.models.prepare_pending(
            &resources.state.configs,
            &mut owner,
            &files.vfs,
            &mut prepare,
            false
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
        owner.catalog.file_handle(b"models/later.md2"),
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
    let ModelSourceFile::Ready { bytes, .. } = &owner.catalog.sources[ready_name.0 as usize] else {
        panic!("retained ready source");
    };
    assert!(Arc::ptr_eq(bytes, &ready_source));
    let counts = owner.assets.counts();
    assert!(!owner.retry_published_file(b"models/later.md2"));
    assert_eq!(owner.assets.counts(), counts);
}

#[test]
fn published_named_pcx_reuses_failed_skin_row_and_rolls_back_backend_rejection() {
    let mut files = support::Fixture::new(&[]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = ModelRequest::native("", RuleSetId::Quake2, Some(palette)).profile();
    let mut owner = ModelAssets::new(assets, [profile], [], Default::default()).unwrap();
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
    let skin = owner.catalog.skin_indices[name.0 as usize].unwrap();
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
    assert!(owner.retry_published_file(request.name));
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
    assert_eq!(owner.catalog.skin_indices[name.0 as usize], Some(skin));
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
    assert!(!owner.retry_published_file(request.name));
    assert_eq!(owner.assets.counts(), counts);
}
