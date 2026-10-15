use qa_content::vfs::*;
use qa_formats::archive::Archive;
use qa_platform::files::ReadFile;
use std::sync::Arc;

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

#[test]
fn retail_kpf_package_uses_the_same_archive_reader_and_localization_catalog() {
    let Some(root) = std::env::var_os("QA_Q2_RERELEASE_KPF_ROOT") else {
        return;
    };
    let mut vfs = Vfs::default();
    let mounts = vfs.mount_product(std::path::Path::new(&root), 0).unwrap();
    assert!(mounts.len() > 1);
    let file = vfs.open(b"localization/loc_english.txt").unwrap();
    let origin = vfs.origin(file).unwrap();
    assert!(origin.path.ends_with("Q2Game.kpf"));
    assert_eq!(origin.kind, MountKind::Pk3);
    let mut reader = qa_formats::archive::ArchiveReader::default();
    let bytes = vfs.read_to_vec(file, 8 * 1024 * 1024, &mut reader).unwrap();
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
        vfs.read_range_reusing(saved, 0, &mut bytes, &mut archive_reader)
            .unwrap(),
        5
    );
    assert_eq!(&bytes[..5], b"saved");
    assert_eq!(
        vfs.read_range_reusing(winning, 0, &mut bytes, &mut archive_reader)
            .unwrap(),
        8
    );
    assert_eq!(&bytes, b"impostor");
    assert!(vfs.unmount(profile));
    assert!(vfs.files_in_mount(profile).next().is_none());
    assert!(
        vfs.read_range_reusing(saved, 0, &mut bytes, &mut archive_reader)
            .is_err()
    );
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
            vfs.read_range_reusing(reference, 0, &mut buffer, &mut archive_reader)
                .unwrap(),
            4
        );
        assert_eq!(&buffer[..4], b"e1m1");
    }
    assert_eq!(vfs.take_lookup_count(), 0);
    assert_eq!(vfs.mounts().count(), 3);
    assert_eq!(vfs.files().count(), 3);
    assert!(vfs.origin(reference).unwrap().path.ends_with("id1"));
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
        vfs.read_range_reusing(modified, 0, &mut buffer, &mut archive_reader)
            .unwrap(),
        3
    );
    assert_eq!(&buffer[..3], b"mod");
    assert!(vfs.unmount(mod_id));
    assert!(
        vfs.read_range_reusing(modified, 0, &mut buffer, &mut archive_reader)
            .is_err()
    );
    assert_eq!(vfs.open(b"maps/e1m1.bsp"), Some(reference));
}

#[test]
fn archive_priority_and_native_duplicate_member_order_are_preserved() {
    let mut archive_reader = qa_formats::archive::ArchiveReader::default();
    let root = Fixture::new("ranges");
    let path = root.0.join("pak0.pak");
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
    fn stored_zip(entries: &[(&[u8], &[u8])]) -> Vec<u8> {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut crc = u32::MAX;
            for &byte in bytes {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    let mask = 0u32.wrapping_sub(crc & 1);
                    crc = (crc >> 1) ^ (0xedb8_8320 & mask);
                }
            }
            !crc
        }

        let mut bytes = Vec::new();
        let mut directory = Vec::new();
        for &(name, data) in entries {
            let offset = u32::try_from(bytes.len()).unwrap();
            let name_length = u16::try_from(name.len()).unwrap();
            let data_length = u32::try_from(data.len()).unwrap();
            let crc = crc32(data);
            let mut local = [0; 30];
            local[..4].copy_from_slice(&0x0403_4b50u32.to_le_bytes());
            local[4..6].copy_from_slice(&20u16.to_le_bytes());
            local[14..18].copy_from_slice(&crc.to_le_bytes());
            local[18..22].copy_from_slice(&data_length.to_le_bytes());
            local[22..26].copy_from_slice(&data_length.to_le_bytes());
            local[26..28].copy_from_slice(&name_length.to_le_bytes());
            bytes.extend(local);
            bytes.extend(name);
            bytes.extend(data);

            let mut central = [0; 46];
            central[..4].copy_from_slice(&0x0201_4b50u32.to_le_bytes());
            central[4..6].copy_from_slice(&20u16.to_le_bytes());
            central[6..8].copy_from_slice(&20u16.to_le_bytes());
            central[16..20].copy_from_slice(&crc.to_le_bytes());
            central[20..24].copy_from_slice(&data_length.to_le_bytes());
            central[24..28].copy_from_slice(&data_length.to_le_bytes());
            central[28..30].copy_from_slice(&name_length.to_le_bytes());
            central[42..46].copy_from_slice(&offset.to_le_bytes());
            directory.extend(central);
            directory.extend(name);
        }
        let directory_offset = u32::try_from(bytes.len()).unwrap();
        let directory_length = u32::try_from(directory.len()).unwrap();
        bytes.extend(directory);
        let count = u16::try_from(entries.len()).unwrap();
        let mut end = [0; 22];
        end[..4].copy_from_slice(&0x0605_4b50u32.to_le_bytes());
        end[8..10].copy_from_slice(&count.to_le_bytes());
        end[10..12].copy_from_slice(&count.to_le_bytes());
        end[12..16].copy_from_slice(&directory_length.to_le_bytes());
        end[16..20].copy_from_slice(&directory_offset.to_le_bytes());
        bytes.extend(end);
        bytes
    }
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
        vfs.read_range_reusing(first, 0, &mut buffer, &mut archive_reader)
            .unwrap(),
        5
    );
    assert_eq!(&buffer[..5], b"first");
    assert_eq!(vfs.origin(first).unwrap().member, b"A");
    let exact = vfs.open(b"EXACT").unwrap();
    assert_eq!(
        vfs.read_range_reusing(exact, 0, &mut buffer, &mut archive_reader)
            .unwrap(),
        3
    );
    assert_eq!(&buffer[..3], b"old");
    let mut reader = qa_formats::archive::ArchiveReader::default();
    assert_eq!(vfs.read_to_vec(first, 5, &mut reader).unwrap(), b"first");
    assert_eq!(vfs.read_file(b"A", 5, &mut reader).unwrap(), b"first");
    assert!(matches!(
        vfs.read_to_vec(first, 4, &mut reader),
        Err(VfsError::Limit)
    ));
    let skin_ref = vfs.open(skin).unwrap();
    assert_eq!(vfs.open(b"models/monsters/ctank/skin.pcx"), Some(skin_ref));
    assert_eq!(vfs.origin(skin_ref).unwrap().member, skin);
    assert_eq!(
        vfs.read_range_reusing(skin_ref, 0, &mut buffer, &mut archive_reader)
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
        vfs.read_range_reusing(last, 0, &mut buffer, &mut archive_reader)
            .unwrap(),
        4
    );
    assert_eq!(&buffer[..4], b"last");
    assert_eq!(vfs.origin(last).unwrap().kind, MountKind::Pak);
    let listing: Vec<_> = vfs
        .files()
        .map(|(reference, name)| (reference, name.to_vec()))
        .collect();
    let duplicates: Vec<_> = listing
        .iter()
        .filter(|(_, name)| name == b"a")
        .map(|(reference, _)| *reference)
        .collect();
    assert_eq!(duplicates.len(), 3);
    assert_eq!(&duplicates[..2], [last, first]);
    assert!(vfs.unmount(next_mount));
    assert_eq!(vfs.open(b"A"), Some(first));
    assert_eq!(vfs.files().count(), 5);
    assert!(
        vfs.read_range_reusing(last, 0, &mut buffer, &mut archive_reader)
            .is_err()
    );

    let zip_path = root.0.join("duplicates.kpf");
    qa_platform::files::write(
        &zip_path,
        stored_zip(&[
            (b"Case-Zip", b"case first"),
            (b"CASE-ZIP", b"case last"),
            (b"exact-zip", b"exact first"),
            (b"exact-zip", b"exact last"),
            (b"priority-zip", b"priority first"),
            (b"PRIORITY-ZIP", b"priority last"),
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
        .map(|(reference, _)| *reference)
        .collect();
    let exact_members: Vec<_> = members
        .iter()
        .filter(|(_, name)| name == b"exact-zip")
        .map(|(reference, _)| *reference)
        .collect();
    let priority_members: Vec<_> = members
        .iter()
        .filter(|(_, name)| name == b"priority-zip")
        .map(|(reference, _)| *reference)
        .collect();
    assert_eq!(case_members.len(), 2);
    assert_eq!(exact_members.len(), 2);
    assert_eq!(priority_members.len(), 2);
    assert_eq!(zip_vfs.origin(case_members[0]).unwrap().member, b"Case-Zip");
    assert_eq!(zip_vfs.origin(case_members[1]).unwrap().member, b"CASE-ZIP");
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
    assert_eq!(case_listing, case_members);
    assert_eq!(exact_listing, exact_members);
    assert_eq!(zip_vfs.take_lookup_count(), 0);
    let case_winner = zip_vfs.open(b"case-zip").unwrap();
    assert_eq!(zip_vfs.take_lookup_count(), 1);
    assert_eq!(case_winner, case_members[1]);
    assert_eq!(zip_vfs.origin(case_winner).unwrap().kind, MountKind::Pk3);
    assert_eq!(zip_vfs.origin(case_winner).unwrap().member, b"CASE-ZIP");
    let mut zip_buffer = [0; 16];
    assert_eq!(
        zip_vfs
            .read_range_reusing(case_winner, 0, &mut zip_buffer, &mut archive_reader)
            .unwrap(),
        9
    );
    assert_eq!(&zip_buffer[..9], b"case last");
    let exact_winner = zip_vfs.open(b"EXACT-ZIP").unwrap();
    assert_eq!(exact_winner, exact_members[1]);
    assert_eq!(
        zip_vfs
            .read_range_reusing(exact_winner, 0, &mut zip_buffer, &mut archive_reader)
            .unwrap(),
        10
    );
    assert_eq!(&zip_buffer[..10], b"exact last");

    let higher = root.0.join("higher");
    qa_platform::files::create_dir_all(&higher).unwrap();
    qa_platform::files::write(higher.join("priority-zip"), b"higher").unwrap();
    let higher_mount = zip_vfs.mount_directory(&higher, 2).unwrap();
    let priority = zip_vfs.open(b"PRIORITY-ZIP").unwrap();
    assert_eq!(zip_vfs.origin(priority).unwrap().mount, higher_mount);
    assert_eq!(zip_vfs.origin(priority).unwrap().kind, MountKind::Directory);
    assert_eq!(
        zip_vfs
            .read_range_reusing(priority, 0, &mut zip_buffer, &mut archive_reader)
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
    assert_eq!(&priority_listing[1..], priority_members);
    assert!(zip_vfs.unmount(higher_mount));
    let priority = zip_vfs.open(b"priority-zip").unwrap();
    assert_eq!(priority, priority_members[1]);
    assert_eq!(zip_vfs.origin(priority).unwrap().member, b"PRIORITY-ZIP");
    assert_eq!(
        zip_vfs
            .read_range_reusing(priority, 0, &mut zip_buffer, &mut archive_reader)
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
    assert_eq!(vfs.read_to_vec(file, 4, &mut reader).unwrap(), b"abcd");
    assert_eq!(vfs.read_file(b"A", 4, &mut reader).unwrap(), b"abcd");
    let mut destination = [0xaa; 6];
    assert_eq!(
        vfs.read_whole_into(file, &mut destination, &mut reader)
            .unwrap(),
        4
    );
    assert_eq!(&destination, b"abcd\xaa\xaa");
    destination.fill(0xbb);
    assert!(matches!(
        vfs.read_whole_into(file, &mut destination[..3], &mut reader),
        Err(VfsError::Limit)
    ));
    assert_eq!(destination, [0xbb; 6]);
    assert!(matches!(
        vfs.read_to_vec(file, 3, &mut reader),
        Err(VfsError::Limit)
    ));
    assert!(matches!(
        vfs.read_file(b"missing", 4, &mut reader),
        Err(VfsError::Missing)
    ));
    assert!(vfs.read_file(b"empty", 0, &mut reader).unwrap().is_empty());
    assert_eq!(
        vfs.read_whole_into(vfs.open(b"empty").unwrap(), &mut [], &mut reader)
            .unwrap(),
        0
    );
    qa_platform::files::write(root.0.join("a"), b"ab").unwrap();
    assert!(matches!(
        vfs.read_to_vec(file, 4, &mut reader),
        Err(VfsError::Format(qa_formats::FormatError::Truncated))
    ));
}
