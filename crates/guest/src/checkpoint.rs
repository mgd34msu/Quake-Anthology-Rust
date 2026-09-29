//! Guest execution checkpoints ported from `src/persistence/execution.ts`.
//!
//! Module identities, game API identities, native ABIs, guest memory
//! layouts, callback bindings, and the four guest checkpoint kinds
//! (`typescript`, `quakec`, `qvm`, `native-guest`). Values reuse the
//! [`qa_world::save`] codec; save failures convert into [`GuestError`].

use qa_world::save::shared::{read_digest, read_random, write_random, SaveRandomState};
use qa_world::save::value::{arr, int, namespaced, obj, str, SaveJson, SaveReader};

use crate::GuestError;

/// Module identity: selected artifact plus its retained digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleIdentity {
    /// Module id (`namespace:name`).
    pub id: String,
    /// Artifact path.
    pub artifact_path: String,
    /// Artifact digest.
    pub digest: String,
    /// Module revision.
    pub revision: String,
}

/// Read a module identity.
pub fn read_module(reader: SaveReader) -> Result<ModuleIdentity, GuestError> {
    Ok(ModuleIdentity {
        id: namespaced(reader.field("id"))?,
        artifact_path: reader.field("artifactPath").string()?,
        digest: read_digest(reader.field("digest"))?,
        revision: reader.field("revision").string()?,
    })
}

/// Write a module identity.
#[must_use]
pub fn write_module(module: &ModuleIdentity) -> SaveJson {
    obj(vec![
        ("id", str(&module.id)),
        ("artifactPath", str(&module.artifact_path)),
        ("digest", str(&module.digest)),
        ("revision", str(&module.revision)),
    ])
}

/// Game API identity with its pinned version contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameApi {
    /// NetQuake QuakeC API (program 6, CRC 5927).
    Q1Netquake,
    /// QuakeWorld QuakeC API (program 6, CRC 54730).
    Q1Quakeworld,
    /// Classic Quake II game API (version 3).
    Q2ClassicGame,
    /// Rerelease Quake II game API (version 2023).
    Q2RereleaseGame,
    /// Rerelease Quake II client-game API (version 2022).
    Q2RereleaseCgame,
    /// Quake III game API (version 7 or 8).
    Q3Qagame {
        /// API version.
        version: i64,
    },
    /// Quake III client-game API (version 3 or 4).
    Q3Cgame {
        /// API version.
        version: i64,
    },
    /// Quake III UI API (version 4 or 6).
    Q3Ui {
        /// API version.
        version: i64,
    },
}

/// Read a game API identity.
pub fn read_api(reader: SaveReader) -> Result<GameApi, GuestError> {
    let kind = reader.field("kind").choice_str(&[
        "q1-netquake",
        "q1-quakeworld",
        "q2-classic-game",
        "q2-rerelease-game",
        "q2-rerelease-cgame",
        "q3-qagame",
        "q3-cgame",
        "q3-ui",
    ])?;
    match kind.as_str() {
        "q1-netquake" => {
            reader.field("programVersion").literal_i64(6)?;
            reader.field("systemCrc").literal_i64(5927)?;
            Ok(GameApi::Q1Netquake)
        }
        "q1-quakeworld" => {
            reader.field("programVersion").literal_i64(6)?;
            reader.field("systemCrc").literal_i64(54730)?;
            Ok(GameApi::Q1Quakeworld)
        }
        "q2-classic-game" => {
            reader.field("version").literal_i64(3)?;
            Ok(GameApi::Q2ClassicGame)
        }
        "q2-rerelease-game" => {
            reader.field("version").literal_i64(2023)?;
            Ok(GameApi::Q2RereleaseGame)
        }
        "q2-rerelease-cgame" => {
            reader.field("version").literal_i64(2022)?;
            Ok(GameApi::Q2RereleaseCgame)
        }
        "q3-qagame" => Ok(GameApi::Q3Qagame {
            version: reader.field("version").choice_i64(&[7, 8])?,
        }),
        "q3-cgame" => Ok(GameApi::Q3Cgame {
            version: reader.field("version").choice_i64(&[3, 4])?,
        }),
        _ => Ok(GameApi::Q3Ui {
            version: reader.field("version").choice_i64(&[4, 6])?,
        }),
    }
}

/// Write a game API identity.
#[must_use]
pub fn write_api(api: &GameApi) -> SaveJson {
    match api {
        GameApi::Q1Netquake => obj(vec![
            ("kind", str("q1-netquake")),
            ("programVersion", int(6)),
            ("systemCrc", int(5927)),
        ]),
        GameApi::Q1Quakeworld => obj(vec![
            ("kind", str("q1-quakeworld")),
            ("programVersion", int(6)),
            ("systemCrc", int(54730)),
        ]),
        GameApi::Q2ClassicGame => obj(vec![("kind", str("q2-classic-game")), ("version", int(3))]),
        GameApi::Q2RereleaseGame => obj(vec![("kind", str("q2-rerelease-game")), ("version", int(2023))]),
        GameApi::Q2RereleaseCgame => obj(vec![("kind", str("q2-rerelease-cgame")), ("version", int(2022))]),
        GameApi::Q3Qagame { version } => obj(vec![("kind", str("q3-qagame")), ("version", int(*version))]),
        GameApi::Q3Cgame { version } => obj(vec![("kind", str("q3-cgame")), ("version", int(*version))]),
        GameApi::Q3Ui { version } => obj(vec![("kind", str("q3-ui")), ("version", int(*version))]),
    }
}

/// Native calling convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeCall {
    /// C declaration calling convention.
    Cdecl,
    /// Standard call.
    Stdcall,
    /// This call (32-bit Windows member ABI).
    Thiscall,
    /// Fast call.
    Fastcall,
    /// Microsoft x64 calling convention.
    MicrosoftX64,
    /// System V i386 calling convention.
    SystemVI386,
    /// System V x86-64 calling convention.
    SystemVX8664,
}

/// Native call ABI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeCallAbi {
    /// 32-bit Windows (`pe32`, 4-byte pointers).
    WindowsI386 {
        /// Calling convention.
        call: NativeCall,
    },
    /// 64-bit Windows (`pe32+`, 8-byte pointers).
    WindowsX8664,
    /// 32-bit Linux (`elf32`, 4-byte pointers).
    LinuxI386,
    /// 64-bit Linux (`elf64`, 8-byte pointers).
    LinuxX8664,
}

/// Read a native call ABI.
pub fn read_native_call_abi(reader: SaveReader) -> Result<NativeCallAbi, GuestError> {
    let kind = reader
        .field("kind")
        .choice_str(&["windows-i386", "windows-x86-64", "linux-i386", "linux-x86-64"])?;
    match kind.as_str() {
        "windows-i386" => {
            reader.field("image").literal_str("pe32")?;
            reader.field("pointerBytes").literal_i64(4)?;
            let call = reader
                .field("call")
                .choice_str(&["cdecl", "stdcall", "thiscall", "fastcall"])?;
            Ok(NativeCallAbi::WindowsI386 {
                call: match call.as_str() {
                    "cdecl" => NativeCall::Cdecl,
                    "stdcall" => NativeCall::Stdcall,
                    "thiscall" => NativeCall::Thiscall,
                    _ => NativeCall::Fastcall,
                },
            })
        }
        "windows-x86-64" => {
            reader.field("image").literal_str("pe32+")?;
            reader.field("pointerBytes").literal_i64(8)?;
            reader.field("call").literal_str("microsoft-x64")?;
            Ok(NativeCallAbi::WindowsX8664)
        }
        "linux-i386" => {
            reader.field("image").literal_str("elf32")?;
            reader.field("pointerBytes").literal_i64(4)?;
            reader.field("call").literal_str("system-v-i386")?;
            Ok(NativeCallAbi::LinuxI386)
        }
        _ => {
            reader.field("image").literal_str("elf64")?;
            reader.field("pointerBytes").literal_i64(8)?;
            reader.field("call").literal_str("system-v-x86-64")?;
            Ok(NativeCallAbi::LinuxX8664)
        }
    }
}

/// Write a native call ABI.
#[must_use]
pub fn write_native_call_abi(abi: &NativeCallAbi) -> SaveJson {
    match abi {
        NativeCallAbi::WindowsI386 { call } => obj(vec![
            ("kind", str("windows-i386")),
            ("image", str("pe32")),
            ("pointerBytes", int(4)),
            (
                "call",
                str(match call {
                    NativeCall::Cdecl => "cdecl",
                    NativeCall::Stdcall => "stdcall",
                    NativeCall::Thiscall => "thiscall",
                    NativeCall::Fastcall => "fastcall",
                    _ => "cdecl",
                }),
            ),
        ]),
        NativeCallAbi::WindowsX8664 => obj(vec![
            ("kind", str("windows-x86-64")),
            ("image", str("pe32+")),
            ("pointerBytes", int(8)),
            ("call", str("microsoft-x64")),
        ]),
        NativeCallAbi::LinuxI386 => obj(vec![
            ("kind", str("linux-i386")),
            ("image", str("elf32")),
            ("pointerBytes", int(4)),
            ("call", str("system-v-i386")),
        ]),
        NativeCallAbi::LinuxX8664 => obj(vec![
            ("kind", str("linux-x86-64")),
            ("image", str("elf64")),
            ("pointerBytes", int(8)),
            ("call", str("system-v-x86-64")),
        ]),
    }
}

/// Read a module-entry native ABI (32-bit Windows entries require `cdecl`).
pub fn read_native_abi(reader: SaveReader) -> Result<NativeCallAbi, GuestError> {
    let abi = read_native_call_abi(reader.clone())?;
    if matches!(&abi, NativeCallAbi::WindowsI386 { call } if *call != NativeCall::Cdecl) {
        return Err(GuestError::from(reader.fail("module entry requires cdecl")));
    }
    Ok(abi)
}

/// Scalar storage width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarStorage {
    /// Signed 8-bit.
    Int8,
    /// Unsigned 8-bit.
    Uint8,
    /// Signed 16-bit.
    Int16,
    /// Unsigned 16-bit.
    Uint16,
    /// Signed 32-bit.
    Int32,
    /// Unsigned 32-bit.
    Uint32,
    /// Signed 64-bit.
    Int64,
    /// Unsigned 64-bit.
    Uint64,
    /// 32-bit float.
    Float32,
    /// 64-bit float.
    Float64,
    /// Native pointer.
    Pointer,
}

const STORAGES: &[&str] = &[
    "int8", "uint8", "int16", "uint16", "int32", "uint32", "int64", "uint64", "float32", "float64", "pointer",
];

fn read_storage(reader: SaveReader) -> Result<ScalarStorage, GuestError> {
    let name = reader.choice_str(STORAGES)?;
    Ok(match name.as_str() {
        "int8" => ScalarStorage::Int8,
        "uint8" => ScalarStorage::Uint8,
        "int16" => ScalarStorage::Int16,
        "uint16" => ScalarStorage::Uint16,
        "int32" => ScalarStorage::Int32,
        "uint32" => ScalarStorage::Uint32,
        "int64" => ScalarStorage::Int64,
        "uint64" => ScalarStorage::Uint64,
        "float32" => ScalarStorage::Float32,
        "float64" => ScalarStorage::Float64,
        _ => ScalarStorage::Pointer,
    })
}

fn write_storage(storage: ScalarStorage) -> SaveJson {
    str(match storage {
        ScalarStorage::Int8 => "int8",
        ScalarStorage::Uint8 => "uint8",
        ScalarStorage::Int16 => "int16",
        ScalarStorage::Uint16 => "uint16",
        ScalarStorage::Int32 => "int32",
        ScalarStorage::Uint32 => "uint32",
        ScalarStorage::Int64 => "int64",
        ScalarStorage::Uint64 => "uint64",
        ScalarStorage::Float32 => "float32",
        ScalarStorage::Float64 => "float64",
        ScalarStorage::Pointer => "pointer",
    })
}

/// One guest struct field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestField {
    /// Field name.
    pub name: String,
    /// Byte offset.
    pub byte_offset: u64,
    /// Element storage.
    pub storage: ScalarStorage,
    /// Element count.
    pub count: u64,
}

/// Guest memory layout (little-endian).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestLayout {
    /// Layout id.
    pub id: String,
    /// Byte length.
    pub byte_length: u64,
    /// Alignment.
    pub alignment: u64,
    /// Pointer width (4 or 8).
    pub pointer_bytes: u64,
    /// Fields.
    pub fields: Vec<GuestField>,
}

fn nonnegative(reader: SaveReader) -> Result<u64, GuestError> {
    let value = reader.integer(0)?;
    u64::try_from(value).map_err(|_| GuestError::from(reader.fail("expected an integer in range")))
}

/// Read a guest layout.
pub fn read_layout(reader: SaveReader) -> Result<GuestLayout, GuestError> {
    reader.field("byteOrder").literal_str("little-endian")?;
    Ok(GuestLayout {
        id: namespaced(reader.field("id"))?,
        byte_length: nonnegative(reader.field("byteLength"))?,
        alignment: {
            let value = reader.field("alignment").integer(1)?;
            u64::try_from(value)
                .map_err(|_| GuestError::from(reader.field("alignment").fail("expected an integer in range")))?
        },
        pointer_bytes: {
            let value = reader.field("pointerBytes").choice_i64(&[4, 8])?;
            u64::try_from(value)
                .map_err(|_| GuestError::from(reader.field("pointerBytes").fail("expected an integer in range")))?
        },
        fields: reader.field("fields").list(|field| -> Result<GuestField, GuestError> {
            Ok(GuestField {
                name: field.field("name").string()?,
                byte_offset: nonnegative(field.field("byteOffset"))?,
                storage: read_storage(field.field("storage"))?,
                count: nonnegative(field.field("count"))?,
            })
        })?,
    })
}

/// Write a guest layout.
#[must_use]
pub fn write_layout(layout: &GuestLayout) -> SaveJson {
    #[allow(clippy::cast_possible_wrap)]
    obj(vec![
        ("id", str(&layout.id)),
        ("byteLength", int(layout.byte_length as i64)),
        ("alignment", int(layout.alignment as i64)),
        ("pointerBytes", int(layout.pointer_bytes as i64)),
        ("byteOrder", str("little-endian")),
        (
            "fields",
            arr(layout
                .fields
                .iter()
                .map(|field| {
                    obj(vec![
                        ("name", str(&field.name)),
                        ("byteOffset", int(field.byte_offset as i64)),
                        ("storage", write_storage(field.storage)),
                        ("count", int(field.count as i64)),
                    ])
                })
                .collect()),
        ),
    ])
}

/// Guest value layout: scalar or aggregate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestValueLayout {
    /// Scalar value.
    Scalar {
        /// Storage.
        storage: ScalarStorage,
    },
    /// Aggregate value.
    Aggregate {
        /// Layout.
        layout: GuestLayout,
    },
}

fn read_value_layout(reader: SaveReader) -> Result<GuestValueLayout, GuestError> {
    if reader.field("kind").choice_str(&["scalar", "aggregate"])? == "scalar" {
        Ok(GuestValueLayout::Scalar {
            storage: read_storage(reader.field("storage"))?,
        })
    } else {
        Ok(GuestValueLayout::Aggregate {
            layout: read_layout(reader.field("layout"))?,
        })
    }
}

fn write_value_layout(layout: &GuestValueLayout) -> SaveJson {
    match layout {
        GuestValueLayout::Scalar { storage } => {
            obj(vec![("kind", str("scalar")), ("storage", write_storage(*storage))])
        }
        GuestValueLayout::Aggregate { layout } => {
            obj(vec![("kind", str("aggregate")), ("layout", write_layout(layout))])
        }
    }
}

/// Saved guest callback reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestCallbackRef {
    /// TypeScript provider callback.
    Typescript {
        /// Provider.
        provider: String,
        /// Callback.
        callback: String,
    },
    /// QuakeC function.
    Quakec {
        /// Module.
        module: ModuleIdentity,
        /// Function index.
        function_index: u64,
    },
    /// QVM instruction.
    Qvm {
        /// Module.
        module: ModuleIdentity,
        /// Instruction index.
        instruction_index: u64,
    },
    /// Native guest offset.
    NativeGuest {
        /// Module.
        module: ModuleIdentity,
        /// Byte offset.
        byte_offset: i128,
        /// Call ABI.
        abi: NativeCallAbi,
    },
}

fn read_callback_ref(reader: SaveReader) -> Result<GuestCallbackRef, GuestError> {
    let kind = reader
        .field("kind")
        .choice_str(&["typescript", "quakec", "qvm", "native-guest"])?;
    match kind.as_str() {
        "typescript" => Ok(GuestCallbackRef::Typescript {
            provider: namespaced(reader.field("provider"))?,
            callback: namespaced(reader.field("callback"))?,
        }),
        "quakec" => Ok(GuestCallbackRef::Quakec {
            module: read_module(reader.field("module"))?,
            function_index: nonnegative(reader.field("functionIndex"))?,
        }),
        "qvm" => Ok(GuestCallbackRef::Qvm {
            module: read_module(reader.field("module"))?,
            instruction_index: nonnegative(reader.field("instructionIndex"))?,
        }),
        _ => Ok(GuestCallbackRef::NativeGuest {
            module: read_module(reader.field("module"))?,
            byte_offset: reader.field("byteOffset").bigint()?,
            abi: read_native_call_abi(reader.field("abi"))?,
        }),
    }
}

fn write_callback_ref(reference: &GuestCallbackRef) -> SaveJson {
    match reference {
        GuestCallbackRef::Typescript { provider, callback } => obj(vec![
            ("kind", str("typescript")),
            ("provider", str(provider)),
            ("callback", str(callback)),
        ]),
        GuestCallbackRef::Quakec { module, function_index } => obj(vec![
            ("kind", str("quakec")),
            ("module", write_module(module)),
            #[allow(clippy::cast_possible_wrap)]
            ("functionIndex", int(*function_index as i64)),
        ]),
        GuestCallbackRef::Qvm {
            module,
            instruction_index,
        } => obj(vec![
            ("kind", str("qvm")),
            ("module", write_module(module)),
            #[allow(clippy::cast_possible_wrap)]
            ("instructionIndex", int(*instruction_index as i64)),
        ]),
        GuestCallbackRef::NativeGuest {
            module,
            byte_offset,
            abi,
        } => obj(vec![
            ("kind", str("native-guest")),
            ("module", write_module(module)),
            ("byteOffset", SaveJson::BigInt(*byte_offset)),
            ("abi", write_native_call_abi(abi)),
        ]),
    }
}

/// Saved callback binding with its signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestCallbackBinding {
    /// Binding id.
    pub id: String,
    /// Reference.
    pub reference: GuestCallbackRef,
    /// Parameters.
    pub parameters: Vec<GuestValueLayout>,
    /// Result (`None` is `void`).
    pub result: Option<GuestValueLayout>,
}

fn read_callback(reader: SaveReader) -> Result<GuestCallbackBinding, GuestError> {
    let result = reader.field("result");
    Ok(GuestCallbackBinding {
        id: namespaced(reader.field("id"))?,
        reference: read_callback_ref(reader.field("reference"))?,
        parameters: reader.field("parameters").list(read_value_layout)?,
        result: match &result.value {
            Some(SaveJson::String(text)) if text == "void" => None,
            _ => Some(read_value_layout(result)?),
        },
    })
}

fn write_callback(binding: &GuestCallbackBinding) -> SaveJson {
    obj(vec![
        ("id", str(&binding.id)),
        ("reference", write_callback_ref(&binding.reference)),
        (
            "parameters",
            arr(binding.parameters.iter().map(write_value_layout).collect()),
        ),
        (
            "result",
            binding.result.as_ref().map_or(str("void"), write_value_layout),
        ),
    ])
}

/// Guest-private retained state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestPrivateState {
    /// Module.
    pub module: ModuleIdentity,
    /// Format id.
    pub format: String,
    /// Payload.
    pub bytes: Vec<u8>,
}

fn read_private(reader: SaveReader) -> Result<GuestPrivateState, GuestError> {
    Ok(GuestPrivateState {
        module: read_module(reader.field("module"))?,
        format: namespaced(reader.field("format"))?,
        bytes: reader.field("bytes").bytes()?,
    })
}

fn write_private(state: &GuestPrivateState) -> SaveJson {
    obj(vec![
        ("module", write_module(&state.module)),
        ("format", str(&state.format)),
        ("bytes", SaveJson::Bytes(state.bytes.clone())),
    ])
}

/// Guest checkpoint: one of four guest kinds plus shared module state.
#[derive(Debug, Clone, PartialEq)]
pub enum GuestCheckpoint {
    /// TypeScript guest with retained host state.
    Typescript {
        /// Module.
        module: ModuleIdentity,
        /// Random states.
        random: Vec<SaveRandomState>,
        /// Callback bindings.
        callbacks: Vec<GuestCallbackBinding>,
        /// API.
        api: GameApi,
        /// Retained state.
        state: GuestPrivateState,
    },
    /// QuakeC guest with VM image.
    Quakec {
        /// Module.
        module: ModuleIdentity,
        /// Random states.
        random: Vec<SaveRandomState>,
        /// Callback bindings.
        callbacks: Vec<GuestCallbackBinding>,
        /// API (Q1 only).
        api: GameApi,
        /// Globals image.
        globals: Vec<u8>,
        /// Entities image.
        entities: Vec<u8>,
        /// Entity stride in bytes.
        entity_stride_bytes: u64,
        /// Entity count.
        entity_count: u64,
        /// Strings image.
        strings: Vec<u8>,
        /// Current statement.
        statement: i64,
        /// Current function.
        function_index: u64,
        /// Argument count.
        argument_count: u64,
        /// Call stack (`statement`, `function`).
        call_stack: Vec<(i64, u64)>,
        /// Locals image.
        locals: Vec<u8>,
        /// Host state.
        host_state: GuestPrivateState,
    },
    /// QVM guest with VM image.
    Qvm {
        /// Module.
        module: ModuleIdentity,
        /// Random states.
        random: Vec<SaveRandomState>,
        /// Callback bindings.
        callbacks: Vec<GuestCallbackBinding>,
        /// API (Q3 only).
        api: GameApi,
        /// ABI profile.
        abi_profile: String,
        /// Data image.
        data: Vec<u8>,
        /// Instruction index.
        instruction_index: i64,
        /// Program stack pointer.
        program_stack: u64,
        /// Operand stack.
        operand_stack: Vec<i64>,
        /// Host state.
        host_state: GuestPrivateState,
    },
    /// Native guest with memory regions and processor state.
    NativeGuest {
        /// Module.
        module: ModuleIdentity,
        /// Random states.
        random: Vec<SaveRandomState>,
        /// Callback bindings.
        callbacks: Vec<GuestCallbackBinding>,
        /// ABI.
        abi: NativeCallAbi,
        /// Memory regions.
        regions: Vec<GuestRegion>,
        /// Processor layout.
        processor_layout: GuestLayout,
        /// Processor state.
        processor_state: Vec<u8>,
        /// Runtime state.
        runtime_state: GuestPrivateState,
    },
}

/// Native guest memory region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestRegion {
    /// Base address.
    pub base: i128,
    /// Permissions.
    pub permissions: String,
    /// Bytes.
    pub bytes: Vec<u8>,
}

/// Read a guest checkpoint.
pub fn read_guest(reader: SaveReader) -> Result<GuestCheckpoint, GuestError> {
    let module = read_module(reader.field("module"))?;
    let random = reader
        .field("random")
        .list(|item| read_random(item).map_err(GuestError::from))?;
    let callbacks = reader.field("callbacks").list(read_callback)?;
    let kind = reader
        .field("kind")
        .choice_str(&["typescript", "quakec", "qvm", "native-guest"])?;
    match kind.as_str() {
        "typescript" => Ok(GuestCheckpoint::Typescript {
            module,
            random,
            callbacks,
            api: read_api(reader.field("api"))?,
            state: read_private(reader.field("state"))?,
        }),
        "quakec" => {
            let api = read_api(reader.field("api"))?;
            if !matches!(api, GameApi::Q1Netquake | GameApi::Q1Quakeworld) {
                return Err(GuestError::from(reader.fail("QuakeC checkpoint requires a Q1 API")));
            }
            Ok(GuestCheckpoint::Quakec {
                module,
                random,
                callbacks,
                api,
                globals: reader.field("globals").bytes()?,
                entities: reader.field("entities").bytes()?,
                entity_stride_bytes: nonnegative(reader.field("entityStrideBytes"))?,
                entity_count: nonnegative(reader.field("entityCount"))?,
                strings: reader.field("strings").bytes()?,
                statement: reader.field("statement").integer(i64::MIN)?,
                function_index: nonnegative(reader.field("functionIndex"))?,
                argument_count: nonnegative(reader.field("argumentCount"))?,
                call_stack: reader
                    .field("callStack")
                    .list(|frame| -> Result<(i64, u64), GuestError> {
                        Ok((
                            frame.field("statement").integer(i64::MIN)?,
                            nonnegative(frame.field("functionIndex"))?,
                        ))
                    })?,
                locals: reader.field("locals").bytes()?,
                host_state: read_private(reader.field("hostState"))?,
            })
        }
        "qvm" => {
            let api = read_api(reader.field("api"))?;
            if !matches!(
                api,
                GameApi::Q3Qagame { .. } | GameApi::Q3Cgame { .. } | GameApi::Q3Ui { .. }
            ) {
                return Err(GuestError::from(reader.fail("QVM checkpoint requires a Q3 API")));
            }
            let abi_profile = reader.field("abiProfile");
            Ok(GuestCheckpoint::Qvm {
                module,
                random,
                callbacks,
                api,
                abi_profile: if abi_profile.is_missing() {
                    "q3-modern".to_string()
                } else {
                    abi_profile.choice_str(&["q3-modern", "q3-1.16n-base"])?
                },
                data: reader.field("data").bytes()?,
                instruction_index: reader.field("instructionIndex").integer(i64::MIN)?,
                program_stack: nonnegative(reader.field("programStack"))?,
                operand_stack: reader.field("operandStack").list(|item| item.integer(i64::MIN))?,
                host_state: read_private(reader.field("hostState"))?,
            })
        }
        _ => Ok(GuestCheckpoint::NativeGuest {
            module,
            random,
            callbacks,
            abi: read_native_abi(reader.field("abi"))?,
            regions: reader
                .field("regions")
                .list(|region| -> Result<GuestRegion, GuestError> {
                    Ok(GuestRegion {
                        base: region.field("base").bigint()?,
                        permissions: region.field("permissions").choice_str(&[
                            "read",
                            "read-write",
                            "read-execute",
                            "read-write-execute",
                        ])?,
                        bytes: region.field("bytes").bytes()?,
                    })
                })?,
            processor_layout: read_layout(reader.field("processorLayout"))?,
            processor_state: reader.field("processorState").bytes()?,
            runtime_state: read_private(reader.field("runtimeState"))?,
        }),
    }
}

/// Shared guest header plus kind-specific members for [`write_guest`].
type GuestHeader<'a> = (
    &'a str,
    &'a ModuleIdentity,
    &'a Vec<SaveRandomState>,
    &'a Vec<GuestCallbackBinding>,
    Vec<(&'a str, SaveJson)>,
);

/// Write a guest checkpoint.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn write_guest(checkpoint: &GuestCheckpoint) -> SaveJson {
    #[allow(clippy::cast_possible_wrap)]
    let (kind, module, random, callbacks, rest): GuestHeader<'_> = match checkpoint {
        GuestCheckpoint::Typescript {
            module,
            random,
            callbacks,
            api,
            state,
        } => (
            "typescript",
            module,
            random,
            callbacks,
            vec![("api", write_api(api)), ("state", write_private(state))],
        ),
        GuestCheckpoint::Quakec {
            module,
            random,
            callbacks,
            api,
            globals,
            entities,
            entity_stride_bytes,
            entity_count,
            strings,
            statement,
            function_index,
            argument_count,
            call_stack,
            locals,
            host_state,
        } => (
            "quakec",
            module,
            random,
            callbacks,
            vec![
                ("api", write_api(api)),
                ("globals", SaveJson::Bytes(globals.clone())),
                ("entities", SaveJson::Bytes(entities.clone())),
                ("entityStrideBytes", int(*entity_stride_bytes as i64)),
                ("entityCount", int(*entity_count as i64)),
                ("strings", SaveJson::Bytes(strings.clone())),
                ("statement", int(*statement)),
                ("functionIndex", int(*function_index as i64)),
                ("argumentCount", int(*argument_count as i64)),
                (
                    "callStack",
                    arr(call_stack
                        .iter()
                        .map(|(statement, function)| {
                            obj(vec![
                                ("statement", int(*statement)),
                                ("functionIndex", int(*function as i64)),
                            ])
                        })
                        .collect()),
                ),
                ("locals", SaveJson::Bytes(locals.clone())),
                ("hostState", write_private(host_state)),
            ],
        ),
        GuestCheckpoint::Qvm {
            module,
            random,
            callbacks,
            api,
            abi_profile,
            data,
            instruction_index,
            program_stack,
            operand_stack,
            host_state,
        } => (
            "qvm",
            module,
            random,
            callbacks,
            vec![
                ("api", write_api(api)),
                ("abiProfile", str(abi_profile)),
                ("data", SaveJson::Bytes(data.clone())),
                ("instructionIndex", int(*instruction_index)),
                ("programStack", int(*program_stack as i64)),
                (
                    "operandStack",
                    arr(operand_stack.iter().map(|value| int(*value)).collect()),
                ),
                ("hostState", write_private(host_state)),
            ],
        ),
        GuestCheckpoint::NativeGuest {
            module,
            random,
            callbacks,
            abi,
            regions,
            processor_layout,
            processor_state,
            runtime_state,
        } => (
            "native-guest",
            module,
            random,
            callbacks,
            vec![
                ("abi", write_native_call_abi(abi)),
                (
                    "regions",
                    arr(regions
                        .iter()
                        .map(|region| {
                            obj(vec![
                                ("base", SaveJson::BigInt(region.base)),
                                ("permissions", str(&region.permissions)),
                                ("bytes", SaveJson::Bytes(region.bytes.clone())),
                            ])
                        })
                        .collect()),
                ),
                ("processorLayout", write_layout(processor_layout)),
                ("processorState", SaveJson::Bytes(processor_state.clone())),
                ("runtimeState", write_private(runtime_state)),
            ],
        ),
    };
    let mut members = vec![
        ("kind", str(kind)),
        ("module", write_module(module)),
        ("random", arr(random.iter().map(write_random).collect())),
        ("callbacks", arr(callbacks.iter().map(write_callback).collect())),
    ];
    members.extend(rest);
    obj(members)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_world::save::value::{decode_checkpoint_value, encode_checkpoint_value};

    fn module() -> ModuleIdentity {
        ModuleIdentity {
            id: "q3:game".to_string(),
            artifact_path: "qagame.qvm".to_string(),
            digest: format!("sha256:{}", "0".repeat(64)),
            revision: "1".to_string(),
        }
    }

    fn round_trip(value: &SaveJson) -> SaveJson {
        decode_checkpoint_value(&encode_checkpoint_value(value)).unwrap()
    }

    #[test]
    fn guest_kinds_round_trip() {
        let private = GuestPrivateState {
            module: module(),
            format: "q3:state".to_string(),
            bytes: vec![1, 2, 3],
        };
        let checkpoints = vec![
            GuestCheckpoint::Typescript {
                module: module(),
                random: vec![SaveRandomState::Q3Lcg { seed: 1, draws: 2 }],
                callbacks: Vec::new(),
                api: GameApi::Q3Qagame { version: 8 },
                state: private.clone(),
            },
            GuestCheckpoint::Quakec {
                module: module(),
                random: Vec::new(),
                callbacks: Vec::new(),
                api: GameApi::Q1Netquake,
                globals: vec![0],
                entities: vec![0, 0],
                entity_stride_bytes: 32,
                entity_count: 1,
                strings: vec![0],
                statement: -1,
                function_index: 0,
                argument_count: 2,
                call_stack: vec![(4, 1)],
                locals: vec![7],
                host_state: private.clone(),
            },
            GuestCheckpoint::Qvm {
                module: module(),
                random: Vec::new(),
                callbacks: vec![GuestCallbackBinding {
                    id: "q3:fire".to_string(),
                    reference: GuestCallbackRef::Qvm {
                        module: module(),
                        instruction_index: 9,
                    },
                    parameters: vec![GuestValueLayout::Scalar {
                        storage: ScalarStorage::Int32,
                    }],
                    result: None,
                }],
                api: GameApi::Q3Qagame { version: 7 },
                abi_profile: "q3-1.16n-base".to_string(),
                data: vec![5],
                instruction_index: 3,
                program_stack: 64,
                operand_stack: vec![1, -2],
                host_state: private.clone(),
            },
            GuestCheckpoint::NativeGuest {
                module: module(),
                random: Vec::new(),
                callbacks: Vec::new(),
                abi: NativeCallAbi::LinuxX8664,
                regions: vec![GuestRegion {
                    base: 0x1000,
                    permissions: "read-execute".to_string(),
                    bytes: vec![0x90],
                }],
                processor_layout: GuestLayout {
                    id: "x64:state".to_string(),
                    byte_length: 8,
                    alignment: 8,
                    pointer_bytes: 8,
                    fields: vec![GuestField {
                        name: "rip".to_string(),
                        byte_offset: 0,
                        storage: ScalarStorage::Uint64,
                        count: 1,
                    }],
                },
                processor_state: vec![0; 8],
                runtime_state: private.clone(),
            },
        ];
        for checkpoint in &checkpoints {
            let json = write_guest(checkpoint);
            assert_eq!(
                read_guest(SaveReader::at(&round_trip(&json), "g")).unwrap(),
                *checkpoint
            );
        }
    }

    #[test]
    fn api_mismatches_fail() {
        let written = write_guest(&GuestCheckpoint::Quakec {
            module: module(),
            random: Vec::new(),
            callbacks: Vec::new(),
            api: GameApi::Q1Netquake,
            globals: Vec::new(),
            entities: Vec::new(),
            entity_stride_bytes: 1,
            entity_count: 0,
            strings: Vec::new(),
            statement: 0,
            function_index: 0,
            argument_count: 0,
            call_stack: Vec::new(),
            locals: Vec::new(),
            host_state: GuestPrivateState {
                module: module(),
                format: "q1:state".to_string(),
                bytes: Vec::new(),
            },
        });
        let SaveJson::Object(members) = written else {
            panic!("expected object");
        };
        let swapped = SaveJson::Object(
            members
                .into_iter()
                .map(|(key, value)| {
                    if key == "api" {
                        (key, write_api(&GameApi::Q3Qagame { version: 8 }))
                    } else {
                        (key, value)
                    }
                })
                .collect(),
        );
        assert!(read_guest(SaveReader::new(&swapped)).is_err());
        let bad_abi = obj(vec![
            ("kind", str("windows-i386")),
            ("image", str("pe32")),
            ("pointerBytes", int(4)),
            ("call", str("stdcall")),
        ]);
        assert!(read_native_abi(SaveReader::new(&bad_abi)).is_err());
        assert!(read_native_call_abi(SaveReader::new(&bad_abi)).is_ok());
    }
}
