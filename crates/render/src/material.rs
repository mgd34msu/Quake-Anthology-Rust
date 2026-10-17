//! Material script discovery is a VFS load operation, never a frame operation.
use crate::shader::{ShaderCatalog, ShaderSource, parse_sources};
use qa_content::vfs::{SearchView, Vfs, VfsError};
use qa_formats::archive::ArchiveReader;

mod finish;
pub mod resources;
pub mod world_load;
pub use finish::{MaterialFinishPolicy, ShaderCompilePolicy};

#[derive(Debug)]
pub enum CatalogLoadError {
    Vfs(VfsError),
    Size,
    ShortRead,
    Name,
}
impl From<VfsError> for CatalogLoadError {
    fn from(error: VfsError) -> Self {
        Self::Vfs(error)
    }
}

/// VFS's sorted winning-file index supplies names and precedence. No separate
/// filesystem scan or hash index exists for shaders.
pub fn load_catalog(
    vfs: &Vfs,
    search: Option<&SearchView>,
) -> Result<ShaderCatalog, CatalogLoadError> {
    let mut source_names = Vec::new();
    let mut source_bytes = Vec::new();
    let mut reader = ArchiveReader::default();
    for (file, name) in vfs.files_with(search) {
        if !name.starts_with(b"scripts/") || !name.ends_with(b".shader") {
            continue;
        }
        if source_names.len() >= 4096 {
            return Err(CatalogLoadError::Size);
        }
        let bytes = vfs
            .read_to_vec(&file, 16 * 1024 * 1024, &mut reader)
            .map_err(|error| match error {
                VfsError::Size | VfsError::Limit | VfsError::Allocation => CatalogLoadError::Size,
                VfsError::ShortRead => CatalogLoadError::ShortRead,
                error => CatalogLoadError::Vfs(error),
            })?;
        source_names.push(
            std::str::from_utf8(name)
                .map_err(|_| CatalogLoadError::Name)?
                .to_owned(),
        );
        source_bytes.push(bytes);
    }
    let sources: Vec<_> = source_names
        .iter()
        .zip(&source_bytes)
        .map(|(name, bytes)| ShaderSource { name, bytes })
        .collect();
    Ok(parse_sources(&sources))
}
