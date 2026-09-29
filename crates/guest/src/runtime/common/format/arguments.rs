//! `printf`/`scanf` varargs reader over guest `va_list` storage.
//!
//! Donor: `src/guest/runtime/common/format/arguments.ts`. `va_list` is
//! passed by value on Windows/i386 and points to the ABI save descriptor on
//! SysV x64.

use crate::core::contracts::GuestAddress;
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::floating_point::binary::{decode_binary, read_bits, BigInt, BinaryValue, BinaryWidth};
use crate::runtime::common::memory::read_unsigned;

/// Runtime dialect selecting varargs and conversion rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormatDialect {
    /// UCRT/MSVCRT rules.
    Windows,
    /// glibc rules.
    SystemV,
}

/// One formatted argument type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatArgumentType {
    /// 32-bit integer lane.
    Int,
    /// 64-bit integer lane.
    Int64,
    /// Pointer lane.
    Pointer,
    /// Binary64 lane.
    Double,
    /// Extended-precision lane (`L` on SysV).
    LongDouble,
}

/// One decoded varargs value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatArgument {
    /// Raw integer lane contents.
    Integer(u64),
    /// Decoded binary float.
    Float(BinaryValue),
}

/// Cursor over guest varargs storage.
#[derive(Debug)]
pub struct FormatArguments {
    cursor: Option<GuestAddress>,
    gp: u32,
    fp: u32,
    registers: Option<GuestAddress>,
    descriptor: Option<GuestAddress>,
    dialect: FormatDialect,
    pointer_bytes: usize,
}

impl FormatArguments {
    /// Open the varargs list at `address`.
    pub fn open(
        memory: &mut SparseGuestMemory,
        dialect: FormatDialect,
        address: Option<GuestAddress>,
    ) -> Result<Self, GuestError> {
        let pointer_bytes = memory.pointer_bytes();
        let descriptor = if dialect == FormatDialect::SystemV && pointer_bytes == 8 {
            address
        } else {
            None
        };
        let mut args = Self {
            cursor: None,
            gp: 0,
            fp: 0,
            registers: None,
            descriptor,
            dialect,
            pointer_bytes,
        };
        if let Some(descriptor) = descriptor {
            args.gp = memory.read_u32(descriptor)?;
            args.fp = memory.read_u32(memory.offset(descriptor, 4)?)?;
            args.cursor = memory.read_pointer(memory.offset(descriptor, 8)?)?;
            args.registers = memory.read_pointer(memory.offset(descriptor, 16)?)?;
            if args.gp > 48
                || !args.gp.is_multiple_of(8)
                || args.fp < 48
                || args.fp > 176
                || !(args.fp - 48).is_multiple_of(16)
            {
                return Err(GuestError::invalid("Invalid System V x64 va_list register offsets"));
            }
        } else {
            args.cursor = address;
        }
        Ok(args)
    }

    /// Read the next argument as `argument_type`.
    pub fn next(
        &mut self,
        memory: &mut SparseGuestMemory,
        argument_type: FormatArgumentType,
    ) -> Result<FormatArgument, GuestError> {
        let extended = argument_type == FormatArgumentType::LongDouble && self.dialect == FormatDialect::SystemV;
        let floating = argument_type == FormatArgumentType::Double || argument_type == FormatArgumentType::LongDouble;
        let size = if extended {
            if self.pointer_bytes == 8 {
                16
            } else {
                12
            }
        } else if floating || argument_type == FormatArgumentType::Int64 {
            8
        } else if argument_type == FormatArgumentType::Pointer {
            self.pointer_bytes
        } else {
            4
        };
        let address = if self.descriptor.is_some() && !extended && (if floating { self.fp < 176 } else { self.gp < 48 })
        {
            let registers = self
                .registers
                .ok_or_else(|| GuestError::invalid("va_list register save area is null"))?;
            let address = memory.offset(registers, i64::from(if floating { self.fp } else { self.gp }))?;
            let Some(descriptor) = self.descriptor else {
                unreachable!("descriptor checked above");
            };
            if floating {
                self.fp += 16;
                memory.write_u32(memory.offset(descriptor, 4)?, self.fp)?;
            } else {
                self.gp += 8;
                memory.write_u32(descriptor, self.gp)?;
            }
            address
        } else {
            let cursor = self
                .cursor
                .ok_or_else(|| GuestError::invalid("va_list argument storage is null"))?;
            let alignment = if self.descriptor.is_some() && extended {
                16u64
            } else {
                self.pointer_bytes as u64
            };
            let aligned = cursor.offset.div_ceil(alignment) * alignment;
            let address = memory.offset(cursor, (aligned - cursor.offset) as i64)?;
            let slot = if self.pointer_bytes == 8 {
                size.div_ceil(8) * 8
            } else {
                size.div_ceil(4) * 4
            };
            self.cursor = Some(memory.offset(address, slot as i64)?);
            if let Some(descriptor) = self.descriptor {
                memory.write_pointer(memory.offset(descriptor, 8)?, self.cursor)?;
            }
            address
        };
        if floating {
            let bits = if extended {
                read_bits(&memory.copy(address, 10)?)
            } else {
                BigInt::from_u64(read_unsigned(memory, address, 8)?)
            };
            Ok(FormatArgument::Float(decode_binary(
                &bits,
                if extended { BinaryWidth::W80 } else { BinaryWidth::W64 },
            )))
        } else {
            Ok(FormatArgument::Integer(read_unsigned(memory, address, size)?))
        }
    }
}
