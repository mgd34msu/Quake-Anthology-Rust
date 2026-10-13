//! Checked integers for fixed-degree binary32 geometry predicates.
use std::cmp::Ordering;

const LIMBS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Int {
    words: [u64; LIMBS],
    len: usize,
    negative: bool,
}

impl Default for Int {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Int {
    pub const ZERO: Self = Self::unsigned(0);
    pub const ONE: Self = Self::unsigned(1);

    const fn unsigned(value: u64) -> Self {
        let mut words = [0; LIMBS];
        words[0] = value;
        Self {
            words,
            len: if value == 0 { 0 } else { 1 },
            negative: false,
        }
    }

    /// Every finite binary32 is exactly X / 2^149, with |X| < 2^277.
    pub fn binary32(value: f32) -> Result<Self, super::Failure> {
        let bits = value.to_bits();
        let exponent = (bits >> 23) & 255;
        if exponent == 255 {
            return Err(super::Failure::Input);
        }
        let fraction = bits & ((1 << 23) - 1);
        let (mantissa, shift) = if exponent == 0 {
            (fraction, 0)
        } else {
            ((1 << 23) | fraction, exponent as usize - 1)
        };
        let mut out = Self::unsigned(u64::from(mantissa)).shift(shift)?;
        out.negative = bits >> 31 != 0 && !out.is_zero();
        Ok(out)
    }

    pub fn is_zero(self) -> bool {
        self.len == 0
    }

    pub fn negative(self) -> bool {
        self.negative
    }

    pub fn neg(mut self) -> Self {
        if !self.is_zero() {
            self.negative = !self.negative;
        }
        self
    }

    fn trim(&mut self) {
        while self.len != 0 && self.words[self.len - 1] == 0 {
            self.len -= 1;
        }
        if self.len == 0 {
            self.negative = false;
        }
    }

    fn magnitude_cmp(&self, other: &Self) -> Ordering {
        match self.len.cmp(&other.len) {
            Ordering::Equal => {
                for index in (0..self.len).rev() {
                    let order = self.words[index].cmp(&other.words[index]);
                    if order != Ordering::Equal {
                        return order;
                    }
                }
                Ordering::Equal
            }
            order => order,
        }
    }

    pub fn shift(&self, bits: usize) -> Result<Self, super::Failure> {
        if self.is_zero() {
            return Ok(*self);
        }
        let bit_len = (self.len - 1) * 64 + 64 - self.words[self.len - 1].leading_zeros() as usize;
        if bit_len.checked_add(bits).is_none_or(|n| n > LIMBS * 64) {
            return Err(super::Failure::Arithmetic);
        }
        let words = bits / 64;
        let offset = bits % 64;
        let mut out = Self::ZERO;
        out.negative = self.negative;
        for index in 0..self.len {
            out.words[index + words] |= self.words[index] << offset;
            if offset != 0 && index + words + 1 < LIMBS {
                out.words[index + words + 1] |= self.words[index] >> (64 - offset);
            }
        }
        out.len = (self.len + words + usize::from(offset != 0)).min(LIMBS);
        out.trim();
        Ok(out)
    }

    fn add_magnitudes(&self, other: &Self) -> Result<Self, super::Failure> {
        let mut out = Self::ZERO;
        let mut carry = 0u128;
        out.len = self.len.max(other.len);
        for index in 0..out.len {
            let sum = u128::from(self.words[index]) + u128::from(other.words[index]) + carry;
            out.words[index] = sum as u64;
            carry = sum >> 64;
        }
        if carry != 0 {
            if out.len == LIMBS {
                return Err(super::Failure::Arithmetic);
            }
            out.words[out.len] = carry as u64;
            out.len += 1;
        }
        Ok(out)
    }

    fn subtract_magnitudes(&self, other: &Self) -> Self {
        let mut out = Self::ZERO;
        out.len = self.len;
        let mut borrow = 0i128;
        for index in 0..self.len {
            let difference =
                i128::from(self.words[index]) - i128::from(other.words[index]) - borrow;
            out.words[index] = difference as u64;
            borrow = if difference < 0 { 1 } else { 0 };
        }
        debug_assert_eq!(borrow, 0);
        out.trim();
        out
    }

    pub fn add(&self, other: &Self) -> Result<Self, super::Failure> {
        if self.negative == other.negative {
            let mut out = self.add_magnitudes(other)?;
            out.negative = self.negative && !out.is_zero();
            Ok(out)
        } else {
            match self.magnitude_cmp(other) {
                Ordering::Greater => {
                    let mut out = self.subtract_magnitudes(other);
                    out.negative = self.negative;
                    Ok(out)
                }
                Ordering::Less => {
                    let mut out = other.subtract_magnitudes(self);
                    out.negative = other.negative;
                    Ok(out)
                }
                Ordering::Equal => Ok(Self::ZERO),
            }
        }
    }

    pub fn mul(&self, other: &Self) -> Result<Self, super::Failure> {
        if self.is_zero() || other.is_zero() {
            return Ok(Self::ZERO);
        }
        if self.len + other.len > LIMBS + 1 {
            return Err(super::Failure::Arithmetic);
        }
        let mut out = Self::ZERO;
        for left in 0..self.len {
            let mut carry = 0u128;
            for right in 0..other.len {
                let index = left + right;
                let product = u128::from(self.words[left]) * u128::from(other.words[right]);
                // (2^64-1)^2 plus two u64 values is at most u128::MAX.
                let sum = product + u128::from(out.words[index]) + carry;
                out.words[index] = sum as u64;
                carry = sum >> 64;
            }
            let index = left + other.len;
            if index == LIMBS {
                if carry != 0 {
                    return Err(super::Failure::Arithmetic);
                }
            } else {
                out.words[index] = carry as u64;
            }
        }
        out.len = (self.len + other.len).min(LIMBS);
        out.negative = self.negative != other.negative;
        out.trim();
        Ok(out)
    }
}
