//! Local lobby menu: host, join, ready, start, and leave rows over a shared local lobby service.
//!
//! Donor provenance: `src/ui/settings/local-lobby.ts` in full. View shapes
//! mirror `src/app/bootstrap/local-lobby.ts` (`ApplicationLocalLobby`,
//! `LocalLobbySelection`) and the lobby member shapes from
//! `src/network/services/online.ts`, kept UI-local so this crate never binds
//! the network service types.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ui::common::controller::NativeUiController;
use crate::ui::common::layout::{menu_row, MenuRowOptions};
use crate::ui::types::{SeatUiController as _, UiControl, UiControlId, UiControlKind, UiMenu, UiMenuId};

/// Menu id for the local lobby root (`menu:local:lobby`).
const ROOT_ID: &str = "menu:local:lobby";
/// Status text when no operation is running and no error is latched.
const IDLE_STATUS: &str = "Local shared service; not a retail platform lobby.";
/// Status fallback when an operation fails without a message.
const FAILED_STATUS: &str = "Local lobby operation failed.";
/// Lobbies and members shown per page.
const PAGE_SIZE: usize = 4;
/// Default hosted lobby name.
const DEFAULT_NAME: &str = "Local lobby";
/// Default hosted lobby capacity.
const DEFAULT_CAPACITY: u32 = 4;
/// Maximum lobby name length in characters.
const MAX_NAME_LENGTH: usize = 64;
/// Minimum hosted lobby capacity.
const MIN_CAPACITY: u32 = 1;
/// Maximum hosted lobby capacity.
const MAX_CAPACITY: u32 = 64;

/// Build a control id from a static template; the templates below always
/// carry the `ui:` namespace and a name part, so a failure is a programming
/// bug.
fn control_id(text: &str) -> UiControlId {
    match UiControlId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI control id is invalid: {text}"),
    }
}

/// Build a menu id from a static template; the template below always carries
/// the `menu:` namespace and a scope part, so a failure is a programming bug.
fn menu_id(text: &str) -> UiMenuId {
    match UiMenuId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI menu id is invalid: {text}"),
    }
}

/// UI-local mirror of the donor `Account`: one lobby member identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyAccountView {
    /// Account id (`account:...`).
    pub id: String,
    /// Display name.
    pub name: String,
}

/// UI-local mirror of the donor `LobbyMember`: one seated membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyMemberView {
    /// Member account.
    pub account: LobbyAccountView,
    /// Readiness flag.
    pub ready: bool,
    /// Seats held by this member.
    pub seats: u32,
}

/// UI-local mirror of the donor lobby phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LobbyPhase {
    /// Open for joins and readiness changes.
    Open,
    /// Host is binding the next match.
    Starting,
    /// A match is running.
    Playing,
}

impl LobbyPhase {
    /// Donor phase name.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            LobbyPhase::Open => "open",
            LobbyPhase::Starting => "starting",
            LobbyPhase::Playing => "playing",
        }
    }
}

/// UI-local mirror of the donor `Lobby`: identity, phase, and membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyView {
    /// Lobby id (`lobby:...`).
    pub id: String,
    /// Lobby name.
    pub name: String,
    /// Current phase.
    pub phase: LobbyPhase,
    /// Player capacity in seats.
    pub capacity: u32,
    /// Current members.
    pub members: Vec<LobbyMemberView>,
    /// Owner account id.
    pub owner: String,
    /// Launched match generation; zero means no match has started.
    pub match_generation: u64,
}

/// UI-local mirror of the donor `CompositionIdentity`: the selected game a
/// hosted lobby will launch. The digest identifies the composition; the
/// description is a human-readable summary for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostComposition {
    /// Composition digest text.
    pub digest: String,
    /// Human-readable composition summary.
    pub description: String,
}

/// UI-local mirror of the donor `LocalLobbySelection`: the selected game
/// handed to [`LocalLobby::host`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSelection {
    /// Selected game composition.
    pub composition: HostComposition,
}

/// UI-local mirror of the donor `ApplicationLocalLobby`.
///
/// The donor methods are asynchronous and serialize through an internal tail
/// queue; this surface is synchronous and reports failures as
/// `Result<(), String>` so menu callbacks can latch the message into the
/// status row inline. Adapters over asynchronous services must complete the
/// operation before returning so the status row reflects the outcome.
pub trait LocalLobby {
    /// Current account id.
    fn account_id(&self) -> String;
    /// Current account name.
    fn account_name(&self) -> String;
    /// Current membership, if any.
    fn current(&self) -> Option<LobbyView>;
    /// Every visible lobby.
    fn list(&self) -> Vec<LobbyView>;
    /// Host a new lobby running `selection`, holding `seats` seats.
    fn host(&mut self, name: &str, capacity: u32, selection: &HostSelection, seats: u32) -> Result<(), String>;
    /// Join one lobby, holding `seats` seats.
    fn join(&mut self, id: &str, seats: u32) -> Result<(), String>;
    /// Set local readiness.
    fn ready(&mut self, value: bool) -> Result<(), String>;
    /// Start the next match as the lobby host.
    fn start(&mut self) -> Result<(), String>;
    /// Leave the current lobby; hosts close it.
    fn leave(&mut self) -> Result<(), String>;
}

/// Mutable menu draft: hosted-lobby name/capacity, pager, and run state.
struct LobbyMenuState {
    name: String,
    capacity: u32,
    busy: bool,
    status: String,
    page: usize,
    generation: u64,
}

/// Run one lobby operation, latching failures into the status row.
///
/// Mirrors the donor `run` closure: re-entrant runs are dropped, the status
/// row clears on entry, and only the latest generation writes back. Calls are
/// synchronous, so `busy` never persists across frames the way the donor
/// asynchronous flow can; `generation` still guards the post-dispose case.
fn run(state: &Rc<RefCell<LobbyMenuState>>, operation: impl FnOnce() -> Result<(), String>) {
    if state.borrow().busy {
        return;
    }
    let request = {
        let mut guard = state.borrow_mut();
        guard.busy = true;
        guard.status.clear();
        guard.generation
    };
    let result = operation();
    let mut guard = state.borrow_mut();
    if request == guard.generation {
        if let Err(message) = result {
            guard.status = if message.is_empty() {
                FAILED_STATUS.to_string()
            } else {
                message
            };
        }
        guard.busy = false;
    }
}

/// Current-lobby provider.
pub type LobbySource = Rc<dyn Fn() -> Option<Rc<RefCell<dyn LocalLobby>>>>;

/// Shared inputs for [`build_local_lobby_menu`].
struct LobbyMenuInputs {
    current: LobbySource,
    host_selection: Rc<dyn Fn() -> Result<HostSelection, String>>,
    read_seats: Rc<dyn Fn() -> u32>,
    controller: Rc<RefCell<NativeUiController>>,
    state: Rc<RefCell<LobbyMenuState>>,
}

/// Build one button row.
fn button(id: &str, label: String, row: i32, enabled: bool, action: Rc<dyn Fn()>) -> UiControl {
    UiControl {
        id: control_id(id),
        label,
        rect: menu_row(row, &MenuRowOptions::default()),
        enabled,
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(move |_| action()),
        },
    }
}

/// Seats held across every member of one lobby.
fn seats_used(lobby: &LobbyView) -> u32 {
    lobby.members.iter().map(|member| member.seats).sum()
}

/// Phase line for the member-list header row.
fn phase_label(lobby: &LobbyView) -> String {
    let phase = match lobby.phase {
        LobbyPhase::Playing => "Playing",
        LobbyPhase::Starting => "Starting host",
        LobbyPhase::Open => "Waiting for readiness",
    };
    format!("{}: {phase}", lobby.name)
}

/// Build the local lobby menu from live service state.
fn build_local_lobby_menu(root: &UiMenuId, inputs: &LobbyMenuInputs) -> UiMenu {
    let owner = (inputs.current)();
    let lobby = owner.as_ref().and_then(|service| service.borrow().current());
    let mut controls: Vec<UiControl> = Vec::new();
    let Some(service) = owner else {
        controls.push(button(
            "ui:lobby:unavailable",
            "No local lobby service is configured.".to_string(),
            0,
            false,
            Rc::new(|| {}),
        ));
        return finish_menu(root, inputs, controls);
    };
    if lobby.is_none() {
        build_join_list(&service, inputs, &mut controls);
    } else if let Some(lobby) = lobby {
        build_member_list(&service, &lobby, inputs, &mut controls);
    }
    finish_menu(root, inputs, controls)
}

/// Build the host form plus the paged open-lobby join list.
fn build_join_list(service: &Rc<RefCell<dyn LocalLobby>>, inputs: &LobbyMenuInputs, controls: &mut Vec<UiControl>) {
    let available: Vec<LobbyView> = service
        .borrow()
        .list()
        .into_iter()
        .filter(|candidate| candidate.phase == LobbyPhase::Open)
        .collect();
    let (busy, name, capacity) = {
        let mut guard = inputs.state.borrow_mut();
        let last = available.len().div_ceil(PAGE_SIZE).saturating_sub(1);
        guard.page = guard.page.min(last);
        (guard.busy, guard.name.clone(), guard.capacity)
    };
    let name_state = Rc::clone(&inputs.state);
    controls.push(UiControl {
        id: control_id("ui:lobby:name"),
        label: "Lobby name".to_string(),
        rect: menu_row(0, &MenuRowOptions::default()),
        enabled: !busy,
        visible: true,
        kind: UiControlKind::TextEntry {
            masked: false,
            text: name,
            maximum_length: MAX_NAME_LENGTH,
            on_change: Rc::new(move |_, value: &str| {
                name_state.borrow_mut().name = value.to_string();
            }),
            on_submit: Rc::new(|_, _| {}),
        },
    });
    let capacity_state = Rc::clone(&inputs.state);
    controls.push(UiControl {
        id: control_id("ui:lobby:capacity"),
        label: "Player capacity".to_string(),
        rect: menu_row(1, &MenuRowOptions::default()),
        enabled: !busy,
        visible: true,
        kind: UiControlKind::Slider {
            minimum: MIN_CAPACITY as f32,
            maximum: MAX_CAPACITY as f32,
            step: 1.0,
            value: capacity as f32,
            value_label: None,
            on_change: Rc::new(move |_, value| {
                let clamped = value.round().clamp(MIN_CAPACITY as f32, MAX_CAPACITY as f32) as u32;
                capacity_state.borrow_mut().capacity = clamped;
            }),
        },
    });
    let host_service = Rc::clone(service);
    let host_inputs_state = Rc::clone(&inputs.state);
    let host_selection = Rc::clone(&inputs.host_selection);
    let host_read_seats = Rc::clone(&inputs.read_seats);
    let host_state = Rc::clone(&inputs.state);
    let host_name = inputs.state.borrow().name.clone();
    controls.push(button(
        "ui:lobby:host",
        "Host selected game".to_string(),
        2,
        !busy && !host_name.trim().is_empty(),
        Rc::new(move || {
            run(&host_state, || {
                let selection = host_selection()?;
                let (name, capacity) = {
                    let guard = host_inputs_state.borrow();
                    (guard.name.clone(), guard.capacity)
                };
                host_service
                    .borrow_mut()
                    .host(&name, capacity, &selection, host_read_seats())
            });
        }),
    ));
    let page = inputs.state.borrow().page;
    for (index, candidate) in available.iter().skip(page * PAGE_SIZE).take(PAGE_SIZE).enumerate() {
        let join_service = Rc::clone(service);
        let join_state = Rc::clone(&inputs.state);
        let join_read_seats = Rc::clone(&inputs.read_seats);
        let id = candidate.id.clone();
        controls.push(button(
            &format!("ui:lobby:join-{index}"),
            format!(
                "Join {} ({}/{})",
                candidate.name,
                seats_used(candidate),
                candidate.capacity
            ),
            3 + index as i32,
            !inputs.state.borrow().busy,
            Rc::new(move || {
                run(&join_state, || join_service.borrow_mut().join(&id, join_read_seats()));
            }),
        ));
    }
    let previous_state = Rc::clone(&inputs.state);
    controls.push(button(
        "ui:lobby:previous",
        "Previous lobbies".to_string(),
        7,
        !busy && page > 0,
        Rc::new(move || {
            let mut guard = previous_state.borrow_mut();
            guard.page = guard.page.saturating_sub(1);
        }),
    ));
    let next_state = Rc::clone(&inputs.state);
    controls.push(button(
        "ui:lobby:next",
        "Next lobbies".to_string(),
        8,
        !busy && (page + 1) * PAGE_SIZE < available.len(),
        Rc::new(move || {
            next_state.borrow_mut().page += 1;
        }),
    ));
}

/// Build the paged member list plus ready/start/leave rows.
fn build_member_list(
    service: &Rc<RefCell<dyn LocalLobby>>,
    lobby: &LobbyView,
    inputs: &LobbyMenuInputs,
    controls: &mut Vec<UiControl>,
) {
    let account_id = service.borrow().account_id();
    let member = lobby
        .members
        .iter()
        .find(|candidate| candidate.account.id == account_id);
    let host = lobby.owner == account_id;
    let (busy, page) = {
        let mut guard = inputs.state.borrow_mut();
        let pages = lobby.members.len().div_ceil(PAGE_SIZE).max(1);
        guard.page = guard.page.min(pages - 1);
        (guard.busy, guard.page)
    };
    controls.push(button("ui:lobby:phase", phase_label(lobby), 0, false, Rc::new(|| {})));
    for (index, entry) in lobby.members.iter().skip(page * PAGE_SIZE).take(PAGE_SIZE).enumerate() {
        let seat_word = if entry.seats == 1 { "seat" } else { "seats" };
        let readiness = if entry.ready { "Ready" } else { "Not ready" };
        controls.push(button(
            &format!("ui:lobby:member-{index}"),
            format!("{}: {readiness} ({} {seat_word})", entry.account.name, entry.seats),
            1 + index as i32,
            false,
            Rc::new(|| {}),
        ));
    }
    let was_ready = member.map(|entry| entry.ready).unwrap_or(false);
    let ready_service = Rc::clone(service);
    let ready_state = Rc::clone(&inputs.state);
    controls.push(button(
        "ui:lobby:ready",
        if was_ready { "Not ready" } else { "Ready" }.to_string(),
        5,
        !busy && lobby.phase == LobbyPhase::Open,
        Rc::new(move || {
            run(&ready_state, || ready_service.borrow_mut().ready(!was_ready));
        }),
    ));
    let start_service = Rc::clone(service);
    let start_state = Rc::clone(&inputs.state);
    let all_ready = lobby.members.iter().all(|entry| entry.ready);
    controls.push(button(
        "ui:lobby:start",
        if lobby.match_generation == 0 {
            "Start match"
        } else {
            "Start next match"
        }
        .to_string(),
        6,
        !busy && host && lobby.phase == LobbyPhase::Open && all_ready,
        Rc::new(move || run(&start_state, || start_service.borrow_mut().start())),
    ));
    let leave_service = Rc::clone(service);
    let leave_state = Rc::clone(&inputs.state);
    controls.push(button(
        "ui:lobby:leave",
        if host { "Close lobby" } else { "Leave lobby" }.to_string(),
        7,
        !inputs.state.borrow().busy,
        Rc::new(move || run(&leave_state, || leave_service.borrow_mut().leave())),
    ));
    let members_state = Rc::clone(&inputs.state);
    let pages = lobby.members.len().div_ceil(PAGE_SIZE).max(1);
    controls.push(button(
        "ui:lobby:members",
        "More members".to_string(),
        8,
        !busy && pages > 1,
        Rc::new(move || {
            let mut guard = members_state.borrow_mut();
            guard.page = (guard.page + 1) % pages;
        }),
    ));
}

/// Append the status and back rows and wrap the menu shell.
fn finish_menu(root: &UiMenuId, inputs: &LobbyMenuInputs, mut controls: Vec<UiControl>) -> UiMenu {
    let guard = inputs.state.borrow();
    let status = if guard.status.is_empty() {
        if guard.busy {
            "Working...".to_string()
        } else {
            IDLE_STATUS.to_string()
        }
    } else {
        guard.status.clone()
    };
    drop(guard);
    controls.push(button("ui:lobby:status", status, 9, false, Rc::new(|| {})));
    let back_controller = Rc::clone(&inputs.controller);
    controls.push(button(
        "ui:lobby:back",
        "Back".to_string(),
        11,
        !inputs.state.borrow().busy,
        Rc::new(move || {
            back_controller.borrow_mut().close_menu();
        }),
    ));
    UiMenu {
        scroll: None,
        id: root.clone(),
        title: "Local lobbies".to_string(),
        full_screen: false,
        controls,
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

/// Registered local lobby menu plus its disposer.
pub struct LobbyMenus {
    /// Root menu id.
    pub root: UiMenuId,
    controller: Rc<RefCell<NativeUiController>>,
    state: Rc<RefCell<LobbyMenuState>>,
}

impl LobbyMenus {
    /// Bump the run generation and unregister the menu.
    pub fn dispose(self) {
        self.state.borrow_mut().generation += 1;
        self.controller.borrow_mut().unregister(&self.root);
    }
}

impl std::fmt::Debug for LobbyMenus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LobbyMenus")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// Register the local lobby menu.
///
/// `current` resolves the lobby service (none when unconfigured),
/// `host_selection` supplies the selected game for the host row, and
/// `read_seats` supplies the seats each host/join takes. Menu factories
/// capture shared state by value; control callbacks re-enter the controller
/// through the shared handle, so callers must not hold a borrow across input
/// or draw calls that activate those controls.
pub fn register_local_lobby_menu(
    controller: &Rc<RefCell<NativeUiController>>,
    current: LobbySource,
    host_selection: Rc<dyn Fn() -> Result<HostSelection, String>>,
    read_seats: Rc<dyn Fn() -> u32>,
) -> LobbyMenus {
    let root = menu_id(ROOT_ID);
    let state = Rc::new(RefCell::new(LobbyMenuState {
        name: DEFAULT_NAME.to_string(),
        capacity: DEFAULT_CAPACITY,
        busy: false,
        status: String::new(),
        page: 0,
        generation: 0,
    }));
    let factory_root = root.clone();
    let inputs = LobbyMenuInputs {
        current,
        host_selection,
        read_seats,
        controller: Rc::clone(controller),
        state: Rc::clone(&state),
    };
    controller.borrow_mut().register(
        root.clone(),
        Rc::new(move || build_local_lobby_menu(&factory_root, &inputs)),
    );
    LobbyMenus {
        root,
        controller: Rc::clone(controller),
        state,
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;

    use crate::ui::common::controller::headless_options;
    use crate::ui::types::UiControlKind;

    use super::*;

    struct FakeLobby {
        account: LobbyAccountView,
        lobbies: Vec<LobbyView>,
        membership: Option<String>,
        calls: Vec<String>,
        fail_with: Option<String>,
    }

    impl FakeLobby {
        fn new(id: &str, name: &str) -> Self {
            Self {
                account: LobbyAccountView {
                    id: id.to_string(),
                    name: name.to_string(),
                },
                lobbies: Vec::new(),
                membership: None,
                calls: Vec::new(),
                fail_with: None,
            }
        }

        fn open(id: &str, name: &str, owner: &str, members: Vec<LobbyMemberView>) -> LobbyView {
            LobbyView {
                id: id.to_string(),
                name: name.to_string(),
                phase: LobbyPhase::Open,
                capacity: 4,
                members,
                owner: owner.to_string(),
                match_generation: 0,
            }
        }

        fn member(id: &str, name: &str, ready: bool, seats: u32) -> LobbyMemberView {
            LobbyMemberView {
                account: LobbyAccountView {
                    id: id.to_string(),
                    name: name.to_string(),
                },
                ready,
                seats,
            }
        }

        fn fail_if_armed(&self) -> Result<(), String> {
            if let Some(message) = &self.fail_with {
                return Err(message.clone());
            }
            Ok(())
        }

        fn current_index(&self) -> Option<usize> {
            self.membership
                .as_ref()
                .and_then(|id| self.lobbies.iter().position(|lobby| &lobby.id == id))
        }
    }

    impl LocalLobby for FakeLobby {
        fn account_id(&self) -> String {
            self.account.id.clone()
        }

        fn account_name(&self) -> String {
            self.account.name.clone()
        }

        fn current(&self) -> Option<LobbyView> {
            self.current_index().map(|index| self.lobbies[index].clone())
        }

        fn list(&self) -> Vec<LobbyView> {
            self.lobbies.clone()
        }

        fn host(&mut self, name: &str, capacity: u32, _selection: &HostSelection, seats: u32) -> Result<(), String> {
            self.fail_if_armed()?;
            self.calls.push(format!("host:{name}:{capacity}:{seats}"));
            let id = format!("lobby:{}", self.lobbies.len() + 1);
            self.lobbies.push(LobbyView {
                id: id.clone(),
                name: name.to_string(),
                phase: LobbyPhase::Open,
                capacity,
                members: vec![LobbyMemberView {
                    account: self.account.clone(),
                    ready: false,
                    seats,
                }],
                owner: self.account.id.clone(),
                match_generation: 0,
            });
            self.membership = Some(id);
            Ok(())
        }

        fn join(&mut self, id: &str, seats: u32) -> Result<(), String> {
            self.fail_if_armed()?;
            self.calls.push(format!("join:{id}:{seats}"));
            let lobby = self
                .lobbies
                .iter_mut()
                .find(|lobby| lobby.id == id)
                .ok_or_else(|| "Lobby no longer exists".to_string())?;
            lobby.members.push(LobbyMemberView {
                account: self.account.clone(),
                ready: false,
                seats,
            });
            self.membership = Some(id.to_string());
            Ok(())
        }

        fn ready(&mut self, value: bool) -> Result<(), String> {
            self.fail_if_armed()?;
            self.calls.push(format!("ready:{value}"));
            let index = self
                .current_index()
                .ok_or_else(|| "No current local lobby membership".to_string())?;
            let account = self.account.clone();
            let member = self.lobbies[index]
                .members
                .iter_mut()
                .find(|entry| entry.account.id == account.id)
                .ok_or_else(|| "No current local lobby membership".to_string())?;
            member.ready = value;
            Ok(())
        }

        fn start(&mut self) -> Result<(), String> {
            self.fail_if_armed()?;
            self.calls.push("start".to_string());
            let index = self
                .current_index()
                .ok_or_else(|| "No current local lobby membership".to_string())?;
            self.lobbies[index].match_generation += 1;
            Ok(())
        }

        fn leave(&mut self) -> Result<(), String> {
            self.fail_if_armed()?;
            self.calls.push("leave".to_string());
            let Some(index) = self.current_index() else {
                return Err("No current local lobby membership".to_string());
            };
            let account = self.account.clone();
            if self.lobbies[index].owner == account.id {
                self.lobbies.remove(index);
            } else if let Some(entry) = self.lobbies[index]
                .members
                .iter()
                .position(|member| member.account.id == account.id)
            {
                self.lobbies[index].members.remove(entry);
            }
            self.membership = None;
            Ok(())
        }
    }

    fn selection() -> HostSelection {
        HostSelection {
            composition: HostComposition {
                digest: "sha256:test".to_string(),
                description: "Test game".to_string(),
            },
        }
    }

    struct Harness {
        lobby: Rc<RefCell<FakeLobby>>,
        inputs: LobbyMenuInputs,
        root: UiMenuId,
        seat: qa_core::identity::SeatId,
    }

    impl Harness {
        fn new(lobby: FakeLobby) -> Self {
            let owner = IdentityOwner::create("local-lobby-test").unwrap();
            let seat = owner.seat(0);
            let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
            let lobby = Rc::new(RefCell::new(lobby));
            let current_lobby: Rc<RefCell<dyn LocalLobby>> = lobby.clone();
            let state = Rc::new(RefCell::new(LobbyMenuState {
                name: DEFAULT_NAME.to_string(),
                capacity: DEFAULT_CAPACITY,
                busy: false,
                status: String::new(),
                page: 0,
                generation: 0,
            }));
            let inputs = LobbyMenuInputs {
                current: Rc::new(move || Some(Rc::clone(&current_lobby))),
                host_selection: Rc::new(|| Ok(selection())),
                read_seats: Rc::new(|| 1),
                controller: Rc::clone(&controller),
                state,
            };
            Self {
                lobby,
                inputs,
                root: menu_id(ROOT_ID),
                seat,
            }
        }

        fn menu(&self) -> UiMenu {
            build_local_lobby_menu(&self.root, &self.inputs)
        }

        fn control(menu: &UiMenu, id: &str) -> UiControl {
            menu.controls
                .iter()
                .find(|control| control.id.as_str() == id)
                .unwrap_or_else(|| panic!("missing control: {id}"))
                .clone()
        }

        fn has_control(menu: &UiMenu, id: &str) -> bool {
            menu.controls.iter().any(|control| control.id.as_str() == id)
        }

        fn label(menu: &UiMenu, id: &str) -> String {
            Self::control(menu, id).label.clone()
        }

        fn enabled(menu: &UiMenu, id: &str) -> bool {
            Self::control(menu, id).enabled
        }

        fn activate(&self, menu: &UiMenu, id: &str) {
            match &Self::control(menu, id).kind {
                UiControlKind::Button { on_activate } => on_activate(self.seat.clone()),
                other => panic!("control is not a button: {id} ({other:?})"),
            }
        }

        fn change_text(&self, menu: &UiMenu, id: &str, value: &str) {
            match &Self::control(menu, id).kind {
                UiControlKind::TextEntry { on_change, .. } => {
                    on_change(self.seat.clone(), value);
                }
                other => panic!("control is not a text entry: {id} ({other:?})"),
            }
        }

        fn change_slider(&self, menu: &UiMenu, id: &str, value: f32) {
            match &Self::control(menu, id).kind {
                UiControlKind::Slider { on_change, .. } => {
                    on_change(self.seat.clone(), value);
                }
                other => panic!("control is not a slider: {id} ({other:?})"),
            }
        }
    }

    #[test]
    fn unavailable_service_row() {
        let owner = IdentityOwner::create("local-lobby-test").unwrap();
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat))));
        let inputs = LobbyMenuInputs {
            current: Rc::new(|| None),
            host_selection: Rc::new(|| Ok(selection())),
            read_seats: Rc::new(|| 1),
            controller,
            state: Rc::new(RefCell::new(LobbyMenuState {
                name: DEFAULT_NAME.to_string(),
                capacity: DEFAULT_CAPACITY,
                busy: false,
                status: String::new(),
                page: 0,
                generation: 0,
            })),
        };
        let menu = build_local_lobby_menu(&menu_id(ROOT_ID), &inputs);
        assert_eq!(menu.title, "Local lobbies");
        assert!(!menu.full_screen);
        assert_eq!(
            Harness::label(&menu, "ui:lobby:unavailable"),
            "No local lobby service is configured."
        );
        assert!(!Harness::enabled(&menu, "ui:lobby:unavailable"));
        assert_eq!(Harness::label(&menu, "ui:lobby:status"), IDLE_STATUS);
        assert_eq!(Harness::label(&menu, "ui:lobby:back"), "Back");
        assert!(!Harness::has_control(&menu, "ui:lobby:host"));
        assert!(!Harness::has_control(&menu, "ui:lobby:ready"));
    }

    #[test]
    fn no_lobby_host_form_rows() {
        let harness = Harness::new(FakeLobby::new("account:self", "Self"));
        let menu = harness.menu();
        match &Harness::control(&menu, "ui:lobby:name").kind {
            UiControlKind::TextEntry {
                text, maximum_length, ..
            } => {
                assert_eq!(text, "Local lobby");
                assert_eq!(*maximum_length, MAX_NAME_LENGTH);
            }
            other => panic!("expected text entry, got {other:?}"),
        }
        match &Harness::control(&menu, "ui:lobby:capacity").kind {
            UiControlKind::Slider {
                value,
                minimum,
                maximum,
                step,
                ..
            } => {
                assert_eq!((*value, *minimum, *maximum, *step), (4.0, 1.0, 64.0, 1.0));
            }
            other => panic!("expected slider, got {other:?}"),
        }
        assert_eq!(Harness::label(&menu, "ui:lobby:host"), "Host selected game");
        assert!(Harness::enabled(&menu, "ui:lobby:host"));
        assert!(!Harness::enabled(&menu, "ui:lobby:previous"));
        assert!(!Harness::enabled(&menu, "ui:lobby:next"));
    }

    #[test]
    fn host_requires_nonblank_name() {
        let harness = Harness::new(FakeLobby::new("account:self", "Self"));
        harness.change_text(&harness.menu(), "ui:lobby:name", "   ");
        assert!(!Harness::enabled(&harness.menu(), "ui:lobby:host"));
        harness.change_text(&harness.menu(), "ui:lobby:name", "Arena");
        harness.change_slider(&harness.menu(), "ui:lobby:capacity", 8.0);
        let menu = harness.menu();
        assert!(Harness::enabled(&menu, "ui:lobby:host"));
        harness.activate(&menu, "ui:lobby:host");
        assert_eq!(harness.lobby.borrow().calls, vec!["host:Arena:8:1".to_string()]);
        assert!(harness.lobby.borrow().current().is_some());
    }

    #[test]
    fn join_list_rows_page_and_filter_closed_phases() {
        let mut fake = FakeLobby::new("account:self", "Self");
        for index in 0..6 {
            let mut lobby = FakeLobby::open(
                &format!("lobby:{index}"),
                &format!("Lobby {index}"),
                "account:other",
                vec![FakeLobby::member("account:other", "Other", true, 2)],
            );
            if index == 5 {
                lobby.phase = LobbyPhase::Playing;
            }
            fake.lobbies.push(lobby);
        }
        let harness = Harness::new(fake);
        let menu = harness.menu();
        assert_eq!(Harness::label(&menu, "ui:lobby:join-0"), "Join Lobby 0 (2/4)");
        assert!(Harness::has_control(&menu, "ui:lobby:join-3"));
        assert!(!Harness::has_control(&menu, "ui:lobby:join-4"));
        assert!(!Harness::enabled(&menu, "ui:lobby:previous"));
        assert!(Harness::enabled(&menu, "ui:lobby:next"));
        harness.activate(&menu, "ui:lobby:next");
        let menu = harness.menu();
        assert_eq!(Harness::label(&menu, "ui:lobby:join-0"), "Join Lobby 4 (2/4)");
        assert!(!Harness::has_control(&menu, "ui:lobby:join-1"));
        assert!(Harness::enabled(&menu, "ui:lobby:previous"));
        assert!(!Harness::enabled(&menu, "ui:lobby:next"));
        harness.activate(&menu, "ui:lobby:join-0");
        assert_eq!(harness.lobby.borrow().calls, vec!["join:lobby:4:1".to_string()]);
        assert_eq!(
            harness.lobby.borrow().current().map(|lobby| lobby.id),
            Some("lobby:4".to_string())
        );
    }

    #[test]
    fn member_list_rows_for_host() {
        let mut fake = FakeLobby::new("account:self", "Self");
        fake.lobbies.push(FakeLobby::open(
            "lobby:1",
            "Arena",
            "account:self",
            vec![
                FakeLobby::member("account:self", "Self", false, 1),
                FakeLobby::member("account:guest", "Guest", true, 2),
            ],
        ));
        fake.membership = Some("lobby:1".to_string());
        let harness = Harness::new(fake);
        let menu = harness.menu();
        assert_eq!(Harness::label(&menu, "ui:lobby:phase"), "Arena: Waiting for readiness");
        assert_eq!(Harness::label(&menu, "ui:lobby:member-0"), "Self: Not ready (1 seat)");
        assert_eq!(Harness::label(&menu, "ui:lobby:member-1"), "Guest: Ready (2 seats)");
        assert_eq!(Harness::label(&menu, "ui:lobby:ready"), "Ready");
        assert!(Harness::enabled(&menu, "ui:lobby:ready"));
        assert_eq!(Harness::label(&menu, "ui:lobby:start"), "Start match");
        assert!(!Harness::enabled(&menu, "ui:lobby:start"));
        assert_eq!(Harness::label(&menu, "ui:lobby:leave"), "Close lobby");
        assert!(!Harness::enabled(&menu, "ui:lobby:members"));
    }

    #[test]
    fn host_ready_start_leave_flow() {
        let harness = Harness::new(FakeLobby::new("account:self", "Self"));
        harness.activate(&harness.menu(), "ui:lobby:host");
        let menu = harness.menu();
        assert_eq!(Harness::label(&menu, "ui:lobby:ready"), "Ready");
        harness.activate(&menu, "ui:lobby:ready");
        let menu = harness.menu();
        assert_eq!(Harness::label(&menu, "ui:lobby:ready"), "Not ready");
        assert_eq!(Harness::label(&menu, "ui:lobby:start"), "Start match");
        assert!(Harness::enabled(&menu, "ui:lobby:start"));
        harness.activate(&menu, "ui:lobby:start");
        let menu = harness.menu();
        assert_eq!(Harness::label(&menu, "ui:lobby:start"), "Start next match");
        harness.activate(&menu, "ui:lobby:leave");
        assert!(harness.lobby.borrow().current().is_none());
        assert_eq!(
            harness.lobby.borrow().calls,
            vec![
                "host:Local lobby:4:1".to_string(),
                "ready:true".to_string(),
                "start".to_string(),
                "leave".to_string(),
            ]
        );
        assert!(Harness::has_control(&harness.menu(), "ui:lobby:host"));
    }

    #[test]
    fn guest_leave_row_and_non_open_phase_labels() {
        let mut fake = FakeLobby::new("account:self", "Self");
        let mut lobby = FakeLobby::open(
            "lobby:1",
            "Arena",
            "account:host",
            vec![
                FakeLobby::member("account:host", "Host", true, 1),
                FakeLobby::member("account:self", "Self", true, 1),
            ],
        );
        lobby.phase = LobbyPhase::Starting;
        fake.lobbies.push(lobby);
        fake.membership = Some("lobby:1".to_string());
        let harness = Harness::new(fake);
        let menu = harness.menu();
        assert_eq!(Harness::label(&menu, "ui:lobby:phase"), "Arena: Starting host");
        assert_eq!(Harness::label(&menu, "ui:lobby:leave"), "Leave lobby");
        assert!(!Harness::enabled(&menu, "ui:lobby:ready"));
        assert!(!Harness::enabled(&menu, "ui:lobby:start"));
        harness.lobby.borrow_mut().lobbies[0].phase = LobbyPhase::Playing;
        assert_eq!(Harness::label(&harness.menu(), "ui:lobby:phase"), "Arena: Playing");
    }

    #[test]
    fn member_pager_cycles() {
        let mut fake = FakeLobby::new("account:self", "Self");
        let members: Vec<LobbyMemberView> = (0..5)
            .map(|index| FakeLobby::member(&format!("account:{index}"), &format!("P{index}"), true, 1))
            .collect();
        fake.lobbies
            .push(FakeLobby::open("lobby:1", "Arena", "account:0", members));
        fake.membership = Some("lobby:1".to_string());
        let harness = Harness::new(fake);
        assert!(Harness::enabled(&harness.menu(), "ui:lobby:members"));
        harness.activate(&harness.menu(), "ui:lobby:members");
        let menu = harness.menu();
        assert_eq!(Harness::label(&menu, "ui:lobby:member-0"), "P4: Ready (1 seat)");
        harness.activate(&menu, "ui:lobby:members");
        assert!(Harness::label(&harness.menu(), "ui:lobby:member-0").starts_with("P0: Ready"));
    }

    #[test]
    fn operation_errors_land_in_status_row() {
        let mut fake = FakeLobby::new("account:self", "Self");
        fake.fail_with = Some("Lobby no longer exists".to_string());
        fake.lobbies.push(FakeLobby::open(
            "lobby:1",
            "Arena",
            "account:other",
            vec![FakeLobby::member("account:other", "Other", true, 1)],
        ));
        let harness = Harness::new(fake);
        harness.activate(&harness.menu(), "ui:lobby:join-0");
        assert_eq!(
            Harness::label(&harness.menu(), "ui:lobby:status"),
            "Lobby no longer exists"
        );
        harness.lobby.borrow_mut().fail_with = Some(String::new());
        harness.activate(&harness.menu(), "ui:lobby:host");
        assert_eq!(Harness::label(&harness.menu(), "ui:lobby:status"), FAILED_STATUS);
    }

    #[test]
    fn host_selection_errors_land_in_status_row() {
        let harness = Harness::new(FakeLobby::new("account:self", "Self"));
        let failing = LobbyMenuInputs {
            current: Rc::clone(&harness.inputs.current),
            host_selection: Rc::new(|| Err("No game selected".to_string())),
            read_seats: Rc::clone(&harness.inputs.read_seats),
            controller: Rc::clone(&harness.inputs.controller),
            state: Rc::clone(&harness.inputs.state),
        };
        let menu = build_local_lobby_menu(&harness.root, &failing);
        harness.activate(&menu, "ui:lobby:host");
        assert!(harness.lobby.borrow().calls.is_empty());
        assert_eq!(
            Harness::label(&build_local_lobby_menu(&harness.root, &failing), "ui:lobby:status"),
            "No game selected"
        );
    }

    #[test]
    fn register_and_dispose_round_trip() {
        let owner = IdentityOwner::create("local-lobby-test").unwrap();
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat))));
        let lobby: Rc<RefCell<dyn LocalLobby>> = Rc::new(RefCell::new(FakeLobby::new("account:self", "Self")));
        let menus = register_local_lobby_menu(
            &controller,
            Rc::new(move || Some(Rc::clone(&lobby))),
            Rc::new(|| Ok(selection())),
            Rc::new(|| 2),
        );
        assert_eq!(menus.root.as_str(), ROOT_ID);
        assert!(controller.borrow().is_registered(&menus.root));
        menus.dispose();
        assert!(!controller.borrow().is_registered(&menu_id(ROOT_ID)));
    }
}
