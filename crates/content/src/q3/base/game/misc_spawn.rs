//! Quake III base/game: misc spawn.
//!
//! Donor provenance: `src/content/q3/base/game/misc-spawn.ts`.

use qa_core::math::{add3, cross3, normalize3, perpendicular_vector, scale3, sub3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::items_core::*;
use crate::q3::base::game::misc::*;
use crate::q3::base::game::missile::*;
use crate::q3::base::game::utilities::move_direction;
use crate::q3::base::shared::definitions::{EntityEvent, Weapon};

// ---------------------------------------------------------------------------
// misc-spawn.ts: g_misc.c spawn wrappers and shooters
// ---------------------------------------------------------------------------

/// Shooter use name.
pub const MISC_SHOOTER_USE: &str = "q3.base.game.misc-spawn.shooter.use";

/// Shooter think name.
pub const MISC_SHOOTER_THINK: &str = "q3.base.game.misc-spawn.shooter.think";

/// Misc spawn host (`MiscSpawnHost`).
pub struct MiscSpawnHost<'a> {
    /// Missile runtime.
    pub missiles: &'a mut dyn MissileFire,
    /// Missile host services.
    pub missile_host: &'a mut dyn MissileHost,
    /// Projectile driver.
    pub missile_driver: &'a mut dyn ProjectileDriver,
    /// Item registry.
    pub item_registry: &'a mut dyn ItemRegistry,
    /// Item table.
    pub items: &'a dyn ItemTable,
    /// Game random.
    pub random: &'a mut dyn GameRandom,
    /// World host.
    pub world: &'a mut dyn WorldOps,
    /// Current time.
    pub time: i32,
    /// Warning sink.
    pub warn: &'a mut dyn FnMut(&str),
}

pub(crate) fn check_misc_spawn_host(host: &mut MiscSpawnHost<'_>) -> Q3GameItemsResult<()> {
    if host.item_registry.product() != host.missile_host.combat().product() {
        return Err(invalid("misc spawn item registry does not match its missile product"));
    }
    Ok(())
}

/// Misc spawn handler selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MiscSpawn {
    /// `info_camp`.
    InfoCamp,
    /// `info_null`.
    InfoNull,
    /// `info_notnull`.
    InfoNotNull,
    /// `light`.
    Light,
    /// `misc_teleporter_dest`.
    TeleporterDestination,
    /// `misc_model`.
    Model,
    /// `misc_portal_surface`.
    PortalSurface,
    /// `misc_portal_camera`.
    PortalCamera,
    /// Weapon shooter.
    Shooter(Weapon),
}

/// Spawn table entries (`miscSpawnHandlers`).
#[must_use]
pub fn misc_spawn_handlers() -> Vec<(&'static str, MiscSpawn)> {
    vec![
        ("info_camp", MiscSpawn::InfoCamp),
        ("info_null", MiscSpawn::InfoNull),
        ("info_notnull", MiscSpawn::InfoNotNull),
        ("light", MiscSpawn::Light),
        ("misc_teleporter_dest", MiscSpawn::TeleporterDestination),
        ("misc_model", MiscSpawn::Model),
        ("misc_portal_surface", MiscSpawn::PortalSurface),
        ("misc_portal_camera", MiscSpawn::PortalCamera),
        ("shooter_rocket", MiscSpawn::Shooter(Weapon::WpRocketLauncher)),
        ("shooter_plasma", MiscSpawn::Shooter(Weapon::WpPlasmagun)),
        ("shooter_grenade", MiscSpawn::Shooter(Weapon::WpGrenadeLauncher)),
    ]
}

/// Run a misc spawn handler.
pub fn run_misc_spawn(
    pool: &mut EntityPool,
    host: &mut MiscSpawnHost<'_>,
    handler: &MiscSpawn,
    slot: Slot,
    variables: &SpawnVariables,
) -> Q3GameItemsResult<()> {
    check_misc_spawn_host(host)?;
    bind_misc_save_callbacks(pool);
    match *handler {
        MiscSpawn::InfoCamp => info_camp(pool, slot),
        MiscSpawn::InfoNull => info_null(pool, slot),
        MiscSpawn::InfoNotNull => info_notnull(pool, slot),
        MiscSpawn::Light => misc_light(pool, slot),
        MiscSpawn::TeleporterDestination => teleporter_destination(pool, slot),
        MiscSpawn::Model => misc_model(pool, slot),
        MiscSpawn::PortalSurface => misc_portal_surface(pool, host, slot),
        MiscSpawn::PortalCamera => misc_portal_camera(pool, host, slot, variables),
        MiscSpawn::Shooter(weapon) => shooter(pool, host, slot, weapon),
    }
}

pub(crate) fn info_camp(pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let origin = pool.at(slot)?.s.origin;
    set_origin(pool, slot, origin)
}

pub(crate) fn info_null(pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    pool.free(slot)
}

pub(crate) fn info_notnull(pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let origin = pool.at(slot)?.s.origin;
    set_origin(pool, slot, origin)
}

pub(crate) fn misc_light(pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    pool.free(slot)
}

pub(crate) fn teleporter_destination(pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    Ok(())
}

pub(crate) fn misc_model(pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    pool.free(slot)
}

pub(crate) fn misc_portal_surface(
    pool: &mut EntityPool,
    host: &mut MiscSpawnHost<'_>,
    slot: Slot,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    spawn_portal_surface(pool, host.world, host.time, slot)
}

pub(crate) fn misc_portal_camera(
    pool: &mut EntityPool,
    host: &mut MiscSpawnHost<'_>,
    slot: Slot,
    variables: &SpawnVariables,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let roll = variables.float("roll", "0").value;
    spawn_portal_camera(pool, host.world, slot, roll)
}

pub(crate) fn shooter_crandom(host: &mut MiscSpawnHost<'_>) -> Q3GameItemsResult<f32> {
    let value = host.random.crandom();
    if !value.is_finite() || value < -1.0 || value > 1.0 {
        return Err(range("shooter crandom() must return a value within [-1, 1]"));
    }
    Ok(value)
}

pub(crate) fn shooter_fire(
    pool: &mut EntityPool,
    host: &mut MiscSpawnHost<'_>,
    slot: Slot,
    direction: MissileDirection,
) -> Q3GameItemsResult<()> {
    let weapon = pool.at(slot)?.s.weapon;
    let start = pool.at(slot)?.s.origin;
    let mut direction = direction;
    match weapon {
        Weapon::WpGrenadeLauncher => {
            host.missiles.fire_grenade(
                pool,
                host.missile_host,
                host.missile_driver,
                slot,
                start,
                &mut direction,
            )?;
        }
        Weapon::WpRocketLauncher => {
            host.missiles.fire_rocket(
                pool,
                host.missile_host,
                host.missile_driver,
                slot,
                start,
                &mut direction,
            )?;
        }
        Weapon::WpPlasmagun => {
            host.missiles.fire_plasma(
                pool,
                host.missile_host,
                host.missile_driver,
                slot,
                start,
                &mut direction,
            )?;
        }
        _ => {}
    }
    Ok(())
}

/// Shooter use callback.
pub fn shooter_use(pool: &mut EntityPool, host: &mut MiscSpawnHost<'_>, slot: Slot) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let initial = match pool.at(slot)?.enemy {
        None => pool.at(slot)?.movedir,
        Some(enemy) => normalize3(sub3(pool.at(enemy)?.r.current_origin, pool.at(slot)?.s.origin)),
    };
    let up = perpendicular_vector(initial);
    let right = cross3(up, initial);
    let spread = pool.at(slot)?.random;
    let vertical = shooter_crandom(host)? * spread;
    let with_vertical = add3(initial, scale3(up, vertical));
    let horizontal = shooter_crandom(host)? * spread;
    let direction = normalize3(add3(with_vertical, scale3(right, horizontal)));
    shooter_fire(pool, host, slot, MissileDirection::from(direction))?;
    pool.add_event(slot, EntityEvent::EvFireWeapon, 0)
}

/// Shooter finish think (target lock).
pub fn shooter_finish(pool: &mut EntityPool, host: &mut MiscSpawnHost<'_>, slot: Slot) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let target = pool.at(slot)?.target.clone();
    let random: &mut dyn GameRandom = &mut *host.random;
    let warn: &mut dyn FnMut(&str) = &mut *host.warn;
    let mut pick = || random.rand_int();
    let mut selection = TargetSelection {
        random_int: &mut pick,
        warn,
    };
    let enemy = pick_target(pool, &mut selection, target.as_deref())?;
    pool.at_mut(slot)?.enemy = enemy;
    pool.at_mut(slot)?.think = None;
    pool.at_mut(slot)?.nextthink = 0;
    Ok(())
}

pub(crate) fn shooter(
    pool: &mut EntityPool,
    host: &mut MiscSpawnHost<'_>,
    slot: Slot,
    weapon: Weapon,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let use_cb = pool.use_cbs.resolve(MISC_SHOOTER_USE)?;
    pool.at_mut(slot)?.use_cb = Some(use_cb);
    pool.at_mut(slot)?.s.weapon = weapon;
    let product = pool.product();
    let item = host.items.find_item_for_weapon(product, weapon)?;
    host.item_registry.register(&item);
    let angles = pool.at(slot)?.s.angles;
    let (direction, cleared) = move_direction(angles);
    pool.at_mut(slot)?.movedir = direction;
    pool.at_mut(slot)?.s.angles = cleared;
    if pool.at(slot)?.random == 0.0 {
        pool.at_mut(slot)?.random = 1.0;
    }
    let radians = std::f32::consts::PI * pool.at(slot)?.random / 180.0;
    pool.at_mut(slot)?.random = (f64::from(radians).sin()) as f32;
    if pool.at(slot)?.target.is_some() {
        let think = pool.think_cbs.resolve(MISC_SHOOTER_THINK)?;
        pool.at_mut(slot)?.think = Some(think);
        pool.at_mut(slot)?.nextthink = host.time.wrapping_add(500);
    }
    pool.link(slot)?;
    Ok(())
}

/// Register misc-spawn save callbacks.
pub fn bind_misc_save_callbacks(pool: &mut EntityPool) {
    pool.use_cbs.intern(MISC_SHOOTER_USE);
    pool.think_cbs.intern(MISC_SHOOTER_THINK);
}

/// Dispatch a misc-spawn use callback; returns false when unhandled.
pub fn dispatch_misc_use(
    pool: &mut EntityPool,
    host: &mut MiscSpawnHost<'_>,
    slot: Slot,
    name: CallbackName,
) -> Q3GameItemsResult<bool> {
    if name != CallbackName(MISC_SHOOTER_USE) {
        return Ok(false);
    }
    shooter_use(pool, host, slot)?;
    Ok(true)
}

/// Dispatch a misc-spawn think callback; returns false when unhandled.
pub fn dispatch_misc_think(
    pool: &mut EntityPool,
    host: &mut MiscSpawnHost<'_>,
    slot: Slot,
    name: CallbackName,
) -> Q3GameItemsResult<bool> {
    if name != CallbackName(MISC_SHOOTER_THINK) {
        return Ok(false);
    }
    shooter_finish(pool, host, slot)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use qa_core::math::vec3;

    use super::*;
    use crate::q3::base::game::items_core::test_support::*;
    use crate::q3::base::game::missile::test_support::*;
    use crate::q3::base::shared::definitions::*;

    #[test]
    fn misc_spawns_cover_table() {
        let handlers = misc_spawn_handlers();
        assert_eq!(handlers.len(), 11);
        let mut pool = EntityPool::new(Product::Baseq3);
        let mut runtime = MissileRuntime::new();
        runtime.bind_save_callbacks(&mut pool);
        let mut missile_host = TestMissileHost::base();
        let mut driver = TestDriver::new();
        let mut registry = TestRegistry {
            product: Product::Baseq3,
            registered: Vec::new(),
        };
        let items = test_items();
        let mut random = TestRandom {
            int_value: 0,
            random_value: 0.5,
            crandom_value: 0.0,
        };
        let mut world = TestWorld::new();
        let mut warnings = Vec::new();
        let mut host = MiscSpawnHost {
            missiles: &mut runtime,
            missile_host: &mut missile_host,
            missile_driver: &mut driver,
            item_registry: &mut registry,
            items: &items,
            random: &mut random,
            world: &mut world,
            time: 1000,
            warn: &mut |text: &str| warnings.push(text.to_string()),
        };
        let vars = SpawnVariables::new(Vec::new());
        let shooter_slot = pool.spawn().unwrap();
        run_misc_spawn(
            &mut pool,
            &mut host,
            &MiscSpawn::Shooter(Weapon::WpRocketLauncher),
            shooter_slot,
            &vars,
        )
        .unwrap();
        assert_eq!(pool.at(shooter_slot).unwrap().s.weapon, Weapon::WpRocketLauncher);
        assert!((pool.at(shooter_slot).unwrap().random - (std::f32::consts::PI / 180.0).sin()).abs() < 1e-6);
        shooter_use(&mut pool, &mut host, shooter_slot).unwrap();
        assert!(pool.events.iter().any(|event| event.event == EntityEvent::EvFireWeapon));
        let null = pool.spawn().unwrap();
        run_misc_spawn(&mut pool, &mut host, &MiscSpawn::InfoNull, null, &vars).unwrap();
        assert!(pool.get(null).is_none());
        let camp = pool.spawn().unwrap();
        pool.at_mut(camp).unwrap().s.origin = vec3(7.0, 8.0, 9.0);
        run_misc_spawn(&mut pool, &mut host, &MiscSpawn::InfoCamp, camp, &vars).unwrap();
        assert_eq!(pool.at(camp).unwrap().s.pos.base, vec3(7.0, 8.0, 9.0));
        assert_eq!(driver.launches.len(), 1);
        assert_eq!(registry.registered.len(), 1);
        assert!(warnings.is_empty());
    }
}
