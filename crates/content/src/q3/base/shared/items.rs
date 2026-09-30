//! Quake III item list and pickup rules (`bg_misc.c`).
//!
//! Donor provenance: `src/content/q3/base/shared/items.ts` (ported from id
//! Software's `code/game/bg_misc.c`, `bg_itemlist` and the `BG_*` item
//! functions, GPL-2.0-or-later).

use qa_core::math::{add3, scale3, vec3, Vec3};
use thiserror::Error;

use super::definitions::{GameType, Holdable, ItemType, Powerup, Product, Team, Weapon, DEFAULT_GRAVITY};

/// Item failure (donor `CommonError("drop", ...)` plus index range errors).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ItemsError {
    /// Dropped-operation failure (donor `CommonError("drop", ...)`).
    #[error("drop: {0}")]
    Drop(String),
    /// Item index out of range (donor `RangeError`).
    #[error("{0}")]
    OutOfRange(String),
}

/// Item kind: the donor's `type`/`tag` pair as one discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// Weapon with its source weapon number.
    Weapon(Weapon),
    /// Ammo with its source weapon number.
    Ammo(Weapon),
    /// Powerup.
    Powerup(Powerup),
    /// Persistant powerup.
    PersistantPowerup(Powerup),
    /// Team item.
    Team(Powerup),
    /// Holdable.
    Holdable(Holdable),
    /// Reserved empty item.
    Bad,
    /// Armor.
    Armor,
    /// Health.
    Health,
}

impl ItemKind {
    /// Item type word.
    #[must_use]
    pub fn item_type(self) -> ItemType {
        match self {
            Self::Weapon(_) => ItemType::ItWeapon,
            Self::Ammo(_) => ItemType::ItAmmo,
            Self::Powerup(_) => ItemType::ItPowerup,
            Self::PersistantPowerup(_) => ItemType::ItPersistantPowerup,
            Self::Team(_) => ItemType::ItTeam,
            Self::Holdable(_) => ItemType::ItHoldable,
            Self::Bad => ItemType::ItBad,
            Self::Armor => ItemType::ItArmor,
            Self::Health => ItemType::ItHealth,
        }
    }

    /// Item tag word.
    #[must_use]
    pub fn tag(self) -> i32 {
        match self {
            Self::Weapon(weapon) | Self::Ammo(weapon) => weapon as i32,
            Self::Powerup(powerup) | Self::PersistantPowerup(powerup) | Self::Team(powerup) => powerup as i32,
            Self::Holdable(holdable) => holdable as i32,
            Self::Bad | Self::Armor | Self::Health => 0,
        }
    }
}

/// One item definition: presentation assets plus its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemDefinition {
    /// Entity class name.
    pub class_name: Option<&'static str>,
    /// Pickup sound path.
    pub pickup_sound: Option<&'static str>,
    /// World models.
    pub world_models: [Option<&'static str>; 4],
    /// Icon path.
    pub icon: Option<&'static str>,
    /// Pickup name.
    pub pickup_name: Option<&'static str>,
    /// Pickup quantity.
    pub quantity: i32,
    /// Precached assets.
    pub precaches: &'static str,
    /// Associated sounds.
    pub sounds: &'static str,
    /// Item kind.
    pub kind: ItemKind,
}

impl ItemDefinition {
    /// Item type word.
    #[must_use]
    pub fn item_type(self) -> ItemType {
        self.kind.item_type()
    }

    /// Item tag word.
    #[must_use]
    pub fn tag(self) -> i32 {
        self.kind.tag()
    }
}

/// Table-row builder: one parameter per definition field keeps the 52 rows
/// compact and auditable against the donor.
#[allow(clippy::too_many_arguments)]
const fn def(
    class_name: Option<&'static str>,
    pickup_sound: Option<&'static str>,
    world_models: [Option<&'static str>; 4],
    icon: Option<&'static str>,
    pickup_name: Option<&'static str>,
    quantity: i32,
    precaches: &'static str,
    sounds: &'static str,
    kind: ItemKind,
) -> ItemDefinition {
    ItemDefinition {
        class_name,
        pickup_sound,
        world_models,
        icon,
        pickup_name,
        quantity,
        precaches,
        sounds,
        kind,
    }
}

/// Index zero is the source's reserved empty item. The terminal C marker is
/// excluded.
static DEFINITIONS: [ItemDefinition; 52] = [
    def(None, None, [None, None, None, None], None, None, 0, "", "", ItemKind::Bad),
    def(
        Some("item_armor_shard"),
        Some("sound/misc/ar1_pkup.wav"),
        [
            Some("models/powerups/armor/shard.md3"),
            Some("models/powerups/armor/shard_sphere.md3"),
            None,
            None,
        ],
        Some("icons/iconr_shard"),
        Some("Armor Shard"),
        5,
        "",
        "",
        ItemKind::Armor,
    ),
    def(
        Some("item_armor_combat"),
        Some("sound/misc/ar2_pkup.wav"),
        [Some("models/powerups/armor/armor_yel.md3"), None, None, None],
        Some("icons/iconr_yellow"),
        Some("Armor"),
        50,
        "",
        "",
        ItemKind::Armor,
    ),
    def(
        Some("item_armor_body"),
        Some("sound/misc/ar2_pkup.wav"),
        [Some("models/powerups/armor/armor_red.md3"), None, None, None],
        Some("icons/iconr_red"),
        Some("Heavy Armor"),
        100,
        "",
        "",
        ItemKind::Armor,
    ),
    def(
        Some("item_health_small"),
        Some("sound/items/s_health.wav"),
        [
            Some("models/powerups/health/small_cross.md3"),
            Some("models/powerups/health/small_sphere.md3"),
            None,
            None,
        ],
        Some("icons/iconh_green"),
        Some("5 Health"),
        5,
        "",
        "",
        ItemKind::Health,
    ),
    def(
        Some("item_health"),
        Some("sound/items/n_health.wav"),
        [
            Some("models/powerups/health/medium_cross.md3"),
            Some("models/powerups/health/medium_sphere.md3"),
            None,
            None,
        ],
        Some("icons/iconh_yellow"),
        Some("25 Health"),
        25,
        "",
        "",
        ItemKind::Health,
    ),
    def(
        Some("item_health_large"),
        Some("sound/items/l_health.wav"),
        [
            Some("models/powerups/health/large_cross.md3"),
            Some("models/powerups/health/large_sphere.md3"),
            None,
            None,
        ],
        Some("icons/iconh_red"),
        Some("50 Health"),
        50,
        "",
        "",
        ItemKind::Health,
    ),
    def(
        Some("item_health_mega"),
        Some("sound/items/m_health.wav"),
        [
            Some("models/powerups/health/mega_cross.md3"),
            Some("models/powerups/health/mega_sphere.md3"),
            None,
            None,
        ],
        Some("icons/iconh_mega"),
        Some("Mega Health"),
        100,
        "",
        "",
        ItemKind::Health,
    ),
    def(
        Some("weapon_gauntlet"),
        Some("sound/misc/w_pkup.wav"),
        [Some("models/weapons2/gauntlet/gauntlet.md3"), None, None, None],
        Some("icons/iconw_gauntlet"),
        Some("Gauntlet"),
        0,
        "",
        "",
        ItemKind::Weapon(Weapon::WpGauntlet),
    ),
    def(
        Some("weapon_shotgun"),
        Some("sound/misc/w_pkup.wav"),
        [Some("models/weapons2/shotgun/shotgun.md3"), None, None, None],
        Some("icons/iconw_shotgun"),
        Some("Shotgun"),
        10,
        "",
        "",
        ItemKind::Weapon(Weapon::WpShotgun),
    ),
    def(
        Some("weapon_machinegun"),
        Some("sound/misc/w_pkup.wav"),
        [Some("models/weapons2/machinegun/machinegun.md3"), None, None, None],
        Some("icons/iconw_machinegun"),
        Some("Machinegun"),
        40,
        "",
        "",
        ItemKind::Weapon(Weapon::WpMachinegun),
    ),
    def(
        Some("weapon_grenadelauncher"),
        Some("sound/misc/w_pkup.wav"),
        [Some("models/weapons2/grenadel/grenadel.md3"), None, None, None],
        Some("icons/iconw_grenade"),
        Some("Grenade Launcher"),
        10,
        "",
        "sound/weapons/grenade/hgrenb1a.wav sound/weapons/grenade/hgrenb2a.wav",
        ItemKind::Weapon(Weapon::WpGrenadeLauncher),
    ),
    def(
        Some("weapon_rocketlauncher"),
        Some("sound/misc/w_pkup.wav"),
        [Some("models/weapons2/rocketl/rocketl.md3"), None, None, None],
        Some("icons/iconw_rocket"),
        Some("Rocket Launcher"),
        10,
        "",
        "",
        ItemKind::Weapon(Weapon::WpRocketLauncher),
    ),
    def(
        Some("weapon_lightning"),
        Some("sound/misc/w_pkup.wav"),
        [Some("models/weapons2/lightning/lightning.md3"), None, None, None],
        Some("icons/iconw_lightning"),
        Some("Lightning Gun"),
        100,
        "",
        "",
        ItemKind::Weapon(Weapon::WpLightning),
    ),
    def(
        Some("weapon_railgun"),
        Some("sound/misc/w_pkup.wav"),
        [Some("models/weapons2/railgun/railgun.md3"), None, None, None],
        Some("icons/iconw_railgun"),
        Some("Railgun"),
        10,
        "",
        "",
        ItemKind::Weapon(Weapon::WpRailgun),
    ),
    def(
        Some("weapon_plasmagun"),
        Some("sound/misc/w_pkup.wav"),
        [Some("models/weapons2/plasma/plasma.md3"), None, None, None],
        Some("icons/iconw_plasma"),
        Some("Plasma Gun"),
        50,
        "",
        "",
        ItemKind::Weapon(Weapon::WpPlasmagun),
    ),
    def(
        Some("weapon_bfg"),
        Some("sound/misc/w_pkup.wav"),
        [Some("models/weapons2/bfg/bfg.md3"), None, None, None],
        Some("icons/iconw_bfg"),
        Some("BFG10K"),
        20,
        "",
        "",
        ItemKind::Weapon(Weapon::WpBfg),
    ),
    def(
        Some("weapon_grapplinghook"),
        Some("sound/misc/w_pkup.wav"),
        [Some("models/weapons2/grapple/grapple.md3"), None, None, None],
        Some("icons/iconw_grapple"),
        Some("Grappling Hook"),
        0,
        "",
        "",
        ItemKind::Weapon(Weapon::WpGrapplingHook),
    ),
    def(
        Some("ammo_shells"),
        Some("sound/misc/am_pkup.wav"),
        [Some("models/powerups/ammo/shotgunam.md3"), None, None, None],
        Some("icons/icona_shotgun"),
        Some("Shells"),
        10,
        "",
        "",
        ItemKind::Ammo(Weapon::WpShotgun),
    ),
    def(
        Some("ammo_bullets"),
        Some("sound/misc/am_pkup.wav"),
        [Some("models/powerups/ammo/machinegunam.md3"), None, None, None],
        Some("icons/icona_machinegun"),
        Some("Bullets"),
        50,
        "",
        "",
        ItemKind::Ammo(Weapon::WpMachinegun),
    ),
    def(
        Some("ammo_grenades"),
        Some("sound/misc/am_pkup.wav"),
        [Some("models/powerups/ammo/grenadeam.md3"), None, None, None],
        Some("icons/icona_grenade"),
        Some("Grenades"),
        5,
        "",
        "",
        ItemKind::Ammo(Weapon::WpGrenadeLauncher),
    ),
    def(
        Some("ammo_cells"),
        Some("sound/misc/am_pkup.wav"),
        [Some("models/powerups/ammo/plasmaam.md3"), None, None, None],
        Some("icons/icona_plasma"),
        Some("Cells"),
        30,
        "",
        "",
        ItemKind::Ammo(Weapon::WpPlasmagun),
    ),
    def(
        Some("ammo_lightning"),
        Some("sound/misc/am_pkup.wav"),
        [Some("models/powerups/ammo/lightningam.md3"), None, None, None],
        Some("icons/icona_lightning"),
        Some("Lightning"),
        60,
        "",
        "",
        ItemKind::Ammo(Weapon::WpLightning),
    ),
    def(
        Some("ammo_rockets"),
        Some("sound/misc/am_pkup.wav"),
        [Some("models/powerups/ammo/rocketam.md3"), None, None, None],
        Some("icons/icona_rocket"),
        Some("Rockets"),
        5,
        "",
        "",
        ItemKind::Ammo(Weapon::WpRocketLauncher),
    ),
    def(
        Some("ammo_slugs"),
        Some("sound/misc/am_pkup.wav"),
        [Some("models/powerups/ammo/railgunam.md3"), None, None, None],
        Some("icons/icona_railgun"),
        Some("Slugs"),
        10,
        "",
        "",
        ItemKind::Ammo(Weapon::WpRailgun),
    ),
    def(
        Some("ammo_bfg"),
        Some("sound/misc/am_pkup.wav"),
        [Some("models/powerups/ammo/bfgam.md3"), None, None, None],
        Some("icons/icona_bfg"),
        Some("Bfg Ammo"),
        15,
        "",
        "",
        ItemKind::Ammo(Weapon::WpBfg),
    ),
    def(
        Some("holdable_teleporter"),
        Some("sound/items/holdable.wav"),
        [Some("models/powerups/holdable/teleporter.md3"), None, None, None],
        Some("icons/teleporter"),
        Some("Personal Teleporter"),
        60,
        "",
        "",
        ItemKind::Holdable(Holdable::HiTeleporter),
    ),
    def(
        Some("holdable_medkit"),
        Some("sound/items/holdable.wav"),
        [
            Some("models/powerups/holdable/medkit.md3"),
            Some("models/powerups/holdable/medkit_sphere.md3"),
            None,
            None,
        ],
        Some("icons/medkit"),
        Some("Medkit"),
        60,
        "",
        "sound/items/use_medkit.wav",
        ItemKind::Holdable(Holdable::HiMedkit),
    ),
    def(
        Some("item_quad"),
        Some("sound/items/quaddamage.wav"),
        [
            Some("models/powerups/instant/quad.md3"),
            Some("models/powerups/instant/quad_ring.md3"),
            None,
            None,
        ],
        Some("icons/quad"),
        Some("Quad Damage"),
        30,
        "",
        "sound/items/damage2.wav sound/items/damage3.wav",
        ItemKind::Powerup(Powerup::PwQuad),
    ),
    def(
        Some("item_enviro"),
        Some("sound/items/protect.wav"),
        [
            Some("models/powerups/instant/enviro.md3"),
            Some("models/powerups/instant/enviro_ring.md3"),
            None,
            None,
        ],
        Some("icons/envirosuit"),
        Some("Battle Suit"),
        30,
        "",
        "sound/items/airout.wav sound/items/protect3.wav",
        ItemKind::Powerup(Powerup::PwBattlesuit),
    ),
    def(
        Some("item_haste"),
        Some("sound/items/haste.wav"),
        [
            Some("models/powerups/instant/haste.md3"),
            Some("models/powerups/instant/haste_ring.md3"),
            None,
            None,
        ],
        Some("icons/haste"),
        Some("Speed"),
        30,
        "",
        "",
        ItemKind::Powerup(Powerup::PwHaste),
    ),
    def(
        Some("item_invis"),
        Some("sound/items/invisibility.wav"),
        [
            Some("models/powerups/instant/invis.md3"),
            Some("models/powerups/instant/invis_ring.md3"),
            None,
            None,
        ],
        Some("icons/invis"),
        Some("Invisibility"),
        30,
        "",
        "",
        ItemKind::Powerup(Powerup::PwInvis),
    ),
    def(
        Some("item_regen"),
        Some("sound/items/regeneration.wav"),
        [
            Some("models/powerups/instant/regen.md3"),
            Some("models/powerups/instant/regen_ring.md3"),
            None,
            None,
        ],
        Some("icons/regen"),
        Some("Regeneration"),
        30,
        "",
        "sound/items/regen.wav",
        ItemKind::Powerup(Powerup::PwRegen),
    ),
    def(
        Some("item_flight"),
        Some("sound/items/flight.wav"),
        [
            Some("models/powerups/instant/flight.md3"),
            Some("models/powerups/instant/flight_ring.md3"),
            None,
            None,
        ],
        Some("icons/flight"),
        Some("Flight"),
        60,
        "",
        "sound/items/flight.wav",
        ItemKind::Powerup(Powerup::PwFlight),
    ),
    def(
        Some("team_CTF_redflag"),
        None,
        [Some("models/flags/r_flag.md3"), None, None, None],
        Some("icons/iconf_red1"),
        Some("Red Flag"),
        0,
        "",
        "",
        ItemKind::Team(Powerup::PwRedflag),
    ),
    def(
        Some("team_CTF_blueflag"),
        None,
        [Some("models/flags/b_flag.md3"), None, None, None],
        Some("icons/iconf_blu1"),
        Some("Blue Flag"),
        0,
        "",
        "",
        ItemKind::Team(Powerup::PwBlueflag),
    ),
    def(
        Some("holdable_kamikaze"),
        Some("sound/items/holdable.wav"),
        [Some("models/powerups/kamikazi.md3"), None, None, None],
        Some("icons/kamikaze"),
        Some("Kamikaze"),
        60,
        "",
        "sound/items/kamikazerespawn.wav",
        ItemKind::Holdable(Holdable::HiKamikaze),
    ),
    def(
        Some("holdable_portal"),
        Some("sound/items/holdable.wav"),
        [Some("models/powerups/holdable/porter.md3"), None, None, None],
        Some("icons/portal"),
        Some("Portal"),
        60,
        "",
        "",
        ItemKind::Holdable(Holdable::HiPortal),
    ),
    def(
        Some("holdable_invulnerability"),
        Some("sound/items/holdable.wav"),
        [Some("models/powerups/holdable/invulnerability.md3"), None, None, None],
        Some("icons/invulnerability"),
        Some("Invulnerability"),
        60,
        "",
        "",
        ItemKind::Holdable(Holdable::HiInvulnerability),
    ),
    def(
        Some("ammo_nails"),
        Some("sound/misc/am_pkup.wav"),
        [Some("models/powerups/ammo/nailgunam.md3"), None, None, None],
        Some("icons/icona_nailgun"),
        Some("Nails"),
        20,
        "",
        "",
        ItemKind::Ammo(Weapon::WpNailgun),
    ),
    def(
        Some("ammo_mines"),
        Some("sound/misc/am_pkup.wav"),
        [Some("models/powerups/ammo/proxmineam.md3"), None, None, None],
        Some("icons/icona_proxlauncher"),
        Some("Proximity Mines"),
        10,
        "",
        "",
        ItemKind::Ammo(Weapon::WpProxLauncher),
    ),
    def(
        Some("ammo_belt"),
        Some("sound/misc/am_pkup.wav"),
        [Some("models/powerups/ammo/chaingunam.md3"), None, None, None],
        Some("icons/icona_chaingun"),
        Some("Chaingun Belt"),
        100,
        "",
        "",
        ItemKind::Ammo(Weapon::WpChaingun),
    ),
    def(
        Some("item_scout"),
        Some("sound/items/scout.wav"),
        [Some("models/powerups/scout.md3"), None, None, None],
        Some("icons/scout"),
        Some("Scout"),
        30,
        "",
        "",
        ItemKind::PersistantPowerup(Powerup::PwScout),
    ),
    def(
        Some("item_guard"),
        Some("sound/items/guard.wav"),
        [Some("models/powerups/guard.md3"), None, None, None],
        Some("icons/guard"),
        Some("Guard"),
        30,
        "",
        "",
        ItemKind::PersistantPowerup(Powerup::PwGuard),
    ),
    def(
        Some("item_doubler"),
        Some("sound/items/doubler.wav"),
        [Some("models/powerups/doubler.md3"), None, None, None],
        Some("icons/doubler"),
        Some("Doubler"),
        30,
        "",
        "",
        ItemKind::PersistantPowerup(Powerup::PwDoubler),
    ),
    def(
        Some("item_ammoregen"),
        Some("sound/items/ammoregen.wav"),
        [Some("models/powerups/ammo.md3"), None, None, None],
        Some("icons/ammo_regen"),
        Some("Ammo Regen"),
        30,
        "",
        "",
        ItemKind::PersistantPowerup(Powerup::PwAmmoregen),
    ),
    def(
        Some("team_CTF_neutralflag"),
        None,
        [Some("models/flags/n_flag.md3"), None, None, None],
        Some("icons/iconf_neutral1"),
        Some("Neutral Flag"),
        0,
        "",
        "",
        ItemKind::Team(Powerup::PwNeutralflag),
    ),
    def(
        Some("item_redcube"),
        Some("sound/misc/am_pkup.wav"),
        [Some("models/powerups/orb/r_orb.md3"), None, None, None],
        Some("icons/iconh_rorb"),
        Some("Red Cube"),
        0,
        "",
        "",
        ItemKind::Team(Powerup::PwNone),
    ),
    def(
        Some("item_bluecube"),
        Some("sound/misc/am_pkup.wav"),
        [Some("models/powerups/orb/b_orb.md3"), None, None, None],
        Some("icons/iconh_borb"),
        Some("Blue Cube"),
        0,
        "",
        "",
        ItemKind::Team(Powerup::PwNone),
    ),
    def(
        Some("weapon_nailgun"),
        Some("sound/misc/w_pkup.wav"),
        [Some("models/weapons/nailgun/nailgun.md3"), None, None, None],
        Some("icons/iconw_nailgun"),
        Some("Nailgun"),
        10,
        "",
        "",
        ItemKind::Weapon(Weapon::WpNailgun),
    ),
    def(
        Some("weapon_prox_launcher"),
        Some("sound/misc/w_pkup.wav"),
        [Some("models/weapons/proxmine/proxmine.md3"), None, None, None],
        Some("icons/iconw_proxlauncher"),
        Some("Prox Launcher"),
        5,
        "",
        "sound/weapons/proxmine/wstbtick.wav sound/weapons/proxmine/wstbactv.wav sound/weapons/proxmine/wstbimpl.wav sound/weapons/proxmine/wstbimpm.wav sound/weapons/proxmine/wstbimpd.wav sound/weapons/proxmine/wstbactv.wav",
        ItemKind::Weapon(Weapon::WpProxLauncher),
    ),
    def(
        Some("weapon_chaingun"),
        Some("sound/misc/w_pkup.wav"),
        [Some("models/weapons/vulcan/vulcan.md3"), None, None, None],
        Some("icons/iconw_chaingun"),
        Some("Chaingun"),
        80,
        "",
        "sound/weapons/vulcan/wvulwind.wav",
        ItemKind::Weapon(Weapon::WpChaingun),
    ),
];

/// Item list for a product: the base game sees the first 36 entries.
#[must_use]
pub fn item_list(product: Product) -> &'static [ItemDefinition] {
    match product {
        Product::Baseq3 => &DEFINITIONS[..36],
        Product::Missionpack => &DEFINITIONS[..],
    }
}

/// Item at an index, or an out-of-range error.
pub fn item_at(product: Product, index: i32) -> Result<&'static ItemDefinition, ItemsError> {
    usize::try_from(index)
        .ok()
        .and_then(|index| item_list(product).get(index))
        .ok_or_else(|| ItemsError::OutOfRange(format!("Item index out of range: {index}")))
}

/// Find an item by pickup name, folding ASCII case like the donor.
#[must_use]
pub fn find_item(product: Product, pickup_name: &str) -> Option<&'static ItemDefinition> {
    let folded = pickup_name.to_ascii_lowercase();
    item_list(product)
        .iter()
        .find(|item| item.pickup_name.is_some_and(|name| name.to_ascii_lowercase() == folded))
}

/// Find the item granting a powerup.
#[must_use]
pub fn find_item_for_powerup(product: Product, powerup: Powerup) -> Option<&'static ItemDefinition> {
    item_list(product).iter().find(|item| {
        matches!(
            item.kind,
            ItemKind::Powerup(tag) | ItemKind::Team(tag) | ItemKind::PersistantPowerup(tag) if tag == powerup
        )
    })
}

/// Find the item granting a holdable.
pub fn find_item_for_holdable(product: Product, holdable: Holdable) -> Result<&'static ItemDefinition, ItemsError> {
    item_list(product)
        .iter()
        .find(|item| item.kind == ItemKind::Holdable(holdable))
        .ok_or_else(|| ItemsError::Drop("HoldableItem not found".to_string()))
}

/// Find the item granting a weapon.
pub fn find_item_for_weapon(product: Product, weapon: Weapon) -> Result<&'static ItemDefinition, ItemsError> {
    item_list(product)
        .iter()
        .find(|item| item.kind == ItemKind::Weapon(weapon))
        .ok_or_else(|| ItemsError::Drop(format!("Couldn't find item for weapon {}", weapon as i32)))
}

/// Pickup entity fields read by the grab check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupEntity {
    /// Item model index.
    pub model_index: i32,
    /// Second model index.
    pub model_index2: i32,
    /// Entity flags word.
    pub generic1: i32,
}

/// Player inventory read by the grab check.
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
    /// Persistant-powerup item index (read only for the mission pack).
    fn persistent_powerup_index(&self) -> i32 {
        0
    }
}

/// Whether armor can be grabbed (`canQ3ArmorBeGrabbed`).
pub fn can_q3_armor_be_grabbed(player: &dyn PlayerInventory) -> Result<bool, ItemsError> {
    if player.product() == Product::Missionpack {
        if item_at(player.product(), player.persistent_powerup_index())?.tag() == Powerup::PwScout as i32 {
            return Ok(false);
        }
        let upper_bound =
            if item_at(player.product(), player.persistent_powerup_index())?.tag() == Powerup::PwGuard as i32 {
                player.max_health()
            } else {
                player.max_health() * 2
            };
        return Ok(player.armor() < upper_bound);
    }
    Ok(player.armor() < player.max_health() * 2)
}

/// Whether an item can be grabbed (`BG_CanItemBeGrabbed`).
pub fn can_item_be_grabbed(
    gametype: i32,
    entity: &PickupEntity,
    player: &dyn PlayerInventory,
) -> Result<bool, ItemsError> {
    if entity.model_index < 1 || entity.model_index as usize >= item_list(player.product()).len() {
        return Err(ItemsError::Drop("BG_CanItemBeGrabbed: index out of range".to_string()));
    }
    let item = item_at(player.product(), entity.model_index)?;
    match item.kind {
        ItemKind::Weapon(_) => Ok(true),
        ItemKind::Ammo(weapon) => Ok(player.ammo(weapon) < 200),
        ItemKind::Armor => can_q3_armor_be_grabbed(player),
        ItemKind::Health => {
            if player.product() == Product::Missionpack
                && item_at(player.product(), player.persistent_powerup_index())?.tag() == Powerup::PwGuard as i32
            {
                return Ok(player.health() < player.max_health());
            }
            Ok(player.health()
                < player.max_health()
                    * if item.quantity == 5 || item.quantity == 100 {
                        2
                    } else {
                        1
                    })
        }
        ItemKind::Powerup(_) => Ok(true),
        ItemKind::PersistantPowerup(_) => {
            if player.product() == Product::Baseq3 || player.persistent_powerup_index() != 0 {
                return Ok(false);
            }
            if entity.generic1 & 2 != 0 && player.team() != Team::TeamRed as i32 {
                return Ok(false);
            }
            if entity.generic1 & 4 != 0 && player.team() != Team::TeamBlue as i32 {
                return Ok(false);
            }
            Ok(true)
        }
        ItemKind::Team(powerup) => {
            if player.product() == Product::Missionpack && gametype == GameType::Gt1fctf as i32 {
                if powerup == Powerup::PwNeutralflag {
                    return Ok(true);
                }
                if player.team() == Team::TeamRed as i32
                    && powerup == Powerup::PwBlueflag
                    && player.powerup(Powerup::PwNeutralflag) != 0
                {
                    return Ok(true);
                }
                if player.team() == Team::TeamBlue as i32
                    && powerup == Powerup::PwRedflag
                    && player.powerup(Powerup::PwNeutralflag) != 0
                {
                    return Ok(true);
                }
            }
            if gametype == GameType::GtCtf as i32 {
                if player.team() == Team::TeamRed as i32 {
                    return Ok(powerup == Powerup::PwBlueflag
                        || (powerup == Powerup::PwRedflag
                            && (entity.model_index2 != 0 || player.powerup(Powerup::PwBlueflag) != 0)));
                }
                if player.team() == Team::TeamBlue as i32 {
                    return Ok(powerup == Powerup::PwRedflag
                        || (powerup == Powerup::PwBlueflag
                            && (entity.model_index2 != 0 || player.powerup(Powerup::PwRedflag) != 0)));
                }
            }
            Ok(player.product() == Product::Missionpack && gametype == GameType::GtHarvester as i32)
        }
        ItemKind::Holdable(_) => Ok(player.holdable_item() == 0),
        ItemKind::Bad => Err(ItemsError::Drop("BG_CanItemBeGrabbed: IT_BAD".to_string())),
    }
}

/// Whether a player touches an item's pickup bounds.
pub fn player_touches_item(player_origin: Vec3, item_position: &Trajectory, at_time: i32) -> Result<bool, ItemsError> {
    let origin = evaluate_trajectory(item_position, at_time)?;
    let x = player_origin.x - origin.x;
    let y = player_origin.y - origin.y;
    let z = player_origin.z - origin.z;
    Ok(!(x > 44.0 || x < -50.0 || y > 36.0 || y < -36.0 || z > 36.0 || z < -36.0))
}

// ---------------------------------------------------------------------------
// Trajectory evaluation (`BG_EvaluateTrajectory`).
//
// UNION PENDING: this mirrors sibling-owned
// `src/content/q3/base/shared/trajectory.ts`, which this file's
// `playerTouchesItem` depends on. At merge, delete this section and rewire
// `player_touches_item` to `super::trajectory::{Trajectory,
// evaluate_trajectory}`. The mirror keeps the donor's exact contract (open
// `i32` type tag, `Result`, donor error text) so the rewire is mechanical.
// ---------------------------------------------------------------------------

/// Trajectory type (`TrajectoryType`, `trType_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum TrajectoryType {
    /// Stationary.
    TrStationary = 0,
    /// Interpolated.
    TrInterpolate = 1,
    /// Linear.
    TrLinear = 2,
    /// Linear stop.
    TrLinearStop = 3,
    /// Sine.
    TrSine = 4,
    /// Gravity.
    TrGravity = 5,
}

/// Trajectory record. The type tag stays an open `i32` like the donor's
/// `Trajectory<number>` so unknown source values keep failing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trajectory {
    /// Trajectory type tag.
    pub trajectory_type: i32,
    /// Start time in milliseconds.
    pub time: i32,
    /// Duration in milliseconds.
    pub duration: i32,
    /// Base position.
    pub base: Vec3,
    /// Delta vector.
    pub delta: Vec3,
}

fn trajectory_seconds(milliseconds: i32) -> f32 {
    milliseconds as f32 * 0.001
}

fn trajectory_periodic_radians(trajectory: &Trajectory, at_time: i32) -> f32 {
    let fraction = at_time.wrapping_sub(trajectory.time) as f32 / (trajectory.duration as f32);
    (fraction * std::f32::consts::PI) * 2.0
}

/// Evaluate a trajectory at a millisecond time (`BG_EvaluateTrajectory`).
fn evaluate_trajectory(trajectory: &Trajectory, at_time: i32) -> Result<Vec3, ItemsError> {
    match trajectory.trajectory_type {
        x if x == TrajectoryType::TrStationary as i32 || x == TrajectoryType::TrInterpolate as i32 => {
            Ok(vec3(trajectory.base.x, trajectory.base.y, trajectory.base.z))
        }
        x if x == TrajectoryType::TrLinear as i32 => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(trajectory.time));
            Ok(add3(trajectory.base, scale3(trajectory.delta, delta_time)))
        }
        x if x == TrajectoryType::TrSine as i32 => {
            let phase = f64::from(trajectory_periodic_radians(trajectory, at_time)).sin() as f32;
            Ok(add3(trajectory.base, scale3(trajectory.delta, phase)))
        }
        x if x == TrajectoryType::TrLinearStop as i32 => {
            let end = trajectory.time.wrapping_add(trajectory.duration);
            let time = if at_time > end { end } else { at_time };
            let delta_time = trajectory_seconds(time.wrapping_sub(trajectory.time)).max(0.0);
            Ok(add3(trajectory.base, scale3(trajectory.delta, delta_time)))
        }
        x if x == TrajectoryType::TrGravity as i32 => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(trajectory.time));
            let result = add3(trajectory.base, scale3(trajectory.delta, delta_time));
            let fall = (0.5 * DEFAULT_GRAVITY as f32 * delta_time) * delta_time;
            Ok(vec3(result.x, result.y, result.z - fall))
        }
        _ => Err(ItemsError::Drop(format!(
            "BG_EvaluateTrajectory: unknown trType: {}",
            trajectory.time
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestInventory {
        product: Product,
        health: i32,
        armor: i32,
        max_health: i32,
        holdable_item: i32,
        team: i32,
        ammo: i32,
        powerup: i32,
        persistent_powerup_index: i32,
    }

    impl TestInventory {
        fn base() -> Self {
            Self {
                product: Product::Baseq3,
                health: 100,
                armor: 0,
                max_health: 100,
                holdable_item: 0,
                team: Team::TeamFree as i32,
                ammo: 0,
                powerup: 0,
                persistent_powerup_index: 0,
            }
        }
    }

    impl PlayerInventory for TestInventory {
        fn product(&self) -> Product {
            self.product
        }
        fn health(&self) -> i32 {
            self.health
        }
        fn armor(&self) -> i32 {
            self.armor
        }
        fn max_health(&self) -> i32 {
            self.max_health
        }
        fn holdable_item(&self) -> i32 {
            self.holdable_item
        }
        fn team(&self) -> i32 {
            self.team
        }
        fn ammo(&self, _weapon: Weapon) -> i32 {
            self.ammo
        }
        fn powerup(&self, _powerup: Powerup) -> i32 {
            self.powerup
        }
        fn persistent_powerup_index(&self) -> i32 {
            self.persistent_powerup_index
        }
    }

    fn pickup(model_index: i32) -> PickupEntity {
        PickupEntity {
            model_index,
            model_index2: 0,
            generic1: 0,
        }
    }

    #[test]
    fn item_lists_cover_both_products() {
        assert_eq!(item_list(Product::Baseq3).len(), 36);
        assert_eq!(item_list(Product::Missionpack).len(), 52);
        assert_eq!(item_at(Product::Baseq3, 0).unwrap().kind, ItemKind::Bad);
        assert_eq!(item_at(Product::Missionpack, 51).unwrap().pickup_name, Some("Chaingun"));
        assert!(matches!(item_at(Product::Baseq3, 36), Err(ItemsError::OutOfRange(_))));
        assert!(matches!(item_at(Product::Baseq3, -1), Err(ItemsError::OutOfRange(_))));
    }

    #[test]
    fn item_kinds_carry_types_and_tags() {
        let gauntlet = item_at(Product::Baseq3, 8).unwrap();
        assert_eq!(gauntlet.item_type(), ItemType::ItWeapon);
        assert_eq!(gauntlet.tag(), Weapon::WpGauntlet as i32);
        let red = item_at(Product::Baseq3, 34).unwrap();
        assert_eq!(red.item_type(), ItemType::ItTeam);
        assert_eq!(red.tag(), Powerup::PwRedflag as i32);
        let cube = item_at(Product::Missionpack, 47).unwrap();
        assert_eq!(cube.item_type(), ItemType::ItTeam);
        assert_eq!(cube.tag(), 0);
        let holdable = item_at(Product::Baseq3, 26).unwrap();
        assert_eq!(holdable.item_type(), ItemType::ItHoldable);
        assert_eq!(holdable.tag(), Holdable::HiTeleporter as i32);
    }

    #[test]
    fn item_lookups_match_donor_rules() {
        assert_eq!(
            find_item(Product::Baseq3, "shotgun").unwrap().tag(),
            Weapon::WpShotgun as i32
        );
        assert_eq!(
            find_item(Product::Baseq3, "SHOTGUN").unwrap().tag(),
            Weapon::WpShotgun as i32
        );
        assert!(find_item(Product::Baseq3, "no such item").is_none());
        assert_eq!(
            find_item_for_powerup(Product::Baseq3, Powerup::PwQuad)
                .unwrap()
                .pickup_name,
            Some("Quad Damage")
        );
        assert!(find_item_for_powerup(Product::Baseq3, Powerup::PwScout).is_none());
        assert_eq!(
            find_item_for_holdable(Product::Baseq3, Holdable::HiMedkit)
                .unwrap()
                .pickup_name,
            Some("Medkit")
        );
        assert!(find_item_for_holdable(Product::Baseq3, Holdable::HiKamikaze).is_err());
        assert_eq!(
            find_item_for_weapon(Product::Missionpack, Weapon::WpChaingun)
                .unwrap()
                .pickup_name,
            Some("Chaingun")
        );
        assert!(find_item_for_weapon(Product::Baseq3, Weapon::WpChaingun).is_err());
    }

    #[test]
    fn grab_checks_follow_bg_rules() {
        let player = TestInventory::base();
        assert!(can_item_be_grabbed(0, &pickup(9), &player).unwrap());
        let mut full_ammo = TestInventory::base();
        full_ammo.ammo = 200;
        assert!(!can_item_be_grabbed(0, &pickup(18), &full_ammo).unwrap());
        assert!(can_item_be_grabbed(0, &pickup(18), &player).unwrap());
        let mut hurt = TestInventory::base();
        hurt.health = 50;
        assert!(can_item_be_grabbed(0, &pickup(5), &hurt).unwrap());
        assert!(!can_item_be_grabbed(0, &pickup(5), &player).unwrap());
        assert!(can_item_be_grabbed(0, &pickup(28), &player).unwrap());
        let mut holding = TestInventory::base();
        holding.holdable_item = 1;
        assert!(!can_item_be_grabbed(0, &pickup(26), &holding).unwrap());
        assert!(can_item_be_grabbed(0, &pickup(26), &player).unwrap());
        assert!(can_item_be_grabbed(0, &pickup(0), &player).is_err());
        assert!(can_item_be_grabbed(0, &pickup(36), &player).is_err());
    }

    #[test]
    fn grab_checks_cover_armor_and_teams() {
        let player = TestInventory::base();
        assert!(can_item_be_grabbed(0, &pickup(2), &player).unwrap());
        let mut armored = TestInventory::base();
        armored.armor = 200;
        assert!(!can_item_be_grabbed(0, &pickup(2), &armored).unwrap());
        let mut red = TestInventory::base();
        red.team = Team::TeamRed as i32;
        assert!(can_item_be_grabbed(GameType::GtCtf as i32, &pickup(35), &red).unwrap());
        assert!(!can_item_be_grabbed(GameType::GtCtf as i32, &pickup(34), &red).unwrap());
        let mut pack = TestInventory::base();
        pack.product = Product::Missionpack;
        assert!(can_item_be_grabbed(GameType::GtHarvester as i32, &pickup(47), &pack).unwrap());
        assert!(!can_item_be_grabbed(GameType::GtFfa as i32, &pickup(47), &pack).unwrap());
        let mut scout = TestInventory::base();
        scout.product = Product::Missionpack;
        scout.persistent_powerup_index = 42;
        assert!(!can_item_be_grabbed(0, &pickup(2), &scout).unwrap());
    }

    #[test]
    fn touch_checks_use_trajectory_origins() {
        let stationary = Trajectory {
            trajectory_type: TrajectoryType::TrStationary as i32,
            time: 0,
            duration: 0,
            base: vec3(100.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 0.0),
        };
        assert!(player_touches_item(vec3(100.0, 0.0, 0.0), &stationary, 0).unwrap());
        assert!(!player_touches_item(vec3(200.0, 0.0, 0.0), &stationary, 0).unwrap());
        let linear = Trajectory {
            trajectory_type: TrajectoryType::TrLinear as i32,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(100.0, 0.0, 0.0),
        };
        assert!(player_touches_item(vec3(100.0, 0.0, 0.0), &linear, 1000).unwrap());
        let gravity = Trajectory {
            trajectory_type: TrajectoryType::TrGravity as i32,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 100.0),
            delta: vec3(0.0, 0.0, 0.0),
        };
        let origin = evaluate_trajectory(&gravity, 1000).unwrap();
        assert_eq!(origin, vec3(0.0, 0.0, 100.0 - 400.0));
        let unknown = Trajectory {
            trajectory_type: 99,
            time: 7,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 0.0),
        };
        assert_eq!(
            player_touches_item(vec3(0.0, 0.0, 0.0), &unknown, 0),
            Err(ItemsError::Drop("BG_EvaluateTrajectory: unknown trType: 7".to_string()))
        );
    }
}
