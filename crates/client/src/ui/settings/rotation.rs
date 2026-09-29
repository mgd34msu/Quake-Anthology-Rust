//! Map rotation menu over one `server:map-rotation` binding.
//!
//! Donor provenance: `src/ui/settings/rotation.ts` in full. The view shapes
//! mirror `src/settings/server/types.ts` (`BoundServerSetting`,
//! `ServerSettingStatus`) and the helpers mirror
//! `src/settings/server/rotation.ts` (`rotationMapName`, `moveRotationMap`);
//! scheduling and storage stay with the existing source owner and the menu
//! only reads these snapshots.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::ClientError;
use crate::ui::common::controller::NativeUiController;
use crate::ui::common::layout::{menu_row, MenuRowOptions};
use crate::ui::types::{SeatUiController as _, UiChoice, UiControl, UiControlId, UiControlKind, UiMenu, UiMenuId};

use super::SettingsMenus;

/// Setting id edited by the rotation menu (donor `server:map-rotation`).
pub const MAP_ROTATION_SETTING_ID: &str = "server:map-rotation";
/// Rotation menu id (donor `menu:server:rotation`).
const ROTATION_MENU_ID: &str = "menu:server:rotation";
/// Map-name rejection message (donor `rotationMapName`).
const MAP_NAME_ERROR: &str = "Enter a map name, such as q2dm1 or q64/outpost";
/// Write failure when the source exposes no rotation setting (donor `write`).
const MISSING_ROTATION_ERROR: &str = "This source has no map rotation setting";
/// Idle notice row (donor `ui:rotation:notice`).
const NOTICE_IDLE: &str = "Save in Server profiles to reuse this order.";
/// Selection placeholder when the rotation is empty (donor choice label).
const EMPTY_ROTATION_LABEL: &str = "Empty: authored exits";

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

/// UI-local mirror of the donor `ServerApplyAt`: when an applied value takes
/// effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerApplyAt {
    /// Applies during play.
    Live,
    /// Applies on the next match.
    NextMatch,
    /// Applies on the next map.
    NextMap,
    /// Applies on server restart.
    Restart,
}

impl ServerApplyAt {
    /// Donor apply-at name.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ServerApplyAt::Live => "live",
            ServerApplyAt::NextMatch => "next-match",
            ServerApplyAt::NextMap => "next-map",
            ServerApplyAt::Restart => "restart",
        }
    }
}

/// UI-local mirror of a donor server-setting control shape.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerSettingKindView {
    /// Boolean toggle (`"0"`/`"1"`).
    Toggle,
    /// Fixed choice list.
    Choice {
        /// Available choices.
        choices: Vec<UiChoice>,
    },
    /// Numeric range.
    Slider {
        /// Minimum value.
        minimum: f32,
        /// Maximum value.
        maximum: f32,
        /// Whole numbers only.
        integer: bool,
    },
    /// Free text.
    TextEntry {
        /// Maximum length in characters.
        maximum_length: usize,
    },
    /// A shape this menu does not render natively; detail falls back to a
    /// text draft like the donor `else` branch.
    Other,
}

/// UI-local mirror of the donor `ServerSettingDefinition` view fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerSettingDefView {
    /// Setting id (`server:...`).
    pub id: String,
    /// Row label.
    pub label: String,
    /// Wrapped into the detail description rows.
    pub description: String,
    /// Reset value.
    pub default_value: String,
    /// Control shape.
    pub kind: ServerSettingKindView,
}

/// UI-local mirror of the donor `BoundServerSetting` plus its
/// `ServerSettingStatus`: one definition snapshot with desired and effective
/// values.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerBindingView {
    /// Setting definition snapshot.
    pub definition: ServerSettingDefView,
    /// Requested value.
    pub desired: String,
    /// Live value.
    pub effective: String,
    /// Whether desired differs from effective.
    pub pending: bool,
    /// When an applied value takes effect.
    pub apply_at: ServerApplyAt,
}

/// Normalize one map name the way the donor `rotationMapName` does: trim,
/// strip one leading `maps/` and one trailing `.bsp` (case-insensitive),
/// then require 1-127 characters of `[A-Za-z0-9_-]` segments separated by
/// single slashes.
pub fn rotation_map_name(value: &str) -> Result<String, ClientError> {
    let name = value.trim();
    let name = name.strip_prefix("maps/").unwrap_or(name);
    let bytes = name.as_bytes();
    let name = if bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".bsp") {
        &name[..name.len() - 4]
    } else {
        name
    };
    let valid = !name.is_empty()
        && name.len() <= 127
        && name
            .split('/')
            .all(|segment| !segment.is_empty() && segment.bytes().all(is_map_char));
    if valid {
        Ok(name.to_string())
    } else {
        Err(ClientError::BadUi(MAP_NAME_ERROR.to_string()))
    }
}

/// Whether a byte belongs in a rotation map-name segment.
fn is_map_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// Swap one rotation entry with its neighbor the way the donor
/// `moveRotationMap` does; an out-of-range index or destination returns the
/// list unchanged.
#[must_use]
pub fn move_rotation_map(maps: &[String], index: usize, direction: i32) -> Vec<String> {
    let destination = index as i64 + i64::from(direction);
    if destination < 0 {
        return maps.to_vec();
    }
    let destination = destination as usize;
    if index >= maps.len() || destination >= maps.len() {
        return maps.to_vec();
    }
    let mut result = maps.to_vec();
    result.swap(index, destination);
    result
}

/// Split a stored rotation value into map names the way the donor `maps`
/// does: runs of whitespace or commas separate entries, empties dropped.
#[must_use]
pub fn rotation_map_values(desired: &str) -> Vec<String> {
    desired
        .split(|char: char| char.is_whitespace() || char == ',')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

/// Write the joined rotation value, reporting the owner message on rejection.
pub type RotationWriteFn = Rc<dyn Fn(&str) -> Result<(), String>>;

/// Live rotation surface: read every binding snapshot, write the joined
/// `server:map-rotation` value.
#[derive(Clone)]
pub struct ServerRotationBindings {
    /// Read every binding snapshot; the menu finds `server:map-rotation`.
    pub read: Rc<dyn Fn() -> Vec<ServerBindingView>>,
    /// Write the joined rotation value.
    pub write: RotationWriteFn,
}

/// Rotation menu draft state (donor `selected`, `draft`, `error`).
#[derive(Debug, Clone, Default)]
pub struct RotationMenuState {
    /// Selected rotation entry, clamped on every build.
    pub selected: usize,
    /// Map-name draft.
    pub draft: String,
    /// Latched action error, shown in the notice row.
    pub error: String,
}

/// Inputs rebuilding the rotation menu.
#[derive(Clone)]
pub struct RotationMenuInputs {
    /// Live rotation surface.
    pub bindings: ServerRotationBindings,
    /// Controller owning the menu stack.
    pub controller: Rc<RefCell<NativeUiController>>,
    /// Draft state.
    pub state: Rc<RefCell<RotationMenuState>>,
}

/// Find the rotation binding snapshot, if the source exposes one.
fn current_rotation_binding(bindings: &ServerRotationBindings) -> Option<ServerBindingView> {
    (bindings.read)()
        .into_iter()
        .find(|binding| binding.definition.id == MAP_ROTATION_SETTING_ID)
}

/// Latch an action outcome into the notice error (donor `action`).
fn finish_action(state: &Rc<RefCell<RotationMenuState>>, outcome: Result<(), String>) {
    state.borrow_mut().error = outcome.err().unwrap_or_default();
}

/// Append the draft map to the rotation (donor add/submit body).
fn add_rotation_map(state: &Rc<RefCell<RotationMenuState>>, bindings: &ServerRotationBindings) {
    let outcome = (|| -> Result<(), String> {
        let current = current_rotation_binding(bindings).ok_or_else(|| MISSING_ROTATION_ERROR.to_string())?;
        let name = rotation_map_name(&state.borrow().draft).map_err(|_| MAP_NAME_ERROR.to_string())?;
        let values = rotation_map_values(&current.desired);
        (bindings.write)(
            &values
                .iter()
                .chain(std::iter::once(&name))
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        )?;
        let reread = current_rotation_binding(bindings)
            .map(|binding| rotation_map_values(&binding.desired))
            .unwrap_or_default();
        let mut state = state.borrow_mut();
        state.selected = reread.len().saturating_sub(1);
        state.draft.clear();
        Ok(())
    })();
    finish_action(state, outcome);
}

/// Write one rotation value list, failing when the source exposes no
/// rotation setting (donor `write`).
fn write_rotation_values(bindings: &ServerRotationBindings, values: &[String]) -> Result<(), String> {
    if current_rotation_binding(bindings).is_none() {
        return Err(MISSING_ROTATION_ERROR.to_string());
    }
    (bindings.write)(&values.join(" "))
}

/// Build one button row (donor `button`).
fn rotation_button(id: &str, label: String, row: i32, enabled: bool, on_activate: Rc<dyn Fn()>) -> UiControl {
    UiControl {
        id: control_id(id),
        label,
        rect: menu_row(row, &MenuRowOptions::default()),
        enabled,
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(move |_| on_activate()),
        },
    }
}

/// Build the rotation menu (donor `registerMapRotationMenu` factory).
#[must_use]
pub fn build_rotation_menu(root: &UiMenuId, inputs: &RotationMenuInputs) -> UiMenu {
    let rows = MenuRowOptions::default();
    let current = current_rotation_binding(&inputs.bindings);
    let values = current
        .as_ref()
        .map(|binding| rotation_map_values(&binding.desired))
        .unwrap_or_default();
    let has_rotation = current.is_some();
    {
        let mut state = inputs.state.borrow_mut();
        state.selected = if values.is_empty() {
            0
        } else {
            state.selected.min(values.len() - 1)
        };
    }
    let (selected, draft, error) = {
        let state = inputs.state.borrow();
        (state.selected, state.draft.clone(), state.error.clone())
    };

    let draft_state = Rc::clone(&inputs.state);
    let submit_state = Rc::clone(&inputs.state);
    let submit_bindings = inputs.bindings.clone();
    let add_state = Rc::clone(&inputs.state);
    let add_bindings = inputs.bindings.clone();
    let select_state = Rc::clone(&inputs.state);
    let select_bindings = inputs.bindings.clone();
    let up_state = Rc::clone(&inputs.state);
    let up_bindings = inputs.bindings.clone();
    let down_state = Rc::clone(&inputs.state);
    let down_bindings = inputs.bindings.clone();
    let remove_state = Rc::clone(&inputs.state);
    let remove_bindings = inputs.bindings.clone();
    let clear_state = Rc::clone(&inputs.state);
    let clear_bindings = inputs.bindings.clone();
    let back_controller = Rc::clone(&inputs.controller);

    let choices = if values.is_empty() {
        vec![UiChoice {
            id: "0".to_string(),
            label: EMPTY_ROTATION_LABEL.to_string(),
        }]
    } else {
        values
            .iter()
            .enumerate()
            .map(|(index, name)| UiChoice {
                id: index.to_string(),
                label: format!("{}. {name}", index + 1),
            })
            .collect()
    };
    let controls = vec![
        UiControl {
            id: control_id("ui:rotation:map"),
            label: "Add map".to_string(),
            rect: menu_row(0, &rows),
            enabled: has_rotation,
            visible: true,
            kind: UiControlKind::TextEntry {
                masked: false,
                text: draft,
                maximum_length: 127,
                on_change: Rc::new(move |_, value| {
                    draft_state.borrow_mut().draft = value.to_string();
                }),
                on_submit: Rc::new(move |_, _| {
                    add_rotation_map(&submit_state, &submit_bindings);
                }),
            },
        },
        rotation_button(
            "ui:rotation:add",
            "Add to end".to_string(),
            1,
            has_rotation,
            Rc::new(move || add_rotation_map(&add_state, &add_bindings)),
        ),
        UiControl {
            id: control_id("ui:rotation:selection"),
            label: "Rotation entry".to_string(),
            rect: menu_row(2, &rows),
            enabled: !values.is_empty(),
            visible: true,
            kind: UiControlKind::Choice {
                choices,
                selected: Some(selected.to_string()),
                on_select: Rc::new(move |_, value| {
                    let count = current_rotation_binding(&select_bindings)
                        .map(|binding| rotation_map_values(&binding.desired).len())
                        .unwrap_or(0);
                    if let Ok(index) = value.parse::<usize>() {
                        if index < count {
                            select_state.borrow_mut().selected = index;
                        }
                    }
                }),
            },
        },
        rotation_button(
            "ui:rotation:up",
            "Move earlier".to_string(),
            3,
            selected > 0,
            Rc::new(move || {
                let values = current_rotation_binding(&up_bindings)
                    .map(|binding| rotation_map_values(&binding.desired))
                    .unwrap_or_default();
                let selected = up_state.borrow().selected;
                let outcome = write_rotation_values(&up_bindings, &move_rotation_map(&values, selected, -1));
                if outcome.is_ok() {
                    up_state.borrow_mut().selected = selected.saturating_sub(1);
                }
                finish_action(&up_state, outcome);
            }),
        ),
        rotation_button(
            "ui:rotation:down",
            "Move later".to_string(),
            4,
            selected + 1 < values.len(),
            Rc::new(move || {
                let values = current_rotation_binding(&down_bindings)
                    .map(|binding| rotation_map_values(&binding.desired))
                    .unwrap_or_default();
                let selected = down_state.borrow().selected;
                let outcome = write_rotation_values(&down_bindings, &move_rotation_map(&values, selected, 1));
                if outcome.is_ok() {
                    down_state.borrow_mut().selected = selected.saturating_add(1);
                }
                finish_action(&down_state, outcome);
            }),
        ),
        rotation_button(
            "ui:rotation:remove",
            "Remove entry".to_string(),
            5,
            !values.is_empty(),
            Rc::new(move || {
                let values = current_rotation_binding(&remove_bindings)
                    .map(|binding| rotation_map_values(&binding.desired))
                    .unwrap_or_default();
                let selected = remove_state.borrow().selected;
                let kept: Vec<String> = values
                    .into_iter()
                    .enumerate()
                    .filter(|(index, _)| *index != selected)
                    .map(|(_, name)| name)
                    .collect();
                finish_action(&remove_state, write_rotation_values(&remove_bindings, &kept));
            }),
        ),
        rotation_button(
            "ui:rotation:clear",
            "Use authored exits".to_string(),
            6,
            !values.is_empty(),
            Rc::new(move || {
                finish_action(&clear_state, write_rotation_values(&clear_bindings, &[]));
            }),
        ),
        rotation_button(
            "ui:rotation:notice",
            if error.is_empty() {
                NOTICE_IDLE.to_string()
            } else {
                error
            },
            8,
            false,
            Rc::new(|| {}),
        ),
        rotation_button(
            "ui:rotation:back",
            "Back".to_string(),
            10,
            true,
            Rc::new(move || back_controller.borrow_mut().close_menu()),
        ),
    ];
    UiMenu {
        scroll: None,
        id: root.clone(),
        title: "Map rotation".to_string(),
        full_screen: false,
        controls,
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

/// Register the rotation menu, returning its root for the caller to track
/// for disposal.
pub(crate) fn register_rotation_menu_tracked(
    controller: &Rc<RefCell<NativeUiController>>,
    bindings: ServerRotationBindings,
) -> UiMenuId {
    let root = menu_id(ROTATION_MENU_ID);
    let inputs = RotationMenuInputs {
        bindings,
        controller: Rc::clone(controller),
        state: Rc::new(RefCell::new(RotationMenuState::default())),
    };
    let factory_root = root.clone();
    controller.borrow_mut().register(
        root.clone(),
        Rc::new(move || build_rotation_menu(&factory_root, &inputs)),
    );
    root
}

/// Register the map rotation menu (donor `registerMapRotationMenu`).
pub fn register_map_rotation_menu(
    controller: &Rc<RefCell<NativeUiController>>,
    bindings: ServerRotationBindings,
) -> SettingsMenus {
    let root = register_rotation_menu_tracked(controller, bindings);
    SettingsMenus::new(controller, root.clone(), vec![root])
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    use crate::ui::common::controller::headless_options;

    fn rotation_binding(desired: &str) -> ServerBindingView {
        ServerBindingView {
            definition: ServerSettingDefView {
                id: MAP_ROTATION_SETTING_ID.to_string(),
                label: "Map rotation".to_string(),
                description: "Ordered map names.".to_string(),
                default_value: String::new(),
                kind: ServerSettingKindView::TextEntry { maximum_length: 2048 },
            },
            desired: desired.to_string(),
            effective: desired.to_string(),
            pending: false,
            apply_at: ServerApplyAt::Live,
        }
    }

    struct FakeRotation {
        bindings: Vec<ServerBindingView>,
        writes: Vec<String>,
        fail_with: Option<String>,
    }

    impl FakeRotation {
        fn with_rotation(desired: &str) -> Rc<RefCell<Self>> {
            Rc::new(RefCell::new(Self {
                bindings: vec![rotation_binding(desired)],
                writes: Vec::new(),
                fail_with: None,
            }))
        }

        fn empty() -> Rc<RefCell<Self>> {
            Rc::new(RefCell::new(Self {
                bindings: Vec::new(),
                writes: Vec::new(),
                fail_with: None,
            }))
        }

        fn bindings(fake: &Rc<RefCell<Self>>) -> ServerRotationBindings {
            let read_fake = Rc::clone(fake);
            let write_fake = Rc::clone(fake);
            ServerRotationBindings {
                read: Rc::new(move || read_fake.borrow().bindings.clone()),
                write: Rc::new(move |value| {
                    let mut fake = write_fake.borrow_mut();
                    if let Some(message) = fake.fail_with.clone() {
                        return Err(message);
                    }
                    fake.writes.push(value.to_string());
                    for binding in &mut fake.bindings {
                        if binding.definition.id == MAP_ROTATION_SETTING_ID {
                            binding.desired = value.to_string();
                        }
                    }
                    Ok(())
                }),
            }
        }
    }

    struct Harness {
        inputs: RotationMenuInputs,
        root: UiMenuId,
        seat: qa_core::identity::SeatId,
        fake: Rc<RefCell<FakeRotation>>,
    }

    impl Harness {
        fn new(fake: Rc<RefCell<FakeRotation>>) -> Self {
            let owner = IdentityOwner::create("rotation-test").expect("test owner");
            let seat = owner.seat(0);
            let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
            let inputs = RotationMenuInputs {
                bindings: FakeRotation::bindings(&fake),
                controller: Rc::clone(&controller),
                state: Rc::new(RefCell::new(RotationMenuState::default())),
            };
            Self {
                inputs,
                root: menu_id(ROTATION_MENU_ID),
                seat,
                fake,
            }
        }

        fn menu(&self) -> UiMenu {
            build_rotation_menu(&self.root, &self.inputs)
        }

        fn control(menu: &UiMenu, id: &str) -> UiControl {
            menu.controls
                .iter()
                .find(|control| control.id.as_str() == id)
                .unwrap_or_else(|| panic!("missing control: {id}"))
                .clone()
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
                UiControlKind::TextEntry { on_change, .. } => on_change(self.seat.clone(), value),
                other => panic!("control is not a text entry: {id} ({other:?})"),
            }
        }

        fn submit_text(&self, menu: &UiMenu, id: &str) {
            let (text, on_submit) = match &Self::control(menu, id).kind {
                UiControlKind::TextEntry { text, on_submit, .. } => (text.clone(), Rc::clone(on_submit)),
                other => panic!("control is not a text entry: {id} ({other:?})"),
            };
            on_submit(self.seat.clone(), &text);
        }

        fn select(&self, menu: &UiMenu, id: &str, value: &str) {
            match &Self::control(menu, id).kind {
                UiControlKind::Choice { on_select, .. } => on_select(self.seat.clone(), value),
                other => panic!("control is not a choice: {id} ({other:?})"),
            }
        }
    }

    #[test]
    fn map_name_accepts_donor_shapes() {
        assert_eq!(rotation_map_name("q2dm1").expect("plain name"), "q2dm1");
        assert_eq!(rotation_map_name("  q2dm1  ").expect("trimmed"), "q2dm1");
        assert_eq!(rotation_map_name("maps/q2dm1").expect("maps prefix"), "q2dm1");
        assert_eq!(rotation_map_name("q2dm1.bsp").expect("bsp suffix"), "q2dm1");
        assert_eq!(rotation_map_name("maps/q2dm1.BSP").expect("upper bsp"), "q2dm1");
        assert_eq!(rotation_map_name("q64/outpost").expect("subdirectory"), "q64/outpost");
        assert_eq!(rotation_map_name("a-b_c/d_e-f").expect("punctuation"), "a-b_c/d_e-f");
    }

    #[test]
    fn map_name_rejects_donor_errors() {
        for invalid in [
            "",
            "   ",
            "maps/",
            ".bsp",
            "has space",
            "bad!char",
            "a//b",
            "/lead",
            "trail/",
            "q2dm1.bspx",
            &"a".repeat(128),
        ] {
            let error = rotation_map_name(invalid).expect_err("invalid map name");
            assert_eq!(error.to_string(), MAP_NAME_ERROR, "input: {invalid:?}");
        }
        assert!(rotation_map_name(&"a".repeat(127)).is_ok());
    }

    #[test]
    fn move_rotation_swaps_neighbors() {
        let maps = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(
            move_rotation_map(&maps, 1, -1),
            vec!["b".to_string(), "a".to_string(), "c".to_string()]
        );
        assert_eq!(
            move_rotation_map(&maps, 1, 1),
            vec!["a".to_string(), "c".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn move_rotation_keeps_out_of_range() {
        let maps = vec!["a".to_string(), "b".to_string()];
        assert_eq!(move_rotation_map(&maps, 0, -1), maps);
        assert_eq!(move_rotation_map(&maps, 1, 1), maps);
        assert_eq!(move_rotation_map(&maps, 5, -1), maps);
        assert_eq!(move_rotation_map(&[], 0, 1), Vec::<String>::new());
    }

    #[test]
    fn rotation_values_split_whitespace_and_commas() {
        assert_eq!(rotation_map_values(""), Vec::<String>::new());
        assert_eq!(rotation_map_values("  , ,"), Vec::<String>::new());
        assert_eq!(
            rotation_map_values("a b,c\t d\n,e"),
            vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string(),
                "e".to_string()
            ]
        );
    }

    #[test]
    fn empty_rotation_shows_placeholder_and_idle_notice() {
        let harness = Harness::new(FakeRotation::with_rotation(""));
        let menu = harness.menu();
        assert_eq!(menu.title, "Map rotation");
        assert!(!menu.full_screen);
        let selection = Harness::control(&menu, "ui:rotation:selection");
        match &selection.kind {
            UiControlKind::Choice { choices, selected, .. } => {
                assert_eq!(choices.len(), 1);
                assert_eq!(choices[0].label, EMPTY_ROTATION_LABEL);
                assert_eq!(selected.as_deref(), Some("0"));
            }
            other => panic!("expected a choice control ({other:?})"),
        }
        assert!(!selection.enabled);
        assert!(!Harness::enabled(&menu, "ui:rotation:up"));
        assert!(!Harness::enabled(&menu, "ui:rotation:down"));
        assert!(!Harness::enabled(&menu, "ui:rotation:remove"));
        assert!(!Harness::enabled(&menu, "ui:rotation:clear"));
        assert!(Harness::enabled(&menu, "ui:rotation:add"));
        assert_eq!(Harness::label(&menu, "ui:rotation:notice"), NOTICE_IDLE);
    }

    #[test]
    fn add_appends_validated_name_and_selects_it() {
        let harness = Harness::new(FakeRotation::with_rotation("a b"));
        harness.change_text(&harness.menu(), "ui:rotation:map", "maps/q2dm1.bsp");
        harness.submit_text(&harness.menu(), "ui:rotation:map");
        assert_eq!(harness.fake.borrow().writes, vec!["a b q2dm1".to_string()]);
        assert_eq!(harness.inputs.state.borrow().draft, "");
        assert_eq!(harness.inputs.state.borrow().selected, 2);
        let menu = harness.menu();
        assert_eq!(Harness::label(&menu, "ui:rotation:notice"), NOTICE_IDLE);
        match &Harness::control(&menu, "ui:rotation:selection").kind {
            UiControlKind::Choice { choices, selected, .. } => {
                assert_eq!(choices.len(), 3);
                assert_eq!(choices[2].label, "3. q2dm1");
                assert_eq!(selected.as_deref(), Some("2"));
            }
            other => panic!("expected a choice control ({other:?})"),
        }
    }

    #[test]
    fn add_button_matches_submit() {
        let harness = Harness::new(FakeRotation::with_rotation(""));
        harness.change_text(&harness.menu(), "ui:rotation:map", "q2dm1");
        harness.activate(&harness.menu(), "ui:rotation:add");
        assert_eq!(harness.fake.borrow().writes, vec!["q2dm1".to_string()]);
        assert_eq!(harness.inputs.state.borrow().selected, 0);
    }

    #[test]
    fn add_rejects_invalid_name_without_writing() {
        let harness = Harness::new(FakeRotation::with_rotation("a"));
        harness.change_text(&harness.menu(), "ui:rotation:map", "not a map!");
        harness.activate(&harness.menu(), "ui:rotation:add");
        assert!(harness.fake.borrow().writes.is_empty());
        assert_eq!(Harness::label(&harness.menu(), "ui:rotation:notice"), MAP_NAME_ERROR);
    }

    #[test]
    fn write_failure_latches_owner_message() {
        let fake = FakeRotation::with_rotation("a");
        fake.borrow_mut().fail_with = Some("Rotation is locked.".to_string());
        let harness = Harness::new(fake);
        harness.change_text(&harness.menu(), "ui:rotation:map", "q2dm1");
        harness.activate(&harness.menu(), "ui:rotation:add");
        assert_eq!(
            Harness::label(&harness.menu(), "ui:rotation:notice"),
            "Rotation is locked."
        );
    }

    #[test]
    fn select_moves_within_range_only() {
        let harness = Harness::new(FakeRotation::with_rotation("a b c"));
        harness.select(&harness.menu(), "ui:rotation:selection", "2");
        assert_eq!(harness.inputs.state.borrow().selected, 2);
        harness.select(&harness.menu(), "ui:rotation:selection", "9");
        assert_eq!(harness.inputs.state.borrow().selected, 2);
        harness.select(&harness.menu(), "ui:rotation:selection", "nope");
        assert_eq!(harness.inputs.state.borrow().selected, 2);
    }

    #[test]
    fn move_buttons_reorder_and_track_selection() {
        let harness = Harness::new(FakeRotation::with_rotation("a b c"));
        harness.select(&harness.menu(), "ui:rotation:selection", "1");
        harness.activate(&harness.menu(), "ui:rotation:up");
        assert_eq!(harness.fake.borrow().writes, vec!["b a c".to_string()]);
        assert_eq!(harness.inputs.state.borrow().selected, 0);
        assert!(!Harness::enabled(&harness.menu(), "ui:rotation:up"));
        harness.activate(&harness.menu(), "ui:rotation:down");
        assert_eq!(
            harness.fake.borrow().writes,
            vec!["b a c".to_string(), "a b c".to_string()]
        );
        assert_eq!(harness.inputs.state.borrow().selected, 1);
    }

    #[test]
    fn remove_and_clear_rewrite_rotation() {
        let harness = Harness::new(FakeRotation::with_rotation("a b c"));
        harness.select(&harness.menu(), "ui:rotation:selection", "1");
        harness.activate(&harness.menu(), "ui:rotation:remove");
        assert_eq!(harness.fake.borrow().writes, vec!["a c".to_string()]);
        harness.activate(&harness.menu(), "ui:rotation:clear");
        assert_eq!(harness.fake.borrow().writes, vec!["a c".to_string(), String::new()]);
        let menu = harness.menu();
        assert_eq!(Harness::label(&menu, "ui:rotation:notice"), NOTICE_IDLE);
        assert!(!Harness::enabled(&menu, "ui:rotation:remove"));
    }

    #[test]
    fn missing_binding_disables_edits_and_reports() {
        let harness = Harness::new(FakeRotation::empty());
        let menu = harness.menu();
        assert!(!Harness::enabled(&menu, "ui:rotation:map"));
        assert!(!Harness::enabled(&menu, "ui:rotation:add"));
        harness.activate(&menu, "ui:rotation:add");
        assert_eq!(
            Harness::label(&harness.menu(), "ui:rotation:notice"),
            MISSING_ROTATION_ERROR
        );
    }

    #[test]
    fn back_closes_menu_and_dispose_unregisters() {
        use crate::ui::types::SeatUiController as _;

        let owner = IdentityOwner::create("rotation-register-test").expect("test owner");
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
        let fake = FakeRotation::with_rotation("a");
        let menus = register_map_rotation_menu(&controller, FakeRotation::bindings(&fake));
        assert_eq!(menus.root.as_str(), ROTATION_MENU_ID);
        assert!(controller.borrow().is_registered(&menus.root));
        controller.borrow_mut().open_menu(&menus.root).expect("open rotation");
        assert_eq!(controller.borrow().active_menu(), Some(menus.root.clone()));
        let inputs = RotationMenuInputs {
            bindings: FakeRotation::bindings(&fake),
            controller: Rc::clone(&controller),
            state: Rc::new(RefCell::new(RotationMenuState::default())),
        };
        let menu = build_rotation_menu(&menus.root, &inputs);
        let back = Harness::control(&menu, "ui:rotation:back");
        match &back.kind {
            UiControlKind::Button { on_activate } => on_activate(seat),
            other => panic!("back is not a button ({other:?})"),
        }
        assert_eq!(controller.borrow().active_menu(), None);
        menus.dispose();
        assert!(!controller.borrow().is_registered(&menu_id(ROTATION_MENU_ID)));
    }
}
