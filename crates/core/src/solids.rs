//! Native packed bounds shared by module linking, wire projection and CLIENT queries.
use crate::primitives::{Bounds, RuleSetId, Vec3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PackedSolid {
    None,
    Box(Bounds),
    Brush,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Field {
    extent: usize,
    shift: u8,
    bits: u8,
    divisor: u8,
    bias: i16,
    minimum: i32,
}
impl Field {
    const fn new(extent: usize, shift: u8, bits: u8, divisor: u8, bias: i16, minimum: i32) -> Self {
        Self {
            extent,
            shift,
            bits,
            divisor,
            bias,
            minimum,
        }
    }
    const fn maximum(self) -> u32 {
        (1 << self.bits) - 1
    }
}

/// Encoding capability data, independent of map, movement and clipping rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SolidCodec {
    fields: &'static [Field],
    brush: u32,
    empty_is_none: bool,
    box_cannot_be_brush: bool,
}
impl SolidCodec {
    pub const QUAKE2: Self = Self {
        fields: &[
            Field::new(0, 0, 5, 8, 0, 1),
            Field::new(2, 5, 5, 8, 0, 1),
            Field::new(3, 10, 6, 8, 32, 1),
        ],
        brush: 31,
        empty_is_none: false,
        box_cannot_be_brush: false,
    };
    pub const QUAKE2_RERELEASE: Self = Self {
        fields: &[
            Field::new(0, 0, 8, 1, 0, 1),
            Field::new(1, 8, 8, 1, 0, 1),
            Field::new(2, 16, 8, 1, 0, 0),
            Field::new(3, 24, 8, 1, 32, 0),
        ],
        brush: 31,
        empty_is_none: true,
        box_cannot_be_brush: true,
    };
    pub const QUAKE3: Self = Self {
        fields: &[
            Field::new(0, 0, 8, 1, 0, 1),
            Field::new(2, 8, 8, 1, 0, 1),
            Field::new(3, 16, 8, 1, 32, 1),
        ],
        brush: 0x00ff_ffff,
        empty_is_none: false,
        box_cannot_be_brush: false,
    };

    pub const fn load(source: RuleSetId) -> Option<Self> {
        [
            None,
            None,
            Some(Self::QUAKE2),
            Some(Self::QUAKE2_RERELEASE),
            Some(Self::QUAKE3),
        ][source as usize]
    }
    pub const fn bsp(self) -> u32 {
        self.brush
    }

    /// Link callers admit finite local bounds. Keep source float operations
    /// before native integer truncation; this does not expand link bounds.
    pub fn pack_bounds(self, bounds: Bounds) -> u32 {
        if self.empty_is_none && bounds.mins == bounds.maxs {
            return 0;
        }
        let extents = [
            bounds.maxs.0[0],
            bounds.maxs.0[1],
            -bounds.mins.0[2],
            bounds.maxs.0[2],
        ];
        let mut word = 0;
        for field in self.fields {
            let value =
                ((extents[field.extent] + f32::from(field.bias)) / f32::from(field.divisor)) as i32;
            word |= (value.clamp(field.minimum, field.maximum() as i32) as u32) << field.shift;
        }
        if self.box_cannot_be_brush && word == self.brush {
            0
        } else {
            word
        }
    }

    /// Classify the complete native word before extracting fields. A native
    /// public32 value is not implicitly narrowed to its negotiated wire width.
    pub fn decode(self, word: u32) -> PackedSolid {
        if word == 0 {
            return PackedSolid::None;
        }
        if word == self.brush {
            return PackedSolid::Brush;
        }
        let mut extents = [0.; 4];
        for field in self.fields {
            let value = ((word >> field.shift) & field.maximum()) as f32 * f32::from(field.divisor)
                - f32::from(field.bias);
            extents[field.extent] = value;
            if field.extent == 0 {
                extents[1] = value;
            }
        }
        PackedSolid::Box(Bounds {
            mins: Vec3([-extents[0], -extents[1], -extents[2]]),
            maxs: Vec3([extents[0], extents[1], extents[3]]),
        })
    }

    /// Preserve same-layout native bits. Foreign boxes are decoded/repacked;
    /// invalid bounds or a target brush collision cannot invent a model ref.
    /// The caller separately resolves a Brush's primary model resource.
    pub fn transcode(self, source: Self, word: u32) -> Option<u32> {
        if self == source {
            return Some(word);
        }
        Some(match source.decode(word) {
            PackedSolid::None => 0,
            PackedSolid::Brush => self.brush,
            PackedSolid::Box(bounds) => {
                if !bounds.is_valid() {
                    return None;
                }
                let word = self.pack_bounds(bounds);
                if word == self.brush {
                    return None;
                }
                word
            }
        })
    }
}
