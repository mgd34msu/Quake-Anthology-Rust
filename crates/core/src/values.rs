//! One safe, load-sized numeric storage implementation for player/HUD values.
use crate::primitives::{NumericValue, ValueBank, ValueBinding, ValueId, ValueReset, ValueWidth};

impl ValueBinding {
    pub fn reset_life(self, bank: &mut ValueBank) -> bool {
        self.reset == ValueReset::Session || bank.set(self.id, NumericValue::default())
    }
    pub fn import(self, bank: &mut ValueBank, bits: u32) -> bool {
        let value = match self.width {
            ValueWidth::Signed16 => NumericValue::integer(i32::from(bits as i16)),
            ValueWidth::Signed32 | ValueWidth::Float32 => NumericValue(bits),
        };
        bank.set(self.id, value)
    }
    pub fn export(self, bank: &ValueBank) -> Option<u32> {
        let value = bank.get(self.id)?;
        Some(match self.width {
            ValueWidth::Signed16 => u32::from(value.0 as u16),
            ValueWidth::Signed32 | ValueWidth::Float32 => value.0,
        })
    }
}

impl NumericValue {
    pub fn integer(value: i32) -> Self {
        Self(value as u32)
    }
    pub fn float(value: f32) -> Self {
        Self(value.to_bits())
    }
    pub fn as_integer(self) -> i32 {
        self.0 as i32
    }
    pub fn as_float(self) -> f32 {
        f32::from_bits(self.0)
    }
}
impl ValueBank {
    pub fn load(capacity: usize) -> Self {
        Self {
            bits: vec![0; capacity].into_boxed_slice(),
        }
    }
    pub fn capacity(&self) -> usize {
        self.bits.len()
    }
    pub fn get(&self, id: ValueId) -> Option<NumericValue> {
        self.bits.get(id.0 as usize).copied().map(NumericValue)
    }
    pub fn set(&mut self, id: ValueId, value: NumericValue) -> bool {
        let Some(slot) = self.bits.get_mut(id.0 as usize) else {
            return false;
        };
        *slot = value.0;
        true
    }
    pub fn clear(&mut self) {
        self.bits.fill(0);
    }
    /// Copy into load-sized snapshot storage; report values that do not fit.
    pub fn copy_from(&mut self, source: &Self) -> usize {
        self.bits.fill(0);
        let count = self.bits.len().min(source.bits.len());
        self.bits[..count].copy_from_slice(&source.bits[..count]);
        source.bits.len() - count
    }
}
