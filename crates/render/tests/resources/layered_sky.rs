use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SKY: AtomicU64 = AtomicU64::new(0);

struct SkyFixture(std::path::PathBuf);
impl SkyFixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "qa-layered-replacement-{}-{}",
            std::process::id(),
            NEXT_SKY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("textures")).unwrap();
        Self(path)
    }
    fn tga(&self, width: u16, height: u16, foreground: [u8; 4]) {
        let mut bytes = vec![0; 18];
        bytes[2] = 2;
        bytes[12..14].copy_from_slice(&width.to_le_bytes());
        bytes[14..16].copy_from_slice(&height.to_le_bytes());
        bytes[16] = 32;
        bytes[17] = 0x28;
        for _ in 0..height {
            for x in 0..width {
                let rgba = if x >= width / 2 {
                    [80, 90, 100, 17]
                } else {
                    match x {
                        0 => [0, 0, 0, 255],
                        1 => foreground,
                        2 => [10, 20, 30, 0],
                        _ => [50, 60, 70, 255],
                    }
                };
                bytes.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
            }
        }
        std::fs::write(self.0.join("textures/sky-replacement.tga"), bytes).unwrap();
    }
}
impl Drop for SkyFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn palette() -> PaletteLighting {
    let colors: Vec<_> = (0..=255u8).flat_map(|index| [index, 40, 80]).collect();
    let shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    PaletteLighting::load(&colors, &shades, None, 256).unwrap()
}

fn mips() -> [Vec<u8>; 4] {
    let mut mips = std::array::from_fn(|mip| vec![0; (256 >> mip) * (128 >> mip)]);
    for row in mips[0].chunks_exact_mut(256) {
        row[..3].copy_from_slice(&[0, 9, 255]);
        row[128..].fill(7);
    }
    mips
}

fn native(mips: &[Vec<u8>; 4]) -> MipTexture<'_> {
    MipTexture {
        name: b"sky-replacement",
        width: 256,
        height: 128,
        shift: 0,
        levels: mips.each_ref().map(Vec::as_slice),
        animation: &[],
        flags: 0,
        contents: 0,
        value: 0,
    }
}

#[test]
fn rgba_layer_split_keeps_black_and_authored_alpha_with_an_opaque_background() {
    let foreground = [[0, 0, 0, 255], [40, 50, 60, 128], [10, 20, 30, 0]];
    let row = [foreground.as_slice(), &[[80, 90, 100, 17]; 3]].concat();
    let pixels = row.repeat(3);
    let split = crate::sky::split_layered_rgba(6, 3, pixels.as_flattened()).unwrap();
    assert_eq!(split.size, 3);
    assert_eq!(&*split.opaque, &[[80, 90, 100, 255]; 9]);
    assert_eq!(&*split.masked, &foreground.repeat(3));
    assert!(crate::sky::split_layered_rgba(4, 1, &[0; 16]).is_err());
    assert!(crate::sky::split_layered_rgba(6, 3, &[0; 71]).is_err());
    assert!(crate::sky::split_layered_rgba(0, 0, &[]).is_err());
}

#[test]
fn layered_replacements_preserve_authored_cpu_layers_and_first_recipe_without_more_io() {
    let root = SkyFixture::new();
    root.tga(512, 256, [40, 50, 60, 128]);
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root.0, 0).unwrap();
    let mips = mips();
    let mip = native(&mips);
    for (level, mask, replaced) in [
        (0, u32::MAX, false),
        (1, 16, false),
        (1, 32, true),
        (2, 32, true),
    ] {
        let mut assets = Assets::load().unwrap();
        let palette = assets.register_palette(palette()).unwrap();
        let settings = ImageSettings {
            override_textures: level,
            texture_overrides: mask,
            ..ImageSettings::native(RuleSetId::Quake)
        };
        let mut images = Images::new(
            &vfs,
            &mut assets,
            RuleSetId::Quake,
            RuleSetId::Quake,
            settings,
            None,
        )
        .unwrap();
        images.set_search_order(ImageSearchOrder::parse(b"tga").unwrap());
        let layers = images.layered_sky(44, &mip, palette).unwrap();
        assert_eq!(images.assets.counts().images, 3);
        assert_eq!(images.assets.counts().materials, 1);
        let background = images.assets.image(layers[0]).unwrap();
        let foreground = images.assets.image(layers[1]).unwrap();
        assert_eq!(background.logical_size(), [128; 2]);
        assert_eq!(foreground.logical_size(), [128; 2]);
        assert!(
            background
                .indexed
                .as_ref()
                .unwrap()
                .mip(0)
                .unwrap()
                .indices()
                .iter()
                .all(|&index| index == 7)
        );
        assert_eq!(
            &foreground
                .indexed
                .as_ref()
                .unwrap()
                .mip(0)
                .unwrap()
                .indices()[..3],
            &[0, 9, 255]
        );
        assert_eq!(
            foreground.indexed.as_ref().unwrap().transparent_index(),
            Some(0)
        );
        if replaced {
            assert_eq!([background.width, background.height], [256; 2]);
            assert_eq!([foreground.width, foreground.height], [256; 2]);
            assert_eq!(&background.rgba[..4], &[80, 90, 100, 255]);
            assert_eq!(
                &foreground.rgba[..12],
                &[0, 0, 0, 255, 40, 50, 60, 128, 10, 20, 30, 0]
            );
            assert_eq!(
                &foreground.prepared.as_ref().unwrap().levels[0].rgba[..12],
                &foreground.rgba[..12]
            );
        } else {
            assert_eq!([background.width, background.height], [128; 2]);
            assert_eq!(&background.rgba[..4], &[7, 40, 80, 255]);
            assert_eq!(
                &foreground.rgba[..12],
                &[7, 40, 80, 0, 9, 40, 80, 255, 255, 40, 80, 255]
            );
        }
        images.set_search_order(ImageSearchOrder::parse(b"").unwrap());
        vfs.take_lookup_count();
        assert_eq!(images.layered_sky(44, &mip, palette).unwrap(), layers);
        assert_eq!(vfs.take_lookup_count(), 0);
    }
}

#[test]
fn invalid_shapes_and_discarded_replacements_retry_without_stale_layer_ids() {
    let root = SkyFixture::new();
    root.tga(384, 128, [40, 50, 60, 128]);
    let mut vfs = Vfs::default();
    let mount = vfs.mount_directory(&root.0, 0).unwrap();
    let mips = mips();
    let mip = native(&mips);
    let mut assets = Assets::load().unwrap();
    let palette = assets.register_palette(palette()).unwrap();
    let prefix = assets.counts();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake,
        RuleSetId::Quake,
        ImageSettings::native(RuleSetId::Quake),
        None,
    )
    .unwrap();
    images.set_search_order(ImageSearchOrder::parse(b"tga").unwrap());
    assert!(matches!(
        images.layered_sky(44, &mip, palette),
        Err(ResourceError::Asset("layered sky requires a 2:1 image"))
    ));
    assert_eq!(images.assets.counts(), prefix);
    let state = images.into_state();
    assert!(vfs.unmount(mount));
    root.tga(512, 256, [40, 50, 60, 128]);
    let mount = vfs.mount_directory(&root.0, 0).unwrap();
    let mark = state.registration_mark();
    let mut images = Images::from_state(&vfs, &mut assets, state);
    let rejected = images.layered_sky(44, &mip, palette).unwrap();
    let mut state = images.into_state();
    state
        .rollback_registration(mark, prefix.images, &mut assets)
        .unwrap();
    assets.discard_registration_suffix(prefix).unwrap();
    assert!(rejected.iter().all(|&id| assets.image(id).is_none()));
    assert!(vfs.unmount(mount));
    root.tga(512, 256, [20, 21, 22, 128]);
    vfs.mount_directory(&root.0, 0).unwrap();
    let mut images = Images::from_state(&vfs, &mut assets, state);
    let retried = images.layered_sky(44, &mip, palette).unwrap();
    assert_eq!(retried, rejected);
    assert_eq!(
        &images.assets.image(retried[1]).unwrap().rgba[4..8],
        &[20, 21, 22, 128]
    );
    assert_eq!(images.assets.counts().images, prefix.images + 2);
}
