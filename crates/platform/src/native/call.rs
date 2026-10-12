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
    fn decode(value: u8) -> Result<Self, NativeError> {
        match value {
            0 => Ok(Self::Word),
            1 => Ok(Self::Float),
            2 => Ok(Self::Double),
            3 => Ok(Self::Void),
            4 => Ok(Self::I8),
            5 => Ok(Self::U8),
            6 => Ok(Self::I16),
            7 => Ok(Self::U16),
            8 => Ok(Self::I32),
            9 => Ok(Self::U32),
            _ => Err(NativeError::Protocol),
        }
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
    fn scalar(kind: NativeScalar, pointer_bytes: u8) -> Self {
        let (mask, sign_shift) = match kind {
            NativeScalar::I8 => (u8::MAX as u64, 56),
            NativeScalar::U8 => (u8::MAX as u64, 0),
            NativeScalar::I16 => (u16::MAX as u64, 48),
            NativeScalar::U16 => (u16::MAX as u64, 0),
            NativeScalar::I32 => (u32::MAX as u64, 32),
            NativeScalar::U32 | NativeScalar::Float => (u32::MAX as u64, 0),
            NativeScalar::Void => (0, 0),
            NativeScalar::Word if pointer_bytes == 4 => (u32::MAX as u64, 0),
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
    pub(super) pointer_bytes: u8,
    signature: u64,
    locations: [Location; 13],
    count: u8,
    result: Location,
}
impl NativeEntry {
    pub fn address(&self) -> u64 {
        self.address
    }
    pub(super) fn bind(
        address: u64,
        abi: NativeAbi,
        pointer_bytes: u8,
        parameters: &[NativeScalar],
        result: NativeScalar,
    ) -> Result<Self, NativeError> {
        if parameters.len() > 13
            || parameters.contains(&NativeScalar::Void)
            || !matches!(pointer_bytes, 4 | 8)
            // This gate implements i386 System V cdecl. PE32 stdcall,
            // thiscall and Windows structure layouts remain separate admission
            // requirements, not Win64's positional register convention.
            || (pointer_bytes == 4 && abi != NativeAbi::SystemV)
            || (pointer_bytes == 4 && address > u64::from(u32::MAX))
        {
            return Err(NativeError::Unsupported);
        }
        let mut locations = [Location {
            slot: 0,
            floating: false,
            mask: u64::MAX,
            sign_shift: 0,
        }; 13];
        let (mut integers, mut floats, mut stack) = (0, 0, 6);
        let mut cells = 0;
        let mut signature = (result as u64) << 52 | (parameters.len() as u64) << 56;
        for (ordinal, &kind) in parameters.iter().enumerate() {
            signature |= (kind as u64) << (ordinal * 4);
            let floating = matches!(kind, NativeScalar::Float | NativeScalar::Double);
            let (slot, floating) = if pointer_bytes == 4 {
                let slot = cells;
                cells += if kind == NativeScalar::Double { 2 } else { 1 };
                (slot, false)
            } else {
                match abi {
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
                }
            };
            locations[ordinal] = Location {
                slot: slot as u8,
                floating,
                ..Location::scalar(kind, pointer_bytes)
            };
        }
        Ok(Self {
            address,
            abi,
            pointer_bytes,
            signature,
            control: match result {
                NativeScalar::Float => 1,
                NativeScalar::Double => 2,
                NativeScalar::Void => 3,
                _ => 0,
            } | ((floats as u64) << 8),
            locations,
            count: parameters.len() as u8,
            result: Location::scalar(result, pointer_bytes),
        })
    }
    pub(super) fn signature(&self) -> u64 {
        self.signature
    }
    pub(super) fn from_signature(
        address: u64,
        abi: NativeAbi,
        pointer_bytes: u8,
        signature: u64,
    ) -> Result<Self, NativeError> {
        let count = ((signature >> 56) & 15) as usize;
        if signature >> 60 != 0 || count > 13 {
            return Err(NativeError::Protocol);
        }
        let mut parameters = [NativeScalar::Word; 13];
        for (index, parameter) in parameters[..count].iter_mut().enumerate() {
            *parameter = NativeScalar::decode(((signature >> (index * 4)) & 15) as u8)?;
        }
        let entry = Self::bind(
            address,
            abi,
            pointer_bytes,
            &parameters[..count],
            NativeScalar::decode(((signature >> 52) & 15) as u8)?,
        )?;
        if entry.signature != signature {
            return Err(NativeError::Protocol);
        }
        Ok(entry)
    }
    pub fn argument_count(&self) -> usize {
        self.count as usize
    }
    pub(super) fn pack(&self, values: [u64; 13]) -> ([u64; 13], [u64; 8]) {
        let mut integers = [0; 13];
        let mut floats = [0; 8];
        for (&value, location) in values.iter().zip(&self.locations[..self.argument_count()]) {
            let value = location.value(value);
            if self.pointer_bytes == 4 {
                let cell = usize::from(location.slot);
                integers[cell / 2] |= (value & u64::from(u32::MAX)) << ((cell % 2) * 32);
                if location.mask == u64::MAX {
                    let cell = cell + 1;
                    integers[cell / 2] |= (value >> 32) << ((cell % 2) * 32);
                }
            } else if location.floating {
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
            if self.pointer_bytes == 4 {
                let cell = usize::from(location.slot);
                let mut word = (integers[cell / 2] >> ((cell % 2) * 32)) & u64::from(u32::MAX);
                if location.mask == u64::MAX {
                    let cell = cell + 1;
                    word |= ((integers[cell / 2] >> ((cell % 2) * 32)) & u64::from(u32::MAX)) << 32;
                }
                *value = location.value(word);
                continue;
            }
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
