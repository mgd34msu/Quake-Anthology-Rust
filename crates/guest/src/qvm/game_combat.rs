//! QVM damage calls against declared source fields and callbacks.
//!
//! Provenance: `src/compat/qvm/game-combat.ts`.
//!
//! Absorbs the pure-Rust types of `src/contracts/qvm-combat.ts`
//! ([`QvmDamageRole`], [`QvmArmorRole`], [`QvmCombatCall`],
//! [`QvmReactionCall`], [`QvmDamageFlags`], [`QvmCombatMass`],
//! [`QvmCombatTeam`]).
//!
//! Local mirrors: `src/contracts/gameplay.ts` (`DamageRequest`/
//! `AttackProvenance` via [`QvmDamageRequest`] and [`QvmDamageCause`]) and
//! `src/world/gameplay/armor.ts` (`attackDamageFlags` via
//! [`qvm_attack_damage_flags`]); foreign inflictor bodies reuse
//! [`qa_world::body::BodyState`].

use qa_core::math::Vec3;
use qa_world::body::BodyState;

use super::game_data::{
    AbiProfile, ModuleIdentity, QvmArtifact, QvmGameData, QvmModule, QvmOpcode, QVM_MAX_PRIVATE_ARGUMENT_WORDS,
};
use super::shared_entity_record::qvm_shared_entity_bytes;
use crate::error::GuestError;

/// Damage-call argument roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmDamageRole {
    /// Target entity pointer.
    Target,
    /// Inflictor entity pointer.
    Inflictor,
    /// Attacker entity pointer.
    Attacker,
    /// Direction vector pointer.
    Direction,
    /// Impact point pointer.
    Point,
    /// Damage amount.
    Amount,
    /// Damage flags.
    Flags,
    /// Means of death.
    Method,
}

impl QvmDamageRole {
    /// Role name as declared.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Target => "target",
            Self::Inflictor => "inflictor",
            Self::Attacker => "attacker",
            Self::Direction => "direction",
            Self::Point => "point",
            Self::Amount => "amount",
            Self::Flags => "flags",
            Self::Method => "method",
        }
    }
}

/// Armor-call argument roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmArmorRole {
    /// Target entity pointer.
    Target,
    /// Damage amount.
    Amount,
    /// Damage flags.
    Flags,
}

impl QvmArmorRole {
    /// Role name as declared.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Target => "target",
            Self::Amount => "amount",
            Self::Flags => "flags",
        }
    }
}

/// Extra argument word kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmCombatExtraKind {
    /// Signed 32-bit word.
    Int32,
    /// Binary32 word.
    Float32,
    /// Data-segment address.
    Address,
}

/// One lowered extra argument word.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmCombatExtra {
    /// Word index.
    pub index: usize,
    /// Word kind.
    pub kind: QvmCombatExtraKind,
    /// Declared value.
    pub value: f64,
}

/// Declared source combat call: named roles plus lowered extras.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmCombatCall {
    /// Role name to word index.
    pub roles: Vec<(String, usize)>,
    /// Lowered extra words.
    pub extras: Vec<QvmCombatExtra>,
}

impl QvmCombatCall {
    /// Resolve a role name to its word index.
    pub fn role(&self, name: &str) -> Result<usize, GuestError> {
        self.roles
            .iter()
            .find(|(role, _)| role == name)
            .map(|(_, index)| *index)
            .ok_or_else(|| GuestError::invalid(format!("Source combat call omits role {name}")))
    }
}

/// Declared damage-reaction observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmReactionCall {
    /// Total argument word count.
    pub arguments: usize,
    /// Target argument position.
    pub target: usize,
    /// Amount argument position.
    pub amount: usize,
}

/// Source damage-flag bit masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmDamageFlags {
    /// Radius-damage mask.
    pub radius: i32,
    /// No-armor mask.
    pub no_armor: i32,
    /// No-knockback mask.
    pub no_knockback: i32,
    /// No-protection mask.
    pub no_protection: i32,
    /// No-team-protection mask.
    pub no_team_protection: i32,
}

/// Combat mass selector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QvmCombatMass {
    /// Constant mass.
    Constant {
        /// Mass value.
        value: f64,
    },
    /// Entity-record mass.
    Entity {
        /// Record offset.
        offset: usize,
        /// Entity mass as int or float storage.
        float_storage: bool,
    },
}

/// Combat team selector: source value plus namespaced team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmCombatTeam {
    /// Source team value.
    pub value: i32,
    /// Namespaced team identity.
    pub team: String,
}

/// Quake I armor effect of a damage cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmQ1ArmorEffect {
    /// Standard armor interaction.
    Standard,
    /// Bypasses armor.
    Bypass,
    /// Half armor effectiveness.
    HalfEffectiveness,
}

/// Damage cause (mirror of `DamageRequest["attack"]["cause"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmDamageCause {
    /// Quake I cause.
    Q1 {
        /// Armor effect.
        armor_effect: QvmQ1ArmorEffect,
    },
    /// Quake II cause with native damage flags.
    Q2 {
        /// Native damage flags.
        damage_flags: i32,
    },
    /// Quake III cause with native damage flags.
    Q3 {
        /// Native damage flags.
        damage_flags: i32,
    },
    /// Environmental cause.
    Environment,
}

/// Damage request (mirror of the `DamageRequest` fields combat lowering uses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmDamageRequest {
    /// Damage cause.
    pub cause: QvmDamageCause,
    /// Whether the delivery is radial.
    pub radius_delivery: bool,
}

/// Decoded attack flags (mirror of `attackDamageFlags`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmAttackDamageFlags {
    /// Bypasses all armor.
    pub no_armor: bool,
    /// Bypasses power armor.
    pub no_power_armor: bool,
    /// Bypasses regular armor.
    pub no_regular_armor: bool,
    /// Energy damage.
    pub energy: bool,
    /// Regular protection scale.
    pub regular_protection_scale: f32,
    /// Suppresses knockback.
    pub no_knockback: bool,
    /// Bypasses protection.
    pub no_protection: bool,
    /// Bypasses team protection.
    pub no_team_protection: bool,
    /// Destroys armor.
    pub destroy_armor: bool,
}

/// Decode native flags by origin, never reinterpreting bit positions.
#[must_use]
pub fn qvm_attack_damage_flags(request: &QvmDamageRequest) -> QvmAttackDamageFlags {
    let q2 = match request.cause {
        QvmDamageCause::Q2 { damage_flags } => damage_flags,
        _ => 0,
    };
    let q3 = match request.cause {
        QvmDamageCause::Q3 { damage_flags } => damage_flags,
        _ => 0,
    };
    QvmAttackDamageFlags {
        no_armor: (q2 | q3) & 2 != 0
            || matches!(
                request.cause,
                QvmDamageCause::Q1 {
                    armor_effect: QvmQ1ArmorEffect::Bypass
                }
            ),
        no_power_armor: q2 & 0x100 != 0,
        no_regular_armor: q2 & 0x80 != 0,
        energy: q2 & 4 != 0,
        regular_protection_scale: if matches!(
            request.cause,
            QvmDamageCause::Q1 {
                armor_effect: QvmQ1ArmorEffect::HalfEffectiveness
            }
        ) {
            0.5
        } else {
            1.0
        },
        no_knockback: q2 & 8 != 0 || q3 & 4 != 0,
        no_protection: q2 & 0x20 != 0 || q3 & 8 != 0,
        no_team_protection: q3 & 0x10 != 0,
        destroy_armor: q2 & 0x40 != 0,
    }
}

/// Lower role words plus extras into a source argument frame.
pub fn qvm_combat_words(call: &QvmCombatCall, values: &[(&str, i32)]) -> Result<Vec<i32>, GuestError> {
    let mut positions: Vec<usize> = call.roles.iter().map(|(_, index)| *index).collect();
    positions.extend(call.extras.iter().map(|extra| extra.index));
    let words = positions.len();
    validate_qvm_combat_positions(&positions, words)?;
    let mut lowered = vec![0i32; words];
    for extra in &call.extras {
        lowered[extra.index] = match extra.kind {
            QvmCombatExtraKind::Float32 => (extra.value as f32).to_bits() as i32,
            QvmCombatExtraKind::Int32 | QvmCombatExtraKind::Address => extra.value as i32,
        };
    }
    for (role, value) in values {
        if let Ok(index) = call.role(role) {
            lowered[index] = *value;
        }
    }
    Ok(lowered)
}

/// Canonicalize source damage flags to the shared bit layout.
#[must_use]
pub fn qvm_canonical_damage_flags(masks: &QvmDamageFlags, flags: i32) -> i32 {
    i32::from(flags & masks.radius != 0)
        | i32::from(flags & masks.no_armor != 0) << 1
        | i32::from(flags & masks.no_knockback != 0) << 2
        | i32::from(flags & masks.no_protection != 0) << 3
        | i32::from(flags & masks.no_team_protection != 0) << 4
}

/// Lower a damage request to source flags, preserving undeclared bits.
#[must_use]
pub fn qvm_source_damage_flags(masks: &QvmDamageFlags, request: &QvmDamageRequest, original: i32) -> i32 {
    let flags = qvm_attack_damage_flags(request);
    let declared = masks.radius | masks.no_armor | masks.no_knockback | masks.no_protection | masks.no_team_protection;
    (original & !declared)
        | if request.radius_delivery { masks.radius } else { 0 }
        | if flags.no_armor { masks.no_armor } else { 0 }
        | if flags.no_knockback { masks.no_knockback } else { 0 }
        | if flags.no_protection { masks.no_protection } else { 0 }
        | if flags.no_team_protection {
            masks.no_team_protection
        } else {
            0
        }
}

/// Check combat argument positions cover each role exactly once.
pub fn validate_qvm_combat_positions(positions: &[usize], words: usize) -> Result<(), GuestError> {
    if words < positions.len() || words > QVM_MAX_PRIVATE_ARGUMENT_WORDS {
        return Err(GuestError::invalid(
            "Source combat call exceeds the QVM OP_ARG capacity or omits required arguments",
        ));
    }
    let mut seen = vec![false; words];
    for position in positions {
        if *position >= words || seen[*position] {
            return Err(GuestError::invalid(
                "Source combat argument positions must cover each declared role exactly once within the original call",
            ));
        }
        seen[*position] = true;
    }
    Ok(())
}

/// Check a combat call lowers every word within artifact data.
pub fn validate_qvm_combat_call(call: &QvmCombatCall, data_bytes: usize) -> Result<(), GuestError> {
    let mut positions: Vec<usize> = call.roles.iter().map(|(_, index)| *index).collect();
    positions.extend(call.extras.iter().map(|extra| extra.index));
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

/// Tier guard: source stat comparison selecting alternate protection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmArmorTierGuard {
    /// Source stat offset.
    pub offset: usize,
    /// Whether the guard requires equality.
    pub equal: bool,
    /// Compared value.
    pub value: i32,
}

/// Tier value: protection for one armor tier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmArmorTierValue {
    /// Tier index.
    pub tier: i32,
    /// Protection fraction.
    pub protection: f32,
}

/// Armor tier table.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmArmorTiers {
    /// Source stat holding the tier.
    pub stat: i32,
    /// Guards enabling the table.
    pub when_any: Vec<QvmArmorTierGuard>,
    /// Tier values.
    pub values: Vec<QvmArmorTierValue>,
    /// Fallback protection.
    pub fallback: f32,
}

/// Declared source armor check.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGameArmorDefinition {
    /// Armor-check entry point.
    pub check_armor: usize,
    /// Armor call roles.
    pub call: QvmCombatCall,
    /// Points stat.
    pub points_stat: i32,
    /// Base protection.
    pub protection: f32,
    /// Tier table, if any.
    pub tiers: Option<QvmArmorTiers>,
}

/// Declared private entity fields consumed by damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmCombatFields {
    /// In-use word offset.
    pub inuse: usize,
    /// Health word offset.
    pub health: usize,
    /// Take-damage word offset.
    pub takedamage: usize,
    /// Parent pointer offset.
    pub parent: usize,
}

/// Declared damage callbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmCombatCallbacks {
    /// Temp-entity allocator entry.
    pub allocate: usize,
    /// Temp-entity release entry.
    pub free: usize,
    /// Damage entry.
    pub damage: usize,
}

/// Declared source combat binding.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGameCombatDefinition {
    /// Damage call roles.
    pub damage_call: QvmCombatCall,
    /// Owning module.
    pub module: ModuleIdentity,
    /// ABI profile.
    pub abi_profile: AbiProfile,
    /// Entity stride.
    pub entity_stride: usize,
    /// Client stride.
    pub client_stride: usize,
    /// Private fields.
    pub fields: QvmCombatFields,
    /// Callbacks.
    pub callbacks: QvmCombatCallbacks,
}

/// Damage inflictor: a located slot or a foreign body.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmGameInflictor {
    /// Located entity slot.
    Entity {
        /// Entity slot.
        slot: usize,
    },
    /// Foreign body materialized as a temp entity.
    Foreign {
        /// Foreign body state.
        body: BodyState,
    },
}

/// Declared damage hit.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGameDamage {
    /// Target slot.
    pub target: usize,
    /// Attacker slot, if any.
    pub attacker: Option<usize>,
    /// Inflictor, if any.
    pub inflictor: Option<QvmGameInflictor>,
    /// Damage direction.
    pub direction: Vec3,
    /// Impact point.
    pub point: Vec3,
    /// Damage amount.
    pub amount: i32,
    /// Source damage flags.
    pub flags: i32,
    /// Means of death.
    pub method: i32,
}

/// Located entity health.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmCombatEntityState {
    /// Current health.
    pub health: i32,
    /// Whether the entity takes damage.
    pub damageable: bool,
}

/// Declared source fields and callbacks consuming damage in the owning
/// executable.
#[derive(Debug, Clone)]
pub struct QvmGameCombat {
    module: QvmModule,
    data: QvmGameData,
    definition: QvmGameCombatDefinition,
    scratch: usize,
    arguments: Vec<i32>,
}

impl QvmGameCombat {
    /// Bind a combat definition to its module, data, and artifact image.
    pub fn bind(
        module: &QvmModule,
        data: &QvmGameData,
        artifact: &QvmArtifact,
        definition: QvmGameCombatDefinition,
    ) -> Result<Self, GuestError> {
        let actual = module.module_id();
        if !definition.module.same_module(&actual)
            || artifact.module.digest != actual.digest
            || artifact.module.id != actual.id
            || definition.abi_profile != module.abi_profile()
        {
            return Err(GuestError::invalid(
                "Source combat declaration differs from its executable",
            ));
        }
        for offset in [
            definition.fields.inuse,
            definition.fields.health,
            definition.fields.takedamage,
            definition.fields.parent,
        ] {
            if offset % 4 != 0
                || offset < qvm_shared_entity_bytes(definition.abi_profile)
                || offset + 4 > definition.entity_stride
            {
                return Err(GuestError::invalid(
                    "Source combat field is outside its declared entity record",
                ));
            }
        }
        for entry in [
            definition.callbacks.allocate,
            definition.callbacks.free,
            definition.callbacks.damage,
        ] {
            if artifact
                .image
                .instruction(entry)
                .map_or(true, |instruction| instruction.opcode != QvmOpcode::OpEnter)
            {
                return Err(GuestError::invalid("Source combat callback is not a function entry"));
            }
        }
        validate_qvm_combat_call(
            &definition.damage_call,
            artifact.image.data_length + artifact.image.literal_length + artifact.image.bss_length,
        )?;
        let arguments = qvm_combat_words(
            &definition.damage_call,
            &[
                ("target", 0),
                ("inflictor", 0),
                ("attacker", 0),
                ("direction", 0),
                ("point", 0),
                ("amount", 0),
                ("flags", 0),
                ("method", 0),
            ],
        )?;
        let scratch = artifact.image.data_end().div_ceil(16) * 16;
        if scratch + 24 > artifact.image.allocated_data_length.saturating_sub(65536) {
            return Err(GuestError::invalid(
                "Source combat requires scratch outside source data and stack",
            ));
        }
        Ok(Self {
            module: module.clone(),
            data: data.clone(),
            definition,
            scratch,
            arguments,
        })
    }

    /// Combat definition.
    #[must_use]
    pub fn definition(&self) -> &QvmGameCombatDefinition {
        &self.definition
    }

    fn entity_window(&self, slot: usize) -> Result<super::game_data::QvmMemoryWindow, GuestError> {
        if self.data.entity_stride_bytes() != self.definition.entity_stride
            || self.data.client_stride_bytes() != self.definition.client_stride
        {
            return Err(GuestError::invalid(
                "Source combat declaration differs from located records",
            ));
        }
        self.data.entity_bytes(slot)
    }

    fn pointer(&self, slot: usize) -> Result<i32, GuestError> {
        self.entity_window(slot)?;
        let base = self.data.checkpoint().entities_word;
        let offset = base + slot.saturating_mul(self.definition.entity_stride);
        i32::try_from(offset).map_err(|_| GuestError::invalid("Source combat pointer exceeds signed words"))
    }

    /// Read located entity health (`None` when not in use).
    pub fn state(&self, slot: usize) -> Result<Option<QvmCombatEntityState>, GuestError> {
        let view = self.entity_window(slot)?;
        let fields = &self.definition.fields;
        if view.get_i32(fields.inuse)? == 0 {
            return Ok(None);
        }
        Ok(Some(QvmCombatEntityState {
            health: view.get_i32(fields.health)?,
            damageable: view.get_i32(fields.takedamage)? != 0,
        }))
    }

    /// Consume damage inside the owning executable.
    pub fn damage(
        &self,
        hit: &QvmGameDamage,
        invoke: Option<&mut dyn FnMut(&[i32]) -> Result<(), GuestError>>,
    ) -> Result<(), GuestError> {
        let state = self.state(hit.target)?;
        let Some(state) = state else {
            return Ok(());
        };
        if !state.damageable {
            return Ok(());
        }
        let memory = self.module.memory();
        let saved = memory.read_bytes(self.scratch, 24)?;
        let write_vector =
            |offset: usize, value: &Vec3| -> Result<(), GuestError> { memory.write_vec3(self.scratch + offset, value) };
        write_vector(0, &hit.direction)?;
        write_vector(12, &hit.point)?;
        let mut temporary: Option<i32> = None;
        let outcome = (|| -> Result<(), GuestError> {
            let attacker = match hit.attacker {
                None => 0,
                Some(slot) => self.pointer(slot)?,
            };
            let mut inflictor = 0;
            if let Some(source) = &hit.inflictor {
                match source {
                    QvmGameInflictor::Entity { slot } => {
                        inflictor = self.pointer(*slot)?;
                    }
                    QvmGameInflictor::Foreign { body } => {
                        let pointer = self.module.call(&[], self.definition.callbacks.allocate)?;
                        let slot = self.data.number_from_pointer(pointer)?;
                        self.entity_window(slot)?
                            .set_i32(self.definition.fields.parent, attacker)?;
                        let entity = self.data.entity_bytes(slot)?;
                        let shift = if self.definition.abi_profile.is_modern() { 0 } else { 12 };
                        entity.set_vec3(436 - shift, &body.bounds.min)?;
                        entity.set_vec3(448 - shift, &body.bounds.max)?;
                        entity.set_vec3(488 - shift, &body.origin)?;
                        entity.set_vec3(500 - shift, &body.angles)?;
                        entity.set_vec3(24, &body.origin)?;
                        entity.set_vec3(36, &body.velocity)?;
                        inflictor = pointer;
                        temporary = Some(pointer);
                    }
                }
            }
            let roles = &self.definition.damage_call;
            let mut words = self.arguments.clone();
            let scratch = i32::try_from(self.scratch)
                .map_err(|_| GuestError::invalid("Source combat scratch exceeds signed words"))?;
            words[roles.role("target")?] = self.pointer(hit.target)?;
            words[roles.role("inflictor")?] = inflictor;
            words[roles.role("attacker")?] = attacker;
            words[roles.role("direction")?] = scratch;
            words[roles.role("point")?] = scratch + 12;
            words[roles.role("amount")?] = hit.amount;
            words[roles.role("flags")?] = hit.flags;
            words[roles.role("method")?] = hit.method;
            match invoke {
                Some(invoke) => invoke(&words),
                None => self.module.call(&words, self.definition.callbacks.damage).map(|_| ()),
            }
        })();
        if let Some(pointer) = temporary {
            let _ = self.module.call(&[pointer], self.definition.callbacks.free);
        }
        memory.write_bytes(self.scratch, &saved)?;
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::super::game_data::{QvmArtifact, QvmImage, QvmInstruction, QvmRole};
    use super::*;
    use qa_core::math::vec3;

    fn damage_call() -> QvmCombatCall {
        QvmCombatCall {
            roles: [
                "target",
                "inflictor",
                "attacker",
                "direction",
                "point",
                "amount",
                "flags",
                "method",
            ]
            .iter()
            .enumerate()
            .map(|(index, role)| ((*role).to_string(), index))
            .collect(),
            extras: Vec::new(),
        }
    }

    fn fixture() -> (QvmModule, QvmGameData, QvmArtifact) {
        let mut image = QvmImage::default();
        image.instructions = vec![
            QvmInstruction::word(QvmOpcode::OpEnter, 16, 0),
            QvmInstruction::word(QvmOpcode::OpEnter, 16, 5),
            QvmInstruction::word(QvmOpcode::OpEnter, 16, 10),
        ];
        image.allocated_data_length = 65536 + 4096;
        let module_id = ModuleIdentity {
            id: "q3:qagame".to_string(),
            artifact_path: "qagame.qvm".to_string(),
            digest: "d".to_string(),
            revision: "r".to_string(),
        };
        let artifact = QvmArtifact {
            module: module_id,
            role: QvmRole::Qagame,
            abi_profile: None,
            image,
        };
        let module = QvmModule::new(artifact.clone(), None, None).unwrap();
        let data = QvmGameData::new(module.memory(), AbiProfile::Modern);
        data.locate(64, 2, 560, 4096, 480).unwrap();
        (module, data, artifact)
    }

    fn definition(module: &ModuleIdentity) -> QvmGameCombatDefinition {
        QvmGameCombatDefinition {
            damage_call: damage_call(),
            module: module.clone(),
            abi_profile: AbiProfile::Modern,
            entity_stride: 560,
            client_stride: 480,
            fields: QvmCombatFields {
                inuse: 516,
                health: 520,
                takedamage: 524,
                parent: 528,
            },
            callbacks: QvmCombatCallbacks {
                allocate: 0,
                free: 1,
                damage: 2,
            },
        }
    }

    #[test]
    fn words_lower_roles_and_float_extras() {
        let call = QvmCombatCall {
            roles: vec![("amount".to_string(), 1)],
            extras: vec![QvmCombatExtra {
                index: 0,
                kind: QvmCombatExtraKind::Float32,
                value: 1.5,
            }],
        };
        assert_eq!(qvm_combat_words(&call, &[("amount", 7)]).unwrap(), vec![0x3FC0_0000, 7]);
        assert!(validate_qvm_combat_call(&call, 1024).is_ok());
        let bad = QvmCombatCall {
            roles: vec![("amount".to_string(), 0)],
            extras: vec![QvmCombatExtra {
                index: 0,
                kind: QvmCombatExtraKind::Int32,
                value: 1.0,
            }],
        };
        assert!(validate_qvm_combat_call(&bad, 1024).is_err());
    }

    #[test]
    fn flags_round_trip_through_masks() {
        let masks = QvmDamageFlags {
            radius: 0x40,
            no_armor: 1,
            no_knockback: 4,
            no_protection: 8,
            no_team_protection: 0x10,
        };
        assert_eq!(qvm_canonical_damage_flags(&masks, 0x40 | 8), 1 | 8);
        let request = QvmDamageRequest {
            cause: QvmDamageCause::Q3 { damage_flags: 2 },
            radius_delivery: true,
        };
        assert_eq!(qvm_source_damage_flags(&masks, &request, 0x20), 0x20 | 0x40 | 1);
        let q1 = QvmDamageRequest {
            cause: QvmDamageCause::Q1 {
                armor_effect: QvmQ1ArmorEffect::Bypass,
            },
            radius_delivery: false,
        };
        assert!(qvm_attack_damage_flags(&q1).no_armor);
        let q2 = QvmDamageRequest {
            cause: QvmDamageCause::Q2 {
                damage_flags: 0x180 | 4 | 8 | 0x20 | 0x40,
            },
            radius_delivery: false,
        };
        let flags = qvm_attack_damage_flags(&q2);
        assert!(flags.no_power_armor && flags.no_regular_armor && flags.energy && flags.destroy_armor);
    }

    #[test]
    fn combat_skips_unused_or_invulnerable_targets() {
        let (module, data, artifact) = fixture();
        let combat = QvmGameCombat::bind(&module, &data, &artifact, definition(&artifact.module)).unwrap();
        assert_eq!(combat.state(0).unwrap(), None);
        data.entity_bytes(0).unwrap().set_i32(516, 1).unwrap();
        data.entity_bytes(0).unwrap().set_i32(520, 75).unwrap();
        let state = combat.state(0).unwrap().unwrap();
        assert_eq!((state.health, state.damageable), (75, false));
        let hit = QvmGameDamage {
            target: 0,
            attacker: None,
            inflictor: None,
            direction: vec3(0.0, 0.0, 1.0),
            point: vec3(1.0, 2.0, 3.0),
            amount: 10,
            flags: 0,
            method: 1,
        };
        combat.damage(&hit, None).unwrap();
        assert!(module.calls().is_empty());
    }

    #[test]
    fn combat_consumes_damage_and_restores_scratch() {
        let (module, data, artifact) = fixture();
        let combat = QvmGameCombat::bind(&module, &data, &artifact, definition(&artifact.module)).unwrap();
        data.entity_bytes(0).unwrap().set_i32(516, 1).unwrap();
        data.entity_bytes(1).unwrap().set_i32(516, 1).unwrap();
        data.entity_bytes(0).unwrap().set_i32(524, 1).unwrap();
        let hit = QvmGameDamage {
            target: 0,
            attacker: Some(1),
            inflictor: Some(QvmGameInflictor::Entity { slot: 1 }),
            direction: vec3(0.0, 0.0, 1.0),
            point: vec3(1.0, 2.0, 3.0),
            amount: 10,
            flags: 3,
            method: 1,
        };
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let captured = std::rc::Rc::clone(&seen);
        let mut invoke = |words: &[i32]| {
            captured.borrow_mut().push(words.to_vec());
            Ok(())
        };
        combat.damage(&hit, Some(&mut invoke)).unwrap();
        let seen = seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0][5], 10);
        assert_eq!(seen[0][6], 3);
        assert_eq!(seen[0][0], 64);
        assert_eq!(seen[0][2], 64 + 560);
    }
}
