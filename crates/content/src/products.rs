//! Product metadata selects startup assets; it never restricts shared services.
use crate::vfs::{FileRef, MountId, MountKind, Vfs};
use qa_core::primitives::ProductId;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edition {
    Classic,
    Rerelease,
    QuakeWorld,
    QuakeLive,
}
pub struct ProductSpec {
    pub key: &'static str,
    pub directory: &'static [u8],
    pub root_hint: &'static [u8],
    pub edition: Edition,
    pub start_map: &'static [u8],
    pub base: Option<ProductId>,
}
macro_rules! product {
    ($key:literal,$dir:literal,$root:literal,$edition:ident,$map:literal,$base:expr) => {
        ProductSpec {
            key: $key,
            directory: $dir,
            root_hint: $root,
            edition: Edition::$edition,
            start_map: $map,
            base: $base,
        }
    };
}
pub static STOCK: &[ProductSpec] = &[
    product!(
        "q1-classic-id1",
        b"id1",
        b"q1",
        Classic,
        b"maps/start.bsp",
        None
    ),
    product!(
        "q1-classic-hipnotic",
        b"hipnotic",
        b"q1",
        Classic,
        b"maps/start.bsp",
        Some(ProductId(0))
    ),
    product!(
        "q1-classic-rogue",
        b"rogue",
        b"q1",
        Classic,
        b"maps/start.bsp",
        Some(ProductId(0))
    ),
    product!(
        "q1-classic-ctf",
        b"ctf",
        b"q1",
        Classic,
        b"maps/ctfstart.bsp",
        Some(ProductId(0))
    ),
    product!(
        "q1-rerelease-id1",
        b"id1",
        b"q1",
        Rerelease,
        b"maps/start.bsp",
        None
    ),
    product!(
        "q1-rerelease-hipnotic",
        b"hipnotic",
        b"q1",
        Rerelease,
        b"maps/start.bsp",
        Some(ProductId(4))
    ),
    product!(
        "q1-rerelease-rogue",
        b"rogue",
        b"q1",
        Rerelease,
        b"maps/start.bsp",
        Some(ProductId(4))
    ),
    product!(
        "q1-rerelease-dopa",
        b"dopa",
        b"q1",
        Rerelease,
        b"maps/start.bsp",
        Some(ProductId(4))
    ),
    product!(
        "q1-rerelease-mg1",
        b"mg1",
        b"q1",
        Rerelease,
        b"maps/start.bsp",
        Some(ProductId(4))
    ),
    product!(
        "q1-rerelease-mg3",
        b"mg3",
        b"q1",
        Rerelease,
        b"maps/start.bsp",
        Some(ProductId(4))
    ),
    product!(
        "q1-rerelease-ctf",
        b"ctf",
        b"q1",
        Rerelease,
        b"maps/ctf1.bsp",
        Some(ProductId(4))
    ),
    product!(
        "q1-quakeworld",
        b"qw",
        b"q1",
        QuakeWorld,
        b"maps/start.bsp",
        Some(ProductId(0))
    ),
    product!(
        "q1-rerelease-quake64",
        b"q64",
        b"q1",
        Rerelease,
        b"maps/start.bsp",
        Some(ProductId(4))
    ),
    product!(
        "q2-classic-baseq2",
        b"baseq2",
        b"q2",
        Classic,
        b"maps/base1.bsp",
        None
    ),
    product!(
        "q2-classic-xatrix",
        b"xatrix",
        b"q2",
        Classic,
        b"maps/xswamp.bsp",
        Some(ProductId(13))
    ),
    product!(
        "q2-classic-rogue",
        b"rogue",
        b"q2",
        Classic,
        b"maps/rmine1.bsp",
        Some(ProductId(13))
    ),
    product!(
        "q2-classic-ctf",
        b"ctf",
        b"q2",
        Classic,
        b"maps/q2ctf1.bsp",
        Some(ProductId(13))
    ),
    product!(
        "q2-classic-lmctf",
        b"lmctf",
        b"q2",
        Classic,
        b"maps/lmctf09.bsp",
        Some(ProductId(13))
    ),
    product!(
        "q2-rerelease-baseq2",
        b"baseq2",
        b"q2",
        Rerelease,
        b"maps/base1.bsp",
        None
    ),
    product!(
        "q2-rerelease-xatrix",
        b"baseq2",
        b"q2",
        Rerelease,
        b"maps/xswamp.bsp",
        Some(ProductId(18))
    ),
    product!(
        "q2-rerelease-rogue",
        b"baseq2",
        b"q2",
        Rerelease,
        b"maps/rmine1.bsp",
        Some(ProductId(18))
    ),
    product!(
        "q2-rerelease-ctf",
        b"baseq2",
        b"q2",
        Rerelease,
        b"maps/q2ctf1.bsp",
        Some(ProductId(18))
    ),
    product!(
        "q2-rerelease-mg2",
        b"baseq2",
        b"q2",
        Rerelease,
        b"maps/mguhub.bsp",
        Some(ProductId(18))
    ),
    product!(
        "q2-rerelease-n64",
        b"baseq2",
        b"q2",
        Rerelease,
        b"maps/q64/rtest.bsp",
        Some(ProductId(18))
    ),
    product!(
        "q3-baseq3",
        b"baseq3",
        b"q3a",
        Classic,
        b"maps/q3dm1.bsp",
        None
    ),
    product!(
        "q3-missionpack",
        b"missionpack",
        b"q3a",
        Classic,
        b"maps/mpteam1.bsp",
        Some(ProductId(24))
    ),
    product!(
        "q3-demota",
        b"demota",
        b"q3a",
        Classic,
        b"maps/q3dm1.bsp",
        None
    ),
    product!(
        "quakelive",
        b"baseq3",
        b"quakelive",
        QuakeLive,
        b"maps/campgrounds.bsp",
        None
    ),
];
pub struct InstalledProduct {
    pub id: ProductId,
    pub directory: PathBuf,
    pub mounts: Vec<MountId>,
    pub start_map: FileRef,
    pub shared_assets: bool,
}
impl InstalledProduct {
    pub fn spec(&self) -> &'static ProductSpec {
        &STOCK[self.id.0 as usize]
    }
}
fn component(path: &Path, name: &[u8]) -> bool {
    path.components()
        .any(|c| c.as_os_str().as_encoded_bytes().eq_ignore_ascii_case(name))
}
fn directory<'a>(path: &'a Path, spec: &ProductSpec) -> Option<&'a Path> {
    let dir = path.ancestors().find(|p| {
        p.file_name()
            .is_some_and(|s| s.as_encoded_bytes().eq_ignore_ascii_case(spec.directory))
    })?;
    let rerelease = component(dir, b"rerelease");
    let live = component(dir, b"quakelive");
    if (spec.edition == Edition::Rerelease) != rerelease
        || (spec.edition == Edition::QuakeLive) != live
    {
        return None;
    }
    // Explicit directory ancestry disambiguates similarly named packs. A
    // standalone directory can still be identified by its own start-map witness.
    for hint in [b"q1".as_slice(), b"q2", b"q3a", b"quakelive"] {
        if component(dir, hint) && hint != spec.root_hint {
            return None;
        }
    }
    Some(dir)
}
pub fn detect(vfs: &Vfs) -> Vec<InstalledProduct> {
    let mut products: Vec<InstalledProduct> = Vec::new();
    for (reference, name) in vfs.files() {
        let Some(origin) = vfs.origin(reference) else {
            continue;
        };
        let physical = if origin.kind == MountKind::Directory {
            let Ok(member) = std::str::from_utf8(origin.member) else {
                continue;
            };
            origin.path.join(member)
        } else {
            origin.path.to_path_buf()
        };
        for (index, spec) in STOCK.iter().enumerate() {
            let Some(dir) = directory(&physical, spec) else {
                continue;
            };
            let is_map = if origin.kind == MountKind::Directory {
                let Ok(member) = physical.strip_prefix(dir) else {
                    continue;
                };
                member
                    .as_os_str()
                    .as_encoded_bytes()
                    .iter()
                    .map(|&b| {
                        if b == b'\\' {
                            b'/'
                        } else {
                            b.to_ascii_lowercase()
                        }
                    })
                    .eq(spec.start_map.iter().copied())
            } else {
                name == spec.start_map
            };
            if !is_map {
                continue;
            }
            let id = ProductId(index as u16);
            if let Some(product) = products
                .iter_mut()
                .find(|p| p.id == id && p.directory == dir)
            {
                let old = vfs.origin(product.start_map).map(|o| o.mount);
                let rank = |mount| {
                    vfs.mounts()
                        .find(|m| m.id == mount)
                        .map(|m| (m.priority, m.id.0))
                };
                if rank(origin.mount) > old.and_then(rank) {
                    product.start_map = reference;
                }
                if !product.mounts.contains(&origin.mount) {
                    product.mounts.push(origin.mount);
                }
            } else {
                products.push(InstalledProduct {
                    id,
                    directory: dir.to_path_buf(),
                    mounts: vec![origin.mount],
                    start_map: reference,
                    shared_assets: false,
                });
            }
        }
    }
    // QuakeWorld uses original Quake assets, not a second asset installation.
    if !products.iter().any(|p| p.id == ProductId(11)) {
        let aliases: Vec<_> = products
            .iter()
            .filter(|p| p.id == ProductId(0))
            .map(|p| InstalledProduct {
                id: ProductId(11),
                directory: p.directory.clone(),
                mounts: p.mounts.clone(),
                start_map: p.start_map,
                shared_assets: true,
            })
            .collect();
        products.extend(aliases);
    }
    // Package membership is metadata; no game family compatibility filter.
    for product in &mut products {
        for mount in vfs.mounts() {
            let path = if mount.kind == MountKind::Directory {
                mount.path.as_path()
            } else {
                mount.path.parent().unwrap_or(&mount.path)
            };
            if path == product.directory && !product.mounts.contains(&mount.id) {
                product.mounts.push(mount.id);
            }
        }
    }
    products.sort_by(|a, b| (a.id.0, &a.directory).cmp(&(b.id.0, &b.directory)));
    products
}
