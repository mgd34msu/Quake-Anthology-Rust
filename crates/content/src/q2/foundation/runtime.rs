//! Map admission (`src/content/q2/foundation/runtime.ts`).
//!
//! Authored Q2 map admission and native team assembly over the reusable
//! entity services.

use std::collections::HashMap;

use qa_core::identity::OwnedActor;

use super::fields::{inhibit_q2_spawn, parse_q2_entities};
use super::host::{Q2Edition, Q2GameServices, Q2Mode, Q2MotionKind, Q2SpawnFields};
use crate::contract::MonsterDefinitionReference;
use crate::monsters::provider_text;

/// Map spawn report (`Q2SpawnReport`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2SpawnReport {
    /// Authored entity count.
    pub authored: usize,
    /// Inhibited spawn fields.
    pub inhibited: Vec<Q2SpawnFields>,
    /// Fields removed by source spawns.
    pub removed_by_source: Vec<Q2SpawnFields>,
    /// Spawned actors.
    pub spawned: Vec<qa_core::identity::ActorId>,
    /// Unsupported actors.
    pub unsupported: Vec<qa_core::identity::ActorId>,
    /// Replaced (foreign monster) actors.
    pub replaced: Vec<OwnedActor>,
}

impl Q2GameServices {
    /// Load authored entities (`load`).
    pub fn load_source(&mut self, source: &str) -> Q2SpawnReport {
        let fields = parse_q2_entities(source, self.options.edition);
        let mut replacements: HashMap<i32, MonsterDefinitionReference> = HashMap::new();
        if self.monster_admission.is_some() {
            for field in &fields {
                if inhibit_q2_spawn(field, &self.options)
                    || self.options.mode == Q2Mode::Deathmatch && field.classname.starts_with("monster_")
                {
                    continue;
                }
                let admission = self.monster_admission.as_ref().expect("admission");
                if let Some(definition) = (admission.resolve)(&field.classname, field) {
                    replacements.insert(field.ordinal, definition);
                }
            }
        }
        let mut inhibited = Vec::new();
        let mut removed_by_source = Vec::new();
        let mut spawned = Vec::new();
        let mut replaced = Vec::new();
        for field in &fields {
            if inhibit_q2_spawn(field, &self.options) {
                inhibited.push(field.clone());
                continue;
            }
            if let Some(definition) = replacements.get(&field.ordinal) {
                let definition = definition.clone();
                let spawn_admission = self.monster_admission.as_ref().expect("admission").spawn;
                let text = format!(
                    "{}/{}",
                    provider_text(&definition.source.provider),
                    definition.classname
                );
                let actor = self.allocate_actor(field, Some(&text));
                self.create_combat(&actor, 0.0, 100.0, false);
                spawn_admission(self, &actor, field, &definition);
                replaced.push(actor);
                continue;
            }
            let actor = self.spawn(field.clone());
            if self.host.actors().is_live(&actor) {
                spawned.push(actor);
            } else {
                removed_by_source.push(field.clone());
            }
        }
        self.find_teams();
        let unsupported: Vec<qa_core::identity::ActorId> = self.unsupported.iter().cloned().collect();
        Q2SpawnReport {
            authored: fields.len(),
            inhibited,
            removed_by_source,
            spawned,
            replaced,
            unsupported,
        }
    }

    /// `G_FindTeams`, followed by the rerelease's Rogue `G_FixTeams`
    /// train repair (`findTeams`).
    fn find_teams(&mut self) {
        let mut actors: Vec<qa_core::identity::ActorId> = self.entities.keys().cloned().collect();
        actors.sort_by_key(|actor| self.source_slots.get(actor).copied().unwrap_or(0));
        for master in actors.clone() {
            let (name, flags) = match self.entity(&master) {
                Some(entity) => (
                    entity.spawn.values.get("team").cloned().unwrap_or_default(),
                    entity.flags,
                ),
                None => continue,
            };
            if name.is_empty() || flags & 0x400 != 0 {
                continue;
            }
            {
                let rerelease = self.options.edition == Q2Edition::Rerelease;
                let entity = self.require_entity_mut(&master);
                entity.team_master = Some(master.clone());
                if rerelease {
                    entity.flags |= 0x4000000;
                }
            }
            let mut chain = master.clone();
            for member in actors.clone() {
                if member == master {
                    continue;
                }
                let slot = self.source_slots.get(&member).copied().unwrap_or(0);
                let master_slot = self.source_slots.get(&master).copied().unwrap_or(0);
                let (same_team, flagged) = match self.entity(&member) {
                    Some(entity) => (
                        entity.spawn.values.get("team").map(String::as_str) == Some(name.as_str()),
                        entity.flags,
                    ),
                    None => continue,
                };
                if slot <= master_slot || !same_team || flagged & 0x400 != 0 {
                    continue;
                }
                {
                    let entity = self.require_entity_mut(&chain);
                    entity.team_chain = Some(member.clone());
                }
                {
                    let entity = self.require_entity_mut(&member);
                    entity.team_master = Some(master.clone());
                    entity.flags |= 0x400;
                }
                chain = member;
            }
        }
        if self.options.edition != Q2Edition::Rerelease {
            return;
        }
        for master in actors.clone() {
            let (name, classname, spawnflags, flags) = match self.entity(&master) {
                Some(entity) => (
                    entity.spawn.values.get("team").cloned().unwrap_or_default(),
                    entity.classname.clone(),
                    entity.spawnflags,
                    entity.flags,
                ),
                None => continue,
            };
            if name.is_empty() || classname != "func_train" || spawnflags & 8 == 0 || flags & 0x400 == 0 {
                continue;
            }
            {
                let entity = self.require_entity_mut(&master);
                entity.team_master = Some(master.clone());
                entity.team_chain = None;
                entity.flags = entity.flags & !0x400 | 0x4000000;
            }
            let mut chain = master.clone();
            for member in actors.clone() {
                if member == master {
                    continue;
                }
                let same_team = match self.entity(&member) {
                    Some(entity) => entity.spawn.values.get("team").map(String::as_str) == Some(name.as_str()),
                    None => continue,
                };
                if !same_team {
                    continue;
                }
                let speed = self.require_entity(&master).speed;
                {
                    let entity = self.require_entity_mut(&chain);
                    entity.team_chain = Some(member.clone());
                }
                {
                    let entity = self.require_entity_mut(&member);
                    entity.team_master = Some(master.clone());
                    entity.team_chain = None;
                    entity.flags = entity.flags & !0x4000000 | 0x400;
                    entity.speed = speed;
                }
                self.set_motion_kind(member.clone(), Q2MotionKind::Push);
                chain = member;
            }
        }
    }
}
