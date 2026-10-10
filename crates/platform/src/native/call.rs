//! Load-selected scalar locations for the one native hardware call gate.
use super::{NativeAbi, NativeError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NativeScalar {
    Word = 0,
    Float = 1,
    Double = 2,
    Void = 3,
    I8 = 4,
    U8 = 5,
    I16 = 6,
    U16 = 7,
    I32 = 8,
    U32 = 9,
}
impl NativeScalar {
    pub fn integer(self) -> bool {
        !matches!(self, Self::Float | Self::Double | Self::Void)
    }
}
#[derive(Clone, Copy)]
struct Location {
    slot: u8,
    floating: bool,
    mask: u64,
    sign_shift: u8,
}
impl Location {
    fn scalar(kind: NativeScalar) -> Self {
        let (mask, sign_shift) = match kind {
            NativeScalar::I8 => (u8::MAX as u64, 56),
            NativeScalar::U8 => (u8::MAX as u64, 0),
            NativeScalar::I16 => (u16::MAX as u64, 48),
            NativeScalar::U16 => (u16::MAX as u64, 0),
            NativeScalar::I32 => (u32::MAX as u64, 32),
            NativeScalar::U32 | NativeScalar::Float => (u32::MAX as u64, 0),
            NativeScalar::Void => (0, 0),
            _ => (u64::MAX, 0),
        };
        Self {
            slot: 0,
            floating: false,
            mask,
            sign_shift,
        }
    }
    fn value(self, value: u64) -> u64 {
        let value = value & self.mask;
        ((value << self.sign_shift) as i64 >> self.sign_shift) as u64
    }
}
#[derive(Clone, Copy)]
pub struct NativeEntry {
    pub(super) address: u64,
    pub(super) abi: NativeAbi,
    pub(super) control: u64,
    locations: [Location; 13],
    count: u8,
    result: Location,
}
impl NativeEntry {
    pub(super) fn bind(
        address: u64,
        abi: NativeAbi,
        parameters: &[NativeScalar],
        result: NativeScalar,
    ) -> Result<Self, NativeError> {
        if parameters.len() > 13 || parameters.contains(&NativeScalar::Void) {
            return Err(NativeError::Unsupported);
        }
        let mut locations = [Location {
            slot: 0,
            floating: false,
            mask: u64::MAX,
            sign_shift: 0,
        }; 13];
        let (mut integers, mut floats, mut stack) = (0, 0, 6);
        for (ordinal, &kind) in parameters.iter().enumerate() {
            let floating = matches!(kind, NativeScalar::Float | NativeScalar::Double);
            let (slot, floating) = match abi {
                // Positional registers: a float in position 2 uses XMM2 even
                // when the preceding parameters are integer words.
                NativeAbi::Microsoft => (ordinal, ordinal < 4 && floating),
                // INTEGER and SSE have independent register sequences.
                NativeAbi::SystemV if floating && floats < 8 => {
                    let slot = floats;
                    floats += 1;
                    (slot, true)
                }
                NativeAbi::SystemV if !floating && integers < 6 => {
                    let slot = integers;
                    integers += 1;
                    (slot, false)
                }
                NativeAbi::SystemV => {
                    let slot = stack;
                    stack += 1;
                    (slot, false)
                }
            };
            locations[ordinal] = Location {
                slot: slot as u8,
                floating,
                ..Location::scalar(kind)
            };
        }
        Ok(Self {
            address,
            abi,
            control: match result {
                NativeScalar::Float => 1,
                NativeScalar::Double => 2,
                NativeScalar::Void => 3,
                _ => 0,
            } | ((floats as u64) << 8),
            locations,
            count: parameters.len() as u8,
            result: Location::scalar(result),
        })
    }
    pub fn argument_count(&self) -> usize {
        self.count as usize
    }
    pub(super) fn pack(&self, values: [u64; 13]) -> ([u64; 13], [u64; 8]) {
        let mut integers = [0; 13];
        let mut floats = [0; 8];
        for (&value, location) in values.iter().zip(&self.locations[..self.argument_count()]) {
            let value = location.value(value);
            if location.floating {
                floats[location.slot as usize] = value;
            } else {
                integers[location.slot as usize] = value;
            }
        }
        (integers, floats)
    }
    pub(super) fn unpack(&self, integers: [u64; 13], floats: [u64; 8]) -> [u64; 13] {
        let mut values = [0; 13];
        for (value, location) in values
            .iter_mut()
            .zip(&self.locations[..self.argument_count()])
        {
            let bank = if location.floating {
                &floats[..]
            } else {
                &integers[..]
            };
            *value = location.value(bank[location.slot as usize]);
        }
        values
    }
    pub(super) fn result(&self, value: u64) -> u64 {
        self.result.value(value)
    }
}
