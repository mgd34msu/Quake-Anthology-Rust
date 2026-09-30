//! SIBLING-MIRROR of `src/content/q3/base/shared/items.ts`.
//!
//! The canonical port is owned by sibling lane impl-content-q3 and will
//! union-merge at `crate::q3::base::shared::items`; this module keeps the
//! predecessor flat-port content so the base group compiles standalone. Delete
//! at unification and re-point imports at the canonical module.

use qa_core::math::Vec3;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::mirrors::*;
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::trajectory::*;

// ---------------------------------------------------------------------------
// shared/items.ts
// ---------------------------------------------------------------------------

/// Item type plus tag word (`ItemDefinition` type/tag pair).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKindTag {
    /// Weapon with its weapon tag.
    Weapon(Weapon),
    /// Ammunition with its weapon tag.
    Ammo(Weapon),
    /// Powerup with its powerup tag.
    Powerup(Powerup),
    /// Persistant powerup with its powerup tag.
    PersistantPowerup(Powerup),
    /// Team item with its powerup tag.
    Team(Powerup),
    /// Holdable with its holdable tag.
    Holdable(Holdable),
    /// Reserved empty item.
    Bad,
    /// Armor.
    Armor,
    /// Health.
    Health,
}

/// Item definition (`ItemDefinition`, `bg_itemlist`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemDefinition {
    /// Class name.
    pub class_name: Option<&'static str>,
    /// Pickup sound.
    pub pickup_sound: Option<&'static str>,
    /// World models.
    pub world_models: [Option<&'static str>; 4],
    /// Icon.
    pub icon: Option<&'static str>,
    /// Pickup name.
    pub pickup_name: Option<&'static str>,
    /// Quantity.
    pub quantity: i32,
    /// Precaches.
    pub precaches: &'static str,
    /// Sounds.
    pub sounds: &'static str,
    /// Type plus tag.
    pub kind: ItemKindTag,
}

impl ItemDefinition {
    /// Item type.
    #[must_use]
    pub fn item_type(&self) -> ItemType {
        match self.kind {
            ItemKindTag::Weapon(_) => ItemType::ItWeapon,
            ItemKindTag::Ammo(_) => ItemType::ItAmmo,
            ItemKindTag::Powerup(_) => ItemType::ItPowerup,
            ItemKindTag::PersistantPowerup(_) => ItemType::ItPersistantPowerup,
            ItemKindTag::Team(_) => ItemType::ItTeam,
            ItemKindTag::Holdable(_) => ItemType::ItHoldable,
            ItemKindTag::Bad => ItemType::ItBad,
            ItemKindTag::Armor => ItemType::ItArmor,
            ItemKindTag::Health => ItemType::ItHealth,
        }
    }

    /// Weapon tag for weapon and ammunition items.
    #[must_use]
    pub fn weapon_tag(&self) -> Option<Weapon> {
        match self.kind {
            ItemKindTag::Weapon(tag) | ItemKindTag::Ammo(tag) => Some(tag),
            _ => None,
        }
    }

    /// Powerup tag for powerup, persistant-powerup, and team items.
    #[must_use]
    pub fn powerup_tag(&self) -> Option<Powerup> {
        match self.kind {
            ItemKindTag::Powerup(tag) | ItemKindTag::PersistantPowerup(tag) | ItemKindTag::Team(tag) => Some(tag),
            _ => None,
        }
    }

    /// Holdable tag for holdable items.
    #[must_use]
    pub fn holdable_tag(&self) -> Option<Holdable> {
        match self.kind {
            ItemKindTag::Holdable(tag) => Some(tag),
            _ => None,
        }
    }
}

// Index zero is the source's reserved empty item. The terminal C marker is
// excluded.
pub(crate) static ITEM_DEFINITIONS: [ItemDefinition; 52] = [
    ItemDefinition {
        class_name: None,
        pickup_sound: None,
        world_models: [None, None, None, None],
        icon: None,
        pickup_name: None,
        quantity: 0,
        kind: ItemKindTag::Bad,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_armor_shard"),
        pickup_sound: Some("sound/misc/ar1_pkup.wav"),
        world_models: [
            Some("models/powerups/armor/shard.md3"),
            Some("models/powerups/armor/shard_sphere.md3"),
            None,
            None,
        ],
        icon: Some("icons/iconr_shard"),
        pickup_name: Some("Armor Shard"),
        quantity: 5,
        kind: ItemKindTag::Armor,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_armor_combat"),
        pickup_sound: Some("sound/misc/ar2_pkup.wav"),
        world_models: [
            Some("models/powerups/armor/armor_yel.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconr_yellow"),
        pickup_name: Some("Armor"),
        quantity: 50,
        kind: ItemKindTag::Armor,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_armor_body"),
        pickup_sound: Some("sound/misc/ar2_pkup.wav"),
        world_models: [
            Some("models/powerups/armor/armor_red.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconr_red"),
        pickup_name: Some("Heavy Armor"),
        quantity: 100,
        kind: ItemKindTag::Armor,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_health_small"),
        pickup_sound: Some("sound/items/s_health.wav"),
        world_models: [
            Some("models/powerups/health/small_cross.md3"),
            Some("models/powerups/health/small_sphere.md3"),
            None,
            None,
        ],
        icon: Some("icons/iconh_green"),
        pickup_name: Some("5 Health"),
        quantity: 5,
        kind: ItemKindTag::Health,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_health"),
        pickup_sound: Some("sound/items/n_health.wav"),
        world_models: [
            Some("models/powerups/health/medium_cross.md3"),
            Some("models/powerups/health/medium_sphere.md3"),
            None,
            None,
        ],
        icon: Some("icons/iconh_yellow"),
        pickup_name: Some("25 Health"),
        quantity: 25,
        kind: ItemKindTag::Health,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_health_large"),
        pickup_sound: Some("sound/items/l_health.wav"),
        world_models: [
            Some("models/powerups/health/large_cross.md3"),
            Some("models/powerups/health/large_sphere.md3"),
            None,
            None,
        ],
        icon: Some("icons/iconh_red"),
        pickup_name: Some("50 Health"),
        quantity: 50,
        kind: ItemKindTag::Health,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_health_mega"),
        pickup_sound: Some("sound/items/m_health.wav"),
        world_models: [
            Some("models/powerups/health/mega_cross.md3"),
            Some("models/powerups/health/mega_sphere.md3"),
            None,
            None,
        ],
        icon: Some("icons/iconh_mega"),
        pickup_name: Some("Mega Health"),
        quantity: 100,
        kind: ItemKindTag::Health,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_gauntlet"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/gauntlet/gauntlet.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_gauntlet"),
        pickup_name: Some("Gauntlet"),
        quantity: 0,
        kind: ItemKindTag::Weapon(Weapon::WpGauntlet),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_shotgun"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/shotgun/shotgun.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_shotgun"),
        pickup_name: Some("Shotgun"),
        quantity: 10,
        kind: ItemKindTag::Weapon(Weapon::WpShotgun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_machinegun"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/machinegun/machinegun.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_machinegun"),
        pickup_name: Some("Machinegun"),
        quantity: 40,
        kind: ItemKindTag::Weapon(Weapon::WpMachinegun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_grenadelauncher"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/grenadel/grenadel.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_grenade"),
        pickup_name: Some("Grenade Launcher"),
        quantity: 10,
        kind: ItemKindTag::Weapon(Weapon::WpGrenadeLauncher),
        precaches: "",
        sounds: "sound/weapons/grenade/hgrenb1a.wav sound/weapons/grenade/hgrenb2a.wav",
    },
    ItemDefinition {
        class_name: Some("weapon_rocketlauncher"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/rocketl/rocketl.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_rocket"),
        pickup_name: Some("Rocket Launcher"),
        quantity: 10,
        kind: ItemKindTag::Weapon(Weapon::WpRocketLauncher),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_lightning"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/lightning/lightning.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_lightning"),
        pickup_name: Some("Lightning Gun"),
        quantity: 100,
        kind: ItemKindTag::Weapon(Weapon::WpLightning),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_railgun"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/railgun/railgun.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_railgun"),
        pickup_name: Some("Railgun"),
        quantity: 10,
        kind: ItemKindTag::Weapon(Weapon::WpRailgun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_plasmagun"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/plasma/plasma.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_plasma"),
        pickup_name: Some("Plasma Gun"),
        quantity: 50,
        kind: ItemKindTag::Weapon(Weapon::WpPlasmagun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_bfg"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [Some("models/weapons2/bfg/bfg.md3"), None, None, None],
        icon: Some("icons/iconw_bfg"),
        pickup_name: Some("BFG10K"),
        quantity: 20,
        kind: ItemKindTag::Weapon(Weapon::WpBfg),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_grapplinghook"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/grapple/grapple.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_grapple"),
        pickup_name: Some("Grappling Hook"),
        quantity: 0,
        kind: ItemKindTag::Weapon(Weapon::WpGrapplingHook),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_shells"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/shotgunam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_shotgun"),
        pickup_name: Some("Shells"),
        quantity: 10,
        kind: ItemKindTag::Ammo(Weapon::WpShotgun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_bullets"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/machinegunam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_machinegun"),
        pickup_name: Some("Bullets"),
        quantity: 50,
        kind: ItemKindTag::Ammo(Weapon::WpMachinegun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_grenades"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/grenadeam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_grenade"),
        pickup_name: Some("Grenades"),
        quantity: 5,
        kind: ItemKindTag::Ammo(Weapon::WpGrenadeLauncher),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_cells"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/plasmaam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_plasma"),
        pickup_name: Some("Cells"),
        quantity: 30,
        kind: ItemKindTag::Ammo(Weapon::WpPlasmagun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_lightning"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/lightningam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_lightning"),
        pickup_name: Some("Lightning"),
        quantity: 60,
        kind: ItemKindTag::Ammo(Weapon::WpLightning),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_rockets"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/rocketam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_rocket"),
        pickup_name: Some("Rockets"),
        quantity: 5,
        kind: ItemKindTag::Ammo(Weapon::WpRocketLauncher),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_slugs"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/railgunam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_railgun"),
        pickup_name: Some("Slugs"),
        quantity: 10,
        kind: ItemKindTag::Ammo(Weapon::WpRailgun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_bfg"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [Some("models/powerups/ammo/bfgam.md3"), None, None, None],
        icon: Some("icons/icona_bfg"),
        pickup_name: Some("Bfg Ammo"),
        quantity: 15,
        kind: ItemKindTag::Ammo(Weapon::WpBfg),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("holdable_teleporter"),
        pickup_sound: Some("sound/items/holdable.wav"),
        world_models: [
            Some("models/powerups/holdable/teleporter.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/teleporter"),
        pickup_name: Some("Personal Teleporter"),
        quantity: 60,
        kind: ItemKindTag::Holdable(Holdable::HiTeleporter),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("holdable_medkit"),
        pickup_sound: Some("sound/items/holdable.wav"),
        world_models: [
            Some("models/powerups/holdable/medkit.md3"),
            Some("models/powerups/holdable/medkit_sphere.md3"),
            None,
            None,
        ],
        icon: Some("icons/medkit"),
        pickup_name: Some("Medkit"),
        quantity: 60,
        kind: ItemKindTag::Holdable(Holdable::HiMedkit),
        precaches: "",
        sounds: "sound/items/use_medkit.wav",
    },
    ItemDefinition {
        class_name: Some("item_quad"),
        pickup_sound: Some("sound/items/quaddamage.wav"),
        world_models: [
            Some("models/powerups/instant/quad.md3"),
            Some("models/powerups/instant/quad_ring.md3"),
            None,
            None,
        ],
        icon: Some("icons/quad"),
        pickup_name: Some("Quad Damage"),
        quantity: 30,
        kind: ItemKindTag::Powerup(Powerup::PwQuad),
        precaches: "",
        sounds: "sound/items/damage2.wav sound/items/damage3.wav",
    },
    ItemDefinition {
        class_name: Some("item_enviro"),
        pickup_sound: Some("sound/items/protect.wav"),
        world_models: [
            Some("models/powerups/instant/enviro.md3"),
            Some("models/powerups/instant/enviro_ring.md3"),
            None,
            None,
        ],
        icon: Some("icons/envirosuit"),
        pickup_name: Some("Battle Suit"),
        quantity: 30,
        kind: ItemKindTag::Powerup(Powerup::PwBattlesuit),
        precaches: "",
        sounds: "sound/items/airout.wav sound/items/protect3.wav",
    },
    ItemDefinition {
        class_name: Some("item_haste"),
        pickup_sound: Some("sound/items/haste.wav"),
        world_models: [
            Some("models/powerups/instant/haste.md3"),
            Some("models/powerups/instant/haste_ring.md3"),
            None,
            None,
        ],
        icon: Some("icons/haste"),
        pickup_name: Some("Speed"),
        quantity: 30,
        kind: ItemKindTag::Powerup(Powerup::PwHaste),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_invis"),
        pickup_sound: Some("sound/items/invisibility.wav"),
        world_models: [
            Some("models/powerups/instant/invis.md3"),
            Some("models/powerups/instant/invis_ring.md3"),
            None,
            None,
        ],
        icon: Some("icons/invis"),
        pickup_name: Some("Invisibility"),
        quantity: 30,
        kind: ItemKindTag::Powerup(Powerup::PwInvis),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_regen"),
        pickup_sound: Some("sound/items/regeneration.wav"),
        world_models: [
            Some("models/powerups/instant/regen.md3"),
            Some("models/powerups/instant/regen_ring.md3"),
            None,
            None,
        ],
        icon: Some("icons/regen"),
        pickup_name: Some("Regeneration"),
        quantity: 30,
        kind: ItemKindTag::Powerup(Powerup::PwRegen),
        precaches: "",
        sounds: "sound/items/regen.wav",
    },
    ItemDefinition {
        class_name: Some("item_flight"),
        pickup_sound: Some("sound/items/flight.wav"),
        world_models: [
            Some("models/powerups/instant/flight.md3"),
            Some("models/powerups/instant/flight_ring.md3"),
            None,
            None,
        ],
        icon: Some("icons/flight"),
        pickup_name: Some("Flight"),
        quantity: 60,
        kind: ItemKindTag::Powerup(Powerup::PwFlight),
        precaches: "",
        sounds: "sound/items/flight.wav",
    },
    ItemDefinition {
        class_name: Some("team_CTF_redflag"),
        pickup_sound: None,
        world_models: [Some("models/flags/r_flag.md3"), None, None, None],
        icon: Some("icons/iconf_red1"),
        pickup_name: Some("Red Flag"),
        quantity: 0,
        kind: ItemKindTag::Team(Powerup::PwRedflag),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("team_CTF_blueflag"),
        pickup_sound: None,
        world_models: [Some("models/flags/b_flag.md3"), None, None, None],
        icon: Some("icons/iconf_blu1"),
        pickup_name: Some("Blue Flag"),
        quantity: 0,
        kind: ItemKindTag::Team(Powerup::PwBlueflag),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("holdable_kamikaze"),
        pickup_sound: Some("sound/items/holdable.wav"),
        world_models: [Some("models/powerups/kamikazi.md3"), None, None, None],
        icon: Some("icons/kamikaze"),
        pickup_name: Some("Kamikaze"),
        quantity: 60,
        kind: ItemKindTag::Holdable(Holdable::HiKamikaze),
        precaches: "",
        sounds: "sound/items/kamikazerespawn.wav",
    },
    ItemDefinition {
        class_name: Some("holdable_portal"),
        pickup_sound: Some("sound/items/holdable.wav"),
        world_models: [
            Some("models/powerups/holdable/porter.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/portal"),
        pickup_name: Some("Portal"),
        quantity: 60,
        kind: ItemKindTag::Holdable(Holdable::HiPortal),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("holdable_invulnerability"),
        pickup_sound: Some("sound/items/holdable.wav"),
        world_models: [
            Some("models/powerups/holdable/invulnerability.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/invulnerability"),
        pickup_name: Some("Invulnerability"),
        quantity: 60,
        kind: ItemKindTag::Holdable(Holdable::HiInvulnerability),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_nails"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/nailgunam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_nailgun"),
        pickup_name: Some("Nails"),
        quantity: 20,
        kind: ItemKindTag::Ammo(Weapon::WpNailgun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_mines"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/proxmineam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_proxlauncher"),
        pickup_name: Some("Proximity Mines"),
        quantity: 10,
        kind: ItemKindTag::Ammo(Weapon::WpProxLauncher),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_belt"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/chaingunam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_chaingun"),
        pickup_name: Some("Chaingun Belt"),
        quantity: 100,
        kind: ItemKindTag::Ammo(Weapon::WpChaingun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_scout"),
        pickup_sound: Some("sound/items/scout.wav"),
        world_models: [Some("models/powerups/scout.md3"), None, None, None],
        icon: Some("icons/scout"),
        pickup_name: Some("Scout"),
        quantity: 30,
        kind: ItemKindTag::PersistantPowerup(Powerup::PwScout),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_guard"),
        pickup_sound: Some("sound/items/guard.wav"),
        world_models: [Some("models/powerups/guard.md3"), None, None, None],
        icon: Some("icons/guard"),
        pickup_name: Some("Guard"),
        quantity: 30,
        kind: ItemKindTag::PersistantPowerup(Powerup::PwGuard),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_doubler"),
        pickup_sound: Some("sound/items/doubler.wav"),
        world_models: [Some("models/powerups/doubler.md3"), None, None, None],
        icon: Some("icons/doubler"),
        pickup_name: Some("Doubler"),
        quantity: 30,
        kind: ItemKindTag::PersistantPowerup(Powerup::PwDoubler),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_ammoregen"),
        pickup_sound: Some("sound/items/ammoregen.wav"),
        world_models: [Some("models/powerups/ammo.md3"), None, None, None],
        icon: Some("icons/ammo_regen"),
        pickup_name: Some("Ammo Regen"),
        quantity: 30,
        kind: ItemKindTag::PersistantPowerup(Powerup::PwAmmoregen),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("team_CTF_neutralflag"),
        pickup_sound: None,
        world_models: [Some("models/flags/n_flag.md3"), None, None, None],
        icon: Some("icons/iconf_neutral1"),
        pickup_name: Some("Neutral Flag"),
        quantity: 0,
        kind: ItemKindTag::Team(Powerup::PwNeutralflag),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_redcube"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [Some("models/powerups/orb/r_orb.md3"), None, None, None],
        icon: Some("icons/iconh_rorb"),
        pickup_name: Some("Red Cube"),
        quantity: 0,
        kind: ItemKindTag::Team(Powerup::PwNone),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_bluecube"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [Some("models/powerups/orb/b_orb.md3"), None, None, None],
        icon: Some("icons/iconh_borb"),
        pickup_name: Some("Blue Cube"),
        quantity: 0,
        kind: ItemKindTag::Team(Powerup::PwNone),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_nailgun"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons/nailgun/nailgun.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_nailgun"),
        pickup_name: Some("Nailgun"),
        quantity: 10,
        kind: ItemKindTag::Weapon(Weapon::WpNailgun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_prox_launcher"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons/proxmine/proxmine.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_proxlauncher"),
        pickup_name: Some("Prox Launcher"),
        quantity: 5,
        kind: ItemKindTag::Weapon(Weapon::WpProxLauncher),
        precaches: "",
        sounds: "sound/weapons/proxmine/wstbtick.wav sound/weapons/proxmine/wstbactv.wav sound/weapons/proxmine/wstbimpl.wav sound/weapons/proxmine/wstbimpm.wav sound/weapons/proxmine/wstbimpd.wav sound/weapons/proxmine/wstbactv.wav",
    },
    ItemDefinition {
        class_name: Some("weapon_chaingun"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons/vulcan/vulcan.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_chaingun"),
        pickup_name: Some("Chaingun"),
        quantity: 80,
        kind: ItemKindTag::Weapon(Weapon::WpChaingun),
        precaches: "",
        sounds: "sound/weapons/vulcan/wvulwind.wav",
    },
];

/// Base-game item count (the missionpack tail starts at index 36).
pub const BASE_ITEM_COUNT: usize = 36;

/// Item list for a product (`itemList`).
#[must_use]
pub fn item_list(product: Product) -> &'static [ItemDefinition] {
    match product {
        Product::Baseq3 => &ITEM_DEFINITIONS[..BASE_ITEM_COUNT],
        Product::Missionpack => &ITEM_DEFINITIONS[..],
    }
}

/// Item definition by index (`itemAt`).
pub fn item_at(product: Product, index: i32) -> Result<&'static ItemDefinition, Q3BaseError> {
    let list = item_list(product);
    usize::try_from(index)
        .ok()
        .and_then(|slot| list.get(slot))
        .ok_or_else(|| Q3BaseError::Range(format!("Item index out of range: {index}")))
}

/// Find an item by pickup name, ASCII case-insensitive (`BG_FindItem`).
#[must_use]
pub fn find_item(product: Product, pickup_name: &str) -> Option<&'static ItemDefinition> {
    let folded = ascii_fold(pickup_name);
    item_list(product)
        .iter()
        .find(|item| item.pickup_name.is_some_and(|name| ascii_fold(name) == folded))
}

/// Find a powerup, team, or persistant-powerup item by tag
/// (`BG_FindItemForPowerup`).
#[must_use]
pub fn find_item_for_powerup(product: Product, powerup: Powerup) -> Option<&'static ItemDefinition> {
    item_list(product)
        .iter()
        .find(|item| item.powerup_tag() == Some(powerup))
}

/// Find a holdable item by tag (`BG_FindItemForHoldable`).
pub fn find_item_for_holdable(product: Product, holdable: Holdable) -> Result<&'static ItemDefinition, Q3BaseError> {
    item_list(product)
        .iter()
        .find(|item| item.holdable_tag() == Some(holdable))
        .ok_or_else(|| Q3BaseError::Drop("HoldableItem not found".to_string()))
}

/// Find a weapon item by tag (`BG_FindItemForWeapon`).
pub fn find_item_for_weapon(product: Product, weapon: Weapon) -> Result<&'static ItemDefinition, Q3BaseError> {
    item_list(product)
        .iter()
        .find(|item| item.weapon_tag() == Some(weapon) && item.item_type() == ItemType::ItWeapon)
        .ok_or_else(|| Q3BaseError::Drop(format!("Couldn't find item for weapon {}", weapon as i32)))
}

/// Pickup entity words read by the grab rules (`PickupEntity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupEntity {
    /// Item model index.
    pub model_index: i32,
    /// Second model index.
    pub model_index2: i32,
    /// Generic value.
    pub generic1: i32,
}

/// Player inventory read by the grab rules (`PlayerInventory`).
pub trait PlayerInventory {
    /// Owning product.
    fn product(&self) -> Product;
    /// Health.
    fn health(&self) -> i32;
    /// Armor points.
    fn armor(&self) -> i32;
    /// Maximum health.
    fn max_health(&self) -> i32;
    /// Holdable item tag.
    fn holdable_item(&self) -> i32;
    /// Team tag.
    fn team(&self) -> i32;
    /// Ammunition for a weapon.
    fn ammo(&self, weapon: Weapon) -> i32;
    /// Powerup time remaining.
    fn powerup(&self, powerup: Powerup) -> i32;
    /// Persistant powerup item index (missionpack only).
    fn persistent_powerup_index(&self) -> i32 {
        0
    }
}

/// Whether armor can be grabbed (`canQ3ArmorBeGrabbed`, inlined source).
pub fn can_q3_armor_be_grabbed(ps: &dyn PlayerInventory) -> Result<bool, Q3BaseError> {
    if ps.product() == Product::Missionpack {
        if item_at(ps.product(), ps.persistent_powerup_index())?.powerup_tag() == Some(Powerup::PwScout) {
            return Ok(false);
        }
        let upper_bound =
            if item_at(ps.product(), ps.persistent_powerup_index())?.powerup_tag() == Some(Powerup::PwGuard) {
                ps.max_health()
            } else {
                ps.max_health() * 2
            };
        return Ok(ps.armor() < upper_bound);
    }
    Ok(ps.armor() < ps.max_health() * 2)
}

/// Whether an item can be grabbed (`BG_CanItemBeGrabbed`).
pub fn can_item_be_grabbed(gametype: i32, ent: &PickupEntity, ps: &dyn PlayerInventory) -> Result<bool, Q3BaseError> {
    if ent.model_index < 1 || ent.model_index >= item_list(ps.product()).len() as i32 {
        return Err(Q3BaseError::Drop("BG_CanItemBeGrabbed: index out of range".to_string()));
    }
    let item = item_at(ps.product(), ent.model_index)?;
    match item.item_type() {
        ItemType::ItWeapon => Ok(true),
        ItemType::ItAmmo => Ok(ps.ammo(item.weapon_tag().unwrap_or(Weapon::WpNone)) < 200),
        ItemType::ItArmor => can_q3_armor_be_grabbed(ps),
        ItemType::ItHealth => {
            if ps.product() == Product::Missionpack
                && item_at(ps.product(), ps.persistent_powerup_index())?.powerup_tag() == Some(Powerup::PwGuard)
            {
                return Ok(ps.health() < ps.max_health());
            }
            let limit = if item.quantity == 5 || item.quantity == 100 {
                2
            } else {
                1
            };
            Ok(ps.health() < ps.max_health() * limit)
        }
        ItemType::ItPowerup => Ok(true),
        ItemType::ItPersistantPowerup => {
            if ps.product() == Product::Baseq3 || ps.persistent_powerup_index() != 0 {
                return Ok(false);
            }
            if (ent.generic1 & 2) != 0 && ps.team() != Team::TeamRed as i32 {
                return Ok(false);
            }
            if (ent.generic1 & 4) != 0 && ps.team() != Team::TeamBlue as i32 {
                return Ok(false);
            }
            Ok(true)
        }
        ItemType::ItTeam => {
            let tag = item.powerup_tag().unwrap_or(Powerup::PwNone);
            if ps.product() == Product::Missionpack && gametype == GameType::Gt1fctf as i32 {
                if tag == Powerup::PwNeutralflag {
                    return Ok(true);
                }
                if ps.team() == Team::TeamRed as i32
                    && tag == Powerup::PwBlueflag
                    && ps.powerup(Powerup::PwNeutralflag) != 0
                {
                    return Ok(true);
                }
                if ps.team() == Team::TeamBlue as i32
                    && tag == Powerup::PwRedflag
                    && ps.powerup(Powerup::PwNeutralflag) != 0
                {
                    return Ok(true);
                }
            }
            if gametype == GameType::GtCtf as i32 {
                if ps.team() == Team::TeamRed as i32 {
                    return Ok(tag == Powerup::PwBlueflag
                        || (tag == Powerup::PwRedflag
                            && (ent.model_index2 != 0 || ps.powerup(Powerup::PwBlueflag) != 0)));
                }
                if ps.team() == Team::TeamBlue as i32 {
                    return Ok(tag == Powerup::PwRedflag
                        || (tag == Powerup::PwBlueflag
                            && (ent.model_index2 != 0 || ps.powerup(Powerup::PwRedflag) != 0)));
                }
            }
            Ok(ps.product() == Product::Missionpack && gametype == GameType::GtHarvester as i32)
        }
        ItemType::ItHoldable => Ok(ps.holdable_item() == 0),
        ItemType::ItBad => Err(Q3BaseError::Drop("BG_CanItemBeGrabbed: IT_BAD".to_string())),
    }
}

/// Whether a player origin touches an item (`BG_PlayerTouchesItem`).
#[must_use]
pub fn player_touches_item(player_origin: Vec3, item_position: &Trajectory, at_time: i32) -> bool {
    let origin = evaluate_trajectory(item_position, at_time);
    let x = player_origin.x - origin.x;
    let y = player_origin.y - origin.y;
    let z = player_origin.z - origin.z;
    !(x > 44.0 || x < -50.0 || y > 36.0 || y < -36.0 || z > 36.0 || z < -36.0)
}

/// ASCII case fold (`asciiFold`).
#[must_use]
pub fn ascii_fold(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_uppercase() {
                (c as u8 + 32) as char
            } else {
                c
            }
        })
        .collect()
}
