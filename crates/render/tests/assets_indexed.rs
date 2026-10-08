use qa_render::surface_cache::{IndexedTexture, PaletteLighting};
use qa_render::{Assets, PaletteId};

fn palette(offset: u8) -> PaletteLighting {
    let rgb: Vec<_> = (0..=255u8)
        .flat_map(|i| [i.wrapping_add(offset), i, 255 - i])
        .collect();
    let shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    PaletteLighting::load(&rgb, &shades, None, 224).unwrap()
}

#[test]
fn one_image_retains_original_mips_and_resolved_gl_palette() {
    let mut assets = Assets::load();
    let first = assets.register_palette(palette(0)).unwrap();
    let second = assets.register_palette(palette(13)).unwrap();
    let indices = [7, 255, 19, 44];
    let texture = IndexedTexture::load(2, 2, [&indices, &[19], &[44], &[7]], true).unwrap();
    let image_id = assets.register_indexed_image(texture, first).unwrap();
    let image = assets.image(image_id).unwrap();
    assert_eq!(&image.rgba[..8], &[7, 7, 248, 255, 255, 255, 0, 0]);
    let indexed = image.indexed.as_ref().unwrap();
    assert_eq!(indexed.mip(0).unwrap().indices(), indices);
    assert_eq!(indexed.mip(1).unwrap().indices(), [19]);
    assert_eq!(indexed.mip(2).unwrap().indices(), [44]);
    assert_eq!(indexed.mip(3).unwrap().indices(), [7]);
    // Software can select another presentation without replacing map assets.
    assert_eq!(
        assets.palette(second).unwrap().color(7).to_le_bytes(),
        [20, 7, 248, 255]
    );
    assert!(assets.palette(PaletteId(2)).is_none());
}

#[test]
fn opaque_index_255_is_not_a_transparent_pixel() {
    let mut assets = Assets::load();
    let palette = assets.register_palette(palette(0)).unwrap();
    let texture = IndexedTexture::load(1, 1, [&[255]; 4], false).unwrap();
    let id = assets.register_indexed_image(texture, palette).unwrap();
    assert_eq!(assets.image(id).unwrap().rgba.as_ref(), [255, 255, 0, 255]);
}
