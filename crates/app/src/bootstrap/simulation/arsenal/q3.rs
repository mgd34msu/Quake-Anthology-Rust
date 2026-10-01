//! Selected Quake III arsenal over the shared inventory.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/arsenal/q3.ts`
//! (`Q3SelectedArsenalOptions`, `Q3SelectedArsenalCheckpoint`, `Q3SelectedArsenal`,
//! `readQ3SelectedArsenalCheckpoint`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_content::contract::ItemId;
use qa_content::q3::base::shared::definitions::{ItemType, Product};
use qa_content::q3::base::shared::items::item_list;
use qa_content::q3::foundation::arsenal::{
    q3_request_weapon, q3_request_weapon_holster, q3_request_weapon_resume, q3_spawn_animation,
    q3_spawn_arsenal_runtime, q3_spawn_loadout, q3_weapon_item, step_q3_arsenal, Q3ArsenalRuntimeState,
    WeaponStepInput as Q3WeaponStepInput, Q3_WEAPON_ITEMS,
};
use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::time::SourceTime;
use qa_net::common::commands::ArsenalIntent;
use qa_world::inventory::{InventoryEntry as WorldInventoryEntry, InventoryTable};
use qa_world::movement::q3::constants::entity_event::{FIRE_WEAPON, USE_ITEM0};
use qa_world::movement::q3::weapon::Q3ExternalWeaponSlot;
use qa_world::movement::types::{
    ActorAnimationState, AnimationState, ArsenalState, MovementEffect, UserCommand, WeaponState,
};
use qa_world::pickups::PickupAmmoReceipt;
use qa_world::registry::ActorRegistry;
use qa_world::save::records::read_inventory_entry;
use qa_world::save::value::{namespaced, SaveReader};

use super::super::arsenal_intent::{resolve_q3_arsenal_controls, ArsenalIntentError};
use super::super::weapon_slot::{
    PrimaryWeaponHandoff, RequestStatus, ResumeOutcome, SourceWeaponHandoff, SourceWeaponRequest,
};
use super::selected::{
    ArsenalFamily, ArsenalView, SelectedArsenal, SelectedArsenalUi, SelectedPickupWeapon, SupplyDrop, WeaponStepInput,
    WeaponStepResult,
};
use super::weapon_status::{q3_arsenal_warning, q3_weapon_status};
use qa_content::contract::PickupSelection;
use qa_content::contract::ProviderReference;

/// Errors in the selected Q3 arsenal.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Q3ArsenalError {
    /// Arsenal already admitted.
    #[error("Selected Q3 arsenal already admitted")]
    AlreadyAdmitted,
    /// Starter belongs to a different arsenal.
    #[error("Selected Q3 starter belongs to a different arsenal")]
    ForeignStarter,
    /// Actor has no selected Q3 arsenal.
    #[error("Actor has no selected Q3 arsenal")]
    MissingArsenal,
    /// Weapon does not belong to the selected Q3 product.
    #[error("Weapon does not belong to the selected Q3 product")]
    ForeignWeapon,
    /// Arsenal intent belongs to a different provider.
    #[error("Arsenal intent belongs to a different provider")]
    ForeignIntent,
    /// Step returned a foreign arsenal.
    #[error("Selected Q3 step returned a foreign arsenal")]
    ForeignStep,
    /// Saved pickup supply differs from selected arsenal composition.
    #[error("Saved pickup supply differs from selected arsenal composition")]
    SupplyMismatch,
    /// Saved arsenal differs from selected Q3 provider.
    #[error("Saved arsenal differs from selected Q3 provider")]
    ProviderMismatch,
    /// Resume item is not a Q3 primary weapon.
    #[error("Resume item is not a Q3 primary weapon")]
    ForeignResume,
    /// Selected Q3 weapon lacks its original item label.
    #[error("Selected Q3 weapon lacks its original item label")]
    MissingLabel,
    /// Selected Q3 player has foreign arsenal state.
    #[error("Selected Q3 player has foreign arsenal state")]
    ForeignState,
    /// Arsenal intent does not fit the selected Q3 product.
    #[error(transparent)]
    Intent(#[from] ArsenalIntentError),
    /// Q3 arsenal step failed.
    #[error(transparent)]
    Arsenal(#[from] qa_content::q3::foundation::arsenal::ArsenalError),
    /// Inventory operation failed.
    #[error(transparent)]
    Inventory(#[from] qa_world::WorldError),
}

/// Equipment-slice read hook.
pub type Q3EquipmentReadHook = Box<dyn Fn(&OwnedActor) -> Q3EquipmentSlice>;
/// Holdable-consume hook.
pub type Q3ConsumeHook = Box<dyn Fn(&OwnedActor, i32)>;
/// Equipment-timer advance hook.
pub type Q3AdvanceHook = Box<dyn Fn(&OwnedActor, i32)>;
/// End-of-command equipment hook.
pub type Q3EndCommandHook = Box<dyn Fn(&OwnedActor, i32)>;
/// Equipment-slice restore hook.
pub type Q3RestoreHook = Box<dyn Fn(&OwnedActor, &Q3EquipmentSlice)>;
/// Firing-delay override hook.
pub type Q3FiringDelayHook = Box<dyn Fn(&OwnedActor, i32) -> i32>;
/// Loadout override hook.
pub type Q3LoadoutHook = Box<dyn Fn(&OwnedActor, &ArsenalState) -> ArsenalState>;
/// Fire callback hook.
pub type Q3FireHook = Box<dyn Fn(&OwnedActor, i32, &WeaponStepInput)>;
/// Holdable-use callback hook.
pub type Q3UseHoldableHook = Box<dyn Fn(&OwnedActor, i32, &WeaponStepInput)>;

/// Selected supply composition for the Q3 arsenal.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SelectedSupply {
    /// Supply profile.
    pub profile: ItemId,
    /// Starter loadout.
    pub loadout: ArsenalState,
    /// Replaced items cleared on admit.
    pub replaced_items: Vec<ItemId>,
}

/// Equipment-owned slice of the Q3 arsenal runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3EquipmentSlice {
    /// Maximum health.
    pub max_health: f64,
    /// Persistent powerup tag.
    pub persistent_powerup_tag: i32,
    /// Holdable item.
    pub holdable_item: i32,
    /// Holdable tag.
    pub holdable_tag: i32,
}

/// Equipment callbacks for the selected Q3 arsenal.
pub struct Q3SelectedEquipment {
    /// Whether the arsenal owns holdable input (default true).
    pub owns_holdables: bool,
    /// Read the equipment-owned runtime slice.
    pub read: Q3EquipmentReadHook,
    /// Consume a holdable.
    pub consume: Q3ConsumeHook,
    /// Advance equipment timers.
    pub advance: Option<Q3AdvanceHook>,
    /// End-of-command equipment update.
    pub end_command: Option<Q3EndCommandHook>,
    /// Restore the equipment-owned runtime slice.
    pub restore: Option<Q3RestoreHook>,
}

/// Options for the selected Q3 arsenal.
pub struct Q3SelectedArsenalOptions {
    /// Owning provider.
    pub provider: ProviderId,
    /// Selected product.
    pub product: Product,
    /// Shared inventory table.
    pub inventory: Rc<RefCell<InventoryTable>>,
    /// Actor registry for inventory liveness.
    pub registry: Rc<ActorRegistry>,
    /// Selected supply composition.
    pub supply: Option<Q3SelectedSupply>,
    /// Firing-delay override.
    pub firing_delay: Option<Q3FiringDelayHook>,
    /// Loadout override.
    pub loadout: Option<Q3LoadoutHook>,
    /// Equipment callbacks.
    pub equipment: Option<Q3SelectedEquipment>,
    /// Fire callback.
    pub fire: Q3FireHook,
    /// Holdable-use callback.
    pub use_holdable: Q3UseHoldableHook,
}

/// Selected Q3 arsenal checkpoint: exact donor shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SelectedArsenalCheckpoint {
    /// Supply profile.
    pub supply_profile: Option<ItemId>,
    /// Arsenal state.
    pub arsenal: ArsenalState,
    /// Private runtime.
    pub runtime: Q3ArsenalRuntimeState,
    /// Torso animation.
    pub torso_animation: i32,
    /// Last fire time in milliseconds.
    pub last_fire_milliseconds: Option<f64>,
    /// Pending holdable use.
    pub pending_use: Option<ItemId>,
}

struct PlayerArsenal {
    actor: OwnedActor,
    arsenal: ArsenalState,
    runtime: Q3ArsenalRuntimeState,
    torso_animation: i32,
    last_fire_milliseconds: Option<f64>,
    pending_use: Option<ItemId>,
}

struct Q3ArsenalShared {
    provider: ProviderId,
    product: Product,
    inventory_items: HashSet<ItemId>,
    players: HashMap<ActorId, PlayerArsenal>,
    inventory: Rc<RefCell<InventoryTable>>,
    registry: Rc<ActorRegistry>,
}

impl Q3ArsenalShared {
    fn count(&self, actor: &ActorId, item: &ItemId) -> f64 {
        self.inventory.borrow().count(&self.registry, actor, item)
    }

    fn entries(&self, actor: &ActorId) -> Vec<WorldInventoryEntry> {
        self.inventory.borrow().entries(&self.registry, actor)
    }

    fn configure(&self, actor: &OwnedActor, item: ItemId, count: f64) -> Result<(), Q3ArsenalError> {
        // Movement ammo entries carry no capacity across the arsenal boundary;
        // weapons admit one, ammunition admits the source pool.
        let capacity = if Q3_WEAPON_ITEMS.iter().any(|weapon| weapon.item == item) {
            1.0
        } else {
            200.0
        };
        self.inventory.borrow_mut().configure(
            &self.registry,
            actor,
            WorldInventoryEntry {
                item,
                count,
                capacity,
                count_policy: None,
            },
        )?;
        Ok(())
    }

    fn configure_entry(&self, actor: &OwnedActor, entry: WorldInventoryEntry) -> Result<(), Q3ArsenalError> {
        self.inventory.borrow_mut().configure(&self.registry, actor, entry)?;
        Ok(())
    }

    fn read(&self, actor: &ActorId) -> Result<ArsenalState, Q3ArsenalError> {
        let player = self.players.get(actor).ok_or(Q3ArsenalError::MissingArsenal)?;
        Ok(ArsenalState {
            ammo: self
                .entries(actor)
                .into_iter()
                .filter(|entry| self.inventory_items.contains(&entry.item))
                .map(|entry| qa_world::movement::types::InventoryEntry {
                    item: entry.item,
                    count: entry.count,
                })
                .collect(),
            ..player.arsenal.clone()
        })
    }

    fn select_inner(&mut self, actor: &ActorId, item: &ItemId) -> Result<bool, Q3ArsenalError> {
        let weapon = Q3_WEAPON_ITEMS.iter().find(|entry| entry.item == *item);
        let Some(weapon) = weapon else {
            return Ok(false);
        };
        if self.product == Product::Baseq3 && weapon.weapon as i32 > 10 {
            return Ok(false);
        }
        if self.count(actor, item) <= 0.0 {
            return Ok(false);
        }
        let player = self.players.get_mut(actor).ok_or(Q3ArsenalError::MissingArsenal)?;
        player.runtime = q3_request_weapon(&player.runtime, weapon.weapon as i32)?;
        Ok(true)
    }
}

fn provider_name(provider: &ProviderId) -> String {
    format!("{}:{}", provider.namespace, provider.name)
}

fn contract<T>(result: Result<T, Q3ArsenalError>) -> T {
    result.unwrap_or_else(|error| panic!("{error}"))
}

/// Selected Q3 arsenal over the shared inventory.
pub struct Q3SelectedArsenal {
    shared: Rc<RefCell<Q3ArsenalShared>>,
    supply: Option<Q3SelectedSupply>,
    firing_delay: Option<Q3FiringDelayHook>,
    loadout: Option<Q3LoadoutHook>,
    equipment: Option<Q3SelectedEquipment>,
    fire: Q3FireHook,
    use_holdable: Q3UseHoldableHook,
}

impl Q3SelectedArsenal {
    /// Create a selected Q3 arsenal.
    pub fn new(options: Q3SelectedArsenalOptions) -> Self {
        let inventory_items = Q3_WEAPON_ITEMS
            .iter()
            .filter(|weapon| options.product == Product::Missionpack || weapon.weapon as i32 <= 10)
            .flat_map(|weapon| {
                weapon.ammo.as_ref().map_or_else(
                    || vec![weapon.item.clone()],
                    |ammo| vec![weapon.item.clone(), ammo.clone()],
                )
            })
            .collect();
        Q3SelectedArsenal {
            shared: Rc::new(RefCell::new(Q3ArsenalShared {
                provider: options.provider,
                product: options.product,
                inventory_items,
                players: HashMap::new(),
                inventory: options.inventory,
                registry: options.registry,
            })),
            supply: options.supply,
            firing_delay: options.firing_delay,
            loadout: options.loadout,
            equipment: options.equipment,
            fire: options.fire,
            use_holdable: options.use_holdable,
        }
    }

    fn clear_replaced_items(&self, actor: &OwnedActor) -> Result<(), Q3ArsenalError> {
        let replaced: Vec<(ItemId, f64)> = {
            let shared = self.shared.borrow();
            let Some(supply) = &self.supply else {
                return Ok(());
            };
            shared
                .entries(actor.id())
                .into_iter()
                .filter(|entry| supply.replaced_items.contains(&entry.item))
                .map(|entry| (entry.item, entry.capacity))
                .collect()
        };
        let shared = self.shared.borrow();
        for (item, capacity) in replaced {
            shared.configure_entry(
                actor,
                WorldInventoryEntry {
                    item,
                    count: 0.0,
                    capacity,
                    count_policy: None,
                },
            )?;
        }
        Ok(())
    }

    fn try_read(&self, actor: &ActorId) -> Result<ArsenalState, Q3ArsenalError> {
        self.shared.borrow().read(actor)
    }

    fn try_select(&mut self, actor: &ActorId, item: &ItemId) -> Result<bool, Q3ArsenalError> {
        self.shared.borrow_mut().select_inner(actor, item)
    }

    /// Whether holdable input is owned for an actor.
    pub fn owns_holdable_input(&self, actor: &ActorId) -> Result<bool, Q3ArsenalError> {
        let shared = self.shared.borrow();
        let player = shared.players.get(actor).ok_or(Q3ArsenalError::MissingArsenal)?;
        Ok(self
            .equipment
            .as_ref()
            .map(|equipment| equipment.owns_holdables)
            .unwrap_or(true)
            && (player.runtime.use_item_held
                || (self
                    .equipment
                    .as_ref()
                    .map(|equipment| (equipment.read)(&player.actor).holdable_item)
                    .unwrap_or(player.runtime.holdable_item))
                    != 0))
    }

    /// Observe holdable input for an actor.
    pub fn observe_holdable_input(
        &mut self,
        actor: &ActorId,
        command: &UserCommand,
        intent: Option<&ArsenalIntent>,
    ) -> Result<(), Q3ArsenalError> {
        if self
            .equipment
            .as_ref()
            .is_some_and(|equipment| !equipment.owns_holdables)
        {
            return Ok(());
        }
        let product = self.shared.borrow().product;
        let arsenal = self.try_read(actor)?;
        let controls =
            resolve_q3_arsenal_controls(&arsenal, intent, command, product).map_err(Q3ArsenalError::Intent)?;
        let mut shared = self.shared.borrow_mut();
        let player = shared.players.get_mut(actor).ok_or(Q3ArsenalError::MissingArsenal)?;
        if !controls.use_holdable {
            player.runtime.use_item_held = false;
            player.runtime.respawned = controls.attack && player.runtime.respawned;
            return Ok(());
        }
        if player.runtime.use_item_held {
            return Ok(());
        }
        let held = self
            .equipment
            .as_ref()
            .map(|equipment| (equipment.read)(&player.actor).holdable_item)
            .unwrap_or(player.runtime.holdable_item);
        let item = item_list(product).get(held as usize);
        if item.is_some_and(|item| item.item_type() == ItemType::ItHoldable) {
            if let Some(class_name) = item.and_then(|item| item.class_name) {
                player.pending_use = Some(format!("q3:{class_name}"));
            }
        }
        Ok(())
    }

    /// Queue a holdable use for an actor.
    pub fn use_item(&mut self, actor: &ActorId, item: &ItemId) -> Result<bool, Q3ArsenalError> {
        if self
            .equipment
            .as_ref()
            .is_some_and(|equipment| !equipment.owns_holdables)
        {
            return Ok(false);
        }
        let product = self.shared.borrow().product;
        let mut shared = self.shared.borrow_mut();
        let player = shared.players.get(actor).ok_or(Q3ArsenalError::MissingArsenal)?;
        let held = self
            .equipment
            .as_ref()
            .map(|equipment| (equipment.read)(&player.actor).holdable_item)
            .unwrap_or(player.runtime.holdable_item);
        let definition = item_list(product).get(held as usize);
        let matches = definition.is_some_and(|definition| {
            definition.item_type() == ItemType::ItHoldable
                && definition
                    .class_name
                    .is_some_and(|class| *item == format!("q3:{class}"))
        });
        if !matches {
            return Ok(false);
        }
        let player = shared.players.get_mut(actor).ok_or(Q3ArsenalError::MissingArsenal)?;
        player.pending_use = Some(item.clone());
        Ok(true)
    }

    fn best_weapon(&self, actor: &ActorId, before: &[PickupAmmoReceipt]) -> Option<ItemId> {
        let shared = self.shared.borrow();
        Q3_WEAPON_ITEMS
            .iter()
            .rev()
            .find(|weapon| {
                (shared.product == Product::Missionpack || weapon.weapon as i32 <= 10)
                    && shared.count(actor, &weapon.item) > 0.0
                    && weapon.ammo.as_ref().is_none_or(|ammo| {
                        before
                            .iter()
                            .find(|grant| grant.item == *ammo)
                            .map(|grant| grant.before)
                            .unwrap_or_else(|| shared.count(actor, ammo))
                            > 0.0
                    })
            })
            .map(|weapon| weapon.item.clone())
    }

    fn try_step(
        &mut self,
        input: &WeaponStepInput,
        intent: Option<&ArsenalIntent>,
    ) -> Result<WeaponStepResult, Q3ArsenalError> {
        let provider = self.shared.borrow().provider.clone();
        let product = self.shared.borrow().product;
        if intent.is_some_and(|intent| intent.provider != provider_name(&provider)) {
            return Err(Q3ArsenalError::ForeignIntent);
        }
        if let Some(weapon) = intent.and_then(|intent| intent.weapon.as_ref()) {
            let known = Q3_WEAPON_ITEMS
                .iter()
                .any(|entry| entry.item == *weapon && (product == Product::Missionpack || entry.weapon as i32 <= 10));
            if !known {
                return Err(Q3ArsenalError::ForeignWeapon);
            }
            self.try_select(input.actor.id(), weapon)?;
        }
        let (actor, runtime, torso_animation) = {
            let shared = self.shared.borrow();
            let player = shared
                .players
                .get(input.actor.id())
                .ok_or(Q3ArsenalError::MissingArsenal)?;
            let mut runtime = player.runtime.clone();
            if let Some(equipment) = &self.equipment {
                let slice = (equipment.read)(&player.actor);
                runtime.max_health = slice.max_health;
                runtime.persistent_powerup_tag = slice.persistent_powerup_tag;
                runtime.holdable_item = slice.holdable_item;
                runtime.holdable_tag = slice.holdable_tag;
            }
            (player.actor.clone(), runtime, player.torso_animation)
        };
        let holdable = runtime.holdable_item;
        let arsenal = self.try_read(input.actor.id())?;
        let controls =
            resolve_q3_arsenal_controls(&arsenal, intent, &input.command, product).map_err(Q3ArsenalError::Intent)?;
        let pending_use = self
            .shared
            .borrow_mut()
            .players
            .get_mut(input.actor.id())
            .ok_or(Q3ArsenalError::MissingArsenal)
            .map(|player| player.pending_use.take())?;
        let held_class = item_list(product)
            .get(holdable as usize)
            .and_then(|item| item.class_name);
        let use_queued = pending_use.as_deref()
            == held_class.map(|class| format!("q3:{class}")).as_deref().or(Some(""))
            && pending_use.is_some();
        let animation = if matches!(input.animation.state, AnimationState::Q3 { .. }) {
            input.animation.clone()
        } else {
            let spawned = q3_spawn_animation();
            let AnimationState::Q3 {
                legs,
                torso: _,
                legs_timer_milliseconds,
                torso_timer_milliseconds,
            } = spawned
            else {
                unreachable!("spawned animation is Q3");
            };
            ActorAnimationState {
                provider: provider.clone(),
                state: AnimationState::Q3 {
                    legs,
                    torso: torso_animation,
                    legs_timer_milliseconds,
                    torso_timer_milliseconds,
                },
            }
        };
        let controls = if self
            .equipment
            .as_ref()
            .is_some_and(|equipment| !equipment.owns_holdables)
        {
            qa_content::q3::foundation::arsenal::Q3ArsenalControls {
                use_holdable: false,
                ..controls
            }
        } else {
            qa_content::q3::foundation::arsenal::Q3ArsenalControls {
                use_holdable: controls.use_holdable || use_queued,
                ..controls
            }
        };
        let delay_state = self.firing_delay.as_deref().map(|delay| {
            let actor = actor.clone();
            move |ms: i32| delay(&actor, ms)
        });
        let delay_override = delay_state.as_ref().map(|closure| closure as &dyn Fn(i32) -> i32);
        let result = step_q3_arsenal(
            &Q3WeaponStepInput {
                actor: actor.clone(),
                command: input.command,
                frame: input.frame,
                arsenal: arsenal.clone(),
                animation,
                environment: input.environment,
                gauntlet_hit: input.gauntlet_hit,
            },
            &runtime,
            &controls,
            delay_override,
        )
        .map_err(Q3ArsenalError::Arsenal)?;
        let elapsed = match input.frame.elapsed {
            SourceTime::Milliseconds(value) => f64::from(value),
            SourceTime::Seconds(value) => f64::from(value) * 1000.0,
        };
        let milliseconds = (elapsed + runtime.fractional_milliseconds).trunc() as i32;
        if let Some(equipment) = &self.equipment {
            if let Some(advance) = &equipment.advance {
                advance(&actor, milliseconds);
            }
        }
        {
            let mut shared = self.shared.borrow_mut();
            let player = shared
                .players
                .get_mut(input.actor.id())
                .ok_or(Q3ArsenalError::MissingArsenal)?;
            player.arsenal = result.arsenal.clone();
            player.runtime = if use_queued && !controls.use_holdable {
                let mut runtime = result.runtime.clone();
                runtime.use_item_held = false;
                runtime
            } else {
                result.runtime.clone()
            };
            if holdable != 0 && result.runtime.holdable_item == 0 {
                if let Some(equipment) = &self.equipment {
                    (equipment.consume)(&actor, holdable);
                }
            }
            if let AnimationState::Q3 { torso, .. } = result.animation.state {
                player.torso_animation = torso;
            }
            for entry in &result.arsenal.ammo {
                shared.configure(&actor, entry.item.clone(), entry.count)?;
            }
        }
        let WeaponState::Q3 { source_weapon, .. } = result.arsenal.state else {
            return Err(Q3ArsenalError::ForeignStep);
        };
        for effect in &result.effects {
            let MovementEffect::Event(event) = effect else {
                continue;
            };
            if event.event == FIRE_WEAPON {
                let fired = match input.frame.time {
                    SourceTime::Milliseconds(value) => f64::from(value),
                    SourceTime::Seconds(value) => f64::from(value) * 1000.0,
                };
                self.shared
                    .borrow_mut()
                    .players
                    .get_mut(input.actor.id())
                    .ok_or(Q3ArsenalError::MissingArsenal)?
                    .last_fire_milliseconds = Some(fired);
                (self.fire)(&actor, source_weapon, input);
            } else if (USE_ITEM0..=USE_ITEM0 + 15).contains(&event.event) {
                (self.use_holdable)(&actor, event.event, input);
            }
        }
        let present = self.shared.borrow().players.contains_key(input.actor.id());
        if present {
            if let Some(equipment) = &self.equipment {
                if let Some(end_command) = &equipment.end_command {
                    end_command(&actor, milliseconds);
                }
            }
        }
        Ok(WeaponStepResult {
            continuation: None,
            arsenal: if present {
                self.try_read(input.actor.id())?
            } else {
                result.arsenal.clone()
            },
            animation: if matches!(input.animation.state, AnimationState::Q3 { .. }) {
                result.animation.clone()
            } else {
                input.animation.clone()
            },
            effects: result.effects.clone(),
        })
    }

    /// Capture a checkpoint for an actor.
    pub fn capture(&self, actor: &ActorId) -> Result<Q3SelectedArsenalCheckpoint, Q3ArsenalError> {
        let shared = self.shared.borrow();
        let player = shared.players.get(actor).ok_or(Q3ArsenalError::MissingArsenal)?;
        let mut runtime = player.runtime.clone();
        if let Some(equipment) = &self.equipment {
            let slice = (equipment.read)(&player.actor);
            runtime.max_health = slice.max_health;
            runtime.persistent_powerup_tag = slice.persistent_powerup_tag;
            runtime.holdable_item = slice.holdable_item;
            runtime.holdable_tag = slice.holdable_tag;
        }
        Ok(Q3SelectedArsenalCheckpoint {
            supply_profile: self.supply.as_ref().map(|supply| supply.profile.clone()),
            arsenal: shared.read(actor)?,
            runtime,
            torso_animation: player.torso_animation,
            last_fire_milliseconds: player.last_fire_milliseconds,
            pending_use: player.pending_use.clone(),
        })
    }

    /// Restore a checkpoint for an actor.
    pub fn restore(
        &mut self,
        actor: &OwnedActor,
        checkpoint: &Q3SelectedArsenalCheckpoint,
    ) -> Result<(), Q3ArsenalError> {
        if checkpoint.supply_profile != self.supply.as_ref().map(|supply| supply.profile.clone()) {
            return Err(Q3ArsenalError::SupplyMismatch);
        }
        {
            let shared = self.shared.borrow();
            if checkpoint.arsenal.provider != shared.provider
                || !matches!(checkpoint.arsenal.state, WeaponState::Q3 { .. })
                || checkpoint.runtime.product != shared.product
            {
                return Err(Q3ArsenalError::ProviderMismatch);
            }
            for entry in &checkpoint.arsenal.ammo {
                if shared.inventory_items.contains(&entry.item) {
                    shared.configure(actor, entry.item.clone(), entry.count)?;
                }
            }
        }
        if let Some(equipment) = &self.equipment {
            if let Some(restore) = &equipment.restore {
                restore(
                    actor,
                    &Q3EquipmentSlice {
                        max_health: checkpoint.runtime.max_health,
                        persistent_powerup_tag: checkpoint.runtime.persistent_powerup_tag,
                        holdable_item: checkpoint.runtime.holdable_item,
                        holdable_tag: checkpoint.runtime.holdable_tag,
                    },
                );
            }
        }
        self.shared.borrow_mut().players.insert(
            actor.id().clone(),
            PlayerArsenal {
                actor: actor.clone(),
                arsenal: checkpoint.arsenal.clone(),
                runtime: checkpoint.runtime.clone(),
                torso_animation: checkpoint.torso_animation,
                last_fire_milliseconds: checkpoint.last_fire_milliseconds,
                pending_use: checkpoint.pending_use.clone(),
            },
        );
        Ok(())
    }

    /// Torso/last-fire view state for an actor.
    pub fn view_state(&self, actor: &ActorId) -> Result<Q3ArsenalViewState, Q3ArsenalError> {
        let shared = self.shared.borrow();
        let player = shared.players.get(actor).ok_or(Q3ArsenalError::MissingArsenal)?;
        Ok(Q3ArsenalViewState {
            torso_animation: player.torso_animation,
            last_fire_milliseconds: player.last_fire_milliseconds,
        })
    }
}

/// Torso/last-fire view state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ArsenalViewState {
    /// Torso animation.
    pub torso_animation: i32,
    /// Last fire time in milliseconds.
    pub last_fire_milliseconds: Option<f64>,
}

struct RefusedHandoffRequest {
    id: u64,
}

impl SourceWeaponRequest for RefusedHandoffRequest {
    fn id(&self) -> u64 {
        self.id
    }

    fn status(&mut self) -> RequestStatus {
        RequestStatus::Refused
    }

    fn cancel(&mut self) {}
}

struct Q3Handoff {
    shared: Rc<RefCell<Q3ArsenalShared>>,
    actor: ActorId,
}

impl SourceWeaponHandoff for Q3Handoff {
    fn provider(&self) -> ProviderId {
        self.shared.borrow().provider.clone()
    }

    fn accepts(&self, item: &ItemId) -> bool {
        let shared = self.shared.borrow();
        Q3_WEAPON_ITEMS.iter().any(|weapon| {
            weapon.item == *item && (shared.product == Product::Missionpack || weapon.weapon as i32 <= 10)
        }) && shared.count(&self.actor, item) > 0.0
    }

    fn select(&mut self, item: &ItemId) -> bool {
        contract(self.shared.borrow_mut().select_inner(&self.actor, item))
    }

    fn holster(&mut self) {
        let mut shared = self.shared.borrow_mut();
        let Some(player) = shared.players.get_mut(&self.actor) else {
            panic!("{}", Q3ArsenalError::MissingArsenal);
        };
        player.runtime = q3_request_weapon_holster(&player.runtime);
    }

    fn is_holstered(&self) -> bool {
        let shared = self.shared.borrow();
        shared
            .players
            .get(&self.actor)
            .is_some_and(|player| player.runtime.external_slot == Q3ExternalWeaponSlot::Holstered)
    }

    fn resume(&mut self, item: Option<&ItemId>) -> ResumeOutcome {
        let mut shared = self.shared.borrow_mut();
        let product = shared.product;
        if item.is_some_and(|item| {
            !Q3_WEAPON_ITEMS
                .iter()
                .any(|weapon| weapon.item == *item && (product == Product::Missionpack || weapon.weapon as i32 <= 10))
        }) {
            panic!("{}", Q3ArsenalError::ForeignResume);
        }
        let player = shared
            .players
            .get_mut(&self.actor)
            .unwrap_or_else(|| panic!("{}", Q3ArsenalError::MissingArsenal));
        let runtime = contract(q3_request_weapon_resume(&player.runtime).map_err(Q3ArsenalError::Arsenal));
        let requested = Q3_WEAPON_ITEMS
            .iter()
            .find(|weapon| weapon.item == *item.unwrap_or(&player.arsenal.active_weapon.clone().unwrap_or_default()));
        player.runtime = match requested {
            None => runtime,
            Some(requested) => {
                let weapon = requested.weapon as i32;
                contract(q3_request_weapon(&runtime, weapon).map_err(Q3ArsenalError::Arsenal))
            }
        };
        let selected =
            item.is_none() || requested.is_some_and(|requested| shared.count(&self.actor, &requested.item) > 0.0);
        ResumeOutcome::Immediate(selected)
    }

    fn restore_request(&mut self, id: u64, _item: Option<&ItemId>) -> Box<dyn SourceWeaponRequest> {
        Box::new(RefusedHandoffRequest { id })
    }

    fn is_deferred(&self) -> bool {
        false
    }
}

impl SelectedArsenal for Q3SelectedArsenal {
    fn family(&self) -> ArsenalFamily {
        ArsenalFamily::Q3
    }

    fn provider(&self) -> ProviderId {
        self.shared.borrow().provider.clone()
    }

    fn catalog(&self) -> Vec<SelectedPickupWeapon> {
        let shared = self.shared.borrow();
        Q3_WEAPON_ITEMS
            .iter()
            .filter(|weapon| shared.product == Product::Missionpack || weapon.weapon as i32 <= 10)
            .map(|weapon| SelectedPickupWeapon {
                item: weapon.item.clone(),
                drop: if weapon.item == "q3:weapon/gauntlet" || weapon.item == "q3:weapon/grapple" {
                    SupplyDrop::None
                } else {
                    SupplyDrop::Supply
                },
            })
            .collect()
    }

    fn has(&self, actor: &ActorId) -> bool {
        self.shared.borrow().players.contains_key(actor)
    }

    fn admit(&mut self, actor: OwnedActor, max_health: f64, team_deathmatch: bool) -> ArsenalState {
        contract(self.try_admit(actor, max_health, team_deathmatch))
    }

    fn read(&self, actor: &ActorId) -> ArsenalState {
        contract(self.try_read(actor))
    }

    fn select(&mut self, actor: &ActorId, item: &ItemId) -> bool {
        contract(self.try_select(actor, item))
    }

    fn pickup_ammo(&mut self, actor: &OwnedActor, grants: &[PickupAmmoReceipt], auto_switch: bool) {
        contract(self.try_pickup_ammo(actor, grants, auto_switch));
    }

    fn pickup_weapons(&mut self, actor: &OwnedActor, weapons: &[ItemId], selection: PickupSelection) {
        contract(self.try_pickup_weapons(actor, weapons, selection));
    }

    fn pending_weapon(&self, actor: &ActorId) -> Option<ItemId> {
        let shared = self.shared.borrow();
        let requested = contract(
            shared
                .players
                .get(actor)
                .ok_or(Q3ArsenalError::MissingArsenal)
                .map(|player| player.runtime.requested_weapon),
        );
        requested.and_then(|requested| q3_weapon_item(requested).map(|item| item.item.clone()))
    }

    fn handoff(&mut self, actor: &ActorId) -> Box<dyn PrimaryWeaponHandoff> {
        contract(
            self.shared
                .borrow()
                .players
                .contains_key(actor)
                .then_some(())
                .ok_or(Q3ArsenalError::MissingArsenal),
        );
        Box::new(Q3Handoff {
            shared: self.shared.clone(),
            actor: actor.clone(),
        })
    }

    fn step(&mut self, input: &WeaponStepInput, intent: Option<&ArsenalIntent>) -> WeaponStepResult {
        contract(self.try_step(input, intent))
    }

    fn remove(&mut self, actor: &ActorId) {
        self.shared.borrow_mut().players.remove(actor);
    }

    fn ui(&self, actor: &ActorId, source: &ProviderReference) -> SelectedArsenalUi {
        contract(self.try_ui(actor, source))
    }

    fn view(&self, actor: &ActorId) -> Option<ArsenalView> {
        contract(self.try_view(actor))
    }
}

impl Q3SelectedArsenal {
    fn try_admit(
        &mut self,
        actor: OwnedActor,
        max_health: f64,
        team_deathmatch: bool,
    ) -> Result<ArsenalState, Q3ArsenalError> {
        if self.shared.borrow().players.contains_key(actor.id()) {
            return Err(Q3ArsenalError::AlreadyAdmitted);
        }
        let (provider, product) = {
            let shared = self.shared.borrow();
            (shared.provider.clone(), shared.product)
        };
        let defaults = self
            .supply
            .as_ref()
            .map(|supply| supply.loadout.clone())
            .unwrap_or_else(|| q3_spawn_loadout(provider.clone(), product, team_deathmatch));
        let arsenal = self
            .loadout
            .as_ref()
            .map(|loadout| loadout(&actor, &defaults))
            .unwrap_or(defaults);
        if arsenal.provider != provider || !matches!(arsenal.state, WeaponState::Q3 { .. }) {
            return Err(Q3ArsenalError::ForeignStarter);
        }
        self.clear_replaced_items(&actor)?;
        {
            let shared = self.shared.borrow();
            for entry in &arsenal.ammo {
                shared.configure(&actor, entry.item.clone(), entry.count)?;
            }
        }
        let spawned = q3_spawn_animation();
        let AnimationState::Q3 { torso, .. } = spawned else {
            unreachable!("spawned animation is Q3");
        };
        self.shared.borrow_mut().players.insert(
            actor.id().clone(),
            PlayerArsenal {
                actor: actor.clone(),
                arsenal,
                runtime: q3_spawn_arsenal_runtime(product, max_health, 0),
                torso_animation: torso,
                last_fire_milliseconds: None,
                pending_use: None,
            },
        );
        self.try_read(actor.id())
    }

    fn try_pickup_ammo(
        &mut self,
        actor: &OwnedActor,
        grants: &[PickupAmmoReceipt],
        auto_switch: bool,
    ) -> Result<(), Q3ArsenalError> {
        let active = {
            let shared = self.shared.borrow();
            shared
                .players
                .get(actor.id())
                .ok_or(Q3ArsenalError::MissingArsenal)?
                .arsenal
                .active_weapon
                .clone()
        };
        if auto_switch && self.best_weapon(actor.id(), grants).as_ref() == active.as_ref() {
            if let Some(next) = self.best_weapon(actor.id(), &[]) {
                self.try_select(actor.id(), &next)?;
            }
        }
        Ok(())
    }

    fn try_pickup_weapons(
        &mut self,
        actor: &OwnedActor,
        weapons: &[ItemId],
        selection: PickupSelection,
    ) -> Result<(), Q3ArsenalError> {
        let active = {
            let shared = self.shared.borrow();
            shared
                .players
                .get(actor.id())
                .ok_or(Q3ArsenalError::MissingArsenal)?
                .arsenal
                .active_weapon
                .clone()
        };
        let current = active
            .as_ref()
            .and_then(|active| Q3_WEAPON_ITEMS.iter().find(|weapon| weapon.item == *active));
        let next = Q3_WEAPON_ITEMS
            .iter()
            .rev()
            .find(|weapon| weapons.contains(&weapon.item));
        if selection == PickupSelection::Never {
            return Ok(());
        }
        if let Some(next) = next {
            if selection == PickupSelection::Always
                || current.is_none_or(|current| next.weapon as i32 > current.weapon as i32)
            {
                self.try_select(actor.id(), &next.item.clone())?;
            }
        }
        Ok(())
    }

    fn try_ui(&self, actor: &ActorId, source: &ProviderReference) -> Result<SelectedArsenalUi, Q3ArsenalError> {
        let arsenal = self.try_read(actor)?;
        let WeaponState::Q3 { source_weapon, .. } = arsenal.state else {
            return Err(Q3ArsenalError::ForeignState);
        };
        let weapon = q3_weapon_item(source_weapon);
        let (product, provider) = {
            let shared = self.shared.borrow();
            (shared.product, shared.provider.clone())
        };
        let _ = provider;
        let items = Q3_WEAPON_ITEMS
            .iter()
            .filter(|entry| product == Product::Missionpack || entry.weapon as i32 <= 10)
            .map(|entry| {
                let count = entry.ammo.as_ref().map(|ammo| self.shared.borrow().count(actor, ammo));
                let label = item_list(product)
                    .iter()
                    .find(|item| item.item_type() == ItemType::ItWeapon && item.tag() == entry.weapon as i32)
                    .and_then(|item| item.pickup_name)
                    .ok_or(Q3ArsenalError::MissingLabel)?;
                Ok(super::super::types::PlayerUiItem {
                    id: entry.item.clone(),
                    label: label.to_string(),
                    kind: super::super::types::PlayerUiItemKind::Weapon,
                    source_ordinal: f64::from(entry.weapon as i32),
                    owned: self.shared.borrow().count(actor, &entry.item) > 0.0,
                    has_ammo: count.is_none_or(|count| count > 0.0),
                    count,
                    warning_count: 0.0,
                })
            })
            .collect::<Result<Vec<_>, Q3ArsenalError>>()?;
        Ok(SelectedArsenalUi {
            active_weapon: arsenal.active_weapon.clone(),
            ammo: weapon.and_then(|weapon| {
                weapon.ammo.as_ref().map(|ammo| super::super::types::UiAmmo {
                    item: ammo.clone(),
                    count: self.shared.borrow().count(actor, ammo),
                })
            }),
            items,
            weapon_status: q3_weapon_status(
                arsenal.active_weapon.as_ref(),
                product,
                |item| self.shared.borrow().count(actor, item) as i32,
                source.clone(),
            ),
            arsenal_warning: q3_arsenal_warning(product, |item| self.shared.borrow().count(actor, item) as i32),
        })
    }

    fn try_view(&self, actor: &ActorId) -> Result<Option<ArsenalView>, Q3ArsenalError> {
        let arsenal = self.try_read(actor)?;
        let WeaponState::Q3 { source_weapon, .. } = arsenal.state else {
            return Err(Q3ArsenalError::ForeignState);
        };
        let product = self.shared.borrow().product;
        let path = item_list(product)
            .iter()
            .find(|entry| entry.item_type() == ItemType::ItWeapon && entry.tag() == source_weapon)
            .and_then(|entry| entry.world_models[0]);
        Ok(path.map(|path| ArsenalView {
            path: path.to_string(),
            frame: 0.0,
        }))
    }
}

/// Read a selected Q3 arsenal checkpoint.
pub fn read_q3_selected_arsenal_checkpoint(
    reader: SaveReader,
) -> Result<Q3SelectedArsenalCheckpoint, qa_world::WorldError> {
    let supply = reader.field("supplyProfile");
    let supply_profile = if supply.is_missing() {
        None
    } else {
        supply.nullable(namespaced)?
    };
    let arsenal = reader.field("arsenal");
    let state = arsenal.field("state");
    let runtime = reader.field("runtime");
    state.field("kind").literal_str("q3")?;
    let product = match runtime
        .field("product")
        .choice_str(&["baseq3", "missionpack"])?
        .as_str()
    {
        "baseq3" => Product::Baseq3,
        _ => Product::Missionpack,
    };
    let external = runtime.field("externalSlot");
    let external_slot = if external.is_missing() {
        Q3ExternalWeaponSlot::Active
    } else {
        match external
            .choice_str(&[
                "active",
                "holster-requested",
                "dropping",
                "holstered",
                "resume-requested",
            ])?
            .as_str()
        {
            "active" => Q3ExternalWeaponSlot::Active,
            "holster-requested" => Q3ExternalWeaponSlot::HolsterRequested,
            "dropping" => Q3ExternalWeaponSlot::Dropping,
            "holstered" => Q3ExternalWeaponSlot::Holstered,
            _ => Q3ExternalWeaponSlot::ResumeRequested,
        }
    };
    let pending = reader.field("pendingUse");
    let pending_use = if pending.is_missing() {
        None
    } else {
        pending.nullable(namespaced)?
    };
    Ok(Q3SelectedArsenalCheckpoint {
        supply_profile,
        arsenal: ArsenalState {
            provider: {
                let id = namespaced(arsenal.field("provider"))?;
                let (namespace, name) = id.split_once(':').unwrap_or(("q3", &id));
                ProviderId::new(namespace, name)
            },
            active_weapon: arsenal.field("activeWeapon").nullable(namespaced)?,
            ammo: arsenal
                .field("ammo")
                .list(read_inventory_entry)?
                .into_iter()
                .map(|entry| qa_world::movement::types::InventoryEntry {
                    item: entry.item,
                    count: entry.count,
                })
                .collect(),
            state: WeaponState::Q3 {
                source_weapon: state.field("sourceWeapon").integer(0)? as i32,
                state: state.field("state").integer(0)? as i32,
                time_milliseconds: state.field("timeMilliseconds").number()? as i32,
            },
        },
        runtime: Q3ArsenalRuntimeState {
            product,
            max_health: runtime.field("maxHealth").number()?,
            spectator: runtime.field("spectator").boolean()?,
            persistent_powerup_tag: runtime.field("persistentPowerupTag").integer(0)? as i32,
            holdable_item: runtime.field("holdableItem").integer(0)? as i32,
            holdable_tag: runtime.field("holdableTag").integer(0)? as i32,
            respawned: runtime.field("respawned").boolean()?,
            use_item_held: runtime.field("useItemHeld").boolean()?,
            event_sequence: runtime.field("eventSequence").integer(0)? as i32,
            fractional_milliseconds: runtime.field("fractionalMilliseconds").number()?,
            external_slot,
            requested_weapon: runtime
                .field("requestedWeapon")
                .nullable(|value| value.integer(0))?
                .map(|weapon| weapon as i32),
        },
        torso_animation: reader.field("torsoAnimation").integer(0)? as i32,
        last_fire_milliseconds: reader.field("lastFireMilliseconds").nullable(|value| value.finite())?,
        pending_use,
    })
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_world::movement::types::{Q3UserCommand, UserCommand};
    use qa_world::save::value::{arr, boolean, int, num, obj, str, SaveJson};

    use super::*;

    fn setup() -> (Rc<ActorRegistry>, Rc<RefCell<InventoryTable>>, OwnedActor) {
        let owner = IdentityOwner::create("q3-arsenal-test").unwrap();
        let mut registry = ActorRegistry::new(owner, 8).unwrap();
        let actor = registry.allocate(ProviderId::new("sim", "test"), "q3:player").unwrap();
        let registry = Rc::new(registry);
        let inventory = Rc::new(RefCell::new(InventoryTable::new()));
        inventory.borrow_mut().create(&registry, &actor, &[]).unwrap();
        (registry, inventory, actor)
    }

    fn options(registry: Rc<ActorRegistry>, inventory: Rc<RefCell<InventoryTable>>) -> Q3SelectedArsenalOptions {
        Q3SelectedArsenalOptions {
            provider: ProviderId::new("sim", "test"),
            product: Product::Baseq3,
            inventory,
            registry,
            supply: None,
            firing_delay: None,
            loadout: None,
            equipment: None,
            fire: Box::new(|_, _, _| {}),
            use_holdable: Box::new(|_, _, _| {}),
        }
    }

    #[test]
    fn admits_selects_and_reads() {
        let (registry, inventory, actor) = setup();
        let mut arsenal = Q3SelectedArsenal::new(options(registry, inventory));
        assert!(!arsenal.has(actor.id()));
        let admitted = arsenal.admit(actor.clone(), 100.0, false);
        assert!(arsenal.has(actor.id()));
        assert_eq!(admitted.active_weapon.as_deref(), Some("q3:weapon/machinegun"));
        assert!(arsenal.select(actor.id(), &"q3:weapon/machinegun".to_string()));
        assert!(!arsenal.select(actor.id(), &"q3:weapon/shotgun".to_string()));
        let read = arsenal.read(actor.id());
        assert!(read.ammo.iter().any(|entry| entry.item == "q3:weapon/machinegun"));
        let ui = arsenal.ui(
            actor.id(),
            &ProviderReference {
                provider: ProviderId::new("sim", "test"),
                content: qa_content::contract::ContentId("q3:baseq3:test:1".to_string()),
            },
        );
        assert!(!ui.items.is_empty());
        assert!(arsenal.view(actor.id()).is_some());
        assert_eq!(arsenal.family(), ArsenalFamily::Q3);
    }

    #[test]
    fn steps_fire_and_capture_restore() {
        let (registry, inventory, actor) = setup();
        let fired = Rc::new(RefCell::new(Vec::new()));
        let observed = fired.clone();
        let mut arsenal_options = options(registry, inventory);
        arsenal_options.fire = Box::new(move |actor, weapon, _| {
            observed.borrow_mut().push((actor.id().clone(), weapon));
        });
        let mut arsenal = Q3SelectedArsenal::new(arsenal_options);
        arsenal.admit(actor.clone(), 100.0, false);
        let input = WeaponStepInput {
            actor: actor.clone(),
            command: UserCommand::Q3(Q3UserCommand {
                server_time_milliseconds: 100,
                angle_words: [0, 0, 0],
                buttons: 1,
                weapon: 2,
                forward_move: 0,
                right_move: 0,
                up_move: 0,
            }),
            frame: qa_core::time::FrameContext {
                frame: 1,
                phase: qa_core::time::FramePhase::ClientCommand,
                time: SourceTime::Milliseconds(100),
                elapsed: SourceTime::Milliseconds(50),
            },
            arsenal: arsenal.read(actor.id()),
            animation: ActorAnimationState {
                provider: ProviderId::new("sim", "test"),
                state: q3_spawn_animation(),
            },
            environment: qa_world::movement::types::MovementEnvironment::default(),
            gauntlet_hit: false,
        };
        let stepped = arsenal.step(&input, None);
        assert!(matches!(stepped.arsenal.state, WeaponState::Q3 { .. }));
        let checkpoint = arsenal.capture(actor.id()).unwrap();
        assert!(checkpoint.supply_profile.is_none());
        assert_eq!(checkpoint.runtime.product, Product::Baseq3);
        arsenal.remove(actor.id());
        assert!(!arsenal.has(actor.id()));
        arsenal.restore(&actor, &checkpoint).unwrap();
        assert!(arsenal.has(actor.id()));
        let state = arsenal.view_state(actor.id()).unwrap();
        assert_eq!(state.torso_animation, checkpoint.torso_animation);
    }

    #[test]
    fn handoffs_select_and_resume() {
        let (registry, inventory, actor) = setup();
        let mut arsenal = Q3SelectedArsenal::new(options(registry, inventory));
        arsenal.admit(actor.clone(), 100.0, false);
        let mut handoff = arsenal.handoff(actor.id());
        assert!(handoff.accepts(&"q3:weapon/machinegun".to_string()));
        assert!(!handoff.accepts(&"q3:weapon/proxlauncher".to_string()));
        assert!(handoff.select(&"q3:weapon/machinegun".to_string()));
        assert!(!handoff.is_holstered());
        assert!(matches!(
            handoff.resume(Some(&"q3:weapon/machinegun".to_string())),
            ResumeOutcome::Immediate(true)
        ));
        assert!(matches!(handoff.resume(None), ResumeOutcome::Immediate(true)));
        handoff.holster();
        assert!(!handoff.is_deferred());
        let mut request = handoff.restore_request(7, None);
        assert_eq!(request.id(), 7);
        assert_eq!(request.status(), RequestStatus::Refused);
        let catalog = arsenal.catalog();
        assert!(catalog
            .iter()
            .any(|weapon| weapon.item == "q3:weapon/gauntlet" && weapon.drop == SupplyDrop::None));
    }

    #[test]
    fn reads_checkpoints() {
        let saved = obj(vec![
            ("supplyProfile", str("q3:supply/base")),
            (
                "arsenal",
                obj(vec![
                    ("provider", str("sim:test")),
                    ("activeWeapon", str("q3:weapon/machinegun")),
                    (
                        "ammo",
                        arr(vec![obj(vec![
                            ("item", str("q3:weapon/machinegun")),
                            ("count", num(1.0)),
                            ("capacity", num(1.0)),
                        ])]),
                    ),
                    (
                        "state",
                        obj(vec![
                            ("kind", str("q3")),
                            ("sourceWeapon", int(2)),
                            ("state", int(0)),
                            ("timeMilliseconds", num(0.0)),
                        ]),
                    ),
                ]),
            ),
            (
                "runtime",
                obj(vec![
                    ("product", str("baseq3")),
                    ("maxHealth", num(100.0)),
                    ("spectator", boolean(false)),
                    ("persistentPowerupTag", int(0)),
                    ("holdableItem", int(0)),
                    ("holdableTag", int(0)),
                    ("respawned", boolean(false)),
                    ("useItemHeld", boolean(false)),
                    ("eventSequence", int(0)),
                    ("fractionalMilliseconds", num(0.0)),
                    ("requestedWeapon", SaveJson::Null),
                ]),
            ),
            ("torsoAnimation", int(0)),
            ("lastFireMilliseconds", num(120.0)),
        ]);
        let reader = SaveReader::new(&saved);
        let checkpoint = read_q3_selected_arsenal_checkpoint(reader).unwrap();
        assert_eq!(checkpoint.supply_profile.as_deref(), Some("q3:supply/base"));
        assert_eq!(checkpoint.torso_animation, 0);
        assert_eq!(checkpoint.last_fire_milliseconds, Some(120.0));
        assert!(checkpoint.pending_use.is_none());
        assert_eq!(checkpoint.arsenal.ammo.len(), 1);
    }
}
