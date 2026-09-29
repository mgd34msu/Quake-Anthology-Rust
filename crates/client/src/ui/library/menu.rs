//! Generic library menu.
//!
//! Ported from donor `src/ui/library/menu.ts`: a searchable entry list with
//! scope memory, optional create/stop actions, a status row, and Back. The
//! saves and mods menus build on this service contract.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::text::draw2d::Rect;
use crate::ui::common::controller::NativeUiController;
use crate::ui::types::{SeatUiController as _, UiControl, UiControlId, UiControlKind, UiListRow, UiMenu, UiMenuId};

fn control_id(text: &str) -> UiControlId {
    match UiControlId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI control id is invalid: {text}"),
    }
}

/// One library entry row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryEntry {
    /// Stable entry identity.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Secondary detail text, if any.
    pub detail: Option<String>,
    /// Unavailability reason; present means the row cannot activate.
    pub unavailable: Option<String>,
}

/// Backing service for a library menu.
pub trait LibraryMenuService {
    /// Current scope; search text and selection are remembered per scope.
    fn scope(&self) -> String {
        String::new()
    }
    /// Current entries.
    fn entries(&self) -> Vec<LibraryEntry>;
    /// Status line text.
    fn status(&self) -> String;
    /// Reload entries.
    fn refresh(&mut self);
    /// Activate one available entry.
    fn activate(&mut self, id: &str);
    /// Create-action label; `None` hides the create row.
    fn create_label(&self) -> Option<String> {
        None
    }
    /// Submit the create row with the current name text.
    fn create_submit(&mut self, name: &str) {
        let _ = name;
    }
    /// Stop-action label; `None` hides the stop button.
    fn stop_label(&self) -> Option<String> {
        None
    }
    /// Run the stop action.
    fn stop_activate(&mut self) {}
}

/// Mutable library menu view state.
#[derive(Debug, Clone, Default)]
pub struct LibraryMenuState {
    /// Search query text.
    pub query: String,
    /// Selected entry id.
    pub selected: Option<String>,
    /// Create-row name text.
    pub name: String,
    /// Last observed scope.
    pub scope: String,
    /// Saved query/selection per scope.
    pub views: HashMap<String, (String, Option<String>)>,
}

/// Live library menu inputs.
pub struct LibraryMenuInputs {
    /// Backing service.
    pub service: Rc<RefCell<dyn LibraryMenuService>>,
    /// Shared controller for the Back button.
    pub controller: Rc<RefCell<NativeUiController>>,
    /// Mutable view state.
    pub state: Rc<RefCell<LibraryMenuState>>,
}

/// Registered library menu: root id plus disposal.
pub struct LibraryMenus {
    /// Menu root id.
    pub root: UiMenuId,
    controller: Rc<RefCell<NativeUiController>>,
}

impl LibraryMenus {
    /// Unregister the library menu.
    pub fn dispose(self) {
        self.controller.borrow_mut().unregister(&self.root);
    }
}

impl std::fmt::Debug for LibraryMenus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibraryMenus")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

fn library_button(
    suffix: &str,
    label: String,
    x: f32,
    y: f32,
    width: f32,
    enabled: bool,
    on_activate: Rc<dyn Fn(qa_core::identity::SeatId)>,
) -> UiControl {
    UiControl {
        id: control_id(&format!("ui:library:{suffix}")),
        label,
        rect: Rect {
            x,
            y,
            width,
            height: 30.0,
        },
        enabled,
        visible: true,
        kind: UiControlKind::Button { on_activate },
    }
}

fn matches_query(entry: &LibraryEntry, query: &str) -> bool {
    let haystack = match &entry.detail {
        Some(detail) => format!("{} {detail}", entry.label),
        None => entry.label.clone(),
    };
    haystack.to_lowercase().contains(&query.to_lowercase())
}

/// Build one library menu frame from the service and view state.
pub fn build_library_menu(id: &UiMenuId, title: &str, inputs: &LibraryMenuInputs) -> UiMenu {
    let next_scope = inputs.service.borrow().scope();
    {
        let mut state = inputs.state.borrow_mut();
        if next_scope != state.scope {
            let saved_scope = state.scope.clone();
            let saved_query = state.query.clone();
            let saved_selected = state.selected.clone();
            state.views.insert(saved_scope, (saved_query, saved_selected));
            let restored = state.views.get(&next_scope).cloned();
            state.query = restored.as_ref().map(|(query, _)| query.clone()).unwrap_or_default();
            state.selected = restored.and_then(|(_, selected)| selected);
            state.scope.clone_from(&next_scope);
        }
    }
    let entries: Vec<LibraryEntry> = {
        let query = inputs.state.borrow().query.clone();
        inputs
            .service
            .borrow()
            .entries()
            .into_iter()
            .filter(|entry| matches_query(entry, &query))
            .collect()
    };
    {
        let mut state = inputs.state.borrow_mut();
        if !entries.iter().any(|entry| Some(&entry.id) == state.selected.as_ref()) {
            state.selected = entries.first().map(|entry| entry.id.clone());
        }
    }
    let snapshot = inputs.state.borrow().clone();
    let state = Rc::clone(&inputs.state);
    let service = Rc::clone(&inputs.service);
    let select_state = Rc::clone(&state);
    let activate_service = Rc::clone(&service);
    let open_service = Rc::clone(&service);
    let open_state = Rc::clone(&state);
    let refresh_service = Rc::clone(&service);
    let query_state = Rc::clone(&state);
    let query_submit = Rc::clone(&state);
    let create_label = service.borrow().create_label();
    let stop_label = service.borrow().stop_label();
    let selected_available = snapshot.selected.as_ref().is_some_and(|selected| {
        entries
            .iter()
            .any(|entry| &entry.id == selected && entry.unavailable.is_none())
    });
    let mut controls = vec![
        UiControl {
            id: control_id("ui:library:search"),
            label: "Search".to_string(),
            rect: Rect {
                x: 48.0,
                y: 94.0,
                width: 544.0,
                height: 30.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::TextEntry {
                masked: false,
                text: snapshot.query.clone(),
                maximum_length: 80,
                on_change: Rc::new(move |_, value: &str| {
                    query_state.borrow_mut().query = value.to_string();
                }),
                on_submit: Rc::new(move |_, value: &str| {
                    query_submit.borrow_mut().query = value.to_string();
                }),
            },
        },
        UiControl {
            id: control_id("ui:library:entries"),
            label: title.to_string(),
            rect: Rect {
                x: 48.0,
                y: 132.0,
                width: 544.0,
                height: 198.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::List {
                row_height: Some(33.0),
                column_widths: None,
                on_activate: Some(Rc::new(move |_, value: &str| {
                    let available = activate_service
                        .borrow()
                        .entries()
                        .into_iter()
                        .any(|entry| entry.id == value && entry.unavailable.is_none());
                    if available {
                        activate_service.borrow_mut().activate(value);
                    }
                })),
                rows: entries
                    .iter()
                    .map(|entry| UiListRow {
                        action: None,
                        id: entry.id.clone(),
                        cells: vec![
                            entry.label.clone(),
                            entry
                                .unavailable
                                .clone()
                                .or_else(|| entry.detail.clone())
                                .unwrap_or_default(),
                        ],
                        image: None,
                        enabled: entry.unavailable.is_none(),
                    })
                    .collect(),
                selected: snapshot.selected.clone(),
                on_select: Rc::new(move |_, value: &str| {
                    select_state.borrow_mut().selected = Some(value.to_string());
                }),
            },
        },
        library_button(
            "open",
            "Open".to_string(),
            48.0,
            338.0,
            164.0,
            selected_available,
            Rc::new(move |_| {
                if let Some(selected) = open_state.borrow().selected.clone() {
                    let available = open_service
                        .borrow()
                        .entries()
                        .into_iter()
                        .any(|entry| entry.id == selected && entry.unavailable.is_none());
                    if available {
                        open_service.borrow_mut().activate(&selected);
                    }
                }
            }),
        ),
        library_button(
            "refresh",
            "Refresh".to_string(),
            222.0,
            338.0,
            164.0,
            true,
            Rc::new(move |_| {
                refresh_service.borrow_mut().refresh();
            }),
        ),
    ];
    if let Some(label) = stop_label {
        let stop_service = Rc::clone(&service);
        controls.push(library_button(
            "stop",
            label,
            396.0,
            338.0,
            196.0,
            true,
            Rc::new(move |_| {
                stop_service.borrow_mut().stop_activate();
            }),
        ));
    }
    if let Some(label) = create_label {
        let name_state = Rc::clone(&state);
        let name_submit = Rc::clone(&state);
        let create_service = Rc::clone(&service);
        let create_enabled = !snapshot.name.trim().is_empty();
        controls.push(UiControl {
            id: control_id("ui:library:name"),
            label: "Name".to_string(),
            rect: Rect {
                x: 48.0,
                y: 376.0,
                width: 354.0,
                height: 30.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::TextEntry {
                masked: false,
                text: snapshot.name.clone(),
                maximum_length: 128,
                on_change: Rc::new(move |_, value: &str| {
                    name_state.borrow_mut().name = value.to_string();
                }),
                on_submit: Rc::new(move |_, value: &str| {
                    name_submit.borrow_mut().name = value.to_string();
                }),
            },
        });
        let submit_state = Rc::clone(&state);
        controls.push(library_button(
            "create",
            label,
            414.0,
            376.0,
            178.0,
            create_enabled,
            Rc::new(move |_| {
                let name = submit_state.borrow().name.clone();
                create_service.borrow_mut().create_submit(&name);
            }),
        ));
    }
    let status = service.borrow().status();
    let status_label = if status.is_empty() && entries.is_empty() {
        "No matching entries".to_string()
    } else {
        status
    };
    let back_controller = Rc::clone(&inputs.controller);
    let open_service = Rc::clone(&service);
    controls.push(UiControl {
        id: control_id("ui:library:status"),
        label: status_label,
        rect: Rect {
            x: 48.0,
            y: 410.0,
            width: 544.0,
            height: 22.0,
        },
        enabled: false,
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(|_| {}),
        },
    });
    controls.push(library_button(
        "back",
        "Back".to_string(),
        48.0,
        438.0,
        544.0,
        true,
        Rc::new(move |_| {
            back_controller.borrow_mut().close_menu();
        }),
    ));
    UiMenu {
        scroll: None,
        id: id.clone(),
        title: title.to_string(),
        full_screen: true,
        controls,
        on_open: Rc::new(move |_| {
            open_service.borrow_mut().refresh();
        }),
        on_close: Rc::new(|_| {}),
    }
}

/// Register a library menu factory on the shared controller.
pub fn register_library_menu(
    controller: &Rc<RefCell<NativeUiController>>,
    id: &UiMenuId,
    title: &str,
    service: Rc<RefCell<dyn LibraryMenuService>>,
) -> LibraryMenus {
    let root = id.clone();
    let factory_root = root.clone();
    let factory_title = title.to_string();
    let shared = Rc::new(LibraryMenuInputs {
        service: Rc::clone(&service),
        controller: Rc::clone(controller),
        state: Rc::new(RefCell::new(LibraryMenuState {
            scope: service.borrow().scope(),
            ..LibraryMenuState::default()
        })),
    });
    controller.borrow_mut().register(
        root.clone(),
        Rc::new(move || build_library_menu(&factory_root, &factory_title, &shared)),
    );
    LibraryMenus {
        root,
        controller: Rc::clone(controller),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    use crate::ui::common::controller::headless_options;

    struct FakeService {
        scope: String,
        entries: Vec<LibraryEntry>,
        status: String,
        refreshes: usize,
        activated: Vec<String>,
        created: Vec<String>,
        stops: usize,
        with_create: bool,
        with_stop: bool,
    }

    impl FakeService {
        fn new() -> Self {
            Self {
                scope: "a".to_string(),
                entries: vec![
                    LibraryEntry {
                        id: "one".to_string(),
                        label: "Alpha".to_string(),
                        detail: Some("first".to_string()),
                        unavailable: None,
                    },
                    LibraryEntry {
                        id: "two".to_string(),
                        label: "Beta".to_string(),
                        detail: None,
                        unavailable: Some("missing".to_string()),
                    },
                ],
                status: String::new(),
                refreshes: 0,
                activated: Vec::new(),
                created: Vec::new(),
                stops: 0,
                with_create: true,
                with_stop: true,
            }
        }
    }

    impl LibraryMenuService for FakeService {
        fn scope(&self) -> String {
            self.scope.clone()
        }
        fn entries(&self) -> Vec<LibraryEntry> {
            self.entries.clone()
        }
        fn status(&self) -> String {
            self.status.clone()
        }
        fn refresh(&mut self) {
            self.refreshes += 1;
        }
        fn activate(&mut self, id: &str) {
            self.activated.push(id.to_string());
        }
        fn create_label(&self) -> Option<String> {
            self.with_create.then(|| "Create".to_string())
        }
        fn create_submit(&mut self, name: &str) {
            self.created.push(name.to_string());
        }
        fn stop_label(&self) -> Option<String> {
            self.with_stop.then(|| "Stop".to_string())
        }
        fn stop_activate(&mut self) {
            self.stops += 1;
        }
    }

    fn harness(service: FakeService) -> (LibraryMenuInputs, Rc<RefCell<FakeService>>, qa_core::identity::SeatId) {
        let owner = IdentityOwner::create("library-menu-test").expect("owner");
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
        let service = Rc::new(RefCell::new(service));
        let inputs = LibraryMenuInputs {
            service: service.clone(),
            controller,
            state: Rc::new(RefCell::new(LibraryMenuState {
                scope: service.borrow().scope(),
                ..LibraryMenuState::default()
            })),
        };
        (inputs, service, seat)
    }

    fn menu_id_for(text: &str) -> UiMenuId {
        match UiMenuId::new(text) {
            Ok(id) => id,
            Err(_) => panic!("test menu id is invalid: {text}"),
        }
    }

    fn control(menu: &UiMenu, id: &str) -> UiControl {
        menu.controls
            .iter()
            .find(|control| control.id.as_str() == id)
            .unwrap_or_else(|| panic!("missing control: {id}"))
            .clone()
    }

    fn activate(menu: &UiMenu, seat: &qa_core::identity::SeatId, id: &str) {
        match &control(menu, id).kind {
            UiControlKind::Button { on_activate } => on_activate(seat.clone()),
            other => panic!("control is not a button: {id} ({other:?})"),
        }
    }

    #[test]
    fn rows_search_and_rects_match_donor() {
        let (inputs, _, _) = harness(FakeService::new());
        let menu = build_library_menu(&menu_id_for("menu:library:test"), "Saves", &inputs);
        assert_eq!(menu.title, "Saves");
        assert!(menu.full_screen);
        assert_eq!(control(&menu, "ui:library:search").rect.y, 94.0);
        assert_eq!(control(&menu, "ui:library:entries").rect.y, 132.0);
        match &control(&menu, "ui:library:entries").kind {
            UiControlKind::List {
                rows,
                selected,
                row_height,
                ..
            } => {
                assert_eq!(*row_height, Some(33.0));
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0].cells, vec!["Alpha".to_string(), "first".to_string()]);
                assert!(rows[0].enabled);
                assert_eq!(rows[1].cells, vec!["Beta".to_string(), "missing".to_string()]);
                assert!(!rows[1].enabled);
                assert_eq!(selected.as_deref(), Some("one"));
            }
            other => panic!("expected a list: {other:?}"),
        }
        assert!(control(&menu, "ui:library:open").enabled);
        assert_eq!(control(&menu, "ui:library:status").label, "");
        inputs.state.borrow_mut().query = "zzz".to_string();
        let menu = build_library_menu(&menu_id_for("menu:library:test"), "Saves", &inputs);
        assert_eq!(control(&menu, "ui:library:status").label, "No matching entries");
        assert!(!control(&menu, "ui:library:open").enabled);
    }

    #[test]
    fn activate_skips_unavailable_and_open_refresh_stop_create_work() {
        let (inputs, service, seat) = harness(FakeService::new());
        let menu = build_library_menu(&menu_id_for("menu:library:test"), "Saves", &inputs);
        match &control(&menu, "ui:library:entries").kind {
            UiControlKind::List { on_activate, .. } => {
                on_activate.as_ref().expect("row activation")(seat.clone(), "two");
            }
            other => panic!("expected a list: {other:?}"),
        }
        assert!(service.borrow().activated.is_empty());
        activate(&menu, &seat, "ui:library:open");
        activate(&menu, &seat, "ui:library:refresh");
        activate(&menu, &seat, "ui:library:stop");
        (menu.on_open)(seat.clone());
        assert_eq!(service.borrow().activated, vec!["one".to_string()]);
        assert_eq!(service.borrow().refreshes, 2);
        assert_eq!(service.borrow().stops, 1);
        assert!(!control(&menu, "ui:library:create").enabled);
        match &control(&menu, "ui:library:name").kind {
            UiControlKind::TextEntry { on_change, .. } => on_change(seat.clone(), "slot-1"),
            other => panic!("expected a text entry: {other:?}"),
        }
        let menu = build_library_menu(&menu_id_for("menu:library:test"), "Saves", &inputs);
        assert!(control(&menu, "ui:library:create").enabled);
        activate(&menu, &seat, "ui:library:create");
        assert_eq!(service.borrow().created, vec!["slot-1".to_string()]);
    }

    #[test]
    fn scope_change_remembers_views() {
        let (inputs, service, _) = harness(FakeService::new());
        inputs.state.borrow_mut().query = "alpha".to_string();
        inputs.state.borrow_mut().selected = Some("one".to_string());
        service.borrow_mut().scope = "b".to_string();
        let menu = build_library_menu(&menu_id_for("menu:library:test"), "Saves", &inputs);
        assert_eq!(control(&menu, "ui:library:search").label, "Search");
        assert_eq!(inputs.state.borrow().query, "");
        service.borrow_mut().scope = "a".to_string();
        let _ = build_library_menu(&menu_id_for("menu:library:test"), "Saves", &inputs);
        assert_eq!(inputs.state.borrow().query, "alpha");
        assert_eq!(inputs.state.borrow().selected.as_deref(), Some("one"));
        let _ = menu;
    }

    #[test]
    fn register_and_dispose_track_menu() {
        let owner = IdentityOwner::create("library-menu-register-test").expect("owner");
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat))));
        let service: Rc<RefCell<dyn LibraryMenuService>> = Rc::new(RefCell::new(FakeService::new()));
        let id = menu_id_for("menu:library:test");
        let menus = register_library_menu(&controller, &id, "Saves", service);
        assert!(controller.borrow().is_registered(&id));
        menus.dispose();
        assert!(!controller.borrow().is_registered(&id));
    }
}
