use std::ops::{BitOr, BitOrAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Contents(pub u64);

impl Contents {
    pub const EMPTY: Self = Self(0);
    pub const SOLID: Self = Self(1);
    pub const AUX: Self = Self(4);
    pub const LAVA: Self = Self(8);
    pub const SLIME: Self = Self(16);
    pub const WATER: Self = Self(32);
    pub const PLAYER_CLIP: Self = Self(0x10000);
    pub const MONSTER_CLIP: Self = Self(0x20000);
    pub const BODY: Self = Self(0x02000000);
    pub const CORPSE: Self = Self(0x04000000);
    pub const WINDOW: Self = Self(1 << 32);
    pub const SKY: Self = Self(1 << 33);
    pub const LADDER: Self = Self(1 << 34);
    pub const FLUID: Self = Self(8 | 16 | 32);

    pub fn intersects(self, mask: Self) -> bool {
        self.0 & mask.0 != 0
    }

    pub fn from_q1(raw: i32) -> Self {
        match raw {
            -2 => Self::SOLID,
            -3 => Self::WATER,
            -4 => Self::SLIME,
            -5 => Self::LAVA,
            -6 => Self::SKY,
            -14..=-9 => Self(Self::WATER.0 | (1 << (35 + (-9 - raw)))),
            _ => Self::EMPTY,
        }
    }

    pub fn from_q2(raw: u32) -> Self {
        let mut bits = u64::from(raw & 0x0f03807d);
        if raw & 2 != 0 {
            bits |= Self::WINDOW.0;
        }
        if raw & 0xc0000000 != 0 {
            bits |= Self::BODY.0;
        }
        if raw & 0x10000000 != 0 {
            bits |= 0x20000000;
        }
        if raw & 0x20000000 != 0 {
            bits |= Self::LADDER.0;
        }
        bits |= u64::from((raw >> 18) & 0x3f) << 35;
        Self(bits)
    }

    pub fn from_q3(raw: u32) -> Self {
        Self(u64::from(raw))
    }
}

impl BitOr for Contents {
    type Output = Self;
    fn bitor(self, right: Self) -> Self {
        Self(self.0 | right.0)
    }
}

impl BitOrAssign for Contents {
    fn bitor_assign(&mut self, right: Self) {
        self.0 |= right.0;
    }
}
