//! System V stdio: `_IO_FILE` layouts and standard streams.
//!
//! Donor: `src/guest/runtime/system-v/stdio.ts` (glibc 2.17
//! `libio/{libio.h,stdfiles.c,fileops.c}`).

use std::rc::Rc;

use crate::core::callbacks::HostCallContext;
use crate::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue, GuestStorage};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::runtime::common::memory::{count, integer, pointer, required_pointer};
use crate::runtime::system_v::contracts::{
    unsupported_system_v, SharedSystemV, SystemVServiceRegistrar, SystemVStream,
};
use crate::runtime::system_v::libc::system_v_allocate;

/// `_IO_FILE` field offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemVFileLayout {
    /// Structure size.
    pub size: usize,
    /// File descriptor offset.
    pub descriptor: i64,
    /// Orientation mode offset.
    pub mode: i64,
    /// Lock pointer offset.
    pub lock: i64,
    /// File offset field.
    pub offset: i64,
    /// Wide-data pointer offset.
    pub wide_data: i64,
    /// Old offset field.
    pub old_offset: i64,
}

/// `_IO_FILE` layout for a pointer width.
pub fn system_v_file_layout(pointer_bytes: usize) -> SystemVFileLayout {
    if pointer_bytes == 8 {
        SystemVFileLayout {
            size: 216,
            descriptor: 112,
            mode: 192,
            lock: 136,
            offset: 144,
            wide_data: 160,
            old_offset: 120,
        }
    } else {
        SystemVFileLayout {
            size: 148,
            descriptor: 56,
            mode: 104,
            lock: 72,
            offset: 76,
            wide_data: 88,
            old_offset: 64,
        }
    }
}

/// Constructed standard streams.
#[derive(Debug, Clone)]
pub struct SystemVStdio {
    /// Standard input `FILE`.
    pub stdin: GuestAddress,
    /// Standard output `FILE`.
    pub stdout: GuestAddress,
    /// Standard error `FILE`.
    pub stderr: GuestAddress,
    /// File layout.
    pub layout: SystemVFileLayout,
    /// Pointer width in bytes.
    pub pointer_bytes: usize,
    /// Thread `errno` slot.
    pub errno_address: GuestAddress,
}

impl SystemVStdio {
    fn field(
        &self,
        memory: &SparseGuestMemory,
        file: GuestAddress,
        pointer_index: i64,
    ) -> Result<GuestAddress, GuestError> {
        memory.offset(file, pointer_index * self.pointer_bytes as i64)
    }

    fn size_result(&self, value: usize) -> GuestCallResult {
        if self.pointer_bytes == 4 {
            GuestCallResult::Value(GuestCallValue::Uint32(value as u32))
        } else {
            GuestCallResult::Value(GuestCallValue::Uint64(value as u64))
        }
    }

    fn descriptor(
        &self,
        memory: &mut crate::core::memory::SparseGuestMemory,
        file: GuestAddress,
    ) -> Result<i32, GuestError> {
        if memory.read_u32(file)? & 0xffff_0000 != 0xfbad_0000 {
            return Err(GuestError::invalid("Invalid guest FILE magic"));
        }
        memory.read_i32(memory.offset(file, self.layout.descriptor)?)
    }

    fn orient(
        &self,
        memory: &mut crate::core::memory::SparseGuestMemory,
        file: GuestAddress,
        wide: bool,
    ) -> Result<bool, GuestError> {
        let mode = memory.offset(file, self.layout.mode)?;
        let desired = if wide { 1 } else { -1 };
        let current = memory.read_i32(mode)?;
        if current == 0 {
            memory.write_i32(mode, desired)?;
        }
        Ok(current == 0 || current == desired)
    }

    fn error(
        &self,
        memory: &mut crate::core::memory::SparseGuestMemory,
        file: GuestAddress,
        errno: i32,
    ) -> Result<i32, GuestError> {
        memory.write_i32(self.errno_address, errno)?;
        let flags = memory.read_u32(file)?;
        memory.write_u32(file, flags | 0x20)?;
        Ok(-1)
    }

    fn output_buffer(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        shared: &SharedSystemV,
        file: GuestAddress,
    ) -> Result<GuestAddress, GuestError> {
        let memory = ctx.memory();
        if let Some(existing) = memory.read_pointer(self.field(memory, file, 7)?)? {
            return Ok(existing);
        }
        let start = system_v_allocate(shared, memory, 8192)?;
        let end = memory.offset(start, 8192)?;
        for index in [4, 5, 7] {
            let slot = self.field(memory, file, index)?;
            memory.write_pointer(slot, Some(start))?;
        }
        for index in [6, 8] {
            let slot = self.field(memory, file, index)?;
            memory.write_pointer(slot, Some(end))?;
        }
        let output_terminal = shared.borrow().capabilities.output_is_terminal;
        if self.descriptor(memory, file)? == 1 && output_terminal {
            let flags = memory.read_u32(file)?;
            memory.write_u32(file, flags | 0x200)?;
        }
        Ok(start)
    }

    /// Flush buffered output; zero on success.
    pub fn flush(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        shared: &SharedSystemV,
        file: GuestAddress,
    ) -> Result<i32, GuestError> {
        let memory = ctx.memory();
        let descriptor = self.descriptor(memory, file)?;
        let base = memory.read_pointer(self.field(memory, file, 4)?)?;
        let next = memory.read_pointer(self.field(memory, file, 5)?)?;
        if descriptor == 0 {
            return Ok(0);
        }
        if descriptor != 1 && descriptor != 2 {
            return self.error(memory, file, 9);
        }
        if let (Some(base), Some(next)) = (base, next) {
            if next.offset > base.offset {
                let output = shared.borrow().capabilities.standard_output.clone();
                let Some(output) = output else {
                    return Err(unsupported_system_v(
                        "libc.so.6",
                        "fflush",
                        None,
                        "no standard output capability supplied",
                    ));
                };
                let length = (next.offset - base.offset) as usize;
                let bytes = memory.copy(base, length)?;
                let stream = if descriptor == 1 {
                    SystemVStream::Stdout
                } else {
                    SystemVStream::Stderr
                };
                let mut written = 0;
                while written < length {
                    let accepted = output(stream, &bytes[written..]);
                    if accepted == 0 || accepted > length - written {
                        memory.write(base, &bytes[written..])?;
                        let slot = self.field(memory, file, 5)?;
                        memory.write_pointer(slot, Some(memory.offset(base, (length - written) as i64)?))?;
                        return self.error(memory, file, 5);
                    }
                    written += accepted;
                }
                let slot = self.field(memory, file, 5)?;
                memory.write_pointer(slot, Some(base))?;
            }
        }
        let flush = shared.borrow().capabilities.standard_flush.clone();
        let stream = if descriptor == 1 {
            SystemVStream::Stdout
        } else {
            SystemVStream::Stderr
        };
        let result = flush.map(|flush| flush(stream)).unwrap_or(0);
        if result == 0 {
            Ok(0)
        } else {
            self.error(ctx.memory(), file, 5)
        }
    }

    fn put_byte(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        shared: &SharedSystemV,
        file: GuestAddress,
        value: u8,
    ) -> Result<i32, GuestError> {
        let memory = ctx.memory();
        if memory.read_u32(file)? & 8 != 0 {
            return self.error(memory, file, 9);
        }
        self.output_buffer(ctx, shared, file)?;
        let memory = ctx.memory();
        let mut next = memory.read_pointer(self.field(memory, file, 5)?)?;
        let end = memory.read_pointer(self.field(memory, file, 6)?)?;
        let (Some(next_value), Some(end)) = (next, end) else {
            return Err(GuestError::callback("Missing FILE output buffer"));
        };
        next = Some(next_value);
        if next_value.offset >= end.offset {
            if self.flush(ctx, shared, file)? != 0 {
                return Ok(-1);
            }
            let memory = ctx.memory();
            next = memory.read_pointer(self.field(memory, file, 5)?)?;
            if next.is_none() {
                return Err(GuestError::callback("Missing FILE write pointer"));
            }
        }
        let memory = ctx.memory();
        let next = next.expect("write pointer checked");
        memory.write_u8(next, value)?;
        let slot = self.field(memory, file, 5)?;
        memory.write_pointer(slot, Some(memory.offset(next, 1)?))?;
        let flags = memory.read_u32(file)?;
        if (flags & 2 != 0 || (flags & 0x200 != 0 && value == 10)) && self.flush(ctx, shared, file)? != 0 {
            return Ok(-1);
        }
        Ok(i32::from(value))
    }

    /// Write one character; returns it, or -1 on failure.
    pub fn put(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        shared: &SharedSystemV,
        file: GuestAddress,
        value: i32,
        wide: bool,
    ) -> Result<i32, GuestError> {
        self.descriptor(ctx.memory(), file)?;
        if !self.orient(ctx.memory(), file, wide)? {
            return Ok(-1);
        }
        if wide && (!(0..=127).contains(&value)) {
            return self.error(ctx.memory(), file, 84);
        }
        if self.put_byte(ctx, shared, file, (value & 255) as u8)? < 0 {
            return Ok(-1);
        }
        Ok(value)
    }

    /// Write bytes; returns the count accepted.
    pub fn write(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        shared: &SharedSystemV,
        file: GuestAddress,
        bytes: &[u8],
    ) -> Result<usize, GuestError> {
        self.descriptor(ctx.memory(), file)?;
        if !self.orient(ctx.memory(), file, false)? {
            return Ok(0);
        }
        let mut written = 0;
        for byte in bytes {
            if self.put_byte(ctx, shared, file, *byte)? < 0 {
                break;
            }
            written += 1;
        }
        Ok(written)
    }

    /// Read one character; -1 at end of input or on failure.
    pub fn get(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        shared: &SharedSystemV,
        file: GuestAddress,
        wide: bool,
    ) -> Result<i32, GuestError> {
        let memory = ctx.memory();
        if self.descriptor(memory, file)? != 0 || memory.read_u32(file)? & 4 != 0 {
            return self.error(memory, file, 9);
        }
        if !self.orient(memory, file, wide)? {
            return Ok(-1);
        }
        let mut next = memory.read_pointer(self.field(memory, file, 1)?)?;
        let mut end = memory.read_pointer(self.field(memory, file, 2)?)?;
        if next.is_none() || end.is_none() || next.expect("checked").offset >= end.expect("checked").offset {
            let input = shared.borrow().capabilities.standard_input.clone();
            let Some(input) = input else {
                return Err(unsupported_system_v(
                    "libc.so.6",
                    "fgetc",
                    None,
                    "no standard input capability supplied",
                ));
            };
            let bytes = input(8192);
            if bytes.len() > 8192 {
                return Err(GuestError::invalid("Input capability exceeded requested count"));
            }
            if bytes.is_empty() {
                let memory = ctx.memory();
                let flags = memory.read_u32(file)?;
                memory.write_u32(file, flags | 0x10)?;
                return Ok(-1);
            }
            let memory = ctx.memory();
            let buffer = match memory.read_pointer(self.field(memory, file, 7)?)? {
                Some(buffer) => buffer,
                None => system_v_allocate(shared, memory, 8192)?,
            };
            memory.write(buffer, &bytes)?;
            let slot = self.field(memory, file, 7)?;
            memory.write_pointer(slot, Some(buffer))?;
            let slot = self.field(memory, file, 8)?;
            memory.write_pointer(slot, Some(memory.offset(buffer, 8192)?))?;
            next = Some(buffer);
            end = Some(memory.offset(buffer, bytes.len() as i64)?);
            let slot = self.field(memory, file, 3)?;
            memory.write_pointer(slot, Some(buffer))?;
            let slot = self.field(memory, file, 2)?;
            memory.write_pointer(slot, end)?;
        }
        let memory = ctx.memory();
        let next = next.expect("input pointer checked");
        let byte = memory.read_u8(next)?;
        let slot = self.field(memory, file, 1)?;
        memory.write_pointer(slot, Some(memory.offset(next, 1)?))?;
        if wide && byte > 127 {
            return self.error(memory, file, 84);
        }
        Ok(i32::from(byte))
    }

    /// Push one character back; -1 when pushback is impossible.
    pub fn unget(
        &self,
        memory: &mut crate::core::memory::SparseGuestMemory,
        file: GuestAddress,
        value: i64,
        wide: bool,
    ) -> Result<i32, GuestError> {
        if value == -1 || value == 0xffff_ffff || !self.orient(memory, file, wide)? {
            return Ok(-1);
        }
        let next = memory.read_pointer(self.field(memory, file, 1)?)?;
        let base = memory.read_pointer(self.field(memory, file, 3)?)?;
        match (next, base) {
            (Some(next), Some(base)) if next.offset > base.offset => {
                let prior = memory.offset(next, -1)?;
                memory.write_u8(prior, (value & 255) as u8)?;
                let slot = self.field(memory, file, 1)?;
                memory.write_pointer(slot, Some(prior))?;
                let flags = memory.read_u32(file)?;
                memory.write_u32(file, flags & !0x10)?;
                Ok(value as i32)
            }
            _ => Ok(-1),
        }
    }
}

/// Build the standard streams and their services.
pub fn build_stdio(host: &mut SystemVServiceRegistrar<'_>) -> Result<SystemVStdio, GuestError> {
    let pointer_bytes = host.pointer_bytes;
    let word = pointer_bytes as i64;
    let layout = system_v_file_layout(pointer_bytes);
    let errno_address = host.errno_address;
    let stdio = SystemVStdio {
        stdin: create_file(host, &layout, 0, None)?,
        stdout: GuestAddress::new(0, 0),
        stderr: GuestAddress::new(0, 0),
        layout,
        pointer_bytes,
        errno_address,
    };
    let stdout = create_file(host, &layout, 1, Some(stdio.stdin))?;
    let stderr = create_file(host, &layout, 2, Some(stdout))?;
    let stdio = SystemVStdio {
        stdout,
        stderr,
        ..stdio
    };
    let version = if pointer_bytes == 4 { "GLIBC_2.0" } else { "GLIBC_2.2.5" };
    for (name, file) in [
        ("stdin", stdio.stdin),
        ("stdout", stdio.stdout),
        ("stderr", stdio.stderr),
    ] {
        let slot = host.allocate(pointer_bytes)?;
        host.memory.write_pointer(slot, Some(file))?;
        host.data("libc.so.6", name, Some(version), slot, pointer_bytes)?;
        host.data(
            "libc.so.6",
            &format!("_IO_2_1_{name}_"),
            Some(if pointer_bytes == 4 { "GLIBC_2.1" } else { version }),
            file,
            layout.size + pointer_bytes,
        )?;
    }
    {
        let stdio = stdio.clone();
        let shared = Rc::clone(&host.shared);
        host.service(
            "libc.so.6",
            "fflush",
            &[Some(version), None],
            &[GuestStorage::Pointer],
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, _, args| {
                let value = match pointer(args, 0)? {
                    Some(file) => stdio.flush(ctx, &shared, file)?,
                    None => {
                        let first = stdio.flush(ctx, &shared, stdio.stdout)?;
                        let second = stdio.flush(ctx, &shared, stdio.stderr)?;
                        if first == 0 && second == 0 {
                            0
                        } else {
                            -1
                        }
                    }
                };
                Ok(GuestCallResult::Value(GuestCallValue::Int32(value)))
            }),
        )?;
    }
    for name in ["fputc", "putc"] {
        let stdio = stdio.clone();
        let shared = Rc::clone(&host.shared);
        host.service(
            "libc.so.6",
            name,
            &[Some(version), None],
            &[GuestStorage::Int32, GuestStorage::Pointer],
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, _, args| {
                let value = stdio.put(
                    ctx,
                    &shared,
                    required_pointer(args, 1)?,
                    ((integer(args, 0)? & 255) as u8) as i32,
                    false,
                )?;
                Ok(GuestCallResult::Value(GuestCallValue::Int32(value)))
            }),
        )?;
    }
    for name in ["fgetc", "getc"] {
        let stdio = stdio.clone();
        let shared = Rc::clone(&host.shared);
        host.service(
            "libc.so.6",
            name,
            &[Some(version), None],
            &[GuestStorage::Pointer],
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, _, args| {
                let value = stdio.get(ctx, &shared, required_pointer(args, 0)?, false)?;
                Ok(GuestCallResult::Value(GuestCallValue::Int32(value)))
            }),
        )?;
    }
    {
        let stdio = stdio.clone();
        host.service(
            "libc.so.6",
            "ungetc",
            &[Some(version), None],
            &[GuestStorage::Int32, GuestStorage::Pointer],
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, _, args| {
                let value = stdio.unget(
                    ctx.memory(),
                    required_pointer(args, 1)?,
                    integer(args, 0)? as i64,
                    false,
                )?;
                Ok(GuestCallResult::Value(GuestCallValue::Int32(value)))
            }),
        )?;
    }
    {
        let stdio = stdio.clone();
        let shared = Rc::clone(&host.shared);
        let pointer_storage = host.pointer_storage();
        host.service(
            "libc.so.6",
            "fwrite",
            &[Some(version), None],
            &[
                GuestStorage::Pointer,
                pointer_storage,
                pointer_storage,
                GuestStorage::Pointer,
            ],
            Some(pointer_storage),
            Rc::new(move |ctx, _, args| {
                let size = count(args, 1)?;
                let total = size
                    .checked_mul(count(args, 2)?)
                    .filter(|total| *total <= 0x1000_0000)
                    .ok_or_else(|| GuestError::invalid("Guest fwrite extent exceeds runtime limit"))?;
                let memory = ctx.memory();
                let bytes = if total == 0 {
                    Vec::new()
                } else {
                    memory.copy(required_pointer(args, 0)?, total)?
                };
                let file = required_pointer(args, 3)?;
                let written = if total == 0 {
                    0
                } else {
                    stdio.write(ctx, &shared, file, &bytes)?
                };
                Ok(stdio.size_result(written.checked_div(size).unwrap_or(0)))
            }),
        )?;
    }
    {
        let stdio = stdio.clone();
        let shared = Rc::clone(&host.shared);
        let pointer_storage = host.pointer_storage();
        host.service(
            "libc.so.6",
            "fread",
            &[Some(version), None],
            &[
                GuestStorage::Pointer,
                pointer_storage,
                pointer_storage,
                GuestStorage::Pointer,
            ],
            Some(pointer_storage),
            Rc::new(move |ctx, _, args| {
                let size = count(args, 1)?;
                let total = size
                    .checked_mul(count(args, 2)?)
                    .filter(|total| *total <= 0x1000_0000)
                    .ok_or_else(|| GuestError::invalid("Guest fread extent exceeds runtime limit"))?;
                if total == 0 {
                    return Ok(stdio.size_result(0));
                }
                let destination = required_pointer(args, 0)?;
                let file = required_pointer(args, 3)?;
                let mut read = 0;
                while read < total {
                    let value = stdio.get(ctx, &shared, file, false)?;
                    if value < 0 {
                        break;
                    }
                    let memory = ctx.memory();
                    memory.write_u8(memory.offset(destination, read as i64)?, value as u8)?;
                    read += 1;
                }
                Ok(stdio.size_result(read / size))
            }),
        )?;
    }
    host.service(
        "libc.so.6",
        "fwide",
        &[Some(if pointer_bytes == 4 { "GLIBC_2.1" } else { version }), None],
        &[GuestStorage::Pointer, GuestStorage::Int32],
        Some(GuestStorage::Int32),
        Rc::new(move |ctx, _, args| {
            let memory = ctx.memory();
            let file = required_pointer(args, 0)?;
            let slot = memory.offset(file, layout.mode)?;
            if memory.read_i32(slot)? == 0 {
                memory.write_i32(slot, (integer(args, 1)? as i64).signum() as i32)?;
            }
            Ok(GuestCallResult::Value(GuestCallValue::Int32(memory.read_i32(slot)?)))
        }),
    )?;
    let table = host.allocate(21 * pointer_bytes)?;
    host.data(
        "libc.so.6",
        "_IO_file_jumps",
        Some(if pointer_bytes == 4 { "GLIBC_2.1" } else { version }),
        table,
        21 * pointer_bytes,
    )?;
    let operations = [
        "finish",
        "overflow",
        "underflow",
        "uflow",
        "pbackfail",
        "xsputn",
        "xsgetn",
        "seekoff",
        "seekpos",
        "setbuf",
        "sync",
        "doallocate",
        "read",
        "write",
        "seek",
        "close",
        "stat",
        "showmanyc",
        "imbue",
    ];
    for (index, operation) in operations.iter().enumerate() {
        let name = format!("__guest_IO_file_{operation}");
        let address = if *operation == "sync" {
            let stdio = stdio.clone();
            let shared = Rc::clone(&host.shared);
            host.service(
                "libc.so.6",
                &name,
                &[None],
                &[GuestStorage::Pointer],
                Some(GuestStorage::Int32),
                Rc::new(move |ctx, _, args| {
                    let value = stdio.flush(ctx, &shared, required_pointer(args, 0)?)?;
                    Ok(GuestCallResult::Value(GuestCallValue::Int32(value)))
                }),
            )?;
            host.resolve_address("libc.so.6", &name, None)
                .ok_or_else(|| GuestError::callback("Missing FILE sync"))?
        } else if *operation == "overflow" {
            let stdio = stdio.clone();
            let shared = Rc::clone(&host.shared);
            host.service(
                "libc.so.6",
                &name,
                &[None],
                &[GuestStorage::Pointer, GuestStorage::Int32],
                Some(GuestStorage::Int32),
                Rc::new(move |ctx, _, args| {
                    let file = required_pointer(args, 0)?;
                    let value = integer(args, 1)? as i64;
                    let result = if value == -1 {
                        if stdio.flush(ctx, &shared, file)? == 0 {
                            0
                        } else {
                            -1
                        }
                    } else {
                        stdio.put(ctx, &shared, file, (value & 255) as i32, false)?
                    };
                    Ok(GuestCallResult::Value(GuestCallValue::Int32(result)))
                }),
            )?;
            host.resolve_address("libc.so.6", &name, None)
                .ok_or_else(|| GuestError::callback("Missing FILE overflow"))?
        } else {
            host.unavailable(
                "libc.so.6",
                &name,
                &[GuestStorage::Pointer],
                None,
                "this FILE virtual operation is not implemented",
            )?
        };
        host.memory
            .write_pointer(host.memory.offset(table, (index as i64 + 2) * word)?, Some(address))?;
    }
    let wide_table = host.allocate(21 * pointer_bytes)?;
    host.data(
        "libc.so.6",
        "_IO_wfile_jumps",
        Some(if pointer_bytes == 4 { "GLIBC_2.1" } else { version }),
        wide_table,
        21 * pointer_bytes,
    )?;
    for (index, operation) in operations.iter().enumerate() {
        let address = if *operation == "sync" {
            host.memory
                .read_pointer(host.memory.offset(table, (index as i64 + 2) * word)?)?
        } else {
            Some(host.unavailable(
                "libc.so.6",
                &format!("__guest_IO_wfile_{operation}"),
                &[GuestStorage::Pointer],
                None,
                "this wide FILE virtual operation is not implemented",
            )?)
        };
        host.memory
            .write_pointer(host.memory.offset(wide_table, (index as i64 + 2) * word)?, address)?;
    }
    for file in [stdio.stdin, stdio.stdout, stdio.stderr] {
        host.memory
            .write_pointer(host.memory.offset(file, layout.size as i64)?, Some(table))?;
        let wide = host
            .memory
            .read_pointer(host.memory.offset(file, layout.wide_data)?)?
            .ok_or_else(|| GuestError::callback("Missing wide FILE data"))?;
        host.memory.write_pointer(
            host.memory.offset(wide, if pointer_bytes == 4 { 176 } else { 304 })?,
            Some(wide_table),
        )?;
    }
    Ok(stdio)
}

fn create_file(
    host: &mut SystemVServiceRegistrar<'_>,
    layout: &SystemVFileLayout,
    descriptor: i32,
    chain: Option<GuestAddress>,
) -> Result<GuestAddress, GuestError> {
    let word = host.pointer_bytes as i64;
    let file = host.allocate(layout.size + host.pointer_bytes)?;
    let flags = 0xfbad_0000u32 | 0x2080 | if descriptor == 0 { 8 } else { 4 } | if descriptor == 2 { 2 } else { 0 };
    host.memory.write_u32(file, flags)?;
    host.memory.write_pointer(host.memory.offset(file, 13 * word)?, chain)?;
    host.memory
        .write_i32(host.memory.offset(file, layout.descriptor)?, descriptor)?;
    host.memory.write_i64(host.memory.offset(file, layout.offset)?, -1)?;
    if host.pointer_bytes == 8 {
        host.memory
            .write_i64(host.memory.offset(file, layout.old_offset)?, -1)?;
    } else {
        host.memory
            .write_i32(host.memory.offset(file, layout.old_offset)?, -1)?;
    }
    let lock = host.allocate(word as usize * 2 + 8)?;
    host.memory
        .write_pointer(host.memory.offset(file, layout.lock)?, Some(lock))?;
    // _IO_wide_data starts with eleven wchar_t pointers and an mbstate_t.
    let wide = host.allocate(if host.pointer_bytes == 8 { 312 } else { 180 })?;
    host.memory
        .write_pointer(host.memory.offset(file, layout.wide_data)?, Some(wide))?;
    Ok(file)
}
