//! Mission-pack items (src/content/q1/missionpacks/items.ts).

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::contract::{ItemId, PickupAmmoGrant, PickupMapKind, PickupSelection, PickupWeaponGrant};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity::Q1Actor;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::TouchSurface;
use crate::q1::foundation::types::{
    weapon_item, Q1Edition, Q1Effect, Q1MoveType, Q1Powerup, Q1Solid, Q1SoundChannel, Q1Weapon, ZERO,
};
use crate::q1::{q1_error, Q1Error};

use super::arsenal::MissionPackArsenal;
use super::messages::{mission_message, mission_pickup_message};
use super::player::MissionPackPlayers;
use super::types::{MissionWeapon, MissionWeaponDefinition, Q1MissionPack, MISSION_WEAPONS};

/// Mission-pack item services (`MissionItemServices`).
pub struct MissionItemServices {
    /// Mission pack.
    pub pack: Q1MissionPack,
}

impl MissionItemServices {
    /// Service view for a pack.
    #[must_use]
    pub fn new(pack: Q1MissionPack) -> Self {
        Self { pack }
    }

    /// Player services.
    fn players(&self) -> MissionPackPlayers {
        MissionPackPlayers::for_pack(self.pack)
    }

    /// Fire the horn of conjuring.
    pub fn horn(&self, game: &mut Q1EntityServices, item: &ActorId, player: &ActorId) -> Result<(), Q1Error> {
        MissionPackArsenal::use_horn(game, item, player)
    }

    /// Grant a vengeance sphere.
    pub fn sphere(&self, game: &mut Q1EntityServices, item: &ActorId, player: &ActorId) -> Result<bool, Q1Error> {
        self.players().sphere(game, item, player)
    }

    /// Grant a timed powerup.
    pub fn powerup(
        &self,
        game: &mut Q1EntityServices,
        player: &ActorId,
        powerup: Q1Powerup,
        seconds: f64,
    ) -> Result<(), Q1Error> {
        self.players().powerup(game, player, powerup, seconds)
    }

    /// Enable Rogue combo weapons.
    pub fn enable_combos(&self, game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
        self.players().enable_combos(game, player)
    }
}

/// Shared item presentation (`ItemAppearance`).
struct ItemAppearance {
    model: String,
    sound: String,
    name: String,
    bounds: Bounds,
}

/// Mission-pack item kind (`MissionItem`).
enum MissionItemKind {
    Weapon { weapon: MissionWeaponDefinition },
    Ammo { item: ItemId, amount: f64 },
    Powerup { powerup: Q1Powerup, seconds: f64 },
    Horn,
    Sphere,
}

/// Mission-pack item (`MissionItem`).
struct MissionItem {
    appearance: ItemAppearance,
    kind: MissionItemKind,
}

fn artifact_bounds() -> Bounds {
    Bounds {
        min: Vec3 {
            x: -16.0,
            y: -16.0,
            z: -24.0,
        },
        max: Vec3 {
            x: 16.0,
            y: 16.0,
            z: 32.0,
        },
    }
}

fn floor_bounds() -> Bounds {
    Bounds {
        min: Vec3 {
            x: -16.0,
            y: -16.0,
            z: 0.0,
        },
        max: Vec3 {
            x: 16.0,
            y: 16.0,
            z: 32.0,
        },
    }
}

fn weapon_bounds() -> Bounds {
    Bounds {
        min: Vec3 {
            x: -16.0,
            y: -16.0,
            z: 0.0,
        },
        max: Vec3 {
            x: 16.0,
            y: 16.0,
            z: 56.0,
        },
    }
}

/// Roll a Rogue random powerup type (`randomType`).
fn random_type(game: &mut Q1EntityServices, item: &ActorId) -> Result<(), Q1Error> {
    let value = game.host.random();
    let rolled = if value < 0.2 {
        "item_powerup_shield"
    } else if value < 0.4 {
        "item_powerup_belt"
    } else if value < 0.6 {
        "item_artifact_invulnerability"
    } else if value < 0.8 {
        "item_artifact_invisibility"
    } else {
        "item_artifact_super_damage"
    };
    game.update_entity(item, |item| {
        item.fields.insert("rogue:random-type".to_string(), rolled.to_string());
    })
}

/// Resolve an entity to its mission-pack item (`definition`).
fn definition(entity: &Q1Actor) -> Option<MissionItem> {
    let name = if entity.classname == "item_random_powerup" {
        entity.text("rogue:random-type")
    } else {
        entity.classname.clone()
    };
    if let Some(weapon) = MISSION_WEAPONS
        .iter()
        .find(|candidate| candidate.pickup == Some(name.as_str()))
    {
        let pickup_name = match weapon.id {
            MissionWeapon::HipnoticLaser => "$qc_laser_cannon",
            MissionWeapon::HipnoticMjolnir => "$qc_mjolnir",
            _ => "$qc_prox_gun",
        };
        return Some(MissionItem {
            appearance: ItemAppearance {
                model: weapon.world_model.to_string(),
                sound: "weapons/pkup.wav".to_string(),
                name: pickup_name.to_string(),
                bounds: weapon_bounds(),
            },
            kind: MissionItemKind::Weapon { weapon: *weapon },
        });
    }
    let powerup_item = |powerup, seconds: f64, model: &str, sound: &str, name: &str, bounds: Bounds| MissionItem {
        appearance: ItemAppearance {
            model: model.to_string(),
            sound: sound.to_string(),
            name: name.to_string(),
            bounds,
        },
        kind: MissionItemKind::Powerup { powerup, seconds },
    };
    if name == "item_artifact_wetsuit" {
        return Some(powerup_item(
            Q1Powerup::HipnoticWetsuit,
            30.0,
            "progs/wetsuit.mdl",
            "misc/weton.wav",
            "$qc_wetsuit",
            artifact_bounds(),
        ));
    }
    if name == "item_artifact_empathy_shields" {
        return Some(powerup_item(
            Q1Powerup::HipnoticEmpathy,
            30.0,
            "progs/empathy.mdl",
            "hipitems/empathy.wav",
            "$qc_empathy_shields",
            floor_bounds(),
        ));
    }
    if name == "item_hornofconjuring" {
        return Some(MissionItem {
            appearance: ItemAppearance {
                model: "progs/horn.mdl".to_string(),
                sound: "hipitems/horn.wav".to_string(),
                name: "$qc_horn_of_conjuring".to_string(),
                bounds: floor_bounds(),
            },
            kind: MissionItemKind::Horn,
        });
    }
    if name == "item_powerup_shield" {
        return Some(powerup_item(
            Q1Powerup::RogueShield,
            30.0,
            "progs/shield.mdl",
            "shield/pickup.wav",
            "$qc_power_shield",
            artifact_bounds(),
        ));
    }
    if name == "item_powerup_belt" {
        return Some(powerup_item(
            Q1Powerup::RogueAntigrav,
            45.0,
            "progs/beltup.mdl",
            "belt/pickup.wav",
            "$qc_anti_grav_belt",
            artifact_bounds(),
        ));
    }
    if name == "item_sphere" {
        return Some(MissionItem {
            appearance: ItemAppearance {
                model: "progs/sphere.mdl".to_string(),
                sound: "sphere/sphere.wav".to_string(),
                name: "$qc_vengeance_sphere".to_string(),
                bounds: Bounds {
                    min: Vec3 {
                        x: -8.0,
                        y: -8.0,
                        z: -8.0,
                    },
                    max: Vec3 { x: 8.0, y: 8.0, z: 8.0 },
                },
            },
            kind: MissionItemKind::Sphere,
        });
    }
    if name == "item_lava_spikes" || name == "item_multi_rockets" || name == "item_plasma" {
        let lava = name == "item_lava_spikes";
        let rockets = name == "item_multi_rockets";
        let big = entity.spawnflags & 1 != 0;
        let amount = (if lava {
            25.0
        } else if rockets {
            5.0
        } else {
            6.0
        }) * (if big { 2.0 } else { 1.0 });
        let base = if lava {
            "lnail"
        } else if rockets {
            "mrock"
        } else {
            "plas"
        };
        return Some(MissionItem {
            appearance: ItemAppearance {
                model: format!("maps/b_{base}{}.bsp", if big { 1 } else { 0 }),
                sound: "weapons/lock4.wav".to_string(),
                name: (if lava {
                    "$qc_lava_nails"
                } else if rockets {
                    "$qc_multi_rockets"
                } else {
                    "plasma"
                })
                .to_string(),
                bounds: Bounds {
                    min: ZERO,
                    max: Vec3 {
                        x: 32.0,
                        y: 32.0,
                        z: 56.0,
                    },
                },
            },
            kind: MissionItemKind::Ammo {
                item: ItemId::from(if lava {
                    "rogue:ammo/lava-nails"
                } else if rockets {
                    "rogue:ammo/multi-rockets"
                } else {
                    "rogue:ammo/plasma"
                }),
                amount,
            },
        });
    }
    if entity.classname == "item_random_powerup" {
        let powerup = if name == "item_artifact_invulnerability" {
            Q1Powerup::Invulnerability
        } else if name == "item_artifact_invisibility" {
            Q1Powerup::Invisibility
        } else {
            Q1Powerup::Quad
        };
        let model = match powerup {
            Q1Powerup::Invulnerability => "invulner",
            Q1Powerup::Invisibility => "invisibl",
            _ => "quaddama",
        };
        let sound = match powerup {
            Q1Powerup::Invulnerability => "protect",
            Q1Powerup::Invisibility => "inv1",
            _ => "damage",
        };
        let pickup_name = match powerup {
            Q1Powerup::Quad => "$qc_quad_damage",
            Q1Powerup::Invisibility => "$qc_ring_of_shadows",
            _ => "$qc_pentagram_of_protection",
        };
        return Some(powerup_item(
            powerup,
            30.0,
            &format!("progs/{model}.mdl"),
            &format!("items/{sound}.wav"),
            pickup_name,
            artifact_bounds(),
        ));
    }
    None
}

/// Hipnotic weapon rank (`hipnoticWeaponRank`).
#[must_use]
pub fn hipnotic_weapon_rank(weapon: Q1Weapon) -> i32 {
    match weapon {
        Q1Weapon::Lightning => 1,
        Q1Weapon::Rocketlauncher => 2,
        Q1Weapon::HipnoticLaser => 3,
        Q1Weapon::Supernailgun => 4,
        Q1Weapon::HipnoticProximity => 5,
        Q1Weapon::Grenadelauncher => 6,
        Q1Weapon::Supershotgun => 7,
        Q1Weapon::Nailgun => 8,
        Q1Weapon::HipnoticMjolnir => 9,
        _ => 10,
    }
}

/// Weapon take verdict (`takeWeapon` return).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TakeWeapon {
    Refused,
    Leave,
    Taken,
}

/// Take a mission-pack weapon (`takeWeapon`).
fn take_weapon(
    game: &mut Q1EntityServices,
    player: &ActorId,
    weapon: &MissionWeaponDefinition,
) -> Result<TakeWeapon, Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    let leave_hook = game.pickup_rules.as_ref().and_then(|rules| rules.weapon_leave);
    let leave = match leave_hook {
        Some(hook) => hook(game)?,
        None => false,
    };
    let item = weapon_item(Q1Weapon::from(weapon.id));
    let admitted = game
        .pickup_admission
        .as_ref()
        .is_some_and(|admission| admission.maps(PickupMapKind::Weapons, &item));
    let owned = if admitted {
        game.pickup_admission
            .as_ref()
            .is_some_and(|admission| admission.owns(player, &item))
    } else {
        game.host.inventory.count(player, &item) > 0.0
    };
    if leave && owned {
        return Ok(TakeWeapon::Refused);
    }
    let auto_switch = game.pickup_rules.as_ref().and_then(|rules| rules.auto_switch);
    if admitted {
        let switch = match auto_switch {
            Some(hook) => hook(game, player, owned)?,
            None => true,
        };
        let deathmatch = game.options().deathmatch;
        let selection = if switch {
            if deathmatch == 0 {
                PickupSelection::Always
            } else {
                PickupSelection::Better
            }
        } else {
            PickupSelection::Never
        };
        let granted = game.pickup_admission.as_ref().is_some_and(|admission| {
            admission.weapon(
                &state.actor,
                &PickupWeaponGrant {
                    item: item.clone(),
                    ammo: vec![PickupAmmoGrant {
                        item: ItemId::from(weapon.ammo.unwrap_or("q1:ammo/cells")),
                        amount: f64::from(weapon.pickup_ammo),
                    }],
                },
                selection,
            )
        });
        if !granted {
            return Ok(TakeWeapon::Refused);
        }
        return Ok(if leave { TakeWeapon::Leave } else { TakeWeapon::Taken });
    }
    game.host.inventory.give(&state.actor, &item, 1.0);
    game.host.inventory.give(
        &state.actor,
        &ItemId::from(weapon.ammo.unwrap_or("q1:ammo/cells")),
        f64::from(weapon.pickup_ammo),
    );
    let switch = match auto_switch {
        Some(hook) => hook(game, player, owned)?,
        None => true,
    };
    if switch {
        let deathmatch = game.options().deathmatch;
        if deathmatch == 0 || weapon.rank < hipnotic_weapon_rank(state.weapon) {
            game.select_weapon(&state.actor, Q1Weapon::from(weapon.id))?;
        }
    }
    Ok(if leave { TakeWeapon::Leave } else { TakeWeapon::Taken })
}

/// Pick up a mission-pack item (`pickup`).
fn pickup(
    game: &mut Q1EntityServices,
    entity: &ActorId,
    other: &ActorId,
    services: &MissionItemServices,
) -> Result<(), Q1Error> {
    let record = match game.entity_ref(entity) {
        Some(record) => record.clone(),
        None => return Ok(()),
    };
    let item = match definition(&record) {
        Some(item) => item,
        None => return Ok(()),
    };
    let state = match game.player_ref(other) {
        Some(state) => state.clone(),
        None => return Ok(()),
    };
    if record.solid != Q1Solid::Trigger {
        return Ok(());
    }
    if !matches!(item.kind, MissionItemKind::Horn) && game.health(other) <= 0.0 {
        return Ok(());
    }
    let mut result = TakeWeapon::Taken;
    match &item.kind {
        MissionItemKind::Weapon { weapon } => {
            result = take_weapon(game, other, weapon)?;
        }
        MissionItemKind::Ammo { item, amount } => {
            let admitted = game
                .pickup_admission
                .as_ref()
                .is_some_and(|admission| admission.maps(PickupMapKind::Ammo, item));
            if admitted {
                let edition = game.options().edition;
                let auto = edition == Q1Edition::Classic
                    || state.auto_switch != crate::q1::foundation::types::Q1AutoSwitch::Never;
                let granted = game.pickup_admission.as_ref().is_some_and(|admission| {
                    admission.ammo(
                        &state.actor,
                        &PickupAmmoGrant {
                            item: item.clone(),
                            amount: *amount,
                        },
                        auto,
                    )
                });
                if !granted {
                    return Ok(());
                }
            } else {
                let best = game.choose_best(&state.actor, None)?;
                if game.host.inventory.give(&state.actor, item, *amount) == 0.0 {
                    return Ok(());
                }
                services.enable_combos(game, other)?;
                let edition = game.options().edition;
                if state.weapon == best
                    && (edition == Q1Edition::Classic
                        || state.auto_switch != crate::q1::foundation::types::Q1AutoSwitch::Never)
                {
                    let best = game.choose_best(&state.actor, None)?;
                    game.select_weapon(&state.actor, best)?;
                }
            }
        }
        MissionItemKind::Powerup { powerup, seconds } => {
            services.powerup(game, other, *powerup, *seconds)?;
        }
        MissionItemKind::Sphere => {
            if !services.sphere(game, entity, other)? {
                return Ok(());
            }
        }
        MissionItemKind::Horn => {}
    }
    if result == TakeWeapon::Refused {
        return Ok(());
    }
    if matches!(item.kind, MissionItemKind::Horn) {
        mission_message(game, Some(other), "$qc_got_horn");
    } else {
        mission_pickup_message(game, other, &item.appearance.name);
    }
    let channel = if matches!(item.kind, MissionItemKind::Weapon { .. } | MissionItemKind::Ammo { .. }) {
        Q1SoundChannel::Item
    } else {
        Q1SoundChannel::Voice
    };
    game.sound(
        other,
        &item.appearance.sound,
        channel,
        if matches!(item.kind, MissionItemKind::Horn) {
            0.0
        } else {
            1.0
        },
        1.0,
    )?;
    game.effect(Q1Effect::Pickup, game.body(entity)?.origin, Some(other), 1);
    if result == TakeWeapon::Leave {
        return Ok(());
    }
    game.update_entity(entity, |entity| {
        entity.solid = Q1Solid::None;
        entity.model.clear();
    })?;
    game.link(entity)?;
    let edition = game.options().edition;
    let deathmatch = game.options().deathmatch;
    let random_powerup = record.classname == "item_random_powerup"
        && matches!(&item.kind, MissionItemKind::Powerup { powerup, .. }
            if !matches!(powerup, Q1Powerup::RogueShield | Q1Powerup::RogueAntigrav));
    let respawn = if matches!(item.kind, MissionItemKind::Sphere) {
        180.0
    } else if matches!(item.kind, MissionItemKind::Ammo { .. })
        && edition == Q1Edition::Rerelease
        && (deathmatch == 3 || deathmatch == 5)
    {
        15.0
    } else if matches!(item.kind, MissionItemKind::Weapon { .. } | MissionItemKind::Ammo { .. }) || random_powerup {
        30.0
    } else {
        60.0
    };
    let returns = if matches!(item.kind, MissionItemKind::Weapon { .. } | MissionItemKind::Ammo { .. }) {
        if edition == Q1Edition::Classic {
            deathmatch == 1
        } else {
            deathmatch != 0 && deathmatch != 2
        }
    } else {
        deathmatch != 0
    };
    if returns {
        let regen = if record.classname == "item_random_powerup" {
            game.named.action("missionpack:random-regen")?
        } else {
            game.named.action("SUB_regen")?
        };
        game.schedule(entity, respawn, &regen)?;
    } else {
        game.cancel(entity);
    }
    if matches!(item.kind, MissionItemKind::Horn) {
        services.horn(game, entity, other)
    } else {
        game.use_targets(entity, Some(other))
    }
}

/// Hipnotic item touch hook.
fn pickup_hipnotic(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    pickup(game, id, other, &MissionItemServices::new(Q1MissionPack::Hipnotic))
}

/// Rogue item touch hook.
fn pickup_rogue(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    pickup(game, id, other, &MissionItemServices::new(Q1MissionPack::Rogue))
}

/// Regenerate a Rogue random powerup (`missionpack:random-regen`).
fn random_regen(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    random_type(game, id)?;
    let record = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Rogue random powerup definition"))?;
    let item = definition(&record).ok_or_else(|| q1_error("Missing Rogue random powerup definition"))?;
    game.update_entity(id, |entity| {
        entity.model = item.appearance.model.clone();
        entity.original_model = item.appearance.model.clone();
        entity.solid = Q1Solid::Trigger;
    })?;
    game.sound_simple(id, "items/itembk2.wav")?;
    game.link(id)
}

/// Spawn a mission-pack item.
fn spawn_mission_item(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let classname = game
        .entity_ref(id)
        .map(|entity| entity.classname.clone())
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let deathmatch = game.options().deathmatch;
    if deathmatch == 0 && (classname == "item_sphere" || classname == "item_random_powerup") {
        return game.remove(id);
    }
    if classname == "item_random_powerup" {
        random_type(game, id)?;
    }
    let record = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let item = definition(&record).ok_or_else(|| q1_error(format!("Unknown mission-pack item {classname}")))?;
    game.update_entity(id, |entity| {
        entity.model = item.appearance.model.clone();
        entity.original_model = item.appearance.model.clone();
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
        if matches!(item.kind, MissionItemKind::Sphere) {
            entity.angular_velocity = Vec3 {
                x: 40.0,
                y: 40.0,
                z: 40.0,
            };
        }
    })?;
    let touch = game.named.touch("missionpack:item-touch")?;
    game.update_entity(id, |entity| entity.touch = Some(touch))?;
    game.set_bounds(id, item.appearance.bounds)?;
    let place = game.named.action("PlaceItem")?;
    game.schedule(id, 0.2, &place)
}

/// Register mission-pack items (`registerMissionPackItems`).
pub fn register_mission_pack_items(game: &mut Q1EntityServices, services: &MissionItemServices) -> Result<(), Q1Error> {
    game.named.register(
        "missionpack:item-touch",
        Q1CallbackHandlers {
            touch: Some(if services.pack == Q1MissionPack::Hipnotic {
                pickup_hipnotic
            } else {
                pickup_rogue
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "missionpack:random-regen",
        Q1CallbackHandlers {
            action: Some(random_regen),
            ..Default::default()
        },
    )?;
    let classnames: &[&str] = if services.pack == Q1MissionPack::Hipnotic {
        &[
            "weapon_laser_gun",
            "weapon_mjolnir",
            "weapon_proximity_gun",
            "item_artifact_wetsuit",
            "item_artifact_empathy_shields",
            "item_hornofconjuring",
        ]
    } else {
        &[
            "item_lava_spikes",
            "item_multi_rockets",
            "item_plasma",
            "item_powerup_shield",
            "item_powerup_belt",
            "item_sphere",
            "item_random_powerup",
        ]
    };
    for classname in classnames {
        game.register_spawn(classname, spawn_mission_item)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::types::test_game;
    use super::*;
    use crate::q1::foundation::entity_services::Q1AttachOptions;

    fn attached_player(game: &mut Q1EntityServices) -> ActorId {
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
        game.set_health(&player, 100.0).expect("health");
        player
    }

    #[test]
    fn weapon_ranks_match_donor() {
        assert_eq!(hipnotic_weapon_rank(Q1Weapon::Lightning), 1);
        assert_eq!(hipnotic_weapon_rank(Q1Weapon::HipnoticLaser), 3);
        assert_eq!(hipnotic_weapon_rank(Q1Weapon::HipnoticMjolnir), 9);
        assert_eq!(hipnotic_weapon_rank(Q1Weapon::Axe), 10);
    }

    #[test]
    fn items_register_touch_and_spawns() {
        let mut game = test_game();
        register_mission_pack_items(&mut game, &MissionItemServices::new(Q1MissionPack::Hipnotic)).expect("register");
        assert!(game.named.touch("missionpack:item-touch").is_ok());
        assert!(game.named.action("missionpack:random-regen").is_ok());
    }

    #[test]
    fn wetsuit_pickup_grants_powerup() {
        let mut game = test_game();
        register_mission_pack_items(&mut game, &MissionItemServices::new(Q1MissionPack::Hipnotic)).expect("register");
        let player = attached_player(&mut game);
        let item = game.create("item_artifact_wetsuit", None, None).expect("item");
        game.update_entity(&item, |entity| entity.solid = Q1Solid::Trigger)
            .expect("trigger");
        pickup(
            &mut game,
            &item,
            &player,
            &MissionItemServices::new(Q1MissionPack::Hipnotic),
        )
        .expect("pickup");
        assert!(game
            .player_ref(&player)
            .expect("state")
            .powerups
            .contains_key(&Q1Powerup::HipnoticWetsuit));
        assert_eq!(game.entity_ref(&item).expect("item").solid, Q1Solid::None);
    }
}
