#[path = "support/assets.rs"]
mod assets;
#[path = "support/retail.rs"]
mod retail;
use qa_formats::bsp::Map;
use std::{collections::BTreeMap, path::Path};

fn main() -> Result<(), String> {
    let root = std::env::args_os().nth(1).ok_or("qfiles root required")?;
    let root = Path::new(&root);
    let vfs = assets::mount(root)?;
    let mut counts = BTreeMap::new();
    let mut failures = 0;
    let mut maps = 0;
    for (reference, name) in vfs.files() {
        if !name.ends_with(b".bsp") {
            continue;
        }
        maps += 1;
        let length = vfs.length(reference).map_err(|e| format!("{e:?}"))? as usize;
        let mut bytes = vec![0; length];
        vfs.read_at(reference, 0, &mut bytes)
            .map_err(|e| format!("{e:?}"))?;
        let origin = vfs.origin(reference).ok_or("origin")?;
        match Map::parse(&bytes) {
            Ok(map) => {
                *counts
                    .entry(format!("{:?}", map.bsp.format))
                    .or_insert(0usize) += 1;
                println!(
                    "loaded {:?}:{:?} format={:?} planes={} vertices={} nodes={} leaves={} faces={} brushes={} surfaces={} models={} extensions={} entity_tail_clamped={} pop_tail_clamped={}",
                    origin.path,
                    String::from_utf8_lossy(origin.member),
                    map.bsp.format,
                    map.planes.len(),
                    map.vertices.len(),
                    map.nodes.len(),
                    map.leaves.len(),
                    map.faces.len(),
                    map.brushes.len(),
                    map.surfaces.len(),
                    map.models.len(),
                    map.extensions.len(),
                    map.bsp.entity_tail_clamped,
                    map.bsp.pop_tail_clamped
                );
            }
            Err(error) => {
                failures += 1;
                println!(
                    "failed {:?}:{:?} {error:?}",
                    origin.path,
                    String::from_utf8_lossy(origin.member)
                );
            }
        }
    }
    println!(
        "scope=headless BSP admission, not rendering or gameplay; maps={maps} failures={failures} formats={counts:?}"
    );
    if failures != 0 {
        return Err(format!("{failures} map admissions failed"));
    }
    Ok(())
}
