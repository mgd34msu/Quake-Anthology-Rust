use super::world::Affine;

/// Carries the native affine advance between perspective chunk endpoints.
pub(super) struct Perspective {
    depth: Affine,
    texture: [Affine; 2],
    adjust: [i64; 2],
    extents: [i64; 2],
    divided: [f32; 2],
    zi: f32,
}

impl Perspective {
    pub(super) fn new(
        depth: Affine,
        texture: [Affine; 2],
        adjust: [i64; 2],
        size: [u32; 2],
        x: u32,
        y: u32,
    ) -> Self {
        Self {
            depth,
            texture,
            adjust,
            extents: size.map(|value| (i64::from(value) << 16) - 1),
            divided: texture.map(|plane| plane.at(x as f32, y as f32)),
            zi: depth.at(x as f32, y as f32),
        }
    }
    pub(super) fn endpoint(&mut self, distance: f32, minimum: i64) -> [i64; 2] {
        // The first endpoint performs no advance, including no addition of zero.
        if minimum != 0 {
            self.divided[0] += self.texture[0].x * distance;
            self.divided[1] += self.texture[1].x * distance;
            self.zi += self.depth.x * distance;
        }
        let z = 65536.0 / self.zi;
        std::array::from_fn(|axis| {
            ((self.divided[axis] * z) as i64 + self.adjust[axis]).clamp(minimum, self.extents[axis])
        })
    }
}

pub(super) fn walk(
    start: u32,
    count: u32,
    chunk: u32,
    mut endpoint: impl FnMut(u32, f32, i64) -> [i64; 2],
    mut write: impl FnMut(u32, [i64; 2]),
) {
    let end = start + count;
    let mut x = start;
    let mut current = endpoint(start, 0.0, 0);
    while x < end {
        let count = chunk.min(end - x);
        let complete = x + count < end;
        let distance = if complete { chunk } else { count - 1 };
        let next = endpoint(x + distance, distance as f32, i64::from(chunk));
        let step = if complete {
            std::array::from_fn(|i| (next[i] - current[i]) >> chunk.trailing_zeros())
        } else if count > 1 {
            std::array::from_fn(|i| (next[i] - current[i]) / i64::from(count - 1))
        } else {
            [0; 2]
        };
        for offset in 0..count {
            write(x + offset, current);
            current[0] += step[0];
            current[1] += step[1];
        }
        x += count;
        current = next;
    }
}

pub(super) fn texel(coordinates: [i64; 2], width: u32, height: u32) -> usize {
    let x = (coordinates[0] >> 16).clamp(0, i64::from(width) - 1) as usize;
    let y = (coordinates[1] >> 16).clamp(0, i64::from(height) - 1) as usize;
    y * width as usize + x
}

#[cfg(test)]
#[path = "../../tests/spans/oracle.rs"]
mod tests;
