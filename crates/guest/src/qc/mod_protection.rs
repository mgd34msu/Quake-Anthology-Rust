//! QuakeC protection channels over private client words.
//!
//! Ported from `src/compat/qc/mod-protection.ts`.
//!
//! Local mirrors (owned by other workers' modules, noted here):
//! `QcProtectionMachine` mirrors the entity-word surface of `QcMachine` from
//! `src/compat/qc/machine.ts`; `QcProtectionServices` mirrors the
//! actor/client/combat surface of `ModHostServices` from
//! `src/world/session/mods.ts`; `QcArmorStageView` mirrors `QcArmorStage`
//! from `src/content/q1/quakec/armor-stage.ts`; `ArmorStageInput`,
//! `QcRegularArmor`, `QcPoweredArmor`, and `ProtectionObserver` mirror
//! `src/contracts/gameplay.ts`; `QcProtectionDispatch` mirrors the
//! invoke/pickup operations.
//!
//! Adaptation: the donor binds read/write/absorb closures into combat
//! reservations; this port exposes `read_regular`/`read_powered`,
//! `write_regular`/`write_powered`, and `absorb` methods the combat
//! authority calls with the lane's reservation context.

use std::collections::HashMap;

use qa_core::identity::{ActorId, ClientId, OwnedActor, ProviderId};
use qa_core::math::Vec3;
use qa_core::time::SourceTime;

use super::mod_provider::{
    ItemId, ModCallbackDeclaration, ModCallbackInput, ModProtectionAbsorb, ModQcArmorStage, ModQcArmorStageFlags,
    ModQcProtection, ModRuntimeValue, ModSourceCall, PoweredKind, ProtectionChannel, QcModInputs, QcProgramView,
    QcValueType,
};
use crate::error::GuestError;

/// Resolved armor-stage region.
#[derive(Debug, Clone, PartialEq)]
pub struct QcArmorStageView {
    /// Function name.
    pub function: String,
    /// Entry statement.
    pub entry: i32,
    /// Target word.
    pub target: i32,
    /// Damage word.
    pub damage: i32,
    /// Flag word, when packed.
    pub flags_word: Option<i32>,
}

/// Validate protection declarations and resolve region absorbs.
pub fn qc_protection_regions(
    program: &dyn QcProgramView,
    declaration: &ModCallbackDeclaration,
) -> Result<Vec<QcArmorStageView>, GuestError> {
    use std::collections::HashSet;
    let mut channels = HashSet::new();
    let mut regions = Vec::new();
    for definition in &declaration.protection {
        if declaration.clients.is_none() || !channels.insert(definition.channel) {
            return Err(GuestError::invalid(
                "QC protection requires clients and one declaration per channel",
            ));
        }
        let (count, selection) = match (&definition.channel, &definition.regular, &definition.powered) {
            (ProtectionChannel::Regular, Some(storage), _) => (
                storage.points.clone(),
                storage.selection.as_ref().map(|selection| {
                    (
                        selection.field.clone(),
                        selection.mask,
                        selection.values.iter().map(|value| value.value).collect::<Vec<_>>(),
                    )
                }),
            ),
            (ProtectionChannel::Powered, _, Some(storage)) => (
                storage.cells.clone(),
                storage.selection.as_ref().map(|selection| {
                    (
                        selection.field.clone(),
                        selection.mask,
                        selection.values.iter().map(|value| value.value).collect::<Vec<_>>(),
                    )
                }),
            ),
            _ => return Err(GuestError::invalid("QC protection requires storage for its channel")),
        };
        let mut names = vec![count];
        if let Some((field, _, _)) = &selection {
            names.push(field.clone());
        }
        for name in &names {
            let bound = declaration.actor_fields.iter().any(|field| {
                field.field == *name && matches!(field.binding, super::mod_provider::ModActorBinding::Private)
            });
            if program.field_type(name) != Some(QcValueType::Float) || !bound {
                return Err(GuestError::invalid(format!(
                    "QC protection requires private float storage {name}"
                )));
            }
        }
        if let Some((field, mask, values)) = &selection {
            if field == &names[0]
                || values.is_empty()
                || values.iter().collect::<HashSet<_>>().len() != values.len()
                || values
                    .iter()
                    .any(|value| !value.is_finite() || f64::from(*value as f32) != *value)
            {
                return Err(GuestError::invalid("QC protection selection is not representable"));
            }
            if let Some(mask) = mask {
                if *mask <= 0
                    || *mask > 0x7f_ffff
                    || values
                        .iter()
                        .any(|value| value.fract() != 0.0 || (*value as i32 & *mask) != *value as i32)
                {
                    return Err(GuestError::invalid("QC protection selection mask is invalid"));
                }
            }
        }
        let flags = [
            definition.flags.no_armor,
            definition.flags.no_power_armor,
            definition.flags.no_regular_armor,
            definition.flags.energy,
            definition.flags.radius,
        ];
        if flags.iter().any(|mask| *mask < 0 || *mask > 0x7f_ffff) {
            return Err(GuestError::invalid(
                "QC protection flags exceed source integer precision",
            ));
        }
        if let ModProtectionAbsorb::Region { call, stage } = &definition.absorb {
            let region = resolve_armor_stage(program, stage)?;
            if call.function != region.function {
                return Err(GuestError::invalid(
                    "QC donor region lacks a qualified standalone frame",
                ));
            }
            let target = program
                .function_named(&region.function)
                .ok_or_else(|| GuestError::invalid("QC donor region lacks a qualified standalone frame"))?;
            let mut parameters: HashMap<i32, usize> = HashMap::new();
            let mut word = target.parameter_start;
            for (index, size) in target.parameter_sizes.iter().enumerate() {
                if call.arguments.get(index).is_some() {
                    parameters.insert(word, index);
                }
                word += *size;
            }
            let mut required = vec![
                (region.target, ModCallbackInput::Self_),
                (region.damage, ModCallbackInput::Amount),
            ];
            if let Some(flags_word) = region.flags_word {
                required.push((flags_word, ModCallbackInput::DamageFlags));
            }
            for (word, name) in required {
                let argument = parameters.get(&word).and_then(|index| call.arguments.get(*index));
                if !matches!(argument, Some(super::mod_provider::ModCallbackValue::Input(input)) if *input == name) {
                    return Err(GuestError::invalid(format!(
                        "QC donor region must initialize {} from its current stage",
                        name.name()
                    )));
                }
            }
            regions.push(region);
        }
    }
    Ok(regions)
}

/// Resolve a declared armor stage structurally.
fn resolve_armor_stage(program: &dyn QcProgramView, stage: &ModQcArmorStage) -> Result<QcArmorStageView, GuestError> {
    if program.function_named(&stage.function).is_none()
        || stage.entry >= stage.exit
        || stage.target < 0
        || stage.damage < 0
        || stage.saved < 0
        || stage.statements.is_empty()
    {
        return Err(GuestError::invalid(
            "QC donor region lacks a qualified standalone frame",
        ));
    }
    let flags_word = match stage.flags {
        ModQcArmorStageFlags::None => None,
        ModQcArmorStageFlags::Bits { word, .. } => {
            if word < 0 {
                return Err(GuestError::invalid(
                    "QC donor region lacks a qualified standalone frame",
                ));
            }
            Some(word)
        }
    };
    Ok(QcArmorStageView {
        function: stage.function.clone(),
        entry: stage.entry,
        target: stage.target,
        damage: stage.damage,
        flags_word,
    })
}

/// Entity-word surface for protection storage.
pub trait QcProtectionMachine {
    /// Read a float by reference and field.
    fn float_for(&self, reference: i32, field: &str) -> Result<f32, GuestError>;
    /// Write a float by reference and field.
    fn set_float_for(&mut self, reference: i32, field: &str, value: f32) -> Result<(), GuestError>;
}

/// Host services for protection.
pub trait QcProtectionServices {
    /// Current source time.
    fn now(&self) -> SourceTime;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Resolve an owned actor.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Client handle for an actor.
    fn client_for_actor(&self, actor: &ActorId) -> Option<ClientId>;
    /// Actor for a client handle.
    fn actor_for_client(&self, client: &ClientId) -> Option<ActorId>;
    /// Entity reference for an actor.
    fn reference_for_actor(&self, actor: &ActorId) -> Option<i32>;
    /// Actor for an entity reference.
    fn actor_for_reference(&self, reference: i32) -> Option<ActorId>;
    /// Reserve a protection channel.
    fn reserve(&mut self, owner: &OwnedActor, channel: ProtectionChannel, rule: &str) -> Result<u64, GuestError>;
    /// Release a protection reservation.
    fn release_reservation(&mut self, reservation: u64) -> Result<(), GuestError>;
}

/// Source dispatch for protection.
pub trait QcProtectionDispatch {
    /// Invoke an absorb call.
    fn invoke(
        &mut self,
        call: &ModSourceCall,
        inputs: &QcModInputs,
        region: Option<&QcArmorStageView>,
    ) -> Result<f64, GuestError>;
    /// Pickup rule ids that may write a channel.
    fn pickup_rule_ids(&self, _actor: &ActorId, _channel: ProtectionChannel) -> Vec<String> {
        Vec::new()
    }
}

/// Regular armor projection.
#[derive(Debug, Clone, PartialEq)]
pub struct QcRegularArmor {
    /// Armor points.
    pub points: f64,
    /// Armor item.
    pub item: Option<ItemId>,
}

/// Powered protection projection.
#[derive(Debug, Clone, PartialEq)]
pub struct QcPoweredArmor {
    /// Protection kind.
    pub kind: PoweredKind,
    /// Cell count.
    pub cells: f64,
}

/// Protection projection across bound channels.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProtectionProjection {
    /// Regular armor.
    pub regular: Option<QcRegularArmor>,
    /// Powered protection.
    pub powered: Option<QcPoweredArmor>,
}

/// Committed protection change.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtectionChange {
    /// Regular change.
    pub regular: Option<(QcRegularArmor, QcRegularArmor)>,
    /// Powered change.
    pub powered: Option<(QcPoweredArmor, QcPoweredArmor)>,
}

/// Observer for committed protection stores.
pub trait ProtectionObserver {
    /// Report one committed store.
    fn stored(&mut self, change: &ProtectionChange);
}

/// Damage delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageDelivery {
    /// Direct damage.
    Direct,
    /// Radius damage.
    Radius,
}

/// Armor-stage damage flags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArmorDamageFlags {
    /// Bypass all armor.
    pub no_armor: bool,
    /// Bypass powered armor.
    pub no_power_armor: bool,
    /// Bypass regular armor.
    pub no_regular_armor: bool,
    /// Energy damage.
    pub energy: bool,
    /// Regular protection scale.
    pub regular_protection_scale: Option<f64>,
}

/// Armor-stage absorb input.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorStageInput {
    /// Attacker, if any.
    pub attacker: Option<ActorId>,
    /// Inflictor, if any.
    pub inflictor: Option<ActorId>,
    /// Incoming amount.
    pub amount: f64,
    /// Knockback scalar.
    pub knockback: f64,
    /// Delivery mode.
    pub delivery: DamageDelivery,
    /// Damage flags.
    pub flags: ArmorDamageFlags,
    /// Damage direction.
    pub direction: Vec3,
    /// Damage point.
    pub point: Vec3,
    /// Damage normal.
    pub normal: Vec3,
}

struct ProtectionLane {
    channel: ProtectionChannel,
    definition: ModQcProtection,
    reservation: u64,
}

struct Entry {
    client: ClientId,
    lanes: Vec<ProtectionLane>,
    bound: bool,
}

struct Stage {
    actor: ActorId,
    observer: Box<dyn ProtectionObserver>,
    before: ProtectionProjection,
}

/// Protection channels over private client words.
pub struct QcModProtection<S, M, D> {
    declaration: ModCallbackDeclaration,
    provider: ProviderId,
    services: S,
    machine: M,
    dispatch: D,
    regions: Vec<QcArmorStageView>,
    entries: HashMap<ActorId, Entry>,
    stages: Vec<Stage>,
}

impl<S: QcProtectionServices, M: QcProtectionMachine, D: QcProtectionDispatch> QcModProtection<S, M, D> {
    /// Build and validate region absorbs.
    pub fn new(
        declaration: ModCallbackDeclaration,
        provider: ProviderId,
        program: &dyn QcProgramView,
        services: S,
        machine: M,
        dispatch: D,
    ) -> Result<Self, GuestError> {
        let regions = qc_protection_regions(program, &declaration)?;
        Ok(Self {
            declaration,
            provider,
            services,
            machine,
            dispatch,
            regions,
            entries: HashMap::new(),
            stages: Vec::new(),
        })
    }

    /// Resolved region absorbs.
    #[must_use]
    pub fn regions(&self) -> &[QcArmorStageView] {
        &self.regions
    }

    /// Reserve a client actor's channels.
    pub fn reserve(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        if self.entries.contains_key(actor) {
            self.require(actor)?;
            return Ok(());
        }
        let owned = self.services.resolve_owned(actor);
        let client = self.services.client_for_actor(actor);
        let (Some(owner), Some(client)) = (owned, client) else {
            return Err(GuestError::invalid("QC protection requires a live canonical client"));
        };
        if self.services.actor_for_client(&client).as_ref() != Some(actor) {
            return Err(GuestError::invalid("QC protection requires a live canonical client"));
        }
        let mut lanes = Vec::new();
        for definition in &self.declaration.protection {
            match self.services.reserve(&owner, definition.channel, &definition.id) {
                Ok(reservation) => lanes.push(ProtectionLane {
                    channel: definition.channel,
                    definition: definition.clone(),
                    reservation,
                }),
                Err(error) => {
                    for lane in &lanes {
                        let _ = self.services.release_reservation(lane.reservation);
                    }
                    return Err(error);
                }
            }
        }
        self.entries.insert(
            actor.clone(),
            Entry {
                client,
                lanes,
                bound: false,
            },
        );
        Ok(())
    }

    /// Require a live entry.
    fn require(&self, actor: &ActorId) -> Result<&Entry, GuestError> {
        let entry = self
            .entries
            .get(actor)
            .ok_or_else(|| GuestError::invalid("QC protection client is retired"))?;
        if !self.services.is_live(actor)
            || self.services.client_for_actor(actor).as_ref() != Some(&entry.client)
            || self.services.actor_for_client(&entry.client).as_ref() != Some(actor)
        {
            return Err(GuestError::invalid("QC protection client is retired"));
        }
        Ok(entry)
    }

    /// Entity reference for a protected actor.
    fn reference(&self, actor: &ActorId) -> Result<i32, GuestError> {
        self.require(actor)?;
        self.services
            .reference_for_actor(actor)
            .ok_or_else(|| GuestError::invalid("QC protection client is retired"))
    }

    /// Read or write a count word.
    fn count(&mut self, actor: &ActorId, name: &str, next: Option<f64>) -> Result<f64, GuestError> {
        let reference = self.reference(actor)?;
        if let Some(next) = next {
            self.machine.set_float_for(reference, name, next as f32)?;
        }
        Ok(f64::from(self.machine.float_for(reference, name)?))
    }

    /// Read a selection word with its mask.
    fn selected(&mut self, actor: &ActorId, field: &str, mask: Option<i32>) -> Result<f64, GuestError> {
        let value = self.count(actor, field, None)?;
        if let Some(mask) = mask {
            Ok(f64::from((value.trunc() as i32) & mask))
        } else {
            Ok(value)
        }
    }

    /// Write a selection word with its mask.
    fn select(&mut self, actor: &ActorId, field: &str, mask: Option<i32>, next: f64) -> Result<(), GuestError> {
        let value = match mask {
            Some(mask) => {
                let current = self.count(actor, field, None)?.trunc() as i32;
                f64::from((current & !mask) | (next.trunc() as i32))
            }
            None => next,
        };
        self.count(actor, field, Some(value)).map(|_| ())
    }

    /// Lane definition for a channel.
    fn lane(&self, actor: &ActorId, channel: ProtectionChannel) -> Result<ModQcProtection, GuestError> {
        let entry = self.require(actor)?;
        entry
            .lanes
            .iter()
            .find(|lane| lane.channel == channel)
            .map(|lane| lane.definition.clone())
            .ok_or_else(|| GuestError::invalid("QC protection requires its declared channel"))
    }

    /// Read regular armor.
    pub fn read_regular(&mut self, actor: &ActorId) -> Result<QcRegularArmor, GuestError> {
        let definition = self.lane(actor, ProtectionChannel::Regular)?;
        let storage = definition
            .regular
            .ok_or_else(|| GuestError::invalid("QC protection requires storage for its channel"))?;
        let points = self.count(actor, &storage.points, None)?;
        let item = match storage.selection.as_ref() {
            None => storage.item,
            Some(selection) => {
                let selected = self.selected(actor, &selection.field, selection.mask)?;
                selection
                    .values
                    .iter()
                    .find(|value| value.value == selected)
                    .and_then(|value| value.item.clone())
            }
        };
        let item = item.ok_or_else(|| GuestError::invalid("QC regular armor selection is undeclared"))?;
        Ok(QcRegularArmor { points, item })
    }

    /// Read powered protection.
    pub fn read_powered(&mut self, actor: &ActorId) -> Result<QcPoweredArmor, GuestError> {
        let definition = self.lane(actor, ProtectionChannel::Powered)?;
        let storage = definition
            .powered
            .ok_or_else(|| GuestError::invalid("QC protection requires storage for its channel"))?;
        let kind = match storage.selection.as_ref() {
            None => storage.kind,
            Some(selection) => {
                let selected = self.selected(actor, &selection.field, selection.mask)?;
                selection
                    .values
                    .iter()
                    .find(|value| value.value == selected)
                    .map(|value| value.kind)
                    .ok_or_else(|| GuestError::invalid("QC powered armor selection is undeclared"))?
            }
        };
        if kind == PoweredKind::None {
            Ok(QcPoweredArmor { kind, cells: 0.0 })
        } else {
            Ok(QcPoweredArmor {
                kind,
                cells: self.count(actor, &storage.cells, None)?,
            })
        }
    }

    /// Validate a regular armor write.
    fn validate_regular(
        &self,
        actor: &ActorId,
        definition: &ModQcProtection,
        next: &QcRegularArmor,
    ) -> Result<(), GuestError> {
        self.require(actor)?;
        let storage = definition
            .regular
            .as_ref()
            .ok_or_else(|| GuestError::invalid("QC protection requires storage for its channel"))?;
        valid_count(next.points)?;
        match storage.selection.as_ref() {
            None => {
                if storage.item != next.item {
                    return Err(GuestError::invalid("QC regular armor item is undeclared"));
                }
            }
            Some(selection) => {
                if !selection.values.iter().any(|value| value.item == next.item) {
                    return Err(GuestError::invalid("QC regular armor item is undeclared"));
                }
            }
        }
        Ok(())
    }

    /// Validate a powered protection write.
    fn validate_powered(
        &self,
        actor: &ActorId,
        definition: &ModQcProtection,
        next: &QcPoweredArmor,
    ) -> Result<(), GuestError> {
        self.require(actor)?;
        valid_count(if next.kind == PoweredKind::None {
            0.0
        } else {
            next.cells
        })?;
        let storage = definition
            .powered
            .as_ref()
            .ok_or_else(|| GuestError::invalid("QC protection requires storage for its channel"))?;
        match storage.selection.as_ref() {
            None => {
                if storage.kind != next.kind {
                    return Err(GuestError::invalid("QC powered armor kind is undeclared"));
                }
            }
            Some(selection) => {
                if !selection.values.iter().any(|value| value.kind == next.kind) {
                    return Err(GuestError::invalid("QC powered armor kind is undeclared"));
                }
            }
        }
        Ok(())
    }

    /// Write regular armor.
    pub fn write_regular(&mut self, actor: &ActorId, next: &QcRegularArmor) -> Result<(), GuestError> {
        let definition = self.lane(actor, ProtectionChannel::Regular)?;
        self.validate_regular(actor, &definition, next)?;
        let storage = definition
            .regular
            .ok_or_else(|| GuestError::invalid("QC protection requires storage for its channel"))?;
        if let Some(selection) = storage.selection.as_ref() {
            let selected = selection
                .values
                .iter()
                .find(|value| value.item == next.item)
                .ok_or_else(|| GuestError::invalid("Missing validated QC armor selection"))?;
            self.select(actor, &selection.field, selection.mask, selected.value)?;
        }
        self.count(actor, &storage.points, Some(next.points))?;
        self.rebase(actor)?;
        Ok(())
    }

    /// Write powered protection.
    pub fn write_powered(&mut self, actor: &ActorId, next: &QcPoweredArmor) -> Result<(), GuestError> {
        let definition = self.lane(actor, ProtectionChannel::Powered)?;
        self.validate_powered(actor, &definition, next)?;
        let storage = definition
            .powered
            .ok_or_else(|| GuestError::invalid("QC protection requires storage for its channel"))?;
        if let Some(selection) = storage.selection.as_ref() {
            let selected = selection
                .values
                .iter()
                .find(|value| value.kind == next.kind)
                .ok_or_else(|| GuestError::invalid("Missing validated QC power selection"))?;
            self.select(actor, &selection.field, selection.mask, selected.value)?;
        }
        self.count(
            actor,
            &storage.cells,
            Some(if next.kind == PoweredKind::None {
                0.0
            } else {
                next.cells
            }),
        )?;
        self.rebase(actor)?;
        Ok(())
    }

    /// Current projection across bound channels.
    fn projection(&mut self, actor: &ActorId) -> Result<ProtectionProjection, GuestError> {
        let channels: Vec<ProtectionChannel> = self.require(actor)?.lanes.iter().map(|lane| lane.channel).collect();
        let mut result = ProtectionProjection::default();
        for channel in channels {
            match channel {
                ProtectionChannel::Regular => result.regular = Some(self.read_regular(actor)?),
                ProtectionChannel::Powered => result.powered = Some(self.read_powered(actor)?),
            }
        }
        Ok(result)
    }

    /// Rebase staged projections after a write.
    fn rebase(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        let state = self.projection(actor)?;
        for stage in &mut self.stages {
            if stage.actor == *actor {
                stage.before = state.clone();
            }
        }
        Ok(())
    }

    /// Activate a reserved actor's channels.
    pub fn activate(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        let bound = self.require(actor)?.bound;
        if bound {
            return Ok(());
        }
        // Validate every lane reads before exposing the channels.
        if self.projection(actor).is_err() {
            self.release(actor)?;
            return Err(GuestError::invalid("QC protection activation failed"));
        }
        if let Some(entry) = self.entries.get_mut(actor) {
            entry.bound = true;
        }
        Ok(())
    }

    /// Reservation for a channel.
    pub fn reservation(&self, actor: &ActorId, channel: ProtectionChannel) -> Result<u64, GuestError> {
        let entry = self.require(actor)?;
        entry
            .lanes
            .iter()
            .find(|lane| lane.channel == channel)
            .map(|lane| lane.reservation)
            .ok_or_else(|| GuestError::invalid("QC protection requires its declared channel"))
    }

    /// Observe an entity store for one reference.
    pub fn observe(&mut self, reference: i32) -> Result<(), GuestError> {
        if self.stages.is_empty() {
            return Ok(());
        }
        let actor = match self.services.actor_for_reference(reference) {
            Some(actor) => actor,
            None => return Ok(()),
        };
        let matching: Vec<usize> = self
            .stages
            .iter()
            .enumerate()
            .filter(|(_, stage)| stage.actor == actor)
            .map(|(index, _)| index)
            .collect();
        let after = if matching.is_empty() {
            ProtectionProjection::default()
        } else {
            self.projection(&actor)?
        };
        let current = matching.last().copied();
        for index in matching {
            let before = std::mem::replace(&mut self.stages[index].before, after.clone());
            if Some(index) != current {
                continue;
            }
            let stage = &mut self.stages[index];
            let regular = match (&before.regular, &after.regular) {
                (Some(before), Some(after)) if before != after => Some((before.clone(), after.clone())),
                _ => None,
            };
            let powered = match (&before.powered, &after.powered) {
                (Some(before), Some(after)) if before != after => Some((before.clone(), after.clone())),
                _ => None,
            };
            if regular.is_some() || powered.is_some() {
                stage.observer.stored(&ProtectionChange { regular, powered });
            }
        }
        Ok(())
    }

    /// Watch protection stores while running a grant.
    pub fn watch<T>(
        &mut self,
        actor: &ActorId,
        observer: Box<dyn ProtectionObserver>,
        run: impl FnOnce(&mut Self) -> Result<T, GuestError>,
    ) -> Result<T, GuestError> {
        if !self.entries.contains_key(actor) {
            return run(self);
        }
        let before = self.projection(actor)?;
        self.stages.push(Stage {
            actor: actor.clone(),
            observer,
            before,
        });
        let result = run(self);
        self.stages.pop();
        result
    }

    /// Absorb damage through one channel.
    pub fn absorb(
        &mut self,
        actor: &ActorId,
        channel: ProtectionChannel,
        input: &ArmorStageInput,
        observer: Box<dyn ProtectionObserver>,
    ) -> Result<f64, GuestError> {
        let definition = self.lane(actor, channel)?;
        let scale = input.flags.regular_protection_scale.unwrap_or(1.0);
        if channel == ProtectionChannel::Regular && scale != 1.0 {
            let call = definition.absorb.call();
            let mentions = call
                .arguments
                .iter()
                .chain(call.globals.iter().map(|global| &global.value))
                .any(|value| matches!(value, super::mod_provider::ModCallbackValue::Input(name) if *name == ModCallbackInput::RegularProtectionScale));
            if !mentions {
                return Err(GuestError::invalid(
                    "QC regular protection scale requires an explicit source input",
                ));
            }
        }
        let flags = definition.flags;
        let damage_flags = (if input.flags.no_armor { flags.no_armor } else { 0 })
            | (if input.flags.no_power_armor {
                flags.no_power_armor
            } else {
                0
            })
            | (if input.flags.no_regular_armor {
                flags.no_regular_armor
            } else {
                0
            })
            | (if input.flags.energy { flags.energy } else { 0 })
            | (if input.delivery == DamageDelivery::Radius {
                flags.radius
            } else {
                0
            });
        let now = self.services.now().as_seconds_f64();
        let mut inputs = QcModInputs::new();
        inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(Some(actor.clone())));
        inputs.insert(
            ModCallbackInput::Attacker,
            ModRuntimeValue::Actor(input.attacker.clone()),
        );
        inputs.insert(
            ModCallbackInput::Inflictor,
            ModRuntimeValue::Actor(input.inflictor.clone()),
        );
        inputs.insert(ModCallbackInput::Amount, ModRuntimeValue::Float(input.amount));
        inputs.insert(
            ModCallbackInput::DamageFlags,
            ModRuntimeValue::Float(f64::from(damage_flags)),
        );
        inputs.insert(ModCallbackInput::Knockback, ModRuntimeValue::Float(input.knockback));
        inputs.insert(ModCallbackInput::Direction, ModRuntimeValue::Vector(input.direction));
        inputs.insert(ModCallbackInput::RegularProtectionScale, ModRuntimeValue::Float(scale));
        inputs.insert(ModCallbackInput::Point, ModRuntimeValue::Vector(input.point));
        inputs.insert(ModCallbackInput::Normal, ModRuntimeValue::Vector(input.normal));
        inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(now));
        let region = match &definition.absorb {
            ModProtectionAbsorb::Region { stage, .. } => {
                self.regions.iter().find(|region| region.entry == stage.entry).cloned()
            }
            ModProtectionAbsorb::Function { .. } => None,
        };
        let call = definition.absorb.call().clone();
        let owned = actor.clone();
        self.watch(actor, observer, |this| {
            let saved = this.dispatch.invoke(&call, &inputs, region.as_ref())?;
            if this.services.is_live(&owned) && this.entries.contains_key(&owned) {
                this.require(&owned)?;
            }
            Ok(saved)
        })
    }

    /// Release a client actor.
    pub fn release(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        let entry = match self.entries.remove(actor) {
            Some(entry) => entry,
            None => return Ok(()),
        };
        let mut errors = Vec::new();
        for lane in entry.lanes {
            if let Err(error) = self.services.release_reservation(lane.reservation) {
                errors.push(error.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(GuestError::Callback(format!(
                "QC protection release failed: {}",
                errors.join("; ")
            )))
        }
    }

    /// Release all client actors.
    pub fn close(&mut self) -> Result<(), GuestError> {
        let mut errors = Vec::new();
        let actors: Vec<ActorId> = self.entries.keys().cloned().collect();
        for actor in actors {
            if let Err(error) = self.release(&actor) {
                errors.push(error.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(GuestError::Callback(format!(
                "QC protection close failed: {}",
                errors.join("; ")
            )))
        }
    }

    /// Require an idle stage stack.
    pub fn assert_idle(&self) -> Result<(), GuestError> {
        if self.stages.is_empty() {
            Ok(())
        } else {
            Err(GuestError::invalid("QC protection requires an idle source stage"))
        }
    }
}

/// Validate a protection count.
fn valid_count(count: f64) -> Result<(), GuestError> {
    if count < 0.0 || !count.is_finite() || f64::from(count as f32) != count {
        return Err(GuestError::invalid("QC protection count is not representable"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::mod_provider::{
        ModActorBinding, ModActorField, ModCallbackValue, ModClientDeclaration, ModPoweredStorage, ModProtectionFlags,
        ModRegularStorage, ProtectionAdmission, QcApiKind, QcFunctionView,
    };

    struct FakeProgram {
        fields: HashMap<String, QcValueType>,
    }

    impl QcProgramView for FakeProgram {
        fn digest(&self) -> &str {
            "abc"
        }

        fn api_kind(&self) -> QcApiKind {
            QcApiKind::Q1Netquake
        }

        fn field_type(&self, name: &str) -> Option<QcValueType> {
            self.fields.get(name).copied()
        }

        fn global_type(&self, _name: &str) -> Option<QcValueType> {
            None
        }

        fn function_named(&self, _name: &str) -> Option<QcFunctionView> {
            None
        }

        fn function_at(&self, _index: i32) -> Option<QcFunctionView> {
            None
        }

        fn functions(&self) -> Vec<QcFunctionView> {
            Vec::new()
        }
    }

    struct FakeServices {
        owner: IdentityOwner,
        live: Vec<ActorId>,
        clients: HashMap<ActorId, ClientId>,
        reservations: HashMap<u64, ProtectionChannel>,
        next: u64,
    }

    impl QcProtectionServices for FakeServices {
        fn now(&self) -> SourceTime {
            SourceTime::Seconds(6.0)
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.live
                .iter()
                .find(|live| *live == actor)
                .and_then(|actor| self.owner.owned_actor(actor, ProviderId::new("mod", "test")).ok())
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

        fn reference_for_actor(&self, actor: &ActorId) -> Option<i32> {
            Some(actor.slot() as i32)
        }

        fn actor_for_reference(&self, reference: i32) -> Option<ActorId> {
            self.live.iter().find(|actor| actor.slot() as i32 == reference).cloned()
        }

        fn reserve(&mut self, _owner: &OwnedActor, channel: ProtectionChannel, _rule: &str) -> Result<u64, GuestError> {
            let id = self.next;
            self.next += 1;
            self.reservations.insert(id, channel);
            Ok(id)
        }

        fn release_reservation(&mut self, reservation: u64) -> Result<(), GuestError> {
            self.reservations
                .remove(&reservation)
                .ok_or_else(|| GuestError::invalid("Unknown reservation"))?;
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeMachine {
        words: HashMap<(i32, String), f32>,
    }

    impl QcProtectionMachine for FakeMachine {
        fn float_for(&self, reference: i32, field: &str) -> Result<f32, GuestError> {
            Ok(self.words.get(&(reference, field.to_string())).copied().unwrap_or(0.0))
        }

        fn set_float_for(&mut self, reference: i32, field: &str, value: f32) -> Result<(), GuestError> {
            self.words.insert((reference, field.to_string()), value);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeDispatch {
        calls: Vec<(String, QcModInputs, Option<QcArmorStageView>)>,
        saved: f64,
    }

    impl QcProtectionDispatch for FakeDispatch {
        fn invoke(
            &mut self,
            call: &ModSourceCall,
            inputs: &QcModInputs,
            region: Option<&QcArmorStageView>,
        ) -> Result<f64, GuestError> {
            self.calls
                .push((call.function.clone(), inputs.clone(), region.cloned()));
            Ok(self.saved)
        }
    }

    struct Discard;

    impl ProtectionObserver for Discard {
        fn stored(&mut self, _change: &ProtectionChange) {}
    }

    fn absorb_call() -> ModSourceCall {
        ModSourceCall {
            function: "absorb".to_string(),
            arguments: vec![ModCallbackValue::Input(ModCallbackInput::Amount)],
            globals: vec![],
        }
    }

    fn declaration() -> ModCallbackDeclaration {
        ModCallbackDeclaration {
            actor_fields: vec![
                ModActorField {
                    field: "armorvalue".to_string(),
                    binding: ModActorBinding::Private,
                },
                ModActorField {
                    field: "cells".to_string(),
                    binding: ModActorBinding::Private,
                },
            ],
            clients: Some(ModClientDeclaration {
                maximum: 4,
                ..ModClientDeclaration::default()
            }),
            protection: vec![
                ModQcProtection {
                    id: "regular".to_string(),
                    admission: ProtectionAdmission::Claim,
                    channel: ProtectionChannel::Regular,
                    regular: Some(ModRegularStorage {
                        points: "armorvalue".to_string(),
                        item: Some("q1:item_armor2".to_string()),
                        selection: None,
                    }),
                    powered: None,
                    absorb: ModProtectionAbsorb::Function { call: absorb_call() },
                    flags: ModProtectionFlags {
                        no_armor: 1,
                        no_power_armor: 2,
                        no_regular_armor: 4,
                        energy: 8,
                        radius: 16,
                    },
                },
                ModQcProtection {
                    id: "powered".to_string(),
                    admission: ProtectionAdmission::Claim,
                    channel: ProtectionChannel::Powered,
                    regular: None,
                    powered: Some(ModPoweredStorage {
                        cells: "cells".to_string(),
                        kind: PoweredKind::Screen,
                        selection: None,
                    }),
                    absorb: ModProtectionAbsorb::Function { call: absorb_call() },
                    flags: ModProtectionFlags {
                        no_armor: 1,
                        no_power_armor: 2,
                        no_regular_armor: 4,
                        energy: 8,
                        radius: 16,
                    },
                },
            ],
            ..ModCallbackDeclaration::default()
        }
    }

    fn program() -> FakeProgram {
        FakeProgram {
            fields: [
                ("armorvalue".to_string(), QcValueType::Float),
                ("cells".to_string(), QcValueType::Float),
            ]
            .into_iter()
            .collect(),
        }
    }

    fn fixture() -> QcModProtection<FakeServices, FakeMachine, FakeDispatch> {
        QcModProtection::new(
            declaration(),
            ProviderId::new("mod", "test"),
            &program(),
            FakeServices {
                owner: IdentityOwner::create("protection").unwrap(),
                live: Vec::new(),
                clients: HashMap::new(),
                reservations: HashMap::new(),
                next: 1,
            },
            FakeMachine::default(),
            FakeDispatch::default(),
        )
        .unwrap()
    }

    fn join(protection: &mut QcModProtection<FakeServices, FakeMachine, FakeDispatch>) -> ActorId {
        let actor = protection.services.owner.actor(2, 1);
        let client = protection.services.owner.client(2, 1);
        protection.services.live.push(actor.clone());
        protection.services.clients.insert(actor.clone(), client);
        actor
    }

    fn absorb_input() -> ArmorStageInput {
        ArmorStageInput {
            attacker: None,
            inflictor: None,
            amount: 30.0,
            knockback: 0.0,
            delivery: DamageDelivery::Radius,
            flags: ArmorDamageFlags {
                no_armor: true,
                no_power_armor: false,
                no_regular_armor: false,
                energy: false,
                regular_protection_scale: None,
            },
            direction: vec3(0.0, 0.0, 1.0),
            point: vec3(1.0, 2.0, 3.0),
            normal: vec3(0.0, 0.0, 1.0),
        }
    }

    #[test]
    fn regions_reject_duplicate_channels() {
        let mut bad = declaration();
        bad.protection.pop();
        bad.protection.push(bad.protection[0].clone());
        assert!(qc_protection_regions(&program(), &bad).is_err());
        assert!(qc_protection_regions(&program(), &declaration()).unwrap().is_empty());
    }

    #[test]
    fn reserve_activate_read_write() {
        let mut protection = fixture();
        let actor = join(&mut protection);
        protection.reserve(&actor).unwrap();
        protection.activate(&actor).unwrap();
        assert!(protection.reservation(&actor, ProtectionChannel::Regular).is_ok());
        protection.machine.words.insert((2, "armorvalue".to_string()), 50.0);
        let regular = protection.read_regular(&actor).unwrap();
        assert_eq!(
            regular,
            QcRegularArmor {
                points: 50.0,
                item: Some("q1:item_armor2".to_string())
            }
        );
        protection
            .write_regular(
                &actor,
                &QcRegularArmor {
                    points: 40.0,
                    item: Some("q1:item_armor2".to_string()),
                },
            )
            .unwrap();
        assert_eq!(protection.read_regular(&actor).unwrap().points, 40.0);
        protection
            .write_powered(
                &actor,
                &QcPoweredArmor {
                    kind: PoweredKind::Screen,
                    cells: 10.0,
                },
            )
            .unwrap();
        assert_eq!(protection.read_powered(&actor).unwrap().cells, 10.0);
        protection.release(&actor).unwrap();
        assert!(protection.services.reservations.is_empty());
    }

    #[test]
    fn writes_validate_items_and_counts() {
        let mut protection = fixture();
        let actor = join(&mut protection);
        protection.reserve(&actor).unwrap();
        protection.activate(&actor).unwrap();
        assert!(protection
            .write_regular(
                &actor,
                &QcRegularArmor {
                    points: 10.0,
                    item: Some("q1:item_armor1".to_string())
                }
            )
            .is_err());
        assert!(protection
            .write_regular(
                &actor,
                &QcRegularArmor {
                    points: -1.0,
                    item: Some("q1:item_armor2".to_string())
                }
            )
            .is_err());
        assert!(protection
            .write_powered(
                &actor,
                &QcPoweredArmor {
                    kind: PoweredKind::Shield,
                    cells: 5.0
                }
            )
            .is_err());
    }

    #[test]
    fn absorb_invokes_with_damage_flags() {
        let mut protection = fixture();
        let actor = join(&mut protection);
        protection.reserve(&actor).unwrap();
        protection.activate(&actor).unwrap();
        protection.dispatch.saved = 12.0;
        let saved = protection
            .absorb(&actor, ProtectionChannel::Regular, &absorb_input(), Box::new(Discard))
            .unwrap();
        assert_eq!(saved, 12.0);
        let (_, inputs, region) = protection.dispatch.calls.last().unwrap();
        assert!(region.is_none());
        assert_eq!(
            inputs.get(&ModCallbackInput::DamageFlags),
            Some(&ModRuntimeValue::Float(17.0))
        );
        assert_eq!(
            inputs.get(&ModCallbackInput::Amount),
            Some(&ModRuntimeValue::Float(30.0))
        );
        protection.assert_idle().unwrap();
    }

    #[test]
    fn absorb_requires_explicit_scale_input() {
        let mut protection = fixture();
        let actor = join(&mut protection);
        protection.reserve(&actor).unwrap();
        protection.activate(&actor).unwrap();
        let mut input = absorb_input();
        input.flags.regular_protection_scale = Some(0.5);
        assert!(protection
            .absorb(&actor, ProtectionChannel::Regular, &input, Box::new(Discard))
            .is_err());
    }

    #[test]
    fn observe_reports_staged_stores() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let mut protection = fixture();
        let actor = join(&mut protection);
        protection.reserve(&actor).unwrap();
        protection.activate(&actor).unwrap();
        protection.machine.words.insert((2, "armorvalue".to_string()), 50.0);
        let changes: Rc<RefCell<Vec<ProtectionChange>>> = Rc::new(RefCell::new(Vec::new()));
        struct Shared {
            changes: Rc<RefCell<Vec<ProtectionChange>>>,
        }
        impl ProtectionObserver for Shared {
            fn stored(&mut self, change: &ProtectionChange) {
                self.changes.borrow_mut().push(change.clone());
            }
        }
        let actor_clone = actor.clone();
        protection
            .watch(
                &actor,
                Box::new(Shared {
                    changes: changes.clone(),
                }),
                |protection| {
                    assert!(protection.assert_idle().is_err());
                    protection.machine.set_float_for(2, "armorvalue", 20.0).unwrap();
                    protection.observe(2).unwrap();
                    Ok(())
                },
            )
            .unwrap();
        let changes = changes.borrow();
        assert_eq!(changes.len(), 1);
        let (before, after) = changes[0].regular.as_ref().unwrap();
        assert_eq!((before.points, after.points), (50.0, 20.0));
        assert_eq!(actor_clone, actor);
        protection.close().unwrap();
    }
}
