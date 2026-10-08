use std::{fs::File, path::Path};

pub fn mount(root: &Path) -> Result<qa_content::vfs::Vfs, String> {
    use qa_formats::archive::Archive;
    use std::sync::Arc;
    let mut paths = Vec::new();
    super::retail::visit(root, &mut paths).map_err(|e| e.to_string())?;
    paths.sort();
    let mut vfs = qa_content::vfs::Vfs::default();
    for (rank, path) in paths.iter().enumerate() {
        if super::retail::chromium_resources(path)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            continue;
        }
        let archive = Arc::new(
            Archive::parse(Arc::new(File::open(path).map_err(|e| e.to_string())?))
                .map_err(|e| format!("{}: {e:?}", path.display()))?,
        );
        vfs.mount_archive(path, rank as i32, archive)
            .map_err(|e| format!("{e:?}"))?;
    }
    vfs.mount_directory(root, -1)
        .map_err(|e| format!("{e:?}"))?;
    Ok(vfs)
}
