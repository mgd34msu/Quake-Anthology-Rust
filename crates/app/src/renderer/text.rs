//! One bitmap-glyph submission path for console and leased HUD text.
use qa_console::presentation::Cell;
use qa_render::{Draw2d, Frame, ShaderBindingId, Viewport};

#[derive(Clone, Copy, Debug)]
pub struct BitmapFont {
    pub shader: ShaderBindingId,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct TextStats {
    pub glyphs: u32,
    pub rejected: u32,
}
impl std::ops::AddAssign for TextStats {
    fn add_assign(&mut self, rhs: Self) {
        self.glyphs += rhs.glyphs;
        self.rejected += rhs.rejected;
    }
}
const COLORS: [[u8; 4]; 8] = [
    [0, 0, 0, 255],
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [255, 255, 0, 255],
    [0, 0, 255, 255],
    [0, 255, 255, 255],
    [255, 0, 255, 255],
    [255; 4],
];
impl BitmapFont {
    pub fn cells(
        self,
        frame: &mut Frame,
        cells: &[Cell],
        x: i64,
        y: i64,
        clip: Viewport,
    ) -> TextStats {
        let mut stats = TextStats::default();
        for (column, cell) in cells.iter().enumerate() {
            stats += self.glyph(
                frame,
                cell.glyph,
                COLORS[usize::from(cell.color)],
                x + column as i64 * 8,
                y,
                clip,
            );
        }
        stats
    }
    pub fn bytes(
        self,
        frame: &mut Frame,
        bytes: &[u8],
        xor: u8,
        x: i64,
        y: i64,
        clip: Viewport,
    ) -> TextStats {
        let mut stats = TextStats::default();
        for (column, &byte) in bytes.iter().enumerate() {
            stats += self.glyph(frame, byte ^ xor, [255; 4], x + column as i64 * 8, y, clip);
        }
        stats
    }
    fn glyph(
        self,
        frame: &mut Frame,
        glyph: u8,
        color: [u8; 4],
        x: i64,
        y: i64,
        clip: Viewport,
    ) -> TextStats {
        if glyph & 127 == b' ' {
            return TextStats::default();
        }
        let x = x as f32;
        let y = y as f32;
        let height = self.height as f32;
        let left = x.max(clip.x as f32);
        let top = y.max(clip.y as f32);
        let right = (x + 8.0).min((u64::from(clip.x) + u64::from(clip.width)) as f32);
        let bottom = (y + height).min((u64::from(clip.y) + u64::from(clip.height)) as f32);
        if right <= left || bottom <= top {
            return TextStats::default();
        }
        let s = f32::from(glyph & 15) * 0.0625;
        let t = f32::from(glyph >> 4) * 0.0625;
        let accepted = frame.draw_2d(Draw2d {
            rect: [left, top, right - left, bottom - top],
            texcoords: [
                s + (left - x) / 8.0 * 0.0625,
                t + (top - y) / height * 0.0625,
                s + (right - x) / 8.0 * 0.0625,
                t + (bottom - y) / height * 0.0625,
            ],
            color,
            shader: self.shader,
        });
        TextStats {
            glyphs: u32::from(accepted),
            rejected: u32::from(!accepted),
        }
    }
}
