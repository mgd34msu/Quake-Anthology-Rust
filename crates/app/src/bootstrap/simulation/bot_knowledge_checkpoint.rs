//! Bot knowledge (weapon-handle to actor) checkpoint. Port of
//! `src/app/bootstrap/simulation/bot-knowledge-checkpoint.ts`.

use std::collections::{HashMap, HashSet};

use qa_core::identity::{ActorId, SavedActorId};
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::value::{SaveJson, SaveReader, arr, int, obj, str as json_str};

/// Checkpoint source dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotKnowledgeSource {
    Q1,
    Q2,
    Q3,
}

impl BotKnowledgeSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Q1 => "q1",
            Self::Q2 => "q2",
            Self::Q3 => "q3",
        }
    }
}

/// Bot knowledge checkpoint failure.
#[derive(Debug, thiserror::Error)]
pub enum BotKnowledgeError {
    #[error(transparent)]
    World(#[from] qa_world::WorldError),
}

/// Live weapon-handle to actor map with checkpoint/restore.
#[derive(Debug)]
pub struct BotKnowledgeStore {
    source: BotKnowledgeSource,
    actors: HashMap<u32, ActorId>,
}

impl BotKnowledgeStore {
    #[must_use]
    pub fn new(source: BotKnowledgeSource, actors: HashMap<u32, ActorId>) -> Self {
        Self { source, actors }
    }

    #[must_use]
    pub fn actors(&self) -> &HashMap<u32, ActorId> {
        &self.actors
    }

    #[must_use]
    pub fn checkpoint(&self) -> SaveJson {
        let mut entries: Vec<(u32, &ActorId)> =
            self.actors.iter().map(|(handle, actor)| (*handle, actor)).collect();
        entries.sort_by_key(|(handle, _)| *handle);
        obj(vec![
            ("version", int(1)),
            ("source", json_str(self.source.as_str())),
            (
                "actors",
                arr(entries
                    .into_iter()
                    .map(|(handle, actor)| {
                        obj(vec![
                            ("handle", int(i64::from(handle))),
                            ("actor", write_saved_actor(SavedActorId::from(actor))),
                        ])
                    })
                    .collect()),
            ),
        ])
    }

    /// Restore from `botKnowledge`, remapping saved actors and weapon handles.
    /// The live map is replaced only after the whole checkpoint validates.
    pub fn restore(
        &mut self,
        reader: SaveReader,
        actor: &dyn Fn(SavedActorId) -> ActorId,
        weapon_handle: &dyn Fn(u32) -> i64,
    ) -> Result<(), BotKnowledgeError> {
        reader.field("version").literal_i64(1)?;
        reader.field("source").literal_str(self.source.as_str())?;
        let mut restored: HashMap<u32, ActorId> = HashMap::new();
        let mut seen: HashSet<u32> = HashSet::new();
        reader.field("actors").list(|entry| -> Result<(), BotKnowledgeError> {
            let raw = entry.field("handle").integer(0)?;
            let saved = u32::try_from(raw)
                .map_err(|_| entry.fail("invalid or duplicate weapon handle mapping"))?;
            let handle = weapon_handle(saved);
            let mapped = u32::try_from(handle)
                .map_err(|_| entry.fail("invalid or duplicate weapon handle mapping"))?;
            if seen.contains(&saved) || restored.contains_key(&mapped) {
                return Err(entry.fail("invalid or duplicate weapon handle mapping").into());
            }
            seen.insert(saved);
            restored.insert(mapped, actor(read_saved_actor(entry.field("actor"))?));
            Ok(())
        })?;
        self.actors = restored;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_world::save::value::SaveReader;
    use qa_core::identity::IdentityOwner;

    fn owner() -> IdentityOwner {
        IdentityOwner::create("test").unwrap()
    }

    fn restore(value: &SaveJson, store: &mut BotKnowledgeStore) -> Result<(), BotKnowledgeError> {
        let owner = owner();
        store.restore(
            SaveReader::at(value, "botKnowledge"),
            &|saved| owner.actor(saved.slot, saved.generation),
            &|saved| i64::from(saved),
        )
    }

    #[test]
    fn checkpoint_round_trips() {
        let owner = owner();
        let store = BotKnowledgeStore::new(
            BotKnowledgeSource::Q2,
            HashMap::from([(7u32, owner.actor(3, 1))]),
        );
        let value = store.checkpoint();
        let mut fresh = BotKnowledgeStore::new(BotKnowledgeSource::Q2, HashMap::new());
        restore(&value, &mut fresh).unwrap();
        assert_eq!(fresh.checkpoint(), value);
    }

    #[test]
    fn wrong_version_is_rejected() {
        let value = obj(vec![
            ("version", int(2)),
            ("source", json_str("q2")),
            ("actors", arr(vec![])),
        ]);
        let mut store = BotKnowledgeStore::new(BotKnowledgeSource::Q2, HashMap::new());
        assert!(restore(&value, &mut store).is_err());
    }

    #[test]
    fn wrong_source_is_rejected() {
        let value = obj(vec![
            ("version", int(1)),
            ("source", json_str("q3")),
            ("actors", arr(vec![])),
        ]);
        let mut store = BotKnowledgeStore::new(BotKnowledgeSource::Q2, HashMap::new());
        assert!(restore(&value, &mut store).is_err());
    }

    #[test]
    fn duplicate_weapon_handle_mapping_is_rejected() {
        let entry = obj(vec![
            ("handle", int(4)),
            ("actor", obj(vec![("slot", int(1)), ("generation", int(1))])),
        ]);
        let value = obj(vec![
            ("version", int(1)),
            ("source", json_str("q1")),
            ("actors", arr(vec![entry.clone(), entry])),
        ]);
        let mut store = BotKnowledgeStore::new(BotKnowledgeSource::Q1, HashMap::new());
        let error = restore(&value, &mut store).unwrap_err().to_string();
        assert!(error.contains("invalid or duplicate weapon handle mapping"), "{error}");
        assert!(store.actors().is_empty());
    }
}
