use qa_content::vfs::Vfs;
use qa_formats::image::RasterPolicy;
use qa_render::{
    Assets,
    assets::{Sampler, Wrap},
    material::resources::{ImageRole, ImageSettings, ImageUse, Images, ResourceError},
    surface_cache::PaletteLighting,
};
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
    fn tga(&self, name: &str, width: u16, height: u16, color: [u8; 3]) {
        let mut bytes = vec![0; 18];
        bytes[2] = 2;
        bytes[12..14].copy_from_slice(&width.to_le_bytes());
        bytes[14..16].copy_from_slice(&height.to_le_bytes());
        bytes[16] = 24;
        bytes[17] = 32;
        for _ in 0..usize::from(width) * usize::from(height) {
            bytes.extend_from_slice(&[color[2], color[1], color[0]]);
        }
        std::fs::write(self.0.join(name), bytes).unwrap();
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
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RasterPolicy::Quake2,
        ImageSettings::native(2),
    )
    .unwrap();
    let opaque = images
        .indexed_pcx("env/native.pcx", palette, None, None, ImageUse::pic())
        .unwrap();
    let masked = images
        .indexed_pcx("env/native.pcx", palette, Some(255), None, ImageUse::pic())
        .unwrap();
    assert_eq!(
        images
            .indexed_pcx("env/native.pcx", palette, None, None, ImageUse::pic())
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
    // GL_Upload8 repairs index 255 independently of an opaque CPU PCX.
    let image = images.assets.image(opaque).unwrap();
    assert_eq!(image.indexed.as_ref().unwrap().transparent_index(), None);
    assert_eq!(image.prepared.as_ref().unwrap().levels[0].rgba[7], 0);
}

#[test]
fn sky_resource_keeps_distinct_native_tga_pixels_and_pcx_dimensions() {
    let root = Fixture::new();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load();
    let palette = assets.register_palette(palette()).unwrap();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RasterPolicy::Quake2,
        ImageSettings::native(2),
    )
    .unwrap();
    let id = images
        .indexed_pcx(
            "env/native.pcx",
            palette,
            None,
            Some("env/native.tga"),
            ImageUse {
                role: ImageRole::CubeSky,
                ..ImageUse::pic()
            },
        )
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
    assert_eq!(
        image.prepared.as_ref().unwrap().levels[0].rgba.as_ref(),
        image.rgba.as_ref()
    );
    assert_eq!(image.prepared.as_ref().unwrap().inverse_intensity, 1.0);
}

#[test]
fn native_q2_surface_and_sky_select_distinct_upload_rules() {
    let root = Fixture::new();
    root.tga("env/odd.tga", 3, 3, [17, 23, 31]);
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load();
    let settings = ImageSettings {
        picmip: 1,
        intensity: 2.5,
        ..ImageSettings::native(2)
    };
    // Different native contexts can coexist in the one numeric image table.
    let mut images = Images::new(&vfs, &mut assets, RasterPolicy::Quake2, settings).unwrap();
    let surface = images.raster("env/odd.tga", ImageUse::default()).unwrap();
    let image = images.assets.image(surface.id).unwrap();
    assert_eq!((image.width, image.height), (3, 3));
    assert_eq!(&image.rgba[..4], &[17, 23, 31, 255]);
    let prepared = image.prepared.as_ref().unwrap();
    assert_eq!(
        (prepared.levels[0].width, prepared.levels[0].height),
        (1, 1)
    );
    assert_eq!(prepared.levels[0].rgba.as_ref(), [42, 57, 77, 255]);
    assert_eq!(
        prepared.inverse_intensity.to_bits(),
        (1.0f32 / 2.5).to_bits()
    );
    drop(images);
    let mut images = Images::new(&vfs, &mut assets, RasterPolicy::Quake2, settings).unwrap();
    let sky = images
        .raster(
            "env/odd.tga",
            ImageUse {
                role: ImageRole::CubeSky,
                ..ImageUse::pic()
            },
        )
        .unwrap();
    assert_ne!(sky.id, surface.id);
    let image = images.assets.image(sky.id).unwrap();
    let prepared = image.prepared.as_ref().unwrap();
    assert_eq!(
        (prepared.levels[0].width, prepared.levels[0].height),
        (4, 4)
    );
    assert_eq!(prepared.levels.len(), 1);
    assert_eq!(&prepared.levels[0].rgba[..4], &[17, 23, 31, 255]);
    assert_eq!(prepared.inverse_intensity, 1.0);
}

#[test]
fn native_q3_first_image_flags_and_sampler_win_on_reuse() {
    let root = Fixture::new();
    root.tga("env/first.tga", 4, 4, [20, 30, 40]);
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RasterPolicy::Quake3,
        ImageSettings::native(3),
    )
    .unwrap();
    let usage = ImageUse {
        sampler: Sampler {
            wrap: Wrap::Clamp,
            mipmaps: false,
            ..Sampler::default()
        },
        allow_picmip: false,
        ..ImageUse::default()
    };
    let first = images.raster("ENV\\FIRST.TGA", usage).unwrap();
    let second = images.raster("env/first.tga", ImageUse::default()).unwrap();
    assert_eq!(first, second);
    assert_eq!(second.sampler, usage.sampler);
    assert_eq!(
        images.assets.image(first.id).unwrap().native_sampler,
        Some(usage.sampler)
    );
    assert_eq!(
        images
            .assets
            .image(first.id)
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .levels
            .len(),
        1
    );
    assert_eq!(images.conflicts.len(), 1);
    assert_eq!(images.conflicts[0].first, usage);
    assert_eq!(images.conflicts[0].requested, ImageUse::default());
    images.raster("env/first.tga", ImageUse::default()).unwrap();
    assert_eq!(images.conflicts.len(), 1);
}

#[test]
fn native_q3_picmip_nomip_fast_path_and_weighted_limit_are_explicit() {
    let root = Fixture::new();
    root.tga("env/mipped.tga", 3, 3, [17, 23, 31]);
    root.tga("env/unmipped.tga", 3, 3, [17, 23, 31]);
    root.tga("env/nopic.tga", 4, 4, [17, 23, 31]);
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load();
    let settings = ImageSettings {
        intensity: 2.0,
        gamma_exponent: 2.0,
        ..ImageSettings::native(3)
    };
    let mut images = Images::new(&vfs, &mut assets, RasterPolicy::Quake3, settings).unwrap();
    let mipped = images
        .raster("env/mipped.tga", ImageUse::default())
        .unwrap();
    assert_eq!(
        images
            .assets
            .image(mipped.id)
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .levels[0]
            .width,
        1
    );
    let unmipped = images.raster("env/unmipped.tga", ImageUse::pic()).unwrap();
    let image = images.assets.image(unmipped.id).unwrap();
    let prepared = image.prepared.as_ref().unwrap();
    assert_eq!(
        (prepared.levels[0].width, prepared.levels[0].height),
        (2, 2)
    );
    // Upload32 returns before gamma lookup when only the initial NPOT resize
    // changed dimensions and the final no-mipmap reduction did not change.
    assert_eq!(&prepared.levels[0].rgba[..4], &[17, 23, 31, 255]);
    let nopic = images
        .raster(
            "env/nopic.tga",
            ImageUse {
                allow_picmip: false,
                ..ImageUse::default()
            },
        )
        .unwrap();
    assert_eq!(
        images
            .assets
            .image(nopic.id)
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .levels[0]
            .width,
        4
    );
    drop(images);
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RasterPolicy::Quake3,
        ImageSettings {
            simple_mipmaps: false,
            ..settings
        },
    )
    .unwrap();
    assert!(matches!(
        images.raster("env/mipped.tga", ImageUse::default()),
        Err(ResourceError::Upload(
            qa_render::assets::upload::UploadError::WeightedMipUnsupported
        ))
    ));
}
