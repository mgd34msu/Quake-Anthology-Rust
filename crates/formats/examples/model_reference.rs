use qa_formats::model::Model;
use std::{fs, path::Path};

fn main() -> Result<(), String> {
    let root = std::env::args_os()
        .nth(1)
        .ok_or("reference folder required")?;
    let root = Path::new(&root);
    for name in [
        "group.mdl",
        "mesh.md2",
        "mesh.md3",
        "mesh.mdc",
        "mesh.md5mesh",
        "group.spr",
        "sprite.sp2",
    ] {
        let bytes = fs::read(root.join(name)).map_err(|e| e.to_string())?;
        let model = Model::parse(&bytes).map_err(|e| format!("{name}: {e:?}"))?;
        println!(
            "{name}: {:?} meshes={} frames={} sprites={} tags={} bones={}",
            model.format,
            model.meshes.len(),
            model.frames.len(),
            model.sprites.len(),
            model.tags.len(),
            model.bones.len()
        );
    }
    let bytes = fs::read(root.join("normals.md3")).map_err(|e| e.to_string())?;
    let expected = fs::read(root.join("original-normals.bin")).map_err(|e| e.to_string())?;
    let model = Model::parse(&bytes).map_err(|e| format!("{e:?}"))?;
    let mut actual = Vec::new();
    for mesh in &model.meshes {
        for vertex in &mesh.vertices {
            for value in vertex.normal.0 {
                actual.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    if actual != expected {
        let differences = actual.iter().zip(&expected).filter(|(a, b)| a != b).count();
        return Err(format!(
            "packed normal byte differences={differences}; actual={} expected={}",
            actual.len(),
            expected.len()
        ));
    }
    println!("PASS: 65536 packed MD3 normals, 196608 original Q3 float32 components bit-identical");
    Ok(())
}
