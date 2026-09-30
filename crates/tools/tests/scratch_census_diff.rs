#[test]
fn census_matches_golden() {
    use qa_tools::inventory::source_census as census;
    let text =
        std::fs::read_to_string("/home/buzzkill/Projects/quake-typescript/verification/source-manifest.json").unwrap();
    let manifest = qa_tools::json::parse_json(&text).unwrap();
    let repos = manifest.get("repositories").unwrap().as_array().unwrap();
    let root = std::path::Path::new("/home/buzzkill/Projects/quake-typescript");
    for spec in census::repositories() {
        let golden = repos
            .iter()
            .find(|r| r.get("id").unwrap().as_str() == Some(&spec.id))
            .unwrap();
        let captured = census::capture_repository(root, &spec).unwrap();
        let actual = captured.to_json();
        // Compare everything except nothing: full deep equality expected.
        if actual != *golden {
            println!("DIFF in {}", spec.id);
            let a_files = actual.get("files").unwrap().as_array().unwrap();
            let g_files = golden.get("files").unwrap().as_array().unwrap();
            println!("  files: mine={} golden={}", a_files.len(), g_files.len());
            for (a, g) in a_files.iter().zip(g_files.iter()) {
                if a != g {
                    let ap = a.get("path").unwrap().as_str().unwrap();
                    println!("  first file diff at {ap}");
                    println!("    mine:   {}", a.render());
                    println!("    golden: {}", g.render());
                    break;
                }
            }
            for key in ["revision", "sourceSetSha256"] {
                if actual.get(key) != golden.get(key) {
                    println!("  {key}: mine={:?} golden={:?}", actual.get(key), golden.get(key));
                }
            }
            if actual.get("submodules") != golden.get("submodules") {
                println!("  submodules differ");
            }
            panic!("census mismatch for {}", spec.id);
        }
        println!("{} matches golden ({} files)", spec.id, captured.files.len());
    }
}
