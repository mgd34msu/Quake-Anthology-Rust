//! Native combat bindings: qualified `G_Damage` retains source stores.
//!
//! Provenance: `src/compat/qvm/game-combat-binding.ts`.
//!
//! Local mirrors: `src/world/actors/body.ts` ([`CombatBodySource`]),
//! `src/world/gameplay/authority.ts` ([`CombatAuthority`], [`ArmorStage`]),
//! `src/world/actors/registry.ts` ([`CombatActorRegistry`]),
//! `src/contracts/gameplay.ts` ([`AuthorityDamageRequest`],
//! [`AuthorityCause`]), and `src/contracts/identity.ts` (`OwnedActor` is
//! [`ActorId`]; donor object identity becomes value equality).
//!
//! Service traits use `&self` with interior mutability so nested authority
//! calls (apply inside run-source-damage) work as in the donor. Hook
//! closures cannot fail, so unresolvable guest pointers proceed with the
//! original call instead of unwinding across an interpreter boundary.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use qa_world::body::BodyState;
use qa_world::combat::{ArmorState, PoweredProtection, RegularArmor};

use super::game_combat::{
    qvm_attack_damage_flags, qvm_canonical_damage_flags, qvm_source_damage_flags, validate_qvm_combat_call,
    validate_qvm_combat_positions, QvmArmorRole, QvmCombatCall, QvmCombatMass, QvmCombatTeam, QvmDamageFlags,
    QvmDamageRequest, QvmDamageRole, QvmGameArmorDefinition, QvmGameCombat, QvmGameCombatDefinition, QvmGameDamage,
    QvmGameInflictor, QvmQ1ArmorEffect, QvmReactionCall,
};
use super::game_combat_scope::{
    QvmDamageReaction, QvmDamageReactions, QvmDamageScopeOptions, QvmDamageScopes, QvmScopeDamageRequest,
    SharedDamageObserver, SourceDamageResult,
};
use super::game_data::{
    AbiProfile, ModuleIdentity, QvmArtifact, QvmFunctionCall, QvmGameData, QvmHookFn, QvmModule, QvmOpcode,
};
use super::shared_entity_record::qvm_shared_entity_bytes;
use crate::error::GuestError;

/// Damage delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageDelivery {
    /// Direct damage.
    Direct,
    /// Radius damage.
    Radius,
}

/// Authority damage cause (mirror of `DamageRequest["attack"]["cause"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityCause {
    /// Quake I cause.
    Q1 {
        /// Armor effect.
        armor_effect: QvmQ1ArmorEffect,
    },
    /// Quake II cause.
    Q2 {
        /// Means of death.
        means_of_death: i32,
        /// Native damage flags.
        damage_flags: i32,
    },
    /// Quake III cause.
    Q3 {
        /// Means of death.
        means_of_death: i32,
        /// Native damage flags.
        damage_flags: i32,
    },
    /// Environmental cause.
    Environment,
}

impl AuthorityCause {
    fn to_request_cause(self) -> super::game_combat::QvmDamageCause {
        use super::game_combat::QvmDamageCause;
        match self {
            Self::Q1 { armor_effect } => QvmDamageCause::Q1 { armor_effect },
            Self::Q2 { damage_flags, .. } => QvmDamageCause::Q2 { damage_flags },
            Self::Q3 { damage_flags, .. } => QvmDamageCause::Q3 { damage_flags },
            Self::Environment => QvmDamageCause::Environment,
        }
    }
}

/// Authority attack provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthorityAttack {
    /// Damage cause.
    pub cause: AuthorityCause,
    /// Attacker actor, if any.
    pub attacker: Option<ActorId>,
    /// Inflictor actor, if any.
    pub inflictor: Option<ActorId>,
    /// Attack weapon identity, if any.
    pub weapon: Option<String>,
    /// Namespaced movement provider.
    pub movement_provider: String,
}

/// Authority damage request (mirror of `DamageRequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct AuthorityDamageRequest {
    /// Target actor.
    pub target: ActorId,
    /// Damage amount.
    pub amount: f64,
    /// Knockback.
    pub knockback: f64,
    /// Damage direction.
    pub direction: Vec3,
    /// Impact point.
    pub point: Vec3,
    /// Impact normal.
    pub normal: Vec3,
    /// Delivery.
    pub delivery: DamageDelivery,
    /// Attack provenance.
    pub attack: AuthorityAttack,
}

impl AuthorityDamageRequest {
    fn to_combat_request(&self) -> QvmDamageRequest {
        QvmDamageRequest {
            cause: self.attack.cause.to_request_cause(),
            radius_delivery: self.delivery == DamageDelivery::Radius,
        }
    }

    fn to_scope_request(&self) -> QvmScopeDamageRequest {
        QvmScopeDamageRequest {
            cause: self.attack.cause.to_request_cause(),
            movement_provider: self.attack.movement_provider.clone(),
        }
    }
}

/// Provenance supplied by the joined source (weapon plus movement provider).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityProvenance {
    /// Attack weapon identity, if any.
    pub weapon: Option<String>,
    /// Namespaced movement provider.
    pub movement_provider: String,
}

/// Armor intercept: computes savings, defaulting to the original behavior.
pub type ArmorIntercept = Rc<dyn Fn(&ArmorHit, &mut dyn FnMut() -> i32) -> f64>;

/// Armor stage kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmorStageKind {
    /// Power stage.
    Power,
    /// Regular stage.
    Regular,
}

/// Armor hit flags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArmorHitFlags {
    /// Stage.
    pub stage: ArmorStageKind,
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
}

/// Armor hit geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorHitGeometry {
    /// Damage direction.
    pub direction: Vec3,
    /// Impact point.
    pub point: Vec3,
    /// Impact normal.
    pub normal: Vec3,
}

/// Armor hit delivered to an intercept.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorHit {
    /// Damage request.
    pub request: AuthorityDamageRequest,
    /// Current amount.
    pub amount: i32,
    /// Hit geometry.
    pub geometry: ArmorHitGeometry,
    /// Hit flags.
    pub flags: ArmorHitFlags,
}

/// Armor intercept binder; returns the intercept remover.
type ArmorBindFn = Rc<dyn Fn(ArmorIntercept) -> Result<Box<dyn FnOnce()>, GuestError>>;

/// Armor stage binding (mirror of `SourceArmorStage`).
#[derive(Clone)]
pub struct ArmorStage {
    /// Bind an intercept; returns its remover.
    pub bind: ArmorBindFn,
}

/// Protection channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtectionChannel {
    /// Regular armor channel.
    Regular,
    /// Powered protection channel.
    Powered,
}

/// Binding read snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct CombatBindingRead {
    /// Health.
    pub health: i32,
    /// Whether the entity takes damage.
    pub can_take_damage: bool,
    /// Mass.
    pub mass: f64,
    /// Armor.
    pub armor: ArmorState,
    /// Team identity, if any.
    pub team: Option<String>,
    /// Invulnerable flag.
    pub invulnerable: bool,
    /// No-knockback flag.
    pub no_knockback: bool,
}

/// Protection binding for one channel.
#[derive(Clone)]
pub struct ProtectionBinding {
    /// Owning module id, if any.
    pub owner: Option<String>,
    /// Armor stage, if any.
    pub stage: Option<ArmorStage>,
}

/// Armor validation hook.
type ValidateArmorFn = Rc<dyn Fn(&ArmorState) -> Result<(), GuestError>>;

/// Combat binding installed on the authority.
#[derive(Clone)]
pub struct CombatBinding {
    /// Read shared combat state.
    pub read: Rc<dyn Fn() -> Result<CombatBindingRead, GuestError>>,
    /// Deliver source damage.
    pub damage: Rc<dyn Fn(AuthorityDamageRequest) -> SourceDamageResult>,
    /// Regular protection.
    pub regular: ProtectionBinding,
    /// Powered protection.
    pub powered: ProtectionBinding,
    /// Validate armor values.
    pub validate_armor: ValidateArmorFn,
    /// Write health.
    pub write_health: Rc<dyn Fn(i32)>,
    /// Write armor.
    pub write_armor: Rc<dyn Fn(&ArmorState)>,
}

/// Gameplay authority (mirror of `GameplayAuthority`).
pub trait CombatAuthority {
    /// Whether the actor is bound.
    fn is_bound(&self, actor: &ActorId) -> bool;
    /// Bind a new actor.
    fn bind_combat(&self, actor: &ActorId, binding: &CombatBinding);
    /// Rebind an actor.
    fn rebind_combat(&self, actor: &ActorId, binding: &CombatBinding);
    /// Apply damage, composing through `run`.
    fn apply(
        &self,
        request: &AuthorityDamageRequest,
        run: &mut dyn FnMut(&AuthorityDamageRequest) -> SourceDamageResult,
    ) -> SourceDamageResult;
    /// Run source damage, reporting through an observer.
    fn run_source_damage(
        &self,
        request: &AuthorityDamageRequest,
        run: &mut dyn FnMut(&SharedDamageObserver, &AuthorityDamageRequest) -> SourceDamageResult,
    ) -> SourceDamageResult;
}

/// Shared body table (mirror of `SharedBodyTable`).
pub trait CombatBodySource {
    /// Read a body, if present.
    fn read(&self, actor: &ActorId) -> Option<BodyState>;
}

/// Session actor registry (mirror of `SessionActorRegistry`).
pub trait CombatActorRegistry {
    /// Resolve an owned actor by id.
    fn resolve_owned(&self, actor: &ActorId) -> Option<ActorId>;
    /// Release an actor.
    fn release(&self, actor: &ActorId);
    /// Subscribe to releases; returns a hook id.
    fn on_release(&self, callback: Rc<dyn Fn(&ActorId)>) -> u64;
    /// Remove a release hook.
    fn remove_release_hook(&self, id: u64);
}

/// Attacker/inflictor/target provenance builder.
type CombatProvenanceFn = Rc<dyn Fn(Option<&ActorId>, Option<&ActorId>, &ActorId) -> AuthorityProvenance>;
/// After-free hook.
type CombatAfterFreeFn = Rc<dyn Fn(i32, &mut QvmFunctionCall)>;

/// Joined source callbacks.
#[derive(Clone)]
pub struct CombatSource {
    /// Session actor registry.
    pub actors: Rc<dyn CombatActorRegistry>,
    /// Resolve the actor owning a slot.
    pub actor: Rc<dyn Fn(usize) -> Option<ActorId>>,
    /// Build provenance for attacker/inflictor/target.
    pub provenance: CombatProvenanceFn,
    /// After-free hook.
    pub after_free: Option<CombatAfterFreeFn>,
}

/// Slot lookup for an actor.
type CombatSlotFn = Rc<dyn Fn(&ActorId) -> Option<usize>>;

/// Combat binding options.
#[derive(Clone)]
pub struct CombatBindingOptions {
    /// Guest module.
    pub module: QvmModule,
    /// Located game data.
    pub data: QvmGameData,
    /// Source artifact.
    pub artifact: QvmArtifact,
    /// Combat profile.
    pub definition: QvmPrimaryCombatProfile,
    /// Body source.
    pub bodies: Rc<dyn CombatBodySource>,
    /// Gameplay authority.
    pub combat: Rc<dyn CombatAuthority>,
    /// Joined source, if any.
    pub source: Option<CombatSource>,
    /// Resolve the slot owning an actor.
    pub slot: CombatSlotFn,
}

/// Declared private combat fields plus the Q3 client pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmPrimaryCombatFields {
    /// In-use word offset.
    pub inuse: usize,
    /// Health word offset.
    pub health: usize,
    /// Take-damage word offset.
    pub takedamage: usize,
    /// Parent pointer offset.
    pub parent: usize,
    /// Q3 client pointer offset.
    pub client: usize,
}

/// Declared reaction fields and calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmPrimaryCombatReactions {
    /// Flags word offset.
    pub flags: usize,
    /// Pain entry offset.
    pub pain: usize,
    /// Death entry offset.
    pub die: usize,
    /// Pain reaction call.
    pub pain_call: QvmReactionCall,
    /// Death reaction call.
    pub die_call: QvmReactionCall,
}

/// Declared team projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmPrimaryCombatTeam {
    /// Persistent stat holding the team value.
    pub persistent_stat: i32,
    /// Source value mappings.
    pub values: Vec<QvmCombatTeam>,
}

/// Declared entity flag masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmPrimaryCombatFlags {
    /// Notarget mask.
    pub notarget: i32,
    /// Invulnerable mask.
    pub invulnerable: i32,
    /// No-knockback mask.
    pub no_knockback: i32,
}

/// Declared shared-combat state projection.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPrimaryCombatState {
    /// Health stat.
    pub health_stat: i32,
    /// Team projection.
    pub team: QvmPrimaryCombatTeam,
    /// Flag masks.
    pub flags: QvmPrimaryCombatFlags,
    /// Mass selector.
    pub mass: QvmCombatMass,
}

/// Primary combat profile: game combat plus armor, reactions, and state.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPrimaryCombatProfile {
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
    pub fields: QvmPrimaryCombatFields,
    /// Callbacks.
    pub callbacks: super::game_combat::QvmCombatCallbacks,
    /// Armor definition.
    pub armor: QvmGameArmorDefinition,
    /// Reactions.
    pub reactions: QvmPrimaryCombatReactions,
    /// Grapple damage method.
    pub grapple_damage_method: i32,
    /// State projection.
    pub state: QvmPrimaryCombatState,
    /// Damage-flag masks.
    pub damage_flags: QvmDamageFlags,
}

impl QvmPrimaryCombatProfile {
    fn game_definition(&self) -> QvmGameCombatDefinition {
        QvmGameCombatDefinition {
            damage_call: self.damage_call.clone(),
            module: self.module.clone(),
            abi_profile: self.abi_profile,
            entity_stride: self.entity_stride,
            client_stride: self.client_stride,
            fields: super::game_combat::QvmCombatFields {
                inuse: self.fields.inuse,
                health: self.fields.health,
                takedamage: self.fields.takedamage,
                parent: self.fields.parent,
            },
            callbacks: self.callbacks,
        }
    }
}

struct IncomingDamage {
    request: AuthorityDamageRequest,
    observer: SharedDamageObserver,
    result: Rc<RefCell<SourceDamageResult>>,
}

struct DamageContext {
    pointer: i32,
    request: AuthorityDamageRequest,
}

struct BindingsInner {
    options: CombatBindingOptions,
    source: QvmGameCombat,
    admitted: HashMap<usize, ActorId>,
    powered: HashMap<ActorId, ArmorIntercept>,
    regular: HashMap<ActorId, ArmorIntercept>,
    removals: Vec<u64>,
    release_hook: Option<u64>,
    remove_armor: Option<u64>,
    incoming: Option<IncomingDamage>,
    contexts: Vec<DamageContext>,
    closed: bool,
}

fn is_namespaced_team(value: &str) -> bool {
    match value.find(':') {
        Some(colon) => colon > 0 && colon + 1 < value.len() && !value[..colon].contains(':'),
        None => false,
    }
}

fn is_single_mask(value: i32) -> bool {
    let bits = value as u32;
    bits != 0 && bits & (bits - 1) == 0
}

fn is_binary32_fraction(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value) && f64::from(value as f32) == value
}

/// Qualified `G_Damage` calls retain their original stores, armor, and reactions.
#[derive(Clone)]
pub struct QvmCombatBindings {
    inner: Rc<RefCell<BindingsInner>>,
    scopes: QvmDamageScopes,
}

impl std::fmt::Debug for QvmCombatBindings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QvmCombatBindings")
            .field("admitted", &self.inner.borrow().admitted.len())
            .finish()
    }
}

impl QvmCombatBindings {
    /// Bind combat, validating the profile and installing damage hooks.
    pub fn bind(options: CombatBindingOptions) -> Result<Self, GuestError> {
        let definition = &options.definition;
        let source = QvmGameCombat::bind(
            &options.module,
            &options.data,
            &options.artifact,
            definition.game_definition(),
        )?;
        for field in [
            definition.reactions.flags,
            definition.reactions.pain,
            definition.reactions.die,
        ] {
            if field % 4 != 0 || field + 4 > definition.entity_stride {
                return Err(GuestError::invalid(
                    "Source combat reaction field is outside its entity record",
                ));
            }
        }
        let armor = &definition.armor;
        if options
            .artifact
            .image
            .instruction(armor.check_armor)
            .is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
        {
            return Err(GuestError::invalid(
                "Source CheckArmor declaration is not a function entry",
            ));
        }
        let client = definition.fields.client;
        if !client.is_multiple_of(4)
            || client < qvm_shared_entity_bytes(definition.abi_profile)
            || client + 4 > definition.entity_stride
        {
            return Err(GuestError::invalid(
                "Source combat requires its declared Q3 client pointer",
            ));
        }
        validate_qvm_combat_call(&armor.call, options.artifact.image.data_end())?;
        for call in [&definition.reactions.pain_call, &definition.reactions.die_call] {
            validate_qvm_combat_positions(&[call.target, call.amount], call.arguments)?;
        }
        let stat = |index: i32| -> Result<(), GuestError> {
            if !(0..16).contains(&index) {
                return Err(GuestError::invalid(
                    "Source armor stat is outside the public player record",
                ));
            }
            Ok(())
        };
        let protection = |value: f64| -> Result<(), GuestError> {
            if !is_binary32_fraction(value) {
                return Err(GuestError::invalid(
                    "Source armor protection must be a binary32 fraction",
                ));
            }
            Ok(())
        };
        stat(armor.points_stat)?;
        protection(f64::from(armor.protection))?;
        stat(definition.state.health_stat)?;
        stat(definition.state.team.persistent_stat)?;
        for values in [
            [
                definition.state.flags.notarget,
                definition.state.flags.invulnerable,
                definition.state.flags.no_knockback,
            ]
            .as_slice(),
            [
                definition.damage_flags.radius,
                definition.damage_flags.no_armor,
                definition.damage_flags.no_knockback,
                definition.damage_flags.no_protection,
                definition.damage_flags.no_team_protection,
            ]
            .as_slice(),
        ] {
            for value in values {
                if !is_single_mask(*value) {
                    return Err(GuestError::invalid(
                        "Source combat flags require individual 32-bit masks",
                    ));
                }
            }
            let unique: HashSet<i32> = values.iter().copied().collect();
            if unique.len() != values.len() {
                return Err(GuestError::invalid("Source combat flags overlap"));
            }
        }
        let mut teams = HashSet::new();
        for value in &definition.state.team.values {
            if teams.contains(&value.value) || !is_namespaced_team(&value.team) {
                return Err(GuestError::invalid(
                    "Source team mappings require unique values and explicit identities",
                ));
            }
            teams.insert(value.value);
        }
        match &definition.state.mass {
            QvmCombatMass::Constant { value } => {
                if !value.is_finite() || *value < 0.0 {
                    return Err(GuestError::invalid("Source mass must be finite and nonnegative"));
                }
            }
            QvmCombatMass::Entity { offset, .. } => {
                if offset % 4 != 0
                    || *offset < qvm_shared_entity_bytes(definition.abi_profile)
                    || offset + 4 > definition.entity_stride
                {
                    return Err(GuestError::invalid("Source mass field is outside its entity record"));
                }
            }
        }
        if let Some(tiers) = &armor.tiers {
            stat(tiers.stat)?;
            protection(f64::from(tiers.fallback))?;
            if tiers.stat == armor.points_stat || tiers.when_any.is_empty() || tiers.values.is_empty() {
                return Err(GuestError::invalid(
                    "Source armor tier declaration is incomplete or aliases its points",
                ));
            }
            let mut seen = HashSet::new();
            for entry in &tiers.values {
                if !seen.insert(entry.tier) {
                    return Err(GuestError::invalid(
                        "Source armor tiers require unique signed integer values",
                    ));
                }
                protection(f64::from(entry.protection))?;
            }
            for condition in &tiers.when_any {
                if condition.offset % 4 != 0 || condition.offset + 4 > options.artifact.image.data_end() {
                    return Err(GuestError::invalid("Source armor mode word is outside source data"));
                }
            }
        }

        let inner = Rc::new(RefCell::new(BindingsInner {
            options: options.clone(),
            source,
            admitted: HashMap::new(),
            powered: HashMap::new(),
            regular: HashMap::new(),
            removals: Vec::new(),
            release_hook: None,
            remove_armor: None,
            incoming: None,
            contexts: Vec::new(),
            closed: false,
        }));
        for role in [
            QvmDamageRole::Target,
            QvmDamageRole::Inflictor,
            QvmDamageRole::Attacker,
            QvmDamageRole::Direction,
            QvmDamageRole::Point,
            QvmDamageRole::Amount,
            QvmDamageRole::Flags,
            QvmDamageRole::Method,
        ] {
            definition.damage_call.role(role.name())?;
        }
        for role in [QvmArmorRole::Target, QvmArmorRole::Amount, QvmArmorRole::Flags] {
            armor.call.role(role.name())?;
        }
        let scopes = {
            let scoped = Rc::clone(&inner);
            let armor_of = move |slot: usize| Self::armor_of(&scoped.borrow(), slot);
            let scoped = Rc::clone(&inner);
            let live = move |actor: &ActorId| Self::live_of(&scoped.borrow(), actor);
            let definition = options.definition.clone();
            QvmDamageScopes::new(QvmDamageScopeOptions {
                module: options.module.clone(),
                data: options.data.clone(),
                health: definition.fields.health,
                target_argument: definition.damage_call.role(QvmDamageRole::Target.name()).unwrap_or(0),
                points_stat: definition.armor.points_stat,
                tier_stat: definition.armor.tiers.as_ref().map(|tiers| tiers.stat),
                mode_words: definition
                    .armor
                    .tiers
                    .as_ref()
                    .map(|tiers| tiers.when_any.iter().map(|condition| condition.offset).collect())
                    .unwrap_or_default(),
                reactions: QvmDamageReactions {
                    pain: definition.reactions.pain,
                    die: definition.reactions.die,
                    pain_call: definition.reactions.pain_call.clone(),
                    die_call: definition.reactions.die_call.clone(),
                },
                armor: Rc::new(armor_of),
                live: Rc::new(live),
            })
        };
        let bindings = Self { inner, scopes };
        let module = bindings.inner.borrow().options.module.clone();
        let damage_entry = bindings.inner.borrow().options.definition.callbacks.damage;
        let hook: QvmHookFn = {
            let bindings = bindings.clone();
            Rc::new(move |call| bindings.enter_damage(call))
        };
        bindings
            .inner
            .borrow_mut()
            .removals
            .push(module.bind_invocation(damage_entry, hook));
        if bindings.inner.borrow().options.source.is_some() {
            let free_entry = bindings.inner.borrow().options.definition.callbacks.free;
            let hook: QvmHookFn = {
                let bindings = bindings.clone();
                Rc::new(move |call| bindings.after_free(call))
            };
            let id = module.bind_invocation(free_entry, hook);
            bindings.inner.borrow_mut().removals.push(id);
            let release: Rc<dyn Fn(&ActorId)> = {
                let bindings = bindings.clone();
                Rc::new(move |actor| bindings.released(actor))
            };
            let registry = bindings
                .inner
                .borrow()
                .options
                .source
                .clone()
                .map(|source| source.actors);
            if let Some(registry) = registry {
                let id = registry.on_release(release);
                bindings.inner.borrow_mut().release_hook = Some(id);
            }
        }
        Ok(bindings)
    }

    fn armor_of(inner: &BindingsInner, slot: usize) -> ArmorState {
        let none = ArmorState {
            regular: RegularArmor::None,
            powered: PoweredProtection::None,
        };
        if slot >= inner.options.data.num_clients() {
            return none;
        }
        let Ok(state) = inner.options.data.copy_player_state(slot) else {
            return none;
        };
        let tiers = Self::active_tiers_of(inner);
        ArmorState {
            regular: RegularArmor::Q3 {
                points: f64::from(
                    state
                        .stats
                        .get(inner.options.definition.armor.points_stat as usize)
                        .copied()
                        .unwrap_or(0),
                ),
                protection: Self::protection_fraction_of(inner, &state.stats, tiers.as_ref()),
            },
            powered: PoweredProtection::None,
        }
    }

    fn live_of(inner: &BindingsInner, actor: &ActorId) -> bool {
        if inner.closed {
            return false;
        }
        let Some(slot) = (inner.options.slot)(actor) else {
            return false;
        };
        let occupied = inner.source.state(slot).map(|state| state.is_some()).unwrap_or(false);
        if !occupied {
            return false;
        }
        match &inner.options.source {
            None => true,
            Some(source) => source.actors.resolve_owned(actor).as_ref() == Some(actor),
        }
    }

    fn active_tiers_of(inner: &BindingsInner) -> Option<super::game_combat::QvmArmorTiers> {
        let tiers = inner.options.definition.armor.tiers.clone()?;
        let active = tiers.when_any.iter().any(|condition| {
            let value = inner.options.module.memory().read_i32(condition.offset).unwrap_or(0);
            if condition.equal {
                value == condition.value
            } else {
                value != condition.value
            }
        });
        active.then_some(tiers)
    }

    fn protection_fraction_of(
        inner: &BindingsInner,
        stats: &[i32; 16],
        tiers: Option<&super::game_combat::QvmArmorTiers>,
    ) -> f64 {
        match tiers {
            None => f64::from(inner.options.definition.armor.protection),
            Some(tiers) => {
                let current = stats.get(tiers.stat as usize).copied().unwrap_or(0);
                tiers
                    .values
                    .iter()
                    .find(|entry| entry.tier == current)
                    .map(|entry| f64::from(entry.protection))
                    .unwrap_or_else(|| f64::from(tiers.fallback))
            }
        }
    }

    /// Release hooks, intercepts, and admissions.
    pub fn close(&self) {
        let mut inner = self.inner.borrow_mut();
        if inner.closed {
            return;
        }
        inner.closed = true;
        inner.powered.clear();
        inner.regular.clear();
        inner.admitted.clear();
        if let Some(id) = inner.remove_armor.take() {
            inner.options.module.remove_hook(id);
        }
        let removals: Vec<u64> = std::mem::take(&mut inner.removals);
        for id in removals {
            inner.options.module.remove_hook(id);
        }
        if let Some(id) = inner.release_hook.take() {
            if let Some(source) = &inner.options.source {
                source.actors.remove_release_hook(id);
            }
        }
    }

    fn live(&self, actor: &ActorId) -> bool {
        Self::live_of(&self.inner.borrow(), actor)
    }

    fn armor(&self, slot: usize) -> ArmorState {
        Self::armor_of(&self.inner.borrow(), slot)
    }

    fn owned(&self, actor: &ActorId) -> Option<ActorId> {
        let inner = self.inner.borrow();
        let slot = (inner.options.slot)(actor)?;
        let owned = inner.admitted.get(&slot)?;
        if owned != actor {
            return None;
        }
        let owned = owned.clone();
        drop(inner);
        self.live(&owned).then_some(owned)
    }

    /// Read the notarget flag (`None` when the actor has no slot).
    pub fn notarget(&self, actor: &ActorId) -> Result<Option<bool>, GuestError> {
        let inner = self.inner.borrow();
        let Some(slot) = (inner.options.slot)(actor) else {
            return Ok(None);
        };
        let flags = inner
            .options
            .data
            .entity_bytes(slot)?
            .get_i32(inner.options.definition.reactions.flags)?;
        Ok(Some(flags & inner.options.definition.state.flags.notarget != 0))
    }

    /// Project legacy saves through the original source armor.
    pub fn normalize_legacy_armor(&self, actor: &ActorId, saved: &ArmorState) -> Result<ArmorState, GuestError> {
        let inner = self.inner.borrow();
        let slot = (inner.options.slot)(actor);
        if slot.is_none() || slot.is_some_and(|slot| slot >= 1022) {
            return Err(GuestError::invalid("Legacy QVM armor has no source actor"));
        }
        let slot = slot.expect("checked combat slot");
        let matches = if slot < inner.options.data.num_clients() {
            let stats = inner.options.data.copy_player_state(slot)?.stats;
            matches!(&saved.regular, RegularArmor::Q3 { points, protection }
                if *points == f64::from(stats[3]) && *protection == 0.66)
        } else {
            matches!(saved.regular, RegularArmor::None)
        };
        if !matches || !matches!(saved.powered, PoweredProtection::None) {
            return Err(GuestError::invalid(
                "Legacy QVM armor disagrees with the original source projection",
            ));
        }
        drop(inner);
        Ok(self.armor(slot))
    }

    /// Lower armor values to source words.
    pub fn armor_write(&self, slot: usize, armor: &ArmorState) -> Result<Option<ArmorWrite>, GuestError> {
        let inner = self.inner.borrow();
        if !matches!(armor.powered, PoweredProtection::None)
            || !matches!(armor.regular, RegularArmor::None | RegularArmor::Q3 { .. })
        {
            return Err(GuestError::invalid("Native Q3 armor requires Q3 armor values"));
        }
        if slot >= inner.options.data.num_clients() {
            if !matches!(armor.regular, RegularArmor::None) {
                return Err(GuestError::invalid("Source non-client has no player armor"));
            }
            return Ok(None);
        }
        let points = match &armor.regular {
            RegularArmor::None => 0.0,
            RegularArmor::Q3 { points, .. } => *points,
            _ => 0.0,
        };
        if points.fract() != 0.0 || points < 0.0 || points > f64::from(i32::MAX) {
            return Err(GuestError::invalid(
                "Source armor points require a nonnegative signed integer",
            ));
        }
        let points = points as i32;
        if matches!(armor.regular, RegularArmor::None) {
            return Ok(Some(ArmorWrite { points, tier: None }));
        }
        let RegularArmor::Q3 { protection, .. } = &armor.regular else {
            return Ok(Some(ArmorWrite { points, tier: None }));
        };
        let stats = inner.options.data.copy_player_state(slot)?.stats;
        let tiers = Self::active_tiers_of(&inner);
        let requested = f64::from(*protection as f32);
        if requested == Self::protection_fraction_of(&inner, &stats, tiers.as_ref()) {
            return Ok(Some(ArmorWrite { points, tier: None }));
        }
        let selected = tiers.as_ref().and_then(|tiers| {
            tiers
                .values
                .iter()
                .find(|entry| f64::from(entry.protection) == requested)
        });
        match (tiers.as_ref(), selected) {
            (Some(tiers), Some(selected)) => Ok(Some(ArmorWrite {
                points,
                tier: Some(ArmorTierWrite {
                    stat: tiers.stat,
                    value: selected.tier,
                }),
            })),
            _ => Err(GuestError::invalid(
                "Requested protection is not representable by the source armor mode",
            )),
        }
    }

    /// Admit an actor, binding or rebinding its combat record.
    pub fn admit(&self, actor: &ActorId) -> Result<(), GuestError> {
        let slot = {
            let inner = self.inner.borrow();
            match (inner.options.slot)(actor) {
                None => return Ok(()),
                Some(slot) if slot >= 1022 => return Ok(()),
                Some(slot) => slot,
            }
        };
        self.inner.borrow_mut().admitted.insert(slot, actor.clone());
        let reader = {
            let bindings = self.clone();
            let actor = actor.clone();
            Rc::new(move || bindings.read_binding(&actor, slot))
        };
        let damage = {
            let bindings = self.clone();
            Rc::new(move |request: AuthorityDamageRequest| bindings.damage(&request))
        };
        let validate_armor = {
            let bindings = self.clone();
            Rc::new(move |armor: &ArmorState| bindings.armor_write(slot, armor).map(|_| ()))
        };
        let write_health = {
            let bindings = self.clone();
            Rc::new(move |health: i32| bindings.write_health(slot, health))
        };
        let write_armor = {
            let bindings = self.clone();
            Rc::new(move |armor: &ArmorState| bindings.write_armor(slot, armor))
        };
        let (regular_stage, powered_stage, owner) = {
            let inner = self.inner.borrow();
            if inner.options.source.is_none() {
                (None, None, inner.options.definition.module.id.clone())
            } else {
                drop(inner);
                (
                    Some(self.armor_stage(actor, ProtectionChannel::Regular)),
                    Some(self.armor_stage(actor, ProtectionChannel::Powered)),
                    self.inner.borrow().options.definition.module.id.clone(),
                )
            }
        };
        let binding = CombatBinding {
            read: reader,
            damage,
            regular: ProtectionBinding {
                owner: Some(owner),
                stage: regular_stage,
            },
            powered: ProtectionBinding {
                owner: None,
                stage: powered_stage,
            },
            validate_armor,
            write_health,
            write_armor,
        };
        let inner = self.inner.borrow();
        if inner.options.combat.is_bound(actor) {
            inner.options.combat.rebind_combat(actor, &binding);
        } else {
            inner.options.combat.bind_combat(actor, &binding);
        }
        Ok(())
    }

    fn read_binding(&self, actor: &ActorId, slot: usize) -> Result<CombatBindingRead, GuestError> {
        let inner = self.inner.borrow();
        let state = inner.source.state(slot)?;
        let flags = inner
            .options
            .data
            .entity_bytes(slot)?
            .get_i32(inner.options.definition.reactions.flags)?;
        let team = if slot < inner.options.data.num_clients() {
            let players = inner.options.data.public_player_bytes(slot)?;
            Some(players.get_i32(248 + inner.options.definition.state.team.persistent_stat as usize * 4)?)
        } else {
            None
        };
        let mass = match &inner.options.definition.state.mass {
            QvmCombatMass::Constant { value } => *value,
            QvmCombatMass::Entity { offset, float_storage } => {
                let view = inner.options.data.entity_bytes(slot)?;
                if *float_storage {
                    f64::from(view.get_f32(*offset)?)
                } else {
                    f64::from(view.get_i32(*offset)?)
                }
            }
        };
        if !mass.is_finite() || mass < 0.0 {
            return Err(GuestError::invalid("Source mass is not representable by shared combat"));
        }
        let _ = actor;
        Ok(CombatBindingRead {
            health: state.map(|state| state.health).unwrap_or(0),
            can_take_damage: state.map(|state| state.damageable).unwrap_or(false),
            mass,
            armor: Self::armor_of(&inner, slot),
            team: team.and_then(|team| {
                inner
                    .options
                    .definition
                    .state
                    .team
                    .values
                    .iter()
                    .find(|value| value.value == team)
                    .map(|value| value.team.clone())
            }),
            invulnerable: flags & inner.options.definition.state.flags.invulnerable != 0,
            no_knockback: flags & inner.options.definition.state.flags.no_knockback != 0,
        })
    }

    fn write_health(&self, slot: usize, health: i32) {
        let inner = self.inner.borrow();
        let _ = inner.options.data.entity_bytes(slot).and_then(|view| {
            view.set_i32(inner.options.definition.fields.health, health)
                .map(|()| view)
        });
        if slot < inner.options.data.num_clients() {
            let _ = inner.options.data.public_player_bytes(slot).and_then(|players| {
                players.set_i32(184 + inner.options.definition.state.health_stat as usize * 4, health)
            });
        }
    }

    fn write_armor(&self, slot: usize, armor: &ArmorState) {
        let Ok(Some(write)) = self.armor_write(slot, armor) else {
            return;
        };
        let inner = self.inner.borrow();
        if slot >= inner.options.data.num_clients() {
            return;
        }
        if let Ok(players) = inner.options.data.public_player_bytes(slot) {
            let _ = players.set_i32(
                184 + inner.options.definition.armor.points_stat as usize * 4,
                write.points,
            );
            if let Some(tier) = write.tier {
                let _ = players.set_i32(184 + tier.stat as usize * 4, tier.value);
            }
        }
    }

    fn armor_stage(&self, actor: &ActorId, channel: ProtectionChannel) -> ArmorStage {
        let bindings = self.clone();
        let actor = actor.clone();
        ArmorStage {
            bind: Rc::new(move |intercept| -> Result<Box<dyn FnOnce()>, GuestError> {
                let mut inner = bindings.inner.borrow_mut();
                if !Self::live_of(&inner, &actor) {
                    return Err(GuestError::invalid("Source armor owner is retired"));
                }
                let map = match channel {
                    ProtectionChannel::Regular => &mut inner.regular,
                    ProtectionChannel::Powered => &mut inner.powered,
                };
                if map.contains_key(&actor) {
                    return Err(GuestError::invalid(format!(
                        "Source {channel:?} armor stage already has an owner"
                    )));
                }
                map.insert(actor.clone(), intercept);
                if inner.remove_armor.is_none() {
                    let check = inner.options.definition.armor.check_armor;
                    let hook: QvmHookFn = {
                        let bindings = bindings.clone();
                        Rc::new(move |call| bindings.check_armor(call))
                    };
                    inner.remove_armor = Some(inner.options.module.bind_function(check, hook));
                }
                let bindings = bindings.clone();
                let actor = actor.clone();
                Ok(Box::new(move || {
                    let inner = bindings.inner.borrow();
                    let current = match channel {
                        ProtectionChannel::Regular => inner.regular.get(&actor),
                        ProtectionChannel::Powered => inner.powered.get(&actor),
                    }
                    .cloned();
                    drop(inner);
                    if current.is_some() {
                        let mut inner = bindings.inner.borrow_mut();
                        let map = match channel {
                            ProtectionChannel::Regular => &mut inner.regular,
                            ProtectionChannel::Powered => &mut inner.powered,
                        };
                        map.remove(&actor);
                    }
                    bindings.remove_unused_armor_hook();
                }) as Box<dyn FnOnce()>)
            }),
        }
    }

    fn remove_unused_armor_hook(&self) {
        let mut inner = self.inner.borrow_mut();
        if !inner.powered.is_empty() || !inner.regular.is_empty() {
            return;
        }
        if let Some(id) = inner.remove_armor.take() {
            inner.options.module.remove_hook(id);
        }
    }

    fn vector(&self, word: i32) -> Vec3 {
        if word == 0 {
            return Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        }
        usize::try_from(word)
            .ok()
            .and_then(|offset| self.inner.borrow().options.module.memory().read_vec3(offset).ok())
            .unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 })
    }

    fn check_armor(&self, call: &mut QvmFunctionCall) -> i32 {
        let (target_role, amount_role, flags_role, no_armor_mask) = {
            let inner = self.inner.borrow();
            (
                inner
                    .options
                    .definition
                    .armor
                    .call
                    .role(QvmArmorRole::Target.name())
                    .unwrap_or(0),
                inner
                    .options
                    .definition
                    .armor
                    .call
                    .role(QvmArmorRole::Amount.name())
                    .unwrap_or(0),
                inner
                    .options
                    .definition
                    .armor
                    .call
                    .role(QvmArmorRole::Flags.name())
                    .unwrap_or(0),
                inner.options.definition.damage_flags.no_armor,
            )
        };
        let target = call.argument(target_role).unwrap_or(-1);
        let Some(frame) = self.scopes.current(target) else {
            return call.proceed();
        };
        let actor = frame.actor();
        let (power, has_regular) = {
            let inner = self.inner.borrow();
            (inner.powered.get(&actor).cloned(), inner.regular.contains_key(&actor))
        };
        if power.is_none() && !has_regular {
            return call.proceed();
        }
        if !self.live(&actor) {
            self.scopes.cancel(&frame, call);
            return 0;
        }
        let context = self
            .inner
            .borrow()
            .contexts
            .iter()
            .rev()
            .find(|context| context.pointer == frame.pointer())
            .map(|context| context.request.clone());
        let Some(request) = context else {
            return call.proceed();
        };
        let amount = call.argument(amount_role).unwrap_or(0);
        let flags = call.argument(flags_role).unwrap_or(0);
        let originating = qvm_attack_damage_flags(&request.to_combat_request());
        let geometry = ArmorHitGeometry {
            direction: request.direction,
            point: request.point,
            normal: request.normal,
        };
        let valid = |saved: f64, amount: i32| saved.is_finite() && saved >= 0.0 && saved <= f64::from(amount.max(0));
        let power_saved = match power {
            None => 0,
            Some(power) => {
                let hit = ArmorHit {
                    request: request.clone(),
                    amount,
                    geometry: geometry.clone(),
                    flags: ArmorHitFlags {
                        stage: ArmorStageKind::Power,
                        no_armor: flags & no_armor_mask != 0,
                        no_power_armor: originating.no_power_armor,
                        no_regular_armor: false,
                        energy: originating.energy,
                        regular_protection_scale: 1.0,
                    },
                };
                let saved = power(&hit, &mut || 0);
                if valid(saved, amount) {
                    saved as i32
                } else {
                    0
                }
            }
        };
        if !self.live(&actor) {
            self.scopes.cancel(&frame, call);
            return 0;
        }
        let remaining = amount - power_saved;
        let amount_index = amount_role;
        if let Some(word) = call.words.get_mut(amount_index) {
            *word = remaining;
        }
        let regular_saved = {
            let regular = self.inner.borrow().regular.get(&actor).cloned();
            match regular {
                None => call.proceed(),
                Some(regular) => {
                    let current_flags = call.argument(flags_role).unwrap_or(0);
                    let hit = ArmorHit {
                        request: request.clone(),
                        amount: remaining,
                        geometry,
                        flags: ArmorHitFlags {
                            stage: ArmorStageKind::Regular,
                            no_armor: current_flags & no_armor_mask != 0,
                            no_power_armor: originating.no_power_armor,
                            no_regular_armor: originating.no_regular_armor,
                            energy: originating.energy,
                            regular_protection_scale: originating.regular_protection_scale,
                        },
                    };
                    let saved = regular(&hit, &mut || call.proceed());
                    if valid(saved, remaining) {
                        saved as i32
                    } else {
                        call.proceed()
                    }
                }
            }
        };
        if let Some(word) = call.words.get_mut(amount_index) {
            *word = amount;
        }
        if !self.live(&actor) {
            self.scopes.cancel(&frame, call);
            return 0;
        }
        power_saved + regular_saved
    }

    fn source_actor(&self, pointer: i32) -> Option<ActorId> {
        let inner = self.inner.borrow();
        let source = inner.options.source.clone()?;
        if pointer == 0 {
            return None;
        }
        let slot = inner.options.data.number_from_pointer(pointer).ok()?;
        if slot >= 1022 {
            return None;
        }
        inner.source.state(slot).ok()??;
        (source.actor)(slot)
    }

    fn enter_damage(&self, call: &mut QvmFunctionCall) -> i32 {
        let incoming = self.inner.borrow_mut().incoming.take();
        if let Some(incoming) = incoming {
            let actor = self.owned(&incoming.request.target);
            let slot = self.inner.borrow().options.slot.clone()(&incoming.request.target);
            if let (Some(actor), Some(slot)) = (actor, slot) {
                let outcome = self.run_scoped(call, &actor, slot, &incoming.request, &incoming.observer);
                *incoming.result.borrow_mut() = outcome;
            }
            return 0;
        }
        if self.inner.borrow().options.source.is_none() {
            return call.proceed();
        }
        let roles = self.inner.borrow().options.definition.damage_call.clone();
        let role = |role: QvmDamageRole| roles.role(role.name()).unwrap_or(0);
        let target = self.source_actor(call.argument(role(QvmDamageRole::Target)).unwrap_or(0));
        let Some(target) = target else {
            return call.proceed();
        };
        let inflictor = self.source_actor(call.argument(role(QvmDamageRole::Inflictor)).unwrap_or(0));
        let attacker = self.source_actor(call.argument(role(QvmDamageRole::Attacker)).unwrap_or(0));
        let flags = call.argument(role(QvmDamageRole::Flags)).unwrap_or(0);
        let amount = call.argument(role(QvmDamageRole::Amount)).unwrap_or(0);
        let direction_word = call.argument(role(QvmDamageRole::Direction)).unwrap_or(0);
        let (masks, method) = {
            let inner = self.inner.borrow();
            (
                inner.options.definition.damage_flags,
                call.argument(role(QvmDamageRole::Method)).unwrap_or(0),
            )
        };
        let provenance = self
            .inner
            .borrow()
            .options
            .source
            .clone()
            .map(|source| (source.provenance)(attacker.as_ref(), inflictor.as_ref(), &target));
        let Some(provenance) = provenance else {
            return call.proceed();
        };
        let request = AuthorityDamageRequest {
            target: target.clone(),
            amount: f64::from(amount),
            knockback: if flags & masks.no_knockback != 0 || direction_word == 0 {
                0.0
            } else {
                f64::from(amount)
            },
            direction: self.vector(direction_word),
            point: self.vector(call.argument(role(QvmDamageRole::Point)).unwrap_or(0)),
            normal: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            delivery: if flags & masks.radius != 0 {
                DamageDelivery::Radius
            } else {
                DamageDelivery::Direct
            },
            attack: AuthorityAttack {
                cause: AuthorityCause::Q3 {
                    means_of_death: method,
                    damage_flags: self.canonical_flags(flags),
                },
                attacker,
                inflictor,
                weapon: provenance.weapon,
                movement_provider: provenance.movement_provider,
            },
        };
        let combat = self.inner.borrow().options.combat.clone();
        let bindings = self.clone();
        combat.apply(&request, &mut |composed| {
            combat.run_source_damage(composed, &mut |observer, effective| {
                bindings.enter_composed(call, &request, observer, effective, flags)
            })
        });
        0
    }

    fn enter_composed(
        &self,
        call: &mut QvmFunctionCall,
        request: &AuthorityDamageRequest,
        observer: &SharedDamageObserver,
        effective: &AuthorityDamageRequest,
        flags: i32,
    ) -> SourceDamageResult {
        let zeros = SourceDamageResult {
            applied_damage: 0,
            reaction: QvmDamageReaction::None,
        };
        let actor = self.owned(&effective.target);
        let slot = self.inner.borrow().options.slot.clone()(&effective.target);
        let (Some(actor), Some(slot)) = (actor, slot) else {
            return zeros;
        };
        if effective == request {
            return self.run_scoped(call, &actor, slot, effective, observer);
        }
        let lowered = match self.lower(effective, slot, flags) {
            Ok(lowered) => lowered,
            Err(_) => return zeros,
        };
        let roles = self.inner.borrow().options.definition.damage_call.clone();
        let source = self.inner.borrow().source.clone();
        let outcome = Rc::new(RefCell::new(zeros));
        let outcome_hook = Rc::clone(&outcome);
        let role_index = |role: QvmDamageRole| roles.role(role.name()).unwrap_or(usize::MAX);
        let _ = source.damage(
            &lowered,
            Some(&mut |words: &[i32]| {
                let saved: Vec<(usize, i32)> = roles
                    .roles
                    .iter()
                    .map(|(_, index)| (*index, call.words.get(*index).copied().unwrap_or(0)))
                    .collect();
                for (index, _) in &saved {
                    let unchanged = (*index == role_index(QvmDamageRole::Direction)
                        && effective.direction == request.direction)
                        || (*index == role_index(QvmDamageRole::Point) && effective.point == request.point)
                        || (*index == role_index(QvmDamageRole::Attacker)
                            && effective.attack.attacker == request.attack.attacker)
                        || (*index == role_index(QvmDamageRole::Inflictor)
                            && effective.attack.inflictor == request.attack.inflictor);
                    if unchanged {
                        continue;
                    }
                    if let (Some(slot), Some(word)) = (call.words.get_mut(*index), words.get(*index)) {
                        *slot = *word;
                    }
                }
                let result = self.run_scoped(call, &actor, slot, effective, observer);
                *outcome_hook.borrow_mut() = result;
                for (index, word) in &saved {
                    if let Some(slot) = call.words.get_mut(*index) {
                        *slot = *word;
                    }
                }
                Ok(())
            }),
        );
        let copied = *outcome.borrow();
        copied
    }

    fn after_free(&self, call: &mut QvmFunctionCall) -> i32 {
        let pointer = call.argument(0).unwrap_or(0);
        let (slot, actor) = {
            let inner = self.inner.borrow();
            let slot = inner.options.data.number_from_pointer(pointer).ok();
            let actor = slot.and_then(|slot| inner.admitted.get(&slot).cloned());
            (slot, actor)
        };
        let result = call.proceed();
        if let (Some(slot), Some(actor)) = (slot, actor) {
            let release = {
                let inner = self.inner.borrow();
                let owned = inner
                    .options
                    .source
                    .as_ref()
                    .and_then(|source| source.actors.resolve_owned(&actor))
                    .as_ref()
                    == Some(&actor);
                let freed = inner.source.state(slot).map(|state| state.is_none()).unwrap_or(false);
                owned && freed
            };
            if release {
                let source = self.inner.borrow().options.source.clone();
                if let Some(source) = source {
                    source.actors.release(&actor);
                }
            }
        }
        let after_free = self
            .inner
            .borrow()
            .options
            .source
            .clone()
            .and_then(|source| source.after_free);
        if let Some(after_free) = after_free {
            after_free(pointer, call);
        }
        result
    }

    fn released(&self, actor: &ActorId) {
        {
            let mut inner = self.inner.borrow_mut();
            inner.powered.remove(actor);
            inner.regular.remove(actor);
            inner.admitted.retain(|_, current| current != actor);
        }
        self.remove_unused_armor_hook();
    }

    fn canonical_flags(&self, source: i32) -> i32 {
        let inner = self.inner.borrow();
        qvm_canonical_damage_flags(&inner.options.definition.damage_flags, source)
    }

    fn lower(&self, request: &AuthorityDamageRequest, slot: usize, original: i32) -> Result<QvmGameDamage, GuestError> {
        let inner = self.inner.borrow();
        let inflictor = request.attack.inflictor.as_ref();
        let source_inflictor = inflictor.and_then(|actor| (inner.options.slot)(actor));
        let inflictor_body = inflictor.and_then(|actor| inner.options.bodies.read(actor));
        let lowered = qvm_source_damage_flags(
            &inner.options.definition.damage_flags,
            &request.to_combat_request(),
            original,
        );
        let amount = request.amount.trunc();
        if amount < f64::from(i32::MIN) || amount > f64::from(i32::MAX) {
            return Err(GuestError::invalid(
                "Source damage arguments require signed integer words",
            ));
        }
        let grapple_weapons = [
            "q3:weapon_grapplinghook",
            "q2:weapon_grapple",
            "q2:weapon_hook",
            "ctf:grapple",
        ];
        let method = if request
            .attack
            .weapon
            .as_deref()
            .is_some_and(|weapon| grapple_weapons.contains(&weapon))
        {
            inner.options.definition.grapple_damage_method
        } else {
            match request.attack.cause {
                AuthorityCause::Q3 { means_of_death, .. } => means_of_death,
                AuthorityCause::Q2 { means_of_death: 56, .. } => inner.options.definition.grapple_damage_method,
                _ => 0,
            }
        };
        Ok(QvmGameDamage {
            target: slot,
            attacker: request
                .attack
                .attacker
                .as_ref()
                .and_then(|actor| (inner.options.slot)(actor)),
            inflictor: match (source_inflictor, inflictor_body) {
                (Some(slot), _) => Some(QvmGameInflictor::Entity { slot }),
                (None, Some(body)) => Some(QvmGameInflictor::Foreign { body }),
                (None, None) => None,
            },
            direction: request.direction,
            point: request.point,
            amount: amount as i32,
            flags: lowered,
            method,
        })
    }

    fn run_scoped(
        &self,
        call: &mut QvmFunctionCall,
        actor: &ActorId,
        slot: usize,
        request: &AuthorityDamageRequest,
        observer: &SharedDamageObserver,
    ) -> SourceDamageResult {
        let pointer = {
            let inner = self.inner.borrow();
            let role = inner
                .options
                .definition
                .damage_call
                .role(QvmDamageRole::Target.name())
                .unwrap_or(0);
            call.argument(role).unwrap_or(0)
        };
        self.inner.borrow_mut().contexts.push(DamageContext {
            pointer,
            request: request.clone(),
        });
        let outcome = self
            .scopes
            .run(call, actor, slot, &request.to_scope_request(), observer, None);
        self.inner.borrow_mut().contexts.pop();
        outcome.unwrap_or(SourceDamageResult {
            applied_damage: 0,
            reaction: QvmDamageReaction::None,
        })
    }

    fn damage(&self, input: &AuthorityDamageRequest) -> SourceDamageResult {
        let zeros = SourceDamageResult {
            applied_damage: 0,
            reaction: QvmDamageReaction::None,
        };
        let combat = self.inner.borrow().options.combat.clone();
        let bindings = self.clone();
        combat.run_source_damage(input, &mut |observer, request| {
            let slot = bindings.inner.borrow().options.slot.clone()(&request.target);
            let result = Rc::new(RefCell::new(zeros));
            let Some(slot) = slot else {
                return *result.borrow();
            };
            let lowered = match bindings.lower(request, slot, 0) {
                Ok(lowered) => lowered,
                Err(_) => return *result.borrow(),
            };
            let source = bindings.inner.borrow().source.clone();
            let entry = bindings.inner.borrow().options.definition.callbacks.damage;
            let module = bindings.inner.borrow().options.module.clone();
            let mut pending = Some(IncomingDamage {
                request: request.clone(),
                observer: Rc::clone(observer),
                result: Rc::clone(&result),
            });
            let _ = source.damage(
                &lowered,
                Some(&mut |words: &[i32]| {
                    let previous = bindings.inner.borrow_mut().incoming.take();
                    bindings.inner.borrow_mut().incoming = pending.take();
                    let outcome = module.call(words, entry);
                    let _ = bindings.inner.borrow_mut().incoming.take();
                    bindings.inner.borrow_mut().incoming = previous;
                    outcome.map(|_| ())
                }),
            );
            let copied = *result.borrow();
            copied
        })
    }
}

/// Lowered armor write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmorWrite {
    /// Armor points.
    pub points: i32,
    /// Tier write, if the mode changes.
    pub tier: Option<ArmorTierWrite>,
}

/// Lowered armor tier write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmorTierWrite {
    /// Tier stat.
    pub stat: i32,
    /// Tier value.
    pub value: i32,
}

#[cfg(test)]
mod tests {
    use super::super::game_combat::{QvmArmorTiers, QvmCombatCallbacks, QvmCombatExtra};
    use super::super::game_combat_scope::{QvmStoredDamage, SourceDamageObserver};
    use super::super::game_data::{AbiProfile, QvmImage, QvmInstruction, QvmRole, QvmSharedMemory};
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    struct FixtureObserver {
        stored: Vec<QvmStoredDamage>,
        reactions: Vec<SourceDamageResult>,
    }

    impl SourceDamageObserver for FixtureObserver {
        fn stored(&mut self, stored: QvmStoredDamage) {
            self.stored.push(stored);
        }

        fn before_reaction(&mut self, result: &SourceDamageResult) {
            self.reactions.push(*result);
        }
    }

    struct FixtureAuthority {
        bound: RefCell<HashMap<ActorId, CombatBinding>>,
    }

    impl CombatAuthority for FixtureAuthority {
        fn is_bound(&self, actor: &ActorId) -> bool {
            self.bound.borrow().contains_key(actor)
        }

        fn bind_combat(&self, actor: &ActorId, binding: &CombatBinding) {
            self.bound.borrow_mut().insert(actor.clone(), binding.clone());
        }

        fn rebind_combat(&self, actor: &ActorId, binding: &CombatBinding) {
            self.bound.borrow_mut().insert(actor.clone(), binding.clone());
        }

        fn apply(
            &self,
            request: &AuthorityDamageRequest,
            run: &mut dyn FnMut(&AuthorityDamageRequest) -> SourceDamageResult,
        ) -> SourceDamageResult {
            run(request)
        }

        fn run_source_damage(
            &self,
            request: &AuthorityDamageRequest,
            run: &mut dyn FnMut(&SharedDamageObserver, &AuthorityDamageRequest) -> SourceDamageResult,
        ) -> SourceDamageResult {
            let observer: SharedDamageObserver = Rc::new(RefCell::new(FixtureObserver {
                stored: Vec::new(),
                reactions: Vec::new(),
            }));
            run(&observer, request)
        }
    }

    /// Release hooks by id.
    type FixtureReleaseHooks = RefCell<Vec<(u64, Rc<dyn Fn(&ActorId)>)>>;

    struct FixtureRegistry {
        owned: RefCell<HashMap<ActorId, ActorId>>,
        hooks: FixtureReleaseHooks,
        next: RefCell<u64>,
        released: RefCell<Vec<ActorId>>,
    }

    impl CombatActorRegistry for FixtureRegistry {
        fn resolve_owned(&self, actor: &ActorId) -> Option<ActorId> {
            self.owned.borrow().get(actor).cloned()
        }

        fn release(&self, actor: &ActorId) {
            self.owned.borrow_mut().remove(actor);
            self.released.borrow_mut().push(actor.clone());
            for (_, hook) in self.hooks.borrow().iter() {
                hook(actor);
            }
        }

        fn on_release(&self, callback: Rc<dyn Fn(&ActorId)>) -> u64 {
            let mut next = self.next.borrow_mut();
            *next += 1;
            self.hooks.borrow_mut().push((*next, callback));
            *next
        }

        fn remove_release_hook(&self, id: u64) {
            self.hooks.borrow_mut().retain(|(known, _)| *known != id);
        }
    }

    struct FixtureBodies;

    impl CombatBodySource for FixtureBodies {
        fn read(&self, _actor: &ActorId) -> Option<BodyState> {
            None
        }
    }

    struct Fixture {
        bindings: QvmCombatBindings,
        memory: QvmSharedMemory,
        actor: ActorId,
        authority: Rc<FixtureAuthority>,
        registry: Rc<FixtureRegistry>,
    }

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
            extras: Vec::<QvmCombatExtra>::new(),
        }
    }

    fn armor_call() -> QvmCombatCall {
        QvmCombatCall {
            roles: ["target", "amount", "flags"]
                .iter()
                .enumerate()
                .map(|(index, role)| ((*role).to_string(), index))
                .collect(),
            extras: Vec::new(),
        }
    }

    fn profile(module: &ModuleIdentity) -> QvmPrimaryCombatProfile {
        QvmPrimaryCombatProfile {
            damage_call: damage_call(),
            module: module.clone(),
            abi_profile: AbiProfile::Modern,
            entity_stride: 560,
            client_stride: 480,
            fields: QvmPrimaryCombatFields {
                inuse: 516,
                health: 520,
                takedamage: 524,
                parent: 528,
                client: 532,
            },
            callbacks: QvmCombatCallbacks {
                allocate: 0,
                free: 1,
                damage: 2,
            },
            armor: QvmGameArmorDefinition {
                check_armor: 3,
                call: armor_call(),
                points_stat: 4,
                protection: 0.5,
                tiers: None::<QvmArmorTiers>,
            },
            reactions: QvmPrimaryCombatReactions {
                flags: 540,
                pain: 544,
                die: 548,
                pain_call: QvmReactionCall {
                    arguments: 2,
                    target: 0,
                    amount: 1,
                },
                die_call: QvmReactionCall {
                    arguments: 2,
                    target: 0,
                    amount: 1,
                },
            },
            grapple_damage_method: 10,
            state: QvmPrimaryCombatState {
                health_stat: 0,
                team: QvmPrimaryCombatTeam {
                    persistent_stat: 0,
                    values: vec![QvmCombatTeam {
                        value: 1,
                        team: "red:one".to_string(),
                    }],
                },
                flags: QvmPrimaryCombatFlags {
                    notarget: 1,
                    invulnerable: 2,
                    no_knockback: 4,
                },
                mass: QvmCombatMass::Constant { value: 100.0 },
            },
            damage_flags: QvmDamageFlags {
                radius: 64,
                no_armor: 1,
                no_knockback: 4,
                no_protection: 8,
                no_team_protection: 16,
            },
        }
    }

    fn fixture(with_source: bool) -> Fixture {
        let image = QvmImage {
            instructions: (0..4)
                .map(|entry| QvmInstruction::word(QvmOpcode::OpEnter, 16, entry * 5))
                .collect(),
            allocated_data_length: 65536 + 4096,
            ..Default::default()
        };
        let module_id = ModuleIdentity {
            id: "q3:qagame".to_string(),
            artifact_path: "qagame.qvm".to_string(),
            digest: "d".to_string(),
            revision: "r".to_string(),
        };
        let artifact = QvmArtifact {
            module: module_id.clone(),
            role: QvmRole::Qagame,
            abi_profile: None,
            image,
        };
        let module = QvmModule::new(artifact.clone(), None, None).unwrap();
        let memory = module.memory();
        let data = QvmGameData::new(memory.clone(), AbiProfile::Modern);
        data.locate(64, 2, 560, 4096, 480).unwrap();
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(0, 1);
        let authority = Rc::new(FixtureAuthority {
            bound: RefCell::new(HashMap::new()),
        });
        let registry = Rc::new(FixtureRegistry {
            owned: RefCell::new(HashMap::new()),
            hooks: RefCell::new(Vec::new()),
            next: RefCell::new(0),
            released: RefCell::new(Vec::new()),
        });
        registry.owned.borrow_mut().insert(actor.clone(), actor.clone());
        let slot_actor = actor.clone();
        let source = with_source.then(|| {
            let registry_hook: Rc<dyn CombatActorRegistry> = registry.clone();
            let actor_hook = actor.clone();
            CombatSource {
                actors: registry_hook,
                actor: Rc::new(move |slot| (slot == 0).then(|| actor_hook.clone())),
                provenance: Rc::new(|_, _, _| AuthorityProvenance {
                    weapon: None,
                    movement_provider: "q3:pmove".to_string(),
                }),
                after_free: None,
            }
        });
        let bindings = QvmCombatBindings::bind(CombatBindingOptions {
            module,
            data,
            artifact,
            definition: profile(&module_id),
            bodies: Rc::new(FixtureBodies),
            combat: authority.clone(),
            source,
            slot: Rc::new(move |candidate| (*candidate == slot_actor).then_some(0)),
        })
        .unwrap();
        Fixture {
            bindings,
            memory,
            actor,
            authority,
            registry,
        }
    }

    fn request(target: &ActorId) -> AuthorityDamageRequest {
        AuthorityDamageRequest {
            target: target.clone(),
            amount: 25.0,
            knockback: 25.0,
            direction: vec3(0.0, 0.0, 1.0),
            point: vec3(1.0, 2.0, 3.0),
            normal: vec3(0.0, 0.0, 1.0),
            delivery: DamageDelivery::Direct,
            attack: AuthorityAttack {
                cause: AuthorityCause::Q3 {
                    means_of_death: 7,
                    damage_flags: 0,
                },
                attacker: None,
                inflictor: None,
                weapon: None,
                movement_provider: "q3:pmove".to_string(),
            },
        }
    }

    #[test]
    fn masks_require_single_bits() {
        assert!(is_single_mask(1));
        assert!(is_single_mask(i32::MIN));
        assert!(!is_single_mask(0));
        assert!(!is_single_mask(3));
        assert!(is_binary32_fraction(0.5));
        assert!(!is_binary32_fraction(0.66));
        assert!(!is_binary32_fraction(2.0));
        assert!(is_namespaced_team("red:one"));
        assert!(!is_namespaced_team("red"));
        assert!(!is_namespaced_team(":x"));
    }

    #[test]
    fn bind_rejects_overlapping_masks() {
        let image = QvmImage {
            instructions: (0..4)
                .map(|entry| QvmInstruction::word(QvmOpcode::OpEnter, 16, entry * 5))
                .collect(),
            allocated_data_length: 65536 + 4096,
            ..Default::default()
        };
        let module_id = ModuleIdentity {
            id: "q3:qagame".to_string(),
            artifact_path: "qagame.qvm".to_string(),
            digest: "d".to_string(),
            revision: "r".to_string(),
        };
        let artifact = QvmArtifact {
            module: module_id.clone(),
            role: QvmRole::Qagame,
            abi_profile: None,
            image,
        };
        let module = QvmModule::new(artifact.clone(), None, None).unwrap();
        let data = QvmGameData::new(module.memory(), AbiProfile::Modern);
        data.locate(64, 2, 560, 4096, 480).unwrap();
        let mut bad = profile(&module_id);
        bad.damage_flags.no_protection = 1;
        let authority = Rc::new(FixtureAuthority {
            bound: RefCell::new(HashMap::new()),
        });
        let outcome = QvmCombatBindings::bind(CombatBindingOptions {
            module,
            data,
            artifact,
            definition: bad,
            bodies: Rc::new(FixtureBodies),
            combat: authority,
            source: None,
            slot: Rc::new(|_| None),
        });
        assert!(outcome.is_err());
    }

    #[test]
    fn admit_binds_and_reads_shared_state() {
        let Fixture {
            bindings,
            memory,
            actor,
            authority,
            ..
        } = fixture(false);
        memory.write_i32(64 + 516, 1).unwrap();
        memory.write_i32(64 + 520, 80).unwrap();
        memory.write_i32(64 + 524, 1).unwrap();
        memory.write_i32(64 + 540, 2).unwrap();
        memory.write_i32(4096 + 184, 80).unwrap();
        memory.write_i32(4096 + 184 + 4 * 4, 25).unwrap();
        memory.write_i32(4096 + 248, 1).unwrap();
        bindings.admit(&actor).unwrap();
        let binding = authority.bound.borrow().get(&actor).cloned().unwrap();
        let read = (binding.read)().unwrap();
        assert_eq!((read.health, read.can_take_damage), (80, true));
        assert_eq!(read.mass, 100.0);
        assert_eq!(read.team.as_deref(), Some("red:one"));
        assert!(read.invulnerable);
        assert!(!read.no_knockback);
        (binding.write_health)(60);
        assert_eq!(memory.read_i32(64 + 520).unwrap(), 60);
        assert_eq!(memory.read_i32(4096 + 184).unwrap(), 60);
        assert_eq!(bindings.notarget(&actor).unwrap(), Some(false));
        memory.write_i32(64 + 540, 1).unwrap();
        assert_eq!(bindings.notarget(&actor).unwrap(), Some(true));
    }

    #[test]
    fn legacy_armor_projection_validates_saved_values() {
        let Fixture {
            bindings,
            memory,
            actor,
            ..
        } = fixture(false);
        bindings.admit(&actor).unwrap();
        memory.write_i32(4096 + 184 + 3 * 4, 30).unwrap();
        memory.write_i32(4096 + 184 + 4 * 4, 25).unwrap();
        let saved = ArmorState {
            regular: RegularArmor::Q3 {
                points: 30.0,
                protection: 0.66,
            },
            powered: PoweredProtection::None,
        };
        let projected = bindings.normalize_legacy_armor(&actor, &saved).unwrap();
        assert!(matches!(
            projected.regular,
            RegularArmor::Q3 { points, .. } if points == 25.0
        ));
        let bad = ArmorState {
            regular: RegularArmor::Q3 {
                points: 31.0,
                protection: 0.66,
            },
            powered: PoweredProtection::None,
        };
        assert!(bindings.normalize_legacy_armor(&actor, &bad).is_err());
    }

    #[test]
    fn incoming_damage_consumes_and_calls_source_entry() {
        let Fixture {
            bindings,
            memory,
            actor,
            ..
        } = fixture(false);
        memory.write_i32(64 + 516, 1).unwrap();
        memory.write_i32(64 + 520, 100).unwrap();
        memory.write_i32(64 + 524, 1).unwrap();
        memory.write_i32(0, 0x1234_5678).unwrap();
        bindings.admit(&actor).unwrap();
        let module = bindings.inner.borrow().options.module.clone();
        let before = module.calls().len();
        let outcome = bindings.damage(&request(&actor));
        assert_eq!(outcome.applied_damage, 0);
        assert_eq!(outcome.reaction, QvmDamageReaction::None);
        let calls = module.calls();
        assert_eq!(calls.len(), before + 1);
        assert_eq!(calls[before].entry, 2);
        assert_eq!(calls[before].words.len(), 8);
        assert_eq!(calls[before].words[0], 64);
        assert_eq!(calls[before].words[5], 25);
        assert!(bindings.inner.borrow().incoming.is_none());
        assert_eq!(memory.read_i32(0).unwrap(), 0x1234_5678);
    }

    #[test]
    fn armor_intercepts_split_savings_and_restore_amount() {
        let Fixture {
            bindings,
            memory,
            actor,
            authority,
            ..
        } = fixture(true);
        memory.write_i32(64 + 516, 1).unwrap();
        memory.write_i32(64 + 524, 1).unwrap();
        bindings.admit(&actor).unwrap();
        let binding = authority.bound.borrow().get(&actor).cloned().unwrap();
        let power_stage = binding.powered.stage.clone().unwrap();
        let regular_stage = binding.regular.stage.clone().unwrap();
        let power: ArmorIntercept = Rc::new(|hit, _| {
            assert_eq!(hit.amount, 40);
            assert_eq!(hit.flags.stage, ArmorStageKind::Power);
            10.0
        });
        let regular: ArmorIntercept = Rc::new(|hit, _| {
            assert_eq!(hit.amount, 30);
            assert_eq!(hit.flags.stage, ArmorStageKind::Regular);
            12.0
        });
        let _remove_power = (power_stage.bind)(power).unwrap();
        let _remove_regular = (regular_stage.bind)(regular).unwrap();
        assert!((power_stage.bind)(Rc::new(|_, _| 0.0)).is_err());

        let request = request(&actor);
        let observer: SharedDamageObserver = Rc::new(RefCell::new(FixtureObserver {
            stored: Vec::new(),
            reactions: Vec::new(),
        }));
        let mut call = QvmFunctionCall::entered(2, vec![64, 0, 0, 0, 0, 40, 0, 7], memory.clone());
        bindings.inner.borrow_mut().contexts.push(DamageContext {
            pointer: 64,
            request: request.clone(),
        });
        let saved = Rc::new(RefCell::new(0));
        let saved_hook = Rc::clone(&saved);
        let bindings_hook = bindings.clone();
        let memory_hook = memory.clone();
        bindings
            .scopes
            .run(
                &mut call,
                &actor,
                0,
                &request.to_scope_request(),
                &observer,
                Some(&|call| {
                    let mut armor_call = QvmFunctionCall::entered(3, vec![64, 40, 0], memory_hook.clone());
                    let total = bindings_hook.check_armor(&mut armor_call);
                    *saved_hook.borrow_mut() = total;
                    assert_eq!(armor_call.words[1], 40);
                    call.proceed()
                }),
            )
            .unwrap();
        bindings.inner.borrow_mut().contexts.pop();
        assert_eq!(*saved.borrow(), 22);
    }

    #[test]
    fn after_free_releases_freed_actors() {
        let Fixture {
            bindings,
            memory,
            actor,
            registry,
            ..
        } = fixture(true);
        memory.write_i32(64 + 516, 1).unwrap();
        bindings.admit(&actor).unwrap();
        let module = bindings.inner.borrow().options.module.clone();
        module.call(&[64], 1).unwrap();
        assert!(registry.released.borrow().is_empty());
        memory.write_i32(64 + 516, 0).unwrap();
        module.call(&[64], 1).unwrap();
        assert_eq!(registry.released.borrow().as_slice(), std::slice::from_ref(&actor));
        assert!(bindings.inner.borrow().admitted.is_empty());
        bindings.close();
        bindings.close();
    }
}
