//! QVM weapon behavior source: source trajectories from guest modules.
//!
//! Provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/qvm-weapon-behavior.ts`.

use std::cell::Cell;
use std::cell::RefCell;
use std::cell::RefMut;
use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;
use std::rc::Weak;

use qa_content::bsp::parse_q1_entities;
use qa_content::bsp::q1_entity_value;
use qa_content::hash::sha256_hex;
use qa_content::mounts::MountedContent;
use qa_core::cmd::Dialect;
use qa_core::cvar::CvarError;
use qa_core::cvar::CvarRegistry;
use qa_core::cvar::CvarSnapshot;
use qa_core::identity::ActorId;
use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;
use qa_guest::error::GuestError;
use qa_guest::qvm::abi::QvmGameImport;
use qa_guest::qvm::artifacts::ResolvedQvmArtifact;
use qa_guest::qvm::entity_tokens::CommonParseCursor;
use qa_guest::qvm::entity_tokens::CommonParseState;
use qa_guest::qvm::file_syscalls::FileMounts;
use qa_guest::qvm::file_syscalls::FilesCheckpoint;
use qa_guest::qvm::file_syscalls::OpenedFile;
use qa_guest::qvm::file_syscalls::QvmFiles;
use qa_guest::qvm::game::QvmGame;
use qa_guest::qvm::game_data::AbiProfile;
use qa_guest::qvm::game_data::CallKind;
use qa_guest::qvm::game_data::ProfileValue;
use qa_guest::qvm::game_data::QvmArtifact as GameArtifact;
use qa_guest::qvm::game_data::QvmCheckpoint;
use qa_guest::qvm::game_data::QvmGameDataState;
use qa_guest::qvm::game_data::QvmHostCall;
use qa_guest::qvm::game_data::QvmImage as GameImage;
use qa_guest::qvm::game_data::QvmInstruction as GameInstruction;
use qa_guest::qvm::game_data::QvmOpcode as GameOpcode;
use qa_guest::qvm::game_data::QvmRole as GameRole;
use qa_guest::qvm::image::QvmImage as ParsedImage;
use qa_guest::qvm::image::QvmInstruction as ParsedInstruction;
use qa_guest::qvm::image::QvmOpcode as ParsedOpcode;
use qa_guest::qvm::image::QvmOperand;
use qa_guest::qvm::mod_provider::ModuleId;
use qa_guest::qvm::mod_provider::QvmAbi;
use qa_guest::qvm::mod_provider::QvmArtifact as ProviderArtifact;
use qa_guest::qvm::mod_provider::QvmImage as ProviderImage;
use qa_guest::qvm::mod_provider::QvmInstruction as ProviderInstruction;
use qa_guest::qvm::mod_provider::QvmOpcode as ProviderOpcode;
use qa_guest::qvm::mod_provider::QvmRole as ProviderRole;
use qa_guest::qvm::shared_entity_record::read_qvm_shared_entity;
use qa_guest::qvm::shared_entity_record::write_qvm_shared_entity;
use qa_guest::qvm::syscalls::QvmAbiProfile as SysAbiProfile;
use qa_guest::qvm::syscalls::QvmRole as SysRole;
use qa_guest::qvm::weapon_behavior_profile::same_weapon_behavior;
use qa_guest::qvm::weapon_behavior_profile::validate_qvm_weapon_profile;
use qa_guest::qvm::weapon_behavior_profile::WeaponBehaviorCallback;
use qa_guest::qvm::weapon_behavior_profile::WeaponBehaviorDefinition;
use qa_world::body::BodyState;
use qa_world::save::records::read_saved_actor;
use qa_world::save::shared::read_vector;
use qa_world::save::value::SaveReader;
use qa_world::WorldError;

use super::quakec_weapon_behavior::WeaponBehaviorDefinitionReader;
use super::quakec_weapon_behavior::WeaponBehaviorInstance;
use super::quakec_weapon_behavior::WeaponBehaviorLaunch;
use super::quakec_weapon_behavior::WeaponBehaviorSource;
use super::quakec_weapon_behavior::WeaponTrajectoryUpdate;
use super::types::SimulationMode;

/// Canonical home of `QvmWeaponProfile` (donor
/// `src/compat/qvm/weapon-behavior-profile.ts`).
///
/// The profile shape already lives in `qa_guest`, ported from the same donor;
/// this re-export is the canonical simulation-lane path so the coordinator can
/// swap the `super::types` opaque to it without duplicating the type.
pub use qa_guest::qvm::weapon_behavior_profile::QvmWeaponProfile;

/// Mirror of `QvmWeaponTarget` from the donor module.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmWeaponTarget {
    /// Target actor.
    pub actor: ActorId,
    /// Target body.
    pub body: BodyState,
    /// Target health.
    pub health: f64,
    /// Target kind.
    pub kind: QvmWeaponTargetKind,
}

/// Mirrored target kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmWeaponTargetKind {
    /// Plain actor.
    Actor,
    /// Player with userinfo and team.
    Player {
        /// Client userinfo.
        userinfo: String,
        /// Team.
        team: QvmWeaponTeam,
    },
}

/// Player team.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmWeaponTeam {
    /// Free.
    Free,
    /// Red.
    Red,
    /// Blue.
    Blue,
    /// Spectator.
    Spectator,
}

impl QvmWeaponTeam {
    fn number(self) -> i32 {
        match self {
            Self::Free => 0,
            Self::Red => 1,
            Self::Blue => 2,
            Self::Spectator => 3,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Red => "red",
            Self::Blue => "blue",
            Self::Spectator => "spectator",
        }
    }
}

/// Live target listing.
pub type QvmWeaponTargets = Rc<dyn Fn() -> Vec<QvmWeaponTarget>>;
/// Print sink.
pub type QvmWeaponPrint = Rc<dyn Fn(&str)>;
/// Currency assertion: panics when the source is not current (donor throws).
pub type QvmWeaponAssertCurrent = Rc<dyn Fn()>;

/// Trap dispatch seam. Donor `qvmCommonSyscall`
/// (`src/compat/qvm/common-syscalls.ts`), `qvmFileSyscall`
/// (`src/compat/qvm/file-syscalls.ts`), and `qvmServerGameSyscall`
/// (`src/compat/qvm/server-game-syscalls.ts`) are sealed to other interpreter
/// stacks; the qagame-side chain has no ported home. The session injects the
/// dispatch; this module owns layout, bindings, entries, and checkpoints, and
/// keeps the donor botlib short-circuit plus unbound-trap rejection.
/// Implementations borrow trap state through short-lived guards and must not
/// reenter the source (no nested trapping calls).
pub trait QvmWeaponTraps {
    /// Handle a trap, returning `None` to decline. `source` exposes the
    /// trap-facing surface (`trap_*`, `mirror`, `slot`, `live`).
    fn dispatch(&mut self, call: &QvmHostCall, source: &QvmWeaponBehaviorSource) -> Result<Option<i32>, GuestError>;
}

/// Mirror of `QvmWeaponBehaviorOptions` from the donor module. The donor
/// `scene` and `realTime` inputs feed only the trap dispatch, so they travel
/// with the injected [`QvmWeaponTraps`] implementation instead of this struct.
pub struct QvmWeaponBehaviorOptions {
    /// Module artifact (hub type from `super::types::PreparedWeaponBehavior`).
    pub artifact: ResolvedQvmArtifact,
    /// Weapon profile (canonical [`QvmWeaponProfile`]).
    pub profile: QvmWeaponProfile,
    /// Content mounts (hub type).
    pub mounts: MountedContent,
    /// Random seed.
    pub seed: i32,
    /// Authored entity text.
    pub entity_text: String,
    /// Simulation mode.
    pub mode: SimulationMode,
    /// Team mode.
    pub team_mode: bool,
    /// Live targets.
    pub targets: QvmWeaponTargets,
    /// Print sink.
    pub print: QvmWeaponPrint,
    /// Currency assertion.
    pub assert_current: QvmWeaponAssertCurrent,
    /// Trap dispatch seam.
    pub traps: Rc<RefCell<dyn QvmWeaponTraps>>,
}

/// QVM weapon behavior failure.
#[derive(Debug, thiserror::Error)]
pub enum QvmWeaponError {
    /// Guest failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
    /// Checkpoint failure.
    #[error(transparent)]
    World(#[from] WorldError),
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
    /// Entity parse failure.
    #[error(transparent)]
    Binary(#[from] qa_core::binary::BinaryError),
    /// Native replacement artifacts cannot execute trajectories.
    #[error("QVM weapon behavior requires a bytecode artifact")]
    NativeArtifact,
    /// Entity text lacks exactly one worldspawn.
    #[error("QVM component requires one authored worldspawn")]
    Worldspawn,
    /// Spawn field cannot be quoted.
    #[error("Unquotable QVM spawn field")]
    Unquotable,
    /// Private layout differs from located data.
    #[error("QVM weapon private layout differs from located source data")]
    LayoutMismatch,
    /// Source is unready or already executing.
    #[error("QVM weapon source is unready or already executing")]
    Busy,
    /// Source time is invalid.
    #[error("Invalid QVM weapon source time")]
    BadTime,
    /// Entity pointer is noncanonical.
    #[error("Noncanonical QVM entity pointer")]
    BadPointer,
    /// Shared actor is unavailable.
    #[error("QVM weapon query refers to an unavailable shared actor")]
    MissingActor,
    /// Client capacity exhausted.
    #[error("QVM weapon source exhausted its admitted client capacity")]
    ClientCapacity,
    /// Client entity cannot use a null pointer.
    #[error("QVM client entity cannot use a null pointer")]
    NullClient,
    /// Client connection was rejected.
    #[error("QVM weapon source rejected client: {0}")]
    ClientRejected(String),
    /// Source allocated an occupied or inactive entity.
    #[error("QVM source allocated an occupied or inactive entity")]
    OccupiedAllocation,
    /// Mirrored actor changed client ownership.
    #[error("QVM mirrored actor changed client ownership")]
    ClientOwnership,
    /// Player team differs from team mode.
    #[error("QVM player team differs from selected source team mode")]
    TeamMode,
    /// Source did not admit the requested team.
    #[error("QVM source did not admit the requested {0} team")]
    TeamDenied(String),
    /// Launch role or ownership differs.
    #[error("QVM weapon launch role or ownership differs")]
    LaunchMismatch,
    /// Activation identity is invalid.
    #[error("Invalid QVM activation identity")]
    BadActivation,
    /// Firing requires a projectile direction.
    #[error("QVM source firing requires a projectile direction")]
    NoDirection,
    /// Fire identity is invalid.
    #[error("Invalid QVM fire identity")]
    BadFire,
    /// Fire did not return a new owned missile.
    #[error("QVM fire did not return a new owned missile")]
    BadMissile,
    /// Projectile already has an instance.
    #[error("QVM projectile already has an instance")]
    AlreadyAttached,
    /// No retained trajectory projectile.
    #[error("No retained QVM trajectory projectile")]
    MissingProjectile,
    /// Trajectory instance is closed.
    #[error("QVM trajectory instance is closed")]
    ClosedInstance,
    /// Think callback is invalid.
    #[error("Invalid source projectile think callback")]
    BadThink,
    /// Checkpoint requires an idle initialized source.
    #[error("QVM weapon checkpoint requires an idle initialized source")]
    CheckpointBusy,
    /// Saved profile differs from the declaration.
    #[error("Saved QVM weapon profile differs from the selected artifact declaration")]
    ProfileMismatch,
    /// Saved actor binding is invalid.
    #[error("Invalid saved QVM weapon actor binding")]
    BadBinding,
    /// Saved client ownership differs.
    #[error("Saved QVM client ownership differs")]
    ClientMismatch,
    /// Unbound saved client.
    #[error("Unbound saved QVM client")]
    UnboundClient,
    /// Duplicate saved actor.
    #[error("Duplicate saved QVM weapon actor")]
    DuplicateActor,
    /// Saved activation is invalid.
    #[error("Invalid saved weapon activation")]
    BadActivationRestore,
    /// Restore and rollback failed.
    #[error("QVM weapon restore and rollback failed: {0}; {1}")]
    RollbackFailed(String, String),
    /// Restore and cleanup failed.
    #[error("QVM weapon restore and cleanup failed: {0}; {1}")]
    CleanupFailed(String, String),
    /// Invalid host checkpoint.
    #[error("Invalid QVM weapon host checkpoint")]
    BadHost,
    /// Invalid client userinfo.
    #[error("Invalid component client userinfo")]
    BadUserinfo,
    /// Invalid saved configstring.
    #[error("Invalid saved component configstring")]
    BadConfigstring,
    /// Invalid source time.
    #[error("invalid source time")]
    BadSourceTime,
    /// Unbound trap.
    #[error("Unbound {0} QVM syscall {1}")]
    UnboundTrap(String, i32),
}
macro_rules! opcode_converters {
    ($($variant:ident,)*) => {
        fn to_game_opcode(opcode: ParsedOpcode) -> GameOpcode {
            match opcode {
                $(ParsedOpcode::$variant => GameOpcode::$variant,)*
            }
        }

        fn to_provider_opcode(opcode: ParsedOpcode) -> ProviderOpcode {
            match opcode {
                $(ParsedOpcode::$variant => ProviderOpcode::$variant,)*
            }
        }
    };
}

opcode_converters!(
    OpUndef,
    OpIgnore,
    OpBreak,
    OpEnter,
    OpLeave,
    OpCall,
    OpPush,
    OpPop,
    OpConst,
    OpLocal,
    OpJump,
    OpEq,
    OpNe,
    OpLti,
    OpLei,
    OpGti,
    OpGei,
    OpLtu,
    OpLeu,
    OpGtu,
    OpGeu,
    OpEqf,
    OpNef,
    OpLtf,
    OpLef,
    OpGtf,
    OpGef,
    OpLoad1,
    OpLoad2,
    OpLoad4,
    OpStore1,
    OpStore2,
    OpStore4,
    OpArg,
    OpBlockCopy,
    OpSex8,
    OpSex16,
    OpNegi,
    OpAdd,
    OpSub,
    OpDivi,
    OpDivu,
    OpModi,
    OpModu,
    OpMuli,
    OpMulu,
    OpBand,
    OpBor,
    OpBxor,
    OpBcom,
    OpLsh,
    OpRshi,
    OpRshu,
    OpNegf,
    OpAddf,
    OpSubf,
    OpDivf,
    OpMulf,
    OpCvif,
    OpCvfi,
);

fn operand_parts(operand: &QvmOperand) -> (i32, u8) {
    match operand {
        QvmOperand::None => (0, 0),
        QvmOperand::Word(word) => (*word, 4),
        QvmOperand::Byte(byte) => (i32::from(*byte), 1),
    }
}

fn convert_instruction(instruction: &ParsedInstruction) -> (GameInstruction, ProviderInstruction) {
    let (operand, operand_width) = operand_parts(&instruction.operand);
    (
        GameInstruction {
            opcode: to_game_opcode(instruction.opcode),
            operand,
            operand_width,
            byte_offset: instruction.byte_offset,
        },
        ProviderInstruction {
            opcode: to_provider_opcode(instruction.opcode),
            operand,
            operand_width,
        },
    )
}

/// Convert a parsed image to the game and provider layouts. Shared with
/// `super::qvm_mod`, which validates mod artifacts through the same
/// provider layout.
pub(crate) fn convert_image(image: &ParsedImage) -> (GameImage, ProviderImage) {
    let (game_instructions, provider_instructions): (Vec<_>, Vec<_>) =
        image.instructions.iter().map(convert_instruction).unzip();
    (
        GameImage {
            source: image.source.clone(),
            instructions: game_instructions,
            code_offset: image.code_offset,
            code_length: image.code_length,
            data_length: image.data_length,
            literal_length: image.literal_length,
            bss_length: image.bss_length,
            initialized_data: image.initialized_data.clone(),
            allocated_data_length: image.allocated_data_length,
            data_mask: image.data_mask as usize,
        },
        ProviderImage {
            instructions: provider_instructions,
            data_length: image.data_length,
            literal_length: image.literal_length,
            bss_length: image.bss_length,
            initialized_length: image.initialized_data.len(),
            allocated_data_length: image.allocated_data_length,
        },
    )
}

pub(crate) fn convert_artifact(
    resolved: &ResolvedQvmArtifact,
) -> Result<(GameArtifact, ProviderArtifact), QvmWeaponError> {
    let ResolvedQvmArtifact::Bytecode {
        module,
        role,
        image,
        abi_profile,
        ..
    } = resolved
    else {
        return Err(QvmWeaponError::NativeArtifact);
    };
    let id = format!("{}:{}", module.id.namespace, module.id.name);
    let digest = format!("{}:{}", module.digest.algorithm, module.digest.value);
    let (game_image, provider_image) = convert_image(image);
    let game_role = match role {
        SysRole::Qagame => GameRole::Qagame,
        SysRole::Cgame => GameRole::Cgame,
        SysRole::Ui => GameRole::Ui,
    };
    let provider_role = match role {
        SysRole::Qagame => ProviderRole::Qagame,
        SysRole::Cgame => ProviderRole::Cgame,
        SysRole::Ui => ProviderRole::Ui,
    };
    let (game_abi, provider_abi) = match abi_profile {
        SysAbiProfile::Modern => (AbiProfile::Modern, QvmAbi::Modern),
        SysAbiProfile::Legacy116n => (AbiProfile::Legacy, QvmAbi::Legacy),
    };
    Ok((
        GameArtifact {
            module: qa_guest::qvm::game_data::ModuleIdentity {
                id: id.clone(),
                artifact_path: module.artifact_path.clone(),
                digest: digest.clone(),
                revision: module.revision.clone(),
            },
            role: game_role,
            abi_profile: Some(game_abi),
            image: game_image,
        },
        ProviderArtifact {
            module: ModuleId {
                id,
                artifact_path: module.artifact_path.clone(),
                digest,
                revision: module.revision.clone(),
            },
            role: provider_role,
            abi_profile: Some(provider_abi),
            image: provider_image,
        },
    ))
}

struct MountsAdapter {
    mounts: Rc<MountedContent>,
}

impl FileMounts for MountsAdapter {
    fn open(&mut self, path: &str) -> Option<OpenedFile> {
        let found = self.mounts.open(path, |_| true).ok()??;
        Some(OpenedFile {
            bytes: found.bytes,
            pk3: matches!(
                found.reference.provenance,
                qa_content::contract::ResourceProvenance::Archive { .. }
            ),
        })
    }

    fn list_files(&mut self, path: &str, extension: &str) -> Vec<String> {
        self.mounts.list_files(path, extension).unwrap_or_default()
    }
}

/// Saved console-variable state. The registry exposes values, order, and
/// effect flags; per-variable modification counters restart on restore.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmCvarSaveState {
    /// Dialect tag.
    pub dialect: String,
    /// Variable snapshots, newest first.
    pub variables: Vec<CvarSnapshot>,
    /// Canonical order (oldest first).
    pub order: Vec<String>,
    /// Modified flags word.
    pub changed_flags: u32,
    /// Userinfo dirty flag.
    pub userinfo_dirty: bool,
}

fn capture_cvar_state(cvars: &mut CvarRegistry) -> QvmCvarSaveState {
    let variables = cvars.snapshots(0);
    let mut order: Vec<String> = variables.iter().map(|snapshot| snapshot.name.clone()).collect();
    order.reverse();
    let changed_flags = cvars.take_modified_flags();
    cvars.mark_modified_flags(changed_flags);
    QvmCvarSaveState {
        dialect: "q3".to_string(),
        variables,
        order,
        changed_flags,
        userinfo_dirty: cvars.userinfo_modified(),
    }
}

fn restore_cvar_state(cvars: &mut CvarRegistry, state: &QvmCvarSaveState) -> Result<(), QvmWeaponError> {
    if state.dialect != "q3" {
        return Err(QvmWeaponError::BadHost);
    }
    cvars.reset_all()?;
    for name in &state.order {
        if let Some(saved) = state.variables.iter().find(|snapshot| &snapshot.name == name) {
            cvars.register(&saved.name, &saved.reset_value, saved.flags)?;
            if saved.value != saved.reset_value {
                cvars.set(&saved.name, &saved.value, true)?;
            }
            if let Some(latched) = &saved.latched_value {
                cvars.stage(&saved.name, latched)?;
            }
        }
    }
    for saved in &state.variables {
        if cvars.get(&saved.name).is_none() {
            cvars.register(&saved.name, &saved.reset_value, saved.flags)?;
            if saved.value != saved.reset_value {
                cvars.set(&saved.name, &saved.value, true)?;
            }
            if let Some(latched) = &saved.latched_value {
                cvars.stage(&saved.name, latched)?;
            }
        }
    }
    cvars.mark_modified_flags(state.changed_flags);
    if !state.userinfo_dirty {
        cvars.clear_userinfo_modified();
    }
    Ok(())
}

/// Weapon-host checkpoint: the state the donor embeds in the module
/// checkpoint through host-state hooks. The game module takes no host-state
/// hooks, so it rides alongside the module checkpoint instead.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmWeaponHostCheckpoint {
    /// Simulation mode.
    pub mode: SimulationMode,
    /// Team mode.
    pub team_mode: bool,
    /// Source time in seconds.
    pub time: f64,
    /// Located table descriptors.
    pub data: QvmGameDataState,
    /// Console variables.
    pub cvars: QvmCvarSaveState,
    /// Configstrings.
    pub configstrings: Vec<QvmConfigstring>,
    /// Open files.
    pub files: FilesCheckpoint,
    /// Entity-text bytes (Latin-1, as owned by the cursor).
    pub entity_text: Vec<u8>,
    /// Client userinfo.
    pub userinfo: Vec<QvmUserinfo>,
    /// Entity-text cursor offset.
    pub cursor: Option<usize>,
    /// Parser state.
    pub parser: ProfileValue,
}

/// Saved configstring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmConfigstring {
    /// Index.
    pub index: i32,
    /// Value.
    pub value: String,
}

/// Saved client userinfo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmUserinfo {
    /// Slot.
    pub slot: i32,
    /// Value.
    pub value: String,
}

/// Saved actor binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmWeaponBinding {
    /// Saved actor.
    pub actor: SavedActorId,
    /// Entity pointer.
    pub pointer: i32,
    /// Binding kind.
    pub kind: QvmWeaponBindingKind,
}

/// Saved binding kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmWeaponBindingKind {
    /// Mirrored target.
    Target,
    /// Admitted client.
    Client,
    /// Owned projectile.
    Projectile,
}

/// Saved retired trajectory.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmWeaponRetired {
    /// Saved actor.
    pub actor: SavedActorId,
    /// Retained trajectory.
    pub trajectory: WeaponTrajectoryUpdate,
}

/// Mirror of `QvmWeaponBehaviorCheckpoint` from the donor module, with the
/// host state alongside the module checkpoint (see
/// [`QvmWeaponHostCheckpoint`]).
#[derive(Debug, Clone)]
pub struct QvmWeaponBehaviorCheckpoint {
    /// Checkpoint version.
    pub version: i64,
    /// Behavior definition.
    pub definition: WeaponBehaviorDefinition,
    /// Module checkpoint.
    pub module: QvmCheckpoint,
    /// Host checkpoint.
    pub host: QvmWeaponHostCheckpoint,
    /// Profile digest.
    pub profile_digest: String,
    /// Activated actors.
    pub activated: Vec<SavedActorId>,
    /// Actor bindings.
    pub bindings: Vec<QvmWeaponBinding>,
    /// Retired trajectories.
    pub retired: Vec<QvmWeaponRetired>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct QvmBinding {
    actor: ActorId,
    pointer: i32,
    kind: QvmWeaponBindingKind,
}

struct Shared {
    game_artifact: GameArtifact,
    mounts: Rc<MountedContent>,
    seed: i32,
    entity_text: String,
    mode: SimulationMode,
    team_mode: bool,
    targets: QvmWeaponTargets,
    print: QvmWeaponPrint,
    assert_current: QvmWeaponAssertCurrent,
    traps: Rc<RefCell<dyn QvmWeaponTraps>>,
    cvars: CvarRegistry,
    files: QvmFiles,
    configstrings: HashMap<i32, String>,
    bindings: HashMap<ActorId, QvmBinding>,
    retired: HashMap<ActorId, WeaponTrajectoryUpdate>,
    instances: HashSet<ActorId>,
    activated: HashSet<ActorId>,
    userinfo: HashMap<i32, String>,
    cursor: CommonParseCursor,
    parser: CommonParseState,
    time: f64,
    ready: bool,
    closed: bool,
    busy: bool,
    generation: u64,
}

/// Executes source callbacks, never a second game frame or selected
/// projectile impact.
pub struct QvmWeaponBehaviorSource {
    definition: Rc<WeaponBehaviorDefinition>,
    profile: Rc<QvmWeaponProfile>,
    game: QvmGame,
    shared: Rc<RefCell<Shared>>,
}

impl std::fmt::Debug for QvmWeaponBehaviorSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmWeaponBehaviorSource")
            .field("definition", &self.definition)
            .finish_non_exhaustive()
    }
}

fn host_call(
    game: &QvmGame,
    definition: &Rc<WeaponBehaviorDefinition>,
    profile: &Rc<QvmWeaponProfile>,
    shared: &Weak<RefCell<Shared>>,
    call: &QvmHostCall,
) -> Result<Option<i32>, GuestError> {
    let shared = shared
        .upgrade()
        .ok_or_else(|| GuestError::runtime("QVM weapon behavior source was released"))?;
    let source = QvmWeaponBehaviorSource {
        definition: Rc::clone(definition),
        profile: Rc::clone(profile),
        game: game.clone(),
        shared: Rc::clone(&shared),
    };
    source.current();
    let traps = Rc::clone(&shared.borrow().traps);
    if let Some(value) = traps.borrow_mut().dispatch(call, &source)? {
        return Ok(Some(value));
    }
    if call.kind == CallKind::Engine
        && call.role == GameRole::Qagame
        && (call.code == QvmGameImport::BotlibSetup as i32 || call.code == QvmGameImport::BotlibAasInitialized as i32)
    {
        return Ok(Some(0));
    }
    let role = match call.role {
        GameRole::Qagame => "qagame",
        GameRole::Cgame => "cgame",
        GameRole::Ui => "ui",
    };
    Err(GuestError::runtime(format!("Unbound {role} QVM syscall {}", call.code)))
}

struct Parts {
    profile: QvmWeaponProfile,
    game_artifact: GameArtifact,
    mounts: Rc<MountedContent>,
    seed: i32,
    entity_text: String,
    mode: SimulationMode,
    team_mode: bool,
    targets: QvmWeaponTargets,
    print: QvmWeaponPrint,
    assert_current: QvmWeaponAssertCurrent,
    traps: Rc<RefCell<dyn QvmWeaponTraps>>,
}

impl QvmWeaponBehaviorSource {
    /// Create an initialized source. The donor awaits module initialization;
    /// the game module initializes synchronously.
    pub fn create(options: QvmWeaponBehaviorOptions) -> Result<Self, QvmWeaponError> {
        let (game_artifact, provider_artifact) = convert_artifact(&options.artifact)?;
        let profile = validate_qvm_weapon_profile(&options.profile, &provider_artifact)?;
        Self::build(
            Parts {
                profile,
                game_artifact,
                mounts: Rc::new(options.mounts),
                seed: options.seed,
                entity_text: options.entity_text,
                mode: options.mode,
                team_mode: options.team_mode,
                targets: options.targets,
                print: options.print,
                assert_current: options.assert_current,
                traps: options.traps,
            },
            true,
        )
    }

    fn build(parts: Parts, initialize: bool) -> Result<Self, QvmWeaponError> {
        let definition = Rc::new(parts.profile.definition.clone());
        let profile = Rc::new(parts.profile);
        let cursor = CommonParseCursor::new(qvm_weapon_initialization_entities(&parts.entity_text)?.into_bytes());
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        for (name, value) in [
            ("g_log", ""),
            ("cm_noCurves", "0"),
            ("cm_playerCurveClip", "1"),
            ("bot_enable", "0"),
            ("sv_maxclients", "64"),
            ("dedicated", "1"),
            ("g_gametype", if parts.team_mode { "3" } else { "0" }),
        ] {
            cvars.register(name, value, 0)?;
        }
        let print = Rc::clone(&parts.print);
        let files = QvmFiles::new(
            Box::new(MountsAdapter {
                mounts: Rc::clone(&parts.mounts),
            }),
            None,
            move |text| print(text),
        );
        let shared = Rc::new(RefCell::new(Shared {
            game_artifact: parts.game_artifact.clone(),
            mounts: parts.mounts,
            seed: parts.seed,
            entity_text: parts.entity_text,
            mode: parts.mode,
            team_mode: parts.team_mode,
            targets: parts.targets,
            print: parts.print,
            assert_current: parts.assert_current,
            traps: parts.traps,
            cvars,
            files,
            configstrings: HashMap::new(),
            bindings: HashMap::new(),
            retired: HashMap::new(),
            instances: HashSet::new(),
            activated: HashSet::new(),
            userinfo: HashMap::new(),
            cursor,
            parser: CommonParseState::default(),
            time: 0.0,
            ready: false,
            closed: false,
            busy: false,
            generation: 0,
        }));
        let game = QvmGame::new(parts.game_artifact, Rc::new(|_| Ok(None)))?;
        game.data.set_client_count(64)?;
        let source = Self {
            definition,
            profile,
            game,
            shared,
        };
        {
            let game = source.game.clone();
            let definition = Rc::clone(&source.definition);
            let profile = Rc::clone(&source.profile);
            let host_shared = Rc::downgrade(&source.shared);
            source.game.module.set_host(Some(Rc::new(move |call| {
                host_call(&game, &definition, &profile, &host_shared, call)
            })));
        }
        if initialize {
            if let Err(error) = source.game.initialize(0, source.shared.borrow().seed, false) {
                source.close();
                return Err(error.into());
            }
            // The executing module locates its tables during initialization; the
            // recording module cannot, so layout validation waits for located
            // tables (donor: immediately after initialization).
            if source.game.data.entity_stride_bytes() != 0 {
                if let Err(error) = source.validate_layout() {
                    source.close();
                    return Err(error);
                }
            }
            source.shared.borrow_mut().ready = true;
        }
        Ok(source)
    }

    fn construct_staged(&self) -> Result<Self, QvmWeaponError> {
        let shared = self.shared.borrow();
        Self::build(
            Parts {
                profile: self.profile.as_ref().clone(),
                game_artifact: shared.game_artifact.clone(),
                mounts: Rc::clone(&shared.mounts),
                seed: shared.seed,
                entity_text: shared.entity_text.clone(),
                mode: shared.mode,
                team_mode: shared.team_mode,
                targets: Rc::clone(&shared.targets),
                print: Rc::clone(&shared.print),
                assert_current: Rc::clone(&shared.assert_current),
                traps: Rc::clone(&shared.traps),
            },
            false,
        )
    }

    fn current(&self) {
        (self.shared.borrow().assert_current)();
    }

    fn operation<T>(&self, run: impl FnOnce() -> Result<T, QvmWeaponError>) -> Result<T, QvmWeaponError> {
        self.current();
        {
            let shared = self.shared.borrow();
            if !shared.ready || shared.busy {
                return Err(QvmWeaponError::Busy);
            }
        }
        self.shared.borrow_mut().busy = true;
        let outcome = run();
        self.shared.borrow_mut().busy = false;
        outcome
    }

    fn validate_layout(&self) -> Result<(), QvmWeaponError> {
        if self.game.data.entity_stride_bytes() != self.profile.layout.entity_stride {
            return Err(QvmWeaponError::LayoutMismatch);
        }
        Ok(())
    }

    fn set_time(&self, time: f64) -> Result<(), QvmWeaponError> {
        let milliseconds = (time * 1000.0).round();
        if !time.is_finite() || time < self.shared.borrow().time || milliseconds > f64::from(i32::MAX) {
            return Err(QvmWeaponError::BadTime);
        }
        self.shared.borrow_mut().time = time;
        self.game
            .module
            .memory()
            .write_i32(self.profile.layout.level_time, milliseconds as i32)?;
        Ok(())
    }

    pub fn slot(&self, pointer: i32) -> Result<usize, QvmWeaponError> {
        let slot = self.game.data.number_from_pointer(pointer)?;
        if pointer <= 0 {
            return Err(QvmWeaponError::BadPointer);
        }
        let window = self.game.data.entity_bytes(slot)?;
        if window.offset != pointer as usize {
            return Err(QvmWeaponError::BadPointer);
        }
        Ok(slot)
    }

    pub fn live(&self, pointer: i32) -> Result<bool, QvmWeaponError> {
        let slot = self.slot(pointer)?;
        Ok(self
            .game
            .data
            .entity_bytes(slot)?
            .get_i32(self.profile.layout.fields.inuse)?
            != 0)
    }

    fn read_record(
        &self,
        pointer: i32,
    ) -> Result<(usize, qa_guest::qvm::shared_entity_record::QvmSharedEntity), QvmWeaponError> {
        let slot = self.slot(pointer)?;
        let window = self.game.data.entity_bytes(slot)?;
        let bytes = window.copy_bytes(0, window.len)?;
        let entity = read_qvm_shared_entity(&bytes, self.game.data.abi_profile())?;
        Ok((slot, entity))
    }

    fn write_record(
        &self,
        slot: usize,
        entity: &qa_guest::qvm::shared_entity_record::QvmSharedEntity,
    ) -> Result<(), QvmWeaponError> {
        let window = self.game.data.entity_bytes(slot)?;
        let mut bytes = window.copy_bytes(0, window.len)?;
        write_qvm_shared_entity(&mut bytes, entity, self.game.data.abi_profile())?;
        window.memory.write_bytes(window.offset, &bytes)?;
        Ok(())
    }

    fn project(&self, pointer: i32, body: &BodyState) -> Result<(), QvmWeaponError> {
        let (slot, mut entity) = self.read_record(pointer)?;
        entity.r.current_origin = body.origin;
        entity.r.current_angles = body.angles;
        entity.r.mins = body.bounds.min;
        entity.r.maxs = body.bounds.max;
        entity.s.origin = body.origin;
        entity.s.angles = body.angles;
        entity.s.pos.base = body.origin;
        entity.s.pos.delta = body.velocity;
        entity.s.pos.time = (self.shared.borrow().time * 1000.0).round() as i32;
        self.write_record(slot, &entity)?;
        Ok(())
    }

    fn trajectory(&self, pointer: i32) -> Result<WeaponTrajectoryUpdate, QvmWeaponError> {
        let (_, entity) = self.read_record(pointer)?;
        Ok(WeaponTrajectoryUpdate {
            origin: entity.r.current_origin,
            velocity: entity.s.pos.delta,
            angles: entity.r.current_angles,
        })
    }

    pub fn actor_for_slot(&self, slot: usize) -> Result<Option<ActorId>, QvmWeaponError> {
        for binding in self.shared.borrow().bindings.values() {
            if self.slot(binding.pointer)? == slot {
                return Ok(Some(binding.actor.clone()));
            }
        }
        Ok(None)
    }

    pub fn mirror(&self, actor: &ActorId) -> Result<i32, QvmWeaponError> {
        let shared = self.shared.borrow();
        let target = shared.targets.as_ref()()
            .into_iter()
            .find(|target| &target.actor == actor)
            .ok_or(QvmWeaponError::MissingActor)?;
        drop(shared);
        let mut binding = self.shared.borrow().bindings.get(actor).cloned();
        if binding.is_none() {
            match &target.kind {
                QvmWeaponTargetKind::Player { userinfo, .. } => {
                    let mut slot = 0;
                    while slot < 64 && self.shared.borrow().userinfo.contains_key(&slot) {
                        slot += 1;
                    }
                    if slot == 64 {
                        return Err(QvmWeaponError::ClientCapacity);
                    }
                    let record = self.game.data.entity_bytes(slot as usize)?;
                    let pointer = record.offset as i32;
                    if pointer == 0 {
                        return Err(QvmWeaponError::NullClient);
                    }
                    self.shared.borrow_mut().bindings.insert(
                        actor.clone(),
                        QvmBinding {
                            actor: actor.clone(),
                            pointer,
                            kind: QvmWeaponBindingKind::Client,
                        },
                    );
                    self.shared.borrow_mut().userinfo.insert(slot, userinfo.clone());
                    if let Err(error) = (|| -> Result<(), QvmWeaponError> {
                        if let Some(denial) = self.game.client_connect(slot, true, false)? {
                            return Err(QvmWeaponError::ClientRejected(denial));
                        }
                        self.game.client_begin(slot)?;
                        Ok(())
                    })() {
                        self.shared.borrow_mut().bindings.remove(actor);
                        self.shared.borrow_mut().userinfo.remove(&slot);
                        return Err(error);
                    }
                    binding = self.shared.borrow().bindings.get(actor).cloned();
                }
                QvmWeaponTargetKind::Actor => {
                    let pointer = self.game.module.call(&[], self.profile.layout.allocate)?;
                    self.slot(pointer)?;
                    if !self.live(pointer)?
                        || self
                            .shared
                            .borrow()
                            .bindings
                            .values()
                            .any(|entry| entry.pointer == pointer)
                    {
                        return Err(QvmWeaponError::OccupiedAllocation);
                    }
                    self.shared.borrow_mut().bindings.insert(
                        actor.clone(),
                        QvmBinding {
                            actor: actor.clone(),
                            pointer,
                            kind: QvmWeaponBindingKind::Target,
                        },
                    );
                    binding = self.shared.borrow().bindings.get(actor).cloned();
                }
            }
        }
        let binding = binding.ok_or(QvmWeaponError::MissingActor)?;
        if let QvmWeaponTargetKind::Player { userinfo, team } = &target.kind {
            if binding.kind != QvmWeaponBindingKind::Client {
                return Err(QvmWeaponError::ClientOwnership);
            }
            let slot = self.slot(binding.pointer)? as i32;
            if self.shared.borrow().userinfo.get(&slot).map(String::as_str) != Some(userinfo.as_str()) {
                self.shared.borrow_mut().userinfo.insert(slot, userinfo.clone());
                self.game.client_userinfo_changed(slot)?;
            }
            let team_mode = self.shared.borrow().team_mode;
            if *team != QvmWeaponTeam::Spectator
                && (team_mode && *team == QvmWeaponTeam::Free || !team_mode && *team != QvmWeaponTeam::Free)
            {
                return Err(QvmWeaponError::TeamMode);
            }
            // PERS_TEAM is public playerState.persistant[3] in both supported source ABIs.
            let window = self.game.data.public_player_bytes(slot as usize)?;
            if window.get_i32(260)? != team.number() {
                self.game
                    .client_command(slot, &[String::from("team"), String::from(team.name())])?;
            }
            let window = self.game.data.public_player_bytes(slot as usize)?;
            if window.get_i32(260)? != team.number() {
                return Err(QvmWeaponError::TeamDenied(String::from(team.name())));
            }
            for (offset, vector) in [(20, target.body.origin), (152, target.body.angles)] {
                window.set_f32(offset, vector.x)?;
                window.set_f32(offset + 4, vector.y)?;
                window.set_f32(offset + 8, vector.z)?;
            }
            window.set_i32(184, target.health.trunc() as i32)?;
        }
        self.project(binding.pointer, &target.body)?;
        let slot = self.slot(binding.pointer)?;
        self.game
            .data
            .entity_bytes(slot)?
            .set_i32(self.profile.layout.fields.health, target.health.trunc() as i32)?;
        Ok(binding.pointer)
    }

    fn refresh_targets(&self, exclude: Option<&ActorId>) -> Result<(), QvmWeaponError> {
        let targets = self.shared.borrow().targets.as_ref()();
        let actors: HashSet<ActorId> = targets.iter().map(|target| target.actor.clone()).collect();
        let stale: Vec<ActorId> = self
            .shared
            .borrow()
            .bindings
            .values()
            .filter(|binding| binding.kind != QvmWeaponBindingKind::Projectile && !actors.contains(&binding.actor))
            .map(|binding| binding.actor.clone())
            .collect();
        for actor in stale {
            self.release(&actor)?;
        }
        for target in &targets {
            if Some(&target.actor) != exclude
                && self
                    .shared
                    .borrow()
                    .bindings
                    .get(&target.actor)
                    .map(|binding| binding.kind)
                    != Some(QvmWeaponBindingKind::Projectile)
            {
                self.mirror(&target.actor)?;
            }
        }
        Ok(())
    }

    fn release(&self, actor: &ActorId) -> Result<(), QvmWeaponError> {
        self.shared.borrow_mut().retired.remove(actor);
        self.shared.borrow_mut().activated.remove(actor);
        let binding = self.shared.borrow_mut().bindings.remove(actor);
        let Some(binding) = binding else { return Ok(()) };
        if binding.kind == QvmWeaponBindingKind::Client {
            let slot = self.slot(binding.pointer)?;
            let outcome = self.game.client_disconnect(slot as i32);
            self.shared.borrow_mut().userinfo.remove(&(slot as i32));
            outcome?;
        } else if self.live(binding.pointer)? {
            self.game.module.call(&[binding.pointer], self.profile.layout.free)?;
        }
        Ok(())
    }

    /// Trap-facing console variables. Guards are short-lived; drop them
    /// before calling back into the source.
    pub fn trap_cvars(&self) -> RefMut<'_, CvarRegistry> {
        RefMut::map(self.shared.borrow_mut(), |shared| &mut shared.cvars)
    }

    /// Trap-facing files.
    pub fn trap_files(&self) -> RefMut<'_, QvmFiles> {
        RefMut::map(self.shared.borrow_mut(), |shared| &mut shared.files)
    }

    /// Trap-facing configstrings.
    pub fn trap_configstrings(&self) -> RefMut<'_, HashMap<i32, String>> {
        RefMut::map(self.shared.borrow_mut(), |shared| &mut shared.configstrings)
    }

    /// Trap-facing client userinfo.
    pub fn trap_userinfo(&self) -> RefMut<'_, HashMap<i32, String>> {
        RefMut::map(self.shared.borrow_mut(), |shared| &mut shared.userinfo)
    }

    /// Trap-facing entity-text cursor.
    pub fn trap_cursor(&self) -> RefMut<'_, CommonParseCursor> {
        RefMut::map(self.shared.borrow_mut(), |shared| &mut shared.cursor)
    }

    /// Trap-facing entity-text parser.
    pub fn trap_parser(&self) -> RefMut<'_, CommonParseState> {
        RefMut::map(self.shared.borrow_mut(), |shared| &mut shared.parser)
    }

    /// Trap-facing source time in seconds.
    pub fn trap_time(&self) -> f64 {
        self.shared.borrow().time
    }

    /// Trap-facing print sink.
    pub fn trap_print(&self) -> QvmWeaponPrint {
        Rc::clone(&self.shared.borrow().print)
    }

    /// Trap-facing game module.
    pub fn trap_game(&self) -> QvmGame {
        self.game.clone()
    }

    /// Currency assertion for trap handlers.
    pub fn trap_current(&self) {
        self.current();
    }

    fn profile_digest(&self) -> String {
        fn push_str(bytes: &mut Vec<u8>, text: &str) {
            bytes.extend_from_slice(&(text.len() as u64).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
        }
        fn push_usize(bytes: &mut Vec<u8>, value: usize) {
            bytes.extend_from_slice(&(value as u64).to_le_bytes());
        }
        let mut bytes = Vec::new();
        let definition = &self.profile.definition;
        for text in [
            &definition.id,
            &definition.title,
            &definition.aspect,
            &definition.module.id,
            &definition.module.artifact_path,
            &definition.module.digest,
            &definition.module.revision,
        ] {
            push_str(&mut bytes, text);
        }
        push_usize(&mut bytes, definition.role as usize);
        fn push_callback(bytes: &mut Vec<u8>, callback: &WeaponBehaviorCallback) {
            match callback {
                WeaponBehaviorCallback::QuakeC { module, function_index } => {
                    bytes.push(0);
                    for text in [&module.id, &module.artifact_path, &module.digest, &module.revision] {
                        push_str(bytes, text);
                    }
                    push_usize(bytes, *function_index);
                }
                WeaponBehaviorCallback::Qvm {
                    module,
                    instruction_index,
                } => {
                    bytes.push(1);
                    for text in [&module.id, &module.artifact_path, &module.digest, &module.revision] {
                        push_str(bytes, text);
                    }
                    push_usize(bytes, *instruction_index);
                }
                WeaponBehaviorCallback::NativeArtifact {
                    module,
                    image_offset,
                    abi,
                } => {
                    bytes.push(2);
                    for text in [&module.id, &module.artifact_path, &module.digest, &module.revision] {
                        push_str(bytes, text);
                    }
                    push_usize(bytes, *image_offset);
                    for text in [&abi.kind, &abi.call, &abi.image] {
                        push_str(bytes, text);
                    }
                    push_usize(bytes, abi.pointer_bytes as usize);
                }
            }
        }
        push_callback(&mut bytes, &definition.fire);
        match &definition.activate {
            None => bytes.push(0),
            Some(callback) => {
                bytes.push(1);
                push_callback(&mut bytes, callback);
            }
        }
        let layout = &self.profile.layout;
        for value in [
            layout.entity_stride,
            layout.level_time,
            layout.allocate,
            layout.free,
            layout.fields.inuse,
            layout.fields.nextthink,
            layout.fields.think,
            layout.fields.health,
        ] {
            push_usize(&mut bytes, value);
        }
        push_usize(&mut bytes, layout.fire_abi as usize);
        sha256_hex(&bytes)
    }
}
/// Attached QVM projectile trajectory.
pub struct QvmWeaponInstance {
    definition: Rc<WeaponBehaviorDefinition>,
    initial: WeaponTrajectoryUpdate,
    actor: ActorId,
    game: QvmGame,
    profile: Rc<QvmWeaponProfile>,
    shared: Rc<RefCell<Shared>>,
    generation: u64,
    closed: Cell<bool>,
}

impl std::fmt::Debug for QvmWeaponInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmWeaponInstance")
            .field("definition", &self.definition)
            .field("initial", &self.initial)
            .finish_non_exhaustive()
    }
}

impl WeaponBehaviorInstance for QvmWeaponInstance {
    type Error = QvmWeaponError;

    fn definition(&self) -> &WeaponBehaviorDefinition {
        self.definition.as_ref()
    }

    fn initial(&self) -> &WeaponTrajectoryUpdate {
        &self.initial
    }

    fn step(&self, body: &BodyState, time_seconds: f64) -> Result<Option<WeaponTrajectoryUpdate>, QvmWeaponError> {
        if self.closed.get() || self.generation != self.shared.borrow().generation {
            return Err(QvmWeaponError::ClosedInstance);
        }
        let source = QvmWeaponBehaviorSource {
            definition: Rc::clone(&self.definition),
            profile: Rc::clone(&self.profile),
            game: self.game.clone(),
            shared: Rc::clone(&self.shared),
        };
        source.operation(|| {
            source.set_time(time_seconds)?;
            source.refresh_targets(None)?;
            let current = source.shared.borrow().bindings.get(&self.actor).cloned();
            let Some(current) = current else { return Ok(None) };
            if !source.live(current.pointer)? {
                return source.retire_binding(&self.actor, body);
            }
            source.project(current.pointer, body)?;
            let fields = source.game.data.entity_bytes(source.slot(current.pointer)?)?;
            let next = fields.get_i32(source.profile.layout.fields.nextthink)?;
            if next <= 0 || next > (time_seconds * 1000.0).round() as i32 {
                return Ok(None);
            }
            let entry = fields.get_i32(source.profile.layout.fields.think)?;
            let is_enter = entry > 0
                && source
                    .shared
                    .borrow()
                    .game_artifact
                    .image
                    .instructions
                    .get(entry as usize)
                    .is_some_and(|instruction| instruction.opcode == GameOpcode::OpEnter);
            if !is_enter {
                return Err(QvmWeaponError::BadThink);
            }
            fields.set_i32(source.profile.layout.fields.nextthink, 0)?;
            source.game.module.call(&[current.pointer], entry as usize)?;
            if !source.live(current.pointer)? {
                return source.retire_binding(&self.actor, body);
            }
            let (_, entity) = source.read_record(current.pointer)?;
            if entity.s.e_type != 3 {
                source
                    .game
                    .module
                    .call(&[current.pointer], source.profile.layout.free)?;
                return source.retire_binding(&self.actor, body);
            }
            Ok(Some(source.trajectory(current.pointer)?))
        })
    }

    fn close(&self) {
        if self.closed.get() {
            return;
        }
        self.closed.set(true);
        if self.generation != self.shared.borrow().generation {
            return;
        }
        let source = QvmWeaponBehaviorSource {
            definition: Rc::clone(&self.definition),
            profile: Rc::clone(&self.profile),
            game: self.game.clone(),
            shared: Rc::clone(&self.shared),
        };
        source.shared.borrow_mut().instances.remove(&self.actor);
        drop(source.release(&self.actor));
    }
}

impl WeaponBehaviorSource for QvmWeaponBehaviorSource {
    type Error = QvmWeaponError;
    type Instance = QvmWeaponInstance;

    fn definition(&self) -> &WeaponBehaviorDefinition {
        self.definition.as_ref()
    }

    fn attach(&self, launch: WeaponBehaviorLaunch) -> Result<Option<QvmWeaponInstance>, QvmWeaponError> {
        self.operation(|| {
            self.set_time(launch.time_seconds)?;
            self.refresh_targets(Some(&launch.projectile.id().clone()))?;
            if launch.role != self.definition.role || self.shared.borrow().bindings.contains_key(launch.projectile.id())
            {
                return Err(QvmWeaponError::LaunchMismatch);
            }
            let shooter = self.mirror(&launch.shooter)?;
            let (shooter_slot, mut shooter_entity) = self.read_record(shooter)?;
            if self.definition.activate.is_some() && !self.shared.borrow().activated.contains(&launch.shooter) {
                let Some(WeaponBehaviorCallback::Qvm { instruction_index, .. }) = &self.definition.activate else {
                    return Err(QvmWeaponError::BadActivation);
                };
                self.game.module.call(&[shooter], *instruction_index)?;
                self.shared.borrow_mut().activated.insert(launch.shooter.clone());
            }
            let velocity = launch.body.velocity;
            let length =
                (f64::from(velocity.x).powi(2) + f64::from(velocity.y).powi(2) + f64::from(velocity.z).powi(2)).sqrt();
            if length.is_nan() || length <= 0.0 {
                return Err(QvmWeaponError::NoDirection);
            }
            shooter_entity.r.current_origin = launch.body.origin;
            shooter_entity.s.pos.delta = Vec3 {
                x: (f64::from(velocity.x) / length) as f32,
                y: (f64::from(velocity.y) / length) as f32,
                z: (f64::from(velocity.z) / length) as f32,
            };
            self.write_record(shooter_slot, &shooter_entity)?;
            let start_offset = if self.game.module.abi_profile().is_modern() {
                488
            } else {
                476
            };
            let WeaponBehaviorCallback::Qvm {
                instruction_index: fire,
                ..
            } = &self.definition.fire
            else {
                return Err(QvmWeaponError::BadFire);
            };
            let fire = *fire;
            let pointer = self
                .game
                .module
                .call(&[shooter, shooter + start_offset, shooter + 36], fire)?;
            self.slot(pointer)?;
            let (_, missile) = self.read_record(pointer)?;
            if !self.live(pointer)?
                || missile.s.e_type != 3
                || self
                    .shared
                    .borrow()
                    .bindings
                    .values()
                    .any(|binding| binding.pointer == pointer)
            {
                return Err(QvmWeaponError::BadMissile);
            }
            self.shared.borrow_mut().bindings.insert(
                launch.projectile.id().clone(),
                QvmBinding {
                    actor: launch.projectile.id().clone(),
                    pointer,
                    kind: QvmWeaponBindingKind::Projectile,
                },
            );
            Ok(Some(self.instance(launch.projectile.id())?))
        })
    }

    fn resume(&self, projectile: &ActorId) -> Result<QvmWeaponInstance, QvmWeaponError> {
        self.current();
        self.instance(projectile)
    }
}

impl QvmWeaponBehaviorSource {
    fn retire_binding(
        &self,
        actor: &ActorId,
        body: &BodyState,
    ) -> Result<Option<WeaponTrajectoryUpdate>, QvmWeaponError> {
        self.shared.borrow_mut().retired.insert(
            actor.clone(),
            WeaponTrajectoryUpdate {
                origin: body.origin,
                velocity: body.velocity,
                angles: body.angles,
            },
        );
        self.shared.borrow_mut().bindings.remove(actor);
        Ok(None)
    }

    fn instance(&self, actor: &ActorId) -> Result<QvmWeaponInstance, QvmWeaponError> {
        let shared = self.shared.borrow();
        if shared.instances.contains(actor) {
            return Err(QvmWeaponError::AlreadyAttached);
        }
        let binding = shared.bindings.get(actor).cloned();
        let retired = shared.retired.get(actor).cloned();
        drop(shared);
        if binding
            .as_ref()
            .is_some_and(|binding| binding.kind != QvmWeaponBindingKind::Projectile)
        {
            return Err(QvmWeaponError::MissingProjectile);
        }
        let initial = match (retired, &binding) {
            (Some(trajectory), _) => trajectory,
            (None, Some(binding)) => self.trajectory(binding.pointer)?,
            (None, None) => return Err(QvmWeaponError::MissingProjectile),
        };
        let generation = self.shared.borrow().generation;
        self.shared.borrow_mut().instances.insert(actor.clone());
        Ok(QvmWeaponInstance {
            definition: Rc::clone(&self.definition),
            initial,
            actor: actor.clone(),
            game: self.game.clone(),
            profile: Rc::clone(&self.profile),
            shared: Rc::clone(&self.shared),
            generation,
            closed: Cell::new(false),
        })
    }

    /// Capture the source checkpoint.
    pub fn checkpoint(&self) -> Result<QvmWeaponBehaviorCheckpoint, QvmWeaponError> {
        self.current();
        {
            let shared = self.shared.borrow();
            if shared.busy || !shared.ready {
                return Err(QvmWeaponError::CheckpointBusy);
            }
        }
        self.operation(|| self.refresh_targets(None))?;
        let mut shared = self.shared.borrow_mut();
        let module = self.game.module.checkpoint()?;
        let data = self.game.data.checkpoint();
        let cvars = capture_cvar_state(&mut shared.cvars);
        let configstrings = shared
            .configstrings
            .iter()
            .map(|(index, value)| QvmConfigstring {
                index: *index,
                value: value.clone(),
            })
            .collect();
        let files = shared.files.capture_checkpoint()?;
        let userinfo = shared
            .userinfo
            .iter()
            .map(|(slot, value)| QvmUserinfo {
                slot: *slot,
                value: value.clone(),
            })
            .collect();
        let host = QvmWeaponHostCheckpoint {
            mode: shared.mode,
            team_mode: shared.team_mode,
            time: shared.time,
            data,
            cvars,
            configstrings,
            files,
            entity_text: shared.cursor.source.clone(),
            userinfo,
            cursor: shared.cursor.offset(),
            parser: shared.parser.capture_save_state(),
        };
        Ok(QvmWeaponBehaviorCheckpoint {
            version: 1,
            definition: self.definition.as_ref().clone(),
            module,
            host,
            profile_digest: self.profile_digest(),
            activated: shared.activated.iter().map(SavedActorId::from).collect(),
            bindings: shared
                .bindings
                .values()
                .map(|binding| QvmWeaponBinding {
                    actor: SavedActorId::from(&binding.actor),
                    pointer: binding.pointer,
                    kind: binding.kind,
                })
                .collect(),
            retired: shared
                .retired
                .iter()
                .map(|(actor, trajectory)| QvmWeaponRetired {
                    actor: SavedActorId::from(actor),
                    trajectory: *trajectory,
                })
                .collect(),
        })
    }

    /// Restore a checkpoint, rolling the module back on failure.
    pub fn restore(
        &self,
        checkpoint: &QvmWeaponBehaviorCheckpoint,
        resolve_actor: &dyn Fn(&SavedActorId) -> ActorId,
    ) -> Result<(), QvmWeaponError> {
        self.operation(|| {
            let staged = self.construct_staged()?;
            Self::stage_validated(&staged, checkpoint, resolve_actor).inspect_err(|_| staged.close())?;
            let outcome = (|| -> Result<(), QvmWeaponError> {
                let previous = self.game.module.checkpoint()?;
                if let Err(error) = self.apply_staged(&staged, checkpoint) {
                    if let Err(rollback) = self.game.module.restore(&previous) {
                        return Err(QvmWeaponError::RollbackFailed(error.to_string(), rollback.to_string()));
                    }
                    return Err(error);
                }
                Ok(())
            })();
            staged.close();
            outcome?;
            self.shared.borrow_mut().generation += 1;
            Ok(())
        })
    }

    /// Stage a restored source without disturbing the live one.
    pub fn restore_staged(
        options: QvmWeaponBehaviorOptions,
        checkpoint: &QvmWeaponBehaviorCheckpoint,
        resolve_actor: &dyn Fn(&SavedActorId) -> ActorId,
    ) -> Result<Self, QvmWeaponError> {
        let (game_artifact, provider_artifact) = convert_artifact(&options.artifact)?;
        let profile = validate_qvm_weapon_profile(&options.profile, &provider_artifact)?;
        let source = Self::build(
            Parts {
                profile,
                game_artifact,
                mounts: Rc::new(options.mounts),
                seed: options.seed,
                entity_text: options.entity_text,
                mode: options.mode,
                team_mode: options.team_mode,
                targets: options.targets,
                print: options.print,
                assert_current: options.assert_current,
                traps: options.traps,
            },
            false,
        )?;
        if let Err(error) = Self::stage_validated(&source, checkpoint, resolve_actor) {
            source.close();
            return Err(error);
        }
        Ok(source)
    }

    fn stage_validated(
        source: &Self,
        checkpoint: &QvmWeaponBehaviorCheckpoint,
        resolve_actor: &dyn Fn(&SavedActorId) -> ActorId,
    ) -> Result<(), QvmWeaponError> {
        let outcome = (|| -> Result<(), QvmWeaponError> {
            if checkpoint.version != 1
                || !same_weapon_behavior(&source.definition, &checkpoint.definition)
                || checkpoint.profile_digest != source.profile_digest()
            {
                return Err(QvmWeaponError::ProfileMismatch);
            }
            source.game.module.restore(&checkpoint.module)?;
            source.apply_host(&checkpoint.host)?;
            source.validate_layout()?;
            let mut pointers = HashSet::new();
            let mut actors = HashSet::new();
            for saved in &checkpoint.bindings {
                let actor = resolve_actor(&saved.actor);
                source.slot(saved.pointer)?;
                if !actors.insert(actor.clone()) || !pointers.insert(saved.pointer) || !source.live(saved.pointer)? {
                    return Err(QvmWeaponError::BadBinding);
                }
                let owned = source
                    .shared
                    .borrow()
                    .userinfo
                    .contains_key(&(source.slot(saved.pointer)? as i32));
                if (saved.kind == QvmWeaponBindingKind::Client) != owned {
                    return Err(QvmWeaponError::ClientMismatch);
                }
                source.shared.borrow_mut().bindings.insert(
                    actor.clone(),
                    QvmBinding {
                        actor,
                        pointer: saved.pointer,
                        kind: saved.kind,
                    },
                );
            }
            let clients = source
                .shared
                .borrow()
                .bindings
                .values()
                .filter(|binding| binding.kind == QvmWeaponBindingKind::Client)
                .count();
            if clients != source.shared.borrow().userinfo.len() {
                return Err(QvmWeaponError::UnboundClient);
            }
            for saved in &checkpoint.retired {
                let actor = resolve_actor(&saved.actor);
                if !actors.insert(actor.clone()) {
                    return Err(QvmWeaponError::DuplicateActor);
                }
                source.shared.borrow_mut().retired.insert(actor, saved.trajectory);
            }
            for saved in &checkpoint.activated {
                let actor = resolve_actor(saved);
                if !source.shared.borrow().bindings.contains_key(&actor)
                    || !source.shared.borrow_mut().activated.insert(actor)
                {
                    return Err(QvmWeaponError::BadActivationRestore);
                }
            }
            source.shared.borrow_mut().ready = true;
            Ok(())
        })();
        outcome
    }

    fn apply_staged(&self, staged: &Self, checkpoint: &QvmWeaponBehaviorCheckpoint) -> Result<(), QvmWeaponError> {
        self.game.module.restore(&checkpoint.module)?;
        self.apply_host(&checkpoint.host)?;
        let staged_shared = staged.shared.borrow();
        let mut shared = self.shared.borrow_mut();
        shared.bindings.clear();
        for (actor, binding) in staged_shared.bindings.iter() {
            shared.bindings.insert(actor.clone(), binding.clone());
        }
        shared.retired.clear();
        for (actor, trajectory) in staged_shared.retired.iter() {
            shared.retired.insert(actor.clone(), *trajectory);
        }
        shared.activated.clear();
        for actor in staged_shared.activated.iter() {
            shared.activated.insert(actor.clone());
        }
        shared.instances.clear();
        Ok(())
    }

    fn apply_host(&self, host: &QvmWeaponHostCheckpoint) -> Result<(), QvmWeaponError> {
        let mut shared = self.shared.borrow_mut();
        if host.entity_text != shared.cursor.source || host.mode != shared.mode || host.team_mode != shared.team_mode {
            return Err(QvmWeaponError::BadHost);
        }
        if !host.time.is_finite() || host.time < 0.0 {
            return Err(QvmWeaponError::BadSourceTime);
        }
        shared.time = host.time;
        self.game.data.restore(&host.data)?;
        restore_cvar_state(&mut shared.cvars, &host.cvars)?;
        shared.configstrings.clear();
        shared.userinfo.clear();
        for entry in &host.userinfo {
            if entry.slot < 0 || entry.slot >= 64 || shared.userinfo.contains_key(&entry.slot) {
                return Err(QvmWeaponError::BadUserinfo);
            }
            shared.userinfo.insert(entry.slot, entry.value.clone());
        }
        for row in &host.configstrings {
            if row.index < 0 || row.index >= 1024 || shared.configstrings.contains_key(&row.index) {
                return Err(QvmWeaponError::BadConfigstring);
            }
            shared.configstrings.insert(row.index, row.value.clone());
        }
        shared.cursor.set_offset(host.cursor)?;
        shared.parser.restore_save_state(&host.parser)?;
        shared.files.restore_checkpoint(&host.files)?;
        Ok(())
    }

    /// Retire the source.
    pub fn close(&self) {
        if self.shared.borrow().closed {
            return;
        }
        self.shared.borrow_mut().closed = true;
        self.shared.borrow_mut().instances.clear();
        self.shared.borrow_mut().bindings.clear();
        self.shared.borrow_mut().retired.clear();
        self.shared.borrow_mut().activated.clear();
        self.shared.borrow_mut().userinfo.clear();
        self.shared.borrow_mut().files.close_all();
        self.game.retire();
    }
}
/// Only source initialization and client spawn points are needed by a trajectory component.
pub fn qvm_weapon_initialization_entities(text: &str) -> Result<String, QvmWeaponError> {
    let classes = [
        "worldspawn",
        "info_player_start",
        "info_player_deathmatch",
        "info_player_intermission",
        "team_CTF_redplayer",
        "team_CTF_blueplayer",
        "team_CTF_redspawn",
        "team_CTF_bluespawn",
    ];
    let entities: Vec<_> = parse_q1_entities(text, "qvm-weapon-behavior")?
        .into_iter()
        .filter(|entity| classes.contains(&q1_entity_value(entity, "classname").unwrap_or("")))
        .collect();
    if entities
        .iter()
        .filter(|entity| q1_entity_value(entity, "classname") == Some("worldspawn"))
        .count()
        != 1
    {
        return Err(QvmWeaponError::Worldspawn);
    }
    let quote = |value: &str| -> Result<String, QvmWeaponError> {
        if value.contains('"') || value.contains('\0') {
            return Err(QvmWeaponError::Unquotable);
        }
        Ok(format!("\"{value}\""))
    };
    let mut out = String::new();
    for entity in &entities {
        out.push_str("{\n");
        for (index, (key, value)) in entity.properties.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            out.push_str(&quote(key)?);
            out.push(' ');
            out.push_str(&quote(value)?);
        }
        out.push_str("\n}\n");
    }
    Ok(out)
}

fn capture_profile_value(value: &ProfileValue) -> qa_world::save::value::SaveJson {
    use qa_world::save::value::SaveJson;
    use qa_world::save::value::{arr, boolean, num, obj, str as json_str};

    match value {
        // Profile producers in this module never emit `Undefined`; it shares
        // the null encoding.
        ProfileValue::Undefined | ProfileValue::Null => SaveJson::Null,
        ProfileValue::Bool(value) => boolean(*value),
        ProfileValue::Int(value) => SaveJson::BigInt(i128::from(*value)),
        ProfileValue::Float(value) => num(*value),
        ProfileValue::Str(value) => json_str(value),
        ProfileValue::Bytes(value) => SaveJson::Bytes(value.clone()),
        ProfileValue::Array(values) => arr(values.iter().map(capture_profile_value).collect()),
        ProfileValue::Record(fields) => obj(fields
            .iter()
            .map(|(key, value)| (key.as_str(), capture_profile_value(value)))
            .collect()),
    }
}

fn read_profile_value(reader: &SaveReader) -> Result<ProfileValue, WorldError> {
    use qa_world::save::value::SaveJson;

    match reader.value {
        None => Err(reader.fail("expected a profile value")),
        Some(SaveJson::Null) => Ok(ProfileValue::Null),
        Some(SaveJson::Bool(value)) => Ok(ProfileValue::Bool(*value)),
        Some(SaveJson::Number(value)) => Ok(ProfileValue::Float(*value)),
        Some(SaveJson::BigInt(value)) => {
            let value = i64::try_from(*value).map_err(|_| reader.fail("expected an integer in range"))?;
            Ok(ProfileValue::Int(value))
        }
        Some(SaveJson::Bytes(value)) => Ok(ProfileValue::Bytes(value.clone())),
        Some(SaveJson::String(value)) => Ok(ProfileValue::Str(value.clone())),
        Some(SaveJson::Array(_)) => Ok(ProfileValue::Array(reader.list(|cell| read_profile_value(&cell))?)),
        Some(SaveJson::Object(members)) => {
            let mut fields = Vec::with_capacity(members.len());
            for (key, _) in members {
                fields.push((key.clone(), read_profile_value(&reader.field(key))?));
            }
            Ok(ProfileValue::Record(fields))
        }
    }
}

/// Capture a weapon checkpoint in donor save shape, with the host state
/// alongside the module checkpoint.
#[must_use]
pub fn capture_qvm_weapon_behavior_checkpoint(
    checkpoint: &QvmWeaponBehaviorCheckpoint,
) -> qa_world::save::value::SaveJson {
    use qa_world::save::value::SaveJson;
    use qa_world::save::value::{arr, boolean, int, num, obj, str as json_str};

    use super::quakec_weapon_behavior::capture_weapon_behavior_definition;

    fn capture_actor(actor: &SavedActorId) -> SaveJson {
        obj(vec![
            ("slot", int(i64::from(actor.slot))),
            ("generation", int(i64::from(actor.generation))),
        ])
    }

    fn capture_trajectory(trajectory: &WeaponTrajectoryUpdate) -> SaveJson {
        let vector = |value: Vec3| {
            obj(vec![
                ("x", num(f64::from(value.x))),
                ("y", num(f64::from(value.y))),
                ("z", num(f64::from(value.z))),
            ])
        };
        obj(vec![
            ("origin", vector(trajectory.origin)),
            ("velocity", vector(trajectory.velocity)),
            ("angles", vector(trajectory.angles)),
        ])
    }

    let module = &checkpoint.module;
    let host = &checkpoint.host;
    obj(vec![
        ("version", int(checkpoint.version)),
        ("definition", capture_weapon_behavior_definition(&checkpoint.definition)),
        (
            "module",
            obj(vec![
                (
                    "module",
                    obj(vec![
                        ("id", json_str(&module.module.id)),
                        ("artifactPath", json_str(&module.module.artifact_path)),
                        ("digest", json_str(&module.module.digest)),
                        ("revision", json_str(&module.module.revision)),
                    ]),
                ),
                ("apiKind", json_str(&module.api_kind)),
                ("apiVersion", int(i64::from(module.api_version))),
                (
                    "abi",
                    json_str(if module.abi_profile.is_modern() {
                        "q3-modern"
                    } else {
                        "q3-1.16n-base"
                    }),
                ),
                ("data", SaveJson::Bytes(module.data.clone())),
                ("instructionIndex", int(module.instruction_index as i64)),
                (
                    "operandStack",
                    arr(module
                        .operand_stack
                        .iter()
                        .map(|value| int(i64::from(*value)))
                        .collect()),
                ),
                ("programStack", int(module.program_stack as i64)),
                (
                    "hostState",
                    obj(vec![
                        (
                            "module",
                            obj(vec![
                                ("id", json_str(&module.host_state.module.id)),
                                ("artifactPath", json_str(&module.host_state.module.artifact_path)),
                                ("digest", json_str(&module.host_state.module.digest)),
                                ("revision", json_str(&module.host_state.module.revision)),
                            ]),
                        ),
                        ("format", json_str(&module.host_state.format)),
                        ("bytes", capture_profile_value(&module.host_state.bytes)),
                    ]),
                ),
            ]),
        ),
        (
            "host",
            obj(vec![
                (
                    "mode",
                    json_str(match host.mode {
                        SimulationMode::Singleplayer => "singleplayer",
                        SimulationMode::Coop => "coop",
                        SimulationMode::Deathmatch => "deathmatch",
                    }),
                ),
                ("teamMode", boolean(host.team_mode)),
                ("time", num(host.time)),
                (
                    "data",
                    obj(vec![
                        ("entitiesWord", int(host.data.entities_word as i64)),
                        ("numEntities", int(host.data.num_entities as i64)),
                        ("entityStride", int(host.data.entity_stride as i64)),
                        ("clientsWord", int(host.data.clients_word as i64)),
                        ("clientStride", int(host.data.client_stride as i64)),
                    ]),
                ),
                (
                    "cvars",
                    obj(vec![
                        ("dialect", json_str(&host.cvars.dialect)),
                        (
                            "variables",
                            arr(host
                                .cvars
                                .variables
                                .iter()
                                .map(|snapshot| {
                                    obj(vec![
                                        ("name", json_str(&snapshot.name)),
                                        ("value", json_str(&snapshot.value)),
                                        ("resetValue", json_str(&snapshot.reset_value)),
                                        (
                                            "latchedValue",
                                            snapshot
                                                .latched_value
                                                .as_ref()
                                                .map_or(SaveJson::Null, |latched| json_str(latched)),
                                        ),
                                        ("flags", int(i64::from(snapshot.flags))),
                                        ("modified", boolean(snapshot.modified)),
                                        ("modificationCount", int(i64::from(snapshot.modification_count))),
                                        ("numericValue", num(f64::from(snapshot.numeric_value))),
                                        ("integerValue", int(i64::from(snapshot.integer_value))),
                                    ])
                                })
                                .collect()),
                        ),
                        (
                            "order",
                            arr(host.cvars.order.iter().map(|name| json_str(name)).collect()),
                        ),
                        ("changedFlags", int(i64::from(host.cvars.changed_flags))),
                        ("userinfoDirty", boolean(host.cvars.userinfo_dirty)),
                    ]),
                ),
                (
                    "configstrings",
                    arr(host
                        .configstrings
                        .iter()
                        .map(|entry| {
                            obj(vec![
                                ("index", int(i64::from(entry.index))),
                                ("value", json_str(&entry.value)),
                            ])
                        })
                        .collect()),
                ),
                (
                    "files",
                    arr(host
                        .files
                        .handles
                        .iter()
                        .map(|(slot, handle)| {
                            let handle = match handle {
                                qa_guest::qvm::file_syscalls::HandleCheckpoint::Read(read) => obj(vec![
                                    ("kind", json_str("read")),
                                    ("bytes", SaveJson::Bytes(read.bytes.clone())),
                                    ("position", int(read.position as i64)),
                                    ("pk3", boolean(read.pk3)),
                                ]),
                                qa_guest::qvm::file_syscalls::HandleCheckpoint::Write(write) => obj(vec![
                                    ("kind", json_str("write")),
                                    ("path", json_str(&write.path)),
                                    (
                                        "mode",
                                        json_str(match write.mode {
                                            qa_guest::qvm::file_syscalls::WriteMode::Write => "write",
                                            qa_guest::qvm::file_syscalls::WriteMode::Append => "append",
                                            qa_guest::qvm::file_syscalls::WriteMode::AppendSync => "append-sync",
                                        }),
                                    ),
                                    ("position", int(write.position as i64)),
                                ]),
                            };
                            obj(vec![("slot", int(i64::from(*slot))), ("handle", handle)])
                        })
                        .collect()),
                ),
                ("entityText", SaveJson::Bytes(host.entity_text.clone())),
                (
                    "userinfo",
                    arr(host
                        .userinfo
                        .iter()
                        .map(|entry| {
                            obj(vec![
                                ("slot", int(i64::from(entry.slot))),
                                ("value", json_str(&entry.value)),
                            ])
                        })
                        .collect()),
                ),
                (
                    "cursor",
                    host.cursor.map_or(SaveJson::Null, |offset| int(offset as i64)),
                ),
                ("parser", capture_profile_value(&host.parser)),
            ]),
        ),
        ("profileDigest", json_str(&checkpoint.profile_digest)),
        (
            "activated",
            arr(checkpoint.activated.iter().map(capture_actor).collect()),
        ),
        (
            "bindings",
            arr(checkpoint
                .bindings
                .iter()
                .map(|binding| {
                    obj(vec![
                        ("actor", capture_actor(&binding.actor)),
                        ("pointer", int(i64::from(binding.pointer))),
                        (
                            "kind",
                            json_str(match binding.kind {
                                QvmWeaponBindingKind::Target => "target",
                                QvmWeaponBindingKind::Client => "client",
                                QvmWeaponBindingKind::Projectile => "projectile",
                            }),
                        ),
                    ])
                })
                .collect()),
        ),
        (
            "retired",
            arr(checkpoint
                .retired
                .iter()
                .map(|entry| {
                    obj(vec![
                        ("actor", capture_actor(&entry.actor)),
                        ("trajectory", capture_trajectory(&entry.trajectory)),
                    ])
                })
                .collect()),
        ),
    ])
}

/// Read a saved weapon checkpoint.
pub fn read_qvm_weapon_behavior_checkpoint(
    reader: &SaveReader,
    expected: &WeaponBehaviorDefinition,
    definitions: &impl WeaponBehaviorDefinitionReader,
) -> Result<QvmWeaponBehaviorCheckpoint, WorldError> {
    use qa_guest::qvm::file_syscalls::HandleCheckpoint;
    use qa_guest::qvm::file_syscalls::ReadHandleCheckpoint;
    use qa_guest::qvm::file_syscalls::WriteHandleCheckpoint;
    use qa_guest::qvm::file_syscalls::WriteMode;
    use qa_guest::qvm::game_data::ModuleIdentity;
    use qa_guest::qvm::game_data::QvmHostState;

    let module = reader.field("module");
    let host = reader.field("host");
    let read_identity = |cell: SaveReader| -> Result<ModuleIdentity, WorldError> {
        Ok(ModuleIdentity {
            id: cell.field("id").string()?,
            artifact_path: cell.field("artifactPath").string()?,
            digest: cell.field("digest").string()?,
            revision: cell.field("revision").string()?,
        })
    };
    let abi = match module
        .field("abi")
        .choice_str(&["q3-modern", "q3-1.16n-base"])?
        .as_str()
    {
        "q3-modern" => AbiProfile::Modern,
        _ => AbiProfile::Legacy,
    };
    let host_state = module.field("hostState");
    let module = QvmCheckpoint {
        module: read_identity(module.field("module"))?,
        api_kind: module.field("apiKind").string()?,
        api_version: i32::try_from(module.field("apiVersion").integer(i64::MIN)?)
            .map_err(|_| module.field("apiVersion").fail("expected an integer in range"))?,
        abi_profile: abi,
        data: module.field("data").bytes()?,
        instruction_index: usize::try_from(module.field("instructionIndex").integer(0)?)
            .map_err(|_| module.field("instructionIndex").fail("expected an integer in range"))?,
        operand_stack: module
            .field("operandStack")
            .list(|cell| cell.integer(i64::MIN))?
            .into_iter()
            .map(|value| {
                i32::try_from(value).map_err(|_| module.field("operandStack").fail("expected an integer in range"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        program_stack: usize::try_from(module.field("programStack").integer(0)?)
            .map_err(|_| module.field("programStack").fail("expected an integer in range"))?,
        host_state: QvmHostState {
            module: read_identity(host_state.field("module"))?,
            format: host_state.field("format").string()?,
            bytes: read_profile_value(&host_state.field("bytes"))?,
        },
    };
    let data = host.field("data");
    let cvars = host.field("cvars");
    let files = host
        .field("files")
        .list(|row| {
            let slot = row.field("slot").integer(i64::MIN)?;
            let slot = i32::try_from(slot).map_err(|_| row.field("slot").fail("expected an integer in range"))?;
            let handle = row.field("handle");
            let kind = handle.field("kind").choice_str(&["read", "write"])?;
            let handle = if kind == "read" {
                HandleCheckpoint::Read(ReadHandleCheckpoint {
                    bytes: handle.field("bytes").bytes()?,
                    position: usize::try_from(handle.field("position").integer(0)?)
                        .map_err(|_| handle.field("position").fail("expected an integer in range"))?,
                    pk3: handle.field("pk3").boolean()?,
                })
            } else {
                let mode = handle.field("mode").choice_str(&["write", "append", "append-sync"])?;
                HandleCheckpoint::Write(WriteHandleCheckpoint {
                    path: handle.field("path").string()?,
                    mode: if mode == "write" {
                        WriteMode::Write
                    } else if mode == "append" {
                        WriteMode::Append
                    } else {
                        WriteMode::AppendSync
                    },
                    position: usize::try_from(handle.field("position").integer(0)?)
                        .map_err(|_| handle.field("position").fail("expected an integer in range"))?,
                })
            };
            Ok((slot, handle))
        })?
        .into_iter()
        .collect::<Vec<_>>();
    let host = QvmWeaponHostCheckpoint {
        mode: match host
            .field("mode")
            .choice_str(&["singleplayer", "coop", "deathmatch"])?
            .as_str()
        {
            "singleplayer" => SimulationMode::Singleplayer,
            "coop" => SimulationMode::Coop,
            _ => SimulationMode::Deathmatch,
        },
        team_mode: host.field("teamMode").boolean()?,
        time: host.field("time").finite()?,
        data: QvmGameDataState {
            entities_word: usize::try_from(data.field("entitiesWord").integer(i64::MIN)?)
                .map_err(|_| data.field("entitiesWord").fail("expected an integer in range"))?,
            num_entities: usize::try_from(data.field("numEntities").integer(0)?)
                .map_err(|_| data.field("numEntities").fail("expected an integer in range"))?,
            entity_stride: usize::try_from(data.field("entityStride").integer(0)?)
                .map_err(|_| data.field("entityStride").fail("expected an integer in range"))?,
            clients_word: usize::try_from(data.field("clientsWord").integer(i64::MIN)?)
                .map_err(|_| data.field("clientsWord").fail("expected an integer in range"))?,
            client_stride: usize::try_from(data.field("clientStride").integer(0)?)
                .map_err(|_| data.field("clientStride").fail("expected an integer in range"))?,
        },
        cvars: QvmCvarSaveState {
            dialect: cvars.field("dialect").string()?,
            variables: cvars.field("variables").list(|cell| {
                Ok(CvarSnapshot {
                    name: cell.field("name").string()?,
                    value: cell.field("value").string()?,
                    reset_value: cell.field("resetValue").string()?,
                    latched_value: cell.field("latchedValue").nullable(|cell| cell.string())?,
                    flags: u32::try_from(cell.field("flags").integer(0)?)
                        .map_err(|_| cell.field("flags").fail("expected an integer in range"))?,
                    modified: cell.field("modified").boolean()?,
                    modification_count: u32::try_from(cell.field("modificationCount").integer(0)?)
                        .map_err(|_| cell.field("modificationCount").fail("expected an integer in range"))?,
                    numeric_value: cell.field("numericValue").number()? as f32,
                    integer_value: i32::try_from(cell.field("integerValue").integer(i64::MIN)?)
                        .map_err(|_| cell.field("integerValue").fail("expected an integer in range"))?,
                })
            })?,
            order: host.field("cvars").field("order").list(|cell| cell.string())?,
            changed_flags: u32::try_from(cvars.field("changedFlags").integer(0)?)
                .map_err(|_| cvars.field("changedFlags").fail("expected an integer in range"))?,
            userinfo_dirty: cvars.field("userinfoDirty").boolean()?,
        },
        configstrings: host.field("configstrings").list(|row| {
            let index = row.field("index").integer(0)?;
            let index = i32::try_from(index).map_err(|_| row.field("index").fail("expected an integer in range"))?;
            Ok(QvmConfigstring {
                index,
                value: row.field("value").string()?,
            })
        })?,
        files: FilesCheckpoint { handles: files },
        entity_text: host.field("entityText").bytes()?,
        userinfo: host.field("userinfo").list(|row| {
            let slot = row.field("slot").integer(0)?;
            let slot = i32::try_from(slot).map_err(|_| row.field("slot").fail("expected an integer in range"))?;
            Ok(QvmUserinfo {
                slot,
                value: row.field("value").string()?,
            })
        })?,
        cursor: host
            .field("cursor")
            .nullable(|cell| cell.integer(0))?
            .map(|offset| offset as usize),
        parser: read_profile_value(&host.field("parser"))?,
    };
    let profile_digest = reader.field("profileDigest").string()?;
    if profile_digest.len() != 64 || !profile_digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(reader.field("profileDigest").fail("invalid profile digest"));
    }
    Ok(QvmWeaponBehaviorCheckpoint {
        version: reader.field("version").literal_i64(1)?,
        definition: definitions.read_definition(reader.field("definition"), expected)?,
        module,
        host,
        profile_digest,
        activated: reader.field("activated").list(|cell| read_saved_actor(cell))?,
        bindings: reader.field("bindings").list(|row| {
            let pointer = row.field("pointer").integer(1)?;
            let pointer =
                i32::try_from(pointer).map_err(|_| row.field("pointer").fail("expected an integer in range"))?;
            let kind = match row
                .field("kind")
                .choice_str(&["target", "client", "projectile"])?
                .as_str()
            {
                "target" => QvmWeaponBindingKind::Target,
                "client" => QvmWeaponBindingKind::Client,
                _ => QvmWeaponBindingKind::Projectile,
            };
            Ok(QvmWeaponBinding {
                actor: read_saved_actor(row.field("actor"))?,
                pointer,
                kind,
            })
        })?,
        retired: reader.field("retired").list(|row| {
            let trajectory = row.field("trajectory");
            Ok(QvmWeaponRetired {
                actor: read_saved_actor(row.field("actor"))?,
                trajectory: WeaponTrajectoryUpdate {
                    origin: read_vector(trajectory.field("origin"))?,
                    velocity: read_vector(trajectory.field("velocity"))?,
                    angles: read_vector(trajectory.field("angles"))?,
                },
            })
        })?,
    })
}
// QVM-PART6-APPENDS-HERE

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::create_mount_plan_id;
    use qa_content::contract::ResolvedMountPlan;
    use qa_content::mounts::open_mount_plan;
    use qa_content::mounts::OpenMountOptions;
    use qa_core::identity::IdentityOwner;
    use qa_core::identity::ProviderId;
    use qa_core::math::vec3;
    use qa_core::math::Bounds;
    use qa_guest::core::contracts::ContentDigest;
    use qa_guest::core::contracts::ModuleIdentity;
    use qa_guest::qvm::artifacts::KnownQvmArtifact;
    use qa_guest::qvm::artifacts::QvmProduct;
    use qa_guest::qvm::artifacts::QvmReplacement;
    use qa_guest::qvm::artifacts::QvmReplacementInstance;
    use qa_guest::qvm::game_data::QvmHostState;
    use qa_guest::qvm::game_data::QvmSharedMemory;
    use qa_guest::qvm::weapon_behavior_profile::FireAbi;
    use qa_guest::qvm::weapon_behavior_profile::QvmWeaponBehaviorLayout;
    use qa_guest::qvm::weapon_behavior_profile::WeaponBehaviorRole;
    use qa_guest::qvm::weapon_behavior_profile::WeaponLayoutFields;
    use qa_world::body::BodyState;
    use qa_world::save::value::SaveReader;

    fn module_id() -> ModuleId {
        ModuleId {
            id: "test:qagame".to_string(),
            artifact_path: "vm/qagame.qvm".to_string(),
            digest: "sha256:abc123".to_string(),
            revision: "r1".to_string(),
        }
    }

    fn definition() -> WeaponBehaviorDefinition {
        let module = module_id();
        WeaponBehaviorDefinition {
            id: "qvm:test".to_string(),
            title: "Test".to_string(),
            role: WeaponBehaviorRole::Rocket,
            aspect: "trajectory".to_string(),
            module: module.clone(),
            fire: WeaponBehaviorCallback::Qvm {
                module,
                instruction_index: 3,
            },
            activate: None,
        }
    }

    fn layout() -> QvmWeaponBehaviorLayout {
        QvmWeaponBehaviorLayout {
            entity_stride: 1024,
            level_time: 1024,
            allocate: 1,
            free: 2,
            fields: WeaponLayoutFields {
                inuse: 516,
                nextthink: 520,
                think: 524,
                health: 528,
            },
            fire_abi: FireAbi::EntityPointerStartDirection,
        }
    }

    fn profile() -> QvmWeaponProfile {
        QvmWeaponProfile {
            definition: definition(),
            layout: layout(),
        }
    }

    fn game_artifact() -> GameArtifact {
        GameArtifact {
            module: qa_guest::qvm::game_data::ModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: "vm/qagame.qvm".to_string(),
                digest: "sha256:abc123".to_string(),
                revision: "r1".to_string(),
            },
            role: GameRole::Qagame,
            abi_profile: Some(AbiProfile::Modern),
            image: GameImage {
                source: "test".to_string(),
                instructions: Vec::new(),
                code_offset: 0,
                code_length: 0,
                data_length: 0,
                literal_length: 0,
                bss_length: 0,
                initialized_data: Vec::new(),
                allocated_data_length: 16384,
                data_mask: usize::MAX,
            },
        }
    }

    fn mounts() -> MountedContent {
        let plan = ResolvedMountPlan {
            id: create_mount_plan_id("test", "r1").unwrap(),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        };
        open_mount_plan(&plan, OpenMountOptions::default()).unwrap()
    }

    fn entity_text() -> String {
        "{\n\"classname\" \"worldspawn\"\n}\n".to_string()
    }

    struct FixedTraps(Option<i32>);

    impl QvmWeaponTraps for FixedTraps {
        fn dispatch(
            &mut self,
            _call: &QvmHostCall,
            _source: &QvmWeaponBehaviorSource,
        ) -> Result<Option<i32>, GuestError> {
            Ok(self.0)
        }
    }

    fn parts() -> Parts {
        Parts {
            profile: profile(),
            game_artifact: game_artifact(),
            mounts: Rc::new(mounts()),
            seed: 7,
            entity_text: entity_text(),
            mode: SimulationMode::Deathmatch,
            team_mode: false,
            targets: Rc::new(Vec::new),
            print: Rc::new(|_| {}),
            assert_current: Rc::new(|| {}),
            traps: Rc::new(RefCell::new(FixedTraps(None))),
        }
    }

    fn staged() -> QvmWeaponBehaviorSource {
        QvmWeaponBehaviorSource::build(parts(), false).unwrap()
    }

    fn ready() -> QvmWeaponBehaviorSource {
        let source = staged();
        source.shared.borrow_mut().ready = true;
        source
    }

    fn parsed_image() -> ParsedImage {
        let operands = [
            QvmOperand::Word(5),
            QvmOperand::Byte(7),
            QvmOperand::None,
            QvmOperand::Word(0),
        ];
        let instructions = operands
            .into_iter()
            .enumerate()
            .map(|(index, operand)| ParsedInstruction {
                byte_offset: index * 5,
                opcode: ParsedOpcode::OpEnter,
                operand,
            })
            .collect();
        ParsedImage {
            source: "test".to_string(),
            instructions,
            code_offset: 0,
            code_length: 20,
            data_length: 4096,
            literal_length: 0,
            bss_length: 0,
            initialized_data: Vec::new(),
            allocated_data_length: 16384,
            data_mask: 16383,
        }
    }

    fn bytecode(abi_profile: SysAbiProfile) -> ResolvedQvmArtifact {
        ResolvedQvmArtifact::Bytecode {
            module: ModuleIdentity::new(
                ProviderId::new("test", "qagame"),
                "vm/qagame.qvm",
                ContentDigest::new("sha256", "abc123"),
                "r1",
            ),
            role: SysRole::Qagame,
            known: None,
            image: parsed_image(),
            abi_profile,
        }
    }

    fn options() -> QvmWeaponBehaviorOptions {
        QvmWeaponBehaviorOptions {
            artifact: bytecode(SysAbiProfile::Modern),
            profile: profile(),
            mounts: mounts(),
            seed: 7,
            entity_text: entity_text(),
            mode: SimulationMode::Deathmatch,
            team_mode: false,
            targets: Rc::new(Vec::new),
            print: Rc::new(|_| {}),
            assert_current: Rc::new(|| {}),
            traps: Rc::new(RefCell::new(FixedTraps(None))),
        }
    }

    fn body() -> BodyState {
        BodyState {
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(1.0, 0.0, 0.0),
            bounds: Bounds {
                min: vec3(-16.0, -16.0, -16.0),
                max: vec3(16.0, 16.0, 16.0),
            },
            ground: None,
        }
    }

    fn launch(owner: &IdentityOwner, role: WeaponBehaviorRole) -> WeaponBehaviorLaunch {
        let projectile = owner
            .owned_actor(&owner.actor(5, 0), ProviderId::new("test", "qvm"))
            .unwrap();
        WeaponBehaviorLaunch {
            projectile,
            shooter: owner.actor(1, 0),
            weapon: "q3:weapon/rocket".to_string(),
            role,
            time_seconds: 1.0,
            body: body(),
        }
    }

    fn host_call_fixture(code: i32) -> QvmHostCall {
        QvmHostCall {
            kind: CallKind::Engine,
            role: GameRole::Qagame,
            code,
            words: Vec::new(),
            guest: QvmSharedMemory::from_bytes(vec![0; 64]),
            abi_profile: AbiProfile::Modern,
            command_arguments: None,
        }
    }

    #[test]
    fn initialization_entities_filter_and_quote() {
        let text = "{\n\"classname\" \"worldspawn\"\n\"message\" \"hi\"\n}\n\
            {\n\"classname\" \"info_player_deathmatch\"\n}\n\
            {\n\"classname\" \"func_wall\"\n}\n";
        let out = qvm_weapon_initialization_entities(text).unwrap();
        assert!(out.contains("\"classname\" \"worldspawn\""));
        assert!(out.contains("\"message\" \"hi\""));
        assert!(out.contains("\"classname\" \"info_player_deathmatch\""));
        assert!(!out.contains("func_wall"));
    }

    #[test]
    fn initialization_entities_reject_worldspawn_count() {
        let none = "{\n\"classname\" \"info_player_start\"\n}\n";
        assert!(matches!(
            qvm_weapon_initialization_entities(none).unwrap_err(),
            QvmWeaponError::Worldspawn
        ));
        let two = "{\n\"classname\" \"worldspawn\"\n}\n{\n\"classname\" \"worldspawn\"\n}\n";
        assert!(matches!(
            qvm_weapon_initialization_entities(two).unwrap_err(),
            QvmWeaponError::Worldspawn
        ));
    }

    #[test]
    fn convert_artifact_maps_bytecode() {
        let (game, provider) = convert_artifact(&bytecode(SysAbiProfile::Modern)).unwrap();
        assert_eq!(game.module.id, "test:qagame");
        assert_eq!(game.module.digest, "sha256:abc123");
        assert_eq!(game.module.artifact_path, "vm/qagame.qvm");
        assert_eq!(game.module.revision, "r1");
        assert_eq!(game.role, GameRole::Qagame);
        assert_eq!(game.abi_profile, Some(AbiProfile::Modern));
        assert_eq!(game.image.instructions.len(), 4);
        assert_eq!(game.image.instructions[0].opcode, GameOpcode::OpEnter);
        assert_eq!(game.image.instructions[0].operand, 5);
        assert_eq!(game.image.instructions[0].operand_width, 4);
        assert_eq!(game.image.instructions[0].byte_offset, 0);
        assert_eq!(game.image.instructions[1].operand, 7);
        assert_eq!(game.image.instructions[1].operand_width, 1);
        assert_eq!(game.image.instructions[2].operand, 0);
        assert_eq!(game.image.instructions[2].operand_width, 0);
        assert_eq!(game.image.data_mask, 16383);
        assert_eq!(provider.module.id, "test:qagame");
        assert_eq!(provider.role, ProviderRole::Qagame);
        assert_eq!(provider.abi_profile, Some(QvmAbi::Modern));
        assert_eq!(provider.image.instructions[0].opcode, ProviderOpcode::OpEnter);
        assert_eq!(provider.image.instructions[0].operand, 5);
        assert_eq!(provider.image.initialized_length, 0);
        assert_eq!(provider.image.allocated_data_length, 16384);
        let (legacy_game, legacy_provider) = convert_artifact(&bytecode(SysAbiProfile::Legacy116n)).unwrap();
        assert_eq!(legacy_game.abi_profile, Some(AbiProfile::Legacy));
        assert_eq!(legacy_provider.abi_profile, Some(QvmAbi::Legacy));
    }

    #[test]
    fn convert_artifact_rejects_native() {
        let resolved = ResolvedQvmArtifact::TypeScript {
            module: ModuleIdentity::new(
                ProviderId::new("test", "qagame"),
                "vm/qagame.qvm",
                ContentDigest::new("sha256", "abc123"),
                "r1",
            ),
            role: SysRole::Qagame,
            known: None,
            replacement: QvmReplacement {
                artifact: KnownQvmArtifact {
                    role: SysRole::Qagame,
                    product: QvmProduct::Baseq3,
                    reference_package: "pak0.pk3".to_string(),
                    related_game_build_date: "date".to_string(),
                    byte_length: 1,
                    digest: "sha256:x".to_string(),
                },
                implementation: ProviderId::new("test", "native"),
                create: Box::new(|_, _| -> QvmReplacementInstance { panic!("native factory must not run") }),
            },
        };
        assert!(matches!(
            convert_artifact(&resolved).unwrap_err(),
            QvmWeaponError::NativeArtifact
        ));
    }

    #[test]
    fn create_builds_ready_source() {
        let source = QvmWeaponBehaviorSource::create(options()).unwrap();
        assert_eq!(source.definition().id, "qvm:test");
        assert_eq!(source.definition().role, WeaponBehaviorRole::Rocket);
        let cvars = source.trap_cvars();
        assert_eq!(cvars.get("g_gametype").unwrap().value, "0");
        assert_eq!(cvars.get("sv_maxclients").unwrap().value, "64");
        drop(cvars);
        assert!(!source.profile_digest().is_empty());
        assert_eq!(source.profile_digest(), source.profile_digest());
    }

    #[test]
    fn host_call_honors_dispatch_first() {
        let source = ready();
        let fired = Rc::new(Cell::new(false));
        let probe = Rc::clone(&fired);
        source.shared.borrow_mut().assert_current = Rc::new(move || probe.set(true));
        source.shared.borrow_mut().traps = Rc::new(RefCell::new(FixedTraps(Some(7))));
        let weak = Rc::downgrade(&source.shared);
        let out = host_call(
            &source.game,
            &source.definition,
            &source.profile,
            &weak,
            &host_call_fixture(QvmGameImport::BotlibSetup as i32),
        )
        .unwrap();
        assert_eq!(out, Some(7));
        assert!(fired.get());
    }

    #[test]
    fn host_call_short_circuits_botlib() {
        let source = ready();
        let weak = Rc::downgrade(&source.shared);
        for code in [
            QvmGameImport::BotlibSetup as i32,
            QvmGameImport::BotlibAasInitialized as i32,
        ] {
            let out = host_call(
                &source.game,
                &source.definition,
                &source.profile,
                &weak,
                &host_call_fixture(code),
            )
            .unwrap();
            assert_eq!(out, Some(0));
        }
    }

    #[test]
    fn host_call_rejects_unbound_and_released() {
        let source = ready();
        let weak = Rc::downgrade(&source.shared);
        let error = host_call(
            &source.game,
            &source.definition,
            &source.profile,
            &weak,
            &host_call_fixture(0),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Unbound qagame QVM syscall 0"), "{error}");
        let mut cgame = host_call_fixture(5);
        cgame.role = GameRole::Cgame;
        let error = host_call(&source.game, &source.definition, &source.profile, &weak, &cgame).unwrap_err();
        assert!(error.to_string().contains("Unbound cgame QVM syscall 5"), "{error}");
        let game = source.game.clone();
        let definition = Rc::clone(&source.definition);
        let profile = Rc::clone(&source.profile);
        drop(source);
        let error = host_call(&game, &definition, &profile, &weak, &host_call_fixture(0)).unwrap_err();
        assert!(error.to_string().contains("released"), "{error}");
    }
    struct TimeProbe;

    impl QvmWeaponTraps for TimeProbe {
        fn dispatch(
            &mut self,
            _call: &QvmHostCall,
            source: &QvmWeaponBehaviorSource,
        ) -> Result<Option<i32>, GuestError> {
            assert_eq!(source.trap_time(), 0.0);
            let cvars = source.trap_cvars();
            assert_eq!(cvars.get("g_gametype").unwrap().value, "0");
            drop(cvars);
            assert_eq!(source.trap_configstrings().len(), 0);
            Ok(Some(1))
        }
    }

    #[test]
    fn trap_surface_exposes_state() {
        let source = ready();
        source.shared.borrow_mut().traps = Rc::new(RefCell::new(TimeProbe));
        let weak = Rc::downgrade(&source.shared);
        let out = host_call(
            &source.game,
            &source.definition,
            &source.profile,
            &weak,
            &host_call_fixture(9),
        )
        .unwrap();
        assert_eq!(out, Some(1));
    }

    #[test]
    fn attach_requires_ready_and_valid_launch() {
        let owner = IdentityOwner::create("attach").unwrap();
        let cold = staged();
        assert!(matches!(
            cold.attach(launch(&owner, WeaponBehaviorRole::Rocket)).unwrap_err(),
            QvmWeaponError::Busy
        ));
        let source = ready();
        assert!(matches!(
            source.attach(launch(&owner, WeaponBehaviorRole::Grenade)).unwrap_err(),
            QvmWeaponError::LaunchMismatch
        ));
        assert!(matches!(
            source.attach(launch(&owner, WeaponBehaviorRole::Rocket)).unwrap_err(),
            QvmWeaponError::MissingActor
        ));
    }

    #[test]
    fn instance_and_release_guards() {
        let owner = IdentityOwner::create("instance").unwrap();
        let source = ready();
        let actor = owner.actor(5, 0);
        assert!(matches!(
            source.resume(&actor).unwrap_err(),
            QvmWeaponError::MissingProjectile
        ));
        source.shared.borrow_mut().instances.insert(actor.clone());
        assert!(matches!(
            source.resume(&actor).unwrap_err(),
            QvmWeaponError::AlreadyAttached
        ));
        source.release(&owner.actor(9, 0)).unwrap();
    }

    #[test]
    fn slot_and_live_track_located_entities() {
        let source = ready();
        source.game.data.locate(1024, 2, 1024, 4096, 512).unwrap();
        assert_eq!(source.slot(1024).unwrap(), 0);
        assert_eq!(source.slot(2048).unwrap(), 1);
        assert!(source.slot(0).is_err());
        assert!(source.slot(-4).is_err());
        assert!(source.slot(1025).is_err());
        assert!(!source.live(1024).unwrap());
        source.game.data.entity_bytes(0).unwrap().set_i32(516, 1).unwrap();
        assert!(source.live(1024).unwrap());
        assert_eq!(source.actor_for_slot(0).unwrap(), None);
        let owner = IdentityOwner::create("slot").unwrap();
        assert!(matches!(
            source.mirror(&owner.actor(1, 0)).unwrap_err(),
            QvmWeaponError::MissingActor
        ));
    }

    #[test]
    fn profile_values_roundtrip() {
        for value in [
            ProfileValue::Null,
            ProfileValue::Bool(true),
            ProfileValue::Int(-42),
            ProfileValue::Float(1.5),
            ProfileValue::Str("hi".to_string()),
            ProfileValue::Bytes(vec![1, 2, 3]),
            ProfileValue::Array(vec![ProfileValue::Int(1), ProfileValue::Bool(false)]),
            ProfileValue::Record(vec![("a".to_string(), ProfileValue::Int(2))]),
        ] {
            let json = capture_profile_value(&value);
            let reader = SaveReader::new(&json);
            assert_eq!(read_profile_value(&reader).unwrap(), value);
        }
        let json = capture_profile_value(&ProfileValue::Undefined);
        let reader = SaveReader::new(&json);
        assert_eq!(read_profile_value(&reader).unwrap(), ProfileValue::Null);
    }

    struct EchoDefinitions(WeaponBehaviorDefinition);

    impl WeaponBehaviorDefinitionReader for EchoDefinitions {
        fn read_definition(
            &self,
            _reader: SaveReader<'_>,
            _expected: &WeaponBehaviorDefinition,
        ) -> Result<WeaponBehaviorDefinition, WorldError> {
            Ok(self.0.clone())
        }
    }

    fn checkpoint_fixture() -> QvmWeaponBehaviorCheckpoint {
        let module = qa_guest::qvm::game_data::ModuleIdentity {
            id: "test:qagame".to_string(),
            artifact_path: "vm/qagame.qvm".to_string(),
            digest: "sha256:abc123".to_string(),
            revision: "r1".to_string(),
        };
        QvmWeaponBehaviorCheckpoint {
            version: 1,
            definition: definition(),
            module: QvmCheckpoint {
                module: module.clone(),
                api_kind: "q3-qagame".to_string(),
                api_version: 8,
                abi_profile: AbiProfile::Modern,
                data: vec![1, 2, 3],
                instruction_index: 4,
                operand_stack: vec![5, 6],
                program_stack: 7,
                host_state: QvmHostState {
                    module,
                    format: "test".to_string(),
                    bytes: ProfileValue::Record(vec![("a".to_string(), ProfileValue::Int(2))]),
                },
            },
            host: QvmWeaponHostCheckpoint {
                mode: SimulationMode::Deathmatch,
                team_mode: false,
                time: 1.5,
                data: QvmGameDataState {
                    entities_word: 0,
                    num_entities: 0,
                    entity_stride: 0,
                    clients_word: 0,
                    client_stride: 0,
                },
                cvars: QvmCvarSaveState {
                    dialect: "q3".to_string(),
                    variables: vec![CvarSnapshot {
                        name: "g_gametype".to_string(),
                        value: "0".to_string(),
                        reset_value: "0".to_string(),
                        latched_value: None,
                        flags: 0,
                        modified: true,
                        modification_count: 1,
                        numeric_value: 0.0,
                        integer_value: 0,
                    }],
                    order: vec!["g_gametype".to_string()],
                    changed_flags: 0,
                    userinfo_dirty: false,
                },
                configstrings: vec![QvmConfigstring {
                    index: 0,
                    value: "cs0".to_string(),
                }],
                files: FilesCheckpoint { handles: Vec::new() },
                entity_text: b"{ \"classname\" \"worldspawn\" }".to_vec(),
                userinfo: vec![QvmUserinfo {
                    slot: 0,
                    value: "ui0".to_string(),
                }],
                cursor: Some(3),
                parser: ProfileValue::Null,
            },
            profile_digest: staged().profile_digest(),
            activated: vec![SavedActorId { slot: 1, generation: 0 }],
            bindings: vec![QvmWeaponBinding {
                actor: SavedActorId { slot: 5, generation: 0 },
                pointer: 1024,
                kind: QvmWeaponBindingKind::Projectile,
            }],
            retired: vec![QvmWeaponRetired {
                actor: SavedActorId { slot: 6, generation: 0 },
                trajectory: WeaponTrajectoryUpdate {
                    origin: vec3(1.0, 2.0, 3.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    angles: vec3(0.0, 0.0, 0.0),
                },
            }],
        }
    }

    #[test]
    fn checkpoint_capture_roundtrips() {
        let checkpoint = checkpoint_fixture();
        let json = capture_qvm_weapon_behavior_checkpoint(&checkpoint);
        let reader = SaveReader::new(&json);
        let back = read_qvm_weapon_behavior_checkpoint(
            &reader,
            &checkpoint.definition,
            &EchoDefinitions(checkpoint.definition.clone()),
        )
        .unwrap();
        assert_eq!(back.version, 1);
        assert_eq!(back.profile_digest, checkpoint.profile_digest);
        assert_eq!(back.module.api_kind, "q3-qagame");
        assert_eq!(back.module.api_version, 8);
        assert_eq!(back.module.data, vec![1, 2, 3]);
        assert_eq!(back.module.instruction_index, 4);
        assert_eq!(back.module.operand_stack, vec![5, 6]);
        assert_eq!(back.module.program_stack, 7);
        assert_eq!(back.host.time, 1.5);
        assert!(!back.host.team_mode);
        assert_eq!(back.host.cvars.dialect, "q3");
        assert_eq!(back.host.cvars.variables, checkpoint.host.cvars.variables);
        assert_eq!(back.host.cvars.order, checkpoint.host.cvars.order);
        assert_eq!(back.host.configstrings, checkpoint.host.configstrings);
        assert_eq!(back.host.userinfo, checkpoint.host.userinfo);
        assert_eq!(back.host.entity_text, checkpoint.host.entity_text);
        assert_eq!(back.host.cursor, Some(3));
        assert_eq!(back.activated, checkpoint.activated);
        assert_eq!(back.bindings, checkpoint.bindings);
        assert_eq!(back.retired.len(), 1);
        assert_eq!(back.retired[0].actor, checkpoint.retired[0].actor);
        assert_eq!(back.retired[0].trajectory.origin, vec3(1.0, 2.0, 3.0));
    }

    #[test]
    fn checkpoint_and_restore_preserve_host() {
        let owner = IdentityOwner::create("restore").unwrap();
        let source = QvmWeaponBehaviorSource::create(options()).unwrap();
        source.game.data.locate(1024, 2, 1024, 4096, 512).unwrap();
        source.set_time(1.5).unwrap();
        source.trap_configstrings().insert(2, "hello".to_string());
        let checkpoint = source.checkpoint().unwrap();
        assert_eq!(checkpoint.host.time, 1.5);
        source.set_time(2.0).unwrap();
        source.trap_configstrings().insert(2, "changed".to_string());
        let resolve = |saved: &SavedActorId| owner.actor(saved.slot, saved.generation);
        source.restore(&checkpoint, &resolve).unwrap();
        assert_eq!(source.trap_time(), 1.5);
        let strings = source.trap_configstrings();
        assert_eq!(strings.get(&2).unwrap(), "hello");
        drop(strings);
        assert_eq!(source.shared.borrow().generation, 1);
    }

    #[test]
    fn cvar_state_roundtrips() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        cvars.register("g_log", "", 0).unwrap();
        cvars.register("sv_maxclients", "64", 0).unwrap();
        cvars.set("sv_maxclients", "12", true).unwrap();
        let state = capture_cvar_state(&mut cvars);
        assert_eq!(state.dialect, "q3");
        let mut fresh = CvarRegistry::new(Dialect::Q3);
        restore_cvar_state(&mut fresh, &state).unwrap();
        let back = capture_cvar_state(&mut fresh);
        assert_eq!(back.order, state.order);
        for snapshot in &state.variables {
            let found = back.variables.iter().find(|entry| entry.name == snapshot.name).unwrap();
            assert_eq!(found.value, snapshot.value);
        }
    }
}
