use qa_content::vfs::Vfs;
use qa_core::primitives::RuleSetId;
use qa_render::{
    Assets,
    assets::{Filter, Sampler, Wrap},
    material::resources::{
        ImageRole, ImageSettings, ImageUse, Images, IndexedTarget, ResourceError,
    },
    surface_cache::{IndexedTexture, PaletteLighting},
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn detached_image_state_keeps_recipe_ids_native_flags_and_conflict_history() {
    let root = Fixture::new();
    root.tga("env/detached.tga", 3, 3, [17, 23, 31]);
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake3,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        None,
    )
    .unwrap();
    let usage = ImageUse::pic();
    let first = images.raster("env/detached.tga", usage, None).unwrap();
    images
        .raster("env/detached.tga", ImageUse::default(), None)
        .unwrap();
    assert_eq!(images.conflicts().len(), 1);
    let image_count = images.assets.images().len();
    let state = images.into_state();
    std::fs::remove_file(root.0.join("env/detached.tga")).unwrap();
    let mut images = Images::from_state(&vfs, &mut assets, state);
    let second = images.raster("ENV\\DETACHED.TGA", usage, None).unwrap();
    assert_eq!(first, second);
    assert_eq!(images.assets.images().len(), image_count);
    assert_eq!(images.conflicts().len(), 1);
    images
        .raster("env/detached.tga", ImageUse::default(), None)
        .unwrap();
    assert_eq!(images.conflicts().len(), 1);
    assert_eq!(images.conflicts()[0].first, usage);
}

#[test]
fn image_policy_identity_keeps_invalid_payloads_and_signed_zero_exact() {
    let mut settings = ImageSettings::native(RuleSetId::Quake2);
    settings.intensity = f32::from_bits(0x7fc0_0012);
    assert_eq!(settings, settings);
    let mut different = settings;
    different.intensity = f32::from_bits(0x7fc0_0013);
    assert_ne!(settings, different);
    settings.intensity = 0.0;
    different.intensity = -0.0;
    assert_ne!(settings, different);
}

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
    fn lmp(&self, name: &str, width: u32, height: u32, indices: &[u8]) {
        std::fs::write(self.0.join(name), qpic(width, height, indices)).unwrap();
    }
    fn wad(&self, name: &str, lumps: &[(&[u8], u8, u8, Vec<u8>)]) {
        let mut bytes = b"WAD2".to_vec();
        bytes.extend_from_slice(&(lumps.len() as i32).to_le_bytes());
        let directory = 12 + lumps.iter().map(|entry| entry.3.len()).sum::<usize>();
        bytes.extend_from_slice(&(directory as i32).to_le_bytes());
        for (_, _, _, data) in lumps {
            bytes.extend_from_slice(data);
        }
        let mut offset = 12;
        for (name, kind, compression, data) in lumps {
            assert!(name.len() <= 16);
            bytes.extend_from_slice(&(offset as i32).to_le_bytes());
            bytes.extend_from_slice(&(data.len() as i32).to_le_bytes());
            bytes.extend_from_slice(&(data.len() as i32).to_le_bytes());
            bytes.extend_from_slice(&[*kind, *compression, 0, 0]);
            let mut raw_name = [0; 16];
            raw_name[..name.len()].copy_from_slice(name);
            bytes.extend_from_slice(&raw_name);
            offset += data.len();
        }
        std::fs::write(self.0.join(name), bytes).unwrap();
    }
}
fn qpic(width: u32, height: u32, indices: &[u8]) -> Vec<u8> {
    assert_eq!(indices.len(), width as usize * height as usize);
    let mut bytes = width.to_le_bytes().to_vec();
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(indices);
    bytes
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

fn bitmap_use() -> ImageUse {
    ImageUse {
        sampler: Sampler {
            wrap: Wrap::Clamp,
            filter: Filter::Nearest,
            mipmaps: false,
        },
        ..ImageUse::pic()
    }
}

#[test]
fn indexed_qpic_and_wad_share_native_registration_upload_and_detached_recipe_cache() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let root = Fixture::new();
        let pixels = [0, 17, 255];
        root.lmp("env/pic.lmp", 3, 1, &pixels);
        let font: Vec<_> = pixels.into_iter().cycle().take(128 * 128).collect();
        root.wad("env/font.wad", &[(b"conchars", 66, 0, font.clone())]);
        let mut vfs = Vfs::default();
        vfs.mount_directory(&root.0, 0).unwrap();
        let mut assets = Assets::load().unwrap();
        let palette = assets.register_palette(palette()).unwrap();
        let mut images = Images::new(
            &vfs,
            &mut assets,
            rules,
            rules,
            ImageSettings::native(rules),
            None,
        )
        .unwrap();
        let usage = bitmap_use();
        let params = images.upload_params(usage).unwrap();
        for (indices, width, height, mask, resolved) in [
            (
                pixels.as_slice(),
                3,
                1,
                None,
                images
                    .indexed_lmp("env/pic.lmp", palette, None, usage)
                    .unwrap(),
            ),
            (
                font.as_slice(),
                128,
                128,
                Some(0),
                images
                    .indexed_wad("env/font.wad", b"conchars", palette, Some(0), usage)
                    .unwrap(),
            ),
        ] {
            assert_eq!(resolved.sampler, usage.sampler);
            let oracle = images
                .assets
                .register_indexed_image(
                    IndexedTexture::load_base(width, height, indices, mask).unwrap(),
                    palette,
                )
                .unwrap();
            images.assets.prepare_image(oracle, params).unwrap();
            let actual = images.assets.image(resolved.id).unwrap();
            let oracle = images.assets.image(oracle).unwrap();
            assert_eq!((actual.width, actual.height), (width, height));
            assert_eq!(actual.rgba, oracle.rgba);
            assert_eq!(actual.prepared, oracle.prepared);
            assert_eq!(actual.native_sampler, oracle.native_sampler);
            let texture = actual.indexed.as_ref().unwrap();
            assert_eq!(texture.mip(0).unwrap().indices(), indices);
            assert_eq!(texture.transparent_index(), mask);
        }
        let file = images
            .indexed_lmp("env/pic.lmp", palette, None, usage)
            .unwrap();
        let wad = images
            .indexed_wad("env/font.wad", b"conchars", palette, Some(0), usage)
            .unwrap();
        let count = images.assets.images().len();
        let state = images.into_state();
        std::fs::remove_file(root.0.join("env/pic.lmp")).unwrap();
        std::fs::remove_file(root.0.join("env/font.wad")).unwrap();
        let mut images = Images::from_state(&vfs, &mut assets, state);
        assert_eq!(
            images
                .indexed_lmp("ENV\\PIC.LMP", palette, None, usage)
                .unwrap(),
            file
        );
        assert_eq!(
            images
                .indexed_wad("ENV\\FONT.WAD", b"conchars", palette, Some(0), usage)
                .unwrap(),
            wad
        );
        assert_eq!(images.assets.images().len(), count);
    }
}

#[test]
fn wad_recipe_keeps_exact_lump_names_masks_and_samplers_separate() {
    let root = Fixture::new();
    root.wad(
        "env/pics.wad",
        &[
            (b"folder\\pic", 66, 0, qpic(2, 1, &[17, 0])),
            (b"folder/pic", 66, 0, qpic(2, 1, &[255, 17])),
        ],
    );
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let palette = assets.register_palette(palette()).unwrap();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake,
        RuleSetId::Quake,
        ImageSettings::native(RuleSetId::Quake),
        None,
    )
    .unwrap();
    let usage = bitmap_use();
    let backslash = images
        .indexed_wad("env/pics.wad", b"folder\\pic", palette, None, usage)
        .unwrap();
    let slash = images
        .indexed_wad("env/pics.wad", b"folder/pic", palette, None, usage)
        .unwrap();
    assert_ne!(backslash, slash);
    let folded = images
        .indexed_wad("env/pics.wad", b"FOLDER\\PIC", palette, None, usage)
        .unwrap();
    assert_ne!(backslash.id, folded.id); // Native folded lookup retains exact recipe identities.
    for (resolved, expected) in [(backslash, [17, 0]), (slash, [255, 17]), (folded, [17, 0])] {
        assert_eq!(
            images
                .assets
                .image(resolved.id)
                .unwrap()
                .indexed
                .as_ref()
                .unwrap()
                .mip(0)
                .unwrap()
                .indices(),
            expected
        );
    }
    let masked = images
        .indexed_wad("env/pics.wad", b"folder\\pic", palette, Some(0), usage)
        .unwrap();
    assert_ne!(masked.id, backslash.id);
    assert_eq!(images.assets.image(masked.id).unwrap().rgba[7], 0);
    assert_eq!(images.assets.image(backslash.id).unwrap().rgba[7], 255);
    let mut repeat = usage;
    repeat.sampler.wrap = Wrap::Repeat;
    let repeated = images
        .indexed_wad("env/pics.wad", b"folder\\pic", palette, None, repeat)
        .unwrap();
    assert_ne!(repeated.id, backslash.id);
    assert_eq!(repeated.sampler.wrap, Wrap::Repeat);
}

#[test]
fn indexed_qpic_and_wad_use_shared_palette_remap_without_changing_source_rgba() {
    let root = Fixture::new();
    root.lmp("env/remap.lmp", 2, 1, &[17, 0]);
    root.wad("env/remap.wad", &[(b"pic", 66, 0, qpic(2, 1, &[17, 0]))]);
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let mut source_rgb = vec![200; 768];
    source_rgb[17 * 3..18 * 3].copy_from_slice(&[7, 11, 19]);
    let mut target_rgb = vec![99; 768];
    target_rgb[42 * 3..43 * 3].copy_from_slice(&[7, 11, 19]);
    let shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    let source = assets
        .register_palette(PaletteLighting::load(&source_rgb, &shades, None, 256).unwrap())
        .unwrap();
    let target = assets
        .register_palette(PaletteLighting::load(&target_rgb, &shades, None, 256).unwrap())
        .unwrap();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake,
        RuleSetId::Quake2,
        ImageSettings::native(RuleSetId::Quake2),
        Some((source, target)),
    )
    .unwrap();
    let usage = bitmap_use();
    let file = images
        .indexed_lmp("env/remap.lmp", source, Some(0), usage)
        .unwrap();
    let wad = images
        .indexed_wad("env/remap.wad", b"pic", source, Some(0), usage)
        .unwrap();
    for resolved in [file, wad] {
        let image = images.assets.image(resolved.id).unwrap();
        assert_eq!(&image.rgba[..4], &[7, 11, 19, 255]);
        assert_eq!(image.rgba[7], 0);
        let texture = image.indexed.as_ref().unwrap();
        assert_eq!(texture.mip(0).unwrap().indices(), [42, 0]);
        assert_eq!(texture.transparent_index(), Some(0));
    }
}

#[test]
fn indexed_resource_admission_preserves_file_limits_and_decoder_errors() {
    let root = Fixture::new();
    std::fs::write(root.0.join("env/bad.lmp"), [0; 8]).unwrap();
    root.wad("env/bad.wad", &[(b"compressed", 66, 1, qpic(1, 1, &[17]))]);
    for path in ["env/large.lmp", "env/large.wad"] {
        std::fs::File::create(root.0.join(path))
            .unwrap()
            .set_len(16 * 1024 * 1024 + 1)
            .unwrap();
    }
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let palette = assets.register_palette(palette()).unwrap();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake,
        RuleSetId::Quake,
        ImageSettings::native(RuleSetId::Quake),
        None,
    )
    .unwrap();
    assert!(matches!(
        images.indexed_lmp("env/large.lmp", palette, None, bitmap_use()),
        Err(ResourceError::Size)
    ));
    assert!(matches!(
        images.indexed_wad("env/large.wad", b"pic", palette, None, bitmap_use()),
        Err(ResourceError::Size)
    ));
    assert!(matches!(
        images.indexed_lmp("env/bad.lmp", palette, None, bitmap_use()),
        Err(ResourceError::Format(_))
    ));
    assert!(matches!(
        images.indexed_wad("env/bad.wad", b"compressed", palette, None, bitmap_use()),
        Err(ResourceError::Format(qa_formats::FormatError::Unsupported))
    ));
    assert!(matches!(
        images.indexed_wad("env/bad.wad", b"missing", palette, None, bitmap_use()),
        Err(ResourceError::Asset("missing indexed WAD lump"))
    ));
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
    assert_eq!(images.conflicts().len(), 1);
    assert_eq!(images.conflicts()[0].first, usage);
    assert_eq!(images.conflicts()[0].requested, ImageUse::default());
    images
        .raster("env/first.tga", ImageUse::default(), None)
        .unwrap();
    assert_eq!(images.conflicts().len(), 1);
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

#[test]
fn generated_native_default_keeps_pixels_first_sampler_and_indexed_profile_capability() {
    use qa_render::assets::StageTexture;
    use qa_render::material::{resources::ImagesState, world_load::native_default_shader};
    for presentation in [RuleSetId::Quake3, RuleSetId::Quake] {
        let vfs = Vfs::default();
        let mut assets = Assets::load().unwrap();
        let colors: Vec<_> = (0..256).flat_map(|value| [value as u8; 3]).collect();
        let shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
        let palette = assets
            .register_palette(PaletteLighting::load(&colors, &shades, None, 256).unwrap())
            .unwrap();
        let mut settings = ImageSettings::native(presentation);
        settings.picmip = 2;
        let mut state = ImagesState::new(RuleSetId::Quake3, presentation, settings, None).unwrap();
        let target = Some(IndexedTarget {
            palette,
            transparent_index: None,
        });
        let fallback = native_default_shader(
            &mut state,
            &mut assets,
            target,
            None,
            qa_render::material::ShaderCompilePolicy::default(),
        )
        .unwrap();
        let binding = assets.shader_binding(fallback).unwrap();
        let material = assets.material(binding.material).unwrap();
        assert_eq!(material.settings.sort, 3.0);
        assert_eq!(material.settings.cull, qa_render::shader::Cull::Front);
        assert_eq!(
            material.stages[0].rgb_gen,
            qa_render::shader::RgbGen::IdentityLighting
        );
        assert_eq!(
            material.stages[0].alpha_gen,
            qa_render::shader::AlphaGen::Identity
        );
        assert!(material.stages[0].depth_write);
        let StageTexture::Image(image) = material.stages[0].texture else {
            panic!("default image")
        };
        let source = assets.image(image).unwrap();
        assert_eq!((source.width, source.height), (16, 16));
        assert_eq!(&source.rgba[..4], &[255; 4]);
        assert_eq!(
            &source.rgba[(8 * 16 + 8) * 4..(8 * 16 + 8) * 4 + 4],
            &[32; 4]
        );
        assert_eq!(source.indexed.as_ref().unwrap().transparent_index(), None);
        let prepared = source.prepared.as_ref().unwrap();
        assert_eq!(
            (prepared.levels[0].width, prepared.levels[0].height),
            (16, 16)
        );
        assert_eq!(prepared.levels.len(), 5);
        let counts = assets.counts();
        let mut images = Images::from_state(&vfs, &mut assets, state);
        // The native first image wins even when this ordinary caller has no
        // indexed raster target, as in a source-Q3 indexed model profile.
        let ordinary = images.raster("*default", ImageUse::pic(), None).unwrap();
        assert_eq!(ordinary.id, image);
        assert_eq!(ordinary.sampler, Sampler::default());
        assert_eq!(images.assets.counts(), counts);
        assert_eq!(
            images.raster("*default", ImageUse::pic(), None).unwrap(),
            ordinary
        );
        if presentation == RuleSetId::Quake3 {
            assert_eq!(images.conflicts().len(), 1);
        }
        // A seeded capability also survives a foreign geometry-source borrow.
        let mut state = ImagesState::new(RuleSetId::Quake2, presentation, settings, None).unwrap();
        assert_eq!(
            native_default_shader(
                &mut state,
                images.assets,
                target,
                Some(fallback),
                qa_render::material::ShaderCompilePolicy::default(),
            )
            .unwrap(),
            fallback
        );
        let mut mixed = Images::from_state(&vfs, images.assets, state);
        assert_eq!(
            mixed
                .raster("*default", ImageUse::default(), None)
                .unwrap()
                .id,
            image
        );
        assert_eq!(mixed.assets.counts(), counts);
        let white = mixed
            .assets
            .shader_binding(qa_render::ShaderBindingId(0))
            .unwrap();
        let white = mixed.assets.material(white.material).unwrap();
        assert_eq!(white.settings.sort, 0.0);
        assert_eq!(white.settings.cull, qa_render::shader::Cull::None);
        assert_eq!(
            white.stages[0].rgb_gen,
            qa_render::shader::RgbGen::ExactVertex
        );
    }
}
