use std::collections::HashMap;

const KINDS: [&str; 7] = [
    "FunctionDeclaration", "FunctionExpression", "ArrowFunction", "MethodDeclaration",
    "GetAccessor", "SetAccessor", "Constructor",
];

#[test]
fn fidelity_vs_golden() {
    let text = std::fs::read_to_string("/home/buzzkill/Projects/quake-typescript/verification/source-manifest.json").unwrap();
    let manifest = qa_tools::json::parse_json(&text).unwrap();
    let repos = manifest.get("repositories").unwrap().as_array().unwrap();
    let repo = repos.iter().find(|r| r.get("id").unwrap().as_str() == Some("q1-ts")).unwrap();
    let files = repo.get("files").unwrap().as_array().unwrap();
    let root = "/home/buzzkill/Projects/quake-1-re-ts";
    let mut compared = 0;
    let mut skipped_changed = 0;
    let mut missing = 0;
    let mut extra = 0;
    let mut field_mismatch = 0;
    let mut shown = 0;
    for file in files {
        let obj = file.as_object().unwrap();
        let get = |k: &str| obj.iter().find(|(key, _)| key == k).map(|(_, v)| v).unwrap();
        let path = get("path").as_str().unwrap();
        let want_sha = get("sha256").as_str().unwrap();
        if !(path.ends_with(".ts") || path.ends_with(".tsx")) { continue; }
        let full = format!("{root}/{path}");
        let bytes = match std::fs::read(&full) {
            Ok(b) => b,
            Err(_) => { skipped_changed += 1; continue; }
        };
        if qa_tools::verify::hash::hash_bytes(&bytes) != want_sha {
            skipped_changed += 1;
            continue;
        }
        let source = String::from_utf8_lossy(&bytes).into_owned();
        let scanned = qa_tools::inventory::ts_scan::scan_declarations(&source);
        let starts = qa_tools::js::line_starts(&source);
        let utf16 = qa_tools::js::Utf16Map::new(&source);
        let line_of = |off: usize| starts.partition_point(|s| *s <= off).max(1);
        let mut mine: HashMap<usize, (String, String, usize, usize, usize, bool)> = HashMap::new();
        for decl in &scanned.declarations {
            if !KINDS.contains(&decl.kind) {
                continue;
            }
            let end_line = line_of(decl.start.max(decl.end.saturating_sub(1)));
            mine.insert(utf16.to_utf16(decl.start), (decl.kind.to_owned(), decl.name.clone(), line_of(decl.start), end_line, utf16.to_utf16(decl.end), decl.body.is_some()));
        }
        let golden_fns = get("functions").as_array().unwrap();
        compared += 1;
        for g in golden_fns {
            let gobj = g.as_object().unwrap();
            let gg = |k: &str| gobj.iter().find(|(key, _)| key == k).map(|(_, v)| v).unwrap();
            let start = gg("startOffset").as_f64().unwrap() as usize;
            match mine.remove(&start) {
                None => {
                    missing += 1;
                    if shown < 100000 {
                        shown += 1;
                        println!("MISSING {path}#{} {:?} line={}", start, gg("kind").as_str(), gg("line").as_f64().unwrap());
                    }
                }
                Some((kind, name, line, end_line, end, has_body)) => {
                    let gkind = gg("kind").as_str().unwrap();
                    let gname = gg("name").as_str().unwrap();
                    let gline = gg("line").as_f64().unwrap() as usize;
                    let gendline = gg("endLine").as_f64().unwrap() as usize;
                    let gend = gg("endOffset").as_f64().unwrap() as usize;
                    let gbody = gg("hasBody").as_bool().unwrap();
                    if kind != gkind || name != gname || line != gline || end_line != gendline || end != gend || has_body != gbody {
                        field_mismatch += 1;
                        if shown < 100000 {
                            shown += 1;
                            println!("FIELD {path}#{}:", start);
                            println!("  golden kind={gkind} name={gname:?} line={gline} endLine={gendline} end={gend} body={gbody}");
                            println!("  mine   kind={kind} name={name:?} line={line} endLine={end_line} end={end} body={has_body}");
                        }
                    }
                }
            }
        }
        for (start, (kind, name, _, _, _, _)) in &mine {
            extra += 1;
            if shown < 100000 {
                shown += 1;
                println!("EXTRA {path}#{start} {kind} {name:?}");
            }
        }
    }
    println!("compared={compared} skipped_changed={skipped_changed} missing={missing} extra={extra} field_mismatch={field_mismatch}");
}
