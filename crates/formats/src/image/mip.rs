use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MipFilter {
    Box,
    Quake3Weighted,
}
impl RgbaImage {
    pub fn mip(&self, filter: MipFilter) -> Result<Self, FormatError> {
        let count = pixel_count(self.width, self.height)?;
        if self.pixels.len() != count * 4 {
            return Err(FormatError::InvalidRecordSize);
        }
        if filter == MipFilter::Quake3Weighted
            && (!self.width.is_power_of_two() || !self.height.is_power_of_two())
        {
            return Err(FormatError::InvalidRange);
        }
        let width = (self.width / 2).max(1);
        let height = (self.height / 2).max(1);
        let length = pixel_count(width, height)? * 4;
        if filter == MipFilter::Quake3Weighted && (self.width == 1 || self.height == 1) {
            return Ok(Self {
                width,
                height,
                pixels: self.pixels[..length].to_vec(),
            });
        }
        let mut pixels = vec![0; length];
        for y in 0..height {
            for x in 0..width {
                for c in 0..4 {
                    let mut sum = 0u32;
                    let divisor = if filter == MipFilter::Quake3Weighted {
                        for dy in -1i32..=2 {
                            for dx in -1i32..=2 {
                                let sx = (x * 2).wrapping_add_signed(dx) & (self.width - 1);
                                let sy = (y * 2).wrapping_add_signed(dy) & (self.height - 1);
                                let weight = if dx == -1 || dx == 2 { 1 } else { 2 }
                                    * if dy == -1 || dy == 2 { 1 } else { 2 };
                                sum += weight
                                    * u32::from(
                                        self.pixels[(sy as usize * self.width as usize
                                            + sx as usize)
                                            * 4
                                            + c],
                                    );
                            }
                        }
                        36
                    } else {
                        for sy in [y * 2, (y * 2 + 1).min(self.height - 1)] {
                            for sx in [x * 2, (x * 2 + 1).min(self.width - 1)] {
                                sum += u32::from(
                                    self.pixels
                                        [(sy as usize * self.width as usize + sx as usize) * 4 + c],
                                );
                            }
                        }
                        4
                    };
                    pixels[(y as usize * width as usize + x as usize) * 4 + c] =
                        (sum / divisor) as u8;
                }
            }
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
    pub fn mixed_cutout_mask(&self) -> bool {
        let mut visible = false;
        let mut rejected = false;
        for pixel in self.pixels.as_chunks::<4>().0 {
            if pixel[3] >= 170 {
                visible = true;
            } else {
                rejected = true;
            }
            if visible && rejected {
                return true;
            }
        }
        false
    }
    /// Port the proven cutout fix: stop before a generated level loses its
    /// mixed GT666 mask. The renderer clamps its sampler to retained levels.
    pub fn mip_chain(&self, filter: MipFilter, cutout: bool) -> Result<Vec<Self>, FormatError> {
        let preserve = cutout && self.mixed_cutout_mask();
        let mut chain = Vec::new();
        let mut previous = self;
        while previous.width > 1 || previous.height > 1 {
            let next = previous.mip(filter)?;
            if preserve && !next.mixed_cutout_mask() {
                break;
            }
            chain.push(next);
            previous = &chain[chain.len() - 1];
        }
        Ok(chain)
    }
}
