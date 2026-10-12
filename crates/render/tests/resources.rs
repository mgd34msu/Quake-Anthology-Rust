use qa_content::vfs::Vfs;
use qa_core::primitives::RuleSetId;
use qa_render::{
    Assets,
    assets::{Sampler, Wrap},
    material::resources::{ImageRole, ImageSettings, ImageUse, Images, IndexedTarget},
    surface_cache::PaletteLighting,
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn shared_rule_selection_preserves_native_uploads_for_all_five_sources() {
    let root = Fixture::new();
    root.tga("odd.tga", 3, 3, [17, 23, 31]);
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let pixels = |rules| {
        let mut assets = Assets::load().unwrap();
        let mut images = Images::new(
            &vfs,
            &mut assets,
            rules,
            rules,
            ImageSettings::native(rules),
            None,
        )
        .unwrap();
        let resolved = images.raster("odd.tga", ImageUse::default(), None).unwrap();
        let image = images.assets.image(resolved.id).unwrap();
        let prepared = image.prepared.as_ref().unwrap();
        (
            prepared
                .levels
                .iter()
                .map(|level| (level.width, level.height, level.rgba.to_vec()))
                .collect::<Vec<_>>(),
            prepared.inverse_intensity.to_bits(),
        )
    };
    assert_eq!(pixels(RuleSetId::QuakeWorld), pixels(RuleSetId::Quake));
    assert_eq!(
        pixels(RuleSetId::Quake2Rerelease),
        pixels(RuleSetId::Quake2)
    );
    assert!(!pixels(RuleSetId::Quake3).0.is_empty());
}

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
    fn rgba(&self, name: &str, width: u16, height: u16, pixels: &[[u8; 4]]) {
        assert_eq!(pixels.len(), usize::from(width) * usize::from(height));
        let mut bytes = vec![0; 18];
        bytes[2] = 2;
        bytes[12..14].copy_from_slice(&width.to_le_bytes());
        bytes[14..16].copy_from_slice(&height.to_le_bytes());
        bytes[16] = 32;
        for &[r, g, b, a] in pixels {
            bytes.extend([b, g, r, a]);
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
fn raster_target_recipe_shares_images_and_preserves_gl_pixels_with_explicit_masks() {
    let root = Fixture::new();
    let pixels = [[3, 4, 5, 255], [3, 4, 5, 0]];
    root.rgba("env/masked.tga", 2, 1, &pixels);
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let palettes = [17, 42].map(|at| {
        let mut colors = vec![200u8; 256 * 3];
        colors[at * 3..at * 3 + 3].copy_from_slice(&[3, 4, 5]);
        if at == 17 {
            colors[18 * 3..18 * 3 + 3].copy_from_slice(&[3, 4, 5]);
        }
        let shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
        assets
            .register_palette(PaletteLighting::load(&colors, &shades, None, 256).unwrap())
            .unwrap()
    });
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake3,
        RuleSetId::Quake2,
        ImageSettings::native(RuleSetId::Quake2),
        None,
    )
    .unwrap();
    let usage = ImageUse::pic();
    let source = images.raster("env/masked.tga", usage, None).unwrap();
    let targets = [
        IndexedTarget {
            palette: palettes[0],
            transparent_index: None,
        },
        IndexedTarget {
            palette: palettes[0],
            transparent_index: Some(17),
        },
        IndexedTarget {
            palette: palettes[1],
            transparent_index: Some(17),
        },
    ];
    let mut ids = vec![source.id];
    for (target, expected) in targets.into_iter().zip([[17, 17], [18, 17], [42, 17]]) {
        let resolved = images
            .raster("env/masked.tga", usage, Some(target))
            .unwrap();
        assert_eq!(
            images
                .raster("ENV\\MASKED.TGA", usage, Some(target))
                .unwrap(),
            resolved
        );
        assert!(!ids.contains(&resolved.id));
        ids.push(resolved.id);
        let source = images.assets.image(source.id).unwrap();
        let converted = images.assets.image(resolved.id).unwrap();
        assert_eq!(source.rgba, converted.rgba);
        assert_eq!(converted.rgba.as_ref(), pixels.as_flattened());
        assert_eq!(source.prepared, converted.prepared);
        let texture = converted.indexed.as_ref().unwrap();
        assert_eq!(texture.transparent_index(), target.transparent_index);
        assert_eq!(texture.mip(0).unwrap().indices(), expected);
    }
    assert_eq!(images.assets.images().len(), ids.len() + 1); // built-in white image.
    assert!(images.assets.image(source.id).unwrap().indexed.is_none());
}

#[test]
fn indexed_pcx_uses_selected_global_palette_and_explicit_mask() {
    let root = Fixture::new();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let palette = assets.register_palette(palette()).unwrap();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake2,
        RuleSetId::Quake2,
        ImageSettings::native(qa_core::primitives::RuleSetId::Quake2),
        None,
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
    let mut assets = Assets::load().unwrap();
    let palette = assets.register_palette(palette()).unwrap();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake2,
        RuleSetId::Quake2,
        ImageSettings::native(qa_core::primitives::RuleSetId::Quake2),
        None,
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
    let mut assets = Assets::load().unwrap();
    let settings = ImageSettings {
        picmip: 1,
        intensity: 2.5,
        ..ImageSettings::native(qa_core::primitives::RuleSetId::Quake2)
    };
    // Different native contexts can coexist in the one numeric image table.
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake2,
        RuleSetId::Quake2,
        settings,
        None,
    )
    .unwrap();
    let surface = images
        .raster("env/odd.tga", ImageUse::default(), None)
        .unwrap();
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
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake2,
        RuleSetId::Quake2,
        settings,
        None,
    )
    .unwrap();
    let sky = images
        .raster(
            "env/odd.tga",
            ImageUse {
                role: ImageRole::CubeSky,
                ..ImageUse::pic()
            },
            None,
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
fn tga_only_q2_sky_keeps_truecolor_gl_and_selected_palette_cpu_pixels() {
    let root = Fixture::new();
    std::fs::remove_file(root.0.join("env/native.pcx")).unwrap();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let mut assets = Assets::load().unwrap();
        let mut colors = vec![0; 256 * 3];
        colors[17 * 3..18 * 3].copy_from_slice(&[3, 4, 5]);
        colors[18 * 3..19 * 3].copy_from_slice(&[3, 4, 5]);
        let shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
        let palette = assets
            .register_palette(PaletteLighting::load(&colors, &shades, None, 256).unwrap())
            .unwrap();
        let mut images = Images::new(
            &vfs,
            &mut assets,
            rules,
            rules,
            ImageSettings::native(rules),
            None,
        )
        .unwrap();
        let usage = ImageUse {
            role: ImageRole::CubeSky,
            ..ImageUse::pic()
        };
        let id = images
            .indexed_pcx(
                "env/native.pcx",
                palette,
                None,
                Some("env/native.tga"),
                usage,
            )
            .unwrap();
        assert_eq!(
            images
                .indexed_pcx(
                    "env/native.pcx",
                    palette,
                    None,
                    Some("env/native.tga"),
                    usage
                )
                .unwrap(),
            id
        );
        let image = images.assets.image(id).unwrap();
        assert_eq!(image.rgba.as_ref(), &[3, 4, 5, 255]);
        assert_eq!(
            image.prepared.as_ref().unwrap().levels[0].rgba.as_ref(),
            image.rgba.as_ref()
        );
        let indexed = image.indexed.as_ref().unwrap();
        assert_eq!(indexed.mip(0).unwrap().indices(), &[17]);
        assert_eq!(indexed.transparent_index(), None);
        let lighting = images.assets.palette(palette).unwrap();
        assert_eq!(lighting.nearest_index([3, 4, 5], Some(17)), 18);
    }
}

#[test]
fn q2_sky_uses_existing_pcx_when_the_truecolor_alternate_is_missing() {
    let root = Fixture::new();
    std::fs::remove_file(root.0.join("env/native.tga")).unwrap();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let palette = assets.register_palette(palette()).unwrap();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake2,
        RuleSetId::Quake2,
        ImageSettings::native(RuleSetId::Quake2),
        None,
    )
    .unwrap();
    let id = images
        .indexed_pcx(
            "env/native.pcx",
            palette,
            None,
            Some("env/native.tga"),
            ImageUse::pic(),
        )
        .unwrap();
    assert_eq!(
        images
            .assets
            .image(id)
            .unwrap()
            .indexed
            .as_ref()
            .unwrap()
            .mip(0)
            .unwrap()
            .indices(),
        &[17, 255]
    );
    assert!(
        images
            .indexed_pcx(
                "env/absent.pcx",
                palette,
                None,
                Some("env/absent.tga"),
                ImageUse::pic()
            )
            .is_err()
    );
}

#[test]
fn native_q3_first_image_flags_and_sampler_win_on_reuse() {
    let root = Fixture::new();
    root.tga("env/first.tga", 4, 4, [20, 30, 40]);
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake3,
        RuleSetId::Quake3,
        ImageSettings::native(qa_core::primitives::RuleSetId::Quake3),
        None,
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
    let first = images.raster("ENV\\FIRST.TGA", usage, None).unwrap();
    let second = images
        .raster("env/first.tga", ImageUse::default(), None)
        .unwrap();
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
    images
        .raster("env/first.tga", ImageUse::default(), None)
        .unwrap();
    assert_eq!(images.conflicts.len(), 1);
}

#[test]
fn native_q3_picmip_nomip_fast_path_and_weighted_selection() {
    let root = Fixture::new();
    root.tga("env/mipped.tga", 3, 3, [17, 23, 31]);
    root.tga("env/unmipped.tga", 3, 3, [17, 23, 31]);
    root.tga("env/nopic.tga", 4, 4, [17, 23, 31]);
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let settings = ImageSettings {
        intensity: 2.0,
        gamma_exponent: 2.0,
        ..ImageSettings::native(qa_core::primitives::RuleSetId::Quake3)
    };
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake3,
        RuleSetId::Quake3,
        settings,
        None,
    )
    .unwrap();
    let mipped = images
        .raster("env/mipped.tga", ImageUse::default(), None)
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
    let unmipped = images
        .raster("env/unmipped.tga", ImageUse::pic(), None)
        .unwrap();
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
            None,
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
        RuleSetId::Quake3,
        RuleSetId::Quake3,
        ImageSettings {
            simple_mipmaps: false,
            ..settings
        },
        None,
    )
    .unwrap();
    let weighted = images
        .raster("env/mipped.tga", ImageUse::default(), None)
        .unwrap();
    let prepared = images
        .assets
        .image(weighted.id)
        .unwrap()
        .prepared
        .as_ref()
        .unwrap();
    assert_eq!(
        (prepared.levels[0].width, prepared.levels[0].height),
        (1, 1)
    );
    assert_eq!(prepared.levels[0].rgba.as_ref(), [5, 8, 15, 255]);
}
