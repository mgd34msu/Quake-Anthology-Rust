//! QVM mod input: observe original source writes and handler decisions.
//!
//! Ports `src/compat/qvm/mod-input.ts`. Input/output declarations come from
//! [`super::mod_actors`]; user-command records reuse [`super::game_input`].
//! This file also owns the mod-client application and input-output mirrors
//! shared with [`super::mod_clients`].

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use super::game_data::{AbiProfile, QvmFunctionCall, QvmModule, QVM_MAX_PRIVATE_ARGUMENT_WORDS};
use super::game_input::{read_qvm_user_command, QvmUserCommandRecord, QVM_USER_COMMAND_BYTES};
use super::mod_actors::{
    resolve_qvm_input_pointer, QvmInputFieldValue, QvmModActorRecord, QvmModClientInput, QvmModInputOutput,
    QvmModInputPointer, QvmModInputPointerBase, QvmModScalar,
};
use super::player_record::QvmPlayerState;
use crate::error::GuestError;

/// Client clock reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QvmModTime {
    /// Seconds.
    Seconds(f64),
    /// Milliseconds.
    Milliseconds(i32),
}

impl QvmModTime {
    /// Clock value in milliseconds.
    #[must_use]
    pub fn as_milliseconds(&self) -> f64 {
        match *self {
            Self::Seconds(value) => value * 1000.0,
            Self::Milliseconds(value) => f64::from(value),
        }
    }
}

/// Client application frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmModClientFrame {
    /// Frame time.
    pub time: QvmModTime,
    /// Frame elapsed time.
    pub elapsed: QvmModTime,
}

/// Client command input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmModClientCommand {
    /// Command kind (`q3` for original commands).
    pub kind: String,
    /// Buttons.
    pub buttons: i32,
    /// Forward move.
    pub forward_move: i32,
    /// Side move.
    pub side_move: i32,
    /// Up move.
    pub up_move: i32,
}

/// Client identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmModClientIdentity {
    /// Actor.
    pub actor: ActorId,
}

/// Mod client application (mirror of `ModClientApplication`).
#[derive(Debug, Clone, PartialEq)]
pub struct ModClientApplication {
    /// Identity.
    pub identity: QvmModClientIdentity,
    /// Command.
    pub command: QvmModClientCommand,
    /// Frame.
    pub frame: QvmModClientFrame,
    /// Absolute aim.
    pub absolute_aim: Vec3,
}

/// Observed input value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QvmModInputValue {
    /// Scalar value.
    Scalar(f64),
    /// Angle value.
    Angles(Vec3),
}

/// Client input output (mirror of `ModClientInputOutput`).
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModClientInputOutput {
    /// Set an input.
    Set {
        /// Input.
        input: QvmModClientInput,
        /// Value.
        value: QvmModInputValue,
    },
    /// Consume inputs.
    Consume {
        /// Inputs.
        inputs: Vec<QvmModClientInput>,
    },
}

/// Provider operations for mod input.
pub trait QvmModInputOperations {
    /// Source pointer for an actor record.
    fn pointer(&self, actor: &ActorId, record: &str) -> Result<usize, GuestError>;
    /// Whether an actor is live.
    fn live(&self, actor: &ActorId) -> bool;
    /// Read a player state.
    fn player_state(&self, actor: &ActorId) -> Result<QvmPlayerState, GuestError>;
}

/// Command capture.
#[derive(Debug, Clone)]
struct CommandCapture {
    /// Output.
    output: QvmModInputOutput,
    /// Command address.
    address: usize,
    /// Command before the handler.
    before: QvmUserCommandRecord,
}

/// Field capture.
#[derive(Debug, Clone)]
struct FieldCapture {
    /// Output.
    output: QvmModInputOutput,
    /// Value before the capture.
    before: QvmModClientInputOutput,
}

/// Output capture.
#[derive(Debug, Clone, Default)]
struct Capture {
    /// Capture identity.
    id: u64,
    /// Application.
    application: Option<ModClientApplication>,
    /// Selected outputs.
    selected: Vec<QvmModInputOutput>,
    /// Published changes.
    changes: Vec<QvmModClientInputOutput>,
    /// Field captures.
    fields: Vec<FieldCapture>,
    /// Command captures.
    commands: Vec<CommandCapture>,
    /// Consumed inputs.
    consumed: Vec<QvmModClientInputOutput>,
}

/// Observe original source writes and handler decisions within their caller frame.
pub struct QvmModInput {
    /// Source module.
    module: QvmModule,
    /// ABI profile.
    abi_profile: AbiProfile,
    /// Actor records.
    records: Vec<QvmModActorRecord>,
    /// Provider operations.
    operations: Rc<dyn QvmModInputOperations>,
    /// Bound hook ids.
    removals: RefCell<Vec<u64>>,
    /// Open applications.
    applications: RefCell<Vec<ModClientApplication>>,
    /// Output captures.
    captures: RefCell<Vec<Capture>>,
    /// Next capture id.
    next_capture: Cell<u64>,
    /// First stashed hook error.
    error: RefCell<Option<GuestError>>,
}

impl QvmModInput {
    /// Bind input outputs over a module.
    pub fn new(
        module: QvmModule,
        abi_profile: AbiProfile,
        records: Vec<QvmModActorRecord>,
        operations: Rc<dyn QvmModInputOperations>,
        outputs: Vec<QvmModInputOutput>,
    ) -> Result<Rc<Self>, GuestError> {
        let this = Rc::new(Self {
            module,
            abi_profile,
            records,
            operations,
            removals: RefCell::new(Vec::new()),
            applications: RefCell::new(Vec::new()),
            captures: RefCell::new(Vec::new()),
            next_capture: Cell::new(0),
            error: RefCell::new(None),
        });
        this.bind(outputs)?;
        Ok(this)
    }

    /// Stash the first hook error.
    fn stash(&self, error: GuestError) {
        if self.error.borrow().is_none() {
            *self.error.borrow_mut() = Some(error);
        }
    }

    /// Take the stashed hook error, if any.
    pub fn take_error(&self) -> Option<GuestError> {
        self.error.borrow_mut().take()
    }

    /// Validate outputs and bind handler entries.
    fn bind(self: &Rc<Self>, outputs: Vec<QvmModInputOutput>) -> Result<(), GuestError> {
        let mut entries: HashMap<usize, Vec<QvmModInputOutput>> = HashMap::new();
        let bound = |output: &QvmModInputOutput| -> Result<(), GuestError> {
            let record = match output {
                QvmModInputOutput::Field { record, .. } => Some(record.as_str()),
                output => output.actor_record(),
            };
            if record.map_or(true, |id| self.records.iter().all(|record| record.id != id)) {
                return Err(GuestError::invalid("QVM input output names an undeclared actor record"));
            }
            Ok(())
        };
        for output in &outputs {
            bound(output)?;
            match output {
                QvmModInputOutput::Field { record, offset, value } => {
                    let record = self
                        .records
                        .iter()
                        .find(|entry| entry.id == *record)
                        .ok_or_else(|| GuestError::invalid("QVM input output names an undeclared actor record"))?;
                    let size = if matches!(value, QvmInputFieldValue::ViewAngles) {
                        12
                    } else {
                        4
                    };
                    if offset + size > record.stride {
                        return Err(GuestError::invalid("QVM input output exceeds its actor record"));
                    }
                    if let QvmInputFieldValue::Scalar { scale, .. } = value {
                        if !scale.is_finite() || *scale <= 0.0 {
                            return Err(GuestError::invalid("QVM input field scale must be positive"));
                        }
                    }
                }
                QvmModInputOutput::Handler {
                    entry,
                    pointer,
                    inputs,
                    returns,
                    ..
                } => {
                    if inputs.contains(&QvmModClientInput::ViewAngles) {
                        return Err(GuestError::invalid("QVM input handler cannot consume view angles"));
                    }
                    self.validate_pointer(pointer)?;
                    if let Some(expected) = returns {
                        let valid = expected.value.is_finite()
                            && match expected.encoding {
                                QvmModScalar::Float32 => f64::from(expected.value as f32).is_finite(),
                                QvmModScalar::Int32 => {
                                    expected.value.fract() == 0.0
                                        && expected.value >= f64::from(i32::MIN)
                                        && expected.value <= f64::from(i32::MAX)
                                }
                            };
                        if !valid {
                            return Err(GuestError::invalid("Invalid QVM input handler return value"));
                        }
                    }
                    entries.entry(*entry).or_default().push(output.clone());
                }
                QvmModInputOutput::Command {
                    entry,
                    pointer,
                    command,
                    inputs,
                    ..
                } => {
                    if inputs.contains(&QvmModClientInput::Impulse) {
                        return Err(GuestError::invalid("QVM input command cannot observe impulses"));
                    }
                    self.validate_pointer(pointer)?;
                    self.validate_pointer(command)?;
                    entries.entry(*entry).or_default().push(output.clone());
                }
            }
        }
        for (entry, declarations) in entries {
            let this = Rc::clone(self);
            let id = self.module.bind_invocation(
                entry,
                Rc::new(move |call: &mut QvmFunctionCall| {
                    this.invoke(call, &declarations).unwrap_or_else(|error| {
                        this.stash(error);
                        0
                    })
                }),
            );
            self.removals.borrow_mut().push(id);
        }
        Ok(())
    }

    /// Validate a pointer path.
    fn validate_pointer(&self, pointer: &QvmModInputPointer) -> Result<(), GuestError> {
        match pointer.base {
            QvmModInputPointerBase::Argument { index } => {
                if index >= QVM_MAX_PRIVATE_ARGUMENT_WORDS {
                    return Err(GuestError::invalid("Invalid QVM input pointer argument"));
                }
            }
            QvmModInputPointerBase::Global { address } => {
                self.module
                    .memory()
                    .read_i32(address)
                    .map_err(|_| GuestError::invalid("Invalid QVM input pointer address"))?;
            }
        }
        Ok(())
    }

    /// Open an application; returns its closer.
    pub fn open(self: &Rc<Self>, application: &ModClientApplication) -> Result<Box<dyn FnOnce()>, GuestError> {
        let suspended = self.captures.borrow().last().map(|capture| capture.id);
        if let Some(id) = suspended {
            if self.is_active_id(id) {
                self.reconcile_last(true)?;
            }
        }
        self.applications.borrow_mut().push(application.clone());
        let this = Rc::clone(self);
        let application = application.clone();
        Ok(Box::new(move || {
            let mut applications = this.applications.borrow_mut();
            if let Some(index) = applications.iter().rposition(|entry| entry == &application) {
                applications.remove(index);
            }
            drop(applications);
            if let Some(id) = suspended {
                if this.is_active_id(id) {
                    if let Err(error) = this.reconcile_last(false) {
                        this.stash(error);
                    }
                }
            }
        }))
    }

    /// Whether a capture id is the live capture.
    fn is_active_id(&self, id: u64) -> bool {
        let captures = self.captures.borrow();
        let applications = self.applications.borrow();
        let Some(capture) = captures.last() else {
            return false;
        };
        if capture.id != id {
            return false;
        }
        let Some(application) = capture.application.as_ref() else {
            return false;
        };
        applications.last() == Some(application) && self.operations.live(&application.identity.actor)
    }

    /// Whether the last capture is live.
    fn last_active(&self) -> bool {
        self.captures
            .borrow()
            .last()
            .is_some_and(|capture| self.is_active_id(capture.id))
    }

    /// Read a field output.
    fn field(&self, output: &QvmModInputOutput, actor: &ActorId) -> Result<QvmModClientInputOutput, GuestError> {
        let (QvmModInputOutput::Field { record, offset, value }) = output else {
            return Err(GuestError::invalid("QVM input field requires a field output"));
        };
        let memory = self.module.memory();
        let address = self.operations.pointer(actor, record)? + offset;
        if matches!(value, QvmInputFieldValue::ViewAngles) {
            let angles = memory.read_vec3(address)?;
            if ![angles.x, angles.y, angles.z].iter().all(|value| value.is_finite()) {
                return Err(GuestError::invalid("QVM input returned nonfinite angles"));
            }
            return Ok(QvmModClientInputOutput::Set {
                input: QvmModClientInput::ViewAngles,
                value: QvmModInputValue::Angles(angles),
            });
        }
        let QvmInputFieldValue::Scalar { input, encoding, scale } = value else {
            return Err(GuestError::invalid("QVM input field requires a scalar value"));
        };
        let raw = match encoding {
            QvmModScalar::Int32 => f64::from(memory.read_i32(address)?),
            QvmModScalar::Float32 => f64::from(memory.read_f32(address)?),
        };
        if !raw.is_finite() {
            return Err(GuestError::invalid("QVM input returned a nonfinite scalar"));
        }
        Ok(QvmModClientInputOutput::Set {
            input: *input,
            value: QvmModInputValue::Scalar(raw / scale),
        })
    }

    /// Handle a bound handler/command invocation.
    fn invoke(&self, call: &mut QvmFunctionCall, declarations: &[QvmModInputOutput]) -> Result<i32, GuestError> {
        let application = self
            .captures
            .borrow()
            .last()
            .filter(|_| self.last_active())
            .and_then(|capture| capture.application.clone());
        let Some(application) = application else {
            return Ok(call.proceed());
        };
        let actor = application.identity.actor.clone();
        let selected: Vec<QvmModInputOutput> = {
            let captures = self.captures.borrow();
            let capture = captures
                .last()
                .ok_or_else(|| GuestError::invalid("QVM input capture disappeared"))?;
            let mut selected = Vec::new();
            for output in declarations {
                if !capture.selected.contains(output) {
                    continue;
                }
                let (record, pointer) = match output {
                    QvmModInputOutput::Handler { record, pointer, .. }
                    | QvmModInputOutput::Command { record, pointer, .. } => (record.as_str(), pointer),
                    QvmModInputOutput::Field { .. } => continue,
                };
                let address = resolve_qvm_input_pointer(pointer, call)?;
                if address == self.operations.pointer(&actor, record)? {
                    selected.push(output.clone());
                }
            }
            selected
        };
        let mut commands = Vec::new();
        for output in &selected {
            if let QvmModInputOutput::Command { command, .. } = output {
                let address = resolve_qvm_input_pointer(command, call)?;
                let bytes = call.guest.read_bytes(address, QVM_USER_COMMAND_BYTES)?;
                commands.push(CommandCapture {
                    output: output.clone(),
                    address,
                    before: read_qvm_user_command(&bytes, self.abi_profile)?,
                });
            }
        }
        {
            let mut captures = self.captures.borrow_mut();
            let capture = captures
                .last_mut()
                .ok_or_else(|| GuestError::invalid("QVM input capture disappeared"))?;
            capture.commands.extend(commands.iter().cloned());
        }
        let result = call.proceed();
        let finished = self.finish(result, &selected, &commands);
        {
            let mut captures = self.captures.borrow_mut();
            if let Some(capture) = captures.last_mut() {
                capture
                    .commands
                    .retain(|command| !commands.iter().any(|fresh| fresh.address == command.address));
            }
        }
        finished?;
        Ok(result)
    }

    /// Publish handler decisions and command changes.
    fn finish(
        &self,
        result: i32,
        selected: &[QvmModInputOutput],
        commands: &[CommandCapture],
    ) -> Result<(), GuestError> {
        if !self.last_active() {
            return Ok(());
        }
        {
            let mut captures = self.captures.borrow_mut();
            let Some(capture) = captures.last_mut() else {
                return Ok(());
            };
            for output in selected {
                let QvmModInputOutput::Handler { inputs, returns, .. } = output else {
                    continue;
                };
                if let Some(expected) = returns {
                    let actual = match expected.encoding {
                        QvmModScalar::Int32 => f64::from(result),
                        QvmModScalar::Float32 => f64::from(f32::from_bits(result as u32)),
                    };
                    let want = match expected.encoding {
                        QvmModScalar::Int32 => expected.value,
                        QvmModScalar::Float32 => f64::from(expected.value as f32),
                    };
                    if actual != want {
                        continue;
                    }
                }
                capture
                    .consumed
                    .push(QvmModClientInputOutput::Consume { inputs: inputs.clone() });
            }
        }
        self.publish_commands(commands, true)
    }

    /// Refresh command captures, publishing on request.
    fn publish_commands(&self, commands: &[CommandCapture], publish: bool) -> Result<(), GuestError> {
        let memory = self.module.memory();
        let application = self
            .captures
            .borrow()
            .last()
            .and_then(|capture| capture.application.clone());
        let Some(application) = application else {
            return Ok(());
        };
        let mut afters = Vec::with_capacity(commands.len());
        for command in commands {
            let bytes = memory.read_bytes(command.address, QVM_USER_COMMAND_BYTES)?;
            afters.push(read_qvm_user_command(&bytes, self.abi_profile)?);
        }
        let mut captures = self.captures.borrow_mut();
        let Some(capture) = captures.last_mut() else {
            return Ok(());
        };
        for (command, after) in commands.iter().zip(afters) {
            if let Some(stored) = capture
                .commands
                .iter_mut()
                .find(|stored| stored.address == command.address)
            {
                stored.before = after;
            }
            if !publish {
                continue;
            }
            let QvmModInputOutput::Command { inputs, .. } = &command.output else {
                continue;
            };
            for input in inputs {
                if *input == QvmModClientInput::ViewAngles {
                    if command.before.angles == after.angles {
                        continue;
                    }
                    let delta = self
                        .operations
                        .player_state(&application.identity.actor)?
                        .delta_angle_words;
                    let word = |index: usize| {
                        (i64::from(after.angles[index]) + i64::from(delta[index]) & 0xffff) as f64 * 360.0 / 65536.0
                    };
                    capture.changes.push(QvmModClientInputOutput::Set {
                        input: QvmModClientInput::ViewAngles,
                        value: QvmModInputValue::Angles(qa_core::math::vec3(
                            word(0) as f32,
                            word(1) as f32,
                            word(2) as f32,
                        )),
                    });
                    continue;
                }
                let value = |command: &QvmUserCommandRecord| match input {
                    QvmModClientInput::Attack => {
                        if command.buttons & 1 == 0 {
                            0.0
                        } else {
                            1.0
                        }
                    }
                    QvmModClientInput::Jump => {
                        if command.up_move >= 10 {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    QvmModClientInput::ForwardMove => f64::from(command.forward_move) / 127.0,
                    QvmModClientInput::SideMove => f64::from(command.right_move) / 127.0,
                    QvmModClientInput::UpMove => f64::from(command.up_move) / 127.0,
                    QvmModClientInput::ViewAngles | QvmModClientInput::Impulse => f64::NAN,
                };
                if value(&command.before) != value(&after) {
                    capture.changes.push(QvmModClientInputOutput::Set {
                        input: *input,
                        value: QvmModInputValue::Scalar(value(&after)),
                    });
                }
            }
        }
        Ok(())
    }

    /// Reconcile the last capture.
    fn reconcile_last(&self, publish: bool) -> Result<(), GuestError> {
        let snapshot = self.captures.borrow().last().cloned();
        let Some(snapshot) = snapshot else {
            return Ok(());
        };
        let Some(application) = snapshot.application.clone() else {
            return Ok(());
        };
        for field in &snapshot.fields {
            let after = self.field(&field.output, &application.identity.actor)?;
            let mut captures = self.captures.borrow_mut();
            let Some(capture) = captures.last_mut() else {
                return Ok(());
            };
            if publish && field.before != after {
                capture.changes.push(after.clone());
            }
            if let Some(stored) = capture.fields.iter_mut().find(|stored| stored.output == field.output) {
                stored.before = after;
            }
        }
        self.publish_commands(&snapshot.commands, publish)
    }

    /// Run outputs under a capture, returning observed changes.
    pub fn output(
        &self,
        outputs: &[QvmModInputOutput],
        application: &ModClientApplication,
        run: &dyn Fn(),
    ) -> Result<Vec<QvmModClientInputOutput>, GuestError> {
        if self.applications.borrow().last() != Some(application) {
            return Err(GuestError::invalid("QVM input output requires its live application"));
        }
        let actor = &application.identity.actor;
        let mut fields = Vec::new();
        for output in outputs {
            if matches!(output, QvmModInputOutput::Field { .. }) {
                fields.push(FieldCapture {
                    output: output.clone(),
                    before: self.field(output, actor)?,
                });
            }
        }
        let id = self.next_capture.get();
        self.next_capture.set(id + 1);
        self.captures.borrow_mut().push(Capture {
            id,
            application: Some(application.clone()),
            selected: outputs.to_vec(),
            changes: Vec::new(),
            fields,
            commands: Vec::new(),
            consumed: Vec::new(),
        });
        run();
        if !self.last_active() {
            self.captures.borrow_mut().pop();
            return Ok(Vec::new());
        }
        self.reconcile_last(true)?;
        let capture = self
            .captures
            .borrow_mut()
            .pop()
            .ok_or_else(|| GuestError::invalid("QVM input capture disappeared"))?;
        Ok(capture.changes.into_iter().chain(capture.consumed).collect())
    }

    /// Remove hooks and forget applications.
    pub fn close(&self) {
        for id in self.removals.borrow_mut().drain(..) {
            self.module.remove_hook(id);
        }
        self.applications.borrow_mut().clear();
        self.captures.borrow_mut().clear();
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::game_data::{
        AbiProfile, ModuleIdentity, QvmArtifact, QvmImage, QvmInstruction, QvmOpcode, QvmRole,
    };
    use super::super::game_input::write_qvm_user_command;
    use super::super::mod_actors::{QvmInputHandlerReturn, QvmModClientInput};
    use super::super::player_record::{qvm_player_state_bytes, read_qvm_player_state};
    use super::*;

    struct FixtureOperations {
        live: Rc<Cell<bool>>,
        state: QvmPlayerState,
    }

    impl QvmModInputOperations for FixtureOperations {
        fn pointer(&self, _actor: &ActorId, record: &str) -> Result<usize, GuestError> {
            if record == "client" {
                Ok(8192)
            } else {
                Err(GuestError::invalid("unknown record"))
            }
        }

        fn live(&self, _actor: &ActorId) -> bool {
            self.live.get()
        }

        fn player_state(&self, _actor: &ActorId) -> Result<QvmPlayerState, GuestError> {
            Ok(self.state.clone())
        }
    }

    struct Fixture {
        input: Rc<QvmModInput>,
        module: QvmModule,
        live: Rc<Cell<bool>>,
        application: ModClientApplication,
    }

    fn field_output() -> QvmModInputOutput {
        QvmModInputOutput::Field {
            record: "client".to_string(),
            offset: 100,
            value: QvmInputFieldValue::Scalar {
                input: QvmModClientInput::ForwardMove,
                encoding: QvmModScalar::Int32,
                scale: 127.0,
            },
        }
    }

    fn handler_output() -> QvmModInputOutput {
        QvmModInputOutput::Handler {
            entry: 9,
            record: "client".to_string(),
            pointer: QvmModInputPointer {
                base: QvmModInputPointerBase::Argument { index: 0 },
                indirections: Vec::new(),
                offset: 0,
            },
            inputs: vec![QvmModClientInput::Attack],
            returns: None,
        }
    }

    fn command_output() -> QvmModInputOutput {
        QvmModInputOutput::Command {
            entry: 9,
            record: "client".to_string(),
            pointer: QvmModInputPointer {
                base: QvmModInputPointerBase::Argument { index: 0 },
                indirections: Vec::new(),
                offset: 0,
            },
            command: QvmModInputPointer {
                base: QvmModInputPointerBase::Argument { index: 1 },
                indirections: Vec::new(),
                offset: 0,
            },
            inputs: vec![QvmModClientInput::ForwardMove],
        }
    }

    fn records() -> Vec<QvmModActorRecord> {
        vec![QvmModActorRecord {
            id: "client".to_string(),
            address: 8192,
            stride: 512,
            capacity: 4,
            fields: Vec::new(),
        }]
    }

    fn application(actor: &ActorId) -> ModClientApplication {
        ModClientApplication {
            identity: QvmModClientIdentity { actor: actor.clone() },
            command: QvmModClientCommand {
                kind: "q3".to_string(),
                buttons: 0,
                forward_move: 0,
                side_move: 0,
                up_move: 0,
            },
            frame: QvmModClientFrame {
                time: QvmModTime::Milliseconds(1000),
                elapsed: QvmModTime::Milliseconds(8),
            },
            absolute_aim: vec3(0.0, 0.0, 0.0),
        }
    }

    fn fixture(outputs: Vec<QvmModInputOutput>) -> (Fixture, ActorId) {
        let owner = IdentityOwner::create("mod-input-test").unwrap();
        let actor = owner.actor(0, 1);
        let mut image = QvmImage::default();
        image.instructions = (0..32)
            .map(|index| QvmInstruction::word(QvmOpcode::OpEnter, 0, index * 8))
            .collect();
        image.data_length = 4096;
        image.allocated_data_length = 65536;
        let artifact = QvmArtifact {
            module: ModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: "test".to_string(),
                digest: "test".to_string(),
                revision: "1".to_string(),
            },
            role: QvmRole::Qagame,
            abi_profile: Some(AbiProfile::Modern),
            image,
        };
        let module = QvmModule::new(artifact, None, None).unwrap();
        module.memory().write_i32(8192 + 100, 63).unwrap();
        let live = Rc::new(Cell::new(true));
        let state = read_qvm_player_state(
            &vec![0u8; qvm_player_state_bytes(AbiProfile::Modern)],
            AbiProfile::Modern,
        )
        .unwrap();
        let input = QvmModInput::new(
            module.clone(),
            AbiProfile::Modern,
            records(),
            Rc::new(FixtureOperations {
                live: Rc::clone(&live),
                state,
            }),
            outputs,
        )
        .unwrap();
        (
            Fixture {
                input,
                module,
                live,
                application: application(&actor),
            },
            actor,
        )
    }

    #[test]
    fn constructor_validates_outputs() {
        assert!(fixture(Vec::new()).0.input.take_error().is_none());

        let field = QvmModInputOutput::Field {
            record: "missing".to_string(),
            offset: 0,
            value: QvmInputFieldValue::ViewAngles,
        };
        let module = fixture(Vec::new()).0.module.clone();
        let operations: Rc<dyn QvmModInputOperations> = Rc::new(FixtureOperations {
            live: Rc::new(Cell::new(true)),
            state: read_qvm_player_state(
                &vec![0u8; qvm_player_state_bytes(AbiProfile::Modern)],
                AbiProfile::Modern,
            )
            .unwrap(),
        });
        let build = |outputs: Vec<QvmModInputOutput>| {
            QvmModInput::new(
                module.clone(),
                AbiProfile::Modern,
                records(),
                Rc::clone(&operations),
                outputs,
            )
        };
        assert!(build(vec![field]).is_err());
        assert!(build(vec![QvmModInputOutput::Field {
            record: "client".to_string(),
            offset: 509,
            value: QvmInputFieldValue::ViewAngles,
        }])
        .is_err());
        assert!(build(vec![QvmModInputOutput::Field {
            record: "client".to_string(),
            offset: 100,
            value: QvmInputFieldValue::Scalar {
                input: QvmModClientInput::ForwardMove,
                encoding: QvmModScalar::Int32,
                scale: 0.0,
            },
        }])
        .is_err());
        assert!(build(vec![QvmModInputOutput::Handler {
            entry: 9,
            record: "client".to_string(),
            pointer: QvmModInputPointer {
                base: QvmModInputPointerBase::Argument { index: 0 },
                indirections: Vec::new(),
                offset: 0,
            },
            inputs: vec![QvmModClientInput::ViewAngles],
            returns: None,
        }])
        .is_err());
        assert!(build(vec![QvmModInputOutput::Handler {
            entry: 9,
            record: "client".to_string(),
            pointer: QvmModInputPointer {
                base: QvmModInputPointerBase::Argument { index: 0 },
                indirections: Vec::new(),
                offset: 0,
            },
            inputs: Vec::new(),
            returns: Some(QvmInputHandlerReturn {
                encoding: QvmModScalar::Int32,
                value: f64::NAN,
            }),
        }])
        .is_err());
        assert!(build(vec![QvmModInputOutput::Command {
            entry: 9,
            record: "client".to_string(),
            pointer: QvmModInputPointer {
                base: QvmModInputPointerBase::Argument { index: 0 },
                indirections: Vec::new(),
                offset: 0,
            },
            command: QvmModInputPointer {
                base: QvmModInputPointerBase::Argument { index: 1 },
                indirections: Vec::new(),
                offset: 0,
            },
            inputs: vec![QvmModClientInput::Impulse],
        }])
        .is_err());
        assert!(build(vec![QvmModInputOutput::Handler {
            entry: 9,
            record: "client".to_string(),
            pointer: QvmModInputPointer {
                base: QvmModInputPointerBase::Argument { index: 62 },
                indirections: Vec::new(),
                offset: 0,
            },
            inputs: Vec::new(),
            returns: None,
        }])
        .is_err());
    }

    #[test]
    fn output_captures_field_changes() {
        let (fixture, actor) = fixture(Vec::new());
        let _close = fixture.input.open(&fixture.application).unwrap();
        let changes = fixture
            .input
            .output(&[field_output()], &fixture.application, &|| {
                fixture.module.memory().write_i32(8192 + 100, 127).unwrap();
            })
            .unwrap();
        assert_eq!(
            changes,
            vec![QvmModClientInputOutput::Set {
                input: QvmModClientInput::ForwardMove,
                value: QvmModInputValue::Scalar(1.0),
            }]
        );
        let changes = fixture
            .input
            .output(&[field_output()], &fixture.application, &|| {})
            .unwrap();
        assert!(changes.is_empty());

        let other = application(&actor);
        let mut foreign = other.clone();
        foreign.frame.time = QvmModTime::Milliseconds(2000);
        assert!(fixture.input.output(&[field_output()], &foreign, &|| {}).is_err());
    }

    #[test]
    fn handler_consumes_on_matching_return() {
        let handler = QvmModInputOutput::Handler {
            entry: 9,
            record: "client".to_string(),
            pointer: QvmModInputPointer {
                base: QvmModInputPointerBase::Argument { index: 0 },
                indirections: Vec::new(),
                offset: 0,
            },
            inputs: vec![QvmModClientInput::Attack],
            returns: None,
        };
        let (fixture, _) = fixture(vec![handler.clone()]);
        let _close = fixture.input.open(&fixture.application).unwrap();
        let changes = fixture
            .input
            .output(&[handler.clone()], &fixture.application, &|| {
                fixture.module.call(&[8192], 9).unwrap();
            })
            .unwrap();
        assert_eq!(
            changes,
            vec![QvmModClientInputOutput::Consume {
                inputs: vec![QvmModClientInput::Attack],
            }]
        );

        let gated = QvmModInputOutput::Handler {
            entry: 9,
            record: "client".to_string(),
            pointer: QvmModInputPointer {
                base: QvmModInputPointerBase::Argument { index: 0 },
                indirections: Vec::new(),
                offset: 0,
            },
            inputs: vec![QvmModClientInput::Attack],
            returns: Some(QvmInputHandlerReturn {
                encoding: QvmModScalar::Int32,
                value: 5.0,
            }),
        };
        let (fixture, _) = fixture(vec![gated.clone()]);
        let _close = fixture.input.open(&fixture.application).unwrap();
        let changes = fixture
            .input
            .output(&[gated], &fixture.application, &|| {
                fixture.module.call(&[8192], 9).unwrap();
            })
            .unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn command_observes_user_command_edits() {
        let (fixture, _) = fixture(vec![command_output()]);
        let _close = fixture.input.open(&fixture.application).unwrap();
        let changes = fixture
            .input
            .output(&[command_output()], &fixture.application, &|| {
                fixture.module.call(&[8192, 16384], 9).unwrap();
            })
            .unwrap();
        assert!(changes.is_empty());

        let before = QvmUserCommandRecord {
            server_time: 100,
            angles: [0, 0, 0],
            buttons: 0,
            weapon: 5,
            forward_move: 0,
            right_move: 0,
            up_move: 0,
        };
        let after = QvmUserCommandRecord {
            forward_move: 127,
            ..before
        };
        let mut bytes = vec![0u8; QVM_USER_COMMAND_BYTES];
        write_qvm_user_command(&mut bytes, &after, AbiProfile::Modern, false).unwrap();
        fixture.module.memory().write_bytes(16384, &bytes).unwrap();
        let changes = fixture
            .input
            .output(&[command_output()], &fixture.application, &|| {
                fixture
                    .input
                    .finish(
                        0,
                        &[command_output()],
                        &[CommandCapture {
                            output: command_output(),
                            address: 16384,
                            before,
                        }],
                    )
                    .unwrap();
            })
            .unwrap();
        assert_eq!(
            changes,
            vec![QvmModClientInputOutput::Set {
                input: QvmModClientInput::ForwardMove,
                value: QvmModInputValue::Scalar(1.0),
            }]
        );
    }

    #[test]
    fn invoke_errors_stash() {
        let handler = QvmModInputOutput::Handler {
            entry: 9,
            record: "client".to_string(),
            pointer: QvmModInputPointer {
                base: QvmModInputPointerBase::Argument { index: 5 },
                indirections: Vec::new(),
                offset: 0,
            },
            inputs: vec![QvmModClientInput::Attack],
            returns: None,
        };
        let (fixture, _) = fixture(vec![handler.clone()]);
        let _close = fixture.input.open(&fixture.application).unwrap();
        fixture
            .input
            .output(&[handler], &fixture.application, &|| {
                assert_eq!(fixture.module.call(&[8192], 9).unwrap(), 0);
            })
            .unwrap();
        assert!(fixture.input.take_error().is_some());
    }

    #[test]
    fn applications_open_and_close() {
        let (fixture, _) = fixture(Vec::new());
        let close = fixture.input.open(&fixture.application).unwrap();
        assert_eq!(fixture.input.applications.borrow().len(), 1);
        fixture.live.set(false);
        let changes = fixture
            .input
            .output(&[field_output()], &fixture.application, &|| {})
            .unwrap();
        assert!(changes.is_empty());
        fixture.live.set(true);
        close();
        assert!(fixture.input.applications.borrow().is_empty());
        assert!(fixture
            .input
            .output(&[field_output()], &fixture.application, &|| {})
            .is_err());
        fixture.input.close();
    }
}
