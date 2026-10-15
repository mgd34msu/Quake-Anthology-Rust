use qa_content::vfs::Vfs;
use qa_formats::archive::Archive;
use qa_platform::files::ReadFile;
use qa_render::material::load_catalog;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);

#[path = "../../formats/tests/support/stored_zip.rs"]
mod zip;

#[test]
fn same_name_with_different_resolved_images_retains_distinct_numeric_materials() {
    use qa_render::{
        Assets,
        assets::{MaterialSettings, Stage, StageTexture},
    };
    let mut assets = Assets::load().unwrap();
    let a = assets.register_image(1, 1, &[0, 0, 0, 255]).unwrap();
    let b = assets.register_image(1, 1, &[255; 4]).unwrap();
    let first = assets
        .register_material(
            "wall",
            &[Stage {
                texture: StageTexture::Image(a),
                ..Stage::default()
            }],
            MaterialSettings::default(),
        )
        .unwrap();
    let second = assets
        .register_material(
            "wall",
            &[Stage {
                texture: StageTexture::Image(b),
                ..Stage::default()
            }],
            MaterialSettings::default(),
        )
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(
        assets.material(first).unwrap().name,
        assets.material(second).unwrap().name
    );
    let repeat = assets
        .register_material(
            "WALL",
            assets.material(first).unwrap().stages.to_vec().as_slice(),
            MaterialSettings::default(),
        )
        .unwrap();
    assert_eq!(repeat, first);
    assert_eq!(
        assets.name(assets.material(first).unwrap().name),
        Some("wall")
    );
    assert_eq!(
        assets
            .register_material(
                "wall",
                &[Stage {
                    texture: StageTexture::Image(a),
                    ..Stage::default()
                }],
                MaterialSettings::default()
            )
            .unwrap(),
        first
    );
}

#[test]
fn replaced_shader_file_contributes_only_the_winning_definitions() {
    let root = std::env::temp_dir().join(format!(
        "qa-material-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let low = root.join("low");
    let high = root.join("high");
    std::fs::create_dir_all(low.join("scripts")).unwrap();
    std::fs::create_dir_all(high.join("scripts")).unwrap();
    std::fs::write(
        low.join("scripts/wall.shader"),
        b"old_only { { map old } } shared { { map low } }",
    )
    .unwrap();
    std::fs::write(
        high.join("scripts/wall.shader"),
        b"new_only { { map new } } shared { { map high } }",
    )
    .unwrap();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&low, 0).unwrap();
    vfs.mount_directory(&high, 1).unwrap();
    let catalog = load_catalog(&vfs).unwrap();
    assert!(catalog.find_canonical("old_only").is_none());
    assert!(catalog.find_canonical("new_only").is_some());
    assert_eq!(catalog.definitions.len(), 2);
    assert!(catalog.diagnostics.is_empty());
    drop(vfs);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn duplicate_zip_shader_member_contributes_only_the_last_definitions() {
    let root = std::env::temp_dir().join(format!(
        "qa-material-zip-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("duplicate.pk3");
    std::fs::write(
        &path,
        zip::stored_zip(&[
            (
                b"scripts/wall.shader",
                b"old_only { { map old } } shared { { map low } }",
            ),
            (
                b"scripts/wall.shader",
                b"new_only { { map new } } shared { { map high } }",
            ),
        ]),
    )
    .unwrap();
    let archive = Archive::parse(Arc::new(ReadFile::open(&path).unwrap())).unwrap();
    let mut vfs = Vfs::default();
    vfs.mount_archive(&path, 0, Arc::new(archive)).unwrap();
    let winner = vfs.open(b"scripts/wall.shader").unwrap();
    let catalog = load_catalog(&vfs).unwrap();
    assert!(catalog.find_canonical("old_only").is_none());
    assert!(catalog.find_canonical("new_only").is_some());
    assert!(catalog.find_canonical("shared").is_some());
    assert_eq!(catalog.definitions.len(), 2);
    assert_eq!(
        vfs.files()
            .find(|(_, name)| *name == b"scripts/wall.shader")
            .unwrap()
            .0,
        winner
    );
    drop(vfs);
    std::fs::remove_dir_all(root).unwrap();
}
