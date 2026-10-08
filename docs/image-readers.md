# Shared image readers

`qa_formats::image` reads QPIC LMP, WAD2/WAD3, BSP mip textures, WAL, PCX,
TGA, BMP, JPEG, PNG and the first GIF frame. Palettes, colormap rows, mip
indices and WAD lump payloads borrow the input where possible. Compressed
rasters decode once into contiguous indices or RGBA bytes. There is one mip
texture implementation for BSPs and standalone resources, and one checked
binary cursor for BSP, model and image readers.

Raster policy is chosen at the file boundary. Standard PCX reconstructs RGB
planes and strips row padding. Original Q2/Q3 PCX policies retain tight indexed
rows and their size limits. Legacy zero-plane screenshot headers are admitted
by the standard reader only with the single-plane palette marker. Standard TGA
honours origin bits; Q3 policy preserves its original bottom-up convention and
clamped oversized RLE packets. BMP has standard padded rows and an explicit
original Q3 policy. Invalid sections and RLE counts return format errors.

Palette expansion applies original-index transparency before player colour
translation, and can separate ordinary and fullbright pixels. The colormap
retains all 64 lookup rows and its fullbright threshold. Load-time box and Q3
weighted mip generation operate on RGBA. The C port's cutout fix stops a mip
chain before its mixed GT666 alpha mask disappears; the renderer must clamp
sampling to the retained levels when it consumes this chain in R3.

PNG expands low-bit palettes and transparency and strips 16-bit channels to
their high byte. JPEG uses a Rust decoder, including the native CMYK loading
convention. GIF produces its first frame on a transparent canvas. The PNG,
JPEG and GIF codec dependencies are Rust; these readers link no C image library.
Decoded image buffers have a 256 MiB admission limit. No reader assigns a
content fingerprint or writes to the asset tree.

## Verification

```sh
cargo test -p qa-formats --test images
cargo run --release -p qa-content --example images -- "$QA_QFILES"
python3 tools/check_images.py --qsrc "$QA_QSRC" --output "$QA_EVIDENCE"
```

On 2026-10-07 the headless corpus run decoded all 36,831 physical image files,
including archive duplicates, and 49,625 image/lump/palette items. Formats were
8,385 JPEG, 4,515 PNG, 6,572 TGA, 3,200 PCX, 20 BMP, 7 GIF, 13,861 WAL, 37 WAD,
226 QPIC LMP, 2 palettes, 2 colormaps and 4 registration bitmaps. WAD image
lumps account for the remaining items. No retail image was modified.

THE-852 covers the twelve entries initially rejected by a one-plane assumption:
ten retail planar PCX entries and two zero-plane screenshots. Fixtures check
planar reconstruction, padding, the native indexed policies and malformed RLE.
The comparison tool compiles unchanged original Q2 `LoadPCX`/`GL_MipMap` and
Q3 `R_MipMap2`. Thirty-two valid indexed PCX fixtures match both legacy profiles
byte for byte, including their palettes; 100 seeded RGBA inputs match both mip
filters byte for byte. Tests also check PNG palette alpha, high-byte stripping,
WAD borrowing, palette translation and cutout retention. Truncations and 10,000
seeded raster mutations return without panicking.

These are reader and conversion checks. JPEG decode was admitted across the
corpus, without a claim of bit-identical DCT output to the C library. THE-776
remains In Progress: the e1m1 grate screenshot and texture filtering require
the R3 renderer and a private gameplay run. No installation is qualified here.

## Sources

Behaviour follows original `quake/WinQuake/wad.c`, `gl_draw.c`, `gl_rmisc.c`,
`quake-2/ref_gl/gl_image.c`, and `quake-iii-arena/code/renderer/tr_image.c`.
The C port's `src/formats/image` supplies proven format and conversion fixes;
`src/render/resources.c` supplies cutout mip retention.

Authorised retired parsing inspected:
`muse-final:crates/content/src/images/indexed.rs`,
`muse-final:crates/content/src/images/mip.rs`,
`muse-final:crates/content/src/images/palette.rs`,
`muse-final:crates/content/src/images/wad.rs`,
`muse-final:crates/content/src/images/q3_tga.rs`,
`muse-final:crates/content/src/images/bmp.rs`, and
`muse-final:crates/content/src/images/tga.rs`.
