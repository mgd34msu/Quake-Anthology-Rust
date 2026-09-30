//! Q1 mission-pack shared types (`src/content/q1/missionpacks/types.ts`).

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{normalize, vadd, vscale, Q1Powerup, Q1Weapon};
use crate::q1::Q1Error;

/// Round to binary32 storage (donor `Math.fround`).
#[must_use]
pub(crate) fn fround(value: f64) -> f64 {
    f64::from(qa_core::numeric::store_f32(value))
}

/// Mission pack id (`Q1MissionPack`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1MissionPack {
    /// Hipnotic expansion.
    Hipnotic,
    /// Rogue expansion.
    Rogue,
}

impl Q1MissionPack {
    /// Donor id text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Q1MissionPack::Hipnotic => "hipnotic",
            Q1MissionPack::Rogue => "rogue",
        }
    }
}

/// Mission-pack weapon id (`MissionWeapon`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MissionWeapon {
    /// Hipnotic laser cannon.
    HipnoticLaser,
    /// Hipnotic mjolnir.
    HipnoticMjolnir,
    /// Hipnotic proximity gun.
    HipnoticProximity,
    /// Rogue lava nailgun.
    RogueLavaNailgun,
    /// Rogue lava super nailgun.
    RogueLavaSupernailgun,
    /// Rogue multi grenade.
    RogueMultiGrenade,
    /// Rogue multi rocket.
    RogueMultiRocket,
    /// Rogue plasma.
    RoguePlasma,
}

impl MissionWeapon {
    /// Donor id text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            MissionWeapon::HipnoticLaser => "hipnotic:laser",
            MissionWeapon::HipnoticMjolnir => "hipnotic:mjolnir",
            MissionWeapon::HipnoticProximity => "hipnotic:proximity",
            MissionWeapon::RogueLavaNailgun => "rogue:lava-nailgun",
            MissionWeapon::RogueLavaSupernailgun => "rogue:lava-supernailgun",
            MissionWeapon::RogueMultiGrenade => "rogue:multi-grenade",
            MissionWeapon::RogueMultiRocket => "rogue:multi-rocket",
            MissionWeapon::RoguePlasma => "rogue:plasma",
        }
    }
}

impl From<MissionWeapon> for Q1Weapon {
    fn from(weapon: MissionWeapon) -> Self {
        match weapon {
            MissionWeapon::HipnoticLaser => Q1Weapon::HipnoticLaser,
            MissionWeapon::HipnoticMjolnir => Q1Weapon::HipnoticMjolnir,
            MissionWeapon::HipnoticProximity => Q1Weapon::HipnoticProximity,
            MissionWeapon::RogueLavaNailgun => Q1Weapon::RogueLavaNailgun,
            MissionWeapon::RogueLavaSupernailgun => Q1Weapon::RogueLavaSupernailgun,
            MissionWeapon::RogueMultiGrenade => Q1Weapon::RogueMultiGrenade,
            MissionWeapon::RogueMultiRocket => Q1Weapon::RogueMultiRocket,
            MissionWeapon::RoguePlasma => Q1Weapon::RoguePlasma,
        }
    }
}

/// Mission-pack powerup id (`MissionPowerup`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MissionPowerup {
    /// Hipnotic wetsuit.
    HipnoticWetsuit,
    /// Hipnotic empathy shield.
    HipnoticEmpathy,
    /// Rogue shield.
    RogueShield,
    /// Rogue antigrav.
    RogueAntigrav,
}

impl MissionPowerup {
    /// Donor id text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            MissionPowerup::HipnoticWetsuit => "hipnotic:wetsuit",
            MissionPowerup::HipnoticEmpathy => "hipnotic:empathy",
            MissionPowerup::RogueShield => "rogue:shield",
            MissionPowerup::RogueAntigrav => "rogue:antigrav",
        }
    }
}

impl From<MissionPowerup> for Q1Powerup {
    fn from(powerup: MissionPowerup) -> Self {
        match powerup {
            MissionPowerup::HipnoticWetsuit => Q1Powerup::HipnoticWetsuit,
            MissionPowerup::HipnoticEmpathy => Q1Powerup::HipnoticEmpathy,
            MissionPowerup::RogueShield => Q1Powerup::RogueShield,
            MissionPowerup::RogueAntigrav => Q1Powerup::RogueAntigrav,
        }
    }
}

/// Mission-pack weapon definition (`MissionWeaponDefinition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissionWeaponDefinition {
    /// Weapon id.
    pub id: MissionWeapon,
    /// View model path.
    pub model: &'static str,
    /// Pickup classname, when the weapon has its own pickup.
    pub pickup: Option<&'static str>,
    /// World model path.
    pub world_model: &'static str,
    /// Ammunition item id, when the weapon consumes ammunition.
    pub ammo: Option<&'static str>,
    /// Ammunition granted on pickup.
    pub pickup_ammo: i32,
    /// Selection rank.
    pub rank: i32,
}

/// Mission-pack weapon roster (`missionWeapons`).
pub const MISSION_WEAPONS: [MissionWeaponDefinition; 8] = [
    MissionWeaponDefinition {
        id: MissionWeapon::HipnoticLaser,
        model: "progs/v_laserg.mdl",
        pickup: Some("weapon_laser_gun"),
        world_model: "progs/g_laserg.mdl",
        ammo: Some("q1:ammo/cells"),
        pickup_ammo: 30,
        rank: 3,
    },
    MissionWeaponDefinition {
        id: MissionWeapon::HipnoticMjolnir,
        model: "progs/v_hammer.mdl",
        pickup: Some("weapon_mjolnir"),
        world_model: "progs/g_hammer.mdl",
        ammo: None,
        pickup_ammo: 30,
        rank: 9,
    },
    MissionWeaponDefinition {
        id: MissionWeapon::HipnoticProximity,
        model: "progs/v_prox.mdl",
        pickup: Some("weapon_proximity_gun"),
        world_model: "progs/g_prox.mdl",
        ammo: Some("q1:ammo/rockets"),
        pickup_ammo: 6,
        rank: 5,
    },
    MissionWeaponDefinition {
        id: MissionWeapon::RogueLavaNailgun,
        model: "progs/v_lava.mdl",
        pickup: None,
        world_model: "progs/g_nail.mdl",
        ammo: Some("rogue:ammo/lava-nails"),
        pickup_ammo: 0,
        rank: 4,
    },
    MissionWeaponDefinition {
        id: MissionWeapon::RogueLavaSupernailgun,
        model: "progs/v_lava2.mdl",
        pickup: None,
        world_model: "progs/g_nail2.mdl",
        ammo: Some("rogue:ammo/lava-nails"),
        pickup_ammo: 0,
        rank: 2,
    },
    MissionWeaponDefinition {
        id: MissionWeapon::RogueMultiGrenade,
        model: "progs/v_multi.mdl",
        pickup: None,
        world_model: "progs/g_rock.mdl",
        ammo: Some("rogue:ammo/multi-rockets"),
        pickup_ammo: 0,
        rank: 6,
    },
    MissionWeaponDefinition {
        id: MissionWeapon::RogueMultiRocket,
        model: "progs/v_multi2.mdl",
        pickup: None,
        world_model: "progs/g_rock2.mdl",
        ammo: Some("rogue:ammo/multi-rockets"),
        pickup_ammo: 0,
        rank: 1,
    },
    MissionWeaponDefinition {
        id: MissionWeapon::RoguePlasma,
        model: "progs/v_plasma.mdl",
        pickup: None,
        world_model: "progs/g_light.mdl",
        ammo: Some("rogue:ammo/plasma"),
        pickup_ammo: 0,
        rank: 0,
    },
];

/// Store an entity actor reference under a key (`setMissionReference`).
/// The foundation remaps these references when the session is restored.
pub fn set_mission_reference(
    game: &mut Q1EntityServices,
    id: &ActorId,
    key: &str,
    actor: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let actor = actor.cloned();
    game.update_entity(id, |entity| {
        entity.references.insert(key.to_string(), actor);
    })
}

/// Read a live entity actor reference (`missionReference`).
#[must_use]
pub fn mission_reference(game: &Q1EntityServices, id: &ActorId, key: &str) -> Option<ActorId> {
    let actor = game.entity_ref(id)?.references.get(key).cloned().flatten()?;
    game.host.actors.is_live(&actor).then_some(actor)
}

/// Store a binary32 numeric field under a key (`setMissionNumber`).
pub fn set_mission_number(game: &mut Q1EntityServices, id: &ActorId, key: &str, value: f64) -> Result<(), Q1Error> {
    let text = fround(value).to_string();
    game.update_entity(id, |entity| {
        entity.fields.insert(key.to_string(), text);
    })
}

/// Angles facing along a velocity (`velocityAngles`).
#[must_use]
pub fn velocity_angles(velocity: Vec3) -> Vec3 {
    let x = f64::from(velocity.x);
    let y = f64::from(velocity.y);
    let z = f64::from(velocity.z);
    let yaw = if x == 0.0 && y == 0.0 {
        0.0
    } else {
        y.atan2(x) * 180.0 / std::f64::consts::PI
    };
    let pitch = if x == 0.0 && y == 0.0 {
        if z > 0.0 {
            90.0
        } else {
            270.0
        }
    } else {
        z.atan2(x.hypot(y)) * 180.0 / std::f64::consts::PI
    };
    Vec3 {
        x: fround(if pitch < 0.0 { pitch + 360.0 } else { pitch }) as f32,
        y: fround(if yaw < 0.0 { yaw + 360.0 } else { yaw }) as f32,
        z: 0.0,
    }
}

/// Grenade toss velocity from view angles and aim (`grenadeVelocity`).
#[must_use]
pub fn grenade_velocity(game: &mut Q1EntityServices, angles: Vec3, aimed: Vec3) -> Vec3 {
    if angles.x == 0.0 {
        let scaled = vscale(aimed, 600.0);
        return Vec3 {
            x: scaled.x,
            y: scaled.y,
            z: 200.0,
        };
    }
    let basis = game.make_vectors(angles);
    vadd(
        vadd(
            vadd(vscale(basis.forward, 600.0), vscale(basis.up, 200.0)),
            vscale(basis.right, (game.host.random() * 2.0 - 1.0) * 10.0),
        ),
        vscale(basis.up, (game.host.random() * 2.0 - 1.0) * 10.0),
    )
}

/// Steer a missile along a velocity (`moveMissile`).
pub fn move_missile(game: &mut Q1EntityServices, id: &ActorId, velocity: Vec3) -> Result<(), Q1Error> {
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(velocity),
            angles: Some(velocity_angles(normalize(velocity))),
            ground: Some(None),
            ..Default::default()
        },
    )
}

/// Test options shared by mission-pack module tests.
#[cfg(test)]
pub(crate) fn test_options() -> crate::q1::foundation::types::Q1FoundationOptions {
    use qa_core::identity::ProviderId;

    use crate::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};

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

/// Test game with a mock host, shared by mission-pack module tests.
#[cfg(test)]
pub(crate) fn test_game() -> Q1EntityServices {
    use crate::q1::foundation::host::mock::mock_host;

    let (host, _) = mock_host();
    Q1EntityServices::new(host, test_options()).expect("game")
}

/// Test game that also returns the mock event log.
#[cfg(test)]
pub(crate) fn test_game_with_events() -> (
    Q1EntityServices,
    std::rc::Rc<std::cell::RefCell<crate::q1::foundation::host::mock::MockEvents>>,
) {
    use crate::q1::foundation::host::mock::mock_host;

    let (host, events) = mock_host();
    (Q1EntityServices::new(host, test_options()).expect("game"), events)
}

#[cfg(test)]
mod tests {
    use qa_core::math::Vec3;

    use super::*;
    use crate::q1::foundation::types::ZERO;

    #[test]
    fn mission_ids_match_donor() {
        assert_eq!(Q1MissionPack::Hipnotic.as_str(), "hipnotic");
        assert_eq!(Q1MissionPack::Rogue.as_str(), "rogue");
        assert_eq!(MissionWeapon::HipnoticLaser.as_str(), "hipnotic:laser");
        assert_eq!(MissionWeapon::RoguePlasma.as_str(), "rogue:plasma");
        assert_eq!(MissionPowerup::HipnoticWetsuit.as_str(), "hipnotic:wetsuit");
        assert_eq!(MissionPowerup::RogueAntigrav.as_str(), "rogue:antigrav");
        assert_eq!(
            Q1Weapon::from(MissionWeapon::RogueMultiRocket),
            Q1Weapon::RogueMultiRocket
        );
        assert_eq!(
            Q1Powerup::from(MissionPowerup::HipnoticEmpathy),
            Q1Powerup::HipnoticEmpathy
        );
    }

    #[test]
    fn mission_weapon_roster_matches_donor() {
        assert_eq!(MISSION_WEAPONS.len(), 8);
        assert_eq!(MISSION_WEAPONS[0].model, "progs/v_laserg.mdl");
        assert_eq!(MISSION_WEAPONS[0].pickup, Some("weapon_laser_gun"));
        assert_eq!(MISSION_WEAPONS[0].ammo, Some("q1:ammo/cells"));
        assert_eq!(MISSION_WEAPONS[1].ammo, None);
        assert_eq!(MISSION_WEAPONS[2].pickup_ammo, 6);
        assert_eq!(MISSION_WEAPONS[7].rank, 0);
        assert_eq!(MISSION_WEAPONS[1].rank, 9);
    }

    #[test]
    fn velocity_angles_match_donor_branches() {
        assert_eq!(
            velocity_angles(ZERO),
            Vec3 {
                x: 270.0,
                y: 0.0,
                z: 0.0
            }
        );
        assert_eq!(
            velocity_angles(Vec3 { x: 1.0, y: 0.0, z: 0.0 }),
            Vec3 { x: 0.0, y: 0.0, z: 0.0 }
        );
        assert_eq!(
            velocity_angles(Vec3 { x: 0.0, y: 0.0, z: 1.0 }),
            Vec3 {
                x: 90.0,
                y: 0.0,
                z: 0.0
            }
        );
        assert_eq!(
            velocity_angles(Vec3 {
                x: 0.0,
                y: -1.0,
                z: 0.0
            }),
            Vec3 {
                x: 0.0,
                y: 270.0,
                z: 0.0
            }
        );
    }

    #[test]
    fn grenade_velocity_levels_with_zero_pitch() {
        let mut game = test_game();
        let velocity = grenade_velocity(&mut game, ZERO, Vec3 { x: 1.0, y: 0.0, z: 0.0 });
        assert_eq!(
            velocity,
            Vec3 {
                x: 600.0,
                y: 0.0,
                z: 200.0
            }
        );
    }

    #[test]
    fn mission_references_round_trip() {
        let mut game = test_game();
        let first = game.create("info_null", None, None).expect("first");
        let second = game.create("info_null", None, None).expect("second");
        set_mission_reference(&mut game, &first, "hipnotic:enemy", Some(&second)).expect("set");
        assert_eq!(mission_reference(&game, &first, "hipnotic:enemy"), Some(second.clone()));
        set_mission_reference(&mut game, &first, "hipnotic:enemy", None).expect("clear");
        assert_eq!(mission_reference(&game, &first, "hipnotic:enemy"), None);
        assert_eq!(mission_reference(&game, &first, "missing"), None);
    }

    #[test]
    fn mission_numbers_store_binary32() {
        let mut game = test_game();
        let id = game.create("info_null", None, None).expect("entity");
        set_mission_number(&mut game, &id, "hipnotic:detonating", 1.0).expect("set");
        assert_eq!(
            game.entity_ref(&id).map(|entity| entity.number("hipnotic:detonating")),
            Some(1.0)
        );
    }

    #[test]
    fn move_missile_faces_velocity_and_clears_ground() {
        let mut game = test_game();
        let id = game.create("hiplaser", None, None).expect("laser");
        move_missile(
            &mut game,
            &id,
            Vec3 {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
        )
        .expect("steer");
        let body = game.body(&id).expect("body");
        assert_eq!(
            body.velocity,
            Vec3 {
                x: 100.0,
                y: 0.0,
                z: 0.0
            }
        );
        assert_eq!(body.angles, Vec3 { x: 0.0, y: 0.0, z: 0.0 });
        assert_eq!(body.ground, None);
    }
}
