use super::*;
use qa_render::material::resources::{ImageSearchOrder, ResolvedImage};

const JPEG: &[u8] = include_bytes!("blue.jpg");

fn settings(level: i32, mask: u32) -> ImageSettings {
    ImageSettings {
        override_textures: level,
        texture_overrides: mask,
        intensity: 1.0,
        ..ImageSettings::native(RuleSetId::Quake2Rerelease)
    }
}

fn mount(root: &Fixture) -> Vfs {
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    vfs
}

fn order(raw: &[u8]) -> ImageSearchOrder {
    ImageSearchOrder::parse(raw).unwrap()
}

fn load(
    vfs: &Vfs,
    rules: RuleSetId,
    settings: ImageSettings,
    formats: &[u8],
    name: &str,
    role: ImageRole,
) -> Result<([u8; 4], [u32; 2], [u32; 2]), ResourceError> {
    let mut assets = Assets::load().unwrap();
    let palette = assets.register_palette(palette()).unwrap();
    let mut images = Images::new(vfs, &mut assets, rules, rules, settings, None).unwrap();
    images.set_search_order(order(formats));
    let resolved = images.raster(
        name,
        ImageUse {
            role,
            ..ImageUse::pic()
        },
        Some(IndexedTarget {
            palette,
            transparent_index: None,
        }),
    )?;
    let image = images.assets.image(resolved.id).unwrap();
    Ok((
        image.rgba[..4].try_into().unwrap(),
        [image.width, image.height],
        image.logical_size(),
    ))
}

#[test]
fn parsed_image_order_uses_shared_named_suffixes_comments_and_legacy_initials() {
    assert_eq!(ImageSearchOrder::default(), order(b"png jpg tga"));
    assert_eq!(order(b"PNG jpg tga jpeg bmp gif"), order(b"pjt JPEG b g"));
    assert_eq!(order(b"jpg JPEG jpg tga png"), order(b"j jpeg t p"));
    assert_eq!(
        order(b"/* skip jpeg */ tga // skip jpg\n png"),
        order(b"t p")
    );
    assert_eq!(order(b" SoUrCe \n"), order(b"source"));
    assert_ne!(order(b"source"), order(b""));
    assert_eq!(order(b"unknown"), order(b""));
    assert_eq!(order(b"\"\" tga"), order(b"tga"));
    assert!(ImageSearchOrder::parse(b"tga \"unfinished").is_err());
}

#[test]
fn rr_original_first_still_falls_back_and_override_levels_keep_distinct_native_branches() {
    let root = Fixture::new();
    root.tga("env/native.tga", 4, 2, [91, 37, 13]);
    root.tga("env/replacement.tga", 4, 2, [91, 37, 13]);
    let vfs = mount(&root);
    let rr = RuleSetId::Quake2Rerelease;
    assert_eq!(
        load(
            &vfs,
            rr,
            settings(0, u32::MAX),
            b"tga",
            "env/native.pcx",
            ImageRole::Pic
        )
        .unwrap()
        .0,
        [7, 11, 19, 255]
    );
    assert_eq!(
        load(
            &vfs,
            rr,
            settings(1, u32::MAX),
            b"tga",
            "env/native.pcx",
            ImageRole::Pic
        )
        .unwrap()
        .0,
        [91, 37, 13, 255]
    );
    assert_eq!(
        load(
            &vfs,
            rr,
            settings(2, 0),
            b"tga",
            "env/native.pcx",
            ImageRole::Pic
        )
        .unwrap()
        .0,
        [7, 11, 19, 255]
    );
    assert_eq!(
        load(
            &vfs,
            rr,
            settings(0, 0),
            b"tga",
            "env/replacement.pcx",
            ImageRole::Pic
        )
        .unwrap()
        .0,
        [91, 37, 13, 255]
    );
    // Level one keeps an explicit truecolor original. Level two omits it when
    // that suffix is absent, then reaches the native PCX fallback.
    assert_eq!(
        load(
            &vfs,
            rr,
            settings(1, u32::MAX),
            b"jpeg",
            "env/native.tga",
            ImageRole::Pic
        )
        .unwrap()
        .0,
        [91, 37, 13, 255]
    );
    assert_eq!(
        load(
            &vfs,
            rr,
            settings(2, u32::MAX),
            b"jpeg",
            "env/native.tga",
            ImageRole::Pic
        )
        .unwrap()
        .0,
        [7, 11, 19, 255]
    );
    assert!(matches!(
        load(
            &vfs,
            rr,
            settings(2, u32::MAX),
            b"jpeg",
            "env/replacement.tga",
            ImageRole::Pic
        ),
        Err(ResourceError::Missing(_))
    ));
    assert_eq!(
        load(
            &vfs,
            RuleSetId::Quake3,
            settings(2, u32::MAX),
            b"jpeg",
            "env/replacement.tga",
            ImageRole::Pic
        )
        .unwrap()
        .0,
        [91, 37, 13, 255]
    );
}

#[test]
fn rr_override_type_bits_are_pic_font_skin_sprite_wall_sky_and_exclude_generated_roles() {
    let root = Fixture::new();
    std::fs::write(root.0.join("env/native.jpeg"), JPEG).unwrap();
    let vfs = mount(&root);
    for (role, bit) in [
        (ImageRole::Pic, 1),
        (ImageRole::Font, 2),
        (ImageRole::Skin, 4),
        (ImageRole::Sprite, 8),
        (ImageRole::Surface, 16),
        (ImageRole::LayeredSky, 32),
        (ImageRole::CubeSky, 32),
    ] {
        let replaced = load(
            &vfs,
            RuleSetId::Quake2Rerelease,
            settings(2, bit),
            b"jpeg",
            "env/native.tga",
            role,
        )
        .unwrap()
        .0;
        assert!(
            replaced[2] > 230 && replaced[0] < 10,
            "{role:?}: {replaced:?}"
        );
        assert_eq!(
            load(
                &vfs,
                RuleSetId::Quake2Rerelease,
                settings(2, !bit),
                b"jpeg",
                "env/native.tga",
                role
            )
            .unwrap()
            .0,
            [3, 4, 5, 255]
        );
    }
    for role in [ImageRole::NativeDefault, ImageRole::Lightmap] {
        assert_eq!(
            load(
                &vfs,
                RuleSetId::Quake2Rerelease,
                settings(2, u32::MAX),
                b"jpeg",
                "env/native.tga",
                role
            )
            .unwrap()
            .0,
            [3, 4, 5, 255]
        );
    }
}

#[test]
fn jpg_and_jpeg_are_separate_suffixes_and_corrupt_preferred_images_stop_search() {
    let root = Fixture::new();
    std::fs::write(root.0.join("env/photo.jpg"), b"broken jpeg").unwrap();
    std::fs::write(root.0.join("env/photo.jpeg"), JPEG).unwrap();
    root.tga("env/photo.tga", 2, 2, [91, 37, 13]);
    let vfs = mount(&root);
    let selected = load(
        &vfs,
        RuleSetId::Quake2Rerelease,
        settings(2, u32::MAX),
        b"jpeg jpg tga",
        "env/photo.tga",
        ImageRole::Pic,
    )
    .unwrap()
    .0;
    assert!(selected[2] > 230 && selected[0] < 10);
    assert!(matches!(
        load(
            &vfs,
            RuleSetId::Quake2Rerelease,
            settings(2, u32::MAX),
            b"jpg jpeg tga",
            "env/photo.tga",
            ImageRole::Pic
        ),
        Err(ResourceError::Format(_))
    ));
    std::fs::remove_file(root.0.join("env/photo.jpg")).unwrap();
    let vfs = mount(&root);
    assert_eq!(
        load(
            &vfs,
            RuleSetId::Quake2Rerelease,
            settings(2, u32::MAX),
            b"jpg jpeg tga",
            "env/photo.tga",
            ImageRole::Pic
        )
        .unwrap()
        .0,
        selected
    );
}

#[test]
fn mutable_order_changes_future_misses_but_cached_rr_type_and_q3_first_flags_remain_owned() {
    let root = Fixture::new();
    for name in ["first", "second"] {
        root.tga(&format!("env/{name}.tga"), 2, 2, [91, 37, 13]);
        std::fs::write(root.0.join(format!("env/{name}.jpeg")), JPEG).unwrap();
    }
    let vfs = mount(&root);
    let mut assets = Assets::load().unwrap();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake3,
        RuleSetId::Quake3,
        settings(2, u32::MAX),
        None,
    )
    .unwrap();
    images.set_search_order(order(b"tga jpeg"));
    let first = images.raster("env/first.tga", bitmap_use(), None).unwrap();
    assert_eq!(
        &images.assets.image(first.id).unwrap().rgba[..4],
        &[91, 37, 13, 255]
    );
    images.set_search_order(order(b"jpeg tga"));
    vfs.take_lookup_count();
    assert_eq!(
        images
            .raster("ENV\\FIRST.TGA", ImageUse::default(), None)
            .unwrap(),
        first
    );
    assert_eq!(vfs.take_lookup_count(), 0);
    assert_eq!(images.conflicts().len(), 1);
    let second = images.raster("env/second.tga", bitmap_use(), None).unwrap();
    assert_ne!(second.id, first.id);
    assert!(images.assets.image(second.id).unwrap().rgba[2] > 230);
    let state = images.into_state();
    let mut images = Images::from_state(&vfs, &mut assets, state);
    images.set_search_order(order(b"tga"));
    vfs.take_lookup_count();
    assert_eq!(
        images.raster("env/second.tga", bitmap_use(), None).unwrap(),
        second
    );
    assert_eq!(vfs.take_lookup_count(), 0);

    let mut assets = Assets::load().unwrap();
    let palette = assets.register_palette(palette()).unwrap();
    let target = Some(IndexedTarget {
        palette,
        transparent_index: None,
    });
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake2Rerelease,
        settings(0, 0),
        None,
    )
    .unwrap();
    let pic = images
        .raster("env/native.pcx", ImageUse::pic(), target)
        .unwrap();
    images.set_search_order(order(b"jpeg"));
    vfs.take_lookup_count();
    assert_eq!(
        images
            .raster("ENV\\NATIVE.TGA", ImageUse::pic(), target)
            .unwrap(),
        pic
    );
    assert_eq!(vfs.take_lookup_count(), 0);
    let font = images
        .raster(
            "env/native.pcx",
            ImageUse {
                role: ImageRole::Font,
                ..ImageUse::pic()
            },
            target,
        )
        .unwrap();
    assert_ne!(pic.id, font.id);
}

#[test]
fn native_pcx_and_wal_keep_original_indices_and_logical_size_with_larger_rgba_replacements() {
    let root = Fixture::new();
    root.tga("env/native.tga", 4, 2, [91, 37, 13]);
    root.tga("env/wall.tga", 32, 32, [91, 37, 13]);
    let mut wal = vec![0; 100];
    wal[32..36].copy_from_slice(&16u32.to_le_bytes());
    wal[36..40].copy_from_slice(&16u32.to_le_bytes());
    for mip in 0..4 {
        let offset = wal.len() as u32;
        wal[40 + mip * 4..44 + mip * 4].copy_from_slice(&offset.to_le_bytes());
        wal.extend(std::iter::repeat_n(10, (16 >> mip) * (16 >> mip)));
    }
    std::fs::write(root.0.join("env/wall.wal"), wal).unwrap();
    let vfs = mount(&root);
    let mut assets = Assets::load().unwrap();
    let palette = assets.register_palette(palette()).unwrap();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake2Rerelease,
        settings(1, u32::MAX),
        None,
    )
    .unwrap();
    images.set_search_order(order(b"tga"));
    let pcx = images
        .indexed_pcx("env/native.pcx", palette, None, None, ImageUse::pic())
        .unwrap();
    let image = images.assets.image(pcx).unwrap();
    assert_eq!([image.width, image.height], [4, 2]);
    assert_eq!(image.logical_size(), [2, 1]);
    assert_eq!(
        image.indexed.as_ref().unwrap().mip(0).unwrap().indices(),
        &[17, 255]
    );
    assert_eq!(&image.rgba[..4], &[91, 37, 13, 255]);
    let wall = images.wal("env/wall.wal", palette, false).unwrap();
    let image = images.assets.image(wall).unwrap();
    assert_eq!([image.width, image.height], [32, 32]);
    assert_eq!(image.logical_size(), [16, 16]);
    assert_eq!(
        image.indexed.as_ref().unwrap().mip(0).unwrap().indices(),
        &[10; 256]
    );
    assert_eq!(&image.rgba[..4], &[91, 37, 13, 255]);
    vfs.take_lookup_count();
    assert_eq!(images.wal("env/wall.wal", palette, false).unwrap(), wall);
    assert_eq!(vfs.take_lookup_count(), 0);
}

#[test]
fn image_suffix_rollback_prunes_selected_recipe_and_retry_keeps_registered_peer() {
    let root = Fixture::new();
    root.tga("env/retained.tga", 2, 2, [91, 37, 13]);
    std::fs::write(root.0.join("env/candidate.jpeg"), JPEG).unwrap();
    let vfs = mount(&root);
    let mut assets = Assets::load().unwrap();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake3,
        RuleSetId::Quake3,
        settings(2, u32::MAX),
        None,
    )
    .unwrap();
    images.set_search_order(order(b"jpeg tga"));
    let retained = images
        .raster("env/retained.tga", ImageUse::pic(), None)
        .unwrap();
    let prefix = images.assets.counts();
    let state = images.into_state();
    let mark = state.registration_mark();
    let mut images = Images::from_state(&vfs, &mut assets, state);
    let rejected = images
        .raster("env/candidate.tga", ImageUse::pic(), None)
        .unwrap();
    let mut state = images.into_state();
    assets.discard_registration_suffix(prefix).unwrap();
    state.rollback_registration(mark, prefix.images);
    assert!(assets.image(rejected.id).is_none());
    let mut images = Images::from_state(&vfs, &mut assets, state);
    vfs.take_lookup_count();
    assert_eq!(
        images
            .raster("env/retained.tga", ImageUse::pic(), None)
            .unwrap(),
        retained
    );
    assert_eq!(vfs.take_lookup_count(), 0);
    let retry: ResolvedImage = images
        .raster("env/candidate.tga", ImageUse::pic(), None)
        .unwrap();
    assert_eq!(retry.id, rejected.id);
    assert!(images.assets.image(retry.id).unwrap().rgba[2] > 230);
    assert!(vfs.take_lookup_count() > 0);
}

#[test]
fn raster_native_pcx_and_forced_fallback_translate_from_source_palette_once() {
    let root = Fixture::new();
    let vfs = mount(&root);
    for (name, level) in [("env/native.pcx", 0), ("env/native.tga", 2)] {
        let mut assets = Assets::load().unwrap();
        let mut source_rgb = vec![0; 768];
        source_rgb[17 * 3..18 * 3].copy_from_slice(&[91, 37, 13]);
        let mut target_rgb = vec![0; 768];
        target_rgb[42 * 3..43 * 3].copy_from_slice(&[91, 37, 13]);
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
            RuleSetId::Quake2Rerelease,
            RuleSetId::Quake2Rerelease,
            settings(level, u32::MAX),
            Some((source, target)),
        )
        .unwrap();
        // The forced branch omits the existing TGA and reaches native PCX.
        images.set_search_order(order(b"jpeg"));
        let resolved = images
            .raster(
                name,
                ImageUse::pic(),
                Some(IndexedTarget {
                    palette: target,
                    transparent_index: None,
                }),
            )
            .unwrap();
        let image = images.assets.image(resolved.id).unwrap();
        assert_eq!(image.logical_size(), [2, 1]);
        assert_eq!(&image.rgba[..4], &[91, 37, 13, 255]);
        assert_eq!(
            image.indexed.as_ref().unwrap().mip(0).unwrap().indices(),
            &[42, 0]
        );
        vfs.take_lookup_count();
        assert_eq!(
            images
                .indexed_pcx("env/native.pcx", source, None, None, ImageUse::pic())
                .unwrap(),
            resolved.id
        );
        assert_eq!(vfs.take_lookup_count(), 0);
    }
}

#[test]
fn embedded_and_wad_replacements_keep_authored_software_data_and_obey_override_controls() {
    let root = Fixture::new();
    std::fs::create_dir_all(root.0.join("textures")).unwrap();
    std::fs::create_dir_all(root.0.join("gfx")).unwrap();
    root.tga("textures/wall.tga", 32, 32, [91, 37, 13]);
    root.tga("gfx/conchars.tga", 256, 256, [91, 37, 13]);
    let font: Vec<_> = [0, 17, 255].into_iter().cycle().take(128 * 128).collect();
    root.wad("gfx.wad", &[(b"conchars", 66, 0, font.clone())]);
    let mip = qa_formats::image::MipTexture {
        name: b"wall",
        width: 16,
        height: 16,
        shift: 0,
        levels: [&[17; 256], &[17; 64], &[17; 16], &[17; 4]],
        animation: &[],
        flags: 0,
        contents: 0,
        value: 0,
    };
    let vfs = mount(&root);
    for (level, mask, replacement) in [
        (1, u32::MAX, true),
        (1, !(16 | 2), false),
        (0, u32::MAX, false),
    ] {
        let mut assets = Assets::load().unwrap();
        let palette = assets.register_palette(palette()).unwrap();
        let mut images = Images::new(
            &vfs,
            &mut assets,
            RuleSetId::Quake,
            RuleSetId::Quake,
            ImageSettings {
                override_textures: level,
                texture_overrides: mask,
                ..ImageSettings::native(RuleSetId::Quake)
            },
            None,
        )
        .unwrap();
        images.set_search_order(order(b"tga"));
        vfs.take_lookup_count();
        let embedded = images.embedded(7, &mip, palette, false).unwrap();
        if !replacement {
            assert_eq!(vfs.take_lookup_count(), 0);
        }
        let image = images.assets.image(embedded).unwrap();
        assert_eq!(image.logical_size(), [16, 16]);
        assert_eq!(
            [image.width, image.height],
            if replacement { [32, 32] } else { [16, 16] }
        );
        assert_eq!(
            image.indexed.as_ref().unwrap().mip(0).unwrap().indices(),
            &[17; 256]
        );
        if replacement {
            assert_eq!(&image.rgba[..4], &[91, 37, 13, 255]);
        }
        let usage = ImageUse {
            role: ImageRole::Font,
            ..bitmap_use()
        };
        let wad = images
            .indexed_wad("gfx.wad", b"conchars", palette, Some(0), usage)
            .unwrap();
        let image = images.assets.image(wad.id).unwrap();
        assert_eq!(image.logical_size(), [128, 128]);
        assert_eq!(
            [image.width, image.height],
            if replacement { [256, 256] } else { [128, 128] }
        );
        assert_eq!(
            image.indexed.as_ref().unwrap().mip(0).unwrap().indices(),
            font
        );
        assert_eq!(image.indexed.as_ref().unwrap().transparent_index(), Some(0));
        if replacement {
            assert_eq!(&image.rgba[..4], &[91, 37, 13, 255]);
        }
        vfs.take_lookup_count();
        assert_eq!(images.embedded(7, &mip, palette, false).unwrap(), embedded);
        assert_eq!(
            images
                .indexed_wad("gfx.wad", b"conchars", palette, Some(0), usage)
                .unwrap(),
            wad
        );
        assert_eq!(vfs.take_lookup_count(), 0);
    }
}
