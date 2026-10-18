use super::*;
use qa_content::products::{native_base, remote_directory};
use qa_core::primitives::RuleSetId;

fn write(root: &std::path::Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    qa_platform::files::create_dir_all(path.parent().unwrap()).unwrap();
    qa_platform::files::write(path, bytes).unwrap();
}
fn read(vfs: &Vfs, search: Option<&SearchView>, name: &[u8]) -> Option<Vec<u8>> {
    let view = vfs.read_view(search);
    let file = view.open(name)?;
    Some(
        view.read_to_vec(&file, 1024, &mut Default::default())
            .unwrap(),
    )
}
#[test]
fn mod_selection_uses_existing_archives_without_changing_a_peer_or_default_reader() {
    let root = Fixture::new("remote-games");
    write(
        &root.0,
        "q3a/baseq3/pak0.pk3",
        &zip::stored_zip(&[
            (b"model.bin".as_slice(), b"base".as_slice()),
            (b"base.bin", b"kept"),
        ]),
    );
    write(
        &root.0,
        "q3a/missionpack/pak0.pk3",
        &zip::stored_zip(&[
            (b"model.bin".as_slice(), b"mod".as_slice()),
            (b"private.bin", b"mod only"),
        ]),
    );
    write(&root.0, "q3a/missionpack/config.cfg", b"mod config");
    let mut vfs = Vfs::default();
    vfs.mount_product(&root.0.join("q3a/baseq3"), 7).unwrap();
    let requested = vfs.default_view();
    let peer = vfs.search_view(vfs.mounts().map(|mount| mount.id)).unwrap();
    let base = native_base(RuleSetId::Quake3).unwrap();
    assert!(vfs.game_search(&peer, base, b"missionpack").is_err());
    let a = vfs.game_search(&requested, base, b"MISSIONPACK").unwrap();
    assert_eq!(read(&vfs, Some(&a), b"model.bin").unwrap(), b"mod");
    assert_eq!(read(&vfs, Some(&a), b"base.bin").unwrap(), b"kept");
    for search in [None, Some(&peer), Some(&requested)] {
        assert_eq!(read(&vfs, search, b"model.bin").unwrap(), b"base");
        assert!(read(&vfs, search, b"private.bin").is_none());
        assert!(
            !vfs.read_view(search)
                .files()
                .any(|(_, name)| name == b"private.bin")
        );
    }
    let private = vfs.read_view(Some(&a)).open(b"private.bin").unwrap();
    assert!(
        vfs.read_view(None)
            .read_to_vec(&private, 1024, &mut Default::default())
            .is_err()
    );
    let same = vfs.game_search(&requested, base, b"missionpack").unwrap();
    assert!(same.same_selection(&a));
    let reset = vfs.game_search(&requested, base, b"").unwrap();
    assert_eq!(read(&vfs, Some(&reset), b"model.bin").unwrap(), b"base");
    assert!(read(&vfs, Some(&reset), b"private.bin").is_none());
    assert_eq!(read(&vfs, Some(&a), b"private.bin").unwrap(), b"mod only");
    let mounts = vfs.mounts().count();
    for game in [b"../escape".as_slice(), b"a/b", b".", b"x\\y", b"missing"] {
        assert!(vfs.game_search(&requested, base, game).is_err());
    }
    assert_eq!(vfs.mounts().count(), mounts);
    let restricted = vfs.restricted_search(Some(&a), &[], &[b".cfg"]).unwrap();
    let narrow = vfs.game_search(&restricted, base, b"missionpack").unwrap();
    assert!(read(&vfs, Some(&narrow), b"private.bin").is_none());
    assert_eq!(
        read(&vfs, Some(&narrow), b"config.cfg").unwrap(),
        b"mod config"
    );
}

#[test]
fn native_defaults_and_unknown_mods_use_product_ancestry_for_every_rule_set() {
    for (index, rules, install, directory) in [
        (0, RuleSetId::Quake, "q1", "id1"),
        (1, RuleSetId::QuakeWorld, "q1", "id1"),
        (2, RuleSetId::Quake2, "q2", "baseq2"),
        (3, RuleSetId::Quake2Rerelease, "q2/rerelease", "baseq2"),
        (4, RuleSetId::Quake3, "q3a", "baseq3"),
    ] {
        let root = Fixture::new(&format!("native-game-{index}"));
        write(&root.0, &format!("{install}/{directory}/base.bin"), b"base");
        write(
            &root.0,
            &format!("{install}/custom+game/mod.bin"),
            b"custom",
        );
        let mut vfs = Vfs::default();
        vfs.mount_product(&root.0.join(install).join(directory), 0)
            .unwrap();
        let requested = vfs.default_view();
        let base = native_base(rules).unwrap();
        let default = vfs.game_search(&requested, base, b"").unwrap();
        assert_eq!(read(&vfs, Some(&default), b"base.bin").unwrap(), b"base");
        let custom = vfs.game_search(&requested, base, b"custom+game").unwrap();
        assert_eq!(read(&vfs, Some(&custom), b"mod.bin").unwrap(), b"custom");
        assert_eq!(read(&vfs, Some(&custom), b"base.bin").unwrap(), b"base");
        assert!(read(&vfs, None, b"mod.bin").is_none());
    }
    let base = native_base(RuleSetId::QuakeWorld).unwrap();
    assert_eq!(remote_directory(base, b"id1").unwrap().as_bytes(), b"qw");
}
