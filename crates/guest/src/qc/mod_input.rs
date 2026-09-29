//! QuakeC client-input application over entity words.
//!
//! Ported from `src/compat/qc/mod-input.ts`.
//!
//! Local mirrors (owned by other workers' modules, noted here):
//! `QcInputMachine`/`QcInputWords` mirror the entity-word surface of
//! `QcMachine` from `src/compat/qc/machine.ts`; `QcCallSite` mirrors the call
//! site from `src/compat/qc/machine.ts`; `QcInputHost` mirrors the
//! reference/liveness callbacks. `ModClientApplication` and
//! `client_input_values` come from `super::mod_clients`.
//!
//! Adaptation: the donor returns a closer closure from `open`; this port
//! returns a scope id closed with `close_scope`, since the closer cannot
//! borrow the input owner.

use std::collections::{HashMap, HashSet};

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use super::mod_clients::{client_input_values, ModClientApplication};
use super::mod_provider::{ClientInputUpdate, ModCallbackInput, ModClientInput, ModClientInputOutput, ModQcInputOutput};
use crate::error::GuestError;

/// Entity-word access for input application.
pub trait QcInputWords {
    /// Read a float.
    fn float(&self, offset: i32) -> Result<f32, GuestError>;
    /// Write a float.
    fn set_float(&mut self, offset: i32, value: f32) -> Result<(), GuestError>;
    /// Read a vector.
    fn vector(&self, offset: i32) -> Result<Vec3, GuestError>;
    /// Write a vector.
    fn set_vector(&mut self, offset: i32, value: Vec3) -> Result<(), GuestError>;
}

/// Machine surface for input application.
pub trait QcInputMachine {
    /// Word storage type.
    type Words: QcInputWords;
    /// Words for an entity reference.
    fn words_for(&mut self, reference: i32) -> Result<&mut Self::Words, GuestError>;
    /// Function index by name.
    fn function_index(&self, name: &str) -> Result<i32, GuestError>;
    /// Current `self` global reference.
    fn self_reference(&self) -> i32;
}

/// Reference and liveness callbacks.
pub trait QcInputHost {
    /// Entity reference for an actor.
    fn reference(&self, actor: &ActorId) -> i32;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
}

/// Function entry observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QcCallSite {
    /// Entered function index.
    pub function_index: i32,
}

/// Declared input field.
#[derive(Debug, Clone, PartialEq)]
pub struct QcModInputField {
    /// Word offset.
    pub offset: i32,
    /// Word width (1 or 3).
    pub words: u32,
    /// Source field name.
    pub field: String,
    /// Input channel.
    pub input: ModClientInput,
    /// Write policy.
    pub update: ClientInputUpdate,
    /// Scalar encoding scale.
    pub scale: Option<f64>,
}

type Snapshot = Vec<(i32, u32, Vec<f32>)>;

struct Frame {
    id: u64,
    actor: ActorId,
    nested: bool,
    saved: Snapshot,
}

struct Capture {
    actor: ActorId,
    reference: i32,
    handlers: HashMap<i32, Vec<ModClientInput>>,
    entered: HashSet<i32>,
}

/// Client-input scopes over entity words.
pub struct QcModInput<M, H> {
    machine: M,
    host: H,
    fields: Vec<QcModInputField>,
    frames: Vec<Frame>,
    captures: Vec<Capture>,
    next_scope: u64,
}

impl<M: QcInputMachine, H: QcInputHost> QcModInput<M, H> {
    /// Build over declared input fields.
    pub fn new(machine: M, host: H, fields: Vec<QcModInputField>) -> Self {
        Self { machine, host, fields, frames: Vec::new(), captures: Vec::new(), next_scope: 1 }
    }

    /// Borrow the machine.
    #[must_use]
    pub fn machine(&self) -> &M {
        &self.machine
    }

    /// Borrow the machine mutably.
    pub fn machine_mut(&mut self) -> &mut M {
        &mut self.machine
    }

    /// Whether any input scope is active.
    #[must_use]
    pub fn active(&self) -> bool {
        !self.frames.is_empty()
    }

    /// Snapshot one field's words.
    fn snapshot_field(words: &M::Words, field: &QcModInputField) -> Result<Vec<f32>, GuestError> {
        if field.words == 3 {
            let vector = words.vector(field.offset)?;
            Ok(vec![vector.x, vector.y, vector.z])
        } else {
            Ok(vec![words.float(field.offset)?])
        }
    }

    /// Restore one field's snapshot.
    fn restore_field(words: &mut M::Words, offset: i32, width: u32, saved: &[f32]) -> Result<(), GuestError> {
        if width == 3 {
            words.set_vector(offset, Vec3 { x: saved[0], y: saved[1], z: saved[2] })
        } else {
            words.set_float(offset, saved[0])
        }
    }

    /// Open an input scope, applying validated values to entity words.
    pub fn open(&mut self, application: &ModClientApplication) -> Result<u64, GuestError> {
        let actor = application.actor.clone();
        let values = client_input_values(application);
        let reference = self.host.reference(&actor);
        let nested = self.frames.iter().any(|frame| frame.actor == actor);
        let mut saved: Snapshot = Vec::with_capacity(self.fields.len());
        {
            let words = self.machine.words_for(reference)?;
            for field in &self.fields {
                saved.push((field.offset, field.words, Self::snapshot_field(words, field)?));
            }
            let applied = Self::apply_fields(&self.fields, words, &values);
            if let Err(error) = applied {
                for (offset, width, bytes) in &saved {
                    Self::restore_field(words, *offset, *width, bytes)?;
                }
                return Err(error);
            }
        }
        let id = self.next_scope;
        self.next_scope += 1;
        self.frames.push(Frame { id, actor, nested, saved });
        Ok(id)
    }

    /// Apply validated values to words.
    fn apply_fields(
        fields: &[QcModInputField],
        words: &mut M::Words,
        values: &HashMap<ModCallbackInput, super::mod_provider::ModRuntimeValue>,
    ) -> Result<(), GuestError> {
        use super::mod_provider::ModRuntimeValue;
        for field in fields {
            let value = values.get(&field.input.as_callback_input());
            match value {
                Some(ModRuntimeValue::Vector(value)) => words.set_vector(field.offset, *value)?,
                Some(ModRuntimeValue::Float(value)) => {
                    if field.update == ClientInputUpdate::Always || *value != 0.0 {
                        let scalar = value * field.scale.unwrap_or(1.0);
                        if !f64::from(scalar as f32).is_finite() {
                            return Err(GuestError::invalid("Source input exceeds the QC scalar ABI"));
                        }
                        words.set_float(field.offset, scalar as f32)?;
                    }
                }
                _ => return Err(GuestError::invalid("Missing validated QuakeC client input")),
            }
        }
        Ok(())
    }

    /// Close an input scope.
    pub fn close_scope(&mut self, scope: u64) -> Result<(), GuestError> {
        let index = match self.frames.iter().position(|frame| frame.id == scope) {
            Some(index) => index,
            None => return Ok(()),
        };
        let frame = self.frames.remove(index);
        if frame.nested && self.frames.iter().any(|parent| parent.actor == frame.actor) {
            let reference = self.host.reference(&frame.actor);
            let words = self.machine.words_for(reference)?;
            for (offset, width, saved) in &frame.saved {
                Self::restore_field(words, *offset, *width, saved)?;
            }
        }
        Ok(())
    }

    /// Validated values for the active scope application.
    pub fn values(
        &self,
        application: &ModClientApplication,
    ) -> Result<HashMap<ModCallbackInput, super::mod_provider::ModRuntimeValue>, GuestError> {
        let frame = self.frames.last().ok_or_else(|| GuestError::invalid("QuakeC client input scope is not active"))?;
        if frame.actor != application.actor {
            return Err(GuestError::invalid("QuakeC client input scope is not active"));
        }
        Ok(client_input_values(application))
    }

    /// Observe a function entry for handler capture.
    pub fn observe_call(&mut self, call: &QcCallSite) {
        let capture = match self.captures.last_mut() {
            Some(capture) => capture,
            None => return,
        };
        let frame_matches = self.frames.last().is_some_and(|frame| frame.actor == capture.actor);
        if frame_matches && self.machine.self_reference() == capture.reference && capture.handlers.contains_key(&call.function_index)
        {
            capture.entered.insert(call.function_index);
        }
    }

    /// Run source with output capture. The runner receives the input owner so
    /// tests and hosts can observe calls while the capture is active.
    pub fn output(
        &mut self,
        outputs: &[ModQcInputOutput],
        application: &ModClientApplication,
        run: &mut dyn FnMut(&mut Self),
    ) -> Result<Vec<ModClientInputOutput>, GuestError> {
        let reference = self.host.reference(&application.actor);
        let mut handlers: HashMap<i32, Vec<ModClientInput>> = HashMap::new();
        for output in outputs {
            if let ModQcInputOutput::Handler { function, inputs } = output {
                handlers.insert(self.machine.function_index(function)?, inputs.clone());
            }
        }
        let mut watched: Vec<(QcModInputField, Before)> = Vec::new();
        {
            let words = self.machine.words_for(reference)?;
            for output in outputs {
                if let ModQcInputOutput::Field { field } = output {
                    let declared = self
                        .fields
                        .iter()
                        .find(|entry| entry.field == *field)
                        .cloned()
                        .ok_or_else(|| GuestError::invalid("QC output lacks its declared input field"))?;
                    let before = if declared.words == 3 {
                        Before::Vector(words.vector(declared.offset)?)
                    } else {
                        Before::Float(words.float(declared.offset)?)
                    };
                    watched.push((declared, before));
                }
            }
        }
        self.captures.push(Capture { actor: application.actor.clone(), reference, handlers, entered: HashSet::new() });
        run(self);
        let capture = self.captures.pop().ok_or_else(|| GuestError::invalid("QuakeC input capture is unbalanced"))?;
        if !self.host.is_live(&application.actor) || !self.frames.iter().any(|frame| frame.actor == application.actor) {
            return Ok(Vec::new());
        }
        let words = self.machine.words_for(reference)?;
        let mut result = Vec::new();
        for (field, before) in &watched {
            if field.input == ModClientInput::ViewAngles {
                let value = words.vector(field.offset)?;
                if before.as_vector() != Some(value) {
                    result.push(ModClientInputOutput::SetAngles { value });
                }
            } else {
                let after = words.float(field.offset)?;
                if before.as_float() != Some(after) {
                    result.push(ModClientInputOutput::SetScalar {
                        input: field.input,
                        value: f64::from(after) / field.scale.unwrap_or(1.0),
                    });
                }
            }
        }
        for index in &capture.entered {
            if let Some(inputs) = capture.handlers.get(index) {
                result.push(ModClientInputOutput::Consume { inputs: inputs.clone() });
            }
        }
        Ok(result)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Before {
    Float(f32),
    Vector(Vec3),
}

impl Before {
    fn as_float(&self) -> Option<f32> {
        match *self {
            Self::Float(value) => Some(value),
            Self::Vector(_) => None,
        }
    }

    fn as_vector(&self) -> Option<Vec3> {
        match *self {
            Self::Vector(value) => Some(value),
            Self::Float(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::mod_clients::{ModClientApplication, ModInputValue};

    #[derive(Default)]
    struct FakeWords {
        floats: HashMap<i32, f32>,
        vectors: HashMap<i32, Vec3>,
    }

    impl QcInputWords for FakeWords {
        fn float(&self, offset: i32) -> Result<f32, GuestError> {
            Ok(self.floats.get(&offset).copied().unwrap_or(0.0))
        }

        fn set_float(&mut self, offset: i32, value: f32) -> Result<(), GuestError> {
            self.floats.insert(offset, value);
            Ok(())
        }

        fn vector(&self, offset: i32) -> Result<Vec3, GuestError> {
            Ok(self.vectors.get(&offset).copied().unwrap_or_else(|| vec3(0.0, 0.0, 0.0)))
        }

        fn set_vector(&mut self, offset: i32, value: Vec3) -> Result<(), GuestError> {
            self.vectors.insert(offset, value);
            Ok(())
        }
    }

    struct FakeMachine {
        words: HashMap<i32, FakeWords>,
        functions: HashMap<String, i32>,
        self_reference: i32,
    }

    impl QcInputMachine for FakeMachine {
        type Words = FakeWords;

        fn words_for(&mut self, reference: i32) -> Result<&mut Self::Words, GuestError> {
            Ok(self.words.entry(reference).or_default())
        }

        fn function_index(&self, name: &str) -> Result<i32, GuestError> {
            self.functions.get(name).copied().ok_or_else(|| GuestError::invalid(format!("Unknown function {name}")))
        }

        fn self_reference(&self) -> i32 {
            self.self_reference
        }
    }

    struct FakeHost {
        live: bool,
    }

    impl QcInputHost for FakeHost {
        fn reference(&self, actor: &ActorId) -> i32 {
            actor.slot() as i32
        }

        fn is_live(&self, _actor: &ActorId) -> bool {
            self.live
        }
    }

    fn application(owner: &IdentityOwner) -> ModClientApplication {
        ModClientApplication {
            actor: owner.actor(3, 1),
            client: owner.client(3, 1),
            values: [
                (ModClientInput::Attack, ModInputValue::Float(1.0)),
                (ModClientInput::Jump, ModInputValue::Float(0.0)),
                (ModClientInput::ViewAngles, ModInputValue::Vector(vec3(10.0, 20.0, 30.0))),
            ]
            .into_iter()
            .collect(),
        }
    }

    fn fields() -> Vec<QcModInputField> {
        vec![
            QcModInputField {
                offset: 1,
                words: 1,
                field: "attack".to_string(),
                input: ModClientInput::Attack,
                update: ClientInputUpdate::Always,
                scale: Some(2.0),
            },
            QcModInputField {
                offset: 2,
                words: 1,
                field: "jump".to_string(),
                input: ModClientInput::Jump,
                update: ClientInputUpdate::Nonzero,
                scale: None,
            },
            QcModInputField {
                offset: 3,
                words: 3,
                field: "v_angle".to_string(),
                input: ModClientInput::ViewAngles,
                update: ClientInputUpdate::Always,
                scale: None,
            },
        ]
    }

    fn fixture() -> (IdentityOwner, QcModInput<FakeMachine, FakeHost>) {
        let owner = IdentityOwner::create("input").unwrap();
        let machine = FakeMachine { words: HashMap::new(), functions: [("use".to_string(), 7)].into_iter().collect(), self_reference: 3 };
        (owner, QcModInput::new(machine, FakeHost { live: true }, fields()))
    }

    #[test]
    fn open_applies_scaled_and_nonzero_values() {
        let (owner, mut input) = fixture();
        input.machine_mut().words_for(3).unwrap().set_float(2, 9.0).unwrap();
        let scope = input.open(&application(&owner)).unwrap();
        assert!(input.active());
        let words = input.machine_mut().words_for(3).unwrap();
        assert_eq!(words.float(1).unwrap(), 2.0);
        assert_eq!(words.float(2).unwrap(), 9.0);
        assert_eq!(words.vector(3).unwrap(), vec3(10.0, 20.0, 30.0));
        input.close_scope(scope).unwrap();
        assert!(!input.active());
        // Outermost close keeps applied values.
        assert_eq!(input.machine_mut().words_for(3).unwrap().float(1).unwrap(), 2.0);
    }

    #[test]
    fn nested_close_restores_parent_words() {
        let (owner, mut input) = fixture();
        let application = application(&owner);
        let outer = input.open(&application).unwrap();
        input.machine_mut().words_for(3).unwrap().set_float(1, 50.0).unwrap();
        let inner = input.open(&application).unwrap();
        assert_eq!(input.machine_mut().words_for(3).unwrap().float(1).unwrap(), 2.0);
        input.close_scope(inner).unwrap();
        assert_eq!(input.machine_mut().words_for(3).unwrap().float(1).unwrap(), 50.0);
        input.close_scope(outer).unwrap();
        input.close_scope(999).unwrap();
    }

    #[test]
    fn values_require_active_scope() {
        let (owner, mut input) = fixture();
        let application = application(&owner);
        assert!(input.values(&application).is_err());
        input.open(&application).unwrap();
        let values = input.values(&application).unwrap();
        assert_eq!(values.get(&ModCallbackInput::Attack), Some(&super::super::mod_provider::ModRuntimeValue::Float(1.0)));
    }

    #[test]
    fn missing_input_aborts_and_restores() {
        let (owner, mut input) = fixture();
        let mut application = application(&owner);
        application.values.remove(&ModClientInput::Attack);
        input.machine_mut().words_for(3).unwrap().set_float(1, 11.0).unwrap();
        assert!(input.open(&application).is_err());
        assert_eq!(input.machine_mut().words_for(3).unwrap().float(1).unwrap(), 11.0);
        assert!(!input.active());
    }

    #[test]
    fn output_diffs_fields_and_handlers() {
        let (owner, mut input) = fixture();
        let application = application(&owner);
        input.open(&application).unwrap();
        let outputs = vec![
            ModQcInputOutput::Field { field: "attack".to_string() },
            ModQcInputOutput::Field { field: "v_angle".to_string() },
            ModQcInputOutput::Handler { function: "use".to_string(), inputs: vec![ModClientInput::Attack] },
        ];
        let result = input
            .output(&outputs, &application, &mut |input| {
                input.machine_mut().words_for(3).unwrap().set_float(1, 4.0).unwrap();
                input.observe_call(&QcCallSite { function_index: 7 });
                input.observe_call(&QcCallSite { function_index: 8 });
            })
            .unwrap();
        assert!(result.contains(&ModClientInputOutput::SetScalar { input: ModClientInput::Attack, value: 2.0 }));
        assert!(result.contains(&ModClientInputOutput::Consume { inputs: vec![ModClientInput::Attack] }));
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn output_rejects_undeclared_fields() {
        let (owner, mut input) = fixture();
        let application = application(&owner);
        input.open(&application).unwrap();
        let outputs = vec![ModQcInputOutput::Field { field: "nope".to_string() }];
        assert!(input.output(&outputs, &application, &mut |_| {}).is_err());
    }
}
