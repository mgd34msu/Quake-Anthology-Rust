//! Primary player profiles: input, weapons, and combat declarations.
//!
//! Provenance: `src/compat/qvm/primary-player-profile.ts`.
//!
//! Absorbs the pure-Rust types of `src/contracts/qvm-combat.ts`.
//! `src/contracts/qvm-grapple.ts` is absorbed by the sibling
//! `grapple_profile` port (other worker); only the `grappleDamageMethod`
//! ordinal of that contract is referenced here. Local mirrors of game-owned
//! outputs: [`QvmInputDefinition`] (`game-input.ts`),
//! [`QvmPrimaryWeaponProfile`] (`game-weapons.ts`),
//! [`QvmPrimaryCombatProfile`] (`game-combat-binding.ts`,
//! `QvmGameArmorDefinition` in `game-combat.ts`),
//! [`QvmEquipmentMovementProfile`] (`game-equipment-movement.ts`),
//! [`SourcePrimaryMatch`] (`content/mods/match.ts`). Regions, images, and
//! dispatchers reuse [`super::mod_provider`] and [`super::mod_weapon_stage`];
//! weapon catalog rows mirror the `{ weapon, item }` projection of the
//! item-catalog owner.

use std::collections::{BTreeMap, HashSet};

use super::mod_provider::{
    namespaced_id, qualify_qvm_region, qualify_qvm_region_evaluation, qvm_player_state_bytes, qvm_shared_entity_bytes,
    InputPointerKind, ModuleId, ProfileReader, ProfileValue, QvmAbi, QvmArtifact, QvmModInputPointer, QvmOpcode,
    QvmRegionEvaluation, QVM_MAX_PRIVATE_ARGUMENT_WORDS,
};
use super::mod_weapon_stage::{
    validate_qvm_weapon_dispatcher, QvmItemField, QvmItemTest, QvmWeaponDispatcherDefinition, StagePredicate,
    StageRequest, StageSelection, TestComparison,
};
use crate::error::GuestError;

// ---------------------------------------------------------------------------
// Combat contract types (`src/contracts/qvm-combat.ts`).
// ---------------------------------------------------------------------------

/// Damage-call role names.
pub const QVM_DAMAGE_ROLES: [&str; 8] = [
    "target",
    "inflictor",
    "attacker",
    "direction",
    "point",
    "amount",
    "flags",
    "method",
];
/// Armor-call role names.
pub const QVM_ARMOR_ROLES: [&str; 3] = ["target", "amount", "flags"];

/// Extra word kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmCombatExtraKind {
    /// Signed word.
    Int32,
    /// Binary32 word.
    Float32,
    /// Address word.
    Address,
}

/// One extra call word.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmCombatExtra {
    /// Argument position.
    pub index: usize,
    /// Kind.
    pub kind: QvmCombatExtraKind,
    /// Value.
    pub value: f64,
}

/// Declared combat call: role positions plus extra words.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmCombatCall {
    /// Role positions.
    pub roles: BTreeMap<String, usize>,
    /// Extra words.
    pub extras: Vec<QvmCombatExtra>,
}

impl QvmCombatCall {
    /// Build a damage call.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn damage(
        target: usize,
        inflictor: usize,
        attacker: usize,
        direction: usize,
        point: usize,
        amount: usize,
        flags: usize,
        method: usize,
        extras: Vec<QvmCombatExtra>,
    ) -> Self {
        Self {
            roles: BTreeMap::from([
                ("target".to_string(), target),
                ("inflictor".to_string(), inflictor),
                ("attacker".to_string(), attacker),
                ("direction".to_string(), direction),
                ("point".to_string(), point),
                ("amount".to_string(), amount),
                ("flags".to_string(), flags),
                ("method".to_string(), method),
            ]),
            extras,
        }
    }

    /// Build an armor call.
    #[must_use]
    pub fn armor(target: usize, amount: usize, flags: usize, extras: Vec<QvmCombatExtra>) -> Self {
        Self {
            roles: BTreeMap::from([
                ("target".to_string(), target),
                ("amount".to_string(), amount),
                ("flags".to_string(), flags),
            ]),
            extras,
        }
    }
}

/// Validate combat argument positions cover each word exactly once.
pub fn validate_qvm_combat_positions(positions: &[usize], words: usize) -> Result<(), GuestError> {
    if words < positions.len() || words > QVM_MAX_PRIVATE_ARGUMENT_WORDS {
        return Err(GuestError::invalid(
            "Source combat call exceeds the QVM OP_ARG capacity or omits required arguments",
        ));
    }
    if positions.iter().collect::<HashSet<_>>().len() != positions.len()
        || positions.iter().any(|index| *index >= words)
    {
        return Err(GuestError::invalid(
            "Source combat argument positions must cover each declared role exactly once within the original call",
        ));
    }
    Ok(())
}

/// Validate one combat call against its artifact data.
pub fn validate_qvm_combat_call(call: &QvmCombatCall, data_bytes: usize) -> Result<(), GuestError> {
    let positions: Vec<usize> = call
        .roles
        .values()
        .copied()
        .chain(call.extras.iter().map(|extra| extra.index))
        .collect();
    let words = positions.len();
    validate_qvm_combat_positions(&positions, words)?;
    for extra in &call.extras {
        match extra.kind {
            QvmCombatExtraKind::Float32 => {
                if !extra.value.is_finite() || !(extra.value as f32).is_finite() {
                    return Err(GuestError::invalid(
                        "Source combat extra requires a finite binary32 value",
                    ));
                }
            }
            QvmCombatExtraKind::Address => {
                if extra.value.fract() != 0.0 || extra.value < 0.0 || extra.value >= data_bytes as f64 {
                    return Err(GuestError::invalid(
                        "Source combat extra address is outside its artifact data",
                    ));
                }
            }
            QvmCombatExtraKind::Int32 => {
                if extra.value.fract() != 0.0 || extra.value < f64::from(i32::MIN) || extra.value > f64::from(i32::MAX)
                {
                    return Err(GuestError::invalid(
                        "Source combat extra requires a signed integer word",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Declared reaction call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmReactionCall {
    /// Argument count.
    pub arguments: usize,
    /// Target position.
    pub target: usize,
    /// Amount position.
    pub amount: usize,
}

/// Damage flag masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmDamageFlags {
    /// Radius mask.
    pub radius: u32,
    /// No-armor mask.
    pub no_armor: u32,
    /// No-knockback mask.
    pub no_knockback: u32,
    /// No-protection mask.
    pub no_protection: u32,
    /// No-team-protection mask.
    pub no_team_protection: u32,
}

/// Mass source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QvmCombatMass {
    /// Constant mass.
    Constant(f64),
    /// Entity field mass.
    Entity {
        /// Field offset.
        offset: usize,
        /// Storage encoding name.
        storage: QvmMassStorage,
    },
}

/// Mass storage encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmMassStorage {
    /// Signed word.
    Int32,
    /// Binary32 word.
    Float32,
}

/// Team mapping.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmCombatTeam {
    /// Source value.
    pub value: i32,
    /// Shared identity.
    pub team: String,
}

// ---------------------------------------------------------------------------
// Shared reader helpers.
// ---------------------------------------------------------------------------

fn read_integer(reader: &ProfileReader<'_>, minimum: i64, maximum: i64) -> Result<i64, GuestError> {
    let value = reader.integer(minimum)?;
    if value > maximum {
        return reader.fail("value exceeds its source range");
    }
    Ok(value)
}

fn read_entry(reader: &ProfileReader<'_>, artifact: &QvmArtifact) -> Result<usize, GuestError> {
    let value = reader.integer(0)? as usize;
    if artifact
        .image
        .instruction(value)
        .is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
    {
        return reader.fail("not an original function entry");
    }
    Ok(value)
}

fn read_aligned(reader: &ProfileReader<'_>, bytes: usize, size: usize) -> Result<usize, GuestError> {
    let value = read_integer(reader, 0, bytes.saturating_sub(size) as i64)? as usize;
    if value % 4 != 0 {
        return reader.fail("source word is unaligned");
    }
    Ok(value)
}

fn read_mask(reader: &ProfileReader<'_>) -> Result<u32, GuestError> {
    let value = read_integer(reader, 1, 0xffff_ffff)? as u32;
    if value & value.wrapping_sub(1) != 0 {
        return reader.fail("expected an individual source flag");
    }
    Ok(value)
}

fn read_fraction(reader: &ProfileReader<'_>) -> Result<f64, GuestError> {
    let value = reader.finite()?;
    if value < 0.0 || value > 1.0 || f64::from(value as f32) != value {
        return reader.fail("expected an original binary32 armor fraction");
    }
    Ok(value)
}

fn read_region(reader: &ProfileReader<'_>) -> Result<RegionRef, GuestError> {
    Ok(RegionRef {
        entry: reader.field("entry")?.integer(0)? as usize,
        join: reader.field("join")?.integer(0)? as usize,
    })
}

fn read_evaluation(reader: &ProfileReader<'_>) -> Result<QvmRegionEvaluation, GuestError> {
    Ok(QvmRegionEvaluation {
        entry: reader.field("entry")?.integer(0)? as usize,
        join: reader.field("join")?.integer(0)? as usize,
        inputs: reader
            .field("inputs")?
            .list(|value| value.integer(0).map(|offset| offset as usize))?,
        result: reader
            .field("result")?
            .nullable(|value| value.integer(0).map(|offset| offset as usize))?,
    })
}

fn read_source_pointer(reader: &ProfileReader<'_>, data_bytes: usize) -> Result<QvmModInputPointer, GuestError> {
    let kind = reader.field("kind")?.choice(&["argument", "global"])?;
    let offset = reader.field("offset")?.integer(0)? as usize;
    let indirections = reader
        .field("indirections")?
        .list(|value| value.integer(0).map(|step| step as usize))?;
    if offset % 4 != 0 || indirections.iter().any(|step| step % 4 != 0) {
        return reader.fail("source pointer path must use aligned words");
    }
    if kind == "argument" {
        Ok(QvmModInputPointer {
            kind: InputPointerKind::Argument {
                index: read_integer(&reader.field("index")?, 0, QVM_MAX_PRIVATE_ARGUMENT_WORDS as i64 - 1)? as usize,
            },
            indirections,
            offset,
        })
    } else {
        Ok(QvmModInputPointer {
            kind: InputPointerKind::Global {
                address: read_aligned(&reader.field("address")?, data_bytes, 4)?,
            },
            indirections,
            offset,
        })
    }
}

/// Record strides shared by primary profiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimaryLayout {
    /// Module identity.
    pub module: ModuleId,
    /// ABI profile.
    pub abi_profile: QvmAbi,
    /// Entity stride.
    pub entity_stride: usize,
    /// Client stride.
    pub client_stride: usize,
}

fn read_layout(reader: &ProfileReader<'_>, artifact: &QvmArtifact) -> Result<PrimaryLayout, GuestError> {
    if artifact.role != super::mod_provider::QvmRole::Qagame {
        return reader.fail("primary player services require a qagame ABI");
    }
    let abi_profile = artifact.abi();
    let entity_stride = read_integer(
        &reader.field("entityStride")?,
        qvm_shared_entity_bytes(abi_profile) as i64,
        artifact.image.allocated_data_length as i64,
    )? as usize;
    let client_stride = read_integer(
        &reader.field("clientStride")?,
        qvm_player_state_bytes(abi_profile) as i64,
        artifact.image.allocated_data_length as i64,
    )? as usize;
    if entity_stride % 4 != 0 || client_stride % 4 != 0 {
        return reader.fail("source record strides must be aligned");
    }
    Ok(PrimaryLayout {
        module: artifact.module.clone(),
        abi_profile,
        entity_stride,
        client_stride,
    })
}

/// Entry/join reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegionRef {
    /// Entry instruction.
    pub entry: usize,
    /// Join instruction.
    pub join: usize,
}

// ---------------------------------------------------------------------------
// Input definition mirror (`game-input.ts`).
// ---------------------------------------------------------------------------

/// Input entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputEntries {
    /// Client-think entry.
    pub client_think: usize,
    /// Run-client entry.
    pub run_client: usize,
    /// Client-spawn entry.
    pub client_spawn: usize,
    /// Move entry.
    pub move_: usize,
    /// Slice entry.
    pub slice: usize,
}

/// Movement modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MovementModes {
    /// Normal mode.
    pub normal: i64,
    /// Noclip mode.
    pub noclip: i64,
    /// Freeze mode.
    pub freeze: i64,
}

/// Input definition (mirror of `QvmInputDefinition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmInputDefinition {
    /// Module identity.
    pub module: ModuleId,
    /// Entity stride.
    pub entity_stride: usize,
    /// Client stride.
    pub client_stride: usize,
    /// Client pointer.
    pub client_pointer: usize,
    /// Intermission values.
    pub intermission: Vec<i64>,
    /// Movement modes, if any.
    pub movement_modes: Option<MovementModes>,
    /// Entries.
    pub entries: InputEntries,
}

/// Read a primary input definition.
pub fn read_qvm_primary_input(
    reader: &ProfileReader<'_>,
    artifact: &QvmArtifact,
) -> Result<QvmInputDefinition, GuestError> {
    let common = read_layout(reader, artifact)?;
    let entries = reader.field("entries")?;
    let modes = reader.field("movementModes")?;
    let movement_modes = if modes.is_undefined() {
        None
    } else {
        Some(MovementModes {
            normal: modes.field("normal")?.integer(i64::MIN)?,
            noclip: modes.field("noclip")?.integer(i64::MIN)?,
            freeze: modes.field("freeze")?.integer(i64::MIN)?,
        })
    };
    Ok(QvmInputDefinition {
        module: common.module,
        entity_stride: common.entity_stride,
        client_stride: common.client_stride,
        client_pointer: read_aligned(&reader.field("clientPointer")?, common.entity_stride, 4)?,
        intermission: reader.field("intermission")?.list(|value| value.integer(i64::MIN))?,
        movement_modes,
        entries: InputEntries {
            client_think: read_entry(&entries.field("clientThink")?, artifact)?,
            run_client: read_entry(&entries.field("runClient")?, artifact)?,
            client_spawn: read_entry(&entries.field("clientSpawn")?, artifact)?,
            move_: read_entry(&entries.field("move")?, artifact)?,
            slice: read_entry(&entries.field("slice")?, artifact)?,
        },
    })
}

/// Whether two input definitions share their primary interface.
#[must_use]
pub fn same_qvm_primary_input(left: &QvmInputDefinition, right: &QvmInputDefinition) -> bool {
    left.client_stride == right.client_stride
        && left.entity_stride == right.entity_stride
        && left.client_pointer == right.client_pointer
        && left.intermission == right.intermission
        && left.module.same_module(&right.module)
        && left.entries == right.entries
}

// ---------------------------------------------------------------------------
// Weapon profile mirror (`game-weapons.ts`).
// ---------------------------------------------------------------------------

/// Equipment movement profile (mirror of `QvmEquipmentMovementProfile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmEquipmentMovementProfile {
    /// Move entry.
    pub move_: usize,
    /// Slice entry.
    pub slice: usize,
    /// Duck entry.
    pub duck: usize,
    /// Movement global.
    pub movement_global: usize,
    /// Locomotion region.
    pub locomotion: RegionRef,
    /// Minimums address.
    pub mins: usize,
    /// Maximums address.
    pub maxs: usize,
    /// Body-trace callback and mask, if any.
    pub body_trace: Option<BodyTrace>,
}

/// Body-trace callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BodyTrace {
    /// Callback address.
    pub callback: usize,
    /// Mask address.
    pub mask: usize,
}

/// Equipment context.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceEquipmentContext {
    /// Provider identity.
    pub provider: String,
    /// Item identity, if any.
    pub item: Option<String>,
}

/// Match team change.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MatchTeam {
    /// Source identity, if any.
    pub source: Option<String>,
    /// Shared identity, if any.
    pub team: Option<String>,
    /// Command arguments.
    pub arguments: Vec<String>,
}

/// Primary match (mirror of `SourcePrimaryMatch`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePrimaryMatch {
    /// Score offset.
    pub score: usize,
    /// Team changes.
    pub teams: Vec<MatchTeam>,
}

/// Read a primary match (mirror of `readSourcePrimaryMatch`).
pub fn read_source_primary_match(
    reader: &ProfileReader<'_>,
    client_bytes: usize,
) -> Result<SourcePrimaryMatch, GuestError> {
    let score = read_integer(&reader.field("score")?, 0, i64::MAX)? as usize;
    let teams = reader.field("teams")?.list(|value| {
        Ok(MatchTeam {
            source: value.field("source")?.nullable(|field| field.string())?,
            team: value.field("team")?.nullable(|field| field.string())?,
            arguments: value.field("arguments")?.list(|field| field.string())?,
        })
    })?;
    if score % 4 != 0 || score + 4 > client_bytes {
        return reader
            .field("score")?
            .fail("score is outside the original client record");
    }
    if teams
        .iter()
        .map(|value| value.source.clone())
        .collect::<HashSet<_>>()
        .len()
        != teams.len()
        || teams
            .iter()
            .map(|value| value.team.clone())
            .collect::<HashSet<_>>()
            .len()
            != teams.len()
        || teams.iter().any(|value| {
            value.arguments.is_empty()
                || value
                    .arguments
                    .iter()
                    .any(|argument| argument.is_empty() || argument.contains(['\0', '\r', '\n']))
        })
    {
        return reader
            .field("teams")?
            .fail("team changes require distinct identities and complete original command arguments");
    }
    Ok(SourcePrimaryMatch { score, teams })
}

/// Damage factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DamageFactor {
    /// Entry instruction.
    pub entry: usize,
    /// Result global.
    pub result: usize,
    /// Stop region.
    pub stop: RegionRef,
}

/// Delay player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DelayPlayer {
    /// Movement global.
    pub movement_global: usize,
    /// Player offset.
    pub player_offset: usize,
}

/// Teleport profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeleportProfile {
    /// Entry instruction.
    pub entry: usize,
    /// Region evaluation.
    pub region: QvmRegionEvaluation,
    /// Objectives evaluation.
    pub objectives: QvmRegionEvaluation,
    /// Spawn entry.
    pub spawn: usize,
    /// View entry.
    pub view: usize,
}

/// Weapon availability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponAvailability {
    /// Movement type.
    pub movement_type: usize,
    /// Excluded values.
    pub excluded: Vec<i64>,
    /// Health offset.
    pub health: usize,
    /// Team offset.
    pub team: usize,
    /// Spectator team.
    pub spectator_team: i64,
    /// Flags offset.
    pub flags: usize,
    /// Respawn flag.
    pub respawn_flag: i32,
}

/// Powerup offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PowerupOffsets {
    /// Quad offset.
    pub quad: usize,
    /// Haste offset.
    pub haste: usize,
    /// Flight offset.
    pub flight: usize,
}

/// Torso animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TorsoAnimation {
    /// Entry instruction.
    pub entry: usize,
    /// Attack value.
    pub attack: usize,
    /// Melee value.
    pub melee: usize,
}

/// Water level offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WaterLevel {
    /// Entity offset.
    pub entity_offset: usize,
    /// Movement offset.
    pub movement_offset: usize,
}

/// Drop ammo source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropAmmo {
    /// Private inventory projection.
    Inventory,
    /// Client offset.
    Offset(usize),
}

/// Drop profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DropProfile {
    /// Entry instruction.
    pub entry: usize,
    /// Argument index.
    pub argument: usize,
    /// Weapon offset.
    pub weapon: usize,
    /// Ammo source.
    pub ammo: DropAmmo,
    /// Region.
    pub region: RegionRef,
}

/// Named grant region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NamedGrant {
    /// Entry instruction.
    pub entry: usize,
    /// Join instruction.
    pub join: usize,
    /// Name local.
    pub name: usize,
    /// Item local.
    pub item: usize,
}

/// Give profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GiveProfile {
    /// Entry instruction.
    pub entry: usize,
    /// Argument index.
    pub argument: usize,
    /// Weapons decision.
    pub weapons: usize,
    /// Ammo decision.
    pub ammo: usize,
    /// Named grant.
    pub named: NamedGrant,
}

/// Primary weapon profile (mirror of `QvmPrimaryWeaponProfile`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPrimaryWeaponProfile {
    /// Module identity.
    pub module: ModuleId,
    /// Match, if any.
    pub match_: Option<SourcePrimaryMatch>,
    /// ABI profile.
    pub abi_profile: QvmAbi,
    /// Equipment movement.
    pub equipment_movement: QvmEquipmentMovementProfile,
    /// Entity stride.
    pub entity_stride: usize,
    /// Client stride.
    pub client_stride: usize,
    /// Client pointer.
    pub client_pointer: usize,
    /// Weapon stage.
    pub stage: QvmWeaponDispatcherDefinition,
    /// Damage factor.
    pub damage_factor: DamageFactor,
    /// Equipment contexts.
    pub equipment_contexts: Vec<SourceEquipmentContext>,
    /// Delay evaluation.
    pub delay: QvmRegionEvaluation,
    /// Delay player.
    pub delay_player: DelayPlayer,
    /// Teleport profile.
    pub teleport: TeleportProfile,
    /// Maximum health offset.
    pub max_health: usize,
    /// Persistent maximum health offset.
    pub persistent_max_health: usize,
    /// Availability.
    pub availability: WeaponAvailability,
    /// Powerups.
    pub powerups: PowerupOffsets,
    /// Torso animation.
    pub torso_animation: TorsoAnimation,
    /// Water level.
    pub water_level: WaterLevel,
    /// Drop profile.
    pub drop: DropProfile,
    /// Give profile.
    pub give: GiveProfile,
}

/// Weapon catalog row (`{ weapon, item }` projection of the item catalog).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmWeaponCatalogRow {
    /// Weapon number.
    pub weapon: i32,
    /// Item identity.
    pub item: String,
}

/// Read a primary weapon profile.
pub fn read_qvm_primary_weapons(
    reader: &ProfileReader<'_>,
    artifact: &QvmArtifact,
    catalog: Option<&[QvmWeaponCatalogRow]>,
) -> Result<QvmPrimaryWeaponProfile, GuestError> {
    let common = read_layout(reader, artifact)?;
    let data_bytes = artifact.image.data_length + artifact.image.literal_length + artifact.image.bss_length;
    let allocated = artifact.image.allocated_data_length;
    let stage = reader.field("stage")?;
    let dispatcher = stage.field("dispatcher")?;
    let selection = stage.field("selection")?;
    let request = stage.field("request")?;
    let actor = dispatcher.field("actor")?;
    let movement = reader.field("equipmentMovement")?;
    let availability = reader.field("availability")?;
    let powers = reader.field("powerups")?;
    let torso = reader.field("torsoAnimation")?;
    let equipment_contexts = reader.field("equipmentContexts")?.list(|value| {
        Ok(SourceEquipmentContext {
            provider: namespaced_id(&value.field("provider")?)?,
            item: value.field("item")?.nullable(namespaced_id)?,
        })
    })?;
    if equipment_contexts
        .iter()
        .map(|context| context.provider.clone())
        .collect::<HashSet<_>>()
        .len()
        != equipment_contexts.len()
    {
        return reader
            .field("equipmentContexts")?
            .fail("duplicate equipment source context");
    }
    let damage = reader.field("damageFactor")?;
    let delay_player = reader.field("delayPlayer")?;
    let water = reader.field("waterLevel")?;
    let teleport = reader.field("teleport")?;
    let drop = reader.field("drop")?;
    let give = reader.field("give")?;
    let named = give.field("named")?;
    let read_test = |value: &ProfileReader<'_>| -> Result<QvmItemTest, GuestError> {
        let field = value.field("field")?;
        Ok(QvmItemTest {
            field: QvmItemField {
                record: field.field("record")?.literal_str("client")?,
                offset: read_aligned(&field.field("offset")?, common.client_stride, 4)?,
            },
            mask: value
                .field("mask")?
                .nullable(|mask| read_integer(mask, 0, 0xffff_ffff).map(|mask| mask as u32))?,
            comparison: match value.field("comparison")?.choice(&["equals", "at-most"])?.as_str() {
                "equals" => TestComparison::Equals,
                _ => TestComparison::AtMost,
            },
            value: read_integer(&value.field("value")?, i64::from(i32::MIN), i64::from(i32::MAX))? as i32,
        })
    };
    let selection_field = selection.field("field")?;
    let profile = QvmPrimaryWeaponProfile {
        module: common.module.clone(),
        match_: if reader.field("match")?.is_undefined() {
            None
        } else {
            Some(read_source_primary_match(
                &reader.field("match")?,
                common.client_stride,
            )?)
        },
        abi_profile: common.abi_profile,
        entity_stride: common.entity_stride,
        client_stride: common.client_stride,
        client_pointer: read_aligned(&reader.field("clientPointer")?, common.entity_stride, 4)?,
        max_health: read_aligned(&reader.field("maxHealth")?, common.client_stride, 4)?,
        persistent_max_health: read_aligned(&reader.field("persistentMaxHealth")?, common.client_stride, 4)?,
        stage: QvmWeaponDispatcherDefinition {
            dispatcher: super::mod_weapon_stage::DispatcherHead {
                entry: read_entry(&dispatcher.field("entry")?, artifact)?,
                actor: super::mod_weapon_stage::QvmWeaponActor {
                    record: actor.field("record")?.literal_str("client")?,
                    pointer: read_source_pointer(&actor.field("pointer")?, data_bytes)?,
                },
            },
            predicates: stage.field("predicates")?.list(|value| {
                Ok(StagePredicate {
                    instruction: value.field("instruction")?.integer(0)? as usize,
                    unselected: value.field("unselected")?.boolean()?,
                })
            })?,
            settled: stage.field("settled")?.list(&read_test)?,
            selection: StageSelection {
                field: QvmItemField {
                    record: selection_field.field("record")?.literal_str("client")?,
                    offset: read_aligned(&selection_field.field("offset")?, common.client_stride, 4)?,
                },
                values: selection.field("values")?.list(|value| {
                    Ok(super::mod_weapon_stage::SelectionValue {
                        value: read_integer(&value.field("value")?, 1, 0x7fff_ffff)? as i32,
                        item: namespaced_id(&value.field("item")?)?,
                    })
                })?,
            },
            request: StageRequest {
                entry: read_entry(&request.field("entry")?, artifact)?,
                argument: read_integer(
                    &request.field("argument")?,
                    0,
                    QVM_MAX_PRIVATE_ARGUMENT_WORDS as i64 - 1,
                )? as usize,
                accepted: request.field("accepted")?.list(&read_test)?,
            },
        },
        equipment_movement: QvmEquipmentMovementProfile {
            move_: read_entry(&movement.field("move")?, artifact)?,
            slice: read_entry(&movement.field("slice")?, artifact)?,
            duck: read_entry(&movement.field("duck")?, artifact)?,
            movement_global: read_aligned(&movement.field("movementGlobal")?, data_bytes, 4)?,
            locomotion: read_region(&movement.field("locomotion")?)?,
            mins: read_aligned(&movement.field("mins")?, allocated, 12)?,
            maxs: read_aligned(&movement.field("maxs")?, allocated, 12)?,
            body_trace: if movement.field("bodyTrace")?.is_undefined() {
                None
            } else {
                let trace = movement.field("bodyTrace")?;
                Some(BodyTrace {
                    callback: read_aligned(&trace.field("callback")?, allocated, 4)?,
                    mask: read_aligned(&trace.field("mask")?, allocated, 4)?,
                })
            },
        },
        availability: WeaponAvailability {
            movement_type: read_aligned(&availability.field("movementType")?, common.client_stride, 4)?,
            excluded: availability.field("excluded")?.list(|value| value.integer(i64::MIN))?,
            health: read_aligned(&availability.field("health")?, common.client_stride, 4)?,
            team: read_aligned(&availability.field("team")?, common.client_stride, 4)?,
            spectator_team: availability.field("spectatorTeam")?.integer(i64::MIN)?,
            flags: read_aligned(&availability.field("flags")?, common.client_stride, 4)?,
            respawn_flag: read_integer(&availability.field("respawnFlag")?, 1, 0x7fff_ffff)? as i32,
        },
        powerups: PowerupOffsets {
            quad: read_aligned(&powers.field("quad")?, common.client_stride, 4)?,
            haste: read_aligned(&powers.field("haste")?, common.client_stride, 4)?,
            flight: read_aligned(&powers.field("flight")?, common.client_stride, 4)?,
        },
        torso_animation: TorsoAnimation {
            entry: read_entry(&torso.field("entry")?, artifact)?,
            attack: read_integer(&torso.field("attack")?, 0, i64::MAX)? as usize,
            melee: read_integer(&torso.field("melee")?, 0, i64::MAX)? as usize,
        },
        water_level: WaterLevel {
            entity_offset: read_aligned(&water.field("entityOffset")?, common.entity_stride, 4)?,
            movement_offset: read_aligned(&water.field("movementOffset")?, allocated, 4)?,
        },
        damage_factor: DamageFactor {
            entry: read_entry(&damage.field("entry")?, artifact)?,
            result: read_aligned(&damage.field("result")?, data_bytes, 4)?,
            stop: read_region(&damage.field("stop")?)?,
        },
        equipment_contexts,
        delay: read_evaluation(&reader.field("delay")?)?,
        delay_player: DelayPlayer {
            movement_global: read_aligned(&delay_player.field("movementGlobal")?, data_bytes, 4)?,
            player_offset: read_aligned(&delay_player.field("playerOffset")?, allocated, 4)?,
        },
        teleport: TeleportProfile {
            entry: read_entry(&teleport.field("entry")?, artifact)?,
            region: read_evaluation(&teleport.field("region")?)?,
            objectives: read_evaluation(&teleport.field("objectives")?)?,
            spawn: read_entry(&teleport.field("spawn")?, artifact)?,
            view: read_entry(&teleport.field("view")?, artifact)?,
        },
        drop: DropProfile {
            entry: read_entry(&drop.field("entry")?, artifact)?,
            argument: read_integer(&drop.field("argument")?, 0, QVM_MAX_PRIVATE_ARGUMENT_WORDS as i64 - 1)? as usize,
            weapon: read_aligned(&drop.field("weapon")?, common.entity_stride, 4)?,
            ammo: if drop.field("ammo")?.value() == &ProfileValue::Str("inventory".to_string()) {
                drop.field("ammo")?.literal_str("inventory")?;
                DropAmmo::Inventory
            } else {
                DropAmmo::Offset(read_aligned(&drop.field("ammo")?, common.client_stride, 64)?)
            },
            region: read_region(&drop.field("region")?)?,
        },
        give: GiveProfile {
            entry: read_entry(&give.field("entry")?, artifact)?,
            argument: read_integer(&give.field("argument")?, 0, QVM_MAX_PRIVATE_ARGUMENT_WORDS as i64 - 1)? as usize,
            weapons: read_integer(&give.field("weapons")?, 0, i64::MAX)? as usize,
            ammo: read_integer(&give.field("ammo")?, 0, i64::MAX)? as usize,
            named: NamedGrant {
                entry: named.field("entry")?.integer(0)? as usize,
                join: named.field("join")?.integer(0)? as usize,
                name: named.field("name")?.integer(0)? as usize,
                item: named.field("item")?.integer(0)? as usize,
            },
        },
    };
    if profile
        .stage
        .selection
        .values
        .iter()
        .map(|value| value.value)
        .collect::<HashSet<_>>()
        .len()
        != profile.stage.selection.values.len()
        || profile
            .stage
            .selection
            .values
            .iter()
            .map(|value| value.item.clone())
            .collect::<HashSet<_>>()
            .len()
            != profile.stage.selection.values.len()
    {
        return selection.fail("weapon selection repeats source values or identities");
    }
    if let Some(catalog) = catalog {
        if profile.stage.selection.values.len() != catalog.len()
            || profile.stage.selection.values.iter().any(|value| {
                !catalog
                    .iter()
                    .any(|item| item.weapon == value.value && item.item == value.item)
            })
        {
            return selection.fail("weapon selection differs from the original item catalog");
        }
    }
    if let DropAmmo::Offset(ammo) = profile.drop.ammo {
        if profile
            .stage
            .selection
            .values
            .iter()
            .any(|value| ammo + value.value as usize * 4 + 4 > common.client_stride)
        {
            return drop.fail("original drop ammo indexing exceeds its declared client record");
        }
    }
    validate_qvm_weapon_dispatcher(&profile.stage, &artifact.image)?;
    qualify_qvm_region(
        &artifact.image.instructions,
        profile.equipment_movement.slice,
        profile.equipment_movement.locomotion.entry,
        profile.equipment_movement.locomotion.join,
    )?;
    qualify_qvm_region(
        &artifact.image.instructions,
        profile.damage_factor.entry,
        profile.damage_factor.stop.entry,
        profile.damage_factor.stop.join,
    )?;
    qualify_qvm_region_evaluation(
        &artifact.image.instructions,
        profile.stage.dispatcher.entry,
        &profile.delay,
        false,
    )?;
    qualify_qvm_region_evaluation(
        &artifact.image.instructions,
        profile.teleport.entry,
        &profile.teleport.region,
        false,
    )?;
    qualify_qvm_region_evaluation(
        &artifact.image.instructions,
        profile.teleport.entry,
        &profile.teleport.objectives,
        false,
    )?;
    qualify_qvm_region(
        &artifact.image.instructions,
        profile.give.entry,
        profile.give.named.entry,
        profile.give.named.join,
    )?;
    qualify_qvm_region(
        &artifact.image.instructions,
        profile.drop.entry,
        profile.drop.region.entry,
        profile.drop.region.join,
    )?;
    if profile.delay.inputs.len() != 1
        || profile.delay.result.is_none()
        || !profile.teleport.region.inputs.is_empty()
        || !profile.teleport.objectives.inputs.is_empty()
    {
        return reader.fail("source effect region inputs differ from the primary player ABI");
    }
    let give_entry = artifact.image.instruction(profile.give.entry);
    if give_entry.is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter) {
        return give.fail("give entry disappeared");
    }
    let frame = give_entry.map_or(0, |instruction| instruction.operand.max(0) as usize);
    for offset in [profile.give.named.name, profile.give.named.item] {
        if offset < 8 || offset % 4 != 0 || offset + 4 > frame {
            return named.fail("named grant local exceeds its original function frame");
        }
    }
    let entries = [
        profile.stage.dispatcher.entry,
        profile.stage.request.entry,
        profile.equipment_movement.duck,
        profile.give.entry,
        profile.drop.entry,
        profile.damage_factor.entry,
        profile.teleport.entry,
    ];
    if entries.into_iter().collect::<HashSet<_>>().len() != entries.len() {
        return reader.fail("primary weapon interfaces overlap original function ownership");
    }
    for pc in [profile.give.weapons, profile.give.ammo] {
        let opcode = artifact.image.instruction(pc).map(|instruction| instruction.opcode);
        let mut owner = pc as i64;
        while owner >= 0
            && artifact
                .image
                .instruction(owner as usize)
                .is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
        {
            owner -= 1;
        }
        if owner != profile.give.entry as i64
            || opcode.is_none_or(|opcode| !opcode.is_branch())
            || pc <= profile.give.entry
        {
            return give.fail("give completion is not an original decision");
        }
    }
    Ok(profile)
}

// ---------------------------------------------------------------------------
// Combat profile mirror (`game-combat-binding.ts`).
// ---------------------------------------------------------------------------

/// Tier condition comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TierComparison {
    /// Equal.
    Equal,
    /// Not equal.
    NotEqual,
}

/// Armor tier condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TierCondition {
    /// Selector address.
    pub offset: usize,
    /// Comparison.
    pub comparison: TierComparison,
    /// Value.
    pub value: i32,
}

/// Armor tier value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TierValue {
    /// Tier.
    pub tier: i32,
    /// Protection fraction.
    pub protection: f64,
}

/// Armor tiers.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorTiers {
    /// Stat index.
    pub stat: usize,
    /// Conditions.
    pub when_any: Vec<TierCondition>,
    /// Values.
    pub values: Vec<TierValue>,
    /// Fallback fraction.
    pub fallback: f64,
}

/// Armor definition (mirror of `QvmGameArmorDefinition`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorDefinition {
    /// Check-armor entry.
    pub check_armor: usize,
    /// Armor call.
    pub call: QvmCombatCall,
    /// Points stat.
    pub points_stat: usize,
    /// Protection fraction.
    pub protection: f64,
    /// Tiers, if any.
    pub tiers: Option<ArmorTiers>,
}

/// Combat fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CombatFields {
    /// In-use offset.
    pub inuse: usize,
    /// Health offset.
    pub health: usize,
    /// Take-damage offset.
    pub takedamage: usize,
    /// Parent offset.
    pub parent: usize,
    /// Client pointer offset.
    pub client: usize,
}

/// Combat callbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CombatCallbacks {
    /// Allocate entry.
    pub allocate: usize,
    /// Free entry.
    pub free: usize,
    /// Damage entry.
    pub damage: usize,
}

/// Combat reactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CombatReactions {
    /// Flags offset.
    pub flags: usize,
    /// Pain offset.
    pub pain: usize,
    /// Die offset.
    pub die: usize,
    /// Pain call.
    pub pain_call: QvmReactionCall,
    /// Die call.
    pub die_call: QvmReactionCall,
}

/// Combat team state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatTeamState {
    /// Persistent stat.
    pub persistent_stat: usize,
    /// Values.
    pub values: Vec<QvmCombatTeam>,
}

/// Combat state flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CombatStateFlags {
    /// Notarget mask.
    pub notarget: u32,
    /// Invulnerable mask.
    pub invulnerable: u32,
    /// No-knockback mask.
    pub no_knockback: u32,
}

/// Combat state.
#[derive(Debug, Clone, PartialEq)]
pub struct CombatState {
    /// Health stat.
    pub health_stat: usize,
    /// Team state.
    pub team: CombatTeamState,
    /// Flags.
    pub flags: CombatStateFlags,
    /// Mass.
    pub mass: QvmCombatMass,
}

/// Primary combat profile (mirror of `QvmPrimaryCombatProfile`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPrimaryCombatProfile {
    /// Damage call.
    pub damage_call: QvmCombatCall,
    /// Module identity.
    pub module: ModuleId,
    /// ABI profile.
    pub abi_profile: QvmAbi,
    /// Entity stride.
    pub entity_stride: usize,
    /// Client stride.
    pub client_stride: usize,
    /// Fields.
    pub fields: CombatFields,
    /// Callbacks.
    pub callbacks: CombatCallbacks,
    /// Armor.
    pub armor: ArmorDefinition,
    /// Reactions.
    pub reactions: CombatReactions,
    /// Grapple damage method.
    pub grapple_damage_method: usize,
    /// State.
    pub state: CombatState,
    /// Damage flags.
    pub damage_flags: QvmDamageFlags,
}

fn read_reaction_call(reader: &ProfileReader<'_>) -> Result<QvmReactionCall, GuestError> {
    let roles = reader.field("roles")?;
    let result = QvmReactionCall {
        arguments: read_integer(&reader.field("arguments")?, 2, QVM_MAX_PRIVATE_ARGUMENT_WORDS as i64)? as usize,
        target: read_integer(&roles.field("target")?, 0, QVM_MAX_PRIVATE_ARGUMENT_WORDS as i64 - 1)? as usize,
        amount: read_integer(&roles.field("amount")?, 0, QVM_MAX_PRIVATE_ARGUMENT_WORDS as i64 - 1)? as usize,
    };
    validate_qvm_combat_positions(&[result.target, result.amount], result.arguments)
        .map_err(|error| GuestError::invalid(format!("reaction: {error}")))?;
    Ok(result)
}

fn read_combat_extras(reader: &ProfileReader<'_>) -> Result<Vec<QvmCombatExtra>, GuestError> {
    reader.list(|extra| {
        Ok(QvmCombatExtra {
            index: read_integer(&extra.field("index")?, 0, QVM_MAX_PRIVATE_ARGUMENT_WORDS as i64 - 1)? as usize,
            kind: match extra.field("kind")?.choice(&["int32", "float32", "address"])?.as_str() {
                "float32" => QvmCombatExtraKind::Float32,
                "address" => QvmCombatExtraKind::Address,
                _ => QvmCombatExtraKind::Int32,
            },
            value: extra.field("value")?.finite()?,
        })
    })
}

/// Read a primary combat profile.
pub fn read_qvm_primary_combat(
    reader: &ProfileReader<'_>,
    artifact: &QvmArtifact,
) -> Result<QvmPrimaryCombatProfile, GuestError> {
    let common = read_layout(reader, artifact)?;
    let fields = reader.field("fields")?;
    let callbacks = reader.field("callbacks")?;
    let reactions = reader.field("reactions")?;
    let armor = reader.field("armor")?;
    let tiers = armor.field("tiers")?;
    let data_bytes = artifact.image.data_length + artifact.image.literal_length + artifact.image.bss_length;
    let state = reader.field("state")?;
    let team = state.field("team")?;
    let flags = state.field("flags")?;
    let mass = state.field("mass")?;
    let damage_flags = reader.field("damageFlags")?;
    let damage_call = reader.field("damageCall")?;
    let damage_roles = damage_call.field("roles")?;
    let armor_call = armor.field("call")?;
    let armor_roles = armor_call.field("roles")?;
    let read_private = |value: &ProfileReader<'_>| -> Result<usize, GuestError> {
        let offset = read_aligned(value, common.entity_stride, 4)?;
        if offset < qvm_shared_entity_bytes(common.abi_profile) {
            return value.fail("private field overlaps the public entity ABI");
        }
        Ok(offset)
    };
    let read_position = |value: &ProfileReader<'_>| -> Result<usize, GuestError> {
        read_integer(value, 0, QVM_MAX_PRIVATE_ARGUMENT_WORDS as i64 - 1).map(|position| position as usize)
    };
    let result = QvmPrimaryCombatProfile {
        damage_call: QvmCombatCall::damage(
            read_position(&damage_roles.field("target")?)?,
            read_position(&damage_roles.field("inflictor")?)?,
            read_position(&damage_roles.field("attacker")?)?,
            read_position(&damage_roles.field("direction")?)?,
            read_position(&damage_roles.field("point")?)?,
            read_position(&damage_roles.field("amount")?)?,
            read_position(&damage_roles.field("flags")?)?,
            read_position(&damage_roles.field("method")?)?,
            read_combat_extras(&damage_call.field("extras")?)?,
        ),
        module: common.module.clone(),
        abi_profile: common.abi_profile,
        entity_stride: common.entity_stride,
        client_stride: common.client_stride,
        fields: CombatFields {
            inuse: read_private(&fields.field("inuse")?)?,
            health: read_private(&fields.field("health")?)?,
            takedamage: read_private(&fields.field("takedamage")?)?,
            parent: read_private(&fields.field("parent")?)?,
            client: read_private(&fields.field("client")?)?,
        },
        callbacks: CombatCallbacks {
            allocate: read_entry(&callbacks.field("allocate")?, artifact)?,
            free: read_entry(&callbacks.field("free")?, artifact)?,
            damage: read_entry(&callbacks.field("damage")?, artifact)?,
        },
        armor: ArmorDefinition {
            check_armor: read_entry(&armor.field("checkArmor")?, artifact)?,
            call: QvmCombatCall::armor(
                read_position(&armor_roles.field("target")?)?,
                read_position(&armor_roles.field("amount")?)?,
                read_position(&armor_roles.field("flags")?)?,
                read_combat_extras(&armor_call.field("extras")?)?,
            ),
            points_stat: read_integer(&armor.field("pointsStat")?, 0, 15)? as usize,
            protection: read_fraction(&armor.field("protection")?)?,
            tiers: if tiers.value() == &ProfileValue::Null {
                None
            } else {
                Some(ArmorTiers {
                    stat: read_integer(&tiers.field("stat")?, 0, 15)? as usize,
                    when_any: tiers.field("whenAny")?.list(|value| {
                        Ok(TierCondition {
                            offset: read_aligned(&value.field("offset")?, data_bytes, 4)?,
                            comparison: match value.field("comparison")?.choice(&["equal", "not-equal"])?.as_str() {
                                "equal" => TierComparison::Equal,
                                _ => TierComparison::NotEqual,
                            },
                            value: read_integer(&value.field("value")?, i64::from(i32::MIN), i64::from(i32::MAX))?
                                as i32,
                        })
                    })?,
                    values: tiers.field("values")?.list(|value| {
                        Ok(TierValue {
                            tier: read_integer(&value.field("tier")?, i64::from(i32::MIN), i64::from(i32::MAX))? as i32,
                            protection: read_fraction(&value.field("protection")?)?,
                        })
                    })?,
                    fallback: read_fraction(&tiers.field("fallback")?)?,
                })
            },
        },
        reactions: CombatReactions {
            flags: read_private(&reactions.field("flags")?)?,
            pain: read_private(&reactions.field("pain")?)?,
            die: read_private(&reactions.field("die")?)?,
            pain_call: read_reaction_call(&reactions.field("painCall")?)?,
            die_call: read_reaction_call(&reactions.field("dieCall")?)?,
        },
        grapple_damage_method: reader.field("grappleDamageMethod")?.integer(0)? as usize,
        state: CombatState {
            health_stat: read_integer(&state.field("healthStat")?, 0, 15)? as usize,
            team: CombatTeamState {
                persistent_stat: read_integer(&team.field("persistentStat")?, 0, 15)? as usize,
                values: team.field("values")?.list(|value| {
                    Ok(QvmCombatTeam {
                        value: read_integer(&value.field("value")?, i64::from(i32::MIN), i64::from(i32::MAX))? as i32,
                        team: namespaced_id(&value.field("team")?)?,
                    })
                })?,
            },
            flags: CombatStateFlags {
                notarget: read_mask(&flags.field("notarget")?)?,
                invulnerable: read_mask(&flags.field("invulnerable")?)?,
                no_knockback: read_mask(&flags.field("noKnockback")?)?,
            },
            mass: if mass.field("kind")?.choice(&["constant", "entity"])? == "constant" {
                QvmCombatMass::Constant(mass.field("value")?.finite()?)
            } else {
                QvmCombatMass::Entity {
                    offset: read_private(&mass.field("offset")?)?,
                    storage: match mass.field("storage")?.choice(&["int32", "float32"])?.as_str() {
                        "float32" => QvmMassStorage::Float32,
                        _ => QvmMassStorage::Int32,
                    },
                }
            },
        },
        damage_flags: QvmDamageFlags {
            radius: read_mask(&damage_flags.field("radius")?)?,
            no_armor: read_mask(&damage_flags.field("noArmor")?)?,
            no_knockback: read_mask(&damage_flags.field("noKnockback")?)?,
            no_protection: read_mask(&damage_flags.field("noProtection")?)?,
            no_team_protection: read_mask(&damage_flags.field("noTeamProtection")?)?,
        },
    };
    validate_qvm_combat_call(&result.damage_call, data_bytes)
        .map_err(|error| GuestError::invalid(format!("combat: {error}")))?;
    validate_qvm_combat_call(&result.armor.call, data_bytes)
        .map_err(|error| GuestError::invalid(format!("armor: {error}")))?;
    if matches!(result.state.mass, QvmCombatMass::Constant(value) if value < 0.0) {
        return mass.fail("source mass must be nonnegative");
    }
    if result
        .state
        .team
        .values
        .iter()
        .map(|value| value.value)
        .collect::<HashSet<_>>()
        .len()
        != result.state.team.values.len()
    {
        return team.fail("source team values must be unique");
    }
    let state_masks = [
        result.state.flags.notarget,
        result.state.flags.invulnerable,
        result.state.flags.no_knockback,
    ];
    let damage_masks = [
        result.damage_flags.radius,
        result.damage_flags.no_armor,
        result.damage_flags.no_knockback,
        result.damage_flags.no_protection,
        result.damage_flags.no_team_protection,
    ];
    if state_masks.into_iter().collect::<HashSet<_>>().len() != state_masks.len()
        || damage_masks.into_iter().collect::<HashSet<_>>().len() != damage_masks.len()
    {
        return reader.fail("source combat flags overlap");
    }
    if let Some(tier) = result.armor.tiers.as_ref() {
        if tier.stat == result.armor.points_stat
            || tier.when_any.is_empty()
            || tier.values.is_empty()
            || tier.values.iter().map(|value| value.tier).collect::<HashSet<_>>().len() != tier.values.len()
        {
            return tiers.fail("armor tier declaration is empty or ambiguous");
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::mod_provider::{QvmImage, QvmInstruction, QvmRole};
    use super::*;

    fn int(value: i64) -> ProfileValue {
        ProfileValue::Int(value)
    }

    fn text(value: &str) -> ProfileValue {
        ProfileValue::Str(value.to_string())
    }

    fn rec(fields: Vec<(&str, ProfileValue)>) -> ProfileValue {
        ProfileValue::record(fields)
    }

    fn arr(items: Vec<ProfileValue>) -> ProfileValue {
        ProfileValue::Array(items)
    }

    fn fixture_module() -> ModuleId {
        ModuleId {
            id: "test:game".to_string(),
            artifact_path: "vm/qagame.qvm".to_string(),
            digest: "sha256:game".to_string(),
            revision: "1".to_string(),
        }
    }

    fn enters(count: usize) -> Vec<QvmInstruction> {
        let mut instructions = Vec::new();
        for _ in 0..count {
            instructions.push(QvmInstruction::word(QvmOpcode::OpEnter, 0));
            instructions.push(QvmInstruction::word(QvmOpcode::OpLeave, 0));
        }
        instructions
    }

    fn simple_artifact() -> QvmArtifact {
        QvmArtifact {
            module: fixture_module(),
            role: QvmRole::Qagame,
            abi_profile: None,
            image: QvmImage {
                instructions: enters(8),
                data_length: 4096,
                literal_length: 0,
                bss_length: 0,
                initialized_length: 4096,
                allocated_data_length: 200000,
            },
        }
    }

    fn weapons_artifact() -> QvmArtifact {
        let instructions = vec![
            QvmInstruction::word(QvmOpcode::OpEnter, 64),
            QvmInstruction::word(QvmOpcode::OpEq, 0),
            QvmInstruction::word(QvmOpcode::OpLocal, 12),
            QvmInstruction::word(QvmOpcode::OpConst, 5),
            QvmInstruction::word(QvmOpcode::OpStore4, 0),
            QvmInstruction::word(QvmOpcode::OpLeave, 0),
            QvmInstruction::word(QvmOpcode::OpEnter, 0),
            QvmInstruction::word(QvmOpcode::OpLeave, 0),
            QvmInstruction::word(QvmOpcode::OpEnter, 0),
            QvmInstruction::word(QvmOpcode::OpLeave, 0),
            QvmInstruction::word(QvmOpcode::OpEnter, 64),
            QvmInstruction::word(QvmOpcode::OpConst, 1),
            QvmInstruction::word(QvmOpcode::OpPop, 0),
            QvmInstruction::word(QvmOpcode::OpLeave, 0),
            QvmInstruction::word(QvmOpcode::OpEnter, 0),
            QvmInstruction::word(QvmOpcode::OpLeave, 0),
            QvmInstruction::word(QvmOpcode::OpEnter, 64),
            QvmInstruction::word(QvmOpcode::OpConst, 1),
            QvmInstruction::word(QvmOpcode::OpPop, 0),
            QvmInstruction::word(QvmOpcode::OpLeave, 0),
            QvmInstruction::word(QvmOpcode::OpEnter, 64),
            QvmInstruction::word(QvmOpcode::OpConst, 1),
            QvmInstruction::word(QvmOpcode::OpPop, 0),
            QvmInstruction::word(QvmOpcode::OpLeave, 0),
            QvmInstruction::word(QvmOpcode::OpEnter, 64),
            QvmInstruction::word(QvmOpcode::OpConst, 1),
            QvmInstruction::word(QvmOpcode::OpPop, 0),
            QvmInstruction::word(QvmOpcode::OpLeave, 0),
            QvmInstruction::word(QvmOpcode::OpEq, 0),
            QvmInstruction::word(QvmOpcode::OpNe, 0),
            QvmInstruction::word(QvmOpcode::OpEnter, 64),
            QvmInstruction::word(QvmOpcode::OpConst, 1),
            QvmInstruction::word(QvmOpcode::OpPop, 0),
            QvmInstruction::word(QvmOpcode::OpLeave, 0),
            QvmInstruction::word(QvmOpcode::OpEnter, 0),
            QvmInstruction::word(QvmOpcode::OpLeave, 0),
        ];
        QvmArtifact {
            module: fixture_module(),
            role: QvmRole::Qagame,
            abi_profile: None,
            image: QvmImage {
                instructions,
                data_length: 4096,
                literal_length: 0,
                bss_length: 0,
                initialized_length: 4096,
                allocated_data_length: 200000,
            },
        }
    }

    fn test_value(offset: i64) -> ProfileValue {
        rec(vec![
            ("field", rec(vec![("record", text("client")), ("offset", int(offset))])),
            ("mask", ProfileValue::Null),
            ("comparison", text("equals")),
            ("value", int(3)),
        ])
    }

    fn weapons_declaration() -> ProfileValue {
        rec(vec![
            ("entityStride", int(520)),
            ("clientStride", int(468)),
            ("clientPointer", int(64)),
            ("maxHealth", int(100)),
            ("persistentMaxHealth", int(104)),
            (
                "stage",
                rec(vec![
                    (
                        "dispatcher",
                        rec(vec![
                            ("entry", int(0)),
                            (
                                "actor",
                                rec(vec![
                                    ("record", text("client")),
                                    (
                                        "pointer",
                                        rec(vec![
                                            ("kind", text("argument")),
                                            ("index", int(0)),
                                            ("offset", int(0)),
                                            ("indirections", arr(vec![])),
                                        ]),
                                    ),
                                ]),
                            ),
                        ]),
                    ),
                    (
                        "predicates",
                        arr(vec![rec(vec![
                            ("instruction", int(1)),
                            ("unselected", ProfileValue::Bool(false)),
                        ])]),
                    ),
                    ("settled", arr(vec![test_value(8)])),
                    (
                        "selection",
                        rec(vec![
                            ("field", rec(vec![("record", text("client")), ("offset", int(4))])),
                            (
                                "values",
                                arr(vec![rec(vec![("value", int(1)), ("item", text("test:mg"))])]),
                            ),
                        ]),
                    ),
                    (
                        "request",
                        rec(vec![
                            ("entry", int(6)),
                            ("argument", int(0)),
                            ("accepted", arr(vec![test_value(8)])),
                        ]),
                    ),
                ]),
            ),
            (
                "equipmentMovement",
                rec(vec![
                    ("move", int(8)),
                    ("slice", int(10)),
                    ("duck", int(14)),
                    ("movementGlobal", int(100)),
                    ("locomotion", rec(vec![("entry", int(11)), ("join", int(13))])),
                    ("mins", int(300)),
                    ("maxs", int(400)),
                ]),
            ),
            (
                "availability",
                rec(vec![
                    ("movementType", int(12)),
                    ("excluded", arr(vec![])),
                    ("health", int(16)),
                    ("team", int(20)),
                    ("spectatorTeam", int(2)),
                    ("flags", int(24)),
                    ("respawnFlag", int(1)),
                ]),
            ),
            (
                "powerups",
                rec(vec![("quad", int(28)), ("haste", int(32)), ("flight", int(36))]),
            ),
            (
                "torsoAnimation",
                rec(vec![("entry", int(34)), ("attack", int(1)), ("melee", int(2))]),
            ),
            (
                "waterLevel",
                rec(vec![("entityOffset", int(40)), ("movementOffset", int(500))]),
            ),
            (
                "damageFactor",
                rec(vec![
                    ("entry", int(16)),
                    ("result", int(200)),
                    ("stop", rec(vec![("entry", int(17)), ("join", int(19))])),
                ]),
            ),
            (
                "equipmentContexts",
                arr(vec![rec(vec![
                    ("provider", text("test:mod")),
                    ("item", ProfileValue::Null),
                ])]),
            ),
            (
                "delay",
                rec(vec![
                    ("entry", int(2)),
                    ("join", int(5)),
                    ("inputs", arr(vec![int(8)])),
                    ("result", int(12)),
                ]),
            ),
            (
                "delayPlayer",
                rec(vec![("movementGlobal", int(120)), ("playerOffset", int(600))]),
            ),
            (
                "teleport",
                rec(vec![
                    ("entry", int(20)),
                    (
                        "region",
                        rec(vec![
                            ("entry", int(21)),
                            ("join", int(23)),
                            ("inputs", arr(vec![])),
                            ("result", ProfileValue::Null),
                        ]),
                    ),
                    (
                        "objectives",
                        rec(vec![
                            ("entry", int(21)),
                            ("join", int(23)),
                            ("inputs", arr(vec![])),
                            ("result", ProfileValue::Null),
                        ]),
                    ),
                    ("spawn", int(8)),
                    ("view", int(8)),
                ]),
            ),
            (
                "drop",
                rec(vec![
                    ("entry", int(30)),
                    ("argument", int(0)),
                    ("weapon", int(44)),
                    ("ammo", text("inventory")),
                    ("region", rec(vec![("entry", int(31)), ("join", int(33))])),
                ]),
            ),
            (
                "give",
                rec(vec![
                    ("entry", int(24)),
                    ("argument", int(0)),
                    ("weapons", int(28)),
                    ("ammo", int(29)),
                    (
                        "named",
                        rec(vec![
                            ("entry", int(25)),
                            ("join", int(27)),
                            ("name", int(8)),
                            ("item", int(12)),
                        ]),
                    ),
                ]),
            ),
        ])
    }

    #[test]
    fn input_reads_and_compares() {
        let declaration = rec(vec![
            ("entityStride", int(520)),
            ("clientStride", int(468)),
            ("clientPointer", int(64)),
            ("intermission", arr(vec![int(0), int(1)])),
            (
                "entries",
                rec(vec![
                    ("clientThink", int(0)),
                    ("runClient", int(2)),
                    ("clientSpawn", int(4)),
                    ("move", int(6)),
                    ("slice", int(8)),
                ]),
            ),
        ]);
        let artifact = simple_artifact();
        let left = read_qvm_primary_input(&ProfileReader::new(&declaration), &artifact).unwrap();
        assert_eq!(left.client_pointer, 64);
        assert!(same_qvm_primary_input(&left, &left));
        let mut other = left.clone();
        other.client_pointer = 68;
        assert!(!same_qvm_primary_input(&left, &other));
        other = left.clone();
        other.movement_modes = Some(MovementModes {
            normal: 0,
            noclip: 1,
            freeze: 2,
        });
        assert!(same_qvm_primary_input(&left, &other));
    }

    #[test]
    fn weapons_read_qualifies_regions() {
        let declaration = weapons_declaration();
        let artifact = weapons_artifact();
        let profile = read_qvm_primary_weapons(&ProfileReader::new(&declaration), &artifact, None).unwrap();
        assert_eq!(profile.stage.dispatcher.entry, 0);
        assert_eq!(profile.teleport.spawn, 8);
        assert_eq!(profile.give.named.name, 8);
        let catalog = [QvmWeaponCatalogRow {
            weapon: 1,
            item: "test:mg".to_string(),
        }];
        read_qvm_primary_weapons(&ProfileReader::new(&declaration), &artifact, Some(&catalog)).unwrap();
        let wrong = [QvmWeaponCatalogRow {
            weapon: 2,
            item: "test:mg".to_string(),
        }];
        assert!(read_qvm_primary_weapons(&ProfileReader::new(&declaration), &artifact, Some(&wrong)).is_err());
    }

    #[test]
    fn combat_reads_calls_and_state() {
        let roles = [
            "target",
            "inflictor",
            "attacker",
            "direction",
            "point",
            "amount",
            "flags",
            "method",
        ];
        let damage_roles = rec(roles
            .into_iter()
            .enumerate()
            .map(|(index, role)| (role, int(index as i64)))
            .collect());
        let declaration = rec(vec![
            ("entityStride", int(1024)),
            ("clientStride", int(468)),
            (
                "damageCall",
                rec(vec![("roles", damage_roles), ("extras", arr(vec![]))]),
            ),
            (
                "state",
                rec(vec![
                    ("healthStat", int(5)),
                    (
                        "team",
                        rec(vec![
                            ("persistentStat", int(3)),
                            (
                                "values",
                                arr(vec![
                                    rec(vec![("value", int(1)), ("team", text("test:red"))]),
                                    rec(vec![("value", int(2)), ("team", text("test:blue"))]),
                                ]),
                            ),
                        ]),
                    ),
                    (
                        "flags",
                        rec(vec![
                            ("notarget", int(1)),
                            ("invulnerable", int(2)),
                            ("noKnockback", int(4)),
                        ]),
                    ),
                    (
                        "mass",
                        rec(vec![("kind", text("constant")), ("value", ProfileValue::Float(100.0))]),
                    ),
                ]),
            ),
            (
                "damageFlags",
                rec(vec![
                    ("radius", int(1)),
                    ("noArmor", int(2)),
                    ("noKnockback", int(4)),
                    ("noProtection", int(8)),
                    ("noTeamProtection", int(16)),
                ]),
            ),
            (
                "fields",
                rec(vec![
                    ("inuse", int(516)),
                    ("health", int(520)),
                    ("takedamage", int(524)),
                    ("parent", int(528)),
                    ("client", int(532)),
                ]),
            ),
            (
                "callbacks",
                rec(vec![("allocate", int(0)), ("free", int(2)), ("damage", int(4))]),
            ),
            (
                "reactions",
                rec(vec![
                    ("flags", int(536)),
                    ("pain", int(540)),
                    ("die", int(544)),
                    (
                        "painCall",
                        rec(vec![
                            ("arguments", int(2)),
                            ("roles", rec(vec![("target", int(0)), ("amount", int(1))])),
                        ]),
                    ),
                    (
                        "dieCall",
                        rec(vec![
                            ("arguments", int(2)),
                            ("roles", rec(vec![("target", int(0)), ("amount", int(1))])),
                        ]),
                    ),
                ]),
            ),
            ("grappleDamageMethod", int(9)),
            (
                "armor",
                rec(vec![
                    ("checkArmor", int(10)),
                    (
                        "call",
                        rec(vec![
                            (
                                "roles",
                                rec(vec![("target", int(0)), ("amount", int(1)), ("flags", int(2))]),
                            ),
                            ("extras", arr(vec![])),
                        ]),
                    ),
                    ("pointsStat", int(4)),
                    ("protection", ProfileValue::Float(0.5)),
                    ("tiers", ProfileValue::Null),
                ]),
            ),
        ]);
        let artifact = simple_artifact();
        let profile = read_qvm_primary_combat(&ProfileReader::new(&declaration), &artifact).unwrap();
        assert_eq!(profile.fields.client, 532);
        assert_eq!(profile.damage_flags.radius, 1);
        assert!(profile.armor.tiers.is_none());
        validate_qvm_combat_call(&profile.damage_call, 4096).unwrap();
        let bad = QvmCombatCall::damage(0, 0, 2, 3, 4, 5, 6, 7, Vec::new());
        assert!(validate_qvm_combat_call(&bad, 4096).is_err());
    }

    #[test]
    fn match_reader_validates_teams() {
        let declaration = rec(vec![
            ("score", int(100)),
            (
                "teams",
                arr(vec![rec(vec![
                    ("source", text("a")),
                    ("team", text("test:red")),
                    ("arguments", arr(vec![text("join")])),
                ])]),
            ),
        ]);
        let result = read_source_primary_match(&ProfileReader::new(&declaration), 468).unwrap();
        assert_eq!(result.teams.len(), 1);
        let bad = rec(vec![("score", int(101)), ("teams", arr(vec![]))]);
        assert!(read_source_primary_match(&ProfileReader::new(&bad), 468).is_err());
    }
}
