use qa_content::{products::detect, vfs::Vfs};
#[test]
fn standalone_products_and_two_rogue_packs_keep_their_own_default_sources() {
    let root = std::env::temp_dir().join(format!("qa-rust-product-mix-{}", std::process::id()));
    let recipes = [
        ("id1", "start"),
        ("baseq2", "base1"),
        ("baseq3", "q3dm1"),
        ("q1/rogue", "start"),
        ("q2/rogue", "rmine1"),
    ];
    let mut vfs = Vfs::default();
    for (dir, map) in recipes {
        let path = root.join(dir);
        std::fs::create_dir_all(path.join("maps")).unwrap();
        std::fs::write(path.join(format!("maps/{map}.bsp")), dir).unwrap();
        vfs.mount_directory(&path, 0).unwrap();
    }
    let products = detect(&vfs);
    assert_eq!(products.len(), 6); // Five installations plus the shared QW recipe.
    for product in products {
        let mut content = [0; 32];
        let n = vfs.read_at(product.start_map, 0, &mut content).unwrap();
        let expected = match product.spec().key {
            "q1-classic-id1" | "q1-quakeworld" => "id1",
            "q2-classic-baseq2" => "baseq2",
            "q3-baseq3" => "baseq3",
            "q1-classic-rogue" => "q1/rogue",
            "q2-classic-rogue" => "q2/rogue",
            other => panic!("unexpected {other}"),
        };
        assert_eq!(&content[..n], expected.as_bytes());
    }
    drop(vfs);
    std::fs::remove_dir_all(root).unwrap();
}
