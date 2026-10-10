//! Load-time ELF relocation arithmetic. Hardware resolvers belong to the child.
mod lifecycle;
use crate::memory::ModuleMemory;
pub(super) use lifecycle::Lifecycle;
use qa_formats::{
    FormatError,
    program::native::{Encoding, Image, Relocation, Symbol},
};

#[derive(Clone, Copy, Debug)]
pub struct TlsBlock {
    pub module: u64,
    pub bytes: u64,
    pub thread_pointer_offset: i64,
}
#[derive(Clone, Copy, Debug)]
pub enum Definition<'a> {
    Address { address: u64, bytes: u64 },
    Tls { block: TlsBlock, offset: u64 },
    Copy { address: u64, bytes: &'a [u8] },
}
pub struct Bindings<'a> {
    /// Dense native symbol ordinals, already resolved by the owning loader.
    /// None uses an ordinary local definition or an undefined weak zero.
    pub symbols: &'a [Option<Definition<'a>>],
    pub local_tls: Option<TlsBlock>,
    /// Native child results indexed by relocation ordinal. The regular pass
    /// runs first; no engine-process foreign resolver is admitted here.
    pub indirect: &'a [Option<u64>],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    Regular,
    Indirect,
}

pub fn deferred(image: &Image, relocation: &Relocation) -> bool {
    relocation.kind == if image.target.bits == 64 { 37 } else { 42 }
        || relocation
            .symbol
            .and_then(|i| image.symbols.get(i))
            .is_some_and(|s| s.kind == 10 && s.defined)
}
fn definition<'a>(
    symbol: Option<(usize, &Symbol)>,
    bindings: &Bindings<'a>,
) -> Result<Definition<'a>, FormatError> {
    let Some((index, symbol)) = symbol else {
        return Ok(Definition::Address {
            address: 0,
            bytes: 0,
        });
    };
    if let Some(value) = bindings.symbols.get(index).copied().flatten() {
        return Ok(value);
    }
    if symbol.kind == 6 && symbol.defined {
        let block = bindings
            .local_tls
            .ok_or(FormatError::InvalidReference("ELF TLS block", index))?;
        if symbol.address > block.bytes || symbol.bytes > block.bytes - symbol.address {
            return Err(FormatError::InvalidRange);
        }
        return Ok(Definition::Tls {
            block,
            offset: symbol.address,
        });
    }
    if symbol.defined && symbol.kind != 10 {
        return Ok(Definition::Address {
            address: symbol.address,
            bytes: symbol.bytes,
        });
    }
    if !symbol.defined && symbol.weak {
        return Ok(Definition::Address {
            address: 0,
            bytes: 0,
        });
    }
    Err(FormatError::InvalidReference(
        "ELF unresolved symbol",
        index,
    ))
}

pub fn bind(image: &mut Image, bindings: &Bindings<'_>, pass: Pass) -> Result<(), FormatError> {
    if image.target.encoding != Encoding::Elf
        || !matches!(image.target.bits, 32 | 64)
        || bindings.symbols.len() != image.symbols.len()
        || bindings.indirect.len() != image.relocations.len()
    {
        return Err(FormatError::InvalidRecordSize);
    }
    let wide = image.target.bits == 64;
    let bias = i128::from(
        image
            .base
            .checked_sub(image.preferred_base)
            .ok_or(FormatError::InvalidRange)?,
    );
    let got = image
        .dynamic
        .iter()
        .find(|(tag, _)| *tag == 3)
        .map(|(_, value)| i128::from(*value) + bias);
    let base = image.base;
    let length = image.bytes.len();
    for (ordinal, relocation) in image.relocations.iter().enumerate() {
        if relocation.kind == 0 || deferred(image, relocation) != (pass == Pass::Indirect) {
            continue;
        }
        let symbol =
            relocation
                .symbol
                .map(|index| {
                    image.symbols.get(index).map(|s| (index, s)).ok_or(
                        FormatError::InvalidReference("ELF relocation symbol", index),
                    )
                })
                .transpose()?;
        let mut memory =
            ModuleMemory::borrow(base, &mut image.bytes).map_err(|_| FormatError::InvalidRange)?;
        let addend = if relocation.kind == 5 {
            0
        } else if let Some(value) = relocation.addend {
            i128::from(value)
        } else {
            let bytes = memory
                .read(relocation.address, relocation.bytes)
                .map_err(|_| FormatError::InvalidRange)?;
            if !matches!(bytes.len(), 1 | 2 | 4 | 8) {
                return Err(FormatError::InvalidRecordSize);
            }
            let mut word = [if bytes[bytes.len() - 1] & 128 != 0 {
                255
            } else {
                0
            }; 8];
            word[..bytes.len()].copy_from_slice(bytes);
            i128::from(i64::from_le_bytes(word))
        };
        let kind = relocation.kind;
        let mut signed = false;
        let value = if kind == 8 || (wide && kind == 38) {
            bias + addend
        } else if kind == if wide { 37 } else { 42 } {
            i128::from(
                bindings.indirect[ordinal].ok_or(FormatError::InvalidReference(
                    "ELF indirect result",
                    ordinal,
                ))?,
            )
        } else {
            let value = definition(symbol, bindings)?;
            if kind == 5 {
                match value {
                    Definition::Copy { address, bytes } => {
                        if address >= base && address - base < length as u64 {
                            return Err(FormatError::InvalidRange);
                        }
                        let copied = bytes.len().min(relocation.bytes);
                        memory
                            .write(relocation.address, &bytes[..copied])
                            .map_err(|_| FormatError::InvalidRange)?;
                    }
                    Definition::Address { address: 0, .. }
                        if symbol.is_some_and(|(_, s)| s.weak) => {}
                    _ => return Err(FormatError::InvalidReference("ELF COPY provider", ordinal)),
                }
                continue;
            }
            let tls_kind = if wide {
                matches!(kind, 16 | 17 | 18 | 23)
            } else {
                matches!(kind, 14 | 17 | 34 | 35 | 36 | 37)
            };
            if tls_kind {
                let (block, offset) = if symbol.is_none() {
                    (
                        bindings
                            .local_tls
                            .ok_or(FormatError::InvalidReference("ELF TLS block", ordinal))?,
                        0,
                    )
                } else {
                    match value {
                        Definition::Tls { block, offset } => (block, offset),
                        _ => return Err(FormatError::InvalidReference("ELF TLS symbol", ordinal)),
                    }
                };
                if block.module == 0 || offset > block.bytes {
                    return Err(FormatError::InvalidRange);
                }
                if kind == if wide { 16 } else { 35 } {
                    i128::from(block.module)
                } else if kind == if wide { 17 } else { 36 } {
                    i128::from(offset) + if wide { addend } else { 0 }
                } else {
                    signed = true;
                    let displacement = i128::from(block.thread_pointer_offset) + i128::from(offset);
                    (if !wide && matches!(kind, 34 | 37) {
                        -displacement
                    } else {
                        displacement
                    }) + addend
                }
            } else {
                let (address, size) = match value {
                    Definition::Address { address, bytes } => (address, bytes),
                    Definition::Copy { address, bytes } => (address, bytes.len() as u64),
                    _ => return Err(FormatError::InvalidReference("ELF address symbol", ordinal)),
                };
                let address = i128::from(address);
                match (wide, kind) {
                    (_, 1) | (true, 10 | 11 | 12 | 14) => {
                        signed = kind == 11;
                        address + addend
                    }
                    (_, 2) | (true, 13 | 15 | 24) => {
                        signed = true;
                        address + addend - i128::from(relocation.address)
                    }
                    (_, 6 | 7) => address,
                    (false, 9) => {
                        address + addend
                            - got.ok_or(FormatError::InvalidReference("ELF GOT", ordinal))?
                    }
                    (false, 10) => {
                        got.ok_or(FormatError::InvalidReference("ELF GOT", ordinal))? + addend
                            - i128::from(relocation.address)
                    }
                    (true, 32 | 33) | (false, 38) => i128::from(size) + addend,
                    _ => {
                        return Err(FormatError::InvalidReference(
                            "ELF relocation type",
                            kind as usize,
                        ));
                    }
                }
            }
        };
        if !matches!(relocation.bytes, 1 | 2 | 4 | 8) {
            return Err(FormatError::InvalidRecordSize);
        }
        if wide && relocation.bytes < 8 {
            let bits = relocation.bytes * 8;
            let (minimum, maximum) = if signed {
                (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
            } else {
                (0, (1i128 << bits) - 1)
            };
            if value < minimum || value > maximum {
                return Err(FormatError::InvalidRange);
            }
        }
        memory
            .write(
                relocation.address,
                &(value as u64).to_le_bytes()[..relocation.bytes],
            )
            .map_err(|_| FormatError::InvalidRange)?;
    }
    Ok(())
}
