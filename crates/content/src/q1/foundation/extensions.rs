//! Source extension points (`src/content/q1/foundation/extensions.ts`).
//!
//! Source modules own ammunition mutations, attack timing, and weapon
//! effects. Hooks receive the game plus subject ids and resolve live
//! records through the game.

use qa_core::identity::ActorId;

use crate::contract::ItemId;
use crate::q1::Q1Error;

use super::entity_services::Q1EntityServices;
use super::types::Q1Weapon;

/// Game hook.
pub type Q1GameHook<T> = fn(game: &mut Q1EntityServices) -> T;
/// Player hook.
pub type Q1PlayerHook<T> = fn(game: &mut Q1EntityServices, player: &ActorId) -> T;
/// Player hook with an item argument.
pub type Q1PlayerItemHook<T> = fn(game: &mut Q1EntityServices, player: &ActorId, item: &ItemId) -> T;
/// Player hook with a seconds argument.
pub type Q1PlayerSecondsHook<T> = fn(game: &mut Q1EntityServices, player: &ActorId, seconds: f64) -> T;
/// Player hook over travel bytes.
pub type Q1PlayerBytesHook<T> = fn(game: &mut Q1EntityServices, player: &ActorId, bytes: &[u8]) -> T;
/// Player hook with a weapon argument.
pub type Q1PlayerWeaponHook<T> = fn(game: &mut Q1EntityServices, player: &ActorId, weapon: Q1Weapon) -> T;
/// Player hook with a weapon and default amount.
pub type Q1PlayerWeaponAmountHook<T> =
    fn(game: &mut Q1EntityServices, player: &ActorId, weapon: Q1Weapon, default_amount: f64) -> T;
/// Player hook with a flag argument.
pub type Q1PlayerFlagHook<T> = fn(game: &mut Q1EntityServices, player: &ActorId, flag: bool) -> T;
/// Entity hook with a default-seconds argument.
pub type Q1EntitySecondsHook<T> = fn(game: &mut Q1EntityServices, entity: &ActorId, default_seconds: f64) -> T;
/// Player hook with an item and amount.
pub type Q1PlayerItemAmountHook<T> = fn(game: &mut Q1EntityServices, player: &ActorId, item: &ItemId, amount: f64) -> T;
/// Player hook with a delay argument.
pub type Q1PlayerDelayHook<T> = fn(game: &mut Q1EntityServices, player: &ActorId, delay: f64) -> T;
/// Player hook with a speed argument.
pub type Q1PlayerSpeedHook<T> = fn(game: &mut Q1EntityServices, player: &ActorId, speed: f64) -> T;
/// Weapon rank hook.
pub type Q1WeaponRankHook = fn(weapon: Q1Weapon) -> i32;

/// Registered source weapon definition (`Q1WeaponDefinition`).
pub struct Q1WeaponDefinition {
    /// Weapon id.
    pub id: Q1Weapon,
    /// Inventory item override.
    pub item: Option<ItemId>,
    /// Ammunition item, if any.
    pub ammo: Option<ItemId>,
    /// Ammunition per shot.
    pub ammo_per_shot: Option<f64>,
    /// View model path.
    pub model: String,
    /// Selection rank.
    pub rank: i32,
    /// Per-player model override.
    pub model_for: Option<Q1PlayerHook<Result<String, Q1Error>>>,
    /// Availability check.
    pub available: Option<Q1PlayerHook<Result<bool, Q1Error>>>,
    /// Best-weapon availability check.
    pub best_available: Option<Q1PlayerHook<Result<bool, Q1Error>>>,
    /// Fire implementation.
    pub fire: Q1PlayerHook<Result<bool, Q1Error>>,
    /// Per-frame animation.
    pub animate: Option<Q1PlayerSecondsHook<Result<(), Q1Error>>>,
}

impl std::fmt::Debug for Q1WeaponDefinition {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Q1WeaponDefinition")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Player behavior extension (`Q1PlayerExtension`).
#[derive(Debug, Default)]
pub struct Q1PlayerExtension {
    /// Extension id.
    pub id: String,
    /// New-player admission hook. Saved inventories and private state
    /// restore through their owners.
    pub attach: Option<Q1PlayerHook<Result<(), Q1Error>>>,
    /// Inventory capacity override.
    pub inventory_capacity: Option<Q1PlayerItemHook<Result<Option<f64>, Q1Error>>>,
    /// Source prethink hook running before ordinary environment and
    /// timed-effect handling.
    pub frame: Option<Q1PlayerSecondsHook<Result<(), Q1Error>>>,
    /// Post-physics hook.
    pub after_physics: Option<Q1PlayerSecondsHook<Result<(), Q1Error>>>,
    /// Travel capture hook. Source spawn parameters survive level
    /// changes independently of ordinary equipment resets.
    pub capture_travel: Option<Q1PlayerHook<Vec<u8>>>,
    /// Travel restore hook.
    pub restore_travel: Option<Q1PlayerBytesHook<Result<(), Q1Error>>>,
}

/// Pickup rules override (`Q1PickupRules`).
#[derive(Debug, Default)]
pub struct Q1PickupRules {
    /// Rules id.
    pub id: String,
    /// Whether picked-up weapons stay in the world.
    pub weapon_leave: Option<Q1GameHook<Result<bool, Q1Error>>>,
    /// Weapon grant override.
    pub weapon_granted: Option<Q1PlayerWeaponHook<Result<Q1Weapon, Q1Error>>>,
    /// Weapon ammo grant override.
    pub weapon_ammo_grant: Option<Q1PlayerWeaponAmountHook<Result<f64, Q1Error>>>,
    /// Weapon rank override.
    pub weapon_rank: Option<Q1WeaponRankHook>,
    /// Automatic switch decision.
    pub auto_switch: Option<Q1PlayerFlagHook<Result<bool, Q1Error>>>,
    /// Respawn interval override.
    pub respawn: Option<Q1EntitySecondsHook<Result<f64, Q1Error>>>,
}

/// Weapon rules override (`Q1WeaponRules`).
#[derive(Debug, Default)]
pub struct Q1WeaponRules {
    /// Rules id.
    pub id: String,
    /// Ammunition consumption override.
    pub consume_ammo: Option<Q1PlayerItemAmountHook<Result<bool, Q1Error>>>,
    /// Pre-fire hook.
    pub before_fire: Option<Q1PlayerHook<Result<(), Q1Error>>>,
    /// Frame delay override.
    pub frame_delay: Option<Q1PlayerDelayHook<Result<f64, Q1Error>>>,
    /// Attack delay override.
    pub attack_delay: Option<Q1PlayerDelayHook<Result<f64, Q1Error>>>,
    /// Nail speed override.
    pub nail_speed: Option<Q1PlayerSpeedHook<Result<f64, Q1Error>>>,
}
