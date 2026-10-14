//! Source PCM layout. File and streaming readers share sample conversion.
use crate::primitives::PcmChannels;
use std::num::NonZeroU32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmWidth {
    Byte,
    Short,
    ThreeBytes,
}
impl PcmWidth {
    pub fn from_bytes(width: u8) -> Option<Self> {
        match width {
            1 => Some(Self::Byte),
            2 => Some(Self::Short),
            3 => Some(Self::ThreeBytes),
            _ => None,
        }
    }
    pub const fn bytes(self) -> usize {
        match self {
            Self::Byte => 1,
            Self::Short => 2,
            Self::ThreeBytes => 3,
        }
    }
    /// Microsoft PCM bytes are unsigned at width one. Native Q3 raw stereo
    /// bytes have their distinct signed interpretation at that boundary.
    pub fn sample(self, bytes: &[u8], signed_byte: bool) -> i16 {
        match self {
            Self::Byte if signed_byte => i16::from(bytes[0] as i8) * 256,
            Self::Byte => (i16::from(bytes[0]) - 128) * 256,
            Self::Short => i16::from_le_bytes([bytes[0], bytes[1]]),
            Self::ThreeBytes => i16::from_le_bytes([bytes[1], bytes[2]]),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmFormat {
    pub rate: NonZeroU32,
    pub channels: PcmChannels,
    pub width: PcmWidth,
}
impl PcmFormat {
    pub fn frame_bytes(self) -> usize {
        self.width.bytes() * self.channels as usize
    }
}
