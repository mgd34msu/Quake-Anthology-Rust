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

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, OwnedActor, ProviderId};
    use qa_world::body::{BodyState, LinkedBody};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use crate::q3::base::records::test_support::*;
    use crate::q3::base::records::*;
    use crate::q3::base::shared::entity_shared::EntityBodyBinding;

    struct FakeSpawnHost {
        product: Product,
        obelisks: RefCell<Vec<String>>,
    }

    impl Q3SpawnHandlersHost for FakeSpawnHost {
        fn product(&self) -> Product {
            self.product
        }

        fn misc_handlers(&self) -> HashMap<String, SpawnHandler> {
            let mut handlers: HashMap<String, SpawnHandler> = HashMap::new();
            handlers.insert(
                "info_null".to_string(),
                Rc::new(|entity: EntityRef, _: &SpawnVariables| {
                    entity.borrow_mut().flags |= 1;
                }),
            );
            handlers
        }

        fn mover_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn trigger_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn target_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn spawn_player_start(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_deathmatch_point(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_team_point(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_team_obelisk(&self, _entity: EntityRef, team: Team) {
            self.obelisks.borrow_mut().push(format!("team:{}", team as i32));
        }

        fn spawn_neutral_obelisk(&self, _entity: EntityRef) {
            self.obelisks.borrow_mut().push("neutral".to_string());
        }
    }

    #[test]
    fn spawn_handlers_cover_routes_and_obelisks() {
        let host = Rc::new(FakeSpawnHost {
            product: Product::Missionpack,
            obelisks: RefCell::new(Vec::new()),
        });
        let handlers = create_q3_spawn_handlers(host.clone()).expect("handlers");
        assert!(handlers.contains_key("info_player_start"));
        assert!(handlers.contains_key("info_player_intermission"));
        assert!(handlers.contains_key("item_botroam"));
        assert!(handlers.contains_key("func_group"));
        assert!(handlers.contains_key("team_redobelisk"));
        let (_records_host, records) = test_records();
        let entity = records.activate(40);
        handlers["func_group"](entity.clone(), &SpawnVariables::default());
        assert_eq!(entity.borrow().flags, 1);
        handlers["team_redobelisk"](entity.clone(), &SpawnVariables::default());
        handlers["team_blueobelisk"](entity.clone(), &SpawnVariables::default());
        handlers["team_neutralobelisk"](entity, &SpawnVariables::default());
        assert_eq!(
            *host.obelisks.borrow(),
            vec![
                format!("team:{}", Team::TeamRed as i32),
                format!("team:{}", Team::TeamBlue as i32),
                "neutral".to_string()
            ]
        );

        let base = Rc::new(FakeSpawnHost {
            product: Product::Baseq3,
            obelisks: RefCell::new(Vec::new()),
        });
        let base_handlers = create_q3_spawn_handlers(base).expect("base");
        assert!(!base_handlers.contains_key("team_redobelisk"));
    }

    struct EmptySpawnHost;

    impl Q3SpawnHandlersHost for EmptySpawnHost {
        fn product(&self) -> Product {
            Product::Baseq3
        }

        fn misc_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn mover_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn trigger_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn target_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn spawn_player_start(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_deathmatch_point(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_team_point(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_team_obelisk(&self, _entity: EntityRef, _team: Team) {}

        fn spawn_neutral_obelisk(&self, _entity: EntityRef) {}
    }

    #[test]
    fn spawn_handlers_require_info_null() {
        let host = Rc::new(EmptySpawnHost);
        assert!(create_q3_spawn_handlers(host).is_err());
    }

    struct DirectBinding {
        active: bool,
        actor: OwnedActor,
        body: Rc<FakeDirectBody>,
    }

    struct FakeDirectBody {
        state: RefCell<BodyState>,
    }

    impl EntityBodyBinding for FakeDirectBody {
        fn read(&self) -> BodyState {
            self.state.borrow().clone()
        }

        fn write(&self, value: BodyState) {
            *self.state.borrow_mut() = value;
        }

        fn linked(&self) -> Option<LinkedBody> {
            None
        }
    }

    impl GameEntityBinding for DirectBinding {
        fn body(&self) -> Rc<dyn EntityBodyBinding> {
            self.body.clone()
        }

        fn actor(&self) -> OwnedActor {
            self.actor.clone()
        }

        fn active(&self) -> bool {
            self.active
        }

        fn health(&self) -> i32 {
            0
        }

        fn set_health(&self, _value: i32) {}

        fn takes_damage(&self) -> bool {
            false
        }

        fn set_takes_damage(&self, _value: bool) {}

        fn schedule(&self, _nextthink: i32) {}

        fn run_think(&self, _time_milliseconds: i32) {}
    }

    struct VecPool {
        entities: Vec<EntityRef>,
    }

    impl Q3EntityPool for VecPool {
        fn num_entities(&self) -> usize {
            self.entities.len()
        }

        fn entity_at(&self, index: usize) -> EntityRef {
            self.entities[index].clone()
        }
    }

    fn team_entity(owner: &IdentityOwner, slot: usize, team: Option<&str>) -> EntityRef {
        let actor = owner
            .owned_actor(&owner.actor(slot as u32, 1), ProviderId::new("q3", "test"))
            .expect("owned");
        let binding = Rc::new(DirectBinding {
            active: true,
            actor,
            body: Rc::new(FakeDirectBody {
                state: RefCell::new(ZERO_BODY.clone()),
            }),
        });
        let entity = Rc::new(RefCell::new(GameEntity::new(slot, binding)));
        entity.borrow_mut().team = team.map(str::to_string);
        entity
    }

    #[test]
    fn entity_teams_chain_and_transfer_targetnames() {
        let owner = test_owner();
        let first = team_entity(&owner, 1, Some("alpha"));
        let second = team_entity(&owner, 2, Some("alpha"));
        second.borrow_mut().targetname = Some("slave-target".to_string());
        let third = team_entity(&owner, 3, Some("beta"));
        let pool = VecPool {
            entities: vec![
                team_entity(&owner, 0, None),
                first.clone(),
                second.clone(),
                third.clone(),
            ],
        };
        let counts = find_q3_entity_teams(&pool);
        assert_eq!(counts, EntityTeamCounts { teams: 2, entities: 3 });
        assert_eq!(first.borrow().targetname, Some("slave-target".to_string()));
        assert_eq!(second.borrow().targetname, None);
        assert_ne!(second.borrow().flags & GameFlags::TEAMSLAVE, 0);
        assert!(Rc::ptr_eq(first.borrow().teamchain.as_ref().unwrap(), &second));
        assert!(third.borrow().teamchain.is_none());
    }
}
