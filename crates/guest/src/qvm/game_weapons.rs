//! Primary weapons: dispatcher, give/drop hooks, and player services.
//!
//! Provenance: `src/compat/qvm/game-weapons.ts`.
//!
//! Async branches collapse to the sync path. Region and branch callbacks are
//! infallible in the hub, so fallible region bodies stash their first error
//! for the post-proceed check. Prepared-weapon failures record through
//! [`QvmPrimaryWeapons::take_error`].
//!
//! Local mirrors: the used surface of `src/contracts/source-match.ts`
//! ([`QvmPrimaryMatch`]; the int32 score check is unrepresentable) and
//! `src/contracts/source-items.ts` ([`QvmEquipmentContext`],
//! [`qvm_equipment_item`]). Selection values reuse [`super::item_storage`],
//! movement scopes reuse [`super::game_input::QvmApplicationScope`],
//! equipment reuses [`super::game_equipment_movement`], and dispatch reuses
//! [`super::mod_weapon_stage`].
//!
//! [`QvmGameData`]: super::game_data::QvmGameData

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use qa_world::combat::ItemId;

use super::game_data::{
    qualify_qvm_region, qualify_qvm_region_evaluation, AbiProfile, ModuleIdentity, QvmArtifact, QvmBranchBinding,
    QvmFunctionCall, QvmGameData, QvmHookFn, QvmModule, QvmOpcode, QvmRegionAccess, QvmRegionBinding,
    QvmRegionDecision, QvmRegionEvaluation,
};
use super::game_equipment_movement::{
    QvmEquipmentMotion, QvmEquipmentMovement, QvmEquipmentMovementProfile, QvmEquipmentServices,
};
use super::game_input::QvmApplicationScope;
use super::game_inventory::QvmInventoryWord;
use super::item_storage::QvmWeaponActor;
use super::mod_weapon_stage::{
    validate_qvm_weapon_dispatcher, QvmWeaponDispatcher, QvmWeaponDispatcherDefinition, QvmWeaponDispatcherOperations,
};
use crate::error::GuestError;

/// Primary match declaration (score binding only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmPrimaryMatch {
    /// Score offset in the client record.
    pub score: usize,
}

/// Equipment cadence context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmEquipmentContext {
    /// Provider name (`namespace:name`).
    pub provider: String,
    /// Cadence item, if any.
    pub item: Option<ItemId>,
}

/// Resolve the cadence item for exactly one matching context.
pub fn qvm_equipment_item(contexts: &[QvmEquipmentContext], provider: &str) -> Result<Option<ItemId>, GuestError> {
    let mut matches = contexts.iter().filter(|context| context.provider == provider);
    let Some(context) = matches.next() else {
        return Err(GuestError::invalid(format!(
            "Equipment {provider} has no declared original cadence context"
        )));
    };
    if matches.next().is_some() {
        return Err(GuestError::invalid(format!(
            "Equipment {provider} has duplicate original cadence contexts"
        )));
    }
    Ok(context.item.clone())
}

/// Qualified region bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmWeaponRegion {
    /// Region entry.
    pub entry: usize,
    /// Region join.
    pub join: usize,
}

/// Give categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmGiveCategory {
    /// Weapons grant.
    Weapons,
    /// Ammo grant.
    Ammo,
}

/// Death-drop projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmDeathDrop {
    /// Dropped weapon value (0 for none).
    pub weapon: i32,
    /// Dropped ammo counter.
    pub ammo: i32,
    /// Projected inventory words, if any.
    pub ammo_words: Option<Vec<QvmInventoryWord>>,
}

/// Drop ammo storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmDropAmmo {
    /// Player-state offset.
    Offset(usize),
    /// Canonical inventory projection.
    Inventory,
}

/// Spawn point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmSpawnPoint {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
}

/// Powerup selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmPowerup {
    /// Quad damage.
    Quad,
    /// Haste.
    Haste,
    /// Flight.
    Flight,
}

/// Primary weapon profile.
#[derive(Clone)]
pub struct QvmPrimaryWeaponProfile {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Match declaration, if any.
    pub match_declaration: Option<QvmPrimaryMatch>,
    /// ABI profile.
    pub abi_profile: AbiProfile,
    /// Equipment movement profile.
    pub equipment_movement: QvmEquipmentMovementProfile,
    /// Entity stride in bytes.
    pub entity_stride: usize,
    /// Client stride in bytes.
    pub client_stride: usize,
    /// Client pointer offset.
    pub client_pointer: usize,
    /// Dispatcher definition.
    pub stage: QvmWeaponDispatcherDefinition,
    /// Damage-factor evaluation.
    pub damage_factor: QvmDamageFactor,
    /// Equipment contexts.
    pub equipment_contexts: Vec<QvmEquipmentContext>,
    /// Weapon-delay evaluation.
    pub delay: QvmRegionEvaluation,
    /// Delay player projection.
    pub delay_player: QvmDelayPlayer,
    /// Teleport evaluation.
    pub teleport: QvmTeleport,
    /// Maximum-health offset.
    pub max_health: usize,
    /// Persistent maximum-health offset.
    pub persistent_max_health: usize,
    /// Availability policy.
    pub availability: QvmAvailability,
    /// Powerup offsets.
    pub powerups: QvmPowerupOffsets,
    /// Torso-animation call.
    pub torso_animation: QvmTorsoAnimation,
    /// Water-level offsets.
    pub water_level: QvmWaterLevel,
    /// Drop hook.
    pub drop: QvmDropHook,
    /// Give hook.
    pub give: QvmGiveHook,
}

/// Damage-factor evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmDamageFactor {
    /// Evaluation entry.
    pub entry: usize,
    /// Result word address.
    pub result: usize,
    /// Stop region.
    pub stop: QvmWeaponRegion,
}

/// Delay player projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmDelayPlayer {
    /// Movement global address.
    pub movement_global: usize,
    /// Player offset within the movement record.
    pub player_offset: usize,
}

/// Teleport evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmTeleport {
    /// Evaluation entry.
    pub entry: usize,
    /// Teleport region.
    pub region: QvmRegionEvaluation,
    /// Objectives region.
    pub objectives: QvmRegionEvaluation,
    /// Spawn entry.
    pub spawn: usize,
    /// View entry.
    pub view: usize,
}

/// Availability policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmAvailability {
    /// Movement-type offset.
    pub movement_type: usize,
    /// Excluded movement types.
    pub excluded: Vec<i32>,
    /// Health offset.
    pub health: usize,
    /// Team offset.
    pub team: usize,
    /// Spectator team value.
    pub spectator_team: i32,
    /// Flags offset.
    pub flags: usize,
    /// Respawn flag.
    pub respawn_flag: i32,
}

/// Powerup offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmPowerupOffsets {
    /// Quad offset.
    pub quad: usize,
    /// Haste offset.
    pub haste: usize,
    /// Flight offset.
    pub flight: usize,
}

/// Torso-animation call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmTorsoAnimation {
    /// Animation entry.
    pub entry: usize,
    /// Attack value.
    pub attack: i32,
    /// Melee value.
    pub melee: i32,
}

/// Water-level offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmWaterLevel {
    /// Entity offset.
    pub entity_offset: usize,
    /// Movement offset.
    pub movement_offset: usize,
}

/// Drop hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmDropHook {
    /// Drop entry.
    pub entry: usize,
    /// Entity argument word.
    pub argument: usize,
    /// Weapon offset within the entity record.
    pub weapon: usize,
    /// Ammo storage.
    pub ammo: QvmDropAmmo,
    /// Drop region.
    pub region: QvmWeaponRegion,
}

/// Give hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmGiveHook {
    /// Give entry.
    pub entry: usize,
    /// Player argument word.
    pub argument: usize,
    /// Weapons completion decision.
    pub weapons: usize,
    /// Ammo completion decision.
    pub ammo: usize,
    /// Named-item region.
    pub named: QvmGiveNamed,
}

/// Named-item region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmGiveNamed {
    /// Region entry.
    pub entry: usize,
    /// Region join.
    pub join: usize,
    /// Name local offset.
    pub name: usize,
    /// Item local offset.
    pub item: usize,
}

/// Weapon host services.
#[derive(Clone)]
pub struct QvmWeaponServices {
    /// Canonical actor for a slot.
    pub actor: Rc<dyn Fn(usize) -> Option<ActorId>>,
    /// Current slot for an actor.
    pub slot: Rc<dyn Fn(&ActorId) -> Option<usize>>,
    /// Whether an actor is selected.
    pub selected: Rc<dyn Fn(&ActorId) -> bool>,
    /// Equipment motion, if any.
    pub equipment_movement: Option<Rc<dyn Fn(&ActorId) -> Option<QvmEquipmentMotion>>>,
    /// Observe an attempted weapon value.
    pub attempted: Rc<dyn Fn(&ActorId, i32)>,
    /// Observe an accepted weapon value.
    pub accepted: Rc<dyn Fn(&ActorId, i32)>,
    /// Observe completion.
    pub completed: Rc<dyn Fn(&ActorId, bool)>,
    /// Observe a give grant.
    pub give: Rc<dyn Fn(&ActorId, QvmGiveCategory)>,
    /// Grant a named item; returns whether the source call is consumed.
    pub give_item: Rc<dyn Fn(&ActorId, &str) -> bool>,
    /// Project a death drop.
    pub drop: Rc<dyn Fn(&ActorId) -> Option<QvmDeathDrop>>,
}

/// Source game handles.
#[derive(Clone)]
pub struct QvmWeaponGame {
    /// Source module.
    pub module: QvmModule,
    /// Located game data.
    pub data: QvmGameData,
}

/// Evaluation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QvmEvaluationKind {
    /// Damage factor.
    DamageFactor,
    /// Teleport.
    Teleport,
    /// Objectives.
    Objectives,
}

/// Pending source-effect evaluation.
#[derive(Debug, Clone, PartialEq)]
struct QvmWeaponEvaluation {
    /// Acting actor.
    actor: ActorId,
    /// Evaluation kind.
    kind: QvmEvaluationKind,
    /// Whether the entry ran.
    entered: bool,
    /// Damage-factor result, if any.
    result: Option<f32>,
}

/// Shared weapon state behind hook closures.
struct QvmWeaponState {
    /// Source game handles.
    game: QvmWeaponGame,
    /// Weapon profile.
    profile: QvmPrimaryWeaponProfile,
    /// Host services.
    services: QvmWeaponServices,
    /// Equipment movement, if any.
    equipment: Option<Rc<QvmEquipmentMovement>>,
    /// Pending evaluations.
    evaluations: Vec<QvmWeaponEvaluation>,
    /// Scratch address.
    scratch: usize,
    /// Whether closed.
    closed: bool,
    /// First recorded failure.
    error: Option<GuestError>,
}

/// Whether an actor is live.
fn live(state: &Rc<RefCell<QvmWeaponState>>, actor: &ActorId) -> bool {
    let inner = state.borrow();
    if inner.closed {
        return false;
    }
    let Some(slot) = (inner.services.slot)(actor) else {
        return false;
    };
    (inner.services.actor)(slot).as_ref() == Some(actor)
}

/// Locate the player record owned by an actor.
fn pointer(state: &Rc<RefCell<QvmWeaponState>>, actor: &ActorId) -> Result<usize, GuestError> {
    let (game, profile, slot) = {
        let inner = state.borrow();
        let slot = (inner.services.slot)(actor);
        (inner.game.clone(), inner.profile.clone(), slot)
    };
    let Some(slot) = slot else {
        return Err(GuestError::invalid("Primary QVM weapon actor is no longer current"));
    };
    if !live(state, actor) {
        return Err(GuestError::invalid("Primary QVM weapon actor is no longer current"));
    }
    let located = game.data.checkpoint();
    if located.entity_stride != profile.entity_stride || located.client_stride != profile.client_stride {
        return Err(GuestError::invalid(
            "Primary QVM weapon records differ from their qualified layout",
        ));
    }
    let found = game.data.entity_bytes(slot)?.get_i32(profile.client_pointer)?;
    game.data.public_player_bytes(slot)?;
    let expected = located.clients_word + slot * located.client_stride;
    if i64::from(found) != expected as i64 {
        return Err(GuestError::invalid(
            "Primary QVM actor does not own its original player state",
        ));
    }
    usize::try_from(found).map_err(|_| GuestError::invalid("Primary QVM actor does not own its original player state"))
}

/// Resolve the dispatcher actor for a source reference.
fn dispatch_actor(
    state: &Rc<RefCell<QvmWeaponState>>,
    source: &QvmWeaponActor,
    call: &mut QvmFunctionCall,
) -> Result<Option<ActorId>, GuestError> {
    if state.borrow().closed {
        return Ok(None);
    }
    let reference = &source.pointer;
    let mut found = match &reference.base {
        super::mod_actors::QvmModInputPointerBase::Argument { index } => call.argument(*index)?,
        super::mod_actors::QvmModInputPointerBase::Global { address } => call.guest.read_i32(*address)?,
    };
    for offset in &reference.indirections {
        let base = usize::try_from(found)
            .map_err(|_| GuestError::invalid("Primary weapon dispatcher has no located source client"))?;
        found = call.guest.read_i32(base + offset)?;
    }
    let base = usize::try_from(found)
        .map_err(|_| GuestError::invalid("Primary weapon dispatcher has no located source client"))?;
    let address = base + reference.offset;
    let slot = {
        let inner = state.borrow();
        let located = inner.game.data.checkpoint();
        address
            .checked_sub(located.clients_word)
            .filter(|offset| located.client_stride != 0 && offset % located.client_stride == 0)
            .map(|offset| offset / located.client_stride)
    };
    let Some(slot) = slot else {
        return Err(GuestError::invalid(
            "Primary weapon dispatcher has no located source client",
        ));
    };
    if source.record != "client" {
        return Err(GuestError::invalid(
            "Primary weapon dispatcher has no located source client",
        ));
    }
    let actor = {
        let inner = state.borrow();
        (inner.services.actor)(slot)
    };
    let Some(actor) = actor else {
        return Ok(None);
    };
    if pointer(state, &actor)? != address {
        return Err(GuestError::invalid(
            "Primary weapon dispatcher has a different source actor",
        ));
    }
    Ok(Some(actor))
}

/// Record the first failure.
fn record_error(state: &Rc<RefCell<QvmWeaponState>>, error: GuestError) {
    let mut inner = state.borrow_mut();
    if inner.error.is_none() {
        inner.error = Some(error);
    }
}

/// Run a hook closure, recording the first failure.
fn run_hook(state: &Rc<RefCell<QvmWeaponState>>, run: impl FnOnce() -> Result<i32, GuestError>) -> i32 {
    match run() {
        Ok(result) => result,
        Err(error) => {
            record_error(state, error);
            0
        }
    }
}

/// Give interception.
fn give_call(state: &Rc<RefCell<QvmWeaponState>>, call: &mut QvmFunctionCall) -> Result<i32, GuestError> {
    let (game, profile, services) = {
        let inner = state.borrow();
        (inner.game.clone(), inner.profile.clone(), inner.services.clone())
    };
    let hook = &profile.give;
    let slot = game.data.number_from_pointer(call.argument(hook.argument)?)?;
    let actor = (services.actor)(slot);
    let Some(actor) = actor else {
        return Ok(call.proceed());
    };
    if !live(state, &actor) {
        return Ok(call.proceed());
    }
    pointer(state, &actor)?;
    for (category, index) in [
        (QvmGiveCategory::Weapons, hook.weapons),
        (QvmGiveCategory::Ammo, hook.ammo),
    ] {
        let state = Rc::clone(state);
        let actor = actor.clone();
        call.branches(vec![QvmBranchBinding {
            instruction_index: index,
            decide: Box::new(move |original, _| {
                if live(&state, &actor) {
                    (state.borrow().services.give)(&actor, category);
                }
                original
            }),
        }]);
    }
    let outcome = Rc::new(RefCell::new(None));
    let stashed = Rc::clone(&outcome);
    let state_ref = Rc::clone(state);
    let named = hook.named;
    call.regions(vec![QvmRegionBinding {
        entry: named.entry,
        join: named.join,
        run: Box::new(|_| QvmRegionDecision::Execute),
        completed: Some(Box::new(move |control| {
            let finished = (|| -> Result<(), GuestError> {
                let item = control.local_word(named.item)?;
                if item != 0 {
                    return Ok(());
                }
                if !live(&state_ref, &actor) {
                    return Ok(());
                }
                let name = control.local_word(named.name)?;
                let guest = state_ref.borrow().game.module.memory();
                let give_item = Rc::clone(&state_ref.borrow().services.give_item);
                if give_item(&actor, &guest.read_string(name)?) {
                    control.cancel_function();
                }
                Ok(())
            })();
            if let Err(error) = finished {
                *stashed.borrow_mut() = Some(error);
            }
        })),
    }]);
    let result = call.proceed();
    if let Some(error) = outcome.borrow_mut().take() {
        return Err(error);
    }
    Ok(result)
}

/// Drop interception.
fn drop_call(state: &Rc<RefCell<QvmWeaponState>>, call: &mut QvmFunctionCall) -> Result<i32, GuestError> {
    let (game, profile, services) = {
        let inner = state.borrow();
        (inner.game.clone(), inner.profile.clone(), inner.services.clone())
    };
    let hook = &profile.drop;
    let entity = call.argument(hook.argument)?;
    let actor = (services.actor)(game.data.number_from_pointer(entity)?);
    let Some(actor) = actor else {
        return Ok(call.proceed());
    };
    if !live(state, &actor) {
        return Ok(call.proceed());
    }
    let saved = Rc::new(RefCell::new(Vec::new()));
    let outcome = Rc::new(RefCell::new(None));
    let restore = {
        let state = Rc::clone(state);
        let saved = Rc::clone(&saved);
        let actor = actor.clone();
        Rc::new(RefCell::new(move || {
            let previous: Vec<(usize, i32)> = saved.borrow_mut().drain(..).collect();
            if !live(&state, &actor) {
                return;
            }
            let memory = state.borrow().game.module.memory();
            for (address, value) in previous {
                if memory.write_i32(address, value).is_err() {
                    break;
                }
            }
        }))
    };
    let run_state = Rc::clone(state);
    let run_saved = Rc::clone(&saved);
    let run_outcome = Rc::clone(&outcome);
    let run_actor = actor.clone();
    let region = hook.region;
    call.regions(vec![QvmRegionBinding {
        entry: region.entry,
        join: region.join,
        run: Box::new(move |_| {
            let projected = (|| -> Result<(), GuestError> {
                let drop = Rc::clone(&run_state.borrow().services.drop);
                let projection = drop(&run_actor);
                let Some(projection) = projection else {
                    return Ok(());
                };
                let values = &run_state.borrow().profile.stage.selection.values;
                if projection.weapon != 0 && !values.iter().any(|value| value.value == projection.weapon) {
                    return Err(GuestError::invalid(
                        "Selected death drop has no original weapon or int32 ammo counter",
                    ));
                }
                let hook = run_state.borrow().profile.drop;
                if hook.ammo == QvmDropAmmo::Inventory && projection.ammo_words.is_none() {
                    return Err(GuestError::invalid(
                        "Original drop requires its declared inventory projection",
                    ));
                }
                let memory = run_state.borrow().game.module.memory();
                let base = usize::try_from(entity).map_err(|_| {
                    GuestError::invalid("Selected death drop has no original weapon or int32 ammo counter")
                })?;
                let mut writes = vec![QvmInventoryWord {
                    address: base.checked_add(hook.weapon).ok_or_else(|| {
                        GuestError::invalid("Selected death drop has no original weapon or int32 ammo counter")
                    })?,
                    value: projection.weapon,
                }];
                match hook.ammo {
                    QvmDropAmmo::Inventory => {
                        writes.extend(projection.ammo_words.unwrap_or_default());
                    }
                    QvmDropAmmo::Offset(offset) => {
                        let player = pointer(&run_state, &run_actor)?;
                        let address = player
                            .checked_add(offset)
                            .and_then(|base| {
                                (projection.weapon as i64 * 4)
                                    .try_into()
                                    .ok()
                                    .and_then(|scaled: usize| base.checked_add(scaled))
                            })
                            .ok_or_else(|| {
                                GuestError::invalid("Selected death drop has no original weapon or int32 ammo counter")
                            })?;
                        writes.push(QvmInventoryWord {
                            address,
                            value: projection.ammo,
                        });
                    }
                }
                let mut saved = Vec::with_capacity(writes.len());
                for word in &writes {
                    saved.push((word.address, memory.read_i32(word.address)?));
                }
                for word in &writes {
                    memory.write_i32(word.address, word.value)?;
                }
                *run_saved.borrow_mut() = saved;
                Ok(())
            })();
            if let Err(error) = projected {
                *run_outcome.borrow_mut() = Some(error);
            }
            QvmRegionDecision::Execute
        }),
        completed: Some({
            let restore = Rc::clone(&restore);
            Box::new(move |_| restore.borrow_mut()())
        }),
    }]);
    let result = call.proceed();
    restore.borrow_mut()();
    if let Some(error) = outcome.borrow_mut().take() {
        return Err(error);
    }
    Ok(result)
}

/// Evaluation interception.
fn evaluate_call(
    state: &Rc<RefCell<QvmWeaponState>>,
    kind: QvmEvaluationKind,
    call: &mut QvmFunctionCall,
) -> Result<i32, GuestError> {
    let frame = state.borrow().evaluations.last().cloned();
    let Some(frame) = frame else {
        return Ok(call.proceed());
    };
    if frame.kind != kind && !(kind == QvmEvaluationKind::Teleport && frame.kind == QvmEvaluationKind::Objectives)
        || frame.entered
    {
        return Ok(call.proceed());
    }
    {
        let mut inner = state.borrow_mut();
        if let Some(frame) = inner.evaluations.last_mut() {
            frame.entered = true;
        }
    }
    let (game, profile, slot) = {
        let inner = state.borrow();
        (
            inner.game.clone(),
            inner.profile.clone(),
            (inner.services.slot)(&frame.actor),
        )
    };
    let Some(slot) = slot else {
        return Err(GuestError::invalid("Primary QVM source effect lost its original actor"));
    };
    if !live(state, &frame.actor) || game.data.number_from_pointer(call.argument(0)?)? != slot {
        return Err(GuestError::invalid("Primary QVM source effect lost its original actor"));
    }
    if kind == QvmEvaluationKind::Teleport {
        let region = if frame.kind == QvmEvaluationKind::Objectives {
            &profile.teleport.objectives
        } else {
            &profile.teleport.region
        };
        return Ok(call.evaluate_region(region, &[]));
    }
    let outcome = Rc::new(RefCell::new(None));
    let stashed = Rc::clone(&outcome);
    let state_ref = Rc::clone(state);
    let actor = frame.actor.clone();
    let result = profile.damage_factor.result;
    call.regions(vec![QvmRegionBinding {
        entry: profile.damage_factor.stop.entry,
        join: profile.damage_factor.stop.join,
        run: Box::new(move |control| {
            let memory = state_ref.borrow().game.module.memory();
            match memory.read_f32(result) {
                Ok(value) => {
                    if let Some(frame) = state_ref
                        .borrow_mut()
                        .evaluations
                        .iter_mut()
                        .find(|frame| frame.actor == actor && frame.kind == QvmEvaluationKind::DamageFactor)
                    {
                        frame.result = Some(value);
                    }
                    control.cancel_function();
                }
                Err(error) => {
                    *stashed.borrow_mut() = Some(error);
                }
            }
            QvmRegionDecision::Skip
        }),
        completed: None,
    }]);
    let value = call.proceed();
    if let Some(error) = outcome.borrow_mut().take() {
        return Err(error);
    }
    Ok(value)
}

/// Primary weapons over the original dispatcher.
pub struct QvmPrimaryWeapons {
    /// Shared state.
    state: Rc<RefCell<QvmWeaponState>>,
    /// Weapon dispatcher.
    dispatcher: QvmWeaponDispatcher<QvmWeaponDispatcherOperations>,
    /// Bound hook ids.
    hooks: Vec<u64>,
}

impl QvmPrimaryWeapons {
    /// Bind weapon entries after validating the profile.
    pub fn new(
        game: QvmWeaponGame,
        artifact: QvmArtifact,
        profile: QvmPrimaryWeaponProfile,
        services: QvmWeaponServices,
    ) -> Result<Self, GuestError> {
        let identity = game.module.module_id();
        if identity.id != profile.module.id
            || identity.digest != profile.module.digest
            || identity.revision != profile.module.revision
            || identity.artifact_path != profile.module.artifact_path
            || artifact.module.digest != identity.digest
            || game.module.abi_profile() != profile.abi_profile
        {
            return Err(GuestError::invalid(
                "Primary QVM weapon profile belongs to another executable",
            ));
        }
        let instructions = &artifact.image.instructions;
        if instructions
            .get(profile.torso_animation.entry)
            .map_or(true, |instruction| instruction.opcode != QvmOpcode::OpEnter)
        {
            return Err(GuestError::invalid(
                "Primary QVM torso animation entry is not an original function",
            ));
        }
        if let Some(match_profile) = &profile.match_declaration {
            if match_profile.score % 4 != 0
                || match_profile
                    .score
                    .checked_add(4)
                    .map_or(true, |end| end > profile.client_stride)
            {
                return Err(GuestError::invalid(
                    "Original QVM score exceeds its declared client record",
                ));
            }
        }
        validate_qvm_weapon_dispatcher(&profile.stage, &artifact.image)?;
        qualify_qvm_region(
            instructions,
            profile.damage_factor.entry,
            profile.damage_factor.stop.entry,
            profile.damage_factor.stop.join,
        )?;
        qualify_qvm_region_evaluation(
            instructions,
            profile.stage.dispatcher.entry,
            &profile.delay,
            QvmRegionAccess::Source,
        )?;
        qualify_qvm_region_evaluation(
            instructions,
            profile.teleport.entry,
            &profile.teleport.region,
            QvmRegionAccess::Source,
        )?;
        qualify_qvm_region_evaluation(
            instructions,
            profile.teleport.entry,
            &profile.teleport.objectives,
            QvmRegionAccess::Source,
        )?;
        if [profile.teleport.spawn, profile.teleport.view]
            .into_iter()
            .any(|entry| {
                instructions
                    .get(entry)
                    .map_or(true, |instruction| instruction.opcode != QvmOpcode::OpEnter)
            })
        {
            return Err(GuestError::invalid("Original QVM spawn selector is not a function"));
        }
        let image = &artifact.image;
        let scratch = image.data_length + image.literal_length + image.bss_length;
        let scratch = scratch.div_ceil(16) * 16;
        let room = image.allocated_data_length.checked_sub(65536);
        if room.map_or(true, |room| scratch.checked_add(36).map_or(true, |end| end > room)) {
            return Err(GuestError::invalid(
                "Source player services require scratch outside source data and stack",
            ));
        }
        qualify_qvm_region(
            instructions,
            profile.give.entry,
            profile.give.named.entry,
            profile.give.named.join,
        )?;
        qualify_qvm_region(
            instructions,
            profile.drop.entry,
            profile.drop.region.entry,
            profile.drop.region.join,
        )?;
        for pc in [profile.give.weapons, profile.give.ammo] {
            let decision = instructions.get(pc);
            if pc <= profile.give.entry || decision.map_or(true, |instruction| !instruction.opcode.is_branch()) {
                return Err(GuestError::invalid(
                    "Primary QVM give grant lacks its original completion decision",
                ));
            }
        }
        let state = Rc::new(RefCell::new(QvmWeaponState {
            game: game.clone(),
            profile: profile.clone(),
            services: services.clone(),
            equipment: None,
            evaluations: Vec::new(),
            scratch,
            closed: false,
            error: None,
        }));
        let operations = |state: &Rc<RefCell<QvmWeaponState>>| {
            let actor_state = Rc::clone(state);
            let pointer_state = Rc::clone(state);
            let live_state = Rc::clone(state);
            let selected_state = Rc::clone(state);
            let attempted_state = Rc::clone(state);
            let accepted_state = Rc::clone(state);
            let completed_state = Rc::clone(state);
            let prepare_state = Rc::clone(state);
            QvmWeaponDispatcherOperations {
                module: state.borrow().game.module.clone(),
                actor: Rc::new(move |source, call| dispatch_actor(&actor_state, source, call)),
                pointer: Rc::new(move |actor, record| {
                    if record != "client" {
                        return Err(GuestError::invalid(
                            "Primary QVM weapon field is not a located client record",
                        ));
                    }
                    pointer(&pointer_state, actor)
                }),
                live: Rc::new(move |actor| live(&live_state, actor)),
                selected: Rc::new(move |actor| (selected_state.borrow().services.selected)(actor)),
                cancellation: Rc::new(|_, call| call.cancellation_scope()),
                attempted: Rc::new(move |actor, value| {
                    (attempted_state.borrow().services.attempted)(actor, value);
                }),
                accepted: Rc::new(move |actor, value| {
                    (accepted_state.borrow().services.accepted)(actor, value);
                }),
                completed: Rc::new(move |actor, reached| {
                    (completed_state.borrow().services.completed)(actor, reached);
                }),
                prepare: Some(Rc::new(move |actor, call| {
                    let equipment = prepare_state.borrow().equipment.clone();
                    let Some(equipment) = equipment else {
                        return None;
                    };
                    match equipment.prepare_weapon(actor, call) {
                        Ok(finish) => finish,
                        Err(error) => {
                            record_error(&prepare_state, error);
                            None
                        }
                    }
                })),
            }
        };
        let dispatcher = QvmWeaponDispatcher::new(profile.stage.clone(), operations(&state));
        if let Some(bridge) = services.equipment_movement.clone() {
            let services_ref = services.clone();
            let equipment = QvmEquipmentMovement::new(
                game.module.clone(),
                game.data.clone(),
                &artifact,
                profile.equipment_movement.clone(),
                QvmEquipmentServices {
                    actor: Rc::new(move |slot| (services_ref.actor)(slot)),
                    live: {
                        let state = Rc::clone(&state);
                        Rc::new(move |actor| live(&state, actor))
                    },
                    equipment: bridge,
                },
            )?;
            state.borrow_mut().equipment = Some(Rc::new(equipment));
        }
        let mut hooks = Vec::new();
        let bind = |hooks: &mut Vec<u64>, entry: usize, hook: QvmHookFn| {
            hooks.push(game.module.bind_invocation(entry, hook));
        };
        let given = Rc::clone(&state);
        bind(
            &mut hooks,
            profile.give.entry,
            Rc::new(move |call| run_hook(&given, || give_call(&given, call))),
        );
        let dropped = Rc::clone(&state);
        bind(
            &mut hooks,
            profile.drop.entry,
            Rc::new(move |call| run_hook(&dropped, || drop_call(&dropped, call))),
        );
        for (kind, entry) in [
            (QvmEvaluationKind::DamageFactor, profile.damage_factor.entry),
            (QvmEvaluationKind::Teleport, profile.teleport.entry),
        ] {
            let evaluated = Rc::clone(&state);
            bind(
                &mut hooks,
                entry,
                Rc::new(move |call| run_hook(&evaluated, || evaluate_call(&evaluated, kind, call))),
            );
        }
        Ok(Self {
            state,
            dispatcher,
            hooks,
        })
    }

    /// Match declaration, if any.
    #[must_use]
    pub fn match_profile(&self) -> Option<QvmPrimaryMatch> {
        self.state.borrow().profile.match_declaration
    }

    /// Read an actor score.
    pub fn score(&self, actor: &ActorId) -> Result<i32, GuestError> {
        let offset = self
            .state
            .borrow()
            .profile
            .match_declaration
            .ok_or_else(|| GuestError::invalid("Original QVM score storage has no declaration"))?
            .score;
        let base = pointer(&self.state, actor)?;
        self.state.borrow().game.module.memory().read_i32(base + offset)
    }

    /// Write an actor score.
    pub fn set_score(&self, actor: &ActorId, score: i32) -> Result<(), GuestError> {
        let offset = self
            .state
            .borrow()
            .profile
            .match_declaration
            .ok_or_else(|| GuestError::invalid("Original QVM score storage has no declaration"))?
            .score;
        let base = pointer(&self.state, actor)?;
        self.state.borrow().game.module.memory().write_i32(base + offset, score)
    }

    /// Run movement through equipment, if any.
    pub fn equipment_movement(
        &self,
        call: &mut QvmFunctionCall,
        kind: QvmApplicationScope,
        run: &mut dyn FnMut(&mut QvmFunctionCall) -> Result<i32, GuestError>,
    ) -> Result<i32, GuestError> {
        let equipment = self.state.borrow().equipment.clone();
        let Some(equipment) = equipment else {
            return run(call);
        };
        equipment.movement(call, kind, run)
    }

    /// Whether an actor settled.
    pub fn settled(&self, actor: &ActorId) -> Result<bool, GuestError> {
        self.dispatcher.settled(actor)
    }

    /// Active weapon item, if any.
    pub fn active(&self, actor: &ActorId) -> Result<Option<ItemId>, GuestError> {
        if !live(&self.state, actor) {
            return Err(GuestError::invalid("Primary QVM weapon actor is no longer current"));
        }
        self.dispatcher.active(actor)
    }

    /// Run a source effect for an actor.
    fn effect(&self, actor: &ActorId, kind: QvmEvaluationKind) -> Result<Option<f32>, GuestError> {
        let slot = {
            let inner = self.state.borrow();
            (inner.services.slot)(actor)
        };
        let Some(slot) = slot else {
            return Err(GuestError::invalid(
                "Primary QVM source effect requires its original actor",
            ));
        };
        if !live(&self.state, actor) {
            return Err(GuestError::invalid(
                "Primary QVM source effect requires its original actor",
            ));
        }
        pointer(&self.state, actor)?;
        self.state.borrow_mut().evaluations.push(QvmWeaponEvaluation {
            actor: actor.clone(),
            kind,
            entered: false,
            result: None,
        });
        let (module, entry, address) = {
            let inner = self.state.borrow();
            let located = inner.game.data.checkpoint();
            let entry = if kind == QvmEvaluationKind::DamageFactor {
                inner.profile.damage_factor.entry
            } else {
                inner.profile.teleport.entry
            };
            (
                inner.game.module.clone(),
                entry,
                located.entities_word + slot * located.entity_stride,
            )
        };
        let outcome = module.call(
            &[i32::try_from(address)
                .map_err(|_| GuestError::invalid("Primary QVM source effect lost its original actor"))?],
            entry,
        );
        let result = self.state.borrow().evaluations.last().and_then(|frame| frame.result);
        self.state.borrow_mut().evaluations.pop();
        outcome?;
        Ok(result)
    }

    /// Drop objectives for an actor.
    pub fn drop_objectives(&self, actor: &ActorId) -> Result<(), GuestError> {
        self.effect(actor, QvmEvaluationKind::Objectives).map(|_| ())
    }

    /// Teleport an actor through the source effect.
    pub fn teleport(&self, actor: &ActorId) -> Result<(), GuestError> {
        self.effect(actor, QvmEvaluationKind::Teleport).map(|_| ())
    }

    /// Select a spawn point through scratch vectors.
    pub fn spawn_point(&self, actor: &ActorId) -> Result<QvmSpawnPoint, GuestError> {
        let (game, profile, scratch, slot) = {
            let inner = self.state.borrow();
            let slot = (inner.services.slot)(actor);
            (inner.game.clone(), inner.profile.clone(), inner.scratch, slot)
        };
        let Some(slot) = slot else {
            return Err(GuestError::invalid(
                "Original QVM spawn selection requires its live player",
            ));
        };
        if !live(&self.state, actor) {
            return Err(GuestError::invalid(
                "Original QVM spawn selection requires its live player",
            ));
        }
        pointer(&self.state, actor)?;
        let memory = game.module.memory();
        let saved = memory.read_bytes(scratch, 36)?;
        let origin = game.data.copy_player_state(slot)?.origin;
        memory.write_vec3(scratch, &origin)?;
        let outcome = game.module.call(
            &[
                i32::try_from(scratch).map_err(|_| GuestError::invalid("QVM spawn scratch escapes its allocation"))?,
                i32::try_from(scratch + 12)
                    .map_err(|_| GuestError::invalid("QVM spawn scratch escapes its allocation"))?,
                i32::try_from(scratch + 24)
                    .map_err(|_| GuestError::invalid("QVM spawn scratch escapes its allocation"))?,
            ],
            profile.teleport.spawn,
        );
        let live_now = live(&self.state, actor);
        let point = (|| -> Result<QvmSpawnPoint, GuestError> {
            outcome?;
            if !live_now {
                return Err(GuestError::invalid("Original QVM spawn selection retired its player"));
            }
            Ok(QvmSpawnPoint {
                origin: memory.read_vec3(scratch + 12)?,
                angles: memory.read_vec3(scratch + 24)?,
            })
        })();
        memory.write_bytes(scratch, &saved)?;
        point
    }

    /// Write teleport state and run the source view hook.
    pub fn teleport_state(
        &self,
        actor: &ActorId,
        origin: &Vec3,
        velocity: &Vec3,
        angles: &Vec3,
        hold_ms: i32,
    ) -> Result<(), GuestError> {
        let (game, profile, scratch, slot) = {
            let inner = self.state.borrow();
            let slot = (inner.services.slot)(actor);
            (inner.game.clone(), inner.profile.clone(), inner.scratch, slot)
        };
        let Some(slot) = slot else {
            return Err(GuestError::invalid("Original QVM teleport requires its live player"));
        };
        if !live(&self.state, actor) {
            return Err(GuestError::invalid("Original QVM teleport requires its live player"));
        }
        pointer(&self.state, actor)?;
        let mut state = game.data.copy_player_state(slot)?;
        state.origin = origin.clone();
        state.velocity = velocity.clone();
        state.ground_entity_number = 1023;
        state.flags ^= 4;
        state.movement_flags |= 64;
        state.movement_time_ms = hold_ms;
        game.data.write_player_state(slot, &state)?;
        let memory = game.module.memory();
        let saved = memory.read_bytes(scratch, 12)?;
        memory.write_vec3(scratch, angles)?;
        let located = game.data.checkpoint();
        let outcome = game.module.call(
            &[
                i32::try_from(located.entities_word + slot * located.entity_stride)
                    .map_err(|_| GuestError::invalid("Original QVM teleport requires its live player"))?,
                i32::try_from(scratch)
                    .map_err(|_| GuestError::invalid("Original QVM teleport requires its live player"))?,
            ],
            profile.teleport.view,
        );
        memory.write_bytes(scratch, &saved)?;
        outcome?;
        Ok(())
    }

    /// Read the source damage factor.
    pub fn damage_factor(&self, actor: &ActorId) -> Result<f32, GuestError> {
        let (memory, result) = {
            let inner = self.state.borrow();
            (inner.game.module.memory(), inner.profile.damage_factor.result)
        };
        let previous = memory.read_i32(result)?;
        let outcome = self.effect(actor, QvmEvaluationKind::DamageFactor);
        memory.write_i32(result, previous)?;
        let value = outcome?
            .ok_or_else(|| GuestError::invalid("Original QVM damage factor did not produce a finite result"))?;
        if !value.is_finite() || value < 0.0 {
            return Err(GuestError::invalid(
                "Original QVM damage factor did not produce a finite result",
            ));
        }
        Ok(value)
    }

    /// Resolve an equipment cadence item.
    pub fn equipment_context(&self, provider: &str) -> Result<Option<ItemId>, GuestError> {
        let profile = self.state.borrow().profile.clone();
        let item = qvm_equipment_item(&profile.equipment_contexts, provider)?;
        if item.is_some() && profile.stage.selection.field.record != "client" {
            return Err(GuestError::invalid(format!(
                "Equipment {provider} requires an original client selection field"
            )));
        }
        if let Some(item) = &item {
            if !profile.stage.selection.values.iter().any(|value| &value.item == item) {
                return Err(GuestError::invalid(format!(
                    "Equipment {provider} names an unavailable original cadence item {item}"
                )));
            }
        }
        Ok(item)
    }

    /// Evaluate weapon delay, projecting an equipment selection first.
    pub fn equipment_delay(&self, actor: &ActorId, provider: &str, milliseconds: i32) -> Result<i32, GuestError> {
        let item = self.equipment_context(provider)?;
        let Some(item) = item else {
            return self.weapon_delay(actor, milliseconds);
        };
        let profile = self.state.borrow().profile.clone();
        let value = profile.stage.selection.values.iter().find(|value| value.item == item);
        let Some(value) = value else {
            return Err(GuestError::invalid(
                "Equipment cadence has no original client selection field",
            ));
        };
        if profile.stage.selection.field.record != "client" {
            return Err(GuestError::invalid(
                "Equipment cadence has no original client selection field",
            ));
        }
        let base = pointer(&self.state, actor)?;
        let memory = self.state.borrow().game.module.memory();
        let address = base + profile.stage.selection.field.offset;
        let previous = memory.read_i32(address)?;
        memory.write_i32(address, value.value as i32)?;
        let outcome = self.weapon_delay(actor, milliseconds);
        if live(&self.state, actor) && pointer(&self.state, actor).is_ok_and(|current| current == base) {
            memory.write_i32(address, previous)?;
        }
        outcome
    }

    /// Evaluate weapon delay under a projected Pmove player.
    pub fn weapon_delay(&self, actor: &ActorId, milliseconds: i32) -> Result<i32, GuestError> {
        if milliseconds < 0 {
            return Err(GuestError::invalid(
                "Original QVM weapon delay requires its int32 input",
            ));
        }
        let (game, profile) = {
            let inner = self.state.borrow();
            (inner.game.clone(), inner.profile.clone())
        };
        let memory = game.module.memory();
        let movement = memory.read_i32(profile.delay_player.movement_global)?;
        if movement <= 0 {
            return Err(GuestError::invalid(
                "Original QVM delay has no established Pmove context",
            ));
        }
        let base = usize::try_from(movement)
            .map_err(|_| GuestError::invalid("Original QVM delay has no established Pmove context"))?;
        let player = base + profile.delay_player.player_offset;
        let previous = memory.read_i32(player)?;
        memory.write_i32(player, pointer(&self.state, actor)? as i32)?;
        let outcome = self.dispatcher.evaluate(actor, &profile.delay, &[milliseconds]);
        memory.write_i32(player, previous)?;
        outcome
    }

    /// Run the torso-attack animation under a projected Pmove player.
    pub fn attack_animation(&self, actor: &ActorId, melee: bool) -> Result<(), GuestError> {
        let (game, profile) = {
            let inner = self.state.borrow();
            (inner.game.clone(), inner.profile.clone())
        };
        let memory = game.module.memory();
        let movement = memory.read_i32(profile.delay_player.movement_global)?;
        if movement <= 0 {
            return Err(GuestError::invalid(
                "Original QVM animation has no established Pmove context",
            ));
        }
        let base = usize::try_from(movement)
            .map_err(|_| GuestError::invalid("Original QVM animation has no established Pmove context"))?;
        let player = base + profile.delay_player.player_offset;
        let previous = memory.read_i32(player)?;
        memory.write_i32(player, pointer(&self.state, actor)? as i32)?;
        let outcome = game.module.call(
            &[if melee {
                profile.torso_animation.melee
            } else {
                profile.torso_animation.attack
            }],
            profile.torso_animation.entry,
        );
        memory.write_i32(player, previous)?;
        outcome?;
        Ok(())
    }

    /// Read the water level from movement state or the entity record.
    pub fn water_level(&self, actor: &ActorId) -> Result<i32, GuestError> {
        let (game, profile, slot) = {
            let inner = self.state.borrow();
            let slot = (inner.services.slot)(actor);
            (inner.game.clone(), inner.profile.clone(), slot)
        };
        let Some(slot) = slot else {
            return Err(GuestError::invalid("Original QVM water state lost its actor"));
        };
        if !live(&self.state, actor) {
            return Err(GuestError::invalid("Original QVM water state lost its actor"));
        }
        let memory = game.module.memory();
        let movement = memory.read_i32(profile.delay_player.movement_global)?;
        if movement > 0 {
            let base = usize::try_from(movement)
                .map_err(|_| GuestError::invalid("Original QVM water state lost its actor"))?;
            if memory.read_i32(base + profile.delay_player.player_offset)? == pointer(&self.state, actor)? as i32 {
                return memory.read_i32(base + profile.water_level.movement_offset);
            }
        }
        game.data.entity_bytes(slot)?.get_i32(profile.water_level.entity_offset)
    }

    /// Read maximum health.
    pub fn max_health(&self, actor: &ActorId) -> Result<i32, GuestError> {
        let base = pointer(&self.state, actor)?;
        let inner = self.state.borrow();
        inner.game.module.memory().read_i32(base + inner.profile.max_health)
    }

    /// Write both maximum-health words.
    pub fn set_max_health(&self, actor: &ActorId, health: i32) -> Result<(), GuestError> {
        if health <= 0 {
            return Err(GuestError::invalid(
                "Original QVM maximum health must be a positive int32",
            ));
        }
        let base = pointer(&self.state, actor)?;
        let inner = self.state.borrow();
        for offset in [inner.profile.max_health, inner.profile.persistent_max_health] {
            inner.game.module.memory().write_i32(base + offset, health)?;
        }
        Ok(())
    }

    /// Whether an actor can fight (and attack, when requested).
    pub fn available(&self, actor: &ActorId, attacking: bool) -> Result<bool, GuestError> {
        let base = pointer(&self.state, actor)?;
        let inner = self.state.borrow();
        let policy = &inner.profile.availability;
        let memory = inner.game.module.memory();
        let word = |offset: usize| memory.read_i32(base + offset);
        Ok(word(policy.health)? > 0
            && word(policy.team)? != policy.spectator_team
            && !policy.excluded.contains(&word(policy.movement_type)?)
            && (!attacking || word(policy.flags)? & policy.respawn_flag == 0))
    }

    /// Read a powerup expiry.
    pub fn powerup_until(&self, actor: &ActorId, powerup: QvmPowerup) -> Result<i32, GuestError> {
        let base = pointer(&self.state, actor)?;
        let inner = self.state.borrow();
        let offset = match powerup {
            QvmPowerup::Quad => inner.profile.powerups.quad,
            QvmPowerup::Haste => inner.profile.powerups.haste,
            QvmPowerup::Flight => inner.profile.powerups.flight,
        };
        inner.game.module.memory().read_i32(base + offset)
    }

    /// Take the first recorded failure, if any.
    pub fn take_error(&self) -> Option<GuestError> {
        self.state.borrow_mut().error.take()
    }

    /// Close hooks, equipment, and the dispatcher.
    pub fn close(&mut self) {
        self.state.borrow_mut().closed = true;
        let module = self.state.borrow().game.module.clone();
        for id in self.hooks.drain(..).rev() {
            module.remove_hook(id);
        }
        if let Some(equipment) = self.state.borrow_mut().equipment.take() {
            equipment.close();
        }
        self.dispatcher.close();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use qa_core::identity::IdentityOwner;

    use super::super::game_data::{ModuleIdentity, QvmArtifact, QvmImage, QvmInstruction, QvmOpcode, QvmRole};
    use super::*;

    #[test]
    fn equipment_item_requires_exactly_one_context() {
        let contexts = vec![
            QvmEquipmentContext {
                provider: "test:grapple".to_string(),
                item: Some("q3:grapple".to_string()),
            },
            QvmEquipmentContext {
                provider: "test:other".to_string(),
                item: None,
            },
        ];
        assert_eq!(
            qvm_equipment_item(&contexts, "test:grapple").unwrap(),
            Some("q3:grapple".to_string())
        );
        assert_eq!(qvm_equipment_item(&contexts, "test:other").unwrap(), None);
        assert!(qvm_equipment_item(&contexts, "test:missing").is_err());
        let mut duplicated = contexts;
        duplicated.push(QvmEquipmentContext {
            provider: "test:grapple".to_string(),
            item: None,
        });
        assert!(qvm_equipment_item(&duplicated, "test:grapple").is_err());
    }
}
