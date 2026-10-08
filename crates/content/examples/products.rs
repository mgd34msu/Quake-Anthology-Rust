#[path = "support/assets.rs"]
mod assets;
#[path = "support/retail.rs"]
mod retail;
fn main() -> Result<(), String> {
    let root = std::env::args_os().nth(1).ok_or("qfiles root required")?;
    let vfs = assets::mount(std::path::Path::new(&root))?;
    let products = qa_content::products::detect(&vfs);
    for product in &products {
        let spec = product.spec();
        let origin = vfs.origin(product.start_map).ok_or("start map origin")?;
        println!(
            "product={} edition={:?} directory={:?} start={} origin={:?}:{:?} mounts={:?} shared_assets={}",
            spec.key,
            spec.edition,
            product.directory,
            String::from_utf8_lossy(spec.start_map),
            origin.path,
            String::from_utf8_lossy(origin.member),
            product.mounts,
            product.shared_assets
        );
        let mut alone = qa_content::vfs::Vfs::default();
        alone
            .mount_product(&product.directory, 0)
            .map_err(|e| format!("{e:?}"))?;
        let reference = alone
            .open(spec.start_map)
            .ok_or("isolated default map missing")?;
        let mut bytes = vec![0; alone.length(reference).map_err(|e| format!("{e:?}"))? as usize];
        alone
            .read_at(reference, 0, &mut bytes)
            .map_err(|e| format!("{e:?}"))?;
        qa_formats::bsp::Map::parse(&bytes)
            .map_err(|e| format!("{} default geometry: {e:?}", spec.key))?;
    }
    println!(
        "scope=headless product/default-map resolution, not launch; products={}",
        products.len()
    );
    Ok(())
}
