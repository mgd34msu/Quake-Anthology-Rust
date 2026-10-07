use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

// Inventory only: Chromium resource packs share the .pak suffix but have
// numeric resource IDs, not Quake asset paths. They are not engine mounts.
pub fn chromium_resources(path: &Path) -> std::io::Result<Option<u32>> {
    let mut file = File::open(path)?;
    let mut header = [0; 9];
    if file.read_exact(&mut header).is_err()
        || u32::from_le_bytes(header[..4].try_into().unwrap()) != 4
        || header[8] > 2
    {
        return Ok(None);
    }
    let count = u32::from_le_bytes(header[4..8].try_into().unwrap());
    if count > u16::MAX as u32 {
        return Ok(None);
    }
    let start = 9 + u64::from(count + 1) * 6;
    let size = file.metadata()?.len();
    let mut previous_id = None;
    let mut previous_offset = start;
    for index in 0..=count {
        let mut row = [0; 6];
        if file.read_exact(&mut row).is_err() {
            return Ok(None);
        }
        let id = u16::from_le_bytes(row[..2].try_into().unwrap());
        let offset = u64::from(u32::from_le_bytes(row[2..].try_into().unwrap()));
        if offset < previous_offset || offset > size || (index == 0 && offset != start) {
            return Ok(None);
        }
        if index == count {
            if id != 0 || offset != size {
                return Ok(None);
            }
        } else if previous_id.is_some_and(|previous| previous >= id) {
            return Ok(None);
        }
        previous_id = Some(id);
        previous_offset = offset;
    }
    Ok(Some(count))
}

pub fn visit(root: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            visit(&path, files)?;
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("pk3"))
        {
            files.push(path);
        }
    }
    Ok(())
}
