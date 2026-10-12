use qa_platform::files::{self, EntryKind, ReadFile};
use std::{
    io::ErrorKind,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "qa-platform-files-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        files::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        files::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn shared_handles_read_independent_ranges_without_a_cursor() {
    let root = Fixture::new();
    let path = root.0.join("ranges.bin");
    files::write(&path, b"0123456789").unwrap();
    let file = Arc::new(ReadFile::open(&path).unwrap());
    let second = Arc::clone(&file);
    let mut bytes = [0; 4];
    file.read_exact_at(6, &mut bytes).unwrap();
    assert_eq!(&bytes, b"6789");
    second.read_exact_at(1, &mut bytes).unwrap();
    assert_eq!(&bytes, b"1234");
    assert_eq!(file.length().unwrap(), 10);
    assert_eq!(file.read_at(10, &mut bytes).unwrap(), 0);
    assert_eq!(&bytes, b"1234");
    file.read_exact_at(u64::MAX, &mut []).unwrap();
}

#[test]
fn exact_reads_report_truncation_and_preserve_the_written_prefix() {
    let root = Fixture::new();
    let path = root.0.join("truncated.bin");
    files::write(&path, b"before").unwrap();
    let file = ReadFile::open(&path).unwrap();
    files::write(&path, b"ab").unwrap();
    let mut bytes = [b'?'; 4];
    assert_eq!(
        file.read_exact_at(0, &mut bytes).unwrap_err().kind(),
        ErrorKind::UnexpectedEof
    );
    assert_eq!(&bytes, b"ab??");
    assert_eq!(file.length().unwrap(), 2);
}

#[test]
fn directory_records_keep_native_name_order_and_entry_kinds() {
    let root = Fixture::new();
    files::write(root.0.join("z.bin"), b"z").unwrap();
    files::write(root.0.join("A.bin"), b"a").unwrap();
    files::create_dir_all(root.0.join("middle")).unwrap();
    let entries = files::directory_entries(&root.0).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.name.as_os_str())
            .collect::<Vec<_>>(),
        ["A.bin", "middle", "z.bin"].map(std::ffi::OsStr::new)
    );
    assert_eq!(
        entries.iter().map(|entry| entry.kind).collect::<Vec<_>>(),
        [EntryKind::File, EntryKind::Directory, EntryKind::File]
    );
    for entry in entries {
        assert_eq!(entry.path, root.0.join(entry.name));
    }
    assert_eq!(
        files::directory_entries(root.0.join("missing"))
            .err()
            .unwrap()
            .kind(),
        ErrorKind::NotFound
    );
}

#[cfg(unix)]
#[test]
fn a_mount_root_follows_its_symlink_but_descendants_do_not() {
    use qa_content::vfs::Vfs;
    use std::os::unix::fs::symlink;

    let root = Fixture::new();
    let actual = root.0.join("actual");
    files::create_dir_all(actual.join("maps")).unwrap();
    files::write(actual.join("maps/base1.bsp"), b"world").unwrap();
    files::write(actual.join("real.cfg"), b"settings").unwrap();
    symlink("real.cfg", actual.join("alias.cfg")).unwrap();
    symlink("maps", actual.join("alias_maps")).unwrap();
    let mounted = root.0.join("root-link");
    symlink(&actual, &mounted).unwrap();
    let records = files::directory_entries(&mounted).unwrap();
    for name in ["alias.cfg", "alias_maps"] {
        assert_eq!(
            records
                .iter()
                .find(|entry| entry.name == name)
                .unwrap()
                .kind,
            EntryKind::Other
        );
    }
    let mut vfs = Vfs::default();
    vfs.mount_directory(&mounted, 0).unwrap();
    let map = vfs.open(b"maps/base1.bsp").unwrap();
    assert_eq!(
        vfs.origin(map).unwrap().path,
        files::canonicalize(&actual).unwrap()
    );
    assert!(vfs.open(b"alias.cfg").is_none());
    assert!(vfs.open(b"alias_maps/base1.bsp").is_none());
}

#[test]
fn whole_file_text_reads_keep_raw_bytes_and_utf8_errors() {
    let root = Fixture::new();
    let path = root.0.join("profile.cfg");
    files::write(&path, b"set name \"player\"\n").unwrap();
    assert_eq!(
        files::read_to_string(&path).unwrap(),
        "set name \"player\"\n"
    );
    files::write(&path, [0xff, 0, b'a']).unwrap();
    assert_eq!(files::read(&path).unwrap(), [0xff, 0, b'a']);
    assert_eq!(
        files::read_to_string(&path).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
    assert_eq!(
        ReadFile::open(root.0.join("missing")).err().unwrap().kind(),
        ErrorKind::NotFound
    );
}
