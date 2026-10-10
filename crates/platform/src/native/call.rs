//! Load-selected scalar locations for the one native hardware call gate.
use super::{NativeAbi, NativeError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NativeScalar {
    Word = 0,
    Float = 1,
    Double = 2,
    Void = 3,
}
#[derive(Clone, Copy)]
struct Location {
    slot: u8,
    floating: bool,
    mask: u64,
}
#[derive(Clone, Copy)]
pub struct NativeEntry {
    pub(super) address: u64,
    pub(super) abi: NativeAbi,
    pub(super) control: u64,
    locations: [Location; 13],
    count: u8,
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
                mask: if kind == NativeScalar::Float {
                    u32::MAX as u64
                } else {
                    u64::MAX
                },
            };
        }
        Ok(Self {
            address,
            abi,
            control: result as u64 | ((floats as u64) << 8),
            locations,
            count: parameters.len() as u8,
        })
    }
    pub fn argument_count(&self) -> usize {
        self.count as usize
    }
    pub(super) fn pack(&self, values: [u64; 13]) -> ([u64; 13], [u64; 8]) {
        let mut integers = [0; 13];
        let mut floats = [0; 8];
        for (&value, location) in values.iter().zip(&self.locations[..self.argument_count()]) {
            let value = value & location.mask;
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
            *value = bank[location.slot as usize] & location.mask;
        }
        values
    }
}
