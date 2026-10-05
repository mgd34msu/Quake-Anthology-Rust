//! Console-charset text for the windowed startup menu.
//!
//! Donor provenance: `src/text/atlas.ts` (`classicCharset`: a 16 by 16
//! cell atlas over `conchars`), `src/text/ui.ts` (`UiTextRenderer.draw`
//! lays out with `layoutText` then draws one `stretchPic` per visible glyph
//! with atlas UVs), and `src/app/bootstrap/menu-font.ts` (`loadMenuFont`:
//! Q1 `conchars` from `gfx.wad` plus `gfx/palette.lmp`, Q2
//! `pics/conchars.pcx` with the `pics/colormap.pcx` palette, Q3
//! `gfx/2d/bigchars` through the texture loader). [`resolve_menu_charset`]
//! follows that shape over the installed catalog mounts; when no content
//! (or no charset) is installed the menu falls back to the synthetic
//! [`conchars_rgba`] atlas: white 8 by 8 glyphs on transparent cells,
//! tinted per run through vertex colors.
//!
//! Donor provenance for proportional text: `src/app/bootstrap/menu-font.ts`
//! (`loadMenuTypography`: rerelease TrueType body/title atlases, else the
//! Q3 `menu/art/font1_prop.tga` proportional atlas, else the classic
//! charset). Glyphs laid out from those atlases carry per-codepoint
//! x/y/width/height/advance cells plus the atlas picture handle in
//! [`GlyphQuad::font`]; [`glyph_batches`] groups consecutive same-atlas
//! quads into emit-order runs so each run binds its own upload.
//!
//! Diagnosis (blank menu labels): the menu capture drew every text run as
//! one measured solid bar and dropped UVs at emit time, so no per-glyph
//! pictures ever reached the GL batches. The faithful shape kept here is
//! layout then per-glyph quads: the resolved (or synthetic) atlas texels
//! upload once beside the 1x1 white image, and [`glyph_batches`] turns
//! captured glyph quads into textured overlay batches over those uploads.

use qa_client::render::types::AlphaTest;
use qa_client::render::types::BatchLighting;
use qa_client::render::types::BatchPrimitive;
use qa_client::render::types::BatchVertices;
use qa_client::render::types::BlendFactor;
use qa_client::render::types::CullFace;
use qa_client::render::types::DepthTest;
use qa_client::render::types::DrawBatch;
use qa_client::render::types::ImageLevel;
use qa_client::render::types::ImageResourceOperation;
use qa_client::render::types::RenderImage;
use qa_client::render::types::RenderState;
use qa_client::render::types::RenderVertex;
use qa_client::render::types::RendererImage;
use qa_client::render::types::TextureBinding;
use qa_client::render::types::TextureFilter;
use qa_client::render::types::TextureSampling;
use qa_client::text::atlas::classic_charset;
use qa_client::text::atlas::TextFontSelection;
use qa_client::text::draw2d::Rect;
use qa_client::text::draw2d::TextureRect;
use qa_client::ClientError;
use qa_content::catalog::InstalledCatalog;
use qa_content::catalog::ProductAvailability;
use qa_content::contract::create_mount_plan_id;
use qa_content::contract::GameFamily;
use qa_content::contract::ResolvedMountPlan;
use qa_content::images::decode_bmp;
use qa_content::images::decode_gif;
use qa_content::images::decode_jpeg;
use qa_content::images::decode_palette;
use qa_content::images::decode_pcx;
use qa_content::images::decode_png;
use qa_content::images::decode_tga;
use qa_content::images::decode_wad;
use qa_content::images::expand_indexed_image;
use qa_content::images::indexed_render_image;
use qa_content::images::IndexedImage;
use qa_content::images::PaletteLayer;
use qa_content::images::PaletteTransparency;
use qa_content::mounts::open_mount_plan;
use qa_content::mounts::MountedContent;
use qa_content::mounts::OpenMountOptions;
use qa_core::math::vec2;
use qa_core::math::vec4;
use qa_core::math::Vec4;

/// Atlas width in pixels (16 cells of 8).
pub(crate) const CONCHARS_WIDTH: u32 = 128;
/// Atlas height in pixels (16 cells of 8).
pub(crate) const CONCHARS_HEIGHT: u32 = 128;
/// Glyph cell edge in pixels.
const CELL: u32 = 8;
/// Headless picture handle for the menu charset (matches the classic `7`
/// used across menu tests; the capture routes this handle to glyph quads).
pub(crate) const FONT_PICTURE_HANDLE: u32 = 7;

/// Blank cell for control codes.
const BLANK: [&str; 8] = [
    "........", "........", "........", "........", "........", "........", "........", "........",
];

/// 8 by 8 glyph rows for codes 32 through 127 (`#` is ink, `.` is clear).
const GLYPHS: [[&str; 8]; 96] = [
    [
        "........", "........", "........", "........", "........", "........", "........", "........",
    ],
    [
        "...##...", "...##...", "...##...", "...##...", "...##...", "........", "...##...", "........",
    ],
    [
        ".##.##..", ".##.##..", ".##.##..", "........", "........", "........", "........", "........",
    ],
    [
        ".#..#...", ".#..#...", "######..", ".#..#...", ".#..#...", "######..", ".#..#...", "........",
    ],
    [
        "..###...", ".##.##..", ".##.....", "..###...", "....##..", ".##.##..", "..###...", "........",
    ],
    [
        "##...##.", "##..##..", "...##...", "..##....", ".##.....", "##..##..", "##...##.", "........",
    ],
    [
        ".###....", "##.##...", "##.##...", ".###....", "##.##.#.", "##..##..", ".####.#.", "........",
    ],
    [
        "...##...", "...##...", "...##...", "........", "........", "........", "........", "........",
    ],
    [
        "....##..", "...##...", "..##....", "..##....", "..##....", "..##....", "...##...", "....##..",
    ],
    [
        "..##....", "...##...", "....##..", "....##..", "....##..", "....##..", "...##...", "..##....",
    ],
    [
        "........", ".##.##..", "..###...", "######..", "..###...", ".##.##..", "........", "........",
    ],
    [
        "........", "...##...", "...##...", ".######.", ".######.", "...##...", "...##...", "........",
    ],
    [
        "........", "........", "........", "........", "...##...", "...##...", "..##....", ".##.....",
    ],
    [
        "........", "........", "........", ".######.", ".######.", "........", "........", "........",
    ],
    [
        "........", "........", "........", "........", "........", "...##...", "...##...", "........",
    ],
    [
        ".....##.", ".....##.", "....##..", "....##..", "...##...", "...##...", "..##....", "..##....",
    ],
    [
        "..####..", ".##..##.", "##..###.", "##.####.", "####.##.", "###..##.", ".##..##.", "..####..",
    ],
    [
        "...##...", "..###...", "...##...", "...##...", "...##...", "...##...", ".######.", "........",
    ],
    [
        "..####..", ".##..##.", ".....##.", "....##..", "...##...", "..##....", ".######.", "........",
    ],
    [
        ".######.", "....##..", "...##...", "....##..", ".....##.", ".##..##.", "..####..", "........",
    ],
    [
        "....##..", "...###..", "..####..", ".##.##..", ".######.", "....##..", "....##..", "........",
    ],
    [
        ".######.", ".##.....", ".#####..", ".....##.", ".....##.", ".##..##.", "..####..", "........",
    ],
    [
        "...###..", "..##....", ".##.....", ".#####..", ".##..##.", ".##..##.", "..####..", "........",
    ],
    [
        ".######.", ".....##.", "....##..", "...##...", "...##...", "...##...", "...##...", "........",
    ],
    [
        "..####..", ".##..##.", ".##..##.", "..####..", ".##..##.", ".##..##.", "..####..", "........",
    ],
    [
        "..####..", ".##..##.", ".##..##.", "..#####.", ".....##.", "....##..", "..###...", "........",
    ],
    [
        "........", "...##...", "...##...", "........", "........", "...##...", "...##...", "........",
    ],
    [
        "........", "...##...", "...##...", "........", "...##...", "...##...", "..##....", ".##.....",
    ],
    [
        "....##..", "...##...", "..##....", ".##.....", "..##....", "...##...", "....##..", "........",
    ],
    [
        "........", "........", ".######.", "........", ".######.", "........", "........", "........",
    ],
    [
        "..##....", "...##...", "....##..", ".....##.", "....##..", "...##...", "..##....", "........",
    ],
    [
        "..####..", ".##..##.", ".....##.", "....##..", "...##...", "........", "...##...", "........",
    ],
    [
        "..####..", ".##..##.", "##.##.##", "##.##.##", "##.##.##", ".##..##.", "..####..", "........",
    ],
    [
        "...##...", "..####..", ".##..##.", ".##..##.", ".######.", ".##..##.", ".##..##.", "........",
    ],
    [
        ".#####..", ".##..##.", ".##..##.", ".#####..", ".##..##.", ".##..##.", ".#####..", "........",
    ],
    [
        "..####..", ".##..##.", ".##.....", ".##.....", ".##.....", ".##..##.", "..####..", "........",
    ],
    [
        ".#####..", ".##..##.", ".##..##.", ".##..##.", ".##..##.", ".##..##.", ".#####..", "........",
    ],
    [
        ".######.", ".##.....", ".##.....", ".#####..", ".##.....", ".##.....", ".######.", "........",
    ],
    [
        ".######.", ".##.....", ".##.....", ".#####..", ".##.....", ".##.....", ".##.....", "........",
    ],
    [
        "..####..", ".##..##.", ".##.....", ".##.###.", ".##..##.", ".##..##.", "..####..", "........",
    ],
    [
        ".##..##.", ".##..##.", ".##..##.", ".######.", ".##..##.", ".##..##.", ".##..##.", "........",
    ],
    [
        ".######.", "...##...", "...##...", "...##...", "...##...", "...##...", ".######.", "........",
    ],
    [
        "...####.", "....##..", "....##..", "....##..", "....##..", ".##.##..", "..###...", "........",
    ],
    [
        ".##..##.", ".##..##.", ".##.##..", ".####...", ".##.##..", ".##..##.", ".##..##.", "........",
    ],
    [
        ".##.....", ".##.....", ".##.....", ".##.....", ".##.....", ".##.....", ".######.", "........",
    ],
    [
        ".##..##.", ".######.", ".######.", ".######.", ".##..##.", ".##..##.", ".##..##.", "........",
    ],
    [
        ".##..##.", ".###.##.", ".###.##.", ".######.", ".##.###.", ".##.###.", ".##..##.", "........",
    ],
    [
        "..####..", ".##..##.", ".##..##.", ".##..##.", ".##..##.", ".##..##.", "..####..", "........",
    ],
    [
        ".#####..", ".##..##.", ".##..##.", ".#####..", ".##.....", ".##.....", ".##.....", "........",
    ],
    [
        "..####..", ".##..##.", ".##..##.", ".##..##.", ".##.##..", "..####..", "...###..", ".....##.",
    ],
    [
        ".#####..", ".##..##.", ".##..##.", ".#####..", ".##.##..", ".##..##.", ".##..##.", "........",
    ],
    [
        "..####..", ".##..##.", ".##.....", "..####..", ".....##.", ".##..##.", "..####..", "........",
    ],
    [
        ".######.", "...##...", "...##...", "...##...", "...##...", "...##...", "...##...", "........",
    ],
    [
        ".##..##.", ".##..##.", ".##..##.", ".##..##.", ".##..##.", ".##..##.", "..####..", "........",
    ],
    [
        ".##..##.", ".##..##.", ".##..##.", ".##..##.", ".##..##.", "..####..", "...##...", "........",
    ],
    [
        ".##..##.", ".##..##.", ".##..##.", ".##..##.", ".######.", ".######.", ".##..##.", "........",
    ],
    [
        ".##..##.", ".##..##.", "..####..", "...##...", "..####..", ".##..##.", ".##..##.", "........",
    ],
    [
        ".##..##.", ".##..##.", "..####..", "...##...", "...##...", "...##...", "...##...", "........",
    ],
    [
        ".######.", ".....##.", "....##..", "...##...", "..##....", ".##.....", ".######.", "........",
    ],
    [
        "...####.", "...##...", "...##...", "...##...", "...##...", "...##...", "...####.", "........",
    ],
    [
        ".##.....", ".##.....", "..##....", "..##....", "...##...", "...##...", "....##..", "....##..",
    ],
    [
        ".####...", "...##...", "...##...", "...##...", "...##...", "...##...", ".####...", "........",
    ],
    [
        "...##...", "..####..", ".##..##.", "........", "........", "........", "........", "........",
    ],
    [
        "........", "........", "........", "........", "........", "........", ".######.", "........",
    ],
    [
        "...##...", "...##...", "....##..", "........", "........", "........", "........", "........",
    ],
    [
        "........", "........", "..####..", ".....##.", "..#####.", ".##..##.", "..#####.", "........",
    ],
    [
        ".##.....", ".##.....", ".#####..", ".##..##.", ".##..##.", ".##..##.", ".#####..", "........",
    ],
    [
        "........", "........", "..####..", ".##..##.", ".##.....", ".##..##.", "..####..", "........",
    ],
    [
        ".....##.", ".....##.", "..#####.", ".##..##.", ".##..##.", ".##..##.", "..#####.", "........",
    ],
    [
        "........", "........", "..####..", ".##..##.", ".######.", ".##.....", "..####..", "........",
    ],
    [
        "...###..", "...##...", "...##...", ".######.", "...##...", "...##...", "...##...", "........",
    ],
    [
        "........", "........", "..#####.", ".##..##.", ".##..##.", "..#####.", ".....##.", "..####..",
    ],
    [
        ".##.....", ".##.....", ".#####..", ".##..##.", ".##..##.", ".##..##.", ".##..##.", "........",
    ],
    [
        "...##...", "........", "...##...", "...##...", "...##...", "...##...", "...##...", "........",
    ],
    [
        "....##..", "........", "...###..", "....##..", "....##..", "....##..", ".##.##..", "..###...",
    ],
    [
        ".##.....", ".##.....", ".##..##.", ".##.##..", ".####...", ".##.##..", ".##..##.", "........",
    ],
    [
        "..###...", "...##...", "...##...", "...##...", "...##...", "...##...", ".######.", "........",
    ],
    [
        "........", "........", "###.###.", "#######.", "#######.", "##.#.##.", "##...##.", "........",
    ],
    [
        "........", "........", ".#####..", ".##..##.", ".##..##.", ".##..##.", ".##..##.", "........",
    ],
    [
        "........", "........", "..####..", ".##..##.", ".##..##.", ".##..##.", "..####..", "........",
    ],
    [
        "........", "........", ".#####..", ".##..##.", ".##..##.", ".#####..", ".##.....", ".##.....",
    ],
    [
        "........", "........", "..#####.", ".##..##.", ".##..##.", "..#####.", ".....##.", ".....##.",
    ],
    [
        "........", "........", ".##.###.", ".###....", ".##.....", ".##.....", ".##.....", "........",
    ],
    [
        "........", "........", "..#####.", ".##.....", "..####..", ".....##.", ".#####..", "........",
    ],
    [
        "...##...", "...##...", ".######.", "...##...", "...##...", "...##...", "...##...", "........",
    ],
    [
        "........", "........", ".##..##.", ".##..##.", ".##..##.", ".##..##.", "..#####.", "........",
    ],
    [
        "........", "........", ".##..##.", ".##..##.", ".##..##.", "..####..", "...##...", "........",
    ],
    [
        "........", "........", ".##..##.", ".##..##.", ".######.", ".######.", ".##..##.", "........",
    ],
    [
        "........", "........", ".##..##.", "..####..", "...##...", "..####..", ".##..##.", "........",
    ],
    [
        "........", "........", ".##..##.", ".##..##.", ".##..##.", "..#####.", ".....##.", "..####..",
    ],
    [
        "........", "........", ".######.", "....##..", "...##...", "..##....", ".######.", "........",
    ],
    [
        "...###..", "...##...", "...##...", ".###....", "...##...", "...##...", "...##...", "...###..",
    ],
    [
        "...##...", "...##...", "...##...", "...##...", "...##...", "...##...", "...##...", "...##...",
    ],
    [
        "..###...", "...##...", "...##...", "....###.", "...##...", "...##...", "...##...", "..###...",
    ],
    [
        "..##.#..", ".##..##.", "........", "........", "........", "........", "........", "........",
    ],
    [
        ".######.", ".##..##.", ".##..##.", ".##..##.", ".##..##.", ".##..##.", ".######.", "........",
    ],
];

/// Rows for one charset cell: blank controls, the glyph table for 32..127,
/// and a mirrored upper half like the classic brown/red pages.
fn cell_rows(code: u32) -> &'static [&'static str; 8] {
    if code < 32 {
        &BLANK
    } else if code < 128 {
        &GLYPHS[(code - 32) as usize]
    } else {
        let base = code - 128;
        if base < 32 {
            &BLANK
        } else {
            &GLYPHS[(base - 32) as usize]
        }
    }
}

/// Build the 128 by 128 charset atlas: white ink on transparent cells in
/// top-down row order (row 0 is the first uploaded row, sampled at `t = 0`).
#[must_use]
pub(crate) fn conchars_rgba() -> Vec<u8> {
    let mut pixels = vec![0u8; (CONCHARS_WIDTH * CONCHARS_HEIGHT * 4) as usize];
    for code in 0..256u32 {
        let glyph = cell_rows(code);
        let cell_x = (code & 15) * CELL;
        let cell_y = (code >> 4) * CELL;
        for (row, pattern) in glyph.iter().enumerate() {
            for (col, cell) in pattern.bytes().enumerate() {
                if cell != b'#' {
                    continue;
                }
                let x = cell_x + col as u32;
                let y = cell_y + row as u32;
                let at = ((y * CONCHARS_WIDTH + x) * 4) as usize;
                pixels[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    pixels
}

/// Menu font selection over the synthetic atlas. Ink is white, so glyphs
/// must tint (`baked` stays false) for runs to keep their text colors.
pub(crate) fn menu_font_selection() -> Result<TextFontSelection, ClientError> {
    menu_font_selection_for(CONCHARS_WIDTH, CONCHARS_HEIGHT, false)
}

/// Menu font selection over one atlas size. Real Q1/Q2 charsets carry
/// baked colors (`baked` true, glyphs draw white); synthetic and Q3
/// charsets are white ink (`baked` false, glyphs tint per run).
pub fn menu_font_selection_for(width: u32, height: u32, baked: bool) -> Result<TextFontSelection, ClientError> {
    Ok(TextFontSelection::Classic {
        classic: classic_charset(FONT_PICTURE_HANDLE, width, height, "conchars", baked)?,
        unicode: None,
    })
}

/// Real console charset decoded from installed game content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuCharset {
    /// Atlas width in pixels (multiple of 16).
    pub width: u32,
    /// Atlas height in pixels (multiple of 16).
    pub height: u32,
    /// Top-down RGBA texels (`width * height * 4`).
    pub pixels: Vec<u8>,
    /// Whether glyph colors are baked (Q1/Q2) or tinted (Q3).
    pub baked: bool,
    /// Winning product id.
    pub product: String,
    /// Winning resource path.
    pub source: String,
}

/// Whether an atlas size hosts 16 by 16 glyph cells.
fn valid_charset_size(width: u32, height: u32, pixels: &[u8]) -> bool {
    width.is_multiple_of(16)
        && height.is_multiple_of(16)
        && width > 0
        && height > 0
        && pixels.len() == width as usize * height as usize * 4
}

/// Installed candidates in donor order: the preferred product first, then
/// catalog order (donor `startup.ts` picks `options.product` or the first
/// installed product).
pub(crate) fn installed_candidates(catalog: &InstalledCatalog, preferred: Option<&str>) -> Vec<String> {
    let mut order = Vec::new();
    if let Some(want) = preferred {
        if catalog.products.iter().any(|product| {
            product.availability == ProductAvailability::Installed
                && (product.id.as_str() == want || product.expectation.id == want)
        }) {
            order.push(want.to_string());
        }
    }
    for product in &catalog.products {
        if product.availability != ProductAvailability::Installed {
            continue;
        }
        let id = product.id.as_str();
        if order.iter().any(|seen| seen == id || *seen == product.expectation.id) {
            continue;
        }
        order.push(id.to_string());
    }
    order
}

/// Open one product's mounts for a charset read.
pub(crate) fn open_charset_mounts(catalog: &InstalledCatalog, content: &str) -> Option<MountedContent> {
    let mounts = catalog.mounts_for(content).ok()?;
    let plan = ResolvedMountPlan {
        id: create_mount_plan_id("windowed-menu-font", "charset").ok()?,
        mounts: mounts.clone(),
        default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
        prefix_orders: Vec::new(),
    };
    open_mount_plan(&plan, OpenMountOptions::default()).ok()
}

/// Q1 `conchars` from `gfx.wad` plus `gfx/palette.lmp` (donor
/// `loadMenuFont` Q1 branch, index 0 transparent).
fn decode_q1_charset(mounts: &MountedContent, product: &str) -> Option<MenuCharset> {
    let wad = mounts.open("gfx.wad", |_| true).ok()??;
    let palette = mounts.open("gfx/palette.lmp", |_| true).ok()??;
    let archive = decode_wad(&wad.bytes, "gfx.wad").ok()?;
    let lump = archive.lumps.iter().find(|value| value.name == "conchars")?;
    if lump.compression != 0 || lump.bytes.len() != 128 * 128 {
        return None;
    }
    let palette = decode_palette(&palette.bytes, "gfx/palette.lmp").ok()?;
    let render = indexed_render_image(
        vec![IndexedImage {
            width: 128,
            height: 128,
            indices: lump.bytes.clone(),
        }],
        palette,
        PaletteTransparency::Index(0),
        None,
        None,
    )
    .ok()?;
    let level = expand_indexed_image(&render, 0, PaletteLayer::Combined).ok()?;
    if !valid_charset_size(level.width, level.height, &level.pixels) {
        return None;
    }
    Some(MenuCharset {
        width: level.width,
        height: level.height,
        pixels: level.pixels,
        baked: true,
        product: product.to_string(),
        source: "gfx.wad:conchars".to_string(),
    })
}

/// Expand Q2 PCX indices with the loader's index-255 rule: transparent
/// texels borrow the first non-255 neighbor so glyph edges keep their
/// halo color (donor `SceneTextureLoader` `.pcx` branch).
fn expand_q2_pcx(width: u32, height: u32, indices: &[u8], colors: &[u8]) -> Option<Vec<u8>> {
    if colors.len() != 768 || indices.len() != width as usize * height as usize {
        return None;
    }
    let mut pixels = vec![0u8; indices.len() * 4];
    let stride = width as usize;
    for (offset, index) in indices.iter().enumerate() {
        let mut color = *index;
        if *index == 255 {
            let above = offset.checked_sub(stride).and_then(|at| indices.get(at).copied());
            let below = indices.get(offset + stride).copied();
            let left = offset.checked_sub(1).and_then(|at| indices.get(at).copied());
            let right = indices.get(offset + 1).copied();
            color = [above, below, left, right]
                .into_iter()
                .flatten()
                .find(|value| *value != 255)
                .unwrap_or(0);
        }
        let base = usize::from(color) * 3;
        let alpha = u8::from(*index != 255) * 255;
        pixels[offset * 4..offset * 4 + 4].copy_from_slice(&[
            colors.get(base).copied().unwrap_or(0),
            colors.get(base + 1).copied().unwrap_or(0),
            colors.get(base + 2).copied().unwrap_or(0),
            alpha,
        ]);
    }
    Some(pixels)
}

/// Q2 `pics/conchars.pcx` with its embedded palette, else the
/// `pics/colormap.pcx` palette (donor `loadMenuFont` Q2 branch).
fn decode_q2_charset(mounts: &MountedContent, product: &str) -> Option<MenuCharset> {
    let asset = mounts.open("pics/conchars.pcx", |_| true).ok()??;
    let decoded = decode_pcx(&asset.bytes, "pics/conchars.pcx").ok()?;
    let colors = match decoded.palette {
        Some(colors) => colors,
        None => {
            let map = mounts.open("pics/colormap.pcx", |_| true).ok()??;
            decode_pcx(&map.bytes, "pics/colormap.pcx").ok()?.palette?
        }
    };
    let width = u32::try_from(decoded.width).ok()?;
    let height = u32::try_from(decoded.height).ok()?;
    let pixels = expand_q2_pcx(width, height, &decoded.indices, &colors)?;
    if !valid_charset_size(width, height, &pixels) {
        return None;
    }
    Some(MenuCharset {
        width,
        height,
        pixels,
        baked: true,
        product: product.to_string(),
        source: "pics/conchars.pcx".to_string(),
    })
}

/// Decode one Q3 charset candidate by suffix.
fn decode_q3_candidate(bytes: &[u8], path: &str) -> Option<(u32, u32, Vec<u8>)> {
    let suffix = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match suffix.as_str() {
        "tga" => {
            let image = decode_tga(bytes, path).ok()?;
            Some((u32::from(image.width), u32::from(image.height), image.pixels))
        }
        "png" => {
            let image = decode_png(bytes, path).ok()?;
            Some((image.width, image.height, image.pixels))
        }
        "jpg" | "jpeg" => {
            let image = decode_jpeg(bytes, path).ok()?;
            Some((image.width, image.height, image.pixels))
        }
        "bmp" => {
            let image = decode_bmp(bytes, path).ok()?;
            Some((image.width, image.height, image.pixels))
        }
        "gif" => {
            let image = decode_gif(bytes, path).ok()?;
            let frame = image.frames.first()?;
            Some((frame.image.width, frame.image.height, frame.image.pixels.clone()))
        }
        "pcx" => {
            let decoded = decode_pcx(bytes, path).ok()?;
            let colors = decoded.palette?;
            let width = u32::try_from(decoded.width).ok()?;
            let height = u32::try_from(decoded.height).ok()?;
            let mut pixels = vec![0u8; decoded.indices.len() * 4];
            for (offset, index) in decoded.indices.iter().enumerate() {
                let base = usize::from(*index) * 3;
                pixels[offset * 4..offset * 4 + 4].copy_from_slice(&[
                    colors.get(base).copied().unwrap_or(0),
                    colors.get(base + 1).copied().unwrap_or(0),
                    colors.get(base + 2).copied().unwrap_or(0),
                    255,
                ]);
            }
            Some((width, height, pixels))
        }
        _ => {
            // Bare `gfx/2d/bigchars` without a suffix: the Steel corpus
            // stores TGA bytes, so try TGA, then PNG, then JPEG.
            decode_q3_candidate(bytes, "gfx/2d/bigchars.tga")
                .or_else(|| decode_q3_candidate(bytes, "gfx/2d/bigchars.png"))
                .or_else(|| decode_q3_candidate(bytes, "gfx/2d/bigchars.jpg"))
        }
    }
}

/// Q3 `gfx/2d/bigchars` through the loader's candidate order (donor
/// `loadMenuFont` Q3 branch, tinted white ink).
fn decode_q3_charset(mounts: &MountedContent, product: &str) -> Option<MenuCharset> {
    for candidate in [
        "gfx/2d/bigchars.tga",
        "gfx/2d/bigchars.jpg",
        "gfx/2d/bigchars.png",
        "gfx/2d/bigchars.jpeg",
        "gfx/2d/bigchars.pcx",
        "gfx/2d/bigchars.bmp",
        "gfx/2d/bigchars.gif",
        "gfx/2d/bigchars",
    ] {
        let asset = mounts.open(candidate, |_| true).ok()??;
        let Some((width, height, pixels)) = decode_q3_candidate(&asset.bytes, candidate) else {
            continue;
        };
        if !valid_charset_size(width, height, &pixels) {
            continue;
        }
        return Some(MenuCharset {
            width,
            height,
            pixels,
            baked: false,
            product: product.to_string(),
            source: candidate.to_string(),
        });
    }
    None
}

/// Resolve the real console charset through installed content mounts:
/// the preferred product first, then catalog order, decoding per family
/// (donor `loadMenuFont`). Returns `None` when no content (or no
/// charset) is installed so the caller keeps the synthetic fallback.
#[must_use]
pub fn resolve_menu_charset(catalog: &InstalledCatalog, preferred: Option<&str>) -> Option<MenuCharset> {
    for content in installed_candidates(catalog, preferred) {
        let Ok(product) = catalog.product(&content) else {
            continue;
        };
        let Some(mounts) = open_charset_mounts(catalog, product.id.as_str()) else {
            continue;
        };
        let decoded = match product.expectation.family {
            GameFamily::Q1 => decode_q1_charset(&mounts, product.id.as_str()),
            GameFamily::Q2 => decode_q2_charset(&mounts, product.id.as_str()),
            GameFamily::Q3 => decode_q3_charset(&mounts, product.id.as_str()),
        };
        if decoded.is_some() {
            return decoded;
        }
    }
    None
}

/// One captured glyph quad: destination pixels, atlas UVs, and run color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GlyphQuad {
    /// Picture handle of the atlas this glyph was laid out from
    /// ([`FONT_PICTURE_HANDLE`] for the classic charset, higher handles
    /// for proportional/TrueType atlases from `load_menu_typography`).
    pub font: u32,
    /// Destination rectangle in drawable pixels.
    pub rect: Rect,
    /// Atlas source coordinates.
    pub uv: TextureRect,
    /// Glyph color.
    pub color: Vec4,
}

/// One uploaded menu font atlas: the headless picture handle carried by
/// laid-out glyphs plus the backend image it binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FontAtlasImage {
    /// Picture handle (`ImagePicture.image`).
    pub handle: u32,
    /// Uploaded backend image.
    pub image: RendererImage,
}

/// Inset glyph UVs by half a texel so edge-exact samples stay inside the
/// glyph cell. Quad edges land exactly on pixel centers, and the GL
/// backend's fixed-point UV interpolation can round an exact cell boundary
/// down into the previous texel: with the Q2 charset that sampled the
/// previous cell's last ink column as stray bars at glyph edges (CPU
/// rendering was unaffected). Interior texel centers are unaffected.
fn inset_glyph_uv(uv: TextureRect, atlas_width: u32, atlas_height: u32) -> TextureRect {
    let half_s = if atlas_width == 0 {
        0.0
    } else {
        0.5 / atlas_width as f32
    };
    let half_t = if atlas_height == 0 {
        0.0
    } else {
        0.5 / atlas_height as f32
    };
    let (s, s2) = if uv.s2 - uv.s > half_s * 2.0 {
        (uv.s + half_s, uv.s2 - half_s)
    } else {
        let mid = (uv.s + uv.s2) / 2.0;
        (mid, mid)
    };
    let (t, t2) = if uv.t2 - uv.t > half_t * 2.0 {
        (uv.t + half_t, uv.t2 - half_t)
    } else {
        let mid = (uv.t + uv.t2) / 2.0;
        (mid, mid)
    };
    TextureRect { s, t, s2, t2 }
}

/// Textured overlay batches for captured glyph quads in NDC space (the same
/// depth-always blended overlay as the flat menu batch, bound to the font
/// atlases instead of the 1x1 white image). Consecutive quads over one
/// atlas group into emit-order runs so painter order survives batching;
/// quads whose handle has no upload are skipped.
pub(crate) fn glyph_batches(glyphs: &[GlyphQuad], width: f32, height: f32, fonts: &[FontAtlasImage]) -> Vec<DrawBatch> {
    let mut batches = Vec::new();
    let mut run: Vec<&GlyphQuad> = Vec::new();
    let flush = |run: &mut Vec<&GlyphQuad>, batches: &mut Vec<DrawBatch>| {
        if run.is_empty() {
            return;
        }
        let handle = run[0].font;
        let Some(font) = fonts.iter().find(|font| font.handle == handle) else {
            run.clear();
            return;
        };
        let mut vertices = Vec::with_capacity(run.len() * 4);
        let mut indices = Vec::with_capacity(run.len() * 6);
        for quad in run.drain(..) {
            let base = vertices.len() as u32;
            let uv = inset_glyph_uv(quad.uv, font.image.width, font.image.height);
            let left = 2.0 * quad.rect.x / width - 1.0;
            let right = 2.0 * (quad.rect.x + quad.rect.width) / width - 1.0;
            let top = 1.0 - 2.0 * quad.rect.y / height;
            let bottom = 1.0 - 2.0 * (quad.rect.y + quad.rect.height) / height;
            for (x, y, s, t) in [
                (left, top, uv.s, uv.t),
                (right, top, uv.s2, uv.t),
                (right, bottom, uv.s2, uv.t2),
                (left, bottom, uv.s, uv.t2),
            ] {
                vertices.push(RenderVertex {
                    position: vec4(x, y, 0.0, 1.0),
                    tex_coord: vec2(s, t),
                    color: quad.color,
                });
            }
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        batches.push(DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices,
            texture: TextureBinding::BindImage(font.image.clone()),
            state: RenderState {
                blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
                depth_test: DepthTest::Always,
                depth_write: false,
                alpha_test: AlphaTest::None,
                cull: CullFace::None,
                depth_range: [0.0, 1.0],
                polygon_offset: None,
            },
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vertices),
        });
    };
    for quad in glyphs {
        if run.last().is_some_and(|open: &&GlyphQuad| open.font != quad.font) {
            flush(&mut run, &mut batches);
        }
        run.push(quad);
    }
    flush(&mut run, &mut batches);
    batches
}

/// Upload operation for one atlas size (real charsets may be 256 by 256;
/// the synthetic fallback is 128 by 128).
pub(crate) fn font_upload_sized(
    font: &RendererImage,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
) -> ImageResourceOperation {
    font_upload_with_filter(font, width, height, pixels, TextureFilter::Nearest)
}

/// Upload operation for one proportional/TrueType atlas (donor mounted
/// fonts sample clamp with linear filtering, unlike the nearest-sampled
/// console charset).
pub(crate) fn font_upload_linear(
    font: &RendererImage,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
) -> ImageResourceOperation {
    font_upload_with_filter(font, width, height, pixels, TextureFilter::Linear)
}

/// Upload operation for one atlas with an explicit filter.
fn font_upload_with_filter(
    font: &RendererImage,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    filter: TextureFilter,
) -> ImageResourceOperation {
    ImageResourceOperation::CreateImage {
        image: font.clone(),
        content: RenderImage::Rgba8 {
            levels: vec![ImageLevel { width, height, pixels }],
            border_color: vec4(0.0, 0.0, 0.0, 0.0),
        },
        sampling: TextureSampling { repeat: false, filter },
    }
}

/// One decoded proportional-font texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedMenuTexture {
    /// Atlas width in pixels.
    pub width: u32,
    /// Atlas height in pixels.
    pub height: u32,
    /// Top-down RGBA texels (`width * height * 4`).
    pub pixels: Vec<u8>,
}

/// Decode one menu font texture by suffix: TGA/PNG/JPEG/BMP/GIF through
/// the Q3 candidate decoder, PCX through the embedded (or provided)
/// palette with the Q2 index-255 halo rule. Returns `None` when the
/// bytes do not decode.
#[must_use]
pub(crate) fn decode_menu_texture(bytes: &[u8], path: &str, palette: Option<&[u8]>) -> Option<DecodedMenuTexture> {
    if path.rsplit('.').next().unwrap_or("").eq_ignore_ascii_case("pcx") {
        let decoded = decode_pcx(bytes, path).ok()?;
        let colors = decoded.palette.or_else(|| palette.map(<[u8]>::to_vec))?;
        let width = u32::try_from(decoded.width).ok()?;
        let height = u32::try_from(decoded.height).ok()?;
        let pixels = expand_q2_pcx(width, height, &decoded.indices, &colors)?;
        return Some(DecodedMenuTexture { width, height, pixels });
    }
    let (width, height, pixels) = decode_q3_candidate(bytes, path)?;
    Some(DecodedMenuTexture { width, height, pixels })
}

#[cfg(test)]
mod tests {
    use qa_client::text::atlas::glyph_uv;
    use qa_client::text::atlas::resolve_text_glyph;
    use qa_core::identity::IdentityOwner;

    use super::*;

    fn cell_ink(pixels: &[u8], code: u32) -> usize {
        let cell_x = ((code & 15) * CELL) as usize;
        let cell_y = ((code >> 4) * CELL) as usize;
        let mut ink = 0;
        for row in 0..CELL as usize {
            for col in 0..CELL as usize {
                let at = ((cell_y + row) * CONCHARS_WIDTH as usize + cell_x + col) * 4;
                if pixels[at + 3] == 255 {
                    assert_eq!(&pixels[at..at + 3], &[255, 255, 255], "ink is white, code {code}");
                    ink += 1;
                } else {
                    assert_eq!(
                        &pixels[at..at + 4],
                        &[0, 0, 0, 0],
                        "cells are ink or clear, code {code}"
                    );
                }
            }
        }
        ink
    }

    #[test]
    fn glyph_rows_are_well_formed() {
        assert_eq!(GLYPHS.len(), 96);
        for (index, glyph) in GLYPHS.iter().enumerate() {
            for row in glyph {
                assert_eq!(row.len(), CELL as usize, "glyph {}", index + 32);
                assert!(
                    row.bytes().all(|cell| cell == b'.' || cell == b'#'),
                    "glyph {} uses ink or clear",
                    index + 32
                );
            }
        }
    }

    #[test]
    fn atlas_marks_printable_glyphs_without_bars_or_blanks() {
        let pixels = conchars_rgba();
        assert_eq!(pixels.len(), (CONCHARS_WIDTH * CONCHARS_HEIGHT * 4) as usize);
        assert_eq!(cell_ink(&pixels, 32), 0, "space stays clear");
        for code in [0, 10, 31] {
            assert_eq!(cell_ink(&pixels, code), 0, "controls stay clear, code {code}");
        }
        for code in 33..127u32 {
            let ink = cell_ink(&pixels, code);
            assert!(
                (4..=56).contains(&ink),
                "glyph {code} is glyph-shaped, got {ink} ink pixels"
            );
        }
        for code in 128..256u32 {
            assert_eq!(
                cell_ink(&pixels, code),
                cell_ink(&pixels, code - 128),
                "upper half mirrors, code {code}"
            );
        }
    }

    #[test]
    fn atlas_top_row_matches_glyph_uv_origin() {
        let selection = menu_font_selection().unwrap();
        let glyph = resolve_text_glyph(&selection, u32::from(b'A'), false).unwrap();
        assert!(glyph.visible);
        assert_eq!((glyph.glyph.x, glyph.glyph.y), (8, 32));
        let uv = glyph_uv(&glyph);
        assert_eq!(
            (uv.s, uv.t, uv.s2, uv.t2),
            (8.0 / 128.0, 32.0 / 128.0, 16.0 / 128.0, 40.0 / 128.0)
        );
        let pixels = conchars_rgba();
        let row0 = ((32 * CONCHARS_WIDTH + 8) * 4) as usize;
        assert_eq!(pixels[row0 + 2 * 4 + 3], 0, "row 0 col 2 of 'A' is clear");
        assert_eq!(pixels[row0 + 3 * 4 + 3], 255, "row 0 col 3 of 'A' is ink");
    }

    #[test]
    fn font_selection_tints_white_ink() {
        let selection = menu_font_selection().unwrap();
        let glyph = resolve_text_glyph(&selection, u32::from(b'Q'), false).unwrap();
        assert!(!glyph.glyph.color, "white ink must tint through vertex colors");
    }

    fn font_image() -> RendererImage {
        let authority = IdentityOwner::create("windowed-menu-text-test").unwrap();
        RendererImage {
            owner: qa_client::render::types::ResourceOwner::new(7, authority.session().clone(), 0),
            ordinal: 0x7FFF_FF02,
            source: qa_client::render::types::ImageSource::Generated {
                name: "windowed-menu-font-test".to_string(),
            },
            width: CONCHARS_WIDTH,
            height: CONCHARS_HEIGHT,
        }
    }

    fn font_atlases(font: &RendererImage) -> Vec<FontAtlasImage> {
        vec![FontAtlasImage {
            handle: FONT_PICTURE_HANDLE,
            image: font.clone(),
        }]
    }

    #[test]
    fn glyph_batches_carry_uvs_in_flat_batch_space() {
        let font = font_image();
        let fonts = font_atlases(&font);
        assert!(glyph_batches(&[], 640.0, 480.0, &fonts).is_empty());
        let quad = GlyphQuad {
            font: FONT_PICTURE_HANDLE,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 48.0,
            },
            uv: TextureRect {
                s: 0.0625,
                t: 0.25,
                s2: 0.125,
                t2: 0.3125,
            },
            color: vec4(1.0, 0.73, 0.35, 1.0),
        };
        let batches = glyph_batches(&[quad], 640.0, 480.0, &fonts);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].texture, TextureBinding::BindImage(font));
        assert_eq!(batches[0].lighting, BatchLighting::Vertex);
        assert_eq!(batches[0].indices, vec![0, 1, 2, 0, 2, 3]);
        let BatchVertices::Single(vertices) = &batches[0].vertices else {
            panic!("glyph batch must be single-textured");
        };
        assert_eq!(vertices.len(), 4);
        let xs: Vec<f32> = vertices.iter().map(|vertex| vertex.position.x).collect();
        let ys: Vec<f32> = vertices.iter().map(|vertex| vertex.position.y).collect();
        assert_eq!(xs, vec![-1.0, -0.8, -0.8, -1.0]);
        assert_eq!(ys, vec![1.0, 1.0, 0.8, 0.8]);
        let uvs: Vec<(f32, f32)> = vertices
            .iter()
            .map(|vertex| (vertex.tex_coord.x, vertex.tex_coord.y))
            .collect();
        let half = 0.5 / CONCHARS_WIDTH as f32;
        assert_eq!(
            uvs,
            vec![
                (0.0625 + half, 0.25 + half),
                (0.125 - half, 0.25 + half),
                (0.125 - half, 0.3125 - half),
                (0.0625 + half, 0.3125 - half),
            ]
        );
        assert!(vertices.iter().all(|vertex| vertex.color.w > 0.0));
    }

    #[test]
    fn glyph_uvs_inset_half_texel_inside_cell() {
        let font = font_image();
        let fonts = font_atlases(&font);
        let quad = GlyphQuad {
            font: FONT_PICTURE_HANDLE,
            rect: Rect {
                x: 276.5,
                y: 155.0,
                width: 26.0,
                height: 26.0,
            },
            uv: TextureRect {
                s: 40.0 / 128.0,
                t: 48.0 / 128.0,
                s2: 48.0 / 128.0,
                t2: 56.0 / 128.0,
            },
            color: vec4(1.0, 1.0, 1.0, 1.0),
        };
        let batches = glyph_batches(&[quad], 960.0, 600.0, &fonts);
        let BatchVertices::Single(vertices) = &batches[0].vertices else {
            panic!("glyph batch must be single-textured");
        };
        // Edge-exact UVs would sit on texel 40/48; the inset keeps every
        // sample strictly inside the cell so fixed-point interpolation
        // cannot round down into the previous cell's ink column.
        for vertex in vertices {
            let s = vertex.tex_coord.x * CONCHARS_WIDTH as f32;
            let t = vertex.tex_coord.y * CONCHARS_HEIGHT as f32;
            assert!(s > 40.0 && s < 48.0, "s stays in the cell, got {s}");
            assert!(t > 48.0 && t < 56.0, "t stays in the cell, got {t}");
        }
    }

    fn atlas_quad(handle: u32, x: f32) -> GlyphQuad {
        GlyphQuad {
            font: handle,
            rect: Rect {
                x,
                y: 0.0,
                width: 8.0,
                height: 8.0,
            },
            uv: TextureRect {
                s: 0.0,
                t: 0.0,
                s2: 0.0625,
                t2: 0.0625,
            },
            color: vec4(1.0, 1.0, 1.0, 1.0),
        }
    }

    fn sized_font_image(ordinal: u32, width: u32, height: u32) -> RendererImage {
        let authority = IdentityOwner::create("windowed-menu-atlas-test").unwrap();
        RendererImage {
            owner: qa_client::render::types::ResourceOwner::new(7, authority.session().clone(), 0),
            ordinal,
            source: qa_client::render::types::ImageSource::Generated {
                name: format!("windowed-menu-atlas-test:{ordinal}"),
            },
            width,
            height,
        }
    }

    #[test]
    fn glyph_batches_group_consecutive_atlas_runs_in_emit_order() {
        let classic = sized_font_image(0x7FFF_FF02, 128, 128);
        let prop = sized_font_image(0x7FFF_FE00, 256, 256);
        let fonts = vec![
            FontAtlasImage {
                handle: FONT_PICTURE_HANDLE,
                image: classic.clone(),
            },
            FontAtlasImage {
                handle: FONT_PICTURE_HANDLE + 1,
                image: prop.clone(),
            },
        ];
        // Proportional glyph, two classic fallback glyphs, then another
        // proportional glyph: three runs, each binding its own upload.
        let quads = vec![
            atlas_quad(FONT_PICTURE_HANDLE + 1, 0.0),
            atlas_quad(FONT_PICTURE_HANDLE, 8.0),
            atlas_quad(FONT_PICTURE_HANDLE, 16.0),
            atlas_quad(FONT_PICTURE_HANDLE + 1, 24.0),
        ];
        let batches = glyph_batches(&quads, 640.0, 480.0, &fonts);
        assert_eq!(batches.len(), 3);
        let bound: Vec<u32> = batches
            .iter()
            .map(|batch| match &batch.texture {
                TextureBinding::BindImage(image) => image.ordinal,
                _ => panic!("glyph batch must bind an atlas image"),
            })
            .collect();
        assert_eq!(bound, vec![prop.ordinal, classic.ordinal, prop.ordinal]);
        let counts: Vec<usize> = batches
            .iter()
            .map(|batch| match &batch.vertices {
                BatchVertices::Single(vertices) => vertices.len() / 4,
                _ => panic!("glyph batch must be single-textured"),
            })
            .collect();
        assert_eq!(counts, vec![1, 2, 1]);
        // The proportional UV inset uses the 256-wide upload, not the
        // 128-wide classic atlas.
        let BatchVertices::Single(vertices) = &batches[0].vertices else {
            panic!("glyph batch must be single-textured");
        };
        let half = 0.5 / 256.0;
        assert!((vertices[0].tex_coord.x - half).abs() < 1e-6);
    }

    #[test]
    fn glyph_batches_skip_handles_without_an_upload() {
        let font = font_image();
        let fonts = font_atlases(&font);
        let quads = vec![atlas_quad(FONT_PICTURE_HANDLE + 9, 0.0)];
        assert!(glyph_batches(&quads, 640.0, 480.0, &fonts).is_empty());
    }

    #[test]
    fn linear_upload_samples_like_donor_mounted_fonts() {
        let font = sized_font_image(0x7FFF_FE00, 256, 48);
        let upload = font_upload_linear(&font, 256, 48, vec![3u8; 256 * 48 * 4]);
        let ImageResourceOperation::CreateImage {
            image,
            content,
            sampling,
        } = upload
        else {
            panic!("proportional upload must create the atlas image");
        };
        assert_eq!(image, font);
        assert!(!sampling.repeat);
        assert_eq!(sampling.filter, TextureFilter::Linear);
        let RenderImage::Rgba8 { levels, .. } = content else {
            panic!("proportional atlas must be RGBA");
        };
        assert_eq!((levels[0].width, levels[0].height), (256, 48));
    }

    #[test]
    fn decode_menu_texture_covers_tga_and_pcx() {
        use qa_content::images::encode_pcx;
        use qa_content::images::encode_tga;
        use qa_content::images::ImageLevel as ContentLevel;
        let tga = encode_tga(&ContentLevel {
            width: 16,
            height: 16,
            pixels: vec![9u8; 16 * 16 * 4],
        });
        let decoded = decode_menu_texture(&tga, "menu/art/font1_prop.tga", None).expect("tga decodes");
        assert_eq!((decoded.width, decoded.height), (16, 16));
        assert_eq!(decoded.pixels, vec![9u8; 16 * 16 * 4]);
        let palette = vec![7u8; 768];
        let pcx = encode_pcx(
            &IndexedImage {
                width: 16,
                height: 16,
                indices: vec![1u8; 16 * 16],
            },
            &palette,
        )
        .expect("pcx");
        let decoded = decode_menu_texture(&pcx, "pics/conchars.pcx", None).expect("pcx decodes");
        assert_eq!((decoded.width, decoded.height), (16, 16));
        assert_eq!(&decoded.pixels[0..4], &[7, 7, 7, 255]);
        assert!(decode_menu_texture(&[0u8; 8], "menu/art/font1_prop.tga", None).is_none());
    }

    #[test]
    fn font_upload_describes_the_atlas() {
        let font = font_image();
        let upload = font_upload_sized(&font, CONCHARS_WIDTH, CONCHARS_HEIGHT, conchars_rgba());
        let ImageResourceOperation::CreateImage {
            image,
            content,
            sampling,
        } = upload
        else {
            panic!("font upload must create the atlas image");
        };
        assert_eq!(image, font);
        assert!(!sampling.repeat);
        assert_eq!(sampling.filter, TextureFilter::Nearest);
        let RenderImage::Rgba8 { levels, .. } = content else {
            panic!("font atlas must be RGBA");
        };
        assert_eq!(levels.len(), 1);
        assert_eq!((levels[0].width, levels[0].height), (CONCHARS_WIDTH, CONCHARS_HEIGHT));
        assert_eq!(levels[0].pixels.len(), (128 * 128 * 4) as usize);
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("qa-wu23-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    fn fake_product(id: &str, family: GameFamily, loose_root: Option<String>) -> qa_content::catalog::CatalogProduct {
        use qa_content::catalog::CatalogProduct;
        use qa_content::catalog::ProductExpectation;
        use qa_content::contract::ContentId;
        CatalogProduct {
            id: ContentId(id.to_string()),
            expectation: ProductExpectation {
                id: id.to_string(),
                family,
                edition: "classic".to_string(),
                campaign: "test".to_string(),
                title: "Test".to_string(),
                content_directory: "test".to_string(),
                base_product: None,
                required_content_archives: Vec::new(),
                required_programs: Vec::new(),
                map_witness: None,
                unresolved_reason: None,
            },
            availability: ProductAvailability::Installed,
            archives: Vec::new(),
            loose_root,
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn fake_catalog(products: Vec<qa_content::catalog::CatalogProduct>) -> InstalledCatalog {
        InstalledCatalog::new(
            std::env::temp_dir().to_string_lossy().into_owned(),
            products,
            Vec::new(),
            1,
            None,
        )
        .expect("catalog")
    }

    fn wad_with_conchars(indices: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + indices.len() + 32);
        out.extend_from_slice(b"WAD2");
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(12u32 + indices.len() as u32).to_le_bytes());
        out.extend_from_slice(indices);
        out.extend_from_slice(&12u32.to_le_bytes());
        out.extend_from_slice(&(indices.len() as u32).to_le_bytes());
        out.extend_from_slice(&(indices.len() as u32).to_le_bytes());
        out.push(0);
        out.push(0);
        out.push(0);
        out.push(0);
        let mut name = [0u8; 16];
        name[..8].copy_from_slice(b"conchars");
        out.extend_from_slice(&name);
        out
    }

    #[test]
    fn resolver_falls_back_without_products() {
        let catalog = fake_catalog(Vec::new());
        assert!(resolve_menu_charset(&catalog, None).is_none());
        assert!(resolve_menu_charset(&catalog, Some("q2-classic-baseq2")).is_none());
    }

    #[test]
    fn resolver_falls_back_when_charset_missing() {
        let dir = scratch_dir("empty");
        let catalog = fake_catalog(vec![fake_product(
            "q2-test-base",
            GameFamily::Q2,
            Some(dir.to_string_lossy().into_owned()),
        )]);
        assert!(resolve_menu_charset(&catalog, None).is_none());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn q1_charset_decodes_through_mounts() {
        use qa_content::contract::ContentMount;
        use qa_content::contract::LooseMount;
        use qa_content::contract::MountId;
        use qa_content::contract::MountIdentity;
        use qa_content::contract::MountPlanId;
        use qa_content::contract::ResolvedMountPlan;
        let dir = scratch_dir("q1");
        let mut indices = vec![1u8; 128 * 128];
        indices[0] = 0;
        std::fs::create_dir_all(dir.join("gfx")).expect("gfx");
        std::fs::write(dir.join("gfx.wad"), wad_with_conchars(&indices)).expect("wad");
        let mut palette = vec![0u8; 768];
        palette[3..6].copy_from_slice(&[255, 255, 255]);
        std::fs::write(dir.join("gfx/palette.lmp"), &palette).expect("palette");
        let mount = ContentMount::Loose(LooseMount {
            identity: MountIdentity {
                id: MountId("mount:wu23:q1".to_string()),
                content: qa_content::contract::ContentId("q1-test".to_string()),
                generation: 1,
            },
            root_path: dir.to_string_lossy().into_owned(),
        });
        let plan = ResolvedMountPlan {
            id: MountPlanId("mount-plan:wu23:q1".to_string()),
            mounts: vec![mount.clone()],
            default_order: vec![mount.identity().id.clone()],
            prefix_orders: Vec::new(),
        };
        let mounts = open_mount_plan(&plan, OpenMountOptions::default()).expect("open");
        let charset = decode_q1_charset(&mounts, "q1-test").expect("q1 charset");
        assert_eq!((charset.width, charset.height), (128, 128));
        assert!(charset.baked);
        assert_eq!(charset.pixels.len(), 128 * 128 * 4);
        assert_eq!(&charset.pixels[0..4], &[0, 0, 0, 0]);
        assert_eq!(&charset.pixels[4..8], &[255, 255, 255, 255]);
        assert_ne!(charset.pixels, conchars_rgba());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    fn loose_mounts(dir: &std::path::Path, tag: &str, content: &str) -> MountedContent {
        use qa_content::contract::ContentMount;
        use qa_content::contract::LooseMount;
        use qa_content::contract::MountId;
        use qa_content::contract::MountIdentity;
        use qa_content::contract::MountPlanId;
        use qa_content::contract::ResolvedMountPlan;
        let mount = ContentMount::Loose(LooseMount {
            identity: MountIdentity {
                id: MountId(format!("mount:wu23:{tag}")),
                content: qa_content::contract::ContentId(content.to_string()),
                generation: 1,
            },
            root_path: dir.to_string_lossy().into_owned(),
        });
        let plan = ResolvedMountPlan {
            id: MountPlanId(format!("mount-plan:wu23:{tag}")),
            mounts: vec![mount.clone()],
            default_order: vec![mount.identity().id.clone()],
            prefix_orders: Vec::new(),
        };
        open_mount_plan(&plan, OpenMountOptions::default()).expect("open")
    }

    #[test]
    fn q2_charset_uses_embedded_palette_with_transparent_halo() {
        use qa_content::images::encode_pcx;
        let dir = scratch_dir("q2");
        let mut indices = vec![1u8; 16 * 16];
        indices[0] = 255;
        let mut palette = vec![0u8; 768];
        palette[3..6].copy_from_slice(&[200, 100, 50]);
        std::fs::create_dir_all(dir.join("pics")).expect("pics");
        let bytes = encode_pcx(
            &IndexedImage {
                width: 16,
                height: 16,
                indices,
            },
            &palette,
        )
        .expect("pcx");
        std::fs::write(dir.join("pics/conchars.pcx"), &bytes).expect("write");
        let mounts = loose_mounts(&dir, "q2", "q2-test");
        let charset = decode_q2_charset(&mounts, "q2-test").expect("q2 charset");
        assert_eq!((charset.width, charset.height), (16, 16));
        assert!(charset.baked);
        assert_eq!(&charset.pixels[0..4], &[200, 100, 50, 0]);
        assert_eq!(&charset.pixels[4..8], &[200, 100, 50, 255]);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn q3_charset_decodes_tga_candidates_as_tinted() {
        use qa_content::images::encode_tga;
        use qa_content::images::ImageLevel as ContentLevel;
        let dir = scratch_dir("q3");
        let mut pixels = vec![0u8; 16 * 16 * 4];
        pixels[0..4].copy_from_slice(&[255, 255, 255, 255]);
        std::fs::create_dir_all(dir.join("gfx/2d")).expect("gfx");
        let bytes = encode_tga(&ContentLevel {
            width: 16,
            height: 16,
            pixels: pixels.clone(),
        });
        std::fs::write(dir.join("gfx/2d/bigchars.tga"), &bytes).expect("write");
        let mounts = loose_mounts(&dir, "q3", "q3-test");
        let charset = decode_q3_charset(&mounts, "q3-test").expect("q3 charset");
        assert_eq!((charset.width, charset.height), (16, 16));
        assert!(!charset.baked);
        assert_eq!(charset.pixels, pixels);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn resolver_prefers_preferred_then_catalog_order() {
        use qa_content::images::encode_pcx;
        let q1_dir = scratch_dir("pref-q1");
        let mut indices = vec![1u8; 128 * 128];
        indices[0] = 0;
        std::fs::create_dir_all(q1_dir.join("gfx")).expect("gfx");
        std::fs::write(q1_dir.join("gfx.wad"), wad_with_conchars(&indices)).expect("wad");
        let mut palette = vec![0u8; 768];
        palette[3..6].copy_from_slice(&[10, 20, 30]);
        std::fs::write(q1_dir.join("gfx/palette.lmp"), &palette).expect("palette");
        let q2_dir = scratch_dir("pref-q2");
        let catalog = fake_catalog(vec![
            fake_product(
                "q1-test-base",
                GameFamily::Q1,
                Some(q1_dir.to_string_lossy().into_owned()),
            ),
            fake_product(
                "q2-test-base",
                GameFamily::Q2,
                Some(q2_dir.to_string_lossy().into_owned()),
            ),
        ]);
        let fallback = resolve_menu_charset(&catalog, Some("q2-test-base")).expect("falls through");
        assert_eq!(fallback.product, "q1-test-base");
        assert_eq!(fallback.source, "gfx.wad:conchars");
        let mut q2_indices = vec![2u8; 16 * 16];
        q2_indices[0] = 1;
        let mut q2_palette = vec![0u8; 768];
        q2_palette[3..6].copy_from_slice(&[1, 2, 3]);
        q2_palette[6..9].copy_from_slice(&[4, 5, 6]);
        std::fs::create_dir_all(q2_dir.join("pics")).expect("pics");
        let bytes = encode_pcx(
            &IndexedImage {
                width: 16,
                height: 16,
                indices: q2_indices,
            },
            &q2_palette,
        )
        .expect("pcx");
        std::fs::write(q2_dir.join("pics/conchars.pcx"), &bytes).expect("write");
        let preferred = resolve_menu_charset(&catalog, Some("q2-test-base")).expect("preferred wins");
        assert_eq!(preferred.product, "q2-test-base");
        assert_eq!((preferred.width, preferred.height), (16, 16));
        std::fs::remove_dir_all(&q1_dir).expect("cleanup");
        std::fs::remove_dir_all(&q2_dir).expect("cleanup");
    }

    #[test]
    fn sized_selection_covers_q3_cells_and_upload() {
        let selection = menu_font_selection_for(256, 256, false).unwrap();
        let glyph = resolve_text_glyph(&selection, u32::from(b'A'), false).unwrap();
        assert!(glyph.visible);
        assert_eq!((glyph.glyph.x, glyph.glyph.y), (16, 64));
        assert_eq!((glyph.glyph.width, glyph.glyph.height), (16, 16));
        let uv = glyph_uv(&glyph);
        assert_eq!(
            (uv.s, uv.t, uv.s2, uv.t2),
            (16.0 / 256.0, 64.0 / 256.0, 32.0 / 256.0, 80.0 / 256.0)
        );
        let font = font_image();
        let upload = font_upload_sized(&font, 256, 256, vec![9u8; 256 * 256 * 4]);
        let ImageResourceOperation::CreateImage { content, .. } = upload else {
            panic!("sized upload must create the atlas image");
        };
        let RenderImage::Rgba8 { levels, .. } = content else {
            panic!("sized atlas must be RGBA");
        };
        assert_eq!((levels[0].width, levels[0].height), (256, 256));
    }
}
