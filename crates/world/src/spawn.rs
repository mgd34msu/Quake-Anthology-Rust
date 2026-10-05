//! Spawn functions: classname dispatch for map entities. The donor binds
//! spawn through gamecode (`ED_ClearEdict`-style edict setup in
//! `src/compat/qc/entity-host.ts` plus per-game spawn tables); this module
//! is the engine side: parse `classname`/origin/angles/spawnflags fields,
//! dispatch to a registered spawn function in deterministic order, and
//! report unknown classnames instead of guessing.

use std::collections::HashMap;

use qa_core::math::{vec3, Vec3};

use crate::combat::CombatState;
use crate::WorldError;

/// Parsed map-entity spawn fields.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpawnFields {
    /// Entity classname.
    pub classname: String,
    /// Spawn origin.
    pub origin: Vec3,
    /// Spawn angles.
    pub angles: Vec3,
    /// Spawn flags bitmask.
    pub spawnflags: i32,
    /// Target entity name.
    pub target: Option<String>,
    /// This entity's target name.
    pub targetname: Option<String>,
    /// Remaining raw fields.
    pub extra: HashMap<String, String>,
}

impl SpawnFields {
    /// Parse spawn fields from key/value pairs. `origin`/`angles` accept
    /// space-separated triples; `spawnflags` parses as `i32`.
    pub fn parse(pairs: &[(&str, &str)]) -> Result<Self, WorldError> {
        let mut fields = Self::default();
        for (key, value) in pairs {
            match *key {
                "classname" => fields.classname = (*value).to_string(),
                "origin" => fields.origin = parse_vec3(value)?,
                "angles" => fields.angles = parse_vec3(value)?,
                // Stock anglehack (`ED_ParseEdict`): QuakeEd writes the
                // scalar `angle` yaw, which rewrites to `angles` as the
                // whole vector `(0, N, 0)`, applied sequentially per key.
                "angle" => fields.angles = parse_angle(value)?,
                "spawnflags" => {
                    fields.spawnflags = value
                        .parse::<i32>()
                        .map_err(|_| WorldError::BadSpawnFields(format!("Invalid spawnflags: {value}")))?;
                }
                "target" => fields.target = Some((*value).to_string()),
                "targetname" => fields.targetname = Some((*value).to_string()),
                _ => {
                    fields.extra.insert((*key).to_string(), (*value).to_string());
                }
            }
        }
        if fields.classname.is_empty() {
            return Err(WorldError::BadSpawnFields("Spawn fields need a classname".to_string()));
        }
        Ok(fields)
    }
}

fn parse_vec3(text: &str) -> Result<Vec3, WorldError> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(WorldError::BadSpawnFields(format!("Invalid vector: {text}")));
    }
    let mut values = [0.0f32; 3];
    for (index, part) in parts.iter().enumerate() {
        let value: f64 = part
            .parse()
            .map_err(|_| WorldError::BadSpawnFields(format!("Invalid vector component: {part}")))?;
        if !value.is_finite() {
            return Err(WorldError::BadSpawnFields(format!("Invalid vector component: {part}")));
        }
        values[index] = value as f32;
    }
    Ok(vec3(values[0], values[1], values[2]))
}

fn parse_angle(text: &str) -> Result<Vec3, WorldError> {
    let yaw: f64 = text
        .parse()
        .map_err(|_| WorldError::BadSpawnFields(format!("Invalid angle: {text}")))?;
    if !yaw.is_finite() {
        return Err(WorldError::BadSpawnFields(format!("Invalid angle: {text}")));
    }
    Ok(vec3(0.0, yaw as f32, 0.0))
}

/// Engine-side spawn request produced by a spawn function.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpawnRequest {
    /// Actor definition (`namespace:name`).
    pub definition: String,
    /// Spawn origin override.
    pub origin: Option<Vec3>,
    /// Initial combat state.
    pub combat: Option<CombatState>,
    /// Initial inventory grants (`item`, `count`).
    pub grants: Vec<(String, f64)>,
}

/// Spawn function: pure mapping from fields to an engine spawn request.
pub type SpawnFunction = Box<dyn Fn(&SpawnFields) -> Result<SpawnRequest, WorldError>>;

/// Classname-to-spawn-function registry.
#[derive(Default)]
pub struct SpawnRegistry {
    functions: HashMap<String, SpawnFunction>,
    order: Vec<String>,
}

impl std::fmt::Debug for SpawnRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SpawnRegistry")
            .field("classnames", &self.order)
            .finish()
    }
}

impl SpawnRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a spawn function for a classname.
    pub fn register(&mut self, classname: &str, function: SpawnFunction) {
        if !self.functions.contains_key(classname) {
            self.order.push(classname.to_string());
        }
        self.functions.insert(classname.to_string(), function);
    }

    /// Registered classnames in registration order.
    #[must_use]
    pub fn classnames(&self) -> &[String] {
        &self.order
    }

    /// Dispatch spawn fields to the registered function.
    pub fn spawn(&self, fields: &SpawnFields) -> Result<SpawnRequest, WorldError> {
        let function = self
            .functions
            .get(&fields.classname)
            .ok_or_else(|| WorldError::UnknownSpawnClass(fields.classname.clone()))?;
        function(fields)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_fields_parse_known_keys() {
        let fields = SpawnFields::parse(&[
            ("classname", "q1:monster_ogre"),
            ("origin", "100 200 32"),
            ("angles", "0 90 0"),
            ("spawnflags", "3"),
            ("targetname", "ogre1"),
            ("health", "200"),
        ])
        .unwrap();
        assert_eq!(fields.classname, "q1:monster_ogre");
        assert_eq!(fields.origin, vec3(100.0, 200.0, 32.0));
        assert_eq!(fields.angles, vec3(0.0, 90.0, 0.0));
        assert_eq!(fields.spawnflags, 3);
        assert_eq!(fields.targetname.as_deref(), Some("ogre1"));
        assert_eq!(fields.extra.get("health").map(String::as_str), Some("200"));
    }

    #[test]
    fn spawn_fields_reject_bad_vectors_and_missing_classname() {
        assert!(SpawnFields::parse(&[("origin", "1 2")]).is_err());
        assert!(SpawnFields::parse(&[("classname", "x"), ("origin", "1 2 oops")]).is_err());
        assert!(SpawnFields::parse(&[("classname", "x"), ("spawnflags", "nope")]).is_err());
        assert!(SpawnFields::parse(&[("origin", "0 0 0")]).is_err());
    }

    #[test]
    fn spawn_fields_anglehack_rewrites_scalar_to_yaw() {
        let fields = SpawnFields::parse(&[("classname", "func_door"), ("angle", "-1")]).unwrap();
        assert_eq!(fields.angles, vec3(0.0, -1.0, 0.0));
        assert!(!fields.extra.contains_key("angle"));
        let fields = SpawnFields::parse(&[("classname", "func_door"), ("angle", "90")]).unwrap();
        assert_eq!(fields.angles, vec3(0.0, 90.0, 0.0));
    }

    #[test]
    fn spawn_fields_angle_and_angles_apply_sequentially() {
        let fields = SpawnFields::parse(&[("classname", "func_door"), ("angle", "90"), ("angles", "0 180 0")]).unwrap();
        assert_eq!(fields.angles, vec3(0.0, 180.0, 0.0));
        let fields = SpawnFields::parse(&[("classname", "func_door"), ("angles", "0 180 0"), ("angle", "90")]).unwrap();
        assert_eq!(fields.angles, vec3(0.0, 90.0, 0.0));
    }

    #[test]
    fn spawn_fields_reject_bad_angles() {
        assert!(SpawnFields::parse(&[("classname", "x"), ("angle", "nope")]).is_err());
        assert!(SpawnFields::parse(&[("classname", "x"), ("angle", "nan")]).is_err());
        assert!(SpawnFields::parse(&[("classname", "x"), ("angle", "1 2")]).is_err());
    }

    #[test]
    fn registry_dispatches_and_reports_unknown() {
        let mut registry = SpawnRegistry::new();
        registry.register(
            "q1:monster_ogre",
            Box::new(|fields| {
                Ok(SpawnRequest {
                    definition: fields.classname.clone(),
                    origin: Some(fields.origin),
                    combat: None,
                    grants: Vec::new(),
                })
            }),
        );
        let fields = SpawnFields::parse(&[("classname", "q1:monster_ogre")]).unwrap();
        let request = registry.spawn(&fields).unwrap();
        assert_eq!(request.definition, "q1:monster_ogre");
        let missing = SpawnFields::parse(&[("classname", "q1:nope")]).unwrap();
        assert_eq!(
            registry.spawn(&missing),
            Err(WorldError::UnknownSpawnClass("q1:nope".to_string()))
        );
    }
}
