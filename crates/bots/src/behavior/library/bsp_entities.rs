//! AAS BSP entities from `src/bots/behavior/library/bsp-entities.ts`
//! (`be_aas_bsp.c` entity lump: `AAS_EntityInfo`, `AAS_ValueForBSPEpairKey`,
//! `AAS_VectorForBSPEpairKey`, `AAS_FloatForBSPEpairKey`,
//! `AAS_IntForBSPEpairKey`).
//!
//! Parses the entity lump into epair maps. Goals resolve item origins,
//! models, and spawn flags here; movers resolve explicit targets.

use std::collections::HashMap;

use qa_core::math::Vec3;

use crate::error::BotsError;

/// Maximum parsed entities.
pub const MAX_BSP_ENTITIES: usize = 4096;

/// One parsed BSP entity: ordered epairs plus a lookup map.
#[derive(Debug, Clone, Default)]
pub struct BspEntity {
    /// Epairs in lump order.
    pub epairs: Vec<(String, String)>,
    /// Lookup map.
    pub map: HashMap<String, String>,
}

impl BspEntity {
    /// Value for a key, or empty.
    #[must_use]
    pub fn value(&self, key: &str) -> &str {
        self.map.get(key).map_or("", String::as_str)
    }

    /// Float value for a key, or 0.
    #[must_use]
    pub fn float(&self, key: &str) -> f32 {
        self.value(key).parse::<f32>().unwrap_or(0.0)
    }

    /// Integer value for a key, or 0.
    #[must_use]
    pub fn int(&self, key: &str) -> i32 {
        self.value(key).parse::<f32>().unwrap_or(0.0) as i32
    }

    /// Vector value for a key, or zero.
    #[must_use]
    pub fn vector(&self, key: &str) -> Vec3 {
        let parts: Vec<f32> = self
            .value(key)
            .split_whitespace()
            .filter_map(|part| part.parse().ok())
            .collect();
        Vec3 {
            x: parts.first().copied().unwrap_or(0.0),
            y: parts.get(1).copied().unwrap_or(0.0),
            z: parts.get(2).copied().unwrap_or(0.0),
        }
    }
}

/// Parsed BSP entity lump.
#[derive(Debug, Clone, Default)]
pub struct AasBspEntities {
    entities: Vec<BspEntity>,
}

impl AasBspEntities {
    /// Empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse the entity lump.
    pub fn load(&mut self, text: &str) -> Result<(), BotsError> {
        let mut entities = Vec::new();
        let mut chars = text.chars().peekable();
        while chars.peek().is_some() {
            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
            if chars.peek().is_none() {
                break;
            }
            if chars.next() != Some('{') {
                return Err(BotsError::BotScript("expected '{' in entity lump".to_owned()));
            }
            let mut entity = BspEntity::default();
            loop {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
                match chars.peek() {
                    None => return Err(BotsError::BotScript("unterminated entity".to_owned())),
                    Some('}') => {
                        chars.next();
                        break;
                    }
                    Some('"') => {
                        let key = read_quoted(&mut chars)?;
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                        }
                        let value = read_quoted(&mut chars)?;
                        entity.map.insert(key.clone(), value.clone());
                        entity.epairs.push((key, value));
                    }
                    Some(other) => {
                        return Err(BotsError::BotScript(format!("unexpected '{other}' in entity lump")));
                    }
                }
            }
            entities.push(entity);
            if entities.len() > MAX_BSP_ENTITIES {
                return Err(BotsError::EntityLimit);
            }
        }
        self.entities = entities;
        Ok(())
    }

    /// Parsed entities.
    #[must_use]
    pub fn entities(&self) -> &[BspEntity] {
        &self.entities
    }

    /// Entity count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether no entities are parsed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Value for an entity key (`AAS_ValueForBSPEpairKey`).
    #[must_use]
    pub fn value_for_key(&self, entity: usize, key: &str) -> &str {
        self.entities.get(entity).map_or("", |entity| entity.value(key))
    }

    /// Vector for an entity key (`AAS_VectorForBSPEpairKey`).
    #[must_use]
    pub fn vector_for_key(&self, entity: usize, key: &str) -> Vec3 {
        self.entities
            .get(entity)
            .map_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 }, |entity| entity.vector(key))
    }

    /// Float for an entity key (`AAS_FloatForBSPEpairKey`).
    #[must_use]
    pub fn float_for_key(&self, entity: usize, key: &str) -> f32 {
        self.entities.get(entity).map_or(0.0, |entity| entity.float(key))
    }

    /// Integer for an entity key (`AAS_IntForBSPEpairKey`).
    #[must_use]
    pub fn int_for_key(&self, entity: usize, key: &str) -> i32 {
        self.entities.get(entity).map_or(0, |entity| entity.int(key))
    }

    /// Next entity with a matching key/value pair (`AAS_NextBSPEntity`).
    pub fn next_entity(&self, after: Option<usize>, key: &str, value: &str) -> Option<usize> {
        let start = after.map_or(0, |index| index + 1);
        self.entities
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, entity)| entity.value(key) == value)
            .map(|(index, _)| index)
    }

    /// Dump entity count for diagnostics.
    #[must_use]
    pub fn dump_summary(&self) -> String {
        format!("{} entities", self.entities.len())
    }

    /// Checkpoint parsed entities.
    #[must_use]
    pub fn checkpoint(&self) -> Vec<BspEntity> {
        self.entities.clone()
    }

    /// Restore checkpointed entities.
    pub fn restore(&mut self, entities: &[BspEntity]) {
        self.entities = entities.to_vec();
    }
}

fn read_quoted(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<String, BotsError> {
    if chars.next() != Some('"') {
        return Err(BotsError::BotScript("expected quoted string".to_owned()));
    }
    let mut text = String::new();
    for c in chars.by_ref() {
        if c == '"' {
            return Ok(text);
        }
        text.push(c);
    }
    Err(BotsError::BotScript("unterminated quoted string".to_owned()))
}
