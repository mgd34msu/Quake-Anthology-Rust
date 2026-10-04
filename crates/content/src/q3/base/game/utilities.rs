//! Quake III base/game: utilities.
//!
//! Donor provenance: `src/content/q3/base/game/utilities.ts`.

use qa_core::math::add3;
use qa_core::math::angle_vectors;
use qa_core::math::cross3;
use qa_core::math::dot3;
use qa_core::math::normalize3;
use qa_core::math::scale3;
use qa_core::math::sub3;
use qa_core::math::vec3;
use qa_core::math::Vec3;
use qa_core::numeric::qvm_float_to_int;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::contract::vertical_move_direction;
use crate::q3::base::game::format::{game_format, GameFormatArgument};
use crate::q3::base::game::state::*;
use crate::q3::base::game::state::{
    ascii_lower, failure, latin1_bytes, latin1_string, range, EntityPool, Q3Driver, Q3GameError,
};
use crate::q3::base::shared::definitions::Team;

// ---------------------------------------------------------------------------
// utilities.ts: scratch rings, configstrings, target dispatch
// ---------------------------------------------------------------------------

/// Temporary vector with binary32-storing setters (`TemporaryVector`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TempVector {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Z.
    pub z: f32,
}

/// Fixed 32-byte vector string (`GameMemoryAllocation` ring cell).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3RingString {
    bytes: [u8; 32],
}

impl Q3RingString {
    /// Empty string.
    #[must_use]
    pub fn new() -> Self {
        Self { bytes: [0; 32] }
    }

    /// Write a NUL-terminated string (`writeString`).
    pub fn write_string(&mut self, value: &str) -> Result<(), Q3GameError> {
        let bytes = latin1_bytes(value).map_err(|_| range("Game strings require non-NUL source bytes"))?;
        if bytes.contains(&0) {
            return Err(range("Game strings require non-NUL source bytes"));
        }
        if bytes.len() + 1 > self.bytes.len() {
            return Err(range("Game string exceeds its source allocation"));
        }
        self.bytes.fill(0);
        self.bytes[..bytes.len()].copy_from_slice(&bytes);
        Ok(())
    }

    /// Read the NUL-terminated string (`readString`).
    #[must_use]
    pub fn read_string(&self) -> String {
        let end = self
            .bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(self.bytes.len());
        latin1_string(&self.bytes[..end])
    }
}

impl Default for Q3RingString {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-game scratch rings (`GameUtilityScratch`).
pub struct GameUtilityScratch {
    vectors: [TempVector; 8],
    strings: [Q3RingString; 8],
    vector_index: usize,
    string_index: usize,
    print: Rc<dyn Fn(&str)>,
}

impl GameUtilityScratch {
    /// New scratch with a print hook.
    #[must_use]
    pub fn new(print: Rc<dyn Fn(&str)>) -> Self {
        Self {
            vectors: [TempVector::default(); 8],
            strings: [
                Q3RingString::new(),
                Q3RingString::new(),
                Q3RingString::new(),
                Q3RingString::new(),
                Q3RingString::new(),
                Q3RingString::new(),
                Q3RingString::new(),
                Q3RingString::new(),
            ],
            vector_index: 0,
            string_index: 0,
            print,
        }
    }

    /// Borrow the next temporary vector (`tv`).
    pub fn tv(&mut self, x: f32, y: f32, z: f32) -> TempVector {
        let vector = TempVector { x, y, z };
        self.vectors[self.vector_index] = vector;
        self.vector_index = (self.vector_index + 1) & 7;
        vector
    }

    /// Format a vector into the next ring string (`vtos`).
    pub fn vtos(&mut self, vector: Vec3) -> Result<Q3RingString, Q3GameError> {
        let value = game_format(
            "(%i %i %i)",
            &[
                GameFormatArgument::Int(qvm_float_to_int(vector.x)),
                GameFormatArgument::Int(qvm_float_to_int(vector.y)),
                GameFormatArgument::Int(qvm_float_to_int(vector.z)),
            ],
        );
        if value.len() >= 32 {
            (self.print)(&game_format(
                "Com_sprintf: overflow of %i in %i\n",
                &[GameFormatArgument::Int(value.len() as i32), GameFormatArgument::Int(32)],
            ));
        }
        let mut string = Q3RingString::new();
        string.write_string(&value[..value.len().min(31)])?;
        self.strings[self.string_index] = string.clone();
        self.string_index = (self.string_index + 1) & 7;
        Ok(string)
    }
}

/// Submit a debug line as a four-point polygon (`DebugLine`).
pub fn debug_line(start: Vec3, end: Vec3, color: i32, polygons: &mut dyn DebugPolygons) -> i32 {
    let direction = normalize3(sub3(end, start));
    let up = vec3(0.0, 0.0, 1.0);
    let dot = dot3(direction, up);
    let cross = if dot > 0.99 || dot < -0.99 {
        vec3(1.0, 0.0, 0.0)
    } else {
        normalize3(cross3(direction, up))
    };
    polygons.create(
        color,
        4,
        &[
            add3(start, scale3(cross, 2.0)),
            add3(start, scale3(cross, -2.0)),
            add3(end, scale3(cross, -2.0)),
            add3(end, scale3(cross, 2.0)),
        ],
    )
}

/// Configstring store (`ConfigStringStore`).
pub trait ConfigStringStore {
    /// Read a configstring.
    fn get(&self, index: usize) -> String;
    /// Write a configstring.
    fn set(&mut self, index: usize, value: &str);
}

/// Configstring registry (`ConfigStringRegistry`).
pub struct ConfigStringRegistry<S: ConfigStringStore> {
    store: S,
}

impl<S: ConfigStringStore> ConfigStringRegistry<S> {
    /// New registry over a store.
    #[must_use]
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Find or create a configstring index (`G_FindConfigstringIndex`).
    pub fn find(
        &mut self,
        name: Option<&str>,
        start: usize,
        maximum: usize,
        create: bool,
    ) -> Result<usize, Q3GameError> {
        let Some(name) = name else { return Ok(0) };
        let text = byte_string(name)?;
        if text.is_empty() {
            return Ok(0);
        }
        if start.checked_add(maximum).is_none_or(|end| end > 1024) || maximum < 1 {
            return Err(range("Configstring range must fit MAX_CONFIGSTRINGS"));
        }
        let mut index = 1usize;
        while index < maximum {
            let value = byte_string(&self.store.get(start + index))?;
            let value = value[..value.len().min(1023)].to_string();
            if value.is_empty() {
                break;
            }
            if value == text {
                return Ok(index);
            }
            index += 1;
        }
        if !create {
            return Ok(0);
        }
        if index == maximum {
            return Err(failure("G_FindConfigstringIndex: overflow"));
        }
        self.store.set(start + index, &text);
        Ok(index)
    }

    /// Model index (`modelIndex`).
    pub fn model_index(&mut self, name: Option<&str>) -> Result<usize, Q3GameError> {
        self.find(name, 32, 256, true)
    }

    /// Sound index (`soundIndex`).
    pub fn sound_index(&mut self, name: Option<&str>) -> Result<usize, Q3GameError> {
        self.find(name, 288, 256, true)
    }
}

pub(crate) fn byte_string(value: &str) -> Result<String, Q3GameError> {
    let text = value.split('\0').next().unwrap_or("").to_string();
    for ch in text.chars() {
        if ch as u32 > 255 {
            return Err(range("Game configstrings require byte characters"));
        }
    }
    Ok(text)
}

/// String-valued entity field (`EntityStringField`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityStringField {
    /// Classname.
    Classname,
    /// Model.
    Model,
    /// Model 2.
    Model2,
    /// Message.
    Message,
    /// Target.
    Target,
    /// Target name.
    Targetname,
    /// Team.
    Team,
    /// Target shader name.
    TargetShaderName,
    /// Target shader new name.
    TargetShaderNewName,
}

pub(crate) fn ascii_fold(value: &str) -> Vec<u8> {
    let text = value.split('\0').next().unwrap_or("");
    ascii_lower(text.as_bytes())
}

/// Find an entity by a string field (`findEntity`).
pub fn find_entity(
    pool: &dyn EntityPool,
    after: Option<usize>,
    field: EntityStringField,
    text: Option<&str>,
) -> Option<usize> {
    let text = text?;
    let folded = ascii_fold(text);
    let mut index = after.map_or(0, |slot| slot + 1);
    while index < pool.num_entities() {
        if let Some(entity) = pool.entity(index) {
            let value = match field {
                EntityStringField::Classname => entity.classname_value().map(str::to_string),
                EntityStringField::Model => entity.model.clone(),
                EntityStringField::Model2 => entity.model2.clone(),
                EntityStringField::Message => entity.message.clone(),
                EntityStringField::Target => entity.target.clone(),
                EntityStringField::Targetname => entity.targetname.clone(),
                EntityStringField::Team => entity.team.clone(),
                EntityStringField::TargetShaderName => entity.target_shader_name.clone(),
                EntityStringField::TargetShaderNewName => entity.target_shader_new_name.clone(),
            };
            if entity.inuse {
                if let Some(value) = value {
                    if ascii_fold(&value) == folded {
                        return Some(index);
                    }
                }
            }
        }
        index += 1;
    }
    None
}

/// Pick a random target by targetname (`G_PickTarget`).
pub fn pick_target(driver: &mut dyn Q3Driver, target_name: Option<&str>) -> Result<Option<usize>, Q3GameError> {
    let Some(target_name) = target_name else {
        driver.warn("G_PickTarget called with NULL targetname\n");
        return Ok(None);
    };
    let mut choices = Vec::new();
    let mut found = None;
    while choices.len() < 32 {
        found = find_entity(driver.pool(), found, EntityStringField::Targetname, Some(target_name));
        let Some(slot) = found else { break };
        choices.push(slot);
    }
    if choices.is_empty() {
        driver.warn(&format!("G_PickTarget: target {target_name} not found\n"));
        return Ok(None);
    }
    let random = driver.game_rand();
    if random < 0 {
        return Err(range("Game rand must return a nonnegative integer"));
    }
    Ok(Some(choices[(random as usize) % choices.len()]))
}

/// Dispatch targets and shader remaps (`useTargets`).
pub fn use_targets(driver: &mut dyn Q3Driver, slot: usize, activator: Option<Participant>) -> Result<(), Q3GameError> {
    let (shader_old, shader_new, target) = match driver.pool().entity(slot) {
        Some(entity) => (
            entity.target_shader_name.clone(),
            entity.target_shader_new_name.clone(),
            entity.target.clone(),
        ),
        None => return Ok(()),
    };
    if let (Some(old), Some(new)) = (shader_old, shader_new) {
        let time = driver.combat().time() as f32 * 0.001;
        driver.remap_shader(&old, &new, time);
    }
    let Some(target) = target else { return Ok(()) };
    let mut current = None;
    loop {
        current = find_entity(driver.pool(), current, EntityStringField::Targetname, Some(&target));
        let Some(target_slot) = current else { break };
        if target_slot == slot {
            driver.warn("WARNING: Entity used itself.\n");
        } else {
            let use_callback = driver
                .pool()
                .entity(target_slot)
                .and_then(|entity| entity.use_callback.clone());
            if let Some(use_callback) = use_callback {
                let other = Participant::Entity(slot);
                use_callback(driver, target_slot, Some(&other), activator.as_ref());
            }
        }
        let inuse = driver.pool().entity(slot).is_some_and(|entity| entity.inuse);
        if !inuse {
            driver.warn("entity was removed while using targets\n");
            return Ok(());
        }
    }
    Ok(())
}

/// Send a command to a team (`teamCommand`).
pub fn team_command(driver: &mut dyn Q3Driver, team: Team, command: &str) {
    let max = driver.pool().max_clients();
    for index in 0..max {
        let send = driver.pool().client(index).is_some_and(|client| {
            client.pers.connected == ConnectionState::Connected as i32 && client.sess.session_team == team as i32
        });
        if send {
            driver.send_server_command(index as i32, command);
        }
    }
}

/// Editor direction conversion (`moveDirection`).
#[must_use]
pub fn move_direction(angles: Vec3) -> (Vec3, Vec3) {
    let direction = vertical_move_direction(angles).unwrap_or_else(|| angle_vectors(angles).forward);
    (direction, vec3(0.0, 0.0, 0.0))
}

// ---------------------------------------------------------------------------
// Unified from `mirrors_game_state.rs` (hoist: q3 state mirror).
// ---------------------------------------------------------------------------

/// Debug polygon allocation (`BotDebugPolygons`).
pub trait DebugPolygons {
    /// Create a polygon (`create`).
    fn create(&mut self, color: i32, count: usize, points: &[Vec3]) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::q3::base::game::state::test_support::*;

    use qa_core::math::vec3;

    use crate::q3::base::shared::definitions::Product;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    #[test]
    fn utilities_scratch_config_and_dispatch() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Product::Baseq3);
        let vector = driver.scratch.tv(1.0, 2.0, 3.0);
        assert_eq!(vector, TempVector { x: 1.0, y: 2.0, z: 3.0 });
        let text = driver.scratch.vtos(vec3(1.0, 2.0, 3.0)).unwrap().read_string();
        assert_eq!(text, "(1 2 3)");
        struct Store {
            values: HashMap<usize, String>,
        }
        impl ConfigStringStore for Store {
            fn get(&self, index: usize) -> String {
                self.values.get(&index).cloned().unwrap_or_default()
            }
            fn set(&mut self, index: usize, value: &str) {
                self.values.insert(index, value.to_string());
            }
        }
        let mut registry = ConfigStringRegistry::new(Store { values: HashMap::new() });
        assert_eq!(registry.model_index(Some("models/a")).unwrap(), 1);
        assert_eq!(registry.model_index(Some("models/a")).unwrap(), 1);
        assert_eq!(registry.model_index(None).unwrap(), 0);
        driver.pool.use_slot(10);
        driver.pool.entities[10].set_classname(Some("Target_Thing".to_string()));
        assert_eq!(
            find_entity(&driver.pool, None, EntityStringField::Classname, Some("target_thing")),
            Some(10)
        );
        assert_eq!(
            find_entity(
                &driver.pool,
                Some(10),
                EntityStringField::Classname,
                Some("target_thing")
            ),
            None
        );
        driver.pool.use_slot(11);
        driver.pool.entities[11].set_classname(Some("user".to_string()));
        driver.pool.entities[11].target = Some("Target_Thing".to_string());
        driver.pool.entities[10].targetname = Some("Target_Thing".to_string());
        let fired = Rc::new(RefCell::new(false));
        let fired_clone = Rc::clone(&fired);
        driver
            .pool
            .callbacks_mut()
            .use_callbacks
            .register(
                "test.use",
                Rc::new(move |_, _, _, _| {
                    *fired_clone.borrow_mut() = true;
                }),
            )
            .unwrap();
        let callback = driver.pool.callbacks().use_callbacks.resolve(Some("test.use")).unwrap();
        driver.pool.entities[10].use_callback = callback;
        use_targets(&mut driver, 11, None).unwrap();
        assert!(*fired.borrow());
        let (direction, zero) = move_direction(vec3(0.0, -1.0, 0.0));
        assert_eq!(direction, vec3(0.0, 0.0, 1.0));
        assert_eq!(zero, vec3(0.0, 0.0, 0.0));
        let (direction, zero) = move_direction(vec3(0.0, -2.0, 0.0));
        assert_eq!(direction, vec3(0.0, 0.0, -1.0));
        assert_eq!(zero, vec3(0.0, 0.0, 0.0));
    }
}
