//! Synthetic console-charset text for the windowed startup menu.
//!
//! Donor provenance: `src/text/atlas.ts` (`classicCharset`: a 16 by 16
//! cell atlas over `conchars`) and `src/text/ui.ts` (`UiTextRenderer.draw`
//! lays out with `layoutText` then draws one `stretchPic` per visible glyph
//! with atlas UVs). The donor loads `conchars` pixels from mounted game
//! content; the windowed menu entry runs with no catalog mounts, so this
//! module generates an equivalent 128 by 128 atlas procedurally: white 8 by
//! 8 glyphs on transparent cells, tinted per run through vertex colors.
//!
//! Diagnosis (blank menu labels): the menu capture drew every text run as
//! one measured solid bar and dropped UVs at emit time, so no per-glyph
//! pictures ever reached the GL batches. The faithful shape kept here is
//! layout then per-glyph quads: [`conchars_rgba`] supplies the atlas texels
//! uploaded once beside the 1x1 white image, and [`glyph_batches`] turns
//! captured glyph quads into one textured overlay batch over that upload.

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
    Ok(TextFontSelection::Classic {
        classic: classic_charset(FONT_PICTURE_HANDLE, CONCHARS_WIDTH, CONCHARS_HEIGHT, "conchars", false)?,
        unicode: None,
    })
}

/// One captured glyph quad: destination pixels, atlas UVs, and run color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GlyphQuad {
    /// Destination rectangle in drawable pixels.
    pub rect: Rect,
    /// Atlas source coordinates.
    pub uv: TextureRect,
    /// Glyph color.
    pub color: Vec4,
}

/// Textured overlay batches for captured glyph quads in NDC space (the same
/// depth-always blended overlay as the flat menu batch, bound to the font
/// atlas instead of the 1x1 white image).
pub(crate) fn glyph_batches(glyphs: &[GlyphQuad], width: f32, height: f32, font: &RendererImage) -> Vec<DrawBatch> {
    let mut vertices = Vec::with_capacity(glyphs.len() * 4);
    let mut indices = Vec::with_capacity(glyphs.len() * 6);
    for quad in glyphs {
        let base = vertices.len() as u32;
        let left = 2.0 * quad.rect.x / width - 1.0;
        let right = 2.0 * (quad.rect.x + quad.rect.width) / width - 1.0;
        let top = 1.0 - 2.0 * quad.rect.y / height;
        let bottom = 1.0 - 2.0 * (quad.rect.y + quad.rect.height) / height;
        for (x, y, s, t) in [
            (left, top, quad.uv.s, quad.uv.t),
            (right, top, quad.uv.s2, quad.uv.t),
            (right, bottom, quad.uv.s2, quad.uv.t2),
            (left, bottom, quad.uv.s, quad.uv.t2),
        ] {
            vertices.push(RenderVertex {
                position: vec4(x, y, 0.0, 1.0),
                tex_coord: vec2(s, t),
                color: quad.color,
            });
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    if vertices.is_empty() {
        return Vec::new();
    }
    vec![DrawBatch {
        fog: None,
        luminance_alpha: false,
        indices,
        texture: TextureBinding::BindImage(font.clone()),
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
    }]
}

/// Upload operation for the menu font atlas (nearest sampling keeps the 8 by
/// 8 cells crisp; the transparent border matches the transparent cells).
pub(crate) fn font_upload(font: &RendererImage, pixels: Vec<u8>) -> ImageResourceOperation {
    ImageResourceOperation::CreateImage {
        image: font.clone(),
        content: RenderImage::Rgba8 {
            levels: vec![ImageLevel {
                width: CONCHARS_WIDTH,
                height: CONCHARS_HEIGHT,
                pixels,
            }],
            border_color: vec4(0.0, 0.0, 0.0, 0.0),
        },
        sampling: TextureSampling {
            repeat: false,
            filter: TextureFilter::Nearest,
        },
    }
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

    #[test]
    fn glyph_batches_carry_uvs_in_flat_batch_space() {
        let font = font_image();
        assert!(glyph_batches(&[], 640.0, 480.0, &font).is_empty());
        let quad = GlyphQuad {
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
        let batches = glyph_batches(&[quad], 640.0, 480.0, &font);
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
        assert_eq!(
            uvs,
            vec![(0.0625, 0.25), (0.125, 0.25), (0.125, 0.3125), (0.0625, 0.3125)]
        );
        assert!(vertices.iter().all(|vertex| vertex.color.w > 0.0));
    }

    #[test]
    fn font_upload_describes_the_atlas() {
        let font = font_image();
        let upload = font_upload(&font, conchars_rgba());
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
}
