//! Input routing settings bindings.
//!
//! Ported from the TypeScript donor's `src/ui/settings/input-routing.ts`.
//! The router is re-read through shared ownership on every access because the
//! application may replace seats while these bindings live.

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::identity::SeatId;

use crate::ui::types::{UiChoice, UiControlId};

use super::{SettingBinding, SettingBindingKind, SettingCategory};

/// Controller assignment for one seat (donor `ControllerSelection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerSelectionView {
    /// Take the next free controller automatically.
    Automatic,
    /// Take no controller.
    None,
    /// Bind the controller with this serial.
    Serial {
        /// Device GUID.
        guid: String,
        /// Device serial.
        serial: String,
    },
    /// Bind the nth controller with this GUID.
    Device {
        /// Device GUID.
        guid: String,
        /// Ordinal among identical GUIDs.
        ordinal: i32,
    },
}

impl ControllerSelectionView {
    /// Stable choice id for this selection (donor `selectionKey`).
    #[must_use]
    pub fn selection_key(&self) -> String {
        match self {
            ControllerSelectionView::Automatic => "automatic".to_string(),
            ControllerSelectionView::None => "none".to_string(),
            ControllerSelectionView::Serial { guid, serial } => {
                format!("serial:{guid}:{serial}")
            }
            ControllerSelectionView::Device { guid, ordinal } => {
                format!("device:{guid}:{ordinal}")
            }
        }
    }
}

/// Connected controller description (donor `ControllerDevice` subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceView {
    /// Device name.
    pub name: String,
    /// Device GUID, when identifiable.
    pub guid: Option<String>,
    /// Device serial, when reported.
    pub serial: Option<String>,
    /// Ordinal among identical GUIDs.
    pub ordinal: i32,
    /// Provider instance handle.
    pub instance: i32,
}

/// Input router surface needed by the routing menus (donor `InputRouter`
/// subset).
pub trait ReRouter {
    /// Seats with local inputs, in player order.
    fn inputs(&self) -> Vec<SeatId>;
    /// Seat owning the keyboard and mouse, if any.
    fn keyboard_seat(&self) -> Option<SeatId>;
    /// Assign the keyboard and mouse seat.
    fn set_keyboard_seat(&mut self, seat: Option<SeatId>);
    /// Refresh capture after a seat assignment changes.
    fn update_capture(&mut self);
    /// Controller selection for a seat.
    fn controller_selection(&self, seat: SeatId) -> ControllerSelectionView;
    /// Assign a controller selection to a seat.
    fn set_controller_selection(&mut self, seat: SeatId, selection: ControllerSelectionView);
    /// Live controller instance routed to a seat, if any.
    fn controller_for(&self, seat: SeatId) -> Option<i32>;
}

/// Selection a device can be bound through, or `None` when the device has no
/// GUID to match on.
fn device_selection(device: &DeviceView) -> Option<ControllerSelectionView> {
    let guid = device.guid.clone()?;
    match &device.serial {
        Some(serial) if !serial.is_empty() => Some(ControllerSelectionView::Serial {
            guid,
            serial: serial.clone(),
        }),
        _ => Some(ControllerSelectionView::Device {
            guid,
            ordinal: device.ordinal,
        }),
    }
}

fn seat_choices(router: &dyn ReRouter) -> Vec<UiChoice> {
    router
        .inputs()
        .iter()
        .map(|seat| UiChoice {
            id: seat.index().to_string(),
            label: format!("Player {}", seat.index() + 1),
        })
        .collect()
}

/// Build a control id from a static template; the routing templates always
/// carry the `ui:` namespace and a name part, so a failure is a programming
/// bug.
fn control_id(text: &str) -> UiControlId {
    match UiControlId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI control id is invalid: {text}"),
    }
}

/// Bind the keyboard/controller routing controls
/// (donor `bindInputRoutingSettings`).
#[must_use]
pub fn bind_input_routing_settings(
    router: Rc<RefCell<dyn ReRouter>>,
    devices: Rc<dyn Fn() -> Vec<DeviceView>>,
) -> Vec<SettingBinding> {
    let selected_seat: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
    let selected = {
        let router = Rc::clone(&router);
        let selected_seat = Rc::clone(&selected_seat);
        Rc::new(move || {
            let router = router.borrow();
            let inputs = router.inputs();
            let wanted = *selected_seat.borrow();
            inputs
                .iter()
                .find(|seat| seat.index() == wanted)
                .or_else(|| inputs.first())
                .cloned()
        })
    };

    let keyboard = {
        let read_router = Rc::clone(&router);
        let choices_router = Rc::clone(&router);
        let write_router = Rc::clone(&router);
        SettingBinding {
            id: control_id("ui:input:keyboard-player"),
            label: "Keyboard and mouse player".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::Choice {
                read: Rc::new(move || {
                    read_router
                        .borrow()
                        .keyboard_seat()
                        .map(|seat| seat.index().to_string())
                        .unwrap_or_else(|| "none".to_string())
                }),
                choices: Rc::new(move || {
                    let mut listed = seat_choices(&*choices_router.borrow());
                    listed.push(UiChoice {
                        id: "none".to_string(),
                        label: "None".to_string(),
                    });
                    listed
                }),
                write: Rc::new(move |value| {
                    let mut router = write_router.borrow_mut();
                    if value == "none" {
                        router.set_keyboard_seat(None);
                        router.update_capture();
                        return;
                    }
                    let seat = router
                        .inputs()
                        .iter()
                        .find(|seat| seat.index().to_string() == value)
                        .cloned();
                    let Some(seat) = seat else {
                        panic!("Unknown keyboard player");
                    };
                    router.set_keyboard_seat(Some(seat));
                    router.update_capture();
                }),
            },
        }
    };

    let controller_player = {
        let read_selected = Rc::clone(&selected);
        let choices_router = Rc::clone(&router);
        let enabled_router = Rc::clone(&router);
        let write_router = Rc::clone(&router);
        let write_selected = Rc::clone(&selected_seat);
        SettingBinding {
            id: control_id("ui:input:controller-player"),
            label: "Assign controller to".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(move || enabled_router.borrow().inputs().len() > 1),
            kind: SettingBindingKind::Choice {
                read: Rc::new(move || {
                    read_selected()
                        .map(|seat| seat.index().to_string())
                        .unwrap_or_else(|| "0".to_string())
                }),
                choices: Rc::new(move || seat_choices(&*choices_router.borrow())),
                write: Rc::new(move |value| {
                    let seat = write_router
                        .borrow()
                        .inputs()
                        .iter()
                        .find(|seat| seat.index().to_string() == value)
                        .cloned();
                    let Some(seat) = seat else {
                        panic!("Unknown controller player");
                    };
                    *write_selected.borrow_mut() = seat.index();
                }),
            },
        }
    };

    let controller_device = {
        let read_router = Rc::clone(&router);
        let read_selected = Rc::clone(&selected);
        let choices_router = Rc::clone(&router);
        let choices_selected = Rc::clone(&selected);
        let choices_devices = Rc::clone(&devices);
        let enabled_selected = Rc::clone(&selected);
        let write_router = Rc::clone(&router);
        let write_selected = Rc::clone(&selected);
        let write_devices = Rc::clone(&devices);
        SettingBinding {
            id: control_id("ui:input:controller-device"),
            label: "Controller device".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(move || enabled_selected().is_some()),
            kind: SettingBindingKind::Choice {
                read: Rc::new(move || {
                    let Some(seat) = read_selected() else {
                        return "none".to_string();
                    };
                    read_router.borrow().controller_selection(seat).selection_key()
                }),
                choices: Rc::new(move || {
                    let mut listed = vec![
                        UiChoice {
                            id: "automatic".to_string(),
                            label: "Automatic".to_string(),
                        },
                        UiChoice {
                            id: "none".to_string(),
                            label: "None".to_string(),
                        },
                    ];
                    for device in choices_devices() {
                        if let Some(target) = device_selection(&device) {
                            listed.push(UiChoice {
                                id: target.selection_key(),
                                label: format!("{} ({})", device.name, device.ordinal + 1),
                            });
                        }
                    }
                    let current = choices_selected().map_or_else(
                        || "none".to_string(),
                        |seat| choices_router.borrow().controller_selection(seat).selection_key(),
                    );
                    if !listed.iter().any(|choice| choice.id == current) {
                        listed.push(UiChoice {
                            id: current,
                            label: "Saved controller (disconnected)".to_string(),
                        });
                    }
                    listed
                }),
                write: Rc::new(move |value| {
                    let Some(seat) = write_selected() else {
                        panic!("No local player");
                    };
                    let live: Vec<DeviceView> = write_devices();
                    let device = live
                        .iter()
                        .find(|device| device_selection(device).is_some_and(|target| target.selection_key() == value));
                    let target = if value == "automatic" {
                        Some(ControllerSelectionView::Automatic)
                    } else if value == "none" {
                        Some(ControllerSelectionView::None)
                    } else {
                        device.and_then(device_selection)
                    };
                    let mut router = write_router.borrow_mut();
                    let Some(target) = target else {
                        if router.controller_selection(seat.clone()).selection_key() == value {
                            return;
                        }
                        panic!("Controller is no longer available");
                    };
                    if !matches!(
                        target,
                        ControllerSelectionView::Automatic | ControllerSelectionView::None
                    ) {
                        for previous in router.inputs() {
                            if previous != seat
                                && (router.controller_selection(previous.clone()).selection_key() == value
                                    || device.is_some_and(|device| {
                                        router.controller_for(previous.clone()) == Some(device.instance)
                                    }))
                            {
                                router.set_controller_selection(previous, ControllerSelectionView::None);
                            }
                        }
                    }
                    router.set_controller_selection(seat, target);
                }),
            },
        }
    };

    vec![keyboard, controller_player, controller_device]
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use qa_core::identity::{IdentityOwner, SeatId};

    use crate::ui::settings::SettingBindingKind;

    use super::{bind_input_routing_settings, ControllerSelectionView, DeviceView, ReRouter};

    struct FakeRouter {
        seats: Vec<SeatId>,
        keyboard: Option<SeatId>,
        selections: HashMap<u32, ControllerSelectionView>,
        routes: HashMap<u32, i32>,
        captures: usize,
    }

    impl FakeRouter {
        fn new(owner: &IdentityOwner, seats: u32) -> Self {
            let mut selections = HashMap::new();
            for index in 0..seats {
                selections.insert(index, ControllerSelectionView::Automatic);
            }
            Self {
                seats: (0..seats).map(|index| owner.seat(index)).collect(),
                keyboard: None,
                selections,
                routes: HashMap::new(),
                captures: 0,
            }
        }
    }

    impl ReRouter for FakeRouter {
        fn inputs(&self) -> Vec<SeatId> {
            self.seats.clone()
        }

        fn keyboard_seat(&self) -> Option<SeatId> {
            self.keyboard.clone()
        }

        fn set_keyboard_seat(&mut self, seat: Option<SeatId>) {
            self.keyboard = seat;
        }

        fn update_capture(&mut self) {
            self.captures += 1;
        }

        fn controller_selection(&self, seat: SeatId) -> ControllerSelectionView {
            self.selections
                .get(&seat.index())
                .cloned()
                .unwrap_or(ControllerSelectionView::None)
        }

        fn set_controller_selection(&mut self, seat: SeatId, selection: ControllerSelectionView) {
            self.selections.insert(seat.index(), selection);
        }

        fn controller_for(&self, seat: SeatId) -> Option<i32> {
            self.routes.get(&seat.index()).copied()
        }
    }

    fn pad(name: &str, guid: Option<&str>, serial: Option<&str>, ordinal: i32, instance: i32) -> DeviceView {
        DeviceView {
            name: name.to_string(),
            guid: guid.map(str::to_string),
            serial: serial.map(str::to_string),
            ordinal,
            instance,
        }
    }

    fn harness(seats: u32, devices: Vec<DeviceView>) -> (Rc<RefCell<FakeRouter>>, Vec<super::SettingBinding>) {
        let owner = IdentityOwner::create("routing-test").expect("owner");
        let router = Rc::new(RefCell::new(FakeRouter::new(&owner, seats)));
        let shared = Rc::clone(&router);
        let rerouter: Rc<RefCell<dyn ReRouter>> = shared;
        let list: Rc<dyn Fn() -> Vec<DeviceView>> = Rc::new(move || devices.clone());
        let bindings = bind_input_routing_settings(rerouter, list);
        (router, bindings)
    }

    fn kind_name(kind: &SettingBindingKind) -> &'static str {
        match kind {
            SettingBindingKind::Toggle { .. } => "toggle",
            SettingBindingKind::Slider { .. } => "slider",
            SettingBindingKind::Choice { .. } => "choice",
            SettingBindingKind::TextEntry { .. } => "text-entry",
            SettingBindingKind::Button { .. } => "button",
        }
    }

    #[test]
    fn selection_keys_match_donor() {
        assert_eq!(ControllerSelectionView::Automatic.selection_key(), "automatic");
        assert_eq!(ControllerSelectionView::None.selection_key(), "none");
        assert_eq!(
            ControllerSelectionView::Serial {
                guid: "g".to_string(),
                serial: "s".to_string(),
            }
            .selection_key(),
            "serial:g:s"
        );
        assert_eq!(
            ControllerSelectionView::Device {
                guid: "g".to_string(),
                ordinal: 2,
            }
            .selection_key(),
            "device:g:2"
        );
    }

    #[test]
    fn keyboard_assignment_updates_capture() {
        let (router, bindings) = harness(2, vec![]);
        match &bindings[0].kind {
            SettingBindingKind::Choice { read, write, choices } => {
                assert_eq!(read(), "none");
                let listed: Vec<String> = choices().iter().map(|c| c.id.clone()).collect();
                assert_eq!(listed, vec!["0", "1", "none"]);
                write("1");
                assert_eq!(read(), "1");
                write("none");
                assert_eq!(read(), "none");
            }
            other => panic!("expected keyboard choice, got {}", kind_name(other)),
        }
        assert_eq!(router.borrow().captures, 2);
        assert!(router.borrow().keyboard_seat().is_none());
    }

    #[test]
    #[should_panic(expected = "Unknown keyboard player")]
    fn keyboard_assignment_rejects_unknown_player() {
        let (_, bindings) = harness(2, vec![]);
        match &bindings[0].kind {
            SettingBindingKind::Choice { write, .. } => write("9"),
            other => panic!("expected keyboard choice, got {}", kind_name(other)),
        }
    }

    #[test]
    fn controller_player_requires_known_seat() {
        let (router, bindings) = harness(2, vec![]);
        assert!((bindings[1].enabled)());
        match &bindings[1].kind {
            SettingBindingKind::Choice { read, write, .. } => {
                assert_eq!(read(), "0");
                write("1");
                assert_eq!(read(), "1");
            }
            other => panic!("expected controller-player choice, got {}", kind_name(other)),
        }
        assert_eq!(router.borrow().captures, 0);

        let (_, single) = harness(1, vec![]);
        assert!(!(single[1].enabled)());
    }

    #[test]
    #[should_panic(expected = "Unknown controller player")]
    fn controller_player_rejects_unknown_seat() {
        let (_, bindings) = harness(2, vec![]);
        match &bindings[1].kind {
            SettingBindingKind::Choice { write, .. } => write("7"),
            other => panic!("expected controller-player choice, got {}", kind_name(other)),
        }
    }

    #[test]
    fn controller_assignment_steals_from_other_seat() {
        let devices = vec![
            pad("Pad", Some("guid-1"), Some("serial-1"), 0, 11),
            pad("Stick", Some("guid-2"), None, 1, 12),
        ];
        let (router, bindings) = harness(2, devices);
        router.borrow_mut().routes.insert(0, 11);
        router.borrow_mut().selections.insert(
            0,
            ControllerSelectionView::Serial {
                guid: "guid-1".to_string(),
                serial: "serial-1".to_string(),
            },
        );
        let key = "serial:guid-1:serial-1";
        match &bindings[2].kind {
            SettingBindingKind::Choice { choices, .. } => {
                let listed: Vec<String> = choices().iter().map(|c| c.id.clone()).collect();
                assert!(listed.contains(&key.to_string()));
                assert!(listed.contains(&"device:guid-2:1".to_string()));
            }
            other => panic!("expected controller-device choice, got {}", kind_name(other)),
        }
        match &bindings[1].kind {
            SettingBindingKind::Choice { write, .. } => write("1"),
            other => panic!("expected controller-player choice, got {}", kind_name(other)),
        }
        match &bindings[2].kind {
            SettingBindingKind::Choice { write, .. } => write(key),
            other => panic!("expected controller-device choice, got {}", kind_name(other)),
        }
        let router = router.borrow();
        assert_eq!(
            router.selections.get(&1),
            Some(&ControllerSelectionView::Serial {
                guid: "guid-1".to_string(),
                serial: "serial-1".to_string(),
            })
        );
        assert_eq!(router.selections.get(&0), Some(&ControllerSelectionView::None));
    }

    #[test]
    fn saved_disconnected_controller_stays_selectable() {
        let (router, bindings) = harness(1, vec![]);
        router.borrow_mut().selections.insert(
            0,
            ControllerSelectionView::Device {
                guid: "gone".to_string(),
                ordinal: 0,
            },
        );
        match &bindings[2].kind {
            SettingBindingKind::Choice { read, write, choices } => {
                assert_eq!(read(), "device:gone:0");
                let listed = choices();
                let saved = listed
                    .iter()
                    .find(|choice| choice.id == "device:gone:0")
                    .expect("saved entry");
                assert_eq!(saved.label, "Saved controller (disconnected)");
                write("device:gone:0");
            }
            other => panic!("expected controller-device choice, got {}", kind_name(other)),
        }
    }

    #[test]
    #[should_panic(expected = "Controller is no longer available")]
    fn controller_assignment_rejects_unlisted_device() {
        let (_, bindings) = harness(1, vec![]);
        match &bindings[2].kind {
            SettingBindingKind::Choice { write, .. } => write("device:other:0"),
            other => panic!("expected controller-device choice, got {}", kind_name(other)),
        }
    }

    #[test]
    #[should_panic(expected = "No local player")]
    fn routing_without_players_reports_no_local_player() {
        let (_, bindings) = harness(0, vec![]);
        assert!(!(bindings[2].enabled)());
        match &bindings[2].kind {
            SettingBindingKind::Choice { read, write, .. } => {
                assert_eq!(read(), "none");
                write("automatic");
            }
            other => panic!("expected controller-device choice, got {}", kind_name(other)),
        }
    }

    #[test]
    fn guidless_devices_are_not_listed() {
        let devices = vec![pad("Mystery", None, None, 0, 3)];
        let (_, bindings) = harness(1, devices);
        match &bindings[2].kind {
            SettingBindingKind::Choice { choices, .. } => {
                let listed: Vec<String> = choices().iter().map(|c| c.id.clone()).collect();
                assert_eq!(listed, vec!["automatic", "none"]);
            }
            other => panic!("expected controller-device choice, got {}", kind_name(other)),
        }
    }
}
