//! QVM gameplay-mod provider: source callbacks over declared actor records.
//!
//! Provenance: `src/compat/qvm/mod-provider.ts`.
//!
//! Absorbs the pure-Rust types of `src/contracts/qvm-mod-callbacks.ts` and
//! `src/contracts/qvm-mod-actor-frame.ts`. `src/contracts/qvm-combat.ts` lives
//! with [`super::primary_player_profile`], item-stage types
//! (`src/contracts/qvm-mod-items.ts`) with [`super::mod_weapon_stage`],
//! protection types with [`super::mod_protection`], and presentation types
//! (`src/contracts/qvm-mod-presentation.ts`) with [`super::mod_presentation`].
//!
//! Shared local mirrors (also used by sibling ports in this batch via
//! `super::mod_provider`): [`ProfileValue`]/[`ProfileReader`] (mirror of
//! `SaveReader` in `src/persistence/value.ts`), [`QvmAbi`], [`ModuleId`],
//! [`QvmRole`], [`QvmOpcode`], [`QvmImage`], [`QvmArtifact`],
//! [`QvmRegionEvaluation`] plus [`qualify_qvm_region`] /
//! [`qualify_qvm_region_evaluation`] (mirrors of `src/compat/qvm/regions.ts`),
//! record byte sizes (mirrors of the `player/entity/client-state/shared-entity`
//! record owners), [`QvmItemLayout`] readers stay with
//! [`super::primary_pickup_profile`]. Validation sub-mirrors ported from their
//! donors: actor bootstrap/frame (`mod-actor-frame.ts`), objective addresses
//! (`mod-objectives.ts`), match fields (`source-match.ts`), item storage
//! (`item-storage.ts`), mod items/actors/pickups validation, client outputs
//! (`world/session/mod-client-outputs.ts`). The entity-token constructor used by
//! validation performs no checks in the donor, so it is a documented no-op.
//! Live execution (interpreter, guest memory, world services, client
//! bindings, input, items, pickups, actor semantics, objectives) is integrated
//! through [`ModProviderHost`]; sub-component runtimes owned by other workers
//! are host seams, while all declaration validation here is complete.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{Vec3, vec3};

use super::mod_presentation::QvmModPresentationDeclaration;
use super::mod_protection::{QvmModProtection, QvmModProtectionScalar};
use super::mod_weapon_stage::{QvmItemField, QvmItemStorage, QvmModItems};
use crate::error::GuestError;

// ---------------------------------------------------------------------------
// Shared declaration-value reader (mirror of `SaveReader`).
// ---------------------------------------------------------------------------

/// Declaration value tree (mirror of the `unknown` values read by `SaveReader`).
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
        Self::Record(fields.into_iter().map(|(key, value)| (key.to_string(), value)).collect())
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
        Self { value, path: "save".to_string() }
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

    /// Read a record field (missing keys read as undefined).
    pub fn field(&self, name: &str) -> Result<ProfileReader<'a>, GuestError> {
        static UNDEFINED: ProfileValue = ProfileValue::Undefined;
        match self.value {
            ProfileValue::Record(_) => {
                let value = self.value.record_get(name).unwrap_or(&UNDEFINED);
                Ok(ProfileReader { value, path: format!("{}.{}", self.path, name) })
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
        if value.is_finite() { Ok(value) } else { self.fail("expected a finite number") }
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
                .map(|(index, item)| read(&ProfileReader { value: item, path: format!("{}[{index}]", self.path) }))
                .collect(),
            _ => self.fail("expected an array"),
        }
    }

    /// Read null as `None`, otherwise delegate.
    pub fn nullable<T>(&self, read: impl FnOnce(&ProfileReader<'a>) -> Result<T, GuestError>) -> Result<Option<T>, GuestError> {
        if matches!(self.value, ProfileValue::Null) { Ok(None) } else { read(self).map(Some) }
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

// ---------------------------------------------------------------------------
// Shared ABI / module / image mirrors.
// ---------------------------------------------------------------------------

/// QVM ABI profile (mirror of `QvmAbiProfile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmAbi {
    /// Modern ABI.
    Modern,
    /// Legacy ABI.
    Legacy,
}

impl QvmAbi {
    /// Whether this is the modern profile.
    #[must_use]
    pub const fn is_modern(self) -> bool {
        matches!(self, Self::Modern)
    }

    /// Declaration name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        if self.is_modern() { "q3-modern" } else { "q3-legacy" }
    }

    /// Parse a declaration name.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "q3-modern" => Some(Self::Modern),
            "q3-legacy" => Some(Self::Legacy),
            _ => None,
        }
    }
}

/// Guest module identity (mirror of `ModuleIdentity`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleId {
    /// Module id.
    pub id: String,
    /// Artifact path.
    pub artifact_path: String,
    /// Artifact digest.
    pub digest: String,
    /// Module revision.
    pub revision: String,
}

impl ModuleId {
    /// Whether two identities name the same exact module bytes.
    #[must_use]
    pub fn same_module(&self, other: &Self) -> bool {
        self.id == other.id && self.digest == other.digest && self.artifact_path == other.artifact_path && self.revision == other.revision
    }
}

/// QVM role (mirror of `QvmRole`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmRole {
    /// Server game module.
    Qagame,
    /// Client game module.
    Cgame,
    /// UI module.
    Ui,
}

/// Maximum private argument words of one source call.
pub const QVM_MAX_PRIVATE_ARGUMENT_WORDS: usize = 62;

/// QVM opcodes in donor enum order.
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

    /// Encoded operand width.
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

/// One decoded QVM instruction (mirror of `QvmInstruction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmInstruction {
    /// Operation.
    pub opcode: QvmOpcode,
    /// Operand word.
    pub operand: i32,
    /// Encoded operand width.
    pub operand_width: u8,
}

impl QvmInstruction {
    /// Build a word instruction.
    #[must_use]
    pub fn word(opcode: QvmOpcode, operand: i32) -> Self {
        let operand_width = opcode.operand_width();
        Self { opcode, operand, operand_width }
    }
}

/// Declared executable image layout (mirror of `QvmImage` extents).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmImage {
    /// Decoded instructions.
    pub instructions: Vec<QvmInstruction>,
    /// Initialized data length.
    pub data_length: usize,
    /// Literal length.
    pub literal_length: usize,
    /// BSS length.
    pub bss_length: usize,
    /// Initialized-bytes length (`initializedData.length`).
    pub initialized_length: usize,
    /// Allocated data length.
    pub allocated_data_length: usize,
}

impl QvmImage {
    /// End of source data (`dataLength + literalLength + bssLength`).
    #[must_use]
    pub fn data_end(&self) -> usize {
        self.data_length + self.literal_length + self.bss_length
    }

    /// Fetch an instruction.
    #[must_use]
    pub fn instruction(&self, index: usize) -> Option<&QvmInstruction> {
        self.instructions.get(index)
    }

    /// End of the function owning `entry` (next `OP_ENTER` or image end).
    #[must_use]
    pub fn function_end(&self, entry: usize) -> usize {
        let mut end = entry + 1;
        while end < self.instructions.len() && self.instructions[end].opcode != QvmOpcode::OpEnter {
            end += 1;
        }
        end
    }
}

/// Declared module artifact (mirror of `QvmModuleOptions["artifact"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmArtifact {
    /// Module identity.
    pub module: ModuleId,
    /// Module role.
    pub role: QvmRole,
    /// ABI profile (`None` means modern).
    pub abi_profile: Option<QvmAbi>,
    /// Image layout and code.
    pub image: QvmImage,
}

impl QvmArtifact {
    /// Effective ABI profile.
    #[must_use]
    pub fn abi(&self) -> QvmAbi {
        self.abi_profile.unwrap_or(QvmAbi::Modern)
    }
}

/// API identity for a role (mirror of `qvmApi`).
#[must_use]
pub const fn qvm_api(role: QvmRole, abi: QvmAbi) -> (&'static str, u32) {
    match (role, abi.is_modern()) {
        (QvmRole::Qagame, true) => ("q3-qagame", 8),
        (QvmRole::Qagame, false) => ("q3-qagame", 7),
        (QvmRole::Cgame, true) => ("q3-cgame", 4),
        (QvmRole::Cgame, false) => ("q3-cgame", 3),
        (QvmRole::Ui, true) => ("q3-ui", 6),
        (QvmRole::Ui, false) => ("q3-ui", 4),
    }
}

// ---------------------------------------------------------------------------
// Shared record byte sizes (mirrors of the record owners).
// ---------------------------------------------------------------------------

/// Player-state record bytes.
#[must_use]
pub const fn qvm_player_state_bytes(abi: QvmAbi) -> usize {
    if abi.is_modern() { 468 } else { 444 }
}

/// Entity-state record bytes.
#[must_use]
pub const fn qvm_entity_state_bytes(abi: QvmAbi) -> usize {
    if abi.is_modern() { 208 } else { 204 }
}

/// Snapshot record bytes.
#[must_use]
pub const fn qvm_snapshot_bytes(abi: QvmAbi) -> usize {
    if abi.is_modern() { 53772 } else { 52724 }
}

/// Shared-entity record bytes.
#[must_use]
pub const fn qvm_shared_entity_bytes(_abi: QvmAbi) -> usize {
    516
}

/// Game-state record bytes.
pub const QVM_GAME_STATE_BYTES: usize = 20100;
/// Reference-entity record bytes.
pub const QVM_REF_ENTITY_BYTES: usize = 140;

// ---------------------------------------------------------------------------
// Shared region qualification (mirror of `src/compat/qvm/regions.ts`).
// ---------------------------------------------------------------------------

/// Original evaluation region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmRegionEvaluation {
    /// Region entry instruction.
    pub entry: usize,
    /// Region join instruction.
    pub join: usize,
    /// Scalar live-in local offsets.
    pub inputs: Vec<usize>,
    /// Result local offset, if any.
    pub result: Option<usize>,
}

/// Qualify a forward original region; returns the owning frame size.
pub fn qualify_qvm_region(instructions: &[QvmInstruction], owner: usize, entry: usize, join: usize) -> Result<usize, GuestError> {
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
                if opcode == QvmOpcode::OpBcom { (1, 0) } else { (2, -1) }
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
        if opcode == QvmOpcode::OpJump {
            let target = pc.checked_sub(1).and_then(|at| instructions.get(at)).ok_or_else(|| fail("indirect jump"))?;
            if target.opcode != QvmOpcode::OpConst {
                return Err(fail("indirect jump"));
            }
            let destination = target.operand;
            if destination <= pc as i32 || destination > join as i32 {
                return Err(fail("escaping or backward edge"));
            }
            pending.push((destination as usize, result));
        } else {
            if pc + 1 > join {
                return Err(fail("escaping or backward edge"));
            }
            pending.push((pc + 1, result));
            if opcode.is_branch() && instruction.operand_width == 4 {
                let destination = instruction.operand;
                if destination <= pc as i32 || destination > join as i32 {
                    return Err(fail("escaping or backward edge"));
                }
                pending.push((destination as usize, result));
            }
        }
    }
    if !joined {
        return Err(fail("does not reach its original join"));
    }
    for (pc, instruction) in instructions.iter().enumerate().take(end).skip(owner + 1) {
        if pc >= entry && pc < join {
            continue;
        }
        let mut destination = None;
        if instruction.opcode.is_branch() && instruction.operand_width == 4 {
            destination = Some(instruction.operand);
        } else if instruction.opcode == QvmOpcode::OpJump {
            if let Some(target) = pc.checked_sub(1).and_then(|at| instructions.get(at)) {
                if target.opcode == QvmOpcode::OpConst {
                    destination = Some(target.operand);
                }
            }
        }
        if let Some(target) = destination {
            if target > entry as i32 && (target as usize) < join {
                return Err(fail("incoming interior edge"));
            }
        }
    }
    Ok(first.operand.max(0) as usize)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RegionOperand {
    Local(usize),
    LocalDerived,
    Constant(i32),
    Unknown,
}

fn region_local(value: &RegionOperand) -> bool {
    matches!(value, RegionOperand::Local(_) | RegionOperand::LocalDerived)
}

/// Qualify a region evaluation with scalar live-ins; returns the frame size.
pub fn qualify_qvm_region_evaluation(
    instructions: &[QvmInstruction],
    owner: usize,
    region: &QvmRegionEvaluation,
    read_only: bool,
) -> Result<usize, GuestError> {
    let fail = |message: String| GuestError::invalid(format!("QVM region: {message}"));
    let frame = qualify_qvm_region(instructions, owner, region.entry, region.join)?;
    let valid = |offset: usize| offset >= 8 && offset % 4 == 0 && offset + 4 <= frame;
    if region.inputs.iter().any(|offset| !valid(*offset))
        || BTreeSet::from_iter(region.inputs.iter().copied()).len() != region.inputs.len()
        || region.result.is_some_and(|result| !valid(result))
    {
        return Err(fail("live-in or result is outside its source frame".to_string()));
    }
    let mut pending: BTreeMap<usize, (Vec<RegionOperand>, BTreeSet<usize>)> = BTreeMap::new();
    pending.insert(region.entry, (Vec::new(), BTreeSet::from_iter(region.inputs.iter().copied())));
    while let Some(pc) = pending.keys().next().copied() {
        let Some((stack, initialized)) = pending.remove(&pc) else {
            return Err(fail("missing input path".to_string()));
        };
        if pc == region.join {
            if region.result.is_some_and(|result| !initialized.contains(&result)) {
                return Err(fail("result is not initialized on every source path".to_string()));
            }
            continue;
        }
        let instruction = instructions.get(pc).ok_or_else(|| fail("missing instruction".to_string()))?;
        let mut stack = stack;
        let mut initialized = initialized;
        let opcode = instruction.opcode;
        if read_only && matches!(opcode, QvmOpcode::OpCall | QvmOpcode::OpArg | QvmOpcode::OpBlockCopy | QvmOpcode::OpBreak) {
            return Err(fail("read-only region cannot call, publish arguments, copy memory or break".to_string()));
        }
        let pop = |stack: &mut Vec<RegionOperand>| stack.pop().ok_or_else(|| fail("invalid operand proof".to_string()));
        if opcode == QvmOpcode::OpLocal {
            if instruction.operand < 8 || instruction.operand % 4 != 0 || instruction.operand as usize + 4 > frame + 48 {
                return Err(fail("local address exceeds its source frame and arguments".to_string()));
            }
            stack.push(RegionOperand::Local(instruction.operand as usize));
        } else if opcode == QvmOpcode::OpConst {
            stack.push(RegionOperand::Constant(instruction.operand));
        } else if opcode == QvmOpcode::OpPush {
            stack.push(RegionOperand::Unknown);
        } else if opcode == QvmOpcode::OpPop || opcode == QvmOpcode::OpArg {
            pop(&mut stack)?;
            if opcode == QvmOpcode::OpArg {
                initialized.insert(instruction.operand.max(0) as usize);
            }
        } else if (opcode as u8) >= (QvmOpcode::OpLoad1 as u8) && (opcode as u8) <= (QvmOpcode::OpLoad4 as u8) {
            let address = pop(&mut stack)?;
            if address == RegionOperand::LocalDerived {
                return Err(fail("reads an unresolved source local address".to_string()));
            }
            if let RegionOperand::Local(offset) = address {
                if offset < frame && !initialized.contains(&offset) {
                    return Err(fail(format!("reads undeclared source local {offset}")));
                }
            }
            stack.push(RegionOperand::Unknown);
        } else if (opcode as u8) >= (QvmOpcode::OpStore1 as u8) && (opcode as u8) <= (QvmOpcode::OpStore4 as u8) {
            if region_local(&pop(&mut stack)?) {
                return Err(fail("stores an escaping source local pointer".to_string()));
            }
            let address = pop(&mut stack)?;
            if read_only && !matches!(address, RegionOperand::Local(offset) if offset >= 8 && offset + 4 <= frame) {
                return Err(fail("read-only region cannot write outside its own local frame".to_string()));
            }
            if let RegionOperand::Local(offset) = address {
                if opcode == QvmOpcode::OpStore4 {
                    initialized.insert(offset);
                }
            }
        } else if opcode == QvmOpcode::OpBlockCopy {
            let source = pop(&mut stack)?;
            if source == RegionOperand::LocalDerived {
                return Err(fail("copies an unresolved source local address".to_string()));
            }
            if let RegionOperand::Local(offset) = source {
                let mut byte = 0;
                while byte < instruction.operand.max(0) as usize {
                    if !initialized.contains(&(offset + byte)) {
                        return Err(fail("copies an undeclared source local".to_string()));
                    }
                    byte += 4;
                }
            }
            let address = pop(&mut stack)?;
            if let RegionOperand::Local(offset) = address {
                let mut byte = 0;
                while byte + 4 <= instruction.operand.max(0) as usize {
                    initialized.insert(offset + byte);
                    byte += 4;
                }
            }
        } else if opcode == QvmOpcode::OpJump {
            let destination = pop(&mut stack)?;
            let RegionOperand::Constant(target) = destination else {
                return Err(fail("jump lost its source target".to_string()));
            };
            merge_region_path(&mut pending, target.max(0) as usize, stack, initialized);
            continue;
        } else if opcode.is_branch() && instruction.operand_width == 4 {
            pop(&mut stack)?;
            pop(&mut stack)?;
            merge_region_path(&mut pending, instruction.operand.max(0) as usize, stack.clone(), initialized.clone());
        } else if opcode == QvmOpcode::OpCall {
            pop(&mut stack)?;
            stack.push(RegionOperand::Unknown);
        } else if (opcode as u8) >= (QvmOpcode::OpAdd as u8) && (opcode as u8) <= (QvmOpcode::OpRshu as u8) && opcode != QvmOpcode::OpBcom
            || (opcode as u8) >= (QvmOpcode::OpAddf as u8) && (opcode as u8) <= (QvmOpcode::OpMulf as u8)
        {
            let right = pop(&mut stack)?;
            let left = pop(&mut stack)?;
            if (opcode == QvmOpcode::OpAdd || opcode == QvmOpcode::OpSub)
                && let RegionOperand::Local(base) = left
                && let RegionOperand::Constant(delta) = right
            {
                let offset = if opcode == QvmOpcode::OpAdd { base as i64 + delta as i64 } else { base as i64 - delta as i64 };
                stack.push(RegionOperand::Local(offset.max(0) as usize));
            } else {
                stack.push(if region_local(&left) || region_local(&right) {
                    RegionOperand::LocalDerived
                } else {
                    RegionOperand::Unknown
                });
            }
        } else if opcode != QvmOpcode::OpIgnore && opcode != QvmOpcode::OpBreak {
            let value = pop(&mut stack)?;
            stack.push(if region_local(&value) { RegionOperand::LocalDerived } else { RegionOperand::Unknown });
        }
        merge_region_path(&mut pending, pc + 1, stack, initialized);
    }
    Ok(frame)
}

fn merge_region_path(
    pending: &mut BTreeMap<usize, (Vec<RegionOperand>, BTreeSet<usize>)>,
    pc: usize,
    stack: Vec<RegionOperand>,
    initialized: BTreeSet<usize>,
) {
    match pending.remove(&pc) {
        None => {
            pending.insert(pc, (stack, initialized));
        }
        Some((before_stack, before_init)) => {
            let merged = stack
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    let before = before_stack.get(index);
                    let same = matches!((&value, before),
                        (RegionOperand::Local(a), Some(RegionOperand::Local(b))) if a == b)
                        || matches!((&value, before),
                        (RegionOperand::Constant(a), Some(RegionOperand::Constant(b))) if a == b);
                    if same {
                        value
                    } else if region_local(&value) || before.is_some_and(region_local) {
                        RegionOperand::LocalDerived
                    } else {
                        RegionOperand::Unknown
                    }
                })
                .collect();
            let init = initialized.intersection(&before_init).copied().collect();
            pending.insert(pc, (merged, init));
        }
    }
}

// ---------------------------------------------------------------------------
// Callback contract types (`src/contracts/qvm-mod-callbacks.ts`).
// ---------------------------------------------------------------------------

/// Scalar encoding of one source word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModScalar {
    /// Signed 32-bit integer.
    Int32,
    /// Binary32 float.
    Float32,
}

/// Callback input name (mirror of `ModCallbackInput`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModCallbackInput {
    /// View angles vector.
    ViewAngles,
    /// Attack input.
    Attack,
    /// Jump input.
    Jump,
    /// Impulse input.
    Impulse,
    /// Forward-move input.
    ForwardMove,
    /// Side-move input.
    SideMove,
    /// Up-move input.
    UpMove,
    /// Calling actor.
    Own,
    /// Other actor.
    Other,
    /// Activating actor.
    Activator,
    /// Attacking actor.
    Attacker,
    /// Inflicting actor.
    Inflictor,
    /// Damage/cargo amount.
    Amount,
    /// Lowered damage flags.
    DamageFlags,
    /// Regular-protection scale.
    RegularProtectionScale,
    /// Knockback.
    Knockback,
    /// Impact point.
    Point,
    /// Impact direction.
    Direction,
    /// Impact normal.
    Normal,
    /// Item identity.
    Item,
    /// Caller time.
    Time,
    /// Frame elapsed time.
    Elapsed,
    /// Observed result.
    Result,
    /// Pickup count override.
    PickupCount,
    /// Pickup has-count flag.
    PickupHasCount,
    /// Pickup dropped flag.
    PickupDropped,
}

impl ModCallbackInput {
    /// Declaration name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ViewAngles => "view-angles",
            Self::Attack => "attack",
            Self::Jump => "jump",
            Self::Impulse => "impulse",
            Self::ForwardMove => "forward-move",
            Self::SideMove => "side-move",
            Self::UpMove => "up-move",
            Self::Own => "self",
            Self::Other => "other",
            Self::Activator => "activator",
            Self::Attacker => "attacker",
            Self::Inflictor => "inflictor",
            Self::Amount => "amount",
            Self::DamageFlags => "damage-flags",
            Self::RegularProtectionScale => "regular-protection-scale",
            Self::Knockback => "knockback",
            Self::Point => "point",
            Self::Direction => "direction",
            Self::Normal => "normal",
            Self::Item => "item",
            Self::Time => "time",
            Self::Elapsed => "elapsed",
            Self::Result => "result",
            Self::PickupCount => "pickup-count",
            Self::PickupHasCount => "pickup-has-count",
            Self::PickupDropped => "pickup-dropped",
        }
    }
}

/// Client input name (mirror of `ModClientInput`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModClientInput {
    /// View angles vector.
    ViewAngles,
    /// Attack input.
    Attack,
    /// Jump input.
    Jump,
    /// Impulse input.
    Impulse,
    /// Forward-move input.
    ForwardMove,
    /// Side-move input.
    SideMove,
    /// Up-move input.
    UpMove,
}

impl ModClientInput {
    /// Lift to a callback input.
    #[must_use]
    pub const fn callback(self) -> ModCallbackInput {
        match self {
            Self::ViewAngles => ModCallbackInput::ViewAngles,
            Self::Attack => ModCallbackInput::Attack,
            Self::Jump => ModCallbackInput::Jump,
            Self::Impulse => ModCallbackInput::Impulse,
            Self::ForwardMove => ModCallbackInput::ForwardMove,
            Self::SideMove => ModCallbackInput::SideMove,
            Self::UpMove => ModCallbackInput::UpMove,
        }
    }
}

/// Declared callback value (mirror of `ModCallbackValue`).
#[derive(Debug, Clone, PartialEq)]
pub enum ModCallbackValue {
    /// Caller input reference.
    Input(ModCallbackInput),
    /// Constant scalar.
    Float(f64),
    /// Constant string.
    Str(String),
    /// Constant vector.
    Vec(Vec3),
}

/// Runtime callback value (mirror of `ModRuntimeValue`).
#[derive(Debug, Clone, PartialEq)]
pub enum ModRuntimeValue {
    /// Scalar.
    Float(f64),
    /// String.
    Str(String),
    /// Vector.
    Vec(Vec3),
    /// Actor reference.
    Actor(Option<ActorId>),
}

/// Actor-valued callback input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActorInput {
    /// Calling actor.
    Own,
    /// Other actor.
    Other,
    /// Activating actor.
    Activator,
    /// Attacking actor.
    Attacker,
    /// Inflicting actor.
    Inflictor,
}

impl ActorInput {
    /// Lift to a callback input.
    #[must_use]
    pub const fn callback(self) -> ModCallbackInput {
        match self {
            Self::Own => ModCallbackInput::Own,
            Self::Other => ModCallbackInput::Other,
            Self::Activator => ModCallbackInput::Activator,
            Self::Attacker => ModCallbackInput::Attacker,
            Self::Inflictor => ModCallbackInput::Inflictor,
        }
    }
}

/// Time-valued callback input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeInput {
    /// Caller time.
    Time,
    /// Frame elapsed time.
    Elapsed,
}

impl TimeInput {
    /// Lift to a callback input.
    #[must_use]
    pub const fn callback(self) -> ModCallbackInput {
        match self {
            Self::Time => ModCallbackInput::Time,
            Self::Elapsed => ModCallbackInput::Elapsed,
        }
    }
}

/// Time units of a `time` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeUnits {
    /// Seconds.
    Seconds,
    /// Milliseconds.
    Milliseconds,
}

/// One lowered source-call word (mirror of `QvmModValue`).
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModValue {
    /// Signed word.
    Int32(ModCallbackValue),
    /// Binary32 word.
    Float32(ModCallbackValue),
    /// Vector word (scratch pointer).
    Vector(ModCallbackValue),
    /// String word (scratch pointer).
    Str(ModCallbackValue),
    /// Actor projection pointer.
    Actor {
        /// Actor record id.
        record: String,
        /// Actor input.
        input: ActorInput,
    },
    /// Admitted client slot.
    Client {
        /// Actor input.
        input: ActorInput,
    },
    /// Time word.
    Time {
        /// Time input.
        input: TimeInput,
        /// Units.
        units: TimeUnits,
        /// Encoding.
        encoding: ModScalar,
    },
    /// Absolute address word.
    Address(i32),
}

/// One lowered source-call global.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModGlobal {
    /// Global address.
    pub address: usize,
    /// Lowered value.
    pub value: QvmModValue,
}

/// Source-call return shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModReturns {
    /// Signed word.
    Int32,
    /// Binary32 word.
    Float32,
    /// No return value.
    Void,
}

/// Declared original source call (mirror of `QvmModSourceCall`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModSourceCall {
    /// Function entry instruction.
    pub entry: usize,
    /// Argument words.
    pub arguments: Vec<QvmModValue>,
    /// Caller globals.
    pub globals: Vec<QvmModGlobal>,
    /// Return shape.
    pub returns: ModReturns,
}

/// Shared team value (mirror of `SourceTeamValue`).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceTeamValue {
    /// Original value.
    pub value: f64,
    /// Shared identity.
    pub team: Option<String>,
}

/// Validate one match field (mirror of `validateSourceMatchField`).
pub fn validate_source_match_field(field: &ModActorBinding) -> Result<(), GuestError> {
    let ModActorBinding::Team { values, .. } = field else { return Ok(()) };
    if values.is_empty()
        || values.iter().map(|entry| entry.value.to_bits()).collect::<HashSet<_>>().len() != values.len()
        || values.iter().map(|entry| entry.team.clone()).collect::<HashSet<_>>().len() != values.len()
        || values.iter().any(|entry| !entry.value.is_finite() || entry.team.as_ref().is_some_and(String::is_empty))
    {
        return Err(GuestError::invalid("Team projection requires distinct original values and shared identities"));
    }
    Ok(())
}

/// Projection direction of one actor field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldAccess {
    /// Canonical-to-source only.
    ReadOnly,
    /// Bidirectional.
    ReadWrite,
}

/// Actor-field binding (mirror of `QvmModActorField`).
#[derive(Debug, Clone, PartialEq)]
pub enum ModActorBinding {
    /// Shared team.
    Team {
        /// Encoding.
        encoding: ModScalar,
        /// Declared values.
        values: Vec<SourceTeamValue>,
    },
    /// Shared score.
    Score {
        /// Encoding.
        encoding: ModScalar,
    },
    /// Shared health.
    Health {
        /// Encoding.
        encoding: ModScalar,
    },
    /// Shared inventory count.
    Inventory {
        /// Encoding.
        encoding: ModScalar,
        /// Item identity.
        item: String,
    },
    /// Shared origin.
    Origin,
    /// Shared velocity.
    Velocity,
    /// Shared angles.
    Angles,
    /// Shared bounds minimum.
    BoundsMin,
    /// Shared bounds maximum.
    BoundsMax,
    /// Linked record pointer.
    Record {
        /// Record id.
        record: String,
    },
    /// Constant word.
    Constant {
        /// Encoding.
        encoding: ModScalar,
        /// Value.
        value: f64,
    },
    /// Constant vector.
    ConstantVector(Vec3),
    /// Private bytes.
    Private {
        /// Byte length.
        byte_length: usize,
    },
}

/// One declared actor-record field.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModActorField {
    /// Field offset.
    pub offset: usize,
    /// Projection direction, if restricted.
    pub access: Option<FieldAccess>,
    /// Binding.
    pub binding: ModActorBinding,
}

/// Byte size of one actor field.
#[must_use]
pub fn mod_field_size(field: &QvmModActorField) -> usize {
    match &field.binding {
        ModActorBinding::Private { byte_length } => *byte_length,
        ModActorBinding::Origin
        | ModActorBinding::Velocity
        | ModActorBinding::Angles
        | ModActorBinding::BoundsMin
        | ModActorBinding::BoundsMax
        | ModActorBinding::ConstantVector(_) => 12,
        _ => 4,
    }
}

/// Whether a field binding is a shared canonical projection.
#[must_use]
pub fn mod_field_is_shared(field: &QvmModActorField) -> bool {
    matches!(
        field.binding,
        ModActorBinding::Team { .. }
            | ModActorBinding::Score { .. }
            | ModActorBinding::Health { .. }
            | ModActorBinding::Inventory { .. }
            | ModActorBinding::Origin
            | ModActorBinding::Velocity
            | ModActorBinding::Angles
            | ModActorBinding::BoundsMin
            | ModActorBinding::BoundsMax
    )
}

/// Encode one value as a source word (mirror of `scalar`).
pub fn encode_mod_scalar(value: f64, encoding: ModScalar) -> Result<i32, GuestError> {
    if !value.is_finite() {
        return Err(GuestError::invalid(format!("Mod value exceeds {}", encoding_name(encoding))));
    }
    match encoding {
        ModScalar::Float32 => {
            let narrowed = value as f32;
            if !narrowed.is_finite() {
                return Err(GuestError::invalid("Mod value exceeds float32"));
            }
            Ok(narrowed.to_bits() as i32)
        }
        ModScalar::Int32 => {
            if value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
                return Err(GuestError::invalid("Mod value exceeds int32"));
            }
            Ok(value.trunc() as i32)
        }
    }
}

fn encoding_name(encoding: ModScalar) -> &'static str {
    match encoding {
        ModScalar::Int32 => "int32",
        ModScalar::Float32 => "float32",
    }
}

/// One declared source array (mirror of `QvmModActorRecord`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModActorRecord {
    /// Record id.
    pub id: String,
    /// Array base address.
    pub address: usize,
    /// Row stride.
    pub stride: usize,
    /// Row capacity.
    pub capacity: usize,
    /// Declared fields.
    pub fields: Vec<QvmModActorField>,
}

/// Original actor-frame loop (mirror of `QvmModActorFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModActorFrame {
    /// Frame call.
    pub call: QvmModSourceCall,
    /// Source clock store.
    pub clock: FrameClock,
    /// Owned-entity loop filters.
    pub owned: Vec<FrameFilter>,
    /// Loop completion edge.
    pub end: FrameEnd,
}

/// Source clock store of an actor frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameClock {
    /// Clock global address.
    pub address: usize,
    /// Store instruction.
    pub store: usize,
    /// Time argument index.
    pub argument: usize,
}

/// Owned-entity loop filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameFilter {
    /// Predicate instruction.
    pub instruction: usize,
    /// Local-address instruction.
    pub local_instruction: usize,
}

/// Loop completion edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameEnd {
    /// Decision instruction.
    pub instruction: usize,
    /// Taken direction at completion.
    pub completed_taken: bool,
}

/// Declared source-actor lifecycle (mirror of `QvmModSourceActors`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModSourceActors {
    /// Allocate entry.
    pub allocate: usize,
    /// Release entry and argument.
    pub release: SourceRelease,
    /// In-use field offset.
    pub inuse: usize,
    /// Event entity-type boundary.
    pub event_entity_type: u32,
    /// Per-actor update call, if any.
    pub update: Option<QvmModSourceCall>,
    /// Original frame loop, if any.
    pub frame: Option<QvmModActorFrame>,
    /// Bootstrap store instructions.
    pub initial_stores: Vec<usize>,
    /// Reaction field offsets, if any.
    pub callbacks: Option<SourceReactionFields>,
}

/// Release entry and argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceRelease {
    /// Entry instruction.
    pub entry: usize,
    /// Pointer argument index.
    pub argument: usize,
}

/// Reaction field offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SourceReactionFields {
    /// Touch callback offset.
    pub touch: Option<usize>,
    /// Use callback offset.
    pub use_: Option<usize>,
    /// Pain callback offset.
    pub pain: Option<usize>,
    /// Die callback offset.
    pub die: Option<usize>,
}

/// Declared combat calls (mirror of `QvmModCombatCalls`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModCombatCalls {
    /// Damage call.
    pub damage: super::primary_player_profile::QvmCombatCall,
    /// Touch call.
    pub touch: super::primary_player_profile::QvmCombatCall,
    /// Use call.
    pub use_: super::primary_player_profile::QvmCombatCall,
    /// Pain call.
    pub pain: super::primary_player_profile::QvmCombatCall,
    /// Die call.
    pub die: super::primary_player_profile::QvmCombatCall,
}

/// Declared source combat (mirror of `QvmModCombat`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModCombat {
    /// Damage entry.
    pub entry: usize,
    /// Health field offset.
    pub health: usize,
    /// Take-damage field offset.
    pub takedamage: usize,
    /// Flags field offset.
    pub flags: usize,
    /// God-mode mask.
    pub godmode: u32,
    /// No-knockback mask.
    pub no_knockback: u32,
    /// Damage globals.
    pub globals: Vec<QvmModGlobal>,
    /// Client combat record, if any.
    pub client: Option<QvmModCombatClient>,
    /// Combat ABI.
    pub abi: QvmModCombatAbi,
}

/// Client combat record.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModCombatClient {
    /// Client pointer offset.
    pub pointer: usize,
    /// Client record id.
    pub record: String,
    /// Health offset.
    pub health: usize,
    /// Armor offset.
    pub armor: usize,
    /// Protection fraction.
    pub protection: f64,
    /// Team offset.
    pub team: usize,
}

/// Combat ABI shape.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModCombatAbi {
    /// Original `G_Damage` ABI.
    Q3GDamage,
    /// Declared calls.
    Declared {
        /// Calls.
        calls: QvmModCombatCalls,
        /// Damage flag masks.
        damage_flags: super::primary_player_profile::QvmDamageFlags,
        /// Mass source.
        mass: super::primary_player_profile::QvmCombatMass,
        /// Team mapping.
        teams: Vec<super::primary_player_profile::QvmCombatTeam>,
    },
}

/// Protection channel (mirror of `ProtectionChannel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtectionChannel {
    /// Regular armor.
    Regular,
    /// Powered armor.
    Powered,
}

/// Inventory write fields of one pickup rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InventoryWriteFields {
    /// Count only.
    Count,
    /// Capacity only.
    Capacity,
    /// Count and capacity.
    CountAndCapacity,
}

/// One pickup write (mirror of `PickupWrite`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PickupWrite {
    /// Protection write.
    Protection {
        /// Channel.
        channel: ProtectionChannel,
    },
    /// Inventory write.
    Inventory {
        /// Item identity.
        item: String,
        /// Written fields.
        fields: InventoryWriteFields,
    },
}

/// Grant acceptance of a gate-then-grant rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrantAccepts {
    /// Nonzero grant result.
    Nonzero,
    /// Always accepted.
    Always,
}

/// Pickup operation (mirror of `OriginalPickupOperation`).
#[derive(Debug, Clone, PartialEq)]
pub enum OriginalPickupOperation {
    /// Single grant call.
    BooleanGrant {
        /// Grant call.
        grant: QvmModSourceCall,
    },
    /// Gate then grant.
    GateThenGrant {
        /// Gate call.
        gate: QvmModSourceCall,
        /// Grant call.
        grant: QvmModSourceCall,
        /// Grant acceptance.
        grant_accepts: GrantAccepts,
    },
}

/// One pickup context word.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupContextField {
    /// Record id.
    pub record: String,
    /// Field offset.
    pub offset: usize,
    /// Lowered value.
    pub value: QvmModValue,
}

/// Declared pickup rule (mirror of `QvmModPickup`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModPickup {
    /// Rule id.
    pub id: String,
    /// Declared writes (nonempty).
    pub writes: Vec<PickupWrite>,
    /// Offered items.
    pub offered: Vec<String>,
    /// Operation.
    pub operation: OriginalPickupOperation,
    /// Context words.
    pub context: Vec<PickupContextField>,
}

/// Callback operation (mirror of `ModCallbackOperation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModCallbackOperation {
    /// Actor think.
    ActorThink,
    /// Actor touch.
    ActorTouch,
    /// Actor use.
    ActorUse,
    /// Actor pain.
    ActorPain,
    /// Actor die.
    /// Damage.
    Damage,
    /// Inventory give.
    InventoryGive,
    /// Inventory consume.
    InventoryConsume,
}

/// Callback stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallbackStage {
    /// Observe.
    Observe,
    /// Transform.
    Transform,
    /// Replace.
    Replace,
}

/// Callback result selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallbackResult {
    /// Damage amount.
    Amount,
    /// Knockback.
    Knockback,
    /// Boolean.
    Boolean,
}

/// Callback binding (mirror of `ModCallbackBinding`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModCallbackBinding {
    /// Callback id.
    pub id: String,
    /// Operation.
    pub operation: ModCallbackOperation,
    /// Stage.
    pub stage: CallbackStage,
    /// Result selector, if any.
    pub result: Option<CallbackResult>,
}

/// Declared source callback (mirror of `QvmModCallback`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModCallback {
    /// Binding.
    pub binding: ModCallbackBinding,
    /// Source call.
    pub call: QvmModSourceCall,
}

/// Client-output vector field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModOutputVector {
    /// Record id.
    pub record: String,
    /// Field offset.
    pub offset: usize,
}

/// Declared client output (mirror of `ModClientOutputDeclaration`).
#[derive(Debug, Clone, PartialEq)]
pub enum ModClientOutputDeclaration {
    /// Body shape bounds.
    BodyShape {
        /// Minimums field.
        min: ModOutputVector,
        /// Maximums field.
        max: ModOutputVector,
    },
    /// View-offset vector.
    ViewOffsetVec {
        /// Field.
        field: ModOutputVector,
    },
    /// View-offset height scalar.
    ViewOffsetHeight {
        /// Field.
        height: QvmModProtectionScalar,
    },
    /// Movement mode.
    MovementMode {
        /// Field.
        field: QvmModProtectionScalar,
        /// Mask, if any.
        mask: Option<u32>,
        /// Declared values.
        values: Vec<OutputModeValue>,
    },
    /// Stance.
    Stance {
        /// Field.
        field: QvmModProtectionScalar,
        /// Mask, if any.
        mask: Option<u32>,
        /// Declared values.
        values: Vec<OutputStanceValue>,
    },
}

/// Movement-mode output value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OutputModeValue {
    /// Source value.
    pub value: f64,
    /// Movement mode.
    pub mode: String,
}

/// Stance output value.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputStanceValue {
    /// Source value.
    pub value: f64,
    /// Crouched flag.
    pub crouched: bool,
}

impl ModClientOutputDeclaration {
    /// Channel name.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::BodyShape { .. } => "body-shape",
            Self::ViewOffsetVec { .. } | Self::ViewOffsetHeight { .. } => "view-offset",
            Self::MovementMode { .. } => "movement-mode",
            Self::Stance { .. } => "stance",
        }
    }
}

/// Validate client outputs (mirror of `validateModClientOutputs`).
pub fn validate_mod_client_outputs(
    declarations: &[ModClientOutputDeclaration],
    check_scalar: &dyn Fn(&QvmModProtectionScalar) -> Result<(), GuestError>,
    check_vector: &dyn Fn(&ModOutputVector) -> Result<(), GuestError>,
) -> Result<(), GuestError> {
    if declarations.iter().map(ModClientOutputDeclaration::kind).collect::<HashSet<_>>().len() != declarations.len() {
        return Err(GuestError::invalid("Duplicate source client output channel"));
    }
    for declaration in declarations {
        match declaration {
            ModClientOutputDeclaration::BodyShape { min, max } => {
                check_vector(min)?;
                check_vector(max)?;
            }
            ModClientOutputDeclaration::ViewOffsetVec { field } => check_vector(field)?,
            ModClientOutputDeclaration::ViewOffsetHeight { height } => check_scalar(height)?,
            ModClientOutputDeclaration::MovementMode { field, mask, values } => {
                check_scalar(field)?;
                check_output_values(*mask, &values.iter().map(|entry| entry.value).collect::<Vec<_>>())?;
            }
            ModClientOutputDeclaration::Stance { field, mask, values } => {
                check_scalar(field)?;
                check_output_values(*mask, &values.iter().map(|entry| entry.value).collect::<Vec<_>>())?;
            }
        }
    }
    Ok(())
}

fn check_output_values(mask: Option<u32>, values: &[f64]) -> Result<(), GuestError> {
    if let Some(mask) = mask {
        if mask == 0 {
            return Err(GuestError::invalid("Invalid source client output mask"));
        }
        for value in values {
            if value.fract() != 0.0 || *value < 0.0 || *value > f64::from(u32::MAX) || ((*value as u32) & mask) != (*value as u32) {
                return Err(GuestError::invalid("Source output values escape their declared mask"));
            }
        }
    }
    if values.is_empty()
        || values.iter().any(|value| !value.is_finite())
        || values.iter().map(f64::to_bits).collect::<HashSet<_>>().len() != values.len()
    {
        return Err(GuestError::invalid("Ambiguous source client output values"));
    }
    Ok(())
}

/// Input binding scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputScope {
    /// Client command.
    ClientCommand,
    /// Movement slice.
    MovementSlice,
}

/// Input output value.
#[derive(Debug, Clone, PartialEq)]
pub enum InputOutputValue {
    /// View angles.
    ViewAngles,
    /// Scalar input.
    Scalar {
        /// Input.
        input: ModClientInput,
        /// Encoding.
        encoding: ModScalar,
        /// Scale.
        scale: f64,
    },
}

/// Declared input output (mirror of `QvmModInputOutput`).
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModInputOutput {
    /// Field write.
    Field {
        /// Record id.
        record: String,
        /// Field offset.
        offset: usize,
        /// Value.
        value: InputOutputValue,
    },
    /// Handler call.
    Handler {
        /// Entry instruction.
        entry: usize,
        /// Actor record id.
        actor_record: String,
        /// Actor pointer.
        actor_pointer: QvmModInputPointer,
        /// Inputs.
        inputs: Vec<ModClientInput>,
        /// Forced return, if any.
        returns: Option<InputHandlerReturn>,
    },
    /// Command call.
    Command {
        /// Entry instruction.
        entry: usize,
        /// Actor record id.
        actor_record: String,
        /// Actor pointer.
        actor_pointer: QvmModInputPointer,
        /// Command pointer.
        command: QvmModInputPointer,
        /// Inputs.
        inputs: Vec<ModClientInput>,
    },
}

/// Forced handler return.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputHandlerReturn {
    /// Encoding.
    pub encoding: ModScalar,
    /// Value.
    pub value: f64,
}

/// Declared input pointer (mirror of `QvmModInputPointer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmModInputPointer {
    /// Pointer root.
    pub kind: InputPointerKind,
    /// Indirection offsets.
    pub indirections: Vec<usize>,
    /// Final offset.
    pub offset: usize,
}

/// Input pointer root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputPointerKind {
    /// Call argument word.
    Argument {
        /// Argument index.
        index: usize,
    },
    /// Global address.
    Global {
        /// Address.
        address: usize,
    },
}

/// Declared input binding phase.
#[derive(Debug, Clone, PartialEq)]
pub enum InputPhase {
    /// Before authoritative movement.
    Before {
        /// Outputs.
        outputs: Vec<QvmModInputOutput>,
    },
    /// After authoritative movement.
    After,
}

/// Declared client input binding.
#[derive(Debug, Clone, PartialEq)]
pub struct ModClientInputBinding {
    /// Scope.
    pub scope: InputScope,
    /// Calls.
    pub calls: Vec<QvmModSourceCall>,
    /// Phase.
    pub phase: InputPhase,
}

/// Declared source clients (mirror of `QvmModClients`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModClients {
    /// Client outputs.
    pub outputs: Vec<ModClientOutputDeclaration>,
    /// Maximum clients.
    pub maximum: usize,
    /// Client record ids.
    pub records: Vec<String>,
    /// Player-state record id.
    pub player_state_record: String,
    /// Admit calls.
    pub admit: Vec<QvmModSourceCall>,
    /// Userinfo calls.
    pub userinfo: Vec<QvmModSourceCall>,
    /// Disconnect calls.
    pub disconnect: Vec<QvmModSourceCall>,
    /// Frame calls, if any.
    pub frame: Vec<QvmModSourceCall>,
    /// Input bindings, if any.
    pub input: Vec<ModClientInputBinding>,
}

/// Objective address (mirror of `QvmModObjectiveAddress`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmModObjectiveAddress {
    /// Direct address.
    Direct(usize),
    /// Global pointer path.
    Pointer {
        /// Global address.
        address: usize,
        /// Indirection offsets.
        indirections: Vec<usize>,
        /// Final offset.
        offset: usize,
    },
}

/// Objective value declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceObjectiveValue {
    /// Source value.
    pub value: f64,
    /// Stage name.
    pub stage: String,
    /// Completion flag.
    pub complete: bool,
}

/// Objective role.
#[derive(Debug, Clone, PartialEq)]
pub enum ObjectiveRole<Call> {
    /// Owned objective.
    Owned {
        /// Campaign gate flag.
        campaign_gate: bool,
        /// Bot-goal flag.
        bot_goal: bool,
        /// Change call, if any.
        change: Option<Call>,
    },
    /// Borrowed objective.
    Borrowed {
        /// Writable flag.
        writable: bool,
    },
}

/// Objective declaration (mirror of `SourceObjectiveDeclaration`).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceObjectiveDeclaration<Storage, Reference, Call> {
    /// Objective id.
    pub id: String,
    /// State storage.
    pub storage: Storage,
    /// Declared values.
    pub values: Vec<SourceObjectiveValue>,
    /// Carrier reference, if any.
    pub carrier: Option<Reference>,
    /// Target reference, if any.
    pub target: Option<Reference>,
    /// Role.
    pub role: ObjectiveRole<Call>,
}

/// Objective scalar storage of a QVM mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmObjectiveStorage {
    /// Address.
    pub address: QvmModObjectiveAddress,
    /// Encoding.
    pub encoding: ModScalar,
}

/// Validate one objective address (mirror of `validateQvmObjectiveAddress`).
pub fn validate_qvm_objective_address(end: usize, address: &QvmModObjectiveAddress) -> Result<(), GuestError> {
    match address {
        QvmModObjectiveAddress::Direct(word) => {
            objective_word(end, *word)?;
        }
        QvmModObjectiveAddress::Pointer { address, indirections, offset } => {
            objective_word(end, *address)?;
            objective_word(end, *offset)?;
            for step in indirections {
                objective_word(end, *step)?;
            }
        }
    }
    Ok(())
}

fn objective_word(end: usize, address: usize) -> Result<usize, GuestError> {
    if address % 4 != 0 || address + 4 > end {
        return Err(GuestError::invalid("Objective address exceeds original QVM data or is not aligned"));
    }
    Ok(address)
}

/// Resolve one objective address (mirror of `resolveQvmObjectiveAddress`).
pub fn resolve_qvm_objective_address(
    read: &dyn Fn(usize) -> Result<i32, GuestError>,
    end: usize,
    address: &QvmModObjectiveAddress,
) -> Result<usize, GuestError> {
    match address {
        QvmModObjectiveAddress::Direct(word) => objective_word(end, *word),
        QvmModObjectiveAddress::Pointer { address, indirections, offset } => {
            let mut pointer = read(objective_word(end, *address)?)?;
            for step in indirections {
                if pointer == 0 {
                    return Err(GuestError::invalid("Objective selector follows a null source pointer"));
                }
                let base = objective_word(end, pointer.max(0) as usize)?;
                pointer = read(objective_word(end, base + *step)?)?;
            }
            if pointer == 0 {
                return Err(GuestError::invalid("Objective selector follows a null source pointer"));
            }
            let base = objective_word(end, pointer.max(0) as usize)?;
            objective_word(end, base + *offset)
        }
    }
}

/// Gameplay-mod callback declaration (mirror of `QvmModCallbackDeclaration`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModCallbackDeclaration {
    /// Declaration version (always 1).
    pub version: u32,
    /// Program path.
    pub program_path: String,
    /// Program digest.
    pub program_digest: String,
    /// ABI profile.
    pub abi_profile: QvmAbi,
    /// Presentation declaration, if any.
    pub presentation: Option<QvmModPresentationDeclaration>,
    /// Spawn entities text, if any.
    pub spawn_entities: Option<String>,
    /// Source clients, if any.
    pub clients: Option<QvmModClients>,
    /// Actor records.
    pub actor_records: Vec<QvmModActorRecord>,
    /// Engine entity record id, if any.
    pub entity_record: Option<String>,
    /// Source actors, if any.
    pub source_actors: Option<QvmModSourceActors>,
    /// Source combat, if any.
    pub combat: Option<QvmModCombat>,
    /// Protection channels.
    pub protection: Vec<QvmModProtection>,
    /// Pickup rules.
    pub pickups: Vec<QvmModPickup>,
    /// Item definitions, if any.
    pub items: Option<QvmModItems>,
    /// Initialization calls.
    pub initialize: Vec<QvmModSourceCall>,
    /// Callbacks.
    pub callbacks: Vec<QvmModCallback>,
    /// Objectives.
    pub objectives: Vec<SourceObjectiveDeclaration<QvmObjectiveStorage, QvmModObjectiveAddress, QvmModSourceCall>>,
}

/// Whether a record id names a client record.
#[must_use]
pub fn is_client_record(declaration: &QvmModCallbackDeclaration, record: &QvmModActorRecord) -> bool {
    declaration.clients.as_ref().is_some_and(|clients| clients.records.contains(&record.id))
}

// ---------------------------------------------------------------------------
// Declaration validation.
// ---------------------------------------------------------------------------

fn check_data_range(end: usize, address: usize, size: usize) -> Result<(), GuestError> {
    if size < 1 || address.saturating_add(size) > end {
        return Err(GuestError::invalid("QVM mod declaration exceeds source data"));
    }
    Ok(())
}

fn check_value(
    value: &QvmModValue,
    available: &BTreeSet<ModCallbackInput>,
    declaration: &QvmModCallbackDeclaration,
    records: &HashMap<String, QvmModActorRecord>,
    end: usize,
) -> Result<(), GuestError> {
    match value {
        QvmModValue::Address(word) => {
            if *word != 0 {
                if *word < 0 {
                    return Err(GuestError::invalid("QVM mod declaration exceeds source data"));
                }
                check_data_range(end, *word as usize, 1)?;
            }
            Ok(())
        }
        QvmModValue::Actor { record, input } => {
            if !records.contains_key(record) || !available.contains(&input.callback()) {
                return Err(GuestError::invalid("Invalid QVM actor argument"));
            }
            Ok(())
        }
        QvmModValue::Client { input } => {
            if declaration.clients.is_none() || !available.contains(&input.callback()) {
                return Err(GuestError::invalid("Invalid QVM client argument"));
            }
            Ok(())
        }
        QvmModValue::Time { input, .. } => {
            if !available.contains(&input.callback()) {
                return Err(GuestError::invalid("Unavailable QVM time input"));
            }
            Ok(())
        }
        QvmModValue::Int32(inner) | QvmModValue::Float32(inner) | QvmModValue::Vector(inner) | QvmModValue::Str(inner) => {
            let kind = match inner {
                ModCallbackValue::Input(name) => match name {
                    ModCallbackInput::Own | ModCallbackInput::Other | ModCallbackInput::Activator | ModCallbackInput::Attacker | ModCallbackInput::Inflictor => "actor",
                    ModCallbackInput::Point | ModCallbackInput::Direction | ModCallbackInput::Normal | ModCallbackInput::ViewAngles => "vector",
                    ModCallbackInput::Item => "string",
                    _ => "float",
                },
                ModCallbackValue::Float(_) => "float",
                ModCallbackValue::Str(_) => "string",
                ModCallbackValue::Vec(_) => "vector",
            };
            if let ModCallbackValue::Input(name) = inner {
                if !available.contains(name) {
                    let name = name.name();
                    return Err(GuestError::invalid(format!("Unavailable QVM callback input {name}")));
                }
            }
            let expected = match value {
                QvmModValue::Int32(_) | QvmModValue::Float32(_) => "float",
                QvmModValue::Vector(_) => "vector",
                _ => "string",
            };
            if kind != expected {
                return Err(GuestError::invalid("QVM callback value has an incompatible representation"));
            }
            if let ModCallbackValue::Float(number) = inner {
                let encoding = match value {
                    QvmModValue::Int32(_) => ModScalar::Int32,
                    QvmModValue::Float32(_) => ModScalar::Float32,
                    _ => return Ok(()),
                };
                encode_mod_scalar(*number, encoding)?;
            }
            Ok(())
        }
    }
}

fn check_call(
    call: &QvmModSourceCall,
    available: &BTreeSet<ModCallbackInput>,
    declaration: &QvmModCallbackDeclaration,
    records: &HashMap<String, QvmModActorRecord>,
    image: &QvmImage,
) -> Result<(), GuestError> {
    if image.instruction(call.entry).is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
        || call.arguments.len() > QVM_MAX_PRIVATE_ARGUMENT_WORDS
    {
        return Err(GuestError::invalid("QVM mod callback requires a source function entry and bounded OP_ARG arguments"));
    }
    let end = image.data_end();
    for value in &call.arguments {
        check_value(value, available, declaration, records, end)?;
    }
    let mut globals = HashSet::new();
    for global in &call.globals {
        let size = if matches!(global.value, QvmModValue::Vector(_)) { 12 } else { 4 };
        check_data_range(end, global.address, size)?;
        if global.address % 4 != 0 {
            return Err(GuestError::invalid("Unaligned QVM callback global"));
        }
        for byte in global.address..global.address + size {
            if !globals.insert(byte) {
                return Err(GuestError::invalid("Overlapping QVM callback globals"));
            }
        }
        check_value(&global.value, available, declaration, records, end)?;
    }
    Ok(())
}

fn available_inputs(names: &[ModCallbackInput]) -> BTreeSet<ModCallbackInput> {
    names.iter().copied().collect()
}

/// Resolve bootstrap constant stores (mirror of `qvmActorBootstrap`).
pub fn qvm_actor_bootstrap(
    stores: &[usize],
    image: &QvmImage,
    records: &[QvmModActorRecord],
) -> Result<Vec<(usize, i32)>, GuestError> {
    let mut destinations = HashSet::new();
    stores
        .iter()
        .map(|pc| {
            let store = image.instruction(*pc);
            let address = pc.checked_sub(2).and_then(|at| image.instruction(at));
            let value = pc.checked_sub(1).and_then(|at| image.instruction(at));
            let (Some(store), Some(address), Some(value)) = (store, address, value) else {
                return Err(GuestError::invalid("QVM source bootstrap is not an original constant store"));
            };
            if store.opcode != QvmOpcode::OpStore4 || address.opcode != QvmOpcode::OpConst || value.opcode != QvmOpcode::OpConst {
                return Err(GuestError::invalid("QVM source bootstrap is not an original constant store"));
            }
            let offset = address.operand;
            let end = image.initialized_length + image.bss_length;
            if offset < 0
                || offset % 4 != 0
                || offset as usize + 4 > end
                || offset as usize + 4 > image.data_length && (offset as usize) < image.initialized_length
                || !destinations.insert(offset)
                || records.iter().any(|record| offset as usize + 4 > record.address && (offset as usize) < record.address + record.stride * record.capacity)
            {
                return Err(GuestError::invalid("QVM source bootstrap overlaps projected or nonwritable storage"));
            }
            Ok((offset as usize, value.operand))
        })
        .collect()
}

/// Validate one actor-frame loop (mirror of `validateQvmModActorFrame`).
pub fn validate_qvm_mod_actor_frame(
    definition: &QvmModActorFrame,
    image: &QvmImage,
    record: &QvmModActorRecord,
    inuse: usize,
) -> Result<(), GuestError> {
    let entry = definition.call.entry;
    let enter = image.instruction(entry);
    if enter.is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
        || definition.call.returns != ModReturns::Void
        || definition.owned.is_empty()
    {
        return Err(GuestError::invalid("QVM actor frame requires its original void caller and entity loops"));
    }
    let frame = enter.map_or(0, |instruction| instruction.operand.max(0) as usize);
    let end = image.function_end(entry);
    let mut decisions = HashSet::new();
    let mut branch = |pc: usize| -> Result<(), GuestError> {
        let instruction = image.instruction(pc);
        if pc <= entry || pc >= end || !decisions.insert(pc) || instruction.is_none_or(|value| !value.opcode.is_branch()) {
            return Err(GuestError::invalid("QVM actor frame predicate is not a distinct original conditional"));
        }
        Ok(())
    };
    branch(definition.end.instruction)?;
    for filter in &definition.owned {
        branch(filter.instruction)?;
        let local = image.instruction(filter.local_instruction);
        let offset = filter.local_instruction.checked_add(2).and_then(|at| image.instruction(at));
        let zero = filter.local_instruction.checked_add(5).and_then(|at| image.instruction(at));
        let local_ok = local.is_some_and(|value| value.opcode == QvmOpcode::OpLocal && value.operand >= 8 && value.operand as usize + 4 <= frame);
        let load_ok = filter.local_instruction.checked_add(1).and_then(|at| image.instruction(at)).is_some_and(|value| value.opcode == QvmOpcode::OpLoad4);
        let offset_ok = offset.is_some_and(|value| value.opcode == QvmOpcode::OpConst && value.operand == inuse as i32);
        let add_ok = filter.local_instruction.checked_add(3).and_then(|at| image.instruction(at)).is_some_and(|value| value.opcode == QvmOpcode::OpAdd);
        let load2_ok = filter.local_instruction.checked_add(4).and_then(|at| image.instruction(at)).is_some_and(|value| value.opcode == QvmOpcode::OpLoad4);
        let zero_ok = zero.is_some_and(|value| value.opcode == QvmOpcode::OpConst && value.operand == 0);
        let ne_ok = image.instruction(filter.instruction).is_some_and(|value| value.opcode == QvmOpcode::OpNe);
        if filter.local_instruction + 6 != filter.instruction || !local_ok || !load_ok || !offset_ok || !add_ok || !load2_ok || !zero_ok || !ne_ok || inuse + 4 > record.stride {
            return Err(GuestError::invalid("QVM actor frame filter differs from its original local entity/in-use predicate"));
        }
    }
    let clock = &definition.clock;
    let argument = definition.call.arguments.get(clock.argument);
    let address = clock.store.checked_sub(3).and_then(|at| image.instruction(at));
    let local = clock.store.checked_sub(2).and_then(|at| image.instruction(at));
    let argument_ok = matches!(argument, Some(QvmModValue::Time { input: TimeInput::Time, units: TimeUnits::Milliseconds, encoding: ModScalar::Int32 }))
        && clock.argument < QVM_MAX_PRIVATE_ARGUMENT_WORDS;
    let address_ok = clock.address % 4 == 0 && clock.address + 4 <= image.initialized_length + image.bss_length;
    let store_ok = clock.store > entry && clock.store < end;
    let const_ok = address.is_some_and(|value| value.opcode == QvmOpcode::OpConst && value.operand == clock.address as i32);
    let local_ok = local.is_some_and(|value| value.opcode == QvmOpcode::OpLocal && value.operand == frame as i32 + 8 + clock.argument as i32 * 4);
    let load_ok = clock.store.checked_sub(1).and_then(|at| image.instruction(at)).is_some_and(|value| value.opcode == QvmOpcode::OpLoad4);
    let store_op_ok = image.instruction(clock.store).is_some_and(|value| value.opcode == QvmOpcode::OpStore4);
    let global_ok = !definition.call.globals.iter().any(|global| global.address == clock.address);
    if !argument_ok || !address_ok || !store_ok || !const_ok || !local_ok || !load_ok || !store_op_ok || !global_ok {
        return Err(GuestError::invalid("QVM actor frame clock differs from its original argument store"));
    }
    Ok(())
}

/// Validate actor semantics (mirror of `validateQvmModActors`).
pub fn validate_qvm_mod_actors_mirror(artifact: &QvmArtifact, declaration: &QvmModCallbackDeclaration) -> Result<(), GuestError> {
    use super::primary_player_profile::validate_qvm_combat_call;
    let callbacks = declaration.source_actors.as_ref().and_then(|actors| actors.callbacks.as_ref());
    let combat = declaration.combat.as_ref();
    if callbacks.is_none() && combat.is_none() {
        return Ok(());
    }
    let record = declaration.entity_record.as_deref().and_then(|id| declaration.actor_records.iter().find(|record| record.id == *id));
    let Some(record) = record else {
        return Err(GuestError::invalid("QVM actor callbacks and combat require declared source actors"));
    };
    if declaration.source_actors.is_none() {
        return Err(GuestError::invalid("QVM actor callbacks and combat require declared source actors"));
    }
    let field = |owner: &QvmModActorRecord, offset: usize| -> Result<(), GuestError> {
        if offset % 4 != 0 || offset + 4 > owner.stride {
            return Err(GuestError::invalid("QVM actor semantic field exceeds its declared record"));
        }
        Ok(())
    };
    if let Some(callbacks) = callbacks {
        for offset in [callbacks.touch, callbacks.use_, callbacks.pain, callbacks.die].into_iter().flatten() {
            field(record, offset)?;
        }
    }
    let Some(combat) = combat else { return Ok(()) };
    if artifact.image.instruction(combat.entry).is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter) {
        return Err(GuestError::invalid("QVM damage entry is not a source function"));
    }
    for offset in [combat.health, combat.takedamage, combat.flags] {
        field(record, offset)?;
    }
    let is_declared = matches!(combat.abi, QvmModCombatAbi::Declared { .. });
    let limit = if is_declared { u32::MAX } else { i32::MAX as u32 };
    for mask in [combat.godmode, combat.no_knockback] {
        if mask == 0 || mask > limit {
            return Err(GuestError::invalid("Invalid QVM combat flag mask"));
        }
    }
    if callbacks.is_none() {
        return Err(GuestError::invalid("QVM source damage requires declared reaction fields"));
    }
    if let QvmModCombatAbi::Declared { calls, damage_flags, mass, teams } = &combat.abi {
        let data_bytes = artifact.image.data_end();
        for call in [&calls.damage, &calls.touch, &calls.use_, &calls.pain, &calls.die] {
            validate_qvm_combat_call(call, data_bytes)?;
        }
        let mut used = 0u32;
        for mask in [damage_flags.radius, damage_flags.no_armor, damage_flags.no_knockback, damage_flags.no_protection, damage_flags.no_team_protection] {
            if mask == 0 || !mask.is_power_of_two() || used & mask != 0 {
                return Err(GuestError::invalid("QVM source damage flag masks overlap or are not single positive bits"));
            }
            used |= mask;
        }
        match mass {
            super::primary_player_profile::QvmCombatMass::Entity { offset, .. } => field(record, *offset)?,
            super::primary_player_profile::QvmCombatMass::Constant(value) => {
                if !value.is_finite() || *value < 0.0 {
                    return Err(GuestError::invalid("Invalid QVM source mass"));
                }
            }
        }
        if combat.client.is_none() && !teams.is_empty() {
            return Err(GuestError::invalid("QVM source team mapping requires a client team field"));
        }
        let mut seen = HashSet::new();
        for team in teams {
            if !seen.insert(team.value) || team.team.find(':').is_none_or(|colon| colon == 0 || colon + 1 >= team.team.len()) {
                return Err(GuestError::invalid("Invalid QVM source team mapping"));
            }
        }
    }
    if let Some(client) = &combat.client {
        field(record, client.pointer)?;
        let client_record = declaration.actor_records.iter().find(|record| record.id == client.record);
        let Some(client_record) = client_record else {
            return Err(GuestError::invalid("QVM combat client has no declared record"));
        };
        for offset in [client.health, client.armor, client.team] {
            field(client_record, offset)?;
        }
        if !client.protection.is_finite() || client.protection < 0.0 || client.protection > 1.0 {
            return Err(GuestError::invalid("Invalid QVM armor protection"));
        }
    }
    Ok(())
}

fn is_const(image: &QvmImage, pc: usize) -> Result<(), GuestError> {
    if image.instruction(pc).is_none_or(|instruction| instruction.opcode != QvmOpcode::OpConst) {
        return Err(GuestError::invalid("QVM item capacity exceeds its source ABI"));
    }
    Ok(())
}

/// Validate item storage (mirror of `validateQvmItemStorage`).
pub fn validate_qvm_item_storage_mirror(
    storage: &[QvmItemStorage],
    items: &HashSet<String>,
    image: &QvmImage,
    field: &dyn Fn(&QvmItemField, bool) -> Result<(), GuestError>,
) -> Result<(), GuestError> {
    use super::mod_weapon_stage::QvmItemCapacity;
    let mut bound = HashSet::new();
    for entry in storage {
        field(entry.field(), false)?;
        match entry {
            QvmItemStorage::Counter { item, capacity, .. } => {
                if !items.contains(item) || !bound.insert(item.clone()) {
                    return Err(GuestError::invalid("QVM item lacks distinct declared storage"));
                }
                match capacity {
                    QvmItemCapacity::Field(field_ref) => field(field_ref, true)?,
                    QvmItemCapacity::Constant(value) => {
                        if *value < 0 {
                            return Err(GuestError::invalid("QVM item capacity exceeds its source ABI"));
                        }
                    }
                    QvmItemCapacity::Source { instruction, overrides } => {
                        is_const(image, *instruction)?;
                        for value in overrides {
                            is_const(image, value.instruction)?;
                            if value.address % 4 != 0 || value.address + 4 > image.initialized_length + image.bss_length {
                                return Err(GuestError::invalid("QVM capacity selector exceeds original source storage"));
                            }
                        }
                    }
                }
            }
            QvmItemStorage::Bits { private_mask, items: packed, .. } => {
                if packed.is_empty() {
                    return Err(GuestError::invalid("Invalid QVM private inventory mask"));
                }
                let mut mask = *private_mask;
                for value in packed {
                    if !items.contains(&value.item) || !bound.insert(value.item.clone()) {
                        return Err(GuestError::invalid("QVM item lacks distinct declared storage"));
                    }
                    if value.mask < 1 || !value.mask.is_power_of_two() || mask & value.mask != 0 {
                        return Err(GuestError::invalid("QVM packed item masks overlap"));
                    }
                    mask |= value.mask;
                }
            }
        }
    }
    if bound.len() != items.len() {
        return Err(GuestError::invalid("QVM item definition has no source storage"));
    }
    Ok(())
}

/// Validate item definitions (mirror of `validateQvmModItems`).
pub fn validate_qvm_mod_items_mirror(
    items: &QvmModItems,
    declaration: &QvmModCallbackDeclaration,
    image: &QvmImage,
) -> Result<(), GuestError> {
    use super::mod_weapon_stage::{QvmItemDefinitionKind, validate_qvm_weapon_stage};
    let Some(clients) = declaration.clients.as_ref() else {
        return Err(GuestError::invalid("QVM items require source client admission"));
    };
    if items.definitions.is_empty() {
        return Err(GuestError::invalid("QVM items require source client admission"));
    }
    let defined: HashSet<&String> = items.definitions.iter().map(|value| &value.item).collect();
    if defined.len() != items.definitions.len() {
        return Err(GuestError::invalid("Duplicate QVM source item definition"));
    }
    let mut occupied: HashMap<(String, usize), bool> = HashMap::new();
    let field = |source: &QvmItemField, usage_capacity: bool, view: bool| -> Result<(), GuestError> {
        let record = declaration.actor_records.iter().find(|value| value.id == source.record);
        let previous = occupied.get(&(source.record.clone(), source.offset)).copied();
        if record.is_none_or(|record| !clients.records.contains(&record.id))
            || source.offset % 4 != 0
            || source.offset + 4 > record.map_or(0, |record| record.stride)
            || !view && previous.is_some_and(|was_capacity| !(usage_capacity && was_capacity))
        {
            return Err(GuestError::invalid("Invalid QVM item source field"));
        }
        if !view {
            occupied.insert((source.record.clone(), source.offset), usage_capacity);
        }
        let record = record.expect("checked record");
        for value in &record.fields {
            let length = mod_field_size(value);
            if value.offset < source.offset + 4
                && source.offset < value.offset + length
                && !matches!(value.binding, ModActorBinding::Private { .. } | ModActorBinding::Constant { .. })
            {
                return Err(GuestError::invalid("QVM item field overlaps another canonical source projection"));
            }
        }
        Ok(())
    };
    validate_qvm_item_storage_mirror(
        &items.storage,
        &defined.into_iter().cloned().collect(),
        image,
        &|source, capacity| field(source, capacity, false),
    )?;
    let weapons: Vec<_> = items.definitions.iter().filter(|value| matches!(value.kind, QvmItemDefinitionKind::Weapon { .. })).collect();
    if (weapons.is_empty()) != (items.weapons.is_none()) {
        return Err(GuestError::invalid("QVM weapon items require an original source consumer"));
    }
    let Some(weapons_decl) = items.weapons.as_ref() else { return Ok(()) };
    let stage = &weapons_decl.stage;
    validate_qvm_weapon_stage(stage, image)?;
    let pointer = |source: &QvmModInputPointer| -> Result<(), GuestError> {
        match source.kind {
            InputPointerKind::Argument { index } => {
                if index >= QVM_MAX_PRIVATE_ARGUMENT_WORDS {
                    return Err(GuestError::invalid("QVM weapon pointer exceeds its original call ABI"));
                }
            }
            InputPointerKind::Global { address } => {
                if address % 4 != 0 || address + 4 > image.initialized_length + image.bss_length {
                    return Err(GuestError::invalid("QVM weapon pointer exceeds its original call ABI"));
                }
            }
        }
        for value in source.indirections.iter().copied().chain([source.offset]) {
            if value % 4 != 0 {
                return Err(GuestError::invalid("QVM weapon pointer is not aligned source storage"));
            }
        }
        Ok(())
    };
    for source in [&stage.dispatcher.actor, &stage.continuation.actor] {
        if !clients.records.contains(&source.record) {
            return Err(GuestError::invalid("QVM weapon pointer is not an admitted client record"));
        }
        pointer(&source.pointer)?;
    }
    let projection = &stage.continuation.projection;
    pointer(&projection.movement)?;
    if projection.byte_length < 12 || projection.byte_length % 4 != 0 {
        return Err(GuestError::invalid("QVM weapon movement projection lacks its caller layout"));
    }
    for offset in [projection.minimum, projection.maximum] {
        if offset % 4 != 0 || offset + 12 > projection.byte_length {
            return Err(GuestError::invalid("QVM weapon bounds exceed their original caller storage"));
        }
    }
    if projection.minimum.abs_diff(projection.maximum) < 12 {
        return Err(GuestError::invalid("QVM weapon caller bounds overlap"));
    }
    let entry = weapons_decl.input.entry;
    let matching: Vec<_> = clients
        .input
        .iter()
        .filter(|binding| binding.calls.iter().any(|call| call.entry == entry))
        .collect();
    if image.instruction(entry).is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
        || matching.len() != 1
        || !matches!(matching[0].scope, InputScope::MovementSlice)
        || !matches!(matching[0].phase, InputPhase::After)
    {
        return Err(GuestError::invalid("QVM weapon input requires one declared original callback after authoritative movement"));
    }
    field(&stage.selection.field, false, true)?;
    field(&weapons_decl.input.clock, false, true)?;
    field(&projection.view_height, false, true)?;
    field(&projection.ground, false, true)?;
    for value in stage.settled.iter().chain(stage.request.accepted.iter()).chain(stage.continuation.when.iter()) {
        field(&value.field, false, true)?;
        if value.mask.is_some_and(|mask| mask < 0 || mask > 0x7fff_ffff) {
            return Err(GuestError::invalid("Invalid QVM weapon source state predicate"));
        }
    }
    if stage.selection.values.len() != weapons.len()
        || stage.selection.values.iter().map(|value| &value.item).collect::<HashSet<_>>().len() != weapons.len()
        || stage.selection.values.iter().map(|value| value.value).collect::<HashSet<_>>().len() != weapons.len()
        || stage.selection.values.iter().any(|value| value.value < 1 || !weapons.iter().any(|weapon| weapon.item == value.item))
    {
        return Err(GuestError::invalid("QVM weapon selection differs from its admitted definitions"));
    }
    for weapon in &weapons {
        if let QvmItemDefinitionKind::Weapon { ammo: Some(ammo), .. } = &weapon.kind {
            if !items.definitions.iter().any(|definition| &definition.item == ammo) {
                return Err(GuestError::invalid("QVM weapon ammo lacks its source storage"));
            }
        }
    }
    Ok(())
}

/// Validate pickup rules (mirror of `validateQvmModPickups`).
pub fn validate_qvm_mod_pickups_mirror(declaration: &QvmModCallbackDeclaration) -> Result<(), GuestError> {
    let mut ids = HashSet::new();
    let mut offered = HashSet::new();
    for rule in &declaration.pickups {
        if declaration.clients.is_none() || !ids.insert(rule.id.clone()) || rule.id.is_empty() || rule.offered.is_empty() || rule.writes.is_empty() {
            return Err(GuestError::invalid("QVM pickups require unique rules and source client admission"));
        }
        for item in &rule.offered {
            if !offered.insert(item.clone()) {
                return Err(GuestError::invalid("Ambiguous QVM original pickup item"));
            }
        }
        for resource in &rule.writes {
            match resource {
                PickupWrite::Protection { channel } => {
                    if !declaration.protection.iter().any(|protection| protection.channel() == *channel) {
                        return Err(GuestError::invalid("QVM pickup has no protection owner"));
                    }
                }
                PickupWrite::Inventory { item, fields } => {
                    let projected = *fields == InventoryWriteFields::Count
                        && declaration.actor_records.iter().flat_map(|record| &record.fields).any(|field| {
                            matches!(&field.binding, ModActorBinding::Inventory { item: bound, .. } if bound == item)
                        });
                    let owned = declaration.items.as_ref().is_some_and(|items| {
                        items.storage.iter().any(|storage| match storage {
                            QvmItemStorage::Bits { items: packed, .. } => {
                                *fields == InventoryWriteFields::Count && packed.iter().any(|entry| &entry.item == item)
                            }
                            QvmItemStorage::Counter { item: stored, capacity, .. } => {
                                stored == item
                                    && (*fields == InventoryWriteFields::Count || matches!(capacity, super::mod_weapon_stage::QvmItemCapacity::Field(_)))
                            }
                        })
                    });
                    if !projected && !owned {
                        return Err(GuestError::invalid("QVM pickup has no declared inventory storage for its requested count/capacity writes"));
                    }
                }
            }
        }
        match &rule.operation {
            OriginalPickupOperation::BooleanGrant { grant } => {
                if grant.returns == ModReturns::Void {
                    return Err(GuestError::invalid("QVM pickup requires its declared source decision"));
                }
            }
            OriginalPickupOperation::GateThenGrant { gate, grant, grant_accepts } => {
                if gate.returns == ModReturns::Void || (*grant_accepts == GrantAccepts::Nonzero && grant.returns == ModReturns::Void) {
                    return Err(GuestError::invalid("QVM pickup requires its declared source decision"));
                }
            }
        }
        let mut occupied = HashSet::new();
        for field in &rule.context {
            let record = declaration.actor_records.iter().find(|record| record.id == field.record);
            let key = (field.record.clone(), field.offset);
            if record.is_none_or(|record| declaration.clients.as_ref().is_some_and(|clients| clients.records.contains(&record.id)))
                || !occupied.insert(key)
                || field.offset % 4 != 0
                || field.offset + 4 > record.map_or(0, |record| record.stride)
            {
                return Err(GuestError::invalid("Invalid QVM pickup source context"));
            }
            let record = record.expect("checked record");
            for binding in &record.fields {
                let width = mod_field_size(binding);
                if binding.offset < field.offset + 4
                    && field.offset < binding.offset + width
                    && !matches!(binding.binding, ModActorBinding::Private { .. } | ModActorBinding::Constant { .. })
                {
                    return Err(GuestError::invalid("QVM pickup context overlaps shared or linked storage"));
                }
            }
            if let Some(source) = declaration.source_actors.as_ref() {
                if Some(record.id.as_str()) == declaration.entity_record.as_deref()
                    && let Some(callbacks) = source.callbacks.as_ref()
                {
                    if field.offset == source.inuse
                        || [callbacks.touch, callbacks.use_, callbacks.pain, callbacks.die].into_iter().flatten().any(|offset| offset == field.offset)
                    {
                        return Err(GuestError::invalid("QVM pickup context overlaps source actor lifetime"));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Validate a gameplay-mod declaration against its artifact.
pub fn validate_qvm_mod(artifact: &QvmArtifact, declaration: &QvmModCallbackDeclaration) -> Result<(), GuestError> {
    if declaration.version != 1 {
        return Err(GuestError::invalid("Gameplay mod differs from its declared QVM artifact or ABI"));
    }
    if artifact.module.digest != declaration.program_digest
        || artifact.module.artifact_path != declaration.program_path
        || artifact.role != QvmRole::Qagame
        || artifact.abi() != declaration.abi_profile
    {
        return Err(GuestError::invalid("Gameplay mod differs from its declared QVM artifact or ABI"));
    }
    let end = artifact.image.data_end();
    let mut records: HashMap<String, QvmModActorRecord> = HashMap::new();
    let mut canonical = HashSet::new();
    for record in &declaration.actor_records {
        if records.contains_key(&record.id) || record.id.is_empty() || record.address == 0 || record.address % 4 != 0 || record.stride < 4 || record.stride % 4 != 0 || record.capacity < 1 || record.capacity > 1024 {
            return Err(GuestError::invalid("Invalid QVM mod actor array"));
        }
        check_data_range(end, record.address, record.stride * record.capacity)?;
        for previous in records.values() {
            if record.address < previous.address + previous.stride * previous.capacity && previous.address < record.address + record.stride * record.capacity {
                return Err(GuestError::invalid("Overlapping QVM mod actor arrays"));
            }
        }
        records.insert(record.id.clone(), record.clone());
        let mut occupied = HashSet::new();
        for field in &record.fields {
            if matches!(field.binding, ModActorBinding::Team { .. } | ModActorBinding::Score { .. }) {
                validate_source_match_field(&field.binding)?;
            }
            let size = mod_field_size(field);
            if field.offset % 4 != 0 || size < 1 || field.offset + size > record.stride {
                return Err(GuestError::invalid("QVM mod actor field exceeds its source record"));
            }
            for byte in field.offset..field.offset + size {
                if !occupied.insert(byte) {
                    return Err(GuestError::invalid("Overlapping QVM mod actor fields"));
                }
            }
            if field.access.is_some() && !mod_field_is_shared(field) {
                return Err(GuestError::invalid("QVM projection direction requires a canonical field"));
            }
            if mod_field_is_shared(field) && field.access != Some(FieldAccess::ReadOnly) {
                let key = match &field.binding {
                    ModActorBinding::Inventory { item, .. } => format!("inventory:{item}"),
                    ModActorBinding::Team { .. } => "team".to_string(),
                    ModActorBinding::Score { .. } => "score".to_string(),
                    ModActorBinding::Health { .. } => "health".to_string(),
                    ModActorBinding::Origin => "origin".to_string(),
                    ModActorBinding::Velocity => "velocity".to_string(),
                    ModActorBinding::Angles => "angles".to_string(),
                    ModActorBinding::BoundsMin => "bounds-min".to_string(),
                    ModActorBinding::BoundsMax => "bounds-max".to_string(),
                    _ => String::new(),
                };
                if !canonical.insert(key.clone()) {
                    return Err(GuestError::invalid(format!("Multiple authoritative QVM mod stores for {key}")));
                }
            }
            if let ModActorBinding::Constant { encoding, value } = &field.binding {
                encode_mod_scalar(*value, *encoding)?;
            }
            if let ModActorBinding::ConstantVector(vector) = &field.binding {
                for value in [vector.x as f64, vector.y as f64, vector.z as f64] {
                    encode_mod_scalar(value, ModScalar::Float32)?;
                }
            }
        }
    }
    if declaration.entity_record.as_ref().is_some_and(|id| !records.contains_key(id)) {
        return Err(GuestError::invalid("Unknown QVM engine entity record"));
    }
    if let Some(clients) = declaration.clients.as_ref() {
        let output_field = |field: &ModOutputVector, scalar: Option<&QvmModProtectionScalar>, length: usize| -> Result<(), GuestError> {
            let (record_id, offset) = match scalar {
                Some(word) => (word.record.as_str(), word.offset),
                None => (field.record.as_str(), field.offset),
            };
            let record = records.get(record_id);
            let inside = record.is_some_and(|record| {
                (clients.records.contains(&record.id) || Some(record.id.as_str()) == declaration.entity_record.as_deref())
                    && offset % 4 == 0
                    && record.fields.iter().any(|value| {
                        (matches!(value.binding, ModActorBinding::Private { byte_length } if value.offset <= offset && offset + length <= value.offset + byte_length))
                            || (length == 12
                                && matches!(value.binding, ModActorBinding::BoundsMin | ModActorBinding::BoundsMax)
                                && value.offset == offset)
                    })
            });
            if !inside {
                return Err(GuestError::invalid("QVM client output requires declared private client storage"));
            }
            Ok(())
        };
        validate_mod_client_outputs(
            &clients.outputs,
            &|field| output_field(&ModOutputVector { record: field.record.clone(), offset: field.offset }, Some(field), 4),
            &|field| output_field(field, None, 12),
        )?;
        for output in &clients.outputs {
            if let ModClientOutputDeclaration::BodyShape { min, max } = output {
                for (field, is_min) in [(min, true), (max, false)] {
                    let bound = records.get(&field.record).and_then(|record| {
                        record.fields.iter().find(|value| {
                            value.offset == field.offset
                                && value.access != Some(FieldAccess::ReadOnly)
                                && (is_min && matches!(value.binding, ModActorBinding::BoundsMin)
                                    || !is_min && matches!(value.binding, ModActorBinding::BoundsMax))
                        })
                    });
                    if bound.is_none() {
                        return Err(GuestError::invalid("Client body shape must name its writable source mins/maxs"));
                    }
                }
            }
        }
        let state = records.get(&clients.player_state_record);
        let entity = declaration.entity_record.as_deref().and_then(|id| records.get(id));
        if clients.maximum < 1
            || clients.maximum > 64
            || entity.is_none_or(|entity| entity.capacity < clients.maximum)
            || declaration.entity_record.as_ref().is_some_and(|id| clients.records.contains(id))
            || clients.records.iter().collect::<HashSet<_>>().len() != clients.records.len()
            || clients.records.iter().any(|id| !records.contains_key(id))
            || state.is_none_or(|state| !clients.records.contains(&state.id) || state.stride < qvm_player_state_bytes(declaration.abi_profile))
            || records.values().any(|record| record.capacity < clients.maximum)
        {
            return Err(GuestError::invalid("Invalid QVM source client record reservation"));
        }
    }
    for record in records.values() {
        for field in &record.fields {
            if let ModActorBinding::Record { record: linked } = &field.binding {
                if !records.contains_key(linked) {
                    return Err(GuestError::invalid("Unknown linked QVM actor record"));
                }
            }
        }
    }
    let image = &artifact.image;
    if let Some(lifecycle) = declaration.source_actors.as_ref() {
        qvm_actor_bootstrap(&lifecycle.initial_stores, image, &declaration.actor_records)?;
        if let Some(frame) = lifecycle.frame.as_ref() {
            let record = declaration.entity_record.as_deref().and_then(|id| records.get(id));
            let Some(record) = record else {
                return Err(GuestError::invalid("QVM actor frame requires exclusive original lifecycle ownership"));
            };
            if lifecycle.update.is_some() {
                return Err(GuestError::invalid("QVM actor frame requires exclusive original lifecycle ownership"));
            }
            validate_qvm_mod_actor_frame(frame, image, record, lifecycle.inuse)?;
            check_call(&frame.call, &available_inputs(&[ModCallbackInput::Time, ModCallbackInput::Elapsed]), declaration, &records, image)?;
        }
    }
    if declaration.presentation.as_ref().is_some_and(|presentation| presentation.is_scene()) {
        let record = declaration.entity_record.as_deref().and_then(|id| records.get(id));
        if declaration.clients.is_none() || record.is_none_or(|record| record.stride < qvm_shared_entity_bytes(declaration.abi_profile)) {
            return Err(GuestError::invalid("QVM scene presentation requires admitted source clients and the original shared entity prefix"));
        }
    }
    if let Some(lifecycle) = declaration.source_actors.as_ref() {
        let entity = declaration.entity_record.as_deref().and_then(|id| records.get(id));
        if entity.is_none_or(|entity| entity.stride < qvm_shared_entity_bytes(declaration.abi_profile))
            || lifecycle.inuse < qvm_shared_entity_bytes(declaration.abi_profile)
            || lifecycle.inuse % 4 != 0
            || entity.is_some_and(|entity| lifecycle.inuse + 4 > entity.stride)
            || lifecycle.release.argument >= QVM_MAX_PRIVATE_ARGUMENT_WORDS
            || lifecycle.allocate == lifecycle.release.entry
            || image.instruction(lifecycle.allocate).is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
            || image.instruction(lifecycle.release.entry).is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
        {
            return Err(GuestError::invalid("Invalid QVM source actor lifecycle"));
        }
        if let Some(update) = lifecycle.update.as_ref() {
            check_call(update, &available_inputs(&[ModCallbackInput::Own, ModCallbackInput::Time, ModCallbackInput::Elapsed]), declaration, &records, image)?;
        }
    }
    for objective in &declaration.objectives {
        validate_qvm_objective_address(end, &objective.storage.address)?;
        for reference in [&objective.carrier, &objective.target].into_iter().flatten() {
            validate_qvm_objective_address(end, reference)?;
        }
        if let ObjectiveRole::Owned { change: Some(change), .. } = &objective.role {
            check_call(
                change,
                &available_inputs(&[ModCallbackInput::Own, ModCallbackInput::Other, ModCallbackInput::Activator, ModCallbackInput::Amount, ModCallbackInput::Time]),
                declaration,
                &records,
                image,
            )?;
        }
    }
    if let Some(items) = declaration.items.as_ref() {
        for item in &items.definitions {
            for call in item.action_calls() {
                check_call(call, &available_inputs(&[ModCallbackInput::Own, ModCallbackInput::Time]), declaration, &records, image)?;
            }
        }
    }
    for call in &declaration.initialize {
        check_call(call, &available_inputs(&[ModCallbackInput::Time]), declaration, &records, image)?;
    }
    if let Some(clients) = declaration.clients.as_ref() {
        for call in clients.admit.iter().chain(clients.userinfo.iter()).chain(clients.disconnect.iter()) {
            check_call(call, &available_inputs(&[ModCallbackInput::Own, ModCallbackInput::Time]), declaration, &records, image)?;
        }
        for call in &clients.frame {
            check_call(call, &available_inputs(&[ModCallbackInput::Own, ModCallbackInput::Time, ModCallbackInput::Elapsed]), declaration, &records, image)?;
        }
        for binding in &clients.input {
            for call in &binding.calls {
                check_call(
                    call,
                    &available_inputs(&[
                        ModCallbackInput::Own,
                        ModCallbackInput::Time,
                        ModCallbackInput::Elapsed,
                        ModCallbackInput::ViewAngles,
                        ModCallbackInput::Attack,
                        ModCallbackInput::Jump,
                        ModCallbackInput::Impulse,
                        ModCallbackInput::ForwardMove,
                        ModCallbackInput::SideMove,
                        ModCallbackInput::UpMove,
                    ]),
                    declaration,
                    &records,
                    image,
                )?;
            }
            if let InputPhase::Before { outputs } = &binding.phase {
                for output in outputs {
                    match output {
                        QvmModInputOutput::Field { .. } => {}
                        QvmModInputOutput::Handler { entry, actor_pointer, returns, .. } | QvmModInputOutput::Command { entry, actor_pointer, .. } => {
                            if image.instruction(*entry).is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter) {
                                return Err(GuestError::invalid("QVM input handler requires an original function entry"));
                            }
                            let pointers: Vec<&QvmModInputPointer> = match output {
                                QvmModInputOutput::Command { command, .. } => vec![actor_pointer, command],
                                _ => vec![actor_pointer],
                            };
                            for pointer in pointers {
                                if let InputPointerKind::Global { address } = pointer.kind {
                                    check_data_range(end, address, 4)?;
                                }
                            }
                            if let QvmModInputOutput::Handler { returns: Some(forced), .. } = output {
                                encode_mod_scalar(forced.value, forced.encoding)?;
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(combat) = declaration.combat.as_ref() {
        check_call(
            &QvmModSourceCall { entry: combat.entry, arguments: Vec::new(), globals: combat.globals.clone(), returns: ModReturns::Void },
            &available_inputs(&[ModCallbackInput::Time]),
            declaration,
            &records,
            image,
        )?;
    }
    validate_qvm_mod_actors_mirror(artifact, declaration)?;
    super::mod_protection::validate_qvm_mod_protection(declaration)?;
    if let Some(items) = declaration.items.as_ref() {
        validate_qvm_mod_items_mirror(items, declaration, image)?;
    }
    for definition in &declaration.protection {
        check_call(
            definition.absorb_call(),
            &available_inputs(&[
                ModCallbackInput::Own,
                ModCallbackInput::Attacker,
                ModCallbackInput::Inflictor,
                ModCallbackInput::Amount,
                ModCallbackInput::Knockback,
                ModCallbackInput::DamageFlags,
                ModCallbackInput::RegularProtectionScale,
                ModCallbackInput::Point,
                ModCallbackInput::Direction,
                ModCallbackInput::Normal,
                ModCallbackInput::Time,
            ]),
            declaration,
            &records,
            image,
        )?;
    }
    validate_qvm_mod_pickups_mirror(declaration)?;
    for rule in &declaration.pickups {
        let available = available_inputs(&[
            ModCallbackInput::Own,
            ModCallbackInput::Other,
            ModCallbackInput::Item,
            ModCallbackInput::Time,
            ModCallbackInput::PickupCount,
            ModCallbackInput::PickupHasCount,
            ModCallbackInput::PickupDropped,
        ]);
        for field in &rule.context {
            check_value(&field.value, &available, declaration, &records, end)?;
        }
        if let OriginalPickupOperation::GateThenGrant { gate, .. } = &rule.operation {
            check_call(gate, &available, declaration, &records, image)?;
        }
        let grant = match &rule.operation {
            OriginalPickupOperation::BooleanGrant { grant } | OriginalPickupOperation::GateThenGrant { grant, .. } => grant,
        };
        check_call(grant, &available, declaration, &records, image)?;
    }
    let mut ids = HashSet::new();
    for call in &declaration.callbacks {
        if !ids.insert(call.binding.id.clone()) || (call.binding.stage != CallbackStage::Observe && call.call.returns == ModReturns::Void) {
            return Err(GuestError::invalid("Duplicate QVM callback or missing source return value"));
        }
        let mut available = available_inputs(&[ModCallbackInput::Own, ModCallbackInput::Time]);
        if call.binding.stage == CallbackStage::Observe {
            available.insert(ModCallbackInput::Result);
        }
        let extra: &[ModCallbackInput] = match call.binding.operation {
            ModCallbackOperation::Damage => &[ModCallbackInput::Attacker, ModCallbackInput::Inflictor, ModCallbackInput::Amount, ModCallbackInput::Knockback, ModCallbackInput::Direction, ModCallbackInput::Point, ModCallbackInput::Normal],
            ModCallbackOperation::InventoryGive | ModCallbackOperation::InventoryConsume => &[ModCallbackInput::Item, ModCallbackInput::Amount],
            ModCallbackOperation::ActorUse => &[ModCallbackInput::Other, ModCallbackInput::Activator],
            ModCallbackOperation::ActorTouch => &[ModCallbackInput::Other],
            ModCallbackOperation::ActorThink => &[ModCallbackInput::Elapsed],
            ModCallbackOperation::ActorPain => &[ModCallbackInput::Attacker, ModCallbackInput::Amount, ModCallbackInput::Knockback],
            ModCallbackOperation::ActorDie => &[ModCallbackInput::Attacker, ModCallbackInput::Inflictor, ModCallbackInput::Amount, ModCallbackInput::Knockback, ModCallbackInput::Point],
        };
        for name in extra {
            available.insert(*name);
        }
        check_call(&call.call, &available, declaration, &records, image)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Checkpoint validation.
// ---------------------------------------------------------------------------

/// Checkpoint view for mod checkpoint validation (mirror of `QvmCheckpoint`).
#[derive(Debug, Clone)]
pub struct QvmModCheckpointView<'a> {
    /// Module identity.
    pub module: &'a ModuleId,
    /// Data image bytes.
    pub data: &'a [u8],
    /// API kind.
    pub api_kind: &'a str,
    /// API version.
    pub api_version: u32,
    /// ABI profile.
    pub abi: QvmAbi,
    /// Saved instruction index.
    pub instruction_index: usize,
    /// Saved operand stack length.
    pub operand_stack_len: usize,
    /// Saved program stack pointer.
    pub program_stack: usize,
    /// Host module identity.
    pub host_module: &'a ModuleId,
    /// Host state format.
    pub host_format: &'a str,
    /// Decoded host state.
    pub host: &'a ProfileValue,
    /// Saved random states.
    pub random_len: usize,
    /// Saved callback bindings.
    pub callbacks_len: usize,
}

/// Saved actor projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedProjection {
    /// Actor.
    pub actor: SavedActorId,
    /// Slot.
    pub slot: usize,
    /// Owned flag.
    pub owned: bool,
}

/// Saved client slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedClientSlot {
    /// Actor.
    pub actor: SavedActorId,
    /// Slot.
    pub slot: usize,
    /// Admitted flag.
    pub admitted: bool,
}

/// Validated host image.
#[derive(Debug, Clone)]
pub struct ModHostImage {
    /// Projections.
    pub projections: Vec<SavedProjection>,
    /// Client slots.
    pub client_slots: Vec<SavedClientSlot>,
    /// Allocation cursor.
    pub next_slot: usize,
    /// Actor templates.
    pub defaults: Vec<(String, Vec<u8>)>,
    /// Configstrings.
    pub configstrings: Vec<(u32, String)>,
}

fn read_saved_actor(reader: &ProfileReader<'_>) -> Result<SavedActorId, GuestError> {
    Ok(SavedActorId { slot: reader.field("slot")?.integer(0)? as u32, generation: reader.field("generation")?.integer(0)? as u32 })
}

/// Validate the host half of a mod checkpoint (mirror of `hostImage`).
pub fn read_mod_host_image(host: &ProfileValue, declaration: &QvmModCallbackDeclaration, data: &[u8]) -> Result<ModHostImage, GuestError> {
    let reader = ProfileReader::new(host);
    reader.field("version")?.literal_int(1)?;
    if let Some(spawn) = declaration.spawn_entities.as_deref() {
        let tokens = reader.field("entityTokens")?;
        if !tokens.is_undefined() {
            tokens.field("source")?.literal_str(spawn)?;
            tokens.field("cursor")?.nullable(|cursor| cursor.integer(0))?;
        }
    }
    let common: Vec<&QvmModActorRecord> =
        declaration.actor_records.iter().filter(|record| !is_client_record(declaration, record)).collect();
    let capacity = common.iter().map(|record| record.capacity).min().unwrap_or(0);
    let next_slot = reader.field("nextSlot")?.integer(0)? as usize;
    if next_slot > capacity {
        return Err(GuestError::invalid("Invalid QVM projection allocation cursor"));
    }
    let entity_capacity = declaration.entity_record.as_deref().and_then(|id| declaration.actor_records.iter().find(|record| record.id == *id)).map_or(0, |record| record.capacity);
    let mut slots = HashSet::new();
    let mut actors = HashSet::new();
    let projections = reader.field("projections")?.list(|entry| {
        let actor = read_saved_actor(&entry.field("actor")?)?;
        let slot = entry.field("slot")?.integer(0)? as usize;
        let owned = entry.field("owned")?.boolean()?;
        let key = (actor.slot, actor.generation);
        let bad = if owned {
            declaration.source_actors.is_none() || slot >= entity_capacity
        } else {
            slot >= next_slot
        };
        if bad || !slots.insert(slot) || !actors.insert(key) {
            return entry.fail("Invalid saved QVM actor projection");
        }
        entry.field("event")?.nullable(|event| event.string())?;
        Ok(SavedProjection { actor, slot, owned })
    })?;
    let client_slots = if reader.field("clientSlots")?.is_undefined() {
        Vec::new()
    } else {
        reader.field("clientSlots")?.list(|entry| {
            Ok(SavedClientSlot {
                actor: read_saved_actor(&entry.field("actor")?)?,
                slot: entry.field("slot")?.integer(0)? as usize,
                admitted: entry.field("admitted")?.boolean()?,
            })
        })?
    };
    let reserved = declaration.clients.as_ref().map_or(0, |clients| clients.maximum);
    let mut occupied = HashSet::new();
    for entry in &client_slots {
        let matches = projections.iter().any(|projection| {
            !projection.owned && projection.slot == entry.slot && projection.actor == entry.actor
        });
        if entry.slot >= reserved || !occupied.insert(entry.slot) || !matches {
            return Err(GuestError::invalid("Invalid QVM saved source client mapping"));
        }
    }
    if projections.iter().any(|entry| entry.slot < reserved && !occupied.contains(&entry.slot)) {
        return Err(GuestError::invalid("QVM saved actor occupies a reserved client row"));
    }
    let events = reader.field("playerEvents")?;
    if !events.is_undefined() && !matches!(events.value(), ProfileValue::Null) {
        let cursors = super::mod_player_events::read_qvm_player_events(&events)?;
        if let Some(cursors) = cursors {
            let record = declaration
                .clients
                .as_ref()
                .and_then(|clients| declaration.actor_records.iter().find(|record| record.id == clients.player_state_record));
            if cursors.clients.len() != client_slots.len() {
                return Err(GuestError::invalid("Saved QVM player event cursors differ from source clients"));
            }
            for cursor in &cursors.clients {
                let client = client_slots.iter().find(|entry| entry.actor == cursor.actor);
                let Some(client) = client else {
                    return Err(GuestError::invalid("Saved QVM player event cursor differs from its source client"));
                };
                let Some(record) = record else {
                    return Err(GuestError::invalid("Saved QVM player event cursor differs from its source client"));
                };
                let at = record.address + client.slot * record.stride + 108;
                let bytes = data.get(at..at + 4).ok_or_else(|| GuestError::invalid("Saved QVM player event cursor differs from its source client"))?;
                if i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) != cursor.observed_sequence {
                    return Err(GuestError::invalid("Saved QVM player event cursor differs from its source client"));
                }
            }
        }
    }
    let defaults = reader.field("defaults")?.list(|entry| {
        let id = entry.field("id")?.string()?;
        let bytes = entry.field("bytes")?.bytes()?;
        let record = declaration.actor_records.iter().find(|record| record.id == id);
        if record.is_none_or(|record| bytes.len() != record.stride * record.capacity) {
            return entry.fail("QVM actor template differs from its source layout");
        }
        Ok((id, bytes))
    })?;
    if defaults.len() != declaration.actor_records.len() {
        return Err(GuestError::invalid("Missing QVM actor templates"));
    }
    let configstrings = reader.field("configstrings")?.list(|entry| {
        Ok((entry.field("index")?.integer(0)? as u32, entry.field("value")?.string()?))
    })?;
    if configstrings.iter().map(|(index, _)| index).collect::<HashSet<_>>().len() != configstrings.len()
        || configstrings.iter().any(|(index, _)| *index >= 1024)
    {
        return Err(GuestError::invalid("Invalid QVM configstrings"));
    }
    Ok(ModHostImage { projections, client_slots, next_slot, defaults, configstrings })
}

/// Validate a full mod checkpoint (mirror of `validateQvmModCheckpoint`).
pub fn validate_qvm_mod_checkpoint(
    artifact: &QvmArtifact,
    declaration: &QvmModCallbackDeclaration,
    checkpoint: &QvmModCheckpointView<'_>,
) -> Result<ModHostImage, GuestError> {
    let expected_api = qvm_api(QvmRole::Qagame, declaration.abi_profile);
    if !checkpoint.module.same_module(&artifact.module)
        || checkpoint.data.len() != artifact.image.allocated_data_length
        || checkpoint.api_kind != expected_api.0
        || checkpoint.api_version != expected_api.1
        || checkpoint.abi != declaration.abi_profile
        || checkpoint.instruction_index != 0
        || checkpoint.operand_stack_len != 0
        || checkpoint.program_stack != checkpoint.data.len()
        || !checkpoint.host_module.same_module(&artifact.module)
        || checkpoint.host_format != "qvm:mod-host-v1"
        || checkpoint.random_len != 0
        || checkpoint.callbacks_len != 0
    {
        return Err(GuestError::invalid("Incompatible QVM gameplay mod checkpoint"));
    }
    read_mod_host_image(checkpoint.host, declaration, checkpoint.data)
}

// ---------------------------------------------------------------------------
// Provider runtime core.
// ---------------------------------------------------------------------------

/// Canonical field value exchanged with host services.
#[derive(Debug, Clone, PartialEq)]
pub enum CanonicalField {
    /// Scalar word.
    Word(f64),
    /// Vector.
    Vec(Vec3),
}

/// Owned entity view for event publication.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityPublishView {
    /// Actor.
    pub actor: ActorId,
    /// Slot.
    pub slot: usize,
    /// Linked flag.
    pub linked: bool,
    /// Current event.
    pub event: i32,
    /// Entity type.
    pub etype: i32,
    /// Current origin.
    pub origin: Vec3,
    /// Entity-state bytes.
    pub state: Vec<u8>,
    /// Model index.
    pub model_index: i32,
    /// Inline model flag.
    pub inline_model: bool,
    /// Frame.
    pub frame: i32,
    /// Current angles.
    pub angles: Vec3,
    /// Server flags.
    pub sv_flags: i32,
}

/// Entity link view for scene publication.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityLinkView {
    /// Linked flag.
    pub linked: bool,
    /// Server flags.
    pub sv_flags: i32,
    /// Single client.
    pub single_client: i32,
    /// Absolute minimums.
    pub abs_min: Vec3,
    /// Absolute maximums.
    pub abs_max: Vec3,
}

/// Presentation record published for one owned entity.
#[derive(Debug, Clone, PartialEq)]
pub struct ModPresentation {
    /// Actor.
    pub actor: ActorId,
    /// Model path.
    pub path: String,
    /// Frame.
    pub frame: i32,
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Source-client render ownership (scene runtime only).
    pub render_owner_source_client: bool,
}

/// Event emitted by the provider.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderEmit {
    /// Server command.
    ServerCommand {
        /// Client number.
        client: i32,
        /// Text.
        text: String,
    },
    /// Entity event.
    EntityEvent {
        /// Actor.
        actor: ActorId,
        /// Entity-state bytes.
        state: Vec<u8>,
        /// Origin.
        origin: Vec3,
        /// Time in milliseconds.
        time_ms: i32,
    },
}

/// Presentation context resolved for one viewer.
#[derive(Debug, Clone)]
pub struct ProviderViewerContext {
    /// Source client slot.
    pub client_number: usize,
    /// Game-state revision.
    pub game_state_revision: u64,
    /// Server time in milliseconds.
    pub server_time: i32,
    /// Viewer player state.
    pub player_state: super::mod_presentation_checkpoint::SourcePlayerState,
}

/// Pickup offer context (mirror of `OriginalPickupOffer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupOffer {
    /// Recipient actor.
    pub recipient: ActorId,
    /// Pickup actor.
    pub pickup: ActorId,
    /// Item identity.
    pub item: String,
}

/// Host services integrating the provider (interpreter, memory, world).
pub trait ModProviderHost {
    /// Assert the provider runs on its owner thread and is open.
    fn current(&self) -> Result<(), GuestError>;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Whether an actor resolves to an owned actor.
    fn is_owned(&self, actor: &ActorId) -> bool;
    /// Caller time in seconds.
    fn time_seconds(&self) -> f64;
    /// Call a source function.
    fn call_module(&mut self, words: &[i32], entry: usize) -> Result<i32, GuestError>;
    /// Run a source command entry.
    fn command_module(&mut self, words: &[i32], argv: &[String]) -> Result<i32, GuestError>;
    /// Read one source word.
    fn read_i32(&self, address: usize) -> Result<i32, GuestError>;
    /// Write one source word.
    fn write_i32(&mut self, address: usize, value: i32) -> Result<(), GuestError>;
    /// Read source bytes.
    fn read_bytes(&self, address: usize, len: usize) -> Result<Vec<u8>, GuestError>;
    /// Write source bytes.
    fn write_bytes(&mut self, address: usize, bytes: &[u8]) -> Result<(), GuestError>;
    /// Copy source bytes.
    fn copy_bytes(&mut self, dest: usize, src: usize, len: usize) -> Result<(), GuestError>;
    /// Current interpreter stack pointer.
    fn stack_pointer(&self) -> usize;
    /// Read a canonical field value.
    fn canonical_field(&self, actor: &ActorId, field: &QvmModActorField) -> Result<CanonicalField, GuestError>;
    /// Commit a source write to canonical state.
    fn commit_field(&mut self, actor: &ActorId, field: &QvmModActorField, value: CanonicalField) -> Result<(), GuestError>;
    /// Whether a field is a published body-shape output.
    fn is_body_output(&self, actor: &ActorId, record: &str, offset: usize) -> bool {
        let _ = (actor, record, offset);
        false
    }
    /// Whether an actor has a client binding.
    fn has_client(&self, actor: &ActorId) -> bool;
    /// Client slot of an actor, if bound.
    fn client_slot(&self, actor: &ActorId) -> Option<usize>;
    /// Whether an actor is an admitted live client.
    fn admitted_client(&self, actor: &ActorId) -> bool;
    /// Bound players as `(actor, slot, admitted)`.
    fn players(&self) -> Vec<(ActorId, usize, bool)>;
    /// Start client bindings after initialization or restore.
    fn start_clients(&mut self) -> Result<(), GuestError>;
    /// Actors receiving client frame calls.
    fn frame_actors(&self) -> Vec<ActorId>;
    /// Release an actor from sub-components.
    fn release_actor_components(&mut self, actor: &ActorId);
    /// Reserve protection channels.
    fn reserve_protection(&mut self) -> Result<(), GuestError>;
    /// Activate protection, pickups, and match bindings.
    fn activate_protection(&mut self) -> Result<(), GuestError>;
    /// Assert protection and pickup runtimes are idle.
    fn assert_subcomponents_idle(&self) -> Result<(), GuestError>;
    /// Close sub-component runtimes, collecting failures.
    fn close_subcomponents(&mut self) -> Vec<GuestError>;
    /// Publish pending player events.
    fn publish_player_events(&mut self);
    /// Discard pending player events.
    fn discard_player_events(&mut self);
    /// Guard one pickup write against the current resource binding.
    fn check_pickup_write(&self, actor: &ActorId, field: &QvmModActorField) -> Result<(), GuestError>;
    /// Adopt an allocated source slot.
    fn adopt_source(&mut self, slot: usize) -> Result<ActorId, GuestError>;
    /// Retire a released source slot.
    fn retire_source(&mut self, slot: usize) -> Result<(), GuestError>;
    /// Run pre-release actor semantics.
    fn before_release(&mut self, actor: &ActorId) -> Result<(), GuestError>;
    /// Release an owned actor.
    fn release_owned(&mut self, actor: &ActorId) -> Result<(), GuestError>;
    /// Begin an original actor-frame run.
    fn begin_actor_frame(&mut self) -> Result<(), GuestError>;
    /// End an original actor-frame run, reporting completion.
    fn end_actor_frame(&mut self) -> Result<bool, GuestError>;
    /// Server info and system info strings.
    fn server_info(&self) -> (String, String);
    /// Build a game-state record from configstrings.
    fn build_game_state(&self, entries: &[(u32, String)]) -> Result<super::mod_presentation_checkpoint::SourceGameState, GuestError>;
    /// Owned entity views for publication.
    fn owned_entity_views(&self) -> Vec<EntityPublishView>;
    /// Player-state bytes of an actor.
    fn player_state_bytes(&self, actor: &ActorId) -> Result<Vec<u8>, GuestError>;
    /// Entity-link view of a slot.
    fn entity_link(&self, slot: usize) -> Result<EntityLinkView, GuestError>;
    /// Emit a provider event.
    fn emit(&mut self, event: ProviderEmit) -> Result<(), GuestError>;
    /// Read a script resource.
    fn read_script(&self, name: &str) -> Option<String>;
    /// Whether commands are bound.
    fn has_commands(&self) -> bool {
        false
    }
}

/// Maximum nested source calls.
pub const MOD_MAX_CALL_DEPTH: usize = 64;

/// Source callbacks over declared actor records.
pub struct QvmModProvider<H: ModProviderHost> {
    host: H,
    artifact: QvmArtifact,
    declaration: QvmModCallbackDeclaration,
    records: HashMap<String, QvmModActorRecord>,
    projections: HashMap<ActorId, usize>,
    retired: HashSet<ActorId>,
    owned: HashSet<ActorId>,
    next_slot: usize,
    scratch_start: usize,
    scratch: usize,
    configstrings: BTreeMap<u32, String>,
    presentation_revision: u64,
    presentation_state: Option<super::mod_presentation_checkpoint::SourceGameState>,
    scene_revision: u64,
    scene_dirty: bool,
    scene_command_sequence: u64,
    scene_commands: VecDeque<(u64, Option<ActorId>, String)>,
    scene_state: Option<super::mod_presentation_checkpoint::ModScenePublication>,
    scene_baseline: Option<super::mod_presentation_checkpoint::ModScenePublication>,
    event_keys: HashMap<ActorId, String>,
    defaults: HashMap<String, Vec<u8>>,
    frames: Vec<ModCallFrame>,
    pickup_depth: usize,
    weapon_inputs: Vec<(ActorId, Option<usize>)>,
    pickup_scopes: Vec<PickupOffer>,
    projection_writes: usize,
    commands_bound: bool,
    closed: bool,
}

struct ModCallFrame {
    observations: Vec<FieldObservation>,
    pending: Vec<PendingCommit>,
    cursor: usize,
}

#[derive(Debug, Clone)]
struct FieldObservation {
    actor: ActorId,
    address: usize,
    field: QvmModActorField,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct PendingCommit {
    actor: ActorId,
    address: usize,
    field: QvmModActorField,
    value: CanonicalField,
}

/// Lowered call ready for execution.
pub struct LoweredCall {
    /// Argument words.
    pub words: Vec<i32>,
    saved_globals: Vec<(usize, Vec<u8>)>,
    saved_scratch: usize,
}

impl<H: ModProviderHost> QvmModProvider<H> {
    /// Open a provider over a validated declaration.
    pub fn open(artifact: QvmArtifact, declaration: QvmModCallbackDeclaration, host: H) -> Result<Self, GuestError> {
        validate_qvm_mod(&artifact, &declaration)?;
        let scratch_start = artifact.image.data_end().div_ceil(16) * 16;
        let records = declaration.actor_records.iter().map(|record| (record.id.clone(), record.clone())).collect();
        Ok(Self {
            host,
            artifact,
            declaration,
            records,
            projections: HashMap::new(),
            retired: HashSet::new(),
            owned: HashSet::new(),
            next_slot: 0,
            scratch_start,
            scratch: scratch_start,
            configstrings: BTreeMap::new(),
            presentation_revision: 0,
            presentation_state: None,
            scene_revision: 0,
            scene_dirty: true,
            scene_command_sequence: 0,
            scene_commands: VecDeque::new(),
            scene_state: None,
            scene_baseline: None,
            event_keys: HashMap::new(),
            defaults: HashMap::new(),
            frames: Vec::new(),
            pickup_depth: 0,
            weapon_inputs: Vec::new(),
            pickup_scopes: Vec::new(),
            projection_writes: 0,
            commands_bound: false,
            closed: false,
        })
    }

    /// Borrow the host.
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Mutably borrow the host.
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    /// Borrow the declaration.
    pub fn declaration(&self) -> &QvmModCallbackDeclaration {
        &self.declaration
    }

    fn current(&self) -> Result<(), GuestError> {
        self.host.current()?;
        if self.closed {
            return Err(GuestError::invalid("QVM gameplay mod is closed"));
        }
        Ok(())
    }

    /// Projection slot of an actor, if projected.
    #[must_use]
    pub fn projection_slot(&self, actor: &ActorId) -> Option<usize> {
        self.projections.get(actor).copied()
    }

    /// Actor projected at a slot, if any.
    #[must_use]
    pub fn actor_at(&self, slot: usize) -> Option<ActorId> {
        self.projections.iter().find(|(_, index)| **index == slot).map(|(actor, _)| actor.clone())
    }

    /// Whether an actor is owned by this source.
    #[must_use]
    pub fn is_source_owned(&self, actor: &ActorId) -> bool {
        self.owned.contains(actor)
    }

    fn actor_records(&self, actor: &ActorId) -> Vec<QvmModActorRecord> {
        self.records.values().filter(|record| !is_client_record(&self.declaration, record) || self.host.has_client(actor)).cloned().collect()
    }

    /// Resolve a record pointer for an actor, projecting on first use.
    pub fn pointer(&mut self, actor: Option<&ActorId>, record_id: &str) -> Result<usize, GuestError> {
        let Some(actor) = actor else { return Ok(0) };
        if !self.host.is_live(actor) {
            return Err(GuestError::invalid("QVM mod cannot project a stale actor"));
        }
        let record = self.records.get(record_id).ok_or_else(|| GuestError::invalid("Missing validated QVM actor record"))?.clone();
        if let Some(slot) = self.projections.get(actor).copied() {
            if slot >= record.capacity || (is_client_record(&self.declaration, &record) && !self.host.has_client(actor)) {
                return Err(GuestError::invalid("Source actor has no declared auxiliary record"));
            }
            return Ok(record.address + slot * record.stride);
        }
        let client = self.host.client_slot(actor);
        let mut next = client.unwrap_or_else(|| self.declaration.clients.as_ref().map_or(0, |clients| clients.maximum));
        let used: HashSet<usize> = self.projections.values().copied().collect();
        if client.is_none() {
            while used.contains(&next) {
                next += 1;
            }
        } else if used.contains(&next) {
            return Err(GuestError::invalid("QVM source client row is already occupied"));
        }
        if self.actor_records(actor).iter().any(|record| next >= record.capacity) {
            return Err(GuestError::invalid("QVM mod actor projection capacity exceeded"));
        }
        let slot = next;
        self.next_slot = self.next_slot.max(slot + 1);
        self.projections.insert(actor.clone(), slot);
        for record in self.actor_records(actor) {
            let address = record.address + slot * record.stride;
            let defaults = self.defaults.get(&record.id).ok_or_else(|| GuestError::invalid("Missing source actor defaults"))?.clone();
            let start = slot * record.stride;
            self.host.write_bytes(address, &defaults[start..start + record.stride])?;
            for field in &record.fields {
                match &field.binding {
                    ModActorBinding::Constant { encoding, value } => {
                        self.host.write_i32(address + field.offset, encode_mod_scalar(*value, *encoding)?)?;
                    }
                    ModActorBinding::ConstantVector(vector) => self.write_vector(address + field.offset, *vector)?,
                    ModActorBinding::Record { record: linked } => {
                        let linked_ptr = if self.declaration.clients.as_ref().is_some_and(|clients| clients.records.contains(linked)) && !self.host.has_client(actor) {
                            0
                        } else {
                            self.pointer(Some(actor), linked)? as i32
                        };
                        self.host.write_i32(address + field.offset, linked_ptr)?;
                    }
                    _ => {}
                }
            }
        }
        if let Some(lifecycle) = self.declaration.source_actors.clone() {
            let entity = self.entity_address(slot)?;
            self.host.write_i32(entity + lifecycle.inuse, 1)?;
            self.host.write_i32(entity, slot as i32)?;
        }
        Ok(record.address + slot * record.stride)
    }

    fn write_vector(&mut self, address: usize, value: Vec3) -> Result<(), GuestError> {
        for (index, component) in [value.x as f64, value.y as f64, value.z as f64].into_iter().enumerate() {
            self.host.write_i32(address + index * 4, encode_mod_scalar(component, ModScalar::Float32)?)?;
        }
        Ok(())
    }

    fn read_vector(&self, address: usize) -> Result<Vec3, GuestError> {
        let bytes = self.host.read_bytes(address, 12)?;
        Ok(vec3(
            f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        ))
    }

    /// Player-state address of an actor.
    pub fn player_address(&mut self, actor: &ActorId) -> Result<usize, GuestError> {
        let record = self.declaration.clients.as_ref().map(|clients| clients.player_state_record.clone());
        let Some(record) = record else {
            return Err(GuestError::invalid("Missing QVM client player state"));
        };
        self.pointer(Some(actor), &record)
    }

    /// Release one actor projection, restoring source defaults.
    pub fn release_projection(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        self.host.release_actor_components(actor);
        let Some(slot) = self.projections.get(actor).copied() else { return Ok(()) };
        if !self.frames.is_empty() || self.pickup_depth != 0 {
            self.retired.insert(actor.clone());
            return Ok(());
        }
        for record in self.records.values() {
            if slot < record.capacity {
                let defaults = self.defaults.get(&record.id).ok_or_else(|| GuestError::invalid("Missing QVM source client defaults"))?.clone();
                let start = slot * record.stride;
                self.host.write_bytes(record.address + slot * record.stride, &defaults[start..start + record.stride])?;
            }
        }
        self.projections.remove(actor);
        self.event_keys.remove(actor);
        Ok(())
    }

    /// Finish projections retired during source calls.
    pub fn finish_retired_projections(&mut self) -> Result<(), GuestError> {
        if !self.frames.is_empty() || self.pickup_depth != 0 {
            return Ok(());
        }
        for actor in std::mem::take(&mut self.retired) {
            self.release_projection(&actor)?;
        }
        Ok(())
    }

    /// Allocate scratch words above source data.
    pub fn allocate_scratch(&mut self, size: usize) -> Result<usize, GuestError> {
        let address = self.scratch;
        self.scratch += size.div_ceil(4) * 4;
        if self.scratch > self.host.stack_pointer().saturating_sub(65536) {
            return Err(GuestError::invalid("QVM mod argument scratch exceeds its reserved space"));
        }
        Ok(address)
    }

    /// Lower one value to a source word.
    pub fn lower(&mut self, value: &QvmModValue, inputs: &BTreeMap<ModCallbackInput, ModRuntimeValue>) -> Result<i32, GuestError> {
        match value {
            QvmModValue::Address(word) => Ok(*word),
            QvmModValue::Actor { record, input } => {
                let Some(ModRuntimeValue::Actor(Some(actor))) = inputs.get(&input.callback()).cloned() else {
                    return Err(GuestError::invalid("Missing QVM actor input"));
                };
                Ok(self.pointer(Some(&actor), record)? as i32)
            }
            QvmModValue::Client { input } => {
                let Some(ModRuntimeValue::Actor(Some(actor))) = inputs.get(&input.callback()).cloned() else {
                    return Err(GuestError::invalid("QVM source call requires an admitted destination client"));
                };
                let Some(slot) = self.host.client_slot(&actor) else {
                    return Err(GuestError::invalid("QVM source call requires an admitted destination client"));
                };
                let Some(entity) = self.declaration.entity_record.clone() else {
                    return Err(GuestError::invalid("QVM source call requires an admitted destination client"));
                };
                self.pointer(Some(&actor), &entity)?;
                Ok(slot as i32)
            }
            QvmModValue::Time { input, units, encoding } => {
                let Some(ModRuntimeValue::Float(value)) = inputs.get(&input.callback()).copied() else {
                    return Err(GuestError::invalid("Missing QVM time input"));
                };
                encode_mod_scalar(value * if *units == TimeUnits::Milliseconds { 1000.0 } else { 1.0 }, *encoding)
            }
            QvmModValue::Int32(inner) | QvmModValue::Float32(inner) | QvmModValue::Vector(inner) | QvmModValue::Str(inner) => {
                let resolved = match inner {
                    ModCallbackValue::Input(name) => inputs.get(name).cloned(),
                    ModCallbackValue::Float(value) => Some(ModRuntimeValue::Float(*value)),
                    ModCallbackValue::Str(value) => Some(ModRuntimeValue::Str(value.clone())),
                    ModCallbackValue::Vec(value) => Some(ModRuntimeValue::Vec(*value)),
                };
                match value {
                    QvmModValue::Int32(_) => {
                        let Some(ModRuntimeValue::Float(number)) = resolved else {
                            return Err(GuestError::invalid("Missing scalar QVM input"));
                        };
                        encode_mod_scalar(number, ModScalar::Int32)
                    }
                    QvmModValue::Float32(_) => {
                        let Some(ModRuntimeValue::Float(number)) = resolved else {
                            return Err(GuestError::invalid("Missing scalar QVM input"));
                        };
                        encode_mod_scalar(number, ModScalar::Float32)
                    }
                    QvmModValue::Vector(_) => {
                        let Some(ModRuntimeValue::Vec(vector)) = resolved else {
                            return Err(GuestError::invalid("Missing vector QVM input"));
                        };
                        let address = self.allocate_scratch(12)?;
                        self.write_vector(address, vector)?;
                        Ok(address as i32)
                    }
                    _ => {
                        let Some(ModRuntimeValue::Str(text)) = resolved else {
                            return Err(GuestError::invalid("Invalid QVM string input"));
                        };
                        if text.contains('\0') {
                            return Err(GuestError::invalid("Invalid QVM string input"));
                        }
                        let address = self.allocate_scratch(text.len() + 1)?;
                        let mut bytes = text.into_bytes();
                        bytes.push(0);
                        self.host.write_bytes(address, &bytes)?;
                        Ok(address as i32)
                    }
                }
            }
        }
    }

    /// Project canonical state into source words before a call.
    pub fn refresh(&mut self) -> Result<(), GuestError> {
        self.projection_writes += 1;
        let result = self.refresh_inner();
        self.projection_writes -= 1;
        result
    }

    fn refresh_inner(&mut self) -> Result<(), GuestError> {
        let projections: Vec<(ActorId, usize)> = self.projections.iter().map(|(actor, slot)| (actor.clone(), *slot)).collect();
        for (actor, slot) in &projections {
            if self.owned.contains(actor) || self.retired.contains(actor) {
                continue;
            }
            for record in self.actor_records(actor) {
                for field in &record.fields {
                    if !mod_field_is_shared(field) || self.host.is_body_output(actor, &record.id, field.offset) {
                        continue;
                    }
                    let address = record.address + slot * record.stride + field.offset;
                    match self.host.canonical_field(actor, field)? {
                        CanonicalField::Word(number) => {
                            let encoding = match &field.binding {
                                ModActorBinding::Team { encoding, .. }
                                | ModActorBinding::Score { encoding }
                                | ModActorBinding::Health { encoding }
                                | ModActorBinding::Inventory { encoding, .. } => *encoding,
                                _ => ModScalar::Int32,
                            };
                            self.host.write_i32(address, encode_mod_scalar(number, encoding)?)?;
                        }
                        CanonicalField::Vec(vector) => self.write_vector(address, vector)?,
                    }
                }
            }
        }
        let observations = self.observe()?;
        for frame in &mut self.frames {
            frame.observations = observations.clone();
        }
        Ok(())
    }

    fn observe(&self) -> Result<Vec<FieldObservation>, GuestError> {
        let mut result = Vec::new();
        for (actor, slot) in &self.projections {
            if self.owned.contains(actor) || self.retired.contains(actor) {
                continue;
            }
            for record in self.actor_records(actor) {
                for field in &record.fields {
                    if !mod_field_is_shared(field) || field.access == Some(FieldAccess::ReadOnly) || self.host.is_body_output(actor, &record.id, field.offset) {
                        continue;
                    }
                    let address = record.address + slot * record.stride + field.offset;
                    result.push(FieldObservation { actor: actor.clone(), address, field: field.clone(), bytes: self.host.read_bytes(address, mod_field_size(field))? });
                }
            }
        }
        Ok(result)
    }

    /// Capture source writes into pending canonical commits.
    pub fn capture_writes(&mut self) -> Result<(), GuestError> {
        let Some(frame_index) = self.frames.len().checked_sub(1) else { return Ok(()) };
        let scope = self.weapon_inputs.last().cloned();
        let mut changed = Vec::new();
        for entry in &self.frames[frame_index].observations {
            let skip = scope.as_ref().is_some_and(|(actor, frame)| {
                *frame == Some(frame_index) && actor == &entry.actor && !matches!(entry.field.binding, ModActorBinding::Health { .. } | ModActorBinding::Inventory { .. })
            });
            if skip {
                continue;
            }
            let current = self.host.read_bytes(entry.address, entry.bytes.len())?;
            if current != entry.bytes {
                changed.push((entry.actor.clone(), entry.address, entry.field.clone()));
            }
        }
        for (actor, field) in changed.iter().map(|(actor, _, field)| (actor, field)) {
            if self.pickup_scopes.last().is_some() && self.projection_writes == 0 {
                self.host.check_pickup_write(actor, field)?;
            }
        }
        let observations = self.observe()?;
        for frame in &mut self.frames {
            frame.observations = observations.clone();
        }
        for (actor, address, field) in changed {
            if self.retired.contains(&actor) {
                continue;
            }
            let value = match &field.binding {
                ModActorBinding::Team { encoding, .. } | ModActorBinding::Score { encoding } | ModActorBinding::Health { encoding } | ModActorBinding::Inventory { encoding, .. } => {
                    let word = self.host.read_i32(address)?;
                    if *encoding == ModScalar::Float32 {
                        let number = f64::from(f32::from_bits(word as u32));
                        encode_mod_scalar(number, *encoding)?;
                        CanonicalField::Word(number)
                    } else {
                        encode_mod_scalar(f64::from(word), *encoding)?;
                        CanonicalField::Word(f64::from(word))
                    }
                }
                _ => {
                    let vector = self.read_vector(address)?;
                    for component in [vector.x as f64, vector.y as f64, vector.z as f64] {
                        encode_mod_scalar(component, ModScalar::Float32)?;
                    }
                    CanonicalField::Vec(vector)
                }
            };
            self.frames[frame_index].pending.push(PendingCommit { actor, address, field, value });
        }
        Ok(())
    }

    /// Flush pending canonical commits.
    pub fn flush(&mut self) -> Result<(), GuestError> {
        self.capture_writes()?;
        let Some(frame_index) = self.frames.len().checked_sub(1) else { return Ok(()) };
        while self.frames[frame_index].cursor < self.frames[frame_index].pending.len() {
            let commit = self.frames[frame_index].cursor;
            self.frames[frame_index].cursor += 1;
            let pending = self.frames[frame_index].pending[commit].clone();
            if !self.host.is_live(&pending.actor) || !self.host.is_owned(&pending.actor) {
                return Err(GuestError::invalid("QVM mod wrote an expired actor"));
            }
            self.host.commit_field(&pending.actor, &pending.field, pending.value)?;
            if !self.retired.contains(&pending.actor) && self.host.is_live(&pending.actor) {
                self.projection_writes += 1;
                let refreshed = self.host.canonical_field(&pending.actor, &pending.field);
                match refreshed {
                    Ok(CanonicalField::Word(number)) => {
                        let encoding = match &pending.field.binding {
                            ModActorBinding::Team { encoding, .. }
                            | ModActorBinding::Score { encoding }
                            | ModActorBinding::Health { encoding }
                            | ModActorBinding::Inventory { encoding, .. } => *encoding,
                            _ => ModScalar::Int32,
                        };
                        let written = self.host.write_i32(pending.address, encode_mod_scalar(number, encoding)?);
                        self.projection_writes -= 1;
                        written?;
                    }
                    Ok(CanonicalField::Vec(vector)) => {
                        let written = self.write_vector(pending.address, vector);
                        self.projection_writes -= 1;
                        written?;
                    }
                    Err(error) => {
                        self.projection_writes -= 1;
                        return Err(error);
                    }
                }
            }
        }
        self.frames[frame_index].pending.clear();
        self.frames[frame_index].cursor = 0;
        Ok(())
    }

    /// Lower a call and open a projection frame.
    pub fn begin_call(&mut self, call: &QvmModSourceCall, inputs: &BTreeMap<ModCallbackInput, ModRuntimeValue>) -> Result<LoweredCall, GuestError> {
        self.current()?;
        if self.frames.len() >= MOD_MAX_CALL_DEPTH {
            return Err(GuestError::invalid("QVM mod callback recursion exceeds 64 calls"));
        }
        if !self.frames.is_empty() {
            self.flush()?;
            self.host.publish_player_events();
        }
        let saved_scratch = self.scratch;
        let mut saved_globals = Vec::new();
        for global in &call.globals {
            let size = if matches!(global.value, QvmModValue::Vector(_)) { 12 } else { 4 };
            saved_globals.push((global.address, self.host.read_bytes(global.address, size)?));
        }
        let lowered = self.lower_call(call, inputs);
        match lowered {
            Ok(words) => {
                if let Some(lifecycle) = self.declaration.source_actors.clone() {
                    if call.entry == lifecycle.release.entry {
                        let pointer = words.get(lifecycle.release.argument).copied().ok_or_else(|| GuestError::invalid("Missing source release argument"))?;
                        let slot = self.pointer_slot(pointer.max(0) as usize)?;
                        let actor = self.actor_at(slot);
                        if actor.as_ref().is_some_and(|actor| !self.owned.contains(actor)) {
                            return Err(GuestError::invalid("QVM source removal of a foreign actor requires its owner continuation"));
                        }
                        if let Some(actor) = actor {
                            self.host.before_release(&actor)?;
                        }
                    }
                }
                self.refresh()?;
                let observations = self.observe()?;
                self.frames.push(ModCallFrame { observations, pending: Vec::new(), cursor: 0 });
                Ok(LoweredCall { words, saved_globals, saved_scratch })
            }
            Err(error) => {
                for (address, bytes) in &saved_globals {
                    self.host.write_bytes(*address, bytes)?;
                }
                self.host.discard_player_events();
                self.scratch = saved_scratch;
                Err(error)
            }
        }
    }

    fn lower_call(&mut self, call: &QvmModSourceCall, inputs: &BTreeMap<ModCallbackInput, ModRuntimeValue>) -> Result<Vec<i32>, GuestError> {
        let words = call.arguments.iter().map(|value| self.lower(value, inputs)).collect::<Result<Vec<_>, _>>()?;
        for global in &call.globals {
            let word = self.lower(&global.value, inputs)?;
            if matches!(global.value, QvmModValue::Vector(_)) {
                self.host.copy_bytes(global.address, word.max(0) as usize, 12)?;
            } else {
                self.host.write_i32(global.address, word)?;
            }
        }
        Ok(words)
    }

    /// Close a projection frame after execution.
    pub fn finish_call(&mut self, lowered: LoweredCall, succeeded: bool) -> Result<(), GuestError> {
        if !self.closed {
            self.current()?;
            if succeeded {
                self.flush()?;
            } else if let Some(frame) = self.frames.last_mut() {
                frame.pending.clear();
                frame.cursor = 0;
            }
            if succeeded {
                self.host.publish_player_events();
            } else {
                self.host.discard_player_events();
            }
        }
        self.frames.pop();
        if !self.closed {
            for (address, bytes) in &lowered.saved_globals {
                self.host.write_bytes(*address, bytes)?;
            }
            self.host.discard_player_events();
        }
        self.scratch = lowered.saved_scratch;
        Ok(())
    }

    /// Invoke a source call with projection tracking.
    pub fn invoke(&mut self, call: &QvmModSourceCall, inputs: &BTreeMap<ModCallbackInput, ModRuntimeValue>) -> Result<f64, GuestError> {
        let lowered = self.begin_call(call, inputs)?;
        let mut succeeded = false;
        let outcome = (|| -> Result<f64, GuestError> {
            let result = self.host.call_module(&lowered.words, call.entry)?;
            self.complete_direct_lifecycle(call, &lowered.words, result)?;
            match call.returns {
                ModReturns::Void => Ok(0.0),
                ModReturns::Int32 => Ok(f64::from(result)),
                ModReturns::Float32 => {
                    let value = f64::from(f32::from_bits(result as u32));
                    if !value.is_finite() {
                        return Err(GuestError::invalid("QVM mod returned a nonfinite scalar"));
                    }
                    Ok(value)
                }
            }
        })();
        if outcome.is_ok() {
            succeeded = true;
        }
        let finished = self.finish_call(lowered, succeeded);
        self.finish_retired_projections()?;
        finished?;
        let value = outcome?;
        if succeeded && !self.closed && self.frames.is_empty() {
            self.publish()?;
        }
        Ok(value)
    }

    fn complete_direct_lifecycle(&mut self, call: &QvmModSourceCall, words: &[i32], result: i32) -> Result<(), GuestError> {
        let Some(lifecycle) = self.declaration.source_actors.clone() else { return Ok(()) };
        if call.entry == lifecycle.allocate {
            let slot = self.pointer_slot(result.max(0) as usize)?;
            self.adopt_source(slot)?;
        } else if call.entry == lifecycle.release.entry {
            let pointer = words.get(lifecycle.release.argument).copied().ok_or_else(|| GuestError::invalid("Missing source release argument"))?;
            self.retire_source_slot(self.pointer_slot(pointer.max(0) as usize)?)?;
        }
        Ok(())
    }

    /// Adopt an allocated source slot.
    pub fn adopt_source(&mut self, slot: usize) -> Result<(), GuestError> {
        let lifecycle = self.declaration.source_actors.clone().ok_or_else(|| GuestError::invalid("Missing QVM source actor declaration"))?;
        if slot < self.declaration.clients.as_ref().map_or(0, |clients| clients.maximum) {
            return Err(GuestError::invalid("QVM allocator returned a reserved client row"));
        }
        if self.actor_at(slot).is_some() {
            return Err(GuestError::invalid("Authored QVM allocator returned an occupied or inactive entity"));
        }
        let entity = self.entity_address(slot)?;
        if self.host.read_i32(entity + lifecycle.inuse)? == 0 {
            return Err(GuestError::invalid("Authored QVM allocator returned an occupied or inactive entity"));
        }
        let actor = self.host.adopt_source(slot)?;
        self.projections.insert(actor.clone(), slot);
        self.owned.insert(actor);
        Ok(())
    }

    /// Retire a released source slot.
    pub fn retire_source_slot(&mut self, slot: usize) -> Result<(), GuestError> {
        let Some(lifecycle) = self.declaration.source_actors.clone() else { return Ok(()) };
        let Some(actor) = self.actor_at(slot) else { return Ok(()) };
        if self.host.read_i32(self.entity_address(slot)? + lifecycle.inuse)? != 0 {
            return Ok(());
        }
        if !self.owned.contains(&actor) {
            return Err(GuestError::invalid("QVM source removed a foreign actor without its owner continuation"));
        }
        self.host.retire_source(slot)?;
        self.owned.remove(&actor);
        self.projections.remove(&actor);
        self.event_keys.remove(&actor);
        Ok(())
    }

    /// Run a pickup rule with borrowed context words.
    pub fn pickup_context<R>(
        &mut self,
        definition: &QvmModPickup,
        offer: &PickupOffer,
        inputs: &BTreeMap<ModCallbackInput, ModRuntimeValue>,
        execute: impl FnOnce(&mut Self) -> Result<R, GuestError>,
    ) -> Result<R, GuestError> {
        self.current()?;
        if offer.pickup == offer.recipient || self.host.has_client(&offer.pickup) || self.owned.contains(&offer.pickup) {
            return Err(GuestError::invalid("QVM pickup context requires a foreign non-player pickup actor"));
        }
        let record = self.declaration.entity_record.clone().ok_or_else(|| GuestError::invalid("QVM pickup requires an actor projection"))?;
        self.pickup_depth += 1;
        let saved_scratch = self.scratch;
        self.pointer(Some(&offer.recipient), &record)?;
        self.pointer(Some(&offer.pickup), &record)?;
        let mut restore = Vec::new();
        for field in &definition.context {
            let address = self.pointer(Some(&offer.pickup), &field.record)? + field.offset;
            let word = self.lower(&field.value, inputs)?;
            restore.push((address, self.host.read_bytes(address, 4)?));
            self.host.write_i32(address, word)?;
        }
        self.pickup_scopes.push(offer.clone());
        let outcome = execute(self);
        self.pickup_scopes.pop();
        if !self.closed {
            for (address, bytes) in &restore {
                self.host.write_bytes(*address, bytes)?;
            }
        }
        self.scratch = saved_scratch;
        self.pickup_depth -= 1;
        self.finish_retired_projections()?;
        outcome
    }

    fn entity_record(&self) -> Result<QvmModActorRecord, GuestError> {
        let record = self.declaration.entity_record.as_deref().and_then(|id| self.records.get(id)).cloned();
        if record.is_none_or(|record| record.stride < qvm_shared_entity_bytes(self.declaration.abi_profile)) {
            return Err(GuestError::invalid("QVM engine service requires its declared sharedEntity_t array"));
        }
        Ok(record.expect("checked record"))
    }

    /// Entity address of a slot.
    pub fn entity_address(&self, slot: usize) -> Result<usize, GuestError> {
        let record = self.entity_record()?;
        if slot >= record.capacity {
            return Err(GuestError::invalid("QVM entity exceeds its declared source array"));
        }
        Ok(record.address + slot * record.stride)
    }

    /// Slot of an entity pointer.
    pub fn pointer_slot(&self, pointer: usize) -> Result<usize, GuestError> {
        let record = self.entity_record()?;
        if pointer < record.address || (pointer - record.address) % record.stride != 0 {
            return Err(GuestError::invalid("QVM entity exceeds its declared source array"));
        }
        let slot = (pointer - record.address) / record.stride;
        self.entity_address(slot)?;
        Ok(slot)
    }

    /// Entity slot of an actor, projecting on first use.
    pub fn entity_slot(&mut self, actor: &ActorId) -> Result<usize, GuestError> {
        let id = self.declaration.entity_record.clone().ok_or_else(|| GuestError::invalid("QVM mod has no engine entity record"))?;
        self.pointer(Some(actor), &id)?;
        self.projections.get(actor).copied().ok_or_else(|| GuestError::invalid("Missing QVM actor slot"))
    }

    /// Set a configstring, bumping the presentation revision on change.
    pub fn set_configstring(&mut self, index: u32, value: String) {
        if self.configstrings.get(&index).is_some_and(|current| *current == value) {
            return;
        }
        self.configstrings.insert(index, value.clone());
        self.presentation_revision += 1;
        self.presentation_state = None;
        self.scene_command(format!("cs {index} {value:?}"), None);
    }

    /// Queue a scene server command (scene runtime only).
    pub fn scene_command(&mut self, text: String, recipient: Option<ActorId>) {
        if !self.declaration.presentation.as_ref().is_some_and(|presentation| presentation.is_scene()) {
            return;
        }
        self.scene_command_sequence += 1;
        self.scene_commands.push_back((self.scene_command_sequence, recipient, text));
        while self.scene_commands.len() > 64 {
            self.scene_commands.pop_front();
        }
        self.scene_dirty = true;
    }

    /// Current game-state record, rebuilding after configstring changes.
    pub fn game_state(&mut self) -> Result<super::mod_presentation_checkpoint::SourceGameState, GuestError> {
        let (server, system) = self.host.server_info();
        let server_changed = self.configstrings.get(&0).is_none_or(|current| *current != server);
        let system_changed = self.configstrings.get(&1).is_none_or(|current| *current != system);
        if server_changed {
            self.set_configstring(0, server);
        }
        if system_changed {
            self.set_configstring(1, system);
        }
        if self.presentation_state.is_none() {
            let entries: Vec<(u32, String)> =
                self.configstrings.iter().filter(|(_, value)| !value.is_empty()).map(|(index, value)| (*index, value.clone())).collect();
            self.presentation_state = Some(self.host.build_game_state(&entries)?);
        }
        Ok(self.presentation_state.clone().expect("built game state"))
    }

    /// Publish entity events and player events after a call settles.
    pub fn publish(&mut self) -> Result<(), GuestError> {
        self.scene_dirty = true;
        self.host.publish_player_events();
        if self.declaration.presentation.as_ref().is_some_and(|presentation| presentation.is_scene()) {
            return Ok(());
        }
        let boundary = self.declaration.source_actors.as_ref().map_or(u32::MAX, |actors| actors.event_entity_type);
        let time_ms = (self.host.time_seconds() * 1000.0).trunc() as i32;
        for view in self.host.owned_entity_views() {
            if !view.linked {
                continue;
            }
            if view.event == 0 && (view.etype as u32) < boundary {
                self.event_keys.remove(&view.actor);
                continue;
            }
            let key = format!("{}:{}", view.event, view.etype);
            if self.event_keys.get(&view.actor).is_some_and(|current| *current == key) {
                continue;
            }
            self.event_keys.insert(view.actor.clone(), key);
            self.host.emit(ProviderEmit::EntityEvent { actor: view.actor, state: view.state, origin: view.origin, time_ms })?;
        }
        Ok(())
    }

    /// Advance source actors and client frames.
    pub fn advance(&mut self, time_seconds: f64, elapsed_seconds: f64) -> Result<(), GuestError> {
        self.current()?;
        let lifecycle = self.declaration.source_actors.clone();
        if let Some(update) = lifecycle.as_ref().and_then(|actors| actors.update.clone()) {
            let mut owned: Vec<(ActorId, usize)> =
                self.owned.iter().map(|actor| (actor.clone(), self.projections.get(actor).copied().unwrap_or(0))).collect();
            owned.sort_by_key(|(_, slot)| *slot);
            for (actor, _) in owned {
                if self.closed {
                    return Ok(());
                }
                if self.host.is_live(&actor) {
                    let inputs = BTreeMap::from([
                        (ModCallbackInput::Own, ModRuntimeValue::Actor(Some(actor.clone()))),
                        (ModCallbackInput::Time, ModRuntimeValue::Float(time_seconds)),
                        (ModCallbackInput::Elapsed, ModRuntimeValue::Float(elapsed_seconds)),
                    ]);
                    self.invoke(&update, &inputs)?;
                }
            }
        }
        let run_frame = lifecycle.as_ref().and_then(|actors| actors.frame.clone());
        let mut clients = true;
        if let Some(frame) = run_frame {
            self.host.begin_actor_frame()?;
            let inputs = BTreeMap::from([
                (ModCallbackInput::Time, ModRuntimeValue::Float(time_seconds)),
                (ModCallbackInput::Elapsed, ModRuntimeValue::Float(elapsed_seconds)),
            ]);
            let outcome = self.invoke(&frame.call, &inputs);
            clients = self.host.end_actor_frame()? && outcome.is_ok();
            outcome?;
        }
        if !self.closed && clients {
            let frame_calls = self.declaration.clients.as_ref().map_or(Vec::new(), |clients| clients.frame.clone());
            for actor in self.host.frame_actors() {
                for call in &frame_calls {
                    let inputs = BTreeMap::from([
                        (ModCallbackInput::Own, ModRuntimeValue::Actor(Some(actor.clone()))),
                        (ModCallbackInput::Time, ModRuntimeValue::Float(time_seconds)),
                        (ModCallbackInput::Elapsed, ModRuntimeValue::Float(elapsed_seconds)),
                    ]);
                    self.invoke(call, &inputs)?;
                }
            }
        }
        if !self.closed {
            self.publish()?;
        }
        Ok(())
    }

    /// Presentation records for owned linked entities.
    pub fn presentations(&self) -> Vec<ModPresentation> {
        let scene = self.declaration.presentation.as_ref().is_some_and(|presentation| presentation.is_scene());
        self.host
            .owned_entity_views()
            .into_iter()
            .filter(|view| {
                view.linked
                    && view.sv_flags & 1 == 0
                    && view.model_index != 0
                    && (view.inline_model || self.configstrings.get(&(32 + view.model_index.max(0) as u32)).is_some_and(|path| !path.is_empty()))
            })
            .map(|view| {
                let path = if view.inline_model {
                    format!("*{}", view.model_index)
                } else {
                    self.configstrings.get(&(32 + view.model_index.max(0) as u32)).cloned().unwrap_or_default()
                };
                ModPresentation { actor: view.actor, path, frame: view.frame, origin: view.origin, angles: view.angles, render_owner_source_client: scene }
            })
            .collect()
    }

    /// Bind the command port.
    pub fn bind_commands(&mut self) -> Result<(), GuestError> {
        self.current()?;
        if self.commands_bound {
            return Err(GuestError::invalid("QVM mod commands are already bound"));
        }
        self.commands_bound = true;
        Ok(())
    }

    /// Run a console command through the source.
    pub fn console_command(&mut self, argv: &[String]) -> Result<bool, GuestError> {
        self.current()?;
        let call = QvmModSourceCall { entry: 0, arguments: vec![QvmModValue::Int32(ModCallbackValue::Float(9.0))], globals: Vec::new(), returns: ModReturns::Int32 };
        let lowered = self.begin_call(&call, &BTreeMap::new())?;
        let mut succeeded = false;
        let outcome = self.host.command_module(&lowered.words, argv).map(|result| result != 0);
        if outcome.is_ok() {
            succeeded = true;
        }
        let finished = self.finish_call(lowered, succeeded);
        self.finish_retired_projections()?;
        finished?;
        let result = outcome?;
        if succeeded && !self.closed && self.frames.is_empty() {
            self.publish()?;
        }
        Ok(result)
    }

    /// Run a client command through the source.
    pub fn presentation_client_command(&mut self, actor: &ActorId, argv: &[String]) -> Result<(), GuestError> {
        self.current()?;
        if !self.host.admitted_client(actor) {
            return Err(GuestError::invalid("Component client command requires an admitted live source client"));
        }
        let slot = self.projections.get(actor).copied();
        if slot.is_none_or(|slot| !self.host.players().iter().any(|(player, bound, admitted)| player == actor && *bound == slot && *admitted)) {
            return Err(GuestError::invalid("Component client command has no current source projection"));
        }
        let call = QvmModSourceCall {
            entry: 0,
            arguments: vec![
                QvmModValue::Int32(ModCallbackValue::Float(6.0)),
                QvmModValue::Int32(ModCallbackValue::Float(slot.expect("checked slot") as f64)),
            ],
            globals: Vec::new(),
            returns: ModReturns::Void,
        };
        let mut inputs = BTreeMap::new();
        inputs.insert(ModCallbackInput::Own, ModRuntimeValue::Actor(Some(actor.clone())));
        inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(self.host.time_seconds()));
        let lowered = self.begin_call(&call, &inputs)?;
        let mut succeeded = false;
        let outcome = self.host.command_module(&lowered.words, argv);
        if outcome.is_ok() {
            succeeded = true;
        }
        let finished = self.finish_call(lowered, succeeded);
        self.finish_retired_projections()?;
        finished?;
        outcome?;
        if succeeded && !self.closed && self.frames.is_empty() {
            self.publish()?;
        }
        Ok(())
    }

    /// Read a script resource.
    pub fn read_script(&self, name: &str) -> Result<Option<String>, GuestError> {
        self.current()?;
        Ok(self.host.read_script(name))
    }

    /// Run initialization calls and remember source defaults.
    pub fn initialize(&mut self) -> Result<(), GuestError> {
        self.host.reserve_protection()?;
        let lifecycle = self.declaration.source_actors.clone();
        for (address, value) in qvm_actor_bootstrap(
            &lifecycle.as_ref().map_or(Vec::new(), |actors| actors.initial_stores.clone()),
            &self.artifact.image,
            &self.declaration.actor_records,
        )? {
            self.host.write_i32(address, value)?;
        }
        for call in self.declaration.initialize.clone() {
            let inputs = BTreeMap::from([(ModCallbackInput::Time, ModRuntimeValue::Float(self.host.time_seconds()))]);
            let lowered = self.begin_call(&call, &inputs)?;
            let mut succeeded = false;
            let outcome = self.host.call_module(&lowered.words, call.entry).and_then(|result| {
                self.complete_direct_lifecycle(&call, &lowered.words, result)?;
                Ok(())
            });
            if outcome.is_ok() {
                succeeded = true;
            }
            self.finish_call(lowered, succeeded)?;
            outcome?;
        }
        self.remember_defaults()?;
        self.host.start_clients()?;
        self.publish()?;
        Ok(())
    }

    fn remember_defaults(&mut self) -> Result<(), GuestError> {
        for record in self.records.values() {
            let bytes = self.host.read_bytes(record.address, record.stride * record.capacity)?;
            self.defaults.insert(record.id.clone(), bytes);
        }
        Ok(())
    }

    /// Reserve protection channels.
    pub fn reserve_protection(&mut self) -> Result<(), GuestError> {
        self.host.reserve_protection()
    }

    /// Activate protection, pickups, and match bindings.
    pub fn activate_protection(&mut self) -> Result<(), GuestError> {
        self.host.activate_protection()
    }

    /// Current scene publication, rebuilding after scene changes.
    pub fn scene_publication(&mut self) -> Result<super::mod_presentation_checkpoint::ModScenePublication, GuestError> {
        self.current()?;
        if !self.frames.is_empty() {
            return Err(GuestError::invalid("Cannot read presentation during an original source call"));
        }
        let game_state = self.game_state()?;
        let server_time = (self.host.time_seconds() * 1000.0).trunc() as i32;
        if !self.scene_dirty {
            if let Some(state) = self.scene_state.clone() {
                if state.server_time == server_time {
                    return Ok(state);
                }
            }
        }
        let mut entities = Vec::new();
        for (actor, slot) in self.projections.clone() {
            if !self.host.is_live(&actor) || self.retired.contains(&actor) {
                continue;
            }
            let link = self.host.entity_link(slot)?;
            let state = super::mod_presentation_checkpoint::SourceEntityState::from_bytes(
                &self.host.read_bytes(self.entity_address(slot)?, super::mod_presentation_checkpoint::entity_state_len(self.declaration.abi_profile))?,
                self.declaration.abi_profile,
            )?;
            entities.push(super::mod_presentation_checkpoint::SceneEntity {
                actor,
                owned: self.owned.contains(&actor),
                linked: link.linked,
                server_flags: link.sv_flags,
                single_client: link.single_client,
                bounds_min: link.abs_min,
                bounds_max: link.abs_max,
                state,
            });
        }
        let mut clients = Vec::new();
        for (actor, slot, admitted) in self.host.players() {
            if !admitted || !self.host.is_live(&actor) {
                continue;
            }
            let state = super::mod_presentation_checkpoint::SourcePlayerState::from_bytes(&self.host.player_state_bytes(&actor)?, self.declaration.abi_profile)?;
            clients.push(super::mod_presentation_checkpoint::SceneClient { actor, slot, state });
        }
        self.scene_revision += 1;
        let commands = self.scene_commands.iter().map(|(sequence, recipient, text)| super::mod_presentation_checkpoint::SceneCommand {
            sequence: *sequence,
            recipient: recipient.clone(),
            text: text.clone(),
        }).collect();
        let state = super::mod_presentation_checkpoint::ModScenePublication {
            revision: self.scene_revision,
            server_time,
            game_state_revision: self.presentation_revision,
            game_state,
            entities,
            clients,
            commands,
        };
        self.scene_state = Some(state.clone());
        self.scene_dirty = false;
        Ok(state)
    }

    /// Presentation context for one viewer, if admitted.
    pub fn context_for(&mut self, viewer: &ActorId) -> Result<Option<ProviderViewerContext>, GuestError> {
        self.current()?;
        if !self.frames.is_empty() {
            return Err(GuestError::invalid("Cannot read presentation during an original source call"));
        }
        if !self.host.admitted_client(viewer) {
            return Ok(None);
        }
        let Some(client_number) = self.host.client_slot(viewer) else {
            return Err(GuestError::invalid("Presentation viewer has no admitted source client slot"));
        };
        if self.declaration.presentation.as_ref().is_some_and(|presentation| presentation.is_scene()) {
            let state = self.scene_publication()?;
            if !state.clients.iter().any(|row| row.actor == *viewer) {
                return Ok(None);
            }
            let player = state.clients.iter().find(|row| row.actor == *viewer).expect("checked player").state.clone();
            return Ok(Some(ProviderViewerContext {
                client_number,
                game_state_revision: state.game_state_revision,
                server_time: state.server_time,
                player_state: player,
            }));
        }
        let game_state_revision = self.presentation_revision;
        let state = super::mod_presentation_checkpoint::SourcePlayerState::from_bytes(&self.host.player_state_bytes(viewer)?, self.declaration.abi_profile)?;
        Ok(Some(ProviderViewerContext {
            client_number,
            game_state_revision,
            server_time: (self.host.time_seconds() * 1000.0).trunc() as i32,
            player_state: state,
        }))
    }

    /// Live projection bindings as `(actor, slot, owned)`.
    pub fn bindings(&self) -> Vec<(ActorId, usize, bool)> {
        self.projections
            .iter()
            .filter(|(actor, _)| self.host.is_live(actor) && !self.retired.contains(*actor))
            .map(|(actor, slot)| (actor.clone(), *slot, self.owned.contains(actor)))
            .collect()
    }

    /// Current and baseline scene publications (scene runtime only).
    pub fn scene(
        &mut self,
    ) -> Result<(super::mod_presentation_checkpoint::ModScenePublication, Option<super::mod_presentation_checkpoint::ModScenePublication>), GuestError> {
        if !self.declaration.presentation.as_ref().is_some_and(|presentation| presentation.is_scene()) {
            return Err(GuestError::invalid("Event-only component has no source scene publication"));
        }
        let current = self.scene_publication()?;
        Ok((current, self.scene_baseline.clone()))
    }

    /// Capture the host half of a checkpoint.
    pub fn capture_host_state(&mut self) -> Result<ProfileValue, GuestError> {
        self.current()?;
        self.host.assert_subcomponents_idle()?;
        let projections = self
            .projections
            .iter()
            .map(|(actor, slot)| {
                let saved = SavedActorId::from(actor);
                ProfileValue::record(vec![
                    ("actor", ProfileValue::record(vec![("slot", ProfileValue::Int(i64::from(saved.slot))), ("generation", ProfileValue::Int(i64::from(saved.generation)))])),
                    ("slot", ProfileValue::Int(*slot as i64)),
                    ("owned", ProfileValue::Bool(self.owned.contains(actor))),
                ])
            })
            .collect();
        let client_slots = self
            .host
            .players()
            .into_iter()
            .map(|(actor, slot, admitted)| {
                let saved = SavedActorId::from(&actor);
                ProfileValue::record(vec![
                    ("actor", ProfileValue::record(vec![("slot", ProfileValue::Int(i64::from(saved.slot))), ("generation", ProfileValue::Int(i64::from(saved.generation)))])),
                    ("slot", ProfileValue::Int(slot as i64)),
                    ("admitted", ProfileValue::Bool(admitted)),
                ])
            })
            .collect();
        let defaults = self
            .defaults
            .iter()
            .map(|(id, bytes)| ProfileValue::record(vec![("id", ProfileValue::Str(id.clone())), ("bytes", ProfileValue::Bytes(bytes.clone()))]))
            .collect();
        let configstrings = self
            .configstrings
            .iter()
            .map(|(index, value)| ProfileValue::record(vec![("index", ProfileValue::Int(i64::from(*index))), ("value", ProfileValue::Str(value.clone()))]))
            .collect();
        Ok(ProfileValue::record(vec![
            ("version", ProfileValue::Int(1)),
            ("projections", ProfileValue::Array(projections)),
            ("nextSlot", ProfileValue::Int(self.next_slot as i64)),
            ("clientSlots", ProfileValue::Array(client_slots)),
            ("defaults", ProfileValue::Array(defaults)),
            ("configstrings", ProfileValue::Array(configstrings)),
            ("presentationRevision", ProfileValue::Int(self.presentation_revision as i64)),
            ("sceneRevision", ProfileValue::Int(self.scene_revision as i64)),
            ("sceneDirty", ProfileValue::Bool(self.scene_dirty)),
            ("sceneCommandSequence", ProfileValue::Int(self.scene_command_sequence as i64)),
        ]))
    }

    /// Restore host state captured by [`QvmModProvider::capture_host_state`].
    pub fn restore_host_state(
        &mut self,
        host: &ProfileValue,
        data: &[u8],
        resolve: &dyn Fn(SavedActorId) -> Option<ActorId>,
    ) -> Result<(), GuestError> {
        self.current()?;
        self.host.assert_subcomponents_idle()?;
        let saved = read_mod_host_image(host, &self.declaration, data)?;
        self.projections.clear();
        self.owned.clear();
        self.event_keys.clear();
        for entry in &saved.projections {
            let actor = resolve(entry.actor).ok_or_else(|| GuestError::invalid("Saved QVM mod actor is unavailable or has the wrong owner"))?;
            self.projections.insert(actor.clone(), entry.slot);
            if entry.owned {
                if !self.host.is_owned(&actor) {
                    return Err(GuestError::invalid("Saved QVM mod actor is unavailable or has the wrong owner"));
                }
                self.owned.insert(actor);
            }
        }
        self.next_slot = saved.next_slot;
        self.defaults.clear();
        for (id, bytes) in saved.defaults {
            self.defaults.insert(id, bytes);
        }
        self.configstrings.clear();
        for (index, value) in saved.configstrings {
            self.configstrings.insert(index, value);
        }
        let reader = ProfileReader::new(host);
        if !reader.field("presentationRevision")?.is_undefined() {
            self.presentation_revision = reader.field("presentationRevision")?.integer(0)? as u64;
        }
        if !reader.field("sceneRevision")?.is_undefined() {
            self.scene_revision = reader.field("sceneRevision")?.integer(0)? as u64;
        }
        if !reader.field("sceneDirty")?.is_undefined() {
            self.scene_dirty = reader.field("sceneDirty")?.boolean()?;
        }
        if !reader.field("sceneCommandSequence")?.is_undefined() {
            self.scene_command_sequence = reader.field("sceneCommandSequence")?.integer(0)? as u64;
        }
        self.presentation_state = None;
        self.scene_state = None;
        self.scene_baseline = None;
        self.frames.clear();
        self.retired.clear();
        self.scratch = self.scratch_start;
        Ok(())
    }

    /// Push a weapon-input scope for the current frame, if any.
    pub fn push_weapon_input(&mut self, actor: ActorId) {
        let frame = if self.frames.is_empty() { None } else { Some(self.frames.len() - 1) };
        self.weapon_inputs.push((actor, frame));
    }

    /// Pop a weapon-input scope.
    pub fn pop_weapon_input(&mut self) {
        self.weapon_inputs.pop();
    }

    /// Close the provider, aggregating cleanup failures.
    pub fn close(&mut self) -> Result<(), GuestError> {
        let mut errors = self.host.close_subcomponents();
        self.closed = true;
        self.host.discard_player_events();
        for actor in std::mem::take(&mut self.owned) {
            if self.host.is_live(&actor) {
                if let Err(error) = self.host.release_owned(&actor) {
                    errors.push(error);
                }
            }
        }
        self.projections.clear();
        self.retired.clear();
        self.event_keys.clear();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(GuestError::callback(format!("QVM mod cleanup failed: {}", errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct FakeHost {
        memory: Vec<u8>,
        live: HashSet<ActorId>,
        owned: HashSet<ActorId>,
        canonical: HashMap<ActorId, f64>,
        calls: Vec<(Vec<i32>, usize)>,
        return_value: i32,
        published: usize,
        discarded: usize,
    }

    impl FakeHost {
        fn new() -> Self {
            Self {
                memory: vec![0; 65536],
                live: HashSet::new(),
                owned: HashSet::new(),
                canonical: HashMap::new(),
                calls: Vec::new(),
                return_value: 0,
                published: 0,
                discarded: 0,
            }
        }
    }

    impl ModProviderHost for FakeHost {
        fn current(&self) -> Result<(), GuestError> {
            Ok(())
        }
        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }
        fn is_owned(&self, actor: &ActorId) -> bool {
            self.owned.contains(actor)
        }
        fn time_seconds(&self) -> f64 {
            1.5
        }
        fn call_module(&mut self, words: &[i32], entry: usize) -> Result<i32, GuestError> {
            self.calls.push((words.to_vec(), entry));
            Ok(self.return_value)
        }
        fn command_module(&mut self, words: &[i32], _argv: &[String]) -> Result<i32, GuestError> {
            self.calls.push((words.to_vec(), usize::MAX));
            Ok(1)
        }
        fn read_i32(&self, address: usize) -> Result<i32, GuestError> {
            Ok(i32::from_le_bytes(self.memory[address..address + 4].try_into().map_err(|_| GuestError::invalid("oob"))?))
        }
        fn write_i32(&mut self, address: usize, value: i32) -> Result<(), GuestError> {
            self.memory[address..address + 4].copy_from_slice(&value.to_le_bytes());
            Ok(())
        }
        fn read_bytes(&self, address: usize, len: usize) -> Result<Vec<u8>, GuestError> {
            Ok(self.memory[address..address + len].to_vec())
        }
        fn write_bytes(&mut self, address: usize, bytes: &[u8]) -> Result<(), GuestError> {
            self.memory[address..address + bytes.len()].copy_from_slice(bytes);
            Ok(())
        }
        fn copy_bytes(&mut self, dest: usize, src: usize, len: usize) -> Result<(), GuestError> {
            self.memory.copy_within(src..src + len, dest);
            Ok(())
        }
        fn stack_pointer(&self) -> usize {
            65536 + 65536
        }
        fn canonical_field(&self, actor: &ActorId, field: &QvmModActorField) -> Result<CanonicalField, GuestError> {
            match &field.binding {
                ModActorBinding::Origin | ModActorBinding::Velocity | ModActorBinding::Angles | ModActorBinding::BoundsMin | ModActorBinding::BoundsMax => {
                    Ok(CanonicalField::Vec(vec3(1.0, 2.0, 3.0)))
                }
                _ => Ok(CanonicalField::Word(self.canonical.get(actor).copied().unwrap_or(0.0))),
            }
        }
        fn commit_field(&mut self, actor: &ActorId, _field: &QvmModActorField, value: CanonicalField) -> Result<(), GuestError> {
            if let CanonicalField::Word(number) = value {
                self.canonical.insert(actor.clone(), number);
            }
            Ok(())
        }
        fn has_client(&self, _actor: &ActorId) -> bool {
            false
        }
        fn client_slot(&self, _actor: &ActorId) -> Option<usize> {
            None
        }
        fn admitted_client(&self, _actor: &ActorId) -> bool {
            false
        }
        fn players(&self) -> Vec<(ActorId, usize, bool)> {
            Vec::new()
        }
        fn start_clients(&mut self) -> Result<(), GuestError> {
            Ok(())
        }
        fn frame_actors(&self) -> Vec<ActorId> {
            Vec::new()
        }
        fn release_actor_components(&mut self, _actor: &ActorId) {}
        fn reserve_protection(&mut self) -> Result<(), GuestError> {
            Ok(())
        }
        fn activate_protection(&mut self) -> Result<(), GuestError> {
            Ok(())
        }
        fn assert_subcomponents_idle(&self) -> Result<(), GuestError> {
            Ok(())
        }
        fn close_subcomponents(&mut self) -> Vec<GuestError> {
            Vec::new()
        }
        fn publish_player_events(&mut self) {
            self.published += 1;
        }
        fn discard_player_events(&mut self) {
            self.discarded += 1;
        }
        fn check_pickup_write(&self, _actor: &ActorId, _field: &QvmModActorField) -> Result<(), GuestError> {
            Ok(())
        }
        fn adopt_source(&mut self, _slot: usize) -> Result<ActorId, GuestError> {
            Err(GuestError::invalid("no source actors in fixture"))
        }
        fn retire_source(&mut self, _slot: usize) -> Result<(), GuestError> {
            Ok(())
        }
        fn before_release(&mut self, _actor: &ActorId) -> Result<(), GuestError> {
            Ok(())
        }
        fn release_owned(&mut self, actor: &ActorId) -> Result<(), GuestError> {
            self.owned.remove(actor);
            Ok(())
        }
        fn begin_actor_frame(&mut self) -> Result<(), GuestError> {
            Ok(())
        }
        fn end_actor_frame(&mut self) -> Result<bool, GuestError> {
            Ok(true)
        }
        fn server_info(&self) -> (String, String) {
            (String::new(), String::new())
        }
        fn build_game_state(&self, entries: &[(u32, String)]) -> Result<super::super::mod_presentation_checkpoint::SourceGameState, GuestError> {
            super::super::mod_presentation_checkpoint::SourceGameState::from_entries(entries.iter().map(|(index, value)| (*index, value.clone())))
        }
        fn owned_entity_views(&self) -> Vec<EntityPublishView> {
            Vec::new()
        }
        fn player_state_bytes(&self, _actor: &ActorId) -> Result<Vec<u8>, GuestError> {
            Ok(vec![0; qvm_player_state_bytes(QvmAbi::Modern)])
        }
        fn entity_link(&self, _slot: usize) -> Result<EntityLinkView, GuestError> {
            Ok(EntityLinkView { linked: false, sv_flags: 0, single_client: 0, abs_min: vec3(0.0, 0.0, 0.0), abs_max: vec3(0.0, 0.0, 0.0) })
        }
        fn emit(&mut self, _event: ProviderEmit) -> Result<(), GuestError> {
            Ok(())
        }
        fn read_script(&self, _name: &str) -> Option<String> {
            None
        }
    }

    fn fixture_module() -> ModuleId {
        ModuleId { id: "test:mod".to_string(), artifact_path: "vm/qagame.qvm".to_string(), digest: "sha256:abc".to_string(), revision: "1".to_string() }
    }

    fn fixture_artifact() -> QvmArtifact {
        QvmArtifact {
            module: fixture_module(),
            role: QvmRole::Qagame,
            abi_profile: None,
            image: QvmImage {
                instructions: vec![QvmInstruction::word(QvmOpcode::OpEnter, 64), QvmInstruction::word(QvmOpcode::OpLeave, 0)],
                data_length: 4096,
                literal_length: 0,
                bss_length: 0,
                initialized_length: 4096,
                allocated_data_length: 8192,
            },
        }
    }

    fn fixture_declaration() -> QvmModCallbackDeclaration {
        QvmModCallbackDeclaration {
            version: 1,
            program_path: "vm/qagame.qvm".to_string(),
            program_digest: "sha256:abc".to_string(),
            abi_profile: QvmAbi::Modern,
            presentation: None,
            spawn_entities: None,
            clients: None,
            actor_records: vec![QvmModActorRecord {
                id: "entity".to_string(),
                address: 64,
                stride: 520,
                capacity: 4,
                fields: vec![QvmModActorField { offset: 0, access: None, binding: ModActorBinding::Private { byte_length: 520 } }],
            }],
            entity_record: Some("entity".to_string()),
            source_actors: None,
            combat: None,
            protection: Vec::new(),
            pickups: Vec::new(),
            items: None,
            initialize: Vec::new(),
            callbacks: Vec::new(),
            objectives: Vec::new(),
        }
    }

    #[test]
    fn scalar_encoding_matches_source_words() {
        assert_eq!(encode_mod_scalar(7.9, ModScalar::Int32).unwrap(), 7);
        assert_eq!(encode_mod_scalar(-3.0, ModScalar::Int32).unwrap(), -3);
        assert!(encode_mod_scalar(1e10, ModScalar::Int32).is_err());
        assert!(encode_mod_scalar(f64::NAN, ModScalar::Float32).is_err());
        assert_eq!(encode_mod_scalar(1.5, ModScalar::Float32).unwrap(), (1.5f32).to_bits() as i32);
    }

    #[test]
    fn profile_reader_reports_paths() {
        let value = ProfileValue::record(vec![("items", ProfileValue::Array(vec![ProfileValue::Int(2)]))]);
        let reader = ProfileReader::new(&value);
        assert_eq!(reader.field("items").unwrap().list(|entry| entry.integer(0)).unwrap(), vec![2]);
        assert!(reader.field("missing").unwrap().is_undefined());
        assert!(reader.field("items").unwrap().field("nope").is_err());
    }

    #[test]
    fn region_qualification_accepts_balanced_paths() {
        let instructions = vec![
            QvmInstruction::word(QvmOpcode::OpEnter, 64),
            QvmInstruction::word(QvmOpcode::OpConst, 5),
            QvmInstruction::word(QvmOpcode::OpPop, 0),
            QvmInstruction::word(QvmOpcode::OpLeave, 0),
            QvmInstruction::word(QvmOpcode::OpEnter, 0),
        ];
        assert_eq!(qualify_qvm_region(&instructions, 0, 1, 3).unwrap(), 64);
        let region = QvmRegionEvaluation { entry: 1, join: 3, inputs: vec![8], result: None };
        assert_eq!(qualify_qvm_region_evaluation(&instructions, 0, &region, false).unwrap(), 64);
        assert!(qualify_qvm_region(&instructions, 0, 1, 4).is_err());
    }

    #[test]
    fn minimal_declaration_validates() {
        validate_qvm_mod(&fixture_artifact(), &fixture_declaration()).unwrap();
    }

    #[test]
    fn declaration_rejects_overlapping_arrays() {
        let mut declaration = fixture_declaration();
        declaration.actor_records.push(QvmModActorRecord {
            id: "other".to_string(),
            address: 100,
            stride: 520,
            capacity: 4,
            fields: Vec::new(),
        });
        assert!(validate_qvm_mod(&fixture_artifact(), &declaration).is_err());
    }

    #[test]
    fn declaration_rejects_wrong_role() {
        let mut artifact = fixture_artifact();
        artifact.role = QvmRole::Cgame;
        assert!(validate_qvm_mod(&artifact, &fixture_declaration()).is_err());
    }

    #[test]
    fn declaration_rejects_duplicate_callbacks() {
        let call = QvmModSourceCall { entry: 0, arguments: Vec::new(), globals: Vec::new(), returns: ModReturns::Int32 };
        let binding = |id: &str| ModCallbackBinding { id: id.to_string(), operation: ModCallbackOperation::Damage, stage: CallbackStage::Observe, result: None };
        let mut declaration = fixture_declaration();
        declaration.callbacks.push(QvmModCallback { binding: binding("test:hit"), call: call.clone() });
        declaration.callbacks.push(QvmModCallback { binding: binding("test:hit"), call });
        assert!(validate_qvm_mod(&fixture_artifact(), &declaration).is_err());
    }

    #[test]
    fn projection_assigns_slots_and_restores_defaults() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut host = FakeHost::new();
        host.live.insert(actor.clone());
        let mut provider = QvmModProvider::open(fixture_artifact(), fixture_declaration(), host).unwrap();
        provider.initialize().unwrap();
        let address = provider.pointer(Some(&actor), "entity").unwrap();
        assert_eq!(address, 64);
        assert_eq!(provider.actor_at(0).unwrap(), actor);
        provider.host_mut().write_i32(64, 41).unwrap();
        provider.release_projection(&actor).unwrap();
        assert_eq!(provider.host().read_i32(64).unwrap(), 0);
        assert!(provider.projection_slot(&actor).is_none());
    }

    #[test]
    fn invoke_lowers_values_and_calls_source() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut host = FakeHost::new();
        host.live.insert(actor.clone());
        host.return_value = 9;
        let mut provider = QvmModProvider::open(fixture_artifact(), fixture_declaration(), host).unwrap();
        provider.initialize().unwrap();
        let call = QvmModSourceCall {
            entry: 0,
            arguments: vec![
                QvmModValue::Int32(ModCallbackValue::Float(3.0)),
                QvmModValue::Actor { record: "entity".to_string(), input: ActorInput::Own },
            ],
            globals: Vec::new(),
            returns: ModReturns::Int32,
        };
        let inputs = BTreeMap::from([(ModCallbackInput::Own, ModRuntimeValue::Actor(Some(actor.clone())))]);
        assert_eq!(provider.invoke(&call, &inputs).unwrap(), 9.0);
        assert_eq!(provider.host().calls, vec![(vec![3, 64], 0)]);
    }

    #[test]
    fn canonical_health_round_trips_through_source() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(2, 1);
        let mut declaration = fixture_declaration();
        declaration.actor_records[0].fields = vec![
            QvmModActorField { offset: 0, access: None, binding: ModActorBinding::Private { byte_length: 512 } },
            QvmModActorField { offset: 516, access: None, binding: ModActorBinding::Health { encoding: ModScalar::Int32 } },
        ];
        let mut host = FakeHost::new();
        host.live.insert(actor.clone());
        host.canonical.insert(actor.clone(), 100.0);
        let mut provider = QvmModProvider::open(fixture_artifact(), declaration, host).unwrap();
        provider.initialize().unwrap();
        let address = provider.pointer(Some(&actor), "entity").unwrap();
        provider.refresh().unwrap();
        assert_eq!(provider.host().read_i32(address + 516).unwrap(), 100);
        provider.host_mut().write_i32(address + 516, 80).unwrap();
        let call = QvmModSourceCall { entry: 0, arguments: Vec::new(), globals: Vec::new(), returns: ModReturns::Void };
        let lowered = provider.begin_call(&call, &BTreeMap::new()).unwrap();
        provider.host_mut().write_i32(address + 516, 80).unwrap();
        provider.finish_call(lowered, true).unwrap();
        assert_eq!(provider.host().canonical.get(&actor), Some(&80.0));
    }

    #[test]
    fn pickup_context_restores_borrowed_words() {
        let owner = IdentityOwner::create("test").unwrap();
        let recipient = owner.actor(1, 1);
        let pickup = owner.actor(2, 1);
        let mut host = FakeHost::new();
        host.live.insert(recipient.clone());
        host.live.insert(pickup.clone());
        let mut provider = QvmModProvider::open(fixture_artifact(), fixture_declaration(), host).unwrap();
        provider.initialize().unwrap();
        let rule = QvmModPickup {
            id: "rule".to_string(),
            writes: vec![PickupWrite::Inventory { item: "test:ammo".to_string(), fields: InventoryWriteFields::Count }],
            offered: vec!["test:ammo".to_string()],
            operation: OriginalPickupOperation::BooleanGrant {
                grant: QvmModSourceCall { entry: 0, arguments: Vec::new(), globals: Vec::new(), returns: ModReturns::Int32 },
            },
            context: vec![PickupContextField { record: "entity".to_string(), offset: 8, value: QvmModValue::Int32(ModCallbackValue::Float(7.0)) }],
        };
        let offer = PickupOffer { recipient: recipient.clone(), pickup: pickup.clone(), item: "test:ammo".to_string() };
        let address = provider.pointer(Some(&pickup), "entity").unwrap() + 8;
        provider
            .pickup_context(&rule, &offer, &BTreeMap::new(), |provider| {
                assert_eq!(provider.host().read_i32(address).unwrap(), 7);
                Ok(())
            })
            .unwrap();
        assert_eq!(provider.host().read_i32(address).unwrap(), 0);
    }

    #[test]
    fn host_state_round_trips() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut host = FakeHost::new();
        host.live.insert(actor.clone());
        let mut provider = QvmModProvider::open(fixture_artifact(), fixture_declaration(), host).unwrap();
        provider.initialize().unwrap();
        provider.pointer(Some(&actor), "entity").unwrap();
        let saved = provider.capture_host_state().unwrap();
        let mut host = FakeHost::new();
        host.live.insert(actor.clone());
        let mut restored = QvmModProvider::open(fixture_artifact(), fixture_declaration(), host).unwrap();
        restored.initialize().unwrap();
        restored.restore_host_state(&saved, &vec![0u8; 8192], &|saved| (saved.slot == 1).then(|| actor.clone())).unwrap();
        assert_eq!(restored.projection_slot(&actor), Some(0));
    }

    #[test]
    fn checkpoint_validation_checks_api_identity() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut host = FakeHost::new();
        host.live.insert(actor.clone());
        let mut provider = QvmModProvider::open(fixture_artifact(), fixture_declaration(), host).unwrap();
        provider.initialize().unwrap();
        provider.pointer(Some(&actor), "entity").unwrap();
        let saved = provider.capture_host_state().unwrap();
        let data = vec![0u8; 8192];
        let artifact = fixture_artifact();
        let declaration = fixture_declaration();
        let view = QvmModCheckpointView {
            module: &artifact.module,
            data: &data,
            api_kind: "q3-qagame",
            api_version: 8,
            abi: QvmAbi::Modern,
            instruction_index: 0,
            operand_stack_len: 0,
            program_stack: 8192,
            host_module: &artifact.module,
            host_format: "qvm:mod-host-v1",
            host: &saved,
            random_len: 0,
            callbacks_len: 0,
        };
        validate_qvm_mod_checkpoint(&artifact, &declaration, &view).unwrap();
        let bad = QvmModCheckpointView { api_version: 7, ..view };
        assert!(validate_qvm_mod_checkpoint(&artifact, &declaration, &bad).is_err());
    }

    #[test]
    fn objective_addresses_validate_and_resolve() {
        validate_qvm_objective_address(4096, &QvmModObjectiveAddress::Direct(64)).unwrap();
        assert!(validate_qvm_objective_address(4096, &QvmModObjectiveAddress::Direct(65)).is_err());
        let resolved = resolve_qvm_objective_address(&|address| Ok(address as i32 + 4), 4096, &QvmModObjectiveAddress::Direct(64)).unwrap();
        assert_eq!(resolved, 64);
        let chained = QvmModObjectiveAddress::Pointer { address: 64, indirections: Vec::new(), offset: 8 };
        assert_eq!(resolve_qvm_objective_address(&|_| Ok(100), 4096, &chained).unwrap(), 108);
    }

    #[test]
    fn bootstrap_resolves_constant_stores() {
        let image = QvmImage {
            instructions: vec![
                QvmInstruction::word(QvmOpcode::OpConst, 128),
                QvmInstruction::word(QvmOpcode::OpConst, 5),
                QvmInstruction::word(QvmOpcode::OpStore4, 0),
            ],
            data_length: 4096,
            literal_length: 0,
            bss_length: 0,
            initialized_length: 4096,
            allocated_data_length: 8192,
        };
        assert_eq!(qvm_actor_bootstrap(&[2], &image, &[]).unwrap(), vec![(128, 5)]);
        assert!(qvm_actor_bootstrap(&[1], &image, &[]).is_err());
    }
}




