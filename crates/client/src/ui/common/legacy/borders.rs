//! Legacy CG rectangle borders (`CG_DrawRect` family).
//!
//! Donor provenance: `src/ui/common/legacy/borders.ts`
//! (`CG_DrawRect`, `CG_DrawTopBottom`, `CG_DrawSides` from id Software's
//! `code/cgame/cg_drawtools.c`). All layout math is `f32`, mirroring the
//! donor's `Math.fround` operation order exactly: since every operand is
//! already `f32`, each donor `f(...)` boundary is a no-op and the
//! parenthesization below preserves the donor's rounding sequence.

use qa_core::math::Vec4;

use crate::text::draw2d::{Draw2D, PictureAsset, Rect, TextureRect};

/// Degenerate UVs used by the CG border painters (donor `uv` constant).
const UV: TextureRect = TextureRect {
    s: 0.0,
    t: 0.0,
    s2: 0.0,
    t2: 0.0,
};

/// Identity marker for a donor `Math.fround` boundary.
///
/// Every value here is already `f32`, so this is a no-op that keeps the
/// donor's rounding order visible at each call site.
#[inline(always)]
fn f(value: f32) -> f32 {
    value
}

/// Paint the top and bottom edges of a CG rectangle (`drawCgTopBottom`).
///
/// Donor order: `r = adjust(rect)`; `vertical = f(f(size) * scaleY)`;
/// top strip `{...r, height: vertical}`; bottom strip at
/// `y = f(f(r.y + r.height) - vertical)` with the same height.
pub fn draw_cg_top_bottom(draw: &mut Draw2D, rect: &Rect, size: f32, picture: PictureAsset) {
    let r = draw.adjust(rect);
    let vertical = f(f(size) * draw.scale_y());
    draw.stretch_pixels(Rect { height: vertical, ..r }, UV, picture);
    draw.stretch_pixels(
        Rect {
            y: f(f(r.y + r.height) - vertical),
            height: vertical,
            ..r
        },
        UV,
        picture,
    );
}

/// Paint the left and right edges of a CG rectangle (`drawCgSides`).
///
/// Donor order: `r = adjust(rect)`; `horizontal = f(f(size) * scaleX)`;
/// left strip `{...r, width: horizontal}`; right strip at
/// `x = f(f(r.x + r.width) - horizontal)` with the same width.
pub fn draw_cg_sides(draw: &mut Draw2D, rect: &Rect, size: f32, picture: PictureAsset) {
    let r = draw.adjust(rect);
    let horizontal = f(f(size) * draw.scale_x());
    draw.stretch_pixels(Rect { width: horizontal, ..r }, UV, picture);
    draw.stretch_pixels(
        Rect {
            x: f(f(r.x + r.width) - horizontal),
            width: horizontal,
            ..r
        },
        UV,
        picture,
    );
}

/// Paint a full CG rectangle outline (`drawCgRect`).
///
/// Sets `color`, paints top/bottom then sides, and resets the color,
/// matching the donor's call order.
pub fn draw_cg_rect(draw: &mut Draw2D, rect: &Rect, size: f32, color: Vec4, picture: PictureAsset) {
    draw.set_color(Some(color));
    draw_cg_top_bottom(draw, rect, size, picture);
    draw_cg_sides(draw, rect, size, picture);
    draw.set_color(None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::draw2d::{CoordinateSpace, DrawCommand, TextCommandSink};
    use qa_core::identity::IdentityOwner;

    /// White image picture handle used by the fixtures.
    fn picture() -> PictureAsset {
        PictureAsset::Image(crate::text::draw2d::ImagePicture {
            image: 1,
            width: 8,
            height: 8,
        })
    }

    /// Recording sink over a 640x480 target plus its owner seat.
    fn sink() -> TextCommandSink {
        let owner = IdentityOwner::create("borders-test").unwrap();
        TextCommandSink::new(
            owner.seat(0),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
        )
    }

    /// Stretch-pic destinations recorded by a sink, in order.
    fn stretch_rects(sink: &TextCommandSink) -> Vec<Rect> {
        sink.commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::StretchPic { rect, .. } => Some(*rect),
                DrawCommand::SetColor(_) | DrawCommand::Material { .. } => None,
            })
            .collect()
    }

    #[test]
    fn top_bottom_paints_exact_strips() {
        let mut sink = sink();
        let rect = Rect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        };
        {
            let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Stretch640);
            draw_cg_top_bottom(&mut draw, &rect, 2.0, picture());
        }
        assert_eq!(
            stretch_rects(&sink),
            vec![
                Rect {
                    x: 10.0,
                    y: 20.0,
                    width: 100.0,
                    height: 2.0,
                },
                Rect {
                    x: 10.0,
                    y: 68.0,
                    width: 100.0,
                    height: 2.0,
                },
            ]
        );
    }

    #[test]
    fn sides_paint_exact_strips() {
        let mut sink = sink();
        let rect = Rect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        };
        {
            let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Stretch640);
            draw_cg_sides(&mut draw, &rect, 3.0, picture());
        }
        assert_eq!(
            stretch_rects(&sink),
            vec![
                Rect {
                    x: 10.0,
                    y: 20.0,
                    width: 3.0,
                    height: 50.0,
                },
                Rect {
                    x: 107.0,
                    y: 20.0,
                    width: 3.0,
                    height: 50.0,
                },
            ]
        );
    }

    #[test]
    fn rect_sets_color_and_paints_four_edges() {
        let mut sink = sink();
        let color = Vec4 {
            x: 1.0,
            y: 0.0,
            z: 1.0,
            w: 1.0,
        };
        {
            let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Stretch640);
            draw_cg_rect(
                &mut draw,
                &Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 64.0,
                    height: 48.0,
                },
                1.0,
                color,
                picture(),
            );
        }
        assert_eq!(stretch_rects(&sink).len(), 4);
        assert_eq!(sink.commands.first(), Some(&DrawCommand::SetColor(color)));
        assert_eq!(
            sink.commands.last(),
            Some(&DrawCommand::SetColor(crate::text::draw2d::WHITE))
        );
    }

    #[test]
    fn fround_order_matches_donor_grouping() {
        // Donor bottom-edge y is f(f(r.y + r.height) - vertical) with
        // vertical = f(f(size) * scaleY); the grouping below must equal the
        // painter output bit-for-bit, including under scaling.
        let mut sink = sink();
        let rect = Rect {
            x: 7.0,
            y: 11.0,
            width: 90.0,
            height: 33.0,
        };
        {
            let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Stretch640);
            draw_cg_top_bottom(&mut draw, &rect, 2.0, picture());
            draw_cg_sides(&mut draw, &rect, 2.0, picture());
        }
        let r = Rect {
            x: 7.0,
            y: 11.0,
            width: 90.0,
            height: 33.0,
        };
        let vertical = f(f(2.0) * 1.0);
        let horizontal = f(f(2.0) * 1.0);
        assert_eq!(
            stretch_rects(&sink),
            vec![
                Rect { height: vertical, ..r },
                Rect {
                    y: f(f(r.y + r.height) - vertical),
                    height: vertical,
                    ..r
                },
                Rect { width: horizontal, ..r },
                Rect {
                    x: f(f(r.x + r.width) - horizontal),
                    width: horizontal,
                    ..r
                },
            ]
        );
    }
}
