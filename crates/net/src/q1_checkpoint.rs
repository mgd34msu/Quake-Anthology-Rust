//! NetQuake/QuakeWorld decoder checkpoint saves.
//!
//! Donor provenance: `captureWireEntity`, `readNqWireEntity`,
//! `captureQwWireEntity`, and `readQwWireEntity` in
//! `src/network/q1/decoder-checkpoint.ts`. The donor reads saves through
//! `SaveReader` from `src/persistence/value.ts`; here saves are the shared
//! [`Json`](crate::common::session::Json) value model, with a small
//! path-tracking reader over it mirroring the donor's `field` / `list` /
//! `number` / `integer` surface and `path: message` failures. Entity state
//! reuses [`EntityState`](crate::q1::EntityState) and
//! [`QwEntityState`](crate::qw::QwEntityState), whose `alpha` / `scale` /
//! `flags` fields exist for these checkpoints.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::common::session::Json;
use crate::q1::EntityState;
use crate::qw::QwEntityState;

/// Checkpoint failure (`SaveFormatError` shape: `path: message`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q1CheckpointError {
    /// A save value failed validation at a path.
    #[error("{path}: {message}")]
    BadSave {
        /// Dotted save path (`save.origin[0]`).
        path: String,
        /// Donor failure text.
        message: String,
    },
}

/// Path-tracking reader over the shared save value model.
struct SaveReader<'a> {
    value: &'a Json,
    path: String,
}

impl<'a> SaveReader<'a> {
    /// Read the root value.
    fn root(value: &'a Json) -> Self {
        Self {
            value,
            path: "save".to_owned(),
        }
    }

    /// Build a failure at this reader's path.
    fn fail<T>(&self, message: &str) -> Result<T, Q1CheckpointError> {
        Err(Q1CheckpointError::BadSave {
            path: self.path.clone(),
            message: message.to_owned(),
        })
    }

    /// Read a record field (`field`); a missing key yields a null child
    /// that fails at the accessor, like the donor's `undefined` child.
    fn field(&self, name: &str) -> Result<SaveReader<'a>, Q1CheckpointError> {
        match self.value {
            Json::Object(fields) => Ok(SaveReader {
                value: fields.get(name).unwrap_or(&Json::Null),
                path: format!("{}.{name}", self.path),
            }),
            _ => self.fail("expected a record"),
        }
    }

    /// Read a number (`number`).
    fn number(&self) -> Result<f64, Q1CheckpointError> {
        match self.value {
            Json::Number(value) => Ok(*value),
            _ => self.fail("expected a number"),
        }
    }

    /// Read a safe integer at or above `minimum` (`integer`).
    fn integer(&self, minimum: i64) -> Result<i64, Q1CheckpointError> {
        let value = self.number()?;
        if value.fract() != 0.0 || value.abs() > 9_007_199_254_740_992.0 || value < minimum as f64 {
            return self.fail("expected an integer in range");
        }
        Ok(value as i64)
    }

    /// Read an array (`list`).
    fn list(&self) -> Result<Vec<SaveReader<'a>>, Q1CheckpointError> {
        match self.value {
            Json::Array(items) => Ok(items
                .iter()
                .enumerate()
                .map(|(index, value)| SaveReader {
                    value,
                    path: format!("{}[{index}]", self.path),
                })
                .collect()),
            _ => self.fail("expected an array"),
        }
    }
}

/// Read a three-component vector (`vector`).
fn vector(reader: &SaveReader<'_>) -> Result<[f64; 3], Q1CheckpointError> {
    let items = reader.list()?;
    if items.len() != 3 {
        return reader.fail("expected a three component vector");
    }
    Ok([items[0].number()?, items[1].number()?, items[2].number()?])
}

/// Shared checkpoint fields captured for both protocols.
struct BaseCheckpoint {
    origin: [f64; 3],
    angles: [f64; 3],
    modelindex: i64,
    frame: i64,
    colormap: i64,
    skin: i64,
    alpha: i64,
    scale: i64,
    effects: i64,
}

/// Capture the shared fields (`captureWireEntity`).
fn capture_base(checkpoint: &BaseCheckpoint) -> BTreeMap<String, Json> {
    BTreeMap::from([
        (
            "origin".to_owned(),
            Json::Array(checkpoint.origin.iter().map(|value| Json::Number(*value)).collect()),
        ),
        (
            "angles".to_owned(),
            Json::Array(checkpoint.angles.iter().map(|value| Json::Number(*value)).collect()),
        ),
        ("modelindex".to_owned(), Json::Number(checkpoint.modelindex as f64)),
        ("frame".to_owned(), Json::Number(checkpoint.frame as f64)),
        ("colormap".to_owned(), Json::Number(checkpoint.colormap as f64)),
        ("skin".to_owned(), Json::Number(checkpoint.skin as f64)),
        ("alpha".to_owned(), Json::Number(checkpoint.alpha as f64)),
        ("scale".to_owned(), Json::Number(checkpoint.scale as f64)),
        ("effects".to_owned(), Json::Number(checkpoint.effects as f64)),
    ])
}

/// Restore the shared fields (`restoreFields`).
fn read_base(reader: &SaveReader<'_>) -> Result<BaseCheckpoint, Q1CheckpointError> {
    Ok(BaseCheckpoint {
        origin: vector(&reader.field("origin")?)?,
        angles: vector(&reader.field("angles")?)?,
        modelindex: reader.field("modelindex")?.integer(0)?,
        frame: reader.field("frame")?.integer(0)?,
        colormap: reader.field("colormap")?.integer(0)?,
        skin: reader.field("skin")?.integer(i64::MIN)?,
        alpha: reader.field("alpha")?.integer(0)?,
        scale: reader.field("scale")?.integer(0)?,
        effects: reader.field("effects")?.integer(i64::MIN)?,
    })
}

/// Capture a NetQuake wire entity (`captureWireEntity`).
#[must_use]
pub fn capture_wire_entity(state: &EntityState) -> Json {
    Json::Object(capture_base(&BaseCheckpoint {
        origin: state.origin,
        angles: state.angles,
        modelindex: i64::from(state.modelindex),
        frame: i64::from(state.frame),
        colormap: i64::from(state.colormap),
        skin: i64::from(state.skin),
        alpha: i64::from(state.alpha),
        scale: i64::from(state.scale),
        effects: i64::from(state.effects),
    }))
}

/// Restore a NetQuake wire entity (`readNqWireEntity`).
pub fn read_nq_wire_entity(value: &Json) -> Result<EntityState, Q1CheckpointError> {
    let reader = SaveReader::root(value);
    let checkpoint = read_base(&reader)?;
    Ok(EntityState {
        modelindex: checkpoint.modelindex as u16,
        frame: checkpoint.frame as u16,
        colormap: checkpoint.colormap as u8,
        skin: checkpoint.skin as u8,
        effects: checkpoint.effects as u8,
        alpha: checkpoint.alpha as u8,
        scale: checkpoint.scale as u8,
        origin: checkpoint.origin,
        angles: checkpoint.angles,
    })
}

/// Capture a QuakeWorld wire entity (`captureQwWireEntity`).
#[must_use]
pub fn capture_qw_wire_entity(state: &QwEntityState) -> Json {
    let mut fields = capture_base(&BaseCheckpoint {
        origin: state.origin,
        angles: state.angles,
        modelindex: i64::from(state.modelindex),
        frame: i64::from(state.frame),
        colormap: i64::from(state.colormap),
        skin: i64::from(state.skinnum),
        alpha: i64::from(state.alpha),
        scale: i64::from(state.scale),
        effects: i64::from(state.effects),
    });
    fields.insert("number".to_owned(), Json::Number(f64::from(state.number)));
    fields.insert("flags".to_owned(), Json::Number(f64::from(state.flags)));
    Json::Object(fields)
}

/// Restore a QuakeWorld wire entity (`readQwWireEntity`).
pub fn read_qw_wire_entity(value: &Json) -> Result<QwEntityState, Q1CheckpointError> {
    let reader = SaveReader::root(value);
    let checkpoint = read_base(&reader)?;
    Ok(QwEntityState {
        number: reader.field("number")?.integer(0)? as u16,
        origin: checkpoint.origin,
        angles: checkpoint.angles,
        modelindex: checkpoint.modelindex as u8,
        frame: checkpoint.frame as u8,
        colormap: checkpoint.colormap as u8,
        skinnum: checkpoint.skin as u8,
        effects: checkpoint.effects as u8,
        flags: reader.field("flags")?.integer(i64::MIN)? as u16,
        alpha: checkpoint.alpha as u8,
        scale: checkpoint.scale as u8,
        solid: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::session::canonical;

    fn nq_state() -> EntityState {
        EntityState {
            modelindex: 3,
            frame: 7,
            colormap: 11,
            skin: 2,
            effects: 5,
            alpha: 0,
            scale: 16,
            origin: [12.5, -4.25, 100.0],
            angles: [0.0, 90.0, 180.0],
        }
    }

    fn qw_state() -> QwEntityState {
        QwEntityState {
            number: 41,
            origin: [1.0, 2.0, 3.0],
            angles: [10.0, 20.0, 30.0],
            modelindex: 4,
            frame: 8,
            colormap: 12,
            skinnum: 3,
            effects: 6,
            flags: 9,
            alpha: 1,
            scale: 16,
            solid: true,
        }
    }

    #[test]
    fn nq_checkpoint_round_trip() {
        let state = nq_state();
        let restored = read_nq_wire_entity(&capture_wire_entity(&state)).unwrap();
        assert_eq!(restored, state);
    }

    #[test]
    fn nq_capture_shape_matches_donor() {
        let captured = capture_wire_entity(&nq_state());
        let text = canonical(&captured).unwrap();
        assert!(text.contains("\"modelindex\":3"), "{text}");
        assert!(text.contains("\"skin\":2"), "{text}");
        assert!(text.contains("\"scale\":16"), "{text}");
        assert!(text.contains("\"origin\":[12.5,-4.25,100]"), "{text}");
    }

    #[test]
    fn qw_checkpoint_round_trip() {
        let state = qw_state();
        let restored = read_qw_wire_entity(&capture_qw_wire_entity(&state)).unwrap();
        assert_eq!(restored.number, state.number);
        assert_eq!(restored.flags, state.flags);
        assert_eq!(restored.skinnum, state.skinnum);
        assert_eq!(restored.origin, state.origin);
        assert_eq!(restored.alpha, state.alpha);
        // `solid` is prediction state, not checkpoint data.
        assert!(!restored.solid);
    }

    #[test]
    fn qw_capture_shape_matches_donor() {
        let captured = capture_qw_wire_entity(&qw_state());
        let text = canonical(&captured).unwrap();
        assert!(text.contains("\"number\":41"), "{text}");
        assert!(text.contains("\"flags\":9"), "{text}");
        assert!(text.contains("\"skin\":3"), "{text}");
    }

    #[test]
    fn short_vector_fails_with_path() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "origin".to_owned(),
            Json::Array(vec![Json::Number(1.0), Json::Number(2.0)]),
        );
        let error = read_nq_wire_entity(&Json::Object(fields)).unwrap_err();
        assert_eq!(
            error,
            Q1CheckpointError::BadSave {
                path: "save.origin".to_owned(),
                message: "expected a three component vector".to_owned(),
            }
        );
        assert_eq!(error.to_string(), "save.origin: expected a three component vector");
    }

    #[test]
    fn missing_field_fails_with_path() {
        let error = read_qw_wire_entity(&Json::Object(BTreeMap::new())).unwrap_err();
        assert_eq!(
            error,
            Q1CheckpointError::BadSave {
                path: "save.origin".to_owned(),
                message: "expected an array".to_owned(),
            }
        );
    }

    #[test]
    fn negative_bounded_integer_fails() {
        let mut value = capture_wire_entity(&nq_state());
        let Json::Object(fields) = &mut value else {
            panic!("capture must be an object");
        };
        fields.insert("modelindex".to_owned(), Json::Number(-1.0));
        let error = read_nq_wire_entity(&value).unwrap_err();
        assert!(error.to_string().contains("save.modelindex"), "{error}");
    }
}
