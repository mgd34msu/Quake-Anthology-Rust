//! Saved-game menus: save, load, name entry, and overwrite confirm.
//!
//! Ported from donor `src/ui/saves/menu.ts`. Four menus share one state
//! bundle (page, name draft, selected slot, busy flag, message line) and an
//! optional [`SavedGameMenuService`]. With no service the menus render the
//! donor's remote-game reason and every action button is disabled.
//!
//! The donor service is async (`refresh`/`save`/`load` return promises and
//! the menu shows "Working..." while they run). This port is synchronous:
//! [`SavedGameMenuService`] methods return plain values, and the "Working..."
//! message is set and cleared within the same call. The busy flag still
//! guards re-entrant activation.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ui::common::controller::NativeUiController;
use crate::ui::common::layout::{menu_row, MenuRowOptions};
use crate::ui::types::{SeatUiController as _, UiControl, UiControlId, UiControlKind, UiMenu, UiMenuId};

/// Save-game menu ids.
pub const SAVE_MENU_ID: &str = "menu:saves:save";
/// Load-game menu id.
pub const LOAD_MENU_ID: &str = "menu:saves:load";
/// New saved-game name entry menu id.
pub const SAVE_NAME_MENU_ID: &str = "menu:saves:name";
/// Overwrite confirmation menu id.
pub const SAVE_CONFIRM_MENU_ID: &str = "menu:saves:overwrite";

/// Rows per save/load page.
const PAGE_SIZE: usize = 5;
/// Message wrap width in characters.
const MESSAGE_WIDTH: usize = 48;
/// Name entry maximum length in characters.
const NAME_MAXIMUM_LENGTH: usize = 48;

/// Reason shown when no service is bound (donor text).
const REMOTE_REASON: &str = "Remote games cannot be saved or loaded here.";

fn menu_id(text: &str) -> UiMenuId {
    match UiMenuId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI menu id is invalid: {text}"),
    }
}

fn control_id(text: &str) -> UiControlId {
    match UiControlId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI control id is invalid: {text}"),
    }
}

fn row(index: i32) -> crate::text::draw2d::Rect {
    menu_row(index, &MenuRowOptions::default())
}

/// Which action an availability check applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SaveAction {
    /// Writing a saved game.
    Save,
    /// Reading a saved game.
    Load,
}

/// One saved-game list row (UI-local projection of the donor startup list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveListRow {
    /// Slot identity.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Map name shown when the slot is available.
    pub map: String,
    /// Reason the slot cannot be used, if any.
    pub unavailable: Option<String>,
}

/// Saved-game list snapshot plus a list-level error, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveListView {
    /// Rows in display order.
    pub rows: Vec<SaveListRow>,
    /// List-level error (for example an unreadable directory).
    pub error: Option<String>,
}

impl SaveListView {
    /// Empty list without an error.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            rows: Vec::new(),
            error: None,
        }
    }
}

/// Saved-game backend behind the menus.
///
/// The donor interface is async; every method here is the synchronous
/// equivalent, returning `Result<_, String>` where the donor promise could
/// reject with an `Error`. The donor's optional `recovery.restart` field is
/// intentionally omitted: the menu code never reads it (only the player-death
/// flow does), so it is not part of the menu contract.
pub trait SavedGameMenuService {
    /// Current list snapshot.
    fn list(&self) -> SaveListView;
    /// Rescan the saved games.
    fn refresh(&mut self) -> Result<(), String>;
    /// Reason `action` is unavailable, if any.
    fn unavailable(&self, action: SaveAction) -> Option<String>;
    /// Save `name`, overwriting slot `overwrite` when set.
    fn save(&mut self, name: &str, overwrite: Option<&str>) -> Result<(), String>;
    /// Load slot `id`.
    fn load(&mut self, id: &str) -> Result<(), String>;
}

/// Shared save-menu state: page, name draft, selected slot, busy, message.
#[derive(Debug)]
struct SavesState {
    page: usize,
    draft: String,
    selected: Option<String>,
    busy: bool,
    message: String,
}

impl SavesState {
    fn new() -> Self {
        Self {
            page: 0,
            draft: "Save 001".to_string(),
            selected: None,
            busy: false,
            message: String::new(),
        }
    }
}

/// Everything the four menu factories and their callbacks share.
struct SavesShared {
    state: Rc<RefCell<SavesState>>,
    service: Option<Rc<RefCell<dyn SavedGameMenuService>>>,
    controller: Rc<RefCell<NativeUiController>>,
    save: UiMenuId,
    load: UiMenuId,
    name: UiMenuId,
    confirm: UiMenuId,
}

impl SavesShared {
    fn reason(&self, saving: bool) -> Option<String> {
        let action = if saving { SaveAction::Save } else { SaveAction::Load };
        match &self.service {
            Some(service) => service.borrow().unavailable(action),
            None => Some(REMOTE_REASON.to_string()),
        }
    }

    /// Run one operation behind the busy flag, routing errors to the message.
    fn run(&self, operation: impl FnOnce() -> Result<(), String>) {
        if self.state.borrow().busy {
            return;
        }
        self.state.borrow_mut().busy = true;
        self.state.borrow_mut().message = "Working...".to_string();
        match operation() {
            Ok(()) => {
                self.state.borrow_mut().message = String::new();
            }
            Err(error) => {
                self.state.borrow_mut().message = error;
            }
        }
        self.state.borrow_mut().busy = false;
    }

    fn refresh(&self) {
        self.run(|| match &self.service {
            Some(service) => service.borrow_mut().refresh(),
            None => Ok(()),
        });
    }

    /// Write the draft name, then close the menu (donor `write`).
    ///
    /// On success the donor sets "Game saved." inside the operation and the
    /// surrounding `run` then clears it, so the visible message ends empty;
    /// this port keeps that exact order.
    fn write(&self) {
        self.run(|| {
            let Some(service) = &self.service else {
                let reason = self.reason(true).unwrap_or_else(|| "Saving unavailable".to_string());
                return Err(reason);
            };
            let (draft, selected) = {
                let state = self.state.borrow();
                (state.draft.clone(), state.selected.clone())
            };
            service.borrow_mut().save(&draft, selected.as_deref())?;
            service.borrow_mut().refresh()?;
            self.controller.borrow_mut().close_menu();
            self.state.borrow_mut().message = "Game saved.".to_string();
            Ok(())
        });
    }
}

/// Wrap a message into at most two disabled button lines (donor `info`).
fn info_buttons(label: &str) -> Vec<UiControl> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in label.split_whitespace() {
        if line.len() + word.len() > MESSAGE_WIDTH {
            lines.push(line);
            line = word.to_string();
        } else {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    lines.push(line);
    lines.truncate(2);
    lines
        .into_iter()
        .enumerate()
        .map(|(index, text)| UiControl {
            id: control_id(&format!("ui:saves:message:{index}")),
            label: text,
            rect: row(9 + index as i32),
            enabled: false,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(|_| {}),
            },
        })
        .collect()
}

fn button(
    shared: &Rc<SavesShared>,
    id: &str,
    label: String,
    row_index: i32,
    enabled: bool,
    on_activate: Rc<dyn Fn()>,
) -> UiControl {
    let busy = shared.state.borrow().busy;
    UiControl {
        id: control_id(&format!("ui:saves:{id}")),
        label,
        rect: row(row_index),
        enabled: enabled && !busy,
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(move |_| on_activate()),
        },
    }
}

fn back_button(shared: &Rc<SavesShared>) -> UiControl {
    let controller = Rc::clone(&shared.controller);
    button(
        shared,
        "back",
        "Cancel".to_string(),
        11,
        true,
        Rc::new(move || {
            controller.borrow_mut().close_menu();
        }),
    )
}

/// Build the save (`saving`) or load list controls (donor `rows`).
fn list_controls(shared: &Rc<SavesShared>, saving: bool) -> Vec<UiControl> {
    let list = match &shared.service {
        Some(service) => service.borrow().list(),
        None => SaveListView::empty(),
    };
    let pages = (list.rows.len().div_ceil(PAGE_SIZE)).max(1);
    {
        let mut state = shared.state.borrow_mut();
        state.page = state.page.min(pages - 1);
    }
    let page = shared.state.borrow().page;
    let reason = shared.reason(saving);
    let mut controls: Vec<UiControl> = list
        .rows
        .iter()
        .skip(page * PAGE_SIZE)
        .take(PAGE_SIZE)
        .enumerate()
        .map(|(index, list_row)| {
            let row_id = list_row.id.clone();
            let row_label = list_row.label.clone();
            let row_unavailable = list_row.unavailable.clone();
            let label = format!(
                "{} - {}",
                list_row.label,
                if list_row.unavailable.is_none() {
                    list_row.map.clone()
                } else {
                    "unavailable".to_string()
                }
            );
            let inner = Rc::clone(shared);
            let slot = row_id.clone();
            button(
                shared,
                &format!("slot:{row_id}"),
                label,
                index as i32 + 1,
                reason.is_none(),
                Rc::new(move || {
                    if let Some(unavailable) = &row_unavailable {
                        inner.state.borrow_mut().message = unavailable.clone();
                        return;
                    }
                    if saving {
                        inner.state.borrow_mut().selected = Some(slot.clone());
                        inner.state.borrow_mut().draft = row_label.clone();
                        let confirm = inner.confirm.clone();
                        let _ = inner.controller.borrow_mut().open_menu(&confirm);
                    } else {
                        let service = inner.service.clone();
                        let id = slot.clone();
                        inner.run(|| match &service {
                            Some(service) => service.borrow_mut().load(&id),
                            None => Ok(()),
                        });
                    }
                }),
            )
        })
        .collect();
    if saving {
        let inner = Rc::clone(shared);
        let rows = list.rows.clone();
        controls.insert(
            0,
            button(
                shared,
                "new",
                "New saved game".to_string(),
                0,
                reason.is_none(),
                Rc::new(move || {
                    let mut index = 1;
                    while rows.iter().any(|row| row.label == format!("Save {index:03}")) {
                        index += 1;
                    }
                    inner.state.borrow_mut().selected = None;
                    inner.state.borrow_mut().draft = format!("Save {index:03}");
                    let name = inner.name.clone();
                    let _ = inner.controller.borrow_mut().open_menu(&name);
                }),
            ),
        );
    }
    if pages > 1 {
        let previous = Rc::clone(shared);
        controls.push(button(
            shared,
            "previous",
            "Previous page".to_string(),
            6,
            true,
            Rc::new(move || {
                let mut state = previous.state.borrow_mut();
                state.page = (state.page + pages - 1) % pages;
            }),
        ));
        let next = Rc::clone(shared);
        controls.push(button(
            shared,
            "next",
            format!("Next page ({}/{pages})", page + 1),
            7,
            true,
            Rc::new(move || {
                let mut state = next.state.borrow_mut();
                state.page = (state.page + 1) % pages;
            }),
        ));
    }
    let refresh_shared = Rc::clone(shared);
    controls.push(button(
        shared,
        "refresh",
        "Refresh".to_string(),
        8,
        true,
        Rc::new(move || {
            refresh_shared.refresh();
        }),
    ));
    let status = if !shared.state.borrow().message.is_empty() {
        shared.state.borrow().message.clone()
    } else if let Some(reason) = reason {
        reason
    } else if let Some(error) = list.error {
        error
    } else if list.rows.is_empty() {
        "No saved games yet.".to_string()
    } else {
        String::new()
    };
    controls.extend(info_buttons(&status));
    controls.push(back_button(shared));
    controls
}

fn build_list_menu(shared: &Rc<SavesShared>, saving: bool) -> UiMenu {
    let (id, title) = if saving {
        (shared.save.clone(), "Save game".to_string())
    } else {
        (shared.load.clone(), "Load game".to_string())
    };
    let factory = Rc::clone(shared);
    let opener = Rc::clone(shared);
    UiMenu {
        scroll: None,
        id,
        title,
        full_screen: true,
        controls: list_controls(&factory, saving),
        on_open: Rc::new(move |_| {
            opener.state.borrow_mut().page = 0;
            opener.refresh();
        }),
        on_close: Rc::new(|_| {}),
    }
}

fn build_name_menu(shared: &Rc<SavesShared>) -> UiMenu {
    let (draft, message, busy) = {
        let state = shared.state.borrow();
        (state.draft.clone(), state.message.clone(), state.busy)
    };
    let change_shared = Rc::clone(shared);
    let submit_shared = Rc::clone(shared);
    let write_shared = Rc::clone(shared);
    UiMenu {
        scroll: None,
        id: shared.name.clone(),
        title: "New saved game".to_string(),
        full_screen: true,
        controls: [
            vec![UiControl {
                id: control_id("ui:saves:name"),
                label: "Name".to_string(),
                rect: row(1),
                enabled: !busy,
                visible: true,
                kind: UiControlKind::TextEntry {
                    masked: false,
                    text: draft,
                    maximum_length: NAME_MAXIMUM_LENGTH,
                    on_change: Rc::new(move |_, value: &str| {
                        change_shared.state.borrow_mut().draft = value.to_string();
                    }),
                    on_submit: Rc::new(move |_, _| {
                        submit_shared.write();
                    }),
                },
            }],
            vec![button(
                shared,
                "write",
                "Save game".to_string(),
                3,
                true,
                Rc::new(move || {
                    write_shared.write();
                }),
            )],
            info_buttons(&message),
            vec![back_button(shared)],
        ]
        .concat(),
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

fn build_confirm_menu(shared: &Rc<SavesShared>) -> UiMenu {
    let (draft, message) = {
        let state = shared.state.borrow();
        (state.draft.clone(), state.message.clone())
    };
    let write_shared = Rc::clone(shared);
    UiMenu {
        scroll: None,
        id: shared.confirm.clone(),
        title: "Overwrite saved game?".to_string(),
        full_screen: true,
        controls: [
            vec![button(shared, "selected", draft, 1, false, Rc::new(|| {}))],
            vec![button(
                shared,
                "overwrite",
                "Overwrite this saved game".to_string(),
                3,
                true,
                Rc::new(move || {
                    write_shared.write();
                }),
            )],
            info_buttons(&message),
            vec![back_button(shared)],
        ]
        .concat(),
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

/// Registered saved-game menus: save/load ids plus disposal.
pub struct SavedGameMenus {
    /// Save menu id.
    pub save: UiMenuId,
    /// Load menu id.
    pub load: UiMenuId,
    name: UiMenuId,
    confirm: UiMenuId,
    controller: Rc<RefCell<NativeUiController>>,
    /// Live state shared with the registered factories (tests rebuild menus).
    #[cfg(test)]
    shared: Rc<SavesShared>,
}

impl SavedGameMenus {
    /// Unregister all four menus in reverse registration order.
    pub fn dispose(self) {
        let mut controller = self.controller.borrow_mut();
        controller.unregister(&self.confirm);
        controller.unregister(&self.name);
        controller.unregister(&self.load);
        controller.unregister(&self.save);
    }
}

impl std::fmt::Debug for SavedGameMenus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedGameMenus")
            .field("save", &self.save)
            .field("load", &self.load)
            .finish_non_exhaustive()
    }
}

/// Register the save, load, name-entry, and overwrite-confirm menus.
pub fn register_saved_game_menus(
    controller: &Rc<RefCell<NativeUiController>>,
    service: Option<Rc<RefCell<dyn SavedGameMenuService>>>,
) -> SavedGameMenus {
    let shared = Rc::new(SavesShared {
        state: Rc::new(RefCell::new(SavesState::new())),
        service,
        controller: Rc::clone(controller),
        save: menu_id(SAVE_MENU_ID),
        load: menu_id(LOAD_MENU_ID),
        name: menu_id(SAVE_NAME_MENU_ID),
        confirm: menu_id(SAVE_CONFIRM_MENU_ID),
    });
    let save_factory = Rc::clone(&shared);
    let load_factory = Rc::clone(&shared);
    let name_factory = Rc::clone(&shared);
    let confirm_factory = Rc::clone(&shared);
    let save = shared.save.clone();
    let load = shared.load.clone();
    let name = shared.name.clone();
    let confirm = shared.confirm.clone();
    {
        let mut controller = controller.borrow_mut();
        controller.register(save.clone(), Rc::new(move || build_list_menu(&save_factory, true)));
        controller.register(load.clone(), Rc::new(move || build_list_menu(&load_factory, false)));
        controller.register(name.clone(), Rc::new(move || build_name_menu(&name_factory)));
        controller.register(confirm.clone(), Rc::new(move || build_confirm_menu(&confirm_factory)));
    }
    SavedGameMenus {
        save,
        load,
        name,
        confirm,
        controller: Rc::clone(controller),
        #[cfg(test)]
        shared,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    use crate::ui::common::controller::headless_options;

    struct FakeService {
        list: SaveListView,
        reason: Option<String>,
        refreshes: usize,
        refresh_error: Option<String>,
        saves: Vec<(String, Option<String>)>,
        save_error: Option<String>,
        loads: Vec<String>,
        load_error: Option<String>,
    }

    impl FakeService {
        fn with_rows(count: usize) -> Self {
            Self {
                list: SaveListView {
                    rows: (1..=count)
                        .map(|index| SaveListRow {
                            id: format!("slot-{index}"),
                            label: format!("Slot {index}"),
                            map: format!("e1m{index}"),
                            unavailable: None,
                        })
                        .collect(),
                    error: None,
                },
                reason: None,
                refreshes: 0,
                refresh_error: None,
                saves: Vec::new(),
                save_error: None,
                loads: Vec::new(),
                load_error: None,
            }
        }
    }

    impl SavedGameMenuService for FakeService {
        fn list(&self) -> SaveListView {
            self.list.clone()
        }

        fn refresh(&mut self) -> Result<(), String> {
            self.refreshes += 1;
            match &self.refresh_error {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }

        fn unavailable(&self, _action: SaveAction) -> Option<String> {
            self.reason.clone()
        }

        fn save(&mut self, name: &str, overwrite: Option<&str>) -> Result<(), String> {
            if let Some(error) = &self.save_error {
                return Err(error.clone());
            }
            self.saves.push((name.to_string(), overwrite.map(str::to_string)));
            Ok(())
        }

        fn load(&mut self, id: &str) -> Result<(), String> {
            if let Some(error) = &self.load_error {
                return Err(error.clone());
            }
            self.loads.push(id.to_string());
            Ok(())
        }
    }

    fn harness(
        service: FakeService,
    ) -> (
        Rc<RefCell<NativeUiController>>,
        Rc<RefCell<FakeService>>,
        SavedGameMenus,
        qa_core::identity::SeatId,
    ) {
        let owner = IdentityOwner::create("saves-menu-test").expect("owner");
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
        let concrete: Rc<RefCell<FakeService>> = Rc::new(RefCell::new(service));
        let erased: Rc<RefCell<dyn SavedGameMenuService>> = concrete.clone();
        let menus = register_saved_game_menus(&controller, Some(erased));
        (controller, concrete, menus, seat)
    }

    fn control(menu: &UiMenu, id: &str) -> UiControl {
        menu.controls
            .iter()
            .find(|control| control.id.as_str() == id)
            .unwrap_or_else(|| panic!("missing control: {id}"))
            .clone()
    }

    fn activate(control: &UiControl, seat: &qa_core::identity::SeatId) {
        match &control.kind {
            UiControlKind::Button { on_activate } => on_activate(seat.clone()),
            other => panic!("expected a button: {other:?}"),
        }
    }

    #[test]
    fn register_dispose_and_open_flow() {
        let (controller, service, menus, _) = harness(FakeService::with_rows(2));
        assert_eq!(menus.save.as_str(), SAVE_MENU_ID);
        assert_eq!(menus.load.as_str(), LOAD_MENU_ID);
        for id in [SAVE_MENU_ID, LOAD_MENU_ID, SAVE_NAME_MENU_ID, SAVE_CONFIRM_MENU_ID] {
            assert!(controller.borrow().is_registered(&menu_id(id)));
        }
        controller.borrow_mut().open_menu(&menus.save).expect("open save");
        assert_eq!(controller.borrow().active_menu(), Some(menus.save.clone()));
        assert_eq!(service.borrow().refreshes, 1);
        controller.borrow_mut().close_menu();
        assert_eq!(controller.borrow().active_menu(), None);
        menus.dispose();
        for id in [SAVE_MENU_ID, LOAD_MENU_ID, SAVE_NAME_MENU_ID, SAVE_CONFIRM_MENU_ID] {
            assert!(!controller.borrow().is_registered(&menu_id(id)));
        }
    }

    fn slot_ids(menu: &UiMenu) -> Vec<String> {
        menu.controls
            .iter()
            .filter(|control| control.id.as_str().starts_with("ui:saves:slot:"))
            .map(|control| control.id.as_str().to_string())
            .collect()
    }

    #[test]
    fn paging_shows_five_rows_and_wraps() {
        let (controller, _, menus, seat) = harness(FakeService::with_rows(7));
        controller.borrow_mut().open_menu(&menus.save).expect("open save");
        let menu = build_list_menu(&menus.shared, true);
        assert_eq!(menu.title, "Save game");
        assert!(menu.full_screen);
        assert_eq!(slot_ids(&menu).len(), 5);
        assert!(control(&menu, "ui:saves:new").enabled);
        assert_eq!(control(&menu, "ui:saves:next").label, "Next page (1/2)");
        activate(&control(&menu, "ui:saves:next"), &seat);
        let menu = build_list_menu(&menus.shared, true);
        assert_eq!(slot_ids(&menu).len(), 2);
        assert_eq!(control(&menu, "ui:saves:next").label, "Next page (2/2)");
        activate(&control(&menu, "ui:saves:next"), &seat);
        let menu = build_list_menu(&menus.shared, true);
        assert_eq!(slot_ids(&menu).len(), 5);
        activate(&control(&menu, "ui:saves:previous"), &seat);
        let menu = build_list_menu(&menus.shared, true);
        assert_eq!(slot_ids(&menu).len(), 2);
        activate(&control(&menu, "ui:saves:previous"), &seat);
        let menu = build_list_menu(&menus.shared, true);
        assert_eq!(slot_ids(&menu).len(), 5);
    }

    #[test]
    fn single_page_hides_paging_buttons() {
        let (_, _, menus, _) = harness(FakeService::with_rows(3));
        let menu = build_list_menu(&menus.shared, false);
        assert_eq!(menu.title, "Load game");
        assert!(menu
            .controls
            .iter()
            .all(|control| { control.id.as_str() != "ui:saves:previous" && control.id.as_str() != "ui:saves:next" }));
        assert!(menu
            .controls
            .iter()
            .all(|control| control.id.as_str() != "ui:saves:new"));
    }

    #[test]
    fn new_save_numbers_first_free_slot_and_writes() {
        let (controller, service, menus, seat) = harness(FakeService {
            list: SaveListView {
                rows: vec![
                    SaveListRow {
                        id: "a".to_string(),
                        label: "Save 001".to_string(),
                        map: "e1m1".to_string(),
                        unavailable: None,
                    },
                    SaveListRow {
                        id: "b".to_string(),
                        label: "Save 002".to_string(),
                        map: "e1m2".to_string(),
                        unavailable: None,
                    },
                ],
                error: None,
            },
            ..FakeService::with_rows(0)
        });
        controller.borrow_mut().open_menu(&menus.save).expect("open save");
        let menu = build_list_menu(&menus.shared, true);
        activate(&control(&menu, "ui:saves:new"), &seat);
        assert_eq!(menus.shared.state.borrow().draft, "Save 003");
        assert_eq!(menus.shared.state.borrow().selected, None);
        assert_eq!(controller.borrow().active_menu(), Some(menu_id(SAVE_NAME_MENU_ID)));
        let menu = build_name_menu(&menus.shared);
        assert_eq!(menu.title, "New saved game");
        match &control(&menu, "ui:saves:name").kind {
            UiControlKind::TextEntry {
                masked,
                text,
                maximum_length,
                on_change,
                on_submit,
            } => {
                assert!(!masked);
                assert_eq!(text, "Save 003");
                assert_eq!(*maximum_length, 48);
                on_change(seat.clone(), "My Save");
                on_submit(seat.clone(), "My Save");
            }
            other => panic!("expected a text entry: {other:?}"),
        }
        assert_eq!(service.borrow().saves, vec![("My Save".to_string(), None)]);
        assert_eq!(controller.borrow().active_menu(), Some(menus.save.clone()));
        assert!(menus.shared.state.borrow().message.is_empty());
        assert!(!menus.shared.state.borrow().busy);
    }

    #[test]
    fn overwrite_flow_saves_selected_slot() {
        let (controller, service, menus, seat) = harness(FakeService::with_rows(2));
        controller.borrow_mut().open_menu(&menus.save).expect("open save");
        let menu = build_list_menu(&menus.shared, true);
        assert_eq!(control(&menu, "ui:saves:slot:slot-1").label, "Slot 1 - e1m1");
        activate(&control(&menu, "ui:saves:slot:slot-1"), &seat);
        assert_eq!(menus.shared.state.borrow().selected, Some("slot-1".to_string()));
        assert_eq!(menus.shared.state.borrow().draft, "Slot 1");
        assert_eq!(controller.borrow().active_menu(), Some(menu_id(SAVE_CONFIRM_MENU_ID)));
        let menu = build_confirm_menu(&menus.shared);
        assert_eq!(menu.title, "Overwrite saved game?");
        assert_eq!(control(&menu, "ui:saves:selected").label, "Slot 1");
        assert!(!control(&menu, "ui:saves:selected").enabled);
        activate(&control(&menu, "ui:saves:overwrite"), &seat);
        assert_eq!(
            service.borrow().saves,
            vec![("Slot 1".to_string(), Some("slot-1".to_string()))]
        );
        assert_eq!(controller.borrow().active_menu(), Some(menus.save.clone()));
    }

    #[test]
    fn load_flow_loads_slot() {
        let (controller, service, menus, seat) = harness(FakeService::with_rows(2));
        controller.borrow_mut().open_menu(&menus.load).expect("open load");
        let menu = build_list_menu(&menus.shared, false);
        activate(&control(&menu, "ui:saves:slot:slot-2"), &seat);
        assert_eq!(service.borrow().loads, vec!["slot-2".to_string()]);
        assert!(menus.shared.state.borrow().message.is_empty());
        assert_eq!(controller.borrow().active_menu(), Some(menus.load.clone()));
    }

    #[test]
    fn unavailable_row_sets_message_and_renders_unavailable() {
        let (_, _, menus, seat) = harness(FakeService {
            list: SaveListView {
                rows: vec![SaveListRow {
                    id: "broken".to_string(),
                    label: "Broken".to_string(),
                    map: "e1m1".to_string(),
                    unavailable: Some("Missing content.".to_string()),
                }],
                error: None,
            },
            ..FakeService::with_rows(0)
        });
        let menu = build_list_menu(&menus.shared, false);
        assert_eq!(control(&menu, "ui:saves:slot:broken").label, "Broken - unavailable");
        activate(&control(&menu, "ui:saves:slot:broken"), &seat);
        assert_eq!(menus.shared.state.borrow().message, "Missing content.");
        let menu = build_list_menu(&menus.shared, false);
        assert_eq!(control(&menu, "ui:saves:message:0").label, "Missing content.");
    }

    #[test]
    fn reason_disables_actions_and_shows_status() {
        let (_, _, menus, _) = harness(FakeService {
            reason: Some("Saving is disabled.".to_string()),
            ..FakeService::with_rows(1)
        });
        let menu = build_list_menu(&menus.shared, true);
        assert!(!control(&menu, "ui:saves:new").enabled);
        assert!(!control(&menu, "ui:saves:slot:slot-1").enabled);
        assert!(control(&menu, "ui:saves:refresh").enabled);
        assert_eq!(control(&menu, "ui:saves:message:0").label, "Saving is disabled.");
    }

    #[test]
    fn busy_blocks_reentrant_operations() {
        let (controller, service, menus, seat) = harness(FakeService::with_rows(1));
        menus.shared.state.borrow_mut().busy = true;
        let menu = build_list_menu(&menus.shared, true);
        assert!(!control(&menu, "ui:saves:refresh").enabled);
        activate(&control(&menu, "ui:saves:refresh"), &seat);
        assert_eq!(service.borrow().refreshes, 0);
        menus.shared.state.borrow_mut().busy = false;
        let _ = controller;
    }

    #[test]
    fn refresh_save_and_load_errors_reach_message() {
        let (controller, service, menus, seat) = harness(FakeService {
            refresh_error: Some("Scan failed.".to_string()),
            save_error: Some("Disk full.".to_string()),
            load_error: Some("Slot gone.".to_string()),
            ..FakeService::with_rows(1)
        });
        controller.borrow_mut().open_menu(&menus.save).expect("open save");
        assert_eq!(menus.shared.state.borrow().message, "Scan failed.");
        service.borrow_mut().refresh_error = None;
        let menu = build_list_menu(&menus.shared, true);
        activate(&control(&menu, "ui:saves:refresh"), &seat);
        assert!(menus.shared.state.borrow().message.is_empty());
        controller
            .borrow_mut()
            .open_menu(&menu_id(SAVE_NAME_MENU_ID))
            .expect("open name");
        let menu = build_name_menu(&menus.shared);
        activate(&control(&menu, "ui:saves:write"), &seat);
        assert_eq!(menus.shared.state.borrow().message, "Disk full.");
        let menu = build_list_menu(&menus.shared, false);
        activate(&control(&menu, "ui:saves:slot:slot-1"), &seat);
        assert_eq!(menus.shared.state.borrow().message, "Slot gone.");
        assert!(!menus.shared.state.borrow().busy);
    }

    #[test]
    fn empty_list_and_list_error_status() {
        let (_, _, menus, _) = harness(FakeService::with_rows(0));
        let menu = build_list_menu(&menus.shared, false);
        assert_eq!(control(&menu, "ui:saves:message:0").label, "No saved games yet.");
        let (_, _, menus, _) = harness(FakeService {
            list: SaveListView {
                rows: Vec::new(),
                error: Some("Directory unreadable.".to_string()),
            },
            ..FakeService::with_rows(0)
        });
        let menu = build_list_menu(&menus.shared, false);
        assert_eq!(control(&menu, "ui:saves:message:0").label, "Directory unreadable.");
    }

    #[test]
    fn missing_service_reports_remote_reason() {
        let owner = IdentityOwner::create("saves-menu-remote-test").expect("owner");
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
        let menus = register_saved_game_menus(&controller, None);
        controller.borrow_mut().open_menu(&menus.save).expect("open save");
        let menu = build_list_menu(&menus.shared, true);
        assert!(!control(&menu, "ui:saves:new").enabled);
        assert_eq!(
            control(&menu, "ui:saves:message:0").label,
            "Remote games cannot be saved or loaded here."
        );
        controller
            .borrow_mut()
            .open_menu(&menu_id(SAVE_NAME_MENU_ID))
            .expect("open name");
        let menu = build_name_menu(&menus.shared);
        activate(&control(&menu, "ui:saves:write"), &seat);
        assert_eq!(
            menus.shared.state.borrow().message,
            "Remote games cannot be saved or loaded here."
        );
        menus.dispose();
    }
}
