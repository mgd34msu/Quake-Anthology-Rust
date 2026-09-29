//! Seat inventory menu.
//!
//! Ported from donor `src/ui/library/inventory.ts`: a list of the seat's
//! current inventory rows plus "Use selected item" and "Close" buttons.
//! Closing the menu stows the held item via `putaway`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::text::draw2d::Rect;
use crate::ui::common::controller::NativeUiController;
use crate::ui::hud::HudInventoryItem;
use crate::ui::types::{SeatUiController as _, UiControl, UiControlId, UiControlKind, UiListRow, UiMenu, UiMenuId};

/// Inventory menu root (`menu:application:inventory`).
pub const INVENTORY_MENU_ID: &str = "menu:application:inventory";

/// Command dispatcher `(name, args)`.
pub type CommandFn = Rc<dyn Fn(&str, &[String])>;

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

/// Live inventory menu inputs.
pub struct InventoryMenuInputs {
    /// Current inventory rows; `None` renders as an empty list.
    pub items: Rc<dyn Fn() -> Option<Vec<HudInventoryItem>>>,
    /// Command dispatcher `(name, args)`.
    pub command: CommandFn,
    /// Shared controller for the Close button.
    pub controller: Rc<RefCell<NativeUiController>>,
    /// Selected row id.
    pub selected: Rc<RefCell<Option<String>>>,
}

/// Registered inventory menu: root id plus disposal.
pub struct InventoryMenus {
    /// Menu root id.
    pub root: UiMenuId,
    controller: Rc<RefCell<NativeUiController>>,
}

impl InventoryMenus {
    /// Unregister the inventory menu.
    pub fn dispose(self) {
        self.controller.borrow_mut().unregister(&self.root);
    }
}

impl std::fmt::Debug for InventoryMenus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InventoryMenus")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// Build one inventory menu frame from the current rows and selection.
pub fn build_inventory_menu(root: &UiMenuId, inputs: &InventoryMenuInputs) -> UiMenu {
    let rows = (inputs.items)().unwrap_or_default();
    {
        let mut selected = inputs.selected.borrow_mut();
        if !rows.iter().any(|item| Some(&item.id) == selected.as_ref()) {
            *selected = rows
                .iter()
                .find(|item| item.selected)
                .map(|item| item.id.clone())
                .or_else(|| rows.first().map(|item| item.id.clone()));
        }
    }
    let selected = inputs.selected.borrow().clone();
    let command = Rc::clone(&inputs.command);
    let use_item: Rc<dyn Fn(&str)> = Rc::new(move |id: &str| {
        command("use", std::slice::from_ref(&id.to_string()));
    });
    let select_state = Rc::clone(&inputs.selected);
    let use_state = Rc::clone(&inputs.selected);
    let use_command = Rc::clone(&use_item);
    let close_controller = Rc::clone(&inputs.controller);
    let putaway = Rc::clone(&inputs.command);
    UiMenu {
        scroll: None,
        id: root.clone(),
        title: "Inventory".to_string(),
        full_screen: false,
        controls: vec![
            UiControl {
                id: control_id("ui:inventory:items"),
                label: "Items".to_string(),
                rect: Rect {
                    x: 64.0,
                    y: 96.0,
                    width: 512.0,
                    height: 264.0,
                },
                enabled: true,
                visible: true,
                kind: UiControlKind::List {
                    row_height: Some(33.0),
                    column_widths: None,
                    on_activate: Some({
                        let use_item = Rc::clone(&use_item);
                        Rc::new(move |_, id: &str| use_item(id))
                    }),
                    rows: rows
                        .iter()
                        .map(|item| UiListRow {
                            action: None,
                            id: item.id.clone(),
                            cells: vec![item.label.clone(), item.count.to_string()],
                            image: item.icon.clone(),
                            enabled: item.count > 0,
                        })
                        .collect(),
                    selected: selected.clone(),
                    on_select: Rc::new(move |_, id: &str| {
                        *select_state.borrow_mut() = Some(id.to_string());
                    }),
                },
            },
            UiControl {
                id: control_id("ui:inventory:use"),
                label: "Use selected item".to_string(),
                rect: Rect {
                    x: 64.0,
                    y: 376.0,
                    width: 512.0,
                    height: 32.0,
                },
                enabled: selected.is_some(),
                visible: true,
                kind: UiControlKind::Button {
                    on_activate: Rc::new(move |_| {
                        if let Some(id) = use_state.borrow().clone() {
                            use_command(&id);
                        }
                    }),
                },
            },
            UiControl {
                id: control_id("ui:inventory:close"),
                label: "Close".to_string(),
                rect: Rect {
                    x: 64.0,
                    y: 424.0,
                    width: 512.0,
                    height: 32.0,
                },
                enabled: true,
                visible: true,
                kind: UiControlKind::Button {
                    on_activate: Rc::new(move |_| {
                        close_controller.borrow_mut().close_menu();
                    }),
                },
            },
        ],
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(move |_| {
            putaway("putaway", &[]);
        }),
    }
}

/// Register the inventory menu factory on the shared controller.
pub fn register_inventory_menu(
    controller: &Rc<RefCell<NativeUiController>>,
    items: Rc<dyn Fn() -> Option<Vec<HudInventoryItem>>>,
    command: CommandFn,
) -> InventoryMenus {
    let root = menu_id(INVENTORY_MENU_ID);
    let factory_root = root.clone();
    let inputs = InventoryMenuInputs {
        items,
        command,
        controller: Rc::clone(controller),
        selected: Rc::new(RefCell::new(None)),
    };
    let shared = Rc::new(inputs);
    controller.borrow_mut().register(
        root.clone(),
        Rc::new(move || build_inventory_menu(&factory_root, &shared)),
    );
    InventoryMenus {
        root,
        controller: Rc::clone(controller),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    use crate::ui::common::controller::headless_options;

    fn item(id: &str, label: &str, count: i32, selected: bool) -> HudInventoryItem {
        HudInventoryItem {
            id: id.to_string(),
            label: label.to_string(),
            count,
            selected,
            binding: None,
            icon: None,
        }
    }

    type CallLog = Rc<RefCell<Vec<(String, Vec<String>)>>>;

    fn harness(rows: Vec<HudInventoryItem>) -> (InventoryMenuInputs, CallLog, qa_core::identity::SeatId) {
        let owner = IdentityOwner::create("library-inventory-test").expect("owner");
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
        let calls: CallLog = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&calls);
        let inputs = InventoryMenuInputs {
            items: Rc::new(move || Some(rows.clone())),
            command: Rc::new(move |name: &str, args: &[String]| {
                record.borrow_mut().push((name.to_string(), args.to_vec()));
            }),
            controller,
            selected: Rc::new(RefCell::new(None)),
        };
        (inputs, calls, seat)
    }

    fn control(menu: &UiMenu, id: &str) -> UiControl {
        menu.controls
            .iter()
            .find(|control| control.id.as_str() == id)
            .unwrap_or_else(|| panic!("missing control: {id}"))
            .clone()
    }

    #[test]
    fn rows_cells_and_rects_match_donor() {
        let (inputs, _, _) = harness(vec![
            item("shells", "Shells", 12, false),
            item("nails", "Nails", 0, true),
        ]);
        let menu = build_inventory_menu(&menu_id(INVENTORY_MENU_ID), &inputs);
        assert_eq!(menu.id.as_str(), INVENTORY_MENU_ID);
        assert_eq!(menu.title, "Inventory");
        assert!(!menu.full_screen);
        assert_eq!(menu.controls.len(), 3);
        let list = control(&menu, "ui:inventory:items");
        assert_eq!(list.label, "Items");
        assert_eq!(
            (list.rect.x, list.rect.y, list.rect.width, list.rect.height),
            (64.0, 96.0, 512.0, 264.0)
        );
        match &list.kind {
            UiControlKind::List {
                rows,
                selected,
                row_height,
                ..
            } => {
                assert_eq!(*row_height, Some(33.0));
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0].cells, vec!["Shells".to_string(), "12".to_string()]);
                assert!(rows[0].enabled);
                assert!(!rows[1].enabled);
                assert_eq!(selected.as_deref(), Some("nails"));
            }
            other => panic!("expected a list: {other:?}"),
        }
        let use_button = control(&menu, "ui:inventory:use");
        assert!(use_button.enabled);
        assert_eq!(use_button.rect.y, 376.0);
        let close = control(&menu, "ui:inventory:close");
        assert!(close.enabled);
        assert_eq!(close.rect.y, 424.0);
    }

    #[test]
    fn empty_rows_disable_use() {
        let (inputs, _, _) = harness(Vec::new());
        let menu = build_inventory_menu(&menu_id(INVENTORY_MENU_ID), &inputs);
        assert!(!control(&menu, "ui:inventory:use").enabled);
    }

    #[test]
    fn select_then_use_dispatches_command() {
        let (inputs, calls, seat) = harness(vec![item("shells", "Shells", 12, false)]);
        let menu = build_inventory_menu(&menu_id(INVENTORY_MENU_ID), &inputs);
        match &control(&menu, "ui:inventory:items").kind {
            UiControlKind::List {
                on_select, on_activate, ..
            } => {
                on_select(seat.clone(), "shells");
                on_activate.as_ref().expect("row activation")(seat.clone(), "shells");
            }
            other => panic!("expected a list: {other:?}"),
        }
        match &control(&menu, "ui:inventory:use").kind {
            UiControlKind::Button { on_activate } => on_activate(seat.clone()),
            other => panic!("expected a button: {other:?}"),
        }
        (menu.on_close)(seat);
        let calls = calls.borrow();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], ("use".to_string(), vec!["shells".to_string()]));
        assert_eq!(calls[1], ("use".to_string(), vec!["shells".to_string()]));
        assert_eq!(calls[2], ("putaway".to_string(), Vec::new()));
    }

    #[test]
    fn register_and_dispose_track_menu() {
        let owner = IdentityOwner::create("library-inventory-register-test").expect("owner");
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat))));
        let menus = register_inventory_menu(&controller, Rc::new(|| None), Rc::new(|_, _| {}));
        assert_eq!(menus.root.as_str(), INVENTORY_MENU_ID);
        assert!(controller.borrow().is_registered(&menus.root));
        let root = menus.root.clone();
        menus.dispose();
        assert!(!controller.borrow().is_registered(&root));
    }
}
