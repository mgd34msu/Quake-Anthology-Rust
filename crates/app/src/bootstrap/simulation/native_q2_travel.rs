//! Native Quake II level-travel payloads.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/native-q2-travel.ts`.
//!
//! The donor's unexported `NativeQ2TravelClients` base is flattened into each
//! travel struct: embedding it would name a donor-private interface in public
//! API, while flattening keeps the exact donor field layout per edition. The
//! donor `edition` literals are carried by the [`NativeQ2Travel`] enum.

use std::collections::HashMap;

use qa_content::contract::ItemId;
use qa_content::q2::foundation::host::Q2Edition;
use qa_core::identity::ClientId;

use super::dropped_pickups::DroppedPickupLevels;
use super::native_q2_rerelease_save::Q2RereleaseVisitedLevel;
use super::types::{HandGrenadeTravel, SelectedArsenalTravel};
use super::weapon_slot::WeaponReference;
use crate::persistence::q2::classic_guest::{ClassicOriginalSaveFiles, Q2ClassicVisitedLevel};

/// Mirror of `ClassicGuestWorld` from donor
/// `src/app/bootstrap/simulation/classic-guest-world.ts` (canonical home:
/// `crate::bootstrap::simulation::classic_guest_world`); unify post-merge.
///
/// Travel only carries the world handle across the source retirement
/// boundary; the guest partition owns every method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ClassicGuestWorld;

impl ClassicGuestWorld {
    /// Donor `edition` discriminant.
    #[must_use]
    pub fn edition(&self) -> Q2Edition {
        Q2Edition::Classic
    }
}

/// Mirror of `RereleaseGuestWorld` from donor
/// `src/app/bootstrap/simulation/rerelease-guest-world.ts` (canonical home:
/// `crate::bootstrap::simulation::rerelease_guest_world`); unify post-merge.
///
/// Travel only carries the world handle across the source retirement
/// boundary; the guest partition owns every method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RereleaseGuestWorld;

impl RereleaseGuestWorld {
    /// Donor `edition` discriminant.
    #[must_use]
    pub fn edition(&self) -> Q2Edition {
        Q2Edition::Rerelease
    }
}

/// Traveling native client phase, mirroring donor `"connected" | "active"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeQ2TravelClientPhase {
    /// Connected, not yet begun.
    Connected,
    /// Active in the game.
    Active,
}

impl NativeQ2TravelClientPhase {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            NativeQ2TravelClientPhase::Connected => "connected",
            NativeQ2TravelClientPhase::Active => "active",
        }
    }
}

/// One traveling native client, mirroring the donor `clients` entries.
///
/// Donor `nativeInventorySelection?: ItemId | null` collapses absent and null
/// into [`None`], matching the hub convention.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeQ2TravelClient {
    /// Traveling client.
    pub client: ClientId,
    /// Client phase.
    pub phase: NativeQ2TravelClientPhase,
    /// Carried hand-grenade state.
    pub hand_grenades: Option<HandGrenadeTravel>,
    /// Carried weapon slot.
    pub weapon_slot: Option<WeaponReference>,
    /// Carried native inventory selection.
    pub native_inventory_selection: Option<ItemId>,
    /// Carried selected arsenal.
    pub selected_arsenal: Option<SelectedArsenalTravel>,
}

/// Classic native travel, mirroring donor `ClassicNativeQ2Travel`.
///
/// The save-file overlay owns chaotic guest state, so this struct is neither
/// [`Clone`] nor [`PartialEq`]; the manual [`std::fmt::Debug`] rendering
/// reports every field except the unsized overlay.
pub struct ClassicNativeQ2Travel {
    /// Retained dropped-pickup cargo by level.
    pub dropped_pickups: Option<DroppedPickupLevels>,
    /// Traveling clients.
    pub clients: Vec<NativeQ2TravelClient>,
    /// Destination spawn point.
    pub spawn_point: String,
    /// Retired classic world, transferred after the destination commits.
    pub world: ClassicGuestWorld,
    /// Original save-file overlay.
    pub files: ClassicOriginalSaveFiles,
    /// Visited hub levels by map path.
    pub visited: HashMap<String, Q2ClassicVisitedLevel>,
}

impl std::fmt::Debug for ClassicNativeQ2Travel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClassicNativeQ2Travel")
            .field("dropped_pickups", &self.dropped_pickups)
            .field("clients", &self.clients)
            .field("spawn_point", &self.spawn_point)
            .field("world", &self.world)
            .field("files", &format_args!("ClassicOriginalSaveFiles(..)"))
            .field("visited", &self.visited)
            .finish()
    }
}

impl ClassicNativeQ2Travel {
    /// Donor `edition` discriminant.
    #[must_use]
    pub fn edition(&self) -> Q2Edition {
        self.world.edition()
    }
}

/// Rerelease native travel, mirroring donor `RereleaseNativeQ2Travel`.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseNativeQ2Travel {
    /// Retained dropped-pickup cargo by level.
    pub dropped_pickups: Option<DroppedPickupLevels>,
    /// Traveling clients.
    pub clients: Vec<NativeQ2TravelClient>,
    /// Destination spawn point.
    pub spawn_point: String,
    /// Retired rerelease world, transferred after the destination commits.
    pub world: RereleaseGuestWorld,
    /// Visited hub levels by map path.
    pub visited: HashMap<String, Q2RereleaseVisitedLevel>,
}

impl RereleaseNativeQ2Travel {
    /// Donor `edition` discriminant.
    #[must_use]
    pub fn edition(&self) -> Q2Edition {
        self.world.edition()
    }
}

/// Native Quake II travel by edition, mirroring donor `NativeQ2Travel`.
#[derive(Debug)]
pub enum NativeQ2Travel {
    /// Classic travel.
    Classic(ClassicNativeQ2Travel),
    /// Rerelease travel.
    Rerelease(RereleaseNativeQ2Travel),
}

impl NativeQ2Travel {
    /// Donor `edition` discriminant.
    #[must_use]
    pub fn edition(&self) -> Q2Edition {
        match self {
            NativeQ2Travel::Classic(travel) => travel.edition(),
            NativeQ2Travel::Rerelease(travel) => travel.edition(),
        }
    }

    /// Traveling clients.
    #[must_use]
    pub fn clients(&self) -> &[NativeQ2TravelClient] {
        match self {
            NativeQ2Travel::Classic(travel) => &travel.clients,
            NativeQ2Travel::Rerelease(travel) => &travel.clients,
        }
    }

    /// Destination spawn point.
    #[must_use]
    pub fn spawn_point(&self) -> &str {
        match self {
            NativeQ2Travel::Classic(travel) => &travel.spawn_point,
            NativeQ2Travel::Rerelease(travel) => &travel.spawn_point,
        }
    }
}

#[cfg(test)]
mod tests {
    use qa_content::contract::{InventoryCountPolicy, InventoryEntry};
    use qa_content::q2::equipment::hand_grenades::{HandGrenadeEquipmentState, HandGrenadeLoadout};
    use qa_content::q2::foundation::weapons::hand_action::HandAction;
    use qa_core::identity::{IdentityOwner, ProviderId};

    use super::*;
    use crate::bootstrap::simulation::native_q2_rerelease_save::{Q2RereleaseLevelState, RereleaseSourceSave};
    use crate::persistence::q2::classic_guest::Q2ClassicLevelState;

    fn client() -> (IdentityOwner, ClientId) {
        let owner = IdentityOwner::create("travel-test").unwrap();
        let client = owner.client(0, 1);
        (owner, client)
    }

    fn grenades() -> HandGrenadeTravel {
        HandGrenadeTravel {
            state: HandGrenadeEquipmentState {
                config: HandGrenadeLoadout {
                    enabled: true,
                    initial_ammo: 1.0,
                    capacity: 5.0,
                    infinite_ammo: false,
                },
                action: HandAction::Idle,
            },
            ammo: InventoryEntry {
                item: "q2:ammo/grenades".to_string(),
                count: 3.0,
                capacity: 5.0,
                count_policy: Some(InventoryCountPolicy::Stack),
            },
            seconds: 12.5,
        }
    }

    fn travel_client(client: ClientId) -> NativeQ2TravelClient {
        NativeQ2TravelClient {
            client,
            phase: NativeQ2TravelClientPhase::Active,
            hand_grenades: Some(grenades()),
            weapon_slot: Some(WeaponReference {
                provider: ProviderId::new("q2", "game"),
                item: "q2:weapon/blaster".to_string(),
            }),
            native_inventory_selection: Some("q2:weapon/shotgun".to_string()),
            selected_arsenal: Some(SelectedArsenalTravel::Q2 {
                weapon: Some("q2:weapon/blaster".to_string()),
                inventory: Vec::new(),
            }),
        }
    }

    #[test]
    fn classic_travel_carries_world_files_and_visits() {
        let (_owner, client) = client();
        let travel = ClassicNativeQ2Travel {
            dropped_pickups: None,
            clients: vec![travel_client(client.clone())],
            spawn_point: "start".to_string(),
            world: ClassicGuestWorld,
            files: ClassicOriginalSaveFiles::new(None),
            visited: HashMap::from([(
                "maps/base1.bsp".to_string(),
                Q2ClassicVisitedLevel {
                    state: Q2ClassicLevelState {
                        configstrings: vec![(0, "level".to_string())],
                        portals: vec![(1, true)],
                    },
                    map: "maps/base1.bsp".to_string(),
                    level: vec![1, 2, 3],
                },
            )]),
        };
        assert_eq!(travel.edition(), Q2Edition::Classic);
        assert_eq!(travel.clients.len(), 1);
        assert_eq!(travel.clients[0].client, client);
        assert_eq!(travel.clients[0].phase, NativeQ2TravelClientPhase::Active);
        assert_eq!(travel.clients[0].phase.as_str(), "active");
        assert_eq!(travel.visited["maps/base1.bsp"].level, vec![1, 2, 3]);
        let rendered = format!("{travel:?}");
        assert!(rendered.contains("start"), "{rendered}");
        assert!(rendered.contains("ClassicOriginalSaveFiles"), "{rendered}");
    }

    #[test]
    fn rerelease_travel_keeps_client_options() {
        let (_owner, client) = client();
        let travel = RereleaseNativeQ2Travel {
            dropped_pickups: Some(HashMap::new()),
            clients: vec![NativeQ2TravelClient {
                phase: NativeQ2TravelClientPhase::Connected,
                hand_grenades: None,
                weapon_slot: None,
                native_inventory_selection: None,
                selected_arsenal: None,
                client: client.clone(),
            }],
            spawn_point: String::new(),
            world: RereleaseGuestWorld,
            visited: HashMap::from([(
                "maps/base1.bsp".to_string(),
                Q2RereleaseVisitedLevel {
                    state: Q2RereleaseLevelState::default(),
                    map: "maps/base1.bsp".to_string(),
                    level: RereleaseSourceSave {
                        native: vec![9],
                        deferred_damage: Vec::new(),
                        projections: Vec::new(),
                    },
                },
            )]),
        };
        assert_eq!(travel.edition(), Q2Edition::Rerelease);
        assert_eq!(travel.clients[0].phase, NativeQ2TravelClientPhase::Connected);
        assert_eq!(travel.clients[0].phase.as_str(), "connected");
        assert!(travel.clients[0].hand_grenades.is_none());
        assert!(travel.dropped_pickups.is_some());
        assert_eq!(travel.visited["maps/base1.bsp"].level.native, vec![9]);
        let round_trip = travel.clone();
        assert_eq!(round_trip, travel);
    }

    #[test]
    fn travel_enum_dispatches_by_edition() {
        let (_owner, client) = client();
        let classic = NativeQ2Travel::Classic(ClassicNativeQ2Travel {
            dropped_pickups: None,
            clients: vec![travel_client(client.clone())],
            spawn_point: "a".to_string(),
            world: ClassicGuestWorld,
            files: ClassicOriginalSaveFiles::new(None),
            visited: HashMap::new(),
        });
        let rerelease = NativeQ2Travel::Rerelease(RereleaseNativeQ2Travel {
            dropped_pickups: None,
            clients: Vec::new(),
            spawn_point: "b".to_string(),
            world: RereleaseGuestWorld,
            visited: HashMap::new(),
        });
        assert_eq!(classic.edition(), Q2Edition::Classic);
        assert_eq!(rerelease.edition(), Q2Edition::Rerelease);
        assert_eq!(classic.clients().len(), 1);
        assert!(rerelease.clients().is_empty());
        assert_eq!(classic.spawn_point(), "a");
        assert_eq!(rerelease.spawn_point(), "b");
        assert_eq!(ClassicGuestWorld.edition(), Q2Edition::Classic);
        assert_eq!(RereleaseGuestWorld.edition(), Q2Edition::Rerelease);
    }
}
