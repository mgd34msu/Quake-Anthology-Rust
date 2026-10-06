//! Native Quake I level travel: spawn-parm capture and restore.
//!
//! Stock `SetChangeParms`/`DecodeLevelParms` (`client.qc:32-96`) over the
//! shared [`Q1SpawnParms`]: the exit captures the leaver's inventory,
//! the app transition carries it with `serverflags`
//! (`SV_SaveSpawnparms`, `sv_main.c:1015`), and the arrival restores it
//! onto the fresh player (`PutClientInServer`, `client.qc:479`).
//! `SetNewParms` is [`Q1SpawnParms::default`], owned by the weapons lane
//! with the item-bit table.
//!
//! [`Q1SpawnParms`]: super::native_q1_weapons::Q1SpawnParms

use qa_core::identity::ActorId;
use qa_world::combat::{CombatState, RegularArmor};
use qa_world::session::Simulation;

use super::native_q1_spawns::{q1_health_of, Q1NativeBehaviors};
use super::native_q1_weapons::{
    q1_w_set_current_ammo, Q1SpawnParms, Q1_IT_INVISIBILITY, Q1_IT_INVULNERABILITY, Q1_IT_KEY1, Q1_IT_KEY2, Q1_IT_QUAD,
    Q1_IT_SUIT,
};

/// App-transition payload (`SV_SaveSpawnparms`, `sv_main.c:1015`): the
/// captured inventory plus the episode flags, carried from the leaver
/// to the arrival.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1TravelCarry {
    /// Captured spawn parms (`SetChangeParms`, or fresh `SetNewParms`
    /// for a flagged return to `start`).
    pub parms: Q1SpawnParms,
    /// Episode-completion bits, preserved across the transition.
    pub serverflags: i32,
}

/// Bits `SetChangeParms` strips: keys and temporary powerups never
/// cross a level transition (`client.qc:38-40`).
const PARM_STRIP_BITS: u32 =
    Q1_IT_KEY1 | Q1_IT_KEY2 | Q1_IT_INVISIBILITY | Q1_IT_INVULNERABILITY | Q1_IT_SUIT | Q1_IT_QUAD;

/// Stock armor-absorption fractions (`items.qc:238-285`), mapping a
/// restored `armortype` back to its source item id (0.3 green, 0.6
/// yellow, 0.8 red).
const ARMOR_YELLOW_ABSORPTION: f64 = 0.6;
/// Stock armor-absorption fractions (`items.qc:238-285`).
const ARMOR_RED_ABSORPTION: f64 = 0.8;

/// Run `SetChangeParms` (`client.qc:32-62`): dead players reset to
/// `SetNewParms`; the living strip keys and powerups, clamp health to
/// 50-100, and capture items, health, armor, ammo (shells floored at
/// 25), weapon, and armor type. Stock mutates the leaver in place, so
/// the capture does too (the old world is discarded at travel).
pub fn q1_set_change_parms(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    player: &ActorId,
) -> Q1SpawnParms {
    if q1_health_of(simulation, player) <= 0.0 {
        return Q1SpawnParms::default();
    }
    behaviors.player_items &= !PARM_STRIP_BITS;
    let health = q1_health_of(simulation, player).clamp(50.0, 100.0);
    if let Some(combat) = simulation.combat_state(player).cloned() {
        let _ignored = simulation.set_combat(player, CombatState { health, ..combat });
    }
    let (armorvalue, armortype) = match simulation.combat_state(player).map(|combat| &combat.armor.regular) {
        Some(RegularArmor::Q1 { points, absorption, .. }) => (*points, *absorption),
        _ => (0.0, 0.0),
    };
    Q1SpawnParms {
        items: behaviors.player_items,
        health,
        armorvalue,
        shells: behaviors.player_ammo.shells.max(25.0),
        nails: behaviors.player_ammo.nails,
        rockets: behaviors.player_ammo.rockets,
        cells: behaviors.player_ammo.cells,
        weapon: behaviors.player_state.weapon,
        armortype,
    }
}

/// Run `DecodeLevelParms` (`client.qc:77-96`): assign the carried
/// inventory onto the fresh player, snapshot it as the level-entry
/// parms (the coop `setspawnparms` source, `pr_cmds.c:1615`), and
/// refresh the current ammo (`W_SetCurrentAmmo`, like `PutClientInServer`).
/// The caller picks fresh `SetNewParms` for a flagged return to `start`
/// (`client.qc:79-83`); the decode itself always assigns.
pub fn q1_decode_level_parms(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    player: &ActorId,
    parms: &Q1SpawnParms,
) {
    behaviors.player_items = parms.items;
    behaviors.player_ammo.shells = parms.shells;
    behaviors.player_ammo.nails = parms.nails;
    behaviors.player_ammo.rockets = parms.rockets;
    behaviors.player_ammo.cells = parms.cells;
    behaviors.player_state.weapon = parms.weapon;
    behaviors.player_state.parms = parms.clone();
    if let Some(combat) = simulation.combat_state(player).cloned() {
        let regular = if parms.armorvalue > 0.0 {
            RegularArmor::Q1 {
                points: parms.armorvalue,
                absorption: parms.armortype,
                item: q1_parm_armor_item(parms.armortype),
            }
        } else {
            RegularArmor::None
        };
        let _ignored = simulation.set_combat(
            player,
            CombatState {
                health: parms.health,
                armor: qa_world::combat::ArmorState {
                    regular,
                    ..combat.armor
                },
                ..combat
            },
        );
    }
    q1_w_set_current_ammo(behaviors);
}

/// Map a restored armor type back to its source item id (stock
/// `armortype` is one of 0.3/0.6/0.8; anything positive reads up to
/// the nearest tier).
fn q1_parm_armor_item(armortype: f64) -> String {
    if armortype >= ARMOR_RED_ABSORPTION {
        "q1:item_armorInv".to_string()
    } else if armortype >= ARMOR_YELLOW_ABSORPTION {
        "q1:item_armor2".to_string()
    } else {
        "q1:item_armor1".to_string()
    }
}

#[cfg(test)]
mod tests {
    use qa_core::math::{vec3, Bounds};
    use qa_world::body::BodyState;
    use qa_world::server::{Server, ServerLogic};

    use super::super::native_q1_weapons::{Q1_IT_AXE, Q1_IT_NAILGUN, Q1_IT_SHELLS, Q1_IT_SHOTGUN};
    use super::*;
    use crate::options::ApplicationOptions;
    use crate::startup::{open_server, StartupConfig};

    fn test_server() -> Server<qa_guest::server::GuestServerLogic> {
        let config = StartupConfig::from_options(&ApplicationOptions::default()).unwrap();
        open_server(&config).unwrap()
    }

    fn spawn_player(server: &mut Server<impl ServerLogic>) -> qa_core::identity::OwnedActor {
        let player = server
            .simulation_mut()
            .spawn(
                qa_core::identity::ProviderId::new("q1", "test"),
                "q1:test_player",
                Some(BodyState {
                    origin: vec3(0.0, 0.0, 0.0),
                    angles: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    bounds: Bounds {
                        min: vec3(-16.0, -16.0, -24.0),
                        max: vec3(16.0, 16.0, 32.0),
                    },
                    ground: None,
                }),
                None,
                Vec::new(),
            )
            .unwrap();
        server
            .simulation_mut()
            .set_combat(player.id(), CombatState::default())
            .unwrap();
        player
    }

    fn loaded_behaviors() -> Q1NativeBehaviors {
        let mut behaviors = Q1NativeBehaviors::new();
        behaviors.player_items = Q1_IT_AXE | Q1_IT_SHOTGUN | Q1_IT_NAILGUN | Q1_IT_KEY1 | Q1_IT_QUAD;
        behaviors.player_ammo.shells = 10.0;
        behaviors.player_ammo.nails = 40.0;
        behaviors.player_state.weapon = Q1_IT_NAILGUN;
        behaviors
    }

    #[test]
    fn set_change_parms_strips_clamps_and_floors() {
        let mut server = test_server();
        let player = spawn_player(&mut server);
        let mut behaviors = loaded_behaviors();
        // Superhealth above the cap, yellow armor worn.
        let worn = server.simulation().combat_state(player.id()).unwrap().armor.clone();
        server
            .simulation_mut()
            .set_combat(
                player.id(),
                CombatState {
                    health: 150.0,
                    armor: qa_world::combat::ArmorState {
                        regular: RegularArmor::Q1 {
                            points: 120.0,
                            absorption: ARMOR_YELLOW_ABSORPTION,
                            item: "q1:item_armor2".to_string(),
                        },
                        ..worn
                    },
                    ..CombatState::default()
                },
            )
            .unwrap();
        let parms = q1_set_change_parms(&mut behaviors, server.simulation_mut(), player.id());
        assert_eq!(parms.items, Q1_IT_AXE | Q1_IT_SHOTGUN | Q1_IT_NAILGUN);
        assert_eq!(behaviors.player_items, parms.items, "leaver strips in place");
        assert_eq!(parms.health, 100.0);
        assert_eq!(parms.armorvalue, 120.0);
        assert_eq!(parms.armortype, ARMOR_YELLOW_ABSORPTION);
        assert_eq!(parms.shells, 25.0, "shells floor at 25");
        assert_eq!(parms.nails, 40.0);
        assert_eq!(parms.weapon, Q1_IT_NAILGUN);
        // Low health rises to the 50 floor.
        server
            .simulation_mut()
            .set_combat(player.id(), CombatState::default())
            .unwrap();
        server.simulation_mut().damage_q1(player.id(), 80.0);
        let parms = q1_set_change_parms(&mut behaviors, server.simulation_mut(), player.id());
        assert_eq!(parms.health, 50.0);
    }

    #[test]
    fn set_change_parms_resets_for_the_dead() {
        let mut server = test_server();
        let player = spawn_player(&mut server);
        let mut behaviors = loaded_behaviors();
        server.simulation_mut().damage_q1(player.id(), 1000.0);
        let parms = q1_set_change_parms(&mut behaviors, server.simulation_mut(), player.id());
        assert_eq!(parms, Q1SpawnParms::default());
    }

    #[test]
    fn decode_level_parms_assigns_snapshots_and_refreshes_ammo() {
        let mut server = test_server();
        let player = spawn_player(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let parms = Q1SpawnParms {
            items: Q1_IT_AXE | Q1_IT_SHOTGUN,
            health: 75.0,
            armorvalue: 100.0,
            shells: 30.0,
            nails: 0.0,
            rockets: 5.0,
            cells: 0.0,
            weapon: Q1_IT_SHOTGUN,
            armortype: 0.3,
        };
        q1_decode_level_parms(&mut behaviors, server.simulation_mut(), player.id(), &parms);
        assert_eq!(
            behaviors.player_items & (Q1_IT_AXE | Q1_IT_SHOTGUN),
            Q1_IT_AXE | Q1_IT_SHOTGUN
        );
        assert_ne!(behaviors.player_items & Q1_IT_SHELLS, 0, "ammo indicator refreshed");
        assert_eq!(behaviors.player_ammo.shells, 30.0);
        assert_eq!(behaviors.player_ammo.rockets, 5.0);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_SHOTGUN);
        assert_eq!(behaviors.player_state.parms, parms, "entry parms snapshotted");
        let combat = server.simulation().combat_state(player.id()).unwrap();
        assert_eq!(combat.health, 75.0);
        match &combat.armor.regular {
            RegularArmor::Q1 {
                points,
                absorption,
                item,
            } => {
                assert_eq!((*points, *absorption), (100.0, 0.3));
                assert_eq!(item, "q1:item_armor1");
            }
            RegularArmor::None => panic!("decode restores worn armor"),
            _ => panic!("decode restores Q1 armor"),
        }
    }

    #[test]
    fn decode_level_parms_clears_missing_armor() {
        let mut server = test_server();
        let player = spawn_player(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let parms = Q1SpawnParms::default();
        q1_decode_level_parms(&mut behaviors, server.simulation_mut(), player.id(), &parms);
        assert!(matches!(
            server.simulation().combat_state(player.id()).unwrap().armor.regular,
            RegularArmor::None
        ));
    }
}
