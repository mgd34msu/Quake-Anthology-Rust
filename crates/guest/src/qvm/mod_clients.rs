//! QVM mod client bindings: source slots over live destination clients.
//!
//! Ports `src/compat/qvm/mod-clients.ts`. Client declarations come from
//! [`super::mod_actors`]; applications and input outputs come from
//! [`super::mod_input`]; player states reuse [`super::player_record`]; final
//! user commands reuse [`super::game_input`]. The destination client services
//! mirror `src/world/session/mod-clients.ts`, the input subscription mirrors
//! the handlers contract of `src/world/session/mod-client-input.ts`, and the
//! command synthesis mirrors `q3CommandForControls`/`relativeQ3SourceCommand`
//! from `src/app/bootstrap/simulation/q3-commands.ts`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_core::identity::{ActorId, ClientId};

use super::game_input::Q3UserCommand;
use super::mod_actors::{QvmModClientInputBinding, QvmModClients, QvmModInputOutput, QvmModSourceCall};
use super::mod_input::{ModClientApplication, QvmModClientCommand, QvmModClientInputOutput, QvmModTime};
use super::player_record::QvmPlayerState;
use crate::error::GuestError;

/// Admitted source client slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmModClientSlot {
    /// Actor.
    pub actor: ActorId,
    /// Source slot.
    pub slot: usize,
    /// Whether admission calls ran.
    pub admitted: bool,
}

/// Client slot with its destination binding.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ClientSlot {
    /// Actor.
    actor: ActorId,
    /// Source slot.
    slot: usize,
    /// Whether admission calls ran.
    admitted: bool,
    /// Destination client.
    client: Option<ClientId>,
}

/// Destination client event kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmModClientEventKind {
    /// Client admitted.
    Admitted,
    /// Userinfo changed.
    Userinfo,
    /// Client disconnecting.
    Disconnecting,
}

/// Destination client event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmModClientEvent {
    /// Kind.
    pub kind: QvmModClientEventKind,
    /// Actor.
    pub actor: ActorId,
}

/// Destination client identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmModClientIdentity {
    /// Destination client.
    pub client: ClientId,
    /// Actor.
    pub actor: ActorId,
}

/// Accepted command angle space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmAngleSpace {
    /// Absolute aim.
    Absolute,
    /// Source-relative words.
    SourceRelative,
}

/// Accepted command source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmInputSource {
    /// Whether the source is a local seat.
    pub local_seat: bool,
}

/// Accepted arsenal intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmArsenalIntent {
    /// Whether the holdable is used.
    pub use_holdable: bool,
}

/// Accepted command input.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmAcceptedInput {
    /// Source.
    pub source: QvmInputSource,
    /// Command.
    pub command: QvmModClientCommand,
    /// Angle space.
    pub angle_space: Option<QvmAngleSpace>,
    /// Arsenal intent.
    pub arsenal: Option<QvmArsenalIntent>,
}

/// Accepted destination client command.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmAcceptedClientCommand {
    /// Input.
    pub input: QvmAcceptedInput,
    /// Time.
    pub time: QvmModTime,
}

/// Opens an input application; returns its closer.
pub type QvmClientInputOpen = Rc<dyn Fn(&ModClientApplication) -> Result<Box<dyn FnOnce()>, GuestError>>;
/// Invokes a bound input call.
pub type QvmClientInputInvoke =
    Rc<dyn Fn(&QvmModSourceCall, &ModClientApplication) -> Result<(), GuestError>>;
/// Runs bound input outputs around a call sequence.
pub type QvmClientInputOutputRunner = Rc<
    dyn Fn(
        &[QvmModInputOutput],
        &ModClientApplication,
        &dyn Fn(),
    ) -> Result<Vec<QvmModClientInputOutput>, GuestError>,
>;
/// Client-event listener.
pub type QvmClientEventListener = Rc<dyn Fn(&QvmModClientEvent) -> Result<(), GuestError>>;

/// Input subscription handlers.
#[derive(Clone)]
pub struct QvmModClientInputHandlers {
    /// Open an application; returns its closer.
    pub open: QvmClientInputOpen,
    /// Invoke a bound call.
    pub invoke: QvmClientInputInvoke,
    /// Run bound outputs around a call sequence.
    pub output: Option<QvmClientInputOutputRunner>,
}

/// Destination client services.
pub trait QvmModClientServices {
    /// Subscribe to client events; returns the unsubscribe handle.
    fn subscribe(&self, on_event: QvmClientEventListener) -> Box<dyn FnOnce()>;
    /// Current destination clients.
    fn clients(&self) -> Vec<QvmModClientIdentity>;
    /// Destination client for an actor.
    fn for_actor(&self, actor: &ActorId) -> Option<ClientId>;
    /// Actor for a destination client.
    fn actor(&self, client: &ClientId) -> Option<ActorId>;
    /// Client userinfo.
    fn userinfo(&self, client: &ClientId) -> String;
    /// Set client userinfo.
    fn set_userinfo(&self, client: &ClientId, value: &str);
    /// Drop a client.
    fn drop_client(&self, client: &ClientId, reason: &str, content: &str);
    /// Accepted command for a client.
    fn command(&self, client: &ClientId) -> Option<QvmAcceptedClientCommand>;
    /// Subscribe to client-input applications.
    fn subscribe_input(
        &self,
        bindings: &[QvmModClientInputBinding],
        handlers: QvmModClientInputHandlers,
    ) -> Box<dyn FnOnce()>;
}

/// Provider operations for client bindings.
pub trait QvmModClientOperations {
    /// Project a client.
    fn project(&self, actor: &ActorId) -> Result<(), GuestError>;
    /// Observe admission.
    fn admitted(&self, _actor: &ActorId) -> Result<(), GuestError> {
        Ok(())
    }
    /// Release a client.
    fn release(&self, actor: &ActorId) -> Result<(), GuestError>;
    /// Invoke a source call.
    fn invoke(
        &self,
        call: &QvmModSourceCall,
        actor: &ActorId,
        application: Option<&ModClientApplication>,
    ) -> Result<(), GuestError>;
    /// Open weapon/input state for an application.
    fn open_input(&self, _application: &ModClientApplication) -> Result<Option<Box<dyn FnOnce()>>, GuestError> {
        Ok(None)
    }
    /// Run input outputs around a call sequence.
    fn output(
        &self,
        _outputs: &[QvmModInputOutput],
        _application: &ModClientApplication,
        run: &dyn Fn(),
    ) -> Result<Option<Vec<QvmModClientInputOutput>>, GuestError> {
        run();
        Ok(None)
    }
    /// Reserved source slots.
    fn reserved_slots(&self) -> Vec<usize> {
        Vec::new()
    }
    /// Read a player state.
    fn player_state(&self, actor: &ActorId) -> Result<QvmPlayerState, GuestError>;
    /// Requested weapon override.
    fn requested_weapon(&self, _actor: &ActorId) -> Option<i32> {
        None
    }
    /// Send a server command.
    fn send(&self, text: &str, recipient: Option<&ActorId>) -> Result<(), GuestError>;
}

/// Lower client controls to a Q3 user command.
pub fn qvm_command_for_controls(
    command: &QvmModClientCommand,
    milliseconds: f64,
    requested_weapon: i32,
    use_holdable: bool,
) -> Q3UserCommand {
    let buttons = (if command.kind == "q3" {
        command.buttons & !4
    } else {
        command.buttons & 1
    }) | if use_holdable { 4 } else { 0 };
    if command.kind == "q3" {
        return Q3UserCommand {
            server_time_ms: command.server_time_ms,
            angle_words: command.angle_words,
            buttons,
            weapon: requested_weapon,
            forward_move: command.forward_move as i32,
            right_move: command.side_move as i32,
            up_move: command.up_move as i32,
        };
    }
    let angles = if command.kind == "q2-classic" {
        command.angle_shorts
    } else {
        let word = |angle: f32| ((f64::from(angle) * 65536.0 / 360.0).trunc() as i64 & 0xffff) as i32;
        [word(command.angles.x), word(command.angles.y), word(command.angles.z)]
    };
    let divisor = if command.kind == "q1-netquake" || command.kind == "q1-quakeworld" {
        320.0
    } else {
        200.0
    };
    let axis = |value: f64| (value * 127.0 / divisor).clamp(-127.0, 127.0).trunc() as i32;
    let up_move = if command.kind == "q2-rerelease" {
        if command.buttons & 8 != 0 {
            127
        } else if command.buttons & 16 != 0 {
            -127
        } else {
            0
        }
    } else if (command.kind == "q1-netquake" || command.kind == "q1-quakeworld") && command.buttons & 2 != 0 {
        127
    } else {
        axis(command.up_move)
    };
    Q3UserCommand {
        server_time_ms: milliseconds.trunc() as i32,
        angle_words: angles,
        buttons,
        weapon: requested_weapon,
        forward_move: axis(command.forward_move),
        right_move: axis(command.side_move),
        up_move,
    }
}

/// Relativize absolute aim against delta angles.
pub fn relative_qvm_source_command(
    local_seat: bool,
    dialect: &str,
    command: &Q3UserCommand,
    delta: [i32; 3],
    angle_space: Option<QvmAngleSpace>,
) -> Q3UserCommand {
    if angle_space == Some(QvmAngleSpace::Absolute) || angle_space.is_none() && local_seat && dialect != "q3" {
        Q3UserCommand {
            angle_words: [
                command.angle_words[0] - delta[0],
                command.angle_words[1] - delta[1],
                command.angle_words[2] - delta[2],
            ],
            ..*command
        }
    } else {
        *command
    }
}

/// Source slots belong to this component; every use resolves the live destination identity.
pub struct QvmModClientBindings {
    /// Owning content.
    content: String,
    /// Destination services.
    services: Rc<dyn QvmModClientServices>,
    /// Client declaration.
    declaration: QvmModClients,
    /// Provider operations.
    operations: Rc<dyn QvmModClientOperations>,
    /// Bound entries.
    entries: RefCell<HashMap<ActorId, ClientSlot>>,
    /// Event unsubscribe handle.
    unsubscribe: RefCell<Option<Box<dyn FnOnce()>>>,
    /// Input unsubscribe handle.
    unsubscribe_input: RefCell<Option<Box<dyn FnOnce()>>>,
    /// Open applications.
    applications: RefCell<Vec<ModClientApplication>>,
}

impl QvmModClientBindings {
    /// Bind source client slots.
    pub fn new(
        content: String,
        services: Rc<dyn QvmModClientServices>,
        declaration: QvmModClients,
        operations: Rc<dyn QvmModClientOperations>,
    ) -> Rc<Self> {
        Rc::new(Self {
            content,
            services,
            declaration,
            operations,
            entries: RefCell::new(HashMap::new()),
            unsubscribe: RefCell::new(None),
            unsubscribe_input: RefCell::new(None),
            applications: RefCell::new(Vec::new()),
        })
    }

    /// Whether an actor has an entry.
    pub fn has(&self, actor: &ActorId) -> bool {
        self.entries.borrow().contains_key(actor)
    }

    /// Whether an actor's entry is live.
    pub fn live(&self, actor: &ActorId) -> bool {
        let entries = self.entries.borrow();
        let Some(entry) = entries.get(actor) else {
            return false;
        };
        let Some(client) = self.services.for_actor(actor) else {
            return false;
        };
        if entry.client.as_ref().is_some_and(|bound| bound != &client) {
            return false;
        }
        self.services.actor(&client).as_ref() == Some(actor)
    }

    /// Whether an actor is admitted.
    pub fn admitted(&self, actor: &ActorId) -> bool {
        self.entries.borrow().get(actor).is_some_and(|entry| entry.admitted) && self.require(actor).is_ok()
    }

    /// Resolve or assign a source slot.
    pub fn slot(&self, actor: &ActorId) -> Result<Option<usize>, GuestError> {
        let client = self.services.for_actor(actor);
        let Some(client) = client else {
            return Ok(self.entries.borrow().get(actor).map(|entry| entry.slot));
        };
        if let Some(previous) = self.entries.borrow().get(actor) {
            self.require(actor)?;
            return Ok(Some(previous.slot));
        }
        let mut used: HashSet<usize> = self.entries.borrow().values().map(|entry| entry.slot).collect();
        for slot in self.operations.reserved_slots() {
            used.insert(slot);
        }
        let mut slot = 0;
        while used.contains(&slot) {
            slot += 1;
        }
        if slot >= self.declaration.maximum {
            return Err(GuestError::invalid("QVM component source client capacity exceeded"));
        }
        self.entries.borrow_mut().insert(
            actor.clone(),
            ClientSlot {
                actor: actor.clone(),
                client: Some(client),
                slot,
                admitted: false,
            },
        );
        Ok(Some(slot))
    }

    /// Require a live entry.
    fn require(&self, actor: &ActorId) -> Result<ClientSlot, GuestError> {
        let entry = self.entries.borrow().get(actor).cloned();
        let client = self.services.for_actor(actor);
        let (Some(entry), Some(client)) = (entry, client) else {
            return Err(GuestError::invalid("QVM component client identity is no longer live"));
        };
        if entry.client.as_ref().is_some_and(|bound| bound != &client)
            || self.services.actor(&client).as_ref() != Some(actor)
        {
            return Err(GuestError::invalid("QVM component client identity is no longer live"));
        }
        if entry.client.is_none() {
            let current = ClientSlot {
                client: Some(client),
                ..entry
            };
            self.entries.borrow_mut().insert(actor.clone(), current.clone());
            return Ok(current);
        }
        Ok(entry)
    }

    /// Entry for a source slot.
    fn at(&self, slot: usize) -> Result<Option<ClientSlot>, GuestError> {
        let actor = self
            .entries
            .borrow()
            .values()
            .find(|entry| entry.slot == slot)
            .map(|entry| entry.actor.clone());
        actor.map(|actor| self.require(&actor)).transpose()
    }

    /// Invoke admission calls.
    fn calls(&self, calls: &[QvmModSourceCall], actor: &ActorId) -> Result<(), GuestError> {
        for call in calls {
            self.operations.invoke(call, actor, None)?;
        }
        Ok(())
    }

    /// Admit an actor.
    fn admit(&self, actor: &ActorId) -> Result<(), GuestError> {
        if self.slot(actor)?.is_none() {
            return Err(GuestError::invalid("QVM component admission requires a live client"));
        }
        let entry = self.require(actor)?;
        self.operations.project(actor)?;
        if !entry.admitted {
            let mut entries = self.entries.borrow_mut();
            if let Some(stored) = entries.get_mut(actor) {
                stored.admitted = true;
            }
            drop(entries);
            self.calls(&self.declaration.admit.clone(), actor)?;
        }
        self.operations.admitted(actor)
    }

    /// Start event and input subscriptions.
    pub fn start(self: &Rc<Self>) -> Result<(), GuestError> {
        if self.unsubscribe.borrow().is_some() {
            return Ok(());
        }
        let this = Rc::clone(self);
        let unsubscribe = self
            .services
            .subscribe(Rc::new(move |event: &QvmModClientEvent| this.on_event(event)));
        *self.unsubscribe.borrow_mut() = Some(unsubscribe);
        for identity in self.services.clients() {
            self.admit(&identity.actor)?;
        }
        let open_this = Rc::clone(self);
        let invoke_this = Rc::clone(self);
        let output_this = Rc::clone(self);
        let handlers = QvmModClientInputHandlers {
            open: Rc::new(move |application: &ModClientApplication| open_this.open_application(application)),
            invoke: Rc::new(move |call: &QvmModSourceCall, application: &ModClientApplication| {
                invoke_this.require(&application.identity.actor)?;
                invoke_this
                    .operations
                    .invoke(call, &application.identity.actor, Some(application))
            }),
            output: Some(Rc::new(
                move |outputs: &[QvmModInputOutput], application: &ModClientApplication, run: &dyn Fn()| {
                    output_this
                        .operations
                        .output(outputs, application, run)?
                        .ok_or_else(|| GuestError::invalid("Source input output adapter is unavailable"))
                },
            )),
        };
        let unsubscribe = self.services.subscribe_input(&self.declaration.input, handlers);
        *self.unsubscribe_input.borrow_mut() = Some(unsubscribe);
        Ok(())
    }

    /// Handle a destination client event.
    fn on_event(&self, event: &QvmModClientEvent) -> Result<(), GuestError> {
        match event.kind {
            QvmModClientEventKind::Admitted => self.admit(&event.actor),
            QvmModClientEventKind::Userinfo => {
                self.admit(&event.actor)?;
                let calls = self.declaration.userinfo.clone();
                self.calls(&calls, &event.actor)
            }
            QvmModClientEventKind::Disconnecting => {
                if !self.entries.borrow().contains_key(&event.actor) {
                    return Ok(());
                }
                self.require(&event.actor)?;
                let calls = self.declaration.disconnect.clone();
                self.calls(&calls, &event.actor)?;
                let released = self.operations.release(&event.actor);
                self.entries.borrow_mut().remove(&event.actor);
                released
            }
        }
    }

    /// Open an input application.
    fn open_application(self: &Rc<Self>, application: &ModClientApplication) -> Result<Box<dyn FnOnce()>, GuestError> {
        let entry = self.require(&application.identity.actor)?;
        if !entry.admitted {
            return Err(GuestError::invalid(
                "QVM input callback requires admitted source client state",
            ));
        }
        self.applications.borrow_mut().push(application.clone());
        let close = match self.operations.open_input(application) {
            Ok(close) => close,
            Err(error) => {
                self.remove_application(application);
                return Err(error);
            }
        };
        let this = Rc::clone(self);
        let application = application.clone();
        Ok(Box::new(move || {
            if let Some(close) = close {
                close();
            }
            this.remove_application(&application);
        }))
    }

    /// Remove an open application.
    fn remove_application(&self, application: &ModClientApplication) {
        let mut applications = self.applications.borrow_mut();
        if let Some(index) = applications.iter().rposition(|entry| entry == application) {
            applications.remove(index);
        }
    }

    /// Restore saved slots.
    pub fn restore(&self, entries: &[QvmModClientSlot]) {
        self.entries.borrow_mut().clear();
        for entry in entries {
            self.entries.borrow_mut().insert(
                entry.actor.clone(),
                ClientSlot {
                    actor: entry.actor.clone(),
                    slot: entry.slot,
                    admitted: entry.admitted,
                    client: self.services.for_actor(&entry.actor),
                },
            );
        }
    }

    /// Run frame calls over admitted live clients in slot order.
    pub fn frame(
        &self,
        invoke: &dyn Fn(&QvmModSourceCall, &ActorId) -> Result<(), GuestError>,
    ) -> Result<(), GuestError> {
        if self.declaration.frame.is_empty() {
            return Ok(());
        }
        let mut clients: Vec<ClientSlot> = self
            .entries
            .borrow()
            .values()
            .filter(|entry| entry.admitted && self.live(&entry.actor))
            .cloned()
            .collect();
        for entry in &mut clients {
            *entry = self.require(&entry.actor)?;
        }
        clients.sort_by_key(|entry| entry.slot);
        for entry in &clients {
            for call in &self.declaration.frame.clone() {
                if self.entries.borrow().get(&entry.actor) != Some(entry) || !self.live(&entry.actor) {
                    break;
                }
                invoke(call, &entry.actor)?;
            }
        }
        Ok(())
    }

    /// Current players.
    pub fn players(&self) -> Result<Vec<QvmModClientSlot>, GuestError> {
        self.entries
            .borrow()
            .values()
            .map(|entry| {
                let current = self.require(&entry.actor)?;
                Ok(QvmModClientSlot {
                    actor: entry.actor.clone(),
                    slot: current.slot,
                    admitted: current.admitted,
                })
            })
            .collect()
    }

    /// Checkpoint current slots.
    pub fn checkpoint(&self) -> Result<Vec<QvmModClientSlot>, GuestError> {
        if !self.applications.borrow().is_empty() {
            return Err(GuestError::invalid(
                "Cannot save during QVM component input application",
            ));
        }
        Ok(self
            .entries
            .borrow()
            .values()
            .map(|entry| QvmModClientSlot {
                actor: entry.actor.clone(),
                slot: entry.slot,
                admitted: entry.admitted,
            })
            .collect())
    }

    /// Forget an actor.
    pub fn forget(&self, actor: &ActorId) {
        self.entries.borrow_mut().remove(actor);
    }

    /// Close subscriptions and forget state.
    pub fn close(&self) {
        if let Some(unsubscribe) = self.unsubscribe_input.borrow_mut().take() {
            unsubscribe();
        }
        if let Some(unsubscribe) = self.unsubscribe.borrow_mut().take() {
            unsubscribe();
        }
        self.applications.borrow_mut().clear();
        self.entries.borrow_mut().clear();
    }

    /// Read userinfo for a source slot.
    pub fn get_userinfo(&self, slot: usize) -> Result<String, GuestError> {
        let entry = self.at(slot)?;
        match entry.and_then(|entry| entry.client) {
            Some(client) => Ok(self.services.userinfo(&client)),
            None => Ok(String::new()),
        }
    }

    /// Write userinfo for a source slot.
    pub fn set_userinfo(&self, slot: usize, value: &str) -> Result<(), GuestError> {
        let entry = self.at(slot)?;
        match entry.and_then(|entry| entry.client) {
            Some(client) => {
                self.services.set_userinfo(&client, value);
                Ok(())
            }
            None => Err(GuestError::invalid(
                "QVM component cannot update an unbound source client",
            )),
        }
    }

    /// Drop a source slot.
    pub fn drop_client(&self, slot: usize, reason: &str) -> Result<(), GuestError> {
        let entry = self.at(slot)?;
        if let Some(client) = entry.and_then(|entry| entry.client) {
            self.services.drop_client(&client, reason, &self.content);
        }
        Ok(())
    }

    /// Send a server command.
    pub fn send_server_command(&self, slot: i32, text: &str) -> Result<(), GuestError> {
        if slot == -1 {
            return self.operations.send(text, None);
        }
        let slot = usize::try_from(slot).map_err(|_| GuestError::invalid("QVM component slot is not a source slot"))?;
        if let Some(entry) = self.at(slot)? {
            self.operations.send(text, Some(&entry.actor))?;
        }
        Ok(())
    }

    /// Topmost open application for an actor.
    fn applied(&self, actor: &ActorId) -> Option<ModClientApplication> {
        self.applications
            .borrow()
            .iter()
            .rev()
            .find(|application| application.identity.actor == *actor)
            .cloned()
    }

    /// Read the user command for a source slot.
    pub fn get_user_command(&self, slot: usize) -> Result<Q3UserCommand, GuestError> {
        let entry = self.at(slot)?;
        let application = entry.as_ref().and_then(|entry| self.applied(&entry.actor));
        if let (Some(entry), Some(application)) = (entry.as_ref(), application.as_ref()) {
            let ps = self.operations.player_state(&entry.actor)?;
            let command = qvm_command_for_controls(
                &application.command,
                application.frame.time.as_milliseconds(),
                self.operations.requested_weapon(&entry.actor).unwrap_or(ps.weapon),
                application.command.kind == "q3" && application.command.buttons & 4 != 0,
            );
            let word = |angle: f32| ((f64::from(angle) * 65536.0 / 360.0).trunc() as i64 & 0xffff) as i32;
            return Ok(Q3UserCommand {
                angle_words: [
                    word(application.absolute_aim.x) - ps.delta_angle_words[0],
                    word(application.absolute_aim.y) - ps.delta_angle_words[1],
                    word(application.absolute_aim.z) - ps.delta_angle_words[2],
                ],
                ..command
            });
        }
        let client = entry.as_ref().and_then(|entry| entry.client.clone());
        let accepted = client.as_ref().and_then(|client| self.services.command(client));
        let (Some(entry), Some(accepted)) = (entry, accepted) else {
            return Err(GuestError::invalid(
                "QVM component usercmd requires an accepted destination client command",
            ));
        };
        let input = &accepted.input;
        let ps = self.operations.player_state(&entry.actor)?;
        let use_holdable = input
            .arsenal
            .map(|arsenal| arsenal.use_holdable)
            .unwrap_or(input.command.kind == "q3" && input.command.buttons & 4 != 0);
        let command = qvm_command_for_controls(
            &input.command,
            accepted.time.as_milliseconds(),
            self.operations.requested_weapon(&entry.actor).unwrap_or(ps.weapon),
            use_holdable,
        );
        Ok(relative_qvm_source_command(
            input.source.local_seat,
            &input.command.kind,
            &command,
            ps.delta_angle_words,
            input.angle_space,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;

    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::game_data::AbiProfile;
    use super::super::mod_actors::{QvmInputPhase, QvmModReturn};
    use super::super::mod_input::{QvmModClientCommand, QvmModClientFrame, QvmModTime};
    use super::super::player_record::{qvm_player_state_bytes, read_qvm_player_state};
    use super::*;

    struct FixtureServices {
        actors: RefCell<HashMap<ActorId, ClientId>>,
        clients: RefCell<HashMap<ClientId, ActorId>>,
        userinfos: RefCell<HashMap<ClientId, String>>,
        commands: RefCell<HashMap<ClientId, QvmAcceptedClientCommand>>,
        listener: RefCell<Option<QvmClientEventListener>>,
        handlers: RefCell<Option<QvmModClientInputHandlers>>,
        unsubscribed: Rc<Cell<bool>>,
        unsubscribed_input: Rc<Cell<bool>>,
        dropped: RefCell<Vec<(ClientId, String)>>,
    }

    impl QvmModClientServices for FixtureServices {
        fn subscribe(&self, on_event: QvmClientEventListener) -> Box<dyn FnOnce()> {
            *self.listener.borrow_mut() = Some(on_event);
            let flag = Rc::clone(&self.unsubscribed);
            Box::new(move || flag.set(true))
        }

        fn clients(&self) -> Vec<QvmModClientIdentity> {
            self.actors
                .borrow()
                .iter()
                .map(|(actor, client)| QvmModClientIdentity {
                    client: client.clone(),
                    actor: actor.clone(),
                })
                .collect()
        }

        fn for_actor(&self, actor: &ActorId) -> Option<ClientId> {
            self.actors.borrow().get(actor).cloned()
        }

        fn actor(&self, client: &ClientId) -> Option<ActorId> {
            self.clients.borrow().get(client).cloned()
        }

        fn userinfo(&self, client: &ClientId) -> String {
            self.userinfos.borrow().get(client).cloned().unwrap_or_default()
        }

        fn set_userinfo(&self, client: &ClientId, value: &str) {
            self.userinfos.borrow_mut().insert(client.clone(), value.to_string());
        }

        fn drop_client(&self, client: &ClientId, reason: &str, _content: &str) {
            self.dropped.borrow_mut().push((client.clone(), reason.to_string()));
        }

        fn command(&self, client: &ClientId) -> Option<QvmAcceptedClientCommand> {
            self.commands.borrow().get(client).cloned()
        }

        fn subscribe_input(
            &self,
            _bindings: &[QvmModClientInputBinding],
            handlers: QvmModClientInputHandlers,
        ) -> Box<dyn FnOnce()> {
            *self.handlers.borrow_mut() = Some(handlers);
            let flag = Rc::clone(&self.unsubscribed_input);
            Box::new(move || flag.set(true))
        }
    }

    struct FixtureOperations {
        projected: RefCell<Vec<ActorId>>,
        released: RefCell<Vec<ActorId>>,
        invoked: RefCell<Vec<(usize, ActorId, bool)>>,
        states: RefCell<HashMap<ActorId, QvmPlayerState>>,
        requested: RefCell<HashMap<ActorId, i32>>,
        sent: RefCell<Vec<(String, Option<ActorId>)>>,
        reserved: RefCell<Vec<usize>>,
    }

    impl QvmModClientOperations for FixtureOperations {
        fn project(&self, actor: &ActorId) -> Result<(), GuestError> {
            self.projected.borrow_mut().push(actor.clone());
            Ok(())
        }

        fn release(&self, actor: &ActorId) -> Result<(), GuestError> {
            self.released.borrow_mut().push(actor.clone());
            Ok(())
        }

        fn invoke(
            &self,
            call: &QvmModSourceCall,
            actor: &ActorId,
            application: Option<&ModClientApplication>,
        ) -> Result<(), GuestError> {
            self.invoked
                .borrow_mut()
                .push((call.entry, actor.clone(), application.is_some()));
            Ok(())
        }

        fn player_state(&self, actor: &ActorId) -> Result<QvmPlayerState, GuestError> {
            self.states
                .borrow()
                .get(actor)
                .cloned()
                .ok_or_else(|| GuestError::invalid("missing test player state"))
        }

        fn requested_weapon(&self, actor: &ActorId) -> Option<i32> {
            self.requested.borrow().get(actor).copied()
        }

        fn send(&self, text: &str, recipient: Option<&ActorId>) -> Result<(), GuestError> {
            self.sent.borrow_mut().push((text.to_string(), recipient.cloned()));
            Ok(())
        }

        fn reserved_slots(&self) -> Vec<usize> {
            self.reserved.borrow().clone()
        }
    }

    struct Fixture {
        bindings: Rc<QvmModClientBindings>,
        services: Rc<FixtureServices>,
        operations: Rc<FixtureOperations>,
        first: ActorId,
        second: ActorId,
    }

    fn call(entry: usize) -> QvmModSourceCall {
        QvmModSourceCall {
            entry,
            arguments: Vec::new(),
            globals: Vec::new(),
            returns: QvmModReturn::Void,
        }
    }

    fn declaration(maximum: usize) -> QvmModClients {
        QvmModClients {
            outputs: Vec::new(),
            maximum,
            records: vec!["client".to_string()],
            player_state_record: "client".to_string(),
            admit: vec![call(1)],
            userinfo: vec![call(2)],
            disconnect: vec![call(3)],
            frame: vec![call(4), call(5)],
            input: vec![QvmModClientInputBinding {
                calls: Vec::new(),
                scope: "movement-slice".to_string(),
                phase: QvmInputPhase::After,
                outputs: Vec::new(),
            }],
        }
    }

    fn player_state() -> QvmPlayerState {
        let mut state = read_qvm_player_state(
            &vec![0u8; qvm_player_state_bytes(AbiProfile::Modern)],
            AbiProfile::Modern,
        )
        .unwrap();
        state.weapon = 7;
        state.delta_angle_words = [10, 20, 30];
        state
    }

    fn q3_command() -> QvmModClientCommand {
        QvmModClientCommand {
            kind: "q3".to_string(),
            buttons: 4,
            server_time_ms: 500,
            angle_words: [1, 2, 3],
            angle_shorts: [0, 0, 0],
            angles: vec3(0.0, 0.0, 0.0),
            forward_move: 10.0,
            side_move: 20.0,
            up_move: 30.0,
        }
    }

    fn application(actor: &ActorId) -> ModClientApplication {
        ModClientApplication {
            identity: super::super::mod_input::QvmModClientIdentity { actor: actor.clone() },
            scope: "movement-slice".to_string(),
            command: q3_command(),
            frame: QvmModClientFrame {
                time: QvmModTime::Milliseconds(1000),
                elapsed: QvmModTime::Milliseconds(8),
            },
            absolute_aim: vec3(90.0, 0.0, 0.0),
        }
    }

    fn make_fixture(maximum: usize) -> Fixture {
        let owner = IdentityOwner::create("mod-clients-test").unwrap();
        let first = owner.actor(0, 1);
        let second = owner.actor(1, 1);
        let client0 = owner.client(0, 1);
        let services = Rc::new(FixtureServices {
            actors: RefCell::new(HashMap::from([(first.clone(), client0.clone())])),
            clients: RefCell::new(HashMap::from([(client0.clone(), first.clone())])),
            userinfos: RefCell::new(HashMap::from([(client0, "name=a".to_string())])),
            commands: RefCell::new(HashMap::new()),
            listener: RefCell::new(None),
            handlers: RefCell::new(None),
            unsubscribed: Rc::new(Cell::new(false)),
            unsubscribed_input: Rc::new(Cell::new(false)),
            dropped: RefCell::new(Vec::new()),
        });
        let operations = Rc::new(FixtureOperations {
            projected: RefCell::new(Vec::new()),
            released: RefCell::new(Vec::new()),
            invoked: RefCell::new(Vec::new()),
            states: RefCell::new(HashMap::from([
                (first.clone(), player_state()),
                (second.clone(), player_state()),
            ])),
            requested: RefCell::new(HashMap::new()),
            sent: RefCell::new(Vec::new()),
            reserved: RefCell::new(Vec::new()),
        });
        let bindings = QvmModClientBindings::new(
            "test-content".to_string(),
            Rc::clone(&services) as Rc<dyn QvmModClientServices>,
            declaration(maximum),
            Rc::clone(&operations) as Rc<dyn QvmModClientOperations>,
        );
        Fixture {
            bindings,
            services,
            operations,
            first,
            second,
        }
    }

    fn emit(fixture: &Fixture, kind: QvmModClientEventKind, actor: &ActorId) -> Result<(), GuestError> {
        let listener = fixture.services.listener.borrow().clone().unwrap();
        listener(&QvmModClientEvent {
            kind,
            actor: actor.clone(),
        })
    }

    #[test]
    fn start_admits_and_routes_events() {
        let fixture = make_fixture(2);
        fixture.bindings.start().unwrap();
        assert!(fixture.bindings.admitted(&fixture.first));
        assert_eq!(fixture.bindings.slot(&fixture.first).unwrap(), Some(0));
        assert_eq!(
            fixture.operations.projected.borrow().as_slice(),
            std::slice::from_ref(&fixture.first)
        );
        assert_eq!(
            fixture.operations.invoked.borrow().as_slice(),
            &[(1, fixture.first.clone(), false)]
        );

        assert_eq!(fixture.bindings.slot(&fixture.second).unwrap(), None);
        emit(&fixture, QvmModClientEventKind::Userinfo, &fixture.first).unwrap();
        assert_eq!(fixture.operations.invoked.borrow().len(), 2);
        assert_eq!(fixture.operations.invoked.borrow()[1].0, 2);

        emit(&fixture, QvmModClientEventKind::Disconnecting, &fixture.first).unwrap();
        assert_eq!(fixture.operations.invoked.borrow()[2].0, 3);
        assert_eq!(fixture.operations.released.borrow().len(), 1);
        assert!(!fixture.bindings.has(&fixture.first));

        let owner = IdentityOwner::create("mod-clients-unknown").unwrap();
        let unknown = owner.actor(9, 1);
        emit(&fixture, QvmModClientEventKind::Disconnecting, &unknown).unwrap();
        assert_eq!(fixture.operations.invoked.borrow().len(), 3);
    }

    #[test]
    fn slot_capacity_and_reserved() {
        let fixture = make_fixture(1);
        fixture.bindings.start().unwrap();
        let owner = IdentityOwner::create("mod-clients-extra").unwrap();
        let extra = owner.actor(2, 1);
        let client = owner.client(1, 1);
        fixture
            .services
            .actors
            .borrow_mut()
            .insert(extra.clone(), client.clone());
        fixture.services.clients.borrow_mut().insert(client, extra.clone());
        assert!(emit(&fixture, QvmModClientEventKind::Admitted, &extra).is_err());
        assert!(fixture.bindings.slot(&extra).is_err());

        let fixture = make_fixture(2);
        fixture.operations.reserved.borrow_mut().push(0);
        fixture.bindings.start().unwrap();
        assert_eq!(fixture.bindings.slot(&fixture.first).unwrap(), Some(1));
    }

    #[test]
    fn live_tracks_identity() {
        let fixture = make_fixture(2);
        fixture.bindings.start().unwrap();
        assert!(fixture.bindings.live(&fixture.first));
        let owner = IdentityOwner::create("mod-clients-rotate").unwrap();
        let rotated = owner.client(3, 1);
        fixture
            .services
            .actors
            .borrow_mut()
            .insert(fixture.first.clone(), rotated.clone());
        assert!(!fixture.bindings.live(&fixture.first));
        assert!(fixture.bindings.get_userinfo(0).is_err());
    }

    #[test]
    fn frame_runs_in_slot_order_and_breaks() {
        let fixture = make_fixture(2);
        fixture.bindings.start().unwrap();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_hook = Rc::clone(&seen);
        fixture
            .bindings
            .frame(&|call, actor| {
                seen_hook.borrow_mut().push((call.entry, actor.clone()));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            seen.borrow().as_slice(),
            &[(4, fixture.first.clone()), (5, fixture.first.clone())]
        );

        let bindings = Rc::clone(&fixture.bindings);
        fixture
            .bindings
            .frame(&|_, actor| {
                bindings.forget(actor);
                Ok(())
            })
            .unwrap();
        assert!(!fixture.bindings.has(&fixture.first));
    }

    #[test]
    fn players_checkpoint_restore_forget() {
        let fixture = make_fixture(2);
        fixture.bindings.start().unwrap();
        assert_eq!(
            fixture.bindings.players().unwrap(),
            vec![QvmModClientSlot {
                actor: fixture.first.clone(),
                slot: 0,
                admitted: true,
            }]
        );
        let saved = fixture.bindings.checkpoint().unwrap();
        fixture.bindings.forget(&fixture.first);
        assert!(fixture.bindings.players().unwrap().is_empty());
        fixture.bindings.restore(&saved);
        assert!(fixture.bindings.admitted(&fixture.first));
    }

    #[test]
    fn userinfo_and_commands() {
        let fixture = make_fixture(2);
        fixture.bindings.start().unwrap();
        assert_eq!(fixture.bindings.get_userinfo(0).unwrap(), "name=a");
        assert_eq!(fixture.bindings.get_userinfo(99).unwrap(), "");
        fixture.bindings.set_userinfo(0, "name=b").unwrap();
        assert_eq!(fixture.bindings.get_userinfo(0).unwrap(), "name=b");
        assert!(fixture.bindings.set_userinfo(99, "x").is_err());
        fixture.bindings.drop_client(0, "bye").unwrap();
        assert_eq!(fixture.services.dropped.borrow().len(), 1);
        fixture.bindings.drop_client(99, "bye").unwrap();
        assert_eq!(fixture.services.dropped.borrow().len(), 1);

        fixture.bindings.send_server_command(-1, "all").unwrap();
        fixture.bindings.send_server_command(0, "one").unwrap();
        fixture.bindings.send_server_command(99, "none").unwrap();
        assert_eq!(
            fixture.operations.sent.borrow().as_slice(),
            &[
                ("all".to_string(), None),
                ("one".to_string(), Some(fixture.first.clone())),
            ]
        );
    }

    #[test]
    fn user_command_application_path() {
        let fixture = make_fixture(2);
        fixture.bindings.start().unwrap();
        let handlers = fixture.services.handlers.borrow().clone().unwrap();
        let application = application(&fixture.first);
        let close = (handlers.open)(&application).unwrap();
        let command = fixture.bindings.get_user_command(0).unwrap();
        assert_eq!(command.server_time_ms, 500);
        assert_eq!(command.angle_words, [16374, -20, -30]);
        assert_eq!(command.buttons, 4);
        assert_eq!(command.weapon, 7);
        assert_eq!(
            (command.forward_move, command.right_move, command.up_move),
            (10, 20, 30)
        );

        fixture
            .operations
            .requested
            .borrow_mut()
            .insert(fixture.first.clone(), 3);
        let command = fixture.bindings.get_user_command(0).unwrap();
        assert_eq!(command.weapon, 3);
        close();
        assert!(fixture.bindings.checkpoint().is_ok());
    }

    #[test]
    fn user_command_accepted_path() {
        let fixture = make_fixture(2);
        fixture.bindings.start().unwrap();
        let client = fixture.services.for_actor(&fixture.first).unwrap();
        fixture.services.commands.borrow_mut().insert(
            client.clone(),
            QvmAcceptedClientCommand {
                input: QvmAcceptedInput {
                    source: QvmInputSource { local_seat: false },
                    command: q3_command(),
                    angle_space: Some(QvmAngleSpace::SourceRelative),
                    arsenal: None,
                },
                time: QvmModTime::Milliseconds(700),
            },
        );
        let command = fixture.bindings.get_user_command(0).unwrap();
        assert_eq!(command.angle_words, [1, 2, 3]);
        assert_eq!(command.server_time_ms, 500);

        fixture.services.commands.borrow_mut().insert(
            client,
            QvmAcceptedClientCommand {
                input: QvmAcceptedInput {
                    source: QvmInputSource { local_seat: true },
                    command: q3_command(),
                    angle_space: Some(QvmAngleSpace::Absolute),
                    arsenal: Some(QvmArsenalIntent { use_holdable: false }),
                },
                time: QvmModTime::Milliseconds(700),
            },
        );
        let command = fixture.bindings.get_user_command(0).unwrap();
        assert_eq!(command.angle_words, [-9, -18, -27]);
        assert_eq!(command.buttons, 0);
    }

    #[test]
    fn user_command_requires_accepted() {
        let fixture = make_fixture(2);
        fixture.bindings.start().unwrap();
        assert!(fixture.bindings.get_user_command(0).is_err());
        assert!(fixture.bindings.get_user_command(99).is_err());
    }

    #[test]
    fn command_synthesis_kinds() {
        let classic = QvmModClientCommand {
            kind: "q2-classic".to_string(),
            buttons: 3,
            server_time_ms: 0,
            angle_words: [0, 0, 0],
            angle_shorts: [7, 8, 9],
            angles: vec3(0.0, 0.0, 0.0),
            forward_move: 100.0,
            side_move: -100.0,
            up_move: 0.0,
        };
        let command = qvm_command_for_controls(&classic, 123.9, 2, false);
        assert_eq!(command.angle_words, [7, 8, 9]);
        assert_eq!(command.buttons, 1);
        assert_eq!(command.forward_move, 63);
        assert_eq!(command.server_time_ms, 123);

        let quake = QvmModClientCommand {
            kind: "q1-netquake".to_string(),
            buttons: 2,
            server_time_ms: 0,
            angle_words: [0, 0, 0],
            angle_shorts: [0, 0, 0],
            angles: vec3(90.0, 0.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
        };
        let command = qvm_command_for_controls(&quake, 10.0, 2, true);
        assert_eq!(command.angle_words[0], 16384);
        assert_eq!(command.buttons, 4);
        assert_eq!(command.up_move, 127);
    }

    #[test]
    fn close_unsubscribes() {
        let fixture = make_fixture(2);
        fixture.bindings.start().unwrap();
        fixture.bindings.close();
        assert!(fixture.services.unsubscribed.get());
        assert!(fixture.services.unsubscribed_input.get());
        assert!(fixture.bindings.players().unwrap().is_empty());
    }
}
