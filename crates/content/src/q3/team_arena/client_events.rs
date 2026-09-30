//! Quake III team-arena: client events.
//!
//! Donor provenance: `src/content/q3/team-arena/client-events.ts`.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::*;
use crate::q3::team_arena::client_spawn::*;
use crate::q3::team_arena::support::*;

// Pattern-position aliases for EntityEvent discriminants (`as` casts are not patterns).
const EV_FALL_MEDIUM: i32 = EntityEvent::EvFallMedium as i32;
const EV_FALL_FAR: i32 = EntityEvent::EvFallFar as i32;
const EV_FIRE_WEAPON: i32 = EntityEvent::EvFireWeapon as i32;
const EV_USE_ITEM1: i32 = EntityEvent::EvUseItem1 as i32;
const EV_USE_ITEM2: i32 = EntityEvent::EvUseItem2 as i32;
const EV_USE_ITEM3: i32 = EntityEvent::EvUseItem3 as i32;
const EV_USE_ITEM4: i32 = EntityEvent::EvUseItem4 as i32;
const EV_USE_ITEM5: i32 = EntityEvent::EvUseItem5 as i32;

// ---------------------------------------------------------------------------
// client-events.ts
// ---------------------------------------------------------------------------

/// Spawn selection for teleports (`ClientSpawnRuntime` subset).
pub trait SpawnSelector {
    /// Select a deathmatch spawn point.
    fn select_spawn_point(&self, avoid: Vec3) -> SpawnPoint;
}

/// Client event services (`ClientEventsContext`).
#[allow(clippy::type_complexity)]
pub struct ClientEvents {
    /// Server world.
    pub world: WorldRef,
    /// Weapon services.
    pub weapons: Rc<dyn WeaponHost>,
    /// Spawn selection.
    pub spawns: Rc<dyn SpawnSelector>,
    /// Item drops.
    pub drops: Rc<dyn DropHost>,
    /// Item lookups.
    pub items: Rc<dyn ItemHost>,
    /// Teleport services.
    pub teleport: Rc<dyn TeleportHost>,
    /// `dmflags` bits.
    pub dmflags: i32,
    /// Optional primary-attack gate.
    pub primary_attack_allowed: Option<Rc<dyn Fn(&ActorId) -> bool>>,
    /// Product.
    pub product: Product,
    /// Combat services.
    pub combat: CombatRef,
    /// Personal portals (missionpack only).
    pub personal_portal: Option<Rc<dyn PortalHost>>,
}

/// Holdable services (`Q3HoldableContext`).
pub struct Q3Holdable {
    /// Weapon services.
    pub weapons: Rc<dyn WeaponHost>,
    /// Teleport effect.
    pub teleport: Rc<dyn Fn(&EntityRef)>,
    /// Product.
    pub product: Product,
    /// Combat services.
    pub combat: CombatRef,
    /// Personal portals (missionpack only).
    pub personal_portal: Option<Rc<dyn PortalHost>>,
}

/// Run server effects from the predictable-event ring (`clientEvents`).
pub fn client_events(context: &ClientEvents, entity: &EntityRef, old_event_sequence: i32) {
    let client = match entity.borrow().client.clone() {
        Some(client) => client,
        None => panic!("ClientEvents requires a client entity"),
    };
    let mut index = old_event_sequence;
    let oldest = client.borrow().ps.event_sequence.wrapping_sub(2);
    if index < oldest {
        index = oldest;
    }
    // Both the sequence bound and ring slot stay live across callbacks.
    while index < client.borrow().ps.event_sequence {
        let event = client.borrow().ps.events.get((index & 1) as usize);
        match event {
            EV_FALL_MEDIUM | EV_FALL_FAR => {
                let e_type = entity.borrow().s.e_type;
                if e_type != EntityType::EtPlayer as i32 || context.dmflags & 8 != 0 {
                    // No damage.
                } else {
                    entity.borrow_mut().pain_debounce_time = context.combat.time().wrapping_add(200);
                    let amount = if event == EntityEvent::EvFallFar as i32 { 10 } else { 5 };
                    context.combat.damage(entity, None, None, None, None, amount, 0, 19);
                }
            }
            EV_FIRE_WEAPON => {
                let allowed = match &context.primary_attack_allowed {
                    Some(gate) => gate(&entity.borrow().actor),
                    None => true,
                };
                if allowed {
                    context.weapons.fire(entity);
                }
            }
            EV_USE_ITEM1 | EV_USE_ITEM2 | EV_USE_ITEM3 | EV_USE_ITEM4 | EV_USE_ITEM5 => {
                // The donor spreads the event context and overrides teleport.
                let drops = context.drops.clone();
                let items = context.items.clone();
                let product = context.product;
                let combat = context.combat.clone();
                let spawns = context.spawns.clone();
                let teleport = context.teleport.clone();
                let holdable = Q3Holdable {
                    weapons: context.weapons.clone(),
                    teleport: Rc::new(move |target: &EntityRef| {
                        drop_q3_teleport_objectives(drops.clone(), items.clone(), product, combat.clone(), target);
                        let origin = target.borrow().client.clone().map(|client| client.borrow().ps.origin);
                        let origin = origin.unwrap_or_else(|| panic!("Teleporter requires a client entity"));
                        let spawn = spawns.select_spawn_point(origin);
                        teleport.teleport_player(target, spawn.origin, spawn.angles);
                    }),
                    product: context.product,
                    combat: context.combat.clone(),
                    personal_portal: context.personal_portal.clone(),
                };
                use_q3_holdable(&holdable, entity, event);
            }
            _ => {}
        }
        index = index.wrapping_add(1);
    }
}

/// Original holdable effects (`useQ3Holdable`).
pub fn use_q3_holdable(context: &Q3Holdable, entity: &EntityRef, event: i32) {
    let client = match entity.borrow().client.clone() {
        Some(client) => client,
        None => panic!("Holdable effects require a client entity"),
    };
    match event {
        EV_USE_ITEM1 => {
            context.combat.pool().rankings.use_holdable(entity.borrow().slot, 1);
            (context.teleport)(entity);
        }
        EV_USE_ITEM2 => {
            context.combat.pool().rankings.use_holdable(entity.borrow().slot, 2);
            let max = {
                let record = client.borrow();
                let slot = match stat_schema(record.ps.product) {
                    StatSchema::Base(layout) => layout.max_health,
                    StatSchema::Missionpack(layout) => layout.max_health,
                };
                record.ps.stats.get(slot as usize)
            };
            entity.borrow_mut().health = max.wrapping_add(25);
        }
        EV_USE_ITEM3 => {
            if context.product == Product::Missionpack {
                client.borrow_mut().invulnerability_time = 0;
                context.weapons.start_kamikaze(entity);
            }
        }
        EV_USE_ITEM4 => {
            if context.product == Product::Missionpack {
                match &context.personal_portal {
                    Some(portal) => {
                        if client.borrow().portal_id != 0 {
                            portal.drop_portal_source(entity);
                        } else {
                            portal.drop_portal_destination(entity);
                        }
                    }
                    None => panic!("Holdable effects require the missionpack portal runtime"),
                }
            }
        }
        EV_USE_ITEM5 if context.product == Product::Missionpack => {
            let time = context.combat.time().wrapping_add(10_000);
            client.borrow_mut().invulnerability_time = time;
        }
        _ => {}
    }
}

/// Drop flags and harvester cubes before a teleporter jump
/// (`dropQ3TeleportObjectives`).
pub fn drop_q3_teleport_objectives(
    drops: Rc<dyn DropHost>,
    items: Rc<dyn ItemHost>,
    product: Product,
    combat: CombatRef,
    entity: &EntityRef,
) {
    let client = match entity.borrow().client.clone() {
        Some(client) => client,
        None => panic!("Teleporter requires a client entity"),
    };
    let powerup = {
        let record = client.borrow();
        if record.ps.powerups.get(Powerup::PwRedflag as usize) != 0 {
            Powerup::PwRedflag as i32
        } else if record.ps.powerups.get(Powerup::PwBlueflag as usize) != 0 {
            Powerup::PwBlueflag as i32
        } else if record.ps.powerups.get(Powerup::PwNeutralflag as usize) != 0 {
            Powerup::PwNeutralflag as i32
        } else {
            Powerup::PwNone as i32
        }
    };
    if powerup != Powerup::PwNone as i32 {
        if let Some(flag) = items.find_item_for_powerup(product, powerup) {
            let dropped = drops.drop_item(entity, &flag, 0);
            let remaining = client
                .borrow()
                .ps
                .powerups
                .get(powerup as usize)
                .wrapping_sub(combat.time());
            dropped.borrow_mut().count = 1.max(remaining / 1000);
            client.borrow_mut().ps.powerups.set(powerup as usize, 0);
        }
    }
    let harvester = product == Product::Missionpack
        && combat.game_type() == GameType::GtHarvester as i32
        && client.borrow().ps.generic1 > 0;
    if harvester {
        let (cube_name, spawnflags) = if client.borrow().sess.session_team == Team::TeamRed as i32 {
            ("Blue Cube", Team::TeamBlue as i32)
        } else {
            ("Red Cube", Team::TeamRed as i32)
        };
        if let Some(cube) = items.find_item(product, cube_name) {
            let count = client.borrow().ps.generic1;
            for _ in 0..count {
                let dropped = drops.drop_item(entity, &cube, 0);
                dropped.borrow_mut().spawnflags = spawnflags;
            }
        }
        client.borrow_mut().ps.generic1 = 0;
    }
}
