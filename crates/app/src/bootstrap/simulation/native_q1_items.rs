//! Native Quake I items for the reachable server.
//!
//! Stock spawn/touch/think gamecode for `item_health` (rotten, normal,
//! megahealth), driven from [`Q1NativeBehaviors`] through the native
//! touch hook. Armor, ammo, weapons, keys, sigils, powerups, and
//! backpacks follow in later slices; each stays a generic inert spawn
//! until then.
//!
//! qsrc: `progs106/items.qc` (`SUB_regen` 6, `PlaceItem` 36, `StartItem`
//! 64, `T_Heal` 78, `item_health` 112, `health_touch` 151,
//! `item_megahealth_rot` 206), `progs106/defs.qc:303` (`IT_SUPERHEALTH`).
//!
//! Skeleton scope notes: pickup/respawn/rot sounds have no sim audio
//! path yet (the parsed noise rides the state for the audio slice);
//! pickup prints queue in [`Q1NativeBehaviors::sprints`] for the HUD
//! slice to drain; the `bf` screen flash rides the presentation slice.
//! `PlaceItem` planting is spawn-time only: items size their touch
//! volume at build and skip the `droptofloor` toss (no toss physics
//! yet; mapped origins already sit at floor level, and the volumes
//! overlap the player regardless). The stock 0.2s `StartItem` delay is
//! moot (no touches run mid-spawn), so items mark at build.
//!
//! [`Q1NativeBehaviors`]: super::native_q1_spawns::Q1NativeBehaviors

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};
use qa_world::combat::CombatState;
use qa_world::movers::MoverTable;
use qa_world::server::{Server, ServerLogic};
use qa_world::session::Simulation;
use qa_world::spawn::SpawnFields;
use qa_world::triggers::{TouchContact, TriggerTable};
use qa_world::WorldError;

use super::native_q1_spawns::{q1_health_of, Q1NativeBehaviors, IT_SUPERHEALTH};
use super::native_q1_triggers::{q1_use_targets, Q1ThinkKind, Q1UseSource};

/// `item_health` H_ROTTEN spawnflag (`items.qc:110`): 15-health box.
const HEALTH_ROTTEN: i32 = 1;
/// `item_health` H_MEGA spawnflag (`items.qc:111`): 100-health box plus
/// superhealth rot.
const HEALTH_MEGA: i32 = 2;

/// Rotten-health `healtype` (`items.qc:129`).
const HEAL_ROTTEN: u8 = 0;
/// Normal-health `healtype` (`items.qc:143`).
const HEAL_NORMAL: u8 = 1;
/// Megahealth `healtype` (`items.qc:136`).
const HEAL_MEGA: u8 = 2;

/// Stock health cap (`T_Heal`, `items.qc:103`; `health_touch`,
/// `items.qc:160`): nothing heals past 250.
const HEALTH_ABSOLUTE_MAX: f64 = 250.0;

/// Deathmatch respawn delay in seconds (`health_touch`, `items.qc:191`;
/// `item_megahealth_rot`, `items.qc:230`).
const ITEM_RESPAWN_SECONDS: f64 = 20.0;
/// First megahealth rot delay in seconds (`health_touch`, `items.qc:181`).
const MEGA_ROT_FIRST_SECONDS: f64 = 5.0;
/// Megahealth rot period in seconds (`item_megahealth_rot`,
/// `items.qc:213`).
const MEGA_ROT_SECONDS: f64 = 1.0;

/// Fixed health-box touch volume (`item_health`, `items.qc:145`).
const HEALTH_BOUNDS: Bounds = Bounds {
    min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    max: Vec3 {
        x: 32.0,
        y: 32.0,
        z: 56.0,
    },
};

/// Live Q1 item gamecode state by kind.
#[derive(Debug, Clone)]
pub enum Q1ItemKind {
    /// `item_health`: heals the toucher on pickup.
    Health {
        /// Heal amount (15 rotten, 25 normal, 100 mega).
        healamount: f64,
        /// Heal type (0 rotten, 1 normal, 2 mega).
        healtype: u8,
    },
}

/// Live Q1 item gamecode state.
#[derive(Debug, Clone)]
pub struct Q1Item {
    /// Kind-specific state.
    pub kind: Q1ItemKind,
    /// Firing inputs (pickup runs `SUB_UseTargets` with the toucher).
    pub source: Q1UseSource,
    /// Pickup noise for the audio slice.
    pub noise: &'static str,
    /// Whether the item is taken (hidden until `SUB_regen`).
    pub taken: bool,
}

/// One queued console print (`sprint`) for the HUD slice: stock prints
/// to the toucher immediately, but the walking skeleton has no client
/// path yet, so gamecode queues and the HUD drains.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Sprint {
    /// Print target (always the player picker).
    pub target: ActorId,
    /// Print text.
    pub text: String,
}

/// Register the native Q1 item spawn functions. Items spawn bodies at
/// the map origin for [`build_q1_item`] to size.
pub fn register_q1_item_spawns(registry: &mut qa_world::spawn::SpawnRegistry) {
    use qa_world::spawn::SpawnRequest;
    registry.register(
        "item_health",
        Box::new(|fields| {
            Ok(SpawnRequest {
                definition: "q1:item_health".to_string(),
                origin: Some(fields.origin),
                combat: None,
                grants: Vec::new(),
            })
        }),
    );
}

/// Whether a classname builds native item state.
#[must_use]
pub fn q1_is_item(classname: &str) -> bool {
    matches!(classname, "item_health")
}

/// Finish a spawned item actor: size its touch volume, parse its kind,
/// mark its touch, and record its gamecode state.
pub fn build_q1_item<L: ServerLogic>(
    server: &mut Server<L>,
    behaviors: &mut Q1NativeBehaviors,
    actor: &qa_core::identity::OwnedActor,
    fields: &SpawnFields,
) -> Result<(), WorldError> {
    match fields.classname.as_str() {
        "item_health" => {
            let (healamount, healtype, noise) = if fields.spawnflags & HEALTH_MEGA != 0 {
                (100.0, HEAL_MEGA, "items/r_item2.wav")
            } else if fields.spawnflags & HEALTH_ROTTEN != 0 {
                (15.0, HEAL_ROTTEN, "items/r_item1.wav")
            } else {
                (25.0, HEAL_NORMAL, "items/health1.wav")
            };
            server.simulation_mut().set_body_bounds(actor.id(), HEALTH_BOUNDS)?;
            behaviors.items.insert(
                actor.id(),
                Q1Item {
                    kind: Q1ItemKind::Health { healamount, healtype },
                    source: Q1UseSource::from_fields(fields),
                    noise,
                    taken: false,
                },
            );
            server.mark_trigger(actor.id())?;
        }
        other => {
            return Err(WorldError::BadSpawnFields(format!("not an item: {other}")));
        }
    }
    Ok(())
}

/// Run `T_Heal` (`items.qc:78-95`): heal a living toucher by the
/// ceiled amount, capped at the health cap (skipped when `ignore`) and
/// the absolute 250 cap. Stock reads the cap off the global `other`
/// instead of the healed entity; the touch always heals the toucher,
/// so the two coincide.
fn q1_heal(
    behaviors: &Q1NativeBehaviors,
    simulation: &mut Simulation,
    target: &ActorId,
    amount: f64,
    ignore: bool,
) -> bool {
    let Some(combat) = simulation.combat_state(target).cloned() else {
        return false;
    };
    if combat.health <= 0.0 {
        return false;
    }
    if !ignore && combat.health >= behaviors.player_max_health {
        return false;
    }
    let mut health = combat.health + amount.ceil();
    if !ignore && health >= behaviors.player_max_health {
        health = behaviors.player_max_health;
    }
    if health > HEALTH_ABSOLUTE_MAX {
        health = HEALTH_ABSOLUTE_MAX;
    }
    let _ignored = simulation.set_combat(target, CombatState { health, ..combat });
    true
}

/// Native touch dispatch for Q1 items, chained after the trigger
/// dispatch: `health_touch` (`items.qc:151`) heals players, hides the
/// box, arms respawn or rot, prints the receipt, and fires targets.
pub fn q1_item_touch(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    triggers: &mut TriggerTable,
    contact: &TouchContact,
) {
    let Some(item) = behaviors.items.get(&contact.trigger).cloned() else {
        return;
    };
    if item.taken {
        return;
    }
    let Q1ItemKind::Health { healamount, healtype } = &item.kind;
    if Some(&contact.other) != behaviors.player.as_ref() {
        return;
    }
    if *healtype == HEAL_MEGA {
        if q1_health_of(simulation, &contact.other) >= HEALTH_ABSOLUTE_MAX {
            return;
        }
        if !q1_heal(behaviors, simulation, &contact.other, *healamount, true) {
            return;
        }
    } else if !q1_heal(behaviors, simulation, &contact.other, *healamount, false) {
        return;
    }
    behaviors.sprints.push(Q1Sprint {
        target: contact.other.clone(),
        text: format!("You receive {} health", *healamount as i64),
    });
    // Stock plays the pickup noise and `bf` screen flash here; the
    // audio and presentation slices own them.
    triggers.unmark(&contact.trigger);
    if let Some(item) = behaviors.items.get_mut(&contact.trigger) {
        item.taken = true;
    }
    let now = simulation.frame().time.as_seconds_f64();
    if *healtype == HEAL_MEGA {
        behaviors.player_items |= IT_SUPERHEALTH;
        behaviors.schedule_think(
            &contact.trigger,
            Q1ThinkKind::MegaRot {
                owner: contact.other.clone(),
            },
            now + MEGA_ROT_FIRST_SECONDS,
        );
    } else if behaviors.deathmatch {
        // Deathmatch 2 keeps the silly old rules (no respawn), but
        // `GameMode` never selects it, so deathmatch here is DM1.
        behaviors.schedule_think(&contact.trigger, Q1ThinkKind::Regen, now + ITEM_RESPAWN_SECONDS);
    }
    q1_use_targets(
        behaviors,
        simulation,
        movers,
        triggers,
        &item.source,
        Some(&contact.other),
    );
}

/// Run `SUB_regen` (`items.qc:6-12`): restore a taken item's model and
/// touch. Stock plays `items/itembk2.wav`; the audio slice owns it.
pub fn q1_item_regen(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    triggers: &mut TriggerTable,
    actor: &ActorId,
) {
    if let Some(item) = behaviors.items.get_mut(actor) {
        item.taken = false;
    } else {
        return;
    }
    let _ignored = triggers.mark(simulation.registry(), actor);
}

/// Run `item_megahealth_rot` (`items.qc:206-233`): while the owner sits
/// above the health cap, rot one point per second; otherwise clear the
/// superhealth bit and, in deathmatch, respawn the box. The bit clears
/// only when the owner is still the admitted player (stock notes a
/// player can die and respawn between rots; respawn lands later).
pub fn q1_item_mega_rot(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    actor: &ActorId,
    owner: &ActorId,
) {
    let now = simulation.frame().time.as_seconds_f64();
    if q1_health_of(simulation, owner) > behaviors.player_max_health {
        if let Some(combat) = simulation.combat_state(owner).cloned() {
            let _ignored = simulation.set_combat(
                owner,
                CombatState {
                    health: combat.health - 1.0,
                    ..combat
                },
            );
        }
        behaviors.schedule_think(
            actor,
            Q1ThinkKind::MegaRot { owner: owner.clone() },
            now + MEGA_ROT_SECONDS,
        );
        return;
    }
    if Some(owner) == behaviors.player.as_ref() {
        behaviors.player_items &= !IT_SUPERHEALTH;
    }
    if behaviors.deathmatch {
        behaviors.schedule_think(actor, Q1ThinkKind::Regen, now + ITEM_RESPAWN_SECONDS);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use qa_core::identity::ActorId;
    use qa_core::math::vec3;
    use qa_core::time::SourceTime;
    use qa_world::body::BodyState;
    use qa_world::combat::CombatState;

    use super::*;
    use crate::options::ApplicationOptions;
    use crate::startup::{open_server, StartupConfig};

    fn test_server() -> Server<qa_guest::server::GuestServerLogic> {
        let config = StartupConfig::from_options(&ApplicationOptions::default()).unwrap();
        open_server(&config).unwrap()
    }

    fn register_all(server: &mut Server<qa_guest::server::GuestServerLogic>) {
        super::super::native_q1_spawns::register_q1_spawns(server.spawns_mut());
        super::super::native_q1_triggers::register_q1_trigger_spawns(server.spawns_mut());
        register_q1_item_spawns(server.spawns_mut());
    }

    fn item_fields(classname: &str, pairs: &[(&str, &str)]) -> SpawnFields {
        let mut full = vec![("classname", classname)];
        full.extend_from_slice(pairs);
        SpawnFields::parse(&full).unwrap()
    }

    fn spawn_item(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        fields: &SpawnFields,
    ) -> qa_core::identity::OwnedActor {
        let actor = server.spawn_entity(fields).unwrap();
        build_q1_item(server, behaviors, &actor, fields).unwrap();
        super::super::native_q1_triggers::q1_note_targetname(behaviors, fields, actor.id());
        actor
    }

    fn spawn_player(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        origin: qa_core::math::Vec3,
        health: f64,
    ) -> qa_core::identity::OwnedActor {
        let player = server
            .simulation_mut()
            .spawn(
                qa_core::identity::ProviderId::new("q1", "test"),
                "q1:test_player",
                Some(BodyState {
                    origin,
                    angles: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    bounds: qa_core::math::Bounds {
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
            .set_combat(
                player.id(),
                CombatState {
                    health,
                    ..CombatState::default()
                },
            )
            .unwrap();
        player
    }

    fn admit_player(behaviors: &mut Q1NativeBehaviors, player: &qa_core::identity::OwnedActor) {
        behaviors.set_player(Some(player.id().clone()));
        behaviors.solids.insert(player.id());
    }

    fn touch(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        item: &ActorId,
        other: &ActorId,
    ) {
        let contact = TouchContact {
            trigger: item.clone(),
            other: other.clone(),
        };
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_item_touch(behaviors, simulation, movers, triggers, &contact);
    }

    fn health_of(server: &Server<qa_guest::server::GuestServerLogic>, actor: &ActorId) -> f64 {
        server
            .simulation()
            .combat_state(actor)
            .map(|combat| combat.health)
            .unwrap_or(0.0)
    }

    #[test]
    fn health_spawn_parses_rotten_normal_and_mega() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        for (flags, amount, healtype, noise) in [
            ("1", 15.0, HEAL_ROTTEN, "items/r_item1.wav"),
            ("0", 25.0, HEAL_NORMAL, "items/health1.wav"),
            ("2", 100.0, HEAL_MEGA, "items/r_item2.wav"),
        ] {
            let item = spawn_item(
                &mut server,
                &mut behaviors,
                &item_fields("item_health", &[("origin", "10 20 30"), ("spawnflags", flags)]),
            );
            let record = behaviors.items.get(item.id()).unwrap();
            assert!(matches!(
                &record.kind,
                Q1ItemKind::Health { healamount, healtype: parsed }
                    if *healamount == amount && *parsed == healtype
            ));
            assert_eq!(record.noise, noise);
            assert!(!record.taken);
            assert!(server.triggers_mut().is_trigger(item.id()));
            let body = server.simulation().body_state(item.id()).unwrap();
            assert_eq!(body.origin, vec3(10.0, 20.0, 30.0));
            assert_eq!(body.bounds.min, vec3(0.0, 0.0, 0.0));
            assert_eq!(body.bounds.max, vec3(32.0, 32.0, 56.0));
        }
    }

    #[test]
    fn health_touch_heals_hides_prints_and_fires() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let relay_fields = item_fields("trigger_relay", &[("targetname", "r1"), ("message", "yum")]);
        let relay = server.spawn_entity(&relay_fields).unwrap();
        super::super::native_q1_triggers::q1_note_use_point(&mut behaviors, relay.id(), &relay_fields);
        super::super::native_q1_triggers::q1_note_targetname(&mut behaviors, &relay_fields, relay.id());
        let item = spawn_item(
            &mut server,
            &mut behaviors,
            &item_fields("item_health", &[("origin", "0 0 0"), ("target", "r1")]),
        );
        let player = spawn_player(&mut server, vec3(8.0, 8.0, 8.0), 50.0);
        admit_player(&mut behaviors, &player);
        touch(&mut server, &mut behaviors, item.id(), player.id());
        assert_eq!(health_of(&server, player.id()), 75.0);
        assert_eq!(behaviors.sprints.len(), 1);
        assert_eq!(behaviors.sprints[0].target, *player.id());
        assert_eq!(behaviors.sprints[0].text, "You receive 25 health");
        assert!(!server.triggers_mut().is_trigger(item.id()));
        assert!(behaviors.items.get(item.id()).unwrap().taken);
        assert_eq!(behaviors.centerprints.len(), 1);
        assert_eq!(behaviors.centerprints[0].text, "yum");
        // Single-player health never respawns.
        assert!(behaviors.thinks.is_empty());
    }

    #[test]
    fn heal_caps_and_refusals_follow_stock() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let normal = spawn_item(
            &mut server,
            &mut behaviors,
            &item_fields("item_health", &[("origin", "0 0 0")]),
        );
        let mega = spawn_item(
            &mut server,
            &mut behaviors,
            &item_fields("item_health", &[("origin", "100 0 0"), ("spawnflags", "2")]),
        );
        // Normal caps at max_health.
        let player = spawn_player(&mut server, vec3(8.0, 8.0, 8.0), 90.0);
        admit_player(&mut behaviors, &player);
        touch(&mut server, &mut behaviors, normal.id(), player.id());
        assert_eq!(health_of(&server, player.id()), 100.0);
        // At the cap, normal refuses without hiding or printing.
        let capped = spawn_item(
            &mut server,
            &mut behaviors,
            &item_fields("item_health", &[("origin", "200 0 0")]),
        );
        touch(&mut server, &mut behaviors, capped.id(), player.id());
        assert!(server.triggers_mut().is_trigger(capped.id()));
        assert_eq!(behaviors.sprints.len(), 1);
        // Mega ignores the cap but stops at 250, and refuses at 250.
        server
            .simulation_mut()
            .set_combat(
                player.id(),
                CombatState {
                    health: 240.0,
                    ..CombatState::default()
                },
            )
            .unwrap();
        touch(&mut server, &mut behaviors, mega.id(), player.id());
        assert_eq!(health_of(&server, player.id()), 250.0);
        let mega2 = spawn_item(
            &mut server,
            &mut behaviors,
            &item_fields("item_health", &[("origin", "300 0 0"), ("spawnflags", "2")]),
        );
        touch(&mut server, &mut behaviors, mega2.id(), player.id());
        assert!(server.triggers_mut().is_trigger(mega2.id()));
        // Dead players and non-players refuse.
        server
            .simulation_mut()
            .set_combat(
                player.id(),
                CombatState {
                    health: 0.0,
                    ..CombatState::default()
                },
            )
            .unwrap();
        touch(&mut server, &mut behaviors, capped.id(), player.id());
        assert!(server.triggers_mut().is_trigger(capped.id()));
        let stranger = spawn_player(&mut server, vec3(8.0, 8.0, 8.0), 50.0);
        touch(&mut server, &mut behaviors, capped.id(), stranger.id());
        assert!(server.triggers_mut().is_trigger(capped.id()));
    }

    #[test]
    fn deathmatch_respawns_taken_health_through_live_ticks() {
        let mut server = test_server();
        register_all(&mut server);
        let shared = Rc::new(RefCell::new(Q1NativeBehaviors::new()));
        shared.borrow_mut().deathmatch = true;
        let item = spawn_item(
            &mut server,
            &mut shared.borrow_mut(),
            &item_fields("item_health", &[("origin", "0 0 0")]),
        );
        let player = spawn_player(&mut server, vec3(4000.0, 4000.0, 4000.0), 50.0);
        admit_player(&mut shared.borrow_mut(), &player);
        super::super::native_q1_spawns::install_q1_native(&mut server, Rc::clone(&shared));
        {
            let contact = TouchContact {
                trigger: item.id().clone(),
                other: player.id().clone(),
            };
            let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
            q1_item_touch(&mut shared.borrow_mut(), simulation, movers, triggers, &contact);
        }
        assert_eq!(health_of(&server, player.id()), 75.0);
        assert_eq!(shared.borrow().sprints.len(), 1);
        assert!(shared.borrow().thinks.iter().any(|think| think.actor == *item.id()
            && think.kind == Q1ThinkKind::Regen
            && (think.due_seconds - 20.0).abs() < 1e-9));
        // Prints accumulate until the HUD slice drains them per frame;
        // the regen restores the box.
        for _ in 0..401 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        assert_eq!(shared.borrow().sprints.len(), 1);
        assert!(!shared.borrow().items.get(item.id()).unwrap().taken);
        assert!(server.triggers_mut().is_trigger(item.id()));
        // And the restored box picks up again.
        server
            .simulation_mut()
            .set_combat(
                player.id(),
                CombatState {
                    health: 50.0,
                    ..CombatState::default()
                },
            )
            .unwrap();
        {
            let contact = TouchContact {
                trigger: item.id().clone(),
                other: player.id().clone(),
            };
            let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
            q1_item_touch(&mut shared.borrow_mut(), simulation, movers, triggers, &contact);
        }
        assert_eq!(health_of(&server, player.id()), 75.0);
    }

    #[test]
    fn megahealth_rots_to_cap_then_respawns_in_deathmatch() {
        let mut server = test_server();
        register_all(&mut server);
        let shared = Rc::new(RefCell::new(Q1NativeBehaviors::new()));
        shared.borrow_mut().deathmatch = true;
        let item = spawn_item(
            &mut server,
            &mut shared.borrow_mut(),
            &item_fields("item_health", &[("origin", "0 0 0"), ("spawnflags", "2")]),
        );
        let player = spawn_player(&mut server, vec3(4000.0, 4000.0, 4000.0), 100.0);
        admit_player(&mut shared.borrow_mut(), &player);
        super::super::native_q1_spawns::install_q1_native(&mut server, Rc::clone(&shared));
        {
            let contact = TouchContact {
                trigger: item.id().clone(),
                other: player.id().clone(),
            };
            let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
            q1_item_touch(&mut shared.borrow_mut(), simulation, movers, triggers, &contact);
        }
        assert_eq!(health_of(&server, player.id()), 200.0);
        assert_ne!(shared.borrow().player_items & IT_SUPERHEALTH, 0);
        // First rot after 5s.
        for _ in 0..101 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        assert_eq!(health_of(&server, player.id()), 199.0);
        assert_ne!(shared.borrow().player_items & IT_SUPERHEALTH, 0);
        // At the cap the rot stops, clears the bit, and respawns the box.
        server
            .simulation_mut()
            .set_combat(
                player.id(),
                CombatState {
                    health: 100.0,
                    ..CombatState::default()
                },
            )
            .unwrap();
        for _ in 0..21 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        assert_eq!(shared.borrow().player_items & IT_SUPERHEALTH, 0);
        assert!(shared
            .borrow()
            .thinks
            .iter()
            .any(|think| think.kind == Q1ThinkKind::Regen));
        for _ in 0..401 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        assert!(!shared.borrow().items.get(item.id()).unwrap().taken);
        assert!(server.triggers_mut().is_trigger(item.id()));
    }
}
