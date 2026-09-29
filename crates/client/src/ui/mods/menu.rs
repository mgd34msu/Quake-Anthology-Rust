//! Mods menu.
//!
//! Ported from donor `src/ui/mods/menu.ts`: a searchable mod list with
//! per-row enable state, an enable/disable toggle (unavailable mods with a
//! reason can be disabled but never enabled), refresh, back, and a wrapped
//! details region under donor-fixed 640x480 rects.

use std::cell::RefCell;
use std::rc::Rc;

use crate::text::draw2d::Rect;
use crate::ui::common::controller::NativeUiController;
use crate::ui::types::{
    SeatCallback, SeatUiController as _, UiControl, UiControlId, UiControlKind, UiListRow, UiMenu, UiMenuId,
    UiMenuScroll,
};

/// Menu id for the mods root (`menu:mods:root` in the donor).
pub const MODS_MENU_ID: &str = "menu:mods:root";

fn control_id(text: &str) -> UiControlId {
    match UiControlId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI control id is invalid: {text}"),
    }
}

fn menu_id(text: &str) -> UiMenuId {
    match UiMenuId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI menu id is invalid: {text}"),
    }
}

/// One mod row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModMenuRow {
    /// Stable mod identity.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Source label.
    pub source: String,
    /// Whether the mod is enabled.
    pub enabled: bool,
    /// Unavailability reason; a disabled mod with a reason cannot be enabled.
    pub unavailable: Option<String>,
}

/// Backing service for the mods menu.
pub trait ModMenuService {
    /// Current rows.
    fn rows(&self) -> Vec<ModMenuRow>;
    /// Enable or disable one mod.
    fn set_enabled(&mut self, id: &str, enabled: bool);
    /// Reload rows.
    fn refresh(&mut self);
    /// Status line text; empty falls back to `"<n> enabled"`.
    fn status(&self) -> String;
}

/// Mutable mods menu view state: search query plus selected row.
#[derive(Debug, Clone, Default)]
pub struct ModMenuState {
    /// Search query text.
    pub query: String,
    /// Selected row id.
    pub selected: Option<String>,
}

/// Live mods menu inputs shared by the registered factory.
pub struct ModMenuInputs {
    /// Backing service.
    pub service: Rc<RefCell<dyn ModMenuService>>,
    /// Shared controller for the Back button.
    pub controller: Rc<RefCell<NativeUiController>>,
    /// Mutable view state.
    pub state: Rc<RefCell<ModMenuState>>,
}

/// Registered mods menu: root id plus disposal.
pub struct ModMenus {
    /// Menu root id.
    pub root: UiMenuId,
    controller: Rc<RefCell<NativeUiController>>,
}

impl ModMenus {
    /// Unregister the mods menu.
    pub fn dispose(self) {
        self.controller.borrow_mut().unregister(&self.root);
    }
}

impl std::fmt::Debug for ModMenus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModMenus")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// Whether one row may change state: enabled rows can always be disabled,
/// disabled rows only when no unavailability reason is present.
fn can_toggle(row: &ModMenuRow) -> bool {
    row.enabled || row.unavailable.is_none()
}

/// Apply the donor toggle rule: flip one row's state when allowed, else no-op.
fn try_toggle(service: &Rc<RefCell<dyn ModMenuService>>, id: &str) {
    let target = service.borrow().rows().into_iter().find(|row| row.id == id);
    if let Some(row) = target {
        if can_toggle(&row) {
            service.borrow_mut().set_enabled(id, !row.enabled);
        }
    }
}

/// Whether one row matches every whitespace-separated query term against
/// its lowercased `"<title> <source>"` text.
fn matches_query(row: &ModMenuRow, query: &str) -> bool {
    let text = format!("{} {}", row.title, row.source).to_lowercase();
    query.split_whitespace().all(|term| text.contains(&term.to_lowercase()))
}

/// Wrap one message into donor-style lines of at most 42 characters,
/// breaking at the last whitespace that fits and cutting long word runs.
fn wrap_message(message: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = message.trim_start();
    while !rest.is_empty() {
        let chars: Vec<char> = rest.chars().collect();
        if chars.len() <= 42 {
            lines.push(rest.to_string());
            break;
        }
        let boundary = (1..=42).rev().find(|&end| chars[end].is_whitespace());
        match boundary {
            Some(end) => {
                lines.push(chars[..end].iter().collect());
                rest = rest[end..].trim_start();
            }
            None => {
                lines.push(chars[..42].iter().collect());
                let cut: usize = chars[..42].iter().map(|c| c.len_utf8()).sum();
                rest = rest[cut..].trim_start();
            }
        }
    }
    lines
}

fn mods_button(suffix: &str, label: String, rect: Rect, enabled: bool, on_activate: SeatCallback) -> UiControl {
    UiControl {
        id: control_id(&format!("ui:mods:{suffix}")),
        label,
        rect,
        enabled,
        visible: true,
        kind: UiControlKind::Button { on_activate },
    }
}

/// Build one mods menu frame from the service and view state.
pub fn build_mod_menu(id: &UiMenuId, inputs: &ModMenuInputs) -> UiMenu {
    let all_rows = inputs.service.borrow().rows();
    let query = inputs.state.borrow().query.clone();
    let rows: Vec<ModMenuRow> = all_rows
        .iter()
        .filter(|row| matches_query(row, &query))
        .cloned()
        .collect();
    let current = rows
        .iter()
        .find(|row| Some(&row.id) == inputs.state.borrow().selected.as_ref())
        .or_else(|| rows.first())
        .cloned();
    inputs.state.borrow_mut().selected = current.as_ref().map(|row| row.id.clone());

    let state = Rc::clone(&inputs.state);
    let service = Rc::clone(&inputs.service);
    let change_state = Rc::clone(&state);
    let submit_state = Rc::clone(&state);
    let select_state = Rc::clone(&state);
    let activate_service = Rc::clone(&service);
    let toggle_service = Rc::clone(&service);
    let toggle_state = Rc::clone(&state);
    let refresh_service = Rc::clone(&service);
    let open_service = Rc::clone(&service);
    let back_controller = Rc::clone(&inputs.controller);

    let mut messages = Vec::new();
    if let Some(row) = current.as_ref() {
        messages.push(format!("{} ({})", row.title, row.source));
        if let Some(reason) = row.unavailable.as_ref() {
            messages.push(reason.clone());
        }
    }
    let status = service.borrow().status();
    if status.is_empty() {
        messages.push(format!("{} enabled", all_rows.iter().filter(|row| row.enabled).count()));
    } else {
        messages.push(status);
    }
    let lines: Vec<String> = messages.iter().flat_map(|message| wrap_message(message)).collect();
    let details: Vec<UiControl> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            mods_button(
                &format!("detail:{index}"),
                line.trim().to_string(),
                Rect {
                    x: 48.0,
                    y: 372.0 + index as f32 * 24.0,
                    width: 528.0,
                    height: 24.0,
                },
                false,
                Rc::new(|_| {}),
            )
        })
        .collect();

    let list_enabled = !rows.is_empty();
    let list_rows = if rows.is_empty() {
        vec![UiListRow {
            action: None,
            id: "empty".to_string(),
            cells: vec![if all_rows.is_empty() {
                "No mods available".to_string()
            } else {
                "No matching mods".to_string()
            }],
            image: None,
            enabled: false,
        }]
    } else {
        rows.iter()
            .map(|row| UiListRow {
                action: None,
                id: row.id.clone(),
                cells: vec![
                    row.title.clone(),
                    row.source.clone(),
                    if row.enabled {
                        "Enabled".to_string()
                    } else if row.unavailable.is_none() {
                        "Disabled".to_string()
                    } else {
                        "Unavailable".to_string()
                    },
                ],
                image: None,
                enabled: true,
            })
            .collect()
    };

    let selected = inputs.state.borrow().selected.clone();
    let toggle_label = if current.as_ref().is_some_and(|row| row.enabled) {
        "Disable"
    } else {
        "Enable"
    };
    let toggle_enabled = current.as_ref().is_some_and(can_toggle);
    let detail_ids: Vec<UiControlId> = details.iter().map(|control| control.id.clone()).collect();
    let mut controls = vec![
        UiControl {
            id: control_id("ui:mods:search"),
            label: "Search mods".to_string(),
            rect: Rect {
                x: 48.0,
                y: 80.0,
                width: 544.0,
                height: 28.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::TextEntry {
                masked: false,
                text: query,
                maximum_length: 80,
                on_change: Rc::new(move |_, value: &str| {
                    change_state.borrow_mut().query = value.to_string();
                }),
                on_submit: Rc::new(move |_, value: &str| {
                    submit_state.borrow_mut().query = value.to_string();
                }),
            },
        },
        mods_button(
            "title",
            "Mod".to_string(),
            Rect {
                x: 48.0,
                y: 112.0,
                width: 216.0,
                height: 28.0,
            },
            false,
            Rc::new(|_| {}),
        ),
        mods_button(
            "source",
            "Source".to_string(),
            Rect {
                x: 264.0,
                y: 112.0,
                width: 168.0,
                height: 28.0,
            },
            false,
            Rc::new(|_| {}),
        ),
        mods_button(
            "state",
            "State".to_string(),
            Rect {
                x: 432.0,
                y: 112.0,
                width: 160.0,
                height: 28.0,
            },
            false,
            Rc::new(|_| {}),
        ),
        UiControl {
            id: control_id("ui:mods:list"),
            label: "Mods".to_string(),
            rect: Rect {
                x: 48.0,
                y: 140.0,
                width: 544.0,
                height: 192.0,
            },
            enabled: list_enabled,
            visible: true,
            kind: UiControlKind::List {
                row_height: Some(32.0),
                column_widths: Some(vec![216.0, 168.0, 144.0]),
                on_activate: Some(Rc::new(move |_, id: &str| {
                    try_toggle(&activate_service, id);
                })),
                rows: list_rows,
                selected,
                on_select: Rc::new(move |_, id: &str| {
                    select_state.borrow_mut().selected = Some(id.to_string());
                }),
            },
        },
        mods_button(
            "toggle",
            toggle_label.to_string(),
            Rect {
                x: 48.0,
                y: 340.0,
                width: 176.0,
                height: 28.0,
            },
            toggle_enabled,
            Rc::new(move |_| {
                if let Some(selected) = toggle_state.borrow().selected.clone() {
                    try_toggle(&toggle_service, &selected);
                }
            }),
        ),
        mods_button(
            "refresh",
            "Refresh".to_string(),
            Rect {
                x: 232.0,
                y: 340.0,
                width: 176.0,
                height: 28.0,
            },
            true,
            Rc::new(move |_| {
                refresh_service.borrow_mut().refresh();
            }),
        ),
        mods_button(
            "back",
            "Back".to_string(),
            Rect {
                x: 416.0,
                y: 340.0,
                width: 176.0,
                height: 28.0,
            },
            true,
            Rc::new(move |_| {
                back_controller.borrow_mut().close_menu();
            }),
        ),
    ];
    controls.extend(details);
    UiMenu {
        scroll: Some(UiMenuScroll {
            rect: Rect {
                x: 48.0,
                y: 372.0,
                width: 544.0,
                height: 76.0,
            },
            content_height: lines.len() as f32 * 24.0,
            controls: detail_ids,
        }),
        id: id.clone(),
        title: "Mods".to_string(),
        full_screen: true,
        controls,
        on_open: Rc::new(move |_| {
            open_service.borrow_mut().refresh();
        }),
        on_close: Rc::new(|_| {}),
    }
}

/// Register the mods menu factory on the shared controller.
pub fn register_mod_menu(
    controller: &Rc<RefCell<NativeUiController>>,
    service: Rc<RefCell<dyn ModMenuService>>,
) -> ModMenus {
    let root = menu_id(MODS_MENU_ID);
    let factory_root = root.clone();
    let shared = Rc::new(ModMenuInputs {
        service,
        controller: Rc::clone(controller),
        state: Rc::new(RefCell::new(ModMenuState::default())),
    });
    controller
        .borrow_mut()
        .register(root.clone(), Rc::new(move || build_mod_menu(&factory_root, &shared)));
    ModMenus {
        root,
        controller: Rc::clone(controller),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, SeatId};

    use crate::ui::common::controller::headless_options;

    struct FakeService {
        rows: Vec<ModMenuRow>,
        status: String,
        toggles: Vec<(String, bool)>,
        refreshes: usize,
    }

    impl FakeService {
        fn new() -> Self {
            Self {
                rows: vec![
                    ModMenuRow {
                        id: "ctf".to_string(),
                        title: "Capture".to_string(),
                        source: "local".to_string(),
                        enabled: false,
                        unavailable: None,
                    },
                    ModMenuRow {
                        id: "arena".to_string(),
                        title: "Arena Pack".to_string(),
                        source: "workshop".to_string(),
                        enabled: true,
                        unavailable: None,
                    },
                    ModMenuRow {
                        id: "beta".to_string(),
                        title: "Beta Maps".to_string(),
                        source: "local".to_string(),
                        enabled: false,
                        unavailable: Some("missing files".to_string()),
                    },
                ],
                status: String::new(),
                toggles: Vec::new(),
                refreshes: 0,
            }
        }
    }

    impl ModMenuService for FakeService {
        fn rows(&self) -> Vec<ModMenuRow> {
            self.rows.clone()
        }
        fn set_enabled(&mut self, id: &str, enabled: bool) {
            self.toggles.push((id.to_string(), enabled));
            if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
                row.enabled = enabled;
            }
        }
        fn refresh(&mut self) {
            self.refreshes += 1;
        }
        fn status(&self) -> String {
            self.status.clone()
        }
    }

    fn harness(service: FakeService) -> (ModMenuInputs, Rc<RefCell<FakeService>>, SeatId) {
        let owner = IdentityOwner::create("mods-menu-test").expect("owner");
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
        let service = Rc::new(RefCell::new(service));
        let inputs = ModMenuInputs {
            service: service.clone(),
            controller,
            state: Rc::new(RefCell::new(ModMenuState::default())),
        };
        (inputs, service, seat)
    }

    fn menu_root() -> UiMenuId {
        menu_id(MODS_MENU_ID)
    }

    fn control(menu: &UiMenu, id: &str) -> UiControl {
        menu.controls
            .iter()
            .find(|control| control.id.as_str() == id)
            .unwrap_or_else(|| panic!("missing control: {id}"))
            .clone()
    }

    fn button_label(menu: &UiMenu, id: &str) -> String {
        control(menu, id).label.clone()
    }

    fn activate(menu: &UiMenu, seat: &SeatId, id: &str) {
        match &control(menu, id).kind {
            UiControlKind::Button { on_activate } => on_activate(seat.clone()),
            other => panic!("control is not a button: {id} ({other:?})"),
        }
    }

    fn list_rows(menu: &UiMenu) -> Vec<UiListRow> {
        match &control(menu, "ui:mods:list").kind {
            UiControlKind::List { rows, .. } => rows.clone(),
            other => panic!("expected a list: {other:?}"),
        }
    }

    #[test]
    fn rows_headers_and_rects_match_donor() {
        let (inputs, _, _) = harness(FakeService::new());
        let menu = build_mod_menu(&menu_root(), &inputs);
        assert_eq!(menu.title, "Mods");
        assert!(menu.full_screen);
        assert_eq!(control(&menu, "ui:mods:search").label, "Search mods");
        assert_eq!(control(&menu, "ui:mods:search").rect.y, 80.0);
        assert_eq!(button_label(&menu, "ui:mods:title"), "Mod");
        assert_eq!(button_label(&menu, "ui:mods:source"), "Source");
        assert_eq!(button_label(&menu, "ui:mods:state"), "State");
        assert!(!control(&menu, "ui:mods:title").enabled);
        match &control(&menu, "ui:mods:list").kind {
            UiControlKind::List {
                row_height,
                column_widths,
                selected,
                ..
            } => {
                assert_eq!(*row_height, Some(32.0));
                assert_eq!(column_widths.clone(), Some(vec![216.0, 168.0, 144.0]));
                assert_eq!(selected.as_deref(), Some("ctf"));
            }
            other => panic!("expected a list: {other:?}"),
        }
        let rows = list_rows(&menu);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0].cells,
            vec!["Capture".to_string(), "local".to_string(), "Disabled".to_string()]
        );
        assert_eq!(
            rows[1].cells,
            vec!["Arena Pack".to_string(), "workshop".to_string(), "Enabled".to_string()]
        );
        assert_eq!(
            rows[2].cells,
            vec!["Beta Maps".to_string(), "local".to_string(), "Unavailable".to_string()]
        );
        assert!(rows.iter().all(|row| row.enabled));
        assert!(control(&menu, "ui:mods:list").enabled);
        assert_eq!(button_label(&menu, "ui:mods:toggle"), "Enable");
        assert!(control(&menu, "ui:mods:toggle").enabled);
    }

    #[test]
    fn multi_term_search_filters_on_title_and_source() {
        let (inputs, _, seat) = harness(FakeService::new());
        match &control(&build_mod_menu(&menu_root(), &inputs), "ui:mods:search").kind {
            UiControlKind::TextEntry {
                on_change,
                maximum_length,
                ..
            } => {
                assert_eq!(*maximum_length, 80);
                on_change(seat.clone(), "pack workshop");
            }
            other => panic!("expected a text entry: {other:?}"),
        }
        let menu = build_mod_menu(&menu_root(), &inputs);
        let rows = list_rows(&menu);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "arena");
        match &control(&menu, "ui:mods:search").kind {
            UiControlKind::TextEntry { on_submit, .. } => on_submit(seat, "  BETA   local "),
            other => panic!("expected a text entry: {other:?}"),
        }
        let menu = build_mod_menu(&menu_root(), &inputs);
        let rows = list_rows(&menu);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "beta");
        // Unavailable disabled rows cannot be enabled: toggle button is off.
        assert_eq!(button_label(&menu, "ui:mods:toggle"), "Enable");
        assert!(!control(&menu, "ui:mods:toggle").enabled);
    }

    #[test]
    fn toggle_rules_skip_unavailable_disabled_rows() {
        let (inputs, service, seat) = harness(FakeService::new());
        let menu = build_mod_menu(&menu_root(), &inputs);
        match &control(&menu, "ui:mods:list").kind {
            UiControlKind::List {
                on_activate, on_select, ..
            } => {
                on_select(seat.clone(), "beta");
                on_activate.as_ref().expect("row activation")(seat.clone(), "beta");
            }
            other => panic!("expected a list: {other:?}"),
        }
        assert!(service.borrow().toggles.is_empty());
        let menu = build_mod_menu(&menu_root(), &inputs);
        match &control(&menu, "ui:mods:list").kind {
            UiControlKind::List { on_activate, .. } => {
                on_activate.as_ref().expect("row activation")(seat.clone(), "ctf");
            }
            other => panic!("expected a list: {other:?}"),
        }
        activate(&menu, &seat, "ui:mods:toggle");
        assert_eq!(service.borrow().toggles, vec![("ctf".to_string(), true)]);
        inputs.state.borrow_mut().selected = Some("arena".to_string());
        let menu = build_mod_menu(&menu_root(), &inputs);
        assert_eq!(button_label(&menu, "ui:mods:toggle"), "Disable");
        activate(&menu, &seat, "ui:mods:toggle");
        assert!(service.borrow().toggles.contains(&("arena".to_string(), false)));
    }

    #[test]
    fn empty_rows_status_and_details_match_donor() {
        let (inputs, service, _) = harness(FakeService::new());
        service.borrow_mut().rows.clear();
        let menu = build_mod_menu(&menu_root(), &inputs);
        assert!(!control(&menu, "ui:mods:list").enabled);
        let rows = list_rows(&menu);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cells, vec!["No mods available".to_string()]);
        assert!(!rows[0].enabled);
        assert!(!control(&menu, "ui:mods:toggle").enabled);

        service.borrow_mut().rows = FakeService::new().rows;
        inputs.state.borrow_mut().query = "zzz".to_string();
        let menu = build_mod_menu(&menu_root(), &inputs);
        let rows = list_rows(&menu);
        assert_eq!(rows[0].cells, vec!["No matching mods".to_string()]);

        inputs.state.borrow_mut().query = String::new();
        inputs.state.borrow_mut().selected = None;
        service.borrow_mut().status = "custom status".to_string();
        let menu = build_mod_menu(&menu_root(), &inputs);
        let detail_labels: Vec<String> = menu
            .controls
            .iter()
            .filter(|control| control.id.as_str().starts_with("ui:mods:detail:"))
            .map(|control| control.label.clone())
            .collect();
        assert!(detail_labels.iter().any(|line| line.contains("Capture (local)")));
        assert!(detail_labels.iter().any(|line| line.contains("custom status")));
        let scroll = menu.scroll.as_ref().expect("details scroll");
        assert_eq!(scroll.rect.y, 372.0);
        assert_eq!(scroll.rect.height, 76.0);
        assert_eq!(scroll.content_height, detail_labels.len() as f32 * 24.0);

        service.borrow_mut().status.clear();
        let menu = build_mod_menu(&menu_root(), &inputs);
        let detail_labels: Vec<String> = menu
            .controls
            .iter()
            .filter(|control| control.id.as_str().starts_with("ui:mods:detail:"))
            .map(|control| control.label.clone())
            .collect();
        assert!(detail_labels.iter().any(|line| line == "1 enabled"));
    }

    #[test]
    fn long_details_wrap_and_refresh_runs_on_open() {
        let (inputs, service, seat) = harness(FakeService::new());
        service.borrow_mut().status = "a very long status line that must wrap across several detail rows".to_string();
        let menu = build_mod_menu(&menu_root(), &inputs);
        let detail_count = menu
            .controls
            .iter()
            .filter(|control| control.id.as_str().starts_with("ui:mods:detail:"))
            .count();
        assert!(detail_count >= 3);
        for control in menu
            .controls
            .iter()
            .filter(|control| control.id.as_str().starts_with("ui:mods:detail:"))
        {
            assert!(
                control.label.chars().count() <= 42,
                "detail line too long: {}",
                control.label
            );
            assert!(!control.enabled);
        }
        activate(&menu, &seat, "ui:mods:refresh");
        (menu.on_open)(seat);
        assert_eq!(service.borrow().refreshes, 2);
    }

    #[test]
    fn register_and_dispose_track_menu() {
        let owner = IdentityOwner::create("mods-menu-register-test").expect("owner");
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat))));
        let service: Rc<RefCell<dyn ModMenuService>> = Rc::new(RefCell::new(FakeService::new()));
        let menus = register_mod_menu(&controller, service);
        assert_eq!(menus.root.as_str(), MODS_MENU_ID);
        assert!(controller.borrow().is_registered(&menus.root));
        let root = menus.root.clone();
        menus.dispose();
        assert!(!controller.borrow().is_registered(&root));
    }
}
