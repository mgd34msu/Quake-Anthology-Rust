//! Q1 map admission (`src/content/q1/foundation/runtime.ts`).
//!
//! Map admission adds authored geometry and skill/deathmatch
//! inhibition to the shared entity services. Worlds, inline brush
//! bounds, and monster admission replacements follow the donor; door
//! linking and precache freezing close the spawn pass.

use std::collections::HashMap;

use qa_core::identity::OwnedActor;
use qa_core::math::Vec3;

use crate::bsp::{q1_entity_value, Q1Entity, Q1Map};
use crate::contract::{ArmorState, MonsterDefinitionReference, PoweredProtectionState, RegularArmorState};

use super::entity::{parse_vector, source_angles};
use super::entity_services::Q1EntityServices;
use super::gameplay::{BodyState, CombatState};
use super::movers::link_doors;
use super::types::{vadd, vsub, POINT, ZERO};
use crate::q1::{q1_error, Q1Error};

/// Monster admission overrides (`Q1Foundation.monsterAdmission`).
pub trait Q1MonsterAdmission {
    /// Resolve a map classname to a monster definition.
    fn resolve(&self, classname: &str, source: &Q1Entity) -> Option<MonsterDefinitionReference>;
    /// Spawn an admitted monster into shared tables.
    fn spawn(&mut self, actor: &OwnedActor, source: &Q1Entity, ordinal: i32, definition: &MonsterDefinitionReference);
}

/// Inhibited map entity report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1InhibitedEntity {
    /// Source ordinal.
    pub ordinal: usize,
    /// Authored classname.
    pub classname: String,
    /// Inhibition reason.
    pub reason: String,
}

/// Compiler-only map entity report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1CompilerOnlyEntity {
    /// Source ordinal.
    pub ordinal: usize,
    /// Authored classname.
    pub classname: String,
}

/// Map spawn report (`Q1SpawnReport`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SpawnReport {
    /// Spawned presentations.
    pub spawned: Vec<super::types::Q1Presentation>,
    /// Inhibited entities.
    pub inhibited: Vec<Q1InhibitedEntity>,
    /// Compiler-only entities.
    pub compiler_only: Vec<Q1CompilerOnlyEntity>,
}

impl Q1EntityServices {
    /// Spawn every map entity (`Q1Foundation.spawnMap`).
    pub fn spawn_map(&mut self, map: &Q1Map<'_>) -> Result<Q1SpawnReport, Q1Error> {
        if !self.entity_ids().is_empty() {
            return Err(q1_error("Q1 map entities already spawned"));
        }
        let mut replacements: HashMap<usize, MonsterDefinitionReference> = HashMap::new();
        if self.monster_admission.is_some() {
            for (ordinal, source) in map.entity_list.iter().enumerate() {
                let classname = q1_entity_value(source, "classname").unwrap_or("");
                let flags = q1_entity_value(source, "spawnflags")
                    .and_then(|flags| flags.parse::<f64>().ok())
                    .unwrap_or(0.0) as i32;
                if self.inhibit(classname, flags).is_some() {
                    continue;
                }
                let definition = self
                    .monster_admission
                    .as_ref()
                    .and_then(|admission| admission.resolve(classname, source));
                if let Some(definition) = definition {
                    replacements.insert(ordinal, definition);
                }
            }
        }
        let file = map.source.rsplit(['/', '\\']).next().unwrap_or(&map.source);
        self.map_name = file.strip_suffix(".bsp").unwrap_or(file).to_string();
        let map_name = self.map_name.clone();
        let max_clients = self.options().max_clients.unwrap_or(0);
        self.precaches
            .begin_world(&format!("maps/{map_name}.bsp"), map.models.len() as i32 - 1)?;
        self.next_dynamic_slot = max_clients.max(0) as u32 + map.entity_list.len() as u32;
        let mut inhibited = Vec::new();
        let mut compiler_only = Vec::new();
        for (ordinal, source) in map.entity_list.iter().enumerate() {
            let classname = q1_entity_value(source, "classname").unwrap_or("").to_string();
            let flags = q1_entity_value(source, "spawnflags")
                .and_then(|flags| flags.parse::<f64>().ok())
                .unwrap_or(0.0) as i32;
            if let Some(reason) = self.inhibit(&classname, flags) {
                inhibited.push(Q1InhibitedEntity {
                    ordinal,
                    classname,
                    reason,
                });
                continue;
            }
            // info_null is explicitly removed by misc.qc after the
            // compiler uses its lighting target.
            if classname == "info_null" {
                compiler_only.push(Q1CompilerOnlyEntity { ordinal, classname });
                continue;
            }
            if let Some(definition) = replacements.get(&ordinal) {
                let slot = if ordinal == 0 {
                    0
                } else {
                    u32::try_from(ordinal as i64 + i64::from(max_clients))
                        .map_err(|_| q1_error("Invalid Q1 source slot"))?
                };
                let provider = self.provider();
                let label = format!("{}/{}", definition.source.provider.name, definition.classname);
                let actor = self.host.actors.allocate_at_source(&provider, slot, &label)?;
                self.host.bodies.create(
                    &actor,
                    &BodyState {
                        origin: parse_vector(q1_entity_value(source, "origin").unwrap_or("")),
                        angles: source_angles(source),
                        velocity: ZERO,
                        bounds: POINT,
                        ground: None,
                    },
                )?;
                self.host.combat.create(
                    &actor,
                    &CombatState {
                        health: 0.0,
                        armor: ArmorState {
                            regular: RegularArmorState::None,
                            powered: PoweredProtectionState::None,
                        },
                        mass: 100.0,
                        can_take_damage: false,
                        invulnerable: false,
                        no_knockback: None,
                        team: None,
                    },
                )?;
                let ordinal = i32::try_from(ordinal).map_err(|_| q1_error("Invalid Q1 source slot"))?;
                if let Some(admission) = self.monster_admission.as_mut() {
                    admission.spawn(&actor, source, ordinal, definition);
                }
                continue;
            }
            let ordinal_arg = i32::try_from(ordinal).map_err(|_| q1_error("Invalid Q1 source slot"))?;
            let id = self.create(&classname, Some(source), Some(ordinal_arg))?;
            let body = self.body(&id)?;
            let model = self
                .entity_ref(&id)
                .map(|entity| entity.model.clone())
                .unwrap_or_default();
            if let Some(index) = model.strip_prefix('*') {
                let model_index: usize = index
                    .parse()
                    .map_err(|_| q1_error(format!("Missing inline model {model}")))?;
                let brush = map
                    .models
                    .get(model_index)
                    .ok_or_else(|| q1_error(format!("Missing inline model {model}")))?;
                // Mod_LoadBrushModel expands each authored bound by
                // one; QC self.size includes that expansion.
                let owned = self
                    .entity_ref(&id)
                    .map(|entity| entity.actor.clone())
                    .expect("map actor");
                let mut updated = body;
                let brush_min = Vec3 {
                    x: brush.bounds.min[0],
                    y: brush.bounds.min[1],
                    z: brush.bounds.min[2],
                };
                let brush_max = Vec3 {
                    x: brush.bounds.max[0],
                    y: brush.bounds.max[1],
                    z: brush.bounds.max[2],
                };
                updated.bounds = qa_core::math::Bounds {
                    min: vsub(brush_min, Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
                    max: vadd(brush_max, Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
                };
                self.host.bodies.write(&owned, &updated)?;
            }
            self.spawn_entity(&id, None)?;
        }
        link_doors(self)?;
        self.precaches.freeze();
        Ok(Q1SpawnReport {
            spawned: self.presentations(),
            inhibited,
            compiler_only,
        })
    }

    /// Skill/deathmatch inhibition reason, if any.
    fn inhibit(&self, classname: &str, flags: i32) -> Option<String> {
        if self.options().deathmatch != 0 && ((flags & 2048) != 0 || classname.starts_with("monster_")) {
            return Some(String::from("deathmatch"));
        }
        if self.options().deathmatch == 0 {
            let bit = if self.options().skill == 0 {
                256
            } else if self.options().skill == 1 {
                512
            } else {
                1024
            };
            if (flags & bit) != 0 {
                return Some(format!("skill-{}", self.options().skill));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::super::host::mock::{mock_host, MockEvents};
    use super::super::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};
    use super::*;
    use crate::bsp::{BspFormat, WorldModel};
    use crate::common::Bounds as CommonBounds;

    fn options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    fn game() -> (Q1EntityServices, std::rc::Rc<std::cell::RefCell<MockEvents>>) {
        let (host, events) = mock_host();
        (Q1EntityServices::new(host, options()).expect("game"), events)
    }

    fn source(classname: &str, properties: &[(&str, &str)]) -> Q1Entity {
        let mut owned = vec![(String::from("classname"), String::from(classname))];
        for (key, value) in properties {
            owned.push((String::from(*key), String::from(*value)));
        }
        Q1Entity { properties: owned }
    }

    #[test]
    fn inhibit_matches_skill_and_deathmatch() {
        let (game, _) = game();
        assert_eq!(game.inhibit("monster_army", 512), Some(String::from("skill-1")));
        assert_eq!(game.inhibit("monster_army", 256), None);
        assert_eq!(game.inhibit("light", 512), Some(String::from("skill-1")));
        assert_eq!(game.inhibit("light", 0), None);
    }

    #[test]
    fn spawn_map_admits_and_reports() {
        let (mut game, _) = game();
        let map = Q1Map {
            format: BspFormat::Bsp29,
            source: String::from("maps/e1m1.bsp"),
            version: 29,
            data: &[],
            lumps: Vec::new(),
            entities: String::new(),
            entity_list: vec![
                source("worldspawn", &[("worldtype", "0")]),
                source("info_null", &[]),
                source("light", &[("spawnflags", "512")]),
                source("trigger_relay", &[("targetname", "r1")]),
            ],
            planes: Vec::new(),
            vertices: Vec::new(),
            textures: Vec::new(),
            texture_offsets: Vec::new(),
            mip_offsets: Vec::new(),
            texture_info: Vec::new(),
            faces: Vec::new(),
            models: vec![WorldModel {
                bounds: CommonBounds {
                    min: [0.0; 3],
                    max: [0.0; 3],
                },
                origin: [0.0; 3],
                headnodes: [0; 4],
                visible_leaves: 0,
                face_first: 0,
                face_count: 0,
            }],
            nodes: Vec::new(),
            leaves: Vec::new(),
            edges: Vec::new(),
            clipnodes: Vec::new(),
            surface_edges: Vec::new(),
            leaf_faces: Vec::new(),
            visibility: &[],
            lighting: &[],
        };
        let report = game.spawn_map(&map).expect("spawn");
        assert_eq!(game.map_name, "e1m1");
        assert_eq!(game.world_type, 0);
        assert!(game.world.is_some());
        assert_eq!(report.inhibited.len(), 1);
        assert_eq!(report.inhibited[0].classname, "light");
        assert_eq!(report.inhibited[0].reason, "skill-1");
        assert_eq!(report.compiler_only.len(), 1);
        assert_eq!(report.compiler_only[0].classname, "info_null");
        assert_eq!(game.next_dynamic_slot, 8);
        assert!(game.spawn_map(&map).is_err());
    }
}
