//! Quake III source presentation snapshots for cgame/network consumers.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/q3/presentation.ts`
//! (`Q3SourcePresentationState`, `q3SourcePresentationState`,
//! `q3PoolPresentationState`, `q3SourceModels`, `q3PoolModels`).
//!
//! The pool-level snapshots land here; the two `Q3SourceRuntime` wrappers
//! (`q3SourcePresentationState`, `q3SourceModels`) land with `super::runtime`
//! in the same partition to keep every commit compiling.
//!
//! Two entity-core projections apply. Client rows carry the pool's owned
//! `PlayerState` mirror (`entities.rs`), not the behavior-rich shared
//! `PlayerState`: the pool never stores the shared type, so converting
//! would invent data. The pool core also records no `r.model` word, so the
//! donor's inline-model arm is reconstructed from the spawn paths the port
//! implements: triggers always carry `NOCLIENT` (filtered before the model
//! arms) and movers always spawn from `*N` brush models, so `ET_MOVER` with
//! a positive model index renders as `*N`, exactly matching donor output
//! for every entity the ported spawns can produce.

use qa_content::contract::{ContentId, GameFamily};
use qa_content::q3::base::game::entities::{EntityPool, PlayerState};
use qa_content::q3::base::game::utilities::ConfigStringStore;
use qa_content::q3::base::shared::definitions::{EntityType, Product, Weapon};
use qa_content::q3::base::shared::entity_shared::ServerEntityFlags;
use qa_content::q3::base::shared::entity_state::EntityState;
use qa_content::q3::base::shared::items::item_at;
use qa_content::q3::base::shared::trajectory::evaluate_trajectory;
use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use super::super::types::SimulationPresentation;
use super::server_state::Q3_CONFIGSTRINGS;

/// One copied source entity row.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SourcePresentationEntity {
    /// Acting actor.
    pub actor: ActorId,
    /// Entity state snapshot.
    pub state: EntityState,
    /// World origin.
    pub origin: Vec3,
    /// Whether linked.
    pub linked: bool,
    /// Server flags.
    pub server_flags: i32,
    /// Single-client target.
    pub single_client: i32,
}

/// One copied source client row.
///
/// The pool core's `PlayerState` is `Clone`-only, so this row (and the
/// state below) cannot implement `Debug`/`PartialEq`.
#[derive(Clone)]
pub struct Q3SourcePresentationClient {
    /// Acting actor.
    pub actor: ActorId,
    /// Client slot.
    pub slot: i32,
    /// Player state snapshot.
    pub state: PlayerState,
}

/// One copied configstring row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3SourcePresentationString {
    /// Configstring index.
    pub index: i32,
    /// Configstring value.
    pub value: String,
}

/// Copied source state for cgame/network consumers.
#[derive(Clone)]
pub struct Q3SourcePresentationState {
    /// Product.
    pub product: Product,
    /// Presentation time in milliseconds.
    pub time: i32,
    /// Copied entities.
    pub entities: Vec<Q3SourcePresentationEntity>,
    /// Copied clients.
    pub clients: Vec<Q3SourcePresentationClient>,
    /// Non-empty configstrings.
    pub configstrings: Vec<Q3SourcePresentationString>,
}

/// Copy source state for cgame/network consumers without advancing events
/// or owning authoritative state.
#[must_use]
pub fn q3_pool_presentation_state(
    pool: &EntityPool,
    product: Product,
    time: i32,
    strings: &dyn ConfigStringStore,
) -> Q3SourcePresentationState {
    let mut entities = Vec::new();
    let mut clients = Vec::new();
    for slot in 0..pool.num_entities() {
        let entity = pool.at(slot as i32);
        let entity = entity.borrow();
        if !entity.inuse {
            continue;
        }
        entities.push(Q3SourcePresentationEntity {
            actor: entity.actor.id.clone(),
            state: entity.s.copy(),
            origin: entity.r.current_origin,
            linked: entity.r.linked,
            server_flags: entity.r.sv_flags,
            single_client: entity.r.single_client,
        });
        if let Some(client) = entity.client.as_ref() {
            clients.push(Q3SourcePresentationClient {
                actor: entity.actor.id.clone(),
                slot: slot as i32,
                state: client.ps.clone(),
            });
        }
    }
    let mut configstrings = Vec::new();
    for index in 0..Q3_CONFIGSTRINGS {
        let value = strings.get(index as usize);
        if !value.is_empty() {
            configstrings.push(Q3SourcePresentationString { index, value });
        }
    }
    Q3SourcePresentationState {
        product,
        time,
        entities,
        clients,
        configstrings,
    }
}

/// Missile world model for a weapon number.
fn missile_model(weapon: i32) -> Option<&'static str> {
    if weapon == Weapon::WpGrapplingHook as i32 || weapon == Weapon::WpRocketLauncher as i32 {
        Some("models/ammo/rocket/rocket.md3")
    } else if weapon == Weapon::WpGrenadeLauncher as i32 {
        Some("models/ammo/grenade1.md3")
    } else if weapon == Weapon::WpProxLauncher as i32 {
        Some("models/weaphits/proxmine.md3")
    } else if weapon == Weapon::WpNailgun as i32 {
        Some("models/weaphits/nail.md3")
    } else if weapon == Weapon::WpBfg as i32 {
        Some("models/weaphits/bfg.md3")
    } else {
        None
    }
}

fn presentation(
    actor: ActorId,
    content: &ContentId,
    path: String,
    state: &EntityState,
    time: i32,
) -> SimulationPresentation {
    SimulationPresentation {
        held_weapon: None,
        native_held_weapon: false,
        weapon_item: None,
        replaces_body: false,
        render_source_client: true,
        flare: None,
        actor,
        content: content.clone(),
        family: GameFamily::Q3,
        path,
        frame: state.frame,
        old_frame: state.frame,
        back_lerp: None,
        skin: 0,
        skin_path: None,
        indexed_skin: None,
        player_colors: None,
        effects: state.e_flags,
        render_flags: 0,
        origin: evaluate_trajectory(&state.pos, time),
        previous_origin: None,
        model_beam: None,
        shader_beam: None,
        model_attachments: None,
        model_anchor: None,
        q3_grapple_cable: None,
        angles: evaluate_trajectory(&state.apos, time),
        scale: 1.0,
        alpha: None,
        visible: true,
        view_weapon: false,
        q3_weapon: None,
    }
}

/// Model access for the shared renderer; sprites, trails and portals remain
/// in source state for cgame.
#[must_use]
pub fn q3_pool_models(
    pool: &EntityPool,
    product: Product,
    time: i32,
    content: &ContentId,
    strings: &dyn ConfigStringStore,
) -> Vec<SimulationPresentation> {
    let mut presentations = Vec::new();
    for slot in 0..pool.num_entities() {
        let entity = pool.at(slot as i32);
        let entity = entity.borrow();
        let state = &entity.s;
        if !entity.inuse
            || entity.client.is_some()
            || !entity.r.linked
            || entity.r.sv_flags & ServerEntityFlags::Noclient.bits() != 0
            || state.e_flags & 0x80 != 0
        {
            continue;
        }
        let mut paths: Vec<Option<String>> = Vec::new();
        if state.e_type == EntityType::EtItem as i32 {
            let item = item_at(product, state.modelindex)
                .unwrap_or_else(|_| panic!("Item index out of range: {}", state.modelindex));
            paths.extend(item.world_models.iter().map(|model| model.map(str::to_string)));
        } else if state.e_type == EntityType::EtMissile as i32 {
            paths.push(missile_model(state.weapon).map(str::to_string));
        } else if state.e_type == EntityType::EtMover as i32 && state.modelindex > 0 {
            paths.push(Some(format!("*{}", state.modelindex)));
        } else if state.modelindex > 0 {
            paths.push(Some(strings.get(32 + state.modelindex as usize)));
        }
        for path in paths.into_iter().flatten() {
            if path.is_empty() {
                continue;
            }
            presentations.push(presentation(entity.actor.id.clone(), content, path, state, time));
        }
    }
    presentations
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::q3::base::game::entities::EntityPoolOptions;
    use std::collections::HashMap;
    use std::rc::Rc;

    struct MapStrings {
        values: HashMap<usize, String>,
    }

    impl ConfigStringStore for MapStrings {
        fn get(&self, index: usize) -> String {
            self.values.get(&index).cloned().unwrap_or_default()
        }

        fn set(&mut self, index: usize, value: &str) {
            self.values.insert(index, value.to_string());
        }
    }

    fn pool(max_clients: usize) -> EntityPool {
        EntityPool::open(EntityPoolOptions {
            product: Product::Baseq3,
            max_clients,
            map_start_time: 0,
            time: Rc::new(|| 0),
            print: Rc::new(|_| {}),
            link: Rc::new(|_| {}),
            unlink: Rc::new(|_| {}),
            event_debug: None,
        })
    }

    fn content() -> ContentId {
        ContentId("q3:baseq3:maps:q3dm1".to_string())
    }

    #[test]
    fn empty_pool_copies_only_configstrings() {
        let pool = pool(1);
        let strings = MapStrings {
            values: HashMap::from([(0, "server".to_string()), (7, "map".to_string())]),
        };
        let state = q3_pool_presentation_state(&pool, Product::Baseq3, 120, &strings);
        assert!(state.entities.is_empty());
        assert!(state.clients.is_empty());
        assert_eq!(state.time, 120);
        assert_eq!(
            state.configstrings,
            vec![
                Q3SourcePresentationString {
                    index: 0,
                    value: "server".to_string()
                },
                Q3SourcePresentationString {
                    index: 7,
                    value: "map".to_string()
                },
            ]
        );
        assert!(q3_pool_models(&pool, Product::Baseq3, 120, &content(), &strings).is_empty());
    }

    #[test]
    fn client_entity_copies_entity_and_client_rows() {
        let pool = pool(2);
        {
            let entity = pool.at(1);
            let mut entity = entity.borrow_mut();
            entity.inuse = true;
            entity.r.linked = true;
            entity.r.current_origin = qa_core::math::vec3(1.0, 2.0, 3.0);
            if let Some(client) = entity.client.as_mut() {
                client.ps.client_num = 1;
            }
        }
        let strings = MapStrings { values: HashMap::new() };
        let state = q3_pool_presentation_state(&pool, Product::Baseq3, 0, &strings);
        assert_eq!(state.entities.len(), 1);
        assert_eq!(state.entities[0].origin, qa_core::math::vec3(1.0, 2.0, 3.0));
        assert!(state.entities[0].linked);
        assert_eq!(state.clients.len(), 1);
        assert_eq!(state.clients[0].slot, 1);
        assert_eq!(state.clients[0].state.client_num, 1);
        assert_eq!(state.clients[0].actor, state.entities[0].actor);
        assert!(q3_pool_models(&pool, Product::Baseq3, 0, &content(), &strings).is_empty());
    }

    #[test]
    fn item_models_expand_world_models() {
        let pool = pool(1);
        {
            let entity = pool.at(5);
            let mut entity = entity.borrow_mut();
            entity.inuse = true;
            entity.r.linked = true;
            entity.s.e_type = EntityType::EtItem as i32;
            entity.s.modelindex = 1;
        }
        let strings = MapStrings { values: HashMap::new() };
        let models = q3_pool_models(&pool, Product::Baseq3, 0, &content(), &strings);
        let expected: Vec<String> = item_at(Product::Baseq3, 1)
            .expect("item 1 exists")
            .world_models
            .iter()
            .flatten()
            .filter(|model| !model.is_empty())
            .map(|model| model.to_string())
            .collect();
        assert!(!expected.is_empty());
        let paths: Vec<&str> = models.iter().map(|model| model.path.as_str()).collect();
        assert_eq!(paths, expected);
        for model in &models {
            assert_eq!(model.family, GameFamily::Q3);
            assert!(model.render_source_client);
            assert!(model.visible);
            assert!(!model.view_weapon);
        }
    }

    #[test]
    fn missile_mover_and_configstring_models() {
        let pool = pool(1);
        {
            let entity = pool.at(5);
            let mut entity = entity.borrow_mut();
            entity.inuse = true;
            entity.r.linked = true;
            entity.s.e_type = EntityType::EtMissile as i32;
            entity.s.weapon = Weapon::WpRocketLauncher as i32;
        }
        {
            let entity = pool.at(6);
            let mut entity = entity.borrow_mut();
            entity.inuse = true;
            entity.r.linked = true;
            entity.s.e_type = EntityType::EtMover as i32;
            entity.s.modelindex = 3;
        }
        {
            let entity = pool.at(7);
            let mut entity = entity.borrow_mut();
            entity.inuse = true;
            entity.r.linked = true;
            entity.s.e_type = EntityType::EtGeneral as i32;
            entity.s.modelindex = 2;
        }
        let strings = MapStrings {
            values: HashMap::from([(34, "models/mapobjects/tree.md3".to_string())]),
        };
        let models = q3_pool_models(&pool, Product::Baseq3, 0, &content(), &strings);
        let paths: Vec<&str> = models.iter().map(|model| model.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["models/ammo/rocket/rocket.md3", "*3", "models/mapobjects/tree.md3",]
        );
    }

    #[test]
    fn filtered_entities_emit_no_models() {
        let pool = pool(1);
        for (slot, flags) in [(5, ServerEntityFlags::Noclient.bits()), (6, 0)] {
            let entity = pool.at(slot);
            let mut entity = entity.borrow_mut();
            entity.inuse = true;
            entity.r.linked = true;
            entity.r.sv_flags = flags;
            entity.s.e_type = EntityType::EtGeneral as i32;
            entity.s.modelindex = 2;
            if slot == 6 {
                entity.s.e_flags = 0x80;
            }
        }
        {
            let entity = pool.at(8);
            let mut entity = entity.borrow_mut();
            entity.inuse = true;
            entity.r.linked = false;
            entity.s.e_type = EntityType::EtGeneral as i32;
            entity.s.modelindex = 2;
        }
        {
            let entity = pool.at(9);
            let mut entity = entity.borrow_mut();
            entity.inuse = true;
            entity.r.linked = true;
            entity.s.e_type = EntityType::EtMissile as i32;
            entity.s.weapon = Weapon::WpNone as i32;
        }
        let strings = MapStrings {
            values: HashMap::from([(34, "models/mapobjects/tree.md3".to_string())]),
        };
        assert!(q3_pool_models(&pool, Product::Baseq3, 0, &content(), &strings).is_empty());
    }
}
