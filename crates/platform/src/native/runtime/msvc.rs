//! x64 MSVC object semantics from windows_msvc.c. Only ABI layout is specific;
//! byte operations, allocations and native calls use the one child runtime.
use super::{NativeError, Runtime};
use crate::native::runtime::Msvc;
use crate::native::{
    NativeAbi,
    NativeScalar::{self, I32, Void, Word},
};

struct FlushLink<'a> {
    object: u64,
    parent: Option<&'a FlushLink<'a>>,
}
impl Runtime {
    fn reference(&self, slot: u64) -> Result<u64, NativeError> {
        let value = self.memory.unsigned(slot, 8)?;
        if value == 0 {
            Err(NativeError::Extent)
        } else {
            Ok(value)
        }
    }
    fn incref(&self, object: u64) -> Result<(), NativeError> {
        let refs = self.memory.unsigned(object + 8, 4)?;
        self.memory.put(object + 8, 4, refs + 1)
    }
    fn decref(&self, object: u64) -> Result<u64, NativeError> {
        let refs = self.memory.unsigned(object + 8, 4)?;
        if refs == 0 {
            return Err(NativeError::Extent);
        }
        self.memory.put(object + 8, 4, refs - 1)?;
        Ok(if refs == 1 { object } else { 0 })
    }
    fn locale_create(&self) -> Result<u64, NativeError> {
        let address = self.allocate(8)?;
        if address == 0 {
            return Err(NativeError::Extent);
        }
        let global = self.state()? + 96;
        self.incref(global)?;
        self.memory.put(address, 8, global)?;
        Ok(address)
    }
    fn locale_destroy(&self, address: u64) -> Result<(), NativeError> {
        if address == 0 {
            return Ok(());
        }
        let global = self.state()? + 96;
        if self.reference(address)? != global || self.decref(global)? != 0 {
            return Err(NativeError::Unsupported);
        }
        self.free(address)
    }
    fn locale_lock(&self, object: u64, mut kind: i32, unlock: bool) -> Result<(), NativeError> {
        let m = &self.memory;
        if unlock {
            kind = m.unsigned(object, 4)? as u32 as i32;
        } else {
            m.put(object, 4, kind as u64)?;
        }
        if !(0..8).contains(&kind) {
            return Ok(());
        }
        let slot = self.state()? + 32 + kind as u64 * 8;
        let thread = m.unsigned(slot, 4)?;
        let depth = m.unsigned(slot + 4, 4)?;
        if unlock {
            if thread != 1 || depth == 0 {
                return Err(NativeError::Extent);
            }
            m.put(slot + 4, 4, depth - 1)?;
            if depth == 1 {
                m.put(slot, 4, 0)?;
            }
        } else {
            if thread != 0 && thread != 1 {
                return Err(NativeError::Unsupported);
            }
            m.put(slot, 4, 1)?;
            m.put(slot + 4, 4, depth + 1)?;
        }
        Ok(())
    }
    fn virtual_call(
        &self,
        object: u64,
        slot: usize,
        types: &[NativeScalar],
        args: &[u64],
        result: NativeScalar,
    ) -> Result<u64, NativeError> {
        if types.len() != args.len() || types.len() > 5 {
            return Err(NativeError::Extent);
        }
        let table = self.reference(object)?;
        let target = self.reference(table + slot as u64 * 8)?;
        let mut kinds = [Word; 6];
        kinds[1..1 + types.len()].copy_from_slice(types);
        let mut words = [0; 6];
        words[0] = object;
        words[1..1 + args.len()].copy_from_slice(args);
        self.foreign(
            target,
            NativeAbi::Microsoft,
            &kinds[..1 + types.len()],
            result,
            &words[..1 + args.len()],
        )
    }
    fn virtual_ios(&self, object: u64) -> Result<u64, NativeError> {
        let table = self.reference(object)?;
        let offset = self.memory.unsigned(table + 4, 4)? as u32 as i32;
        Ok(object.wrapping_add(offset as i64 as u64))
    }
    fn setstate(&self, object: u64, state: u32) -> Result<(), NativeError> {
        let m = &self.memory;
        let previous = m.unsigned(object + 16, 4)? as u32;
        let buffer = m.unsigned(object + 72, 8)?;
        let value = (previous | state | if buffer == 0 { 4 } else { 0 }) & 0x17;
        m.put(object + 16, 4, u64::from(value))?;
        if u64::from(value) & m.unsigned(object + 20, 4)? != 0 {
            return Err(NativeError::Unsupported);
        }
        Ok(())
    }
    fn ios_construct(&self, object: u64) -> Result<(), NativeError> {
        self.memory.fill(object, 0, 96)?;
        self.memory.put(object, 8, self.state()? + 240)
    }
    fn ios_initialize(&self, object: u64, buffer: u64) -> Result<(), NativeError> {
        let m = &self.memory;
        for (offset, value) in [(8, 0), (32, 6), (40, 0), (48, 0), (56, 0)] {
            m.put(object + offset, 8, value)?;
        }
        m.put(object + 16, 4, if buffer == 0 { 4 } else { 0 })?;
        m.put(object + 20, 4, 0)?;
        m.put(object + 24, 4, 0x201)?;
        m.put(object + 64, 8, self.locale_create()?)?;
        m.put(object + 72, 8, buffer)?;
        m.put(object + 80, 8, 0)?;
        m.put(object + 88, 1, 32)
    }
    fn ios_destroy(&self, object: u64) -> Result<(), NativeError> {
        let m = &self.memory;
        m.put(object, 8, self.state()? + 240)?;
        if m.unsigned(object + 8, 8)? != 0 {
            return Err(NativeError::Unsupported);
        }
        let mut node = m.unsigned(object + 56, 8)?;
        while node != 0 {
            let callback = self.reference(node + 16)?;
            let index = m.unsigned(node + 8, 4)?;
            self.foreign(
                callback,
                NativeAbi::Microsoft,
                &[I32, Word, I32],
                Void,
                &[0, object, index],
            )?;
            node = m.unsigned(node, 8)?;
        }
        for offset in [48, 56] {
            let mut node = m.unsigned(object + offset, 8)?;
            while node != 0 {
                let next = m.unsigned(node, 8)?;
                self.free(node)?;
                node = next;
            }
            m.put(object + offset, 8, 0)?;
        }
        self.locale_destroy(m.unsigned(object + 64, 8)?)?;
        m.put(object + 64, 8, 0)
    }
    fn buffer_field(&self, object: u64, offset: u64) -> Result<u64, NativeError> {
        self.memory.unsigned(self.reference(object + offset)?, 8)
    }
    fn available(&self, object: u64, input: bool) -> Result<i32, NativeError> {
        if self.buffer_field(object, if input { 56 } else { 64 })? == 0 {
            return Ok(0);
        }
        Ok(self
            .memory
            .unsigned(self.reference(object + if input { 80 } else { 88 })?, 4)? as u32
            as i32)
    }
    fn bump(&self, object: u64, count: i32, input: bool) -> Result<u64, NativeError> {
        let slot = self.reference(object + if input { 56 } else { 64 })?;
        let address = self.reference(slot)?;
        let count_slot = self.reference(object + if input { 80 } else { 88 })?;
        let available = self.memory.unsigned(count_slot, 4)? as u32;
        self.memory
            .put(slot, 8, address.wrapping_add(count as i64 as u64))?;
        self.memory.put(
            count_slot,
            4,
            u64::from(available.wrapping_sub(count as u32)),
        )?;
        Ok(address)
    }
    fn buffer_get(&self, object: u64, consume: bool) -> Result<i32, NativeError> {
        if self.available(object, true)? > 0 {
            let address = if consume {
                self.bump(object, 1, true)?
            } else {
                self.buffer_field(object, 56)?
            };
            return Ok(i32::from(self.memory.read(address)?));
        }
        Ok(self.virtual_call(object, if consume { 7 } else { 6 }, &[], &[], I32)? as u32 as i32)
    }
    fn buffer_put(&self, object: u64, byte: u8) -> Result<i32, NativeError> {
        if self.available(object, false)? > 0 {
            let address = self.bump(object, 1, false)?;
            self.memory.put(address, 1, u64::from(byte))?;
            return Ok(i32::from(byte));
        }
        Ok(self.virtual_call(object, 3, &[I32], &[u64::from(byte)], I32)? as u32 as i32)
    }
    fn buffer_destroy(&self, object: u64) -> Result<(), NativeError> {
        self.memory.put(object, 8, self.state()? + 264)?;
        self.locale_destroy(self.memory.unsigned(object + 96, 8)?)?;
        self.memory.put(object + 96, 8, 0)
    }
    fn locked(
        &self,
        buffer: u64,
        body: impl FnOnce() -> Result<(), NativeError>,
    ) -> Result<(), NativeError> {
        if buffer != 0 {
            self.virtual_call(buffer, 1, &[], &[], Void)?;
        }
        let result = body();
        let unlocked = if buffer != 0 {
            self.virtual_call(buffer, 2, &[], &[], Void).map(|_| ())
        } else {
            Ok(())
        };
        result.and(unlocked)
    }
    fn suffix(&self, object: u64) -> Result<(), NativeError> {
        let base = self.virtual_ios(object)?;
        if self.memory.unsigned(base + 16, 4)? != 0 || self.memory.unsigned(base + 24, 4)? & 2 == 0
        {
            return Ok(());
        }
        let buffer = self.memory.unsigned(base + 72, 8)?;
        if buffer != 0 && self.virtual_call(buffer, 13, &[], &[], I32)? as u32 == u32::MAX {
            self.setstate(base, 4)?;
        }
        Ok(())
    }
    fn flush(&self, object: u64, parent: Option<&FlushLink<'_>>) -> Result<(), NativeError> {
        let mut at = parent;
        while let Some(link) = at {
            if link.object == object {
                return Err(NativeError::Extent);
            }
            at = link.parent;
        }
        let base = self.virtual_ios(object)?;
        let buffer = self.memory.unsigned(base + 72, 8)?;
        if buffer == 0 {
            return Ok(());
        }
        self.locked(buffer, || {
            if self.memory.unsigned(base + 16, 4)? == 0 {
                let tied = self.memory.unsigned(base + 80, 8)?;
                let link = FlushLink { object, parent };
                if tied != 0 && tied != object {
                    self.flush(tied, Some(&link))?;
                }
                if self.memory.unsigned(base + 16, 4)? == 0
                    && self.virtual_call(buffer, 13, &[], &[], I32)? as u32 == u32::MAX
                {
                    self.setstate(base, 4)?;
                }
            }
            self.suffix(object)
        })
    }
    fn insert_number(&self, object: u64, integer: u64, bits: u32) -> Result<(), NativeError> {
        let base = self.virtual_ios(object)?;
        let buffer = self.memory.unsigned(base + 72, 8)?;
        self.locked(buffer, || {
            if self.memory.unsigned(base + 16, 4)? == 0 && buffer != 0 {
                let tied = self.memory.unsigned(base + 80, 8)?;
                if tied != 0 && tied != object {
                    self.flush(tied, None)?;
                }
                if self.memory.unsigned(base + 16, 4)? == 0 {
                    let implementation = self.reference(self.reference(base + 64)?)?;
                    if implementation != self.state()? + 96
                        || self.memory.unsigned(implementation + 24, 8)? != 0
                    {
                        return Err(NativeError::Unsupported);
                    }
                    let flags = self.memory.unsigned(base + 24, 4)? as u32;
                    let width = self.memory.unsigned(base + 40, 8)? as i64;
                    let fill = self.memory.read(base + 88)?;
                    if width > 0x1000000 {
                        return Err(NativeError::Extent);
                    }
                    let radix = match flags & 0xe00 {
                        0x400 => 8,
                        0x800 => 16,
                        _ => 10,
                    };
                    let mask = if bits == 32 {
                        u32::MAX as u64
                    } else {
                        u64::MAX
                    };
                    let integer = integer & mask;
                    let negative = radix == 10 && integer & (1u64 << (bits - 1)) != 0;
                    let mut magnitude = if negative {
                        integer.wrapping_neg() & mask
                    } else {
                        integer
                    };
                    let alphabet = if flags & 4 != 0 {
                        b"0123456789ABCDEF"
                    } else {
                        b"0123456789abcdef"
                    };
                    let mut digits = [0; 65];
                    let mut count = 0;
                    loop {
                        digits[count] = alphabet[(magnitude % radix) as usize];
                        count += 1;
                        magnitude /= radix;
                        if magnitude == 0 {
                            break;
                        }
                    }
                    let mut prefix = [0; 2];
                    let mut prefix_count = 0;
                    if negative || radix == 10 && flags & 0x20 != 0 {
                        prefix[0] = if negative { b'-' } else { b'+' };
                        prefix_count = 1;
                    }
                    if radix != 10 && integer != 0 && flags & 8 != 0 {
                        prefix[0] = b'0';
                        prefix_count = 1;
                        if radix == 16 {
                            prefix[1] = if flags & 4 != 0 { b'X' } else { b'x' };
                            prefix_count = 2;
                        }
                    }
                    let padding =
                        width.max(0) as usize - (width.max(0) as usize).min(prefix_count + count);
                    let adjustment = flags & 0x1c0;
                    let total = count + prefix_count + padding;
                    for index in 0..total {
                        let prefix_at = if adjustment != 0x40 && adjustment != 0x100 {
                            padding
                        } else {
                            0
                        };
                        let digits_at = prefix_at
                            + prefix_count
                            + if adjustment == 0x100 { padding } else { 0 };
                        let byte = if (prefix_at..prefix_at + prefix_count).contains(&index) {
                            prefix[index - prefix_at]
                        } else if (digits_at..digits_at + count).contains(&index) {
                            digits[count - 1 - (index - digits_at)]
                        } else {
                            fill
                        };
                        if self.buffer_put(buffer, byte)? == -1 {
                            self.setstate(base, 4)?;
                            break;
                        }
                    }
                    self.memory.put(base + 40, 8, 0)?;
                }
            }
            self.setstate(base, 0)?;
            self.suffix(object)
        })
    }
    fn insert_buffer(&self, object: u64, source: u64) -> Result<(), NativeError> {
        let base = self.virtual_ios(object)?;
        let target = self.memory.unsigned(base + 72, 8)?;
        self.locked(target, || {
            let mut copied = false;
            let mut additional = 0;
            if self.memory.unsigned(base + 16, 4)? == 0 && target != 0 && source != 0 {
                let tied = self.memory.unsigned(base + 80, 8)?;
                if tied != 0 && tied != object {
                    self.flush(tied, None)?;
                }
                if self.memory.unsigned(base + 16, 4)? == 0 {
                    loop {
                        let character = self.buffer_get(source, false)?;
                        if character == -1 {
                            break;
                        }
                        if self.buffer_put(target, character as u8)? == -1 {
                            additional |= 4;
                            break;
                        }
                        self.buffer_get(source, true)?;
                        copied = true;
                    }
                }
            }
            self.memory.put(base + 40, 8, 0)?;
            self.setstate(
                base,
                if source == 0 {
                    4
                } else {
                    additional | if copied { 0 } else { 2 }
                },
            )?;
            self.suffix(object)
        })
    }
    fn stream_transfer(
        &self,
        object: u64,
        data: u64,
        requested: i64,
        input: bool,
    ) -> Result<u64, NativeError> {
        if object == 0 || data == 0 {
            return Err(NativeError::Extent);
        }
        if requested <= 0 {
            return Ok(0);
        }
        if requested > 0x10000000 {
            return Err(NativeError::Extent);
        }
        let mut copied = 0;
        while copied < requested as usize {
            let count =
                (self.available(object, input)?.max(0) as usize).min(requested as usize - copied);
            if count > 0 {
                let buffer = self.bump(object, count as i32, input)?;
                let (to, from) = if input {
                    (data + copied as u64, buffer)
                } else {
                    (buffer, data + copied as u64)
                };
                self.memory.copy(to, from, count)?;
                copied += count;
            } else if input {
                let byte = self.buffer_get(object, true)?;
                if byte == -1 {
                    break;
                }
                self.memory
                    .put(data + copied as u64, 1, u64::from(byte as u8))?;
                copied += 1;
            } else {
                if self.buffer_put(object, self.memory.read(data + copied as u64)?)? == -1 {
                    break;
                }
                copied += 1;
            }
        }
        Ok(copied as u64)
    }
    pub(super) fn msvc(&self, operation: Msvc, a: [u64; 13]) -> Result<u64, NativeError> {
        let object = a[0];
        let b = a[1];
        let m = &self.memory;
        let state = self.state()?;
        Ok(match operation {
            Msvc::Incref => {
                self.incref(object)?;
                0
            }
            Msvc::Decref => self.decref(object)?,
            Msvc::FacetDtor | Msvc::FacetDelete => {
                if matches!(operation, Msvc::FacetDelete) && object == state + 96 {
                    return Err(NativeError::Unsupported);
                }
                m.put(object, 8, state + 208)?;
                if matches!(operation, Msvc::FacetDelete) && b & 1 != 0 {
                    self.free(object)?;
                }
                object
            }
            Msvc::FacetCtor => {
                m.put(object, 8, state + 208)?;
                m.put(object + 8, 4, b)?;
                object
            }
            Msvc::LocaleInit => {
                if object & 255 != 0 {
                    self.incref(state + 96)?;
                }
                state + 96
            }
            Msvc::Global => state + 96,
            Msvc::Lock => {
                self.locale_lock(object, b as i32, false)?;
                object
            }
            Msvc::Unlock => {
                self.locale_lock(object, 0, true)?;
                0
            }
            Msvc::LocinfoCtor => {
                let length = m.length(b)?;
                if length != 0 && (length != 1 || m.read(b)? != b'C') {
                    return Err(NativeError::Unsupported);
                }
                m.fill(object, 0, 104)?;
                self.locale_lock(object, 0, false)?;
                for (offset, bytes, value) in [(72, 4, 67), (88, 2, 67)] {
                    let address = self.allocate(bytes)?;
                    if address == 0 {
                        return Err(NativeError::Extent);
                    }
                    m.put(address, bytes, value)?;
                    m.put(object + offset, 8, address)?;
                }
                object
            }
            Msvc::LocinfoDtor => {
                for offset in (8..=88).step_by(16) {
                    self.free(m.unsigned(object + offset, 8)?)?;
                    m.put(object + offset, 8, 0)?;
                }
                self.locale_lock(object, 0, true)?;
                0
            }
            Msvc::True => state + 164,
            Msvc::False => state + 172,
            Msvc::Lconv => state + 416,
            Msvc::Cvtvec => {
                m.fill(b, 0, 44)?;
                m.put(b + 4, 4, 1)?;
                m.put(b + 8, 4, 1)?;
                b
            }
            Msvc::IosDtor | Msvc::IosDelete => {
                self.ios_destroy(object)?;
                if matches!(operation, Msvc::IosDelete) && b & 1 != 0 {
                    self.free(object)?;
                }
                object
            }
            Msvc::IosCtor => {
                self.ios_construct(object)?;
                object
            }
            Msvc::Rdbuf => m.unsigned(object + 72, 8)?,
            Msvc::Setstate => {
                self.setstate(object, b as u32)?;
                0
            }
            Msvc::Good => u64::from(m.unsigned(object + 16, 4)? == 0),
            Msvc::OstreamCtor | Msvc::IostreamCtor => {
                let output = matches!(operation, Msvc::OstreamCtor);
                if output && a[2] != 0 {
                    return Err(NativeError::Unsupported);
                }
                let virtual_base = a[if output { 3 } else { 2 }] as i32;
                if virtual_base != 0 {
                    m.put(object, 8, state + if output { 384 } else { 392 })?;
                    if !output {
                        m.put(object + 16, 8, state + 400)?;
                    }
                    self.ios_construct(object + if output { 16 } else { 32 })?;
                }
                if !output {
                    m.put(object + 8, 8, 0)?;
                }
                let base = self.virtual_ios(object)?;
                m.put(base, 8, state + if output { 248 } else { 256 })?;
                m.put(
                    base - 4,
                    4,
                    base.wrapping_sub(object)
                        .wrapping_sub(if output { 16 } else { 32 }),
                )?;
                self.ios_initialize(base, b)?;
                object
            }
            Msvc::OstreamDtor | Msvc::IostreamDtor => {
                let output = matches!(operation, Msvc::OstreamDtor);
                let original = object
                    .checked_sub(if output { 16 } else { 32 })
                    .ok_or(NativeError::Extent)?;
                let base = self.virtual_ios(original)?;
                m.put(base, 8, state + if output { 248 } else { 256 })?;
                m.put(
                    base - 4,
                    4,
                    base.wrapping_sub(original)
                        .wrapping_sub(if output { 16 } else { 32 }),
                )?;
                0
            }
            Msvc::OstreamDelete => return Err(NativeError::Unsupported),
            Msvc::BufferDtor | Msvc::BufferDelete => {
                self.buffer_destroy(object)?;
                if matches!(operation, Msvc::BufferDelete) && b & 1 != 0 {
                    self.free(object)?;
                }
                object
            }
            Msvc::BufferLock | Msvc::Imbue | Msvc::Sync | Msvc::Showmany | Msvc::Uncaught => 0,
            Msvc::BufferOverflow | Msvc::BufferUnderflow => u32::MAX as u64,
            Msvc::Setbuf => object,
            Msvc::Eback | Msvc::Pbase | Msvc::Gptr | Msvc::Pptr => self.buffer_field(
                object,
                match operation {
                    Msvc::Eback => 24,
                    Msvc::Pbase => 32,
                    Msvc::Gptr => 56,
                    _ => 64,
                },
            )?,
            Msvc::Egptr | Msvc::Epptr => {
                let input = matches!(operation, Msvc::Egptr);
                let next = self.buffer_field(object, if input { 56 } else { 64 })?;
                if next == 0 {
                    0
                } else {
                    next.wrapping_add(self.available(object, input)? as i64 as u64)
                }
            }
            Msvc::Uflow => {
                if self.virtual_call(object, 6, &[], &[], I32)? as u32 == u32::MAX {
                    u32::MAX as u64
                } else {
                    u64::from(m.read(self.bump(object, 1, true)?)?)
                }
            }
            Msvc::Putc => self.buffer_put(object, b as u8)? as i64 as u64,
            Msvc::Getn | Msvc::Putn => {
                self.stream_transfer(object, b, a[2] as i64, matches!(operation, Msvc::Getn))?
            }
            Msvc::Sputn => self.virtual_call(object, 9, &[Word, Word], &[b, a[2]], Word)?,
            Msvc::Seekoff | Msvc::Seekpos => {
                m.fill(b, 0, 24)?;
                m.put(b + 8, 8, u64::MAX)?;
                b
            }
            Msvc::BufferCtor => {
                m.fill(object, 0, 104)?;
                m.put(object, 8, state + 264)?;
                for (slot, target) in [(24, 8), (32, 16), (56, 40), (64, 48), (80, 72), (88, 76)] {
                    m.put(object + slot, 8, object + target)?;
                }
                m.put(object + 96, 8, self.locale_create()?)?;
                object
            }
            Msvc::Flush => {
                self.flush(object, None)?;
                object
            }
            Msvc::Suffix => {
                self.suffix(object)?;
                0
            }
            Msvc::Tellp => {
                let base = self.virtual_ios(object)?;
                if m.unsigned(base + 16, 4)? & 6 != 0 {
                    m.fill(b, 0, 24)?;
                    m.put(b + 8, 8, u64::MAX)?;
                    b
                } else {
                    self.virtual_call(
                        self.reference(base + 72)?,
                        10,
                        &[Word, Word, I32, I32],
                        &[b, 0, 1, 2],
                        Word,
                    )?
                }
            }
            Msvc::Integer32 | Msvc::Integer64 => {
                self.insert_number(
                    object,
                    b,
                    if matches!(operation, Msvc::Integer32) {
                        32
                    } else {
                        64
                    },
                )?;
                object
            }
            Msvc::InsertBuffer => {
                self.insert_buffer(object, b)?;
                object
            }
        })
    }
}
