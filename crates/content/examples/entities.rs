#[path = "support/assets.rs"]
mod assets;
#[path = "support/retail.rs"]
mod retail;
use qa_formats::{
    bsp::{Bsp, BspFormat, Lump},
    entities::{EntityLump, EntitySyntax},
};
use std::{collections::BTreeMap, path::Path};
fn main() -> Result<(), String> {
    let root = std::env::args_os().nth(1).ok_or("qfiles root required")?;
    let vfs = assets::mount(Path::new(&root))?;
    let evidence = std::env::args_os().nth(2).map(std::path::PathBuf::from);
    if let Some(ref path) = evidence {
        std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
    }
    let mut selected = BTreeMap::new();
    let mut maps = 0;
    let mut entities = 0;
    let mut fields = 0;
    let mut failures = 0;
    for (reference, name) in vfs.files() {
        if !name.ends_with(b".bsp") {
            continue;
        }
        let mut bytes = vec![0; vfs.length(reference).map_err(|e| format!("{e:?}"))? as usize];
        vfs.read_at(reference, 0, &mut bytes)
            .map_err(|e| format!("{e:?}"))?;
        let bsp = Bsp::parse(&bytes).map_err(|e| format!("{e:?}"))?;
        let syntax = match bsp.format {
            BspFormat::Quake2 | BspFormat::Qbsp => EntitySyntax::Quake2,
            BspFormat::Quake3 | BspFormat::Quake3Test | BspFormat::QuakeLive => {
                EntitySyntax::Quake3
            }
            _ => EntitySyntax::Quake,
        };
        maps += 1;
        match EntityLump::parse(bsp.bytes(Lump::Entities), syntax) {
            Ok(lump) => {
                entities += lump.records.len();
                fields += lump.fields.len();
                let base = name.rsplit(|&b| b == b'/').next().unwrap_or(name);
                if [b"e1m1.bsp".as_slice(), b"base1.bsp", b"q3dm1.bsp"].contains(&base) {
                    *selected
                        .entry(String::from_utf8_lossy(base).into_owned())
                        .or_insert(0usize) += 1;
                    if let Some(ref root) = evidence {
                        let stem = format!("{}-{}", syntax as u8, maps);
                        std::fs::write(root.join(format!("{stem}.ent")), bsp.bytes(Lump::Entities))
                            .map_err(|e| e.to_string())?;
                        let mut encoded = Vec::new();
                        for record in &lump.records {
                            encoded.extend(((record.end - record.start) as u32).to_le_bytes());
                            for entry in &lump.fields[record.clone()] {
                                let mut key = lump.names.get(entry.key).ok_or("key")?.to_vec();
                                if syntax != EntitySyntax::Quake {
                                    key.make_ascii_lowercase();
                                }
                                for value in [key.as_slice(), entry.value] {
                                    encoded.extend((value.len() as u32).to_le_bytes());
                                    encoded.extend(value);
                                }
                            }
                        }
                        std::fs::write(root.join(format!("{stem}.raw")), encoded)
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
            Err(e) => {
                failures += 1;
                let origin = vfs.origin(reference).ok_or("origin")?;
                println!(
                    "failed {:?}:{:?} {e:?}",
                    origin.path,
                    String::from_utf8_lossy(origin.member)
                );
            }
        }
    }
    println!(
        "scope=headless entity lump parsing, not live spawning; maps={maps} entities={entities} fields={fields} failures={failures} selected={selected:?}"
    );
    if failures > 0 {
        return Err(format!("{failures} entity lumps failed"));
    }
    if selected.len() != 3 {
        return Err("three gate maps required".into());
    }
    Ok(())
}
