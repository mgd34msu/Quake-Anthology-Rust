use qa_content::vfs::*;
use qa_formats::archive::Archive;
use std::{fs::File, sync::Arc};

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("qa-rust-vfs-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
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
    let root = Fixture::new("profile-filter");
    for name in [
        "q1/id1/view.JSON",
        "config.cfg",
        "maps/level.bsp",
        "assets/hidden.json",
        "SaVeS/hidden.cfg",
    ] {
        let path = root.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"saved").unwrap();
    }
    let product = root.0.join("product");
    std::fs::create_dir_all(product.join("q1/id1")).unwrap();
    std::fs::write(product.join("q1/id1/view.JSON"), b"impostor").unwrap();
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
    assert_eq!(vfs.read_at(saved, 0, &mut bytes).unwrap(), 5);
    assert_eq!(&bytes[..5], b"saved");
    assert_eq!(vfs.read_at(winning, 0, &mut bytes).unwrap(), 8);
    assert_eq!(&bytes, b"impostor");
    assert!(vfs.unmount(profile));
    assert!(vfs.files_in_mount(profile).next().is_none());
    assert!(vfs.read_at(saved, 0, &mut bytes).is_err());
}

#[test]
fn three_products_and_mod_share_one_index_and_numeric_reads_keep_origin() {
    let root = Fixture::new("products");
    let mut vfs = Vfs::default();
    for (product, map) in [("id1", "e1m1"), ("baseq2", "base1"), ("baseq3", "q3dm1")] {
        let directory = root.0.join(product);
        std::fs::create_dir_all(directory.join("maps")).unwrap();
        std::fs::write(directory.join("maps").join(format!("{map}.bsp")), map).unwrap();
        vfs.mount_directory(&directory, 0).unwrap();
    }
    let reference = vfs.open(b"MAPS\\E1M1.BSP").unwrap();
    assert_eq!(vfs.take_lookup_count(), 1);
    let mut buffer = [0; 16];
    for _ in 0..10_000 {
        assert_eq!(vfs.read_at(reference, 0, &mut buffer).unwrap(), 4);
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
    std::fs::create_dir_all(mod_dir.join("maps")).unwrap();
    std::fs::write(mod_dir.join("maps/E1M1.BSP"), b"mod").unwrap();
    let mod_id = vfs.mount_directory(&mod_dir, 100).unwrap();
    let modified = vfs.open(b"maps/e1m1.bsp").unwrap();
    assert_ne!(reference, modified);
    assert_eq!(vfs.read_at(modified, 0, &mut buffer).unwrap(), 3);
    assert_eq!(&buffer[..3], b"mod");
    assert!(vfs.unmount(mod_id));
    assert!(vfs.read_at(modified, 0, &mut buffer).is_err());
    assert_eq!(vfs.open(b"maps/e1m1.bsp"), Some(reference));
}

#[test]
fn higher_archive_priority_wins_but_first_duplicate_inside_one_archive_wins() {
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
    let skin = b"models/monsters/tank/../ctank/skin.pcx";
    std::fs::write(
        &path,
        pack(&[(b"A", b"first"), (b"a", b"second"), (skin, b"skin")]),
    )
    .unwrap();
    let file = Arc::new(File::open(&path).unwrap());
    let mut vfs = Vfs::default();
    vfs.mount_archive(&path, 1, Arc::new(Archive::parse(file).unwrap()))
        .unwrap();
    let mut buffer = [0; 8];
    let first = vfs.open(b"a").unwrap();
    assert_eq!(vfs.read_at(first, 0, &mut buffer).unwrap(), 5);
    assert_eq!(&buffer[..5], b"first");
    let skin_ref = vfs.open(skin).unwrap();
    assert_eq!(vfs.open(b"models/monsters/ctank/skin.pcx"), Some(skin_ref));
    assert_eq!(vfs.origin(skin_ref).unwrap().member, skin);
    assert_eq!(vfs.read_at(skin_ref, 0, &mut buffer).unwrap(), 4);
    assert_eq!(&buffer[..4], b"skin");
    let bad_path = root.0.join("escape.pak");
    std::fs::write(&bad_path, pack(&[(b"models/../../escape", b"bad")])).unwrap();
    let bad = Archive::parse(Arc::new(File::open(&bad_path).unwrap())).unwrap();
    assert!(vfs.mount_archive(&bad_path, 100, Arc::new(bad)).is_err());
    assert_eq!(vfs.mounts().count(), 1);
    let next = root.0.join("pak1.pak");
    std::fs::write(&next, pack(&[(b"a", b"last")])).unwrap();
    let next_file = Arc::new(File::open(&next).unwrap());
    vfs.mount_archive(&next, 2, Arc::new(Archive::parse(next_file).unwrap()))
        .unwrap();
    let last = vfs.open(b"A").unwrap();
    assert_eq!(vfs.read_at(last, 0, &mut buffer).unwrap(), 4);
    assert_eq!(&buffer[..4], b"last");
    assert_eq!(vfs.origin(last).unwrap().kind, MountKind::Pak);
}
