use super::*;
use crate::{
    Runtime,
    host::FrameHost,
    modules::{Q3Spec, load_q3},
};
use qa_console::commands::Console;
use qa_session::timing::TickRate;

#[allow(dead_code)]
#[path = "../../model_assets/support.rs"]
mod files;
#[path = "../../support/service_program.rs"]
mod program;
#[path = "../../../../formats/tests/support/stored_zip.rs"]
mod zip;

fn host(
    directories: &[&str],
    archived: bool,
    explicit: Option<&str>,
) -> (files::Fixture, FrameHost) {
    use qa_formats::program::qvm::Opcode::*;
    let fixture = files::Fixture::new(&[]);
    let root = fixture.vfs.mounts().next().unwrap().path.clone();
    let image = program::bytes(&[(Enter, 64), (Const, 0), (Leave, 64)], &[]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let mut views = Vec::new();
    let mut mounts = vec!["baseq3"];
    for &directory in directories {
        if !mounts.contains(&directory) {
            mounts.push(directory);
        }
    }
    for directory in mounts {
        let root = root.join("q3a").join(directory);
        qa_platform::files::create_dir_all(root.join("vm")).unwrap();
        if archived {
            qa_platform::files::write(
                root.join("pak0.pk3"),
                zip::stored_zip(&[(b"vm/game.qvm", image.as_slice())]),
            )
            .unwrap();
        } else {
            qa_platform::files::write(root.join("vm/game.qvm"), &image).unwrap();
        }
        // The global winner is deliberately the base game. Each request's
        // actual program origin must survive its independently selected view.
        let mounts = runtime
            .vfs
            .mount_product(&root, if directory == "baseq3" { 100 } else { 1 })
            .unwrap();
        if views.len() < directories.len() && directory != "baseq3" {
            views.push(runtime.vfs.search_view(mounts).unwrap());
        }
    }
    if directories == ["baseq3"] {
        views.push(
            runtime
                .vfs
                .search_view(runtime.vfs.mounts().map(|mount| mount.id))
                .unwrap(),
        );
    }
    let specs = views
        .into_iter()
        .map(|view| {
            let mut spec = Q3Spec::parse("game:vm/game.qvm").unwrap();
            spec.search = Some(view);
            spec
        })
        .collect::<Vec<_>>();
    let mut requests = Vec::new();
    load_q3(
        &mut runtime,
        &specs,
        &mut requests,
        TickRate::fixed(50).unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(requests.len(), directories.len());
    assert!(requests.iter().all(|request| request.source.is_some()));
    let mut console = Console::new(Context::default()).unwrap();
    if let Some(value) = explicit {
        set(&mut console.cvars, "fs_game", value);
    }
    let host = FrameHost::load_modules(console, runtime, TickRate::FrameDriven, requests).unwrap();
    (fixture, host)
}

#[test]
fn server_filesystem_default_follows_the_program_origin_and_preserves_explicit_values() {
    for archived in [false, true] {
        for directory in ["baseq3", "missionpack", "custom+game"] {
            for explicit in [None, Some("owner_choice")] {
                let (_fixture, host) = host(&[directory], archived, explicit);
                let expected =
                    explicit.unwrap_or(if directory == "baseq3" { "" } else { directory });
                let view = host
                    .console
                    .cvars
                    .bind(
                        "fs_game",
                        Context {
                            side: qa_console::catalog::Scope::Server,
                            ..Context::default()
                        },
                    )
                    .unwrap();
                assert_eq!(host.console.cvars.read(view).unwrap().as_str(), expected);
                assert_eq!(
                    host.console.cvars.is_explicit(view.canonical()),
                    explicit.is_some()
                );
                let module = ModuleId(1);
                let modules = host.modules.as_ref().unwrap();
                let search = modules.storage.file_search(module);
                let source = modules.storage.file_source(module);
                let mut info = ServerInfo::load(
                    &host.console.cvars,
                    Context::default(),
                    host.runtime.vfs.mounts().count(),
                );
                assert!(info.prepare(&host.console.cvars, &host.runtime.vfs, search, source));
                let text = info.text(ConfigLayout::Q3_SYSTEM_INFO);
                let value = qa_core::text::info_value(text, b"fs_game", true)
                    .map_or(&b""[..], |range| &text[range]);
                assert_eq!(value, expected.as_bytes());
            }
        }
    }
}

#[test]
fn mixed_provider_sources_cannot_choose_each_others_global_filesystem_default() {
    let (_fixture, host) = host(&["missionpack", "custom+game"], true, None);
    let view = host
        .console
        .cvars
        .bind(
            "fs_game",
            Context {
                side: qa_console::catalog::Scope::Server,
                ..Context::default()
            },
        )
        .unwrap();
    assert_eq!(host.console.cvars.read(view).unwrap().as_str(), "");
}
