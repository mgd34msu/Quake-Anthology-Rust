//! QuakeC machine surface used by the Q1 QuakeC bindings.
//!
//! Donor provenance: `src/compat/qc/program.ts` (`QcOpcode`,
//! `QcProgram`, `QcStatement`, `QcFunction`, `signedQcBranch`),
//! `src/compat/qc/machine.ts` (`QcMachine`, boundaries, calls, regions,
//! observations), `src/compat/qc/memory.ts` (`QcWords`), and the used
//! `src/compat/qc/source-call.ts` behavior (`qcSourceValueType`,
//! `validateQcSourceCall`, `withQcSourceCall`).
//!
//! [`QcProgramView`] is borrowed program data the `compat` lane builds
//! from the real `QcProgram`. [`QcMachineView`] and the execution
//! traits are implemented by the `compat` lane for the real VM.
//! Boundaries are concrete handler structs this module's bindings
//! compose; the VM installs them. Gameplay-behavior traits that need
//! [`QcError`] ([`GameplayAuthority`], [`SourceArmorStage`]) live here;
//! the gameplay data they carry lives in [`super::qc_gameplay`].

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use qa_core::numeric::NumericOps;

use crate::contract::{ModCallbackInput, ModCallbackValue, ModClientInput, ModRuntimeValue, ModSourceCall};

use super::qc_gameplay::{
    ArmorStageInput, DamageOutcome, DamageRequest, QcActorRegistry, QcActorSlots, SourceDamageObserver,
    SourceDamageResult,
};
use super::{fround, QcError};

/// QuakeC opcode (donor `QcOpcode`; discriminants are the file encoding).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QcOpcode {
    /// End of function.
    Done = 0,
    /// Float multiply.
    MulF = 1,
    /// Vector dot product.
    MulV = 2,
    /// Float-vector scale.
    MulFV = 3,
    /// Vector-float scale.
    MulVF = 4,
    /// Float divide.
    DivF = 5,
    /// Float add.
    AddF = 6,
    /// Vector add.
    AddV = 7,
    /// Float subtract.
    SubF = 8,
    /// Vector subtract.
    SubV = 9,
    /// Float equality.
    EqF = 10,
    /// Vector equality.
    EqV = 11,
    /// String equality.
    EqS = 12,
    /// Entity equality.
    EqE = 13,
    /// Function equality.
    EqFn = 14,
    /// Float inequality.
    NeF = 15,
    /// Vector inequality.
    NeV = 16,
    /// String inequality.
    NeS = 17,
    /// Entity inequality.
    NeE = 18,
    /// Function inequality.
    NeFn = 19,
    /// Less than or equal.
    Le = 20,
    /// Greater than or equal.
    Ge = 21,
    /// Less than.
    Lt = 22,
    /// Greater than.
    Gt = 23,
    /// Load float field.
    LoadF = 24,
    /// Load vector field.
    LoadV = 25,
    /// Load string field.
    LoadS = 26,
    /// Load entity field.
    LoadEnt = 27,
    /// Load field field.
    LoadFld = 28,
    /// Load function field.
    LoadFn = 29,
    /// Address of entity field.
    Address = 30,
    /// Store float.
    StoreF = 31,
    /// Store vector.
    StoreV = 32,
    /// Store string.
    StoreS = 33,
    /// Store entity.
    StoreEnt = 34,
    /// Store field.
    StoreFld = 35,
    /// Store function.
    StoreFn = 36,
    /// Store float through pointer.
    StorePF = 37,
    /// Store vector through pointer.
    StorePV = 38,
    /// Store string through pointer.
    StorePS = 39,
    /// Store entity through pointer.
    StorePEnt = 40,
    /// Store field through pointer.
    StorePFld = 41,
    /// Store function through pointer.
    StorePFn = 42,
    /// Return from function.
    Return = 43,
    /// Float logical not.
    NotF = 44,
    /// Vector logical not.
    NotV = 45,
    /// String logical not.
    NotS = 46,
    /// Entity logical not.
    NotEnt = 47,
    /// Function logical not.
    NotFn = 48,
    /// Branch if true.
    If = 49,
    /// Branch if false.
    IfNot = 50,
    /// Call with 0 arguments.
    Call0 = 51,
    /// Call with 1 argument.
    Call1 = 52,
    /// Call with 2 arguments.
    Call2 = 53,
    /// Call with 3 arguments.
    Call3 = 54,
    /// Call with 4 arguments.
    Call4 = 55,
    /// Call with 5 arguments.
    Call5 = 56,
    /// Call with 6 arguments.
    Call6 = 57,
    /// Call with 7 arguments.
    Call7 = 58,
    /// Call with 8 arguments.
    Call8 = 59,
    /// Set actor state.
    State = 60,
    /// Unconditional branch.
    Goto = 61,
    /// Logical and.
    And = 62,
    /// Logical or.
    Or = 63,
    /// Bitwise and.
    BitAnd = 64,
    /// Bitwise or.
    BitOr = 65,
}

impl QcOpcode {
    /// File encoding.
    #[must_use]
    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// Decode a file opcode.
    #[must_use]
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Done),
            1 => Some(Self::MulF),
            2 => Some(Self::MulV),
            3 => Some(Self::MulFV),
            4 => Some(Self::MulVF),
            5 => Some(Self::DivF),
            6 => Some(Self::AddF),
            7 => Some(Self::AddV),
            8 => Some(Self::SubF),
            9 => Some(Self::SubV),
            10 => Some(Self::EqF),
            11 => Some(Self::EqV),
            12 => Some(Self::EqS),
            13 => Some(Self::EqE),
            14 => Some(Self::EqFn),
            15 => Some(Self::NeF),
            16 => Some(Self::NeV),
            17 => Some(Self::NeS),
            18 => Some(Self::NeE),
            19 => Some(Self::NeFn),
            20 => Some(Self::Le),
            21 => Some(Self::Ge),
            22 => Some(Self::Lt),
            23 => Some(Self::Gt),
            24 => Some(Self::LoadF),
            25 => Some(Self::LoadV),
            26 => Some(Self::LoadS),
            27 => Some(Self::LoadEnt),
            28 => Some(Self::LoadFld),
            29 => Some(Self::LoadFn),
            30 => Some(Self::Address),
            31 => Some(Self::StoreF),
            32 => Some(Self::StoreV),
            33 => Some(Self::StoreS),
            34 => Some(Self::StoreEnt),
            35 => Some(Self::StoreFld),
            36 => Some(Self::StoreFn),
            37 => Some(Self::StorePF),
            38 => Some(Self::StorePV),
            39 => Some(Self::StorePS),
            40 => Some(Self::StorePEnt),
            41 => Some(Self::StorePFld),
            42 => Some(Self::StorePFn),
            43 => Some(Self::Return),
            44 => Some(Self::NotF),
            45 => Some(Self::NotV),
            46 => Some(Self::NotS),
            47 => Some(Self::NotEnt),
            48 => Some(Self::NotFn),
            49 => Some(Self::If),
            50 => Some(Self::IfNot),
            51 => Some(Self::Call0),
            52 => Some(Self::Call1),
            53 => Some(Self::Call2),
            54 => Some(Self::Call3),
            55 => Some(Self::Call4),
            56 => Some(Self::Call5),
            57 => Some(Self::Call6),
            58 => Some(Self::Call7),
            59 => Some(Self::Call8),
            60 => Some(Self::State),
            61 => Some(Self::Goto),
            62 => Some(Self::And),
            63 => Some(Self::Or),
            64 => Some(Self::BitAnd),
            65 => Some(Self::BitOr),
            _ => None,
        }
    }

    /// Whether the opcode is a call (`Call0..=Call8`).
    #[must_use]
    pub fn is_call(self) -> bool {
        matches!(
            self,
            Self::Call0
                | Self::Call1
                | Self::Call2
                | Self::Call3
                | Self::Call4
                | Self::Call5
                | Self::Call6
                | Self::Call7
                | Self::Call8
        )
    }

    /// Call argument count for call opcodes.
    #[must_use]
    pub fn call_arity(self) -> Option<u32> {
        self.is_call().then(|| self.as_u32() - QcOpcode::Call0.as_u32())
    }
}

/// Version 6 branch displacements are signed 16-bit values (donor
/// `signedQcBranch`).
#[must_use]
pub fn signed_qc_branch(word: u16) -> i32 {
    i32::from(word as i16)
}

/// QuakeC value type (donor `QcValueType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcValueType {
    /// Void.
    Void,
    /// String.
    Str,
    /// Float.
    Float,
    /// Vector.
    Vector,
    /// Entity.
    Entity,
    /// Field.
    Field,
    /// Function.
    Function,
    /// Pointer.
    Pointer,
    /// Compiler-only definition with unknown text-save type.
    Opaque,
}

/// Global or field definition used by the bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcDefinitionView {
    /// Value type.
    pub def_type: QcValueType,
    /// Word offset.
    pub offset: usize,
    /// Definition name.
    pub name: String,
}

/// Compiled statement used by the bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QcStatementView {
    /// Opcode.
    pub opcode: QcOpcode,
    /// Operand A.
    pub a: u16,
    /// Operand B.
    pub b: u16,
    /// Operand C.
    pub c: u16,
}

/// Compiled function used by the bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcFunctionView {
    /// Function index.
    pub index: usize,
    /// First statement (negative for numbered builtins).
    pub first_statement: i32,
    /// Frame start word.
    pub parameter_start: usize,
    /// Frame word count.
    pub local_words: usize,
    /// Function name.
    pub name: String,
    /// Parameter word widths.
    pub parameter_sizes: Vec<usize>,
    /// Whether the function is a named builtin.
    pub named_builtin: bool,
}

/// QuakeC API identity used by the bindings (donor `QuakeCApiIdentity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcApiKind {
    /// NetQuake system layout.
    Netquake,
    /// QuakeWorld system layout.
    Quakeworld,
}

/// Borrowed program data (donor `QcProgram`).
///
/// The `compat` lane builds this from the real program; name maps keep
/// the first definition, matching the donor constructor.
#[derive(Debug, Clone)]
pub struct QcProgramView<'a> {
    /// Artifact label (donor `source`).
    pub source: &'a str,
    /// System layout identity.
    pub api: QcApiKind,
    /// Compiled statements.
    pub statements: &'a [QcStatementView],
    /// Global definitions.
    pub globals: &'a [QcDefinitionView],
    /// Entity field definitions.
    pub fields: &'a [QcDefinitionView],
    /// Function table.
    pub functions: &'a [QcFunctionView],
    /// Initial global words.
    pub initial_globals: &'a [u8],
    /// Artifact digest (`sha256:...`).
    pub digest: &'a str,
    /// First global definition per name.
    globals_by_name: BTreeMap<&'a str, usize>,
    /// First field definition per name.
    fields_by_name: BTreeMap<&'a str, usize>,
    /// First function per name.
    functions_by_name: BTreeMap<&'a str, usize>,
}

impl<'a> QcProgramView<'a> {
    /// Borrow program data and index its names.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: &'a str,
        api: QcApiKind,
        statements: &'a [QcStatementView],
        globals: &'a [QcDefinitionView],
        fields: &'a [QcDefinitionView],
        functions: &'a [QcFunctionView],
        initial_globals: &'a [u8],
        digest: &'a str,
    ) -> Self {
        let mut view = Self {
            source,
            api,
            statements,
            globals,
            fields,
            functions,
            initial_globals,
            digest,
            globals_by_name: BTreeMap::new(),
            fields_by_name: BTreeMap::new(),
            functions_by_name: BTreeMap::new(),
        };
        for (index, definition) in globals.iter().enumerate() {
            view.globals_by_name.entry(definition.name.as_str()).or_insert(index);
        }
        for (index, definition) in fields.iter().enumerate() {
            view.fields_by_name.entry(definition.name.as_str()).or_insert(index);
        }
        for (index, function) in functions.iter().enumerate() {
            view.functions_by_name.entry(function.name.as_str()).or_insert(index);
        }
        view
    }

    /// Function by index (donor `functionAt`).
    pub fn function_at(&self, index: usize) -> Result<&QcFunctionView, QcError> {
        if index == 0 {
            return Err(QcError::program(format!("invalid function {index}"), self.source));
        }
        self.functions
            .get(index)
            .ok_or_else(|| QcError::program(format!("invalid function {index}"), self.source))
    }

    /// Function by name (donor `functionNamed`).
    pub fn function_named(&self, name: &str) -> Result<&QcFunctionView, QcError> {
        let function = self
            .functions_by_name
            .get(name)
            .and_then(|index| self.functions.get(*index));
        match function {
            Some(function) if function.index != 0 => Ok(function),
            _ => Err(QcError::program(format!("missing function {name}"), self.source)),
        }
    }

    /// Global definition by name (donor `globalsByName`).
    #[must_use]
    pub fn global_named(&self, name: &str) -> Option<&QcDefinitionView> {
        self.globals_by_name
            .get(name)
            .and_then(|index| self.globals.get(*index))
    }

    /// Field definition by name (donor `fieldsByName`).
    #[must_use]
    pub fn field_named(&self, name: &str) -> Option<&QcDefinitionView> {
        self.fields_by_name.get(name).and_then(|index| self.fields.get(*index))
    }

    /// Exclusive end statement of the function starting at `first`.
    #[must_use]
    pub fn function_end(&self, first: i32) -> usize {
        self.functions
            .iter()
            .filter(|function| function.first_statement > first)
            .map(|function| usize::try_from(function.first_statement).unwrap_or(usize::MAX))
            .min()
            .unwrap_or(self.statements.len())
    }

    /// Initial global word as `i32` (donor `DataView.getInt32`).
    pub fn initial_i32(&self, word: usize) -> Result<i32, QcError> {
        let bytes = self.initial_word_bytes(word)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Initial global word as `f32` (donor `DataView.getFloat32`).
    pub fn initial_f32(&self, word: usize) -> Result<f32, QcError> {
        let bytes = self.initial_word_bytes(word)?;
        Ok(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Initial global word count.
    #[must_use]
    pub fn initial_words(&self) -> usize {
        self.initial_globals.len() / 4
    }

    /// Raw bytes of one initial word.
    fn initial_word_bytes(&self, word: usize) -> Result<&[u8], QcError> {
        let start = word.saturating_mul(4);
        self.initial_globals
            .get(start..start.saturating_add(4))
            .ok_or_else(|| QcError::program(format!("word {word} outside initial globals"), self.source))
    }
}

/// Standalone inline result scope (donor `standalone.scope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StandaloneScope {
    /// Frame-local result word.
    Frame,
    /// Global result word.
    Global,
}

/// Standalone inline result (donor `QcInlineRegion.standalone`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InlineStandalone {
    /// Result word.
    pub saved: usize,
    /// Result scope.
    pub scope: StandaloneScope,
}

/// Inline source region (donor `QcInlineRegion`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QcInlineRegion {
    /// Owning function index.
    pub function_index: usize,
    /// Region entry statement.
    pub entry: usize,
    /// Region join statement.
    pub exit: usize,
    /// Whether the region may be replaced.
    pub replaceable: bool,
    /// Standalone result, when the region can execute alone.
    pub standalone: Option<InlineStandalone>,
}

/// Function call site (donor `QcCallSite`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QcCallSite {
    /// Called function index.
    pub function_index: usize,
    /// Calling function index.
    pub caller: usize,
    /// Call statement.
    pub statement: usize,
}

/// Observed entity store (donor `QcEntityStoreObservation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcEntityStoreObservation {
    /// Executing function index.
    pub function_index: usize,
    /// Executing statement.
    pub statement: usize,
    /// Entity reference.
    pub reference: i32,
    /// Stored word.
    pub word: usize,
    /// Bytes before the store.
    pub before: Vec<u8>,
    /// Bytes after the store.
    pub after: Vec<u8>,
}

/// Owned QuakeC word storage (donor `QcWords`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcWordsBuf {
    /// Little-endian word bytes.
    bytes: Vec<u8>,
}

impl QcWordsBuf {
    /// Wrap whole-word bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, QcError> {
        if !bytes.len().is_multiple_of(4) {
            return Err(QcError::program("QC storage must contain whole words", "progs.dat"));
        }
        Ok(Self { bytes })
    }

    /// Word count.
    #[must_use]
    pub fn len_words(&self) -> usize {
        self.bytes.len() / 4
    }

    /// Whether the storage is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Borrow the raw bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Read a word as `i32`.
    pub fn int(&self, word: usize) -> Result<i32, QcError> {
        let bytes = self.word_bytes(word)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read a word as `f64` (donor `float` returns a `number`).
    pub fn float(&self, word: usize) -> Result<f64, QcError> {
        let bytes = self.word_bytes(word)?;
        Ok(f64::from(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])))
    }

    /// Read three words as a vector.
    pub fn vector(&self, word: usize) -> Result<Vec3, QcError> {
        #[allow(clippy::cast_possible_truncation)]
        Ok(Vec3 {
            x: self.float(word)? as f32,
            y: self.float(word + 1)? as f32,
            z: self.float(word + 2)? as f32,
        })
    }

    /// Write a word as `i32`.
    pub fn set_int(&mut self, word: usize, value: i32) -> Result<(), QcError> {
        let offset = self.word_offset(word)?;
        self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write a word as binary32.
    pub fn set_float(&mut self, word: usize, value: f64) -> Result<(), QcError> {
        let offset = self.word_offset(word)?;
        self.bytes[offset..offset + 4].copy_from_slice(&(value as f32).to_le_bytes());
        Ok(())
    }

    /// Overwrite bytes at a byte offset.
    pub fn set_bytes(&mut self, offset: usize, bytes: &[u8]) -> Result<(), QcError> {
        let end = offset.saturating_add(bytes.len());
        if end > self.bytes.len() {
            return Err(QcError::program(
                format!("word write outside {}-word storage", self.len_words()),
                "progs.dat",
            ));
        }
        self.bytes[offset..end].copy_from_slice(bytes);
        Ok(())
    }

    /// Raw bytes of one word.
    fn word_bytes(&self, word: usize) -> Result<&[u8], QcError> {
        let offset = self.word_offset(word)?;
        Ok(&self.bytes[offset..offset + 4])
    }

    /// Byte offset of one word.
    fn word_offset(&self, word: usize) -> Result<usize, QcError> {
        let offset = word.saturating_mul(4);
        if offset + 4 > self.bytes.len() {
            return Err(QcError::program(
                format!("word {word} outside {}-word storage", self.len_words()),
                "progs.dat",
            ));
        }
        Ok(offset)
    }
}

/// Global definition info (offset plus type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QcGlobalInfo {
    /// Word offset.
    pub offset: usize,
    /// Value type.
    pub def_type: QcValueType,
}

/// QuakeC machine behavior used by the bindings (donor `QcMachine`).
///
/// Implemented by the `compat` lane for the real VM. Float reads return
/// the binary32 word widened to `f64`, matching the donor `number`.
pub trait QcMachineView {
    /// Artifact label of the running program.
    fn program_source(&self) -> &str;
    /// Artifact digest of the running program.
    fn program_digest(&self) -> &str;
    /// Arithmetic operations bound to the machine profile.
    fn numeric(&self) -> NumericOps;
    /// Global word as `i32`.
    fn global_int(&self, word: usize) -> Result<i32, QcError>;
    /// Global word as `f64`.
    fn global_float(&self, word: usize) -> Result<f64, QcError>;
    /// Write a global word as `i32`.
    fn set_global_int(&self, word: usize, value: i32) -> Result<(), QcError>;
    /// Write a global word as binary32.
    fn set_global_float(&self, word: usize, value: f64) -> Result<(), QcError>;
    /// Three global words as a vector.
    fn global_vector(&self, word: usize) -> Result<Vec3, QcError>;
    /// Write three global words as a vector.
    fn set_global_vector(&self, word: usize, value: Vec3) -> Result<(), QcError>;
    /// Global definition info by name.
    fn global_definition(&self, name: &str) -> Result<QcGlobalInfo, QcError>;
    /// Global word offset by name (donor `globalOffset`).
    fn global_offset(&self, name: &str) -> Result<usize, QcError> {
        Ok(self.global_definition(name)?.offset)
    }
    /// Entity field word offset by name (donor `fieldOffset`).
    fn field_offset(&self, name: &str) -> Result<usize, QcError>;
    /// Call argument as `i32` (donor `argInt`).
    fn arg_int(&self, index: usize) -> Result<i32, QcError>;
    /// Call argument as `f64` (donor `argFloat`).
    fn arg_float(&self, index: usize) -> Result<f64, QcError>;
    /// String at an offset (donor `strings.get`).
    fn strings_get(&self, offset: i32) -> Result<String, QcError>;
    /// Intern an engine string (donor `strings.setEngine`).
    fn set_engine_string(&self, name: &str, value: &str, capacity: usize) -> Result<i32, QcError>;
    /// Entity slot for a reference (donor `entities.slot`).
    fn entity_slot(&self, reference: i32) -> Result<usize, QcError>;
    /// Reference for an entity slot (donor `entities.reference`).
    fn entity_reference(&self, slot: usize) -> Result<i32, QcError>;
    /// Entity word as `i32`.
    fn entity_int(&self, slot: usize, word: usize) -> Result<i32, QcError>;
    /// Entity word as `f64`.
    fn entity_float(&self, slot: usize, word: usize) -> Result<f64, QcError>;
    /// Three entity words as a vector.
    fn entity_vector(&self, slot: usize, word: usize) -> Result<Vec3, QcError>;
    /// Write an entity word as `i32`.
    fn set_entity_int(&self, slot: usize, word: usize, value: i32) -> Result<(), QcError>;
    /// Write an entity word as binary32.
    fn set_entity_float(&self, slot: usize, word: usize, value: f64) -> Result<(), QcError>;
    /// Full entity field bytes for snapshots.
    fn entity_snapshot(&self, slot: usize) -> Result<Vec<u8>, QcError>;
    /// Call staging bytes (donor `globals.bytes.slice(4, 112)`).
    fn staging_snapshot(&self) -> Vec<u8>;
    /// Restore call staging bytes.
    fn restore_staging(&self, bytes: &[u8]);
    /// Raw global bytes at a word offset.
    fn global_range(&self, offset: usize, words: usize) -> Result<Vec<u8>, QcError>;
    /// Restore raw global bytes (offsets come from the VM itself).
    fn set_global_range(&self, offset: usize, bytes: &[u8]);
    /// Execute a function (donor `execute`).
    fn execute(&self, function: usize, argc: usize) -> Result<(), QcError>;
    /// Execute a standalone region (donor `executeRegion`).
    fn execute_region(&self, region: &QcInlineRegion, argc: usize) -> Result<f64, QcError>;
}

/// Machine accessor (donor `machine: () => QcMachine`).
pub type MachineFn<'a> = Box<dyn (Fn() -> &'a (dyn QcMachineView + 'a)) + 'a>;

/// World-host subset the bindings borrow (donor
/// `Pick<QcWorldHostOptions, "program" | "entities" | "actors" |
/// "slots">`; entity memory is reached through [`QcMachineView`]).
pub struct QcHostSource<'a> {
    /// Borrowed program data.
    pub program: &'a QcProgramView<'a>,
    /// Session actor registry.
    pub actors: &'a dyn QcActorRegistry,
    /// Source actor slots.
    pub slots: &'a dyn QcActorSlots,
}

/// Source-function prepare hook (donor `QcFunctionExecution.run`
/// argument).
pub type QcPrepareHook<'a> = &'a mut dyn FnMut(&dyn QcMachineView) -> Result<(), QcError>;

/// Function execution continuation (donor `QcFunctionExecution`).
///
/// Implemented by the `compat` lane for the real VM.
pub trait QcFunctionExecution {
    /// Run the source function, optionally preparing the machine first.
    fn run(&self, prepare: Option<QcPrepareHook<'_>>) -> Result<(), QcError>;
    /// Skip the call with replacement return words (donor `skip`).
    fn skip(&self, words: [i32; 3]);
    /// Owner identity for [`QcError::cancelled`] (donor `cancel`
    /// diverges; the caller must return the built error immediately).
    fn cancel_owner(&self) -> u64;
}

/// Inline region continuation (donor `QcInlineContinuation`).
///
/// Implemented by the `compat` lane for the real VM.
pub trait QcInlineContinuation {
    /// Execute the region.
    fn run(&self) -> Result<(), QcError>;
    /// Skip the region to its join.
    fn skip_to_join(&self) -> Result<(), QcError>;
}

/// Function-boundary dispatch (donor `QcFunctionBoundary.run`).
pub type QcFunctionDispatch<'a> = Box<dyn Fn(&QcCallSite, &dyn QcFunctionExecution) -> Result<(), QcError> + 'a>;

/// Composable function boundary (donor `QcFunctionBoundary`).
pub struct QcFunctionBoundary<'a> {
    /// Intercepted function indices.
    pub functions: HashSet<usize>,
    /// Boundary dispatch.
    pub run: QcFunctionDispatch<'a>,
}

impl std::fmt::Debug for QcFunctionBoundary<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QcFunctionBoundary")
            .field("functions", &self.functions)
            .finish_non_exhaustive()
    }
}

/// Inline-boundary dispatch (donor `QcInlineBoundary.run`).
pub type QcInlineDispatch<'a> = Box<dyn Fn(&QcInlineRegion, &dyn QcInlineContinuation) -> Result<(), QcError> + 'a>;

/// Composable inline boundary (donor `QcInlineBoundary`).
pub struct QcInlineBoundary<'a> {
    /// Intercepted regions.
    pub regions: Vec<QcInlineRegion>,
    /// Boundary dispatch.
    pub run: QcInlineDispatch<'a>,
}

impl std::fmt::Debug for QcInlineBoundary<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QcInlineBoundary")
            .field("regions", &self.regions)
            .finish_non_exhaustive()
    }
}

/// Canonical source-damage execution (donor
/// `GameplayAuthority.runSourceDamage` argument).
pub type SourceDamageExecute<'a> =
    &'a mut dyn FnMut(&Rc<dyn SourceDamageObserver>, DamageRequest) -> Result<SourceDamageResult, QcError>;

/// Shared combat authority behavior used by the damage binding (donor
/// `GameplayAuthority`: `apply`, `runSourceDamage`,
/// `damageOperation.active`).
///
/// Implemented by the gameplay owner; the observer is shared because the
/// damage frame holds it until the source call unwinds.
pub trait GameplayAuthority {
    /// Apply damage, optionally through source bytecode.
    fn apply(
        &self,
        request: DamageRequest,
        source_damage: Option<&mut dyn FnMut(DamageRequest) -> Result<DamageOutcome, QcError>>,
    ) -> Result<DamageOutcome, QcError>;
    /// Run canonical source damage with an observer.
    fn run_source_damage(
        &self,
        request: DamageRequest,
        execute: SourceDamageExecute<'_>,
    ) -> Result<DamageOutcome, QcError>;
    /// Whether a composed damage operation is active.
    fn damage_operation_active(&self) -> bool;
}

/// Armor intercept: savings for an input, with the original region
/// available as a fallback (donor `SourceArmorStage.bind` intercept).
pub type ArmorIntercept = Box<dyn Fn(&ArmorStageInput, &dyn Fn() -> Result<f64, QcError>) -> Result<f64, QcError>>;

/// Source armor stage (donor `SourceArmorStage`).
pub trait SourceArmorStage {
    /// Bind an interceptor, returning its release.
    fn bind(&self, intercept: ArmorIntercept) -> Result<Box<dyn FnOnce()>, QcError>;
}

/// Source value type (donor `qcSourceValueType`).
#[must_use]
pub fn qc_source_value_type(value: &ModCallbackValue) -> QcValueType {
    match value {
        ModCallbackValue::Float(_) => QcValueType::Float,
        ModCallbackValue::Str(_) => QcValueType::Str,
        ModCallbackValue::Vector(_) => QcValueType::Vector,
        ModCallbackValue::Input(input) => match input {
            ModCallbackInput::Slf
            | ModCallbackInput::Other
            | ModCallbackInput::Activator
            | ModCallbackInput::Attacker
            | ModCallbackInput::Inflictor => QcValueType::Entity,
            ModCallbackInput::Point
            | ModCallbackInput::Direction
            | ModCallbackInput::Normal
            | ModCallbackInput::Client(ModClientInput::ViewAngles) => QcValueType::Vector,
            ModCallbackInput::Item => QcValueType::Str,
            _ => QcValueType::Float,
        },
    }
}

/// Donor spelling of a callback input for diagnostics.
fn input_name(input: ModCallbackInput) -> &'static str {
    match input {
        ModCallbackInput::Client(ModClientInput::ViewAngles) => "view-angles",
        ModCallbackInput::Client(ModClientInput::Attack) => "attack",
        ModCallbackInput::Client(ModClientInput::Jump) => "jump",
        ModCallbackInput::Client(ModClientInput::Impulse) => "impulse",
        ModCallbackInput::Client(ModClientInput::ForwardMove) => "forward-move",
        ModCallbackInput::Client(ModClientInput::SideMove) => "side-move",
        ModCallbackInput::Client(ModClientInput::UpMove) => "up-move",
        ModCallbackInput::Slf => "self",
        ModCallbackInput::Other => "other",
        ModCallbackInput::Activator => "activator",
        ModCallbackInput::Attacker => "attacker",
        ModCallbackInput::Inflictor => "inflictor",
        ModCallbackInput::Amount => "amount",
        ModCallbackInput::DamageFlags => "damage-flags",
        ModCallbackInput::RegularProtectionScale => "regular-protection-scale",
        ModCallbackInput::Knockback => "knockback",
        ModCallbackInput::Point => "point",
        ModCallbackInput::Direction => "direction",
        ModCallbackInput::Normal => "normal",
        ModCallbackInput::Item => "item",
        ModCallbackInput::Time => "time",
        ModCallbackInput::Elapsed => "elapsed",
        ModCallbackInput::Result => "result",
        ModCallbackInput::PickupCount => "pickup-count",
        ModCallbackInput::PickupHasCount => "pickup-has-count",
        ModCallbackInput::PickupDropped => "pickup-dropped",
    }
}

/// Validate a declared source call (donor `validateQcSourceCall`).
pub fn validate_qc_source_call(
    program: &QcProgramView,
    call: &ModSourceCall,
    available: &HashSet<ModCallbackInput>,
    label: &str,
) -> Result<(), QcError> {
    for value in call
        .arguments
        .iter()
        .chain(call.globals.iter().map(|global| &global.value))
    {
        if let ModCallbackValue::Input(name) = value {
            if !available.contains(name) {
                return Err(QcError::program(
                    format!("Mod {label} cannot read {}", input_name(*name)),
                    program.source,
                ));
            }
        }
    }
    let function = program.function_named(&call.function)?;
    let signature = function.parameter_sizes.len() == call.arguments.len()
        && function.parameter_sizes.iter().enumerate().all(|(index, size)| {
            call.arguments
                .get(index)
                .is_some_and(|value| *size == usize::from(qc_source_value_type(value) == QcValueType::Vector) * 2 + 1)
        });
    if !signature {
        return Err(QcError::program(
            format!("Mod callback {} has an incompatible source signature", call.function),
            program.source,
        ));
    }
    let mut globals = HashSet::new();
    for global in &call.globals {
        let expected = qc_source_value_type(&global.value);
        let matches = program
            .global_named(&global.name)
            .is_some_and(|definition| definition.def_type == expected);
        if !globals.insert(global.name.clone()) || !matches {
            return Err(QcError::program(
                format!(
                    "Mod callback global {} is duplicated or has an incompatible type",
                    global.name
                ),
                program.source,
            ));
        }
    }
    Ok(())
}

/// Write one staged source value (donor `writeQcSourceValue`).
fn write_qc_source_value(
    machine: &dyn QcMachineView,
    offset: usize,
    value: &ModRuntimeValue,
    reference: &dyn Fn(Option<&ActorId>) -> Result<i32, QcError>,
) -> Result<(), QcError> {
    match value {
        ModRuntimeValue::Float(value) => {
            if !fround(*value).is_finite() {
                return Err(QcError::program(
                    "Mod callback number exceeds binary32 range",
                    machine.program_source(),
                ));
            }
            machine.set_global_float(offset, *value)?;
        }
        ModRuntimeValue::Vector(value) => {
            if ![f64::from(value.x), f64::from(value.y), f64::from(value.z)]
                .iter()
                .all(|component| fround(*component).is_finite())
            {
                return Err(QcError::program(
                    "Mod callback vector exceeds binary32 range",
                    machine.program_source(),
                ));
            }
            machine.set_global_vector(offset, *value)?;
        }
        ModRuntimeValue::Str(value) => {
            let offset_int =
                machine.set_engine_string(&format!("mod-value:{}", value.0), &value.0, 128.max(value.0.len() + 1))?;
            machine.set_global_int(offset, offset_int)?;
        }
        ModRuntimeValue::Actor(value) => {
            machine.set_global_int(offset, reference(value.as_ref())?)?;
        }
    }
    Ok(())
}

/// Stage the declared original ABI only for one invocation, including
/// nested calls and faults (donor `withQcSourceCall`).
pub fn with_qc_source_call<R>(
    machine: &dyn QcMachineView,
    call: &ModSourceCall,
    inputs: &HashMap<ModCallbackInput, ModRuntimeValue>,
    reference: &dyn Fn(Option<&ActorId>) -> Result<i32, QcError>,
    execute: &mut dyn FnMut(usize) -> Result<R, QcError>,
) -> Result<R, QcError> {
    let resolve = |value: &ModCallbackValue| -> Result<ModRuntimeValue, QcError> {
        match value {
            ModCallbackValue::Input(name) => inputs.get(name).cloned().ok_or_else(|| {
                QcError::program(
                    format!("Gameplay callback {} has no {} input", call.function, input_name(*name)),
                    machine.program_source(),
                )
            }),
            ModCallbackValue::Float(value) => Ok(ModRuntimeValue::Float(*value)),
            ModCallbackValue::Str(value) => Ok(ModRuntimeValue::Str(value.clone())),
            ModCallbackValue::Vector(value) => Ok(ModRuntimeValue::Vector(*value)),
        }
    };
    let args = call.arguments.iter().map(resolve).collect::<Result<Vec<_>, _>>()?;
    let mut staged_globals = Vec::with_capacity(call.globals.len());
    for global in &call.globals {
        let value = resolve(&global.value)?;
        let definition = machine
            .global_definition(&global.name)
            .map_err(|_| QcError::program("Missing validated callback global", machine.program_source()))?;
        staged_globals.push((definition, value));
    }
    let staging = machine.staging_snapshot();
    let mut saved = Vec::with_capacity(staged_globals.len());
    for (definition, _) in &staged_globals {
        let width = usize::from(definition.def_type == QcValueType::Vector) * 2 + 1;
        saved.push((definition.offset, machine.global_range(definition.offset, width)?));
    }
    let result = (|| -> Result<R, QcError> {
        for (index, value) in args.iter().enumerate() {
            write_qc_source_value(machine, 4 + index * 3, value, reference)?;
        }
        for (definition, value) in &staged_globals {
            write_qc_source_value(machine, definition.offset, value, reference)?;
        }
        execute(args.len())
    })();
    machine.restore_staging(&staging);
    for (offset, bytes) in &saved {
        machine.set_global_range(*offset, bytes);
    }
    result
}
