//! QVM mod protection: original armor absorption over declared source storage.
//!
//! Provenance: `src/compat/qvm/mod-protection.ts`.
//!
//! Absorbs the protection types of `src/contracts/qvm-mod-callbacks.ts`
//! (`QvmModProtection*`). Source calls, actor records, and channels reuse
//! [`super::mod_provider`]. Local mirrors: [`RegularArmorState`] /
//! [`PoweredProtectionState`] (the source-owned variants of
//! `src/contracts/gameplay.ts`), [`ArmorStageInput`] / [`ArmorStageResult`],
//! [`ProtectionClaim`], and the [`ProtectionObserver`] report trait. Combat
//! authority, client identity, guest memory, and source invocation integrate
//! through [`ProtectionHost`]; reservation handles are host-side tokens whose
//! bound closures delegate back to this channel's read/validate/write/absorb
//! methods. [`QvmModProtection::observe`] diffs before/after snapshots instead
//! of tapping committed-memory writes (the memory owner integrates the live
//! listener); multi-channel combined reports arrive by passing sibling
//! channels explicitly.

use std::collections::{BTreeMap, HashMap, HashSet};

use qa_core::identity::{ActorId, ClientId, ProviderId};
use qa_core::math::Vec3;

use super::mod_provider::{
    mod_field_size, ModActorBinding, ModCallbackInput, ModCallbackValue, ModReturns, ModRuntimeValue, ModScalar,
    ProtectionChannel, QvmModCallbackDeclaration, QvmModSourceCall,
};
use crate::error::GuestError;

/// One source scalar word.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmModProtectionScalar {
    /// Record id.
    pub record: String,
    /// Field offset.
    pub offset: usize,
    /// Encoding.
    pub encoding: ModScalar,
}

/// Source selection of one stored value.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModProtectionSelection<V> {
    /// Selection field.
    pub field: QvmModProtectionScalar,
    /// Mask, if any.
    pub mask: Option<u32>,
    /// Declared values.
    pub values: Vec<SelectionEntry<V>>,
}

/// One declared selection value.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionEntry<V> {
    /// Source value.
    pub value: f64,
    /// Selected value.
    pub selected: V,
}

/// Protection admission.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProtectionAdmission {
    /// Claim the channel.
    Claim,
    /// Replace the current primary.
    ReplaceCurrentPrimary,
    /// Replace a primary owner.
    ReplacePrimary {
        /// Owner.
        owner: ProviderId,
    },
}

/// Lowered damage-flag masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProtectionFlags {
    /// No-armor mask.
    pub no_armor: u32,
    /// No-power-armor mask.
    pub no_power_armor: u32,
    /// No-regular-armor mask.
    pub no_regular_armor: u32,
    /// Energy mask.
    pub energy: u32,
    /// Radius mask.
    pub radius: u32,
}

/// Regular-armor source storage.
#[derive(Debug, Clone, PartialEq)]
pub struct RegularStorage {
    /// Armor points.
    pub points: QvmModProtectionScalar,
    /// Fixed item, if any.
    pub item: Option<String>,
    /// Item selection, if any.
    pub selection: Option<QvmModProtectionSelection<Option<String>>>,
}

/// Powered kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PoweredKind {
    /// None.
    None,
    /// Screen.
    Screen,
    /// Shield.
    Shield,
}

/// Powered-armor source storage.
#[derive(Debug, Clone, PartialEq)]
pub struct PoweredStorage {
    /// Cell count.
    pub cells: QvmModProtectionScalar,
    /// Kind selection.
    pub selection: QvmModProtectionSelection<PoweredKind>,
}

/// Declared protection channel (mirror of `QvmModProtection`).
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModProtection {
    /// Regular armor channel.
    Regular {
        /// Rule id.
        id: String,
        /// Admission.
        admission: ProtectionAdmission,
        /// Absorption call.
        absorb: QvmModSourceCall,
        /// Flag masks.
        flags: ProtectionFlags,
        /// Storage.
        storage: RegularStorage,
    },
    /// Powered armor channel.
    Powered {
        /// Rule id.
        id: String,
        /// Admission.
        admission: ProtectionAdmission,
        /// Absorption call.
        absorb: QvmModSourceCall,
        /// Flag masks.
        flags: ProtectionFlags,
        /// Storage.
        storage: PoweredStorage,
    },
}

impl QvmModProtection {
    /// Channel of this definition.
    #[must_use]
    pub const fn channel(&self) -> ProtectionChannel {
        match self {
            Self::Regular { .. } => ProtectionChannel::Regular,
            Self::Powered { .. } => ProtectionChannel::Powered,
        }
    }

    /// Rule id.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Regular { id, .. } | Self::Powered { id, .. } => id,
        }
    }

    /// Absorption call.
    #[must_use]
    pub fn absorb_call(&self) -> &QvmModSourceCall {
        match self {
            Self::Regular { absorb, .. } | Self::Powered { absorb, .. } => absorb,
        }
    }

    /// Flag masks.
    #[must_use]
    pub const fn flags(&self) -> &ProtectionFlags {
        match self {
            Self::Regular { flags, .. } | Self::Powered { flags, .. } => flags,
        }
    }

    /// Storage words read by this channel.
    #[must_use]
    pub fn storage_fields(&self) -> Vec<&QvmModProtectionScalar> {
        match self {
            Self::Regular { storage, .. } => {
                let mut fields = vec![&storage.points];
                if let Some(selection) = storage.selection.as_ref() {
                    fields.push(&selection.field);
                }
                fields
            }
            Self::Powered { storage, .. } => vec![&storage.cells, &storage.selection.field],
        }
    }
}

/// Validate protection declarations.
pub fn validate_qvm_mod_protection(declaration: &QvmModCallbackDeclaration) -> Result<(), GuestError> {
    let mut channels = HashSet::new();
    let mut ids = HashSet::new();
    let mut occupied = HashSet::new();
    for definition in &declaration.protection {
        if declaration.clients.is_none()
            || !channels.insert(definition.channel())
            || !ids.insert(definition.id().to_string())
            || definition.id().is_empty()
        {
            return Err(GuestError::invalid(
                "QVM protection requires unique channels and rules with source client admission",
            ));
        }
        if definition.absorb_call().returns == ModReturns::Void {
            return Err(GuestError::invalid("QVM protection requires original source savings"));
        }
        let flags = definition.flags();
        for mask in [
            flags.no_armor,
            flags.no_power_armor,
            flags.no_regular_armor,
            flags.energy,
            flags.radius,
        ] {
            if mask > 0x7fff_ffff {
                return Err(GuestError::invalid("Invalid QVM protection flag mask"));
            }
        }
        for field in definition.storage_fields() {
            let record = declaration
                .actor_records
                .iter()
                .find(|record| record.id == field.record);
            let key = (field.record.clone(), field.offset);
            if record.is_none_or(|record| field.offset % 4 != 0 || field.offset + 4 > record.stride)
                || !occupied.insert(key)
            {
                return Err(GuestError::invalid("Invalid QVM protection source storage"));
            }
            let record = record.expect("checked record");
            for value in &record.fields {
                let width = mod_field_size(value);
                if value.offset < field.offset + 4
                    && field.offset < value.offset + width
                    && !matches!(
                        value.binding,
                        ModActorBinding::Private { .. } | ModActorBinding::Constant { .. }
                    )
                {
                    return Err(GuestError::invalid(
                        "QVM protection storage overlaps a shared or linked source field",
                    ));
                }
            }
        }
        let selections: Vec<(&QvmModProtectionScalar, Option<u32>, Vec<f64>)> = match definition {
            QvmModProtection::Regular { storage, .. } => storage
                .selection
                .as_ref()
                .map(|selection| {
                    (
                        &selection.field,
                        selection.mask,
                        selection.values.iter().map(|entry| entry.value).collect(),
                    )
                })
                .into_iter()
                .collect(),
            QvmModProtection::Powered { storage, .. } => {
                vec![(
                    &storage.selection.field,
                    storage.selection.mask,
                    storage.selection.values.iter().map(|entry| entry.value).collect(),
                )]
            }
        };
        for (_, mask, values) in selections {
            if mask.is_some_and(|mask| mask > 0x7fff_ffff) {
                return Err(GuestError::invalid("Invalid QVM protection selection mask"));
            }
            let mut seen = HashSet::new();
            for value in &values {
                let masked = mask.is_some_and(|mask| {
                    value.fract() != 0.0
                        || *value < f64::from(i32::MIN)
                        || *value > f64::from(i32::MAX)
                        || ((*value as i32) & (mask as i32) != (*value as i32))
                });
                if !value.is_finite() || !seen.insert(value.to_bits()) || masked {
                    return Err(GuestError::invalid("Invalid QVM protection selection"));
                }
            }
            if values.is_empty() {
                return Err(GuestError::invalid("Empty QVM protection selection"));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Runtime mirrors.
// ---------------------------------------------------------------------------

/// Source-owned regular armor.
#[derive(Debug, Clone, PartialEq)]
pub struct RegularArmorState {
    /// Armor points.
    pub points: f64,
    /// Armor item, if any.
    pub item: Option<String>,
}

/// Source-owned powered protection.
#[derive(Debug, Clone, PartialEq)]
pub enum PoweredProtectionState {
    /// No protection.
    None,
    /// Active protection.
    Active {
        /// Kind.
        kind: PoweredKind,
        /// Cells.
        cells: f64,
    },
}

/// Regular armor change.
#[derive(Debug, Clone, PartialEq)]
pub struct RegularChange {
    /// Before.
    pub before: RegularArmorState,
    /// After.
    pub after: RegularArmorState,
}

/// Powered protection change.
#[derive(Debug, Clone, PartialEq)]
pub struct PoweredChange {
    /// Before.
    pub before: PoweredProtectionState,
    /// After.
    pub after: PoweredProtectionState,
}

/// Committed protection store.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProtectionStore {
    /// Regular change, if any.
    pub regular: Option<RegularChange>,
    /// Powered change, if any.
    pub powered: Option<PoweredChange>,
}

/// Observer of committed protection stores.
pub trait ProtectionObserver {
    /// Report one committed store.
    fn stored(&mut self, change: ProtectionStore);
}

/// Damage delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageDelivery {
    /// Direct damage.
    Direct,
    /// Radius damage.
    Radius,
}

/// Damage request (mirror of `DamageRequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    /// Target actor.
    pub target: ActorId,
    /// Attacking actor.
    pub attacker: ActorId,
    /// Inflicting actor.
    pub inflictor: ActorId,
    /// Knockback.
    pub knockback: f64,
    /// Delivery.
    pub delivery: DamageDelivery,
}

/// Armor damage flags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArmorDamageFlags {
    /// No armor.
    pub no_armor: bool,
    /// No power armor.
    pub no_power_armor: bool,
    /// No regular armor.
    pub no_regular_armor: bool,
    /// Energy damage.
    pub energy: bool,
    /// Regular protection scale, if any.
    pub regular_protection_scale: Option<f64>,
}

/// Armor stage input.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorStageInput {
    /// Damage request.
    pub request: DamageRequest,
    /// Impact point.
    pub point: Vec3,
    /// Impact direction.
    pub direction: Vec3,
    /// Impact normal.
    pub normal: Vec3,
    /// Damage amount.
    pub amount: f64,
    /// Flags.
    pub flags: ArmorDamageFlags,
}

/// Armor stage result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArmorStageResult {
    /// Saved damage.
    pub saved: f64,
}

/// Protection claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionClaim {
    /// Owner.
    pub owner: ProviderId,
    /// Rule id.
    pub rule: String,
    /// Admission.
    pub admission: ProtectionAdmission,
}

/// Host services of one protection channel.
pub trait ProtectionHost {
    /// Assert the caller runs on the owner thread.
    fn current(&self) -> Result<(), GuestError>;
    /// Canonical clients as `(actor, client)`.
    fn canonical_clients(&self) -> Vec<(ActorId, ClientId)>;
    /// Client bound to an actor, if any.
    fn client_for_actor(&self, actor: &ActorId) -> Option<ClientId>;
    /// Actor bound to a client, if any.
    fn actor_for_client(&self, client: &ClientId) -> Option<ActorId>;
    /// Whether an actor is live.
    fn is_live_actor(&self, actor: &ActorId) -> bool;
    /// Whether an actor resolves to an owned actor.
    fn resolve_owned(&self, actor: &ActorId) -> bool;
    /// Whether an actor has admitted source storage.
    fn is_eligible(&self, actor: &ActorId) -> bool;
    /// Resolve a record pointer for an actor.
    fn pointer(&self, actor: &ActorId, record: &str) -> Result<usize, GuestError>;
    /// Read one scalar word.
    fn read_scalar(&self, address: usize, encoding: ModScalar) -> Result<f64, GuestError>;
    /// Write one scalar word.
    fn write_scalar(&mut self, address: usize, encoding: ModScalar, value: f64) -> Result<(), GuestError>;
    /// Reserve a protection channel, returning a token.
    fn reserve_protection(
        &mut self,
        actor: &ActorId,
        channel: ProtectionChannel,
        claim: &ProtectionClaim,
    ) -> Result<u64, GuestError>;
    /// Bind a reserved channel.
    fn bind_protection(&mut self, actor: &ActorId, channel: ProtectionChannel) -> Result<(), GuestError>;
    /// Close a reservation token.
    fn close_reservation(&mut self, actor: &ActorId, channel: ProtectionChannel, token: u64) -> Result<(), GuestError>;
    /// Invoke the absorption call.
    fn invoke_protection(
        &mut self,
        call: &QvmModSourceCall,
        inputs: &BTreeMap<ModCallbackInput, ModRuntimeValue>,
    ) -> Result<f64, GuestError>;
    /// Caller time in seconds.
    fn time_seconds(&self) -> f64;
}

struct ProtectionEntry {
    client: ClientId,
    reservation: u64,
    bound: bool,
}

/// One source client and its original storage own each admitted channel.
pub struct QvmModProtectionRuntime<H: ProtectionHost> {
    definition: QvmModProtection,
    claim: ProtectionClaim,
    host: H,
    entries: HashMap<ActorId, ProtectionEntry>,
    stages: Vec<ActorId>,
    active: bool,
}

impl<H: ProtectionHost> QvmModProtectionRuntime<H> {
    /// Create a channel runtime.
    pub fn new(definition: QvmModProtection, owner: ProviderId, host: H) -> Self {
        let claim = ProtectionClaim {
            owner,
            rule: definition.id().to_string(),
            admission: match &definition {
                QvmModProtection::Regular { admission, .. } | QvmModProtection::Powered { admission, .. } => {
                    admission.clone()
                }
            },
        };
        Self {
            definition,
            claim,
            host,
            entries: HashMap::new(),
            stages: Vec::new(),
            active: false,
        }
    }

    /// Borrow the host.
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Mutably borrow the host.
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    /// Whether the channel is active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Channel of this runtime.
    #[must_use]
    pub fn channel(&self) -> ProtectionChannel {
        self.definition.channel()
    }

    /// Reserve every canonical client.
    pub fn reserve(&mut self) -> Result<(), GuestError> {
        let clients = self.host.canonical_clients();
        for (actor, _) in clients {
            self.reserve_actor(&actor)?;
        }
        Ok(())
    }

    /// Reserve one actor.
    pub fn reserve_actor(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        self.host.current()?;
        if self.entries.contains_key(actor) {
            self.require(actor)?;
            return Ok(());
        }
        let client = self.host.client_for_actor(actor);
        let owned = self.host.resolve_owned(actor);
        if client.is_none()
            || !owned
            || self
                .host
                .actor_for_client(client.as_ref().expect("checked client"))
                .as_ref()
                != Some(actor)
        {
            return Err(GuestError::invalid("QVM protection requires a live canonical client"));
        }
        let reservation = self
            .host
            .reserve_protection(actor, self.definition.channel(), &self.claim)?;
        self.entries.insert(
            actor.clone(),
            ProtectionEntry {
                client: client.expect("checked client"),
                reservation,
                bound: false,
            },
        );
        Ok(())
    }

    fn require(&self, actor: &ActorId) -> Result<&ProtectionEntry, GuestError> {
        let entry = self.entries.get(actor);
        if entry.is_none()
            || !self.host.is_live_actor(actor)
            || self.host.client_for_actor(actor).as_ref() != entry.map(|entry| &entry.client)
            || entry.is_some_and(|entry| self.host.actor_for_client(&entry.client).as_ref() != Some(actor))
        {
            return Err(GuestError::invalid("QVM protection client is no longer live"));
        }
        Ok(entry.expect("checked entry"))
    }

    fn address(&self, actor: &ActorId, field: &QvmModProtectionScalar) -> Result<usize, GuestError> {
        self.host.current()?;
        self.require(actor)?;
        if !self.host.is_eligible(actor) {
            return Err(GuestError::invalid(
                "QVM protection requires admitted source client storage",
            ));
        }
        Ok(self.host.pointer(actor, &field.record)? + field.offset)
    }

    fn scalar(&self, actor: &ActorId, field: &QvmModProtectionScalar) -> Result<f64, GuestError> {
        let address = self.address(actor, field)?;
        self.host.read_scalar(address, field.encoding)
    }

    fn count(&self, actor: &ActorId, field: &QvmModProtectionScalar) -> Result<f64, GuestError> {
        let value = self.scalar(actor, field)?;
        if !value.is_finite() || value < 0.0 {
            return Err(GuestError::invalid("Invalid QVM protection count"));
        }
        Ok(value)
    }

    fn selected<V: Clone + PartialEq>(
        &self,
        actor: &ActorId,
        selection: &QvmModProtectionSelection<V>,
    ) -> Result<V, GuestError> {
        let source = self.scalar(actor, &selection.field)?;
        let value = match selection.mask {
            None => source,
            Some(mask) => f64::from((source.trunc() as i32) & (mask as i32)),
        };
        selection
            .values
            .iter()
            .find(|entry| entry.value == value)
            .map(|entry| entry.selected.clone())
            .ok_or_else(|| GuestError::invalid("QVM protection source selection is undeclared"))
    }

    /// Read regular armor state.
    pub fn regular(&self, actor: &ActorId) -> Result<RegularArmorState, GuestError> {
        let QvmModProtection::Regular { storage, .. } = &self.definition else {
            return Err(GuestError::invalid("QVM protection does not own regular armor"));
        };
        Ok(RegularArmorState {
            points: self.count(actor, &storage.points)?,
            item: match storage.selection.as_ref() {
                None => storage.item.clone(),
                Some(selection) => self.selected(actor, selection)?,
            },
        })
    }

    /// Read powered protection state.
    pub fn powered(&self, actor: &ActorId) -> Result<PoweredProtectionState, GuestError> {
        let QvmModProtection::Powered { storage, .. } = &self.definition else {
            return Err(GuestError::invalid("QVM protection does not own powered armor"));
        };
        let kind = self.selected(actor, &storage.selection)?;
        if kind == PoweredKind::None {
            return Ok(PoweredProtectionState::None);
        }
        Ok(PoweredProtectionState::Active {
            kind,
            cells: self.count(actor, &storage.cells)?,
        })
    }

    fn validate_count(&self, field: &QvmModProtectionScalar, count: f64) -> Result<(), GuestError> {
        if !count.is_finite()
            || count < 0.0
            || field.encoding == ModScalar::Int32 && (count.fract() != 0.0 || count > f64::from(i32::MAX))
            || field.encoding == ModScalar::Float32 && !(count as f32).is_finite()
        {
            return Err(GuestError::invalid(
                "Protection count is not representable by its QVM source storage",
            ));
        }
        Ok(())
    }

    /// Validate a regular armor write.
    pub fn validate_regular(&self, actor: &ActorId, next: &RegularArmorState) -> Result<(), GuestError> {
        let current = self.regular(actor)?;
        let QvmModProtection::Regular { storage, .. } = &self.definition else {
            return Err(GuestError::invalid(
                "QVM regular armor selection requires its original source operation",
            ));
        };
        if current.item != next.item {
            return Err(GuestError::invalid(
                "QVM regular armor selection requires its original source operation",
            ));
        }
        self.validate_count(&storage.points, next.points)
    }

    /// Validate a powered protection write.
    pub fn validate_powered(&self, actor: &ActorId, next: &PoweredProtectionState) -> Result<(), GuestError> {
        let current = self.powered(actor)?;
        let current_kind = match &current {
            PoweredProtectionState::None => PoweredKind::None,
            PoweredProtectionState::Active { kind, .. } => *kind,
        };
        let next_kind = match next {
            PoweredProtectionState::None => PoweredKind::None,
            PoweredProtectionState::Active { kind, .. } => *kind,
        };
        if current_kind != next_kind {
            return Err(GuestError::invalid(
                "QVM powered armor selection requires its original source operation",
            ));
        }
        if let PoweredProtectionState::Active { cells, .. } = next {
            let QvmModProtection::Powered { storage, .. } = &self.definition else {
                return Err(GuestError::invalid(
                    "QVM powered armor selection requires its original source operation",
                ));
            };
            self.validate_count(&storage.cells, *cells)?;
        }
        Ok(())
    }

    fn write_word(&mut self, actor: &ActorId, field: &QvmModProtectionScalar, value: f64) -> Result<(), GuestError> {
        let address = self.address(actor, field)?;
        self.host.write_scalar(address, field.encoding, value)
    }

    /// Write regular armor points.
    pub fn write_regular(&mut self, actor: &ActorId, next: &RegularArmorState) -> Result<(), GuestError> {
        self.validate_regular(actor, next)?;
        let QvmModProtection::Regular { storage, .. } = self.definition.clone() else {
            return Err(GuestError::invalid("Missing QVM regular source storage"));
        };
        self.write_word(actor, &storage.points, next.points)
    }

    /// Write powered protection cells.
    pub fn write_powered(&mut self, actor: &ActorId, next: &PoweredProtectionState) -> Result<(), GuestError> {
        self.validate_powered(actor, next)?;
        let QvmModProtection::Powered { storage, .. } = self.definition.clone() else {
            return Err(GuestError::invalid("Missing QVM powered source storage"));
        };
        if let PoweredProtectionState::Active { cells, .. } = next {
            self.write_word(actor, &storage.cells, *cells)?;
        }
        Ok(())
    }

    /// Bind one actor's reservation.
    pub fn bind_actor(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        if !self.active || !self.host.is_eligible(actor) {
            return Ok(());
        }
        self.require(actor)?;
        if self.entries.get(actor).is_some_and(|entry| entry.bound) {
            return Ok(());
        }
        match self.definition.channel() {
            ProtectionChannel::Regular => {
                self.regular(actor)?;
            }
            ProtectionChannel::Powered => {
                self.powered(actor)?;
            }
        }
        self.host.bind_protection(actor, self.definition.channel())?;
        if let Some(entry) = self.entries.get_mut(actor) {
            entry.bound = true;
        }
        Ok(())
    }

    /// Activate the channel.
    pub fn activate(&mut self) -> Result<(), GuestError> {
        self.reserve()?;
        self.active = true;
        self.sync()
    }

    /// Bind every reserved actor.
    pub fn sync(&mut self) -> Result<(), GuestError> {
        if self.active {
            let actors: Vec<ActorId> = self.entries.keys().cloned().collect();
            for actor in actors {
                self.bind_actor(&actor)?;
            }
        }
        Ok(())
    }

    fn has_scale_lowering(&self) -> bool {
        let absorb = self.definition.absorb_call();
        absorb
            .arguments
            .iter()
            .chain(absorb.globals.iter().map(|global| &global.value))
            .any(|value| {
                matches!(
                    value,
                    super::mod_provider::QvmModValue::Int32(ModCallbackValue::Input(
                        ModCallbackInput::RegularProtectionScale
                    )) | super::mod_provider::QvmModValue::Float32(ModCallbackValue::Input(
                        ModCallbackInput::RegularProtectionScale
                    ))
                )
            })
    }

    /// Absorb damage through the original source function.
    pub fn absorb(
        &mut self,
        actor: &ActorId,
        siblings: &[Self],
        input: &ArmorStageInput,
        observer: &mut dyn ProtectionObserver,
    ) -> Result<ArmorStageResult, GuestError> {
        if input.request.target != *actor {
            return Err(GuestError::invalid(
                "QVM protection target differs from its source owner",
            ));
        }
        self.require(actor)?;
        let scale = input.flags.regular_protection_scale.unwrap_or(1.0);
        if self.definition.channel() == ProtectionChannel::Regular && scale != 1.0 && !self.has_scale_lowering() {
            return Err(GuestError::invalid(
                "QVM regular protection scale has no declared source lowering",
            ));
        }
        let masks = *self.definition.flags();
        let lowered = (if input.flags.no_armor { masks.no_armor } else { 0 })
            | (if input.flags.no_power_armor {
                masks.no_power_armor
            } else {
                0
            })
            | (if input.flags.no_regular_armor {
                masks.no_regular_armor
            } else {
                0
            })
            | (if input.flags.energy { masks.energy } else { 0 })
            | (if input.request.delivery == DamageDelivery::Radius {
                masks.radius
            } else {
                0
            });
        let mut inputs = BTreeMap::new();
        inputs.insert(ModCallbackInput::Own, ModRuntimeValue::Actor(Some(actor.clone())));
        inputs.insert(
            ModCallbackInput::Attacker,
            ModRuntimeValue::Actor(Some(input.request.attacker.clone())),
        );
        inputs.insert(
            ModCallbackInput::Inflictor,
            ModRuntimeValue::Actor(Some(input.request.inflictor.clone())),
        );
        inputs.insert(ModCallbackInput::Amount, ModRuntimeValue::Float(input.amount));
        inputs.insert(
            ModCallbackInput::Knockback,
            ModRuntimeValue::Float(input.request.knockback),
        );
        inputs.insert(
            ModCallbackInput::DamageFlags,
            ModRuntimeValue::Float(f64::from(lowered)),
        );
        inputs.insert(ModCallbackInput::RegularProtectionScale, ModRuntimeValue::Float(scale));
        inputs.insert(ModCallbackInput::Point, ModRuntimeValue::Vec(input.point));
        inputs.insert(ModCallbackInput::Direction, ModRuntimeValue::Vec(input.direction));
        inputs.insert(ModCallbackInput::Normal, ModRuntimeValue::Vec(input.normal));
        inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(self.host.time_seconds()));
        let call = self.definition.absorb_call().clone();
        let saved = self.observe(actor, siblings, observer, |runtime| {
            runtime.host.invoke_protection(&call, &inputs)
        })?;
        Ok(ArmorStageResult { saved })
    }

    fn channel_regular(&self, actor: &ActorId) -> Option<RegularArmorState> {
        if self.definition.channel() != ProtectionChannel::Regular || !self.entries.contains_key(actor) {
            return None;
        }
        self.regular(actor).ok()
    }

    fn channel_powered(&self, actor: &ActorId) -> Option<PoweredProtectionState> {
        if self.definition.channel() != ProtectionChannel::Powered || !self.entries.contains_key(actor) {
            return None;
        }
        self.powered(actor).ok()
    }

    /// Observe source execution, reporting committed store changes.
    pub fn observe<R>(
        &mut self,
        actor: &ActorId,
        siblings: &[Self],
        observer: &mut dyn ProtectionObserver,
        execute: impl FnOnce(&mut Self) -> Result<R, GuestError>,
    ) -> Result<R, GuestError> {
        let before_regular = std::iter::once(&*self)
            .chain(siblings.iter())
            .find_map(|channel| channel.channel_regular(actor));
        let before_powered = std::iter::once(&*self)
            .chain(siblings.iter())
            .find_map(|channel| channel.channel_powered(actor));
        self.stages.push(actor.clone());
        let outcome = execute(self);
        let retained = self.stages.iter().rposition(|stage| stage == actor);
        if let Some(index) = retained {
            self.stages.remove(index);
        }
        let result = outcome?;
        if self.entries.contains_key(actor) && self.host.is_live_actor(actor) {
            self.require(actor)?;
        }
        let after_regular = std::iter::once(&*self)
            .chain(siblings.iter())
            .find_map(|channel| channel.channel_regular(actor));
        let after_powered = std::iter::once(&*self)
            .chain(siblings.iter())
            .find_map(|channel| channel.channel_powered(actor));
        let regular_change = match (before_regular, after_regular) {
            (Some(before), Some(after)) if before != after => Some(RegularChange { before, after }),
            _ => None,
        };
        let powered_change = match (before_powered, after_powered) {
            (Some(before), Some(after)) if before != after => Some(PoweredChange { before, after }),
            _ => None,
        };
        if regular_change.is_some() {
            observer.stored(ProtectionStore {
                regular: regular_change,
                powered: powered_change,
            });
        } else if powered_change.is_some() {
            observer.stored(ProtectionStore {
                regular: None,
                powered: powered_change,
            });
        }
        Ok(result)
    }

    /// Assert no observed execution is running.
    pub fn assert_idle(&self, siblings: &[Self]) -> Result<(), GuestError> {
        if !self.stages.is_empty() || siblings.iter().any(|sibling| !sibling.stages.is_empty()) {
            return Err(GuestError::invalid(
                "Cannot save or restore during QVM protection execution",
            ));
        }
        Ok(())
    }

    /// Release one actor.
    pub fn release(&mut self, actor: &ActorId) {
        self.stages.retain(|stage| stage != actor);
        if let Some(entry) = self.entries.remove(actor) {
            let _ = self
                .host
                .close_reservation(actor, self.definition.channel(), entry.reservation);
        }
    }

    /// Close the channel, aggregating cleanup failures.
    pub fn close(&mut self) -> Result<(), GuestError> {
        self.active = false;
        let mut errors = Vec::new();
        for actor in self.entries.keys().cloned().collect::<Vec<_>>() {
            let entry = self.entries.remove(&actor).expect("tracked actor");
            if let Err(error) = self
                .host
                .close_reservation(&actor, self.definition.channel(), entry.reservation)
            {
                errors.push(error);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(GuestError::callback(format!(
                "QVM protection cleanup failed: {}",
                errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::mod_provider::{
        ModActorBinding, QvmModActorField, QvmModActorRecord, QvmModClients, QvmModSourceCall,
    };
    use super::*;

    struct FakeHost {
        live: HashSet<ActorId>,
        clients: HashMap<ActorId, ClientId>,
        words: HashMap<usize, f64>,
        tokens: u64,
        bound: Vec<(ActorId, ProtectionChannel)>,
        saved: f64,
        seen_inputs: Vec<ModCallbackInput>,
    }

    impl FakeHost {
        fn new() -> Self {
            Self {
                live: HashSet::new(),
                clients: HashMap::new(),
                words: HashMap::new(),
                tokens: 0,
                bound: Vec::new(),
                saved: 0.0,
                seen_inputs: Vec::new(),
            }
        }
    }

    impl ProtectionHost for FakeHost {
        fn current(&self) -> Result<(), GuestError> {
            Ok(())
        }
        fn canonical_clients(&self) -> Vec<(ActorId, ClientId)> {
            self.clients
                .iter()
                .map(|(actor, client)| (actor.clone(), client.clone()))
                .collect()
        }
        fn client_for_actor(&self, actor: &ActorId) -> Option<ClientId> {
            self.clients.get(actor).cloned()
        }
        fn actor_for_client(&self, client: &ClientId) -> Option<ActorId> {
            self.clients
                .iter()
                .find(|(_, bound)| *bound == client)
                .map(|(actor, _)| actor.clone())
        }
        fn is_live_actor(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }
        fn resolve_owned(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }
        fn is_eligible(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }
        fn pointer(&self, _actor: &ActorId, _record: &str) -> Result<usize, GuestError> {
            Ok(1000)
        }
        fn read_scalar(&self, address: usize, _encoding: ModScalar) -> Result<f64, GuestError> {
            Ok(self.words.get(&address).copied().unwrap_or(0.0))
        }
        fn write_scalar(&mut self, address: usize, _encoding: ModScalar, value: f64) -> Result<(), GuestError> {
            self.words.insert(address, value);
            Ok(())
        }
        fn reserve_protection(
            &mut self,
            _actor: &ActorId,
            _channel: ProtectionChannel,
            _claim: &ProtectionClaim,
        ) -> Result<u64, GuestError> {
            self.tokens += 1;
            Ok(self.tokens)
        }
        fn bind_protection(&mut self, actor: &ActorId, channel: ProtectionChannel) -> Result<(), GuestError> {
            self.bound.push((actor.clone(), channel));
            Ok(())
        }
        fn close_reservation(
            &mut self,
            _actor: &ActorId,
            _channel: ProtectionChannel,
            _token: u64,
        ) -> Result<(), GuestError> {
            Ok(())
        }
        fn invoke_protection(
            &mut self,
            _call: &QvmModSourceCall,
            inputs: &BTreeMap<ModCallbackInput, ModRuntimeValue>,
        ) -> Result<f64, GuestError> {
            self.seen_inputs = inputs.keys().copied().collect();
            Ok(self.saved)
        }
        fn time_seconds(&self) -> f64 {
            2.0
        }
    }

    struct FakeObserver {
        stores: Vec<ProtectionStore>,
    }

    impl ProtectionObserver for FakeObserver {
        fn stored(&mut self, change: ProtectionStore) {
            self.stores.push(change);
        }
    }

    fn absorb_call() -> QvmModSourceCall {
        QvmModSourceCall {
            entry: 3,
            arguments: Vec::new(),
            globals: Vec::new(),
            returns: ModReturns::Int32,
        }
    }

    fn regular_definition() -> QvmModProtection {
        QvmModProtection::Regular {
            id: "test:armor".to_string(),
            admission: ProtectionAdmission::Claim,
            absorb: absorb_call(),
            flags: ProtectionFlags {
                no_armor: 1,
                no_power_armor: 2,
                no_regular_armor: 4,
                energy: 8,
                radius: 16,
            },
            storage: RegularStorage {
                points: QvmModProtectionScalar {
                    record: "client".to_string(),
                    offset: 8,
                    encoding: ModScalar::Int32,
                },
                item: Some("test:armor".to_string()),
                selection: None,
            },
        }
    }

    fn powered_definition() -> QvmModProtection {
        QvmModProtection::Powered {
            id: "test:cells".to_string(),
            admission: ProtectionAdmission::Claim,
            absorb: absorb_call(),
            flags: ProtectionFlags {
                no_armor: 1,
                no_power_armor: 2,
                no_regular_armor: 4,
                energy: 8,
                radius: 16,
            },
            storage: PoweredStorage {
                cells: QvmModProtectionScalar {
                    record: "client".to_string(),
                    offset: 12,
                    encoding: ModScalar::Int32,
                },
                selection: QvmModProtectionSelection {
                    field: QvmModProtectionScalar {
                        record: "client".to_string(),
                        offset: 16,
                        encoding: ModScalar::Int32,
                    },
                    mask: None,
                    values: vec![
                        SelectionEntry {
                            value: 0.0,
                            selected: PoweredKind::None,
                        },
                        SelectionEntry {
                            value: 1.0,
                            selected: PoweredKind::Shield,
                        },
                    ],
                },
            },
        }
    }

    fn fixture_declaration(definitions: Vec<QvmModProtection>) -> QvmModCallbackDeclaration {
        QvmModCallbackDeclaration {
            version: 1,
            program_path: "vm/qagame.qvm".to_string(),
            program_digest: "sha256:abc".to_string(),
            abi_profile: super::super::mod_provider::QvmAbi::Modern,
            presentation: None,
            spawn_entities: None,
            clients: Some(QvmModClients {
                outputs: Vec::new(),
                maximum: 2,
                records: vec!["client".to_string()],
                player_state_record: "client".to_string(),
                admit: Vec::new(),
                userinfo: Vec::new(),
                disconnect: Vec::new(),
                frame: Vec::new(),
                input: Vec::new(),
            }),
            actor_records: vec![QvmModActorRecord {
                id: "client".to_string(),
                address: 64,
                stride: 512,
                capacity: 4,
                fields: vec![QvmModActorField {
                    offset: 0,
                    access: None,
                    binding: ModActorBinding::Private { byte_length: 512 },
                }],
            }],
            entity_record: None,
            source_actors: None,
            combat: None,
            protection: definitions,
            pickups: Vec::new(),
            items: None,
            initialize: Vec::new(),
            callbacks: Vec::new(),
            objectives: Vec::new(),
        }
    }

    #[test]
    fn validation_accepts_distinct_channels() {
        validate_qvm_mod_protection(&fixture_declaration(vec![regular_definition(), powered_definition()])).unwrap();
    }

    #[test]
    fn validation_rejects_duplicate_channels() {
        assert!(
            validate_qvm_mod_protection(&fixture_declaration(vec![regular_definition(), regular_definition()]))
                .is_err()
        );
        let mut void = regular_definition();
        if let QvmModProtection::Regular { absorb, .. } = &mut void {
            absorb.returns = ModReturns::Void;
        }
        assert!(validate_qvm_mod_protection(&fixture_declaration(vec![void])).is_err());
    }

    #[test]
    fn regular_round_trip_binds_and_absorbs() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let client = owner.client(0, 1);
        let mut host = FakeHost::new();
        host.live.insert(actor.clone());
        host.clients.insert(actor.clone(), client);
        host.words.insert(1008, 50.0);
        host.saved = 12.0;
        let mut runtime = QvmModProtectionRuntime::new(regular_definition(), ProviderId::new("test", "mod"), host);
        runtime.activate().unwrap();
        assert!(runtime.is_active());
        assert_eq!(runtime.host().bound, vec![(actor.clone(), ProtectionChannel::Regular)]);
        assert_eq!(runtime.regular(&actor).unwrap().points, 50.0);
        runtime
            .write_regular(
                &actor,
                &RegularArmorState {
                    points: 40.0,
                    item: Some("test:armor".to_string()),
                },
            )
            .unwrap();
        assert_eq!(runtime.regular(&actor).unwrap().points, 40.0);
        assert!(runtime
            .write_regular(
                &actor,
                &RegularArmorState {
                    points: 5.0,
                    item: Some("test:other".to_string())
                }
            )
            .is_err());
        let input = ArmorStageInput {
            request: DamageRequest {
                target: actor.clone(),
                attacker: actor.clone(),
                inflictor: actor.clone(),
                knockback: 0.0,
                delivery: DamageDelivery::Direct,
            },
            point: vec3(0.0, 0.0, 0.0),
            direction: vec3(0.0, 0.0, 1.0),
            normal: vec3(0.0, 0.0, 1.0),
            amount: 30.0,
            flags: ArmorDamageFlags {
                no_armor: false,
                no_power_armor: false,
                no_regular_armor: false,
                energy: true,
                regular_protection_scale: None,
            },
        };
        let mut observer = FakeObserver { stores: Vec::new() };
        let result = runtime.absorb(&actor, &[], &input, &mut observer).unwrap();
        assert_eq!(result.saved, 12.0);
        assert!(runtime.host().seen_inputs.contains(&ModCallbackInput::DamageFlags));
        runtime.assert_idle(&[]).unwrap();
        runtime.close().unwrap();
    }

    #[test]
    fn absorb_rejects_undeclared_scale_and_foreign_targets() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let other = owner.actor(2, 1);
        let client = owner.client(0, 1);
        let mut host = FakeHost::new();
        host.live.insert(actor.clone());
        host.clients.insert(actor.clone(), client);
        host.words.insert(1008, 50.0);
        let mut runtime = QvmModProtectionRuntime::new(regular_definition(), ProviderId::new("test", "mod"), host);
        runtime.activate().unwrap();
        let scaled = ArmorStageInput {
            request: DamageRequest {
                target: actor.clone(),
                attacker: actor.clone(),
                inflictor: actor.clone(),
                knockback: 0.0,
                delivery: DamageDelivery::Direct,
            },
            point: vec3(0.0, 0.0, 0.0),
            direction: vec3(0.0, 0.0, 1.0),
            normal: vec3(0.0, 0.0, 1.0),
            amount: 30.0,
            flags: ArmorDamageFlags {
                no_armor: false,
                no_power_armor: false,
                no_regular_armor: false,
                energy: false,
                regular_protection_scale: Some(0.5),
            },
        };
        let mut observer = FakeObserver { stores: Vec::new() };
        assert!(runtime.absorb(&actor, &[], &scaled, &mut observer).is_err());
        let foreign = ArmorStageInput {
            request: DamageRequest {
                target: other.clone(),
                attacker: actor.clone(),
                inflictor: actor.clone(),
                knockback: 0.0,
                delivery: DamageDelivery::Direct,
            },
            ..scaled
        };
        assert!(runtime.absorb(&actor, &[], &foreign, &mut observer).is_err());
    }

    #[test]
    fn observe_reports_committed_stores() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let client = owner.client(0, 1);
        let mut host = FakeHost::new();
        host.live.insert(actor.clone());
        host.clients.insert(actor.clone(), client);
        host.words.insert(1008, 50.0);
        let mut runtime = QvmModProtectionRuntime::new(regular_definition(), ProviderId::new("test", "mod"), host);
        runtime.activate().unwrap();
        let mut observer = FakeObserver { stores: Vec::new() };
        runtime
            .observe(&actor, &[], &mut observer, |runtime| {
                runtime.host_mut().words.insert(1008, 25.0);
                Ok(())
            })
            .unwrap();
        assert_eq!(observer.stores.len(), 1);
        assert_eq!(observer.stores[0].regular.as_ref().unwrap().after.points, 25.0);
    }

    #[test]
    fn powered_states_follow_selection() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let client = owner.client(0, 1);
        let mut host = FakeHost::new();
        host.live.insert(actor.clone());
        host.clients.insert(actor.clone(), client);
        host.words.insert(1012, 9.0);
        host.words.insert(1016, 1.0);
        let mut runtime = QvmModProtectionRuntime::new(powered_definition(), ProviderId::new("test", "mod"), host);
        runtime.activate().unwrap();
        assert_eq!(
            runtime.powered(&actor).unwrap(),
            PoweredProtectionState::Active {
                kind: PoweredKind::Shield,
                cells: 9.0
            }
        );
        runtime.host_mut().words.insert(1016, 0.0);
        assert_eq!(runtime.powered(&actor).unwrap(), PoweredProtectionState::None);
        runtime.host_mut().words.insert(1016, 7.0);
        assert!(runtime.powered(&actor).is_err());
    }
}
