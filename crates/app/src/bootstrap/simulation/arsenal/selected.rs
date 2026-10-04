//! Selected-arsenal boundary: one family implementation per provider.
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/arsenal/selected.ts`.

use qa_content::contract::{ItemId, PickupSelection, ProviderReference};
use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::time::FrameContext;
use qa_net::common::commands::ArsenalIntent;
use qa_world::movement::q1::types::{Q1MovementState, QwMovementState};
use qa_world::movement::q2::types::{Q2MovementState, Q2RereleaseMovementState};
use qa_world::movement::q3::types::Q3MovementState;
use qa_world::movement::types::{
    ActorAnimationState, ArsenalState, MovementContinuation, MovementEffect, MovementEnvironment, UserCommand,
};
use qa_world::pickups::PickupAmmoReceipt;

pub use super::super::weapon_slot::{PrimaryWeaponHandoff, WeaponReference};

/// Selected arsenal family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArsenalFamily {
    /// Quake 1.
    Q1,
    /// Quake 2.
    Q2,
    /// Quake 3.
    Q3,
}

/// Pickup drop behavior for a selected weapon. Absorbed; see absorbed-contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplyDrop {
    /// Drops supply.
    Supply,
    /// Drops nothing.
    None,
}

/// Catalog weapon. Absorbed; see absorbed-contracts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedPickupWeapon {
    /// Weapon item.
    pub item: ItemId,
    /// Drop behavior.
    pub drop: SupplyDrop,
}

/// Arsenal ammo warning. Absorbed from donor ui contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArsenalAmmoWarning {
    /// No warning.
    None,
    /// Low ammo.
    Low,
    /// Empty.
    Empty,
}

/// Weapon HUD ammo readout. Absorbed from donor ui contract.
#[derive(Debug, Clone, PartialEq)]
pub enum WeaponAmmoStatus {
    /// Unmetered.
    Unmetered,
    /// Finite pool.
    Finite {
        /// Ammo item.
        item: ItemId,
        /// Count.
        count: f64,
        /// Whether an attack can start.
        has_ammo_to_start: bool,
        /// Whether the pool is at or below its warning threshold.
        low: bool,
    },
}

/// Weapon HUD status. Absorbed from donor ui contract.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponHudStatus {
    /// Owning source.
    pub source: ProviderReference,
    /// Weapon item.
    pub item: ItemId,
    /// Display label.
    pub label: String,
    /// Ammo readout.
    pub ammo: WeaponAmmoStatus,
}

/// HUD item types, canonicalized in the simulation types hub.
pub use super::super::types::{PlayerUiItem, PlayerUiItemKind, UiAmmo};

/// Arsenal-owned slice of the player HUD. Absorbed from donor simulation types.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedArsenalUi {
    /// Active weapon.
    pub active_weapon: Option<ItemId>,
    /// Active ammo.
    pub ammo: Option<UiAmmo>,
    /// Item rows.
    pub items: Vec<PlayerUiItem>,
    /// Weapon status.
    pub weapon_status: Option<WeaponHudStatus>,
    /// Ammo warning.
    pub arsenal_warning: ArsenalAmmoWarning,
}

/// Held-weapon view model. Absorbed from donor selected-arsenal shape.
#[derive(Debug, Clone, PartialEq)]
pub struct ArsenalView {
    /// Model path.
    pub path: String,
    /// Frame.
    pub frame: f64,
}

/// Movement command, mirroring donor `MovementCommand` (`UserCommand`).
pub type MovementCommand = UserCommand;

/// Render a provider as `namespace:name` (shared arsenal helper).
pub(crate) fn provider_name(provider: &ProviderId) -> String {
    format!("{}:{}", provider.namespace, provider.name)
}

/// Unwrap an arsenal result, panicking with the donor `contract` message (shared arsenal helper).
pub(crate) fn contract<T, E: std::fmt::Display>(result: Result<T, E>) -> T {
    result.unwrap_or_else(|error| panic!("{error}"))
}

/// Any-family movement state, mirroring donor `MovementState`.
#[derive(Debug, Clone, PartialEq)]
pub enum MovementState {
    /// NetQuake state.
    Q1Netquake(Q1MovementState),
    /// QuakeWorld state.
    Q1Quakeworld(QwMovementState),
    /// Quake II classic state.
    Q2Classic(Q2MovementState),
    /// Quake II rerelease state.
    Q2Rerelease(Q2RereleaseMovementState),
    /// Quake III state.
    Q3(Q3MovementState),
}

/// Weapon step input, mirroring donor `WeaponStepInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponStepInput {
    /// Acting actor.
    pub actor: OwnedActor,
    /// Movement command.
    pub command: MovementCommand,
    /// Frame context.
    pub frame: FrameContext,
    /// Current arsenal.
    pub arsenal: ArsenalState,
    /// Current animation.
    pub animation: ActorAnimationState,
    /// Environment.
    pub environment: MovementEnvironment,
    /// Gauntlet hit flag.
    pub gauntlet_hit: bool,
}

/// Weapon step result, mirroring donor `WeaponStepResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponStepResult {
    /// Continuation, when stepping continues.
    pub continuation: Option<MovementContinuation<MovementState>>,
    /// Next arsenal.
    pub arsenal: ArsenalState,
    /// Next animation.
    pub animation: ActorAnimationState,
    /// Ordered effects.
    pub effects: Vec<MovementEffect>,
}

/// One family's selected arsenal.
pub trait SelectedArsenal {
    /// Family tag.
    fn family(&self) -> ArsenalFamily;
    /// Owning provider.
    fn provider(&self) -> ProviderId;
    /// Pickup catalog.
    fn catalog(&self) -> Vec<SelectedPickupWeapon>;
    /// Whether the actor is admitted.
    fn has(&self, actor: &ActorId) -> bool;
    /// Admit an actor.
    fn admit(&mut self, actor: OwnedActor, max_health: f64, team_deathmatch: bool) -> ArsenalState;
    /// Read arsenal state.
    fn read(&self, actor: &ActorId) -> ArsenalState;
    /// Select a weapon.
    fn select(&mut self, actor: &ActorId, item: &ItemId) -> bool;
    /// Apply ammo receipts.
    fn pickup_ammo(&mut self, actor: &OwnedActor, grants: &[PickupAmmoReceipt], auto_switch: bool);
    /// Apply weapon pickups.
    fn pickup_weapons(&mut self, actor: &OwnedActor, weapons: &[ItemId], selection: PickupSelection);
    /// Pending weapon, if any.
    fn pending_weapon(&self, actor: &ActorId) -> Option<ItemId>;
    /// Primary handoff for the weapon slot.
    fn handoff(&mut self, actor: &ActorId) -> Box<dyn PrimaryWeaponHandoff>;
    /// Step weapons for one frame.
    fn step(&mut self, input: &WeaponStepInput, intent: Option<&ArsenalIntent>) -> WeaponStepResult;
    /// Remove an actor.
    fn remove(&mut self, actor: &ActorId);
    /// Arsenal HUD slice.
    fn ui(&self, actor: &ActorId, source: &ProviderReference) -> SelectedArsenalUi;
    /// Held-weapon view model, if any.
    fn view(&self, actor: &ActorId) -> Option<ArsenalView>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pickup_weapon_roundtrips() {
        let weapon = SelectedPickupWeapon {
            item: "q1:shotgun".to_string(),
            drop: SupplyDrop::Supply,
        };
        assert_eq!(weapon.clone().drop, SupplyDrop::Supply);
        assert_ne!(SupplyDrop::Supply, SupplyDrop::None);
    }

    #[test]
    fn warning_variants_are_distinct() {
        assert_ne!(ArsenalAmmoWarning::None, ArsenalAmmoWarning::Low);
        assert_ne!(ArsenalAmmoWarning::Low, ArsenalAmmoWarning::Empty);
    }

    #[test]
    fn families_are_copy() {
        let family = ArsenalFamily::Q2;
        assert_eq!(family, ArsenalFamily::Q2);
        assert_ne!(family, ArsenalFamily::Q3);
    }

    #[test]
    fn shared_provider_name_renders_pair() {
        assert_eq!(provider_name(&ProviderId::new("q1", "base")), "q1:base");
    }

    #[test]
    fn shared_contract_passes_ok_through() {
        let value: Result<i32, &str> = Ok(7);
        assert_eq!(contract(value), 7);
    }

    #[test]
    #[should_panic(expected = "boom")]
    fn shared_contract_panics_with_display() {
        let value: Result<i32, &str> = Err("boom");
        contract(value);
    }

    #[test]
    fn ui_holds_item_rows() {
        let ui = SelectedArsenalUi {
            active_weapon: Some("q1:axe".to_string()),
            ammo: Some(UiAmmo {
                item: "q1:shells".to_string(),
                count: 12.0,
            }),
            items: vec![PlayerUiItem {
                id: "q1:axe".to_string(),
                label: "Axe".to_string(),
                kind: PlayerUiItemKind::Weapon,
                source_ordinal: 0.0,
                owned: true,
                has_ammo: true,
                count: None,
                warning_count: 0.0,
            }],
            weapon_status: None,
            arsenal_warning: ArsenalAmmoWarning::None,
        };
        assert_eq!(ui.items.len(), 1);
        assert_eq!(ui.items[0].kind, PlayerUiItemKind::Weapon);
    }
}
