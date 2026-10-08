use qa_content::vfs::Vfs;
use qa_render::{Assets, material::resources::Images, surface_cache::PaletteLighting};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "qa-render-pcx-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("env")).unwrap();
        let mut pcx = vec![0; 128];
        pcx[..4].copy_from_slice(&[10, 5, 1, 8]);
        pcx[8..10].copy_from_slice(&1u16.to_le_bytes());
        pcx[65] = 1;
        pcx[66..68].copy_from_slice(&2u16.to_le_bytes());
        pcx.extend_from_slice(&[17, 0xc1, 255, 12]);
        pcx.extend((0..256).flat_map(|_| [200, 201, 202]));
        std::fs::write(root.join("env/native.pcx"), pcx).unwrap();
        let mut tga = vec![0; 18];
        tga[2] = 2;
        tga[12..14].copy_from_slice(&1u16.to_le_bytes());
        tga[14..16].copy_from_slice(&1u16.to_le_bytes());
        tga[16] = 24;
        tga[17] = 32;
        tga.extend_from_slice(&[5, 4, 3]);
        std::fs::write(root.join("env/native.tga"), tga).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn palette() -> PaletteLighting {
    let colors: Vec<_> = (0..256).flat_map(|_| [7, 11, 19]).collect();
    let shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    PaletteLighting::load(&colors, &shades, None, 256).unwrap()
}

#[test]
fn indexed_pcx_uses_selected_global_palette_and_explicit_mask() {
    let root = Fixture::new();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load();
    let palette = assets.register_palette(palette()).unwrap();
    let mut images = Images::new(&vfs, &mut assets);
    let opaque = images
        .indexed_pcx("env/native.pcx", palette, None, None)
        .unwrap();
    let masked = images
        .indexed_pcx("env/native.pcx", palette, Some(255), None)
        .unwrap();
    assert_eq!(
        images
            .indexed_pcx("env/native.pcx", palette, None, None)
            .unwrap(),
        opaque
    );
    assert_ne!(opaque, masked);
    assert_eq!(
        images.assets.image(opaque).unwrap().rgba.as_ref(),
        [7, 11, 19, 255, 7, 11, 19, 255]
    );
    assert_eq!(
        images.assets.image(masked).unwrap().rgba.as_ref(),
        [7, 11, 19, 255, 7, 11, 19, 0]
    );
}

#[test]
fn sky_resource_keeps_distinct_native_tga_pixels_and_pcx_dimensions() {
    let root = Fixture::new();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load();
    let palette = assets.register_palette(palette()).unwrap();
    let mut images = Images::new(&vfs, &mut assets);
    let id = images
        .indexed_pcx("env/native.pcx", palette, None, Some("env/native.tga"))
        .unwrap();
    let image = images.assets.image(id).unwrap();
    assert_eq!((image.width, image.height), (1, 1));
    assert_eq!(image.rgba.as_ref(), [3, 4, 5, 255]);
    let indexed = image.indexed.as_ref().unwrap();
    let mip = indexed.mip(0).unwrap();
    assert_eq!((mip.width, mip.height), (2, 1));
    assert_eq!(mip.indices(), [17, 255]);
    assert_eq!(indexed.transparent_index(), None);
    assert!(indexed.mip(1).is_none());
}
