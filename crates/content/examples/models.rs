#[path = "support/assets.rs"]
mod assets;
#[path = "support/retail.rs"]
mod retail;
use qa_formats::model::Model;
use std::{collections::BTreeMap, path::Path};

fn main() -> Result<(), String> {
    let root = std::env::args_os().nth(1).ok_or("qfiles root required")?;
    let root = Path::new(&root);
    let vfs = assets::mount(root)?;
    let mut counts = BTreeMap::new();
    let mut failures = 0;
    let mut models = 0;
    for (reference, name) in vfs.files() {
        let extensions: [&[u8]; 7] = [
            b".mdl",
            b".md2",
            b".md3",
            b".mdc",
            b".spr",
            b".sp2",
            b".md5mesh",
        ];
        if !extensions.iter().any(|ext| name.ends_with(ext)) {
            continue;
        }
        models += 1;
        let length = vfs.length(reference).map_err(|e| format!("{e:?}"))? as usize;
        let mut bytes = vec![0; length];
        vfs.read_at(reference, 0, &mut bytes)
            .map_err(|e| format!("{e:?}"))?;
        let origin = vfs.origin(reference).ok_or("origin")?;
        match Model::parse(&bytes) {
            Ok(model) => {
                *counts
                    .entry(format!("{:?}", model.format))
                    .or_insert(0usize) += 1;
                println!(
                    "loaded {:?}:{:?} format={:?} meshes={} frames={} vertices={} triangles={} sprites={} tags={} bones={} skins={} frame_groups={} skin_groups={}",
                    origin.path,
                    String::from_utf8_lossy(origin.member),
                    model.format,
                    model.meshes.len(),
                    model.frames.len(),
                    model.meshes.iter().map(|m| m.vertices.len()).sum::<usize>(),
                    model
                        .meshes
                        .iter()
                        .map(|m| m.triangles.len())
                        .sum::<usize>(),
                    model.sprites.len(),
                    model.tags.len(),
                    model.bones.len(),
                    model.skins.len(),
                    model.frame_groups.len(),
                    model.skin_groups.len()
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
        "scope=headless model admission, not rendering or gameplay; models={models} failures={failures} formats={counts:?}"
    );
    if failures != 0 {
        return Err(format!("{failures} model admissions failed"));
    }
    Ok(())
}
