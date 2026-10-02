//! Arena-selection menu registration.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/base-arena-select-menu.ts`
//! (`registerArenaSelectionMenu`). Menu navigation goes through
//! [`ArenaMenuController`], implemented by the native controller; tier and
//! selected-arena state live in shared cells because factories are `'static`.

use std::cell::RefCell;
use std::rc::Rc;

use qa_client::text::draw2d::Rect;
use qa_client::ui::common::controller::{NativeUiController, UiMenuFactory};
use qa_client::ui::types::{
    SeatChoiceCallback, SeatUiController, UiChoice, UiControl, UiControlId, UiControlKind, UiListRow, UiMenu, UiMenuId,
};
use qa_client::ClientError;

use super::base_arena_selection::{ArenaSelection, ArenaSelectionRow};

/// Menu navigation surface used by arena menus.
pub trait ArenaMenuController {
    /// Register a menu factory.
    fn register_menu(&mut self, id: UiMenuId, factory: UiMenuFactory);
    /// Unregister a menu.
    fn unregister_menu(&mut self, id: &UiMenuId);
    /// Open a menu.
    fn open_menu(&mut self, id: &UiMenuId) -> Result<(), ClientError>;
    /// Close the top menu.
    fn close_menu(&mut self);
    /// Close every open menu.
    fn close_all_menus(&mut self);
}

impl ArenaMenuController for NativeUiController {
    fn register_menu(&mut self, id: UiMenuId, factory: UiMenuFactory) {
        self.register(id, factory);
    }

    fn unregister_menu(&mut self, id: &UiMenuId) {
        self.unregister(id);
    }

    fn open_menu(&mut self, id: &UiMenuId) -> Result<(), ClientError> {
        SeatUiController::open_menu(self, id)
    }

    fn close_menu(&mut self) {
        SeatUiController::close_menu(self);
    }

    fn close_all_menus(&mut self) {
        self.close_all();
    }
}

/// Open a menu that must have been registered by the arena menus.
pub fn open_arena_menu(controller: &Rc<RefCell<dyn ArenaMenuController>>, id: &UiMenuId) {
    controller.borrow_mut().open_menu(id).expect("arena menu is registered");
}

/// Close the top menu.
pub fn close_arena_menu(controller: &Rc<RefCell<dyn ArenaMenuController>>) {
    controller.borrow_mut().close_menu();
}

/// Selection service backing the menu.
pub trait ArenaSelectionMenuService {
    /// Read the current selection, if arenas loaded.
    fn read(&self) -> Option<ArenaSelection>;
    /// Choose a map.
    fn choose(&mut self, map: &str);
}

/// Shared tier/selected state.
#[derive(Debug, Default)]
struct SelectionMenuState {
    tier: String,
    selected: Option<String>,
}

fn control_id(id: &str) -> UiControlId {
    UiControlId::new(id).expect("static arena control id")
}

fn build_selection_menu<S>(
    id: &UiMenuId,
    controller: &Rc<RefCell<dyn ArenaMenuController>>,
    service: &Rc<RefCell<S>>,
    state: &Rc<RefCell<SelectionMenuState>>,
) -> UiMenu
where
    S: ArenaSelectionMenuService + 'static,
{
    let selection = service.borrow().read();
    {
        let mut state = state.borrow_mut();
        if let Some(selection) = &selection {
            if !selection.tiers.iter().any(|row| row.id == state.tier) {
                state.tier = selection
                    .rows
                    .iter()
                    .find(|row| Some(row.arena.map.clone()) == selection.current)
                    .map(|row| row.tier.clone())
                    .or_else(|| selection.tiers.first().map(|tier| tier.id.clone()))
                    .unwrap_or_default();
            }
        }
        let rows: Vec<&ArenaSelectionRow> = selection
            .iter()
            .flat_map(|selection| selection.rows.iter())
            .filter(|row| row.tier == state.tier)
            .collect();
        if !rows.iter().any(|row| Some(row.arena.map.clone()) == state.selected) {
            state.selected = rows
                .iter()
                .find(|row| {
                    Some(row.arena.map.clone()) == selection.as_ref().and_then(|selection| selection.current.clone())
                })
                .map(|row| row.arena.map.clone())
                .or_else(|| rows.first().map(|row| row.arena.map.clone()));
        }
    }
    let state_view = state.borrow();
    let rows: Vec<ArenaSelectionRow> = selection
        .as_ref()
        .map(|selection| {
            selection
                .rows
                .iter()
                .filter(|row| row.tier == state_view.tier)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let current = rows
        .iter()
        .find(|row| Some(row.arena.map.clone()) == state_view.selected)
        .cloned();
    drop(state_view);

    let choose_service = Rc::clone(service);
    let choose_map = current
        .clone()
        .filter(|row| row.available)
        .map(|row| row.arena.map.clone());
    let choose = move || {
        if let Some(map) = &choose_map {
            choose_service.borrow_mut().choose(map);
        }
    };
    let choose_callback = Rc::new({
        let choose = choose;
        move |_: qa_core::identity::SeatId| choose()
    });

    let tier_state = Rc::clone(state);
    let tier_select: SeatChoiceCallback = Rc::new(move |_, value: &str| {
        let mut state = tier_state.borrow_mut();
        state.tier = value.to_string();
        state.selected = None;
    });
    let list_state = Rc::clone(state);
    let list_select: SeatChoiceCallback = Rc::new(move |_, value: &str| {
        list_state.borrow_mut().selected = Some(value.to_string());
    });
    let activate_rows: Vec<(String, bool)> = rows.iter().map(|row| (row.arena.map.clone(), row.available)).collect();
    let activate_service = Rc::clone(service);
    let list_activate: SeatChoiceCallback = Rc::new(move |_, value: &str| {
        if activate_rows.iter().any(|(map, available)| map == value && *available) {
            activate_service.borrow_mut().choose(value);
        }
    });

    let opponents = match &current {
        None => "No single-player arenas are available.".to_string(),
        Some(row) => {
            let bots = row.arena.bots.join(", ");
            format!("Opponents: {}", if bots.is_empty() { "None".to_string() } else { bots })
        }
    };
    let limits = match &current {
        None => String::new(),
        Some(row) => [
            if row.arena.frag_limit > 0 {
                format!("{} frags", row.arena.frag_limit)
            } else {
                String::new()
            },
            if row.arena.time_limit > 0 {
                format!("{} minutes", row.arena.time_limit)
            } else {
                String::new()
            },
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · "),
    };
    let label = |suffix: &str, text: String, y: f32| UiControl {
        id: control_id(&format!("ui:arena-select:{suffix}")),
        label: text,
        rect: Rect {
            x: 48.0,
            y,
            width: 544.0,
            height: 30.0,
        },
        enabled: false,
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(|_| {}),
        },
    };

    let back_controller = Rc::clone(controller);
    let open_state = Rc::clone(state);
    UiMenu {
        scroll: None,
        id: id.clone(),
        title: "Choose an arena".to_string(),
        full_screen: true,
        controls: vec![
            UiControl {
                id: control_id("ui:arena-select:tier"),
                label: "Tier".to_string(),
                rect: Rect {
                    x: 48.0,
                    y: 94.0,
                    width: 544.0,
                    height: 32.0,
                },
                enabled: selection.is_some(),
                visible: true,
                kind: UiControlKind::Choice {
                    choices: selection
                        .as_ref()
                        .map(|selection| {
                            selection
                                .tiers
                                .iter()
                                .map(|tier| UiChoice {
                                    id: tier.id.clone(),
                                    label: tier.label.clone(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    selected: {
                        let tier = state.borrow().tier.clone();
                        if tier.is_empty() {
                            None
                        } else {
                            Some(tier)
                        }
                    },
                    on_select: tier_select,
                },
            },
            UiControl {
                id: control_id("ui:arena-select:arenas"),
                label: "Arenas".to_string(),
                rect: Rect {
                    x: 48.0,
                    y: 144.0,
                    width: 544.0,
                    height: 156.0,
                },
                enabled: !rows.is_empty(),
                visible: true,
                kind: UiControlKind::List {
                    row_height: Some(39.0),
                    column_widths: None,
                    on_activate: Some(list_activate),
                    rows: rows
                        .iter()
                        .map(|row| UiListRow {
                            action: None,
                            id: row.arena.map.clone(),
                            cells: vec![
                                row.arena.title.clone(),
                                if row.available {
                                    row.record.clone()
                                } else {
                                    "Locked".to_string()
                                },
                            ],
                            image: None,
                            enabled: true,
                        })
                        .collect(),
                    selected: state.borrow().selected.clone(),
                    on_select: list_select,
                },
            },
            label("opponents", opponents, 318.0),
            label("limits", limits, 350.0),
            UiControl {
                id: control_id("ui:arena-select:play"),
                label: if current.as_ref().is_some_and(|row| !row.available) {
                    "Win the preceding tier to unlock".to_string()
                } else {
                    "Choose difficulty".to_string()
                },
                rect: Rect {
                    x: 48.0,
                    y: 392.0,
                    width: 544.0,
                    height: 32.0,
                },
                enabled: current.as_ref().is_some_and(|row| row.available),
                visible: true,
                kind: UiControlKind::Button {
                    on_activate: choose_callback,
                },
            },
            UiControl {
                id: control_id("ui:arena-select:back"),
                label: "Back".to_string(),
                rect: Rect {
                    x: 48.0,
                    y: 436.0,
                    width: 544.0,
                    height: 32.0,
                },
                enabled: true,
                visible: true,
                kind: UiControlKind::Button {
                    on_activate: Rc::new(move |_| {
                        back_controller.borrow_mut().close_menu();
                    }),
                },
            },
        ],
        on_open: Rc::new(move |_| {
            let mut state = open_state.borrow_mut();
            state.tier = String::new();
            state.selected = None;
        }),
        on_close: Rc::new(|_| {}),
    }
}

/// Register the arena-selection menu (`registerArenaSelectionMenu`).
pub fn register_arena_selection_menu<S>(
    controller: Rc<RefCell<dyn ArenaMenuController>>,
    id: UiMenuId,
    service: Rc<RefCell<S>>,
) -> Box<dyn FnOnce()>
where
    S: ArenaSelectionMenuService + 'static,
{
    let state = Rc::new(RefCell::new(SelectionMenuState::default()));
    let factory_state = Rc::clone(&state);
    let factory_service = Rc::clone(&service);
    let factory_controller = Rc::clone(&controller);
    let factory_id = id.clone();
    controller.borrow_mut().register_menu(
        id.clone(),
        Rc::new(move || build_selection_menu(&factory_id, &factory_controller, &factory_service, &factory_state)),
    );
    Box::new(move || {
        controller.borrow_mut().unregister_menu(&id);
    })
}

#[cfg(test)]
mod tests {
    use super::super::base_arena_catalog::BaseArena;
    use super::super::base_arena_selection::ArenaSelectionTier;
    use super::*;
    use qa_core::identity::{IdentityOwner, SeatId};

    struct StubController {
        menus: std::collections::HashMap<UiMenuId, UiMenuFactory>,
        opened: Vec<UiMenuId>,
        closed: usize,
    }

    impl StubController {
        fn build(&self, id: &UiMenuId) -> UiMenu {
            self.menus.get(id).unwrap()()
        }
    }

    impl ArenaMenuController for StubController {
        fn register_menu(&mut self, id: UiMenuId, factory: UiMenuFactory) {
            self.menus.insert(id, factory);
        }

        fn unregister_menu(&mut self, id: &UiMenuId) {
            self.menus.remove(id);
        }

        fn open_menu(&mut self, id: &UiMenuId) -> Result<(), ClientError> {
            if !self.menus.contains_key(id) {
                return Err(ClientError::BadUi("unknown menu".to_string()));
            }
            self.opened.push(id.clone());
            Ok(())
        }

        fn close_menu(&mut self) {
            self.closed += 1;
        }

        fn close_all_menus(&mut self) {}
    }

    struct StubService {
        selection: Option<ArenaSelection>,
        chosen: Vec<String>,
    }

    impl ArenaSelectionMenuService for StubService {
        fn read(&self) -> Option<ArenaSelection> {
            self.selection.clone()
        }

        fn choose(&mut self, map: &str) {
            self.chosen.push(map.to_string());
        }
    }

    fn test_selection() -> ArenaSelection {
        let row = |map: &str, tier: &str, available: bool| ArenaSelectionRow {
            arena: BaseArena {
                number: 0,
                map: map.to_string(),
                title: "Arena".to_string(),
                bots: vec!["Sarge".to_string()],
                special: String::new(),
                selection: 0,
                frag_limit: 10,
                time_limit: 0,
            },
            tier: tier.to_string(),
            available,
            record: "Not completed".to_string(),
        };
        ArenaSelection {
            tiers: vec![
                ArenaSelectionTier {
                    id: "1".to_string(),
                    label: "Tier 1".to_string(),
                },
                ArenaSelectionTier {
                    id: "2".to_string(),
                    label: "Tier 2".to_string(),
                },
            ],
            rows: vec![row("maps/q3dm1.bsp", "1", true), row("maps/q3dm2.bsp", "2", false)],
            current: Some("maps/q3dm1.bsp".to_string()),
        }
    }

    fn seat() -> SeatId {
        IdentityOwner::create("test").unwrap().seat(0)
    }

    type Harness = (
        Rc<RefCell<StubController>>,
        Rc<RefCell<StubService>>,
        UiMenuId,
        Box<dyn FnOnce()>,
    );

    fn harness() -> Harness {
        let stub = Rc::new(RefCell::new(StubController {
            menus: std::collections::HashMap::new(),
            opened: Vec::new(),
            closed: 0,
        }));
        let controller: Rc<RefCell<dyn ArenaMenuController>> = stub.clone();
        let service = Rc::new(RefCell::new(StubService {
            selection: Some(test_selection()),
            chosen: Vec::new(),
        }));
        let id = UiMenuId::new("menu:application:arena-selection").unwrap();
        let unregister = register_arena_selection_menu(controller, id.clone(), Rc::clone(&service));
        (stub, service, id, unregister)
    }

    #[test]
    fn menu_lists_current_tier_and_details() {
        let (controller, service, id, _unregister) = harness();
        let menu = controller.borrow().build(&id);
        assert_eq!(menu.title, "Choose an arena");
        assert_eq!(menu.controls.len(), 6);
        let list = &menu.controls[1];
        match &list.kind {
            UiControlKind::List { rows, selected, .. } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].id, "maps/q3dm1.bsp");
                assert_eq!(selected.as_deref(), Some("maps/q3dm1.bsp"));
            }
            kind => panic!("expected list, got {kind:?}"),
        }
        assert!(menu.controls[2].label.contains("Sarge"));
        assert!(menu.controls[3].label.contains("10 frags"));
        assert_eq!(menu.controls[4].label, "Choose difficulty");
        assert!(menu.controls[4].enabled);
        let _ = service;
    }

    #[test]
    fn tier_switch_and_choose_flow() {
        let (controller, service, id, _unregister) = harness();
        let menu = controller.borrow().build(&id);
        match &menu.controls[0].kind {
            UiControlKind::Choice { on_select, .. } => on_select(seat(), "2"),
            kind => panic!("expected choice, got {kind:?}"),
        }
        let menu = controller.borrow().build(&id);
        match &menu.controls[1].kind {
            UiControlKind::List { rows, on_activate, .. } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].cells[1], "Locked");
                (on_activate.clone().unwrap())(seat(), "maps/q3dm2.bsp");
            }
            kind => panic!("expected list, got {kind:?}"),
        }
        assert!(service.borrow().chosen.is_empty());
        match &menu.controls[0].kind {
            UiControlKind::Choice { on_select, .. } => on_select(seat(), "1"),
            kind => panic!("expected choice, got {kind:?}"),
        }
        let menu = controller.borrow().build(&id);
        match &menu.controls[4].kind {
            UiControlKind::Button { on_activate } => on_activate(seat()),
            kind => panic!("expected button, got {kind:?}"),
        }
        assert_eq!(service.borrow().chosen, vec!["maps/q3dm1.bsp".to_string()]);
    }

    #[test]
    fn empty_selection_disables_controls() {
        let stub = Rc::new(RefCell::new(StubController {
            menus: std::collections::HashMap::new(),
            opened: Vec::new(),
            closed: 0,
        }));
        let controller: Rc<RefCell<dyn ArenaMenuController>> = stub.clone();
        let service = Rc::new(RefCell::new(StubService {
            selection: None,
            chosen: Vec::new(),
        }));
        let id = UiMenuId::new("menu:application:arena-selection").unwrap();
        let unregister = register_arena_selection_menu(controller, id.clone(), Rc::clone(&service));
        let menu = stub.borrow().build(&id);
        assert!(!menu.controls[0].enabled);
        assert!(!menu.controls[1].enabled);
        assert!(!menu.controls[4].enabled);
        assert!(menu.controls[2].label.contains("No single-player arenas"));
        unregister();
        assert!(!stub.borrow().menus.contains_key(&id));
    }
}
