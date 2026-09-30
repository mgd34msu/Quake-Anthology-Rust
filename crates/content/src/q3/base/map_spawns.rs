//! Quake III base: map spawns.
//!
//! Donor provenance: `src/content/q3/base/map-spawns.ts`.

use std::collections::HashMap;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::state::GameFlags;
use crate::q3::base::records::{EntityRef, Q3BaseError, Q3EntityPool};
use crate::q3::base::shared::definitions::*;

// ---------------------------------------------------------------------------
// map-spawns.ts
// ---------------------------------------------------------------------------

/// Spawn variables (`SpawnVariables`, game/spawn.ts, minimal).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpawnVariables {
    /// Key/value pairs.
    pub vars: Vec<(String, String)>,
}

/// Spawn handler (`SpawnHandler`, game/spawn.ts).
pub type SpawnHandler = Rc<dyn Fn(EntityRef, &SpawnVariables)>;

/// Spawn handler host services (`Q3SpawnHandlersHost`).
///
/// Handler tables and spawn functions live in the sibling game-layer and
/// team-arena ports; the host supplies them so the parent can unify at
/// merge.
pub trait Q3SpawnHandlersHost {
    /// Product.
    fn product(&self) -> Product;
    /// Misc spawn handlers (`miscSpawnHandlers`).
    fn misc_handlers(&self) -> HashMap<String, SpawnHandler>;
    /// Mover spawn handlers (`MoverSpawnRuntime.handlers`).
    fn mover_handlers(&self) -> HashMap<String, SpawnHandler>;
    /// Trigger spawn handlers (`triggerSpawnHandlers`).
    fn trigger_handlers(&self) -> HashMap<String, SpawnHandler>;
    /// Target spawn handlers (`targetSpawnHandlers`).
    fn target_handlers(&self) -> HashMap<String, SpawnHandler>;
    /// Player start spawn (`spawnPlayerStart`).
    fn spawn_player_start(&self) -> SpawnHandler;
    /// Deathmatch point spawn (`spawnDeathmatchPoint`).
    fn spawn_deathmatch_point(&self) -> SpawnHandler;
    /// Team point spawn (`spawnTeamPoint`).
    fn spawn_team_point(&self) -> SpawnHandler;
    /// Team obelisk spawn (`TeamRuntime.spawnTeamObelisk`).
    fn spawn_team_obelisk(&self, entity: EntityRef, team: Team);
    /// Neutral obelisk spawn (`TeamRuntime.spawnNeutralObelisk`).
    fn spawn_neutral_obelisk(&self, entity: EntityRef);
}

/// All source classname routes, including the two intentionally empty
/// native spawn functions (`createQ3SpawnHandlers`).
pub fn create_q3_spawn_handlers(
    host: Rc<dyn Q3SpawnHandlersHost>,
) -> Result<HashMap<String, SpawnHandler>, Q3BaseError> {
    let mut handlers = HashMap::new();
    handlers.insert("info_player_start".to_string(), host.spawn_player_start());
    handlers.insert("info_player_deathmatch".to_string(), host.spawn_deathmatch_point());
    handlers.insert(
        "info_player_intermission".to_string(),
        Rc::new(|_: EntityRef, _: &SpawnVariables| {}) as SpawnHandler,
    );
    handlers.insert(
        "item_botroam".to_string(),
        Rc::new(|_: EntityRef, _: &SpawnVariables| {}) as SpawnHandler,
    );
    for classname in [
        "team_CTF_redplayer",
        "team_CTF_blueplayer",
        "team_CTF_redspawn",
        "team_CTF_bluespawn",
    ] {
        handlers.insert(classname.to_string(), host.spawn_team_point());
    }
    handlers.extend(host.misc_handlers());
    handlers.extend(host.mover_handlers());
    handlers.extend(host.trigger_handlers());
    handlers.extend(host.target_handlers());
    let Some(remove) = handlers.get("info_null").cloned() else {
        return Err(Q3BaseError::Invalid(
            "Source info_null handler is unavailable".to_string(),
        ));
    };
    handlers.insert("func_group".to_string(), remove);
    if host.product() == Product::Missionpack {
        let red = host.clone();
        handlers.insert(
            "team_redobelisk".to_string(),
            Rc::new(move |entity, _| red.spawn_team_obelisk(entity, Team::TeamRed)),
        );
        let blue = host.clone();
        handlers.insert(
            "team_blueobelisk".to_string(),
            Rc::new(move |entity, _| blue.spawn_team_obelisk(entity, Team::TeamBlue)),
        );
        handlers.insert(
            "team_neutralobelisk".to_string(),
            Rc::new(move |entity, _| host.spawn_neutral_obelisk(entity)),
        );
    }
    Ok(handlers)
}

/// Linked entity team counts (`findQ3EntityTeams` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityTeamCounts {
    /// Team count.
    pub teams: i32,
    /// Chained entity count.
    pub entities: i32,
}

/// Link entity teams (`findQ3EntityTeams`).
///
/// Preserves prepended teammate order and transfers each slave's
/// targetname to its master.
pub fn find_q3_entity_teams(pool: &dyn Q3EntityPool) -> EntityTeamCounts {
    let mut teams = 0;
    let mut entities = 0;
    let num = pool.num_entities();
    let mut index = 1;
    while index < num {
        let master = pool.entity_at(index);
        let (inuse, team, slave) = {
            let borrowed = master.borrow();
            (
                borrowed.inuse(),
                borrowed.team.clone(),
                borrowed.flags & GameFlags::TEAMSLAVE != 0,
            )
        };
        if !inuse || team.is_none() || slave {
            index += 1;
            continue;
        }
        master.borrow_mut().teammaster = Some(Rc::downgrade(&master));
        teams += 1;
        entities += 1;
        let mut next = index + 1;
        while next < num {
            let entity = pool.entity_at(next);
            let (inuse, team_name, slave, same) = {
                let borrowed = entity.borrow();
                (
                    borrowed.inuse(),
                    borrowed.team.clone(),
                    borrowed.flags & GameFlags::TEAMSLAVE != 0,
                    borrowed.team == team,
                )
            };
            if !inuse || team_name.is_none() || slave || !same {
                next += 1;
                continue;
            }
            entities += 1;
            let head = master.borrow_mut().teamchain.take();
            entity.borrow_mut().teamchain = head;
            master.borrow_mut().teamchain = Some(entity.clone());
            entity.borrow_mut().teammaster = Some(Rc::downgrade(&master));
            entity.borrow_mut().flags |= GameFlags::TEAMSLAVE;
            if let Some(targetname) = entity.borrow_mut().targetname.take() {
                master.borrow_mut().targetname = Some(targetname);
            }
            next += 1;
        }
        index += 1;
    }
    EntityTeamCounts { teams, entities }
}
