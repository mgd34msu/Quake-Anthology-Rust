//! Quake III base/game: item lifecycle.
//!
//! Donor provenance: `src/content/q3/base/game/item-lifecycle.ts`.

use qa_core::identity::ActorId;
use qa_core::math::{vec3, Vec3};
use qa_core::numeric::qvm_float_to_int;
use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::death::*;
use crate::q3::base::game::entities::*;
use crate::q3::base::game::format::*;
use crate::q3::base::game::ground::*;
use crate::q3::base::game::hitscan::*;
use crate::q3::base::game::mirrors_game_sim::*;

// ---------------------------------------------------------------------------
// Item lifecycle (item-lifecycle.ts).
// ---------------------------------------------------------------------------

/// Items configstring index (`CS_ITEMS`).
pub(crate) const CS_ITEMS: i32 = 27;

/// Item bounding radius (`ITEM_RADIUS`).
pub(crate) const ITEM_RADIUS: f32 = 15.0;

/// Server frame time milliseconds (`FRAME_TIME`).
pub(crate) const FRAME_TIME: i32 = 100;

/// Ammo respawn seconds (`RESPAWN_AMMO`, item-pickup.ts).
pub const RESPAWN_AMMO: i32 = 40;

/// Armor respawn seconds (`RESPAWN_ARMOR`, item-pickup.ts).
pub(crate) const RESPAWN_ARMOR: i32 = 25;

/// Health respawn seconds (`RESPAWN_HEALTH`, item-pickup.ts).
pub(crate) const RESPAWN_HEALTH: i32 = 35;

/// Holdable respawn seconds (`RESPAWN_HOLDABLE`, item-pickup.ts).
pub(crate) const RESPAWN_HOLDABLE: i32 = 60;

/// Mega-health respawn seconds (`RESPAWN_MEGAHEALTH`, item-pickup.ts).
pub(crate) const RESPAWN_MEGAHEALTH: i32 = 35;

/// Powerup respawn seconds (`RESPAWN_POWERUP`, item-pickup.ts).
pub(crate) const RESPAWN_POWERUP: i32 = 120;

/// Per-level item registration (`ItemRegistry`).
pub struct ItemRegistry {
    /// Product.
    pub product: Q3Product,
    /// Product table.
    pub table: Rc<Q3ItemTable>,
    /// Registration flags.
    registered: Vec<u8>,
}

impl ItemRegistry {
    /// Empty registry for a product table (`new ItemRegistry(product)`).
    #[must_use]
    pub fn new(product: Q3Product, table: Rc<Q3ItemTable>) -> Self {
        let len = table.len();
        Self {
            product,
            table,
            registered: vec![0; len],
        }
    }

    /// Capture registration flags (`captureSaveState`).
    #[must_use]
    pub fn capture_save_state(&self) -> Vec<u8> {
        self.registered.clone()
    }

    /// Restore registration flags (`restoreSaveState`).
    pub fn restore_save_state(&mut self, bytes: &[u8]) {
        if bytes.len() != self.registered.len() || bytes.iter().any(|byte| *byte != 0 && *byte != 1) {
            panic!("invalid registered item table");
        }
        self.registered.copy_from_slice(bytes);
    }

    /// Register an item (`register`).
    pub fn register(&mut self, item: &ItemDefinition) {
        let Some(index) = self.table.index_of(item) else {
            panic!("Registered item does not belong to its table");
        };
        self.registered[index] = 1;
    }

    /// Registration test (`isRegistered`).
    #[must_use]
    pub fn is_registered(&self, item: &ItemDefinition) -> bool {
        let Some(index) = self.table.index_of(item) else {
            panic!("Registered item does not belong to its table");
        };
        self.registered[index] != 0
    }

    /// Clear and register always-present items (`clear`).
    pub fn clear(&mut self, game_type: i32) {
        self.registered.fill(0);
        let machinegun = self.table.find_item_for_weapon(Weapon::Machinegun as i32).clone();
        self.register(&machinegun);
        let gauntlet = self.table.find_item_for_weapon(Weapon::Gauntlet as i32).clone();
        self.register(&gauntlet);
        if self.product != Q3Product::Missionpack || game_type != GameType::Harvester as i32 {
            return;
        }
        for pickup_name in ["Red Cube", "Blue Cube"] {
            let item = self.table.find_item(pickup_name).cloned().unwrap_or_else(|| {
                panic!("Required item {pickup_name} is missing");
            });
            self.register(&item);
        }
    }

    /// Publish the registration configstring (`save`).
    pub fn save(&self, set_configstring: &dyn Fn(i32, String), log: &dyn Fn(String)) -> i32 {
        let mut value = String::new();
        let mut count = 0;
        for registered in &self.registered {
            if *registered != 0 {
                count += 1;
                value.push('1');
            } else {
                value.push('0');
            }
        }
        log(format!("{count} items registered\n"));
        set_configstring(CS_ITEMS, value);
        count
    }
}

/// Ammo grant (`PickupAmmoGrant`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupAmmoGrant {
    /// Ammo item.
    pub item: ItemId,
    /// Amount.
    pub amount: i32,
}

/// Supply offer (`PickupSupplyOffer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickupSupplyOffer {
    /// Ammo offer.
    Ammo {
        /// Grant.
        offer: PickupAmmoGrant,
    },
    /// Ammo-plus-weapon offer.
    AmmoWeapon {
        /// Grant.
        offer: PickupAmmoGrant,
        /// Weapon item.
        weapon: ItemId,
    },
    /// Weapon offer.
    Weapon {
        /// Weapon item.
        item: ItemId,
        /// Ammo grants.
        ammo: Vec<PickupAmmoGrant>,
    },
}

/// Supply availability (`PickupSupplyObservation.availability`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SupplyAvailability {
    /// Ready for pickup.
    Ready {
        /// Recipient eligible.
        eligible: bool,
    },
    /// Respawning at source seconds.
    Respawning {
        /// Respawn time seconds.
        at_seconds: f32,
    },
    /// Inactive.
    Inactive,
}

/// Supply observation (`PickupSupplyObservation`).
#[derive(Debug, Clone, PartialEq)]
pub struct PickupSupplyObservation {
    /// Pickup actor.
    pub actor: ActorId,
    /// Offer.
    pub offer: PickupSupplyOffer,
    /// Availability.
    pub availability: SupplyAvailability,
}

/// Ammo receipt (`PickupAmmoReceipt`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupAmmoReceipt {
    /// Item.
    pub item: ItemId,
    /// Count before.
    pub before: i32,
    /// Count given.
    pub given: i32,
}

/// Supply preview (`PickupSupplyPreview`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupSupplyPreview {
    /// Accepted.
    pub accepted: bool,
    /// Ammo receipts.
    pub ammo: Vec<PickupAmmoReceipt>,
    /// Weapon receipts.
    pub weapons: Vec<PickupAmmoReceipt>,
}

/// Protection channel (`ProtectionChannel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionChannel {
    /// Regular armor.
    Regular,
    /// Powered protection.
    Powered,
}

/// Pickup resource (`PickupResource`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickupResource {
    /// Protection write.
    Protection {
        /// Channel.
        channel: ProtectionChannel,
    },
    /// Inventory write.
    Inventory {
        /// Item.
        item: ItemId,
    },
}

/// Pickup count (`PickupCount`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupCount {
    /// Default quantity.
    Default,
    /// Override amount.
    Override {
        /// Amount.
        amount: i32,
    },
}

/// Pickup grant kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupGrant {
    /// Map-coupled grant.
    MapCoupled,
    /// Source-effect grant.
    SourceEffect,
}

/// Original pickup offer (`OriginalPickupOffer`).
#[derive(Debug, Clone, PartialEq)]
pub struct OriginalPickupOffer {
    /// Recipient actor.
    pub recipient: ActorId,
    /// Pickup actor.
    pub pickup: ActorId,
    /// Source provider.
    pub source: ProviderId,
    /// Offered item.
    pub item: ItemId,
    /// Default resource.
    pub default_resource: Option<PickupResource>,
    /// Count.
    pub count: PickupCount,
    /// Dropped item.
    pub dropped: bool,
    /// Source time.
    pub time: SourceTime,
    /// Grant kind.
    pub grant: Option<PickupGrant>,
}

/// Original pickup continuation (`OriginalPickupContinuation`).
#[derive(Clone)]
pub struct OriginalContinuation {
    /// Eligibility probe.
    pub eligible: Option<Rc<dyn Fn() -> bool>>,
    /// Original pickup logic.
    pub original: Rc<dyn Fn() -> bool>,
    /// Completion logic.
    pub complete: Rc<dyn Fn(bool)>,
}

/// Original pickup outcome (`OriginalPickupOutcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginalPickupOutcome {
    /// Accepted.
    Accepted,
    /// Refused.
    Refused,
    /// Stale.
    Stale,
}

/// Original pickup admission (`OriginalPickupAdmission`).
#[derive(Clone)]
pub struct OriginalPickupAdmission {
    /// Touch dispatch.
    pub touch: Rc<dyn Fn(OriginalPickupOffer, OriginalContinuation) -> OriginalPickupOutcome>,
}

/// Source pickup descriptor (`SourcePickupDescriptor`).
#[derive(Debug, Clone, PartialEq)]
pub struct SourcePickupDescriptor {
    /// Item actor.
    pub item_actor: ActorId,
    /// Player actor.
    pub player_actor: ActorId,
    /// Item definition.
    pub item: ItemDefinition,
    /// Count override.
    pub count: i32,
    /// Generic 1.
    pub generic1: i32,
    /// Dropped item.
    pub dropped: bool,
    /// Game type.
    pub game_type: i32,
    /// Weapon respawn seconds.
    pub weapon_respawn_seconds: i32,
    /// Team weapon respawn seconds.
    pub team_weapon_respawn_seconds: i32,
}

/// Source pickup admission (`SourcePickupAdmission`).
#[derive(Debug, Clone, PartialEq)]
pub enum SourcePickupAdmission {
    /// Native handling.
    Native,
    /// Rejected.
    Rejected,
    /// Picked with a respawn interval.
    Picked {
        /// Respawn seconds.
        respawn_seconds: i32,
    },
}

/// Source pickup preview (`SourcePickupPreview`).
#[derive(Debug, Clone, PartialEq)]
pub enum SourcePickupPreview {
    /// Native handling.
    Native,
    /// Rejected.
    Rejected,
    /// Selected supply offer.
    Selected {
        /// Offer.
        offer: PickupSupplyOffer,
        /// Preview.
        preview: PickupSupplyPreview,
    },
}

/// Item lifecycle callbacks.
#[derive(Clone)]
pub struct ItemLifecycleCallbacks {
    /// Item touch callback.
    pub touch: TouchCallback,
    /// Item respawn think callback.
    pub respawn: ThinkCallback,
}

/// Weapon pickup context (`WeaponPickupContext`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponPickupContext {
    /// Game type.
    pub game_type: i32,
    /// Weapon respawn seconds.
    pub weapon_respawn_seconds: i32,
    /// Team weapon respawn seconds.
    pub team_weapon_respawn_seconds: i32,
}

/// Weapon respawn seconds (`q3WeaponRespawnSeconds`, item-pickup.ts).
#[must_use]
pub fn q3_weapon_respawn_seconds(context: &WeaponPickupContext) -> i32 {
    if context.game_type == GameType::Team as i32 {
        context.team_weapon_respawn_seconds
    } else {
        context.weapon_respawn_seconds
    }
}

/// Item respawn seconds (`q3ItemRespawnSeconds`, item-pickup.ts).
#[must_use]
pub fn q3_item_respawn_seconds(item: &ItemDefinition, context: &WeaponPickupContext) -> i32 {
    match item.item_type {
        ItemType::Weapon => q3_weapon_respawn_seconds(context),
        ItemType::Ammo => RESPAWN_AMMO,
        ItemType::Armor => RESPAWN_ARMOR,
        ItemType::Health => {
            if item.quantity == 100 {
                RESPAWN_MEGAHEALTH
            } else {
                RESPAWN_HEALTH
            }
        }
        ItemType::Holdable => RESPAWN_HOLDABLE,
        ItemType::Powerup => RESPAWN_POWERUP,
        ItemType::PersistantPowerup => -1,
        ItemType::Team => {
            panic!("Team objective lifecycle requires its original pickup handler");
        }
        ItemType::Bad => panic!("Invalid item has no pickup lifecycle"),
    }
}

/// Item pickup context (`ItemPickupContext`).
#[derive(Clone)]
pub struct ItemPickupContextMirror {
    /// Game type.
    pub game_type: i32,
    /// Weapon respawn seconds.
    pub weapon_respawn_seconds: i32,
    /// Team weapon respawn seconds.
    pub team_weapon_respawn_seconds: i32,
    /// Time milliseconds.
    pub time: i32,
    /// Client records.
    pub clients: Vec<GameClient>,
    /// Solid sight-line trace.
    pub trace_solid_line: Rc<dyn Fn(Vec3, Vec3) -> f32>,
    /// Handicap reader.
    pub handicap_for_client: Rc<dyn Fn(i32) -> String>,
}

/// Item lifecycle context (`ItemLifecycleContext`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct ItemLifecycleContext {
    /// Original pickup admission.
    pub original_pickups: Option<OriginalPickupAdmission>,
    /// Item callbacks.
    pub callbacks: Option<ItemLifecycleCallbacks>,
    /// Supply preview hook.
    pub preview_pickup: Option<Rc<dyn Fn(&SourcePickupDescriptor) -> SourcePickupPreview>>,
    /// Pickup admission hook.
    pub admit_pickup: Option<Rc<dyn Fn(&SourcePickupDescriptor) -> SourcePickupAdmission>>,
    /// Entity pool.
    pub entities: PoolHandle,
    /// Server world.
    pub world: ServerWorldMirror,
    /// Product.
    pub product: Q3Product,
    /// Game type.
    pub game_type: i32,
    /// Weapon respawn seconds.
    pub weapon_respawn_seconds: i32,
    /// Team weapon respawn seconds.
    pub team_weapon_respawn_seconds: i32,
    /// Handicap reader.
    pub handicap_for_client: Rc<dyn Fn(i32) -> String>,
    /// Team pickup handler.
    pub team_pickup: Rc<dyn Fn(EntityRef, EntityRef) -> i32>,
    /// Target dispatcher.
    pub use_targets: Rc<dyn Fn(EntityRef, EntityRef)>,
    /// Sound indexer.
    pub sound_index: Rc<dyn Fn(&str) -> i32>,
    /// Game random.
    pub random: Rc<RefCell<GameRandomMirror>>,
    /// Item registry.
    pub registry: Rc<RefCell<ItemRegistry>>,
    /// Log sink.
    pub log: Rc<dyn Fn(String)>,
    /// Warning sink.
    pub warn: Rc<dyn Fn(String)>,
    /// Item table.
    pub item_table: Rc<Q3ItemTable>,
    /// Native pickup rules (`pickupItem`, item-pickup.ts).
    pub pickup_item: Rc<dyn Fn(EntityRef, EntityRef, &ItemPickupContextMirror) -> i32>,
}

/// Lifecycle time milliseconds (`gameTime`).
pub(crate) fn lifecycle_time(context: &ItemLifecycleContext) -> i32 {
    (context.entities.borrow().options.time)()
}

/// Product consistency check (`checkContext`).
pub(crate) fn check_lifecycle_context(context: &ItemLifecycleContext) {
    if context.entities.borrow().options.product != context.product {
        panic!("Item lifecycle product does not match its entity pool and registry");
    }
}

/// Pool ownership check (`requireOwned`).
pub(crate) fn require_lifecycle_owned(context: &ItemLifecycleContext, entity: &EntityRef) {
    let slot = entity.borrow().slot;
    let owned = context
        .entities
        .borrow()
        .get(slot as i32)
        .is_some_and(|record| Rc::ptr_eq(&record, entity));
    if !owned {
        panic!("Item lifecycle entity does not belong to its entity pool or was replaced");
    }
}

/// Item definition check (`requireItem`).
pub(crate) fn require_lifecycle_item(context: &ItemLifecycleContext, entity: &EntityRef) -> ItemDefinition {
    let item = entity.borrow().item.clone();
    match item {
        Some(item) if context.item_table.index_of(&item).is_some_and(|index| index >= 1) => item,
        _ => panic!("Item entity does not contain a product item definition"),
    }
}

/// Table index check (`tableIndex`).
pub(crate) fn lifecycle_table_index(context: &ItemLifecycleContext, item: &ItemDefinition) -> i32 {
    match context.item_table.index_of(item) {
        Some(index) if index >= 1 => index as i32,
        _ => panic!("Item does not belong to the product item table"),
    }
}

/// Published item check (`requirePublishedItem`).
pub(crate) fn require_published_item(context: &ItemLifecycleContext, entity: &EntityRef) -> ItemDefinition {
    let item = require_lifecycle_item(context, entity);
    if entity.borrow().s.modelindex != lifecycle_table_index(context, &item) {
        panic!("Item entity model index does not match its item definition");
    }
    item
}

/// Validated unit random draw (`unitRandom`).
pub(crate) fn lifecycle_unit_random(random: &Rc<RefCell<GameRandomMirror>>) -> f32 {
    let value = random.borrow_mut().random_value();
    if !value.is_finite() || value < 0.0 || value > 1.0 {
        panic!("Game random() must return a value within [0, 1]");
    }
    value
}

/// Centered random draw (`crandom`).
pub(crate) fn lifecycle_crandom(random: &Rc<RefCell<GameRandomMirror>>) -> f32 {
    2.0 * (lifecycle_unit_random(random) - 0.5)
}

/// Nonnegative random integer (`randomInteger`).
pub(crate) fn lifecycle_random_integer(random: &Rc<RefCell<GameRandomMirror>>) -> i32 {
    let value = random.borrow_mut().rand_value();
    if value < 0 {
        panic!("Game rand() must return a nonnegative safe integer");
    }
    value
}

/// Pickup context for native rules (`pickupContext`).
pub(crate) fn lifecycle_pickup_context(context: &ItemLifecycleContext, time: i32) -> ItemPickupContextMirror {
    let max_clients = context.entities.borrow().max_clients();
    let mut clients = Vec::with_capacity(max_clients);
    for index in 0..max_clients {
        clients.push(context.entities.borrow().client_at(index as i32));
    }
    let world = context.world.clone();
    ItemPickupContextMirror {
        game_type: context.game_type,
        weapon_respawn_seconds: context.weapon_respawn_seconds,
        team_weapon_respawn_seconds: context.team_weapon_respawn_seconds,
        time,
        clients,
        trace_solid_line: Rc::new(move |start: Vec3, end: Vec3| {
            (world.trace)(&ServerTraceQuery {
                start,
                end,
                shape: TraceShape::Point,
                pass_entity_num: ENTITYNUM_NONE,
                mask: CONTENTS_SOLID,
            })
            .fraction
        }),
        handicap_for_client: context.handicap_for_client.clone(),
    }
}

/// Random team member (`teamMember`).
pub(crate) fn lifecycle_team_member(context: &ItemLifecycleContext, entity: &EntityRef) -> EntityRef {
    if entity.borrow().team.is_none() {
        return entity.clone();
    }
    let master = entity.borrow().teammaster.clone();
    let Some(master) = master else {
        panic!("RespawnItem: bad teammaster");
    };
    let mut count = 0;
    let mut cursor: Option<EntityRef> = Some(master.clone());
    while let Some(current) = cursor.clone() {
        require_lifecycle_owned(context, &current);
        count += 1;
        if count > context.entities.borrow().num_entities() as i32 {
            panic!("RespawnItem: cyclic teamchain");
        }
        cursor = current.borrow().teamchain.clone();
    }
    let choice = lifecycle_random_integer(&context.random) % count;
    let mut cursor: Option<EntityRef> = Some(master);
    for _ in 0..choice {
        cursor = match cursor {
            Some(current) => current.borrow().teamchain.clone(),
            None => panic!("RespawnItem: broken teamchain"),
        };
    }
    cursor.unwrap_or_else(|| panic!("RespawnItem: broken teamchain"))
}

/// Respawn sound event (`spawnRespawnSound`).
pub(crate) fn lifecycle_spawn_respawn_sound(context: &ItemLifecycleContext, entity: &EntityRef, path: &str) {
    let event = if entity.borrow().speed != 0.0 {
        EntityEvent::GeneralSound as i32
    } else {
        EntityEvent::GlobalSound as i32
    };
    let origin = entity.borrow().s.pos.base;
    let temporary = context.entities.borrow_mut().temp_entity(origin, event);
    temporary.borrow_mut().s.event_parm = (context.sound_index)(path);
    temporary.borrow_mut().r.sv_flags |= server_entity_flags::BROADCAST;
}

/// Respawn an item (`respawnItem`, `RespawnItem`).
pub fn respawn_item(entity: &EntityRef, context: &ItemLifecycleContext) {
    check_lifecycle_context(context);
    require_lifecycle_owned(context, entity);
    let selected = lifecycle_team_member(context, entity);
    let item = require_published_item(context, &selected);
    {
        let mut borrowed = selected.borrow_mut();
        borrowed.r.contents = CONTENTS_TRIGGER;
        borrowed.s.e_flags &= !EF_NODRAW;
        borrowed.r.sv_flags &= !server_entity_flags::NOCLIENT;
    }
    (context.entities.borrow().options.link)(selected.clone());
    if item.item_type == ItemType::Powerup {
        lifecycle_spawn_respawn_sound(context, &selected, "sound/items/poweruprespawn.wav");
    }
    if item.item_type == ItemType::Holdable && item.tag == Holdable::Kamikaze as i32 {
        lifecycle_spawn_respawn_sound(context, &selected, "sound/items/kamikazerespawn.wav");
    }
    context
        .entities
        .borrow()
        .add_event(&selected, EntityEvent::ItemRespawn as i32, 0);
    selected.borrow_mut().nextthink = 0;
}

/// Source pickup descriptor (`pickupDescriptor`).
pub(crate) fn pickup_descriptor(
    entity: &EntityRef,
    recipient: &EntityRef,
    item: &ItemDefinition,
    context: &ItemLifecycleContext,
) -> SourcePickupDescriptor {
    let borrowed = entity.borrow();
    SourcePickupDescriptor {
        item_actor: borrowed.actor.id.clone(),
        player_actor: recipient.borrow().actor.id.clone(),
        item: item.clone(),
        count: borrowed.count,
        generic1: borrowed.s.generic1,
        dropped: (borrowed.flags & game_flags::DROPPED_ITEM) != 0,
        game_type: context.game_type,
        weapon_respawn_seconds: context.weapon_respawn_seconds,
        team_weapon_respawn_seconds: context.team_weapon_respawn_seconds,
    }
}

/// Supply observation result (`observeQ3Supply`).
#[derive(Debug, Clone, PartialEq)]
pub struct SupplyObservation {
    /// Observation.
    pub observation: PickupSupplyObservation,
    /// Preview.
    pub preview: PickupSupplyPreview,
}

/// Supply observation for weapon/ammo items (`observeQ3Supply`).
#[must_use]
pub fn observe_q3_supply(
    entity: &EntityRef,
    recipient: &EntityRef,
    context: &ItemLifecycleContext,
) -> Option<SupplyObservation> {
    let pool = context.entities.borrow();
    let owned = |record: &EntityRef| -> bool {
        pool.get(record.borrow().slot as i32)
            .is_some_and(|known| Rc::ptr_eq(&known, record))
    };
    let callbacks = context.callbacks.as_ref()?;
    if context.product != Q3Product::Baseq3
        || !entity.borrow().inuse
        || !recipient.borrow().inuse
        || !owned(entity)
        || !owned(recipient)
        || recipient.borrow().client.is_none()
    {
        return None;
    }
    let touch_matches = match (entity.borrow().touch.clone(), Some(callbacks.touch.clone())) {
        (Some(left), Some(right)) => Rc::ptr_eq(&left, &right),
        _ => false,
    };
    if !touch_matches {
        return None;
    }
    let item_type = entity.borrow().item.as_ref().map(|item| item.item_type);
    if !matches!(item_type, Some(ItemType::Weapon | ItemType::Ammo)) {
        return None;
    }
    drop(pool);
    let item = require_published_item(context, entity);
    let supplied = context
        .preview_pickup
        .as_ref()
        .map(|preview| preview(&pickup_descriptor(entity, recipient, &item, context)))?;
    let SourcePickupPreview::Selected { offer, preview } = supplied else {
        return None;
    };
    let borrowed = entity.borrow();
    let ready = (borrowed.r.contents & CONTENTS_TRIGGER) != 0
        && (borrowed.s.e_flags & EF_NODRAW) == 0
        && (borrowed.r.sv_flags & server_entity_flags::NOCLIENT) == 0
        && !borrowed.free_after_event
        && !borrowed.unlink_after_event;
    let respawn_matches = match (borrowed.think.clone(), Some(callbacks.respawn.clone())) {
        (Some(left), Some(right)) => Rc::ptr_eq(&left, &right),
        _ => false,
    };
    let respawning = borrowed.team.is_none()
        && borrowed.teammaster.is_none()
        && borrowed.teamchain.is_none()
        && (borrowed.flags & (game_flags::DROPPED_ITEM | game_flags::TEAMSLAVE)) == 0
        && !borrowed.free_after_event
        && !borrowed.unlink_after_event
        && respawn_matches
        && borrowed.nextthink > 0;
    let nextthink = borrowed.nextthink;
    let actor = borrowed.actor.id.clone();
    drop(borrowed);
    let availability = if ready {
        SupplyAvailability::Ready {
            eligible: recipient.borrow().health >= 1,
        }
    } else if respawning {
        SupplyAvailability::Respawning {
            at_seconds: nextthink as f32 / 1000.0,
        }
    } else {
        SupplyAvailability::Inactive
    };
    Some(SupplyObservation {
        observation: PickupSupplyObservation {
            actor,
            offer,
            availability,
        },
        preview,
    })
}

/// Per-touch attempt state shared by the original/complete closures.
pub(crate) struct TouchAttempt {
    /// Predict the pickup event.
    predict: bool,
    /// Respawn seconds.
    respawn: i32,
    /// Original logic ran.
    original_ran: bool,
}

/// Item touch (`touchItem`, `Touch_Item`).
pub fn touch_item(
    entity: &EntityRef,
    other: DamageParticipant,
    _contact: &TouchContactMirror,
    context: &ItemLifecycleContext,
) {
    bind_item_save_callbacks(context);
    check_lifecycle_context(context);
    require_lifecycle_owned(context, entity);
    let DamageParticipant::Native(other_entity) = &other else {
        return;
    };
    require_lifecycle_owned(context, other_entity);
    {
        let borrowed = other_entity.borrow();
        let Some(client) = borrowed.client.as_ref() else {
            return;
        };
        if borrowed.health < 1 {
            return;
        }
        if client.ps.product != context.product {
            panic!("Item touch player product does not match lifecycle product");
        }
    }
    let item = require_published_item(context, entity);
    let pickup_state = PickupEntity {
        model_index: entity.borrow().s.modelindex,
        model_index2: entity.borrow().s.modelindex2,
        generic1: entity.borrow().s.generic1,
    };
    let item_actor = entity.borrow().actor.id.clone();
    let player_actor = other_entity.borrow().actor.id.clone();
    let pool = context.entities.clone();
    let entity_slot = entity.borrow().slot;
    let other_slot = other_entity.borrow().slot;
    let entity_handle = entity.clone();
    let other_handle = other_entity.clone();
    let live: Rc<dyn Fn() -> bool> = Rc::new(move || {
        let pool = pool.borrow();
        entity_handle.borrow().inuse
            && other_handle.borrow().inuse
            && pool
                .get(entity_slot as i32)
                .is_some_and(|record| Rc::ptr_eq(&record, &entity_handle))
            && pool
                .get(other_slot as i32)
                .is_some_and(|record| Rc::ptr_eq(&record, &other_handle))
            && entity_handle.borrow().actor.id == item_actor
            && other_handle.borrow().actor.id == player_actor
    });
    let class_name = item.class_name.clone().unwrap_or_else(|| {
        panic!("Pickup item has no classname");
    });
    let now = lifecycle_time(context);
    let predict = other_entity
        .borrow()
        .client
        .as_ref()
        .is_some_and(|client| client.pers.predict_item_pickup);
    let attempt = Rc::new(RefCell::new(TouchAttempt {
        predict,
        respawn: 0,
        original_ran: false,
    }));
    let original: Rc<dyn Fn() -> bool> = {
        let entity = entity.clone();
        let other_entity = other_entity.clone();
        let item = item.clone();
        let context = context.clone();
        let live = live.clone();
        let attempt = attempt.clone();
        let class_name = class_name.clone();
        Rc::new(move || {
            attempt.borrow_mut().original_ran = true;
            let admission = context
                .admit_pickup
                .as_ref()
                .map_or(SourcePickupAdmission::Native, |admit| {
                    admit(&pickup_descriptor(&entity, &other_entity, &item, &context))
                });
            if !live() || matches!(admission, SourcePickupAdmission::Rejected) {
                return false;
            }
            if matches!(admission, SourcePickupAdmission::Native) {
                let inventory = q3_item_inventory(other_entity.borrow().client.as_ref().unwrap_or_else(|| {
                    panic!("Item touch player product does not match lifecycle product");
                }));
                if !context
                    .item_table
                    .can_item_be_grabbed(context.game_type, &pickup_state, &inventory)
                {
                    return false;
                }
            }
            (context.log)(format!("Item: {} {class_name}\n", other_entity.borrow().s.number));
            if !live() {
                return false;
            }
            match &admission {
                SourcePickupAdmission::Picked { respawn_seconds } => {
                    attempt.borrow_mut().respawn = *respawn_seconds;
                }
                SourcePickupAdmission::Native | SourcePickupAdmission::Rejected => {
                    if item.item_type == ItemType::Team {
                        attempt.borrow_mut().respawn = (context.team_pickup)(entity.clone(), other_entity.clone());
                    } else {
                        let pickup = lifecycle_pickup_context(&context, now);
                        attempt.borrow_mut().respawn =
                            (context.pickup_item)(entity.clone(), other_entity.clone(), &pickup);
                        if item.item_type == ItemType::Powerup {
                            attempt.borrow_mut().predict = false;
                        }
                    }
                }
            }
            if attempt.borrow().respawn == 0 || !live() {
                return false;
            }
            if matches!(admission, SourcePickupAdmission::Native) {
                let quantity = if entity.borrow().count != 0 {
                    entity.borrow().count
                } else {
                    item.quantity
                };
                let slot = other_entity.borrow().slot as i32;
                let entities = context.entities.borrow();
                let mut rankings = entities.rankings.borrow_mut();
                match item.item_type {
                    ItemType::Weapon => rankings.pickup_weapon(slot, item.tag),
                    ItemType::Ammo => rankings.pickup_ammo(slot, item.tag, quantity),
                    ItemType::Health => rankings.pickup_health(slot, quantity),
                    ItemType::Armor => rankings.pickup_armor(slot, item.quantity),
                    ItemType::Powerup => rankings.pickup_powerup(slot, item.tag),
                    ItemType::Holdable => rankings.pickup_holdable(slot, item.tag),
                    _ => {}
                }
            }
            true
        })
    };
    let complete: Rc<dyn Fn(bool)> = {
        let entity = entity.clone();
        let other_entity = other_entity.clone();
        let item = item.clone();
        let context = context.clone();
        let live = live.clone();
        let attempt = attempt.clone();
        let class_name = class_name.clone();
        Rc::new(move |taken: bool| {
            if !taken || !live() {
                return;
            }
            if !attempt.borrow().original_ran {
                let respawn = q3_item_respawn_seconds(
                    &item,
                    &WeaponPickupContext {
                        game_type: context.game_type,
                        weapon_respawn_seconds: context.weapon_respawn_seconds,
                        team_weapon_respawn_seconds: context.team_weapon_respawn_seconds,
                    },
                );
                if respawn == 0 {
                    return;
                }
                attempt.borrow_mut().respawn = respawn;
                if item.item_type == ItemType::Powerup {
                    attempt.borrow_mut().predict = false;
                }
                (context.log)(format!("Item: {} {class_name}\n", other_entity.borrow().s.number));
                if !live() {
                    return;
                }
            }
            let modelindex = entity.borrow().s.modelindex;
            if attempt.borrow().predict {
                context.entities.borrow().add_predictable_event(
                    &other_entity,
                    EntityEvent::ItemPickup as i32,
                    modelindex,
                );
            } else {
                context
                    .entities
                    .borrow()
                    .add_event(&other_entity, EntityEvent::ItemPickup as i32, modelindex);
            }
            if !live() {
                return;
            }
            if item.item_type == ItemType::Powerup || item.item_type == ItemType::Team {
                let origin = entity.borrow().s.pos.base;
                let temporary = context
                    .entities
                    .borrow_mut()
                    .temp_entity(origin, EntityEvent::GlobalItemPickup as i32);
                if !live() {
                    return;
                }
                temporary.borrow_mut().s.event_parm = modelindex;
                if entity.borrow().speed == 0.0 {
                    temporary.borrow_mut().r.sv_flags |= server_entity_flags::BROADCAST;
                } else {
                    let number = other_entity.borrow().s.number;
                    let mut borrowed = temporary.borrow_mut();
                    borrowed.r.sv_flags |= server_entity_flags::SINGLECLIENT;
                    borrowed.r.single_client = number;
                }
            }
            (context.use_targets)(entity.clone(), other_entity.clone());
            if !live() {
                return;
            }
            if entity.borrow().wait == -1.0 {
                let mut borrowed = entity.borrow_mut();
                borrowed.r.sv_flags |= server_entity_flags::NOCLIENT;
                borrowed.s.e_flags |= EF_NODRAW;
                borrowed.r.contents = 0;
                borrowed.unlink_after_event = true;
                return;
            }
            if entity.borrow().wait != 0.0 {
                attempt.borrow_mut().respawn = qvm_float_to_int(entity.borrow().wait);
            }
            if entity.borrow().random != 0.0 {
                let spread = entity.borrow().random;
                let respawn =
                    qvm_float_to_int(attempt.borrow().respawn as f32 + lifecycle_crandom(&context.random) * spread);
                attempt.borrow_mut().respawn = respawn;
                if !live() {
                    return;
                }
                if attempt.borrow().respawn < 1 {
                    attempt.borrow_mut().respawn = 1;
                }
            }
            if (entity.borrow().flags & game_flags::DROPPED_ITEM) != 0 {
                entity.borrow_mut().free_after_event = true;
            }
            {
                let mut borrowed = entity.borrow_mut();
                borrowed.r.sv_flags |= server_entity_flags::NOCLIENT;
                borrowed.s.e_flags |= EF_NODRAW;
                borrowed.r.contents = 0;
            }
            let respawn = attempt.borrow().respawn;
            if respawn <= 0 {
                let mut borrowed = entity.borrow_mut();
                borrowed.nextthink = 0;
                borrowed.think = None;
            } else {
                let think = match context.callbacks.as_ref() {
                    Some(callbacks) => callbacks.respawn.clone(),
                    None => context
                        .entities
                        .borrow()
                        .callbacks
                        .borrow()
                        .think
                        .resolve(Some("q3.base.game.item-lifecycle.touchItem.think"))
                        .unwrap_or_else(|| {
                            panic!("Unknown native Q3 callback q3.base.game.item-lifecycle.touchItem.think")
                        }),
                };
                let mut borrowed = entity.borrow_mut();
                borrowed.nextthink = now.wrapping_add(respawn.wrapping_mul(1000));
                borrowed.think = Some(think);
            }
            (context.entities.borrow().options.link)(entity.clone());
        })
    };
    if context.original_pickups.is_none() {
        let taken = original();
        complete(taken);
        return;
    }
    let weapon = if matches!(item.item_type, ItemType::Weapon | ItemType::Ammo) {
        q3_weapon_item(item.tag)
    } else {
        None
    };
    let offered: ItemId = match item.item_type {
        ItemType::Weapon => weapon.map_or(format!("q3:{class_name}"), |entry| entry.item.to_string()),
        ItemType::Ammo => weapon.map_or(format!("q3:{class_name}"), |entry| entry.ammo.unwrap_or("").to_string()),
        _ => format!("q3:{class_name}"),
    };
    let default_resource = match item.item_type {
        ItemType::Armor => Some(PickupResource::Protection {
            channel: ProtectionChannel::Regular,
        }),
        ItemType::Weapon | ItemType::Ammo => Some(PickupResource::Inventory { item: offered.clone() }),
        _ => None,
    };
    let admission = context
        .original_pickups
        .clone()
        .unwrap_or_else(|| OriginalPickupAdmission {
            touch: Rc::new(|_, _| OriginalPickupOutcome::Stale),
        });
    (admission.touch)(
        OriginalPickupOffer {
            recipient: other_entity.borrow().actor.id.clone(),
            pickup: entity.borrow().actor.id.clone(),
            source: entity.borrow().actor.owner.clone(),
            item: offered,
            default_resource,
            count: if entity.borrow().count == 0 {
                PickupCount::Default
            } else {
                PickupCount::Override {
                    amount: entity.borrow().count,
                }
            },
            dropped: (entity.borrow().flags & game_flags::DROPPED_ITEM) != 0,
            time: SourceTime::Milliseconds { value: now },
            grant: if item.item_type == ItemType::Team {
                Some(PickupGrant::MapCoupled)
            } else {
                None
            },
        },
        OriginalContinuation {
            eligible: None,
            original,
            complete,
        },
    );
}

/// Float respawn schedule (`sourceFloatSchedule`).
pub(crate) fn source_float_schedule(time: i32, seconds: f32) -> i32 {
    qvm_float_to_int(time as f32 + seconds * 1000.0)
}

/// Finish spawning an item (`finishSpawningItem`, `FinishSpawningItem`).
pub fn finish_spawning_item(entity: &EntityRef, context: &ItemLifecycleContext) {
    bind_item_save_callbacks(context);
    check_lifecycle_context(context);
    require_lifecycle_owned(context, entity);
    let item = require_lifecycle_item(context, entity);
    {
        let mut borrowed = entity.borrow_mut();
        borrowed.r.mins = vec3(-ITEM_RADIUS, -ITEM_RADIUS, -ITEM_RADIUS);
        borrowed.r.maxs = vec3(ITEM_RADIUS, ITEM_RADIUS, ITEM_RADIUS);
        borrowed.s.e_type = EntityType::Item as i32;
        borrowed.s.modelindex = lifecycle_table_index(context, &item);
        borrowed.s.modelindex2 = 0;
        borrowed.r.contents = CONTENTS_TRIGGER;
        borrowed.touch = Some(match context.callbacks.as_ref() {
            Some(callbacks) => callbacks.touch.clone(),
            None => context
                .entities
                .borrow()
                .callbacks
                .borrow()
                .touch
                .resolve(Some("q3.base.game.item-lifecycle.finishSpawningItem.touch"))
                .unwrap_or_else(|| {
                    panic!("Unknown native Q3 callback q3.base.game.item-lifecycle.finishSpawningItem.touch")
                }),
        });
        borrowed.use_callback = context
            .entities
            .borrow()
            .callbacks
            .borrow()
            .use_callbacks
            .resolve(Some("q3.base.game.item-lifecycle.finishSpawningItem.use"));
    }
    if (entity.borrow().spawnflags & 1) != 0 {
        let origin = entity.borrow().s.origin;
        set_origin(entity, origin);
    } else {
        let origin = entity.borrow().s.origin;
        let (mins, maxs, actor) = {
            let borrowed = entity.borrow();
            (borrowed.r.mins, borrowed.r.maxs, borrowed.actor.id.clone())
        };
        let destination = vec3(origin.x, origin.y, origin.z - 4096.0);
        let trace = (context.world.trace_actor)(&ActorTraceQuery {
            start: origin,
            end: destination,
            shape: TraceShape::Box { mins, maxs },
            pass_actor: Some(actor),
            mask: CONTENTS_SOLID,
        });
        if trace.solidity != TraceSolidity::Clear {
            let classname = entity.borrow().classname.clone();
            let where_text = context
                .entities
                .borrow()
                .utilities
                .borrow_mut()
                .vtos(origin)
                .read_string();
            (context.warn)(game_format(
                "FinishSpawningItem: %s startsolid at %s\n",
                &[
                    classname.map_or(GameFormatArgument::Null, GameFormatArgument::Text),
                    GameFormatArgument::Text(where_text),
                ],
            ));
            context.entities.borrow().free(entity);
            return;
        }
        trace_ground(entity, &trace.hit, &context.entities.borrow());
        set_origin(entity, trace.end);
    }
    if (entity.borrow().flags & game_flags::TEAMSLAVE) != 0 || entity.borrow().targetname.is_some() {
        let mut borrowed = entity.borrow_mut();
        borrowed.s.e_flags |= EF_NODRAW;
        borrowed.r.contents = 0;
        return;
    }
    if item.item_type == ItemType::Powerup {
        let delay = 45.0 + lifecycle_crandom(&context.random) * 15.0;
        let think = match context.callbacks.as_ref() {
            Some(callbacks) => callbacks.respawn.clone(),
            None => context
                .entities
                .borrow()
                .callbacks
                .borrow()
                .think
                .resolve(Some("q3.base.game.item-lifecycle.touchItem.think"))
                .unwrap_or_else(|| panic!("Unknown native Q3 callback q3.base.game.item-lifecycle.touchItem.think")),
        };
        let mut borrowed = entity.borrow_mut();
        borrowed.s.e_flags |= EF_NODRAW;
        borrowed.r.contents = 0;
        borrowed.nextthink = source_float_schedule(lifecycle_time(context), delay);
        borrowed.think = Some(think);
        return;
    }
    (context.entities.borrow().options.link)(entity.clone());
}

/// Spawn an item (`spawnItem`, `G_SpawnItem`).
pub fn spawn_item(
    entity: &EntityRef,
    item: &ItemDefinition,
    variables: &SpawnVariables,
    is_disabled: &dyn Fn() -> bool,
    context: &ItemLifecycleContext,
) {
    bind_item_save_callbacks(context);
    check_lifecycle_context(context);
    require_lifecycle_owned(context, entity);
    lifecycle_table_index(context, item);
    entity.borrow_mut().random = variables.float("random", "0").value;
    entity.borrow_mut().wait = variables.float("wait", "0").value;
    context.registry.borrow_mut().register(item);
    if is_disabled() {
        return;
    }
    {
        let mut borrowed = entity.borrow_mut();
        borrowed.item = Some(item.clone());
        borrowed.nextthink = lifecycle_time(context).wrapping_add(FRAME_TIME * 2);
        borrowed.think = context
            .entities
            .borrow()
            .callbacks
            .borrow()
            .think
            .resolve(Some("q3.base.game.item-lifecycle.spawnItem.think"));
        borrowed.physics_bounce = 0.5;
    }
    if item.item_type == ItemType::Powerup {
        (context.sound_index)("sound/items/poweruprespawn.wav");
        entity.borrow_mut().speed = variables.float("noglobalsound", "0").value;
    }
    if context.product == Q3Product::Missionpack && item.item_type == ItemType::PersistantPowerup {
        let spawnflags = entity.borrow().spawnflags;
        entity.borrow_mut().s.generic1 = spawnflags;
    }
}

/// Register item lifecycle callbacks (`bindItemSaveCallbacks`).
pub fn bind_item_save_callbacks(context: &ItemLifecycleContext) {
    let pool = context.entities.clone();
    let saved = context.clone();
    pool.borrow().callbacks.borrow_mut().think.intern(
        "q3.base.game.item-lifecycle.touchItem.think",
        Rc::new(move |entity: EntityRef| {
            respawn_item(&entity, &saved);
        }),
    );
    let saved = context.clone();
    pool.borrow().callbacks.borrow_mut().touch.intern(
        "q3.base.game.item-lifecycle.finishSpawningItem.touch",
        Rc::new(
            move |this: EntityRef, other: DamageParticipant, contact: TouchContactMirror| {
                touch_item(&this, other, &contact, &saved);
            },
        ),
    );
    let saved = context.clone();
    pool.borrow().callbacks.borrow_mut().use_callbacks.intern(
        "q3.base.game.item-lifecycle.finishSpawningItem.use",
        Rc::new(move |entity: EntityRef, _, _| {
            respawn_item(&entity, &saved);
        }),
    );
    let saved = context.clone();
    pool.borrow().callbacks.borrow_mut().think.intern(
        "q3.base.game.item-lifecycle.spawnItem.think",
        Rc::new(move |entity: EntityRef| {
            finish_spawning_item(&entity, &saved);
        }),
    );
}
