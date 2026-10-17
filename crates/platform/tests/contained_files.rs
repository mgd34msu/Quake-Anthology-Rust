#[cfg(target_os = "linux")]
mod linux {
    use qa_platform::files::StagedFile;
    use std::{
        fs,
        io::ErrorKind,
        os::unix::fs::symlink,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "qa-contained-files-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn create(&self, name: &str, maximum: u64, buffer: usize) -> StagedFile {
            StagedFile::create(&self.0, name, maximum, buffer).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn entries(path: &Path) -> Vec<PathBuf> {
        fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    }

    #[test]
    fn invalid_paths_fail_before_creating_any_components() {
        let root = Fixture::new();
        for name in [
            "",
            "/absolute",
            "one/",
            "one//two",
            ".",
            "..",
            "one/./two",
            "one/../two",
            "one\\two",
            "one:two",
            "one/nu\0l",
        ] {
            assert_eq!(
                StagedFile::create(&root.0, name, 16, 8)
                    .err()
                    .unwrap()
                    .kind(),
                ErrorKind::InvalidInput,
                "{name:?}"
            );
        }
        assert!(entries(&root.0).is_empty());
        assert!(StagedFile::create(&root.0, "one/two", 16, 64 * 1024 * 1024 + 1).is_err());
        assert!(entries(&root.0).is_empty());
    }

    #[test]
    fn streaming_publication_is_exclusive_and_leaves_only_complete_destination() {
        let root = Fixture::new();
        let mut staged = root.create("skins/native.pcx", 12, 3);
        assert!(staged.is_empty());
        staged.append(b"abc").unwrap();
        staged.append(b"defghijkl").unwrap();
        assert_eq!(staged.len(), 12);
        assert!(!root.0.join("skins/native.pcx").exists());
        assert_eq!(staged.publish().unwrap(), 12);
        assert_eq!(
            fs::read(root.0.join("skins/native.pcx")).unwrap(),
            b"abcdefghijkl"
        );
        assert_eq!(entries(&root.0.join("skins")).len(), 1);

        let mut collision = root.create("skins/native.pcx", 8, 3);
        collision.append(b"changed").unwrap();
        assert_eq!(
            collision.publish().unwrap_err().kind(),
            ErrorKind::AlreadyExists
        );
        assert_eq!(
            fs::read(root.0.join("skins/native.pcx")).unwrap(),
            b"abcdefghijkl"
        );
        assert_eq!(entries(&root.0.join("skins")).len(), 1);
        fs::create_dir(root.0.join("skins/directory")).unwrap();
        let mut directory_collision = root.create("skins/directory", 4, 4);
        directory_collision.append(b"ours").unwrap();
        assert!(directory_collision.publish().is_err());
        assert!(root.0.join("skins/directory").is_dir());
        assert!(entries(&root.0.join("skins/directory")).is_empty());
    }

    #[test]
    fn directory_and_destination_symlinks_never_escape_the_root() {
        let root = Fixture::new();
        let outside = Fixture::new();
        fs::write(outside.0.join("existing"), b"outside").unwrap();
        symlink(&outside.0, root.0.join("linked")).unwrap();
        assert!(StagedFile::create(&root.0, "linked/new", 10, 4).is_err());
        assert!(!outside.0.join("new").exists());
        symlink(outside.0.join("existing"), root.0.join("target")).unwrap();
        let mut staged = root.create("target", 4, 4);
        staged.append(b"ours").unwrap();
        assert_eq!(
            staged.publish().unwrap_err().kind(),
            ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(outside.0.join("existing")).unwrap(), b"outside");
        assert!(
            fs::symlink_metadata(root.0.join("target"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let linked_root = root.0.join("root-link");
        symlink(&outside.0, &linked_root).unwrap();
        assert!(StagedFile::create(&linked_root, "new", 4, 4).is_err());
        assert!(!outside.0.join("new").exists());
    }

    #[test]
    fn retained_parent_survives_rename_and_replacement_by_symlink() {
        let root = Fixture::new();
        let outside = Fixture::new();
        let mut staged = root.create("skins/kept", 8, 4);
        staged.append(b"before").unwrap();
        fs::rename(root.0.join("skins"), root.0.join("retained")).unwrap();
        symlink(&outside.0, root.0.join("skins")).unwrap();
        staged.append(b"++").unwrap();
        assert_eq!(staged.publish().unwrap(), 8);
        assert_eq!(fs::read(root.0.join("retained/kept")).unwrap(), b"before++");
        assert!(entries(&outside.0).is_empty());
        assert_eq!(entries(&root.0.join("retained")).len(), 1);
    }

    #[test]
    fn staged_inspection_flushes_and_preserves_the_sequential_write_position() {
        let root = Fixture::new();
        let mut staged = root.create("package", 12, 16);
        staged.append(b"first").unwrap();
        let reader = staged.reader().unwrap();
        assert_eq!(reader.length().unwrap(), 5);
        let mut bytes = [0; 5];
        reader.read_exact_at(0, &mut bytes).unwrap();
        assert_eq!(&bytes, b"first");
        staged.append(b"second").unwrap();
        let later = staged.reader().unwrap();
        let mut bytes = [0; 11];
        later.read_exact_at(0, &mut bytes).unwrap();
        assert_eq!(&bytes, b"firstsecond");
        assert!(!root.0.join("package").exists());
        assert_eq!(staged.publish().unwrap(), 11);
        reader.read_exact_at(0, &mut bytes).unwrap();
        assert_eq!(&bytes, b"firstsecond");
        assert_eq!(fs::read(root.0.join("package")).unwrap(), bytes);
    }

    #[test]
    fn publication_links_the_open_inode_when_the_temporary_name_is_swapped() {
        let root = Fixture::new();
        let outside = Fixture::new();
        fs::write(outside.0.join("foreign"), b"foreign").unwrap();
        let mut staged = root.create("kept", 8, 4);
        staged.append(b"owned").unwrap();
        let temporary = entries(&root.0).pop().unwrap();
        fs::rename(&temporary, root.0.join("moved-stage")).unwrap();
        symlink(outside.0.join("foreign"), &temporary).unwrap();
        let inspected = staged.reader().unwrap();
        let mut bytes = [0; 5];
        inspected.read_exact_at(0, &mut bytes).unwrap();
        assert_eq!(&bytes, b"owned");
        assert_eq!(staged.publish().unwrap(), 5);
        assert_eq!(fs::read(root.0.join("kept")).unwrap(), b"owned");
        assert!(!fs::symlink_metadata(&temporary).is_ok());
        assert_eq!(fs::read(outside.0.join("foreign")).unwrap(), b"foreign");
    }

    #[test]
    fn limit_failure_poison_cancel_and_drop_remove_the_staged_prefix() {
        let root = Fixture::new();
        let mut staged = root.create("skins/too-large", 4, 3);
        staged.append(b"abc").unwrap();
        assert_eq!(
            staged.append(b"de").unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(staged.len(), 3);
        assert_eq!(
            staged.append(b"d").unwrap_err().kind(),
            ErrorKind::BrokenPipe
        );
        assert_eq!(staged.publish().unwrap_err().kind(), ErrorKind::BrokenPipe);
        assert!(entries(&root.0.join("skins")).is_empty());
        let mut canceled = root.create("skins/canceled", 8, 4);
        canceled.append(b"prefix").unwrap();
        canceled.cancel().unwrap();
        assert!(entries(&root.0.join("skins")).is_empty());
        let mut dropped = root.create("skins/dropped", 8, 4);
        dropped.append(b"prefix").unwrap();
        drop(dropped);
        assert!(entries(&root.0.join("skins")).is_empty());
        let empty = root.create("skins/empty", 0, 0);
        assert_eq!(empty.publish().unwrap(), 0);
        assert_eq!(fs::metadata(root.0.join("skins/empty")).unwrap().len(), 0);
    }

    #[test]
    fn concurrent_stages_keep_independent_names_and_only_one_destination_wins() {
        let root = Fixture::new();
        let mut first = root.create("asset", 8, 4);
        let mut second = root.create("asset", 8, 4);
        first.append(b"first").unwrap();
        second.append(b"second").unwrap();
        assert_eq!(entries(&root.0).len(), 2);
        first.publish().unwrap();
        assert_eq!(
            second.publish().unwrap_err().kind(),
            ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(root.0.join("asset")).unwrap(), b"first");
        assert_eq!(entries(&root.0).len(), 1);
    }

    #[test]
    fn late_publication_failure_cleans_a_partially_received_file() {
        let root = Fixture::new();
        let mut staged = root.create("asset", 8, 3);
        staged.append(b"prefix").unwrap();
        let temporary = entries(&root.0).pop().unwrap();
        fs::remove_file(temporary).unwrap();
        assert!(staged.publish().is_err());
        assert!(entries(&root.0).is_empty());
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn contained_staging_reports_unsupported_platforms() {
    assert_eq!(
        qa_platform::files::StagedFile::create(std::path::Path::new("."), "asset", 1, 1)
            .err()
            .unwrap()
            .kind(),
        std::io::ErrorKind::Unsupported
    );
}
