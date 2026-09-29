//! QVM syscall classification and intrinsic dispatch.
//!
//! Port of `src/compat/qvm/syscalls.ts` (`QvmRole`, `QvmHostCall`, `QvmHost`,
//! `QvmUnboundSyscallError`, `rejectQvmSyscall`, `createQvmSystemCall`).
//!
//! Intrinsics (memory/math/vector/snap) execute here; every engine-owned
//! operation reaches the explicit host. Unknown trap words classify as
//! extensions rather than errors.
//!
//! Local mirrors (owned by other workers; see the return notes):
//!
//! - `QvmAbiProfile` mirrors `QvmAbiProfile` from the donor execution
//!   contracts (`"q3-modern"` / `"q3-1.16n-base"`).
//! - `decode_legacy_qvm_game_import` mirrors `decodeLegacyQvmGameImport` from
//!   `src/compat/qvm/legacy-bot-abi.ts` (owned by the legacy-ABI worker),
//!   including its full trap table, so legacy classification works standalone.
//!
//! Sync-port note: the donor host returns `number | Promise<number>`; here
//! `QvmHost::handle_syscall` is `&self` and returns `Result` directly (shared
//! so recursive guest calls reenter the host).

use std::ops::{Deref, DerefMut};

use crate::error::GuestError;

use super::abi::{
    decode_qvm_cgame_import, decode_qvm_game_import, decode_qvm_ui_import, QvmCgameImport,
    QvmGameImport, QvmUiImport,
};
use super::interpreter::{QvmSyscall, QvmSystemCallHandler};
use super::math_syscalls::qvm_math_syscall;
use super::memory::QvmMemory;
use super::memory_syscalls::qvm_memory_syscall;
use super::snap_vector_syscalls::qvm_snap_vector_syscall;
use super::vector_syscalls::qvm_vector_syscall;

/// QVM module role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmRole {
    /// Server game.
    Qagame,
    /// Client game.
    Cgame,
    /// User interface.
    Ui,
}

impl QvmRole {
    /// Donor `"qagame"` / `"cgame"` / `"ui"` spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Qagame => "qagame",
            Self::Cgame => "cgame",
            Self::Ui => "ui",
        }
    }

    /// Intrinsic role naming (`"game"` for the server game).
    #[must_use]
    pub fn syscall_role(self) -> QvmSyscallRole {
        match self {
            Self::Qagame => QvmSyscallRole::Game,
            Self::Cgame => QvmSyscallRole::Cgame,
            Self::Ui => QvmSyscallRole::Ui,
        }
    }
}

/// Role naming used by the intrinsic handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmSyscallRole {
    /// Server game (`"game"` in the donor intrinsics).
    Game,
    /// Client game.
    Cgame,
    /// User interface.
    Ui,
}

/// QVM ABI profile: modern 1.32b or legacy 1.16n-base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmAbiProfile {
    /// Modern 1.32b ABI.
    Modern,
    /// Legacy 1.16n-base ABI.
    Legacy116n,
}

impl QvmAbiProfile {
    /// Donor `"q3-modern"` / `"q3-1.16n-base"` spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Modern => "q3-modern",
            Self::Legacy116n => "q3-1.16n-base",
        }
    }

    /// Parse a donor spelling.
    pub fn parse(text: &str) -> Result<Self, GuestError> {
        match text {
            "q3-modern" => Ok(Self::Modern),
            "q3-1.16n-base" => Ok(Self::Legacy116n),
            other => Err(GuestError::invalid(format!("Unknown QVM ABI profile {other}"))),
        }
    }
}

impl Default for QvmAbiProfile {
    fn default() -> Self {
        Self::Modern
    }
}

/// Decoded engine trap code for one role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmTrapCode {
    /// Server-game import.
    Game(QvmGameImport),
    /// Client-game import.
    Cgame(QvmCgameImport),
    /// UI import.
    Ui(QvmUiImport),
}

/// Host call classification: an engine trap or a raw extension number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmHostCode {
    /// Decoded engine trap.
    Engine(QvmTrapCode),
    /// Unclassified extension trap word.
    Extension(i32),
}

/// Classified host call: the live syscall plus role, code, ABI, and argv.
///
/// Borrows the interpreter-owned frame so dispatch needs no move.
pub struct QvmHostCall<'x, 'a, 'c> {
    /// Live syscall frame (words, memory, recursive entry).
    pub call: &'x mut QvmSyscall<'a, 'c>,
    /// Calling module role.
    pub role: QvmRole,
    /// Classified trap code.
    pub code: QvmHostCode,
    /// Active ABI profile.
    pub abi_profile: QvmAbiProfile,
    /// Command arguments captured by the module, if any.
    pub command_arguments: Option<Vec<String>>,
}

impl<'x, 'a, 'c> Deref for QvmHostCall<'x, 'a, 'c> {
    type Target = QvmSyscall<'a, 'c>;

    fn deref(&self) -> &Self::Target {
        self.call
    }
}

impl<'x, 'a, 'c> DerefMut for QvmHostCall<'x, 'a, 'c> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.call
    }
}

impl<'x, 'a, 'c> std::fmt::Debug for QvmHostCall<'x, 'a, 'c> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmHostCall")
            .field("role", &self.role)
            .field("code", &self.code)
            .field("abi_profile", &self.abi_profile)
            .finish_non_exhaustive()
    }
}

/// Engine host: handles every non-intrinsic trap synchronously.
pub trait QvmHost {
    /// Handle a classified call.
    fn handle_syscall(&self, call: &mut QvmHostCall<'_, '_, '_>) -> Result<i32, GuestError>;
}

impl<F> QvmHost for F
where
    F: for<'x, 'a, 'c> Fn(&mut QvmHostCall<'x, 'a, 'c>) -> Result<i32, GuestError>,
{
    fn handle_syscall(&self, call: &mut QvmHostCall<'_, '_, '_>) -> Result<i32, GuestError> {
        self(call)
    }
}

impl QvmHost for Box<dyn QvmHost + '_> {
    fn handle_syscall(&self, call: &mut QvmHostCall<'_, '_, '_>) -> Result<i32, GuestError> {
        (**self).handle_syscall(call)
    }
}

/// Error for a trap no host implements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmUnboundSyscall {
    /// Calling module role.
    pub role: QvmRole,
    /// Raw trap word.
    pub code: i32,
}

impl std::fmt::Display for QvmUnboundSyscall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Unbound {} QVM syscall {}", self.role.as_str(), self.code)
    }
}

impl QvmUnboundSyscall {
    /// Convert into the callback error.
    #[must_use]
    pub fn into_error(self) -> GuestError {
        GuestError::callback(self.to_string())
    }
}

/// Reject a call no host implements.
pub fn reject_qvm_syscall(call: &QvmHostCall<'_, '_, '_>) -> Result<i32, GuestError> {
    let code = match call.code {
        QvmHostCode::Engine(QvmTrapCode::Game(code)) => code as i32,
        QvmHostCode::Engine(QvmTrapCode::Cgame(code)) => code as i32,
        QvmHostCode::Engine(QvmTrapCode::Ui(code)) => code as i32,
        QvmHostCode::Extension(word) => word,
    };
    Err(QvmUnboundSyscall { role: call.role, code }.into_error())
}

/// Legacy 1.16n/1.17 game imports: words 0-40 decode modern, the rest map
/// through the legacy bot table. (Local mirror of `legacy-bot-abi.ts`.)
#[must_use]
pub fn decode_legacy_qvm_game_import(word: i32) -> Option<QvmGameImport> {
    use QvmGameImport as G;
    if (0..=40).contains(&word) {
        return decode_qvm_game_import(word);
    }
    match word {
        200 => Some(G::BotlibSetup),
        201 => Some(G::BotlibShutdown),
        202 => Some(G::BotlibLibvarSet),
        203 => Some(G::BotlibLibvarGet),
        204 => Some(G::BotlibPcAddGlobalDefine),
        205 => Some(G::BotlibStartFrame),
        206 => Some(G::BotlibLoadMap),
        207 => Some(G::BotlibUpdatentity),
        208 => Some(G::BotlibTest),
        209 => Some(G::BotlibGetSnapshotEntity),
        210 => Some(G::BotlibGetConsoleMessage),
        211 => Some(G::BotlibUserCommand),
        303 => Some(G::BotlibAasEntityInfo),
        304 => Some(G::BotlibAasInitialized),
        305 => Some(G::BotlibAasPresenceTypeBoundingBox),
        306 => Some(G::BotlibAasTime),
        307 => Some(G::BotlibAasPointAreaNum),
        308 => Some(G::BotlibAasTraceAreas),
        309 => Some(G::BotlibAasPointContents),
        310 => Some(G::BotlibAasNextBspEntity),
        311 => Some(G::BotlibAasValueForBspEpairKey),
        312 => Some(G::BotlibAasVectorForBspEpairKey),
        313 => Some(G::BotlibAasFloatForBspEpairKey),
        314 => Some(G::BotlibAasIntForBspEpairKey),
        315 => Some(G::BotlibAasAreaReachability),
        316 => Some(G::BotlibAasAreaTravelTimeToGoalArea),
        317 => Some(G::BotlibAasSwimming),
        318 => Some(G::BotlibAasPredictClientMovement),
        400 => Some(G::BotlibEaSay),
        401 => Some(G::BotlibEaSayTeam),
        406 => Some(G::BotlibEaGesture),
        407 => Some(G::BotlibEaCommand),
        408 => Some(G::BotlibEaSelectWeapon),
        409 => Some(G::BotlibEaTalk),
        410 => Some(G::BotlibEaAttack),
        411 => Some(G::BotlibEaUse),
        412 => Some(G::BotlibEaRespawn),
        413 => Some(G::BotlibEaJump),
        414 => Some(G::BotlibEaDelayedJump),
        415 => Some(G::BotlibEaCrouch),
        416 => Some(G::BotlibEaMoveUp),
        417 => Some(G::BotlibEaMoveDown),
        418 => Some(G::BotlibEaMoveForward),
        419 => Some(G::BotlibEaMoveBack),
        420 => Some(G::BotlibEaMoveLeft),
        421 => Some(G::BotlibEaMoveRight),
        422 => Some(G::BotlibEaMove),
        423 => Some(G::BotlibEaView),
        424 => Some(G::BotlibEaEndRegular),
        425 => Some(G::BotlibEaGetInput),
        426 => Some(G::BotlibEaResetInput),
        500 => Some(G::BotlibAiLoadCharacter),
        501 => Some(G::BotlibAiFreeCharacter),
        502 => Some(G::BotlibAiCharacteristicFloat),
        503 => Some(G::BotlibAiCharacteristicBfloat),
        504 => Some(G::BotlibAiCharacteristicInteger),
        505 => Some(G::BotlibAiCharacteristicBinteger),
        506 => Some(G::BotlibAiCharacteristicString),
        507 => Some(G::BotlibAiAllocChatState),
        508 => Some(G::BotlibAiFreeChatState),
        509 => Some(G::BotlibAiQueueConsoleMessage),
        510 => Some(G::BotlibAiRemoveConsoleMessage),
        511 => Some(G::BotlibAiNextConsoleMessage),
        512 => Some(G::BotlibAiNumConsoleMessage),
        513 => Some(G::BotlibAiInitialChat),
        514 => Some(G::BotlibAiReplyChat),
        515 => Some(G::BotlibAiChatLength),
        516 => Some(G::BotlibAiEnterChat),
        517 => Some(G::BotlibAiStringContains),
        518 => Some(G::BotlibAiFindMatch),
        519 => Some(G::BotlibAiMatchVariable),
        520 => Some(G::BotlibAiUnifyWhiteSpaces),
        521 => Some(G::BotlibAiReplaceSynonyms),
        522 => Some(G::BotlibAiLoadChatFile),
        523 => Some(G::BotlibAiSetChatGender),
        524 => Some(G::BotlibAiSetChatName),
        525 => Some(G::BotlibAiResetGoalState),
        526 => Some(G::BotlibAiResetAvoidGoals),
        527 => Some(G::BotlibAiPushGoal),
        528 => Some(G::BotlibAiPopGoal),
        529 => Some(G::BotlibAiEmptyGoalStack),
        530 => Some(G::BotlibAiDumpAvoidGoals),
        531 => Some(G::BotlibAiDumpGoalStack),
        532 => Some(G::BotlibAiGoalName),
        533 => Some(G::BotlibAiGetTopGoal),
        534 => Some(G::BotlibAiGetSecondGoal),
        535 => Some(G::BotlibAiChooseLtgItem),
        536 => Some(G::BotlibAiChooseNbgItem),
        537 => Some(G::BotlibAiTouchingGoal),
        538 => Some(G::BotlibAiItemGoalInVisButNotVisible),
        539 => Some(G::BotlibAiGetLevelItemGoal),
        540 => Some(G::BotlibAiAvoidGoalTime),
        541 => Some(G::BotlibAiInitLevelItems),
        542 => Some(G::BotlibAiUpdateEntityItems),
        543 => Some(G::BotlibAiLoadItemWeights),
        544 => Some(G::BotlibAiFreeItemWeights),
        545 => Some(G::BotlibAiSaveGoalFuzzyLogic),
        546 => Some(G::BotlibAiAllocGoalState),
        547 => Some(G::BotlibAiFreeGoalState),
        548 => Some(G::BotlibAiResetMoveState),
        549 => Some(G::BotlibAiMoveToGoal),
        550 => Some(G::BotlibAiMoveInDirection),
        551 => Some(G::BotlibAiResetAvoidReach),
        552 => Some(G::BotlibAiResetLastAvoidReach),
        553 => Some(G::BotlibAiReachabilityArea),
        554 => Some(G::BotlibAiMovementViewTarget),
        555 => Some(G::BotlibAiAllocMoveState),
        556 => Some(G::BotlibAiFreeMoveState),
        557 => Some(G::BotlibAiInitMoveState),
        558 => Some(G::BotlibAiChooseBestFightWeapon),
        559 => Some(G::BotlibAiGetWeaponInfo),
        560 => Some(G::BotlibAiLoadWeaponWeights),
        561 => Some(G::BotlibAiAllocWeaponState),
        562 => Some(G::BotlibAiFreeWeaponState),
        563 => Some(G::BotlibAiResetWeaponState),
        564 => Some(G::BotlibAiGeneticParentsAndChildSelection),
        565 => Some(G::BotlibAiInterbreedGoalFuzzyLogic),
        566 => Some(G::BotlibAiMutateGoalFuzzyLogic),
        567 => Some(G::BotlibAiGetNextCampSpotGoal),
        568 => Some(G::BotlibAiGetMapLocationGoal),
        569 => Some(G::BotlibAiNumInitialChats),
        570 => Some(G::BotlibAiGetChatMessage),
        571 => Some(G::BotlibAiRemoveFromAvoidGoals),
        572 => Some(G::BotlibAiPredictVisiblePosition),
        _ => None,
    }
}

fn classify<'x, 'a, 'c>(
    role: QvmRole,
    call: &'x mut QvmSyscall<'a, 'c>,
    command_arguments: Option<Vec<String>>,
    abi_profile: QvmAbiProfile,
) -> Result<QvmHostCall<'x, 'a, 'c>, GuestError> {
    let word = call.words.get_i32(0)?;
    let engine = match role {
        QvmRole::Qagame => {
            let code = if abi_profile == QvmAbiProfile::Modern {
                decode_qvm_game_import(word)
            } else {
                decode_legacy_qvm_game_import(word)
            };
            code.map(QvmTrapCode::Game)
        }
        QvmRole::Cgame => decode_qvm_cgame_import(word).map(QvmTrapCode::Cgame),
        QvmRole::Ui => {
            if abi_profile == QvmAbiProfile::Modern {
                decode_qvm_ui_import(word).map(QvmTrapCode::Ui)
            } else if (46..=49).contains(&word) {
                None
            } else if (50..=58).contains(&word) {
                decode_qvm_ui_import(word - 4).map(QvmTrapCode::Ui)
            } else {
                decode_qvm_ui_import(word).map(QvmTrapCode::Ui)
            }
        }
    };
    Ok(QvmHostCall {
        call,
        role,
        code: engine.map_or(QvmHostCode::Extension(word), QvmHostCode::Engine),
        abi_profile,
        command_arguments,
    })
}

fn legacy_supported(role: QvmRole, trap: i32) -> bool {
    if role == QvmRole::Qagame {
        (0..=40).contains(&trap)
            || (100..=106).contains(&trap)
            || trap == 110
            || trap == 111
            || decode_legacy_qvm_game_import(trap).is_some()
            || (402..=405).contains(&trap)
    } else {
        (0..=58).contains(&trap)
            || (100..=106).contains(&trap)
            || (role == QvmRole::Cgame && (trap == 107 || trap == 108))
            || (role == QvmRole::Ui && (trap == 110 || trap == 111))
    }
}

/// Bound system-call dispatcher: intrinsics first, then the explicit host.
///
/// `command_arguments` supplies the module's captured argv on demand.
pub struct QvmSystemCall<H: QvmHost> {
    role: QvmRole,
    host: H,
    command_arguments: Box<dyn Fn() -> Option<Vec<String>>>,
    abi_profile: QvmAbiProfile,
}

impl<H: QvmHost> std::fmt::Debug for QvmSystemCall<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmSystemCall")
            .field("role", &self.role)
            .field("abi_profile", &self.abi_profile)
            .finish_non_exhaustive()
    }
}

/// Build the dispatcher for `role` over `host`.
pub fn create_qvm_system_call<H: QvmHost>(
    role: QvmRole,
    host: H,
    command_arguments: Box<dyn Fn() -> Option<Vec<String>>>,
    abi_profile: QvmAbiProfile,
) -> QvmSystemCall<H> {
    QvmSystemCall { role, host, command_arguments, abi_profile }
}

impl<H: QvmHost> QvmSystemCall<H> {
    /// Dispatch one trap: intrinsics first, then the explicit host.
    pub fn dispatch(&self, call: &mut QvmSyscall<'_, '_>) -> Result<i32, GuestError> {
        if self.abi_profile != QvmAbiProfile::Modern {
            let trap = call.words.get_i32(0)?;
            if !legacy_supported(self.role, trap) {
                return Err(GuestError::invalid(format!(
                    "Legacy QVM ABI service {}/{} is not implemented",
                    self.role.as_str(),
                    trap
                )));
            }
        }
        let guest: QvmMemory = call.guest.clone();
        let source_role = self.role.syscall_role();
        let math_role =
            if self.abi_profile != QvmAbiProfile::Modern && self.role == QvmRole::Ui {
                QvmSyscallRole::Game
            } else {
                source_role
            };
        if let Some(value) = qvm_memory_syscall(source_role, &call.words, &guest)? {
            return Ok(value);
        }
        if let Some(value) = qvm_math_syscall(math_role, &call.words)? {
            return Ok(value);
        }
        if let Some(value) = qvm_vector_syscall(source_role, &call.words, &guest)? {
            return Ok(value);
        }
        if let Some(value) = qvm_snap_vector_syscall(source_role, &call.words, &guest)? {
            return Ok(value);
        }
        let mut classified = classify(self.role, call, (self.command_arguments)(), self.abi_profile)?;
        self.host.handle_syscall(&mut classified)
    }
}

impl<H: QvmHost> QvmSystemCallHandler for QvmSystemCall<H> {
    fn handle_syscall(&self, call: &mut QvmSyscall<'_, '_>) -> Result<i32, GuestError> {
        self.dispatch(call)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::super::allocation::QvmAllocationProfile;
    use super::super::image::{QvmImage, QvmInstruction, QvmOpcode, QvmOperand};
    use super::super::interpreter::{QvmInterpreter, QvmSemantics};

    fn image(program: Vec<(QvmOpcode, QvmOperand)>) -> QvmImage {
        let mut offset = 0usize;
        let instructions = program
            .into_iter()
            .map(|(opcode, operand)| {
                let instruction = QvmInstruction { byte_offset: offset, opcode, operand };
                offset += 1 + opcode.operand_width();
                instruction
            })
            .collect();
        QvmImage {
            source: "test".to_string(),
            instructions,
            code_offset: 0,
            code_length: offset,
            data_length: 256,
            literal_length: 0,
            bss_length: 0,
            initialized_data: vec![0; 256],
            allocated_data_length: 256,
            data_mask: 255,
        }
    }

    #[test]
    fn intrinsics_run_before_the_host() {
        use QvmOpcode as O;
        let mut vm = QvmInterpreter::new(
            image(vec![
                (O::OpEnter, QvmOperand::Word(8)),
                (O::OpConst, QvmOperand::Word(64)),
                (O::OpArg, QvmOperand::Byte(8)),
                (O::OpConst, QvmOperand::Word(0x41)),
                (O::OpArg, QvmOperand::Byte(12)),
                (O::OpConst, QvmOperand::Word(4)),
                (O::OpArg, QvmOperand::Byte(16)),
                (O::OpConst, QvmOperand::Word(-101)),
                (O::OpCall, QvmOperand::None),
                (O::OpLeave, QvmOperand::Word(8)),
            ]),
            QvmAllocationProfile::Unaccounted,
            None,
            QvmSemantics::Interpreted,
        )
        .unwrap();
        let reached = Rc::new(RefCell::new(false));
        let reached_clone = Rc::clone(&reached);
        let system = create_qvm_system_call(
            QvmRole::Qagame,
            move |_call: &mut QvmHostCall<'_, '_, '_>| -> Result<i32, GuestError> {
                *reached_clone.borrow_mut() = true;
                Ok(0)
            },
            Box::new(|| None),
            QvmAbiProfile::Modern,
        );
        assert_eq!(vm.invoke(&system, &[0; 10], 0, None).unwrap(), 0);
        assert_eq!(vm.memory().read_bytes(64, 4).unwrap(), vec![0x41; 4]);
        assert!(!*reached.borrow());
    }

    #[test]
    fn engine_traps_reach_the_host_classified() {
        use QvmOpcode as O;
        let mut vm = QvmInterpreter::new(
            image(vec![
                (O::OpEnter, QvmOperand::Word(8)),
                (O::OpConst, QvmOperand::Word(64)),
                (O::OpArg, QvmOperand::Byte(8)),
                (O::OpConst, QvmOperand::Word(-1)),
                (O::OpCall, QvmOperand::None),
                (O::OpLeave, QvmOperand::Word(8)),
            ]),
            QvmAllocationProfile::Unaccounted,
            None,
            QvmSemantics::Interpreted,
        )
        .unwrap();
        vm.memory().write_bytes(64, b"hi\0").unwrap();
        let got = Rc::new(RefCell::new(String::new()));
        let got_clone = Rc::clone(&got);
        let system = create_qvm_system_call(
            QvmRole::Qagame,
            move |call: &mut QvmHostCall<'_, '_, '_>| -> Result<i32, GuestError> {
                assert_eq!(call.code, QvmHostCode::Engine(QvmTrapCode::Game(QvmGameImport::GPrint)));
                assert_eq!(call.role, QvmRole::Qagame);
                *got_clone.borrow_mut() = call.guest.read_string(call.words.get_i32(4)?)?;
                Ok(3)
            },
            Box::new(|| Some(vec!["prog".to_string()])),
            QvmAbiProfile::Modern,
        );
        assert_eq!(vm.invoke(&system, &[0; 10], 0, None).unwrap(), 3);
        assert_eq!(*got.borrow(), "hi");
    }

    #[test]
    fn unknown_traps_classify_as_extensions() {
        use QvmOpcode as O;
        let mut vm = QvmInterpreter::new(
            image(vec![
                (O::OpEnter, QvmOperand::Word(8)),
                (O::OpConst, QvmOperand::Word(-1000)),
                (O::OpCall, QvmOperand::None),
                (O::OpLeave, QvmOperand::Word(8)),
            ]),
            QvmAllocationProfile::Unaccounted,
            None,
            QvmSemantics::Interpreted,
        )
        .unwrap();
        let system = create_qvm_system_call(
            QvmRole::Cgame,
            |call: &mut QvmHostCall<'_, '_, '_>| -> Result<i32, GuestError> {
                assert_eq!(call.code, QvmHostCode::Extension(999));
                reject_qvm_syscall(call)
            },
            Box::new(|| None),
            QvmAbiProfile::Modern,
        );
        let error = vm.invoke(&system, &[0; 10], 0, None).unwrap_err();
        assert_eq!(error.to_string(), "Unbound cgame QVM syscall 999");
    }

    #[test]
    fn legacy_profile_rejects_unimplemented_services() {
        use QvmOpcode as O;
        let mut vm = QvmInterpreter::new(
            image(vec![
                (O::OpEnter, QvmOperand::Word(8)),
                (O::OpConst, QvmOperand::Word(-100)),
                (O::OpCall, QvmOperand::None),
                (O::OpLeave, QvmOperand::Word(8)),
            ]),
            QvmAllocationProfile::Unaccounted,
            None,
            QvmSemantics::Interpreted,
        )
        .unwrap();
        let system = create_qvm_system_call(
            QvmRole::Qagame,
            |_call: &mut QvmHostCall<'_, '_, '_>| -> Result<i32, GuestError> { Ok(0) },
            Box::new(|| None),
            QvmAbiProfile::Legacy116n,
        );
        assert!(vm.invoke(&system, &[0; 10], 0, None).is_err());
    }

    #[test]
    fn legacy_game_table_matches_donor() {
        assert_eq!(
            decode_legacy_qvm_game_import(40),
            Some(QvmGameImport::GDebugPolygonDelete)
        );
        assert_eq!(decode_legacy_qvm_game_import(41), None);
        assert_eq!(decode_legacy_qvm_game_import(407), Some(QvmGameImport::BotlibEaCommand));
        assert_eq!(decode_legacy_qvm_game_import(402), None);
        assert_eq!(decode_legacy_qvm_game_import(572), Some(QvmGameImport::BotlibAiPredictVisiblePosition));
        assert_eq!(decode_legacy_qvm_game_import(573), None);
    }

    #[test]
    fn roles_and_profiles_round_trip() {
        assert_eq!(QvmRole::Qagame.as_str(), "qagame");
        assert_eq!(QvmRole::Qagame.syscall_role(), QvmSyscallRole::Game);
        assert_eq!(QvmAbiProfile::parse("q3-modern").unwrap(), QvmAbiProfile::Modern);
        assert_eq!(QvmAbiProfile::parse("q3-1.16n-base").unwrap(), QvmAbiProfile::Legacy116n);
        assert!(QvmAbiProfile::parse("q9").is_err());
    }

    #[test]
    fn unbound_syscalls_report_role_and_code() {
        let error = QvmUnboundSyscall { role: QvmRole::Ui, code: 4242 }.into_error();
        assert_eq!(error.to_string(), "Unbound ui QVM syscall 4242");
    }
}
