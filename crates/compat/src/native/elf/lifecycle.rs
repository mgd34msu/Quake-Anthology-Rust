//! Relocated ELF shared-object init/fini order, resolved only at load.
use crate::memory::{ModuleMemory, Width};
use qa_formats::{FormatError, program::native::Image};

pub struct Lifecycle {
    pub initialize: Vec<u64>,
    pub finalize: Vec<u64>,
}
impl Lifecycle {
    pub fn load(image: &mut Image, source_bytes: usize) -> Result<Self, FormatError> {
        let pointer_bytes = match image.target.bits {
            32 => 4,
            64 => 8,
            _ => return Err(FormatError::InvalidRecordSize),
        };
        let bias = image
            .base
            .checked_sub(image.preferred_base)
            .ok_or(FormatError::InvalidRange)?;
        let tag = |wanted| {
            image
                .dynamic
                .iter()
                .find(|&&(tag, _)| tag == wanted)
                .map(|&(_, value)| value)
        };
        let address = |value: u64| bias.checked_add(value).ok_or(FormatError::InvalidRange);
        let memory = ModuleMemory::borrow(image.base, &mut image.bytes[..source_bytes])
            .map_err(|_| FormatError::InvalidRange)?;
        let mut initialize = Vec::new();
        let mut finalize = Vec::new();
        if let Some(value) = tag(12).filter(|&value| value != 0) {
            initialize.push(address(value)?);
        }
        for (pointer, size, reverse, output) in [
            (25, 27, false, &mut initialize),
            (26, 28, true, &mut finalize),
        ] {
            match (tag(pointer), tag(size)) {
                (None, None) => {}
                (Some(at), Some(bytes)) => {
                    if bytes % pointer_bytes as u64 != 0 {
                        return Err(FormatError::InvalidRecordSize);
                    }
                    let bytes = usize::try_from(bytes).map_err(|_| FormatError::InvalidRange)?;
                    let at = address(at)?;
                    memory
                        .read(at, bytes)
                        .map_err(|_| FormatError::InvalidRange)?;
                    for offset in (0..bytes).step_by(pointer_bytes) {
                        let entry = at + offset as u64;
                        let value = if pointer_bytes == 4 {
                            memory.read_field(entry, Width::Word).map(u64::from)
                        } else {
                            memory.read_u64(entry)
                        };
                        output.push(value.map_err(|_| FormatError::InvalidRange)?);
                    }
                    if reverse {
                        output.reverse();
                    }
                }
                _ => return Err(FormatError::InvalidRecordSize),
            }
        }
        if let Some(value) = tag(13).filter(|&value| value != 0) {
            finalize.push(address(value)?);
        }
        // DT_PREINIT_ARRAY is ignored for shared objects, as required by gABI.
        Ok(Self {
            initialize,
            finalize,
        })
    }
}
