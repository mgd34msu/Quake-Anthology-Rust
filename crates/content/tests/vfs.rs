use qa_content::vfs::*;
use qa_formats::archive::Archive;
use qa_platform::files::ReadFile;
use std::sync::Arc;

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
            .refresh_published_file(parent, name.as_bytes())
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
            vfs.refresh_published_file(parent, name.as_bytes()).unwrap(),
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
        .refresh_published_file(parent, b"packs/new.pk3")
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
        assert!(vfs.refresh_published_file(parent, name.as_bytes()).is_err());
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
    assert_eq!(vfs.refresh_published_file(parent, b"plain").unwrap(), None);
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
    let first = vfs.search_view([mounts[0]]).unwrap();
    let peer = vfs.search_view([mounts[1], mounts[0], mounts[1]]).unwrap();
    let reversed = vfs.search_view([mounts[0], mounts[1]]).unwrap();
    let empty = vfs.search_view([]).unwrap();
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
    }
    assert!(vfs.open_with(b"same.dat", Some(&empty)).is_none());
    assert_eq!(vfs.open(b"same.dat"), Some(default.clone()));
    let other = Vfs::default();
    let foreign = other.search_view([]).unwrap();
    assert!(!vfs.accepts_search(&foreign));
    assert!(vfs.open_with(b"same.dat", Some(&foreign)).is_none());
    // A later mount cannot silently enter an already admitted caller view.
    let path = root.0.join("late");
    qa_platform::files::create_dir_all(&path).unwrap();
    qa_platform::files::write(path.join("same.dat"), b"late").unwrap();
    qa_platform::files::write(path.join("late.dat"), b"new").unwrap();
    let late = vfs.mount_directory(&path, 99).unwrap();
    assert!(vfs.open(b"late.dat").is_some());
    assert!(vfs.open_with(b"late.dat", Some(&peer)).is_none());
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
