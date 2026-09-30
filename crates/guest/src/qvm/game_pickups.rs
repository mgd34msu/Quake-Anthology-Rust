//! Original pickups: Touch_Item interception with qualified grants.
//!
//! Provenance: `src/compat/qvm/game-pickups.ts`.
//!
//! The hub runs hooks synchronously and never executes registered regions, so
//! async branches collapse to the sync path, cancellations surface as recorded
//! hook errors filtered by frame identity, and the grant-region body is tested
//! white-box through the call's recorded bindings.
//!
//! Local mirrors (used surface of `src/contracts/original-pickups.ts` and
//! `src/contracts/pickups.ts`): [`QvmPickupResource`], [`QvmPickupCount`],
//! [`QvmPickupCargoEntry`], [`QvmOriginalPickupOffer`],
//! [`QvmPickupOutcome`], [`QvmPickupSelection`], [`QvmAmmoGrant`],
//! [`QvmPickupSupplyOffer`]. Times reuse [`super::game_input::QvmSourceTime`],
//! protection channels reuse
//! [`super::game_combat_binding::ProtectionChannel`], catalog records reuse
//! [`super::item_catalog`], and projections reuse
//! [`super::game_inventory::QvmInventoryWord`]. The offer `source` keeps the
//! raw `namespace:name` string the donor stores (donor `ProviderId` is a
//! string, not the structured [`qa_core::identity::ProviderId`]).
//!
//! [`QvmGameData`]: super::game_data::QvmGameData

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::{ActorId, OwnedActor};
use qa_world::combat::ItemId;

use super::game_combat_binding::ProtectionChannel;
use super::game_data::{
    qualify_qvm_region, qualify_qvm_region_evaluation, AbiProfile, ModuleIdentity, QvmArtifact, QvmCancellationScope,
    QvmFunctionCall, QvmGameData, QvmHookFn, QvmMemoryWindow, QvmModule, QvmOpcode, QvmRegionAccess, QvmRegionBinding,
    QvmRegionDecision, QvmRegionEvaluation, QvmWriteRange,
};
use super::game_input::QvmSourceTime;
use super::game_inventory::QvmInventoryWord;
use super::item_catalog::{read_qvm_item_records, QvmCatalogRecord, QvmItemLayout};
use crate::error::GuestError;

/// Default pickup resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmPickupResource {
    /// Protection channel.
    Protection {
        /// Channel.
        channel: ProtectionChannel,
    },
    /// Inventory item.
    Inventory {
        /// Item id.
        item: ItemId,
    },
}

/// Pickup count: default or override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmPickupCount {
    /// Default count.
    Default,
    /// Override amount.
    Override {
        /// Amount.
        amount: i32,
    },
}

/// Cargo entry kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmPickupCargoKind {
    /// Counter cargo.
    Counter,
    /// Weapon cargo.
    Weapon,
}

/// Pickup cargo entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmPickupCargoEntry {
    /// Cargo kind.
    pub kind: QvmPickupCargoKind,
    /// Item id.
    pub item: ItemId,
    /// Count.
    pub count: i32,
}

/// Grant coupling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmPickupGrantKind {
    /// Map-coupled grant (owns objectives).
    MapCoupled,
    /// Source effect without inventory/protection grant.
    SourceEffect,
}

/// Original pickup offer.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmOriginalPickupOffer {
    /// Recipient actor.
    pub recipient: ActorId,
    /// Pickup actor.
    pub pickup: ActorId,
    /// Source module id (`namespace:name`).
    pub source: String,
    /// Offered item.
    pub item: ItemId,
    /// Default resource, if any.
    pub default_resource: Option<QvmPickupResource>,
    /// Pickup count.
    pub count: QvmPickupCount,
    /// Whether the item was dropped.
    pub dropped: bool,
    /// Offer time.
    pub time: QvmSourceTime,
    /// Cargo entries, if any.
    pub cargo: Option<Vec<QvmPickupCargoEntry>>,
    /// Grant coupling, if any.
    pub grant: Option<QvmPickupGrantKind>,
}

/// Pickup outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmPickupOutcome {
    /// Accepted.
    Accepted,
    /// Refused.
    Refused,
    /// Stale.
    Stale,
}

/// Source pickup selection.
#[derive(Clone)]
pub enum QvmPickupSelection {
    /// Run the original source.
    Original,
    /// Blocked.
    Blocked,
    /// Stale.
    Stale,
    /// Replacement grant.
    Replacement {
        /// Whether the replacement is current.
        current: Rc<dyn Fn() -> bool>,
        /// Replacement grant outcome.
        grant: Rc<dyn Fn() -> QvmPickupOutcome>,
    },
}

/// Ammo grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmAmmoGrant {
    /// Item id.
    pub item: ItemId,
    /// Amount.
    pub amount: i32,
}

/// Supply offer (ammo and weapon arms).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmPickupSupplyOffer {
    /// Ammo grant.
    Ammo {
        /// Grant.
        offer: QvmAmmoGrant,
    },
    /// Weapon grant with ammo.
    Weapon {
        /// Weapon item.
        item: ItemId,
        /// Ammo grants.
        ammo: Vec<QvmAmmoGrant>,
    },
}

/// Supply quantity callback.
pub type QvmSupplyQuantity = Rc<dyn Fn(f64) -> Result<i32, GuestError>>;

/// Held supply with optional quantity callback.
#[derive(Clone)]
pub struct QvmPickupSupply {
    /// Supply offer.
    pub offer: QvmPickupSupplyOffer,
    /// Quantity callback, if any.
    pub quantity: Option<QvmSupplyQuantity>,
}

/// Pickup eligibility context.
pub struct QvmPickupEligibility<'a> {
    /// Gate call.
    pub call: &'a mut QvmFunctionCall,
    /// Item entity record.
    pub item: QvmMemoryWindow,
    /// Player record.
    pub player: QvmMemoryWindow,
}

/// Grant eligibility callback.
pub type QvmGrantEligibility = Rc<dyn for<'a> Fn(QvmPickupEligibility<'a>) -> Result<bool, GuestError>>;

/// Grant weapon storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmGrantWeaponStorage {
    /// Canonical inventory storage.
    Inventory,
    /// Player-state words.
    Words {
        /// Ownership-bits offset.
        bits_offset: usize,
        /// Ammo-counters offset.
        ammo_offset: usize,
    },
}

/// Grant weapon quantity evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmGrantWeapon {
    /// Quantity evaluation.
    pub quantity: QvmRegionEvaluation,
    /// Weapon storage.
    pub storage: QvmGrantWeaponStorage,
}

/// Grant operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmGrantOperation {
    /// Lifecycle return constant.
    Return {
        /// Accepted-return instruction.
        accepted_return: usize,
    },
    /// Qualified region.
    Region {
        /// Region entry.
        entry: usize,
        /// Region join.
        join: usize,
        /// Quantity local offset.
        quantity: usize,
        /// Weapon evaluation, if any.
        weapon: Option<QvmGrantWeapon>,
    },
}

/// Qualified pickup grant.
#[derive(Clone)]
pub struct QvmPickupGrant {
    /// Item type.
    pub item_type: i32,
    /// Grant entry.
    pub entry: usize,
    /// Qualified call sites.
    pub calls: Vec<usize>,
    /// Grant operation.
    pub operation: QvmGrantOperation,
    /// Eligibility callback.
    pub eligible: QvmGrantEligibility,
}

/// Pickup gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmPickupGate {
    /// Gate entry.
    pub entry: usize,
    /// Qualified call sites.
    pub calls: Vec<usize>,
    /// Item argument word.
    pub item_argument: usize,
    /// Player argument word.
    pub player_argument: usize,
}

/// Pickup targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmPickupTargets {
    /// Targets entry.
    pub entry: usize,
    /// Qualified call sites.
    pub calls: Vec<usize>,
}

/// Entity record fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmPickupFields {
    /// In-use offset.
    pub inuse: usize,
    /// Client offset.
    pub client: usize,
    /// Health offset.
    pub health: usize,
    /// Item offset.
    pub item: usize,
    /// Count offset.
    pub count: usize,
    /// Flags offset.
    pub flags: usize,
}

/// Original pickup profile.
#[derive(Clone)]
pub struct QvmPickupProfile {
    /// Owning module.
    pub module: ModuleIdentity,
    /// ABI profile.
    pub abi_profile: AbiProfile,
    /// Entity stride in bytes.
    pub entity_stride: usize,
    /// Client stride in bytes.
    pub client_stride: usize,
    /// Entity record fields.
    pub fields: QvmPickupFields,
    /// Dropped flag.
    pub dropped_flag: i32,
    /// Item table layout.
    pub items: QvmItemLayout,
    /// Touch entry.
    pub touch: usize,
    /// Eligibility gate.
    pub gate: QvmPickupGate,
    /// Targets entry.
    pub targets: QvmPickupTargets,
    /// Free entry.
    pub free: usize,
    /// Map-coupled item types.
    pub objective_types: Vec<i32>,
    /// Qualified grants.
    pub grants: Vec<QvmPickupGrant>,
}

/// Resolved catalog item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmResolvedItem {
    /// Item id.
    pub item: ItemId,
    /// Default resource, if any.
    pub resource: Option<QvmPickupResource>,
}

/// Source game handles.
#[derive(Clone)]
pub struct QvmPickupGame {
    /// Source module.
    pub module: QvmModule,
    /// Located game data.
    pub data: QvmGameData,
}

/// Live catalog records.
pub type QvmPickupCatalog = Rc<dyn Fn() -> Vec<QvmCatalogRecord>>;

/// Supply bridge.
#[derive(Clone)]
pub struct QvmPickupSupplyBridge {
    /// Ammo item for a record.
    pub ammo: Rc<dyn Fn(&QvmCatalogRecord) -> Option<ItemId>>,
    /// Whether an actor owns an item.
    pub owns: Rc<dyn Fn(&OwnedActor, &ItemId) -> bool>,
    /// Project inventory words.
    pub project: Option<Rc<dyn Fn(&OwnedActor, &ItemId, i32) -> Result<Vec<QvmInventoryWord>, GuestError>>>,
}

/// Pickup lifetime.
#[derive(Clone)]
pub enum QvmPickupLifetime {
    /// Host owns the free hook and calls [`QvmPrimaryPickups::after_free`].
    SharedFreeHook,
    /// Binding owns the free hook.
    OwnFreeHook {
        /// Retire a freed actor.
        retire: Rc<dyn Fn(OwnedActor)>,
    },
}

/// Pickup options.
pub struct QvmPickupOptions {
    /// Source game handles.
    pub game: QvmPickupGame,
    /// Source artifact.
    pub artifact: QvmArtifact,
    /// Pickup profile.
    pub profile: QvmPickupProfile,
    /// Live catalog, if any.
    pub catalog: Option<QvmPickupCatalog>,
    /// Canonical actor for a slot.
    pub actor: Rc<dyn Fn(usize) -> Option<OwnedActor>>,
    /// Whether an actor generation is current.
    pub current: Rc<dyn Fn(&OwnedActor, usize) -> bool>,
    /// Resolve a catalog record.
    pub resolve_item: Rc<dyn Fn(&QvmCatalogRecord) -> QvmResolvedItem>,
    /// Current source time.
    pub time: Rc<dyn Fn() -> QvmSourceTime>,
    /// Supply bridge, if any.
    pub supply: Option<QvmPickupSupplyBridge>,
    /// Run the source offer under the item lock.
    pub run_source: Rc<
        dyn Fn(
            &QvmOriginalPickupOffer,
            &mut dyn FnMut(QvmPickupSelection) -> Result<i32, GuestError>,
        ) -> Result<i32, GuestError>,
    >,
    /// Pickup lifetime.
    pub lifetime: QvmPickupLifetime,
}

/// Held frame supply.
#[derive(Clone)]
struct QvmFrameSupply {
    /// Supply offer.
    offer: QvmPickupSupplyOffer,
    /// Quantity callback, if any.
    quantity: Option<QvmSupplyQuantity>,
}

/// Live pickup frame.
#[derive(Clone)]
struct QvmPickupFrame {
    /// Frame ordinal.
    id: u64,
    /// Source offer.
    offer: QvmOriginalPickupOffer,
    /// Held supply, if any.
    supply: Option<QvmFrameSupply>,
    /// Cancellation token.
    cancellation: QvmCancellationScope,
    /// Item actor.
    item: OwnedActor,
    /// Recipient actor.
    recipient: OwnedActor,
    /// Item slot.
    item_slot: usize,
    /// Recipient slot.
    recipient_slot: usize,
    /// Item pointer word.
    item_pointer: i32,
    /// Player pointer word.
    player_pointer: i32,
    /// Recipient pointer word.
    recipient_pointer: i32,
    /// Item catalog record.
    item_record: QvmCatalogRecord,
    /// Grant index, if any.
    grant: Option<usize>,
    /// Active selection.
    selection: QvmPickupSelection,
    /// Whether a watched word freed a party.
    invalid: bool,
    /// Whether the source granted.
    granted: bool,
    /// Cancellation error, if any.
    cancelled: Option<GuestError>,
}

/// Pending quantity evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct QvmEvaluating {
    /// Frame ordinal.
    frame_id: u64,
    /// Evaluation region.
    region: QvmRegionEvaluation,
    /// Whether the grant entry ran.
    entered: bool,
}

/// Shared pickup state behind hook closures.
struct QvmPickupState {
    /// Pickup options.
    options: Rc<QvmPickupOptions>,
    /// Source module.
    module: QvmModule,
    /// Bound hook ids.
    hooks: Vec<u64>,
    /// Live frames, innermost last.
    frames: Vec<QvmPickupFrame>,
    /// Pending evaluations.
    evaluating: Vec<QvmEvaluating>,
    /// Snapshot catalog records.
    items: Option<Vec<QvmCatalogRecord>>,
    /// Lifecycle returns by grant index.
    returns: HashMap<usize, i32>,
    /// Return PCs by call-site instruction.
    return_pcs: HashMap<usize, usize>,
    /// Next frame ordinal.
    next_frame: u64,
    /// Whether closed.
    closed: bool,
    /// First recorded hook failure.
    error: Option<GuestError>,
}

/// Word as an allocation address.
fn address(word: i32) -> Result<usize, GuestError> {
    usize::try_from(word).map_err(|_| GuestError::invalid("QVM pickup pointer escapes its allocation"))
}

/// Open an entity record after stride validation.
fn entity(state: &Rc<RefCell<QvmPickupState>>, slot: usize) -> Result<QvmMemoryWindow, GuestError> {
    let options = Rc::clone(&state.borrow().options);
    if options.game.data.entity_stride_bytes() != options.profile.entity_stride
        || options.game.data.client_stride_bytes() != options.profile.client_stride
    {
        return Err(GuestError::invalid(
            "Original pickup source records differ from their profile",
        ));
    }
    options.game.data.entity_bytes(slot)
}

/// Whether a call entered through qualified call sites.
fn called_from(
    state: &Rc<RefCell<QvmPickupState>>,
    call: &QvmFunctionCall,
    sites: &[usize],
) -> Result<bool, GuestError> {
    // QVM OP_CALL writes its return byte PC eight bytes before the live argument words.
    let Some(offset) = call.stack_address.checked_sub(8) else {
        return Ok(false);
    };
    let pc = call.guest.read_i32(offset)?;
    let inner = state.borrow();
    Ok(sites.iter().any(|site| {
        inner
            .return_pcs
            .get(site)
            .is_some_and(|expected| i64::try_from(*expected).is_ok_and(|expected| expected == i64::from(pc)))
    }))
}

/// Whether a frame is still live.
fn live(state: &Rc<RefCell<QvmPickupState>>, frame: &QvmPickupFrame) -> Result<bool, GuestError> {
    let options = Rc::clone(&state.borrow().options);
    if state.borrow().closed
        || frame.invalid
        || !(options.current)(&frame.item, frame.item_slot)
        || !(options.current)(&frame.recipient, frame.recipient_slot)
    {
        return Ok(false);
    }
    let fields = &options.profile.fields;
    let item = entity(state, frame.item_slot)?;
    let recipient = entity(state, frame.recipient_slot)?;
    if item.get_i32(fields.inuse)? == 0
        || recipient.get_i32(fields.inuse)? == 0
        || i64::from(item.get_i32(fields.item)?) != frame.item_record.address as i64
        || usize::try_from(item.get_i32(160)?).unwrap_or(usize::MAX) != frame.item_record.index
        || recipient.get_i32(fields.client)? != frame.player_pointer
    {
        return Ok(false);
    }
    if let QvmPickupSelection::Replacement { current, .. } = &frame.selection {
        return Ok(current());
    }
    Ok(true)
}

/// Cancel a frame through a live call.
fn cancel_frame(state: &Rc<RefCell<QvmPickupState>>, call: &mut QvmFunctionCall, id: u64) -> Result<i32, GuestError> {
    let error = GuestError::invalid(format!("QVM pickup frame {id} cancelled"));
    if let Some(frame) = state.borrow_mut().frames.iter_mut().find(|frame| frame.id == id) {
        frame.cancelled = Some(error.clone());
    }
    call.cancel_function();
    Err(error)
}

/// Whether an error is a recorded frame cancellation.
fn is_cancellation(state: &Rc<RefCell<QvmPickupState>>, error: &GuestError) -> bool {
    state
        .borrow()
        .frames
        .iter()
        .any(|frame| frame.cancelled.as_ref() == Some(error))
}

/// Run a hook closure, recording the first non-cancellation failure.
fn run_hook(state: &Rc<RefCell<QvmPickupState>>, run: impl FnOnce() -> Result<i32, GuestError>) -> i32 {
    match run() {
        Ok(result) => result,
        Err(error) => {
            if is_cancellation(state, &error) {
                return 0;
            }
            let mut inner = state.borrow_mut();
            if inner.error.is_none() {
                inner.error = Some(error);
            }
            0
        }
    }
}

/// Apply projected words; returns a one-shot restore.
fn project(
    state: &Rc<RefCell<QvmPickupState>>,
    writes: &[QvmInventoryWord],
) -> Result<impl FnMut() -> Result<(), GuestError>, GuestError> {
    let memory = state.borrow().options.game.module.memory();
    let mut before = Vec::with_capacity(writes.len());
    for word in writes {
        before.push((word.address, memory.read_i32(word.address)?));
    }
    for word in writes {
        memory.write_i32(word.address, word.value)?;
    }
    let mut active = true;
    Ok(move || {
        if !active {
            return Ok(());
        }
        active = false;
        for (address, value) in &before {
            memory.write_i32(*address, *value)?;
        }
        Ok(())
    })
}

/// Touch_Item interception.
fn touch(state: &Rc<RefCell<QvmPickupState>>, call: &mut QvmFunctionCall) -> Result<i32, GuestError> {
    let options = Rc::clone(&state.borrow().options);
    if state.borrow().closed {
        return Ok(call.proceed());
    }
    let profile = &options.profile;
    let item_pointer = call.argument(0)?;
    let recipient_pointer = call.argument(1)?;
    let item_slot = options.game.data.number_from_pointer(item_pointer)?;
    let recipient_slot = options.game.data.number_from_pointer(recipient_pointer)?;
    let recipient = entity(state, recipient_slot)?;
    let player_pointer = recipient.get_i32(profile.fields.client)?;
    if player_pointer == 0 || recipient.get_i32(profile.fields.health)? < 1 {
        return Ok(call.proceed());
    }
    let item_word = entity(state, item_slot)?.get_i32(profile.fields.item)?;
    let records: Vec<QvmCatalogRecord> = options
        .catalog
        .as_ref()
        .map(|catalog| catalog())
        .or_else(|| state.borrow().items.clone())
        .unwrap_or_default();
    let item_record = records
        .iter()
        .find(|record| usize::try_from(item_word).is_ok_and(|word| word == record.address))
        .cloned();
    let Some(item_record) = item_record else {
        return Err(GuestError::invalid(
            "Original pickup has an undeclared source item descriptor",
        ));
    };
    if usize::try_from(entity(state, item_slot)?.get_i32(160)?).unwrap_or(usize::MAX) != item_record.index {
        return Err(GuestError::invalid(
            "Original pickup has an undeclared source item descriptor",
        ));
    }
    let item = (options.actor)(item_slot);
    let actor = (options.actor)(recipient_slot);
    let (Some(item), Some(actor)) = (item, actor) else {
        return Err(GuestError::invalid(
            "Original pickup requires live canonical source actors",
        ));
    };
    let resolved = (options.resolve_item)(&item_record);
    let count = entity(state, item_slot)?.get_i32(profile.fields.count)?;
    let offer = QvmOriginalPickupOffer {
        recipient: actor.id().clone(),
        pickup: item.id().clone(),
        source: profile.module.id.clone(),
        item: resolved.item,
        default_resource: resolved.resource,
        count: if count == 0 {
            QvmPickupCount::Default
        } else {
            QvmPickupCount::Override { amount: count }
        },
        dropped: entity(state, item_slot)?.get_i32(profile.fields.flags)? & profile.dropped_flag != 0,
        time: (options.time)(),
        cargo: None,
        grant: if profile.objective_types.contains(&item_record.item_type) {
            Some(QvmPickupGrantKind::MapCoupled)
        } else {
            None
        },
    };
    let cancellation = call.cancellation_scope();
    let state = Rc::clone(state);
    (options.run_source)(&offer, &mut |selection| {
        if matches!(selection, QvmPickupSelection::Blocked | QvmPickupSelection::Stale) {
            return Ok(0);
        }
        let options = Rc::clone(&state.borrow().options);
        let grant = options
            .profile
            .grants
            .iter()
            .position(|grant| grant.item_type == item_record.item_type);
        if matches!(selection, QvmPickupSelection::Replacement { .. }) && grant.is_none() {
            return Err(GuestError::invalid(
                "Original pickup replacement has no qualified grant boundary",
            ));
        }
        let id = {
            let mut inner = state.borrow_mut();
            let id = inner.next_frame;
            inner.next_frame += 1;
            inner.frames.push(QvmPickupFrame {
                id,
                offer: offer.clone(),
                supply: None,
                cancellation,
                item: item.clone(),
                recipient: actor.clone(),
                item_slot,
                recipient_slot,
                item_pointer,
                player_pointer,
                recipient_pointer,
                item_record: item_record.clone(),
                grant,
                selection,
                invalid: false,
                granted: false,
                cancelled: None,
            });
            id
        };
        let watch = observe_frame(&state, id)?;
        let result = call.proceed();
        finish_frame(&state, id, watch);
        Ok(result)
    })
}

/// Watch a frame's party in-use words; returns the watch id.
fn observe_frame(state: &Rc<RefCell<QvmPickupState>>, id: u64) -> Result<u64, GuestError> {
    let (memory, ranges) = {
        let inner = state.borrow();
        let options = Rc::clone(&inner.options);
        let Some(frame) = inner.frames.iter().find(|frame| frame.id == id).cloned() else {
            return Err(GuestError::invalid("Original pickup frame expired"));
        };
        let inuse = options.profile.fields.inuse;
        let memory = options.game.module.memory();
        let mut ranges = Vec::new();
        for pointer in [frame.item_pointer, frame.recipient_pointer] {
            let base = address(pointer)?;
            let offset = base
                .checked_add(inuse)
                .ok_or_else(|| GuestError::invalid("QVM pickup watch escapes its allocation"))?;
            ranges.push(QvmWriteRange {
                byte_offset: offset,
                byte_length: 4,
            });
        }
        (memory, ranges)
    };
    let watched = Rc::clone(state);
    let watch = memory.observe_writes(
        ranges,
        Rc::new(move |_| {
            let inner = watched.borrow();
            let options = Rc::clone(&inner.options);
            let Some(frame) = inner.frames.iter().find(|frame| frame.id == id).cloned() else {
                return;
            };
            let inuse = options.profile.fields.inuse;
            let freed = [frame.item_slot, frame.recipient_slot].into_iter().any(|slot| {
                options
                    .game
                    .data
                    .entity_bytes(slot)
                    .and_then(|record| record.get_i32(inuse))
                    .is_ok_and(|value| value == 0)
            });
            drop(inner);
            if freed {
                if let Some(frame) = watched.borrow_mut().frames.iter_mut().find(|frame| frame.id == id) {
                    frame.invalid = true;
                }
            }
        }),
        None,
    );
    Ok(watch)
}

/// Finish a frame: stop its watch, remove it, dispose when closed and idle.
fn finish_frame(state: &Rc<RefCell<QvmPickupState>>, id: u64, watch: u64) {
    let memory = state.borrow().options.game.module.memory();
    memory.remove_observer(watch);
    let mut inner = state.borrow_mut();
    if let Some(index) = inner.frames.iter().position(|frame| frame.id == id) {
        inner.frames.remove(index);
    }
    if inner.closed && inner.frames.is_empty() {
        let hooks = std::mem::take(&mut inner.hooks);
        let module = inner.module.clone();
        drop(inner);
        for id in hooks {
            module.remove_hook(id);
        }
    }
}

/// Eligibility gate interception.
fn gate(state: &Rc<RefCell<QvmPickupState>>, call: &mut QvmFunctionCall) -> Result<i32, GuestError> {
    let frame = state.borrow().frames.last().cloned();
    let options = Rc::clone(&state.borrow().options);
    let Some(frame) = frame else {
        return Ok(call.proceed());
    };
    let profile = &options.profile;
    if !called_from(state, call, &profile.gate.calls)?
        || call.argument(profile.gate.item_argument)? != frame.item_pointer
        || call.argument(profile.gate.player_argument)? != frame.player_pointer
    {
        return Ok(call.proceed());
    }
    if !live(state, &frame)? {
        return cancel_frame(state, call, frame.id);
    }
    if !matches!(frame.selection, QvmPickupSelection::Replacement { .. }) {
        return Ok(call.proceed());
    }
    let Some(grant_index) = frame.grant else {
        return Ok(call.proceed());
    };
    let item = entity(state, frame.item_slot)?;
    let player = QvmMemoryWindow::new(
        options.game.module.memory(),
        address(frame.player_pointer)?,
        profile.client_stride,
    )?;
    let eligible = (options.profile.grants[grant_index].eligible)(QvmPickupEligibility { call, item, player })?;
    call.effect(|| {});
    if !live(state, &frame)? {
        return cancel_frame(state, call, frame.id);
    }
    Ok(i32::from(eligible))
}

/// Evaluate a supply quantity against projected words.
fn quantity(
    state: &Rc<RefCell<QvmPickupState>>,
    frame_id: u64,
    grant_index: usize,
    weapon: &QvmGrantWeapon,
    count: f64,
) -> Result<i32, GuestError> {
    let frame = state.borrow().frames.last().cloned();
    let Some(frame) = frame else {
        return Err(GuestError::invalid("Original pickup quantity has expired"));
    };
    if frame.id != frame_id || frame.supply.is_none() || !live(state, &frame)? {
        return Err(GuestError::invalid("Original pickup quantity has expired"));
    }
    let options = Rc::clone(&state.borrow().options);
    let word = count.trunc();
    if !count.is_finite() || word < f64::from(i32::MIN) || word > f64::from(i32::MAX) {
        return Err(GuestError::invalid("Original QVM pickup counter exceeds its int32 ABI"));
    }
    let word = word as i32;
    let ammo = options
        .supply
        .as_ref()
        .and_then(|supply| (supply.ammo)(&frame.item_record));
    let supply_project = options.supply.as_ref().and_then(|supply| supply.project.clone());
    if weapon.storage == QvmGrantWeaponStorage::Inventory && supply_project.is_none() {
        return Err(GuestError::invalid(
            "Original pickup requires its declared inventory projection",
        ));
    }
    let writes = if weapon.storage == QvmGrantWeaponStorage::Inventory {
        match (&ammo, &supply_project) {
            (Some(ammo), Some(project)) => project(&frame.recipient, ammo, word)?,
            _ => Vec::new(),
        }
    } else if let QvmGrantWeaponStorage::Words { ammo_offset, .. } = &weapon.storage {
        let base = address(frame.player_pointer)?;
        let offset = base
            .checked_add(*ammo_offset)
            .and_then(|base| base.checked_add(frame.item_record.tag as usize * 4))
            .ok_or_else(|| GuestError::invalid("QVM pickup quantity escapes its allocation"))?;
        vec![QvmInventoryWord {
            address: offset,
            value: word,
        }]
    } else {
        Vec::new()
    };
    let mut restore = project(state, &writes)?;
    state.borrow_mut().evaluating.push(QvmEvaluating {
        frame_id,
        region: weapon.quantity.clone(),
        entered: false,
    });
    let outcome = options.game.module.call(
        &[frame.item_pointer, frame.recipient_pointer],
        options.profile.grants[grant_index].entry,
    );
    restore()?;
    state.borrow_mut().evaluating.pop();
    outcome
}

/// Shared grant-region outcome.
type QvmGrantRegionOutcome = Rc<RefCell<Option<GuestError>>>;

/// Build the grant-region entry callback.
fn grant_region(
    state: &Rc<RefCell<QvmPickupState>>,
    frame_id: u64,
    grant_index: usize,
    entry: usize,
    join: usize,
    quantity_offset: usize,
    weapon: Option<QvmGrantWeapon>,
    restore: Rc<RefCell<Box<dyn FnMut() -> Result<(), GuestError>>>>,
    outcome: QvmGrantRegionOutcome,
) -> QvmRegionBinding {
    let state = Rc::clone(state);
    QvmRegionBinding {
        entry,
        join,
        run: Box::new(move |control| {
            if restore.borrow_mut()().is_err() {
                *outcome.borrow_mut() = Some(GuestError::invalid("Original pickup projection lost its word"));
                return QvmRegionDecision::Skip;
            }
            let frame = state.borrow().frames.iter().find(|frame| frame.id == frame_id).cloned();
            let Some(frame) = frame else {
                *outcome.borrow_mut() = Some(GuestError::invalid("Original pickup quantity has expired"));
                return QvmRegionDecision::Skip;
            };
            let dead = match live(&state, &frame) {
                Ok(live) => !live,
                Err(error) => {
                    *outcome.borrow_mut() = Some(error);
                    return QvmRegionDecision::Skip;
                }
            };
            if dead {
                control.cancel_function();
                let error = GuestError::invalid(format!("QVM pickup frame {frame_id} cancelled"));
                if let Some(frame) = state.borrow_mut().frames.iter_mut().find(|frame| frame.id == frame_id) {
                    frame.cancelled = Some(error.clone());
                }
                *outcome.borrow_mut() = Some(error);
                return QvmRegionDecision::Skip;
            }
            let options = Rc::clone(&state.borrow().options);
            if let Some(supply) = &options.supply {
                let ammo = (supply.ammo)(&frame.item_record);
                let amount = match control.local_word(quantity_offset) {
                    Ok(amount) => amount,
                    Err(error) => {
                        *outcome.borrow_mut() = Some(error);
                        return QvmRegionDecision::Skip;
                    }
                };
                if weapon.is_none() && ammo.is_none() {
                    *outcome.borrow_mut() = Some(GuestError::invalid(
                        "Original ammo pickup has no admitted source counter",
                    ));
                    return QvmRegionDecision::Skip;
                }
                let offer = if weapon.is_none() {
                    let ammo = ammo.expect("checked admitted counter");
                    QvmPickupSupplyOffer::Ammo {
                        offer: QvmAmmoGrant { item: ammo, amount },
                    }
                } else {
                    QvmPickupSupplyOffer::Weapon {
                        item: frame.offer.item.clone(),
                        ammo: ammo
                            .map(|ammo| vec![QvmAmmoGrant { item: ammo, amount }])
                            .unwrap_or_default(),
                    }
                };
                let quantity = weapon.as_ref().map(|weapon| {
                    let state = Rc::clone(&state);
                    let weapon = weapon.clone();
                    Rc::new(move |count: f64| quantity(&state, frame_id, grant_index, &weapon, count))
                        as QvmSupplyQuantity
                });
                if let Some(frame) = state.borrow_mut().frames.iter_mut().find(|frame| frame.id == frame_id) {
                    frame.supply = Some(QvmFrameSupply { offer, quantity });
                }
            }
            let take = (|| -> Result<(), GuestError> {
                let grant_fn = {
                    let mut inner = state.borrow_mut();
                    let Some(frame) = inner.frames.iter_mut().find(|frame| frame.id == frame_id) else {
                        return Err(GuestError::invalid("Original pickup quantity has expired"));
                    };
                    if frame.granted {
                        return Err(GuestError::invalid(
                            "Original pickup source caller attempted a second grant",
                        ));
                    }
                    frame.granted = true;
                    match &frame.selection {
                        QvmPickupSelection::Replacement { grant, .. } => Some(Rc::clone(grant)),
                        _ => None,
                    }
                };
                let accepted = grant_fn.is_some_and(|grant| grant() == QvmPickupOutcome::Accepted);
                let frame = state.borrow().frames.iter().find(|frame| frame.id == frame_id).cloned();
                let Some(frame) = frame else {
                    return Err(GuestError::invalid("Original pickup quantity has expired"));
                };
                if !accepted || !live(&state, &frame)? {
                    control.cancel_function();
                    let error = GuestError::invalid(format!("QVM pickup frame {frame_id} cancelled"));
                    if let Some(frame) = state.borrow_mut().frames.iter_mut().find(|frame| frame.id == frame_id) {
                        frame.cancelled = Some(error.clone());
                    }
                    return Err(error);
                }
                Ok(())
            })();
            if let Some(frame) = state.borrow_mut().frames.iter_mut().find(|frame| frame.id == frame_id) {
                frame.supply = None;
            }
            if let Err(error) = take {
                *outcome.borrow_mut() = Some(error);
            }
            QvmRegionDecision::Skip
        }),
        completed: None,
    }
}

/// Grant interception.
fn grant(
    state: &Rc<RefCell<QvmPickupState>>,
    call: &mut QvmFunctionCall,
    grant_index: usize,
) -> Result<i32, GuestError> {
    let evaluating = state.borrow().evaluating.last().cloned();
    if let Some(evaluating) = evaluating.filter(|evaluating| !evaluating.entered) {
        let frame = state
            .borrow()
            .frames
            .iter()
            .find(|frame| frame.id == evaluating.frame_id)
            .cloned();
        let Some(frame) = frame else {
            return Err(GuestError::invalid("Original pickup evaluation lost its source caller"));
        };
        if frame.grant != Some(grant_index)
            || !live(state, &frame)?
            || call.argument(0)? != frame.item_pointer
            || call.argument(1)? != frame.recipient_pointer
        {
            return Err(GuestError::invalid("Original pickup evaluation lost its source caller"));
        }
        if let Some(evaluating) = state.borrow_mut().evaluating.last_mut() {
            evaluating.entered = true;
        }
        return Ok(call.evaluate_region(&evaluating.region, &[]));
    }
    let frame = state.borrow().frames.last().cloned();
    let options = Rc::clone(&state.borrow().options);
    let Some(frame) = frame else {
        return Ok(call.proceed());
    };
    let profile_grant = &options.profile.grants[grant_index];
    if frame.grant != Some(grant_index)
        || !called_from(state, call, &profile_grant.calls)?
        || call.argument(0)? != frame.item_pointer
        || call.argument(1)? != frame.recipient_pointer
    {
        return Ok(call.proceed());
    }
    if !live(state, &frame)? {
        return cancel_frame(state, call, frame.id);
    }
    if !matches!(frame.selection, QvmPickupSelection::Replacement { .. }) {
        return Ok(call.proceed());
    }
    let frame_id = frame.id;
    let take = |state: &Rc<RefCell<QvmPickupState>>, call: &mut QvmFunctionCall| -> Result<(), GuestError> {
        let grant_fn = {
            let mut inner = state.borrow_mut();
            let Some(frame) = inner.frames.iter_mut().find(|frame| frame.id == frame_id) else {
                return Err(GuestError::invalid("Original pickup quantity has expired"));
            };
            if frame.granted {
                return Err(GuestError::invalid(
                    "Original pickup source caller attempted a second grant",
                ));
            }
            frame.granted = true;
            match &frame.selection {
                QvmPickupSelection::Replacement { grant, .. } => Some(Rc::clone(grant)),
                _ => None,
            }
        };
        let accepted = grant_fn.is_some_and(|grant| grant() == QvmPickupOutcome::Accepted);
        let frame = state.borrow().frames.iter().find(|frame| frame.id == frame_id).cloned();
        let Some(frame) = frame else {
            return Err(GuestError::invalid("Original pickup quantity has expired"));
        };
        if !accepted || !live(state, &frame)? {
            return cancel_frame(state, call, frame.id).map(|_| ());
        }
        Ok(())
    };
    match profile_grant.operation.clone() {
        QvmGrantOperation::Return { .. } => {
            take(state, call)?;
            state
                .borrow()
                .returns
                .get(&grant_index)
                .copied()
                .ok_or_else(|| GuestError::invalid("Missing original pickup lifecycle return"))
        }
        QvmGrantOperation::Region {
            entry,
            join,
            quantity: quantity_offset,
            weapon,
        } => {
            let mut writes = Vec::new();
            if let (Some(weapon), Some(supply)) = (&weapon, &options.supply) {
                let owned = (supply.owns)(&frame.recipient, &frame.offer.item);
                match &weapon.storage {
                    QvmGrantWeaponStorage::Inventory => {
                        let Some(project) = &supply.project else {
                            return Err(GuestError::invalid(
                                "Original pickup requires its declared inventory projection",
                            ));
                        };
                        writes = project(&frame.recipient, &frame.offer.item, i32::from(owned))?;
                    }
                    QvmGrantWeaponStorage::Words { bits_offset, .. } => {
                        let base = address(frame.player_pointer)?;
                        let at = base
                            .checked_add(*bits_offset)
                            .ok_or_else(|| GuestError::invalid("QVM pickup ownership escapes its allocation"))?;
                        let memory = options.game.module.memory();
                        let before = memory.read_i32(at)?;
                        let mask = 1i32.wrapping_shl(frame.item_record.tag as u32);
                        writes.push(QvmInventoryWord {
                            address: at,
                            value: if owned { before | mask } else { before & !mask },
                        });
                    }
                }
            }
            let restore: Rc<RefCell<Box<dyn FnMut() -> Result<(), GuestError>>>> =
                Rc::new(RefCell::new(Box::new(project(state, &writes)?)));
            let outcome: QvmGrantRegionOutcome = Rc::new(RefCell::new(None));
            call.regions(vec![grant_region(
                state,
                frame.id,
                grant_index,
                entry,
                join,
                quantity_offset,
                weapon,
                Rc::clone(&restore),
                Rc::clone(&outcome),
            )]);
            let result = call.proceed();
            restore.borrow_mut()()?;
            if let Some(error) = outcome.borrow_mut().take() {
                return Err(error);
            }
            Ok(result)
        }
    }
}

/// Targets interception.
fn targets(state: &Rc<RefCell<QvmPickupState>>, call: &mut QvmFunctionCall) -> Result<i32, GuestError> {
    let frame = state.borrow().frames.last().cloned();
    let options = Rc::clone(&state.borrow().options);
    let Some(frame) = frame else {
        return Ok(call.proceed());
    };
    if !called_from(state, call, &options.profile.targets.calls)? {
        return Ok(call.proceed());
    }
    if !live(state, &frame)? {
        return cancel_frame(state, call, frame.id);
    }
    let result = call.proceed();
    call.effect(|| {});
    if !live(state, &frame)? {
        return cancel_frame(state, call, frame.id);
    }
    Ok(result)
}

/// Original pickups over Touch_Item.
pub struct QvmPrimaryPickups {
    /// Shared state.
    state: Rc<RefCell<QvmPickupState>>,
}

impl QvmPrimaryPickups {
    /// Bind pickup entries after validating the profile against the artifact.
    pub fn new(options: QvmPickupOptions) -> Result<Self, GuestError> {
        let module = options.game.module.module_id();
        if options.artifact.module.id != module.id
            || options.artifact.module.digest != module.digest
            || options.profile.module.id != module.id
            || options.profile.module.digest != module.digest
            || options.profile.module.revision != module.revision
            || options.profile.module.artifact_path != module.artifact_path
            || options.profile.abi_profile != options.game.module.abi_profile()
        {
            return Err(GuestError::invalid(
                "Original pickup profile differs from its QVM executable",
            ));
        }
        let fields = [
            options.profile.fields.inuse,
            options.profile.fields.client,
            options.profile.fields.health,
            options.profile.fields.item,
            options.profile.fields.count,
            options.profile.fields.flags,
        ];
        for field in fields {
            if field % 4 != 0
                || field
                    .checked_add(4)
                    .map_or(true, |end| end > options.profile.entity_stride)
            {
                return Err(GuestError::invalid(
                    "Original pickup field is outside its entity record",
                ));
            }
        }
        let instructions = &options.artifact.image.instructions;
        let entry = |index: usize| -> Result<(), GuestError> {
            if instructions
                .get(index)
                .map_or(true, |instruction| instruction.opcode != QvmOpcode::OpEnter)
            {
                return Err(GuestError::invalid("Original pickup requires a source function entry"));
            }
            Ok(())
        };
        let mut return_pcs = HashMap::new();
        let mut calls = |target: usize, sites: &[usize]| -> Result<(), GuestError> {
            entry(target)?;
            for site in sites {
                let instruction = instructions.get(*site);
                let previous = site.checked_sub(1).and_then(|before| instructions.get(before));
                let matched = instruction.is_some_and(|instruction| instruction.opcode == QvmOpcode::OpCall)
                    && previous.is_some_and(|previous| {
                        previous.opcode == QvmOpcode::OpConst && i64::from(previous.operand) == target as i64
                    });
                if !matched {
                    return Err(GuestError::invalid(
                        "Original pickup call site differs from its qualified target",
                    ));
                }
                return_pcs.insert(*site, instructions[*site].byte_offset + 1);
            }
            Ok(())
        };
        entry(options.profile.touch)?;
        entry(options.profile.free)?;
        calls(options.profile.gate.entry, &options.profile.gate.calls)?;
        calls(options.profile.targets.entry, &options.profile.targets.calls)?;
        let mut returns = HashMap::new();
        let mut types = Vec::new();
        for (index, grant) in options.profile.grants.iter().enumerate() {
            if types.contains(&grant.item_type) {
                return Err(GuestError::invalid("Original pickup has duplicate grant types"));
            }
            types.push(grant.item_type);
            calls(grant.entry, &grant.calls)?;
            match &grant.operation {
                QvmGrantOperation::Region {
                    entry, join, weapon, ..
                } => {
                    qualify_qvm_region(instructions, grant.entry, *entry, *join)?;
                    if let Some(weapon) = weapon {
                        qualify_qvm_region_evaluation(
                            instructions,
                            grant.entry,
                            &weapon.quantity,
                            QvmRegionAccess::Source,
                        )?;
                    }
                }
                QvmGrantOperation::Return { accepted_return } => {
                    let next = accepted_return.checked_add(1);
                    let operand = match (
                        instructions.get(*accepted_return),
                        next.and_then(|next| instructions.get(next)),
                    ) {
                        (Some(value), Some(leave))
                            if value.opcode == QvmOpcode::OpConst
                                && leave.opcode == QvmOpcode::OpLeave
                                && value.operand != 0 =>
                        {
                            value.operand
                        }
                        _ => {
                            return Err(GuestError::invalid(
                                "Original pickup lifecycle return is not a qualified source constant",
                            ));
                        }
                    };
                    let mut owner = *accepted_return;
                    loop {
                        match instructions.get(owner) {
                            Some(instruction) if instruction.opcode == QvmOpcode::OpEnter => break,
                            Some(_) => {
                                owner = owner.checked_sub(1).ok_or_else(|| {
                                    GuestError::invalid(
                                        "Original pickup lifecycle return belongs to another source function",
                                    )
                                })?;
                            }
                            None => {
                                return Err(GuestError::invalid(
                                    "Original pickup lifecycle return belongs to another source function",
                                ));
                            }
                        }
                    }
                    if owner != grant.entry {
                        return Err(GuestError::invalid(
                            "Original pickup lifecycle return belongs to another source function",
                        ));
                    }
                    returns.insert(index, operand);
                }
            }
        }
        if options.profile.items.live_source && options.catalog.is_none() {
            return Err(GuestError::invalid(
                "Live original pickups require their source catalog owner",
            ));
        }
        let items = if options.catalog.is_none() {
            Some(read_qvm_item_records(
                &options.artifact.image.initialized_data,
                &options.profile.items,
                None,
                None,
            )?)
        } else {
            None
        };
        let module = options.game.module.clone();
        let profile = options.profile.clone();
        let state = Rc::new(RefCell::new(QvmPickupState {
            options: Rc::new(options),
            module: module.clone(),
            hooks: Vec::new(),
            frames: Vec::new(),
            evaluating: Vec::new(),
            items,
            returns,
            return_pcs,
            next_frame: 0,
            closed: false,
            error: None,
        }));
        let mut hooks = Vec::new();
        let bind = |hooks: &mut Vec<u64>, entry: usize, hook: QvmHookFn| {
            hooks.push(module.bind_invocation(entry, hook));
        };
        let touched = Rc::clone(&state);
        bind(
            &mut hooks,
            profile.touch,
            Rc::new(move |call| run_hook(&touched, || touch(&touched, call))),
        );
        let gated = Rc::clone(&state);
        bind(
            &mut hooks,
            profile.gate.entry,
            Rc::new(move |call| run_hook(&gated, || gate(&gated, call))),
        );
        for (index, profile_grant) in profile.grants.iter().enumerate() {
            let granted = Rc::clone(&state);
            bind(
                &mut hooks,
                profile_grant.entry,
                Rc::new(move |call| run_hook(&granted, || grant(&granted, call, index))),
            );
        }
        let targeted = Rc::clone(&state);
        bind(
            &mut hooks,
            profile.targets.entry,
            Rc::new(move |call| run_hook(&targeted, || targets(&targeted, call))),
        );
        if matches!(state.borrow().options.lifetime, QvmPickupLifetime::OwnFreeHook { .. }) {
            let freed = Rc::clone(&state);
            bind(
                &mut hooks,
                profile.free,
                Rc::new(move |call| {
                    run_hook(&freed, || {
                        let options = Rc::clone(&freed.borrow().options);
                        let pointer = call.argument(0)?;
                        let slot = options.game.data.number_from_pointer(pointer)?;
                        let actor = (options.actor)(slot);
                        let result = call.proceed();
                        if let Some(actor) = actor {
                            if entity(&freed, slot)?.get_i32(options.profile.fields.inuse)? == 0 {
                                if let QvmPickupLifetime::OwnFreeHook { retire } = &options.lifetime {
                                    retire(actor);
                                }
                            }
                        }
                        after_free(&freed, pointer, call)?;
                        Ok(result)
                    })
                }),
            );
        }
        state.borrow_mut().hooks = hooks;
        Ok(Self { state })
    }

    /// Invalidate frames touching a freed pointer.
    pub fn after_free(&self, pointer: i32, control: &mut QvmFunctionCall) -> Result<(), GuestError> {
        after_free(&self.state, pointer, control)
    }

    /// Take the held supply for an offer.
    pub fn supply(&self, offer: &QvmOriginalPickupOffer) -> Result<QvmPickupSupply, GuestError> {
        let frame = self.state.borrow().frames.last().cloned();
        let Some(frame) = frame else {
            return Err(GuestError::invalid(
                "Selected QVM supply requires its held original grant",
            ));
        };
        let held = frame.supply.clone();
        let Some(held) = held else {
            return Err(GuestError::invalid(
                "Selected QVM supply requires its held original grant",
            ));
        };
        if !frame.granted
            || frame.offer.item != offer.item
            || frame.item.id() != &offer.pickup
            || frame.recipient.id() != &offer.recipient
            || !live(&self.state, &frame)?
        {
            return Err(GuestError::invalid(
                "Selected QVM supply requires its held original grant",
            ));
        }
        Ok(QvmPickupSupply {
            offer: held.offer,
            quantity: held.quantity,
        })
    }

    /// Fail while a pickup executes.
    pub fn assert_idle(&self) -> Result<(), GuestError> {
        if !self.state.borrow().frames.is_empty() {
            return Err(GuestError::invalid("Cannot save during original QVM pickup execution"));
        }
        Ok(())
    }

    /// Take the first recorded hook failure, if any.
    pub fn take_error(&self) -> Option<GuestError> {
        self.state.borrow_mut().error.take()
    }

    /// Close the binding, disposing hooks once idle.
    pub fn close(&mut self) {
        let mut inner = self.state.borrow_mut();
        inner.closed = true;
        if !inner.frames.is_empty() {
            return;
        }
        let hooks = std::mem::take(&mut inner.hooks);
        let module = inner.module.clone();
        drop(inner);
        for id in hooks {
            module.remove_hook(id);
        }
    }

    /// Drive the touch hook with a crafted call (test seam).
    #[cfg(test)]
    fn test_touch(&self, call: &mut QvmFunctionCall) -> Result<i32, GuestError> {
        touch(&self.state, call)
    }

    /// Drive the gate hook with a crafted call (test seam).
    #[cfg(test)]
    fn test_gate(&self, call: &mut QvmFunctionCall) -> Result<i32, GuestError> {
        gate(&self.state, call)
    }

    /// Drive a grant hook with a crafted call (test seam).
    #[cfg(test)]
    fn test_grant(&self, call: &mut QvmFunctionCall, grant_index: usize) -> Result<i32, GuestError> {
        grant(&self.state, call, grant_index)
    }

    /// Drive the targets hook with a crafted call (test seam).
    #[cfg(test)]
    fn test_targets(&self, call: &mut QvmFunctionCall) -> Result<i32, GuestError> {
        targets(&self.state, call)
    }

    /// Push a synthetic frame with its in-use watch (test seam for the hub's
    /// stub `proceed`, which cannot nest gate/grant calls inside touch).
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn test_push_frame(
        &self,
        offer: QvmOriginalPickupOffer,
        item: OwnedActor,
        recipient: OwnedActor,
        item_slot: usize,
        recipient_slot: usize,
        item_record: QvmCatalogRecord,
        grant: Option<usize>,
        selection: QvmPickupSelection,
    ) -> Result<(u64, u64), GuestError> {
        let options = Rc::clone(&self.state.borrow().options);
        let located = options.game.data.checkpoint();
        let item_pointer = (located.entities_word + item_slot * located.entity_stride) as i32;
        let recipient_pointer = (located.entities_word + recipient_slot * located.entity_stride) as i32;
        let player_pointer = entity(&self.state, recipient_slot)?.get_i32(options.profile.fields.client)?;
        let id = {
            let mut inner = self.state.borrow_mut();
            let id = inner.next_frame;
            inner.next_frame += 1;
            inner.frames.push(QvmPickupFrame {
                id,
                offer,
                supply: None,
                cancellation: QvmCancellationScope,
                item,
                recipient,
                item_slot,
                recipient_slot,
                item_pointer,
                player_pointer,
                recipient_pointer,
                item_record,
                grant,
                selection,
                invalid: false,
                granted: false,
                cancelled: None,
            });
            id
        };
        let watch = observe_frame(&self.state, id)?;
        Ok((id, watch))
    }

    /// Pop a synthetic frame (test seam).
    #[cfg(test)]
    fn test_pop_frame(&self, id: u64, watch: u64) {
        finish_frame(&self.state, id, watch);
    }
}

/// Invalidate frames touching a freed pointer.
fn after_free(
    state: &Rc<RefCell<QvmPickupState>>,
    pointer: i32,
    control: &mut QvmFunctionCall,
) -> Result<(), GuestError> {
    let options = Rc::clone(&state.borrow().options);
    options.game.data.number_from_pointer(pointer)?;
    let base = address(pointer)?;
    let at = base
        .checked_add(options.profile.fields.inuse)
        .ok_or_else(|| GuestError::invalid("QVM pickup pointer escapes its allocation"))?;
    if options.game.module.memory().read_i32(at)? != 0 {
        return Ok(());
    }
    let frames: Vec<QvmPickupFrame> = state.borrow().frames.clone();
    for frame in frames.iter().rev() {
        if frame.item_pointer == pointer || frame.recipient_pointer == pointer {
            if let Some(live) = state.borrow_mut().frames.iter_mut().find(|live| live.id == frame.id) {
                live.invalid = true;
            }
            cancel_frame(state, control, frame.id).map(|_| ())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;

    use qa_core::identity::{IdentityOwner, ProviderId};

    use super::super::game_data::{
        ModuleIdentity, QvmArtifact, QvmGameData, QvmImage, QvmInstruction, QvmOpcode, QvmRegionControl, QvmRole,
        QvmSharedMemory,
    };
    use super::super::game_input::{QvmSourceTime, QvmTimeKind};
    use super::super::item_catalog::{QvmItemAddress, QvmItemCount, QvmItemFields};
    use super::*;

    const ENTITIES: usize = 4096;
    const CLIENTS: usize = 8192;
    const ENTITY_STRIDE: usize = 256;
    const CLIENT_STRIDE: usize = 512;
    const ITEM_SLOT: usize = 2;
    const RECIPIENT_SLOT: usize = 0;
    const ITEM_TABLE: usize = 512;
    const STACK: usize = 32768;

    fn item_pointer(slot: usize) -> i32 {
        (ENTITIES + slot * ENTITY_STRIDE) as i32
    }

    fn instructions() -> Vec<QvmInstruction> {
        let mut code: Vec<(QvmOpcode, i32)> = vec![
            (QvmOpcode::OpEnter, 0), // 0 touch
            (QvmOpcode::OpEnter, 0), // 1 free
            (QvmOpcode::OpEnter, 0), // 2 gate
            (QvmOpcode::OpConst, 2),
            (QvmOpcode::OpCall, 0),  // 4 gate call
            (QvmOpcode::OpEnter, 0), // 5 targets
            (QvmOpcode::OpConst, 5),
            (QvmOpcode::OpCall, 0),  // 7 targets call
            (QvmOpcode::OpEnter, 0), // 8 grant0
            (QvmOpcode::OpConst, 8),
            (QvmOpcode::OpCall, 0),   // 10 grant0 call
            (QvmOpcode::OpConst, 99), // 11 accepted return
            (QvmOpcode::OpLeave, 0),  // 12
            (QvmOpcode::OpEnter, 0),  // 13 grant1
            (QvmOpcode::OpConst, 13),
            (QvmOpcode::OpCall, 0),   // 15 grant1 call
            (QvmOpcode::OpConst, 0),  // 16 region entry
            (QvmOpcode::OpPop, 0),    // 17
            (QvmOpcode::OpIgnore, 0), // 18 join
            (QvmOpcode::OpLeave, 0),  // 19
            (QvmOpcode::OpEnter, 0),  // 20 foreign function
            (QvmOpcode::OpConst, 5),  // 21 foreign return
            (QvmOpcode::OpLeave, 0),  // 22
        ];
        code.drain(..)
            .enumerate()
            .map(|(index, (opcode, operand))| QvmInstruction::word(opcode, operand, index * 8))
            .collect()
    }

    fn return_pc(site: usize) -> i32 {
        (site * 8 + 1) as i32
    }

    fn initialized_data() -> Vec<u8> {
        let mut data = vec![0u8; 1024];
        let word = |data: &mut Vec<u8>, offset: usize, value: i32| {
            data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        word(&mut data, ITEM_TABLE, 640);
        word(&mut data, ITEM_TABLE + 8, 672);
        word(&mut data, ITEM_TABLE + 16, 2);
        word(&mut data, ITEM_TABLE + 20, 3);
        word(&mut data, ITEM_TABLE + 64, 704);
        word(&mut data, ITEM_TABLE + 72, 720);
        word(&mut data, ITEM_TABLE + 80, 7);
        word(&mut data, ITEM_TABLE + 84, 1);
        data[640..655].copy_from_slice(b"weapon_shotgun\0");
        data[672..680].copy_from_slice(b"Shotgun\0");
        data[704..714].copy_from_slice(b"item_quad\0");
        data[720..725].copy_from_slice(b"Quad\0");
        data
    }

    fn layout() -> QvmItemLayout {
        QvmItemLayout {
            address: QvmItemAddress::Direct(ITEM_TABLE),
            count: QvmItemCount::Direct(2),
            live_source: false,
            stride: 64,
            fields: QvmItemFields {
                class_name: 0,
                pickup_name: 8,
                item_type: 16,
                tag: 20,
            },
            weapon_type: 0,
            ammo_type: 0,
        }
    }

    struct Fixture {
        pickups: Rc<RefCell<QvmPrimaryPickups>>,
        module: QvmModule,
        data: QvmGameData,
        artifact: QvmArtifact,
        profile: QvmPickupProfile,
        actors: HashSet<ActorId>,
        owned: HashSet<OwnedActor>,
        dead: Rc<RefCell<HashSet<ActorId>>>,
        offers: Rc<RefCell<Vec<QvmOriginalPickupOffer>>>,
        selection: Rc<RefCell<QvmPickupSelection>>,
        owns: Rc<RefCell<bool>>,
        projects: Rc<RefCell<Vec<(ItemId, i32)>>>,
        retired: Rc<RefCell<Vec<ActorId>>>,
        records: Vec<QvmCatalogRecord>,
    }

    impl Fixture {
        fn memory(&self) -> QvmSharedMemory {
            self.module.memory()
        }

        fn set_entity_word(&self, slot: usize, offset: usize, value: i32) {
            self.data.entity_bytes(slot).unwrap().set_i32(offset, value).unwrap();
        }

        fn offer(&self, item: &str, pickup_slot: usize) -> QvmOriginalPickupOffer {
            QvmOriginalPickupOffer {
                recipient: self.actor(RECIPIENT_SLOT),
                pickup: self.actor(pickup_slot),
                source: "test:qagame".to_string(),
                item: item.to_string(),
                default_resource: None,
                count: QvmPickupCount::Default,
                dropped: false,
                time: QvmSourceTime {
                    kind: QvmTimeKind::Milliseconds,
                    value: 1000.0,
                },
                cargo: None,
                grant: None,
            }
        }

        fn actor(&self, slot: usize) -> ActorId {
            self.actors
                .iter()
                .find(|actor| actor.slot() == slot as u32)
                .cloned()
                .unwrap()
        }

        fn owned(&self, slot: usize) -> OwnedActor {
            self.owned
                .iter()
                .find(|owned| owned.id().slot() == slot as u32)
                .cloned()
                .unwrap()
        }

        fn replacement(&self, outcome: QvmPickupOutcome) -> QvmPickupSelection {
            QvmPickupSelection::Replacement {
                current: Rc::new(|| true),
                grant: Rc::new(move || outcome),
            }
        }

        fn call(&self, entry: usize, words: Vec<i32>, pc: i32) -> QvmFunctionCall {
            let mut call = QvmFunctionCall::entered(entry, words, self.memory());
            call.stack_address = STACK;
            self.memory().write_i32(STACK - 8, pc).unwrap();
            call
        }
    }

    fn fixture() -> Fixture {
        let owner = IdentityOwner::create("pickup-test").unwrap();
        let provider = ProviderId::new("test", "provider");
        let mut actors = HashSet::new();
        let mut owned = HashSet::new();
        for slot in 0..4u32 {
            let id = owner.actor(slot, 1);
            actors.insert(id.clone());
            owned.insert(owner.owned_actor(&id, provider.clone()).unwrap());
        }
        let mut image = QvmImage::default();
        image.instructions = instructions();
        image.initialized_data = initialized_data();
        image.allocated_data_length = 65536;
        let artifact = QvmArtifact {
            module: ModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: "test".to_string(),
                digest: "test".to_string(),
                revision: "1".to_string(),
            },
            role: QvmRole::Qagame,
            abi_profile: None,
            image,
        };
        let module = QvmModule::new(artifact.clone(), None, None).unwrap();
        let data = QvmGameData::new(module.memory(), AbiProfile::Modern);
        data.locate(ENTITIES as i32, 4, ENTITY_STRIDE, CLIENTS as i32, CLIENT_STRIDE)
            .unwrap();
        for slot in 0..4 {
            data.entity_bytes(slot).unwrap().set_i32(0, 1).unwrap();
        }
        data.entity_bytes(RECIPIENT_SLOT)
            .unwrap()
            .set_i32(16, CLIENTS as i32)
            .unwrap();
        data.entity_bytes(RECIPIENT_SLOT).unwrap().set_i32(32, 100).unwrap();
        data.entity_bytes(ITEM_SLOT)
            .unwrap()
            .set_i32(48, ITEM_TABLE as i32)
            .unwrap();
        data.entity_bytes(ITEM_SLOT).unwrap().set_i32(64, 0).unwrap();
        data.entity_bytes(ITEM_SLOT).unwrap().set_i32(80, 0).unwrap();
        data.entity_bytes(ITEM_SLOT).unwrap().set_i32(160, 0).unwrap();
        data.entity_bytes(3)
            .unwrap()
            .set_i32(48, (ITEM_TABLE + 64) as i32)
            .unwrap();
        data.entity_bytes(3).unwrap().set_i32(160, 1).unwrap();
        let dead = Rc::new(RefCell::new(HashSet::new()));
        let actor_dead = Rc::clone(&dead);
        let actors_lookup = actors.clone();
        let owned_lookup = owned.clone();
        let offers = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&offers);
        let selection = Rc::new(RefCell::new(QvmPickupSelection::Blocked));
        let selected = Rc::clone(&selection);
        let owns = Rc::new(RefCell::new(false));
        let owned_flag = Rc::clone(&owns);
        let projects = Rc::new(RefCell::new(Vec::new()));
        let projected = Rc::clone(&projects);
        let retired = Rc::new(RefCell::new(Vec::new()));
        let retired_actor = Rc::clone(&retired);
        let profile = QvmPickupProfile {
            module: module.module_id(),
            abi_profile: AbiProfile::Modern,
            entity_stride: ENTITY_STRIDE,
            client_stride: CLIENT_STRIDE,
            fields: QvmPickupFields {
                inuse: 0,
                client: 16,
                health: 32,
                item: 48,
                count: 64,
                flags: 80,
            },
            dropped_flag: 1,
            items: layout(),
            touch: 0,
            gate: QvmPickupGate {
                entry: 2,
                calls: vec![4],
                item_argument: 0,
                player_argument: 1,
            },
            targets: QvmPickupTargets {
                entry: 5,
                calls: vec![7],
            },
            free: 1,
            objective_types: vec![7],
            grants: vec![
                QvmPickupGrant {
                    item_type: 2,
                    entry: 8,
                    calls: vec![10],
                    operation: QvmGrantOperation::Return { accepted_return: 11 },
                    eligible: Rc::new(|_| Ok(true)),
                },
                QvmPickupGrant {
                    item_type: 7,
                    entry: 13,
                    calls: vec![15],
                    operation: QvmGrantOperation::Region {
                        entry: 16,
                        join: 18,
                        quantity: 8,
                        weapon: Some(QvmGrantWeapon {
                            quantity: QvmRegionEvaluation {
                                entry: 16,
                                join: 18,
                                inputs: Vec::new(),
                                result: None,
                            },
                            storage: QvmGrantWeaponStorage::Words {
                                bits_offset: 64,
                                ammo_offset: 128,
                            },
                        }),
                    },
                    eligible: Rc::new(|_| Ok(true)),
                },
            ],
        };
        let artifact_base = artifact.clone();
        let profile_base = profile.clone();
        let options = QvmPickupOptions {
            game: QvmPickupGame {
                module: module.clone(),
                data: data.clone(),
            },
            artifact,
            profile,
            catalog: None,
            actor: Rc::new(move |slot| {
                owned_lookup
                    .iter()
                    .find(|owned| owned.id().slot() == slot as u32)
                    .cloned()
            }),
            current: Rc::new(move |actor: &OwnedActor, _| !actor_dead.borrow().contains(actor.id())),
            resolve_item: Rc::new(|record| {
                let item = if record.item_type == 2 { "q3:shotgun" } else { "q3:quad" };
                QvmResolvedItem {
                    item: item.to_string(),
                    resource: None,
                }
            }),
            time: Rc::new(|| QvmSourceTime {
                kind: QvmTimeKind::Milliseconds,
                value: 1000.0,
            }),
            supply: Some(QvmPickupSupplyBridge {
                ammo: Rc::new(|record| {
                    if record.item_type == 2 {
                        Some("q3:shells".to_string())
                    } else {
                        None
                    }
                }),
                owns: Rc::new(move |_, _| *owned_flag.borrow()),
                project: Some(Rc::new(move |_, item: &ItemId, count: i32| {
                    projected.borrow_mut().push((item.clone(), count));
                    Ok(vec![QvmInventoryWord {
                        address: CLIENTS + 256,
                        value: count,
                    }])
                })),
            }),
            run_source: Rc::new(
                move |offer: &QvmOriginalPickupOffer,
                      execute: &mut dyn FnMut(QvmPickupSelection) -> Result<i32, GuestError>| {
                    recorded.borrow_mut().push(offer.clone());
                    execute(selected.borrow().clone())
                },
            ),
            lifetime: QvmPickupLifetime::OwnFreeHook {
                retire: Rc::new(move |actor| {
                    retired_actor.borrow_mut().push(actor.id().clone());
                }),
            },
        };
        let records = read_qvm_item_records(&initialized_data(), &layout(), None, None).unwrap();
        let pickups = Rc::new(RefCell::new(QvmPrimaryPickups::new(options).unwrap()));
        Fixture {
            pickups,
            module,
            data,
            artifact: artifact_base,
            profile: profile_base,
            actors: actors_lookup,
            owned,
            dead,
            offers,
            selection,
            owns,
            projects,
            retired,
            records,
        }
    }

    fn try_build(
        fixture: &Fixture,
        mutate: impl FnOnce(&mut QvmPickupProfile, &mut QvmArtifact),
        supply: Option<QvmPickupSupplyBridge>,
    ) -> Result<QvmPrimaryPickups, GuestError> {
        let mut profile = fixture.profile.clone();
        let mut artifact = fixture.artifact.clone();
        mutate(&mut profile, &mut artifact);
        QvmPrimaryPickups::new(QvmPickupOptions {
            game: QvmPickupGame {
                module: fixture.module.clone(),
                data: fixture.data.clone(),
            },
            artifact,
            profile,
            catalog: None,
            actor: Rc::new(|_| None),
            current: Rc::new(|_, _| true),
            resolve_item: Rc::new(|_| QvmResolvedItem {
                item: "q3:x".to_string(),
                resource: None,
            }),
            time: Rc::new(|| QvmSourceTime {
                kind: QvmTimeKind::Milliseconds,
                value: 0.0,
            }),
            supply,
            run_source: Rc::new(|_, execute| execute(QvmPickupSelection::Blocked)),
            lifetime: QvmPickupLifetime::SharedFreeHook,
        })
    }

    fn error_text(result: Result<QvmPrimaryPickups, GuestError>) -> String {
        format!("{:?}", result.unwrap_err())
    }

    #[test]
    fn constructor_validates_profile_and_calls() {
        let fixture = fixture();
        assert_eq!(fixture.records.len(), 2);
        assert_eq!(fixture.records[0].item_type, 2);
        assert_eq!(fixture.records[1].tag, 1);

        let bad = try_build(&fixture, |_, artifact| artifact.module.digest = "bad".to_string(), None);
        assert!(error_text(bad).contains("differs from its QVM executable"));
        let bad = try_build(&fixture, |profile, _| profile.module.id = "bad".to_string(), None);
        assert!(error_text(bad).contains("differs from its QVM executable"));
        let bad = try_build(&fixture, |profile, _| profile.abi_profile = AbiProfile::Legacy, None);
        assert!(error_text(bad).contains("differs from its QVM executable"));
        let bad = try_build(&fixture, |profile, _| profile.fields.inuse = 2, None);
        assert!(error_text(bad).contains("outside its entity record"));
        let bad = try_build(&fixture, |profile, _| profile.fields.flags = 256, None);
        assert!(error_text(bad).contains("outside its entity record"));
        let bad = try_build(&fixture, |profile, _| profile.touch = 3, None);
        assert!(error_text(bad).contains("source function entry"));
        let bad = try_build(&fixture, |profile, _| profile.gate.calls = vec![3], None);
        assert!(error_text(bad).contains("call site differs"));
        let bad = try_build(&fixture, |profile, _| profile.grants[1].item_type = 2, None);
        assert!(error_text(bad).contains("duplicate grant types"));
        let bad = try_build(
            &fixture,
            |profile, _| {
                profile.grants[1].operation = QvmGrantOperation::Region {
                    entry: 13,
                    join: 18,
                    quantity: 8,
                    weapon: None,
                };
            },
            None,
        );
        assert!(error_text(bad).contains("QVM region"));
        let bad = try_build(
            &fixture,
            |profile, _| {
                profile.grants[0].operation = QvmGrantOperation::Return { accepted_return: 10 };
            },
            None,
        );
        assert!(error_text(bad).contains("not a qualified source constant"));
        let bad = try_build(
            &fixture,
            |profile, _| {
                profile.grants[0].operation = QvmGrantOperation::Return { accepted_return: 16 };
            },
            None,
        );
        assert!(error_text(bad).contains("not a qualified source constant"));
        let bad = try_build(
            &fixture,
            |profile, _| {
                profile.grants[0].operation = QvmGrantOperation::Return { accepted_return: 21 };
            },
            None,
        );
        assert!(error_text(bad).contains("belongs to another source function"));
        let bad = try_build(&fixture, |profile, _| profile.items.live_source = true, None);
        assert!(error_text(bad).contains("source catalog owner"));
    }

    #[test]
    fn touch_builds_offer_and_runs_source() {
        let fixture = fixture();
        let result = fixture
            .module
            .call(&[item_pointer(ITEM_SLOT), item_pointer(RECIPIENT_SLOT)], 0)
            .unwrap();
        assert_eq!(result, 0);
        assert_eq!(fixture.offers.borrow().len(), 1);
        let offer = fixture.offers.borrow()[0].clone();
        assert_eq!(offer.recipient, fixture.actor(RECIPIENT_SLOT));
        assert_eq!(offer.pickup, fixture.actor(ITEM_SLOT));
        assert_eq!(offer.source, "test:qagame");
        assert_eq!(offer.item, "q3:shotgun");
        assert_eq!(offer.count, QvmPickupCount::Default);
        assert!(!offer.dropped);
        assert_eq!(offer.grant, None);
        fixture.pickups.borrow().assert_idle().unwrap();

        fixture.set_entity_word(ITEM_SLOT, 64, 5);
        fixture.set_entity_word(ITEM_SLOT, 80, 1);
        fixture.set_entity_word(ITEM_SLOT, 48, (ITEM_TABLE + 64) as i32);
        fixture.set_entity_word(ITEM_SLOT, 160, 1);
        fixture
            .module
            .call(&[item_pointer(ITEM_SLOT), item_pointer(RECIPIENT_SLOT)], 0)
            .unwrap();
        let offer = fixture.offers.borrow()[1].clone();
        assert_eq!(offer.item, "q3:quad");
        assert_eq!(offer.count, QvmPickupCount::Override { amount: 5 });
        assert!(offer.dropped);
        assert_eq!(offer.grant, Some(QvmPickupGrantKind::MapCoupled));
        *fixture.selection.borrow_mut() = QvmPickupSelection::Original;
        fixture
            .module
            .call(&[item_pointer(ITEM_SLOT), item_pointer(RECIPIENT_SLOT)], 0)
            .unwrap();
        assert_eq!(fixture.offers.borrow().len(), 3);
        fixture.pickups.borrow().assert_idle().unwrap();
        assert!(fixture.pickups.borrow().take_error().is_none());
    }

    #[test]
    fn touch_skips_unusable_recipients() {
        let fixture = fixture();
        fixture.set_entity_word(RECIPIENT_SLOT, 32, 0);
        fixture
            .module
            .call(&[item_pointer(ITEM_SLOT), item_pointer(RECIPIENT_SLOT)], 0)
            .unwrap();
        assert!(fixture.offers.borrow().is_empty());
        fixture.set_entity_word(RECIPIENT_SLOT, 32, 100);
        fixture.set_entity_word(RECIPIENT_SLOT, 16, 0);
        fixture
            .module
            .call(&[item_pointer(ITEM_SLOT), item_pointer(RECIPIENT_SLOT)], 0)
            .unwrap();
        assert!(fixture.offers.borrow().is_empty());
        assert!(fixture.pickups.borrow().take_error().is_none());
    }

    #[test]
    fn touch_rejects_undeclared_items() {
        let fixture = fixture();
        fixture.set_entity_word(ITEM_SLOT, 48, 9999);
        assert_eq!(
            fixture
                .module
                .call(&[item_pointer(ITEM_SLOT), item_pointer(RECIPIENT_SLOT)], 0)
                .unwrap(),
            0
        );
        let error = fixture.pickups.borrow().take_error().unwrap();
        assert!(format!("{error:?}").contains("undeclared source item descriptor"));
        fixture.set_entity_word(ITEM_SLOT, 48, ITEM_TABLE as i32);
        fixture.set_entity_word(ITEM_SLOT, 160, 1);
        fixture
            .module
            .call(&[item_pointer(ITEM_SLOT), item_pointer(RECIPIENT_SLOT)], 0)
            .unwrap();
        assert!(fixture.pickups.borrow().take_error().is_some());
    }

    #[test]
    fn gate_checks_eligibility() {
        let fixture = fixture();
        let offer = fixture.offer("q3:shotgun", ITEM_SLOT);
        let (id, watch) = fixture
            .pickups
            .borrow()
            .test_push_frame(
                offer,
                fixture.owned(ITEM_SLOT),
                fixture.owned(RECIPIENT_SLOT),
                ITEM_SLOT,
                RECIPIENT_SLOT,
                fixture.records[0].clone(),
                Some(0),
                fixture.replacement(QvmPickupOutcome::Accepted),
            )
            .unwrap();
        let mut call = fixture.call(2, vec![item_pointer(ITEM_SLOT), CLIENTS as i32], return_pc(4));
        assert_eq!(fixture.pickups.borrow().test_gate(&mut call).unwrap(), 1);
        let mut foreign = fixture.call(2, vec![item_pointer(ITEM_SLOT), 1234], return_pc(4));
        assert_eq!(fixture.pickups.borrow().test_gate(&mut foreign).unwrap(), 0);
        let mut caller = fixture.call(5, vec![item_pointer(ITEM_SLOT), CLIENTS as i32], 999);
        assert_eq!(fixture.pickups.borrow().test_targets(&mut caller).unwrap(), 0);
        fixture.set_entity_word(ITEM_SLOT, 0, 0);
        let mut dead = fixture.call(2, vec![item_pointer(ITEM_SLOT), CLIENTS as i32], return_pc(4));
        let error = fixture.pickups.borrow().test_gate(&mut dead).unwrap_err();
        assert!(format!("{error:?}").contains("cancelled"));
        fixture.set_entity_word(ITEM_SLOT, 0, 1);
        fixture.dead.borrow_mut().insert(fixture.actor(RECIPIENT_SLOT));
        let mut gone = fixture.call(2, vec![item_pointer(ITEM_SLOT), CLIENTS as i32], return_pc(4));
        let error = fixture.pickups.borrow().test_gate(&mut gone).unwrap_err();
        assert!(format!("{error:?}").contains("cancelled"));
        fixture.dead.borrow_mut().remove(&fixture.actor(RECIPIENT_SLOT));
        fixture.pickups.borrow().test_pop_frame(id, watch);
        assert!(fixture.pickups.borrow().take_error().is_none());
    }

    #[test]
    fn return_grant_takes_once() {
        let fixture = fixture();
        let words = vec![item_pointer(ITEM_SLOT), item_pointer(RECIPIENT_SLOT)];
        let push = |fixture: &Fixture, selection: QvmPickupSelection| {
            fixture.pickups.borrow().test_push_frame(
                fixture.offer("q3:shotgun", ITEM_SLOT),
                fixture.owned(ITEM_SLOT),
                fixture.owned(RECIPIENT_SLOT),
                ITEM_SLOT,
                RECIPIENT_SLOT,
                fixture.records[0].clone(),
                Some(0),
                selection,
            )
        };
        let (id, watch) = push(&fixture, fixture.replacement(QvmPickupOutcome::Accepted)).unwrap();
        let mut call = fixture.call(8, words.clone(), return_pc(10));
        assert_eq!(fixture.pickups.borrow().test_grant(&mut call, 0).unwrap(), 99);
        let mut twice = fixture.call(8, words.clone(), return_pc(10));
        let error = fixture.pickups.borrow().test_grant(&mut twice, 0).unwrap_err();
        assert!(format!("{error:?}").contains("second grant"));
        fixture.pickups.borrow().test_pop_frame(id, watch);

        let (id, watch) = push(&fixture, fixture.replacement(QvmPickupOutcome::Refused)).unwrap();
        let mut refused = fixture.call(8, words.clone(), return_pc(10));
        let error = fixture.pickups.borrow().test_grant(&mut refused, 0).unwrap_err();
        assert!(format!("{error:?}").contains("cancelled"));
        fixture.pickups.borrow().test_pop_frame(id, watch);

        let (id, watch) = push(&fixture, QvmPickupSelection::Original).unwrap();
        let mut original = fixture.call(8, words, return_pc(10));
        assert_eq!(fixture.pickups.borrow().test_grant(&mut original, 0).unwrap(), 0);
        fixture.pickups.borrow().test_pop_frame(id, watch);
        assert!(fixture.pickups.borrow().take_error().is_none());
    }

    #[test]
    fn region_grant_projects_runs_and_skips() {
        let fixture = fixture();
        *fixture.owns.borrow_mut() = true;
        let bits = CLIENTS + 64;
        fixture.memory().write_i32(bits, 0).unwrap();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&seen);
        let memory = fixture.memory();
        let watch = memory.observe_writes(
            vec![QvmWriteRange {
                byte_offset: bits,
                byte_length: 4,
            }],
            Rc::new(move |_| {
                recorded.borrow_mut().push(memory.read_i32(bits).unwrap_or(-1));
            }),
            None,
        );
        let grants = Rc::new(RefCell::new(0));
        let counted = Rc::clone(&grants);
        let held = Rc::new(RefCell::new(None));
        let captured = Rc::clone(&held);
        let wrong = Rc::new(RefCell::new(false));
        let mismatched = Rc::clone(&wrong);
        let pickups = Rc::clone(&fixture.pickups);
        let offer = fixture.offer("q3:quad", 3);
        let probe = offer.clone();
        let selection = QvmPickupSelection::Replacement {
            current: Rc::new(|| true),
            grant: Rc::new(move || {
                *counted.borrow_mut() += 1;
                let supply = pickups.borrow().supply(&probe).unwrap();
                *captured.borrow_mut() = Some(supply.offer.clone());
                let mut foreign = probe.clone();
                foreign.item = "q3:other".to_string();
                *mismatched.borrow_mut() = pickups.borrow().supply(&foreign).is_err();
                QvmPickupOutcome::Accepted
            }),
        };
        let (id, frame_watch) = fixture
            .pickups
            .borrow()
            .test_push_frame(
                offer,
                fixture.owned(3),
                fixture.owned(RECIPIENT_SLOT),
                3,
                RECIPIENT_SLOT,
                fixture.records[1].clone(),
                Some(1),
                selection,
            )
            .unwrap();
        let mut call = fixture.call(13, vec![item_pointer(3), item_pointer(RECIPIENT_SLOT)], return_pc(15));
        assert_eq!(fixture.pickups.borrow().test_grant(&mut call, 1).unwrap(), 0);
        assert_eq!(call.region_bindings.len(), 1);
        assert_eq!(call.region_bindings[0].entry, 16);
        assert_eq!(call.region_bindings[0].join, 18);
        assert_eq!(*seen.borrow(), vec![2, 0]);
        assert_eq!(fixture.memory().read_i32(bits).unwrap(), 0);
        let mut binding = call.region_bindings.pop().unwrap();
        let mut control = QvmRegionControl::default();
        control.locals.insert(8, 25);
        assert_eq!((binding.run)(&mut control), QvmRegionDecision::Skip);
        assert_eq!(*grants.borrow(), 1);
        assert_eq!(
            *held.borrow(),
            Some(QvmPickupSupplyOffer::Weapon {
                item: "q3:quad".to_string(),
                ammo: Vec::new(),
            })
        );
        assert!(*wrong.borrow());
        assert_eq!((binding.run)(&mut control), QvmRegionDecision::Skip);
        assert_eq!(*grants.borrow(), 1);
        assert!(fixture.projects.borrow().is_empty());
        fixture.memory().remove_observer(watch);
        fixture.pickups.borrow().test_pop_frame(id, frame_watch);
        assert!(fixture.pickups.borrow().take_error().is_none());
    }

    #[test]
    fn quantity_projects_and_evaluates() {
        let fixture = fixture();
        let ammo = CLIENTS + 128 + 4;
        fixture.memory().write_i32(ammo, 9).unwrap();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&seen);
        let memory = fixture.memory();
        let watch = memory.observe_writes(
            vec![QvmWriteRange {
                byte_offset: ammo,
                byte_length: 4,
            }],
            Rc::new(move |_| {
                recorded.borrow_mut().push(memory.read_i32(ammo).unwrap_or(-1));
            }),
            None,
        );
        let results = Rc::new(RefCell::new(Vec::new()));
        let noted = Rc::clone(&results);
        let pickups = Rc::clone(&fixture.pickups);
        let offer = fixture.offer("q3:quad", 3);
        let probe = offer.clone();
        let selection = QvmPickupSelection::Replacement {
            current: Rc::new(|| true),
            grant: Rc::new(move || {
                let supply = pickups.borrow().supply(&probe).unwrap();
                let quantity = supply.quantity.clone().unwrap();
                noted.borrow_mut().push(quantity(5.0).unwrap());
                noted
                    .borrow_mut()
                    .push(if quantity(f64::NAN).is_err() { -101 } else { 0 });
                noted.borrow_mut().push(if quantity(1e20).is_err() { -201 } else { 0 });
                QvmPickupOutcome::Accepted
            }),
        };
        let (id, frame_watch) = fixture
            .pickups
            .borrow()
            .test_push_frame(
                offer,
                fixture.owned(3),
                fixture.owned(RECIPIENT_SLOT),
                3,
                RECIPIENT_SLOT,
                fixture.records[1].clone(),
                Some(1),
                selection,
            )
            .unwrap();
        let mut call = fixture.call(13, vec![item_pointer(3), item_pointer(RECIPIENT_SLOT)], return_pc(15));
        fixture.pickups.borrow().test_grant(&mut call, 1).unwrap();
        let mut binding = call.region_bindings.pop().unwrap();
        let mut control = QvmRegionControl::default();
        control.locals.insert(8, 25);
        assert_eq!((binding.run)(&mut control), QvmRegionDecision::Skip);
        assert_eq!(*results.borrow(), vec![0, -101, -201]);
        assert_eq!(*seen.borrow(), vec![5, 9]);
        assert_eq!(fixture.memory().read_i32(ammo).unwrap(), 9);
        let calls = fixture.module.calls();
        assert_eq!(calls.last().unwrap().entry, 13);
        assert_eq!(
            calls.last().unwrap().words,
            vec![item_pointer(3), item_pointer(RECIPIENT_SLOT)]
        );
        fixture.memory().remove_observer(watch);
        fixture.pickups.borrow().test_pop_frame(id, frame_watch);
        assert!(fixture.pickups.borrow().take_error().is_none());
    }

    #[test]
    fn inventory_storage_requires_declared_projection() {
        let fixture = fixture();
        let mut code = instructions();
        for (index, (opcode, operand)) in [
            (QvmOpcode::OpEnter, 0),
            (QvmOpcode::OpConst, 23),
            (QvmOpcode::OpCall, 0),
            (QvmOpcode::OpConst, 0),
            (QvmOpcode::OpPop, 0),
            (QvmOpcode::OpIgnore, 0),
            (QvmOpcode::OpLeave, 0),
        ]
        .into_iter()
        .enumerate()
        {
            code.push(QvmInstruction::word(opcode, operand, (23 + index) * 8));
        }
        let supply = QvmPickupSupplyBridge {
            ammo: Rc::new(|_| None),
            owns: Rc::new(|_, _| false),
            project: None,
        };
        let pickups = try_build(
            &fixture,
            |profile, artifact| {
                artifact.image.instructions = code;
                profile.grants.push(QvmPickupGrant {
                    item_type: 9,
                    entry: 23,
                    calls: vec![25],
                    operation: QvmGrantOperation::Region {
                        entry: 26,
                        join: 28,
                        quantity: 8,
                        weapon: Some(QvmGrantWeapon {
                            quantity: QvmRegionEvaluation {
                                entry: 26,
                                join: 28,
                                inputs: Vec::new(),
                                result: None,
                            },
                            storage: QvmGrantWeaponStorage::Inventory,
                        }),
                    },
                    eligible: Rc::new(|_| Ok(true)),
                });
            },
            Some(supply),
        )
        .unwrap();
        let (id, watch) = pickups
            .test_push_frame(
                fixture.offer("q3:quad", 3),
                fixture.owned(3),
                fixture.owned(RECIPIENT_SLOT),
                3,
                RECIPIENT_SLOT,
                fixture.records[1].clone(),
                Some(2),
                fixture.replacement(QvmPickupOutcome::Accepted),
            )
            .unwrap();
        let mut call = fixture.call(23, vec![item_pointer(3), item_pointer(RECIPIENT_SLOT)], return_pc(25));
        let error = pickups.test_grant(&mut call, 2).unwrap_err();
        assert!(format!("{error:?}").contains("declared inventory projection"));
        pickups.test_pop_frame(id, watch);
    }

    #[test]
    fn targets_and_free_lifetime() {
        let fixture = fixture();
        let (id, watch) = fixture
            .pickups
            .borrow()
            .test_push_frame(
                fixture.offer("q3:shotgun", ITEM_SLOT),
                fixture.owned(ITEM_SLOT),
                fixture.owned(RECIPIENT_SLOT),
                ITEM_SLOT,
                RECIPIENT_SLOT,
                fixture.records[0].clone(),
                Some(0),
                fixture.replacement(QvmPickupOutcome::Accepted),
            )
            .unwrap();
        let mut call = fixture.call(5, vec![0, 0], return_pc(7));
        assert_eq!(fixture.pickups.borrow().test_targets(&mut call).unwrap(), 0);
        fixture.set_entity_word(ITEM_SLOT, 0, 0);
        let mut dead = fixture.call(5, vec![0, 0], return_pc(7));
        assert!(format!("{:?}", fixture.pickups.borrow().test_targets(&mut dead).unwrap_err()).contains("cancelled"));
        fixture.set_entity_word(ITEM_SLOT, 0, 1);
        fixture.pickups.borrow().test_pop_frame(id, watch);

        fixture.module.call(&[item_pointer(ITEM_SLOT)], 1).unwrap();
        assert!(fixture.retired.borrow().is_empty());
        fixture.set_entity_word(ITEM_SLOT, 0, 0);
        fixture.module.call(&[item_pointer(ITEM_SLOT)], 1).unwrap();
        assert_eq!(fixture.retired.borrow().as_slice(), &[fixture.actor(ITEM_SLOT)]);
        fixture.set_entity_word(ITEM_SLOT, 0, 1);
        assert!(fixture.pickups.borrow().take_error().is_none());

        let (id, watch) = fixture
            .pickups
            .borrow()
            .test_push_frame(
                fixture.offer("q3:shotgun", ITEM_SLOT),
                fixture.owned(ITEM_SLOT),
                fixture.owned(RECIPIENT_SLOT),
                ITEM_SLOT,
                RECIPIENT_SLOT,
                fixture.records[0].clone(),
                Some(0),
                fixture.replacement(QvmPickupOutcome::Accepted),
            )
            .unwrap();
        let mut control = fixture.call(1, vec![item_pointer(ITEM_SLOT)], 0);
        assert!(fixture
            .pickups
            .borrow()
            .after_free(item_pointer(ITEM_SLOT), &mut control)
            .is_ok());
        fixture.set_entity_word(ITEM_SLOT, 0, 0);
        let mut control = fixture.call(1, vec![item_pointer(ITEM_SLOT)], 0);
        assert!(format!(
            "{:?}",
            fixture
                .pickups
                .borrow()
                .after_free(item_pointer(ITEM_SLOT), &mut control)
                .unwrap_err()
        )
        .contains("cancelled"));
        fixture.set_entity_word(ITEM_SLOT, 0, 1);
        fixture.pickups.borrow().test_pop_frame(id, watch);
    }

    #[test]
    fn supply_close_and_idle() {
        let fixture = fixture();
        let (id, watch) = fixture
            .pickups
            .borrow()
            .test_push_frame(
                fixture.offer("q3:shotgun", ITEM_SLOT),
                fixture.owned(ITEM_SLOT),
                fixture.owned(RECIPIENT_SLOT),
                ITEM_SLOT,
                RECIPIENT_SLOT,
                fixture.records[0].clone(),
                Some(0),
                fixture.replacement(QvmPickupOutcome::Accepted),
            )
            .unwrap();
        assert!(fixture
            .pickups
            .borrow()
            .supply(&fixture.offer("q3:shotgun", ITEM_SLOT))
            .is_err());
        assert!(fixture.pickups.borrow().assert_idle().is_err());
        fixture.pickups.borrow().test_pop_frame(id, watch);
        fixture.pickups.borrow().assert_idle().unwrap();
        fixture.pickups.borrow_mut().close();
        fixture
            .module
            .call(&[item_pointer(ITEM_SLOT), item_pointer(RECIPIENT_SLOT)], 0)
            .unwrap();
        assert!(fixture.offers.borrow().is_empty());
    }

    #[test]
    fn live_catalog_owner() {
        let fixture = fixture();
        let records = fixture.records.clone();
        let mut profile = fixture.profile.clone();
        profile.items.live_source = true;
        let pickups = QvmPrimaryPickups::new(QvmPickupOptions {
            game: QvmPickupGame {
                module: fixture.module.clone(),
                data: fixture.data.clone(),
            },
            artifact: fixture.artifact.clone(),
            profile,
            catalog: Some(Rc::new(move || records.clone())),
            actor: Rc::new({
                let owned = fixture.owned.clone();
                move |slot| owned.iter().find(|owned| owned.id().slot() == slot as u32).cloned()
            }),
            current: Rc::new(|_, _| true),
            resolve_item: Rc::new(|_| QvmResolvedItem {
                item: "q3:shells".to_string(),
                resource: None,
            }),
            time: Rc::new(|| QvmSourceTime {
                kind: QvmTimeKind::Milliseconds,
                value: 0.0,
            }),
            supply: None,
            run_source: Rc::new(|_, execute| execute(QvmPickupSelection::Blocked)),
            lifetime: QvmPickupLifetime::SharedFreeHook,
        })
        .unwrap();
        let _ = pickups;
    }
}
