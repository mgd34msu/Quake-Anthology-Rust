//! Monster missiles and backpacks (`src/content/q1/base/projectiles.ts`).
//!
//! Monster missile and backpack QuakeC. Copyright (C) 1996-2022 id
//! Software LLC. GPL-2.0-or-later.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::contract::{ItemId, PickupCargoEntry, PickupCargoKind, PickupSelection};
use crate::q1::base::monsters::BaseMonster;
use crate::q1::foundation::entity::Q1ProjectileKind;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{
    length, normalize, vadd, vscale, vsub, weapon_item, yaw_for, Q1AutoSwitch, Q1BeamStyle, Q1Edition, Q1Effect,
    Q1Event, Q1MessageArg, Q1MessagePart, Q1MoveType, Q1Solid, Q1SoundChannel, Q1TraceRequest, Q1Weapon, POINT, ZERO,
};
use crate::q1::Q1Error;

pub use crate::q1::foundation::monsters::{throw_gib, throw_head};

/// Backpack weapon selection (`BackpackContents["selection"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackpackSelection {
    /// Source default.
    SourceDefault,
    /// Rank-based selection.
    Rank,
}

impl BackpackSelection {
    /// Donor selection text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            BackpackSelection::SourceDefault => "source-default",
            BackpackSelection::Rank => "rank",
        }
    }
}

/// Extra backpack cargo (`BackpackContents["extra"]` entry).
#[derive(Debug, Clone, PartialEq)]
pub struct BackpackExtra {
    /// Item id.
    pub item: ItemId,
    /// Count.
    pub count: f64,
}

/// Backpack drop request (`BackpackContents`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BackpackDrop {
    /// Dropped weapon, if any.
    pub weapon: Option<Q1Weapon>,
    /// Shells.
    pub shells: f64,
    /// Nails.
    pub nails: f64,
    /// Rockets.
    pub rockets: f64,
    /// Cells.
    pub cells: f64,
    /// Extra cargo.
    pub extra: Vec<BackpackExtra>,
    /// Weapon selection override.
    pub selection: Option<BackpackSelection>,
    /// Underwater lightning avoidance override.
    pub avoid_underwater_lightning: Option<bool>,
    /// Owner pickup delay override.
    pub owner_pickup_delay: Option<f64>,
}

/// Stored backpack contents.
#[derive(Debug, Clone, PartialEq)]
pub struct BackpackContents {
    /// Dropped weapon, if any.
    pub weapon: Option<Q1Weapon>,
    /// Shells.
    pub shells: f64,
    /// Nails.
    pub nails: f64,
    /// Rockets.
    pub rockets: f64,
    /// Cells.
    pub cells: f64,
    /// Extra cargo.
    pub extra: Vec<BackpackExtra>,
    /// Weapon selection.
    pub selection: BackpackSelection,
    /// Underwater lightning avoidance.
    pub avoid_underwater_lightning: bool,
    /// Owner pickup delay.
    pub owner_pickup_delay: f64,
}

/// Backpack launch (`dropBackpack` launch).
#[derive(Debug, Clone, PartialEq)]
pub struct BackpackLaunch {
    /// Launch origin.
    pub origin: Vec3,
    /// Launch velocity.
    pub velocity: Vec3,
    /// Launch movement.
    pub movement: Q1MoveType,
}

/// Backpack pickup message (`backpackMessage` return).
#[derive(Debug, Clone, PartialEq)]
pub struct BackpackMessage {
    /// Message text.
    pub text: String,
    /// Message parts.
    pub parts: Vec<Q1MessagePart>,
}

/// Spike kind (`launchSpike` kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpikeKind {
    /// Spike.
    Spike,
    /// Superspike.
    Superspike,
    /// Wizard spike.
    Wizard,
    /// Knight spike.
    Knight,
}

fn backpack_weapon_name(weapon: Q1Weapon) -> Option<(&'static str, &'static str)> {
    match weapon {
        Q1Weapon::Axe => Some(("Axe", "$qc_axe")),
        Q1Weapon::Shotgun => Some(("Shotgun", "$qc_shotgun")),
        Q1Weapon::Supershotgun => Some(("Double-barrelled Shotgun", "$qc_double_shotgun")),
        Q1Weapon::Nailgun => Some(("Nailgun", "$qc_nailgun")),
        Q1Weapon::Supernailgun => Some(("Super Nailgun", "$qc_super_nailgun")),
        Q1Weapon::Grenadelauncher => Some(("Grenade Launcher", "$qc_grenade_launcher")),
        Q1Weapon::Rocketlauncher => Some(("Rocket Launcher", "$qc_rocket_launcher")),
        Q1Weapon::Lightning => Some(("Thunderbolt", "$qc_thunderbolt")),
        Q1Weapon::HipnoticLaser => Some(("Laser Cannon", "$qc_laser_cannon")),
        Q1Weapon::HipnoticProximity => Some(("Proximity Gun", "$qc_prox_gun")),
        Q1Weapon::HipnoticMjolnir => Some(("Mjolnir", "$qc_mjolnir")),
        _ => None,
    }
}

/// Compose one pickup notice from its newly acquired weapon and
/// backpack ammunition (`backpackMessage`).
#[must_use]
pub fn backpack_message(contents: &BackpackContents, edition: Q1Edition, new_weapon: bool) -> BackpackMessage {
    let mut entries: Vec<(&str, &str, f64)> = vec![
        ("shells", "$qc_backpack_shells", contents.shells),
        ("nails", "$qc_backpack_nails", contents.nails),
        ("rockets", "$qc_backpack_rockets", contents.rockets),
        ("cells", "$qc_backpack_cells", contents.cells),
    ];
    for extra in &contents.extra {
        if extra.item == "rogue:ammo/lava-nails" {
            entries.push(("lava nails", "$qc_backpack_lava_nails", extra.count));
        } else if extra.item == "rogue:ammo/multi-rockets" {
            entries.push(("multi rockets", "$qc_backpack_multi_rockets", extra.count));
        } else if extra.item == "rogue:ammo/plasma" {
            entries.push(("plasma balls", "$qc_backpack_plasma_balls", extra.count));
        }
    }
    let weapon = if new_weapon {
        contents.weapon.and_then(backpack_weapon_name)
    } else {
        None
    };
    if edition == Q1Edition::Classic {
        let mut classic: Vec<String> = weapon.map(|(name, _)| format!("the {name}")).into_iter().collect();
        classic.extend(
            entries
                .iter()
                .filter(|(_, _, count)| *count > 0.0)
                .map(|(name, _, count)| format!("{count} {name}")),
        );
        return BackpackMessage {
            text: format!("You get {}", classic.join(", ")),
            parts: Vec::new(),
        };
    }
    let mut parts = vec![Q1MessagePart {
        text: String::from("$qc_backpack_got"),
        args: None,
    }];
    let mut items: Vec<Q1MessagePart> = weapon
        .map(|(_, localized)| Q1MessagePart {
            text: localized.to_string(),
            args: None,
        })
        .into_iter()
        .collect();
    items.extend(
        entries
            .iter()
            .filter(|(_, _, count)| *count > 0.0)
            .map(|(_, localized, count)| Q1MessagePart {
                text: (*localized).to_string(),
                args: Some(vec![Q1MessageArg::Number(*count)]),
            }),
    );
    for (index, item) in items.into_iter().enumerate() {
        if index > 0 {
            parts.push(Q1MessagePart {
                text: String::from(", "),
                args: None,
            });
        }
        parts.push(item);
    }
    BackpackMessage {
        text: String::from("$qc_backpack_got"),
        parts,
    }
}

/// Drop a backpack (`dropBackpack`). Empty drops return no pack.
pub fn drop_backpack(
    game: &mut Q1EntityServices,
    origin: Vec3,
    contents: &BackpackDrop,
    launch: Option<&BackpackLaunch>,
) -> Result<Option<ActorId>, Q1Error> {
    let total = contents.shells
        + contents.nails
        + contents.rockets
        + contents.cells
        + contents.extra.iter().map(|entry| entry.count).sum::<f64>();
    if total == 0.0 {
        return Ok(None);
    }
    let pack = game.create("item_backpack", None, None)?;
    let movement = launch.map(|launch| launch.movement).unwrap_or(Q1MoveType::Toss);
    game.update_entity(&pack, |entity| {
        entity.model = String::from("progs/backpack.mdl");
        entity.solid = Q1Solid::Trigger;
        entity.movement = movement;
    })?;
    let rerelease = game.options().edition == Q1Edition::Rerelease;
    let weapon = contents.weapon;
    super::creatures::set_backpack_contents(
        game,
        &pack,
        BackpackContents {
            weapon,
            extra: contents.extra.clone(),
            selection: contents.selection.unwrap_or(BackpackSelection::SourceDefault),
            avoid_underwater_lightning: contents.avoid_underwater_lightning.unwrap_or(rerelease),
            owner_pickup_delay: contents.owner_pickup_delay.unwrap_or(0.0),
            shells: contents.shells.max(
                if rerelease && matches!(weapon, Some(Q1Weapon::Shotgun | Q1Weapon::Supershotgun)) {
                    5.0
                } else {
                    0.0
                },
            ),
            nails: contents.nails.max(
                if rerelease && matches!(weapon, Some(Q1Weapon::Nailgun | Q1Weapon::Supernailgun)) {
                    20.0
                } else {
                    0.0
                },
            ),
            rockets: contents.rockets.max(
                if rerelease && matches!(weapon, Some(Q1Weapon::Rocketlauncher | Q1Weapon::Grenadelauncher)) {
                    5.0
                } else {
                    0.0
                },
            ),
            cells: contents.cells.max(if rerelease && weapon == Some(Q1Weapon::Lightning) {
                15.0
            } else {
                0.0
            }),
        },
    )?;
    let at = launch.map(|launch| launch.origin).unwrap_or_else(|| {
        vadd(
            origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: -24.0,
            },
        )
    });
    let velocity = launch.map(|launch| launch.velocity).unwrap_or_else(|| Vec3 {
        x: -100.0 + game.host.random() as f32 * 200.0,
        y: -100.0 + game.host.random() as f32 * 200.0,
        z: 300.0,
    });
    game.set_body(
        &pack,
        &BodyPatch {
            origin: Some(at),
            velocity: Some(velocity),
            bounds: Some(Bounds {
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
            }),
            ..Default::default()
        },
    )?;
    let touch = game.named.touch("base:backpack_touch")?;
    game.update_entity(&pack, |entity| entity.touch = Some(touch))?;
    game.schedule(&pack, 120.0, "SUB_Remove")?;
    game.link(&pack)?;
    Ok(Some(pack))
}

fn backpack_feedback(
    game: &mut Q1EntityServices,
    pack: &ActorId,
    other: &ActorId,
    actor: &qa_core::identity::OwnedActor,
    contents: &BackpackContents,
    new_weapon: bool,
) -> Result<(), Q1Error> {
    let message = backpack_message(contents, game.options().edition, new_weapon);
    game.host.emit(Q1Event::Message {
        player: other.clone(),
        text: message.text,
        center: false,
        args: None,
        parts: if message.parts.is_empty() {
            None
        } else {
            Some(message.parts)
        },
    });
    game.sound(actor.id(), "weapons/lock4.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
    let origin = game.body(pack).map(|body| body.origin)?;
    game.effect(Q1Effect::Pickup, origin, Some(other), 1);
    Ok(())
}

fn base_weapon_rank(weapon: Q1Weapon) -> i32 {
    match weapon {
        Q1Weapon::Lightning => 1,
        Q1Weapon::Rocketlauncher => 2,
        Q1Weapon::Supernailgun => 3,
        Q1Weapon::Grenadelauncher => 4,
        Q1Weapon::Supershotgun => 5,
        Q1Weapon::Nailgun => 6,
        _ => 7,
    }
}

fn backpack_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let (pack, other) = (id.clone(), other.clone());
    let contents = super::creatures::backpack_contents(game, &pack)?;
    let weapon = contents.weapon;
    let mut ammo: Vec<(ItemId, f64)> = vec![
        (ItemId::from("q1:ammo/shells"), contents.shells),
        (ItemId::from("q1:ammo/nails"), contents.nails),
        (ItemId::from("q1:ammo/rockets"), contents.rockets),
        (ItemId::from("q1:ammo/cells"), contents.cells),
    ];
    ammo.extend(contents.extra.iter().map(|entry| (entry.item.clone(), entry.count)));
    let player = game.player_ref(&other).cloned();
    let actor = game.host.actors.resolve_owned(&other);
    let Some(actor) = actor else { return Ok(()) };
    if !game.is_player(&other) || game.health(&other) <= 0.0 || !game.is_live(&pack) {
        return Ok(());
    }
    let owner = game.entity_ref(&pack).and_then(|entity| entity.owner.clone());
    let next_think = game.entity_ref(&pack).map(|entity| entity.next_think).unwrap_or(0.0);
    if owner.as_ref().is_some_and(|owner| same_actor(&other, owner))
        && next_think - game.time > 120.0 - contents.owner_pickup_delay
    {
        return Ok(());
    }
    let new_weapon = match weapon {
        Some(weapon) => {
            let item = weapon_item(weapon);
            let owned = match game.pickup_admission.as_ref() {
                Some(admission) => admission.owns(&other, &item),
                None => game.host.inventory.count(&other, &item) > 0.0,
            };
            !owned
        }
        None => false,
    };
    if game.pickup_admission.is_some() {
        let mut cargo: Vec<PickupCargoEntry> = ammo
            .iter()
            .map(|(item, count)| PickupCargoEntry {
                kind: PickupCargoKind::Counter,
                item: item.clone(),
                count: *count,
            })
            .collect();
        let mut selection = PickupSelection::Never;
        if let Some(weapon) = weapon {
            let item = weapon_item(weapon);
            cargo.push(PickupCargoEntry {
                kind: PickupCargoKind::Weapon,
                item: item.clone(),
                count: 1.0,
            });
            let owned = game
                .pickup_admission
                .as_ref()
                .is_some_and(|admission| admission.owns(&other, &item));
            let auto_switch_hook = game.pickup_rules.as_ref().and_then(|rules| rules.auto_switch);
            let auto_switch = match &player {
                None => true,
                Some(player) => match auto_switch_hook {
                    Some(hook) => hook(game, &other, owned)?,
                    None => {
                        game.options().edition == Q1Edition::Classic
                            || player.auto_switch == Q1AutoSwitch::Always
                            || player.auto_switch == Q1AutoSwitch::New && !owned
                    }
                },
            };
            let always = contents.selection != BackpackSelection::Rank
                && game.options().edition == Q1Edition::Classic
                && game.options().deathmatch == 0;
            let underwater = !always
                && player.is_some()
                && contents.avoid_underwater_lightning
                && player.as_ref().is_some_and(|player| player.water_level != 0)
                && weapon == Q1Weapon::Lightning;
            selection = if !auto_switch || underwater {
                PickupSelection::Never
            } else if always {
                PickupSelection::Always
            } else {
                PickupSelection::Better
            };
        }
        if let Some(admission) = game.pickup_admission.as_ref() {
            admission.cargo(&actor, &cargo, selection);
        }
        if !game.is_live(&pack) || game.host.actors.resolve_owned(&other) != Some(actor.clone()) {
            return Ok(());
        }
        backpack_feedback(game, &pack, &other, &actor, &contents, new_weapon)?;
        if game.is_live(&pack) {
            game.remove(&pack)?;
        }
        return Ok(());
    }
    let Some(player) = player else { return Ok(()) };
    let had_weapon = match weapon {
        Some(weapon) => game.host.inventory.count(&other, &weapon_item(weapon)) > 0.0,
        None => true,
    };
    for (item, count) in &ammo {
        game.host.inventory.give(&player.actor, item, *count);
    }
    if let Some(weapon) = weapon {
        game.host.inventory.give(&player.actor, &weapon_item(weapon), 1.0);
    }
    let selected = weapon.unwrap_or(player.weapon);
    let granted_hook = game.pickup_rules.as_ref().and_then(|rules| rules.weapon_granted);
    if let Some(hook) = granted_hook {
        hook(game, &other, selected)?;
    }
    backpack_feedback(game, &pack, &other, &actor, &contents, new_weapon)?;
    let switch_hook = game.pickup_rules.as_ref().and_then(|rules| rules.auto_switch);
    let switch = match switch_hook {
        Some(hook) => hook(game, &other, had_weapon)?,
        None => {
            game.options().edition == Q1Edition::Classic
                || player.auto_switch == Q1AutoSwitch::Always
                || player.auto_switch == Q1AutoSwitch::New && !had_weapon
        }
    };
    if switch {
        let rank_hook = game.pickup_rules.as_ref().and_then(|rules| rules.weapon_rank);
        let rank = |weapon: Q1Weapon| {
            rank_hook
                .map(|hook| hook(weapon))
                .unwrap_or_else(|| base_weapon_rank(weapon))
        };
        let always = contents.selection != BackpackSelection::Rank
            && game.options().edition == Q1Edition::Classic
            && game.options().deathmatch == 0;
        if always
            || rank(selected) < rank(player.weapon)
                && (!contents.avoid_underwater_lightning || player.water_level == 0 || selected != Q1Weapon::Lightning)
        {
            game.select_weapon(&player.actor, selected)?;
        }
    }
    game.remove(&pack)
}

/// Spawn a meat spray (`spawnMeatSpray`).
pub fn spawn_meat_spray(
    game: &mut Q1EntityServices,
    owner: &ActorId,
    origin: Vec3,
    velocity: Vec3,
) -> Result<ActorId, Q1Error> {
    let owner = owner.clone();
    let missile = game.create("meat_spray", None, None)?;
    let angles = game.body(&owner).map(|body| body.angles)?;
    game.update_entity(&missile, |entity| {
        entity.owner = Some(owner);
        entity.movement = Q1MoveType::Bounce;
        entity.solid = Q1Solid::None;
    })?;
    game.make_vectors(if game.options().edition == Q1Edition::Rerelease {
        Vec3 {
            x: -angles.x,
            y: angles.y,
            z: angles.z,
        }
    } else {
        angles
    });
    let roll = game.host.random() as f32;
    game.set_body(
        &missile,
        &BodyPatch {
            origin: Some(origin),
            velocity: Some(Vec3 {
                x: velocity.x,
                y: velocity.y,
                z: velocity.z + 250.0 + 50.0 * roll,
            }),
            bounds: Some(POINT),
            ..Default::default()
        },
    )?;
    game.update_entity(&missile, |entity| {
        entity.angular_velocity = Vec3 {
            x: 3000.0,
            y: 1000.0,
            z: 2000.0,
        };
        entity.model = String::from("progs/zom_gib.mdl");
    })?;
    game.schedule(&missile, 1.0, "SUB_Remove")?;
    game.link(&missile)?;
    Ok(missile)
}

/// Create a missile (`createMissile`).
pub fn create_missile(
    game: &mut Q1EntityServices,
    owner: Option<&ActorId>,
    classname: &str,
    model: &str,
    origin: Vec3,
    velocity: Vec3,
    lifetime: f64,
) -> Result<ActorId, Q1Error> {
    let missile = game.create(classname, None, None)?;
    let owner = owner.cloned();
    let model = format!("progs/{model}.mdl");
    game.update_entity(&missile, |entity| {
        entity.owner = owner;
        entity.model = model;
        entity.solid = Q1Solid::Bbox;
        entity.movement = Q1MoveType::Flymissile;
    })?;
    game.set_body(
        &missile,
        &BodyPatch {
            origin: Some(origin),
            velocity: Some(velocity),
            bounds: Some(POINT),
            angles: Some(Vec3 {
                x: (f64::from(velocity.z).atan2(f64::from(velocity.x).hypot(f64::from(velocity.y))) * 180.0
                    / std::f64::consts::PI) as f32,
                y: yaw_for(velocity) as f32,
                z: 0.0,
            }),
            ..Default::default()
        },
    )?;
    game.schedule(&missile, lifetime, "SUB_Remove")?;
    game.link(&missile)?;
    Ok(missile)
}

/// Launch a spike (`launchSpike`).
pub fn launch_spike(
    game: &mut Q1EntityServices,
    owner: Option<&ActorId>,
    origin: Vec3,
    velocity: Vec3,
    kind: SpikeKind,
) -> Result<ActorId, Q1Error> {
    let (classname, model) = match kind {
        SpikeKind::Wizard => ("wizard_spike", "w_spike"),
        SpikeKind::Knight => ("knight_spike", "k_spike"),
        SpikeKind::Spike => ("spike", "spike"),
        SpikeKind::Superspike => ("superspike", "spike"),
    };
    let missile = create_missile(game, owner, classname, model, origin, velocity, 6.0)?;
    let projectile = if kind == SpikeKind::Superspike {
        Q1ProjectileKind::Superspike
    } else {
        Q1ProjectileKind::Spike
    };
    let touch = game.named.touch("projectile_touch")?;
    game.update_entity(&missile, |entity| {
        entity.projectile = Some(projectile);
        entity.touch = Some(touch);
    })?;
    Ok(missile)
}

/// Launch an enforcer laser (`launchLaser`).
pub fn launch_laser(
    game: &mut Q1EntityServices,
    owner: Option<&ActorId>,
    origin: Vec3,
    direction: Vec3,
) -> Result<ActorId, Q1Error> {
    let missile = create_missile(
        game,
        owner,
        "enforcer_laser",
        "laser",
        origin,
        vscale(normalize(direction), 600.0),
        5.0,
    )?;
    let touch = game.named.touch("base:laser_touch")?;
    game.update_entity(&missile, |entity| {
        entity.effects = 8;
        entity.touch = Some(touch);
    })?;
    Ok(missile)
}

fn laser_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let (missile, other) = (id.clone(), other.clone());
    let owner = game.entity_ref(&missile).and_then(|entity| entity.owner.clone());
    if owner.as_ref().is_some_and(|owner| same_actor(&other, owner)) {
        return Ok(());
    }
    let body = game.body(&missile)?;
    if game.host.contents(body.origin) == crate::q1::foundation::host::Q1Contents::Sky {
        return game.remove(&missile);
    }
    game.sound(&missile, "enforcer/enfstop.wav", Q1SoundChannel::Weapon, 3.0, 1.0)?;
    let hit = vsub(body.origin, vscale(normalize(body.velocity), 8.0));
    if game.health(&other) != 0.0 {
        game.effect(Q1Effect::Blood, hit, Some(&other), 15);
        game.damage_direct(&other, Some(&missile), owner.as_ref(), 15.0);
    } else {
        game.effect(Q1Effect::Gunshot, hit, None, 1);
    }
    game.remove(&missile)
}

/// Turn a missile into an explosion sprite (`spriteExplosion`).
pub fn sprite_explosion(game: &mut Q1EntityServices, missile: &ActorId) -> Result<(), Q1Error> {
    let missile = missile.clone();
    let origin = game.body(&missile).map(|body| body.origin)?;
    game.effect(Q1Effect::Explosion, origin, None, 1);
    game.update_entity(&missile, |entity| {
        entity.touch = None;
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
        entity.model = String::from("progs/s_explod.spr");
        entity.frame = 0;
    })?;
    game.set_body(
        &missile,
        &BodyPatch {
            velocity: Some(ZERO),
            ..Default::default()
        },
    )?;
    game.link(&missile)?;
    game.schedule(&missile, 0.1, "base:explosion_frame")
}

/// Launch an ogre grenade (`launchOgreGrenade`).
pub fn launch_ogre_grenade(monster: &mut BaseMonster) -> Result<(), Q1Error> {
    let Some(target) = monster.target()? else { return Ok(()) };
    let origin = monster.origin()?;
    monster
        .game
        .effect(Q1Effect::Muzzleflash, origin, Some(&monster.id.clone()), 1);
    monster.game.sound(
        &monster.id.clone(),
        "weapons/grenade.wav",
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    monster.make_vectors()?;
    let direction = normalize(vsub(target, origin));
    let missile = create_missile(
        monster.game,
        Some(&monster.id.clone()),
        "ogre_grenade",
        "grenade",
        origin,
        Vec3 {
            x: direction.x * 600.0,
            y: direction.y * 600.0,
            z: 200.0,
        },
        2.5,
    )?;
    monster.game.update_entity(&missile, |entity| {
        entity.movement = Q1MoveType::Bounce;
        entity.angular_velocity = Vec3 {
            x: 300.0,
            y: 300.0,
            z: 300.0,
        };
    })?;
    let touch = monster.game.named.touch("base:ogre_grenade_touch")?;
    monster
        .game
        .update_entity(&missile, |entity| entity.touch = Some(touch))?;
    monster.game.schedule(&missile, 2.5, "base:ogre_grenade_explode")
}

fn ogre_grenade_explode(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let missile = id.clone();
    let owner = game.entity_ref(&missile).and_then(|entity| entity.owner.clone());
    game.radius_damage(&missile, owner.as_ref(), 40.0, None, None, "");
    game.sound_simple(&missile, "weapons/r_exp3.wav")?;
    sprite_explosion(game, &missile)
}

fn ogre_grenade_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let (missile, other) = (id.clone(), other.clone());
    if game
        .entity_ref(&missile)
        .and_then(|entity| entity.owner.clone())
        .as_ref()
        .is_some_and(|owner| same_actor(&other, owner))
    {
        return Ok(());
    }
    if game.is_player(&other) || game.entity_ref(&other).is_some_and(|entity| entity.aimed_damage) {
        return ogre_grenade_explode(game, &missile);
    }
    game.sound_simple(&missile, "weapons/bounce.wav")?;
    if length(game.body(&missile).map(|body| body.velocity)?) == 0.0 {
        game.update_entity(&missile, |entity| entity.angular_velocity = ZERO)?;
    }
    Ok(())
}

/// Launch a zombie grenade (`launchZombieGrenade`).
pub fn launch_zombie_grenade(monster: &mut BaseMonster, offset: Vec3) -> Result<(), Q1Error> {
    let Some(target) = monster.target()? else { return Ok(()) };
    let axes = monster.game.basis;
    let origin = monster.origin()?;
    let at = vadd(
        origin,
        vadd(
            vscale(axes.forward, f64::from(offset.x)),
            vadd(
                vscale(axes.right, f64::from(offset.y)),
                vscale(axes.up, f64::from(offset.z) - 24.0),
            ),
        ),
    );
    monster.game.sound(
        &monster.id.clone(),
        "zombie/z_shot1.wav",
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    monster.make_vectors()?;
    let direction = normalize(vsub(target, at));
    let missile = create_missile(
        monster.game,
        Some(&monster.id.clone()),
        "zombie_grenade",
        "zom_gib",
        at,
        Vec3 {
            x: direction.x * 600.0,
            y: direction.y * 600.0,
            z: 200.0,
        },
        2.5,
    )?;
    let touch = monster.game.named.touch("base:zombie_grenade_touch")?;
    monster.game.update_entity(&missile, |entity| {
        entity.movement = Q1MoveType::Bounce;
        entity.angular_velocity = Vec3 {
            x: 3000.0,
            y: 1000.0,
            z: 2000.0,
        };
        entity.touch = Some(touch);
    })
}

fn zombie_grenade_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let (missile, other) = (id.clone(), other.clone());
    if game
        .entity_ref(&missile)
        .and_then(|entity| entity.owner.clone())
        .as_ref()
        .is_some_and(|owner| same_actor(&other, owner))
    {
        return Ok(());
    }
    if game.host.combat.read(&other).is_some_and(|state| state.can_take_damage) {
        let owner = game.entity_ref(&missile).and_then(|entity| entity.owner.clone());
        game.damage_direct(&other, Some(&missile), owner.as_ref(), 10.0);
        game.sound(&missile, "zombie/z_hit.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        return game.remove(&missile);
    }
    game.sound(&missile, "zombie/z_miss.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    game.set_body(
        &missile,
        &BodyPatch {
            velocity: Some(ZERO),
            ..Default::default()
        },
    )?;
    let touch = game.named.touch("base:remove_touch")?;
    game.update_entity(&missile, |entity| {
        entity.angular_velocity = ZERO;
        entity.touch = Some(touch);
    })
}

/// Launch a vore ball (`launchVoreBall`).
pub fn launch_vore_ball(monster: &mut BaseMonster) -> Result<(), Q1Error> {
    let target = monster.target()?;
    let enemy = monster.monster.enemy.clone();
    let (Some(target), Some(enemy)) = (target, enemy) else {
        return Ok(());
    };
    let origin = monster.origin()?;
    let direction = normalize(vsub(
        vadd(
            target,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 10.0,
            },
        ),
        origin,
    ));
    let missile = create_missile(
        monster.game,
        Some(&monster.id.clone()),
        "vore_ball",
        "v_spike",
        vadd(
            origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 10.0,
            },
        ),
        vscale(direction, 400.0),
        5.0,
    )?;
    monster.game.update_entity(&missile, |entity| {
        entity.angular_velocity = Vec3 {
            x: 300.0,
            y: 300.0,
            z: 300.0,
        };
    })?;
    monster
        .game
        .effect(Q1Effect::Muzzleflash, origin, Some(&monster.id.clone()), 1);
    monster.game.sound(
        &monster.id.clone(),
        "shalrath/attack2.wav",
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    super::creatures::set_projectile_target(monster.game, &missile, &enemy)?;
    let touch = monster.game.named.touch("base:vore_touch")?;
    monster
        .game
        .update_entity(&missile, |entity| entity.touch = Some(touch))?;
    let delay = (monster.distance()? * 0.002).max(0.1);
    monster.game.schedule(&missile, delay, "base:vore_home")
}

fn vore_home(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let missile = id.clone();
    let enemy = super::creatures::projectile_target(game, &missile)?;
    let Some(body) = game.host.bodies.read(&enemy) else {
        return game.remove(&missile);
    };
    if game.health(&enemy) < 1.0 {
        return game.remove(&missile);
    }
    let speed = if game.options().edition == Q1Edition::Classic && game.options().skill == 3 {
        350.0
    } else {
        250.0
    };
    let origin = game.body(&missile).map(|body| body.origin)?;
    game.set_body(
        &missile,
        &BodyPatch {
            velocity: Some(vscale(
                normalize(vsub(
                    vadd(
                        body.origin,
                        Vec3 {
                            x: 0.0,
                            y: 0.0,
                            z: 10.0,
                        },
                    ),
                    origin,
                )),
                speed,
            )),
            ..Default::default()
        },
    )?;
    game.schedule(&missile, 0.2, "base:vore_home")
}

fn vore_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let (missile, other) = (id.clone(), other.clone());
    if game
        .entity_ref(&missile)
        .and_then(|entity| entity.owner.clone())
        .as_ref()
        .is_some_and(|owner| same_actor(&other, owner))
    {
        return Ok(());
    }
    if game.host.classname(&other) == "monster_zombie" {
        game.damage_direct(&other, Some(&missile.clone()), Some(&missile.clone()), 110.0);
    }
    let owner = game.entity_ref(&missile).and_then(|entity| entity.owner.clone());
    game.radius_damage(&missile, owner.as_ref(), 40.0, None, None, "");
    game.sound(&missile, "weapons/r_exp3.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    sprite_explosion(game, &missile)
}

fn remove_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    game.remove(id)
}

fn explosion_frame(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    game.update_entity(&id, |entity| entity.frame += 1)?;
    if game.entity_ref(&id).map(|entity| entity.frame).unwrap_or(0) >= 6 {
        return game.remove(&id);
    }
    game.schedule(&id, 0.1, "base:explosion_frame")
}

/// Register projectile callbacks (`registerProjectileCallbacks`).
pub fn register_projectile_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1TouchHandler};
    let touch = |handler: Q1TouchHandler| Q1CallbackHandlers {
        touch: Some(handler),
        ..Default::default()
    };
    let action = |handler: Q1ActionHandler| Q1CallbackHandlers {
        action: Some(handler),
        ..Default::default()
    };
    game.named.register("base:backpack_touch", touch(backpack_touch))?;
    game.named.register("base:laser_touch", touch(laser_touch))?;
    game.named
        .register("base:ogre_grenade_touch", touch(ogre_grenade_touch))?;
    game.named
        .register("base:ogre_grenade_explode", action(ogre_grenade_explode))?;
    game.named
        .register("base:zombie_grenade_touch", touch(zombie_grenade_touch))?;
    game.named.register("base:remove_touch", touch(remove_touch))?;
    game.named.register("base:vore_touch", touch(vore_touch))?;
    game.named.register("base:vore_home", action(vore_home))?;
    game.named.register("base:explosion_frame", action(explosion_frame))?;
    Ok(())
}

/// Cast Shambler lightning (`castLightning`).
pub fn cast_lightning(monster: &mut BaseMonster) -> Result<(), Q1Error> {
    let Some(target) = monster.target()? else { return Ok(()) };
    monster.face()?;
    let origin = monster.origin()?;
    monster
        .game
        .effect(Q1Effect::Muzzleflash, origin, Some(&monster.id.clone()), 1);
    monster.controller.lightning_count += 1.0;
    let start = vadd(
        origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 40.0,
        },
    );
    let direction = normalize(vsub(
        vadd(
            target,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 16.0,
            },
        ),
        start,
    ));
    let wall = monster.game.host.trace(&Q1TraceRequest {
        start,
        end: vadd(origin, vscale(direction, 600.0)),
        bounds: POINT,
        ignore: Some(monster.id.clone()),
        monsters: false,
        missile: false,
    });
    monster.game.host.emit(Q1Event::Beam {
        style: Q1BeamStyle::Lightning1,
        actor: monster.id.clone(),
        start,
        end: wall.end,
    });
    let delta = vsub(wall.end, start);
    let side = Vec3 {
        x: -delta.y * 16.0,
        y: -delta.y * 16.0,
        z: 0.0,
    };
    let mut hit: Vec<ActorId> = Vec::new();
    for offset in [ZERO, side, vscale(side, -1.0)] {
        let trace = monster.game.host.trace(&Q1TraceRequest {
            start: vadd(start, offset),
            end: vadd(wall.end, offset),
            bounds: POINT,
            ignore: Some(monster.id.clone()),
            monsters: true,
            missile: false,
        });
        if let Some(target_actor) = trace.actor {
            if monster
                .game
                .host
                .combat
                .read(&target_actor)
                .is_some_and(|state| state.can_take_damage)
                && !hit.iter().any(|id| same_actor(id, &target_actor))
            {
                hit.push(target_actor.clone());
                monster.game.damage_direct(
                    &target_actor,
                    Some(&monster.id.clone()),
                    Some(&monster.id.clone()),
                    10.0,
                );
                monster.game.effect(Q1Effect::Blood, trace.end, Some(&target_actor), 40);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::*;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::foundation::entity_services::Q1AttachOptions;
    use crate::q1::foundation::host::mock::mock_host;
    use crate::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram, Q1Weapon};

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
    fn backpack_message_formats_editions() {
        let contents = BackpackContents {
            weapon: Some(Q1Weapon::Shotgun),
            shells: 5.0,
            nails: 0.0,
            rockets: 0.0,
            cells: 0.0,
            extra: Vec::new(),
            selection: BackpackSelection::SourceDefault,
            avoid_underwater_lightning: false,
            owner_pickup_delay: 0.0,
        };
        let classic = backpack_message(&contents, Q1Edition::Classic, true);
        assert_eq!(classic.text, "You get the Shotgun, 5 shells");
        assert!(classic.parts.is_empty());
        let rerelease = backpack_message(&contents, Q1Edition::Rerelease, true);
        assert_eq!(rerelease.text, "$qc_backpack_got");
        assert_eq!(rerelease.parts.len(), 4);
    }

    #[test]
    fn backpack_drop_and_touch_flow() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
        game.set_health(&player, 100.0).expect("health");
        let pack = drop_backpack(
            &mut game,
            ZERO,
            &BackpackDrop {
                weapon: Some(Q1Weapon::Nailgun),
                nails: 30.0,
                ..Default::default()
            },
            None,
        )
        .expect("drop")
        .expect("pack");
        game.invoke_touch(&pack, &player, None, None).expect("touch");
        assert!(game.entity_ref(&pack).is_none());
        assert_eq!(game.host.inventory.count(&player, &ItemId::from("q1:ammo/nails")), 30.0);
        assert_eq!(
            game.player_ref(&player).map(|player| player.weapon),
            Some(Q1Weapon::Nailgun)
        );
        let empty = drop_backpack(&mut game, ZERO, &BackpackDrop::default(), None).expect("empty");
        assert!(empty.is_none());
    }

    #[test]
    fn spikes_vore_and_explosions() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let spike = launch_spike(
            &mut game,
            None,
            ZERO,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 100.0,
            },
            SpikeKind::Wizard,
        )
        .expect("spike");
        let entity = game.entity_ref(&spike).cloned().expect("missile");
        assert_eq!(entity.classname, "wizard_spike");
        assert_eq!(entity.projectile, Some(Q1ProjectileKind::Spike));
        sprite_explosion(&mut game, &spike).expect("explode");
        assert_eq!(
            game.entity_ref(&spike).map(|entity| entity.model.clone()),
            Some(String::from("progs/s_explod.spr"))
        );
        for _ in 0..6 {
            let _ = game.invoke_action(&spike, "base:explosion_frame");
        }
        assert!(game.entity_ref(&spike).is_none());
        let gib = throw_gib(&mut game, ZERO, "gib1", -60.0).expect("gib");
        assert_eq!(
            game.entity_ref(&gib).map(|entity| entity.classname.clone()),
            Some(String::from("gib"))
        );
    }
}
