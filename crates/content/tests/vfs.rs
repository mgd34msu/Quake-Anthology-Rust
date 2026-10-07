use qa_content::vfs::*;
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
    for path in [
        b"".as_slice(),
        b"/maps/a",
        b"maps//a",
        b"../a",
        b"maps/./a",
        b"C:\\a",
        b"a\0b",
    ] {
        assert!(normalize(path).is_err());
    }
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
    std::fs::write(&path, b"firstsecondlast").unwrap();
    let file = Arc::new(File::open(&path).unwrap());
    let mut vfs = Vfs::default();
    vfs.mount_ranges(
        &path,
        MountKind::Pak,
        1,
        Arc::clone(&file),
        [
            FileRange {
                name: b"A".to_vec(),
                offset: 0,
                length: 5,
            },
            FileRange {
                name: b"a".to_vec(),
                offset: 5,
                length: 6,
            },
        ],
    )
    .unwrap();
    let mut buffer = [0; 8];
    let first = vfs.open(b"a").unwrap();
    assert_eq!(vfs.read_at(first, 0, &mut buffer).unwrap(), 5);
    assert_eq!(&buffer[..5], b"first");
    vfs.mount_ranges(
        &path,
        MountKind::Pk3,
        2,
        file,
        [FileRange {
            name: b"a".to_vec(),
            offset: 11,
            length: 4,
        }],
    )
    .unwrap();
    let last = vfs.open(b"A").unwrap();
    assert_eq!(vfs.read_at(last, 0, &mut buffer).unwrap(), 4);
    assert_eq!(&buffer[..4], b"last");
    assert_eq!(vfs.origin(last).unwrap().kind, MountKind::Pk3);
}
