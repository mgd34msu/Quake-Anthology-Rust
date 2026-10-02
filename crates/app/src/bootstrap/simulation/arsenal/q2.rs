//! Selected Quake II arsenal over the game services.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/arsenal/q2.ts`
//! (`Q2SelectedWeaponTurnState`, `Q2SelectedArsenalOptions`, `projectQ2Arsenal`,
//! `Q2SelectedArsenal`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_content::contract::{InventoryEntry, ItemId, ProviderReference};
use qa_content::q2::foundation::host::{Q2Edition, Q2GameServices};
use qa_content::q2::foundation::weapons::definitions::base_weapons;
use qa_content::q2::foundation::weapons::player::Q2WeaponContext;
use qa_content::q2::foundation::weapons::player::{
    bind_player_weapon, is_holstered, registered_weapon_definitions, request_holster, request_weapon, resume_primary,
    tick_player_weapon, weapon_definition, weapon_firing_interval, weapon_set_loop,
};
use qa_content::q2::foundation::weapons::turn::{
    begin_q2_weapon_turn, early_q2_weapon_turn, latch_q2_weapon_buttons, Q2WeaponTurnState,
};
use qa_content::q2::foundation::weapons::types::{
    PrimaryHandoff, Q2WeaponDefinition, Q2WeaponInput, Q2WeaponName, Q2WeaponOwner, Q2WeaponPhase, Q2WeaponState,
};
use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::time::SourceTime;
use qa_net::common::commands::ArsenalIntent;
use qa_world::movement::types::{ArsenalState, WeaponState};
use qa_world::pickups::PickupAmmoReceipt;

use super::super::q3_commands::command_buttons;
use super::super::weapon_slot::{
    PrimaryWeaponHandoff, RequestStatus, ResumeOutcome, SourceWeaponHandoff, SourceWeaponRequest,
};
use super::selected::{
    ArsenalFamily, ArsenalView, SelectedArsenal, SelectedArsenalUi, SelectedPickupWeapon, SupplyDrop, WeaponStepInput,
    WeaponStepResult,
};
use super::weapon_status::q2_weapon_status;
use qa_content::contract::PickupSelection;
use qa_content::q2::foundation::items::q2_base_weapon_display_name;
use qa_content::q2::missionpacks::items::q2_mission_weapon_display_name;

/// Errors in the selected Q2 arsenal.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Q2ArsenalError {
    /// Arsenal requires an explicit pickup preference for every registered weapon.
    #[error("Selected Q2 arsenal requires an explicit pickup preference for every registered weapon")]
    PickupPreference,
    /// Missing source Q2 inventory definition.
    #[error("Missing source Q2 inventory definition {item}")]
    MissingInventoryDefinition {
        /// Missing item.
        item: ItemId,
    },
    /// Starter weapon is outside the registered arsenal.
    #[error("Selected Q2 starter weapon is outside the registered arsenal")]
    ForeignStarterWeapon,
    /// Starter inventory is invalid.
    #[error("Selected Q2 starter inventory is invalid")]
    InvalidStarterInventory,
    /// Starter weapon is not owned.
    #[error("Selected Q2 starter weapon is not owned")]
    StarterNotOwned,
    /// Arsenal already admitted.
    #[error("Selected Q2 arsenal already admitted")]
    AlreadyAdmitted,
    /// Actor has no selected Q2 arsenal.
    #[error("Actor has no selected Q2 arsenal")]
    MissingArsenal,
    /// Arsenal has no command continuation.
    #[error("Selected Q2 arsenal has no command continuation")]
    MissingTurn,
    /// Q2 player has no arsenal.
    #[error("Q2 player has no arsenal")]
    NoArsenal,
    /// Arsenal intent belongs to another provider.
    #[error("Arsenal intent belongs to another provider")]
    ForeignIntent,
    /// Weapon does not belong to selected Q2 arsenal.
    #[error("Weapon does not belong to selected Q2 arsenal")]
    ForeignWeapon,
    /// Invalid saved Q2 firing credit.
    #[error("Invalid saved Q2 firing credit")]
    InvalidFiringCredit,
}

/// Firing credit for extra classic ticks.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SelectedFiring {
    /// Firing weapon.
    pub weapon: Q2WeaponName,
    /// Credit toward extra ticks.
    pub credit: f64,
}

/// Selected Q2 weapon turn state: exact donor shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SelectedWeaponTurnState {
    /// Buttons.
    pub buttons: i32,
    /// Latched buttons.
    pub latched_buttons: i32,
    /// Weapon thunk.
    pub weapon_thunk: bool,
    /// Firing credit.
    pub firing: Option<Q2SelectedFiring>,
}

impl Q2SelectedWeaponTurnState {
    fn base(&self) -> Q2WeaponTurnState {
        Q2WeaponTurnState {
            buttons: self.buttons,
            latched_buttons: self.latched_buttons,
            weapon_thunk: self.weapon_thunk,
        }
    }

    fn apply(&mut self, base: &Q2WeaponTurnState) {
        self.buttons = base.buttons;
        self.latched_buttons = base.latched_buttons;
        self.weapon_thunk = base.weapon_thunk;
    }
}

/// One source inventory definition.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2InventoryDefinition {
    /// Item.
    pub item: ItemId,
    /// Capacity.
    pub capacity: f64,
}

/// One starter inventory entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2LoadoutEntry {
    /// Item.
    pub item: ItemId,
    /// Count.
    pub count: f64,
}

/// Starter loadout.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SelectedLoadout {
    /// Starter weapon item.
    pub weapon: ItemId,
    /// Starter inventory.
    pub inventory: Vec<Q2LoadoutEntry>,
}

/// Weapon observation for one actor.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SelectedObservation {
    /// Weapon owner.
    pub owner: Q2WeaponOwner,
    /// Weapon input.
    pub input: Q2WeaponInput,
}

/// Options for the selected Q2 arsenal. The donor `weapons` service dissolved
/// into the game arena in Rust, so the shared game services carry both.
pub struct Q2SelectedArsenalOptions {
    /// Shared game services.
    pub game: Rc<RefCell<Q2GameServices>>,
    /// Source inventory definitions.
    pub inventory_definitions: Vec<Q2InventoryDefinition>,
    /// Weakest to strongest shared pickup preference.
    pub pickup_order: Option<Vec<ItemId>>,
    /// Replaced items cleared on admit.
    pub replaced_items: Vec<ItemId>,
    /// Starter loadout.
    pub loadout: Option<Q2SelectedLoadout>,
    /// Weapon observation.
    pub observe: Rc<dyn Fn(&ActorId) -> Q2SelectedObservation>,
}

fn provider_name(provider: &ProviderId) -> String {
    format!("{}:{}", provider.namespace, provider.name)
}

fn contract<T>(result: Result<T, Q2ArsenalError>) -> T {
    result.unwrap_or_else(|error| panic!("{error}"))
}

fn phase_word(phase: Q2WeaponPhase) -> i32 {
    match phase {
        Q2WeaponPhase::Ready => 0,
        Q2WeaponPhase::Activating => 1,
        Q2WeaponPhase::Dropping => 2,
        Q2WeaponPhase::Firing => 3,
    }
}

/// Project the selected Q2 arsenal for an actor.
pub fn project_q2_arsenal(
    actor: &ActorId,
    provider: &ProviderId,
    game: &mut Q2GameServices,
) -> Result<ArsenalState, Q2ArsenalError> {
    let state = game.weapons.states.get(actor).ok_or(Q2ArsenalError::NoArsenal)?.clone();
    let active = state.weapon.as_ref().map(|weapon| weapon_definition(game, weapon).item);
    let pending = state
        .pending
        .as_ref()
        .map(|weapon| weapon_definition(game, weapon).item);
    let ammo = game
        .host
        .inventory()
        .entries(actor)
        .into_iter()
        .map(|entry| qa_world::movement::types::InventoryEntry {
            item: entry.item,
            count: entry.count,
        })
        .collect();
    Ok(ArsenalState {
        provider: provider.clone(),
        active_weapon: active,
        ammo,
        state: WeaponState::Q2 {
            gun_frame: state.frame,
            state: phase_word(state.phase),
            pending_weapon: pending,
            machinegun_shots: state.machinegun_shots,
            grenade_time: SourceTime::Seconds(state.grenade_time as f32),
            grenade_blew_up: state.grenade_blew_up,
        },
    })
}

/// Selected Q2 arsenal over the game services.
pub struct Q2SelectedArsenal {
    game: Rc<RefCell<Q2GameServices>>,
    provider: ProviderId,
    definitions: Vec<Q2WeaponDefinition>,
    pickup_order: Vec<ItemId>,
    inventory_definitions: Vec<Q2InventoryDefinition>,
    replaced_items: Vec<ItemId>,
    loadout: Option<Q2SelectedLoadout>,
    observe: Rc<dyn Fn(&ActorId) -> Q2SelectedObservation>,
    turns: HashMap<ActorId, Q2SelectedWeaponTurnState>,
}

impl Q2SelectedArsenal {
    /// Create a selected Q2 arsenal.
    pub fn new(options: Q2SelectedArsenalOptions) -> Result<Self, Q2ArsenalError> {
        let provider = options.game.borrow().options.provider.clone();
        let definitions = registered_weapon_definitions(&options.game.borrow());
        let pickup_order = options.pickup_order.clone().unwrap_or_else(|| {
            base_weapons()
                .iter()
                .map(|weapon| weapon.definition.item.clone())
                .collect()
        });
        if definitions
            .iter()
            .any(|definition| !pickup_order.contains(&definition.item))
        {
            return Err(Q2ArsenalError::PickupPreference);
        }
        for definition in &definitions {
            let items = definition.ammo.as_ref().map_or_else(
                || vec![definition.item.clone()],
                |ammo| vec![definition.item.clone(), ammo.clone()],
            );
            for item in items {
                if !options.inventory_definitions.iter().any(|entry| entry.item == item) {
                    return Err(Q2ArsenalError::MissingInventoryDefinition { item });
                }
            }
        }
        if let Some(loadout) = &options.loadout {
            if !definitions.iter().any(|definition| definition.item == loadout.weapon) {
                return Err(Q2ArsenalError::ForeignStarterWeapon);
            }
            for entry in &loadout.inventory {
                if entry.count.fract() != 0.0
                    || entry.count < 0.0
                    || !definitions.iter().any(|definition| {
                        definition.item == entry.item || definition.ammo.as_deref() == Some(entry.item.as_str())
                    })
                {
                    return Err(Q2ArsenalError::InvalidStarterInventory);
                }
            }
        }
        Ok(Q2SelectedArsenal {
            game: options.game,
            provider,
            definitions,
            pickup_order,
            inventory_definitions: options.inventory_definitions,
            replaced_items: options.replaced_items,
            loadout: options.loadout,
            observe: options.observe,
            turns: HashMap::new(),
        })
    }

    /// Drop the command continuation after a foreign actor release. The Rust
    /// arena has no release subscription; owners call this explicitly.
    pub fn on_actor_released(&mut self, actor: &ActorId) {
        self.turns.remove(actor);
    }

    fn require(&self, actor: &ActorId) -> Result<Q2WeaponState, Q2ArsenalError> {
        self.game
            .borrow()
            .weapons
            .states
            .get(actor)
            .cloned()
            .ok_or(Q2ArsenalError::MissingArsenal)
    }

    fn require_turn(&mut self, actor: &ActorId) -> Result<&mut Q2SelectedWeaponTurnState, Q2ArsenalError> {
        self.turns.get_mut(actor).ok_or(Q2ArsenalError::MissingTurn)
    }

    fn try_read(&self, actor: &ActorId) -> Result<ArsenalState, Q2ArsenalError> {
        let mut game = self.game.borrow_mut();
        let arsenal = project_q2_arsenal(actor, &self.provider, &mut game)?;
        Ok(ArsenalState {
            ammo: arsenal
                .ammo
                .into_iter()
                .filter(|entry| {
                    self.definitions.iter().any(|definition| {
                        definition.item == entry.item || definition.ammo.as_deref() == Some(entry.item.as_str())
                    })
                })
                .collect(),
            ..arsenal
        })
    }

    fn try_admit(&mut self, actor: OwnedActor) -> Result<ArsenalState, Q2ArsenalError> {
        if self.game.borrow().weapons.states.contains_key(actor.id()) {
            return Err(Q2ArsenalError::AlreadyAdmitted);
        }
        let items: Vec<ItemId> = self
            .definitions
            .iter()
            .flat_map(|definition| {
                definition.ammo.as_ref().map_or_else(
                    || vec![definition.item.clone()],
                    |ammo| vec![definition.item.clone(), ammo.clone()],
                )
            })
            .collect();
        {
            let mut game = self.game.borrow_mut();
            let replaced: Vec<InventoryEntry> = game
                .host
                .inventory()
                .entries(actor.id())
                .into_iter()
                .filter(|entry| self.replaced_items.contains(&entry.item) && !items.contains(&entry.item))
                .collect();
            for mut entry in replaced {
                entry.count = 0.0;
                game.host.inventory().configure(&actor, &entry);
            }
            for definition in &self.inventory_definitions.clone() {
                if items.contains(&definition.item)
                    && !game
                        .host
                        .inventory()
                        .entries(actor.id())
                        .iter()
                        .any(|current| current.item == definition.item)
                {
                    game.host.inventory().configure(
                        &actor,
                        &InventoryEntry {
                            item: definition.item.clone(),
                            count: 0.0,
                            capacity: definition.capacity,
                            count_policy: None,
                        },
                    );
                }
            }
            if let Some(loadout) = self.loadout.clone() {
                let current: Vec<InventoryEntry> = game
                    .host
                    .inventory()
                    .entries(actor.id())
                    .into_iter()
                    .filter(|entry| items.contains(&entry.item))
                    .collect();
                for mut entry in current {
                    entry.count = entry.capacity.min(
                        loadout
                            .inventory
                            .iter()
                            .find(|value| value.item == entry.item)
                            .map(|value| value.count)
                            .unwrap_or(0.0),
                    );
                    game.host.inventory().configure(&actor, &entry);
                }
            } else if game
                .host
                .inventory()
                .count(actor.id(), &"q2:weapon_blaster".to_string())
                == 0.0
            {
                game.host
                    .inventory()
                    .give(&actor, &"q2:weapon_blaster".to_string(), 1.0);
            }
        }
        let weapon = match &self.loadout {
            None => Some("blaster".to_string()),
            Some(loadout) => self
                .definitions
                .iter()
                .find(|definition| definition.item == loadout.weapon)
                .map(|definition| definition.name.clone()),
        };
        let owned = match &weapon {
            None => false,
            Some(weapon) => {
                let mut game = self.game.borrow_mut();
                let item = weapon_definition(&game, weapon).item.clone();
                game.host.inventory().count(actor.id(), &item) >= 1.0
            }
        };
        if !owned {
            return Err(Q2ArsenalError::StarterNotOwned);
        }
        {
            let mut game = self.game.borrow_mut();
            bind_player_weapon(&mut game, &actor, Q2WeaponState::new(weapon));
        }
        self.turns.insert(
            actor.id().clone(),
            Q2SelectedWeaponTurnState {
                buttons: 0,
                latched_buttons: 0,
                weapon_thunk: false,
                firing: None,
            },
        );
        self.try_read(actor.id())
    }

    fn try_select(&mut self, actor: &ActorId, item: &ItemId) -> Result<bool, Q2ArsenalError> {
        let definition = self
            .definitions
            .iter()
            .find(|definition| definition.item == *item)
            .cloned();
        let Some(definition) = definition else {
            return Ok(false);
        };
        let observed = (self.observe)(actor);
        let mut game = self.game.borrow_mut();
        let result = request_weapon(&mut game, &observed.owner.actor, &definition.name, false);
        Ok(matches!(
            result,
            qa_content::q2::foundation::weapons::player::Q2WeaponSelection::Selected
                | qa_content::q2::foundation::weapons::player::Q2WeaponSelection::Current
        ))
    }

    fn try_step(
        &mut self,
        input: &WeaponStepInput,
        intent: Option<&ArsenalIntent>,
    ) -> Result<WeaponStepResult, Q2ArsenalError> {
        if intent.is_some_and(|intent| intent.provider != provider_name(&self.provider)) {
            return Err(Q2ArsenalError::ForeignIntent);
        }
        if let Some(weapon) = intent.and_then(|intent| intent.weapon.as_ref()) {
            if !self.definitions.iter().any(|definition| definition.item == *weapon) {
                return Err(Q2ArsenalError::ForeignWeapon);
            }
            self.try_select(input.actor.id(), weapon)?;
        }
        let before = self.try_read(input.actor.id())?;
        let observed = (self.observe)(input.actor.id());
        let game = self.game.clone();
        let latched = (command_buttons(&input.command) & !1) | if observed.input.attack { 1 } else { 0 };
        {
            let turn = self.require_turn(input.actor.id())?;
            let mut base = turn.base();
            latch_q2_weapon_buttons(&mut base, latched);
            turn.apply(&base);
        }
        if !observed.input.spectator {
            let mut base = self.require_turn(input.actor.id())?.base();
            let mut ticked = false;
            {
                let mut game = game.borrow_mut();
                let owner = observed.owner.clone();
                let input = observed.input.clone();
                let thunk = base.weapon_thunk;
                early_q2_weapon_turn(&mut base, &mut |latched_attack| {
                    ticked = true;
                    tick_player_weapon(
                        &mut game,
                        &owner.actor,
                        Q2WeaponInput {
                            latched_attack,
                            weapon_thunk: thunk,
                            ..input.clone()
                        },
                    );
                });
            }
            let _ = ticked;
            self.require_turn(input.actor.id())?.apply(&base);
        }
        {
            let weapon = self
                .game
                .borrow()
                .weapons
                .states
                .get(input.actor.id())
                .and_then(|state| state.weapon.clone());
            let turn = self.require_turn(input.actor.id())?;
            if turn.firing.as_ref().map(|firing| &firing.weapon) != weapon.as_ref() {
                turn.firing = None;
            }
        }
        Ok(WeaponStepResult {
            continuation: None,
            arsenal: if self.game.borrow_mut().host.actors().is_live(input.actor.id()) {
                self.try_read(input.actor.id())?
            } else {
                before
            },
            animation: input.animation.clone(),
            effects: Vec::new(),
        })
    }

    /// Run the frame turn for an actor.
    pub fn frame(&mut self, actor: &ActorId) -> Result<(), Q2ArsenalError> {
        let mut base = self.require_turn(actor)?.base();
        let observed = (self.observe)(actor);
        let state = self.require(actor)?;
        let elapsed = self.game.borrow().host.frame_seconds();
        {
            let turn = self.require_turn(actor)?;
            if turn.firing.as_ref().map(|firing| &firing.weapon) != state.weapon.as_ref() {
                turn.firing = None;
            }
        }
        let edition_classic = self.game.borrow().options.edition == Q2Edition::Classic;
        let health = self
            .game
            .borrow_mut()
            .host
            .combat()
            .read(actor)
            .map(|combat| combat.health)
            .unwrap_or(0.0);
        if edition_classic
            && !observed.input.spectator
            && state.phase == Q2WeaponPhase::Firing
            && state.weapon.is_some()
            && health > 0.0
        {
            let interval = weapon_firing_interval(&mut self.game.borrow_mut(), actor, elapsed);
            if interval != elapsed {
                let turn = self.require_turn(actor)?;
                let firing = turn.firing.get_or_insert_with(|| Q2SelectedFiring {
                    weapon: state.weapon.clone().unwrap(),
                    credit: 0.0,
                });
                firing.credit += elapsed / interval - 1.0;
            }
        }
        {
            let game = self.game.clone();
            let mut ticked = false;
            {
                let mut borrowed = game.borrow_mut();
                let owner = observed.owner.clone();
                let input = observed.input.clone();
                let thunk = base.weapon_thunk;
                begin_q2_weapon_turn(&mut base, !observed.input.spectator, &mut |latched_attack| {
                    ticked = true;
                    tick_player_weapon(
                        &mut borrowed,
                        &owner.actor,
                        Q2WeaponInput {
                            latched_attack,
                            weapon_thunk: thunk,
                            ..input.clone()
                        },
                    );
                });
            }
            let _ = ticked;
            self.require_turn(actor)?.apply(&base);
        }
        let firing = self.require_turn(actor)?.firing.clone();
        let firing_weapon = firing.as_ref().map(|firing| firing.weapon.clone());
        let turn_snapshot = self.require_turn(actor)?.clone();
        while firing.as_ref().is_some_and(|firing| firing.credit >= 1.0)
            && self.game.borrow_mut().host.actors().is_live(actor)
            && self.turns.get(actor) == Some(&turn_snapshot)
            && self.game.borrow().weapons.states.get(actor) == Some(&state)
            && self
                .require(actor)
                .map(|current| {
                    current.weapon.as_ref() == firing_weapon.as_ref() && current.phase == Q2WeaponPhase::Firing
                })
                .unwrap_or(false)
            && self
                .game
                .borrow_mut()
                .host
                .combat()
                .read(actor)
                .map(|combat| combat.health)
                .unwrap_or(0.0)
                > 0.0
        {
            let mut firing = firing.clone().unwrap();
            firing.credit -= 1.0;
            self.require_turn(actor)?.firing = Some(firing.clone());
            let current = (self.observe)(actor);
            if current.input.spectator {
                break;
            }
            {
                let mut game = self.game.borrow_mut();
                tick_player_weapon(
                    &mut game,
                    &current.owner.actor,
                    Q2WeaponInput {
                        latched_attack: false,
                        weapon_thunk: false,
                        ..current.input.clone()
                    },
                );
            }
            let next = self.require_turn(actor)?.firing.clone();
            if next != Some(firing) {
                break;
            }
        }
        if let Some(firing) = firing {
            let current = self.game.borrow().weapons.states.get(actor).cloned();
            if current.as_ref().and_then(|state| state.weapon.clone()) != Some(firing.weapon.clone())
                || current != Some(state)
            {
                self.require_turn(actor)?.firing = None;
            } else if let Some(turn) = self.turns.get_mut(actor) {
                if let Some(credit) = turn.firing.as_mut() {
                    credit.credit %= 1.0;
                }
            }
        }
        if self
            .game
            .borrow_mut()
            .host
            .combat()
            .read(actor)
            .map(|combat| combat.health)
            .unwrap_or(0.0)
            > 0.0
        {
            self.require_turn(actor)?.latched_buttons = 0;
        }
        Ok(())
    }

    /// Capture the weapon turn for an actor.
    pub fn capture_turn(&mut self, actor: &ActorId) -> Result<Q2SelectedWeaponTurnState, Q2ArsenalError> {
        let state = self.require(actor)?;
        let turn = self.require_turn(actor)?;
        Ok(Q2SelectedWeaponTurnState {
            firing: turn
                .firing
                .clone()
                .filter(|firing| firing.weapon == state.weapon.clone().unwrap_or_default()),
            ..turn.clone()
        })
    }

    /// Restore a weapon turn for an actor.
    pub fn restore_turn(&mut self, actor: &ActorId, turn: &Q2SelectedWeaponTurnState) -> Result<(), Q2ArsenalError> {
        let state = self.require(actor)?;
        if let Some(firing) = &turn.firing {
            if Some(&firing.weapon) != state.weapon.as_ref()
                || !firing.credit.is_finite()
                || firing.credit < 0.0
                || firing.credit >= 1.0
            {
                return Err(Q2ArsenalError::InvalidFiringCredit);
            }
        }
        self.turns.insert(actor.clone(), turn.clone());
        Ok(())
    }

    fn try_ui(&self, actor: &ActorId, source: &ProviderReference) -> Result<SelectedArsenalUi, Q2ArsenalError> {
        let state = self.require(actor)?;
        let active = state.weapon.as_ref().map(|weapon| {
            let game = self.game.borrow();
            weapon_definition(&game, weapon)
        });
        let counts: HashMap<ItemId, f64> = self
            .definitions
            .iter()
            .flat_map(|definition| {
                definition.ammo.as_ref().map_or_else(
                    || vec![definition.item.clone()],
                    |ammo| vec![definition.item.clone(), ammo.clone()],
                )
            })
            .map(|item| {
                let count = self.game.borrow_mut().host.inventory().count(actor, &item);
                (item, count)
            })
            .collect();
        let status = |item: &ItemId| counts.get(item).copied().unwrap_or(0.0) as i32;
        let items = self
            .definitions
            .iter()
            .enumerate()
            .map(|(index, definition)| {
                let count = definition
                    .ammo
                    .as_ref()
                    .map(|ammo| counts.get(ammo).copied().unwrap_or(0.0));
                super::super::types::PlayerUiItem {
                    id: definition.item.clone(),
                    label: q2_base_weapon_display_name(&definition.item)
                        .or_else(|| q2_mission_weapon_display_name(&definition.name))
                        .unwrap_or_else(|| definition.name.clone()),
                    kind: super::super::types::PlayerUiItemKind::Weapon,
                    source_ordinal: (index + 1) as f64,
                    owned: counts.get(&definition.item).copied().unwrap_or(0.0) > 0.0,
                    has_ammo: definition.ammo.is_none()
                        || count.is_some_and(|count| count >= f64::from(definition.quantity)),
                    count,
                    warning_count: f64::from(definition.warning),
                }
            })
            .collect();
        Ok(SelectedArsenalUi {
            active_weapon: active.as_ref().map(|active| active.item.clone()),
            ammo: active.as_ref().and_then(|active| {
                active.ammo.as_ref().map(|ammo| super::super::types::UiAmmo {
                    item: ammo.clone(),
                    count: counts.get(ammo).copied().unwrap_or(0.0),
                })
            }),
            items,
            weapon_status: q2_weapon_status(active.as_ref(), status, source.clone()),
            arsenal_warning: super::selected::ArsenalAmmoWarning::None,
        })
    }

    fn try_view(&self, actor: &ActorId) -> Result<Option<ArsenalView>, Q2ArsenalError> {
        let state = self.require(actor)?;
        let Some(weapon) = state.weapon.clone() else {
            return Ok(None);
        };
        if state.primary_handoff == PrimaryHandoff::Holstered {
            return Ok(None);
        }
        let game = self.game.borrow();
        let definition = weapon_definition(&game, &weapon);
        Ok(Some(ArsenalView {
            path: state.view_model.clone().unwrap_or(definition.view_model.clone()),
            frame: f64::from(state.frame),
        }))
    }
}

struct RefusedQ2Request {
    id: u64,
}

impl SourceWeaponRequest for RefusedQ2Request {
    fn id(&self) -> u64 {
        self.id
    }

    fn status(&mut self) -> RequestStatus {
        RequestStatus::Refused
    }

    fn cancel(&mut self) {}
}

struct Q2Handoff {
    game: Rc<RefCell<Q2GameServices>>,
    provider: ProviderId,
    definitions: Vec<Q2WeaponDefinition>,
    observe: Rc<dyn Fn(&ActorId) -> Q2SelectedObservation>,
    actor: ActorId,
}

impl SourceWeaponHandoff for Q2Handoff {
    fn provider(&self) -> ProviderId {
        self.provider.clone()
    }

    fn accepts(&self, item: &ItemId) -> bool {
        let definition = self.definitions.iter().find(|definition| definition.item == *item);
        let Some(definition) = definition else {
            return false;
        };
        let mut game = self.game.borrow_mut();
        game.host.inventory().count(&self.actor, item) > 0.0
            && definition
                .ammo
                .as_ref()
                .is_none_or(|ammo| game.host.inventory().count(&self.actor, ammo) >= f64::from(definition.quantity))
    }

    fn select(&mut self, item: &ItemId) -> bool {
        let definition = self
            .definitions
            .iter()
            .find(|definition| definition.item == *item)
            .cloned();
        let Some(definition) = definition else {
            return false;
        };
        let observed = (self.observe)(&self.actor);
        let mut game = self.game.borrow_mut();
        let result = request_weapon(&mut game, &observed.owner.actor, &definition.name, false);
        matches!(
            result,
            qa_content::q2::foundation::weapons::player::Q2WeaponSelection::Selected
                | qa_content::q2::foundation::weapons::player::Q2WeaponSelection::Current
        )
    }

    fn holster(&mut self) {
        let observed = (self.observe)(&self.actor);
        request_holster(&mut self.game.borrow_mut(), &observed.owner.actor);
    }

    fn is_holstered(&self) -> bool {
        let observed = (self.observe)(&self.actor);
        is_holstered(&self.game.borrow(), &observed.owner.actor.id().clone())
    }

    fn resume(&mut self, item: Option<&ItemId>) -> ResumeOutcome {
        let observation = (self.observe)(&self.actor);
        let definition = match item {
            None => None,
            Some(item) => match self.definitions.iter().find(|definition| definition.item == *item) {
                None => panic!("{}", Q2ArsenalError::ForeignWeapon),
                found => found.cloned(),
            },
        };
        {
            let mut game = self.game.borrow_mut();
            resume_primary(
                &mut game,
                &observation.owner.actor,
                &observation.input,
                definition.as_ref().map(|definition| definition.name.as_str()),
            );
        }
        let active = self
            .game
            .borrow()
            .weapons
            .states
            .get(&self.actor)
            .and_then(|state| state.weapon.clone());
        let active_item = active.as_ref().map(|weapon| {
            let game = self.game.borrow();
            weapon_definition(&game, weapon).item.clone()
        });
        ResumeOutcome::Immediate(item.is_none() || active_item.as_ref() == item)
    }

    fn restore_request(&mut self, id: u64, _item: Option<&ItemId>) -> Box<dyn SourceWeaponRequest> {
        Box::new(RefusedQ2Request { id })
    }

    fn is_deferred(&self) -> bool {
        false
    }
}

impl SelectedArsenal for Q2SelectedArsenal {
    fn family(&self) -> ArsenalFamily {
        ArsenalFamily::Q2
    }

    fn provider(&self) -> ProviderId {
        self.provider.clone()
    }

    fn catalog(&self) -> Vec<SelectedPickupWeapon> {
        self.definitions
            .iter()
            .map(|weapon| SelectedPickupWeapon {
                item: weapon.item.clone(),
                drop: SupplyDrop::Supply,
            })
            .collect()
    }

    fn has(&self, actor: &ActorId) -> bool {
        self.game.borrow().weapons.states.contains_key(actor)
    }

    fn admit(&mut self, actor: OwnedActor, _max_health: f64, _team_deathmatch: bool) -> ArsenalState {
        contract(self.try_admit(actor))
    }

    fn read(&self, actor: &ActorId) -> ArsenalState {
        contract(self.try_read(actor))
    }

    fn select(&mut self, actor: &ActorId, item: &ItemId) -> bool {
        contract(self.try_select(actor, item))
    }

    fn pickup_ammo(&mut self, actor: &OwnedActor, grants: &[PickupAmmoReceipt], auto_switch: bool) {
        if auto_switch
            && grants
                .iter()
                .any(|grant| grant.item == "q2:ammo_grenades" && grant.before == 0.0)
        {
            self.pickup_weapons(actor, &["q2:ammo_grenades".to_string()], PickupSelection::Better);
        }
    }

    fn pickup_weapons(&mut self, actor: &OwnedActor, weapons: &[ItemId], selection: PickupSelection) {
        contract(self.try_pickup_weapons(actor, weapons, selection));
    }

    fn pending_weapon(&self, actor: &ActorId) -> Option<ItemId> {
        let state = contract(self.require(actor));
        state.pending.as_ref().map(|pending| {
            let game = self.game.borrow();
            weapon_definition(&game, pending).item.clone()
        })
    }

    fn handoff(&mut self, actor: &ActorId) -> Box<dyn PrimaryWeaponHandoff> {
        contract(self.require(actor).map(|_| ()));
        Box::new(Q2Handoff {
            game: self.game.clone(),
            provider: self.provider.clone(),
            definitions: self.definitions.clone(),
            observe: self.observe.clone(),
            actor: actor.clone(),
        })
    }

    fn step(&mut self, input: &WeaponStepInput, intent: Option<&ArsenalIntent>) -> WeaponStepResult {
        contract(self.try_step(input, intent))
    }

    fn remove(&mut self, actor: &ActorId) {
        contract(self.try_remove(actor));
    }

    fn ui(&self, actor: &ActorId, source: &ProviderReference) -> SelectedArsenalUi {
        contract(self.try_ui(actor, source))
    }

    fn view(&self, actor: &ActorId) -> Option<ArsenalView> {
        contract(self.try_view(actor))
    }
}

impl Q2SelectedArsenal {
    fn try_pickup_weapons(
        &mut self,
        actor: &OwnedActor,
        items: &[ItemId],
        selection: PickupSelection,
    ) -> Result<(), Q2ArsenalError> {
        if selection == PickupSelection::Never {
            return Ok(());
        }
        for item in items {
            let current = self
                .pending_weapon(actor.id())
                .or_else(|| self.read(actor.id()).active_weapon.clone());
            let rank = |item: &ItemId| {
                self.pickup_order
                    .iter()
                    .position(|order| order == item)
                    .map(|index| index as i32)
                    .unwrap_or(-1)
            };
            if selection == PickupSelection::Always || rank(item) > current.as_ref().map(rank).unwrap_or(-1) {
                self.try_select(actor.id(), item)?;
            }
        }
        Ok(())
    }

    fn try_remove(&mut self, actor: &ActorId) -> Result<(), Q2ArsenalError> {
        let live = self.game.borrow_mut().host.actors().is_live(actor);
        if self.game.borrow().weapons.states.contains_key(actor) && live {
            let observed = (self.observe)(actor);
            let mut game = self.game.borrow_mut();
            let state = game.weapons.states.get(actor).cloned();
            if let Some(state) = state {
                if let Some(weapon) = state.weapon.clone() {
                    let definition = weapon_definition(&game, &weapon).clone();
                    let now = game.host.now();
                    let rerelease = game.options.edition == Q2Edition::Rerelease;
                    let context = Q2WeaponContext {
                        owner: observed.owner.clone(),
                        input: observed.input.clone(),
                        definition,
                        now,
                        rerelease,
                        silenced: false,
                    };
                    let mut state = state;
                    weapon_set_loop(&context, &mut game, &mut state, "");
                    game.weapons.states.insert(actor.clone(), state);
                }
            }
        }
        self.game.borrow_mut().weapons.states.remove(actor);
        self.game.borrow_mut().weapons.inputs.remove(actor);
        self.turns.remove(actor);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use qa_content::contract::{ArmorState, InventoryEntry, PoweredProtectionState, RegularArmorState};
    use qa_content::q2::foundation::host::{
        Q2Edition, Q2FoundationHost, Q2GameOptions, Q2LandmarkCarry, Q2Mode, Q2Motion, Q2PlayerViewState,
        Q2PresentationEvent, Q2Solid, Q2TraceRequest,
    };
    use qa_content::q2::support::contracts::{
        ActorObservation, BodyAttachment, BodyState as Q2BodyState, CombatState as Q2CombatState, CombatTraitChanges,
        DamageOutcome, DamageRequest, LinkedBody, PowerArmorCells, TraceContact, TraceFamily, TraceHit, TraceResult,
        TransitionIntent,
    };
    use qa_content::q2::support::tables::{
        Q2ActorRegistry, Q2BodyTable, Q2CallbackTable, Q2CombatAuthority, Q2InventoryTable,
    };
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{Bounds, Vec3};
    use qa_world::movement::types::{ActorAnimationState, UserCommand};

    use super::*;

    struct MockActors {
        owner: IdentityOwner,
        actors: HashMap<ActorId, OwnedActor>,
        live: HashSet<ActorId>,
    }

    impl MockActors {
        fn new() -> Self {
            MockActors {
                owner: IdentityOwner::create("q2-arsenal-test").unwrap(),
                actors: HashMap::new(),
                live: HashSet::new(),
            }
        }
    }

    impl Q2ActorRegistry for MockActors {
        fn allocate(&mut self, owner: &ProviderId, definition: &str) -> OwnedActor {
            let id = self.owner.actor(self.actors.len() as u32 + 1, 1);
            let owned = self.owner.owned_actor(&id, owner.clone()).unwrap();
            self.live.insert(id.clone());
            self.actors.insert(id, owned.clone());
            let _ = definition;
            owned
        }

        fn allocate_at_source(&mut self, owner: &ProviderId, source_slot: u32, definition: &str) -> OwnedActor {
            let _ = source_slot;
            self.allocate(owner, definition)
        }

        fn source_of(&self, _actor: &ActorId) -> Option<(ProviderId, u32)> {
            None
        }

        fn release(&mut self, actor: &OwnedActor) {
            self.live.remove(actor.id());
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.actors.get(actor).cloned()
        }

        fn observations(&self) -> Vec<ActorObservation> {
            self.actors
                .values()
                .filter(|actor| self.live.contains(actor.id()))
                .map(|actor| ActorObservation {
                    id: actor.id().clone(),
                    owner: ProviderId::new("sim", "test"),
                    definition: "player".to_string(),
                })
                .collect()
        }

        fn resolve_saved(&self, _saved: qa_core::identity::SavedActorId) -> Option<OwnedActor> {
            None
        }

        fn reference_saved(&self, _saved: qa_core::identity::SavedActorId) -> ActorId {
            self.owner.actor(999, 1)
        }

        fn assert_owned(&self, _actor: &OwnedActor) {}
    }

    struct MockInventory {
        stores: HashMap<ActorId, HashMap<ItemId, InventoryEntry>>,
    }

    impl Q2InventoryTable for MockInventory {
        fn create(&mut self, actor: &OwnedActor, entries: &[InventoryEntry]) {
            let store = self.stores.entry(actor.id().clone()).or_default();
            for entry in entries {
                store.insert(entry.item.clone(), entry.clone());
            }
        }

        fn entries(&self, actor: &ActorId) -> Vec<InventoryEntry> {
            self.stores
                .get(actor)
                .map(|store| store.values().cloned().collect())
                .unwrap_or_default()
        }

        fn has(&self, actor: &ActorId) -> bool {
            self.stores.contains_key(actor)
        }

        fn count(&self, actor: &ActorId, item: &ItemId) -> f64 {
            self.stores
                .get(actor)
                .and_then(|store| store.get(item))
                .map(|entry| entry.count)
                .unwrap_or(0.0)
        }

        fn consume(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> bool {
            if self.count(actor.id(), item) < count {
                return false;
            }
            if let Some(entry) = self.stores.get_mut(actor.id()).and_then(|store| store.get_mut(item)) {
                entry.count -= count;
            }
            true
        }

        fn give(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> f64 {
            let store = self.stores.entry(actor.id().clone()).or_default();
            let entry = store.entry(item.clone()).or_insert(InventoryEntry {
                item: item.clone(),
                count: 0.0,
                capacity: f64::MAX,
                count_policy: None,
            });
            entry.count += count;
            count
        }

        fn configure(&mut self, actor: &OwnedActor, entry: &InventoryEntry) {
            self.stores
                .entry(actor.id().clone())
                .or_default()
                .insert(entry.item.clone(), entry.clone());
        }

        fn adjust_source_counter(&mut self, actor: &OwnedActor, item: &ItemId, delta: f64) -> f64 {
            let store = self.stores.entry(actor.id().clone()).or_default();
            let entry = store.entry(item.clone()).or_insert(InventoryEntry {
                item: item.clone(),
                count: 0.0,
                capacity: f64::MAX,
                count_policy: None,
            });
            entry.count += delta;
            entry.count
        }
    }

    #[derive(Default)]
    struct MockBodies {
        states: HashMap<ActorId, Q2BodyState>,
        attachments: HashMap<ActorId, BodyAttachment>,
        linked: HashMap<ActorId, LinkedBody>,
    }

    impl Q2BodyTable for MockBodies {
        fn create(&mut self, actor: &OwnedActor, initial: &Q2BodyState) {
            self.states.insert(actor.id().clone(), initial.clone());
        }

        fn read(&self, actor: &ActorId) -> Option<Q2BodyState> {
            self.states.get(actor).cloned()
        }

        fn write(&mut self, actor: &OwnedActor, state: &Q2BodyState) {
            self.states.insert(actor.id().clone(), state.clone());
        }

        fn attach(&mut self, actor: &OwnedActor, attachment: &BodyAttachment) {
            self.attachments.insert(actor.id().clone(), attachment.clone());
        }

        fn detach(&mut self, actor: &OwnedActor) {
            self.attachments.remove(actor.id());
        }

        fn attachment(&self, actor: &ActorId) -> Option<BodyAttachment> {
            self.attachments.get(actor).cloned()
        }

        fn linked(&self, actor: &ActorId) -> Option<LinkedBody> {
            self.linked.get(actor).cloned()
        }

        fn link(&mut self, actor: &OwnedActor, _origin: Option<Vec3>) {
            if let Some(state) = self.states.get(actor.id()).cloned() {
                self.linked.insert(
                    actor.id().clone(),
                    LinkedBody {
                        actor: actor.id().clone(),
                        absolute_bounds: state.bounds,
                        link_count: 1,
                        state,
                    },
                );
            }
        }

        fn unlink(&mut self, actor: &OwnedActor) {
            self.linked.remove(actor.id());
        }
    }

    #[derive(Default)]
    struct MockCallbacks {
        bound: HashSet<ActorId>,
    }

    impl Q2CallbackTable for MockCallbacks {
        fn bind(&mut self, actor: &OwnedActor) {
            self.bound.insert(actor.id().clone());
        }

        fn unbind(&mut self, actor: &ActorId) {
            self.bound.remove(actor);
        }

        fn is_bound(&self, actor: &ActorId) -> bool {
            self.bound.contains(actor)
        }

        fn forward_use(&mut self, _actor: &OwnedActor, _other: Option<&ActorId>, _activator: Option<&ActorId>) {}
    }

    #[derive(Default)]
    struct MockCombat {
        states: HashMap<ActorId, Q2CombatState>,
    }

    impl Q2CombatAuthority for MockCombat {
        fn create(&mut self, actor: &OwnedActor, initial: &Q2CombatState) {
            self.states.insert(actor.id().clone(), initial.clone());
        }

        fn read(&self, actor: &ActorId) -> Option<Q2CombatState> {
            self.states.get(actor).cloned()
        }

        fn set_health(&mut self, actor: &OwnedActor, health: f64) {
            if let Some(state) = self.states.get_mut(actor.id()) {
                state.health = health;
            }
        }

        fn set_armor(&mut self, actor: &OwnedActor, armor: &ArmorState) {
            if let Some(state) = self.states.get_mut(actor.id()) {
                state.armor = armor.clone();
            }
        }

        fn set_regular_points(&mut self, _actor: &OwnedActor, _points: f64, _initial: Option<&RegularArmorState>) {}

        fn set_regular_armor(&mut self, _actor: &OwnedActor, _regular: &RegularArmorState) {}

        fn set_powered_protection(&mut self, _actor: &OwnedActor, _powered: &PoweredProtectionState) {}

        fn set_traits(&mut self, actor: &OwnedActor, changes: &CombatTraitChanges) {
            if let Some(state) = self.states.get_mut(actor.id()) {
                if let Some(can_take_damage) = changes.can_take_damage {
                    state.can_take_damage = can_take_damage;
                }
                if let Some(mass) = changes.mass {
                    state.mass = mass;
                }
                if let Some(invulnerable) = changes.invulnerable {
                    state.invulnerable = invulnerable;
                }
                if let Some(no_knockback) = changes.no_knockback {
                    state.no_knockback = no_knockback;
                }
            }
        }

        fn bind_power_armor_cells(&mut self, _actor: &OwnedActor, _cells: Box<dyn PowerArmorCells>) {}

        fn apply(&mut self, input: &DamageRequest) -> DamageOutcome {
            DamageOutcome::StaleTarget { request: input.clone() }
        }
    }

    struct MockHost {
        actors: MockActors,
        bodies: MockBodies,
        callbacks: MockCallbacks,
        combat: MockCombat,
        inventory: MockInventory,
    }

    impl Q2FoundationHost for MockHost {
        fn actors(&mut self) -> &mut dyn Q2ActorRegistry {
            &mut self.actors
        }

        fn bodies(&mut self) -> &mut dyn Q2BodyTable {
            &mut self.bodies
        }

        fn callbacks(&mut self) -> &mut dyn Q2CallbackTable {
            &mut self.callbacks
        }

        fn combat(&mut self) -> &mut dyn Q2CombatAuthority {
            &mut self.combat
        }

        fn inventory(&mut self) -> &mut dyn Q2InventoryTable {
            &mut self.inventory
        }

        fn now(&self) -> f64 {
            0.0
        }

        fn frame_seconds(&self) -> f64 {
            0.1
        }

        fn gravity(&self) -> f64 {
            800.0
        }

        fn random(&mut self) -> f64 {
            0.5
        }

        fn schedule(&mut self, _actor: &OwnedActor, _due_seconds: Option<f64>) {}

        fn touch_triggers(&mut self, _actor: &OwnedActor) {}

        fn trace(&mut self, request: &Q2TraceRequest) -> TraceResult {
            TraceResult {
                fraction: 1.0,
                end: request.end,
                start_solid: false,
                all_solid: false,
                contact: TraceContact::None,
                hit: TraceHit::None,
                family: TraceFamily::Q2(qa_content::q2::support::contracts::Q2TraceFields {
                    contents: 0,
                    surface: None,
                    source_plane: qa_content::q2::support::contracts::Q2BspPlane {
                        normal: qa_core::math::vec3(0.0, 0.0, 1.0),
                        distance: 0.0,
                        plane_type: 0,
                        signbits: 0,
                    },
                    secondary: None,
                }),
            }
        }

        fn point_contents(&mut self, _point: Vec3) -> i32 {
            0
        }

        fn in_pvs(&mut self, _first: Vec3, _second: Vec3) -> bool {
            true
        }

        fn in_phs(&mut self, _first: Vec3, _second: Vec3) -> bool {
            true
        }

        fn areas_connected(&mut self, _first: Vec3, _second: Vec3) -> bool {
            true
        }

        fn nearby(&mut self, _origin: Vec3, _radius: f64) -> Vec<ActorId> {
            Vec::new()
        }

        fn players(&mut self) -> Vec<ActorId> {
            Vec::new()
        }

        fn world_actor(&mut self) -> ActorId {
            self.actors.owner.actor(0, 1)
        }

        fn is_player(&mut self, actor: &ActorId) -> bool {
            self.actors.is_live(actor)
        }

        fn is_monster(&mut self, _actor: &ActorId) -> bool {
            false
        }

        fn inline_model_bounds(&mut self, _model: i32) -> Bounds {
            Bounds {
                min: qa_core::math::vec3(0.0, 0.0, 0.0),
                max: qa_core::math::vec3(0.0, 0.0, 0.0),
            }
        }

        fn set_solid(&mut self, _actor: &OwnedActor, _solid: Q2Solid, _model: Option<i32>) {}

        fn set_motion(&mut self, _motion: &Q2Motion) {}

        fn set_area_portal(&mut self, _portal: i32, _open: bool) {}

        fn emit(&mut self, _event: Q2PresentationEvent) {}

        fn player_view_state(&mut self, _player: &ActorId) -> Option<Q2PlayerViewState> {
            None
        }

        fn key_consumed(&mut self, _player: &ActorId) {}

        fn prepare_level_change(&mut self, _map: &str, _landmark: Option<&Q2LandmarkCarry>, _server_flags: i32) {}

        fn transition(&mut self, _intent: TransitionIntent) {}

        fn diagnostic(&mut self, _message: &str) {}
    }

    fn test_game() -> (Rc<RefCell<Q2GameServices>>, OwnedActor) {
        let mut actors = MockActors::new();
        let actor = actors.allocate(&ProviderId::new("sim", "test"), "player");
        let mut game = Q2GameServices::new(
            Box::new(MockHost {
                actors,
                bodies: MockBodies::default(),
                callbacks: MockCallbacks::default(),
                combat: MockCombat::default(),
                inventory: MockInventory { stores: HashMap::new() },
            }),
            Q2GameOptions {
                edition: Q2Edition::Classic,
                map_name: "test".to_string(),
                skill: 1,
                mode: Q2Mode::Singleplayer,
                deathmatch_flags: 0,
                max_clients: 1,
                provider: ProviderId::new("sim", "test"),
                damage_powerup_owner: None,
                source_damage_modifier: None,
                campaign: ProviderId::new("sim", "test"),
                combat_provider: ProviderId::new("sim", "test"),
                inventory_provider: ProviderId::new("sim", "test"),
                movement_provider: ProviderId::new("sim", "test"),
            },
            Vec::new(),
        );
        for weapon in base_weapons() {
            game.weapons
                .definitions
                .insert(weapon.definition.name.clone(), weapon.definition.clone());
        }
        game.entities.insert(
            actor.id().clone(),
            qa_content::q2::foundation::host::Q2Entity::new(
                actor.clone(),
                qa_content::q2::foundation::host::Q2SpawnFields {
                    ordinal: 0,
                    classname: "player".to_string(),
                    values: std::collections::BTreeMap::new(),
                },
            ),
        );
        let game = Rc::new(RefCell::new(game));
        (game, actor)
    }

    fn test_input() -> Q2WeaponInput {
        Q2WeaponInput {
            attack: false,
            latched_attack: false,
            holster: false,
            angles: qa_core::math::vec3(0.0, 0.0, 0.0),
            ducked: false,
            spectator: false,
            notarget: false,
            hand: qa_content::q2::foundation::weapons::types::WeaponHand::Right,
            animate_player: false,
            quad_until: 0.0,
            double_until: 0.0,
            quad_fire_until: 0.0,
            haste: false,
            no_stack_double: false,
            instant_switch: false,
            quick_switch: false,
            infinite_ammo: false,
            players_collide: false,
            gravity: 800.0,
            weapon_thunk: false,
            view_height: 22.0,
        }
    }

    fn arsenal(game: Rc<RefCell<Q2GameServices>>, actor: &OwnedActor) -> Q2SelectedArsenal {
        let owner = actor.clone();
        let definitions: Vec<Q2InventoryDefinition> = base_weapons()
            .iter()
            .flat_map(|weapon| {
                weapon.definition.ammo.as_ref().map_or_else(
                    || vec![weapon.definition.item.clone()],
                    |ammo| vec![weapon.definition.item.clone(), ammo.clone()],
                )
            })
            .map(|item| Q2InventoryDefinition { item, capacity: 100.0 })
            .collect();
        Q2SelectedArsenal::new(Q2SelectedArsenalOptions {
            game,
            inventory_definitions: definitions,
            pickup_order: None,
            replaced_items: Vec::new(),
            loadout: None,
            observe: Rc::new(move |_| Q2SelectedObservation {
                owner: Q2WeaponOwner {
                    actor: owner.clone(),
                    view_height: 22.0,
                },
                input: test_input(),
            }),
        })
        .unwrap()
    }

    #[test]
    fn admits_reads_and_selects() {
        let (game, actor) = test_game();
        let mut arsenal = arsenal(game, &actor);
        assert!(!arsenal.has(actor.id()));
        let admitted = arsenal.admit(actor.clone(), 100.0, false);
        assert!(arsenal.has(actor.id()));
        assert_eq!(admitted.active_weapon.as_deref(), Some("q2:weapon_blaster"));
        assert!(!arsenal.select(actor.id(), &"q2:weapon_shotgun".to_string()));
        assert_eq!(arsenal.pending_weapon(actor.id()), None);
        let ui = arsenal.ui(
            actor.id(),
            &ProviderReference {
                provider: ProviderId::new("sim", "test"),
                content: qa_content::contract::ContentId("q2:base:test:1".to_string()),
            },
        );
        assert!(!ui.items.is_empty());
        assert!(arsenal.view(actor.id()).is_some());
        assert_eq!(arsenal.family(), ArsenalFamily::Q2);
    }

    #[test]
    fn steps_and_turns() {
        let (game, actor) = test_game();
        let mut arsenal = arsenal(game, &actor);
        arsenal.admit(actor.clone(), 100.0, false);
        let input = WeaponStepInput {
            actor: actor.clone(),
            command: UserCommand::Q2Classic(qa_world::movement::types::Q2UserCommand {
                milliseconds: 100,
                angle_shorts: [0, 0, 0],
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0,
                impulse: 0,
                light_level: 0,
            }),
            frame: qa_core::time::FrameContext {
                frame: 1,
                phase: qa_core::time::FramePhase::ClientCommand,
                time: SourceTime::Milliseconds(100),
                elapsed: SourceTime::Milliseconds(100),
            },
            arsenal: arsenal.read(actor.id()),
            animation: ActorAnimationState {
                provider: ProviderId::new("sim", "test"),
                state: qa_world::movement::types::AnimationState::Q2 {
                    frame: 0,
                    end_frame: 0,
                    priority: 0,
                    duck: false,
                    run: false,
                },
            },
            environment: qa_world::movement::types::MovementEnvironment::default(),
            gauntlet_hit: false,
        };
        let stepped = arsenal.step(&input, None);
        assert!(matches!(stepped.arsenal.state, WeaponState::Q2 { .. }));
        let turn = arsenal.capture_turn(actor.id()).unwrap();
        arsenal.restore_turn(actor.id(), &turn).unwrap();
        arsenal.frame(actor.id()).unwrap();
        let handoff = arsenal.handoff(actor.id());
        assert!(handoff.accepts(&"q2:weapon_blaster".to_string()));
        assert!(!handoff.is_holstered());
        arsenal.remove(actor.id());
        assert!(!arsenal.has(actor.id()));
    }

    #[test]
    fn constructor_validates_composition() {
        let (game, actor) = test_game();
        let bad = Q2SelectedArsenalOptions {
            game: game.clone(),
            inventory_definitions: Vec::new(),
            pickup_order: None,
            replaced_items: Vec::new(),
            loadout: None,
            observe: Rc::new(move |_| Q2SelectedObservation {
                owner: Q2WeaponOwner {
                    actor: actor.clone(),
                    view_height: 22.0,
                },
                input: test_input(),
            }),
        };
        assert!(matches!(
            Q2SelectedArsenal::new(bad),
            Err(Q2ArsenalError::MissingInventoryDefinition { .. })
        ));
    }
}
