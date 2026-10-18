use qa_content::vfs::*;
use qa_formats::archive::Archive;
use qa_platform::files::ReadFile;
use std::sync::Arc;

#[path = "vfs/games.rs"]
mod games;
#[path = "../../formats/tests/support/stored_zip.rs"]
mod zip;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("qa-rust-vfs-{name}-{}", std::process::id()));
        qa_platform::files::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        qa_platform::files::remove_dir_all(&self.0).unwrap();
    }
}

fn pack(entries: &[(&[u8], &[u8])]) -> Vec<u8> {
    let mut bytes = b"PACK\0\0\0\0\0\0\0\0".to_vec();
    let mut directory = Vec::new();
    for &(name, data) in entries {
        let mut row = [0; 64];
        row[..name.len()].copy_from_slice(name);
        row[56..60].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        row[60..64].copy_from_slice(&(data.len() as u32).to_le_bytes());
        directory.extend(row);
        bytes.extend(data);
    }
    let offset = bytes.len() as u32;
    bytes[4..8].copy_from_slice(&offset.to_le_bytes());
    bytes[8..12].copy_from_slice(&(directory.len() as u32).to_le_bytes());
    bytes.extend(directory);
    bytes
}

#[test]
fn retail_kpf_package_uses_the_same_archive_reader_and_localization_catalog() {
    let Some(root) = std::env::var_os("QA_Q2_RERELEASE_KPF_ROOT") else {
        return;
    };
    let mut vfs = Vfs::default();
    let mounts = vfs.mount_product(std::path::Path::new(&root), 0).unwrap();
    assert!(mounts.len() > 1);
    let file = vfs.open(b"localization/loc_english.txt").unwrap();
    let origin = vfs.origin(&file).unwrap();
    assert!(origin.path.ends_with("Q2Game.kpf"));
    assert_eq!(origin.kind, MountKind::Pk3);
    let mut reader = qa_formats::archive::ArchiveReader::default();
    let bytes = vfs
        .read_to_vec(&file, 8 * 1024 * 1024, &mut reader)
        .unwrap();
    let catalog = qa_content::localization::Catalog::load(&bytes).unwrap();
    let mut text = qa_formats::localization::LocText::default();
    catalog
        .format(
            b"$g_entered_game",
            1,
            |_| Ok::<_, ()>(Some(b"Player".as_slice())),
            &mut text,
        )
        .unwrap();
    assert_eq!(text.as_bytes(), b"Player entered the game\n");
    assert!(vfs.open(b"fonts/qconfont.kfont").is_some());
}

#[test]
fn normalized_paths_reject_traversal_and_native_absolute_paths() {
    assert_eq!(normalize(b"Maps\\E1M1.BSP").unwrap(), b"maps/e1m1.bsp");
    assert_eq!(
        normalize(b"models/monsters/tank/../ctank/skin.pcx").unwrap(),
        b"models/monsters/ctank/skin.pcx"
    );
    for path in [
        b"".as_slice(),
        b"/maps/a",
        b"maps//a",
        b"../a",
        b"a/../../b",
        b"maps/./a",
        b"C:\\a",
        b"a\0b",
    ] {
        assert!(normalize(path).is_err());
    }
}

#[test]
fn profile_mount_filters_binary_assets_and_preserves_read_authority() {
    let mut archive_reader = qa_formats::archive::ArchiveReader::default();
    let root = Fixture::new("profile-filter");
    for name in [
        "q1/id1/view.JSON",
        "config.cfg",
        "maps/level.bsp",
        "assets/hidden.json",
        "SaVeS/hidden.cfg",
    ] {
        let path = root.0.join(name);
        qa_platform::files::create_dir_all(path.parent().unwrap()).unwrap();
        qa_platform::files::write(path, b"saved").unwrap();
    }
    let product = root.0.join("product");
    qa_platform::files::create_dir_all(product.join("q1/id1")).unwrap();
    qa_platform::files::write(product.join("q1/id1/view.JSON"), b"impostor").unwrap();
    let mut vfs = Vfs::default();
    let profile = vfs.mount_settings_directory(&root.0, -100).unwrap();
    vfs.mount_directory(&product, 0).unwrap();
    assert!(vfs.open(b"maps/level.bsp").is_none());
    assert!(vfs.open(b"assets/hidden.json").is_none());
    assert!(vfs.open(b"saves/hidden.cfg").is_none());
    let saved = vfs
        .files_in_mount(profile)
        .find(|(_, name)| *name == b"q1/id1/view.json")
        .unwrap()
        .0;
    let winning = vfs.open(b"q1/id1/view.json").unwrap();
    assert_ne!(saved, winning);
    let mut bytes = [0; 8];
    assert_eq!(
        vfs.read_range_reusing(&saved, 0, &mut bytes, &mut archive_reader)
            .unwrap(),
        5
    );
    assert_eq!(&bytes[..5], b"saved");
    assert_eq!(
        vfs.read_range_reusing(&winning, 0, &mut bytes, &mut archive_reader)
            .unwrap(),
        8
    );
    assert_eq!(&bytes, b"impostor");
    assert!(vfs.unmount(profile));
    assert!(vfs.files_in_mount(profile).next().is_none());
    assert!(
        vfs.read_range_reusing(&saved, 0, &mut bytes, &mut archive_reader)
            .is_err()
    );
}

#[test]
fn explicit_content_directory_selection_excludes_settings_archives_and_retired_mounts() {
    let root = Fixture::new("download-destinations");
    let first = root.0.join("first");
    let second = root.0.join("second");
    qa_platform::files::create_dir_all(&first).unwrap();
    qa_platform::files::create_dir_all(&second).unwrap();
    let package = root.0.join("pak0.pk3");
    qa_platform::files::write(
        &package,
        zip::stored_zip(&[(b"skins/archive.pcx", b"source")]),
    )
    .unwrap();
    let mut vfs = Vfs::default();
    let a = vfs.mount_directory(&first, 0).unwrap();
    let b = vfs.mount_directory(&second, 1).unwrap();
    let settings = vfs.mount_settings_directory(&root.0, 10).unwrap();
    let file = Arc::new(ReadFile::open(&package).unwrap());
    let archive = vfs
        .mount_archive(&package, 20, Arc::new(Archive::parse(file).unwrap()))
        .unwrap();
    assert_eq!(vfs.directory_path(a), Some(first.as_path()));
    assert_eq!(vfs.directory_path(b), Some(second.as_path()));
    assert_eq!(vfs.directory_path(settings), None);
    assert_eq!(vfs.directory_path(archive), None);
    assert!(vfs.unmount(a));
    assert_eq!(vfs.directory_path(a), None);
    assert_eq!(vfs.directory_path(b), Some(second.as_path()));
}

#[test]
fn three_products_and_mod_share_one_index_and_numeric_reads_keep_origin() {
    let mut archive_reader = qa_formats::archive::ArchiveReader::default();
    let root = Fixture::new("products");
    let mut vfs = Vfs::default();
    for (product, map) in [("id1", "e1m1"), ("baseq2", "base1"), ("baseq3", "q3dm1")] {
        let directory = root.0.join(product);
        qa_platform::files::create_dir_all(directory.join("maps")).unwrap();
        qa_platform::files::write(directory.join("maps").join(format!("{map}.bsp")), map).unwrap();
        vfs.mount_directory(&directory, 0).unwrap();
    }
    let reference = vfs.open(b"MAPS\\E1M1.BSP").unwrap();
    assert_eq!(vfs.take_lookup_count(), 1);
    let mut buffer = [0; 16];
    for _ in 0..10_000 {
        assert_eq!(
            vfs.read_range_reusing(&reference, 0, &mut buffer, &mut archive_reader)
                .unwrap(),
            4
        );
        assert_eq!(&buffer[..4], b"e1m1");
    }
    assert_eq!(vfs.take_lookup_count(), 0);
    assert_eq!(vfs.mounts().count(), 3);
    assert_eq!(vfs.files().count(), 3);
    assert!(vfs.origin(&reference).unwrap().path.ends_with("id1"));
    for map in [b"maps/base1.bsp".as_slice(), b"maps/q3dm1.bsp"] {
        assert!(vfs.open(map).is_some());
    }
    let mod_dir = root.0.join("mod");
    qa_platform::files::create_dir_all(mod_dir.join("maps")).unwrap();
    qa_platform::files::write(mod_dir.join("maps/E1M1.BSP"), b"mod").unwrap();
    let mod_id = vfs.mount_directory(&mod_dir, 100).unwrap();
    let modified = vfs.open(b"maps/e1m1.bsp").unwrap();
    assert_ne!(reference, modified);
    assert_eq!(
        vfs.read_range_reusing(&modified, 0, &mut buffer, &mut archive_reader)
            .unwrap(),
        3
    );
    assert_eq!(&buffer[..3], b"mod");
    assert!(vfs.unmount(mod_id));
    assert!(
        vfs.read_range_reusing(&modified, 0, &mut buffer, &mut archive_reader)
            .is_err()
    );
    assert_eq!(vfs.open(b"maps/e1m1.bsp"), Some(reference));
}

#[test]
fn retained_source_precedence_matches_winners_after_unlink_unmount_and_vfs_drop() {
    use std::cmp::Ordering::{Equal, Greater, Less};

    let root = Fixture::new("retained-precedence");
    let mut vfs = Vfs::default();
    let mut reader = qa_formats::archive::ArchiveReader::default();
    let mut accepted = Vec::new();
    for (name, priority) in [("older", 7), ("later", 7), ("lower", 6), ("higher", 8)] {
        let directory = root.0.join(name);
        qa_platform::files::create_dir_all(&directory).unwrap();
        qa_platform::files::write(directory.join("shared"), name).unwrap();
        qa_platform::files::write(directory.join("other"), b"other").unwrap();
        let mount = vfs.mount_directory(&directory, priority).unwrap();
        let file = vfs
            .files_in_mount(mount)
            .find(|(_, name)| *name == b"shared")
            .unwrap()
            .0;
        assert_eq!(
            vfs.read_to_vec(&file, 16, &mut reader).unwrap(),
            name.as_bytes()
        );
        let rank = vfs.source_precedence(&file).unwrap();
        let other = vfs
            .files_in_mount(mount)
            .find(|(_, name)| *name == b"other")
            .unwrap()
            .0;
        assert_eq!(
            rank.compare(&vfs.source_precedence(&other).unwrap()),
            Some(Equal)
        );
        accepted.push((mount, file, rank));
    }
    let [
        (older_mount, older_file, older),
        (later_mount, later_file, later),
        (_, _, lower),
        (higher_mount, higher_file, higher),
    ] = accepted.as_slice()
    else {
        panic!()
    };
    assert_eq!(later.compare(older), Some(Greater));
    assert_eq!(lower.compare(older), Some(Less));
    assert_eq!(higher.compare(later), Some(Greater));
    assert_eq!(vfs.open(b"shared"), Some(higher_file.clone()));
    assert!(vfs.unmount(*higher_mount));
    assert_eq!(vfs.open(b"shared"), Some(later_file.clone()));
    assert!(vfs.unmount(*later_mount));
    assert_eq!(vfs.open(b"shared"), Some(older_file.clone()));

    qa_platform::files::remove_file(root.0.join("older/shared")).unwrap();
    assert_eq!(
        vfs.source_precedence(older_file).unwrap().compare(older),
        Some(Equal)
    );
    assert!(vfs.unmount(*older_mount));
    assert!(matches!(
        vfs.source_precedence(older_file),
        Err(VfsError::Handle)
    ));
    assert!(matches!(
        vfs.source_precedence(higher_file),
        Err(VfsError::Handle)
    ));
    drop(vfs);
    assert_eq!(later.compare(older), Some(Greater));
    assert_eq!(lower.compare(older), Some(Less));
    assert_eq!(higher.compare(later), Some(Greater));
    assert_eq!(older.compare(&older.clone()), Some(Equal));
}

#[test]
fn source_precedence_from_independent_vfs_owners_is_incomparable() {
    let root = Fixture::new("precedence-owner");
    qa_platform::files::write(root.0.join("shared"), b"same source").unwrap();
    let mut first = Vfs::default();
    let mut second = Vfs::default();
    first.mount_directory(&root.0, 7).unwrap();
    second.mount_directory(&root.0, 7).unwrap();
    let first = first
        .source_precedence(&first.open(b"shared").unwrap())
        .unwrap();
    let second = second
        .source_precedence(&second.open(b"shared").unwrap())
        .unwrap();
    assert_eq!(first.compare(&second), None);
    assert_eq!(second.compare(&first), None);
}

#[test]
fn archive_priority_and_native_duplicate_member_order_are_preserved() {
    let mut archive_reader = qa_formats::archive::ArchiveReader::default();
    let root = Fixture::new("ranges");
    let path = root.0.join("pak0.pak");
    let skin = b"models/monsters/tank/../ctank/skin.pcx";
    qa_platform::files::write(
        &path,
        pack(&[
            (b"A", b"first"),
            (b"a", b"second"),
            (b"exact", b"old"),
            (b"exact", b"new"),
            (skin, b"skin"),
        ]),
    )
    .unwrap();
    let file = Arc::new(ReadFile::open(&path).unwrap());
    let mut vfs = Vfs::default();
    vfs.mount_archive(&path, 1, Arc::new(Archive::parse(file).unwrap()))
        .unwrap();
    let mut buffer = [0; 8];
    let first = vfs.open(b"a").unwrap();
    assert_eq!(
        vfs.read_range_reusing(&first, 0, &mut buffer, &mut archive_reader)
            .unwrap(),
        5
    );
    assert_eq!(&buffer[..5], b"first");
    assert_eq!(vfs.origin(&first).unwrap().member, b"A");
    let exact = vfs.open(b"EXACT").unwrap();
    assert_eq!(
        vfs.read_range_reusing(&exact, 0, &mut buffer, &mut archive_reader)
            .unwrap(),
        3
    );
    assert_eq!(&buffer[..3], b"old");
    let mut reader = qa_formats::archive::ArchiveReader::default();
    assert_eq!(vfs.read_to_vec(&first, 5, &mut reader).unwrap(), b"first");
    assert_eq!(vfs.read_file(b"A", 5, &mut reader).unwrap(), b"first");
    assert!(matches!(
        vfs.read_to_vec(&first, 4, &mut reader),
        Err(VfsError::Limit)
    ));
    let skin_ref = vfs.open(skin).unwrap();
    assert_eq!(
        vfs.open(b"models/monsters/ctank/skin.pcx"),
        Some(skin_ref.clone())
    );
    assert_eq!(vfs.origin(&skin_ref).unwrap().member, skin);
    assert_eq!(
        vfs.read_range_reusing(&skin_ref, 0, &mut buffer, &mut archive_reader)
            .unwrap(),
        4
    );
    assert_eq!(&buffer[..4], b"skin");
    let bad_path = root.0.join("escape.pak");
    qa_platform::files::write(&bad_path, pack(&[(b"models/../../escape", b"bad")])).unwrap();
    let bad = Archive::parse(Arc::new(ReadFile::open(&bad_path).unwrap())).unwrap();
    assert!(vfs.mount_archive(&bad_path, 100, Arc::new(bad)).is_err());
    assert_eq!(vfs.mounts().count(), 1);
    let next = root.0.join("pak1.pak");
    qa_platform::files::write(&next, pack(&[(b"a", b"last")])).unwrap();
    let next_file = Arc::new(ReadFile::open(&next).unwrap());
    let next_mount = vfs
        .mount_archive(&next, 1, Arc::new(Archive::parse(next_file).unwrap()))
        .unwrap();
    let last = vfs.open(b"A").unwrap();
    assert_eq!(
        vfs.read_range_reusing(&last, 0, &mut buffer, &mut archive_reader)
            .unwrap(),
        4
    );
    assert_eq!(&buffer[..4], b"last");
    assert_eq!(vfs.origin(&last).unwrap().kind, MountKind::Pak);
    let listing: Vec<_> = vfs
        .files()
        .map(|(reference, name)| (reference, name.to_vec()))
        .collect();
    let duplicates: Vec<_> = listing
        .iter()
        .filter(|(_, name)| name == b"a")
        .map(|(reference, _)| reference.clone())
        .collect();
    assert_eq!(duplicates.len(), 3);
    assert_eq!(&duplicates[..2], [last.clone(), first.clone()]);
    assert!(vfs.unmount(next_mount));
    assert_eq!(vfs.open(b"A"), Some(first));
    assert_eq!(vfs.files().count(), 5);
    assert!(
        vfs.read_range_reusing(&last, 0, &mut buffer, &mut archive_reader)
            .is_err()
    );

    let zip_path = root.0.join("q3a/baseq3/duplicates.kpf");
    qa_platform::files::create_dir_all(zip_path.parent().unwrap()).unwrap();
    qa_platform::files::write(
        &zip_path,
        zip::stored_zip(&[
            (b"Case-Zip", b"case first"),
            (b"CASE-ZIP", b"case last"),
            (b"exact-zip", b"exact first"),
            (b"exact-zip", b"exact last"),
            (b"priority-zip", b"priority first"),
            (b"PRIORITY-ZIP", b"priority last"),
            (b"maps/q3dm1.bsp", b"map first"),
            (b"maps/q3dm1.bsp", b"map last"),
        ]),
    )
    .unwrap();
    let mut zip_vfs = Vfs::default();
    let zip_mount = zip_vfs
        .mount_archive(
            &zip_path,
            1,
            Arc::new(Archive::parse(Arc::new(ReadFile::open(&zip_path).unwrap())).unwrap()),
        )
        .unwrap();
    let members: Vec<_> = zip_vfs
        .files_in_mount(zip_mount)
        .map(|(reference, name)| (reference, name.to_vec()))
        .collect();
    let case_members: Vec<_> = members
        .iter()
        .filter(|(_, name)| name == b"case-zip")
        .map(|(reference, _)| reference.clone())
        .collect();
    let exact_members: Vec<_> = members
        .iter()
        .filter(|(_, name)| name == b"exact-zip")
        .map(|(reference, _)| reference.clone())
        .collect();
    let priority_members: Vec<_> = members
        .iter()
        .filter(|(_, name)| name == b"priority-zip")
        .map(|(reference, _)| reference.clone())
        .collect();
    let map_members: Vec<_> = members
        .iter()
        .filter(|(_, name)| name == b"maps/q3dm1.bsp")
        .map(|(reference, _)| reference.clone())
        .collect();
    assert_eq!(case_members.len(), 2);
    assert_eq!(exact_members.len(), 2);
    assert_eq!(priority_members.len(), 2);
    assert_eq!(map_members.len(), 2);
    assert_eq!(
        zip_vfs
            .source_precedence(&case_members[0])
            .unwrap()
            .compare(&zip_vfs.source_precedence(&case_members[1]).unwrap()),
        Some(std::cmp::Ordering::Equal)
    );
    assert_eq!(
        zip_vfs.origin(&case_members[0]).unwrap().member,
        b"Case-Zip"
    );
    assert_eq!(
        zip_vfs.origin(&case_members[1]).unwrap().member,
        b"CASE-ZIP"
    );
    let case_listing: Vec<_> = zip_vfs
        .files()
        .filter(|(_, name)| *name == b"case-zip")
        .map(|(reference, _)| reference)
        .collect();
    let exact_listing: Vec<_> = zip_vfs
        .files()
        .filter(|(_, name)| *name == b"exact-zip")
        .map(|(reference, _)| reference)
        .collect();
    assert_eq!(
        case_listing,
        [case_members[1].clone(), case_members[0].clone()]
    );
    assert_eq!(
        exact_listing,
        [exact_members[1].clone(), exact_members[0].clone()]
    );
    assert_eq!(zip_vfs.take_lookup_count(), 0);
    let case_winner = zip_vfs.open(b"case-zip").unwrap();
    assert_eq!(zip_vfs.take_lookup_count(), 1);
    assert_eq!(case_winner, case_listing[0]);
    assert_eq!(case_winner, case_members[1]);
    assert_eq!(zip_vfs.origin(&case_winner).unwrap().kind, MountKind::Pk3);
    assert_eq!(zip_vfs.origin(&case_winner).unwrap().member, b"CASE-ZIP");
    let mut zip_buffer = [0; 16];
    assert_eq!(
        zip_vfs
            .read_range_reusing(&case_winner, 0, &mut zip_buffer, &mut archive_reader)
            .unwrap(),
        9
    );
    assert_eq!(&zip_buffer[..9], b"case last");
    let exact_winner = zip_vfs.open(b"EXACT-ZIP").unwrap();
    assert_eq!(exact_winner, exact_members[1]);
    assert_eq!(
        zip_vfs
            .read_range_reusing(&exact_winner, 0, &mut zip_buffer, &mut archive_reader)
            .unwrap(),
        10
    );
    assert_eq!(&zip_buffer[..10], b"exact last");
    let start_map = zip_vfs.open(b"maps/q3dm1.bsp").unwrap();
    let products = qa_content::products::detect(&zip_vfs);
    let product = products
        .iter()
        .find(|product| product.spec().key == "q3-baseq3")
        .unwrap();
    assert_eq!(start_map, map_members[1]);
    assert_eq!(product.start_map, start_map);
    assert_eq!(
        zip_vfs
            .read_range_reusing(&start_map, 0, &mut zip_buffer, &mut archive_reader)
            .unwrap(),
        8
    );
    assert_eq!(&zip_buffer[..8], b"map last");

    let higher = root.0.join("higher");
    qa_platform::files::create_dir_all(&higher).unwrap();
    qa_platform::files::write(higher.join("priority-zip"), b"higher").unwrap();
    let higher_mount = zip_vfs.mount_directory(&higher, 2).unwrap();
    let priority = zip_vfs.open(b"PRIORITY-ZIP").unwrap();
    assert_eq!(zip_vfs.origin(&priority).unwrap().mount, higher_mount);
    assert_eq!(
        zip_vfs.origin(&priority).unwrap().kind,
        MountKind::Directory
    );
    assert_eq!(
        zip_vfs
            .read_range_reusing(&priority, 0, &mut zip_buffer, &mut archive_reader)
            .unwrap(),
        6
    );
    assert_eq!(&zip_buffer[..6], b"higher");
    let priority_listing: Vec<_> = zip_vfs
        .files()
        .filter(|(_, name)| *name == b"priority-zip")
        .map(|(reference, _)| reference)
        .collect();
    assert_eq!(priority_listing[0], priority);
    assert_eq!(
        &priority_listing[1..],
        &[priority_members[1].clone(), priority_members[0].clone()]
    );
    assert!(zip_vfs.unmount(higher_mount));
    let priority = zip_vfs.open(b"priority-zip").unwrap();
    assert_eq!(priority, priority_members[1]);
    assert_eq!(zip_vfs.origin(&priority).unwrap().member, b"PRIORITY-ZIP");
    assert_eq!(
        zip_vfs
            .read_range_reusing(&priority, 0, &mut zip_buffer, &mut archive_reader)
            .unwrap(),
        13
    );
    assert_eq!(&zip_buffer[..13], b"priority last");
}

#[test]
fn whole_file_reads_keep_caps_empty_files_and_truncation_errors() {
    let root = Fixture::new("whole-file");
    qa_platform::files::write(root.0.join("a"), b"abcd").unwrap();
    qa_platform::files::write(root.0.join("empty"), b"").unwrap();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut reader = qa_formats::archive::ArchiveReader::default();
    let file = vfs.open(b"a").unwrap();
    assert_eq!(vfs.read_to_vec(&file, 4, &mut reader).unwrap(), b"abcd");
    assert_eq!(vfs.read_file(b"A", 4, &mut reader).unwrap(), b"abcd");
    let mut destination = [0xaa; 6];
    assert_eq!(
        vfs.read_whole_into(&file, &mut destination, &mut reader)
            .unwrap(),
        4
    );
    assert_eq!(&destination, b"abcd\xaa\xaa");
    destination.fill(0xbb);
    assert!(matches!(
        vfs.read_whole_into(&file, &mut destination[..3], &mut reader),
        Err(VfsError::Limit)
    ));
    assert_eq!(destination, [0xbb; 6]);
    assert!(matches!(
        vfs.read_to_vec(&file, 3, &mut reader),
        Err(VfsError::Limit)
    ));
    assert!(matches!(
        vfs.read_file(b"missing", 4, &mut reader),
        Err(VfsError::Missing)
    ));
    assert!(vfs.read_file(b"empty", 0, &mut reader).unwrap().is_empty());
    assert_eq!(
        vfs.read_whole_into(&vfs.open(b"empty").unwrap(), &mut [], &mut reader)
            .unwrap(),
        0
    );
    qa_platform::files::write(root.0.join("a"), b"ab").unwrap();
    assert!(matches!(
        vfs.read_to_vec(&file, 4, &mut reader),
        Err(VfsError::Format(qa_formats::FormatError::Truncated))
    ));
}

#[test]
fn directory_refresh_preserves_accepted_sources_and_updates_only_current_prefix() {
    let root = Fixture::new("directory-refresh");
    let loose = root.0.join("loose");
    qa_platform::files::create_dir_all(loose.join("SkInS")).unwrap();
    for (name, bytes) in [
        ("SkInS/base.pcx", b"old".as_slice()),
        ("SkInS/deleted.pcx", b"deleted"),
        ("SkInS/fallback.pcx", b"loose"),
        ("music.ogg", b"music"),
        ("skins-neighbor", b"neighbor"),
    ] {
        qa_platform::files::write(loose.join(name), bytes).unwrap();
    }
    let archive_path = root.0.join("fallback.pk3");
    qa_platform::files::write(
        &archive_path,
        zip::stored_zip(&[(b"skins/fallback.pcx", b"archive")]),
    )
    .unwrap();
    let mut vfs = Vfs::default();
    let archive = vfs
        .mount_archive(
            &archive_path,
            0,
            Arc::new(Archive::parse(Arc::new(ReadFile::open(&archive_path).unwrap())).unwrap()),
        )
        .unwrap();
    let mount = vfs.mount_directory(&loose, 10).unwrap();
    let old = vfs.open(b"skins/base.pcx").unwrap();
    let old_copy = old.clone();
    let weak = Arc::downgrade(&vfs.source_file(&old).unwrap());
    let old_rank = vfs.source_precedence(&old).unwrap();
    let music = vfs.open(b"music.ogg").unwrap();
    let neighbor = vfs.open(b"skins-neighbor").unwrap();
    let archived = vfs
        .files_in_mount(archive)
        .find(|(_, name)| *name == b"skins/fallback.pcx")
        .unwrap()
        .0;
    qa_platform::files::write(loose.join("replacement.tmp"), b"replacement").unwrap();
    qa_platform::files::rename(loose.join("replacement.tmp"), loose.join("SkInS/base.pcx"))
        .unwrap();
    qa_platform::files::write(loose.join("SkInS/new.pcx"), b"new").unwrap();
    qa_platform::files::remove_file(loose.join("SkInS/deleted.pcx")).unwrap();
    qa_platform::files::remove_file(loose.join("SkInS/fallback.pcx")).unwrap();
    vfs.refresh_directory_prefix(mount, b"SKINS").unwrap();

    let current = vfs.open(b"skins/base.pcx").unwrap();
    assert_ne!(old, current);
    assert_eq!(old_copy, old);
    let mut reader = qa_formats::archive::ArchiveReader::default();
    assert_eq!(vfs.read_to_vec(&old, 32, &mut reader).unwrap(), b"old");
    assert_eq!(vfs.length(&old).unwrap(), 3);
    assert_eq!(vfs.origin(&old).unwrap().member, b"SkInS/base.pcx");
    assert_eq!(
        vfs.read_to_vec(&current, 32, &mut reader).unwrap(),
        b"replacement"
    );
    assert_eq!(
        old_rank.compare(&vfs.source_precedence(&current).unwrap()),
        Some(std::cmp::Ordering::Equal)
    );
    assert_eq!(
        vfs.read_file(b"skins/new.pcx", 32, &mut reader).unwrap(),
        b"new"
    );
    assert!(vfs.open(b"skins/deleted.pcx").is_none());
    assert_eq!(vfs.open(b"skins/fallback.pcx"), Some(archived));
    assert_eq!(vfs.open(b"music.ogg"), Some(music));
    assert_eq!(vfs.open(b"skins-neighbor"), Some(neighbor));
    // The current index no longer owns the displaced descriptor.
    drop(old);
    assert!(weak.upgrade().is_some());
    drop(old_copy);
    assert!(weak.upgrade().is_none());

    let mut other = Vfs::default();
    other.mount_directory(&loose, 10).unwrap();
    assert!(matches!(other.length(&current), Err(VfsError::Handle)));
    assert!(vfs.unmount(mount));
    assert!(matches!(vfs.length(&current), Err(VfsError::Handle)));
}

#[test]
fn repeated_directory_refresh_releases_replaced_descriptors_and_deleted_names() {
    let root = Fixture::new("refresh-lifetimes");
    qa_platform::files::create_dir_all(root.0.join("skins")).unwrap();
    let path = root.0.join("skins/current");
    qa_platform::files::write(&path, b"first").unwrap();
    let mut vfs = Vfs::default();
    let mount = vfs.mount_directory(&root.0, 0).unwrap();
    let mut reader = qa_formats::archive::ArchiveReader::default();
    for number in 0u32..64 {
        let accepted = vfs.open(b"skins/current").unwrap();
        let weak = Arc::downgrade(&vfs.source_file(&accepted).unwrap());
        let transient = format!("skins/download-{number}");
        qa_platform::files::write(root.0.join(&transient), b"download").unwrap();
        qa_platform::files::write(root.0.join("next"), number.to_le_bytes()).unwrap();
        qa_platform::files::rename(root.0.join("next"), &path).unwrap();
        vfs.refresh_directory_prefix(mount, b"skins").unwrap();
        assert!(weak.upgrade().is_some());
        drop(accepted);
        assert!(weak.upgrade().is_none());
        assert_eq!(
            vfs.read_file(b"skins/current", 8, &mut reader).unwrap(),
            number.to_le_bytes()
        );
        assert_eq!(vfs.files_in_mount(mount).count(), 2);
        qa_platform::files::remove_file(root.0.join(&transient)).unwrap();
        vfs.refresh_directory_prefix(mount, b"skins").unwrap();
        assert!(vfs.open(transient.as_bytes()).is_none());
        assert_eq!(vfs.files_in_mount(mount).count(), 1);
    }
}

#[test]
fn directory_refresh_preserves_settings_filter_and_failed_scan_is_atomic() {
    let root = Fixture::new("refresh-filter-atomic");
    let content = root.0.join("content");
    for directory in ["skins", "assets/skins", "SaVeS"] {
        qa_platform::files::create_dir_all(content.join(directory)).unwrap();
    }
    qa_platform::files::write(content.join("skins/settings.cfg"), b"accepted").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        qa_platform::files::write(content.join("capture:backup.bin"), b"excluded").unwrap();
        qa_platform::files::write(
            content.join(std::ffi::OsString::from_vec(b"capture\xff.bin".to_vec())),
            b"excluded",
        )
        .unwrap();
    }
    let mut vfs = Vfs::default();
    let mount = vfs.mount_settings_directory(&content, -100).unwrap();
    let accepted = vfs.open(b"skins/settings.cfg").unwrap();
    qa_platform::files::write(content.join("skins/new.JSON"), b"new").unwrap();
    qa_platform::files::write(content.join("skins/image.pcx"), b"binary").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        qa_platform::files::write(content.join("skins/capture:backup.bin"), b"excluded").unwrap();
        qa_platform::files::write(
            content
                .join("skins")
                .join(std::ffi::OsString::from_vec(b"capture\xff.bin".to_vec())),
            b"excluded",
        )
        .unwrap();
    }
    qa_platform::files::write(content.join("assets/skins/hidden.cfg"), b"hidden").unwrap();
    qa_platform::files::write(content.join("SaVeS/hidden.json"), b"hidden").unwrap();
    vfs.refresh_directory_prefix(mount, b"").unwrap();
    assert!(vfs.open(b"skins/new.json").is_some());
    assert!(vfs.open(b"skins/image.pcx").is_none());
    assert!(vfs.open(b"assets/skins/hidden.cfg").is_none());
    assert!(vfs.open(b"saves/hidden.json").is_none());
    let current = vfs.open(b"skins/settings.cfg").unwrap();
    let current_source = Arc::downgrade(&vfs.source_file(&current).unwrap());
    let moved = root.0.join("moved");
    qa_platform::files::rename(&content, &moved).unwrap();
    assert!(vfs.refresh_directory_prefix(mount, b"skins").is_err());
    assert_eq!(vfs.open(b"skins/settings.cfg"), Some(current));
    assert_eq!(vfs.files().count(), 2);
    assert!(current_source.upgrade().is_some());
    let mut reader = qa_formats::archive::ArchiveReader::default();
    assert_eq!(
        vfs.read_to_vec(&accepted, 16, &mut reader).unwrap(),
        b"accepted"
    );
    qa_platform::files::rename(&moved, &content).unwrap();
    qa_platform::files::remove_dir_all(content.join("skins")).unwrap();
    // A missing prefix removes that prefix, without retiring its accepted handles.
    vfs.refresh_directory_prefix(mount, b"skins").unwrap();
    assert_eq!(vfs.files().count(), 0);
    assert!(vfs.open(b"skins/settings.cfg").is_none());
    assert!(current_source.upgrade().is_none());
    assert_eq!(
        vfs.read_to_vec(&accepted, 16, &mut reader).unwrap(),
        b"accepted"
    );
}

#[test]
fn published_packages_share_startup_order_keep_handles_and_retry_without_duplicate_mounts() {
    let root = Fixture::new("published-packages");
    qa_platform::files::write(root.0.join("shared"), b"loose").unwrap();
    qa_platform::files::write(root.0.join("pak2.pak"), pack(&[(b"shared", b"pak2")])).unwrap();
    let mut vfs = Vfs::default();
    let mounts = vfs.mount_product(&root.0, 100).unwrap();
    let parent = mounts[0];
    let retained = vfs.open(b"shared").unwrap();
    let retained_precedence = vfs.source_precedence(&retained).unwrap();
    let mut reader = Default::default();
    let read = |vfs: &Vfs, name: &[u8], reader: &mut qa_formats::archive::ArchiveReader| {
        let file = vfs.open(name).unwrap();
        vfs.read_to_vec(&file, 4096, reader).unwrap()
    };
    for (name, bytes, expected) in [
        (
            "pak10.pak",
            pack(&[(b"shared", b"pak10"), (b"models/late", b"new model")]),
            b"pak10".as_slice(),
        ),
        ("pak1.pak", pack(&[(b"shared", b"pak1")]), b"pak10"),
        ("z.pk3", zip::stored_zip(&[(b"shared", b"z")]), b"z"),
        ("a.pk3", zip::stored_zip(&[(b"shared", b"a")]), b"z"),
        (
            "Middle.KPF",
            zip::stored_zip(&[(b"shared", b"middle")]),
            b"z",
        ),
    ] {
        qa_platform::files::write(root.0.join(name), bytes).unwrap();
        let package = vfs
            .refresh_published_file(parent, name.as_bytes(), None)
            .unwrap()
            .unwrap();
        let count = vfs.mounts().count();
        if name == "pak1.pak" {
            let member = vfs.files_in_mount(package).next().unwrap().0;
            assert_eq!(
                retained_precedence.compare(&vfs.source_precedence(&member).unwrap()),
                Some(std::cmp::Ordering::Greater)
            );
        }
        assert_eq!(
            vfs.refresh_published_file(parent, name.as_bytes(), None)
                .unwrap(),
            Some(package)
        );
        assert_eq!(vfs.mounts().count(), count);
        assert_eq!(read(&vfs, b"shared", &mut reader), expected);
        assert_eq!(
            vfs.read_to_vec(&retained, 4096, &mut reader).unwrap(),
            b"pak2"
        );
    }
    assert_eq!(read(&vfs, b"models/late", &mut reader), b"new model");
    // A winning file from a different mount cannot redirect this authority-bound publication.
    let other = root.0.join("other");
    qa_platform::files::create_dir_all(&other).unwrap();
    qa_platform::files::create_dir_all(other.join("packs")).unwrap();
    qa_platform::files::write(other.join("packs/new.pk3"), b"not an archive").unwrap();
    qa_platform::files::write(other.join("shared"), b"higher product").unwrap();
    vfs.mount_directory(&other, 101).unwrap();
    qa_platform::files::create_dir_all(root.0.join("packs")).unwrap();
    qa_platform::files::write(
        root.0.join("packs/new.pk3"),
        zip::stored_zip(&[(b"sound/late.wav", b"selected product")]),
    )
    .unwrap();
    let package = vfs
        .refresh_published_file(parent, b"packs/new.pk3", None)
        .unwrap()
        .unwrap();
    assert_eq!(vfs.files_in_mount(package).count(), 1);
    assert_eq!(
        read(&vfs, b"sound/late.wav", &mut reader),
        b"selected product"
    );
    assert_eq!(read(&vfs, b"packs/new.pk3", &mut reader), b"not an archive");
    assert_eq!(read(&vfs, b"shared", &mut reader), b"higher product");
}

#[test]
fn malformed_published_packages_never_admit_members_or_replace_existing_winners() {
    let root = Fixture::new("bad-published-packages");
    qa_platform::files::write(root.0.join("shared"), b"retained").unwrap();
    let mut vfs = Vfs::default();
    let parent = vfs.mount_directory(&root.0, 0).unwrap();
    let retained = vfs.open(b"shared").unwrap();
    for (name, data) in [
        ("broken.pk3", b"not a ZIP".to_vec()),
        (
            "escape.pk3",
            zip::stored_zip(&[(b"../outside", b"rejected")]),
        ),
    ] {
        qa_platform::files::write(root.0.join(name), data).unwrap();
        let count = vfs.mounts().count();
        assert!(
            vfs.refresh_published_file(parent, name.as_bytes(), None)
                .is_err()
        );
        assert_eq!(vfs.mounts().count(), count);
        let winning = vfs.open(b"shared").unwrap();
        assert_eq!(vfs.origin(&winning).unwrap().mount, parent);
        assert_eq!(
            vfs.read_to_vec(&winning, 4096, &mut Default::default())
                .unwrap(),
            b"retained"
        );
        assert!(vfs.open(b"outside").is_none());
    }
    qa_platform::files::write(root.0.join("plain"), b"plain bytes").unwrap();
    assert_eq!(
        vfs.refresh_published_file(parent, b"plain", None).unwrap(),
        None
    );
    assert_eq!(vfs.mounts().count(), 1);
    let mut reader = Default::default();
    assert_eq!(
        vfs.read_to_vec(&retained, 4096, &mut reader).unwrap(),
        b"retained"
    );
    for suffix in [b"pak".as_slice(), b"pk3", b"pkz", b"kpf", b"zip"] {
        assert!(qa_formats::archive::ArchiveKind::from_path(suffix).is_none());
        let mut path = b"packs/test.".to_vec();
        path.extend_from_slice(suffix);
        assert!(qa_formats::archive::ArchiveKind::from_path(&path).is_some());
    }
}

#[test]
fn independent_search_views_select_cached_candidates_without_mutating_other_callers() {
    let root = Fixture::new("search-views");
    let mut vfs = Vfs::default();
    let mut mounts = vec![];
    for (name, data) in [("first", b"first".as_slice()), ("peer", b"peer")] {
        let path = root.0.join(name);
        qa_platform::files::create_dir_all(&path).unwrap();
        qa_platform::files::write(path.join("same.dat"), data).unwrap();
        qa_platform::files::write(path.join("unique.dat"), name.as_bytes()).unwrap();
        mounts.push(vfs.mount_directory(&path, mounts.len() as i32).unwrap());
    }
    let default = vfs.open(b"same.dat").unwrap();
    let live = vfs.read_view(None).retain().unwrap();
    assert!(live.same_selection(&vfs.default_view()));
    assert!(live.same_selection(&live.clone()));
    let first = vfs.search_view([mounts[0]]).unwrap();
    let peer = vfs.search_view([mounts[1], mounts[0], mounts[1]]).unwrap();
    let reversed = vfs.search_view([mounts[0], mounts[1]]).unwrap();
    let empty = vfs.search_view([]).unwrap();
    assert!(first.same_selection(&vfs.read_view(Some(&first)).retain().unwrap()));
    assert!(first.same_selection(&vfs.search_view([mounts[0]]).unwrap()));
    assert!(!first.same_selection(&reversed));
    assert!(!live.same_selection(&peer));
    assert!(!live.same_selection(&empty));
    assert_eq!(
        vfs.open_with(b"same.dat", Some(&live)),
        Some(default.clone())
    );
    let mut reader = Default::default();
    for (view, expected) in [
        (&first, b"first".as_slice()),
        (&peer, b"peer"),
        (&reversed, b"first"),
    ] {
        let reference = vfs.open_with(b"SAME.DAT", Some(view)).unwrap();
        assert_eq!(
            vfs.read_to_vec(&reference, 4096, &mut reader).unwrap(),
            expected
        );
        let listed: Vec<_> = vfs.files_with(Some(view)).collect();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].1, b"same.dat");
        assert_eq!(listed[1].1, b"unique.dat");
        assert_eq!(listed[0].0, reference);
    }
    assert!(vfs.open_with(b"same.dat", Some(&empty)).is_none());
    assert_eq!(vfs.files_with(Some(&empty)).count(), 0);
    assert_eq!(vfs.files_with(None).count(), 2);
    assert_eq!(vfs.files().count(), 4);
    assert_eq!(vfs.open(b"same.dat"), Some(default.clone()));
    let other = Vfs::default();
    let foreign = other.search_view([]).unwrap();
    assert!(!live.same_selection(&other.default_view()));
    assert!(matches!(
        vfs.read_view(Some(&foreign)).retain(),
        Err(VfsError::Handle)
    ));
    assert!(!vfs.accepts_search(&foreign));
    assert!(vfs.open_with(b"same.dat", Some(&foreign)).is_none());
    assert_eq!(vfs.files_with(Some(&foreign)).count(), 0);
    // A later mount cannot silently enter an already admitted caller view.
    let path = root.0.join("late");
    qa_platform::files::create_dir_all(&path).unwrap();
    qa_platform::files::write(path.join("same.dat"), b"late").unwrap();
    qa_platform::files::write(path.join("late.dat"), b"new").unwrap();
    let late = vfs.mount_directory(&path, 99).unwrap();
    assert!(vfs.open_with(b"late.dat", Some(&live)).is_some());
    assert_eq!(vfs.files_with(Some(&live)).count(), 3);
    assert!(first.same_selection(&vfs.search_view([mounts[0]]).unwrap()));
    assert!(vfs.open(b"late.dat").is_some());
    assert!(vfs.open_with(b"late.dat", Some(&peer)).is_none());
    assert_eq!(vfs.files_with(Some(&peer)).count(), 2);
    let refreshed = vfs.search_view([late, mounts[0]]).unwrap();
    let current = vfs.open_with(b"same.dat", Some(&refreshed)).unwrap();
    assert_eq!(
        vfs.read_to_vec(&current, 4096, &mut reader).unwrap(),
        b"late"
    );
    assert_eq!(
        vfs.read_to_vec(&default, 4096, &mut reader).unwrap(),
        b"peer"
    );
    vfs.unmount(mounts[0]);
    assert!(vfs.open_with(b"same.dat", Some(&first)).is_none());
    assert_eq!(vfs.files_with(Some(&first)).count(), 0);
    assert!(vfs.search_view([mounts[0]]).is_err());
    assert!(vfs.open_with(b"same.dat", Some(&peer)).is_some());
    for (name, data, expected) in [
        (
            "duplicates.pak",
            pack(&[(b"same.dat", b"first"), (b"SAME.DAT", b"last")]),
            b"first".as_slice(),
        ),
        (
            "duplicates.pk3",
            zip::stored_zip(&[(b"same.dat", b"first"), (b"SAME.DAT", b"last")]),
            b"last",
        ),
    ] {
        let path = root.0.join(name);
        qa_platform::files::write(&path, data).unwrap();
        let archive = Arc::new(Archive::parse(Arc::new(ReadFile::open(&path).unwrap())).unwrap());
        let mount = vfs.mount_archive(&path, 100, archive).unwrap();
        let view = vfs.search_view([mount]).unwrap();
        let reference = vfs.open_with(b"same.dat", Some(&view)).unwrap();
        assert_eq!(
            vfs.read_to_vec(&reference, 4096, &mut reader).unwrap(),
            expected
        );
    }
}

#[test]
fn server_product_files_select_loose_sources_from_the_named_product() {
    let fixture = Fixture::new("server-products");
    let low = fixture.0.join("low/baseq3");
    let high = fixture.0.join("high/baseq3");
    let other = fixture.0.join("mod");
    for root in [&low, &high, &other] {
        qa_platform::files::create_dir_all(root).unwrap();
    }
    for (root, bytes) in [
        (&low, b"low".as_slice()),
        (&high, b"high"),
        (&other, b"other"),
    ] {
        qa_platform::files::write(&root.join("custom.pk3"), bytes).unwrap();
    }
    let mut vfs = Vfs::default();
    vfs.mount_directory(&low, 1).unwrap();
    vfs.mount_directory(&high, 2).unwrap();
    vfs.mount_directory(&other, 3).unwrap();
    let package = fixture.0.join("impostor.pk3");
    qa_platform::files::write(&package, &zip::stored_zip(&[(b"custom.pk3", b"member")])).unwrap();
    vfs.mount_archive(
        &package,
        4,
        Arc::new(Archive::parse(Arc::new(ReadFile::open(&package).unwrap())).unwrap()),
    )
    .unwrap();
    let read = |file: FileRef| {
        vfs.read_to_vec(&file, 1024, &mut Default::default())
            .unwrap()
    };
    assert_eq!(read(vfs.open(b"custom.pk3").unwrap()), b"member");
    assert_eq!(
        read(vfs.open_product_file(b"baseq3/custom.pk3").unwrap()),
        b"high"
    );
    assert_eq!(
        read(vfs.open_product_file(b"mod/custom.pk3").unwrap()),
        b"other"
    );
    for path in [
        b"unknown/custom.pk3".as_slice(),
        b"baseq3/../custom.pk3",
        b"../custom.pk3",
        b"baseq3",
        b"/custom.pk3",
    ] {
        assert!(vfs.open_product_file(path).is_none());
    }
}

#[test]
fn mounted_q3_metadata_survives_empty_archives_and_retires_with_the_mount() {
    let fixture = Fixture::new("native-package-metadata");
    let package = fixture.0.join("empty.pk3");
    qa_platform::files::write(&package, &zip::stored_zip(&[(b"empty", b"")])).unwrap();
    let mut vfs = Vfs::default();
    let archive = Arc::new(Archive::parse(Arc::new(ReadFile::open(&package).unwrap())).unwrap());
    let retained = Arc::downgrade(&archive);
    let expected = archive.q3_checksums().unwrap().checksum();
    let first = vfs.mount_archive(&package, 1, archive).unwrap();
    assert_eq!(
        vfs.q3_package_checksums(first).unwrap().checksum(),
        expected
    );
    let retained_file = vfs.open(b"empty").unwrap();
    let empty = fixture.0.join("no-members.pk3");
    qa_platform::files::write(&empty, &zip::stored_zip(&[])).unwrap();
    let empty = vfs
        .mount_archive(
            &empty,
            1,
            Arc::new(Archive::parse(Arc::new(ReadFile::open(&empty).unwrap())).unwrap()),
        )
        .unwrap();
    assert_eq!(vfs.files_in_mount(empty).count(), 0);
    assert!(vfs.q3_package_checksums(empty).is_some());
    let directory = vfs.mount_directory(&fixture.0, 0).unwrap();
    assert!(vfs.q3_package_checksums(directory).is_none());
    let pak = fixture.0.join("native.pak");
    qa_platform::files::write(&pak, pack(&[(b"entry", b"bytes")])).unwrap();
    let pak = vfs
        .mount_archive(
            &pak,
            2,
            Arc::new(Archive::parse(Arc::new(ReadFile::open(&pak).unwrap())).unwrap()),
        )
        .unwrap();
    assert!(vfs.q3_package_checksums(pak).is_none());
    assert!(vfs.unmount(first));
    assert!(vfs.q3_package_checksums(first).is_none());
    assert!(retained.upgrade().is_some());
    drop(retained_file);
    assert!(retained.upgrade().is_none());
    assert!(vfs.q3_package_checksums(empty).is_some());
}

#[test]
fn staged_package_admission_keeps_the_inspected_inode_and_rejects_bad_member_paths() {
    let root = Fixture::new("staged-package");
    let mut vfs = Vfs::default();
    let parent = vfs.mount_directory(&root.0, 0).unwrap();
    let original = zip::stored_zip(&[(b"models/kept", b"validated source")]);
    let mut staged =
        qa_platform::files::StagedFile::create(&root.0, "kept.pk3", 4096, 4096).unwrap();
    staged.append(&original).unwrap();
    let package = PreparedArchive::read(Arc::new(staged.reader().unwrap())).unwrap();
    assert!(!root.0.join("kept.pk3").exists());
    staged.publish().unwrap();
    // Renaming a replacement over the filename cannot redirect the inspected
    // archive's retained source while its publication is being indexed.
    qa_platform::files::write(root.0.join("replacement"), b"not the inspected package").unwrap();
    qa_platform::files::rename(root.0.join("replacement"), root.0.join("kept.pk3")).unwrap();
    let mount = vfs
        .refresh_published_file(parent, b"kept.pk3", Some(&package))
        .unwrap()
        .unwrap();
    drop(package);
    let file = vfs.open(b"models/kept").unwrap();
    assert_eq!(vfs.origin(&file).unwrap().mount, mount);
    assert_eq!(
        vfs.read_to_vec(&file, 64, &mut Default::default()).unwrap(),
        b"validated source"
    );
    let mut bad = qa_platform::files::StagedFile::create(&root.0, "bad.pk3", 4096, 4096).unwrap();
    bad.append(&zip::stored_zip(&[(b"../outside", b"rejected")]))
        .unwrap();
    assert!(matches!(
        PreparedArchive::read(Arc::new(bad.reader().unwrap())),
        Err(VfsError::Path)
    ));
    drop(bad);
    assert!(!root.0.join("bad.pk3").exists());
    assert_eq!(vfs.mounts().count(), 2);
}

#[test]
fn loaded_listing_keeps_source_order_scopes_empty_directories_and_explicit_refreshes() {
    let root = Fixture::new("loaded-listing");
    let low = root.0.join("low");
    let high = root.0.join("high");
    for path in [&low, &high] {
        qa_platform::files::create_dir_all(path.join("maps")).unwrap();
        qa_platform::files::create_dir_all(path.join("models/players")).unwrap();
    }
    qa_platform::files::create_dir_all(low.join("models/players/empty-low")).unwrap();
    qa_platform::files::create_dir_all(high.join("models/players/empty-high")).unwrap();
    qa_platform::files::create_dir_all(high.join("models/players/zip-only")).unwrap();
    for (path, name) in [
        (&low, "a.bsp"),
        (&low, "z.bsp"),
        (&high, "a.bsp"),
        (&high, "b.bsp"),
    ] {
        qa_platform::files::write(path.join("maps").join(name), name).unwrap();
    }
    let mut vfs = Vfs::default();
    let low_mount = vfs.mount_directory(&low, 0).unwrap();
    vfs.mount_directory(&high, 10).unwrap();
    let package = root.0.join("pak0.pk3");
    qa_platform::files::write(
        &package,
        zip::stored_zip(&[
            (b"maps/last.bsp", b"last"),
            (b"maps/deep/nested.bsp", b"nested"),
            (b"maps/first.bsp", b"first"),
            (b"maps/deep/", b""),
            (b"maps/a.bsp", b"a"),
            (b"maps/deep/more/too-deep.bsp", b"deep"),
            (b"models/players/zip-only/", b""),
        ]),
    )
    .unwrap();
    vfs.mount_archive(
        &package,
        20,
        Arc::new(Archive::parse(Arc::new(ReadFile::open(&package).unwrap())).unwrap()),
    )
    .unwrap();
    let view = vfs.search_view([low_mount]).unwrap();
    let mut listing = FileListing::default();
    listing.prepare(&vfs).unwrap();
    let list = |listing: &mut FileListing,
                vfs: &Vfs,
                view: Option<&SearchView>,
                path: &[u8],
                extension: &[u8]| {
        let mut names = Vec::new();
        let count = listing
            .visit(
                vfs,
                view,
                FileListQuery {
                    path,
                    extension,
                    maximum: 4095,
                },
                |name| {
                    let mut bytes = name.bytes.to_vec();
                    if name.trailing_slash {
                        bytes.push(b'/');
                    }
                    names.push(bytes);
                    true
                },
            )
            .unwrap();
        assert_eq!(count, names.len());
        names
    };
    assert_eq!(
        list(&mut listing, &vfs, None, b"maps", b".BSP"),
        [
            b"last.bsp".as_slice(),
            b"deep/nested.bsp",
            b"first.bsp",
            b"a.bsp",
            b"b.bsp",
            b"z.bsp"
        ]
    );
    assert_eq!(
        list(&mut listing, &vfs, Some(&view), b"maps", b".bsp"),
        [b"a.bsp".as_slice(), b"z.bsp"]
    );
    assert_eq!(
        list(&mut listing, &vfs, None, b"models/players", b"/"),
        [
            b"zip-only/".as_slice(),
            b"empty-high",
            b"zip-only",
            b"empty-low"
        ]
    );
    assert_eq!(
        list(&mut listing, &vfs, None, b"maps", b""),
        [
            b"last.bsp".as_slice(),
            b"deep/nested.bsp",
            b"first.bsp",
            b"deep/",
            b"a.bsp",
            b"b.bsp",
            b"z.bsp"
        ]
    );
    assert_eq!(
        list(&mut listing, &vfs, None, b"maps/", b".bsp"),
        [
            b"last.bsp".as_slice(),
            b"deep/nested.bsp",
            b"first.bsp",
            b"a.bsp",
            b"deep/more/too-deep.bsp",
            b"b.bsp",
            b"z.bsp"
        ]
    );
    let live = vfs.default_view();
    assert_eq!(
        list(&mut listing, &vfs, Some(&live), b"maps", b".bsp"),
        list(&mut listing, &vfs, None, b"maps", b".bsp")
    );
    qa_platform::files::create_dir_all(low.join("models/players/arrived")).unwrap();
    assert_eq!(
        list(&mut listing, &vfs, Some(&view), b"models/players", b"/"),
        [b"empty-low".as_slice()]
    );
    vfs.refresh_directory_prefix(low_mount, b"models/players")
        .unwrap();
    listing.prepare(&vfs).unwrap();
    assert_eq!(
        list(&mut listing, &vfs, Some(&view), b"models/players", b"/"),
        [b"arrived".as_slice(), b"empty-low"]
    );
    vfs.unmount(low_mount);
    assert!(list(&mut listing, &vfs, Some(&view), b"models/players", b"/").is_empty());
}

#[test]
fn retained_file_search_handles_keep_explicit_orders_and_reject_other_vfs_owners() {
    let root = Fixture::new("retained-search-handles");
    let mut vfs = Vfs::default();
    let mut mounts = Vec::new();
    for (priority, text) in [(0, b"low".as_slice()), (1, b"high")] {
        let path = root.0.join(priority.to_string());
        qa_platform::files::create_dir_all(&path).unwrap();
        qa_platform::files::write(path.join("same.txt"), text).unwrap();
        mounts.push(vfs.mount_directory(&path, priority).unwrap());
    }
    let default = vfs.bind_search(None).unwrap();
    assert_eq!(default.id(), None);
    assert!(default.view().same_selection(&vfs.default_view()));
    assert!(vfs.file_search(None).unwrap().is_none());
    let low = vfs.search_view([mounts[0]]).unwrap();
    let first = vfs.bind_search(Some(&low)).unwrap();
    let low_id = first.id().unwrap();
    let repeated = vfs
        .bind_search(Some(&vfs.search_view([mounts[0], mounts[0]]).unwrap()))
        .unwrap();
    assert_eq!(repeated.id(), Some(low_id));
    let high = vfs.search_view([mounts[1]]).unwrap();
    let second = vfs.bind_search(Some(&high)).unwrap();
    assert_ne!(second.id(), Some(low_id));
    drop(first);
    drop(low);
    let restored = vfs.file_search(Some(low_id)).unwrap().unwrap();
    assert_eq!(
        vfs.read_view(Some(restored.view()))
            .read_file(b"same.txt", 64, &mut Default::default())
            .unwrap(),
        b"low"
    );
    assert_eq!(
        vfs.read_view(Some(restored.view()))
            .mounts()
            .map(|mount| mount.id)
            .collect::<Vec<_>>(),
        [mounts[0]]
    );
    assert_eq!(vfs.read_view(None).mounts().count(), 2);
    assert_eq!(
        vfs.read_view(Some(restored.view()))
            .files()
            .map(|(_, path)| path.to_vec())
            .collect::<Vec<_>>(),
        [b"same.txt".to_vec()]
    );
    let foreign = Vfs::default();
    assert!(foreign.file_search(Some(low_id)).is_err());
    assert!(foreign.bind_search(Some(restored.view())).is_err());
    let foreign_bound = foreign
        .bind_search(Some(&foreign.search_view([]).unwrap()))
        .unwrap();
    assert!(vfs.file_search(foreign_bound.id()).is_err());
    assert_eq!(
        vfs.read_view(Some(foreign_bound.view())).mounts().count(),
        0
    );
    assert_eq!(vfs.read_view(Some(foreign_bound.view())).files().count(), 0);
    vfs.unmount(mounts[0]);
    assert_eq!(vfs.read_view(Some(restored.view())).mounts().count(), 0);

    assert_eq!(vfs.bind_search(Some(default.view())).unwrap().id(), None);
}

#[test]
fn q3_pure_reads_keep_native_order_loose_exceptions_and_unfiltered_probes() {
    let root = Fixture::new("q3-pure-read-policy");
    let loose = root.0.join("loose");
    qa_platform::files::create_dir_all(&loose).unwrap();
    for name in [
        "config.CFG",
        "menu.menu",
        "rules.game",
        "record.dm_68",
        "journal.dat",
        "song.ogg",
        "effect.wav",
        "old.dm_67",
        "config.cfg.exe",
    ] {
        qa_platform::files::write(loose.join(name), name.as_bytes()).unwrap();
    }
    let mut vfs = Vfs::default();
    let loose_mount = vfs.mount_directory(&loose, 1000).unwrap();
    let mut mounts = Vec::new();
    for (name, priority, entries) in [
        (
            "first",
            10,
            vec![
                (b"models/shared.md3".as_slice(), b"first".as_slice()),
                (b"models/first.md3", b"first"),
            ],
        ),
        (
            "second",
            20,
            vec![
                (b"models/shared.md3".as_slice(), b"second".as_slice()),
                (b"models/second.md3", b"second"),
            ],
        ),
        (
            "denied",
            100,
            vec![
                (b"models/shared.md3".as_slice(), b"denied".as_slice()),
                (b"models/denied.md3", b"denied"),
            ],
        ),
    ] {
        let path = root.0.join(format!("{name}.pk3"));
        qa_platform::files::write(&path, zip::stored_zip(&entries)).unwrap();
        let archive = Arc::new(Archive::parse(Arc::new(ReadFile::open(&path).unwrap())).unwrap());
        let sum = archive.q3_checksums().unwrap().checksum();
        let mount = vfs.mount_archive(&path, priority, archive).unwrap();
        mounts.push((mount, sum));
    }
    let pure = vfs
        .q3_search_view(None, &[mounts[0].1, mounts[1].1], false)
        .unwrap();
    let view = vfs.read_view(Some(&pure));
    let file = view.open(b"models/shared.md3").unwrap();
    assert_eq!(vfs.origin(&file).unwrap().mount, mounts[0].0);
    assert_eq!(
        view.read_to_vec(&file, 32, &mut Default::default())
            .unwrap(),
        b"first"
    );
    assert_eq!(
        vfs.origin(&vfs.open(b"models/shared.md3").unwrap())
            .unwrap()
            .mount,
        mounts[2].0
    );
    for name in [
        b"config.cfg".as_slice(),
        b"menu.menu",
        b"rules.game",
        b"record.dm_68",
        b"journal.dat",
    ] {
        assert_eq!(
            vfs.origin(&view.open(name).unwrap()).unwrap().mount,
            loose_mount
        );
    }
    for name in [
        b"models/denied.md3".as_slice(),
        b"song.ogg",
        b"effect.wav",
        b"old.dm_67",
        b"config.cfg.exe",
    ] {
        assert!(view.open(name).is_none(), "{name:?}");
        assert!(view.query(name).is_some(), "{name:?}");
        assert!(!vfs.files_with(Some(&pure)).any(|(_, path)| path == name));
    }
    let mut listing = FileListing::default();
    listing.prepare(&vfs).unwrap();
    let mut names = Vec::new();
    listing
        .visit(
            &vfs,
            Some(&pure),
            FileListQuery {
                path: b"models",
                extension: b".md3",
                maximum: 4095,
            },
            |name| {
                names.push(name.bytes.to_vec());
                true
            },
        )
        .unwrap();
    assert_eq!(
        names,
        [b"shared.md3".as_slice(), b"first.md3", b"second.md3"]
    );
    names.clear();
    // Original Q3 listings restrict archives, but enumerate loose filenames
    // even when an actual read of the listed extension would be denied.
    listing
        .visit(
            &vfs,
            Some(&pure),
            FileListQuery {
                path: b"",
                extension: b".ogg",
                maximum: 4095,
            },
            |name| {
                names.push(name.bytes.to_vec());
                true
            },
        )
        .unwrap();
    assert_eq!(names, [b"song.ogg"]);
    let reversed = vfs
        .q3_search_view(None, &[mounts[1].1, mounts[0].1], false)
        .unwrap();
    assert_eq!(
        vfs.origin(
            &vfs.open_with(b"models/shared.md3", Some(&reversed))
                .unwrap()
        )
        .unwrap()
        .mount,
        mounts[1].0
    );
    assert!(!reversed.same_selection(&pure));
    let only_first = vfs.search_view([mounts[0].0, loose_mount]).unwrap();
    let narrowed = vfs
        .q3_search_view(Some(&only_first), &[mounts[1].1], false)
        .unwrap();
    assert!(
        vfs.open_with(b"models/shared.md3", Some(&narrowed))
            .is_none()
    );
    assert!(
        vfs.query_with(b"models/shared.md3", Some(&narrowed))
            .is_some()
    );
    assert!(
        vfs.query_with(b"models/second.md3", Some(&narrowed))
            .is_none()
    );
    let ordinary = vfs.q3_search_view(None, &[], false).unwrap();
    assert!(ordinary.same_selection(&vfs.default_view()));
    assert!(vfs.open_with(b"song.ogg", Some(&ordinary)).is_some());
    let demo = vfs.q3_search_view(None, &[], true).unwrap();
    assert!(vfs.open_with(b"models/denied.md3", Some(&demo)).is_some());
    assert!(vfs.open_with(b"song.ogg", Some(&demo)).is_none());
    assert!(vfs.open_with(b"config.cfg", Some(&demo)).is_some());
    let unrestricted_order = vfs
        .search_view([mounts[0].0, mounts[1].0, mounts[2].0, loose_mount])
        .unwrap();
    assert!(!unrestricted_order.same_selection(&pure));
    assert_ne!(
        vfs.bind_search(Some(&unrestricted_order)).unwrap().id(),
        vfs.bind_search(Some(&pure)).unwrap().id()
    );
    let foreign = Vfs::default();
    assert!(matches!(
        foreign.q3_search_view(Some(&pure), &[], true),
        Err(VfsError::Handle)
    ));
    assert!(foreign.query_with(b"config.cfg", Some(&pure)).is_none());
    assert!(vfs.unmount(mounts[0].0));
    assert_eq!(
        vfs.origin(&vfs.open_with(b"models/shared.md3", Some(&pure)).unwrap())
            .unwrap()
            .mount,
        mounts[1].0
    );
}

#[test]
fn repeated_pure_checksums_admit_each_mount_without_merging_source_identity() {
    let root = Fixture::new("q3-pure-duplicate-metadata");
    let path = root.0.join("same.pk3");
    qa_platform::files::write(&path, zip::stored_zip(&[(b"model.md3", b"same")])).unwrap();
    let mut vfs = Vfs::default();
    let archive = Arc::new(Archive::parse(Arc::new(ReadFile::open(&path).unwrap())).unwrap());
    let sum = archive.q3_checksums().unwrap().checksum();
    let low = vfs.mount_archive(&path, 1, Arc::clone(&archive)).unwrap();
    let high = vfs.mount_archive(&path, 2, archive).unwrap();
    let pure = vfs.q3_search_view(None, &[sum, sum, sum], false).unwrap();
    assert_eq!(
        vfs.origin(&vfs.open_with(b"model.md3", Some(&pure)).unwrap())
            .unwrap()
            .mount,
        high
    );
    assert!(vfs.unmount(high));
    assert_eq!(
        vfs.origin(&vfs.open_with(b"model.md3", Some(&pure)).unwrap())
            .unwrap()
            .mount,
        low
    );
}

#[test]
fn q3_references_follow_actual_selected_opens_and_native_report_order() {
    use qa_core::{random::Random, text::FixedText};
    let root = Fixture::new("q3-references");
    let mut vfs = Vfs::default();
    let mut mounts = Vec::new();
    for (name, entries) in [
        (
            "modules",
            vec![
                (b"vm/cgame.qvm".as_slice(), b"client".as_slice()),
                (b"vm/ui.qvm", b"ui"),
                (b"vm/qagame.qvm", b"server"),
            ],
        ),
        (
            "world",
            vec![(b"textures/wall.tga".as_slice(), b"texture".as_slice())],
        ),
        (
            "scripts",
            vec![
                (b"scripts/fx.shader".as_slice(), b"shader".as_slice()),
                (b"readme.txt", b"text"),
                (b"setup.cfg", b"cfg"),
                (b"game.config", b"config"),
                (b"a.bot", b"bot"),
                (b"a.arena", b"arena"),
                (b"a.menu", b"menu"),
                (b"levelshots/map.tga", b"levelshot"),
            ],
        ),
    ] {
        let path = root.0.join(format!("{name}.pk3"));
        qa_platform::files::write(&path, zip::stored_zip(&entries)).unwrap();
        let archive = Arc::new(Archive::parse(Arc::new(ReadFile::open(&path).unwrap())).unwrap());
        let id = vfs.mount_archive(&path, 0, archive).unwrap();
        mounts.push(id);
    }
    let feed = 0x8abb_ccdd;
    let module_word = vfs
        .q3_package_checksums(mounts[0])
        .unwrap()
        .pure_checksum(feed);
    let world_word = vfs
        .q3_package_checksums(mounts[1])
        .unwrap()
        .pure_checksum(feed);
    let order = vfs.search_view([mounts[1], mounts[0], mounts[2]]).unwrap();
    let tracked = vfs
        .track_q3_references(Some(&order), feed, Random::new(1))
        .unwrap();
    let peer = vfs
        .track_q3_references(Some(&order), feed, Random::new(1))
        .unwrap();
    assert!(!peer.same_selection(&tracked));
    assert_ne!(
        vfs.bind_search(Some(&peer)).unwrap().id(),
        vfs.bind_search(Some(&tracked)).unwrap().id()
    );
    let view = vfs.read_view(Some(&tracked));
    let mut report = FixedText::<8191>::default();
    let empty = format!("cp 9 @ {}", feed as i32);
    assert!(view.query(b"vm/cgame.qvm").is_some());
    let enumerated = vfs.files_with(Some(&tracked)).count();
    assert!(enumerated > 0);
    let mut listing = FileListing::default();
    listing.prepare(&vfs).unwrap();
    listing
        .visit(
            &vfs,
            Some(&tracked),
            FileListQuery {
                path: b"vm",
                extension: b".qvm",
                maximum: 64,
            },
            |_| true,
        )
        .unwrap();
    tracked
        .q3_reference_command(&vfs, 9, false, &mut report)
        .unwrap();
    assert_eq!(report.as_bytes(), empty.as_bytes());
    assert!(matches!(
        tracked.q3_reference_command(&vfs, 9, true, &mut report),
        Err(VfsError::Missing)
    ));
    for (_, name) in vfs.files_in_mount(mounts[2]) {
        assert!(view.open(name).is_some());
    }
    tracked
        .q3_reference_command(&vfs, 9, false, &mut report)
        .unwrap();
    assert_eq!(report.as_bytes(), empty.as_bytes());
    let cgame = view.open(b"vm/cgame.qvm").unwrap();
    assert_eq!(
        view.read_to_vec(&cgame, 64, &mut Default::default())
            .unwrap(),
        b"client"
    );
    assert!(view.open(b"vm/ui.qvm").is_some());
    assert!(view.open(b"vm/qagame.qvm").is_some());
    // Resource loaders may retain enumerated numeric file references. The first
    // actual read still participates, without another name lookup or reader.
    let wall = vfs.files_in_mount(mounts[1]).next().unwrap().0;
    assert_eq!(
        view.read_to_vec(&wall, 64, &mut Default::default())
            .unwrap(),
        b"texture"
    );
    assert!(view.open(b"textures/wall.tga").is_some());
    tracked
        .q3_reference_command(&vfs, 9, true, &mut report)
        .unwrap();
    let expected = format!(
        "cp 9 {} {} @ {} {} {}",
        module_word as i32,
        module_word as i32,
        world_word as i32,
        module_word as i32,
        (feed ^ world_word ^ module_word ^ 2) as i32
    );
    assert_eq!(report.as_bytes(), expected.as_bytes());
    let sums: Vec<_> = mounts
        .iter()
        .map(|&id| vfs.q3_package_checksums(id).unwrap().checksum())
        .collect();
    let reordered = vfs.q3_search_view(Some(&tracked), &sums, false).unwrap();
    reordered
        .q3_reference_command(&vfs, 9, true, &mut report)
        .unwrap();
    assert_eq!(
        report.as_bytes(),
        format!(
            "cp 9 {} {} @ {} {} {}",
            module_word as i32,
            module_word as i32,
            module_word as i32,
            world_word as i32,
            (feed ^ world_word ^ module_word ^ 2) as i32
        )
        .as_bytes()
    );
    peer.q3_reference_command(&vfs, 9, false, &mut report)
        .unwrap();
    assert_eq!(report.as_bytes(), empty.as_bytes());
    let peer_view = vfs.read_view(Some(&peer));
    assert!(peer_view.open(b"vm/cgame.qvm").is_some());
    assert!(peer_view.open(b"vm/ui.qvm").is_some());
    peer.q3_reference_command(&vfs, 9, true, &mut report)
        .unwrap();
    let peer_report = report.as_str().to_owned();
    tracked.clear_q3_renderer_references(&vfs).unwrap();
    assert!(matches!(
        tracked.q3_reference_command(&vfs, 9, true, &mut report),
        Err(VfsError::Missing)
    ));
    tracked
        .q3_reference_command(&vfs, 9, false, &mut report)
        .unwrap();
    assert_eq!(
        report.as_str(),
        format!(
            "cp 9 @ {} {} {}",
            world_word as i32,
            module_word as i32,
            (feed ^ world_word ^ module_word ^ 2) as i32
        )
    );
    peer.q3_reference_command(&vfs, 9, true, &mut report)
        .unwrap();
    assert_eq!(report.as_str(), peer_report);
    assert!(matches!(
        tracked.clear_q3_renderer_references(&Vfs::default()),
        Err(VfsError::Handle)
    ));
    assert!(view.open(b"vm/cgame.qvm").is_some());
    assert!(matches!(
        tracked.q3_reference_command(&vfs, 9, true, &mut report),
        Err(VfsError::Missing)
    ));
    assert!(view.open(b"vm/ui.qvm").is_some());
    tracked
        .q3_reference_command(&vfs, 9, true, &mut report)
        .unwrap();
    assert_eq!(report.as_str(), expected);
    let mut short = FixedText::<4>::default();
    assert!(matches!(
        tracked.q3_reference_command(&vfs, 9, true, &mut short),
        Err(VfsError::Limit)
    ));
    assert!(short.as_bytes().is_empty());
    assert!(matches!(
        tracked.q3_reference_command(&Vfs::default(), 9, true, &mut report),
        Err(VfsError::Handle)
    ));
    assert!(vfs.unmount(mounts[0]));
    assert!(matches!(
        tracked.q3_reference_command(&vfs, 9, true, &mut report),
        Err(VfsError::Missing)
    ));
    tracked
        .q3_reference_command(&vfs, 9, false, &mut report)
        .unwrap();
    assert_eq!(
        report.as_bytes(),
        format!(
            "cp 9 @ {} {}",
            world_word as i32,
            (feed ^ world_word ^ 1) as i32
        )
        .as_bytes()
    );
}

#[test]
fn q3_loose_open_wire_marker_reuses_the_shared_random_stream_once_per_open() {
    use qa_core::{
        random::{Algorithm, Random},
        text::FixedText,
    };
    let root = Fixture::new("q3-loose-reference");
    qa_platform::files::write(root.0.join("config.cfg"), b"settings").unwrap();
    qa_platform::files::write(root.0.join("texture.tga"), b"loose").unwrap();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let random = Random::with_algorithm(1, Algorithm::GlibcType3);
    let mut expected = random.clone();
    let tracked = vfs.track_q3_references(None, 37, random).unwrap();
    let view = vfs.read_view(Some(&tracked));
    assert_eq!(
        view.read_file(b"config.cfg", 64, &mut Default::default())
            .unwrap(),
        b"settings"
    );
    let mut report = FixedText::<8191>::default();
    tracked
        .q3_reference_command(&vfs, 5, false, &mut report)
        .unwrap();
    assert_eq!(report.as_bytes(), b"cp 5 @ 37");
    let loose = view.open(b"texture.tga").unwrap();
    for _ in 0..3 {
        assert_eq!(
            view.read_to_vec(&loose, 64, &mut Default::default())
                .unwrap(),
            b"loose"
        );
    }
    let first = expected.next_u32() as i32;
    tracked
        .q3_reference_command(&vfs, 5, false, &mut report)
        .unwrap();
    assert_eq!(
        report.as_bytes(),
        format!("cp 5 {first} {first} @ {first} 37").as_bytes()
    );
    view.read_file(b"texture.tga", 64, &mut Default::default())
        .unwrap();
    let second = expected.next_u32() as i32;
    tracked
        .q3_reference_command(&vfs, 5, false, &mut report)
        .unwrap();
    assert_eq!(
        report.as_bytes(),
        format!("cp 5 {second} {second} @ {second} 37").as_bytes()
    );
}

#[test]
fn retained_numeric_file_reads_enforce_the_selected_namespace_and_policy() {
    let root = Fixture::new("scoped-numeric-read-policy");
    qa_platform::files::write(root.0.join("asset.tga"), b"retained").unwrap();
    let mut vfs = Vfs::default();
    let mount = vfs.mount_directory(&root.0, 0).unwrap();
    let retained = vfs.open(b"asset.tga").unwrap();
    let selected = vfs.search_view([mount]).unwrap();
    let denied = vfs.q3_search_view(Some(&selected), &[123], false).unwrap();
    let empty = vfs.search_view([]).unwrap();
    let foreign = Vfs::default().search_view([]).unwrap();
    for selection in [&denied, &empty, &foreign] {
        assert!(matches!(
            vfs.read_view(Some(selection))
                .read_to_vec(&retained, 64, &mut Default::default()),
            Err(VfsError::Handle)
        ));
        let mut bytes = [0x5a; 64];
        assert!(matches!(
            vfs.read_view(Some(selection)).read_whole_into(
                &retained,
                &mut bytes,
                &mut Default::default()
            ),
            Err(VfsError::Handle)
        ));
        assert_eq!(bytes, [0x5a; 64]);
    }
    assert_eq!(
        vfs.read_view(Some(&selected))
            .read_to_vec(&retained, 64, &mut Default::default())
            .unwrap(),
        b"retained"
    );
    // The source remains the accepted inode, including after publication of a
    // replacement. Checking policy must not redirect an existing native handle.
    qa_platform::files::write(root.0.join("new"), b"replacement").unwrap();
    qa_platform::files::rename(root.0.join("new"), root.0.join("asset.tga")).unwrap();
    vfs.refresh_directory_prefix(mount, b"").unwrap();
    assert_eq!(
        vfs.read_view(Some(&selected))
            .read_to_vec(&retained, 64, &mut Default::default())
            .unwrap(),
        b"retained"
    );
    assert_eq!(
        vfs.read_view(Some(&selected))
            .read_file(b"asset.tga", 64, &mut Default::default())
            .unwrap(),
        b"replacement"
    );
}

#[test]
fn retained_names_support_owner_checked_reload_after_source_retirement() {
    let root = Fixture::new("retained-module-name");
    qa_platform::files::write(root.0.join("module.qvm"), b"original").unwrap();
    let mut vfs = Vfs::default();
    let mount = vfs.mount_directory(&root.0, 0).unwrap();
    let file = vfs.open(b"module.qvm").unwrap();
    let source = vfs.search_view([mount]).unwrap();
    assert!(vfs.unmount(mount));
    assert!(matches!(vfs.length(&file), Err(VfsError::Handle)));
    assert_eq!(
        vfs.read_view(Some(&source)).retained_name(&file).unwrap(),
        b"module.qvm"
    );
    qa_platform::files::write(root.0.join("module.qvm"), b"new").unwrap();
    vfs.mount_directory(&root.0, 10).unwrap();
    let view = vfs.read_view(None);
    assert_eq!(
        view.read_file(
            view.retained_name(&file).unwrap(),
            64,
            &mut Default::default()
        )
        .unwrap(),
        b"new"
    );
    let foreign = Vfs::default();
    assert!(matches!(
        foreign.read_view(None).retained_name(&file),
        Err(VfsError::Handle)
    ));
    let foreign_source = foreign.default_view();
    assert!(matches!(
        vfs.read_view(Some(&foreign_source)).retained_name(&file),
        Err(VfsError::Handle)
    ));
}

#[test]
fn native_pure_server_metadata_uses_admitted_archive_order_without_loose_shadow_or_references() {
    let files = Fixture::new("server-pure-metadata");
    let loose = files.0.join("loose");
    qa_platform::files::create_dir_all(loose.join("vm")).unwrap();
    qa_platform::files::write(loose.join("vm/ui.qvm"), b"loose ui").unwrap();
    let mut vfs = Vfs::default();
    let directory = vfs.mount_directory(&loose, 100).unwrap();
    let mut packs = Vec::new();
    let feed = 0x8123_4567;
    for (name, priority, entries) in [
        (
            "first",
            1,
            vec![
                (b"vm/ui.qvm".as_slice(), b"first ui".as_slice()),
                (b"vm/cgame.qvm", b"first cgame"),
            ],
        ),
        (
            "second",
            2,
            vec![(b"vm/cgame.qvm".as_slice(), b"second cgame".as_slice())],
        ),
    ] {
        let path = files.0.join(format!("{name}.pk3"));
        qa_platform::files::write(&path, zip::stored_zip(&entries)).unwrap();
        let archive = Arc::new(Archive::parse(Arc::new(ReadFile::open(&path).unwrap())).unwrap());
        let checks = archive.q3_checksums().unwrap();
        let regular = checks.checksum();
        let pure = checks.pure_checksum(feed);
        packs.push((
            vfs.mount_archive(&path, priority, archive).unwrap(),
            regular,
            pure,
        ));
    }
    let selected = vfs
        .search_view([directory, packs[0].0, packs[1].0])
        .unwrap();
    let tracked = vfs
        .track_q3_references(Some(&selected), feed, qa_core::random::Random::new(1))
        .unwrap();
    let view = vfs.read_view(Some(&tracked));
    assert_eq!(
        view.q3_loaded_pure_checksums(feed).unwrap(),
        [packs[0].2, packs[1].2]
    );
    assert_eq!(
        vfs.origin(&view.query(b"vm/ui.qvm").unwrap())
            .unwrap()
            .mount,
        directory
    );
    for name in [b"vm/ui.qvm".as_slice(), b"vm/cgame.qvm"] {
        assert_eq!(view.q3_file_pure_checksum(name, feed), Some(packs[0].2));
    }
    let policy = vfs
        .q3_search_view(Some(&tracked), &[packs[1].1], false)
        .unwrap();
    let admitted = vfs.read_view(Some(&policy));
    assert_eq!(
        admitted.q3_loaded_pure_checksums(feed).unwrap(),
        [packs[1].2]
    );
    assert_eq!(
        admitted.q3_file_pure_checksum(b"vm/cgame.qvm", feed),
        Some(packs[1].2)
    );
    assert_eq!(admitted.q3_file_pure_checksum(b"vm/ui.qvm", feed), None);
    let mut command = qa_core::text::FixedText::<8191>::default();
    tracked
        .q3_reference_command(&vfs, 7, false, &mut command)
        .unwrap();
    assert_eq!(command.as_str(), format!("cp 7 @ {}", feed as i32));
    assert!(
        Vfs::default()
            .read_view(Some(&tracked))
            .q3_loaded_pure_checksums(feed)
            .is_err()
    );
    assert!(vfs.unmount(packs[0].0));
    let view = vfs.read_view(Some(&tracked));
    assert_eq!(view.q3_loaded_pure_checksums(feed).unwrap(), [packs[1].2]);
    assert_eq!(view.q3_file_pure_checksum(b"vm/ui.qvm", feed), None);
}

#[test]
fn source_revision_changes_on_empty_mount_refresh_and_retirement_without_content_probes() {
    let root = Fixture::new("source-revision");
    let mut vfs = Vfs::default();
    let initial = vfs.revision();
    let mount = vfs.mount_directory(&root.0, 0).unwrap();
    let mounted = vfs.revision();
    assert!(mounted > initial);
    vfs.refresh_directory_prefix(mount, b"").unwrap();
    let refreshed = vfs.revision();
    assert!(refreshed > mounted);
    assert!(vfs.unmount(mount));
    assert!(vfs.revision() > refreshed);
    let retired = vfs.revision();
    assert!(!vfs.unmount(mount));
    assert_eq!(vfs.revision(), retired);
    assert_eq!(vfs.take_lookup_count(), 0);
}

#[test]
fn q3_server_package_info_uses_selected_native_order_and_actual_open_references() {
    use qa_core::{
        random::Random,
        text::{InfoBytes, InfoRules, info_value},
    };
    let files = Fixture::new("server-package-info");
    for game in ["baseq3", "missionpack"] {
        qa_platform::files::create_dir_all(files.0.join(game)).unwrap();
    }
    for (game, pak, name, data) in [
        (
            "baseq3",
            "pak0",
            b"maps/native.bsp".as_slice(),
            b"map".as_slice(),
        ),
        ("baseq3", "pak1", b"scripts/native.shader", b"shader"),
        ("baseq3", "pak2", b"vm/qagame.qvm", b"game"),
        ("missionpack", "pak0", b"sound/extra.wav", b"sound"),
    ] {
        qa_platform::files::write(
            files.0.join(game).join(format!("{pak}.pk3")),
            zip::stored_zip(&[(name, data)]),
        )
        .unwrap();
    }
    let mut vfs = Vfs::default();
    let base = vfs.mount_product(&files.0.join("baseq3"), 0).unwrap();
    let expansion = vfs.mount_product(&files.0.join("missionpack"), 1).unwrap();
    let order = [expansion[1], base[3], base[2], base[1]];
    let selected = vfs.search_view(order).unwrap();
    let tracked = vfs
        .track_q3_references(Some(&selected), 17, Random::new(1))
        .unwrap();
    let peer = vfs
        .track_q3_references(Some(&selected), 17, Random::new(2))
        .unwrap();
    let rules = InfoRules {
        bytes: 8192,
        key_bytes: 8192,
        value_bytes: 8192,
        filter: InfoBytes::Exact,
    };
    let mut info = [0; 8192];
    let mut mounts = Vec::with_capacity(vfs.mounts().count());
    let stale = b"\\sv_serverid\\9\\sv_paks\\stale\\sv_pakNames\\stale";
    info[..stale.len()].copy_from_slice(stale);
    let mut length = stale.len();
    let value =
        |info: &[u8], key: &[u8]| info_value(info, key, true).map(|range| info[range].to_vec());
    let signed = |ids: &[MountId]| {
        ids.iter()
            .map(|&id| {
                format!(
                    "{} ",
                    vfs.q3_package_checksums(id).unwrap().checksum() as i32
                )
            })
            .collect::<String>()
    };
    let checksums = signed(&order);
    let view = vfs.read_view(Some(&tracked));
    let before = view.q3_reference_revision().unwrap();
    view.q3_package_info(true, &mut info, &mut length, rules, &mut mounts)
        .unwrap();
    assert_eq!(
        value(&info[..length], b"sv_paks").unwrap(),
        checksums.as_bytes()
    );
    assert_eq!(
        value(&info[..length], b"sv_pakNames").unwrap(),
        b"pak0 pak2 pak1 pak0"
    );
    assert_eq!(
        value(&info[..length], b"sv_referencedPaks").unwrap(),
        signed(&[expansion[1]]).as_bytes()
    );
    assert_eq!(
        value(&info[..length], b"sv_referencedPakNames").unwrap(),
        b"missionpack/pak0   "
    );
    assert_eq!(value(&info[..length], b"sv_serverid").unwrap(), b"9");
    // Reports, enumeration, and probes must not turn an unused base pack into
    // a required download. Opens use the same per-account reference primitive.
    assert!(view.query(b"vm/qagame.qvm").is_some());
    assert!(view.open(b"scripts/native.shader").is_some());
    assert_eq!(view.q3_reference_revision().unwrap(), before);
    assert!(view.open(b"maps/native.bsp").is_some());
    assert!(view.q3_reference_revision().unwrap() > before);
    let opened = view.q3_reference_revision().unwrap();
    assert!(view.open(b"maps/native.bsp").is_some());
    assert_eq!(view.q3_reference_revision().unwrap(), opened);
    assert_eq!(
        vfs.read_view(Some(&peer)).q3_reference_revision().unwrap(),
        before
    );
    assert!(view.open(b"vm/qagame.qvm").is_some());
    view.q3_package_info(false, &mut info, &mut length, rules, &mut mounts)
        .unwrap();
    assert!(value(&info[..length], b"sv_paks").is_none());
    assert!(value(&info[..length], b"sv_pakNames").is_none());
    assert_eq!(
        value(&info[..length], b"sv_referencedPaks").unwrap(),
        signed(&[expansion[1], base[3], base[1]]).as_bytes()
    );
    assert_eq!(
        value(&info[..length], b"sv_referencedPakNames").unwrap(),
        b"missionpack/pak0 baseq3/pak2  baseq3/pak0"
    );
    vfs.read_view(Some(&peer))
        .q3_package_info(true, &mut info, &mut length, rules, &mut mounts)
        .unwrap();
    assert_eq!(
        value(&info[..length], b"sv_referencedPaks").unwrap(),
        signed(&[expansion[1]]).as_bytes()
    );
    let base_checksum = vfs.q3_package_checksums(base[1]).unwrap().checksum();
    let restricted = vfs
        .q3_search_view(Some(&tracked), &[base_checksum], false)
        .unwrap();
    vfs.read_view(Some(&restricted))
        .q3_package_info(true, &mut info, &mut length, rules, &mut mounts)
        .unwrap();
    assert_eq!(
        value(&info[..length], b"sv_paks").unwrap(),
        signed(&[base[1]]).as_bytes()
    );
    assert_eq!(value(&info[..length], b"sv_pakNames").unwrap(), b"pak0");
    assert!(
        Vfs::default()
            .read_view(Some(&tracked))
            .q3_package_info(true, &mut info, &mut length, rules, &mut mounts)
            .is_err()
    );
    let mut tiny = [0; 32];
    assert!(matches!(
        view.q3_package_info(
            true,
            &mut tiny,
            &mut 0,
            InfoRules { bytes: 32, ..rules },
            &mut mounts
        ),
        Err(VfsError::Limit)
    ));
    assert!(vfs.unmount(base[3]));
    vfs.read_view(Some(&tracked))
        .q3_package_info(true, &mut info, &mut length, rules, &mut mounts)
        .unwrap();
    assert_eq!(
        value(&info[..length], b"sv_pakNames").unwrap(),
        b"pak0 pak1 pak0"
    );
}
