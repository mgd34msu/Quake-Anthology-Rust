//! Product metadata chooses a default start map; shared services remain unchanged.
use qa_content::{
    products::{self, Edition},
    vfs::Vfs,
};
use qa_core::primitives::RuleSetId;
use std::{io, path::Path};

#[derive(Debug)]
pub struct DefaultGame {
    pub product: &'static str,
    pub map: &'static str,
    pub rules: RuleSetId,
}

fn selected(vfs: &Vfs) -> Result<Option<DefaultGame>, String> {
    let installed = products::detect(vfs);
    let Some(product) = installed
        .iter()
        .filter(|product| {
            !product.shared_assets
                && vfs.open(product.spec().start_map).as_ref() == Some(&product.start_map)
        })
        .min_by_key(|product| product.id.0)
    else {
        return Ok(None);
    };
    let spec = product.spec();
    Ok(Some(DefaultGame {
        product: spec.key,
        map: std::str::from_utf8(spec.start_map).map_err(|e| e.to_string())?,
        rules: spec.client_rules().ok_or("product has no client preset")?,
    }))
}

/// Explicit mounts win. Otherwise probe base installations in stock catalog
/// order, retaining only the selected product in the existing VFS.
pub fn default_game<'a>(
    vfs: &mut Vfs,
    roots: impl IntoIterator<Item = &'a Path>,
) -> Result<DefaultGame, String> {
    if vfs.mounts().next().is_some() {
        return selected(vfs)?.ok_or("mounted content has no stock start map; use --map".into());
    }
    for root in roots {
        for spec in products::STOCK.iter().filter(|spec| spec.base.is_none()) {
            let hint = std::str::from_utf8(spec.root_hint).map_err(|e| e.to_string())?;
            let directory = std::str::from_utf8(spec.directory).map_err(|e| e.to_string())?;
            let parent = root.join(hint);
            let parent = if spec.edition == Edition::Rerelease {
                parent.join("rerelease")
            } else {
                parent
            };
            for path in [parent.join(directory), root.join(directory)] {
                let mut content = Vfs::default();
                match content.mount_product(&path, 0) {
                    Err(qa_content::vfs::VfsError::Io(error))
                        if error.kind() == io::ErrorKind::NotFound =>
                    {
                        continue;
                    }
                    Err(error) => return Err(format!("content {}: {error}", path.display())),
                    Ok(_) => {}
                }
                if let Some(game) = selected(&content)? {
                    *vfs = content;
                    return Ok(game);
                }
            }
        }
    }
    Err("no installed game found beside qa-rust or in the working directory; use --content and --map".into())
}
