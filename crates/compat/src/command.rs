//! Native command C strings projected into the module's owned memory.
use crate::{memory::ModuleMemory, services::CallError};

const TOKENS: usize = 1024;
const TOKEN_BYTES: usize = qa_console::text::MAX_TEXT + TOKENS + 1;
const ARG_BYTES: usize = qa_console::text::MAX_TEXT + 1;

/// The shared console remains the tokenizer; this only supplies native pointers.
pub struct NativeCommand {
    address: u64,
    offsets: [u16; TOKENS],
    count: usize,
}
impl NativeCommand {
    pub const fn byte_length() -> usize {
        TOKEN_BYTES + ARG_BYTES
    }
    pub fn load(address: u64) -> Result<Self, CallError> {
        address
            .checked_add(Self::byte_length() as u64)
            .ok_or(CallError::Memory)?;
        Ok(Self {
            address,
            offsets: [0; TOKENS],
            count: 0,
        })
    }
    pub fn prepare(
        &mut self,
        memory: &mut ModuleMemory<'_>,
        tokens: &[&[u8]],
        raw_args: &[u8],
    ) -> Result<(), CallError> {
        if tokens.len() > TOKENS || raw_args.len() >= ARG_BYTES {
            return Err(CallError::Capacity);
        }
        if raw_args.contains(&0) || tokens.iter().any(|token| token.contains(&0)) {
            return Err(CallError::Text);
        }
        tokens
            .iter()
            .try_fold(1usize, |size, token| {
                size.checked_add(token.len()).and_then(|n| n.checked_add(1))
            })
            .filter(|&size| size <= TOKEN_BYTES)
            .ok_or(CallError::Capacity)?;
        let output = memory.read_mut(self.address, Self::byte_length())?;
        output[0] = 0;
        let mut at = 1;
        for (offset, token) in self.offsets.iter_mut().zip(tokens) {
            *offset = at as u16;
            output[at..at + token.len()].copy_from_slice(token);
            at += token.len();
            output[at] = 0;
            at += 1;
        }
        output[TOKEN_BYTES..TOKEN_BYTES + raw_args.len()].copy_from_slice(raw_args);
        output[TOKEN_BYTES + raw_args.len()] = 0;
        self.count = tokens.len();
        Ok(())
    }
    pub fn argv(&self, index: u32) -> u64 {
        self.address
            + if (index as usize) < self.count {
                u64::from(self.offsets[index as usize])
            } else {
                0
            }
    }
    pub fn args(&self) -> u64 {
        self.address + TOKEN_BYTES as u64
    }
}
