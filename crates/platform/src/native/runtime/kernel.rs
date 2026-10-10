//! Single-thread Windows state in the owned native child.
use super::{NativeError, Runtime};
use crate::native::{
    NativeAbi, NativeScalar,
    runtime::{Kernel, Slot, TIME_OFFSET, Time},
};

#[derive(Clone, Copy, Default)]
struct Fls {
    callback: u64,
    value: u64,
    allocated: bool,
}
pub(super) struct State {
    tls: [bool; 1088],
    fls: [Fls; 128],
    exception_filter: u64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            tls: [false; 1088],
            fls: [Fls::default(); 128],
            exception_filter: 0,
        }
    }
}
impl Runtime {
    fn teb(&self) -> Result<u64, NativeError> {
        self.config
            .and_then(|c| c.teb)
            .ok_or(NativeError::Unsupported)
    }
    fn last_error(&self, value: u32) -> Result<(), NativeError> {
        self.memory.put(self.teb()? + 0x68, 4, u64::from(value))
    }
    fn tls_slot(&self, slot: usize) -> Result<u64, NativeError> {
        let teb = self.teb()?;
        Ok(if slot < 64 {
            teb + 0x1480 + slot as u64 * 8
        } else {
            self.memory
                .unsigned(teb + 0x1780, 8)?
                .checked_add((slot - 64) as u64 * 8)
                .ok_or(NativeError::Extent)?
        })
    }
    pub(super) fn kernel(&self, operation: Kernel, a: [u64; 13]) -> Result<u64, NativeError> {
        let memory = &self.memory;
        match operation {
            Kernel::Time(kind) => {
                let value = match kind {
                    Time::Counter => memory.unsigned(self.state()? + TIME_OFFSET as u64, 8)?,
                    Time::Frequency => 1_000_000_000,
                    Time::File => (self.wall_millis()? as u64)
                        .wrapping_mul(10000)
                        .wrapping_add(116444736000000000),
                };
                memory.put(a[0], 8, value)?;
                Ok(1)
            }
            Kernel::Teb(offset, width) => memory.unsigned(self.teb()? + offset, width),
            Kernel::SetError => {
                self.last_error(a[0] as u32)?;
                Ok(0)
            }
            Kernel::CurrentProcess => Ok(u64::MAX),
            Kernel::Feature => Ok(u64::from(a[0] == 6 || a[0] == 10)),
            Kernel::Srw(acquire) => {
                let held = memory.unsigned(a[0], 8)? != 0;
                if held == acquire {
                    return Err(if acquire {
                        NativeError::Unsupported
                    } else {
                        NativeError::Extent
                    });
                }
                memory.put(a[0], 8, u64::from(acquire))?;
                Ok(0)
            }
            Kernel::SlistInit => {
                memory.fill(a[0], 0, 16)?;
                Ok(0)
            }
            Kernel::SlistFlush => {
                if a[0] % 16 != 0 {
                    return Err(NativeError::Extent);
                }
                let lower = memory.unsigned(a[0], 8)?;
                let upper = memory.unsigned(a[0] + 8, 8)?;
                let value = upper & !15;
                if value != 0 {
                    memory.put(a[0], 8, (lower & !65535).wrapping_add(65536))?;
                    memory.put(a[0] + 8, 8, upper & 15)?;
                }
                Ok(value)
            }
            Kernel::DisableThread => {
                let vector = self.teb()? + crate::native::runtime::STATIC_TLS_OFFSET as u64;
                if a[0] == memory.base && memory.unsigned(vector, 8)? == 0 {
                    Ok(1)
                } else {
                    self.last_error(87)?;
                    Ok(0)
                }
            }
            Kernel::ExceptionFilter => {
                let mut state = self.kernel.lock().map_err(|_| NativeError::Protocol)?;
                Ok(std::mem::replace(&mut state.exception_filter, a[0]))
            }
            Kernel::TlsAlloc => {
                let mut state = self.kernel.lock().map_err(|_| NativeError::Protocol)?;
                let Some(index) = state.tls.iter().position(|&used| !used) else {
                    return Ok(u64::from(u32::MAX));
                };
                memory.put(self.tls_slot(index)?, 8, 0)?;
                state.tls[index] = true;
                Ok(index as u64)
            }
            Kernel::Tls(action) => {
                let mut state = self.kernel.lock().map_err(|_| NativeError::Protocol)?;
                let index = a[0] as usize;
                if !state.tls.get(index).copied().unwrap_or(false) {
                    self.last_error(87)?;
                    return Ok(0);
                }
                match action {
                    Slot::Free => {
                        state.tls[index] = false;
                        Ok(1)
                    }
                    Slot::Set => {
                        memory.put(self.tls_slot(index)?, 8, a[1])?;
                        Ok(1)
                    }
                    Slot::Get => {
                        self.last_error(0)?;
                        memory.unsigned(self.tls_slot(index)?, 8)
                    }
                }
            }
            Kernel::FlsAlloc => {
                let mut state = self.kernel.lock().map_err(|_| NativeError::Protocol)?;
                let Some(index) = state.fls.iter().position(|slot| !slot.allocated) else {
                    self.last_error(8)?;
                    return Ok(u64::from(u32::MAX));
                };
                state.fls[index] = Fls {
                    callback: a[0],
                    value: 0,
                    allocated: true,
                };
                Ok(index as u64)
            }
            Kernel::Fls(action) => {
                let mut state = self.kernel.lock().map_err(|_| NativeError::Protocol)?;
                let Some(slot) = state
                    .fls
                    .get_mut(a[0] as usize)
                    .filter(|slot| slot.allocated)
                else {
                    self.last_error(87)?;
                    return Ok(0);
                };
                match action {
                    Slot::Get => {
                        self.last_error(0)?;
                        Ok(slot.value)
                    }
                    Slot::Set => {
                        slot.value = a[1];
                        Ok(1)
                    }
                    Slot::Free => {
                        let freed = std::mem::take(slot);
                        drop(state);
                        // End the metadata lock and clear the slot before the
                        // native destructor can reenter a runtime or engine import.
                        if freed.callback != 0 && freed.value != 0 {
                            self.foreign(
                                freed.callback,
                                NativeAbi::Microsoft,
                                &[NativeScalar::Word],
                                NativeScalar::Void,
                                &[freed.value],
                            )?;
                        }
                        Ok(1)
                    }
                }
            }
        }
    }
}
