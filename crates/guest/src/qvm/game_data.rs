//! Located QVM game/client tables plus the shared runtime mirrors used by this port.
//!
//! Provenance: `src/compat/qvm/game-data.ts` (`SV_LocateGameData`,
//! `SV_GentityNum`, `SV_GameClientNum`, `SV_NumForGentity`) for
//! [`QvmGameData`] and [`QvmGameDataState`].
//!
//! Local mirrors (other workers own the canonical modules; sibling
//! `client_state`/`legacy_presentation` ports use the same shapes, so the
//! parent can unify these by import path):
//!
//! - `src/contracts/execution.ts`: [`AbiProfile`], [`ModuleIdentity`],
//!   [`QvmCheckpoint`].
//! - `src/compat/qvm/image.ts`: [`QvmOpcode`], [`QvmInstruction`],
//!   [`QvmImage`], [`QvmArtifact`], [`QVM_MAX_PRIVATE_ARGUMENT_WORDS`].
//! - `src/compat/qvm/memory.ts`: [`QvmSharedMemory`], [`QvmMemoryWindow`],
//!   [`QvmWriteRange`], [`QvmCommittedWrite`].
//! - `src/compat/qvm/module.ts`: [`QvmModule`].
//! - `src/compat/qvm/interpreter.ts`: [`QvmFunctionCall`],
//!   [`QvmFunctionObservation`], [`QvmRegionControl`], [`QvmBranchBinding`],
//!   [`QvmRegionBinding`], [`QvmRegionEvaluation`].
//! - `src/compat/qvm/syscalls.ts`: [`QvmRole`], [`CallKind`], [`QvmHostCall`].
//! - `src/compat/qvm/abi.ts`: [`QvmGameImport`], [`QvmGameExport`],
//!   [`QvmCgameImport`], [`QvmCgameExport`].
//! - `src/compat/qvm/regions.ts`: [`qualify_qvm_region`],
//!   [`qualify_qvm_region_evaluation`].
//! - `src/compat/qvm/body-scope.ts`: [`qualify_qvm_body_calls`].
//! - `src/persistence/value.ts`: [`ProfileValue`], [`ProfileReader`],
//!   [`namespaced_id`].
//!
//! Borrowed `DataView` records from the donor become explicit
//! [`QvmMemoryWindow`] handles or owned structs in this port; every fallible
//! operation returns [`GuestError`] instead of throwing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::math::{vec3, Vec3};

use super::entity_record::qvm_entity_state_bytes;
use super::player_record::qvm_player_state_bytes;
use super::player_record::QvmPlayerState;
use super::shared_entity_record::qvm_shared_entity_bytes;
use super::shared_entity_record::QvmSharedEntity;
use crate::error::GuestError;

/// Selected QVM ABI: modern (`q3-modern`) or legacy (`q3-1.16n-base`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AbiProfile {
    /// Modern Quake III ABI.
    #[default]
    Modern,
    /// Legacy 1.16n/1.17 ABI.
    Legacy,
}

impl AbiProfile {
    /// Whether this is the modern ABI.
    #[must_use]
    pub const fn is_modern(self) -> bool {
        matches!(self, Self::Modern)
    }
}

/// Guest module identity: namespaced id plus artifact coordinates.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModuleIdentity {
    /// Namespaced provider id (`namespace:name`).
    pub id: String,
    /// Artifact path the module was loaded from.
    pub artifact_path: String,
    /// Content digest of the artifact.
    pub digest: String,
    /// Artifact revision.
    pub revision: String,
}

impl ModuleIdentity {
    /// Whether two identities name the same artifact revision.
    #[must_use]
    pub fn same_module(&self, other: &Self) -> bool {
        self.id == other.id
            && self.artifact_path == other.artifact_path
            && self.digest == other.digest
            && self.revision == other.revision
    }
}

/// Guest module role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QvmRole {
    /// Server game module.
    #[default]
    Qagame,
    /// Client game module.
    Cgame,
    /// User-interface module.
    Ui,
}

/// Engine trap versus raw extension call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    /// Classified engine trap.
    Engine,
    /// Unclassified raw trap number.
    Extension,
}

/// `OP_ARG` encodes a byte offset; aligned words occupy caller offsets 8
/// through 252, hence 62 addressable private argument words.
pub const QVM_MAX_PRIVATE_ARGUMENT_WORDS: usize = 62;

/// QVM opcodes in donor enum order (discriminants match `image.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum QvmOpcode {
    /// Undefined.
    OpUndef = 0,
    /// Ignored.
    OpIgnore,
    /// Breakpoint.
    OpBreak,
    /// Function entry.
    OpEnter,
    /// Function return.
    OpLeave,
    /// Call.
    OpCall,
    /// Push.
    OpPush,
    /// Pop.
    OpPop,
    /// Push constant.
    OpConst,
    /// Push local address.
    OpLocal,
    /// Jump.
    OpJump,
    /// Integer equality branch.
    OpEq,
    /// Integer inequality branch.
    OpNe,
    /// Signed less-than branch.
    OpLti,
    /// Signed less-or-equal branch.
    OpLei,
    /// Signed greater-than branch.
    OpGti,
    /// Signed greater-or-equal branch.
    OpGei,
    /// Unsigned less-than branch.
    OpLtu,
    /// Unsigned less-or-equal branch.
    OpLeu,
    /// Unsigned greater-than branch.
    OpGtu,
    /// Unsigned greater-or-equal branch.
    OpGeu,
    /// Float equality branch.
    OpEqf,
    /// Float inequality branch.
    OpNef,
    /// Float less-than branch.
    OpLtf,
    /// Float less-or-equal branch.
    OpLef,
    /// Float greater-than branch.
    OpGtf,
    /// Float greater-or-equal branch.
    OpGef,
    /// Load byte.
    OpLoad1,
    /// Load half word.
    OpLoad2,
    /// Load word.
    OpLoad4,
    /// Store byte.
    OpStore1,
    /// Store half word.
    OpStore2,
    /// Store word.
    OpStore4,
    /// Publish argument word.
    OpArg,
    /// Block copy.
    OpBlockCopy,
    /// Sign-extend byte.
    OpSex8,
    /// Sign-extend half word.
    OpSex16,
    /// Integer negate.
    OpNegi,
    /// Integer add.
    OpAdd,
    /// Integer subtract.
    OpSub,
    /// Signed integer divide.
    OpDivi,
    /// Unsigned integer divide.
    OpDivu,
    /// Signed integer modulo.
    OpModi,
    /// Unsigned integer modulo.
    OpModu,
    /// Integer multiply.
    OpMuli,
    /// Unsigned integer multiply.
    OpMulu,
    /// Bitwise and.
    OpBand,
    /// Bitwise or.
    OpBor,
    /// Bitwise xor.
    OpBxor,
    /// Bitwise complement.
    OpBcom,
    /// Logical shift left.
    OpLsh,
    /// Arithmetic shift right.
    OpRshi,
    /// Logical shift right.
    OpRshu,
    /// Float negate.
    OpNegf,
    /// Float add.
    OpAddf,
    /// Float subtract.
    OpSubf,
    /// Float divide.
    OpDivf,
    /// Float multiply.
    OpMulf,
    /// Integer to float.
    OpCvif,
    /// Float to integer.
    OpCvfi,
}

impl QvmOpcode {
    /// Whether this is a conditional branch (`OP_EQ..=OP_GEF`).
    #[must_use]
    pub const fn is_branch(self) -> bool {
        (self as u8) >= (Self::OpEq as u8) && (self as u8) <= (Self::OpGef as u8)
    }

    /// Encoded operand width: 4 for word opcodes, 1 for `OP_ARG`, else 0.
    #[must_use]
    pub const fn operand_width(self) -> u8 {
        match self {
            Self::OpEnter | Self::OpLeave | Self::OpConst | Self::OpLocal | Self::OpBlockCopy => 4,
            Self::OpArg => 1,
            _ if (self as u8) >= (Self::OpEq as u8) && (self as u8) <= (Self::OpGef as u8) => 4,
            _ => 0,
        }
    }
}

/// One decoded QVM instruction with its source program counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmInstruction {
    /// Operation.
    pub opcode: QvmOpcode,
    /// Decoded operand (meaningful when `operand_width` is nonzero).
    pub operand: i32,
    /// Encoded operand width in bytes.
    pub operand_width: u8,
    /// Offset from the code segment start (the source PC).
    pub byte_offset: usize,
}

impl QvmInstruction {
    /// Build a word-operand instruction.
    #[must_use]
    pub fn word(opcode: QvmOpcode, operand: i32, byte_offset: usize) -> Self {
        Self {
            opcode,
            operand,
            operand_width: 4,
            byte_offset,
        }
    }

    /// Build a single-byte instruction.
    #[must_use]
    pub fn single(opcode: QvmOpcode, byte_offset: usize) -> Self {
        Self {
            opcode,
            operand: 0,
            operand_width: 0,
            byte_offset,
        }
    }

    /// Build an `OP_ARG` instruction.
    #[must_use]
    pub fn arg(operand: i32, byte_offset: usize) -> Self {
        Self {
            opcode: QvmOpcode::OpArg,
            operand,
            operand_width: 1,
            byte_offset,
        }
    }
}

/// Decoded QVM image: code plus data-segment geometry.
#[derive(Debug, Clone, Default)]
pub struct QvmImage {
    /// Image source label.
    pub source: String,
    /// Decoded instructions in PC order.
    pub instructions: Vec<QvmInstruction>,
    /// Code segment offset.
    pub code_offset: usize,
    /// Code segment length.
    pub code_length: usize,
    /// Initialized data length.
    pub data_length: usize,
    /// Literal length.
    pub literal_length: usize,
    /// BSS length.
    pub bss_length: usize,
    /// Total allocated data length (data, literals, BSS, padding, stack).
    pub allocated_data_length: usize,
    /// Owned initialized data followed by literals.
    pub initialized_data: Vec<u8>,
    /// Data address mask.
    pub data_mask: usize,
}

impl QvmImage {
    /// End of declared source data (`data + literals + bss`).
    #[must_use]
    pub fn data_end(&self) -> usize {
        self.data_length + self.literal_length + self.bss_length
    }

    /// Fetch one instruction by index.
    #[must_use]
    pub fn instruction(&self, index: usize) -> Option<&QvmInstruction> {
        self.instructions.get(index)
    }

    /// Index one past the last instruction of the function entered at
    /// `entry` (the next `OP_ENTER` or the end of the image).
    #[must_use]
    pub fn function_end(&self, entry: usize) -> usize {
        let mut end = entry + 1;
        while end < self.instructions.len() && self.instructions[end].opcode != QvmOpcode::OpEnter {
            end += 1;
        }
        end
    }
}

/// Loaded module artifact: identity, role, ABI, and image.
#[derive(Debug, Clone)]
pub struct QvmArtifact {
    /// Module identity.
    pub module: ModuleIdentity,
    /// Module role.
    pub role: QvmRole,
    /// Declared ABI profile (`None` selects modern).
    pub abi_profile: Option<AbiProfile>,
    /// Decoded image.
    pub image: QvmImage,
}

/// Effective ABI profile of an artifact.
#[must_use]
pub fn artifact_abi(artifact: &QvmArtifact) -> AbiProfile {
    artifact.abi_profile.unwrap_or(AbiProfile::Modern)
}

/// Server-game trap codes in donor `QvmGameImport` order.
pub struct QvmGameImport;

impl QvmGameImport {
    /// Print trap.
    pub const G_PRINT: i32 = 0;
    /// Error trap.
    pub const G_ERROR: i32 = 1;
    /// Milliseconds trap.
    pub const G_MILLISECONDS: i32 = 2;
    /// Locate game-data tables trap.
    pub const G_LOCATE_GAME_DATA: i32 = 15;
    /// Drop-client trap.
    pub const G_DROP_CLIENT: i32 = 16;
    /// Send server command trap.
    pub const G_SEND_SERVER_COMMAND: i32 = 17;
    /// Set configstring trap.
    pub const G_SET_CONFIGSTRING: i32 = 18;
    /// Get configstring trap.
    pub const G_GET_CONFIGSTRING: i32 = 19;
    /// Get userinfo trap.
    pub const G_GET_USERINFO: i32 = 20;
    /// Set userinfo trap.
    pub const G_SET_USERINFO: i32 = 21;
    /// Get serverinfo trap.
    pub const G_GET_SERVERINFO: i32 = 22;
    /// Set brush model trap.
    pub const G_SET_BRUSH_MODEL: i32 = 23;
    /// Trace trap.
    pub const G_TRACE: i32 = 24;
    /// Point-contents trap.
    pub const G_POINT_CONTENTS: i32 = 25;
    /// Adjust area-portal state trap.
    pub const G_ADJUST_AREA_PORTAL_STATE: i32 = 28;
    /// Areas-connected trap.
    pub const G_AREAS_CONNECTED: i32 = 29;
    /// Link entity trap.
    pub const G_LINKENTITY: i32 = 30;
    /// Unlink entity trap.
    pub const G_UNLINKENTITY: i32 = 31;
    /// Entities-in-box trap.
    pub const G_ENTITIES_IN_BOX: i32 = 32;
    /// Entity-contact trap.
    pub const G_ENTITY_CONTACT: i32 = 33;
    /// Get user command trap.
    pub const G_GET_USERCMD: i32 = 36;
    /// Get entity token trap.
    pub const G_GET_ENTITY_TOKEN: i32 = 37;
    /// Capsule trace trap.
    pub const G_TRACECAPSULE: i32 = 43;
    /// Capsule entity-contact trap.
    pub const G_ENTITY_CONTACTCAPSULE: i32 = 44;
}

/// Server-game export codes in donor `QvmGameExport` order.
pub struct QvmGameExport;

impl QvmGameExport {
    /// Game init export.
    pub const GAME_INIT: i32 = 0;
    /// Game shutdown export.
    pub const GAME_SHUTDOWN: i32 = 1;
    /// Client connect export.
    pub const GAME_CLIENT_CONNECT: i32 = 2;
    /// Client begin export.
    pub const GAME_CLIENT_BEGIN: i32 = 3;
    /// Client userinfo-changed export.
    pub const GAME_CLIENT_USERINFO_CHANGED: i32 = 4;
    /// Client disconnect export.
    pub const GAME_CLIENT_DISCONNECT: i32 = 5;
    /// Client command export.
    pub const GAME_CLIENT_COMMAND: i32 = 6;
    /// Client think export.
    pub const GAME_CLIENT_THINK: i32 = 7;
    /// Run frame export.
    pub const GAME_RUN_FRAME: i32 = 8;
    /// Console command export.
    pub const GAME_CONSOLE_COMMAND: i32 = 9;
    /// Bot AI start-frame export.
    pub const BOTAI_START_FRAME: i32 = 10;
}

/// Client-game trap codes used by this port (donor `QvmCgameImport` order).
pub struct QvmCgameImport;

impl QvmCgameImport {
    /// Add reference entity to scene trap.
    pub const CG_R_ADDREFENTITYTOSCENE: i32 = 41;
    /// Get game state trap.
    pub const CG_GETGAMESTATE: i32 = 50;
    /// Get current snapshot number trap.
    pub const CG_GETCURRENTSNAPSHOTNUMBER: i32 = 51;
    /// Get snapshot trap.
    pub const CG_GETSNAPSHOT: i32 = 52;
    /// Get server command trap.
    pub const CG_GETSERVERCOMMAND: i32 = 53;
}

/// Client-game export codes in donor `QvmCgameExport` order.
pub struct QvmCgameExport;

impl QvmCgameExport {
    /// Cgame init export.
    pub const CG_INIT: i32 = 0;
    /// Cgame shutdown export.
    pub const CG_SHUTDOWN: i32 = 1;
    /// Console command export.
    pub const CG_CONSOLE_COMMAND: i32 = 2;
    /// Draw active frame export.
    pub const CG_DRAW_ACTIVE_FRAME: i32 = 3;
    /// Crosshair player export.
    pub const CG_CROSSHAIR_PLAYER: i32 = 4;
    /// Last attacker export.
    pub const CG_LAST_ATTACKER: i32 = 5;
    /// Key event export.
    pub const CG_KEY_EVENT: i32 = 6;
    /// Mouse event export.
    pub const CG_MOUSE_EVENT: i32 = 7;
    /// Event handling export.
    pub const CG_EVENT_HANDLING: i32 = 8;
}

/// One observed memory range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmWriteRange {
    /// Range start as an allocation offset.
    pub byte_offset: usize,
    /// Range length in bytes.
    pub byte_length: usize,
}

/// Before/after bytes of one intersecting store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmWriteRangeEvent {
    /// Store start as an allocation offset.
    pub byte_offset: usize,
    /// Bytes before the store.
    pub before: Vec<u8>,
    /// Bytes after the store.
    pub after: Vec<u8>,
}

/// Committed stores delivered to write observers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QvmCommittedWrite {
    /// Intersecting stores in mutation order.
    pub ranges: Vec<QvmWriteRangeEvent>,
}

impl QvmCommittedWrite {
    /// Whether any store intersects `range`.
    #[must_use]
    pub fn touches(&self, range: &QvmWriteRange) -> bool {
        self.ranges.iter().any(|write| {
            write.byte_offset < range.byte_offset + range.byte_length
                && range.byte_offset < write.byte_offset + write.after.len()
        })
    }
}

/// Write-observer callback.
pub type QvmWatchCallback = Rc<dyn Fn(&QvmCommittedWrite)>;

struct QvmWatch {
    id: u64,
    ranges: Vec<QvmWriteRange>,
    publish: QvmWatchCallback,
    commit: Option<QvmWatchCallback>,
}

struct QvmSharedMemoryInner {
    bytes: Vec<u8>,
    live: bool,
    next_watch: u64,
    watches: Vec<QvmWatch>,
}

/// Shared guest allocation with write observers (mirror of `QvmMemory`).
///
/// Clones share one allocation so located tables, module hooks, and fixtures
/// observe the same bytes. Method names align with the sibling
/// `client_state::SyscallMemory` port where the surface overlaps.
#[derive(Clone)]
pub struct QvmSharedMemory {
    inner: Rc<RefCell<QvmSharedMemoryInner>>,
}

impl std::fmt::Debug for QvmSharedMemory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.borrow();
        formatter
            .debug_struct("QvmSharedMemory")
            .field("len", &inner.bytes.len())
            .field("live", &inner.live)
            .field("watches", &inner.watches.len())
            .finish()
    }
}

impl QvmSharedMemory {
    /// Allocate a zeroed guest allocation of `byte_len` bytes.
    pub fn new(byte_len: usize) -> Result<Self, GuestError> {
        if byte_len == 0 {
            return Err(GuestError::invalid("QVM allocation requires a nonzero length"));
        }
        Ok(Self {
            inner: Rc::new(RefCell::new(QvmSharedMemoryInner {
                bytes: vec![0; byte_len],
                live: true,
                next_watch: 1,
                watches: Vec::new(),
            })),
        })
    }

    /// Wrap existing bytes as the guest allocation.
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(QvmSharedMemoryInner {
                bytes,
                live: true,
                next_watch: 1,
                watches: Vec::new(),
            })),
        }
    }

    /// Allocation length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.borrow().bytes.len()
    }

    /// Whether the allocation is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the allocation is live.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.inner.borrow().live
    }

    /// Fail when the allocation is retired.
    pub fn assert_live(&self) -> Result<(), GuestError> {
        if self.is_live() {
            Ok(())
        } else {
            Err(GuestError::invalid("QVM memory is retired"))
        }
    }

    /// Retire the allocation; further access fails.
    pub fn close(&self) {
        self.inner.borrow_mut().live = false;
    }

    fn check_span(&self, offset: usize, len: usize) -> Result<usize, GuestError> {
        self.assert_live()?;
        let end = offset
            .checked_add(len)
            .ok_or_else(|| GuestError::invalid("QVM memory span exceeds the allocation"))?;
        if end > self.len() {
            return Err(GuestError::invalid("QVM memory span exceeds the allocation"));
        }
        Ok(end)
    }

    /// Decode a guest word: 0 is null, otherwise an allocation offset.
    #[must_use]
    pub fn pointer(&self, word: i32) -> Option<usize> {
        if word <= 0 {
            return None;
        }
        let offset = word as usize;
        if offset >= self.len() {
            return None;
        } else {
            Some(offset)
        }
    }

    /// Resolve a guest word plus length to an allocation range.
    pub fn span(&self, word: i32, len: usize) -> Result<std::ops::Range<usize>, GuestError> {
        let Some(offset) = self.pointer(word) else {
            return Err(GuestError::invalid("QVM memory access through a null pointer"));
        };
        let end = self.check_span(offset, len)?;
        Ok(offset..end)
    }

    /// Read one byte.
    pub fn get(&self, offset: usize) -> Result<u8, GuestError> {
        self.check_span(offset, 1)?;
        Ok(self.inner.borrow().bytes[offset])
    }

    /// Write one byte.
    pub fn set(&self, offset: usize, value: u8) -> Result<(), GuestError> {
        self.mutate(offset, 1, |bytes| bytes[0] = value)
    }

    /// Read a little-endian `i32`.
    pub fn read_i32(&self, offset: usize) -> Result<i32, GuestError> {
        self.check_span(offset, 4)?;
        let inner = self.inner.borrow();
        let mut word = [0u8; 4];
        word.copy_from_slice(&inner.bytes[offset..offset + 4]);
        Ok(i32::from_le_bytes(word))
    }

    /// Write a little-endian `i32`.
    pub fn write_i32(&self, offset: usize, value: i32) -> Result<(), GuestError> {
        let bytes = value.to_le_bytes();
        self.mutate(offset, 4, |slot| slot.copy_from_slice(&bytes))
    }

    /// Read a little-endian `u16`.
    pub fn read_u16(&self, offset: usize) -> Result<u16, GuestError> {
        self.check_span(offset, 2)?;
        let inner = self.inner.borrow();
        let mut word = [0u8; 2];
        word.copy_from_slice(&inner.bytes[offset..offset + 2]);
        Ok(u16::from_le_bytes(word))
    }

    /// Write a little-endian `u16`.
    pub fn write_u16(&self, offset: usize, value: u16) -> Result<(), GuestError> {
        let bytes = value.to_le_bytes();
        self.mutate(offset, 2, |slot| slot.copy_from_slice(&bytes))
    }

    /// Read a little-endian `f32`.
    pub fn read_f32(&self, offset: usize) -> Result<f32, GuestError> {
        self.check_span(offset, 4)?;
        let inner = self.inner.borrow();
        let mut word = [0u8; 4];
        word.copy_from_slice(&inner.bytes[offset..offset + 4]);
        Ok(f32::from_le_bytes(word))
    }

    /// Write a little-endian `f32`.
    pub fn write_f32(&self, offset: usize, value: f32) -> Result<(), GuestError> {
        let bytes = value.to_le_bytes();
        self.mutate(offset, 4, |slot| slot.copy_from_slice(&bytes))
    }

    /// Read a signed byte.
    pub fn read_i8(&self, offset: usize) -> Result<i8, GuestError> {
        Ok(self.get(offset)? as i8)
    }

    /// Write a signed byte.
    pub fn write_i8(&self, offset: usize, value: i8) -> Result<(), GuestError> {
        self.set(offset, value as u8)
    }

    /// Read a three-component vector.
    pub fn read_vec3(&self, offset: usize) -> Result<Vec3, GuestError> {
        Ok(vec3(
            self.read_f32(offset)?,
            self.read_f32(offset + 4)?,
            self.read_f32(offset + 8)?,
        ))
    }

    /// Write a three-component vector.
    pub fn write_vec3(&self, offset: usize, value: &Vec3) -> Result<(), GuestError> {
        self.write_f32(offset, value.x)?;
        self.write_f32(offset + 4, value.y)?;
        self.write_f32(offset + 8, value.z)?;
        Ok(())
    }

    /// Read a vector through a guest pointer word (null reads zero).
    pub fn read_vec3_ptr(&self, word: i32) -> Result<Vec3, GuestError> {
        match self.pointer(word) {
            None => Ok(vec3(0.0, 0.0, 0.0)),
            Some(offset) => self.read_vec3(offset),
        }
    }

    /// Copy bytes out of the allocation.
    pub fn read_bytes(&self, offset: usize, len: usize) -> Result<Vec<u8>, GuestError> {
        let end = self.check_span(offset, len)?;
        Ok(self.inner.borrow().bytes[offset..end].to_vec())
    }

    /// Copy bytes into the allocation.
    pub fn write_bytes(&self, offset: usize, bytes: &[u8]) -> Result<(), GuestError> {
        self.mutate(offset, bytes.len(), |slot| slot.copy_from_slice(bytes))
    }

    /// Fill a span with one byte value.
    pub fn fill(&self, offset: usize, len: usize, value: u8) -> Result<(), GuestError> {
        self.mutate(offset, len, |slot| slot.fill(value))
    }

    /// Copy bytes within the allocation.
    pub fn copy_bytes(&self, destination: usize, source: usize, len: usize) -> Result<(), GuestError> {
        let bytes = self.read_bytes(source, len)?;
        self.write_bytes(destination, &bytes)
    }

    /// Read a NUL-terminated Latin-1 string through a guest pointer.
    pub fn read_string(&self, word: i32) -> Result<String, GuestError> {
        let Some(offset) = self.pointer(word) else {
            return Err(GuestError::invalid("QVM string read through a null pointer"));
        };
        self.assert_live()?;
        let inner = self.inner.borrow();
        let mut text = String::new();
        let mut cursor = offset;
        while cursor < inner.bytes.len() {
            let byte = inner.bytes[cursor];
            if byte == 0 {
                return Ok(text);
            }
            text.push(byte as char);
            cursor += 1;
        }
        Err(GuestError::invalid("QVM string is unterminated"))
    }

    /// Write `text` with `qStrncpyz` semantics: at most `capacity - 1` bytes
    /// plus a NUL terminator.
    pub fn write_string(&self, word: i32, text: &str, capacity: usize) -> Result<(), GuestError> {
        let Some(offset) = self.pointer(word) else {
            return Err(GuestError::invalid("QVM string write through a null pointer"));
        };
        if capacity == 0 {
            return Err(GuestError::invalid("QVM string write requires a nonzero capacity"));
        }
        self.check_span(offset, capacity)?;
        let bytes = text.as_bytes();
        let count = bytes.len().min(capacity - 1);
        self.mutate(offset, capacity, |slot| {
            slot[..count].copy_from_slice(&bytes[..count]);
            slot[count] = 0;
        })
    }

    /// Bounded string write: capacity must fit the allocation.
    pub fn write_bounded_string(&self, word: i32, text: &str, capacity: usize) -> Result<(), GuestError> {
        self.write_string(word, text, capacity)
    }

    /// Mutate a span, then publish one committed write to intersecting
    /// observers (publish callbacks first, commit callbacks second).
    pub fn mutate(&self, offset: usize, len: usize, apply: impl FnOnce(&mut [u8])) -> Result<(), GuestError> {
        let end = self.check_span(offset, len)?;
        let before = self.inner.borrow().bytes[offset..end].to_vec();
        {
            let mut inner = self.inner.borrow_mut();
            apply(&mut inner.bytes[offset..end]);
        }
        let after = self.inner.borrow().bytes[offset..end].to_vec();
        if before == after {
            return Ok(());
        }
        let event = QvmCommittedWrite {
            ranges: vec![QvmWriteRangeEvent {
                byte_offset: offset,
                before,
                after,
            }],
        };
        let callbacks: Vec<(QvmWatchCallback, Option<QvmWatchCallback>)> = self
            .inner
            .borrow()
            .watches
            .iter()
            .filter(|watch| watch.ranges.iter().any(|range| event.touches(range)))
            .map(|watch| (Rc::clone(&watch.publish), watch.commit.clone()))
            .collect();
        for (publish, _) in &callbacks {
            publish(&event);
        }
        for (_, commit) in &callbacks {
            if let Some(commit) = commit {
                commit(&event);
            }
        }
        Ok(())
    }

    /// Observe stores intersecting `ranges`; returns a watch id for removal.
    pub fn observe_writes(
        &self,
        ranges: Vec<QvmWriteRange>,
        publish: QvmWatchCallback,
        commit: Option<QvmWatchCallback>,
    ) -> u64 {
        let mut inner = self.inner.borrow_mut();
        let id = inner.next_watch;
        inner.next_watch += 1;
        inner.watches.push(QvmWatch {
            id,
            ranges,
            publish,
            commit,
        });
        id
    }

    /// Remove a write observer; returns whether one was present.
    pub fn remove_observer(&self, id: u64) -> bool {
        let mut inner = self.inner.borrow_mut();
        let before = inner.watches.len();
        inner.watches.retain(|watch| watch.id != id);
        inner.watches.len() != before
    }

    /// Remove all write observers.
    pub fn clear_observers(&self) {
        self.inner.borrow_mut().watches.clear();
    }
}

/// Live window into shared guest memory (mirror of a `DataView` slice).
#[derive(Debug, Clone)]
pub struct QvmMemoryWindow {
    /// Backing allocation.
    pub memory: QvmSharedMemory,
    /// Window start as an allocation offset.
    pub offset: usize,
    /// Window length in bytes.
    pub len: usize,
}

impl QvmMemoryWindow {
    /// Open a window over `memory`.
    pub fn new(memory: QvmSharedMemory, offset: usize, len: usize) -> Result<Self, GuestError> {
        memory.check_span(offset, len)?;
        Ok(Self { memory, offset, len })
    }

    /// Whether the window is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn at(&self, offset: usize, len: usize) -> Result<usize, GuestError> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| GuestError::invalid("QVM window access exceeds its extent"))?;
        if end > self.len {
            return Err(GuestError::invalid("QVM window access exceeds its extent"));
        }
        Ok(self.offset + offset)
    }

    /// Read a little-endian `i32` at a window-relative offset.
    pub fn get_i32(&self, offset: usize) -> Result<i32, GuestError> {
        self.memory.read_i32(self.at(offset, 4)?)
    }

    /// Write a little-endian `i32` at a window-relative offset.
    pub fn set_i32(&self, offset: usize, value: i32) -> Result<(), GuestError> {
        self.memory.write_i32(self.at(offset, 4)?, value)
    }

    /// Read a little-endian `f32` at a window-relative offset.
    pub fn get_f32(&self, offset: usize) -> Result<f32, GuestError> {
        self.memory.read_f32(self.at(offset, 4)?)
    }

    /// Write a little-endian `f32` at a window-relative offset.
    pub fn set_f32(&self, offset: usize, value: f32) -> Result<(), GuestError> {
        self.memory.write_f32(self.at(offset, 4)?, value)
    }

    /// Read a byte at a window-relative offset.
    pub fn get_u8(&self, offset: usize) -> Result<u8, GuestError> {
        self.memory.get(self.at(offset, 1)?)
    }

    /// Write a byte at a window-relative offset.
    pub fn set_u8(&self, offset: usize, value: u8) -> Result<(), GuestError> {
        self.memory.set(self.at(offset, 1)?, value)
    }

    /// Read a signed byte at a window-relative offset.
    pub fn get_i8(&self, offset: usize) -> Result<i8, GuestError> {
        self.memory.read_i8(self.at(offset, 1)?)
    }

    /// Write a signed byte at a window-relative offset.
    pub fn set_i8(&self, offset: usize, value: i8) -> Result<(), GuestError> {
        self.memory.write_i8(self.at(offset, 1)?, value)
    }

    /// Read a vector at a window-relative offset.
    pub fn get_vec3(&self, offset: usize) -> Result<Vec3, GuestError> {
        self.memory.read_vec3(self.at(offset, 12)?)
    }

    /// Write a vector at a window-relative offset.
    pub fn set_vec3(&self, offset: usize, value: &Vec3) -> Result<(), GuestError> {
        self.memory.write_vec3(self.at(offset, 12)?, value)
    }

    /// Copy window bytes out.
    pub fn copy_bytes(&self, offset: usize, len: usize) -> Result<Vec<u8>, GuestError> {
        self.memory.read_bytes(self.at(offset, len)?, len)
    }
}

/// Decoded host call: trap words plus routing metadata.
#[derive(Debug, Clone)]
pub struct QvmHostCall {
    /// Engine or extension classification.
    pub kind: CallKind,
    /// Calling module role.
    pub role: QvmRole,
    /// Trap code.
    pub code: i32,
    /// Live words: trap number at index 0, arguments after.
    pub words: Vec<i32>,
    /// Guest allocation.
    pub guest: QvmSharedMemory,
    /// Module ABI profile.
    pub abi_profile: AbiProfile,
    /// Console-command arguments, when the trap runs under one.
    pub command_arguments: Option<Vec<String>>,
}

impl QvmHostCall {
    /// Read word `index` as `i32`.
    pub fn int(&self, index: usize) -> Result<i32, GuestError> {
        self.words
            .get(index)
            .copied()
            .ok_or_else(|| GuestError::invalid("QVM host call word is outside its frame"))
    }

    /// Read word `index` as `f32`.
    pub fn float(&self, index: usize) -> Result<f32, GuestError> {
        Ok(f32::from_bits(self.int(index)? as u32))
    }
}

/// Host trap handler: `None` declines, `Some` supplies the trap result.
pub type QvmHostFn = Rc<dyn Fn(&QvmHostCall) -> Result<Option<i32>, GuestError>>;

/// Cancellation scope token (mirror of `QvmCancellationScope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmCancellationScope;

/// Region control for branch/region callbacks.
#[derive(Debug, Default)]
pub struct QvmRegionControl {
    /// Local words visible to the region.
    pub locals: HashMap<usize, i32>,
    /// Whether the region cancelled its invocation.
    pub cancelled: bool,
}

impl QvmRegionControl {
    /// Read a region local word.
    pub fn local_word(&self, offset: usize) -> Result<i32, GuestError> {
        self.locals
            .get(&offset)
            .copied()
            .ok_or_else(|| GuestError::invalid("QVM region local is not initialized"))
    }

    /// Cancel the enclosing invocation (records the request; the caller
    /// unwinds by returning immediately).
    pub fn cancel_function(&mut self) {
        self.cancelled = true;
    }
}

/// Branch decision callback: receives the original direction plus control.
pub type QvmBranchDecide = Box<dyn Fn(bool, &mut QvmRegionControl) -> bool>;

/// One bound original conditional decision.
pub struct QvmBranchBinding {
    /// Conditional instruction index.
    pub instruction_index: usize,
    /// Decision callback.
    pub decide: QvmBranchDecide,
}

/// Region outcome: run the original region or skip it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmRegionDecision {
    /// Run the original region.
    Execute,
    /// Skip the original region.
    Skip,
}

/// Region entry callback.
pub type QvmRegionRun = Box<dyn FnMut(&mut QvmRegionControl) -> QvmRegionDecision>;

/// Region completion callback.
pub type QvmRegionCompleted = Box<dyn FnMut(&mut QvmRegionControl)>;

/// One bound original region.
pub struct QvmRegionBinding {
    /// Region entry instruction.
    pub entry: usize,
    /// Region join instruction.
    pub join: usize,
    /// Entry callback.
    pub run: QvmRegionRun,
    /// Completion callback.
    pub completed: Option<QvmRegionCompleted>,
}

/// Standalone region evaluation request (mirror of `QvmRegionEvaluation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmRegionEvaluation {
    /// Region entry instruction.
    pub entry: usize,
    /// Region join instruction.
    pub join: usize,
    /// Local offsets supplied as live-ins.
    pub inputs: Vec<usize>,
    /// Local offset read as the result, if any.
    pub result: Option<usize>,
}

/// Region access granted to an evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmRegionAccess {
    /// Full source access.
    Source,
    /// Locals-only writes, no calls.
    ReadOnly,
}

/// Live source function invocation (mirror of `QvmFunctionCall`).
///
/// The mirror is synchronous: donor `proceedAsync` paths collapse onto
/// [`QvmFunctionCall::proceed`], and `cancelFunction` records the unwind and
/// returns its zero value so ported code returns immediately instead of
/// throwing across an interpreter boundary.
pub struct QvmFunctionCall {
    /// Entered instruction index.
    pub instruction_index: usize,
    /// Exact original `OP_CALL` site, or `None` for host-entered calls.
    pub caller_instruction: Option<usize>,
    /// Live argument words.
    pub words: Vec<i32>,
    /// Allocation-relative stack address of word zero.
    pub stack_address: usize,
    /// Full guest allocation.
    pub memory: QvmSharedMemory,
    /// Guest allocation handle (same allocation as `memory`).
    pub guest: QvmSharedMemory,
    /// Value the next [`QvmFunctionCall::proceed`] returns.
    pub proceed_value: i32,
    /// Whether [`QvmFunctionCall::proceed`] ran.
    pub proceeded: bool,
    /// Whether [`QvmFunctionCall::cancel_function`] ran.
    pub cancelled: bool,
    /// Bound branch decisions.
    pub branch_bindings: Vec<QvmBranchBinding>,
    /// Bound regions.
    pub region_bindings: Vec<QvmRegionBinding>,
    /// Recorded region evaluations.
    pub evaluated: Vec<(QvmRegionEvaluation, Vec<i32>)>,
    /// Value region evaluations return.
    pub eval_value: i32,
    /// Delivered host effects.
    pub effects: usize,
}

impl QvmFunctionCall {
    /// Build a host-entered call with `words` argument words.
    #[must_use]
    pub fn entered(instruction_index: usize, words: Vec<i32>, memory: QvmSharedMemory) -> Self {
        Self {
            instruction_index,
            caller_instruction: None,
            words,
            stack_address: 0,
            memory: memory.clone(),
            guest: memory,
            proceed_value: 0,
            proceeded: false,
            cancelled: false,
            branch_bindings: Vec::new(),
            region_bindings: Vec::new(),
            evaluated: Vec::new(),
            eval_value: 0,
            effects: 0,
        }
    }

    /// Read argument word `index`.
    pub fn argument(&self, index: usize) -> Result<i32, GuestError> {
        self.words
            .get(index)
            .copied()
            .ok_or_else(|| GuestError::invalid("QVM call argument is outside its frame"))
    }

    /// Run the original body once; returns the canned fixture value.
    pub fn proceed(&mut self) -> i32 {
        self.proceeded = true;
        self.proceed_value
    }

    /// Deliver a synchronous host effect under this invocation.
    pub fn effect(&mut self, perform: impl FnOnce()) {
        perform();
        self.effects += 1;
    }

    /// Open a cancellation scope for this invocation.
    #[must_use]
    pub fn cancellation_scope(&self) -> QvmCancellationScope {
        QvmCancellationScope
    }

    /// Bind original conditional decisions for this invocation.
    pub fn branches(&mut self, bindings: Vec<QvmBranchBinding>) {
        self.branch_bindings.extend(bindings);
    }

    /// Bind original regions for this invocation.
    pub fn regions(&mut self, bindings: Vec<QvmRegionBinding>) {
        self.region_bindings.extend(bindings);
    }

    /// Run a qualified standalone region; returns the canned fixture value.
    pub fn evaluate_region(&mut self, region: &QvmRegionEvaluation, inputs: &[i32]) -> i32 {
        self.evaluated.push((region.clone(), inputs.to_vec()));
        self.eval_value
    }

    /// Cancel this invocation: records the unwind and returns its zero value.
    pub fn cancel_function(&mut self) -> i32 {
        self.cancelled = true;
        0
    }

    /// Drive branch binding `index` with an original direction (fixture aid).
    pub fn decide_branch(&mut self, index: usize, taken: bool) -> bool {
        let mut control = QvmRegionControl::default();
        if let Some(binding) = self.branch_bindings.get_mut(index) {
            (binding.decide)(taken, &mut control)
        } else {
            taken
        }
    }
}

/// Non-replacing function observation (mirror of `QvmFunctionObservation`).
#[derive(Debug)]
pub struct QvmFunctionObservation {
    /// Observed argument words.
    pub words: Vec<i32>,
    /// Whether the observation cancelled its invocation.
    pub cancelled: bool,
}

impl QvmFunctionObservation {
    /// Read argument word `index`.
    pub fn argument(&self, index: usize) -> Result<i32, GuestError> {
        self.words
            .get(index)
            .copied()
            .ok_or_else(|| GuestError::invalid("QVM observation argument is outside its frame"))
    }

    /// Cancel the observed invocation.
    pub fn cancel_function(&mut self) {
        self.cancelled = true;
    }
}

/// Function hook: receives the live call, returns the trap result.
pub type QvmHookFn = Rc<dyn Fn(&mut QvmFunctionCall) -> i32>;

/// Hook resolver consulted when no direct hook is bound.
pub type QvmHookResolver = Rc<dyn Fn(usize, i32, &[i32]) -> Option<QvmHookFn>>;

/// Function observer.
pub type QvmObserveFn = Rc<dyn Fn(&mut QvmFunctionObservation)>;

/// Recorded host call into the module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmModuleCall {
    /// Argument words.
    pub words: Vec<i32>,
    /// Entered instruction index.
    pub entry: usize,
}

/// Recorded console-command call into the module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmModuleCommand {
    /// Argument words.
    pub words: Vec<i32>,
    /// Command arguments.
    pub arguments: Vec<String>,
}

/// Host-state checkpoint callbacks (mirror of `QvmHostState`).
#[derive(Clone)]
pub struct QvmHostStateFns {
    /// Capture host state as a format tag plus value tree.
    pub checkpoint: Rc<dyn Fn() -> (String, ProfileValue)>,
    /// Restore host state from a value tree.
    pub restore: Rc<dyn Fn(&ProfileValue) -> Result<(), GuestError>>,
}

/// Module checkpoint (mirror of `QvmCheckpoint`).
#[derive(Debug, Clone)]
pub struct QvmCheckpoint {
    /// Module identity.
    pub module: ModuleIdentity,
    /// API kind tag.
    pub api_kind: String,
    /// API version.
    pub api_version: i32,
    /// ABI profile.
    pub abi_profile: AbiProfile,
    /// Full data allocation bytes.
    pub data: Vec<u8>,
    /// Suspended instruction index.
    pub instruction_index: usize,
    /// Suspended operand stack.
    pub operand_stack: Vec<i32>,
    /// Suspended program stack pointer.
    pub program_stack: usize,
    /// Host state.
    pub host_state: QvmHostState,
}

/// Host-state checkpoint (mirror of `GuestPrivateState`).
#[derive(Debug, Clone)]
pub struct QvmHostState {
    /// Owning module.
    pub module: ModuleIdentity,
    /// State format tag.
    pub format: String,
    /// State value tree.
    pub bytes: ProfileValue,
}

struct QvmHookEntry {
    id: u64,
    entry: usize,
    hook: QvmHookFn,
}

struct QvmModuleInner {
    artifact: QvmArtifact,
    memory: QvmSharedMemory,
    retired: bool,
    next_hook: u64,
    hooks: Vec<QvmHookEntry>,
    resolver: Option<QvmHookResolver>,
    observers: Vec<QvmHookEntryObserver>,
    calls: Vec<QvmModuleCall>,
    commands: Vec<QvmModuleCommand>,
    host: Option<QvmHostFn>,
    host_state: Option<QvmHostStateFns>,
    default_return: i32,
    source_callback_value: i32,
    region_value: i32,
    counter_value: i32,
    cancel_requests: usize,
    source_callbacks: Vec<(usize, Vec<i32>)>,
}

struct QvmHookEntryObserver {
    id: u64,
    entry: usize,
    observer: QvmObserveFn,
}

/// Guest module handle (mirror of `QvmModule`).
///
/// The mirror records calls and drives bound hooks with fabricated
/// [`QvmFunctionCall`] values; it executes no guest code. Both
/// `bind_function` and `bind_invocation` hooks fire on host calls (the real
/// interpreter distinguishes host-entered from VM-entered invocations).
#[derive(Clone)]
pub struct QvmModule {
    inner: Rc<RefCell<QvmModuleInner>>,
}

impl std::fmt::Debug for QvmModule {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.borrow();
        formatter
            .debug_struct("QvmModule")
            .field("module", &inner.artifact.module.id)
            .field("role", &inner.artifact.role)
            .field("retired", &inner.retired)
            .field("calls", &inner.calls.len())
            .finish()
    }
}

impl QvmModule {
    /// Build a module over a fresh zeroed allocation.
    pub fn new(
        artifact: QvmArtifact,
        host: Option<QvmHostFn>,
        host_state: Option<QvmHostStateFns>,
    ) -> Result<Self, GuestError> {
        let len = artifact.image.allocated_data_length.max(64);
        let memory = QvmSharedMemory::new(len)?;
        if !artifact.image.initialized_data.is_empty() {
            let count = artifact.image.initialized_data.len().min(memory.len());
            memory.write_bytes(0, &artifact.image.initialized_data[..count])?;
        }
        Ok(Self {
            inner: Rc::new(RefCell::new(QvmModuleInner {
                artifact,
                memory,
                retired: false,
                next_hook: 1,
                hooks: Vec::new(),
                resolver: None,
                observers: Vec::new(),
                calls: Vec::new(),
                commands: Vec::new(),
                host,
                host_state,
                default_return: 0,
                source_callback_value: 0,
                region_value: 0,
                counter_value: 0,
                cancel_requests: 0,
                source_callbacks: Vec::new(),
            })),
        })
    }

    /// Effective ABI profile.
    #[must_use]
    pub fn abi_profile(&self) -> AbiProfile {
        let inner = self.inner.borrow();
        inner.artifact.abi_profile.unwrap_or(AbiProfile::Modern)
    }

    /// Module identity.
    #[must_use]
    pub fn module_id(&self) -> ModuleIdentity {
        self.inner.borrow().artifact.module.clone()
    }

    /// Module role.
    #[must_use]
    pub fn role(&self) -> QvmRole {
        self.inner.borrow().artifact.role
    }

    /// Shared guest allocation.
    #[must_use]
    pub fn memory(&self) -> QvmSharedMemory {
        self.inner.borrow().memory.clone()
    }

    /// Whether the module is retired.
    #[must_use]
    pub fn is_retired(&self) -> bool {
        self.inner.borrow().retired
    }

    /// Fail when retired.
    pub fn assert_live(&self) -> Result<(), GuestError> {
        if self.is_retired() {
            return Err(GuestError::invalid("QVM module has been retired"));
        }
        self.memory().assert_live()
    }

    /// Retire the module and its allocation.
    pub fn retire(&self) {
        self.inner.borrow_mut().retired = true;
        self.memory().close();
    }

    /// Restart guest data from `bytes`, preserving hooks.
    pub fn restart(&self, bytes: &[u8]) -> Result<(), GuestError> {
        self.assert_live()?;
        let memory = self.memory();
        if bytes.len() != memory.len() {
            return Err(GuestError::invalid("QVM restart image differs from the allocation"));
        }
        memory.write_bytes(0, bytes)
    }

    /// Bind a function hook; returns a hook id for removal.
    pub fn bind_function(&self, entry: usize, hook: QvmHookFn) -> u64 {
        let mut inner = self.inner.borrow_mut();
        let id = inner.next_hook;
        inner.next_hook += 1;
        inner.hooks.push(QvmHookEntry { id, entry, hook });
        id
    }

    /// Bind an invocation hook; returns a hook id for removal.
    pub fn bind_invocation(&self, entry: usize, hook: QvmHookFn) -> u64 {
        self.bind_function(entry, hook)
    }

    /// Install the hook resolver consulted when no direct hook is bound.
    pub fn bind_function_resolver(&self, resolver: QvmHookResolver) {
        self.inner.borrow_mut().resolver = Some(resolver);
    }

    /// Observe calls without replacing them; returns an observer id.
    pub fn observe_function(&self, entry: usize, observer: QvmObserveFn) -> u64 {
        let mut inner = self.inner.borrow_mut();
        let id = inner.next_hook;
        inner.next_hook += 1;
        inner.observers.push(QvmHookEntryObserver { id, entry, observer });
        id
    }

    /// Remove a hook or observer by id.
    pub fn remove_hook(&self, id: u64) -> bool {
        let mut inner = self.inner.borrow_mut();
        let hooks = inner.hooks.len();
        let observers = inner.observers.len();
        inner.hooks.retain(|hook| hook.id != id);
        inner.observers.retain(|observer| observer.id != id);
        inner.hooks.len() != hooks || inner.observers.len() != observers
    }

    /// Call a source function, driving bound hooks and observers.
    pub fn call(&self, words: &[i32], entry: usize) -> Result<i32, GuestError> {
        self.assert_live()?;
        self.inner.borrow_mut().calls.push(QvmModuleCall {
            words: words.to_vec(),
            entry,
        });
        let memory = self.memory();
        let hooks: Vec<QvmHookFn> = {
            let inner = self.inner.borrow();
            let mut hooks: Vec<QvmHookFn> = inner
                .hooks
                .iter()
                .filter(|hook| hook.entry == entry)
                .map(|hook| Rc::clone(&hook.hook))
                .collect();
            if hooks.is_empty() {
                if let Some(resolver) = inner.resolver.clone() {
                    let first = words.first().copied().unwrap_or(0);
                    if let Some(hook) = resolver(entry, first, words) {
                        hooks.push(hook);
                    }
                }
            }
            hooks
        };
        let observers: Vec<QvmObserveFn> = self
            .inner
            .borrow()
            .observers
            .iter()
            .filter(|observer| observer.entry == entry)
            .map(|observer| Rc::clone(&observer.observer))
            .collect();
        let mut observation = QvmFunctionObservation {
            words: words.to_vec(),
            cancelled: false,
        };
        for observer in observers {
            observer(&mut observation);
        }
        if hooks.is_empty() {
            return Ok(self.inner.borrow().default_return);
        }
        let mut call = QvmFunctionCall::entered(entry, words.to_vec(), memory);
        let mut result = 0;
        for hook in hooks {
            result = hook(&mut call);
        }
        Ok(result)
    }

    /// Call a source console command.
    pub fn command(&self, words: &[i32], arguments: &[String]) -> Result<i32, GuestError> {
        self.assert_live()?;
        self.inner.borrow_mut().commands.push(QvmModuleCommand {
            words: words.to_vec(),
            arguments: arguments.to_vec(),
        });
        Ok(self.inner.borrow().default_return)
    }

    /// Recorded host calls.
    #[must_use]
    pub fn calls(&self) -> Vec<QvmModuleCall> {
        self.inner.borrow().calls.clone()
    }

    /// Recorded console commands.
    #[must_use]
    pub fn commands(&self) -> Vec<QvmModuleCommand> {
        self.inner.borrow().commands.clone()
    }

    /// Set the value bare calls and commands return.
    pub fn set_default_return(&self, value: i32) {
        self.inner.borrow_mut().default_return = value;
    }

    /// Install or clear the host trap handler.
    pub fn set_host(&self, host: Option<QvmHostFn>) {
        self.inner.borrow_mut().host = host;
    }

    /// Invoke a source callback under a live call (fixture-canned).
    pub fn invoke_source_callback(&self, callback: usize, arguments: &[i32]) -> i32 {
        self.inner
            .borrow_mut()
            .source_callbacks
            .push((callback, arguments.to_vec()));
        self.inner.borrow().source_callback_value
    }

    /// Recorded source-callback invocations.
    #[must_use]
    pub fn source_callbacks(&self) -> Vec<(usize, Vec<i32>)> {
        self.inner.borrow().source_callbacks.clone()
    }

    /// Set the value source callbacks return.
    pub fn set_source_callback_value(&self, value: i32) {
        self.inner.borrow_mut().source_callback_value = value;
    }

    /// Evaluate a standalone region (fixture-canned).
    pub fn evaluate_region(
        &self,
        _arguments: &[i32],
        _owner: usize,
        _region: &QvmRegionEvaluation,
        _inputs: &[i32],
    ) -> i32 {
        self.inner.borrow().region_value
    }

    /// Set the value region evaluations return.
    pub fn set_region_value(&self, value: i32) {
        self.inner.borrow_mut().region_value = value;
    }

    /// Evaluate a counter operation (fixture-canned).
    pub fn evaluate_counter(&self, _arguments: &[i32], _owner: usize, _address: usize, _functions: &[usize]) -> i32 {
        self.inner.borrow().counter_value
    }

    /// Set the value counter evaluations return.
    pub fn set_counter_value(&self, value: i32) {
        self.inner.borrow_mut().counter_value = value;
    }

    /// Top of the interpreter stack (the allocation end in the mirror).
    #[must_use]
    pub fn stack_pointer(&self) -> usize {
        self.memory().len()
    }

    /// Request cancellation of a scoped invocation.
    pub fn cancel_function(&self) {
        self.inner.borrow_mut().cancel_requests += 1;
    }

    /// Recorded cancellation requests.
    #[must_use]
    pub fn cancel_requests(&self) -> usize {
        self.inner.borrow().cancel_requests
    }

    /// Dispatch a host trap through the module host.
    pub fn dispatch_host(&self, call: &QvmHostCall) -> Result<i32, GuestError> {
        let host = self.inner.borrow().host.clone();
        match host {
            None => Err(GuestError::callback(format!(
                "Unbound {:?} QVM syscall {}",
                call.role, call.code
            ))),
            Some(host) => match host(call)? {
                Some(value) => Ok(value),
                None => Err(GuestError::callback(format!(
                    "Unbound {:?} QVM syscall {}",
                    call.role, call.code
                ))),
            },
        }
    }

    /// Capture a module checkpoint.
    pub fn checkpoint(&self) -> Result<QvmCheckpoint, GuestError> {
        self.assert_live()?;
        let inner = self.inner.borrow();
        let data = inner.memory.read_bytes(0, inner.memory.len())?;
        let (format, bytes) = match &inner.host_state {
            Some(state) => (state.checkpoint)(),
            None => ("qvm:empty-host-v1".to_string(), ProfileValue::Null),
        };
        Ok(QvmCheckpoint {
            module: inner.artifact.module.clone(),
            api_kind: match inner.artifact.role {
                QvmRole::Qagame => "q3-qagame".to_string(),
                QvmRole::Cgame => "q3-cgame".to_string(),
                QvmRole::Ui => "q3-ui".to_string(),
            },
            api_version: 0,
            abi_profile: inner.artifact.abi_profile.unwrap_or(AbiProfile::Modern),
            data,
            instruction_index: 0,
            operand_stack: Vec::new(),
            program_stack: inner.memory.len(),
            host_state: QvmHostState {
                module: inner.artifact.module.clone(),
                format,
                bytes,
            },
        })
    }

    /// Restore a module checkpoint.
    pub fn restore(&self, checkpoint: &QvmCheckpoint) -> Result<(), GuestError> {
        self.assert_live()?;
        let memory = self.memory();
        if checkpoint.data.len() != memory.len() {
            return Err(GuestError::BadSave(
                "QVM checkpoint differs from the allocation".to_string(),
            ));
        }
        memory.write_bytes(0, &checkpoint.data)?;
        if let Some(state) = self.inner.borrow().host_state.clone() {
            (state.restore)(&checkpoint.host_state.bytes)?;
        }
        Ok(())
    }
}

/// Profile/declaration value tree (mirror of `unknown` values read by
/// `SaveReader`).
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileValue {
    /// Missing field.
    Undefined,
    /// Explicit null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// Floating-point number.
    Float(f64),
    /// Text.
    Str(String),
    /// Raw bytes.
    Bytes(Vec<u8>),
    /// Array.
    Array(Vec<ProfileValue>),
    /// Record.
    Record(Vec<(String, ProfileValue)>),
}

impl ProfileValue {
    /// Look up a record field.
    #[must_use]
    pub fn record_get(&self, name: &str) -> Option<&ProfileValue> {
        match self {
            Self::Record(fields) => fields.iter().find(|(key, _)| key == name).map(|(_, value)| value),
            _ => None,
        }
    }

    /// Build a record from fields.
    #[must_use]
    pub fn record(fields: Vec<(&str, ProfileValue)>) -> Self {
        Self::Record(
            fields
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }
}

/// Value-tree reader (mirror of `SaveReader`).
#[derive(Debug, Clone)]
pub struct ProfileReader<'a> {
    value: &'a ProfileValue,
    path: String,
}

impl<'a> ProfileReader<'a> {
    /// Read from the root value.
    #[must_use]
    pub fn new(value: &'a ProfileValue) -> Self {
        Self {
            value,
            path: "save".to_string(),
        }
    }

    /// The current value.
    #[must_use]
    pub fn value(&self) -> &'a ProfileValue {
        self.value
    }

    /// Whether the current value is missing.
    #[must_use]
    pub fn is_undefined(&self) -> bool {
        matches!(self.value, ProfileValue::Undefined)
    }

    /// Fail with a path-qualified declaration error.
    pub fn fail<T>(&self, message: &str) -> Result<T, GuestError> {
        Err(GuestError::invalid(format!("{}: {message}", self.path)))
    }

    /// Read a record field (missing keys read as undefined). Like the donor,
    /// reading a field of a non-record value fails immediately.
    pub fn field(&self, name: &str) -> Result<ProfileReader<'a>, GuestError> {
        static UNDEFINED: ProfileValue = ProfileValue::Undefined;
        match self.value {
            ProfileValue::Record(_) => {
                let value = self.value.record_get(name).unwrap_or(&UNDEFINED);
                Ok(ProfileReader {
                    value,
                    path: format!("{}.{}", self.path, name),
                })
            }
            _ => self.fail("expected a record"),
        }
    }

    /// Read a string.
    pub fn string(&self) -> Result<String, GuestError> {
        match self.value {
            ProfileValue::Str(value) => Ok(value.clone()),
            _ => self.fail("expected a string"),
        }
    }

    /// Read a boolean.
    pub fn boolean(&self) -> Result<bool, GuestError> {
        match self.value {
            ProfileValue::Bool(value) => Ok(*value),
            _ => self.fail("expected a boolean"),
        }
    }

    /// Read a number.
    pub fn number(&self) -> Result<f64, GuestError> {
        match self.value {
            ProfileValue::Int(value) => Ok(*value as f64),
            ProfileValue::Float(value) => Ok(*value),
            _ => self.fail("expected a number"),
        }
    }

    /// Read a finite number.
    pub fn finite(&self) -> Result<f64, GuestError> {
        let value = self.number()?;
        if value.is_finite() {
            Ok(value)
        } else {
            self.fail("expected a finite number")
        }
    }

    /// Read an integer at or above `minimum`.
    pub fn integer(&self, minimum: i64) -> Result<i64, GuestError> {
        match self.value {
            ProfileValue::Int(value) if *value >= minimum => Ok(*value),
            ProfileValue::Float(value) if value.fract() == 0.0 && *value >= minimum as f64 => Ok(*value as i64),
            _ => self.fail("expected an integer in range"),
        }
    }

    /// Read raw bytes.
    pub fn bytes(&self) -> Result<Vec<u8>, GuestError> {
        match self.value {
            ProfileValue::Bytes(value) => Ok(value.clone()),
            _ => self.fail("expected raw checkpoint bytes"),
        }
    }

    /// Require an exact string.
    pub fn literal_str(&self, expected: &str) -> Result<String, GuestError> {
        match self.value {
            ProfileValue::Str(value) if value == expected => Ok(value.clone()),
            _ => self.fail(&format!("expected {expected}")),
        }
    }

    /// Require an exact integer.
    pub fn literal_int(&self, expected: i64) -> Result<i64, GuestError> {
        match self.value {
            ProfileValue::Int(value) if *value == expected => Ok(*value),
            _ => self.fail(&format!("expected {expected}")),
        }
    }

    /// Require an exact boolean.
    pub fn literal_bool(&self, expected: bool) -> Result<bool, GuestError> {
        match self.value {
            ProfileValue::Bool(value) if *value == expected => Ok(*value),
            _ => self.fail(&format!("expected {expected}")),
        }
    }

    /// Require one of the given strings.
    pub fn choice(&self, choices: &[&str]) -> Result<String, GuestError> {
        match self.value {
            ProfileValue::Str(value) if choices.contains(&value.as_str()) => Ok(value.clone()),
            _ => self.fail(&format!("expected {}", choices.join(" or "))),
        }
    }

    /// Read an array with an item reader.
    pub fn list<T>(&self, read: impl Fn(&ProfileReader<'a>) -> Result<T, GuestError>) -> Result<Vec<T>, GuestError> {
        match self.value {
            ProfileValue::Array(items) => items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    read(&ProfileReader {
                        value: item,
                        path: format!("{}[{index}]", self.path),
                    })
                })
                .collect(),
            _ => self.fail("expected an array"),
        }
    }

    /// Read null as `None`, otherwise delegate.
    pub fn nullable<T>(
        &self,
        read: impl FnOnce(&ProfileReader<'a>) -> Result<T, GuestError>,
    ) -> Result<Option<T>, GuestError> {
        if matches!(self.value, ProfileValue::Null) {
            Ok(None)
        } else {
            read(self).map(Some)
        }
    }
}

/// Read a namespaced identity (`namespace:name`).
pub fn namespaced_id(reader: &ProfileReader<'_>) -> Result<String, GuestError> {
    let value = reader.string()?;
    match value.find(':') {
        Some(colon) if colon > 0 && colon + 1 < value.len() => Ok(value),
        _ => reader.fail("expected a namespaced identity"),
    }
}

/// Qualify a forward original region with no escaping edge or live operand at
/// either boundary; returns the owning frame size in bytes.
pub fn qualify_qvm_region(
    instructions: &[QvmInstruction],
    owner: usize,
    entry: usize,
    join: usize,
) -> Result<usize, GuestError> {
    let fail = |message: &str| GuestError::invalid(format!("QVM region: {message}"));
    let first = instructions.get(owner).ok_or_else(|| fail("owner is not a function"))?;
    if first.opcode != QvmOpcode::OpEnter {
        return Err(fail("owner is not a function"));
    }
    let mut end = owner + 1;
    while end < instructions.len() && instructions[end].opcode != QvmOpcode::OpEnter {
        end += 1;
    }
    if entry <= owner || join <= entry || join >= end {
        return Err(fail("region is outside its owning function"));
    }
    let mut depth: HashMap<usize, i32> = HashMap::new();
    let mut pending = vec![(entry, 0i32)];
    let mut joined = false;
    while let Some((pc, count)) = pending.pop() {
        if let Some(known) = depth.get(&pc) {
            if *known != count {
                return Err(fail("paths disagree on operand depth"));
            }
            continue;
        }
        depth.insert(pc, count);
        if pc == join {
            if count != 0 {
                return Err(fail("join retains live operands"));
            }
            joined = true;
            continue;
        }
        let instruction = instructions.get(pc).ok_or_else(|| fail("no original instruction"))?;
        let opcode = instruction.opcode;
        let (required, change): (i32, i32) = match opcode {
            QvmOpcode::OpConst | QvmOpcode::OpLocal | QvmOpcode::OpPush => (0, 1),
            QvmOpcode::OpPop | QvmOpcode::OpArg | QvmOpcode::OpJump => (1, -1),
            QvmOpcode::OpStore1 | QvmOpcode::OpStore2 | QvmOpcode::OpStore4 | QvmOpcode::OpBlockCopy => (2, -2),
            _ if opcode.is_branch() => (2, -2),
            _ if (opcode as u8) >= (QvmOpcode::OpAdd as u8) && (opcode as u8) <= (QvmOpcode::OpRshu as u8)
                || (opcode as u8) >= (QvmOpcode::OpAddf as u8) && (opcode as u8) <= (QvmOpcode::OpMulf as u8) =>
            {
                if opcode == QvmOpcode::OpBcom {
                    (1, 0)
                } else {
                    (2, -1)
                }
            }
            QvmOpcode::OpCall
            | QvmOpcode::OpLoad1
            | QvmOpcode::OpLoad2
            | QvmOpcode::OpLoad4
            | QvmOpcode::OpSex8
            | QvmOpcode::OpSex16
            | QvmOpcode::OpNegi
            | QvmOpcode::OpNegf
            | QvmOpcode::OpCvif
            | QvmOpcode::OpCvfi => (1, 0),
            QvmOpcode::OpIgnore | QvmOpcode::OpBreak => (0, 0),
            _ => return Err(fail("frame or unsupported instruction")),
        };
        if count < required {
            return Err(fail("requires operands from outside its boundary"));
        }
        let result = count + change;
        let mut targets = vec![pc + 1];
        if opcode == QvmOpcode::OpJump {
            targets.clear();
            let target = instructions
                .get(pc.wrapping_sub(1))
                .ok_or_else(|| fail("indirect jump"))?;
            if target.opcode != QvmOpcode::OpConst || target.operand < 0 {
                return Err(fail("indirect jump"));
            }
            targets.push(target.operand as usize);
        } else if opcode.is_branch() && instruction.operand_width == 4 {
            if instruction.operand < 0 {
                return Err(fail("escaping or backward edge"));
            }
            targets.push(instruction.operand as usize);
        }
        for target in targets {
            if target <= pc || target > join {
                return Err(fail("escaping or backward edge"));
            }
            pending.push((target, result));
        }
    }
    if !joined {
        return Err(fail("does not reach its original join"));
    }
    for pc in (owner + 1)..end {
        if pc >= entry && pc < join {
            continue;
        }
        let Some(instruction) = instructions.get(pc) else {
            continue;
        };
        let destination = if instruction.opcode.is_branch() && instruction.operand_width == 4 {
            Some(instruction.operand)
        } else if instruction.opcode == QvmOpcode::OpJump {
            match instructions.get(pc.wrapping_sub(1)) {
                Some(target) if target.opcode == QvmOpcode::OpConst => Some(target.operand),
                _ => None,
            }
        } else {
            None
        };
        if let Some(destination) = destination {
            if destination > entry as i32 && (destination as usize) < join {
                return Err(fail("incoming interior edge"));
            }
        }
    }
    Ok(first.operand.max(0) as usize)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum QvmRegionOperand {
    Local(usize),
    LocalDerived,
    Constant(i32),
    Unknown,
}

fn operand_is_local(operand: &QvmRegionOperand) -> bool {
    matches!(operand, QvmRegionOperand::Local(_) | QvmRegionOperand::LocalDerived)
}

/// Check scalar live-ins of a standalone source frame; returns the owning
/// frame size in bytes.
pub fn qualify_qvm_region_evaluation(
    instructions: &[QvmInstruction],
    owner: usize,
    region: &QvmRegionEvaluation,
    access: QvmRegionAccess,
) -> Result<usize, GuestError> {
    let fail = |message: String| GuestError::invalid(format!("QVM region evaluation: {message}"));
    let frame = qualify_qvm_region(instructions, owner, region.entry, region.join)?;
    let valid = |offset: usize| offset >= 8 && offset % 4 == 0 && offset + 4 <= frame;
    let unique: std::collections::HashSet<usize> = region.inputs.iter().copied().collect();
    if region.inputs.iter().any(|offset| !valid(*offset))
        || unique.len() != region.inputs.len()
        || region.result.is_some_and(|offset| !valid(offset))
    {
        return Err(fail("live-in or result is outside its source frame".to_string()));
    }
    struct Path {
        stack: Vec<QvmRegionOperand>,
        initialized: std::collections::HashSet<usize>,
    }
    fn merge(
        pending: &mut HashMap<usize, Path>,
        target: usize,
        stack: Vec<QvmRegionOperand>,
        initialized: std::collections::HashSet<usize>,
    ) {
        match pending.remove(&target) {
            None => {
                pending.insert(target, Path { stack, initialized });
            }
            Some(previous) => {
                let merged = stack
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let before = previous.stack.get(index);
                        match (&value, before) {
                            (QvmRegionOperand::Local(offset), Some(QvmRegionOperand::Local(previous)))
                                if offset == previous =>
                            {
                                value
                            }
                            (QvmRegionOperand::Constant(current), Some(QvmRegionOperand::Constant(previous)))
                                if current == previous =>
                            {
                                QvmRegionOperand::Constant(*current)
                            }
                            _ if operand_is_local(&value) || before.is_some_and(operand_is_local) => {
                                QvmRegionOperand::LocalDerived
                            }
                            _ => QvmRegionOperand::Unknown,
                        }
                    })
                    .collect();
                let kept: std::collections::HashSet<usize> = initialized
                    .into_iter()
                    .filter(|offset| previous.initialized.contains(offset))
                    .collect();
                pending.insert(
                    target,
                    Path {
                        stack: merged,
                        initialized: kept,
                    },
                );
            }
        }
    }
    let mut pending: HashMap<usize, Path> = HashMap::new();
    pending.insert(
        region.entry,
        Path {
            stack: Vec::new(),
            initialized: unique,
        },
    );
    while !pending.is_empty() {
        let pc = *pending.keys().min().expect("pending region path");
        let path = pending.remove(&pc).expect("pending region path");
        if pc == region.join {
            if let Some(result) = region.result {
                if !path.initialized.contains(&result) {
                    return Err(fail("result is not initialized on every source path".to_string()));
                }
            }
            continue;
        }
        let instruction = instructions
            .get(pc)
            .ok_or_else(|| fail("missing instruction".to_string()))?;
        let mut stack = path.stack;
        let mut initialized = path.initialized;
        let opcode = instruction.opcode;
        if access == QvmRegionAccess::ReadOnly
            && matches!(
                opcode,
                QvmOpcode::OpCall | QvmOpcode::OpArg | QvmOpcode::OpBlockCopy | QvmOpcode::OpBreak
            )
        {
            return Err(fail(
                "read-only region cannot call, publish arguments, copy memory or break".to_string(),
            ));
        }
        let pop = |stack: &mut Vec<QvmRegionOperand>| -> Result<QvmRegionOperand, GuestError> {
            stack.pop().ok_or_else(|| fail("invalid operand proof".to_string()))
        };
        if opcode == QvmOpcode::OpLocal {
            if instruction.operand < 8
                || instruction.operand % 4 != 0
                || (instruction.operand as usize) + 4 > frame + 48
            {
                return Err(fail("local address exceeds its source frame and arguments".to_string()));
            }
            stack.push(QvmRegionOperand::Local(instruction.operand as usize));
        } else if opcode == QvmOpcode::OpConst {
            stack.push(QvmRegionOperand::Constant(instruction.operand));
        } else if opcode == QvmOpcode::OpPush {
            stack.push(QvmRegionOperand::Unknown);
        } else if opcode == QvmOpcode::OpPop || opcode == QvmOpcode::OpArg {
            pop(&mut stack)?;
            if opcode == QvmOpcode::OpArg && instruction.operand >= 0 {
                initialized.insert(instruction.operand as usize);
            }
        } else if (opcode as u8) >= (QvmOpcode::OpLoad1 as u8) && (opcode as u8) <= (QvmOpcode::OpLoad4 as u8) {
            let address = pop(&mut stack)?;
            if address == QvmRegionOperand::LocalDerived {
                return Err(fail("reads an unresolved source local address".to_string()));
            }
            if let QvmRegionOperand::Local(offset) = address {
                if offset < frame && !initialized.contains(&offset) {
                    return Err(fail(format!("reads undeclared source local {offset}")));
                }
            }
            stack.push(QvmRegionOperand::Unknown);
        } else if (opcode as u8) >= (QvmOpcode::OpStore1 as u8) && (opcode as u8) <= (QvmOpcode::OpStore4 as u8) {
            if operand_is_local(&pop(&mut stack)?) {
                return Err(fail("stores an escaping source local pointer".to_string()));
            }
            let address = pop(&mut stack)?;
            if access == QvmRegionAccess::ReadOnly {
                match address {
                    QvmRegionOperand::Local(offset) if offset >= 8 && offset + 4 <= frame => {}
                    _ => {
                        return Err(fail(
                            "read-only region cannot write outside its own local frame".to_string(),
                        ));
                    }
                }
            }
            if let QvmRegionOperand::Local(offset) = address {
                if opcode == QvmOpcode::OpStore4 {
                    initialized.insert(offset);
                }
            }
        } else if opcode == QvmOpcode::OpBlockCopy {
            let source = pop(&mut stack)?;
            if source == QvmRegionOperand::LocalDerived {
                return Err(fail("copies an unresolved source local address".to_string()));
            }
            if let QvmRegionOperand::Local(offset) = source {
                let mut cursor = 0;
                while cursor < instruction.operand {
                    if !initialized.contains(&(offset + cursor.max(0) as usize)) {
                        return Err(fail("copies an undeclared source local".to_string()));
                    }
                    cursor += 4;
                }
            }
            let address = pop(&mut stack)?;
            if let QvmRegionOperand::Local(offset) = address {
                let mut cursor = 0;
                while cursor + 4 <= instruction.operand {
                    initialized.insert(offset + cursor.max(0) as usize);
                    cursor += 4;
                }
            }
        } else if opcode == QvmOpcode::OpJump {
            let destination = pop(&mut stack)?;
            match destination {
                QvmRegionOperand::Constant(target) if target >= 0 => {
                    merge(&mut pending, target as usize, stack, initialized);
                    continue;
                }
                _ => return Err(fail("jump lost its source target".to_string())),
            }
        } else if opcode.is_branch() && instruction.operand_width == 4 {
            pop(&mut stack)?;
            pop(&mut stack)?;
            if instruction.operand >= 0 {
                merge(
                    &mut pending,
                    instruction.operand as usize,
                    stack.clone(),
                    initialized.clone(),
                );
            }
        } else if opcode == QvmOpcode::OpCall {
            pop(&mut stack)?;
            stack.push(QvmRegionOperand::Unknown);
        } else if ((opcode as u8) >= (QvmOpcode::OpAdd as u8)
            && (opcode as u8) <= (QvmOpcode::OpRshu as u8)
            && opcode != QvmOpcode::OpBcom)
            || (opcode as u8) >= (QvmOpcode::OpAddf as u8) && (opcode as u8) <= (QvmOpcode::OpMulf as u8)
        {
            let right = pop(&mut stack)?;
            let left = pop(&mut stack)?;
            match (&left, &right) {
                (QvmRegionOperand::Local(offset), QvmRegionOperand::Constant(value))
                    if opcode == QvmOpcode::OpAdd || opcode == QvmOpcode::OpSub =>
                {
                    let next = if opcode == QvmOpcode::OpAdd {
                        offset as i32 + value
                    } else {
                        offset as i32 - value
                    };
                    stack.push(QvmRegionOperand::Local(next.max(0) as usize));
                }
                _ if operand_is_local(&left) || operand_is_local(&right) => {
                    stack.push(QvmRegionOperand::LocalDerived);
                }
                _ => stack.push(QvmRegionOperand::Unknown),
            }
        } else if opcode != QvmOpcode::OpIgnore && opcode != QvmOpcode::OpBreak {
            let value = pop(&mut stack)?;
            stack.push(if operand_is_local(&value) {
                QvmRegionOperand::LocalDerived
            } else {
                QvmRegionOperand::Unknown
            });
        }
        merge(&mut pending, pc + 1, stack, initialized);
    }
    Ok(frame)
}

/// Qualify original player-to-mesh calls for a body scope.
///
/// Returns `(call-site, part)` pairs in ascending call-site order. `parts`
/// selects explicit parts per call site; otherwise every direct player call
/// to the mesh entry qualifies as `default_part`.
#[allow(clippy::too_many_arguments)]
pub fn qualify_qvm_body_calls<P: Clone>(
    image: &QvmImage,
    player_entry: usize,
    player_argument: i32,
    mesh_entry: usize,
    entity_argument: i32,
    state_argument: i32,
    shader_offset: i32,
    parts: Option<&[(usize, P)]>,
    default_part: P,
) -> Result<Vec<(usize, P)>, GuestError> {
    for argument in [player_argument, entity_argument, state_argument] {
        if argument < 0 || argument as usize >= QVM_MAX_PRIVATE_ARGUMENT_WORDS {
            return Err(GuestError::invalid(
                "Source body arguments differ from the original refEntity ABI",
            ));
        }
    }
    if shader_offset != 112 {
        return Err(GuestError::invalid(
            "Source body arguments differ from the original refEntity ABI",
        ));
    }
    let player = image.instruction(player_entry);
    let mesh = image.instruction(mesh_entry);
    if player.map_or(true, |instruction| instruction.opcode != QvmOpcode::OpEnter)
        || mesh.map_or(true, |instruction| instruction.opcode != QvmOpcode::OpEnter)
    {
        return Err(GuestError::invalid(
            "Source body scope requires original function entries",
        ));
    }
    let end = image.function_end(player_entry);
    let valid = |index: usize| {
        index > player_entry
            && index < end
            && image
                .instruction(index)
                .is_some_and(|instruction| instruction.opcode == QvmOpcode::OpCall)
            && image
                .instruction(index - 1)
                .is_some_and(|target| target.opcode == QvmOpcode::OpConst && target.operand as usize == mesh_entry)
    };
    let mut calls: Vec<(usize, P)> = Vec::new();
    match parts {
        None => {
            for index in (player_entry + 1)..end {
                if valid(index) {
                    calls.push((index, default_part.clone()));
                }
            }
        }
        Some(parts) => {
            for (site, part) in parts {
                if !valid(*site) || calls.iter().any(|(known, _)| known == site) {
                    return Err(GuestError::invalid(
                        "Source body part does not name a distinct original player-to-mesh call",
                    ));
                }
                calls.push((*site, part.clone()));
            }
        }
    }
    if calls.is_empty() {
        return Err(GuestError::invalid(
            "Source body scope has no qualified original mesh calls",
        ));
    }
    calls.sort_by_key(|(site, _)| *site);
    Ok(calls)
}

fn check_i32(value: i64, what: &str) -> Result<i32, GuestError> {
    i32::try_from(value).map_err(|_| GuestError::invalid(format!("{what} requires a signed 32-bit word")))
}

/// Located game-data state: table words plus strides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmGameDataState {
    /// Entity table word (0 when unlocated).
    pub entities_word: usize,
    /// Located entity count.
    pub num_entities: usize,
    /// Entity stride in bytes.
    pub entity_stride: usize,
    /// Client table word (0 when unlocated).
    pub clients_word: usize,
    /// Client stride in bytes.
    pub client_stride: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QvmGameTables {
    entities: Option<usize>,
    entity_stride: usize,
    count: usize,
    clients: Option<usize>,
    client_stride: usize,
    client_count: usize,
}

/// Located entity/client tables (port of `SV_LocateGameData` and friends).
///
/// Windows borrow the shared allocation; relocation changes table
/// descriptors, not previously opened windows. Clones share one descriptor
/// block, so hosts, combat, and providers observe the same tables.
#[derive(Debug, Clone)]
pub struct QvmGameData {
    memory: QvmSharedMemory,
    abi_profile: AbiProfile,
    tables: Rc<RefCell<QvmGameTables>>,
}

impl QvmGameData {
    /// Build unlocated game data over `memory`.
    #[must_use]
    pub fn new(memory: QvmSharedMemory, abi_profile: AbiProfile) -> Self {
        Self {
            memory,
            abi_profile,
            tables: Rc::new(RefCell::new(QvmGameTables {
                entities: None,
                entity_stride: 0,
                count: 0,
                clients: None,
                client_stride: 0,
                client_count: 64,
            })),
        }
    }

    /// ABI profile of the located records.
    #[must_use]
    pub fn abi_profile(&self) -> AbiProfile {
        self.abi_profile
    }

    /// Narrow the native client capacity (1..=64).
    pub fn set_client_count(&self, count: usize) -> Result<(), GuestError> {
        if !(1..=64).contains(&count) {
            return Err(GuestError::invalid("Q3 client count must be within 1..64"));
        }
        self.tables.borrow_mut().client_count = count;
        Ok(())
    }

    /// Configured client capacity.
    #[must_use]
    pub fn num_clients(&self) -> usize {
        self.tables.borrow().client_count
    }

    /// Located entity count.
    #[must_use]
    pub fn num_entities(&self) -> usize {
        self.tables.borrow().count
    }

    /// Entity stride in bytes.
    #[must_use]
    pub fn entity_stride_bytes(&self) -> usize {
        self.tables.borrow().entity_stride
    }

    /// Client stride in bytes.
    #[must_use]
    pub fn client_stride_bytes(&self) -> usize {
        self.tables.borrow().client_stride
    }

    /// Full entity record window.
    pub fn entity_bytes(&self, number: usize) -> Result<QvmMemoryWindow, GuestError> {
        let offset = self.entity_offset(number)?;
        QvmMemoryWindow::new(self.memory.clone(), offset, self.entity_stride_bytes())
    }

    /// Full client record window.
    pub fn client_bytes(&self, number: usize) -> Result<QvmMemoryWindow, GuestError> {
        let offset = self.client_offset(number)?;
        QvmMemoryWindow::new(self.memory.clone(), offset, self.client_stride_bytes())
    }

    /// Public shared-entity prefix window.
    pub fn public_entity_bytes(&self, number: usize) -> Result<QvmMemoryWindow, GuestError> {
        let offset = self.entity_offset(number)?;
        QvmMemoryWindow::new(self.memory.clone(), offset, qvm_shared_entity_bytes(self.abi_profile))
    }

    /// Public player-state prefix window.
    pub fn public_player_bytes(&self, number: usize) -> Result<QvmMemoryWindow, GuestError> {
        let offset = self.client_offset(number)?;
        QvmMemoryWindow::new(self.memory.clone(), offset, qvm_player_state_bytes(self.abi_profile))
    }

    /// Clear located tables.
    pub fn clear(&self) {
        let mut tables = self.tables.borrow_mut();
        tables.entities = None;
        tables.clients = None;
        tables.entity_stride = 0;
        tables.client_stride = 0;
        tables.count = 0;
    }

    /// Capture table descriptors.
    #[must_use]
    pub fn checkpoint(&self) -> QvmGameDataState {
        let tables = self.tables.borrow();
        let word = |offset: Option<usize>| match offset {
            None => 0,
            Some(0) => self.memory.len(),
            Some(offset) => offset,
        };
        QvmGameDataState {
            entities_word: word(tables.entities),
            num_entities: tables.count,
            entity_stride: tables.entity_stride,
            clients_word: word(tables.clients),
            client_stride: tables.client_stride,
        }
    }

    /// Restore table descriptors.
    pub fn restore(&self, state: &QvmGameDataState) -> Result<(), GuestError> {
        if state.entities_word == 0
            && state.clients_word == 0
            && state.num_entities == 0
            && state.entity_stride == 0
            && state.client_stride == 0
        {
            self.clear();
            return Ok(());
        }
        self.locate(
            state.entities_word as i32,
            state.num_entities,
            state.entity_stride,
            state.clients_word as i32,
            state.client_stride,
        )
    }

    /// Locate entity/client tables (port of `SV_LocateGameData`).
    pub fn locate(
        &self,
        entities_word: i32,
        num_entities: usize,
        entity_stride: usize,
        clients_word: i32,
        client_stride: usize,
    ) -> Result<(), GuestError> {
        check_i32(num_entities as i64, "Entity count")?;
        check_i32(entity_stride as i64, "Entity stride")?;
        check_i32(client_stride as i64, "Client stride")?;
        if num_entities > 1024 {
            return Err(GuestError::invalid("Q3 wire entity capacity is 1024 slots"));
        }
        let stride = |value: usize, minimum: usize| -> Result<(), GuestError> {
            if value < minimum || value % 4 != 0 {
                return Err(GuestError::invalid("Game-data stride is undersized or unaligned"));
            }
            Ok(())
        };
        stride(entity_stride, qvm_shared_entity_bytes(self.abi_profile))?;
        stride(client_stride, qvm_player_state_bytes(self.abi_profile))?;
        let entities = self.offset(entities_word)?;
        let clients = self.offset(clients_word)?;
        let (Some(entities), Some(clients)) = (entities, clients) else {
            return Err(GuestError::invalid("Game-data tables require aligned nonnull pointers"));
        };
        if entities % 4 != 0 || clients % 4 != 0 {
            return Err(GuestError::invalid("Game-data tables require aligned nonnull pointers"));
        }
        QvmMemoryWindow::new(
            self.memory.clone(),
            entities,
            num_entities.saturating_mul(entity_stride),
        )?;
        QvmMemoryWindow::new(self.memory.clone(), clients, client_stride)?;
        let mut tables = self.tables.borrow_mut();
        tables.entities = Some(entities);
        tables.entity_stride = entity_stride;
        tables.count = num_entities;
        tables.clients = Some(clients);
        tables.client_stride = client_stride;
        Ok(())
    }

    fn offset(&self, word: i32) -> Result<Option<usize>, GuestError> {
        if word == 0 {
            return Ok(None);
        }
        if word < 0 {
            return Err(GuestError::invalid("Game-data pointer is outside the allocation"));
        }
        let offset = word as usize;
        if offset == self.memory.len() {
            return Ok(Some(0));
        }
        if offset > self.memory.len() {
            return Err(GuestError::invalid("Game-data pointer is outside the allocation"));
        }
        Ok(Some(offset))
    }

    fn entity_offset(&self, number: usize) -> Result<usize, GuestError> {
        let tables = self.tables.borrow();
        if number >= tables.count {
            return Err(GuestError::invalid("Entity slot is outside located game data"));
        }
        self.indexed(tables.entities, tables.entity_stride, number)
    }

    fn client_offset(&self, number: usize) -> Result<usize, GuestError> {
        let tables = self.tables.borrow();
        if number >= tables.client_count {
            return Err(GuestError::invalid("Client slot is outside configured game data"));
        }
        let stride = tables.client_stride;
        let offset = self.indexed(tables.clients, stride, number)?;
        QvmMemoryWindow::new(self.memory.clone(), offset, stride)?;
        Ok(offset)
    }

    fn indexed(&self, base: Option<usize>, stride: usize, number: usize) -> Result<usize, GuestError> {
        let Some(base) = base else {
            return Err(GuestError::invalid("Game-data table has a null source pointer"));
        };
        let displacement = stride
            .checked_mul(number)
            .ok_or_else(|| GuestError::invalid("Game-data pointer arithmetic leaves the allocation"))?;
        check_i32(displacement as i64, "Game-data displacement")?;
        let offset = base + displacement;
        if offset > self.memory.len() {
            return Err(GuestError::invalid(
                "Game-data pointer arithmetic leaves the interpreter allocation",
            ));
        }
        Ok(offset)
    }

    /// Parse the shared-entity prefix of slot `number`.
    pub fn entity(&self, number: usize) -> Result<QvmSharedEntity, GuestError> {
        let window = self.public_entity_bytes(number)?;
        let bytes = window.copy_bytes(0, window.len)?;
        super::shared_entity_record::read_qvm_shared_entity(&bytes, self.abi_profile)
    }

    /// Parse the shared entity addressed by a guest word.
    pub fn entity_from_pointer(&self, word: i32) -> Result<QvmSharedEntity, GuestError> {
        let number = self.number_from_pointer(word)?;
        self.entity(number)
    }

    /// Resolve a guest word to an entity slot (port of `SV_NumForGentity`).
    pub fn number_from_pointer(&self, word: i32) -> Result<usize, GuestError> {
        let tables = self.tables.borrow();
        let (Some(offset), Some(base)) = (self.offset(word)?, tables.entities) else {
            return Err(GuestError::invalid("Entity numbering requires nonnull source pointers"));
        };
        if tables.entity_stride == 0 {
            return Err(GuestError::invalid("Entity numbering requires a nonzero source stride"));
        }
        if offset < base || (offset - base) % tables.entity_stride != 0 {
            return Err(GuestError::invalid("Entity pointer is not a record boundary"));
        }
        let number = (offset - base) / tables.entity_stride;
        drop(tables);
        self.entity_offset(number)?;
        Ok(number)
    }

    /// Copy the player state of client `number`.
    pub fn copy_player_state(&self, number: usize) -> Result<QvmPlayerState, GuestError> {
        let window = self.public_player_bytes(number)?;
        let bytes = window.copy_bytes(0, window.len)?;
        super::player_record::read_qvm_player_state(&bytes, self.abi_profile)
    }

    /// Write the player state of client `number`, preserving private slots.
    pub fn write_player_state(&self, number: usize, state: &QvmPlayerState) -> Result<(), GuestError> {
        let window = self.public_player_bytes(number)?;
        let mut bytes = window.copy_bytes(0, window.len)?;
        super::player_record::write_qvm_player_state_preserve(&mut bytes, state, self.abi_profile)?;
        self.memory.write_bytes(window.offset, &bytes)
    }

    /// Read the ping field the server writes directly.
    pub fn player_ping(&self, number: usize) -> Result<i32, GuestError> {
        let window = self.public_player_bytes(number)?;
        let at = if self.abi_profile.is_modern() { 452 } else { 440 };
        window.get_i32(at)
    }

    /// Write the ping field the server owns directly.
    pub fn set_player_ping(&self, number: usize, ping: i32) -> Result<(), GuestError> {
        let window = self.public_player_bytes(number)?;
        let at = if self.abi_profile.is_modern() { 452 } else { 440 };
        window.set_i32(at, ping)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn located() -> QvmGameData {
        let memory = QvmSharedMemory::new(4096).unwrap();
        let data = QvmGameData::new(memory, AbiProfile::Modern);
        data.locate(64, 4, 560, 2304, 480).unwrap();
        data
    }

    #[test]
    fn locate_validates_strides_and_alignment() {
        let memory = QvmSharedMemory::new(4096).unwrap();
        let data = QvmGameData::new(memory, AbiProfile::Modern);
        assert!(data.locate(64, 4, 500, 2304, 480).is_err());
        assert!(data.locate(64, 4, 560, 2304, 400).is_err());
        assert!(data.locate(65, 4, 560, 2304, 480).is_err());
        assert!(data.locate(0, 4, 560, 0, 480).is_err());
        assert!(data.locate(64, 2000, 560, 2304, 480).is_err());
    }

    #[test]
    fn entity_numbering_round_trips() {
        let data = located();
        assert_eq!(data.number_from_pointer(64).unwrap(), 0);
        assert_eq!(data.number_from_pointer(64 + 560 * 3).unwrap(), 3);
        assert!(data.number_from_pointer(65).is_err());
        assert!(data.number_from_pointer(0).is_err());
    }

    #[test]
    fn checkpoint_restore_round_trips() {
        let data = located();
        let saved = data.checkpoint();
        assert_eq!(saved.entities_word, 64);
        assert_eq!(saved.num_entities, 4);
        data.clear();
        assert_eq!(data.num_entities(), 0);
        data.restore(&saved).unwrap();
        assert_eq!(data.num_entities(), 4);
        assert_eq!(data.entity_stride_bytes(), 560);
    }

    #[test]
    fn player_ping_writes_directly() {
        let data = located();
        data.set_player_ping(1, 87).unwrap();
        assert_eq!(data.player_ping(1).unwrap(), 87);
        assert!(data.set_player_ping(99, 1).is_err());
    }

    #[test]
    fn memory_observers_see_before_and_after() {
        let memory = QvmSharedMemory::new(64).unwrap();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&seen);
        memory.observe_writes(
            vec![QvmWriteRange {
                byte_offset: 8,
                byte_length: 4,
            }],
            Rc::new(move |event| {
                captured.borrow_mut().push(event.clone());
            }),
            None,
        );
        memory.write_i32(8, 0x1122_3344).unwrap();
        memory.write_i32(32, 7).unwrap();
        let seen = seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].ranges[0].before, vec![0, 0, 0, 0]);
        assert_eq!(seen[0].ranges[0].after, vec![0x44, 0x33, 0x22, 0x11]);
        assert!(seen[0].touches(&QvmWriteRange {
            byte_offset: 10,
            byte_length: 2,
        }));
        assert!(!seen[0].touches(&QvmWriteRange {
            byte_offset: 12,
            byte_length: 4,
        }));
    }

    #[test]
    fn module_drives_hooks_and_records_calls() {
        let module = QvmModule::new(
            QvmArtifact {
                module: ModuleIdentity {
                    id: "q3:qagame".to_string(),
                    artifact_path: "qagame.qvm".to_string(),
                    digest: "d".to_string(),
                    revision: "r".to_string(),
                },
                role: QvmRole::Qagame,
                abi_profile: None,
                image: QvmImage::default(),
            },
            None,
            None,
        )
        .unwrap();
        assert_eq!(module.call(&[1, 2], 9).unwrap(), 0);
        let hook: QvmHookFn = Rc::new(|call| {
            call.words[0] += 1;
            call.proceed_value = 41;
            call.proceed()
        });
        let id = module.bind_function(9, hook);
        assert_eq!(module.call(&[1, 2], 9).unwrap(), 41);
        assert!(module.remove_hook(id));
        assert_eq!(module.call(&[1, 2], 9).unwrap(), 0);
        assert_eq!(module.calls().len(), 3);
    }

    #[test]
    fn qualify_region_accepts_forward_branch() {
        let instructions = vec![
            QvmInstruction::word(QvmOpcode::OpEnter, 16, 0),
            QvmInstruction::word(QvmOpcode::OpConst, 3, 1),
            QvmInstruction::word(QvmOpcode::OpConst, 4, 6),
            QvmInstruction::word(QvmOpcode::OpEq, 5, 11),
            QvmInstruction::single(QvmOpcode::OpPop, 16),
            QvmInstruction::single(QvmOpcode::OpLeave, 17),
        ];
        assert_eq!(qualify_qvm_region(&instructions, 0, 1, 4).unwrap(), 16);
        assert!(qualify_qvm_region(&instructions, 0, 4, 1).is_err());
    }

    #[test]
    fn qualify_evaluation_tracks_locals() {
        let instructions = vec![
            QvmInstruction::word(QvmOpcode::OpEnter, 16, 0),
            QvmInstruction::word(QvmOpcode::OpLocal, 8, 1),
            QvmInstruction::single(QvmOpcode::OpLoad4, 6),
            QvmInstruction::single(QvmOpcode::OpPop, 7),
            QvmInstruction::single(QvmOpcode::OpLeave, 8),
        ];
        let region = QvmRegionEvaluation {
            entry: 1,
            join: 4,
            inputs: vec![8],
            result: None,
        };
        assert_eq!(
            qualify_qvm_region_evaluation(&instructions, 0, &region, QvmRegionAccess::Source).unwrap(),
            16
        );
        let missing = QvmRegionEvaluation {
            entry: 1,
            join: 4,
            inputs: vec![],
            result: None,
        };
        assert!(qualify_qvm_region_evaluation(&instructions, 0, &missing, QvmRegionAccess::Source).is_err());
    }

    #[test]
    fn profile_reader_mirrors_save_reader_surface() {
        let value = ProfileValue::record(vec![
            ("name", ProfileValue::Str("q3:test".to_string())),
            ("count", ProfileValue::Int(3)),
            ("tags", ProfileValue::Array(vec![ProfileValue::Int(1)])),
        ]);
        let reader = ProfileReader::new(&value);
        assert_eq!(reader.field("name").unwrap().string().unwrap(), "q3:test");
        assert_eq!(reader.field("count").unwrap().integer(0).unwrap(), 3);
        assert!(reader.field("missing").unwrap().is_undefined());
        assert_eq!(namespaced_id(&reader.field("name").unwrap()).unwrap(), "q3:test");
        assert!(namespaced_id(&reader.field("count").unwrap()).is_err());
        assert_eq!(
            reader.field("tags").unwrap().list(|item| item.integer(0)).unwrap(),
            vec![1]
        );
        assert_eq!(
            reader.field("name").unwrap().nullable(|item| item.string()).unwrap(),
            Some("q3:test".to_string())
        );
        assert!(reader.field("count").unwrap().field("deeper").is_err());
    }
}
