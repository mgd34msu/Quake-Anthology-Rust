use qa_content::vfs::Vfs;
use qa_render::material::load_catalog;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn same_name_with_different_resolved_images_retains_distinct_numeric_materials() {
    use qa_render::{
        Assets,
        assets::{MaterialSettings, Stage, StageTexture},
    };
    let mut assets = Assets::load();
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
