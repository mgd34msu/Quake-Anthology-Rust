//! Headless comparison against extracted, unchanged original image routines.
use qa_formats::image::{self, DecodedImage, ImageFormat, MipFilter, RasterPolicy, RgbaImage};
use std::{fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("reference directory required")?,
    );
    for case in 0..32 {
        let bytes = fs::read(root.join(format!("{case}.pcx")))?;
        let expected = fs::read(root.join(format!("{case}.pcx.raw")))?;
        for policy in [RasterPolicy::Quake2, RasterPolicy::Quake3] {
            let DecodedImage::Indexed(image) = image::decode(&bytes, ImageFormat::Pcx, policy)
                .map_err(|e| format!("{case}: {e:?}"))?
            else {
                return Err("indexed PCX required".into());
            };
            assert_eq!(image.width.to_le_bytes(), expected[..4]);
            assert_eq!(image.height.to_le_bytes(), expected[4..8]);
            let end = 8 + image.indices.len();
            assert_eq!(&*image.indices, &expected[8..end]);
            let palette = image.palette.ok_or("palette required")?;
            let rgb: Vec<_> = palette
                .0
                .iter()
                .flat_map(|p| p[..3].iter().copied())
                .collect();
            assert_eq!(rgb, expected[end..]);
        }
    }
    let input = fs::read(root.join("mips.bin"))?;
    let expected = fs::read(root.join("mips.raw"))?;
    let mut at = 0;
    let mut out = 0;
    let mut cases = 0;
    while at < input.len() {
        let width = u32::from_le_bytes(input[at..at + 4].try_into()?);
        let height = u32::from_le_bytes(input[at + 4..at + 8].try_into()?);
        at += 8;
        let len = (width * height * 4) as usize;
        let image = RgbaImage {
            width,
            height,
            pixels: input[at..at + len].to_vec(),
        };
        at += len;
        for filter in [MipFilter::Box, MipFilter::Quake3Weighted] {
            let result = image.mip(filter).map_err(|e| format!("{cases}: {e:?}"))?;
            assert_eq!(result.pixels, expected[out..out + len / 4]);
            out += len / 4;
        }
        cases += 1;
    }
    assert_eq!(out, expected.len());
    assert_eq!(cases, 100);
    println!(
        "PASS: 32 original-C PCX fixtures in both legacy profiles; 100 RGBA inputs, both original-C mip filters, byte-identical"
    );
    Ok(())
}
