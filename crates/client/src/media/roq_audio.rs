//! RoQ RLL audio decode.
//!
//! Donor provenance: `src/media/roq-audio.ts` (`SourceRoqAudio`,
//! `RllSetupTable` and the `RllDecode` functions from id Software
//! `code/client/cl_cin.c`, Copyright (C) 1999-2005 Id Software, Inc.
//! GPL-2.0-or-later).

use crate::ClientError;

/// Square-table delta table (`SourceRoqAudio`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRoqAudio {
    square: [i16; 256],
}

impl Default for SourceRoqAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceRoqAudio {
    /// Zeroed table (state before `setup_table`).
    #[must_use]
    pub fn new() -> Self {
        Self { square: [0; 256] }
    }

    /// Build the square table (`RllSetupTable`).
    pub fn setup_table(&mut self) {
        for (index, slot) in self.square.iter_mut().enumerate() {
            let signed = if index < 128 {
                index as i32 * index as i32
            } else {
                -((index - 128) as i32 * (index - 128) as i32)
            };
            *slot = signed as i16;
        }
    }

    fn delta(&self, value: u8) -> i32 {
        i32::from(self.square[usize::from(value)])
    }

    fn byte(input: &[u8], index: usize) -> Result<u8, ClientError> {
        input
            .get(index)
            .copied()
            .ok_or_else(|| ClientError::BadMedia("RoQ audio input is truncated".to_string()))
    }

    fn write(output: &mut [i16], index: usize, value: i16) -> Result<(), ClientError> {
        output
            .get_mut(index)
            .map(|slot| *slot = value)
            .ok_or_else(|| ClientError::BadMedia("RoQ audio output is truncated".to_string()))
    }

    /// `RllDecodeMonoToMono`.
    pub fn decode_mono_to_mono(
        &self,
        input: &[u8],
        output: &mut [i16],
        size: usize,
        signed_output: bool,
        flag: u16,
    ) -> Result<usize, ClientError> {
        let mut previous = if signed_output {
            i32::from(flag) - 0x8000
        } else {
            i32::from(flag)
        };
        for index in 0..size {
            previous = ((previous + self.delta(Self::byte(input, index)?)) << 16) >> 16;
            Self::write(output, index, previous as i16)?;
        }
        Ok(size)
    }

    /// `RllDecodeMonoToStereo` (the C chained assignment writes the
    /// right-hand sample first).
    pub fn decode_mono_to_stereo(
        &self,
        input: &[u8],
        output: &mut [i16],
        size: usize,
        signed_output: bool,
        flag: u16,
    ) -> Result<usize, ClientError> {
        let mut previous = if signed_output {
            i32::from(flag) - 0x8000
        } else {
            i32::from(flag)
        };
        for index in 0..size {
            previous = ((previous + self.delta(Self::byte(input, index)?)) << 16) >> 16;
            Self::write(output, index * 2 + 1, previous as i16)?;
            Self::write(output, index * 2, previous as i16)?;
        }
        Ok(size)
    }

    /// `RllDecodeStereoToStereo`.
    pub fn decode_stereo_to_stereo(
        &self,
        input: &[u8],
        output: &mut [i16],
        size: usize,
        signed_output: bool,
        flag: u16,
    ) -> Result<usize, ClientError> {
        let bias = if signed_output { 0x8000 } else { 0 };
        let mut left = i32::from(flag & 0xff00) - bias;
        let mut right = i32::from((flag & 0xff) << 8) - bias;
        let mut index = 0;
        while index < size {
            left = ((left + self.delta(Self::byte(input, index)?)) << 16) >> 16;
            right = ((right + self.delta(Self::byte(input, index + 1)?)) << 16) >> 16;
            Self::write(output, index, left as i16)?;
            Self::write(output, index + 1, right as i16)?;
            index += 2;
        }
        Ok(size / 2)
    }

    /// `RllDecodeStereoToMono` (size counts output mono samples and
    /// consumes twice that many input bytes).
    pub fn decode_stereo_to_mono(
        &self,
        input: &[u8],
        output: &mut [i16],
        size: usize,
        signed_output: bool,
        flag: u16,
    ) -> Result<usize, ClientError> {
        let bias = if signed_output { 0x8000 } else { 0 };
        let mut left = i32::from(flag & 0xff00) - bias;
        let mut right = i32::from((flag & 0xff) << 8) - bias;
        for index in 0..size {
            left += self.delta(Self::byte(input, index * 2)?);
            right += self.delta(Self::byte(input, index * 2 + 1)?);
            Self::write(output, index, (left.wrapping_add(right) / 2) as i16)?;
        }
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio() -> SourceRoqAudio {
        let mut audio = SourceRoqAudio::new();
        audio.setup_table();
        audio
    }

    #[test]
    fn decode_vectors_match_donor() {
        // Oracle vectors from the donor under bun.
        let audio = audio();
        let mut out = [0i16; 6];
        assert_eq!(
            audio
                .decode_mono_to_mono(&[0, 1, 2, 127, 128, 255], &mut out, 6, false, 1000)
                .unwrap(),
            6
        );
        assert_eq!(out, [1000, 1001, 1005, 17134, 17134, 1005]);
        let mut out = [0i16; 2];
        assert_eq!(
            audio
                .decode_mono_to_mono(&[1, 2], &mut out, 2, true, 0x8000 + 100)
                .unwrap(),
            2
        );
        assert_eq!(out, [101, 105]);
        let mut out = [0i16; 8];
        assert_eq!(
            audio
                .decode_mono_to_stereo(&[0, 1, 2, 3], &mut out, 4, false, 500)
                .unwrap(),
            4
        );
        assert_eq!(out, [500, 500, 501, 501, 505, 505, 514, 514]);
        let mut out = [0i16; 4];
        assert_eq!(
            audio
                .decode_stereo_to_stereo(&[1, 2, 3, 4], &mut out, 4, false, 0x1234)
                .unwrap(),
            2
        );
        assert_eq!(out, [4609, 13316, 4618, 13332]);
        let mut out = [0i16; 2];
        assert_eq!(
            audio
                .decode_stereo_to_stereo(&[10, 20], &mut out, 2, true, 0x8000)
                .unwrap(),
            1
        );
        assert_eq!(out, [100, -32368]);
        let mut out = [0i16; 2];
        assert_eq!(
            audio
                .decode_stereo_to_mono(&[1, 2, 3, 4], &mut out, 2, false, 0x1000)
                .unwrap(),
            2
        );
        assert_eq!(out, [2050, 2063]);
        let mut out = [0i16; 3];
        audio
            .decode_mono_to_mono(&[127, 127, 127], &mut out, 3, false, 30000)
            .unwrap();
        assert_eq!(out, [-19407, -3278, 12851]);
    }

    #[test]
    fn truncation_is_an_error() {
        let audio = audio();
        let mut out = [0i16; 4];
        assert!(audio.decode_mono_to_mono(&[1], &mut out, 2, false, 0).is_err());
        let mut out = [0i16; 1];
        assert!(audio.decode_mono_to_mono(&[1, 2], &mut out, 2, false, 0).is_err());
        // Odd stereo sizes overrun the input pair.
        let mut out = [0i16; 4];
        assert!(audio
            .decode_stereo_to_stereo(&[1, 2, 3], &mut out, 3, false, 0)
            .is_err());
        let fresh = SourceRoqAudio::new();
        let mut out = [0i16; 1];
        // Zero state before setup: deltas are zero.
        fresh.decode_mono_to_mono(&[200], &mut out, 1, false, 7).unwrap();
        assert_eq!(out, [7]);
    }
}
