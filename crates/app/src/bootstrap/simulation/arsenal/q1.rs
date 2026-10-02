//! Selected Quake I arsenal over the source entity services.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/arsenal/q1.ts`
//! (`Q1SelectedArsenalTravel`, `Q1SelectedArsenalOptions`, `Q1SelectedArsenal`).

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use qa_content::contract::InventoryEntry;
use qa_content::contract::{ItemId, ProviderReference, SourceWeaponHandoff as ContractHandoff};
use qa_content::q1::composition::commands::q1_weapon_impulse;
use qa_content::q1::foundation::entity_services::{Q1AttachOptions, Q1EntityServices};
use qa_content::q1::foundation::pickups::{q1_ammo_pickup_selection, q1_weapon_pickup_selection};
use qa_content::q1::foundation::types::{
    is_q1_base_weapon, q1_weapon_bit, Q1AutoSwitch, Q1PlayerState, Q1Weapon, WEAPONS,
};
use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::Vec3;
use qa_core::time::SourceTime;
use qa_net::common::commands::ArsenalIntent;
use qa_world::movement::q3::constants::command_buttons::TALK;
use qa_world::movement::types::{ActorAnimationState, ArsenalState, UserCommand, WeaponState};
use qa_world::pickups::PickupAmmoReceipt;

use super::super::q3_commands::command_buttons;
use super::super::weapon_slot::{
    PrimaryWeaponHandoff, RequestStatus, ResumeOutcome, SourceWeaponHandoff, SourceWeaponRequest,
};
use super::selected::{
    ArsenalFamily, ArsenalView, SelectedArsenal, SelectedArsenalUi, SelectedPickupWeapon, SupplyDrop, WeaponStepInput,
    WeaponStepResult,
};
use super::weapon_status::{q1_weapon_display_name, q1_weapon_status, Q1WeaponStatusSource};
use qa_content::contract::PickupSelection;

/// Errors in the selected Q1 arsenal.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Q1ArsenalError {
    /// Arsenal already admitted.
    #[error("Selected Q1 arsenal already admitted")]
    AlreadyAdmitted,
    /// Actor has no selected Q1 arsenal.
    #[error("Actor has no selected Q1 arsenal")]
    MissingArsenal,
    /// Travel weapon is not owned by this product.
    #[error("Selected Q1 travel weapon is not owned by this product")]
    ForeignTravelWeapon,
    /// Travel contains another component's inventory.
    #[error("Selected Q1 travel contains another component's inventory")]
    ForeignTravelInventory,
    /// Invalid travel extension.
    #[error("Invalid selected Q1 travel extension: {id}")]
    InvalidTravelExtension {
        /// Extension id.
        id: String,
    },
    /// Player has a weapon from another product.
    #[error("Selected Q1 player has a weapon from another product")]
    ForeignWeapon,
    /// Arsenal intent belongs to a different provider.
    #[error("Arsenal intent belongs to a different provider")]
    ForeignIntent,
    /// Weapon does not belong to the selected Q1 product.
    #[error("Weapon does not belong to the selected Q1 product")]
    ForeignProductWeapon,
    /// Source entity services failed.
    #[error(transparent)]
    Game(#[from] qa_content::q1::Q1Error),
}

/// One travel extension payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1TravelExtension {
    /// Extension id.
    pub id: String,
    /// Captured bytes.
    pub bytes: Vec<u8>,
}

/// Selected Q1 arsenal travel: exact donor shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SelectedArsenalTravel {
    /// Traveling weapon.
    pub weapon: Q1Weapon,
    /// Traveling inventory.
    pub inventory: Vec<InventoryEntry>,
    /// Travel extension payloads.
    pub extensions: Vec<Q1TravelExtension>,
}

/// Native player admission.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1NativePlayer {
    /// Starting weapon.
    pub weapon: Q1Weapon,
    /// Maximum health.
    pub max_health: f64,
    /// Autoswitch policy override.
    pub auto_switch: Option<Q1AutoSwitch>,
}

/// Weapon observation for a step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1Observation {
    /// View angles.
    pub view_angles: Vec3,
    /// Water level.
    pub water_level: i32,
}

/// Native player admission hook.
pub type Q1NativePlayerHook = Box<dyn Fn(&ActorId) -> Q1NativePlayer>;
/// Fired-animation hook.
pub type Q1FiredHook = Box<dyn Fn(&ActorId, Q1Weapon, &ActorAnimationState) -> ActorAnimationState>;
/// Impulse hook.
pub type Q1ImpulseHook = Box<dyn Fn(&Q1PlayerState, i32) -> bool>;
/// Pre-pickup hook.
pub type Q1PreparePickupHook = Box<dyn Fn(&Q1PlayerState)>;
/// Weapon observation hook.
pub type Q1ObserveHook = Box<dyn Fn(&ActorId) -> Q1Observation>;

/// Options for the selected Q1 arsenal.
pub struct Q1SelectedArsenalOptions {
    /// Source entity services.
    pub game: Rc<RefCell<Q1EntityServices>>,
    /// Native player admission.
    pub native_player: Option<Q1NativePlayerHook>,
    /// Replaced items cleared on admit.
    pub replaced_items: Vec<ItemId>,
    /// Fired-animation hook.
    pub fired: Option<Q1FiredHook>,
    /// Impulse hook.
    pub impulse: Option<Q1ImpulseHook>,
    /// Pre-pickup hook.
    pub prepare_pickup: Option<Q1PreparePickupHook>,
    /// Weapon observation.
    pub observe: Q1ObserveHook,
}

struct StatusSource {
    game: Rc<RefCell<Q1EntityServices>>,
}

impl Q1WeaponStatusSource for StatusSource {
    fn status_weapon_item(&self, weapon: Q1Weapon) -> ItemId {
        self.game.borrow().weapon_item(weapon)
    }

    fn status_weapon_ammo(&self, weapon: Q1Weapon) -> Option<ItemId> {
        self.game.borrow().weapon_ammo(weapon)
    }

    fn status_ammo_count(&self, actor: &ActorId, item: &ItemId) -> f64 {
        self.game.borrow().host.inventory.count(actor, item)
    }

    fn status_can_fire(&mut self, actor: &ActorId, weapon: Q1Weapon) -> bool {
        let game = self.game.clone();
        let actor = actor.clone();
        self.game
            .borrow_mut()
            .weapon_available(
                &actor,
                weapon,
                qa_content::q1::foundation::entity_services::Q1WeaponPurpose::Fire,
                Some(&|item| game.borrow().host.inventory.count(&actor, item)),
            )
            .unwrap_or_else(|error| panic!("{error}"))
    }
}

fn provider_name(provider: &ProviderId) -> String {
    format!("{}:{}", provider.namespace, provider.name)
}

fn contract<T>(result: Result<T, Q1ArsenalError>) -> T {
    result.unwrap_or_else(|error| panic!("{error}"))
}

/// Selected Q1 arsenal over the source entity services.
pub struct Q1SelectedArsenal {
    game: Rc<RefCell<Q1EntityServices>>,
    provider: ProviderId,
    weapons: Vec<Q1Weapon>,
    native_player: Option<Q1NativePlayerHook>,
    replaced_items: Vec<ItemId>,
    fired: Option<Q1FiredHook>,
    impulse_hook: Option<Q1ImpulseHook>,
    prepare_pickup: Option<Q1PreparePickupHook>,
    observe: Q1ObserveHook,
}

impl Q1SelectedArsenal {
    /// Create a selected Q1 arsenal.
    pub fn new(options: Q1SelectedArsenalOptions) -> Self {
        let game = options.game;
        let provider = game.borrow().provider();
        let mut seen = HashSet::new();
        let mut weapons = Vec::new();
        for weapon in WEAPONS
            .iter()
            .map(|base| Q1Weapon::from(*base))
            .chain(game.borrow().registered_weapons.keys().copied().collect::<Vec<_>>())
        {
            if seen.insert(weapon) {
                weapons.push(weapon);
            }
        }
        Q1SelectedArsenal {
            game,
            provider,
            weapons,
            native_player: options.native_player,
            replaced_items: options.replaced_items,
            fired: options.fired,
            impulse_hook: options.impulse,
            prepare_pickup: options.prepare_pickup,
            observe: options.observe,
        }
    }

    /// Source entity services.
    pub fn game(&self) -> Rc<RefCell<Q1EntityServices>> {
        self.game.clone()
    }

    fn player(&self, actor: &ActorId) -> Result<Q1PlayerState, Q1ArsenalError> {
        self.game
            .borrow()
            .players
            .get(actor)
            .cloned()
            .ok_or(Q1ArsenalError::MissingArsenal)
    }

    fn try_read(&self, actor: &ActorId) -> Result<ArsenalState, Q1ArsenalError> {
        let game = self.game.borrow();
        let player = game.players.get(actor).ok_or(Q1ArsenalError::MissingArsenal)?;
        if !self.weapons.contains(&player.weapon) {
            return Err(Q1ArsenalError::ForeignWeapon);
        }
        Ok(ArsenalState {
            provider: self.provider.clone(),
            active_weapon: Some(game.weapon_item(player.weapon)),
            ammo: game
                .host
                .inventory
                .entries(actor)
                .into_iter()
                .filter(|entry| {
                    self.weapons.iter().any(|weapon| {
                        game.weapon_item(*weapon) == entry.item
                            || game.weapon_ammo(*weapon).as_deref() == Some(entry.item.as_str())
                    })
                })
                .map(|entry| qa_world::movement::types::InventoryEntry {
                    item: entry.item,
                    count: entry.count,
                })
                .collect(),
            state: WeaponState::Q1 {
                frame: player.weapon_frame,
                attack_finished_seconds: player.attack_finished,
                source_weapon: if is_q1_base_weapon(player.weapon) {
                    match player.weapon {
                        Q1Weapon::Axe => q1_weapon_bit(qa_content::q1::foundation::types::Q1BaseWeapon::Axe),
                        Q1Weapon::Shotgun => q1_weapon_bit(qa_content::q1::foundation::types::Q1BaseWeapon::Shotgun),
                        Q1Weapon::Supershotgun => {
                            q1_weapon_bit(qa_content::q1::foundation::types::Q1BaseWeapon::Supershotgun)
                        }
                        Q1Weapon::Nailgun => q1_weapon_bit(qa_content::q1::foundation::types::Q1BaseWeapon::Nailgun),
                        Q1Weapon::Supernailgun => {
                            q1_weapon_bit(qa_content::q1::foundation::types::Q1BaseWeapon::Supernailgun)
                        }
                        Q1Weapon::Grenadelauncher => {
                            q1_weapon_bit(qa_content::q1::foundation::types::Q1BaseWeapon::Grenadelauncher)
                        }
                        Q1Weapon::Rocketlauncher => {
                            q1_weapon_bit(qa_content::q1::foundation::types::Q1BaseWeapon::Rocketlauncher)
                        }
                        Q1Weapon::Lightning => {
                            q1_weapon_bit(qa_content::q1::foundation::types::Q1BaseWeapon::Lightning)
                        }
                        _ => 0,
                    }
                } else {
                    0
                },
            },
        })
    }

    fn try_admit(&mut self, actor: OwnedActor, max_health: f64) -> Result<ArsenalState, Q1ArsenalError> {
        if self.game.borrow().players.contains_key(actor.id()) {
            return Err(Q1ArsenalError::AlreadyAdmitted);
        }
        let replaced: Vec<InventoryEntry> = self
            .game
            .borrow()
            .host
            .inventory
            .entries(actor.id())
            .into_iter()
            .filter(|entry| self.replaced_items.contains(&entry.item))
            .collect();
        for mut entry in replaced {
            entry.count = 0.0;
            self.game.borrow_mut().host.inventory.configure(&actor, &entry)?;
        }
        let native = self.native_player.as_ref().map(|native| native(actor.id()));
        if native.is_none() {
            self.game.borrow_mut().initialize_weapon_inventory(&actor)?;
        }
        let registered: Vec<Q1Weapon> = self.game.borrow().registered_weapons.keys().copied().collect();
        for weapon in registered {
            let item = self.game.borrow().weapon_item(weapon);
            let count = self.game.borrow().host.inventory.count(actor.id(), &item);
            self.game.borrow_mut().host.inventory.configure(
                &actor,
                &InventoryEntry {
                    item,
                    count,
                    capacity: 1.0,
                    count_policy: None,
                },
            )?;
        }
        self.game.borrow_mut().attach_player(
            &actor,
            &Q1AttachOptions {
                weapon: native.as_ref().map(|native| native.weapon),
                initialize_inventory: false,
                max_health: Some(native.as_ref().map(|native| native.max_health).unwrap_or(max_health)),
            },
        )?;
        if let Some(auto_switch) = native.as_ref().and_then(|native| native.auto_switch) {
            self.game.borrow_mut().update_player(actor.id(), |player| {
                player.auto_switch = auto_switch;
            })?;
        }
        self.try_read(actor.id())
    }

    /// Capture travel for an actor.
    pub fn capture_travel(&self, actor: &ActorId) -> Result<Q1SelectedArsenalTravel, Q1ArsenalError> {
        let player = self.player(actor)?;
        // The services keep player extensions private without a travel
        // accessor; captures carry no extension payloads until the content
        // partition exposes one.
        Ok(Q1SelectedArsenalTravel {
            weapon: player.weapon,
            inventory: self.try_read(actor).map(|arsenal| {
                let game = self.game.borrow();
                game.host
                    .inventory
                    .entries(actor)
                    .into_iter()
                    .filter(|entry| arsenal.ammo.iter().any(|ammo| ammo.item == entry.item))
                    .collect()
            })?,
            extensions: Vec::new(),
        })
    }

    /// Admit travel for an actor.
    pub fn admit_travel(
        &mut self,
        actor: OwnedActor,
        max_health: f64,
        travel: &Q1SelectedArsenalTravel,
    ) -> Result<ArsenalState, Q1ArsenalError> {
        if self.game.borrow().players.contains_key(actor.id()) {
            return Err(Q1ArsenalError::AlreadyAdmitted);
        }
        let owned = {
            let game = self.game.borrow();
            self.weapons.contains(&travel.weapon)
                && travel
                    .inventory
                    .iter()
                    .any(|entry| entry.item == game.weapon_item(travel.weapon) && entry.count > 0.0)
        };
        if !owned {
            return Err(Q1ArsenalError::ForeignTravelWeapon);
        }
        {
            let game = self.game.borrow();
            for entry in &travel.inventory {
                let known = self.weapons.iter().any(|weapon| {
                    game.weapon_item(*weapon) == entry.item
                        || game.weapon_ammo(*weapon).as_deref() == Some(entry.item.as_str())
                });
                if !known {
                    return Err(Q1ArsenalError::ForeignTravelInventory);
                }
            }
        }
        if let Some(extension) = travel.extensions.first() {
            return Err(Q1ArsenalError::InvalidTravelExtension {
                id: extension.id.clone(),
            });
        }
        self.game.borrow_mut().attach_player(
            &actor,
            &Q1AttachOptions {
                weapon: Some(travel.weapon),
                initialize_inventory: false,
                max_health: Some(max_health),
            },
        )?;
        for entry in &travel.inventory {
            let capacity = self
                .game
                .borrow_mut()
                .inventory_capacity(actor.id(), &entry.item)?
                .unwrap_or(entry.capacity);
            let mut restored = entry.clone();
            restored.capacity = capacity;
            self.game.borrow_mut().host.inventory.configure(&actor, &restored)?;
        }
        self.try_read(actor.id())
    }

    fn try_select(&mut self, actor: &ActorId, item: &ItemId) -> Result<bool, Q1ArsenalError> {
        let player = self.player(actor)?;
        let weapon = self
            .weapons
            .iter()
            .find(|weapon| self.game.borrow().weapon_item(**weapon) == *item)
            .copied();
        let Some(weapon) = weapon else {
            return Ok(false);
        };
        Ok(self.game.borrow_mut().select_weapon(&player.actor, weapon)?)
    }

    /// Send an impulse for an actor.
    pub fn impulse(&mut self, actor: &ActorId, value: i32) -> Result<bool, Q1ArsenalError> {
        let player = self.player(actor)?;
        if value == 0 || self.game.borrow().time < player.attack_finished {
            return Ok(false);
        }
        if let Some(impulse) = &self.impulse_hook {
            return Ok(impulse(&player, value));
        }
        self.game.borrow_mut().pipe_impulse(&player, value)
    }

    fn try_pickup_ammo(
        &mut self,
        actor: &OwnedActor,
        grants: &[PickupAmmoReceipt],
        auto_switch: bool,
    ) -> Result<(), Q1ArsenalError> {
        let player = self.player(actor.id())?;
        if let Some(prepare) = &self.prepare_pickup {
            prepare(&player);
        }
        let before = {
            let grants = grants.to_vec();
            let game = self.game.clone();
            let id = actor.id().clone();
            self.game.borrow_mut().choose_best(
                actor,
                Some(&|item| {
                    grants
                        .iter()
                        .find(|grant| grant.item == *item)
                        .map(|grant| grant.before)
                        .unwrap_or_else(|| game.borrow().host.inventory.count(&id, item))
                }),
            )?
        };
        let native_auto = self.native_player.as_ref().map(|native| native(actor.id()).auto_switch);
        let auto = auto_switch && native_auto.flatten().unwrap_or(player.auto_switch) != Q1AutoSwitch::Never;
        self.game.borrow_mut().choose_after_pickup(actor, before, auto)
    }

    fn try_pickup_weapons(
        &mut self,
        actor: &OwnedActor,
        weapons: &[ItemId],
        selection: PickupSelection,
    ) -> Result<(), Q1ArsenalError> {
        let player = self.player(actor.id())?;
        if let Some(prepare) = &self.prepare_pickup {
            prepare(&player);
        }
        let native_auto = self.native_player.as_ref().map(|native| native(actor.id()).auto_switch);
        let never = native_auto.flatten().unwrap_or(player.auto_switch) == Q1AutoSwitch::Never;
        for item in weapons {
            let weapon = self
                .weapons
                .iter()
                .find(|weapon| self.game.borrow().weapon_item(**weapon) == *item)
                .copied();
            if let Some(weapon) = weapon {
                let selection = if never { PickupSelection::Never } else { selection };
                self.game.borrow_mut().weapon_pickup(actor, weapon, selection)?;
            }
        }
        Ok(())
    }

    fn try_step(
        &mut self,
        input: &WeaponStepInput,
        intent: Option<&ArsenalIntent>,
    ) -> Result<WeaponStepResult, Q1ArsenalError> {
        if intent.is_some_and(|intent| intent.provider != provider_name(&self.provider)) {
            return Err(Q1ArsenalError::ForeignIntent);
        }
        if let Some(weapon) = intent.and_then(|intent| intent.weapon.as_ref()) {
            if !self
                .weapons
                .iter()
                .any(|candidate| self.game.borrow().weapon_item(*candidate) == *weapon)
            {
                return Err(Q1ArsenalError::ForeignProductWeapon);
            }
            let current = self.game.borrow().weapon_item(self.player(input.actor.id())?.weapon);
            if current != *weapon {
                self.try_select(input.actor.id(), weapon)?;
            }
        }
        let before = self.try_read(input.actor.id())?;
        let observation = (self.observe)(input.actor.id());
        let seconds = match input.frame.time {
            SourceTime::Seconds(value) => f64::from(value),
            SourceTime::Milliseconds(value) => f64::from(value) / 1000.0,
        };
        let buttons = command_buttons(&input.command);
        let pressed = buttons & 1 != 0 && (!matches!(input.command, UserCommand::Q3(_)) || buttons & TALK == 0);
        let weapon = self.player(input.actor.id())?.weapon;
        let fired = self.weapon_input(&input.actor, pressed, observation, seconds)?;
        let live = self.game.borrow().host.actors.is_live(input.actor.id());
        let animation = if fired && live {
            self.fired
                .as_ref()
                .map(|fired| fired(input.actor.id(), weapon, &input.animation))
                .unwrap_or_else(|| input.animation.clone())
        } else {
            input.animation.clone()
        };
        Ok(WeaponStepResult {
            continuation: None,
            arsenal: if live { self.try_read(input.actor.id())? } else { before },
            animation,
            effects: Vec::new(),
        })
    }

    /// Donor `weaponInput`: latch attack-held, then run the source attack.
    fn weapon_input(
        &mut self,
        actor: &OwnedActor,
        pressed: bool,
        observation: Q1Observation,
        seconds: f64,
    ) -> Result<bool, Q1ArsenalError> {
        self.game.borrow_mut().time = seconds;
        self.game.borrow_mut().update_player(actor.id(), |player| {
            player.view_angles = observation.view_angles;
            player.water_level = observation.water_level;
            player.attack_held = pressed;
            if !pressed && player.continuous_firing {
                player.continuous_firing = false;
                player.weapon_animation_at = -1.0;
                player.weapon_frame = 0;
            }
        })?;
        if !pressed {
            return Ok(false);
        }
        Ok(self
            .game
            .borrow_mut()
            .attack(actor, observation.view_angles, seconds, observation.water_level)?)
    }

    /// Run weapon frames for every player.
    pub fn frame(&mut self, seconds: f64) -> Result<(), Q1ArsenalError> {
        let actors: Vec<OwnedActor> = self
            .game
            .borrow()
            .players
            .values()
            .map(|player| player.actor.clone())
            .collect();
        for actor in actors {
            self.game.borrow_mut().weapon_frame(&actor, seconds)?;
        }
        Ok(())
    }

    fn try_ui(&self, actor: &ActorId, source: &ProviderReference) -> Result<SelectedArsenalUi, Q1ArsenalError> {
        let player = self.player(actor)?;
        let ammo = self.game.borrow().weapon_ammo(player.weapon);
        let mut status = StatusSource {
            game: self.game.clone(),
        };
        let weapon_status = q1_weapon_status(&mut status, actor, player.weapon, source.clone());
        let items = self
            .weapons
            .clone()
            .into_iter()
            .enumerate()
            .map(|(index, weapon)| {
                let game = self.game.clone();
                let owner = actor.clone();
                let item = self.game.borrow().weapon_item(weapon);
                let ammo = self.game.borrow().weapon_ammo(weapon);
                let count = ammo
                    .as_ref()
                    .map(|ammo| self.game.borrow().host.inventory.count(actor, ammo));
                let owned = self.game.borrow().host.inventory.count(actor, &item) > 0.0;
                let has_ammo = self.game.borrow_mut().weapon_available(
                    actor,
                    weapon,
                    qa_content::q1::foundation::entity_services::Q1WeaponPurpose::Best,
                    Some(&|item| game.borrow().host.inventory.count(&owner, item)),
                )?;
                Ok(super::super::types::PlayerUiItem {
                    id: item.clone(),
                    label: q1_weapon_display_name(weapon),
                    kind: super::super::types::PlayerUiItemKind::Weapon,
                    source_ordinal: (index + 1) as f64,
                    owned,
                    has_ammo,
                    count,
                    warning_count: 0.0,
                })
            })
            .collect::<Result<Vec<_>, Q1ArsenalError>>()?;
        Ok(SelectedArsenalUi {
            active_weapon: Some(self.game.borrow().weapon_item(player.weapon)),
            ammo: ammo.map(|ammo| super::super::types::UiAmmo {
                count: self.game.borrow().host.inventory.count(actor, &ammo),
                item: ammo,
            }),
            items,
            weapon_status: Some(weapon_status),
            arsenal_warning: super::selected::ArsenalAmmoWarning::None,
        })
    }

    fn try_view(&self, actor: &ActorId) -> Result<ArsenalView, Q1ArsenalError> {
        let player = self.player(actor)?;
        let path = self.game.borrow_mut().weapon_model(player.weapon, Some(actor))?;
        Ok(ArsenalView {
            path,
            frame: f64::from(player.weapon_frame),
        })
    }
}

struct Q1Handoff {
    inner: ContractHandoff,
}

impl SourceWeaponHandoff for Q1Handoff {
    fn provider(&self) -> ProviderId {
        match &self.inner {
            ContractHandoff::Immediate(handoff) => handoff.provider().clone(),
            ContractHandoff::SourceInput(handoff) => handoff.provider().clone(),
        }
    }

    fn accepts(&self, item: &ItemId) -> bool {
        match &self.inner {
            ContractHandoff::Immediate(handoff) => handoff.accepts(item),
            ContractHandoff::SourceInput(handoff) => handoff.accepts(item),
        }
    }

    fn select(&mut self, item: &ItemId) -> bool {
        match &self.inner {
            ContractHandoff::Immediate(handoff) => handoff.select(item),
            ContractHandoff::SourceInput(handoff) => handoff.select(item),
        }
    }

    fn holster(&mut self) {
        match &self.inner {
            ContractHandoff::Immediate(handoff) => handoff.holster(),
            ContractHandoff::SourceInput(handoff) => handoff.holster(),
        }
    }

    fn is_holstered(&self) -> bool {
        match &self.inner {
            ContractHandoff::Immediate(handoff) => handoff.is_holstered(),
            ContractHandoff::SourceInput(handoff) => handoff.is_holstered(),
        }
    }

    fn resume(&mut self, item: Option<&ItemId>) -> ResumeOutcome {
        match &self.inner {
            ContractHandoff::Immediate(handoff) => ResumeOutcome::Immediate(handoff.resume(item)),
            ContractHandoff::SourceInput(handoff) => ResumeOutcome::Deferred(Box::new(Q1Request {
                inner: handoff.resume(item),
            })),
        }
    }

    fn restore_request(&mut self, id: u64, item: Option<&ItemId>) -> Box<dyn SourceWeaponRequest> {
        match &self.inner {
            ContractHandoff::Immediate(_) => Box::new(RefusedQ1Request { id }),
            ContractHandoff::SourceInput(handoff) => Box::new(Q1Request {
                inner: handoff.restore_request(id, item),
            }),
        }
    }

    fn is_deferred(&self) -> bool {
        matches!(self.inner, ContractHandoff::SourceInput(_))
    }
}

struct Q1Request {
    inner: Box<dyn qa_content::contract::SourceWeaponRequest>,
}

impl SourceWeaponRequest for Q1Request {
    fn id(&self) -> u64 {
        self.inner.id()
    }

    fn status(&mut self) -> RequestStatus {
        match self.inner.status() {
            qa_content::contract::SourceWeaponRequestStatus::Pending => RequestStatus::Pending,
            qa_content::contract::SourceWeaponRequestStatus::Accepted => RequestStatus::Accepted,
            qa_content::contract::SourceWeaponRequestStatus::Refused => RequestStatus::Refused,
        }
    }

    fn cancel(&mut self) {
        self.inner.cancel();
    }
}

struct RefusedQ1Request {
    id: u64,
}

impl SourceWeaponRequest for RefusedQ1Request {
    fn id(&self) -> u64 {
        self.id
    }

    fn status(&mut self) -> RequestStatus {
        RequestStatus::Refused
    }

    fn cancel(&mut self) {}
}

impl SelectedArsenal for Q1SelectedArsenal {
    fn family(&self) -> ArsenalFamily {
        ArsenalFamily::Q1
    }

    fn provider(&self) -> ProviderId {
        self.provider.clone()
    }

    fn catalog(&self) -> Vec<SelectedPickupWeapon> {
        let game = self.game.borrow();
        self.weapons
            .iter()
            .map(|weapon| SelectedPickupWeapon {
                item: game.weapon_item(*weapon),
                drop: if *weapon == Q1Weapon::RogueGrapple || *weapon == Q1Weapon::CtfGrapple {
                    SupplyDrop::None
                } else {
                    SupplyDrop::Supply
                },
            })
            .collect()
    }

    fn has(&self, actor: &ActorId) -> bool {
        self.game.borrow().players.contains_key(actor)
    }

    fn admit(&mut self, actor: OwnedActor, max_health: f64, _team_deathmatch: bool) -> ArsenalState {
        contract(self.try_admit(actor, max_health))
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

    fn pending_weapon(&self, _actor: &ActorId) -> Option<ItemId> {
        contract(self.player(_actor).map(|_| None))
    }

    fn handoff(&mut self, actor: &ActorId) -> Box<dyn PrimaryWeaponHandoff> {
        let player = contract(self.player(actor));
        let inner = contract(
            self.game
                .borrow_mut()
                .primary_weapon_handoff(&player.actor)
                .map_err(Q1ArsenalError::Game),
        );
        Box::new(Q1Handoff { inner })
    }

    fn step(&mut self, input: &WeaponStepInput, intent: Option<&ArsenalIntent>) -> WeaponStepResult {
        contract(self.try_step(input, intent))
    }

    fn remove(&mut self, actor: &ActorId) {
        self.game.borrow_mut().players.remove(actor);
    }

    fn ui(&self, actor: &ActorId, source: &ProviderReference) -> SelectedArsenalUi {
        contract(self.try_ui(actor, source))
    }

    fn view(&self, actor: &ActorId) -> Option<ArsenalView> {
        Some(contract(self.try_view(actor)))
    }
}

trait Q1ArsenalGame {
    fn pipe_impulse(&mut self, player: &Q1PlayerState, value: i32) -> Result<bool, Q1ArsenalError>;
    fn choose_after_pickup(&mut self, actor: &OwnedActor, before: Q1Weapon, auto: bool) -> Result<(), Q1ArsenalError>;
    fn weapon_pickup(
        &mut self,
        actor: &OwnedActor,
        weapon: Q1Weapon,
        selection: PickupSelection,
    ) -> Result<(), Q1ArsenalError>;
}

impl Q1ArsenalGame for Q1EntityServices {
    fn pipe_impulse(&mut self, player: &Q1PlayerState, value: i32) -> Result<bool, Q1ArsenalError> {
        Ok(q1_weapon_impulse(self, player, value)?)
    }

    fn choose_after_pickup(&mut self, actor: &OwnedActor, before: Q1Weapon, auto: bool) -> Result<(), Q1ArsenalError> {
        Ok(q1_ammo_pickup_selection(self, actor.id(), before, auto)?)
    }

    fn weapon_pickup(
        &mut self,
        actor: &OwnedActor,
        weapon: Q1Weapon,
        selection: PickupSelection,
    ) -> Result<(), Q1ArsenalError> {
        Ok(q1_weapon_pickup_selection(self, actor.id(), weapon, selection)?)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use qa_content::contract::InventoryEntry;
    use qa_content::q1::foundation::host::{
        Q1ActorCallbackTable, Q1GameplayAuthority, Q1SessionActorRegistry, Q1SharedBodyTable, Q1SharedInventoryTable,
    };
    use qa_core::identity::{IdentityOwner, SavedActorId};

    use super::*;

    struct MockActors {
        owner: IdentityOwner,
        actors: HashMap<ActorId, OwnedActor>,
        live: HashSet<ActorId>,
    }

    impl MockActors {
        fn new() -> Self {
            MockActors {
                owner: IdentityOwner::create("q1-arsenal-test").unwrap(),
                actors: HashMap::new(),
                live: HashSet::new(),
            }
        }
    }

    impl Q1SessionActorRegistry for MockActors {
        fn allocate_at_source(
            &mut self,
            owner: &ProviderId,
            source_slot: u32,
            _definition: &str,
        ) -> Result<OwnedActor, qa_content::q1::Q1Error> {
            let id = self.owner.actor(source_slot + 100, 1);
            let owned = self.owner.owned_actor(&id, owner.clone()).unwrap();
            self.live.insert(id.clone());
            self.actors.insert(id, owned.clone());
            Ok(owned)
        }

        fn assert_owned(&self, actor: &OwnedActor) -> Result<(), qa_content::q1::Q1Error> {
            assert!(self.actors.contains_key(actor.id()));
            Ok(())
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.actors.get(actor).cloned()
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn observations(&self) -> Vec<qa_content::q1::foundation::gameplay::ActorObservation> {
            Vec::new()
        }

        fn source_of(&self, _actor: &ActorId) -> Option<qa_content::q1::foundation::gameplay::SourceSlot> {
            None
        }

        fn release(&mut self, actor: &OwnedActor) -> Result<(), qa_content::q1::Q1Error> {
            self.live.remove(actor.id());
            Ok(())
        }

        fn resolve_saved(&self, _saved: &SavedActorId) -> Option<OwnedActor> {
            None
        }

        fn reference_saved(&mut self, _saved: &SavedActorId) -> ActorId {
            self.owner.actor(999, 1)
        }
    }

    struct MockInventory {
        stores: HashMap<ActorId, HashMap<ItemId, InventoryEntry>>,
    }

    impl MockInventory {
        fn new() -> Self {
            MockInventory { stores: HashMap::new() }
        }
    }

    impl Q1SharedInventoryTable for MockInventory {
        fn create(&mut self, actor: &OwnedActor, entries: &[InventoryEntry]) -> Result<(), qa_content::q1::Q1Error> {
            let store = self.stores.entry(actor.id().clone()).or_default();
            for entry in entries {
                store.insert(entry.item.clone(), entry.clone());
            }
            Ok(())
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
            let have = self.count(actor.id(), item);
            if have < count {
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

        fn configure(&mut self, actor: &OwnedActor, entry: &InventoryEntry) -> Result<(), qa_content::q1::Q1Error> {
            self.stores
                .entry(actor.id().clone())
                .or_default()
                .insert(entry.item.clone(), entry.clone());
            Ok(())
        }

        fn adjust_source_counter(
            &mut self,
            actor: &OwnedActor,
            item: &ItemId,
            delta: f64,
        ) -> Result<f64, qa_content::q1::Q1Error> {
            let store = self.stores.entry(actor.id().clone()).or_default();
            let entry = store.entry(item.clone()).or_insert(InventoryEntry {
                item: item.clone(),
                count: 0.0,
                capacity: f64::MAX,
                count_policy: None,
            });
            entry.count += delta;
            Ok(entry.count)
        }
    }

    struct MockBodies;
    impl Q1SharedBodyTable for MockBodies {
        fn create(
            &mut self,
            _actor: &OwnedActor,
            _initial: &qa_content::q1::foundation::gameplay::BodyState,
        ) -> Result<(), qa_content::q1::Q1Error> {
            Ok(())
        }
        fn read(&self, _actor: &ActorId) -> Option<qa_content::q1::foundation::gameplay::BodyState> {
            None
        }
        fn write(
            &mut self,
            _actor: &OwnedActor,
            _state: &qa_content::q1::foundation::gameplay::BodyState,
        ) -> Result<(), qa_content::q1::Q1Error> {
            Ok(())
        }
        fn link(&mut self, _actor: &OwnedActor) -> Result<(), qa_content::q1::Q1Error> {
            Ok(())
        }
        fn linked(&self, _actor: &ActorId) -> Option<qa_content::q1::foundation::gameplay::LinkedBody> {
            None
        }
        fn attach(
            &mut self,
            _actor: &OwnedActor,
            _attachment: &qa_content::q1::foundation::gameplay::BodyAttachment,
        ) -> Result<(), qa_content::q1::Q1Error> {
            Ok(())
        }
        fn detach(&mut self, _actor: &OwnedActor) -> Result<(), qa_content::q1::Q1Error> {
            Ok(())
        }
    }

    struct MockCallbacks;
    impl Q1ActorCallbackTable for MockCallbacks {
        fn bind(&mut self, _actor: &OwnedActor) {}
        fn unbind(&mut self, _actor: &OwnedActor) {}
        fn is_bound(&self, _actor: &ActorId) -> bool {
            false
        }
    }

    struct MockCombat {
        states: HashMap<ActorId, qa_content::q1::foundation::gameplay::CombatState>,
    }
    impl Q1GameplayAuthority for MockCombat {
        fn create(
            &mut self,
            actor: &OwnedActor,
            initial: &qa_content::q1::foundation::gameplay::CombatState,
        ) -> Result<(), qa_content::q1::Q1Error> {
            self.states.insert(actor.id().clone(), initial.clone());
            Ok(())
        }
        fn read(&self, actor: &ActorId) -> Option<qa_content::q1::foundation::gameplay::CombatState> {
            self.states.get(actor).cloned()
        }
        fn set_health(&mut self, actor: &OwnedActor, health: f64) -> Result<(), qa_content::q1::Q1Error> {
            if let Some(state) = self.states.get_mut(actor.id()) {
                state.health = health;
            }
            Ok(())
        }
        fn set_armor(
            &mut self,
            _actor: &OwnedActor,
            _armor: &qa_content::contract::ArmorState,
        ) -> Result<(), qa_content::q1::Q1Error> {
            Ok(())
        }
        fn set_traits(
            &mut self,
            _actor: &OwnedActor,
            _traits: qa_content::q1::foundation::gameplay::CombatTraits,
        ) -> Result<(), qa_content::q1::Q1Error> {
            Ok(())
        }
        fn set_regular_armor(
            &mut self,
            _actor: &OwnedActor,
            _regular: &qa_content::contract::RegularArmorState,
        ) -> Result<(), qa_content::q1::Q1Error> {
            Ok(())
        }
        fn set_regular_points(&mut self, _actor: &OwnedActor, _points: f64) -> Result<(), qa_content::q1::Q1Error> {
            Ok(())
        }
        fn bind_damage_adjustment(
            &mut self,
            _actor: &OwnedActor,
            _adjust: qa_content::q1::foundation::host::Q1DamageAdjustHook,
        ) {
        }
        fn apply(
            &mut self,
            _request: &qa_content::q1::foundation::gameplay::DamageRequest,
        ) -> qa_content::q1::foundation::gameplay::DamageOutcome {
            panic!("combat apply unsupported in arsenal tests")
        }
    }

    use qa_content::q1::foundation::host::{
        Q1CheckBottomHook, Q1CheckClientHook, Q1ClassnameHook, Q1Contents, Q1ContentsHook, Q1PowerupHook,
        Q1PusherStatus, Q1PusherStep, Q1RandomHook, Q1TraceHook,
    };
    use qa_content::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1Trace, Q1TraceRequest};

    fn test_host() -> qa_content::q1::foundation::host::Q1FoundationHost {
        qa_content::q1::foundation::host::Q1FoundationHost {
            actors: Box::new(MockActors::new()),
            bodies: Box::new(MockBodies),
            callbacks: Box::new(MockCallbacks),
            combat: Box::new(MockCombat { states: HashMap::new() }),
            inventory: Box::new(MockInventory::new()),
            original_pickups: None,
            punch_angles: None,
            weapon_behavior: None,
            register_entity: None,
            source_damage_modifier: None,
            source_damage_powerup_owner: None,
            random: Box::new(|| 0.5) as Q1RandomHook,
            trace: Box::new(|request: &Q1TraceRequest| Q1Trace {
                fraction: 1.0,
                end: request.end,
                normal: qa_core::math::vec3(0.0, 0.0, 1.0),
                actor: None,
                start_solid: false,
                all_solid: false,
                sky: false,
                in_open: true,
                in_water: false,
            }) as Q1TraceHook,
            contents: Box::new(|_| Q1Contents::Empty) as Q1ContentsHook,
            walk_move: Box::new(|_: &OwnedActor, _: f64, _: f64| true),
            change_yaw: Box::new(|_: &OwnedActor| {}),
            move_to_goal: Box::new(
                |_: &OwnedActor, _: &ActorId, _: f64, _: Option<qa_content::q1::foundation::host::Q1GoalMode>| {},
            ),
            check_bottom: Box::new(|_: &ActorId| true) as Q1CheckBottomHook,
            schedule_think: Box::new(|_: &OwnedActor, _: f64| {}),
            cancel_think: Box::new(|_: &OwnedActor| {}),
            emit: Box::new(|_: qa_content::q1::foundation::types::Q1Event| {}),
            transition: Box::new(|_: qa_content::q1::foundation::gameplay::TransitionIntent| {}),
            players: Box::new(Vec::new),
            check_client: Box::new(|_: &OwnedActor| None) as Q1CheckClientHook,
            classname: Box::new(|_: &ActorId| "player".to_string()) as Q1ClassnameHook,
            powerup: Box::new(|_: &OwnedActor, _: qa_content::q1::foundation::types::Q1Powerup, _: f64| {})
                as Q1PowerupHook,
            step_pusher: Box::new(|actor, _| Q1PusherStep {
                actor: actor.clone(),
                status: Q1PusherStatus::Blocked,
                moved: Vec::new(),
            }),
            weapon_impact: None,
            weapon_volume: None,
            monster_target: None,
            set_gravity: None,
            control_player: None,
            source_target: None,
            powerup_expires: None,
            source_damage_multiplier: None,
        }
    }

    fn test_options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: Some(ProviderId::new("sim", "test")),
            precache_program: None,
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("sim", "test"),
            combat_provider: ProviderId::new("sim", "test"),
            movement_provider: ProviderId::new("sim", "test"),
            inventory_provider: ProviderId::new("sim", "test"),
            gravity: 800.0,
            max_clients: None,
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    fn setup() -> (Rc<RefCell<Q1EntityServices>>, OwnedActor) {
        let game = Q1EntityServices::new(test_host(), test_options()).unwrap();
        let game = Rc::new(RefCell::new(game));
        let actor = game
            .borrow_mut()
            .host
            .actors
            .allocate_at_source(&ProviderId::new("sim", "test"), 1, "player")
            .unwrap();
        (game, actor)
    }

    fn arsenal(game: Rc<RefCell<Q1EntityServices>>) -> Q1SelectedArsenal {
        Q1SelectedArsenal::new(Q1SelectedArsenalOptions {
            game,
            native_player: None,
            replaced_items: Vec::new(),
            fired: None,
            impulse: None,
            prepare_pickup: None,
            observe: Box::new(|_| Q1Observation {
                view_angles: qa_core::math::vec3(0.0, 0.0, 0.0),
                water_level: 0,
            }),
        })
    }

    #[test]
    fn admits_reads_and_selects() {
        let (game, actor) = setup();
        let mut arsenal = arsenal(game);
        assert!(!arsenal.has(actor.id()));
        let admitted = arsenal.admit(actor.clone(), 100.0, false);
        assert!(arsenal.has(actor.id()));
        assert_eq!(admitted.provider, ProviderId::new("sim", "test"));
        assert!(arsenal.select(actor.id(), &"q1:weapon/axe".to_string()));
        assert!(!arsenal.select(actor.id(), &"q1:weapon/shotgun".to_string()));
        assert!(!arsenal.select(actor.id(), &"q1:weapon/nope".to_string()));
        assert_eq!(arsenal.pending_weapon(actor.id()), None);
        let ui = arsenal.ui(
            actor.id(),
            &ProviderReference {
                provider: ProviderId::new("sim", "test"),
                content: qa_content::contract::ContentId("q1:base:test:1".to_string()),
            },
        );
        assert!(!ui.items.is_empty());
        assert!(arsenal.view(actor.id()).is_some());
        assert_eq!(arsenal.family(), ArsenalFamily::Q1);
        assert!(!arsenal.catalog().is_empty());
    }

    #[test]
    fn travel_roundtrips() {
        let (game, actor) = setup();
        let mut arsenal = arsenal(game.clone());
        arsenal.admit(actor.clone(), 100.0, false);
        let travel = arsenal.capture_travel(actor.id()).unwrap();
        assert!(travel.extensions.is_empty());
        assert!(!travel.inventory.is_empty());
        let before = arsenal.read(actor.id());
        arsenal.remove(actor.id());
        let restored = arsenal.admit_travel(actor.clone(), 100.0, &travel).unwrap();
        assert_eq!(restored.active_weapon, before.active_weapon);
        assert_eq!(restored.ammo.len(), before.ammo.len());
        let foreign = Q1SelectedArsenalTravel {
            weapon: travel.weapon,
            inventory: vec![InventoryEntry {
                item: "q2:weapon_blaster".to_string(),
                count: 1.0,
                capacity: 1.0,
                count_policy: None,
            }],
            extensions: Vec::new(),
        };
        arsenal.remove(actor.id());
        assert_eq!(
            arsenal.admit_travel(actor.clone(), 100.0, &foreign).unwrap_err(),
            Q1ArsenalError::ForeignTravelWeapon
        );
    }

    #[test]
    fn impulses_steps_and_handoffs() {
        let (game, actor) = setup();
        let mut arsenal = arsenal(game);
        arsenal.admit(actor.clone(), 100.0, false);
        assert!(!arsenal.impulse(actor.id(), 0).unwrap());
        assert!(arsenal.impulse(actor.id(), 2).unwrap());
        let input = WeaponStepInput {
            actor: actor.clone(),
            command: UserCommand::Q1Quakeworld(qa_world::movement::types::QwUserCommand {
                milliseconds: 50,
                angles: qa_core::math::vec3(0.0, 0.0, 0.0),
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0,
                impulse: 0,
            }),
            frame: qa_core::time::FrameContext {
                frame: 1,
                phase: qa_core::time::FramePhase::ClientCommand,
                time: SourceTime::Milliseconds(50),
                elapsed: SourceTime::Milliseconds(50),
            },
            arsenal: arsenal.read(actor.id()),
            animation: ActorAnimationState {
                provider: ProviderId::new("sim", "test"),
                state: qa_world::movement::types::AnimationState::Q1 {
                    frame: 0,
                    next_frame_seconds: 0.0,
                },
            },
            environment: qa_world::movement::types::MovementEnvironment::default(),
            gauntlet_hit: false,
        };
        let stepped = arsenal.step(&input, None);
        assert!(matches!(stepped.arsenal.state, WeaponState::Q1 { .. }));
        arsenal.frame(0.05).unwrap();
        let mut handoff = arsenal.handoff(actor.id());
        assert!(!handoff.is_deferred());
        assert!(handoff.accepts(&"q1:weapon/axe".to_string()));
        let mut request = handoff.restore_request(3, None);
        assert_eq!(request.status(), RequestStatus::Refused);
    }
}
