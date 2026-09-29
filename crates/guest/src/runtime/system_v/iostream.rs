//! System V iostreams: `ios_base::Init`, stream buffers, standard streams.
//!
//! Donor: `src/guest/runtime/system-v/iostream.ts` (libstdc++ 4.8
//! `ios_init.cc`, `ios.cc`, `basic_ios.tcc`, `stdio_sync_filebuf.h`).

use std::rc::Rc;

use crate::core::callbacks::HostCallContext;
use crate::core::contracts::{GuestAddress, GuestCallContext, GuestCallResult, GuestCallValue, GuestStorage};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::runtime::common::memory::{integer, required_pointer, write_unsigned};
use crate::runtime::system_v::contracts::{invoke_nested, unsupported_system_v, SystemVContext};
use crate::runtime::system_v::cxx_data::{
    abi_data, abi_function, abi_slot, abi_type, abi_unsupported, abi_vtable, CxxBase, SharedAbi,
};
use crate::runtime::system_v::locale::SystemVClassicLocale;
use crate::runtime::system_v::stdio::SystemVStdio;

/// `basic_ios` field offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemVIosLayout {
    /// Structure size.
    pub size: usize,
    /// Precision offset.
    pub precision: i64,
    /// Width offset.
    pub width: i64,
    /// Flags offset.
    pub flags: i64,
    /// Exceptions offset.
    pub exceptions: i64,
    /// State offset.
    pub state: i64,
    /// Callbacks offset.
    pub callbacks: i64,
    /// Local words offset.
    pub local_words: i64,
    /// Word size offset.
    pub word_size: i64,
    /// Words pointer offset.
    pub words: i64,
    /// Locale offset.
    pub locale: i64,
    /// Tie pointer offset.
    pub tie: i64,
    /// Fill offset.
    pub fill: i64,
    /// Fill-initialized offset.
    pub fill_initialized: i64,
    /// Buffer offset.
    pub buffer: i64,
    /// Ctype offset.
    pub ctype: i64,
    /// Num-put offset.
    pub num_put: i64,
    /// Num-get offset.
    pub num_get: i64,
}

/// `basic_ios` layout for a pointer width and character width.
pub fn system_v_ios_layout(pointer_bytes: usize, wide: bool) -> SystemVIosLayout {
    if pointer_bytes == 8 {
        SystemVIosLayout {
            size: 264,
            precision: 8,
            width: 16,
            flags: 24,
            exceptions: 28,
            state: 32,
            callbacks: 40,
            local_words: 64,
            word_size: 192,
            words: 200,
            locale: 208,
            tie: 216,
            fill: 224,
            fill_initialized: if wide { 228 } else { 225 },
            buffer: 232,
            ctype: 240,
            num_put: 248,
            num_get: 256,
        }
    } else {
        SystemVIosLayout {
            size: if wide { 140 } else { 136 },
            precision: 4,
            width: 8,
            flags: 12,
            exceptions: 16,
            state: 20,
            callbacks: 24,
            local_words: 36,
            word_size: 100,
            words: 104,
            locale: 108,
            tie: 112,
            fill: 116,
            fill_initialized: if wide { 120 } else { 117 },
            buffer: if wide { 124 } else { 120 },
            ctype: if wide { 128 } else { 124 },
            num_put: if wide { 132 } else { 128 },
            num_get: if wide { 136 } else { 132 },
        }
    }
}

/// One standard stream object.
#[derive(Debug, Clone)]
pub struct SystemVStandardStream {
    /// Stream name.
    pub name: String,
    /// Stream object address.
    pub address: GuestAddress,
    /// Embedded `ios` address.
    pub ios: GuestAddress,
    /// Wide-character stream.
    pub wide: bool,
    /// Input stream.
    pub input: bool,
}

/// Constructed standard streams with lazy `Init` construction.
#[derive(Debug, Clone)]
pub struct SystemVIostreams {
    /// Standard FILE streams.
    pub stdio: SystemVStdio,
    /// Standard stream objects.
    pub streams: Vec<SystemVStandardStream>,
    /// `Init` reference count.
    pub refcount: GuestAddress,
    /// `synced_with_stdio` flag.
    pub synchronized: GuestAddress,
    context: SystemVContext,
    abi: SharedAbi,
    locale: SystemVClassicLocale,
    pointer_bytes: usize,
}

impl SystemVIostreams {
    /// Installed classic locale.
    #[must_use]
    pub fn locale(&self) -> &super::locale::SystemVClassicLocale {
        &self.locale
    }

    /// Resolve the `ios` subobject through the stream vtable.
    fn ios(&self, memory: &mut SparseGuestMemory, stream: GuestAddress) -> Result<GuestAddress, GuestError> {
        let vptr = memory
            .read_pointer(stream)?
            .ok_or_else(|| GuestError::invalid("Unconstructed guest stream"))?;
        let slot = abi_slot(memory, vptr, -3 * self.pointer_bytes as i64)?;
        let offset = if self.pointer_bytes == 4 {
            i64::from(memory.read_i32(slot)?)
        } else {
            memory.read_i64(slot)?
        };
        memory.offset(stream, offset)
    }

    /// Assign stream state, raising on masked exceptions.
    pub fn clear(
        &self,
        memory: &mut SparseGuestMemory,
        ios: GuestAddress,
        wide: bool,
        mut state: u32,
    ) -> Result<(), GuestError> {
        let layout = system_v_ios_layout(self.pointer_bytes, wide);
        if memory.read_pointer(abi_slot(memory, ios, layout.buffer)?)?.is_none() {
            state |= 1;
        }
        memory.write_u32(abi_slot(memory, ios, layout.state)?, state)?;
        if state & memory.read_u32(abi_slot(memory, ios, layout.exceptions)?)? != 0 {
            return Err(unsupported_system_v(
                "libstdc++.so.6",
                "__throw_ios_failure",
                Some("GLIBCXX_3.4"),
                "guest iostream exception propagation is not implemented",
            ));
        }
        Ok(())
    }

    /// Flush a stream through its buffer's `sync` method.
    pub fn flush(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        context: &GuestCallContext,
        stream: GuestAddress,
        wide: bool,
    ) -> Result<(), GuestError> {
        let layout = system_v_ios_layout(self.pointer_bytes, wide);
        let pointer = self.pointer_bytes as i64;
        let memory = ctx.memory();
        let ios = self.ios(memory, stream)?;
        if memory.read_u32(abi_slot(memory, ios, layout.state)?)? != 0 {
            return Ok(());
        }
        let tied = memory.read_pointer(abi_slot(memory, ios, layout.tie)?)?;
        if let Some(tied) = tied {
            self.flush(ctx, context, tied, wide)?;
        }
        let memory = ctx.memory();
        let buffer = memory.read_pointer(abi_slot(memory, ios, layout.buffer)?)?;
        let Some(buffer) = buffer else {
            return Ok(());
        };
        let vptr = memory
            .read_pointer(buffer)?
            .ok_or_else(|| GuestError::callback("Stream buffer has no vtable"))?;
        let sync = memory
            .read_pointer(abi_slot(memory, vptr, 6 * pointer)?)?
            .ok_or_else(|| GuestError::callback("Stream buffer has no sync method"))?;
        let synchronize = |streams: &Self, ctx: &mut HostCallContext<'_, '_>| -> Result<(), GuestError> {
            let result = invoke_nested(
                ctx,
                &streams.context.shared,
                streams.pointer_bytes,
                context,
                sync,
                &[GuestStorage::Pointer],
                Some(GuestStorage::Int32),
                vec![GuestCallValue::Pointer(Some(buffer))],
            )?;
            let GuestCallResult::Value(GuestCallValue::Int32(value)) = result else {
                return Err(GuestError::invalid("Stream buffer sync returned wrong type"));
            };
            if value == -1 {
                let memory = ctx.memory();
                let state = memory.read_u32(abi_slot(memory, ios, layout.state)?)? | 1;
                streams.clear(memory, ios, wide, state)?;
            }
            Ok(())
        };
        synchronize(self, ctx)?;
        let memory = ctx.memory();
        if memory.read_u32(abi_slot(memory, ios, layout.flags)?)? & 0x2000 != 0 {
            synchronize(self, ctx)?;
        }
        Ok(())
    }

    /// Run `ios_base::Init` construction once.
    fn initialize(&self, memory: &mut SparseGuestMemory) -> Result<(), GuestError> {
        let previous = memory.read_i32(self.refcount)?;
        memory.write_i32(self.refcount, previous + 1)?;
        if previous != 0 {
            return Ok(());
        }
        memory.write_u8(self.synchronized, 1)?;
        for wide in [false, true] {
            let input = self.stream_buffer(memory, self.stdio.stdin, wide)?;
            let output = self.stream_buffer(memory, self.stdio.stdout, wide)?;
            let error = self.stream_buffer(memory, self.stdio.stderr, wide)?;
            for stream in self.streams.iter().filter(|stream| stream.wide == wide) {
                let buffer = if stream.input {
                    input
                } else if stream.name.ends_with("cout") {
                    output
                } else {
                    error
                };
                self.construct_stream(memory, stream, buffer)?;
            }
            let output = self
                .streams
                .iter()
                .find(|stream| stream.wide == wide && stream.name.ends_with("cout"))
                .ok_or_else(|| GuestError::callback("Missing standard output stream"))?;
            for stream in self
                .streams
                .iter()
                .filter(|stream| stream.wide == wide && (stream.input || stream.name.ends_with("cerr")))
            {
                let layout = system_v_ios_layout(self.pointer_bytes, wide);
                memory.write_pointer(abi_slot(memory, stream.ios, layout.tie)?, Some(output.address))?;
                if !stream.input {
                    memory.write_u32(abi_slot(memory, stream.ios, layout.flags)?, 0x3002)?;
                }
            }
        }
        let count = memory.read_i32(self.refcount)?;
        memory.write_i32(self.refcount, count + 1)?;
        Ok(())
    }

    /// Construct one standard stream object over its buffer.
    fn construct_stream(
        &self,
        memory: &mut SparseGuestMemory,
        stream: &SystemVStandardStream,
        buffer: GuestAddress,
    ) -> Result<(), GuestError> {
        let ctx = &self.context;
        let pointer = self.pointer_bytes as i64;
        let layout = system_v_ios_layout(self.pointer_bytes, stream.wide);
        let ios = stream.ios;
        let char = if stream.wide { "w" } else { "c" };
        let ios_base = abi_type(ctx, memory, &self.abi, "St8ios_base", &[])?;
        let basic_ios = abi_type(
            ctx,
            memory,
            &self.abi,
            &format!("St9basic_iosI{char}St11char_traitsI{char}EE"),
            &[CxxBase {
                address: ios_base,
                offset: 0,
                flags: 2,
            }],
        )?;
        let name = if stream.wide {
            format!(
                "St13basic_{}IwSt11char_traitsIwEE",
                if stream.input { "istream" } else { "ostream" }
            )
        } else if stream.input {
            "Si".to_string()
        } else {
            "So".to_string()
        };
        let info = abi_type(
            ctx,
            memory,
            &self.abi,
            &name,
            &[CxxBase {
                address: basic_ios,
                offset: -3 * pointer,
                flags: 3,
            }],
        )?;
        let prefix = (if stream.input { 2 } else { 1 }) * pointer;
        // Primary address point: virtual-base offset, offset-to-top, RTTI,
        // then two destructors.
        let table = match ctx.resolve_address("libstdc++.so.6", &format!("_ZTV{name}"), Some("GLIBCXX_3.4")) {
            Some(table) => table,
            None => {
                let table = abi_data(
                    ctx,
                    memory,
                    &format!("_ZTV{name}"),
                    10 * pointer as usize,
                    "GLIBCXX_3.4",
                )?;
                write_unsigned(memory, table, self.pointer_bytes, prefix as i128)?;
                memory.write_pointer(abi_slot(memory, table, 2 * pointer)?, Some(info))?;
                let destructor = abi_unsupported(ctx, memory, &format!("__guest_{name}_destructor"))?;
                memory.write_pointer(abi_slot(memory, table, 3 * pointer)?, Some(destructor))?;
                let deleting = abi_unsupported(ctx, memory, &format!("__guest_{name}_deleting_destructor"))?;
                memory.write_pointer(abi_slot(memory, table, 4 * pointer)?, Some(deleting))?;
                write_unsigned(
                    memory,
                    abi_slot(memory, table, 5 * pointer)?,
                    self.pointer_bytes,
                    -(prefix as i128),
                )?;
                write_unsigned(
                    memory,
                    abi_slot(memory, table, 6 * pointer)?,
                    self.pointer_bytes,
                    -(prefix as i128),
                )?;
                memory.write_pointer(abi_slot(memory, table, 7 * pointer)?, Some(info))?;
                let virtual_destructor = abi_unsupported(ctx, memory, &format!("__guest_{name}_virtual_destructor"))?;
                memory.write_pointer(abi_slot(memory, table, 8 * pointer)?, Some(virtual_destructor))?;
                let virtual_deleting =
                    abi_unsupported(ctx, memory, &format!("__guest_{name}_virtual_deleting_destructor"))?;
                memory.write_pointer(abi_slot(memory, table, 9 * pointer)?, Some(virtual_deleting))?;
                table
            }
        };
        memory.write_pointer(stream.address, Some(abi_slot(memory, table, 3 * pointer)?))?;
        memory.write_pointer(ios, Some(abi_slot(memory, table, 8 * pointer)?))?;
        write_unsigned(memory, abi_slot(memory, ios, layout.precision)?, self.pointer_bytes, 6)?;
        memory.write_u32(abi_slot(memory, ios, layout.flags)?, 0x1002)?;
        memory.write_i32(abi_slot(memory, ios, layout.word_size)?, 8)?;
        memory.write_pointer(
            abi_slot(memory, ios, layout.words)?,
            Some(abi_slot(memory, ios, layout.local_words)?),
        )?;
        let retained = self.locale.retain(memory)?;
        memory.write_pointer(abi_slot(memory, ios, layout.locale)?, Some(retained))?;
        memory.write_pointer(abi_slot(memory, ios, layout.buffer)?, Some(buffer))?;
        let index = usize::from(stream.wide);
        memory.write_pointer(abi_slot(memory, ios, layout.ctype)?, Some(self.locale.ctype[index]))?;
        memory.write_pointer(abi_slot(memory, ios, layout.num_put)?, Some(self.locale.num_put[index]))?;
        memory.write_pointer(abi_slot(memory, ios, layout.num_get)?, Some(self.locale.num_get[index]))?;
        Ok(())
    }

    /// Build a `stdio_sync_filebuf` over `file`.
    fn stream_buffer(
        &self,
        memory: &mut SparseGuestMemory,
        file: GuestAddress,
        wide: bool,
    ) -> Result<GuestAddress, GuestError> {
        let ctx = &self.context;
        let pointer = self.pointer_bytes as i64;
        let char = if wide { "w" } else { "c" };
        let name = format!("N9__gnu_cxx18stdio_sync_filebufI{char}St11char_traitsI{char}EEE");
        let address = ctx.allocate(memory, 10 * pointer as usize)?;
        let base_info = abi_type(
            ctx,
            memory,
            &self.abi,
            &format!("St15basic_streambufI{char}St11char_traitsI{char}EE"),
            &[],
        )?;
        let info = abi_type(
            ctx,
            memory,
            &self.abi,
            &name,
            &[CxxBase {
                address: base_info,
                offset: 0,
                flags: 2,
            }],
        )?;
        let table = match ctx.resolve_address("libstdc++.so.6", &format!("_ZTV{name}"), Some("GLIBCXX_3.4")) {
            Some(table) => table,
            None => {
                let signed = ctx.signed_pointer_storage();
                let character = if wide {
                    GuestStorage::Uint32
                } else {
                    GuestStorage::Int32
                };
                let stdio = self.stdio.clone();
                let shared = Rc::clone(&ctx.shared);
                let pointer_bytes = self.pointer_bytes;
                let entries = vec![
                    abi_unsupported(ctx, memory, &format!("__guest_{name}_destructor"))?,
                    abi_unsupported(ctx, memory, &format!("__guest_{name}_deleting_destructor"))?,
                    // These two inherited basic_streambuf virtual methods
                    // intentionally leave its state unchanged.
                    abi_function(
                        ctx,
                        memory,
                        &format!("__guest_{name}_imbue"),
                        &[GuestStorage::Pointer, GuestStorage::Pointer],
                        None,
                        Rc::new(|_, _, _| Ok(GuestCallResult::Void)),
                        "GLIBCXX_3.4",
                    )?,
                    abi_function(
                        ctx,
                        memory,
                        &format!("__guest_{name}_setbuf"),
                        &[GuestStorage::Pointer, GuestStorage::Pointer, signed],
                        Some(GuestStorage::Pointer),
                        Rc::new(|_, _, args| {
                            Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(required_pointer(
                                args, 0,
                            )?))))
                        }),
                        "GLIBCXX_3.4",
                    )?,
                    abi_unsupported(ctx, memory, &format!("__guest_{name}_seekoff"))?,
                    abi_unsupported(ctx, memory, &format!("__guest_{name}_seekpos"))?,
                    {
                        let stdio = stdio.clone();
                        let shared = Rc::clone(&shared);
                        abi_function(
                            ctx,
                            memory,
                            &format!("__guest_{name}_sync"),
                            &[GuestStorage::Pointer],
                            Some(GuestStorage::Int32),
                            Rc::new(move |ctx, _, args| {
                                let buffer = required_pointer(args, 0)?;
                                let file = stream_file(ctx.memory(), buffer, pointer_bytes)?;
                                let value = stdio.flush(ctx, &shared, file)?;
                                Ok(GuestCallResult::Value(GuestCallValue::Int32(value)))
                            }),
                            "GLIBCXX_3.4",
                        )?
                    },
                    abi_function(
                        ctx,
                        memory,
                        &format!("__guest_{name}_showmanyc"),
                        &[GuestStorage::Pointer],
                        Some(signed),
                        Rc::new(move |_, _, _| {
                            Ok(if pointer_bytes == 4 {
                                GuestCallResult::Value(GuestCallValue::Int32(0))
                            } else {
                                GuestCallResult::Value(GuestCallValue::Int64(0))
                            })
                        }),
                        "GLIBCXX_3.4",
                    )?,
                    {
                        let stdio = stdio.clone();
                        let shared = Rc::clone(&shared);
                        abi_function(
                            ctx,
                            memory,
                            &format!("__guest_{name}_xsgetn"),
                            &[GuestStorage::Pointer, GuestStorage::Pointer, signed],
                            Some(signed),
                            Rc::new(move |ctx, _, args| {
                                let buffer = required_pointer(args, 0)?;
                                let destination = required_pointer(args, 1)?;
                                let count = integer(args, 2)?.max(0).min(usize::MAX as i128) as usize;
                                let mut read = 0;
                                while read < count {
                                    let file = stream_file(ctx.memory(), buffer, pointer_bytes)?;
                                    let value = stdio.get(ctx, &shared, file, wide)?;
                                    if value < 0 {
                                        break;
                                    }
                                    let memory = ctx.memory();
                                    if wide {
                                        memory.write_u32(memory.offset(destination, read as i64 * 4)?, value as u32)?;
                                    } else {
                                        memory.write_u8(memory.offset(destination, read as i64)?, value as u8)?;
                                    }
                                    read += 1;
                                }
                                let memory = ctx.memory();
                                let last = if read > 0 {
                                    if wide {
                                        memory.read_u32(memory.offset(destination, (read - 1) as i64 * 4)?)?
                                    } else {
                                        u32::from(memory.read_u8(memory.offset(destination, (read - 1) as i64)?)?)
                                    }
                                } else {
                                    0xffff_ffff
                                };
                                memory.write_u32(memory.offset(buffer, 9 * pointer)?, last)?;
                                Ok(if pointer_bytes == 4 {
                                    GuestCallResult::Value(GuestCallValue::Int32(read as i32))
                                } else {
                                    GuestCallResult::Value(GuestCallValue::Int64(read as i64))
                                })
                            }),
                            "GLIBCXX_3.4",
                        )?
                    },
                    {
                        let stdio = stdio.clone();
                        let shared = Rc::clone(&shared);
                        abi_function(
                            ctx,
                            memory,
                            &format!("__guest_{name}_underflow"),
                            &[GuestStorage::Pointer],
                            Some(character),
                            Rc::new(move |ctx, _, args| {
                                let buffer = required_pointer(args, 0)?;
                                let file = stream_file(ctx.memory(), buffer, pointer_bytes)?;
                                let value = stdio.get(ctx, &shared, file, wide)?;
                                let returned = stdio.unget(ctx.memory(), file, i64::from(value), wide)?;
                                Ok(character_result(character, returned))
                            }),
                            "GLIBCXX_3.4",
                        )?
                    },
                    {
                        let stdio = stdio.clone();
                        let shared = Rc::clone(&shared);
                        abi_function(
                            ctx,
                            memory,
                            &format!("__guest_{name}_uflow"),
                            &[GuestStorage::Pointer],
                            Some(character),
                            Rc::new(move |ctx, _, args| {
                                let buffer = required_pointer(args, 0)?;
                                let file = stream_file(ctx.memory(), buffer, pointer_bytes)?;
                                let value = stdio.get(ctx, &shared, file, wide)?;
                                let slot = ctx.memory().offset(buffer, 9 * pointer)?;
                                ctx.memory().write_u32(slot, value as u32)?;
                                Ok(character_result(character, value))
                            }),
                            "GLIBCXX_3.4",
                        )?
                    },
                    {
                        let stdio = stdio.clone();
                        abi_function(
                            ctx,
                            memory,
                            &format!("__guest_{name}_pbackfail"),
                            &[GuestStorage::Pointer, character],
                            Some(character),
                            Rc::new(move |ctx, _, args| {
                                let buffer = required_pointer(args, 0)?;
                                let supplied = integer(args, 1)? as i64;
                                let memory = ctx.memory();
                                let value = if supplied == -1 || supplied == 0xffff_ffff {
                                    i64::from(memory.read_i32(memory.offset(buffer, 9 * pointer)?)?)
                                } else {
                                    supplied
                                };
                                let file = stream_file(memory, buffer, pointer_bytes)?;
                                let returned = stdio.unget(memory, file, value, wide)?;
                                memory.write_i32(memory.offset(buffer, 9 * pointer)?, -1)?;
                                Ok(character_result(character, returned))
                            }),
                            "GLIBCXX_3.4",
                        )?
                    },
                    {
                        let stdio = stdio.clone();
                        let shared = Rc::clone(&shared);
                        abi_function(
                            ctx,
                            memory,
                            &format!("__guest_{name}_xsputn"),
                            &[GuestStorage::Pointer, GuestStorage::Pointer, signed],
                            Some(signed),
                            Rc::new(move |ctx, _, args| {
                                let buffer = required_pointer(args, 0)?;
                                let source = required_pointer(args, 1)?;
                                let count = integer(args, 2)?.max(0).min(usize::MAX as i128) as usize;
                                let file = stream_file(ctx.memory(), buffer, pointer_bytes)?;
                                let mut written = 0;
                                while written < count {
                                    let memory = ctx.memory();
                                    let value = if wide {
                                        memory.read_u32(memory.offset(source, written as i64 * 4)?)? as i32
                                    } else {
                                        i32::from(memory.read_u8(memory.offset(source, written as i64)?)?)
                                    };
                                    if stdio.put(ctx, &shared, file, value, wide)? < 0 {
                                        break;
                                    }
                                    written += 1;
                                }
                                Ok(if pointer_bytes == 4 {
                                    GuestCallResult::Value(GuestCallValue::Int32(written as i32))
                                } else {
                                    GuestCallResult::Value(GuestCallValue::Int64(written as i64))
                                })
                            }),
                            "GLIBCXX_3.4",
                        )?
                    },
                    {
                        let stdio = stdio.clone();
                        let shared = Rc::clone(&shared);
                        abi_function(
                            ctx,
                            memory,
                            &format!("__guest_{name}_overflow"),
                            &[GuestStorage::Pointer, character],
                            Some(character),
                            Rc::new(move |ctx, _, args| {
                                let buffer = required_pointer(args, 0)?;
                                let value = integer(args, 1)? as i64;
                                let file = stream_file(ctx.memory(), buffer, pointer_bytes)?;
                                let result = if value == -1 || value == 0xffff_ffff {
                                    if stdio.flush(ctx, &shared, file)? == 0 {
                                        0
                                    } else {
                                        -1
                                    }
                                } else {
                                    stdio.put(ctx, &shared, file, value as i32, wide)?
                                };
                                Ok(character_result(character, result))
                            }),
                            "GLIBCXX_3.4",
                        )?
                    },
                ];
                abi_vtable(ctx, memory, &name, info, &entries)?;
                ctx.resolve_address("libstdc++.so.6", &format!("_ZTV{name}"), Some("GLIBCXX_3.4"))
                    .ok_or_else(|| GuestError::callback("Missing constructed streambuf vtable"))?
            }
        };
        memory.write_pointer(address, Some(abi_slot(memory, table, 2 * pointer)?))?;
        let retained = self.locale.retain(memory)?;
        memory.write_pointer(abi_slot(memory, address, 7 * pointer)?, Some(retained))?;
        memory.write_pointer(abi_slot(memory, address, 8 * pointer)?, Some(file))?;
        memory.write_i32(abi_slot(memory, address, 9 * pointer)?, -1)?;
        Ok(address)
    }
}

fn character_result(storage: GuestStorage, value: i32) -> GuestCallResult {
    if storage == GuestStorage::Uint32 {
        GuestCallResult::Value(GuestCallValue::Uint32(value as u32))
    } else {
        GuestCallResult::Value(GuestCallValue::Int32(value))
    }
}

fn stream_file(
    memory: &mut SparseGuestMemory,
    buffer: GuestAddress,
    pointer_bytes: usize,
) -> Result<GuestAddress, GuestError> {
    memory
        .read_pointer(memory.offset(buffer, 8 * pointer_bytes as i64)?)?
        .ok_or_else(|| GuestError::callback("Missing synchronized FILE"))
}

/// Build the standard streams and their services.
pub fn build_iostreams(
    ctx: &SystemVContext,
    memory: &mut SparseGuestMemory,
    abi: &SharedAbi,
    stdio: SystemVStdio,
    locale: SystemVClassicLocale,
) -> Result<SystemVIostreams, GuestError> {
    let pointer_bytes = ctx.pointer_bytes;
    let pointer = pointer_bytes as i64;
    let refcount = abi_data(ctx, memory, "_ZNSt8ios_base4Init11_S_refcountE", 4, "GLIBCXX_3.4")?;
    let synchronized = abi_data(
        ctx,
        memory,
        "_ZNSt8ios_base4Init20_S_synced_with_stdioE",
        1,
        "GLIBCXX_3.4",
    )?;
    memory.write_u8(synchronized, 1)?;
    let mut streams = Vec::new();
    for name in ["cin", "cout", "cerr", "clog", "wcin", "wcout", "wcerr", "wclog"] {
        let wide = name.starts_with('w');
        let input = name.ends_with("cin");
        let prefix = (if input { 2 } else { 1 }) * pointer;
        let address = abi_data(
            ctx,
            memory,
            &format!("_ZSt{}{name}", name.len()),
            prefix as usize + system_v_ios_layout(pointer_bytes, wide).size,
            "GLIBCXX_3.4",
        )?;
        let ios = abi_slot(memory, address, prefix)?;
        streams.push(SystemVStandardStream {
            name: name.to_string(),
            address,
            ios,
            wide,
            input,
        });
    }
    let iostreams = SystemVIostreams {
        stdio,
        streams,
        refcount,
        synchronized,
        context: ctx.clone(),
        abi: Rc::clone(abi),
        locale,
        pointer_bytes,
    };
    for name in ["_ZNSt8ios_base4InitC1Ev", "_ZNSt8ios_base4InitC2Ev"] {
        let iostreams = iostreams.clone();
        abi_function(
            ctx,
            memory,
            name,
            &[GuestStorage::Pointer],
            None,
            Rc::new(move |ctx, _, _| {
                iostreams.initialize(ctx.memory())?;
                Ok(GuestCallResult::Void)
            }),
            "GLIBCXX_3.4",
        )?;
    }
    for name in ["_ZNSt8ios_base4InitD1Ev", "_ZNSt8ios_base4InitD2Ev"] {
        let iostreams = iostreams.clone();
        abi_function(
            ctx,
            memory,
            name,
            &[GuestStorage::Pointer],
            None,
            Rc::new(move |ctx, context, _| {
                let previous = ctx.memory().read_i32(iostreams.refcount)?;
                ctx.memory().write_i32(iostreams.refcount, previous - 1)?;
                if previous == 2 {
                    for stream in iostreams.streams.iter().filter(|stream| !stream.input) {
                        iostreams.flush(ctx, context, stream.address, stream.wide)?;
                    }
                }
                Ok(GuestCallResult::Void)
            }),
            "GLIBCXX_3.4",
        )?;
    }
    for wide in [false, true] {
        let c = if wide { "w" } else { "c" };
        let clearer = iostreams.clone();
        abi_function(
            ctx,
            memory,
            &format!("_ZNSt9basic_iosI{c}St11char_traitsI{c}EE5clearESt12_Ios_Iostate"),
            &[GuestStorage::Pointer, GuestStorage::Int32],
            None,
            Rc::new(move |ctx, _, args| {
                clearer.clear(ctx.memory(), required_pointer(args, 0)?, wide, integer(args, 1)? as u32)?;
                Ok(GuestCallResult::Void)
            }),
            "GLIBCXX_3.4",
        )?;
        let flusher = iostreams.clone();
        let name = if wide {
            "_ZNSt13basic_ostreamIwSt11char_traitsIwEE5flushEv".to_string()
        } else {
            "_ZNSo5flushEv".to_string()
        };
        abi_function(
            ctx,
            memory,
            &name,
            &[GuestStorage::Pointer],
            Some(GuestStorage::Pointer),
            Rc::new(move |ctx, context, args| {
                let stream = required_pointer(args, 0)?;
                flusher.flush(ctx, context, stream, wide)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(stream))))
            }),
            "GLIBCXX_3.4",
        )?;
    }
    Ok(iostreams)
}
