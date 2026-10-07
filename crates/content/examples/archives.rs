#[path = "support/retail.rs"]
mod retail;
use qa_content::vfs::Vfs;
use qa_formats::archive::Archive;
use retail::{chromium_resources, visit};
use std::{fs::File, path::Path, sync::Arc};

fn main() -> Result<(), String> {
    let root = std::env::args_os().nth(1).ok_or("qfiles root required")?;
    let root = Path::new(&root);
    let mut paths = Vec::new();
    visit(root, &mut paths).map_err(|e| e.to_string())?;
    paths.sort();
    let mut vfs = Vfs::default();
    let mut members = 0;
    let mut samples = 0;
    let mut unrelated = 0;
    for (rank, path) in paths.iter().enumerate() {
        if let Some(resources) = chromium_resources(path).map_err(|e| e.to_string())? {
            println!(
                "{{\"container\":\"Chromium resource pack v4\",\"path\":{:?},\"resources\":{resources},\"game_mount\":false}}",
                path.to_string_lossy()
            );
            unrelated += 1;
            continue;
        }
        let archive = Arc::new(
            Archive::parse(Arc::new(File::open(path).map_err(|e| e.to_string())?))
                .map_err(|e| format!("{}: {e:?}", path.display()))?,
        );
        members += archive.entries.len();
        if let Some((entry, metadata)) = archive
            .entries
            .iter()
            .enumerate()
            .find(|(_, e)| !e.directory && e.length > 0 && e.length < 16 * 1024 * 1024)
        {
            let mut bytes = vec![0; metadata.length as usize];
            archive
                .read_into(entry, &mut bytes)
                .map_err(|e| format!("{} member {entry}: {e:?}", path.display()))?;
            samples += 1;
        }
        vfs.mount_archive(path, rank as i32, archive)
            .map_err(|e| format!("{} mount: {e:?}", path.display()))?;
        println!("{{\"mounted\":{:?}}}", path.to_string_lossy());
    }
    println!(
        "{{\"scope\":\"headless owned retail archive mounts, not gameplay\",\"files\":{},\"unrelated_containers\":{unrelated},\"members\":{members},\"sampled_members\":{samples},\"mounts\":{}}}",
        paths.len(),
        vfs.mounts().count()
    );
    let mut products = Vfs::default();
    for (index, path) in [
        root.join("q1/id1"),
        root.join("q2/baseq2"),
        root.join("q3a/baseq3"),
    ]
    .iter()
    .enumerate()
    {
        products
            .mount_product(path, index as i32 * 1000)
            .map_err(|e| format!("{} product: {e:?}", path.display()))?;
    }
    for path in [
        b"maps/e1m1.bsp".as_slice(),
        b"maps/base1.bsp",
        b"maps/q3dm1.bsp",
    ] {
        let reference = products
            .open(path)
            .ok_or_else(|| format!("missing {}", String::from_utf8_lossy(path)))?;
        let length = usize::try_from(products.length(reference).map_err(|e| format!("{e:?}"))?)
            .map_err(|e| e.to_string())?;
        let mut bytes = vec![0; length];
        products
            .read_at(reference, 0, &mut bytes)
            .map_err(|e| format!("{e:?}"))?;
        let origin = products.origin(reference).ok_or("origin")?;
        println!(
            "map={} bytes={length} origin={}",
            String::from_utf8_lossy(path),
            origin.path.display()
        );
    }
    for (native, canonical, expected) in [
        (
            b"models/monsters/tank/../ctank/skin.pcx".as_slice(),
            b"models/monsters/ctank/skin.pcx".as_slice(),
            60795,
        ),
        (
            b"models/monsters/tank/../ctank/pain.pcx".as_slice(),
            b"models/monsters/ctank/pain.pcx".as_slice(),
            61879,
        ),
    ] {
        let skin = products.open(native).ok_or("original ctank path missing")?;
        if products.open(canonical) != Some(skin) {
            return Err("ctank normalized lookup differs".into());
        }
        let mut bytes = vec![0; products.length(skin).map_err(|e| format!("{e:?}"))? as usize];
        products
            .read_at(skin, 0, &mut bytes)
            .map_err(|e| format!("{e:?}"))?;
        if bytes.len() != expected {
            return Err("ctank payload length differs".into());
        }
        println!(
            "ctank_bytes={} original_member={:?}",
            bytes.len(),
            String::from_utf8_lossy(products.origin(skin).ok_or("ctank origin")?.member)
        );
    }
    Ok(())
}
