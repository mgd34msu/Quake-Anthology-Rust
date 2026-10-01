//! Q2 base entity hooks (`src/content/q2/base/entities/types.ts`).
//!
//! Integrations owned by the existing movement, player, monster and
//! presentation providers.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q2::foundation::host::Q2GameServices;
use crate::q2::foundation::monsters::types::MonsterContext;
use crate::q2::foundation::movers::Q2MoverModule;

/// Local clock time (`localTime` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2LocalTime {
    /// Hour.
    pub hour: i32,
    /// Minute.
    pub minute: i32,
    /// Second.
    pub second: i32,
}

/// Blaster hook (`Q2Ballistics[fireBlaster]`).
pub type Q2FireBlasterHook = fn(ActorId, &mut Q2GameServices, Vec3, Vec3, f64, f64, i64, bool, i32) -> ActorId;

/// Rocket hook (`Q2Ballistics[fireRocket]`).
pub type Q2FireRocketHook = fn(ActorId, &mut Q2GameServices, Vec3, Vec3, f64, f64, f64, f64) -> ActorId;

/// Teleport hook (`teleportPlayer`).
pub type Q2TeleportPlayerHook = fn(ActorId, Vec3, Vec3);

/// Push hook (`playerPush`).
pub type Q2PlayerPushHook = fn(ActorId, Vec3);

/// Gravity hook (`setActorGravity`).
pub type Q2SetActorGravityHook = fn(ActorId, f64);

/// Local-time hook (`localTime`).
pub type Q2LocalTimeHook = fn() -> Q2LocalTime;

/// Turret-driver admission hook (`turretDriver`).
///
/// Admits the turret driver's infantry state to the permanent monster
/// runner and returns the transient context. The driver state does not
/// retain the context; later thinks rebuild it on demand.
pub type Q2TurretDriverHook = for<'a> fn(ActorId, &'a mut Q2GameServices) -> MonsterContext<'a>;

/// Restored-monster lookup hook (`monsterContext`).
///
/// Looks up an already restored monster; must not admit or spawn one.
pub type Q2MonsterLookupHook = fn(&Q2GameServices, &ActorId) -> bool;

/// Monster resume hook (`resumeMonster`).
pub type Q2ResumeMonsterHook = fn(ActorId, &mut Q2GameServices);

/// Base entity hooks (`Q2BaseEntityHooks`).
#[derive(Debug, Clone, Copy)]
pub struct Q2BaseEntityHooks {
    /// Foundation mover module.
    pub movers: Q2MoverModule,
    /// Blaster ballistics.
    pub fire_blaster: Q2FireBlasterHook,
    /// Rocket ballistics.
    pub fire_rocket: Q2FireRocketHook,
    /// Teleport a player.
    pub teleport_player: Q2TeleportPlayerHook,
    /// Push a player.
    pub player_push: Q2PlayerPushHook,
    /// Set actor gravity.
    pub set_actor_gravity: Q2SetActorGravityHook,
    /// Read the local clock.
    pub local_time: Q2LocalTimeHook,
    /// Admit a turret driver.
    pub turret_driver: Q2TurretDriverHook,
    /// Look up a restored monster.
    pub monster_context: Q2MonsterLookupHook,
    /// Resume a monster.
    pub resume_monster: Q2ResumeMonsterHook,
}
