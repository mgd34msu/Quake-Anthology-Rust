//! Material script discovery is a VFS load operation, never a frame operation.
use crate::shader::{ShaderCatalog, ShaderSource, parse_sources};
use qa_content::vfs::{Vfs, VfsError};
use qa_formats::archive::ArchiveReader;

pub mod resources;
pub mod world_load;

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
pub fn load_catalog(vfs: &Vfs) -> Result<ShaderCatalog, CatalogLoadError> {
    let mut source_names = Vec::new();
    let mut source_bytes = Vec::new();
    let mut reader = ArchiveReader::default();
    let mut previous_name: &[u8] = &[];
    for (file, name) in vfs.files() {
        // The index includes lower-priority entries immediately after the
        // winning name. A replaced script must not contribute old definitions.
        if name == previous_name {
            continue;
        }
        previous_name = name;
        if !name.starts_with(b"scripts/") || !name.ends_with(b".shader") {
            continue;
        }
        let length = usize::try_from(vfs.length(file)?).map_err(|_| CatalogLoadError::Size)?;
        // A corrupt package must not reserve unbounded memory for a script.
        if length > 16 * 1024 * 1024 || source_names.len() >= 4096 {
            return Err(CatalogLoadError::Size);
        }
        let mut bytes = vec![0; length];
        if vfs.read_into_reusing(file, &mut bytes, &mut reader)? != length {
            return Err(CatalogLoadError::ShortRead);
        }
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
