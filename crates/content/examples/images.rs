#[path = "support/assets.rs"]
mod assets;
#[path = "support/retail.rs"]
mod retail;
use qa_formats::image::{
    self, Colormap, ImageFormat, MipFormat, MipTexture, Palette, RasterPolicy, Wad, WadImage,
};
use std::{collections::BTreeMap, path::Path};

fn check(name: &[u8], bytes: &[u8]) -> Result<(String, usize), qa_formats::FormatError> {
    if name.ends_with(b".wad") {
        let wad = Wad::parse(bytes)?;
        let mut images = 0;
        for index in 0..wad.lumps.len() {
            match wad.image(index)? {
                WadImage::Raw(_) => (),
                _ => images += 1,
            }
        }
        return Ok(("WAD".into(), images));
    }
    if name.ends_with(b".wal") {
        let _texture = MipTexture::parse(bytes, MipFormat::Wal)?;
        return Ok(("WAL".into(), 1));
    }
    let base = name.rsplit(|&b| b == b'/').next().unwrap_or(name);
    if base.eq_ignore_ascii_case(b"palette.lmp") {
        Palette::from_rgb(bytes)?;
        return Ok(("Palette".into(), 1));
    }
    if base.eq_ignore_ascii_case(b"colormap.lmp") {
        Colormap::parse(bytes)?;
        return Ok(("Colormap".into(), 1));
    }
    if base.eq_ignore_ascii_case(b"pop.lmp") {
        if bytes.len() != 256 {
            return Err(qa_formats::FormatError::InvalidRecordSize);
        }
        return Ok(("Registration bitmap".into(), 0));
    }
    if base.eq_ignore_ascii_case(b"conchars.lmp") && bytes.len() == 128 * 128 {
        return Ok(("Raw font".into(), 1));
    }
    let format = ImageFormat::from_path(name).ok_or(qa_formats::FormatError::Unsupported)?;
    let _image = image::decode(bytes, format, RasterPolicy::Standard)?;
    Ok((format!("{format:?}"), 1))
}
fn main() -> Result<(), String> {
    let root = std::env::args_os().nth(1).ok_or("qfiles root required")?;
    let vfs = assets::mount(Path::new(&root))?;
    let mut counts = BTreeMap::new();
    let mut failures = 0;
    let mut files = 0;
    let mut images = 0;
    for (reference, name) in vfs.files() {
        if ImageFormat::from_path(name).is_none()
            && !name.ends_with(b".wal")
            && !name.ends_with(b".wad")
        {
            continue;
        }
        files += 1;
        let mut bytes = vec![0; vfs.length(reference).map_err(|e| format!("{e:?}"))? as usize];
        vfs.read_at(reference, 0, &mut bytes)
            .map_err(|e| format!("{e:?}"))?;
        let origin = vfs.origin(reference).ok_or("origin")?;
        match check(name, &bytes) {
            Ok((format, count)) => {
                *counts.entry(format).or_insert(0usize) += 1;
                images += count;
            }
            Err(error) => {
                failures += 1;
                println!(
                    "failed {:?}:{:?} bytes={} {error:?}",
                    origin.path,
                    String::from_utf8_lossy(origin.member),
                    bytes.len()
                );
            }
        }
    }
    println!(
        "scope=headless image admission and decode, not rendering; files={files} images={images} failures={failures} formats={counts:?}"
    );
    if failures > 0 {
        return Err(format!("{failures} images failed"));
    }
    Ok(())
}
