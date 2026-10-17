use super::*;
use qa_render::material::resources::{IndexedSkinLayout, IndexedSkinTranslation, IndexedSkinView};

fn source_file() -> Fixture {
    let root = Fixture::new();
    let path = root.0.join("env/native.pcx");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[10..12].copy_from_slice(&2u16.to_le_bytes());
    // Extend the existing two-column PCX fixture to three authored rows.
    bytes.splice(128..131, [16, 96, 31, 111, 7, 8]);
    std::fs::write(path, bytes).unwrap();
    root
}

fn palettes(assets: &mut Assets) -> (qa_render::PaletteId, qa_render::PaletteId) {
    let source: Vec<_> = (0..=255u8).flat_map(|index| [index, 0, 0]).collect();
    let mut target = vec![255; 768];
    for (index, red) in [(42, 159), (43, 48), (44, 144), (45, 63), (46, 0)] {
        target[index * 3..index * 3 + 3].copy_from_slice(&[red, 0, 0]);
    }
    let shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    (
        assets
            .register_palette(PaletteLighting::load(&source, &shades, None, 256).unwrap())
            .unwrap(),
        assets
            .register_palette(PaletteLighting::load(&target, &shades, None, 256).unwrap())
            .unwrap(),
    )
}

fn request(palette: qa_render::PaletteId, target: qa_render::PaletteId) -> IndexedSkinTranslation {
    IndexedSkinTranslation {
        palette,
        target: Some(target),
        indexed_colors: Some([9, 3]),
        rgba_colors: Some([0, 12]),
        crop: Some([0, 0, 2, 2]),
        coordinate_extent: [2, 2],
        upload_extent: Some([4, 2]),
        usage: ImageUse {
            role: ImageRole::Skin,
            ..bitmap_use()
        },
    }
}

fn layout() -> IndexedSkinLayout {
    IndexedSkinLayout {
        storage: Some([4, 4]),
        padding: 0,
        omit_last_row: true,
    }
}

#[test]
fn borrowed_native_members_keep_distinct_ordinals_in_the_shared_image_cache() {
    let root = source_file();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let (palette, target) = palettes(&mut assets);
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        None,
    )
    .unwrap();
    let mut settings = request(palette, target);
    settings.crop = None;
    settings.coordinate_extent = [2, 1];
    settings.upload_extent = Some([2, 1]);
    let first = IndexedSkinView::new([2, 1], &[16, 96]).unwrap();
    let next = IndexedSkinView::new([2, 1], &[31, 111]).unwrap();
    vfs.take_lookup_count();
    let a = images
        .translated_skin("progs/player.mdl", 0, first, settings)
        .unwrap();
    let b = images
        .translated_skin("progs/player.mdl", 1, next, settings)
        .unwrap();
    assert_ne!(a.id, b.id);
    assert_eq!(
        &images.assets.image(a.id).unwrap().rgba[..4],
        &[0, 0, 0, 255]
    );
    assert_eq!(
        &images.assets.image(b.id).unwrap().rgba[..4],
        &[15, 0, 0, 255]
    );
    assert_eq!(
        images
            .assets
            .image(a.id)
            .unwrap()
            .indexed
            .as_ref()
            .unwrap()
            .mip(0)
            .unwrap()
            .indices(),
        &[42, 43]
    );
    assert_eq!(
        images
            .assets
            .image(b.id)
            .unwrap()
            .indexed
            .as_ref()
            .unwrap()
            .mip(0)
            .unwrap()
            .indices(),
        &[44, 45]
    );
    assert_eq!(
        images
            .translated_skin("progs/player.mdl", 0, first, settings)
            .unwrap(),
        a
    );
    assert_eq!(vfs.take_lookup_count(), 0);
    assert!(IndexedSkinView::new([2, 1], &[16]).is_err());
    assert!(IndexedSkinView::new([0, 1], &[]).is_err());
}

#[test]
fn lmp_skin_source_uses_the_existing_indexed_reader_and_translation_path() {
    let root = source_file();
    let bytes: Vec<_> = 2u32
        .to_le_bytes()
        .into_iter()
        .chain(1u32.to_le_bytes())
        .chain([16, 96])
        .collect();
    std::fs::write(root.0.join("env/native.lmp"), bytes).unwrap();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let (palette, target) = palettes(&mut assets);
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        None,
    )
    .unwrap();
    let source = images
        .indexed_skin_source("env/native.lmp", Default::default())
        .unwrap();
    assert_eq!(source.decoded_extent(), [2, 1]);
    assert_eq!(source.indices(), &[16, 96]);
    let mut settings = request(palette, target);
    settings.crop = None;
    settings.coordinate_extent = [2, 1];
    let resolved = images
        .translated_skin("env/native.lmp", 0, source.view(), settings)
        .unwrap();
    assert_eq!(
        images.assets.image(resolved.id).unwrap().logical_size(),
        [2, 1]
    );
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
        &[42, 43]
    );
}

#[test]
fn translated_indexed_skin_retains_padding_dual_colors_palette_mapping_and_cropped_rgba() {
    let root = source_file();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let (palette, target) = palettes(&mut assets);
    let before = assets.counts();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        Some((palette, target)),
    )
    .unwrap();
    let source = images
        .indexed_skin_source("env/native.pcx", layout())
        .unwrap();
    assert_eq!(source.decoded_extent(), [2, 3]);
    assert_eq!(source.storage_extent(), [4, 4]);
    assert_eq!(
        source.indices(),
        [16, 96, 0, 0, 31, 111, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(images.assets.counts(), before);
    vfs.take_lookup_count();
    let settings = request(palette, target);
    let resolved = images
        .translated_skin("env/native.pcx", 0, source.view(), settings)
        .unwrap();
    let image = images.assets.image(resolved.id).unwrap();
    assert_eq!((image.width, image.height), (2, 2));
    assert_eq!(image.logical_size(), [4, 4]);
    assert_eq!(
        &*image.rgba,
        &[0, 0, 0, 255, 207, 0, 0, 255, 15, 0, 0, 255, 192, 0, 0, 255]
    );
    let texture = image.indexed.as_ref().unwrap();
    assert_eq!(texture.coordinate_extent(), [2, 2]);
    let base = texture.mip(0).unwrap();
    assert_eq!((base.width, base.height), (4, 4));
    assert_eq!(
        base.indices(),
        [
            42, 43, 46, 46, 44, 45, 46, 46, 46, 46, 46, 46, 46, 46, 46, 46
        ]
    );
    let prepared = &image.prepared.as_ref().unwrap().levels[0];
    assert_eq!((prepared.width, prepared.height), (4, 2));
    assert_eq!(
        prepared
            .rgba
            .chunks_exact(4)
            .map(|pixel| pixel[0])
            .collect::<Vec<_>>(),
        [0, 0, 207, 207, 15, 15, 192, 192]
    );
    let count = images.assets.counts();
    std::fs::remove_file(root.0.join("env/native.pcx")).unwrap();
    assert_eq!(
        images
            .translated_skin("ENV\\NATIVE.PCX", 0, source.view(), settings)
            .unwrap(),
        resolved
    );
    assert_eq!(images.assets.counts(), count);
    assert_eq!(vfs.take_lookup_count(), 0);
    assert_eq!(source.indices()[4..6], [31, 111]);
}

#[test]
fn translated_skin_cache_keys_keep_each_consumed_crop_color_coordinate_and_upload_policy() {
    let root = source_file();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let (palette, target) = palettes(&mut assets);
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        None,
    )
    .unwrap();
    let source = images
        .indexed_skin_source("env/native.pcx", layout())
        .unwrap();
    let first = request(palette, target);
    let variants = [
        first,
        IndexedSkinTranslation {
            indexed_colors: None,
            ..first
        },
        IndexedSkinTranslation {
            rgba_colors: None,
            ..first
        },
        IndexedSkinTranslation {
            coordinate_extent: [4, 4],
            ..first
        },
        IndexedSkinTranslation {
            crop: Some([1, 0, 1, 2]),
            ..first
        },
        IndexedSkinTranslation {
            upload_extent: Some([2, 2]),
            ..first
        },
        IndexedSkinTranslation {
            target: None,
            ..first
        },
    ];
    let mut ids = Vec::new();
    for settings in variants {
        let image = images
            .translated_skin("env/native.pcx", 0, source.view(), settings)
            .unwrap();
        assert!(!ids.contains(&image.id));
        assert_eq!(
            images
                .translated_skin("env/native.pcx", 0, source.view(), settings)
                .unwrap(),
            image
        );
        ids.push(image.id);
    }
    let native = images
        .assets
        .image(*ids.last().unwrap())
        .unwrap()
        .indexed
        .as_ref()
        .unwrap();
    assert_eq!(&native.mip(0).unwrap().indices()[..2], &[159, 48]);
    assert_eq!(native.coordinate_extent(), [2, 2]);
}

#[test]
fn translated_skin_rollback_retry_and_reload_use_the_existing_recipe_owner() {
    let root = source_file();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let (palette, target) = palettes(&mut assets);
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        None,
    )
    .unwrap();
    let source = images
        .indexed_skin_source("env/native.pcx", layout())
        .unwrap();
    let settings = request(palette, target);
    let prefix = images.assets.counts();
    let mut staged = None;
    let result = images.try_registration(|images| -> Result<(), ResourceError> {
        staged = Some(
            images
                .translated_skin("env/native.pcx", 0, source.view(), settings)?
                .id,
        );
        Err(ResourceError::Asset("reject skin backend"))
    });
    assert!(result.is_err());
    assert_eq!(images.assets.counts(), prefix);
    let admitted = images
        .translated_skin("env/native.pcx", 0, source.view(), settings)
        .unwrap();
    assert_eq!(Some(admitted.id), staged);
    let pixels = images.assets.image(admitted.id).unwrap().rgba.to_vec();
    let mut state = images.into_state();
    let reload = state.begin_reload();
    std::fs::remove_file(root.0.join("env/native.pcx")).unwrap();
    vfs.take_lookup_count();
    let mut images = Images::from_state(&vfs, &mut assets, state);
    images.replay_reload(&reload).unwrap();
    assert_eq!(images.assets.counts().images, prefix.images + 1);
    assert_eq!(vfs.take_lookup_count(), 0);
    let suffix = images
        .translated_skin("env/native.pcx", 0, source.view(), settings)
        .unwrap();
    assert_ne!(suffix.id, admitted.id);
    assert_eq!(&*images.assets.image(suffix.id).unwrap().rgba, pixels);
    let mut state = images.into_state();
    state.restore_reload(reload);
    assets
        .discard_registration_suffix(qa_render::assets::AssetCounts {
            images: prefix.images + 1,
            ..prefix
        })
        .unwrap();
    let mut images = Images::from_state(&vfs, &mut assets, state);
    assert_eq!(
        images
            .translated_skin("env/native.pcx", 0, source.view(), settings)
            .unwrap(),
        admitted
    );
    assert_eq!(vfs.take_lookup_count(), 0);
}

#[test]
fn indexed_skin_invalid_layout_and_derived_inputs_leave_resource_prefix_unchanged() {
    let root = source_file();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let (palette, target) = palettes(&mut assets);
    let prefix = assets.counts();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        None,
    )
    .unwrap();
    for storage in [[1, 3], [2, 2], [0, 4], [8193, 4]] {
        assert!(
            images
                .indexed_skin_source(
                    "env/native.pcx",
                    IndexedSkinLayout {
                        storage: Some(storage),
                        ..Default::default()
                    }
                )
                .is_err()
        );
        assert_eq!(images.assets.counts(), prefix);
    }
    let source = images
        .indexed_skin_source("env/native.pcx", layout())
        .unwrap();
    let settings = request(palette, target);
    for settings in [
        IndexedSkinTranslation {
            indexed_colors: Some([16, 0]),
            ..settings
        },
        IndexedSkinTranslation {
            rgba_colors: Some([0, 255]),
            ..settings
        },
        IndexedSkinTranslation {
            crop: Some([3, 0, 2, 2]),
            ..settings
        },
        IndexedSkinTranslation {
            coordinate_extent: [5, 4],
            ..settings
        },
        IndexedSkinTranslation {
            upload_extent: Some([0, 2]),
            ..settings
        },
        IndexedSkinTranslation {
            target: Some(qa_render::PaletteId(u32::MAX)),
            ..settings
        },
    ] {
        assert!(
            images
                .translated_skin("env/native.pcx", 0, source.view(), settings)
                .is_err()
        );
        assert_eq!(images.assets.counts(), prefix);
    }
    assert!(
        images
            .translated_skin("env/native.pcx", 0, source.view(), request(palette, target))
            .is_ok()
    );
}

#[test]
fn qw_storage_crop_and_coordinate_domains_stay_independent_of_exact_upload_size() {
    let root = source_file();
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let (palette, _) = palettes(&mut assets);
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::QuakeWorld,
        RuleSetId::QuakeWorld,
        ImageSettings::native(RuleSetId::QuakeWorld),
        None,
    )
    .unwrap();
    let source = images
        .indexed_skin_source(
            "env/native.pcx",
            IndexedSkinLayout {
                storage: Some([320, 200]),
                padding: 0,
                omit_last_row: true,
            },
        )
        .unwrap();
    let retained = images
        .indexed_skin_source(
            "env/native.pcx",
            IndexedSkinLayout {
                storage: Some([320, 200]),
                padding: 0,
                omit_last_row: false,
            },
        )
        .unwrap();
    let settings = IndexedSkinTranslation {
        palette,
        target: None,
        indexed_colors: None,
        rgba_colors: None,
        crop: Some([0, 0, 296, 194]),
        coordinate_extent: [296, 194],
        upload_extent: Some([64, 64]),
        usage: ImageUse {
            role: ImageRole::Skin,
            ..bitmap_use()
        },
    };
    let first = images
        .translated_skin("env/native.pcx", 0, source.view(), settings)
        .unwrap();
    let last_row = images
        .translated_skin("env/native.pcx", 0, retained.view(), settings)
        .unwrap();
    assert_ne!(first.id, last_row.id);
    for (id, last) in [(first.id, [0, 0]), (last_row.id, [7, 8])] {
        let image = images.assets.image(id).unwrap();
        assert_eq!([image.width, image.height], [296, 194]);
        assert_eq!(image.logical_size(), [320, 200]);
        let indexed = image.indexed.as_ref().unwrap();
        assert_eq!(indexed.coordinate_extent(), [296, 194]);
        assert_eq!(&indexed.mip(0).unwrap().indices()[640..642], &last);
        let levels = &image.prepared.as_ref().unwrap().levels;
        assert_eq!(levels.len(), 1);
        assert_eq!([levels[0].width, levels[0].height], [64, 64]);
    }
}
