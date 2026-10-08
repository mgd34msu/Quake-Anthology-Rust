use qa_formats::{FormatError, image::*};
use std::{borrow::Cow, io::Cursor};

fn pcx(planes: u8) -> Vec<u8> {
    let mut b = vec![0; 128];
    b[..4].copy_from_slice(&[10, 5, 1, 8]);
    b[8..10].copy_from_slice(&1u16.to_le_bytes());
    b[10..12].copy_from_slice(&1u16.to_le_bytes());
    b[65] = planes;
    b[66..68].copy_from_slice(&4u16.to_le_bytes());
    b
}
fn tga() -> Vec<u8> {
    let mut b = vec![0; 18];
    b[2] = 2;
    b[12] = 2;
    b[14] = 2;
    b[16] = 32;
    b[17] = 32;
    b.extend([
        30, 20, 10, 0, 60, 50, 40, 255, 90, 80, 70, 128, 120, 110, 100, 255,
    ]);
    b
}
fn wal() -> Vec<u8> {
    let mut b = vec![0; 100];
    b[..4].copy_from_slice(b"wall");
    b[32..36].copy_from_slice(&16u32.to_le_bytes());
    b[36..40].copy_from_slice(&16u32.to_le_bytes());
    for i in 0..4 {
        let offset = b.len() as u32;
        b[40 + i * 4..44 + i * 4].copy_from_slice(&offset.to_le_bytes());
        b.extend(vec![
            if i % 2 == 0 { 255 } else { 31 };
            (16 >> i) * (16 >> i)
        ]);
    }
    b
}
#[test]
fn pcx_keeps_padding_out_of_indices_and_reconstructs_rgb_planes() {
    let mut b = pcx(1);
    b.extend([1, 2, 3, 4, 5, 6, 7, 8]);
    b.push(12);
    b.extend(vec![0; 768]);
    let DecodedImage::Indexed(im) = decode(&b, ImageFormat::Pcx, RasterPolicy::Standard).unwrap()
    else {
        panic!()
    };
    assert_eq!(&*im.indices, &[1, 2, 5, 6]);
    let DecodedImage::Indexed(im) = decode(&b, ImageFormat::Pcx, RasterPolicy::Quake3).unwrap()
    else {
        panic!()
    };
    assert_eq!(&*im.indices, &[1, 2, 3, 4]);
    let mut b = pcx(3);
    b.extend([
        1, 2, 0, 0, 3, 4, 0, 0, 5, 6, 0, 0, 7, 8, 0, 0, 9, 10, 0, 0, 11, 12, 0, 0,
    ]);
    let DecodedImage::Rgba(im) = decode(&b, ImageFormat::Pcx, RasterPolicy::Standard).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        im.pixels,
        [1, 3, 5, 255, 2, 4, 6, 255, 7, 9, 11, 255, 8, 10, 12, 255]
    );
    let mut b = pcx(0);
    b.extend([1, 2, 3, 4, 5, 6, 7, 8]);
    b.push(12);
    b.extend(vec![0; 768]);
    assert!(decode(&b, ImageFormat::Pcx, RasterPolicy::Standard).is_ok());
    b[128] = 0xc0;
    assert!(decode(&b, ImageFormat::Pcx, RasterPolicy::Standard).is_err());
}
#[test]
fn original_index_controls_cutouts_before_translation_and_fullbright_split() {
    let mut rgb = vec![0; 768];
    for i in 0..256 {
        rgb[i * 3..i * 3 + 3].copy_from_slice(&[i as u8, 0, 0]);
    }
    let palette = Palette::from_rgb(&rgb).unwrap();
    let image = IndexedImage {
        width: 3,
        height: 1,
        indices: Cow::Borrowed(&[255, 240, 18]),
        palette: None,
    };
    let mut translation = std::array::from_fn(|i| i as u8);
    translation[255] = 19;
    let mut options = PaletteOptions {
        transparent_index: Some(255),
        fullbright: Some(224..=255),
        translation: Some(&translation),
        layer: PaletteLayer::Combined,
    };
    let rgba = image.expand(&palette, &options).unwrap();
    assert_eq!(rgba.pixels, [19, 0, 0, 0, 240, 0, 0, 255, 18, 0, 0, 255]);
    options.layer = PaletteLayer::Ordinary;
    let ordinary = image.expand(&palette, &options).unwrap();
    assert_eq!(
        [ordinary.pixels[3], ordinary.pixels[7], ordinary.pixels[11]],
        [0, 0, 255]
    );
    options.layer = PaletteLayer::Fullbright;
    let bright = image.expand(&palette, &options).unwrap();
    assert_eq!(
        [bright.pixels[3], bright.pixels[7], bright.pixels[11]],
        [0, 255, 0]
    );
    let top = player_translation(9, 3).unwrap();
    assert_eq!(&top[16..32], &(144..160).rev().collect::<Vec<u8>>());
    let mut bytes = vec![0; 16385];
    bytes[16384] = 32;
    let map = Colormap::parse(&bytes).unwrap();
    assert_eq!(map.rows.len(), 64);
    assert_eq!(map.first_fullbright, 224);
}
#[test]
fn tga_orientation_and_rle_policy_follow_native_boundary_options() {
    let b = tga();
    let DecodedImage::Rgba(standard) =
        decode(&b, ImageFormat::Tga, RasterPolicy::Standard).unwrap()
    else {
        panic!()
    };
    assert_eq!(&standard.pixels[..4], &[10, 20, 30, 0]);
    let DecodedImage::Rgba(legacy) = decode(&b, ImageFormat::Tga, RasterPolicy::Quake3).unwrap()
    else {
        panic!()
    };
    assert_eq!(&legacy.pixels[..4], &[70, 80, 90, 128]);
    let mut b = b[..18].to_vec();
    b[2] = 10;
    b.push(0x84);
    b.extend([1, 2, 3, 4]);
    assert!(decode(&b, ImageFormat::Tga, RasterPolicy::Standard).is_err());
    let DecodedImage::Rgba(legacy) = decode(&b, ImageFormat::Tga, RasterPolicy::Quake3).unwrap()
    else {
        panic!()
    };
    assert_eq!(legacy.pixels, [3, 2, 1, 4].repeat(4));
}
#[test]
fn png_palette_alpha_and_sixteen_bit_strip_match_native_conversion() {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut bytes), 2, 1);
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(png::BitDepth::One);
        encoder.set_palette(vec![10, 20, 30, 40, 50, 60]);
        encoder.set_trns(vec![0, 255]);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[0b0100_0000])
            .unwrap();
    }
    let DecodedImage::Rgba(im) = decode(&bytes, ImageFormat::Png, RasterPolicy::Standard).unwrap()
    else {
        panic!()
    };
    assert_eq!(im.pixels, [10, 20, 30, 0, 40, 50, 60, 255]);
    bytes.clear();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut bytes), 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Sixteen);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[0x12, 0xff, 0x34, 0xff, 0x56, 0xff, 0x78, 0xff])
            .unwrap();
    }
    let DecodedImage::Rgba(im) = decode(&bytes, ImageFormat::Png, RasterPolicy::Standard).unwrap()
    else {
        panic!()
    };
    assert_eq!(im.pixels, [0x12, 0x34, 0x56, 0x78]);
    let end = bytes.len() - 1;
    bytes[end] ^= 1;
    assert!(decode(&bytes, ImageFormat::Png, RasterPolicy::Standard).is_err());
}
#[test]
fn native_mip_levels_borrow_their_indices_and_preserve_palette_alpha() {
    let b = wal();
    let texture = MipTexture::parse(&b, MipFormat::Wal).unwrap();
    assert_eq!(texture.name, b"wall");
    for (i, level) in texture.levels.iter().enumerate() {
        assert_eq!(level.len(), (16 >> i) * (16 >> i));
    }
    assert!(std::ptr::eq(texture.levels[0].as_ptr(), b[100..].as_ptr()));
    let palette = Palette::from_rgb(&vec![128; 768]).unwrap();
    for (i, level) in texture.levels.iter().enumerate() {
        let image = IndexedImage {
            width: 16 >> i,
            height: 16 >> i,
            indices: Cow::Borrowed(level),
            palette: None,
        };
        let rgba = image
            .expand(
                &palette,
                &PaletteOptions {
                    transparent_index: Some(255),
                    ..PaletteOptions::default()
                },
            )
            .unwrap();
        assert_eq!(rgba.pixels[3], if i % 2 == 0 { 0 } else { 255 });
    }
    let mut image = RgbaImage {
        width: 8,
        height: 8,
        pixels: vec![255; 8 * 8 * 4],
    };
    for y in 0..8 {
        for x in 0..4 {
            image.pixels[(y * 8 + x) * 4 + 3] = 0;
        }
    }
    let chain = image.mip_chain(MipFilter::Box, true).unwrap();
    assert_eq!(
        chain
            .iter()
            .map(|m| (m.width, m.height))
            .collect::<Vec<_>>(),
        [(4, 4), (2, 2)]
    );
    assert!(chain.iter().all(RgbaImage::mixed_cutout_mask));
    assert_eq!(image.mip_chain(MipFilter::Box, false).unwrap().len(), 3);
}
#[test]
fn wad_qpic_and_font_views_keep_the_source_alive_without_lump_copies() {
    let mut bytes = b"WAD2".to_vec();
    bytes.extend(2i32.to_le_bytes());
    bytes.extend(0i32.to_le_bytes());
    let picture_at = bytes.len();
    bytes.extend(2i32.to_le_bytes());
    bytes.extend(1i32.to_le_bytes());
    bytes.extend([7, 255]);
    let font_at = bytes.len();
    bytes.extend(vec![0; 128 * 128]);
    let directory = bytes.len();
    bytes[8..12].copy_from_slice(&(directory as i32).to_le_bytes());
    for (offset, size, kind, name) in [
        (picture_at, 10i32, 66, b"PIC".as_slice()),
        (font_at, 128 * 128, 64, b"CONCHARS"),
    ] {
        bytes.extend((offset as i32).to_le_bytes());
        bytes.extend(size.to_le_bytes());
        bytes.extend(size.to_le_bytes());
        bytes.extend([kind, 0, 0, 0]);
        let mut text = [0; 16];
        text[..name.len()].copy_from_slice(name);
        bytes.extend(text);
    }
    let wad = Wad::parse(&bytes).unwrap();
    assert_eq!(wad.find(b"pic"), Some(0));
    let WadImage::Image(pic) = wad.image(0).unwrap() else {
        panic!()
    };
    assert_eq!(&*pic.indices, &[7, 255]);
    assert!(std::ptr::eq(
        pic.indices.as_ptr(),
        bytes[picture_at + 8..].as_ptr()
    ));
    let WadImage::Image(font) = wad.image(1).unwrap() else {
        panic!()
    };
    assert_eq!((font.width, font.height), (128, 128));
}
#[test]
fn malformed_raster_data_returns_a_scoped_error() {
    let mut p = pcx(1);
    p.extend([1, 2, 3, 4, 5, 6, 7, 8]);
    let mut l = 2u32.to_le_bytes().to_vec();
    l.extend(2u32.to_le_bytes());
    l.extend([1, 2, 3, 4]);
    let fixtures = [
        (p, ImageFormat::Pcx),
        (tga(), ImageFormat::Tga),
        (l, ImageFormat::Lmp),
    ];
    for (bytes, format) in &fixtures {
        for end in 0..bytes.len() {
            assert!(decode(&bytes[..end], *format, RasterPolicy::Standard).is_err());
        }
    }
    let mut seed = 0x494d4147u32;
    for i in 0..10000 {
        let (bytes, format) = &fixtures[i % fixtures.len()];
        let mut b = bytes.clone();
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let at = seed as usize % b.len();
        b[at] ^= (seed >> 24) as u8;
        let _scoped_result = decode(&b, *format, RasterPolicy::Standard);
    }
    assert!(matches!(
        decode(&[0; 8], ImageFormat::Lmp, RasterPolicy::Standard),
        Err(FormatError::InvalidRange)
    ));
}
