use super::*;

pub(super) fn verify(module: Vec<u8>, ui: Vec<u8>) {
    let mut selected_ui = ui.clone();
    let old = b"renderer ui init\n\0";
    let new = b"updated! ui init\n\0";
    let start = selected_ui
        .windows(old.len())
        .position(|text| text == old)
        .unwrap();
    selected_ui[start..start + old.len()].copy_from_slice(new);
    let (_fixture, mut host, mut source, ids, _) =
        setup_renderers(module.clone(), Some(ui.clone()));
    let root = host.runtime.vfs.mounts().next().unwrap().path.clone();
    for (directory, image, marker) in [
        ("baseq3", ui.as_slice(), b"base".as_slice()),
        ("missionpack", selected_ui.as_slice(), b"mod".as_slice()),
    ] {
        let target = root.join("q3a").join(directory);
        qa_platform::files::create_dir_all(target.join("vm")).unwrap();
        qa_platform::files::create_dir_all(target.join("gfx")).unwrap();
        qa_platform::files::write(
            target.join("gfx/palette.lmp"),
            (0..256u16)
                .flat_map(|value| [value as u8; 3])
                .collect::<Vec<_>>(),
        )
        .unwrap();
        qa_platform::files::write(
            target.join("gfx/colormap.lmp"),
            (0..64)
                .flat_map(|_| 0..=255u8)
                .chain(32i32.to_le_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        qa_platform::files::write(target.join("vm/ui"), image).unwrap();
        qa_platform::files::write(target.join("vm/restart"), &module).unwrap();
        qa_platform::files::write(target.join("selected.bin"), marker).unwrap();
    }
    host.runtime
        .vfs
        .mount_product(&root.join("q3a/baseq3"), -1)
        .unwrap();
    queue_game_directory(&mut host, ids[0], "a", 11, "missionpack");
    queue_gamestate(&mut host, ids[1], "a", 22);
    until_running(&mut host, &mut source);
    for _ in 0..12 {
        source.step(&mut host);
        if marker_count(&host, b"updated! ui init") == 1 {
            break;
        }
    }
    assert_eq!(marker_count(&host, b"updated! ui init"), 1);
    let base_inits = marker_count(&host, b"renderer ui init");
    assert!(base_inits >= 1);
    let searches = ids.map(|client| {
        host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .file_search()
            .clone()
    });
    let read = |host: &FrameHost, search: Option<&qa_content::vfs::SearchView>| {
        host.runtime
            .vfs
            .read_view(search)
            .read_file(b"selected.bin", 32, &mut Default::default())
            .unwrap()
    };
    assert_eq!(read(&host, Some(&searches[0])), b"mod");
    assert_eq!(read(&host, Some(&searches[1])), b"base");
    assert_eq!(read(&host, None), b"base");
    let peer_calls = host.module_counts(ModuleId(4)).unwrap().calls;
    queue_gamestate(&mut host, ids[0], "a", 11);
    until_running(&mut host, &mut source);
    for _ in 0..12 {
        source.step(&mut host);
        if marker_count(&host, b"renderer ui init") > base_inits {
            break;
        }
    }
    assert_eq!(marker_count(&host, b"updated! ui init"), 1);
    assert_eq!(marker_count(&host, b"renderer ui init"), base_inits + 1);
    assert!(host.module_counts(ModuleId(4)).unwrap().calls > peer_calls);
    assert!(
        host.runtime.client_resources[ids[1].0 as usize]
            .as_ref()
            .unwrap()
            .state
            .file_search()
            .same_selection(&searches[1])
    );
    for module in [ModuleId(1), ModuleId(2), ModuleId(3), ModuleId(4)] {
        assert_eq!(host.module_state(module), Some(State::Running));
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
    }
    host.shutdown_modules_with_driver(&mut source.driver());
    println!(
        "queued fs_game selected/reloaded native UI and cgame files without changing peer or default namespace"
    );
}
