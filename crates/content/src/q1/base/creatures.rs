//! Base creature services (`src/content/q1/base/creatures.ts`).
//!
//! Base Quake campaign and boss behavior. Copyright (C) 1996-2022 id
//! Software LLC. GPL-2.0-or-later.
//!
//! The donor's `Q1Creatures` class holds per-game maps behind a
//! `WeakMap`. Here the maps live in [`Q1CreatureState`], owned by the
//! provider registry; this module implements the store operations,
//! species registration, checkpoint fields, and the `q1Creatures`
//! accessors as free functions, since a borrowing handle cannot cross
//! the bare function pointers used for callbacks.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::base::monsters::BaseMonsterState;
use crate::q1::base::projectiles::{BackpackContents, BackpackSelection};
use crate::q1::base::provider::update_base;
use crate::q1::base::species::{species_by_classname, BASE_SPECIES};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{Q1Weapon, WEAPONS};
use crate::q1::{q1_error, Q1Error};
use crate::value::{arr, boolean, int, namespaced, num, obj, str as save_str, SaveJson, SaveReader};

/// Pending wizard shot (`Q1Creatures["wizardShots"]` value).
#[derive(Debug, Clone, PartialEq)]
pub struct WizardShot {
    /// Shot target.
    pub enemy: ActorId,
    /// Lateral offset.
    pub right: Vec3,
}

/// Base creature store (`Q1Creatures` maps). Ordered vectors preserve
/// the donor `Map` insertion order for iteration and checkpoints.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q1CreatureState {
    /// Monster controllers by actor.
    pub monsters: Vec<(ActorId, BaseMonsterState)>,
    /// Backpack contents by actor.
    pub backpacks: Vec<(ActorId, BackpackContents)>,
    /// Homing projectile targets by actor.
    pub projectile_targets: Vec<(ActorId, ActorId)>,
    /// Pending wizard shots by actor.
    pub wizard_shots: Vec<(ActorId, WizardShot)>,
    /// Hell Knight melee alternation counter.
    pub hell_knight_type: f64,
}

fn map_get<K: PartialEq, V: Clone>(entries: &[(K, V)], key: &K) -> Option<V> {
    entries
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.clone())
}

fn map_set<K: PartialEq, V>(entries: &mut Vec<(K, V)>, key: K, value: V) {
    match entries.iter_mut().find(|(candidate, _)| *candidate == key) {
        Some((_, current)) => *current = value,
        None => entries.push((key, value)),
    }
}

fn map_remove<K: PartialEq, V>(entries: &mut Vec<(K, V)>, key: &K) {
    entries.retain(|(candidate, _)| candidate != key);
}

/// Load a monster controller or fail with the donor message.
pub(crate) fn monster_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    classname: &str,
) -> Result<BaseMonsterState, Q1Error> {
    update_base(game, |state| map_get(&state.creatures.monsters, id))?
        .ok_or_else(|| q1_error(format!("Missing Q1 monster controller for {classname}")))
}

/// Store a monster controller. Entities already released were cleaned
/// by the release hook and are not resurrected.
pub(crate) fn store_monster_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    controller: BaseMonsterState,
) -> Result<(), Q1Error> {
    if game.entity_ref(id).is_none() {
        return Ok(());
    }
    update_base(game, |state| {
        map_set(&mut state.creatures.monsters, id.clone(), controller)
    })
}

/// Read backpack contents or fail with the donor message.
pub(crate) fn backpack_contents(game: &Q1EntityServices, id: &ActorId) -> Result<BackpackContents, Q1Error> {
    update_base(game, |state| map_get(&state.creatures.backpacks, id))?
        .ok_or_else(|| q1_error("Missing source backpack contents"))
}

/// Store backpack contents.
pub(crate) fn set_backpack_contents(
    game: &Q1EntityServices,
    id: &ActorId,
    contents: BackpackContents,
) -> Result<(), Q1Error> {
    update_base(game, |state| {
        map_set(&mut state.creatures.backpacks, id.clone(), contents)
    })
}

/// Read a homing projectile target or fail with the donor message.
pub(crate) fn projectile_target(game: &Q1EntityServices, id: &ActorId) -> Result<ActorId, Q1Error> {
    update_base(game, |state| map_get(&state.creatures.projectile_targets, id))?
        .ok_or_else(|| q1_error("Vore missile has no source enemy"))
}

/// Store a homing projectile target.
pub(crate) fn set_projectile_target(game: &Q1EntityServices, id: &ActorId, enemy: &ActorId) -> Result<(), Q1Error> {
    let enemy = enemy.clone();
    update_base(game, |state| {
        map_set(&mut state.creatures.projectile_targets, id.clone(), enemy)
    })
}

/// Read a pending wizard shot or fail with the donor message.
pub(crate) fn wizard_shot(game: &Q1EntityServices, id: &ActorId) -> Result<WizardShot, Q1Error> {
    update_base(game, |state| map_get(&state.creatures.wizard_shots, id))?
        .ok_or_else(|| q1_error("Wizard shot has no source target"))
}

/// Store a pending wizard shot.
pub(crate) fn set_wizard_shot(game: &Q1EntityServices, id: &ActorId, shot: WizardShot) -> Result<(), Q1Error> {
    update_base(game, |state| {
        map_set(&mut state.creatures.wizard_shots, id.clone(), shot)
    })
}

/// Register base species spawn handlers (`registerSpecies`).
pub fn register_species(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    for species in BASE_SPECIES {
        for classname in species.classnames {
            game.register_spawn(classname, super::monsters::spawn_base_monster)?;
        }
    }
    Ok(())
}

/// Register creature callbacks: base monster callbacks, projectile
/// callbacks, and the wizard fast-fire timer.
pub fn register_creature_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    super::monsters::register_monster_callbacks(game, "base")?;
    super::projectiles::register_projectile_callbacks(game)?;
    game.named.register(
        "base:wizard_fastfire",
        crate::q1::foundation::callbacks::Q1CallbackHandlers {
            action: Some(super::monster_actions::wizard_fast_fire),
            ..Default::default()
        },
    )
}

/// Next Hell Knight melee frame, alternating slice, smash, and back to
/// walk (`nextHellKnightMelee`).
pub fn next_hell_knight_melee(state: &mut Q1CreatureState) -> String {
    state.hell_knight_type += 1.0;
    if state.hell_knight_type == 1.0 {
        String::from("hknight_slice1")
    } else if state.hell_knight_type == 2.0 {
        String::from("hknight_smash1")
    } else {
        state.hell_knight_type = 0.0;
        String::from("hknight_watk1")
    }
}

fn saved_actor_json(actor: &ActorId) -> SaveJson {
    obj(vec![
        ("slot", int(i64::from(actor.slot()))),
        ("generation", int(i64::from(actor.generation()))),
    ])
}

/// Capture creature checkpoint fields (`captureFields`).
pub fn capture_creature_fields(state: &Q1CreatureState) -> Vec<(&'static str, SaveJson)> {
    vec![
        ("hellKnightType", num(state.hell_knight_type)),
        (
            "monsters",
            arr(state
                .monsters
                .iter()
                .map(|(actor, controller)| {
                    obj(vec![
                        ("actor", saved_actor_json(actor)),
                        ("state", controller.capture()),
                    ])
                })
                .collect()),
        ),
        (
            "backpacks",
            arr(state
                .backpacks
                .iter()
                .map(|(actor, contents)| {
                    obj(vec![
                        ("actor", saved_actor_json(actor)),
                        (
                            "contents",
                            obj(vec![
                                (
                                    "weapon",
                                    contents
                                        .weapon
                                        .map(|weapon| save_str(weapon.as_str()))
                                        .unwrap_or(SaveJson::Null),
                                ),
                                ("shells", num(contents.shells)),
                                ("nails", num(contents.nails)),
                                ("rockets", num(contents.rockets)),
                                ("cells", num(contents.cells)),
                                (
                                    "extra",
                                    arr(contents
                                        .extra
                                        .iter()
                                        .map(|entry| {
                                            obj(vec![
                                                ("item", save_str(entry.item.as_str())),
                                                ("count", num(entry.count)),
                                            ])
                                        })
                                        .collect()),
                                ),
                                ("selection", save_str(contents.selection.as_str())),
                                ("avoidUnderwaterLightning", boolean(contents.avoid_underwater_lightning)),
                                ("ownerPickupDelay", num(contents.owner_pickup_delay)),
                            ]),
                        ),
                    ])
                })
                .collect()),
        ),
        (
            "targets",
            arr(state
                .projectile_targets
                .iter()
                .map(|(actor, enemy)| {
                    obj(vec![
                        ("actor", saved_actor_json(actor)),
                        ("enemy", saved_actor_json(enemy)),
                    ])
                })
                .collect()),
        ),
        (
            "shots",
            arr(state
                .wizard_shots
                .iter()
                .map(|(actor, shot)| {
                    obj(vec![
                        ("actor", saved_actor_json(actor)),
                        ("enemy", saved_actor_json(&shot.enemy)),
                        (
                            "right",
                            obj(vec![
                                ("x", num(f64::from(shot.right.x))),
                                ("y", num(f64::from(shot.right.y))),
                                ("z", num(f64::from(shot.right.z))),
                            ]),
                        ),
                    ])
                })
                .collect()),
        ),
    ]
}

/// Restore creature checkpoint fields (`restoreFields`).
pub fn restore_creature_fields(game: &mut Q1EntityServices, root: SaveReader) -> Result<Q1CreatureState, Q1Error> {
    let mut state = Q1CreatureState {
        hell_knight_type: root.field("hellKnightType").number()?,
        ..Default::default()
    };
    for (actor, controller) in root.field("monsters").list(|reader| {
        let owner = super::provider::resolve_saved_actor(game, reader.field("actor"))?;
        let entity = game
            .entity_ref(owner.id())
            .ok_or_else(|| Q1Error::from(reader.fail("missing source monster entity")))?;
        let spec = species_by_classname(&entity.classname)
            .ok_or_else(|| Q1Error::from(reader.fail("unknown base monster")))?;
        let controller = BaseMonsterState::restore(spec.species, String::from("base"), reader.field("state"))?;
        Ok::<_, Q1Error>((owner.id().clone(), controller))
    })? {
        state.monsters.push((actor, controller));
    }
    let mut weapons: Vec<String> = WEAPONS
        .iter()
        .map(|weapon| Q1Weapon::from(*weapon).as_str().to_string())
        .collect();
    for weapon in game.registered_weapons.keys() {
        if !weapons.contains(&weapon.as_str().to_string()) {
            weapons.push(weapon.as_str().to_string());
        }
    }
    for (actor, contents) in root.field("backpacks").list(|reader| {
        let owner = super::provider::resolve_saved_actor(game, reader.field("actor"))?;
        let data = reader.field("contents");
        let weapon = data.field("weapon").nullable(|value| {
            let text = value.string()?;
            Q1Weapon::parse(&text)
                .ok()
                .filter(|weapon| weapons.contains(&weapon.as_str().to_string()))
                .ok_or_else(|| Q1Error::from(value.fail(&format!("expected {}", weapons.join(" or ")))))
        })?;
        let mut extra = Vec::new();
        for entry in data.field("extra").list(|entry| {
            Ok::<_, Q1Error>(crate::q1::base::projectiles::BackpackExtra {
                item: namespaced(entry.field("item"))?,
                count: entry.field("count").number()?,
            })
        })? {
            extra.push(entry);
        }
        let selection = match data
            .field("selection")
            .choice_str(&["source-default", "rank"])?
            .as_str()
        {
            "rank" => BackpackSelection::Rank,
            _ => BackpackSelection::SourceDefault,
        };
        Ok::<_, Q1Error>((
            owner.id().clone(),
            BackpackContents {
                weapon,
                shells: data.field("shells").number()?,
                nails: data.field("nails").number()?,
                rockets: data.field("rockets").number()?,
                cells: data.field("cells").number()?,
                extra,
                selection,
                avoid_underwater_lightning: data.field("avoidUnderwaterLightning").boolean()?,
                owner_pickup_delay: data.field("ownerPickupDelay").number()?,
            },
        ))
    })? {
        state.backpacks.push((actor, contents));
    }
    for (actor, enemy) in root.field("targets").list(|reader| {
        let owner = super::provider::resolve_saved_actor(game, reader.field("actor"))?;
        let saved = super::provider::saved_actor(reader.field("enemy"))?;
        let enemy = game.host.actors.reference_saved(&saved);
        Ok::<_, Q1Error>((owner.id().clone(), enemy))
    })? {
        state.projectile_targets.push((actor, enemy));
    }
    for (actor, shot) in root.field("shots").list(|reader| {
        let owner = super::provider::resolve_saved_actor(game, reader.field("actor"))?;
        let saved = super::provider::saved_actor(reader.field("enemy"))?;
        let enemy = game.host.actors.reference_saved(&saved);
        let right = reader.field("right");
        Ok::<_, Q1Error>((
            owner.id().clone(),
            WizardShot {
                enemy,
                right: Vec3 {
                    x: right.field("x").number()? as f32,
                    y: right.field("y").number()? as f32,
                    z: right.field("z").number()? as f32,
                },
            },
        ))
    })? {
        state.wizard_shots.push((actor, shot));
    }
    Ok(state)
}

/// Duplicate initialized source state for a cloned entity (`clone`).
pub fn clone_creature_fields(state: &mut Q1CreatureState, source: &ActorId, target: &ActorId) {
    if let Some(controller) = map_get(&state.monsters, source) {
        map_set(&mut state.monsters, target.clone(), controller);
    }
    if let Some(contents) = map_get(&state.backpacks, source) {
        map_set(&mut state.backpacks, target.clone(), contents);
    }
    if let Some(enemy) = map_get(&state.projectile_targets, source) {
        map_set(&mut state.projectile_targets, target.clone(), enemy);
    }
    if let Some(shot) = map_get(&state.wizard_shots, source) {
        map_set(&mut state.wizard_shots, target.clone(), shot);
    }
}

/// Drop creature state for a released actor.
pub fn release_creature_actor(state: &mut Q1CreatureState, actor: &ActorId) {
    map_remove(&mut state.monsters, actor);
    map_remove(&mut state.backpacks, actor);
    map_remove(&mut state.projectile_targets, actor);
    map_remove(&mut state.wizard_shots, actor);
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::*;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::foundation::host::mock::mock_host;
    use crate::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};

    fn options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    #[test]
    fn species_spawn_and_release_cycle() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let knight = game.create("monster_knight", None, None).expect("knight");
        game.spawn_entity(&knight, None).expect("spawn");
        assert_eq!(game.total_monsters, 1);
        let controller = monster_controller(
            &game,
            &knight,
            &game
                .entity_ref(&knight)
                .map(|entity| entity.classname.clone())
                .unwrap_or_default(),
        )
        .expect("controller");
        assert_eq!(controller.current_frame, "knight_stand1");
        let contents = BackpackContents {
            weapon: None,
            shells: 0.0,
            nails: 0.0,
            rockets: 2.0,
            cells: 0.0,
            extra: Vec::new(),
            selection: BackpackSelection::SourceDefault,
            avoid_underwater_lightning: false,
            owner_pickup_delay: 0.0,
        };
        set_backpack_contents(&game, &knight, contents).expect("store");
        assert_eq!(backpack_contents(&game, &knight).expect("contents").rockets, 2.0);
        game.remove(&knight).expect("remove");
        assert!(update_base(&game, |state| map_get(&state.creatures.monsters, &knight).is_none()).expect("state"));
        assert!(update_base(&game, |state| map_get(&state.creatures.backpacks, &knight).is_none()).expect("state"));
    }

    #[test]
    fn checkpoint_fields_round_trip() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let knight = game.create("monster_knight", None, None).expect("knight");
        game.spawn_entity(&knight, None).expect("spawn");
        let fields = update_base(&game, |state| capture_creature_fields(&state.creatures)).expect("capture");
        let names: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
        assert!(names.contains(&"monsters"));
        let restored = restore_creature_fields(&mut game, SaveReader::at(&obj(fields), "test")).expect("restore");
        assert_eq!(restored.monsters.len(), 1);
        assert_eq!(restored.monsters[0].1.current_frame, "knight_stand1");
    }
}
