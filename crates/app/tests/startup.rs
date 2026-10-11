use qa_app::startup::default_game;
use qa_content::vfs::Vfs;
use qa_core::primitives::RuleSetId;
use std::path::Path;

fn product(root: &Path, directory: &str, map: &str) {
    let maps = root.join(directory).join("maps");
    std::fs::create_dir_all(&maps).unwrap();
    std::fs::write(maps.join(format!("{map}.bsp")), b"test start witness").unwrap();
}

#[test]
fn default_product_uses_catalog_priority_and_only_mounts_the_selected_game() {
    let root = std::env::temp_dir().join(format!("qa-default-products-{}", std::process::id()));
    product(&root, "q1/id1", "start");
    product(&root, "q2/baseq2", "base1");
    let mut vfs = Vfs::default();
    let game = default_game(&mut vfs, [&root as &Path]).unwrap();
    assert_eq!(
        (game.product, game.map, game.rules),
        ("q1-classic-id1", "maps/start.bsp", RuleSetId::Quake)
    );
    assert!(vfs.open(b"maps/start.bsp").is_some());
    assert!(vfs.open(b"maps/base1.bsp").is_none());
    drop(vfs);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_mounts_and_rerelease_metadata_win_without_discovery() {
    let root = std::env::temp_dir().join(format!("qa-default-explicit-{}", std::process::id()));
    product(&root, "q1/id1", "start");
    product(&root, "q2/rerelease/baseq2", "base1");
    let mut vfs = Vfs::default();
    vfs.mount_product(&root.join("q2/rerelease/baseq2"), 0)
        .unwrap();
    let game = default_game(&mut vfs, [&root as &Path]).unwrap();
    assert_eq!(
        (game.product, game.map, game.rules),
        (
            "q2-rerelease-baseq2",
            "maps/base1.bsp",
            RuleSetId::Quake2Rerelease
        )
    );
    assert!(vfs.open(b"maps/start.bsp").is_none());
    drop(vfs);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn standalone_base_directory_and_missing_installations_are_distinct() {
    let root = std::env::temp_dir().join(format!("qa-default-standalone-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut vfs = Vfs::default();
    assert!(
        default_game(&mut vfs, [&root as &Path])
            .unwrap_err()
            .contains("no installed game")
    );
    product(&root, "baseq2", "base1");
    let game = default_game(&mut vfs, [&root as &Path]).unwrap();
    assert_eq!(game.rules, RuleSetId::Quake2);
    assert_eq!(game.map, "maps/base1.bsp");
    drop(vfs);
    std::fs::remove_dir_all(root).unwrap();
}
