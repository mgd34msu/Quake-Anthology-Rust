//! Entity field storage for guest game modules. QC-style gamecode reads
//! entity state through named fields (`origin`, `health`, `enemy`, ...;
//! donor `src/compat/qc/memory.ts` field offsets plus the cleared edict
//! fields in `src/compat/qc/entity-host.ts`: `model`, `takedamage`,
//! `modelindex`, `colormap`, `skin`, `frame`, `solid`, `origin`, `angles`,
//! `nextthink`). This table is the engine side: typed values keyed by
//! actor and field name, with per-module defaults and checkpoints.

use std::collections::HashMap;

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{vec3, Vec3};

use crate::error::GuestError;

/// Typed entity field value.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    /// Single float.
    Float(f32),
    /// Single integer.
    Int(i32),
    /// Three-component vector.
    Vector(Vec3),
    /// Text field.
    Text(String),
    /// Entity reference (null when free).
    Entity(Option<SavedActorId>),
}

impl FieldValue {
    /// Zero value for a field type name (`float`, `int`, `vector`,
    /// `string`, `entity`).
    pub fn zero(type_name: &str) -> Result<Self, GuestError> {
        match type_name {
            "float" => Ok(Self::Float(0.0)),
            "int" => Ok(Self::Int(0)),
            "vector" => Ok(Self::Vector(vec3(0.0, 0.0, 0.0))),
            "string" => Ok(Self::Text(String::new())),
            "entity" => Ok(Self::Entity(None)),
            _ => Err(GuestError::UnknownField(type_name.to_string())),
        }
    }

    /// Read as float.
    pub fn as_float(&self, field: &str) -> Result<f32, GuestError> {
        match *self {
            Self::Float(value) => Ok(value),
            _ => Err(GuestError::FieldType(field.to_string())),
        }
    }

    /// Read as integer.
    pub fn as_int(&self, field: &str) -> Result<i32, GuestError> {
        match *self {
            Self::Int(value) => Ok(value),
            _ => Err(GuestError::FieldType(field.to_string())),
        }
    }

    /// Read as vector.
    pub fn as_vector(&self, field: &str) -> Result<Vec3, GuestError> {
        match self {
            Self::Vector(value) => Ok(*value),
            _ => Err(GuestError::FieldType(field.to_string())),
        }
    }

    /// Read as text.
    pub fn as_text(&self, field: &str) -> Result<&str, GuestError> {
        match self {
            Self::Text(value) => Ok(value),
            _ => Err(GuestError::FieldType(field.to_string())),
        }
    }

    /// Read as entity reference.
    pub fn as_entity(&self, field: &str) -> Result<Option<SavedActorId>, GuestError> {
        match *self {
            Self::Entity(value) => Ok(value),
            _ => Err(GuestError::FieldType(field.to_string())),
        }
    }
}

/// Field defaults for one game module: field name to zero type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldLayout {
    fields: Vec<(String, String)>,
}

impl FieldLayout {
    /// Empty layout.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a field with a zero type name.
    #[must_use]
    pub fn field(mut self, name: &str, type_name: &str) -> Self {
        self.fields.push((name.to_string(), type_name.to_string()));
        self
    }

    /// Declared fields in declaration order.
    #[must_use]
    pub fn fields(&self) -> &[(String, String)] {
        &self.fields
    }

    /// Standard QC entity fields cleared by `ED_ClearEdict`.
    #[must_use]
    pub fn qc_entity() -> Self {
        Self::new()
            .field("model", "string")
            .field("takedamage", "float")
            .field("modelindex", "float")
            .field("colormap", "float")
            .field("skin", "float")
            .field("frame", "float")
            .field("solid", "float")
            .field("origin", "vector")
            .field("angles", "vector")
            .field("nextthink", "float")
    }
}

/// Per-actor entity field storage.
#[derive(Debug, Clone, Default)]
pub struct FieldTable {
    values: HashMap<ActorId, HashMap<String, FieldValue>>,
}

impl FieldTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate an actor's fields from a layout, clearing any previous
    /// values (donor `ED_ClearEdict`).
    pub fn allocate(&mut self, actor: &ActorId, layout: &FieldLayout) -> Result<(), GuestError> {
        let mut fields = HashMap::new();
        for (name, type_name) in layout.fields() {
            fields.insert(name.clone(), FieldValue::zero(type_name)?);
        }
        if fields.contains_key("nextthink") {
            fields.insert("nextthink".to_string(), FieldValue::Float(-1.0));
        }
        self.values.insert(actor.clone(), fields);
        Ok(())
    }

    /// Free an actor's fields.
    pub fn free(&mut self, actor: &ActorId) {
        self.values.remove(actor);
    }

    /// Whether an actor has allocated fields.
    #[must_use]
    pub fn is_allocated(&self, actor: &ActorId) -> bool {
        self.values.contains_key(actor)
    }

    /// Read a field.
    pub fn get(&self, actor: &ActorId, field: &str) -> Result<&FieldValue, GuestError> {
        self.values
            .get(actor)
            .and_then(|fields| fields.get(field))
            .ok_or_else(|| GuestError::UnknownField(field.to_string()))
    }

    /// Write a field; the value type must match the allocated type.
    pub fn set(&mut self, actor: &ActorId, field: &str, value: FieldValue) -> Result<(), GuestError> {
        let fields = self
            .values
            .get_mut(actor)
            .ok_or_else(|| GuestError::UnknownField(field.to_string()))?;
        let slot = fields
            .get_mut(field)
            .ok_or_else(|| GuestError::UnknownField(field.to_string()))?;
        if std::mem::discriminant(slot) != std::mem::discriminant(&value) {
            return Err(GuestError::FieldType(field.to_string()));
        }
        *slot = value;
        Ok(())
    }

    /// Checkpoint all fields.
    #[must_use]
    pub fn checkpoint(&self) -> Vec<FieldCheckpoint> {
        let mut saved: Vec<FieldCheckpoint> = self
            .values
            .iter()
            .flat_map(|(actor, fields)| {
                let saved = SavedActorId::from(actor);
                fields.iter().map(move |(name, value)| FieldCheckpoint {
                    actor: saved,
                    field: name.clone(),
                    value: value.clone(),
                })
            })
            .collect();
        saved.sort_by(|left, right| {
            (left.actor.slot, left.actor.generation, &left.field).cmp(&(
                right.actor.slot,
                right.actor.generation,
                &right.field,
            ))
        });
        saved
    }

    /// Restore fields; every record must name a live actor.
    pub fn restore(
        &mut self,
        live: &dyn Fn(&SavedActorId) -> Option<ActorId>,
        saved: &[FieldCheckpoint],
    ) -> Result<(), GuestError> {
        self.values.clear();
        for record in saved {
            let actor =
                live(&record.actor).ok_or_else(|| GuestError::BadSave("Field names a missing actor".to_string()))?;
            self.values
                .entry(actor)
                .or_default()
                .insert(record.field.clone(), record.value.clone());
        }
        Ok(())
    }
}

/// Saved field record.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldCheckpoint {
    /// Owning actor.
    pub actor: SavedActorId,
    /// Field name.
    pub field: String,
    /// Field value.
    pub value: FieldValue,
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    #[test]
    fn qc_layout_clears_like_ed_clear_edict() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut table = FieldTable::new();
        table.allocate(&actor, &FieldLayout::qc_entity()).unwrap();
        assert_eq!(table.get(&actor, "nextthink").unwrap(), &FieldValue::Float(-1.0));
        assert_eq!(
            table.get(&actor, "origin").unwrap(),
            &FieldValue::Vector(vec3(0.0, 0.0, 0.0))
        );
        assert_eq!(table.get(&actor, "solid").unwrap(), &FieldValue::Float(0.0));
    }

    #[test]
    fn typed_access_rejects_mismatches() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut table = FieldTable::new();
        table.allocate(&actor, &FieldLayout::qc_entity()).unwrap();
        table
            .set(&actor, "origin", FieldValue::Vector(vec3(1.0, 2.0, 3.0)))
            .unwrap();
        assert_eq!(
            table.get(&actor, "origin").unwrap().as_vector("origin").unwrap(),
            vec3(1.0, 2.0, 3.0)
        );
        assert_eq!(
            table.set(&actor, "origin", FieldValue::Float(1.0)),
            Err(GuestError::FieldType("origin".to_string()))
        );
        assert_eq!(
            table.get(&actor, "missing"),
            Err(GuestError::UnknownField("missing".to_string()))
        );
    }

    #[test]
    fn field_checkpoint_round_trip() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(2, 3);
        let mut table = FieldTable::new();
        table.allocate(&actor, &FieldLayout::qc_entity()).unwrap();
        table.set(&actor, "frame", FieldValue::Float(7.0)).unwrap();
        let saved = table.checkpoint();
        assert!(saved.iter().any(|record| record.field == "frame"
            && record.value == FieldValue::Float(7.0)
            && record.actor.slot == 2
            && record.actor.generation == 3));
        let mut restored = FieldTable::new();
        restored
            .restore(&|saved| (saved.slot == 2).then(|| actor.clone()), &saved)
            .unwrap();
        assert_eq!(restored.get(&actor, "frame").unwrap(), &FieldValue::Float(7.0));
        let mut missing = FieldTable::new();
        assert!(missing.restore(&|_| None, &saved).is_err());
    }
}
