use super::*;
#[path = "../support/client_asset_frames.rs"]
mod frames;

struct Reject(usize);
impl qa_app::renderer::AssetPreparer for Reject {
    fn prepare_assets(
        &mut self,
        _: &qa_render::Assets,
        _: Option<qa_render::Limits>,
    ) -> Result<(), String> {
        self.0 += 1;
        Err("skin backend rejected".into())
    }
}

fn call(
    host: &mut FrameHost,
    command: u32,
    args: &[u64],
    backend: Option<&mut dyn qa_app::renderer::AssetPreparer>,
) -> u32 {
    scene_words(host, ModuleId(1), None, command, args, backend).unwrap()
}

pub(crate) fn downloaded_skins(native: bool) {
    let mut fixture = model_support::Fixture::surfaces();
    let path = "models/late.skin";
    let bytes = b"body,textures/glow\nglow,textures/body\n";
    let mut initial = data();
    initial[900..920].fill(0);
    initial[900..900 + path.len()].copy_from_slice(path.as_bytes());
    let program = if native {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            native_scene_data(true, &initial)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            qvm_scene_data(true, &initial)
        }
    } else {
        qvm_scene_data(true, &initial)
    };
    let mut assets = qa_render::Assets::load().unwrap();
    let palette = model_support::palette(&mut assets);
    let profile = ModelRequest::native("", RuleSetId::Quake3, Some(palette)).profile();
    let (mut host, profile) = entity_host_program(
        true,
        &mut fixture,
        program,
        assets,
        profile,
        Protocol::Quake2_34,
    );
    let model = call(&mut host, 12, &[], None);
    assert!(
        model > 0,
        "model: {:?}, {:?}, {:?}",
        host.module_state(ModuleId(1)),
        host.module_counts(ModuleId(1)),
        presented_prints(&host)
            .into_iter()
            .map(|(_, _, text)| String::from_utf8_lossy(&text).into_owned())
            .collect::<Vec<_>>()
    );
    call(&mut host, 14, &[u64::from(model)], None);
    call(&mut host, 18, &[0], None);
    assert_eq!(call(&mut host, 16, &[], None), 0);
    host.runtime.vfs.take_lookup_count();
    assert_eq!(call(&mut host, 16, &[], None), 0);
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);

    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(
            qa_render::Limits::default(),
            host.runtime.model_assets.as_mut().unwrap().assets_mut(),
            vec![],
        )
        .unwrap()
        .0,
    );
    host.bind_client_model_assets([]);
    let mut source = frames::Source::load(host.runtime.model_assets.as_ref().unwrap().assets());
    host.publish_model_admission(source.cpu.admitted_assets().unwrap())
        .unwrap();
    // The native server supplies the bytes; its transport still consumes ACKs.
    host.runtime
        .network
        .get_mut(ClientId(0), Endpoint::Server)
        .unwrap()
        .commands = None;
    source.step(&mut host);
    let mount = host.runtime.vfs.mounts().next().unwrap().id;
    host.configure_client_downloads(ClientId(0), mount, mount, 8192, 64)
        .unwrap();
    let context = Context {
        source: RuleSetId::Quake2,
        seat: SeatId::FIRST,
        ..Default::default()
    };
    let allow = host
        .console
        .cvars
        .bind("cl_allowDownload", context)
        .unwrap();
    host.console.cvars.force_write(allow, "1").unwrap();
    host.console
        .append(&format!("download {path}\n"), context)
        .unwrap();
    host.execute_commands();
    source.step(&mut host);
    let mut block = [0; 1100];
    let length = qa_network::downloads::write_chunk(
        Protocol::Quake2_34,
        qa_network::downloads::Chunk::Sequential {
            percent: 100,
            bytes: Some(bytes),
            compressed: false,
        },
        &mut block,
    )
    .unwrap();
    frames::queue(&mut host, [block[..length].to_vec()]);
    source.step(&mut host);
    assert!(host.runtime.vfs.open(path.as_bytes()).is_some());
    let prefix = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .counts();
    let mut rejected = Reject(0);
    assert_eq!(call(&mut host, 16, &[], Some(&mut rejected)), 0);
    assert_eq!(rejected.0, 1);
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        prefix
    );
    assert_eq!(source.cpu.admitted_assets(), Some(prefix));
    let skin = call(&mut host, 16, &[], Some(&mut source));
    assert!(
        skin > 0,
        "published {path} must replace the failed context entry"
    );
    host.runtime.vfs.take_lookup_count();
    let attempts = source.attempts;
    assert_eq!(call(&mut host, 16, &[], Some(&mut source)), skin);
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    assert_eq!(source.attempts, attempts);
    call(&mut host, 17, &[u64::from(skin)], None);
    let owner = host.runtime.model_assets.as_ref().unwrap();
    let registration = owner
        .catalog()
        .renderer_registration(model as i32, profile)
        .unwrap();
    let bindings = owner
        .catalog()
        .bindings(
            registration,
            0,
            qa_app::model_assets::ModelMaterialPolicy {
                custom_skin: skin as i32,
                ..Default::default()
            },
        )
        .unwrap()
        .collect::<Vec<_>>();
    assert_eq!(bindings.len(), 2);
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    call(&mut host, 13, &[], None);
    call(&mut host, 3, &[], None);
    let packet = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
    let view = packet
        .commands()
        .iter()
        .find_map(|command| match command {
            Command::View(view) => Some(view),
            _ => None,
        })
        .unwrap();
    let entities = packet.entities(view.scene.entities);
    assert_eq!(entities.len(), bindings.len());
    for (entity, binding) in entities.iter().zip(bindings) {
        assert_eq!(
            entity
                .shader
                .and_then(|shader| shader.source(host.native_shaders())),
            Some(binding.shader)
        );
    }
    assert_eq!(packet.rejected, 0);
    assert!(host.runtime.scene.as_mut().unwrap().recycle(packet).is_ok());
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    println!(
        "queued skin publication, backend rollback and native model bindings passed: native={native}"
    );
}
