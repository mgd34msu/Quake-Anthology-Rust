//! Quake III base/game: misc.
//!
//! Donor provenance: `src/content/q3/base/game/misc.ts`.

use qa_core::math::{add3, normalize3, scale3, sub3, vec3, Bounds, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_items::*;

// ---------------------------------------------------------------------------
// misc.ts: teleport, portal, G_KillBox
// ---------------------------------------------------------------------------

pub(crate) const MOD_TELEFRAG: i32 = 18;

pub(crate) const EF_TELEPORT_BIT: i32 = 4;

/// Portal surface think name.
pub const PORTAL_SURFACE_THINK: &str = "q3.base.game.misc.spawnPortalSurface.think";

/// Teleport context (`TeleportContext`).
pub struct TeleportContext<'a> {
    /// Combat host.
    pub combat: &'a mut dyn CombatOps,
    /// World host.
    pub world: &'a mut dyn WorldOps,
}

/// Kill players at a client's destination (`killBox`).
pub fn kill_box(
    pool: &mut EntityPool,
    combat: &mut dyn CombatOps,
    world: &mut dyn WorldOps,
    slot: Slot,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let entity = &pool.entities[slot];
    let Some(client) = entity.client.as_ref() else {
        return Err(invalid("G_KillBox requires a client entity"));
    };
    let origin = client.ps.origin;
    let mins = add3(origin, entity.r.mins);
    let maxs = add3(origin, entity.r.maxs);
    let contacts = world.area_actors(pool, Bounds { min: mins, max: maxs }, 1024);
    for actor in contacts {
        if !combat.linked_bounds(pool, actor) || !combat.is_player(pool, actor) {
            continue;
        }
        let Some(hit) = combat.participant(pool, actor) else {
            continue;
        };
        combat.damage(
            pool,
            hit,
            DamageParticipant::Entity(slot),
            DamageParticipant::Entity(slot),
            None,
            None,
            100000,
            DamageFlags::NO_PROTECTION,
            MOD_TELEFRAG,
            None,
        )?;
    }
    Ok(())
}

/// Teleport a player (`teleportPlayer`).
pub fn teleport_player(
    pool: &mut EntityPool,
    context: &mut TeleportContext<'_>,
    player: Slot,
    origin: Vec3,
    angles: Vec3,
) -> Q3GameItemsResult<()> {
    pool.require_owned(player)?;
    if pool.entities[player].client.is_none() {
        return Err(invalid("TeleportPlayer requires a client entity"));
    }
    let team = pool.entities[player]
        .client
        .as_ref()
        .expect("client checked")
        .sess
        .session_team;
    let number = pool.entities[player].s.number;
    let client_num = pool.entities[player].s.client_num;
    if team != Team::Spectator {
        let start = pool.entities[player].client.as_ref().expect("client checked").ps.origin;
        let out = context
            .combat
            .temp_entity(pool, &mut *context.world, start, EntityEvent::PlayerTeleportOut)?;
        pool.at_mut(out)?.s.client_num = client_num;
        let incoming = context
            .combat
            .temp_entity(pool, &mut *context.world, origin, EntityEvent::PlayerTeleportIn)?;
        pool.at_mut(incoming)?.s.client_num = client_num;
    }
    context.world.unlink(pool, number)?;
    {
        let entity = &mut pool.entities[player];
        let client = entity.client_mut()?;
        client.ps.origin = vec3(origin.x, origin.y, origin.z + 1.0);
        client.ps.velocity = scale3(qvm_angle_vectors(angles).forward, 400.0);
        client.ps.pm_time = 160;
        client.ps.pm_flags |= MoveFlags::TIME_KNOCKBACK;
        client.ps.e_flags ^= EF_TELEPORT_BIT;
    }
    set_client_view_angle(pool, player, angles)?;
    if team != Team::Spectator {
        kill_box(pool, context.combat, &mut *context.world, player)?;
    }
    player_state_to_entity_state(pool, player, true)?;
    let origin = pool.entities[player].client.as_ref().expect("client checked").ps.origin;
    pool.entities[player].r.current_origin = origin;
    if team != Team::Spectator {
        context.world.link(pool, player)?;
    }
    Ok(())
}

/// Portal context (`PortalContext`).
pub struct PortalContext<'a> {
    /// World host.
    pub world: &'a mut dyn WorldOps,
    /// Current time.
    pub time: i32,
    /// Nonnegative random integer.
    pub random_int: &'a mut dyn FnMut() -> i32,
    /// Warning sink.
    pub warn: &'a mut dyn FnMut(&str),
}

/// Locate a portal camera (`locateCamera`).
pub fn locate_camera(pool: &mut EntityPool, context: &mut PortalContext<'_>, slot: Slot) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let target_name = pool.at(slot)?.target.clone();
    let owner = {
        let mut selection = TargetSelection {
            random_int: &mut *context.random_int,
            warn: &mut *context.warn,
        };
        pick_target(pool, &mut selection, target_name.as_deref())?
    };
    let Some(owner) = owner else {
        (context.warn)("Couldn't find target for misc_partal_surface\n");
        pool.free(slot)?;
        return Ok(());
    };
    let owner_number = pool.at(owner)?.s.number;
    let owner_client = pool.at(owner)?.s.client_num;
    let owner_origin = pool.at(owner)?.s.origin;
    let owner_target = pool.at(owner)?.target.clone();
    let owner_spawnflags = pool.at(owner)?.spawnflags;
    {
        let entity = &mut pool.entities[slot];
        entity.r.owner_num = owner_number;
        if owner_spawnflags & 1 != 0 {
            entity.s.frame = 25;
        } else if owner_spawnflags & 2 != 0 {
            entity.s.frame = 75;
        }
        entity.s.powerups = if owner_spawnflags & 4 != 0 { 0 } else { 1 };
        entity.s.client_num = owner_client;
        entity.s.origin2 = owner_origin;
    }
    let target = {
        let mut selection = TargetSelection {
            random_int: &mut *context.random_int,
            warn: &mut *context.warn,
        };
        pick_target(pool, &mut selection, owner_target.as_deref())?
    };
    let direction = match target {
        Some(target) => {
            let target_origin = pool.at(target)?.s.origin;
            normalize3(sub3(target_origin, owner_origin))
        }
        None => {
            let angles = pool.at(owner)?.s.angles;
            let (direction, cleared) = move_direction(angles);
            pool.at_mut(owner)?.s.angles = cleared;
            direction
        }
    };
    pool.at_mut(slot)?.s.event_parm = direction_to_byte(Some(direction));
    Ok(())
}

/// Spawn a portal surface (`spawnPortalSurface`).
pub fn spawn_portal_surface(
    pool: &mut EntityPool,
    world: &mut dyn WorldOps,
    time: i32,
    slot: Slot,
) -> Q3GameItemsResult<()> {
    bind_portal_save_callbacks(pool);
    pool.require_owned(slot)?;
    {
        let entity = &mut pool.entities[slot];
        entity.r.mins = vec3(0.0, 0.0, 0.0);
        entity.r.maxs = vec3(0.0, 0.0, 0.0);
    }
    world.link(pool, slot)?;
    pool.entities[slot].r.sv_flags = ServerEntityFlags::PORTAL;
    pool.entities[slot].s.e_type = EntityType::Portal as i32;
    if pool.entities[slot].target.is_none() {
        let origin = pool.entities[slot].s.origin;
        pool.entities[slot].s.origin2 = origin;
    } else {
        let think = pool.think_cbs.resolve(PORTAL_SURFACE_THINK)?;
        pool.entities[slot].think = Some(think);
        pool.entities[slot].nextthink = time.wrapping_add(100);
    }
    Ok(())
}

/// Spawn a portal camera (`spawnPortalCamera`); roll is the parsed `roll` spawn float.
pub fn spawn_portal_camera(
    pool: &mut EntityPool,
    world: &mut dyn WorldOps,
    slot: Slot,
    roll: f32,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    {
        let entity = &mut pool.entities[slot];
        entity.r.mins = vec3(0.0, 0.0, 0.0);
        entity.r.maxs = vec3(0.0, 0.0, 0.0);
    }
    world.link(pool, slot)?;
    let packed = roll / 360.0 * 256.0;
    pool.entities[slot].s.client_num = if (-2_147_483_648.0..2_147_483_648.0).contains(&packed) {
        packed.trunc() as i32
    } else {
        i32::MIN
    };
    Ok(())
}

/// Register portal save callbacks (`bindPortalSaveCallbacks`).
pub fn bind_portal_save_callbacks(pool: &mut EntityPool) {
    pool.think_cbs.intern(PORTAL_SURFACE_THINK);
}
