#[test]
fn probe_single_file() {
    use qa_tools::inventory::source_census as census;
    let text =
        std::fs::read_to_string("/home/buzzkill/Projects/quake-typescript/verification/source-manifest.json").unwrap();
    let manifest = qa_tools::json::parse_json(&text).unwrap();
    let repos = manifest.get("repositories").unwrap().as_array().unwrap();
    let root = std::path::Path::new("/home/buzzkill/Projects/quake-typescript");
    let spec = census::repositories().into_iter().find(|s| s.id == "q3-ts").unwrap();
    let captured = census::capture_repository(root, &spec).unwrap();
    let actual = captured.to_json();
    let golden = repos
        .iter()
        .find(|r| r.get("id").unwrap().as_str() == Some("q3-ts"))
        .unwrap();
    let target = match std::env::var("PROBE_FILE") {
        Ok(target) => target,
        Err(_) => {
            println!("PROBE_FILE unset; interactive probe skipped");
            return;
        }
    };
    let a_files = actual.get("files").unwrap().as_array().unwrap();
    let g_files = golden.get("files").unwrap().as_array().unwrap();
    let a = a_files
        .iter()
        .find(|f| f.get("path").unwrap().as_str() == Some(&target))
        .unwrap();
    let g = g_files
        .iter()
        .find(|f| f.get("path").unwrap().as_str() == Some(&target))
        .unwrap();
    let af = a.get("functions").unwrap().as_array().unwrap();
    let gf = g.get("functions").unwrap().as_array().unwrap();
    println!("counts: mine={} golden={}", af.len(), gf.len());
    for (i, (x, y)) in af.iter().zip(gf.iter()).enumerate() {
        if x != y {
            println!("first diff at index {i}:");
            println!("  mine:   {}", x.render());
            println!("  golden: {}", y.render());
            return;
        }
    }
    if af.len() != gf.len() {
        println!("length differs; extra:");
        for x in af.iter().skip(gf.len()) {
            println!("  mine extra: {}", x.render());
        }
        for y in gf.iter().skip(af.len()) {
            println!("  golden extra: {}", y.render());
        }
    } else {
        println!("identical");
    }
}
