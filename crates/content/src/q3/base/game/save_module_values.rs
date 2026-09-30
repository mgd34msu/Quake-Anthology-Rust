//! Quake III base/game: save module values.
//!
//! Donor provenance: `src/content/q3/base/game/save-module-values.ts`.

use crate::value::boolean;
use crate::value::num;
use crate::value::obj;
use crate::value::str;
use crate::value::SaveJson;
use crate::value::SaveReader;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_state::*;
use crate::q3::base::game::save_values::*;

// ---------------------------------------------------------------------------
// save-module-values.ts: module entity and cvar reads
// ---------------------------------------------------------------------------

/// Read a module entity slot (`readModuleEntity`).
pub fn read_module_entity(reader: &SaveReader, pool: &dyn EntityPool) -> Result<usize, Q3GameError> {
    let slot = reader.integer(0)?;
    if slot > 1023 {
        return Err(reader.fail("module source entity slot exceeds table extent").into());
    }
    #[allow(clippy::cast_possible_truncation)]
    let slot = slot as usize;
    if pool.entity(slot).is_none() {
        return Err(reader.fail("module references an absent source entity slot").into());
    }
    Ok(slot)
}

/// Cvar snapshot (`CvarSnapshot`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CvarSnapshot {
    /// Name.
    pub name: String,
    /// Value.
    pub value: String,
    /// Reset value.
    pub reset_value: String,
    /// Latched value.
    pub latched_value: Option<String>,
    /// Flags.
    pub flags: i32,
    /// Modified.
    pub modified: bool,
    /// Modification count.
    pub modification_count: i32,
    /// Numeric value.
    pub numeric_value: f64,
    /// Integer value.
    pub integer_value: i32,
}

/// Capture a module cvar (`captureModuleCvar`).
#[must_use]
pub fn capture_module_cvar(value: &Q3CvarSnapshot) -> SaveJson {
    obj(vec![
        ("name", str(&value.name)),
        ("value", str(&value.value)),
        ("resetValue", str(&value.reset_value)),
        ("latchedValue", opt_str_to_json(value.latched_value.as_deref())),
        ("flags", num_i32(value.flags)),
        ("modified", boolean(value.modified)),
        ("modificationCount", num_i32(value.modification_count)),
        ("numericValue", num(value.numeric_value)),
        ("integerValue", num_i32(value.integer_value)),
    ])
}

/// Read a module cvar (`readModuleCvar`).
pub fn read_module_cvar(reader: &SaveReader) -> Result<Q3CvarSnapshot, Q3GameError> {
    #[allow(clippy::cast_possible_truncation)]
    Ok(Q3CvarSnapshot {
        name: reader.field("name").string()?,
        value: reader.field("value").string()?,
        reset_value: reader.field("resetValue").string()?,
        latched_value: reader.field("latchedValue").nullable(|value| value.string())?,
        flags: reader.field("flags").integer(i64::MIN)? as i32,
        modified: reader.field("modified").boolean()?,
        modification_count: reader.field("modificationCount").integer(i64::MIN)? as i32,
        numeric_value: reader.field("numericValue").number()?,
        integer_value: reader.field("integerValue").integer(i64::MIN)? as i32,
    })
}
